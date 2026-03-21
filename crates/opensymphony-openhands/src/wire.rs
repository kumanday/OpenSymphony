//! Minimal typed wire models for the OpenHands HTTP and WebSocket contract.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use opensymphony_domain::ExecutionStatus;

use crate::error::{OpenHandsError, Result};

/// Minimal tool configuration understood by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolConfig {
    /// Tool class name, such as `TerminalTool`.
    pub name: String,
    /// Optional tool parameters.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub params: Map<String, Value>,
}

/// Minimal LLM configuration mirrored into the agent payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Provider/model selector.
    pub model: String,
    /// Optional API key value for direct local testing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Optional provider base URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Optional provider-specific API version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
    /// Optional usage identifier for telemetry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_id: Option<String>,
    /// Optional output token cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Whether streamed completion logs should be emitted.
    #[serde(default)]
    pub log_completions: bool,
    /// Output directory for streamed completion logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_completions_folder: Option<String>,
    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Minimal agent payload used during conversation creation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Optional explicit agent kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// LLM configuration for the agent.
    pub llm: LlmConfig,
    /// Tool list requested for the conversation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolConfig>,
    /// Optional subset of built-in tools to enable by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_default_tools: Vec<String>,
    /// Optional regex for tool filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_tools_regex: Option<String>,
    /// Optional MCP configuration merged into the server runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<Value>,
    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Workspace subset sent to the server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OpenHandsWorkspace {
    /// Issue workspace path used for tool execution.
    pub working_dir: String,
    /// Optional explicit workspace kind for future compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Minimal confirmation policy payload.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationPolicy {
    /// Policy kind, such as `NeverConfirm`.
    pub kind: String,
    /// Forward-compatible extension fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl ConfirmationPolicy {
    /// Builds the `NeverConfirm` policy used by the local MVP.
    #[must_use]
    pub fn never_confirm() -> Self {
        Self {
            kind: "NeverConfirm".to_string(),
            extra: Map::new(),
        }
    }
}

/// Text/image content block accepted by the message API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    /// Plain-text content.
    Text {
        /// Text body sent to the agent.
        text: String,
    },
    /// Image content with one or more URLs.
    Image {
        /// Remote image URLs attached to the message.
        image_urls: Vec<String>,
    },
}

impl ContentBlock {
    /// Builds a single text block.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }
}

/// User-message payload posted to `/events`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendMessageRequest {
    /// Sender role. OpenSymphony always uses `user`.
    pub role: String,
    /// Content blocks sent to the agent.
    pub content: Vec<ContentBlock>,
    /// Whether the server should auto-run immediately.
    pub run: bool,
    /// Optional sender tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<String>,
}

impl SendMessageRequest {
    /// Builds the local MVP user-message payload.
    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![ContentBlock::text(text)],
            run: false,
            sender: None,
        }
    }
}

/// Minimal create-conversation request subset used by OpenSymphony.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateConversationRequest {
    /// Agent definition for the conversation.
    pub agent: AgentConfig,
    /// Workspace configuration for tool execution.
    pub workspace: OpenHandsWorkspace,
    /// Stable conversation identifier to reuse per issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Local persistence directory recorded by OpenSymphony.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence_dir: Option<String>,
    /// Server-side confirmation policy.
    pub confirmation_policy: ConfirmationPolicy,
    /// Optional initial message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_message: Option<SendMessageRequest>,
    /// Maximum iterations per run.
    pub max_iterations: u32,
    /// Whether server-side stuck detection is enabled.
    pub stuck_detection: bool,
    /// Whether the server should auto-title the conversation.
    #[serde(default)]
    pub autotitle: bool,
    /// Optional server-side hook configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_config: Option<Value>,
    /// Optional server-side plugin definitions.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<Value>,
    /// Optional secrets passed to the conversation.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, Value>,
}

/// Minimal conversation snapshot returned by the REST API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationInfo {
    /// Stable conversation identifier.
    #[serde(alias = "conversation_id")]
    pub id: String,
    /// Optional conversation title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Workspace definition returned by the server.
    pub workspace: OpenHandsWorkspace,
    /// Optional agent payload returned by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Value>,
    /// Optional persistence directory reported by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persistence_dir: Option<String>,
    /// Optional iteration cap returned by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<u32>,
    /// Optional stuck-detection flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stuck_detection: Option<bool>,
    /// Current execution status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<RemoteExecutionStatus>,
    /// Optional confirmation policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_policy: Option<ConfirmationPolicy>,
    /// Optional timestamps preserved for diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Optional update timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    /// Raw forward-compatible fields.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Success response returned by simple command operations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SuccessResponse {
    /// Whether the command succeeded.
    pub success: bool,
}

/// Server metadata used for diagnostics and supervisor status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// SDK version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_version: Option<String>,
    /// Title string returned by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Uptime in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime: Option<u64>,
    /// Idle time in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_time: Option<u64>,
}

/// Paged event-search response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventPage {
    /// Events returned in this page.
    pub items: Vec<Value>,
    /// Cursor for the next page, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_id: Option<String>,
}

/// Execution statuses used by the pinned server contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteExecutionStatus {
    /// Conversation is idle and ready for work.
    Idle,
    /// Conversation is actively executing.
    Running,
    /// Conversation is paused.
    Paused,
    /// Conversation is waiting for user confirmation.
    WaitingForConfirmation,
    /// Conversation finished the current task.
    Finished,
    /// Conversation ended in an error state.
    Error,
    /// Conversation was declared stuck by the runtime.
    Stuck,
    /// Conversation is being deleted.
    Deleting,
    /// Future status value not modeled yet.
    #[serde(other)]
    Unknown,
}

impl RemoteExecutionStatus {
    /// Returns whether the remote status is terminal for a run.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Finished | Self::Error | Self::Stuck)
    }

    /// Maps the transport-level status into the normalized orchestrator-facing status.
    #[must_use]
    pub fn to_domain(self) -> ExecutionStatus {
        match self {
            Self::Idle => ExecutionStatus::Idle,
            Self::Running | Self::Paused | Self::WaitingForConfirmation => ExecutionStatus::Running,
            Self::Finished => ExecutionStatus::Success,
            Self::Error | Self::Stuck => ExecutionStatus::Error,
            Self::Deleting => ExecutionStatus::Cancelled,
            Self::Unknown => ExecutionStatus::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct BaseEventFields {
    id: String,
    timestamp: String,
    source: String,
    kind: String,
}

/// Typed subset of the state-update event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationStateUpdateEvent {
    /// Updated key, including the special `full_state` snapshot key.
    pub key: String,
    /// Raw JSON value for the update payload.
    pub value: Value,
}

/// Typed subset of streamed LLM-completion logs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LlmCompletionLogEvent {
    /// Intended filename for the completion log.
    pub filename: String,
    /// JSON-encoded completion log payload.
    pub log_data: String,
    /// Model name for context.
    #[serde(default)]
    pub model_name: String,
    /// Usage identifier of the emitting LLM.
    #[serde(default)]
    pub usage_id: String,
}

/// Typed top-level runtime error event.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConversationErrorEvent {
    /// Stable error code.
    pub code: String,
    /// Human-readable detail.
    pub detail: String,
}

/// Typed or unknown event payload retained by the event cache.
#[derive(Clone, Debug, PartialEq)]
pub enum RuntimeEventPayload {
    /// State-update event used for readiness and live mirroring.
    ConversationStateUpdate(ConversationStateUpdateEvent),
    /// Streamed LLM completion log payload.
    LlmCompletionLog(LlmCompletionLogEvent),
    /// Top-level runtime failure.
    ConversationError(ConversationErrorEvent),
    /// Any future event kind retained as raw JSON only.
    Unknown,
}

/// Event envelope retained by the adapter with typed and raw views.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeEventEnvelope {
    /// Stable event identifier.
    pub id: String,
    /// Server-provided event timestamp.
    pub timestamp: String,
    /// Event source string.
    pub source: String,
    /// Event kind discriminator.
    pub kind: String,
    /// Typed payload when the event kind is modeled.
    pub payload: RuntimeEventPayload,
    /// Original JSON retained for forward compatibility.
    pub raw_json: Value,
}

impl RuntimeEventEnvelope {
    /// Decodes an event envelope from raw JSON while preserving unknown shapes.
    pub fn from_json(raw_json: Value) -> Result<Self> {
        let base: BaseEventFields = serde_json::from_value(raw_json.clone()).map_err(|source| {
            OpenHandsError::Protocol {
                message: format!("invalid event envelope: {source}"),
            }
        })?;
        let payload = match base.kind.as_str() {
            "ConversationStateUpdateEvent" => RuntimeEventPayload::ConversationStateUpdate(
                serde_json::from_value(raw_json.clone())?,
            ),
            "LLMCompletionLogEvent" => {
                RuntimeEventPayload::LlmCompletionLog(serde_json::from_value(raw_json.clone())?)
            }
            "ConversationErrorEvent" => {
                RuntimeEventPayload::ConversationError(serde_json::from_value(raw_json.clone())?)
            }
            _ => RuntimeEventPayload::Unknown,
        };
        Ok(Self {
            id: base.id,
            timestamp: base.timestamp,
            source: base.source,
            kind: base.kind,
            payload,
            raw_json,
        })
    }

    /// Returns the current execution status when the event carries one.
    #[must_use]
    pub fn execution_status(&self) -> Option<RemoteExecutionStatus> {
        match &self.payload {
            RuntimeEventPayload::ConversationStateUpdate(event) => {
                if event.key == "execution_status" {
                    serde_json::from_value(event.value.clone()).ok()
                } else if event.key == "full_state" {
                    event
                        .value
                        .get("execution_status")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok())
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_serializes_expected_subset() {
        let request = CreateConversationRequest {
            agent: AgentConfig {
                kind: Some("Agent".to_string()),
                llm: LlmConfig {
                    model: "gpt-4o".to_string(),
                    api_key: Some("secret".to_string()),
                    base_url: None,
                    api_version: None,
                    usage_id: Some("opensymphony".to_string()),
                    max_output_tokens: None,
                    log_completions: false,
                    log_completions_folder: None,
                    extra: Map::new(),
                },
                tools: vec![ToolConfig {
                    name: "TerminalTool".to_string(),
                    params: Map::new(),
                }],
                include_default_tools: Vec::new(),
                filter_tools_regex: None,
                mcp_config: None,
                extra: Map::new(),
            },
            workspace: OpenHandsWorkspace {
                working_dir: "/tmp/workspace".to_string(),
                kind: None,
                extra: Map::new(),
            },
            conversation_id: Some("05fc9f03-9d41-4f6e-9871-b07213d1c350".to_string()),
            persistence_dir: Some(".opensymphony/openhands".to_string()),
            confirmation_policy: ConfirmationPolicy::never_confirm(),
            initial_message: Some(SendMessageRequest::user_text("hello")),
            max_iterations: 500,
            stuck_detection: true,
            autotitle: false,
            hook_config: None,
            plugins: Vec::new(),
            secrets: BTreeMap::new(),
        };

        let json = serde_json::to_value(request).expect("request should serialize");
        assert_eq!(
            json["conversation_id"],
            "05fc9f03-9d41-4f6e-9871-b07213d1c350"
        );
        assert_eq!(json["workspace"]["working_dir"], "/tmp/workspace");
        assert_eq!(json["persistence_dir"], ".opensymphony/openhands");
        assert_eq!(json["confirmation_policy"]["kind"], "NeverConfirm");
        assert_eq!(json["initial_message"]["run"], false);
    }

    #[test]
    fn decode_state_update_event_from_raw_json() {
        let raw = serde_json::json!({
            "id": "event-1",
            "timestamp": "2026-03-21T15:00:00",
            "source": "environment",
            "kind": "ConversationStateUpdateEvent",
            "key": "execution_status",
            "value": "finished"
        });
        let event = RuntimeEventEnvelope::from_json(raw).expect("event should decode");
        assert_eq!(
            event.execution_status(),
            Some(RemoteExecutionStatus::Finished)
        );
    }
}
