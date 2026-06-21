use std::{process::Stdio, time::Duration};

use crate::opensymphony_testkit::FakeOpenHandsServer;
use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
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
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &[]);

    wait_for_health(&format!("http://{bind_addr}/healthz"))
        .await
        .expect("run command should become healthy from the project directory");
    wait_for_http_ok(&format!("http://{bind_addr}/api/v1/capabilities"))
        .await
        .expect("run command should expose gateway capabilities");
    wait_for_http_ok(&format!("http://{bind_addr}/api/v1/dashboard/snapshot"))
        .await
        .expect("run command should expose the dashboard snapshot API");

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
    write_memory_config(project.path());
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
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &[]);

    wait_for_health(&format!("http://{bind_addr}/healthz"))
        .await
        .expect("run command should ignore doctor-only config fields");

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_routing_dry_run_selects_codex_and_emits_route_decision() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start_with_active_issue().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files_with_workflow_extra(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!("control_plane:\n  bind: {bind_addr}\n"),
        r#"routing:
  harness: codex_app_server
  model: gpt-5-codex-test
  model_profile: codex-chatgpt-local-keychain
"#,
    );
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &["--dry-run"]);

    wait_for_dry_run_route_decision(&format!("http://{bind_addr}/api/v1/snapshot"))
        .await
        .expect("dry-run route decision should appear in the control snapshot");

    terminate_child(&mut child).await;
}

#[test]
fn run_fails_with_install_guidance_when_managed_local_tooling_is_missing() {
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();
    std::fs::write(
        project.path().join("WORKFLOW.md"),
        r#"---
tracker:
  kind: linear
  endpoint: http://127.0.0.1:9/graphql
  project_slug: test-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
---

# Test Workflow

Run the scheduler.
"#,
    )
    .expect("workflow should be written");
    std::fs::write(
        project.path().join("config.yaml"),
        format!(
            "control_plane:\n  bind: {bind_addr}\nopenhands:\n  tool_dir: ./managed/openhands-server\nlinear:\n  enabled: false\n"
        ),
    )
    .expect("config should be written");
    write_memory_config(project.path());

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("run")
        .current_dir(project.path())
        .env("LINEAR_API_KEY", "test-linear-key")
        .output()
        .expect("run command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "run should fail when managed-local tooling is missing: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stderr.contains("opensymphony install openhands")
            && stderr.contains("opensymphony doctor --config <path>"),
        "run should explain how to provision the managed-local tooling: stderr={stderr}",
    );
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
    write_project_files_with_workflow_extra(
        project_root,
        linear_base_url,
        openhands_base_url,
        config_contents,
        "",
    );
}

fn write_project_files_with_workflow_extra(
    project_root: &std::path::Path,
    linear_base_url: &str,
    openhands_base_url: &str,
    config_contents: String,
    workflow_extra: &str,
) {
    std::fs::write(
        project_root.join("WORKFLOW.md"),
        format!(
            "---\ntracker:\n  kind: linear\n  endpoint: {linear_base_url}\n  project_slug: test-project\n  active_states:\n    - In Progress\n  terminal_states:\n    - Done\nworkspace:\n  root: ./var/workspaces\nopenhands:\n  transport:\n    base_url: {openhands_base_url}\n    session_api_key_env: OPENHANDS_API_KEY\n{workflow_extra}---\n\n# Test Workflow\n\nRun the scheduler.\n"
        ),
    )
    .expect("workflow should be written");
    std::fs::write(project_root.join("config.yaml"), config_contents)
        .expect("config should be written");
}

fn write_memory_config(project_root: &std::path::Path) {
    let memory_dir = project_root.join(".opensymphony/memory");
    std::fs::create_dir_all(&memory_dir).expect("memory dir should be written");
    std::fs::write(memory_dir.join("memory.yaml"), "areas: {}\n")
        .expect("memory config should be written");
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
    wait_for_http_ok(url).await
}

async fn wait_for_http_ok(url: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if http_endpoint_ready(url).await {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(format!("timed out waiting for {url}"))
}

async fn health_endpoint_ready(url: &str) -> bool {
    http_endpoint_ready(url).await
}

async fn wait_for_dry_run_route_decision(url: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(url).send().await
            && response.status().is_success()
            && let Ok(snapshot) = response.json::<Value>().await
            && route_decision_visible(&snapshot)
        {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(format!(
        "timed out waiting for dry-run route decision at {url}"
    ))
}

fn route_decision_visible(envelope: &Value) -> bool {
    envelope["snapshot"]["issues"]
        .as_array()
        .and_then(|issues| issues.iter().find(|issue| issue["identifier"] == "COE-429"))
        .is_some_and(|issue| {
            issue["transport_target"] == "codex_app_server"
                && issue["recent_events"].as_array().is_some_and(|events| {
                    events.iter().any(|event| {
                        event["kind"] == "routing.decision"
                            && event["payload"]["harness_kind"] == "codex_app_server"
                            && event["payload"]["model"] == "gpt-5-codex-test"
                            && event["payload"]["model_profile"] == "codex-chatgpt-local-keychain"
                    })
                })
        })
}

async fn http_endpoint_ready(url: &str) -> bool {
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
        Self::start_with_active_issue_flag(false).await
    }

    async fn start_with_active_issue() -> Self {
        Self::start_with_active_issue_flag(true).await
    }

    async fn start_with_active_issue_flag(active_issue: bool) -> Self {
        let app = Router::new()
            .route("/graphql", post(handle_graphql))
            .with_state(active_issue);
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

async fn handle_graphql(
    axum::extract::State(active_issue): axum::extract::State<bool>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let active_query = body["variables"]["stateNames"]
        .as_array()
        .is_some_and(|states| states.iter().any(|state| state == "In Progress"));
    let nodes = if active_issue && active_query {
        vec![json!({
            "id": "issue-429",
            "identifier": "COE-429",
            "url": "https://linear.app/trilogy-ai-coe/issue/COE-429/codex-approvals-and-cross-harness-routing",
            "title": "Codex approvals and cross-harness routing",
            "description": "Dry-run route proof",
            "priority": 1.0,
            "createdAt": "2026-06-21T00:00:00Z",
            "updatedAt": "2026-06-21T00:00:00Z",
            "state": {
                "id": "state-started",
                "name": "In Progress",
                "type": "started"
            },
            "parent": null,
            "children": {
                "nodes": []
            },
            "labels": {
                "nodes": []
            },
            "inverseRelations": {
                "nodes": [],
                "pageInfo": {
                    "hasNextPage": false,
                    "endCursor": null
                }
            }
        })]
    } else {
        Vec::new()
    };
    Json(json!({
        "data": {
            "issues": {
                "nodes": nodes,
                "pageInfo": {
                    "hasNextPage": false,
                    "endCursor": null
                }
            }
        }
    }))
}
