//! Shared domain contracts for OpenSymphony.

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Fixed continuation delay after a clean worker exit.
pub const CONTINUATION_RETRY_DELAY_MS: i64 = 1_000;

/// Lowercase a tracker state name for stable comparisons.
pub fn normalize_state_name(state: &str) -> String {
    state.trim().to_lowercase()
}

fn normalize_labels<I>(labels: I) -> Vec<String>
where
    I: IntoIterator,
    I::Item: Into<String>,
{
    labels
        .into_iter()
        .map(Into::into)
        .map(|label| label.trim().to_lowercase())
        .filter(|label| !label.is_empty())
        .collect()
}

/// Best-effort blocker reference attached to an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerRef {
    pub id: Option<String>,
    pub identifier: Option<String>,
    pub state: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl BlockerRef {
    /// Return true when the blocker is in one of the configured terminal states.
    pub fn is_terminal(&self, terminal_states: &[String]) -> bool {
        self.state
            .as_deref()
            .map(normalize_state_name)
            .is_some_and(|state| terminal_states.iter().any(|terminal| terminal == &state))
    }
}

/// Normalized tracker issue model used across orchestration, prompting, and snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub state: String,
    pub branch_name: Option<String>,
    pub url: Option<String>,
    pub labels: Vec<String>,
    pub blocked_by: Vec<BlockerRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Issue {
    /// Build a minimal issue with the fields required by scheduling and prompt rendering.
    pub fn new(
        id: impl Into<String>,
        identifier: impl Into<String>,
        title: impl Into<String>,
        state: impl Into<String>,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: id.into(),
            identifier: identifier.into(),
            title: title.into(),
            description: None,
            priority: None,
            state: state.into(),
            branch_name: None,
            url: None,
            labels: Vec::new(),
            blocked_by: Vec::new(),
            created_at,
            updated_at: created_at,
        }
    }

    /// Attach a description to the issue.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set the issue priority.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = Some(priority);
        self
    }

    /// Replace the normalized label set.
    pub fn with_labels<I>(mut self, labels: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        self.labels = normalize_labels(labels);
        self
    }

    /// Replace the blocker references.
    pub fn with_blockers(mut self, blockers: Vec<BlockerRef>) -> Self {
        self.blocked_by = blockers;
        self
    }

    /// Override the update timestamp.
    pub fn with_updated_at(mut self, updated_at: DateTime<Utc>) -> Self {
        self.updated_at = updated_at;
        self
    }

    /// Override the branch name.
    pub fn with_branch_name(mut self, branch_name: impl Into<String>) -> Self {
        self.branch_name = Some(branch_name.into());
        self
    }

    /// Lowercased state for comparisons.
    pub fn normalized_state(&self) -> String {
        normalize_state_name(&self.state)
    }

    /// Return true when the issue is in one of the configured terminal states.
    pub fn is_terminal(&self, terminal_states: &[String]) -> bool {
        let normalized = self.normalized_state();
        terminal_states.iter().any(|state| state == &normalized)
    }

    /// Return true when the issue is in one of the configured active states and not terminal.
    pub fn is_active(&self, active_states: &[String], terminal_states: &[String]) -> bool {
        let normalized = self.normalized_state();
        active_states.iter().any(|state| state == &normalized)
            && !terminal_states.iter().any(|state| state == &normalized)
    }

    /// Return true when any blocker is not terminal.
    pub fn has_non_terminal_blockers(&self, terminal_states: &[String]) -> bool {
        self.blocked_by
            .iter()
            .any(|blocker| !blocker.is_terminal(terminal_states))
    }
}

/// Filesystem assignment for a single issue workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceAssignment {
    pub workspace_key: String,
    pub path: PathBuf,
    pub created_now: bool,
}

/// High-level worker phase used by the orchestrator and control plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunPhase {
    PreparingWorkspace,
    BuildingPrompt,
    LaunchingAgentProcess,
    InitializingSession,
    StreamingTurn,
    Finishing,
    Succeeded,
    Failed,
    TimedOut,
    Stalled,
    CanceledByReconciliation,
}

/// Terminal worker outcome used by retry logic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerOutcomeKind {
    Succeeded,
    Failed,
    TimedOut,
    Stalled,
    CanceledByReconciliation,
}

/// Result reported back to the orchestrator when a worker stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerOutcome {
    pub kind: WorkerOutcomeKind,
    pub error: Option<String>,
}

impl WorkerOutcome {
    pub fn succeeded() -> Self {
        Self {
            kind: WorkerOutcomeKind::Succeeded,
            error: None,
        }
    }

    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::Failed,
            error: Some(error.into()),
        }
    }

    pub fn timed_out(error: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::TimedOut,
            error: Some(error.into()),
        }
    }

    pub fn stalled(error: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::Stalled,
            error: Some(error.into()),
        }
    }

    pub fn canceled_by_reconciliation(error: impl Into<String>) -> Self {
        Self {
            kind: WorkerOutcomeKind::CanceledByReconciliation,
            error: Some(error.into()),
        }
    }
}

/// Aggregate token counters reported by the runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

impl TokenUsage {
    pub fn add_assign(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
    }
}

/// Latest rate-limit snapshot carried over from runtime events.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitSnapshot {
    pub requests_remaining: Option<u64>,
    pub tokens_remaining: Option<u64>,
    pub resets_at: Option<DateTime<Utc>>,
}

/// Live worker session data retained by the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSession {
    pub phase: RunPhase,
    pub turn_count: u32,
    pub last_event_kind: Option<String>,
    pub last_event_message: Option<String>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub token_usage: TokenUsage,
    pub rate_limits: Option<RateLimitSnapshot>,
}

impl Default for RuntimeSession {
    fn default() -> Self {
        Self {
            phase: RunPhase::PreparingWorkspace,
            turn_count: 0,
            last_event_kind: None,
            last_event_message: None,
            last_event_at: None,
            token_usage: TokenUsage::default(),
            rate_limits: None,
        }
    }
}

impl RuntimeSession {
    /// Update the last seen event metadata while preserving other counters.
    pub fn with_event(
        mut self,
        kind: impl Into<String>,
        message: impl Into<String>,
        event_at: DateTime<Utc>,
    ) -> Self {
        self.last_event_kind = Some(kind.into());
        self.last_event_message = Some(message.into());
        self.last_event_at = Some(event_at);
        self
    }

    /// Replace the phase.
    pub fn with_phase(mut self, phase: RunPhase) -> Self {
        self.phase = phase;
        self
    }
}

/// One execution attempt for an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAttempt {
    pub issue_id: String,
    pub issue_identifier: String,
    pub attempt: Option<u32>,
    pub workspace_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub session: RuntimeSession,
}

/// Scheduled retry metadata held by the orchestrator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryEntry {
    pub issue_id: String,
    pub identifier: String,
    pub attempt: u32,
    pub due_at: DateTime<Utc>,
    pub error: Option<String>,
}

/// Internal claim state used by the orchestrator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrchestrationState {
    Unclaimed,
    Claimed,
    Running,
    RetryQueued,
    Released,
}

/// Aggregate runtime totals for the control plane.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeTotals {
    pub token_usage: TokenUsage,
    pub runtime_seconds: u64,
}

/// Snapshot entry for a running issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningIssueSnapshot {
    pub issue: Issue,
    pub attempt: Option<u32>,
    pub workspace_path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub session: RuntimeSession,
    pub orchestration_state: OrchestrationState,
}

/// Snapshot entry for a queued retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryQueueSnapshot {
    pub issue_id: String,
    pub identifier: String,
    pub attempt: u32,
    pub due_at: DateTime<Utc>,
    pub error: Option<String>,
}

/// Control-plane view derived from the orchestrator state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorSnapshot {
    pub generated_at: DateTime<Utc>,
    pub poll_interval_ms: u64,
    pub max_concurrent_agents: usize,
    pub claimed_issue_ids: Vec<String>,
    pub completed_issue_ids: Vec<String>,
    pub running: Vec<RunningIssueSnapshot>,
    pub retry_queue: Vec<RetryQueueSnapshot>,
    pub runtime_totals: RuntimeTotals,
    pub rate_limits: Option<RateLimitSnapshot>,
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).single().unwrap()
    }

    #[test]
    fn issue_normalizes_labels_and_states() {
        let issue = Issue::new("1", "OSYM-1", "Title", "In Progress", timestamp(1))
            .with_labels(["Needs Review", " BUG "]);

        assert_eq!(issue.labels, vec!["needs review", "bug"]);
        assert_eq!(issue.normalized_state(), "in progress");
    }

    #[test]
    fn blockers_only_count_when_not_terminal() {
        let issue = Issue::new("1", "OSYM-1", "Title", "Todo", timestamp(1)).with_blockers(vec![
            BlockerRef {
                id: Some("b1".into()),
                identifier: Some("OSYM-2".into()),
                state: Some("Done".into()),
                created_at: None,
                updated_at: None,
            },
            BlockerRef {
                id: Some("b2".into()),
                identifier: Some("OSYM-3".into()),
                state: Some("In Progress".into()),
                created_at: None,
                updated_at: None,
            },
        ]);

        assert!(issue.has_non_terminal_blockers(&[
            "done".into(),
            "closed".into(),
            "cancelled".into(),
        ]));
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let snapshot = OrchestratorSnapshot {
            generated_at: timestamp(10),
            poll_interval_ms: 30_000,
            max_concurrent_agents: 4,
            claimed_issue_ids: vec!["issue-1".into()],
            completed_issue_ids: vec!["issue-2".into()],
            running: vec![RunningIssueSnapshot {
                issue: Issue::new("1", "OSYM-1", "Title", "In Progress", timestamp(1)),
                attempt: Some(1),
                workspace_path: PathBuf::from("/tmp/OSYM-1"),
                started_at: timestamp(2),
                session: RuntimeSession::default().with_phase(RunPhase::StreamingTurn),
                orchestration_state: OrchestrationState::Running,
            }],
            retry_queue: vec![RetryQueueSnapshot {
                issue_id: "1".into(),
                identifier: "OSYM-1".into(),
                attempt: 2,
                due_at: timestamp(20),
                error: Some("transient".into()),
            }],
            runtime_totals: RuntimeTotals {
                token_usage: TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                    total_tokens: 15,
                },
                runtime_seconds: 8,
            },
            rate_limits: Some(RateLimitSnapshot {
                requests_remaining: Some(10),
                tokens_remaining: Some(50),
                resets_at: Some(timestamp(30)),
            }),
        };

        let json = serde_json::to_string(&snapshot).unwrap();
        let decoded: OrchestratorSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, snapshot);
    }
}
