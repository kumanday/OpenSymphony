use std::{ffi::OsString, path::PathBuf, process::Command};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::opensymphony_testkit::FakeOpenHandsServer;
use serde_yaml::Value;
use tempfile::TempDir;

#[tokio::test]
async fn doctor_live_probe_succeeds_against_fake_server() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let repo_root = repo_root();
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let workspace_root = temp_dir.path().join("var/workspaces");
    let target_repo = temp_dir.path().join("target-repo");
    let config_path = temp_dir.path().join("doctor.yaml");
    std::fs::create_dir_all(&target_repo).expect("target repo should be created");
    std::fs::write(
        target_repo.join("WORKFLOW.md"),
        doctor_workflow_source(&workspace_root, server.base_url()),
    )
    .expect("workflow should be written");
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("target_repo".to_string()),
                Value::String(target_repo.display().to_string()),
            ),
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
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
                    [(Value::String("enabled".to_string()), Value::Bool(false))]
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
    let fake_uv = fake_command_on_path("uv");

    let path = fake_uv.path.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_opensymphony"))
            .arg("doctor")
            .arg("--config")
            .arg(&config_path)
            .arg("--live-openhands")
            .env("PATH", path)
            .output()
    })
    .await
    .expect("doctor child task should join")
    .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "doctor live probe should succeed against the fake server: stdout={stdout}, stderr={stderr}",
    );
}

#[test]
fn doctor_defaults_target_repo_from_checkout_root_even_outside_the_repo_cwd() {
    let repo_root = repo_root();
    let config_dir =
        tempfile::tempdir_in(repo_root.join("examples/configs")).expect("config dir should exist");
    let config_path = config_dir.path().join("doctor-default-target.yaml");
    let outside_repo = TempDir::new().expect("outside repo dir should be created");
    let fake_uv = fake_command_on_path("uv");
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
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
                    [(Value::String("enabled".to_string()), Value::Bool(false))]
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
        .current_dir(outside_repo.path())
        .env("PATH", fake_uv.path)
        .output()
        .expect("doctor command should run");

    assert!(
        output.status.success(),
        "doctor should succeed with checkout-root target repo fallback from outside the repo cwd: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn doctor_fails_when_required_env_placeholder_is_unset() {
    let repo_root = repo_root();
    let config_dir =
        tempfile::tempdir_in(repo_root.join("examples/configs")).expect("config dir should exist");
    let config_path = config_dir.path().join("doctor-missing-env.yaml");
    let missing_var = "OSYM_TEST_MISSING_TOOL_DIR";
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("target_repo".to_string()),
                Value::String(repo_root.join("examples/target-repo").display().to_string()),
            ),
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
                        (
                            Value::String("tool_dir".to_string()),
                            Value::String(format!("${{{missing_var}}}")),
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
                    [(Value::String("enabled".to_string()), Value::Bool(false))]
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
        .env_remove(missing_var)
        .output()
        .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "doctor should fail when a required env placeholder is unset: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains(missing_var) || stderr.contains(missing_var),
        "doctor error should mention the missing env placeholder: stdout={stdout}, stderr={stderr}",
    );
}

#[test]
fn doctor_ignores_unset_optional_live_placeholders_without_live_openhands() {
    let repo_root = repo_root();
    let config_dir =
        tempfile::tempdir_in(repo_root.join("examples/configs")).expect("config dir should exist");
    let config_path = config_dir
        .path()
        .join("doctor-optional-live-placeholder.yaml");
    let missing_var = "OSYM_TEST_MISSING_PROBE_MODEL";
    let fake_uv = fake_command_on_path("uv");
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("target_repo".to_string()),
                Value::String(repo_root.join("examples/target-repo").display().to_string()),
            ),
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
                        (
                            Value::String("tool_dir".to_string()),
                            Value::String(
                                repo_root
                                    .join("tools/openhands-server")
                                    .display()
                                    .to_string(),
                            ),
                        ),
                        (
                            Value::String("probe_model".to_string()),
                            Value::String(format!("${{{missing_var}}}")),
                        ),
                        (Value::String("probe_api_key_env".to_string()), Value::Null),
                    ]
                    .into_iter()
                    .collect(),
                ),
            ),
            (
                Value::String("linear".to_string()),
                Value::Mapping(
                    [(Value::String("enabled".to_string()), Value::Bool(false))]
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
        .env("PATH", fake_uv.path)
        .env_remove(missing_var)
        .output()
        .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "doctor should ignore unset live-only placeholders when live checks are disabled: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        !stdout.contains(missing_var) && !stderr.contains(missing_var),
        "static doctor should not fail on the unset live-only placeholder: stdout={stdout}, stderr={stderr}",
    );
}

#[test]
fn doctor_reports_local_safety_warning_and_repo_root_path() {
    let repo_root = repo_root();
    let fake_uv = fake_command_on_path("uv");

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("doctor")
        .arg("--config")
        .arg("examples/configs/local-dev.yaml")
        .current_dir(&repo_root)
        .env("PATH", fake_uv.path)
        .output()
        .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "doctor should succeed in the repo fixture environment: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("[WARN] local-safety: trusted-machine mode only"),
        "doctor output should state the trusted-machine limitation: stdout={stdout}",
    );
    assert!(
        stdout.contains(&format!(
            "[PASS] repo: found Cargo workspace at {}",
            repo_root.display()
        )),
        "doctor should print the resolved repo root path instead of an empty detail: stdout={stdout}",
    );
}

#[test]
fn doctor_fails_when_required_prerequisite_is_missing() {
    let repo_root = repo_root();
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");

    for command in ["cargo", "curl", "git"] {
        write_fake_executable(fake_bin_dir.path().join(command));
    }

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("doctor")
        .arg("--config")
        .arg("examples/configs/local-dev.yaml")
        .current_dir(&repo_root)
        .env("PATH", path_only(fake_bin_dir.path()))
        .output()
        .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "doctor should fail when a required prerequisite is missing: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("[FAIL] prereq-uv: uv is not on PATH"),
        "doctor should name the missing prerequisite explicitly: stdout={stdout}",
    );
}

#[test]
fn doctor_accepts_present_prerequisites_from_path() {
    let repo_root = repo_root();
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let home_dir = TempDir::new().expect("fake home dir should be created");

    for command in ["cargo", "curl", "git", "uv"] {
        write_fake_executable(fake_bin_dir.path().join(command));
    }
    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("doctor")
        .arg("--config")
        .arg("examples/configs/local-dev.yaml")
        .current_dir(&repo_root)
        .env("HOME", home_dir.path())
        .env("PATH", path_only(fake_bin_dir.path()))
        .output()
        .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "doctor should succeed when required prerequisites are present: stdout={stdout}, stderr={stderr}",
    );
    for check in ["prereq-cargo", "prereq-curl", "prereq-git", "prereq-uv"] {
        assert!(
            stdout.contains(&format!("[PASS] {check}:")),
            "doctor should report a passing prerequisite check for {check}: stdout={stdout}",
        );
    }
    assert!(
        stdout.contains("[PASS] openhands-install:"),
        "doctor should report the managed-local tooling bootstrap: stdout={stdout}",
    );
    assert!(
        home_dir
            .path()
            .join(".opensymphony/openhands-server/install.sh")
            .is_file(),
        "doctor should bootstrap the managed-local tooling into the HOME-based tool dir",
    );
}

#[test]
fn doctor_bootstraps_missing_managed_local_tooling_into_explicit_configured_dir() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let target_repo = temp_dir.path().join("target-repo");
    let workspace_root = temp_dir.path().join("var/workspaces");
    let tool_dir = temp_dir.path().join("managed/openhands-server");
    let config_path = temp_dir.path().join("doctor.yaml");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");

    std::fs::create_dir_all(&target_repo).expect("target repo should be created");
    std::fs::write(
        target_repo.join("WORKFLOW.md"),
        doctor_workflow_source(&workspace_root, "http://127.0.0.1:8000"),
    )
    .expect("workflow should be written");
    let config = serde_yaml::to_string(&Value::Mapping(
        [
            (
                Value::String("target_repo".to_string()),
                Value::String(target_repo.display().to_string()),
            ),
            (
                Value::String("openhands".to_string()),
                Value::Mapping(
                    [
                        (
                            Value::String("tool_dir".to_string()),
                            Value::String(tool_dir.display().to_string()),
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
                    [(Value::String("enabled".to_string()), Value::Bool(false))]
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

    for command in ["cargo", "curl", "git", "uv"] {
        write_fake_executable(fake_bin_dir.path().join(command));
    }
    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("doctor")
        .arg("--config")
        .arg(&config_path)
        .env("PATH", path_only(fake_bin_dir.path()))
        .output()
        .expect("doctor command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "doctor should bootstrap missing managed-local tooling: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("[PASS] openhands-install: installed pinned OpenHands tooling 1.24.0"),
        "doctor should report the bootstrap install explicitly: stdout={stdout}",
    );
    assert!(
        tool_dir.join("install.sh").is_file()
            && tool_dir.join("run-local.sh").is_file()
            && tool_dir.join("pyproject.toml").is_file()
            && tool_dir.join("uv.lock").is_file()
            && tool_dir.join("version.txt").is_file(),
        "doctor should materialize the managed-local tooling bundle into the configured tool dir",
    );
}

#[test]
fn run_local_launcher_enforces_pinned_supervised_contract() {
    let repo_root = repo_root();
    let tool_dir = repo_root.join("tools/openhands-server");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let log_path = fake_bin_dir.path().join("uv.log");
    let fake_uv = fake_bin_dir.path().join("uv");
    std::fs::write(
        &fake_uv,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$PWD\" > \"{}\"\nprintf 'RUNTIME=%s\\n' \"${{RUNTIME:-}}\" >> \"{}\"\nprintf '%s\\n' \"$@\" >> \"{}\"\n",
            log_path.display(),
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
        .env("OPENHANDS_SERVER_PORT", "8123")
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
    let observed_runtime = lines.next().unwrap_or_default();
    let args = lines.collect::<Vec<_>>();
    let has_project_arg = args
        .windows(2)
        .any(|window| matches!(window, ["--project" | "--directory", value] if *value == tool_dir.display().to_string()));

    assert_eq!(observed_runtime, "RUNTIME=process");
    assert!(
        observed_cwd == tool_dir.display().to_string() || has_project_arg,
        "launcher should either cd into the tool dir or pass it to uv; cwd={observed_cwd}, args={args:?}",
    );
    assert!(args.contains(&"--locked"));
    assert!(
        args.windows(2)
            .any(|window| matches!(window, ["--extra", "agent-server"]))
    );
    assert!(
        args.windows(2)
            .any(|window| matches!(window, ["--module", "openhands.agent_server"]))
    );
    assert!(
        args.windows(2)
            .any(|window| matches!(window, ["--host", "127.0.0.1"]))
    );
    assert!(
        args.windows(2)
            .any(|window| matches!(window, ["--port", "8123"]))
    );
}

#[test]
fn run_local_launcher_rejects_extra_agent_server_flags() {
    let repo_root = repo_root();
    let tool_dir = repo_root.join("tools/openhands-server");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let log_path = fake_bin_dir.path().join("uv.log");
    let fake_uv = fake_bin_dir.path().join("uv");
    std::fs::write(
        &fake_uv,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf 'unexpected\\n' > \"{}\"\n",
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
        .arg("--debug")
        .current_dir(&repo_root)
        .env("PATH", format!("{}:{path}", fake_bin_dir.path().display()))
        .status()
        .expect("launcher should run");
    assert!(!status.success(), "launcher should reject extra CLI flags");
    assert!(
        !log_path.exists(),
        "launcher should fail before invoking uv when extra flags are passed"
    );
}

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if manifest_dir.join("Cargo.toml").is_file() && manifest_dir.join("README.md").is_file() {
        manifest_dir
    } else {
        manifest_dir
            .parent()
            .expect("crate dir should have workspace parent")
            .parent()
            .expect("workspace root should exist")
            .to_path_buf()
    }
}

fn path_only(path: &std::path::Path) -> OsString {
    std::env::join_paths([path]).expect("path should join")
}

struct FakeCommandPath {
    _dir: TempDir,
    path: OsString,
}

fn fake_command_on_path(command: &str) -> FakeCommandPath {
    let dir = TempDir::new().expect("fake bin dir should be created");
    write_fake_executable(dir.path().join(command));

    let inherited = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![dir.path().to_path_buf()];
    paths.extend(std::env::split_paths(&inherited));

    FakeCommandPath {
        path: std::env::join_paths(paths).expect("path should join"),
        _dir: dir,
    }
}

fn write_fake_executable(path: PathBuf) {
    std::fs::write(&path, "#!/bin/sh\nexit 0\n").expect("fake executable should be written");
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&path)
            .expect("fake executable metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("fake executable should be executable");
    }
}

fn write_bash_wrapper(path: PathBuf) {
    let bash = real_bash_path();
    std::fs::write(
        &path,
        format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", bash.display()),
    )
    .expect("bash wrapper should be written");
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&path)
            .expect("bash wrapper metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("bash wrapper should be executable");
    }
}

fn real_bash_path() -> PathBuf {
    let output = Command::new("bash")
        .args(["-lc", "command -v bash"])
        .output()
        .expect("bash lookup should run");
    assert!(
        output.status.success(),
        "bash lookup should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("bash lookup output should be UTF-8")
            .trim(),
    )
}

fn doctor_workflow_source(workspace_root: &std::path::Path, base_url: &str) -> String {
    format!(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
workspace:
  root: {}
openhands:
  transport:
    base_url: {}
---

# Doctor Probe

Issue: {{{{ issue.identifier }}}}
"#,
        workspace_root.display(),
        base_url,
    )
}
