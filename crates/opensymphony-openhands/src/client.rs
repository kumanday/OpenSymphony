//! Minimal REST and WebSocket client for the pinned OpenHands server contract.

use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Request;
use tokio_tungstenite::tungstenite::http::header::{HeaderName, HeaderValue};
use url::Url;

use crate::config::{HttpAuth, TransportConfig, WebSocketAuthMode};
use crate::error::{OpenHandsError, Result};
use crate::wire::{
    ConversationInfo, CreateConversationRequest, EventPage, RuntimeEventEnvelope,
    SendMessageRequest, ServerInfo, SuccessResponse,
};

const SESSION_API_KEY_HEADER: &str = "x-session-api-key";

/// REST and WebSocket client facade for the OpenHands agent-server.
#[derive(Clone, Debug)]
pub struct OpenHandsClient {
    http: reqwest::Client,
    transport: TransportConfig,
}

/// Result of triggering a background run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunConversationResponse {
    /// Whether the server reported that the conversation was already running.
    pub already_running: bool,
}

impl OpenHandsClient {
    /// Builds a new client around the provided transport configuration.
    #[must_use]
    pub fn new(transport: TransportConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            transport,
        }
    }

    /// Returns the immutable transport configuration.
    #[must_use]
    pub fn transport(&self) -> &TransportConfig {
        &self.transport
    }

    /// Fetches the server diagnostics surface exposed outside `/api`.
    pub async fn server_info(&self) -> Result<ServerInfo> {
        self.request_json(
            reqwest::Method::GET,
            self.transport.join_root_path("/server_info")?,
            None::<&()>,
        )
        .await
    }

    /// Performs a simple health probe.
    pub async fn health(&self) -> Result<()> {
        self.probe_path("/health").await
    }

    /// Performs a readiness probe.
    pub async fn ready(&self) -> Result<()> {
        self.probe_path("/ready").await
    }

    /// Probes a root-relative path and returns success on any 2xx status.
    pub async fn probe_path(&self, path: &str) -> Result<()> {
        let url = self.transport.join_root_path(path)?;
        let request = self.http.request(reqwest::Method::GET, url.clone());
        let response = request
            .send()
            .await
            .map_err(|source| OpenHandsError::HttpTransport {
                method: "GET".to_string(),
                url: url.to_string(),
                source,
            })?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(OpenHandsError::HttpStatus {
            method: "GET".to_string(),
            url: url.to_string(),
            status,
            body,
        })
    }

    /// Creates or reuses a conversation through `POST /api/conversations`.
    pub async fn create_conversation(
        &self,
        request: &CreateConversationRequest,
    ) -> Result<ConversationInfo> {
        self.request_json(
            reqwest::Method::POST,
            self.transport.join_rest_path("/conversations")?,
            Some(request),
        )
        .await
    }

    /// Fetches the authoritative conversation state, returning `None` on 404.
    pub async fn get_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationInfo>> {
        self.request_json_allow_not_found(
            reqwest::Method::GET,
            self.transport
                .join_rest_path(&format!("/conversations/{conversation_id}"))?,
        )
        .await
    }

    /// Sends a user message event to the server.
    pub async fn send_user_message(
        &self,
        conversation_id: &str,
        request: &SendMessageRequest,
    ) -> Result<SuccessResponse> {
        self.request_json(
            reqwest::Method::POST,
            self.transport
                .join_rest_path(&format!("/conversations/{conversation_id}/events"))?,
            Some(request),
        )
        .await
    }

    /// Triggers the background run endpoint and tolerates the server's 409 no-op case.
    pub async fn run_conversation(&self, conversation_id: &str) -> Result<RunConversationResponse> {
        let url = self
            .transport
            .join_rest_path(&format!("/conversations/{conversation_id}/run"))?;
        let response = self
            .send_request(reqwest::Method::POST, url.clone(), Option::<&()>::None)
            .await?;
        match response.status() {
            status if status.is_success() => Ok(RunConversationResponse {
                already_running: false,
            }),
            reqwest::StatusCode::CONFLICT => Ok(RunConversationResponse {
                already_running: true,
            }),
            status => {
                let body = response.text().await.unwrap_or_default();
                Err(OpenHandsError::HttpStatus {
                    method: "POST".to_string(),
                    url: url.to_string(),
                    status,
                    body,
                })
            }
        }
    }

    /// Fetches a single search page from the event API.
    pub async fn search_events_page(
        &self,
        conversation_id: &str,
        page_id: Option<&str>,
        limit: u32,
    ) -> Result<EventPage> {
        let mut url = self
            .transport
            .join_rest_path(&format!("/conversations/{conversation_id}/events/search"))?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &limit.to_string());
            if let Some(page_id) = page_id {
                query.append_pair("page_id", page_id);
            }
        }
        self.request_json(reqwest::Method::GET, url, None::<&()>)
            .await
    }

    /// Fetches and decodes every event page, preserving unknown events as raw JSON.
    pub async fn search_events_all(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<RuntimeEventEnvelope>> {
        let mut page_id = None;
        let mut events = Vec::new();

        loop {
            let page = self
                .search_events_page(conversation_id, page_id.as_deref(), 100)
                .await?;
            for item in page.items {
                events.push(RuntimeEventEnvelope::from_json(item)?);
            }
            match page.next_page_id {
                Some(next_page_id) => page_id = Some(next_page_id),
                None => return Ok(events),
            }
        }
    }

    /// Builds a WebSocket request for the event stream, preserving any base path prefix.
    pub fn websocket_request(&self, conversation_id: &str) -> Result<(Url, Request<()>)> {
        let mut url = self
            .transport
            .join_root_path(&format!("/sockets/events/{conversation_id}"))?;
        let scheme = match url.scheme() {
            "https" => "wss",
            "http" => "ws",
            "wss" => "wss",
            "ws" => "ws",
            other => {
                return Err(OpenHandsError::InvalidConfig {
                    message: format!("unsupported websocket scheme derived from {other}"),
                });
            }
        };
        url.set_scheme(scheme)
            .map_err(|_| OpenHandsError::InvalidConfig {
                message: "failed to set websocket URL scheme".to_string(),
            })?;

        let mut query_value = None;
        if let Some(api_key) = self.transport.session_api_key() {
            match self.transport.websocket_auth {
                WebSocketAuthMode::None => {}
                WebSocketAuthMode::QueryParam => query_value = Some(api_key.to_string()),
                WebSocketAuthMode::Header => {}
                WebSocketAuthMode::Auto => {}
            }
        }
        if let Some(api_key) = query_value {
            url.query_pairs_mut()
                .append_pair(&self.transport.websocket_query_param_name, &api_key);
        }

        let mut request =
            url.as_str()
                .into_client_request()
                .map_err(|source| OpenHandsError::Protocol {
                    message: format!("failed to build websocket request: {source}"),
                })?;
        if let Some(api_key) = self.transport.session_api_key() {
            if matches!(
                self.transport.websocket_auth,
                WebSocketAuthMode::Header | WebSocketAuthMode::Auto
            ) {
                let name = HeaderName::from_static(SESSION_API_KEY_HEADER);
                let value =
                    HeaderValue::from_str(api_key).map_err(|_| OpenHandsError::InvalidConfig {
                        message: "session API key contains invalid header bytes".to_string(),
                    })?;
                request.headers_mut().insert(name, value);
            }
        }

        Ok((url, request))
    }

    async fn request_json<T, B>(
        &self,
        method: reqwest::Method,
        url: Url,
        body: Option<&B>,
    ) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let response = self.send_request(method.clone(), url.clone(), body).await?;
        if response.status().is_success() {
            response
                .json()
                .await
                .map_err(|source| OpenHandsError::HttpTransport {
                    method: method.to_string(),
                    url: url.to_string(),
                    source,
                })
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(OpenHandsError::HttpStatus {
                method: method.to_string(),
                url: url.to_string(),
                status,
                body,
            })
        }
    }

    async fn request_json_allow_not_found<T>(
        &self,
        method: reqwest::Method,
        url: Url,
    ) -> Result<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        let response = self
            .send_request(method.clone(), url.clone(), None::<&()>)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if response.status().is_success() {
            return response.json().await.map(Some).map_err(|source| {
                OpenHandsError::HttpTransport {
                    method: method.to_string(),
                    url: url.to_string(),
                    source,
                }
            });
        }
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(OpenHandsError::HttpStatus {
            method: method.to_string(),
            url: url.to_string(),
            status,
            body,
        })
    }

    async fn send_request<B>(
        &self,
        method: reqwest::Method,
        url: Url,
        body: Option<&B>,
    ) -> Result<reqwest::Response>
    where
        B: serde::Serialize + ?Sized,
    {
        let mut request = self.http.request(method.clone(), url.clone());
        if let HttpAuth::SessionApiKey(api_key) = &self.transport.http_auth {
            request = request.header(SESSION_API_KEY_HEADER, api_key);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        request
            .send()
            .await
            .map_err(|source| OpenHandsError::HttpTransport {
                method: method.to_string(),
                url: url.to_string(),
                source,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HttpAuth, TransportConfig, WebSocketAuthMode};

    #[test]
    fn websocket_request_preserves_base_path_and_header_auth() {
        let client = OpenHandsClient::new(TransportConfig {
            base_url: Url::parse("https://example.com/runtime/456/api")
                .expect("static test URL must parse"),
            http_auth: HttpAuth::SessionApiKey("secret".to_string()),
            websocket_auth: WebSocketAuthMode::Header,
            websocket_query_param_name: "session_api_key".to_string(),
        });
        let (url, request) = client
            .websocket_request("abc")
            .expect("websocket request should build");
        assert_eq!(
            url.as_str(),
            "wss://example.com/runtime/456/api/sockets/events/abc"
        );
        assert_eq!(
            request
                .headers()
                .get(SESSION_API_KEY_HEADER)
                .expect("session API key header should be present"),
            "secret"
        );
    }

    #[test]
    fn websocket_request_can_use_query_param_auth() {
        let client = OpenHandsClient::new(TransportConfig {
            base_url: Url::parse("http://127.0.0.1:8000").expect("static test URL must parse"),
            http_auth: HttpAuth::SessionApiKey("secret".to_string()),
            websocket_auth: WebSocketAuthMode::QueryParam,
            websocket_query_param_name: "session_api_key".to_string(),
        });
        let (url, request) = client
            .websocket_request("abc")
            .expect("websocket request should build");
        assert_eq!(
            url.as_str(),
            "ws://127.0.0.1:8000/sockets/events/abc?session_api_key=secret"
        );
        assert!(request.headers().get(SESSION_API_KEY_HEADER).is_none());
    }
}
