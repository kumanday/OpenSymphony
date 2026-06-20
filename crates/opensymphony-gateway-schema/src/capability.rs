use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// Capability discovery response for `/api/v1/capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayCapabilities {
    pub schema_version: SchemaVersion,
    pub gateway_version: String,
    pub supported_api_versions: Vec<String>,
    pub transports: Vec<TransportCapability>,
    #[serde(default)]
    pub harnesses: Vec<HarnessCapability>,
    pub features: Vec<FeatureCapability>,
    pub auth_modes: Vec<AuthMode>,
    pub max_event_page_size: u32,
    pub max_terminal_frame_batch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportCapability {
    pub transport: String,
    pub modes: Vec<String>,
    pub supported_encodings: Vec<String>,
    pub bidirectional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureCapability {
    pub feature: String,
    pub available: bool,
    pub requires_auth: bool,
    pub requires_plan: Option<String>,
}

/// Public harness capability metadata exposed through `/api/v1/capabilities`.
///
/// This DTO intentionally uses stable strings for harness and transport names so
/// clients can render unknown future harnesses without depending on private Rust
/// adapter types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessCapability {
    pub kind: String,
    pub display_name: String,
    pub available: bool,
    pub adapter_contract_version: String,
    pub runtime_contract_version: Option<String>,
    pub actions: HarnessActionCapability,
    pub event_streams: HarnessEventStreamCapability,
    pub approvals: HarnessApprovalCapability,
    pub model_settings: HarnessModelSettingsCapability,
    pub transport: HarnessTransportCapability,
    pub cancellation: HarnessCancellationCapability,
    pub pause_resume: HarnessPauseResumeCapability,
    pub history: HarnessHistoryCapability,
    pub notes: Vec<String>,
    pub feature_gaps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessActionCapability {
    pub start_run: bool,
    pub send_user_message: bool,
    pub retry: bool,
    pub cancel: bool,
    pub pause: bool,
    pub resume: bool,
    pub approve: bool,
    pub reject: bool,
    pub comment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessEventStreamCapability {
    pub runtime_events: bool,
    pub terminal_frames: bool,
    pub replay_from_cursor: bool,
    pub raw_payload_refs: bool,
    pub delivery_modes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessApprovalCapability {
    pub tool_approval: bool,
    pub human_decision: bool,
    pub policy_metadata: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessModelSettingsCapability {
    pub api_compatible_settings: bool,
    pub subscription_credentials: bool,
    pub per_run_overrides: bool,
    pub credential_reference_kinds: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessTransportCapability {
    pub protocol: String,
    pub modes: Vec<String>,
    pub local: bool,
    pub remote: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessCancellationCapability {
    pub cancel_run: bool,
    pub force_stop: bool,
    pub acknowledges_cancel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessPauseResumeCapability {
    pub pause: bool,
    pub resume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessHistoryCapability {
    pub fetch_history: bool,
    pub reconcile_after_ready: bool,
    pub reconnect_and_replay: bool,
    pub preserve_unknown_events: bool,
}

impl HarnessCapability {
    pub fn openhands_agent_server() -> Self {
        Self {
            kind: "openhands_agent_server".into(),
            display_name: "OpenHands agent-server".into(),
            available: true,
            adapter_contract_version: "harness-adapter-v1".into(),
            runtime_contract_version: Some("openhands-sdk-agent-server-v1".into()),
            actions: HarnessActionCapability {
                start_run: true,
                send_user_message: true,
                retry: true,
                cancel: true,
                pause: false,
                resume: false,
                approve: false,
                reject: false,
                comment: true,
            },
            event_streams: HarnessEventStreamCapability {
                runtime_events: true,
                terminal_frames: true,
                replay_from_cursor: true,
                raw_payload_refs: true,
                delivery_modes: vec!["http_history".into(), "websocket".into()],
            },
            approvals: HarnessApprovalCapability {
                tool_approval: false,
                human_decision: false,
                policy_metadata: false,
            },
            model_settings: HarnessModelSettingsCapability {
                api_compatible_settings: true,
                subscription_credentials: false,
                per_run_overrides: true,
                credential_reference_kinds: vec!["env".into()],
            },
            transport: HarnessTransportCapability {
                protocol: "http_websocket".into(),
                modes: vec!["rest".into(), "websocket".into()],
                local: true,
                remote: true,
            },
            cancellation: HarnessCancellationCapability {
                cancel_run: true,
                force_stop: true,
                acknowledges_cancel: true,
            },
            pause_resume: HarnessPauseResumeCapability {
                pause: false,
                resume: false,
            },
            history: HarnessHistoryCapability {
                fetch_history: true,
                reconcile_after_ready: true,
                reconnect_and_replay: true,
                preserve_unknown_events: true,
            },
            notes: vec![
                "Initial production adapter; reuses one conversation per issue by default.".into(),
            ],
            feature_gaps: vec![
                "OpenHands pause/resume is not exposed by the current agent-server contract."
                    .into(),
                "Approval center normalization is reserved for a follow-up harness phase.".into(),
            ],
        }
    }

    pub fn codex_app_server_future() -> Self {
        Self {
            kind: "codex_app_server".into(),
            display_name: "Codex app-server".into(),
            available: false,
            adapter_contract_version: "harness-adapter-v1".into(),
            runtime_contract_version: None,
            actions: HarnessActionCapability {
                start_run: true,
                send_user_message: true,
                retry: true,
                cancel: true,
                pause: false,
                resume: false,
                approve: true,
                reject: true,
                comment: true,
            },
            event_streams: HarnessEventStreamCapability {
                runtime_events: true,
                terminal_frames: true,
                replay_from_cursor: true,
                raw_payload_refs: true,
                delivery_modes: vec!["json_rpc_notifications".into()],
            },
            approvals: HarnessApprovalCapability {
                tool_approval: true,
                human_decision: true,
                policy_metadata: true,
            },
            model_settings: HarnessModelSettingsCapability {
                api_compatible_settings: true,
                subscription_credentials: true,
                per_run_overrides: true,
                credential_reference_kinds: vec![
                    "model_settings_ref".into(),
                    "inherited_subscription_login".into(),
                    "capability_token".into(),
                    "signed_bearer".into(),
                ],
            },
            transport: HarnessTransportCapability {
                protocol: "json_rpc_2_0".into(),
                modes: vec!["stdio".into(), "websocket_experimental".into()],
                local: true,
                remote: true,
            },
            cancellation: HarnessCancellationCapability {
                cancel_run: true,
                force_stop: false,
                acknowledges_cancel: true,
            },
            pause_resume: HarnessPauseResumeCapability {
                pause: false,
                resume: false,
            },
            history: HarnessHistoryCapability {
                fetch_history: true,
                reconcile_after_ready: true,
                reconnect_and_replay: true,
                preserve_unknown_events: true,
            },
            notes: vec!["Future adapter shaped around JSON-RPC requests and notifications.".into()],
            feature_gaps: vec![
                "Production adapter implementation is out of scope for COE-408.".into(),
                "Pause/resume semantics need protocol confirmation before being advertised as available."
                    .into(),
            ],
        }
    }

    pub fn rust_native_future() -> Self {
        Self {
            kind: "rust_native".into(),
            display_name: "Rust-native harness".into(),
            available: false,
            adapter_contract_version: "harness-adapter-v1".into(),
            runtime_contract_version: None,
            actions: HarnessActionCapability {
                start_run: true,
                send_user_message: true,
                retry: true,
                cancel: true,
                pause: true,
                resume: true,
                approve: true,
                reject: true,
                comment: true,
            },
            event_streams: HarnessEventStreamCapability {
                runtime_events: true,
                terminal_frames: true,
                replay_from_cursor: true,
                raw_payload_refs: true,
                delivery_modes: vec!["in_process".into(), "subprocess_rpc".into()],
            },
            approvals: HarnessApprovalCapability {
                tool_approval: true,
                human_decision: true,
                policy_metadata: true,
            },
            model_settings: HarnessModelSettingsCapability {
                api_compatible_settings: true,
                subscription_credentials: true,
                per_run_overrides: true,
                credential_reference_kinds: vec!["model_settings_ref".into()],
            },
            transport: HarnessTransportCapability {
                protocol: "in_process_or_rpc".into(),
                modes: vec!["in_process".into(), "subprocess_rpc".into()],
                local: true,
                remote: false,
            },
            cancellation: HarnessCancellationCapability {
                cancel_run: true,
                force_stop: true,
                acknowledges_cancel: true,
            },
            pause_resume: HarnessPauseResumeCapability {
                pause: true,
                resume: true,
            },
            history: HarnessHistoryCapability {
                fetch_history: true,
                reconcile_after_ready: true,
                reconnect_and_replay: true,
                preserve_unknown_events: true,
            },
            notes: vec![
                "Future high-performance local adapter for stable Rust execution APIs.".into(),
            ],
            feature_gaps: vec!["Concrete SDK/runtime selection is not implemented yet.".into()],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    None,
    ApiKey,
    BearerToken,
    SubscriptionOAuth,
}
