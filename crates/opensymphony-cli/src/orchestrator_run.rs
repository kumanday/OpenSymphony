use std::{
    collections::{HashMap, HashSet, VecDeque},
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use clap::Args;
use opensymphony_control::{
    AgentServerStatus, ControlPlaneServer, DaemonSnapshot, DaemonState, DaemonStatus,
    IssueRuntimeState, IssueSnapshot, MetricsSnapshot, RecentEvent, RecentEventKind, SnapshotStore,
    WorkerOutcome,
};
use opensymphony_domain::{
    ConversationMetadata, HealthStatus, IssueIdentifier, NormalizedIssue, OrchestratorSnapshot,
    SchedulerStatus, TimestampMs, WorkerOutcomeKind, WorkerOutcomeRecord, WorkspaceKey,
    WorkspaceRecord,
};
use opensymphony_linear::{LinearClient, LinearConfig, LinearError};
use opensymphony_openhands::{
    IssueSessionError, IssueSessionObserver, IssueSessionResult, IssueSessionRunner,
    IssueSessionRunnerConfig, LocalServerSupervisor, LocalServerTooling, OpenHandsClient,
    OpenHandsError, SupervisedServerConfig, SupervisorConfig, TransportConfig,
};
use opensymphony_orchestrator::{
    RecoveryRecord, Scheduler, SchedulerConfig, SchedulerError, TrackerBackend, WorkerAbortReason,
    WorkerBackend, WorkerLaunch, WorkerStartRequest, WorkerUpdate, WorkspaceBackend,
};
use opensymphony_workflow::{ProcessEnvironment, ResolvedWorkflow, WorkflowDefinition};
use opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, RunDescriptor, WorkspaceError,
    WorkspaceManager, WorkspaceManagerConfig,
};
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    fs,
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::{MissedTickBehavior, interval, timeout},
};
use tracing::{info, warn};
use url::Url;

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const DEFAULT_CONTROL_PLANE_BIND: &str = "127.0.0.1:3000";
const RECENT_EVENT_LIMIT: usize = 24;
const DEFAULT_WORKER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Args, Clone)]
pub struct RunArgs {
    #[arg(help = "Runtime config YAML path; defaults to ./config.yaml when present")]
    #[arg(long)]
    pub config: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct RunConfigFile {
    #[serde(default)]
    target_repo: Option<String>,
    #[serde(default)]
    control_plane: ControlPlaneConfigFile,
    #[serde(default)]
    openhands: RunOpenHandsConfigFile,
}

#[derive(Debug, Default, Deserialize)]
struct ControlPlaneConfigFile {
    #[serde(default)]
    bind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RunOpenHandsConfigFile {
    #[serde(default)]
    tool_dir: Option<String>,
}

struct RunRuntimeConfig {
    config_path: Option<PathBuf>,
    target_repo: PathBuf,
    workflow_path: PathBuf,
    workflow: ResolvedWorkflow,
    bind: SocketAddr,
    tool_dir: Option<PathBuf>,
}

#[derive(Debug, Error)]
enum RunCommandError {
    #[error("failed to determine the current working directory: {0}")]
    CurrentDir(#[source] std::io::Error),
    #[error("failed to read {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to expand {path}: {detail}")]
    ResolveConfig { path: PathBuf, detail: String },
    #[error("invalid control-plane bind address `{value}`: {source}")]
    InvalidBind {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
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
    #[error("failed to build tracker client: {0}")]
    Tracker(#[from] LinearError),
    #[error("failed to create workspace manager: {0}")]
    WorkspaceManager(#[from] WorkspaceError),
    #[error("failed to prepare OpenHands transport: {0}")]
    Transport(#[from] OpenHandsError),
    #[error("failed to load local OpenHands tooling: {0}")]
    Tooling(#[from] opensymphony_openhands::LocalToolingError),
    #[error("failed to start local OpenHands supervisor: {0}")]
    Supervisor(#[from] opensymphony_openhands::SupervisorError),
    #[error("failed to build scheduler configuration: {0}")]
    SchedulerConfig(#[from] SchedulerError),
    #[error("failed to bind control-plane listener: {0}")]
    BindListener(#[source] std::io::Error),
    #[error("control-plane server exited unexpectedly: {0}")]
    Serve(#[source] std::io::Error),
    #[error(
        "workflow config requires a local OpenHands server, but no `openhands.tool_dir` was provided via config"
    )]
    MissingToolDir,
    #[error(
        "OpenHands transport URL `{value}` does not include an explicit port and has no default port"
    )]
    MissingTransportPort { value: String },
}

#[derive(Debug, Error)]
enum CliWorkspaceError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Identifier(#[from] opensymphony_domain::IdentifierError),
    #[error("failed to remove workspace {path}: {source}")]
    RemoveWorkspace {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Error)]
enum CliWorkerError {
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

struct RuntimeTrackerBackend {
    client: LinearClient,
}

struct RuntimeWorkspaceBackend {
    manager: Arc<WorkspaceManager>,
}

struct RuntimeWorkerBackend {
    client: OpenHandsClient,
    workflow: Arc<ResolvedWorkflow>,
    workspace_manager: Arc<WorkspaceManager>,
    runner_config: IssueSessionRunnerConfig,
    launch_timeout: Duration,
    updates_tx: mpsc::UnboundedSender<WorkerUpdate>,
    updates_rx: mpsc::UnboundedReceiver<WorkerUpdate>,
    tasks: HashMap<String, ActiveWorkerTask>,
}

struct ActiveWorkerTask {
    handle: JoinHandle<()>,
    run: opensymphony_domain::RunAttempt,
}

struct SchedulerObserver {
    worker_id: String,
    launch_tx: Option<oneshot::Sender<LaunchReport>>,
    updates_tx: mpsc::UnboundedSender<WorkerUpdate>,
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
            worker_id: opensymphony_domain::WorkerId::new(worker_id)
                .expect("worker id should remain valid"),
            observed_at,
            event_id,
            event_kind,
            summary,
        });
    }
}

pub async fn run_command(args: RunArgs) -> ExitCode {
    match run_orchestrator(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_orchestrator(args: RunArgs) -> Result<(), RunCommandError> {
    let runtime = resolve_runtime_config(&args).await?;
    info!(
        config = runtime
            .config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        target_repo = %runtime.target_repo.display(),
        workflow = %runtime.workflow_path.display(),
        bind = %runtime.bind,
        "starting OpenSymphony orchestrator"
    );

    let tracker = build_tracker_backend(&runtime.workflow)?;
    let workspace_manager = Arc::new(WorkspaceManager::new(build_workspace_manager_config(
        &runtime.workflow,
    ))?);
    let workspace = RuntimeWorkspaceBackend {
        manager: workspace_manager.clone(),
    };

    let (transport, mut supervisor) = build_runtime_transport(&runtime).await?;
    let client = OpenHandsClient::new(transport);
    client.openapi_probe().await?;

    let worker = RuntimeWorkerBackend::new(
        client.clone(),
        Arc::new(runtime.workflow.clone()),
        workspace_manager,
    );
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        worker,
        SchedulerConfig::from_workflow(&runtime.workflow)?,
    );

    let mut recent_events = VecDeque::new();
    push_recent_event(
        &mut recent_events,
        RecentEventKind::SnapshotPublished,
        None,
        format!("loaded {}", runtime.workflow_path.display()),
        Utc::now(),
    );

    let initial_snapshot = map_snapshot(
        &scheduler.snapshot(now_timestamp()),
        runtime.workflow.config.workspace.root.as_path(),
        &terminal_state_set(&runtime.workflow),
        current_agent_server_status(&mut supervisor, client.base_url()),
        &recent_events,
    );

    let store = SnapshotStore::new(initial_snapshot);
    let listener = TcpListener::bind(runtime.bind)
        .await
        .map_err(RunCommandError::BindListener)?;
    let server = ControlPlaneServer::new(store.clone());
    let mut server_task = tokio::spawn(async move { server.serve(listener).await });

    let poll_interval = Duration::from_millis(runtime.workflow.config.polling.interval_ms);
    let mut ticker = interval(poll_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal");
                break;
            }
            result = &mut server_task => {
                match result {
                    Ok(Ok(())) => break,
                    Ok(Err(error)) => return Err(RunCommandError::Serve(error)),
                    Err(error) => return Err(RunCommandError::Serve(std::io::Error::other(error.to_string()))),
                }
            }
            _ = ticker.tick() => {
                let observed_at = now_timestamp();
                match scheduler.tick(observed_at).await {
                    Ok(snapshot) => {
                        push_recent_event(
                            &mut recent_events,
                            RecentEventKind::SnapshotPublished,
                            None,
                            format!(
                                "polled tracker; running={}, retry_queue={}",
                                snapshot.daemon.running_issue_count,
                                snapshot.daemon.retry_queue_count
                            ),
                            Utc::now(),
                        );
                        store.publish(map_snapshot(
                            &snapshot,
                            runtime.workflow.config.workspace.root.as_path(),
                            &terminal_state_set(&runtime.workflow),
                            current_agent_server_status(&mut supervisor, client.base_url()),
                            &recent_events,
                        )).await;
                    }
                    Err(error) => {
                        warn!(%error, "scheduler tick failed");
                        push_recent_event(
                            &mut recent_events,
                            RecentEventKind::Warning,
                            None,
                            format!("scheduler tick failed: {error}"),
                            Utc::now(),
                        );
                        let snapshot = scheduler.snapshot(observed_at);
                        store.publish(map_snapshot(
                            &snapshot,
                            runtime.workflow.config.workspace.root.as_path(),
                            &terminal_state_set(&runtime.workflow),
                            current_agent_server_status(&mut supervisor, client.base_url()),
                            &recent_events,
                        )).await;
                    }
                }
            }
        }
    }

    if let Some(mut supervisor) = supervisor {
        let _ = supervisor.stop();
    }

    Ok(())
}

async fn resolve_runtime_config(args: &RunArgs) -> Result<RunRuntimeConfig, RunCommandError> {
    let cwd = env::current_dir().map_err(RunCommandError::CurrentDir)?;
    let config_path = match &args.config {
        Some(path) => Some(resolve_relative_to(&cwd, path)),
        None => {
            let candidate = cwd.join(DEFAULT_CONFIG_FILE);
            candidate.exists().then_some(candidate)
        }
    };

    let config = match &config_path {
        Some(path) => load_run_config(path).await?,
        None => RunConfigFile::default(),
    };
    let config_root = config_path
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(cwd.as_path());
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|path| super::resolve_path(config_root, path))
        .unwrap_or_else(|| cwd.clone());
    let workflow_path = target_repo.join("WORKFLOW.md");
    let workflow = WorkflowDefinition::load_from_path(&workflow_path).map_err(|source| {
        RunCommandError::LoadWorkflow {
            path: workflow_path.clone(),
            source,
        }
    })?;
    let workflow = workflow
        .resolve_with_process_env(&target_repo)
        .map_err(|source| RunCommandError::ResolveWorkflow {
            path: workflow_path.clone(),
            source,
        })?;
    let bind_value = config
        .control_plane
        .bind
        .as_deref()
        .unwrap_or(DEFAULT_CONTROL_PLANE_BIND);
    let bind = bind_value
        .parse()
        .map_err(|source| RunCommandError::InvalidBind {
            value: bind_value.to_string(),
            source,
        })?;
    let tool_dir = config
        .openhands
        .tool_dir
        .as_deref()
        .map(|path| super::resolve_path(config_root, path));

    Ok(RunRuntimeConfig {
        config_path,
        target_repo,
        workflow_path,
        workflow,
        bind,
        tool_dir,
    })
}

async fn load_run_config(path: &Path) -> Result<RunConfigFile, RunCommandError> {
    let raw = fs::read_to_string(path)
        .await
        .map_err(|source| RunCommandError::ReadConfig {
            path: path.to_path_buf(),
            source,
        })?;
    let config = serde_yaml::from_str::<RunConfigFile>(&raw).map_err(|source| {
        RunCommandError::ParseConfig {
            path: path.to_path_buf(),
            source,
        }
    })?;
    resolve_run_config(path, config)
}

fn resolve_run_config(
    path: &Path,
    mut config: RunConfigFile,
) -> Result<RunConfigFile, RunCommandError> {
    config.target_repo = config
        .target_repo
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    config.control_plane.bind = config
        .control_plane
        .bind
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    config.openhands.tool_dir = config
        .openhands
        .tool_dir
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    Ok(config)
}

fn expand_run_value(path: &Path, value: String) -> Result<String, RunCommandError> {
    super::expand_env_tokens(&value).map_err(|error| RunCommandError::ResolveConfig {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })
}

fn resolve_relative_to(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn build_tracker_backend(
    workflow: &ResolvedWorkflow,
) -> Result<RuntimeTrackerBackend, LinearError> {
    let tracker = &workflow.config.tracker;
    let mut config = LinearConfig::new(tracker.api_key.clone(), tracker.project_slug.clone());
    config.base_url = tracker.endpoint.clone();
    config.active_states = tracker.active_states.clone();
    config.terminal_states = tracker.terminal_states.clone();
    Ok(RuntimeTrackerBackend {
        client: LinearClient::new(config)?,
    })
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

async fn build_runtime_transport(
    runtime: &RunRuntimeConfig,
) -> Result<(TransportConfig, Option<LocalServerSupervisor>), RunCommandError> {
    let transport = TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)?;
    let Some(supervisor_base_url) = transport.managed_local_server_base_url()? else {
        return Ok((transport, None));
    };

    let tool_dir = runtime
        .tool_dir
        .clone()
        .ok_or(RunCommandError::MissingToolDir)?;
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
    let status = supervisor.start()?;
    let transport = TransportConfig::new(status.base_url).with_auth(transport.auth().clone());
    Ok((transport, Some(supervisor)))
}

impl TrackerBackend for RuntimeTrackerBackend {
    type Error = LinearError;

    async fn candidate_issues(
        &mut self,
    ) -> Result<Vec<opensymphony_domain::TrackerIssue>, Self::Error> {
        self.client.candidate_issues().await
    }

    async fn terminal_issues(
        &mut self,
    ) -> Result<Vec<opensymphony_domain::TrackerIssue>, Self::Error> {
        self.client.terminal_issues().await
    }

    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<opensymphony_domain::TrackerIssueStateSnapshot>, Self::Error> {
        self.client.issue_states_by_ids(issue_ids).await
    }
}

impl WorkspaceBackend for RuntimeWorkspaceBackend {
    type Error = CliWorkspaceError;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        _observed_at: TimestampMs,
    ) -> Result<WorkspaceRecord, Self::Error> {
        let ensured = self.manager.ensure(&issue_descriptor(issue)).await?;
        Ok(WorkspaceRecord {
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
        Ok(Vec::new())
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
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
    fn new(
        client: OpenHandsClient,
        workflow: Arc<ResolvedWorkflow>,
        workspace_manager: Arc<WorkspaceManager>,
    ) -> Self {
        let (updates_tx, updates_rx) = mpsc::unbounded_channel();
        Self {
            client,
            workflow: workflow.clone(),
            workspace_manager,
            runner_config: IssueSessionRunnerConfig::from_workflow(&workflow),
            launch_timeout: DEFAULT_WORKER_LAUNCH_TIMEOUT,
            updates_tx,
            updates_rx,
            tasks: HashMap::new(),
        }
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
        let runner = IssueSessionRunner::new(self.client.clone(), self.runner_config.clone());
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
                run: request.run.clone(),
            },
        );

        match timeout(self.launch_timeout, launch_rx).await {
            Ok(Ok(LaunchReport::Conversation(conversation))) => Ok(WorkerLaunch {
                conversation: *conversation,
            }),
            Ok(Ok(LaunchReport::Failed(detail))) => {
                if let Some(task) = self.tasks.remove(worker_id.as_str()) {
                    task.handle.await?;
                }
                Err(CliWorkerError::LaunchFailed(detail))
            }
            Ok(Err(_)) => {
                if let Some(task) = self.tasks.remove(worker_id.as_str()) {
                    task.handle.await?;
                }
                Err(CliWorkerError::LaunchChannelClosed)
            }
            Err(_) => {
                if let Some(task) = self.tasks.remove(worker_id.as_str()) {
                    task.handle.abort();
                }
                Err(CliWorkerError::LaunchTimeout(self.launch_timeout))
            }
        }
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
                    worker_id: opensymphony_domain::WorkerId::new(worker_id)
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
        worker_id: &opensymphony_domain::WorkerId,
        _reason: WorkerAbortReason,
    ) -> Result<(), Self::Error> {
        if let Some(task) = self.tasks.remove(worker_id.as_str()) {
            task.handle.abort();
        }
        Ok(())
    }
}

fn map_snapshot(
    snapshot: &OrchestratorSnapshot,
    workspace_root: &Path,
    terminal_states: &HashSet<String>,
    agent_server: AgentServerStatus,
    recent_events: &VecDeque<RecentEvent>,
) -> DaemonSnapshot {
    let generated_at = timestamp_to_datetime(snapshot.generated_at);
    let last_poll_at = snapshot
        .daemon
        .last_poll_at
        .map(timestamp_to_datetime)
        .unwrap_or(generated_at);
    DaemonSnapshot {
        generated_at,
        daemon: DaemonStatus {
            state: map_daemon_state(snapshot.daemon.health),
            last_poll_at,
            workspace_root: workspace_root.display().to_string(),
            status_line: format!(
                "poll={}ms, running={}, retry_queue={}",
                snapshot.daemon.poll_interval_ms,
                snapshot.daemon.running_issue_count,
                snapshot.daemon.retry_queue_count
            ),
        },
        agent_server,
        metrics: MetricsSnapshot {
            running_issues: snapshot.daemon.running_issue_count as u32,
            retry_queue_depth: snapshot.daemon.retry_queue_count as u32,
            total_tokens: snapshot.daemon.usage.total_tokens,
            total_cost_micros: snapshot.daemon.usage.estimated_cost_usd_micros.unwrap_or(0),
        },
        issues: snapshot
            .issues
            .iter()
            .map(|issue| map_issue(issue, terminal_states, generated_at))
            .collect(),
        recent_events: recent_events.iter().cloned().collect(),
    }
}

fn map_issue(
    issue: &opensymphony_domain::IssueSnapshot,
    terminal_states: &HashSet<String>,
    generated_at: DateTime<Utc>,
) -> IssueSnapshot {
    let runtime_state = match issue.runtime.state {
        SchedulerStatus::Running | SchedulerStatus::Claimed => IssueRuntimeState::Running,
        SchedulerStatus::RetryQueued => IssueRuntimeState::RetryQueued,
        SchedulerStatus::Released => match issue
            .last_worker_outcome
            .as_ref()
            .map(|outcome| outcome.outcome)
        {
            Some(
                WorkerOutcomeKind::Failed
                | WorkerOutcomeKind::TimedOut
                | WorkerOutcomeKind::Stalled,
            ) => IssueRuntimeState::Failed,
            _ => IssueRuntimeState::Completed,
        },
        SchedulerStatus::Unclaimed => IssueRuntimeState::Idle,
    };
    let last_outcome = map_worker_outcome(issue, runtime_state);
    let last_event_at = issue
        .conversation
        .as_ref()
        .and_then(|conversation| conversation.last_event_at)
        .map(timestamp_to_datetime)
        .or_else(|| {
            issue
                .last_worker_outcome
                .as_ref()
                .map(|outcome| timestamp_to_datetime(outcome.finished_at))
        })
        .unwrap_or(generated_at);

    IssueSnapshot {
        identifier: issue.issue.identifier.to_string(),
        title: issue.issue.title.clone(),
        tracker_state: issue.issue.state.name.clone(),
        runtime_state,
        last_outcome,
        last_event_at,
        conversation_id_suffix: issue
            .conversation
            .as_ref()
            .map(|conversation| suffix(conversation.conversation_id.as_str()))
            .unwrap_or_else(|| "-".to_string()),
        workspace_path_suffix: issue
            .workspace
            .as_ref()
            .map(|workspace| suffix_path(&workspace.path))
            .unwrap_or_else(|| "-".to_string()),
        retry_count: issue
            .retry
            .as_ref()
            .map(|retry| retry.normal_retry_count)
            .unwrap_or(0),
        blocked: issue.issue.blocked_by.iter().any(|blocker| {
            blocker
                .state
                .as_deref()
                .is_none_or(|state| !is_terminal_state(terminal_states, state))
        }) || (!issue.issue.sub_issues.is_empty()
            && issue
                .issue
                .sub_issues
                .iter()
                .any(|sub_issue| !is_terminal_state(terminal_states, &sub_issue.state))),
        server_base_url: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.server_base_url.clone()),
        transport_target: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.transport_target.clone()),
        http_auth_mode: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.http_auth_mode.clone()),
        websocket_auth_mode: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.websocket_auth_mode.clone()),
        websocket_query_param_name: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.websocket_query_param_name.clone()),
    }
}

fn map_worker_outcome(
    issue: &opensymphony_domain::IssueSnapshot,
    runtime_state: IssueRuntimeState,
) -> WorkerOutcome {
    match runtime_state {
        IssueRuntimeState::Running => WorkerOutcome::Running,
        IssueRuntimeState::RetryQueued => match issue
            .last_worker_outcome
            .as_ref()
            .map(|outcome| outcome.outcome)
        {
            Some(WorkerOutcomeKind::Succeeded) => WorkerOutcome::Continued,
            Some(WorkerOutcomeKind::Cancelled) => WorkerOutcome::Canceled,
            Some(
                WorkerOutcomeKind::Failed
                | WorkerOutcomeKind::TimedOut
                | WorkerOutcomeKind::Stalled,
            ) => WorkerOutcome::Failed,
            None => WorkerOutcome::Continued,
        },
        IssueRuntimeState::Completed => match issue
            .last_worker_outcome
            .as_ref()
            .map(|outcome| outcome.outcome)
        {
            Some(WorkerOutcomeKind::Cancelled) => WorkerOutcome::Canceled,
            Some(
                WorkerOutcomeKind::Failed
                | WorkerOutcomeKind::TimedOut
                | WorkerOutcomeKind::Stalled,
            ) => WorkerOutcome::Failed,
            _ => WorkerOutcome::Completed,
        },
        IssueRuntimeState::Failed => WorkerOutcome::Failed,
        IssueRuntimeState::Idle => WorkerOutcome::Unknown,
        IssueRuntimeState::Releasing => WorkerOutcome::Unknown,
    }
}

fn current_agent_server_status(
    supervisor: &mut Option<LocalServerSupervisor>,
    base_url: &str,
) -> AgentServerStatus {
    if let Some(supervisor) = supervisor.as_mut()
        && let Ok(status) = supervisor.status()
    {
        return AgentServerStatus {
            reachable: matches!(status.state, opensymphony_openhands::ServerState::Ready),
            base_url: status.base_url,
            conversation_count: 0,
            status_line: format!("{:?}", status.state).to_ascii_lowercase(),
        };
    }

    AgentServerStatus {
        reachable: true,
        base_url: base_url.to_string(),
        conversation_count: 0,
        status_line: "reachable".to_string(),
    }
}

fn push_recent_event(
    recent_events: &mut VecDeque<RecentEvent>,
    kind: RecentEventKind,
    issue_identifier: Option<IssueIdentifier>,
    summary: String,
    happened_at: DateTime<Utc>,
) {
    recent_events.push_front(RecentEvent {
        happened_at,
        issue_identifier: issue_identifier.map(|identifier| identifier.to_string()),
        kind,
        summary,
    });
    while recent_events.len() > RECENT_EVENT_LIMIT {
        let _ = recent_events.pop_back();
    }
}

fn terminal_state_set(workflow: &ResolvedWorkflow) -> HashSet<String> {
    workflow
        .config
        .tracker
        .terminal_states
        .iter()
        .map(|state| state.trim().to_ascii_lowercase())
        .collect()
}

fn is_terminal_state(terminal_states: &HashSet<String>, state: &str) -> bool {
    terminal_states.contains(&state.trim().to_ascii_lowercase())
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

fn map_daemon_state(health: HealthStatus) -> DaemonState {
    match health {
        HealthStatus::Unknown | HealthStatus::Starting => DaemonState::Starting,
        HealthStatus::Healthy => DaemonState::Ready,
        HealthStatus::Degraded | HealthStatus::Failed => DaemonState::Degraded,
    }
}

fn suffix(value: &str) -> String {
    if value.len() <= 8 {
        value.to_string()
    } else {
        value[value.len() - 8..].to_string()
    }
}

fn suffix_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn timestamp_to_datetime(value: TimestampMs) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value.as_u64() as i64).unwrap_or_else(Utc::now)
}

fn datetime_to_timestamp_ms(value: DateTime<Utc>) -> TimestampMs {
    TimestampMs::new(value.timestamp_millis().max(0) as u64)
}

fn now_timestamp() -> TimestampMs {
    TimestampMs::new(Utc::now().timestamp_millis().max(0) as u64)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use opensymphony_domain::{
        IssueId, IssueState, IssueStateCategory, RunAttempt, WorkerId, WorkspaceKey,
    };
    use tempfile::TempDir;

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

    fn sample_workspace(workspace_root: &Path) -> WorkspaceRecord {
        WorkspaceRecord {
            path: workspace_root.join("COE-284"),
            workspace_key: WorkspaceKey::new("COE-284").expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        }
    }
}
