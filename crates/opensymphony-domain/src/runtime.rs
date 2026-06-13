use std::{fmt, num::NonZeroU32, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{
    ConversationId, DurationMs, IssueId, IssueIdentifier, TimestampMs, WorkerId, WorkspaceKey,
};

/// Normalized liveness phase for a long-running OpenSymphony execution turn.
///
/// These phases are OpenSymphony-normalized and do not leak OpenHands wire details
/// into the orchestrator core. They align with the six conceptual liveness states
/// in the run-detail view: `active`, `quiet`, `degraded`, `stalled`, `detached`,
/// and `terminal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLivenessPhase {
    /// Waiting for a prior turn (e.g., in a reused conversation) to complete
    /// before a new message can be sent.
    WaitingOnPriorTurn,
    /// A turn is actively executing; the harness monitors for progress.
    RunningTurn,
    /// A turn is active but no liveness signal arrived within the recent idle
    /// window. Progress-based stall detection still applies, so the run is not
    /// yet declared stalled and may still emit new events.
    Quiet,
    /// The runtime stream is reachable but not in a healthy state (e.g., the
    /// server returned a `/run` conflict or repeated partial failures).
    /// Progress-based stall detection should still escalate to `Stalled` if no
    /// signals appear.
    Degraded,
    /// Stream disconnected; attempting REST reconcile to find progress.
    Reconciling,
    /// Scheduler has declared a stall and is cancelling the underlying run.
    Cancelling,
    /// No progress was observed within the idle timeout; run is considered stalled.
    Stalled,
    /// The underlying run could not be stopped; execution is detached from this
    /// OpenSymphony worker. Subsequent retries must not duplicate in-flight work.
    Detached,
    /// OpenHands reported a terminal execution status (`finished`, `error`, or
    /// `stuck`). The run is no longer mid-flight; only flush + cleanup remain.
    Terminal,
}

impl RuntimeLivenessPhase {
    /// Map a phase to the six conceptual liveness states surfaced in the
    /// run-detail view: active, quiet, degraded, stalled, detached, terminal.
    pub fn liveness_state(self) -> LivenessState {
        match self {
            Self::WaitingOnPriorTurn | Self::RunningTurn => LivenessState::Active,
            Self::Quiet => LivenessState::Quiet,
            Self::Degraded => LivenessState::Degraded,
            Self::Reconciling | Self::Cancelling | Self::Stalled => LivenessState::Stalled,
            Self::Detached => LivenessState::Detached,
            Self::Terminal => LivenessState::Terminal,
        }
    }
}

/// Six-state aggregation surfaced in the run-detail view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LivenessState {
    /// Turn is actively executing (`RunningTurn` or `WaitingOnPriorTurn`).
    Active,
    /// Turn is alive but idle within the recent window (`Quiet`).
    Quiet,
    /// Runtime stream is reachable but degraded (`Degraded`).
    Degraded,
    /// Progress-based stall: `Stalled`, `Reconciling`, or `Cancelling`.
    Stalled,
    /// Detached: no longer bounded by this worker (`Detached`).
    Detached,
    /// Terminal: OpenHands reported `finished`, `error`, or `stuck` (`Terminal`).
    Terminal,
}

impl fmt::Display for LivenessState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Quiet => write!(f, "quiet"),
            Self::Degraded => write!(f, "degraded"),
            Self::Stalled => write!(f, "stalled"),
            Self::Detached => write!(f, "detached"),
            Self::Terminal => write!(f, "terminal"),
        }
    }
}

impl fmt::Display for RuntimeLivenessPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WaitingOnPriorTurn => write!(f, "waiting_on_prior_turn"),
            Self::RunningTurn => write!(f, "running_turn"),
            Self::Quiet => write!(f, "quiet"),
            Self::Degraded => write!(f, "degraded"),
            Self::Reconciling => write!(f, "reconciling"),
            Self::Cancelling => write!(f, "cancelling"),
            Self::Stalled => write!(f, "stalled"),
            Self::Detached => write!(f, "detached"),
            Self::Terminal => write!(f, "terminal"),
        }
    }
}

/// Structured snapshot of runtime progress emitted by the session runner.
///
/// Feeds the gateway and event journal so operators can see liveness
/// signals during long-running turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeProgressSnapshot {
    /// Current liveness phase.
    pub phase: RuntimeLivenessPhase,
    /// Aggregated liveness state derived from [`phase`](Self::phase).
    /// Six surfaces: active, quiet, degraded, stalled, detached, terminal.
    pub liveness_state: LivenessState,
    /// Monotonic count of events observed since the session was created.
    pub event_count: u64,
    /// Delta of new events since the last snapshot (zero if unchanged).
    pub event_delta: u64,
    /// Total input tokens consumed so far.
    pub input_tokens: u64,
    /// Delta of input tokens since the last snapshot.
    pub input_token_delta: u64,
    /// Total output tokens produced so far.
    pub output_tokens: u64,
    /// Delta of output tokens since the last snapshot.
    pub output_token_delta: u64,
    /// Total cache-read tokens reported by the provider, if available.
    pub cache_read_tokens: u64,
    /// Delta of cache-read tokens since the last snapshot.
    pub cache_read_token_delta: u64,
    /// Current execution status reported by the runtime, if available.
    pub execution_status: Option<String>,
    /// Stream health reported by the runtime mirror.
    pub stream_health: StreamHealth,
    /// History-sync status reported by the runtime mirror.
    pub history_sync_status: HistorySyncStatus,
    /// Reconnect status reported by the runtime mirror.
    pub reconnect_status: ReconnectStatus,
    /// Timestamp of the most recent liveness signal (event, token bump, status change).
    pub last_activity_at: Option<TimestampMs>,
    /// Sliding deadline after which the run is considered stalled without new progress.
    pub stall_deadline_at: Option<TimestampMs>,
    /// Stable cursor for the most recently observed event (`event_id`).
    pub last_event_cursor: Option<String>,
    /// Kind of the most recent event (typed envelope kind tag).
    pub last_event_kind: Option<String>,
    /// Wall-clock timestamp for the most recent event (separate from logical
    /// progress timestamps for backfill vs live edge purposes).
    pub last_event_at: Option<TimestampMs>,
    /// Detach metadata, populated only if the run was detached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detach_metadata: Option<DetachMetadata>,
}

impl RuntimeProgressSnapshot {
    /// Create an initial snapshot with zero counters.
    pub fn initial(phase: RuntimeLivenessPhase) -> Self {
        let liveness_state = phase.liveness_state();
        Self {
            phase,
            liveness_state,
            event_count: 0,
            event_delta: 0,
            input_tokens: 0,
            input_token_delta: 0,
            output_tokens: 0,
            output_token_delta: 0,
            cache_read_tokens: 0,
            cache_read_token_delta: 0,
            execution_status: None,
            stream_health: StreamHealth::Unknown,
            history_sync_status: HistorySyncStatus::Idle,
            reconnect_status: ReconnectStatus::Connected,
            last_activity_at: None,
            stall_deadline_at: None,
            last_event_cursor: None,
            last_event_kind: None,
            last_event_at: None,
            detach_metadata: None,
        }
    }

    /// Start building an updated snapshot from this snapshot's baseline.
    pub fn update_with(&self, phase: RuntimeLivenessPhase) -> RuntimeProgressSnapshotBuilder<'_> {
        RuntimeProgressSnapshotBuilder {
            previous: self,
            phase,
            event_count: self.event_count,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            execution_status: self.execution_status.clone(),
            stream_health: self.stream_health,
            history_sync_status: self.history_sync_status,
            reconnect_status: self.reconnect_status,
            last_activity_at: self.last_activity_at,
            stall_deadline_at: self.stall_deadline_at,
            last_event_cursor: self.last_event_cursor.clone(),
            last_event_kind: self.last_event_kind.clone(),
            last_event_at: self.last_event_at,
            detach_metadata: self.detach_metadata.clone(),
        }
    }
}

/// Stream health of the underlying harness runtime, surfaced in
/// [`RuntimeProgressSnapshot::stream_health`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamHealth {
    /// Stream state not yet observed.
    Unknown,
    /// Stream is connecting or awaiting its first `ConversationStateUpdateEvent`.
    Attaching,
    /// REST history sync is currently in progress.
    HistorySyncing,
    /// WebSocket is connected and the readiness barrier has been crossed.
    Ready,
    /// A disconnect or transport error is being recovered; the stream attempts
    /// to reconcile via REST history and a fresh WebSocket attach.
    Reconnecting,
    /// WebSocket closed cleanly before the worker released the conversation.
    Disconnected,
    /// WebSocket attempts exhausted; the runtime mirror may transition to a
    /// degraded or detached state.
    Failed,
    /// The runtime is detached because the worker lost exclusive ownership.
    Detached,
}

impl fmt::Display for StreamHealth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => write!(f, "unknown"),
            Self::Attaching => write!(f, "attaching"),
            Self::HistorySyncing => write!(f, "history_syncing"),
            Self::Ready => write!(f, "ready"),
            Self::Reconnecting => write!(f, "reconnecting"),
            Self::Disconnected => write!(f, "disconnected"),
            Self::Failed => write!(f, "failed"),
            Self::Detached => write!(f, "detached"),
        }
    }
}

/// REST history sync status surfaced in
/// [`RuntimeProgressSnapshot::history_sync_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySyncStatus {
    /// No sync attempted yet.
    Idle,
    /// Sync is currently in progress.
    InProgress,
    /// Sync completed cleanly and no further history has been received.
    Synced,
    /// Sync completed but newer history may have arrived after we last synced.
    Stale,
    /// Sync failed; the mirror is recovering via REST retry.
    Failed,
}

impl fmt::Display for HistorySyncStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Synced => write!(f, "synced"),
            Self::Stale => write!(f, "stale"),
            Self::Failed => write!(f, "failed"),
        }
    }
}

/// WebSocket reconnect status surfaced in
/// [`RuntimeProgressSnapshot::reconnect_status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconnectStatus {
    /// WebSocket is connected.
    Connected,
    /// A reconnect attempt is currently scheduled.
    Pending,
    /// Reconnect attempts exhausted; the stream mark the run degraded.
    Exhausted,
    /// WebSocket closed cleanly without reconnect.
    Closed,
}

impl fmt::Display for ReconnectStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connected => write!(f, "connected"),
            Self::Pending => write!(f, "pending"),
            Self::Exhausted => write!(f, "exhausted"),
            Self::Closed => write!(f, "closed"),
        }
    }
}

/// Builder for updating a [`RuntimeProgressSnapshot`] with delta computation.
///
/// Provides a fluent interface instead of a 7-argument `update` method,
/// satisfying clippy's argument-count lint without sacrificing ergonomics.
pub struct RuntimeProgressSnapshotBuilder<'a> {
    previous: &'a RuntimeProgressSnapshot,
    phase: RuntimeLivenessPhase,
    event_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    execution_status: Option<String>,
    stream_health: StreamHealth,
    history_sync_status: HistorySyncStatus,
    reconnect_status: ReconnectStatus,
    last_activity_at: Option<TimestampMs>,
    stall_deadline_at: Option<TimestampMs>,
    last_event_cursor: Option<String>,
    last_event_kind: Option<String>,
    last_event_at: Option<TimestampMs>,
    detach_metadata: Option<DetachMetadata>,
}

impl RuntimeProgressSnapshotBuilder<'_> {
    pub fn with_event_count(mut self, count: u64) -> Self {
        self.event_count = count;
        self
    }
    pub fn with_input_tokens(mut self, count: u64) -> Self {
        self.input_tokens = count;
        self
    }
    pub fn with_output_tokens(mut self, count: u64) -> Self {
        self.output_tokens = count;
        self
    }
    pub fn with_cache_read_tokens(mut self, count: u64) -> Self {
        self.cache_read_tokens = count;
        self
    }
    pub fn with_execution_status(mut self, status: Option<String>) -> Self {
        self.execution_status = status;
        self
    }
    pub fn with_stream_health(mut self, health: StreamHealth) -> Self {
        self.stream_health = health;
        self
    }
    pub fn with_history_sync_status(mut self, status: HistorySyncStatus) -> Self {
        self.history_sync_status = status;
        self
    }
    pub fn with_reconnect_status(mut self, status: ReconnectStatus) -> Self {
        self.reconnect_status = status;
        self
    }
    pub fn with_last_activity_at(mut self, ts: Option<TimestampMs>) -> Self {
        self.last_activity_at = ts;
        self
    }
    pub fn with_stall_deadline_at(mut self, ts: Option<TimestampMs>) -> Self {
        self.stall_deadline_at = ts;
        self
    }
    pub fn with_last_event_cursor(mut self, cursor: Option<String>) -> Self {
        self.last_event_cursor = cursor;
        self
    }
    pub fn with_last_event_kind(mut self, kind: Option<String>) -> Self {
        self.last_event_kind = kind;
        self
    }
    pub fn with_last_event_at(mut self, ts: Option<TimestampMs>) -> Self {
        self.last_event_at = ts;
        self
    }
    pub fn with_detach_metadata(mut self, metadata: Option<DetachMetadata>) -> Self {
        self.detach_metadata = metadata;
        self
    }

    /// Produce the new snapshot with computed deltas.
    pub fn build(self) -> RuntimeProgressSnapshot {
        RuntimeProgressSnapshot {
            event_delta: self.event_count.saturating_sub(self.previous.event_count),
            input_token_delta: self.input_tokens.saturating_sub(self.previous.input_tokens),
            output_token_delta: self
                .output_tokens
                .saturating_sub(self.previous.output_tokens),
            cache_read_token_delta: self
                .cache_read_tokens
                .saturating_sub(self.previous.cache_read_tokens),
            liveness_state: self.phase.liveness_state(),
            phase: self.phase,
            event_count: self.event_count,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            execution_status: self.execution_status,
            stream_health: self.stream_health,
            history_sync_status: self.history_sync_status,
            reconnect_status: self.reconnect_status,
            last_activity_at: self.last_activity_at,
            stall_deadline_at: self.stall_deadline_at,
            last_event_cursor: self.last_event_cursor,
            last_event_kind: self.last_event_kind,
            last_event_at: self.last_event_at,
            detach_metadata: self.detach_metadata,
        }
    }
}

/// Metadata recorded when a run is detached because the underlying OpenHands
/// execution could not be stopped or is no longer reachable by this worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetachMetadata {
    /// Reason the run was detached.
    pub reason: DetachReason,
    /// Timestamp when detachment was recorded.
    pub detached_at: TimestampMs,
    /// Last known execution status of the underlying runtime.
    pub last_execution_status: Option<String>,
    /// Summary explaining the detachment.
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetachReason {
    /// Stop/cancel was attempted but failed.
    CancelFailed,
    /// Stop/cancel is not supported by the runtime.
    CancelUnsupported,
    /// The runtime became unreachable (connection lost, server gone).
    Unreachable,
    /// The worker was shut down while the run was still active.
    WorkerShutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub path: PathBuf,
    pub workspace_key: WorkspaceKey,
    pub created_now: bool,
    pub created_at: Option<TimestampMs>,
    pub updated_at: Option<TimestampMs>,
    pub last_seen_tracker_refresh_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RetryAttempt(NonZeroU32);

impl RetryAttempt {
    pub const fn first() -> Self {
        Self(NonZeroU32::MIN)
    }

    pub fn new(value: u32) -> Result<Self, RetryCalculationError> {
        match NonZeroU32::new(value) {
            Some(value) => Ok(Self(value)),
            None => Err(RetryCalculationError::ZeroAttempt),
        }
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }

    pub fn after(previous: Option<Self>) -> Result<Self, RetryCalculationError> {
        match previous {
            Some(previous) => previous
                .checked_next()
                .ok_or(RetryCalculationError::AttemptOverflow),
            None => Ok(Self::first()),
        }
    }

    pub fn checked_next(self) -> Option<Self> {
        self.0
            .get()
            .checked_add(1)
            .and_then(NonZeroU32::new)
            .map(Self)
    }
}

impl fmt::Display for RetryAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.get())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RetryCalculationError {
    #[error("retry attempt must be greater than zero")]
    ZeroAttempt,
    #[error("retry attempt overflowed the supported range")]
    AttemptOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStreamState {
    Detached,
    Attaching,
    Ready,
    Reconnecting,
    Closed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationMetadata {
    pub conversation_id: ConversationId,
    pub server_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_query_param_name: Option<String>,
    pub fresh_conversation: bool,
    pub runtime_contract_version: Option<String>,
    pub stream_state: RuntimeStreamState,
    pub last_event_id: Option<String>,
    pub last_event_kind: Option<String>,
    pub last_event_at: Option<TimestampMs>,
    pub last_event_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_activity: Vec<ConversationActivityEvent>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub total_tokens: u64,
    #[serde(default)]
    pub runtime_seconds: u64,
}

const MAX_ACTIVITY_EVENTS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationActivityEvent {
    pub event_id: String,
    pub happened_at: TimestampMs,
    pub kind: String,
    pub summary: String,
}

impl ConversationMetadata {
    pub fn observe_event(
        &mut self,
        event_at: TimestampMs,
        event_id: Option<String>,
        event_kind: Option<String>,
        summary: Option<String>,
    ) {
        if self
            .last_event_at
            .is_some_and(|last_event_at| event_at < last_event_at)
        {
            return;
        }

        self.last_event_at = Some(event_at);
        self.last_event_id = event_id.clone();
        self.last_event_kind = event_kind.clone();
        self.last_event_summary = summary.clone();

        if let (Some(event_id), Some(event_kind), Some(summary)) = (event_id, event_kind, summary) {
            self.recent_activity.push(ConversationActivityEvent {
                event_id,
                happened_at: event_at,
                kind: event_kind,
                summary,
            });
            while self.recent_activity.len() > MAX_ACTIVITY_EVENTS {
                self.recent_activity.remove(0);
            }
        }
    }

    pub fn add_tokens(&mut self, input: u64, output: u64) {
        self.input_tokens += input;
        self.output_tokens += output;
        self.total_tokens += input + output;
    }

    pub fn add_runtime_seconds(&mut self, seconds: u64) {
        self.runtime_seconds += seconds;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub continuation_delay_ms: DurationMs,
    pub failure_base_delay_ms: DurationMs,
    pub max_backoff_ms: DurationMs,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            continuation_delay_ms: DurationMs::new(1_000),
            failure_base_delay_ms: DurationMs::new(10_000),
            max_backoff_ms: DurationMs::new(300_000),
        }
    }
}

impl RetryPolicy {
    pub fn failure_delay(self, attempt: RetryAttempt) -> DurationMs {
        let exponent = attempt.get().saturating_sub(1).min(63);
        let multiplier = 1_u64.checked_shl(exponent).unwrap_or(u64::MAX);
        let uncapped = self
            .failure_base_delay_ms
            .as_u64()
            .saturating_mul(multiplier);

        DurationMs::new(uncapped.min(self.max_backoff_ms.as_u64()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    Continuation,
    Failure,
    Stalled,
    Cancelled,
    Reconciliation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryEntry {
    pub issue_id: IssueId,
    pub identifier: IssueIdentifier,
    pub attempt: RetryAttempt,
    pub normal_retry_count: u32,
    pub scheduled_at: TimestampMs,
    pub due_at: TimestampMs,
    pub reason: RetryReason,
    pub error: Option<String>,
}

impl RetryEntry {
    pub fn continuation(
        issue: &super::NormalizedIssue,
        previous_attempt: Option<RetryAttempt>,
        normal_retry_count: u32,
        scheduled_at: TimestampMs,
        policy: RetryPolicy,
    ) -> Result<Self, RetryCalculationError> {
        let attempt = RetryAttempt::after(previous_attempt)?;

        Ok(Self {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            attempt,
            normal_retry_count: normal_retry_count.saturating_add(1),
            scheduled_at,
            due_at: scheduled_at.saturating_add(policy.continuation_delay_ms),
            reason: RetryReason::Continuation,
            error: None,
        })
    }

    pub fn failure(
        issue: &super::NormalizedIssue,
        previous_attempt: Option<RetryAttempt>,
        normal_retry_count: u32,
        scheduled_at: TimestampMs,
        reason: RetryReason,
        error: Option<String>,
        policy: RetryPolicy,
    ) -> Result<Self, RetryCalculationError> {
        let attempt = RetryAttempt::after(previous_attempt)?;

        Ok(Self {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            attempt,
            normal_retry_count,
            scheduled_at,
            due_at: scheduled_at.saturating_add(policy.failure_delay(attempt)),
            reason,
            error,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAttempt {
    pub worker_id: WorkerId,
    pub issue_id: IssueId,
    pub issue_identifier: IssueIdentifier,
    pub workspace_path: PathBuf,
    pub claimed_at: TimestampMs,
    pub started_at: Option<TimestampMs>,
    pub attempt: Option<RetryAttempt>,
    pub normal_retry_count: u32,
    pub turn_count: u32,
    pub max_turns: u32,
}

impl RunAttempt {
    pub fn new(
        worker_id: WorkerId,
        issue_id: IssueId,
        issue_identifier: IssueIdentifier,
        workspace_path: PathBuf,
        claimed_at: TimestampMs,
        attempt: Option<RetryAttempt>,
        max_turns: u32,
    ) -> Self {
        Self {
            worker_id,
            issue_id,
            issue_identifier,
            workspace_path,
            claimed_at,
            started_at: None,
            attempt,
            normal_retry_count: 0,
            turn_count: 0,
            max_turns,
        }
    }

    pub fn with_normal_retry_count(mut self, normal_retry_count: u32) -> Self {
        self.normal_retry_count = normal_retry_count;
        self
    }

    pub fn mark_started(mut self, started_at: TimestampMs) -> Self {
        self.started_at = Some(started_at);
        self
    }

    pub fn record_turn_started(&mut self) {
        self.turn_count = self.turn_count.saturating_add(1);
    }
}

/// Tracks progress-based stall detection with sliding deadlines.
///
/// Splits the semantics of:
/// - **Idle/progress timeout** (`idle_timeout_ms`): No liveness signal for this
///   duration triggers a stall. Slides forward on each new signal.
/// - **Total runtime cap** (`total_runtime_cap_ms`): Absolute wall-clock limit
///   regardless of progress, anchored to `started_at`. Only enforced when `Some`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StallMetadata {
    /// Timestamp when the run started (used to anchor the hard runtime cap).
    pub started_at: TimestampMs,
    /// Timestamp of the most recent liveness signal.
    pub last_activity_at: TimestampMs,
    /// Idle timeout in milliseconds. Slides forward on each progress signal.
    #[serde(alias = "stall_timeout_ms")]
    pub idle_timeout_ms: DurationMs,
    /// Absolute wall-clock cap on total runtime, if configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_runtime_cap_ms: Option<DurationMs>,
    /// Timestamp when the run becomes stalled (idle deadline or runtime cap, whichever is sooner).
    pub stalled_at: TimestampMs,
}

impl StallMetadata {
    /// Create with an idle timeout and no total runtime cap.
    pub fn new(started_at: TimestampMs, idle_timeout_ms: DurationMs) -> Self {
        Self::with_runtime_cap(started_at, idle_timeout_ms, None)
    }

    /// Create with both an idle timeout and a total runtime cap.
    pub fn with_runtime_cap(
        started_at: TimestampMs,
        idle_timeout_ms: DurationMs,
        total_runtime_cap_ms: Option<DurationMs>,
    ) -> Self {
        let idle_deadline = started_at.saturating_add(idle_timeout_ms);
        let stalled_at = match total_runtime_cap_ms {
            Some(cap) => {
                let hard_cap = started_at.saturating_add(cap);
                // Whichever deadline is sooner
                idle_deadline.min(hard_cap)
            }
            None => idle_deadline,
        };
        Self {
            started_at,
            last_activity_at: started_at,
            idle_timeout_ms,
            total_runtime_cap_ms,
            stalled_at,
        }
    }

    /// Record a new progress signal and slide the idle deadline forward.
    ///
    /// The total runtime cap (if configured) remains anchored to `started_at`
    /// and does NOT slide with activity signals.
    ///
    /// Returns `true` if the activity timestamp advanced the stall deadline.
    pub fn observe_activity(&mut self, activity_at: TimestampMs) -> bool {
        if activity_at < self.last_activity_at {
            return false;
        }

        self.last_activity_at = activity_at;

        // Idle deadline slides with each activity signal
        let idle_deadline = activity_at.saturating_add(self.idle_timeout_ms);
        // Hard cap remains anchored to the original start time
        let new_stalled_at = match self.total_runtime_cap_ms {
            Some(cap) => {
                let hard_cap = self.started_at.saturating_add(cap);
                idle_deadline.min(hard_cap)
            }
            None => idle_deadline,
        };

        // Only count as progress if the deadline actually moved forward
        let advanced = new_stalled_at > self.stalled_at;
        self.stalled_at = new_stalled_at;
        advanced
    }

    /// Check whether the current time has passed the stall deadline.
    pub fn is_stalled_at(&self, now: TimestampMs) -> bool {
        now >= self.stalled_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutcomeKind {
    Succeeded,
    Failed,
    TimedOut,
    Stalled,
    Cancelled,
    /// The underlying runtime could not be stopped; execution is detached
    /// from this OpenSymphony worker.
    Detached,
    /// An explicit cancel/stop was attempted but the runtime refused or
    /// the cancellation mechanism itself failed.
    CancelFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerOutcomeRecord {
    pub worker_id: WorkerId,
    pub attempt: Option<RetryAttempt>,
    pub outcome: WorkerOutcomeKind,
    pub started_at: TimestampMs,
    pub finished_at: TimestampMs,
    pub turn_count: u32,
    pub summary: Option<String>,
    pub error: Option<String>,
}

impl WorkerOutcomeRecord {
    pub fn from_run(
        run: &RunAttempt,
        outcome: WorkerOutcomeKind,
        finished_at: TimestampMs,
        summary: Option<String>,
        error: Option<String>,
    ) -> Self {
        Self {
            worker_id: run.worker_id.clone(),
            attempt: run.attempt,
            outcome,
            started_at: run.started_at.unwrap_or(run.claimed_at),
            finished_at,
            turn_count: run.turn_count,
            summary,
            error,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    Completed,
    TrackerInactive,
    TrackerTerminal,
    Cancelled,
    RetryExhausted,
}

impl ReleaseReason {
    pub const fn preserves_reactivation_state(self) -> bool {
        matches!(self, Self::TrackerInactive)
    }
}
