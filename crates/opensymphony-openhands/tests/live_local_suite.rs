use std::{
    collections::BTreeMap,
    env, fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        Path as AxumPath, Query, State, WebSocketUpgrade,
        ws::{Message as AxumMessage, WebSocket},
    },
    http::{Method, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use opensymphony_domain::{
    IssueId, IssueIdentifier, IssueState, IssueStateCategory, NormalizedIssue, RetryAttempt,
    RunAttempt, TimestampMs, WorkerId, WorkerOutcomeKind,
};
use opensymphony_openhands::{
    ConversationCreateRequest, EventEnvelope, IssueConversationManifest, IssueSessionContext,
    IssueSessionPromptKind, IssueSessionRunner, IssueSessionRunnerConfig, LocalServerSupervisor,
    LocalServerTooling, OpenHandsClient, RuntimeStreamConfig, SendMessageRequest,
    SupervisedServerConfig, SupervisorConfig, TransportConfig,
};
use opensymphony_workflow::{ResolvedWorkflow, WorkflowDefinition};
use opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, RunDescriptor, WorkspaceManager,
    WorkspaceManagerConfig,
};
use serde_json::{Value, json};
use tokio::{
    net::TcpListener as TokioTcpListener,
    sync::Mutex,
    task::JoinHandle,
    time::{Instant, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::Message as TungsteniteMessage};

const LIVE_GATE_ENV: &str = "OPENSYMPHONY_LIVE_OPENHANDS";
const LIVE_MODEL_ENV: &str = "OPENSYMPHONY_OPENHANDS_MODEL";
const LIVE_SUITE_BASE_URL_ENV: &str = "OPENSYMPHONY_LIVE_SUITE_BASE_URL";
const LIVE_SUITE_OUTPUT_DIR_ENV: &str = "OPENSYMPHONY_LIVE_SUITE_OUTPUT_DIR";
const CHECKLIST_PATH: &str = "notes/live-suite-checklist.md";
const FIRST_REPLY_TEXT: &str = "run 1: workspace-created";
const SECOND_REPLY_TEXT: &str = "run 2: conversation-reused";

#[tokio::test]
#[ignore = "requires a prepared local machine with live OpenHands credentials"]
async fn live_local_issue_lifecycle_captures_workspace_artifacts_and_reuses_conversation() {
    if env::var(LIVE_GATE_ENV).as_deref() != Ok("1") {
        eprintln!("skipping live local suite; set {LIVE_GATE_ENV}=1 to enable it");
        return;
    }

    let artifacts = ArtifactLayout::for_scenario("lifecycle");
    let server = LiveServer::acquire();
    let summary_path = artifacts.scenario_root.join("summary.json");

    match run_lifecycle_scenario(&artifacts, &server).await {
        Ok(summary) => write_json(&summary_path, &summary),
        Err(error) => {
            write_json(
                &summary_path,
                &json!({
                    "status": "failed",
                    "error": error,
                    "scenario_root": artifacts.scenario_root.display().to_string(),
                }),
            );
            panic!("{error}");
        }
    }
}

#[tokio::test]
#[ignore = "requires a prepared local machine with live OpenHands credentials"]
async fn live_local_runtime_stream_recovers_after_one_fault_injected_websocket_drop() {
    if env::var(LIVE_GATE_ENV).as_deref() != Ok("1") {
        eprintln!("skipping live local suite; set {LIVE_GATE_ENV}=1 to enable it");
        return;
    }

    let artifacts = ArtifactLayout::for_scenario("reconnect");
    let server = LiveServer::acquire();
    let summary_path = artifacts.scenario_root.join("summary.json");

    match run_reconnect_scenario(&artifacts, &server).await {
        Ok(summary) => write_json(&summary_path, &summary),
        Err(error) => {
            write_json(
                &summary_path,
                &json!({
                    "status": "failed",
                    "error": error,
                    "scenario_root": artifacts.scenario_root.display().to_string(),
                }),
            );
            panic!("{error}");
        }
    }
}

#[derive(Debug)]
struct ArtifactLayout {
    output_root: PathBuf,
    scenario_root: PathBuf,
}

impl ArtifactLayout {
    fn for_scenario(name: &str) -> Self {
        let output_root = env::var_os(LIVE_SUITE_OUTPUT_DIR_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(default_output_root);
        let scenario_root = output_root.join(name);
        fs::create_dir_all(&scenario_root).expect("scenario artifact directory should exist");
        Self {
            output_root,
            scenario_root,
        }
    }
}

fn default_output_root() -> PathBuf {
    let repo_root = repo_root();
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let root = repo_root.join("target").join("live-local").join(timestamp);
    fs::create_dir_all(&root).expect("default live-local artifact root should exist");
    root
}

enum LiveServer {
    External {
        base_url: String,
    },
    Managed {
        base_url: String,
        supervisor: Box<LocalServerSupervisor>,
    },
}

impl LiveServer {
    fn acquire() -> Self {
        if let Ok(base_url) = env::var(LIVE_SUITE_BASE_URL_ENV) {
            return Self::External { base_url };
        }

        let tooling = LocalServerTooling::load(repo_root().join("tools/openhands-server"))
            .expect("pinned OpenHands tooling should load");
        let mut config = SupervisedServerConfig::new(tooling);
        config.port_override = Some(free_port());
        let mut supervisor =
            LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
        let status = supervisor
            .start()
            .expect("live suite should start a managed local server");

        Self::Managed {
            base_url: status.base_url,
            supervisor: Box::new(supervisor),
        }
    }

    fn base_url(&self) -> &str {
        match self {
            Self::External { base_url } => base_url,
            Self::Managed { base_url, .. } => base_url,
        }
    }

    fn mode(&self) -> &'static str {
        match self {
            Self::External { .. } => "external",
            Self::Managed { .. } => "managed",
        }
    }
}

impl Drop for LiveServer {
    fn drop(&mut self) {
        if let Self::Managed { supervisor, .. } = self {
            let _ = supervisor.stop();
        }
    }
}

async fn run_lifecycle_scenario(
    artifacts: &ArtifactLayout,
    server: &LiveServer,
) -> Result<Value, String> {
    let model = env::var(LIVE_MODEL_ENV)
        .map_err(|_| format!("missing required environment variable {LIVE_MODEL_ENV}"))?;
    let target_repo = artifacts.scenario_root.join("target-repo");
    let workspace_root = artifacts.scenario_root.join("workspaces");
    let workflow_path = seed_live_target_repo(&target_repo, &workspace_root, server.base_url())?;
    let workflow = resolve_workflow(&workflow_path, &model)?;
    let manager = workspace_manager(&workflow)?;
    let transport = TransportConfig::from_workflow(&workflow, &workflow_env(&model))
        .map_err(|error| format!("failed to resolve transport config from workflow: {error}"))?;
    let client = OpenHandsClient::new(transport);
    let runner = IssueSessionRunner::new(client.clone(), runner_config(&workflow));
    let issue = sample_issue();
    let ensured = manager
        .ensure(&issue_descriptor(&issue))
        .await
        .map_err(|error| format!("failed to ensure workspace: {error}"))?;
    let max_turns = u32::try_from(workflow.config.agent.max_turns)
        .map_err(|_| "workflow max_turns does not fit u32".to_string())?;

    let mut first_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("live-run-1", 1))
        .await
        .map_err(|error| format!("failed to prepare first run: {error}"))?;
    let first_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut first_manifest,
            &issue,
            &run_attempt(
                &issue,
                ensured.handle.workspace_path(),
                "worker-1",
                None,
                max_turns,
            ),
            &workflow,
        )
        .await
        .map_err(|error| format!("first issue session run failed: {error}"))?;

    if first_result.prompt_kind != IssueSessionPromptKind::Full {
        return Err(format!(
            "first run should use the full prompt, got {:?}",
            first_result.prompt_kind
        ));
    }
    if first_result.worker_outcome.outcome != WorkerOutcomeKind::Succeeded {
        return Err(format!(
            "first run should succeed, got {:?}",
            first_result.worker_outcome.outcome
        ));
    }

    let first_conversation = read_conversation_manifest(&manager, &ensured.handle).await?;
    let first_context = read_session_context(&manager, &ensured.handle).await?;
    let first_message_texts = latest_reply_texts(
        client
            .search_all_events(
                uuid::Uuid::parse_str(
                    &first_result
                        .conversation
                        .as_ref()
                        .ok_or_else(|| {
                            "first run did not persist conversation metadata".to_string()
                        })?
                        .conversation_id
                        .to_string(),
                )
                .map_err(|error| format!("first conversation ID should parse: {error}"))?,
            )
            .await
            .map_err(|error| format!("failed to fetch first-run events: {error}"))?
            .items(),
    );
    if !first_message_texts
        .iter()
        .any(|text| text == FIRST_REPLY_TEXT)
    {
        return Err(format!(
            "first run did not record the expected assistant reply: {first_message_texts:?}"
        ));
    }

    let mut second_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("live-run-2", 2))
        .await
        .map_err(|error| format!("failed to prepare second run: {error}"))?;
    let second_result = runner
        .run(
            &manager,
            &ensured.handle,
            &mut second_manifest,
            &issue,
            &run_attempt(
                &issue,
                ensured.handle.workspace_path(),
                "worker-2",
                Some(RetryAttempt::new(2).expect("retry attempt should be valid")),
                max_turns,
            ),
            &workflow,
        )
        .await
        .map_err(|error| format!("second issue session run failed: {error}"))?;

    if second_result.prompt_kind != IssueSessionPromptKind::Continuation {
        return Err(format!(
            "second run should use continuation guidance, got {:?}",
            second_result.prompt_kind
        ));
    }
    if second_result.worker_outcome.outcome != WorkerOutcomeKind::Succeeded {
        return Err(format!(
            "second run should succeed, got {:?}",
            second_result.worker_outcome.outcome
        ));
    }

    let first_conversation_id = first_result
        .conversation
        .as_ref()
        .ok_or_else(|| "first run did not persist conversation metadata".to_string())?
        .conversation_id
        .to_string();
    let second_conversation_id = second_result
        .conversation
        .as_ref()
        .ok_or_else(|| "second run did not persist conversation metadata".to_string())?
        .conversation_id
        .to_string();
    if first_conversation_id != second_conversation_id {
        return Err(format!(
            "conversation ID was not reused: {first_conversation_id} != {second_conversation_id}"
        ));
    }

    let second_conversation = read_conversation_manifest(&manager, &ensured.handle).await?;
    let second_context = read_session_context(&manager, &ensured.handle).await?;
    let all_events = client
        .search_all_events(
            uuid::Uuid::parse_str(&second_conversation_id)
                .map_err(|error| format!("conversation ID should parse: {error}"))?,
        )
        .await
        .map_err(|error| format!("failed to fetch conversation events: {error}"))?;
    let message_texts = latest_reply_texts(all_events.items());
    if !message_texts.iter().any(|text| text == FIRST_REPLY_TEXT) {
        return Err(format!(
            "reused conversation lost the first assistant reply: {message_texts:?}"
        ));
    }
    if message_texts.last().map(String::as_str) != Some(SECOND_REPLY_TEXT) {
        return Err(format!(
            "second run did not record the expected continuation reply: {message_texts:?}"
        ));
    }

    let required_artifacts = vec![
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/issue.json"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/run.json"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/conversation.json"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/openhands/create-conversation-request.json"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/generated/session-context.json"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/prompts/last-full-prompt.md"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/prompts/last-continuation-prompt.md"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/logs/git-status-before.txt"),
        ensured
            .handle
            .workspace_path()
            .join(".opensymphony/logs/git-status-after.txt"),
        ensured.handle.workspace_path().join("AGENTS.md"),
        ensured.handle.workspace_path().join("WORKFLOW.md"),
        ensured.handle.workspace_path().join(CHECKLIST_PATH),
    ];
    for artifact in &required_artifacts {
        if !artifact.exists() {
            return Err(format!(
                "expected live-suite artifact is missing: {}",
                artifact.display()
            ));
        }
    }

    Ok(json!({
        "status": "passed",
        "scenario": "lifecycle",
        "output_root": artifacts.output_root.display().to_string(),
        "server": {
            "mode": server.mode(),
            "base_url": server.base_url(),
        },
        "workspace_path": ensured.handle.workspace_path().display().to_string(),
        "workflow_path": workflow_path.display().to_string(),
        "conversation_id": second_conversation_id,
        "first_run": {
            "prompt_kind": first_result.prompt_kind.as_str(),
            "run_status": format!("{:?}", first_result.run_status),
            "assistant_reply": FIRST_REPLY_TEXT,
            "conversation_manifest": summarize_conversation_manifest(&first_conversation),
            "session_context": summarize_session_context(&first_context),
        },
        "second_run": {
            "prompt_kind": second_result.prompt_kind.as_str(),
            "run_status": format!("{:?}", second_result.run_status),
            "assistant_reply": SECOND_REPLY_TEXT,
            "conversation_manifest": summarize_conversation_manifest(&second_conversation),
            "session_context": summarize_session_context(&second_context),
        },
        "message_texts": message_texts,
        "artifacts": required_artifacts
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
    }))
}

async fn run_reconnect_scenario(
    artifacts: &ArtifactLayout,
    server: &LiveServer,
) -> Result<Value, String> {
    let model = env::var(LIVE_MODEL_ENV)
        .map_err(|_| format!("missing required environment variable {LIVE_MODEL_ENV}"))?;
    let proxy =
        FaultInjectingProxy::start(server.base_url(), artifacts.scenario_root.join("proxy.log"))
            .await?;

    let working_dir = artifacts.scenario_root.join("workspace");
    let persistence_dir = working_dir.join(".opensymphony/openhands");
    fs::create_dir_all(&persistence_dir)
        .map_err(|error| format!("failed to create reconnect workspace: {error}"))?;

    let client = OpenHandsClient::new(TransportConfig::new(proxy.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        working_dir.display().to_string(),
        persistence_dir.display().to_string(),
        Some(model),
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .map_err(|error| format!("failed to create reconnect conversation: {error}"))?;
    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(15),
                reconnect_initial_backoff: Duration::from_millis(100),
                reconnect_max_backoff: Duration::from_millis(400),
                max_reconnect_attempts: 5,
            },
        )
        .await
        .map_err(|error| format!("failed to attach reconnect runtime stream: {error}"))?;

    client
        .send_message(
            conversation.conversation_id,
            &SendMessageRequest::user_text(
                "This is the OpenSymphony live reconnect probe. Reply with the exact text `OpenSymphony reconnect probe OK` and then finish.",
            ),
        )
        .await
        .map_err(|error| format!("failed to send reconnect probe message: {error}"))?;
    client
        .run_conversation(conversation.conversation_id)
        .await
        .map_err(|error| format!("failed to trigger reconnect probe run: {error}"))?;

    let deadline = Instant::now() + Duration::from_secs(240);
    let mut observed_events = Vec::new();
    while stream.state_mirror().terminal_status().is_none() {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err("timed out waiting for reconnect probe to finish".to_string());
        };
        let event = timeout(remaining, stream.next_event())
            .await
            .map_err(|_| "timed out waiting for the next reconnect event".to_string())?
            .map_err(|error| format!("reconnect probe stream failed: {error}"))?;
        match event {
            Some(event) => observed_events.push(json!({
                "id": event.id,
                "kind": event.kind,
                "timestamp": event.timestamp.to_rfc3339(),
            })),
            None if stream.state_mirror().terminal_status().is_some() => break,
            None => {
                return Err(
                    "runtime stream ended before a terminal status was observed".to_string()
                );
            }
        }
    }

    let all_events = client
        .search_all_events(conversation.conversation_id)
        .await
        .map_err(|error| format!("failed to search reconnect events: {error}"))?;
    let message_texts = latest_reply_texts(all_events.items());
    let assistant_reply_found = message_texts
        .iter()
        .any(|text| text.contains("OpenSymphony reconnect probe OK"));
    if !assistant_reply_found {
        return Err(format!(
            "reconnect probe did not find the expected assistant reply in message events: {message_texts:?}"
        ));
    }

    let proxy_summary = proxy.summary().await;
    if proxy_summary.websocket_connections < 2 || proxy_summary.dropped_connections != 1 {
        return Err(format!(
            "expected one injected reconnect with at least two websocket connections, got {proxy_summary:?}"
        ));
    }

    Ok(json!({
        "status": "passed",
        "scenario": "reconnect",
        "output_root": artifacts.output_root.display().to_string(),
        "server": {
            "mode": server.mode(),
            "base_url": server.base_url(),
        },
        "proxy": {
            "base_url": proxy.base_url(),
            "websocket_connections": proxy_summary.websocket_connections,
            "dropped_connections": proxy_summary.dropped_connections,
            "log_path": proxy.log_path.display().to_string(),
            "log": proxy_summary.log,
        },
        "conversation_id": conversation.conversation_id.to_string(),
        "terminal_status": format!("{:?}", stream.state_mirror().terminal_status()),
        "execution_status": stream.state_mirror().execution_status(),
        "assistant_reply_found": assistant_reply_found,
        "observed_events": observed_events,
        "message_texts": message_texts,
        "artifacts": [
            working_dir.display().to_string(),
            persistence_dir.display().to_string(),
            proxy.log_path.display().to_string(),
        ],
    }))
}

fn seed_live_target_repo(
    target_repo: &Path,
    workspace_root: &Path,
    base_url: &str,
) -> Result<PathBuf, String> {
    if target_repo.exists() {
        fs::remove_dir_all(target_repo)
            .map_err(|error| format!("failed to remove stale target repo: {error}"))?;
    }
    fs::create_dir_all(target_repo.join("notes"))
        .map_err(|error| format!("failed to create target repo notes directory: {error}"))?;
    fs::create_dir_all(workspace_root)
        .map_err(|error| format!("failed to create workspace root: {error}"))?;

    let workflow_path = target_repo.join("WORKFLOW.md");
    fs::write(
        target_repo.join("README.md"),
        "# Live Suite Target Repo\n\nThis fixture is generated by the live local suite.\n",
    )
    .map_err(|error| format!("failed to write target repo README: {error}"))?;
    fs::write(
        target_repo.join("AGENTS.md"),
        "# Live Suite Target Repo\n\n- Follow `WORKFLOW.md` first.\n- Keep edits limited to the repository checkout inside the issue workspace.\n- Leave `.opensymphony/` artifacts available for debugging.\n",
    )
    .map_err(|error| format!("failed to write target repo AGENTS: {error}"))?;
    fs::write(target_repo.join(".gitignore"), ".opensymphony/\n")
        .map_err(|error| format!("failed to write target repo .gitignore: {error}"))?;
    fs::write(
        target_repo.join(CHECKLIST_PATH),
        "# Live Suite Checklist\n\n- First worker lifetime expects the assistant reply `run 1: workspace-created`\n- Second worker lifetime expects the assistant reply `run 2: conversation-reused`\n",
    )
    .map_err(|error| format!("failed to write live-suite checklist: {error}"))?;
    fs::write(
        &workflow_path,
        workflow_source(target_repo, workspace_root, base_url),
    )
    .map_err(|error| format!("failed to write live-suite workflow: {error}"))?;

    run_git(target_repo, ["init", "--quiet"])?;
    run_git(
        target_repo,
        [
            "config",
            "user.email",
            "opensymphony-live-suite@example.invalid",
        ],
    )?;
    run_git(
        target_repo,
        ["config", "user.name", "OpenSymphony Live Suite"],
    )?;
    run_git(target_repo, ["add", "."])?;
    run_git(
        target_repo,
        ["commit", "--quiet", "-m", "Seed live suite target repo"],
    )?;

    Ok(workflow_path)
}

fn workflow_source(target_repo: &Path, workspace_root: &Path, base_url: &str) -> String {
    let clone_source = shell_quote(target_repo.display().to_string());
    format!(
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
hooks:
  after_create: |
    git clone --quiet --no-local {} .
  before_run: |
    git status --short > .opensymphony/logs/git-status-before.txt
  after_run: |
    git status --short > .opensymphony/logs/git-status-after.txt
openhands:
  transport:
    base_url: {}
  conversation:
    max_iterations: 6
    agent:
      llm:
        model: ${{OPENSYMPHONY_OPENHANDS_MODEL}}
---
# Live Suite Workflow

Operate only inside this repository checkout.
Do not use external tools or modify repository files.
This live suite expects two worker lifetimes on the same conversation.
If the conversation does not already contain the exact assistant reply `{}`, respond with exactly `{}` and then finish.
If the conversation already contains `{}` but does not yet contain `{}`, respond with exactly `{}` and then finish.
Do not add any extra words before or after the required reply.

Issue: {{{{ issue.identifier }}}}
Title: {{{{ issue.title }}}}
Description:
{{{{ issue.description }}}}
"#,
        workspace_root.display(),
        clone_source,
        base_url,
        FIRST_REPLY_TEXT,
        FIRST_REPLY_TEXT,
        FIRST_REPLY_TEXT,
        SECOND_REPLY_TEXT,
        SECOND_REPLY_TEXT,
    )
}

fn resolve_workflow(path: &Path, model: &str) -> Result<ResolvedWorkflow, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("failed to read workflow {}: {error}", path.display()))?;
    let workflow = WorkflowDefinition::parse(&source)
        .map_err(|error| format!("failed to parse workflow {}: {error}", path.display()))?;
    workflow
        .resolve(
            path.parent()
                .ok_or_else(|| format!("workflow has no parent directory: {}", path.display()))?,
            &workflow_env(model),
        )
        .map_err(|error| format!("failed to resolve workflow {}: {error}", path.display()))
}

fn workflow_env(model: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("LINEAR_API_KEY".to_string(), "linear-token".to_string()),
        (LIVE_MODEL_ENV.to_string(), model.to_string()),
    ])
}

fn workspace_manager(workflow: &ResolvedWorkflow) -> Result<WorkspaceManager, String> {
    WorkspaceManager::new(WorkspaceManagerConfig {
        root: workflow.config.workspace.root.clone(),
        hooks: HookConfig {
            after_create: workflow
                .config
                .hooks
                .after_create
                .clone()
                .map(HookDefinition::shell),
            before_run: workflow
                .config
                .hooks
                .before_run
                .clone()
                .map(HookDefinition::shell),
            after_run: workflow
                .config
                .hooks
                .after_run
                .clone()
                .map(HookDefinition::shell),
            before_remove: workflow
                .config
                .hooks
                .before_remove
                .clone()
                .map(HookDefinition::shell),
            timeout: Duration::from_millis(workflow.config.hooks.timeout_ms),
        },
        cleanup: CleanupConfig::default(),
    })
    .map_err(|error| format!("failed to build workspace manager: {error}"))
}

fn sample_issue() -> NormalizedIssue {
    NormalizedIssue {
        id: IssueId::new("issue-live-suite").expect("issue ID should be valid"),
        identifier: IssueIdentifier::new("COE-LIVE-273").expect("issue identifier should be valid"),
        title: "Complete the next live-suite checklist item".to_string(),
        description: Some(
            "Open `notes/live-suite-checklist.md`, complete exactly one unchecked item, update the checklist, and stop.".to_string(),
        ),
        priority: Some(1),
        state: IssueState {
            id: None,
            name: "In Progress".to_string(),
            category: IssueStateCategory::Active,
        },
        branch_name: None,
        url: None,
        labels: vec!["live-suite".to_string()],
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

fn runner_config(workflow: &ResolvedWorkflow) -> IssueSessionRunnerConfig {
    let mut config = IssueSessionRunnerConfig::from_workflow(workflow);
    config.runtime_stream.readiness_timeout = Duration::from_secs(20);
    config.runtime_stream.reconnect_initial_backoff = Duration::from_millis(150);
    config.runtime_stream.reconnect_max_backoff = Duration::from_millis(600);
    config.runtime_stream.max_reconnect_attempts = 5;
    config.terminal_wait_timeout = Duration::from_secs(90);
    config.finished_drain_timeout = Duration::from_millis(500);
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

async fn read_conversation_manifest(
    manager: &WorkspaceManager,
    handle: &opensymphony_workspace::WorkspaceHandle,
) -> Result<IssueConversationManifest, String> {
    let raw = manager
        .read_text_artifact(handle, &handle.conversation_manifest_path())
        .await
        .map_err(|error| format!("failed to read conversation manifest: {error}"))?
        .ok_or_else(|| "conversation manifest should exist".to_string())?;
    serde_json::from_str(&raw)
        .map_err(|error| format!("failed to decode conversation manifest: {error}"))
}

async fn read_session_context(
    manager: &WorkspaceManager,
    handle: &opensymphony_workspace::WorkspaceHandle,
) -> Result<IssueSessionContext, String> {
    let raw = manager
        .read_text_artifact(handle, &handle.generated_dir().join("session-context.json"))
        .await
        .map_err(|error| format!("failed to read session context: {error}"))?
        .ok_or_else(|| "session context should exist".to_string())?;
    serde_json::from_str(&raw).map_err(|error| format!("failed to decode session context: {error}"))
}

fn summarize_conversation_manifest(manifest: &IssueConversationManifest) -> Value {
    json!({
        "conversation_id": manifest.conversation_id.to_string(),
        "fresh_conversation": manifest.fresh_conversation,
        "workflow_prompt_seeded": manifest.workflow_prompt_seeded,
        "last_prompt_kind": manifest.last_prompt_kind.map(|kind| kind.as_str()),
        "last_execution_status": manifest.last_execution_status,
        "last_event_id": manifest.last_event_id,
        "last_event_kind": manifest.last_event_kind,
    })
}

fn summarize_session_context(context: &IssueSessionContext) -> Value {
    json!({
        "run_id": context.run_id,
        "prompt_kind": context.prompt_kind.as_str(),
        "conversation_id": context.conversation_id.to_string(),
        "fresh_conversation": context.fresh_conversation,
        "workflow_prompt_seeded": context.workflow_prompt_seeded,
        "last_execution_status": context.last_execution_status,
        "last_event_id": context.last_event_id,
        "last_event_kind": context.last_event_kind,
    })
}

fn latest_reply_texts(events: &[EventEnvelope]) -> Vec<String> {
    events.iter().filter_map(extract_reply_text).collect()
}

fn extract_reply_text(event: &EventEnvelope) -> Option<String> {
    match event.kind.as_str() {
        "MessageEvent" => {
            let content = event
                .payload
                .get("llm_message")
                .and_then(|message| message.get("content"))
                .or_else(|| event.payload.get("content"))?
                .as_array()?;
            let entry = content.first()?;
            entry.get("text")?.as_str().map(ToOwned::to_owned)
        }
        "ActionEvent" => event
            .payload
            .get("action")
            .and_then(|action| action.get("message"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        "ObservationEvent" => {
            let content = event
                .payload
                .get("observation")
                .and_then(|observation| observation.get("content"))?
                .as_array()?;
            let entry = content.first()?;
            entry.get("text")?.as_str().map(ToOwned::to_owned)
        }
        _ => None,
    }
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("artifact JSON parent directory should exist");
    }
    let rendered = serde_json::to_string_pretty(value).expect("artifact JSON should render");
    fs::write(path, format!("{rendered}\n")).expect("artifact JSON should write");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir should have workspace parent")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<(), String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| format!("failed to run git in {}: {error}", cwd.display()))?;
    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "git {:?} failed in {} with status {}: stdout={} stderr={}",
        args,
        cwd.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    ))
}

fn shell_quote(value: String) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[derive(Debug, Default)]
struct ProxyInner {
    websocket_connections: usize,
    dropped_connections: usize,
    drop_injected: bool,
    log: Vec<String>,
}

#[derive(Debug, Clone)]
struct ProxyState {
    upstream_base_url: String,
    client: reqwest::Client,
    inner: Arc<Mutex<ProxyInner>>,
}

#[derive(Debug)]
struct ProxySummary {
    websocket_connections: usize,
    dropped_connections: usize,
    log: Vec<String>,
}

struct FaultInjectingProxy {
    base_url: String,
    inner: Arc<Mutex<ProxyInner>>,
    task: JoinHandle<()>,
    log_path: PathBuf,
}

impl FaultInjectingProxy {
    async fn start(upstream_base_url: &str, log_path: PathBuf) -> Result<Self, String> {
        let listener = TokioTcpListener::bind(("127.0.0.1", free_port()))
            .await
            .map_err(|error| format!("failed to bind fault-injecting proxy: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("failed to read fault-injecting proxy address: {error}"))?;
        let inner = Arc::new(Mutex::new(ProxyInner::default()));
        let state = ProxyState {
            upstream_base_url: upstream_base_url.to_string(),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|error| format!("failed to build proxy HTTP client: {error}"))?,
            inner: inner.clone(),
        };

        let app = Router::new()
            .route("/api/conversations", post(proxy_post_conversations))
            .route(
                "/api/conversations/{conversation_id}",
                get(proxy_get_conversation),
            )
            .route(
                "/api/conversations/{conversation_id}/events",
                post(proxy_post_events),
            )
            .route(
                "/api/conversations/{conversation_id}/run",
                post(proxy_post_run),
            )
            .route(
                "/api/conversations/{conversation_id}/events/search",
                get(proxy_get_event_search),
            )
            .route(
                "/sockets/events/{conversation_id}",
                get(proxy_events_socket),
            )
            .with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("fault-injecting proxy should serve until aborted");
        });

        Ok(Self {
            base_url: format!("http://{address}"),
            inner,
            task,
            log_path,
        })
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn summary(&self) -> ProxySummary {
        let inner = self.inner.lock().await;
        write_proxy_log(&self.log_path, &inner.log);
        ProxySummary {
            websocket_connections: inner.websocket_connections,
            dropped_connections: inner.dropped_connections,
            log: inner.log.clone(),
        }
    }
}

impl Drop for FaultInjectingProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn proxy_post_conversations(
    State(state): State<ProxyState>,
    body: Bytes,
) -> impl IntoResponse {
    forward_http(&state, Method::POST, "/api/conversations", None, body).await
}

async fn proxy_get_conversation(
    State(state): State<ProxyState>,
    AxumPath(conversation_id): AxumPath<String>,
) -> impl IntoResponse {
    forward_http(
        &state,
        Method::GET,
        &format!("/api/conversations/{conversation_id}"),
        None,
        Bytes::new(),
    )
    .await
}

async fn proxy_post_events(
    State(state): State<ProxyState>,
    AxumPath(conversation_id): AxumPath<String>,
    body: Bytes,
) -> impl IntoResponse {
    forward_http(
        &state,
        Method::POST,
        &format!("/api/conversations/{conversation_id}/events"),
        None,
        body,
    )
    .await
}

async fn proxy_post_run(
    State(state): State<ProxyState>,
    AxumPath(conversation_id): AxumPath<String>,
    body: Bytes,
) -> impl IntoResponse {
    forward_http(
        &state,
        Method::POST,
        &format!("/api/conversations/{conversation_id}/run"),
        None,
        body,
    )
    .await
}

async fn proxy_get_event_search(
    State(state): State<ProxyState>,
    AxumPath(conversation_id): AxumPath<String>,
    Query(query): Query<BTreeMap<String, String>>,
) -> impl IntoResponse {
    forward_http(
        &state,
        Method::GET,
        &format!("/api/conversations/{conversation_id}/events/search"),
        Some(&query),
        Bytes::new(),
    )
    .await
}

async fn forward_http(
    state: &ProxyState,
    method: Method,
    path: &str,
    query: Option<&BTreeMap<String, String>>,
    body: Bytes,
) -> Response<Body> {
    let mut url = format!("{}{}", state.upstream_base_url, path);
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in query {
            serializer.append_pair(key, value);
        }
        url.push('?');
        url.push_str(&serializer.finish());
    }

    let mut request = state.client.request(method.clone(), &url);
    if !body.is_empty() {
        request = request
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec());
    }

    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(error.to_string()))
                .expect("proxy error response should build");
        }
    };

    let status = upstream.status();
    let headers = upstream.headers().clone();
    let body = match upstream.bytes().await {
        Ok(body) => body,
        Err(error) => {
            return Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .body(Body::from(error.to_string()))
                .expect("proxy body error response should build");
        }
    };

    let mut response = Response::builder().status(status);
    for (name, value) in &headers {
        response = response.header(name, value);
    }
    response
        .body(Body::from(body))
        .expect("proxied HTTP response should build")
}

async fn proxy_events_socket(
    ws: WebSocketUpgrade,
    State(state): State<ProxyState>,
    AxumPath(conversation_id): AxumPath<String>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| proxy_websocket(socket, state, conversation_id))
}

async fn proxy_websocket(socket: WebSocket, state: ProxyState, conversation_id: String) {
    let connection_number = {
        let mut inner = state.inner.lock().await;
        inner.websocket_connections += 1;
        let connection_number = inner.websocket_connections;
        inner
            .log
            .push(format!("connection {connection_number} accepted"));
        connection_number
    };

    let upstream_url = websocket_url(&state.upstream_base_url, &conversation_id);
    let (upstream, _) = match connect_async(&upstream_url).await {
        Ok(upstream) => upstream,
        Err(error) => {
            let mut inner = state.inner.lock().await;
            inner.log.push(format!(
                "connection {connection_number} failed to connect upstream: {error}"
            ));
            return;
        }
    };

    let inject_drop = {
        let mut inner = state.inner.lock().await;
        let inject = !inner.drop_injected;
        if inject {
            inner.drop_injected = true;
            inner.log.push(format!(
                "connection {connection_number} will inject a websocket drop after readiness"
            ));
        } else {
            inner.log.push(format!(
                "connection {connection_number} will pass through normally"
            ));
        }
        inject
    };

    let (mut client_sender, mut client_receiver) = socket.split();
    let (mut upstream_sender, mut upstream_receiver) = upstream.split();

    while let Some(result) = tokio::select! {
        client_message = client_receiver.next() => client_message.map(EitherMessage::Client),
        upstream_message = upstream_receiver.next() => upstream_message.map(EitherMessage::Upstream),
    } {
        match result {
            EitherMessage::Client(Ok(message)) => {
                let Some(message) = axum_to_tungstenite(message) else {
                    continue;
                };
                if upstream_sender.send(message).await.is_err() {
                    break;
                }
            }
            EitherMessage::Client(Err(error)) => {
                let mut inner = state.inner.lock().await;
                inner.log.push(format!(
                    "connection {connection_number} client read error: {error}"
                ));
                break;
            }
            EitherMessage::Upstream(Ok(message)) => {
                let should_drop = inject_drop && should_drop_after_ready(&message);
                let Some(axum_message) = tungstenite_to_axum(message) else {
                    continue;
                };
                if client_sender.send(axum_message).await.is_err() {
                    break;
                }
                if should_drop {
                    let mut inner = state.inner.lock().await;
                    inner.dropped_connections += 1;
                    inner.log.push(format!(
                        "connection {connection_number} dropped after forwarding the readiness frame"
                    ));
                    let _ = client_sender.close().await;
                    let _ = upstream_sender.close().await;
                    break;
                }
            }
            EitherMessage::Upstream(Err(error)) => {
                let mut inner = state.inner.lock().await;
                inner.log.push(format!(
                    "connection {connection_number} upstream read error: {error}"
                ));
                break;
            }
        }
    }
}

enum EitherMessage {
    Client(Result<AxumMessage, axum::Error>),
    Upstream(Result<TungsteniteMessage, tokio_tungstenite::tungstenite::Error>),
}

fn websocket_url(base_url: &str, conversation_id: &str) -> String {
    let url = base_url
        .replace("http://", "ws://")
        .replace("https://", "wss://");
    format!("{url}/sockets/events/{conversation_id}")
}

fn should_drop_after_ready(message: &TungsteniteMessage) -> bool {
    match message {
        TungsteniteMessage::Text(text) => text.contains("ConversationStateUpdateEvent"),
        _ => false,
    }
}

fn axum_to_tungstenite(message: AxumMessage) -> Option<TungsteniteMessage> {
    match message {
        AxumMessage::Text(text) => Some(TungsteniteMessage::Text(text.to_string())),
        AxumMessage::Binary(data) => Some(TungsteniteMessage::Binary(data.to_vec())),
        AxumMessage::Ping(data) => Some(TungsteniteMessage::Ping(data.to_vec())),
        AxumMessage::Pong(data) => Some(TungsteniteMessage::Pong(data.to_vec())),
        AxumMessage::Close(frame) => Some(TungsteniteMessage::Close(frame.map(|frame| {
            tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
    }
}

fn tungstenite_to_axum(message: TungsteniteMessage) -> Option<AxumMessage> {
    match message {
        TungsteniteMessage::Text(text) => Some(AxumMessage::Text(text.to_string().into())),
        TungsteniteMessage::Binary(data) => Some(AxumMessage::Binary(data.into())),
        TungsteniteMessage::Ping(data) => Some(AxumMessage::Ping(data.into())),
        TungsteniteMessage::Pong(data) => Some(AxumMessage::Pong(data.into())),
        TungsteniteMessage::Close(frame) => Some(AxumMessage::Close(frame.map(|frame| {
            axum::extract::ws::CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            }
        }))),
        TungsteniteMessage::Frame(_) => None,
    }
}

fn write_proxy_log(path: &Path, lines: &[String]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("proxy log parent directory should exist");
    }
    let rendered = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    fs::write(path, rendered).expect("proxy log should write");
}
