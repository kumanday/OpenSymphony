use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use opensymphony_openhands::{
    AuthConfig, Conversation, ConversationCreateRequest, EventEnvelope, OpenHandsClient,
    OpenHandsError, RuntimeStreamConfig, SearchConversationEventsResponse, SendMessageRequest,
    TerminalExecutionStatus, TransportConfig,
};
use serde_json::{json, Value};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};
use uuid::Uuid;

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
async fn runtime_stream_replays_initial_snapshot_when_post_ready_reconcile_is_empty() {
    let state = InitialReplayState::default();
    let server = TestServer::start(initial_replay_router(state)).await;
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
        "stream should wait for future websocket activity once the initial snapshot replay is drained"
    );
}

#[tokio::test]
async fn runtime_stream_yields_buffered_event_before_reconnect_exhaustion() {
    let state = DeferredReconnectState::default();
    let server = TestServer::start(deferred_reconnect_router(state)).await;
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
    Router::new().route("/sockets/events/:conversation_id", get(readiness_socket))
}

async fn readiness_socket(
    Path(_conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> impl IntoResponse {
    websocket.on_upgrade(handle_readiness_socket)
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
        .send(Message::Ping(vec![1, 2, 3]))
        .await
        .expect("ping should send");
    socket
        .send(Message::Text(
            serde_json::to_string(&unrelated).expect("event should serialize"),
        ))
        .await
        .expect("unrelated event should send");
    socket
        .send(Message::Text(
            serde_json::to_string(&ready).expect("event should serialize"),
        ))
        .await
        .expect("ready event should send");
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
        .route("/api/conversations/:conversation_id", get(get_conversation))
        .route(
            "/api/conversations/:conversation_id/events",
            post(send_message),
        )
        .route(
            "/api/conversations/:conversation_id/run",
            post(run_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(search_events),
        )
        .route(
            "/sockets/events/:conversation_id",
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
                .send(Message::Text(
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
struct InitialReplayState {
    conversation: Arc<Mutex<Option<Conversation>>>,
    search_count: Arc<Mutex<usize>>,
}

fn initial_replay_router(state: InitialReplayState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(initial_replay_create_conversation),
        )
        .route(
            "/api/conversations/:conversation_id",
            get(initial_replay_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(initial_replay_search_events),
        )
        .route(
            "/sockets/events/:conversation_id",
            get(initial_replay_events_socket),
        )
        .with_state(state)
}

async fn initial_replay_create_conversation(
    State(state): State<InitialReplayState>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    let conversation = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "running".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };
    *state.conversation.lock().await = Some(conversation.clone());
    Ok(Json(conversation))
}

async fn initial_replay_get_conversation(
    State(state): State<InitialReplayState>,
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

async fn initial_replay_search_events(
    State(state): State<InitialReplayState>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    let mut search_count = state.search_count.lock().await;
    *search_count += 1;
    let events = if *search_count == 1 {
        let running = EventEnvelope::new(
            "evt-running",
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
        vec![
            running.clone(),
            EventEnvelope::new(
                "evt-log",
                running.timestamp + chrono::Duration::seconds(1),
                "llm",
                "LLMCompletionLogEvent",
                json!({
                    "model": "fake-model",
                    "tokens": 42,
                }),
            ),
        ]
    } else {
        Vec::new()
    };

    Ok(Json(SearchConversationEventsResponse {
        events,
        next_page_id: None,
    }))
}

async fn initial_replay_events_socket(
    websocket: WebSocketUpgrade,
    Path(_conversation_id): Path<Uuid>,
) -> impl IntoResponse {
    websocket.on_upgrade(async move |mut socket| {
        socket
            .send(Message::Text(
                serde_json::to_string(&EventEnvelope::state_update("evt-ready", "running"))
                    .expect("ready event should serialize"),
            ))
            .await
            .expect("ready event should send");
        tokio::time::sleep(Duration::from_secs(1)).await;
    })
}

#[derive(Clone, Default)]
struct DeferredReconnectState {
    conversation: Arc<Mutex<Option<Conversation>>>,
    connect_count: Arc<Mutex<usize>>,
}

fn deferred_reconnect_router(state: DeferredReconnectState) -> Router {
    Router::new()
        .route(
            "/api/conversations",
            post(deferred_reconnect_create_conversation),
        )
        .route(
            "/api/conversations/:conversation_id",
            get(deferred_reconnect_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(deferred_reconnect_search_events),
        )
        .route(
            "/sockets/events/:conversation_id",
            get(deferred_reconnect_events_socket),
        )
        .with_state(state)
}

async fn deferred_reconnect_create_conversation(
    State(state): State<DeferredReconnectState>,
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

async fn deferred_reconnect_get_conversation(
    State(state): State<DeferredReconnectState>,
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

async fn deferred_reconnect_search_events(
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    Ok(Json(SearchConversationEventsResponse {
        events: vec![],
        next_page_id: None,
    }))
}

async fn deferred_reconnect_events_socket(
    State(state): State<DeferredReconnectState>,
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
                .send(Message::Text(
                    serde_json::to_string(&EventEnvelope::state_update("evt-ready", "idle"))
                        .expect("ready event should serialize"),
                ))
                .await
                .expect("ready event should send");
            socket
                .send(Message::Text(
                    serde_json::to_string(&EventEnvelope::new(
                        "evt-runtime",
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
                    .expect("runtime event should serialize"),
                ))
                .await
                .expect("runtime event should send");
        }

        socket
            .send(Message::Close(None))
            .await
            .expect("socket close should send");
    })
}

#[derive(Clone, Default)]
struct ProbeState {
    conversation: Arc<Mutex<Option<Conversation>>>,
    events: Arc<Mutex<Vec<EventEnvelope>>>,
    send_count: Arc<Mutex<usize>>,
    run_count: Arc<Mutex<usize>>,
}

fn probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/:conversation_id",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/:conversation_id/run",
            post(probe_run_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(probe_search_events),
        )
        .route("/sockets/events/:conversation_id", get(probe_events_socket))
        .with_state(state)
}

fn failed_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/:conversation_id",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/:conversation_id/run",
            post(failed_probe_run_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(probe_search_events),
        )
        .route("/sockets/events/:conversation_id", get(probe_events_socket))
        .with_state(state)
}

fn reconnect_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/:conversation_id",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events",
            post(reconnect_probe_send_message),
        )
        .route(
            "/api/conversations/:conversation_id/run",
            post(reconnect_probe_run_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/:conversation_id",
            get(reconnect_probe_events_socket),
        )
        .with_state(state)
}

fn reconnect_error_then_finished_probe_router(state: ProbeState) -> Router {
    Router::new()
        .route("/api/conversations", post(probe_create_conversation))
        .route(
            "/api/conversations/:conversation_id",
            get(probe_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events",
            post(probe_send_message),
        )
        .route(
            "/api/conversations/:conversation_id/run",
            post(error_then_finished_probe_run_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(probe_search_events),
        )
        .route(
            "/sockets/events/:conversation_id",
            get(reconnect_probe_events_socket),
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
            .send(Message::Text(
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
            .send(Message::Text(
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
            "/api/conversations/:conversation_id",
            get(readiness_replay_get_conversation),
        )
        .route(
            "/api/conversations/:conversation_id/events/search",
            get(readiness_replay_search_events),
        )
        .route(
            "/sockets/events/:conversation_id",
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
            .send(Message::Text(
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
