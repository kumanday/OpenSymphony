//! Codex app-server local harness helpers.
//!
//! OpenSymphony-generated JSON-RPC requests intentionally use monotonic numeric
//! IDs. The session owns outgoing request allocation and response correlation;
//! benchmark tooling still compares IDs by string so it can report future Codex
//! response-shape changes cleanly.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::opensymphony_gateway_schema::model_settings::{
    ConfiguredValueSource, CredentialMode, CredentialReferenceKind, CredentialStorageMode,
    ModelSettingsProfile,
};
use crate::{
    opensymphony_domain::HarnessAdapter,
    opensymphony_gateway_schema::{
        approval::{
            ApprovalActor, ApprovalKind, ApprovalRequest, ApprovalRiskLevel, ApprovalRiskSummary,
            ApprovalStatus, ApprovalTargetContext,
        },
        capability::HarnessCapability,
        envelope::EntityRef,
        event_journal::{EventActor, EventKind, EventRecord},
        version::SchemaVersion,
    },
};

pub const CODEX_APP_SERVER_KIND: &str = "codex_app_server";
pub const CODEX_APP_SERVER_CONTRACT: &str = "codex-app-server-json-rpc-v2";

#[derive(Debug, thiserror::Error)]
pub enum CodexSchemaValidationError {
    #[error("failed to read installed Codex app-server schema at {path}: {source}")]
    SchemaRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse installed Codex app-server schema JSON: {0}")]
    SchemaParse(#[from] serde_json::Error),
    #[error("installed Codex app-server schema has unexpected shape: {0}")]
    SchemaShape(String),
    #[error("failed to compile installed Codex app-server schema: {0}")]
    SchemaCompile(String),
    #[error("failed to serialize Codex JSON-RPC request for schema validation: {0}")]
    Serialize(String),
    #[error(
        "installed Codex app-server schema rejected `{method}` request: {errors}. Update Codex, or update OpenSymphony's Codex adapter if the installed schema is newer and incompatible."
    )]
    Invalid { method: String, errors: String },
}

#[derive(Debug, Clone)]
pub struct CodexAppServerSchemaValidator {
    validator: jsonschema::Validator,
}

impl CodexAppServerSchemaValidator {
    pub fn from_schema_file(path: impl AsRef<Path>) -> Result<Self, CodexSchemaValidationError> {
        let path = path.as_ref();
        let schema =
            fs::read_to_string(path).map_err(|source| CodexSchemaValidationError::SchemaRead {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_schema_str(&schema)
    }

    pub fn from_schema_str(schema: &str) -> Result<Self, CodexSchemaValidationError> {
        Self::from_schema_json(serde_json::from_str(schema)?)
    }

    pub fn from_schema_json(schema: Value) -> Result<Self, CodexSchemaValidationError> {
        let (definitions_key, definitions) = schema
            .get("definitions")
            .map(|definitions| ("definitions", definitions))
            .or_else(|| {
                schema
                    .get("$defs")
                    .map(|definitions| ("$defs", definitions))
            })
            .ok_or_else(|| {
                CodexSchemaValidationError::SchemaShape(
                    "missing top-level definitions or $defs object".into(),
                )
            })?;
        if definitions.get("ClientRequest").is_none() {
            return Err(CodexSchemaValidationError::SchemaShape(
                "missing definitions/$defs ClientRequest schema".into(),
            ));
        }
        let mut client_request_schema = serde_json::Map::new();
        if let Some(schema_uri) = schema.get("$schema") {
            client_request_schema.insert("$schema".into(), schema_uri.clone());
        }
        if let Some(schema_id) = schema.get("$id") {
            client_request_schema.insert("$id".into(), schema_id.clone());
        }
        client_request_schema.insert(
            "$ref".into(),
            Value::String(format!("#/{definitions_key}/ClientRequest")),
        );
        client_request_schema.insert(definitions_key.into(), definitions.clone());
        let client_request_schema = Value::Object(client_request_schema);
        let validator = jsonschema::validator_for(&client_request_schema)
            .map_err(|error| CodexSchemaValidationError::SchemaCompile(error.to_string()))?;
        Ok(Self { validator })
    }

    pub fn validate_request(
        &self,
        request: &JsonRpcRequestEnvelope,
    ) -> Result<(), CodexSchemaValidationError> {
        let value = serde_json::to_value(request)
            .map_err(|error| CodexSchemaValidationError::Serialize(error.to_string()))?;
        let errors = self
            .validator
            .iter_errors(&value)
            .take(5)
            .map(|error| format!("{error} at {}", error.instance_path()))
            .collect::<Vec<_>>();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(CodexSchemaValidationError::Invalid {
                method: request.method.clone(),
                errors: errors.join("; "),
            })
        }
    }
}

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

    pub fn start_issue_thread_request(
        &self,
        session: &mut CodexJsonRpcSession,
        cwd: impl Into<String>,
        model: Option<String>,
        config: Value,
    ) -> Result<CodexHarnessRequest, serde_json::Error> {
        Ok(CodexHarnessRequest {
            lifecycle: CodexLifecycleRequest::Start,
            request: session.thread_start(CodexThreadStartParams {
                approval_policy: Some(CodexApprovalPolicy::Never),
                cwd: Some(cwd.into()),
                model,
                model_provider: None,
                base_instructions: None,
                developer_instructions: None,
                ephemeral: Some(false),
                sandbox: Some(CodexThreadSandboxMode::DangerFullAccess),
                config: Some(config),
            })?,
        })
    }

    pub fn start_issue_turn_request(
        &self,
        session: &mut CodexJsonRpcSession,
        thread_id: impl Into<String>,
        cwd: impl Into<String>,
        model: Option<String>,
        workflow_prompt: impl Into<String>,
    ) -> Result<CodexHarnessRequest, serde_json::Error> {
        Ok(CodexHarnessRequest {
            lifecycle: CodexLifecycleRequest::Start,
            request: session.turn_start(CodexTurnStartParams {
                thread_id: thread_id.into(),
                input: vec![CodexUserInput::Text {
                    text: workflow_prompt.into(),
                    text_elements: Vec::new(),
                }],
                approval_policy: Some(CodexApprovalPolicy::Never),
                cwd: Some(cwd.into()),
                model,
                sandbox_policy: Some(CodexSandboxPolicy::danger_full_access()),
                client_user_message_id: None,
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
                approval_policy: Some(CodexApprovalPolicy::Never),
                cwd: Some(cwd.into()),
                model: None,
                sandbox_policy: Some(CodexSandboxPolicy::danger_full_access()),
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
        let mut args = vec![
            "--dangerously-bypass-hook-trust".into(),
            "app-server".into(),
        ];
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
    pub approval_policy: Option<CodexApprovalPolicy>,
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
    pub sandbox: Option<CodexThreadSandboxMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexApprovalPolicy {
    #[serde(rename = "never")]
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexThreadSandboxMode {
    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexSandboxPolicy {
    #[serde(rename = "type")]
    pub policy_type: CodexSandboxPolicyType,
}

impl CodexSandboxPolicy {
    pub fn danger_full_access() -> Self {
        Self {
            policy_type: CodexSandboxPolicyType::DangerFullAccess,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodexSandboxPolicyType {
    #[serde(rename = "dangerFullAccess")]
    DangerFullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexTurnStartParams {
    pub thread_id: String,
    pub input: Vec<CodexUserInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<CodexApprovalPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_policy: Option<CodexSandboxPolicy>,
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
    TokenUsageUpdated,
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
    pub token_usage: Option<CodexTokenUsage>,
    pub raw: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexTokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
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
        "thread/tokenUsage/updated" => NormalizedCodexEventKind::TokenUsageUpdated,
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
        token_usage: codex_token_usage(&params),
        raw,
    })
}

fn string_param(params: &Value, key: &str) -> Option<String> {
    params.get(key)?.as_str().map(str::to_owned)
}

fn codex_token_usage(params: &Value) -> Option<CodexTokenUsage> {
    let usage = params.get("tokenUsage")?;
    let total = usage.get("total").unwrap_or(usage);
    let input_tokens = u64_param(total, &["inputTokens", "input_tokens", "prompt_tokens"]);
    let output_tokens = u64_param(
        total,
        &["outputTokens", "output_tokens", "completion_tokens"],
    );
    let cache_read_tokens = u64_param(
        total,
        &["cachedInputTokens", "cacheReadTokens", "cache_read_tokens"],
    );
    let explicit_total_tokens = u64_param(total, &["totalTokens", "total_tokens"]);
    let total_tokens = if explicit_total_tokens > 0 {
        explicit_total_tokens
    } else {
        input_tokens + output_tokens + cache_read_tokens
    };
    if input_tokens == 0 && output_tokens == 0 && cache_read_tokens == 0 && total_tokens == 0 {
        return None;
    }
    Some(CodexTokenUsage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        total_tokens,
    })
}

fn u64_param(params: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| params.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

pub fn codex_event_payload(event: &NormalizedCodexEvent) -> Value {
    let mut payload = json!({
        "source_kind": event.method,
        "thread_id": event.thread_id,
        "turn_id": event.turn_id,
        "item_id": event.item_id,
        "message_delta": event.message_delta,
        "raw_payload": event.raw,
    });
    if let Some(usage) = event.token_usage
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            "usage".into(),
            json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "cache_read_tokens": usage.cache_read_tokens,
                "total_tokens": usage.total_tokens,
            }),
        );
    }
    payload
}

pub fn normalized_event_to_journal_record(
    run_id: impl Into<String>,
    sequence: u64,
    event: &NormalizedCodexEvent,
) -> EventRecord {
    let run_id = run_id.into();
    let (kind, summary) = codex_event_journal_kind_and_summary(event);
    EventRecord::builder()
        .sequence(sequence)
        .actor(EventActor::harness(CODEX_APP_SERVER_KIND))
        .entity_ref(EntityRef::run(run_id.clone()))
        .summary(summary)
        .kind(kind)
        .payload(codex_event_payload(event))
        .raw_payload_ref(format!("codex:{run_id}:{sequence}"))
        .build()
}

pub fn codex_approval_request_from_event(
    run_id: impl Into<String>,
    issue_id: impl Into<String>,
    issue_identifier: impl Into<String>,
    requested_at: DateTime<Utc>,
    event: &NormalizedCodexEvent,
) -> Option<ApprovalRequest> {
    if event.kind != NormalizedCodexEventKind::ApprovalRequested {
        return None;
    }
    let run_id = run_id.into();
    let issue_id = issue_id.into();
    let issue_identifier = issue_identifier.into();
    let params = event.raw.get("params").unwrap_or(&Value::Null);
    let approval_id =
        first_string_param(params, &["approvalId", "approval_id", "itemId", "item_id"])
            .or_else(|| event.item_id.clone())?;
    let command = first_string_param(params, &["command", "shellCommand", "toolCommand"]);
    let file_path = first_string_param(params, &["filePath", "path"]);
    let title = first_string_param(params, &["title", "label"])
        .or_else(|| {
            command
                .as_ref()
                .map(|command| format!("Approve command `{command}`"))
        })
        .unwrap_or_else(|| "Codex approval request".into());
    let description = first_string_param(params, &["description", "message", "reason"])
        .unwrap_or_else(|| "Codex requested operator approval before continuing.".into());
    let kind = if command.is_some() {
        ApprovalKind::CommandExecution
    } else if file_path.is_some() {
        ApprovalKind::FileWrite
    } else {
        ApprovalKind::ToolUse
    };
    let correlation_id = [
        event.thread_id.as_deref(),
        event.turn_id.as_deref(),
        Some(approval_id.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(":");

    Some(ApprovalRequest {
        schema_version: SchemaVersion::v1(),
        approval_id,
        run_id: run_id.clone(),
        issue_id: issue_id.clone(),
        kind,
        title,
        description,
        proposed_action: params.as_object().map(|_| params.clone()),
        actor: Some(ApprovalActor {
            actor_id: CODEX_APP_SERVER_KIND.into(),
            actor_kind: "harness".into(),
            display_name: Some("Codex app-server".into()),
        }),
        target_context: Some(ApprovalTargetContext {
            file_path,
            command,
            issue_id: Some(issue_id),
            issue_identifier: Some(issue_identifier),
            run_id: Some(run_id),
        }),
        risk_summary: Some(codex_approval_risk_summary(params)),
        requested_at,
        expires_at: None,
        status: ApprovalStatus::Pending,
        correlation_id,
        decided_at: None,
    })
}

pub fn codex_approval_decision_audit_record(
    run_id: impl Into<String>,
    sequence: u64,
    approval_id: impl Into<String>,
    decision: CodexApprovalDecision,
    message: Option<String>,
) -> EventRecord {
    let run_id = run_id.into();
    let approval_id = approval_id.into();
    let (kind, summary) = match decision {
        CodexApprovalDecision::Approve => (
            EventKind::ApprovalGranted,
            format!("Codex approval {approval_id} decision recorded for gateway forwarding"),
        ),
        CodexApprovalDecision::Reject => (
            EventKind::ApprovalDenied,
            format!("Codex approval {approval_id} rejection recorded for gateway forwarding"),
        ),
    };
    EventRecord::builder()
        .sequence(sequence)
        .actor(EventActor::system("opensymphony_approval_bridge"))
        .entity_ref(EntityRef::run(run_id.clone()))
        .summary(summary)
        .kind(kind)
        .payload(json!({
            "approval_id": approval_id,
            "decision": decision.as_protocol_value(),
            "message": message,
            "harness_kind": CODEX_APP_SERVER_KIND,
        }))
        .raw_payload_ref(format!("codex:{run_id}:approval-decision:{sequence}"))
        .build()
}

fn first_string_param(params: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| string_param(params, key))
}

fn codex_approval_risk_summary(params: &Value) -> ApprovalRiskSummary {
    let mut reasons = Vec::new();
    let command = first_string_param(params, &["command", "shellCommand", "toolCommand"]);
    let normalized_command = command.as_ref().map(|command| command.to_ascii_lowercase());
    let file_path = first_string_param(params, &["filePath", "path"]);
    let level = match (normalized_command.as_deref(), file_path.as_deref()) {
        (Some(command), _)
            if command.contains("sudo")
                || command.contains("rm -rf")
                || command.contains("chmod")
                || command.contains("chown") =>
        {
            reasons.push("Command can mutate privileged or destructive host state.".into());
            ApprovalRiskLevel::High
        }
        (Some(_), _) => {
            reasons.push("Command execution requires explicit operator approval.".into());
            ApprovalRiskLevel::Medium
        }
        (None, Some(_)) => {
            reasons.push("File write can mutate workspace or host state.".into());
            ApprovalRiskLevel::Medium
        }
        (None, None) => ApprovalRiskLevel::Unknown,
    };
    ApprovalRiskSummary { level, reasons }
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
        NormalizedCodexEventKind::TokenUsageUpdated => (
            EventKind::HarnessEventNormalized {
                source_kind: event.method.clone(),
            },
            "Codex token usage updated".into(),
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
