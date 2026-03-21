use std::{net::SocketAddr, num::NonZeroU64, time::Duration};

use chrono::{Duration as ChronoDuration, Utc};
use clap::{Parser, Subcommand};
use opensymphony_control::{ControlPlaneServer, SnapshotStore};
use opensymphony_domain::{
    AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot,
    MetricsSnapshot, RecentEvent, RecentEventKind, WorkerOutcome,
};
use opensymphony_tui::TuiError;
use thiserror::Error;
use tracing::info;
use tracing_subscriber::EnvFilter;
use url::Url;

#[derive(Debug, Parser)]
#[command(
    name = "opensymphony",
    about = "OpenSymphony local control-plane and FrankenTUI demo"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Daemon {
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: SocketAddr,
        #[arg(long, default_value = "1200")]
        sample_interval_ms: NonZeroU64,
    },
    Tui {
        #[arg(long, default_value = "http://127.0.0.1:3000/")]
        url: Url,
        #[arg(long)]
        exit_after_ms: Option<u64>,
    },
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    init_tracing();
    let cli = Cli::parse();

    match cli.command {
        Command::Daemon {
            bind,
            sample_interval_ms,
        } => run_daemon(bind, sample_interval_ms).await,
        Command::Tui { url, exit_after_ms } => run_tui(url, exit_after_ms).await,
    }
}

async fn run_daemon(bind: SocketAddr, sample_interval_ms: NonZeroU64) -> Result<(), CliError> {
    let store = SnapshotStore::new(sample_snapshot(0));
    spawn_demo_updates(
        store.clone(),
        Duration::from_millis(sample_interval_ms.get()),
    );
    let server = ControlPlaneServer::new(store);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(%bind, "control plane listening");

    let server_task = tokio::spawn(async move { server.serve(listener).await });
    tokio::select! {
        result = server_task => {
            result.map_err(CliError::Join)??;
            Ok(())
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down control plane");
            Ok(())
        }
    }
}

async fn run_tui(url: Url, exit_after_ms: Option<u64>) -> Result<(), CliError> {
    let exit_after = exit_after_ms.map(Duration::from_millis);
    tokio::task::spawn_blocking(move || opensymphony_tui::run_operator(url, exit_after))
        .await
        .map_err(CliError::Join)?
        .map_err(CliError::Tui)
}

fn spawn_demo_updates(store: SnapshotStore, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        let mut step = 1_u64;
        loop {
            ticker.tick().await;
            let snapshot = sample_snapshot(step);
            store.publish(snapshot).await;
            step += 1;
        }
    });
}

fn sample_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc::now();
    let runtime = match step % 4 {
        0 => IssueRuntimeState::Running,
        1 => IssueRuntimeState::Running,
        2 => IssueRuntimeState::RetryQueued,
        _ => IssueRuntimeState::Completed,
    };
    let outcome = match step % 4 {
        0 | 1 => WorkerOutcome::Running,
        2 => WorkerOutcome::Continued,
        _ => WorkerOutcome::Completed,
    };
    let daemon_state = if step == 0 {
        DaemonState::Starting
    } else {
        DaemonState::Ready
    };

    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: daemon_state,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony/workspaces".to_owned(),
            status_line: "scheduler heartbeat healthy".to_owned(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3002".to_owned(),
            conversation_count: 3,
            status_line: "local agent-server healthy".to_owned(),
        },
        metrics: MetricsSnapshot {
            running_issues: if matches!(runtime, IssueRuntimeState::Completed) {
                0
            } else {
                1
            },
            retry_queue_depth: if matches!(runtime, IssueRuntimeState::RetryQueued) {
                1
            } else {
                0
            },
            total_tokens: 8_000 + (step * 240),
            total_cost_micros: 340_000 + (step * 9_500),
        },
        issues: vec![
            IssueSnapshot {
                identifier: "COE-255".to_owned(),
                title: "Observability and FrankenTUI".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: runtime,
                last_outcome: outcome,
                last_event_at: now,
                conversation_id_suffix: "255-live".to_owned(),
                workspace_path_suffix: "COE-255".to_owned(),
                retry_count: if matches!(runtime, IssueRuntimeState::RetryQueued) {
                    1
                } else {
                    0
                },
                blocked: false,
            },
            IssueSnapshot {
                identifier: "OSYM-401".to_owned(),
                title: "Control-plane API and snapshot store".to_owned(),
                tracker_state: "Done".to_owned(),
                runtime_state: IssueRuntimeState::Completed,
                last_outcome: WorkerOutcome::Completed,
                last_event_at: now - ChronoDuration::seconds(45),
                conversation_id_suffix: "401-done".to_owned(),
                workspace_path_suffix: "OSYM-401".to_owned(),
                retry_count: 0,
                blocked: false,
            },
            IssueSnapshot {
                identifier: "OSYM-402".to_owned(),
                title: "FrankenTUI operator client".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: if step.is_multiple_of(2) {
                    IssueRuntimeState::Running
                } else {
                    IssueRuntimeState::Idle
                },
                last_outcome: if step.is_multiple_of(2) {
                    WorkerOutcome::Running
                } else {
                    WorkerOutcome::Unknown
                },
                last_event_at: now - ChronoDuration::seconds(10),
                conversation_id_suffix: "402-ui".to_owned(),
                workspace_path_suffix: "OSYM-402".to_owned(),
                retry_count: 0,
                blocked: false,
            },
        ],
        recent_events: vec![
            RecentEvent {
                happened_at: now,
                issue_identifier: Some("COE-255".to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: format!("snapshot sequence advanced to step {step}"),
            },
            RecentEvent {
                happened_at: now - ChronoDuration::seconds(5),
                issue_identifier: Some("COE-255".to_owned()),
                kind: RecentEventKind::ClientAttached,
                summary: "FrankenTUI watcher connected to the control plane".to_owned(),
            },
            RecentEvent {
                happened_at: now - ChronoDuration::seconds(12),
                issue_identifier: Some("OSYM-402".to_owned()),
                kind: RecentEventKind::WorkerStarted,
                summary: "operator client reducer refreshed after live update".to_owned(),
            },
        ],
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("opensymphony=info,opensymphony_control=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[derive(Debug, Error)]
enum CliError {
    #[error("failed to bind control-plane listener: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("FrankenTUI failed: {0}")]
    Tui(#[from] TuiError),
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::{Parser, error::ErrorKind};

    #[test]
    fn daemon_rejects_zero_sample_interval() {
        let error = Cli::try_parse_from(["opensymphony", "daemon", "--sample-interval-ms", "0"])
            .expect_err("zero sample interval should be rejected");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn daemon_accepts_positive_sample_interval() {
        let cli = Cli::try_parse_from(["opensymphony", "daemon", "--sample-interval-ms", "250"])
            .expect("CLI fixture should parse");

        match cli.command {
            Command::Daemon {
                sample_interval_ms, ..
            } => {
                assert_eq!(sample_interval_ms.get(), 250);
            }
            Command::Tui { .. } => panic!("expected daemon command"),
        }
    }
}
