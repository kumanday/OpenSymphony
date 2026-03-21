use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use opensymphony_openhands::{
    ConversationCreateRequest, EventCache, EventEnvelope, KnownEvent, OpenHandsClient,
    TransportConfig,
};
use opensymphony_testkit::{FakeOpenHandsConfig, FakeOpenHandsServer};

#[tokio::test]
async fn fake_server_supports_create_ready_and_reconcile() {
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

    let ready_event = client
        .wait_for_readiness(conversation.conversation_id, Duration::from_secs(2))
        .await
        .expect("websocket readiness should succeed");
    let event_cache = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("event reconciliation should succeed");

    assert!(matches!(
        KnownEvent::from_envelope(&ready_event),
        KnownEvent::ConversationStateUpdate(_)
    ));
    assert!(event_cache.items().len() >= 4);
    assert!(event_cache
        .items()
        .iter()
        .any(|event| event.kind == "LLMCompletionLogEvent"));
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
        .emit_state_update(conversation.conversation_id, "completed")
        .await
        .expect("second state update should be recorded");

    let cache = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("reconcile should read all pages");

    assert_eq!(cache.items().len(), 3);
}
