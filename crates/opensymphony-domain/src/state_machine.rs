use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ConversationMetadata, DurationMs, IssueId, NormalizedIssue, ReleaseReason, RetryAttempt,
    RetryEntry, RunAttempt, StallMetadata, TimestampMs, WorkerId, WorkerOutcomeRecord,
    WorkspaceKey, WorkspaceRecord,
};

const MAX_RECENT_WORKER_OUTCOMES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerStatus {
    Unclaimed,
    Claimed,
    Running,
    RetryQueued,
    Released,
}

impl SchedulerStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unclaimed => "unclaimed",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::RetryQueued => "retry_queued",
            Self::Released => "released",
        }
    }
}

impl std::fmt::Display for SchedulerStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionAction {
    Claim,
    StartRunning,
    RecordTurnStarted,
    ObserveRuntimeEvent,
    QueueRetry,
    Release,
    Reopen,
}

impl TransitionAction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claim => "claim",
            Self::StartRunning => "start_running",
            Self::RecordTurnStarted => "record_turn_started",
            Self::ObserveRuntimeEvent => "observe_runtime_event",
            Self::QueueRetry => "queue_retry",
            Self::Release => "release",
            Self::Reopen => "reopen",
        }
    }
}

impl std::fmt::Display for TransitionAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StateTransitionError {
    #[error("cannot {action} while issue is {from}")]
    InvalidTransition {
        from: SchedulerStatus,
        action: TransitionAction,
    },
    #[error("retry attempt mismatch: expected {expected:?}, got {actual:?}")]
    AttemptMismatch {
        expected: Option<RetryAttempt>,
        actual: Option<RetryAttempt>,
    },
    #[error("issue mismatch: expected {expected}, got {actual}")]
    IssueMismatch { expected: IssueId, actual: IssueId },
    #[error("cannot claim issue without attached workspace; run requested path {attempted:?}")]
    WorkspaceNotAttached { attempted: PathBuf },
    #[error(
        "workspace does not match issue identity: expected key {expected_key} at a path ending in {expected_key}, got key {actual_key} at {actual_path:?}"
    )]
    WorkspaceIssueMismatch {
        expected_key: WorkspaceKey,
        actual_key: WorkspaceKey,
        actual_path: PathBuf,
    },
    #[error(
        "workspace identity mismatch: expected key {expected_key} at {expected_path:?}, got key {actual_key} at {actual_path:?}"
    )]
    WorkspaceIdentityMismatch {
        expected_key: WorkspaceKey,
        actual_key: WorkspaceKey,
        expected_path: PathBuf,
        actual_path: PathBuf,
    },
    #[error("workspace path mismatch: expected {expected:?}, got {actual:?}")]
    WorkspacePathMismatch { expected: PathBuf, actual: PathBuf },
    #[error("cannot start running issue without conversation metadata")]
    ConversationNotAttached,
    #[error("worker mismatch: expected {expected}, got {actual}")]
    WorkerMismatch {
        expected: WorkerId,
        actual: WorkerId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "details")]
pub enum SchedulerState {
    Unclaimed {
        since: TimestampMs,
    },
    Claimed {
        run: RunAttempt,
    },
    Running {
        run: RunAttempt,
        stall: StallMetadata,
    },
    RetryQueued {
        retry: RetryEntry,
    },
    Released {
        released_at: TimestampMs,
        reason: ReleaseReason,
    },
}

impl SchedulerState {
    pub const fn status(&self) -> SchedulerStatus {
        match self {
            Self::Unclaimed { .. } => SchedulerStatus::Unclaimed,
            Self::Claimed { .. } => SchedulerStatus::Claimed,
            Self::Running { .. } => SchedulerStatus::Running,
            Self::RetryQueued { .. } => SchedulerStatus::RetryQueued,
            Self::Released { .. } => SchedulerStatus::Released,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueExecution {
    issue: NormalizedIssue,
    workspace: Option<WorkspaceRecord>,
    conversation: Option<ConversationMetadata>,
    state: SchedulerState,
    last_worker_outcome: Option<WorkerOutcomeRecord>,
    recent_worker_outcomes: Vec<WorkerOutcomeRecord>,
}

impl IssueExecution {
    pub fn new(issue: NormalizedIssue, observed_at: TimestampMs) -> Self {
        Self {
            issue,
            workspace: None,
            conversation: None,
            state: SchedulerState::Unclaimed { since: observed_at },
            last_worker_outcome: None,
            recent_worker_outcomes: Vec::new(),
        }
    }

    pub fn issue(&self) -> &NormalizedIssue {
        &self.issue
    }

    pub fn refresh_issue(&mut self, issue: NormalizedIssue) -> Result<(), StateTransitionError> {
        if issue.id != self.issue.id {
            return Err(StateTransitionError::IssueMismatch {
                expected: self.issue.id.clone(),
                actual: issue.id,
            });
        }

        self.issue = issue;
        Ok(())
    }

    pub fn workspace(&self) -> Option<&WorkspaceRecord> {
        self.workspace.as_ref()
    }

    pub fn state(&self) -> &SchedulerState {
        &self.state
    }

    pub fn status(&self) -> SchedulerStatus {
        self.state.status()
    }

    pub fn last_worker_outcome(&self) -> Option<&WorkerOutcomeRecord> {
        self.last_worker_outcome.as_ref()
    }

    pub fn recent_worker_outcomes(&self) -> &[WorkerOutcomeRecord] {
        &self.recent_worker_outcomes
    }

    pub fn current_run(&self) -> Option<&RunAttempt> {
        match &self.state {
            SchedulerState::Claimed { run } | SchedulerState::Running { run, .. } => Some(run),
            _ => None,
        }
    }

    pub fn conversation(&self) -> Option<&ConversationMetadata> {
        self.conversation.as_ref()
    }

    pub fn retry(&self) -> Option<&RetryEntry> {
        match &self.state {
            SchedulerState::RetryQueued { retry } => Some(retry),
            _ => None,
        }
    }

    pub fn attach_workspace(
        &mut self,
        workspace: WorkspaceRecord,
    ) -> Result<(), StateTransitionError> {
        if let Some(current) = &self.workspace {
            let expected_path = comparable_workspace_path(&current.path);
            let actual_path = comparable_workspace_path(&workspace.path);

            if current.workspace_key != workspace.workspace_key || expected_path != actual_path {
                return Err(StateTransitionError::WorkspaceIdentityMismatch {
                    expected_key: current.workspace_key.clone(),
                    actual_key: workspace.workspace_key.clone(),
                    expected_path,
                    actual_path,
                });
            }
        } else {
            let expected_key = issue_workspace_key(&self.issue);
            let actual_path = comparable_workspace_path(&workspace.path);

            if workspace.workspace_key != expected_key
                || !workspace_path_matches_key(&actual_path, &expected_key)
            {
                return Err(StateTransitionError::WorkspaceIssueMismatch {
                    expected_key,
                    actual_key: workspace.workspace_key.clone(),
                    actual_path,
                });
            }
        }

        self.workspace = Some(workspace);
        Ok(())
    }

    pub fn claim(mut self, run: RunAttempt) -> Result<Self, StateTransitionError> {
        self.validate_run_binding(&run)?;

        let normal_retry_count = match &self.state {
            SchedulerState::Unclaimed { .. } => {
                if run.attempt.is_some() {
                    return Err(StateTransitionError::AttemptMismatch {
                        expected: None,
                        actual: run.attempt,
                    });
                }
                0
            }
            SchedulerState::RetryQueued { retry } => {
                if run.attempt != Some(retry.attempt) {
                    return Err(StateTransitionError::AttemptMismatch {
                        expected: Some(retry.attempt),
                        actual: run.attempt,
                    });
                }
                retry.normal_retry_count
            }
            _ => {
                return Err(StateTransitionError::InvalidTransition {
                    from: self.status(),
                    action: TransitionAction::Claim,
                });
            }
        };

        self.state = SchedulerState::Claimed {
            run: run.with_normal_retry_count(normal_retry_count),
        };
        Ok(self)
    }

    pub fn start_running(
        mut self,
        started_at: TimestampMs,
        stall_timeout_ms: DurationMs,
        session: Option<ConversationMetadata>,
    ) -> Result<Self, StateTransitionError> {
        match self.state {
            SchedulerState::Claimed { run } => {
                if let Some(session) = session {
                    self.conversation = Some(session);
                }
                if self.conversation.is_none() {
                    return Err(StateTransitionError::ConversationNotAttached);
                }
                self.state = SchedulerState::Running {
                    run: run.mark_started(started_at),
                    stall: StallMetadata::new(started_at, stall_timeout_ms),
                };
                Ok(self)
            }
            _ => Err(StateTransitionError::InvalidTransition {
                from: self.status(),
                action: TransitionAction::StartRunning,
            }),
        }
    }

    pub fn record_turn_started(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<(), StateTransitionError> {
        match &mut self.state {
            SchedulerState::Running { run, stall, .. } => {
                run.record_turn_started();
                stall.observe_activity(observed_at);
                Ok(())
            }
            _ => Err(StateTransitionError::InvalidTransition {
                from: self.status(),
                action: TransitionAction::RecordTurnStarted,
            }),
        }
    }

    pub fn observe_runtime_event(
        &mut self,
        event_at: TimestampMs,
        event_id: Option<String>,
        event_kind: Option<String>,
        summary: Option<String>,
    ) -> Result<(), StateTransitionError> {
        match &mut self.state {
            SchedulerState::Running { stall, .. } => {
                if let Some(session) = &mut self.conversation {
                    session.observe_event(event_at, event_id, event_kind, summary);
                }
                stall.observe_activity(event_at);
                Ok(())
            }
            _ => Err(StateTransitionError::InvalidTransition {
                from: self.status(),
                action: TransitionAction::ObserveRuntimeEvent,
            }),
        }
    }

    pub fn queue_retry(
        mut self,
        retry: RetryEntry,
        outcome: WorkerOutcomeRecord,
    ) -> Result<Self, StateTransitionError> {
        self.validate_retry_binding(&retry)?;

        let expected_attempt = match &self.state {
            SchedulerState::Claimed { run } | SchedulerState::Running { run, .. } => {
                self.validate_outcome_binding(run, &outcome)?;
                RetryAttempt::after(run.attempt).ok()
            }
            _ => {
                return Err(StateTransitionError::InvalidTransition {
                    from: self.status(),
                    action: TransitionAction::QueueRetry,
                });
            }
        };

        if expected_attempt != Some(retry.attempt) {
            return Err(StateTransitionError::AttemptMismatch {
                expected: expected_attempt,
                actual: Some(retry.attempt),
            });
        }

        self.record_outcome(outcome);
        self.state = SchedulerState::RetryQueued { retry };
        Ok(self)
    }

    pub fn release(
        mut self,
        released_at: TimestampMs,
        reason: ReleaseReason,
        outcome: Option<WorkerOutcomeRecord>,
    ) -> Result<Self, StateTransitionError> {
        if matches!(self.state, SchedulerState::Released { .. }) {
            return Err(StateTransitionError::InvalidTransition {
                from: self.status(),
                action: TransitionAction::Release,
            });
        }

        if let Some(outcome) = outcome {
            self.record_outcome(outcome);
        }

        self.state = SchedulerState::Released {
            released_at,
            reason,
        };
        Ok(self)
    }

    pub fn reopen(mut self, observed_at: TimestampMs) -> Result<Self, StateTransitionError> {
        match self.state {
            SchedulerState::Released { reason, .. } => {
                if !reason.preserves_reactivation_state() {
                    self.workspace = None;
                    self.conversation = None;
                }
                self.state = SchedulerState::Unclaimed { since: observed_at };
                Ok(self)
            }
            _ => Err(StateTransitionError::InvalidTransition {
                from: self.status(),
                action: TransitionAction::Reopen,
            }),
        }
    }

    pub fn snapshot(&self) -> crate::IssueSnapshot {
        crate::IssueSnapshot::from(self)
    }

    fn record_outcome(&mut self, outcome: WorkerOutcomeRecord) {
        self.last_worker_outcome = Some(outcome.clone());
        if self.recent_worker_outcomes.len() == MAX_RECENT_WORKER_OUTCOMES {
            self.recent_worker_outcomes.remove(0);
        }
        self.recent_worker_outcomes.push(outcome);
    }

    fn validate_run_binding(&self, run: &RunAttempt) -> Result<(), StateTransitionError> {
        if run.issue_id != self.issue.id {
            return Err(StateTransitionError::IssueMismatch {
                expected: self.issue.id.clone(),
                actual: run.issue_id.clone(),
            });
        }

        let Some(workspace) = &self.workspace else {
            return Err(StateTransitionError::WorkspaceNotAttached {
                attempted: run.workspace_path.clone(),
            });
        };

        let expected = comparable_workspace_path(&workspace.path);
        let actual = comparable_workspace_path(&run.workspace_path);

        if expected != actual {
            return Err(StateTransitionError::WorkspacePathMismatch { expected, actual });
        }

        Ok(())
    }

    fn validate_retry_binding(&self, retry: &RetryEntry) -> Result<(), StateTransitionError> {
        if retry.issue_id != self.issue.id {
            return Err(StateTransitionError::IssueMismatch {
                expected: self.issue.id.clone(),
                actual: retry.issue_id.clone(),
            });
        }

        Ok(())
    }

    fn validate_outcome_binding(
        &self,
        run: &RunAttempt,
        outcome: &WorkerOutcomeRecord,
    ) -> Result<(), StateTransitionError> {
        if outcome.worker_id != run.worker_id {
            return Err(StateTransitionError::WorkerMismatch {
                expected: run.worker_id.clone(),
                actual: outcome.worker_id.clone(),
            });
        }

        if outcome.attempt != run.attempt {
            return Err(StateTransitionError::AttemptMismatch {
                expected: run.attempt,
                actual: outcome.attempt,
            });
        }

        Ok(())
    }
}

fn issue_workspace_key(issue: &NormalizedIssue) -> WorkspaceKey {
    WorkspaceKey::new(issue.identifier.as_str())
        .expect("normalized issue identifiers cannot be empty")
}

fn workspace_path_matches_key(path: &Path, key: &WorkspaceKey) -> bool {
    path.file_name()
        .is_some_and(|name| name == OsStr::new(key.as_str()))
}

fn comparable_workspace_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        std::fs::canonicalize(path).unwrap_or_else(|_| normalize_workspace_path(path))
    } else {
        normalize_workspace_path(path)
    }
}

fn normalize_workspace_path(path: &Path) -> PathBuf {
    let mut prefix: Option<OsString> = None;
    let mut has_root = false;
    let mut parts: Vec<OsString> = Vec::new();

    for component in path.components() {
        match component {
            Component::Prefix(component) => prefix = Some(component.as_os_str().to_os_string()),
            Component::RootDir => has_root = true,
            Component::CurDir => {}
            Component::ParentDir => match parts.last() {
                Some(last) if last != OsStr::new("..") => {
                    parts.pop();
                }
                _ if !has_root => parts.push(OsString::from("..")),
                _ => {}
            },
            Component::Normal(component) => parts.push(component.to_os_string()),
        }
    }

    let mut normalized = PathBuf::new();
    if let Some(prefix) = prefix {
        normalized.push(prefix);
    }
    if has_root {
        normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR));
    }
    for part in parts {
        normalized.push(part);
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}
