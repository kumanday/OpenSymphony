use std::{process::Stdio, time::Duration};

use crate::opensymphony_testkit::{FakeOpenHandsConfig, FakeOpenHandsServer};
use crate::{
    opensymphony_domain::{ConversationId, IssueId, IssueIdentifier},
    opensymphony_openhands::IssueConversationManifest,
    opensymphony_workspace::{
        CleanupConfig, HookConfig, IssueDescriptor, RunDescriptor, RunStatus, WorkspaceManager,
        WorkspaceManagerConfig,
    },
};
use axum::{Json, Router, routing::post};
use chrono::Utc;
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
    let linear = MockLinearGraphqlServer::start_with_active_issue().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files_with_workflow_extra(
        project.path(),
        linear.base_url(),
        "http://127.0.0.1:9",
        format!("control_plane:\n  bind: {bind_addr}\n"),
        r#"  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
routing:
  harness: codex_app_server
  model: gpt-5-codex-test
  model_profile: codex-chatgpt-local-keychain
polling:
  interval_ms: 50
"#,
    );
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &["--dry-run"]);

    wait_for_dry_run_route_decision(&format!("http://{bind_addr}/api/v1/snapshot"))
        .await
        .expect("dry-run route decision should appear in the control snapshot");

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_recovers_human_review_worker_and_interrupts_when_tracker_reports_merging() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start_with_human_review_merging_transition().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();
    let conversation_id = uuid::Uuid::new_v4();

    write_merging_supersede_project_files(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        bind_addr,
    );
    write_memory_config(project.path());
    seed_recovered_human_review_workspace(project.path(), conversation_id).await;
    seed_fake_openhands_conversation(&openhands, conversation_id, project.path()).await;

    let mut child = spawn_run_child(project.path(), &[]);

    let snapshot = wait_for_merging_supersede_event(&format!("http://{bind_addr}/api/v1/snapshot"))
        .await
        .expect("run command should interrupt recovered Human Review polling after Merging");
    print_merging_supersede_evidence(&snapshot);
    let interrupt_requests = openhands.interrupt_request_count(conversation_id).await;
    assert_eq!(
        interrupt_requests, 1,
        "run command should call the OpenHands /interrupt route exactly once"
    );
    println!("fake OpenHands interrupt requests: {interrupt_requests}");

    terminate_child(&mut child).await;
}

#[cfg(unix)]
#[tokio::test]
async fn run_interrupts_active_codex_stdio_worker_when_tracker_reports_merging() {
    let linear = MockLinearGraphqlServer::start_with_human_review_merging_transition().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();
    let fake_codex = project.path().join("fake-codex");
    let log_path = project.path().join("fake-codex.log");

    write_fake_codex_interruptible_child(&fake_codex, &log_path);
    write_codex_merging_supersede_project_files(project.path(), linear.base_url(), bind_addr);
    write_memory_config(project.path());

    let mut child = spawn_run_child_with_codex_bin(
        project.path(),
        &[],
        fake_codex
            .to_str()
            .expect("fake codex path should be utf-8"),
    );

    wait_for_codex_merging_interrupt_ack(&format!("http://{bind_addr}/api/v1/snapshot"))
        .await
        .expect("run command should acknowledge active Codex stdio interrupt after Merging");

    let log = std::fs::read_to_string(&log_path).expect("fake Codex log should exist");
    assert!(log.contains("\"method\":\"turn/interrupt\""));
    assert!(log.contains("\"threadId\":\"fake-thread\""));
    assert!(log.contains("\"turnId\":\"turn-1\""));

    terminate_child(&mut child).await;
}

#[tokio::test]
async fn run_dispatches_gateway_cancel_to_openhands_interrupt() {
    let openhands = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        run_terminal_status: "running",
        ..Default::default()
    })
    .await
    .expect("fake OpenHands server should start");
    let linear = MockLinearGraphqlServer::start_with_active_issue().await;
    let project = TempDir::new().expect("temp project should exist");
    let bind_addr = reserve_socket_addr();

    write_project_files_with_workflow_extra(
        project.path(),
        linear.base_url(),
        openhands.base_url(),
        format!("control_plane:\n  bind: {bind_addr}\nlinear:\n  enabled: false\n"),
        "polling:\n  interval_ms: 50\n",
    );
    write_memory_config(project.path());

    let mut child = spawn_run_child(project.path(), &[]);
    let gateway_base = format!("http://{bind_addr}");
    wait_for_running_issue(&format!("{gateway_base}/api/v1/snapshot"), "COE-429")
        .await
        .expect("run command should expose a running issue before cancel");

    let response = reqwest::Client::new()
        .post(format!("{gateway_base}/api/v1/actions/dispatch"))
        .json(&json!({
            "schema_version": { "major": 1, "minor": 0, "patch": 0 },
            "correlation_id": "corr_cancel_gateway_to_harness",
            "action_kind": "cancel",
            "target_entity": {
                "entity_kind": "run",
                "entity_id": "COE-429"
            },
            "idempotency_key": "cancel-COE-429"
        }))
        .send()
        .await
        .expect("cancel action should be posted");
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    wait_for_openhands_interrupt(&openhands)
        .await
        .expect("gateway cancel should reach fake OpenHands interrupt");

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
        .env_remove("OPENSYMPHONY_HARNESS")
        .env_remove("OPENSYMPHONY_MODEL")
        .env_remove("OPENSYMPHONY_MODEL_PROFILE")
        .env_remove("LLM_MODEL")
        .env_remove("LLM_API_KEY")
        .env_remove("LLM_BASE_URL")
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
    spawn_run_child_configured(project_root, extra_args, None)
}

fn spawn_run_child_with_codex_bin(
    project_root: &std::path::Path,
    extra_args: &[&str],
    codex_bin: &str,
) -> Child {
    spawn_run_child_configured(project_root, extra_args, Some(codex_bin))
}

fn spawn_run_child_configured(
    project_root: &std::path::Path,
    extra_args: &[&str],
    codex_bin: Option<&str>,
) -> Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_opensymphony"));
    command
        .arg("run")
        .args(extra_args)
        .current_dir(project_root)
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .env_remove("OPENSYMPHONY_HARNESS")
        .env_remove("OPENSYMPHONY_MODEL")
        .env_remove("OPENSYMPHONY_MODEL_PROFILE")
        .env_remove("LLM_MODEL")
        .env_remove("LLM_API_KEY")
        .env_remove("LLM_BASE_URL")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(codex_bin) = codex_bin {
        command.env("OPENSYMPHONY_CODEX_BIN", codex_bin);
    }
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

fn write_merging_supersede_project_files(
    project_root: &std::path::Path,
    linear_base_url: &str,
    openhands_base_url: &str,
    bind_addr: std::net::SocketAddr,
) {
    std::fs::write(
        project_root.join("WORKFLOW.md"),
        format!(
            "---\ntracker:\n  kind: linear\n  endpoint: {linear_base_url}\n  project_slug: test-project\n  active_states:\n    - In Progress\n    - Human Review\n    - Merging\n  terminal_states:\n    - Done\nworkspace:\n  root: ./var/workspaces\npolling:\n  interval_ms: 50\nopenhands:\n  transport:\n    base_url: {openhands_base_url}\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n# Test Workflow\n\nRun the scheduler.\n"
        ),
    )
    .expect("workflow should be written");
    std::fs::write(
        project_root.join("config.yaml"),
        format!("control_plane:\n  bind: {bind_addr}\nlinear:\n  enabled: false\n"),
    )
    .expect("config should be written");
}

fn write_codex_merging_supersede_project_files(
    project_root: &std::path::Path,
    linear_base_url: &str,
    bind_addr: std::net::SocketAddr,
) {
    std::fs::write(
        project_root.join("WORKFLOW.md"),
        format!(
            "---\ntracker:\n  kind: linear\n  endpoint: {linear_base_url}\n  project_slug: test-project\n  active_states:\n    - In Progress\n    - Human Review\n    - Merging\n  terminal_states:\n    - Done\nworkspace:\n  root: ./var/workspaces\npolling:\n  interval_ms: 50\nrouting:\n  harness: codex_app_server\n  model: gpt-5-codex-test\n  model_profile: codex-chatgpt-local-keychain\nopenhands:\n  transport:\n    base_url: http://127.0.0.1:9\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n# Test Workflow\n\nRun the scheduler.\n"
        ),
    )
    .expect("workflow should be written");
    std::fs::write(
        project_root.join("config.yaml"),
        format!("control_plane:\n  bind: {bind_addr}\nlinear:\n  enabled: false\n"),
    )
    .expect("config should be written");
}

async fn seed_recovered_human_review_workspace(
    project_root: &std::path::Path,
    conversation_id: uuid::Uuid,
) {
    let workspace_manager = WorkspaceManager::new(WorkspaceManagerConfig {
        root: project_root.join("var/workspaces"),
        hooks: HookConfig::default(),
        cleanup: CleanupConfig::default(),
    })
    .expect("workspace manager should be constructed");
    let ensured = workspace_manager
        .ensure(&IssueDescriptor {
            issue_id: "issue-492".to_string(),
            identifier: "COE-492".to_string(),
            title: "Merging Supersedes Human Review Polling".to_string(),
            current_state: "Human Review".to_string(),
            last_seen_tracker_refresh_at: None,
        })
        .await
        .expect("workspace should be created");
    let mut run_manifest = workspace_manager
        .start_run(
            &ensured.handle,
            &RunDescriptor::new("run-worker-recovered-openhands", 1),
        )
        .await
        .expect("run manifest should be written");
    run_manifest.status = RunStatus::Running;
    workspace_manager
        .write_json_artifact(
            &ensured.handle,
            &ensured.handle.run_manifest_path(),
            &run_manifest,
        )
        .await
        .expect("running run manifest should be persisted");

    let now = Utc::now();
    let conversation_manifest = IssueConversationManifest {
        issue_id: IssueId::new("issue-492").expect("issue id should be valid"),
        identifier: IssueIdentifier::new("COE-492").expect("identifier should be valid"),
        conversation_id: ConversationId::new(conversation_id.to_string())
            .expect("conversation id should be valid"),
        reuse_policy: "per_issue".to_string(),
        server_base_url: Some("http://127.0.0.1".to_string()),
        transport_target: Some("openhands_agent_server".to_string()),
        http_auth_mode: Some("none".to_string()),
        websocket_auth_mode: Some("none".to_string()),
        websocket_query_param_name: None,
        persistence_dir: project_root.join(".openhands"),
        created_at: now,
        updated_at: now,
        last_attached_at: now,
        launch_profile: None,
        llm_config_fingerprint: None,
        fresh_conversation: false,
        workflow_prompt_seeded: true,
        reset_reason: None,
        runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_string()),
        codex_archive_state: None,
        last_turn_id: None,
        active_run_id: None,
        prepared_run_id: None,
        trigger_pending_run_id: None,
        last_prompt_kind: None,
        last_prompt_at: None,
        last_prompt_path: None,
        last_execution_status: Some("running".to_string()),
        last_event_id: None,
        last_event_kind: None,
        last_event_at: None,
        last_event_summary: None,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        last_token_accumulation_at: None,
    };
    workspace_manager
        .write_text_artifact(
            &ensured.handle,
            &ensured.handle.conversation_manifest_path(),
            &serde_json::to_string_pretty(&conversation_manifest)
                .expect("conversation manifest should encode"),
        )
        .await
        .expect("conversation manifest should be written");
}

async fn seed_fake_openhands_conversation(
    openhands: &FakeOpenHandsServer,
    conversation_id: uuid::Uuid,
    project_root: &std::path::Path,
) {
    let response = reqwest::Client::new()
        .post(format!("{}/api/conversations", openhands.base_url()))
        .json(&json!({
            "conversation_id": conversation_id,
            "workspace": {
                "working_dir": project_root.join("var/workspaces/COE-492").display().to_string(),
                "kind": "local"
            },
            "persistence_dir": project_root.join(".openhands").display().to_string(),
            "max_iterations": 4,
            "stuck_detection": true,
            "confirmation_policy": {
                "kind": "NeverConfirm"
            },
            "agent": {
                "kind": "Agent",
                "llm": {
                    "model": "fake-model",
                    "api_key": "test-openhands-key"
                }
            }
        }))
        .send()
        .await
        .expect("fake OpenHands conversation create should send");
    assert!(
        response.status().is_success(),
        "fake OpenHands conversation should be seeded: {}",
        response.status()
    );
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

async fn wait_for_running_issue(url: &str, identifier: &str) -> Result<(), String> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(url).send().await
            && response.status().is_success()
            && let Ok(snapshot) = response.json::<Value>().await
            && issue_runtime_state_visible(&snapshot, identifier, "running")
        {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err(format!("timed out waiting for running issue at {url}"))
}

async fn wait_for_openhands_interrupt(openhands: &FakeOpenHandsServer) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if openhands.total_interrupt_request_count().await == 1 {
            return Ok(());
        }
        sleep(Duration::from_millis(50)).await;
    }
    Err("timed out waiting for fake OpenHands interrupt request".to_string())
}

async fn wait_for_merging_supersede_event(url: &str) -> Result<Value, String> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(url).send().await
            && response.status().is_success()
            && let Ok(snapshot) = response.json::<Value>().await
            && merging_supersede_event_visible(&snapshot)
        {
            return Ok(snapshot);
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err(format!(
        "timed out waiting for Merging supersede event at {url}"
    ))
}

async fn wait_for_codex_merging_interrupt_ack(url: &str) -> Result<Value, String> {
    let client = reqwest::Client::new();
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(url).send().await
            && response.status().is_success()
            && let Ok(snapshot) = response.json::<Value>().await
            && codex_merging_interrupt_ack_visible(&snapshot)
        {
            return Ok(snapshot);
        }
        sleep(Duration::from_millis(250)).await;
    }
    Err(format!(
        "timed out waiting for Codex Merging interrupt acknowledgement at {url}"
    ))
}

fn issue_runtime_state_visible(envelope: &Value, identifier: &str, state: &str) -> bool {
    envelope["snapshot"]["issues"]
        .as_array()
        .and_then(|issues| {
            issues
                .iter()
                .find(|issue| issue["identifier"] == identifier)
        })
        .is_some_and(|issue| issue["runtime_state"] == state)
}

fn print_merging_supersede_evidence(envelope: &Value) {
    if let Some(issue) = envelope["snapshot"]["issues"]
        .as_array()
        .and_then(|issues| issues.iter().find(|issue| issue["identifier"] == "COE-492"))
    {
        let interrupt_events = issue["recent_events"]
            .as_array()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| event["kind"] == "scheduler.interrupt_requested")
                    .count()
            })
            .unwrap_or_default();
        println!(
            "merging supersede snapshot: identifier={} tracker_state={} interrupt_requested_events={} reason=tracker_merging_supersedes_human_review",
            issue["identifier"], issue["tracker_state"], interrupt_events
        );
    }
}

fn merging_supersede_event_visible(envelope: &Value) -> bool {
    envelope["snapshot"]["issues"]
        .as_array()
        .and_then(|issues| issues.iter().find(|issue| issue["identifier"] == "COE-492"))
        .is_some_and(|issue| {
            issue["tracker_state"] == "Merging"
                && issue["recent_events"].as_array().is_some_and(|events| {
                    events.iter().any(|event| {
                        event["kind"] == "scheduler.interrupt_requested"
                            && event["payload"]["reason"]
                                == "tracker_merging_supersedes_human_review"
                    })
                })
        })
}

fn codex_merging_interrupt_ack_visible(envelope: &Value) -> bool {
    envelope["snapshot"]["issues"]
        .as_array()
        .and_then(|issues| issues.iter().find(|issue| issue["identifier"] == "COE-492"))
        .is_some_and(|issue| {
            issue["tracker_state"] == "Merging"
                && issue["transport_target"] == "codex_app_server"
                && issue["recent_events"].as_array().is_some_and(|events| {
                    let requested = events.iter().any(|event| {
                        event["kind"] == "scheduler.interrupt_requested"
                            && event["payload"]["reason"]
                                == "tracker_merging_supersedes_human_review"
                    });
                    let acknowledged = events.iter().any(|event| {
                        event["kind"] == "scheduler.interrupt_acknowledged"
                            && event["summary"]
                                .as_str()
                                .is_some_and(|summary| summary.contains("turn/interrupt"))
                    });
                    requested && acknowledged
                })
        })
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

#[cfg(unix)]
const FAKE_CODEX_SCHEMA: &str = r#"{"$schema":"http://json-schema.org/draft-07/schema#","definitions":{"ClientRequest":{"type":"object","required":["jsonrpc","id","method","params"],"properties":{"jsonrpc":{"const":"2.0"},"id":{"type":"integer"},"method":{"enum":["initialize","thread/start","turn/start","turn/interrupt"]},"params":{"type":"object"}}}}}"#;

#[cfg(unix)]
fn write_fake_codex_interruptible_child(path: &std::path::Path, log_path: &std::path::Path) {
    write_executable(
        path,
        &format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
if [ "${{1:-}}" = "app-server" ] && [ "${{2:-}}" = "generate-json-schema" ]; then
  out_dir="${{4:-}}"
  mkdir -p "$out_dir"
  cat > "$out_dir/codex_app_server_protocol.v2.schemas.json" <<'JSON'
{schema}
JSON
  exit 0
fi
printf 'PWD=%s\n' "$PWD" >> "{log}"
printf 'ARGS=%s\n' "$*" >> "{log}"
while IFS= read -r line; do
  printf 'STDIN=%s\n' "$line" >> "{log}"
  id=$(printf '%s\n' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    *'"method":"thread/start"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"fake-thread"}}}}}}\n' "$id"
      ;;
    *'"method":"turn/start"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"turn":{{"id":"turn-1","items":[],"status":"inProgress"}}}}}}\n' "$id"
      ;;
    *'"method":"turn/interrupt"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"status":"accepted"}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turnId":"turn-1","status":"interrupted"}}}}\n'
      exit 0
      ;;
  esac
done
"#,
            log = log_path.display(),
            schema = FAKE_CODEX_SCHEMA
        ),
    );
}

#[cfg(unix)]
fn write_executable(path: &std::path::Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, contents).expect("fake executable should be written");
    let mut permissions = std::fs::metadata(path)
        .expect("fake executable metadata should load")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("fake executable should be executable");
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
        Self::start_with_mode(if active_issue {
            MockLinearMode::ActiveInProgress
        } else {
            MockLinearMode::Empty
        })
        .await
    }

    async fn start_with_human_review_merging_transition() -> Self {
        Self::start_with_mode(MockLinearMode::HumanReviewThenMerging).await
    }

    async fn start_with_mode(mode: MockLinearMode) -> Self {
        let app = Router::new()
            .route("/graphql", post(handle_graphql))
            .with_state(mode);
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

#[derive(Clone, Copy)]
enum MockLinearMode {
    Empty,
    ActiveInProgress,
    HumanReviewThenMerging,
}

impl Drop for MockLinearGraphqlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_graphql(
    axum::extract::State(mode): axum::extract::State<MockLinearMode>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let query = body["query"].as_str().unwrap_or_default();
    if query.contains("query IssueStatesByIds") {
        let nodes = match mode {
            MockLinearMode::HumanReviewThenMerging => vec![linear_issue_state_node("Merging")],
            MockLinearMode::Empty | MockLinearMode::ActiveInProgress => Vec::new(),
        };
        return Json(json!({
            "data": {
                "issues": {
                    "nodes": nodes,
                    "pageInfo": {
                        "hasNextPage": false,
                        "endCursor": null
                    }
                }
            }
        }));
    }

    let active_query = body["variables"]["stateNames"]
        .as_array()
        .is_some_and(|states| states.iter().any(|state| state == "In Progress"));
    let human_review_query = body["variables"]["stateNames"]
        .as_array()
        .is_some_and(|states| states.iter().any(|state| state == "Human Review"));
    let nodes = match mode {
        MockLinearMode::ActiveInProgress if active_query => vec![linear_issue_node(
            "issue-429",
            "COE-429",
            "Codex approvals and cross-harness routing",
            "In Progress",
        )],
        MockLinearMode::HumanReviewThenMerging if human_review_query => vec![linear_issue_node(
            "issue-492",
            "COE-492",
            "Merging Supersedes Human Review Polling",
            "Human Review",
        )],
        MockLinearMode::Empty
        | MockLinearMode::ActiveInProgress
        | MockLinearMode::HumanReviewThenMerging => Vec::new(),
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

fn linear_issue_node(id: &str, identifier: &str, title: &str, state: &str) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "url": format!("https://linear.app/trilogy-ai-coe/issue/{identifier}/test"),
        "title": title,
        "description": "Run command proof",
        "priority": 1.0,
        "branchName": null,
        "createdAt": "2026-06-21T00:00:00Z",
        "updatedAt": "2026-06-21T00:00:00Z",
        "state": {
            "id": format!("state-{}", state.to_ascii_lowercase().replace(' ', "-")),
            "name": state,
            "type": "started"
        },
        "project": null,
        "parent": null,
        "projectMilestone": null,
        "attachments": {
            "nodes": []
        },
        "children": {
            "nodes": []
        },
        "labels": {
            "nodes": [],
            "pageInfo": {
                "hasNextPage": false,
                "endCursor": null
            }
        },
        "inverseRelations": {
            "nodes": [],
            "pageInfo": {
                "hasNextPage": false,
                "endCursor": null
            }
        }
    })
}

fn linear_issue_state_node(state: &str) -> Value {
    json!({
        "id": "issue-492",
        "identifier": "COE-492",
        "updatedAt": "2026-06-21T00:01:00Z",
        "state": {
            "id": format!("state-{}", state.to_ascii_lowercase().replace(' ', "-")),
            "name": state,
            "type": "started"
        }
    })
}
