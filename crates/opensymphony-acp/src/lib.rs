//! Executable ACP v1 stdio client. Durable ownership and worker routing live in the host.
//!
//! `SessionHost` retains process ownership; `run_turn` remains a one-turn compatibility API.
//! A protocol stop reason is distinct from proof that remote delegated work has stopped.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::{
    Client, Dispatch, Lines, RawJsonRpcMessage, UntypedMessage,
    schema::{
        ProtocolVersion,
        v1::{
            AuthMethod, AuthenticateRequest, CancelNotification, ContentBlock, InitializeRequest,
            InitializeResponse, LoadSessionRequest, NewSessionRequest, PromptRequest,
            ResumeSessionRequest, TextContent,
        },
    },
};
use aho_corasick::{AhoCorasick, AhoCorasickKind, MatchKind};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
    sync::{mpsc, oneshot},
};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LinesCodec},
    sync::CancellationToken,
};
use tracing::instrument::WithSubscriber;

mod atomic_file;
mod durable;
mod extensions;
mod host;
mod operator;
mod projection;
mod services;
mod session_config;
pub use extensions::OperationCapability;
pub use host::*;
pub use operator::{
    AcpOperatorDeliveryFence, AcpOperatorEvent, AcpOperatorReply, AcpOperatorRequest,
};
pub use projection::{RuntimeProjection, RuntimeUpdate, profile_capabilities, run_capability};
pub use services::HostServices;
pub use session_config::SessionConfiguration;

pub(crate) fn launch_profile_fingerprint(
    profile: &AcpProfile,
    services: &HostServices,
    limits: &ClientLimits,
) -> Result<String, String> {
    durable::profile_fingerprint(profile, services, limits).map_err(|error| error.to_string())
}
#[cfg(windows)]
mod windows_path;
#[cfg(windows)]
mod windows_process;

use crate::opensymphony_gateway_schema::approval::OperatorAnswer;
pub use crate::opensymphony_workflow::AcpProfile;
use crate::opensymphony_workspace::{
    environment_variable_names_equal, has_environment_name_collision, insert_environment_value,
    redact_runtime_diagnostic, runtime_field_is_sensitive, sanitize_workspace_key,
};

#[cfg(not(windows))]
use crate::opensymphony_workspace::{configure_process_group, terminate_process_tree};

pub const SDK_VERSION: &str = "2.2.0";
pub const SCHEMA_VERSION: &str = "1.9.1";

/// Host-owned launch inputs; neither cwd nor resolved secrets belong in a profile.
/// `workspace_key` is the sanitized final directory name; `workspace_root` is its
/// immediate parent. The retained host also verifies the manager-owned handle,
/// repository binding and generation for nested or generation-suffixed workspaces.
pub struct LaunchContext {
    pub workspace_root: PathBuf,
    pub workspace_key: String,
    pub issue_workspace: PathBuf,
    pub environment: BTreeMap<String, String>,
    pub excluded_environment: BTreeSet<String>,
    pub services: HostServices,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientLimits {
    pub frame_bytes: usize,
    pub queued_frames: usize,
    /// Cumulative wire bytes awaiting SDK dispatch, independently of frame count.
    pub queued_bytes: usize,
    /// Callback responses awaiting completed stdin writes, including SDK-internal queues.
    pub callback_frames: usize,
    /// Cumulative encoded callback response bytes, including LF delimiters.
    pub callback_bytes: usize,
    pub evidence_frames: usize,
    /// Cumulative serialized bytes of retained, redacted SourceFrame values.
    pub evidence_bytes: usize,
    pub stderr_bytes: usize,
    pub file_bytes: usize,
    pub terminal_output_bytes: usize,
    pub terminal_count: usize,
    pub pending_callbacks: usize,
    pub callback_timeout: Duration,
    pub setup_timeout: Duration,
    /// Client wall-clock bound for a turn. Zero disables this bound, allowing
    /// the orchestrator's activity-based stall and abort policy to own liveness.
    pub prompt_timeout: Duration,
    pub cancel_timeout: Duration,
    pub reap_timeout: Duration,
}

impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            frame_bytes: 1024 * 1024,
            queued_frames: 128,
            queued_bytes: 4 * 1024 * 1024,
            callback_frames: 128,
            callback_bytes: 4 * 1024 * 1024,
            evidence_frames: 256,
            evidence_bytes: 1024 * 1024,
            stderr_bytes: 16 * 1024,
            file_bytes: 256 * 1024,
            terminal_output_bytes: 64 * 1024,
            terminal_count: 16,
            pending_callbacks: 64,
            callback_timeout: Duration::from_secs(300),
            setup_timeout: Duration::from_secs(30),
            prompt_timeout: Duration::from_secs(300),
            cancel_timeout: Duration::from_secs(10),
            reap_timeout: Duration::from_secs(5),
        }
    }
}

async fn wait_prompt_timeout(timeout: Duration) {
    if timeout.is_zero() {
        std::future::pending().await
    } else {
        tokio::time::sleep(timeout).await;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClientError {
    #[error("invalid ACP launch configuration: {0}")]
    InvalidConfiguration(String),
    #[error(
        "ACP workspace must be the existing canonical scheduler-bound checkout under its workspace root"
    )]
    InvalidWorkspace,
    #[error("ACP launch failed ({0:?})")]
    Launch(io::ErrorKind),
    #[error("ACP peer closed its output; prompt may have been submitted: {submitted}")]
    Disconnected { submitted: bool },
    #[error("ACP setup failed: {0}")]
    Setup(String),
    #[error(
        "ACP authentication is required; prompt may have been submitted: {submitted}; configure an advertised noninteractive auth.method_id or establish agent login before launch"
    )]
    AuthenticationRequired { submitted: bool },
    #[error("ACP setup deadline exceeded")]
    SetupTimeout,
    #[error("ACP cancelled before prompt submission")]
    CancelledBeforePrompt,
    #[error("ACP transport or RPC failed; prompt may have been submitted: {submitted}")]
    Protocol { submitted: bool },
    #[error(
        "ACP {method} RPC error {code}: {message}; prompt may have been submitted: {submitted}"
    )]
    Rpc {
        method: &'static str,
        code: i32,
        message: String,
        submitted: bool,
    },
    #[error("ACP prompt deadline exceeded; outcome is uncertain")]
    PromptTimeout,
    #[error("ACP cancellation deadline exceeded; cancellation is unacknowledged")]
    CancelTimeout,
    #[error(
        "ACP input/output resource limit exceeded; prompt may have been submitted: {submitted}"
    )]
    ResourceLimit { submitted: bool },
    #[error("ACP child could not be terminated and reaped")]
    Teardown,
    #[error("ACP extension operation deadline exceeded; outcome is unknown")]
    OperationTimeoutUnknown,
    #[error("ACP extension operation failed or returned an invalid result")]
    OperationFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFrame {
    pub sequence: u64,
    pub direction: String,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub payload: Value,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SessionUpdate {
    pub session_id: String,
    pub update: Value,
}

impl std::fmt::Debug for SessionUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionUpdate")
            .field("update", &self.update)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct TurnReport {
    /// Only `end_turn` is a successful turn. Unknown values remain unsupported outcomes.
    pub stop_reason: String,
    pub cancellation_requested: bool,
    pub cancellation_acknowledged: bool,
    pub session_id: String,
    pub initialization: InitializeResponse,
    pub configuration: SessionConfiguration,
}

// Negotiation metadata and opaque peer identifiers are operational values, not
// diagnostics: a peer can echo credentials in either. Captured frames are redacted.
impl std::fmt::Debug for TurnReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TurnReport")
            .field("succeeded", &self.succeeded())
            .field("cancellation_requested", &self.cancellation_requested)
            .field("cancellation_acknowledged", &self.cancellation_acknowledged)
            .finish_non_exhaustive()
    }
}

impl TurnReport {
    pub fn succeeded(&self) -> bool {
        self.stop_reason == "end_turn"
    }
}

#[derive(Debug)]
pub struct RunResult {
    pub outcome: Result<TurnReport, ClientError>,
    pub evidence: Vec<SourceFrame>,
    pub evidence_truncated: bool,
    pub stderr: String,
    pub process_reaped: bool,
    /// Signalling a departed process group can fail independently of reaping its child.
    pub process_tree_signal_error: Option<io::ErrorKind>,
}

/// Reservations precede SDK enqueue and survive every internal queue until stdin flush.
/// Retaining the SDK's exact encoding also rejects unreserved SDK-generated responses.
#[derive(Default)]
struct CallbackOutput {
    frames: VecDeque<(String, bool, Option<oneshot::Sender<bool>>)>,
    bytes: usize,
}

fn reserve_and_enqueue_operator_response<T>(
    output: &Mutex<CallbackOutput>,
    frame: String,
    limits: &ClientLimits,
    acknowledgement: &mut Option<oneshot::Sender<bool>>,
    enqueue: impl FnOnce() -> T,
) -> Option<T> {
    let mut output = output.lock().unwrap_or_else(|e| e.into_inner());
    output
        .admit_with_ack(frame, limits, false, acknowledgement)
        .then(enqueue)
}

impl CallbackOutput {
    fn admit(&mut self, frame: String, limits: &ClientLimits) -> bool {
        self.admit_with_opaque_payload(frame, limits, false)
    }

    fn admit_with_opaque_payload(
        &mut self,
        frame: String,
        limits: &ClientLimits,
        opaque_payload: bool,
    ) -> bool {
        self.admit_with_ack(frame, limits, opaque_payload, &mut None)
    }

    fn admit_with_ack(
        &mut self,
        frame: String,
        limits: &ClientLimits,
        opaque_payload: bool,
        acknowledgement: &mut Option<oneshot::Sender<bool>>,
    ) -> bool {
        let bytes = frame.len() + 1; // LinesCodec adds LF.
        if frame.len() > limits.frame_bytes
            || self.frames.len() >= limits.callback_frames
            || bytes > limits.callback_bytes.saturating_sub(self.bytes)
        {
            return false;
        }
        self.bytes += bytes;
        self.frames
            .push_back((frame, opaque_payload, acknowledgement.take()));
        true
    }

    fn matches_front(&self, frame: &str) -> bool {
        self.frames
            .front()
            .is_some_and(|(expected, _, _)| expected == frame)
    }

    fn flushed(&mut self) {
        if let Some((frame, _, acknowledgement)) = self.frames.pop_front() {
            self.bytes -= frame.len() + 1;
            if let Some(acknowledgement) = acknowledgement {
                let _ = acknowledgement.send(true);
            }
        }
    }

    fn fail_all(&mut self) {
        while let Some((_, _, acknowledgement)) = self.frames.pop_front() {
            if let Some(acknowledgement) = acknowledgement {
                let _ = acknowledgement.send(false);
            }
        }
        self.bytes = 0;
    }
}

impl Drop for CallbackOutput {
    fn drop(&mut self) {
        self.fail_all();
    }
}

#[derive(Default)]
struct QueuedInput {
    frame_sizes: VecDeque<usize>,
    bytes: usize,
}

impl QueuedInput {
    fn admit(&mut self, bytes: usize, max_frames: usize, max_bytes: usize) -> bool {
        if self.frame_sizes.len() >= max_frames || bytes > max_bytes.saturating_sub(self.bytes) {
            return false;
        }
        self.bytes += bytes;
        self.frame_sizes.push_back(bytes);
        true
    }

    fn dispatched(&mut self) {
        // The SDK dispatch callback applies every accepted frame in wire order.
        if let Some(bytes) = self.frame_sizes.pop_front() {
            self.bytes -= bytes;
        }
    }
}

struct SecretRedactor {
    matcher: AhoCorasick,
}

impl SecretRedactor {
    fn new(mut secrets: Vec<String>) -> Result<Self, ClientError> {
        secrets.sort();
        secrets.dedup();
        if secrets.len() > 1024
            || secrets
                .iter()
                .map(String::len)
                .fold(0usize, usize::saturating_add)
                > 1024 * 1024
        {
            return Err(ClientError::InvalidConfiguration(
                "credential redaction inputs exceed the 1024-pattern or 1 MiB budget".into(),
            ));
        }
        let matcher = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .kind(Some(AhoCorasickKind::ContiguousNFA))
            .build(secrets)
            .map_err(|_| {
                ClientError::InvalidConfiguration(
                    "credential redaction matcher could not be built".into(),
                )
            })?;
        Ok(Self { matcher })
    }

    fn contains_secret(&self, text: &str) -> bool {
        self.matcher.is_match(text)
    }

    fn contains_in_json(&self, value: &Value) -> bool {
        match value {
            Value::String(value) => self.contains_secret(value),
            Value::Array(values) => values.iter().any(|value| self.contains_in_json(value)),
            Value::Object(values) => values
                .iter()
                .any(|(key, value)| self.contains_secret(key) || self.contains_in_json(value)),
            _ => false,
        }
    }

    fn redact(&self, text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        self.matcher
            .replace_all_with(text, &mut result, |_, _, output| {
                output.push_str("[redacted]");
                true
            });
        result
    }
}

struct Capture {
    frames: Vec<SourceFrame>,
    sequence: u64,
    truncated: bool,
    max: usize,
    bytes: usize,
    max_bytes: usize,
    secrets: SecretRedactor,
    publisher: Option<EventPublisher>,
}

impl Capture {
    fn redact_stop_reason(&self, reason: &str) -> String {
        let redacted = self.secrets.redact(reason);
        if redacted.len() > 1024 || redacted.chars().any(char::is_control) {
            // Replacement can expand a bounded peer value beyond durable
            // metadata limits. Preserve terminal evidence without persisting
            // a partial replacement or protocol control text.
            "[redacted]".into()
        } else {
            redacted
        }
    }

    fn redact(&self, value: &mut Value, diagnostic: bool) {
        match value {
            Value::String(s) => {
                *s = self.secrets.redact(s);
                if diagnostic {
                    // Known-secret matching sees the complete value before truncation;
                    // generic preview normalization only examines a bounded prefix.
                    let preview = s.chars().take(2048).collect::<String>();
                    *s = redact_runtime_diagnostic(&preview);
                }
            }
            Value::Array(values) => values.iter_mut().for_each(|v| self.redact(v, diagnostic)),
            Value::Object(values) => {
                for (mut key, mut value) in std::mem::take(values) {
                    if runtime_field_is_sensitive(&key)
                        || [
                            "token",
                            "secret",
                            "password",
                            "authorization",
                            "api_key",
                            "apikey",
                            "credential",
                        ]
                        .iter()
                        .any(|s| key.to_ascii_lowercase().contains(s))
                    {
                        value = json!("[redacted]");
                    } else {
                        self.redact(&mut value, diagnostic);
                    }
                    key = self.secrets.redact(&key);
                    values.insert(key, value);
                }
            }
            _ => {}
        }
    }
    fn redact_frame(&self, payload: &mut Value, diagnostic: bool) {
        // These numeric counters are protocol usage, not credential tokens.
        // The exception is limited to a recognized prompt response shape;
        // arbitrary token-named fields and configuration still use full redaction.
        let usage = projection::reported_turn_usage(payload);
        self.redact(payload, diagnostic);
        if let Some(usage) = usage
            && let Some(target) = payload
                .pointer_mut("/result/usage")
                .and_then(Value::as_object_mut)
        {
            for (name, value) in usage {
                if !self.secrets.contains_secret(&name)
                    && !self.secrets.contains_secret(&value.to_string())
                {
                    target.insert(name, value);
                }
            }
        }
    }

    fn record(&mut self, direction: &str, mut payload: Value) {
        self.sequence += 1;
        if payload.get("method").and_then(Value::as_str) == Some("fs/write_text_file")
            && let Some(content) = payload.pointer_mut("/params/content")
        {
            *content = json!("[redacted]");
        }
        if let Some(servers) = payload
            .pointer_mut("/params/mcpServers")
            .and_then(Value::as_array_mut)
        {
            for server in servers {
                // Stdio argv can carry opaque credentials through generic flags
                // such as --header. Capture only the attachment shape, never its
                // argument values; the wire request remains unchanged.
                if let Some(args) = server.get_mut("args").and_then(Value::as_array_mut) {
                    for arg in args {
                        *arg = json!("[redacted]");
                    }
                }
                for field in ["env", "headers"] {
                    if let Some(entries) = server.get_mut(field).and_then(Value::as_array_mut) {
                        for entry in entries {
                            if let Some(value) = entry.get_mut("value") {
                                *value = json!("[redacted]");
                            }
                        }
                    }
                }
            }
        }
        if let Some(publisher) = &self.publisher {
            let mut source = payload.clone();
            self.redact_frame(&mut source, false);
            publisher.publish(SourceFrame {
                sequence: self.sequence,
                direction: direction.into(),
                observed_at: chrono::Utc::now(),
                payload: source,
            });
        }
        if self.frames.len() == self.max || self.bytes == self.max_bytes {
            self.truncated = true;
            return;
        }
        self.redact_frame(&mut payload, true);
        self.retain_frame(SourceFrame {
            sequence: self.sequence,
            direction: direction.into(),
            observed_at: chrono::Utc::now(),
            payload,
        });
    }

    fn retain_frame(&mut self, frame: SourceFrame) {
        let Ok(encoded) = serde_json::to_vec(&frame) else {
            self.truncated = true;
            return;
        };
        if encoded.len() > self.max_bytes.saturating_sub(self.bytes) {
            self.truncated = true;
            return;
        }
        self.bytes += encoded.len();
        self.frames.push(frame);
    }
}

type SharedCapture = Arc<Mutex<Capture>>;

/// Memory access belongs to the run-scoped worker overlay, never the daemon's
/// ambient environment. This also protects terminal callbacks, which inherit
/// the validated ACP child environment.
pub(crate) fn is_reserved_memory_environment_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        name.to_ascii_uppercase()
            .starts_with("OPENSYMPHONY_MEMORY_")
    }
    #[cfg(not(windows))]
    {
        name.starts_with("OPENSYMPHONY_MEMORY_")
    }
}

fn capture(capture: &SharedCapture, direction: &str, payload: Value) {
    capture
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .record(direction, payload);
}

fn rpc_failure(
    method: &'static str,
    error: &agent_client_protocol::Error,
    capture: &SharedCapture,
    submitted: bool,
) -> Option<ClientError> {
    if error.code == agent_client_protocol::schema::v1::ErrorCode::AuthRequired
        || agent_client_protocol::is_incoming_transport_closed(error)
    {
        return None;
    }
    let mut message = Value::String(error.message.clone());
    capture
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .redact(&mut message, true);
    Some(ClientError::Rpc {
        method,
        code: error.code.into(),
        message: message.as_str().unwrap_or_default().to_owned(),
        submitted,
    })
}

fn io_failure() -> io::Error {
    io::Error::other("ACP transport rejected a frame")
}

struct PreparedLaunch {
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    secrets: SecretRedactor,
}

fn validate_launch(
    profile: &AcpProfile,
    context: &LaunchContext,
    limits: &ClientLimits,
) -> Result<PreparedLaunch, ClientError> {
    profile
        .validate()
        .map_err(|e| ClientError::InvalidConfiguration(e.to_string()))?;
    if !(256..=16 * 1024 * 1024).contains(&limits.frame_bytes)
        || !(1..=4096).contains(&limits.queued_frames)
        || !(256..=64 * 1024 * 1024).contains(&limits.queued_bytes)
        || !(1..=4096).contains(&limits.callback_frames)
        || !(256..=64 * 1024 * 1024).contains(&limits.callback_bytes)
        || limits.evidence_frames > 4096
        || limits.evidence_bytes > 16 * 1024 * 1024
        || limits.stderr_bytes > 1024 * 1024
        || limits.file_bytes > 16 * 1024 * 1024
        || limits.terminal_output_bytes > 1024 * 1024
        || !(1..=128).contains(&limits.terminal_count)
        || !(1..=128).contains(&limits.pending_callbacks)
        || [
            limits.setup_timeout,
            limits.cancel_timeout,
            limits.reap_timeout,
            limits.callback_timeout,
        ]
        .iter()
        .any(Duration::is_zero)
    {
        return Err(ClientError::InvalidConfiguration(
            "invalid resource limits".into(),
        ));
    }
    let root = context
        .workspace_root
        .canonicalize()
        .map_err(|_| ClientError::InvalidWorkspace)?;
    let cwd = context
        .issue_workspace
        .canonicalize()
        .map_err(|_| ClientError::InvalidWorkspace)?;
    if !context.workspace_root.is_absolute()
        || !context.issue_workspace.is_absolute()
        || sanitize_workspace_key(&context.workspace_key)
            .ok()
            .as_deref()
            != Some(&context.workspace_key)
        || cwd != root.join(&context.workspace_key)
        || cwd != context.issue_workspace
        || !cwd.is_dir()
    {
        return Err(ClientError::InvalidWorkspace);
    }
    if has_environment_name_collision(profile.env_refs.keys().map(String::as_str)) {
        return Err(ClientError::InvalidConfiguration(
            "env_refs contains duplicate platform-equivalent targets".into(),
        ));
    }
    let excluded = |key: &str| {
        context
            .excluded_environment
            .iter()
            .any(|e| environment_variable_names_equal(e, key))
    };
    let mut environment = context
        .environment
        .iter()
        .filter(|(key, _)| {
            !excluded(key)
                && (!profile
                    .env_refs
                    .values()
                    .any(|source| environment_variable_names_equal(source, key))
                    || profile
                        .env_refs
                        .keys()
                        .any(|target| environment_variable_names_equal(target, key)))
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut secrets = Vec::new();
    for (target, source) in &profile.env_refs {
        if is_reserved_memory_environment_name(target)
            || is_reserved_memory_environment_name(source)
        {
            return Err(ClientError::InvalidConfiguration(
                "env_refs cannot remap a run-scoped memory grant".into(),
            ));
        }
        if excluded(target) || excluded(source) {
            return Err(ClientError::InvalidConfiguration(
                "env_refs cannot expose excluded checkout credentials".into(),
            ));
        }
        let value = context
            .environment
            .iter()
            .find(|(key, _)| environment_variable_names_equal(key, source))
            .map(|(_, value)| value)
            .filter(|v| !v.is_empty() && !v.contains('\0'))
            .ok_or_else(|| {
                ClientError::InvalidConfiguration(
                    "env_refs references a missing or invalid variable".into(),
                )
            })?;
        secrets.push(value.clone());
        insert_environment_value(&mut environment, target.clone(), value.clone());
    }
    for (key, value) in &context.environment {
        if !value.is_empty()
            && (excluded(key)
                || runtime_field_is_sensitive(key)
                || ["TOKEN", "SECRET", "PASSWORD", "KEY", "CREDENTIAL"]
                    .iter()
                    .any(|s| key.to_ascii_uppercase().contains(s)))
        {
            secrets.push(value.clone());
        }
    }
    if environment
        .iter()
        .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
    {
        return Err(ClientError::InvalidConfiguration(
            "invalid launch environment".into(),
        ));
    }
    if context.services.mcp_servers.len() > 32
        || serde_json::to_vec(&context.services.mcp_servers)
            .map_or(true, |v| v.len() > limits.frame_bytes / 2)
    {
        return Err(ClientError::InvalidConfiguration(
            "scoped MCP configuration exceeds limits".into(),
        ));
    }
    let excluded_values = SecretRedactor::new(
        context
            .environment
            .iter()
            .filter(|(name, value)| excluded(name) && !value.is_empty())
            .map(|(_, value)| value.clone())
            .collect(),
    )?;
    for server in &context.services.mcp_servers {
        use agent_client_protocol::schema::v1::McpServer;
        if excluded_values.contains_in_json(
            &serde_json::to_value(server)
                .map_err(|_| ClientError::InvalidConfiguration("invalid MCP attachment".into()))?,
        ) {
            return Err(ClientError::InvalidConfiguration(
                "MCP attachments cannot expose excluded checkout credentials".into(),
            ));
        }
        match server {
            McpServer::Http(server) => {
                validate_mcp_url(&server.url)?;
                secrets.extend(
                    server
                        .headers
                        .iter()
                        .filter(|h| {
                            runtime_field_is_sensitive(&h.name)
                                || h.name.eq_ignore_ascii_case("cookie")
                        })
                        .map(|h| h.value.clone())
                        .filter(|v| !v.is_empty()),
                );
            }
            McpServer::Sse(server) => {
                validate_mcp_url(&server.url)?;
                secrets.extend(
                    server
                        .headers
                        .iter()
                        .filter(|h| {
                            runtime_field_is_sensitive(&h.name)
                                || h.name.eq_ignore_ascii_case("cookie")
                        })
                        .map(|h| h.value.clone())
                        .filter(|v| !v.is_empty()),
                );
            }
            McpServer::Stdio(server) => {
                if server.env.iter().any(|v| excluded(&v.name)) {
                    return Err(ClientError::InvalidConfiguration(
                        "MCP environment cannot expose excluded checkout credentials".into(),
                    ));
                }
                // Argument values are separate JSON strings, so generic text
                // redaction cannot associate a token with the preceding flag.
                let header_argument =
                    |flag: &str| flag == "-H" || flag.eq_ignore_ascii_case("--header");
                let sensitive_argument = |flag: &str| {
                    runtime_field_is_sensitive(flag)
                        || flag.eq_ignore_ascii_case("--oauth2-bearer")
                        || header_argument(flag)
                };
                for (index, argument) in server.args.iter().enumerate() {
                    let (secret, header) = match argument.split_once('=') {
                        Some((flag, value)) if sensitive_argument(flag) => {
                            (Some(value), header_argument(flag))
                        }
                        _ if index > 0 && sensitive_argument(&server.args[index - 1]) => (
                            Some(argument.as_str()),
                            header_argument(&server.args[index - 1]),
                        ),
                        _ => (None, false),
                    };
                    if let Some(value) = secret.filter(|value| !value.is_empty()) {
                        secrets.push(value.to_owned());
                        if header && let Some((_, header_value)) = value.split_once(':') {
                            let header_value = header_value.trim();
                            if !header_value.is_empty() {
                                secrets.push(header_value.to_owned());
                            }
                        }
                    }
                }
                secrets.extend(
                    server
                        .env
                        .iter()
                        .filter(|e| runtime_field_is_sensitive(&e.name))
                        .map(|e| e.value.clone())
                        .filter(|v| !v.is_empty()),
                );
            }
            _ => {
                return Err(ClientError::InvalidConfiguration(
                    "unsupported MCP transport".into(),
                ));
            }
        }
    }
    secrets.extend(
        secrets
            .clone()
            .into_iter()
            .filter_map(|value| value.strip_prefix("Bearer ").map(str::to_owned))
            .filter(|value| !value.is_empty()),
    );
    let secrets = SecretRedactor::new(secrets)?;
    if secrets.contains_secret(&profile.command)
        || profile.args.iter().any(|arg| secrets.contains_secret(arg))
    {
        return Err(ClientError::InvalidConfiguration(
            "credentials cannot appear in argv".into(),
        ));
    }
    Ok(PreparedLaunch {
        cwd,
        environment,
        secrets,
    })
}

// Credentials belong in resolved headers, which are registered with the redactor.
// Reject credential-bearing URLs before either a process or source observer exists.
fn validate_mcp_url(value: &str) -> Result<(), ClientError> {
    let invalid =
        || ClientError::InvalidConfiguration("invalid or credential-bearing MCP URL".into());
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

/// Run a fresh session through setup, one prompt, cancellation, and supervised teardown.
/// The bounded update channel is optional. A full or closed supplied channel fails the run
/// visibly, so callers must drain it concurrently. The launch environment is explicit:
/// callers supply scoped memory grants and checkout credential exclusions before this call.
pub async fn run_turn(
    profile: &AcpProfile,
    context: LaunchContext,
    prompt: String,
    cancellation: CancellationToken,
    updates: Option<mpsc::Sender<SessionUpdate>>,
    limits: ClientLimits,
) -> Result<RunResult, ClientError> {
    run_connection(
        profile,
        context,
        prompt,
        cancellation,
        updates,
        limits,
        None,
    )
    .await
}

async fn run_connection(
    profile: &AcpProfile,
    context: LaunchContext,
    prompt: String,
    cancellation: CancellationToken,
    updates: Option<mpsc::Sender<SessionUpdate>>,
    limits: ClientLimits,
    mut driver: Option<&mut SessionDriver>,
) -> Result<RunResult, ClientError> {
    let PreparedLaunch {
        cwd,
        environment,
        secrets,
    } = validate_launch(profile, &context, &limits)?;
    if cancellation.is_cancelled() {
        return Err(ClientError::CancelledBeforePrompt);
    }
    let capture_state = Arc::new(Mutex::new(Capture {
        frames: Vec::new(),
        sequence: 0,
        truncated: false,
        max: limits.evidence_frames,
        bytes: 0,
        max_bytes: limits.evidence_bytes,
        secrets,
        publisher: driver.as_ref().map(|driver| driver.publisher.clone()),
    }));
    let mut command = Command::new(&profile.command);
    command
        .args(&profile.args)
        .env_clear()
        .envs(&environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(not(unix))]
    command.current_dir(&cwd);
    let queued = Arc::new(Mutex::new(QueuedInput::default()));
    let callback_output = Arc::new(Mutex::new(CallbackOutput::default()));
    let resource_failure = Arc::new(AtomicBool::new(false));
    let submitted = Arc::new(AtomicBool::new(false));
    let fatal = CancellationToken::new();
    let service_shutdown = CancellationToken::new();
    let _service_guard = service_shutdown.clone().drop_guard();
    let initial_callback_epoch = CancellationToken::new();
    initial_callback_epoch.cancel();
    let (service_sender, service_actor) = services::Services::new(
        cwd.clone(),
        environment,
        context.services.clone(),
        limits.clone(),
        callback_output.clone(),
        resource_failure.clone(),
        fatal.clone(),
        initial_callback_epoch,
        service_shutdown.clone(),
    )
    .map_err(|_| ClientError::InvalidWorkspace)?;
    #[cfg(unix)]
    service_actor
        .pin_child_cwd(&mut command)
        .map_err(|_| ClientError::InvalidWorkspace)?;
    #[cfg(not(windows))]
    configure_process_group(&mut command);
    #[cfg(not(windows))]
    let mut child = command.spawn().map_err(|e| ClientError::Launch(e.kind()))?;
    #[cfg(windows)]
    let mut child =
        windows_process::WindowsChild::spawn(command).map_err(|e| ClientError::Launch(e.kind()))?;
    #[cfg(not(windows))]
    let process_id = child.id();
    #[cfg(unix)]
    let mut group_guard = crate::opensymphony_workspace::ProcessGroupGuard::new(process_id);
    if let Some(driver) = driver.as_deref_mut() {
        driver.launched(child.id()).await?;
    }
    let stdin = child.stdin.take().ok_or(ClientError::Teardown)?;
    let stdout = child.stdout.take().ok_or(ClientError::Teardown)?;
    let stderr = child.stderr.take().ok_or(ClientError::Teardown)?;
    let service_run = service_actor.run();
    tokio::pin!(service_run);
    let configuration = Arc::new(Mutex::new(SessionConfiguration::default()));
    let operator_router = driver
        .as_ref()
        .map(|driver| driver.operator_router.clone())
        .unwrap_or_else(|| Arc::new(Mutex::new(None)));
    let permission_policy = profile.permissions.mode;
    let cursor_enabled = extensions::cursor_enabled(profile);
    let input = FramedRead::new(stdout, LinesCodec::new_with_max_length(limits.frame_bytes));
    let incoming = input.map({
        let queued = queued.clone();
        let capture_state = capture_state.clone();
        let resource_failure = resource_failure.clone();
        move |line| {
            let line = line.map_err(|_| {
                resource_failure.store(true, Ordering::Release);
                io_failure()
            })?;
            let value: Value = serde_json::from_str(&line).map_err(|_| io_failure())?;
            // ACP uses one JSON-RPC object per LF frame; batches are outside this transport contract.
            if !value.is_object() || value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                return Err(io_failure());
            }
            if !queued.lock().unwrap_or_else(|e| e.into_inner()).admit(
                line.len(),
                limits.queued_frames,
                limits.queued_bytes,
            ) {
                resource_failure.store(true, Ordering::Release);
                return Err(io_failure());
            }
            capture(&capture_state, "incoming", value);
            Ok(line)
        }
    });
    let output = FramedWrite::new(stdin, LinesCodec::new_with_max_length(limits.frame_bytes));
    // The SDK awaits SinkExt::send for each frame. Keep the reservation until that
    // send flushes the framed writer; preprocessing or start_send is too early.
    let outgoing = futures_util::sink::unfold(output, {
        let callback_output = callback_output.clone();
        let capture_state = capture_state.clone();
        let resource_failure = resource_failure.clone();
        let submitted = submitted.clone();
        move |mut output, line: String| {
            let callback_output = callback_output.clone();
            let capture_state = capture_state.clone();
            let resource_failure = resource_failure.clone();
            let submitted = submitted.clone();
            async move {
                if line.len() > limits.frame_bytes {
                    resource_failure.store(true, Ordering::Release);
                    callback_output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .fail_all();
                    return Err(io_failure());
                }
                let mut value: Value = match serde_json::from_str(&line) {
                    Ok(value) => value,
                    Err(_) => {
                        callback_output
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .fail_all();
                        return Err(io_failure());
                    }
                };
                let is_response = value.get("method").is_none();
                if is_response {
                    let output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                    if !output.matches_front(&line) {
                        // Unreserved SDK errors cannot bypass callback admission.
                        drop(output);
                        callback_output
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .fail_all();
                        return Err(io_failure());
                    }
                    if output.frames.front().is_some_and(|(_, opaque, _)| *opaque) {
                        for field in ["/result/content", "/result/output"] {
                            if let Some(content) = value.pointer_mut(field) {
                                *content = json!("[redacted]");
                            }
                        }
                    }
                }
                if value.get("method").and_then(Value::as_str) == Some("session/prompt") {
                    submitted.store(true, Ordering::Release);
                }
                capture(&capture_state, "outgoing", value);
                if !matches!(
                    tokio::time::timeout(limits.callback_timeout, output.send(line)).await,
                    Ok(Ok(()))
                ) {
                    resource_failure.store(true, Ordering::Release);
                    callback_output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .fail_all();
                    return Err(io_failure());
                }
                if is_response {
                    callback_output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .flushed();
                }
                Ok(output)
            }
        }
    });
    let active_session = Arc::new(Mutex::new(None::<String>));
    // Some ACP peers announce initial configuration before replying to session/new.
    // Keep those updates bounded until the response supplies the authoritative ID.
    let pending_session_updates = Arc::new(Mutex::new(Vec::<(String, Value)>::new()));
    let setup_updates = updates.clone();
    let client = Client.builder().on_receive_dispatch(
        {
            let active_session = active_session.clone();
            let pending_session_updates = pending_session_updates.clone();
            let callback_output = callback_output.clone();
            let limits = limits.clone();
            let resource_failure = resource_failure.clone();
            let fatal = fatal.clone();
            let redactor = capture_state.clone();
            let configuration = configuration.clone();
            let service_sender = service_sender.clone();
            let operator_router = operator_router.clone();
            async move |message: Dispatch, _cx| {
                queued
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .dispatched();
                match message {
                    Dispatch::Request(request, responder) => {
                        let service_method = request.method.starts_with("fs/")
                            || request.method.starts_with("terminal/");
                        let session = request.params.get("sessionId").and_then(Value::as_str);
                        if service_method
                            && session.is_some()
                            && active_session
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_deref()
                                == session
                        {
                            return service_sender
                                .enqueue(request.method, request.params, responder)
                                .inspect_err(|_| {
                                    resource_failure.store(true, Ordering::Release);
                                    fatal.cancel();
                                });
                        }
                        if cursor_enabled && request.method == "cursor/update_todos" {
                            // The pinned CLI sends this as an ID-bearing request without
                            // sessionId. Bind it to the connection's active session and
                            // prompt epoch, never to an asserted peer session.
                            let session = active_session.lock().unwrap_or_else(|e| e.into_inner()).clone();
                            let active = operator_router
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_ref()
                                .is_some_and(|(_, epoch)| !epoch.is_cancelled());
                            let bound = session.is_some()
                                && request.params.get("sessionId")
                                    .is_none_or(|id| id.as_str() == session.as_deref());
                            let response = Ok(if active && bound
                                && extensions::cursor_todos(&request.params)
                            {
                                json!({"outcome":{"outcome":"accepted","todos":request.params["todos"]}})
                            } else {
                                json!({"outcome":{"outcome":"rejected"}})
                            });
                            let frame = serde_json::to_string(&RawJsonRpcMessage::response(
                                responder.id().clone(), response.clone(),
                            ))?;
                            let mut output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                            if !output.admit(frame, &limits) {
                                resource_failure.store(true, Ordering::Release);
                                fatal.cancel();
                                return Err(agent_client_protocol::Error::internal_error());
                            }
                            return responder.respond_with_result(response);
                        }
                        let operator_method = matches!(request.method.as_str(),
                            "session/request_permission" | "elicitation/create")
                            || (cursor_enabled && matches!(request.method.as_str(),
                                "cursor/create_plan"));
                        if operator_method {
                            let session = active_session.lock().unwrap_or_else(|e| e.into_inner()).clone();
                            let rpc_id = serde_json::to_value(responder.id())?;
                            // Reserve against the private peer ID's encoded size, not the
                            // public opaque interaction token shown to operators.
                            let private_rpc_id = rpc_id.to_string();
                            let mut safe_params = request.params.clone();
                            redactor.lock().unwrap_or_else(|e| e.into_inner()).redact(&mut safe_params, false);
                            let interaction = session.as_deref().ok_or_else(agent_client_protocol::Error::invalid_params)
                                .and_then(|session| operator::parse_interaction(&request.method, &safe_params, rpc_id.clone(), session, limits.callback_timeout));
                            // Reject an option ID that changed during redaction; never send
                            // an altered opaque ID back to the peer or expose a secret.
                            let interaction = interaction.and_then(|safe| {
                                let original = operator::parse_interaction(&request.method, &request.params, rpc_id, &safe.session_id, limits.callback_timeout)?;
                                if !operator::same_binding_ids(&safe, &original) {
                                    return Err(agent_client_protocol::Error::invalid_params());
                                }
                                Ok(safe)
                            });
                            let interaction = match interaction {
                                Ok(interaction) => interaction,
                                Err(_) => {
                                    // A malformed or unbound callback never reaches an
                                    // operator. Return the protocol's cancellation outcome
                                    // so the peer can finish its current turn safely.
                                    let response = Ok(if request.method == "elicitation/create" {
                                        json!({"action":"cancel"})
                                    } else {
                                        json!({"outcome":{"outcome":"cancelled"}})
                                    });
                                    let frame = serde_json::to_string(&RawJsonRpcMessage::response(responder.id().clone(), response.clone()))?;
                                    let mut output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                                    if !output.admit(frame, &limits) {
                                        resource_failure.store(true, Ordering::Release);
                                        fatal.cancel();
                                        return Err(agent_client_protocol::Error::internal_error());
                                    }
                                    return responder.respond_with_result(response);
                                }
                            };
                            let route = operator_router.lock().unwrap_or_else(|e| e.into_inner()).clone();
                            // An automatic allow/deny is still a callback decision and
                            // must obey the same active-turn epoch as routed requests.
                            // The prompt response revokes this epoch in ordered dispatch
                            // before an adjacent late callback can run.
                            let (sender, epoch) = match route {
                                Some((sender, epoch)) if !epoch.is_cancelled() => (sender, epoch),
                                _ => {
                                    let response = Ok(operator::response_for_method(&request.method, &interaction, OperatorAnswer::Cancel));
                                    let frame = serde_json::to_string(&RawJsonRpcMessage::response(responder.id().clone(), response.clone()))?;
                                    let mut output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                                    if !output.admit(frame, &limits) {
                                        resource_failure.store(true, Ordering::Release);
                                        fatal.cancel();
                                        return Err(agent_client_protocol::Error::internal_error());
                                    }
                                    return responder.respond_with_result(response);
                                }
                            };
                            if let Some(answer) = operator::automatic_answer(permission_policy, &interaction) {
                                let response = Ok(operator::response_for_method(&request.method, &interaction, answer));
                                let frame = serde_json::to_string(&RawJsonRpcMessage::response(responder.id().clone(), response.clone()))?;
                                let mut output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                                if !output.admit(frame, &limits) {
                                    resource_failure.store(true, Ordering::Release);
                                    fatal.cancel();
                                    return Err(agent_client_protocol::Error::internal_error());
                                }
                                return responder.respond_with_result(response);
                            }
                            let reservation = match service_sender.reserve_operator(
                                &request.method,
                                &request.params,
                                &private_rpc_id,
                            ) {
                                Ok(reservation) => reservation,
                                Err(_) => {
                                    let response = Ok(operator::response_for_method(&request.method, &interaction, OperatorAnswer::Cancel));
                                    let frame = serde_json::to_string(&RawJsonRpcMessage::response(responder.id().clone(), response.clone()))?;
                                    let mut output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                                    if !output.admit(frame, &limits) {
                                        resource_failure.store(true, Ordering::Release);
                                        fatal.cancel();
                                        return Err(agent_client_protocol::Error::internal_error());
                                    }
                                    return responder.respond_with_result(response);
                                }
                            };
                            if let Some(sender) = sender {
                                let (reply, receive) = oneshot::channel();
                                if sender.try_send(AcpOperatorEvent::Opened(Box::new(AcpOperatorRequest { interaction: interaction.clone(), reply }))).is_ok() {
                                    let callback_output = callback_output.clone();
                                    let resource_failure = resource_failure.clone();
                                    let fatal = fatal.clone();
                                    let limits = limits.clone();
                                    tokio::spawn(async move {
                                        let _reservation = reservation;
                                        let (answer, reply) = tokio::select! {
                                            biased;
                                            _ = epoch.cancelled() => (OperatorAnswer::Cancel, None),
                                            result = receive => result.map(|reply: AcpOperatorReply| (reply.answer.clone(), Some(reply))).unwrap_or((OperatorAnswer::Cancel, None)),
                                            _ = tokio::time::sleep(limits.callback_timeout) => (OperatorAnswer::Cancel, None),
                                        };
                                        let (answer, mut acknowledgement) = match reply {
                                            Some(reply) if reply.delivery.claim() => (answer, Some(reply.acknowledgement)),
                                            Some(reply) => {
                                                let _ = reply.acknowledgement.send(false);
                                                (OperatorAnswer::Cancel, None)
                                            }
                                            None => (answer, None),
                                        };
                                        let response = Ok(operator::response_for_method(&request.method, &interaction, answer));
                                        let frame = serde_json::to_string(&RawJsonRpcMessage::response(responder.id().clone(), response.clone()));
                                        let delivered = frame.ok().and_then(|frame| {
                                            reserve_and_enqueue_operator_response(&callback_output, frame, &limits, &mut acknowledgement, || responder.respond_with_result(response))
                                        });
                                        let Some(delivered) = delivered else {
                                            resource_failure.store(true, Ordering::Release);
                                            fatal.cancel();
                                            if let Some(acknowledgement) = acknowledgement { let _ = acknowledgement.send(false); }
                                            return;
                                        };
                                        if delivered.is_err() {
                                            resource_failure.store(true, Ordering::Release);
                                            fatal.cancel();
                                            callback_output.lock().unwrap_or_else(|e| e.into_inner()).fail_all();
                                            return;
                                        }
                                        // The worker may be processing a burst of Opened
                                        // events while callbacks expire. A full channel must
                                        // not discard the only closure for an accepted
                                        // interaction; bound the wait and fail the turn if
                                        // the worker stops draining it.
                                        if tokio::time::timeout(
                                            limits.cancel_timeout,
                                            sender.send(AcpOperatorEvent::Closed(interaction.request_id)),
                                        ).await.is_err() {
                                            resource_failure.store(true, Ordering::Release);
                                            fatal.cancel();
                                        }
                                    });
                                    return Ok(());
                                }
                            }
                            // A permission with no operator route fails visibly. A form
                            // can safely return protocol cancellation so direct clients
                            // can continue without an interactive operator.
                            if request.method == "session/request_permission" {
                                fatal.cancel();
                            }
                            let response = Ok(operator::response_for_method(&request.method, &interaction, OperatorAnswer::Cancel));
                            let frame = serde_json::to_string(&RawJsonRpcMessage::response(responder.id().clone(), response.clone()))?;
                            let mut output = callback_output.lock().unwrap_or_else(|e| e.into_inner());
                            if output.admit(frame, &limits) {
                                return responder.respond_with_result(response);
                            }
                            return Err(agent_client_protocol::Error::internal_error());
                        }
                        let response = Err(agent_client_protocol::Error::method_not_found());
                        let frame = serde_json::to_string(&RawJsonRpcMessage::response(
                            responder.id().clone(),
                            response.clone(),
                        ))?;
                        let mut callback_output =
                            callback_output.lock().unwrap_or_else(|e| e.into_inner());
                        if !callback_output.admit(frame, &limits) {
                            resource_failure.store(true, Ordering::Release);
                            fatal.cancel();
                            // Individual-request responders send nothing on drop. Batches
                            // are rejected at ingress, so saturation cannot enqueue a reply.
                            return Err(agent_client_protocol::Error::internal_error());
                        }
                        responder.respond_with_result(response)
                    }
                    Dispatch::Notification(notification) => {
                        if notification.method == "session/update" {
                            let id = notification.params.get("sessionId").and_then(Value::as_str);
                            let update = notification.params.get("update").filter(|update| {
                                update.is_object()
                                    && update
                                        .get("sessionUpdate")
                                        .and_then(Value::as_str)
                                        .is_some()
                            });
                            let (Some(id), Some(update)) = (id, update) else {
                                fatal.cancel();
                                return Err(agent_client_protocol::Error::invalid_params());
                            };
                            let bound_session = active_session
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .clone();
                            if bound_session.is_none() {
                                let mut pending = pending_session_updates
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner());
                                let used: usize = pending.iter().map(|(id, value)| id.len() + value.to_string().len()).sum();
                                let bytes = id.len() + update.to_string().len();
                                if pending.len() >= limits.queued_frames
                                    || used.checked_add(bytes).is_none_or(|total| total > limits.queued_bytes)
                                {
                                    resource_failure.store(true, Ordering::Release);
                                    fatal.cancel();
                                    return Err(agent_client_protocol::Error::internal_error());
                                }
                                pending.push((id.to_owned(), update.clone()));
                                return Ok(());
                            }
                            if bound_session.as_deref() != Some(id) {
                                fatal.cancel();
                                return Err(agent_client_protocol::Error::invalid_params());
                            }
                            configuration
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .update(update)
                                .inspect_err(|_| fatal.cancel())?;
                            if let Some(tx) = &updates {
                                let mut update = update.clone();
                                redactor
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .redact(&mut update, false);
                                tx.try_send(SessionUpdate {
                                    session_id: id.into(),
                                    update,
                                })
                                .map_err(|_| {
                                    resource_failure.store(true, Ordering::Release);
                                    fatal.cancel();
                                    agent_client_protocol::Error::internal_error()
                                })?;
                            }
                        }
                        Ok(())
                    }
                    Dispatch::Response(result, router) => router.route_with_result(result),
                }
            }
        },
        agent_client_protocol::on_receive_dispatch!(),
    );
    let mut phase_error = None;
    let run = client
        .connect_with(Lines::new(outgoing, incoming), async |connection| {
            let one_turn_epoch = cancellation.child_token();
            let setup = async {
                let initialization = connection
                    .send_request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(context.services.capabilities()))
                    .block_task()
                    .await.inspect_err(|error| {
                        phase_error = rpc_failure("initialize", error, &capture_state, false);
                    })?;
                if initialization.protocol_version != ProtocolVersion::V1 {
                    phase_error = Some(ClientError::Setup("agent did not negotiate ACP v1".into()));
                    return Err(agent_client_protocol::Error::internal_error());
                }
                for capability in &profile.required_capabilities {
                    let caps = &initialization.agent_capabilities.prompt_capabilities;
                    let present = match capability.as_str() {
                        "prompt.image" => caps.image,
                        "prompt.audio" => caps.audio,
                        "prompt.embedded_context" => caps.embedded_context,
                        _ => false,
                    };
                    if !present {
                        phase_error = Some(ClientError::Setup(format!("required agent capability `{capability}` is unavailable")));
                        return Err(agent_client_protocol::Error::internal_error());
                    }
                }
                if let Some(auth) = &profile.auth {
                    if !initialization.auth_methods.iter().any(|m| {
                        matches!(m, AuthMethod::Agent(_)) && m.id().0.as_ref() == auth.method_id
                    }) {
                        phase_error = Some(ClientError::Setup(
                            "configured authentication method is unavailable or requires an unsupported interactive flow".into(),
                        ));
                        return Err(agent_client_protocol::Error::internal_error());
                    }
                    connection.send_request(AuthenticateRequest::new(auth.method_id.clone()))
                        .block_task().await.inspect_err(|error| {
                        phase_error = rpc_failure("authenticate", error, &capture_state, false);
                    })?;
                }
                if let Err(error) = session_config::validate_transports(&context.services.mcp_servers, &initialization.agent_capabilities) {
                    phase_error = Some(error);
                    return Err(agent_client_protocol::Error::internal_error());
                }
                let restoration = match driver.as_deref_mut() {
                    Some(driver) => driver.restoration(&initialization).map_err(|error| {
                        phase_error = Some(ClientError::Setup(error.to_string()));
                        agent_client_protocol::Error::internal_error()
                    })?,
                    None => None,
                };
                let restored_id = if let Some((session_id, replay)) = restoration {
                    // Bind before loading: replay notifications can precede the load response.
                    *active_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session_id.clone());
                    let restored = if replay {
                        let (loaded_tx, loaded_rx) = oneshot::channel();
                        let publisher = driver.as_ref().map(|driver| driver.publisher.clone());
                        let session_configuration = configuration.clone();
                        let restoration_binding = active_session.clone();
                        connection.send_request(LoadSessionRequest::new(session_id.clone(), cwd.clone()).mcp_servers(context.services.mcp_servers.clone()))
                            .on_receiving_result(async move |result| {
                                if let Ok(session) = &result {
                                    session_configuration.lock().unwrap_or_else(|e| e.into_inner()).initial(&serde_json::to_value(session)?);
                                } else {
                                    // Revoke in ordered dispatch before an adjacent old-session callback.
                                    *restoration_binding.lock().unwrap_or_else(|e| e.into_inner()) = None;
                                }
                                if let Some(publisher) = publisher {
                                    publisher.end_replay();
                                }
                                let _ = loaded_tx.send(result);
                                Ok(())
                            })?;
                        loaded_rx.await.map_err(|_| agent_client_protocol::Error::internal_error())?.map(|_| ())
                    } else {
                        let (resumed_tx, resumed_rx) = oneshot::channel();
                        let session_configuration = configuration.clone();
                        let restoration_binding = active_session.clone();
                        connection.send_request(ResumeSessionRequest::new(session_id.clone(), cwd.clone()).mcp_servers(context.services.mcp_servers.clone()))
                            .on_receiving_result(async move |result| {
                                if let Ok(session) = &result {
                                    session_configuration.lock().unwrap_or_else(|e| e.into_inner()).initial(&serde_json::to_value(session)?);
                                } else {
                                    *restoration_binding.lock().unwrap_or_else(|e| e.into_inner()) = None;
                                }
                                let _ = resumed_tx.send(result);
                                Ok(())
                            })?;
                        resumed_rx.await.map_err(|_| agent_client_protocol::Error::internal_error())?.map(|_| ())
                    };
                    match restored {
                        Ok(()) => Some(session_id),
                        Err(error) if driver.as_deref_mut().is_some_and(|driver| driver.reset_missing_session(&error)) => {
                            // Retire accepted restoration work and handles before binding a fresh session.
                            if let Err(error) = service_sender.end_turn(limits.setup_timeout).await {
                                phase_error = Some(error);
                                return Err(agent_client_protocol::Error::internal_error());
                            }
                            None
                        },
                        Err(error) => {
                            phase_error = rpc_failure(if replay { "session/load" } else { "session/resume" }, &error, &capture_state, false);
                            return Err(error);
                        }
                    }
                } else { None };
                let session_id = if let Some(session_id) = restored_id { session_id.into() } else {
                    let (session_tx, session_rx) = oneshot::channel();
                    let active_session = active_session.clone();
                    let pending_session_updates = pending_session_updates.clone();
                    let session_configuration = configuration.clone();
                    let redactor = capture_state.clone();
                    let updates = setup_updates.clone();
                    let resource_failure = resource_failure.clone();
                    let fatal = fatal.clone();
                    // Bind in ordered response dispatch before any adjacent session update.
                    connection.send_request(NewSessionRequest::new(cwd.clone()).mcp_servers(context.services.mcp_servers.clone()))
                        .on_receiving_result(async move |result| {
                            if let Ok(session) = &result {
                                let session_id = session.session_id.0.to_string();
                                let snapshot = serde_json::to_value(session)?;
                                session_configuration.lock().unwrap_or_else(|e| e.into_inner()).initial(&snapshot);
                                for (id, update) in std::mem::take(&mut *pending_session_updates.lock().unwrap_or_else(|e| e.into_inner())) {
                                    if id != session_id {
                                        fatal.cancel();
                                        return Err(agent_client_protocol::Error::invalid_params());
                                    }
                                    // The response is authoritative where it supplies a field;
                                    // an omitted field keeps its earlier announced value.
                                    SessionConfiguration::default().update(&update)?;
                                    let covered = match update.get("sessionUpdate").and_then(Value::as_str) {
                                        Some("config_option_update") => snapshot.get("configOptions").is_some(),
                                        Some("current_mode_update") => snapshot.pointer("/modes/currentModeId").is_some(),
                                        _ => false,
                                    };
                                    if !covered {
                                        session_configuration.lock().unwrap_or_else(|e| e.into_inner()).update(&update)?;
                                    }
                                    if let Some(tx) = &updates {
                                        let mut safe_update = update;
                                        redactor.lock().unwrap_or_else(|e| e.into_inner()).redact(&mut safe_update, false);
                                        tx.try_send(SessionUpdate { session_id: id, update: safe_update }).map_err(|_| {
                                            resource_failure.store(true, Ordering::Release);
                                            fatal.cancel();
                                            agent_client_protocol::Error::internal_error()
                                        })?;
                                    }
                                }
                                *active_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session_id);
                            }
                            let _ = session_tx.send(result);
                            Ok(())
                        })?;
                    session_rx.await
                        .map_err(|_| agent_client_protocol::Error::internal_error())?
                        .inspect_err(|error| {
                            phase_error = rpc_failure("session/new", error, &capture_state, false);
                        })?.session_id
                };
                if let Err(error) = session_config::apply(&connection, profile, session_id.0.as_ref(), &configuration, &capture_state).await {
                    phase_error = Some(error);
                    return Err(agent_client_protocol::Error::internal_error());
                }
                if driver.is_none()
                    && let Err(error) = service_sender.begin_turn(one_turn_epoch.clone(), limits.setup_timeout).await {
                    phase_error = Some(error);
                    return Err(agent_client_protocol::Error::internal_error());
                }
                if driver.is_none() {
                    *operator_router.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((None, one_turn_epoch.clone()));
                }
                if driver.is_some()
                    && let Err(error) = service_sender.end_turn(limits.setup_timeout).await {
                    phase_error = Some(error);
                    return Err(agent_client_protocol::Error::internal_error());
                }
                Ok((initialization, session_id))

            };
            let setup_result = tokio::select! {
                _ = cancellation.cancelled() => {
                    phase_error = Some(ClientError::CancelledBeforePrompt);
                    return Err(agent_client_protocol::Error::internal_error());
                },
                result = tokio::time::timeout(limits.setup_timeout, setup) => result,
            };
            let (initialization, session_id) = match setup_result {
                Ok(result) => result?,
                Err(_) => {
                    phase_error = Some(ClientError::SetupTimeout);
                    return Err(agent_client_protocol::Error::internal_error());
                }
            };
            if let Some(driver) = driver.as_deref_mut() {
                return driver.drive(connection, initialization, session_id, &capture_state, &limits, &service_sender, &configuration, profile).await
                    .map_err(|error| {
                        phase_error = Some(error);
                        agent_client_protocol::Error::internal_error()
                    });
            }
            if cancellation.is_cancelled() {
                phase_error = Some(ClientError::CancelledBeforePrompt);
                return Err(agent_client_protocol::Error::internal_error());
            }
            let request = PromptRequest::new(
                session_id.clone(), vec![ContentBlock::Text(TextContent::new(prompt))],
            );
            // Raw SDK response decoding preserves future stop reasons; request serialization stays typed.
            let request = UntypedMessage::new("session/prompt", request)?;
            let (response_tx, response_rx) = oneshot::channel();
            connection.send_request(request).on_receiving_result(async move |result| {
                // Match retained prompts: revoke callbacks before the next
                // inbound frame is dispatched, including adjacent late writes.
                one_turn_epoch.cancel();
                let _ = response_tx.send(result);
                Ok(())
            })?;
            let response = async {
                response_rx.await.map_err(|_| agent_client_protocol::Error::internal_error())?
            };
            tokio::pin!(response);
            let mut cancellation_requested = false;
            let result = tokio::select! {
                result = &mut response => result,
                _ = cancellation.cancelled() => {
                    cancellation_requested = true;
                    connection.send_notification(CancelNotification::new(session_id.clone()))?;
                    match tokio::time::timeout(limits.cancel_timeout, &mut response).await {
                        Ok(result) => result,
                        Err(_) => {
                            phase_error = Some(ClientError::CancelTimeout);
                            return Err(agent_client_protocol::Error::internal_error());
                        }
                    }
                },
                _ = wait_prompt_timeout(limits.prompt_timeout) => {
                    phase_error = Some(ClientError::PromptTimeout);
                    return Err(agent_client_protocol::Error::internal_error());
                }
            }.inspect_err(|error| {
                phase_error = rpc_failure("session/prompt", error, &capture_state, submitted.load(Ordering::Acquire));
            })?;
            let stop_reason = result.get("stopReason").and_then(Value::as_str)
                .ok_or_else(agent_client_protocol::Error::invalid_params)?;
            let stop_reason = capture_state.lock().unwrap_or_else(|error| error.into_inner())
                .redact_stop_reason(stop_reason);
            if let Err(error) = service_sender.end_turn(limits.setup_timeout).await {
                phase_error = Some(error);
                return Err(agent_client_protocol::Error::internal_error());
            }
            Ok(TurnReport {
                cancellation_acknowledged: cancellation_requested && stop_reason == "cancelled",
                cancellation_requested, stop_reason, session_id: session_id.0.to_string(), initialization,
                configuration: configuration.lock().unwrap_or_else(|e|e.into_inner()).clone(),
            })
        })
        .with_subscriber(tracing::subscriber::NoSubscriber::default());
    let stderr_drain = drain_stderr(stderr, limits.stderr_bytes);
    tokio::pin!(stderr_drain);
    let mut stderr_result = None;
    let mut service_completed = None;
    let result = {
        tokio::pin!(run);
        loop {
            tokio::select! {
                completed = &mut service_run => {
                    service_completed = Some(completed);
                    break Err(agent_client_protocol::Error::internal_error());
                },
                _ = fatal.cancelled() => break Err(agent_client_protocol::Error::internal_error()),
                result = &mut run => break result,
                stderr = &mut stderr_drain, if stderr_result.is_none() => {
                    stderr_result = Some(stderr);
                }
            }
        }
    };
    let mut outcome = result.map_err(|error| {
        phase_error.unwrap_or_else(|| {
            if resource_failure.load(Ordering::Acquire) {
                ClientError::ResourceLimit {
                    submitted: submitted.load(Ordering::Acquire),
                }
            } else if error.code == agent_client_protocol::schema::v1::ErrorCode::AuthRequired {
                ClientError::AuthenticationRequired {
                    submitted: submitted.load(Ordering::Acquire),
                }
            } else if agent_client_protocol::is_incoming_transport_closed(&error) {
                ClientError::Disconnected {
                    submitted: submitted.load(Ordering::Acquire),
                }
            } else {
                ClientError::Protocol {
                    submitted: submitted.load(Ordering::Acquire),
                }
            }
        })
    });
    if fatal.is_cancelled() && outcome.is_ok() {
        outcome = Err(ClientError::Protocol {
            submitted: submitted.load(Ordering::Acquire),
        });
    }
    if resource_failure.load(Ordering::Acquire) {
        outcome = Err(ClientError::ResourceLimit {
            submitted: submitted.load(Ordering::Acquire),
        });
    }
    service_shutdown.cancel();
    if !match service_completed {
        Some(completed) => completed,
        None => service_run.await,
    } {
        outcome = Err(ClientError::Teardown);
    }
    let reap_deadline = tokio::time::Instant::now() + limits.reap_timeout;
    #[cfg(windows)]
    let stopped_result = child.start_kill();
    #[cfg(not(windows))]
    let stopped_result = tokio::time::timeout_at(
        reap_deadline,
        terminate_process_tree(&mut child, process_id),
    )
    .await
    .unwrap_or_else(|_| Err(io::ErrorKind::TimedOut.into()));
    let reaped_result = tokio::time::timeout_at(reap_deadline, child.wait()).await;
    let process_reaped = matches!(reaped_result, Ok(Ok(_)));
    #[cfg(unix)]
    if process_reaped && stopped_result.is_ok() {
        group_guard.disarm();
    }
    if !process_reaped
        || stopped_result
            .as_ref()
            .is_err_and(|e| e.kind() == io::ErrorKind::TimedOut)
    {
        outcome = Err(ClientError::Teardown);
    }
    let stderr = match stderr_result {
        Some(s) => s,
        None => tokio::time::timeout(limits.reap_timeout, &mut stderr_drain)
            .await
            .unwrap_or_else(|_| "[stderr drain deadline exceeded]".into()),
    };
    let mut state = capture_state.lock().unwrap_or_else(|e| e.into_inner());
    let mut stderr = json!(stderr);
    state.redact(&mut stderr, true);
    Ok(RunResult {
        outcome,
        evidence: std::mem::take(&mut state.frames),
        evidence_truncated: state.truncated,
        stderr: stderr.as_str().unwrap_or_default().into(),
        process_reaped,
        process_tree_signal_error: stopped_result.err().map(|e| e.kind()),
    })
}

async fn drain_stderr(stderr: tokio::process::ChildStderr, max: usize) -> String {
    let mut reader = BufReader::new(stderr);
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut overflow = false;
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(count) => {
                if bytes.len() + count <= max && !overflow {
                    bytes.extend_from_slice(&buffer[..count]);
                } else {
                    overflow = true;
                    bytes.clear();
                }
            }
            Err(_) => return "[stderr read failed]".into(),
        }
    }
    if overflow {
        "[stderr capture limit exceeded]".into()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// ACP protocol and execution remain behind this adapter boundary.
pub struct AcpAdapter;
impl crate::opensymphony_domain::HarnessAdapter for AcpAdapter {
    fn harness_kind(&self) -> &'static str {
        "acp"
    }
    fn capabilities(&self) -> crate::opensymphony_gateway_schema::capability::HarnessCapability {
        crate::opensymphony_gateway_schema::capability::HarnessCapability::acp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_matching_handles_many_patterns_overlaps_and_large_input() {
        let mut secrets = (0..128)
            .map(|i| format!("credential-{i:03}-value"))
            .collect::<Vec<_>>();
        secrets.extend(["overlap".into(), "overlapping".into(), "[redacted]".into()]);
        let redactor = SecretRedactor::new(secrets).expect("bounded matcher");
        assert_eq!(
            redactor.redact("overlapping overlap credential-127-value"),
            "[redacted] [redacted] [redacted]"
        );
        let input = format!("{}credential-127-value", "x".repeat(16 * 1024 * 1024));
        let result = redactor.redact(&input);
        assert!(result.ends_with("[redacted]"));
        assert_eq!(result.len(), 16 * 1024 * 1024 + "[redacted]".len());
        assert!(SecretRedactor::new(vec!["x".repeat(1024 * 1024 + 1)]).is_err());
        assert!(SecretRedactor::new((0..1025).map(|i| format!("secret-{i}")).collect()).is_err());
    }

    #[test]
    fn terminal_reason_redaction_remains_durable_when_replacement_expands() {
        let capture = Capture {
            frames: Vec::new(),
            sequence: 0,
            truncated: false,
            max: 0,
            bytes: 0,
            max_bytes: 0,
            publisher: None,
            secrets: SecretRedactor::new(vec!["x".into()]).expect("matcher"),
        };
        assert_eq!(capture.redact_stop_reason("vendor_x"), "vendor_[redacted]");
        assert_eq!(capture.redact_stop_reason(&"x".repeat(1024)), "[redacted]");
        assert_eq!(capture.redact_stop_reason("vendor\nreason"), "[redacted]");
    }

    #[test]
    fn queued_input_enforces_both_budgets_and_releases_dispatched_bytes() {
        let mut queue = QueuedInput::default();
        assert!(queue.admit(3, 4, 8));
        assert!(queue.admit(5, 4, 8));
        assert!(!queue.admit(1, 4, 8));
        assert_eq!(queue.bytes, 8);
        assert_eq!(queue.frame_sizes.len(), 2);
        queue.dispatched();
        assert_eq!(queue.bytes, 5);
        assert!(!queue.admit(4, 4, 8));
        assert!(queue.admit(3, 4, 8));
        queue.dispatched();
        queue.dispatched();
        assert_eq!(queue.bytes, 0);
        assert!(!queue.admit(9, 4, 8));
        assert!(queue.admit(1, 1, 8));
        assert!(!queue.admit(1, 1, 8));
    }

    #[test]
    fn callback_output_budget_counts_encoded_frames_until_flush() {
        let frame = serde_json::to_string(&RawJsonRpcMessage::response(
            serde_json::from_value(json!("\"\\".repeat(128))).expect("id"),
            Err(agent_client_protocol::Error::method_not_found()),
        ))
        .expect("response frame");
        for budget in [
            frame.len(),
            frame.len() + 1,
            2 * (frame.len() + 1) - 1,
            2 * (frame.len() + 1),
        ] {
            let limits = ClientLimits {
                callback_bytes: budget,
                ..ClientLimits::default()
            };
            let mut output = CallbackOutput::default();
            let expected = budget / (frame.len() + 1);
            assert_eq!(output.admit(frame.clone(), &limits), expected >= 1);
            assert_eq!(output.admit(frame.clone(), &limits), expected >= 2);
            assert_eq!(output.bytes, expected * (frame.len() + 1));
            assert!(!output.matches_front("unreserved response"));
            if expected > 0 {
                assert!(output.matches_front(&frame));
                output.flushed();
                assert!(output.admit(frame.clone(), &limits));
            }
        }
        let mut output = CallbackOutput::default();
        let limits = ClientLimits {
            callback_frames: 1,
            ..ClientLimits::default()
        };
        assert!(output.admit(frame.clone(), &limits));
        assert!(!output.admit(frame.clone(), &limits));
        output.flushed();
        assert_eq!(output.bytes, 0);
        assert!(!output.admit(
            frame.clone(),
            &ClientLimits {
                frame_bytes: frame.len() - 1,
                ..limits
            }
        ));
    }

    #[test]
    fn concurrent_operator_reservations_keep_sdk_enqueue_order() {
        let output = Arc::new(Mutex::new(CallbackOutput::default()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let limits = ClientLimits::default();
        let (entered, first_enqueuing) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let first = {
            let output = Arc::clone(&output);
            let sent = Arc::clone(&sent);
            let limits = limits.clone();
            std::thread::spawn(move || {
                reserve_and_enqueue_operator_response(
                    &output,
                    "first".into(),
                    &limits,
                    &mut None,
                    || {
                        entered.send(()).expect("first entered SDK enqueue");
                        released.recv().expect("release first enqueue");
                        sent.lock().expect("sent frames").push("first");
                    },
                )
            })
        };
        first_enqueuing
            .recv()
            .expect("first has reserved its frame");
        assert!(
            output.try_lock().is_err(),
            "reservation must stay locked through SDK enqueue"
        );
        let second = {
            let output = Arc::clone(&output);
            let sent = Arc::clone(&sent);
            let limits = limits.clone();
            std::thread::spawn(move || {
                reserve_and_enqueue_operator_response(
                    &output,
                    "second".into(),
                    &limits,
                    &mut None,
                    || {
                        sent.lock().expect("sent frames").push("second");
                    },
                )
            })
        };
        release.send(()).expect("release first enqueue");
        first
            .join()
            .expect("first task")
            .expect("first reservation");
        second
            .join()
            .expect("second task")
            .expect("second reservation");
        assert_eq!(*sent.lock().expect("sent frames"), ["first", "second"]);
        assert_eq!(
            output
                .lock()
                .expect("reservations")
                .frames
                .iter()
                .map(|(frame, _, _)| frame.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn operator_acknowledgement_tracks_flush_or_sink_failure() {
        let mut output = CallbackOutput::default();
        let limits = ClientLimits::default();
        let (success, mut flushed) = oneshot::channel();
        let mut success = Some(success);
        assert!(output.admit_with_ack("first".into(), &limits, false, &mut success));
        assert!(success.is_none());
        assert!(
            flushed.try_recv().is_err(),
            "SDK enqueue is not transport delivery"
        );
        output.flushed();
        assert!(flushed.try_recv().expect("flush acknowledgement"));

        let (failure, mut failed) = oneshot::channel();
        assert!(output.admit_with_ack("second".into(), &limits, false, &mut Some(failure)));
        output.fail_all();
        assert!(!failed.try_recv().expect("write failure acknowledgement"));
    }

    #[test]
    fn usage_counters_survive_frame_redaction_without_exempting_credentials() {
        let capture = Capture {
            frames: Vec::new(),
            sequence: 0,
            truncated: false,
            max: 10,
            bytes: 0,
            max_bytes: 1024,
            publisher: None,
            secrets: SecretRedactor::new(vec!["987654".into()]).expect("matcher"),
        };
        for diagnostic in [false, true] {
            let mut response = json!({"id": 1, "result": {"stopReason": "end_turn", "usage": {
                "inputTokens": 4, "outputTokens": 2, "thoughtTokens": 987654,
                "cachedReadTokens": "credential", "apiToken": "secret"
            }}});
            capture.redact_frame(&mut response, diagnostic);
            assert_eq!(response["result"]["usage"]["inputTokens"], 4);
            assert_eq!(response["result"]["usage"]["outputTokens"], 2);
            for key in ["thoughtTokens", "cachedReadTokens", "apiToken"] {
                assert_eq!(response["result"]["usage"][key], "[redacted]");
            }
            let mut unrelated = json!({"method": "_vendor/private", "params": {"inputTokens": 4}});
            capture.redact_frame(&mut unrelated, diagnostic);
            assert_eq!(unrelated["params"]["inputTokens"], "[redacted]");
        }
    }

    #[test]
    fn evidence_byte_budget_includes_metadata_and_accepts_exact_boundary() {
        let frame = SourceFrame {
            sequence: 1,
            direction: "incoming".into(),
            observed_at: chrono::DateTime::from_timestamp(0, 0).expect("epoch"),
            payload: json!({"jsonrpc": "2.0", "method": "_future/notice", "params": ["a", "b"]}),
        };
        let size = serde_json::to_vec(&frame).expect("frame JSON").len();
        for budget in [0, size - 1, size, size * 2 - 1, size * 2] {
            let mut capture = Capture {
                frames: Vec::new(),
                sequence: 0,
                truncated: false,
                max: 256,
                bytes: 0,
                max_bytes: budget,
                secrets: SecretRedactor::new(Vec::new()).expect("empty matcher"),
                publisher: None,
            };
            capture.retain_frame(frame.clone());
            capture.retain_frame(frame.clone());
            assert_eq!(capture.frames.len(), budget / size);
            assert_eq!(capture.bytes, size * (budget / size));
            assert_eq!(capture.truncated, budget < size * 2);
        }
    }
}
