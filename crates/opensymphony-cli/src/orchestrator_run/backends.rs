//! Runtime backend adapters for tracker, workspace, and worker orchestration.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use crate::opensymphony_codex::{
    CODEX_APP_SERVER_CONTRACT, CODEX_APP_SERVER_KIND, CodexAppServerAdapter,
    CodexAppServerSchemaValidator, CodexContractGeneration, CodexJsonRpcSession,
    JsonRpcRequestEnvelope, NormalizedCodexEvent, NormalizedCodexEventKind,
    codex_approval_request_from_event, codex_event_summary, normalize_server_notification,
};
use crate::opensymphony_domain::{
    ConversationId, ConversationMetadata, IssueId, IssueIdentifier, IssueState, IssueStateCategory,
    NormalizedIssue, RuntimeStreamState, TimestampMs, TrackerIssue, WorkerOutcomeKind,
    WorkerOutcomeRecord, WorkspaceKey,
};
use crate::opensymphony_linear::{LinearClient, LinearConfig, LinearError, WorkpadComment};
use crate::opensymphony_openhands::{
    ConversationMoveOutcome, ConversationStoreKind, IssueConversationManifest, IssueSessionError,
    IssueSessionObserver, IssueSessionPromptKind, IssueSessionResult, IssueSessionRunner,
    IssueSessionRunnerConfig, LocalServerSupervisor, LocalServerTooling, MemoryWorkerAccess,
    OPENHANDS_CONVERSATIONS_PATH_ENV, OpenHandsClient, OpenHandsConversationStorePaths,
    OpenHandsError, SupervisedServerConfig, SupervisorConfig, TransportConfig,
    WorkpadComment as SessionWorkpadComment, WorkpadCommentSource,
};
use crate::opensymphony_orchestrator::{
    RecoveryRecord, TrackerBackend, WorkerAbortReason, WorkerBackend, WorkerLaunch,
    WorkerStartRequest, WorkerUpdate, WorkspaceBackend,
};
use crate::opensymphony_workflow::{ProcessEnvironment, ResolvedWorkflow};
use crate::opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, RunDescriptor, RunManifest,
    RunStatus, WorkspaceError, WorkspaceHandle, WorkspaceManager, WorkspaceManagerConfig,
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

#[derive(Debug, Error)]
pub(super) enum CliWorkspaceError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Identifier(#[from] crate::opensymphony_domain::IdentifierError),
    #[error("failed to remove workspace {path}: {source}")]
    RemoveWorkspace {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
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
}

pub(super) struct RuntimeWorkerBackend {
    client: OpenHandsClient,
    workflow: Arc<ResolvedWorkflow>,
    workspace_manager: Arc<WorkspaceManager>,
    runner_config: IssueSessionRunnerConfig,
    workpad_comment_source: Option<Arc<dyn WorkpadCommentSource>>,
    codex_bin: String,
    codex_schema_validators: CodexSchemaValidatorCache,
    launch_timeout: Duration,
    updates_tx: mpsc::UnboundedSender<WorkerUpdate>,
    updates_rx: mpsc::UnboundedReceiver<WorkerUpdate>,
    tasks: HashMap<String, ActiveWorkerTask>,
}

type CodexSchemaValidatorCache = Arc<AsyncMutex<HashMap<String, CodexAppServerSchemaValidator>>>;

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

struct LinearWorkpadCommentSource {
    client: LinearClient,
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

pub(super) fn build_workspace_manager_config(
    workflow: &ResolvedWorkflow,
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
            remove_terminal_workspaces: false,
        },
    }
}

pub(super) async fn build_runtime_transport(
    runtime: &RunRuntimeConfig,
    prepared_tooling: Option<LocalServerTooling>,
    memory_env: Option<&RuntimeMemoryEnv>,
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

    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.client.terminal_issues().await
    }

    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<crate::opensymphony_domain::TrackerIssueStateSnapshot>, Self::Error> {
        self.client.issue_states_by_ids(issue_ids).await
    }
}

impl RuntimeWorkspaceBackend {
    pub(super) fn new(manager: Arc<WorkspaceManager>, workflow: &ResolvedWorkflow) -> Self {
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
        }
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
                had_in_flight_run,
            });
        }
        Ok(recoveries)
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &crate::opensymphony_domain::WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error> {
        if terminal {
            match fs::remove_dir_all(&workspace.path).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => {
                    return Err(CliWorkspaceError::RemoveWorkspace {
                        path: workspace.path.clone(),
                        source,
                    });
                }
            }
        }
        Ok(())
    }
}

impl RuntimeWorkerBackend {
    pub(super) fn new(
        client: OpenHandsClient,
        workflow: Arc<ResolvedWorkflow>,
        workspace_manager: Arc<WorkspaceManager>,
        memory_env: Option<RuntimeMemoryEnv>,
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
            codex_bin: env::var("OPENSYMPHONY_CODEX_BIN").unwrap_or_else(|_| "codex".into()),
            codex_schema_validators: Arc::new(AsyncMutex::new(HashMap::new())),
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

    fn spawn_worker_task(&mut self, request: WorkerStartRequest) -> PendingLaunch {
        let mut runner = IssueSessionRunner::new(self.client.clone(), self.runner_config.clone());
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
        let pending_route = route.clone();
        let codex_bin = self.codex_bin.clone();
        let codex_schema_validators = Arc::clone(&self.codex_schema_validators);
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
            let run_descriptor = RunDescriptor::new(format!("run-{launch_worker_id}"), attempt);
            let mut run_manifest = match workspace_manager
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
                let outcome = run_codex_stdio_issue(
                    &route,
                    &workspace_manager,
                    &ensured.handle,
                    &mut run_manifest,
                    &issue,
                    &run,
                    &workflow,
                    &codex_bin,
                    &codex_schema_validators,
                    &updates_tx,
                    &mut launch_tx,
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
            let result = runner
                .run_with_observer(
                    &workspace_manager,
                    &ensured.handle,
                    &mut run_manifest,
                    &issue,
                    &run,
                    &workflow,
                    &mut observer,
                )
                .await;

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

fn dry_run_conversation_metadata(
    run: &crate::opensymphony_domain::RunAttempt,
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: ConversationId::new(format!("route-preview-{}", run.worker_id))
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

fn inject_memory_env(
    env: &mut std::collections::BTreeMap<String, String>,
    memory: &RuntimeMemoryEnv,
) {
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
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    launch_tx: &mut Option<oneshot::Sender<LaunchReport>>,
) -> WorkerOutcomeRecord {
    match try_run_codex_stdio_issue(
        route,
        workspace_manager,
        workspace,
        issue,
        run,
        workflow,
        codex_bin,
        codex_schema_validators,
        updates_tx,
        launch_tx,
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
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    workflow: &ResolvedWorkflow,
    codex_bin: &str,
    codex_schema_validators: &CodexSchemaValidatorCache,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    launch_tx: &mut Option<oneshot::Sender<LaunchReport>>,
) -> Result<(WorkerOutcomeRecord, RunStatus), String> {
    let adapter =
        CodexAppServerAdapter::local_stdio(codex_bin, "opensymphony", env!("CARGO_PKG_VERSION"));
    let schema_validator =
        cached_installed_codex_schema_validator(codex_schema_validators, codex_bin).await?;
    let (program, args) = adapter.launch().to_command();
    let mut child = Command::new(&program)
        .args(args)
        .current_dir(workspace.workspace_path())
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
    let mut pending_terminal = None;

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
        &mut pending_terminal,
    )
    .await
    .map_err(|error| with_codex_stderr(error, &stderr_tail))?;

    let prompt = workflow
        .render_prompt(issue, run.attempt.map(|attempt| attempt.get()))
        .map_err(|source| format!("failed to render workflow prompt for Codex route: {source}"))?;
    let model = codex_model_from_route(route);
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
        .map_err(|source| format!("failed to build Codex thread/start request: {source}"))?;
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
        &mut pending_terminal,
    )
    .await
    .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
    let conversation_id = codex_thread_id_from_start_response(&thread_start_response)
        .map_err(|error| with_codex_stderr(error, &stderr_tail))?;
    write_codex_conversation_manifest(workspace_manager, workspace, issue, &conversation_id, route)
        .await
        .map_err(|error| with_codex_stderr(error.to_string(), &stderr_tail))?;
    if let Some(sender) = launch_tx.take() {
        let _ = sender.send(LaunchReport::Conversation(Box::new(
            codex_conversation_metadata(conversation_id.clone(), route),
        )));
    }
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
    read_response_line(
        &mut reader,
        turn_start.request.id,
        updates_tx,
        &run.worker_id.to_string(),
        issue,
        run,
        &mut pending_terminal,
    )
    .await
    .map_err(|error| with_codex_stderr(error, &stderr_tail))?;

    let terminal = read_until_codex_terminal(
        &mut reader,
        updates_tx,
        &run.worker_id.to_string(),
        issue,
        run,
        &mut pending_terminal,
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
        })
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

fn codex_thread_id_from_start_response(value: &serde_json::Value) -> Result<String, String> {
    let thread_id = value
        .get("result")
        .and_then(|result| {
            result
                .get("threadId")
                .or_else(|| result.get("thread_id"))
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    result
                        .get("thread")
                        .and_then(|thread| thread.get("id"))
                        .and_then(serde_json::Value::as_str)
                })
        })
        .filter(|thread_id| !thread_id.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "Codex thread/start response missing non-empty threadId/thread_id or thread.id: {value}"
            )
        })?;
    Ok(thread_id.to_string())
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

async fn read_response_line(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request_id: u64,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    worker_id: &str,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    pending_terminal: &mut Option<CodexTerminalOutcome>,
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
        if let Some(event) = emit_codex_notification(updates_tx, worker_id, issue, run, value)
            && pending_terminal.is_none()
            && let Some(outcome) = codex_terminal_outcome(&event)
        {
            *pending_terminal = Some(outcome);
        }
    }
}

fn codex_response_id_matches(value: &serde_json::Value, request_id: u64) -> bool {
    let Some(id) = value.get("id") else {
        return false;
    };
    id.as_u64() == Some(request_id) || id.as_str().is_some_and(|id| id == request_id.to_string())
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

async fn read_until_codex_terminal(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    updates_tx: &mpsc::UnboundedSender<WorkerUpdate>,
    worker_id: &str,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    pending_terminal: &mut Option<CodexTerminalOutcome>,
) -> Result<CodexTerminalOutcome, String> {
    if let Some(outcome) = pending_terminal.take() {
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
        if let Some(event) = emit_codex_notification(updates_tx, worker_id, issue, run, value)
            && let Some(outcome) = codex_terminal_outcome(&event)
        {
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
            (WorkerOutcomeKind::Succeeded, RunStatus::Succeeded)
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
                Some("cancelled" | "canceled") => {
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
) -> Result<(), String> {
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
        workflow_prompt_seeded: true,
        reset_reason: None,
        runtime_contract_version: Some(CODEX_APP_SERVER_CONTRACT.to_string()),
        last_prompt_kind: Some(IssueSessionPromptKind::Full),
        last_prompt_at: Some(now),
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
        let pending = self.spawn_worker_task(request);
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
            .map(|request| self.spawn_worker_task(request))
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
        url: None,
        labels: Vec::new(),
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
        ConversationId, IssueId, IssueIdentifier, IssueState, IssueStateCategory, RunAttempt,
        TrackerIssueStateKind, WorkerId, WorkspaceKey,
    };
    use crate::opensymphony_workflow::WorkflowDefinition;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

    fn empty_codex_schema_cache() -> CodexSchemaValidatorCache {
        Arc::new(AsyncMutex::new(HashMap::new()))
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
    fn codex_start_response_requires_real_thread_id() {
        let thread_id = codex_thread_id_from_start_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "threadId": "thread-7"
            }
        }))
        .expect("threadId should be accepted");
        assert_eq!(thread_id, "thread-7");

        let missing = codex_thread_id_from_start_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {}
        }))
        .expect_err("missing thread id should fail launch");
        assert!(missing.contains("missing non-empty threadId/thread_id"));

        let empty = codex_thread_id_from_start_response(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "thread_id": "  "
            }
        }))
        .expect_err("empty thread id should fail launch");
        assert!(empty.contains("missing non-empty threadId/thread_id"));
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
            &updates_tx,
            &mut launch_tx,
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
            &updates_tx,
            &mut launch_tx,
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
            &updates_tx,
            &mut launch_tx,
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
            &updates_tx,
            &mut launch_tx,
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
        let tempdir = TempDir::new().expect("tempdir should exist");
        let fake_codex = tempdir.path().join("fake-codex-schema-changing");
        let count_path = tempdir.path().join("schema-count-changing.log");
        write_fake_codex_schema_generator_with_marker(&fake_codex, &count_path, "first");
        let cache = empty_codex_schema_cache();
        let codex_bin = fake_codex
            .to_str()
            .expect("fake codex schema path should be utf-8");

        cached_installed_codex_schema_validator(&cache, codex_bin)
            .await
            .expect("first schema load should compile");
        write_fake_codex_schema_generator_with_marker(&fake_codex, &count_path, "second marker");
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
        assert_eq!(recovered.workspace.path, ensured.handle.workspace_path());
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
        .resolve_with_process_env(tempdir.path())
        .expect("workflow should resolve");
        let runtime = RunRuntimeConfig {
            config_path: None,
            target_repo: tempdir.path().to_path_buf(),
            workflow_path: tempdir.path().join("WORKFLOW.md"),
            workflow,
            bind: "127.0.0.1:3000".parse().expect("bind should parse"),
            tool_dir: None,
            openhands_conversation_store: None,
            memory: super::super::config::RunMemoryConfig {
                auto_capture: true,
                auto_archive: false,
                server: None,
            },
        };

        let error = match build_runtime_transport(&runtime, None, None).await {
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

    fn sample_workflow(base_dir: &Path, workspace_root: &Path) -> ResolvedWorkflow {
        let source = format!(
            "---\ntracker:\n  kind: linear\n  endpoint: http://127.0.0.1:3001/graphql\n  api_key: test-linear-key\n  project_slug: sample-project\n  active_states:\n    - In Progress\n  terminal_states:\n    - Done\nworkspace:\n  root: {}\nopenhands:\n  transport:\n    base_url: http://127.0.0.1:1\n    session_api_key_env: OPENHANDS_API_KEY\n---\n\n# Test Workflow\n\nRun the scheduler.\n",
            workspace_root.display()
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
            url: None,
            labels: Vec::new(),
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

    const FAKE_CODEX_SCHEMA: &str = r#"{"$schema":"http://json-schema.org/draft-07/schema#","definitions":{"ClientRequest":{"oneOf":[{"type":"object","required":["jsonrpc","id","method","params"],"properties":{"jsonrpc":{"const":"2.0"},"id":{"type":"integer"},"method":{"enum":["initialize"]},"params":{"type":"object"}}},{"type":"object","required":["jsonrpc","id","method","params"],"properties":{"jsonrpc":{"const":"2.0"},"id":{"type":"integer"},"method":{"enum":["thread/start"]},"params":{"type":"object","required":["approvalPolicy","cwd","sandbox"],"properties":{"approvalPolicy":{"enum":["never"]},"cwd":{"type":"string"},"sandbox":{"enum":["danger-full-access"]}}}}},{"type":"object","required":["jsonrpc","id","method","params"],"properties":{"jsonrpc":{"const":"2.0"},"id":{"type":"integer"},"method":{"enum":["turn/start"]},"params":{"type":"object","required":["approvalPolicy","cwd","input","sandboxPolicy","threadId"],"properties":{"approvalPolicy":{"enum":["never"]},"cwd":{"type":"string"},"input":{"type":"array"},"sandboxPolicy":{"type":"object","required":["type"],"properties":{"type":{"enum":["dangerFullAccess"]}},"additionalProperties":false},"threadId":{"type":"string"}}}}}]}}}"#;

    #[cfg(unix)]
    fn write_fake_codex_child(path: &Path, log_path: &Path) {
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
      printf '{{"jsonrpc":"2.0","method":"turn/completed","params":{{"threadId":"fake-thread","turnId":"turn-1"}}}}\n'
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

        fs::write(path, contents).expect("fake executable should be written");
        let mut permissions = fs::metadata(path)
            .expect("fake executable metadata should load")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fake executable should be executable");
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
            labels: issue.labels.clone(),
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
