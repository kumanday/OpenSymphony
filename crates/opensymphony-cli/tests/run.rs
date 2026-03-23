use std::{process::Stdio, time::Duration};

use axum::{Json, Router, routing::post};
use opensymphony_testkit::FakeOpenHandsServer;
use serde_json::json;
use tempfile::TempDir;
use tokio::{
    net::TcpListener,
    process::{Child, Command},
    task::JoinHandle,
    time::{Instant, sleep},
};

#[tokio::test]
async fn run_auto_detects_config_and_workflow_from_project_directory() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!("control_plane:\n  bind: {bind_addr}\n"),
    );

    let mut child = spawn_run_child(project.path(), &[]);

    wait_for_health(&format!("http://{bind_addr}/healthz"))
        .await
        .expect("run command should become healthy from the project directory");

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_config_flag_overrides_auto_detected_config_file() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start().await;
    let project = TempDir::new().expect("temp project should exist");
    let default_bind = reserve_socket_addr();
    let override_bind = reserve_socket_addr();

    write_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!("control_plane:\n  bind: {default_bind}\n"),
    );
    std::fs::write(
        project.path().join("override.yaml"),
        format!("control_plane:\n  bind: {override_bind}\n"),
    )
    .expect("override config should be written");

    let mut child = spawn_run_child(project.path(), &["--config", "override.yaml"]);

    wait_for_health(&format!("http://{override_bind}/healthz"))
        .await
        .expect("explicit --config should control the bind address");
    assert!(
        !health_endpoint_ready(&format!("http://{default_bind}/healthz")).await,
        "default auto-detected config should not be used when --config is passed",
    );

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_accepts_existing_repo_config_shape_with_extra_doctor_fields() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!(
            "target_repo: .\ncontrol_plane:\n  bind: {bind_addr}\nopenhands:\n  probe_model: fake-model\n  probe_api_key_env: FAKE_API_KEY\nlinear:\n  enabled: false\n"
        ),
    );

    let mut child = spawn_run_child(project.path(), &[]);

    wait_for_health(&format!("http://{bind_addr}/healthz"))
        .await
        .expect("run command should ignore doctor-only config fields");

    terminate_child(&mut child).await;
}

fn spawn_run_child(project_root: &std::path::Path, extra_args: &[&str]) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_opensymphony"));
    command
        .arg("run")
        .args(extra_args)
        .current_dir(project_root)
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.spawn().expect("run command should spawn")
}

fn write_project_files(
    project_root: &std::path::Path,
    linear_base_url: &str,
    openhands_base_url: &str,
    config_contents: String,
) {
    std::fs::write(
        project_root.join("WORKFLOW.md"),
        format!(
            "---\ntracker:\n  kind: linear\n  endpoint: {linear_base_url}\n  project_slug: test-project\n  active_states:\n    - In Progress\n  terminal_states:\n    - Done\nworkspace:\n  root: ./var/workspaces\nopenhands:\n  transport:\n    base_url: {openhands_base_url}\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n# Test Workflow\n\nRun the scheduler.\n"
        ),
    )
    .expect("workflow should be written");
    std::fs::write(project_root.join("config.yaml"), config_contents)
        .expect("config should be written");
}

fn reserve_socket_addr() -> std::net::SocketAddr {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("temporary listener should bind");
    let address = listener
        .local_addr()
        .expect("temporary listener should expose its address");
    drop(listener);
    address
}

async fn wait_for_health(url: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if health_endpoint_ready(url).await {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(format!("timed out waiting for {url}"))
}

async fn health_endpoint_ready(url: &str) -> bool {
    match reqwest::Client::new().get(url).send().await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

async fn terminate_child(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

struct MockLinearGraphqlServer {
    base_url: String,
    task: JoinHandle<()>,
}

impl MockLinearGraphqlServer {
    async fn start() -> Self {
        let app = Router::new().route("/graphql", post(handle_graphql));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock Linear listener should bind");
        let address = listener
            .local_addr()
            .expect("mock Linear listener should expose an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock Linear server should run");
        });

        Self {
            base_url: format!("http://{address}/graphql"),
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for MockLinearGraphqlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_graphql() -> Json<serde_json::Value> {
    Json(json!({
        "data": {
            "issues": {
                "nodes": [],
                "pageInfo": {
                    "hasNextPage": false,
                    "endCursor": null
                }
            }
        }
    }))
}
