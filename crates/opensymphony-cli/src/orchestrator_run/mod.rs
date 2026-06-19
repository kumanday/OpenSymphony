pub(crate) mod backends;
mod config;
mod snapshot;

use std::{
    collections::{BTreeSet, VecDeque},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};

use crate::opensymphony_control::{RecentEvent, RecentEventKind, SnapshotStore};
use crate::opensymphony_domain::TimestampMs;
use crate::opensymphony_gateway::{GatewayServer, LinearTaskGraphClient};
use crate::opensymphony_linear::LinearError;
use crate::opensymphony_openhands::OpenHandsError;
use crate::opensymphony_orchestrator::{
    IssueStateCategory, OrchestratorSnapshot, Scheduler, SchedulerConfig, SchedulerError,
};
use crate::opensymphony_workspace::WorkspaceError;
use chrono::{DateTime, Utc};
use clap::Args;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    time::{MissedTickBehavior, interval},
};
use tracing::{info, warn};

use self::{
    backends::{
        RuntimeWorkerBackend, RuntimeWorkspaceBackend, build_linear_client,
        build_runtime_transport, build_tracker_backend, build_workspace_manager_config,
        prepare_active_conversation_store,
    },
    config::{RunRuntimeConfig, resolve_runtime_config},
    snapshot::{
        current_agent_server_status, current_memory_server_status, map_snapshot, push_recent_event,
        terminal_state_set,
    },
};

#[derive(Debug, Args, Clone)]
pub struct RunArgs {
    #[arg(help = "Runtime config YAML path; defaults to ./config.yaml when present")]
    #[arg(long)]
    pub config: Option<PathBuf>,
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
        source: crate::opensymphony_workflow::WorkflowLoadError,
    },
    #[error("failed to resolve workflow {path}: {source}")]
    ResolveWorkflow {
        path: PathBuf,
        #[source]
        source: crate::opensymphony_workflow::WorkflowConfigError,
    },
    #[error(
        "memory auto-capture is enabled but {path} is missing; run `opensymphony memory init` or `opensymphony update` from the target repo before `opensymphony run`"
    )]
    MissingMemoryConfig { path: PathBuf },
    #[error("failed to build tracker client: {0}")]
    Tracker(#[from] LinearError),
    #[error("failed to create workspace manager: {0}")]
    WorkspaceManager(#[from] WorkspaceError),
    #[error("failed to prepare OpenHands transport: {0}")]
    Transport(#[from] OpenHandsError),
    #[error("failed to prepare OpenHands conversation store: {0}")]
    ConversationStore(#[from] crate::opensymphony_openhands::ConversationStoreError),
    #[error(
        "managed local OpenHands tooling at {tool_dir} is missing or invalid: {detail}. Run `opensymphony install openhands` or `opensymphony doctor --config <path>`."
    )]
    ToolingSetupRequired { tool_dir: PathBuf, detail: String },
    #[error("failed to start local OpenHands supervisor: {0}")]
    Supervisor(#[from] crate::opensymphony_openhands::SupervisorError),
    #[error("failed to start memory server: {0}")]
    MemoryServer(#[from] crate::opensymphony_memory::MemoryError),
    #[error("failed to build scheduler configuration: {0}")]
    SchedulerConfig(#[from] SchedulerError),
    #[error("failed to bind control-plane listener: {0}")]
    BindListener(#[source] std::io::Error),
    #[error("control-plane server exited unexpectedly: {0}")]
    Serve(#[source] std::io::Error),
    #[error(
        "workflow config requires a managed local OpenHands server, but `openhands.tool_dir` is missing from config.yaml (recommended: ~/.opensymphony/openhands-server)"
    )]
    MissingToolDir,
    #[error(
        "OpenHands transport URL `{value}` does not include an explicit port and has no default port"
    )]
    MissingTransportPort { value: String },
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

    let mut tracker = build_tracker_backend(&runtime.workflow)?;
    let workspace_manager = Arc::new(crate::opensymphony_workspace::WorkspaceManager::new(
        build_workspace_manager_config(&runtime.workflow),
    )?);
    let workspace = RuntimeWorkspaceBackend::new(workspace_manager.clone(), &runtime.workflow);
    let managed_local_preparation =
        prepare_active_conversation_store(&runtime, &mut tracker, workspace_manager.as_ref())
            .await?;
    let active_store_preparation = &managed_local_preparation.active_conversations;
    let legacy_store_migration = &managed_local_preparation.legacy_conversations;
    if legacy_store_migration.moved_to_archived > 0 {
        info!(
            moved_to_archived = legacy_store_migration.moved_to_archived,
            already_archived = legacy_store_migration.already_archived,
            missing = legacy_store_migration.missing,
            skipped_non_terminal = legacy_store_migration.skipped_non_terminal,
            skipped_without_manifest = legacy_store_migration.skipped_without_manifest,
            skipped_invalid_manifest = legacy_store_migration.skipped_invalid_manifest,
            "migrated terminal OpenHands conversations into the repo archived store"
        );
    }
    if active_store_preparation.moved > 0 {
        info!(
            moved = active_store_preparation.moved,
            already_active = active_store_preparation.already_active,
            missing = active_store_preparation.missing,
            skipped_without_workspace = active_store_preparation.skipped_without_workspace,
            skipped_without_manifest = active_store_preparation.skipped_without_manifest,
            skipped_invalid_manifest = active_store_preparation.skipped_invalid_manifest,
            "prepared repo-scoped active OpenHands conversations before server startup"
        );
    }

    let memory_server = start_runtime_memory_server(&runtime).await?;
    let memory_env = memory_server.as_ref().map(|server| RuntimeMemoryEnv {
        endpoint: server.endpoint().to_string(),
        token: runtime
            .memory
            .server
            .as_ref()
            .and_then(|server| server.token.clone()),
        project: runtime.workflow.config.tracker.project_slug.clone(),
        execution_repo: runtime.target_repo.display().to_string(),
    });
    if let Some(env) = &memory_env {
        info!(endpoint = %env.endpoint, "started OpenSymphony memory server");
    }

    let (transport, mut supervisor) = build_runtime_transport(
        &runtime,
        managed_local_preparation.tooling,
        memory_env.as_ref(),
    )
    .await?;
    let client = crate::opensymphony_openhands::OpenHandsClient::new(transport);
    client.openapi_probe().await?;

    let worker = RuntimeWorkerBackend::new(
        client.clone(),
        Arc::new(runtime.workflow.clone()),
        workspace_manager,
        memory_env.clone(),
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
    if let Some(env) = &memory_env {
        push_recent_event(
            &mut recent_events,
            RecentEventKind::SnapshotPublished,
            None,
            format!("memory server listening at {}", env.endpoint),
            Utc::now(),
        );
    }

    let initial_snapshot = map_snapshot(
        &scheduler.snapshot(now_timestamp()),
        runtime.workflow.config.workspace.root.as_path(),
        &terminal_state_set(&runtime.workflow),
        current_agent_server_status(&mut supervisor, client.base_url()),
        current_memory_server_status(memory_server.as_ref()),
        &recent_events,
    );

    let store = SnapshotStore::new(initial_snapshot);
    let listener = TcpListener::bind(runtime.bind)
        .await
        .map_err(RunCommandError::BindListener)?;
    let server = GatewayServer::new(store.clone())
        .with_linear_task_graph(build_optional_task_graph_client(&runtime.workflow));
    let mut server_task = tokio::spawn(async move { server.serve(listener).await });

    let bootstrap_snapshot = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("received shutdown signal");
            server_task.abort();
            if let Some(server) = &memory_server {
                server.abort();
            }
            if let Some(mut supervisor) = supervisor {
                let _ = supervisor.stop();
            }
            return Ok(());
        }
        result = &mut server_task => {
            match result {
                Ok(Ok(())) => {
                    if let Some(mut supervisor) = supervisor {
                        let _ = supervisor.stop();
                    }
                    if let Some(server) = &memory_server {
                        server.abort();
                    }
                    return Ok(());
                }
                Ok(Err(error)) => return Err(RunCommandError::Serve(error)),
                Err(error) => return Err(RunCommandError::Serve(std::io::Error::other(error.to_string()))),
            }
        }
        result = scheduler.bootstrap(now_timestamp()) => result?,
    };
    let mut auto_capture_completed_issues = terminal_issue_identifiers(&bootstrap_snapshot);
    push_recent_event(
        &mut recent_events,
        RecentEventKind::SnapshotPublished,
        None,
        format!(
            "recovered startup state; running={}, retry_queue={}",
            bootstrap_snapshot.daemon.running_issue_count,
            bootstrap_snapshot.daemon.retry_queue_count
        ),
        Utc::now(),
    );
    store
        .publish(map_snapshot(
            &bootstrap_snapshot,
            runtime.workflow.config.workspace.root.as_path(),
            &terminal_state_set(&runtime.workflow),
            current_agent_server_status(&mut supervisor, client.base_url()),
            current_memory_server_status(memory_server.as_ref()),
            &recent_events,
        ))
        .await;

    let poll_interval =
        std::time::Duration::from_millis(runtime.workflow.config.polling.interval_ms);
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
            result = async {
                ticker.tick().await;
                let observed_at = now_timestamp();
                (observed_at, scheduler.tick(observed_at).await)
            } => {
                let (observed_at, result) = result;
                match result {
                    Ok(snapshot) => {
                        let current_terminal_issues = terminal_issue_identifiers(&snapshot);
                        let auto_capture_candidates = auto_capture_candidates(
                            &current_terminal_issues,
                            &mut auto_capture_completed_issues,
                            runtime.memory.auto_capture,
                        );
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
                            current_memory_server_status(memory_server.as_ref()),
                            &recent_events,
                        )).await;
                        if !auto_capture_candidates.is_empty() {
                            let auto_capture_result = super::memory::auto_capture_terminal(
                                &runtime.target_repo,
                                &runtime.workflow_path,
                                &auto_capture_candidates,
                                runtime.openhands_conversation_store.as_ref(),
                                runtime.memory.auto_archive,
                            )
                            .await;
                            mark_auto_capture_completed(
                                &mut auto_capture_completed_issues,
                                &auto_capture_candidates,
                                &auto_capture_result,
                            );
                            publish_auto_capture_event(
                                auto_capture_result,
                                &snapshot,
                                SnapshotPublishContext {
                                    runtime: &runtime,
                                    supervisor: &mut supervisor,
                                    agent_server_base_url: client.base_url(),
                                    memory_server: memory_server.as_ref(),
                                    recent_events: &mut recent_events,
                                    store: &store,
                                },
                            ).await;
                        }
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
                            current_memory_server_status(memory_server.as_ref()),
                            &recent_events,
                        )).await;
                    }
                }
            }
        }
    }

    server_task.abort();
    if let Some(server) = &memory_server {
        server.abort();
    }
    if let Some(mut supervisor) = supervisor {
        let _ = supervisor.stop();
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeMemoryEnv {
    pub(super) endpoint: String,
    pub(super) token: Option<String>,
    pub(super) project: String,
    pub(super) execution_repo: String,
}

async fn start_runtime_memory_server(
    runtime: &RunRuntimeConfig,
) -> Result<Option<super::memory::MemoryServerHandle>, RunCommandError> {
    let Some(server) = runtime.memory.server.as_ref() else {
        return Ok(None);
    };
    let config = crate::opensymphony_memory::MemoryConfig::load(&runtime.target_repo, None)?;
    super::memory::start_memory_server(config, server.bind, server.token.clone())
        .await
        .map(Some)
        .map_err(RunCommandError::MemoryServer)
}

async fn publish_auto_capture_event(
    result: Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
    snapshot: &OrchestratorSnapshot,
    context: SnapshotPublishContext<'_>,
) {
    if record_auto_capture_recent_event(context.recent_events, result) {
        context
            .store
            .publish(map_snapshot(
                snapshot,
                context.runtime.workflow.config.workspace.root.as_path(),
                &terminal_state_set(&context.runtime.workflow),
                current_agent_server_status(context.supervisor, context.agent_server_base_url),
                current_memory_server_status(context.memory_server),
                context.recent_events,
            ))
            .await;
    }
}

struct SnapshotPublishContext<'a> {
    runtime: &'a RunRuntimeConfig,
    supervisor: &'a mut Option<crate::opensymphony_openhands::LocalServerSupervisor>,
    agent_server_base_url: &'a str,
    memory_server: Option<&'a super::memory::MemoryServerHandle>,
    recent_events: &'a mut VecDeque<RecentEvent>,
    store: &'a SnapshotStore,
}

fn record_auto_capture_recent_event(
    recent_events: &mut VecDeque<RecentEvent>,
    result: Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
) -> bool {
    match result {
        Ok(report) => {
            if report.captured_issue_keys.is_empty() && report.warnings.is_empty() {
                return false;
            }
            let mut summary = if report.captured_issue_keys.is_empty() {
                "memory capture reported no new capsules".to_string()
            } else {
                format!(
                    "memory captured {} issue(s)",
                    report.captured_issue_keys.len()
                )
            };
            if !report.docs_written.is_empty() {
                summary.push_str(&format!(", synced {} doc(s)", report.docs_written.len()));
            }
            if !report.archived_issue_keys.is_empty() {
                summary.push_str(&format!(
                    ", archived {} issue(s)",
                    report.archived_issue_keys.len()
                ));
            }
            if !report.warnings.is_empty() {
                summary.push_str(&format!(", {} warning(s)", report.warnings.len()));
            }
            push_recent_event(
                recent_events,
                if report.warnings.is_empty() {
                    RecentEventKind::SnapshotPublished
                } else {
                    RecentEventKind::Warning
                },
                None,
                summary,
                Utc::now(),
            );
            true
        }
        Err(error) => {
            warn!(%error, "automatic memory capture failed");
            push_recent_event(
                recent_events,
                RecentEventKind::Warning,
                None,
                format!("automatic memory capture failed: {error}"),
                Utc::now(),
            );
            true
        }
    }
}

fn build_optional_task_graph_client(
    workflow: &crate::opensymphony_workflow::ResolvedWorkflow,
) -> Option<Arc<dyn LinearTaskGraphClient>> {
    optional_task_graph_client(build_linear_client(workflow))
}

fn optional_task_graph_client(
    client: Result<crate::opensymphony_linear::LinearClient, LinearError>,
) -> Option<Arc<dyn LinearTaskGraphClient>> {
    match client {
        Ok(client) => Some(Arc::new(client) as Arc<dyn LinearTaskGraphClient>),
        Err(error) => {
            warn!(
                %error,
                "Linear task graph reader unavailable; task graph endpoint will return 503"
            );
            None
        }
    }
}

fn terminal_issue_identifiers(snapshot: &OrchestratorSnapshot) -> BTreeSet<String> {
    snapshot
        .issues
        .iter()
        .filter(|issue| issue.issue.state.category == IssueStateCategory::Terminal)
        .map(|issue| issue.issue.identifier.to_string())
        .collect()
}

fn auto_capture_candidates(
    current_terminal_issues: &BTreeSet<String>,
    completed_issues: &mut BTreeSet<String>,
    auto_capture_enabled: bool,
) -> Vec<String> {
    completed_issues.retain(|issue| current_terminal_issues.contains(issue));
    if !auto_capture_enabled {
        *completed_issues = current_terminal_issues.clone();
        return Vec::new();
    }
    current_terminal_issues
        .difference(completed_issues)
        .cloned()
        .collect()
}

fn mark_auto_capture_completed(
    completed_issues: &mut BTreeSet<String>,
    candidates: &[String],
    result: &Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
) {
    match result {
        Ok(report) if report.workflow_completed() && !report.completed_issue_keys.is_empty() => {
            completed_issues.extend(report.completed_issue_keys.iter().cloned());
        }
        Ok(report) if report.workflow_completed() && report.warnings.is_empty() => {
            completed_issues.extend(candidates.iter().cloned());
        }
        Ok(_) | Err(_) => {}
    }
}

pub(super) fn timestamp_to_datetime(value: TimestampMs) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value.as_u64() as i64).unwrap_or_else(Utc::now)
}

pub(super) fn datetime_to_timestamp_ms(value: DateTime<Utc>) -> TimestampMs {
    TimestampMs::new(value.timestamp_millis().max(0) as u64)
}

pub(super) fn now_timestamp() -> TimestampMs {
    TimestampMs::new(Utc::now().timestamp_millis().max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_memory::MemoryError;

    fn issue_set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn optional_task_graph_client_returns_none_when_linear_reader_is_unavailable() {
        let client = optional_task_graph_client(Err(LinearError::InvalidConfiguration(
            "missing task graph config".to_owned(),
        )));

        assert!(
            client.is_none(),
            "gateway task graph reader should fail closed instead of aborting run startup",
        );
    }

    #[test]
    fn auto_capture_candidates_retry_until_capture_completes() {
        let current = issue_set(&["COE-1", "COE-2"]);
        let mut completed = issue_set(&["COE-1"]);

        let candidates = auto_capture_candidates(&current, &mut completed, true);

        assert_eq!(candidates, vec!["COE-2".to_string()]);
        mark_auto_capture_completed(
            &mut completed,
            &candidates,
            &Err(MemoryError::InvalidInput("capture failed".to_string())),
        );
        assert_eq!(completed, issue_set(&["COE-1"]));

        let retry_candidates = auto_capture_candidates(&current, &mut completed, true);
        assert_eq!(retry_candidates, vec!["COE-2".to_string()]);
    }

    #[test]
    fn auto_capture_candidates_forget_reopened_issues() {
        let current = issue_set(&["COE-2"]);
        let mut completed = issue_set(&["COE-1", "COE-2"]);

        let candidates = auto_capture_candidates(&current, &mut completed, true);

        assert!(candidates.is_empty());
        assert_eq!(completed, issue_set(&["COE-2"]));
    }

    #[test]
    fn auto_capture_result_waits_for_post_capture_steps_before_completing() {
        let mut completed = issue_set(&["COE-1"]);
        let candidates = vec!["COE-2".to_string()];
        let result = Ok(super::super::memory::AutoMemoryReport {
            completed_issue_keys: Vec::new(),
            captured_issue_keys: vec!["COE-2".to_string()],
            archived_issue_keys: Vec::new(),
            docs_written: Vec::new(),
            capture_completed: true,
            docs_sync_completed: false,
            archive_completed: true,
            warnings: vec!["docs sync failed after capture".to_string()],
        });

        mark_auto_capture_completed(&mut completed, &candidates, &result);

        assert_eq!(completed, issue_set(&["COE-1"]));
    }

    #[test]
    fn auto_capture_result_marks_full_workflow_complete() {
        let mut completed = issue_set(&["COE-1"]);
        let candidates = vec!["COE-2".to_string()];
        let result = Ok(super::super::memory::AutoMemoryReport {
            completed_issue_keys: vec!["COE-2".to_string()],
            captured_issue_keys: vec!["COE-2".to_string()],
            archived_issue_keys: Vec::new(),
            docs_written: vec![PathBuf::from("docs/runtime.md")],
            capture_completed: true,
            docs_sync_completed: true,
            archive_completed: true,
            warnings: Vec::new(),
        });

        mark_auto_capture_completed(&mut completed, &candidates, &result);

        assert_eq!(completed, issue_set(&["COE-1", "COE-2"]));
    }

    #[test]
    fn auto_capture_result_does_not_mark_default_noop_complete() {
        let mut completed = issue_set(&["COE-1"]);
        let candidates = vec!["COE-2".to_string()];
        let result = Ok(super::super::memory::AutoMemoryReport::default());

        mark_auto_capture_completed(&mut completed, &candidates, &result);

        assert_eq!(completed, issue_set(&["COE-1"]));
    }
}
