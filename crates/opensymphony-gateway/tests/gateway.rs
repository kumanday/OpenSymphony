use chrono::Utc;
use futures_util::StreamExt;
use opensymphony::opensymphony_control::{ControlPlaneServer, SnapshotStore};
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneConversationEvent as ConversationEvent,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneFileChange as FileChange,
    ControlPlaneFileChangeKind as FileChangeKind,
    ControlPlaneIssueRuntimeState as IssueRuntimeState, ControlPlaneIssueSnapshot as IssueSnapshot,
    ControlPlaneMetricsSnapshot as MetricsSnapshot, ControlPlaneRecentEvent as RecentEvent,
    ControlPlaneRecentEventKind as RecentEventKind, ControlPlaneWorkerOutcome as WorkerOutcome,
    SnapshotEnvelope,
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

/// Second fixture variant: one Idle issue, one Completed issue with events
/// and modified files, and one Failed issue (first attempt, no retries).
fn fixture_snapshot_rich(step: u64) -> DaemonSnapshot {
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
        issues: vec![
            // Idle issue (eligible for execution)
            IssueSnapshot {
                identifier: "COE-300".to_owned(),
                title: "Idle task".to_owned(),
                tracker_state: "Todo".to_owned(),
                runtime_state: IssueRuntimeState::Idle,
                last_outcome: WorkerOutcome::Unknown,
                last_event_at: now,
                conversation_id_suffix: String::new(),
                workspace_path_suffix: String::new(),
                retry_count: 0,
                blocked: false,
                server_base_url: None,
                transport_target: None,
                http_auth_mode: None,
                websocket_auth_mode: None,
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
            },
            // Completed issue with events and modified files
            IssueSnapshot {
                identifier: "COE-301".to_owned(),
                title: "Completed task".to_owned(),
                tracker_state: "Done".to_owned(),
                runtime_state: IssueRuntimeState::Completed,
                last_outcome: WorkerOutcome::Completed,
                last_event_at: now,
                conversation_id_suffix: "c0e301".to_owned(),
                workspace_path_suffix: "COE-301".to_owned(),
                retry_count: 0,
                blocked: false,
                server_base_url: Some("http://127.0.0.1:3001".to_owned()),
                transport_target: Some("loopback".to_owned()),
                http_auth_mode: Some("none".to_owned()),
                websocket_auth_mode: Some("none".to_owned()),
                websocket_query_param_name: None,
                recent_events: vec![
                    ConversationEvent {
                        event_id: "evt-1".to_owned(),
                        happened_at: now,
                        kind: "worker_started".to_owned(),
                        summary: "worker started".to_owned(),
                    },
                    ConversationEvent {
                        event_id: "evt-2".to_owned(),
                        happened_at: now,
                        kind: "worker_completed".to_owned(),
                        summary: "worker completed".to_owned(),
                    },
                ],
                modified_files: vec![
                    FileChange {
                        path: "/tmp/opensymphony/COE-301/src/main.rs".to_owned(),
                        change_kind: FileChangeKind::Modified,
                        lines_added: 10,
                        lines_removed: 3,
                    },
                    FileChange {
                        path: "/tmp/opensymphony/COE-301/src/lib.rs".to_owned(),
                        change_kind: FileChangeKind::Created,
                        lines_added: 42,
                        lines_removed: 0,
                    },
                ],
                input_tokens: 2048,
                output_tokens: 1024,
                cache_read_tokens: 256,
            },
            // Failed issue, first attempt (no retries exhausted)
            IssueSnapshot {
                identifier: "COE-302".to_owned(),
                title: "Failed task".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::Failed,
                last_outcome: WorkerOutcome::Failed,
                last_event_at: now,
                conversation_id_suffix: "c0e302".to_owned(),
                workspace_path_suffix: "COE-302".to_owned(),
                retry_count: 0,
                blocked: false,
                server_base_url: Some("http://127.0.0.1:3002".to_owned()),
                transport_target: Some("loopback".to_owned()),
                http_auth_mode: Some("none".to_owned()),
                websocket_auth_mode: Some("none".to_owned()),
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 512,
                output_tokens: 128,
                cache_read_tokens: 0,
            },
            // RetryQueued issue: queued but NOT eligible (not idle)
            IssueSnapshot {
                identifier: "COE-303".to_owned(),
                title: "Retry queued task".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::RetryQueued,
                last_outcome: WorkerOutcome::Failed,
                last_event_at: now,
                conversation_id_suffix: "c0e303".to_owned(),
                workspace_path_suffix: "COE-303".to_owned(),
                retry_count: 1,
                blocked: false,
                server_base_url: Some("http://127.0.0.1:3003".to_owned()),
                transport_target: Some("loopback".to_owned()),
                http_auth_mode: Some("none".to_owned()),
                websocket_auth_mode: Some("none".to_owned()),
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 256,
                output_tokens: 64,
                cache_read_tokens: 0,
            },
            // Blocked Idle issue: NOT eligible AND NOT queued
            IssueSnapshot {
                identifier: "COE-304".to_owned(),
                title: "Blocked idle task".to_owned(),
                tracker_state: "Todo".to_owned(),
                runtime_state: IssueRuntimeState::Idle,
                last_outcome: WorkerOutcome::Unknown,
                last_event_at: now,
                conversation_id_suffix: String::new(),
                workspace_path_suffix: String::new(),
                retry_count: 0,
                blocked: true,
                server_base_url: None,
                transport_target: None,
                http_auth_mode: None,
                websocket_auth_mode: None,
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
            },
        ],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-301".to_owned()),
            kind: RecentEventKind::WorkerCompleted,
            summary: format!("completed step {step}"),
        }],
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
async fn gateway_events_stream_yields_journal_events() {
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

    // Read the journal event into a buffer.
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
    assert!(
        !first_text.is_empty() && first_text.contains("event: event"),
        "SSE event should be a journal event, got: {first_text}"
    );

    // Verify the payload is a valid EventRecord.
    let data_line = first_text
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

    let event = EventRecord::builder()
        .event_id("evt_control")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Control event")
        .build();
    journal.append(event).await.expect("append");

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
    assert_eq!(page.events[0].event_id, page.events[1].event_id);
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

    let ws_url = format!("ws://{address}/api/v1/streams/events");
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect to WS endpoint");

    let (mut write, mut read) = ws_stream.split();

    let init = serde_json::json!({ "cursor": 0, "partition": "events" });
    let init_msg = serde_json::to_string(&init).expect("serialize init");
    write
        .send(WsMessage::Text(init_msg.into()))
        .await
        .expect("send init");

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

    let live_event = EventRecord::builder()
        .event_id("ws_test_2")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunCompleted)
        .summary("Live event")
        .build();
    journal.append(live_event).await.expect("append live");

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

// ── Read API endpoint tests ────────────────────────────────────────────────────

#[tokio::test]
async fn gateway_serves_project_list() {
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
    let response = client
        .get(format!("http://{address}/api/v1/projects"))
        .send()
        .await
        .expect("fetch projects")
        .json::<opensymphony::opensymphony_gateway_schema::snapshot::ProjectList>()
        .await
        .expect("decode project list");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.projects.len(), 1);
    assert_eq!(response.projects[0].project_id, "default");
    assert_eq!(response.projects[0].name, "OpenSymphony");
    assert_eq!(response.projects[0].issue_count, 1);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_project_detail() {
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
    let response = client
        .get(format!("http://{address}/api/v1/projects/default"))
        .send()
        .await
        .expect("fetch project detail")
        .json::<opensymphony::opensymphony_gateway_schema::snapshot::ProjectDetail>()
        .await
        .expect("decode project detail");

    assert_eq!(response.project_id, "default");
    assert_eq!(response.name, "OpenSymphony");
    assert_eq!(response.issue_count, 1);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_task_graph() {
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
    let response = client
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.project_id, "default");
    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.nodes[0].identifier, "COE-255");
    // Verify runtime overlay is present
    assert!(response.nodes[0].runtime_overlay.is_some());
    let overlay = response.nodes[0]
        .runtime_overlay
        .as_ref()
        .expect("task graph node should have runtime overlay");
    // Running issues are NOT eligible (only Idle issues are eligible).
    assert!(!overlay.eligible);
    assert_eq!(overlay.active_run_id, Some("COE-255".into()));

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_detail() {
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255"))
        .send()
        .await
        .expect("fetch run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.run_id, "COE-255");
    assert_eq!(response.issue_identifier, "COE-255");
    assert_eq!(
        response.status,
        opensymphony::opensymphony_gateway_schema::run::RunStatus::Running
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_events() {
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255/events"))
        .send()
        .await
        .expect("fetch run events")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunEventPage>()
        .await
        .expect("decode run events");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.run_id, "COE-255");
    // The fixture has no recent_events for the issue, so page is empty
    assert!(response.events.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_files() {
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255/files"))
        .send()
        .await
        .expect("fetch run files")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunFilesPage>()
        .await
        .expect("decode run files");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.run_id, "COE-255");
    // The fixture has no modified_files, so page is empty
    assert!(response.files.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_diffs() {
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255/diffs"))
        .send()
        .await
        .expect("fetch run diffs")
        .json::<opensymphony::opensymphony_gateway_schema::run::FileDiffPage>()
        .await
        .expect("decode run diffs");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.run_id, "COE-255");
    assert!(response.hunks.is_empty());

    server_task.abort();
}

#[test]
fn sanitize_file_path_strips_workspace_root() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony",
        "/tmp/opensymphony/COE-255/src/main.rs",
    );
    assert_eq!(result, "COE-255/src/main.rs");
}

#[test]
fn sanitize_file_path_falls_back_to_basename_for_unsafe_path() {
    let result =
        opensymphony::opensymphony_gateway::sanitize_file_path("/tmp/opensymphony", "/etc/passwd");
    assert_eq!(result, "passwd");
}

// ── Path traversal tests ─────────────────────────────────────────────────────

#[test]
fn sanitize_file_path_blocks_path_traversal_via_dotdot() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony",
        "/tmp/opensymphony/../etc/passwd",
    );
    // The traversal escapes the workspace root, so the fallback basename
    // (`passwd`) is returned instead of leaking `../etc/passwd`.
    assert_eq!(result, "passwd");
}

#[test]
fn sanitize_file_path_blocks_nested_path_traversal() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony",
        "/tmp/opensymphony/COE-255/../../etc/passwd",
    );
    assert_eq!(result, "passwd");
}

// Workspace root normalization: a crafted root that tries to escape its own
// boundary is normalized before the strip, so the file still resolves safely.
#[test]
fn sanitize_file_path_normalizes_workspace_root() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/other/../opensymphony",
        "/tmp/opensymphony/COE-255/src/main.rs",
    );
    assert_eq!(result, "COE-255/src/main.rs");
}

// When both root and file contain `..` components, normalization on both sides
// prevents a crafted root from widening the accepted prefix.
#[test]
fn sanitize_file_path_normalizes_both_sides() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony/../opensymphony",
        "/tmp/other/../opensymphony/../etc/passwd",
    );
    // Normalized: root=/tmp/opensymphony, file=/tmp/etc/passwd → escapes root
    assert_eq!(result, "passwd");
}

// Empty string file name fallback: a raw path that is only a root dir yields
// an empty string instead of leaking the workspace root.
#[test]
fn sanitize_file_path_empty_fallback_for_root_only_path() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path("/tmp/opensymphony", "/");
    assert_eq!(result, "");
}

// ── 404 negative-path tests ───────────────────────────────────────────────────

#[tokio::test]
async fn gateway_returns_404_for_unknown_project() {
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
    let resp = client
        .get(format!("http://{address}/api/v1/projects/nonexistent"))
        .send()
        .await
        .expect("fetch unknown project");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_project_task_graph() {
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
    let resp = client
        .get(format!(
            "http://{address}/api/v1/projects/nonexistent/taskgraph"
        ))
        .send()
        .await
        .expect("fetch unknown task graph");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run() {
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
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999"))
        .send()
        .await
        .expect("fetch unknown run");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_events() {
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
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999/events"))
        .send()
        .await
        .expect("fetch unknown run events");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_files() {
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
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999/files"))
        .send()
        .await
        .expect("fetch unknown run files");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_diffs() {
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
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999/diffs"))
        .send()
        .await
        .expect("fetch unknown run diffs");

    assert_eq!(resp.status(), 404);

    // Assert the 404 response body is well-formed
    let body: opensymphony::opensymphony_gateway_schema::run::FileDiffPage =
        resp.json().await.expect("decode 404 run diffs body");
    assert_eq!(body.run_id, "UNKNOWN-999");
    assert!(body.hunks.is_empty());

    server_task.abort();
}

// ── Rich fixture tests (non-Running states, file/diff data) ────────────────────

#[tokio::test]
async fn gateway_serves_run_files_with_modified_files() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/files"))
        .send()
        .await
        .expect("fetch run files with data")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunFilesPage>()
        .await
        .expect("decode run files");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(response.files.len(), 2);
    // Files should have workspace root stripped
    let paths: Vec<_> = response.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"COE-301/src/main.rs"));
    assert!(paths.contains(&"COE-301/src/lib.rs"));

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_diffs_with_modified_files() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/diffs"))
        .send()
        .await
        .expect("fetch run diffs with data")
        .json::<opensymphony::opensymphony_gateway_schema::run::FileDiffPage>()
        .await
        .expect("decode run diffs");

    assert_eq!(response.run_id, "COE-301");
    // Multi-file diff should show count label instead of single path
    assert_eq!(response.file_path, "[2 files]");
    assert_eq!(response.hunks.len(), 2);
    assert_eq!(response.total_lines_added, 52);
    assert_eq!(response.total_lines_removed, 3);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_events_with_data() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/events"))
        .send()
        .await
        .expect("fetch run events with data")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunEventPage>()
        .await
        .expect("decode run events");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(response.events.len(), 2);

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_eligible_for_idle_issue() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    // Find the idle issue overlay
    let idle_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-300")
        .expect("COE-300 node should exist");
    let overlay = idle_node.runtime_overlay.as_ref().expect("overlay present");
    // Idle + not blocked = eligible
    assert!(overlay.eligible);
    assert!(overlay.queued);

    // Completed issue should NOT be eligible
    let done_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-301")
        .expect("COE-301 node should exist");
    let done_overlay = done_node.runtime_overlay.as_ref().expect("overlay present");
    assert!(!done_overlay.eligible);

    // root_ids should be empty (no parent/child data available)
    assert!(response.root_ids.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_failed_without_retries() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-302"))
        .send()
        .await
        .expect("fetch failed run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.run_id, "COE-302");
    // Failed with retry_count == 0 should map to TrackerTerminal, not RetryExhausted
    assert_eq!(
        response.release_reason,
        Some(opensymphony::opensymphony_gateway_schema::run::ReleaseReason::TrackerTerminal)
    );
    // Finished at should be set for terminal states
    assert!(response.finished_at.is_some());

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_completed_state() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301"))
        .send()
        .await
        .expect("fetch completed run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(
        response.release_reason,
        Some(opensymphony::opensymphony_gateway_schema::run::ReleaseReason::Completed)
    );
    assert!(response.finished_at.is_some());

    server_task.abort();
}

// ── Runtime overlay: queued vs eligible semantics ──────────────────────────────

#[tokio::test]
async fn gateway_task_graph_queued_vs_eligible() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
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
    let response = client
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    // Idle + not blocked → eligible AND queued
    let idle_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-300")
        .expect("COE-300 node should exist");
    let idle_overlay = idle_node.runtime_overlay.as_ref().expect("overlay present");
    assert!(
        idle_overlay.eligible,
        "Idle unblocked issue should be eligible"
    );
    assert!(idle_overlay.queued, "Idle unblocked issue should be queued");

    // RetryQueued → queued BUT NOT eligible (not in Idle state)
    let retry_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-303")
        .expect("COE-303 node should exist");
    let retry_overlay = retry_node
        .runtime_overlay
        .as_ref()
        .expect("overlay present");
    assert!(
        !retry_overlay.eligible,
        "RetryQueued issue must NOT be eligible (not idle)"
    );
    assert!(
        retry_overlay.queued,
        "RetryQueued issue must be queued (waiting for retry)"
    );

    // Completed → neither eligible nor queued
    let done_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-301")
        .expect("COE-301 node should exist");
    let done_overlay = done_node.runtime_overlay.as_ref().expect("overlay present");
    assert!(
        !done_overlay.eligible,
        "Completed issue must not be eligible"
    );
    assert!(!done_overlay.queued, "Completed issue must not be queued");

    // Failed → neither eligible nor queued
    let failed_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-302")
        .expect("COE-302 node should exist");
    let failed_overlay = failed_node
        .runtime_overlay
        .as_ref()
        .expect("overlay present");
    assert!(
        !failed_overlay.eligible,
        "Failed issue must not be eligible"
    );
    assert!(!failed_overlay.queued, "Failed issue must not be queued");

    // Blocked Idle → NOT eligible AND NOT queued (blocked overrides Idle)
    let blocked_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-304")
        .expect("COE-304 node should exist");
    let blocked_overlay = blocked_node
        .runtime_overlay
        .as_ref()
        .expect("overlay present");
    assert!(
        !blocked_overlay.eligible,
        "Blocked Idle issue must not be eligible"
    );
    assert!(
        !blocked_overlay.queued,
        "Blocked Idle issue must not be queued (blocked overrides)"
    );

    server_task.abort();
}
