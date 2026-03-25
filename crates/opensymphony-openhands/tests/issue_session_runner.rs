use std::{collections::BTreeMap, path::Path, time::Duration};

use opensymphony_domain::{
    IssueId, IssueIdentifier, IssueState, IssueStateCategory, NormalizedIssue, RetryAttempt,
    RunAttempt, TimestampMs, WorkerId, WorkerOutcomeKind,
};
use opensymphony_openhands::{
    ConversationCreateRequest, EventEnvelope, IssueConversationManifest, IssueSessionContext,
    IssueSessionPromptKind, IssueSessionRunner, IssueSessionRunnerConfig,
    LLM_SUMMARIZING_CONDENSER_KIND, OpenHandsClient, TransportConfig,
};
use opensymphony_testkit::{FakeOpenHandsConfig, FakeOpenHandsServer};
use opensymphony_workflow::{ResolvedWorkflow, WorkflowDefinition};
use opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, RunDescriptor, WorkspaceManager,
    WorkspaceManagerConfig,
};
use tempfile::TempDir;

fn sample_issue(identifier: &str) -> NormalizedIssue {
    NormalizedIssue {
        id: IssueId::new(format!("issue-{identifier}")).expect("issue ID should be valid"),
        identifier: IssueIdentifier::new(identifier.to_string())
            .expect("issue identifier should be valid"),
        title: format!("Ticket {identifier}"),
        description: Some("Build the issue session runner".to_string()),
        priority: Some(1),
        state: IssueState {
            id: None,
            name: "In Progress".to_string(),
            category: IssueStateCategory::Active,
        },
        branch_name: None,
        url: None,
        labels: vec!["runtime".to_string()],
        parent_id: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: Some(TimestampMs::new(1)),
        updated_at: Some(TimestampMs::new(2)),
    }
}

fn issue_descriptor(issue: &NormalizedIssue) -> IssueDescriptor {
    IssueDescriptor {
        issue_id: issue.id.to_string(),
        identifier: issue.identifier.to_string(),
        title: issue.title.clone(),
        current_state: issue.state.name.clone(),
        last_seen_tracker_refresh_at: None,
    }
}

fn workspace_manager(root: &Path, hooks: HookConfig) -> WorkspaceManager {
    WorkspaceManager::new(WorkspaceManagerConfig {
        root: root.to_path_buf(),
        hooks,
        cleanup: CleanupConfig::default(),
    })
    .expect("workspace manager should build")
}

fn workflow_for(workspace_root: &Path, base_url: &str) -> ResolvedWorkflow {
    workflow_for_with_persistence_dir(workspace_root, base_url, ".opensymphony/openhands")
}

fn workflow_for_with_condenser(workspace_root: &Path, base_url: &str) -> ResolvedWorkflow {
    let workflow = WorkflowDefinition::parse(&format!(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: {}
openhands:
  transport:
    base_url: {}
  conversation:
    agent:
      condenser:
        enabled: true
        max_size: 240
        keep_first: 2
---

# Assignment

Issue: {{{{ issue.identifier }}}}
Title: {{{{ issue.title }}}}
{{% if attempt %}}Attempt: {{{{ attempt }}}}{{% endif %}}
"#,
        workspace_root.display(),
        base_url,
    ))
    .expect("workflow should parse");

    workflow
        .resolve(
            workspace_root,
            &BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
        )
        .expect("workflow should resolve")
}

fn workflow_for_with_persistence_dir(
    workspace_root: &Path,
    base_url: &str,
    persistence_dir_relative: &str,
) -> ResolvedWorkflow {
    let workflow = WorkflowDefinition::parse(&format!(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: {}
openhands:
  transport:
    base_url: {}
  conversation:
    persistence_dir_relative: {}
---

# Assignment

Issue: {{{{ issue.identifier }}}}
Title: {{{{ issue.title }}}}
{{% if attempt %}}Attempt: {{{{ attempt }}}}{{% endif %}}
"#,
        workspace_root.display(),
        base_url,
        persistence_dir_relative,
    ))
    .expect("workflow should parse");

    workflow
        .resolve(
            workspace_root,
            &BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
        )
        .expect("workflow should resolve")
}

fn workflow_with_llm_provider_overrides(
    workspace_root: &Path,
    base_url: &str,
    api_key_env: Option<&str>,
    base_url_env: Option<&str>,
) -> ResolvedWorkflow {
    let api_key_line = api_key_env
        .map(|name| format!("        api_key_env: {name}\n"))
        .unwrap_or_default();
    let base_url_line = base_url_env
        .map(|name| format!("        base_url_env: {name}\n"))
        .unwrap_or_default();
    let workflow = WorkflowDefinition::parse(&format!(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: {}
openhands:
  transport:
    base_url: {}
  conversation:
    agent:
      llm:
        model: openai/gpt-5.4
{api_key_line}{base_url_line}
---

# Assignment

Issue: {{{{ issue.identifier }}}}
"#,
        workspace_root.display(),
        base_url,
    ))
    .expect("workflow should parse");

    workflow
        .resolve(
            workspace_root,
            &BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
        )
        .expect("workflow should resolve")
}

fn workflow_for_with_agent_block(
    workspace_root: &Path,
    base_url: &str,
    agent_block: &str,
) -> ResolvedWorkflow {
    let workflow = WorkflowDefinition::parse(&format!(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: {}
openhands:
  transport:
    base_url: {}
  conversation:
    persistence_dir_relative: .opensymphony/openhands
    agent:
{}
---

# Assignment

Issue: {{{{ issue.identifier }}}}
Title: {{{{ issue.title }}}}
{{% if attempt %}}Attempt: {{{{ attempt }}}}{{% endif %}}
"#,
        workspace_root.display(),
        base_url,
        agent_block
    ))
    .expect("workflow should parse");

    workflow
        .resolve(
            workspace_root,
            &BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
        )
        .expect("workflow should resolve")
}

fn runner_config(workflow: &ResolvedWorkflow) -> IssueSessionRunnerConfig {
    let mut config = IssueSessionRunnerConfig::from_workflow(workflow);
    config.runtime_stream.readiness_timeout = std::time::Duration::from_secs(2);
    config.terminal_wait_timeout = std::time::Duration::from_secs(2);
    config.finished_drain_timeout = std::time::Duration::from_millis(200);
    config
}

fn run_attempt(
    issue: &NormalizedIssue,
    workspace_path: &Path,
    worker_id: &str,
    attempt: Option<RetryAttempt>,
    max_turns: u32,
) -> RunAttempt {
    RunAttempt::new(
        WorkerId::new(worker_id).expect("worker ID should be valid"),
        issue.id.clone(),
        issue.identifier.clone(),
        workspace_path.to_path_buf(),
        TimestampMs::new(10),
        attempt,
        max_turns,
    )
}

fn latest_message_texts(events: &[EventEnvelope]) -> Vec<String> {
    events
        .iter()
        .filter(|event| event.kind == "MessageEvent")
        .filter_map(extract_message_text)
        .collect()
}

fn extract_message_text(event: &EventEnvelope) -> Option<String> {
    let content = event.payload.get("content")?.as_array()?;
    let entry = content.first()?;
    entry.get("text")?.as_str().map(ToOwned::to_owned)
}

async fn read_conversation_manifest(
    manager: &WorkspaceManager,
    handle: &opensymphony_workspace::WorkspaceHandle,
) -> IssueConversationManifest {
    let raw = manager
        .read_text_artifact(handle, &handle.conversation_manifest_path())
        .await
        .expect("conversation manifest should be readable")
        .expect("conversation manifest should exist");
    serde_json::from_str(&raw).expect("conversation manifest should decode")
}

async fn read_session_context(
    manager: &WorkspaceManager,
    handle: &opensymphony_workspace::WorkspaceHandle,
) -> IssueSessionContext {
    let raw = manager
        .read_text_artifact(handle, &handle.generated_dir().join("session-context.json"))
        .await
        .expect("session context should be readable")
        .expect("session context should exist");
    serde_json::from_str(&raw).expect("session context should decode")
}

async fn read_create_conversation_request(
    manager: &WorkspaceManager,
    handle: &opensymphony_workspace::WorkspaceHandle,
) -> ConversationCreateRequest {
    let raw = manager
        .read_text_artifact(
            handle,
            &handle
                .openhands_dir()
                .join("create-conversation-request.json"),
        )
        .await
        .expect("create request should be readable")
        .expect("create request should exist");
    serde_json::from_str(&raw).expect("create request should decode")
}

#[tokio::test]
async fn issue_session_runner_reuses_conversation_and_switches_to_continuation_prompt() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for(&workspace_root, server.base_url());
    let issue = sample_issue("COE-266");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client.clone(), runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut first_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("first run manifest should prepare");
    let first_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-1",
        None,
        max_turns,
    );
    let first_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut first_manifest,
            &issue,
            &first_run,
            &workflow,
        )
        .await
        .expect("first issue session run should succeed");

    assert_eq!(first_result.prompt_kind, IssueSessionPromptKind::Full);
    assert_eq!(
        first_result.run_status,
        opensymphony_workspace::RunStatus::Succeeded
    );
    assert_eq!(
        first_result.worker_outcome.outcome,
        WorkerOutcomeKind::Succeeded
    );
    assert!(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .fresh_conversation
    );
    assert_eq!(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .transport_target
            .as_deref(),
        Some("loopback")
    );
    assert_eq!(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .http_auth_mode
            .as_deref(),
        Some("none")
    );
    assert_eq!(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .websocket_auth_mode
            .as_deref(),
        Some("none")
    );

    let first_conversation = read_conversation_manifest(&manager, &ensured.handle).await;
    assert!(first_conversation.workflow_prompt_seeded);
    assert_eq!(
        first_conversation.last_prompt_kind,
        Some(IssueSessionPromptKind::Full)
    );
    let launch_profile = first_conversation
        .launch_profile
        .as_ref()
        .expect("conversation manifest should persist a launch profile");
    assert_eq!(launch_profile.workspace_kind, "LocalWorkspace");
    assert_eq!(launch_profile.confirmation_policy_kind, "NeverConfirm");
    assert_eq!(launch_profile.agent_kind, "Agent");
    assert_eq!(launch_profile.llm_model, "openai/gpt-5.4");
    assert_eq!(
        launch_profile.agent_tools.as_ref().map(|tools| tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()),
        Some(vec!["TerminalTool", "FileEditorTool"])
    );
    assert_eq!(launch_profile.agent_include_default_tools, None);
    assert!(launch_profile.stuck_detection);
    let first_prompt = manager
        .read_text_artifact(
            &ensured.handle,
            &ensured.handle.prompts_dir().join("last-full-prompt.md"),
        )
        .await
        .expect("full prompt should be readable")
        .expect("full prompt should exist");
    assert!(first_prompt.contains("Issue: COE-266"));

    let conversation_id = uuid::Uuid::parse_str(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .conversation_id
            .as_str(),
    )
    .expect("conversation ID should parse");
    let create_request = read_create_conversation_request(&manager, &ensured.handle).await;
    assert_eq!(
        create_request.agent.tools.as_ref().map(|tools| tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>()),
        Some(vec!["TerminalTool", "FileEditorTool"])
    );
    assert_eq!(create_request.agent.include_default_tools, None);
    let first_messages = latest_message_texts(
        client
            .search_all_events(conversation_id)
            .await
            .expect("events should be searchable")
            .items(),
    );
    assert_eq!(first_messages.len(), 1);
    assert!(first_messages[0].contains("Issue: COE-266"));

    let mut second_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-2", 2))
        .await
        .expect("second run manifest should prepare");
    let second_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-2",
        Some(RetryAttempt::new(2).expect("retry attempt should be valid")),
        max_turns,
    );
    let second_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut second_manifest,
            &issue,
            &second_run,
            &workflow,
        )
        .await
        .expect("second issue session run should succeed");

    assert_eq!(
        second_result.prompt_kind,
        IssueSessionPromptKind::Continuation
    );
    assert_eq!(
        second_result.run_status,
        opensymphony_workspace::RunStatus::Succeeded
    );
    assert_eq!(
        second_result.worker_outcome.outcome,
        WorkerOutcomeKind::Succeeded
    );
    assert_eq!(
        first_result
            .conversation
            .as_ref()
            .expect("first conversation metadata should exist")
            .conversation_id,
        second_result
            .conversation
            .as_ref()
            .expect("second conversation metadata should exist")
            .conversation_id
    );
    assert!(
        !second_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .fresh_conversation
    );

    let second_conversation = read_conversation_manifest(&manager, &ensured.handle).await;
    assert_eq!(
        second_conversation.last_prompt_kind,
        Some(IssueSessionPromptKind::Continuation)
    );
    let continuation_prompt = manager
        .read_text_artifact(
            &ensured.handle,
            &ensured
                .handle
                .prompts_dir()
                .join("last-continuation-prompt.md"),
        )
        .await
        .expect("continuation prompt should be readable")
        .expect("continuation prompt should exist");
    assert!(continuation_prompt.contains("The original workflow prompt is already present"));
    assert!(!continuation_prompt.contains("# Assignment"));

    let second_messages = latest_message_texts(
        client
            .search_all_events(conversation_id)
            .await
            .expect("events should be searchable after continuation")
            .items(),
    );
    assert_eq!(second_messages.len(), 2);
    assert!(second_messages[1].contains("The original workflow prompt is already present"));

    let session_context = read_session_context(&manager, &ensured.handle).await;
    assert_eq!(
        session_context.prompt_kind,
        IssueSessionPromptKind::Continuation
    );
    assert_eq!(
        session_context.transport_target.as_deref(),
        Some("loopback")
    );
    assert_eq!(session_context.http_auth_mode.as_deref(), Some("none"));
    assert_eq!(session_context.websocket_auth_mode.as_deref(), Some("none"));
    assert_eq!(
        session_context
            .worker_outcome
            .expect("worker outcome should be persisted")
            .outcome,
        WorkerOutcomeKind::Succeeded
    );
}

#[tokio::test]
async fn issue_session_runner_honors_configured_agent_tool_overrides() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for_with_agent_block(
        &workspace_root,
        server.base_url(),
        r#"      tools:
        - name: ReadFileTool
        - name: BrowserToolSet
          params:
            start_url: https://example.com
      include_default_tools:
        - FinishTool
        - ThinkTool"#,
    );
    let issue = sample_issue("COE-293-tools");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client, runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut run_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("run manifest should prepare");
    let run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-tools",
        None,
        max_turns,
    );
    runner
        .run(
            &manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
        )
        .await
        .expect("issue session run should succeed");

    let create_request = read_create_conversation_request(&manager, &ensured.handle).await;
    let tools = create_request
        .agent
        .tools
        .expect("configured tool overrides should be serialized");
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name, "ReadFileTool");
    assert!(tools[0].params.is_empty());
    assert_eq!(tools[1].name, "BrowserToolSet");
    assert_eq!(
        tools[1].params.get("start_url"),
        Some(&serde_json::Value::String(
            "https://example.com".to_string()
        ))
    );
    assert_eq!(
        create_request.agent.include_default_tools,
        Some(vec!["FinishTool".to_string(), "ThinkTool".to_string()])
    );

    let manifest = read_conversation_manifest(&manager, &ensured.handle).await;
    let launch_profile = manifest
        .launch_profile
        .as_ref()
        .expect("conversation manifest should persist a launch profile");
    assert_eq!(
        launch_profile.agent_tools.as_ref().map(|tools| {
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>()
        }),
        Some(vec!["ReadFileTool", "BrowserToolSet"])
    );
    assert_eq!(
        launch_profile.agent_include_default_tools,
        Some(vec!["FinishTool".to_string(), "ThinkTool".to_string()])
    );
}

#[tokio::test]
async fn issue_session_runner_reports_failure_when_current_turn_terminal_error_is_observed() {
    let server = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        run_terminal_status: "error",
        ..FakeOpenHandsConfig::default()
    })
    .await
    .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for(&workspace_root, server.base_url());
    let issue = sample_issue("COE-266-failure");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client.clone(), runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut failing_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-failure", 1))
        .await
        .expect("failing run manifest should prepare");
    let failing_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-failure",
        None,
        max_turns,
    );
    let result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut failing_manifest,
            &issue,
            &failing_run,
            &workflow,
        )
        .await
        .expect("failing session run should return a normalized result");

    assert_eq!(result.prompt_kind, IssueSessionPromptKind::Full);
    assert_eq!(result.run_status, opensymphony_workspace::RunStatus::Failed);
    assert_eq!(result.worker_outcome.outcome, WorkerOutcomeKind::Failed);
    assert!(
        result
            .worker_outcome
            .error
            .as_deref()
            .expect("failure error should be recorded")
            .contains("error")
    );

    let session_context = read_session_context(&manager, &ensured.handle).await;
    let worker_outcome = session_context
        .worker_outcome
        .expect("worker outcome should be persisted");
    assert_eq!(worker_outcome.outcome, WorkerOutcomeKind::Failed);
    assert!(
        worker_outcome
            .error
            .as_deref()
            .expect("failure error should be persisted")
            .contains("error")
    );
}

#[tokio::test]
async fn issue_session_runner_waits_for_an_already_running_turn_before_retrying() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for(&workspace_root, server.base_url());
    let issue = sample_issue("COE-253-running");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client.clone(), runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut first_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("first run manifest should prepare");
    let first_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-1",
        None,
        max_turns,
    );
    let first_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut first_manifest,
            &issue,
            &first_run,
            &workflow,
        )
        .await
        .expect("first issue session run should succeed");
    let conversation_id = uuid::Uuid::parse_str(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .conversation_id
            .as_str(),
    )
    .expect("conversation ID should parse");

    server
        .emit_state_update(conversation_id, "running")
        .await
        .expect("conversation should become active");

    let mut second_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-2", 2))
        .await
        .expect("second run manifest should prepare");
    let second_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-2",
        Some(RetryAttempt::new(2).expect("retry attempt should be valid")),
        max_turns,
    );

    let (second_result, _) = tokio::join!(
        runner.run(
            &manager,
            &ensured.handle,
            &mut second_manifest,
            &issue,
            &second_run,
            &workflow,
        ),
        async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server
                .emit_state_update(conversation_id, "finished")
                .await
                .expect("active turn should finish");
        }
    );
    let second_result = second_result.expect("second issue session run should succeed");
    assert_eq!(
        second_result.prompt_kind,
        IssueSessionPromptKind::Continuation
    );
    assert_eq!(
        second_result.run_status,
        opensymphony_workspace::RunStatus::Succeeded
    );
    assert_eq!(
        second_result.worker_outcome.outcome,
        WorkerOutcomeKind::Succeeded
    );

    let messages = latest_message_texts(
        client
            .search_all_events(conversation_id)
            .await
            .expect("events should be searchable after retry")
            .items(),
    );
    assert_eq!(messages.len(), 2);
    assert!(messages[1].contains("The original workflow prompt is already present"));
}

#[tokio::test]
async fn issue_session_runner_rehydrates_a_missing_conversation_without_downgrading_to_full_prompt()
{
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for(&workspace_root, server.base_url());
    let issue = sample_issue("COE-253-rehydrate");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client.clone(), runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut first_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("first run manifest should prepare");
    let first_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-1",
        None,
        max_turns,
    );
    let first_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut first_manifest,
            &issue,
            &first_run,
            &workflow,
        )
        .await
        .expect("first issue session run should succeed");
    let conversation_id = uuid::Uuid::parse_str(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .conversation_id
            .as_str(),
    )
    .expect("conversation ID should parse");

    server
        .fail_next_conversation_gets(conversation_id, 1)
        .await
        .expect("fake server should fail one fetch");

    let mut second_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-2", 2))
        .await
        .expect("second run manifest should prepare");
    let second_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-2",
        Some(RetryAttempt::new(2).expect("retry attempt should be valid")),
        max_turns,
    );
    let second_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut second_manifest,
            &issue,
            &second_run,
            &workflow,
        )
        .await
        .expect("rehydrated issue session run should succeed");

    assert_eq!(
        second_result.prompt_kind,
        IssueSessionPromptKind::Continuation
    );
    assert_eq!(
        first_result
            .conversation
            .as_ref()
            .expect("first conversation metadata should exist")
            .conversation_id,
        second_result
            .conversation
            .as_ref()
            .expect("second conversation metadata should exist")
            .conversation_id
    );
    assert!(
        !second_result
            .conversation
            .as_ref()
            .expect("second conversation metadata should exist")
            .fresh_conversation
    );

    let create_request = read_create_conversation_request(&manager, &ensured.handle).await;
    assert_eq!(create_request.conversation_id, conversation_id);

    let manifest = read_conversation_manifest(&manager, &ensured.handle).await;
    assert!(manifest.workflow_prompt_seeded);
    assert_eq!(
        manifest.last_prompt_kind,
        Some(IssueSessionPromptKind::Continuation)
    );

    let messages = latest_message_texts(
        client
            .search_all_events(conversation_id)
            .await
            .expect("events should be searchable after rehydrate")
            .items(),
    );
    assert_eq!(messages.len(), 2);
    assert!(messages[1].contains("The original workflow prompt is already present"));
}

#[tokio::test]
async fn issue_session_runner_forwards_configured_condenser_to_create_request() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for_with_condenser(&workspace_root, server.base_url());
    let issue = sample_issue("COE-288");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client, runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("run manifest should prepare");
    let run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-1",
        None,
        max_turns,
    );
    runner
        .run(
            &manager,
            &ensured.handle,
            &mut manifest,
            &issue,
            &run,
            &workflow,
        )
        .await
        .expect("issue session run should succeed");

    let create_request = read_create_conversation_request(&manager, &ensured.handle).await;
    let agent_llm = create_request.agent.llm.clone();
    let condenser = create_request
        .agent
        .condenser
        .as_ref()
        .expect("condenser should be forwarded");
    assert_eq!(condenser.kind, LLM_SUMMARIZING_CONDENSER_KIND);
    assert_eq!(condenser.max_size, 240);
    assert_eq!(condenser.keep_first, 2);
    assert_eq!(&condenser.llm, &agent_llm.with_usage_id("condenser"));

    let persisted_manifest = read_conversation_manifest(&manager, &ensured.handle).await;
    let persisted_condenser = persisted_manifest
        .launch_profile
        .as_ref()
        .and_then(|profile| profile.condenser.as_ref())
        .expect("launch profile should persist condenser settings");
    assert_eq!(persisted_condenser.max_size, 240);
    assert_eq!(persisted_condenser.keep_first, 2);
}

#[tokio::test]
async fn issue_session_runner_uses_the_configured_persistence_dir_for_create_and_reuse() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_for_with_persistence_dir(
        &workspace_root,
        server.base_url(),
        ".opensymphony/runtime-cache",
    );
    let issue = sample_issue("COE-253-persistence");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client.clone(), runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut first_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("first run manifest should prepare");
    let first_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-1",
        None,
        max_turns,
    );
    let first_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut first_manifest,
            &issue,
            &first_run,
            &workflow,
        )
        .await
        .expect("first issue session run should succeed");
    let expected_persistence_dir = ensured
        .handle
        .workspace_path()
        .join(".opensymphony/runtime-cache");
    let first_conversation_id = uuid::Uuid::parse_str(
        first_result
            .conversation
            .as_ref()
            .expect("conversation metadata should exist")
            .conversation_id
            .as_str(),
    )
    .expect("conversation ID should parse");

    let create_request = read_create_conversation_request(&manager, &ensured.handle).await;
    assert_eq!(
        Path::new(&create_request.persistence_dir),
        expected_persistence_dir.as_path()
    );

    let first_conversation = client
        .get_conversation(first_conversation_id)
        .await
        .expect("conversation should be fetchable");
    assert_eq!(
        Path::new(&first_conversation.persistence_dir),
        expected_persistence_dir.as_path()
    );

    let first_manifest_state = read_conversation_manifest(&manager, &ensured.handle).await;
    assert_eq!(
        first_manifest_state.persistence_dir,
        expected_persistence_dir
    );

    let mut second_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-2", 2))
        .await
        .expect("second run manifest should prepare");
    let second_run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-2",
        Some(RetryAttempt::new(2).expect("retry attempt should be valid")),
        max_turns,
    );
    let second_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut second_manifest,
            &issue,
            &second_run,
            &workflow,
        )
        .await
        .expect("second issue session run should succeed");

    assert_eq!(
        second_result.prompt_kind,
        IssueSessionPromptKind::Continuation
    );
    assert_eq!(
        first_result
            .conversation
            .as_ref()
            .expect("first conversation metadata should exist")
            .conversation_id,
        second_result
            .conversation
            .as_ref()
            .expect("second conversation metadata should exist")
            .conversation_id
    );

    let second_manifest_state = read_conversation_manifest(&manager, &ensured.handle).await;
    assert_eq!(
        second_manifest_state.persistence_dir,
        expected_persistence_dir
    );
}

#[tokio::test]
async fn issue_session_runner_forwards_workflow_owned_llm_provider_overrides() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_with_llm_provider_overrides(
        &workspace_root,
        server.base_url(),
        Some("WORKFLOW_OPENHANDS_API_KEY"),
        Some("WORKFLOW_OPENHANDS_BASE_URL"),
    );
    let issue = sample_issue("COE-280-provider");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::with_environment(
        client,
        runner_config(&workflow),
        BTreeMap::from([
            (
                "WORKFLOW_OPENHANDS_API_KEY".to_string(),
                "provider-secret".to_string(),
            ),
            (
                "WORKFLOW_OPENHANDS_BASE_URL".to_string(),
                "https://provider.example.test/v1".to_string(),
            ),
        ]),
    );
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut run_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("provider-run", 1))
        .await
        .expect("run manifest should prepare");
    let run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-provider",
        None,
        max_turns,
    );
    let result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
        )
        .await
        .expect("provider-backed session run should succeed");

    assert_eq!(
        result.run_status,
        opensymphony_workspace::RunStatus::Succeeded
    );
    let create_request = read_create_conversation_request(&manager, &ensured.handle).await;
    assert_eq!(
        create_request.agent.llm.api_key.as_deref(),
        Some("provider-secret")
    );
    assert_eq!(
        create_request.agent.llm.base_url.as_deref(),
        Some("https://provider.example.test/v1")
    );

    let manifest = read_conversation_manifest(&manager, &ensured.handle).await;
    let launch_profile = manifest
        .launch_profile
        .as_ref()
        .expect("launch profile should be persisted");
    assert_eq!(
        launch_profile.llm_api_key_env.as_deref(),
        Some("WORKFLOW_OPENHANDS_API_KEY")
    );
    assert_eq!(
        launch_profile.llm_base_url_env.as_deref(),
        Some("WORKFLOW_OPENHANDS_BASE_URL")
    );
}

#[tokio::test]
async fn issue_session_runner_fails_when_workflow_owned_llm_provider_env_is_missing() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(&workspace_root, HookConfig::default());
    let workflow = workflow_with_llm_provider_overrides(
        &workspace_root,
        server.base_url(),
        Some("WORKFLOW_MISSING_API_KEY"),
        None,
    );
    let issue = sample_issue("COE-280-missing-provider");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner =
        IssueSessionRunner::with_environment(client, runner_config(&workflow), BTreeMap::new());
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    let mut run_manifest = manager
        .start_run(
            &ensured.handle,
            &RunDescriptor::new("missing-provider-run", 1),
        )
        .await
        .expect("run manifest should prepare");
    let run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-missing-provider",
        None,
        max_turns,
    );
    let result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
        )
        .await
        .expect("missing provider env should surface as a failed run");

    assert_eq!(result.run_status, opensymphony_workspace::RunStatus::Failed);
    assert_eq!(result.worker_outcome.outcome, WorkerOutcomeKind::Failed);
    assert!(
        result
            .worker_outcome
            .error
            .as_deref()
            .expect("provider env error should be persisted")
            .contains("WORKFLOW_MISSING_API_KEY")
    );
    assert!(
        manager
            .read_text_artifact(
                &ensured.handle,
                &ensured
                    .handle
                    .openhands_dir()
                    .join("create-conversation-request.json"),
            )
            .await
            .expect("request artifact lookup should succeed")
            .is_none()
    );
}

#[tokio::test]
async fn issue_session_runner_smoke_executes_in_temp_repo_workspace() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = workspace_manager(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell("git init -q .")),
            ..HookConfig::default()
        },
    );
    let workflow = workflow_for(&workspace_root, server.base_url());
    let issue = sample_issue("COE-266-smoke");
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .expect("workspace should exist");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let runner = IssueSessionRunner::new(client, runner_config(&workflow));
    let max_turns = u32::try_from(workflow.config.agent.max_turns).expect("max_turns should fit");

    assert!(
        tokio::fs::try_exists(ensured.handle.workspace_path().join(".git"))
            .await
            .expect("git repo marker should be readable")
    );

    let mut run_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("smoke-run", 1))
        .await
        .expect("smoke run manifest should prepare");
    let run = run_attempt(
        &issue,
        ensured.handle.workspace_path(),
        "worker-smoke",
        None,
        max_turns,
    );
    let result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
        )
        .await
        .expect("smoke run should succeed");

    assert_eq!(
        result.run_status,
        opensymphony_workspace::RunStatus::Succeeded
    );
    assert_eq!(result.worker_outcome.outcome, WorkerOutcomeKind::Succeeded);
    assert!(
        manager
            .read_text_artifact(
                &ensured.handle,
                &ensured.handle.prompts_dir().join("last-full-prompt.md"),
            )
            .await
            .expect("full prompt should be readable")
            .is_some()
    );
}
