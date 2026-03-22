use std::{fmt, num::NonZeroU32, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ConversationId, DurationMs, IssueId, IssueIdentifier, TimestampMs, WorkerId, WorkspaceKey,
};

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
    pub fresh_conversation: bool,
    pub runtime_contract_version: Option<String>,
    pub stream_state: RuntimeStreamState,
    pub last_event_id: Option<String>,
    pub last_event_kind: Option<String>,
    pub last_event_at: Option<TimestampMs>,
    pub last_event_summary: Option<String>,
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
        self.last_event_id = event_id;
        self.last_event_kind = event_kind;
        self.last_event_summary = summary;
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
        issue: &crate::NormalizedIssue,
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
        issue: &crate::NormalizedIssue,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StallMetadata {
    pub last_activity_at: TimestampMs,
    pub stall_timeout_ms: DurationMs,
    pub stalled_at: TimestampMs,
}

impl StallMetadata {
    pub fn new(last_activity_at: TimestampMs, stall_timeout_ms: DurationMs) -> Self {
        Self {
            last_activity_at,
            stall_timeout_ms,
            stalled_at: last_activity_at.saturating_add(stall_timeout_ms),
        }
    }

    pub fn observe_activity(&mut self, activity_at: TimestampMs) {
        if activity_at < self.last_activity_at {
            return;
        }

        self.last_activity_at = activity_at;
        self.stalled_at = activity_at.saturating_add(self.stall_timeout_ms);
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
