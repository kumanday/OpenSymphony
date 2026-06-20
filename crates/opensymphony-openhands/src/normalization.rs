//! OpenHands → OpenSymphony event normalization.
//!
//! The [`normalize_event`] entry point converts a raw [`EventEnvelope`] produced
//! by the OpenHands agent-server into a typed `NormalizedEvent` that downstream
//! consumers (event journal, gateway, TUI, dashboards) can reason about without
//! depending on OpenHands wire protocol details.
//!
//! Invariants:
//!
//! - High-value OpenHands events always decode into typed payloads and the
//!   mapped [`EventKind`].
//! - Unknown event kinds are still preserved through [`NormalizedEvent::raw_payload`]
//!   and a synthetic `raw_payload_ref`, so diagnostics and forward compatibility
//!   continue to work even when OpenHands adds a new event type.
//! - The normalization is total: every input produces an output and never panics
//!   or fails the surrounding run.
//! - Rest history + WebSocket event reconciliation produce the same output as a
//!   stream that received the events in one go (event IDs are passed through
//!   and ordering is preserved through the caller's `EventCache`).

use crate::opensymphony_domain::ConversationId;
use crate::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};
use crate::opensymphony_gateway_schema::event_journal::{EventId, EventKind, EventRecord};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use super::{
    events::{
        ActionEventPayload, KnownEvent, LlmCompletionLogEvent, MessageEventPayload,
        ObservationEventPayload, UnknownEvent,
    },
    models::EventEnvelope,
};

/// Marker string used as the prefix for synthetic `raw_payload_ref` values on
/// events that originate from outside the runtime cache or that the harness
/// could not normalize into a typed payload.
pub const UNKNOWN_RAW_REF_PREFIX: &str = "raw://openhands/unknown";

/// Errors that can occur when normalizing an OpenHands event envelope into an
/// OpenSymphony envelope.
///
/// Most failures are recoverable and the runner continues with a best-effort
/// representation of the event; this enum captures the rare paths that would
/// represent a programming bug rather than a real runtime failure.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum NormalizationError {
    /// The harness id was empty, which would result in an unsafe entity ref.
    #[error("normalization context requires a non-empty harness id")]
    MissingHarnessId,
    /// The conversation id was empty, which would result in an unsafe cursor.
    #[error("normalization context requires a non-empty conversation id")]
    MissingConversationId,
}

/// Input to [`normalize_event`] — provides the harness identity, conversation
/// identity, and optional correlation metadata that every normalized envelope
/// carries along.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizationContext {
    /// Stable harness identifier (e.g., `openhands-agent-server-v1`).
    pub harness_id: String,
    /// Stable conversation identifier — the entity the normalized event belongs to.
    pub conversation_id: ConversationId,
    /// Optional entity reference for the owning issue (if known to the runner).
    pub issue_entity: Option<EntityRef>,
    /// Optional correlation ID forwarded into the normalized envelope.
    pub correlation_id: Option<EventId>,
    /// Schema version to record on the envelope (defaults to v1).
    pub schema_version: String,
}

impl NormalizationContext {
    /// Build a context with the mandatory harness id and conversation id,
    /// defaulting the schema version and leaving optional fields unset.
    pub fn new(
        harness_id: impl Into<String>,
        conversation_id: impl Into<ConversationId>,
    ) -> Result<Self, NormalizationError> {
        let harness_id = harness_id.into();
        if harness_id.is_empty() {
            return Err(NormalizationError::MissingHarnessId);
        }
        let conversation_id = conversation_id.into();
        if conversation_id.as_str().is_empty() {
            return Err(NormalizationError::MissingConversationId);
        }
        Ok(Self {
            harness_id,
            conversation_id,
            issue_entity: None,
            correlation_id: None,
            schema_version: "v1".to_string(),
        })
    }

    /// Attach the owning issue entity ref so the envelope self-describes the
    /// project/issue lineage.
    pub fn with_issue_entity(mut self, entity: EntityRef) -> Self {
        self.issue_entity = Some(entity);
        self
    }

    /// Attach a correlation id so events can be linked to a triggering action.
    pub fn with_correlation_id(mut self, id: impl Into<EventId>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    /// Override the schema version (defaults to `v1`).
    pub fn with_schema_version(mut self, version: impl Into<String>) -> Self {
        self.schema_version = version.into();
        self
    }

    fn entity_ref(&self) -> EntityRef {
        EntityRef {
            kind: EntityKind::Conversation,
            id: self.conversation_id.to_string(),
            identifier: None,
        }
    }
}

/// Result of [`normalize_event`]: a typed [`EventRecordBuilder`](EventRecord)
/// plus the raw payload so downstream consumers can still forward unknown
/// event kinds without losing fidelity.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedEvent {
    /// Fully-built journal record.
    pub record: EventRecord,
    /// Original OpenHands payload (for diagnostics + forward compatibility).
    /// Always populated — the mirror never produces `None` because the raw
    /// payload is preserved for diagnostic consumers even when the encoded
    /// [`EventKind`] is fully typed.
    pub raw_payload: Value,
    /// Synthesized raw payload reference when normalization produced an
    /// `EventKind::Unknown` envelope.
    pub raw_payload_ref: Option<String>,
}

/// Convert an OpenHands event envelope into a normalized OpenSymphony envelope.
///
/// `normalize_event` is **total**: every (well-formed or malformed) envelope
/// produces a [`NormalizedEvent`] without panicking or returning `Err`. The
/// caller-side input validation lives on [`NormalizationContext::new`], which
/// is the only point at which a [`NormalizationError`] can surface — at that
/// step the caller has already opted into a recoverable error path (empty
/// harness id or conversation id) and can supply a fixed context before
/// re-invoking the normalizer.
///
/// Envelopes whose `source` is empty (no upstream actor signal at all) are
/// routed through the `Unknown` envelope path with a synthetic `harness`
/// actor and a `source_missing` summary so the journal still sees the raw
/// payload instead of failing the run.
pub fn normalize_event(
    envelope: &EventEnvelope,
    context: &NormalizationContext,
) -> NormalizedEvent {
    if envelope.source.is_empty() {
        // Total fallback: route to the Unknown envelope machinery directly so
        // the run keeps going and the raw payload is still preserved through
        // the journal. This is what the docstring promises.
        let synthetic = UnknownEvent {
            kind: envelope.kind.clone(),
            payload: envelope.payload.clone(),
            key: envelope.key.clone(),
            value: envelope.value.clone(),
        };
        let mut record = normalize_unknown(envelope, &synthetic);
        // Override the summary so consumers can tell that this came in with
        // no actor signal at all (as opposed to a genuine unknown kind).
        record.summary = format!("source_missing envelope kind={}", envelope.kind);
        return build_normalized_event(envelope, context, record);
    }

    let known = KnownEvent::from_envelope(envelope);
    let result = match known {
        KnownEvent::ConversationStateUpdate(_) => normalize_state_update(envelope),
        KnownEvent::Message(payload) => normalize_message(envelope, &payload),
        KnownEvent::Action(payload) => normalize_action(envelope, &payload),
        KnownEvent::Observation(payload) => normalize_observation(envelope, &payload),
        KnownEvent::LlmCompletionLog(payload) => normalize_llm_completion(envelope, &payload),
        KnownEvent::ConversationError(_) => normalize_conversation_error(envelope),
        KnownEvent::Unknown(unknown) => normalize_unknown(envelope, &unknown),
    };

    build_normalized_event(envelope, context, result)
}

/// Encode a typed OpenHands state-update event into OpenSymphony
/// `HarnessConversationStateUpdate`.
pub fn normalize_state_update(envelope: &EventEnvelope) -> NormalizedRecord {
    NormalizedRecord {
        kind: EventKind::HarnessConversationStateUpdate,
        payload: envelope.payload.clone(),
        summary: summarize_state_update(&envelope.payload),
        raw_payload_ref: None,
    }
}

/// Encode an OpenHands message into `HarnessEventNormalized`.
pub fn normalize_message(
    envelope: &EventEnvelope,
    payload: &MessageEventPayload,
) -> NormalizedRecord {
    // Suppress unused parameter when the payload preview already exposes
    // the relevant fields; envelope is used to surface the llm_message fallback.
    let _ = envelope;
    NormalizedRecord {
        kind: EventKind::HarnessEventNormalized {
            source_kind: envelope.kind.clone(),
        },
        payload: json!({
            "role": payload.role,
            "preview": payload.text_preview.clone(),
            "content": envelope.payload.get("llm_message")
                .and_then(|msg| msg.get("content"))
                .or_else(|| envelope.payload.get("content"))
                .cloned()
                .unwrap_or(Value::Null),
        }),
        summary: payload
            .text_preview
            .clone()
            .unwrap_or_else(|| format!("[{}]", payload.role)),
        raw_payload_ref: None,
    }
}

/// Encode an OpenHands action into `HarnessToolCall`.
pub fn normalize_action(
    envelope: &EventEnvelope,
    payload: &ActionEventPayload,
) -> NormalizedRecord {
    let _ = envelope;
    let mut body = json!({
        "action_id": payload.action_id,
        "tool_name": payload.tool_name,
        "message": payload.message,
    });
    if let (Value::Object(map), Value::Object(arguments)) = (&mut body, &payload.arguments) {
        for (key, value) in arguments {
            map.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    NormalizedRecord {
        kind: EventKind::HarnessToolCall,
        payload: body,
        summary: payload
            .message
            .clone()
            .or_else(|| payload.tool_name.clone())
            .unwrap_or_else(|| "tool call".to_string()),
        raw_payload_ref: None,
    }
}

/// Encode an OpenHands observation into `HarnessToolResult`.
pub fn normalize_observation(
    envelope: &EventEnvelope,
    payload: &ObservationEventPayload,
) -> NormalizedRecord {
    let content = envelope
        .payload
        .get("observation")
        .and_then(|obs| obs.get("content"))
        .cloned()
        .unwrap_or(Value::Null);
    NormalizedRecord {
        kind: EventKind::HarnessToolResult,
        payload: json!({
            "observation_id": payload.observation_id,
            "tool_name": payload.tool_name,
            "exit_code": payload.exit_code,
            "preview": payload.text_preview,
            "content": content,
        }),
        summary: payload
            .text_preview
            .clone()
            .or_else(|| payload.tool_name.clone())
            .unwrap_or_else(|| "tool result".to_string()),
        raw_payload_ref: None,
    }
}

/// Encode an OpenHands LLM completion log into `HarnessEventNormalized`.
pub fn normalize_llm_completion(
    envelope: &EventEnvelope,
    payload: &LlmCompletionLogEvent,
) -> NormalizedRecord {
    let usage = payload
        .token_usage()
        .map(|(input, output)| json!({ "prompt": input, "completion": output }))
        .unwrap_or(Value::Null);
    let usage_id = payload.model().or_else(|| {
        envelope
            .payload
            .get("model")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    });
    NormalizedRecord {
        kind: EventKind::HarnessEventNormalized {
            source_kind: envelope.kind.clone(),
        },
        payload: json!({
            "model": usage_id,
            "usage": usage,
        }),
        summary: usage_id
            .clone()
            .map(|model| format!("llm completion ({model})"))
            .unwrap_or_else(|| "llm completion".to_string()),
        raw_payload_ref: None,
    }
}

/// Encode an OpenHands conversation error into `HarnessEventNormalized` so the
/// error reaches the journal without breaking the run.
pub fn normalize_conversation_error(envelope: &EventEnvelope) -> NormalizedRecord {
    let message = envelope
        .payload
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| envelope.payload.get("detail").and_then(Value::as_str))
        .unwrap_or("conversation error");
    NormalizedRecord {
        kind: EventKind::HarnessEventNormalized {
            source_kind: envelope.kind.clone(),
        },
        payload: envelope.payload.clone(),
        summary: format!("conversation error: {message}"),
        raw_payload_ref: None,
    }
}

/// Encode an OpenHands event whose kind we cannot yet decode as
/// `EventKind::Unknown` while preserving the full raw payload.
pub fn normalize_unknown(envelope: &EventEnvelope, _unknown: &UnknownEvent) -> NormalizedRecord {
    let raw_payload = unknown_payload(envelope);
    let raw_payload_ref = synthetic_raw_ref(envelope);
    NormalizedRecord {
        kind: EventKind::Unknown {
            raw_kind: envelope.kind.clone(),
        },
        payload: envelope.key.clone().map_or(
            Value::Null,
            |key| json!({ "key": key, "value": envelope.value.clone().unwrap_or(Value::Null) }),
        ),
        summary: format!("unknown openhands event: {}", envelope.kind),
        raw_payload_ref: Some(raw_payload_ref.clone()),
    }
    .with_raw(raw_payload)
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedRecord {
    pub kind: EventKind,
    pub payload: Value,
    pub summary: String,
    pub raw_payload_ref: Option<String>,
}

impl NormalizedRecord {
    fn with_raw(self, raw_payload: Value) -> Self {
        let raw_payload_ref = self
            .raw_payload_ref
            .clone()
            .unwrap_or_else(|| synthetic_raw_ref_for_raw(&raw_payload));
        Self {
            kind: self.kind,
            payload: self.payload,
            summary: self.summary,
            raw_payload_ref: Some(raw_payload_ref),
        }
    }
}

fn unknown_payload(envelope: &EventEnvelope) -> Value {
    if !envelope.payload.is_null() {
        return envelope.payload.clone();
    }
    if let (Some(key), Some(value)) = (envelope.key.clone(), envelope.value.clone()) {
        return json!({ "key": key, "value": value });
    }
    Value::Null
}

fn synthetic_raw_ref(envelope: &EventEnvelope) -> String {
    format!(
        "{UNKNOWN_RAW_REF_PREFIX}/{}/{}/{}",
        envelope.source, envelope.kind, envelope.id
    )
}

fn synthetic_raw_ref_for_raw(_value: &Value) -> String {
    // Hash-derived synthetic ref keeps forwards-compatibility diagnostics discoverable
    // for arbitrary payloads without dumping the full content into the marker.
    format!("{UNKNOWN_RAW_REF_PREFIX}/synthetic/{}", Uuid::new_v4())
}

fn build_normalized_event(
    envelope: &EventEnvelope,
    context: &NormalizationContext,
    record: NormalizedRecord,
) -> NormalizedEvent {
    let actor = derive_actor(envelope, context);
    let entity_ref = context.entity_ref();
    let record_id = ensure_event_id(envelope, &record);

    let mut entity_refs = vec![entity_ref];
    if let Some(issue) = context.issue_entity.clone() {
        entity_refs.push(issue);
    }

    let raw_payload_ref = record.raw_payload_ref.clone();

    let mut builder = EventRecord::builder()
        .event_id(record_id)
        .actor(actor)
        .kind(record.kind.clone())
        .happened_at(envelope.timestamp)
        .summary(record.summary.clone())
        .entity_refs(entity_refs);
    if let Some(correlation) = context.correlation_id.clone() {
        builder = builder.correlation_id(correlation);
    }
    if let Some(raw_ref) = raw_payload_ref.clone() {
        builder = builder.raw_payload_ref(raw_ref);
    }
    builder = builder.payload(record.payload.clone());

    NormalizedEvent {
        record: builder.build(),
        raw_payload: raw_payload_if_unknown(envelope, &record),
        raw_payload_ref,
    }
}

fn raw_payload_if_unknown(envelope: &EventEnvelope, record: &NormalizedRecord) -> Value {
    if matches!(record.kind, EventKind::Unknown { .. }) || record.raw_payload_ref.is_some() {
        unknown_payload(envelope)
    } else {
        envelope.payload.clone()
    }
}

fn derive_actor(
    envelope: &EventEnvelope,
    context: &NormalizationContext,
) -> crate::opensymphony_gateway_schema::event_journal::EventActor {
    use crate::opensymphony_gateway_schema::event_journal::EventActor;
    match envelope.source.as_str() {
        "user" => EventActor::user(envelope.source.clone()),
        "agent" | "assistant" => EventActor::agent(envelope.source.clone()),
        "llm" => EventActor::agent(envelope.source.clone()),
        "runtime" | "system" => EventActor::system(envelope.source.clone()),
        _ => EventActor::harness(context.harness_id.clone()),
    }
}

fn ensure_event_id(envelope: &EventEnvelope, _record: &NormalizedRecord) -> EventId {
    if envelope.id.is_empty() {
        Uuid::new_v4().to_string()
    } else {
        envelope.id.clone()
    }
}

fn summarize_state_update(payload: &Value) -> String {
    let status = payload
        .get("execution_status")
        .and_then(Value::as_str)
        .or_else(|| {
            payload
                .get("state_delta")
                .and_then(|d| d.get("execution_status"))
                .and_then(Value::as_str)
        });
    match status {
        Some(status) => format!("runtime status: {status}"),
        None => "runtime state update".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_openhands::Conversation;
    use chrono::{Duration as ChronoDuration, Utc};
    use serde_json::json;

    fn context() -> NormalizationContext {
        NormalizationContext::new(
            "openhands-agent-server-v1",
            ConversationId::new("conv-123").expect("conv id"),
        )
        .expect("ctx")
    }

    fn context_with_correlation() -> (NormalizationContext, EventId) {
        let corr = "corr-100";
        let ctx = NormalizationContext::new(
            "openhands-agent-server-v1",
            ConversationId::new("conv-123").expect("conv id"),
        )
        .expect("ctx")
        .with_correlation_id(corr);
        (ctx, corr.to_string())
    }

    fn conversation() -> Conversation {
        Conversation {
            conversation_id: Uuid::nil(),
            workspace: crate::opensymphony_openhands::WorkspaceConfig {
                working_dir: "/tmp/conv".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/conv/persistence".to_string(),
            max_iterations: 4,
            stuck_detection: true,
            execution_status: "idle".to_string(),
            confirmation_policy: crate::opensymphony_openhands::ConfirmationPolicy {
                kind: "NeverConfirm".to_string(),
            },
            agent: crate::opensymphony_openhands::AgentConfig {
                kind: "Agent".to_string(),
                llm: crate::opensymphony_openhands::LlmConfig {
                    model: "openai/gpt-5.4".to_string(),
                    api_key: None,
                    base_url: None,
                    usage_id: None,
                    extra_headers: None,
                    litellm_extra_body: None,
                    stream: None,
                },
                condenser: None,
                tools: None,
                include_default_tools: None,
            },
            stats: None,
        }
    }

    #[test]
    fn context_rejects_empty_fields() {
        assert!(
            NormalizationContext::new("", ConversationId::new("x").expect("valid id")).is_err()
        );
        let bad_id = "";
        match ConversationId::new(bad_id) {
            Err(_) => {}
            Ok(_) => panic!("invalid id should be rejected"),
        }
    }

    #[test]
    fn state_update_normalizes_with_status_summary() {
        let envelope = EventEnvelope::new(
            "evt-state",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": "running",
                "state_delta": {
                    "execution_status": "running",
                }
            }),
        );
        let ctx = context();
        let normalized = normalize_event(&envelope, &ctx);
        assert!(matches!(
            normalized.record.kind,
            EventKind::HarnessConversationStateUpdate
        ));
        assert_eq!(normalized.record.summary, "runtime status: running");
        assert_eq!(normalized.record.event_id, "evt-state");
        assert!(
            normalized
                .record
                .entity_refs
                .iter()
                .any(|r| matches!(r.kind, EntityKind::Conversation) && r.id == "conv-123")
        );
    }

    #[test]
    fn message_event_normalizes_with_role_and_preview() {
        let envelope = EventEnvelope::new(
            "evt-message",
            Utc::now(),
            "agent",
            "MessageEvent",
            json!({
                "role": "assistant",
                "content": [
                    { "type": "text", "text": "hello world" }
                ]
            }),
        );
        let normalized = normalize_event(&envelope, &context());
        match normalized.record.kind.clone() {
            EventKind::HarnessEventNormalized { source_kind } => {
                assert_eq!(source_kind, "MessageEvent");
            }
            other => panic!("unexpected kind {other:?}"),
        }
        let payload = normalized
            .record
            .payload
            .as_ref()
            .expect("payload present for message events");
        assert!(payload.get("role").is_some());
        assert!(payload.get("preview").is_some());
        assert!(normalized.record.summary.starts_with("hello"));
    }

    #[test]
    fn action_event_normalizes_tool_call_with_arguments() {
        let envelope = EventEnvelope::new(
            "evt-action",
            Utc::now(),
            "runtime",
            "ActionEvent",
            json!({
                "action": {
                    "tool_name": "terminal",
                    "message": "running ls",
                    "command": "ls -la",
                }
            }),
        );
        let normalized = normalize_event(&envelope, &context());
        assert!(matches!(normalized.record.kind, EventKind::HarnessToolCall));
        let payload = normalized
            .record
            .payload
            .as_ref()
            .expect("payload present for tool call events");
        assert_eq!(
            payload.get("tool_name").and_then(Value::as_str),
            Some("terminal")
        );
        // `c0396c1` propagated `action_id` from the envelope id into the normalized
        // payload so the action/observation chain stays correlatable. Locking
        // it here means dropping the field later would surface immediately.
        assert_eq!(
            payload.get("action_id").and_then(Value::as_str),
            Some("evt-action"),
            "action_id must propagate so the action/observation chain stays correlatable"
        );
        assert!(normalized.record.summary.contains("running ls"));
    }

    #[test]
    fn observation_event_normalizes_tool_result_with_exit_code() {
        let envelope = EventEnvelope::new(
            "evt-observation",
            Utc::now(),
            "runtime",
            "ObservationEvent",
            json!({
                "observation": {
                    "tool_name": "terminal",
                    "exit_code": 0,
                    "content": [
                        { "type": "text", "text": "ok" }
                    ]
                }
            }),
        );
        let normalized = normalize_event(&envelope, &context());
        assert!(matches!(
            normalized.record.kind,
            EventKind::HarnessToolResult
        ));
        let payload = normalized
            .record
            .payload
            .as_ref()
            .expect("payload present for tool result events");
        assert_eq!(payload.get("exit_code").and_then(Value::as_i64), Some(0));
    }

    #[test]
    fn llm_completion_log_extracts_token_usage() {
        let envelope = EventEnvelope::new(
            "evt-llm",
            Utc::now(),
            "llm",
            "LLMCompletionLogEvent",
            json!({
                "model": "openai/gpt-5.4",
                "usage": { "prompt_tokens": 100, "completion_tokens": 200 }
            }),
        );
        let normalized = normalize_event(&envelope, &context());
        let payload = normalized
            .record
            .payload
            .as_ref()
            .expect("payload present for llm completion events");
        let usage = payload.get("usage").expect("usage payload");
        assert_eq!(usage.get("prompt").and_then(Value::as_u64), Some(100));
        assert_eq!(usage.get("completion").and_then(Value::as_u64), Some(200));
    }

    #[test]
    fn conversation_error_normalizes_with_summary() {
        let envelope = EventEnvelope::new(
            "evt-error",
            Utc::now(),
            "runtime",
            "ConversationErrorEvent",
            json!({ "message": "OOM in tool" }),
        );
        let normalized = normalize_event(&envelope, &context());
        assert!(normalized.record.summary.contains("OOM in tool"));
        assert!(matches!(
            normalized.record.kind,
            EventKind::HarnessEventNormalized { .. }
        ));
    }

    #[test]
    fn unknown_event_keeps_raw_payload_for_diagnostics() {
        let envelope = EventEnvelope::new(
            "evt-unknown",
            Utc::now(),
            "runtime",
            "FutureOpenHandsEvent",
            json!({ "future": true, "details": "raw" }),
        );
        let normalized = normalize_event(&envelope, &context());
        match normalized.record.kind.clone() {
            EventKind::Unknown { raw_kind } => assert_eq!(raw_kind, "FutureOpenHandsEvent"),
            other => panic!("unexpected kind {other:?}"),
        }
        let raw_ref = normalized
            .record
            .raw_payload_ref
            .as_ref()
            .expect("raw_payload_ref");
        assert!(raw_ref.starts_with(UNKNOWN_RAW_REF_PREFIX));
        assert_eq!(
            normalized.raw_payload,
            json!({ "future": true, "details": "raw" })
        );
    }

    #[test]
    fn forward_compatible_key_value_decodes_into_unknown() {
        let mut envelope = EventEnvelope::new(
            "evt-key-value",
            Utc::now(),
            "runtime",
            "ForwardStateDelta",
            Value::Null,
        );
        envelope.key = Some("unknown_key".to_string());
        envelope.value = Some(json!({ "structured": true }));
        let normalized = normalize_event(&envelope, &context());
        match normalized.record.kind.clone() {
            EventKind::Unknown { raw_kind } => {
                assert_eq!(raw_kind, "ForwardStateDelta");
            }
            other => panic!("unexpected kind {other:?}"),
        }
        assert!(
            normalized
                .record
                .raw_payload_ref
                .as_ref()
                .expect("raw ref")
                .starts_with(UNKNOWN_RAW_REF_PREFIX)
        );
    }

    #[test]
    fn normalization_never_panics_on_empty_payload_or_unknown_kinds() {
        let mut envelope = EventEnvelope::new(
            "evt-edge",
            Utc::now() - ChronoDuration::seconds(1),
            "user",
            "AnythingAtAll",
            Value::Null,
        );
        envelope.key = None;
        envelope.value = None;
        let normalized = normalize_event(&envelope, &context());
        // Must produce a normalized event without raising; unknown at minimum.
        match normalized.record.kind {
            EventKind::Unknown { .. } | EventKind::HarnessEventNormalized { .. } => {}
            other => panic!("unexpected kind {other:?}"),
        }
    }

    #[test]
    fn correlation_id_propagates_into_normalized_envelope() {
        let envelope = EventEnvelope::new(
            "evt-state",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({ "execution_status": "running" }),
        );
        let (ctx, corr) = context_with_correlation();
        let normalized = normalize_event(&envelope, &ctx);
        assert_eq!(normalized.record.correlation_id, Some(corr));
    }

    #[test]
    fn user_source_routes_to_event_user_actor() {
        let envelope = EventEnvelope::new(
            "evt-msg",
            Utc::now(),
            "user",
            "MessageEvent",
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": "hi" }]
            }),
        );
        let normalized = normalize_event(&envelope, &context());
        assert_eq!(normalized.record.actor.kind_label(), "user");
    }

    #[test]
    fn runtime_source_routes_to_system_actor() {
        let envelope = EventEnvelope::new(
            "evt-runtime",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({ "execution_status": "running" }),
        );
        let normalized = normalize_event(&envelope, &context());
        assert_eq!(normalized.record.actor.kind_label(), "system");
        assert_eq!(normalized.record.actor.actor_id(), "runtime");
    }

    #[test]
    fn openhands_source_routes_to_harness_actor() {
        let envelope = EventEnvelope::new(
            "evt-other",
            Utc::now(),
            "some-other-source",
            "PersistedEvent",
            Value::Null,
        );
        let normalized = normalize_event(&envelope, &context());
        assert_eq!(normalized.record.actor.kind_label(), "harness");
        assert_eq!(
            normalized.record.actor.actor_id(),
            "openhands-agent-server-v1"
        );
    }

    #[test]
    fn empty_source_is_total_routes_to_unknown_with_harness_actor() {
        let envelope = EventEnvelope::new(
            "evt-source-missing",
            Utc::now(),
            "",
            "Anything",
            json!({ "lost": true }),
        );
        let normalized = normalize_event(&envelope, &context());
        match normalized.record.kind {
            EventKind::Unknown { raw_kind } => assert_eq!(raw_kind, "Anything"),
            other => panic!("expected Unknown, got {other:?}"),
        }
        assert!(
            normalized
                .record
                .summary
                .contains("source_missing envelope kind=Anything"),
            "summary should explain the missing source: {}",
            normalized.record.summary
        );
        // Harness actor is the default for unrecognized source (including empty).
        assert_eq!(
            normalized.record.actor.actor_id(),
            "openhands-agent-server-v1",
            "harness actor must be used when source cannot identify the upstream actor"
        );
        // Raw payload still preserved for diagnostics / forward compatibility.
        assert_eq!(normalized.raw_payload, json!({ "lost": true }));
    }

    #[test]
    fn raw_payload_for_known_event_is_the_envelope_payload() {
        let envelope = EventEnvelope::new(
            "evt-state",
            Utc::now(),
            "runtime",
            "ConversationStateUpdateEvent",
            json!({ "execution_status": "running" }),
        );
        let normalized = normalize_event(&envelope, &context());
        assert!(matches!(
            normalized.record.kind,
            EventKind::HarnessConversationStateUpdate
        ));
        // Raw payload is always populated (paper trail for diagnostics).
        assert!(!normalized.raw_payload.is_null());
    }

    #[test]
    fn builtin_conversation_is_constructible_for_helper_usage() {
        // Smoke-checks that `Conversation` keeps the same shape used by the
        // mirror module when supplied a runtime conversation.
        let _ = conversation();
    }
}
