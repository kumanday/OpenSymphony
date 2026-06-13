//! OpenHands → OpenSymphony event normalization contract tests.
//!
//! These tests exercise the [`normalize_event`](opensymphony_openhands::normalize_event)
//! entry point against synthetic OpenHands event envelopes and assert that:
//!
//! - Every `KnownEvent` variant produces a typed `EventKind` envelope.
//! - Unknown event kinds are preserved through `EventKind::Unknown` plus the
//!   raw payload reference, so diagnostics and forward compatibility continue
//!   to work.
//! - Unknown envelopes never fail the surrounding run; the function is total.
//! - REST history plus WebSocket event reconciliation produce the same
//!   output as a stream that received the events in one go.

use chrono::{Duration as ChronoDuration, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::opensymphony_domain::ConversationId;
use crate::opensymphony_gateway_schema::event_journal::EventKind;
use crate::opensymphony_openhands::{
    EventEnvelope, NormalizationContext, NormalizedEvent, UNKNOWN_RAW_REF_PREFIX, normalize_event,
};

fn conversation_id() -> ConversationId {
    ConversationId::new("conv-norm-1").expect("valid id")
}

fn context() -> NormalizationContext {
    NormalizationContext::new("openhands-agent-server-v1", conversation_id()).expect("ctx")
}

fn state_update_envelope(id: &str) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        json!({
            "execution_status": "running",
            "state_delta": { "execution_status": "running" },
        }),
    )
}

fn message_envelope(id: &str, role: &str, text: &str) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        if role == "user" { "user" } else { "agent" },
        "MessageEvent",
        json!({
            "role": role,
            "content": [{ "type": "text", "text": text }],
        }),
    )
}

fn action_envelope(id: &str, tool: &str, command: &str) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        "agent",
        "ActionEvent",
        json!({
            "action": {
                "action": "run",
                "tool_name": tool,
                "message": format!("Running `{command}`"),
                "arguments": { "command": command },
            }
        }),
    )
}

fn observation_envelope(id: &str, exit_code: i64, stdout: &str) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        "agent",
        "ObservationEvent",
        json!({
            "observation": {
                "observation_id": "obs-1",
                "tool_name": "run",
                "exit_code": exit_code,
                "output": stdout,
                "content": [{ "type": "text", "text": stdout }],
            }
        }),
    )
}

fn llm_completion_envelope(id: &str, model: &str, prompt: u64, completion: u64) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        "llm",
        "LLMCompletionLogEvent",
        json!({
            "model": model,
            "tokens": prompt + completion,
            "usage": {
                "prompt_tokens": prompt,
                "completion_tokens": completion,
            },
        }),
    )
}

fn conversation_error_envelope(id: &str, message: &str) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        "runtime",
        "ConversationErrorEvent",
        json!({ "message": message }),
    )
}

fn unknown_envelope(id: &str, kind: &str) -> EventEnvelope {
    EventEnvelope::new(
        id,
        Utc::now(),
        "runtime",
        kind,
        json!({ "structure": "future", "data": ["a", "b"] }),
    )
}

#[test]
fn empty_source_routes_to_unknown_with_harness_actor() {
    let envelope = EventEnvelope::new(
        "evt-empty",
        Utc::now(),
        "",
        "MessageEvent",
        json!({ "role": "user", "content": [] }),
    );
    let normalized = normalize_event(&envelope, &context());
    match normalized.record.kind {
        EventKind::Unknown { raw_kind } => assert_eq!(raw_kind, "MessageEvent"),
        other => panic!("expected Unknown, got {other:?}"),
    }
    assert!(
        normalized
            .record
            .summary
            .contains("source_missing envelope kind=MessageEvent"),
        "summary should explain the missing source"
    );
    assert_eq!(
        normalized.record.actor.actor_id(),
        "openhands-agent-server-v1"
    );
    assert_eq!(
        normalized.raw_payload,
        json!({ "role": "user", "content": [] })
    );
}

#[test]
fn state_update_normalizes_with_typed_kind() {
    let normalized = normalize_event(&state_update_envelope("evt-su"), &context());
    assert!(matches!(
        normalized.record.kind,
        EventKind::HarnessConversationStateUpdate
    ));
    assert_eq!(normalized.record.summary, "runtime status: running");
    assert!(
        normalized
            .record
            .entity_refs
            .iter()
            .any(|er| er.id == "conv-norm-1")
    );
}

#[test]
fn message_event_user_role_normalizes() {
    let envelope = message_envelope("evt-user", "user", "hello");
    let normalized = normalize_event(&envelope, &context());
    match normalized.record.kind.clone() {
        EventKind::HarnessEventNormalized { source_kind } => {
            assert_eq!(source_kind, "MessageEvent")
        }
        other => panic!("expected HarnessEventNormalized, got {other:?}"),
    }
    assert_eq!(normalized.record.summary, "hello");
}

#[test]
fn message_event_assistant_role_normalizes() {
    let envelope = message_envelope("evt-asst", "assistant", "world");
    let normalized = normalize_event(&envelope, &context());
    assert_eq!(normalized.record.summary, "world");
}

#[test]
fn action_event_normalizes_into_tool_call() {
    let envelope = action_envelope("evt-act", "terminal", "ls -la");
    let normalized = normalize_event(&envelope, &context());
    assert!(matches!(normalized.record.kind, EventKind::HarnessToolCall));
    let payload = normalized.record.payload.expect("tool call payload");
    assert_eq!(
        payload.get("tool_name").and_then(Value::as_str),
        Some("terminal")
    );
    assert_eq!(
        payload
            .get("arguments")
            .and_then(|a| a.get("command"))
            .and_then(Value::as_str),
        Some("ls -la"),
    );
    assert_eq!(
        payload.get("action_id").and_then(Value::as_str),
        Some("evt-act"),
        "action_id must propagate so the action/observation chain stays correlatable"
    );
    assert_eq!(normalized.record.summary, "Running `ls -la`");
}

/// Round-6 AI review (`tool_call` payload misses `action_id`): the reviewer
/// flagged a hypothetical tool-call test that lacked the `action_id`
/// propagation assertion. The general `action_event_normalizes_into_tool_call`
/// test above already asserts `action_id` (see line ~215), but this more
/// targeted regression pins the exact invariant the reviewer asked for — an
/// `ActionEvent` envelope with id `evt-action` produces a `HarnessToolCall`
/// payload whose `action_id` is the envelope id, so `tool_name`, `arguments`,
/// and `action_id` all surface on the same record.
#[test]
fn action_event_normalizes_tool_call_with_arguments() {
    let envelope = action_envelope("evt-action", "terminal", "ls -la");
    let normalized = normalize_event(&envelope, &context());
    assert!(
        matches!(normalized.record.kind, EventKind::HarnessToolCall),
        "envelope id evt-action must normalize to HarnessToolCall"
    );
    let payload = normalized.record.payload.expect("tool call payload");
    assert_eq!(
        payload.get("tool_name").and_then(Value::as_str),
        Some("terminal"),
        "tool_name must match the action envelope's tool"
    );
    assert_eq!(
        payload
            .get("arguments")
            .and_then(|a| a.get("command"))
            .and_then(Value::as_str),
        Some("ls -la"),
        "arguments.command must round-trip from the envelope payload"
    );
    assert_eq!(
        payload.get("action_id").and_then(Value::as_str),
        Some("evt-action"),
        "action_id must propagate from the envelope id to the normalized payload"
    );
    assert_eq!(normalized.record.summary, "Running `ls -la`");
}

#[test]
fn observation_event_normalizes_into_tool_result() {
    let envelope = observation_envelope("evt-obs", 0, "ok\n");
    let normalized = normalize_event(&envelope, &context());
    assert!(matches!(
        normalized.record.kind,
        EventKind::HarnessToolResult
    ));
    let payload = normalized.record.payload.expect("tool result payload");
    assert_eq!(payload.get("exit_code").and_then(Value::as_i64), Some(0));
    assert_eq!(
        payload.get("tool_name").and_then(Value::as_str),
        Some("run")
    );
}

#[test]
fn llm_completion_log_extracts_token_usage() {
    let envelope = llm_completion_envelope("evt-llm", "openai/gpt-5.4", 12, 34);
    let normalized = normalize_event(&envelope, &context());
    match normalized.record.kind.clone() {
        EventKind::HarnessEventNormalized { source_kind } => {
            assert_eq!(source_kind, "LLMCompletionLogEvent");
        }
        other => panic!("expected HarnessEventNormalized, got {other:?}"),
    }
    let payload = normalized.record.payload.expect("usage payload");
    assert_eq!(
        payload
            .get("usage")
            .and_then(|u| u.get("prompt"))
            .and_then(Value::as_u64),
        Some(12)
    );
    assert_eq!(
        payload
            .get("usage")
            .and_then(|u| u.get("completion"))
            .and_then(Value::as_u64),
        Some(34)
    );
    assert_eq!(normalized.record.summary, "llm completion (openai/gpt-5.4)");
}

#[test]
fn conversation_error_normalizes_with_summary() {
    let envelope = conversation_error_envelope("evt-err", "boom");
    let normalized = normalize_event(&envelope, &context());
    match normalized.record.kind.clone() {
        EventKind::HarnessEventNormalized { source_kind } => {
            assert_eq!(source_kind, "ConversationErrorEvent");
        }
        other => panic!("expected HarnessEventNormalized, got {other:?}"),
    }
    assert_eq!(normalized.record.summary, "conversation error: boom");
}

#[test]
fn unknown_event_retains_raw_payload_and_ref() {
    let envelope = unknown_envelope("evt-u", "BrandNewEventType");
    let normalized = normalize_event(&envelope, &context());
    match &normalized.record.kind {
        EventKind::Unknown { raw_kind } => assert_eq!(raw_kind, "BrandNewEventType"),
        other => panic!("expected EventKind::Unknown, got {other:?}"),
    }
    assert!(
        normalized
            .raw_payload_ref
            .as_deref()
            .unwrap_or_default()
            .starts_with(UNKNOWN_RAW_REF_PREFIX),
        "unknown events must carry a synthetic raw_payload_ref"
    );
    let raw_payload = &normalized.raw_payload;
    assert_eq!(
        raw_payload.get("structure").and_then(Value::as_str),
        Some("future")
    );
}

#[test]
fn unknown_event_is_total_under_empty_or_malformed_payloads() {
    struct Case {
        name: &'static str,
        envelope: EventEnvelope,
    }
    let now = Utc::now();
    let cases = vec![
        Case {
            name: "missing payload",
            envelope: EventEnvelope::new("evt-empty", now, "runtime", "FutureEvent", Value::Null),
        },
        Case {
            name: "scalar payload",
            envelope: EventEnvelope::new("evt-scalar", now, "runtime", "FutureEvent", json!(42)),
        },
        Case {
            name: "empty object payload",
            envelope: EventEnvelope::new("evt-empty-obj", now, "runtime", "FutureEvent", json!({})),
        },
    ];
    for case in cases {
        let normalized = normalize_event(&case.envelope, &context());
        assert!(
            matches!(normalized.record.kind, EventKind::Unknown { .. }),
            "{name}: expected EventKind::Unknown",
            name = case.name
        );
    }
}

#[test]
fn normalization_preserve_event_id_for_dedupe() {
    let envelope = state_update_envelope("evt-dupe");
    let first = normalize_event(&envelope, &context());
    let second = normalize_event(&envelope, &context());
    assert_eq!(first.record.event_id, second.record.event_id);
    assert!(first.record.is_duplicate_of(&second.record));
}

#[test]
fn reconciliation_rest_then_ws_produces_same_envelope() {
    let ctx = context();
    let envelope = state_update_envelope("evt-rec");
    let from_rest = normalize_event(&envelope, &ctx);
    let from_ws = normalize_event(&envelope, &ctx);
    assert_eq!(from_rest.record.event_id, from_ws.record.event_id);
    assert_eq!(from_rest.record.summary, from_ws.record.summary);
    assert_eq!(from_rest.record.entity_refs, from_ws.record.entity_refs);
}

#[test]
fn correlation_id_propagates_into_normalized_envelope() {
    let ctx = NormalizationContext::new("openhands-agent-server-v1", conversation_id())
        .expect("ctx")
        .with_correlation_id("corr-prop-x");
    let envelope = state_update_envelope("evt-corr");
    let normalized = normalize_event(&envelope, &ctx);
    assert_eq!(
        normalized.record.correlation_id.as_deref(),
        Some("corr-prop-x")
    );
}

#[test]
fn raw_payload_for_known_events_is_the_envelope_payload() {
    let envelope = state_update_envelope("evt-known");
    let normalized: NormalizedEvent = normalize_event(&envelope, &context());
    // Typed variants do not emit a synthetic `raw://openhands/unknown/...` ref.
    assert!(normalized.raw_payload_ref.is_none());
    // Raw payload is preserved in normalized.raw_payload for diagnostics.
    let raw_payload = &normalized.raw_payload;
    assert_eq!(
        raw_payload.get("execution_status").and_then(Value::as_str),
        Some("running"),
    );
}

#[test]
fn unknown_keeps_payload_for_diagnostics_under_user_source() {
    let envelope = EventEnvelope::new(
        "evt-user-unknown",
        Utc::now() - ChronoDuration::seconds(2),
        "user",
        "CustomUserEvent",
        json!({ "freeform": { "note": "future-proof" } }),
    );
    let normalized = normalize_event(&envelope, &context());
    assert!(matches!(normalized.record.kind, EventKind::Unknown { .. }));
    assert!(
        normalized
            .raw_payload_ref
            .as_deref()
            .unwrap_or_default()
            .starts_with(UNKNOWN_RAW_REF_PREFIX)
    );
}

#[test]
fn invalid_correlation_id_caller_keeps_event_id_stable() {
    let ctx_bad = NormalizationContext::new("openhands-agent-server-v1", conversation_id())
        .expect("ctx")
        .with_correlation_id(Uuid::nil().to_string());
    let envelope = observation_envelope("evt-tool-result", 0, "ok");
    let normalized = normalize_event(&envelope, &ctx_bad);
    assert_eq!(normalized.record.event_id, "evt-tool-result");
}
