use std::time::Duration;

use axum::{Json, Router, routing::get};
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
    fixture_snapshot_with_state(step, DaemonState::Ready)
}

fn fixture_snapshot_with_state(step: u64, daemon_state: DaemonState) -> DaemonSnapshot {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("fixture timestamp should be valid")
        + chrono::Duration::seconds(step as i64);
    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: daemon_state,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony".to_owned(),
            status_line: daemon_status_line(daemon_state).to_owned(),
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

fn daemon_status_line(state: DaemonState) -> &'static str {
    match state {
        DaemonState::Starting => "starting",
        DaemonState::Ready => "ready",
        DaemonState::Degraded => "degraded",
        DaemonState::Stopped => "stopped",
    }
}

#[tokio::test]
async fn serves_snapshot_and_streams_updates() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose an address");
    let server = ControlPlaneServer::new(store.clone());
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("control-plane server should run");
    });

    let client = ControlPlaneClient::new(
        Url::parse(&format!("http://{address}/")).expect("test base URL should parse"),
    );
    let current = client
        .fetch_snapshot()
        .await
        .expect("client should fetch the current snapshot");
    assert_eq!(current.sequence, 1);
    assert_eq!(current.snapshot.issues[0].identifier, "COE-255");

    let mut stream = client
        .stream_updates()
        .expect("client should open the update stream");
    let initial: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("initial stream event should arrive")
        .expect("stream should not end before the initial snapshot")
        .expect("initial stream event should decode");
    assert_eq!(initial.sequence, 1);

    let expected = store.publish(fixture_snapshot(1)).await;

    let streamed: SnapshotEnvelope = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("streamed update should arrive")
        .expect("stream should stay open for the published update")
        .expect("streamed update should decode");

    assert_eq!(streamed.sequence, expected.sequence);
    assert_eq!(
        streamed.snapshot.recent_events[0].summary,
        "published step 1"
    );

    stream.close();
    server_task.abort();
}

#[tokio::test]
async fn health_endpoint_reflects_daemon_state() {
    let cases = [
        (DaemonState::Starting, "starting"),
        (DaemonState::Ready, "ok"),
        (DaemonState::Degraded, "degraded"),
        (DaemonState::Stopped, "stopped"),
    ];

    for (daemon_state, expected_status) in cases {
        let store = SnapshotStore::new(fixture_snapshot_with_state(0, daemon_state));
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

        let response = reqwest::get(format!("http://{address}/healthz"))
            .await
            .expect("health endpoint should respond")
            .error_for_status()
            .expect("health endpoint should return success")
            .json::<opensymphony_control::HealthResponse>()
            .await
            .expect("health response should decode");

        assert_eq!(response.status, expected_status);

        server_task.abort();
    }
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

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose an address");
    let server = Router::new().route("/api/v1/snapshot", get(stalled_snapshot));
    let server_task = tokio::spawn(async move {
        axum::serve(listener, server)
            .await
            .expect("stalled test server should run");
    });

    let client = ControlPlaneClient::with_snapshot_timeout(
        Url::parse(&format!("http://{address}/")).expect("test base URL should parse"),
        Duration::from_millis(50),
    );
    let started = tokio::time::Instant::now();
    let error = client
        .fetch_snapshot()
        .await
        .expect_err("hung snapshot endpoint should time out");

    match error {
        ControlPlaneClientError::Request(source) => assert!(source.is_timeout()),
        other => panic!("expected request timeout error, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));

    server_task.abort();
}

#[tokio::test]
async fn stream_updates_times_out_when_event_stream_goes_idle() {
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

    let client = ControlPlaneClient::with_timeouts(
        Url::parse(&format!("http://{address}/")).expect("test base URL should parse"),
        Duration::from_secs(5),
        Duration::from_secs(5),
        Duration::from_millis(50),
    );
    let mut stream = client
        .stream_updates()
        .expect("client should open the update stream");

    let initial = stream
        .next()
        .await
        .expect("stream should yield the bootstrap snapshot")
        .expect("bootstrap snapshot should decode");
    assert_eq!(initial.sequence, 1);

    let started = tokio::time::Instant::now();
    let error = stream
        .next()
        .await
        .expect("idle stream should surface a timeout error")
        .expect_err("idle stream should not decode as a snapshot");

    match error {
        ControlPlaneClientError::Stream(source) => {
            assert!(matches!(
                source.as_ref(),
                reqwest_eventsource::Error::Transport(transport) if transport.is_timeout()
            ));
        }
        other => panic!("expected stream timeout error, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));

    stream.close();
    server_task.abort();
}

#[tokio::test]
async fn stream_updates_times_out_when_event_stream_never_establishes() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose an address");
    let server_task = tokio::spawn(async move {
        let (_connection, _) = listener
            .accept()
            .await
            .expect("client should connect to the hanging listener");
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let client = ControlPlaneClient::with_timeouts(
        Url::parse(&format!("http://{address}/")).expect("test base URL should parse"),
        Duration::from_secs(5),
        Duration::from_millis(50),
        Duration::from_secs(5),
    );
    let mut stream = client
        .stream_updates()
        .expect("client should open the update stream");

    let started = tokio::time::Instant::now();
    let error = stream
        .next()
        .await
        .expect("stream establishment should surface a timeout error")
        .expect_err("never-established stream should not decode as a snapshot");

    match error {
        ControlPlaneClientError::StreamConnectTimeout(timeout) => {
            assert_eq!(timeout, Duration::from_millis(50));
        }
        other => panic!("expected stream connect timeout error, got {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(1));

    stream.close();
    server_task.abort();
}
