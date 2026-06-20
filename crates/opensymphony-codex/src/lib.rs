//! Feature-gated Codex app-server prototype helpers.
//!
//! OpenSymphony-generated JSON-RPC requests intentionally use monotonic numeric
//! IDs in this prototype. The session owns outgoing request allocation and
//! response correlation; benchmark tooling still compares IDs by string so it
//! can report future Codex response-shape changes cleanly.

use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::opensymphony_gateway_schema::model_settings::{
    ConfiguredValueSource, CredentialMode, CredentialReferenceKind, CredentialStorageMode,
    ModelSettingsProfile,
};

pub const CODEX_APP_SERVER_FEATURE: &str = "codex-app-server-prototype";
pub const CODEX_APP_SERVER_KIND: &str = "codex_app_server";
pub const CODEX_APP_SERVER_CONTRACT: &str = "codex-app-server-json-rpc-v2";

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
    ItemStarted,
    ItemCompleted,
    AgentMessageDelta,
    PlanDelta,
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
        "item/started" => NormalizedCodexEventKind::ItemStarted,
        "item/completed" => NormalizedCodexEventKind::ItemCompleted,
        "item/agentMessage/delta" => NormalizedCodexEventKind::AgentMessageDelta,
        "item/plan/delta" => NormalizedCodexEventKind::PlanDelta,
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
