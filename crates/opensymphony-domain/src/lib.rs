use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use thiserror::Error;

pub const CONTINUATION_RETRY_DELAY_MS: i64 = 1_000;
pub const FAILURE_RETRY_BASE_DELAY_MS: i64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueBlocker {
    pub id: String,
    pub identifier: String,
    pub state: String,
    #[serde(default)]
    pub is_terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub blocked_by: Vec<IssueBlocker>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Issue {
    pub fn priority_rank(&self) -> i32 {
        match self.priority {
            Some(1) => 4,
            Some(2) => 3,
            Some(3) => 2,
            Some(4) => 1,
            _ => 0,
        }
    }

    pub fn is_blocked_by_active_issue(&self) -> bool {
        self.blocked_by.iter().any(|blocker| !blocker.is_terminal)
    }
}

pub fn compare_issues(left: &Issue, right: &Issue) -> Ordering {
    right
        .priority_rank()
        .cmp(&left.priority_rank())
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.identifier.cmp(&right.identifier))
}

pub fn sort_candidates(issues: &mut [Issue]) {
    issues.sort_by(compare_issues);
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueStateSnapshot {
    pub id: String,
    pub identifier: String,
    pub state: String,
    pub is_active: bool,
    pub is_terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkerLifecycleState {
    Unclaimed,
    Claimed,
    Running,
    RetryQueued,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RetryReason {
    Continuation,
    Failure,
    Stall,
    Recovery,
    Reconcile,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkerOutcomeKind {
    Success,
    Failure,
    Cancelled,
    Stalled,
    Released,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkerOutcome {
    pub kind: WorkerOutcomeKind,
    pub detail: Option<String>,
    pub execution_status: Option<String>,
    pub observed_at: DateTime<Utc>,
}

impl WorkerOutcome {
    pub fn success() -> Self {
        Self {
            kind: WorkerOutcomeKind::Success,
            detail: None,
            execution_status: Some("completed".to_string()),
            observed_at: Utc::now(),
        }
    }

    pub fn failure(detail: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::Failure,
            detail: Some(detail.into()),
            execution_status: Some("failed".to_string()),
            observed_at: Utc::now(),
        }
    }

    pub fn cancelled(detail: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::Cancelled,
            detail: Some(detail.into()),
            execution_status: Some("cancelled".to_string()),
            observed_at: Utc::now(),
        }
    }

    pub fn stalled(detail: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::Stalled,
            detail: Some(detail.into()),
            execution_status: Some("stalled".to_string()),
            observed_at: Utc::now(),
        }
    }

    pub fn released(detail: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::Released,
            detail: Some(detail.into()),
            execution_status: Some("released".to_string()),
            observed_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryEntry {
    pub issue: Issue,
    pub attempt: u32,
    pub reason: RetryReason,
    pub scheduled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptContext {
    pub number: u32,
    pub continuation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SchedulerConfig {
    pub max_concurrency: usize,
    pub max_retry_backoff_ms: i64,
    pub stall_timeout_ms: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            max_retry_backoff_ms: 300_000,
            stall_timeout_ms: 300_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrchestratorSnapshot {
    pub running_issue_ids: Vec<String>,
    pub queued_retry_ids: Vec<String>,
}

pub fn continuation_retry_at(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::milliseconds(CONTINUATION_RETRY_DELAY_MS)
}

pub fn failure_retry_delay(attempt: u32, max_backoff_ms: i64) -> Duration {
    let power = attempt.saturating_sub(1).min(16);
    let multiplier = 1_i64.checked_shl(power).unwrap_or(i64::MAX);
    let delay_ms = FAILURE_RETRY_BASE_DELAY_MS
        .saturating_mul(multiplier)
        .min(max_backoff_ms);
    Duration::milliseconds(delay_ms)
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TrackerError {
    #[error("tracker auth error: {0}")]
    Auth(String),
    #[error("tracker rate limited: {0}")]
    RateLimited(String),
    #[error("tracker transport error: {0}")]
    Transport(String),
    #[error("tracker timeout: {0}")]
    Timeout(String),
    #[error("tracker invalid response: {0}")]
    InvalidResponse(String),
    #[error("tracker not found: {0}")]
    NotFound(String),
    #[error("tracker permission denied: {0}")]
    PermissionDenied(String),
}

#[async_trait]
pub trait IssueTracker: Send + Sync {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, TrackerError>;

    async fn fetch_states_by_issue_ids(
        &self,
        issue_ids: &[String],
    ) -> Result<Vec<IssueStateSnapshot>, TrackerError>;

    async fn fetch_terminal_issues(&self) -> Result<Vec<Issue>, TrackerError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn issue(identifier: &str, priority: Option<u8>, created_at: DateTime<Utc>) -> Issue {
        Issue {
            id: identifier.to_string(),
            identifier: identifier.to_string(),
            title: identifier.to_string(),
            description: None,
            priority,
            state: "Todo".to_string(),
            labels: vec![],
            blocked_by: vec![],
            created_at,
            updated_at: created_at,
        }
    }

    #[test]
    fn sorts_by_priority_then_age_then_identifier() {
        let mut issues = vec![
            issue(
                "C-1",
                Some(3),
                Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 2).unwrap(),
            ),
            issue(
                "A-1",
                Some(1),
                Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 3).unwrap(),
            ),
            issue(
                "B-1",
                Some(1),
                Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 1).unwrap(),
            ),
        ];

        sort_candidates(&mut issues);

        let ordered = issues
            .into_iter()
            .map(|issue| issue.identifier)
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["B-1", "A-1", "C-1"]);
    }

    #[test]
    fn failure_backoff_caps_at_configured_limit() {
        let delay = failure_retry_delay(8, 120_000);
        assert_eq!(delay.num_milliseconds(), 120_000);
    }

    #[test]
    fn detects_active_blockers() {
        let issue = Issue {
            id: "1".to_string(),
            identifier: "ABC-1".to_string(),
            title: "Blocked".to_string(),
            description: None,
            priority: None,
            state: "Todo".to_string(),
            labels: vec![],
            blocked_by: vec![IssueBlocker {
                id: "2".to_string(),
                identifier: "ABC-2".to_string(),
                state: "Todo".to_string(),
                is_terminal: false,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        assert!(issue.is_blocked_by_active_issue());
    }
}
