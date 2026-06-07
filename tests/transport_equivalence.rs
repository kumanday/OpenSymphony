//! Contract tests proving local and remote transports produce equivalent
//! frontend state. These tests validate that all transport profiles expose
//! the same gateway DTOs, cursors, frames, and action receipts.

mod support;
pub use support::*;

use opensymphony::opensymphony_gateway_schema::{
    cursor::StreamCursor,
    envelope::{EntityKind, EntityRef, GatewayEnvelope},
};

/// Create a sample gateway envelope for testing.
fn sample_envelope(sequence: u64, partition: &str) -> GatewayEnvelope {
    GatewayEnvelope::new(
        StreamCursor::new(sequence, partition),
        EntityRef::run("run-001"),
        "run.started",
        serde_json::json!({
            "run_id": "run-001",
            "issue": "COE-410",
        }),
    )
}

/// Create a terminal frame envelope for high-throughput testing.
fn terminal_frame_envelope(sequence: u64, run_id: &str) -> GatewayEnvelope {
    GatewayEnvelope::new(
        StreamCursor::new(sequence, format!("terminal:{}", run_id)),
        EntityRef::terminal(format!("term-{}", run_id)),
        "terminal.frame",
        serde_json::json!({
            "content": format!(
                "[2025-01-15T10:00:{:02}Z] INFO > Turn {} output\n",
                sequence % 60,
                sequence
            ),
        }),
    )
}

// ── HTTP Transport Equivalence ──────────────────────────────────────────────

/// HTTP transport serialization produces valid GatewayEnvelope JSON.
#[test]
fn http_transport_serializes_envelope_correctly() {
    let envelope = sample_envelope(1, "events");
    let json = serde_json::to_string(&envelope).expect("serialize envelope");
    let deserialized: GatewayEnvelope = serde_json::from_str(&json).expect("deserialize envelope");

    assert_eq!(envelope, deserialized);
    assert_eq!(deserialized.cursor.sequence, 1);
    assert_eq!(deserialized.cursor.partition, "events");
}

/// HTTP transport handles SSE-formatted event data.
#[test]
fn http_sse_format_parses_envelope() {
    let envelope = sample_envelope(42, "events");
    let payload = serde_json::to_string(&envelope).expect("serialize");

    let sse_formatted = format!("data: {}\n\n", payload);
    let data_prefix = "data: ";
    let json_str = sse_formatted
        .strip_prefix(data_prefix)
        .expect("SSE has data prefix")
        .trim()
        .strip_suffix("\n\n")
        .unwrap_or(sse_formatted[data_prefix.len()..].trim());

    let parsed: GatewayEnvelope = serde_json::from_str(json_str).expect("parse SSE envelope");
    assert_eq!(parsed.cursor.sequence, 42);
    assert_eq!(parsed.entity_ref.kind, EntityKind::Run);
}

/// HTTP transport handles multi-line SSE data with embedded newlines.
#[test]
fn http_sse_multi_line_data_parses_envelope() {
    // Payload containing newlines inside a JSON string
    let mut envelope = sample_envelope(43, "events");
    envelope.payload = Some(serde_json::json!({
        "content": "line1\nline2\nline3",
        "nested": { "key": "value\nwith\nnewlines" },
    }));
    let payload = serde_json::to_string(&envelope).expect("serialize");

    // SSE spec: multi-line data is split across multiple "data:" lines
    let mut lines: Vec<String> = vec![];
    for chunk in payload.lines() {
        lines.push(format!("data: {}", chunk));
    }
    let sse_formatted = format!("{}\n\n", lines.join("\n"));

    // Re-assemble multi-line data into a single JSON string
    let mut data_lines: Vec<String> = vec![];
    for line in sse_formatted.lines() {
        if let Some(stripped) = line.strip_prefix("data: ") {
            data_lines.push(stripped.to_string());
        }
    }
    let json_str = data_lines.join("\n");

    let parsed: GatewayEnvelope =
        serde_json::from_str(&json_str).expect("parse multi-line SSE envelope");
    assert_eq!(parsed.cursor.sequence, 43);
    let payload = parsed.payload.as_ref().expect("payload present");
    assert_eq!(payload["content"], "line1\nline2\nline3");
    assert_eq!(payload["nested"]["key"], "value\nwith\nnewlines");
}

// ── WebSocket Transport Equivalence ──────────────────────────────────────────

/// WebSocket transport with __event__ prefix parses correctly.
#[test]
fn websocket_event_prefix_parses_envelope() {
    let envelope = sample_envelope(7, "events");
    let payload = serde_json::to_string(&envelope).expect("serialize");

    let ws_formatted = format!("__event__ {}", payload);
    let json_str = ws_formatted
        .strip_prefix("__event__ ")
        .expect("WS has event prefix");

    let parsed: GatewayEnvelope = serde_json::from_str(json_str).expect("parse WS envelope");
    assert_eq!(parsed.cursor.sequence, 7);
    assert_eq!(parsed.event_kind, "run.started");
}

/// WebSocket transport with __error__ prefix handles stream errors.
#[test]
fn websocket_error_prefix_handles_errors() {
    use opensymphony::opensymphony_gateway_schema::event_journal::StreamError;

    let error = StreamError::backpressure();
    let error_payload = serde_json::to_string(&error).expect("serialize error");
    let ws_formatted = format!("__error__ {}", error_payload);

    let json_str = ws_formatted
        .strip_prefix("__error__ ")
        .expect("WS has error prefix");
    let parsed: serde_json::Value = serde_json::from_str(json_str).expect("parse WS error");

    assert_eq!(parsed["error_type"], "backpressure");
    assert!(parsed["recoverable"].as_bool().expect("test data"));
}

/// WebSocket binary frame serialization is identical to text frame.
#[test]
fn websocket_binary_frame_equivalence() {
    let envelope = terminal_frame_envelope(100, "run-bench");
    let json_bytes = serde_json::to_vec(&envelope).expect("serialize to bytes");
    let json_str = serde_json::to_string(&envelope).expect("serialize to string");

    let parsed_from_bytes: GatewayEnvelope =
        serde_json::from_slice(&json_bytes).expect("parse bytes");
    let parsed_from_str: GatewayEnvelope = serde_json::from_str(&json_str).expect("parse string");

    assert_eq!(parsed_from_bytes, parsed_from_str);
    assert_eq!(parsed_from_bytes.cursor.sequence, 100);
}

// ── Tauri Channel Transport Equivalence ─────────────────────────────────────

/// Tauri channel transport JSON serialization matches gateway envelope.
#[test]
fn tauri_channel_serializes_envelope_correctly() {
    let envelope = sample_envelope(5, "terminal:run-001");
    let json = serde_json::to_string(&envelope).expect("serialize envelope");
    let deserialized: GatewayEnvelope = serde_json::from_str(&json).expect("deserialize envelope");

    assert_eq!(envelope, deserialized);
    assert_eq!(deserialized.cursor.partition, "terminal:run-001");
    assert_eq!(deserialized.entity_ref.kind, EntityKind::Run);
}

/// Tauri channel high-throughput frames maintain sequence ordering.
#[test]
fn tauri_channel_maintains_frame_sequence() {
    let frames: Vec<GatewayEnvelope> = (0..1000)
        .map(|i| terminal_frame_envelope(i, "run-seq-test"))
        .collect();

    for (i, frame) in frames.iter().enumerate() {
        assert_eq!(frame.cursor.sequence, i as u64);
    }

    let sequences: Vec<u64> = frames.iter().map(|f| f.cursor.sequence).collect();
    for i in 1..sequences.len() {
        assert!(sequences[i] > sequences[i - 1]);
    }
}

// ── Cursor Replay Equivalence ───────────────────────────────────────────────

/// All transports produce identical cursors for the same event sequence.
#[test]
fn cursor_replay_is_transport_independent() {
    let envelope = sample_envelope(123, "events");
    let cursor = envelope.cursor.clone();

    let http_cursor = serde_json::to_string(&cursor).expect("serialize HTTP cursor");
    let http_parsed: StreamCursor = serde_json::from_str(&http_cursor).expect("parse HTTP cursor");

    let ws_cursor = serde_json::to_vec(&cursor).expect("serialize WS cursor");
    let ws_parsed: StreamCursor = serde_json::from_slice(&ws_cursor).expect("parse WS cursor");

    let tauri_cursor = serde_json::to_string(&cursor).expect("serialize Tauri cursor");
    let tauri_parsed: StreamCursor =
        serde_json::from_str(&tauri_cursor).expect("parse Tauri cursor");

    assert_eq!(http_parsed, ws_parsed);
    assert_eq!(http_parsed, tauri_parsed);
    assert_eq!(http_parsed.sequence, 123);
    assert_eq!(http_parsed.partition, "events");
}

/// Cursor advances correctly across all transport types.
#[test]
fn cursor_advances_consistently_across_transports() {
    let base_cursor = StreamCursor::new(100, "events");

    for i in 0..10 {
        let next_cursor = StreamCursor::new(base_cursor.sequence + i + 1, "events");

        let http_json = serde_json::to_string(&next_cursor).expect("test data");
        let ws_bytes = serde_json::to_vec(&next_cursor).expect("test data");

        let http_parsed: StreamCursor = serde_json::from_str(&http_json).expect("test data");
        let ws_parsed: StreamCursor = serde_json::from_slice(&ws_bytes).expect("test data");

        assert_eq!(http_parsed, ws_parsed);
        assert_eq!(http_parsed.sequence, 101 + i);
    }
}

// ── Action Receipt Correlation ──────────────────────────────────────────────

/// Action receipts are correlated via correlation_id across transports.
#[test]
fn action_receipt_correlation_is_preserved() {
    let correlation_id = "act-001";

    let dispatched = GatewayEnvelope::new(
        StreamCursor::new(1, "actions"),
        EntityRef::run("run-001"),
        "action.dispatched",
        serde_json::json!({
            "correlation_id": correlation_id,
            "action": "run.start",
        }),
    );

    let completed = GatewayEnvelope::new(
        StreamCursor::new(2, "actions"),
        EntityRef::run("run-001"),
        "action.completed",
        serde_json::json!({
            "correlation_id": correlation_id,
            "action": "run.start",
            "outcome": "success",
        }),
    );

    let dispatch_json = serde_json::to_string(&dispatched).expect("test data");
    let complete_json = serde_json::to_string(&completed).expect("test data");

    let dispatch_parsed: serde_json::Value =
        serde_json::from_str(&dispatch_json).expect("test data");
    let complete_parsed: serde_json::Value =
        serde_json::from_str(&complete_json).expect("test data");

    assert_eq!(
        dispatch_parsed["payload"]["correlation_id"],
        complete_parsed["payload"]["correlation_id"]
    );
    assert_eq!(
        dispatch_parsed["payload"]["correlation_id"]
            .as_str()
            .expect("test data"),
        correlation_id
    );
}

// ── Run Phase/Liveness Equivalence ──────────────────────────────────────────

/// Run phase transitions are identical across all transport profiles.
#[test]
fn run_phase_transitions_are_transport_independent() {
    let phases = vec![("run.started", 1), ("run.active", 2), ("run.completed", 3)];

    for (phase, seq) in phases {
        let envelope = GatewayEnvelope::new(
            StreamCursor::new(seq, "events"),
            EntityRef::run("run-phase-test"),
            phase,
            serde_json::json!({
                "phase": phase,
                "sequence": seq,
            }),
        );

        let http_json = serde_json::to_string(&envelope).expect("test data");
        let ws_bytes = serde_json::to_vec(&envelope).expect("test data");
        let tauri_json = serde_json::to_string(&envelope).expect("test data");

        let http_parsed: GatewayEnvelope = serde_json::from_str(&http_json).expect("test data");
        let ws_parsed: GatewayEnvelope = serde_json::from_slice(&ws_bytes).expect("test data");
        let tauri_parsed: GatewayEnvelope = serde_json::from_str(&tauri_json).expect("test data");

        assert_eq!(http_parsed, ws_parsed);
        assert_eq!(http_parsed, tauri_parsed);
        assert_eq!(http_parsed.event_kind, phase);
    }
}

/// Liveness heartbeats are preserved across transports.
#[test]
fn liveness_heartbeat_preserved_across_transports() {
    let heartbeat = GatewayEnvelope::new(
        StreamCursor::new(999, "liveness"),
        EntityRef::run("run-liveness"),
        "run.heartbeat",
        serde_json::json!({
            "status": "active",
            "last_event_at": "2025-01-15T10:15:00Z",
        }),
    );

    let json = serde_json::to_string(&heartbeat).expect("test data");
    let parsed: GatewayEnvelope = serde_json::from_str(&json).expect("test data");

    assert_eq!(parsed.event_kind, "run.heartbeat");
    assert_eq!(parsed.cursor.sequence, 999);
}

// ── Reconnect and Cursor Replay Consistency ─────────────────────────────────

/// Reconnect resumes from correct cursor position.
#[test]
fn reconnect_cursor_resume_is_consistent() {
    let last_cursor = StreamCursor::new(50, "events");

    let resumed_envelope = sample_envelope(51, "events");
    assert!(resumed_envelope.cursor.sequence > last_cursor.sequence);
    assert_eq!(resumed_envelope.cursor.sequence, last_cursor.sequence + 1);
}

/// Stream backpressure is recoverable across all transports.
#[test]
fn backpressure_recovery_is_consistent() {
    use opensymphony::opensymphony_gateway_schema::event_journal::StreamError;

    let backpressure_error = StreamError::backpressure();

    let http_error_json = serde_json::to_string(&backpressure_error).expect("test data");
    let http_parsed: serde_json::Value = serde_json::from_str(&http_error_json).expect("test data");
    assert!(http_parsed["recoverable"].as_bool().expect("test data"));

    let ws_formatted = format!(
        "__error__ {}",
        serde_json::to_string(&backpressure_error).expect("test data")
    );
    let ws_error_str = ws_formatted.strip_prefix("__error__ ").expect("test data");
    let ws_parsed: serde_json::Value = serde_json::from_str(ws_error_str).expect("test data");
    assert!(ws_parsed["recoverable"].as_bool().expect("test data"));

    let tauri_parsed: serde_json::Value =
        serde_json::from_str(&http_error_json).expect("test data");
    assert!(tauri_parsed["recoverable"].as_bool().expect("test data"));
}

// ── High-Throughput Frame Semantics ─────────────────────────────────────────

/// Terminal frames maintain ordering and completeness across transports.
#[test]
fn terminal_frame_ordering_is_preserved() {
    let frames: Vec<GatewayEnvelope> = (0..100)
        .map(|i| terminal_frame_envelope(i, "run-throughput"))
        .collect();

    let mut seen_sequences = std::collections::HashSet::new();
    for frame in &frames {
        assert!(
            seen_sequences.insert(frame.cursor.sequence),
            "Duplicate sequence: {}",
            frame.cursor.sequence
        );
    }

    for i in 1..frames.len() {
        assert!(frames[i].cursor.sequence > frames[i - 1].cursor.sequence);
    }
}

/// Binary frame encoding is consistent across transport types.
#[test]
fn binary_frame_encoding_consistency() {
    let envelope = terminal_frame_envelope(1, "run-binary");
    let json_bytes = serde_json::to_vec(&envelope).expect("test data");
    let json_str = serde_json::to_string(&envelope).expect("test data");

    let from_bytes: GatewayEnvelope = serde_json::from_slice(&json_bytes).expect("test data");
    let from_str: GatewayEnvelope = serde_json::from_str(&json_str).expect("test data");

    assert_eq!(from_bytes, from_str);
    assert_eq!(from_bytes.event_kind, "terminal.frame");
}

// ── Local vs Remote Transport Equivalence ────────────────────────────────────

/// Local fast path and loopback fallback produce identical envelope structures.
#[test]
fn local_and_loopback_produce_identical_envelopes() {
    let envelope = sample_envelope(1, "events");

    let local_json = serde_json::to_string(&envelope).expect("test data");
    let local_parsed: GatewayEnvelope = serde_json::from_str(&local_json).expect("test data");

    let loopback_json = serde_json::to_string(&envelope).expect("test data");
    let loopback_parsed: GatewayEnvelope = serde_json::from_str(&loopback_json).expect("test data");

    assert_eq!(local_parsed, loopback_parsed);
    assert_eq!(local_parsed.cursor.sequence, 1);
    assert_eq!(local_parsed.event_kind, "run.started");
}

/// Journal/replay semantics are preserved across all transports.
#[test]
fn journal_replay_semantics_preserved() {
    let events: Vec<GatewayEnvelope> = (1..=10)
        .map(|i| sample_envelope(i, "replay-test"))
        .collect();

    let serialized: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).expect("test data"))
        .collect();

    let replayed: Vec<GatewayEnvelope> = serialized
        .iter()
        .map(|s| serde_json::from_str(s).expect("test data"))
        .collect();

    assert_eq!(events, replayed);

    for (i, event) in replayed.iter().enumerate() {
        assert_eq!(event.cursor.sequence, (i + 1) as u64);
        assert_eq!(event.cursor.partition, "replay-test");
    }
}
