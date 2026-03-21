//! Scriptable fake OpenHands server for contract tests.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast, oneshot};
use url::Url;
use uuid::Uuid;

use opensymphony_openhands::{
    ConversationInfo, CreateConversationRequest, EventPage, OpenHandsClient, RemoteExecutionStatus,
    SendMessageRequest, ServerInfo, TransportConfig,
};

const SESSION_API_KEY_HEADER: &str = "x-session-api-key";

/// Testkit failure modes.
#[derive(Debug, Error)]
pub enum TestkitError {
    /// HTTP server startup failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Requested conversation was missing in the fake state.
    #[error("conversation not found in fake server: {conversation_id}")]
    MissingConversation {
        /// Missing conversation identifier.
        conversation_id: String,
    },
    /// Internal JSON conversion failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Recorded requests for one conversation.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversationRecord {
    /// Conversation snapshot returned by REST.
    pub info: ConversationInfo,
    /// Create requests received for this conversation.
    pub create_requests: Vec<CreateConversationRequest>,
    /// User-message requests received through `/events`.
    pub messages: Vec<SendMessageRequest>,
    /// Number of `/run` invocations received.
    pub run_count: usize,
    /// Event search storage retained by the fake server.
    pub events: Vec<Value>,
}

/// Scripted run behavior executed after `POST /run`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptedRun {
    /// Ordered run steps emitted by the fake server.
    pub steps: Vec<RunStep>,
}

impl ScriptedRun {
    /// Builds a scripted run from ordered steps.
    #[must_use]
    pub fn new(steps: Vec<RunStep>) -> Self {
        Self { steps }
    }

    fn default_success() -> Self {
        Self::new(vec![
            RunStep::Emit {
                event: execution_status_event(RemoteExecutionStatus::Running),
                delay: Duration::from_millis(5),
            },
            RunStep::Emit {
                event: execution_status_event(RemoteExecutionStatus::Finished),
                delay: Duration::from_millis(5),
            },
        ])
    }
}

/// One scripted run step.
#[derive(Clone, Debug, PartialEq)]
pub enum RunStep {
    /// Store and broadcast the given event after the optional delay.
    Emit {
        /// Raw JSON event to store and broadcast.
        event: Value,
        /// Delay applied before the event is emitted.
        delay: Duration,
    },
    /// Force every active WebSocket subscriber to disconnect after the delay.
    Disconnect {
        /// Delay applied before the forced disconnect is sent.
        delay: Duration,
    },
}

#[derive(Clone, Debug)]
enum WsServerMessage {
    Event(Value),
    Disconnect,
}

#[derive(Debug, Default)]
struct FakeState {
    session_api_key: Option<String>,
    conversation_get_delay: Duration,
    event_search_failures: HashMap<String, usize>,
    message_post_failures: usize,
    ready_event_delays: HashMap<String, VecDeque<Duration>>,
    websocket_handshake_delays: HashMap<String, VecDeque<Duration>>,
    conversations: HashMap<String, ConversationEntry>,
}

#[derive(Debug)]
struct ConversationEntry {
    info: ConversationInfo,
    create_requests: Vec<CreateConversationRequest>,
    messages: Vec<SendMessageRequest>,
    run_count: usize,
    events: Vec<Value>,
    runs: VecDeque<ScriptedRun>,
    stream: broadcast::Sender<WsServerMessage>,
}

impl ConversationEntry {
    fn snapshot(&self) -> ConversationRecord {
        ConversationRecord {
            info: self.info.clone(),
            create_requests: self.create_requests.clone(),
            messages: self.messages.clone(),
            run_count: self.run_count,
            events: self.events.clone(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    state: Arc<Mutex<FakeState>>,
}

#[derive(Deserialize)]
struct SearchEventsQuery {
    page_id: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct WebSocketQuery {
    session_api_key: Option<String>,
}

/// Running fake server with helpers for scripting and inspection.
#[derive(Debug)]
pub struct FakeOpenHandsServer {
    base_url: Url,
    state: Arc<Mutex<FakeState>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl FakeOpenHandsServer {
    /// Starts the fake server on a random loopback port.
    pub async fn start() -> Result<Self, TestkitError> {
        let state = Arc::new(Mutex::new(FakeState::default()));
        let app_state = AppState {
            state: state.clone(),
        };
        let app = Router::new()
            .route("/health", get(health))
            .route("/ready", get(ready))
            .route("/server_info", get(server_info))
            .route("/api/conversations", post(start_conversation))
            .route(
                "/api/conversations/{conversation_id}",
                get(get_conversation),
            )
            .route(
                "/api/conversations/{conversation_id}/events",
                post(post_event),
            )
            .route(
                "/api/conversations/{conversation_id}/events/search",
                get(search_events),
            )
            .route(
                "/api/conversations/{conversation_id}/run",
                post(run_conversation),
            )
            .route("/sockets/events/{conversation_id}", get(events_socket))
            .with_state(app_state);

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let base_url =
            Url::parse(&format!("http://{address}")).expect("loopback address must parse");
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        Ok(Self {
            base_url,
            state,
            shutdown_tx: Some(shutdown_tx),
            task,
        })
    }

    /// Returns the server root URL suitable for `TransportConfig`.
    #[must_use]
    pub fn base_url(&self) -> Url {
        self.base_url.clone()
    }

    /// Returns a ready-to-use client bound to this fake server.
    #[must_use]
    pub fn client(&self) -> OpenHandsClient {
        OpenHandsClient::new(TransportConfig {
            base_url: self.base_url(),
            http_auth: match self.auth_key_sync() {
                Some(key) => opensymphony_openhands::HttpAuth::SessionApiKey(key),
                None => opensymphony_openhands::HttpAuth::None,
            },
            http_connect_timeout_ms: 5_000,
            http_request_timeout_ms: 30_000,
            websocket_auth: opensymphony_openhands::WebSocketAuthMode::Auto,
            websocket_query_param_name: "session_api_key".to_string(),
        })
    }

    /// Sets the optional session API key required for subsequent REST and WebSocket requests.
    pub async fn set_session_api_key(&self, api_key: Option<&str>) {
        self.state.lock().await.session_api_key = api_key.map(ToString::to_string);
    }

    /// Delays `GET /api/conversations/{id}` responses to widen timing-sensitive contract tests.
    pub async fn set_conversation_get_delay(&self, delay: Duration) {
        self.state.lock().await.conversation_get_delay = delay;
    }

    /// Delays the next WebSocket handshake for the named conversation.
    pub async fn enqueue_websocket_handshake_delay(
        &self,
        conversation_id: &str,
        delay: Duration,
    ) -> Result<(), TestkitError> {
        let mut state = self.state.lock().await;
        let Some(_) = state.conversations.get(conversation_id) else {
            return Err(TestkitError::MissingConversation {
                conversation_id: conversation_id.to_string(),
            });
        };
        state
            .websocket_handshake_delays
            .entry(conversation_id.to_string())
            .or_default()
            .push_back(delay);
        Ok(())
    }

    /// Delays the next initial ready snapshot after the WebSocket handshake succeeds.
    pub async fn enqueue_ready_event_delay(
        &self,
        conversation_id: &str,
        delay: Duration,
    ) -> Result<(), TestkitError> {
        let mut state = self.state.lock().await;
        let Some(_) = state.conversations.get(conversation_id) else {
            return Err(TestkitError::MissingConversation {
                conversation_id: conversation_id.to_string(),
            });
        };
        state
            .ready_event_delays
            .entry(conversation_id.to_string())
            .or_default()
            .push_back(delay);
        Ok(())
    }

    /// Enqueues a scripted run for the named conversation.
    pub async fn enqueue_run(
        &self,
        conversation_id: &str,
        run: ScriptedRun,
    ) -> Result<(), TestkitError> {
        let mut state = self.state.lock().await;
        let entry = state
            .conversations
            .get_mut(conversation_id)
            .ok_or_else(|| TestkitError::MissingConversation {
                conversation_id: conversation_id.to_string(),
            })?;
        entry.runs.push_back(run);
        Ok(())
    }

    /// Fails the next `GET /events/search` requests for one conversation.
    pub async fn fail_next_event_searches(
        &self,
        conversation_id: &str,
        count: usize,
    ) -> Result<(), TestkitError> {
        let mut state = self.state.lock().await;
        if !state.conversations.contains_key(conversation_id) {
            return Err(TestkitError::MissingConversation {
                conversation_id: conversation_id.to_string(),
            });
        }
        if count == 0 {
            state.event_search_failures.remove(conversation_id);
        } else {
            state
                .event_search_failures
                .insert(conversation_id.to_string(), count);
        }
        Ok(())
    }

    /// Fails the next `POST /events` calls regardless of conversation.
    pub async fn fail_next_message_posts(&self, count: usize) {
        self.state.lock().await.message_post_failures = count;
    }

    /// Pushes one event into the fake server store and broadcasts it to subscribers.
    pub async fn push_event(
        &self,
        conversation_id: &str,
        event: Value,
    ) -> Result<(), TestkitError> {
        let (sender, raw_event) = {
            let mut state = self.state.lock().await;
            let entry = state
                .conversations
                .get_mut(conversation_id)
                .ok_or_else(|| TestkitError::MissingConversation {
                    conversation_id: conversation_id.to_string(),
                })?;
            update_conversation_from_event(&mut entry.info, &event);
            entry.events.push(event.clone());
            (entry.stream.clone(), event)
        };
        let _ = sender.send(WsServerMessage::Event(raw_event));
        Ok(())
    }

    /// Returns a snapshot of one conversation record.
    pub async fn conversation_record(
        &self,
        conversation_id: &str,
    ) -> Result<ConversationRecord, TestkitError> {
        let state = self.state.lock().await;
        let entry = state.conversations.get(conversation_id).ok_or_else(|| {
            TestkitError::MissingConversation {
                conversation_id: conversation_id.to_string(),
            }
        })?;
        Ok(entry.snapshot())
    }

    fn auth_key_sync(&self) -> Option<String> {
        self.state
            .try_lock()
            .ok()
            .and_then(|state| state.session_api_key.clone())
    }
}

impl Drop for FakeOpenHandsServer {
    fn drop(&mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        self.task.abort();
    }
}

async fn health(State(app): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    "OK".into_response()
}

async fn ready(State(app): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(serde_json::json!({ "status": "ready" })).into_response()
}

async fn server_info(State(app): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(ServerInfo {
        version: Some("fake-1.14.0".to_string()),
        sdk_version: Some("fake-sdk-1.14.0".to_string()),
        title: Some("Fake OpenHands Server".to_string()),
        uptime: Some(1),
        idle_time: Some(0),
    })
    .into_response()
}

async fn start_conversation(
    State(app): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateConversationRequest>,
) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let conversation_id = request
        .conversation_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut state = app.state.lock().await;
    if let Some(entry) = state.conversations.get_mut(&conversation_id) {
        entry.create_requests.push(request);
        return (StatusCode::OK, Json(entry.info.clone())).into_response();
    }

    let (sender, _) = broadcast::channel(32);
    let info = ConversationInfo {
        id: conversation_id.clone(),
        title: None,
        workspace: request.workspace.clone(),
        agent: Some(serde_json::to_value(&request.agent).expect("agent config must serialize")),
        persistence_dir: request.persistence_dir.clone(),
        max_iterations: Some(request.max_iterations),
        stuck_detection: Some(request.stuck_detection),
        execution_status: Some(RemoteExecutionStatus::Idle),
        confirmation_policy: Some(request.confirmation_policy.clone()),
        created_at: Some(Utc::now().to_rfc3339()),
        updated_at: Some(Utc::now().to_rfc3339()),
        extra: Map::new(),
    };
    state.conversations.insert(
        conversation_id,
        ConversationEntry {
            info: info.clone(),
            create_requests: vec![request],
            messages: Vec::new(),
            run_count: 0,
            events: Vec::new(),
            runs: VecDeque::new(),
            stream: sender,
        },
    );
    (StatusCode::CREATED, Json(info)).into_response()
}

async fn get_conversation(
    State(app): State<AppState>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (delay, info) = {
        let state = app.state.lock().await;
        (
            state.conversation_get_delay,
            state
                .conversations
                .get(&conversation_id)
                .map(|entry| entry.info.clone()),
        )
    };
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    match info {
        Some(entry) => (StatusCode::OK, Json(entry)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn post_event(
    State(app): State<AppState>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    Json(message): Json<SendMessageRequest>,
) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let maybe_sender = {
        let mut state = app.state.lock().await;
        if state.message_post_failures > 0 {
            state.message_post_failures -= 1;
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        let Some(entry) = state.conversations.get_mut(&conversation_id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        entry.messages.push(message.clone());
        let raw_event = generic_event(
            "MessageEvent",
            "user",
            serde_json::json!({
                "role": message.role,
                "content": message.content,
                "run": message.run,
            }),
        );
        entry.events.push(raw_event.clone());
        Some((entry.stream.clone(), raw_event))
    };
    if let Some((sender, event)) = maybe_sender {
        let _ = sender.send(WsServerMessage::Event(event));
    }
    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

async fn search_events(
    State(app): State<AppState>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<SearchEventsQuery>,
) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let (events, search_failure) = {
        let mut state = app.state.lock().await;
        let search_failure = match state.event_search_failures.get_mut(&conversation_id) {
            Some(remaining) if *remaining > 0 => {
                *remaining -= 1;
                true
            }
            _ => false,
        };
        if search_failure {
            if state
                .event_search_failures
                .get(&conversation_id)
                .copied()
                .unwrap_or_default()
                == 0
            {
                state.event_search_failures.remove(&conversation_id);
            }
            (Vec::new(), true)
        } else {
            let Some(entry) = state.conversations.get(&conversation_id) else {
                return StatusCode::NOT_FOUND.into_response();
            };
            (entry.events.clone(), false)
        }
    };
    if search_failure {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let offset = query
        .page_id
        .as_deref()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    let items: Vec<_> = events.iter().skip(offset).take(limit).cloned().collect();
    let next_page_id = if offset + items.len() < events.len() {
        Some((offset + items.len()).to_string())
    } else {
        None
    };
    (
        StatusCode::OK,
        Json(EventPage {
            items,
            next_page_id,
        }),
    )
        .into_response()
}

async fn run_conversation(
    State(app): State<AppState>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&app.state, &headers, None).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let run = {
        let mut state = app.state.lock().await;
        let Some(entry) = state.conversations.get_mut(&conversation_id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        entry.run_count += 1;
        entry
            .runs
            .pop_front()
            .unwrap_or_else(ScriptedRun::default_success)
    };
    tokio::spawn(execute_run(app.state.clone(), conversation_id, run));
    (StatusCode::OK, Json(serde_json::json!({ "success": true }))).into_response()
}

async fn events_socket(
    ws: WebSocketUpgrade,
    State(app): State<AppState>,
    Path(conversation_id): Path<String>,
    headers: HeaderMap,
    Query(query): Query<WebSocketQuery>,
) -> impl IntoResponse {
    if !authorized(&app.state, &headers, query.session_api_key).await {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let Some((snapshot, receiver, handshake_delay, ready_delay)) =
        prepare_socket(&app.state, &conversation_id).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Some(delay) = handshake_delay {
        tokio::time::sleep(delay).await;
    }
    ws.on_upgrade(move |socket| socket_task(socket, snapshot, receiver, ready_delay))
}

async fn prepare_socket(
    state: &Arc<Mutex<FakeState>>,
    conversation_id: &str,
) -> Option<(
    Value,
    broadcast::Receiver<WsServerMessage>,
    Option<Duration>,
    Option<Duration>,
)> {
    let mut state = state.lock().await;
    let (snapshot, receiver) = {
        let entry = state.conversations.get(conversation_id)?;
        (
            full_state_event_from_info(&entry.info),
            entry.stream.subscribe(),
        )
    };
    let handshake_delay = state
        .websocket_handshake_delays
        .get_mut(conversation_id)
        .and_then(VecDeque::pop_front);
    let ready_delay = state
        .ready_event_delays
        .get_mut(conversation_id)
        .and_then(VecDeque::pop_front);
    Some((snapshot, receiver, handshake_delay, ready_delay))
}

async fn socket_task(
    mut socket: WebSocket,
    snapshot: Value,
    mut receiver: broadcast::Receiver<WsServerMessage>,
    ready_delay: Option<Duration>,
) {
    if let Some(delay) = ready_delay {
        tokio::time::sleep(delay).await;
    }
    if socket
        .send(Message::Text(snapshot.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        match receiver.recv().await {
            Ok(WsServerMessage::Event(event)) => {
                if socket
                    .send(Message::Text(event.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Ok(WsServerMessage::Disconnect) => return,
            Err(_) => return,
        }
    }
}

async fn execute_run(state: Arc<Mutex<FakeState>>, conversation_id: String, run: ScriptedRun) {
    for step in run.steps {
        match step {
            RunStep::Emit { event, delay } => {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let sender = {
                    let mut state = state.lock().await;
                    let Some(entry) = state.conversations.get_mut(&conversation_id) else {
                        return;
                    };
                    update_conversation_from_event(&mut entry.info, &event);
                    entry.events.push(event.clone());
                    entry.stream.clone()
                };
                let _ = sender.send(WsServerMessage::Event(event));
            }
            RunStep::Disconnect { delay } => {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                let sender = {
                    let state = state.lock().await;
                    let Some(entry) = state.conversations.get(&conversation_id) else {
                        return;
                    };
                    entry.stream.clone()
                };
                let _ = sender.send(WsServerMessage::Disconnect);
            }
        }
    }
}

async fn authorized(
    state: &Arc<Mutex<FakeState>>,
    headers: &HeaderMap,
    query_api_key: Option<String>,
) -> bool {
    let expected = state.lock().await.session_api_key.clone();
    let Some(expected) = expected else {
        return true;
    };
    if query_api_key.as_deref() == Some(expected.as_str()) {
        return true;
    }
    headers
        .get(SESSION_API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some(expected.as_str())
}

fn update_conversation_from_event(info: &mut ConversationInfo, event: &Value) {
    let Some(kind) = event.get("kind").and_then(Value::as_str) else {
        return;
    };
    if kind != "ConversationStateUpdateEvent" {
        return;
    }
    let Some(key) = event.get("key").and_then(Value::as_str) else {
        return;
    };
    let Some(value) = event.get("value") else {
        return;
    };
    match key {
        "execution_status" => {
            info.execution_status = serde_json::from_value(value.clone()).ok();
        }
        "full_state" => {
            if let Some(status) = value
                .get("execution_status")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok())
            {
                info.execution_status = Some(status);
            }
        }
        _ => {}
    }
    info.updated_at = Some(Utc::now().to_rfc3339());
}

fn full_state_event_from_info(info: &ConversationInfo) -> Value {
    full_state_event(serde_json::json!({
        "id": info.id,
        "workspace": info.workspace,
        "execution_status": info.execution_status.unwrap_or(RemoteExecutionStatus::Idle),
        "persistence_dir": info.persistence_dir,
        "title": info.title,
    }))
}

/// Builds one generic event with the given kind, source, and payload object.
#[must_use]
pub fn generic_event(kind: &str, source: &str, payload: Value) -> Value {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert("id".to_string(), Value::String(Uuid::new_v4().to_string()));
    object.insert(
        "timestamp".to_string(),
        Value::String(Utc::now().to_rfc3339()),
    );
    object.insert("source".to_string(), Value::String(source.to_string()));
    object.insert("kind".to_string(), Value::String(kind.to_string()));
    Value::Object(object)
}

/// Builds a `ConversationStateUpdateEvent` for the supplied full-state snapshot.
#[must_use]
pub fn full_state_event(snapshot: Value) -> Value {
    generic_event(
        "ConversationStateUpdateEvent",
        "environment",
        serde_json::json!({
            "key": "full_state",
            "value": snapshot,
        }),
    )
}

/// Builds a `ConversationStateUpdateEvent` that only changes `execution_status`.
#[must_use]
pub fn execution_status_event(status: RemoteExecutionStatus) -> Value {
    generic_event(
        "ConversationStateUpdateEvent",
        "environment",
        serde_json::json!({
            "key": "execution_status",
            "value": status,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensymphony_domain::{IssueRef, PromptSet};
    use opensymphony_openhands::{
        AgentConfig, AttachedConversation, ConfirmationPolicy, ContentBlock, IssueSessionRequest,
        IssueSessionRunner, OpenHandsError, OpenHandsWorkspace, RemoteExecutionStatus,
        WebSocketConfig,
    };
    use opensymphony_workspace::ConversationManifest;
    use tempfile::TempDir;

    fn agent() -> AgentConfig {
        AgentConfig {
            kind: Some("Agent".to_string()),
            llm: opensymphony_openhands::LlmConfig {
                model: "gpt-4o".to_string(),
                api_key: Some("fake".to_string()),
                base_url: None,
                api_version: None,
                usage_id: Some("test".to_string()),
                max_output_tokens: None,
                log_completions: false,
                log_completions_folder: None,
                extra: Map::new(),
            },
            tools: Vec::new(),
            include_default_tools: Vec::new(),
            filter_tools_regex: None,
            mcp_config: None,
            extra: Map::new(),
        }
    }

    #[tokio::test]
    async fn client_contract_covers_http_and_event_pagination()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        let request = CreateConversationRequest {
            agent: agent(),
            workspace: OpenHandsWorkspace {
                working_dir: "/tmp/test".to_string(),
                kind: None,
                extra: Map::new(),
            },
            conversation_id: Some("conv-1".to_string()),
            persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
            confirmation_policy: ConfirmationPolicy::never_confirm(),
            initial_message: None,
            max_iterations: 500,
            stuck_detection: true,
            autotitle: false,
            hook_config: None,
            plugins: Vec::new(),
            secrets: HashMap::new().into_iter().collect(),
            tool_module_qualnames: HashMap::new().into_iter().collect(),
        };

        let conversation = client.create_conversation(&request).await?;
        assert_eq!(conversation.id, "conv-1");
        client
            .send_user_message("conv-1", &SendMessageRequest::user_text("hello"))
            .await?;
        server
            .push_event(
                "conv-1",
                generic_event(
                    "MessageEvent",
                    "agent",
                    serde_json::json!({"body": "world"}),
                ),
            )
            .await?;

        let page_one = client.search_events_page("conv-1", None, 1).await?;
        let page_two = client
            .search_events_page("conv-1", page_one.next_page_id.as_deref(), 1)
            .await?;
        assert_eq!(page_one.items.len(), 1);
        assert_eq!(page_two.items.len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn client_root_probes_honor_session_api_key() -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        server.set_session_api_key(Some("secret")).await;
        let client = server.client();

        client.health().await?;
        client.ready().await?;
        let info = client.server_info().await?;

        assert_eq!(info.title.as_deref(), Some("Fake OpenHands Server"));
        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_reconciles_disconnect_window() -> Result<(), Box<dyn std::error::Error>>
    {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-reconnect".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        server
            .enqueue_run(
                "conv-reconnect",
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Disconnect {
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: generic_event(
                            "MessageEvent",
                            "agent",
                            serde_json::json!({"body": "after disconnect"}),
                        ),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Finished),
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await?;

        let mut attached = AttachedConversation::attach(
            client.clone(),
            "conv-reconnect",
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await?;
        client.run_conversation("conv-reconnect").await?;
        let final_info = attached.wait_for_terminal(Duration::from_secs(2)).await?;
        assert_eq!(
            final_info.execution_status,
            Some(RemoteExecutionStatus::Finished)
        );
        let events = attached.cached_events().await;
        assert!(events.iter().any(|event| {
            event.raw_json.get("body") == Some(&Value::String("after disconnect".to_string()))
        }));
        attached.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_retries_reconnect_until_reconcile_succeeds()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-reconnect-retry".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        server
            .enqueue_run(
                "conv-reconnect-retry",
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Disconnect {
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: generic_event(
                            "MessageEvent",
                            "agent",
                            serde_json::json!({"body": "after failed reconcile"}),
                        ),
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await?;

        let attached = AttachedConversation::attach(
            client.clone(),
            "conv-reconnect-retry",
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await?;

        server
            .fail_next_event_searches("conv-reconnect-retry", 1)
            .await?;
        client.run_conversation("conv-reconnect-retry").await?;

        let message = Value::String("after failed reconcile".to_string());
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let events = attached.cached_events().await;
                if events
                    .iter()
                    .any(|event| event.raw_json.get("body") == Some(&message))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disconnect-window message should be reconciled after retry");

        attached.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_times_out_blackholed_handshake()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-blackhole-handshake".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        server
            .enqueue_websocket_handshake_delay(
                "conv-blackhole-handshake",
                Duration::from_millis(200),
            )
            .await?;

        let error = AttachedConversation::attach(
            client,
            "conv-blackhole-handshake",
            WebSocketConfig {
                ready_timeout_ms: 50,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await
        .expect_err("handshake should time out");
        match error {
            OpenHandsError::Timeout { operation, .. } => {
                assert_eq!(operation, "websocket handshake");
            }
            other => panic!("expected websocket handshake timeout, got {other:?}"),
        }

        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_close_interrupts_blackholed_reconnect()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-reconnect-blackhole".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        server
            .enqueue_run(
                "conv-reconnect-blackhole",
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Disconnect {
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await?;

        let attached = AttachedConversation::attach(
            client.clone(),
            "conv-reconnect-blackhole",
            WebSocketConfig {
                ready_timeout_ms: 50,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await?;

        server
            .enqueue_websocket_handshake_delay("conv-reconnect-blackhole", Duration::from_secs(1))
            .await?;
        client.run_conversation("conv-reconnect-blackhole").await?;

        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::time::timeout(Duration::from_millis(150), attached.close()).await??;
        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_closes_after_post_terminal_event()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        server
            .set_conversation_get_delay(Duration::from_millis(25))
            .await;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-post-terminal".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        server
            .enqueue_run(
                "conv-post-terminal",
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Finished),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: generic_event(
                            "MessageEvent",
                            "agent",
                            serde_json::json!({"body": "after finish"}),
                        ),
                        delay: Duration::from_millis(1),
                    },
                ]),
            )
            .await?;

        let mut attached = AttachedConversation::attach(
            client.clone(),
            "conv-post-terminal",
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await?;
        client.run_conversation("conv-post-terminal").await?;
        let final_info = attached.wait_for_terminal(Duration::from_secs(2)).await?;
        assert_eq!(
            final_info.execution_status,
            Some(RemoteExecutionStatus::Finished)
        );
        tokio::time::timeout(Duration::from_secs(1), attached.close()).await??;
        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_tolerates_terminal_reconcile_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-terminal-reconcile".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        let mut attached = AttachedConversation::attach(
            client.clone(),
            "conv-terminal-reconcile",
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await?;

        server
            .fail_next_event_searches("conv-terminal-reconcile", 1)
            .await?;
        client.run_conversation("conv-terminal-reconcile").await?;

        let final_info = attached.wait_for_terminal(Duration::from_secs(2)).await?;
        assert_eq!(
            final_info.execution_status,
            Some(RemoteExecutionStatus::Finished)
        );
        tokio::time::timeout(Duration::from_secs(1), attached.close()).await??;
        Ok(())
    }

    #[tokio::test]
    async fn issue_session_runner_reuses_conversation_and_switches_prompt_kind()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        let temp_dir = TempDir::new()?;
        let workspace = opensymphony_workspace::WorkspaceLayout::new(temp_dir.path(), "COE-253")?;
        workspace.create()?;

        let conversation_id = "session-conv".to_string();
        ConversationManifest {
            issue_id: "issue-1".to_string(),
            identifier: "COE-253".to_string(),
            conversation_id: conversation_id.clone(),
            server_base_url: client.transport().base_url.to_string(),
            persistence_dir: workspace.openhands_dir.display().to_string(),
            created_at: Utc::now(),
            last_attached_at: Utc::now(),
            fresh_conversation: true,
            reset_reason: None,
            runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
        }
        .save(&workspace.conversation_manifest_path)?;

        server
            .enqueue_run(
                &conversation_id,
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Finished),
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await
            .expect_err("pre-existing manifest should refer to a missing server conversation");
        let runner = IssueSessionRunner::new(
            client.clone(),
            opensymphony_openhands::ConversationConfig {
                runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
                persistence_dir_relative: ".opensymphony/openhands".to_string(),
                agent: agent(),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
            },
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .with_run_timeout(Duration::from_secs(2));

        let _ = server
            .enqueue_run(
                &conversation_id,
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Finished),
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await;

        let first = runner
            .execute(&IssueSessionRequest {
                issue: IssueRef {
                    issue_id: "issue-1".to_string(),
                    identifier: "COE-253".to_string(),
                    title: "Runtime adapter".to_string(),
                },
                workspace: workspace.clone(),
                prompts: PromptSet {
                    full_prompt: "full prompt".to_string(),
                    continuation_prompt: "continue prompt".to_string(),
                },
            })
            .await?;
        assert_eq!(first.prompt_kind, opensymphony_domain::PromptKind::Fresh);

        server
            .enqueue_run(
                &conversation_id,
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Finished),
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await?;
        let second = runner
            .execute(&IssueSessionRequest {
                issue: IssueRef {
                    issue_id: "issue-1".to_string(),
                    identifier: "COE-253".to_string(),
                    title: "Runtime adapter".to_string(),
                },
                workspace: workspace.clone(),
                prompts: PromptSet {
                    full_prompt: "full prompt".to_string(),
                    continuation_prompt: "continue prompt".to_string(),
                },
            })
            .await?;
        assert_eq!(
            second.prompt_kind,
            opensymphony_domain::PromptKind::Continuation
        );

        let record = server.conversation_record(&conversation_id).await?;
        let texts: Vec<_> = record
            .messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::Image { .. } => None,
            })
            .collect();
        assert_eq!(texts, vec!["full prompt", "continue prompt"]);
        Ok(())
    }

    #[tokio::test]
    async fn issue_session_runner_resets_stale_workspace_binding()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        let temp_dir = TempDir::new()?;
        let workspace = opensymphony_workspace::WorkspaceLayout::new(temp_dir.path(), "COE-253")?;
        workspace.create()?;

        let stale_conversation_id = "stale-workspace-conv".to_string();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/old-workspace".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some(stale_conversation_id.clone()),
                persistence_dir: Some("/tmp/old-workspace/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        ConversationManifest {
            issue_id: "issue-1".to_string(),
            identifier: "COE-253".to_string(),
            conversation_id: stale_conversation_id.clone(),
            server_base_url: client.transport().base_url.to_string(),
            persistence_dir: workspace.openhands_dir.display().to_string(),
            created_at: Utc::now(),
            last_attached_at: Utc::now(),
            fresh_conversation: false,
            reset_reason: None,
            runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
        }
        .save(&workspace.conversation_manifest_path)?;

        let runner = IssueSessionRunner::new(
            client.clone(),
            opensymphony_openhands::ConversationConfig {
                runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
                persistence_dir_relative: ".opensymphony/openhands".to_string(),
                agent: agent(),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
            },
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .with_run_timeout(Duration::from_secs(2));

        let outcome = runner
            .execute(&IssueSessionRequest {
                issue: IssueRef {
                    issue_id: "issue-1".to_string(),
                    identifier: "COE-253".to_string(),
                    title: "Runtime adapter".to_string(),
                },
                workspace: workspace.clone(),
                prompts: PromptSet {
                    full_prompt: "full prompt".to_string(),
                    continuation_prompt: "continue prompt".to_string(),
                },
            })
            .await?;

        assert_eq!(outcome.prompt_kind, opensymphony_domain::PromptKind::Fresh);
        assert_ne!(outcome.conversation_id, stale_conversation_id);

        let stale_record = server.conversation_record(&stale_conversation_id).await?;
        assert!(stale_record.messages.is_empty());
        assert_eq!(stale_record.run_count, 0);

        let fresh_record = server.conversation_record(&outcome.conversation_id).await?;
        let expected_working_dir = workspace.issue_workspace.display().to_string();
        let expected_persistence_dir = workspace.openhands_dir.display().to_string();
        assert_eq!(
            fresh_record.info.workspace.working_dir,
            expected_working_dir
        );
        assert_eq!(
            fresh_record.info.persistence_dir.as_deref(),
            Some(expected_persistence_dir.as_str())
        );
        let texts: Vec<_> = fresh_record
            .messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::Image { .. } => None,
            })
            .collect();
        assert_eq!(texts, vec!["full prompt"]);

        let manifest = ConversationManifest::load(&workspace.conversation_manifest_path)?
            .expect("conversation manifest should be rewritten");
        assert_eq!(
            manifest.reset_reason.as_deref(),
            Some("workspace_binding_changed")
        );

        Ok(())
    }

    #[tokio::test]
    async fn issue_session_runner_retries_full_prompt_after_initial_send_failure()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        let temp_dir = TempDir::new()?;
        let workspace = opensymphony_workspace::WorkspaceLayout::new(temp_dir.path(), "COE-253")?;
        workspace.create()?;

        let runner = IssueSessionRunner::new(
            client.clone(),
            opensymphony_openhands::ConversationConfig {
                runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
                persistence_dir_relative: ".opensymphony/openhands".to_string(),
                agent: agent(),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
            },
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .with_run_timeout(Duration::from_secs(2));

        let request = IssueSessionRequest {
            issue: IssueRef {
                issue_id: "issue-1".to_string(),
                identifier: "COE-253".to_string(),
                title: "Runtime adapter".to_string(),
            },
            workspace: workspace.clone(),
            prompts: PromptSet {
                full_prompt: "full prompt".to_string(),
                continuation_prompt: "continue prompt".to_string(),
            },
        };

        server.fail_next_message_posts(1).await;
        let error = runner
            .execute(&request)
            .await
            .expect_err("first prompt submission should fail");
        assert!(matches!(error, OpenHandsError::HttpStatus { .. }));
        assert!(
            ConversationManifest::load(&workspace.conversation_manifest_path)?.is_none(),
            "fresh manifests should not persist before the first prompt is accepted"
        );

        let outcome = runner.execute(&request).await?;
        assert_eq!(outcome.prompt_kind, opensymphony_domain::PromptKind::Fresh);

        let manifest = ConversationManifest::load(&workspace.conversation_manifest_path)?
            .expect("successful retry should persist the fresh manifest");
        assert_eq!(manifest.conversation_id, outcome.conversation_id);

        let record = server.conversation_record(&outcome.conversation_id).await?;
        let texts: Vec<_> = record
            .messages
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                ContentBlock::Text { text } => Some(text.as_str()),
                ContentBlock::Image { .. } => None,
            })
            .collect();
        assert_eq!(texts, vec!["full prompt"]);

        Ok(())
    }

    #[tokio::test]
    async fn issue_session_runner_resets_corrupted_manifest()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        let temp_dir = TempDir::new()?;
        let workspace = opensymphony_workspace::WorkspaceLayout::new(temp_dir.path(), "COE-253")?;
        workspace.create()?;
        std::fs::write(&workspace.conversation_manifest_path, "{not json")?;

        let runner = IssueSessionRunner::new(
            client.clone(),
            opensymphony_openhands::ConversationConfig {
                runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
                persistence_dir_relative: ".opensymphony/openhands".to_string(),
                agent: agent(),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
            },
            WebSocketConfig {
                ready_timeout_ms: 1_000,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .with_run_timeout(Duration::from_secs(2));

        let outcome = runner
            .execute(&IssueSessionRequest {
                issue: IssueRef {
                    issue_id: "issue-1".to_string(),
                    identifier: "COE-253".to_string(),
                    title: "Runtime adapter".to_string(),
                },
                workspace: workspace.clone(),
                prompts: PromptSet {
                    full_prompt: "full prompt".to_string(),
                    continuation_prompt: "continue prompt".to_string(),
                },
            })
            .await?;

        assert_eq!(outcome.prompt_kind, opensymphony_domain::PromptKind::Fresh);
        let manifest = ConversationManifest::load(&workspace.conversation_manifest_path)?
            .expect("corrupted manifest should be replaced");
        assert_eq!(manifest.reset_reason.as_deref(), Some("corrupted_manifest"));

        Ok(())
    }

    #[tokio::test]
    async fn attached_stream_close_interrupts_reconnect_readiness_wait()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = FakeOpenHandsServer::start().await?;
        let client = server.client();
        client
            .create_conversation(&CreateConversationRequest {
                agent: agent(),
                workspace: OpenHandsWorkspace {
                    working_dir: "/tmp/test".to_string(),
                    kind: None,
                    extra: Map::new(),
                },
                conversation_id: Some("conv-ready-blackhole".to_string()),
                persistence_dir: Some("/tmp/test/.opensymphony/openhands".to_string()),
                confirmation_policy: ConfirmationPolicy::never_confirm(),
                initial_message: None,
                max_iterations: 500,
                stuck_detection: true,
                autotitle: false,
                hook_config: None,
                plugins: Vec::new(),
                secrets: HashMap::new().into_iter().collect(),
                tool_module_qualnames: HashMap::new().into_iter().collect(),
            })
            .await?;

        server
            .enqueue_run(
                "conv-ready-blackhole",
                ScriptedRun::new(vec![
                    RunStep::Emit {
                        event: execution_status_event(RemoteExecutionStatus::Running),
                        delay: Duration::from_millis(5),
                    },
                    RunStep::Disconnect {
                        delay: Duration::from_millis(5),
                    },
                ]),
            )
            .await?;

        let attached = AttachedConversation::attach(
            client.clone(),
            "conv-ready-blackhole",
            WebSocketConfig {
                ready_timeout_ms: 200,
                reconnect_initial_ms: 10,
                reconnect_max_ms: 20,
                poll_interval_ms: 10,
            },
        )
        .await?;

        server
            .enqueue_ready_event_delay("conv-ready-blackhole", Duration::from_secs(1))
            .await?;
        client.run_conversation("conv-ready-blackhole").await?;

        tokio::time::sleep(Duration::from_millis(50)).await;
        tokio::time::timeout(Duration::from_millis(150), attached.close()).await??;
        Ok(())
    }
}
