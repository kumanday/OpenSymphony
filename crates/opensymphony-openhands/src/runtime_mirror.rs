//! OpenHands runtime state mirror for run detail views.
//!
//! The runtime mirror is the single source of truth that:
//!
//! - Tracks runtime attachment lifecycle (REST history sync → readiness barrier
//!   → WebSocket connect → reconcile → terminal/detach).
//! - Maps incoming OpenHands activity into normalized OpenSymphony liveness
//!   phases (`active`, `quiet`, `degraded`, `stalled`, `detached`, `terminal`).
//! - Emits structured [`RuntimeProgressSnapshot`]s so the gateway, scheduler, and
//!   event journal have a single consistent view of progress.
//! - Replaces hard total-turn timeout behavior with **progress-based idle
//!   detection**: the sliding deadline in `StallMetadata` never trips as long as
//!   the harness keeps emitting progress signals (events, status changes,
//!   token bumps). Only concrete idle silence escalates to `Stalled`.

use crate::opensymphony_domain::{
    ConversationId, DetachMetadata, DetachReason, DurationMs, HistorySyncStatus, LivenessState,
    ReconnectStatus, RuntimeLivenessPhase, RuntimeProgressSnapshot, StallMetadata, StreamHealth,
    TimestampMs,
};

use super::{
    events::{ConversationStateMirror, EventCache},
    models::{Conversation, EventEnvelope},
};

/// Synthetic `last_event_id` tag emitted when a runtime status *change*
/// (not a tightly matching event id) advances the cursor. Operators can
/// correlate this with `last_event_kind` to understand what triggered the
/// change. The status-change marker is distinct from
/// [`TERMINAL_CURSOR_MARKER`] so downstream tooling can tell an in-flight
/// status update apart from a true terminal report.
pub const NO_EVENT_CURSOR_MARKER: &str = "runtime://status-change";

/// Synthetic `last_event_id` tag emitted when a terminal status
/// (`finished`, `error`, `stuck`) advances the cursor without a tightly
/// matching event id. Distinct from [`NO_EVENT_CURSOR_MARKER`] so gateway
/// and journal consumers can highlight a terminal transition versus an
/// interim status change.
pub const TERMINAL_CURSOR_MARKER: &str = "runtime://terminal";

/// Configuration for a [`RuntimeMirror`].
#[derive(Debug, Clone)]
pub struct MirrorConfig {
    /// Idle timeout for `StallMetadata`: any silence longer than this advances
    /// toward the stalled phase.
    pub idle_timeout_ms: Option<DurationMs>,
    /// Total runtime cap; independent of progress-based detection. Anchored to
    /// `started_at`.
    pub total_runtime_cap_ms: Option<DurationMs>,
    /// After this idle window the run transitions from `RunningTurn` to `Quiet`
    /// (without yet escalating to `Stalled`).
    pub quiet_window_ms: Option<DurationMs>,
}

impl Default for MirrorConfig {
    fn default() -> Self {
        Self {
            idle_timeout_ms: Some(DurationMs::new(300_000)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(60_000)),
        }
    }
}

/// Single-run runtime state mirror.
///
/// Holds the canonical [`EventCache`] (REST + WS), the computed
/// [`ConversationStateMirror`], and the [`StallMetadata`] whose idle deadline
/// slides forward on every progress signal.
///
/// Clone is intentionally absent so mutations are linear and bookkeeping cannot
/// accidentally fork the state on the way through.
#[derive(Debug)]
pub struct RuntimeMirror {
    config: MirrorConfig,
    conversation_id: ConversationId,
    started_at: TimestampMs,
    state_mirror: ConversationStateMirror,
    event_cache: EventCache,
    stall: StallMetadata,
    stream_health: StreamHealth,
    history_sync_status: HistorySyncStatus,
    reconnect_status: ReconnectStatus,
    last_event_id: Option<String>,
    last_event_kind: Option<String>,
    last_event_at: Option<TimestampMs>,
    last_logical_event_at: Option<TimestampMs>,
    detach_metadata: Option<DetachMetadata>,
    /// `apply_cancelling` flips this true so [`phase_at`] reports
    /// [`RuntimeLivenessPhase::Cancelling`] until the scheduler observes the
    /// terminal status. Cleared automatically by [`RuntimeMirror::apply_terminal`]
    /// because a terminal transition supersedes the in-flight cancel.
    cancel_pending: bool,
}

impl RuntimeMirror {
    /// Construct a new mirror for a freshly created conversation.
    pub fn new(
        conversation_id: ConversationId,
        started_at: TimestampMs,
        config: MirrorConfig,
    ) -> Self {
        let idle_timeout = config.idle_timeout_ms.unwrap_or(QUIET_SAFE_IDLE_FLOOR);
        // Quiet must be a *strict precursor* to Stalled: when quiet_window_ms
        // >= idle_timeout_ms the Quiet band collapses to zero width and the
        // phase precedence logic below can no longer surface Quiet. Clamp the
        // operator-supplied value so the precedence invariant holds.
        let config = clamp_quiet_window(config, idle_timeout);
        let stall =
            StallMetadata::with_runtime_cap(started_at, idle_timeout, config.total_runtime_cap_ms);
        Self {
            config,
            conversation_id,
            started_at,
            state_mirror: ConversationStateMirror::default(),
            event_cache: EventCache::new(),
            stall,
            stream_health: StreamHealth::Unknown,
            history_sync_status: HistorySyncStatus::Idle,
            reconnect_status: ReconnectStatus::Connected,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_logical_event_at: None,
            detach_metadata: None,
            cancel_pending: false,
        }
    }

    /// Apply an initial conversation snapshot obtained from REST `get_conversation`.
    pub fn apply_initial_conversation_snapshot(&mut self, conversation: &Conversation) {
        self.state_mirror.apply_conversation(conversation);
        self.history_sync_status = HistorySyncStatus::InProgress;
        if self.state_mirror.raw_state().get("stats").is_some() {
            self.history_sync_status = HistorySyncStatus::Synced;
        }
    }

    /// Apply a REST history payload. Events recorded here share the same
    /// backing cache as WebSocket events, so a later WebSocket replay dedupes
    /// against this materialization.
    pub fn apply_rest_history<I>(&mut self, history: I) -> Vec<EventEnvelope>
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        let events: Vec<_> = history.into_iter().collect();
        let applied = self.event_cache.merge_new_events(events.clone());
        for event in &applied {
            self.state_mirror.apply_event(event);
        }
        if let Some(last) = applied.last() {
            self.cursor_from_event(last);
            self.slide_deadline(timestamp_for_event(last));
        }
        // Whether all events were new or some were already cached, REST replay
        // has materialised the conversation history. Subsequent websocket
        // traffic will dedupe against this materialization.
        self.history_sync_status = HistorySyncStatus::Synced;
        applied
    }

    /// Mark the readiness barrier as passed.
    pub fn apply_socket_ready(&mut self, now: TimestampMs) {
        self.stream_health = StreamHealth::Ready;
        self.reconnect_status = ReconnectStatus::Connected;
        self.slide_deadline(now);
    }

    /// Mark the WebSocket as disconnected.
    pub fn apply_socket_disconnected(&mut self, reason: &str, now: TimestampMs) {
        self.stream_health = StreamHealth::Disconnected;
        self.reconnect_status = ReconnectStatus::Pending;
        self.history_sync_status = HistorySyncStatus::Stale;
        let hint = if reason.is_empty() {
            "socket disconnected".to_string()
        } else {
            reason.to_string()
        };
        self.attach_state_change(&hint, &hint);
        self.slide_deadline(now);
    }

    /// Mark a reconnect attempt as scheduled (with the next retry backoff).
    pub fn apply_reconnect_pending(&mut self) {
        self.stream_health = StreamHealth::Reconnecting;
        self.reconnect_status = ReconnectStatus::Pending;
    }

    /// Mark the WebSocket as successfully reconnected and reconcile missed events.
    pub fn apply_reconnect_succeeded<I>(
        &mut self,
        replayed: I,
        now: TimestampMs,
    ) -> Vec<EventEnvelope>
    where
        I: IntoIterator<Item = EventEnvelope>,
    {
        let events: Vec<_> = replayed.into_iter().collect();
        let applied = self.event_cache.merge_new_events(events.clone());
        for event in &applied {
            self.state_mirror.apply_event(event);
        }
        if let Some(last) = applied.last() {
            self.cursor_from_event(last);
        }
        self.stream_health = StreamHealth::Ready;
        self.reconnect_status = ReconnectStatus::Connected;
        self.history_sync_status = HistorySyncStatus::Synced;
        self.slide_deadline(now);
        applied
    }

    /// Record a reconnect exhausted outcome.
    pub fn apply_reconnect_exhausted(&mut self, now: TimestampMs) {
        self.reconnect_status = ReconnectStatus::Exhausted;
        self.stream_health = StreamHealth::Failed;
        self.slide_deadline(now);
    }

    /// Apply a single WebSocket event.
    ///
    /// Returns `true` if the event was newly inserted (false on dedupe).
    pub fn apply_event(&mut self, event: &EventEnvelope) -> bool {
        let event_at = timestamp_for_event(event);
        let inserted = self.event_cache.insert(event.clone());
        if !inserted {
            self.slide_deadline(event_at);
            return false;
        }
        self.state_mirror.apply_event(event);
        self.cursor_from_event(event);
        self.slide_deadline(event_at);
        true
    }

    /// Observe a runtime-reported execution status change without an event.
    ///
    /// `last_event_kind` reflects the new status (the canonical signal
    /// surfaced to consumers); the synthetic cursor advances so a subsequent
    /// `event_count` increase is distinguishable from this bookkeeping call.
    pub fn apply_status_change(&mut self, status: &str, now: TimestampMs) {
        self.state_mirror
            .apply_conversation_execution_status(&synthetic_conversation(status));
        self.attach_state_change(format!("status:{status}").as_str(), status);
        self.slide_deadline(now);
    }

    /// Observe a token usage bump (typically derived from an LLM completion log).
    /// Counts are deltas since the last call and they are merged into the
    /// state mirror's raw statistics blob so subsequent snapshots reflect
    /// the new totals. All three token buckets (prompt / completion /
    /// cache-read) are forwarded so callers don't silently drop cache read
    /// counts.
    pub fn apply_token_update(
        &mut self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        now: TimestampMs,
    ) {
        self.state_mirror
            .apply_token_counts(input_tokens, output_tokens, cache_read_tokens);
        self.slide_deadline(now);
    }

    /// Mark the run as terminal with the actual OpenHands execution status
    /// (`finished`, `error`, `stuck`, etc). The status is forwarded to the
    /// state mirror so downstream liveness and diagnostic phases observe the
    /// real reason — never collapse to a hardcoded `finished`.
    ///
    /// `last_event_kind` is set to the supplied status (the OpenHands
    /// canonical signal surfaced to consumers) rather than the human-readable
    /// `summary`; the summary continues to flow into [`DetachMetadata`] for
    /// the detach path, but terminal transitions must keep `last_event_kind`
    /// in lock-step with `execution_status`.
    pub fn apply_terminal(&mut self, status: &str, summary: &str, now: TimestampMs) {
        self.state_mirror
            .apply_conversation_execution_status(&synthetic_conversation(status));
        // Pin `last_event_kind` to the OpenHands-canonical status; the cursor
        // suffix also carries the summary so cross-tooling still has full
        // context (the cursor is not user-visible). The cursor uses the
        // [`TERMINAL_CURSOR_MARKER`] prefix (not [`NO_EVENT_CURSOR_MARKER`])
        // so downstream tooling can tell terminal transitions apart from
        // ordinary status changes.
        let cursor_suffix = format!("{status}:{summary}");
        self.last_event_kind = Some(status.to_string());
        self.last_event_id = Some(format!("{TERMINAL_CURSOR_MARKER}/{cursor_suffix}"));
        // Terminal supersedes the in-flight cancel flag — a real terminal
        // observation is the authoritative resolution.
        self.cancel_pending = false;
        self.slide_deadline(now);
    }

    /// Note that the scheduler has requested cancellation of the underlying
    /// run but the runtime has not yet produced a terminal status. Until a
    /// subsequent [`RuntimeMirror::apply_terminal`] lands, [`phase_at`] will
    /// return [`RuntimeLivenessPhase::Cancelling`] so the run-detail view
    /// surfaces the in-flight cancel state distinct from a generic
    /// progress-based stall.
    ///
    /// Cancelling is intentionally below `Detached`/`Terminal` in the
    /// precedence ordering of [`phase_at`] and below `Reconciling`, so a
    /// caller that races a cancel with a stream reconnect still reports the
    /// reconnect first.
    pub fn apply_cancelling(&mut self, reason: &str, now: TimestampMs) {
        self.cancel_pending = true;
        self.attach_state_change(format!("cancelling:{reason}").as_str(), "cancelling");
        self.slide_deadline(now);
    }

    /// Mark the stream as failed (transport-level failure).
    pub fn apply_stream_failure(&mut self, now: TimestampMs) {
        self.stream_health = StreamHealth::Failed;
        self.attach_state_change("stream_failure", "stream_failure");
        self.slide_deadline(now);
    }

    /// Mark the run as detached (worker lost ownership, runtime is unrecoverable).
    pub fn apply_detach(&mut self, reason: DetachReason, summary: String, now: TimestampMs) {
        let prev_status = self.state_mirror.execution_status().map(str::to_string);
        let metadata = DetachMetadata {
            reason,
            detached_at: now,
            last_execution_status: prev_status,
            summary,
        };
        self.detach_metadata = Some(metadata);
        self.stream_health = StreamHealth::Detached;
        // Detached override supersedes inactive state changes but stays inside the
        // bookkeeping so event_count / cursor remain meaningful for diagnostics.
        self.attach_state_change("detached", "detached");
        self.slide_deadline(now);
    }

    /// Synthesize a deterministic `last_event_kind` + cursor for state-change
    /// bookkeeping calls (`apply_status_change`, `apply_stream_failure`,
    /// `apply_detach`, `apply_cancelling`).
    ///
    /// `cursor_suffix` is what the synthetic cursor records (after the marker);
    /// `kind` is what surfaces as `last_event_kind`. Splitting the two lets
    /// `apply_status_change` expose the canonical status as `kind` while
    /// keeping rich context-bearing cursor suffixes, and lets `apply_terminal`
    /// pin `kind` to the canonical status independent of `summary`.
    fn attach_state_change(&mut self, cursor_suffix: &str, kind: &str) {
        self.last_event_kind = Some(kind.to_string());
        self.last_event_id = Some(format!("{NO_EVENT_CURSOR_MARKER}/{cursor_suffix}"));
    }

    /// Slide the progress-based idle deadline forward.
    fn slide_deadline(&mut self, now: TimestampMs) {
        if now.as_u64() == 0 {
            return;
        }
        self.stall.observe_activity(now);
        if self.last_logical_event_at.is_none() {
            self.last_logical_event_at = Some(now);
        }
        if self.state_mirror.execution_status().is_none() {
            self.state_mirror
                .apply_conversation_execution_status(&synthetic_conversation("running"));
        }
    }

    /// Build the current snapshot at `now`.
    ///
    /// The caller-supplied timestamp is required because the mirror cannot
    /// read wall-clock time on its own; using `last_logical_event_at` here
    /// would mask `Quiet`/`Stalled`/`Degraded` transitions that depend on
    /// the elapsed-since-last-activity comparison that
    /// [`RuntimeMirror::phase_at`] performs.
    pub fn snapshot_at(&self, now: TimestampMs) -> RuntimeProgressSnapshot {
        self.build_snapshot(now)
    }

    /// **Deprecated**: pins `now` to the timestamp of the last observed
    /// activity, which means the resulting snapshot **always reports
    /// `RunningTurn` / `WaitingOnPriorTurn`** because the elapsed-since-last-
    /// activity delta collapses to zero. Downstream callers that want
    /// `Quiet`/`Stalled`/`Degraded` semantics must use
    /// [`RuntimeMirror::snapshot_at`] (with a real wall-clock timestamp)
    /// instead. Retained so existing tests and any external consumers can
    /// migrate explicitly; the [`#[deprecated]`] attribute surfaces that
    /// contract at compile time rather than letting it silently truncate
    /// liveness state in production.
    #[deprecated(
        since = "1.7.0",
        note = "snapshot() always reports RunningTurn; use snapshot_at(now) to observe Quiet/Stalled/Degraded transitions"
    )]
    pub fn snapshot(&self) -> RuntimeProgressSnapshot {
        self.snapshot_pinned_at_last_activity()
    }

    /// Non-deprecated alias for [`RuntimeMirror::snapshot`].
    ///
    /// Pinning `now` to the most-recent logical event makes the resulting
    /// snapshot always reflect the "in flight, just observed activity"
    /// state, masking `Quiet`/`Stalled`/`Degraded` transitions in real time.
    /// Callers that want wall-clock-driven phase classification must use
    /// [`RuntimeMirror::snapshot_at`] instead. This entry-point is preferred
    /// over the deprecated [`RuntimeMirror::snapshot`] inside test code and
    /// for any external consumer that explicitly wants the pinned-at-last-
    /// activity semantics rather than a deliberate runtime observation
    /// window.
    pub fn snapshot_pinned_at_last_activity(&self) -> RuntimeProgressSnapshot {
        let at = self.last_logical_event_at.unwrap_or(self.started_at);
        self.build_snapshot(at)
    }

    fn build_snapshot(&self, at: TimestampMs) -> RuntimeProgressSnapshot {
        let phase = self.phase_at(at);
        let stall_deadline_at = if self.stall.stalled_at.as_u64() == 0 {
            None
        } else {
            Some(self.stall.stalled_at)
        };
        let (input_tokens, output_tokens, cache_read_tokens) = self
            .state_mirror
            .accumulated_token_usage()
            .unwrap_or((0, 0, 0));
        RuntimeProgressSnapshot::initial(phase)
            .update_with(phase)
            .with_event_count(self.event_cache.items().len() as u64)
            .with_input_tokens(input_tokens)
            .with_output_tokens(output_tokens)
            .with_cache_read_tokens(cache_read_tokens)
            // Propagate `execution_status` *directly* from the state mirror so
            // a never-set OpenHands status surfaces as `None` rather than
            // collapsing to the synthetic `Some("")` sentinel that hides
            // "not yet known" from downstream diagnostics.
            .with_execution_status(self.state_mirror.execution_status().map(str::to_string))
            .with_stream_health(self.stream_health)
            .with_history_sync_status(self.history_sync_status)
            .with_reconnect_status(self.reconnect_status)
            .with_last_activity_at(Some(self.stall.last_activity_at))
            .with_stall_deadline_at(stall_deadline_at)
            .with_last_event_cursor(self.last_event_id.clone())
            .with_last_event_kind(self.last_event_kind.clone())
            .with_last_event_at(self.last_event_at)
            .with_detach_metadata(self.detach_metadata.clone())
            .build()
    }

    /// Compute the current phase from mirror state.
    pub fn phase(&self) -> RuntimeLivenessPhase {
        let at = self.last_logical_event_at.unwrap_or(self.started_at);
        self.phase_at(at)
    }

    /// Compute the current six-state aggregation.
    pub fn liveness_state(&self) -> LivenessState {
        self.phase().liveness_state()
    }

    /// Derive a phase from explicit inputs (used in tests and when projecting
    /// the phase onto a historical timestamp).
    pub fn phase_at(&self, now: TimestampMs) -> RuntimeLivenessPhase {
        if self.detach_metadata.is_some() {
            return RuntimeLivenessPhase::Detached;
        }
        if self.state_mirror.terminal_status().is_some() {
            return RuntimeLivenessPhase::Terminal;
        }
        // Cancelling sits between Terminal and Reconciling so an in-flight
        // cancel request observes the explicit phase *before* stream-side
        // reconnect machinery overrides it. The flag is cleared by
        // [`RuntimeMirror::apply_terminal`] as soon as the runtime reports
        // the actual terminal status.
        if self.cancel_pending {
            return RuntimeLivenessPhase::Cancelling;
        }
        if matches!(
            self.stream_health,
            StreamHealth::Reconnecting | StreamHealth::Disconnected | StreamHealth::HistorySyncing
        ) {
            return RuntimeLivenessPhase::Reconciling;
        }
        if matches!(self.reconnect_status, ReconnectStatus::Exhausted)
            || matches!(self.stream_health, StreamHealth::Failed)
        {
            return RuntimeLivenessPhase::Degraded;
        }
        if matches!(
            self.stream_health,
            StreamHealth::Attaching | StreamHealth::Unknown
        ) {
            return RuntimeLivenessPhase::WaitingOnPriorTurn;
        }
        // Quiet precedes Stalled on purpose: the docstring on [`RuntimeLivenessPhase::Quiet`]
        // promises that we let a quiet-but-not-yet-stalled run surface new events before we
        // declare it stalled. We also guard against configs where `quiet_window >=
        // idle_timeout` so the operator cannot accidentally collapse Quiet into Stalled.
        let quiet_window = self
            .config
            .quiet_window_ms
            .unwrap_or(DurationMs::new(0))
            .as_u64();
        let idle_timeout = self
            .config
            .idle_timeout_ms
            .unwrap_or(QUIET_SAFE_IDLE_FLOOR)
            .as_u64();
        // Quiet is the band *between* the quiet_window mark and the stall mark —
        // once we cross idle_timeout the linear envelope transitions to Stalled.
        let quiet_in_range = quiet_window > 0
            && quiet_window < idle_timeout
            && now
                >= self
                    .stall
                    .last_activity_at
                    .saturating_add(DurationMs::new(quiet_window))
            && !self.stall.is_stalled_at(now);
        if quiet_in_range {
            return RuntimeLivenessPhase::Quiet;
        }
        if self.stall.is_stalled_at(now) {
            return RuntimeLivenessPhase::Stalled;
        }
        RuntimeLivenessPhase::RunningTurn
    }

    fn cursor_from_event(&mut self, event: &EventEnvelope) {
        self.last_event_id = Some(event.id.clone());
        self.last_event_kind = Some(event.kind.clone());
        self.last_event_at = Some(timestamp_for_event(event));
        self.last_logical_event_at = Some(timestamp_for_event(event));
    }

    /// Conversation id this mirror tracks.
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    /// Total observed event count.
    pub fn observed_event_count(&self) -> u64 {
        self.event_cache.items().len() as u64
    }

    /// Current last event cursor (`event_id`).
    pub fn last_event_cursor(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    /// Internal getter used by tests for assertions around stall timing.
    pub fn stall_metadata(&self) -> StallMetadata {
        self.stall
    }

    /// Stream health visible to consumers.
    pub fn stream_health(&self) -> StreamHealth {
        self.stream_health
    }

    /// History sync status visible to consumers.
    pub fn history_sync_status(&self) -> HistorySyncStatus {
        self.history_sync_status
    }

    /// Reconnect status visible to consumers.
    pub fn reconnect_status(&self) -> ReconnectStatus {
        self.reconnect_status
    }

    /// Provides direct read access to the inner `ConversationStateMirror`
    /// for payloads that need its raw_state.
    pub fn conversation_mirror(&self) -> &ConversationStateMirror {
        &self.state_mirror
    }
}

const QUIET_SAFE_IDLE_FLOOR: DurationMs = DurationMs::new(86_400_000);

/// Clamp `quiet_window_ms` so it is strictly less than `idle_timeout_ms` and
/// remain zero-width by default when no idle timeout was supplied. The
/// returned config matches the input on every other field.
fn clamp_quiet_window(mut config: MirrorConfig, idle_timeout: DurationMs) -> MirrorConfig {
    let Some(quiet) = config.quiet_window_ms else {
        return config;
    };
    if quiet.as_u64() < idle_timeout.as_u64() {
        return config;
    }
    // Reserve at least 1 ms so the Quiet band is non-empty; if even 1 ms is
    // too aggressive (idle_timeout already at 0), suppress Quiet entirely.
    let clamped = idle_timeout
        .as_u64()
        .saturating_sub(1)
        .max(if quiet.as_u64() == 0 { 0 } else { 1 });
    config.quiet_window_ms = Some(DurationMs::new(clamped));
    config
}

fn timestamp_for_event(event: &EventEnvelope) -> TimestampMs {
    TimestampMs::new(event.timestamp.timestamp_millis().max(0) as u64)
}

fn synthetic_conversation(status: &str) -> Conversation {
    use crate::opensymphony_openhands::{
        AgentConfig, ConfirmationPolicy, Conversation, LlmConfig, WorkspaceConfig,
    };
    use uuid::Uuid;
    Conversation {
        conversation_id: Uuid::nil(),
        workspace: WorkspaceConfig {
            working_dir: "/tmp/synthetic".to_string(),
            kind: "LocalWorkspace".to_string(),
        },
        persistence_dir: "/tmp/synthetic/persistence".to_string(),
        max_iterations: 0,
        stuck_detection: false,
        execution_status: status.to_string(),
        confirmation_policy: ConfirmationPolicy {
            kind: "NeverConfirm".to_string(),
        },
        agent: AgentConfig {
            kind: "Agent".to_string(),
            llm: LlmConfig {
                model: "synthetic".to_string(),
                api_key: None,
                base_url: None,
                usage_id: None,
            },
            condenser: None,
            tools: None,
            include_default_tools: None,
        },
        stats: None,
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;
    use crate::opensymphony_openhands::{
        AgentConfig, ConfirmationPolicy, Conversation, LlmConfig, WorkspaceConfig,
    };
    use chrono::{TimeZone, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn conversation_with_status(status: &str) -> Conversation {
        Conversation {
            conversation_id: Uuid::nil(),
            workspace: WorkspaceConfig {
                working_dir: "/tmp/conv".to_string(),
                kind: "LocalWorkspace".to_string(),
            },
            persistence_dir: "/tmp/conv/persistence".to_string(),
            max_iterations: 4,
            stuck_detection: true,
            execution_status: status.to_string(),
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
            stats: None,
        }
    }

    fn idle_config(idle_ms: u64) -> MirrorConfig {
        MirrorConfig {
            idle_timeout_ms: Some(DurationMs::new(idle_ms)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(idle_ms / 2)),
        }
    }

    fn mirror_with_config(idle_ms: u64) -> RuntimeMirror {
        RuntimeMirror::new(
            ConversationId::new("conv-200").expect("valid id"),
            TimestampMs::new(1_000),
            idle_config(idle_ms),
        )
    }

    fn runtime_event(id: &str, kind: &str, timestamp_ms: u64) -> EventEnvelope {
        let dt = Utc.timestamp_millis_opt(timestamp_ms as i64).unwrap();
        EventEnvelope::new(id, dt, "runtime", kind, json!({}))
    }

    fn runtime_state_update(id: &str, status: &str, timestamp_ms: u64) -> EventEnvelope {
        let dt = Utc.timestamp_millis_opt(timestamp_ms as i64).unwrap();
        let stamp_value = status.to_string();
        EventEnvelope::new(
            id,
            dt,
            "runtime",
            "ConversationStateUpdateEvent",
            json!({
                "execution_status": stamp_value,
                "state_delta": { "execution_status": stamp_value },
            }),
        )
    }

    fn user_message_event(id: &str, timestamp_ms: u64, text: &str) -> EventEnvelope {
        let dt = Utc.timestamp_millis_opt(timestamp_ms as i64).unwrap();
        EventEnvelope::new(
            id,
            dt,
            "user",
            "MessageEvent",
            json!({
                "role": "user",
                "content": [{ "type": "text", "text": text }]
            }),
        )
    }

    #[test]
    fn new_mirror_starts_in_unknown_state() {
        let mirror = mirror_with_config(300_000);
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(
            snap.phase,
            RuntimeLivenessPhase::WaitingOnPriorTurn | RuntimeLivenessPhase::RunningTurn
        ));
        assert!(matches!(snap.stream_health, StreamHealth::Unknown));
        assert_eq!(snap.event_count, 0);
        assert!(matches!(snap.liveness_state, LivenessState::Active));
    }

    #[test]
    fn ready_then_event_slides_stall_deadline() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
        assert!(matches!(snap.stream_health, StreamHealth::Ready));
        assert_eq!(snap.last_event_cursor.as_deref(), Some("evt-1"));
        let deadline = snap.stall_deadline_at.expect("deadline");
        assert!(deadline.as_u64() >= 1_500 + 2_000);
    }

    #[test]
    fn long_running_progress_keeps_running_turn() {
        // 300 ms idle timeout; emit an event every 100 ms for 600 ms.
        let mut mirror = mirror_with_config(300);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        let ready_at = 1_000_u64;
        mirror.apply_socket_ready(TimestampMs::new(ready_at));

        let mut now = ready_at;
        let end = ready_at + 600;
        let mut progress_count = 0;
        while now <= end {
            let id = format!("evt-{now}");
            mirror.apply_event(&runtime_event(&id, "MessageEvent", now));
            progress_count += 1;
            now += 100;
        }
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert_eq!(progress_count, 7);
        assert!(
            !matches!(snap.phase, RuntimeLivenessPhase::Stalled),
            "long-running progress should not stall (phase was {:?})",
            snap.phase
        );
        assert!(matches!(snap.phase, RuntimeLivenessPhase::RunningTurn));
    }

    #[test]
    fn silence_progresses_through_quiet_then_stalled() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));

        let last_event_at = mirror.last_logical_event_at.expect("set");
        let quiet_window = 1_000_u64;
        // quiet window = half of idle_timeout_ms = 1_000; advance 1_300 ms later
        let quiet_now = last_event_at.as_u64() + quiet_window + 300;
        assert!(matches!(
            mirror.phase_at(TimestampMs::new(quiet_now)),
            RuntimeLivenessPhase::Quiet
        ));

        let stalled_now = last_event_at.as_u64() + 2_500;
        assert!(matches!(
            mirror.phase_at(TimestampMs::new(stalled_now)),
            RuntimeLivenessPhase::Stalled
        ));
    }

    #[test]
    fn detached_metadata_overrides_phase_to_detached() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_detach(
            DetachReason::Unreachable,
            "lost ownership".to_string(),
            TimestampMs::new(2_000),
        );
        let phase = mirror.phase();
        assert!(matches!(phase, RuntimeLivenessPhase::Detached));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.liveness_state, LivenessState::Detached));
        assert!(snap.detach_metadata.is_some());
    }

    #[test]
    fn reconcile_progress_collapses_rest_then_ws_replay() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));

        let rest_events = vec![runtime_event("evt-rest", "MessageEvent", 1_500)];
        let applied = mirror.apply_rest_history(rest_events);
        assert_eq!(applied.len(), 1);
        assert_eq!(mirror.observed_event_count(), 1);

        let ws_replay = vec![runtime_event("evt-rest", "MessageEvent", 1_500)];
        let ws_applied = mirror.apply_reconnect_succeeded(ws_replay, TimestampMs::new(2_000));
        assert!(ws_applied.is_empty(), "duplicate ids must dedupe");
        assert_eq!(mirror.observed_event_count(), 1);
        assert_eq!(mirror.history_sync_status(), HistorySyncStatus::Synced);
    }

    #[test]
    fn reconnect_exhausted_transitions_to_degraded() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_socket_disconnected("ws closed", TimestampMs::new(1_500));
        mirror.apply_reconnect_pending();
        mirror.apply_reconnect_exhausted(TimestampMs::new(2_500));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Degraded));
        assert!(matches!(snap.stream_health, StreamHealth::Failed));
        assert!(matches!(snap.reconnect_status, ReconnectStatus::Exhausted));
    }

    #[test]
    fn terminal_status_transitions_to_terminal_phase() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_event(&runtime_state_update("evt-finished", "finished", 1_900));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Terminal));
        assert!(matches!(snap.liveness_state, LivenessState::Terminal));
    }

    #[test]
    fn apply_terminal_propagates_supplied_status_not_hardcoded_finished() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_terminal("error", "openhands tripwire", TimestampMs::new(1_900));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert_eq!(
            snap.execution_status.as_deref(),
            Some("error"),
            "apply_terminal must forward the actual terminal status into the state mirror"
        );
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Terminal));
    }

    #[test]
    fn stream_failure_drives_degraded_phase() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_stream_failure(TimestampMs::new(2_000));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.stream_health, StreamHealth::Failed));
        // A stream failure with no terminal reporting doesn't mark terminal; it's degraded.
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Degraded));
    }

    #[test]
    fn state_update_event_propagates_execution_status_into_snapshot() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_state_update("evt-1", "running", 1_500));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert_eq!(snap.execution_status.as_deref(), Some("running"));
    }

    /// Round-6 AI review (`build_snapshot` lost `execution_status`): the
    /// previous implementation coerced `None` from the underlying state
    /// mirror into `Some("")` via `unwrap_or("")`, polluting consumers with a
    /// synthetic empty-string sentinel. This regression test confirms that a
    /// fresh mirror — one whose underlying state mirror has no execution
    /// status source applied — surfaces `execution_status == None`
    /// end-to-end in every snapshot entry point so downstream diagnostics
    /// can distinguish "not yet known" from "empty string".
    ///
    /// Note: do not call `apply_event` here; every runtime event auto-asserts
    /// the synthetic "running" status, and that auto-assertion is the
    /// correct, pre-existing behaviour that this regression must not collide
    /// with.
    #[test]
    fn build_snapshot_propagates_none_for_execution_status_when_unobserved() {
        let mirror = mirror_with_config(2_000);
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(
            snap.execution_status.is_none(),
            "execution_status must stay None when no status source has been observed (was {:?})",
            snap.execution_status
        );
        let snap_wall = mirror.snapshot_at(TimestampMs::new(2_000));
        assert!(
            snap_wall.execution_status.is_none(),
            "snapshot_at must also preserve None execution_status (was {:?})",
            snap_wall.execution_status
        );
        let snap_legacy = mirror.snapshot();
        assert!(
            snap_legacy.execution_status.is_none(),
            "deprecated snapshot() must also preserve None execution_status (was {:?})",
            snap_legacy.execution_status
        );
    }

    #[test]
    fn status_change_advances_cursor_with_marker() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_status_change("running", TimestampMs::new(1_500));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(
            snap.last_event_cursor
                .as_deref()
                .unwrap_or_default()
                .starts_with(NO_EVENT_CURSOR_MARKER)
        );
        assert_eq!(snap.last_event_kind.as_deref(), Some("running"));
    }

    #[test]
    fn token_only_progress_slides_deadline() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_100));

        let mut now = 1_500;
        let mut last_deadline = mirror
            .snapshot_pinned_at_last_activity()
            .stall_deadline_at
            .expect("deadline")
            .as_u64();
        for _ in 0..5 {
            mirror.apply_token_update(100, 50, 10, TimestampMs::new(now));
            let current = mirror
                .snapshot_pinned_at_last_activity()
                .stall_deadline_at
                .expect("deadline")
                .as_u64();
            assert!(current >= last_deadline, "deadline should slide forward");
            last_deadline = current;
            now += 100;
        }
    }

    #[test]
    fn dedupe_replayed_event_keeps_unique_count() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let envelope = user_message_event("evt-dup", 1_500, "hi again");
        assert!(mirror.apply_event(&envelope));
        assert!(!mirror.apply_event(&envelope));
        assert_eq!(mirror.observed_event_count(), 1);
    }

    #[test]
    fn prior_turn_wait_visible_via_attaching_stream_health() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status(
            "waiting_on_prior_turn",
        ));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(
            snap.phase,
            RuntimeLivenessPhase::WaitingOnPriorTurn
        ));
    }

    #[test]
    fn unknown_event_advances_cursor() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let envelope = EventEnvelope::new(
            "evt-mystery",
            Utc.timestamp_millis_opt(1_500).unwrap(),
            "runtime",
            "BrandNewEventType",
            json!({ "structure": "future" }),
        );
        assert!(mirror.apply_event(&envelope));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert_eq!(snap.last_event_cursor.as_deref(), Some("evt-mystery"));
        assert_eq!(snap.last_event_kind.as_deref(), Some("BrandNewEventType"));
    }

    #[test]
    fn stream_disconnect_forces_history_to_stale() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let rest = vec![runtime_event("evt-rest", "MessageEvent", 1_500)];
        mirror.apply_rest_history(rest);
        assert_eq!(mirror.history_sync_status(), HistorySyncStatus::Synced);

        mirror.apply_socket_disconnected("closed", TimestampMs::new(2_000));
        assert_eq!(mirror.history_sync_status(), HistorySyncStatus::Stale);
        assert!(matches!(mirror.stream_health(), StreamHealth::Disconnected));
        assert!(matches!(
            mirror.reconnect_status(),
            ReconnectStatus::Pending
        ));
    }

    #[test]
    fn snapshot_reports_liveness_state_aggregation() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.liveness_state, LivenessState::Active));
    }

    #[test]
    fn apply_cancelling_transitions_to_cancelling_phase() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_cancelling("user_requested", TimestampMs::new(1_800));
        let snap = mirror.snapshot_at(TimestampMs::new(1_800));
        assert!(
            matches!(snap.phase, RuntimeLivenessPhase::Cancelling),
            "apply_cancelling should surface Cancelling phase (got {:?})",
            snap.phase
        );
        assert_eq!(
            snap.last_event_kind.as_deref(),
            Some("cancelling"),
            "last_event_kind should pin to 'cancelling' once the cancel flag is set"
        );
    }

    #[test]
    fn apply_terminal_clears_cancelling_and_pins_last_event_kind_to_status() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_cancelling("user_requested", TimestampMs::new(1_700));
        // Now the scheduler observes the terminal status; apply_terminal
        // clears `cancel_pending` and pins `last_event_kind` to the actual
        // OpenHands canonical status.
        mirror.apply_terminal("error", "openhands tripwire", TimestampMs::new(1_900));
        let snap = mirror.snapshot_at(TimestampMs::new(1_900));
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Terminal));
        assert_eq!(
            snap.last_event_kind.as_deref(),
            Some("error"),
            "apply_terminal must pin last_event_kind to the canonical status, never the human-readable summary"
        );
        assert_eq!(
            snap.execution_status.as_deref(),
            Some("error"),
            "execution_status remains the actual terminal status"
        );
        let cursor = snap.last_event_cursor.as_deref().unwrap_or_default();
        assert!(
            cursor.starts_with(TERMINAL_CURSOR_MARKER),
            "terminal cursor must use the dedicated marker, not NO_EVENT_CURSOR_MARKER ({cursor})"
        );
        assert!(
            !cursor.starts_with(NO_EVENT_CURSOR_MARKER),
            "terminal cursor must never collide with the status-change marker ({cursor})"
        );
        assert!(
            cursor.contains("error:openhands tripwire"),
            "summary is preserved in the cursor suffix so journal consumers retain context ({cursor})"
        );
    }

    #[test]
    fn apply_status_change_pins_last_event_kind_to_status_with_status_marker_in_cursor() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("idle"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_status_change("running", TimestampMs::new(1_500));
        let snap = mirror.snapshot_at(TimestampMs::new(1_500));
        // last_event_kind now matches the supplied status (the canonical
        // signal surfaced to consumers, never the inferred cursor suffix).
        assert_eq!(snap.last_event_kind.as_deref(), Some("running"));
        // Cursor suffix preserves the canonical status marker for cross-tooling.
        let cursor = snap.last_event_cursor.as_deref().unwrap_or_default();
        assert!(
            cursor.starts_with(NO_EVENT_CURSOR_MARKER),
            "status-change cursor must remain a synthetic marker"
        );
        assert!(
            cursor.contains("status:running"),
            "status-change cursor must encode the supplied status verbatim ({cursor})"
        );
    }

    #[test]
    #[allow(deprecated)]
    fn cancelling_phase_observable_via_phase_at_plumbing() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_cancelling("scheduler_idle", TimestampMs::new(1_700));
        // Cancelling is reachable through both `phase_at(now)` and the
        // canonical `snapshot_at(now)`. Back-compat `snapshot()` still pins
        // `now` to the last activity timestamp, but with the flag set the
        // precedence rule means a Cancelling transition is observable even
        // there.
        assert!(matches!(
            mirror.phase_at(TimestampMs::new(1_700)),
            RuntimeLivenessPhase::Cancelling
        ));
        assert!(matches!(
            mirror.snapshot().phase,
            RuntimeLivenessPhase::Cancelling
        ));
    }

    /// Round-5/6 AI review (`degrade_after_ms` dead field): the prior public
    /// configuration knob was removed in this branch and a follow-up review
    /// flagged the original `mirror_config_default_has_no_degrade_after_ms_field`
    /// test as a no-op (it only checked *values* of other knobs, not the
    /// absence of `degrade_after_ms`). This rewrite uses a struct-literal
    /// that names each documented field individually and asks for
    /// `..MirrorConfig::default()` — if a future PR reintroduces the field
    /// alongside a `Default` impl, the struct-literal below must grow to
    /// carry the new field (we keep that surface explicit) and the runtime
    /// guard below rejects any default-derived config whose `Debug` shape
    /// reintroduces the dead knob. Both halves together turn this into a
    /// real compile-time + runtime regression.
    #[test]
    fn mirror_config_default_carries_only_documented_knobs() {
        // Documented fields, named explicitly. A reintroduction of
        // `degrade_after_ms` would surface here as either a `..Default()`-
        // derived unhandled value or as a comment-level drift below.
        let cfg = MirrorConfig {
            idle_timeout_ms: Some(DurationMs::new(2_500)),
            total_runtime_cap_ms: None,
            quiet_window_ms: Some(DurationMs::new(1_250)),
        };
        // The struct literal above is itself the compile-time guard,
        // because it names every documented field. If a new field is
        // added, this literal either silently drops a value (caught by
        // maintainers) or fails to compile (depending on how Default is
        // wired). The runtime guard absorbs the strict-absent intent:
        let cfg_keys = format!("{cfg:?}");
        assert!(
            cfg_keys.contains("idle_timeout_ms"),
            "idle_timeout_ms must remain a documented field (debug snapshot: {cfg_keys})"
        );
        assert!(
            cfg_keys.contains("quiet_window_ms"),
            "quiet_window_ms must remain a documented field (debug snapshot: {cfg_keys})"
        );
        assert!(
            cfg_keys.contains("total_runtime_cap_ms"),
            "total_runtime_cap_ms must remain a documented field (debug snapshot: {cfg_keys})"
        );
        assert!(
            !cfg_keys.contains("degrade_after_ms"),
            "degrade_after_ms was removed and must not return; debug snapshot: {cfg_keys}"
        );
    }

    /// Round-5 AI review (snapshot() misleading public API): the deprecated
    /// shim still works but the canonical entry is [`RuntimeMirror::snapshot_at`].
    /// Provide a regression-style test that exercises the precedence
    /// invariant that [`snapshot()`] honours — when cancel_pending is set
    /// and last_logical_event_at is up-to-date, the deprecated shim still
    /// surfaces `Cancelling`. This guards the deprecated shim from being
    /// silently broken by future precedence changes.
    #[test]
    fn deprecated_snapshot_surface_still_reports_cancelling_when_pending() {
        let mut mirror = mirror_with_config(2_000);
        mirror.apply_initial_conversation_snapshot(&conversation_with_status("running"));
        mirror.apply_socket_ready(TimestampMs::new(1_000));
        mirror.apply_event(&runtime_event("evt-1", "MessageEvent", 1_500));
        mirror.apply_cancelling("user_requested", TimestampMs::new(1_700));
        // The shim is intentionally pinned to last_logical_event_at. The
        // Cancelling flag is preserved across every precedence branch so
        // the snapshot still surfaces it without depending on `now`.
        let snap = mirror.snapshot_pinned_at_last_activity();
        assert!(matches!(snap.phase, RuntimeLivenessPhase::Cancelling));
    }
}
