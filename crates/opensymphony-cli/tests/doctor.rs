use std::{path::PathBuf, process::ExitCode};

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
