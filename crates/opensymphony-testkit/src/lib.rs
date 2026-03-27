use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use opensymphony_openhands::{
    AgentConfig, ConfirmationPolicy, Conversation, ConversationCreateRequest,
    ConversationStateUpdatePayload, EventEnvelope, KnownEvent, SearchConversationEventsResponse,
    SendMessageRequest, WorkspaceConfig,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Mutex, broadcast},
    task::JoinHandle,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct FakeOpenHandsConfig {
    pub search_page_size: usize,
    pub run_terminal_status: &'static str,
    pub initial_execution_status: &'static str,
}

impl Default for FakeOpenHandsConfig {
    fn default() -> Self {
        Self {
            search_page_size: 2,
            run_terminal_status: "finished",
            initial_execution_status: "idle",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FakeConversationBuilder {
    workspace: WorkspaceConfig,
    persistence_dir: String,
    max_iterations: u32,
    stuck_detection: bool,
    execution_status: String,
    confirmation_policy: ConfirmationPolicy,
    agent: AgentConfig,
}

impl FakeConversationBuilder {
    pub fn from_request(request: &ConversationCreateRequest) -> Self {
        Self {
            workspace: request.workspace.clone(),
            persistence_dir: request.persistence_dir.clone(),
            max_iterations: request.max_iterations,
            stuck_detection: request.stuck_detection,
            execution_status: "idle".to_string(),
            confirmation_policy: request.confirmation_policy.clone(),
            agent: request.agent.clone(),
        }
    }

    pub fn execution_status(mut self, execution_status: impl Into<String>) -> Self {
        self.execution_status = execution_status.into();
        self
    }

    pub fn build(self, conversation_id: Uuid) -> Conversation {
        Conversation {
            conversation_id,
            workspace: self.workspace,
            persistence_dir: self.persistence_dir,
            max_iterations: self.max_iterations,
            stuck_detection: self.stuck_detection,
            execution_status: self.execution_status,
            confirmation_policy: self.confirmation_policy,
            agent: self.agent,
            stats: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FakeEventStreamBuilder {
    base_timestamp: chrono::DateTime<Utc>,
}

impl Default for FakeEventStreamBuilder {
    fn default() -> Self {
        Self {
            base_timestamp: Utc::now(),
        }
    }
}

impl FakeEventStreamBuilder {
    pub fn new(base_timestamp: chrono::DateTime<Utc>) -> Self {
        Self { base_timestamp }
    }

    pub fn custom_at(
        &self,
        id: impl Into<String>,
        offset_ms: i64,
        source: impl Into<String>,
        kind: impl Into<String>,
        payload: Value,
    ) -> EventEnvelope {
        EventEnvelope::new(
            id,
            self.base_timestamp + chrono::Duration::milliseconds(offset_ms),
            source,
            kind,
            payload,
        )
    }

    pub fn state_update_at(
        &self,
        id: impl Into<String>,
        offset_ms: i64,
        execution_status: impl Into<String>,
    ) -> EventEnvelope {
        let execution_status = execution_status.into();
        self.custom_at(
            id,
            offset_ms,
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": execution_status,
                "state_delta": {
                    "execution_status": execution_status,
                },
            }),
        )
    }

    pub fn llm_completion_at(
        &self,
        id: impl Into<String>,
        offset_ms: i64,
        model: impl Into<String>,
        tokens: u64,
    ) -> EventEnvelope {
        self.custom_at(
            id,
            offset_ms,
            "llm",
            "LLMCompletionLogEvent",
            json!({
                "model": model.into(),
                "tokens": tokens,
            }),
        )
    }

    pub fn conversation_error_at(
        &self,
        id: impl Into<String>,
        offset_ms: i64,
        message: impl Into<String>,
    ) -> EventEnvelope {
        self.custom_at(
            id,
            offset_ms,
            "runtime",
            "ConversationErrorEvent",
            json!({
                "message": message.into(),
            }),
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct FakeSearchScript {
    responses: Vec<SearchConversationEventsResponse>,
}

impl FakeSearchScript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn response(mut self, events: Vec<EventEnvelope>) -> Self {
        self.responses.push(SearchConversationEventsResponse {
            events,
            next_page_id: None,
        });
        self
    }

    pub fn paged_response(
        mut self,
        events: Vec<EventEnvelope>,
        next_page_id: Option<String>,
    ) -> Self {
        self.responses.push(SearchConversationEventsResponse {
            events,
            next_page_id,
        });
        self
    }
}

#[derive(Debug, Clone)]
pub enum FakeSocketAction {
    Event(EventEnvelope),
    Text(String),
    Ping(Vec<u8>),
    Close,
}

#[derive(Debug, Clone, Default)]
pub struct FakeSocketScript {
    actions: Vec<FakeSocketAction>,
}

impl FakeSocketScript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn event(mut self, event: EventEnvelope) -> Self {
        self.actions.push(FakeSocketAction::Event(event));
        self
    }

    pub fn text(mut self, payload: impl Into<String>) -> Self {
        self.actions.push(FakeSocketAction::Text(payload.into()));
        self
    }

    pub fn ping(mut self, payload: impl Into<Vec<u8>>) -> Self {
        self.actions.push(FakeSocketAction::Ping(payload.into()));
        self
    }

    pub fn close(mut self) -> Self {
        self.actions.push(FakeSocketAction::Close);
        self
    }
}

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    conversations: HashMap<Uuid, FakeConversation>,
    conversation_get_not_found: HashMap<Uuid, usize>,
    search_page_size: usize,
    run_terminal_status: String,
    initial_execution_status: String,
    next_event_index: u64,
}

struct FakeConversation {
    summary: Conversation,
    events: Vec<EventEnvelope>,
    sender: broadcast::Sender<EventEnvelope>,
    control_sender: broadcast::Sender<SocketControl>,
    scripted_search_responses: VecDeque<SearchConversationEventsResponse>,
    socket_scripts: VecDeque<FakeSocketScript>,
}

#[derive(Clone, Debug)]
enum SocketControl {
    Close,
}

pub struct FakeOpenHandsServer {
    base_url: String,
    state: AppState,
    task: JoinHandle<()>,
}

impl FakeOpenHandsServer {
    pub async fn start() -> std::io::Result<Self> {
        Self::start_with_config(FakeOpenHandsConfig::default()).await
    }

    pub async fn start_with_config(config: FakeOpenHandsConfig) -> std::io::Result<Self> {
        let state = AppState {
            inner: Arc::new(Mutex::new(Inner {
                conversations: HashMap::new(),
                conversation_get_not_found: HashMap::new(),
                search_page_size: config.search_page_size,
                run_terminal_status: config.run_terminal_status.to_string(),
                initial_execution_status: config.initial_execution_status.to_string(),
                next_event_index: 1,
            })),
        };

        let app = Router::new()
            .route("/openapi.json", get(openapi))
            .route("/api/conversations", post(create_conversation))
            .route(
                "/api/conversations/{conversation_id}",
                get(get_conversation).delete(delete_conversation),
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
            .route("/sockets/events/{conversation_id}", get(events_socket))
            .with_state(state.clone());

        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("fake OpenHands server should stay up");
        });

        Ok(Self {
            base_url: format!("http://{address}"),
            state,
            task,
        })
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub async fn emit_state_update(
        &self,
        conversation_id: Uuid,
        execution_status: impl Into<String>,
    ) -> Result<(), FakeServerError> {
        let execution_status = execution_status.into();
        let event = {
            let mut inner = self.state.inner.lock().await;
            let id = next_event_id(&mut inner);
            EventEnvelope::new(
                id,
                Utc::now(),
                "runtime",
                "ConversationStateUpdateEvent",
                json!({
                    "execution_status": execution_status,
                    "state_delta": {
                        "execution_status": execution_status,
                    },
                }),
            )
        };

        self.insert_event(conversation_id, event).await
    }

    pub async fn insert_event(
        &self,
        conversation_id: Uuid,
        event: EventEnvelope,
    ) -> Result<(), FakeServerError> {
        let mut inner = self.state.inner.lock().await;
        let conversation = inner
            .conversations
            .get_mut(&conversation_id)
            .ok_or(FakeServerError::ConversationNotFound(conversation_id))?;
        apply_event_to_conversation(conversation, event);
        Ok(())
    }

    pub async fn event_count(&self, conversation_id: Uuid) -> Result<usize, FakeServerError> {
        let inner = self.state.inner.lock().await;
        let conversation = inner
            .conversations
            .get(&conversation_id)
            .ok_or(FakeServerError::ConversationNotFound(conversation_id))?;
        Ok(conversation.events.len())
    }

    pub async fn fail_next_conversation_gets(
        &self,
        conversation_id: Uuid,
        count: usize,
    ) -> Result<(), FakeServerError> {
        let mut inner = self.state.inner.lock().await;
        if !inner.conversations.contains_key(&conversation_id) {
            return Err(FakeServerError::ConversationNotFound(conversation_id));
        }
        if count == 0 {
            inner.conversation_get_not_found.remove(&conversation_id);
        } else {
            inner
                .conversation_get_not_found
                .insert(conversation_id, count);
        }
        Ok(())
    }

    pub async fn drop_websocket_connections(
        &self,
        conversation_id: Uuid,
    ) -> Result<(), FakeServerError> {
        let inner = self.state.inner.lock().await;
        let conversation = inner
            .conversations
            .get(&conversation_id)
            .ok_or(FakeServerError::ConversationNotFound(conversation_id))?;
        let _ = conversation.control_sender.send(SocketControl::Close);
        Ok(())
    }

    pub async fn script_search_responses(
        &self,
        conversation_id: Uuid,
        script: FakeSearchScript,
    ) -> Result<(), FakeServerError> {
        let mut inner = self.state.inner.lock().await;
        let conversation = inner
            .conversations
            .get_mut(&conversation_id)
            .ok_or(FakeServerError::ConversationNotFound(conversation_id))?;
        conversation.scripted_search_responses = script.responses.into();
        Ok(())
    }

    pub async fn script_socket_connections(
        &self,
        conversation_id: Uuid,
        scripts: Vec<FakeSocketScript>,
    ) -> Result<(), FakeServerError> {
        let mut inner = self.state.inner.lock().await;
        let conversation = inner
            .conversations
            .get_mut(&conversation_id)
            .ok_or(FakeServerError::ConversationNotFound(conversation_id))?;
        conversation.socket_scripts = scripts.into();
        Ok(())
    }
}

impl Drop for FakeOpenHandsServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FakeServerError {
    #[error("conversation not found: {0}")]
    ConversationNotFound(Uuid),
}

async fn openapi() -> Json<Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Fake OpenHands agent-server",
            "version": "0.1.0",
        }
    }))
}

async fn create_conversation(
    State(state): State<AppState>,
    Json(request): Json<ConversationCreateRequest>,
) -> Result<Json<Conversation>, StatusCode> {
    let mut inner = state.inner.lock().await;
    if let Some(existing) = inner.conversations.get(&request.conversation_id) {
        return Ok(Json(existing.summary.clone()));
    }

    let summary = FakeConversationBuilder::from_request(&request)
        .execution_status(inner.initial_execution_status.clone())
        .build(request.conversation_id);

    let (sender, _) = broadcast::channel(32);
    let (control_sender, _) = broadcast::channel(8);
    let ready_event = EventEnvelope::new(
        next_event_id(&mut inner),
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        json!({
            "execution_status": summary.execution_status.clone(),
            "state_delta": {
                "execution_status": summary.execution_status.clone(),
            },
        }),
    );

    inner.conversations.insert(
        summary.conversation_id,
        FakeConversation {
            summary: summary.clone(),
            events: vec![ready_event],
            sender,
            control_sender,
            scripted_search_responses: VecDeque::new(),
            socket_scripts: VecDeque::new(),
        },
    );

    Ok(Json(summary))
}

async fn get_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    let mut inner = state.inner.lock().await;
    let not_found = match inner.conversation_get_not_found.get_mut(&conversation_id) {
        Some(remaining) if *remaining > 0 => {
            *remaining -= 1;
            true
        }
        _ => false,
    };
    if not_found
        && inner
            .conversation_get_not_found
            .get(&conversation_id)
            .copied()
            .unwrap_or_default()
            == 0
    {
        inner.conversation_get_not_found.remove(&conversation_id);
    }
    if not_found {
        return Err(StatusCode::NOT_FOUND);
    }

    let conversation = inner
        .conversations
        .get(&conversation_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation.summary.clone()))
}

async fn delete_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let mut inner = state.inner.lock().await;
    if inner.conversations.remove(&conversation_id).is_none() {
        return Err(StatusCode::NOT_FOUND);
    }
    inner.conversation_get_not_found.remove(&conversation_id);
    Ok(Json(json!({ "success": true })))
}

async fn send_message(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<Value>, StatusCode> {
    let mut inner = state.inner.lock().await;
    let event = EventEnvelope::new(
        next_event_id(&mut inner),
        Utc::now(),
        "user",
        "MessageEvent",
        serde_json::to_value(request).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );

    let conversation = inner
        .conversations
        .get_mut(&conversation_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    apply_event_to_conversation(conversation, event);
    Ok(Json(json!({ "success": true })))
}

async fn run_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
) -> Result<Json<Value>, StatusCode> {
    let mut inner = state.inner.lock().await;
    let already_running = inner
        .conversations
        .get(&conversation_id)
        .map(|conversation| run_in_progress(&conversation.summary.execution_status))
        .ok_or(StatusCode::NOT_FOUND)?;
    if already_running {
        return Err(StatusCode::CONFLICT);
    }
    let terminal_status = inner.run_terminal_status.clone();
    let running_event = EventEnvelope::new(
        next_event_id(&mut inner),
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
    let completion_event = EventEnvelope::new(
        next_event_id(&mut inner),
        Utc::now(),
        "llm",
        "LLMCompletionLogEvent",
        json!({
            "model": "fake-model",
            "tokens": 42,
        }),
    );
    let finished_event = EventEnvelope::new(
        next_event_id(&mut inner),
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        json!({
            "execution_status": terminal_status.clone(),
            "state_delta": {
                "execution_status": terminal_status,
            },
        }),
    );

    let conversation = inner
        .conversations
        .get_mut(&conversation_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    apply_event_to_conversation(conversation, running_event);
    apply_event_to_conversation(conversation, completion_event);
    apply_event_to_conversation(conversation, finished_event);
    Ok(Json(json!({ "success": true })))
}

#[derive(Deserialize)]
struct SearchQuery {
    page_id: Option<String>,
}

async fn search_events(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchConversationEventsResponse>, StatusCode> {
    let offset = query
        .page_id
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let mut inner = state.inner.lock().await;
    let page_size = inner.search_page_size;
    let conversation = inner
        .conversations
        .get_mut(&conversation_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    if let Some(response) = conversation.scripted_search_responses.pop_front() {
        return Ok(Json(response));
    }
    let page = conversation
        .events
        .iter()
        .skip(offset)
        .take(page_size)
        .cloned()
        .collect::<Vec<_>>();
    let next_offset = offset + page.len();
    let next_page_id = (next_offset < conversation.events.len()).then(|| next_offset.to_string());

    Ok(Json(SearchConversationEventsResponse {
        events: page,
        next_page_id,
    }))
}

async fn events_socket(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
    websocket: WebSocketUpgrade,
) -> Result<impl IntoResponse, StatusCode> {
    {
        let inner = state.inner.lock().await;
        if !inner.conversations.contains_key(&conversation_id) {
            return Err(StatusCode::NOT_FOUND);
        }
    }

    Ok(websocket.on_upgrade(move |socket| handle_socket(state, conversation_id, socket)))
}

async fn handle_socket(state: AppState, conversation_id: Uuid, mut socket: WebSocket) {
    let (mut receiver, mut control_receiver, ready_event, script) = {
        let mut inner = state.inner.lock().await;
        let conversation = match inner.conversations.get_mut(&conversation_id) {
            Some(conversation) => conversation,
            None => return,
        };
        (
            conversation.sender.subscribe(),
            conversation.control_sender.subscribe(),
            latest_state_event(conversation),
            conversation.socket_scripts.pop_front(),
        )
    };

    if let Some(script) = script {
        for action in script.actions {
            match action {
                FakeSocketAction::Event(event) => {
                    let payload = match serde_json::to_string(&event) {
                        Ok(payload) => payload,
                        Err(_) => continue,
                    };
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
                FakeSocketAction::Text(payload) => {
                    if socket.send(Message::Text(payload.into())).await.is_err() {
                        return;
                    }
                }
                FakeSocketAction::Ping(payload) => {
                    if socket.send(Message::Ping(payload.into())).await.is_err() {
                        return;
                    }
                }
                FakeSocketAction::Close => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
        }
    } else if socket
        .send(Message::Text(
            serde_json::to_string(&ready_event)
                .expect("serializing ready event should succeed")
                .into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            received = receiver.recv() => {
                let event = match received {
                    Ok(event) => event,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let payload = match serde_json::to_string(&event) {
                    Ok(payload) => payload,
                    Err(_) => continue,
                };

                if socket.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
            control = control_receiver.recv() => {
                match control {
                    Ok(SocketControl::Close) => {
                        let _ = socket.send(Message::Close(None)).await;
                        break;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

fn latest_state_event(conversation: &FakeConversation) -> EventEnvelope {
    conversation
        .events
        .iter()
        .filter(|event| event.kind == "ConversationStateUpdateEvent")
        .max_by(|left, right| compare_events(left, right))
        .cloned()
        .unwrap_or_else(|| {
            EventEnvelope::state_update("ws-ready", conversation.summary.execution_status.clone())
        })
}

fn apply_event_to_conversation(conversation: &mut FakeConversation, event: EventEnvelope) {
    conversation.events.push(event.clone());
    refresh_summary_state(conversation);
    let _ = conversation.sender.send(event);
}

fn refresh_summary_state(conversation: &mut FakeConversation) {
    if let Some(execution_status) = conversation
        .events
        .iter()
        .filter_map(|event| match KnownEvent::from_envelope(event) {
            KnownEvent::ConversationStateUpdate(ConversationStateUpdatePayload {
                execution_status: Some(execution_status),
                ..
            }) => Some((event.timestamp, event.id.as_str(), execution_status)),
            _ => None,
        })
        .max_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(right.1)))
        .map(|(_, _, execution_status)| execution_status)
    {
        conversation.summary.execution_status = execution_status;
    }
}

fn compare_events(left: &EventEnvelope, right: &EventEnvelope) -> std::cmp::Ordering {
    left.timestamp
        .cmp(&right.timestamp)
        .then_with(|| left.id.cmp(&right.id))
}

fn next_event_id(inner: &mut Inner) -> String {
    let current = inner.next_event_index;
    inner.next_event_index += 1;
    format!("evt-{current}")
}

fn run_in_progress(status: &str) -> bool {
    !matches!(status, "idle" | "finished" | "error" | "stuck")
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::{FakeConversationBuilder, FakeEventStreamBuilder};
    use opensymphony_openhands::{ConversationCreateRequest, KnownEvent};

    #[test]
    fn conversation_builder_preserves_request_fields_and_overrides_status() {
        let request = ConversationCreateRequest::doctor_probe(
            "/tmp/workspace",
            "/tmp/workspace/.opensymphony/openhands",
            Some("gpt-test".to_string()),
            Some("secret".to_string()),
        );

        let conversation = FakeConversationBuilder::from_request(&request)
            .execution_status("running")
            .build(request.conversation_id);

        assert_eq!(conversation.conversation_id, request.conversation_id);
        assert_eq!(conversation.workspace, request.workspace);
        assert_eq!(conversation.persistence_dir, request.persistence_dir);
        assert_eq!(conversation.max_iterations, request.max_iterations);
        assert_eq!(conversation.stuck_detection, request.stuck_detection);
        assert_eq!(conversation.execution_status, "running");
        assert_eq!(
            conversation.confirmation_policy,
            request.confirmation_policy
        );
        assert_eq!(conversation.agent, request.agent);
    }

    #[test]
    fn event_stream_builder_emits_deterministic_offsets() {
        let base = chrono::Utc
            .with_ymd_and_hms(2026, 3, 23, 12, 0, 0)
            .single()
            .expect("timestamp should be valid");
        let fixtures = FakeEventStreamBuilder::new(base);
        let state = fixtures.state_update_at("evt-state", -1_000, "queued");
        let log = fixtures.llm_completion_at("evt-log", 2_000, "fake-model", 42);

        assert_eq!(
            state.timestamp,
            base - chrono::Duration::milliseconds(1_000)
        );
        assert_eq!(log.timestamp, base + chrono::Duration::milliseconds(2_000));
        assert!(matches!(
            KnownEvent::from_envelope(&state),
            KnownEvent::ConversationStateUpdate(_)
        ));
        assert_eq!(log.kind, "LLMCompletionLogEvent");
        assert_eq!(log.payload["tokens"], 42);
    }
}
