use std::{process::Stdio, time::Duration};

use chrono::Utc;
use opensymphony_openhands::{
    ConversationLaunchProfile, EventEnvelope, OpenHandsClient, RUNTIME_CONTRACT_VERSION,
    TransportConfig,
};
use opensymphony_testkit::FakeOpenHandsServer;
use opensymphony_workflow::WorkflowDefinition;
use opensymphony_workspace::{
    CleanupConfig, HookConfig, IssueDescriptor, WorkspaceManager, WorkspaceManagerConfig,
};
use serde_json::json;
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, process::Command};
use uuid::Uuid;

#[tokio::test]
async fn debug_resumes_existing_conversation_history_and_sends_follow_up_input() {
    let openhands = FakeOpenHandsServer::start()
        .await
        .expect("fake OpenHands server should start");
    let project = TempDir::new().expect("temp project should exist");
    let workspace_root = project.path().join("var").join("workspaces");

    write_project_files(project.path(), &workspace_root, openhands.base_url());

    let workflow_path = project.path().join("WORKFLOW.md");
    let workflow = WorkflowDefinition::load_from_path(&workflow_path)
        .expect("workflow should load")
        .resolve_with_process_env(project.path())
        .expect("workflow should resolve");
    let manager = WorkspaceManager::new(WorkspaceManagerConfig {
        root: workspace_root.clone(),
        hooks: HookConfig::default(),
        cleanup: CleanupConfig {
            remove_terminal_workspaces: false,
        },
    })
    .expect("workspace manager should build");
    let issue = IssueDescriptor {
        issue_id: "issue-287".to_string(),
        identifier: "COE-287".to_string(),
        title: "Debuggable session".to_string(),
        current_state: "In Progress".to_string(),
        last_seen_tracker_refresh_at: None,
    };
    let ensured = manager
        .ensure(&issue)
        .await
        .expect("workspace should exist");

    let transport = TransportConfig::from_workflow(
        &workflow,
        &std::collections::BTreeMap::from([(
            "OPENHANDS_API_KEY".to_string(),
            "test-openhands-key".to_string(),
        )]),
    )
    .expect("transport should resolve");
    let client = OpenHandsClient::new(transport);
    let launch_profile =
        ConversationLaunchProfile::from_workflow(&workflow).expect("launch profile should build");
    let conversation_id = Uuid::new_v4();
    let request = launch_profile.to_create_request(
        ensured.handle.workspace_path(),
        &ensured.handle.openhands_dir(),
        Some(conversation_id),
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation should be created");
    assert_eq!(conversation.conversation_id, conversation_id);

    openhands
        .insert_event(
            conversation_id,
            EventEnvelope::new(
                "assistant-history",
                Utc::now(),
                "assistant",
                "MessageEvent",
                json!({
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "Earlier implementation rationale" }],
                }),
            ),
        )
        .await
        .expect("assistant history event should insert");
    openhands
        .fail_next_conversation_gets(conversation_id, 1)
        .await
        .expect("first conversation GET should be forced to fail");

    std::fs::write(
        ensured.handle.conversation_manifest_path(),
        serde_json::to_vec_pretty(&json!({
            "issue_id": issue.issue_id,
            "identifier": issue.identifier,
            "conversation_id": conversation_id.to_string(),
            "server_base_url": openhands.base_url(),
            "transport_target": "loopback",
            "http_auth_mode": "header",
            "websocket_auth_mode": "query_param",
            "websocket_query_param_name": "session_api_key",
            "persistence_dir": ensured.handle.openhands_dir(),
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "last_attached_at": Utc::now(),
            "launch_profile": serde_json::to_value(&launch_profile)
                .expect("launch profile JSON should render"),
            "fresh_conversation": false,
            "workflow_prompt_seeded": true,
            "runtime_contract_version": RUNTIME_CONTRACT_VERSION,
        }))
        .expect("conversation manifest JSON should render"),
    )
    .expect("conversation manifest should write");

    let mut child = Command::new(env!("CARGO_BIN_EXE_opensymphony"));
    child
        .arg("debug")
        .arg("COE-287")
        .current_dir(project.path())
        .env("OPENHANDS_API_KEY", "test-openhands-key")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = child.spawn().expect("debug command should spawn");
    let mut stdin = child.stdin.take().expect("debug stdin should exist");
    stdin
        .write_all(b"Why did you implement it this way?\n/exit\n")
        .await
        .expect("debug stdin should accept scripted input");
    drop(stdin);

    let output = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("debug command should finish promptly")
        .expect("debug command output should collect");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "debug command should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("Resumed conversation"),
        "debug command should announce the resumed conversation: stdout={stdout}",
    );
    assert!(
        stdout.contains("Earlier implementation rationale"),
        "debug command should print recent conversation history: stdout={stdout}",
    );
    assert!(
        stdout.contains("debug>"),
        "debug command should expose the interactive prompt: stdout={stdout}",
    );

    let events = client
        .search_all_events(conversation_id)
        .await
        .expect("conversation events should be searchable");
    let user_messages = events
        .items()
        .iter()
        .filter(|event| event.kind == "MessageEvent")
        .filter_map(|event| {
            let role = event.payload.get("role")?.as_str()?;
            if role != "user" {
                return None;
            }
            let content = event.payload.get("content")?.as_array()?;
            let entry = content.first()?;
            entry.get("text")?.as_str().map(ToOwned::to_owned)
        })
        .collect::<Vec<_>>();
    assert!(
        user_messages
            .iter()
            .any(|message| message == "Why did you implement it this way?"),
        "debug command should append the follow-up prompt to the resumed conversation: {user_messages:?}",
    );
}

fn write_project_files(
    project_root: &std::path::Path,
    workspace_root: &std::path::Path,
    openhands_base_url: &str,
) {
    std::fs::create_dir_all(workspace_root).expect("workspace root should exist");
    std::fs::write(
        project_root.join("WORKFLOW.md"),
        format!(
            "---\ntracker:\n  kind: linear\n  endpoint: https://api.linear.app/graphql\n  api_key: test-linear-key\n  project_slug: test-project\n  active_states:\n    - In Progress\n  terminal_states:\n    - Done\nworkspace:\n  root: {}\nopenhands:\n  transport:\n    base_url: {openhands_base_url}\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n# Test Workflow\n\nResume the stored issue conversation.\n",
            workspace_root.display()
        ),
    )
    .expect("workflow should be written");
}
