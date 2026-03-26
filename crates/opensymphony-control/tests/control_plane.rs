use std::time::Duration;

use axum::Router;
use chrono::{TimeZone, Utc};
use opensymphony_control::{ControlPlaneClient, ControlPlaneServer, SnapshotStore};
use opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
};
use tokio::net::TcpListener;
use url::Url;

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("valid fixed test timestamp")
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
            conversation_count: 2,
            status_line: "healthy".to_owned(),
        },
        metrics: MetricsSnapshot {
            running_issues: 1,
            retry_queue_depth: 0,
            total_tokens: 4096 + step,
            total_cost_micros: 120_000,
        },
        issues: vec![IssueSnapshot {
            identifier: "COE-255".to_owned(),
            title: "Observability and FrankenTUI".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Running,
            last_outcome: WorkerOutcome::Running,
            last_event_at: now,
            conversation_id_suffix: "c0e255".to_owned(),
            workspace_path_suffix: "COE-255".to_owned(),
            retry_count: 0,
            blocked: false,
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            transport_target: Some("loopback".to_owned()),
            http_auth_mode: Some("none".to_owned()),
            websocket_auth_mode: Some("none".to_owned()),
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-255".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: format!("published step {step}"),
        }],
    }
}

#[tokio::test]
async fn serves_snapshot_and_streams_updates() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server = ControlPlaneServer::new(store.clone());
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test control-plane server should serve")
    });

    let client = ControlPlaneClient::new(
        Url::parse(&format!("http://{address}/")).expect("valid root control-plane base url"),
    );
    let current = client
        .fetch_snapshot()
        .await
        .expect("fetch current snapshot from test server");
    assert_eq!(current.sequence, 1);
    assert_eq!(current.snapshot.issues[0].identifier, "COE-255");

    let mut stream = client
        .stream_updates()
        .expect("open control-plane event stream");
    let initial: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timed out waiting for initial snapshot")
        .expect("stream should yield an initial snapshot item")
        .expect("initial snapshot should decode");
    assert_eq!(initial.sequence, 1);

    let expected = store.publish(fixture_snapshot(1)).await;

    let streamed: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timed out waiting for streamed snapshot")
        .expect("stream should yield an updated snapshot item")
        .expect("updated snapshot should decode");

    assert_eq!(streamed.sequence, expected.sequence);
    assert_eq!(
        streamed.snapshot.recent_events[0].summary,
        "published step 1"
    );

    stream.close();
    server_task.abort();
}

#[tokio::test]
async fn client_handles_path_prefixed_base_url_without_trailing_slash() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind prefixed test listener");
    let address = listener
        .local_addr()
        .expect("prefixed test listener address");
    let app = Router::new().nest(
        "/opensymphony",
        ControlPlaneServer::new(store.clone()).router(),
    );
    let server_task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("prefixed test server should serve")
    });

    let client = ControlPlaneClient::new(
        Url::parse(&format!("http://{address}/opensymphony"))
            .expect("valid prefixed control-plane base url"),
    );
    let current = client
        .fetch_snapshot()
        .await
        .expect("fetch current snapshot from prefixed test server");
    assert_eq!(current.sequence, 1);
    assert_eq!(current.snapshot.issues[0].identifier, "COE-255");

    let mut stream = client
        .stream_updates()
        .expect("open prefixed control-plane event stream");
    let initial: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timed out waiting for prefixed initial snapshot")
        .expect("prefixed stream should yield an initial snapshot item")
        .expect("prefixed initial snapshot should decode");
    assert_eq!(initial.sequence, 1);

    stream.close();
    server_task.abort();
}
