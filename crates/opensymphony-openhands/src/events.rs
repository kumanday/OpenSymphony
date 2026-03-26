use std::{cmp::Ordering, collections::HashSet};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::models::{Conversation, ConversationStateUpdatePayload, EventEnvelope};

#[derive(Debug, Clone, PartialEq)]
pub struct LlmCompletionLogEvent {
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversationErrorEvent {
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UnknownEvent {
    pub kind: String,
    pub payload: Value,
    pub key: Option<String>,
    pub value: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TextContent {
    #[serde(rename = "type")]
    pub content_type: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageEventPayload {
    pub role: String,
    pub content: Vec<TextContent>,
    pub text_preview: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActionEventPayload {
    pub action_id: String,
    pub tool_name: Option<String>,
    pub message: Option<String>,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservationEventPayload {
    pub observation_id: String,
    pub tool_name: Option<String>,
    pub content: Vec<TextContent>,
    pub text_preview: Option<String>,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum KnownEvent {
    ConversationStateUpdate(ConversationStateUpdatePayload),
    LlmCompletionLog(LlmCompletionLogEvent),
    ConversationError(ConversationErrorEvent),
    Message(MessageEventPayload),
    Action(ActionEventPayload),
    Observation(ObservationEventPayload),
    Unknown(UnknownEvent),
}

impl KnownEvent {
    pub fn from_envelope(event: &EventEnvelope) -> Self {
        match event.kind.as_str() {
            "ConversationStateUpdateEvent" => decode_state_update(event)
                .map(KnownEvent::ConversationStateUpdate)
                .unwrap_or_else(|| KnownEvent::Unknown(unknown_event(event))),
            "LLMCompletionLogEvent" => KnownEvent::LlmCompletionLog(LlmCompletionLogEvent {
                payload: event.payload.clone(),
            }),
            "ConversationErrorEvent" => KnownEvent::ConversationError(ConversationErrorEvent {
                payload: event.payload.clone(),
            }),
            "MessageEvent" => decode_message_event(event)
                .map(KnownEvent::Message)
                .unwrap_or_else(|| KnownEvent::Unknown(unknown_event(event))),
            "ActionEvent" => decode_action_event(event)
                .map(KnownEvent::Action)
                .unwrap_or_else(|| KnownEvent::Unknown(unknown_event(event))),
            "ObservationEvent" => decode_observation_event(event)
                .map(KnownEvent::Observation)
                .unwrap_or_else(|| KnownEvent::Unknown(unknown_event(event))),
            _ => KnownEvent::Unknown(unknown_event(event)),
        }
    }

    pub fn activity_summary(&self) -> Option<ActivitySummary> {
        match self {
            KnownEvent::Message(msg) => Some(ActivitySummary {
                kind: ActivityKind::Message,
                preview: msg.text_preview.clone().unwrap_or_else(|| {
                    msg.content
                        .iter()
                        .filter_map(|c| c.text.as_deref())
                        .next()
                        .unwrap_or("message")
                        .chars()
                        .take(60)
                        .collect()
                }),
                tool_name: None,
            }),
            KnownEvent::Action(action) => Some(ActivitySummary {
                kind: ActivityKind::ToolCall,
                preview: action
                    .message
                    .clone()
                    .unwrap_or_else(|| "action".to_string()),
                tool_name: action.tool_name.clone(),
            }),
            KnownEvent::Observation(obs) => Some(ActivitySummary {
                kind: ActivityKind::ToolResult,
                preview: obs.text_preview.clone().unwrap_or_else(|| {
                    obs.content
                        .iter()
                        .filter_map(|c| c.text.as_deref())
                        .next()
                        .unwrap_or("result")
                        .chars()
                        .take(60)
                        .collect()
                }),
                tool_name: obs.tool_name.clone(),
            }),
            KnownEvent::ConversationStateUpdate(payload) => {
                payload
                    .execution_status
                    .as_ref()
                    .map(|status| ActivitySummary {
                        kind: ActivityKind::StateChange,
                        preview: format!("status: {}", status),
                        tool_name: None,
                    })
            }
            KnownEvent::ConversationError(err) => err
                .payload
                .get("message")
                .and_then(Value::as_str)
                .map(|msg| ActivitySummary {
                    kind: ActivityKind::Error,
                    preview: msg.to_string(),
                    tool_name: None,
                }),
            KnownEvent::LlmCompletionLog(_) | KnownEvent::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivityKind {
    StateChange,
    Message,
    ToolCall,
    ToolResult,
    Error,
}

impl ActivityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActivityKind::StateChange => "state",
            ActivityKind::Message => "message",
            ActivityKind::ToolCall => "tool",
            ActivityKind::ToolResult => "result",
            ActivityKind::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ActivitySummary {
    pub kind: ActivityKind,
    pub preview: String,
    pub tool_name: Option<String>,
}

fn decode_message_event(event: &EventEnvelope) -> Option<MessageEventPayload> {
    let role = event
        .payload
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();

    let content: Vec<TextContent> = event
        .payload
        .get("llm_message")
        .and_then(|msg| msg.get("content"))
        .or_else(|| event.payload.get("content"))
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_default();

    let text_preview: Option<String> = content
        .iter()
        .filter_map(|c| c.text.as_deref())
        .next()
        .map(|t: &str| t.chars().take(80).collect());

    Some(MessageEventPayload {
        role,
        content,
        text_preview,
    })
}

fn decode_action_event(event: &EventEnvelope) -> Option<ActionEventPayload> {
    let action = event.payload.get("action")?;

    let action_id = event.id.clone();
    let tool_name = action
        .get("tool_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let message = action
        .get("message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let arguments = action.clone();

    Some(ActionEventPayload {
        action_id,
        tool_name,
        message,
        arguments,
    })
}

fn decode_observation_event(event: &EventEnvelope) -> Option<ObservationEventPayload> {
    let observation = event.payload.get("observation")?;

    let observation_id = event.id.clone();
    let tool_name = observation
        .get("tool_name")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let exit_code = observation
        .get("exit_code")
        .and_then(Value::as_i64)
        .map(|c| c as i32);

    let content: Vec<TextContent> = observation
        .get("content")
        .and_then(|c| serde_json::from_value(c.clone()).ok())
        .unwrap_or_default();

    let text_preview: Option<String> = content
        .iter()
        .filter_map(|c| c.text.as_deref())
        .next()
        .map(|t: &str| t.chars().take(80).collect());

    Some(ObservationEventPayload {
        observation_id,
        tool_name,
        content,
        text_preview,
        exit_code,
    })
}

fn unknown_event(event: &EventEnvelope) -> UnknownEvent {
    UnknownEvent {
        kind: event.kind.clone(),
        payload: event.payload.clone(),
        key: event.key.clone(),
        value: event.value.clone(),
    }
}

fn decode_state_update(event: &EventEnvelope) -> Option<ConversationStateUpdatePayload> {
    if !event.payload.is_null() && event.payload != Value::Object(Default::default()) {
        if let Ok(payload) = serde_json::from_value(event.payload.clone()) {
            return Some(payload);
        }

        if let Some(payload) = decode_forward_compatible_state_update(&event.payload) {
            return Some(payload);
        }
    }

    let key = event.key.as_deref()?;
    let value = event.value.clone()?;
    match key {
        "full_state" => Some(ConversationStateUpdatePayload {
            execution_status: value
                .get("execution_status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            state_delta: value,
        }),
        "execution_status" => Some(ConversationStateUpdatePayload {
            execution_status: value.as_str().map(ToOwned::to_owned),
            state_delta: serde_json::json!({
                "execution_status": value,
            }),
        }),
        other => Some(ConversationStateUpdatePayload {
            execution_status: None,
            state_delta: serde_json::json!({
                other: value,
            }),
        }),
    }
}

fn decode_forward_compatible_state_update(
    payload: &Value,
) -> Option<ConversationStateUpdatePayload> {
    let state_delta = payload.get("state_delta").cloned().or_else(|| {
        payload
            .get("execution_status")
            .and_then(Value::as_str)
            .map(|status| {
                json!({
                    "execution_status": status,
                })
            })
    })?;

    let execution_status = payload
        .get("execution_status")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            state_delta
                .get("execution_status")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        });

    Some(ConversationStateUpdatePayload {
        execution_status,
        state_delta,
    })
}

#[derive(Debug, Clone, Default)]
pub struct EventCache {
    events: Vec<EventEnvelope>,
    ids: HashSet<String>,
}

impl EventCache {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
            ids: HashSet::new(),
        }
    }

    pub fn insert(&mut self, event: EventEnvelope) -> bool {
        if !self.ids.insert(event.id.clone()) {
            return false;
        }

        let position = self
            .events
            .binary_search_by(|existing| compare_events(existing, &event))
            .unwrap_or_else(|index| index);
        self.events.insert(position, event);
        true
    }

    pub fn merge_new_events<I>(&mut self, events: I) -> Vec<EventEnvelope>
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        let mut inserted = events
            .into_iter()
            .filter(|event| self.insert(event.clone()))
            .collect::<Vec<_>>();
        inserted.sort_by(compare_events);
        inserted
    }

    pub fn extend<I>(&mut self, events: I) -> usize
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        self.merge_new_events(events).len()
    }

    pub fn items(&self) -> &[EventEnvelope] {
        &self.events
    }
}

fn compare_events(left: &EventEnvelope, right: &EventEnvelope) -> Ordering {
    left.timestamp
        .cmp(&right.timestamp)
        .then_with(|| left.id.cmp(&right.id))
}

#[derive(Debug, Clone, Default)]
pub struct ConversationStateMirror {
    execution_status: Option<String>,
    raw_state: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalExecutionStatus {
    Finished,
    Error,
    Stuck,
}

impl ConversationStateMirror {
    pub fn execution_status(&self) -> Option<&str> {
        self.execution_status.as_deref()
    }

    pub fn raw_state(&self) -> &Value {
        &self.raw_state
    }

    pub fn apply_conversation(&mut self, conversation: &Conversation) {
        self.raw_state = Value::Object(Default::default());
        self.apply_conversation_execution_status(conversation);
    }

    pub fn apply_conversation_execution_status(&mut self, conversation: &Conversation) {
        let status = conversation.execution_status.clone();
        self.execution_status = Some(status.clone());
        match &mut self.raw_state {
            Value::Object(state) => {
                state.insert("execution_status".to_string(), Value::String(status));
            }
            raw_state => {
                *raw_state = serde_json::json!({
                    "execution_status": status,
                });
            }
        }
    }

    pub fn rebuild_from(&mut self, conversation: &Conversation, events: &[EventEnvelope]) {
        self.apply_conversation(conversation);
        for event in events {
            self.apply_event(event);
        }
    }

    pub fn apply_event(&mut self, event: &EventEnvelope) {
        if let KnownEvent::ConversationStateUpdate(payload) = KnownEvent::from_envelope(event) {
            if let Some(status) = payload.execution_status {
                self.execution_status = Some(status);
            }
            merge_json(&mut self.raw_state, payload.state_delta);
        }
    }

    pub fn terminal_status(&self) -> Option<TerminalExecutionStatus> {
        match self.execution_status() {
            Some("finished") => Some(TerminalExecutionStatus::Finished),
            Some("error") => Some(TerminalExecutionStatus::Error),
            Some("stuck") => Some(TerminalExecutionStatus::Stuck),
            _ => None,
        }
    }
}

fn merge_json(target: &mut Value, delta: Value) {
    match (target, delta) {
        (Value::Object(target_map), Value::Object(delta_map)) => {
            for (key, value) in delta_map {
                merge_json(target_map.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, value) => {
            *target = value;
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration as ChronoDuration, Utc};
    use serde_json::json;

    use super::{
        ConversationStateMirror, EventCache, KnownEvent, TerminalExecutionStatus, UnknownEvent,
    };
    use crate::models::{
        AgentConfig, ConfirmationPolicy, Conversation, ConversationStateUpdatePayload,
        EventEnvelope, LlmConfig, WorkspaceConfig,
    };

    #[test]
    fn known_event_decoding_preserves_known_and_unknown_payloads() {
        let state_update = EventEnvelope::new(
            "evt-state",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "running",
                "state_delta": {
                    "execution_status": "running",
                },
            }),
        );
        let llm_log = EventEnvelope::new(
            "evt-llm",
            Utc::now(),
            "llm",
            "LLMCompletionLogEvent",
            json!({
                "model": "fake-model",
            }),
        );
        let error_event = EventEnvelope::new(
            "evt-error",
            Utc::now(),
            "runtime",
            "ConversationErrorEvent",
            json!({
                "message": "boom",
            }),
        );
        let unknown_event = EventEnvelope::new(
            "evt-unknown",
            Utc::now(),
            "runtime",
            "ForwardCompatibleEvent",
            json!({
                "opaque": true,
            }),
        );

        assert_eq!(
            KnownEvent::from_envelope(&state_update),
            KnownEvent::ConversationStateUpdate(ConversationStateUpdatePayload {
                execution_status: Some("running".to_string()),
                state_delta: json!({
                    "execution_status": "running",
                }),
            })
        );
        assert!(matches!(
            KnownEvent::from_envelope(&llm_log),
            KnownEvent::LlmCompletionLog(_)
        ));
        assert!(matches!(
            KnownEvent::from_envelope(&error_event),
            KnownEvent::ConversationError(_)
        ));
        assert_eq!(
            KnownEvent::from_envelope(&unknown_event),
            KnownEvent::Unknown(UnknownEvent {
                kind: "ForwardCompatibleEvent".to_string(),
                payload: json!({
                    "opaque": true,
                }),
                key: None,
                value: None,
            })
        );
    }

    #[test]
    fn event_cache_orders_and_deduplicates_new_events() {
        let mut cache = EventCache::new();
        let newer = EventEnvelope::new(
            "evt-2",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({}),
        );
        let older = EventEnvelope::new(
            "evt-1",
            Utc::now() - ChronoDuration::seconds(10),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({}),
        );

        let inserted = cache.merge_new_events(vec![newer.clone(), older.clone(), older.clone()]);

        assert_eq!(inserted, vec![older.clone(), newer.clone()]);
        assert_eq!(cache.items()[0].id, older.id);
        assert_eq!(cache.items()[1].id, newer.id);
    }

    #[test]
    fn state_mirror_rebuild_keeps_latest_terminal_status_after_out_of_order_events() {
        let conversation = Conversation {
            conversation_id: uuid::Uuid::nil(),
            workspace: WorkspaceConfig {
                working_dir: "/tmp/workspace".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/workspace/.opensymphony/openhands".to_string(),
            max_iterations: 4,
            stuck_detection: true,
            execution_status: "idle".to_string(),
            confirmation_policy: ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: AgentConfig {
                kind: "Agent".to_string(),
                llm: LlmConfig {
                    model: "openai/gpt-5.4".to_string(),
                    api_key: None,
                    base_url: None,
                    usage_id: None,
                },
                condenser: None,
                tools: None,
                include_default_tools: None,
            },
        };
        let running = EventEnvelope::new(
            "evt-running",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "running",
                "state_delta": {
                    "execution_status": "running",
                },
            }),
        );
        let stale = EventEnvelope::new(
            "evt-queued",
            running.timestamp - ChronoDuration::seconds(5),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "queued",
                "state_delta": {
                    "execution_status": "queued",
                },
            }),
        );
        let finished = EventEnvelope::new(
            "evt-finished",
            running.timestamp + ChronoDuration::seconds(5),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "finished",
                "state_delta": {
                    "execution_status": "finished",
                },
            }),
        );

        let mut cache = EventCache::new();
        cache.merge_new_events(vec![running, stale, finished]);

        let mut mirror = ConversationStateMirror::default();
        mirror.rebuild_from(&conversation, cache.items());

        assert_eq!(mirror.execution_status(), Some("finished"));
        assert_eq!(
            mirror.terminal_status(),
            Some(TerminalExecutionStatus::Finished)
        );
    }
}
