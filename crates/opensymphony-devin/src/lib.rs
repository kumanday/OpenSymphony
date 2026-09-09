//! Devin cloud harness helpers.
//!
//! Devin is a remote implementation agent: the agent runtime, the checkout, and
//! the shell all live in a Devin-owned cloud workspace. OpenSymphony therefore
//! keeps its local sanitized issue workspace for manifests, journals, and
//! evidence only, and binds the repository to the remote workspace through
//! [`DevinRemoteWorkspaceBinding`] instead of handing the local checkout to the
//! harness as an execution cwd.
//!
//! The capability advertised by this adapter is intentionally unavailable:
//! remote workspace ownership, event normalization, auth/secret handling, and
//! tenant isolation still need hardening evidence before hosted routing can be
//! presented as production-ready.

use std::{fmt, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{
    opensymphony_domain::HarnessAdapter,
    opensymphony_gateway_schema::{
        capability::HarnessCapability,
        envelope::EntityRef,
        event_journal::{EventActor, EventKind, EventRecord},
    },
};

pub const DEVIN_CLOUD_AGENT_KIND: &str = "devin_cloud_agent";
pub const DEVIN_CLOUD_API_CONTRACT: &str = "devin-cloud-api-v1";
pub const DEFAULT_DEVIN_API_BASE_URL: &str = "https://api.devin.ai/v1";
pub const DEFAULT_DEVIN_API_KEY_ENV: &str = "DEVIN_API_KEY";
pub const DEFAULT_DEVIN_EVENT_POLL_INTERVAL_MS: u64 = 2_000;
pub const DEFAULT_DEVIN_REQUEST_TIMEOUT_MS: u64 = 30_000;

/// Effective containment label recorded for runs routed to Devin.
///
/// The local workspace is never the execution directory for this harness.
pub const DEVIN_REMOTE_CONTAINMENT: &str = "devin_owned_remote_workspace";

#[derive(Debug, thiserror::Error)]
pub enum DevinConfigError {
    #[error("devin.api.base_url must be an absolute https URL: {0}")]
    InvalidBaseUrl(String),
    #[error("devin.api.base_url must use the https scheme for remote Devin routing")]
    InsecureBaseUrl,
    #[error("devin.api.base_url must not embed credentials")]
    CredentialsInBaseUrl,
    #[error("devin.api.api_key_env must be an environment variable name")]
    InvalidApiKeyEnv,
    #[error("devin repository binding must be an absolute https git URL: {0}")]
    InvalidRepositoryUrl(String),
}

#[derive(Debug, thiserror::Error)]
pub enum DevinClientError {
    #[error("devin api credential `{env}` is not set in the worker environment")]
    MissingCredential { env: String },
    #[error("failed to build devin api url from `{base_url}` and `{path}`: {source}")]
    InvalidUrl {
        base_url: String,
        path: String,
        #[source]
        source: url::ParseError,
    },
    #[error("devin api transport error: {0}")]
    Transport(String),
    #[error("devin api returned status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("failed to decode devin api response: {0}")]
    Decode(String),
}

/// API token wrapper that never renders its secret value.
#[derive(Clone, PartialEq, Eq)]
pub struct DevinApiToken(String);

impl DevinApiToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DevinApiToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DevinApiToken(redacted)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinCloudConfig {
    pub base_url: String,
    pub api_key_env: String,
    pub event_poll_interval: Duration,
    pub request_timeout: Duration,
}

impl Default for DevinCloudConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DEVIN_API_BASE_URL.to_owned(),
            api_key_env: DEFAULT_DEVIN_API_KEY_ENV.to_owned(),
            event_poll_interval: Duration::from_millis(DEFAULT_DEVIN_EVENT_POLL_INTERVAL_MS),
            request_timeout: Duration::from_millis(DEFAULT_DEVIN_REQUEST_TIMEOUT_MS),
        }
    }
}

impl DevinCloudConfig {
    pub fn validate(&self) -> Result<(), DevinConfigError> {
        let parsed = Url::parse(&self.base_url)
            .map_err(|error| DevinConfigError::InvalidBaseUrl(error.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(DevinConfigError::InsecureBaseUrl);
        }
        if parsed.host().is_none() {
            return Err(DevinConfigError::InvalidBaseUrl("missing host".into()));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DevinConfigError::CredentialsInBaseUrl);
        }
        if !is_environment_name(&self.api_key_env) {
            return Err(DevinConfigError::InvalidApiKeyEnv);
        }

        Ok(())
    }
}

fn is_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
        && value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
}

/// Repository binding for a Devin-owned remote workspace.
///
/// `local_evidence_path` is the sanitized OpenSymphony issue workspace. It holds
/// run manifests, journals, and evidence for the run; it is *not* the directory
/// Devin executes in and must never be advertised as such.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinRemoteWorkspaceBinding {
    pub issue_workspace_key: String,
    pub local_evidence_path: PathBuf,
    pub repository_url: String,
    pub base_branch: Option<String>,
}

impl DevinRemoteWorkspaceBinding {
    pub fn new(
        issue_workspace_key: impl Into<String>,
        local_evidence_path: impl Into<PathBuf>,
        repository_url: impl Into<String>,
        base_branch: Option<String>,
    ) -> Result<Self, DevinConfigError> {
        let repository_url = repository_url.into();
        let parsed = Url::parse(&repository_url)
            .map_err(|error| DevinConfigError::InvalidRepositoryUrl(error.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(DevinConfigError::InvalidRepositoryUrl(
                "must use the https scheme".into(),
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DevinConfigError::InvalidRepositoryUrl(
                "must not embed credentials".into(),
            ));
        }

        Ok(Self {
            issue_workspace_key: issue_workspace_key.into(),
            local_evidence_path: local_evidence_path.into(),
            repository_url,
            base_branch,
        })
    }

    /// Containment label for the run: Devin owns the execution environment.
    pub fn effective_containment(&self) -> &'static str {
        DEVIN_REMOTE_CONTAINMENT
    }
}

#[derive(Debug, Clone, Default)]
pub struct DevinCloudAdapter {
    config: DevinCloudConfig,
}

impl DevinCloudAdapter {
    pub fn new(config: DevinCloudConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &DevinCloudConfig {
        &self.config
    }

    pub fn requests(&self) -> DevinRequestBuilder {
        DevinRequestBuilder::new(self.config.base_url.clone())
    }

    /// Feature gaps that block hosted Devin routing today.
    pub fn unavailability_reason(&self) -> String {
        let capability = self.capabilities();
        format!(
            "harness `{}` is advertised as unavailable: {}",
            capability.kind,
            capability.feature_gaps.join(" ")
        )
    }
}

impl HarnessAdapter for DevinCloudAdapter {
    fn harness_kind(&self) -> &'static str {
        DEVIN_CLOUD_AGENT_KIND
    }

    fn capabilities(&self) -> HarnessCapability {
        HarnessCapability::devin_cloud_future()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevinHttpMethod {
    Get,
    Post,
}

impl DevinHttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevinLifecycleRequest {
    SessionCreate,
    SessionResume,
    MessageSend,
    RunStart,
    RunCancel,
    EventFetch,
}

/// Transport-agnostic description of a Devin cloud API call.
///
/// Credentials are injected by [`DevinCloudClient`] at send time so request
/// descriptions stay safe to log, snapshot, and unit test.
#[derive(Debug, Clone, PartialEq)]
pub struct DevinRequest {
    pub lifecycle: DevinLifecycleRequest,
    pub method: DevinHttpMethod,
    pub path: String,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinRequestBuilder {
    base_url: String,
}

impl DevinRequestBuilder {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn create_session(
        &self,
        prompt: impl Into<String>,
        binding: &DevinRemoteWorkspaceBinding,
        idempotency_key: Option<String>,
    ) -> DevinRequest {
        let mut body = json!({
            "prompt": prompt.into(),
            "repository": {
                "url": binding.repository_url,
                "base_branch": binding.base_branch,
            },
        });
        if let Some(key) = idempotency_key
            && let Some(object) = body.as_object_mut()
        {
            object.insert("idempotency_key".into(), Value::String(key));
        }

        DevinRequest {
            lifecycle: DevinLifecycleRequest::SessionCreate,
            method: DevinHttpMethod::Post,
            path: "/sessions".into(),
            body: Some(body),
        }
    }

    pub fn resume_session(&self, session_id: &str) -> DevinRequest {
        DevinRequest {
            lifecycle: DevinLifecycleRequest::SessionResume,
            method: DevinHttpMethod::Post,
            path: format!("/sessions/{}/resume", encode_path_segment(session_id)),
            body: Some(json!({})),
        }
    }

    pub fn send_message(&self, session_id: &str, message: impl Into<String>) -> DevinRequest {
        DevinRequest {
            lifecycle: DevinLifecycleRequest::MessageSend,
            method: DevinHttpMethod::Post,
            path: format!("/sessions/{}/messages", encode_path_segment(session_id)),
            body: Some(json!({ "message": message.into() })),
        }
    }

    pub fn start_run(&self, session_id: &str, prompt: impl Into<String>) -> DevinRequest {
        DevinRequest {
            lifecycle: DevinLifecycleRequest::RunStart,
            method: DevinHttpMethod::Post,
            path: format!("/sessions/{}/runs", encode_path_segment(session_id)),
            body: Some(json!({ "prompt": prompt.into() })),
        }
    }

    pub fn cancel_run(&self, session_id: &str, run_id: &str) -> DevinRequest {
        DevinRequest {
            lifecycle: DevinLifecycleRequest::RunCancel,
            method: DevinHttpMethod::Post,
            path: format!(
                "/sessions/{}/runs/{}/cancel",
                encode_path_segment(session_id),
                encode_path_segment(run_id)
            ),
            body: Some(json!({})),
        }
    }

    pub fn fetch_events(&self, session_id: &str, after_cursor: Option<u64>) -> DevinRequest {
        let path = match after_cursor {
            Some(cursor) => format!(
                "/sessions/{}/events?after={cursor}",
                encode_path_segment(session_id)
            ),
            None => format!("/sessions/{}/events", encode_path_segment(session_id)),
        };

        DevinRequest {
            lifecycle: DevinLifecycleRequest::EventFetch,
            method: DevinHttpMethod::Get,
            path,
            body: None,
        }
    }

    pub fn absolute_url(&self, request: &DevinRequest) -> Result<Url, DevinClientError> {
        let joined = format!("{}{}", self.base_url.trim_end_matches('/'), request.path);
        Url::parse(&joined).map_err(|source| DevinClientError::InvalidUrl {
            base_url: self.base_url.clone(),
            path: request.path.clone(),
            source,
        })
    }
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// HTTPS client for the Devin cloud API.
#[derive(Debug, Clone)]
pub struct DevinCloudClient {
    http: reqwest::Client,
    requests: DevinRequestBuilder,
    token: DevinApiToken,
}

impl DevinCloudClient {
    /// Builds a client, reading the API token from the configured environment
    /// variable. The token is never stored in configuration or manifests.
    pub fn from_environment(
        config: &DevinCloudConfig,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, DevinClientError> {
        let token = environment(&config.api_key_env)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DevinClientError::MissingCredential {
                env: config.api_key_env.clone(),
            })?;
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .https_only(true)
            .build()
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;

        Ok(Self {
            http,
            requests: DevinRequestBuilder::new(config.base_url.clone()),
            token: DevinApiToken::new(token.trim()),
        })
    }

    pub fn requests(&self) -> &DevinRequestBuilder {
        &self.requests
    }

    pub async fn send(&self, request: &DevinRequest) -> Result<Value, DevinClientError> {
        let url = self.requests.absolute_url(request)?;
        let mut builder = match request.method {
            DevinHttpMethod::Get => self.http.get(url),
            DevinHttpMethod::Post => self.http.post(url),
        }
        .bearer_auth(self.token.expose());
        if let Some(body) = request.body.as_ref() {
            builder = builder.json(body);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(DevinClientError::Status {
                status: status.as_u16(),
                body,
            });
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&body).map_err(|error| DevinClientError::Decode(error.to_string()))
    }

    pub async fn fetch_events(
        &self,
        session_id: &str,
        after_cursor: Option<u64>,
    ) -> Result<Vec<NormalizedDevinEvent>, DevinClientError> {
        let response = self
            .send(&self.requests.fetch_events(session_id, after_cursor))
            .await?;
        Ok(normalize_event_page(&response))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedDevinEventKind {
    SessionCreated,
    SessionResumed,
    RunStarted,
    RunCompleted,
    RunFailed,
    RunCancelled,
    AgentMessage,
    UserMessage,
    ToolCall,
    ToolResult,
    StatusChanged,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedDevinEvent {
    pub kind: NormalizedDevinEventKind,
    pub event_type: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub cursor: Option<u64>,
    pub message: Option<String>,
    pub status: Option<String>,
    pub raw: Value,
}

/// Normalizes one Devin event payload.
///
/// Unrecognized event types are retained as [`NormalizedDevinEventKind::Unknown`]
/// with the full raw JSON so future Devin schema additions replay unchanged.
pub fn normalize_devin_event(raw: Value) -> Option<NormalizedDevinEvent> {
    let event_type = first_string(&raw, &["type", "event_type", "kind"])?;
    let kind = match event_type.as_str() {
        "session.created" | "session_created" => NormalizedDevinEventKind::SessionCreated,
        "session.resumed" | "session_resumed" => NormalizedDevinEventKind::SessionResumed,
        "run.started" | "run_started" => NormalizedDevinEventKind::RunStarted,
        "run.completed" | "run_completed" => NormalizedDevinEventKind::RunCompleted,
        "run.failed" | "run_failed" => NormalizedDevinEventKind::RunFailed,
        "run.cancelled" | "run.canceled" | "run_cancelled" => {
            NormalizedDevinEventKind::RunCancelled
        }
        "devin_message" | "agent_message" | "message.agent" => {
            NormalizedDevinEventKind::AgentMessage
        }
        "user_message" | "message.user" => NormalizedDevinEventKind::UserMessage,
        "tool_call" | "tool.call" => NormalizedDevinEventKind::ToolCall,
        "tool_result" | "tool.result" => NormalizedDevinEventKind::ToolResult,
        "status_update" | "session.status" => NormalizedDevinEventKind::StatusChanged,
        "error" => NormalizedDevinEventKind::Error,
        _ => NormalizedDevinEventKind::Unknown,
    };

    Some(NormalizedDevinEvent {
        kind,
        event_type,
        session_id: first_string(&raw, &["session_id", "sessionId"]),
        run_id: first_string(&raw, &["run_id", "runId"]),
        cursor: first_u64(&raw, &["cursor", "sequence", "seq"]),
        message: first_string(&raw, &["message", "text", "content"]),
        status: first_string(&raw, &["status", "status_enum"]),
        raw,
    })
}

/// Normalizes an events page, accepting either a bare array or an object with
/// an `events` array.
pub fn normalize_event_page(response: &Value) -> Vec<NormalizedDevinEvent> {
    let events = response
        .as_array()
        .or_else(|| response.get("events").and_then(Value::as_array))
        .cloned()
        .unwrap_or_default();

    events
        .into_iter()
        .filter_map(normalize_devin_event)
        .collect()
}

pub fn devin_event_summary(event: &NormalizedDevinEvent) -> String {
    match event.kind {
        NormalizedDevinEventKind::SessionCreated => "Devin session created".into(),
        NormalizedDevinEventKind::SessionResumed => "Devin session resumed".into(),
        NormalizedDevinEventKind::RunStarted => "Devin run started".into(),
        NormalizedDevinEventKind::RunCompleted => "Devin run completed".into(),
        NormalizedDevinEventKind::RunFailed => "Devin run failed".into(),
        NormalizedDevinEventKind::RunCancelled => "Devin run cancelled".into(),
        NormalizedDevinEventKind::AgentMessage => event
            .message
            .clone()
            .unwrap_or_else(|| "Devin agent message".into()),
        NormalizedDevinEventKind::UserMessage => "Operator message forwarded to Devin".into(),
        NormalizedDevinEventKind::ToolCall => "Devin tool call".into(),
        NormalizedDevinEventKind::ToolResult => "Devin tool result".into(),
        NormalizedDevinEventKind::StatusChanged => match event.status.as_deref() {
            Some(status) => format!("Devin status changed to {status}"),
            None => "Devin status changed".into(),
        },
        NormalizedDevinEventKind::Error => event
            .message
            .clone()
            .unwrap_or_else(|| "Devin reported an error".into()),
        NormalizedDevinEventKind::Unknown => {
            format!("Unknown Devin event `{}`", event.event_type)
        }
    }
}

pub fn devin_event_payload(event: &NormalizedDevinEvent) -> Value {
    json!({
        "source_kind": event.event_type,
        "session_id": event.session_id,
        "run_id": event.run_id,
        "cursor": event.cursor,
        "status": event.status,
        "message": event.message,
        "raw_payload": event.raw,
    })
}

pub fn normalized_event_to_journal_record(
    run_id: impl Into<String>,
    sequence: u64,
    event: &NormalizedDevinEvent,
) -> EventRecord {
    let run_id = run_id.into();
    let kind = match event.kind {
        NormalizedDevinEventKind::RunStarted => EventKind::RunStarted,
        NormalizedDevinEventKind::RunCompleted => EventKind::RunCompleted,
        NormalizedDevinEventKind::RunFailed | NormalizedDevinEventKind::Error => {
            EventKind::RunFailed
        }
        NormalizedDevinEventKind::RunCancelled => EventKind::RunCancelled,
        NormalizedDevinEventKind::ToolCall => EventKind::HarnessToolCall,
        NormalizedDevinEventKind::ToolResult => EventKind::HarnessToolResult,
        NormalizedDevinEventKind::Unknown => EventKind::Unknown {
            raw_kind: event.event_type.clone(),
        },
        _ => EventKind::HarnessEventNormalized {
            source_kind: event.event_type.clone(),
        },
    };

    EventRecord::builder()
        .sequence(sequence)
        .actor(EventActor::harness(DEVIN_CLOUD_AGENT_KIND))
        .entity_ref(EntityRef::run(run_id.clone()))
        .summary(devin_event_summary(event))
        .kind(kind)
        .payload(devin_event_payload(event))
        .raw_payload_ref(format!("devin:{run_id}:{sequence}"))
        .build()
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

fn first_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_u64))
}
