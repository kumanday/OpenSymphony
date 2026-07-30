//! Runtime backend adapters for tracker, workspace, and worker orchestration.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    env, io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use crate::opensymphony_codex::{
    CODEX_APP_SERVER_CONTRACT, CODEX_APP_SERVER_KIND, CodexAppServerAdapter,
    CodexAppServerSchemaValidator, CodexContractGeneration, CodexJsonRpcSession,
    JsonRpcRequestEnvelope, NormalizedCodexEvent, NormalizedCodexEventKind,
    codex_approval_request_from_event, codex_event_summary, normalize_server_notification,
    turn_status,
};
use crate::opensymphony_domain::{
    ConversationId, ConversationMetadata, HarnessInterruptReason, IssueId, IssueIdentifier,
    IssueState, IssueStateCategory, NormalizedIssue, RetryEntry, RetryReason, RuntimeStreamState,
    TimestampMs, TrackerErrorCategory, TrackerIssue, TrackerIssueSummary, WorkerOutcomeKind,
    WorkerOutcomeRecord, WorkspaceKey,
};
use crate::opensymphony_linear::{LinearClient, LinearConfig, LinearError, WorkpadComment};
use crate::opensymphony_openhands::{
    ConversationMoveOutcome, ConversationStoreKind, IssueConversationManifest, IssueSessionError,
    IssueSessionObserver, IssueSessionPromptKind, IssueSessionResult, IssueSessionRunner,
    IssueSessionRunnerConfig, LocalServerSupervisor, LocalServerTooling, MemoryWorkerAccess,
    OPENHANDS_CONVERSATIONS_PATH_ENV, OpenHandsClient, OpenHandsConversationStorePaths,
    OpenHandsError, SupervisedServerConfig, SupervisorConfig, TransportConfig,
    WorkpadComment as SessionWorkpadComment, WorkpadCommentSource, build_continuation_guidance,
};
use crate::opensymphony_orchestrator::{
    RecoveredRun, RecoveryRecord, RetryExhaustionRecord, TrackerBackend, WorkerAbortReason,
    WorkerBackend, WorkerInterruptAcknowledgement, WorkerLaunch, WorkerStartRequest, WorkerUpdate,
    WorkspaceBackend,
};
use crate::opensymphony_workflow::{Environment, ProcessEnvironment, ResolvedWorkflow};
use crate::opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, IssueLifecycleState, RunDescriptor,
    RunManifest, RunStatus, WorkspaceError, WorkspaceHandle, WorkspaceManager,
    WorkspaceManagerConfig,
};
use async_trait::async_trait;
use thiserror::Error;
use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStderr, ChildStdin, Command},
    sync::{Mutex as AsyncMutex, mpsc, oneshot},
    task::JoinHandle,
    time::{timeout, timeout_at},
};
use url::Url;

use super::{
    RunCommandError, RuntimeMemoryEnv, config::RunRuntimeConfig, datetime_to_timestamp_ms,
    now_timestamp, timestamp_to_datetime,
};

const DEFAULT_WORKER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);
const CODEX_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_WORKER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(75);
const CODEX_SCHEMA_GENERATION_TIMEOUT: Duration = Duration::from_secs(30);
const CODEX_TERMINAL_TIMEOUT: Duration = Duration::from_secs(300);
const CODEX_STDERR_TAIL_LINES: usize = 20;
const CODEX_SCHEMA_STDERR_PREVIEW_CHARS: usize = 500;
const OPENHANDS_AGENT_SERVER_KIND: &str = "openhands_agent_server";

#[derive(Debug, Error)]
pub(super) enum CliWorkspaceError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Identifier(#[from] crate::opensymphony_domain::IdentifierError),
    #[error("Codex lifecycle recovery failed: {0}")]
    CodexLifecycle(String),
    #[error("retry state persistence failed: {0}")]
    RetryState(String),
}

#[derive(Debug, Error)]
pub(super) enum CliWorkerError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("worker launch timed out after {0:?}")]
    LaunchTimeout(Duration),
    #[error("worker failed before reporting a conversation launch: {0}")]
    LaunchFailed(String),
    #[error("worker exited before reporting a conversation launch")]
    LaunchChannelClosed,
    #[error("worker task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("worker interrupt failed: {0}")]
    InterruptFailed(String),
}

#[derive(Debug)]
enum LaunchReport {
    Conversation(Box<ConversationMetadata>),
    Failed(String),
}

pub(super) struct RuntimeTrackerBackend {
    client: LinearClient,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ActiveConversationStorePreparation {
    pub moved: usize,
    pub already_active: usize,
    pub missing: usize,
    pub skipped_without_workspace: usize,
    pub skipped_without_manifest: usize,
    pub skipped_invalid_manifest: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct LegacyConversationStoreMigration {
    pub moved_to_archived: usize,
    pub already_archived: usize,
    pub missing: usize,
    pub skipped_non_terminal: usize,
    pub skipped_without_manifest: usize,
    pub skipped_invalid_manifest: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ManagedLocalPreparation {
    pub active_conversations: ActiveConversationStorePreparation,
    pub legacy_conversations: LegacyConversationStoreMigration,
    pub tooling: Option<LocalServerTooling>,
}

pub(super) struct RuntimeWorkspaceBackend {
    manager: Arc<WorkspaceManager>,
    active_states: HashSet<String>,
    terminal_states: HashSet<String>,
    terminal_cleanup_paths: HashSet<PathBuf>,
    codex_bin: String,
    retain_failed: bool,
    retry_state_root: PathBuf,
}

pub(super) struct RuntimeWorkerBackend {
    client: OpenHandsClient,
    workflow: Arc<ResolvedWorkflow>,
    workspace_manager: Arc<WorkspaceManager>,
    runner_config: IssueSessionRunnerConfig,
    workpad_comment_source: Option<Arc<dyn WorkpadCommentSource>>,
    worker_env: BTreeMap<String, String>,
    codex_bin: String,
    codex_schema_validators: CodexSchemaValidatorCache,
    codex_interrupts: CodexInterruptRegistry,
    launch_timeout: Duration,
    updates_tx: mpsc::UnboundedSender<WorkerUpdate>,
    updates_rx: mpsc::UnboundedReceiver<WorkerUpdate>,
    tasks: HashMap<String, ActiveWorkerTask>,
}

type CodexSchemaValidatorCache = Arc<AsyncMutex<HashMap<String, CodexAppServerSchemaValidator>>>;
type CodexInterruptRegistry = Arc<Mutex<HashMap<String, Arc<AsyncMutex<CodexInterruptChannel>>>>>;
type CodexInterruptResponseRegistry = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<(), String>>>>>;

struct ActiveWorkerTask {
    handle: JoinHandle<()>,
    run: crate::opensymphony_domain::RunAttempt,
}

struct PendingLaunch {
    worker_id: String,
    route: crate::opensymphony_orchestrator::HarnessRouteDecision,
    launch_rx: oneshot::Receiver<LaunchReport>,
}

struct SchedulerObserver {
    worker_id: String,
    launch_tx: Option<oneshot::Sender<LaunchReport>>,
    updates_tx: mpsc::UnboundedSender<WorkerUpdate>,
}

struct CodexInterruptChannel {
    stdin: ChildStdin,
    session: CodexJsonRpcSession,
    schema_validator: CodexAppServerSchemaValidator,
    thread_id: String,
    turn_id: String,
    responses: CodexInterruptResponseRegistry,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
}

struct CodexInterruptRegistration {
    registry: CodexInterruptRegistry,
    thread_id: String,
}

struct LinearWorkpadCommentSource {
    client: LinearClient,
}

#[derive(Clone, Debug, Default)]
struct OverlayEnvironment {
    overrides: BTreeMap<String, String>,
}

impl Environment for OverlayEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        self.overrides
            .get(name)
            .cloned()
            .or_else(|| env::var_os(name).map(|value| value.to_string_lossy().into_owned()))
    }
}

#[async_trait]
impl WorkpadCommentSource for LinearWorkpadCommentSource {
    async fn fetch_workpad_comment(
        &self,
        issue_id: &str,
    ) -> Result<Option<SessionWorkpadComment>, String> {
        self.client
            .fetch_workpad_comment(issue_id)
            .await
            .map(|comment| comment.map(workpad_comment_from_linear))
            .map_err(|error| error.to_string())
    }
}

impl IssueSessionObserver for SchedulerObserver {
    fn on_launch(&mut self, conversation: &ConversationMetadata) {
        if let Some(sender) = self.launch_tx.take() {
            let _ = sender.send(LaunchReport::Conversation(Box::new(conversation.clone())));
        }
    }

    fn on_runtime_event(
        &mut self,
        observed_at: TimestampMs,
        event_id: Option<String>,
        event_kind: Option<String>,
        summary: Option<String>,
        payload: Option<serde_json::Value>,
    ) {
        let worker_id = self.worker_id.clone();
        let _ = self.updates_tx.send(WorkerUpdate::RuntimeEvent {
            worker_id: crate::opensymphony_domain::WorkerId::new(worker_id)
                .expect("worker id should remain valid"),
            observed_at,
            event_id,
            event_kind,
            summary,
            payload,
        });
    }

    fn on_conversation_update(&mut self, conversation: &ConversationMetadata) {
        let worker_id = self.worker_id.clone();
        let _ = self
            .updates_tx
            .send(WorkerUpdate::ConversationMetadataUpdate {
                worker_id: crate::opensymphony_domain::WorkerId::new(worker_id)
                    .expect("worker id should remain valid"),
                conversation: conversation.clone(),
            });
    }
}

pub(super) fn build_linear_client(
    workflow: &ResolvedWorkflow,
) -> Result<LinearClient, LinearError> {
    let tracker = &workflow.config.tracker;
    let mut config = LinearConfig::new(tracker.api_key.clone(), tracker.project_slug.clone());
    config.base_url = tracker.endpoint.clone();
    config.project_id = tracker.project_id.clone();
    config.active_states = tracker.active_states.clone();
    config.terminal_states = tracker.terminal_states.clone();
    LinearClient::new(config)
}

fn workpad_comment_from_linear(comment: WorkpadComment) -> SessionWorkpadComment {
    SessionWorkpadComment {
        id: comment.id,
        body: comment.body,
        updated_at: comment.updated_at,
    }
}

pub(super) fn build_tracker_backend(
    workflow: &ResolvedWorkflow,
) -> Result<RuntimeTrackerBackend, LinearError> {
    Ok(RuntimeTrackerBackend {
        client: build_linear_client(workflow)?,
    })
}

pub(super) async fn prepare_active_conversation_store(
    runtime: &RunRuntimeConfig,
    tracker: &mut RuntimeTrackerBackend,
    workspace_manager: &WorkspaceManager,
) -> Result<ManagedLocalPreparation, RunCommandError> {
    let Some(conversation_store) = runtime.openhands_conversation_store.as_ref() else {
        return Ok(ManagedLocalPreparation::default());
    };
    let transport = TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)?;
    let supervised = transport.managed_local_server_base_url()?.is_some()
        && runtime.workflow.extensions.openhands.local_server.enabled;
    if !supervised {
        return Ok(ManagedLocalPreparation::default());
    }
    let tool_dir = runtime
        .tool_dir
        .clone()
        .ok_or(RunCommandError::MissingToolDir)?;
    // Validate tooling once before mutating conversation stores; the prepared
    // handle is passed through to `build_runtime_transport` so startup does not
    // touch the managed install twice on the normal path.
    let tooling = LocalServerTooling::load(tool_dir.clone()).map_err(|error| {
        RunCommandError::ToolingSetupRequired {
            tool_dir,
            detail: error.to_string(),
        }
    })?;
    conversation_store.ensure_active_and_archived()?;
    let legacy_conversations = migrate_legacy_workspace_conversations(
        workspace_manager,
        conversation_store,
        &runtime.workflow,
    )
    .await?;
    let active_issues = tracker.client.candidate_issues().await?;
    let active_conversations = prepare_active_conversation_store_for_issues(
        workspace_manager,
        conversation_store,
        &active_issues,
    )
    .await?;
    Ok(ManagedLocalPreparation {
        active_conversations,
        legacy_conversations,
        tooling: Some(tooling),
    })
}

// Temporary compatibility shim for pre repo-scoped OpenHands stores. Once the
// legacy flat store has aged out for real users, this function can be removed
// without touching normal active-store preparation or server startup.
async fn migrate_legacy_workspace_conversations(
    workspace_manager: &WorkspaceManager,
    conversation_store: &OpenHandsConversationStorePaths,
    workflow: &ResolvedWorkflow,
) -> Result<LegacyConversationStoreMigration, RunCommandError> {
    let mut report = LegacyConversationStoreMigration::default();
    let terminal_states = workflow
        .config
        .tracker
        .terminal_states
        .iter()
        .map(|state| state.trim().to_ascii_lowercase())
        .collect::<HashSet<_>>();

    for (workspace, issue_manifest) in workspace_manager.list_all_workspaces().await? {
        if !terminal_states.contains(&issue_manifest.current_state.trim().to_ascii_lowercase()) {
            report.skipped_non_terminal += 1;
            continue;
        }

        let manifest_path = workspace.conversation_manifest_path();
        let Some(raw_manifest) = workspace_manager
            .read_text_artifact(&workspace, &manifest_path)
            .await?
        else {
            report.skipped_without_manifest += 1;
            continue;
        };
        let manifest = match serde_json::from_str::<IssueConversationManifest>(&raw_manifest) {
            Ok(manifest) => manifest,
            Err(error) => {
                report.skipped_invalid_manifest += 1;
                tracing::warn!(
                    issue = %issue_manifest.identifier,
                    manifest = %manifest_path.display(),
                    %error,
                    "skipping legacy OpenHands conversation migration for invalid manifest"
                );
                continue;
            }
        };
        if conversation_manifest_is_codex(&manifest) {
            continue;
        }

        match conversation_store.move_conversation_to(
            manifest.conversation_id.as_str(),
            ConversationStoreKind::Archived,
        )? {
            ConversationMoveOutcome::Moved { from, .. } => {
                report.moved_to_archived += 1;
                tracing::info!(
                    issue = %issue_manifest.identifier,
                    conversation_id = %manifest.conversation_id,
                    from = %from,
                    "moved terminal OpenHands conversation into the repo archived store"
                );
            }
            ConversationMoveOutcome::AlreadyInTarget { .. } => {
                report.already_archived += 1;
            }
            ConversationMoveOutcome::Missing => {
                report.missing += 1;
                tracing::warn!(
                    issue = %issue_manifest.identifier,
                    conversation_id = %manifest.conversation_id,
                    "terminal OpenHands conversation was not found in active, archived, or legacy stores"
                );
            }
        }
    }

    Ok(report)
}

async fn prepare_active_conversation_store_for_issues(
    workspace_manager: &WorkspaceManager,
    conversation_store: &OpenHandsConversationStorePaths,
    active_issues: &[TrackerIssue],
) -> Result<ActiveConversationStorePreparation, RunCommandError> {
    let mut report = ActiveConversationStorePreparation::default();

    for issue in active_issues {
        let Some(workspace) = workspace_manager
            .find_workspace_by_issue_reference(issue.identifier.as_str())
            .await?
        else {
            report.skipped_without_workspace += 1;
            continue;
        };

        let manifest_path = workspace.conversation_manifest_path();
        let Some(raw_manifest) = workspace_manager
            .read_text_artifact(&workspace, &manifest_path)
            .await?
        else {
            report.skipped_without_manifest += 1;
            continue;
        };
        let manifest = match serde_json::from_str::<IssueConversationManifest>(&raw_manifest) {
            Ok(manifest) => manifest,
            Err(error) => {
                report.skipped_invalid_manifest += 1;
                tracing::warn!(
                    issue = %issue.identifier,
                    manifest = %manifest_path.display(),
                    %error,
                    "skipping active OpenHands conversation store migration for invalid manifest"
                );
                continue;
            }
        };
        if conversation_manifest_is_codex(&manifest) {
            continue;
        }

        match conversation_store.move_conversation_to(
            manifest.conversation_id.as_str(),
            ConversationStoreKind::Active,
        )? {
            ConversationMoveOutcome::Moved { from, .. } => {
                report.moved += 1;
                tracing::info!(
                    issue = %issue.identifier,
                    conversation_id = %manifest.conversation_id,
                    from = %from,
                    "moved active OpenHands conversation into the repo active store"
                );
            }
            ConversationMoveOutcome::AlreadyInTarget { .. } => {
                report.already_active += 1;
            }
            ConversationMoveOutcome::Missing => {
                report.missing += 1;
                tracing::warn!(
                    issue = %issue.identifier,
                    conversation_id = %manifest.conversation_id,
                    "active OpenHands conversation was not found in active, archived, or legacy stores"
                );
            }
        }
    }

    Ok(report)
}

fn conversation_manifest_is_codex(manifest: &IssueConversationManifest) -> bool {
    manifest.transport_target.as_deref() == Some(CODEX_APP_SERVER_KIND)
        || manifest.runtime_contract_version.as_deref() == Some(CODEX_APP_SERVER_CONTRACT)
}

#[cfg(test)]
pub(super) fn build_workspace_manager_config(
    workflow: &ResolvedWorkflow,
) -> WorkspaceManagerConfig {
    let mut config = build_workspace_manager_config_with_retention(workflow, true, true);
    // Unit tests exercise backend behavior without the scheduler's
    // outcome-aware cleanup decision, so keep their fixtures available for
    // manifest assertions.
    config.cleanup.remove_terminal_workspaces = false;
    config
}

pub(super) fn build_workspace_manager_config_with_retention(
    workflow: &ResolvedWorkflow,
    _retain_failed: bool,
    preserve_terminal_workspaces: bool,
) -> WorkspaceManagerConfig {
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
            remove_terminal_workspaces: !preserve_terminal_workspaces,
        },
    }
}

pub(super) async fn build_runtime_transport(
    runtime: &RunRuntimeConfig,
    prepared_tooling: Option<LocalServerTooling>,
    memory_env: Option<&RuntimeMemoryEnv>,
    worker_env: &BTreeMap<String, String>,
) -> Result<(TransportConfig, Option<LocalServerSupervisor>), RunCommandError> {
    let transport = TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)?;
    let local_server = &runtime.workflow.extensions.openhands.local_server;
    let supervisor_base_url = transport.managed_local_server_base_url()?;
    let supervised = supervisor_base_url.is_some() && local_server.enabled;
    if local_server.command.is_some() && !supervised {
        return Err(OpenHandsError::InvalidConfiguration {
            detail:
                "`openhands.local_server.command` requires a managed local OpenHands target with `local_server.enabled: true`"
                    .to_string(),
        }
        .into());
    }

    if !supervised {
        return Ok((transport, None));
    }
    let Some(supervisor_base_url) = supervisor_base_url else {
        return Ok((transport, None));
    };

    let tool_dir = runtime
        .tool_dir
        .clone()
        .ok_or(RunCommandError::MissingToolDir)?;
    let tooling = match prepared_tooling {
        Some(tooling) => tooling,
        None => LocalServerTooling::load(tool_dir.clone()).map_err(|error| {
            RunCommandError::ToolingSetupRequired {
                tool_dir,
                detail: error.to_string(),
            }
        })?,
    };
    let url =
        Url::parse(&supervisor_base_url).expect("validated managed supervisor URL should parse");
    let mut config = SupervisedServerConfig::new(tooling);
    config.command = local_server.command.clone();
    config.extra_env = local_server.env.clone();
    config.extra_env.extend(worker_env.clone());
    if let Some(conversation_store) = runtime.openhands_conversation_store.as_ref() {
        conversation_store.ensure_active_and_archived()?;
        config.extra_env.insert(
            OPENHANDS_CONVERSATIONS_PATH_ENV.to_string(),
            conversation_store.active.display().to_string(),
        );
    }
    if let Some(memory_env) = memory_env {
        inject_memory_env(&mut config.extra_env, memory_env);
    }
    config.startup_timeout = Duration::from_millis(local_server.startup_timeout_ms);
    config.probe.path = local_server.readiness_probe_path.clone();
    config.port_override = Some(transport_port_override(&url)?);

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    let status = supervisor.start()?;
    let transport = TransportConfig::new(status.base_url).with_auth(transport.auth().clone());
    Ok((transport, Some(supervisor)))
}

impl TrackerBackend for RuntimeTrackerBackend {
    type Error = LinearError;

    async fn candidate_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.client.candidate_issues().await
    }

    async fn candidate_issue_summaries(&mut self) -> Result<Vec<TrackerIssueSummary>, Self::Error> {
        self.client.candidate_issue_summaries().await
    }

    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.client.terminal_issues().await
    }

    async fn issues_by_identifiers(
        &mut self,
        identifiers: &[String],
    ) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.client.project_issues_by_identifiers(identifiers).await
    }

    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<crate::opensymphony_domain::TrackerIssueStateSnapshot>, Self::Error> {
        self.client.issue_states_by_ids(issue_ids).await
    }

    fn error_category(error: &Self::Error) -> Option<TrackerErrorCategory> {
        Some(error.category())
    }

    fn retry_after(error: &Self::Error) -> Option<Duration> {
        error.retry_after()
    }
}

impl RuntimeWorkspaceBackend {
    #[cfg(test)]
    pub(super) fn new(manager: Arc<WorkspaceManager>, workflow: &ResolvedWorkflow) -> Self {
        Self::new_with_retention(manager, workflow, true)
    }

    #[cfg(test)]
    pub(super) fn new_with_retention(
        manager: Arc<WorkspaceManager>,
        workflow: &ResolvedWorkflow,
        retain_failed: bool,
    ) -> Self {
        let retry_state_root = manager.config().root.join(".opensymphony-retry-state");
        Self::new_with_retention_and_state_root(manager, workflow, retain_failed, retry_state_root)
    }

    pub(super) fn new_with_retention_and_state_root(
        manager: Arc<WorkspaceManager>,
        workflow: &ResolvedWorkflow,
        retain_failed: bool,
        retry_state_root: PathBuf,
    ) -> Self {
        Self {
            manager,
            active_states: workflow
                .config
                .tracker
                .active_states
                .iter()
                .map(|state| normalized_state_name(state))
                .collect(),
            terminal_states: workflow
                .config
                .tracker
                .terminal_states
                .iter()
                .map(|state| normalized_state_name(state))
                .collect(),
            terminal_cleanup_paths: HashSet::new(),
            codex_bin: env::var("OPENSYMPHONY_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
            retain_failed,
            retry_state_root,
        }
    }
}

impl RuntimeWorkspaceBackend {
    async fn cleanup_workspace_with_policy(
        &mut self,
        workspace: &crate::opensymphony_domain::WorkspaceRecord,
        terminal: bool,
        force_remove: bool,
    ) -> Result<(), CliWorkspaceError> {
        if terminal && (force_remove || !self.terminal_cleanup_paths.contains(&workspace.path)) {
            let Some(handle) = self
                .manager
                .list_all_workspaces()
                .await?
                .into_iter()
                .find_map(|(handle, _)| {
                    (handle.workspace_path() == workspace.path).then_some(handle)
                })
            else {
                return Ok(());
            };
            let manifest_path = handle.conversation_manifest_path();
            if let Some(raw_manifest) = self
                .manager
                .read_text_artifact(&handle, &manifest_path)
                .await?
            {
                match serde_json::from_str::<IssueConversationManifest>(&raw_manifest) {
                    Ok(mut manifest) if conversation_manifest_is_codex(&manifest) => {
                        if let Err(error) = archive_terminal_codex_thread(
                            &self.manager,
                            &handle,
                            &mut manifest,
                            &self.codex_bin,
                        )
                        .await
                        {
                            tracing::warn!(
                                issue = %handle.identifier(),
                                thread_id = %manifest.conversation_id,
                                %error,
                                "preserving terminal Codex workspace for archive retry"
                            );
                            return Err(CliWorkspaceError::CodexLifecycle(error));
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(
                            issue = %handle.identifier(),
                            manifest = %manifest_path.display(),
                            %error,
                            "continuing terminal cleanup with invalid conversation manifest"
                        );
                    }
                }
            }
            if force_remove {
                self.manager
                    .cleanup_failed_terminal_workspace(&handle)
                    .await?;
            } else {
                self.manager
                    .cleanup(&handle, IssueLifecycleState::Terminal)
                    .await?;
            }
            self.terminal_cleanup_paths.insert(workspace.path.clone());
        }
        Ok(())
    }
}

impl WorkspaceBackend for RuntimeWorkspaceBackend {
    type Error = CliWorkspaceError;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        _observed_at: TimestampMs,
    ) -> Result<crate::opensymphony_domain::WorkspaceRecord, Self::Error> {
        let ensured = self.manager.ensure(&issue_descriptor(issue)).await?;
        self.terminal_cleanup_paths
            .remove(ensured.handle.workspace_path());
        Ok(crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())?,
            created_now: ensured.created,
            created_at: Some(datetime_to_timestamp_ms(ensured.issue_manifest.created_at)),
            updated_at: Some(datetime_to_timestamp_ms(ensured.issue_manifest.updated_at)),
            last_seen_tracker_refresh_at: ensured
                .issue_manifest
                .last_seen_tracker_refresh_at
                .map(datetime_to_timestamp_ms),
        })
    }

    async fn recover_workspaces(&mut self) -> Result<Vec<RecoveryRecord>, Self::Error> {
        let mut recoveries = Vec::new();
        for (handle, manifest) in self.manager.list_all_workspaces().await? {
            let run_manifest = self.manager.load_run_manifest(&handle).await?;
            let had_in_flight_run = run_manifest.as_ref().is_some_and(|run| {
                matches!(
                    run.status,
                    RunStatus::Preparing | RunStatus::Prepared | RunStatus::Running
                )
            });
            let conversation_manifest =
                recovered_conversation_manifest(&self.manager, &handle).await?;
            let harness_kind = conversation_manifest
                .as_ref()
                .map(recovered_harness_kind_from_manifest);
            let recovered_run = run_manifest
                .as_ref()
                .filter(|run| recoverable_run_manifest(run, conversation_manifest.as_ref()))
                .and_then(|_| {
                    recovered_run_from_manifests(
                        run_manifest.as_ref(),
                        conversation_manifest.as_ref(),
                    )
                });

            recoveries.push(RecoveryRecord {
                issue: normalized_issue_from_manifest(
                    &manifest,
                    &self.active_states,
                    &self.terminal_states,
                )?,
                workspace: crate::opensymphony_domain::WorkspaceRecord {
                    path: handle.workspace_path().to_path_buf(),
                    workspace_key: WorkspaceKey::new(handle.workspace_key().to_string())?,
                    created_now: false,
                    created_at: Some(datetime_to_timestamp_ms(manifest.created_at)),
                    updated_at: Some(datetime_to_timestamp_ms(manifest.updated_at)),
                    last_seen_tracker_refresh_at: manifest
                        .last_seen_tracker_refresh_at
                        .map(datetime_to_timestamp_ms),
                },
                successful_run: run_manifest
                    .as_ref()
                    .is_some_and(|run| run.status == RunStatus::Succeeded),
                cancelled_run: run_manifest
                    .as_ref()
                    .is_some_and(|run| run.status == RunStatus::Cancelled),
                completed_run: run_manifest.as_ref().is_some_and(|run| {
                    matches!(
                        run.status,
                        RunStatus::Succeeded | RunStatus::Failed | RunStatus::Cancelled
                    )
                }),
                had_in_flight_run,
                pending_retry: run_manifest.as_ref().is_some_and(|run| run.pending_retry),
                normal_retry_count: run_manifest
                    .as_ref()
                    .map(|run| run.normal_retry_count)
                    .unwrap_or_default(),
                retry_scheduled_at: run_manifest
                    .as_ref()
                    .and_then(|run| run.retry_scheduled_at.map(TimestampMs::new)),
                retry_due_at: run_manifest
                    .as_ref()
                    .and_then(|run| run.retry_due_at.map(TimestampMs::new)),
                retry_reason: run_manifest
                    .as_ref()
                    .and_then(|run| run.retry_reason.as_deref())
                    .and_then(retry_reason_from_manifest),
                retry_error: run_manifest
                    .as_ref()
                    .and_then(|run| run.retry_error.clone()),
                harness_kind,
                interrupt_reason: run_manifest
                    .as_ref()
                    .and_then(|run| run.interrupt_reason.as_deref())
                    .and_then(interrupt_reason_from_manifest),
                recovered_run: had_in_flight_run.then_some(recovered_run).flatten(),
            });
        }
        Ok(recoveries)
    }

    async fn recover_retry_exhaustion(
        &mut self,
    ) -> Result<Vec<RetryExhaustionRecord>, Self::Error> {
        let directory = self.retry_state_root.join("retry-exhaustion");
        let mut entries = match fs::read_dir(&directory).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(CliWorkspaceError::RetryState(format!(
                    "failed to list {}: {error}",
                    directory.display()
                )));
            }
        };
        let mut records = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            CliWorkspaceError::RetryState(format!(
                "failed to read {}: {error}",
                directory.display()
            ))
        })? {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path).await.map_err(|error| {
                CliWorkspaceError::RetryState(format!("failed to read {}: {error}", path.display()))
            })?;
            let record = serde_json::from_str::<RetryExhaustionRecord>(&raw).map_err(|error| {
                CliWorkspaceError::RetryState(format!(
                    "failed to parse {}: {error}",
                    path.display()
                ))
            })?;
            records.push(record);
        }
        records.sort_by(|left, right| left.issue.identifier.cmp(&right.issue.identifier));
        Ok(records)
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &crate::opensymphony_domain::WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error> {
        self.cleanup_workspace_with_policy(workspace, terminal, false)
            .await
    }

    async fn cleanup_failed_workspace(
        &mut self,
        workspace: &crate::opensymphony_domain::WorkspaceRecord,
    ) -> Result<(), Self::Error> {
        self.cleanup_workspace_with_policy(workspace, true, true)
            .await
    }

    async fn persist_retry_count(
        &mut self,
        _workspace: &crate::opensymphony_domain::WorkspaceRecord,
        _normal_retry_count: u32,
    ) -> Result<(), Self::Error> {
        // The queued retry marker must survive until start_run writes the
        // replacement manifest. Clearing it here creates a crash window
        // between scheduler preparation and worker launch.
        Ok(())
    }

    async fn persist_interrupt_reason(
        &mut self,
        workspace: &crate::opensymphony_domain::WorkspaceRecord,
        reason: HarnessInterruptReason,
    ) -> Result<(), Self::Error> {
        let Some((handle, _)) = self
            .manager
            .list_all_workspaces()
            .await?
            .into_iter()
            .find(|(handle, _)| handle.workspace_path() == workspace.path)
        else {
            return Err(CliWorkspaceError::Workspace(WorkspaceError::ReadManifest {
                path: workspace.path.join(".opensymphony/run.json"),
                source: io::Error::new(io::ErrorKind::NotFound, "workspace is not managed"),
            }));
        };
        let Some(mut manifest) = self.manager.load_run_manifest(&handle).await? else {
            return Err(CliWorkspaceError::Workspace(WorkspaceError::ReadManifest {
                path: workspace.path.join(".opensymphony/run.json"),
                source: io::Error::new(io::ErrorKind::NotFound, "run manifest is missing"),
            }));
        };
        manifest.interrupt_reason = Some(reason.as_str().to_owned());
        manifest.updated_at = chrono::Utc::now();
        self.manager.write_run_manifest(&handle, &manifest).await?;
        Ok(())
    }

    async fn persist_retry_pending(
        &mut self,
        workspace: &crate::opensymphony_domain::WorkspaceRecord,
        retry: &RetryEntry,
    ) -> Result<(), Self::Error> {
        let Some((handle, _)) = self
            .manager
            .list_all_workspaces()
            .await?
            .into_iter()
            .find(|(handle, _)| handle.workspace_path() == workspace.path)
        else {
            return Err(CliWorkspaceError::Workspace(WorkspaceError::ReadManifest {
                path: workspace.path.join(".opensymphony/run.json"),
                source: io::Error::new(io::ErrorKind::NotFound, "workspace is not managed"),
            }));
        };
        let mut manifest = match self.manager.load_run_manifest(&handle).await? {
            Some(manifest) => manifest,
            None => {
                let run = RunDescriptor::new(
                    format!("retry-pending-{}", handle.workspace_key()),
                    retry.attempt.get(),
                )
                // Recovery increments a pending manifest's count when it
                // reconstructs the queued retry. Store the predecessor here
                // so a launch failure before run.json gets exactly one
                // retry attempt after restart.
                .with_normal_retry_count(retry.normal_retry_count.saturating_sub(1));
                let mut manifest = RunManifest::new(&handle, &run);
                // The failed launch never produced an executable run manifest.
                // Create a non-in-flight marker so recovery sees the durable
                // pending retry instead of repeatedly treating the workspace as
                // an initial dispatch.
                manifest.status = RunStatus::PreparationFailed;
                manifest.status_detail =
                    Some("worker launch failed before a run manifest was created".to_string());
                manifest
            }
        };
        manifest.pending_retry = true;
        manifest.retry_scheduled_at = Some(retry.scheduled_at.as_u64());
        manifest.retry_due_at = Some(retry.due_at.as_u64());
        manifest.retry_reason = Some(retry_reason_for_manifest(retry.reason));
        manifest.retry_error = retry.error.clone();
        manifest.updated_at = chrono::Utc::now();
        self.manager.write_run_manifest(&handle, &manifest).await?;
        Ok(())
    }

    async fn persist_retry_exhaustion(
        &mut self,
        issue: &NormalizedIssue,
        normal_retry_count: u32,
    ) -> Result<(), Self::Error> {
        let key = crate::opensymphony_workspace::sanitize_workspace_key(issue.identifier.as_str())?;
        let directory = self.retry_state_root.join("retry-exhaustion");
        fs::create_dir_all(&directory).await.map_err(|error| {
            CliWorkspaceError::RetryState(format!(
                "failed to create {}: {error}",
                directory.display()
            ))
        })?;
        let path = directory.join(format!("{key}.json"));
        let temporary = directory.join(format!(".{key}.json.tmp"));
        let contents = serde_json::to_vec_pretty(&RetryExhaustionRecord {
            issue: issue.clone(),
            normal_retry_count,
        })
        .map_err(|error| CliWorkspaceError::RetryState(error.to_string()))?;
        fs::write(&temporary, contents).await.map_err(|error| {
            CliWorkspaceError::RetryState(format!(
                "failed to write {}: {error}",
                temporary.display()
            ))
        })?;
        if let Err(error) = replace_retry_exhaustion_marker(&temporary, &path).await {
            let _ = fs::remove_file(&temporary).await;
            return Err(CliWorkspaceError::RetryState(format!(
                "failed to activate {}: {error}",
                path.display()
            )));
        }
        Ok(())
    }

    async fn clear_retry_exhaustion(&mut self, identifier: &str) -> Result<(), Self::Error> {
        let key = crate::opensymphony_workspace::sanitize_workspace_key(identifier)?;
        let path = self
            .retry_state_root
            .join("retry-exhaustion")
            .join(format!("{key}.json"));
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(CliWorkspaceError::RetryState(format!(
                "failed to clear {}: {error}",
                path.display()
            ))),
        }
    }

    fn retain_failed_workspaces(&self) -> bool {
        self.retain_failed
    }
}

async fn replace_retry_exhaustion_marker(temporary: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        fs::rename(temporary, destination).await
    }

    #[cfg(windows)]
    {
        let temporary = temporary.to_path_buf();
        let destination = destination.to_path_buf();
        tokio::task::spawn_blocking(move || {
            replace_retry_exhaustion_marker_windows(&temporary, &destination)
        })
        .await
        .map_err(|error| io::Error::other(format!("marker replacement task failed: {error}")))?
    }
}

#[cfg(windows)]
fn replace_retry_exhaustion_marker_windows(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing_name: *const u16, new_name: *const u16, flags: u32) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let existing = temporary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let replacement = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are NUL-terminated UTF-16 buffers owned for the call.
    let replaced = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn retry_reason_for_manifest(reason: RetryReason) -> String {
    match reason {
        RetryReason::Continuation => "continuation",
        RetryReason::Failure => "failure",
        RetryReason::Stalled => "stalled",
        RetryReason::Cancelled => "cancelled",
        RetryReason::Reconciliation => "reconciliation",
    }
    .to_owned()
}

fn interrupt_reason_from_manifest(value: &str) -> Option<HarnessInterruptReason> {
    match value {
        "operator_cancel" => Some(HarnessInterruptReason::OperatorCancel),
        "tracker_merging_supersedes_human_review" => {
            Some(HarnessInterruptReason::TrackerMergingSupersedesHumanReview)
        }
        "scheduler_abort" => Some(HarnessInterruptReason::SchedulerAbort),
        _ => None,
    }
}

fn retry_reason_from_manifest(value: &str) -> Option<RetryReason> {
    match value {
        "continuation" => Some(RetryReason::Continuation),
        "failure" => Some(RetryReason::Failure),
        "stalled" => Some(RetryReason::Stalled),
        "cancelled" => Some(RetryReason::Cancelled),
        "reconciliation" => Some(RetryReason::Reconciliation),
        _ => None,
    }
}

async fn recovered_conversation_manifest(
    manager: &WorkspaceManager,
    handle: &WorkspaceHandle,
) -> Result<Option<IssueConversationManifest>, WorkspaceError> {
    let manifest_path = handle.conversation_manifest_path();
    let Some(raw_manifest) = manager.read_text_artifact(handle, &manifest_path).await? else {
        return Ok(None);
    };
    let manifest = match serde_json::from_str::<IssueConversationManifest>(&raw_manifest) {
        Ok(manifest) => manifest,
        Err(error) => {
            tracing::warn!(
                manifest = %manifest_path.display(),
                %error,
                "skipping recovered scheduler harness kind for invalid conversation manifest"
            );
            return Ok(None);
        }
    };
    Ok(Some(manifest))
}

fn recovered_harness_kind_from_manifest(manifest: &IssueConversationManifest) -> String {
    if conversation_manifest_is_codex(manifest) {
        return CODEX_APP_SERVER_KIND.to_string();
    }
    match manifest.transport_target.as_deref() {
        // Older OpenHands manifests recorded the transport mechanism rather
        // than the public harness kind. Recovery must route those records
        // through the OpenHands adapter instead of exposing an unknown kind
        // to scheduler capability validation.
        Some("loopback" | "remote" | OPENHANDS_AGENT_SERVER_KIND) => {
            return OPENHANDS_AGENT_SERVER_KIND.to_string();
        }
        Some(transport_target) => return transport_target.to_string(),
        None => {}
    }
    // Manifests written before transport_target was introduced were all
    // produced by the OpenHands-backed runtime. Keep recovery on that
    // interrupt path instead of turning a missing optional field into an
    // unknown harness that can never be stopped.
    OPENHANDS_AGENT_SERVER_KIND.to_string()
}

fn recoverable_run_manifest(
    run_manifest: &RunManifest,
    conversation_manifest: Option<&IssueConversationManifest>,
) -> bool {
    run_manifest.status == RunStatus::Running
        || (run_manifest.status == RunStatus::Prepared
            && conversation_manifest.is_some_and(|manifest| {
                conversation_manifest_is_codex(manifest)
                    && manifest
                        .last_turn_id
                        .as_deref()
                        .is_some_and(|turn_id| !turn_id.trim().is_empty())
            }))
}

fn recovered_run_from_manifests(
    run_manifest: Option<&RunManifest>,
    conversation_manifest: Option<&IssueConversationManifest>,
) -> Option<RecoveredRun> {
    let run_manifest = run_manifest?;
    let conversation_manifest = conversation_manifest?;
    let worker_id = run_manifest
        .run_id
        .strip_prefix("run-")
        .unwrap_or(run_manifest.run_id.as_str());
    let worker_id = match crate::opensymphony_domain::WorkerId::new(worker_id.to_string()) {
        Ok(worker_id) => worker_id,
        Err(error) => {
            tracing::warn!(
                run_id = %run_manifest.run_id,
                %error,
                "skipping recovered scheduler run for invalid worker id"
            );
            return None;
        }
    };
    Some(RecoveredRun {
        worker_id,
        conversation: conversation_metadata_from_manifest(conversation_manifest),
        normal_retry_count: run_manifest.normal_retry_count,
    })
}

fn conversation_metadata_from_manifest(
    manifest: &IssueConversationManifest,
) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: manifest.conversation_id.clone(),
        server_base_url: manifest.server_base_url.clone(),
        transport_target: manifest.transport_target.clone(),
        http_auth_mode: manifest.http_auth_mode.clone(),
        websocket_auth_mode: manifest.websocket_auth_mode.clone(),
        websocket_query_param_name: manifest.websocket_query_param_name.clone(),
        fresh_conversation: manifest.fresh_conversation,
        runtime_contract_version: manifest.runtime_contract_version.clone(),
        // Recovery reconstructs metadata from persisted manifests only. It is
        // not a live WebSocket attachment, so callers must reattach/reconcile
        // before treating the stream as ready.
        stream_state: RuntimeStreamState::Closed,
        last_event_id: manifest.last_event_id.clone(),
        last_event_kind: manifest.last_event_kind.clone(),
        last_event_at: manifest.last_event_at.map(datetime_to_timestamp_ms),
        last_event_summary: manifest.last_event_summary.clone(),
        recent_activity: Vec::new(),
        input_tokens: manifest.input_tokens,
        output_tokens: manifest.output_tokens,
        cache_read_tokens: manifest.cache_read_tokens,
        total_tokens: manifest.input_tokens.saturating_add(manifest.output_tokens),
        runtime_seconds: 0,
        next_activity_sequence: 0,
    }
}

impl RuntimeWorkerBackend {
    pub(super) fn new(
        client: OpenHandsClient,
        workflow: Arc<ResolvedWorkflow>,
        workspace_manager: Arc<WorkspaceManager>,
        memory_env: Option<RuntimeMemoryEnv>,
        worker_env: BTreeMap<String, String>,
    ) -> Self {
        let (updates_tx, updates_rx) = mpsc::unbounded_channel();
        let workpad_comment_source = match build_linear_client(&workflow) {
            Ok(client) => {
                Some(Arc::new(LinearWorkpadCommentSource { client })
                    as Arc<dyn WorkpadCommentSource>)
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "failed to build the Linear workpad comment source; config-drift rehydrate prompts will fall back to workspace-only recovery"
                );
                None
            }
        };
        Self {
            client,
            workflow: workflow.clone(),
            workspace_manager,
            runner_config: IssueSessionRunnerConfig::from_workflow(&workflow)
                .with_memory(memory_env.as_ref().map(memory_access_from_runtime)),
            workpad_comment_source,
            worker_env,
            codex_bin: env::var("OPENSYMPHONY_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
            codex_schema_validators: Arc::new(AsyncMutex::new(HashMap::new())),
            codex_interrupts: Arc::new(Mutex::new(HashMap::new())),
            launch_timeout: DEFAULT_WORKER_LAUNCH_TIMEOUT,
            updates_tx,
            updates_rx,
            tasks: HashMap::new(),
        }
    }

    fn abort_tracked_task(&mut self, worker_id: &str) {
        if let Some(task) = self.tasks.remove(worker_id) {
            task.handle.abort();
        }
    }

    fn abort_all_tracked_tasks(&mut self) {
        let active_count = self.tasks.len();
        if active_count == 0 {
            return;
        }

        tracing::info!(
            active_count,
            "aborting tracked worker tasks during backend shutdown"
        );
        for (_, task) in self.tasks.drain() {
            task.handle.abort();
        }
    }

    fn spawn_worker_task(&mut self, request: WorkerStartRequest, recovered: bool) -> PendingLaunch {
        let mut runner = IssueSessionRunner::with_environment(
            self.client.clone(),
            self.runner_config.clone(),
            OverlayEnvironment {
                overrides: self.worker_env.clone(),
            },
        );
        if let Some(source) = self.workpad_comment_source.clone() {
            runner = runner.with_workpad_comment_source(source);
        }
        let workspace_manager = self.workspace_manager.clone();
        let workflow = self.workflow.clone();
        let updates_tx = self.updates_tx.clone();
        let worker_id = request.run.worker_id.clone();
        let observer_worker_id = worker_id.clone();
        let finished_worker_id = worker_id.clone();
        let (launch_tx, launch_rx) = oneshot::channel();
        let run = request.run.clone();
        let route = request.route.clone();
        let recovered = recovered
            && matches!(
                route.harness_kind.as_str(),
                OPENHANDS_AGENT_SERVER_KIND | CODEX_APP_SERVER_KIND
            );
        let pending_route = route.clone();
        let codex_bin = self.codex_bin.clone();
        let worker_env = self.worker_env.clone();
        let codex_schema_validators = Arc::clone(&self.codex_schema_validators);
        let codex_interrupts = Arc::clone(&self.codex_interrupts);
        let issue = request.issue.clone();
        let launch_worker_id = worker_id.clone();
        let handle = tokio::spawn(async move {
            let mut launch_tx = Some(launch_tx);
            let ensured = match workspace_manager.ensure(&issue_descriptor(&issue)).await {
                Ok(ensured) => ensured,
                Err(error) => {
                    report_launch_failure(
                        &mut launch_tx,
                        format!("failed to ensure workspace: {error}"),
                    );
                    return;
                }
            };
            let attempt = run.attempt.map(|attempt| attempt.get()).unwrap_or(1);
            let run_descriptor = RunDescriptor::new(format!("run-{launch_worker_id}"), attempt)
                .with_normal_retry_count(run.normal_retry_count);
            let mut run_manifest = if recovered {
                match workspace_manager.load_run_manifest(&ensured.handle).await {
                    Ok(Some(run_manifest)) => {
                        let conversation_manifest = if run_manifest.status == RunStatus::Prepared
                            && route.harness_kind == CODEX_APP_SERVER_KIND
                        {
                            match load_codex_conversation_manifest(
                                &workspace_manager,
                                &ensured.handle,
                                &issue,
                            )
                            .await
                            {
                                Ok(manifest) => manifest,
                                Err(error) => {
                                    report_launch_failure(&mut launch_tx, error);
                                    return;
                                }
                            }
                        } else {
                            None
                        };
                        if recoverable_run_manifest(&run_manifest, conversation_manifest.as_ref()) {
                            run_manifest
                        } else {
                            report_launch_failure(
                                &mut launch_tx,
                                format!(
                                    "recovered workspace run is not attachable (status {})",
                                    run_manifest.status
                                ),
                            );
                            return;
                        }
                    }
                    Ok(None) => {
                        report_launch_failure(
                            &mut launch_tx,
                            "recovered workspace run manifest is missing",
                        );
                        return;
                    }
                    Err(error) => {
                        report_launch_failure(
                            &mut launch_tx,
                            format!("failed to load recovered workspace run: {error}"),
                        );
                        return;
                    }
                }
            } else {
                match workspace_manager
                    .start_run(&ensured.handle, &run_descriptor)
                    .await
                {
                    Ok(run_manifest) => run_manifest,
                    Err(error) => {
                        report_launch_failure(
                            &mut launch_tx,
                            format!("failed to prepare workspace run: {error}"),
                        );
                        return;
                    }
                }
            };

            if route.dry_run {
                if let Some(sender) = launch_tx.take() {
                    let _ = sender.send(LaunchReport::Conversation(Box::new(
                        dry_run_conversation_metadata(&run, &route),
                    )));
                }
                let finish_error = finish_route_dry_run_workspace_run(
                    &workspace_manager,
                    &ensured.handle,
                    &mut run_manifest,
                    &route,
                )
                .await
                .err();
                let outcome = WorkerOutcomeRecord::from_run(
                    &run,
                    if finish_error.is_some() {
                        WorkerOutcomeKind::Failed
                    } else {
                        WorkerOutcomeKind::Succeeded
                    },
                    now_timestamp(),
                    Some(match &finish_error {
                        Some(_) => "routing dry-run workspace finalization failed".into(),
                        None => route.summary(),
                    }),
                    finish_error.map(|error| error.to_string()),
                );
                let _ = updates_tx.send(WorkerUpdate::Finished {
                    worker_id: finished_worker_id.clone(),
                    outcome,
                });
                return;
            }

            if route.harness_kind == "codex_app_server" {
                let outcome = run_codex_stdio_issue_with_mode(
                    &route,
                    &workspace_manager,
                    &ensured.handle,
                    &mut run_manifest,
                    &issue,
                    &run,
                    &workflow,
                    &codex_bin,
                    &codex_schema_validators,
                    &codex_interrupts,
                    &updates_tx,
                    &mut launch_tx,
                    &worker_env,
                    recovered,
                )
                .await;
                let _ = updates_tx.send(WorkerUpdate::Finished {
                    worker_id: finished_worker_id.clone(),
                    outcome,
                });
                return;
            }

            let mut observer = SchedulerObserver {
                worker_id: observer_worker_id.to_string(),
                launch_tx,
                updates_tx: updates_tx.clone(),
            };
            let result = if recovered {
                runner
                    .recover_with_observer(
                        &workspace_manager,
                        &ensured.handle,
                        &mut run_manifest,
                        &issue,
                        &run,
                        &mut observer,
                    )
                    .await
            } else {
                runner
                    .run_with_observer(
                        &workspace_manager,
                        &ensured.handle,
                        &mut run_manifest,
                        &issue,
                        &run,
                        &workflow,
                        &mut observer,
                    )
                    .await
            };

            if observer.launch_tx.is_some() {
                report_launch_failure(
                    &mut observer.launch_tx,
                    pending_launch_failure_detail(&result),
                );
                return;
            }

            let outcome = match result {
                Ok(result) => result.worker_outcome,
                Err(error) => WorkerOutcomeRecord::from_run(
                    &run,
                    WorkerOutcomeKind::Failed,
                    now_timestamp(),
                    Some("worker task failed before completing".to_string()),
                    Some(error.to_string()),
                ),
            };
            let _ = updates_tx.send(WorkerUpdate::Finished {
                worker_id: finished_worker_id.clone(),
                outcome,
            });
        });

        self.tasks.insert(
            worker_id.to_string(),
            ActiveWorkerTask {
                handle,
                run: request.run,
            },
        );

        PendingLaunch {
            worker_id: worker_id.to_string(),
            route: pending_route,
            launch_rx,
        }
    }

    async fn resolve_launch_result(
        &mut self,
        worker_id: &str,
        route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
        launch_timeout: Duration,
        result: Result<
            Result<LaunchReport, oneshot::error::RecvError>,
            tokio::time::error::Elapsed,
        >,
    ) -> Result<WorkerLaunch, CliWorkerError> {
        match result {
            Ok(Ok(LaunchReport::Conversation(conversation))) => {
                let conversation = annotate_route_decision(*conversation, worker_id, route);
                Ok(WorkerLaunch { conversation })
            }
            Ok(Ok(LaunchReport::Failed(detail))) => {
                if let Some(task) = self.tasks.remove(worker_id) {
                    task.handle.await?;
                }
                Err(CliWorkerError::LaunchFailed(detail))
            }
            Ok(Err(_)) => {
                if let Some(task) = self.tasks.remove(worker_id) {
                    task.handle.await?;
                }
                Err(CliWorkerError::LaunchChannelClosed)
            }
            Err(_) => {
                self.abort_tracked_task(worker_id);
                Err(CliWorkerError::LaunchTimeout(launch_timeout))
            }
        }
    }

    fn launch_timeout_for_route(
        &self,
        route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    ) -> Duration {
        if route.harness_kind == "codex_app_server" {
            CODEX_WORKER_LAUNCH_TIMEOUT
        } else {
            self.launch_timeout
        }
    }
}

fn annotate_route_decision(
    mut conversation: ConversationMetadata,
    worker_id: &str,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> ConversationMetadata {
    conversation.observe_event(
        now_timestamp(),
        Some(format!("route-{worker_id}-{}", route.harness_kind)),
        Some("routing.decision".into()),
        Some(route.summary()),
        Some(route_decision_payload(route)),
    );
    conversation
}

fn route_decision_payload(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> serde_json::Value {
    serde_json::json!({
        "task_type": &route.task_type,
        "harness_kind": &route.harness_kind,
        "model": &route.model,
        "model_profile": &route.model_profile,
        "reason": &route.reason,
        "dry_run": route.dry_run,
        "user_override": route.user_override,
    })
}

/// Prefix of the synthetic conversation id a `--dry-run` route preview uses.
/// A real conversation/Codex thread id never starts with this, so it marks
/// preview metadata that must not be surfaced as a resumable Codex thread.
pub(super) const ROUTE_PREVIEW_CONVERSATION_PREFIX: &str = "route-preview-";

fn dry_run_conversation_metadata(
    run: &crate::opensymphony_domain::RunAttempt,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: ConversationId::new(format!(
            "{ROUTE_PREVIEW_CONVERSATION_PREFIX}{}",
            run.worker_id
        ))
        .expect("route preview conversation id should not be empty"),
        server_base_url: None,
        transport_target: Some(route.harness_kind.clone()),
        http_auth_mode: None,
        websocket_auth_mode: None,
        websocket_query_param_name: None,
        fresh_conversation: true,
        runtime_contract_version: Some("opensymphony-routing-alpha-v1".into()),
        stream_state: RuntimeStreamState::Closed,
        last_event_id: None,
        last_event_kind: None,
        last_event_at: None,
        last_event_summary: None,
        recent_activity: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
        runtime_seconds: 0,
        next_activity_sequence: 0,
    }
}

async fn finish_route_dry_run_workspace_run(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> Result<(), WorkspaceError> {
    run_manifest.status = RunStatus::Succeeded;
    run_manifest.status_detail = Some(format!("routing dry-run ended: {}", route.summary()));
    workspace_manager
        .finish_run(workspace, run_manifest, RunStatus::Succeeded)
        .await
}

fn inject_memory_env(env: &mut BTreeMap<String, String>, memory: &RuntimeMemoryEnv) {
    env.insert(
        "OPENSYMPHONY_MEMORY_ENDPOINT".to_string(),
        memory.endpoint.clone(),
    );
    env.insert(
        "OPENSYMPHONY_MEMORY_PROJECT".to_string(),
        memory.project.clone(),
    );
    env.insert(
        "OPENSYMPHONY_MEMORY_PROJECT_SET".to_string(),
        memory.project.clone(),
    );
    env.insert(
        "OPENSYMPHONY_MEMORY_EXECUTION_REPO".to_string(),
        memory.execution_repo.clone(),
    );
    if let Some(token) = &memory.token {
        env.insert("OPENSYMPHONY_MEMORY_TOKEN".to_string(), token.clone());
    }
}

fn memory_access_from_runtime(memory: &RuntimeMemoryEnv) -> MemoryWorkerAccess {
    MemoryWorkerAccess {
        endpoint: memory.endpoint.clone(),
        token: memory.token.clone(),
        project: Some(memory.project.clone()),
        execution_repo: Some(memory.execution_repo.clone()),
    }
}

impl Drop for RuntimeWorkerBackend {
    fn drop(&mut self) {
        self.abort_all_tracked_tasks();
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
async fn run_codex_stdio_issue(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    workflow: &ResolvedWorkflow,
    codex_bin: &str,
    codex_schema_validators: &CodexSchemaValidatorCache,
    codex_interrupts: &CodexInterruptRegistry,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    launch_tx: &mut Option<oneshot::Sender<LaunchReport>>,
    worker_env: &BTreeMap<String, String>,
) -> WorkerOutcomeRecord {
    run_codex_stdio_issue_with_mode(
        route,
        workspace_manager,
        workspace,
        run_manifest,
        issue,
        run,
        workflow,
        codex_bin,
        codex_schema_validators,
        codex_interrupts,
        updates_tx,
        launch_tx,
        worker_env,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_codex_stdio_issue_with_mode(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    workflow: &ResolvedWorkflow,
    codex_bin: &str,
    codex_schema_validators: &CodexSchemaValidatorCache,
    codex_interrupts: &CodexInterruptRegistry,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    launch_tx: &mut Option<oneshot::Sender<LaunchReport>>,
    worker_env: &BTreeMap<String, String>,
    recovered: bool,
) -> WorkerOutcomeRecord {
    match try_run_codex_stdio_issue(
        route,
        workspace_manager,
        workspace,
        run_manifest,
        issue,
        run,
        workflow,
        codex_bin,
        codex_schema_validators,
        codex_interrupts,
        updates_tx,
        launch_tx,
        worker_env,
        recovered,
    )
    .await
    {
        Ok((outcome, status)) => {
            match finish_codex_workspace_run(workspace_manager, workspace, run_manifest, status)
                .await
            {
                Ok(()) => outcome,
                Err(error) => {
                    let detail = record_codex_finish_failure(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        status,
                        error,
                    )
                    .await;
                    WorkerOutcomeRecord::from_run(
                        run,
                        WorkerOutcomeKind::Failed,
                        now_timestamp(),
                        Some("Codex app-server workspace finalization failed".into()),
                        Some(detail),
                    )
                }
            }
        }
        Err(error) => {
            let mut detail = error.clone();
            if let Err(finish_error) = finish_codex_workspace_run(
                workspace_manager,
                workspace,
                run_manifest,
                RunStatus::Failed,
            )
            .await
            {
                let finish_detail = record_codex_finish_failure(
                    workspace_manager,
                    workspace,
                    run_manifest,
                    RunStatus::Failed,
                    finish_error,
                )
                .await;
                detail = format!("{detail}; {finish_detail}");
            }
            if launch_tx.is_some() {
                report_launch_failure(launch_tx, detail.clone());
            }
            WorkerOutcomeRecord::from_run(
                run,
                WorkerOutcomeKind::Failed,
                now_timestamp(),
                Some("Codex app-server worker failed".into()),
                Some(detail),
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn try_run_codex_stdio_issue(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    workflow: &ResolvedWorkflow,
    codex_bin: &str,
    codex_schema_validators: &CodexSchemaValidatorCache,
    codex_interrupts: &CodexInterruptRegistry,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    launch_tx: &mut Option<oneshot::Sender<LaunchReport>>,
    worker_env: &BTreeMap<String, String>,
    recovered: bool,
) -> Result<(WorkerOutcomeRecord, RunStatus), String> {
    let adapter =
        CodexAppServerAdapter::local_stdio(codex_bin, "opensymphony", env!("CARGO_PKG_VERSION"));
    let schema_validator =
        cached_installed_codex_schema_validator(codex_schema_validators, codex_bin).await?;
    let (program, args) = adapter.launch().to_command();
    let mut child = Command::new(&program)
        .args(args)
        .current_dir(workspace.workspace_path())
        .envs(worker_env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|source| {
            format!(
                "failed to launch `{program} --dangerously-bypass-hook-trust app-server --stdio`: {source}"
            )
        })?;
    let mut stdin = child.stdin.take().ok_or("Codex child stdin missing")?;
    let stdout = child.stdout.take().ok_or("Codex child stdout missing")?;
    let stderr = child.stderr.take().ok_or("Codex child stderr missing")?;
    let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
    let mut stderr_task = AbortOnDrop::new(tokio::spawn(drain_codex_stderr(
        stderr,
        run.worker_id.to_string(),
        Arc::clone(&stderr_tail),
    )));
    let mut reader = BufReader::new(stdout).lines();
    let mut session = adapter.session();
    let mut read_state = CodexReadState::default();
    let interrupt_responses = Arc::new(Mutex::new(HashMap::new()));

    let initialize = session.initialize();
    write_codex_request(
        &mut stdin,
        &schema_validator,
        &initialize,
        "initialize",
        &stderr_tail,
    )
    .await?;
    read_response_line(
        &mut reader,
        initialize.id,
        updates_tx,
        &run.worker_id.to_string(),
        issue,
        run,
        &mut read_state,
    )
    .await
    .map_err(|error| with_codex_stderr(error, &stderr_tail))?;

    let model = codex_model_from_route(route);
    let mut existing_manifest =
        load_codex_conversation_manifest(workspace_manager, workspace, issue)
            .await
            .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
    if let Some(manifest) = existing_manifest.as_mut() {
        ensure_codex_thread_active(workspace_manager, workspace, manifest, codex_bin)
            .await
            .map_err(|error| {
                codex_lifecycle_error(
                    issue,
                    Some(manifest.conversation_id.as_str()),
                    "archive recovery",
                    error,
                )
            })?;
    }
    if recovered
        && run_manifest.status == RunStatus::Prepared
        && existing_manifest.as_ref().is_some_and(|manifest| {
            manifest
                .last_turn_id
                .as_deref()
                .is_some_and(|turn_id| !turn_id.trim().is_empty())
        })
    {
        run_manifest.status = RunStatus::Running;
        run_manifest.status_detail = Some("reattaching to an active Codex turn".to_owned());
        run_manifest.updated_at = chrono::Utc::now();
        workspace_manager
            .write_run_manifest(workspace, run_manifest)
            .await
            .map_err(|error| format!("failed to persist recovered Codex run state: {error}"))?;
    }
    if recovered && existing_manifest.is_none() {
        return Err(codex_lifecycle_error(
            issue,
            None,
            "recovery",
            "persisted Codex conversation manifest is missing; refusing to create a new thread",
        ));
    }
    let first_run_prompt = if existing_manifest.is_none() {
        Some(
            workflow
                .render_prompt(issue, run.attempt.map(|attempt| attempt.get()))
                .map_err(|source| {
                    format!("failed to render workflow prompt for Codex route: {source}")
                })?,
        )
    } else {
        None
    };
    let mut resume_terminal = None;
    let (conversation_id, mut manifest, prompt_kind, fresh_conversation) = match existing_manifest {
        Some(manifest) => {
            let conversation_id = manifest.conversation_id.to_string();
            let resume = adapter
                .resume_issue_request(
                    &mut session,
                    conversation_id.clone(),
                    workspace.workspace_path().display().to_string(),
                    model.clone(),
                )
                .map_err(|source| {
                    codex_lifecycle_error(
                        issue,
                        Some(&conversation_id),
                        "thread/resume request",
                        source,
                    )
                })?;
            write_codex_request(
                &mut stdin,
                &schema_validator,
                &resume.request,
                "thread/resume",
                &stderr_tail,
            )
            .await
            .map_err(|error| {
                codex_lifecycle_error(
                    issue,
                    Some(&conversation_id),
                    "thread/resume",
                    with_codex_stderr(error, &stderr_tail),
                )
            })?;
            let resume_response = read_response_line(
                &mut reader,
                resume.request.id,
                updates_tx,
                &run.worker_id.to_string(),
                issue,
                run,
                &mut read_state,
            )
            .await
            .map_err(|error| {
                codex_lifecycle_error(
                    issue,
                    Some(&conversation_id),
                    "thread/resume",
                    with_codex_stderr(error, &stderr_tail),
                )
            })?;
            let resumed_thread_id =
                codex_thread_id_from_response(&resume_response).map_err(|error| {
                    codex_lifecycle_error(
                        issue,
                        Some(&conversation_id),
                        "thread/resume response validation",
                        error,
                    )
                })?;
            if resumed_thread_id != conversation_id {
                return Err(codex_lifecycle_error(
                    issue,
                    Some(&conversation_id),
                    "thread/resume response validation",
                    format!("returned thread id `{resumed_thread_id}` instead"),
                ));
            }
            resume_terminal = manifest.last_turn_id.as_deref().and_then(|turn_id| {
                codex_terminal_outcome_from_resume_response(&resume_response, turn_id)
            });
            let prompt_kind = if manifest.workflow_prompt_seeded {
                IssueSessionPromptKind::Continuation
            } else {
                IssueSessionPromptKind::Full
            };
            (conversation_id, manifest, prompt_kind, false)
        }
        None => {
            let thread_start = adapter
                .start_issue_thread_request(
                    &mut session,
                    workspace.workspace_path().display().to_string(),
                    model.clone(),
                    serde_json::json!({
                        "opensymphonyRoute": {
                            "harness": &route.harness_kind,
                            "model": &model,
                            "modelProfile": &route.model_profile,
                            "reason": &route.reason,
                        }
                    }),
                )
                .map_err(|source| {
                    format!("failed to build Codex thread/start request: {source}")
                })?;
            write_codex_request(
                &mut stdin,
                &schema_validator,
                &thread_start.request,
                "thread/start",
                &stderr_tail,
            )
            .await?;
            let thread_start_response = read_response_line(
                &mut reader,
                thread_start.request.id,
                updates_tx,
                &run.worker_id.to_string(),
                issue,
                run,
                &mut read_state,
            )
            .await
            .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
            let conversation_id = codex_thread_id_from_response(&thread_start_response)
                .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
            let manifest = match write_codex_conversation_manifest(
                workspace_manager,
                workspace,
                issue,
                &conversation_id,
                route,
            )
            .await
            {
                Ok(manifest) => manifest,
                Err(error) => {
                    let rollback = match adapter
                        .archive_issue_thread_request(&mut session, conversation_id.clone())
                    {
                        Ok(archive) => match write_codex_request(
                            &mut stdin,
                            &schema_validator,
                            &archive.request,
                            "thread/archive rollback",
                            &stderr_tail,
                        )
                        .await
                        {
                            Ok(()) => match read_response_line(
                                &mut reader,
                                archive.request.id,
                                updates_tx,
                                &run.worker_id.to_string(),
                                issue,
                                run,
                                &mut read_state,
                            )
                            .await
                            {
                                Ok(_) => "rollback archive accepted".to_string(),
                                Err(rollback_error) => format!(
                                    "rollback archive response failed: {}",
                                    with_codex_stderr(rollback_error, &stderr_tail)
                                ),
                            },
                            Err(rollback_error) => format!(
                                "rollback archive request failed: {}",
                                with_codex_stderr(rollback_error, &stderr_tail)
                            ),
                        },
                        Err(rollback_error) => {
                            format!("rollback archive could not be built: {rollback_error}")
                        }
                    };
                    return Err(codex_lifecycle_error(
                        issue,
                        Some(&conversation_id),
                        "initial manifest persistence",
                        format!("{error}; {rollback}"),
                    ));
                }
            };
            (
                conversation_id,
                manifest,
                IssueSessionPromptKind::Full,
                true,
            )
        }
    };
    if recovered {
        let turn_id = manifest
            .last_turn_id
            .clone()
            .filter(|turn_id| !turn_id.trim().is_empty())
            .ok_or_else(|| {
                codex_lifecycle_error(
                    issue,
                    Some(&conversation_id),
                    "recovery",
                    "persisted Codex turn id is missing; refusing to start a new turn",
                )
            })?;
        if let Some(outcome) = resume_terminal {
            read_state.pending_terminal = Some(outcome);
        }
        let _interrupt_registration = register_codex_interrupt_channel(
            codex_interrupts,
            conversation_id.clone(),
            CodexInterruptChannel {
                stdin,
                session,
                schema_validator,
                thread_id: conversation_id.clone(),
                turn_id,
                responses: Arc::clone(&interrupt_responses),
                stderr_tail: Arc::clone(&stderr_tail),
            },
        )?;
        if let Some(sender) = launch_tx.take() {
            let _ = sender.send(LaunchReport::Conversation(Box::new(
                codex_conversation_metadata(conversation_id.clone(), route),
            )));
        }
        let terminal = read_until_codex_terminal(
            &mut reader,
            updates_tx,
            &run.worker_id.to_string(),
            issue,
            run,
            &mut read_state,
            &interrupt_responses,
        )
        .await
        .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
        let summary = format!(
            "Codex app-server recovery completed with terminal event {:?}",
            terminal.event_kind
        );
        let _ = child.kill().await;
        stderr_task.abort();
        return Ok((
            WorkerOutcomeRecord::from_run(
                run,
                terminal.outcome,
                now_timestamp(),
                Some(summary),
                None,
            ),
            terminal.status,
        ));
    }
    let prompt = match (prompt_kind, first_run_prompt) {
        (IssueSessionPromptKind::Full, Some(prompt)) => prompt,
        (IssueSessionPromptKind::Full, None) => workflow
            .render_prompt(issue, run.attempt.map(|attempt| attempt.get()))
            .map_err(|source| {
                format!("failed to render workflow prompt for Codex route: {source}")
            })?,
        (IssueSessionPromptKind::Continuation, _) => build_continuation_guidance(issue, run),
    };
    let turn_start = adapter
        .start_issue_turn_request(
            &mut session,
            conversation_id.clone(),
            workspace.workspace_path().display().to_string(),
            model,
            prompt,
        )
        .map_err(|source| format!("failed to build Codex turn/start request: {source}"))?;
    write_codex_request(
        &mut stdin,
        &schema_validator,
        &turn_start.request,
        "turn/start",
        &stderr_tail,
    )
    .await?;
    let turn_start_response = read_response_line(
        &mut reader,
        turn_start.request.id,
        updates_tx,
        &run.worker_id.to_string(),
        issue,
        run,
        &mut read_state,
    )
    .await
    .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
    update_codex_conversation_manifest(
        workspace_manager,
        workspace,
        &mut manifest,
        prompt_kind,
        fresh_conversation,
        route,
    )
    .await
    .map_err(|error| {
        codex_lifecycle_error(
            issue,
            Some(&conversation_id),
            "turn/start manifest update",
            with_codex_stderr(error, &stderr_tail),
        )
    })?;
    let turn_id = match codex_turn_id_from_start_response(&turn_start_response)
        .or_else(|| read_state.pending_turn_id.take())
    {
        Some(turn_id) => turn_id,
        None => read_until_codex_turn_id(
            &mut reader,
            updates_tx,
            &run.worker_id.to_string(),
            issue,
            run,
            &mut read_state,
        )
        .await
        .map_err(|error| with_codex_stderr(error, &stderr_tail))?,
    };
    persist_codex_turn_id(workspace_manager, workspace, &mut manifest, &turn_id)
        .await
        .map_err(|error| {
            codex_lifecycle_error(
                issue,
                Some(&conversation_id),
                "turn/start turn id persistence",
                with_codex_stderr(error, &stderr_tail),
            )
        })?;
    if !recovered {
        persist_codex_run_started(
            workspace_manager,
            workspace,
            run_manifest,
            &conversation_id,
            prompt_kind,
        )
        .await
        .map_err(|error| {
            codex_lifecycle_error(
                issue,
                Some(&conversation_id),
                "run status persistence",
                with_codex_stderr(error, &stderr_tail),
            )
        })?;
    }
    let _interrupt_registration = register_codex_interrupt_channel(
        codex_interrupts,
        conversation_id.clone(),
        CodexInterruptChannel {
            stdin,
            session,
            schema_validator,
            thread_id: conversation_id.clone(),
            turn_id: turn_id.clone(),
            responses: Arc::clone(&interrupt_responses),
            stderr_tail: Arc::clone(&stderr_tail),
        },
    )?;
    if let Some(sender) = launch_tx.take() {
        let _ = sender.send(LaunchReport::Conversation(Box::new(
            codex_conversation_metadata(conversation_id.clone(), route),
        )));
    }

    let terminal = read_until_codex_terminal(
        &mut reader,
        updates_tx,
        &run.worker_id.to_string(),
        issue,
        run,
        &mut read_state,
        &interrupt_responses,
    )
    .await
    .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
    let summary = format!(
        "Codex app-server route completed with terminal event {:?}",
        terminal.event_kind
    );
    let _ = child.kill().await;
    stderr_task.abort();
    Ok((
        WorkerOutcomeRecord::from_run(run, terminal.outcome, now_timestamp(), Some(summary), None),
        terminal.status,
    ))
}

async fn cached_installed_codex_schema_validator(
    cache: &CodexSchemaValidatorCache,
    codex_bin: &str,
) -> Result<CodexAppServerSchemaValidator, String> {
    let key = codex_schema_cache_key(codex_bin).await;
    if let Some(validator) = cache.lock().await.get(&key).cloned() {
        return Ok(validator);
    }

    let validator = load_installed_codex_schema_validator(codex_bin).await?;
    let mut validators = cache.lock().await;
    let validator = validators.entry(key).or_insert(validator);
    Ok(validator.clone())
}

async fn codex_schema_cache_key(codex_bin: &str) -> String {
    let Some(fingerprint) = codex_binary_fingerprint(codex_bin).await else {
        return format!("{codex_bin}|unfingerprinted");
    };
    format!("{codex_bin}|{fingerprint}")
}

async fn codex_binary_fingerprint(codex_bin: &str) -> Option<String> {
    let executable = resolve_executable_path(codex_bin)?;
    let metadata = fs::metadata(&executable).await.ok()?;
    let canonical = fs::canonicalize(&executable).await.unwrap_or(executable);
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    Some(format!(
        "{}:{}:{modified}",
        canonical.display(),
        metadata.len()
    ))
}

fn resolve_executable_path(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() || program.contains('/') || program.contains('\\') {
        return Some(path.to_path_buf());
    }

    for dir in env::split_paths(&env::var_os("PATH")?) {
        let candidate = dir.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let Some(pathext) = env::var_os("PATHEXT") else {
                continue;
            };
            for extension in env::split_paths(&pathext) {
                let candidate = dir.join(format!("{program}{}", extension.to_string_lossy()));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

async fn load_installed_codex_schema_validator(
    codex_bin: &str,
) -> Result<CodexAppServerSchemaValidator, String> {
    let schema_dir = tempfile::tempdir()
        .map_err(|source| format!("failed to create Codex schema tempdir: {source}"))?;
    let generation =
        CodexContractGeneration::json_schema_with_program(codex_bin, schema_dir.path());
    let (program, args) = generation.to_command();
    let output = timeout(CODEX_SCHEMA_GENERATION_TIMEOUT, async {
        let mut command = Command::new(&program);
        command.args(&args).kill_on_drop(true);
        command.output().await
    })
    .await
    .map_err(|_| {
        format!(
            "timed out after {}s generating Codex app-server JSON schema with `{program} {}`. Update Codex to a compatible app-server build.",
            CODEX_SCHEMA_GENERATION_TIMEOUT.as_secs(),
            args.join(" ")
        )
    })?
        .map_err(|source| {
            format!(
                "failed to generate Codex app-server JSON schema with `{program} {}`: {source}. Update Codex to a build that supports `codex app-server generate-json-schema`.",
                args.join(" ")
            )
        })?;
    if !output.status.success() {
        let stderr_preview = codex_schema_stderr_preview(&output.stderr)
            .map(|preview| format!(" stderr preview: {preview}."))
            .unwrap_or_default();
        return Err(format!(
            "Codex app-server JSON schema generation failed with status {} and {} stderr byte(s).{} Update Codex to a compatible app-server build.",
            output.status,
            output.stderr.len(),
            stderr_preview
        ));
    }
    let schema_path = schema_dir
        .path()
        .join("codex_app_server_protocol.v2.schemas.json");
    CodexAppServerSchemaValidator::from_schema_file(&schema_path).map_err(|source| {
        format!(
            "failed to compile installed Codex app-server schema from {}: {source}",
            schema_path.display()
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexArchiveState {
    Active,
    Archived,
    Missing,
}

struct CodexLifecycleSession {
    child: tokio::process::Child,
    stdin: ChildStdin,
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    adapter: CodexAppServerAdapter,
    session: CodexJsonRpcSession,
    validator: CodexAppServerSchemaValidator,
}

impl CodexLifecycleSession {
    async fn start(codex_bin: &str, cwd: &Path) -> Result<Self, String> {
        let adapter = CodexAppServerAdapter::local_stdio(
            codex_bin,
            "opensymphony",
            env!("CARGO_PKG_VERSION"),
        );
        let validator = load_installed_codex_schema_validator(codex_bin).await?;
        let (program, args) = adapter.launch().to_command();
        let mut child = Command::new(&program)
            .args(args)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| format!("failed to launch Codex lifecycle app-server: {source}"))?;
        let mut stdin = child.stdin.take().ok_or("Codex lifecycle stdin missing")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Codex lifecycle stdout missing")?;
        let mut reader = BufReader::new(stdout).lines();
        let mut session = adapter.session();
        let initialize = session.initialize();
        write_codex_lifecycle_request(&mut stdin, &validator, &initialize, "initialize").await?;
        read_codex_lifecycle_response(&mut reader, initialize.id).await?;
        Ok(Self {
            child,
            stdin,
            reader,
            adapter,
            session,
            validator,
        })
    }

    async fn request<F>(&mut self, operation: &str, build: F) -> Result<serde_json::Value, String>
    where
        F: FnOnce(
            &CodexAppServerAdapter,
            &mut CodexJsonRpcSession,
        )
            -> Result<crate::opensymphony_codex::CodexHarnessRequest, serde_json::Error>,
    {
        let request = build(&self.adapter, &mut self.session)
            .map_err(|source| format!("failed to build Codex {operation} request: {source}"))?;
        write_codex_lifecycle_request(
            &mut self.stdin,
            &self.validator,
            &request.request,
            operation,
        )
        .await?;
        read_codex_lifecycle_response(&mut self.reader, request.request.id).await
    }

    async fn stop(&mut self) {
        let _ = self.child.kill().await;
    }
}

async fn send_codex_lifecycle_request<F>(
    codex_bin: &str,
    cwd: &Path,
    operation: &str,
    build: F,
) -> Result<serde_json::Value, String>
where
    F: FnOnce(
        &CodexAppServerAdapter,
        &mut CodexJsonRpcSession,
    ) -> Result<crate::opensymphony_codex::CodexHarnessRequest, serde_json::Error>,
{
    let mut session = CodexLifecycleSession::start(codex_bin, cwd).await?;
    let response = session.request(operation, build).await;
    session.stop().await;
    response
}

async fn write_codex_lifecycle_request(
    stdin: &mut ChildStdin,
    validator: &CodexAppServerSchemaValidator,
    request: &JsonRpcRequestEnvelope,
    operation: &str,
) -> Result<(), String> {
    validator
        .validate_request(request)
        .map_err(|source| source.to_string())?;
    stdin
        .write_all(
            CodexJsonRpcSession::encode_line(request)
                .map_err(|source| source.to_string())?
                .as_bytes(),
        )
        .await
        .map_err(|source| format!("failed to write Codex {operation} request: {source}"))?;
    stdin
        .flush()
        .await
        .map_err(|source| format!("failed to flush Codex {operation} request: {source}"))
}

async fn read_codex_lifecycle_response(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: u64,
) -> Result<serde_json::Value, String> {
    let deadline = tokio::time::Instant::now() + CODEX_RESPONSE_TIMEOUT;
    loop {
        let line = timeout_at(deadline, reader.next_line())
            .await
            .map_err(|_| format!("timed out waiting for Codex lifecycle response id {request_id}"))?
            .map_err(|source| format!("failed reading Codex lifecycle stdout: {source}"))?
            .ok_or_else(|| {
                format!("Codex lifecycle stdout closed before response id {request_id}")
            })?;
        let value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|source| format!("invalid Codex lifecycle JSON: {source}"))?;
        if codex_response_id_matches(&value, request_id) {
            reject_codex_json_rpc_error(request_id, &value)?;
            return Ok(value);
        }
    }
}

async fn inspect_codex_archive_state(
    codex_bin: &str,
    workspace: &WorkspaceHandle,
    thread_id: &str,
) -> Result<CodexArchiveState, String> {
    let mut session = CodexLifecycleSession::start(codex_bin, workspace.workspace_path()).await?;
    let result: Result<CodexArchiveState, String> = async {
        for archived in [true, false] {
            let mut cursor = None;
            loop {
                let response = session
                    .request("thread/list", |adapter, session| {
                        adapter.list_issue_threads_request(
                            session,
                            workspace.workspace_path().display().to_string(),
                            archived,
                            cursor.clone(),
                        )
                    })
                    .await?;
                if response["result"]["data"]
                    .as_array()
                    .is_some_and(|threads| {
                        threads.iter().any(|thread| {
                            thread.get("id").and_then(serde_json::Value::as_str) == Some(thread_id)
                        })
                    })
                {
                    return Ok(if archived {
                        CodexArchiveState::Archived
                    } else {
                        CodexArchiveState::Active
                    });
                }
                cursor = response["result"]["nextCursor"]
                    .as_str()
                    .map(ToOwned::to_owned);
                if cursor.is_none() {
                    break;
                }
            }
        }
        Ok(CodexArchiveState::Missing)
    }
    .await;
    session.stop().await;
    result
}

async fn persist_codex_archive_state(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut IssueConversationManifest,
    state: &str,
) -> Result<(), String> {
    manifest.codex_archive_state = Some(state.to_string());
    manifest.updated_at = chrono::Utc::now();
    manager
        .write_json_artifact(workspace, &workspace.conversation_manifest_path(), manifest)
        .await
        .map_err(|error| error.to_string())
}

async fn ensure_codex_thread_active(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut IssueConversationManifest,
    codex_bin: &str,
) -> Result<(), String> {
    let thread_id = manifest.conversation_id.to_string();
    match inspect_codex_archive_state(codex_bin, workspace, &thread_id).await? {
        CodexArchiveState::Active if manifest.codex_archive_state.as_deref() == Some("active") => {
            Ok(())
        }
        CodexArchiveState::Active => {
            persist_codex_archive_state(manager, workspace, manifest, "active").await
        }
        CodexArchiveState::Archived => {
            persist_codex_archive_state(manager, workspace, manifest, "unarchiving").await?;
            send_codex_lifecycle_request(
                codex_bin,
                workspace.workspace_path(),
                "thread/unarchive",
                |adapter, session| {
                    adapter.unarchive_issue_thread_request(session, thread_id.clone())
                },
            )
            .await?;
            persist_codex_archive_state(manager, workspace, manifest, "active").await
        }
        CodexArchiveState::Missing => Err(format!(
            "canonical Codex thread {thread_id} is missing; recover manually with `codex resume {thread_id}`"
        )),
    }
}

async fn archive_terminal_codex_thread(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut IssueConversationManifest,
    codex_bin: &str,
) -> Result<(), String> {
    let thread_id = manifest.conversation_id.to_string();
    match inspect_codex_archive_state(codex_bin, workspace, &thread_id).await? {
        CodexArchiveState::Archived => {
            if manifest.codex_archive_state.as_deref() == Some("archived") {
                Ok(())
            } else {
                persist_codex_archive_state(manager, workspace, manifest, "archived").await
            }
        }
        CodexArchiveState::Active => {
            persist_codex_archive_state(manager, workspace, manifest, "archiving").await?;
            send_codex_lifecycle_request(
                codex_bin,
                workspace.workspace_path(),
                "thread/archive",
                |adapter, session| adapter.archive_issue_thread_request(session, thread_id.clone()),
            )
            .await?;
            persist_codex_archive_state(manager, workspace, manifest, "archived").await
        }
        CodexArchiveState::Missing => Err(format!(
            "canonical Codex thread {thread_id} is missing; preserving workspace for repair"
        )),
    }
}

fn codex_schema_stderr_preview(stderr: &[u8]) -> Option<String> {
    if stderr.is_empty() {
        return None;
    }
    let decoded = String::from_utf8_lossy(stderr);
    let mut chars = decoded.chars();
    let mut preview = chars
        .by_ref()
        .take(CODEX_SCHEMA_STDERR_PREVIEW_CHARS)
        .map(|character| match character {
            '\n' | '\t' => character,
            character if character.is_control() => ' ',
            character => character,
        })
        .collect::<String>();
    if chars.next().is_some() {
        preview.push_str("...");
    }
    Some(format!("{preview:?}"))
}

async fn write_codex_request(
    stdin: &mut ChildStdin,
    schema_validator: &CodexAppServerSchemaValidator,
    request: &JsonRpcRequestEnvelope,
    request_name: &str,
    stderr_tail: &Arc<Mutex<VecDeque<String>>>,
) -> Result<(), String> {
    schema_validator
        .validate_request(request)
        .map_err(|source| with_codex_stderr(source.to_string(), stderr_tail))?;
    stdin
        .write_all(
            CodexJsonRpcSession::encode_line(request)
                .map_err(|source| source.to_string())?
                .as_bytes(),
        )
        .await
        .map_err(|source| {
            with_codex_stderr(
                format!("failed to write Codex {request_name} request: {source}"),
                stderr_tail,
            )
        })?;
    stdin.flush().await.map_err(|source| {
        with_codex_stderr(
            format!("failed to flush Codex {request_name} request: {source}"),
            stderr_tail,
        )
    })
}

async fn send_codex_stdio_interrupt(
    registry: &CodexInterruptRegistry,
    command: &crate::opensymphony_domain::HarnessInterruptCommand,
) -> Result<WorkerInterruptAcknowledgement, CliWorkerError> {
    let thread_id = command.conversation_id.as_str();
    let channel = registry
        .lock()
        .map_err(|_| {
            CliWorkerError::InterruptFailed("Codex interrupt registry lock poisoned".to_string())
        })?
        .get(thread_id)
        .cloned();
    let Some(channel) = channel else {
        return Err(CliWorkerError::InterruptFailed(format!(
            "Codex stdio worker for thread `{thread_id}` does not have an active turn interrupt channel"
        )));
    };

    let mut channel = channel.lock().await;
    let detail = channel
        .send_interrupt()
        .await
        .map_err(CliWorkerError::InterruptFailed)?;
    Ok(WorkerInterruptAcknowledgement {
        accepted: true,
        detail: Some(detail),
        timed_out: false,
    })
}

impl CodexInterruptChannel {
    async fn send_interrupt(&mut self) -> Result<String, String> {
        let request = self.session.request(
            "turn/interrupt",
            serde_json::json!({
                "threadId": &self.thread_id,
                "turnId": &self.turn_id,
            }),
        );
        let request_id = request.id;
        let (response_tx, response_rx) = oneshot::channel();
        self.insert_response_waiter(request_id, response_tx)?;
        write_codex_request(
            &mut self.stdin,
            &self.schema_validator,
            &request,
            "turn/interrupt",
            &self.stderr_tail,
        )
        .await
        .inspect_err(|_| self.remove_response_waiter(request_id))?;
        let response = timeout(CODEX_RESPONSE_TIMEOUT, response_rx)
            .await
            .map_err(|_| {
                self.remove_response_waiter(request_id);
                format!("timed out waiting for Codex interrupt response id {request_id}")
            })?
            .map_err(|_| {
                format!("Codex interrupt response channel closed for request id {request_id}")
            })?;
        response?;
        Ok(format!(
            "Codex interrupt acknowledged with `turn/interrupt` for thread `{}` turn `{}` (request id {request_id})",
            self.thread_id, self.turn_id
        ))
    }

    fn insert_response_waiter(
        &self,
        request_id: u64,
        sender: oneshot::Sender<Result<(), String>>,
    ) -> Result<(), String> {
        self.responses
            .lock()
            .map_err(|_| "Codex interrupt response registry lock poisoned".to_string())?
            .insert(request_id, sender);
        Ok(())
    }

    fn remove_response_waiter(&self, request_id: u64) {
        if let Ok(mut responses) = self.responses.lock() {
            responses.remove(&request_id);
        }
    }
}

fn register_codex_interrupt_channel(
    registry: &CodexInterruptRegistry,
    thread_id: String,
    channel: CodexInterruptChannel,
) -> Result<CodexInterruptRegistration, String> {
    registry
        .lock()
        .map_err(|_| "Codex interrupt registry lock poisoned".to_string())?
        .insert(thread_id.clone(), Arc::new(AsyncMutex::new(channel)));
    Ok(CodexInterruptRegistration {
        registry: Arc::clone(registry),
        thread_id,
    })
}

impl Drop for CodexInterruptRegistration {
    fn drop(&mut self) {
        if let Ok(mut registry) = self.registry.lock() {
            registry.remove(&self.thread_id);
        }
    }
}

struct AbortOnDrop<T> {
    handle: Option<JoinHandle<T>>,
}

impl<T> AbortOnDrop<T> {
    fn new(handle: JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    fn abort(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.abort();
    }
}

fn codex_model_from_route(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> Option<String> {
    route.model.clone()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexThreadResponse {
    #[serde(default, alias = "thread_id")]
    thread_id: Option<String>,
    #[serde(default)]
    thread: Option<CodexThreadResponseThread>,
}

#[derive(serde::Deserialize)]
struct CodexThreadResponseThread {
    id: String,
}

fn codex_thread_id_from_response(value: &serde_json::Value) -> Result<String, String> {
    let result = value
        .get("result")
        .cloned()
        .ok_or_else(|| "Codex thread response did not include a result object".to_string())?;
    let response: CodexThreadResponse = serde_json::from_value(result)
        .map_err(|error| format!("Codex thread response had an invalid result shape: {error}"))?;
    response
        .thread_id
        .or_else(|| response.thread.map(|thread| thread.id))
        .filter(|thread_id| !thread_id.trim().is_empty())
        .ok_or_else(|| "Codex thread response missing a non-empty thread id".to_string())
}

fn codex_terminal_outcome_from_resume_response(
    value: &serde_json::Value,
    expected_turn_id: &str,
) -> Option<CodexTerminalOutcome> {
    let result = value.get("result")?;
    let turn = result
        .get("thread")
        .and_then(|thread| thread.get("turns"))
        .or_else(|| result.get("turns"))?
        .as_array()?
        .iter()
        .find(|turn| {
            turn.get("id")
                .or_else(|| turn.get("turnId"))
                .and_then(serde_json::Value::as_str)
                == Some(expected_turn_id)
        })?;
    let status = turn
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase)?;
    let (outcome, status) = match status.as_str() {
        "completed" | "succeeded" | "success" => {
            (WorkerOutcomeKind::Succeeded, RunStatus::Succeeded)
        }
        "failed" | "error" => (WorkerOutcomeKind::Failed, RunStatus::Failed),
        "cancelled" | "canceled" | "interrupted" => {
            (WorkerOutcomeKind::Cancelled, RunStatus::Cancelled)
        }
        _ => return None,
    };
    Some(CodexTerminalOutcome {
        event_kind: NormalizedCodexEventKind::TurnCompleted,
        outcome,
        status,
    })
}

fn codex_lifecycle_error(
    issue: &NormalizedIssue,
    thread_id: Option<&str>,
    operation: &str,
    detail: impl std::fmt::Display,
) -> String {
    let thread_id = thread_id.unwrap_or("<unknown>");
    format!(
        "Codex lifecycle {operation} failed for issue {} and canonical thread {thread_id}: {detail}",
        issue.identifier
    )
}

fn codex_turn_id_from_start_response(value: &serde_json::Value) -> Option<String> {
    value
        .get("result")
        .and_then(|result| {
            result
                .get("turnId")
                .or_else(|| result.get("turn_id"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    result
                        .get("turn")
                        .and_then(|turn| {
                            turn.get("id")
                                .or_else(|| turn.get("turnId"))
                                .or_else(|| turn.get("turn_id"))
                        })
                        .and_then(serde_json::Value::as_str)
                })
        })
        .filter(|turn_id| !turn_id.trim().is_empty())
        .map(ToOwned::to_owned)
}

async fn read_until_codex_turn_id(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    worker_id: &str,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    read_state: &mut CodexReadState,
) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + CODEX_RESPONSE_TIMEOUT;
    loop {
        let line = timeout_at(deadline, reader.next_line())
            .await
            .map_err(|_| "timed out waiting for Codex turn id after turn/start".to_string())?
            .map_err(|source| format!("failed reading Codex stdout: {source}"))?
            .ok_or("Codex stdout closed before reporting turn id")?;
        let value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|source| format!("invalid Codex JSON: {source}"))?;
        let Some(event) = emit_codex_notification(updates_tx, worker_id, issue, run, value) else {
            continue;
        };
        if read_state.pending_terminal.is_none()
            && let Some(outcome) = codex_terminal_outcome(&event)
        {
            read_state.pending_terminal = Some(outcome);
        }
        if let Some(turn_id) = event.turn_id.filter(|turn_id| !turn_id.trim().is_empty()) {
            return Ok(turn_id);
        }
    }
}

async fn drain_codex_stderr(
    stderr: ChildStderr,
    worker_id: String,
    tail: Arc<Mutex<VecDeque<String>>>,
) {
    let mut lines = BufReader::new(stderr).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                push_codex_stderr_tail(&tail, line.clone());
                tracing::debug!(%worker_id, stderr = %line, "Codex app-server stderr");
            }
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(%worker_id, %error, "failed to drain Codex app-server stderr");
                break;
            }
        }
    }
}

fn push_codex_stderr_tail(tail: &Arc<Mutex<VecDeque<String>>>, line: String) {
    if let Ok(mut tail) = tail.lock() {
        if tail.len() == CODEX_STDERR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

fn with_codex_stderr(error: String, tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    let Ok(tail) = tail.lock() else {
        return error;
    };
    if tail.is_empty() {
        return error;
    }
    format!(
        "{error}; Codex emitted {} recent stderr line(s); raw stderr is kept in debug logs only",
        tail.len()
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CodexTerminalOutcome {
    event_kind: NormalizedCodexEventKind,
    outcome: WorkerOutcomeKind,
    status: RunStatus,
}

#[derive(Debug, Default)]
struct CodexReadState {
    pending_terminal: Option<CodexTerminalOutcome>,
    pending_turn_id: Option<String>,
}

async fn read_response_line(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: u64,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    worker_id: &str,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    read_state: &mut CodexReadState,
) -> Result<serde_json::Value, String> {
    let deadline = tokio::time::Instant::now() + CODEX_RESPONSE_TIMEOUT;
    loop {
        let line = timeout_at(deadline, reader.next_line())
            .await
            .map_err(|_| format!("timed out waiting for Codex response id {request_id}"))?
            .map_err(|source| format!("failed reading Codex stdout: {source}"))?
            .ok_or_else(|| format!("Codex stdout closed before response id {request_id}"))?;
        let value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|source| format!("invalid Codex JSON: {source}"))?;
        if codex_response_id_matches(&value, request_id) {
            reject_codex_json_rpc_error(request_id, &value)?;
            return Ok(value);
        }
        if let Some(event) = emit_codex_notification(updates_tx, worker_id, issue, run, value) {
            capture_codex_turn_id(&event, &mut read_state.pending_turn_id);
            if read_state.pending_terminal.is_none()
                && let Some(outcome) = codex_terminal_outcome(&event)
            {
                read_state.pending_terminal = Some(outcome);
            }
        }
    }
}

fn codex_response_id_matches(value: &serde_json::Value, request_id: u64) -> bool {
    let Some(id) = value.get("id") else {
        return false;
    };
    id.as_u64() == Some(request_id) || id.as_str().is_some_and(|id| id == request_id.to_string())
}

fn codex_response_id(value: &serde_json::Value) -> Option<u64> {
    let id = value.get("id")?;
    id.as_u64()
        .or_else(|| id.as_str().and_then(|id| id.parse().ok()))
}

fn reject_codex_json_rpc_error(request_id: u64, value: &serde_json::Value) -> Result<(), String> {
    let Some(error) = value.get("error").filter(|error| !error.is_null()) else {
        return Ok(());
    };
    let detail = error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| error.to_string());
    Err(format!(
        "Codex response id {request_id} returned JSON-RPC error: {detail}"
    ))
}

fn capture_codex_turn_id(event: &NormalizedCodexEvent, pending_turn_id: &mut Option<String>) {
    if pending_turn_id.is_some() {
        return;
    }
    if let Some(turn_id) = event
        .turn_id
        .as_deref()
        .filter(|turn_id| !turn_id.trim().is_empty())
    {
        *pending_turn_id = Some(turn_id.to_string());
    }
}

fn complete_codex_interrupt_response(
    responses: &CodexInterruptResponseRegistry,
    value: &serde_json::Value,
) -> bool {
    let Some(response_id) = codex_response_id(value) else {
        return false;
    };
    let sender = responses
        .lock()
        .ok()
        .and_then(|mut responses| responses.remove(&response_id));
    if let Some(sender) = sender {
        let _ = sender.send(reject_codex_json_rpc_error(response_id, value));
    }
    true
}

fn codex_interrupt_response_pending(responses: &CodexInterruptResponseRegistry) -> bool {
    responses
        .lock()
        .is_ok_and(|responses| !responses.is_empty())
}

async fn read_until_codex_terminal(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    worker_id: &str,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    read_state: &mut CodexReadState,
    interrupt_responses: &CodexInterruptResponseRegistry,
) -> Result<CodexTerminalOutcome, String> {
    if !codex_interrupt_response_pending(interrupt_responses)
        && let Some(outcome) = read_state.pending_terminal.take()
    {
        return Ok(outcome);
    }

    loop {
        let line = timeout(CODEX_TERMINAL_TIMEOUT, reader.next_line())
            .await
            .map_err(|_| "timed out waiting for Codex terminal notification".to_string())?
            .map_err(|source| format!("failed reading Codex stdout: {source}"))?
            .ok_or("Codex stdout closed before terminal notification")?;
        let value: serde_json::Value = serde_json::from_str(&line)
            .map_err(|source| format!("invalid Codex JSON: {source}"))?;
        if complete_codex_interrupt_response(interrupt_responses, &value) {
            if !codex_interrupt_response_pending(interrupt_responses)
                && let Some(outcome) = read_state.pending_terminal.take()
            {
                return Ok(outcome);
            }
            continue;
        }
        if let Some(event) = emit_codex_notification(updates_tx, worker_id, issue, run, value)
            && let Some(outcome) = codex_terminal_outcome(&event)
        {
            if codex_interrupt_response_pending(interrupt_responses) {
                read_state.pending_terminal.get_or_insert(outcome);
                continue;
            }
            return Ok(outcome);
        }
    }
}

fn emit_codex_notification(
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    worker_id: &str,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    value: serde_json::Value,
) -> Option<NormalizedCodexEvent> {
    let event = normalize_server_notification(value)?;
    let Ok(worker_id) = crate::opensymphony_domain::WorkerId::new(worker_id.to_string()) else {
        return Some(event);
    };
    let observed_at = now_timestamp();
    let _ = updates_tx.send(WorkerUpdate::RuntimeEvent {
        worker_id: worker_id.clone(),
        observed_at,
        event_id: event.item_id.clone().or_else(|| event.turn_id.clone()),
        event_kind: Some(format!("codex.{}", event.method)),
        summary: Some(codex_event_summary(&event)),
        payload: Some(event.raw.clone()),
    });
    if let Some(usage) = event.token_usage {
        let _ = updates_tx.send(WorkerUpdate::TokenUsageUpdate {
            worker_id: worker_id.clone(),
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_tokens,
            total_tokens: usage.total_tokens,
        });
    }
    if let Some(approval) = codex_approval_request_from_event(
        run.worker_id.as_str(),
        issue.id.as_str(),
        issue.identifier.as_str(),
        timestamp_to_datetime(observed_at),
        &event,
    ) {
        let payload = match serde_json::to_value(&approval) {
            Ok(payload) => Some(payload),
            Err(error) => {
                tracing::warn!(
                    approval_id = %approval.approval_id,
                    %error,
                    "failed to serialize Codex approval request payload"
                );
                None
            }
        };
        let _ = updates_tx.send(WorkerUpdate::RuntimeEvent {
            worker_id,
            observed_at,
            event_id: Some(format!("approval:{}", approval.approval_id)),
            event_kind: Some("approval.requested".into()),
            summary: Some(format!("Approval requested: {}", approval.title)),
            payload,
        });
    }
    Some(event)
}

fn codex_terminal_outcome(event: &NormalizedCodexEvent) -> Option<CodexTerminalOutcome> {
    let (outcome, status) = match event.kind {
        NormalizedCodexEventKind::TurnCompleted => {
            if turn_status(event).as_deref() == Some("interrupted") {
                (WorkerOutcomeKind::Cancelled, RunStatus::Cancelled)
            } else {
                (WorkerOutcomeKind::Succeeded, RunStatus::Succeeded)
            }
        }
        NormalizedCodexEventKind::TurnCancelled => {
            (WorkerOutcomeKind::Cancelled, RunStatus::Cancelled)
        }
        NormalizedCodexEventKind::Error => (WorkerOutcomeKind::Failed, RunStatus::Failed),
        NormalizedCodexEventKind::ThreadStatusChanged => {
            let status = event
                .raw
                .get("params")
                .and_then(|params| params.get("status"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_ascii_lowercase);
            match status.as_deref() {
                Some("completed" | "succeeded" | "success") => {
                    (WorkerOutcomeKind::Succeeded, RunStatus::Succeeded)
                }
                Some("failed" | "error") => (WorkerOutcomeKind::Failed, RunStatus::Failed),
                Some("cancelled" | "canceled" | "interrupted") => {
                    (WorkerOutcomeKind::Cancelled, RunStatus::Cancelled)
                }
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(CodexTerminalOutcome {
        event_kind: event.kind,
        outcome,
        status,
    })
}

async fn finish_codex_workspace_run(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    status: RunStatus,
) -> Result<(), WorkspaceError> {
    run_manifest.status = status;
    run_manifest.status_detail = Some(format!("Codex app-server route ended with {status}"));
    workspace_manager
        .finish_run(workspace, run_manifest, status)
        .await
}

async fn record_codex_finish_failure(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    attempted_status: RunStatus,
    error: WorkspaceError,
) -> String {
    let detail = format!("failed to finish Codex workspace run as {attempted_status}: {error}");
    run_manifest.status = RunStatus::Failed;
    run_manifest.status_detail = Some(format!(
        "Codex app-server workspace finalization failed after {attempted_status}"
    ));
    if let Err(failed_error) = workspace_manager
        .finish_run(workspace, run_manifest, RunStatus::Failed)
        .await
    {
        return format!("{detail}; additionally failed to persist failed status: {failed_error}");
    }
    detail
}

async fn write_codex_conversation_manifest(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    issue: &NormalizedIssue,
    thread_id: &str,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> Result<IssueConversationManifest, String> {
    let now = chrono::Utc::now();
    let conversation_id = ConversationId::new(thread_id.to_string())
        .map_err(|error| format!("invalid Codex thread id for conversation manifest: {error}"))?;
    let manifest = IssueConversationManifest {
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        conversation_id,
        reuse_policy: "per_issue".to_string(),
        server_base_url: None,
        transport_target: Some(CODEX_APP_SERVER_KIND.to_string()),
        http_auth_mode: None,
        websocket_auth_mode: None,
        websocket_query_param_name: None,
        persistence_dir: workspace.metadata_dir(),
        created_at: now,
        updated_at: now,
        last_attached_at: now,
        launch_profile: None,
        llm_config_fingerprint: None,
        fresh_conversation: true,
        workflow_prompt_seeded: false,
        reset_reason: None,
        runtime_contract_version: Some(CODEX_APP_SERVER_CONTRACT.to_string()),
        codex_archive_state: Some("active".to_string()),
        last_turn_id: None,
        last_prompt_kind: None,
        last_prompt_at: None,
        last_prompt_path: None,
        last_execution_status: None,
        last_event_id: None,
        last_event_kind: Some("thread/start".into()),
        last_event_at: Some(now),
        last_event_summary: Some(route.summary()),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        last_token_accumulation_at: None,
    };
    workspace_manager
        .write_json_artifact(
            workspace,
            &workspace.conversation_manifest_path(),
            &manifest,
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(manifest)
}

async fn load_codex_conversation_manifest(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    issue: &NormalizedIssue,
) -> Result<Option<IssueConversationManifest>, String> {
    let path = workspace.conversation_manifest_path();
    let Some(raw) = workspace_manager
        .read_text_artifact(workspace, &path)
        .await
        .map_err(|error| {
            format!(
                "failed to read conversation manifest {}: {error}",
                path.display()
            )
        })?
    else {
        return Ok(None);
    };
    let manifest: IssueConversationManifest = serde_json::from_str(&raw)
        .map_err(|error| format!("invalid conversation manifest {}: {error}", path.display()))?;
    let thread_id = manifest.conversation_id.as_str();
    if !conversation_manifest_is_codex(&manifest) {
        return Err(codex_lifecycle_error(
            issue,
            Some(thread_id),
            "manifest validation",
            "manifest belongs to a different harness",
        ));
    }
    if manifest.issue_id != issue.id
        || manifest.identifier != issue.identifier
        || manifest.reuse_policy != "per_issue"
        || manifest.persistence_dir != workspace.metadata_dir()
        || thread_id.trim().is_empty()
    {
        return Err(codex_lifecycle_error(
            issue,
            Some(thread_id),
            "manifest validation",
            format!(
                "manifest {} is incompatible with the current issue workspace",
                path.display()
            ),
        ));
    }
    Ok(Some(manifest))
}

async fn update_codex_conversation_manifest(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut IssueConversationManifest,
    prompt_kind: IssueSessionPromptKind,
    fresh_conversation: bool,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    manifest.workflow_prompt_seeded = true;
    manifest.fresh_conversation = fresh_conversation;
    manifest.updated_at = now;
    manifest.last_attached_at = now;
    manifest.last_prompt_kind = Some(prompt_kind);
    manifest.last_prompt_at = Some(now);
    manifest.last_event_kind = Some("turn/start".to_string());
    manifest.last_event_at = Some(now);
    manifest.last_event_summary = Some(route.summary());
    workspace_manager
        .write_json_artifact(workspace, &workspace.conversation_manifest_path(), manifest)
        .await
        .map_err(|error| error.to_string())
}

async fn persist_codex_turn_id(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut IssueConversationManifest,
    turn_id: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    manifest.last_turn_id = Some(turn_id.to_owned());
    manifest.updated_at = now;
    manifest.last_attached_at = now;
    workspace_manager
        .write_json_artifact(workspace, &workspace.conversation_manifest_path(), manifest)
        .await
        .map_err(|error| error.to_string())
}

async fn persist_codex_run_started(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    run_manifest: &mut RunManifest,
    conversation_id: &str,
    prompt_kind: IssueSessionPromptKind,
) -> Result<(), String> {
    run_manifest.status = RunStatus::Running;
    run_manifest.status_detail = Some(format!(
        "{} prompt sent to Codex conversation {conversation_id}",
        prompt_kind.as_str()
    ));
    run_manifest.updated_at = chrono::Utc::now();
    workspace_manager
        .write_run_manifest(workspace, run_manifest)
        .await
        .map_err(|error| error.to_string())
}

fn codex_conversation_metadata(
    conversation_id: String,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: ConversationId::new(conversation_id)
            .expect("Codex conversation id should not be empty"),
        server_base_url: None,
        transport_target: Some(route.harness_kind.clone()),
        http_auth_mode: None,
        websocket_auth_mode: None,
        websocket_query_param_name: None,
        fresh_conversation: true,
        runtime_contract_version: Some("codex-app-server-json-rpc-v2".into()),
        stream_state: RuntimeStreamState::Closed,
        last_event_id: None,
        last_event_kind: None,
        last_event_at: None,
        last_event_summary: Some(route.summary()),
        recent_activity: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
        runtime_seconds: 0,
        next_activity_sequence: 0,
    }
}

fn transport_port_override(url: &Url) -> Result<u16, RunCommandError> {
    url.port_or_known_default()
        .ok_or_else(|| RunCommandError::MissingTransportPort {
            value: url.as_str().to_string(),
        })
}

fn report_launch_failure(
    launch_tx: &mut Option<oneshot::Sender<LaunchReport>>,
    detail: impl Into<String>,
) {
    if let Some(sender) = launch_tx.take() {
        let _ = sender.send(LaunchReport::Failed(detail.into()));
    }
}

fn pending_launch_failure_detail(result: &Result<IssueSessionResult, IssueSessionError>) -> String {
    match result {
        Ok(result) => {
            let detail = result
                .worker_outcome
                .error
                .clone()
                .or_else(|| result.worker_outcome.summary.clone())
                .unwrap_or_else(|| {
                    "worker finished before reporting a conversation launch".to_string()
                });
            format!("worker finished before reporting a conversation launch: {detail}")
        }
        Err(error) => format!("worker failed before reporting a conversation launch: {error}"),
    }
}

impl WorkerBackend for RuntimeWorkerBackend {
    type Error = CliWorkerError;

    async fn start_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error> {
        let pending = self.spawn_worker_task(request, false);
        let worker_id = pending.worker_id.clone();
        let route = pending.route.clone();
        let launch_timeout = self.launch_timeout_for_route(&route);
        self.resolve_launch_result(
            &worker_id,
            &route,
            launch_timeout,
            timeout(launch_timeout, pending.launch_rx).await,
        )
        .await
    }

    async fn start_workers(
        &mut self,
        requests: Vec<WorkerStartRequest>,
    ) -> Vec<Result<WorkerLaunch, Self::Error>> {
        let pending = requests
            .into_iter()
            .map(|request| self.spawn_worker_task(request, false))
            .collect::<Vec<_>>();
        let ordered_launches = pending
            .iter()
            .map(|launch| (launch.worker_id.clone(), launch.route.clone()))
            .collect::<Vec<_>>();

        let mut waiters = Vec::with_capacity(pending.len());
        for launch in pending {
            let timeout_duration = self.launch_timeout_for_route(&launch.route);
            let worker_id = launch.worker_id;
            let rx = launch.launch_rx;
            let worker_id_for_task = worker_id.clone();
            let handle =
                tokio::spawn(
                    async move { (worker_id_for_task, timeout(timeout_duration, rx).await) },
                );
            waiters.push((worker_id, handle));
        }

        let mut completed = HashMap::new();
        for (worker_id, handle) in waiters {
            match handle.await {
                Ok((worker_id, outcome)) => {
                    completed.insert(worker_id, outcome);
                }
                Err(join_error) => {
                    tracing::error!(error = %join_error, "worker launch waiter task failed");
                    completed.insert(
                        worker_id,
                        Ok(Ok(LaunchReport::Failed(format!(
                            "worker launch waiter task failed: {join_error}"
                        )))),
                    );
                }
            }
        }

        let mut launches = Vec::with_capacity(ordered_launches.len());
        for (worker_id, route) in ordered_launches {
            let outcome = completed
                .remove(&worker_id)
                .unwrap_or(Ok(Ok(LaunchReport::Failed(
                    "worker launch waiter finished without a result".to_string(),
                ))));
            launches.push(
                self.resolve_launch_result(
                    &worker_id,
                    &route,
                    self.launch_timeout_for_route(&route),
                    outcome,
                )
                .await,
            );
        }
        launches
    }

    async fn recover_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error> {
        let pending = self.spawn_worker_task(request, true);
        let worker_id = pending.worker_id.clone();
        let route = pending.route.clone();
        let launch_timeout = self.launch_timeout_for_route(&route);
        self.resolve_launch_result(
            &worker_id,
            &route,
            launch_timeout,
            timeout(launch_timeout, pending.launch_rx).await,
        )
        .await
    }

    async fn poll_updates(&mut self) -> Result<Vec<WorkerUpdate>, Self::Error> {
        let mut updates = Vec::new();
        while let Ok(update) = self.updates_rx.try_recv() {
            if let WorkerUpdate::Finished { worker_id, .. } = &update
                && let Some(task) = self.tasks.remove(worker_id.as_str())
            {
                let _ = task.handle.await;
            }
            updates.push(update);
        }

        let finished = self
            .tasks
            .iter()
            .filter_map(|(worker_id, task)| task.handle.is_finished().then_some(worker_id.clone()))
            .collect::<Vec<_>>();
        for worker_id in finished {
            let Some(task) = self.tasks.remove(worker_id.as_str()) else {
                continue;
            };
            if let Err(error) = task.handle.await {
                updates.push(WorkerUpdate::Finished {
                    worker_id: crate::opensymphony_domain::WorkerId::new(worker_id)
                        .expect("worker id should remain valid"),
                    outcome: WorkerOutcomeRecord::from_run(
                        &task.run,
                        WorkerOutcomeKind::Failed,
                        now_timestamp(),
                        Some("worker task terminated unexpectedly".to_string()),
                        Some(error.to_string()),
                    ),
                });
            }
        }

        Ok(updates)
    }

    async fn abort_worker(
        &mut self,
        worker_id: &crate::opensymphony_domain::WorkerId,
        _reason: WorkerAbortReason,
    ) -> Result<(), Self::Error> {
        self.abort_tracked_task(worker_id.as_str());
        Ok(())
    }

    async fn interrupt_worker(
        &mut self,
        command: crate::opensymphony_domain::HarnessInterruptCommand,
    ) -> Result<WorkerInterruptAcknowledgement, Self::Error> {
        if command.harness_kind == CODEX_APP_SERVER_KIND {
            return send_codex_stdio_interrupt(&self.codex_interrupts, &command).await;
        }
        if command.harness_kind != OPENHANDS_AGENT_SERVER_KIND {
            return Err(CliWorkerError::InterruptFailed(format!(
                "harness `{}` does not expose a scheduler-side interrupt channel",
                command.harness_kind
            )));
        }

        let mut runner = IssueSessionRunner::with_environment(
            self.client.clone(),
            self.runner_config.clone(),
            OverlayEnvironment {
                overrides: self.worker_env.clone(),
            },
        );
        if let Some(source) = self.workpad_comment_source.clone() {
            runner = runner.with_workpad_comment_source(source);
        }
        let acknowledgement = runner
            .interrupt(&command)
            .await
            .map_err(|error| CliWorkerError::InterruptFailed(openhands_error_detail(&error)))?;
        let accepted = acknowledgement
            .execution_status
            .as_deref()
            .is_some_and(openhands_execution_stopped);
        Ok(WorkerInterruptAcknowledgement {
            accepted,
            detail: acknowledgement
                .diagnostic
                .or_else(|| {
                    acknowledgement
                        .execution_status
                        .map(|status| format!("OpenHands interrupt acknowledged with `{status}`"))
                })
                .or_else(|| Some("OpenHands interrupt acknowledged".to_string())),
            timed_out: acknowledgement.timed_out,
        })
    }
}

fn openhands_execution_stopped(status: &str) -> bool {
    matches!(status, "paused" | "idle" | "finished" | "error" | "stuck")
}

fn openhands_error_detail(error: &OpenHandsError) -> String {
    match error {
        OpenHandsError::InvalidConfiguration { detail } => {
            format!("openhands.invalid_configuration: {detail}")
        }
        OpenHandsError::Transport { operation, detail } => {
            format!("openhands.transport.{operation}: {detail}")
        }
        OpenHandsError::HttpStatus {
            operation,
            status_code,
            body,
        } => format!(
            "openhands.http_status.{operation}.{status_code}: {}",
            truncated_diagnostic_body(body)
        ),
        OpenHandsError::Protocol { operation, detail } => {
            format!("openhands.protocol.{operation}: {detail}")
        }
        OpenHandsError::WebSocketTransport { operation, detail } => {
            format!("openhands.websocket.{operation}: {detail}")
        }
        OpenHandsError::MalformedWebSocketEvent { detail, snippet } => {
            format!("openhands.websocket.malformed_event: {detail}; payload prefix: {snippet}")
        }
        OpenHandsError::ReadinessTimeout(timeout) => {
            format!("openhands.websocket.readiness_timeout: {timeout:?}")
        }
        OpenHandsError::ProbeActivityTimeout(timeout) => {
            format!("openhands.probe.activity_timeout: {timeout:?}")
        }
        OpenHandsError::ProbeRunUnhealthy(detail) => {
            format!("openhands.probe.unhealthy: {detail}")
        }
        OpenHandsError::WebSocketClosed => {
            "openhands.websocket.closed_before_readiness".to_string()
        }
        OpenHandsError::ReconnectExhausted {
            attempts,
            last_error,
        } => format!(
            "openhands.websocket.reconnect_exhausted: attempts={attempts}; last_error={last_error}"
        ),
    }
}

fn truncated_diagnostic_body(body: &str) -> String {
    const MAX_CHARS: usize = 240;

    let mut chars = body.chars();
    let prefix: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{prefix}...")
    } else {
        prefix
    }
}

fn normalized_state_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn issue_state_category(
    name: &str,
    active_states: &HashSet<String>,
    terminal_states: &HashSet<String>,
) -> IssueStateCategory {
    let normalized = normalized_state_name(name);
    if terminal_states.contains(&normalized) {
        IssueStateCategory::Terminal
    } else if active_states.contains(&normalized) {
        IssueStateCategory::Active
    } else {
        IssueStateCategory::NonActive
    }
}

fn normalized_issue_from_manifest(
    manifest: &crate::opensymphony_workspace::IssueManifest,
    active_states: &HashSet<String>,
    terminal_states: &HashSet<String>,
) -> Result<NormalizedIssue, CliWorkspaceError> {
    Ok(NormalizedIssue {
        id: IssueId::new(manifest.issue_id.clone())?,
        identifier: IssueIdentifier::new(manifest.identifier.clone())?,
        title: manifest.title.clone(),
        description: None,
        priority: None,
        state: IssueState {
            id: None,
            name: manifest.current_state.clone(),
            category: issue_state_category(&manifest.current_state, active_states, terminal_states),
        },
        branch_name: None,
        pr_url: None,
        url: None,
        labels: Vec::new(),
        project_id: None,
        project_slug: None,
        project_name: None,
        parent_id: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: Some(datetime_to_timestamp_ms(manifest.created_at)),
        updated_at: Some(datetime_to_timestamp_ms(manifest.updated_at)),
    })
}

fn issue_descriptor(issue: &NormalizedIssue) -> IssueDescriptor {
    IssueDescriptor {
        issue_id: issue.id.to_string(),
        identifier: issue.identifier.to_string(),
        title: issue.title.clone(),
        current_state: issue.state.name.clone(),
        last_seen_tracker_refresh_at: issue.updated_at.map(timestamp_to_datetime),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        fs,
        future::pending,
        path::{Path, PathBuf},
    };

    use crate::opensymphony_domain::{
        ConversationId, HarnessInterruptCommand, HarnessInterruptExpectedNextState,
        HarnessInterruptReason, IssueId, IssueIdentifier, IssueState, IssueStateCategory,
        RetryAttempt, RunAttempt, TrackerIssueStateKind, WorkerId, WorkspaceKey,
    };
    use crate::opensymphony_workflow::WorkflowDefinition;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

    fn empty_codex_schema_cache() -> CodexSchemaValidatorCache {
        Arc::new(AsyncMutex::new(HashMap::new()))
    }

    fn empty_codex_interrupt_registry() -> CodexInterruptRegistry {
        Arc::new(Mutex::new(HashMap::new()))
    }

    #[cfg(unix)]
    #[allow(clippy::too_many_arguments)]
    async fn run_fake_codex_attempt(
        workflow: &ResolvedWorkflow,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        issue: &NormalizedIssue,
        route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
        codex_bin: &Path,
        run_id: &str,
        attempt: u32,
        retry_attempt: Option<u32>,
    ) -> (WorkerOutcomeRecord, RunManifest, LaunchReport) {
        let mut run_manifest = workspace_manager
            .start_run(workspace, &RunDescriptor::new(run_id, attempt))
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new(format!("worker-{run_id}")).expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            workspace.workspace_path().to_path_buf(),
            TimestampMs::new(u64::from(attempt)),
            retry_attempt.map(|value| RetryAttempt::new(value).expect("retry attempt is valid")),
            8,
        );
        let (updates_tx, _updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let outcome = run_codex_stdio_issue(
            route,
            workspace_manager,
            workspace,
            &mut run_manifest,
            issue,
            &run,
            workflow,
            codex_bin.to_str().expect("fake codex path should be utf-8"),
            &empty_codex_schema_cache(),
            &empty_codex_interrupt_registry(),
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;
        let launch = launch_rx.await.expect("worker should report launch result");
        (outcome, run_manifest, launch)
    }

    fn sample_conversation_manifest(conversation_id: &str) -> IssueConversationManifest {
        let now = chrono::Utc::now();
        IssueConversationManifest {
            issue_id: IssueId::new("issue-contract").expect("issue id should be valid"),
            identifier: IssueIdentifier::new("COE-479").expect("identifier should be valid"),
            conversation_id: ConversationId::new(conversation_id.to_string())
                .expect("conversation id should be valid"),
            reuse_policy: "per_issue".to_string(),
            server_base_url: None,
            transport_target: None,
            http_auth_mode: None,
            websocket_auth_mode: None,
            websocket_query_param_name: None,
            persistence_dir: PathBuf::from(".opensymphony"),
            created_at: now,
            updated_at: now,
            last_attached_at: now,
            launch_profile: None,
            llm_config_fingerprint: None,
            fresh_conversation: true,
            workflow_prompt_seeded: true,
            reset_reason: None,
            runtime_contract_version: None,
            codex_archive_state: None,
            last_turn_id: None,
            last_prompt_kind: None,
            last_prompt_at: None,
            last_prompt_path: None,
            last_execution_status: None,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            last_token_accumulation_at: None,
        }
    }

    #[test]
    fn codex_interrupted_turn_completion_is_cancelled_terminal_outcome() {
        let event = normalize_server_notification(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "Interrupted"
                }
            }
        }))
        .expect("notification should normalize");
        assert_eq!(event.turn_id.as_deref(), Some("turn-1"));

        let outcome = codex_terminal_outcome(&event).expect("interrupted turn is terminal");
        assert_eq!(outcome.outcome, WorkerOutcomeKind::Cancelled);
        assert_eq!(outcome.status, RunStatus::Cancelled);
    }

    #[test]
    fn codex_interrupted_thread_status_is_cancelled_terminal_outcome() {
        let event = normalize_server_notification(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "thread/status/changed",
            "params": {
                "threadId": "thread-1",
                "status": "Interrupted"
            }
        }))
        .expect("notification should normalize");

        let outcome = codex_terminal_outcome(&event).expect("interrupted thread is terminal");
        assert_eq!(outcome.outcome, WorkerOutcomeKind::Cancelled);
        assert_eq!(outcome.status, RunStatus::Cancelled);
    }

    #[test]
    fn codex_json_rpc_error_response_is_launch_failure() {
        let error = reject_codex_json_rpc_error(
            4,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 4,
                "error": {
                    "code": -32000,
                    "message": "not logged in"
                }
            }),
        )
        .expect_err("JSON-RPC error envelopes must fail the worker launch path");

        assert!(error.contains("response id 4"));
        assert!(error.contains("not logged in"));
    }

    #[test]
    fn codex_json_rpc_null_error_is_not_launch_failure() {
        reject_codex_json_rpc_error(
            4,
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 4,
                "result": {},
                "error": null
            }),
        )
        .expect("JSON-RPC error:null is equivalent to an absent error field");
    }

    #[test]
    fn codex_response_id_matches_numbers_and_equivalent_strings() {
        assert!(codex_response_id_matches(
            &serde_json::json!({ "id": 7 }),
            7
        ));
        assert!(codex_response_id_matches(
            &serde_json::json!({ "id": "7" }),
            7
        ));
        assert!(!codex_response_id_matches(
            &serde_json::json!({ "id": "07" }),
            7
        ));
        assert!(!codex_response_id_matches(
            &serde_json::json!({ "id": "turn-7" }),
            7
        ));
    }

    #[test]
    fn codex_schema_stderr_preview_is_bounded_and_sanitized() {
        let stderr = format!(
            "schema failed\u{0000}{}",
            "x".repeat(CODEX_SCHEMA_STDERR_PREVIEW_CHARS + 20)
        );

        let preview = codex_schema_stderr_preview(stderr.as_bytes())
            .expect("non-empty stderr should produce preview");

        assert!(preview.contains("schema failed"));
        assert!(preview.contains("..."));
        assert!(!preview.contains('\u{0000}'));
        assert!(preview.len() < CODEX_SCHEMA_STDERR_PREVIEW_CHARS + 80);
    }

    #[test]
    fn codex_thread_response_requires_real_thread_id() {
        let thread_id = codex_thread_id_from_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "threadId": "thread-7"
            }
        }))
        .expect("threadId should be accepted");
        assert_eq!(thread_id, "thread-7");

        let missing = codex_thread_id_from_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {}
        }))
        .expect_err("missing thread id should fail launch");
        assert!(missing.contains("missing a non-empty thread id"));

        let empty = codex_thread_id_from_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "thread_id": "  "
            }
        }))
        .expect_err("empty thread id should fail launch");
        assert!(empty.contains("missing a non-empty thread id"));
    }

    #[test]
    fn codex_resume_response_reconciles_the_persisted_terminal_turn() {
        let completed = codex_terminal_outcome_from_resume_response(
            &serde_json::json!({
                "result": {
                    "thread": {
                        "id": "thread-7",
                        "turns": [{"id": "turn-7", "status": "completed"}]
                    }
                }
            }),
            "turn-7",
        )
        .expect("completed persisted turn should be terminal");
        assert_eq!(completed.outcome, WorkerOutcomeKind::Succeeded);
        assert_eq!(completed.status, RunStatus::Succeeded);

        let cancelled = codex_terminal_outcome_from_resume_response(
            &serde_json::json!({
                "result": {
                    "turns": [{"turnId": "turn-8", "status": "interrupted"}]
                }
            }),
            "turn-8",
        )
        .expect("interrupted persisted turn should be terminal");
        assert_eq!(cancelled.outcome, WorkerOutcomeKind::Cancelled);
        assert_eq!(cancelled.status, RunStatus::Cancelled);

        assert!(
            codex_terminal_outcome_from_resume_response(
                &serde_json::json!({
                    "result": {
                        "thread": {
                            "turns": [{"id": "other-turn", "status": "completed"}]
                        }
                    }
                }),
                "turn-7",
            )
            .is_none()
        );
    }

    #[test]
    fn codex_manifest_detection_accepts_runtime_contract_only() {
        let manifest = IssueConversationManifest {
            transport_target: None,
            runtime_contract_version: Some(CODEX_APP_SERVER_CONTRACT.to_string()),
            ..sample_conversation_manifest("thread-contract")
        };

        assert!(conversation_manifest_is_codex(&manifest));
    }

    #[test]
    fn recovered_harness_kind_defaults_to_openhands_without_transport_target() {
        let manifest = sample_conversation_manifest("legacy-openhands");

        assert_eq!(
            recovered_harness_kind_from_manifest(&manifest),
            OPENHANDS_AGENT_SERVER_KIND
        );
    }

    #[test]
    fn recovered_harness_kind_maps_legacy_openhands_transport_targets() {
        for transport_target in ["loopback", "remote"] {
            let manifest = IssueConversationManifest {
                transport_target: Some(transport_target.to_owned()),
                ..sample_conversation_manifest("legacy-openhands-transport")
            };

            assert_eq!(
                recovered_harness_kind_from_manifest(&manifest),
                OPENHANDS_AGENT_SERVER_KIND
            );
        }
    }

    #[test]
    fn recovered_harness_kind_preserves_codex_runtime_contract() {
        let manifest = IssueConversationManifest {
            transport_target: Some("remote".to_owned()),
            runtime_contract_version: Some(CODEX_APP_SERVER_CONTRACT.to_owned()),
            ..sample_conversation_manifest("codex-legacy-transport")
        };

        assert_eq!(
            recovered_harness_kind_from_manifest(&manifest),
            CODEX_APP_SERVER_KIND
        );
    }

    #[test]
    fn openhands_http_status_diagnostic_truncates_body() {
        let error = OpenHandsError::HttpStatus {
            operation: "interrupt",
            status_code: 500,
            body: "x".repeat(300),
        };
        let detail = openhands_error_detail(&error);

        assert!(detail.starts_with("openhands.http_status.interrupt.500: "));
        assert!(detail.ends_with("..."));
        assert!(detail.len() < 300);
    }

    #[test]
    fn codex_stderr_tail_is_counted_but_not_persisted_in_worker_errors() {
        let tail = Arc::new(Mutex::new(VecDeque::new()));
        for line in 0..25 {
            push_codex_stderr_tail(&tail, format!("stderr-line-{line}"));
        }

        let error = with_codex_stderr("Codex stdout closed".into(), &tail);

        assert!(error.contains("20 recent stderr line(s)"));
        assert!(error.contains("debug logs only"));
        assert!(!error.contains("stderr-line-4"));
        assert!(!error.contains("stderr-line-5"));
        assert!(!error.contains("stderr-line-24"));
    }

    #[test]
    fn codex_notification_emits_approval_center_runtime_event() {
        let issue = sample_issue();
        let run = RunAttempt::new(
            WorkerId::new("worker-approval").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            PathBuf::from("/tmp/opensymphony-worker-approval"),
            TimestampMs::new(1),
            None,
            8,
        );
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();

        let event = emit_codex_notification(
            &updates_tx,
            run.worker_id.as_str(),
            &issue,
            &run,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "item/permissions/requestApproval",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "approval-1",
                    "command": "rg approval crates"
                }
            }),
        )
        .expect("Codex approval notification should normalize");

        assert_eq!(event.kind, NormalizedCodexEventKind::ApprovalRequested);
        let raw_event = updates_rx
            .try_recv()
            .expect("raw Codex runtime event should be emitted");
        let approval_event = updates_rx
            .try_recv()
            .expect("approval-center runtime event should be emitted");

        assert!(matches!(
            raw_event,
            WorkerUpdate::RuntimeEvent {
                event_kind: Some(kind),
                ..
            } if kind == "codex.item/permissions/requestApproval"
        ));
        match approval_event {
            WorkerUpdate::RuntimeEvent {
                event_id,
                event_kind,
                payload,
                ..
            } => {
                assert_eq!(event_id.as_deref(), Some("approval:approval-1"));
                assert_eq!(event_kind.as_deref(), Some("approval.requested"));
                let payload = payload.expect("approval payload should serialize");
                assert_eq!(payload["approval_id"], "approval-1");
                assert_eq!(payload["run_id"], "worker-approval");
                assert_eq!(payload["status"], "pending");
            }
            other => panic!("expected runtime event, got {other:?}"),
        }
    }

    #[test]
    fn codex_notification_runtime_event_uses_content_summary() {
        let issue = sample_issue();
        let run = RunAttempt::new(
            WorkerId::new("worker-content").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            PathBuf::from("/tmp/opensymphony-worker-content"),
            TimestampMs::new(1),
            None,
            8,
        );
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();

        let event = emit_codex_notification(
            &updates_tx,
            run.worker_id.as_str(),
            &issue,
            &run,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "item/commandExecution/outputDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "cmd-1",
                    "delta": "cargo test passed token=secret"
                }
            }),
        )
        .expect("Codex command notification should normalize");

        assert_eq!(
            event.kind,
            NormalizedCodexEventKind::CommandExecutionOutputDelta
        );
        match updates_rx
            .try_recv()
            .expect("raw Codex runtime event should be emitted")
        {
            WorkerUpdate::RuntimeEvent {
                event_kind,
                summary,
                ..
            } => {
                assert_eq!(
                    event_kind.as_deref(),
                    Some("codex.item/commandExecution/outputDelta")
                );
                assert_eq!(
                    summary.as_deref(),
                    Some("Codex command output: cargo test passed token=[redacted]")
                );
            }
            other => panic!("expected runtime event, got {other:?}"),
        }
    }

    #[test]
    fn codex_token_usage_notification_emits_metadata_update() {
        let issue = sample_issue();
        let run = RunAttempt::new(
            WorkerId::new("worker-token-usage").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            PathBuf::from("/tmp/opensymphony-worker-token-usage"),
            TimestampMs::new(1),
            None,
            8,
        );
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();

        let event = emit_codex_notification(
            &updates_tx,
            run.worker_id.as_str(),
            &issue,
            &run,
            serde_json::json!({
                "jsonrpc": "2.0",
                "method": "thread/tokenUsage/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "tokenUsage": {
                        "total": {
                            "cachedInputTokens": 30,
                            "inputTokens": 100,
                            "outputTokens": 50,
                            "reasoningOutputTokens": 5,
                            "totalTokens": 150
                        }
                    }
                }
            }),
        )
        .expect("Codex token usage notification should normalize");

        assert_eq!(event.kind, NormalizedCodexEventKind::TokenUsageUpdated);
        match updates_rx
            .try_recv()
            .expect("raw Codex runtime event should be emitted")
        {
            WorkerUpdate::RuntimeEvent {
                event_kind,
                payload,
                ..
            } => {
                assert_eq!(
                    event_kind.as_deref(),
                    Some("codex.thread/tokenUsage/updated")
                );
                let payload = payload.expect("token event payload should be present");
                assert_eq!(payload["params"]["tokenUsage"]["total"]["inputTokens"], 100);
                assert_eq!(payload["params"]["tokenUsage"]["total"]["outputTokens"], 50);
                assert_eq!(
                    payload["params"]["tokenUsage"]["total"]["cachedInputTokens"],
                    30
                );
                assert_eq!(payload["params"]["tokenUsage"]["total"]["totalTokens"], 150);
            }
            other => panic!("expected runtime event, got {other:?}"),
        }
        match updates_rx
            .try_recv()
            .expect("token metadata update should be emitted")
        {
            WorkerUpdate::TokenUsageUpdate {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                total_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 100);
                assert_eq!(output_tokens, 50);
                assert_eq!(cache_read_tokens, 30);
                assert_eq!(total_tokens, 150);
            }
            other => panic!("expected token usage update, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_drives_fake_child_lifecycle() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(&ensured.handle, &RunDescriptor::new("run-fake-codex", 1))
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir.path().join("fake-codex.log");
        let fake_codex = tempdir.path().join("fake-codex");
        write_fake_codex_child(&fake_codex, &log_path);
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();

        let outcome = run_codex_stdio_issue(
            &route,
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex path should be utf-8"),
            &codex_schema_validators,
            &codex_interrupts,
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Succeeded);
        assert_eq!(run_manifest.status, RunStatus::Succeeded);
        let launch = launch_rx
            .await
            .expect("launch report should be sent before terminal completion");
        match launch {
            LaunchReport::Conversation(conversation) => {
                assert_eq!(conversation.conversation_id.as_str(), "fake-thread");
                assert_eq!(conversation.stream_state, RuntimeStreamState::Closed);
            }
            LaunchReport::Failed(error) => panic!("fake child should launch: {error}"),
        }
        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert!(log.contains(&format!(
            "PWD={}",
            ensured.handle.workspace_path().display()
        )));
        assert!(log.contains("ARGS=--dangerously-bypass-hook-trust app-server --stdio"));
        assert!(log.contains("\"method\":\"initialize\""));
        assert!(log.contains("\"method\":\"thread/start\""));
        assert!(log.contains("\"method\":\"turn/start\""));
        let manifest: IssueConversationManifest = serde_json::from_str(
            &fs::read_to_string(ensured.handle.conversation_manifest_path())
                .expect("Codex conversation manifest should exist"),
        )
        .expect("Codex conversation manifest should decode");
        assert_eq!(manifest.conversation_id.as_str(), "fake-thread");
        assert_eq!(
            manifest.transport_target.as_deref(),
            Some(CODEX_APP_SERVER_KIND)
        );
        assert_eq!(
            manifest.runtime_contract_version.as_deref(),
            Some(CODEX_APP_SERVER_CONTRACT)
        );
        assert!(manifest.workflow_prompt_seeded);
        assert_eq!(manifest.codex_archive_state.as_deref(), Some("active"));
        assert!(
            std::iter::from_fn(|| updates_rx.try_recv().ok()).any(|update| {
                matches!(
                    update,
                    WorkerUpdate::RuntimeEvent {
                        event_kind: Some(kind),
                        ..
                    } if kind == "codex.turn/completed"
                )
            }),
            "terminal Codex notification should be forwarded as a runtime event"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_reuses_one_thread_across_retries() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let log_path = tempdir.path().join("fake-codex-reuse.log");
        let fake_codex = tempdir.path().join("fake-codex-reuse");
        write_fake_codex_child(&fake_codex, &log_path);
        let mut route = codex_test_route(false);
        route.model = Some("gpt-5-codex".to_string());

        let (initial, _, _) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &route,
            &fake_codex,
            "initial",
            1,
            None,
        )
        .await;
        assert_eq!(initial.outcome, WorkerOutcomeKind::Succeeded);
        let initial_manifest_raw = fs::read_to_string(ensured.handle.conversation_manifest_path())
            .expect("initial manifest should exist");
        let mut unseeded_manifest: IssueConversationManifest =
            serde_json::from_str(&initial_manifest_raw).expect("initial manifest should decode");
        assert!(unseeded_manifest.fresh_conversation);
        let created_at = unseeded_manifest.created_at;
        unseeded_manifest.workflow_prompt_seeded = false;
        workspace_manager
            .write_json_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &unseeded_manifest,
            )
            .await
            .expect("unseeded manifest should persist");

        let (unseeded_retry, _, _) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &route,
            &fake_codex,
            "unseeded-retry",
            2,
            Some(1),
        )
        .await;
        assert_eq!(unseeded_retry.outcome, WorkerOutcomeKind::Succeeded);
        let seeded_manifest_raw = fs::read_to_string(ensured.handle.conversation_manifest_path())
            .expect("seeded manifest should exist");
        let seeded_manifest: IssueConversationManifest =
            serde_json::from_str(&seeded_manifest_raw).expect("seeded manifest should decode");
        assert_eq!(seeded_manifest.conversation_id.as_str(), "fake-thread");
        assert_eq!(seeded_manifest.created_at, created_at);
        assert!(seeded_manifest.workflow_prompt_seeded);
        assert!(
            !seeded_manifest.fresh_conversation,
            "resuming an unseeded manifest must not be recorded as a fresh conversation"
        );

        let (seeded_retry, _, _) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &route,
            &fake_codex,
            "seeded-retry",
            3,
            Some(2),
        )
        .await;
        assert_eq!(seeded_retry.outcome, WorkerOutcomeKind::Succeeded);

        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert_eq!(log.matches(r#""method":"thread/start""#).count(), 1);
        assert_eq!(log.matches(r#""method":"thread/resume""#).count(), 2);
        assert_eq!(log.matches(r#""method":"turn/start""#).count(), 3);
        assert!(log.contains(r#""model":"gpt-5-codex""#));
        assert!(log.contains("Run the scheduler."));
        assert!(log.contains("The original workflow prompt is already present"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_reuses_manifest_for_every_retry_state() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let log_path = tempdir.path().join("fake-codex-retry-states.log");
        let fake_codex = tempdir.path().join("fake-codex-retry-states");
        write_fake_codex_child(&fake_codex, &log_path);
        let route = codex_test_route(false);
        let (initial, _, _) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &route,
            &fake_codex,
            "initial",
            1,
            None,
        )
        .await;
        assert_eq!(initial.outcome, WorkerOutcomeKind::Succeeded);
        let manifest_path = ensured.handle.conversation_manifest_path();
        let initial_manifest: IssueConversationManifest = serde_json::from_str(
            &fs::read_to_string(&manifest_path).expect("initial manifest should exist"),
        )
        .expect("initial manifest should decode");
        let canonical_thread_id = initial_manifest.conversation_id.to_string();
        let created_at = initial_manifest.created_at;

        for (attempt, status) in ["failed", "stalled", "cancelled", "recovery"]
            .iter()
            .enumerate()
        {
            let mut manifest: IssueConversationManifest = serde_json::from_str(
                &fs::read_to_string(&manifest_path).expect("manifest should exist"),
            )
            .expect("manifest should decode");
            manifest.last_execution_status = Some((*status).to_string());
            workspace_manager
                .write_json_artifact(&ensured.handle, &manifest_path, &manifest)
                .await
                .expect("retry-state manifest should persist");
            let (outcome, _, _) = run_fake_codex_attempt(
                &workflow,
                &workspace_manager,
                &ensured.handle,
                &issue,
                &route,
                &fake_codex,
                &format!("retry-{status}"),
                attempt as u32 + 2,
                Some(attempt as u32 + 1),
            )
            .await;
            assert_eq!(
                outcome.outcome,
                WorkerOutcomeKind::Succeeded,
                "{status} retry"
            );
            let manifest: IssueConversationManifest = serde_json::from_str(
                &fs::read_to_string(&manifest_path).expect("manifest should remain readable"),
            )
            .expect("manifest should decode after retry");
            assert_eq!(manifest.conversation_id.to_string(), canonical_thread_id);
            assert_eq!(manifest.created_at, created_at);
        }

        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert_eq!(log.matches(r#""method":"thread/start""#).count(), 1);
        assert_eq!(log.matches(r#""method":"thread/resume""#).count(), 4);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_resume_failure_preserves_the_manifest() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let log_path = tempdir.path().join("fake-codex-resume-error.log");
        let fake_codex = tempdir.path().join("fake-codex-resume-error");
        let route = codex_test_route(false);
        write_fake_codex_child(&fake_codex, &log_path);
        let (initial, _, _) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &route,
            &fake_codex,
            "initial",
            1,
            None,
        )
        .await;
        assert_eq!(initial.outcome, WorkerOutcomeKind::Succeeded);
        let manifest_path = ensured.handle.conversation_manifest_path();
        let before = fs::read_to_string(&manifest_path).expect("manifest should exist");

        write_fake_codex_resume_error_child(&fake_codex, &log_path);
        let (outcome, _, launch) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &route,
            &fake_codex,
            "resume-failure",
            2,
            Some(1),
        )
        .await;
        assert_eq!(outcome.outcome, WorkerOutcomeKind::Failed);
        let error = outcome.error.expect("failure should include diagnostics");
        assert!(error.contains("issue COE-284"));
        assert!(error.contains("canonical thread fake-thread"));
        assert!(matches!(launch, LaunchReport::Failed(detail) if detail.contains("thread/resume")));
        assert_eq!(
            fs::read_to_string(&manifest_path).expect("manifest should remain readable"),
            before
        );
        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert!(log.contains(r#""method":"thread/resume""#));
        assert!(!log.contains(r#""method":"thread/start""#));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_manifest_write_failure_archives_without_starting_a_turn() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let log_path = tempdir.path().join("fake-codex-manifest-write.log");
        let fake_codex = tempdir.path().join("fake-codex-manifest-write");
        write_fake_codex_manifest_write_failure_child(&fake_codex, &log_path);
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-manifest-write-failure", 1),
            )
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-manifest-write-failure").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let (updates_tx, _updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let route = codex_test_route(false);
        let outcome = run_codex_stdio_issue(
            &route,
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex path should be utf-8"),
            &empty_codex_schema_cache(),
            &empty_codex_interrupt_registry(),
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;
        let launch = launch_rx
            .await
            .expect("worker should report launch failure");
        assert_eq!(outcome.outcome, WorkerOutcomeKind::Failed);
        let error = outcome.error.expect("failure should include diagnostics");
        assert!(error.contains("canonical thread fake-thread"));
        assert!(
            matches!(launch, LaunchReport::Failed(detail) if detail.contains("rollback archive accepted"))
        );
        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert!(log.contains(r#""method":"thread/start""#));
        assert!(log.contains(r#""method":"thread/archive""#));
        assert!(!log.contains(r#""method":"turn/start""#));
        assert!(ensured.handle.conversation_manifest_path().is_dir());
    }

    #[tokio::test]
    async fn runtime_workspace_cleanup_uses_manager_retention_and_hooks() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(WorkspaceManagerConfig {
                root: workspace_root,
                hooks: HookConfig {
                    before_remove: Some(HookDefinition::shell(
                        "echo before_remove >> .opensymphony/logs/before_remove.txt",
                    )),
                    ..HookConfig::default()
                },
                cleanup: CleanupConfig {
                    remove_terminal_workspaces: false,
                },
            })
            .expect("workspace manager should be constructed"),
        );
        let issue = sample_terminal_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        workspace_manager
            .write_json_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &sample_conversation_manifest("thread"),
            )
            .await
            .expect("conversation manifest should persist");
        let workspace = crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())
                .expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };
        let mut backend = RuntimeWorkspaceBackend::new(Arc::clone(&workspace_manager), &workflow);

        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("manager-owned cleanup should succeed");
        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("repeated terminal cleanup should be a no-op");

        assert!(ensured.handle.workspace_path().is_dir());
        assert!(ensured.handle.conversation_manifest_path().is_file());
        assert_eq!(
            fs::read_to_string(ensured.handle.logs_dir().join("before_remove.txt"))
                .expect("before-remove hook should run")
                .trim(),
            "before_remove"
        );

        let mut reopened_issue = issue.clone();
        reopened_issue.state = IssueState {
            id: None,
            name: "In Progress".to_string(),
            category: IssueStateCategory::Active,
        };
        backend
            .ensure_workspace(&reopened_issue, TimestampMs::new(2))
            .await
            .expect("reopened workspace should be ensured");
        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("a later terminal transition should run cleanup again");
        assert_eq!(
            fs::read_to_string(ensured.handle.logs_dir().join("before_remove.txt"))
                .expect("before-remove hook should run after reopen")
                .lines()
                .collect::<Vec<_>>(),
            ["before_remove", "before_remove"]
        );
    }

    #[tokio::test]
    async fn runtime_failed_cleanup_overrides_terminal_workspace_retention() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(WorkspaceManagerConfig {
                root: workspace_root,
                hooks: HookConfig::default(),
                cleanup: CleanupConfig {
                    remove_terminal_workspaces: false,
                },
            })
            .expect("workspace manager should be constructed"),
        );
        let issue = sample_terminal_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let workspace = crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())
                .expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };
        let mut backend = RuntimeWorkspaceBackend::new(Arc::clone(&workspace_manager), &workflow);

        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("ordinary terminal cleanup should honor retention");
        assert!(ensured.handle.workspace_path().exists());

        backend
            .cleanup_failed_workspace(&workspace)
            .await
            .expect("failed cleanup should remove retained terminal workspace");
        assert!(!ensured.handle.workspace_path().exists());
    }

    #[tokio::test]
    async fn runtime_workspace_cleanup_runs_manager_cleanup_for_invalid_manifest() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(WorkspaceManagerConfig {
                root: workspace_root,
                hooks: HookConfig {
                    before_remove: Some(HookDefinition::shell(
                        "echo before_remove >> .opensymphony/logs/before_remove.txt",
                    )),
                    ..HookConfig::default()
                },
                cleanup: CleanupConfig {
                    remove_terminal_workspaces: false,
                },
            })
            .expect("workspace manager should be constructed"),
        );
        let issue = sample_terminal_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        workspace_manager
            .write_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                "{\"canonical\":\"thread\"}",
            )
            .await
            .expect("invalid conversation manifest should persist");
        let workspace = crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())
                .expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };
        let mut backend = RuntimeWorkspaceBackend::new(Arc::clone(&workspace_manager), &workflow);

        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("invalid manifest cleanup should still run manager cleanup");

        assert_eq!(
            fs::read_to_string(ensured.handle.logs_dir().join("before_remove.txt"))
                .expect("before-remove hook should run for invalid manifest")
                .trim(),
            "before_remove"
        );
        assert!(ensured.handle.conversation_manifest_path().is_file());
        assert!(backend.terminal_cleanup_paths.contains(&workspace.path));
        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("repeated terminal cleanup should keep hooks once-only");

        assert_eq!(
            fs::read_to_string(ensured.handle.logs_dir().join("before_remove.txt"))
                .expect("before-remove hook should remain once-only")
                .trim(),
            "before_remove"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_workspace_cleanup_skips_rearchiving_until_the_issue_reopens() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_terminal_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut manifest = sample_conversation_manifest("fake-thread");
        manifest.transport_target = Some(CODEX_APP_SERVER_KIND.to_string());
        manifest.runtime_contract_version = Some(CODEX_APP_SERVER_CONTRACT.to_string());
        workspace_manager
            .write_json_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &manifest,
            )
            .await
            .expect("Codex conversation manifest should persist");
        let workspace = crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())
                .expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };
        let log_path = tempdir.path().join("fake-codex-terminal-rearchive.log");
        let fake_codex = tempdir.path().join("fake-codex-terminal-rearchive");
        write_fake_codex_child(&fake_codex, &log_path);
        let mut backend = RuntimeWorkspaceBackend::new(Arc::clone(&workspace_manager), &workflow);
        backend.codex_bin = fake_codex.to_string_lossy().into_owned();

        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("initial terminal archive should succeed");
        let lifecycle_processes = fs::read_to_string(&log_path)
            .expect("fake Codex lifecycle log should exist")
            .matches("PWD=")
            .count();
        assert_eq!(
            lifecycle_processes, 2,
            "archive-state inspection and archive should use one process each"
        );

        let unarchived_manifest_raw = workspace_manager
            .read_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
            )
            .await
            .expect("manifest read should succeed")
            .expect("manifest should exist");
        let mut unarchived_manifest: IssueConversationManifest =
            serde_json::from_str(&unarchived_manifest_raw)
                .expect("Codex conversation manifest should decode");
        unarchived_manifest.codex_archive_state = Some("active".to_string());
        workspace_manager
            .write_json_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &unarchived_manifest,
            )
            .await
            .expect("debug unarchive state should persist");

        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("later terminal poll should remain once-only");

        let rearchived_manifest_raw = workspace_manager
            .read_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
            )
            .await
            .expect("manifest read should succeed")
            .expect("manifest should exist");
        let retained_manifest: IssueConversationManifest =
            serde_json::from_str(&rearchived_manifest_raw)
                .expect("Codex conversation manifest should decode");
        assert_eq!(
            retained_manifest.codex_archive_state.as_deref(),
            Some("active")
        );
    }

    #[tokio::test]
    async fn runtime_workspace_cleanup_ignores_removed_terminal_workspace() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_terminal_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let workspace = crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())
                .expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };
        fs::remove_dir_all(ensured.handle.workspace_path())
            .expect("terminal workspace should be removable out of band");
        let mut backend = RuntimeWorkspaceBackend::new(Arc::clone(&workspace_manager), &workflow);

        backend
            .cleanup_workspace(&workspace, true)
            .await
            .expect("an already-removed terminal workspace should be a no-op");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_first_run_prompt_render_failure_does_not_start_a_thread() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow_with_prompt(
            tempdir.path(),
            &workspace_root,
            "{{ issue.missing_field }}",
        );
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let log_path = tempdir.path().join("fake-codex-render-failure.log");
        let fake_codex = tempdir.path().join("fake-codex-render-failure");
        write_fake_codex_child(&fake_codex, &log_path);

        let (outcome, _, launch) = run_fake_codex_attempt(
            &workflow,
            &workspace_manager,
            &ensured.handle,
            &issue,
            &codex_test_route(false),
            &fake_codex,
            "render-failure",
            1,
            None,
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Failed);
        assert!(
            matches!(launch, LaunchReport::Failed(detail) if detail.contains("failed to render workflow prompt"))
        );
        assert!(
            !ensured.handle.conversation_manifest_path().exists(),
            "a failed first render must not persist an unseeded manifest"
        );
        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert!(!log.contains(r#""method":"thread/start""#));
        assert!(!log.contains(r#""method":"thread/archive""#));
        assert!(!log.contains(r#""method":"turn/start""#));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_recovery_reconciles_without_starting_a_new_turn() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-recovery", 1),
            )
            .await
            .expect("run should start");
        run_manifest.status = RunStatus::Running;
        workspace_manager
            .write_run_manifest(&ensured.handle, &run_manifest)
            .await
            .expect("running status should be persisted");

        let mut conversation_manifest = sample_conversation_manifest("fake-thread");
        conversation_manifest.issue_id = issue.id.clone();
        conversation_manifest.identifier = issue.identifier.clone();
        conversation_manifest.persistence_dir = ensured.handle.metadata_dir();
        conversation_manifest.transport_target = Some(CODEX_APP_SERVER_KIND.to_string());
        conversation_manifest.runtime_contract_version =
            Some(CODEX_APP_SERVER_CONTRACT.to_string());
        conversation_manifest.codex_archive_state = Some("active".to_string());
        conversation_manifest.last_turn_id = Some("turn-1".to_string());
        workspace_manager
            .write_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &serde_json::to_string_pretty(&conversation_manifest)
                    .expect("conversation manifest should encode"),
            )
            .await
            .expect("conversation manifest should be written");

        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-recovery").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let log_path = tempdir.path().join("fake-codex-recovery.log");
        let fake_codex = tempdir.path().join("fake-codex-recovery");
        write_fake_codex_recovery_child(&fake_codex, &log_path);
        let (updates_tx, _updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();

        let outcome = run_codex_stdio_issue_with_mode(
            &codex_test_route(false),
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex path should be utf-8"),
            &codex_schema_validators,
            &codex_interrupts,
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
            true,
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Cancelled);
        assert_eq!(run_manifest.status, RunStatus::Cancelled);
        assert!(matches!(
            launch_rx.await.expect("launch report should be sent"),
            LaunchReport::Conversation(conversation)
                if conversation.conversation_id.as_str() == "fake-thread"
        ));
        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert!(log.contains(r#""method":"thread/resume""#));
        assert!(!log.contains(r#""method":"turn/start""#));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_accepts_scheduler_interrupt_for_active_turn() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-interrupt", 1),
            )
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-interrupt").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir.path().join("fake-codex-interrupt.log");
        let fake_codex = tempdir.path().join("fake-codex-interrupt");
        write_fake_codex_interruptible_child(&fake_codex, &log_path);
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();
        let worker_codex_interrupts = Arc::clone(&codex_interrupts);
        let fake_codex_path = fake_codex
            .to_str()
            .expect("fake codex path should be utf-8")
            .to_string();

        let worker = tokio::spawn(async move {
            let outcome = run_codex_stdio_issue(
                &route,
                &workspace_manager,
                &ensured.handle,
                &mut run_manifest,
                &issue,
                &run,
                &workflow,
                &fake_codex_path,
                &codex_schema_validators,
                &worker_codex_interrupts,
                &updates_tx,
                &mut launch_tx,
                &BTreeMap::new(),
            )
            .await;
            (outcome, run_manifest.status)
        });

        let launch = timeout(Duration::from_secs(5), launch_rx)
            .await
            .expect("launch should not time out")
            .expect("launch sender should stay alive");
        assert!(matches!(
            launch,
            LaunchReport::Conversation(conversation)
                if conversation.conversation_id.as_str() == "fake-thread"
        ));

        let acknowledgement = send_codex_stdio_interrupt(
            &codex_interrupts,
            &HarnessInterruptCommand {
                run_id: "COE-505".to_string(),
                issue_id: IssueId::new("issue-codex-interrupt").expect("issue id should be valid"),
                harness_kind: CODEX_APP_SERVER_KIND.to_string(),
                conversation_id: ConversationId::new("fake-thread")
                    .expect("thread id should be valid"),
                turn_id: None,
                reason: HarnessInterruptReason::TrackerMergingSupersedesHumanReview,
                expected_next_state: HarnessInterruptExpectedNextState::CloseoutPending,
            },
        )
        .await
        .expect("active Codex stdio interrupt should be accepted");
        assert!(acknowledgement.accepted);
        assert!(
            acknowledgement
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("turn `turn-1`"))
        );

        let (outcome, status) = timeout(Duration::from_secs(5), worker)
            .await
            .expect("worker should finish after interrupt")
            .expect("worker task should not panic");
        assert_eq!(outcome.outcome, WorkerOutcomeKind::Cancelled);
        assert_eq!(status, RunStatus::Cancelled);
        let log = fs::read_to_string(&log_path).expect("fake child log should exist");
        assert!(log.contains("\"method\":\"turn/interrupt\""));
        assert!(log.contains("\"threadId\":\"fake-thread\""));
        assert!(log.contains("\"turnId\":\"turn-1\""));
        assert!(
            std::iter::from_fn(|| updates_rx.try_recv().ok()).any(|update| {
                matches!(
                    update,
                    WorkerUpdate::RuntimeEvent {
                        event_kind: Some(kind),
                        summary: Some(summary),
                        ..
                    } if kind == "codex.turn/completed"
                        && summary.contains("Codex turn interrupted turn-1")
                )
            })
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_rejects_scheduler_interrupt_error_response() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-interrupt-error", 1),
            )
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-interrupt-error").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir.path().join("fake-codex-interrupt-error.log");
        let fake_codex = tempdir.path().join("fake-codex-interrupt-error");
        write_fake_codex_interrupt_error_child(&fake_codex, &log_path);
        let (updates_tx, _updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();
        let worker_codex_interrupts = Arc::clone(&codex_interrupts);
        let fake_codex_path = fake_codex
            .to_str()
            .expect("fake codex path should be utf-8")
            .to_string();

        let worker = tokio::spawn(async move {
            let outcome = run_codex_stdio_issue(
                &route,
                &workspace_manager,
                &ensured.handle,
                &mut run_manifest,
                &issue,
                &run,
                &workflow,
                &fake_codex_path,
                &codex_schema_validators,
                &worker_codex_interrupts,
                &updates_tx,
                &mut launch_tx,
                &BTreeMap::new(),
            )
            .await;
            (outcome, run_manifest.status)
        });

        let launch = timeout(Duration::from_secs(5), launch_rx)
            .await
            .expect("launch should not time out")
            .expect("launch sender should stay alive");
        assert!(matches!(
            launch,
            LaunchReport::Conversation(conversation)
                if conversation.conversation_id.as_str() == "fake-thread"
        ));

        let error = send_codex_stdio_interrupt(
            &codex_interrupts,
            &HarnessInterruptCommand {
                run_id: "COE-505".to_string(),
                issue_id: IssueId::new("issue-codex-interrupt").expect("issue id should be valid"),
                harness_kind: CODEX_APP_SERVER_KIND.to_string(),
                conversation_id: ConversationId::new("fake-thread")
                    .expect("thread id should be valid"),
                turn_id: None,
                reason: HarnessInterruptReason::TrackerMergingSupersedesHumanReview,
                expected_next_state: HarnessInterruptExpectedNextState::CloseoutPending,
            },
        )
        .await
        .expect_err("Codex interrupt JSON-RPC error should reject acknowledgement");
        assert!(error.to_string().contains("fake interrupt rejected"));

        let (outcome, status) = timeout(Duration::from_secs(5), worker)
            .await
            .expect("worker should finish after fake terminal event")
            .expect("worker task should not panic");
        assert_eq!(outcome.outcome, WorkerOutcomeKind::Succeeded);
        assert_eq!(status, RunStatus::Succeeded);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_keeps_turn_id_seen_before_start_response() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-turn-id-before-response", 1),
            )
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-turn-id-before-response")
                .expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir
            .path()
            .join("fake-codex-turn-id-before-response.log");
        let fake_codex = tempdir.path().join("fake-codex-turn-id-before-response");
        write_fake_codex_turn_id_before_response_child(&fake_codex, &log_path);
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();

        let outcome = run_codex_stdio_issue(
            &route,
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex path should be utf-8"),
            &codex_schema_validators,
            &codex_interrupts,
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Succeeded);
        assert_eq!(run_manifest.status, RunStatus::Succeeded);
        let launch = launch_rx
            .await
            .expect("launch report should be sent before terminal completion");
        assert!(matches!(
            launch,
            LaunchReport::Conversation(conversation)
                if conversation.conversation_id.as_str() == "fake-thread"
        ));
        assert!(
            std::iter::from_fn(|| updates_rx.try_recv().ok()).any(|update| {
                matches!(
                    update,
                    WorkerUpdate::RuntimeEvent {
                        event_kind: Some(kind),
                        summary: Some(summary),
                        ..
                    } if kind == "codex.turn/started"
                        && summary.contains("turn-pre-response")
                )
            }),
            "pre-response turn/started notification should be forwarded and retained"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_keeps_terminal_notification_seen_before_start_response() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-out-of-order", 1),
            )
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-out-of-order").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir.path().join("fake-codex-out-of-order.log");
        let fake_codex = tempdir.path().join("fake-codex-out-of-order");
        write_fake_codex_terminal_before_response_child(&fake_codex, &log_path);
        let (updates_tx, mut updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();

        let outcome = run_codex_stdio_issue(
            &route,
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex path should be utf-8"),
            &codex_schema_validators,
            &codex_interrupts,
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Succeeded);
        assert_eq!(run_manifest.status, RunStatus::Succeeded);
        let launch = launch_rx.await.expect("launch report should still be sent");
        assert!(matches!(
            launch,
            LaunchReport::Conversation(conversation)
                if conversation.conversation_id.as_str() == "fake-thread"
        ));
        assert!(
            std::iter::from_fn(|| updates_rx.try_recv().ok()).any(|update| {
                matches!(
                    update,
                    WorkerUpdate::RuntimeEvent {
                        event_kind: Some(kind),
                        ..
                    } if kind == "codex.turn/completed"
                )
            }),
            "out-of-order terminal notification should still be forwarded"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_worker_surfaces_fake_child_json_rpc_error() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-error", 1),
            )
            .await
            .expect("run should start");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-error").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir.path().join("fake-codex-error.log");
        let fake_codex = tempdir.path().join("fake-codex-error");
        write_fake_codex_error_child(&fake_codex, &log_path);
        let (updates_tx, _updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();

        let outcome = run_codex_stdio_issue(
            &route,
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex error path should be utf-8"),
            &codex_schema_validators,
            &codex_interrupts,
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Failed);
        assert_eq!(run_manifest.status, RunStatus::Failed);
        let error = outcome.error.expect("failure should include detail");
        assert!(error.contains("JSON-RPC error"));
        assert!(error.contains("fake initialize failure"));
        assert!(!error.contains("fake child stderr before failure"));
        let launch = launch_rx
            .await
            .expect("launch failure should be reported to caller");
        assert!(matches!(
            launch,
            LaunchReport::Failed(detail)
                if detail.contains("fake initialize failure")
                    && !detail.contains("fake child stderr before failure")
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_stdio_error_path_records_workspace_finalization_failure() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspaces");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be ensured");
        let mut run_manifest = workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-fake-codex-error-finish", 1),
            )
            .await
            .expect("run should start");
        let run_manifest_path = ensured.handle.run_manifest_path();
        fs::remove_file(&run_manifest_path).expect("run manifest file should be removable");
        fs::create_dir(&run_manifest_path)
            .expect("run manifest path should be replaceable by a directory");
        let run = RunAttempt::new(
            WorkerId::new("worker-fake-codex-error-finish").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            ensured.handle.workspace_path().to_path_buf(),
            TimestampMs::new(1),
            None,
            8,
        );
        let route = codex_test_route(false);
        let log_path = tempdir.path().join("fake-codex-error-finish.log");
        let fake_codex = tempdir.path().join("fake-codex-error-finish");
        write_fake_codex_error_child(&fake_codex, &log_path);
        let (updates_tx, _updates_rx) = mpsc::unbounded_channel();
        let (launch_tx, launch_rx) = oneshot::channel();
        let mut launch_tx = Some(launch_tx);
        let codex_schema_validators = empty_codex_schema_cache();
        let codex_interrupts = empty_codex_interrupt_registry();

        let outcome = run_codex_stdio_issue(
            &route,
            &workspace_manager,
            &ensured.handle,
            &mut run_manifest,
            &issue,
            &run,
            &workflow,
            fake_codex
                .to_str()
                .expect("fake codex error path should be utf-8"),
            &codex_schema_validators,
            &codex_interrupts,
            &updates_tx,
            &mut launch_tx,
            &BTreeMap::new(),
        )
        .await;

        assert_eq!(outcome.outcome, WorkerOutcomeKind::Failed);
        assert_eq!(run_manifest.status, RunStatus::Failed);
        let error = outcome.error.expect("failure should include detail");
        assert!(error.contains("fake initialize failure"));
        assert!(error.contains("failed to finish Codex workspace run as failed"));
        assert!(error.contains("additionally failed to persist failed status"));
        let launch = launch_rx
            .await
            .expect("launch failure should be reported to caller");
        assert!(matches!(
            launch,
            LaunchReport::Failed(detail)
                if detail.contains("fake initialize failure")
                    && detail.contains("failed to finish Codex workspace run as failed")
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_schema_validator_cache_reuses_compiled_installed_schema() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let fake_codex = tempdir.path().join("fake-codex-schema");
        let count_path = tempdir.path().join("schema-count.log");
        write_fake_codex_schema_generator(&fake_codex, &count_path);
        let cache = empty_codex_schema_cache();
        let codex_bin = fake_codex
            .to_str()
            .expect("fake codex schema path should be utf-8");

        cached_installed_codex_schema_validator(&cache, codex_bin)
            .await
            .expect("first schema load should compile");
        cached_installed_codex_schema_validator(&cache, codex_bin)
            .await
            .expect("second schema load should use cache");

        let generations =
            fs::read_to_string(&count_path).expect("schema generation count should exist");
        assert_eq!(
            generations.lines().count(),
            1,
            "schema generation should run once per Codex binary path"
        );
        assert_eq!(cache.lock().await.len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_schema_validator_cache_invalidates_when_binary_changes() {
        use std::os::unix::fs::symlink;

        let tempdir = TempDir::new().expect("tempdir should exist");
        let fake_codex = tempdir.path().join("fake-codex-schema-changing");
        let first_codex = tempdir.path().join("fake-codex-schema-changing-first");
        let second_codex = tempdir.path().join("fake-codex-schema-changing-second");
        let count_path = tempdir.path().join("schema-count-changing.log");
        write_fake_codex_schema_generator_with_marker(&first_codex, &count_path, "first");
        write_fake_codex_schema_generator_with_marker(&second_codex, &count_path, "second marker");
        symlink(&first_codex, &fake_codex).expect("first fake codex symlink should be created");
        let cache = empty_codex_schema_cache();
        let codex_bin = fake_codex
            .to_str()
            .expect("fake codex schema path should be utf-8");

        cached_installed_codex_schema_validator(&cache, codex_bin)
            .await
            .expect("first schema load should compile");
        fs::remove_file(&fake_codex).expect("fake codex symlink should be removable");
        symlink(&second_codex, &fake_codex).expect("second fake codex symlink should be created");
        cached_installed_codex_schema_validator(&cache, codex_bin)
            .await
            .expect("changed binary should force a second schema load");

        let generations =
            fs::read_to_string(&count_path).expect("schema generation count should exist");
        assert_eq!(
            generations.lines().count(),
            2,
            "schema generation should run again after the Codex binary changes"
        );
    }

    #[tokio::test]
    async fn routing_dry_run_finishes_workspace_manifest_and_records_one_route_event() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        let workflow = Arc::new(sample_workflow(tempdir.path(), &workspace_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let mut backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            Arc::clone(&workspace_manager),
            None,
            BTreeMap::new(),
        );
        let issue = sample_issue();
        let workspace = sample_workspace(&workspace_root);
        let run = RunAttempt::new(
            WorkerId::new("worker-dry-run").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            workspace.path.clone(),
            TimestampMs::new(1),
            None,
            8,
        );

        let launch = backend
            .start_worker(WorkerStartRequest {
                issue: issue.clone(),
                workspace,
                run,
                route: codex_test_route(true),
            })
            .await
            .expect("dry-run worker should launch");

        assert_eq!(
            launch.conversation.last_event_kind.as_deref(),
            Some("routing.decision")
        );
        assert_eq!(launch.conversation.recent_activity.len(), 1);

        let mut saw_finished = false;
        for _ in 0..10 {
            let updates = backend
                .poll_updates()
                .await
                .expect("dry-run updates should poll");
            saw_finished |= updates
                .iter()
                .any(|update| matches!(update, WorkerUpdate::Finished { .. }));
            if saw_finished {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(saw_finished, "dry-run worker should finish");

        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should still be inspectable");
        let manifest = workspace_manager
            .load_run_manifest(&ensured.handle)
            .await
            .expect("run manifest should load")
            .expect("run manifest should exist");
        assert_eq!(manifest.status, RunStatus::Succeeded);
        assert!(
            manifest
                .status_detail
                .as_deref()
                .is_some_and(|detail| detail.contains("routing dry-run ended"))
        );
    }

    #[tokio::test]
    async fn recovered_worker_reuses_running_manifest_without_before_run_hook() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        let workflow = Arc::new(sample_workflow(tempdir.path(), &workspace_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(WorkspaceManagerConfig {
                root: workspace_root.clone(),
                hooks: HookConfig {
                    before_run: Some(HookDefinition::shell(
                        "echo before_run >> .opensymphony/logs/before_run.txt",
                    )),
                    ..HookConfig::default()
                },
                cleanup: CleanupConfig {
                    remove_terminal_workspaces: false,
                },
            })
            .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        let mut run_manifest = workspace_manager
            .start_run(&ensured.handle, &RunDescriptor::new("run-recovered", 1))
            .await
            .expect("initial run should be persisted");
        run_manifest.status = RunStatus::Running;
        workspace_manager
            .write_run_manifest(&ensured.handle, &run_manifest)
            .await
            .expect("running recovery manifest should be persisted");

        let mut backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            Arc::clone(&workspace_manager),
            None,
            BTreeMap::new(),
        );
        let workspace = sample_workspace(&workspace_root);
        let run = RunAttempt::new(
            WorkerId::new("worker-recovered").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            workspace.path.clone(),
            TimestampMs::new(1),
            None,
            8,
        );

        let launch = backend
            .recover_worker(WorkerStartRequest {
                issue: issue.clone(),
                workspace,
                run,
                route: codex_test_route(true),
            })
            .await
            .expect("recovered dry-run worker should launch");
        assert_eq!(
            launch.conversation.last_event_kind.as_deref(),
            Some("routing.decision")
        );

        for _ in 0..10 {
            if backend
                .poll_updates()
                .await
                .expect("recovered worker updates should poll")
                .iter()
                .any(|update| matches!(update, WorkerUpdate::Finished { .. }))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let hook_invocations = fs::read_to_string(ensured.handle.logs_dir().join("before_run.txt"))
            .expect("initial run should have invoked before_run")
            .lines()
            .count();
        assert_eq!(hook_invocations, 1);
        assert_eq!(
            workspace_manager
                .load_run_manifest(&ensured.handle)
                .await
                .expect("recovered run manifest should load")
                .expect("recovered run manifest should exist")
                .status,
            RunStatus::Succeeded
        );
    }

    #[test]
    fn transport_port_override_reports_missing_port_separately() {
        let url = Url::parse("custom-scheme://openhands.local").expect("URL should parse");

        let error = transport_port_override(&url).expect_err("custom scheme should need a port");

        assert!(matches!(
            error,
            RunCommandError::MissingTransportPort { value }
                if value == "custom-scheme://openhands.local"
        ));
    }

    #[test]
    fn overlay_environment_prefers_runtime_overrides() {
        let env = OverlayEnvironment {
            overrides: BTreeMap::from([(
                "LINEAR_API_KEY".to_string(),
                "Bearer minted".to_string(),
            )]),
        };

        assert_eq!(env.get("LINEAR_API_KEY").as_deref(), Some("Bearer minted"));
    }

    #[test]
    fn memory_env_injection_sets_worker_cli_scope() {
        let memory = RuntimeMemoryEnv {
            endpoint: "http://127.0.0.1:8765/mcp".to_string(),
            token: Some("read-token".to_string()),
            project: "project-alpha".to_string(),
            execution_repo: "/tmp/project-alpha/services/api".to_string(),
        };
        let mut env = BTreeMap::new();

        inject_memory_env(&mut env, &memory);

        assert_eq!(
            env.get("OPENSYMPHONY_MEMORY_ENDPOINT").map(String::as_str),
            Some("http://127.0.0.1:8765/mcp")
        );
        assert_eq!(
            env.get("OPENSYMPHONY_MEMORY_TOKEN").map(String::as_str),
            Some("read-token")
        );
        assert_eq!(
            env.get("OPENSYMPHONY_MEMORY_PROJECT").map(String::as_str),
            Some("project-alpha")
        );
        assert_eq!(
            env.get("OPENSYMPHONY_MEMORY_PROJECT_SET")
                .map(String::as_str),
            Some("project-alpha")
        );
        assert_eq!(
            env.get("OPENSYMPHONY_MEMORY_EXECUTION_REPO")
                .map(String::as_str),
            Some("/tmp/project-alpha/services/api")
        );
    }

    #[tokio::test]
    async fn start_worker_reports_workspace_setup_failures_before_launch() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let blocked_root = tempdir.path().join("workspace-root");
        fs::write(&blocked_root, "not a directory").expect("blocking file should be created");

        let workflow = Arc::new(sample_workflow(tempdir.path(), &blocked_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let mut backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            workspace_manager,
            None,
            BTreeMap::new(),
        );

        let issue = sample_issue();
        let workspace = sample_workspace(&blocked_root);
        let run = RunAttempt::new(
            WorkerId::new("worker-1").expect("worker id should be valid"),
            issue.id.clone(),
            issue.identifier.clone(),
            workspace.path.clone(),
            TimestampMs::new(1),
            None,
            8,
        );

        let error = backend
            .start_worker(WorkerStartRequest {
                issue,
                workspace,
                run,
                route: crate::opensymphony_orchestrator::HarnessRouteDecision {
                    task_type: "issue_execution".into(),
                    harness_kind: "openhands_agent_server".into(),
                    model: None,
                    model_profile: None,
                    reason: "test default route".into(),
                    dry_run: false,
                    user_override: false,
                },
            })
            .await
            .expect_err("workspace setup failure should fail the launch immediately");

        assert!(matches!(
            error,
            CliWorkerError::LaunchFailed(detail)
                if detail.contains("failed to ensure workspace")
        ));
        assert!(
            backend.tasks.is_empty(),
            "failed launches should not leave worker tasks behind"
        );
        assert!(
            backend
                .poll_updates()
                .await
                .expect("poll_updates should succeed")
                .is_empty(),
            "launch failures should be surfaced through start_worker, not queued as runtime updates",
        );
    }

    #[tokio::test]
    async fn recover_workspaces_loads_managed_manifests_and_inflight_runs() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        workspace_manager
            .start_run(&ensured.handle, &RunDescriptor::new("run-recovery", 2))
            .await
            .expect("run manifest should be written");
        let mut run_manifest = workspace_manager
            .load_run_manifest(&ensured.handle)
            .await
            .expect("run manifest should load")
            .expect("run manifest should exist");
        run_manifest.status = RunStatus::Running;
        workspace_manager
            .write_run_manifest(&ensured.handle, &run_manifest)
            .await
            .expect("running status should be persisted");
        let mut conversation_manifest = sample_conversation_manifest("conv-recovery");
        conversation_manifest.transport_target = Some(CODEX_APP_SERVER_KIND.to_string());
        workspace_manager
            .write_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &serde_json::to_string_pretty(&conversation_manifest)
                    .expect("conversation manifest should encode"),
            )
            .await
            .expect("conversation manifest should be written");

        let mut backend = RuntimeWorkspaceBackend::new(workspace_manager, &workflow);
        let recoveries = backend
            .recover_workspaces()
            .await
            .expect("workspace recovery should succeed");

        assert_eq!(recoveries.len(), 1);
        let recovered = &recoveries[0];
        assert_eq!(
            recovered.issue.identifier.to_string(),
            issue.identifier.to_string()
        );
        assert_eq!(recovered.issue.state.category, IssueStateCategory::Active);
        assert!(recovered.had_in_flight_run);
        assert_eq!(
            recovered.harness_kind.as_deref(),
            Some(CODEX_APP_SERVER_KIND)
        );
        let recovered_run = recovered
            .recovered_run
            .as_ref()
            .expect("in-flight run should be recovered");
        assert_eq!(recovered_run.worker_id.as_str(), "recovery");
        assert_eq!(
            recovered_run.conversation.conversation_id.as_str(),
            "conv-recovery"
        );
        assert_eq!(
            recovered_run.conversation.stream_state,
            RuntimeStreamState::Closed
        );
        assert_eq!(recovered.workspace.path, ensured.handle.workspace_path());
    }

    #[tokio::test]
    async fn recover_workspaces_retries_pre_run_records_instead_of_reattaching() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-prepared-recovery", 1),
            )
            .await
            .expect("prepared run should be persisted");
        let conversation_manifest = sample_conversation_manifest("conv-prepared-recovery");
        workspace_manager
            .write_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &serde_json::to_string_pretty(&conversation_manifest)
                    .expect("conversation manifest should encode"),
            )
            .await
            .expect("conversation manifest should be written");

        let mut backend = RuntimeWorkspaceBackend::new(workspace_manager, &workflow);
        let recoveries = backend
            .recover_workspaces()
            .await
            .expect("workspace recovery should succeed");

        assert_eq!(recoveries.len(), 1);
        assert!(recoveries[0].had_in_flight_run);
        assert!(recoveries[0].recovered_run.is_none());
    }

    #[tokio::test]
    async fn recover_workspaces_reattaches_prepared_codex_run_with_active_turn() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        workspace_manager
            .start_run(
                &ensured.handle,
                &RunDescriptor::new("run-prepared-codex-recovery", 1),
            )
            .await
            .expect("prepared run should be persisted");
        let mut conversation_manifest =
            sample_conversation_manifest("conv-prepared-codex-recovery");
        conversation_manifest.transport_target = Some(CODEX_APP_SERVER_KIND.to_owned());
        conversation_manifest.last_turn_id = Some("turn-active".to_owned());
        workspace_manager
            .write_text_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &serde_json::to_string_pretty(&conversation_manifest)
                    .expect("conversation manifest should encode"),
            )
            .await
            .expect("conversation manifest should be written");

        let mut backend = RuntimeWorkspaceBackend::new(workspace_manager, &workflow);
        let recoveries = backend
            .recover_workspaces()
            .await
            .expect("workspace recovery should succeed");

        assert_eq!(recoveries.len(), 1);
        assert!(recoveries[0].had_in_flight_run);
        assert!(recoveries[0].recovered_run.is_some());
    }

    #[tokio::test]
    async fn recover_workspaces_restores_a_pending_first_retry() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        let mut run_manifest = workspace_manager
            .start_run(&ensured.handle, &RunDescriptor::new("run-pending-retry", 1))
            .await
            .expect("run manifest should be written");
        workspace_manager
            .finish_run(&ensured.handle, &mut run_manifest, RunStatus::Failed)
            .await
            .expect("completed run should be persisted");
        run_manifest.pending_retry = true;
        run_manifest.retry_scheduled_at = Some(250);
        run_manifest.retry_due_at = Some(1_200);
        run_manifest.retry_reason = Some("failure".to_owned());
        run_manifest.retry_error = Some("transient failure".to_owned());
        workspace_manager
            .write_run_manifest(&ensured.handle, &run_manifest)
            .await
            .expect("pending retry marker should be persisted");

        let mut backend = RuntimeWorkspaceBackend::new(workspace_manager, &workflow);
        let recoveries = backend
            .recover_workspaces()
            .await
            .expect("workspace recovery should succeed");

        assert_eq!(recoveries.len(), 1);
        assert!(!recoveries[0].had_in_flight_run);
        assert!(recoveries[0].pending_retry);
        assert_eq!(recoveries[0].normal_retry_count, 0);
        assert_eq!(
            recoveries[0].retry_scheduled_at,
            Some(TimestampMs::new(250))
        );
        assert_eq!(recoveries[0].retry_due_at, Some(TimestampMs::new(1_200)));
        assert_eq!(recoveries[0].retry_reason, Some(RetryReason::Failure));
        assert_eq!(
            recoveries[0].retry_error.as_deref(),
            Some("transient failure")
        );
    }

    #[tokio::test]
    async fn persist_retry_pending_creates_manifest_when_launch_failed_before_start_run() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        let workspace = crate::opensymphony_domain::WorkspaceRecord {
            path: ensured.handle.workspace_path().to_path_buf(),
            workspace_key: WorkspaceKey::new(ensured.handle.workspace_key().to_string())
                .expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };
        let retry = RetryEntry {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            attempt: RetryAttempt::new(1).expect("retry attempt should be valid"),
            normal_retry_count: 1,
            scheduled_at: TimestampMs::new(250),
            due_at: TimestampMs::new(1_200),
            reason: RetryReason::Failure,
            error: Some("launch failed".to_owned()),
        };
        let mut backend = RuntimeWorkspaceBackend::new(workspace_manager.clone(), &workflow);

        backend
            .persist_retry_pending(&workspace, &retry)
            .await
            .expect("pending retry should create a durable manifest");

        let manifest = workspace_manager
            .load_run_manifest(&ensured.handle)
            .await
            .expect("run manifest should load")
            .expect("run manifest should exist");
        assert_eq!(manifest.status, RunStatus::PreparationFailed);
        assert!(manifest.pending_retry);
        // The pending manifest is synthetic: recovery increments its stored
        // predecessor once when reconstructing the queued retry.
        assert_eq!(manifest.normal_retry_count, 0);
        assert_eq!(manifest.retry_due_at, Some(1_200));
    }

    #[tokio::test]
    async fn retry_exhaustion_is_recovered_from_instance_state() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        let state_root = tempdir.path().join("state-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should exist");
        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let issue = sample_issue();
        let mut backend = RuntimeWorkspaceBackend::new_with_retention_and_state_root(
            workspace_manager,
            &workflow,
            false,
            state_root.clone(),
        );

        backend
            .persist_retry_exhaustion(&issue, 3)
            .await
            .expect("retry exhaustion should persist");
        backend
            .persist_retry_exhaustion(&issue, 4)
            .await
            .expect("retry exhaustion should replace an existing marker");
        let records = backend
            .recover_retry_exhaustion()
            .await
            .expect("retry exhaustion should recover");

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].issue.identifier, issue.identifier);
        assert_eq!(records[0].normal_retry_count, 4);
        assert!(state_root.join("retry-exhaustion/COE-284.json").is_file());
        assert!(!workspace_root.join("COE-284").exists());
    }

    #[tokio::test]
    async fn active_store_preparation_moves_legacy_current_issue_before_startup() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        let tool_dir = tempdir.path().join("openhands-server");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");
        fs::create_dir_all(&tool_dir).expect("tool dir should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let issue = sample_issue();
        let ensured = workspace_manager
            .ensure(&issue_descriptor(&issue))
            .await
            .expect("workspace should be created");
        let conversation_id =
            Uuid::parse_str("dd258bb7-cc1b-415c-9892-e19af34a2e66").expect("uuid");
        let store = OpenHandsConversationStorePaths::for_tool_dir(&tool_dir, tempdir.path())
            .expect("conversation store paths should resolve");
        let legacy_path = store.legacy_root.join(conversation_id.simple().to_string());
        fs::create_dir_all(&legacy_path).expect("legacy conversation should be created");
        let manifest = sample_issue_conversation_manifest(&issue, &ensured.handle, conversation_id);
        workspace_manager
            .write_json_artifact(
                &ensured.handle,
                &ensured.handle.conversation_manifest_path(),
                &manifest,
            )
            .await
            .expect("conversation manifest should be written");

        let report = prepare_active_conversation_store_for_issues(
            &workspace_manager,
            &store,
            &[sample_tracker_issue(&issue)],
        )
        .await
        .expect("active conversation store should prepare");

        assert_eq!(report.moved, 1);
        assert!(!legacy_path.exists());
        assert!(
            store
                .active
                .join(conversation_id.simple().to_string())
                .is_dir()
        );
    }

    #[tokio::test]
    async fn legacy_store_migration_archives_terminal_workspace_conversations_only() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        let tool_dir = tempdir.path().join("openhands-server");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");
        fs::create_dir_all(&tool_dir).expect("tool dir should be created");

        let workflow = sample_workflow(tempdir.path(), &workspace_root);
        let workspace_manager = WorkspaceManager::new(build_workspace_manager_config(&workflow))
            .expect("workspace manager should be constructed");
        let terminal_issue = sample_terminal_issue();
        let terminal_workspace = workspace_manager
            .ensure(&issue_descriptor(&terminal_issue))
            .await
            .expect("terminal workspace should be created");
        let active_issue = sample_issue();
        let active_workspace = workspace_manager
            .ensure(&issue_descriptor(&active_issue))
            .await
            .expect("active workspace should be created");
        let terminal_conversation_id =
            Uuid::parse_str("dd258bb7-cc1b-415c-9892-e19af34a2e66").expect("uuid");
        let active_conversation_id =
            Uuid::parse_str("7fbd147f-3599-4bda-b6de-079c8f813e22").expect("uuid");
        let store = OpenHandsConversationStorePaths::for_tool_dir(&tool_dir, tempdir.path())
            .expect("conversation store paths should resolve");
        let terminal_legacy_path = store
            .legacy_root
            .join(terminal_conversation_id.simple().to_string());
        let active_legacy_path = store
            .legacy_root
            .join(active_conversation_id.simple().to_string());
        fs::create_dir_all(&terminal_legacy_path)
            .expect("terminal legacy conversation should be created");
        fs::create_dir_all(&active_legacy_path)
            .expect("active legacy conversation should be created");
        workspace_manager
            .write_json_artifact(
                &terminal_workspace.handle,
                &terminal_workspace.handle.conversation_manifest_path(),
                &sample_issue_conversation_manifest(
                    &terminal_issue,
                    &terminal_workspace.handle,
                    terminal_conversation_id,
                ),
            )
            .await
            .expect("terminal conversation manifest should be written");
        workspace_manager
            .write_json_artifact(
                &active_workspace.handle,
                &active_workspace.handle.conversation_manifest_path(),
                &sample_issue_conversation_manifest(
                    &active_issue,
                    &active_workspace.handle,
                    active_conversation_id,
                ),
            )
            .await
            .expect("active conversation manifest should be written");

        let report = migrate_legacy_workspace_conversations(&workspace_manager, &store, &workflow)
            .await
            .expect("legacy conversations should migrate");

        assert_eq!(report.moved_to_archived, 1);
        assert_eq!(report.skipped_non_terminal, 1);
        assert!(!terminal_legacy_path.exists());
        assert!(
            store
                .archived
                .join(terminal_conversation_id.simple().to_string())
                .is_dir()
        );
        assert!(active_legacy_path.is_dir());
    }

    #[tokio::test]
    async fn build_runtime_transport_rejects_launcher_overrides_for_external_targets() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  endpoint: http://127.0.0.1:3001/graphql
  api_key: test-linear-key
  project_slug: sample-project
  active_states:
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: https://127.0.0.1:8000/runtime
  local_server:
    command:
      - bash
      - custom-run.sh
---

# Test Workflow

Run the scheduler.
"#,
        )
        .expect("workflow should parse")
        .resolve(tempdir.path(), &BTreeMap::new())
        .expect("workflow should resolve");
        let runtime = RunRuntimeConfig {
            config_path: None,
            central_config: false,
            config_generation: "test".to_owned(),
            target_repo: tempdir.path().to_path_buf(),
            workflow_path: tempdir.path().join("WORKFLOW.md"),
            workflow,
            bind: "127.0.0.1:3000".parse().expect("bind should parse"),
            tool_dir: None,
            openhands_conversation_store: None,
            retry_max_attempts: None,
            state_root: None,
            memory_catalog_root: None,
            retain_failed: true,
            preserve_terminal_workspaces: true,
            memory: super::super::config::RunMemoryConfig {
                auto_capture: true,
                auto_archive: false,
                server: None,
            },
        };

        let error = match build_runtime_transport(&runtime, None, None, &BTreeMap::new()).await {
            Ok(_) => panic!("external targets should reject launcher overrides"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            RunCommandError::Transport(OpenHandsError::InvalidConfiguration { detail })
                if detail.contains("openhands.local_server.command")
        ));
    }

    #[tokio::test]
    async fn runtime_worker_backend_aborts_tracked_tasks_on_drop() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = Arc::new(sample_workflow(tempdir.path(), &workspace_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let mut backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            workspace_manager,
            None,
            BTreeMap::new(),
        );

        let workspace = sample_workspace(&workspace_root);
        let run = RunAttempt::new(
            WorkerId::new("worker-drop").expect("worker id should be valid"),
            IssueId::new("issue-drop").expect("issue id should be valid"),
            IssueIdentifier::new("COE-286").expect("issue identifier should be valid"),
            workspace.path.clone(),
            TimestampMs::new(1),
            None,
            8,
        );
        let (aborted_tx, aborted_rx) = oneshot::channel();
        let handle = tokio::spawn(async move {
            let _notifier = AbortNotifier(Some(aborted_tx));
            pending::<()>().await;
        });
        backend
            .tasks
            .insert("worker-drop".to_string(), ActiveWorkerTask { handle, run });

        drop(backend);

        match timeout(Duration::from_millis(100), aborted_rx).await {
            Ok(Ok(())) | Ok(Err(_)) => {}
            Err(_) => panic!("dropping the backend should abort tracked tasks"),
        }
    }

    #[tokio::test]
    async fn codex_scheduler_interrupt_reports_missing_active_stdio_channel() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = Arc::new(sample_workflow(tempdir.path(), &workspace_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let mut backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            workspace_manager,
            None,
            BTreeMap::new(),
        );

        let error = backend
            .interrupt_worker(HarnessInterruptCommand {
                run_id: "COE-505".to_string(),
                issue_id: IssueId::new("issue-codex-interrupt").expect("issue id should be valid"),
                harness_kind: CODEX_APP_SERVER_KIND.to_string(),
                conversation_id: ConversationId::new("thread-1")
                    .expect("thread id should be valid"),
                turn_id: Some("turn-1".to_string()),
                reason: HarnessInterruptReason::TrackerMergingSupersedesHumanReview,
                expected_next_state: HarnessInterruptExpectedNextState::CloseoutPending,
            })
            .await
            .expect_err("Codex stdio scheduler interrupt without a live channel should fail");

        assert!(
            error.to_string().contains(
                "Codex stdio worker for thread `thread-1` does not have an active turn interrupt channel"
            )
        );
    }

    #[tokio::test]
    async fn unknown_scheduler_interrupt_harness_is_rejected_before_openhands() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = Arc::new(sample_workflow(tempdir.path(), &workspace_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let mut backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            workspace_manager,
            None,
            BTreeMap::new(),
        );

        let error = backend
            .interrupt_worker(HarnessInterruptCommand {
                run_id: "COE-492".to_string(),
                issue_id: IssueId::new("issue-unknown-interrupt")
                    .expect("issue id should be valid"),
                harness_kind: "experimental_worker".to_string(),
                conversation_id: ConversationId::new("conv-unknown")
                    .expect("conversation id should be valid"),
                turn_id: None,
                reason: HarnessInterruptReason::TrackerMergingSupersedesHumanReview,
                expected_next_state: HarnessInterruptExpectedNextState::CloseoutPending,
            })
            .await
            .expect_err("unknown harnesses must not be routed to OpenHands");

        assert!(error.to_string().contains(
            "harness `experimental_worker` does not expose a scheduler-side interrupt channel"
        ));
    }

    #[test]
    fn openhands_interrupt_acknowledgement_requires_a_stopped_state() {
        assert!(openhands_execution_stopped("paused"));
        assert!(openhands_execution_stopped("finished"));
        assert!(!openhands_execution_stopped("running"));
        assert!(!openhands_execution_stopped("waiting"));
    }

    #[tokio::test]
    async fn codex_route_uses_launch_timeout_buffer() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workspace_root = tempdir.path().join("workspace-root");
        fs::create_dir_all(&workspace_root).expect("workspace root should be created");

        let workflow = Arc::new(sample_workflow(tempdir.path(), &workspace_root));
        let workspace_manager = Arc::new(
            WorkspaceManager::new(build_workspace_manager_config(&workflow))
                .expect("workspace manager should be constructed"),
        );
        let backend = RuntimeWorkerBackend::new(
            OpenHandsClient::new(TransportConfig::new("http://127.0.0.1:1")),
            workflow,
            workspace_manager,
            None,
            BTreeMap::new(),
        );
        let codex_route = codex_test_route(false);
        let openhands_route = crate::opensymphony_orchestrator::HarnessRouteDecision {
            task_type: "issue_execution".into(),
            harness_kind: "openhands_agent_server".into(),
            model: None,
            model_profile: None,
            reason: "test default route".into(),
            dry_run: false,
            user_override: false,
        };

        assert_eq!(
            backend.launch_timeout_for_route(&codex_route),
            CODEX_WORKER_LAUNCH_TIMEOUT
        );
        assert_eq!(
            backend.launch_timeout_for_route(&openhands_route),
            DEFAULT_WORKER_LAUNCH_TIMEOUT
        );
        assert!(CODEX_WORKER_LAUNCH_TIMEOUT > CODEX_RESPONSE_TIMEOUT * 2);
    }

    #[test]
    fn workspace_manager_config_keeps_terminal_cleanup_outcome_aware() {
        let tempdir = TempDir::new().expect("tempdir should exist");
        let workflow = sample_workflow(
            &tempdir.path().join("repo"),
            &tempdir.path().join("workspaces"),
        );
        assert!(
            !build_workspace_manager_config_with_retention(&workflow, true, true)
                .cleanup
                .remove_terminal_workspaces
        );
        assert!(
            build_workspace_manager_config_with_retention(&workflow, false, false)
                .cleanup
                .remove_terminal_workspaces
        );
    }

    fn sample_workflow(base_dir: &Path, workspace_root: &Path) -> ResolvedWorkflow {
        sample_workflow_with_prompt(
            base_dir,
            workspace_root,
            "# Test Workflow\n\nRun the scheduler.",
        )
    }

    fn sample_workflow_with_prompt(
        base_dir: &Path,
        workspace_root: &Path,
        prompt: &str,
    ) -> ResolvedWorkflow {
        let source = format!(
            "---\ntracker:\n  kind: linear\n  endpoint: http://127.0.0.1:3001/graphql\n  api_key: test-linear-key\n  project_slug: sample-project\n  active_states:\n    - In Progress\n  terminal_states:\n    - Done\nworkspace:\n  root: {}\nopenhands:\n  transport:\n    base_url: http://127.0.0.1:1\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n{prompt}\n",
            workspace_root.display(),
        );
        WorkflowDefinition::parse(&source)
            .expect("workflow should parse")
            .resolve_with_process_env(base_dir)
            .expect("workflow should resolve")
    }

    fn sample_issue() -> NormalizedIssue {
        NormalizedIssue {
            id: IssueId::new("issue-1").expect("issue id should be valid"),
            identifier: IssueIdentifier::new("COE-284").expect("issue identifier should be valid"),
            title: "Test issue".to_string(),
            description: None,
            priority: None,
            state: IssueState {
                id: None,
                name: "In Progress".to_string(),
                category: IssueStateCategory::Active,
            },
            branch_name: None,
            pr_url: None,
            url: None,
            labels: Vec::new(),
            project_id: None,
            project_slug: None,
            project_name: None,
            parent_id: None,
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: None,
            updated_at: None,
        }
    }

    fn sample_terminal_issue() -> NormalizedIssue {
        let mut issue = sample_issue();
        issue.id = IssueId::new("issue-done").expect("issue id should be valid");
        issue.identifier =
            IssueIdentifier::new("COE-285").expect("issue identifier should be valid");
        issue.state = IssueState {
            id: None,
            name: "Done".to_string(),
            category: IssueStateCategory::Terminal,
        };
        issue
    }

    fn codex_test_route(dry_run: bool) -> crate::opensymphony_orchestrator::HarnessRouteDecision {
        crate::opensymphony_orchestrator::HarnessRouteDecision {
            task_type: "issue_execution".into(),
            harness_kind: "codex_app_server".into(),
            model: None,
            model_profile: Some("codex-chatgpt-local-keychain".into()),
            reason: "test codex route".into(),
            dry_run,
            user_override: false,
        }
    }

    const FAKE_CODEX_SCHEMA: &str = r#"{"$schema":"http://json-schema.org/draft-07/schema#","definitions":{"ClientRequest":{"type":"object","required":["jsonrpc","id","method","params"],"properties":{"jsonrpc":{"const":"2.0"},"id":{"type":"integer"},"method":{"enum":["initialize","thread/start","thread/resume","thread/list","thread/archive","thread/unarchive","turn/start","turn/interrupt"]},"params":{"type":"object"}}}}}"#;

    #[cfg(unix)]
    fn write_fake_codex_child(path: &Path, log_path: &Path) {
        write_fake_codex_child_with_thread_start_setup(path, log_path, "");
    }

    #[cfg(unix)]
    fn write_fake_codex_manifest_write_failure_child(path: &Path, log_path: &Path) {
        write_fake_codex_child_with_thread_start_setup(
            path,
            log_path,
            "mkdir -p .opensymphony/conversation.json",
        );
    }

    #[cfg(unix)]
    fn write_fake_codex_child_with_thread_start_setup(
        path: &Path,
        log_path: &Path,
        thread_start_setup: &str,
    ) {
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
      {thread_start_setup}
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"fake-thread"}}}}}}\n' "$id"
      ;;
    *'"method":"thread/resume"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"fake-thread"}}}}}}\n' "$id"
      ;;
    *'"method":"thread/list"'*)
      if printf '%s' "$line" | grep -q '"archived":true'; then
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"data":[],"nextCursor":null}}}}\n' "$id"
      else
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"data":[{{"id":"fake-thread"}}],"nextCursor":null}}}}\n' "$id"
      fi
      ;;
    *'"method":"thread/archive"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    *'"method":"thread/unarchive"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    *'"method":"turn/start"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"turn":{{"id":"turn-1","items":[],"status":"inProgress"}}}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turnId":"turn-1"}}}}\n'
      ;;
  esac
done
"#,
                log = log_path.display(),
                schema = FAKE_CODEX_SCHEMA,
                thread_start_setup = thread_start_setup
            ),
        );
    }

    #[cfg(unix)]
    fn write_fake_codex_recovery_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
while IFS= read -r line; do
  printf 'STDIN=%s\n' "$line" >> "{log}"
  id=$(printf '%s\n' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    *'"method":"thread/list"'*)
      if printf '%s' "$line" | grep -q '"archived":true'; then
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"data":[],"nextCursor":null}}}}\n' "$id"
      else
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"data":[{{"id":"fake-thread"}}],"nextCursor":null}}}}\n' "$id"
      fi
      ;;
    *'"method":"thread/resume"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"thread":{{"id":"fake-thread"}}}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turn":{{"id":"turn-1","status":"interrupted"}}}}}}\n'
      ;;
    *'"method":"turn/start"'*)
      printf 'unexpected turn/start\n' >&2
      exit 97
      ;;
  esac
done
"#,
                log = log_path.display(),
                schema = FAKE_CODEX_SCHEMA,
            ),
        );
    }

    #[cfg(unix)]
    fn write_fake_codex_resume_error_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
while IFS= read -r line; do
  printf 'STDIN=%s\n' "$line" >> "{log}"
  id=$(printf '%s\n' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    *'"method":"thread/list"'*)
      if printf '%s' "$line" | grep -q '"archived":true'; then
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"data":[],"nextCursor":null}}}}\n' "$id"
      else
        printf '{{"jsonrpc":"2.0","id":%s,"result":{{"data":[{{"id":"fake-thread"}}],"nextCursor":null}}}}\n' "$id"
      fi
      ;;
    *'"method":"thread/unarchive"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    *'"method":"thread/resume"'*)
      printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32000,"message":"fake resume rejected"}}}}\n' "$id"
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
    fn write_fake_codex_interruptible_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
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
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turn":{{"id":"turn-1","status":"interrupted"}}}}}}\n'
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"status":"accepted"}}}}\n' "$id"
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
    fn write_fake_codex_interrupt_error_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
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
      printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32000,"message":"fake interrupt rejected"}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turnId":"turn-1","status":"completed"}}}}\n'
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
    fn write_fake_codex_turn_id_before_response_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
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
      printf '{{"jsonrpc":"2.0","method":"turn/started","params":{{"threadId":"fake-thread","turnId":"turn-pre-response"}}}}\n'
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"turn":{{"items":[],"status":"inProgress"}}}}}}\n' "$id"
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","status":"completed"}}}}\n'
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
    fn write_fake_codex_terminal_before_response_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
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
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turnId":"turn-1"}}}}\n'
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"turn":{{"id":"turn-1","items":[],"status":"inProgress"}}}}}}\n' "$id"
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
    fn write_fake_codex_error_child(path: &Path, log_path: &Path) {
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
printf 'PWD=%s\n' "$PWD" > "{log}"
printf 'ARGS=%s\n' "$*" >> "{log}"
while IFS= read -r line; do
  printf 'STDIN=%s\n' "$line" >> "{log}"
  id=$(printf '%s\n' "$line" | sed -E 's/.*"id":([0-9]+).*/\1/')
  printf 'fake child stderr before failure\n' >&2
  printf '{{"jsonrpc":"2.0","id":%s,"error":{{"code":-32000,"message":"fake initialize failure"}}}}\n' "$id"
  exit 0
done
"#,
                log = log_path.display(),
                schema = FAKE_CODEX_SCHEMA
            ),
        );
    }

    #[cfg(unix)]
    fn write_fake_codex_schema_generator(path: &Path, count_path: &Path) {
        write_fake_codex_schema_generator_with_marker(path, count_path, "default");
    }

    #[cfg(unix)]
    fn write_fake_codex_schema_generator_with_marker(path: &Path, count_path: &Path, marker: &str) {
        write_executable(
            path,
            &format!(
                r#"#!/usr/bin/env bash
# {marker}
set -euo pipefail
if [ "${{1:-}}" = "app-server" ] && [ "${{2:-}}" = "generate-json-schema" ]; then
  printf 'generated\n' >> "{count}"
  out_dir="${{4:-}}"
  mkdir -p "$out_dir"
  cat > "$out_dir/codex_app_server_protocol.v2.schemas.json" <<'JSON'
{schema}
JSON
  exit 0
fi
echo "unexpected fake codex invocation: $*" >&2
exit 64
"#,
                count = count_path.display(),
                marker = marker,
                schema = FAKE_CODEX_SCHEMA
            ),
        );
    }

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;

        let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temp_path, contents).expect("fake executable should be written");
        let mut permissions = fs::metadata(&temp_path)
            .expect("fake executable metadata should load")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&temp_path, permissions).expect("fake executable should be executable");
        fs::rename(&temp_path, path).expect("fake executable should be replaced atomically");
    }

    fn sample_issue_conversation_manifest(
        issue: &NormalizedIssue,
        workspace: &WorkspaceHandle,
        conversation_id: Uuid,
    ) -> IssueConversationManifest {
        let now = chrono::Utc::now();
        IssueConversationManifest {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            conversation_id: ConversationId::new(conversation_id.to_string())
                .expect("conversation id should be valid"),
            reuse_policy: "per_issue".to_string(),
            server_base_url: None,
            transport_target: None,
            http_auth_mode: None,
            websocket_auth_mode: None,
            websocket_query_param_name: None,
            persistence_dir: workspace.workspace_path().join(".openhands"),
            created_at: now,
            updated_at: now,
            last_attached_at: now,
            launch_profile: None,
            llm_config_fingerprint: None,
            fresh_conversation: false,
            workflow_prompt_seeded: true,
            reset_reason: None,
            runtime_contract_version: None,
            codex_archive_state: None,
            last_turn_id: None,
            last_prompt_kind: None,
            last_prompt_at: None,
            last_prompt_path: None,
            last_execution_status: None,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            last_token_accumulation_at: None,
        }
    }

    fn sample_tracker_issue(issue: &NormalizedIssue) -> TrackerIssue {
        TrackerIssue {
            id: issue.id.to_string(),
            identifier: issue.identifier.to_string(),
            url: issue
                .url
                .clone()
                .unwrap_or_else(|| format!("https://linear.example/{}", issue.identifier)),
            title: issue.title.clone(),
            description: issue.description.clone(),
            priority: issue.priority,
            state: issue.state.name.clone(),
            state_kind: tracker_issue_state_kind_from_category(&issue.state.category),
            branch_name: issue.branch_name.clone(),
            pr_url: issue.pr_url.clone(),
            labels: issue.labels.clone(),
            project_id: issue.project_id.clone(),
            project_slug: issue.project_slug.clone(),
            project_name: issue.project_name.clone(),
            parent_id: issue.parent_id.as_ref().map(ToString::to_string),
            parent: None,
            project_milestone: None,
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    fn tracker_issue_state_kind_from_category(
        category: &IssueStateCategory,
    ) -> TrackerIssueStateKind {
        match category {
            IssueStateCategory::Active => TrackerIssueStateKind::Started,
            IssueStateCategory::NonActive => TrackerIssueStateKind::Unstarted,
            IssueStateCategory::Terminal => TrackerIssueStateKind::Completed,
        }
    }

    fn sample_workspace(workspace_root: &Path) -> crate::opensymphony_domain::WorkspaceRecord {
        crate::opensymphony_domain::WorkspaceRecord {
            path: workspace_root.join("COE-284"),
            workspace_key: WorkspaceKey::new("COE-284").expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        }
    }

    struct AbortNotifier(Option<oneshot::Sender<()>>);

    impl Drop for AbortNotifier {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
}
