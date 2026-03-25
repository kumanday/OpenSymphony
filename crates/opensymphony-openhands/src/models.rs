use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, ser::SerializeMap};
use serde_json::{Value, json};
use uuid::Uuid;

fn default_true() -> bool {
    true
}

pub const LLM_SUMMARIZING_CONDENSER_KIND: &str = "LLMSummarizingCondenser";
pub const LLM_SUMMARIZING_CONDENSER_USAGE_ID: &str = "condenser";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub working_dir: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmationPolicy {
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmConfig {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_id: Option<String>,
}

impl LlmConfig {
    pub fn with_usage_id(&self, usage_id: impl Into<String>) -> Self {
        let mut cloned = self.clone();
        cloned.usage_id = Some(usage_id.into());
        cloned
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CondenserConfig {
    pub kind: String,
    pub llm: LlmConfig,
    pub max_size: u64,
    pub keep_first: u64,
}

impl CondenserConfig {
    pub fn llm_summarizing(llm: LlmConfig, max_size: u64, keep_first: u64) -> Self {
        Self {
            kind: LLM_SUMMARIZING_CONDENSER_KIND.to_string(),
            llm: llm.with_usage_id(LLM_SUMMARIZING_CONDENSER_USAGE_ID),
            max_size,
            keep_first,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    pub kind: String,
    pub llm: LlmConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condenser: Option<CondenserConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stdio_servers: Vec<McpStdioServerConfig>,
}

impl McpConfig {
    pub fn from_stdio_servers(stdio_servers: Vec<McpStdioServerConfig>) -> Option<Self> {
        if stdio_servers.is_empty() {
            None
        } else {
            Some(Self { stdio_servers })
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpStdioServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationCreateRequest {
    pub conversation_id: Uuid,
    pub workspace: WorkspaceConfig,
    pub persistence_dir: String,
    pub max_iterations: u32,
    pub stuck_detection: bool,
    pub confirmation_policy: ConfirmationPolicy,
    pub agent: AgentConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config: Option<McpConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorProbeConfig {
    pub max_iterations: u32,
    pub stuck_detection: bool,
    pub confirmation_policy_kind: String,
    pub agent_kind: String,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub mcp_config: Option<McpConfig>,
}

impl Default for DoctorProbeConfig {
    fn default() -> Self {
        Self {
            max_iterations: 4,
            stuck_detection: true,
            confirmation_policy_kind: "NeverConfirm".to_string(),
            agent_kind: "Agent".to_string(),
            model: None,
            api_key: None,
            base_url: None,
            mcp_config: None,
        }
    }
}

impl ConversationCreateRequest {
    pub fn doctor_probe(
        working_dir: impl Into<String>,
        persistence_dir: impl Into<String>,
        model: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        Self::doctor_probe_with_config(
            working_dir,
            persistence_dir,
            DoctorProbeConfig {
                model,
                api_key,
                ..DoctorProbeConfig::default()
            },
        )
    }

    pub fn doctor_probe_with_config(
        working_dir: impl Into<String>,
        persistence_dir: impl Into<String>,
        config: DoctorProbeConfig,
    ) -> Self {
        let model = config.model.unwrap_or_else(|| "openai/gpt-5.4".to_string());
        Self {
            conversation_id: Uuid::new_v4(),
            workspace: WorkspaceConfig {
                working_dir: working_dir.into(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: persistence_dir.into(),
            max_iterations: config.max_iterations,
            stuck_detection: config.stuck_detection,
            confirmation_policy: ConfirmationPolicy {
                kind: config.confirmation_policy_kind,
            },
            agent: AgentConfig {
                kind: config.agent_kind,
                llm: LlmConfig {
                    model,
                    api_key: config.api_key,
                    base_url: config.base_url,
                    usage_id: None,
                },
                condenser: None,
            },
            mcp_config: config.mcp_config,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conversation {
    #[serde(alias = "id")]
    pub conversation_id: Uuid,
    pub workspace: WorkspaceConfig,
    pub persistence_dir: String,
    pub max_iterations: u32,
    pub stuck_detection: bool,
    pub execution_status: String,
    pub confirmation_policy: ConfirmationPolicy,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextContent {
    pub r#type: String,
    pub text: String,
    #[serde(default)]
    pub cache_prompt: bool,
}

impl TextContent {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            r#type: "text".to_string(),
            text: value.into(),
            cache_prompt: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SendMessageRequest {
    pub role: String,
    pub content: Vec<TextContent>,
    #[serde(default)]
    pub run: bool,
}

impl SendMessageRequest {
    pub fn user_text(value: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: vec![TextContent::text(value)],
            run: false,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationRunRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcceptedResponse {
    #[serde(default = "default_true")]
    pub success: bool,
}

impl AcceptedResponse {
    pub fn accepted() -> Self {
        Self { success: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationStateUpdatePayload {
    #[serde(default)]
    pub execution_status: Option<String>,
    #[serde(default)]
    pub state_delta: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventEnvelope {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub kind: String,
    pub payload: Value,
    pub key: Option<String>,
    pub value: Option<Value>,
}

impl EventEnvelope {
    pub fn new(
        id: impl Into<String>,
        timestamp: DateTime<Utc>,
        source: impl Into<String>,
        kind: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            id: id.into(),
            timestamp,
            source: source.into(),
            kind: kind.into(),
            payload,
            key: None,
            value: None,
        }
    }

    pub fn state_update(id: impl Into<String>, execution_status: impl Into<String>) -> Self {
        let execution_status = execution_status.into();
        Self {
            id: id.into(),
            timestamp: Utc::now(),
            source: "runtime".to_string(),
            kind: "ConversationStateUpdateEvent".to_string(),
            payload: json!({
                "execution_status": execution_status,
                "state_delta": {
                    "execution_status": execution_status,
                },
            }),
            key: None,
            value: None,
        }
    }
}

impl Serialize for EventEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("id", &self.id)?;
        map.serialize_entry("timestamp", &self.timestamp.to_rfc3339())?;
        map.serialize_entry("source", &self.source)?;
        map.serialize_entry("kind", &self.kind)?;
        if let Some(key) = &self.key {
            map.serialize_entry("key", key)?;
        }
        if let Some(value) = &self.value {
            map.serialize_entry("value", value)?;
        }

        match &self.payload {
            Value::Object(payload) => {
                for (key, value) in payload {
                    map.serialize_entry(key, value)?;
                }
            }
            Value::Null => {}
            other => {
                map.serialize_entry("payload", other)?;
            }
        }

        map.end()
    }
}

impl<'de> Deserialize<'de> for EventEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut raw = serde_json::Map::<String, Value>::deserialize(deserializer)?;
        let id = take_required_string(&mut raw, "id")?;
        let timestamp = take_required_timestamp(&mut raw, "timestamp")?;
        let source = take_optional_string(&mut raw, "source").unwrap_or_default();
        let kind = take_required_string(&mut raw, "kind")?;
        let key = take_optional_string(&mut raw, "key");
        let value = raw.remove("value");
        let nested_payload = raw.remove("payload");
        let payload = match (nested_payload, raw.is_empty()) {
            (Some(Value::Object(payload)), true) => Value::Object(payload),
            (Some(Value::Null), true) | (None, true) => Value::Object(raw),
            (Some(payload), true) => payload,
            (Some(payload), false) => {
                raw.insert("payload".to_string(), payload);
                Value::Object(raw)
            }
            (None, false) => Value::Object(raw),
        };

        Ok(Self {
            id,
            timestamp,
            source,
            kind,
            payload,
            key,
            value,
        })
    }
}

fn take_required_string<E>(
    raw: &mut serde_json::Map<String, Value>,
    field: &str,
) -> Result<String, E>
where
    E: serde::de::Error,
{
    raw.remove(field)
        .ok_or_else(|| E::custom(format!("missing required field `{field}`")))?
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| E::custom(format!("expected `{field}` to be a string")))
}

fn take_optional_string(raw: &mut serde_json::Map<String, Value>, field: &str) -> Option<String> {
    raw.remove(field)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
}

fn take_required_timestamp<E>(
    raw: &mut serde_json::Map<String, Value>,
    field: &str,
) -> Result<DateTime<Utc>, E>
where
    E: serde::de::Error,
{
    let value = raw
        .remove(field)
        .ok_or_else(|| E::custom(format!("missing required field `{field}`")))?;
    let raw_timestamp = value
        .as_str()
        .ok_or_else(|| E::custom(format!("expected `{field}` to be a string timestamp")))?;

    if let Ok(timestamp) = DateTime::parse_from_rfc3339(raw_timestamp) {
        return Ok(timestamp.with_timezone(&Utc));
    }

    chrono::NaiveDateTime::parse_from_str(raw_timestamp, "%Y-%m-%dT%H:%M:%S%.f")
        .map(|naive| DateTime::from_naive_utc_and_offset(naive, Utc))
        .map_err(E::custom)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchConversationEventsResponse {
    #[serde(alias = "items")]
    pub events: Vec<EventEnvelope>,
    #[serde(default)]
    pub next_page_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn conversation_create_request_serializes_minimal_contract() {
        let request = ConversationCreateRequest {
            conversation_id: Uuid::parse_str("11111111-2222-3333-4444-555555555555")
                .expect("uuid should parse"),
            workspace: WorkspaceConfig {
                working_dir: "/tmp/workspace".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/workspace/.opensymphony/openhands".to_string(),
            max_iterations: 7,
            stuck_detection: true,
            confirmation_policy: ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: AgentConfig {
                kind: "Agent".to_string(),
                llm: LlmConfig {
                    model: "fake-model".to_string(),
                    api_key: Some("fake-key".to_string()),
                    base_url: None,
                    usage_id: None,
                },
                condenser: None,
            },
            mcp_config: None,
        };

        let value = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "conversation_id": "11111111-2222-3333-4444-555555555555",
                "workspace": {
                    "working_dir": "/tmp/workspace",
                    "kind": "LocalWorkspace",
                },
                "persistence_dir": "/tmp/workspace/.opensymphony/openhands",
                "max_iterations": 7,
                "stuck_detection": true,
                "confirmation_policy": {
                    "kind": "NeverConfirm",
                },
                "agent": {
                    "kind": "Agent",
                    "llm": {
                        "model": "fake-model",
                        "api_key": "fake-key",
                    },
                },
            })
        );
    }

    #[test]
    fn conversation_create_request_serializes_mcp_stdio_subset() {
        let request = ConversationCreateRequest {
            conversation_id: Uuid::parse_str("11111111-2222-3333-4444-555555555555")
                .expect("uuid should parse"),
            workspace: WorkspaceConfig {
                working_dir: "/tmp/workspace".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/workspace/.opensymphony/openhands".to_string(),
            max_iterations: 7,
            stuck_detection: true,
            confirmation_policy: ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: AgentConfig {
                kind: "Agent".to_string(),
                llm: LlmConfig {
                    model: "fake-model".to_string(),
                    api_key: Some("fake-key".to_string()),
                    base_url: None,
                    usage_id: None,
                },
                condenser: None,
            },
            mcp_config: Some(McpConfig {
                stdio_servers: vec![McpStdioServerConfig {
                    name: "linear".to_string(),
                    command: "opensymphony".to_string(),
                    args: vec!["linear-mcp".to_string()],
                    env: BTreeMap::new(),
                }],
            }),
        };

        let value = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(
            value,
            json!({
                "conversation_id": "11111111-2222-3333-4444-555555555555",
                "workspace": {
                    "working_dir": "/tmp/workspace",
                    "kind": "LocalWorkspace",
                },
                "persistence_dir": "/tmp/workspace/.opensymphony/openhands",
                "max_iterations": 7,
                "stuck_detection": true,
                "confirmation_policy": {
                    "kind": "NeverConfirm",
                },
                "agent": {
                    "kind": "Agent",
                    "llm": {
                        "model": "fake-model",
                        "api_key": "fake-key",
                    },
                },
                "mcp_config": {
                    "stdio_servers": [
                        {
                            "name": "linear",
                            "command": "opensymphony",
                            "args": ["linear-mcp"],
                        }
                    ],
                },
            })
        );
    }

    #[test]
    fn conversation_run_request_serializes_to_empty_object() {
        let value = serde_json::to_value(ConversationRunRequest::default())
            .expect("request should serialize");

        assert_eq!(value, json!({}));
    }

    #[test]
    fn conversation_create_request_serializes_optional_condenser() {
        let request = ConversationCreateRequest {
            conversation_id: Uuid::parse_str("11111111-2222-3333-4444-555555555555")
                .expect("uuid should parse"),
            workspace: WorkspaceConfig {
                working_dir: "/tmp/workspace".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/workspace/.opensymphony/openhands".to_string(),
            max_iterations: 7,
            stuck_detection: true,
            confirmation_policy: ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: AgentConfig {
                kind: "Agent".to_string(),
                llm: LlmConfig {
                    model: "fake-model".to_string(),
                    api_key: Some("fake-key".to_string()),
                    base_url: Some("https://example.com/v1".to_string()),
                    usage_id: None,
                },
                condenser: Some(CondenserConfig::llm_summarizing(
                    LlmConfig {
                        model: "fake-model".to_string(),
                        api_key: Some("fake-key".to_string()),
                        base_url: Some("https://example.com/v1".to_string()),
                        usage_id: None,
                    },
                    240,
                    2,
                )),
            },
            mcp_config: None,
        };

        let value = serde_json::to_value(&request).expect("request should serialize");

        assert_eq!(
            value["agent"]["condenser"],
            json!({
                "kind": "LLMSummarizingCondenser",
                "llm": {
                    "model": "fake-model",
                    "api_key": "fake-key",
                    "base_url": "https://example.com/v1",
                    "usage_id": "condenser",
                },
                "max_size": 240,
                "keep_first": 2,
            })
        );
    }

    #[test]
    fn event_envelope_deserializes_flattened_agent_server_events() {
        let value = json!({
            "id": "evt-message",
            "timestamp": "2026-03-23T12:07:58.942514",
            "source": "user",
            "kind": "MessageEvent",
            "llm_message": {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "hello",
                        "cache_prompt": false,
                    }
                ],
                "thinking_blocks": [],
            },
            "activated_skills": [],
            "extended_content": [],
        });

        let event: EventEnvelope =
            serde_json::from_value(value).expect("flattened event should decode");

        assert_eq!(event.kind, "MessageEvent");
        assert_eq!(
            event
                .payload
                .get("llm_message")
                .and_then(|value| value.get("content"))
                .and_then(Value::as_array)
                .and_then(|content| content.first())
                .and_then(|entry| entry.get("text"))
                .and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn event_envelope_round_trips_nested_payload_state_updates() {
        let event = EventEnvelope::state_update("evt-state", "finished");

        let encoded = serde_json::to_value(&event).expect("event should encode");
        assert_eq!(
            encoded,
            json!({
                "id": "evt-state",
                "timestamp": event.timestamp.to_rfc3339(),
                "source": "runtime",
                "kind": "ConversationStateUpdateEvent",
                "execution_status": "finished",
                "state_delta": {
                    "execution_status": "finished",
                },
            })
        );

        let decoded: EventEnvelope =
            serde_json::from_value(encoded).expect("state update should decode");
        assert_eq!(
            decoded.payload,
            json!({
                "execution_status": "finished",
                "state_delta": {
                    "execution_status": "finished",
                },
            })
        );
    }
}
