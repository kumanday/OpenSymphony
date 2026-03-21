use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use opensymphony_openhands::{
    Conversation, ConversationCreateRequest, EventEnvelope, OpenHandsClient,
    SearchConversationEventsResponse, SendMessageRequest, TransportConfig,
};
use serde::Deserialize;
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
async fn session_api_key_authenticates_rest_operations() {
    let server = TestServer::start(auth_router("secret-token")).await;
    let mut transport = TransportConfig::new(server.base_url());
    transport.session_api_key = Some("secret-token".to_string());
    let client = OpenHandsClient::new(transport);
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
            serde_json::to_string(&unrelated)
                .expect("event should serialize"),
        ))
        .await
        .expect("unrelated event should send");
    socket
        .send(Message::Text(
            serde_json::to_string(&ready)
                .expect("event should serialize"),
        ))
        .await
        .expect("ready event should send");
}

#[derive(Clone)]
struct AuthState {
    expected_key: String,
    conversation: Arc<Mutex<Option<Conversation>>>,
    ready_events: Arc<Mutex<Vec<EventEnvelope>>>,
}

fn auth_router(expected_key: &str) -> Router {
    let state = AuthState {
        expected_key: expected_key.to_string(),
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
        .with_state(state)
}

#[derive(Debug, Deserialize, Default)]
struct AuthQuery {
    session_api_key: Option<String>,
    page_id: Option<String>,
}

async fn create_conversation(
    State(state): State<AuthState>,
    Query(query): Query<AuthQuery>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    ensure_auth(&state, &query)?;
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
    Query(query): Query<AuthQuery>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    ensure_auth(&state, &query)?;
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
    Query(query): Query<AuthQuery>,
    Path(_conversation_id): Path<Uuid>,
    Json(_request): Json<SendMessageRequest>,
) -> Result<Json<Value>, StatusCode> {
    ensure_auth(&state, &query)?;
    Ok(Json(json!({ "success": true })))
}

async fn run_conversation(
    State(state): State<AuthState>,
    Query(query): Query<AuthQuery>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    ensure_auth(&state, &query)?;
    Ok(Json(json!({ "success": true })))
}

async fn search_events(
    State(state): State<AuthState>,
    Query(query): Query<AuthQuery>,
    Path(_conversation_id): Path<Uuid>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    ensure_auth(&state, &query)?;
    let offset = query
        .page_id
        .as_deref()
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

fn ensure_auth(state: &AuthState, query: &AuthQuery) -> Result<(), StatusCode> {
    match query.session_api_key.as_deref() {
        Some(value) if value == state.expected_key => Ok(()),
        _ => Err(StatusCode::UNAUTHORIZED),
    }
}
