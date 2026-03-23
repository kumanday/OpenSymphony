use std::{cmp::Ordering, collections::HashSet};

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

#[derive(Debug, Clone, PartialEq)]
pub enum KnownEvent {
    ConversationStateUpdate(ConversationStateUpdatePayload),
    LlmCompletionLog(LlmCompletionLogEvent),
    ConversationError(ConversationErrorEvent),
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
            _ => KnownEvent::Unknown(unknown_event(event)),
        }
    }
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
                },
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
