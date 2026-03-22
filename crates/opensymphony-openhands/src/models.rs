use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentConfig {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmConfig>,
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
}

impl ConversationCreateRequest {
    pub fn doctor_probe(
        working_dir: impl Into<String>,
        persistence_dir: impl Into<String>,
        model: Option<String>,
        api_key: Option<String>,
    ) -> Self {
        Self {
            conversation_id: Uuid::new_v4(),
            workspace: WorkspaceConfig {
                working_dir: working_dir.into(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: persistence_dir.into(),
            max_iterations: 4,
            stuck_detection: true,
            confirmation_policy: ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: AgentConfig {
                kind: "Agent".to_string(),
                llm: model.map(|model| LlmConfig { model, api_key }),
            },
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConversationStateUpdatePayload {
    #[serde(default)]
    pub execution_status: Option<String>,
    #[serde(default)]
    pub state_delta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventEnvelope {
    pub id: String,
    #[serde(with = "flexible_timestamp")]
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub source: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
}

mod flexible_timestamp {
    use chrono::{DateTime, NaiveDateTime, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &DateTime<Utc>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_rfc3339())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if let Ok(timestamp) = DateTime::parse_from_rfc3339(&value) {
            return Ok(timestamp.with_timezone(&Utc));
        }

        let naive = NaiveDateTime::parse_from_str(&value, "%Y-%m-%dT%H:%M:%S%.f")
            .map_err(serde::de::Error::custom)?;
        Ok(DateTime::from_naive_utc_and_offset(naive, Utc))
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchConversationEventsResponse {
    #[serde(alias = "items")]
    pub events: Vec<EventEnvelope>,
    #[serde(default)]
    pub next_page_id: Option<String>,
}
