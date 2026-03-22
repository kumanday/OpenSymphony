use std::{collections::HashMap, sync::Arc};

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
    Conversation, ConversationCreateRequest, ConversationStateUpdatePayload, EventEnvelope,
    KnownEvent, SearchConversationEventsResponse, SendMessageRequest,
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
}

impl Default for FakeOpenHandsConfig {
    fn default() -> Self {
        Self {
            search_page_size: 2,
        }
    }
}

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    conversations: HashMap<Uuid, FakeConversation>,
    search_page_size: usize,
    next_event_index: u64,
}

struct FakeConversation {
    summary: Conversation,
    events: Vec<EventEnvelope>,
    sender: broadcast::Sender<EventEnvelope>,
    control_sender: broadcast::Sender<SocketControl>,
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
                search_page_size: config.search_page_size,
                next_event_index: 1,
            })),
        };

        let app = Router::new()
            .route("/openapi.json", get(openapi))
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
    let summary = Conversation {
        conversation_id: request.conversation_id,
        workspace: request.workspace,
        persistence_dir: request.persistence_dir,
        max_iterations: request.max_iterations,
        stuck_detection: request.stuck_detection,
        execution_status: "idle".to_string(),
        confirmation_policy: request.confirmation_policy,
        agent: request.agent,
    };

    let (sender, _) = broadcast::channel(32);
    let (control_sender, _) = broadcast::channel(8);
    let ready_event = EventEnvelope::new(
        next_event_id(&mut inner),
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        json!({
            "execution_status": "idle",
            "state_delta": {
                "execution_status": "idle",
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
        },
    );

    Ok(Json(summary))
}

async fn get_conversation(
    State(state): State<AppState>,
    Path(conversation_id): Path<Uuid>,
) -> Result<Json<Conversation>, StatusCode> {
    let inner = state.inner.lock().await;
    let conversation = inner
        .conversations
        .get(&conversation_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(conversation.summary.clone()))
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
            "execution_status": "finished",
            "state_delta": {
                "execution_status": "finished",
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
    let inner = state.inner.lock().await;
    let conversation = inner
        .conversations
        .get(&conversation_id)
        .ok_or(StatusCode::NOT_FOUND)?;
    let offset = query
        .page_id
        .as_deref()
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let page_size = inner.search_page_size;
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
    let (mut receiver, mut control_receiver, ready_event) = {
        let inner = state.inner.lock().await;
        let conversation = match inner.conversations.get(&conversation_id) {
            Some(conversation) => conversation,
            None => return,
        };
        (
            conversation.sender.subscribe(),
            conversation.control_sender.subscribe(),
            latest_state_event(conversation),
        )
    };

    if socket
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
