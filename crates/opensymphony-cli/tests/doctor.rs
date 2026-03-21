use std::{
    path::PathBuf,
    process::{Command, ExitCode},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use opensymphony_cli::run_doctor_command;
use opensymphony_testkit::FakeOpenHandsServer;
use serde_yaml::Value;
use tempfile::TempDir;

#[tokio::test]
async fn doctor_live_probe_succeeds_against_fake_server() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir should have workspace parent")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf();
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let workspace_root = temp_dir.path().join("var/workspaces");
    let config_path = temp_dir.path().join("doctor.yaml");
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("workspace_root".to_string()),
                Value::String(workspace_root.display().to_string()),
            ),
            (
                Value::String("target_repo".to_string()),
                Value::String(repo_root.join("examples/target-repo").display().to_string()),
            ),
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
                        (
                            Value::String("base_url".to_string()),
                            Value::String(server.base_url().to_string()),
                        ),
                        (
                            Value::String("tool_dir".to_string()),
                            Value::String(
                                repo_root
                                    .join("tools/openhands-server")
                                    .display()
                                    .to_string(),
                            ),
                        ),
                        (Value::String("probe_model".to_string()), Value::Null),
                        (Value::String("probe_api_key_env".to_string()), Value::Null),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
            (
                Value::String("linear".to_string()),
                Value::Mapping(
                    [
                        (Value::String("enabled".to_string()), Value::Bool(false)),
                        (
                            Value::String("api_key_env".to_string()),
                            Value::String("LINEAR_API_KEY".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    ))
    .expect("config should serialize");
    std::fs::write(&config_path, config).expect("config should be written");

    let status = run_doctor_command(config_path, true).await;
    assert_eq!(status, ExitCode::SUCCESS);
}

#[test]
fn doctor_defaults_target_repo_from_checkout_root() {
    let repo_root = repo_root();
    let config_dir =
        tempfile::tempdir_in(repo_root.join("examples/configs")).expect("config dir should exist");
    let config_path = config_dir.path().join("doctor-default-target.yaml");
    let workspace_root = config_dir.path().join("workspaces");
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("workspace_root".to_string()),
                Value::String(workspace_root.display().to_string()),
            ),
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
                        (
                            Value::String("base_url".to_string()),
                            Value::String("http://127.0.0.1:8000".to_string()),
                        ),
                        (
                            Value::String("tool_dir".to_string()),
                            Value::String(
                                repo_root
                                    .join("tools/openhands-server")
                                    .display()
                                    .to_string(),
                            ),
                        ),
                        (Value::String("probe_model".to_string()), Value::Null),
                        (Value::String("probe_api_key_env".to_string()), Value::Null),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
            (
                Value::String("linear".to_string()),
                Value::Mapping(
                    [
                        (Value::String("enabled".to_string()), Value::Bool(false)),
                        (
                            Value::String("api_key_env".to_string()),
                            Value::String("LINEAR_API_KEY".to_string()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
        ]
        .into_iter()
        .collect(),
    ))
    .expect("config should serialize");
    std::fs::write(&config_path, config).expect("config should be written");

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("doctor")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&repo_root)
        .output()
        .expect("doctor command should run");

    assert!(
        output.status.success(),
        "doctor should succeed with bundled target repo fallback: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn run_local_launcher_is_independent_of_caller_cwd() {
    let repo_root = repo_root();
    let tool_dir = repo_root.join("tools/openhands-server");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let log_path = fake_bin_dir.path().join("uv.log");
    let fake_uv = fake_bin_dir.path().join("uv");
    std::fs::write(
        &fake_uv,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$PWD\" > \"{}\"\nprintf '%s\\n' \"$@\" >> \"{}\"\n",
            log_path.display(),
            log_path.display(),
        ),
    )
    .expect("fake uv should be written");
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&fake_uv)
            .expect("fake uv metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_uv, perms).expect("fake uv should be executable");
    }

    let path = std::env::var("PATH").unwrap_or_default();
    let status = Command::new("bash")
        .arg(tool_dir.join("run-local.sh"))
        .current_dir(&repo_root)
        .env("PATH", format!("{}:{path}", fake_bin_dir.path().display()))
        .status()
        .expect("launcher should run");
    assert!(
        status.success(),
        "fake uv launcher should exit successfully"
    );

    let log = std::fs::read_to_string(&log_path).expect("fake uv should have logged its call");
    let mut lines = log.lines();
    let observed_cwd = lines.next().unwrap_or_default();
    let args = lines.collect::<Vec<_>>();
    let has_project_arg = args
        .windows(2)
        .any(|window| matches!(window, ["--project" | "--directory", value] if *value == tool_dir.display().to_string()));

    assert!(
        observed_cwd == tool_dir.display().to_string() || has_project_arg,
        "launcher should either cd into the tool dir or pass it to uv; cwd={observed_cwd}, args={args:?}",
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir should have workspace parent")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}
