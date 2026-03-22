use std::cmp::Ordering;

use serde_json::Value;

use crate::models::{Conversation, ConversationStateUpdatePayload, EventEnvelope};

#[derive(Debug, Clone, PartialEq)]
pub enum KnownEvent {
    ConversationStateUpdate(ConversationStateUpdatePayload),
    LlmCompletionLog,
    Unknown,
}

impl KnownEvent {
    pub fn from_envelope(event: &EventEnvelope) -> Self {
        match event.kind.as_str() {
            "ConversationStateUpdateEvent" => decode_state_update(event)
                .map(KnownEvent::ConversationStateUpdate)
                .unwrap_or(KnownEvent::Unknown),
            "LLMCompletionLogEvent" => KnownEvent::LlmCompletionLog,
            _ => KnownEvent::Unknown,
        }
    }
}

fn decode_state_update(event: &EventEnvelope) -> Option<ConversationStateUpdatePayload> {
    if !event.payload.is_null() && event.payload != Value::Object(Default::default()) {
        return serde_json::from_value(event.payload.clone()).ok();
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

#[derive(Debug, Clone, Default)]
pub struct EventCache {
    events: Vec<EventEnvelope>,
}

impl EventCache {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    pub fn insert(&mut self, event: EventEnvelope) -> bool {
        if self.events.iter().any(|existing| existing.id == event.id) {
            return false;
        }

        let position = self
            .events
            .binary_search_by(|existing| compare_events(existing, &event))
            .unwrap_or_else(|index| index);
        self.events.insert(position, event);
        true
    }

    pub fn extend<I>(&mut self, events: I) -> usize
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        events
            .into_iter()
            .filter(|event| self.insert(event.clone()))
            .count()
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

impl ConversationStateMirror {
    pub fn execution_status(&self) -> Option<&str> {
        self.execution_status.as_deref()
    }

    pub fn raw_state(&self) -> &Value {
        &self.raw_state
    }

    pub fn apply_conversation(&mut self, conversation: &Conversation) {
        self.execution_status = Some(conversation.execution_status.clone());
        self.raw_state = serde_json::json!({
            "execution_status": conversation.execution_status,
        });
    }

    pub fn apply_event(&mut self, event: &EventEnvelope) {
        if let KnownEvent::ConversationStateUpdate(payload) = KnownEvent::from_envelope(event) {
            if let Some(status) = payload.execution_status {
                self.execution_status = Some(status);
            }
            merge_json(&mut self.raw_state, payload.state_delta);
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
