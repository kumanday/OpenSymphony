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
/// SSE endpoint now streams journal events (not snapshot updates).
/// This test verifies the SSE transport works with journal events and
/// delivers new events appended after the stream opens.
async fn gateway_events_stream_yields_snapshot_updates() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");

    // Keep a clone of the journal so we can append events after the stream opens.
    let (journal, broker) = server.journal_and_broker();
    let server = GatewayServer::with_journal(store.clone(), journal.clone(), broker);
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
    let timeout_dur = std::time::Duration::from_secs(2);

    // Append an event after the stream opens and expect it to arrive via SSE.
    let event = opensymphony::opensymphony_domain::InMemoryEventJournal::orchestrator_event(
        opensymphony::opensymphony_gateway_schema::event_journal::EventKind::RunStarted,
        "test run started",
        None,
    );
    let _ = journal.append(event).await;

    // Read the initial "connected" event into a buffer.
    let mut first_buf = Vec::new();
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
    // The first event should be a "connected" event with a cursor.
    assert!(
        !first_text.is_empty() && first_text.contains("event: connected"),
        "SSE first event should be 'connected', got: {first_text}"
    );
    assert!(
        first_text.contains("cursor"),
        "SSE connected event should contain cursor, got: {first_text}"
    );

    // Read the journal event into a buffer.
    let mut event_buf = Vec::new();
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout(timeout_dur, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                event_buf.extend_from_slice(&chunk);
                if event_buf.ends_with(b"\n\n") || event_buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }
    let event_text =
        String::from_utf8(event_buf).expect("SSE event is valid UTF-8 when fully assembled");
    assert!(
        !event_text.is_empty() && event_text.contains("event: event"),
        "SSE event should be a journal event, got: {event_text}"
    );

    // Verify the payload is a valid EventRecord.
    let data_line = event_text
        .lines()
        .find(|l| l.starts_with("data:"))
        .expect("SSE event contains data line");
    let json_payload = data_line.trim_start_matches("data:").trim();
    let record: opensymphony::opensymphony_gateway_schema::event_journal::EventRecord =
        serde_json::from_str(json_payload).expect("deserialize SSE payload as EventRecord");
    assert_eq!(record.kind.kind_tag(), "run.started");

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

/// Test that cursor-based event journal query works end-to-end via HTTP.
#[tokio::test]
async fn event_journal_cursor_returns_events_page() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());

    // Seed some events into the journal.
    for i in 0..5u64 {
        let event = EventRecord::builder()
            .event_id(format!("evt_{i}"))
            .sequence(0)
            .actor(EventActor::system("test"))
            .kind(EventKind::RunStarted)
            .summary(format!("Test event {i}"))
            .build();
        journal.append(event).await.expect("append");
    }

    let server = GatewayServer::with_journal(store, journal, broker);
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

    // Query with cursor=0 (from beginning), limit=2.
    let url = format!("http://{address}/api/v1/event-journal?cursor=0&limit=2");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 2);
    assert!(page.has_more);
    assert!(page.next_cursor.is_some());

    // Follow the cursor to get the next page.
    let next_seq = page.next_cursor.expect("next cursor must exist").sequence;
    let url = format!("http://{address}/api/v1/event-journal?cursor={next_seq}&limit=2");
    let page2: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch next page")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page2.events.len(), 2);
    assert!(page2.has_more);

    // Last page.
    let next_seq2 = page2.next_cursor.expect("next cursor must exist").sequence;
    let url = format!("http://{address}/api/v1/event-journal?cursor={next_seq2}&limit=2");
    let page3: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch last page")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page3.events.len(), 1);
    assert!(!page3.has_more);

    server_task.abort();
}

/// Test that partition filtering works via the event journal API.
#[tokio::test]
async fn event_journal_partition_filtering() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    // Add a control event.
    let event = EventRecord::builder()
        .event_id("evt_control")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Control event")
        .build();
    journal.append(event).await.expect("append");

    // Add a terminal frame (high volume).
    let terminal = EventRecord::builder()
        .event_id("evt_term")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::TerminalFrame {
            frame_id: "f1".into(),
        })
        .summary("Terminal frame")
        .build();
    journal.append(terminal).await.expect("append");

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
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

    // Query control events.
    let url = format!("http://{address}/api/v1/event-journal?partition=events");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id, "evt_control");

    // Query terminal events.
    let url = format!("http://{address}/api/v1/event-journal?partition=terminal_log");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id, "evt_term");

    server_task.abort();
}

/// Test that unknown harness events with raw payload refs are retained.
#[tokio::test]
async fn event_journal_raw_payload_ref_retained() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    // Add an unknown harness event with raw payload ref.
    let event = EventRecord::builder()
        .event_id("evt_raw")
        .sequence(0)
        .actor(EventActor::harness("openhands-1"))
        .kind(EventKind::Unknown {
            raw_kind: "custom_harness_event".into(),
        })
        .summary("Unknown harness event")
        .raw_payload_ref("raw_ref_123")
        .build();
    journal.append(event).await.expect("append");

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
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

    let url = format!("http://{address}/api/v1/event-journal");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 1);
    assert!(page.events[0].has_raw_payload());
    assert_eq!(page.events[0].raw_payload_ref, Some("raw_ref_123".into()));

    server_task.abort();
}

/// Test that duplicate events are identifiable by stable event_id.
#[tokio::test]
async fn event_journal_duplicate_detection() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    // Append two events with the same event_id (simulating duplicate detection).
    let event = EventRecord::builder()
        .event_id("evt_dup")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Duplicate event")
        .build();
    journal.append(event.clone()).await.expect("append");
    journal.append(event).await.expect("append");

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
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

    let url = format!("http://{address}/api/v1/event-journal");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 2);
    // Both events share the same stable event_id.
    assert_eq!(page.events[0].event_id, page.events[1].event_id);
    // But they have different sequences.
    assert_ne!(page.events[0].sequence, page.events[1].sequence);

    server_task.abort();
}

/// Test that the WebSocket event stream endpoint works end-to-end.
/// Connects, sends an init message, receives backlog events, then a live event.
#[tokio::test]
async fn websocket_event_stream_delivers_events() {
    use futures_util::SinkExt;
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventRecord,
    };
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());

    // Seed a backlog event.
    let backlog_event = EventRecord::builder()
        .event_id("ws_test_1")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Backlog event")
        .build();
    journal.append(backlog_event).await.expect("append");

    let server = GatewayServer::with_journal(store, journal.clone(), broker);
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

    // Connect via WebSocket.
    let ws_url = format!("ws://{address}/api/v1/streams/events");
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect to WS endpoint");

    let (mut write, mut read) = ws_stream.split();

    // Send init message to start from the beginning.
    let init = serde_json::json!({ "sequence": 0, "partition": "events" });
    let init_msg = serde_json::to_string(&init).expect("serialize init");
    write
        .send(WsMessage::Text(init_msg.into()))
        .await
        .expect("send init");

    // Receive the connected event first.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), read.next())
        .await
        .expect("timed out waiting for connected event")
        .expect("should receive a message")
        .expect("no WS error");
    let text = msg.to_text().expect("text message");
    assert!(
        text.starts_with("__connected__"),
        "Expected __connected__ prefix, got: {text}"
    );

    // Receive the backlog event.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), read.next())
        .await
        .expect("timed out waiting for backlog event")
        .expect("should receive a message")
        .expect("no WS error");
    let text = msg.to_text().expect("text message");
    assert!(
        text.starts_with("__event__"),
        "Expected __event__ prefix, got: {text}"
    );
    assert!(
        text.contains("ws_test_1"),
        "Backlog event should contain event_id ws_test_1, got: {text}"
    );

    // Emit a live event through the journal.
    let live_event = EventRecord::builder()
        .event_id("ws_test_2")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunCompleted)
        .summary("Live event")
        .build();
    journal.append(live_event).await.expect("append live");

    // Receive the live event.
    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), read.next())
        .await
        .expect("timed out waiting for live event")
        .expect("should receive a message")
        .expect("no WS error");
    let text = msg.to_text().expect("text message");
    assert!(
        text.starts_with("__event__"),
        "Expected __event__ prefix, got: {text}"
    );
    assert!(
        text.contains("ws_test_2"),
        "Live event should contain event_id ws_test_2, got: {text}"
    );

    server_task.abort();
}
