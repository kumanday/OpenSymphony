use std::{cmp::Ordering, collections::VecDeque, time::Duration};

use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE},
    RequestBuilder,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::{
    net::TcpStream,
    time::{sleep, timeout, timeout_at, Instant},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use tracing::debug;
use url::Url;
use uuid::Uuid;

use crate::events::{ConversationStateMirror, EventCache, KnownEvent, TerminalExecutionStatus};
use crate::models::{
    AcceptedResponse, Conversation, ConversationCreateRequest, ConversationRunRequest,
    EventEnvelope, SearchConversationEventsResponse, SendMessageRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyAuth {
    name: String,
    value: String,
}

impl ApiKeyAuth {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpAuth {
    None,
    QueryParam(ApiKeyAuth),
    Header(ApiKeyAuth),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebSocketAuth {
    None,
    QueryParam(ApiKeyAuth),
    Header(ApiKeyAuth),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub http: HttpAuth,
    pub websocket: WebSocketAuth,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self::none()
    }
}

impl AuthConfig {
    pub fn none() -> Self {
        Self {
            http: HttpAuth::None,
            websocket: WebSocketAuth::None,
        }
    }

    pub fn query_param_api_key(name: impl Into<String>, value: impl Into<String>) -> Self {
        let key = ApiKeyAuth::new(name, value);
        Self {
            http: HttpAuth::QueryParam(key.clone()),
            websocket: WebSocketAuth::QueryParam(key),
        }
    }

    pub fn header_api_key(name: impl Into<String>, value: impl Into<String>) -> Self {
        let key = ApiKeyAuth::new(name, value);
        Self {
            http: HttpAuth::Header(key.clone()),
            websocket: WebSocketAuth::Header(key),
        }
    }

    pub fn header_api_key_with_websocket_query_fallback(
        header_name: impl Into<String>,
        websocket_query_param: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        let value = value.into();
        Self {
            http: HttpAuth::Header(ApiKeyAuth::new(header_name, value.clone())),
            websocket: WebSocketAuth::QueryParam(ApiKeyAuth::new(websocket_query_param, value)),
        }
    }

    fn apply_http_query(&self, url: &mut Url) {
        if let HttpAuth::QueryParam(key) = &self.http {
            url.query_pairs_mut().append_pair(key.name(), key.value());
        }
    }

    fn apply_websocket_query(&self, url: &mut Url) {
        if let WebSocketAuth::QueryParam(key) = &self.websocket {
            url.query_pairs_mut().append_pair(key.name(), key.value());
        }
    }

    fn apply_http_headers(
        &self,
        request: RequestBuilder,
    ) -> Result<RequestBuilder, OpenHandsError> {
        match &self.http {
            HttpAuth::Header(key) => Ok(request.header(
                parse_header_name(key.name())?,
                parse_header_value(key.value())?,
            )),
            _ => Ok(request),
        }
    }

    fn apply_websocket_headers(&self, headers: &mut HeaderMap) -> Result<(), OpenHandsError> {
        if let WebSocketAuth::Header(key) = &self.websocket {
            headers.insert(
                parse_header_name(key.name())?,
                parse_header_value(key.value())?,
            );
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportConfig {
    base_url: String,
    auth: AuthConfig,
}

impl TransportConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            auth: AuthConfig::default(),
        }
    }

    pub fn with_auth(mut self, auth: AuthConfig) -> Self {
        self.auth = auth;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn auth(&self) -> &AuthConfig {
        &self.auth
    }

    fn endpoint(&self, suffix: &str) -> Result<Url, OpenHandsError> {
        let mut url = self.parsed_base_url()?;
        let base_path = url.path().trim_end_matches('/');
        let path = format!("{base_path}{suffix}");
        let normalized = if path.is_empty() {
            "/".to_string()
        } else {
            path
        };
        url.set_path(&normalized);
        self.auth.apply_http_query(&mut url);
        Ok(url)
    }

    fn websocket_request(
        &self,
        conversation_id: Uuid,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, OpenHandsError> {
        let mut url = self.parsed_base_url()?;
        let scheme = match url.scheme() {
            "http" => "ws",
            "https" => "wss",
            other => {
                return Err(OpenHandsError::invalid_configuration(format!(
                    "unsupported base URL scheme `{other}`"
                )));
            }
        };
        url.set_scheme(scheme).map_err(|_| {
            OpenHandsError::invalid_configuration(format!(
                "failed to apply websocket scheme `{scheme}`"
            ))
        })?;

        let base_path = url.path().trim_end_matches('/');
        let path = if base_path.is_empty() {
            format!("/sockets/events/{conversation_id}")
        } else {
            format!("{base_path}/sockets/events/{conversation_id}")
        };
        url.set_path(&path);
        self.auth.apply_websocket_query(&mut url);

        let mut request = url.as_str().into_client_request().map_err(|error| {
            OpenHandsError::invalid_configuration(format!(
                "invalid websocket request `{url}`: {error}"
            ))
        })?;
        self.auth.apply_websocket_headers(request.headers_mut())?;
        Ok(request)
    }

    fn apply_http_auth(&self, request: RequestBuilder) -> Result<RequestBuilder, OpenHandsError> {
        self.auth.apply_http_headers(request)
    }

    fn parsed_base_url(&self) -> Result<Url, OpenHandsError> {
        Url::parse(&self.base_url).map_err(|error| {
            OpenHandsError::invalid_configuration(format!(
                "invalid base URL `{}`: {error}",
                self.base_url
            ))
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpenHandsError {
    #[error("invalid transport configuration: {detail}")]
    InvalidConfiguration { detail: String },
    #[error("{operation} transport failed: {detail}")]
    Transport {
        operation: &'static str,
        detail: String,
    },
    #[error("{operation} returned HTTP {status_code}: {body}")]
    HttpStatus {
        operation: &'static str,
        status_code: u16,
        body: String,
    },
    #[error("{operation} protocol error: {detail}")]
    Protocol {
        operation: &'static str,
        detail: String,
    },
    #[error("{operation} websocket failed: {detail}")]
    WebSocketTransport {
        operation: &'static str,
        detail: String,
    },
    #[error("websocket event decoding failed: {detail}; payload prefix: {snippet}")]
    MalformedWebSocketEvent { detail: String, snippet: String },
    #[error("websocket readiness timed out after {0:?}")]
    ReadinessTimeout(Duration),
    #[error("probe run activity was not observed after {0:?}")]
    ProbeActivityTimeout(Duration),
    #[error("probe run reported an unhealthy runtime: {0}")]
    ProbeRunUnhealthy(String),
    #[error("websocket closed before readiness")]
    WebSocketClosed,
    #[error("runtime stream reconnect exhausted after {attempts} attempt(s): {last_error}")]
    ReconnectExhausted { attempts: usize, last_error: String },
}

impl OpenHandsError {
    fn invalid_configuration(detail: impl Into<String>) -> Self {
        Self::InvalidConfiguration {
            detail: detail.into(),
        }
    }

    fn transport(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self::Transport {
            operation,
            detail: error.to_string(),
        }
    }

    fn protocol(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self::Protocol {
            operation,
            detail: error.to_string(),
        }
    }

    fn websocket_transport(operation: &'static str, error: impl std::fmt::Display) -> Self {
        Self::WebSocketTransport {
            operation,
            detail: error.to_string(),
        }
    }
}

type RuntimeSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;
const STREAM_READ_AHEAD_WINDOW: Duration = Duration::from_millis(5);
const UNREADY_EVENT_ID: &str = "runtime-stream-unready";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStreamConfig {
    pub readiness_timeout: Duration,
    pub reconnect_initial_backoff: Duration,
    pub reconnect_max_backoff: Duration,
    pub max_reconnect_attempts: usize,
}

impl Default for RuntimeStreamConfig {
    fn default() -> Self {
        Self {
            readiness_timeout: Duration::from_secs(30),
            reconnect_initial_backoff: Duration::from_secs(1),
            reconnect_max_backoff: Duration::from_secs(30),
            max_reconnect_attempts: 8,
        }
    }
}

pub struct RuntimeEventStream {
    client: OpenHandsClient,
    conversation_id: Uuid,
    config: RuntimeStreamConfig,
    socket: Option<RuntimeSocket>,
    conversation: Conversation,
    ready_event: EventEnvelope,
    event_cache: EventCache,
    state_mirror: ConversationStateMirror,
    pending_events: VecDeque<EventEnvelope>,
    reconnect_pending: bool,
}

impl RuntimeEventStream {
    fn new(
        client: OpenHandsClient,
        conversation_id: Uuid,
        config: RuntimeStreamConfig,
        conversation: Conversation,
    ) -> Self {
        let ready_event = EventEnvelope::state_update(UNREADY_EVENT_ID, "idle");
        let mut state_mirror = ConversationStateMirror::default();
        state_mirror.apply_conversation(&conversation);
        Self {
            client,
            conversation_id,
            config,
            socket: None,
            conversation,
            ready_event,
            event_cache: EventCache::new(),
            state_mirror,
            pending_events: VecDeque::new(),
            reconnect_pending: false,
        }
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    pub fn ready_event(&self) -> &EventEnvelope {
        &self.ready_event
    }

    pub fn event_cache(&self) -> &EventCache {
        &self.event_cache
    }

    pub fn state_mirror(&self) -> &ConversationStateMirror {
        &self.state_mirror
    }

    pub async fn next_event(&mut self) -> Result<Option<EventEnvelope>, OpenHandsError> {
        loop {
            if let Some(event) = self.poll_next_event_once().await? {
                return Ok(Some(event));
            }

            if self.socket.is_none() {
                return Ok(None);
            }
        }
    }

    async fn poll_next_event_once(&mut self) -> Result<Option<EventEnvelope>, OpenHandsError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }

        if self.reconnect_pending {
            self.reconnect_pending = false;
            self.reconnect().await?;

            if let Some(event) = self.pending_events.pop_front() {
                return Ok(Some(event));
            }
        }

        let stream_read = {
            let Some(socket) = self.socket.as_mut() else {
                return Ok(None);
            };
            read_next_socket_event(socket).await
        };

        match stream_read {
            StreamRead::Event(event) => {
                let mut drained_events = vec![event];
                let reconnect_signal = self.drain_buffered_socket_events(&mut drained_events).await;
                self.push_new_events(drained_events, true);

                match reconnect_signal {
                    Some(StreamRead::Closed) => {
                        self.socket.take();
                        if self.pending_events.is_empty() {
                            self.reconnect().await?;
                        } else {
                            self.reconnect_pending = true;
                        }
                    }
                    Some(StreamRead::Transport(error)) => {
                        debug!(
                            error = %error,
                            "runtime websocket read failed while draining buffered events; attempting reconnect"
                        );
                        self.socket.take();
                        if self.pending_events.is_empty() {
                            self.reconnect().await?;
                        } else {
                            self.reconnect_pending = true;
                        }
                    }
                    Some(StreamRead::Event(_)) => unreachable!(
                        "draining buffered events should not return nested stream events"
                    ),
                    None => {}
                }

                Ok(self.pending_events.pop_front())
            }
            StreamRead::Closed => {
                self.socket.take();
                self.reconnect().await?;
                Ok(self.pending_events.pop_front())
            }
            StreamRead::Transport(error) => {
                debug!(error = %error, "runtime websocket read failed; attempting reconnect");
                self.socket.take();
                self.reconnect().await?;
                Ok(self.pending_events.pop_front())
            }
        }
    }

    async fn drain_buffered_socket_events(
        &mut self,
        drained_events: &mut Vec<EventEnvelope>,
    ) -> Option<StreamRead> {
        loop {
            let next = {
                let socket = self
                    .socket
                    .as_mut()
                    .expect("socket should be present while draining buffered events");
                read_buffered_socket_event(socket, STREAM_READ_AHEAD_WINDOW).await
            };

            match next {
                Some(StreamRead::Event(event)) => drained_events.push(event),
                Some(StreamRead::Closed) => return Some(StreamRead::Closed),
                Some(StreamRead::Transport(error)) => {
                    return Some(StreamRead::Transport(error));
                }
                None => return None,
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), OpenHandsError> {
        if let Some(mut socket) = self.socket.take() {
            socket.close(None).await.map_err(|error| {
                OpenHandsError::websocket_transport("close runtime stream", error)
            })?;
        }
        Ok(())
    }

    async fn attach(mut self) -> Result<Self, OpenHandsError> {
        self.refresh_conversation().await?;
        let initial_cache = self.client.search_all_events(self.conversation_id).await?;
        self.push_new_events(initial_cache.items().iter().cloned(), true);
        self.connect_ready_and_reconcile().await?;
        Ok(self)
    }

    async fn refresh_conversation(&mut self) -> Result<(), OpenHandsError> {
        self.conversation = self.client.get_conversation(self.conversation_id).await?;
        self.rebuild_state_mirror();
        Ok(())
    }

    async fn reconnect(&mut self) -> Result<(), OpenHandsError> {
        let mut attempts = 0usize;
        let mut delay = self.config.reconnect_initial_backoff;

        loop {
            attempts += 1;
            if attempts > 1 {
                sleep(delay).await;
                delay = delay
                    .saturating_mul(2)
                    .min(self.config.reconnect_max_backoff);
            }

            let error = match self.refresh_conversation().await {
                Ok(()) => match self.connect_ready_and_reconcile().await {
                    Ok(()) => return Ok(()),
                    Err(error) => error,
                },
                Err(error) => error,
            };

            if attempts >= self.config.max_reconnect_attempts {
                return Err(OpenHandsError::ReconnectExhausted {
                    attempts,
                    last_error: error.to_string(),
                });
            }
        }
    }

    async fn connect_ready_and_reconcile(&mut self) -> Result<(), OpenHandsError> {
        let mut socket = self.client.connect_websocket(self.conversation_id).await?;
        let ready_event =
            wait_for_readiness_on_stream(&mut socket, self.config.readiness_timeout).await?;
        self.ready_event = ready_event.clone();
        self.socket = Some(socket);

        let reconciled = self.client.search_all_events(self.conversation_id).await?;
        self.push_new_events(reconciled.items().iter().cloned(), true);
        self.rebuild_state_mirror();
        Ok(())
    }

    fn push_new_events<I>(&mut self, events: I, queue_new: bool) -> usize
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        let inserted = self.event_cache.merge_new_events(events);
        if inserted.is_empty() {
            return 0;
        }

        if queue_new {
            self.queue_pending_events(&inserted);
        }
        if inserted.iter().any(|event| {
            matches!(
                KnownEvent::from_envelope(event),
                KnownEvent::ConversationStateUpdate(_)
            )
        }) {
            self.rebuild_state_mirror();
        }
        inserted.len()
    }

    fn queue_pending_events(&mut self, inserted: &[EventEnvelope]) {
        for event in inserted {
            let position = self
                .pending_events
                .iter()
                .position(|pending| compare_pending_events(pending, event) == Ordering::Greater)
                .unwrap_or(self.pending_events.len());
            self.pending_events.insert(position, event.clone());
        }
    }

    fn rebuild_state_mirror(&mut self) {
        self.state_mirror
            .rebuild_from(&self.conversation, self.event_cache.items());
        self.apply_terminal_conversation_fallback();
    }

    fn apply_terminal_conversation_fallback(&mut self) {
        if matches!(
            self.conversation.execution_status.as_str(),
            "finished" | "error" | "stuck"
        ) && self.state_mirror.terminal_status().is_none()
        {
            self.state_mirror
                .apply_conversation_execution_status(&self.conversation);
        }
    }
}

fn compare_pending_events(left: &EventEnvelope, right: &EventEnvelope) -> Ordering {
    left.timestamp
        .cmp(&right.timestamp)
        .then_with(|| left.id.cmp(&right.id))
}

#[derive(Debug)]
enum StreamRead {
    Event(EventEnvelope),
    Closed,
    Transport(OpenHandsError),
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
        let response = send(self.get_request("/openapi.json")?, "probe OpenAPI").await?;
        read_success_body(response, "probe OpenAPI")
            .await
            .map(|_| ())
    }

    pub async fn create_conversation(
        &self,
        request: &ConversationCreateRequest,
    ) -> Result<Conversation, OpenHandsError> {
        let response = send(
            self.json_request(
                self.post_request("/api/conversations")?,
                "create conversation",
                request,
            )?,
            "create conversation",
        )
        .await?;
        decode_json(response, "create conversation").await
    }

    pub async fn get_conversation(
        &self,
        conversation_id: Uuid,
    ) -> Result<Conversation, OpenHandsError> {
        let response = send(
            self.get_request(&format!("/api/conversations/{conversation_id}"))?,
            "fetch conversation",
        )
        .await?;
        decode_json(response, "fetch conversation").await
    }

    pub async fn send_message(
        &self,
        conversation_id: Uuid,
        request: &SendMessageRequest,
    ) -> Result<AcceptedResponse, OpenHandsError> {
        let response = send(
            self.json_request(
                self.post_request(&format!("/api/conversations/{conversation_id}/events"))?,
                "send conversation event",
                request,
            )?,
            "send conversation event",
        )
        .await?;
        decode_accepted(response, "send conversation event").await
    }

    pub async fn run_conversation(
        &self,
        conversation_id: Uuid,
    ) -> Result<AcceptedResponse, OpenHandsError> {
        let response = send(
            self.json_request(
                self.post_request(&format!("/api/conversations/{conversation_id}/run"))?,
                "trigger conversation run",
                &ConversationRunRequest::default(),
            )?,
            "trigger conversation run",
        )
        .await?;
        decode_accepted(response, "trigger conversation run").await
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

        let response = send(
            self.transport.apply_http_auth(self.http.get(url))?,
            "search conversation events",
        )
        .await?;
        decode_json(response, "search conversation events").await
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

    pub async fn attach_runtime_stream(
        &self,
        conversation_id: Uuid,
        config: RuntimeStreamConfig,
    ) -> Result<RuntimeEventStream, OpenHandsError> {
        let conversation = self.get_conversation(conversation_id).await?;
        RuntimeEventStream::new(self.clone(), conversation_id, config, conversation)
            .attach()
            .await
    }

    pub async fn wait_for_readiness(
        &self,
        conversation_id: Uuid,
        wait_timeout: Duration,
    ) -> Result<EventEnvelope, OpenHandsError> {
        let mut stream = self.connect_websocket(conversation_id).await?;
        wait_for_readiness_on_stream(&mut stream, wait_timeout).await
    }

    pub async fn run_probe(
        &self,
        request: &ConversationCreateRequest,
        wait_timeout: Duration,
    ) -> Result<OpenHandsProbeResult, OpenHandsError> {
        let conversation = self.create_conversation(request).await?;
        let mut stream = self
            .attach_runtime_stream(
                conversation.conversation_id,
                RuntimeStreamConfig {
                    readiness_timeout: wait_timeout,
                    reconnect_initial_backoff: Duration::from_millis(100),
                    reconnect_max_backoff: Duration::from_secs(1),
                    max_reconnect_attempts: 4,
                },
            )
            .await?;
        self.send_message(
            conversation.conversation_id,
            &SendMessageRequest::user_text(
                "Reply with the exact text `OpenSymphony doctor probe OK` and then finish.",
            ),
        )
        .await?;
        self.run_conversation(conversation.conversation_id).await?;
        wait_for_probe_terminal_state(&mut stream, wait_timeout).await?;
        let conversation = self.get_conversation(conversation.conversation_id).await?;
        let ready_event = stream.ready_event().clone();
        let event_cache = stream.event_cache().clone();
        let state_mirror = stream.state_mirror().clone();
        stream.close().await?;

        Ok(OpenHandsProbeResult {
            conversation,
            ready_event,
            event_cache,
            state_mirror,
        })
    }

    fn get_request(&self, suffix: &str) -> Result<RequestBuilder, OpenHandsError> {
        let url = self.transport.endpoint(suffix)?;
        self.transport.apply_http_auth(self.http.get(url))
    }

    fn post_request(&self, suffix: &str) -> Result<RequestBuilder, OpenHandsError> {
        let url = self.transport.endpoint(suffix)?;
        self.transport.apply_http_auth(self.http.post(url))
    }

    fn json_request<T>(
        &self,
        request: RequestBuilder,
        operation: &'static str,
        payload: &T,
    ) -> Result<RequestBuilder, OpenHandsError>
    where
        T: Serialize,
    {
        let body = serde_json::to_vec(payload)
            .map_err(|error| OpenHandsError::protocol(operation, error))?;
        Ok(request.header(CONTENT_TYPE, "application/json").body(body))
    }

    async fn connect_websocket(
        &self,
        conversation_id: Uuid,
    ) -> Result<RuntimeSocket, OpenHandsError> {
        let ws_request = self.transport.websocket_request(conversation_id)?;
        let (stream, _) = connect_async(ws_request).await.map_err(|error| {
            OpenHandsError::websocket_transport("connect runtime stream", error)
        })?;
        Ok(stream)
    }
}

async fn send(
    request: RequestBuilder,
    operation: &'static str,
) -> Result<reqwest::Response, OpenHandsError> {
    request
        .send()
        .await
        .map_err(|error| OpenHandsError::transport(operation, error))
}

async fn read_success_body(
    response: reqwest::Response,
    operation: &'static str,
) -> Result<Option<Value>, OpenHandsError> {
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| OpenHandsError::transport(operation, error))?;
    if !status.is_success() {
        return Err(OpenHandsError::HttpStatus {
            operation,
            status_code: status.as_u16(),
            body,
        });
    }

    if body.trim().is_empty() {
        return Ok(None);
    }

    serde_json::from_str(&body)
        .map(Some)
        .map_err(|error| OpenHandsError::protocol(operation, error))
}

async fn decode_json<T>(
    response: reqwest::Response,
    operation: &'static str,
) -> Result<T, OpenHandsError>
where
    T: DeserializeOwned,
{
    let value = read_success_body(response, operation)
        .await?
        .ok_or_else(|| OpenHandsError::protocol(operation, "expected JSON response body"))?;
    serde_json::from_value(value).map_err(|error| OpenHandsError::protocol(operation, error))
}

async fn decode_accepted(
    response: reqwest::Response,
    operation: &'static str,
) -> Result<AcceptedResponse, OpenHandsError> {
    let Some(value) = read_success_body(response, operation).await? else {
        return Ok(AcceptedResponse::accepted());
    };

    let accepted: AcceptedResponse = serde_json::from_value(value)
        .map_err(|error| OpenHandsError::protocol(operation, error))?;
    if accepted.success {
        Ok(accepted)
    } else {
        Err(OpenHandsError::protocol(
            operation,
            "response reported `success=false`",
        ))
    }
}

fn parse_text_event(payload: &str) -> Result<EventEnvelope, OpenHandsError> {
    serde_json::from_str(payload).map_err(|error| OpenHandsError::MalformedWebSocketEvent {
        detail: error.to_string(),
        snippet: payload.chars().take(160).collect(),
    })
}

fn parse_binary_event(payload: &[u8]) -> Result<EventEnvelope, OpenHandsError> {
    serde_json::from_slice(payload).map_err(|error| OpenHandsError::MalformedWebSocketEvent {
        detail: error.to_string(),
        snippet: String::from_utf8_lossy(payload).chars().take(160).collect(),
    })
}

fn parse_header_name(name: &str) -> Result<HeaderName, OpenHandsError> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|error| {
        OpenHandsError::invalid_configuration(format!("invalid auth header name `{name}`: {error}"))
    })
}

fn parse_header_value(value: &str) -> Result<HeaderValue, OpenHandsError> {
    HeaderValue::from_str(value).map_err(|error| {
        OpenHandsError::invalid_configuration(format!("invalid auth header value: {error}"))
    })
}

async fn wait_for_readiness_on_stream(
    stream: &mut RuntimeSocket,
    wait_timeout: Duration,
) -> Result<EventEnvelope, OpenHandsError> {
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
            Some(Err(error)) => {
                return Err(OpenHandsError::websocket_transport(
                    "wait for readiness",
                    error,
                ));
            }
        }
    }
}

async fn read_next_socket_event(stream: &mut RuntimeSocket) -> StreamRead {
    loop {
        match stream.next().await {
            Some(Ok(Message::Text(payload))) => match parse_text_event(&payload) {
                Ok(event) => return StreamRead::Event(event),
                Err(error) => {
                    debug!(error = %error, "ignoring undecodable websocket text frame during streaming");
                }
            },
            Some(Ok(Message::Binary(payload))) => match parse_binary_event(&payload) {
                Ok(event) => return StreamRead::Event(event),
                Err(error) => {
                    debug!(error = %error, "ignoring undecodable websocket binary frame during streaming");
                }
            },
            Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {
                debug!("ignoring websocket control frame during streaming");
            }
            Some(Ok(Message::Frame(_))) => {
                debug!("ignoring raw websocket frame during streaming");
            }
            Some(Ok(Message::Close(_))) | None => return StreamRead::Closed,
            Some(Err(error)) => {
                return StreamRead::Transport(OpenHandsError::websocket_transport(
                    "read runtime event",
                    error,
                ));
            }
        }
    }
}

async fn read_buffered_socket_event(
    stream: &mut RuntimeSocket,
    read_ahead_window: Duration,
) -> Option<StreamRead> {
    timeout(read_ahead_window, read_next_socket_event(stream))
        .await
        .ok()
}

async fn wait_for_probe_terminal_state(
    stream: &mut RuntimeEventStream,
    wait_timeout: Duration,
) -> Result<(), OpenHandsError> {
    let deadline = Instant::now() + wait_timeout;

    loop {
        if let Some(event) = stream.pending_conversation_error_event() {
            return Err(OpenHandsError::ProbeRunUnhealthy(format!(
                "received {} {} before a successful terminal status",
                event.kind, event.id
            )));
        }

        match stream.state_mirror().terminal_status() {
            Some(TerminalExecutionStatus::Finished) => return Ok(()),
            Some(TerminalExecutionStatus::Error) | Some(TerminalExecutionStatus::Stuck) => {
                return Err(OpenHandsError::ProbeRunUnhealthy(format!(
                    "terminal execution_status `{}`",
                    stream.state_mirror().execution_status().unwrap_or_default()
                )));
            }
            None => {}
        }

        let next_event = timeout_at(deadline, stream.poll_next_event_once())
            .await
            .map_err(|_| OpenHandsError::ProbeActivityTimeout(wait_timeout))??;
        let Some(event) = next_event else {
            continue;
        };

        if matches!(
            KnownEvent::from_envelope(&event),
            KnownEvent::ConversationError(_)
        ) {
            return Err(OpenHandsError::ProbeRunUnhealthy(format!(
                "received {} {} before a successful terminal status",
                event.kind, event.id
            )));
        }
    }
}

impl RuntimeEventStream {
    fn pending_conversation_error_event(&self) -> Option<&EventEnvelope> {
        self.pending_events.iter().find(|event| {
            matches!(
                KnownEvent::from_envelope(event),
                KnownEvent::ConversationError(_)
            )
        })
    }
}
