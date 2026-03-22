#![cfg(unix)]

use std::{path::PathBuf, process::Command};

use chrono::{TimeZone, Utc};
use opensymphony_control::{ControlPlaneServer, SnapshotStore};
use opensymphony_domain::{
    AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot,
    MetricsSnapshot, RecentEvent, RecentEventKind, WorkerOutcome,
};
use tokio::net::TcpListener;

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("fixture timestamp should be valid")
        + chrono::Duration::seconds(step as i64);
    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: DaemonState::Ready,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony".to_owned(),
            status_line: "ready".to_owned(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3000".to_owned(),
            conversation_count: 1,
            status_line: "healthy".to_owned(),
        },
        metrics: MetricsSnapshot {
            running_issues: 1,
            retry_queue_depth: 0,
            total_tokens: 1024,
            total_cost_micros: 50_000,
        },
        issues: vec![IssueSnapshot {
            identifier: "COE-271".to_owned(),
            title: "FrankenTUI operator client".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Running,
            last_outcome: WorkerOutcome::Running,
            last_event_at: now,
            conversation_id_suffix: "conv-0".to_owned(),
            workspace_path_suffix: "workspace-0".to_owned(),
            retry_count: 0,
            blocked: false,
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-271".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: format!("published step {step}"),
        }],
    }
}

fn opensymphony_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_opensymphony"))
}

fn scripted_tui_command(url: &str, exit_after_ms: u64) -> Command {
    let mut command = Command::new("script");
    command
        .arg("-q")
        .arg("/dev/null")
        .arg(opensymphony_binary())
        .arg("tui")
        .arg("--url")
        .arg(url)
        .arg("--exit-after-ms")
        .arg(exit_after_ms.to_string());
    command
}

#[test]
fn scripted_tui_exits_non_zero_when_control_plane_never_becomes_live() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("probe listener should bind");
    let address = listener
        .local_addr()
        .expect("probe listener should expose its address");
    drop(listener);

    let output = scripted_tui_command(&format!("http://{address}/"), 900)
        .output()
        .expect("scripted tui should run");

    assert!(
        !output.status.success(),
        "expected scripted TUI attach to fail without a live control plane; stdout={}; stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[tokio::test]
async fn scripted_tui_exits_zero_after_healthy_attach() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose an address");
    let server = ControlPlaneServer::new(store);
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("control-plane server should run");
    });

    let output = tokio::task::spawn_blocking(move || {
        scripted_tui_command(&format!("http://{address}/"), 900).output()
    })
    .await
    .expect("scripted tui task should join")
    .expect("scripted tui should run");

    assert!(
        output.status.success(),
        "expected scripted TUI attach to succeed after the live stream comes up; stdout={}; stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    server_task.abort();
}
