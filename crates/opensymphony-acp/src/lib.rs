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
            RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
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
mod host;
mod services;
mod session_config;
pub use host::*;
pub use services::HostServices;
pub use session_config::SessionConfiguration;
#[cfg(windows)]
mod windows_path;
#[cfg(windows)]
mod windows_process;

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
/// `workspace_key` is the scheduler's sanitized checkout key (including repository suffixes).
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
    frames: VecDeque<String>,
    bytes: usize,
}

impl CallbackOutput {
    fn admit(&mut self, frame: String, limits: &ClientLimits) -> bool {
        let bytes = frame.len() + 1; // LinesCodec adds LF.
        if frame.len() > limits.frame_bytes
            || self.frames.len() >= limits.callback_frames
            || bytes > limits.callback_bytes.saturating_sub(self.bytes)
        {
            return false;
        }
        self.bytes += bytes;
        self.frames.push_back(frame);
        true
    }

    fn matches_front(&self, frame: &str) -> bool {
        self.frames
            .front()
            .is_some_and(|expected| expected == frame)
    }

    fn flushed(&mut self) {
        if let Some(frame) = self.frames.pop_front() {
            self.bytes -= frame.len() + 1;
        }
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
    fn record(&mut self, direction: &str, mut payload: Value) {
        self.sequence += 1;
        if let Some(servers) = payload
            .pointer_mut("/params/mcpServers")
            .and_then(Value::as_array_mut)
        {
            for server in servers {
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
            self.redact(&mut source, false);
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
        self.redact(&mut payload, true);
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
            limits.prompt_timeout,
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
    for server in &context.services.mcp_servers {
        use agent_client_protocol::schema::v1::McpServer;
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
                let sensitive_argument = |flag: &str| {
                    runtime_field_is_sensitive(flag) || flag.eq_ignore_ascii_case("--oauth2-bearer")
                };
                for (index, argument) in server.args.iter().enumerate() {
                    let secret = match argument.split_once('=') {
                        Some((flag, value)) if sensitive_argument(flag) => Some(value),
                        _ if index > 0 && sensitive_argument(&server.args[index - 1]) => {
                            Some(argument.as_str())
                        }
                        _ => None,
                    };
                    if let Some(value) = secret.filter(|value| !value.is_empty()) {
                        secrets.push(value.to_owned());
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
        || url
            .query_pairs()
            .any(|(key, _)| runtime_field_is_sensitive(&key))
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
        .current_dir(&cwd)
        .env_clear()
        .envs(&environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
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
    let queued = Arc::new(Mutex::new(QueuedInput::default()));
    let callback_output = Arc::new(Mutex::new(CallbackOutput::default()));
    let resource_failure = Arc::new(AtomicBool::new(false));
    let submitted = Arc::new(AtomicBool::new(false));
    let fatal = CancellationToken::new();
    let service_shutdown = CancellationToken::new();
    let _service_guard = service_shutdown.clone().drop_guard();
    let (service_sender, service_actor) = services::Services::new(
        cwd.clone(),
        environment,
        context.services.clone(),
        limits.clone(),
        callback_output.clone(),
        resource_failure.clone(),
        fatal.clone(),
        CancellationToken::new(),
        service_shutdown.clone(),
    );
    let service_run = service_actor.run();
    tokio::pin!(service_run);
    let configuration = Arc::new(Mutex::new(SessionConfiguration::default()));
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
                    return Err(io_failure());
                }
                let value: Value = serde_json::from_str(&line).map_err(|_| io_failure())?;
                let is_response = value.get("method").is_none();
                if is_response
                    && !callback_output
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .matches_front(&line)
                {
                    // Invalid incoming RPC envelopes can provoke SDK-generated errors.
                    // They must not bypass callback admission or release another charge.
                    return Err(io_failure());
                }
                if value.get("method").and_then(Value::as_str) == Some("session/prompt") {
                    submitted.store(true, Ordering::Release);
                }
                capture(&capture_state, "outgoing", value);
                output.send(line).await.map_err(|_| io_failure())?;
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
    let client = Client.builder().on_receive_dispatch(
        {
            let active_session = active_session.clone();
            let callback_output = callback_output.clone();
            let limits = limits.clone();
            let resource_failure = resource_failure.clone();
            let fatal = fatal.clone();
            let redactor = capture_state.clone();
            let configuration = configuration.clone();
            let service_sender = service_sender.clone();
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
                        let response = if request.method == "session/request_permission" {
                            match serde_json::from_value::<RequestPermissionRequest>(request.params)
                            {
                                Ok(request)
                                    if active_session
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .as_deref()
                                        == Some(request.session_id.0.as_ref()) =>
                                {
                                    // No operator policy is advertised in this slice.
                                    serde_json::to_value(RequestPermissionResponse::new(
                                        RequestPermissionOutcome::Cancelled,
                                    ))
                                    .map_err(agent_client_protocol::Error::from)
                                }
                                _ => Err(agent_client_protocol::Error::invalid_params()),
                            }
                        } else {
                            Err(agent_client_protocol::Error::method_not_found())
                        };
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
                            if active_session
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_deref()
                                != Some(id)
                            {
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
                            if let Err(error) = service_sender.begin_turn(cancellation.child_token(), limits.setup_timeout).await {
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
                    let session_configuration = configuration.clone();
                    // Bind in ordered response dispatch before any adjacent session update.
                    connection.send_request(NewSessionRequest::new(cwd.clone()).mcp_servers(context.services.mcp_servers.clone()))
                        .on_receiving_result(async move |result| {
                            if let Ok(session) = &result {
                                *active_session.lock().unwrap_or_else(|e| e.into_inner()) =
                                    Some(session.session_id.0.to_string());
                                session_configuration.lock().unwrap_or_else(|e| e.into_inner()).initial(&serde_json::to_value(session)?);
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
                    && let Err(error) = service_sender.begin_turn(cancellation.clone(), limits.setup_timeout).await {
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
            let response = connection.send_request(request).block_task();
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
                _ = tokio::time::sleep(limits.prompt_timeout) => {
                    phase_error = Some(ClientError::PromptTimeout);
                    return Err(agent_client_protocol::Error::internal_error());
                }
            }.inspect_err(|error| {
                phase_error = rpc_failure("session/prompt", error, &capture_state, submitted.load(Ordering::Acquire));
            })?;
            let stop_reason = result.get("stopReason").and_then(Value::as_str)
                .ok_or_else(agent_client_protocol::Error::invalid_params)?.to_owned();
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
