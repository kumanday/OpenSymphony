//! Codex app-server local harness helpers.
//!
//! OpenSymphony-generated JSON-RPC requests intentionally use monotonic numeric
//! IDs. The session owns outgoing request allocation and response correlation;
//! benchmark tooling still compares IDs by string so it can report future Codex
//! response-shape changes cleanly.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::opensymphony_gateway_schema::model_settings::{
    ConfiguredValueSource, CredentialMode, CredentialReferenceKind, CredentialStorageMode,
    ModelSettingsProfile,
};
use crate::{
    opensymphony_domain::HarnessAdapter,
    opensymphony_gateway_schema::{
        capability::HarnessCapability,
        envelope::EntityRef,
        event_journal::{EventActor, EventKind, EventRecord},
    },
};

pub const CODEX_APP_SERVER_KIND: &str = "codex_app_server";
pub const CODEX_APP_SERVER_CONTRACT: &str = "codex-app-server-json-rpc-v2";
pub const CODEX_DEFAULT_MODEL_PROVIDER: &str = "openai";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexContractArtifact {
    JsonSchema,
    TypeScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexContractGeneration {
    program: String,
    artifact: CodexContractArtifact,
    out_dir: PathBuf,
}

impl CodexContractGeneration {
    pub fn json_schema(out_dir: impl Into<PathBuf>) -> Self {
        Self::json_schema_with_program("codex", out_dir)
    }

    pub fn json_schema_with_program(
        program: impl Into<String>,
        out_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            artifact: CodexContractArtifact::JsonSchema,
            out_dir: out_dir.into(),
        }
    }

    pub fn typescript(out_dir: impl Into<PathBuf>) -> Self {
        Self::typescript_with_program("codex", out_dir)
    }

    pub fn typescript_with_program(
        program: impl Into<String>,
        out_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            artifact: CodexContractArtifact::TypeScript,
            out_dir: out_dir.into(),
        }
    }

    pub fn artifact(&self) -> CodexContractArtifact {
        self.artifact
    }

    pub fn to_command(&self) -> (String, Vec<String>) {
        let generator = match self.artifact {
            CodexContractArtifact::JsonSchema => "generate-json-schema",
            CodexContractArtifact::TypeScript => "generate-ts",
        };
        (
            self.program.clone(),
            vec![
                "app-server".into(),
                generator.into(),
                "--out".into(),
                self.out_dir.to_string_lossy().to_string(),
            ],
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAppServerAdapter {
    launch: CodexAppServerLaunch,
    client_name: String,
    client_version: String,
}

impl CodexAppServerAdapter {
    pub fn local_stdio(
        program: impl Into<String>,
        client_name: impl Into<String>,
        client_version: impl Into<String>,
    ) -> Self {
        Self {
            launch: CodexAppServerLaunch::stdio_with_program(program),
            client_name: client_name.into(),
            client_version: client_version.into(),
        }
    }

    pub fn launch(&self) -> &CodexAppServerLaunch {
        &self.launch
    }

    pub fn session(&self) -> CodexJsonRpcSession {
        CodexJsonRpcSession::new(self.client_name.clone(), self.client_version.clone())
    }

    pub fn start_issue_request(
        &self,
        session: &mut CodexJsonRpcSession,
        cwd: impl Into<String>,
        model: impl Into<String>,
        workflow_prompt: impl Into<String>,
        config: Value,
    ) -> Result<CodexHarnessRequest, serde_json::Error> {
        Ok(CodexHarnessRequest {
            lifecycle: CodexLifecycleRequest::Start,
            request: session.thread_start(CodexThreadStartParams {
                cwd: Some(cwd.into()),
                model: Some(model.into()),
                // Codex CLI app-server currently exposes OpenAI/ChatGPT-backed
                // model ids through this local harness path.
                model_provider: Some(CODEX_DEFAULT_MODEL_PROVIDER.into()),
                base_instructions: Some(workflow_prompt.into()),
                developer_instructions: None,
                ephemeral: Some(false),
                config: Some(config),
            })?,
        })
    }

    pub fn resume_issue_request(
        &self,
        session: &mut CodexJsonRpcSession,
        thread_id: impl Into<String>,
        cwd: impl Into<String>,
        continuation: impl Into<String>,
    ) -> Result<CodexHarnessRequest, serde_json::Error> {
        Ok(CodexHarnessRequest {
            lifecycle: CodexLifecycleRequest::Resume,
            request: session.turn_start(CodexTurnStartParams {
                thread_id: thread_id.into(),
                input: vec![CodexUserInput::Text {
                    text: continuation.into(),
                    text_elements: Vec::new(),
                }],
                cwd: Some(cwd.into()),
                model: None,
                client_user_message_id: None,
            })?,
        })
    }

    pub fn cancel_turn_request(
        &self,
        session: &mut CodexJsonRpcSession,
        turn_id: impl Into<String>,
    ) -> CodexHarnessRequest {
        CodexHarnessRequest {
            lifecycle: CodexLifecycleRequest::Cancel,
            request: session.request("turn/cancel", json!({ "turnId": turn_id.into() })),
        }
    }

    pub fn approval_response(
        &self,
        session: &mut CodexJsonRpcSession,
        approval_id: impl Into<String>,
        decision: CodexApprovalDecision,
        message: Option<String>,
    ) -> CodexHarnessRequest {
        let mut params = json!({
            "approvalId": approval_id.into(),
            "decision": decision.as_protocol_value(),
        });
        if let Some(message) = message
            && let Some(object) = params.as_object_mut()
        {
            object.insert("message".into(), Value::String(message));
        }
        CodexHarnessRequest {
            lifecycle: CodexLifecycleRequest::Approval,
            request: session.request("approval/respond", params),
        }
    }
}

impl HarnessAdapter for CodexAppServerAdapter {
    fn harness_kind(&self) -> &'static str {
        CODEX_APP_SERVER_KIND
    }

    fn capabilities(&self) -> HarnessCapability {
        HarnessCapability::codex_app_server_local()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexLifecycleRequest {
    Start,
    Resume,
    Cancel,
    Approval,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodexHarnessRequest {
    pub lifecycle: CodexLifecycleRequest,
    pub request: JsonRpcRequestEnvelope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexApprovalDecision {
    Approve,
    Reject,
}

impl CodexApprovalDecision {
    fn as_protocol_value(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Reject => "reject",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexAppServerTransport {
    Stdio,
    WebSocketLoopback { port: u16 },
}

/// WebSocket auth arguments for the Codex app-server prototype.
///
/// This prototype's launch helpers return UTF-8 `String` arguments for stable
/// tests and documentation snapshots. Token and shared-secret file paths are
/// therefore limited to paths that can be passed losslessly as UTF-8; non-UTF-8
/// local paths are out of scope until a production adapter carries `OsString`
/// arguments or constructs a `std::process::Command` directly.
///
/// Auth file paths and token hashes are passed as command-line arguments by the
/// current Codex CLI contract, so they may be visible through process-list
/// inspection on shared hosts. Do not use this prototype with real shared-host
/// secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexWebSocketAuth {
    CapabilityToken {
        token_file: PathBuf,
        token_sha256: String,
    },
    SignedBearerToken {
        shared_secret_file: PathBuf,
        issuer: String,
        audience: String,
        max_clock_skew_seconds: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexAppServerLaunch {
    program: String,
    pub transport: CodexAppServerTransport,
    pub extra_args: Vec<String>,
    pub websocket_auth: Option<CodexWebSocketAuth>,
}

impl CodexAppServerLaunch {
    pub fn stdio() -> Self {
        Self::stdio_with_program("codex")
    }

    pub fn stdio_with_program(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            transport: CodexAppServerTransport::Stdio,
            extra_args: Vec::new(),
            websocket_auth: None,
        }
    }

    pub fn loopback_websocket(port: u16) -> Self {
        Self::loopback_websocket_with_program("codex", port)
    }

    pub fn loopback_websocket_with_program(program: impl Into<String>, port: u16) -> Self {
        Self {
            program: program.into(),
            transport: CodexAppServerTransport::WebSocketLoopback { port },
            extra_args: Vec::new(),
            websocket_auth: None,
        }
    }

    pub fn program(&self) -> &str {
        &self.program
    }

    pub fn to_command(&self) -> (String, Vec<String>) {
        (self.program.clone(), self.command_args())
    }

    /// Builds UTF-8 CLI arguments for prototype evidence and tests.
    ///
    /// Auth file paths are converted with `Path::to_string_lossy()` because this
    /// helper returns `Vec<String>`. Use UTF-8 paths with the prototype; a
    /// production harness should preserve native path bytes with `OsString` or
    /// `std::process::Command` arguments.
    ///
    /// Auth-related CLI arguments may be visible in process listings. Keep
    /// prototype runs to local trusted environments and avoid real shared-host
    /// secrets.
    pub fn command_args(&self) -> Vec<String> {
        let mut args = vec!["app-server".into()];
        args.extend(self.extra_args.clone());
        match &self.transport {
            CodexAppServerTransport::Stdio => args.push("--stdio".into()),
            CodexAppServerTransport::WebSocketLoopback { port } => {
                args.extend(["--listen".into(), format!("ws://127.0.0.1:{port}")]);
                if let Some(auth) = &self.websocket_auth {
                    match auth {
                        CodexWebSocketAuth::CapabilityToken {
                            token_file,
                            token_sha256,
                        } => {
                            args.extend([
                                "--ws-auth".into(),
                                "capability-token".into(),
                                "--ws-token-file".into(),
                                token_file.to_string_lossy().to_string(),
                                "--ws-token-sha256".into(),
                                token_sha256.clone(),
                            ]);
                        }
                        CodexWebSocketAuth::SignedBearerToken {
                            shared_secret_file,
                            issuer,
                            audience,
                            max_clock_skew_seconds,
                        } => {
                            args.extend([
                                "--ws-auth".into(),
                                "signed-bearer-token".into(),
                                "--ws-shared-secret-file".into(),
                                shared_secret_file.to_string_lossy().to_string(),
                                "--ws-issuer".into(),
                                issuer.clone(),
                                "--ws-audience".into(),
                                audience.clone(),
                            ]);
                            if let Some(seconds) = max_clock_skew_seconds {
                                args.extend([
                                    "--ws-max-clock-skew-seconds".into(),
                                    seconds.to_string(),
                                ]);
                            }
                        }
                    }
                }
            }
        }
        args
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexModelCredentialReuse {
    pub profile_id: String,
    pub model_source: ConfiguredValueSource,
    pub model_reference: String,
    pub credential_reference_id: String,
    pub credential_reference_kind: CredentialReferenceKind,
    pub storage_mode: CredentialStorageMode,
    pub can_supply_subscription_credentials: bool,
    pub config_overrides: BTreeMap<String, String>,
}

impl CodexModelCredentialReuse {
    pub fn from_profile(profile: &ModelSettingsProfile) -> Option<Self> {
        if !profile
            .compatible_harnesses
            .iter()
            .any(|kind| kind == CODEX_APP_SERVER_KIND)
        {
            return None;
        }

        let mut config_overrides = BTreeMap::new();
        if profile.model.source == ConfiguredValueSource::Literal {
            config_overrides.insert("model".into(), profile.model.reference.clone());
        }

        Some(Self {
            profile_id: profile.id.clone(),
            model_source: profile.model.source,
            model_reference: profile.model.reference.clone(),
            credential_reference_id: profile.credential_reference.id.clone(),
            credential_reference_kind: profile.credential_reference.kind,
            storage_mode: profile.storage_mode,
            can_supply_subscription_credentials: profile.credential_mode
                == CredentialMode::Subscription,
            config_overrides,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequestEnvelope {
    pub jsonrpc: String,
    /// OpenSymphony-generated prototype requests use numeric JSON-RPC IDs so a
    /// session can allocate a monotonic sequence and correlate responses without
    /// accepting client-supplied null IDs.
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexJsonRpcSession {
    next_id: u64,
    pub client_name: String,
    pub client_version: String,
}

impl CodexJsonRpcSession {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            next_id: 1,
            client_name: client_name.into(),
            client_version: client_version.into(),
        }
    }

    pub fn initialize(&mut self) -> JsonRpcRequestEnvelope {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": self.client_name,
                    "version": self.client_version,
                },
                "capabilities": {},
            }),
        )
    }

    pub fn thread_start(
        &mut self,
        params: CodexThreadStartParams,
    ) -> Result<JsonRpcRequestEnvelope, serde_json::Error> {
        Ok(self.request("thread/start", serde_json::to_value(params)?))
    }

    pub fn turn_start(
        &mut self,
        params: CodexTurnStartParams,
    ) -> Result<JsonRpcRequestEnvelope, serde_json::Error> {
        Ok(self.request("turn/start", serde_json::to_value(params)?))
    }

    pub fn request(&mut self, method: impl Into<String>, params: Value) -> JsonRpcRequestEnvelope {
        let id = self.next_id;
        self.next_id += 1;
        JsonRpcRequestEnvelope {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        }
    }

    pub fn encode_line(request: &JsonRpcRequestEnvelope) -> Result<String, serde_json::Error> {
        serde_json::to_string(request).map(|line| format!("{line}\n"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub developer_instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTurnStartParams {
    pub thread_id: String,
    pub input: Vec<CodexUserInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_user_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CodexUserInput {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(default)]
        text_elements: Vec<Value>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedCodexEventKind {
    ThreadStarted,
    ThreadStatusChanged,
    TurnStarted,
    TurnCompleted,
    TurnCancelled,
    ItemStarted,
    ItemCompleted,
    AgentMessageDelta,
    PlanDelta,
    ApprovalRequested,
    ApprovalCompleted,
    Error,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedCodexEvent {
    pub kind: NormalizedCodexEventKind,
    pub method: String,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub message_delta: Option<String>,
    pub raw: Value,
}

/// Normalize a server notification after the transport layer has accepted the
/// JSON-RPC envelope version.
pub fn normalize_server_notification(raw: Value) -> Option<NormalizedCodexEvent> {
    if raw.get("id").is_some() {
        return None;
    }
    let method = raw.get("method")?.as_str()?.to_owned();
    let params = raw.get("params").cloned().unwrap_or(Value::Null);
    let kind = match method.as_str() {
        "thread/started" => NormalizedCodexEventKind::ThreadStarted,
        "thread/status/changed" => NormalizedCodexEventKind::ThreadStatusChanged,
        "turn/started" => NormalizedCodexEventKind::TurnStarted,
        "turn/completed" => NormalizedCodexEventKind::TurnCompleted,
        "turn/cancelled" | "turn/canceled" => NormalizedCodexEventKind::TurnCancelled,
        "item/started" => NormalizedCodexEventKind::ItemStarted,
        "item/completed" => NormalizedCodexEventKind::ItemCompleted,
        "item/agentMessage/delta" => NormalizedCodexEventKind::AgentMessageDelta,
        "item/plan/delta" => NormalizedCodexEventKind::PlanDelta,
        "item/permissions/requestApproval" => NormalizedCodexEventKind::ApprovalRequested,
        "approval/completed" => NormalizedCodexEventKind::ApprovalCompleted,
        "error" => NormalizedCodexEventKind::Error,
        _ => NormalizedCodexEventKind::Unknown,
    };

    Some(NormalizedCodexEvent {
        kind,
        method,
        thread_id: string_param(&params, "threadId"),
        turn_id: string_param(&params, "turnId"),
        item_id: string_param(&params, "itemId"),
        message_delta: string_param(&params, "delta"),
        raw,
    })
}

fn string_param(params: &Value, key: &str) -> Option<String> {
    params.get(key)?.as_str().map(str::to_owned)
}

pub fn normalized_event_to_journal_record(
    run_id: impl Into<String>,
    sequence: u64,
    event: &NormalizedCodexEvent,
) -> EventRecord {
    let run_id = run_id.into();
    let payload = json!({
        "source_kind": event.method,
        "thread_id": event.thread_id,
        "turn_id": event.turn_id,
        "item_id": event.item_id,
        "message_delta": event.message_delta,
        "raw_payload": event.raw,
    });
    let (kind, summary) = codex_event_journal_kind_and_summary(event);
    EventRecord::builder()
        .sequence(sequence)
        .actor(EventActor::harness(CODEX_APP_SERVER_KIND))
        .entity_ref(EntityRef::run(run_id.clone()))
        .summary(summary)
        .kind(kind)
        .payload(payload)
        .raw_payload_ref(format!("codex:{run_id}:{sequence}"))
        .build()
}

fn codex_event_journal_kind_and_summary(event: &NormalizedCodexEvent) -> (EventKind, String) {
    match event.kind {
        NormalizedCodexEventKind::ThreadStarted => (
            EventKind::HarnessEventNormalized {
                source_kind: event.method.clone(),
            },
            format!(
                "Codex thread started{}",
                id_suffix(event.thread_id.as_deref())
            ),
        ),
        NormalizedCodexEventKind::TurnStarted => (
            EventKind::HarnessEventNormalized {
                source_kind: event.method.clone(),
            },
            format!("Codex turn started{}", id_suffix(event.turn_id.as_deref())),
        ),
        NormalizedCodexEventKind::TurnCompleted => (
            EventKind::HarnessEventNormalized {
                source_kind: event.method.clone(),
            },
            format!(
                "Codex turn completed{}",
                id_suffix(event.turn_id.as_deref())
            ),
        ),
        NormalizedCodexEventKind::TurnCancelled => (
            EventKind::HarnessEventNormalized {
                source_kind: event.method.clone(),
            },
            format!(
                "Codex turn cancelled{}",
                id_suffix(event.turn_id.as_deref())
            ),
        ),
        NormalizedCodexEventKind::ApprovalRequested => (
            EventKind::ApprovalRequested,
            format!(
                "Codex requested approval{}",
                id_suffix(event.item_id.as_deref())
            ),
        ),
        NormalizedCodexEventKind::ApprovalCompleted => (
            approval_completed_kind(event),
            format!(
                "Codex approval completed{}",
                id_suffix(event.item_id.as_deref())
            ),
        ),
        NormalizedCodexEventKind::Error => (
            EventKind::RunFailed,
            error_summary(event).unwrap_or_else(|| "Codex app-server reported an error".into()),
        ),
        NormalizedCodexEventKind::ThreadStatusChanged => (
            thread_status_kind(event),
            format!("Codex event: {}", event.method),
        ),
        NormalizedCodexEventKind::ItemStarted
        | NormalizedCodexEventKind::ItemCompleted
        | NormalizedCodexEventKind::AgentMessageDelta
        | NormalizedCodexEventKind::PlanDelta => (
            EventKind::HarnessEventNormalized {
                source_kind: event.method.clone(),
            },
            format!("Codex event: {}", event.method),
        ),
        NormalizedCodexEventKind::Unknown => (
            EventKind::Unknown {
                raw_kind: event.method.clone(),
            },
            format!("Codex event: {}", event.method),
        ),
    }
}

fn id_suffix(id: Option<&str>) -> String {
    id.map(|value| format!(" {value}")).unwrap_or_default()
}

fn error_summary(event: &NormalizedCodexEvent) -> Option<String> {
    let params = event.raw.get("params")?;
    let message = params.get("message")?.as_str()?;
    Some(format!("Codex app-server error: {message}"))
}

fn approval_completed_kind(event: &NormalizedCodexEvent) -> EventKind {
    let params = event.raw.get("params").unwrap_or(&Value::Null);
    let decision = params
        .get("decision")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    match decision.as_deref() {
        Some("approve") => EventKind::ApprovalGranted,
        Some("reject") => EventKind::ApprovalDenied,
        _ => EventKind::HarnessEventNormalized {
            source_kind: event.method.clone(),
        },
    }
}

fn thread_status_kind(event: &NormalizedCodexEvent) -> EventKind {
    let params = event.raw.get("params").unwrap_or(&Value::Null);
    let status = params
        .get("status")
        .and_then(Value::as_str)
        .map(str::to_ascii_lowercase);
    match status.as_deref() {
        Some("completed") => EventKind::RunCompleted,
        Some("failed") => EventKind::RunFailed,
        Some("cancelled") => EventKind::RunCancelled,
        _ => EventKind::HarnessEventNormalized {
            source_kind: event.method.clone(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexBenchmarkRequirement {
    pub dimension: &'static str,
    pub probe: &'static str,
    pub acceptance_signal: &'static str,
}

pub fn websocket_benchmark_requirements() -> Vec<CodexBenchmarkRequirement> {
    vec![
        CodexBenchmarkRequirement {
            dimension: "throughput",
            probe: "send a batch of JSON-RPC thread/loaded/list requests over ws://127.0.0.1",
            acceptance_signal: "all responses arrive with matching ids and measured requests/sec",
        },
        CodexBenchmarkRequirement {
            dimension: "queue behavior",
            probe: "enqueue many requests without awaiting per-request responses",
            acceptance_signal: "response count matches sent count and p50/p95 latency is recorded",
        },
        CodexBenchmarkRequirement {
            dimension: "reconnect",
            probe: "close the WebSocket, reconnect, and run initialize again",
            acceptance_signal: "new connection reaches ready state and responds after reconnect",
        },
        CodexBenchmarkRequirement {
            dimension: "secure exposure",
            probe: "verify localhost-only default plus capability-token and signed-bearer flags",
            acceptance_signal: "non-loopback exposure remains gated by explicit auth settings",
        },
    ]
}
