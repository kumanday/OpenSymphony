use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{
    RequestBuilder,
    header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::time::{Instant, timeout_at};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tracing::debug;
use url::Url;
use uuid::Uuid;

use crate::events::{ConversationStateMirror, EventCache, KnownEvent};
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

    pub async fn wait_for_readiness(
        &self,
        conversation_id: Uuid,
        wait_timeout: Duration,
    ) -> Result<EventEnvelope, OpenHandsError> {
        let ws_request = self.transport.websocket_request(conversation_id)?;
        let (mut stream, _) = connect_async(ws_request)
            .await
            .map_err(|error| OpenHandsError::websocket_transport("wait for readiness", error))?;
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
        self.send_message(
            conversation.conversation_id,
            &SendMessageRequest::user_text(
                "Reply with the exact text `OpenSymphony doctor probe OK` and then finish.",
            ),
        )
        .await?;
        self.run_conversation(conversation.conversation_id).await?;

        let (conversation, event_cache) = self
            .wait_for_probe_activity(conversation.conversation_id, wait_timeout)
            .await?;
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

    async fn wait_for_probe_activity(
        &self,
        conversation_id: Uuid,
        wait_timeout: Duration,
    ) -> Result<(Conversation, EventCache), OpenHandsError> {
        let deadline = Instant::now() + wait_timeout;

        loop {
            let conversation = self.get_conversation(conversation_id).await?;
            let event_cache = self.search_all_events(conversation_id).await?;
            match probe_activity_observed(&conversation, &event_cache) {
                ProbeActivity::Healthy => return Ok((conversation, event_cache)),
                ProbeActivity::Unhealthy(detail) => {
                    return Err(OpenHandsError::ProbeRunUnhealthy(detail));
                }
                ProbeActivity::Pending => {}
            }

            if Instant::now() >= deadline {
                return Err(OpenHandsError::ProbeActivityTimeout(wait_timeout));
            }

            tokio::time::sleep(Duration::from_millis(200)).await;
        }
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
}

enum ProbeActivity {
    Pending,
    Healthy,
    Unhealthy(String),
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

fn probe_activity_observed(conversation: &Conversation, event_cache: &EventCache) -> ProbeActivity {
    let mut state_mirror = ConversationStateMirror::default();
    state_mirror.apply_conversation(conversation);

    for event in event_cache.items() {
        if event.kind == "ConversationErrorEvent" {
            return ProbeActivity::Unhealthy(format!(
                "received {} {} before a successful terminal status",
                event.kind, event.id
            ));
        }

        state_mirror.apply_event(event);
    }

    match state_mirror.execution_status() {
        Some("finished") => ProbeActivity::Healthy,
        Some("error") | Some("stuck") => ProbeActivity::Unhealthy(format!(
            "terminal execution_status `{}`",
            state_mirror.execution_status().unwrap_or_default()
        )),
        _ => ProbeActivity::Pending,
    }
}
