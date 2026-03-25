mod backends;
mod config;
mod snapshot;

use std::{collections::VecDeque, path::PathBuf, process::ExitCode, sync::Arc};

use chrono::{DateTime, Utc};
use clap::Args;
use opensymphony_control::{ControlPlaneServer, RecentEventKind, SnapshotStore};
use opensymphony_domain::TimestampMs;
use opensymphony_linear::LinearError;
use opensymphony_openhands::OpenHandsError;
use opensymphony_orchestrator::{Scheduler, SchedulerConfig, SchedulerError};
use opensymphony_workspace::WorkspaceError;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    time::{MissedTickBehavior, interval},
};
use tracing::{info, warn};

use self::{
    backends::{
        RuntimeWorkerBackend, RuntimeWorkspaceBackend, build_runtime_transport,
        build_tracker_backend, build_workspace_manager_config,
    },
    config::resolve_runtime_config,
    snapshot::{current_agent_server_status, map_snapshot, push_recent_event, terminal_state_set},
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
    let workspace_manager = Arc::new(opensymphony_workspace::WorkspaceManager::new(
        build_workspace_manager_config(&runtime.workflow),
    )?);
    let workspace = RuntimeWorkspaceBackend::new(workspace_manager.clone());

    let (transport, mut supervisor) = build_runtime_transport(&runtime).await?;
    let client = opensymphony_openhands::OpenHandsClient::new(transport);
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

pub(super) fn timestamp_to_datetime(value: TimestampMs) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value.as_u64() as i64).unwrap_or_else(Utc::now)
}

pub(super) fn datetime_to_timestamp_ms(value: DateTime<Utc>) -> TimestampMs {
    TimestampMs::new(value.timestamp_millis().max(0) as u64)
}

pub(super) fn now_timestamp() -> TimestampMs {
    TimestampMs::new(Utc::now().timestamp_millis().max(0) as u64)
}
