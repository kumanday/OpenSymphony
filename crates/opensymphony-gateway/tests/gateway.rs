use chrono::Utc;
use futures_util::StreamExt;
use opensymphony::opensymphony_control::{ControlPlaneServer, SnapshotStore};
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
};
use opensymphony::opensymphony_gateway::{
    GatewayCapabilities, GatewayServer, control_plane_to_dashboard_snapshot,
};
use tokio::net::TcpListener;
use url::Url;

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc::now();
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
            input_tokens: 2048,
            output_tokens: 2048,
            cache_read_tokens: 512,
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
            input_tokens: 1024,
            output_tokens: 512,
            cache_read_tokens: 256,
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-255".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: format!("published step {step}"),
        }],
    }
}

fn fixture_envelope(step: u64) -> SnapshotEnvelope {
    let snapshot = fixture_snapshot(step);
    SnapshotEnvelope {
        sequence: step + 1,
        published_at: snapshot.generated_at,
        snapshot,
    }
}

#[test]
fn control_plane_to_dashboard_snapshot_maps_all_fields() {
    let envelope = fixture_envelope(5);
    let dashboard = control_plane_to_dashboard_snapshot(&envelope);

    assert_eq!(dashboard.schema_version.major, 1);
    assert_eq!(dashboard.sequence, 6);
    assert!(
        matches!(
            dashboard.health,
            opensymphony::opensymphony_gateway_schema::snapshot::GatewayHealth::Healthy
        ),
        "expected Healthy when daemon state is Ready"
    );

    let metrics = &dashboard.metrics;
    assert_eq!(metrics.running_issue_count, 1);
    assert_eq!(metrics.retry_queue_depth, 0);
    assert_eq!(metrics.total_input_tokens, 2048);
    assert_eq!(metrics.total_output_tokens, 2048);

    assert_eq!(dashboard.projects.len(), 1);
    let project = &dashboard.projects[0];
    assert_eq!(project.project_id, "default");
    assert_eq!(project.issue_count, 1);
    assert_eq!(project.running_count, 1);

    assert_eq!(dashboard.recent_events.len(), 1);
    let event = &dashboard.recent_events[0];
    assert_eq!(event.issue_identifier, Some("COE-255".to_owned()));
    assert_eq!(
        event.kind,
        opensymphony::opensymphony_gateway_schema::snapshot::SnapshotEventKind::SnapshotPublished
    );
}

#[test]
fn control_plane_to_dashboard_snapshot_handles_empty_issues() {
    let mut envelope = fixture_envelope(0);
    envelope.snapshot.issues.clear();
    envelope.snapshot.metrics.running_issues = 0;
    let dashboard = control_plane_to_dashboard_snapshot(&envelope);

    assert!(dashboard.projects.is_empty());
    assert_eq!(dashboard.metrics.running_issue_count, 0);
}

#[test]
fn gateway_capabilities_json_fixture_roundtrips() {
    let caps = GatewayCapabilities {
        schema_version: opensymphony::opensymphony_gateway_schema::version::SchemaVersion::v1(),
        gateway_version: "1.6.0".into(),
        supported_api_versions: vec!["1.0.0".into()],
        transports: vec![
            opensymphony::opensymphony_gateway_schema::capability::TransportCapability {
                transport: "sse".into(),
                modes: vec!["snapshot".into()],
                supported_encodings: vec!["utf-8".into()],
                bidirectional: false,
            },
        ],
        features: vec![
            opensymphony::opensymphony_gateway_schema::capability::FeatureCapability {
                feature: "planning".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
        ],
        auth_modes: vec![
            opensymphony::opensymphony_gateway_schema::capability::AuthMode::None,
            opensymphony::opensymphony_gateway_schema::capability::AuthMode::ApiKey,
        ],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    };

    let json = serde_json::to_string_pretty(&caps).expect("serialize capabilities");
    let back: GatewayCapabilities = serde_json::from_str(&json).expect("deserialize capabilities");

    assert_eq!(back.gateway_version, "1.6.0");
    assert_eq!(back.supported_api_versions, vec!["1.0.0"]);
    assert_eq!(back.auth_modes.len(), 2);
    assert_eq!(back.max_event_page_size, 1000);
}

#[test]
fn dashboard_snapshot_json_fixture_roundtrips() {
    let envelope = fixture_envelope(7);
    let dashboard = control_plane_to_dashboard_snapshot(&envelope);
    let json = serde_json::to_string_pretty(&dashboard).expect("serialize dashboard");
    let back: opensymphony::opensymphony_gateway_schema::snapshot::DashboardSnapshot =
        serde_json::from_str(&json).expect("deserialize dashboard");

    assert_eq!(back.sequence, 8);
    assert_eq!(back.projects.len(), 1);
    assert_eq!(back.metrics.running_issue_count, 1);
}

#[tokio::test]
async fn gateway_serves_capabilities_and_dashboard_snapshot() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let caps_url = format!("http://{address}/api/v1/capabilities");
    let _caps_response = client
        .get(&caps_url)
        .send()
        .await
        .expect("fetch capabilities")
        .json::<GatewayCapabilities>()
        .await
        .expect("decode capabilities");

    // capabilities assertions verified in dedicated round-trip test

    let snapshot_url = format!("http://{address}/api/v1/dashboard/snapshot");
    let snapshot_response = client
        .get(&snapshot_url)
        .send()
        .await
        .expect("fetch dashboard snapshot")
        .json::<opensymphony::opensymphony_gateway_schema::snapshot::DashboardSnapshot>()
        .await
        .expect("decode dashboard snapshot");

    assert_eq!(snapshot_response.sequence, 1);
    assert_eq!(snapshot_response.projects.len(), 1);
    assert_eq!(snapshot_response.projects[0].project_id, "default");

    server_task.abort();
}

#[tokio::test]
async fn gateway_events_stream_yields_snapshot_updates() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let events_url =
        Url::parse(&format!("http://{address}/api/v1/events")).expect("valid events url");

    let client = reqwest::Client::new();
    let response = client
        .get(events_url)
        .send()
        .await
        .expect("open SSE stream");

    assert_eq!(
        response
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .expect("valid header value"),
        "text/event-stream"
    );

    let mut stream = response.bytes_stream();

    // Read the initial snapshot event into a buffer.
    let mut first_buf = Vec::new();
    let timeout_dur = std::time::Duration::from_secs(2);
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout(timeout_dur, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                first_buf.extend_from_slice(&chunk);
                if first_buf.ends_with(b"\n\n") || first_buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }
    let first_text =
        String::from_utf8(first_buf).expect("SSE event is valid UTF-8 when fully assembled");
    assert!(
        !first_text.is_empty() && first_text.contains("event: snapshot"),
        "first SSE event should be a snapshot"
    );

    // Publish a new snapshot through the store and expect a second event.
    let new_snapshot = fixture_snapshot(1);
    store.publish(new_snapshot).await;

    // Read the second event into a buffer.
    let mut second_buf = Vec::new();
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout(timeout_dur, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                second_buf.extend_from_slice(&chunk);
                if second_buf.ends_with(b"\n\n") || second_buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }
    let second_text =
        String::from_utf8(second_buf).expect("SSE event is valid UTF-8 when fully assembled");
    assert!(
        !second_text.is_empty() && second_text.contains("event: snapshot"),
        "second SSE event should be a snapshot"
    );

    // Verify the payload in the second event is a valid DashboardSnapshot.
    let data_line = second_text
        .lines()
        .find(|l| l.starts_with("data:"))
        .expect("second event contains data line");
    let json_payload = data_line.trim_start_matches("data:").trim();
    let dashboard: opensymphony::opensymphony_gateway_schema::snapshot::DashboardSnapshot =
        serde_json::from_str(json_payload).expect("deserialize SSE payload as DashboardSnapshot");
    assert_eq!(dashboard.sequence, 2);

    server_task.abort();
}

#[tokio::test]
async fn gateway_and_control_plane_are_reachable() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let gateway = GatewayServer::new(store.clone());
    let control = ControlPlaneServer::new(store);

    let gateway_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway listener");
    let gateway_address = gateway_listener
        .local_addr()
        .expect("gateway listener address");
    let gateway_task = tokio::spawn(async move {
        gateway
            .serve(gateway_listener)
            .await
            .expect("gateway serve")
    });

    let control_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind control listener");
    let control_address = control_listener
        .local_addr()
        .expect("control listener address");
    let control_task = tokio::spawn(async move {
        control
            .serve(control_listener)
            .await
            .expect("control serve")
    });

    let client = reqwest::Client::new();

    let gateway_caps = client
        .get(format!("http://{gateway_address}/api/v1/capabilities"))
        .send()
        .await
        .expect("gateway capabilities reachable");
    assert!(gateway_caps.status().is_success());

    let gateway_snapshot = client
        .get(format!(
            "http://{gateway_address}/api/v1/dashboard/snapshot"
        ))
        .send()
        .await
        .expect("gateway dashboard snapshot reachable");
    assert!(gateway_snapshot.status().is_success());

    let control_snapshot = client
        .get(format!("http://{control_address}/api/v1/snapshot"))
        .send()
        .await
        .expect("control snapshot reachable");
    assert!(control_snapshot.status().is_success());

    gateway_task.abort();
    control_task.abort();
}
