use std::time::Duration;

use futures_util::StreamExt;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::time::{timeout_at, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::debug;
use url::Url;
use uuid::Uuid;

use crate::events::{ConversationStateMirror, EventCache, KnownEvent};
use crate::models::{
    Conversation, ConversationCreateRequest, EventEnvelope, SearchConversationEventsResponse,
    SendMessageRequest,
};

#[derive(Debug, Clone)]
pub struct TransportConfig {
    pub base_url: String,
    pub session_api_key: Option<String>,
}

impl TransportConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            session_api_key: None,
        }
    }

    fn endpoint(&self, suffix: &str) -> Result<Url, OpenHandsError> {
        let mut url = Url::parse(&self.base_url)?;
        let base_path = url.path().trim_end_matches('/');
        let path = format!("{base_path}{suffix}");
        let normalized = if path.is_empty() {
            "/".to_string()
        } else {
            path
        };
        url.set_path(&normalized);
        if let Some(session_api_key) = &self.session_api_key {
            url.query_pairs_mut()
                .append_pair("session_api_key", session_api_key);
        }
        Ok(url)
    }

    fn ws_url(&self, conversation_id: Uuid) -> Result<Url, OpenHandsError> {
        let mut url = Url::parse(&self.base_url)?;
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            other => return Err(OpenHandsError::UnsupportedScheme(other.to_string())),
        };
        url.set_scheme(scheme)
            .map_err(|_| OpenHandsError::UnsupportedScheme(scheme.to_string()))?;

        let base_path = url.path().trim_end_matches('/');
        let path = if base_path.is_empty() {
            format!("/sockets/events/{conversation_id}")
        } else {
            format!("{base_path}/sockets/events/{conversation_id}")
        };
        url.set_path(&path);
        if let Some(session_api_key) = &self.session_api_key {
            url.query_pairs_mut()
                .append_pair("session_api_key", session_api_key);
        }
        Ok(url)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenHandsError {
    #[error("request failed: {0}")]
    Request(Box<reqwest::Error>),
    #[error("websocket failed: {0}")]
    WebSocket(Box<tokio_tungstenite::tungstenite::Error>),
    #[error("url parsing failed: {0}")]
    Url(Box<url::ParseError>),
    #[error("json decoding failed: {0}")]
    Json(Box<serde_json::Error>),
    #[error("websocket event decoding failed: {error}; payload prefix: {snippet}")]
    MalformedWebSocketEvent {
        #[source]
        error: Box<serde_json::Error>,
        snippet: String,
    },
    #[error("unexpected HTTP status {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },
    #[error("websocket readiness timed out after {0:?}")]
    ReadinessTimeout(Duration),
    #[error("websocket closed before readiness")]
    WebSocketClosed,
    #[error("unsupported base URL scheme: {0}")]
    UnsupportedScheme(String),
}

impl From<reqwest::Error> for OpenHandsError {
    fn from(error: reqwest::Error) -> Self {
        Self::Request(Box::new(error))
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for OpenHandsError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(error))
    }
}

impl From<url::ParseError> for OpenHandsError {
    fn from(error: url::ParseError) -> Self {
        Self::Url(Box::new(error))
    }
}

impl From<serde_json::Error> for OpenHandsError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(Box::new(error))
    }
}

#[derive(Debug, Clone)]
pub struct OpenHandsProbeResult {
    pub conversation: Conversation,
    pub ready_event: EventEnvelope,
    pub event_cache: EventCache,
    pub state_mirror: ConversationStateMirror,
}

#[derive(Clone)]
pub struct OpenHandsClient {
    http: reqwest::Client,
    transport: TransportConfig,
}

impl OpenHandsClient {
    pub fn new(transport: TransportConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            transport,
        }
    }

    pub async fn openapi_probe(&self) -> Result<(), OpenHandsError> {
        let response = self
            .http
            .get(self.transport.endpoint("/openapi.json")?)
            .send()
            .await?;
        ensure_success(response).await.map(|_| ())
    }

    pub async fn create_conversation(
        &self,
        request: &ConversationCreateRequest,
    ) -> Result<Conversation, OpenHandsError> {
        let response = self
            .http
            .post(self.transport.endpoint("/api/conversations")?)
            .json(request)
            .send()
            .await?;
        decode_json(response).await
    }

    pub async fn get_conversation(
        &self,
        conversation_id: Uuid,
    ) -> Result<Conversation, OpenHandsError> {
        let response = self
            .http
            .get(
                self.transport
                    .endpoint(&format!("/api/conversations/{conversation_id}"))?,
            )
            .send()
            .await?;
        decode_json(response).await
    }

    pub async fn send_message(
        &self,
        conversation_id: Uuid,
        request: &SendMessageRequest,
    ) -> Result<(), OpenHandsError> {
        let response = self
            .http
            .post(
                self.transport
                    .endpoint(&format!("/api/conversations/{conversation_id}/events"))?,
            )
            .json(request)
            .send()
            .await?;
        ensure_success(response).await.map(|_| ())
    }

    pub async fn run_conversation(&self, conversation_id: Uuid) -> Result<(), OpenHandsError> {
        let response = self
            .http
            .post(
                self.transport
                    .endpoint(&format!("/api/conversations/{conversation_id}/run"))?,
            )
            .json(&serde_json::json!({}))
            .send()
            .await?;
        ensure_success(response).await.map(|_| ())
    }

    pub async fn search_events_page(
        &self,
        conversation_id: Uuid,
        page_id: Option<&str>,
    ) -> Result<SearchConversationEventsResponse, OpenHandsError> {
        let mut url = self.transport.endpoint(&format!(
            "/api/conversations/{conversation_id}/events/search"
        ))?;
        if let Some(page_id) = page_id {
            url.query_pairs_mut().append_pair("page_id", page_id);
        }

        let response = self.http.get(url).send().await?;
        decode_json(response).await
    }

    pub async fn search_all_events(
        &self,
        conversation_id: Uuid,
    ) -> Result<EventCache, OpenHandsError> {
        let mut page_id: Option<String> = None;
        let mut cache = EventCache::new();
        loop {
            let page = self
                .search_events_page(conversation_id, page_id.as_deref())
                .await?;
            cache.extend(page.events);
            match page.next_page_id {
                Some(next_page_id) => page_id = Some(next_page_id),
                None => return Ok(cache),
            }
        }
    }

    pub async fn wait_for_readiness(
        &self,
        conversation_id: Uuid,
        wait_timeout: Duration,
    ) -> Result<EventEnvelope, OpenHandsError> {
        let ws_url = self.transport.ws_url(conversation_id)?;
        let (mut stream, _) = connect_async(ws_url.as_str()).await?;
        let deadline = Instant::now() + wait_timeout;

        loop {
            let next_message = timeout_at(deadline, stream.next())
                .await
                .map_err(|_| OpenHandsError::ReadinessTimeout(wait_timeout))?;

            match next_message {
                Some(Ok(Message::Text(payload))) => match parse_text_event(&payload) {
                    Ok(event)
                        if matches!(
                            KnownEvent::from_envelope(&event),
                            KnownEvent::ConversationStateUpdate(_)
                        ) =>
                    {
                        return Ok(event);
                    }
                    Ok(event) => {
                        debug!(event_kind = %event.kind, "ignoring non-readiness websocket event");
                    }
                    Err(error) => {
                        debug!(error = %error, "ignoring undecodable websocket text frame before readiness");
                    }
                },
                Some(Ok(Message::Binary(payload))) => match parse_binary_event(&payload) {
                    Ok(event)
                        if matches!(
                            KnownEvent::from_envelope(&event),
                            KnownEvent::ConversationStateUpdate(_)
                        ) =>
                    {
                        return Ok(event);
                    }
                    Ok(event) => {
                        debug!(event_kind = %event.kind, "ignoring non-readiness websocket event");
                    }
                    Err(error) => {
                        debug!(error = %error, "ignoring undecodable websocket binary frame before readiness");
                    }
                },
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                    debug!("ignoring websocket control frame before readiness");
                }
                Some(Ok(Message::Frame(_))) => {
                    debug!("ignoring raw websocket frame before readiness");
                }
                Some(Ok(Message::Close(_))) | None => return Err(OpenHandsError::WebSocketClosed),
                Some(Err(error)) => return Err(OpenHandsError::WebSocket(Box::new(error))),
            }
        }
    }

    pub async fn run_probe(
        &self,
        request: &ConversationCreateRequest,
        wait_timeout: Duration,
    ) -> Result<OpenHandsProbeResult, OpenHandsError> {
        let conversation = self.create_conversation(request).await?;
        let conversation = self.get_conversation(conversation.conversation_id).await?;
        let ready_event = self
            .wait_for_readiness(conversation.conversation_id, wait_timeout)
            .await?;
        let event_cache = self.search_all_events(conversation.conversation_id).await?;
        let mut state_mirror = ConversationStateMirror::default();
        state_mirror.apply_conversation(&conversation);
        for event in event_cache.items() {
            state_mirror.apply_event(event);
        }

        Ok(OpenHandsProbeResult {
            conversation,
            ready_event,
            event_cache,
            state_mirror,
        })
    }
}

async fn ensure_success(response: reqwest::Response) -> Result<Value, OpenHandsError> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(OpenHandsError::HttpStatus { status, body });
    }

    serde_json::from_str(&body).map_err(OpenHandsError::from)
}

async fn decode_json<T>(response: reqwest::Response) -> Result<T, OpenHandsError>
where
    T: DeserializeOwned,
{
    let value = ensure_success(response).await?;
    serde_json::from_value(value).map_err(OpenHandsError::from)
}

fn parse_text_event(payload: &str) -> Result<EventEnvelope, OpenHandsError> {
    serde_json::from_str(payload).map_err(|error| OpenHandsError::MalformedWebSocketEvent {
        error: Box::new(error),
        snippet: payload.chars().take(160).collect(),
    })
}

fn parse_binary_event(payload: &[u8]) -> Result<EventEnvelope, OpenHandsError> {
    serde_json::from_slice(payload).map_err(|error| OpenHandsError::MalformedWebSocketEvent {
        error: Box::new(error),
        snippet: String::from_utf8_lossy(payload).chars().take(160).collect(),
    })
}
