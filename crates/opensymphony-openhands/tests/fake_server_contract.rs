use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use opensymphony_openhands::{
    ConversationCreateRequest, EventCache, EventEnvelope, KnownEvent, OpenHandsClient,
    RuntimeStreamConfig, TerminalExecutionStatus, TransportConfig,
};
use opensymphony_testkit::{FakeOpenHandsConfig, FakeOpenHandsServer};

#[tokio::test]
async fn fake_server_runtime_stream_attaches_reconciles_and_detects_terminal_state() {
    let server = FakeOpenHandsServer::start_with_config(FakeOpenHandsConfig {
        search_page_size: 2,
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
    assert!(stream
        .event_cache()
        .items()
        .iter()
        .any(|event| event.kind == "LLMCompletionLogEvent"));
    assert_eq!(
        stream.state_mirror().terminal_status(),
        Some(TerminalExecutionStatus::Finished)
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

    assert_eq!(first.id, running.id);
    assert_eq!(second.id, queued.id);
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
    assert!(stream
        .event_cache()
        .items()
        .iter()
        .any(|event| event.id == finished.id));
}
