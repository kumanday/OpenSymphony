//! Runtime backend adapters for tracker, workspace, and worker orchestration.

use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

use opensymphony_domain::{
    ConversationMetadata, NormalizedIssue, TimestampMs, WorkerOutcomeKind, WorkerOutcomeRecord,
    WorkspaceKey,
};
use opensymphony_linear::{LinearClient, LinearConfig, LinearError};
use opensymphony_openhands::{
    IssueSessionError, IssueSessionObserver, IssueSessionResult, IssueSessionRunner,
    IssueSessionRunnerConfig, LocalServerSupervisor, LocalServerTooling, OpenHandsClient,
    OpenHandsError, SupervisedServerConfig, SupervisorConfig, TransportConfig,
};
use opensymphony_orchestrator::{
    RecoveryRecord, TrackerBackend, WorkerAbortReason, WorkerBackend, WorkerLaunch,
    WorkerStartRequest, WorkerUpdate, WorkspaceBackend,
};
use opensymphony_workflow::{ProcessEnvironment, ResolvedWorkflow};
use opensymphony_workspace::{
    CleanupConfig, HookConfig, HookDefinition, IssueDescriptor, RunDescriptor, WorkspaceError,
    WorkspaceManager, WorkspaceManagerConfig,
};
use thiserror::Error;
use tokio::{
    fs,
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::timeout,
};
use url::Url;

use super::{
    RunCommandError, config::RunRuntimeConfig, datetime_to_timestamp_ms, now_timestamp,
    timestamp_to_datetime,
};

const DEFAULT_WORKER_LAUNCH_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
pub(super) enum CliWorkspaceError {
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

pub(super) struct RuntimeWorkspaceBackend {
    manager: Arc<WorkspaceManager>,
}

pub(super) struct RuntimeWorkerBackend {
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

pub(super) fn build_tracker_backend(
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
) -> Result<(TransportConfig, Option<LocalServerSupervisor>), RunCommandError> {
    let transport = TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)?;
    let diagnostics = transport.diagnostics()?;
    let local_server = &runtime.workflow.extensions.openhands.local_server;
    let supervised = diagnostics.managed_local_server_candidate && local_server.enabled;
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

    let tool_dir = runtime
        .tool_dir
        .clone()
        .ok_or(RunCommandError::MissingToolDir)?;
    let tooling = LocalServerTooling::load(tool_dir)?;
    let base_url = &runtime.workflow.extensions.openhands.transport.base_url;
    let url = Url::parse(base_url).expect("validated transport base URL should parse");
    let mut config = SupervisedServerConfig::new(tooling);
    config.command = local_server.command.clone();
    config.extra_env = local_server.env.clone();
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

impl RuntimeWorkspaceBackend {
    pub(super) fn new(manager: Arc<WorkspaceManager>) -> Self {
        Self { manager }
    }
}

impl WorkspaceBackend for RuntimeWorkspaceBackend {
    type Error = CliWorkspaceError;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        _observed_at: TimestampMs,
    ) -> Result<opensymphony_domain::WorkspaceRecord, Self::Error> {
        let ensured = self.manager.ensure(&issue_descriptor(issue)).await?;
        Ok(opensymphony_domain::WorkspaceRecord {
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
        workspace: &opensymphony_domain::WorkspaceRecord,
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
    use std::{fs, path::Path};

    use opensymphony_domain::{
        IssueId, IssueIdentifier, IssueState, IssueStateCategory, RunAttempt, WorkerId,
        WorkspaceKey,
    };
    use opensymphony_workflow::WorkflowDefinition;
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
    base_url: http://127.0.0.1:8000/runtime
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
        };

        let error = match build_runtime_transport(&runtime).await {
            Ok(_) => panic!("external targets should reject launcher overrides"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            RunCommandError::Transport(OpenHandsError::InvalidConfiguration { detail })
                if detail.contains("openhands.local_server.command")
        ));
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

    fn sample_workspace(workspace_root: &Path) -> opensymphony_domain::WorkspaceRecord {
        opensymphony_domain::WorkspaceRecord {
            path: workspace_root.join("COE-284"),
            workspace_key: WorkspaceKey::new("COE-284").expect("workspace key should be valid"),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        }
    }
}
