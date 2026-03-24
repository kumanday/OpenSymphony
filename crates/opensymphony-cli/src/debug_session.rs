use std::{
    collections::HashSet,
    env, io,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::Args;
use opensymphony_openhands::{
    ConversationLaunchProfile, EventEnvelope, IssueConversationManifest, IssueSessionRunnerConfig,
    KnownEvent, LocalServerSupervisor, LocalServerTooling, OpenHandsClient, OpenHandsError,
    RuntimeEventStream, SendMessageRequest, SupervisedServerConfig, SupervisorConfig,
    SupervisorError, TerminalExecutionStatus, TransportConfig,
};
use opensymphony_workflow::{ProcessEnvironment, ResolvedWorkflow, WorkflowDefinition};
use opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, WorkspaceError, WorkspaceHandle, WorkspaceManager,
    WorkspaceManagerConfig,
};
use serde::Deserialize;
use thiserror::Error;
use tokio::{fs, time::timeout_at};
use url::Url;
use uuid::Uuid;

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const RECENT_HISTORY_LIMIT: usize = 8;

#[derive(Debug, Args, Clone)]
pub struct DebugArgs {
    #[arg(help = "Linear issue identifier or persisted issue ID to resume")]
    pub issue_id: String,
    #[arg(help = "Runtime config YAML path; defaults to ./config.yaml when present")]
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct DebugConfigFile {
    #[serde(default)]
    target_repo: Option<String>,
    #[serde(default)]
    openhands: DebugOpenHandsConfigFile,
}

#[derive(Debug, Default, Deserialize)]
struct DebugOpenHandsConfigFile {
    #[serde(default)]
    tool_dir: Option<String>,
}

struct DebugRuntimeConfig {
    repo_root: PathBuf,
    workflow: ResolvedWorkflow,
    tool_dir: Option<PathBuf>,
}

#[derive(Debug, Error)]
enum DebugCommandError {
    #[error("failed to determine the current working directory: {0}")]
    CurrentDir(#[source] io::Error),
    #[error("failed to read {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to expand {path}: {detail}")]
    ResolveConfig { path: PathBuf, detail: String },
    #[error("failed to load workflow {path}: {source}")]
    LoadWorkflow {
        path: PathBuf,
        #[source]
        source: opensymphony_workflow::WorkflowLoadError,
    },
    #[error("failed to resolve workflow {path}: {source}")]
    ResolveWorkflow {
        path: PathBuf,
        #[source]
        source: opensymphony_workflow::WorkflowConfigError,
    },
    #[error("failed to create workspace manager: {0}")]
    WorkspaceManager(#[from] WorkspaceError),
    #[error(
        "no managed workspace for issue reference `{issue_reference}` exists under {workspace_root}"
    )]
    WorkspaceNotFound {
        issue_reference: String,
        workspace_root: PathBuf,
    },
    #[error("conversation manifest is missing: {path}")]
    ConversationManifestMissing { path: PathBuf },
    #[error("failed to decode conversation manifest {path}: {source}")]
    DecodeConversationManifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("conversation manifest contains invalid conversation id `{value}`: {source}")]
    InvalidConversationId {
        value: String,
        #[source]
        source: uuid::Error,
    },
    #[error("failed to build conversation launch profile: {detail}")]
    LaunchProfile { detail: String },
    #[error(transparent)]
    Transport(#[from] OpenHandsError),
    #[error(transparent)]
    Tooling(#[from] opensymphony_openhands::LocalToolingError),
    #[error(transparent)]
    Supervisor(#[from] SupervisorError),
    #[error(
        "workflow config requires a local OpenHands server, but no tooling directory was configured and {repo_root}/tools/openhands-server was not found"
    )]
    MissingToolDir { repo_root: PathBuf },
    #[error(
        "OpenHands transport URL `{value}` does not include an explicit port and has no default port"
    )]
    MissingTransportPort { value: String },
    #[error("runtime rehydration returned conversation {actual}, expected {expected}")]
    RehydratedConversationMismatch { expected: Uuid, actual: Uuid },
    #[error("rehydrated conversation {conversation_id} did not expose persisted history")]
    PersistedHistoryMissing { conversation_id: Uuid },
    #[error(
        "conversation {conversation_id} remained active past the wait timeout ({timeout_ms} ms)"
    )]
    ActiveTurnTimeout {
        conversation_id: Uuid,
        timeout_ms: u128,
    },
    #[error("conversation {conversation_id} ended before the debug turn reached a terminal status")]
    StreamEnded { conversation_id: Uuid },
    #[error("conversation {conversation_id} emitted ConversationErrorEvent {event_id}")]
    ConversationError {
        conversation_id: Uuid,
        event_id: String,
    },
    #[error("conversation {conversation_id} reported terminal execution_status `{status}`")]
    TerminalStatus {
        conversation_id: Uuid,
        status: String,
    },
    #[error(
        "debug interaction timed out waiting for new runtime activity on conversation {conversation_id}"
    )]
    DebugTurnTimeout { conversation_id: Uuid },
    #[error("terminal I/O failed: {0}")]
    TerminalIo(#[source] io::Error),
}

#[derive(Clone, Copy)]
enum TranscriptRole {
    User,
    Assistant,
    Action,
    Observation,
}

impl TranscriptRole {
    fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Action => "action",
            Self::Observation => "observation",
        }
    }
}

struct TranscriptEntry {
    event_id: String,
    role: TranscriptRole,
    text: String,
}

pub async fn run_command(args: DebugArgs) -> ExitCode {
    match run_debug_session(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_debug_session(args: DebugArgs) -> Result<(), DebugCommandError> {
    let runtime = resolve_runtime_config(&args).await?;
    let manager = WorkspaceManager::new(build_workspace_manager_config(&runtime.workflow))?;
    let workspace = manager
        .find_workspace_by_issue_reference(&args.issue_id)
        .await?
        .ok_or_else(|| DebugCommandError::WorkspaceNotFound {
            issue_reference: args.issue_id.clone(),
            workspace_root: runtime.workflow.config.workspace.root.clone(),
        })?;
    let manifest = load_conversation_manifest(&manager, &workspace).await?;
    let (client, mut supervisor, server_message) = build_debug_client(&runtime)?;
    let config = IssueSessionRunnerConfig::from_workflow(&runtime.workflow);
    let conversation_id = parse_conversation_id(&manifest)?;
    let mut stream =
        attach_or_rehydrate_stream(&client, &runtime.workflow, &workspace, &manifest, &config)
            .await?;

    println!(
        "Resumed conversation {} for issue {} in {}",
        manifest.conversation_id,
        workspace.identifier(),
        workspace.workspace_path().display()
    );
    println!("{server_message}");

    if turn_is_in_progress(stream.state_mirror().execution_status().unwrap_or("idle")) {
        println!("Waiting for the current OpenHands turn to finish before accepting input...");
        wait_for_turn_to_stop(&mut stream, conversation_id, config.terminal_wait_timeout).await?;
    }

    print_recent_history(stream.event_cache().items());
    println!(
        "Type a prompt to continue the conversation. Use /history to reprint recent context and /exit to quit."
    );

    let result = interactive_debug_loop(
        &client,
        &mut stream,
        conversation_id,
        config.terminal_wait_timeout,
    )
    .await;

    let close_result = stream.close().await;
    drop(supervisor.take());
    result?;
    close_result?;
    Ok(())
}

async fn resolve_runtime_config(args: &DebugArgs) -> Result<DebugRuntimeConfig, DebugCommandError> {
    let current_dir = env::current_dir().map_err(DebugCommandError::CurrentDir)?;
    let config_path = match &args.config {
        Some(path) => Some(resolve_cli_path(&current_dir, path)),
        None => {
            let default = current_dir.join(DEFAULT_CONFIG_FILE);
            default.is_file().then_some(default)
        }
    };

    let repo_root_hint = super::find_cargo_workspace_root(&current_dir);
    let default_target_repo = if current_dir.join("WORKFLOW.md").is_file() {
        current_dir.clone()
    } else if let Some(repo_root) = repo_root_hint
        .as_ref()
        .filter(|repo_root| repo_root.join("WORKFLOW.md").is_file())
    {
        repo_root.clone()
    } else {
        current_dir.clone()
    };

    let (target_repo, configured_tool_dir) = if let Some(path) = config_path.as_ref() {
        let config = load_config(path).await?;
        let config_root = path.parent().unwrap_or(&current_dir);
        let target_repo = config
            .target_repo
            .as_deref()
            .map(|value| resolve_config_path(path, config_root, value))
            .transpose()?
            .unwrap_or_else(|| default_target_repo.clone());
        let tool_dir = config
            .openhands
            .tool_dir
            .as_deref()
            .map(|value| resolve_config_path(path, config_root, value))
            .transpose()?;
        (target_repo, tool_dir)
    } else {
        (default_target_repo, None)
    };

    let repo_root = super::find_cargo_workspace_root(&target_repo)
        .unwrap_or_else(|| super::normalize_workspace_root(&target_repo));
    let inferred_tool_dir = repo_root.join("tools").join("openhands-server");
    let tool_dir =
        configured_tool_dir.or_else(|| inferred_tool_dir.is_dir().then_some(inferred_tool_dir));
    let workflow_path = target_repo.join("WORKFLOW.md");
    let workflow = WorkflowDefinition::load_from_path(&workflow_path).map_err(|source| {
        DebugCommandError::LoadWorkflow {
            path: workflow_path.clone(),
            source,
        }
    })?;
    let workflow = workflow
        .resolve_with_process_env(&target_repo)
        .map_err(|source| DebugCommandError::ResolveWorkflow {
            path: workflow_path.clone(),
            source,
        })?;

    Ok(DebugRuntimeConfig {
        repo_root,
        workflow,
        tool_dir,
    })
}

async fn load_config(path: &Path) -> Result<DebugConfigFile, DebugCommandError> {
    let raw = fs::read_to_string(path)
        .await
        .map_err(|source| DebugCommandError::ReadConfig {
            path: path.to_path_buf(),
            source,
        })?;
    serde_yaml::from_str(&raw).map_err(|source| DebugCommandError::ParseConfig {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve_config_path(
    config_path: &Path,
    config_root: &Path,
    raw: &str,
) -> Result<PathBuf, DebugCommandError> {
    let expanded =
        super::expand_env_tokens(raw).map_err(|error| DebugCommandError::ResolveConfig {
            path: config_path.to_path_buf(),
            detail: error.to_string(),
        })?;
    Ok(super::resolve_path(config_root, &expanded))
}

fn resolve_cli_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn build_workspace_manager_config(workflow: &ResolvedWorkflow) -> WorkspaceManagerConfig {
    let hooks = &workflow.config.hooks;
    WorkspaceManagerConfig {
        root: workflow.config.workspace.root.clone(),
        hooks: HookConfig {
            after_create: hooks.after_create.clone().map(HookDefinition::shell),
            before_run: hooks.before_run.clone().map(HookDefinition::shell),
            after_run: hooks.after_run.clone().map(HookDefinition::shell),
            before_remove: hooks.before_remove.clone().map(HookDefinition::shell),
            timeout: Duration::from_millis(hooks.timeout_ms),
        },
        cleanup: CleanupConfig {
            remove_terminal_workspaces: false,
        },
    }
}

fn build_debug_client(
    runtime: &DebugRuntimeConfig,
) -> Result<(OpenHandsClient, Option<LocalServerSupervisor>, String), DebugCommandError> {
    let transport = TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)?;
    let Some(supervisor_base_url) = transport.managed_local_server_base_url()? else {
        let message = format!(
            "Using configured OpenHands server at {}.",
            transport.base_url()
        );
        return Ok((OpenHandsClient::new(transport), None, message));
    };

    let tool_dir = runtime
        .tool_dir
        .clone()
        .ok_or_else(|| DebugCommandError::MissingToolDir {
            repo_root: runtime.repo_root.clone(),
        })?;
    let tooling = LocalServerTooling::load(tool_dir)?;
    let url =
        Url::parse(&supervisor_base_url).expect("validated managed supervisor URL should parse");
    let mut config = SupervisedServerConfig::new(tooling);
    config.extra_env = runtime
        .workflow
        .extensions
        .openhands
        .local_server
        .env
        .clone();
    config.startup_timeout = Duration::from_millis(
        runtime
            .workflow
            .extensions
            .openhands
            .local_server
            .startup_timeout_ms,
    );
    config.probe.path = runtime
        .workflow
        .extensions
        .openhands
        .local_server
        .readiness_probe_path
        .clone();
    config.port_override = Some(transport_port_override(&url)?);

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    match supervisor.start() {
        Ok(status) => {
            let base_url = status.base_url.clone();
            let transport = TransportConfig::new(&base_url).with_auth(transport.auth().clone());
            Ok((
                OpenHandsClient::new(transport),
                Some(supervisor),
                format!("Started local OpenHands server at {base_url} for the debug session."),
            ))
        }
        Err(SupervisorError::ExistingReadyServer { base_url, .. }) => {
            let transport = TransportConfig::new(&base_url).with_auth(transport.auth().clone());
            Ok((
                OpenHandsClient::new(transport),
                None,
                format!("Using existing OpenHands server at {base_url}."),
            ))
        }
        Err(error) => Err(DebugCommandError::Supervisor(error)),
    }
}

fn transport_port_override(url: &Url) -> Result<u16, DebugCommandError> {
    url.port_or_known_default()
        .ok_or_else(|| DebugCommandError::MissingTransportPort {
            value: url.as_str().to_string(),
        })
}

async fn load_conversation_manifest(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
) -> Result<IssueConversationManifest, DebugCommandError> {
    let path = workspace.conversation_manifest_path();
    let raw = manager
        .read_text_artifact(workspace, &path)
        .await?
        .ok_or_else(|| DebugCommandError::ConversationManifestMissing { path: path.clone() })?;
    serde_json::from_str(&raw)
        .map_err(|source| DebugCommandError::DecodeConversationManifest { path, source })
}

fn parse_conversation_id(manifest: &IssueConversationManifest) -> Result<Uuid, DebugCommandError> {
    Uuid::parse_str(manifest.conversation_id.as_str()).map_err(|source| {
        DebugCommandError::InvalidConversationId {
            value: manifest.conversation_id.to_string(),
            source,
        }
    })
}

async fn attach_or_rehydrate_stream(
    client: &OpenHandsClient,
    workflow: &ResolvedWorkflow,
    workspace: &WorkspaceHandle,
    manifest: &IssueConversationManifest,
    config: &IssueSessionRunnerConfig,
) -> Result<RuntimeEventStream, DebugCommandError> {
    let conversation_id = parse_conversation_id(manifest)?;
    let stream_config = config.runtime_stream.clone();
    match client
        .attach_runtime_stream(conversation_id, stream_config.clone())
        .await
    {
        Ok(stream) => Ok(stream),
        Err(error) if should_rehydrate_after_attach_failure(&error) => {
            let launch_profile = resolve_launch_profile(manifest, workflow)
                .map_err(|detail| DebugCommandError::LaunchProfile { detail })?;
            let request = launch_profile.to_create_request(
                workspace.workspace_path(),
                &manifest.persistence_dir,
                Some(conversation_id),
            );
            let conversation = client.create_conversation(&request).await?;
            if conversation.conversation_id != conversation_id {
                return Err(DebugCommandError::RehydratedConversationMismatch {
                    expected: conversation_id,
                    actual: conversation.conversation_id,
                });
            }

            let stream = client
                .attach_runtime_stream(conversation_id, stream_config)
                .await?;
            if stream.event_cache().items().len() <= 1 {
                return Err(DebugCommandError::PersistedHistoryMissing { conversation_id });
            }

            Ok(stream)
        }
        Err(error) => Err(error.into()),
    }
}

fn should_rehydrate_after_attach_failure(error: &OpenHandsError) -> bool {
    matches!(
        error,
        OpenHandsError::HttpStatus {
            status_code: 404,
            ..
        }
    )
}

fn resolve_launch_profile(
    manifest: &IssueConversationManifest,
    workflow: &ResolvedWorkflow,
) -> Result<ConversationLaunchProfile, String> {
    manifest
        .launch_profile
        .clone()
        .map(Ok)
        .unwrap_or_else(|| ConversationLaunchProfile::from_workflow(workflow))
}

async fn interactive_debug_loop(
    client: &OpenHandsClient,
    stream: &mut RuntimeEventStream,
    conversation_id: Uuid,
    wait_timeout: Duration,
) -> Result<(), DebugCommandError> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut line = String::new();

    loop {
        print!("debug> ");
        stdout.flush().map_err(DebugCommandError::TerminalIo)?;
        line.clear();
        let read = stdin
            .read_line(&mut line)
            .map_err(DebugCommandError::TerminalIo)?;
        if read == 0 {
            return Ok(());
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if matches!(input, "/exit" | "exit" | "quit") {
            return Ok(());
        }
        if input == "/history" {
            print_recent_history(stream.event_cache().items());
            continue;
        }

        run_debug_turn(client, stream, conversation_id, input, wait_timeout).await?;
    }
}

async fn run_debug_turn(
    client: &OpenHandsClient,
    stream: &mut RuntimeEventStream,
    conversation_id: Uuid,
    prompt: &str,
    wait_timeout: Duration,
) -> Result<(), DebugCommandError> {
    if turn_is_in_progress(stream.state_mirror().execution_status().unwrap_or("idle")) {
        wait_for_turn_to_stop(stream, conversation_id, wait_timeout).await?;
    }

    let baseline_event_ids = stream
        .event_cache()
        .items()
        .iter()
        .map(|event| event.id.clone())
        .collect::<HashSet<_>>();

    client
        .send_message(conversation_id, &SendMessageRequest::user_text(prompt))
        .await?;
    loop {
        match client.run_conversation(conversation_id).await {
            Ok(_) => break,
            Err(OpenHandsError::HttpStatus {
                status_code: 409, ..
            }) => {
                wait_for_turn_to_stop(stream, conversation_id, wait_timeout).await?;
                let _ = stream.reconcile_events().await;
            }
            Err(error) => return Err(DebugCommandError::Transport(error)),
        }
    }

    wait_for_turn_terminal(stream, &baseline_event_ids, conversation_id, wait_timeout).await?;
    let new_entries = transcript_entries(stream.event_cache().items())
        .into_iter()
        .filter(|entry| !baseline_event_ids.contains(&entry.event_id))
        .filter(|entry| !matches!(entry.role, TranscriptRole::User))
        .collect::<Vec<_>>();

    if new_entries.is_empty() {
        println!("assistant> (no printable assistant text was emitted for this turn)");
    } else {
        for entry in new_entries {
            println!("{}> {}", entry.role.label(), entry.text);
        }
    }

    Ok(())
}

async fn wait_for_turn_to_stop(
    stream: &mut RuntimeEventStream,
    conversation_id: Uuid,
    wait_timeout: Duration,
) -> Result<(), DebugCommandError> {
    if stream
        .state_mirror()
        .execution_status()
        .is_none_or(turn_has_stopped)
    {
        return Ok(());
    }

    let deadline = tokio::time::Instant::now() + wait_timeout;
    loop {
        if stream
            .state_mirror()
            .execution_status()
            .is_some_and(turn_has_stopped)
        {
            return Ok(());
        }

        match timeout_at(deadline, stream.next_event()).await {
            Err(_) => {
                return Err(DebugCommandError::ActiveTurnTimeout {
                    conversation_id,
                    timeout_ms: wait_timeout.as_millis(),
                });
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => {
                if let Ok(inserted) = stream.reconcile_events().await
                    && inserted > 0
                {
                    continue;
                }
            }
            Ok(Err(error)) => {
                if stream
                    .state_mirror()
                    .execution_status()
                    .is_some_and(turn_has_stopped)
                {
                    return Ok(());
                }
                return Err(DebugCommandError::Transport(error));
            }
        }
    }
}

async fn wait_for_turn_terminal(
    stream: &mut RuntimeEventStream,
    baseline_event_ids: &HashSet<String>,
    conversation_id: Uuid,
    wait_timeout: Duration,
) -> Result<(), DebugCommandError> {
    let deadline = tokio::time::Instant::now() + wait_timeout;
    loop {
        if let Some(result) = current_turn_outcome(stream, baseline_event_ids, conversation_id) {
            return result;
        }

        match timeout_at(deadline, stream.next_event()).await {
            Err(_) => {
                if let Ok(inserted) = stream.reconcile_events().await
                    && inserted > 0
                {
                    continue;
                }
                return Err(DebugCommandError::DebugTurnTimeout { conversation_id });
            }
            Ok(Ok(Some(_))) => {}
            Ok(Ok(None)) => {
                if let Ok(inserted) = stream.reconcile_events().await
                    && inserted > 0
                {
                    continue;
                }
                if let Some(result) =
                    current_turn_outcome(stream, baseline_event_ids, conversation_id)
                {
                    return result;
                }
                return Err(DebugCommandError::StreamEnded { conversation_id });
            }
            Ok(Err(error)) => {
                if let Some(result) =
                    current_turn_outcome(stream, baseline_event_ids, conversation_id)
                {
                    return result;
                }
                return Err(DebugCommandError::Transport(error));
            }
        }
    }
}

fn current_turn_outcome(
    stream: &RuntimeEventStream,
    baseline_event_ids: &HashSet<String>,
    conversation_id: Uuid,
) -> Option<Result<(), DebugCommandError>> {
    let current_turn_events = stream
        .event_cache()
        .items()
        .iter()
        .filter(|event| !baseline_event_ids.contains(&event.id))
        .collect::<Vec<_>>();
    if current_turn_events.is_empty() {
        return None;
    }

    if let Some(error_event) = current_turn_events.iter().find(|event| {
        matches!(
            KnownEvent::from_envelope(event),
            KnownEvent::ConversationError(_)
        )
    }) {
        return Some(Err(DebugCommandError::ConversationError {
            conversation_id,
            event_id: error_event.id.clone(),
        }));
    }

    match stream.state_mirror().terminal_status() {
        Some(TerminalExecutionStatus::Finished) => Some(Ok(())),
        Some(TerminalExecutionStatus::Error) | Some(TerminalExecutionStatus::Stuck) => {
            Some(Err(DebugCommandError::TerminalStatus {
                conversation_id,
                status: stream
                    .state_mirror()
                    .execution_status()
                    .unwrap_or("unknown")
                    .to_string(),
            }))
        }
        None => None,
    }
}

fn print_recent_history(events: &[EventEnvelope]) {
    let entries = transcript_entries(events);
    if entries.is_empty() {
        println!("No prior printable transcript entries were found in the resumed conversation.");
        return;
    }

    println!("Recent conversation history:");
    let start = entries.len().saturating_sub(RECENT_HISTORY_LIMIT);
    for entry in &entries[start..] {
        println!(
            "{}> {}",
            entry.role.label(),
            summarize_history_text(&entry.text)
        );
    }
}

fn transcript_entries(events: &[EventEnvelope]) -> Vec<TranscriptEntry> {
    events.iter().filter_map(extract_transcript_entry).collect()
}

fn extract_transcript_entry(event: &EventEnvelope) -> Option<TranscriptEntry> {
    match event.kind.as_str() {
        "MessageEvent" => {
            let (role, content) = if let Some(message) = event.payload.get("llm_message") {
                (
                    TranscriptRole::Assistant,
                    message
                        .get("content")?
                        .as_array()?
                        .first()?
                        .get("text")?
                        .as_str()?,
                )
            } else {
                let role = match event
                    .payload
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("user") => TranscriptRole::User,
                    _ => TranscriptRole::Assistant,
                };
                (
                    role,
                    event
                        .payload
                        .get("content")?
                        .as_array()?
                        .first()?
                        .get("text")?
                        .as_str()?,
                )
            };
            Some(TranscriptEntry {
                event_id: event.id.clone(),
                role,
                text: normalize_text(content),
            })
        }
        "ActionEvent" => Some(TranscriptEntry {
            event_id: event.id.clone(),
            role: TranscriptRole::Action,
            text: normalize_text(
                event
                    .payload
                    .get("action")
                    .and_then(|action| action.get("message"))?
                    .as_str()?,
            ),
        }),
        "ObservationEvent" => Some(TranscriptEntry {
            event_id: event.id.clone(),
            role: TranscriptRole::Observation,
            text: normalize_text(
                event
                    .payload
                    .get("observation")
                    .and_then(|observation| observation.get("content"))?
                    .as_array()?
                    .first()?
                    .get("text")?
                    .as_str()?,
            ),
        }),
        _ => None,
    }
}

fn normalize_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn summarize_history_text(text: &str) -> String {
    const LIMIT: usize = 160;
    if text.chars().count() <= LIMIT {
        text.to_string()
    } else {
        let shortened = text.chars().take(LIMIT - 3).collect::<String>();
        format!("{shortened}...")
    }
}

fn turn_is_in_progress(status: &str) -> bool {
    !matches!(status, "idle" | "finished" | "error" | "stuck")
}

fn turn_has_stopped(status: &str) -> bool {
    !turn_is_in_progress(status)
}

#[cfg(test)]
mod tests {
    use super::should_rehydrate_after_attach_failure;
    use opensymphony_openhands::OpenHandsError;

    #[test]
    fn rehydrate_only_when_conversation_is_missing() {
        assert!(should_rehydrate_after_attach_failure(
            &OpenHandsError::HttpStatus {
                operation: "fetch conversation",
                status_code: 404,
                body: "missing".to_string(),
            }
        ));
        assert!(!should_rehydrate_after_attach_failure(
            &OpenHandsError::HttpStatus {
                operation: "fetch conversation",
                status_code: 401,
                body: "unauthorized".to_string(),
            }
        ));
        assert!(!should_rehydrate_after_attach_failure(
            &OpenHandsError::Transport {
                operation: "fetch conversation",
                detail: "connection refused".to_string(),
            }
        ));
    }
}
