use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    future::Future,
};

use chrono::{DateTime, Utc};
use opensymphony_domain::{
    ComponentHealthSnapshot, ConversationMetadata, DaemonSnapshot, DurationMs, HealthStatus,
    IdentifierError, IssueExecution, IssueId, IssueIdentifier, IssueRef, IssueSnapshot, IssueState,
    IssueStateCategory, NormalizedIssue, OrchestratorSnapshot, ReleaseReason,
    RetryCalculationError, RetryEntry, RetryPolicy, RetryReason, RunAttempt, SchedulerStatus,
    StateTransitionError, TimestampMs, TrackerIssue, TrackerIssueStateSnapshot, TrackerStateId,
    WorkerId, WorkerOutcomeKind, WorkerOutcomeRecord, WorkspaceRecord,
};
use opensymphony_workflow::ResolvedWorkflow;
use thiserror::Error;
use tokio::{
    select,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, warn};

use crate::{filter_issues_for_dispatch, sort_issues_for_dispatch};

const DISABLED_STALL_TIMEOUT_MS: u64 = u64::MAX / 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerConfig {
    pub poll_interval_ms: u64,
    pub max_concurrent_agents: u32,
    pub max_turns: u32,
    pub max_concurrent_agents_by_state: BTreeMap<String, u32>,
    pub retry_policy: RetryPolicy,
    pub stall_timeout_ms: Option<u64>,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
}

impl SchedulerConfig {
    pub fn from_workflow(workflow: &ResolvedWorkflow) -> Result<Self, SchedulerError> {
        Ok(Self {
            poll_interval_ms: workflow.config.polling.interval_ms,
            max_concurrent_agents: u32::try_from(workflow.config.agent.max_concurrent_agents)
                .map_err(|_| SchedulerError::InvalidConfiguration {
                    detail: format!(
                        "workflow max_concurrent_agents {} exceeds u32::MAX ({})",
                        workflow.config.agent.max_concurrent_agents,
                        u32::MAX
                    ),
                })?,
            max_turns: u32::try_from(workflow.config.agent.max_turns).map_err(|_| {
                SchedulerError::InvalidConfiguration {
                    detail: format!(
                        "workflow max_turns {} exceeds u32::MAX ({})",
                        workflow.config.agent.max_turns,
                        u32::MAX
                    ),
                }
            })?,
            max_concurrent_agents_by_state: workflow
                .config
                .agent
                .max_concurrent_agents_by_state
                .iter()
                .map(|(state, limit)| {
                    u32::try_from(*limit)
                        .map(|limit| (state.clone(), limit))
                        .map_err(|_| SchedulerError::InvalidConfiguration {
                            detail: format!(
                                "workflow max_concurrent_agents_by_state[{state}] {limit} exceeds u32::MAX ({})",
                                u32::MAX
                            ),
                        })
                })
                .collect::<Result<_, _>>()?,
            retry_policy: RetryPolicy {
                max_backoff_ms: DurationMs::new(workflow.config.agent.max_retry_backoff_ms),
                ..RetryPolicy::default()
            },
            stall_timeout_ms: workflow.config.agent.stall_timeout_ms,
            active_states: workflow.config.tracker.active_states.clone(),
            terminal_states: workflow.config.tracker.terminal_states.clone(),
        })
    }

    fn terminal_state_set(&self) -> HashSet<String> {
        normalized_state_set(&self.terminal_states)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub issue: NormalizedIssue,
    pub workspace: WorkspaceRecord,
    pub had_in_flight_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStartRequest {
    pub issue: NormalizedIssue,
    pub workspace: WorkspaceRecord,
    pub run: RunAttempt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLaunch {
    pub conversation: ConversationMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerUpdate {
    RuntimeEvent {
        worker_id: WorkerId,
        observed_at: TimestampMs,
        event_id: Option<String>,
        event_kind: Option<String>,
        summary: Option<String>,
    },
    Finished {
        worker_id: WorkerId,
        outcome: WorkerOutcomeRecord,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerAbortReason {
    TrackerInactive,
    TrackerTerminal,
    Stalled,
}

#[allow(async_fn_in_trait)]
pub trait TrackerBackend {
    type Error: std::fmt::Display + Send + Sync + 'static;

    async fn candidate_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error>;
    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error>;
    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, Self::Error>;
}

#[allow(async_fn_in_trait)]
pub trait WorkspaceBackend {
    type Error: std::fmt::Display + Send + Sync + 'static;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        observed_at: TimestampMs,
    ) -> Result<WorkspaceRecord, Self::Error>;

    async fn recover_workspaces(&mut self) -> Result<Vec<RecoveryRecord>, Self::Error>;

    async fn cleanup_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error>;
}

#[allow(async_fn_in_trait)]
pub trait WorkerBackend {
    type Error: std::fmt::Display + Send + Sync + 'static;

    async fn start_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error>;

    async fn poll_updates(&mut self) -> Result<Vec<WorkerUpdate>, Self::Error>;

    async fn abort_worker(
        &mut self,
        worker_id: &WorkerId,
        reason: WorkerAbortReason,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("invalid scheduler configuration: {detail}")]
    InvalidConfiguration { detail: String },
    #[error("tracker backend failed: {detail}")]
    Tracker { detail: String },
    #[error("workspace backend failed: {detail}")]
    Workspace { detail: String },
    #[error("worker backend failed: {detail}")]
    Worker { detail: String },
    #[error(transparent)]
    StateTransition(#[from] StateTransitionError),
    #[error(transparent)]
    RetryCalculation(#[from] RetryCalculationError),
    #[error(transparent)]
    Identifier(#[from] IdentifierError),
}

pub struct Scheduler<T, W, M> {
    tracker: T,
    workspace: W,
    worker: M,
    config: SchedulerConfig,
    executions: BTreeMap<IssueId, IssueExecution>,
    worker_index: HashMap<WorkerId, IssueId>,
    pending_recovery: Option<Vec<RecoveryRecord>>,
    recovered: bool,
    next_worker_ordinal: u64,
    last_poll_at: Option<TimestampMs>,
    health: HealthStatus,
}

impl<T, W, M> Scheduler<T, W, M>
where
    T: TrackerBackend,
    W: WorkspaceBackend,
    M: WorkerBackend,
{
    pub fn new(tracker: T, workspace: W, worker: M, config: SchedulerConfig) -> Self {
        Self {
            tracker,
            workspace,
            worker,
            config,
            executions: BTreeMap::new(),
            worker_index: HashMap::new(),
            pending_recovery: None,
            recovered: false,
            next_worker_ordinal: 0,
            last_poll_at: None,
            health: HealthStatus::Starting,
        }
    }

    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    pub fn tracker(&self) -> &T {
        &self.tracker
    }

    pub fn tracker_mut(&mut self) -> &mut T {
        &mut self.tracker
    }

    pub fn workspace(&self) -> &W {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut W {
        &mut self.workspace
    }

    pub fn worker(&self) -> &M {
        &self.worker
    }

    pub fn worker_mut(&mut self) -> &mut M {
        &mut self.worker
    }

    pub fn executions(&self) -> &BTreeMap<IssueId, IssueExecution> {
        &self.executions
    }

    pub fn execution(&self, issue_id: &IssueId) -> Option<&IssueExecution> {
        self.executions.get(issue_id)
    }

    pub fn snapshot(&self, generated_at: TimestampMs) -> OrchestratorSnapshot {
        let daemon = DaemonSnapshot::new(
            self.health,
            self.config.poll_interval_ms,
            self.config.max_concurrent_agents,
            self.last_poll_at,
            ComponentHealthSnapshot::default(),
            Default::default(),
        );
        let mut issues = self
            .executions
            .values()
            .map(IssueSnapshot::from)
            .collect::<Vec<_>>();
        issues.sort_by(|left, right| left.issue.identifier.cmp(&right.issue.identifier));
        OrchestratorSnapshot::new(generated_at, daemon, issues)
    }

    pub async fn tick(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<OrchestratorSnapshot, SchedulerError> {
        if self.pending_recovery.is_none() {
            self.pending_recovery =
                Some(self.workspace.recover_workspaces().await.map_err(|error| {
                    SchedulerError::Workspace {
                        detail: error.to_string(),
                    }
                })?);
        }

        let updates = self
            .worker
            .poll_updates()
            .await
            .map_err(|error| SchedulerError::Worker {
                detail: error.to_string(),
            })?;
        self.apply_worker_updates(updates).await?;

        let tracker_snapshot = self.load_tracker_snapshot().await?;
        self.bootstrap_recovery(&tracker_snapshot, observed_at)
            .await?;
        self.reconcile_tracker_state(&tracker_snapshot, observed_at)
            .await?;
        self.handle_stalls(observed_at).await?;
        self.dispatch_ready_issues(&tracker_snapshot.active, observed_at)
            .await?;

        self.last_poll_at = Some(observed_at);
        self.health = HealthStatus::Healthy;
        Ok(self.snapshot(observed_at))
    }

    pub async fn run_until_shutdown<F>(&mut self, shutdown: F) -> Result<(), SchedulerError>
    where
        F: Future<Output = ()>,
    {
        let mut shutdown = std::pin::pin!(shutdown);
        let mut ticker = interval(std::time::Duration::from_millis(
            self.config.poll_interval_ms,
        ));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            select! {
                _ = shutdown.as_mut() => break,
                _ = ticker.tick() => {
                    let now = TimestampMs::new(current_epoch_millis());
                    if let Err(error) = self.tick(now).await {
                        self.health = HealthStatus::Degraded;
                        warn!(%error, "scheduler tick failed");
                    }
                }
            }
        }

        Ok(())
    }

    async fn load_tracker_snapshot(&mut self) -> Result<TrackerSnapshot, SchedulerError> {
        let active =
            self.tracker
                .candidate_issues()
                .await
                .map_err(|error| SchedulerError::Tracker {
                    detail: error.to_string(),
                })?;
        let terminal =
            self.tracker
                .terminal_issues()
                .await
                .map_err(|error| SchedulerError::Tracker {
                    detail: error.to_string(),
                })?;

        let active_ids = active
            .iter()
            .map(|issue| issue.id.clone())
            .collect::<HashSet<_>>();
        let terminal_ids = terminal
            .iter()
            .map(|issue| issue.id.clone())
            .collect::<HashSet<_>>();

        let mut lookup_ids = self
            .executions
            .keys()
            .map(|id| id.as_str().to_string())
            .collect::<BTreeSet<_>>();
        if let Some(records) = &self.pending_recovery {
            lookup_ids.extend(
                records
                    .iter()
                    .map(|record| record.issue.id.as_str().to_string()),
            );
        }
        lookup_ids.retain(|id| !active_ids.contains(id) && !terminal_ids.contains(id));

        let state_by_id = if lookup_ids.is_empty() {
            HashMap::new()
        } else {
            self.tracker
                .issue_states_by_ids(&lookup_ids.into_iter().collect::<Vec<_>>())
                .await
                .map_err(|error| SchedulerError::Tracker {
                    detail: error.to_string(),
                })?
                .into_iter()
                .map(|snapshot| (snapshot.id.clone(), snapshot))
                .collect()
        };

        Ok(TrackerSnapshot {
            active_by_id: active
                .iter()
                .cloned()
                .map(|issue| (issue.id.clone(), issue))
                .collect(),
            terminal_by_id: terminal
                .iter()
                .cloned()
                .map(|issue| (issue.id.clone(), issue))
                .collect(),
            state_by_id,
            active,
        })
    }

    async fn bootstrap_recovery(
        &mut self,
        tracker_snapshot: &TrackerSnapshot,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        if self.recovered {
            return Ok(());
        }

        let Some(records) = self.pending_recovery.take() else {
            self.recovered = true;
            return Ok(());
        };

        for record in records {
            let issue_id = record.issue.id.clone();
            if let Some(active_issue) = tracker_snapshot.active_by_id.get(issue_id.as_str()) {
                let normalized = normalize_tracker_issue(active_issue, &self.config)?;
                self.upsert_active_execution(normalized, observed_at, Some(record.workspace))?;
                continue;
            }

            if tracker_snapshot
                .terminal_by_id
                .contains_key(issue_id.as_str())
            {
                self.workspace
                    .cleanup_workspace(&record.workspace, true)
                    .await
                    .map_err(|error| SchedulerError::Workspace {
                        detail: error.to_string(),
                    })?;
                continue;
            }

            let mut issue = record.issue.clone();
            if let Some(snapshot) = tracker_snapshot.state_by_id.get(issue_id.as_str()) {
                issue.state = issue_state_from_name(&snapshot.state.name, &self.config);
            }

            let mut execution = IssueExecution::new(issue.clone(), observed_at);
            execution.attach_workspace(record.workspace)?;
            let execution = execution.release(observed_at, ReleaseReason::TrackerInactive, None)?;
            self.executions.entry(issue.id.clone()).or_insert(execution);
        }

        self.recovered = true;
        Ok(())
    }

    async fn reconcile_tracker_state(
        &mut self,
        tracker_snapshot: &TrackerSnapshot,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        for tracker_issue in tracker_snapshot.active_by_id.values() {
            let normalized = normalize_tracker_issue(tracker_issue, &self.config)?;
            self.upsert_active_execution(normalized, observed_at, None)?;
        }

        let existing_ids = self.executions.keys().cloned().collect::<Vec<_>>();
        for issue_id in existing_ids {
            if tracker_snapshot
                .active_by_id
                .contains_key(issue_id.as_str())
            {
                continue;
            }

            if let Some(tracker_issue) = tracker_snapshot.terminal_by_id.get(issue_id.as_str()) {
                let normalized = normalize_tracker_issue(tracker_issue, &self.config)?;
                self.release_issue(
                    issue_id.clone(),
                    normalized,
                    observed_at,
                    ReleaseReason::TrackerTerminal,
                    true,
                    Some(WorkerAbortReason::TrackerTerminal),
                )
                .await?;
                continue;
            }

            if let Some(snapshot) = tracker_snapshot.state_by_id.get(issue_id.as_str()) {
                let category = state_category_from_name(&snapshot.state.name, &self.config);
                if category == IssueStateCategory::Active {
                    continue;
                }

                let normalized = if let Some(existing) = self.executions.get(&issue_id) {
                    let mut issue = existing.issue().clone();
                    issue.state = issue_state_from_name(&snapshot.state.name, &self.config);
                    issue
                } else {
                    minimal_issue_from_state_snapshot(snapshot, &self.config)?
                };

                let (reason, cleanup, abort_reason) = match category {
                    IssueStateCategory::Terminal => (
                        ReleaseReason::TrackerTerminal,
                        true,
                        Some(WorkerAbortReason::TrackerTerminal),
                    ),
                    IssueStateCategory::NonActive => (
                        ReleaseReason::TrackerInactive,
                        false,
                        Some(WorkerAbortReason::TrackerInactive),
                    ),
                    IssueStateCategory::Active => continue,
                };
                self.release_issue(
                    issue_id.clone(),
                    normalized,
                    observed_at,
                    reason,
                    cleanup,
                    abort_reason,
                )
                .await?;
            }
        }

        Ok(())
    }

    async fn dispatch_ready_issues(
        &mut self,
        active_issues: &[TrackerIssue],
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let mut ready =
            filter_issues_for_dispatch(active_issues.to_vec(), &self.config.terminal_state_set());
        sort_issues_for_dispatch(&mut ready);

        for tracker_issue in ready {
            if self.worker_index.len()
                >= usize::try_from(self.config.max_concurrent_agents).unwrap_or(usize::MAX)
            {
                break;
            }

            let normalized = normalize_tracker_issue(&tracker_issue, &self.config)?;
            let issue_id = normalized.id.clone();
            let should_dispatch = match self.executions.get(&issue_id) {
                Some(execution) => match execution.status() {
                    SchedulerStatus::Unclaimed => true,
                    SchedulerStatus::RetryQueued => execution
                        .retry()
                        .is_some_and(|retry| retry.due_at <= observed_at),
                    SchedulerStatus::Released => false,
                    SchedulerStatus::Claimed | SchedulerStatus::Running => false,
                },
                None => true,
            };
            if !should_dispatch {
                continue;
            }

            if let Some(limit) = state_limit_for(
                &self.config.max_concurrent_agents_by_state,
                &normalized.state.name,
            ) {
                let running_in_state = self
                    .executions
                    .values()
                    .filter(|execution| {
                        execution.status() == SchedulerStatus::Running
                            && execution
                                .issue()
                                .state
                                .name
                                .eq_ignore_ascii_case(&normalized.state.name)
                    })
                    .count();
                if running_in_state >= usize::try_from(limit).unwrap_or(usize::MAX) {
                    continue;
                }
            }

            let workspace = self
                .workspace
                .ensure_workspace(&normalized, observed_at)
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?;

            let issue_id = normalized.id.clone();
            let worker_id = self.next_worker_id()?;
            let previous_retry = self
                .executions
                .get(&issue_id)
                .and_then(IssueExecution::retry)
                .map(|retry| retry.attempt);
            let run = RunAttempt::new(
                worker_id.clone(),
                normalized.id.clone(),
                normalized.identifier.clone(),
                workspace.path.clone(),
                observed_at,
                previous_retry,
                self.config.max_turns,
            );

            let mut execution = self
                .executions
                .remove(&issue_id)
                .unwrap_or_else(|| IssueExecution::new(normalized.clone(), observed_at));
            execution.refresh_issue(normalized.clone())?;
            execution.attach_workspace(workspace.clone())?;
            execution = execution.claim(run.clone())?;
            let claimed_run = execution
                .current_run()
                .cloned()
                .expect("claimed execution must expose the claimed run");

            let start_request = WorkerStartRequest {
                issue: normalized.clone(),
                workspace,
                run: claimed_run.clone(),
            };
            match self.worker.start_worker(start_request).await {
                Ok(launch) => {
                    execution = execution.start_running(
                        observed_at,
                        effective_stall_timeout(self.config.stall_timeout_ms),
                        Some(launch.conversation),
                    )?;
                    execution.record_turn_started(observed_at)?;
                    self.worker_index.insert(worker_id, issue_id.clone());
                    debug!(issue_id = %issue_id, "dispatched scheduler worker");
                }
                Err(error) => {
                    let detail = error.to_string();
                    let outcome = WorkerOutcomeRecord::from_run(
                        &claimed_run,
                        WorkerOutcomeKind::Failed,
                        observed_at,
                        Some("failed to start worker".to_string()),
                        Some(detail),
                    );
                    execution = self.resolve_finished_execution(execution, outcome, observed_at)?;
                }
            }

            self.executions.insert(issue_id, execution);
        }

        Ok(())
    }

    async fn apply_worker_updates(
        &mut self,
        updates: Vec<WorkerUpdate>,
    ) -> Result<(), SchedulerError> {
        for update in updates {
            match update {
                WorkerUpdate::RuntimeEvent {
                    worker_id,
                    observed_at,
                    event_id,
                    event_kind,
                    summary,
                } => {
                    let Some(issue_id) = self.worker_index.get(&worker_id).cloned() else {
                        continue;
                    };
                    if let Some(execution) = self.executions.get_mut(&issue_id) {
                        execution.observe_runtime_event(
                            observed_at,
                            event_id,
                            event_kind,
                            summary,
                        )?;
                    }
                }
                WorkerUpdate::Finished { worker_id, outcome } => {
                    let Some(issue_id) = self.worker_index.remove(&worker_id) else {
                        continue;
                    };
                    let Some(execution) = self.executions.remove(&issue_id) else {
                        continue;
                    };
                    let finished_at = outcome.finished_at;
                    let execution =
                        self.resolve_finished_execution(execution, outcome, finished_at)?;
                    self.executions.insert(issue_id, execution);
                }
            }
        }

        Ok(())
    }

    async fn handle_stalls(&mut self, observed_at: TimestampMs) -> Result<(), SchedulerError> {
        if self.config.stall_timeout_ms.is_none() {
            return Ok(());
        }

        let stalled = self
            .executions
            .iter()
            .filter_map(|(issue_id, execution)| match execution.state() {
                opensymphony_domain::SchedulerState::Running { stall, .. }
                    if stall.stalled_at <= observed_at =>
                {
                    Some(issue_id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for issue_id in stalled {
            let Some(mut execution) = self.executions.remove(&issue_id) else {
                continue;
            };
            let Some(run) = execution.current_run().cloned() else {
                self.executions.insert(issue_id, execution);
                continue;
            };

            self.abort_worker(&run.worker_id, WorkerAbortReason::Stalled)
                .await?;
            let outcome = WorkerOutcomeRecord::from_run(
                &run,
                WorkerOutcomeKind::Stalled,
                observed_at,
                Some("worker exceeded the configured stall timeout".to_string()),
                Some("scheduler stall timeout reached".to_string()),
            );
            execution = self.resolve_finished_execution(execution, outcome, observed_at)?;
            self.executions.insert(issue_id, execution);
        }

        Ok(())
    }

    fn upsert_active_execution(
        &mut self,
        issue: NormalizedIssue,
        observed_at: TimestampMs,
        recovered_workspace: Option<WorkspaceRecord>,
    ) -> Result<(), SchedulerError> {
        let issue_id = issue.id.clone();
        let mut execution = match self.executions.remove(&issue_id) {
            Some(existing) => existing,
            None => IssueExecution::new(issue.clone(), observed_at),
        };
        if execution.status() == SchedulerStatus::Released {
            execution = execution.reopen(observed_at)?;
        }
        execution.refresh_issue(issue.clone())?;
        if let Some(workspace) = recovered_workspace {
            execution.attach_workspace(workspace)?;
        }
        self.executions.insert(issue_id, execution);
        Ok(())
    }

    async fn release_issue(
        &mut self,
        issue_id: IssueId,
        issue: NormalizedIssue,
        observed_at: TimestampMs,
        reason: ReleaseReason,
        cleanup_terminal: bool,
        abort_reason: Option<WorkerAbortReason>,
    ) -> Result<(), SchedulerError> {
        let Some(mut execution) = self.executions.remove(&issue_id) else {
            return Ok(());
        };

        execution.refresh_issue(issue)?;
        if let Some(run) = execution.current_run().cloned()
            && let Some(abort_reason) = abort_reason
        {
            self.abort_worker(&run.worker_id, abort_reason).await?;
        }
        if execution.status() != SchedulerStatus::Released {
            execution = execution.release(observed_at, reason, None)?;
        }
        if cleanup_terminal && let Some(workspace) = execution.workspace().cloned() {
            self.workspace
                .cleanup_workspace(&workspace, true)
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?;
        }
        self.executions.insert(issue_id, execution);
        Ok(())
    }

    async fn abort_worker(
        &mut self,
        worker_id: &WorkerId,
        reason: WorkerAbortReason,
    ) -> Result<(), SchedulerError> {
        self.worker_index.remove(worker_id);
        self.worker
            .abort_worker(worker_id, reason)
            .await
            .map_err(|error| SchedulerError::Worker {
                detail: error.to_string(),
            })
    }

    fn resolve_finished_execution(
        &self,
        execution: IssueExecution,
        outcome: WorkerOutcomeRecord,
        observed_at: TimestampMs,
    ) -> Result<IssueExecution, SchedulerError> {
        let active = execution.issue().state.category == IssueStateCategory::Active;
        if !active {
            let reason = match execution.issue().state.category {
                IssueStateCategory::Terminal => ReleaseReason::TrackerTerminal,
                IssueStateCategory::NonActive => ReleaseReason::TrackerInactive,
                IssueStateCategory::Active => ReleaseReason::Completed,
            };
            return Ok(execution.release(observed_at, reason, Some(outcome))?);
        }

        match outcome.outcome {
            WorkerOutcomeKind::Succeeded => {
                let run = execution
                    .current_run()
                    .expect("running execution must have a run");
                let retry = RetryEntry::continuation(
                    execution.issue(),
                    run.attempt,
                    run.normal_retry_count,
                    observed_at,
                    self.config.retry_policy,
                )?;
                Ok(execution.queue_retry(retry, outcome)?)
            }
            WorkerOutcomeKind::Failed | WorkerOutcomeKind::TimedOut => {
                let run = execution
                    .current_run()
                    .expect("running execution must have a run");
                let retry = RetryEntry::failure(
                    execution.issue(),
                    run.attempt,
                    run.normal_retry_count,
                    observed_at,
                    RetryReason::Failure,
                    outcome.error.clone().or(outcome.summary.clone()),
                    self.config.retry_policy,
                )?;
                Ok(execution.queue_retry(retry, outcome)?)
            }
            WorkerOutcomeKind::Stalled => {
                let run = execution
                    .current_run()
                    .expect("running execution must have a run");
                let retry = RetryEntry::failure(
                    execution.issue(),
                    run.attempt,
                    run.normal_retry_count,
                    observed_at,
                    RetryReason::Stalled,
                    outcome.error.clone().or(outcome.summary.clone()),
                    self.config.retry_policy,
                )?;
                Ok(execution.queue_retry(retry, outcome)?)
            }
            WorkerOutcomeKind::Cancelled => {
                let run = execution
                    .current_run()
                    .expect("running execution must have a run");
                let retry = RetryEntry::failure(
                    execution.issue(),
                    run.attempt,
                    run.normal_retry_count,
                    observed_at,
                    RetryReason::Cancelled,
                    outcome.error.clone().or(outcome.summary.clone()),
                    self.config.retry_policy,
                )?;
                Ok(execution.queue_retry(retry, outcome)?)
            }
        }
    }

    fn next_worker_id(&mut self) -> Result<WorkerId, SchedulerError> {
        self.next_worker_ordinal = self.next_worker_ordinal.saturating_add(1);
        WorkerId::new(format!("scheduler-worker-{}", self.next_worker_ordinal))
            .map_err(SchedulerError::Identifier)
    }
}

struct TrackerSnapshot {
    active: Vec<TrackerIssue>,
    active_by_id: HashMap<String, TrackerIssue>,
    terminal_by_id: HashMap<String, TrackerIssue>,
    state_by_id: HashMap<String, TrackerIssueStateSnapshot>,
}

fn normalize_tracker_issue(
    issue: &TrackerIssue,
    config: &SchedulerConfig,
) -> Result<NormalizedIssue, SchedulerError> {
    Ok(NormalizedIssue {
        id: IssueId::new(issue.id.clone())?,
        identifier: IssueIdentifier::new(issue.identifier.clone())?,
        title: issue.title.clone(),
        description: issue.description.clone(),
        priority: issue.priority,
        state: issue_state_from_name(&issue.state, config),
        branch_name: None,
        url: Some(issue.url.clone()),
        labels: issue.labels.clone(),
        parent_id: match &issue.parent_id {
            Some(parent_id) => Some(IssueId::new(parent_id.clone())?),
            None => None,
        },
        blocked_by: issue
            .blocked_by
            .iter()
            .map(|blocker| {
                Ok(opensymphony_domain::BlockerRef {
                    id: Some(IssueId::new(blocker.id.clone())?),
                    identifier: Some(IssueIdentifier::new(blocker.identifier.clone())?),
                    state: Some(blocker.state.name.clone()),
                    created_at: None,
                    updated_at: None,
                })
            })
            .collect::<Result<Vec<_>, SchedulerError>>()?,
        sub_issues: issue
            .sub_issues
            .iter()
            .map(|child| {
                Ok(IssueRef {
                    id: IssueId::new(child.id.clone())?,
                    identifier: IssueIdentifier::new(child.identifier.clone())?,
                    state: child.state.clone(),
                })
            })
            .collect::<Result<Vec<_>, SchedulerError>>()?,
        created_at: Some(datetime_to_timestamp(issue.created_at)),
        updated_at: Some(datetime_to_timestamp(issue.updated_at)),
    })
}

fn minimal_issue_from_state_snapshot(
    snapshot: &TrackerIssueStateSnapshot,
    config: &SchedulerConfig,
) -> Result<NormalizedIssue, SchedulerError> {
    Ok(NormalizedIssue {
        id: IssueId::new(snapshot.id.clone())?,
        identifier: IssueIdentifier::new(snapshot.identifier.clone())?,
        title: snapshot.identifier.clone(),
        description: None,
        priority: None,
        state: issue_state_from_name(&snapshot.state.name, config),
        branch_name: None,
        url: None,
        labels: Vec::new(),
        parent_id: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: None,
        updated_at: Some(datetime_to_timestamp(snapshot.updated_at)),
    })
}

fn issue_state_from_name(name: &str, config: &SchedulerConfig) -> IssueState {
    IssueState {
        id: TrackerStateId::new(name.to_ascii_lowercase().replace(' ', "-")).ok(),
        name: name.to_string(),
        category: state_category_from_name(name, config),
    }
}

fn state_category_from_name(name: &str, config: &SchedulerConfig) -> IssueStateCategory {
    if matches_state_name(name, &config.terminal_states) {
        IssueStateCategory::Terminal
    } else if matches_state_name(name, &config.active_states) {
        IssueStateCategory::Active
    } else {
        IssueStateCategory::NonActive
    }
}

fn state_limit_for(limits: &BTreeMap<String, u32>, state_name: &str) -> Option<u32> {
    limits
        .iter()
        .find_map(|(state, limit)| state.eq_ignore_ascii_case(state_name).then_some(*limit))
}

fn normalized_state_set(states: &[String]) -> HashSet<String> {
    states
        .iter()
        .map(|state| state.trim().to_ascii_lowercase())
        .collect()
}

fn matches_state_name(name: &str, states: &[String]) -> bool {
    let name = name.trim().to_ascii_lowercase();
    states
        .iter()
        .any(|state| state.trim().eq_ignore_ascii_case(&name))
}

fn effective_stall_timeout(stall_timeout_ms: Option<u64>) -> DurationMs {
    DurationMs::new(stall_timeout_ms.unwrap_or(DISABLED_STALL_TIMEOUT_MS))
}

fn datetime_to_timestamp(datetime: DateTime<Utc>) -> TimestampMs {
    let millis = datetime.timestamp_millis();
    if millis <= 0 {
        TimestampMs::new(0)
    } else {
        TimestampMs::new(millis as u64)
    }
}

fn current_epoch_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
