use serde::{Deserialize, Serialize};

use crate::{
    ConversationMetadata, IssueExecution, NormalizedIssue, ReleaseReason, RetryEntry,
    SchedulerState, SchedulerStatus, TimestampMs, WorkerOutcomeRecord, WorkspaceRecord,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    #[default]
    Unknown,
    Starting,
    Healthy,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentHealthSnapshot {
    pub status: HealthStatus,
    pub detail: Option<String>,
    pub updated_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeUsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub runtime_seconds: u64,
    pub estimated_cost_usd_micros: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonSnapshot {
    pub health: HealthStatus,
    pub poll_interval_ms: u64,
    pub max_concurrent_agents: u32,
    pub running_issue_count: usize,
    pub retry_queue_count: usize,
    pub last_poll_at: Option<TimestampMs>,
    pub agent_server: ComponentHealthSnapshot,
    pub usage: RuntimeUsageTotals,
}

impl DaemonSnapshot {
    pub fn new(
        health: HealthStatus,
        poll_interval_ms: u64,
        max_concurrent_agents: u32,
        last_poll_at: Option<TimestampMs>,
        agent_server: ComponentHealthSnapshot,
        usage: RuntimeUsageTotals,
    ) -> Self {
        Self {
            health,
            poll_interval_ms,
            max_concurrent_agents,
            running_issue_count: 0,
            retry_queue_count: 0,
            last_poll_at,
            agent_server,
            usage,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerAttemptSnapshot {
    pub worker_id: crate::WorkerId,
    pub attempt: Option<crate::RetryAttempt>,
    pub turn_count: u32,
    pub max_turns: u32,
}

impl From<&crate::RunAttempt> for WorkerAttemptSnapshot {
    fn from(run: &crate::RunAttempt) -> Self {
        Self {
            worker_id: run.worker_id.clone(),
            attempt: run.attempt,
            turn_count: run.turn_count,
            max_turns: run.max_turns,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrySnapshot {
    pub attempt: crate::RetryAttempt,
    pub normal_retry_count: u32,
    pub scheduled_at: TimestampMs,
    pub due_at: TimestampMs,
    pub reason: crate::RetryReason,
    pub error: Option<String>,
}

impl From<&RetryEntry> for RetrySnapshot {
    fn from(retry: &RetryEntry) -> Self {
        Self {
            attempt: retry.attempt,
            normal_retry_count: retry.normal_retry_count,
            scheduled_at: retry.scheduled_at,
            due_at: retry.due_at,
            reason: retry.reason,
            error: retry.error.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStateSnapshot {
    pub state: SchedulerStatus,
    pub claimed_at: Option<TimestampMs>,
    pub started_at: Option<TimestampMs>,
    pub released_at: Option<TimestampMs>,
    pub release_reason: Option<ReleaseReason>,
    pub worker: Option<WorkerAttemptSnapshot>,
    pub last_event_at: Option<TimestampMs>,
    pub stalled_at: Option<TimestampMs>,
}

impl RuntimeStateSnapshot {
    fn from_execution(execution: &IssueExecution) -> Self {
        let conversation = execution.conversation();

        match execution.state() {
            SchedulerState::Unclaimed { .. } => Self {
                state: SchedulerStatus::Unclaimed,
                claimed_at: None,
                started_at: None,
                released_at: None,
                release_reason: None,
                worker: None,
                last_event_at: conversation.and_then(|conversation| conversation.last_event_at),
                stalled_at: None,
            },
            SchedulerState::Claimed { run } => Self {
                state: SchedulerStatus::Claimed,
                claimed_at: Some(run.claimed_at),
                started_at: run.started_at,
                released_at: None,
                release_reason: None,
                worker: Some(WorkerAttemptSnapshot::from(run)),
                last_event_at: conversation.and_then(|conversation| conversation.last_event_at),
                stalled_at: None,
            },
            SchedulerState::Running { run, stall } => Self {
                state: SchedulerStatus::Running,
                claimed_at: Some(run.claimed_at),
                started_at: run.started_at,
                released_at: None,
                release_reason: None,
                worker: Some(WorkerAttemptSnapshot::from(run)),
                last_event_at: conversation
                    .and_then(|conversation| conversation.last_event_at)
                    .or(Some(stall.last_activity_at)),
                stalled_at: Some(stall.stalled_at),
            },
            SchedulerState::RetryQueued { .. } => Self {
                state: SchedulerStatus::RetryQueued,
                claimed_at: None,
                started_at: None,
                released_at: None,
                release_reason: None,
                worker: None,
                last_event_at: conversation.and_then(|conversation| conversation.last_event_at),
                stalled_at: None,
            },
            SchedulerState::Released {
                released_at,
                reason,
            } => Self {
                state: SchedulerStatus::Released,
                claimed_at: None,
                started_at: None,
                released_at: Some(*released_at),
                release_reason: Some(*reason),
                worker: None,
                last_event_at: conversation.and_then(|conversation| conversation.last_event_at),
                stalled_at: None,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSnapshot {
    pub issue: NormalizedIssue,
    pub runtime: RuntimeStateSnapshot,
    pub workspace: Option<WorkspaceRecord>,
    pub conversation: Option<ConversationMetadata>,
    pub retry: Option<RetrySnapshot>,
    pub last_worker_outcome: Option<WorkerOutcomeRecord>,
    pub recent_worker_outcomes: Vec<WorkerOutcomeRecord>,
}

impl From<&IssueExecution> for IssueSnapshot {
    fn from(execution: &IssueExecution) -> Self {
        Self {
            issue: execution.issue().clone(),
            runtime: RuntimeStateSnapshot::from_execution(execution),
            workspace: execution.workspace().cloned(),
            conversation: execution.conversation().cloned(),
            retry: execution.retry().map(RetrySnapshot::from),
            last_worker_outcome: execution.last_worker_outcome().cloned(),
            recent_worker_outcomes: execution.recent_worker_outcomes().to_vec(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorSnapshot {
    pub generated_at: TimestampMs,
    pub daemon: DaemonSnapshot,
    pub issues: Vec<IssueSnapshot>,
}

impl OrchestratorSnapshot {
    pub fn new(
        generated_at: TimestampMs,
        daemon: DaemonSnapshot,
        issues: Vec<IssueSnapshot>,
    ) -> Self {
        let running_issue_count = issues
            .iter()
            .filter(|issue| issue.runtime.state == SchedulerStatus::Running)
            .count();
        let retry_queue_count = issues
            .iter()
            .filter(|issue| issue.runtime.state == SchedulerStatus::RetryQueued)
            .count();

        Self {
            generated_at,
            daemon: DaemonSnapshot {
                running_issue_count,
                retry_queue_count,
                ..daemon
            },
            issues,
        }
    }
}
