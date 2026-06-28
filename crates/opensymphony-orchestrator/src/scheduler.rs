use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    future::Future,
    time::Duration,
};

use crate::opensymphony_domain::{
    ComponentHealthSnapshot, ConversationMetadata, DaemonSnapshot, DurationMs,
    HarnessInterruptCommand, HarnessInterruptExpectedNextState, HarnessInterruptReason,
    HealthStatus, IdentifierError, IssueExecution, IssueId, IssueIdentifier, IssueRef,
    IssueSnapshot, IssueState, IssueStateCategory, NormalizedIssue, OrchestratorSnapshot,
    ReleaseReason, RetryCalculationError, RetryEntry, RetryPolicy, RetryReason, RunAttempt,
    RuntimeUsageTotals, SchedulerStatus, StateTransitionError, TimestampMs, TrackerErrorCategory,
    TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot, TrackerIssueSummary, TrackerStateId, WorkerId, WorkerOutcomeKind,
    WorkerOutcomeRecord, WorkspaceRecord,
};
use crate::opensymphony_gateway_schema::capability::{HarnessCapability, HarnessKind};
use crate::opensymphony_workflow::{ResolvedWorkflow, RoutingConfig};
use chrono::{DateTime, Utc};
use thiserror::Error;
use tokio::{
    select,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, warn};

use super::filter_issues_for_dispatch;

const DISABLED_STALL_TIMEOUT_MS: u64 = u64::MAX / 4;
const ROUTING_TASK_ISSUE_EXECUTION: &str = "issue_execution";
const RUNNING_STATE_REFRESH_INTERVAL_MS: u64 = 30_000;
const DISPATCH_DISCOVERY_INTERVAL_MS: u64 = 60_000;
const TERMINAL_REFRESH_INTERVAL_MS: u64 = 300_000;
const FULL_DETAIL_REFRESH_INTERVAL_MS: u64 = 3_600_000;
const HUMAN_REVIEW_STATE: &str = "human review";
const MERGING_STATE: &str = "merging";

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
    pub routing: RoutingConfig,
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
                    let normalized_state = normalized_state_name(state);
                    u32::try_from(*limit)
                        .map(|limit| (normalized_state, limit))
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
            routing: workflow.config.routing.clone(),
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
    pub harness_kind: Option<String>,
    pub recovered_run: Option<RecoveredRun>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredRun {
    pub worker_id: WorkerId,
    pub conversation: ConversationMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStartRequest {
    pub issue: NormalizedIssue,
    pub workspace: WorkspaceRecord,
    pub run: RunAttempt,
    pub route: HarnessRouteDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessRouteDecision {
    pub task_type: String,
    pub harness_kind: String,
    pub model: Option<String>,
    pub model_profile: Option<String>,
    pub reason: String,
    pub dry_run: bool,
    pub user_override: bool,
}

impl HarnessRouteDecision {
    pub fn summary(&self) -> String {
        let profile = self
            .model_profile
            .as_deref()
            .unwrap_or("<default model profile>");
        let model = self.model.as_deref().unwrap_or("<harness default model>");
        let mode = if self.dry_run { "dry-run " } else { "" };
        format!(
            "{mode}selected harness `{}` with model `{model}` and profile `{profile}`: {}",
            self.harness_kind, self.reason
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLaunch {
    pub conversation: ConversationMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum WorkerUpdate {
    RuntimeEvent {
        worker_id: WorkerId,
        observed_at: TimestampMs,
        event_id: Option<String>,
        event_kind: Option<String>,
        summary: Option<String>,
        payload: Option<serde_json::Value>,
    },
    ConversationMetadataUpdate {
        worker_id: WorkerId,
        conversation: ConversationMetadata,
    },
    TokenUsageUpdate {
        worker_id: WorkerId,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        total_tokens: u64,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerInterruptAcknowledgement {
    pub accepted: bool,
    pub detail: Option<String>,
}

#[allow(async_fn_in_trait)]
pub trait TrackerBackend {
    type Error: std::fmt::Display + Send + Sync + 'static;

    async fn candidate_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error>;
    async fn candidate_issue_summaries(&mut self) -> Result<Vec<TrackerIssueSummary>, Self::Error> {
        Ok(self
            .candidate_issues()
            .await?
            .into_iter()
            .map(tracker_issue_summary_from_issue)
            .collect())
    }
    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error>;
    async fn issues_by_identifiers(
        &mut self,
        identifiers: &[String],
    ) -> Result<Vec<TrackerIssue>, Self::Error> {
        let requested = identifiers
            .iter()
            .map(|identifier| identifier.to_ascii_uppercase())
            .collect::<HashSet<_>>();
        Ok(self
            .candidate_issues()
            .await?
            .into_iter()
            .filter(|issue| requested.contains(&issue.identifier.to_ascii_uppercase()))
            .collect())
    }
    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, Self::Error>;
    fn error_category(_error: &Self::Error) -> Option<TrackerErrorCategory> {
        None
    }
    fn retry_after(_error: &Self::Error) -> Option<Duration> {
        None
    }
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

    async fn start_workers(
        &mut self,
        requests: Vec<WorkerStartRequest>,
    ) -> Vec<Result<WorkerLaunch, Self::Error>> {
        let mut launches = Vec::with_capacity(requests.len());
        for request in requests {
            launches.push(self.start_worker(request).await);
        }
        launches
    }

    async fn poll_updates(&mut self) -> Result<Vec<WorkerUpdate>, Self::Error>;

    async fn abort_worker(
        &mut self,
        worker_id: &WorkerId,
        reason: WorkerAbortReason,
    ) -> Result<(), Self::Error>;

    async fn interrupt_worker(
        &mut self,
        command: HarnessInterruptCommand,
    ) -> Result<WorkerInterruptAcknowledgement, Self::Error> {
        Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some(format!(
                "harness `{}` does not expose a scheduler-side interrupt channel",
                command.harness_kind
            )),
        })
    }
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
    running_counts_by_state: HashMap<String, usize>,
    worker_index: HashMap<WorkerId, IssueId>,
    worker_harness_kind: HashMap<WorkerId, String>,
    pending_recovery: Option<Vec<RecoveryRecord>>,
    recovered: bool,
    next_worker_ordinal: u64,
    last_poll_at: Option<TimestampMs>,
    last_running_state_refresh_at: Option<TimestampMs>,
    last_dispatch_discovery_at: Option<TimestampMs>,
    last_terminal_refresh_at: Option<TimestampMs>,
    last_full_detail_refresh_at: Option<TimestampMs>,
    linear_blocked_until: Option<TimestampMs>,
    health: HealthStatus,
}

enum DispatchCandidates {
    Full(Vec<TrackerIssue>),
    Summary(Vec<TrackerIssueSummary>),
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
            running_counts_by_state: HashMap::new(),
            worker_index: HashMap::new(),
            worker_harness_kind: HashMap::new(),
            pending_recovery: None,
            recovered: false,
            next_worker_ordinal: 0,
            last_poll_at: None,
            last_running_state_refresh_at: None,
            last_dispatch_discovery_at: None,
            last_terminal_refresh_at: None,
            last_full_detail_refresh_at: None,
            linear_blocked_until: None,
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
        let mut issues = self
            .executions
            .values()
            .map(IssueSnapshot::from)
            .collect::<Vec<_>>();
        issues.sort_by(|left, right| left.issue.identifier.cmp(&right.issue.identifier));

        // Aggregate token usage from all issues
        let total_input_tokens: u64 = issues
            .iter()
            .filter_map(|issue| issue.conversation.as_ref())
            .map(|conversation| conversation.input_tokens)
            .sum();
        let total_output_tokens: u64 = issues
            .iter()
            .filter_map(|issue| issue.conversation.as_ref())
            .map(|conversation| conversation.output_tokens)
            .sum();
        let total_cache_read_tokens: u64 = issues
            .iter()
            .filter_map(|issue| issue.conversation.as_ref())
            .map(|conversation| conversation.cache_read_tokens)
            .sum();
        let total_tokens: u64 = issues
            .iter()
            .filter_map(|issue| issue.conversation.as_ref())
            .map(|conversation| conversation.effective_total_tokens())
            .sum();

        let daemon = DaemonSnapshot::new(
            self.health,
            self.config.poll_interval_ms,
            self.config.max_concurrent_agents,
            self.last_poll_at,
            ComponentHealthSnapshot::default(),
            RuntimeUsageTotals {
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
                cache_read_tokens: total_cache_read_tokens,
                total_tokens,
                runtime_seconds: 0,
                estimated_cost_usd_micros: None,
            },
        );

        OrchestratorSnapshot::new(generated_at, daemon, issues)
    }

    pub async fn bootstrap(
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

        if let Some(tracker_snapshot) = self.load_tracker_snapshot(observed_at).await? {
            self.record_full_detail_refresh(observed_at);
            self.bootstrap_recovery(&tracker_snapshot, observed_at)
                .await?;
            self.reconcile_tracker_state(&tracker_snapshot, observed_at)
                .await?;
        }

        self.last_poll_at = Some(observed_at);
        self.refresh_health_from_linear_cooldown(observed_at);
        Ok(self.snapshot(observed_at))
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

        self.expire_linear_cooldown(observed_at);
        let mut dispatch_candidates = None;
        if !self.linear_cooldown_active(observed_at) {
            if due(
                self.last_full_detail_refresh_at,
                FULL_DETAIL_REFRESH_INTERVAL_MS,
                observed_at,
            ) {
                if let Some(tracker_snapshot) = self.load_tracker_snapshot(observed_at).await? {
                    self.record_full_detail_refresh(observed_at);
                    self.bootstrap_recovery(&tracker_snapshot, observed_at)
                        .await?;
                    self.reconcile_tracker_state(&tracker_snapshot, observed_at)
                        .await?;
                    dispatch_candidates = Some(DispatchCandidates::Full(tracker_snapshot.active));
                }
            } else if due(
                self.last_terminal_refresh_at,
                TERMINAL_REFRESH_INTERVAL_MS,
                observed_at,
            ) {
                self.refresh_terminal_issues(observed_at).await?;
            } else if due(
                self.last_running_state_refresh_at,
                RUNNING_STATE_REFRESH_INTERVAL_MS,
                observed_at,
            ) && self.has_running_executions()
            {
                self.refresh_running_issue_states(observed_at).await?;
            } else if due(
                self.last_dispatch_discovery_at,
                DISPATCH_DISCOVERY_INTERVAL_MS,
                observed_at,
            ) {
                dispatch_candidates = self
                    .load_dispatch_candidates(observed_at)
                    .await?
                    .map(DispatchCandidates::Summary);
            }
        }

        if let Some(candidates) = dispatch_candidates {
            match candidates {
                DispatchCandidates::Full(candidates) => {
                    self.dispatch_ready_issues(&candidates, observed_at).await?;
                }
                DispatchCandidates::Summary(candidates) => {
                    self.dispatch_summary_candidates(&candidates, observed_at)
                        .await?;
                }
            }
        }

        self.handle_stalls(observed_at).await?;

        let known_candidates = self.known_dispatch_candidates(observed_at);
        if !known_candidates.is_empty() {
            self.dispatch_ready_issues(&known_candidates, observed_at)
                .await?;
        }

        self.last_poll_at = Some(observed_at);
        self.refresh_health_from_linear_cooldown(observed_at);
        Ok(self.snapshot(observed_at))
    }

    pub async fn run_until_shutdown<F>(&mut self, shutdown: F) -> Result<(), SchedulerError>
    where
        F: Future<Output = ()>,
    {
        let mut shutdown = std::pin::pin!(shutdown);
        let mut ticker = interval(Duration::from_millis(self.config.poll_interval_ms));
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

    async fn load_tracker_snapshot(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<Option<TrackerSnapshot>, SchedulerError> {
        let active = match self.tracker.candidate_issues().await {
            Ok(active) => active,
            Err(error) => {
                if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                    return Ok(None);
                }
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };
        let terminal = match self.tracker.terminal_issues().await {
            Ok(terminal) => terminal,
            Err(error) => {
                if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                    return Ok(None);
                }
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };

        let active_ids = active
            .iter()
            .map(|issue| issue.id.as_str())
            .collect::<HashSet<_>>();
        let terminal_ids = terminal
            .iter()
            .map(|issue| issue.id.as_str())
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
        lookup_ids
            .retain(|id| !active_ids.contains(id.as_str()) && !terminal_ids.contains(id.as_str()));

        let active_index = active
            .iter()
            .enumerate()
            .map(|(index, issue)| (issue.id.clone(), index))
            .collect();
        let terminal_state_by_id = terminal
            .into_iter()
            .map(|issue| (issue.id, issue.state))
            .collect();

        let state_by_id = if lookup_ids.is_empty() {
            HashMap::new()
        } else {
            let snapshots = self
                .tracker
                .issue_states_by_ids(&lookup_ids.into_iter().collect::<Vec<_>>())
                .await;
            match snapshots {
                Ok(snapshots) => snapshots
                    .into_iter()
                    .map(|snapshot| (snapshot.id.clone(), snapshot))
                    .collect(),
                Err(error) => {
                    if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                        return Ok(None);
                    }
                    return Err(SchedulerError::Tracker {
                        detail: error.to_string(),
                    });
                }
            }
        };

        Ok(Some(TrackerSnapshot {
            active_index,
            terminal_state_by_id,
            state_by_id,
            active,
        }))
    }

    async fn refresh_running_issue_states(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let issue_ids = self
            .executions
            .iter()
            .filter(|(_, execution)| {
                matches!(
                    execution.status(),
                    SchedulerStatus::Claimed | SchedulerStatus::Running
                )
            })
            .map(|(id, _)| id.as_str().to_string())
            .collect::<Vec<_>>();
        if issue_ids.is_empty() {
            self.last_running_state_refresh_at = Some(observed_at);
            return Ok(());
        }

        let snapshots = match self.tracker.issue_states_by_ids(&issue_ids).await {
            Ok(snapshots) => snapshots,
            Err(error) => {
                if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                    return Ok(());
                }
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };
        self.last_running_state_refresh_at = Some(observed_at);
        let tracker_snapshot = TrackerSnapshot {
            active: Vec::new(),
            active_index: HashMap::new(),
            terminal_state_by_id: HashMap::new(),
            state_by_id: snapshots
                .into_iter()
                .map(|snapshot| (snapshot.id.clone(), snapshot))
                .collect(),
        };
        self.reconcile_tracker_state(&tracker_snapshot, observed_at)
            .await
    }

    async fn refresh_terminal_issues(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let terminal = match self.tracker.terminal_issues().await {
            Ok(terminal) => terminal,
            Err(error) => {
                if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                    return Ok(());
                }
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };
        self.last_terminal_refresh_at = Some(observed_at);
        let tracker_snapshot = TrackerSnapshot {
            active: Vec::new(),
            active_index: HashMap::new(),
            terminal_state_by_id: terminal
                .into_iter()
                .map(|issue| (issue.id, issue.state))
                .collect(),
            state_by_id: HashMap::new(),
        };
        self.reconcile_tracker_state(&tracker_snapshot, observed_at)
            .await
    }

    async fn load_dispatch_candidates(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<Option<Vec<TrackerIssueSummary>>, SchedulerError> {
        match self.tracker.candidate_issue_summaries().await {
            Ok(active) => {
                self.last_dispatch_discovery_at = Some(observed_at);
                Ok(Some(active))
            }
            Err(error) => {
                if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                    Ok(None)
                } else {
                    Err(SchedulerError::Tracker {
                        detail: error.to_string(),
                    })
                }
            }
        }
    }

    fn record_full_detail_refresh(&mut self, observed_at: TimestampMs) {
        self.last_running_state_refresh_at = Some(observed_at);
        self.last_dispatch_discovery_at = Some(observed_at);
        self.last_terminal_refresh_at = Some(observed_at);
        self.last_full_detail_refresh_at = Some(observed_at);
    }

    fn expire_linear_cooldown(&mut self, observed_at: TimestampMs) {
        if self
            .linear_blocked_until
            .is_some_and(|blocked_until| blocked_until <= observed_at)
        {
            self.linear_blocked_until = None;
        }
    }

    fn linear_cooldown_active(&self, observed_at: TimestampMs) -> bool {
        match self.linear_blocked_until {
            Some(blocked_until) if blocked_until > observed_at => true,
            Some(_) | None => false,
        }
    }

    fn refresh_health_from_linear_cooldown(&mut self, observed_at: TimestampMs) {
        self.health = if self.linear_cooldown_active(observed_at) {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };
    }

    fn set_linear_cooldown_from_tracker_error(
        &mut self,
        error: &T::Error,
        observed_at: TimestampMs,
    ) -> bool {
        if T::error_category(error) != Some(TrackerErrorCategory::RateLimited) {
            return false;
        }

        let delay_ms = T::retry_after(error)
            .map(duration_millis_saturating)
            .unwrap_or(self.config.poll_interval_ms)
            .max(self.config.poll_interval_ms);
        let blocked_until = observed_at.saturating_add(DurationMs::new(delay_ms));
        self.linear_blocked_until = Some(
            self.linear_blocked_until
                .map_or(blocked_until, |existing| existing.max(blocked_until)),
        );
        self.health = HealthStatus::Degraded;
        warn!(
            delay_ms,
            blocked_until_ms = blocked_until.as_u64(),
            "Linear tracker is rate limited; deferring Linear reads"
        );
        true
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
            let recovered_harness_kind = record.harness_kind.clone();
            if let Some(active_issue) = tracker_snapshot.active_issue(&issue_id) {
                let normalized = normalize_tracker_issue(active_issue, &self.config)?;
                self.upsert_active_execution(normalized, observed_at, Some(record.workspace))?;
                self.restore_recovered_run(
                    &issue_id,
                    record.recovered_run,
                    recovered_harness_kind,
                    observed_at,
                )?;
                continue;
            }

            if tracker_snapshot.contains_terminal(issue_id.as_str()) {
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
        for tracker_issue in &tracker_snapshot.active {
            let normalized = normalize_tracker_issue(tracker_issue, &self.config)?;
            if self
                .interrupt_human_review_polling_for_merging(&normalized, observed_at)
                .await?
            {
                continue;
            }
            self.upsert_active_execution(normalized, observed_at, None)?;
        }

        let existing_ids = self.executions.keys().cloned().collect::<Vec<_>>();
        for issue_id in existing_ids {
            if tracker_snapshot.contains_active(issue_id.as_str()) {
                continue;
            }

            if let Some(terminal_state_name) =
                tracker_snapshot.terminal_state_name(issue_id.as_str())
            {
                let Some(existing) = self.executions.get(&issue_id) else {
                    continue;
                };
                let mut normalized = existing.issue().clone();
                normalized.state = issue_state_from_name(terminal_state_name, &self.config);
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
                    if let Some(existing) = self.executions.get(&issue_id) {
                        let mut issue = existing.issue().clone();
                        issue.state = issue_state_from_name(&snapshot.state.name, &self.config);
                        if self
                            .interrupt_human_review_polling_for_merging(&issue, observed_at)
                            .await?
                        {
                            continue;
                        }
                        self.refresh_execution_issue(&issue_id, issue)?;
                    }
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

    fn known_dispatch_candidates(&self, observed_at: TimestampMs) -> Vec<TrackerIssue> {
        self.executions
            .values()
            .filter(|execution| {
                if execution.issue().state.category != IssueStateCategory::Active {
                    return false;
                }
                match execution.status() {
                    SchedulerStatus::Unclaimed => true,
                    SchedulerStatus::RetryQueued => execution
                        .retry()
                        .is_some_and(|retry| retry.due_at <= observed_at),
                    SchedulerStatus::Released
                    | SchedulerStatus::Claimed
                    | SchedulerStatus::Running => false,
                }
            })
            .map(|execution| tracker_issue_from_normalized(execution.issue()))
            .collect()
    }

    fn refresh_execution_issue(
        &mut self,
        issue_id: &IssueId,
        issue: NormalizedIssue,
    ) -> Result<(), SchedulerError> {
        if let Some(mut execution) = self.remove_execution(issue_id) {
            execution.refresh_issue(issue)?;
            self.insert_execution(issue_id.clone(), execution);
        }
        Ok(())
    }

    async fn interrupt_human_review_polling_for_merging(
        &mut self,
        issue: &NormalizedIssue,
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        let Some((issue_id, mut execution, run, harness_kind)) =
            self.merging_interrupt_candidate(issue)
        else {
            return Ok(false);
        };

        let command = Self::prepare_merging_interrupt(
            &mut execution,
            issue,
            &run,
            harness_kind,
            observed_at,
        )?;
        if let Some(command) = command {
            let result = self.worker.interrupt_worker(command).await;
            Self::apply_merging_interrupt_result(&mut execution, observed_at, result)?;
        }

        self.insert_execution(issue_id, execution);
        Ok(true)
    }

    fn merging_interrupt_candidate(
        &self,
        issue: &NormalizedIssue,
    ) -> Option<(IssueId, IssueExecution, RunAttempt, String)> {
        let existing = self.executions.get(&issue.id)?;
        if !is_human_review_to_merging(existing.issue(), issue)
            || !matches!(
                existing.status(),
                SchedulerStatus::Claimed | SchedulerStatus::Running
            )
        {
            return None;
        }

        let issue_id = issue.id.clone();
        let execution = self
            .executions
            .get(&issue_id)
            .cloned()
            .expect("execution existed before clone");
        let run = execution.current_run().cloned()?;
        let harness_kind = self
            .worker_harness_kind
            .get(&run.worker_id)
            .cloned()
            .unwrap_or_else(|| "<unknown>".to_string());
        if harness_kind == "<unknown>" {
            warn!(
                issue_id = %issue.id,
                worker_id = %run.worker_id,
                "missing scheduler harness kind for tracker-merging interrupt"
            );
        }

        Some((issue_id, execution, run, harness_kind))
    }

    fn prepare_merging_interrupt(
        execution: &mut IssueExecution,
        issue: &NormalizedIssue,
        run: &RunAttempt,
        harness_kind: String,
        observed_at: TimestampMs,
    ) -> Result<Option<HarnessInterruptCommand>, SchedulerError> {
        execution.refresh_issue(issue.clone())?;
        let (command, queued) = execution.request_interrupt(
            harness_kind,
            None,
            HarnessInterruptReason::TrackerMergingSupersedesHumanReview,
            HarnessInterruptExpectedNextState::CloseoutPending,
            observed_at,
        )?;

        if !queued {
            return Ok(None);
        }

        execution.observe_runtime_event(
            observed_at,
            Some(format!(
                "tracker-merging-supersedes-human-review-{}",
                observed_at.as_u64()
            )),
            Some("scheduler.interrupt_requested".to_string()),
            Some(
                "Tracker state Merging superseded Human Review polling: tracker_merging_supersedes_human_review"
                    .to_string(),
            ),
            Some(serde_json::json!({
                "reason": HarnessInterruptReason::TrackerMergingSupersedesHumanReview.as_str(),
                "from_state": HUMAN_REVIEW_STATE,
                "to_state": issue.state.name,
                "worker_id": run.worker_id.as_str(),
            })),
        )?;
        Ok(Some(command))
    }

    fn apply_merging_interrupt_result(
        execution: &mut IssueExecution,
        observed_at: TimestampMs,
        result: Result<WorkerInterruptAcknowledgement, M::Error>,
    ) -> Result<(), SchedulerError> {
        match result {
            Ok(acknowledgement) if acknowledgement.accepted => {
                execution.acknowledge_interrupt(observed_at)?;
                if let Some(detail) = acknowledgement.detail {
                    execution.observe_runtime_event(
                        observed_at,
                        Some(format!(
                            "tracker-merging-interrupt-acknowledged-{}",
                            observed_at.as_u64()
                        )),
                        Some("scheduler.interrupt_acknowledged".to_string()),
                        Some(detail),
                        Some(serde_json::json!({
                            "reason": HarnessInterruptReason::TrackerMergingSupersedesHumanReview.as_str(),
                        })),
                    )?;
                }
            }
            Ok(acknowledgement) => {
                execution.fail_interrupt(
                    observed_at,
                    acknowledgement
                        .detail
                        .unwrap_or_else(|| "worker interrupt request was not accepted".to_string()),
                )?;
            }
            Err(error) => {
                execution.fail_interrupt(observed_at, error.to_string())?;
            }
        }
        Ok(())
    }

    fn restore_recovered_run(
        &mut self,
        issue_id: &IssueId,
        recovered_run: Option<RecoveredRun>,
        harness_kind: Option<String>,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let Some(recovered_run) = recovered_run else {
            return Ok(());
        };
        let Some(execution) = self.executions.get(issue_id).cloned() else {
            return Ok(());
        };
        if execution.status() != SchedulerStatus::Unclaimed {
            return Ok(());
        }
        let Some(workspace) = execution.workspace().cloned() else {
            return Ok(());
        };
        let run = RunAttempt::new(
            recovered_run.worker_id.clone(),
            execution.issue().id.clone(),
            execution.issue().identifier.clone(),
            workspace.path.clone(),
            observed_at,
            None,
            self.config.max_turns,
        );
        let mut execution = execution.claim(run.clone())?;
        execution = execution.start_running(
            observed_at,
            effective_stall_timeout(self.config.stall_timeout_ms),
            Some(recovered_run.conversation),
        )?;
        execution.record_turn_started(observed_at)?;
        self.worker_index
            .insert(run.worker_id.clone(), issue_id.clone());
        if let Some(harness_kind) = harness_kind.filter(|kind| !kind.trim().is_empty()) {
            self.worker_harness_kind
                .insert(run.worker_id.clone(), harness_kind);
        }
        self.insert_execution(issue_id.clone(), execution);
        Ok(())
    }

    fn has_running_executions(&self) -> bool {
        self.executions.values().any(|execution| {
            matches!(
                execution.status(),
                SchedulerStatus::Claimed | SchedulerStatus::Running
            )
        })
    }

    async fn dispatch_summary_candidates(
        &mut self,
        summaries: &[TrackerIssueSummary],
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let ready = filter_issue_summaries_for_dispatch(
            summaries.to_vec(),
            &self.config.terminal_state_set(),
        );
        let available_capacity = usize::try_from(self.config.max_concurrent_agents)
            .unwrap_or(usize::MAX)
            .saturating_sub(self.worker_index.len());
        if available_capacity == 0 {
            return Ok(());
        }

        let identifiers = ready
            .iter()
            .take(available_capacity)
            .map(|issue| issue.identifier.clone())
            .collect::<Vec<_>>();
        if identifiers.is_empty() {
            return Ok(());
        }
        let mut detailed_by_identifier = match self
            .tracker
            .issues_by_identifiers(&identifiers)
            .await
        {
            Ok(issues) => issues
                .into_iter()
                .map(|issue| (issue.identifier.to_ascii_uppercase(), issue))
                .collect::<HashMap<_, _>>(),
            Err(error) => {
                if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                    return Ok(());
                }
                if T::error_category(&error) == Some(TrackerErrorCategory::NotFound) {
                    warn!(
                        "skipping dispatch discovery because selected issue details were not found"
                    );
                    return Ok(());
                }
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };

        let mut detailed = Vec::new();
        for summary in ready.into_iter().take(available_capacity) {
            let key = summary.identifier.to_ascii_uppercase();
            let Some(detailed_issue) = detailed_by_identifier.remove(&key) else {
                warn!(
                    identifier = %summary.identifier,
                    "skipping stale dispatch candidate missing from detail refresh"
                );
                continue;
            };
            let normalized = normalize_tracker_issue(&detailed_issue, &self.config)?;
            if normalized.state.category != IssueStateCategory::Active {
                warn!(
                    identifier = %normalized.identifier,
                    state = %normalized.state.name,
                    "skipping dispatch candidate no longer in an active state"
                );
                continue;
            }
            detailed.push(detailed_issue);
        }

        self.dispatch_ready_issues(&detailed, observed_at).await
    }

    async fn dispatch_ready_issues(
        &mut self,
        active_issues: &[TrackerIssue],
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let ready =
            filter_issues_for_dispatch(active_issues.to_vec(), &self.config.terminal_state_set());
        let available_capacity = usize::try_from(self.config.max_concurrent_agents)
            .unwrap_or(usize::MAX)
            .saturating_sub(self.worker_index.len());
        if available_capacity == 0 {
            return Ok(());
        }

        let mut pending_launches = Vec::new();
        let mut planned_running_by_state: HashMap<String, usize> = HashMap::new();

        for tracker_issue in ready {
            if pending_launches.len() >= available_capacity {
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

            let state_key = normalized_state_name(&normalized.state.name);

            if let Some(limit) =
                state_limit_for(&self.config.max_concurrent_agents_by_state, &state_key)
            {
                let running_in_state = self.running_count_for_normalized_state(&state_key)
                    + planned_running_by_state
                        .get(&state_key)
                        .copied()
                        .unwrap_or_default();
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
            let route = decide_issue_route(&normalized, &self.config)?;

            let mut execution = self
                .remove_execution(&issue_id)
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
                route,
            };

            *planned_running_by_state.entry(state_key).or_default() += 1;
            pending_launches.push((issue_id, execution, claimed_run, start_request));
        }

        let start_results = self
            .worker
            .start_workers(
                pending_launches
                    .iter()
                    .map(|(_, _, _, request)| request.clone())
                    .collect(),
            )
            .await;

        for ((issue_id, mut execution, claimed_run, start_request), result) in
            pending_launches.into_iter().zip(start_results.into_iter())
        {
            match result {
                Ok(launch) => {
                    execution = execution.start_running(
                        observed_at,
                        effective_stall_timeout(self.config.stall_timeout_ms),
                        Some(launch.conversation),
                    )?;
                    execution.record_turn_started(observed_at)?;
                    self.worker_index
                        .insert(claimed_run.worker_id.clone(), issue_id.clone());
                    self.worker_harness_kind.insert(
                        claimed_run.worker_id.clone(),
                        start_request.route.harness_kind,
                    );
                    debug!(issue_id = %issue_id, "dispatched scheduler worker");
                }
                Err(error) => {
                    let detail = error.to_string();
                    warn!(issue_id = %issue_id, error = %detail, "failed to launch scheduler worker");
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

            self.insert_execution(issue_id, execution);
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
                    payload,
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
                            payload,
                        )?;
                    }
                }
                WorkerUpdate::Finished { worker_id, outcome } => {
                    let Some(issue_id) = self.worker_index.remove(&worker_id) else {
                        continue;
                    };
                    self.worker_harness_kind.remove(&worker_id);
                    let Some(execution) = self.remove_execution(&issue_id) else {
                        continue;
                    };
                    let finished_at = outcome.finished_at;
                    let execution =
                        self.resolve_finished_execution(execution, outcome, finished_at)?;
                    self.insert_execution(issue_id, execution);
                }
                WorkerUpdate::ConversationMetadataUpdate {
                    worker_id,
                    conversation,
                } => {
                    let Some(issue_id) = self.worker_index.get(&worker_id).cloned() else {
                        continue;
                    };
                    if let Some(execution) = self.executions.get_mut(&issue_id) {
                        execution.update_conversation(conversation);
                    }
                }
                WorkerUpdate::TokenUsageUpdate {
                    worker_id,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    total_tokens,
                } => {
                    let Some(issue_id) = self.worker_index.get(&worker_id).cloned() else {
                        continue;
                    };
                    if let Some(execution) = self.executions.get_mut(&issue_id) {
                        execution.update_conversation_token_usage(
                            input_tokens,
                            output_tokens,
                            cache_read_tokens,
                            total_tokens,
                        );
                    }
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
                crate::opensymphony_domain::SchedulerState::Running { stall, .. }
                    if stall.stalled_at <= observed_at =>
                {
                    Some(issue_id.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        for issue_id in stalled {
            let Some(mut execution) = self.remove_execution(&issue_id) else {
                continue;
            };
            let Some(run) = execution.current_run().cloned() else {
                self.insert_execution(issue_id, execution);
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
            self.insert_execution(issue_id, execution);
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
        let execution = match self.remove_execution(&issue_id) {
            Some(existing) => existing,
            None => IssueExecution::new(issue.clone(), observed_at),
        };

        // Do not reopen executions that were released due to terminal worker outcomes
        // (Detached/CancelFailed). These represent runs that could not be safely stopped
        // and reopening them would duplicate still-active work.
        let was_terminal_outcome = matches!(
            execution.last_worker_outcome().map(|o| o.outcome),
            Some(WorkerOutcomeKind::Detached | WorkerOutcomeKind::CancelFailed)
        );
        let mut execution =
            if execution.status() == SchedulerStatus::Released && !was_terminal_outcome {
                execution.reopen(observed_at)?
            } else {
                execution
            };

        execution.refresh_issue(issue.clone())?;
        if let Some(workspace) = recovered_workspace {
            execution.attach_workspace(workspace)?;
        }
        self.insert_execution(issue_id, execution);
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
        let Some(mut execution) = self.remove_execution(&issue_id) else {
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
        self.insert_execution(issue_id, execution);
        Ok(())
    }

    async fn abort_worker(
        &mut self,
        worker_id: &WorkerId,
        reason: WorkerAbortReason,
    ) -> Result<(), SchedulerError> {
        self.worker_index.remove(worker_id);
        self.worker_harness_kind.remove(worker_id);
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
        if let Some(reason) = non_active_release_reason(execution.issue().state.category.clone()) {
            return Ok(execution.release(observed_at, reason, Some(outcome))?);
        }

        // Detached and CancelFailed are terminal outcomes: release the execution instead of
        // queuing a retry. The underlying OpenHands run may still be active and retrying
        // would duplicate work.
        if matches!(
            outcome.outcome,
            WorkerOutcomeKind::Detached | WorkerOutcomeKind::CancelFailed
        ) {
            return Ok(execution.release(
                observed_at,
                ReleaseReason::TrackerInactive,
                Some(outcome),
            )?);
        }

        self.queue_retry_for_outcome(execution, outcome, observed_at)
    }

    fn queue_retry_for_outcome(
        &self,
        execution: IssueExecution,
        outcome: WorkerOutcomeRecord,
        observed_at: TimestampMs,
    ) -> Result<IssueExecution, SchedulerError> {
        let run = execution
            .current_run()
            .expect("running execution must have a run");
        let retry_reason = if tracker_merging_interrupt_cancelled(&execution, &outcome) {
            None
        } else {
            retry_reason_for_outcome(outcome.outcome)
        };
        let retry = match retry_reason {
            None => RetryEntry::continuation(
                execution.issue(),
                run.attempt,
                run.normal_retry_count,
                observed_at,
                self.config.retry_policy,
            )?,
            Some(reason) => RetryEntry::failure(
                execution.issue(),
                run.attempt,
                run.normal_retry_count,
                observed_at,
                reason,
                outcome.error.clone().or(outcome.summary.clone()),
                self.config.retry_policy,
            )?,
        };
        Ok(execution.queue_retry(retry, outcome)?)
    }

    fn next_worker_id(&mut self) -> Result<WorkerId, SchedulerError> {
        self.next_worker_ordinal = self.next_worker_ordinal.saturating_add(1);
        WorkerId::new(format!("scheduler-worker-{}", self.next_worker_ordinal))
            .map_err(SchedulerError::Identifier)
    }

    fn remove_execution(&mut self, issue_id: &IssueId) -> Option<IssueExecution> {
        let execution = self.executions.remove(issue_id)?;
        self.decrement_running_count(&execution);
        self.debug_assert_running_counts();
        Some(execution)
    }

    fn insert_execution(&mut self, issue_id: IssueId, execution: IssueExecution) {
        let current_key = running_state_key_for_execution(&execution);
        if let Some(previous) = self.executions.insert(issue_id, execution) {
            self.decrement_running_count(&previous);
        }
        if let Some(state_key) = current_key {
            *self.running_counts_by_state.entry(state_key).or_default() += 1;
        }
        self.debug_assert_running_counts();
    }

    fn running_count_for_normalized_state(&self, state_key: &str) -> usize {
        self.running_counts_by_state
            .get(state_key)
            .copied()
            .unwrap_or_default()
    }

    fn decrement_running_count(&mut self, execution: &IssueExecution) {
        let Some(state_key) = running_state_key_for_execution(execution) else {
            return;
        };
        let count = self
            .running_counts_by_state
            .get_mut(&state_key)
            .expect("running execution must have a cached count");
        *count -= 1;
        if *count == 0 {
            self.running_counts_by_state.remove(&state_key);
        }
    }

    fn debug_assert_running_counts(&self) {
        #[cfg(debug_assertions)]
        {
            let mut expected = HashMap::new();
            for execution in self.executions.values() {
                if let Some(state_key) = running_state_key_for_execution(execution) {
                    *expected.entry(state_key).or_insert(0) += 1;
                }
            }
            debug_assert_eq!(self.running_counts_by_state, expected);
        }
    }
}

struct TrackerSnapshot {
    active: Vec<TrackerIssue>,
    active_index: HashMap<String, usize>,
    terminal_state_by_id: HashMap<String, String>,
    state_by_id: HashMap<String, TrackerIssueStateSnapshot>,
}

impl TrackerSnapshot {
    fn active_issue(&self, issue_id: &IssueId) -> Option<&TrackerIssue> {
        self.active_index
            .get(issue_id.as_str())
            .and_then(|index| self.active.get(*index))
    }

    fn contains_active(&self, issue_id: &str) -> bool {
        self.active_index.contains_key(issue_id)
    }

    fn contains_terminal(&self, issue_id: &str) -> bool {
        self.terminal_state_by_id.contains_key(issue_id)
    }

    fn terminal_state_name(&self, issue_id: &str) -> Option<&str> {
        self.terminal_state_by_id.get(issue_id).map(String::as_str)
    }
}

pub fn decide_issue_route(
    _issue: &NormalizedIssue,
    config: &SchedulerConfig,
) -> Result<HarnessRouteDecision, SchedulerError> {
    let capability = harness_capability(&config.routing.harness)?;
    if !capability.available || !capability.actions.start_run {
        return Err(SchedulerError::InvalidConfiguration {
            detail: format!(
                "selected harness `{}` cannot start issue execution",
                config.routing.harness
            ),
        });
    }

    Ok(HarnessRouteDecision {
        task_type: ROUTING_TASK_ISSUE_EXECUTION.into(),
        harness_kind: config.routing.harness.clone(),
        model: config.routing.model.clone(),
        model_profile: config.routing.model_profile.clone(),
        reason: routing_reason(&config.routing),
        dry_run: config.routing.dry_run,
        user_override: config.routing.harness_from_env
            || config.routing.model_from_env
            || config.routing.model_profile_from_env,
    })
}

fn routing_reason(routing: &RoutingConfig) -> String {
    let mut parts = Vec::new();
    parts.push(if routing.harness_from_env {
        format!("harness selected by {}", routing.harness_env)
    } else {
        "harness selected by workflow routing.harness".into()
    });
    if routing.model.is_some() {
        parts.push(if routing.model_from_env {
            format!("model selected by {}", routing.model_env)
        } else {
            "model selected by workflow routing.model".into()
        });
    }
    if routing.model_profile.is_some() {
        parts.push(if routing.model_profile_from_env {
            format!("model profile selected by {}", routing.model_profile_env)
        } else {
            "model profile selected by workflow routing.model_profile".into()
        });
    }
    parts.join("; ")
}

fn harness_capability(kind: &str) -> Result<HarnessCapability, SchedulerError> {
    HarnessKind::parse(kind)
        .map(HarnessKind::capability)
        .ok_or_else(|| SchedulerError::InvalidConfiguration {
            detail: format!("unknown routing harness `{kind}`"),
        })
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
        branch_name: issue.branch_name.clone(),
        pr_url: issue.pr_url.clone(),
        url: Some(issue.url.clone()),
        labels: issue.labels.clone(),
        project_id: issue.project_id.clone(),
        project_slug: issue.project_slug.clone(),
        project_name: issue.project_name.clone(),
        parent_id: match &issue.parent_id {
            Some(parent_id) => Some(IssueId::new(parent_id.clone())?),
            None => None,
        },
        blocked_by: issue
            .blocked_by
            .iter()
            .map(|blocker| {
                Ok(crate::opensymphony_domain::BlockerRef {
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
        pr_url: None,
        url: None,
        labels: Vec::new(),
        project_id: None,
        project_slug: None,
        project_name: None,
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

fn state_limit_for(limits: &BTreeMap<String, u32>, state_key: &str) -> Option<u32> {
    limits.get(state_key).copied().or_else(|| {
        limits.iter().find_map(|(configured_state, limit)| {
            (normalized_state_name(configured_state) == state_key).then_some(*limit)
        })
    })
}

fn non_active_release_reason(category: IssueStateCategory) -> Option<ReleaseReason> {
    match category {
        IssueStateCategory::Terminal => Some(ReleaseReason::TrackerTerminal),
        IssueStateCategory::NonActive => Some(ReleaseReason::TrackerInactive),
        IssueStateCategory::Active => None,
    }
}

fn retry_reason_for_outcome(outcome: WorkerOutcomeKind) -> Option<RetryReason> {
    match outcome {
        WorkerOutcomeKind::Succeeded => None,
        WorkerOutcomeKind::Failed | WorkerOutcomeKind::TimedOut => Some(RetryReason::Failure),
        WorkerOutcomeKind::Stalled => Some(RetryReason::Stalled),
        WorkerOutcomeKind::Cancelled => Some(RetryReason::Cancelled),
        // Detached and CancelFailed are terminal: do not retry automatically because
        // the underlying OpenHands run may still be active and retrying would duplicate work.
        WorkerOutcomeKind::Detached | WorkerOutcomeKind::CancelFailed => None,
    }
}

fn tracker_merging_interrupt_cancelled(
    execution: &IssueExecution,
    outcome: &WorkerOutcomeRecord,
) -> bool {
    outcome.outcome == WorkerOutcomeKind::Cancelled
        && execution.interrupt().is_some_and(|interrupt| {
            interrupt.command.reason == HarnessInterruptReason::TrackerMergingSupersedesHumanReview
        })
}

fn normalized_state_set(states: &[String]) -> HashSet<String> {
    states
        .iter()
        .map(|state| normalized_state_name(state))
        .collect()
}

fn matches_state_name(name: &str, states: &[String]) -> bool {
    let normalized = normalized_state_name(name);
    states
        .iter()
        .any(|state| normalized_state_name(state) == normalized)
}

fn running_state_key_for_execution(execution: &IssueExecution) -> Option<String> {
    (execution.status() == SchedulerStatus::Running)
        .then(|| normalized_state_name(&execution.issue().state.name))
}

fn is_human_review_to_merging(previous: &NormalizedIssue, current: &NormalizedIssue) -> bool {
    normalized_state_name(&previous.state.name) == HUMAN_REVIEW_STATE
        && normalized_state_name(&current.state.name) == MERGING_STATE
}

fn normalized_state_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn tracker_issue_summary_from_issue(issue: TrackerIssue) -> TrackerIssueSummary {
    TrackerIssueSummary {
        id: issue.id,
        identifier: issue.identifier,
        url: issue.url,
        title: issue.title,
        priority: issue.priority,
        state: issue.state,
        state_kind: issue.state_kind,
        blocked_by: issue.blocked_by,
        sub_issues: issue.sub_issues,
        created_at: issue.created_at,
        updated_at: issue.updated_at,
    }
}

fn tracker_issue_from_normalized(issue: &NormalizedIssue) -> TrackerIssue {
    TrackerIssue {
        id: issue.id.to_string(),
        identifier: issue.identifier.to_string(),
        url: issue.url.clone().unwrap_or_default(),
        title: issue.title.clone(),
        description: issue.description.clone(),
        priority: issue.priority,
        state: issue.state.name.clone(),
        state_kind: tracker_state_kind_from_issue_state(&issue.state),
        branch_name: issue.branch_name.clone(),
        pr_url: issue.pr_url.clone(),
        labels: issue.labels.clone(),
        project_id: issue.project_id.clone(),
        project_slug: issue.project_slug.clone(),
        project_name: issue.project_name.clone(),
        parent_id: issue.parent_id.as_ref().map(ToString::to_string),
        parent: None,
        project_milestone: None,
        blocked_by: issue
            .blocked_by
            .iter()
            .filter_map(|blocker| {
                let id = blocker.id.as_ref()?.to_string();
                let identifier = blocker.identifier.as_ref()?.to_string();
                let state_name = blocker.state.clone().unwrap_or_default();
                Some(TrackerIssueBlocker {
                    id,
                    identifier: identifier.clone(),
                    title: identifier,
                    state: tracker_issue_state_from_name(&state_name),
                })
            })
            .collect(),
        sub_issues: issue
            .sub_issues
            .iter()
            .map(|child| TrackerIssueRef {
                id: child.id.to_string(),
                identifier: child.identifier.to_string(),
                title: None,
                url: None,
                state: child.state.clone(),
            })
            .collect(),
        created_at: timestamp_to_datetime(issue.created_at),
        updated_at: timestamp_to_datetime(issue.updated_at),
    }
}

fn filter_issue_summaries_for_dispatch<I>(
    summaries: I,
    terminal_states: &HashSet<String>,
) -> Vec<TrackerIssueSummary>
where
    I: IntoIterator<Item = TrackerIssueSummary>,
{
    let mut filtered = summaries
        .into_iter()
        .filter(|issue| should_dispatch_issue_summary(issue, terminal_states))
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| {
        summary_priority_rank(left)
            .cmp(&summary_priority_rank(right))
            .then_with(|| left.sub_issues.len().cmp(&right.sub_issues.len()))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.identifier.cmp(&right.identifier))
    });
    filtered
}

fn should_dispatch_issue_summary(
    issue: &TrackerIssueSummary,
    terminal_states: &HashSet<String>,
) -> bool {
    !issue
        .blocked_by
        .iter()
        .any(|blocker| !blocker.is_terminal())
        && (issue.sub_issues.is_empty()
            || issue
                .sub_issues
                .iter()
                .all(|sub_issue| sub_issue.is_terminal(terminal_states)))
}

fn summary_priority_rank(issue: &TrackerIssueSummary) -> u8 {
    issue.priority.unwrap_or(u8::MAX)
}

fn tracker_issue_state_from_name(name: &str) -> TrackerIssueState {
    let kind = tracker_state_kind_from_name(name);
    TrackerIssueState {
        id: normalized_state_name(name),
        name: name.to_string(),
        tracker_type: tracker_type_for_state_kind(&kind).to_string(),
        kind,
    }
}

fn tracker_state_kind_from_issue_state(state: &IssueState) -> TrackerIssueStateKind {
    match state.category {
        IssueStateCategory::Active => TrackerIssueStateKind::Started,
        IssueStateCategory::Terminal => tracker_state_kind_from_name(&state.name),
        IssueStateCategory::NonActive => tracker_state_kind_from_name(&state.name),
    }
}

fn tracker_state_kind_from_name(name: &str) -> TrackerIssueStateKind {
    match normalized_state_name(name).as_str() {
        "backlog" => TrackerIssueStateKind::Backlog,
        "todo" => TrackerIssueStateKind::Unstarted,
        "done" | "completed" | "closed" => TrackerIssueStateKind::Completed,
        "canceled" | "cancelled" => TrackerIssueStateKind::Canceled,
        "triage" | "triaged" => TrackerIssueStateKind::Triage,
        "in progress" | "review" | "human review" => TrackerIssueStateKind::Started,
        other => TrackerIssueStateKind::Unknown(other.to_string()),
    }
}

fn tracker_type_for_state_kind(kind: &TrackerIssueStateKind) -> &'static str {
    match kind {
        TrackerIssueStateKind::Backlog => "backlog",
        TrackerIssueStateKind::Unstarted => "unstarted",
        TrackerIssueStateKind::Started => "started",
        TrackerIssueStateKind::Completed => "completed",
        TrackerIssueStateKind::Canceled => "canceled",
        TrackerIssueStateKind::Triage => "triage",
        TrackerIssueStateKind::Unknown(_) => "unknown",
    }
}

fn effective_stall_timeout(stall_timeout_ms: Option<u64>) -> DurationMs {
    DurationMs::new(stall_timeout_ms.unwrap_or(DISABLED_STALL_TIMEOUT_MS))
}

fn due(last_observed_at: Option<TimestampMs>, interval_ms: u64, observed_at: TimestampMs) -> bool {
    last_observed_at
        .is_none_or(|last| observed_at.as_u64() >= last.as_u64().saturating_add(interval_ms))
}

fn duration_millis_saturating(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn datetime_to_timestamp(datetime: DateTime<Utc>) -> TimestampMs {
    let millis = datetime.timestamp_millis();
    if millis <= 0 {
        TimestampMs::new(0)
    } else {
        TimestampMs::new(millis as u64)
    }
}

fn timestamp_to_datetime(timestamp: Option<TimestampMs>) -> DateTime<Utc> {
    let millis = timestamp.map(|value| value.as_u64()).unwrap_or_default();
    let millis = i64::try_from(millis).unwrap_or(i64::MAX);
    DateTime::<Utc>::from_timestamp_millis(millis).unwrap_or_else(Utc::now)
}

fn current_epoch_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
