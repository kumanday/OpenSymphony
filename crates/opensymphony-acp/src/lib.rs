//! Executable ACP v1 stdio client. Durable ownership and worker routing live in the host.
//!
//! `run_turn` owns one process for one complete turn; it never resumes or retries a prompt.
//! A protocol stop reason is distinct from proof that remote delegated work has stopped.
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use agent_client_protocol::{
    Client, Dispatch, Lines, UntypedMessage,
    schema::{
        ProtocolVersion,
        v1::{
            AuthMethod, AuthenticateRequest, CancelNotification, ContentBlock, InitializeRequest,
            InitializeResponse, NewSessionRequest, PromptRequest, RequestPermissionOutcome,
            RequestPermissionRequest, RequestPermissionResponse, TextContent,
        },
    },
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, BufReader},
    process::Command,
    sync::mpsc,
};
use tokio_util::{
    codec::{FramedRead, FramedWrite, LinesCodec},
    sync::CancellationToken,
};
use tracing::instrument::WithSubscriber;

#[cfg(windows)]
mod windows_process;

pub use crate::opensymphony_workflow::AcpProfile;
use crate::opensymphony_workspace::{
    environment_variable_names_equal, insert_environment_value, redact_runtime_diagnostic,
    sanitize_workspace_key,
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
}

#[derive(Debug, Clone)]
pub struct ClientLimits {
    pub frame_bytes: usize,
    pub queued_frames: usize,
    pub evidence_frames: usize,
    /// Cumulative serialized bytes of retained, redacted SourceFrame values.
    pub evidence_bytes: usize,
    pub stderr_bytes: usize,
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
            evidence_frames: 256,
            evidence_bytes: 1024 * 1024,
            stderr_bytes: 16 * 1024,
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

struct Capture {
    frames: Vec<SourceFrame>,
    sequence: u64,
    truncated: bool,
    max: usize,
    bytes: usize,
    max_bytes: usize,
    secrets: Vec<String>,
}

impl Capture {
    fn redact(&self, value: &mut Value, diagnostic: bool) {
        match value {
            Value::String(s) => {
                for secret in &self.secrets {
                    *s = s.replace(secret, "[redacted]");
                }
                if diagnostic {
                    *s = redact_runtime_diagnostic(s);
                }
            }
            Value::Array(values) => values.iter_mut().for_each(|v| self.redact(v, diagnostic)),
            Value::Object(values) => {
                for (mut key, mut value) in std::mem::take(values) {
                    if [
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
                    for secret in &self.secrets {
                        key = key.replace(secret, "[redacted]");
                    }
                    values.insert(key, value);
                }
            }
            _ => {}
        }
    }
    fn record(&mut self, direction: &str, mut payload: Value) {
        self.sequence += 1;
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

fn io_failure() -> io::Error {
    io::Error::other("ACP transport rejected a frame")
}

struct PreparedLaunch {
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    secrets: Vec<String>,
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
        || limits.evidence_frames > 4096
        || limits.evidence_bytes > 16 * 1024 * 1024
        || limits.stderr_bytes > 1024 * 1024
        || [
            limits.setup_timeout,
            limits.prompt_timeout,
            limits.cancel_timeout,
            limits.reap_timeout,
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
        if profile.command.contains(value) || profile.args.iter().any(|a| a.contains(value)) {
            return Err(ClientError::InvalidConfiguration(
                "referenced credentials cannot appear in argv".into(),
            ));
        }
        secrets.push(value.clone());
        insert_environment_value(&mut environment, target.clone(), value.clone());
    }
    for (key, value) in &context.environment {
        if !value.is_empty()
            && (excluded(key)
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
    if secrets.iter().any(|secret| {
        profile.command.contains(secret) || profile.args.iter().any(|arg| arg.contains(secret))
    }) {
        return Err(ClientError::InvalidConfiguration(
            "credentials cannot appear in argv".into(),
        ));
    }
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    secrets.dedup();
    Ok(PreparedLaunch {
        cwd,
        environment,
        secrets,
    })
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
    }));
    let mut command = Command::new(&profile.command);
    command
        .args(&profile.args)
        .current_dir(&cwd)
        .env_clear()
        .envs(environment)
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
    let stdin = child.stdin.take().ok_or(ClientError::Teardown)?;
    let stdout = child.stdout.take().ok_or(ClientError::Teardown)?;
    let stderr = child.stderr.take().ok_or(ClientError::Teardown)?;
    let queued = Arc::new(AtomicUsize::new(0));
    let resource_failure = Arc::new(AtomicBool::new(false));
    let submitted = Arc::new(AtomicBool::new(false));
    let fatal = CancellationToken::new();
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
            if queued.fetch_add(1, Ordering::AcqRel) >= limits.queued_frames {
                resource_failure.store(true, Ordering::Release);
                return Err(io_failure());
            }
            capture(&capture_state, "incoming", value);
            Ok(line)
        }
    });
    let output = FramedWrite::new(stdin, LinesCodec::new_with_max_length(limits.frame_bytes));
    let outgoing = <_ as SinkExt<String>>::sink_map_err(output, |_| io_failure()).with({
        let capture_state = capture_state.clone();
        let resource_failure = resource_failure.clone();
        let submitted = submitted.clone();
        move |line: String| {
            let result = if line.len() > limits.frame_bytes {
                resource_failure.store(true, Ordering::Release);
                Err(io_failure())
            } else {
                serde_json::from_str::<Value>(&line)
                    .map(|value| {
                        // The complete serialized envelope passed the local size gate.
                        // Any subsequent transport failure can leave remote execution uncertain.
                        if value.get("method").and_then(Value::as_str) == Some("session/prompt") {
                            submitted.store(true, Ordering::Release);
                        }
                        capture(&capture_state, "outgoing", value);
                        line
                    })
                    .map_err(|_| io_failure())
            };
            std::future::ready(result)
        }
    });
    let active_session = Arc::new(Mutex::new(None::<String>));
    let client = Client.builder().on_receive_dispatch(
        {
            let active_session = active_session.clone();
            let resource_failure = resource_failure.clone();
            let fatal = fatal.clone();
            let redactor = capture_state.clone();
            async move |message: Dispatch, _cx| {
                queued.fetch_sub(1, Ordering::AcqRel);
                match message {
                    Dispatch::Request(request, responder) => {
                        if request.method == "session/request_permission" {
                            let request: RequestPermissionRequest =
                                serde_json::from_value(request.params)
                                    .map_err(|_| agent_client_protocol::Error::invalid_params())?;
                            if active_session
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .as_deref()
                                != Some(request.session_id.0.as_ref())
                            {
                                return responder.respond_with_error(
                                    agent_client_protocol::Error::invalid_params(),
                                );
                            }
                            // No operator policy is advertised in this slice. Cancel promptly and never grant access.
                            responder.respond(serde_json::to_value(
                                RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled),
                            )?)
                        } else {
                            responder.respond_with_error(
                                    agent_client_protocol::Error::method_not_found(),
                                )
                        }
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
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await?;
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
                        .block_task().await?;
                }
                let session = connection.send_request(NewSessionRequest::new(cwd)).block_task().await?;
                *active_session.lock().unwrap_or_else(|e| e.into_inner()) = Some(session.session_id.0.to_string());
                Ok((initialization, session.session_id))
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
            }?;
            let stop_reason = result.get("stopReason").and_then(Value::as_str)
                .ok_or_else(agent_client_protocol::Error::invalid_params)?.to_owned();
            Ok(TurnReport {
                cancellation_acknowledged: cancellation_requested && stop_reason == "cancelled",
                cancellation_requested, stop_reason, session_id: session_id.0.to_string(), initialization,
            })
        })
        .with_subscriber(tracing::subscriber::NoSubscriber::default());
    let stderr_drain = drain_stderr(stderr, limits.stderr_bytes);
    tokio::pin!(stderr_drain);
    let mut stderr_result = None;
    let result = {
        tokio::pin!(run);
        loop {
            tokio::select! {
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
                secrets: Vec::new(),
            };
            capture.retain_frame(frame.clone());
            capture.retain_frame(frame.clone());
            assert_eq!(capture.frames.len(), budget / size);
            assert_eq!(capture.bytes, size * (budget / size));
            assert_eq!(capture.truncated, budget < size * 2);
        }
    }
}
