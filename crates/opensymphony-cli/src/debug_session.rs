use std::{
    cmp::Ordering,
    collections::HashSet,
    env, io,
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use crate::opensymphony_openhands::{
    ConversationLaunchProfile, ConversationMoveOutcome, ConversationStoreKind, EventEnvelope,
    IssueConversationManifest, IssueSessionRunnerConfig, KnownEvent, LocalServerSupervisor,
    LocalServerTooling, OPENHANDS_CONVERSATIONS_PATH_ENV, OpenHandsClient,
    OpenHandsConversationStorePaths, OpenHandsError, RuntimeEventStream, SendMessageRequest,
    SupervisedServerConfig, SupervisorConfig, SupervisorError, TerminalExecutionStatus,
    TransportConfig,
};
use crate::opensymphony_workflow::{ProcessEnvironment, ResolvedWorkflow, WorkflowDefinition};
use crate::opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueManifest, WorkspaceError, WorkspaceHandle,
    WorkspaceManager, WorkspaceManagerConfig,
};
use clap::Args;
use crossterm::{
    cursor::{self, MoveTo},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{self, Clear, ClearType},
};
use serde::Deserialize;
use thiserror::Error;
use tokio::{fs, time::timeout_at};
use url::Url;
use uuid::Uuid;

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const RECENT_HISTORY_LIMIT: usize = 8;
const RECENT_EVENT_SCAN_LIMIT: usize = 100;

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
    workflow: ResolvedWorkflow,
    tool_dir: Option<PathBuf>,
    conversation_store: Option<OpenHandsConversationStorePaths>,
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
        source: crate::opensymphony_workflow::WorkflowLoadError,
    },
    #[error("failed to resolve workflow {path}: {source}")]
    ResolveWorkflow {
        path: PathBuf,
        #[source]
        source: crate::opensymphony_workflow::WorkflowConfigError,
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
    #[error("failed to prepare OpenHands conversation store: {0}")]
    ConversationStore(#[from] crate::opensymphony_openhands::ConversationStoreError),
    #[error(transparent)]
    Tooling(#[from] crate::opensymphony_openhands::LocalToolingError),
    #[error(transparent)]
    Supervisor(#[from] SupervisorError),
    #[error(
        "workflow config requires a managed local OpenHands server, but `openhands.tool_dir` is missing from config.yaml (recommended: ~/.opensymphony/openhands-server)"
    )]
    MissingToolDir,
    #[error(
        "managed local OpenHands tooling at {tool_dir} is missing or invalid: {detail}. Run `opensymphony install openhands` or `opensymphony doctor --config <path>`."
    )]
    ToolingSetupRequired { tool_dir: PathBuf, detail: String },
    #[error(
        "OpenHands transport URL `{value}` does not include an explicit port and has no default port"
    )]
    MissingTransportPort { value: String },
    #[error("runtime rehydration returned conversation {actual}, expected {expected}")]
    RehydratedConversationMismatch { expected: Uuid, actual: Uuid },
    #[error("rehydrated conversation {conversation_id} did not expose persisted history")]
    PersistedHistoryMissing { conversation_id: Uuid },
    #[error(
        "archived conversation {conversation_id} was not found in managed OpenHands store {store_path}. The debug session launched OpenHands with this `OH_CONVERSATIONS_PATH`; verify the conversation directory exists in that store or rerun archive/migration for the issue."
    )]
    ArchivedConversationMissingFromManagedStore {
        conversation_id: Uuid,
        store_path: PathBuf,
    },
    #[error(
        "archived conversation {conversation_id} could not be attached from expected store {store_path}. The conversation may be missing from that store, or the already-running/external OpenHands server may be using a different `OH_CONVERSATIONS_PATH`. Stop the existing server or free the port, then retry."
    )]
    ArchivedConversationUnavailable {
        conversation_id: Uuid,
        store_path: PathBuf,
    },
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

    fn ansi_prefix(self) -> &'static str {
        match self {
            Self::User => "\x1b[1;32m",
            Self::Assistant => "\x1b[1;35m",
            Self::Action => "\x1b[1;36m",
            Self::Observation => "\x1b[90m",
        }
    }
}

struct TranscriptEntry {
    event_id: String,
    role: TranscriptRole,
    text: String,
}

enum DebugInput {
    Exit,
    RecentHistory,
    FullHistory,
    Prompt(String),
}

struct EventBaseline {
    ids: HashSet<String>,
    latest: Option<EventPosition>,
}

struct DebugEventPrinter<'a> {
    baseline: &'a EventBaseline,
    printed_event_ids: HashSet<String>,
}

struct EventPosition {
    timestamp: chrono::DateTime<chrono::Utc>,
    id: String,
}

impl EventBaseline {
    fn capture(events: &[EventEnvelope]) -> Self {
        let ids = events
            .iter()
            .map(|event| event.id.clone())
            .collect::<HashSet<_>>();
        let latest = events
            .iter()
            .max_by(|left, right| compare_event_position(left, right))
            .map(|event| EventPosition {
                timestamp: event.timestamp,
                id: event.id.clone(),
            });
        Self { ids, latest }
    }

    fn is_current_turn_event(&self, event: &EventEnvelope) -> bool {
        if self.ids.contains(&event.id) {
            return false;
        }

        self.latest.as_ref().is_none_or(|latest| {
            event
                .timestamp
                .cmp(&latest.timestamp)
                .then_with(|| event.id.cmp(&latest.id))
                == Ordering::Greater
        })
    }
}

impl<'a> DebugEventPrinter<'a> {
    fn new(baseline: &'a EventBaseline) -> Self {
        Self {
            baseline,
            printed_event_ids: HashSet::new(),
        }
    }

    fn print_event(&mut self, event: &EventEnvelope) {
        if !self.baseline.is_current_turn_event(event)
            || !self.printed_event_ids.insert(event.id.clone())
        {
            return;
        }

        let Some(entry) = extract_transcript_entry(event) else {
            return;
        };
        if matches!(entry.role, TranscriptRole::User) {
            return;
        }

        print_transcript_entry(&entry, true, true);
    }

    fn into_printed_event_ids(self) -> HashSet<String> {
        self.printed_event_ids
    }
}

struct ArchivedAttachContext<'a> {
    selected_store_path: Option<&'a Path>,
    launched_managed_server: bool,
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
    let issue_manifest = manager.load_issue_manifest(&workspace).await?;
    let manifest = load_conversation_manifest(&manager, &workspace).await?;
    let config = IssueSessionRunnerConfig::from_workflow(&runtime.workflow);
    let conversation_id = parse_conversation_id(&manifest)?;
    let store_kind =
        prepare_debug_conversation_store(&runtime, conversation_id, issue_manifest.as_ref())?;
    let selected_store_path = store_kind
        .and_then(|kind| {
            runtime
                .conversation_store
                .as_ref()
                .map(|paths| paths.path_for(kind))
        })
        .map(Path::to_path_buf);
    let (client, mut supervisor, server_message) = build_debug_client(&runtime, store_kind)?;
    println!("{server_message}");
    println!(
        "Attaching to conversation {} for issue {}...",
        manifest.conversation_id,
        workspace.identifier()
    );
    let launched_managed_server = supervisor.is_some();
    let mut stream = match attach_or_rehydrate_stream(
        &client,
        &runtime.workflow,
        &workspace,
        &manifest,
        &config,
        store_kind != Some(ConversationStoreKind::Archived),
        ArchivedAttachContext {
            selected_store_path: selected_store_path.as_deref(),
            launched_managed_server,
        },
    )
    .await
    {
        Ok(stream) => stream,
        Err(error) => {
            if let Some(supervisor) = supervisor.as_mut()
                && let Err(stop_error) = supervisor.stop()
            {
                tracing::warn!(%stop_error, "failed to stop debug OpenHands supervisor after attach failure");
            }
            return Err(error);
        }
    };

    println!(
        "Resumed conversation {} for issue {} in {}",
        manifest.conversation_id,
        workspace.identifier(),
        workspace.workspace_path().display()
    );

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

    let conversation_store = configured_tool_dir
        .as_ref()
        .map(|tool_dir| OpenHandsConversationStorePaths::for_tool_dir(tool_dir, &target_repo))
        .transpose()?;

    Ok(DebugRuntimeConfig {
        workflow,
        tool_dir: configured_tool_dir,
        conversation_store,
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

fn prepare_debug_conversation_store(
    runtime: &DebugRuntimeConfig,
    conversation_id: Uuid,
    issue_manifest: Option<&IssueManifest>,
) -> Result<Option<ConversationStoreKind>, DebugCommandError> {
    let Some(store) = runtime.conversation_store.as_ref() else {
        return Ok(None);
    };
    store.ensure_active_and_archived()?;

    match store.locate_conversation(&conversation_id.to_string())? {
        Some(located) if located.kind != ConversationStoreKind::Legacy => Ok(Some(located.kind)),
        Some(_) => {
            let target = debug_store_target_for_issue(&runtime.workflow, issue_manifest);
            match store.move_conversation_to(&conversation_id.to_string(), target)? {
                ConversationMoveOutcome::Moved { to, .. }
                | ConversationMoveOutcome::AlreadyInTarget { kind: to, .. } => Ok(Some(to)),
                ConversationMoveOutcome::Missing => Ok(Some(target)),
            }
        }
        None => Ok(Some(ConversationStoreKind::Active)),
    }
}

fn debug_store_target_for_issue(
    workflow: &ResolvedWorkflow,
    issue_manifest: Option<&IssueManifest>,
) -> ConversationStoreKind {
    let Some(issue_manifest) = issue_manifest else {
        return ConversationStoreKind::Active;
    };
    let current_state = issue_manifest.current_state.trim();
    if workflow
        .config
        .tracker
        .terminal_states
        .iter()
        .any(|state| state.trim().eq_ignore_ascii_case(current_state))
    {
        ConversationStoreKind::Archived
    } else {
        ConversationStoreKind::Active
    }
}

fn build_debug_client(
    runtime: &DebugRuntimeConfig,
    conversation_store_kind: Option<ConversationStoreKind>,
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
        .ok_or(DebugCommandError::MissingToolDir)?;
    let tooling = LocalServerTooling::load(tool_dir.clone()).map_err(|error| {
        DebugCommandError::ToolingSetupRequired {
            tool_dir,
            detail: error.to_string(),
        }
    })?;
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
    let conversation_store_path = conversation_store_kind.and_then(|kind| {
        runtime
            .conversation_store
            .as_ref()
            .map(|paths| paths.path_for(kind))
    });
    if let Some(path) = conversation_store_path {
        config.extra_env.insert(
            OPENHANDS_CONVERSATIONS_PATH_ENV.to_string(),
            path.display().to_string(),
        );
    }
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
    println!("Checking local OpenHands server at {supervisor_base_url}...");
    match supervisor.start() {
        Ok(status) => {
            let base_url = status.base_url.clone();
            let transport = TransportConfig::new(&base_url).with_auth(transport.auth().clone());
            let store_suffix = conversation_store_path
                .map(|path| format!(" Conversation store: {}.", path.display()))
                .unwrap_or_default();
            Ok((
                OpenHandsClient::new(transport),
                Some(supervisor),
                format!(
                    "Started local OpenHands server at {base_url} for the debug session.{store_suffix}"
                ),
            ))
        }
        Err(SupervisorError::ExistingReadyServer { base_url, .. }) => {
            let transport = TransportConfig::new(&base_url).with_auth(transport.auth().clone());
            let store_suffix = conversation_store_path
                .map(|path| format!(" Expected conversation store: {}.", path.display()))
                .unwrap_or_default();
            Ok((
                OpenHandsClient::new(transport),
                None,
                format!("Using existing OpenHands server at {base_url}.{store_suffix}"),
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
    rehydrate_on_missing: bool,
    archived_context: ArchivedAttachContext<'_>,
) -> Result<RuntimeEventStream, DebugCommandError> {
    let conversation_id = parse_conversation_id(manifest)?;
    let stream_config = config.runtime_stream.clone();
    match client
        .attach_runtime_stream_with_recent_events(
            conversation_id,
            stream_config.clone(),
            RECENT_EVENT_SCAN_LIMIT,
        )
        .await
    {
        Ok(stream) => Ok(stream),
        Err(error) if should_rehydrate_after_attach_failure(&error) && !rehydrate_on_missing => {
            let store_path = archived_context
                .selected_store_path
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("<unknown>"));
            if archived_context.launched_managed_server {
                Err(
                    DebugCommandError::ArchivedConversationMissingFromManagedStore {
                        conversation_id,
                        store_path,
                    },
                )
            } else {
                Err(DebugCommandError::ArchivedConversationUnavailable {
                    conversation_id,
                    store_path,
                })
            }
        }
        Err(error) if should_rehydrate_after_attach_failure(&error) => {
            let launch_profile = resolve_launch_profile(manifest, workflow)
                .map_err(|detail| DebugCommandError::LaunchProfile { detail })?;
            let request = launch_profile
                .to_create_request(
                    &ProcessEnvironment,
                    workspace.workspace_path(),
                    &manifest.persistence_dir,
                    Some(conversation_id),
                )
                .map_err(|detail| DebugCommandError::LaunchProfile { detail })?;
            let conversation = client.create_conversation(&request).await?;
            if conversation.conversation_id != conversation_id {
                return Err(DebugCommandError::RehydratedConversationMismatch {
                    expected: conversation_id,
                    actual: conversation.conversation_id,
                });
            }

            let stream = client
                .attach_runtime_stream_with_recent_events(
                    conversation_id,
                    stream_config,
                    RECENT_EVENT_SCAN_LIMIT,
                )
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
        let Some(input) = read_debug_prompt(&stdin, &mut stdout, &mut line)? else {
            return Ok(());
        };
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        match parse_debug_input(input) {
            DebugInput::Exit => return Ok(()),
            DebugInput::RecentHistory => {
                print_recent_history(stream.event_cache().items());
            }
            DebugInput::FullHistory => {
                print_full_history(stream).await?;
            }
            DebugInput::Prompt(prompt) => {
                run_debug_turn(client, stream, conversation_id, &prompt, wait_timeout).await?;
            }
        }
    }
}

fn read_debug_prompt(
    stdin: &io::Stdin,
    stdout: &mut io::Stdout,
    line: &mut String,
) -> Result<Option<String>, DebugCommandError> {
    if stdin.is_terminal() && stdout.is_terminal() {
        read_raw_debug_prompt(stdout)
    } else {
        read_line_debug_prompt(stdin, stdout, line)
    }
}

fn read_line_debug_prompt(
    stdin: &io::Stdin,
    stdout: &mut io::Stdout,
    line: &mut String,
) -> Result<Option<String>, DebugCommandError> {
    write!(stdout, "debug> ").map_err(DebugCommandError::TerminalIo)?;
    stdout.flush().map_err(DebugCommandError::TerminalIo)?;
    line.clear();
    let read = stdin
        .read_line(line)
        .map_err(DebugCommandError::TerminalIo)?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
}

fn read_raw_debug_prompt(stdout: &mut io::Stdout) -> Result<Option<String>, DebugCommandError> {
    let _guard = RawDebugPromptGuard::enter(stdout)?;
    let prompt_origin = cursor::position().unwrap_or((0, 0));
    write!(stdout, "debug> ").map_err(DebugCommandError::TerminalIo)?;
    stdout.flush().map_err(DebugCommandError::TerminalIo)?;

    let mut input = String::new();
    loop {
        match event::read().map_err(DebugCommandError::TerminalIo)? {
            Event::Paste(text) => {
                append_debug_input(stdout, &mut input, &text)?;
            }
            Event::Resize(_, _) => {
                redraw_debug_prompt(stdout, prompt_origin, &input)?;
            }
            Event::Key(key) if key.kind == KeyEventKind::Release => {}
            Event::Key(key) => match key.code {
                KeyCode::Enter
                    if key
                        .modifiers
                        .intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL) =>
                {
                    append_debug_input(stdout, &mut input, "\n")?;
                }
                KeyCode::Enter => {
                    write!(stdout, "\r\n").map_err(DebugCommandError::TerminalIo)?;
                    stdout.flush().map_err(DebugCommandError::TerminalIo)?;
                    return Ok(Some(input));
                }
                KeyCode::Char('j') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    append_debug_input(stdout, &mut input, "\n")?;
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    write!(stdout, "^C\r\n").map_err(DebugCommandError::TerminalIo)?;
                    stdout.flush().map_err(DebugCommandError::TerminalIo)?;
                    return Ok(None);
                }
                KeyCode::Char('d')
                    if key.modifiers.contains(KeyModifiers::CONTROL) && input.is_empty() =>
                {
                    write!(stdout, "\r\n").map_err(DebugCommandError::TerminalIo)?;
                    stdout.flush().map_err(DebugCommandError::TerminalIo)?;
                    return Ok(None);
                }
                KeyCode::Char(character) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    append_debug_input(stdout, &mut input, &character.to_string())?;
                }
                KeyCode::Tab => {
                    append_debug_input(stdout, &mut input, "\t")?;
                }
                KeyCode::Backspace => {
                    input.pop();
                    redraw_debug_prompt(stdout, prompt_origin, &input)?;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn append_debug_input(
    stdout: &mut io::Stdout,
    input: &mut String,
    text: &str,
) -> Result<(), DebugCommandError> {
    let normalized = normalize_debug_input_fragment(text);
    input.push_str(&normalized);
    echo_debug_input(stdout, &normalized)
}

fn normalize_debug_input_fragment(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn redraw_debug_prompt(
    stdout: &mut io::Stdout,
    prompt_origin: (u16, u16),
    input: &str,
) -> Result<(), DebugCommandError> {
    execute!(
        stdout,
        MoveTo(prompt_origin.0, prompt_origin.1),
        Clear(ClearType::FromCursorDown)
    )
    .map_err(DebugCommandError::TerminalIo)?;
    write!(stdout, "debug> ").map_err(DebugCommandError::TerminalIo)?;
    echo_debug_input(stdout, input)
}

fn echo_debug_input(stdout: &mut io::Stdout, text: &str) -> Result<(), DebugCommandError> {
    for character in text.chars() {
        match character {
            '\n' => write!(stdout, "\r\n...> ").map_err(DebugCommandError::TerminalIo)?,
            '\t' => write!(stdout, "\t").map_err(DebugCommandError::TerminalIo)?,
            character if character.is_control() => {}
            character => write!(stdout, "{character}").map_err(DebugCommandError::TerminalIo)?,
        }
    }
    stdout.flush().map_err(DebugCommandError::TerminalIo)
}

struct RawDebugPromptGuard;

impl RawDebugPromptGuard {
    fn enter(stdout: &mut io::Stdout) -> Result<Self, DebugCommandError> {
        terminal::enable_raw_mode().map_err(DebugCommandError::TerminalIo)?;
        if let Err(error) = execute!(stdout, EnableBracketedPaste) {
            let _ = terminal::disable_raw_mode();
            return Err(DebugCommandError::TerminalIo(error));
        }
        Ok(Self)
    }
}

impl Drop for RawDebugPromptGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(stdout, DisableBracketedPaste);
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

    let baseline = EventBaseline::capture(stream.event_cache().items());
    print_transcript_entry(
        &TranscriptEntry {
            event_id: "debug-user-input".to_string(),
            role: TranscriptRole::User,
            text: prompt.to_string(),
        },
        true,
        false,
    );

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

    let printed_event_ids =
        wait_for_turn_terminal(stream, &baseline, conversation_id, wait_timeout).await?;
    let current_turn_event_ids = stream
        .event_cache()
        .items()
        .iter()
        .filter(|event| baseline.is_current_turn_event(event))
        .map(|event| event.id.clone())
        .collect::<HashSet<_>>();
    let new_entries = transcript_entries(stream.event_cache().items())
        .into_iter()
        .filter(|entry| current_turn_event_ids.contains(&entry.event_id))
        .filter(|entry| !printed_event_ids.contains(&entry.event_id))
        .filter(|entry| !matches!(entry.role, TranscriptRole::User))
        .collect::<Vec<_>>();

    if new_entries.is_empty() {
        println!();
        println!(
            "{} (no printable assistant text was emitted for this turn)",
            formatted_role_label(TranscriptRole::Assistant)
        );
    } else {
        for entry in new_entries {
            print_transcript_entry(&entry, true, false);
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

    let baseline = EventBaseline::capture(stream.event_cache().items());
    let mut printer = DebugEventPrinter::new(&baseline);
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
            Ok(Ok(Some(event))) => {
                printer.print_event(&event);
            }
            Ok(Ok(None)) => {
                if let Ok(inserted) = reconcile_debug_events(stream).await
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
    baseline: &EventBaseline,
    conversation_id: Uuid,
    wait_timeout: Duration,
) -> Result<HashSet<String>, DebugCommandError> {
    let mut printer = DebugEventPrinter::new(baseline);
    let deadline = tokio::time::Instant::now() + wait_timeout;
    loop {
        if let Some(result) = current_turn_outcome(stream, baseline, conversation_id) {
            return result.map(|_| printer.into_printed_event_ids());
        }

        match timeout_at(deadline, stream.next_event()).await {
            Err(_) => {
                if let Ok(inserted) = reconcile_debug_events(stream).await
                    && inserted > 0
                {
                    continue;
                }
                return Err(DebugCommandError::DebugTurnTimeout { conversation_id });
            }
            Ok(Ok(Some(event))) => {
                printer.print_event(&event);
            }
            Ok(Ok(None)) => {
                if let Ok(inserted) = reconcile_debug_events(stream).await
                    && inserted > 0
                {
                    continue;
                }
                if let Some(result) = current_turn_outcome(stream, baseline, conversation_id) {
                    return result.map(|_| printer.into_printed_event_ids());
                }
                return Err(DebugCommandError::StreamEnded { conversation_id });
            }
            Ok(Err(error)) => {
                if let Some(result) = current_turn_outcome(stream, baseline, conversation_id) {
                    return result.map(|_| printer.into_printed_event_ids());
                }
                return Err(DebugCommandError::Transport(error));
            }
        }
    }
}

fn current_turn_outcome(
    stream: &RuntimeEventStream,
    baseline: &EventBaseline,
    conversation_id: Uuid,
) -> Option<Result<(), DebugCommandError>> {
    let current_turn_events = stream
        .event_cache()
        .items()
        .iter()
        .filter(|event| baseline.is_current_turn_event(event))
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
        print_transcript_entry(entry, true, true);
    }
}

async fn print_full_history(stream: &mut RuntimeEventStream) -> Result<(), DebugCommandError> {
    println!("Loading full conversation history...");
    stream.reconcile_events().await?;
    let events = stream.event_cache().items();
    let entries = transcript_entries(events);
    if entries.is_empty() {
        println!("No printable transcript entries were found in the resumed conversation.");
        return Ok(());
    }

    println!(
        "Full conversation history ({} printable entries from {} events):",
        entries.len(),
        events.len()
    );
    for entry in &entries {
        print_transcript_entry(entry, true, true);
    }
    Ok(())
}

async fn reconcile_debug_events(stream: &mut RuntimeEventStream) -> Result<usize, OpenHandsError> {
    stream
        .reconcile_recent_events(RECENT_EVENT_SCAN_LIMIT)
        .await
}

fn parse_debug_input(input: &str) -> DebugInput {
    match input.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["/exit"] | ["exit"] | ["quit"] => DebugInput::Exit,
        ["/history"] => DebugInput::RecentHistory,
        ["/history", "all"] => DebugInput::FullHistory,
        _ => DebugInput::Prompt(input.to_string()),
    }
}

fn transcript_entries(events: &[EventEnvelope]) -> Vec<TranscriptEntry> {
    events.iter().filter_map(extract_transcript_entry).collect()
}

fn print_transcript_entry(entry: &TranscriptEntry, blank_before: bool, summarize: bool) {
    if blank_before {
        println!();
    }
    let text = if summarize {
        summarize_history_text(&entry.text)
    } else {
        entry.text.clone()
    };
    println!("{} {}", formatted_role_label(entry.role), text);
}

fn formatted_role_label(role: TranscriptRole) -> String {
    let label = format!("{}>", role.label());
    if terminal_colors_enabled() {
        format!("{}{}\x1b[0m", role.ansi_prefix(), label)
    } else {
        label
    }
}

fn terminal_colors_enabled() -> bool {
    io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none()
}

fn extract_transcript_entry(event: &EventEnvelope) -> Option<TranscriptEntry> {
    match event.kind.as_str() {
        "MessageEvent" => {
            let (role, content) = if let Some(message) = event.payload.get("llm_message") {
                (TranscriptRole::Assistant, first_content_text(message)?)
            } else {
                let role = match event
                    .payload
                    .get("role")
                    .and_then(serde_json::Value::as_str)
                {
                    Some("user") => TranscriptRole::User,
                    _ => TranscriptRole::Assistant,
                };
                (role, first_content_text(&event.payload)?)
            };
            transcript_entry(event, role, content)
        }
        "ActionEvent" => action_text(&event.payload)
            .as_deref()
            .and_then(|text| transcript_entry(event, TranscriptRole::Action, text)),
        "ObservationEvent" => first_content_text(&event.payload)
            .or_else(|| {
                event
                    .payload
                    .get("observation")
                    .and_then(first_content_text)
            })
            .and_then(|text| transcript_entry(event, TranscriptRole::Observation, text)),
        _ => None,
    }
}

fn transcript_entry(
    event: &EventEnvelope,
    role: TranscriptRole,
    text: &str,
) -> Option<TranscriptEntry> {
    let text = normalize_text(text);
    (!text.is_empty()).then(|| TranscriptEntry {
        event_id: event.id.clone(),
        role,
        text,
    })
}

fn first_content_text(value: &serde_json::Value) -> Option<&str> {
    value
        .get("content")?
        .as_array()?
        .first()?
        .get("text")?
        .as_str()
}

fn action_text(payload: &serde_json::Value) -> Option<String> {
    let summary = payload
        .get("summary")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let detail = payload
        .get("action")
        .and_then(|action| {
            action
                .get("message")
                .or_else(|| action.get("command"))
                .and_then(serde_json::Value::as_str)
        })
        .or_else(|| payload.get("command").and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (summary, detail) {
        (Some(summary), Some(detail)) if summary != detail => Some(format!("{summary}: {detail}")),
        (Some(summary), _) => Some(summary.to_string()),
        (_, Some(detail)) => Some(detail.to_string()),
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

fn compare_event_position(left: &EventEnvelope, right: &EventEnvelope) -> Ordering {
    left.timestamp
        .cmp(&right.timestamp)
        .then_with(|| left.id.cmp(&right.id))
}

fn turn_is_in_progress(status: &str) -> bool {
    !matches!(status, "idle" | "finished" | "error" | "stuck")
}

fn turn_has_stopped(status: &str) -> bool {
    !turn_is_in_progress(status)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::opensymphony_openhands::{
        ConversationStoreKind, OpenHandsConversationStorePaths, OpenHandsError,
    };
    use crate::opensymphony_workflow::WorkflowDefinition;
    use crate::opensymphony_workspace::IssueManifest;
    use chrono::Utc;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::{
        DebugCommandError, DebugInput, DebugRuntimeConfig, build_debug_client,
        normalize_debug_input_fragment, parse_debug_input, prepare_debug_conversation_store,
        should_rehydrate_after_attach_failure,
    };

    #[test]
    fn debug_slash_commands_are_parsed_locally() {
        assert!(matches!(
            parse_debug_input("/history"),
            DebugInput::RecentHistory
        ));
        assert!(matches!(
            parse_debug_input("/history all"),
            DebugInput::FullHistory
        ));
        assert!(matches!(parse_debug_input("/exit"), DebugInput::Exit));
        assert!(matches!(parse_debug_input("quit"), DebugInput::Exit));

        match parse_debug_input("/history now please") {
            DebugInput::Prompt(prompt) => assert_eq!(prompt, "/history now please"),
            _ => panic!("unsupported slash forms should remain ordinary prompts"),
        }
    }

    #[test]
    fn debug_prompt_preserves_multiline_prompt_text() {
        match parse_debug_input("first line\nsecond line") {
            DebugInput::Prompt(prompt) => assert_eq!(prompt, "first line\nsecond line"),
            _ => panic!("multiline input should be sent as a prompt"),
        }
    }

    #[test]
    fn debug_prompt_normalizes_pasted_newlines() {
        assert_eq!(
            normalize_debug_input_fragment("one\r\ntwo\rthree\nfour"),
            "one\ntwo\nthree\nfour"
        );
    }

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

    #[test]
    fn build_debug_client_requires_tool_dir_for_managed_local_transport() {
        let runtime = sample_debug_runtime(None);

        let error = match build_debug_client(&runtime, None) {
            Err(error) => error,
            Ok(_) => panic!("managed-local debug should require tool_dir"),
        };

        assert!(matches!(error, DebugCommandError::MissingToolDir));
    }

    #[test]
    fn build_debug_client_reports_install_guidance_for_invalid_tooling() {
        let temp_dir = TempDir::new().expect("temp dir should exist");
        let tool_dir = temp_dir.path().join("missing/openhands-server");
        let runtime = sample_debug_runtime(Some(tool_dir.clone()));

        let error = match build_debug_client(&runtime, None) {
            Err(error) => error,
            Ok(_) => panic!("invalid tooling should be reported"),
        };

        match error {
            DebugCommandError::ToolingSetupRequired {
                tool_dir: reported,
                detail,
            } => {
                assert_eq!(reported, tool_dir);
                assert!(
                    detail.contains("required local OpenHands tooling file is missing"),
                    "tooling detail should explain the missing managed-local file: {detail}",
                );
            }
            other => panic!("expected tooling setup guidance, got {other}"),
        }
    }

    #[test]
    fn debug_store_preparation_moves_legacy_terminal_issue_to_archive_store() {
        let repo = TempDir::new().expect("repo should exist");
        let tool_dir = TempDir::new().expect("tool dir should exist");
        let store = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("store paths should resolve");
        let conversation_id =
            Uuid::parse_str("dd258bb7-cc1b-415c-9892-e19af34a2e66").expect("uuid");
        let legacy_path = store.legacy_root.join(conversation_id.simple().to_string());
        std::fs::create_dir_all(&legacy_path).expect("legacy conversation should exist");
        let mut runtime = sample_debug_runtime(None);
        runtime.conversation_store = Some(store.clone());
        let issue_manifest = sample_issue_manifest("Done");

        let kind =
            prepare_debug_conversation_store(&runtime, conversation_id, Some(&issue_manifest))
                .expect("store should prepare");

        assert_eq!(kind, Some(ConversationStoreKind::Archived));
        assert!(!legacy_path.exists());
        assert!(
            store
                .archived
                .join(conversation_id.simple().to_string())
                .is_dir()
        );
    }

    #[test]
    fn debug_store_preparation_keeps_existing_archived_conversation_archived() {
        let repo = TempDir::new().expect("repo should exist");
        let tool_dir = TempDir::new().expect("tool dir should exist");
        let store = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("store paths should resolve");
        let conversation_id =
            Uuid::parse_str("dd258bb7-cc1b-415c-9892-e19af34a2e66").expect("uuid");
        let archived_path = store.archived.join(conversation_id.simple().to_string());
        std::fs::create_dir_all(&archived_path).expect("archived conversation should exist");
        let mut runtime = sample_debug_runtime(None);
        runtime.conversation_store = Some(store);

        let kind = prepare_debug_conversation_store(
            &runtime,
            conversation_id,
            Some(&sample_issue_manifest("Todo")),
        )
        .expect("store should prepare");

        assert_eq!(kind, Some(ConversationStoreKind::Archived));
        assert!(archived_path.is_dir());
    }

    fn sample_issue_manifest(current_state: &str) -> IssueManifest {
        IssueManifest {
            issue_id: "issue-1".to_string(),
            identifier: "COE-1".to_string(),
            title: "Sample".to_string(),
            current_state: current_state.to_string(),
            sanitized_workspace_key: "COE-1".to_string(),
            workspace_path: PathBuf::from("/tmp/COE-1"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_seen_tracker_refresh_at: None,
        }
    }

    fn sample_debug_runtime(tool_dir: Option<PathBuf>) -> DebugRuntimeConfig {
        let temp_dir = TempDir::new().expect("temp dir should exist");
        let target_repo = temp_dir.path().join("target-repo");
        std::fs::create_dir_all(&target_repo).expect("target repo should exist");
        let workflow = WorkflowDefinition::parse(
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
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
---

# Debug Session
"#,
        )
        .expect("workflow should parse")
        .resolve(
            &target_repo,
            &super::super::DoctorWorkflowEnvironment {
                fallback_linear_api_key: true,
            },
        )
        .expect("workflow should resolve");

        DebugRuntimeConfig {
            workflow,
            tool_dir,
            conversation_store: None,
        }
    }
}
