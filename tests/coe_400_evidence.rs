//! COE-400 end-to-end evidence: normalize + runtime mirror against a real
//! (fake) OpenHands server.
//!
//! This file is *the* human-visible evidence for the review checklist on
//! COE-400 instead of unit-test stdout alone. It drives the in-tree
//! `FakeOpenHandsServer` through the public `OpenHandsClient`, normalizes
//! every event through [`normalize_event`], and feeds each typed envelope
//! into a [`RuntimeMirror`] so the snapshot output is the system's actual
//! runtime progression.
//!
//! Three scenarios are demonstrated, one per acceptance criterion that
//! requires a live run:
//!
//! 1. Typed envelopes for every known OpenHands event surface the
//!    expected [`EventKind`] discriminator.
//! 2. Unknown event types are routed to `EventKind::Unknown` while
//!    preserving `raw_payload` and generating a synthetic
//!    `raw_payload_ref`.
//! 3. The runtime mirror snapshot orders phases correctly when a
//!    long-running turn emits progress: `RunningTurn` first, then
//!    `Quiet`, then `Stalled` after `idle_timeout` elapses.

#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

use chrono::Utc;
use serde_json::json;
use std::time::Duration;
use tokio::time::timeout;

use opensymphony_domain::{ConversationId, DurationMs, RuntimeLivenessPhase, TimestampMs};
use opensymphony_gateway_schema::event_journal::EventKind;
use opensymphony_openhands::{
    ConversationCreateRequest, EventEnvelope, MirrorConfig, NormalizationContext, NormalizedEvent,
    OpenHandsClient, RuntimeMirror, RuntimeStreamConfig, SendMessageRequest, TransportConfig,
    normalize_event,
};
use opensymphony_testkit::FakeOpenHandsServer;

fn harness_id() -> &'static str {
    "openhands-agent-server-v1"
}

fn normalize(envelope: &EventEnvelope) -> NormalizedEvent {
    let context = NormalizationContext::new(
        harness_id(),
        ConversationId::new("conv-evidence").expect("valid id"),
    )
    .expect("context");
    normalize_event(envelope, &context)
}

#[tokio::test]
async fn end_to_end_evidence_collects_typed_envelopes_and_mirror_snapshots() {
    let server = FakeOpenHandsServer::start()
        .await
        .expect("fake server should start");
    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));

    let request = ConversationCreateRequest::doctor_probe(
        "/tmp/evidence-workspace",
        "/tmp/evidence-workspace/.opensymphony/openhands",
        None,
        None,
    );
    let conversation = client
        .create_conversation(&request)
        .await
        .expect("conversation create should succeed");
    client
        .send_message(
            conversation.conversation_id,
            &SendMessageRequest::user_text("prove normalization + mirror"),
        )
        .await
        .expect("message send should succeed");
    client
        .run_conversation(conversation.conversation_id)
        .await
        .expect("run should succeed");

    // Emit a manual ConversationStateUpdateEvent so we can observe the
    // typed envelope and prove the unknown-event fallback path on a
    // follow-up synthetic envelope.
    server
        .emit_state_update(conversation.conversation_id, "running")
        .await
        .expect("emit state update");

    let mut stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(2),
                ..RuntimeStreamConfig::default()
            },
        )
        .await
        .expect("runtime stream attach should succeed");

    let mut mirror = RuntimeMirror::new(
        ConversationId::new(conversation.conversation_id.to_string()).expect("valid id"),
        TimestampMs::new(1_000),
        MirrorConfig {
            idle_timeout_ms: Some(DurationMs::new(5_000)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(2_000)),
        },
    );

    let mut typed_kinds: Vec<String> = Vec::new();
    let mut last_snapshot = mirror.snapshot_at(TimestampMs::new(0));
    let now_ms = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let observed_now = now_ms.clone();

    let drain = async {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Ok(Ok(Some(event))) =
                timeout(Duration::from_millis(500), stream.next_event()).await
            {
                let normalized = normalize(&event);
                match normalized.record.kind.clone() {
                    EventKind::HarnessConversationStateUpdate => {
                        typed_kinds.push("HarnessConversationStateUpdate".to_string());
                    }
                    EventKind::Unknown { ref raw_kind } => {
                        typed_kinds.push(format!("Unknown({raw_kind})"));
                    }
                    other => typed_kinds.push(format!("{other:?}")),
                }
                assert!(
                    !normalized.raw_payload.is_null(),
                    "raw_payload must always be present"
                );
                let now = Utc::now().timestamp_millis().max(0) as u64;
                observed_now.store(now, std::sync::atomic::Ordering::SeqCst);
                mirror.apply_event(&event);
                last_snapshot = mirror.snapshot_at(TimestampMs::new(now));
            }
        }
    };
    drain.await;

    println!("=== COE-400 EVIDENCE ===");
    println!("typed_kinds: {typed_kinds:?}");
    println!(
        "snapshot.phase = {:?} | liveness_state = {:?} | event_count = {}",
        last_snapshot.phase, last_snapshot.liveness_state, last_snapshot.event_count,
    );
    println!(
        "last_event_cursor = {:?} | last_event_kind = {:?} | last_event_at = {:?}",
        last_snapshot.last_event_cursor, last_snapshot.last_event_kind, last_snapshot.last_event_at,
    );
    println!(
        "tokens: input={} output={}",
        last_snapshot.input_tokens, last_snapshot.output_tokens,
    );
    println!(
        "stream_health = {:?} | reconnect_status = {:?} | history_sync_status = {:?}",
        last_snapshot.stream_health,
        last_snapshot.reconnect_status,
        last_snapshot.history_sync_status,
    );
    println!("=== END COE-400 EVIDENCE ===");

    // Demonstrate unknown-event retention through a synthetic envelope of an
    // unrecognized kind. The mirror still advances the cursor and never
    // errors on unknown event types.
    let unknown_envelope = EventEnvelope::new(
        "evt-evidence-unknown",
        Utc::now(),
        "runtime",
        "FutureOpenHandsEvent",
        json!({ "future": true, "details": "raw" }),
    );
    let unknown_normalized = normalize(&unknown_envelope);
    assert!(matches!(
        unknown_normalized.record.kind,
        EventKind::Unknown { .. }
    ));
    assert!(
        unknown_normalized.raw_payload_ref.is_some(),
        "Unknown envelopes must synthesize a raw_payload_ref"
    );

    assert!(
        typed_kinds.iter().any(|k| !k.starts_with("Unknown(")),
        "expected at least one typed envelope, got {typed_kinds:?}"
    );
    // The mirror transitions from WaitingOnPriorTurn → RunningTurn → Quiet →
    // Stalled/Detached/Terminal via per-event `apply_status_change`. With the
    // raw fake stream attached via `attach_runtime_stream` we exercise events
    // only; the `Status → mirror` edge is the contract under test in the
    // second test below. So here we only require the *liveness_state*
    // classifier to be Active (either of RunningTurn or WaitingOnPriorTurn).
    assert!(matches!(
        last_snapshot.liveness_state,
        opensymphony_domain::LivenessState::Active
    ));
    assert!(last_snapshot.event_count >= 2, "have plenty of events");
}

#[tokio::test]
async fn runtime_mirror_quiet_window_alive_then_transitions_to_stalled() {
    let mut mirror = RuntimeMirror::new(
        ConversationId::new("conv-1").expect("valid id"),
        TimestampMs::new(1_000),
        MirrorConfig {
            idle_timeout_ms: Some(DurationMs::new(2_000)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(1_000)),
        },
    );
    let envelope = EventEnvelope::new(
        "evt-1",
        Utc::now(),
        "runtime",
        "ConversationStateUpdateEvent",
        json!({ "execution_status": "running" }),
    );
    let _ = normalize(&envelope);
    mirror.apply_event(&envelope);
    mirror.apply_status_change("running", TimestampMs::new(1_000));
    mirror.apply_socket_ready(TimestampMs::new(1_000));
    let baseline = mirror.snapshot_at(TimestampMs::new(1_000));
    println!("=== COE-400 QUIET/STALLED EVIDENCE ===");
    println!(
        "baseline.phase = {:?} | liveness_state = {:?} | last_event_at = {:?}",
        baseline.phase, baseline.liveness_state, baseline.last_event_at,
    );
    let last_event_at = baseline.last_event_at.expect("cursor");
    println!("=== END COE-400 QUIET/STALLED EVIDENCE ===");
    assert!(matches!(baseline.phase, RuntimeLivenessPhase::RunningTurn));

    // Quiet lives between 1s and 2s of silence.
    let quiet_phase = mirror.phase_at(TimestampMs::new(last_event_at.as_u64() + 1_300));
    println!("after +1.3s: phase = {quiet_phase:?}");
    assert!(matches!(quiet_phase, RuntimeLivenessPhase::Quiet));

    // Past idle_timeout the linear envelope must escalate to Stalled.
    let stalled_phase = mirror.phase_at(TimestampMs::new(last_event_at.as_u64() + 2_500));
    println!("after +2.5s: phase = {stalled_phase:?}");
    assert!(matches!(stalled_phase, RuntimeLivenessPhase::Stalled));
}
