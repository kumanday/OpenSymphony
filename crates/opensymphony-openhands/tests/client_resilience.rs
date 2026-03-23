use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use opensymphony_openhands::{
    AuthConfig, Conversation, ConversationCreateRequest, EventEnvelope, OpenHandsClient,
    OpenHandsError, RuntimeStreamConfig, SearchConversationEventsResponse, SendMessageRequest,
    TerminalExecutionStatus, TransportConfig,
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use uuid::Uuid;

fn text_message(value: String) -> Message {
    Message::Text(value.into())
}

#[tokio::test]
async fn wait_for_readiness_ignores_non_state_frames_before_ready_event() {
    let server = TestServer::start(readiness_router()).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));

    client
        .wait_for_readiness(Uuid::new_v4(), Duration::from_secs(2))
        .await
        .expect("readiness should tolerate ping and unrelated events");
}

#[tokio::test]
async fn wait_for_readiness_accepts_forward_compatible_state_update_kind() {
    let server = TestServer::start(forward_compatible_readiness_router()).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));

    let ready = client
        .wait_for_readiness(Uuid::new_v4(), Duration::from_millis(100))
        .await
        .expect("readiness should key off the state-update event kind");

    assert_eq!(ready.kind, "ConversationStateUpdateEvent");
}

#[tokio::test]
async fn query_param_api_key_authenticates_rest_and_websocket_operations() {
    let server = TestServer::start(auth_router(AuthExpectations {
        rest: ExpectedAuth::QueryParam {
            name: "session_api_key".to_string(),
            value: "secret-token".to_string(),
        },
        websocket: ExpectedAuth::QueryParam {
            name: "session_api_key".to_string(),
            value: "secret-token".to_string(),
        },
    }))
    .await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()).with_auth(
        AuthConfig::query_param_api_key("session_api_key", "secret-token"),
    ));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );

    let conversation = client
        .create_conversation(&request)
        .await
        .expect("create should carry auth");
    client
        .get_conversation(conversation.conversation_id)
        .await
        .expect("get should carry auth");
    client
        .send_message(
            conversation.conversation_id,
            &SendMessageRequest::user_text("hello"),
        )
        .await
        .expect("send_message should carry auth");
    client
        .run_conversation(conversation.conversation_id)
        .await
        .expect("run should carry auth");
    client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("search should carry auth");
    client
        .wait_for_readiness(conversation.conversation_id, Duration::from_secs(2))
        .await
        .expect("websocket readiness should carry query auth");
}

#[tokio::test]
async fn header_api_key_with_websocket_query_fallback_authenticates_operations() {
    let server = TestServer::start(auth_router(AuthExpectations {
        rest: ExpectedAuth::Header {
            name: "x-session-api-key".to_string(),
            value: "secret-token".to_string(),
        },
        websocket: ExpectedAuth::QueryParam {
            name: "session_api_key".to_string(),
            value: "secret-token".to_string(),
        },
    }))
    .await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()).with_auth(
        AuthConfig::header_api_key_with_websocket_query_fallback(
            "x-session-api-key",
            "session_api_key",
            "secret-token",
        ),
    ));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );

    let conversation = client
        .create_conversation(&request)
        .await
        .expect("create should carry header auth");
    client
        .wait_for_readiness(conversation.conversation_id, Duration::from_secs(2))
        .await
        .expect("websocket readiness should use query fallback");
    client
        .send_message(
            conversation.conversation_id,
            &SendMessageRequest::user_text("hello"),
        )
        .await
        .expect("send_message should carry header auth");
    client
        .run_conversation(conversation.conversation_id)
        .await
        .expect("run should carry header auth");
}

#[tokio::test]
async fn path_prefixed_base_urls_preserve_rest_and_websocket_authentication() {
    let server = TestServer::start(Router::new().nest(
        "/runtime",
        auth_router(AuthExpectations {
            rest: ExpectedAuth::Header {
                name: "x-session-api-key".to_string(),
                value: "secret-token".to_string(),
            },
            websocket: ExpectedAuth::QueryParam {
                name: "session_api_key".to_string(),
                value: "secret-token".to_string(),
            },
        }),
    ))
    .await;
    let client = OpenHandsClient::new(
        TransportConfig::new(format!("{}/runtime", server.base_url())).with_auth(
            AuthConfig::header_api_key_with_websocket_query_fallback(
                "x-session-api-key",
                "session_api_key",
                "secret-token",
            ),
        ),
    );
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );

    let conversation = client
        .create_conversation(&request)
        .await
        .expect("create should preserve the path prefix");
    client
        .wait_for_readiness(conversation.conversation_id, Duration::from_secs(2))
        .await
        .expect("websocket readiness should preserve the path prefix");
    client
        .run_conversation(conversation.conversation_id)
        .await
        .expect("run should preserve the path prefix");
}

#[tokio::test]
async fn auth_failure_maps_to_stable_http_status_error() {
    let server = TestServer::start(auth_router(AuthExpectations {
        rest: ExpectedAuth::QueryParam {
            name: "session_api_key".to_string(),
            value: "secret-token".to_string(),
        },
        websocket: ExpectedAuth::QueryParam {
            name: "session_api_key".to_string(),
            value: "secret-token".to_string(),
        },
    }))
    .await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );

    let error = client
        .create_conversation(&request)
        .await
        .expect_err("missing auth should fail");

    match error {
        OpenHandsError::HttpStatus {
            operation,
            status_code,
            ..
        } => {
            assert_eq!(operation, "create conversation");
            assert_eq!(status_code, 401);
        }
        other => panic!("expected stable HTTP status error, got {other:?}"),
    }
}

#[tokio::test]
async fn malformed_payload_maps_to_stable_protocol_error() {
    let server = TestServer::start(malformed_payload_router()).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        None,
        None,
    );

    let error = client
        .create_conversation(&request)
        .await
        .expect_err("invalid JSON should fail");

    match error {
        OpenHandsError::Protocol { operation, .. } => {
            assert_eq!(operation, "create conversation");
        }
        other => panic!("expected stable protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn run_probe_exercises_message_and_run_endpoints() {
    let state = ProbeState::default();
    let server = TestServer::start(probe_router(state.clone())).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    client
        .run_probe(&request, Duration::from_secs(2))
        .await
        .expect("probe should succeed");

    assert_eq!(*state.send_count.lock().await, 1);
    assert_eq!(*state.run_count.lock().await, 1);
}

#[tokio::test]
async fn run_probe_rejects_failure_only_event_streams() {
    let state = ProbeState::default();
    let server = TestServer::start(failed_probe_router(state.clone())).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    let result = client.run_probe(&request, Duration::from_secs(1)).await;

    assert!(
        result.is_err(),
        "probe should fail when the runtime only emits a ConversationErrorEvent"
    );
}

#[tokio::test]
async fn run_probe_uses_refreshed_terminal_state_after_reconnect_without_new_events() {
    let state = ProbeState::default();
    let server = TestServer::start(reconnect_probe_router(state.clone())).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    let result = client
        .run_probe(&request, Duration::from_secs(2))
        .await
        .expect("probe should succeed from refreshed terminal state after reconnect");

    assert_eq!(result.conversation.execution_status, "finished");
    assert_eq!(
        result.state_mirror.terminal_status(),
        Some(TerminalExecutionStatus::Finished)
    );
    assert_eq!(*state.send_count.lock().await, 1);
    assert_eq!(*state.run_count.lock().await, 1);
}

#[tokio::test]
async fn run_probe_prefers_terminal_rest_refresh_over_reconnect_exhaustion() {
    let state = ProbeState::default();
    let server = TestServer::start(terminal_rest_refresh_probe_router(state)).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    let result = client
        .run_probe(&request, Duration::from_secs(2))
        .await
        .expect(
            "probe should succeed from terminal REST state even if websocket reconnect exhausts",
        );

    assert_eq!(result.conversation.execution_status, "finished");
    assert_eq!(
        result.state_mirror.terminal_status(),
        Some(TerminalExecutionStatus::Finished)
    );
}

#[tokio::test]
async fn run_probe_reuses_terminal_stream_snapshot_when_final_conversation_refresh_fails() {
    let state = ProbeState::default();
    let server = TestServer::start(final_refresh_failure_probe_router(state.clone())).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    let result = client
        .run_probe(&request, Duration::from_secs(2))
        .await
        .expect(
            "probe should succeed from the terminal stream snapshot even if the final conversation GET fails",
        );

    assert_eq!(result.conversation.execution_status, "finished");
    assert_eq!(
        result.state_mirror.terminal_status(),
        Some(TerminalExecutionStatus::Finished)
    );
    assert_eq!(*state.get_count.lock().await, 2);
}

#[tokio::test]
async fn run_probe_rejects_error_events_even_when_finished_state_is_already_mirrored() {
    let state = ProbeState::default();
    let server = TestServer::start(reconnect_error_then_finished_probe_router(state.clone())).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    let result = client.run_probe(&request, Duration::from_secs(2)).await;

    assert!(
        result.is_err(),
        "probe should fail when a queued ConversationErrorEvent precedes the mirrored finished state"
    );
}

#[tokio::test]
async fn wait_for_probe_terminal_state_polls_once_more_before_accepting_finished() {
    let state = ProbeState::default();
    let server = TestServer::start(finished_then_error_probe_router(state.clone())).await;
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/workspace",
        "/tmp/workspace/.opensymphony/openhands",
        Some("fake-model".to_string()),
        Some("fake-key".to_string()),
    );

    let result = client.run_probe(&request, Duration::from_secs(2)).await;

    assert!(
        result.is_err(),
        "probe should fail when a ConversationErrorEvent arrives on the next scheduler turn after the mirrored finished state"
    );
}

#[tokio::test]
async fn runtime_stream_does_not_replay_reconnect_readiness_barriers_without_new_events() {
    let state = ReadinessReplayState::default();
    let server = TestServer::start(readiness_replay_router(state)).await;
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
            opensymphony_openhands::RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                reconnect_initial_backoff: Duration::from_millis(25),
                reconnect_max_backoff: Duration::from_millis(100),
                max_reconnect_attempts: 4,
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    let result = tokio::time::timeout(Duration::from_millis(200), stream.next_event()).await;

    assert!(
        result.is_err(),
        "reconnect readiness barrier should not surface as a replayable event when search adds no new runtime activity"
    );
}

#[tokio::test]
async fn attach_runtime_stream_applies_readiness_snapshot_to_state_mirror() {
    let state = ReadinessMirrorState::default();
    let server = TestServer::start(readiness_mirror_router(state)).await;
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

    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("running"),
        "the ready-state barrier should refresh the mirror even when REST state and reconcile lag behind it"
    );
}

#[tokio::test]
async fn attach_runtime_stream_applies_forward_compatible_readiness_snapshot_to_state_mirror() {
    let state = ReadinessMirrorState::default();
    let server = TestServer::start(forward_compatible_readiness_mirror_router(state)).await;
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

    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("running"),
        "the ready-state barrier should still refresh the mirror when the readiness frame only exposes a forward-compatible state_delta"
    );
}

#[tokio::test]
async fn attach_runtime_stream_applies_newer_reused_conversation_readiness_snapshot_to_state_mirror()
 {
    let state = ReadinessMirrorState::default();
    let server = TestServer::start(reused_conversation_readiness_mirror_router(state)).await;
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

    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("running"),
        "a newer ready barrier should override stale terminal REST state when a reused conversation restarts"
    );
}

#[tokio::test]
async fn attach_runtime_stream_uses_ready_barrier_when_newer_persisted_state_update_is_undecodable()
{
    let state = ReadinessMirrorState::default();
    let server = TestServer::start(undecodable_newer_state_readiness_mirror_router(state)).await;
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

    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("running"),
        "a newer but undecodable persisted state update should not suppress a usable ready barrier"
    );
}

#[tokio::test]
async fn runtime_stream_preserves_newer_ready_barrier_state_after_later_out_of_order_state_update()
{
    let state = ReadinessMirrorState::default();
    let server = TestServer::start(ready_barrier_rebuild_regression_router(state)).await;
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

    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("running"),
        "the ready barrier should make the mirror reflect the newest running state after attach"
    );

    let stale = tokio::time::timeout(Duration::from_millis(200), stream.next_event())
        .await
        .expect("stale queued event should arrive")
        .expect("stream read should succeed")
        .expect("stale queued event should exist");
    assert_eq!(stale.id, "evt-stale-queued");
    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("running"),
        "later mirror rebuilds should keep the newer ready barrier state instead of regressing to stale queued data"
    );
}

struct TestServer {
    base_url: String,
    task: JoinHandle<()>,
}

impl TestServer {
    async fn start(app: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener.local_addr().expect("address should resolve");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test server should stay up");
        });
        Self {
            base_url: format!("http://{address}"),
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn readiness_router() -> Router {
    Router::new().route("/sockets/events/{conversation_id}", get(readiness_socket))
}

fn forward_compatible_readiness_router() -> Router {
    Router::new().route(
        "/sockets/events/{conversation_id}",
        get(forward_compatible_readiness_socket),
    )
}

async fn readiness_socket(
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(handle_readiness_socket)
}

async fn forward_compatible_readiness_socket(
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        let ready = EventEnvelope::new(
            "evt-ready-forward-compatible",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": {
                    "current": "running",
                },
                "state_delta": {
                    "execution_status": "running",
                },
            }),
        );

        socket
            .send(text_message(
                serde_json::to_string(&ready).expect("event should serialize"),
            ))
            .await
            .expect("forward-compatible ready event should send");
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
}

async fn handle_readiness_socket(mut socket: WebSocket) {
    let unrelated = EventEnvelope::new(
        "evt-unrelated",
        Utc::now(),
        "user",
        "MessageEvent",
        json!({
            "role": "user",
            "content": [{ "type": "text", "text": "hello" }],
        }),
    );
    let ready = EventEnvelope::state_update("evt-ready", "idle");

    socket
        .send(Message::Ping(vec![1, 2, 3].into()))
        .await
        .expect("ping should send");
    socket
        .send(text_message(
            serde_json::to_string(&unrelated).expect("event should serialize"),
        ))
        .await
        .expect("unrelated event should send");
    socket
        .send(text_message(
            serde_json::to_string(&ready).expect("event should serialize"),
        ))
        .await
        .expect("ready event should send");
}

#[derive(Clone, Default)]
struct ReadinessMirrorState {
    conversation: Arc<Mutex<Option<Conversation>>>,
}

fn readiness_mirror_router(state: ReadinessMirrorState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(readiness_mirror_create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(readiness_mirror_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(readiness_mirror_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(readiness_mirror_events_socket),
        )
        .with_state(state)
}

fn forward_compatible_readiness_mirror_router(state: ReadinessMirrorState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(readiness_mirror_create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(readiness_mirror_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(readiness_mirror_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(forward_compatible_readiness_mirror_events_socket),
        )
        .with_state(state)
}

fn reused_conversation_readiness_mirror_router(state: ReadinessMirrorState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(reused_conversation_readiness_mirror_create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(readiness_mirror_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(readiness_mirror_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(readiness_mirror_events_socket),
        )
        .with_state(state)
}

fn undecodable_newer_state_readiness_mirror_router(state: ReadinessMirrorState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(readiness_mirror_create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(readiness_mirror_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(undecodable_newer_state_readiness_mirror_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(readiness_mirror_events_socket),
        )
        .with_state(state)
}

fn ready_barrier_rebuild_regression_router(state: ReadinessMirrorState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(readiness_mirror_create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(readiness_mirror_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(readiness_mirror_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(ready_barrier_rebuild_regression_events_socket),
        )
        .with_state(state)
}

async fn readiness_mirror_create_conversation(
    State(state): State<ReadinessMirrorState>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "queued".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };
    *state.conversation.lock().await = Some(conversation.clone());
    Ok(Json(conversation))
}

async fn reused_conversation_readiness_mirror_create_conversation(
    State(state): State<ReadinessMirrorState>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "finished".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };
    *state.conversation.lock().await = Some(conversation.clone());
    Ok(Json(conversation))
}

async fn readiness_mirror_get_conversation(
    State(state): State<ReadinessMirrorState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = state
        .conversation
        .lock()
        .await
        .clone()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation))
}

async fn readiness_mirror_search_events(
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    Ok(Json(SearchConversationEventsResponse {
        events: Vec::new(),
        next_page_id: None,
    }))
}

async fn undecodable_newer_state_readiness_mirror_search_events(
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    let ready_timestamp = Utc::now();
    Ok(Json(SearchConversationEventsResponse {
        events: vec![EventEnvelope::new(
            "evt-unknown-later-state",
            ready_timestamp + chrono::Duration::seconds(1),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": {
                    "current": "queued",
                },
            }),
        )],
        next_page_id: None,
    }))
}

async fn readiness_mirror_events_socket(
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::new(
                    "evt-ready-running",
                    Utc::now(),
                    "runtime",
                    "ConversationStateUpdateEvent",
                    json!({
                        "execution_status": "running",
                        "state_delta": {
                            "execution_status": "running",
                        },
                    }),
                ))
                .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
}

async fn forward_compatible_readiness_mirror_events_socket(
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::new(
                    "evt-ready-running-forward-compatible",
                    Utc::now(),
                    "runtime",
                    "ConversationStateUpdateEvent",
                    json!({
                        "execution_status": {
                            "current": "running",
                        },
                        "state_delta": {
                            "execution_status": "running",
                        },
                    }),
                ))
                .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
}

async fn ready_barrier_rebuild_regression_events_socket(
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        let ready = EventEnvelope::new(
            "evt-ready-running",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "running",
                "state_delta": {
                    "execution_status": "running",
                },
            }),
        );
        let stale_queued = EventEnvelope::new(
            "evt-stale-queued",
            ready.timestamp - chrono::Duration::seconds(1),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "queued",
                "state_delta": {
                    "execution_status": "queued",
                },
            }),
        );

        socket
            .send(text_message(
                serde_json::to_string(&ready).expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");
        socket
            .send(text_message(
                serde_json::to_string(&stale_queued).expect("stale queued event should serialize"),
            ))
            .await
            .expect("stale queued event should send");
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
}

#[derive(Clone)]
struct AuthState {
    expectations: AuthExpectations,
    conversation: Arc<Mutex<Option<Conversation>>>,
    ready_events: Arc<Mutex<Vec<EventEnvelope>>>,
}

#[derive(Clone)]
struct AuthExpectations {
    rest: ExpectedAuth,
    websocket: ExpectedAuth,
}

#[derive(Clone)]
enum ExpectedAuth {
    QueryParam { name: String, value: String },
    Header { name: String, value: String },
}

fn auth_router(expectations: AuthExpectations) -> Router {
    let state = AuthState {
        expectations,
        conversation: Arc::new(Mutex::new(None)),
        ready_events: Arc::new(Mutex::new(vec![EventEnvelope::state_update(
            "evt-ready",
            "idle",
        )])),
    };

    Router::new()
        .route("/api/conversations", post(create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(authenticated_readiness_socket),
        )
        .with_state(state)
}

async fn create_conversation(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    ensure_expected_auth(&state.expectations.rest, &headers, &query)?;
    let conversation = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "idle".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };
    *state.conversation.lock().await = Some(conversation.clone());
    Ok(Json(conversation))
}

async fn get_conversation(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    ensure_expected_auth(&state.expectations.rest, &headers, &query)?;
    let conversation = state
        .conversation
        .lock()
        .await
        .clone()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation))
}

async fn send_message(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(_conversation_id): Path<Uuid>,
    Json(_request): Json<SendMessageRequest>,
) -> Result<Json<Value>, StatusCode> {
    ensure_expected_auth(&state.expectations.rest, &headers, &query)?;
    Ok(Json(json!({ "success": true })))
}

async fn run_conversation(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    ensure_expected_auth(&state.expectations.rest, &headers, &query)?;
    Ok(Json(json!({ "success": true })))
}

async fn search_events(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    ensure_expected_auth(&state.expectations.rest, &headers, &query)?;
    let offset = query
        .get("page_id")
        .map(String::as_str)
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let events = state.ready_events.lock().await;
    let page = events
        .iter()
        .skip(offset)
        .take(1)
        .cloned()
        .collect::<Vec<_>>();
    let next_page_id = (offset + page.len() < events.len()).then(|| (offset + 1).to_string());
    Ok(Json(SearchConversationEventsResponse {
        events: page,
        next_page_id,
    }))
}

async fn authenticated_readiness_socket(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    ensure_expected_auth(&state.expectations.websocket, &headers, &query)?;
    let ready_events = state.ready_events.lock().await.clone();

    Ok(websocket.on_upgrade(async move |mut socket| {
        for event in ready_events {
            socket
                .send(text_message(
                    serde_json::to_string(&event).expect("event should serialize"),
                ))
                .await
                .expect("ready event should send");
        }
    }))
}

fn ensure_expected_auth(
    expected: &ExpectedAuth,
    headers: &HeaderMap,
    query: &HashMap<String, String>,
) -> Result<(), StatusCode> {
    match expected {
        ExpectedAuth::QueryParam { name, value } => match query.get(name) {
            Some(actual) if actual == value => Ok(()),
            _ => Err(StatusCode::UNAUTHORIZED),
        },
        ExpectedAuth::Header { name, value } => match headers.get(name) {
            Some(actual)
                if actual
                    .to_str()
                    .map(|candidate| candidate == value)
                    .unwrap_or(false) =>
            {
                Ok(())
            }
            _ => Err(StatusCode::UNAUTHORIZED),
        },
    }
}

fn malformed_payload_router() -> Router {
    Router::new().route("/api/conversations", post(malformed_create_conversation))
}

async fn malformed_create_conversation() -> impl IntoResponse {
    (StatusCode::OK, "not-json")
}

#[derive(Clone, Default)]
struct ProbeState {
    conversation: Arc<Mutex<Option<Conversation>>>,
    events: Arc<Mutex<Vec<EventEnvelope>>>,
    send_count: Arc<Mutex<usize>>,
    run_count: Arc<Mutex<usize>>,
    connect_count: Arc<Mutex<usize>>,
    get_count: Arc<Mutex<usize>>,
}

fn probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(probe_events_socket),
        )
        .with_state(state)
}

fn failed_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(failed_probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(probe_events_socket),
        )
        .with_state(state)
}

fn reconnect_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(reconnect_probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(reconnect_probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(reconnect_probe_events_socket),
        )
        .with_state(state)
}

fn terminal_rest_refresh_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(reconnect_probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(reconnect_probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(exhausting_reconnect_probe_events_socket),
        )
        .with_state(state)
}

fn final_refresh_failure_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(final_refresh_failure_probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(final_refresh_failure_probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(final_refresh_failure_probe_events_socket),
        )
        .with_state(state)
}

fn reconnect_error_then_finished_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(error_then_finished_probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(reconnect_probe_events_socket),
        )
        .with_state(state)
}

fn finished_then_error_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/{conversation_id}",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/{conversation_id}/run",
            post(final_refresh_failure_probe_run_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(finished_then_error_probe_events_socket),
        )
        .with_state(state)
}

async fn probe_create_conversation(
    State(state): State<ProbeState>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "idle".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };
    *state.conversation.lock().await = Some(conversation.clone());
    *state.events.lock().await = vec![EventEnvelope::state_update("evt-ready", "idle")];
    Ok(Json(conversation))
}

async fn probe_get_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    *state.get_count.lock().await += 1;
    let conversation = state
        .conversation
        .lock()
        .await
        .clone()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation))
}

async fn final_refresh_failure_probe_get_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    let mut get_count = state.get_count.lock().await;
    *get_count += 1;
    let current_get = *get_count;
    drop(get_count);

    if current_get >= 2 && *state.run_count.lock().await > 0 {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    let conversation = state
        .conversation
        .lock()
        .await
        .clone()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation))
}

async fn probe_send_message(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<Value>, StatusCode> {
    *state.send_count.lock().await += 1;
    state.events.lock().await.push(EventEnvelope::new(
        "evt-message",
        Utc::now(),
        "user",
        "MessageEvent",
        serde_json::to_value(request).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    ));
    Ok(Json(json!({ "success": true })))
}

async fn probe_run_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    *state.run_count.lock().await += 1;
    state.events.lock().await.push(EventEnvelope::new(
        "evt-complete",
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        json!({
            "execution_status": "finished",
            "state_delta": {
                "execution_status": "finished",
            },
        }),
    ));
    Ok(Json(json!({ "success": true })))
}

async fn failed_probe_run_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    *state.run_count.lock().await += 1;
    state.events.lock().await.push(EventEnvelope::new(
        "evt-error",
        Utc::now(),
        "runtime",
        "ConversationErrorEvent",
        json!({
            "message": "synthetic probe failure",
        }),
    ));
    Ok(Json(json!({ "success": true })))
}

async fn reconnect_probe_send_message(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
    Json(_request): Json<SendMessageRequest>,
) -> Result<Json<Value>, StatusCode> {
    *state.send_count.lock().await += 1;
    Ok(Json(json!({ "success": true })))
}

async fn reconnect_probe_run_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    *state.run_count.lock().await += 1;
    let mut conversation = state.conversation.lock().await;
    let conversation = conversation.as_mut().ok_or(StatusCode::NOT_FOUND)?;
    conversation.execution_status = "finished".to_string();
    Ok(Json(json!({ "success": true })))
}

async fn final_refresh_failure_probe_run_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    *state.run_count.lock().await += 1;
    Ok(Json(json!({ "success": true })))
}

async fn error_then_finished_probe_run_conversation(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    *state.run_count.lock().await += 1;
    let now = Utc::now();
    state.events.lock().await.extend([
        EventEnvelope::new(
            "evt-error",
            now,
            "runtime",
            "ConversationErrorEvent",
            json!({
                "message": "synthetic probe failure",
            }),
        ),
        EventEnvelope::new(
            "evt-finished",
            now + chrono::Duration::seconds(1),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "finished",
                "state_delta": {
                    "execution_status": "finished",
                },
            }),
        ),
    ]);
    let mut conversation = state.conversation.lock().await;
    let conversation = conversation.as_mut().ok_or(StatusCode::NOT_FOUND)?;
    conversation.execution_status = "finished".to_string();
    Ok(Json(json!({ "success": true })))
}

async fn probe_search_events(
    State(state): State<ProbeState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    let events = state.events.lock().await.clone();
    Ok(Json(SearchConversationEventsResponse {
        events,
        next_page_id: None,
    }))
}

async fn probe_events_socket(
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::state_update("evt-ready", "idle"))
                    .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");
    })
}

async fn reconnect_probe_events_socket(
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::state_update("evt-ready", "idle"))
                    .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");
        socket
            .send(Message::Close(None))
            .await
            .expect("socket close should send");
    })
}

async fn exhausting_reconnect_probe_events_socket(
    State(state): State<ProbeState>,
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    let mut connect_count = state.connect_count.lock().await;
    *connect_count += 1;
    let connection_number = *connect_count;
    drop(connect_count);

    websocket.on_upgrade(async move |mut socket| {
        if connection_number == 1 {
            socket
                .send(text_message(
                    serde_json::to_string(&EventEnvelope::state_update("evt-ready", "idle"))
                        .expect("ready event should serialize"),
                ))
                .await
                .expect("ready event should send");
        }

        socket
            .send(Message::Close(None))
            .await
            .expect("socket close should send");
    })
}

async fn final_refresh_failure_probe_events_socket(
    State(state): State<ProbeState>,
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::state_update("evt-ready", "idle"))
                    .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");

        loop {
            if *state.run_count.lock().await > 0 {
                socket
                    .send(text_message(
                        serde_json::to_string(&EventEnvelope::new(
                            "evt-finished",
                            Utc::now(),
                            "runtime",
                            "ConversationStateUpdateEvent",
                            json!({
                                "execution_status": "finished",
                                "state_delta": {
                                    "execution_status": "finished",
                                },
                            }),
                        ))
                        .expect("finished event should serialize"),
                    ))
                    .await
                    .expect("finished event should send");
                break;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let _ = socket.recv().await;
    })
}

async fn finished_then_error_probe_events_socket(
    State(state): State<ProbeState>,
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::state_update("evt-ready", "idle"))
                    .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");

        loop {
            if *state.run_count.lock().await > 0 {
                socket
                    .send(text_message(
                        serde_json::to_string(&EventEnvelope::new(
                            "evt-finished",
                            Utc::now(),
                            "runtime",
                            "ConversationStateUpdateEvent",
                            json!({
                                "execution_status": "finished",
                                "state_delta": {
                                    "execution_status": "finished",
                                },
                            }),
                        ))
                        .expect("finished event should serialize"),
                    ))
                    .await
                    .expect("finished event should send");
                tokio::task::yield_now().await;
                socket
                    .send(text_message(
                        serde_json::to_string(&EventEnvelope::new(
                            "evt-error-after-finished",
                            Utc::now(),
                            "runtime",
                            "ConversationErrorEvent",
                            json!({
                                "message": "synthetic probe failure after finished",
                            }),
                        ))
                        .expect("error event should serialize"),
                    ))
                    .await
                    .expect("error event should send");
                break;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let _ = socket.recv().await;
    })
}

#[derive(Clone, Default)]
struct ReadinessReplayState {
    conversation: Arc<Mutex<Option<Conversation>>>,
    connect_count: Arc<Mutex<usize>>,
}

fn readiness_replay_router(state: ReadinessReplayState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(readiness_replay_create_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}",
            get(readiness_replay_get_conversation),
        )
        .route(
            "/api/conversations/{conversation_id}/events/search",
            get(readiness_replay_search_events),
        )
        .route(
            "/sockets/events/{conversation_id}",
            get(readiness_replay_events_socket),
        )
        .with_state(state)
}

async fn readiness_replay_create_conversation(
    State(state): State<ReadinessReplayState>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "idle".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };
    *state.conversation.lock().await = Some(conversation.clone());
    Ok(Json(conversation))
}

async fn readiness_replay_get_conversation(
    State(state): State<ReadinessReplayState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = state
        .conversation
        .lock()
        .await
        .clone()
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation))
}

async fn readiness_replay_search_events(
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    Ok(Json(SearchConversationEventsResponse {
        events: vec![],
        next_page_id: None,
    }))
}

async fn readiness_replay_events_socket(
    State(state): State<ReadinessReplayState>,
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    let mut connect_count = state.connect_count.lock().await;
    *connect_count += 1;
    let connection_number = *connect_count;
    drop(connect_count);

    websocket.on_upgrade(async move |mut socket| {
        let ready_id = if connection_number == 1 {
            "evt-ready"
        } else {
            "evt-ready-reconnect"
        };
        socket
            .send(text_message(
                serde_json::to_string(&EventEnvelope::new(
                    ready_id,
                    Utc::now(),
                    "runtime",
                    "ConversationStateUpdateEvent",
                    json!({
                        "execution_status": "idle",
                        "state_delta": {
                            "execution_status": "idle",
                        },
                    }),
                ))
                .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");

        if connection_number == 1 {
            socket
                .send(Message::Close(None))
                .await
                .expect("first socket close should send");
        } else {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    })
}
