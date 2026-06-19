//! Runtime backend adapters for tracker, workspace, and worker orchestration.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use crate::opensymphony_domain::{
    ConversationMetadata, IssueId, IssueIdentifier, IssueState, IssueStateCategory,
    NormalizedIssue, TimestampMs, TrackerIssue, WorkerOutcomeKind, WorkerOutcomeRecord,
    WorkspaceKey,
};
use crate::opensymphony_linear::{LinearClient, LinearConfig, LinearError, WorkpadComment};
use crate::opensymphony_openhands::{
    ConversationMoveOutcome, ConversationStoreKind, IssueConversationManifest, IssueSessionError,
    IssueSessionObserver, IssueSessionResult, IssueSessionRunner, IssueSessionRunnerConfig,
    LocalServerSupervisor, LocalServerTooling, MemoryWorkerAccess,
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
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, RunDescriptor, RunStatus,
    WorkspaceError, WorkspaceManager, WorkspaceManagerConfig,
};
use async_trait::async_trait;
use thiserror::Error;
use tokio::{
    fs,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};
use url::Url;

use super::{
    RunCommandError, RuntimeMemoryEnv, config::RunRuntimeConfig, datetime_to_timestamp_ms,
    now_timestamp, timestamp_to_datetime,
};

const DEFAULT_WORKER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);

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
    launch_timeout: Duration,
    updates_tx: mpsc::UnboundedSender<WorkerUpdate>,
    updates_rx: mpsc::UnboundedReceiver<WorkerUpdate>,
    tasks: HashMap<String, ActiveWorkerTask>,
}

struct ActiveWorkerTask {
    handle: JoinHandle<()>,
    run: crate::opensymphony_domain::RunAttempt,
}

struct PendingLaunch {
    worker_id: String,
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
    ) {
        let worker_id = self.worker_id.clone();
        let _ = self.updates_tx.send(WorkerUpdate::RuntimeEvent {
            worker_id: crate::opensymphony_domain::WorkerId::new(worker_id)
                .expect("worker id should remain valid"),
            observed_at,
            event_id,
            event_kind,
            summary,
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
            launch_rx,
        }
    }

    async fn resolve_launch_result(
        &mut self,
        worker_id: &str,
        result: Result<
            Result<LaunchReport, oneshot::error::RecvError>,
            tokio::time::error::Elapsed,
        >,
    ) -> Result<WorkerLaunch, CliWorkerError> {
        match result {
            Ok(Ok(LaunchReport::Conversation(conversation))) => Ok(WorkerLaunch {
                conversation: *conversation,
            }),
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
                Err(CliWorkerError::LaunchTimeout(self.launch_timeout))
            }
        }
    }
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
        self.resolve_launch_result(
            &worker_id,
            timeout(self.launch_timeout, pending.launch_rx).await,
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
        let ordered_worker_ids = pending
            .iter()
            .map(|launch| launch.worker_id.clone())
            .collect::<Vec<_>>();

        let mut waiters = Vec::with_capacity(pending.len());
        for launch in pending {
            let worker_id = launch.worker_id;
            let timeout_duration = self.launch_timeout;
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

        let mut launches = Vec::with_capacity(ordered_worker_ids.len());
        for worker_id in ordered_worker_ids {
            let outcome = completed
                .remove(&worker_id)
                .unwrap_or(Ok(Ok(LaunchReport::Failed(
                    "worker launch waiter finished without a result".to_string(),
                ))));
            launches.push(self.resolve_launch_result(&worker_id, outcome).await);
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
    use std::{collections::BTreeMap, fs, future::pending, path::Path};

    use crate::opensymphony_domain::{
        ConversationId, IssueId, IssueIdentifier, IssueState, IssueStateCategory, RunAttempt,
        TrackerIssueStateKind, WorkerId, WorkspaceKey,
    };
    use crate::opensymphony_workflow::WorkflowDefinition;
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

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

    fn sample_issue_conversation_manifest(
        issue: &NormalizedIssue,
        workspace: &crate::opensymphony_workspace::WorkspaceHandle,
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
