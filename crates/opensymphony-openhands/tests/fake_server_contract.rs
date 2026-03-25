use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use opensymphony_openhands::{
    ConversationCreateRequest, DoctorProbeConfig, EventCache, EventEnvelope, KnownEvent,
    OpenHandsClient, OpenHandsError, RuntimeStreamConfig, TerminalExecutionStatus, TransportConfig,
};
use opensymphony_testkit::{
    FakeEventStreamBuilder, FakeOpenHandsConfig, FakeOpenHandsServer, FakeSearchScript,
    FakeSocketScript,
};

#[tokio::test]
async fn fake_server_runtime_stream_attaches_reconciles_and_detects_terminal_state() {
    let server = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        search_page_size: 2,
        ..FakeOpenHandsConfig::default()
    })
    .await
    .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );

    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    client
        .send_message(
            conversation.conversation_id,
            &opensymphony_openhands::SendMessageRequest::user_text("hello"),
        )
        .await
        .expect("message send should succeed");
    client
        .run_conversation(conversation.conversation_id)
        .await
        .expect("run should succeed");

    let stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    assert!(matches!(
        KnownEvent::from_envelope(stream.ready_event()),
        KnownEvent::ConversationStateUpdate(_)
    ));
    assert!(stream.event_cache().items().len() >= 4);
    assert!(
        stream
            .event_cache()
            .items()
            .iter()
            .any(|event| event.kind == "LLMCompletionLogEvent")
    );
    assert_eq!(
        stream.state_mirror().terminal_status(),
        Some(TerminalExecutionStatus::Finished)
    );
}

#[tokio::test]
async fn runtime_stream_replays_initial_snapshot_events_on_attach() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    let running = EventEnvelope::new(
        "evt-running",
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({
            "execution_status": "running",
            "state_delta": {
                "execution_status": "running",
            },
        }),
    );
    let completion = EventEnvelope::new(
        "evt-log",
        running.timestamp + ChronoDuration::seconds(1),
        "llm",
        "LLMCompletionLogEvent",
        serde_json::json!({
            "model": "fake-model",
            "tokens": 42,
        }),
    );

    server
        .insert_event(conversation.conversation_id, running)
        .await
        .expect("running event should be persisted");
    server
        .insert_event(conversation.conversation_id, completion)
        .await
        .expect("completion event should be persisted");

    let expected_ids = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("initial search should succeed")
        .items()
        .iter()
        .map(|event| event.id.clone())
        .collect::<Vec<_>>();

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    let mut replayed_ids = Vec::new();
    for _ in 0..expected_ids.len() {
        let event = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
            .await
            .expect("replayed attach event should arrive")
            .expect("stream read should succeed")
            .expect("replayed attach event should exist");
        replayed_ids.push(event.id);
    }

    assert_eq!(replayed_ids, expected_ids);
    let no_extra = tokio::time::timeout(Duration::from_millis(200), stream.next_event()).await;
    assert!(
        no_extra.is_err(),
        "attach replay should not fabricate extra events after the initial REST snapshot is drained"
    );
}

#[tokio::test]
async fn event_cache_orders_by_timestamp_and_deduplicates_ids() {
    let mut cache = EventCache::new();
    let newer = EventEnvelope::new(
        "evt-2",
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({}),
    );
    let older = EventEnvelope::new(
        "evt-1",
        Utc::now() - ChronoDuration::seconds(10),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({}),
    );
    let duplicate = older.clone();

    assert!(cache.insert(newer));
    assert!(cache.insert(older.clone()));
    assert!(!cache.insert(duplicate));
    assert_eq!(cache.items()[0].id, older.id);
    assert_eq!(cache.items().len(), 2);
}

#[tokio::test]
async fn reconciliation_walks_multiple_pages() {
    let server = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        search_page_size: 1,
        ..FakeOpenHandsConfig::default()
    })
    .await
    .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    server
        .emit_state_update(conversation.conversation_id, "running")
        .await
        .expect("state update should be recorded");
    server
        .emit_state_update(conversation.conversation_id, "finished")
        .await
        .expect("second state update should be recorded");

    let cache = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("reconcile should read all pages");

    assert_eq!(cache.items().len(), 3);
}

#[tokio::test]
async fn runtime_stream_keeps_latest_state_when_out_of_order_events_arrive() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");
    let initial = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("initial persisted snapshot event should arrive")
        .expect("stream read should succeed")
        .expect("initial persisted snapshot event should exist");
    assert_eq!(initial.id, "evt-1");
    let running = EventEnvelope::new(
        "evt-running",
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({
            "execution_status": "running",
            "state_delta": {
                "execution_status": "running",
            },
        }),
    );
    let queued = EventEnvelope::new(
        "evt-queued",
        running.timestamp - ChronoDuration::seconds(5),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({
            "execution_status": "queued",
            "state_delta": {
                "execution_status": "queued",
            },
        }),
    );

    server
        .insert_event(conversation.conversation_id, running.clone())
        .await
        .expect("running event should be recorded");
    server
        .insert_event(conversation.conversation_id, queued.clone())
        .await
        .expect("queued event should be recorded");

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("first stream event should arrive")
        .expect("stream read should succeed")
        .expect("first stream event should exist");
    let second = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("second stream event should arrive")
        .expect("stream read should succeed")
        .expect("second stream event should exist");

    assert_eq!(first.id, queued.id);
    assert_eq!(second.id, running.id);
    let ordered_ids = stream
        .event_cache()
        .items()
        .iter()
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ordered_ids.first().copied(), Some("evt-queued"));
    assert_eq!(ordered_ids.last().copied(), Some("evt-running"));
    assert_eq!(stream.state_mirror().execution_status(), Some("running"));
}

#[tokio::test]
async fn create_conversation_reuses_existing_requested_id_without_losing_history() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    let persisted = EventEnvelope::new(
        "evt-persisted",
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({
            "execution_status": "running",
            "state_delta": {
                "execution_status": "running",
            },
        }),
    );
    server
        .insert_event(conversation.conversation_id, persisted.clone())
        .await
        .expect("persisted history should be stored");

    let recreated = client
        .create_conversation(&request)
        .await
        .expect("recreate by requested ID should succeed");
    let events = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("history should still be searchable");

    assert_eq!(recreated.conversation_id, conversation.conversation_id);
    assert!(
        events.items().iter().any(|event| event.id == persisted.id),
        "same-ID create should not drop persisted history"
    );
}

#[tokio::test]
async fn delete_conversation_then_recreate_requested_id_resets_history_and_updates_config() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe_with_config(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        DoctorProbeConfig {
            model: Some("openai/gpt-5.4".to_string()),
            api_key: Some("old-secret".to_string()),
            ..DoctorProbeConfig::default()
        },
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    server
        .insert_event(
            conversation.conversation_id,
            EventEnvelope::new(
                "evt-persisted",
                Utc::now(),
                "runtime",
                "ConversationStateUpdateEvent",
                serde_json::json!({
                    "execution_status": "running",
                    "state_delta": {
                        "execution_status": "running",
                    },
                }),
            ),
        )
        .await
        .expect("persisted history should be stored");

    client
        .delete_conversation(conversation.conversation_id)
        .await
        .expect("conversation delete should succeed");

    let recreated = client
        .create_conversation(&ConversationCreateRequest {
            agent: opensymphony_openhands::AgentConfig {
                llm: opensymphony_openhands::LlmConfig {
                    api_key: Some("new-secret".to_string()),
                    ..request.agent.clone().llm
                },
                ..request.agent.clone()
            },
            ..request.clone()
        })
        .await
        .expect("recreate by requested ID should succeed");
    let events = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("history should still be searchable after recreation");

    assert_eq!(recreated.conversation_id, conversation.conversation_id);
    assert_eq!(recreated.agent.llm.api_key.as_deref(), Some("new-secret"));
    assert!(
        events
            .items()
            .iter()
            .all(|event| event.id != "evt-persisted"),
        "delete + recreate should drop the old persisted history"
    );
}

#[tokio::test]
async fn run_conversation_returns_conflict_while_execution_is_already_active() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    server
        .emit_state_update(conversation.conversation_id, "running")
        .await
        .expect("conversation should become active");

    let error = client
        .run_conversation(conversation.conversation_id)
        .await
        .expect_err("active conversation should reject a second run");

    assert!(matches!(
        error,
        OpenHandsError::HttpStatus {
            status_code: 409,
            ..
        }
    ));
}

#[tokio::test]
async fn attach_runtime_stream_replays_initial_persisted_snapshot() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    server
        .emit_state_update(conversation.conversation_id, "running")
        .await
        .expect("running state should be recorded");
    server
        .emit_state_update(conversation.conversation_id, "finished")
        .await
        .expect("finished state should be recorded");

    let initial_snapshot = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("initial search should succeed");
    let expected_ids = initial_snapshot
        .items()
        .iter()
        .map(|event| event.id.clone())
        .collect::<Vec<_>>();

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    let mut replayed_ids = Vec::new();
    for _ in 0..expected_ids.len() {
        let event = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
            .await
            .expect("snapshot replay should not stall")
            .expect("stream read should succeed")
            .expect("replayed snapshot event should exist");
        replayed_ids.push(event.id);
    }

    assert_eq!(replayed_ids, expected_ids);
}

#[tokio::test]
async fn scripted_fake_server_replays_initial_snapshot_when_post_ready_reconcile_is_empty() {
    let server = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        initial_execution_status: "running",
        ..FakeOpenHandsConfig::default()
    })
    .await
    .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    let fixtures = FakeEventStreamBuilder::new(Utc::now());
    let running = fixtures.state_update_at("evt-running", 0, "running");
    let log = fixtures.llm_completion_at("evt-log", 1_000, "fake-model", 42);
    let ready = fixtures.state_update_at("evt-ready", 2_000, "running");

    server
        .script_search_responses(
            conversation.conversation_id,
            FakeSearchScript::new()
                .response(vec![running.clone(), log.clone()])
                .response(vec![]),
        )
        .await
        .expect("search script should be configured");
    server
        .script_socket_connections(
            conversation.conversation_id,
            vec![FakeSocketScript::new().event(ready)],
        )
        .await
        .expect("socket script should be configured");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    let first = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("first replayed snapshot event should arrive")
        .expect("stream read should succeed")
        .expect("first replayed snapshot event should exist");
    let second = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("second replayed snapshot event should arrive")
        .expect("stream read should succeed")
        .expect("second replayed snapshot event should exist");

    assert_eq!(
        [first.id.as_str(), second.id.as_str()],
        ["evt-running", "evt-log"]
    );
    let no_extra = tokio::time::timeout(Duration::from_millis(200), stream.next_event()).await;
    assert!(
        no_extra.is_err(),
        "stream should wait for future websocket activity once the scripted initial snapshot replay is drained"
    );
}

#[tokio::test]
async fn scripted_fake_server_drains_buffered_socket_events_before_later_attach_backlog() {
    let server = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        initial_execution_status: "running",
        ..FakeOpenHandsConfig::default()
    })
    .await
    .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    let fixtures = FakeEventStreamBuilder::new(Utc::now());
    let running = fixtures.state_update_at("evt-running", 0, "running");
    let queued_live = fixtures.state_update_at("evt-queued-live", 1_000, "queued");
    let log = fixtures.llm_completion_at("evt-log", 2_000, "fake-model", 42);
    let ready = fixtures.state_update_at("evt-ready", 0, "running");

    server
        .script_search_responses(
            conversation.conversation_id,
            FakeSearchScript::new()
                .response(vec![running.clone(), log.clone()])
                .response(vec![]),
        )
        .await
        .expect("search script should be configured");
    server
        .script_socket_connections(
            conversation.conversation_id,
            vec![FakeSocketScript::new().event(ready).event(queued_live)],
        )
        .await
        .expect("socket script should be configured");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    let first = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("first replayed event should arrive")
        .expect("stream read should succeed")
        .expect("first replayed event should exist");
    let second = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("buffered socket event should arrive")
        .expect("stream read should succeed")
        .expect("buffered socket event should exist");
    let third = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("later replayed event should arrive")
        .expect("stream read should succeed")
        .expect("later replayed event should exist");

    assert_eq!(
        [first.id.as_str(), second.id.as_str(), third.id.as_str()],
        ["evt-running", "evt-queued-live", "evt-log"],
        "buffered live socket frames should be merged before a later attach-backlog item is yielded"
    );
}

#[tokio::test]
async fn runtime_stream_reconnects_and_recovers_missed_events() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                reconnect_initial_backoff: Duration::from_millis(25),
                reconnect_max_backoff: Duration::from_millis(100),
                max_reconnect_attempts: 4,
            },
        )
        .await
        .expect("runtime stream attach should succeed");
    let initial = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("initial persisted snapshot event should arrive")
        .expect("stream read should succeed")
        .expect("initial persisted snapshot event should exist");
    assert_eq!(initial.id, "evt-1");
    let completion_log = EventEnvelope::new(
        "evt-log",
        Utc::now(),
        "llm",
        "LLMCompletionLogEvent",
        serde_json::json!({
            "model": "fake-model",
            "tokens": 7,
        }),
    );
    let finished = EventEnvelope::new(
        "evt-finished",
        completion_log.timestamp + ChronoDuration::seconds(1),
        "runtime",
        "ConversationStateUpdateEvent",
        serde_json::json!({
            "execution_status": "finished",
            "state_delta": {
                "execution_status": "finished",
            },
        }),
    );

    server
        .drop_websocket_connections(conversation.conversation_id)
        .await
        .expect("existing websocket should drop");
    server
        .insert_event(conversation.conversation_id, completion_log.clone())
        .await
        .expect("missed log event should be persisted for reconcile");
    server
        .insert_event(conversation.conversation_id, finished.clone())
        .await
        .expect("missed event should be persisted for reconcile");

    let first = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("first recovered event should arrive after reconnect")
        .expect("stream read should succeed")
        .expect("first recovered event should exist");
    let second = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("second recovered event should arrive after reconnect")
        .expect("stream read should succeed")
        .expect("second recovered event should exist");

    assert_eq!(first.id, completion_log.id);
    assert_eq!(second.id, finished.id);

    let recovered_ids = stream
        .event_cache()
        .items()
        .iter()
        .filter(|event| event.id == completion_log.id || event.id == finished.id)
        .map(|event| event.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(recovered_ids, vec!["evt-log", "evt-finished"]);

    assert_eq!(
        stream.state_mirror().terminal_status(),
        Some(TerminalExecutionStatus::Finished)
    );
    assert!(
        stream
            .event_cache()
            .items()
            .iter()
            .any(|event| event.id == finished.id)
    );
}

#[tokio::test]
async fn scripted_fake_server_yields_buffered_event_before_reconnect_exhaustion() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    let fixtures = FakeEventStreamBuilder::new(Utc::now());
    let ready = fixtures.state_update_at("evt-ready", 0, "idle");
    let runtime = fixtures.state_update_at("evt-runtime", 1_000, "running");

    server
        .script_search_responses(
            conversation.conversation_id,
            FakeSearchScript::new().response(vec![]).response(vec![]),
        )
        .await
        .expect("search script should be configured");
    server
        .script_socket_connections(
            conversation.conversation_id,
            vec![
                FakeSocketScript::new().event(ready).event(runtime).close(),
                FakeSocketScript::new().close(),
            ],
        )
        .await
        .expect("socket script should be configured");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                reconnect_initial_backoff: Duration::from_millis(25),
                reconnect_max_backoff: Duration::from_millis(25),
                max_reconnect_attempts: 1,
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("buffered event should arrive")
        .expect("stream read should succeed")
        .expect("buffered event should exist");
    assert_eq!(event.id, "evt-runtime");

    let error = stream
        .next_event()
        .await
        .expect_err("reconnect exhaustion should surface after buffered delivery");
    match error {
        OpenHandsError::ReconnectExhausted { attempts, .. } => assert_eq!(attempts, 1),
        other => panic!("expected reconnect exhaustion after buffered delivery, got {other:?}"),
    }
}

#[tokio::test]
async fn scripted_fake_server_close_clears_pending_reconnect_and_replay_state() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    let fixtures = FakeEventStreamBuilder::new(Utc::now());
    let ready = fixtures.state_update_at("evt-ready", 0, "idle");
    let runtime = fixtures.state_update_at("evt-runtime", 1_000, "running");

    server
        .script_search_responses(
            conversation.conversation_id,
            FakeSearchScript::new().response(vec![]).response(vec![]),
        )
        .await
        .expect("search script should be configured");
    server
        .script_socket_connections(
            conversation.conversation_id,
            vec![
                FakeSocketScript::new().event(ready).event(runtime).close(),
                FakeSocketScript::new().close(),
            ],
        )
        .await
        .expect("socket script should be configured");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                reconnect_initial_backoff: Duration::from_millis(25),
                reconnect_max_backoff: Duration::from_millis(25),
                max_reconnect_attempts: 1,
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let event = tokio::time::timeout(Duration::from_secs(2), stream.next_event())
        .await
        .expect("buffered event should arrive before close")
        .expect("stream read should succeed")
        .expect("buffered event should exist");
    assert_eq!(event.id, "evt-runtime");

    stream.close().await.expect("close should succeed");

    let closed = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("closed stream should return promptly")
        .expect("polling a closed stream should not fail");
    assert!(
        closed.is_none(),
        "close should clear any queued replay or reconnect work so the stream stays closed"
    );
}
