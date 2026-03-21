use std::time::Duration;

use axum::{routing::get, Json, Router};
use chrono::{TimeZone, Utc};
use opensymphony_control::{
    ControlPlaneClient, ControlPlaneClientError, ControlPlaneServer, SnapshotStore,
};
use opensymphony_domain::{
    AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot,
    MetricsSnapshot, RecentEvent, RecentEventKind, SnapshotEnvelope, WorkerOutcome,
};
use tokio::net::TcpListener;
use url::Url;

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap()
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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = ControlPlaneServer::new(store.clone());
    let server_task = tokio::spawn(async move { server.serve(listener).await.unwrap() });

    let client = ControlPlaneClient::new(Url::parse(&format!("http://{address}/")).unwrap());
    let current = client.fetch_snapshot().await.unwrap();
    assert_eq!(current.sequence, 1);
    assert_eq!(current.snapshot.issues[0].identifier, "COE-255");

    let mut stream = client.stream_updates().unwrap();
    let initial: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(initial.sequence, 1);

    let expected = store.publish(fixture_snapshot(1)).await;

    let streamed: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert_eq!(streamed.sequence, expected.sequence);
    assert_eq!(
        streamed.snapshot.recent_events[0].summary,
        "published step 1"
    );

    stream.close();
    server_task.abort();
}

#[tokio::test]
async fn fetch_snapshot_times_out_when_snapshot_endpoint_hangs() {
    async fn stalled_snapshot() -> Json<SnapshotEnvelope> {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Json(SnapshotEnvelope {
            sequence: 99,
            published_at: Utc::now(),
            snapshot: fixture_snapshot(99),
        })
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = Router::new().route("/api/v1/snapshot", get(stalled_snapshot));
    let server_task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });

    let client = ControlPlaneClient::with_snapshot_timeout(
        Url::parse(&format!("http://{address}/")).unwrap(),
        Duration::from_millis(50),
    );
    let started = tokio::time::Instant::now();
    let error = client.fetch_snapshot().await.unwrap_err();

    match error {
        ControlPlaneClientError::Request(source) => assert!(source.is_timeout()),
        other => panic!("expected request timeout error, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));

    server_task.abort();
}
