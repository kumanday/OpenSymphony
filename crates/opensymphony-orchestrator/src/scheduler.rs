use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    future::Future,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use crate::opensymphony_domain::{
    ComponentHealthSnapshot, ConversationMetadata, DaemonSnapshot, DurationMs,
    HarnessInterruptCommand, HarnessInterruptExpectedNextState, HarnessInterruptReason,
    HarnessInterruptStatus, HealthStatus, IdentifierError, IssueExecution, IssueId,
    IssueIdentifier, IssueRef, IssueSnapshot, IssueState, IssueStateCategory, NormalizedIssue,
    OrchestratorSnapshot, ReleaseReason, RepositoryBindingOutcome, RepositoryRouting, RetryAttempt,
    RetryCalculationError, RetryEntry, RetryPolicy, RetryReason, RunAttempt, RuntimeUsageTotals,
    SchedulerStatus, StateTransitionError, TimestampMs, TrackerErrorCategory, TrackerIssue,
    TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot, TrackerIssueSummary, TrackerStateId, WorkerId, WorkerOutcomeKind,
    WorkerOutcomeRecord, WorkspaceRecord, managed_repository_aliases,
};
use crate::opensymphony_gateway_schema::capability::{HarnessCapability, HarnessKind};
use crate::opensymphony_workflow::{ResolvedWorkflow, RoutingConfig};
use crate::opensymphony_workspace::{
    CleanupTarget, CleanupTerminalOutcome, checkout_workspace_key, redact_runtime_diagnostic,
    sanitize_workspace_key,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    select,
    time::{MissedTickBehavior, interval, timeout},
};
use tracing::{debug, warn};

use super::filter_issues_for_dispatch;
use super::{
    DurableOrchestratorState, HierarchyBlockedReason, HierarchySnapshot, LeaseRecord,
    LeaseResource, ParentAttemptRoot, ParentAttemptStatus, ParentCleanupReceipt,
    ParentCleanupStatus, ParentEligibilityEvidence, ParentIntegrationController,
    ParentIntegrationError, ParentProviderOperationKind, ParentRepairAttempt, ParentRepairPolicy,
    ParentRepairStatus, ParentRepositoryTarget, ParentSubtreeCleanupIntent,
    ParentSubtreeCleanupStatus, ParentSubtreeCleanupTarget,
};

const DISABLED_STALL_TIMEOUT_MS: u64 = u64::MAX / 4;
const ROUTING_TASK_ISSUE_EXECUTION: &str = "issue_execution";
const RUNNING_STATE_REFRESH_INTERVAL_MS: u64 = 30_000;
const DISPATCH_DISCOVERY_INTERVAL_MS: u64 = 60_000;
const TERMINAL_REFRESH_INTERVAL_MS: u64 = 300_000;
const FULL_DETAIL_REFRESH_INTERVAL_MS: u64 = 3_600_000;
const HUMAN_REVIEW_STATE: &str = "human review";
const MERGING_STATE: &str = "merging";
const PARENT_ELIGIBILITY_TIMEOUT: Duration = Duration::from_secs(30);
// A repository-neutral child can fan out into several leaf merge-evidence
// requests. The runtime provider evaluates those leaves eight at a time, so
// reserve one base timeout for each provider batch in the descendant subtree.
const PARENT_ELIGIBILITY_PROVIDER_CONCURRENCY: usize = 8;

fn parent_eligibility_timeout(provider_work_units: usize) -> Duration {
    let batches = provider_work_units
        .max(1)
        .saturating_add(PARENT_ELIGIBILITY_PROVIDER_CONCURRENCY - 1)
        / PARENT_ELIGIBILITY_PROVIDER_CONCURRENCY;
    let multiplier = u32::try_from(batches).unwrap_or(u32::MAX);
    PARENT_ELIGIBILITY_TIMEOUT.saturating_mul(multiplier)
}

fn workspace_key_changed_for_issue(execution: &IssueExecution, issue: &NormalizedIssue) -> bool {
    let Some(workspace) = execution.workspace() else {
        return false;
    };
    // Repository binding drift is handled separately. This guard keeps legacy
    // recovered workspaces with the same identifier compatible while still
    // fencing a verified checkout when the tracker identifier changes.
    if execution.issue().identifier == issue.identifier {
        return false;
    }

    let expected_key = match issue.repository_binding.as_ref() {
        Some(RepositoryBindingOutcome::Resolved(binding)) => checkout_workspace_key(
            issue.identifier.as_str(),
            issue.id.as_str(),
            binding.repository_id().as_str(),
        ),
        _ => sanitize_workspace_key(issue.identifier.as_str()),
    };

    expected_key
        .ok()
        .is_some_and(|expected| expected != workspace.workspace_key.as_str())
}

fn project_identity_changed(previous: &NormalizedIssue, current: &NormalizedIssue) -> bool {
    match (
        previous.project_id.as_deref(),
        current.project_id.as_deref(),
    ) {
        (Some(previous_id), Some(current_id)) => previous_id.trim() != current_id.trim(),
        (Some(_), None) | (None, Some(_)) => true,
        (None, None) => matches!(
            (previous.project_slug.as_deref(), current.project_slug.as_deref()),
            (Some(previous_slug), Some(current_slug))
                if !previous_slug.eq_ignore_ascii_case(current_slug)
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerConfig {
    pub poll_interval_ms: u64,
    pub max_concurrent_agents: u32,
    pub max_turns: u32,
    pub max_concurrent_agents_by_state: BTreeMap<String, u32>,
    pub retry_policy: RetryPolicy,
    pub max_retry_attempts: Option<u32>,
    pub stall_timeout_ms: Option<u64>,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub tracker_project_id: Option<String>,
    pub tracker_project_slug: Option<String>,
    pub tracker_project_ids: Vec<String>,
    pub tracker_project_id_slug_fallbacks: Vec<bool>,
    pub tracker_project_slugs: Vec<String>,
    pub routing: RoutingConfig,
    pub repository_routing: Option<RepositoryRouting>,
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
            max_retry_attempts: None,
            stall_timeout_ms: workflow.config.agent.stall_timeout_ms,
            active_states: workflow.config.tracker.active_states.clone(),
            terminal_states: workflow.config.tracker.terminal_states.clone(),
            tracker_project_id: workflow.config.tracker.project_id.clone(),
            tracker_project_slug: Some(workflow.config.tracker.project_slug.clone()),
            tracker_project_ids: workflow.config.tracker.project_ids.clone(),
            tracker_project_id_slug_fallbacks: workflow
                .config
                .tracker
                .project_id_slug_fallbacks
                .clone(),
            tracker_project_slugs: workflow.config.tracker.project_slugs.clone(),
            routing: workflow.config.routing.clone(),
            repository_routing: None,
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
    pub successful_run: bool,
    pub cancelled_run: bool,
    pub completed_run: bool,
    pub had_in_flight_run: bool,
    pub pending_retry: bool,
    pub normal_retry_count: u32,
    pub retry_scheduled_at: Option<TimestampMs>,
    pub retry_due_at: Option<TimestampMs>,
    pub retry_reason: Option<RetryReason>,
    pub retry_error: Option<String>,
    pub harness_kind: Option<String>,
    pub interrupt_reason: Option<HarnessInterruptReason>,
    pub recovered_run: Option<RecoveredRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryExhaustionRecord {
    pub issue: NormalizedIssue,
    pub normal_retry_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPendingRecord {
    pub issue: NormalizedIssue,
    pub retry: RetryEntry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveredRun {
    pub worker_id: WorkerId,
    pub conversation: ConversationMetadata,
    pub normal_retry_count: u32,
    pub repository_binding: Option<crate::opensymphony_domain::RepositoryBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerStartRequest {
    pub issue: NormalizedIssue,
    pub workspace: WorkspaceRecord,
    pub run: RunAttempt,
    pub route: HarnessRouteDecision,
    /// The issue was restored from durable process-recovery state.
    pub memory_grant_registry_recovered: bool,
    /// A parent retry must attach this durable controller-owned conversation.
    /// The backend rejects a missing or different manifest before session launch.
    pub expected_parent_conversation_id: Option<String>,
    /// Active requested-change repair resumed in the existing parent
    /// conversation. Provider receipts remain scheduler-owned.
    pub parent_repair: Option<ParentRepairAttempt>,
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
    /// Timestamp persisted by the worker path when the run actually began.
    /// This is the boundary used to fence provider evidence for this run.
    pub started_at: Option<TimestampMs>,
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
    BindingSuperseded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerInterruptAcknowledgement {
    pub accepted: bool,
    pub detail: Option<String>,
    pub timed_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkerMetadata {
    issue_id: IssueId,
    harness_kind: Option<String>,
    dry_run: bool,
}

impl WorkerMetadata {
    fn new(issue_id: IssueId, harness_kind: Option<String>) -> Self {
        Self {
            issue_id,
            harness_kind,
            dry_run: false,
        }
    }

    fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.dry_run = dry_run;
        self
    }
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
    async fn issue_by_id(&mut self, issue_id: &str) -> Result<Option<TrackerIssue>, Self::Error> {
        Ok(self
            .issues_by_identifiers(&[issue_id.to_owned()])
            .await?
            .into_iter()
            .next())
    }
    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, Self::Error>;
    async fn parent_eligibility(
        &mut self,
        _parent: &TrackerIssue,
        hierarchy: &HierarchySnapshot,
    ) -> Result<ParentEligibilityEvidence, Self::Error> {
        Ok(ParentEligibilityEvidence::tracker_only_for_snapshot(
            hierarchy,
        ))
    }
    async fn parent_repair_snapshot(
        &mut self,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<super::ParentRepairProviderSnapshot>, Self::Error> {
        Ok(None)
    }
    async fn ensure_parent_repair_pull_request(
        &mut self,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<(String, String)>, Self::Error> {
        Ok(None)
    }
    async fn merge_parent_repair(
        &mut self,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<super::ParentRepairProviderSnapshot>, Self::Error> {
        Ok(None)
    }
    async fn request_parent_repair_review(
        &mut self,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<super::ParentRepairProviderSnapshot>, Self::Error> {
        Ok(None)
    }
    fn parent_repair_review_budget_exhausted(&self, _repair: &ParentRepairAttempt) -> bool {
        false
    }
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

    async fn recovered_run_started_at(
        &mut self,
    ) -> Result<BTreeMap<IssueId, TimestampMs>, Self::Error> {
        Ok(BTreeMap::new())
    }

    async fn load_orchestrator_state(&mut self) -> Result<Option<serde_json::Value>, Self::Error> {
        Ok(None)
    }

    async fn persist_orchestrator_state(
        &mut self,
        _state: &serde_json::Value,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn workspace_lease_resource(
        &mut self,
        _issue: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
    ) -> Result<Option<LeaseResource>, Self::Error> {
        Ok(None)
    }

    async fn workspace_has_active_lease(
        &mut self,
        _workspace: &WorkspaceRecord,
    ) -> Result<bool, Self::Error> {
        Ok(false)
    }

    async fn cleanup_target_for_resource(
        &mut self,
        _resource: &LeaseResource,
        _outcome: CleanupTerminalOutcome,
    ) -> Result<Option<CleanupTarget>, Self::Error> {
        Ok(None)
    }

    async fn parent_workspace_targets(
        &mut self,
        _issue: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
    ) -> Result<Vec<ParentRepositoryTarget>, Self::Error> {
        Ok(Vec::new())
    }

    async fn reconcile_parent_repair_branch(
        &mut self,
        _parent: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
        _target: &ParentRepositoryTarget,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<(Option<String>, String)>, Self::Error> {
        Ok(None)
    }

    async fn prepare_parent_repair(
        &mut self,
        _parent: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
        _target: &ParentRepositoryTarget,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }

    async fn reconcile_parent_repair_push(
        &mut self,
        _parent: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
        _target: &ParentRepositoryTarget,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<(String, Option<String>, bool)>, Self::Error> {
        Ok(None)
    }

    async fn publish_parent_repair(
        &mut self,
        _parent: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
        _target: &ParentRepositoryTarget,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<String>, Self::Error> {
        Ok(None)
    }

    async fn refresh_parent_repair(
        &mut self,
        _parent: &NormalizedIssue,
        _workspace: &WorkspaceRecord,
        _target: &ParentRepositoryTarget,
        _repair: &ParentRepairAttempt,
    ) -> Result<Option<(String, PathBuf, String)>, Self::Error> {
        Ok(None)
    }

    async fn recover_retry_exhaustion(
        &mut self,
    ) -> Result<Vec<RetryExhaustionRecord>, Self::Error> {
        Ok(Vec::new())
    }

    async fn recover_retry_pending(&mut self) -> Result<Vec<RetryPendingRecord>, Self::Error> {
        Ok(Vec::new())
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error>;

    async fn cleanup_failed_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<(), Self::Error> {
        self.cleanup_workspace(workspace, true).await
    }

    async fn remove_workspace(&mut self, workspace: &WorkspaceRecord) -> Result<(), Self::Error> {
        self.cleanup_workspace(workspace, true).await
    }

    async fn cleanup_generation(&mut self, target: &CleanupTarget) -> Result<(), Self::Error> {
        self.cleanup_workspace(&target.workspace, true).await
    }

    async fn prepare_cleanup_generation(
        &mut self,
        _target: &CleanupTarget,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn persist_retry_count(
        &mut self,
        _workspace: &WorkspaceRecord,
        _normal_retry_count: u32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn persist_retry_exhaustion(
        &mut self,
        _issue: &NormalizedIssue,
        _normal_retry_count: u32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn clear_retry_exhaustion(&mut self, _identifier: &str) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn persist_retry_pending_without_workspace(
        &mut self,
        _issue: &NormalizedIssue,
        _retry: &RetryEntry,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn clear_retry_pending(&mut self, _issue_id: &IssueId) -> Result<(), Self::Error> {
        Ok(())
    }

    fn revoke_issue_resources(&mut self, _issue_identifier: &str) {}

    async fn persist_retry_pending(
        &mut self,
        _workspace: &WorkspaceRecord,
        _retry: &RetryEntry,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    async fn persist_interrupt_reason(
        &mut self,
        _workspace: &WorkspaceRecord,
        _reason: HarnessInterruptReason,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn retain_failed_workspaces(&self) -> bool {
        false
    }
}

#[allow(async_fn_in_trait)]
pub trait WorkerBackend {
    type Error: std::fmt::Display + Send + Sync + 'static;

    async fn start_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error>;

    async fn recover_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error> {
        self.start_worker(request).await
    }

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
            timed_out: false,
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
    #[error(transparent)]
    ParentIntegration(#[from] ParentIntegrationError),
    #[error("parent repair operation is unavailable: {detail}")]
    ParentRepairUnavailable { detail: String },
}

pub struct Scheduler<T, W, M> {
    tracker: T,
    workspace: W,
    worker: M,
    config: SchedulerConfig,
    executions: BTreeMap<IssueId, IssueExecution>,
    running_counts_by_state: HashMap<String, usize>,
    worker_metadata: HashMap<WorkerId, WorkerMetadata>,
    parent_issue_ids: HashSet<IssueId>,
    terminal_undispatched_parent_ids: HashSet<IssueId>,
    terminal_child_failure_ids: HashSet<IssueId>,
    parent_eligibility_checked_at: BTreeMap<IssueId, TimestampMs>,
    hierarchy_state: DurableOrchestratorState,
    hierarchy_state_dirty: bool,
    durable_state_loaded: bool,
    pending_retry_persistence: BTreeMap<IssueId, RetryEntry>,
    pending_retry_exhaustion_persistence: BTreeMap<IssueId, RetryExhaustionRecord>,
    pending_finished_updates: BTreeMap<IssueId, (IssueExecution, WorkerOutcomeRecord)>,
    pending_recovery: Option<Vec<RecoveryRecord>>,
    pending_retry_exhaustion: Option<Vec<RetryExhaustionRecord>>,
    pending_retry_recovery: Option<Vec<RetryPendingRecord>>,
    recovered_memory_issue_ids: HashSet<IssueId>,
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
    Full {
        issues: Vec<TrackerIssue>,
        reachable_child_edges: BTreeSet<(IssueId, IssueId)>,
    },
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
            worker_metadata: HashMap::new(),
            parent_issue_ids: HashSet::new(),
            terminal_undispatched_parent_ids: HashSet::new(),
            terminal_child_failure_ids: HashSet::new(),
            parent_eligibility_checked_at: BTreeMap::new(),
            hierarchy_state: DurableOrchestratorState::default(),
            hierarchy_state_dirty: false,
            durable_state_loaded: false,
            pending_retry_persistence: BTreeMap::new(),
            pending_retry_exhaustion_persistence: BTreeMap::new(),
            pending_finished_updates: BTreeMap::new(),
            pending_recovery: None,
            pending_retry_exhaustion: None,
            pending_retry_recovery: None,
            recovered_memory_issue_ids: HashSet::new(),
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

    pub async fn begin_parent_repair(
        &mut self,
        parent_id: &IssueId,
        repository_id: &crate::opensymphony_domain::CanonicalRepositoryId,
        defect_key: &str,
        observed_at: TimestampMs,
    ) -> Result<String, SchedulerError> {
        self.load_recovery_state().await?;
        let (parent, workspace) = self
            .executions
            .get(parent_id)
            .and_then(|execution| {
                execution
                    .workspace()
                    .cloned()
                    .map(|workspace| (execution.issue().clone(), workspace))
            })
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no active workspace"),
            })?;
        let legacy_target = self
            .hierarchy_state
            .parent_integrations
            .get(parent_id)
            .and_then(|controller| controller.targets.get(repository_id))
            .is_some_and(|target| {
                target.target_branch.is_empty()
                    && target.repair_policy == ParentRepairPolicy::default()
            });
        if legacy_target {
            let current_target = self
                .workspace
                .parent_workspace_targets(&parent, &workspace)
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?
                .into_iter()
                .find(|target| target.repository_id == *repository_id)
                .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                    detail: format!("parent {parent_id} has no migrated target {repository_id}"),
                })?;
            let previous = self.hierarchy_state.clone();
            self.hierarchy_state
                .parent_integrations
                .get_mut(parent_id)
                .expect("controller was checked")
                .migrate_legacy_repair_target(current_target)?;
            if let Err(error) = self.persist_orchestrator_state().await {
                self.hierarchy_state = previous;
                return Err(error);
            }
        }
        let (target, input_version) = {
            let controller = self
                .hierarchy_state
                .parent_integrations
                .get(parent_id)
                .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                    detail: format!("parent {parent_id} has no integration controller"),
                })?;
            (
                controller
                    .targets
                    .get(repository_id)
                    .cloned()
                    .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                        detail: format!("parent {parent_id} has no target {repository_id}"),
                    })?,
                parent_controller_input_version(controller),
            )
        };
        let previous = self.hierarchy_state.clone();
        let resource = self
            .hierarchy_state
            .descendant_resources_for(parent_id)
            .into_iter()
            .find(|resource| resource.repository_id == *repository_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no leased resource for {repository_id}"),
            })?;
        let repair_id = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .expect("controller was checked")
            .begin_repair(
                defect_key,
                repository_id.clone(),
                &target.checkout_handle,
                &target.target_commit,
                &input_version,
                observed_at,
            )?;
        if let Err(error) = self.hierarchy_state.acquire_leases(vec![LeaseRecord {
            kind: super::LeaseKind::Repair,
            resource,
            owner: super::LeaseOwner::repair(parent_id),
            hierarchy_generation: self
                .hierarchy_state
                .parent_integrations
                .get(parent_id)
                .expect("controller was checked")
                .hierarchy_generation,
            acquired_at: observed_at.as_u64(),
            expires_at: None,
            released_at: None,
        }]) {
            self.hierarchy_state = previous;
            return Err(SchedulerError::Workspace {
                detail: error.to_string(),
            });
        }
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }

        let repair = self.repair(parent_id, &repair_id)?.clone();
        if repair.status != ParentRepairStatus::PreparingBranch {
            return Ok(repair_id);
        }
        if !repair.operations.iter().any(|operation| {
            operation.kind == ParentProviderOperationKind::ReconcileBranch
                && operation.receipt.is_none()
        }) {
            self.persist_repair_intent(
                parent_id,
                &repair_id,
                ParentProviderOperationKind::ReconcileBranch,
                &input_version,
                observed_at,
            )
            .await?;
        }
        let (branch_head, instruction_hash) = self
            .workspace
            .reconcile_parent_repair_branch(&parent, &workspace, &target, &repair)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: "workspace backend does not support parent repairs".to_owned(),
            })?;
        if instruction_hash != repair.instruction_hash {
            return Err(SchedulerError::ParentRepairUnavailable {
                detail: format!(
                    "repository instructions changed before repair: expected {}, got {}",
                    repair.instruction_hash, instruction_hash
                ),
            });
        }
        self.complete_repair_operation(
            parent_id,
            &repair_id,
            ParentProviderOperationKind::ReconcileBranch,
            if branch_head.is_some() {
                "found"
            } else {
                "missing"
            },
            observed_at,
        )
        .await?;
        if let Some(branch_head) = branch_head {
            if branch_head != repair.target_commit
                && repair.pushed_commit.as_deref() != Some(&branch_head)
            {
                return Err(SchedulerError::ParentRepairUnavailable {
                    detail: format!(
                        "repair branch {} points at an unrecorded commit",
                        repair.branch
                    ),
                });
            }
            if repair.operations.iter().any(|operation| {
                operation.kind == ParentProviderOperationKind::CreateBranch
                    && operation.receipt.is_none()
            }) {
                self.complete_repair_operation(
                    parent_id,
                    &repair_id,
                    ParentProviderOperationKind::CreateBranch,
                    "reconciled",
                    observed_at,
                )
                .await?;
            }
        } else {
            self.persist_repair_intent(
                parent_id,
                &repair_id,
                ParentProviderOperationKind::CreateBranch,
                &input_version,
                observed_at,
            )
            .await?;
            let created_instruction_hash = self
                .workspace
                .prepare_parent_repair(&parent, &workspace, &target, &repair)
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?
                .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                    detail: "workspace backend does not support branch creation".to_owned(),
                })?;
            if created_instruction_hash != repair.instruction_hash {
                return Err(SchedulerError::ParentRepairUnavailable {
                    detail: "repository instructions changed during branch creation".to_owned(),
                });
            }
            self.complete_repair_operation(
                parent_id,
                &repair_id,
                ParentProviderOperationKind::CreateBranch,
                "created",
                observed_at,
            )
            .await?;
        }
        let previous = self.hierarchy_state.clone();
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .expect("controller was checked");
        controller.record_repair_branch_ready(&repair_id)?;
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(repair_id)
    }

    pub async fn publish_parent_repair(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        observed_at: TimestampMs,
    ) -> Result<String, SchedulerError> {
        let (parent, workspace) = self.parent_context(parent_id)?;
        let (target, repair, input_version) = self.repair_context(parent_id, repair_id)?;
        let publication_input_version = format!(
            "{input_version};repair-cycle:{}",
            repair.requested_change_count
        );
        self.persist_repair_intent(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcilePush,
            &publication_input_version,
            observed_at,
        )
        .await?;
        let (local_commit, remote_commit, has_uncommitted_changes) = self
            .workspace
            .reconcile_parent_repair_push(&parent, &workspace, &target, &repair)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: "workspace backend does not support repair push reconciliation".to_owned(),
            })?;
        self.complete_repair_operation(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcilePush,
            if remote_commit.is_some() {
                "found"
            } else {
                "missing"
            },
            observed_at,
        )
        .await?;
        if let Some(remote_commit) = remote_commit.as_deref()
            && remote_commit != local_commit
            && repair.pushed_commit.as_deref() != Some(remote_commit)
        {
            return Err(SchedulerError::ParentRepairUnavailable {
                detail: "remote repair branch was force-pushed".to_owned(),
            });
        }
        if !has_uncommitted_changes && local_commit == repair.target_commit {
            self.mark_parent_repair_implementation_required(parent_id, repair_id)
                .await?;
            return Err(SchedulerError::ParentRepairUnavailable {
                detail: "repair branch has no commit beyond its recorded target".to_owned(),
            });
        }
        let publish_required =
            has_uncommitted_changes || remote_commit.as_deref() != Some(local_commit.as_str());
        let commit = if !publish_required {
            if repair.operations.iter().any(|operation| {
                operation.kind == ParentProviderOperationKind::Push
                    && operation.input_version == publication_input_version
                    && operation.receipt.is_none()
            }) {
                self.complete_repair_operation(
                    parent_id,
                    repair_id,
                    ParentProviderOperationKind::Push,
                    "reconciled",
                    observed_at,
                )
                .await?;
            }
            local_commit
        } else {
            self.persist_repair_intent(
                parent_id,
                repair_id,
                ParentProviderOperationKind::Push,
                &publication_input_version,
                observed_at,
            )
            .await?;
            let commit = self
                .workspace
                .publish_parent_repair(&parent, &workspace, &target, &repair)
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?
                .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                    detail: "workspace backend does not support repair publication".to_owned(),
                })?;
            if commit == repair.target_commit {
                self.mark_parent_repair_implementation_required(parent_id, repair_id)
                    .await?;
                return Err(SchedulerError::ParentRepairUnavailable {
                    detail: "repair branch has no commit beyond its recorded target".to_owned(),
                });
            }
            self.complete_repair_operation(
                parent_id,
                repair_id,
                ParentProviderOperationKind::Push,
                "pushed",
                observed_at,
            )
            .await?;
            commit
        };
        {
            let previous = self.hierarchy_state.clone();
            self.hierarchy_state
                .parent_integrations
                .get_mut(parent_id)
                .expect("controller was checked")
                .record_repair_push(repair_id, &commit, &publication_input_version, observed_at)?;
            if let Err(error) = self.persist_orchestrator_state().await {
                self.hierarchy_state = previous;
                return Err(error);
            }
        }

        self.persist_repair_intent(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcilePullRequest,
            &publication_input_version,
            observed_at,
        )
        .await?;
        let repair = self.repair(parent_id, repair_id)?.clone();
        let snapshot = match self.tracker.parent_repair_snapshot(&repair).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_linear_cooldown_from_tracker_error(&error, observed_at);
                self.apply_parent_repair_snapshot(
                    parent_id,
                    repair_id,
                    unavailable_provider_snapshot(&repair),
                    &input_version,
                    observed_at,
                )
                .await?;
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };
        let pull_request = match snapshot
            .and_then(|snapshot| snapshot.pull_request_id.zip(snapshot.pull_request_url))
        {
            Some(pull_request) => {
                self.complete_repair_operation(
                    parent_id,
                    repair_id,
                    ParentProviderOperationKind::ReconcilePullRequest,
                    "found",
                    observed_at,
                )
                .await?;
                pull_request
            }
            None => {
                self.complete_repair_operation(
                    parent_id,
                    repair_id,
                    ParentProviderOperationKind::ReconcilePullRequest,
                    "missing",
                    observed_at,
                )
                .await?;
                self.persist_repair_intent(
                    parent_id,
                    repair_id,
                    ParentProviderOperationKind::CreatePullRequest,
                    &publication_input_version,
                    observed_at,
                )
                .await?;
                self.tracker
                    .ensure_parent_repair_pull_request(&repair)
                    .await
                    .map_err(|error| self.tracker_operation_error(error, observed_at))?
                    .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                        detail: "provider backend does not support repair pull requests".to_owned(),
                    })?
            }
        };
        let previous = self.hierarchy_state.clone();
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .expect("controller was checked");
        controller.record_repair_pull_request(
            repair_id,
            &pull_request.0,
            &pull_request.1,
            &publication_input_version,
            observed_at,
        )?;
        if controller
            .repair(repair_id)?
            .operations
            .iter()
            .any(|operation| {
                operation.kind == ParentProviderOperationKind::CreatePullRequest
                    && operation.receipt.is_none()
            })
        {
            let key = controller
                .repair(repair_id)?
                .operations
                .iter()
                .find(|operation| {
                    operation.kind == ParentProviderOperationKind::CreatePullRequest
                        && operation.receipt.is_none()
                })
                .expect("checked pending create operation")
                .idempotency_key
                .clone();
            controller.record_provider_operation(
                repair_id,
                &key,
                "created",
                Some(pull_request.0.clone()),
                observed_at,
            )?;
        }
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(pull_request.1)
    }

    pub async fn request_parent_repair_review(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        observed_at: TimestampMs,
    ) -> Result<ParentRepairStatus, SchedulerError> {
        let (_, repair, input_version) = self.repair_context(parent_id, repair_id)?;
        let review_input_version = format!(
            "{input_version};repair-head:{}",
            repair.pushed_commit.as_deref().unwrap_or("missing")
        );
        self.persist_repair_intent(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcileReview,
            &review_input_version,
            observed_at,
        )
        .await?;
        let snapshot = self
            .tracker
            .parent_repair_snapshot(&repair)
            .await
            .map_err(|error| self.tracker_operation_error(error, observed_at))?
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: "provider backend does not support repair review reconciliation".to_owned(),
            })?;
        let already_reviewed = snapshot.review_head_commit.as_deref()
            == repair.pushed_commit.as_deref()
            && (snapshot.review_approved || snapshot.review_rejected || snapshot.changes_requested);
        self.complete_repair_operation_with_detail(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcileReview,
            if already_reviewed { "found" } else { "pending" },
            snapshot.review_request_cursor.clone(),
            observed_at,
        )
        .await?;
        let snapshot = if already_reviewed {
            snapshot
        } else {
            if self.tracker.parent_repair_review_budget_exhausted(&repair) {
                let previous = self.hierarchy_state.clone();
                let controller = self
                    .hierarchy_state
                    .parent_integrations
                    .get_mut(parent_id)
                    .expect("controller was checked");
                controller.record_repair_review_budget_exhausted(
                    repair_id,
                    &review_input_version,
                    observed_at,
                )?;
                if let Err(error) = self.persist_orchestrator_state().await {
                    self.hierarchy_state = previous;
                    return Err(error);
                }
                return Ok(ParentRepairStatus::ReviewBudgetExhausted);
            }
            self.persist_repair_intent(
                parent_id,
                repair_id,
                ParentProviderOperationKind::RequestReview,
                &review_input_version,
                observed_at,
            )
            .await?;
            let repair = self.repair(parent_id, repair_id)?.clone();
            let snapshot = self
                .tracker
                .request_parent_repair_review(&repair)
                .await
                .map_err(|error| self.tracker_operation_error(error, observed_at))?
                .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                    detail: "provider backend does not support repair review requests".to_owned(),
                })?;
            self.complete_repair_operation(
                parent_id,
                repair_id,
                ParentProviderOperationKind::RequestReview,
                "requested",
                observed_at,
            )
            .await?;
            snapshot
        };
        self.apply_parent_repair_snapshot(
            parent_id,
            repair_id,
            snapshot,
            &input_version,
            observed_at,
        )
        .await
    }

    async fn advance_parent_repairs(
        &mut self,
        observed_at: TimestampMs,
        freshly_inactive_issue_ids: &HashSet<IssueId>,
    ) -> Result<(), SchedulerError> {
        let repairs = self
            .hierarchy_state
            .parent_integrations
            .iter()
            .flat_map(|(parent_id, controller)| {
                controller
                    .repair_attempts
                    .iter()
                    .filter(|repair| repair.status != ParentRepairStatus::Completed)
                    .map(move |repair| {
                        (
                            parent_id.clone(),
                            repair.id.clone(),
                            repair.repository_id.clone(),
                            repair.status,
                            repair
                                .operations
                                .first()
                                .map(|operation| operation.idempotency_key.clone())
                                .unwrap_or_default(),
                            repair.pushed_commit.clone(),
                            repair.operations.clone(),
                        )
                    })
            })
            .collect::<Vec<_>>();
        for (parent_id, repair_id, repository_id, status, defect_key, pushed, operations) in repairs
        {
            if self.linear_cooldown_active(observed_at) {
                break;
            }
            if !self.parent_repair_advancement_allowed(&parent_id) {
                continue;
            }
            if freshly_inactive_issue_ids.contains(&parent_id) {
                continue;
            }
            let advancement = async {
                match status {
                    ParentRepairStatus::PreparingBranch => {
                        self.begin_parent_repair(
                            &parent_id,
                            &repository_id,
                            &defect_key,
                            observed_at,
                        )
                        .await?;
                    }
                    ParentRepairStatus::Implementing => {
                        let implementation_completed = self
                            .repair(&parent_id, &repair_id)?
                            .implementation_completed;
                        let execution_released =
                            self.executions.get(&parent_id).is_some_and(|execution| {
                                execution.status() == SchedulerStatus::Released
                            });
                        if implementation_completed && execution_released {
                            self.publish_parent_repair(&parent_id, &repair_id, observed_at)
                                .await?;
                        } else if !implementation_completed {
                            self.queue_parent_repair_retry(&parent_id, observed_at)
                                .await?;
                        }
                    }
                    ParentRepairStatus::AwaitingPullRequest => {
                        self.publish_parent_repair(&parent_id, &repair_id, observed_at)
                            .await?;
                    }
                    ParentRepairStatus::AwaitingReview => {
                        let (_, _, input_version) = self.repair_context(&parent_id, &repair_id)?;
                        let review_input_version = format!(
                            "{input_version};repair-head:{}",
                            pushed.as_deref().unwrap_or("missing")
                        );
                        let requested = operations.iter().any(|operation| {
                            operation.kind == ParentProviderOperationKind::RequestReview
                                && operation.input_version == review_input_version
                                && operation.receipt.is_some()
                        });
                        if requested {
                            self.reconcile_parent_repair(&parent_id, &repair_id, observed_at)
                                .await?;
                        } else {
                            self.request_parent_repair_review(&parent_id, &repair_id, observed_at)
                                .await?;
                        }
                    }
                    ParentRepairStatus::AwaitingMerge => {
                        self.merge_parent_repair(&parent_id, &repair_id, observed_at)
                            .await?;
                    }
                    ParentRepairStatus::Refreshing => {
                        self.refresh_parent_repair(&parent_id, &repair_id, observed_at)
                            .await?;
                    }
                    ParentRepairStatus::ChangesRequested => {
                        self.queue_parent_repair_retry(&parent_id, observed_at)
                            .await?;
                    }
                    ParentRepairStatus::FailedChecks
                    | ParentRepairStatus::ReviewRejected
                    | ParentRepairStatus::ReviewBudgetExhausted
                    | ParentRepairStatus::ProviderUnavailable
                    | ParentRepairStatus::ExternallyClosed
                    | ParentRepairStatus::ForcePushed
                    | ParentRepairStatus::MergeConflict => {
                        self.reconcile_parent_repair(&parent_id, &repair_id, observed_at)
                            .await?;
                    }
                    ParentRepairStatus::Completed => {}
                }
                Ok::<(), SchedulerError>(())
            }
            .await;
            if let Err(error) = advancement {
                let detail = redact_runtime_diagnostic(&error.to_string());
                warn!(
                    parent_id = %parent_id,
                    repair_id,
                    error = %detail,
                    "parent repair advancement failed; preserving the repair stage for retry"
                );
                let previous = self.hierarchy_state.clone();
                if let Some(controller) =
                    self.hierarchy_state.parent_integrations.get_mut(&parent_id)
                {
                    if let Err(record_error) = controller.record_repair_advancement_failure(
                        &repair_id,
                        &detail,
                        observed_at,
                    ) {
                        warn!(
                            parent_id = %parent_id,
                            repair_id,
                            error = %record_error,
                            "failed to record parent repair advancement failure"
                        );
                        continue;
                    }
                    if let Err(persist_error) = self.persist_orchestrator_state().await {
                        self.hierarchy_state = previous;
                        self.hierarchy_state_dirty = true;
                        warn!(
                            parent_id = %parent_id,
                            repair_id,
                            error = %persist_error,
                            "failed to persist parent repair advancement failure"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    fn parent_repair_advancement_allowed(&self, parent_id: &IssueId) -> bool {
        let Some(execution) = self.executions.get(parent_id) else {
            return false;
        };
        if execution.workspace().is_none()
            || execution.issue().state.category != IssueStateCategory::Active
            || matches!(
                execution.state(),
                crate::opensymphony_domain::SchedulerState::Released {
                    reason: ReleaseReason::TrackerInactive
                        | ReleaseReason::TrackerTerminal
                        | ReleaseReason::Cancelled
                        | ReleaseReason::RetryExhausted,
                    ..
                }
            )
        {
            return false;
        }
        let Some(controller) = self.hierarchy_state.parent_integrations.get(parent_id) else {
            return false;
        };
        !controller.state.terminal()
            && self
                .hierarchy_state
                .hierarchy
                .get(parent_id)
                .is_some_and(|snapshot| snapshot.accepts_event(controller.hierarchy_generation))
    }

    async fn queue_parent_repair_retry(
        &mut self,
        parent_id: &IssueId,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let Some(execution) = self.remove_execution(parent_id) else {
            return Ok(());
        };
        if execution.status() == SchedulerStatus::RetryQueued {
            self.insert_execution(parent_id.clone(), execution);
            return Ok(());
        }
        if execution.status() != SchedulerStatus::Released {
            self.insert_execution(parent_id.clone(), execution);
            return Ok(());
        }
        let previous_attempt = execution
            .last_worker_outcome()
            .and_then(|outcome| outcome.attempt);
        let normal_retry_count = previous_attempt.map_or(0, RetryAttempt::get);
        if self.retry_limit_reached(normal_retry_count) {
            if let Err(error) = self
                .persist_retry_exhaustion(execution.issue(), normal_retry_count)
                .await
            {
                self.insert_execution(parent_id.clone(), execution);
                return Err(error);
            }
            let exhausted = execution.clone().reopen(observed_at).and_then(|execution| {
                execution.release(observed_at, ReleaseReason::RetryExhausted, None)
            });
            let mut exhausted = match exhausted {
                Ok(exhausted) => exhausted,
                Err(error) => {
                    self.insert_execution(parent_id.clone(), execution);
                    return Err(error.into());
                }
            };
            exhausted.set_retry_count_override(normal_retry_count);
            self.insert_execution(parent_id.clone(), exhausted);
            return Ok(());
        }
        let next_execution = (|| -> Result<IssueExecution, SchedulerError> {
            let retry = RetryEntry::continuation(
                execution.issue(),
                previous_attempt,
                normal_retry_count,
                observed_at,
                self.config.retry_policy,
            )?;
            Ok(execution
                .clone()
                .reopen(observed_at)?
                .restore_retry(retry)?)
        })();
        let next_execution = match next_execution {
            Ok(execution) => execution,
            Err(error) => {
                self.insert_execution(parent_id.clone(), execution);
                return Err(error);
            }
        };
        self.insert_execution(parent_id.clone(), next_execution);
        self.persist_retry_if_queued(parent_id).await
    }

    pub async fn reconcile_parent_repair(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        observed_at: TimestampMs,
    ) -> Result<ParentRepairStatus, SchedulerError> {
        let (_, repair, input_version) = self.repair_context(parent_id, repair_id)?;
        let snapshot = match self.tracker.parent_repair_snapshot(&repair).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_linear_cooldown_from_tracker_error(&error, observed_at);
                return self
                    .apply_parent_repair_snapshot(
                        parent_id,
                        repair_id,
                        unavailable_provider_snapshot(&repair),
                        &input_version,
                        observed_at,
                    )
                    .await;
            }
        }
        .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
            detail: "provider backend does not support repair reconciliation".to_owned(),
        })?;
        let previous = self.hierarchy_state.clone();
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .expect("controller was checked");
        controller.reconcile_repair_provider(repair_id, snapshot, &input_version, observed_at)?;
        let status = controller.repair(repair_id)?.status;
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(status)
    }

    pub async fn merge_parent_repair(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        observed_at: TimestampMs,
    ) -> Result<ParentRepairStatus, SchedulerError> {
        let (_, repair, input_version) = self.repair_context(parent_id, repair_id)?;
        if repair.status != ParentRepairStatus::AwaitingMerge {
            return Err(SchedulerError::ParentRepairUnavailable {
                detail: format!("repair {repair_id} has not satisfied central review policy"),
            });
        }
        let merge_input_version = format!(
            "{input_version};repair-head:{}",
            repair.pushed_commit.as_deref().unwrap_or("missing")
        );
        self.persist_repair_intent(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcileMerge,
            &merge_input_version,
            observed_at,
        )
        .await?;
        let snapshot = match self.tracker.parent_repair_snapshot(&repair).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.set_linear_cooldown_from_tracker_error(&error, observed_at);
                return self
                    .apply_parent_repair_snapshot(
                        parent_id,
                        repair_id,
                        unavailable_provider_snapshot(&repair),
                        &input_version,
                        observed_at,
                    )
                    .await
                    .and(Err(SchedulerError::Tracker {
                        detail: error.to_string(),
                    }));
            }
        }
        .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
            detail: "provider backend does not support repair merge reconciliation".to_owned(),
        })?;
        if snapshot.merged {
            self.complete_repair_operation(
                parent_id,
                repair_id,
                ParentProviderOperationKind::ReconcileMerge,
                "already_merged",
                observed_at,
            )
            .await?;
            return self
                .apply_parent_repair_snapshot(
                    parent_id,
                    repair_id,
                    snapshot,
                    &input_version,
                    observed_at,
                )
                .await;
        }
        self.complete_repair_operation(
            parent_id,
            repair_id,
            ParentProviderOperationKind::ReconcileMerge,
            "open",
            observed_at,
        )
        .await?;
        let current_status = self
            .apply_parent_repair_snapshot(
                parent_id,
                repair_id,
                snapshot,
                &input_version,
                observed_at,
            )
            .await?;
        if current_status != ParentRepairStatus::AwaitingMerge {
            return Ok(current_status);
        }
        self.persist_repair_intent(
            parent_id,
            repair_id,
            ParentProviderOperationKind::Merge,
            &merge_input_version,
            observed_at,
        )
        .await?;
        let snapshot = self
            .tracker
            .merge_parent_repair(&repair)
            .await
            .map_err(|error| self.tracker_operation_error(error, observed_at))?
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: "provider backend does not support repair merge".to_owned(),
            })?;
        self.apply_parent_repair_snapshot(
            parent_id,
            repair_id,
            snapshot,
            &input_version,
            observed_at,
        )
        .await
    }

    async fn mark_parent_repair_implementation_required(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
    ) -> Result<(), SchedulerError> {
        let previous = self.hierarchy_state.clone();
        self.hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no integration controller"),
            })?
            .record_repair_implementation_required(repair_id)?;
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(())
    }

    pub async fn refresh_parent_repair(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        observed_at: TimestampMs,
    ) -> Result<String, SchedulerError> {
        let (parent, workspace) = self.parent_context(parent_id)?;
        let (target, repair, input_version) = self.repair_context(parent_id, repair_id)?;
        if repair.status != ParentRepairStatus::Refreshing {
            return Err(SchedulerError::ParentRepairUnavailable {
                detail: format!("repair {repair_id} has no merged target to refresh"),
            });
        }
        self.persist_repair_intent(
            parent_id,
            repair_id,
            ParentProviderOperationKind::RefreshTarget,
            &input_version,
            observed_at,
        )
        .await?;
        let (refreshed, refreshed_instruction_path, refreshed_instruction_hash) = self
            .workspace
            .refresh_parent_repair(&parent, &workspace, &target, &repair)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: "workspace backend does not support post-repair refresh".to_owned(),
            })?;
        let previous = self.hierarchy_state.clone();
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .expect("controller was checked");
        let key = controller
            .repair(repair_id)?
            .operations
            .iter()
            .find(|operation| {
                operation.kind == ParentProviderOperationKind::RefreshTarget
                    && operation.receipt.is_none()
            })
            .expect("refresh intent was persisted")
            .idempotency_key
            .clone();
        controller.record_provider_operation(
            repair_id,
            &key,
            "refreshed",
            Some(refreshed.clone()),
            observed_at,
        )?;
        controller.record_repair_refresh(
            repair_id,
            &refreshed,
            &refreshed_instruction_path,
            &refreshed_instruction_hash,
            &input_version,
            observed_at,
        )?;
        if let Some(attempt_id) = controller
            .attempts
            .iter()
            .rev()
            .find(|attempt| attempt.status != ParentAttemptStatus::Running)
            .map(|attempt| attempt.id.clone())
        {
            controller.prepare_retry(
                &attempt_id,
                "merged parent repair refreshed; run final verification on the new target",
                observed_at,
            )?;
        }
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        self.queue_parent_repair_retry(parent_id, observed_at)
            .await?;
        Ok(refreshed)
    }

    fn parent_context(
        &self,
        parent_id: &IssueId,
    ) -> Result<(NormalizedIssue, WorkspaceRecord), SchedulerError> {
        let execution = self.executions.get(parent_id).ok_or_else(|| {
            SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no active execution"),
            }
        })?;
        let workspace = execution.workspace().cloned().ok_or_else(|| {
            SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no active workspace"),
            }
        })?;
        Ok((execution.issue().clone(), workspace))
    }

    fn repair(
        &self,
        parent_id: &IssueId,
        repair_id: &str,
    ) -> Result<&ParentRepairAttempt, SchedulerError> {
        self.hierarchy_state
            .parent_integrations
            .get(parent_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no integration controller"),
            })?
            .repair(repair_id)
            .map_err(Into::into)
    }

    fn repair_context(
        &self,
        parent_id: &IssueId,
        repair_id: &str,
    ) -> Result<(ParentRepositoryTarget, ParentRepairAttempt, String), SchedulerError> {
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get(parent_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no integration controller"),
            })?;
        let repair = controller.repair(repair_id)?.clone();
        let target = controller
            .targets
            .get(&repair.repository_id)
            .cloned()
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("repair {repair_id} target is unavailable"),
            })?;
        Ok((target, repair, parent_controller_input_version(controller)))
    }

    async fn persist_repair_intent(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        kind: ParentProviderOperationKind,
        input_version: &str,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let previous = self.hierarchy_state.clone();
        self.hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no integration controller"),
            })?
            .begin_provider_operation(repair_id, kind, input_version, observed_at)?;
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(())
    }

    async fn complete_repair_operation(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        kind: ParentProviderOperationKind,
        status: &str,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        self.complete_repair_operation_with_detail(
            parent_id,
            repair_id,
            kind,
            status,
            None,
            observed_at,
        )
        .await
    }

    async fn complete_repair_operation_with_detail(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        kind: ParentProviderOperationKind,
        status: &str,
        detail: Option<String>,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let previous = self.hierarchy_state.clone();
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no integration controller"),
            })?;
        let key = controller
            .repair(repair_id)?
            .operations
            .iter()
            .find(|operation| operation.kind == kind && operation.receipt.is_none())
            .map(|operation| operation.idempotency_key.clone())
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("repair {repair_id} has no pending {kind:?} intent"),
            })?;
        controller.record_provider_operation(repair_id, &key, status, detail, observed_at)?;
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(())
    }

    async fn apply_parent_repair_snapshot(
        &mut self,
        parent_id: &IssueId,
        repair_id: &str,
        snapshot: super::ParentRepairProviderSnapshot,
        input_version: &str,
        observed_at: TimestampMs,
    ) -> Result<ParentRepairStatus, SchedulerError> {
        let previous = self.hierarchy_state.clone();
        let controller = self
            .hierarchy_state
            .parent_integrations
            .get_mut(parent_id)
            .ok_or_else(|| SchedulerError::ParentRepairUnavailable {
                detail: format!("parent {parent_id} has no integration controller"),
            })?;
        controller.reconcile_repair_provider(repair_id, snapshot, input_version, observed_at)?;
        let status = controller.repair(repair_id)?.status;
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous;
            return Err(error);
        }
        Ok(status)
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

        let mut snapshot = OrchestratorSnapshot::new(generated_at, daemon, issues);
        snapshot.hierarchy = self
            .hierarchy_state
            .hierarchy
            .iter()
            .map(|(issue_id, hierarchy)| {
                (
                    issue_id.as_str().to_owned(),
                    crate::opensymphony_domain::HierarchyStateSnapshot {
                        generation: hierarchy.generation,
                        blocked_reason: hierarchy
                            .blocked_reason
                            .as_ref()
                            .or(hierarchy.eligibility_blocked_reason.as_ref())
                            .map(|reason| format!("{reason:?}")),
                    },
                )
            })
            .collect();
        snapshot
    }

    pub async fn bootstrap(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<OrchestratorSnapshot, SchedulerError> {
        self.load_recovery_state().await?;

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
        self.load_recovery_state().await?;
        self.flush_pending_retry_persistence().await?;
        self.flush_pending_retry_exhaustion_persistence().await?;
        self.reconcile_parent_subtree_cleanup(observed_at).await?;

        self.expire_linear_cooldown(observed_at);
        let mut pre_update_full_snapshot = if !self.linear_cooldown_active(observed_at)
            && due(
                self.last_full_detail_refresh_at,
                FULL_DETAIL_REFRESH_INTERVAL_MS,
                observed_at,
            ) {
            if let Some(tracker_snapshot) = self.load_tracker_snapshot(observed_at).await? {
                self.record_full_detail_refresh(observed_at);
                let hierarchy_changed = self.reconcile_hierarchy_snapshots(&tracker_snapshot)?;
                let terminal_hierarchy_changed =
                    self.reconcile_terminal_hierarchy_snapshots(&tracker_snapshot)?;
                if hierarchy_changed || terminal_hierarchy_changed || self.hierarchy_state_dirty {
                    self.persist_orchestrator_state().await?;
                }
                self.fence_hierarchy_changed_runs(observed_at).await?;
                Some(tracker_snapshot)
            } else {
                None
            }
        } else {
            None
        };

        if !self.linear_cooldown_active(observed_at)
            && pre_update_full_snapshot.is_none()
            && !self.pending_finished_updates.is_empty()
            && let Some(tracker_snapshot) = self.load_tracker_snapshot(observed_at).await?
        {
            self.record_full_detail_refresh(observed_at);
            let hierarchy_changed = self.reconcile_hierarchy_snapshots(&tracker_snapshot)?;
            let terminal_hierarchy_changed =
                self.reconcile_terminal_hierarchy_snapshots(&tracker_snapshot)?;
            if hierarchy_changed || terminal_hierarchy_changed || self.hierarchy_state_dirty {
                self.persist_orchestrator_state().await?;
            }
            self.fence_hierarchy_changed_runs(observed_at).await?;
            pre_update_full_snapshot = Some(tracker_snapshot);
        }

        if !self.linear_cooldown_active(observed_at)
            && pre_update_full_snapshot.is_none()
            && due(
                self.last_running_state_refresh_at,
                RUNNING_STATE_REFRESH_INTERVAL_MS,
                observed_at,
            )
            && self.has_reconcilable_executions()
        {
            self.refresh_running_issue_states(observed_at).await?;
        }

        if pre_update_full_snapshot.is_some() {
            self.flush_pending_finished_updates().await?;
        }

        let updates = self
            .worker
            .poll_updates()
            .await
            .map_err(|error| SchedulerError::Worker {
                detail: error.to_string(),
            })?;
        self.apply_worker_updates(updates).await?;
        if !self.linear_cooldown_active(observed_at) {
            let freshly_inactive_issue_ids = pre_update_full_snapshot
                .as_ref()
                .map(|snapshot| {
                    self.hierarchy_state
                        .parent_integrations
                        .keys()
                        .filter(|parent_id| !snapshot.contains_active(parent_id.as_str()))
                        .cloned()
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            self.advance_parent_repairs(observed_at, &freshly_inactive_issue_ids)
                .await?;
        }

        let mut dispatch_candidates = None;
        if let Some(tracker_snapshot) = pre_update_full_snapshot.as_ref() {
            self.bootstrap_recovery(tracker_snapshot, observed_at)
                .await?;
            self.reconcile_tracker_state(tracker_snapshot, observed_at)
                .await?;
            dispatch_candidates = Some(DispatchCandidates::Full {
                issues: tracker_snapshot.active.clone(),
                reachable_child_edges: self
                    .required_child_edges_in_tracker_snapshot(tracker_snapshot),
            });
        }
        if !self.linear_cooldown_active(observed_at) {
            if pre_update_full_snapshot.is_none()
                && due(
                    self.last_full_detail_refresh_at,
                    FULL_DETAIL_REFRESH_INTERVAL_MS,
                    observed_at,
                )
            {
                if let Some(tracker_snapshot) = self.load_tracker_snapshot(observed_at).await? {
                    self.record_full_detail_refresh(observed_at);
                    self.bootstrap_recovery(&tracker_snapshot, observed_at)
                        .await?;
                    self.reconcile_tracker_state(&tracker_snapshot, observed_at)
                        .await?;
                    dispatch_candidates = Some(DispatchCandidates::Full {
                        reachable_child_edges: self
                            .required_child_edges_in_tracker_snapshot(&tracker_snapshot),
                        issues: tracker_snapshot.active,
                    });
                }
            } else if due(
                self.last_terminal_refresh_at,
                TERMINAL_REFRESH_INTERVAL_MS,
                observed_at,
            ) {
                self.refresh_terminal_issues(observed_at).await?;
            }
            if due(
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
                DispatchCandidates::Full {
                    issues,
                    reachable_child_edges,
                } => {
                    self.dispatch_ready_issues(&issues, observed_at, Some(&reachable_child_edges))
                        .await?;
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
            self.dispatch_ready_issues(&known_candidates, observed_at, None)
                .await?;
        }

        self.last_poll_at = Some(observed_at);
        self.refresh_health_from_linear_cooldown(observed_at);
        Ok(self.snapshot(observed_at))
    }

    pub async fn acknowledge_terminal_capture(
        &mut self,
        issue_identifiers: &[String],
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let captured = issue_identifiers.iter().collect::<BTreeSet<_>>();
        let parent_ids = self
            .executions
            .iter()
            .filter(|(_, execution)| {
                captured.contains(&execution.issue().identifier.to_string())
                    && self
                        .hierarchy_state
                        .parent_integrations
                        .contains_key(&execution.issue().id)
            })
            .map(|(issue_id, _)| issue_id.clone())
            .collect::<Vec<_>>();
        let mut changed = false;
        for parent_id in parent_ids {
            let Some(execution) = self.executions.get(&parent_id).cloned() else {
                continue;
            };
            let Some(parent_workspace) = execution.workspace().cloned() else {
                continue;
            };
            let Some(controller) = self
                .hierarchy_state
                .parent_integrations
                .get(&parent_id)
                .cloned()
            else {
                continue;
            };
            if controller.subtree_cleanup.is_some() {
                continue;
            }
            if controller.has_unreconciled_harness() {
                return Err(SchedulerError::Workspace {
                    detail: format!(
                        "captured parent {parent_id} still has an unreconciled harness cleanup fence"
                    ),
                });
            }
            let outcome = match controller.state {
                super::ParentIntegrationState::Completed => {
                    if controller.final_evidence.is_none()
                        || controller.repair_attempts.iter().any(|repair| {
                            repair.status != ParentRepairStatus::Completed
                                || repair.operations.iter().any(|operation| {
                                    operation.receipt.is_none() || operation.completed_at.is_none()
                                })
                        })
                    {
                        return Err(SchedulerError::Workspace {
                            detail: format!(
                                "captured parent {parent_id} is missing final evidence or repair receipts"
                            ),
                        });
                    }
                    CleanupTerminalOutcome::Succeeded
                }
                super::ParentIntegrationState::Failed { .. } => CleanupTerminalOutcome::Failed,
                super::ParentIntegrationState::Canceled { .. } => CleanupTerminalOutcome::Canceled,
                _ => {
                    return Err(SchedulerError::Workspace {
                        detail: format!(
                            "captured parent {parent_id} has not reached a terminal controller state"
                        ),
                    });
                }
            };
            let retained = outcome != CleanupTerminalOutcome::Succeeded
                && self.workspace.retain_failed_workspaces();
            let resources = self.hierarchy_state.descendant_resources_for(&parent_id);
            let mut descendants = Vec::with_capacity(resources.len());
            for resource in resources {
                let cleanup = self
                    .workspace
                    .cleanup_target_for_resource(&resource, CleanupTerminalOutcome::Succeeded)
                    .await
                    .map_err(|error| SchedulerError::Workspace {
                        detail: error.to_string(),
                    })?
                    .ok_or_else(|| SchedulerError::Workspace {
                        detail: format!(
                            "captured parent {} is missing retained workspace generation {}",
                            parent_id, resource.checkout_generation
                        ),
                    })?;
                descendants.push(ParentSubtreeCleanupTarget {
                    resource: Some(resource.clone()),
                    cleanup,
                    prepared_at: None,
                    cleaned_at: None,
                });
            }
            descendants.sort_by(|left, right| {
                let left_resource = left.resource.as_ref().expect("descendant resource");
                let right_resource = right.resource.as_ref().expect("descendant resource");
                hierarchy_depth(&self.hierarchy_state, &parent_id, &right_resource.issue_id)
                    .cmp(&hierarchy_depth(
                        &self.hierarchy_state,
                        &parent_id,
                        &left_resource.issue_id,
                    ))
                    .then_with(|| left_resource.cmp(right_resource))
            });
            let intent = ParentSubtreeCleanupIntent {
                hierarchy_generation: controller.hierarchy_generation,
                capture_acknowledged_at: observed_at,
                requested_at: observed_at,
                outcome,
                status: if retained {
                    ParentSubtreeCleanupStatus::Retained
                } else {
                    ParentSubtreeCleanupStatus::Pending
                },
                parent_root: ParentSubtreeCleanupTarget {
                    resource: None,
                    cleanup: CleanupTarget {
                        issue_id: parent_id.to_string(),
                        identifier: execution.issue().identifier.to_string(),
                        workspace: parent_workspace,
                        generation: format!("parent:{}", controller.hierarchy_generation),
                        outcome,
                    },
                    prepared_at: None,
                    cleaned_at: None,
                },
                descendants,
                retry_count: 0,
                last_error: None,
            };
            self.hierarchy_state
                .parent_integrations
                .get_mut(&parent_id)
                .expect("parent controller was cloned")
                .subtree_cleanup = Some(intent);
            changed = true;
        }
        if changed {
            self.persist_orchestrator_state().await?;
        }
        self.reconcile_parent_subtree_cleanup(observed_at).await
    }

    async fn reconcile_parent_subtree_cleanup(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let parent_ids = self
            .hierarchy_state
            .parent_integrations
            .iter()
            .filter(|(_, controller)| {
                controller.subtree_cleanup.as_ref().is_some_and(|cleanup| {
                    matches!(
                        cleanup.status,
                        ParentSubtreeCleanupStatus::Pending | ParentSubtreeCleanupStatus::Removing
                    )
                })
            })
            .map(|(parent_id, _)| parent_id.clone())
            .collect::<Vec<_>>();
        for parent_id in parent_ids {
            let parent_root = self
                .hierarchy_state
                .parent_integrations
                .get(&parent_id)
                .and_then(|controller| controller.subtree_cleanup.as_ref())
                .map(|cleanup| cleanup.parent_root.clone())
                .expect("cleanup intent exists");
            if parent_root.prepared_at.is_none() {
                match self
                    .workspace
                    .prepare_cleanup_generation(&parent_root.cleanup)
                    .await
                {
                    Ok(()) => {
                        self.hierarchy_state
                            .parent_integrations
                            .get_mut(&parent_id)
                            .and_then(|controller| controller.subtree_cleanup.as_mut())
                            .expect("cleanup intent exists")
                            .parent_root
                            .prepared_at = Some(observed_at);
                        self.persist_orchestrator_state().await?;
                    }
                    Err(error) => {
                        let cleanup = self
                            .hierarchy_state
                            .parent_integrations
                            .get_mut(&parent_id)
                            .and_then(|controller| controller.subtree_cleanup.as_mut())
                            .expect("cleanup intent exists");
                        cleanup.status = ParentSubtreeCleanupStatus::Pending;
                        cleanup.retry_count = cleanup.retry_count.saturating_add(1);
                        cleanup.last_error = Some(redact_runtime_diagnostic(&error.to_string()));
                        self.persist_orchestrator_state().await?;
                        continue;
                    }
                }
            }
            let descendants = self
                .hierarchy_state
                .parent_integrations
                .get(&parent_id)
                .and_then(|controller| controller.subtree_cleanup.as_ref())
                .map(|cleanup| cleanup.descendants.clone())
                .unwrap_or_default();
            for target in &descendants {
                let resource = target.resource.as_ref().expect("descendant resource");
                if self.hierarchy_state.release_parent_resource_leases(
                    &parent_id,
                    resource,
                    observed_at.as_u64(),
                ) {
                    self.persist_orchestrator_state().await?;
                }
            }
            if let Some(cleanup) = self
                .hierarchy_state
                .parent_integrations
                .get_mut(&parent_id)
                .and_then(|controller| controller.subtree_cleanup.as_mut())
            {
                cleanup.status = ParentSubtreeCleanupStatus::Removing;
                cleanup.last_error = None;
            }
            self.persist_orchestrator_state().await?;

            let mut failed = false;
            for (index, target) in descendants.iter().enumerate() {
                if target.cleaned_at.is_some() {
                    continue;
                }
                let resource = target.resource.as_ref().expect("descendant resource");
                if self
                    .hierarchy_state
                    .active_for_at(resource, observed_at.as_u64())
                {
                    continue;
                }
                match self.workspace.cleanup_generation(&target.cleanup).await {
                    Ok(()) => {
                        if let Some(execution) = self.executions.get_mut(&resource.issue_id)
                            && execution.workspace().is_some_and(|workspace| {
                                workspace.path == target.cleanup.workspace.path
                            })
                        {
                            execution.clear_workspace();
                        }
                        self.hierarchy_state
                            .parent_integrations
                            .get_mut(&parent_id)
                            .and_then(|controller| controller.subtree_cleanup.as_mut())
                            .expect("cleanup intent exists")
                            .descendants[index]
                            .cleaned_at = Some(observed_at);
                        self.persist_orchestrator_state().await?;
                    }
                    Err(error) => {
                        let cleanup = self
                            .hierarchy_state
                            .parent_integrations
                            .get_mut(&parent_id)
                            .and_then(|controller| controller.subtree_cleanup.as_mut())
                            .expect("cleanup intent exists");
                        cleanup.status = ParentSubtreeCleanupStatus::Pending;
                        cleanup.retry_count = cleanup.retry_count.saturating_add(1);
                        cleanup.last_error = Some(redact_runtime_diagnostic(&error.to_string()));
                        self.persist_orchestrator_state().await?;
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                continue;
            }
            let cleanup = self
                .hierarchy_state
                .parent_integrations
                .get(&parent_id)
                .and_then(|controller| controller.subtree_cleanup.as_ref())
                .expect("cleanup intent exists")
                .clone();
            if cleanup
                .descendants
                .iter()
                .any(|target| target.cleaned_at.is_none())
            {
                if let Some(cleanup) = self
                    .hierarchy_state
                    .parent_integrations
                    .get_mut(&parent_id)
                    .and_then(|controller| controller.subtree_cleanup.as_mut())
                {
                    cleanup.status = ParentSubtreeCleanupStatus::Pending;
                }
                self.persist_orchestrator_state().await?;
                continue;
            }
            if cleanup.parent_root.cleaned_at.is_none() {
                match self
                    .workspace
                    .cleanup_generation(&cleanup.parent_root.cleanup)
                    .await
                {
                    Ok(()) => {
                        if let Some(execution) = self.executions.get_mut(&parent_id)
                            && execution.workspace().is_some_and(|workspace| {
                                workspace.path == cleanup.parent_root.cleanup.workspace.path
                            })
                        {
                            execution.clear_workspace();
                        }
                        let cleanup = self
                            .hierarchy_state
                            .parent_integrations
                            .get_mut(&parent_id)
                            .and_then(|controller| controller.subtree_cleanup.as_mut())
                            .expect("cleanup intent exists");
                        cleanup.parent_root.cleaned_at = Some(observed_at);
                        cleanup.status = ParentSubtreeCleanupStatus::Completed;
                        cleanup.last_error = None;
                        self.persist_orchestrator_state().await?;
                    }
                    Err(error) => {
                        let cleanup = self
                            .hierarchy_state
                            .parent_integrations
                            .get_mut(&parent_id)
                            .and_then(|controller| controller.subtree_cleanup.as_mut())
                            .expect("cleanup intent exists");
                        cleanup.status = ParentSubtreeCleanupStatus::Pending;
                        cleanup.retry_count = cleanup.retry_count.saturating_add(1);
                        cleanup.last_error = Some(redact_runtime_diagnostic(&error.to_string()));
                        self.persist_orchestrator_state().await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn interrupt_operator_cancel(
        &mut self,
        target: &str,
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        let Some((issue_id, mut execution, run, harness_kind)) =
            self.operator_cancel_candidate(target)
        else {
            return Ok(false);
        };

        let (command, queued) = execution.request_interrupt(
            harness_kind,
            None,
            HarnessInterruptReason::OperatorCancel,
            HarnessInterruptExpectedNextState::Paused,
            observed_at,
        )?;

        self.persist_interrupt_intent(&execution).await?;

        if queued {
            execution.observe_runtime_event(
                observed_at,
                Some(format!(
                    "operator-cancel-interrupt-{}",
                    observed_at.as_u64()
                )),
                Some("scheduler.interrupt_requested".to_string()),
                Some("Operator cancel requested: operator_cancel".to_string()),
                Some(serde_json::json!({
                    "reason": HarnessInterruptReason::OperatorCancel.as_str(),
                    "worker_id": run.worker_id.as_str(),
                    "target": target,
                })),
            )?;
            let result = self.worker.interrupt_worker(command).await;
            Self::apply_interrupt_result(
                &mut execution,
                HarnessInterruptReason::OperatorCancel,
                observed_at,
                result,
            )?;
        }

        self.insert_execution(issue_id, execution);
        Ok(true)
    }

    /// Clear a durable `HierarchyChanged` block after an operator or parent
    /// controller has deliberately accepted the current child-edge snapshot.
    /// The transition keeps retained evidence and leases intact; the next
    /// eligibility pass must reacquire the current generation's leases.
    pub async fn replan_parent(
        &mut self,
        parent_id: &IssueId,
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        self.load_recovery_state().await?;
        let Some(snapshot) = self.hierarchy_state.hierarchy.get(parent_id) else {
            return Ok(false);
        };
        if snapshot.blocked_reason != Some(HierarchyBlockedReason::HierarchyChanged) {
            return Ok(false);
        }
        if self.executions.get(parent_id).is_some_and(|execution| {
            matches!(
                execution.status(),
                SchedulerStatus::Claimed | SchedulerStatus::Running
            )
        }) {
            return Err(SchedulerError::Workspace {
                detail: format!(
                    "cannot replan hierarchy parent {parent_id} while its fenced execution is still running"
                ),
            });
        }

        let previous_state = self.hierarchy_state.clone();
        let generation = if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(parent_id) {
            snapshot.replan();
            Some(snapshot.generation)
        } else {
            None
        };
        if let Some(generation) = generation {
            self.hierarchy_state
                .rebind_ancestor_leases(parent_id, generation);
        }
        let controller_replaced = generation.is_some_and(|generation| {
            self.hierarchy_state
                .parent_integrations
                .get(parent_id)
                .is_some_and(|controller| controller.hierarchy_generation != generation)
        });
        if controller_replaced {
            self.hierarchy_state.parent_integrations.insert(
                parent_id.clone(),
                ParentIntegrationController::new(
                    parent_id.clone(),
                    generation.expect("controller replacement requires a generation"),
                )?,
            );
        }
        self.parent_eligibility_checked_at.remove(parent_id);
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_state;
            return Err(error);
        }
        if controller_replaced
            && let Some(issue) = self
                .executions
                .get(parent_id)
                .map(|execution| execution.issue().clone())
        {
            self.reopen_completed_parent_for_new_controller(&issue, observed_at)?;
        }
        Ok(true)
    }

    /// Replan a hierarchy parent addressed by the tracker identifier exposed
    /// by the gateway action envelope. The scheduler resolves the identifier
    /// through the tracker before mutating its durable hierarchy state.
    pub async fn replan_parent_target(
        &mut self,
        target: &str,
        accepted_generation: u64,
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        self.load_recovery_state().await?;
        let lookup_target = self
            .executions
            .values()
            .find(|execution| {
                execution.conversation().is_some_and(|conversation| {
                    let conversation_id = conversation.conversation_id.as_str();
                    conversation_id.eq_ignore_ascii_case(target)
                        || conversation_id_suffix(conversation_id).eq_ignore_ascii_case(target)
                })
            })
            .map(|execution| execution.issue().identifier.as_str().to_owned())
            .unwrap_or_else(|| target.to_owned());
        let issues = match self
            .tracker
            .issues_by_identifiers(std::slice::from_ref(&lookup_target))
            .await
        {
            Ok(issues) => issues,
            Err(error) => {
                if T::error_category(&error) == Some(TrackerErrorCategory::NotFound) {
                    warn!(target = %lookup_target, "replan target disappeared before execution");
                    return Ok(false);
                }
                return Err(SchedulerError::Tracker {
                    detail: error.to_string(),
                });
            }
        };
        let Some(issue) = issues.into_iter().find(|issue| {
            issue.identifier.eq_ignore_ascii_case(&lookup_target) || issue.id == lookup_target
        }) else {
            return Ok(false);
        };
        let issue_id = IssueId::new(issue.id.clone())?;
        let Some(expected_generation) = self
            .hierarchy_state
            .hierarchy
            .get(&issue_id)
            .filter(|snapshot| {
                snapshot.blocked_reason == Some(HierarchyBlockedReason::HierarchyChanged)
            })
            .map(|snapshot| snapshot.generation)
        else {
            return Ok(false);
        };
        if expected_generation != accepted_generation {
            warn!(
                parent = %lookup_target,
                accepted_generation,
                current_generation = expected_generation,
                "rejecting replan for a stale accepted hierarchy generation"
            );
            return Ok(false);
        }

        // Refresh the child edges before clearing the durable block. If the
        // accepted snapshot changed while the action was in flight, preserve
        // the new HierarchyChanged fence instead of silently accepting scope
        // the operator did not explicitly replan.
        let tracker_snapshot = self
            .load_tracker_snapshot(observed_at)
            .await?
            .ok_or_else(|| SchedulerError::Tracker {
                detail: "replan reachability refresh deferred by tracker rate limit".to_owned(),
            })?;
        let reachable_child_edges =
            self.required_child_edges_in_tracker_snapshot(&tracker_snapshot);
        let previous_state = self.hierarchy_state.clone();
        self.reconcile_hierarchy_issue(&issue, Some(&reachable_child_edges))?;
        let current_generation = self
            .hierarchy_state
            .hierarchy
            .get(&issue_id)
            .map(|snapshot| snapshot.generation);
        if self.hierarchy_state != previous_state
            && let Err(error) = self.persist_orchestrator_state().await
        {
            self.hierarchy_state = previous_state;
            return Err(error);
        }
        if current_generation != Some(expected_generation) {
            warn!(
                parent = %lookup_target,
                expected_generation,
                current_generation = ?current_generation,
                "rejecting replan for a hierarchy generation that changed before execution"
            );
            return Ok(false);
        }
        self.replan_parent(&issue_id, observed_at).await
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
        if let Some(records) = &self.pending_retry_recovery {
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
            .iter()
            .map(|issue| (issue.id.clone(), issue.state.clone()))
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
            terminal,
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
                    SchedulerStatus::Claimed
                        | SchedulerStatus::Running
                        | SchedulerStatus::RetryQueued
                )
            })
            .map(|(id, _)| id.as_str().to_string())
            .collect::<Vec<_>>();
        if issue_ids.is_empty() {
            self.last_running_state_refresh_at = Some(observed_at);
            return Ok(());
        }

        let has_running_parent = issue_ids.iter().any(|issue_id| {
            IssueId::new(issue_id.clone())
                .ok()
                .is_some_and(|issue_id| self.parent_issue_ids.contains(&issue_id))
        });
        let active = if has_running_parent {
            match self.tracker.candidate_issues().await {
                Ok(active) => active,
                Err(error) => {
                    if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                        return Ok(());
                    }
                    return Err(SchedulerError::Tracker {
                        detail: error.to_string(),
                    });
                }
            }
        } else {
            Vec::new()
        };
        let active_ids = active
            .iter()
            .map(|issue| issue.id.as_str())
            .collect::<HashSet<_>>();
        let state_issue_ids = issue_ids
            .into_iter()
            .filter(|issue_id| !active_ids.contains(issue_id.as_str()))
            .collect::<Vec<_>>();
        let snapshots = if state_issue_ids.is_empty() {
            Vec::new()
        } else {
            match self.tracker.issue_states_by_ids(&state_issue_ids).await {
                Ok(snapshots) => snapshots,
                Err(error) => {
                    if self.set_linear_cooldown_from_tracker_error(&error, observed_at) {
                        return Ok(());
                    }
                    return Err(SchedulerError::Tracker {
                        detail: error.to_string(),
                    });
                }
            }
        };
        self.last_running_state_refresh_at = Some(observed_at);
        let tracker_snapshot = TrackerSnapshot {
            active_index: active
                .iter()
                .enumerate()
                .map(|(index, issue)| (issue.id.clone(), index))
                .collect(),
            terminal_state_by_id: HashMap::new(),
            state_by_id: snapshots
                .into_iter()
                .map(|snapshot| (snapshot.id.clone(), snapshot))
                .collect(),
            active,
            terminal: Vec::new(),
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
                .iter()
                .map(|issue| (issue.id.clone(), issue.state.clone()))
                .collect(),
            state_by_id: HashMap::new(),
            terminal,
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

    fn reconcile_hierarchy_snapshots(
        &mut self,
        tracker_snapshot: &TrackerSnapshot,
    ) -> Result<bool, SchedulerError> {
        let reachable_child_edges = self.required_child_edges_in_tracker_snapshot(tracker_snapshot);
        let mut durable_state_changed = false;
        for tracker_issue in &tracker_snapshot.active {
            durable_state_changed |=
                self.reconcile_hierarchy_issue(tracker_issue, Some(&reachable_child_edges))?;
        }
        self.hierarchy_state_dirty |= durable_state_changed;
        Ok(durable_state_changed)
    }

    fn reconcile_terminal_hierarchy_snapshots(
        &mut self,
        tracker_snapshot: &TrackerSnapshot,
    ) -> Result<bool, SchedulerError> {
        for issue in &tracker_snapshot.terminal {
            let Ok(issue_id) = IssueId::new(issue.id.clone()) else {
                continue;
            };
            if self
                .hierarchy_state
                .hierarchy
                .get(&issue_id)
                .is_none_or(|snapshot| !snapshot.has_dispatched_execution_fence())
            {
                self.terminal_undispatched_parent_ids.insert(issue_id);
            } else {
                self.terminal_undispatched_parent_ids.remove(&issue_id);
            }
        }
        let issues = tracker_snapshot
            .terminal
            .iter()
            .filter(|issue| {
                IssueId::new(issue.id.clone())
                    .is_ok_and(|issue_id| self.hierarchy_state.hierarchy.contains_key(&issue_id))
            })
            .cloned()
            .collect::<Vec<_>>();
        if issues.is_empty() {
            return Ok(false);
        }
        let reachable_child_edges = self.required_child_edges_in_tracker_snapshot(tracker_snapshot);
        let mut changed = false;
        for issue in issues {
            changed |= self.reconcile_hierarchy_issue(&issue, Some(&reachable_child_edges))?;
        }
        Ok(changed)
    }

    fn required_child_edges_in_tracker_snapshot(
        &self,
        tracker_snapshot: &TrackerSnapshot,
    ) -> BTreeSet<(IssueId, IssueId)> {
        let mut edges = BTreeSet::new();
        for issue in tracker_snapshot
            .active
            .iter()
            .chain(tracker_snapshot.terminal.iter())
        {
            let Ok(issue_id) = IssueId::new(issue.id.clone()) else {
                continue;
            };
            let snapshot =
                HierarchySnapshot::new_with_canceled_states(issue, &self.config.terminal_states);
            edges.extend(
                snapshot
                    .required_child_edges
                    .into_iter()
                    .filter(|edge| edge.required)
                    .map(|edge| (issue_id.clone(), edge.child_id)),
            );
            if !is_canceled_tracker_issue(issue, &self.config.terminal_states)
                && let Some(parent_id) = issue
                    .parent_id
                    .as_deref()
                    .and_then(|parent_id| IssueId::new(parent_id.to_owned()).ok())
            {
                edges.insert((parent_id, issue_id));
            }
        }
        edges
    }

    async fn fence_hierarchy_changed_runs(
        &mut self,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let mut cleared_in_flight_fence = false;
        let issue_ids = self
            .hierarchy_state
            .hierarchy
            .iter()
            .filter(|(_, snapshot)| {
                snapshot.blocked_reason == Some(HierarchyBlockedReason::HierarchyChanged)
            })
            .filter_map(|(issue_id, _)| {
                self.executions.get(issue_id).and_then(|execution| {
                    matches!(
                        execution.status(),
                        SchedulerStatus::Claimed | SchedulerStatus::Running
                    )
                    .then(|| issue_id.clone())
                })
            })
            .collect::<Vec<_>>();
        for issue_id in issue_ids {
            let Some(issue) = self
                .executions
                .get(&issue_id)
                .map(|execution| execution.issue().clone())
            else {
                continue;
            };
            self.release_issue(
                issue_id.clone(),
                issue,
                observed_at,
                ReleaseReason::TrackerInactive,
                false,
                Some(WorkerAbortReason::TrackerInactive),
            )
            .await?;
            if self
                .executions
                .get(&issue_id)
                .is_some_and(|execution| execution.status() == SchedulerStatus::Released)
                && let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(&issue_id)
            {
                cleared_in_flight_fence |= snapshot.clear_in_flight_dispatch();
            }
        }
        if cleared_in_flight_fence {
            self.hierarchy_state_dirty = true;
            self.persist_orchestrator_state().await?;
        }
        Ok(())
    }

    fn reconcile_hierarchy_issue(
        &mut self,
        tracker_issue: &TrackerIssue,
        reachable_child_edges: Option<&BTreeSet<(IssueId, IssueId)>>,
    ) -> Result<bool, SchedulerError> {
        let normalized = normalize_tracker_issue(tracker_issue, &self.config)?;
        let terminal_failure_resolved =
            self.executions
                .get(&normalized.id)
                .is_some_and(|execution| {
                    execution.retry().is_none()
                        && execution.last_worker_outcome().is_some_and(|outcome| {
                            matches!(outcome.outcome, WorkerOutcomeKind::Succeeded)
                        })
                });
        if self.terminal_child_failure_ids.contains(&normalized.id) && terminal_failure_resolved {
            self.terminal_child_failure_ids.remove(&normalized.id);
        }
        let existing_snapshot = self.hierarchy_state.hierarchy.get(&normalized.id).cloned();
        if reachable_child_edges.is_none()
            && existing_snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.required_child_edges
                    != HierarchySnapshot::new_with_canceled_states(
                        tracker_issue,
                        &self.config.terminal_states,
                    )
                    .required_child_edges
            })
        {
            // Keep the old scope and its leases until the full observation can
            // distinguish removed children from children moved to another parent.
            self.last_full_detail_refresh_at = None;
            return Ok(false);
        }
        let should_retain_parent_identity = !tracker_issue.sub_issues.is_empty()
            || existing_snapshot.as_ref().is_some_and(|snapshot| {
                !snapshot.required_child_edges.is_empty()
                    || snapshot.frozen
                    || snapshot.blocked_reason.is_some()
            });
        if !should_retain_parent_identity {
            self.parent_issue_ids.remove(&normalized.id);
            self.terminal_undispatched_parent_ids.remove(&normalized.id);
            self.terminal_child_failure_ids.remove(&normalized.id);
            self.hierarchy_state
                .run_hierarchy_generations
                .remove(&normalized.id);
            self.hierarchy_state
                .parent_integrations
                .remove(&normalized.id);
            if let Some(snapshot) = self.hierarchy_state.hierarchy.remove(&normalized.id) {
                let removed_child_ids = snapshot
                    .required_child_edges
                    .iter()
                    .filter(|edge| edge.required)
                    .map(|edge| edge.child_id.clone())
                    .collect::<Vec<_>>();
                self.hierarchy_state.release_removed_subtree_leases(
                    &normalized.id,
                    &removed_child_ids,
                    reachable_child_edges,
                    current_epoch_millis(),
                );
                return Ok(true);
            }
            return Ok(false);
        }
        self.parent_issue_ids.insert(normalized.id.clone());
        if !self.hierarchy_state.hierarchy.contains_key(&normalized.id) {
            let snapshot = HierarchySnapshot::new_with_canceled_states(
                tracker_issue,
                &self.config.terminal_states,
            );
            let terminal_undispatched = normalized.state.category == IssueStateCategory::Terminal
                && !snapshot.has_dispatched_execution_fence();
            if terminal_undispatched {
                self.terminal_undispatched_parent_ids
                    .insert(normalized.id.clone());
            }
            self.hierarchy_state
                .hierarchy
                .insert(normalized.id.clone(), snapshot);
            self.hierarchy_state.parent_integrations.insert(
                normalized.id.clone(),
                ParentIntegrationController::new(normalized.id.clone(), 1)?,
            );
            if terminal_undispatched {
                self.hierarchy_state
                    .release_subtree_evidence_for_undispatched_parent_with_reachability(
                        &normalized.id,
                        reachable_child_edges,
                        current_epoch_millis(),
                    );
            }
            return Ok(true);
        }
        let terminal_undispatched = normalized.state.category == IssueStateCategory::Terminal
            && self
                .hierarchy_state
                .hierarchy
                .get(&normalized.id)
                .is_some_and(|snapshot| !snapshot.has_dispatched_execution_fence());
        if terminal_undispatched {
            self.terminal_undispatched_parent_ids
                .insert(normalized.id.clone());
        } else {
            self.terminal_undispatched_parent_ids.remove(&normalized.id);
        }
        let released_terminal_parent_evidence = terminal_undispatched
            && self
                .hierarchy_state
                .release_subtree_evidence_for_undispatched_parent_with_reachability(
                    &normalized.id,
                    reachable_child_edges,
                    current_epoch_millis(),
                );
        let parent_waiting_for_tracker_confirmation =
            self.parent_waiting_for_tracker_confirmation(&normalized.id);
        let reactivated_parent = !parent_waiting_for_tracker_confirmation
            && self
                .hierarchy_state
                .hierarchy
                .get(&normalized.id)
                .is_some_and(|snapshot| {
                    snapshot.dispatch_claimed()
                        && !snapshot.has_in_flight_dispatch()
                        && normalized.state.category == IssueStateCategory::Active
                        && !self
                            .executions
                            .get(&normalized.id)
                            .is_some_and(|execution| {
                                matches!(
                                    execution.status(),
                                    SchedulerStatus::Claimed | SchedulerStatus::Running
                                )
                            })
                        && !self.hierarchy_state.leases.iter().any(|lease| {
                            lease.active()
                                && normalized.parent_id.is_none()
                                && (lease.owner == super::LeaseOwner::ancestor(&normalized.id)
                                    || (lease.kind == super::LeaseKind::Review
                                        && lease.owner.id.starts_with(&format!(
                                            "review:{normalized_id}:",
                                            normalized_id = normalized.id
                                        ))))
                        })
                });
        let reconciliation =
            if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(&normalized.id) {
                if reactivated_parent {
                    // Reconcile the detailed child edges before making a
                    // previously terminal parent dispatchable again. A
                    // changed scope must advance the generation that the new
                    // run will use.
                    snapshot.replan();
                }
                let previous_child_ids = snapshot
                    .required_child_edges
                    .iter()
                    .filter(|edge| edge.required)
                    .map(|edge| edge.child_id.clone())
                    .collect::<BTreeSet<_>>();
                let reconciliation = snapshot.reconcile_with_canceled_states(
                    &tracker_issue.sub_issues,
                    &self.config.terminal_states,
                );
                if !matches!(&reconciliation, super::HierarchyReconciliation::Unchanged) {
                    let current_child_ids = snapshot
                        .required_child_edges
                        .iter()
                        .filter(|edge| edge.required)
                        .map(|edge| edge.child_id.clone())
                        .collect::<BTreeSet<_>>();
                    let removed_child_ids = previous_child_ids
                        .difference(&current_child_ids)
                        .cloned()
                        .collect::<Vec<_>>();
                    Some((snapshot.generation, current_child_ids, removed_child_ids))
                } else {
                    None
                }
            } else {
                None
            };
        if let Some((generation, current_child_ids, removed_child_ids)) = reconciliation {
            self.parent_eligibility_checked_at.remove(&normalized.id);
            self.hierarchy_state
                .terminal_orchestrator_issues
                .remove(&normalized.id);
            let retained_child_ids = current_child_ids
                .iter()
                .filter(|child_id| {
                    !self.executions.get(*child_id).is_some_and(|execution| {
                        matches!(
                            execution.status(),
                            SchedulerStatus::Claimed | SchedulerStatus::Running
                        )
                    })
                })
                .cloned()
                .collect::<BTreeSet<_>>();
            self.hierarchy_state
                .rebind_leaf_leases(&retained_child_ids, generation);
            self.hierarchy_state.release_removed_subtree_leases(
                &normalized.id,
                &removed_child_ids,
                reachable_child_edges,
                current_epoch_millis(),
            );
            for child_id in &retained_child_ids {
                self.hierarchy_state
                    .run_hierarchy_generations
                    .insert(child_id.clone(), generation);
            }
            if self
                .hierarchy_state
                .hierarchy
                .get(&normalized.id)
                .is_some_and(|snapshot| snapshot.blocked_reason.is_none())
            {
                self.hierarchy_state.parent_integrations.insert(
                    normalized.id.clone(),
                    ParentIntegrationController::new(normalized.id.clone(), generation)?,
                );
                self.reopen_completed_parent_for_new_controller(
                    &normalized,
                    TimestampMs::new(current_epoch_millis()),
                )?;
            }
            return Ok(true);
        }
        if reactivated_parent {
            let generation = self
                .hierarchy_state
                .hierarchy
                .get(&normalized.id)
                .map(|snapshot| snapshot.generation);
            if let Some(generation) = generation
                && self
                    .hierarchy_state
                    .hierarchy
                    .get(&normalized.id)
                    .is_some_and(|snapshot| snapshot.blocked_reason.is_none())
            {
                self.parent_eligibility_checked_at.remove(&normalized.id);
                self.hierarchy_state.parent_integrations.insert(
                    normalized.id.clone(),
                    ParentIntegrationController::new(normalized.id.clone(), generation)?,
                );
                self.reopen_completed_parent_for_new_controller(
                    &normalized,
                    TimestampMs::new(current_epoch_millis()),
                )?;
            }
        }
        Ok(released_terminal_parent_evidence || reactivated_parent)
    }

    fn reopen_completed_parent_for_new_controller(
        &mut self,
        issue: &NormalizedIssue,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        if issue.state.category != IssueStateCategory::Active
            || !self.executions.get(&issue.id).is_some_and(|execution| {
                matches!(
                    execution.state(),
                    crate::opensymphony_orchestrator::SchedulerState::Released {
                        reason: ReleaseReason::Completed,
                        ..
                    }
                )
            })
        {
            return Ok(());
        }
        let mut execution = self
            .remove_execution(&issue.id)
            .expect("completed parent execution was checked above");
        execution = execution.reopen(observed_at)?;
        execution.refresh_issue(issue.clone())?;
        self.insert_execution(issue.id.clone(), execution);
        Ok(())
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

    fn tracker_operation_error(
        &mut self,
        error: T::Error,
        observed_at: TimestampMs,
    ) -> SchedulerError {
        self.set_linear_cooldown_from_tracker_error(&error, observed_at);
        SchedulerError::Tracker {
            detail: error.to_string(),
        }
    }

    async fn bootstrap_recovery(
        &mut self,
        tracker_snapshot: &TrackerSnapshot,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let recovered_in_flight_parent_ids = self
            .pending_recovery
            .as_ref()
            .into_iter()
            .flatten()
            .filter(|record| record.had_in_flight_run && record.recovered_run.is_some())
            .map(|record| record.issue.id.clone())
            .collect::<HashSet<_>>();
        for parent_id in &recovered_in_flight_parent_ids {
            if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(parent_id) {
                self.hierarchy_state_dirty |= snapshot.restore_recovered_dispatch_fence();
            }
        }
        let hierarchy_changed = self.reconcile_hierarchy_snapshots(tracker_snapshot)?;
        let terminal_hierarchy_changed =
            self.reconcile_terminal_hierarchy_snapshots(tracker_snapshot)?;
        if hierarchy_changed || terminal_hierarchy_changed || self.hierarchy_state_dirty {
            self.persist_orchestrator_state().await?;
        }
        if self.recovered {
            return Ok(());
        }

        let Some(records) = self.pending_recovery.take() else {
            self.recovered = true;
            return Ok(());
        };

        let mut pending_retry_by_issue = self
            .pending_retry_recovery
            .take()
            .unwrap_or_default()
            .into_iter()
            .map(|pending| (pending.issue.id.clone(), pending))
            .collect::<HashMap<_, _>>();
        let mut records = records;
        if self
            .migrate_legacy_in_flight_parent_controllers(&records, observed_at)
            .await?
        {
            self.persist_orchestrator_state().await?;
        }
        for record in &records {
            if let Some(controller) = self
                .hierarchy_state
                .parent_integrations
                .get_mut(&record.issue.id)
            {
                if record.recovered_run.is_some() {
                    controller.reconcile_restart(true, observed_at)?;
                } else if record.successful_run || record.completed_run || record.cancelled_run {
                    controller.reconcile_terminal_run_after_restart(observed_at)?;
                } else {
                    controller.reconcile_restart(false, observed_at)?;
                }
                self.hierarchy_state_dirty = true;
            }
        }
        for controller in self.hierarchy_state.parent_integrations.values_mut() {
            let has_unlaunched_attempt = controller.attempts.iter().any(|attempt| {
                attempt.status == ParentAttemptStatus::Running
                    && attempt.conversation_id.is_none()
                    && attempt.commands.is_empty()
                    && attempt.resources.is_empty()
            });
            if has_unlaunched_attempt {
                controller.reconcile_restart(false, observed_at)?;
                self.hierarchy_state_dirty = true;
            }
        }
        if self.hierarchy_state_dirty {
            self.persist_orchestrator_state().await?;
        }
        // Recovery manifests intentionally do not persist a second hierarchy
        // identity. Hydrate the parent edge from the provider's full issue
        // detail so retained intermediate-parent leases preserve the higher
        // ancestor owner, which may be outside the active project scan while
        // the intermediate issue is terminal.
        let recovered_parent_ids = tracker_snapshot
            .active
            .iter()
            .chain(tracker_snapshot.terminal.iter())
            .filter_map(|issue| {
                let issue_id = IssueId::new(issue.id.clone()).ok()?;
                let parent_id = issue
                    .parent_id
                    .as_deref()
                    .and_then(|parent_id| IssueId::new(parent_id.to_owned()).ok())?;
                Some((issue_id, parent_id))
            })
            .collect::<HashMap<_, _>>();
        for record in &mut records {
            if record.issue.parent_id.is_none()
                && let Some(parent_id) = recovered_parent_ids.get(&record.issue.id)
            {
                record.issue.parent_id = Some(parent_id.clone());
            }
        }
        let mut dispatch_transition_changed = false;
        let in_flight_issue_ids = records
            .iter()
            .filter(|record| record.had_in_flight_run)
            .map(|record| record.issue.id.clone())
            .collect::<HashSet<_>>();
        let terminal_recovered_in_flight_parent_ids = records
            .iter()
            .filter(|record| {
                record.had_in_flight_run
                    && record.recovered_run.is_some()
                    && tracker_snapshot.contains_terminal(record.issue.id.as_str())
            })
            .map(|record| record.issue.id.clone())
            .collect::<HashSet<_>>();
        let intended_parent_ids = self
            .hierarchy_state
            .hierarchy
            .iter()
            .filter(|(_, snapshot)| snapshot.dispatch_intended())
            .map(|(parent_id, _)| parent_id.clone())
            .collect::<Vec<_>>();
        for parent_id in intended_parent_ids {
            if in_flight_issue_ids.contains(&parent_id) {
                let nested_child_ids = self
                    .hierarchy_state
                    .hierarchy
                    .get(&parent_id)
                    .map(|snapshot| {
                        snapshot
                            .required_child_edges
                            .iter()
                            .filter(|edge| {
                                edge.required
                                    && self.hierarchy_state.hierarchy.contains_key(&edge.child_id)
                            })
                            .map(|edge| edge.child_id.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(&parent_id) {
                    snapshot.mark_dispatched();
                }
                if !terminal_recovered_in_flight_parent_ids.contains(&parent_id) {
                    self.hierarchy_state
                        .release_leaf_leases_for_parent(&parent_id, observed_at.as_u64());
                    self.hierarchy_state.release_ancestor_leases_for_children(
                        &nested_child_ids,
                        observed_at.as_u64(),
                    );
                }
            } else {
                if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(&parent_id) {
                    snapshot.clear_dispatch_intent();
                }
            }
            dispatch_transition_changed = true;
        }
        if dispatch_transition_changed {
            self.persist_orchestrator_state().await?;
        }
        self.recovered_memory_issue_ids
            .extend(records.iter().map(|record| record.issue.id.clone()));
        for record in &mut records {
            let issue_id = record.issue.id.clone();
            if let Some(pending) = pending_retry_by_issue.remove(&issue_id) {
                let has_durable_workspace_retry = record.pending_retry
                    || record.had_in_flight_run
                    || record.successful_run
                    || record.cancelled_run
                    || record.completed_run;
                if has_durable_workspace_retry {
                    // A run manifest is authoritative when it exists. The
                    // external marker is a duplicate from the pre-start
                    // window and can be removed safely.
                    self.workspace
                        .clear_retry_pending(&issue_id)
                        .await
                        .map_err(|error| SchedulerError::Workspace {
                            detail: error.to_string(),
                        })?;
                } else if tracker_snapshot.active_issue(&issue_id).is_some() {
                    // The workspace has only its issue metadata: start_run
                    // never became durable. Merge the still-authoritative
                    // external RetryEntry into this recovery record so the
                    // workspace is retained and its retry budget survives.
                    record.pending_retry = true;
                    record.normal_retry_count = pending.retry.normal_retry_count.saturating_sub(1);
                    record.retry_scheduled_at = Some(pending.retry.scheduled_at);
                    record.retry_due_at = Some(pending.retry.due_at);
                    record.retry_reason = Some(pending.retry.reason);
                    record.retry_error = pending.retry.error;
                } else {
                    // Do not restore a retry from stale local state when the
                    // current project-filtered tracker snapshot cannot prove
                    // that the issue is still active and in scope.
                    self.workspace
                        .clear_retry_pending(&issue_id)
                        .await
                        .map_err(|error| SchedulerError::Workspace {
                            detail: error.to_string(),
                        })?;
                }
            }
        }

        for pending in pending_retry_by_issue.into_values() {
            if tracker_snapshot.contains_terminal(pending.issue.id.as_str()) {
                self.workspace
                    .clear_retry_pending(&pending.issue.id)
                    .await
                    .map_err(|error| SchedulerError::Workspace {
                        detail: error.to_string(),
                    })?;
                continue;
            }

            let Some(active_issue) = tracker_snapshot.active_issue(&pending.issue.id) else {
                // The project-filtered active snapshot is the required live
                // proof for externally persisted retry state.
                self.workspace
                    .clear_retry_pending(&pending.issue.id)
                    .await
                    .map_err(|error| SchedulerError::Workspace {
                        detail: error.to_string(),
                    })?;
                continue;
            };
            let issue = normalize_tracker_issue(active_issue, &self.config)?;
            let execution = IssueExecution::new(issue.clone(), observed_at);
            if self.retry_count_exceeds_limit(pending.retry.normal_retry_count) {
                self.insert_execution(issue.id.clone(), execution);
                self.persist_retry_exhaustion(
                    &issue,
                    pending.retry.normal_retry_count.saturating_sub(1),
                )
                .await?;
                self.mark_recovered_retry_exhausted(
                    &issue.id,
                    pending.retry.normal_retry_count.saturating_sub(1),
                    observed_at,
                )?;
            } else {
                self.insert_execution(issue.id.clone(), execution.restore_retry(pending.retry)?);
            }
        }

        for record in self.pending_retry_exhaustion.take().unwrap_or_default() {
            if let Some(active_issue) = tracker_snapshot.active_issue(&record.issue.id) {
                let normalized = normalize_tracker_issue(active_issue, &self.config)?;
                let execution = IssueExecution::new(normalized.clone(), observed_at).release(
                    observed_at,
                    ReleaseReason::RetryExhausted,
                    None,
                )?;
                let mut execution = execution;
                execution.set_retry_count_override(record.normal_retry_count);
                self.insert_execution(normalized.id.clone(), execution);
                continue;
            }
            if tracker_snapshot.contains_terminal(record.issue.id.as_str()) {
                self.workspace
                    .clear_retry_exhaustion(record.issue.identifier.as_str())
                    .await
                    .map_err(|error| SchedulerError::Workspace {
                        detail: error.to_string(),
                    })?;
                continue;
            }
            let mut issue = record.issue.clone();
            if let Some(snapshot) = tracker_snapshot.state_by_id.get(record.issue.id.as_str()) {
                issue.state = issue_state_from_name(&snapshot.state.name, &self.config);
            }
            let execution = IssueExecution::new(issue.clone(), observed_at).release(
                observed_at,
                ReleaseReason::RetryExhausted,
                None,
            )?;
            let mut execution = execution;
            execution.set_retry_count_override(record.normal_retry_count);
            self.insert_execution(issue.id.clone(), execution);
        }

        let mut retry_records = Vec::new();
        for (record_index, record) in records.iter().cloned().enumerate() {
            let mut recovered_run = record.recovered_run.clone();
            let mut recovered_workspace = Some(record.workspace.clone());
            if let Some(recovered_run) = recovered_run.as_ref() {
                self.reserve_recovered_worker_id(&recovered_run.worker_id);
            }
            let issue_id = record.issue.id.clone();
            let recovered_harness_kind = record.harness_kind.clone();
            if let Some(active_issue) = tracker_snapshot.active_issue(&issue_id) {
                let normalized = normalize_tracker_issue(active_issue, &self.config)?;
                let recovered_binding = recovered_run
                    .as_ref()
                    .and_then(|run| {
                        run.repository_binding
                            .clone()
                            .map(RepositoryBindingOutcome::Resolved)
                    })
                    .or_else(|| {
                        record
                            .issue
                            .repository_binding
                            .as_ref()
                            .and_then(RepositoryBindingOutcome::resolved_binding)
                            .cloned()
                            .map(RepositoryBindingOutcome::Resolved)
                    });
                let binding_changed = RepositoryBindingOutcome::binding_changed_opt(
                    normalized.repository_binding.as_ref(),
                    recovered_binding.as_ref(),
                );
                // A legacy configuration is a locator, not proof of the
                // repository currently on disk. Do not attach an in-flight
                // worker to the current binding when neither the run nor the
                // recovered issue manifest proves its old identity. Park it
                // for the normal retry path instead.
                if recovered_run.is_some()
                    && recovered_binding.is_none()
                    && normalized.repository_binding.is_some()
                {
                    let recovered_run_ref = recovered_run
                        .as_ref()
                        .expect("unproven recovery should retain its recovered run");
                    if !self
                        .stop_recovered_run_before_workspace_removal(
                            &record,
                            recovered_run_ref,
                            recovered_harness_kind.as_deref(),
                            observed_at,
                        )
                        .await?
                    {
                        continue;
                    }
                    self.workspace
                        .remove_workspace(&record.workspace)
                        .await
                        .map_err(|error| SchedulerError::Workspace {
                            detail: error.to_string(),
                        })?;
                    recovered_workspace = None;
                    recovered_run = None;
                }
                // A metadata-only recovery has no run to supersede. If its
                // recovered issue manifest proves a different repository than
                // the live tracker binding, discard the old workspace before
                // restoring the retry so the replacement is materialized by
                // the normal dispatch path.
                if recovered_run.is_none() && recovered_workspace.is_some() && binding_changed {
                    self.release_stale_binding_leases(
                        &record.issue,
                        recovered_workspace
                            .as_ref()
                            .expect("metadata-only recovery workspace should be present"),
                        observed_at,
                    )
                    .await?;
                    self.workspace
                        .remove_workspace(&record.workspace)
                        .await
                        .map_err(|error| SchedulerError::Workspace {
                            detail: error.to_string(),
                        })?;
                    recovered_workspace = None;
                }
                self.upsert_active_execution(normalized.clone(), observed_at, recovered_workspace)?;
                if record.had_in_flight_run {
                    if recovered_run.is_some() {
                        self.restore_recovered_run(
                            &issue_id,
                            recovered_run,
                            recovered_harness_kind,
                            record.interrupt_reason,
                            observed_at,
                        )
                        .await?;
                        if binding_changed {
                            self.supersede_binding(
                                issue_id.clone(),
                                normalized.clone(),
                                observed_at,
                                false,
                            )
                            .await?;
                        }
                        if record.interrupt_reason.is_some() {
                            self.retry_recovered_interrupt(&issue_id, observed_at)
                                .await?;
                        }
                    } else if self.retry_limit_reached(record.normal_retry_count) {
                        self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                            .await?;
                        self.mark_recovered_retry_exhausted(
                            &issue_id,
                            record.normal_retry_count,
                            observed_at,
                        )?;
                    } else {
                        let normal_retry_count = record.normal_retry_count.saturating_add(1);
                        let retry = RetryEntry {
                            issue_id: normalized.id.clone(),
                            identifier: normalized.identifier.clone(),
                            attempt: RetryAttempt::new(normal_retry_count)?,
                            normal_retry_count,
                            scheduled_at: observed_at,
                            due_at: observed_at,
                            reason: RetryReason::Reconciliation,
                            error: None,
                        };
                        let execution = self
                            .remove_execution(&issue_id)
                            .expect("active recovery execution should be present");
                        self.insert_execution(issue_id.clone(), execution.restore_retry(retry)?);
                    }
                } else if record.cancelled_run && !record.pending_retry {
                    let execution = self
                        .remove_execution(&issue_id)
                        .expect("active recovery execution should be present");
                    if record.interrupt_reason
                        == Some(HarnessInterruptReason::TrackerMergingSupersedesHumanReview)
                        && normalized_state_name(&normalized.state.name) == MERGING_STATE
                    {
                        let normal_retry_count = record.normal_retry_count.saturating_add(1);
                        if self.retry_count_exceeds_limit(normal_retry_count) {
                            self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                                .await?;
                            self.insert_execution(
                                issue_id.clone(),
                                execution.release(
                                    observed_at,
                                    ReleaseReason::RetryExhausted,
                                    None,
                                )?,
                            );
                            self.mark_recovered_retry_exhausted(
                                &issue_id,
                                record.normal_retry_count,
                                observed_at,
                            )?;
                        } else {
                            let retry = RetryEntry {
                                issue_id: normalized.id.clone(),
                                identifier: normalized.identifier.clone(),
                                attempt: RetryAttempt::new(normal_retry_count)?,
                                normal_retry_count,
                                scheduled_at: observed_at,
                                due_at: observed_at,
                                reason: RetryReason::Continuation,
                                error: None,
                            };
                            self.insert_execution(
                                issue_id.clone(),
                                execution.restore_retry(retry)?,
                            );
                        }
                    } else {
                        self.insert_execution(
                            issue_id.clone(),
                            execution.release(observed_at, ReleaseReason::Cancelled, None)?,
                        );
                    }
                } else if record.pending_retry {
                    let normal_retry_count = record.normal_retry_count.saturating_add(1);
                    // A retry whose count equals the configured maximum is
                    // still the final permitted dispatch; only a pending
                    // retry beyond that count must be parked here.
                    if self.retry_count_exceeds_limit(normal_retry_count) {
                        // The durable pending marker's count is the next
                        // undispatched attempt. Parking it must not turn
                        // that attempt into an already-consumed retry.
                        self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                            .await?;
                        self.mark_recovered_retry_exhausted(
                            &issue_id,
                            record.normal_retry_count,
                            observed_at,
                        )?;
                    } else {
                        let retry = RetryEntry {
                            issue_id: normalized.id.clone(),
                            identifier: normalized.identifier.clone(),
                            attempt: RetryAttempt::new(normal_retry_count)?,
                            normal_retry_count,
                            scheduled_at: record.retry_scheduled_at.unwrap_or(observed_at),
                            due_at: record.retry_due_at.unwrap_or(observed_at),
                            reason: record.retry_reason.unwrap_or(RetryReason::Reconciliation),
                            error: record.retry_error.clone(),
                        };
                        let execution = self
                            .remove_execution(&issue_id)
                            .expect("active recovery execution should be present");
                        self.insert_execution(issue_id.clone(), execution.restore_retry(retry)?);
                    }
                } else if record.successful_run {
                    let execution = self
                        .remove_execution(&issue_id)
                        .expect("active recovery execution should be present");
                    let already_exhausted = retry_exhausted_release(&execution);
                    if self.parent_waiting_for_tracker_confirmation(&issue_id) {
                        self.insert_execution(
                            issue_id.clone(),
                            execution.release(observed_at, ReleaseReason::Completed, None)?,
                        );
                    } else if self.retry_limit_reached(record.normal_retry_count) {
                        let mut execution = if already_exhausted {
                            execution
                        } else {
                            self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                                .await?;
                            execution.release(observed_at, ReleaseReason::RetryExhausted, None)?
                        };
                        execution.set_retry_count_override(record.normal_retry_count);
                        self.insert_execution(issue_id.clone(), execution);
                    } else {
                        let previous_attempt = (record.normal_retry_count > 0)
                            .then(|| RetryAttempt::new(record.normal_retry_count))
                            .transpose()?;
                        let retry = RetryEntry::continuation(
                            &normalized,
                            previous_attempt,
                            record.normal_retry_count,
                            observed_at,
                            self.config.retry_policy,
                        )?;
                        let execution = if already_exhausted {
                            execution.reopen(observed_at)?
                        } else {
                            execution
                        };
                        self.insert_execution(issue_id.clone(), execution.restore_retry(retry)?);
                    }
                } else if record.completed_run {
                    if self.retry_limit_reached(record.normal_retry_count) {
                        self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                            .await?;
                        self.mark_recovered_retry_exhausted(
                            &issue_id,
                            record.normal_retry_count,
                            observed_at,
                        )?;
                    } else {
                        let normal_retry_count = record.normal_retry_count.saturating_add(1);
                        let retry = RetryEntry {
                            issue_id: normalized.id.clone(),
                            identifier: normalized.identifier.clone(),
                            attempt: RetryAttempt::new(normal_retry_count)?,
                            normal_retry_count,
                            scheduled_at: observed_at,
                            due_at: observed_at,
                            reason: RetryReason::Reconciliation,
                            error: None,
                        };
                        let execution = self
                            .remove_execution(&issue_id)
                            .expect("active recovery execution should be present");
                        self.insert_execution(issue_id.clone(), execution.restore_retry(retry)?);
                    }
                } else if self.retry_limit_reached(record.normal_retry_count) {
                    self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                        .await?;
                    self.mark_recovered_retry_exhausted(
                        &issue_id,
                        record.normal_retry_count,
                        observed_at,
                    )?;
                } else if record.normal_retry_count > 0 {
                    let normal_retry_count = record.normal_retry_count.saturating_add(1);
                    if self.retry_count_exceeds_limit(normal_retry_count) {
                        self.persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                            .await?;
                        self.mark_recovered_retry_exhausted(
                            &issue_id,
                            record.normal_retry_count,
                            observed_at,
                        )?;
                        continue;
                    }
                    let retry = RetryEntry {
                        issue_id: normalized.id.clone(),
                        identifier: normalized.identifier.clone(),
                        attempt: RetryAttempt::new(normal_retry_count)?,
                        normal_retry_count,
                        scheduled_at: observed_at,
                        due_at: observed_at,
                        reason: RetryReason::Reconciliation,
                        error: None,
                    };
                    let execution = self
                        .remove_execution(&issue_id)
                        .expect("active recovery execution should be present");
                    self.insert_execution(issue_id.clone(), execution.restore_retry(retry)?);
                }
                continue;
            }

            if tracker_snapshot.contains_terminal(issue_id.as_str()) {
                let issue_has_children = tracker_snapshot
                    .terminal_issue(&issue_id)
                    .map(|issue| !issue.sub_issues.is_empty())
                    .unwrap_or(!record.issue.sub_issues.is_empty());
                let parent_finalized = self.parent_finalized_success(&issue_id, issue_has_children);
                if record.successful_run && !record.had_in_flight_run && parent_finalized {
                    self.record_terminal_orchestrator_success(&issue_id).await?;
                }
                if let Some(recovered_run) = record.recovered_run.as_ref() {
                    // A tracker terminal state does not prove that a run which
                    // was in flight at restart has stopped. Fence and stop the
                    // recovered worker before exposing a terminal outcome to
                    // parent eligibility.
                    if !self
                        .stop_recovered_run_before_workspace_removal(
                            &record,
                            recovered_run,
                            record.harness_kind.as_deref(),
                            observed_at,
                        )
                        .await?
                    {
                        retry_records.push(record);
                        continue;
                    }
                }
                if !parent_finalized {
                    let mut execution = IssueExecution::new(record.issue.clone(), observed_at);
                    execution.attach_workspace(record.workspace.clone())?;
                    self.insert_execution(
                        issue_id.clone(),
                        execution.release(observed_at, ReleaseReason::TrackerTerminal, None)?,
                    );
                    self.terminal_child_failure_ids.insert(issue_id.clone());
                    continue;
                }
                if let Err(error) = self
                    .retain_terminal_child_lease_for_workspace(&record.issue, &record.workspace)
                    .await
                {
                    retry_records.push(record);
                    retry_records.extend(records.iter().skip(record_index + 1).cloned());
                    self.pending_recovery = Some(retry_records);
                    return Err(error);
                }
                let workspace_has_active_lease =
                    match self.workspace_has_active_lease(&record.workspace).await {
                        Ok(has_active_lease) => has_active_lease,
                        Err(error) => {
                            retry_records.push(record);
                            retry_records.extend(records.iter().skip(record_index + 1).cloned());
                            self.pending_recovery = Some(retry_records);
                            return Err(error);
                        }
                    };
                if workspace_has_active_lease {
                    let mut execution = IssueExecution::new(record.issue.clone(), observed_at);
                    execution.attach_workspace(record.workspace.clone())?;
                    self.insert_execution(
                        issue_id.clone(),
                        execution.release(observed_at, ReleaseReason::TrackerTerminal, None)?,
                    );
                    if record.had_in_flight_run
                        || (record.completed_run && !record.successful_run && !record.cancelled_run)
                    {
                        self.terminal_child_failure_ids.insert(issue_id.clone());
                    }
                    retry_records.push(record);
                    continue;
                }
                let retain_failed = !record.successful_run
                    && !record.cancelled_run
                    && self.retry_limit_reached(record.normal_retry_count)
                    && self.workspace.retain_failed_workspaces();
                let parent_workspace =
                    self.is_parent_integration_workspace(&issue_id, issue_has_children);
                let cleanup_result = if parent_workspace || retain_failed {
                    Ok(())
                } else if !record.successful_run
                    && !record.cancelled_run
                    && self.retry_limit_reached(record.normal_retry_count)
                {
                    self.workspace
                        .cleanup_failed_workspace(&record.workspace)
                        .await
                } else {
                    self.workspace
                        .cleanup_workspace(&record.workspace, true)
                        .await
                };
                match cleanup_result {
                    Ok(()) => {
                        self.workspace
                            .clear_retry_exhaustion(record.issue.identifier.as_str())
                            .await
                            .map_err(|error| SchedulerError::Workspace {
                                detail: error.to_string(),
                            })?;
                        if parent_workspace {
                            let mut execution =
                                IssueExecution::new(record.issue.clone(), observed_at);
                            execution.attach_workspace(record.workspace.clone())?;
                            self.insert_execution(
                                issue_id.clone(),
                                execution.release(
                                    observed_at,
                                    ReleaseReason::TrackerTerminal,
                                    None,
                                )?,
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(issue = %issue_id, %error, "deferring terminal workspace cleanup retry");
                        retry_records.push(record);
                    }
                }
                continue;
            }

            let mut issue = record.issue.clone();
            if let Some(snapshot) = tracker_snapshot.state_by_id.get(issue_id.as_str()) {
                issue.state = issue_state_from_name(&snapshot.state.name, &self.config);
            }

            let mut execution = IssueExecution::new(issue.clone(), observed_at);
            execution.attach_workspace(record.workspace)?;
            let reason =
                if !record.cancelled_run && self.retry_limit_reached(record.normal_retry_count) {
                    ReleaseReason::RetryExhausted
                } else {
                    ReleaseReason::TrackerInactive
                };
            if reason == ReleaseReason::RetryExhausted {
                self.persist_retry_exhaustion(&issue, record.normal_retry_count)
                    .await?;
            }
            let mut execution = execution.release(observed_at, reason, None)?;
            if reason == ReleaseReason::RetryExhausted {
                execution.set_retry_count_override(record.normal_retry_count);
            }
            self.executions.entry(issue.id.clone()).or_insert(execution);
        }

        if retry_records.is_empty() {
            self.recovered = true;
        } else {
            self.pending_recovery = Some(retry_records);
        }
        if self.prune_recovered_memory_issue_ids() {
            self.hierarchy_state_dirty = true;
            self.persist_orchestrator_state().await?;
        }
        Ok(())
    }

    fn prune_recovered_memory_issue_ids(&mut self) -> bool {
        let mut retained = self
            .pending_recovery
            .as_ref()
            .into_iter()
            .flatten()
            .map(|record| record.issue.id.clone())
            .collect::<HashSet<_>>();
        retained.extend(
            self.pending_retry_recovery
                .as_ref()
                .into_iter()
                .flatten()
                .map(|record| record.issue.id.clone()),
        );
        retained.extend(
            self.executions
                .iter()
                .filter(|(_, execution)| !matches!(execution.status(), SchedulerStatus::Released))
                .map(|(issue_id, _)| issue_id.clone()),
        );
        let before = self.recovered_memory_issue_ids.len();
        self.recovered_memory_issue_ids
            .retain(|issue_id| retained.contains(issue_id));
        self.recovered_memory_issue_ids.len() != before
    }

    async fn reconcile_tracker_state(
        &mut self,
        tracker_snapshot: &TrackerSnapshot,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let durable_state_changed = self.reconcile_hierarchy_snapshots(tracker_snapshot)?
            || self.reconcile_terminal_hierarchy_snapshots(tracker_snapshot)?;
        self.fence_hierarchy_changed_runs(observed_at).await?;
        for tracker_issue in &tracker_snapshot.active {
            let normalized = normalize_tracker_issue(tracker_issue, &self.config)?;
            let retry_cleanup_workspace = self
                .executions
                .get(&normalized.id)
                .filter(|execution| retry_exhausted_release(execution))
                .and_then(|execution| execution.workspace().cloned());
            if let Some(workspace) = retry_cleanup_workspace
                && !self.workspace.retain_failed_workspaces()
                && !self.workspace_has_active_lease(&workspace).await?
            {
                match self.workspace.cleanup_failed_workspace(&workspace).await {
                    Ok(()) => {
                        if let Some(execution) = self.executions.get_mut(&normalized.id) {
                            execution.clear_workspace();
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            issue = %normalized.id,
                            %error,
                            "retry-exhausted workspace cleanup failed; will retry on the next reconciliation"
                        );
                    }
                }
            }
            if let Some(execution) = self.executions.get(&normalized.id) {
                let repository_binding_changed = RepositoryBindingOutcome::binding_changed_opt(
                    execution.issue().repository_binding.as_ref(),
                    normalized.repository_binding.as_ref(),
                );
                let workspace_key_changed = workspace_key_changed_for_issue(execution, &normalized);
                let retain_workspace = execution.workspace().is_some()
                    && project_identity_changed(execution.issue(), &normalized)
                    && !repository_binding_changed
                    && !workspace_key_changed;
                if execution.workspace().is_some()
                    && (repository_binding_changed || workspace_key_changed || retain_workspace)
                {
                    self.supersede_binding(
                        normalized.id.clone(),
                        normalized,
                        observed_at,
                        retain_workspace,
                    )
                    .await?;
                    continue;
                }
            }
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
                        issue.labels = snapshot.labels.clone();
                        if snapshot.project_identity_known {
                            issue.project_id = snapshot.project_id.clone();
                            issue.project_slug = snapshot.project_slug.clone();
                        }
                        if snapshot.is_parent {
                            self.parent_issue_ids.insert(issue_id.clone());
                        } else {
                            self.parent_issue_ids.remove(&issue_id);
                        }
                        issue.repository_binding = resolve_repository_binding(
                            &self.config,
                            &issue.labels,
                            issue.project_id.as_deref(),
                            issue.project_slug.as_deref(),
                            snapshot.is_parent,
                        );
                        if !project_belongs_to_configured_project(
                            &self.config,
                            issue.project_id.as_deref(),
                            issue.project_slug.as_deref(),
                            snapshot.project_identity_known,
                        ) {
                            self.release_issue(
                                issue_id.clone(),
                                issue,
                                observed_at,
                                ReleaseReason::TrackerInactive,
                                false,
                                Some(WorkerAbortReason::TrackerInactive),
                            )
                            .await?;
                            continue;
                        }
                        let repository_binding_changed =
                            RepositoryBindingOutcome::binding_changed_opt(
                                existing.issue().repository_binding.as_ref(),
                                issue.repository_binding.as_ref(),
                            );
                        let retain_workspace = project_identity_changed(existing.issue(), &issue)
                            && !repository_binding_changed;
                        if matches!(
                            existing.status(),
                            SchedulerStatus::Claimed
                                | SchedulerStatus::Running
                                | SchedulerStatus::RetryQueued
                        ) && (repository_binding_changed || retain_workspace)
                        {
                            self.supersede_binding(
                                issue_id.clone(),
                                issue,
                                observed_at,
                                retain_workspace,
                            )
                            .await?;
                            continue;
                        }
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

                if category == IssueStateCategory::NonActive
                    && self
                        .executions
                        .get(&issue_id)
                        .is_some_and(retry_exhausted_release)
                {
                    if let Some(execution) = self.executions.get(&issue_id) {
                        self.workspace
                            .revoke_issue_resources(execution.issue().identifier.as_str());
                    }
                    self.cleanup_retry_exhausted_workspace_if_ready(&issue_id)
                        .await;
                    self.refresh_execution_issue(&issue_id, normalized)?;
                    continue;
                }

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

        if durable_state_changed || self.hierarchy_state_dirty {
            self.persist_orchestrator_state().await?;
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
                if execution.issue().sub_issues.is_empty()
                    && self.parent_issue_ids.contains(&execution.issue().id)
                {
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

        let command =
            Self::prepare_merging_interrupt(&mut execution, issue, harness_kind, observed_at)?;
        if let Some(command) = command {
            self.persist_interrupt_intent(&execution).await?;
            let result = self.worker.interrupt_worker(command).await;
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
            Self::apply_interrupt_result(
                &mut execution,
                HarnessInterruptReason::TrackerMergingSupersedesHumanReview,
                observed_at,
                result,
            )?;
        }

        self.insert_execution(issue_id, execution);
        Ok(true)
    }

    async fn stop_recovered_run_before_workspace_removal(
        &mut self,
        record: &RecoveryRecord,
        recovered_run: &RecoveredRun,
        harness_kind: Option<&str>,
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        let run = RunAttempt::new(
            recovered_run.worker_id.clone(),
            record.issue.id.clone(),
            record.issue.identifier.clone(),
            record.workspace.path.clone(),
            observed_at,
            (recovered_run.normal_retry_count > 0)
                .then(|| RetryAttempt::new(recovered_run.normal_retry_count))
                .transpose()?,
            self.config.max_turns,
        )
        .with_normal_retry_count(recovered_run.normal_retry_count)
        .with_repository_binding(recovered_run.repository_binding.clone());
        let mut execution = IssueExecution::new(record.issue.clone(), observed_at);
        execution.attach_workspace(record.workspace.clone())?;
        execution = execution.claim(run.clone())?.start_running(
            observed_at,
            effective_stall_timeout(self.config.stall_timeout_ms),
            Some(recovered_run.conversation.clone()),
        )?;
        self.worker_metadata.insert(
            recovered_run.worker_id.clone(),
            WorkerMetadata::new(
                record.issue.id.clone(),
                harness_kind
                    .filter(|kind| !kind.trim().is_empty())
                    .map(str::to_owned),
            ),
        );
        let remote_stopped = self
            .abort_worker(
                &mut execution,
                &run,
                WorkerAbortReason::BindingSuperseded,
                observed_at,
            )
            .await?;
        if !remote_stopped {
            self.insert_execution(record.issue.id.clone(), execution);
        }
        Ok(remote_stopped)
    }

    async fn supersede_binding(
        &mut self,
        issue_id: IssueId,
        replacement: NormalizedIssue,
        observed_at: TimestampMs,
        retain_workspace: bool,
    ) -> Result<(), SchedulerError> {
        let Some(mut execution) = self.remove_execution(&issue_id) else {
            self.insert_execution(issue_id, IssueExecution::new(replacement, observed_at));
            return Ok(());
        };

        if let Some(run) = execution.current_run().cloned() {
            let remote_stopped = match self
                .abort_worker(
                    &mut execution,
                    &run,
                    WorkerAbortReason::BindingSuperseded,
                    observed_at,
                )
                .await
            {
                Ok(remote_stopped) => remote_stopped,
                Err(error) => {
                    // The old generation remains authoritative when stop
                    // persistence or the harness abort fails. Reinsert it so
                    // the next reconciliation retries the stop instead of
                    // dispatching a replacement beside a live worker.
                    self.insert_execution(issue_id.clone(), execution);
                    return Err(error);
                }
            };
            if !remote_stopped {
                // Keep the old generation fenced until the harness confirms
                // its stop. A later reconciliation retries the stop instead
                // of allowing the replacement to share an active worker.
                self.insert_execution(issue_id, execution);
                return Ok(());
            }
        }

        // Binding or project supersession ends the old issue scope even when
        // the retained checkout is idle and no live worker needed aborting.
        self.workspace
            .revoke_issue_resources(execution.issue().identifier.as_str());

        let retry = execution.retry().cloned();
        if !retain_workspace
            && let Some(workspace) = execution.workspace().cloned()
            && let Err(error) = self.workspace.remove_workspace(&workspace).await
        {
            self.insert_execution(issue_id, execution);
            return Err(SchedulerError::Workspace {
                detail: error.to_string(),
            });
        }

        let has_resolved_replacement = replacement
            .repository_binding
            .as_ref()
            .and_then(RepositoryBindingOutcome::resolved_binding)
            .is_some();
        let mut replacement_execution = IssueExecution::new(replacement.clone(), observed_at);
        if let Some(retry) = retry.as_ref() {
            replacement_execution = replacement_execution.restore_retry(retry.clone())?;
        }
        let replacement_workspace = if retain_workspace {
            execution.workspace().cloned()
        } else if retry.is_some() && has_resolved_replacement {
            match self
                .workspace
                .ensure_workspace(&replacement, observed_at)
                .await
            {
                Ok(workspace) => Some(workspace),
                Err(error) => {
                    if let Some(retry) = retry.as_ref()
                        && let Err(persist_error) = self
                            .workspace
                            .persist_retry_pending_without_workspace(&replacement, retry)
                            .await
                    {
                        self.insert_execution(issue_id.clone(), replacement_execution);
                        return Err(SchedulerError::Workspace {
                            detail: format!(
                                "{}; retry persistence also failed: {}",
                                error, persist_error
                            ),
                        });
                    }
                    self.insert_execution(issue_id, replacement_execution);
                    return Err(SchedulerError::Workspace {
                        detail: error.to_string(),
                    });
                }
            }
        } else {
            None
        };
        if let Some(workspace) = replacement_workspace.as_ref() {
            replacement_execution.attach_workspace(workspace.clone())?;
        }
        if let Some(retry) = retry {
            if let Some(workspace) = replacement_workspace.as_ref() {
                if let Err(error) = self
                    .workspace
                    .persist_retry_pending(workspace, &retry)
                    .await
                {
                    self.pending_retry_persistence
                        .insert(issue_id.clone(), retry.clone());
                    self.insert_execution(issue_id.clone(), replacement_execution);
                    return Err(SchedulerError::Workspace {
                        detail: error.to_string(),
                    });
                }
                if let Err(error) = self.workspace.clear_retry_pending(&issue_id).await {
                    self.insert_execution(issue_id.clone(), replacement_execution);
                    return Err(SchedulerError::Workspace {
                        detail: error.to_string(),
                    });
                }
            } else {
                if let Err(error) = self
                    .workspace
                    .persist_retry_pending_without_workspace(&replacement, &retry)
                    .await
                {
                    self.insert_execution(issue_id.clone(), replacement_execution);
                    return Err(SchedulerError::Workspace {
                        detail: error.to_string(),
                    });
                }
            }
        }
        self.insert_execution(issue_id, replacement_execution);
        Ok(())
    }

    fn merging_interrupt_candidate(
        &self,
        issue: &NormalizedIssue,
    ) -> Option<(IssueId, IssueExecution, RunAttempt, String)> {
        let existing = self.executions.get(&issue.id)?;
        let retrying_failed_merging_interrupt = existing.interrupt().is_some_and(|interrupt| {
            interrupt.command.reason == HarnessInterruptReason::TrackerMergingSupersedesHumanReview
                && matches!(
                    interrupt.status,
                    HarnessInterruptStatus::Failed | HarnessInterruptStatus::TimedOut
                )
                && normalized_state_name(&existing.issue().state.name) == MERGING_STATE
                && normalized_state_name(&issue.state.name) == MERGING_STATE
        });
        if (!is_human_review_to_merging(existing.issue(), issue)
            && !retrying_failed_merging_interrupt)
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
            .worker_metadata
            .get(&run.worker_id)
            .and_then(|metadata| metadata.harness_kind.clone())
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

    fn operator_cancel_candidate(
        &self,
        target: &str,
    ) -> Option<(IssueId, IssueExecution, RunAttempt, String)> {
        let (issue_id, execution) = self.executions.iter().find(|(_, execution)| {
            execution
                .issue()
                .identifier
                .as_str()
                .eq_ignore_ascii_case(target)
                || execution
                    .current_run()
                    .is_some_and(|run| run.issue_identifier.as_str().eq_ignore_ascii_case(target))
                || execution
                    .conversation()
                    .is_some_and(|conversation| conversation.conversation_id.as_str() == target)
        })?;
        if !matches!(
            execution.status(),
            SchedulerStatus::Claimed | SchedulerStatus::Running
        ) {
            return None;
        }

        let execution = execution.clone();
        let run = execution.current_run().cloned()?;
        let harness_kind = self
            .worker_metadata
            .get(&run.worker_id)
            .and_then(|metadata| metadata.harness_kind.clone())
            .unwrap_or_else(|| "<unknown>".to_string());
        Some((issue_id.clone(), execution, run, harness_kind))
    }

    fn prepare_merging_interrupt(
        execution: &mut IssueExecution,
        issue: &NormalizedIssue,
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

        Ok(Some(command))
    }

    async fn persist_interrupt_intent(
        &mut self,
        execution: &IssueExecution,
    ) -> Result<(), SchedulerError> {
        let Some(workspace) = execution.workspace().cloned() else {
            return Ok(());
        };
        let Some(reason) = execution
            .interrupt()
            .map(|interrupt| interrupt.command.reason)
        else {
            return Ok(());
        };
        self.workspace
            .persist_interrupt_reason(&workspace, reason)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })
    }

    fn apply_interrupt_result(
        execution: &mut IssueExecution,
        reason: HarnessInterruptReason,
        observed_at: TimestampMs,
        result: Result<WorkerInterruptAcknowledgement, M::Error>,
    ) -> Result<(), SchedulerError> {
        match result {
            Ok(acknowledgement) if acknowledgement.timed_out => {
                execution.timeout_interrupt(
                    observed_at,
                    acknowledgement.detail.unwrap_or_else(|| {
                        "worker interrupt acknowledgement timed out".to_string()
                    }),
                )?;
            }
            Ok(acknowledgement) if acknowledgement.accepted => {
                execution.acknowledge_interrupt(observed_at)?;
                if let Some(detail) = acknowledgement.detail {
                    execution.observe_runtime_event(
                        observed_at,
                        Some(format!(
                            "{}-interrupt-acknowledged-{}",
                            reason.as_str(),
                            observed_at.as_u64()
                        )),
                        Some("scheduler.interrupt_acknowledged".to_string()),
                        Some(detail),
                        Some(serde_json::json!({
                            "reason": reason.as_str(),
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

    /// Migrate runs written before parent controllers were persisted. The
    /// recovered run manifest and parent workspace envelope are the durable
    /// proof that launch already happened, so recreate the missing controller
    /// and bind it to that exact conversation before backend reattachment.
    async fn migrate_legacy_in_flight_parent_controllers(
        &mut self,
        records: &[RecoveryRecord],
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        let mut changed = false;
        for record in records {
            if !record.had_in_flight_run {
                continue;
            }
            let Some(recovered_run) = record.recovered_run.as_ref() else {
                continue;
            };
            if self
                .hierarchy_state
                .parent_integrations
                .contains_key(&record.issue.id)
            {
                continue;
            }
            let Some(snapshot) = self
                .hierarchy_state
                .hierarchy
                .get(&record.issue.id)
                .cloned()
            else {
                continue;
            };
            if snapshot.required_child_edges.is_empty() && record.issue.sub_issues.is_empty() {
                continue;
            }

            let targets = self
                .workspace
                .parent_workspace_targets(&record.issue, &record.workspace)
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?;
            let started_at = self
                .hierarchy_state
                .run_started_at_by_issue
                .get(&record.issue.id)
                .copied()
                .unwrap_or(observed_at);
            let admission_input_version = parent_input_version(&snapshot);
            let targets_input_version = parent_targets_input_version(snapshot.generation, &targets);
            let mut controller =
                ParentIntegrationController::new(record.issue.id.clone(), snapshot.generation)?;
            controller.admit(&admission_input_version, started_at)?;
            controller.record_workspace_prepared(targets, &targets_input_version, started_at)?;
            controller.start_attempt(
                "parent integration harness run",
                format!("parent-run:{}", recovered_run.worker_id),
                ParentAttemptRoot::ParentRoot,
                recovered_run.conversation.conversation_id.as_str(),
                effective_stall_timeout(self.config.stall_timeout_ms).as_u64(),
                targets_input_version,
                started_at,
            )?;
            self.hierarchy_state
                .parent_integrations
                .insert(record.issue.id.clone(), controller);
            changed = true;
        }
        Ok(changed)
    }

    async fn restore_recovered_run(
        &mut self,
        issue_id: &IssueId,
        recovered_run: Option<RecoveredRun>,
        harness_kind: Option<String>,
        interrupt_reason: Option<HarnessInterruptReason>,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let Some(recovered_run) = recovered_run else {
            return Ok(());
        };
        let Some(current_execution) = self.executions.get(issue_id).cloned() else {
            return Ok(());
        };
        if current_execution.status() != SchedulerStatus::Unclaimed {
            return Ok(());
        }
        let Some(workspace) = current_execution.workspace().cloned() else {
            return Ok(());
        };
        let retry = if recovered_run.normal_retry_count > 0 {
            Some(RetryEntry {
                issue_id: current_execution.issue().id.clone(),
                identifier: current_execution.issue().identifier.clone(),
                attempt: RetryAttempt::new(recovered_run.normal_retry_count)?,
                normal_retry_count: recovered_run.normal_retry_count,
                scheduled_at: observed_at,
                due_at: observed_at,
                reason: RetryReason::Reconciliation,
                error: None,
            })
        } else {
            None
        };
        let recovered_binding = recovered_run
            .repository_binding
            .clone()
            .map(RepositoryBindingOutcome::Resolved);
        let binding_changed = RepositoryBindingOutcome::binding_changed_opt(
            current_execution.issue().repository_binding.as_ref(),
            recovered_binding.as_ref(),
        );
        let mut recovery_issue = current_execution.issue().clone();
        if binding_changed {
            // The persisted run owns the old generation. Attach that binding
            // before claim so a canonical identity change can be superseded
            // safely after reattachment. Same-identity recovery keeps the
            // refreshed issue metadata while the run retains its proof.
            recovery_issue.repository_binding = recovered_binding.clone();
        }
        let mut execution = current_execution.clone();
        if execution.issue() != &recovery_issue {
            execution.refresh_issue(recovery_issue.clone())?;
        }
        if let Some(retry) = retry {
            execution = execution.restore_retry(retry)?;
        }
        let run = RunAttempt::new(
            recovered_run.worker_id.clone(),
            recovery_issue.id.clone(),
            recovery_issue.identifier.clone(),
            workspace.path.clone(),
            observed_at,
            (recovered_run.normal_retry_count > 0)
                .then(|| RetryAttempt::new(recovered_run.normal_retry_count))
                .transpose()?,
            self.config.max_turns,
        )
        .with_normal_retry_count(recovered_run.normal_retry_count)
        .with_repository_binding(recovered_run.repository_binding.clone().or_else(|| {
            recovery_issue
                .repository_binding
                .as_ref()
                .and_then(RepositoryBindingOutcome::resolved_binding)
                .cloned()
        }));
        let route = recovered_route(
            decide_issue_route(&recovery_issue, &self.config)?,
            harness_kind.as_deref(),
        )?;
        execution = execution.claim(run.clone())?;
        let expected_parent_conversation_id = (!recovery_issue.sub_issues.is_empty())
            .then(|| {
                self.hierarchy_state
                    .parent_integrations
                    .get(issue_id)
                    .and_then(|controller| controller.conversation_id.clone())
            })
            .flatten();
        let start_request = WorkerStartRequest {
            issue: recovery_issue,
            workspace: workspace.clone(),
            run: run.clone(),
            route: route.clone(),
            memory_grant_registry_recovered: self.recovered_memory_issue_ids.contains(issue_id),
            expected_parent_conversation_id,
            parent_repair: self
                .hierarchy_state
                .parent_integrations
                .get(issue_id)
                .and_then(|controller| {
                    controller
                        .repair_attempts
                        .iter()
                        .rev()
                        .find(|repair| {
                            repair.status == ParentRepairStatus::ChangesRequested
                                || (repair.status == ParentRepairStatus::Implementing
                                    && !repair.implementation_completed)
                        })
                        .cloned()
                }),
        };
        self.remove_execution(issue_id);
        let launch = match self.worker.recover_worker(start_request).await {
            Ok(launch) => launch,
            Err(error) => {
                let detail = error.to_string();
                warn!(
                    issue_id = %issue_id,
                    error = %detail,
                    "failed to reattach recovered scheduler worker"
                );
                let outcome = WorkerOutcomeRecord::from_run(
                    &run,
                    WorkerOutcomeKind::Failed,
                    observed_at,
                    Some("failed to reattach recovered worker".to_string()),
                    Some(detail),
                );
                let execution = self
                    .resolve_finished_execution(execution, outcome, observed_at)
                    .await?;
                self.insert_execution(issue_id.clone(), execution);
                self.persist_retry_if_queued(issue_id).await?;
                return Ok(());
            }
        };
        self.recovered_memory_issue_ids.remove(issue_id);
        execution = execution.start_running(
            observed_at,
            effective_stall_timeout(self.config.stall_timeout_ms),
            Some(launch.conversation.clone()),
        )?;
        execution.record_turn_started(observed_at)?;
        if let Some(reason) = interrupt_reason {
            let expected_next_state = match reason {
                HarnessInterruptReason::OperatorCancel => HarnessInterruptExpectedNextState::Paused,
                HarnessInterruptReason::TrackerMergingSupersedesHumanReview => {
                    HarnessInterruptExpectedNextState::CloseoutPending
                }
                HarnessInterruptReason::SchedulerAbort => {
                    HarnessInterruptExpectedNextState::Released
                }
            };
            execution.restore_interrupt_intent(
                harness_kind
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_string()),
                reason,
                expected_next_state,
                observed_at,
            )?;
        }
        self.worker_metadata.insert(
            run.worker_id.clone(),
            WorkerMetadata::new(
                issue_id.clone(),
                harness_kind
                    .filter(|kind| !kind.trim().is_empty())
                    .or(Some(route.harness_kind.clone())),
            )
            .with_dry_run(route.dry_run),
        );
        if !execution.issue().sub_issues.is_empty() && !route.dry_run {
            let conversation_id = launch.conversation.conversation_id.to_string();
            let previous_state = self.hierarchy_state.clone();
            let attachment_result = self
                .hierarchy_state
                .parent_integrations
                .get(issue_id)
                .and_then(|controller| controller.current_attempt_id())
                .map(str::to_owned)
                .ok_or_else(|| SchedulerError::Workspace {
                    detail: "recovered parent launch has no durable attempt intent".to_owned(),
                })
                .and_then(|attempt_id| {
                    self.hierarchy_state
                        .parent_integrations
                        .get_mut(issue_id)
                        .ok_or_else(|| SchedulerError::Workspace {
                            detail: "recovered parent launch has no durable integration controller"
                                .to_owned(),
                        })?
                        .attach_attempt_conversation(&attempt_id, &conversation_id)
                        .map_err(SchedulerError::from)
                });
            if let Err(error) = attachment_result {
                self.hierarchy_state = previous_state;
                execution = self
                    .retain_recovered_execution_after_attachment_failure(
                        issue_id,
                        execution,
                        &run,
                        observed_at,
                        &error.to_string(),
                    )
                    .await?;
                self.insert_execution(issue_id.clone(), execution);
                return Err(error);
            }
            if let Err(error) = self.persist_orchestrator_state().await {
                self.hierarchy_state = previous_state;
                execution = self
                    .retain_recovered_execution_after_attachment_failure(
                        issue_id,
                        execution,
                        &run,
                        observed_at,
                        &error.to_string(),
                    )
                    .await?;
                self.insert_execution(issue_id.clone(), execution);
                return Err(error);
            }
        }
        self.insert_execution(issue_id.clone(), execution);
        Ok(())
    }

    async fn retain_recovered_execution_after_attachment_failure(
        &mut self,
        issue_id: &IssueId,
        mut execution: IssueExecution,
        run: &RunAttempt,
        observed_at: TimestampMs,
        detail: &str,
    ) -> Result<IssueExecution, SchedulerError> {
        match self
            .abort_worker(
                &mut execution,
                run,
                WorkerAbortReason::TrackerInactive,
                observed_at,
            )
            .await
        {
            Ok(true) => {
                let outcome = WorkerOutcomeRecord::from_run(
                    run,
                    WorkerOutcomeKind::Failed,
                    observed_at,
                    Some("recovered parent attachment was not durable".to_owned()),
                    Some(detail.to_owned()),
                );
                execution = execution.release(
                    observed_at,
                    ReleaseReason::TrackerInactive,
                    Some(outcome),
                )?;
            }
            Ok(false) => warn!(
                issue_id = %issue_id,
                "retaining recovered parent worker after durable attachment failure until its stop is acknowledged"
            ),
            Err(abort_error) => warn!(
                issue_id = %issue_id,
                error = %abort_error,
                "retaining recovered parent worker after durable attachment failure because abort failed"
            ),
        }
        Ok(execution)
    }

    async fn retry_recovered_interrupt(
        &mut self,
        issue_id: &IssueId,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let Some(mut execution) = self.remove_execution(issue_id) else {
            return Ok(());
        };
        let Some(interrupt) = execution.interrupt().cloned() else {
            self.insert_execution(issue_id.clone(), execution);
            return Ok(());
        };
        if !matches!(
            interrupt.status,
            HarnessInterruptStatus::Failed | HarnessInterruptStatus::TimedOut
        ) {
            self.insert_execution(issue_id.clone(), execution);
            return Ok(());
        }

        let request = execution.request_interrupt(
            interrupt.command.harness_kind,
            interrupt.command.turn_id,
            interrupt.command.reason,
            interrupt.command.expected_next_state,
            observed_at,
        );
        let (command, queued) = match request {
            Ok(request) => request,
            Err(error) => {
                self.insert_execution(issue_id.clone(), execution);
                return Err(error.into());
            }
        };
        if queued {
            let reason = command.reason;
            let result = self.worker.interrupt_worker(command).await;
            if let Err(error) =
                Self::apply_interrupt_result(&mut execution, reason, observed_at, result)
            {
                self.insert_execution(issue_id.clone(), execution);
                return Err(error);
            }
        }
        self.insert_execution(issue_id.clone(), execution);
        Ok(())
    }

    fn retry_limit_reached(&self, normal_retry_count: u32) -> bool {
        self.config
            .max_retry_attempts
            .is_some_and(|max_attempts| normal_retry_count >= max_attempts)
    }

    fn retry_count_exceeds_limit(&self, normal_retry_count: u32) -> bool {
        self.config
            .max_retry_attempts
            .is_some_and(|max_attempts| normal_retry_count > max_attempts)
    }

    fn mark_recovered_retry_exhausted(
        &mut self,
        issue_id: &IssueId,
        normal_retry_count: u32,
        observed_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let Some(execution) = self.remove_execution(issue_id) else {
            return Ok(());
        };
        if retry_exhausted_release(&execution) {
            self.insert_execution(issue_id.clone(), execution);
            return Ok(());
        }
        let mut execution = execution.release(observed_at, ReleaseReason::RetryExhausted, None)?;
        execution.set_retry_count_override(normal_retry_count);
        self.insert_execution(issue_id.clone(), execution);
        Ok(())
    }

    fn has_reconcilable_executions(&self) -> bool {
        self.executions.values().any(|execution| {
            matches!(
                execution.status(),
                SchedulerStatus::Claimed | SchedulerStatus::Running | SchedulerStatus::RetryQueued
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
            .saturating_sub(self.worker_metadata.len());
        if available_capacity == 0 {
            return Ok(());
        }

        let identifiers = ready
            .iter()
            .map(|issue| issue.identifier.clone())
            .collect::<Vec<_>>();
        if identifiers.is_empty() {
            return Ok(());
        }
        let batch_size = available_capacity.max(1);
        for identifier_batch in identifiers.chunks(batch_size) {
            let mut detailed_by_identifier = match self
                .tracker
                .issues_by_identifiers(identifier_batch)
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
            for summary in ready
                .iter()
                .filter(|summary| identifier_batch.contains(&summary.identifier))
            {
                let key = summary.identifier.to_ascii_uppercase();
                let Some(detailed_issue) = detailed_by_identifier.remove(&key) else {
                    warn!(
                        identifier = %summary.identifier,
                        "skipping stale dispatch candidate missing from detail refresh"
                    );
                    continue;
                };
                if !tracker_issue_belongs_to_configured_project(&detailed_issue, &self.config) {
                    warn!(
                        identifier = %detailed_issue.identifier,
                        "skipping dispatch candidate outside the configured tracker project"
                    );
                    continue;
                }
                let normalized = normalize_tracker_issue(&detailed_issue, &self.config)?;
                if normalized.state.category != IssueStateCategory::Active {
                    warn!(
                        identifier = %normalized.identifier,
                        state = %normalized.state.name,
                        "skipping dispatch candidate no longer in an active state"
                    );
                    continue;
                }
                // A released execution blocks dispatch until the hourly full
                // refresh reconciles it. When the tracker reactivates such an
                // issue (e.g. Backlog back to Todo after its workspace was
                // recovered and parked), reopen it here so the 60s discovery
                // cadence picks it up instead.
                let needs_reopen = self
                    .executions
                    .get(&normalized.id)
                    .is_some_and(|execution| {
                        execution.status() == SchedulerStatus::Released
                            && !terminal_worker_outcome_prevents_reopen(execution)
                    });
                if needs_reopen {
                    if self
                        .interrupt_human_review_polling_for_merging(&normalized, observed_at)
                        .await?
                    {
                        continue;
                    }
                    self.upsert_active_execution(normalized, observed_at, None)?;
                }
                detailed.push(detailed_issue);
            }

            self.dispatch_ready_issues(&detailed, observed_at, None)
                .await?;
            if self.worker_metadata.len()
                >= usize::try_from(self.config.max_concurrent_agents).unwrap_or(usize::MAX)
            {
                break;
            }
        }

        Ok(())
    }

    async fn dispatch_ready_issues(
        &mut self,
        active_issues: &[TrackerIssue],
        observed_at: TimestampMs,
        reachable_child_edges: Option<&BTreeSet<(IssueId, IssueId)>>,
    ) -> Result<(), SchedulerError> {
        let ready =
            filter_issues_for_dispatch(active_issues.to_vec(), &self.config.terminal_state_set());
        let available_capacity = usize::try_from(self.config.max_concurrent_agents)
            .unwrap_or(usize::MAX)
            .saturating_sub(self.worker_metadata.len());
        if available_capacity == 0 {
            return Ok(());
        }

        let mut pending_launches = Vec::new();
        let mut first_error = None;
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

            if normalized
                .repository_binding
                .as_ref()
                .is_some_and(|binding| binding.resolved_binding().is_none())
            {
                // Keep typed routing failures visible to the control plane but
                // never materialize a workspace for a blocked candidate.
                let mut execution = self
                    .remove_execution(&issue_id)
                    .unwrap_or_else(|| IssueExecution::new(normalized.clone(), observed_at));
                execution.refresh_issue(normalized)?;
                self.insert_execution(issue_id, execution);
                continue;
            }

            let state_key = normalized_state_name(&normalized.state.name);
            let issue_id = normalized.id.clone();

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

            if self.reconcile_hierarchy_issue(&tracker_issue, None)? || self.hierarchy_state_dirty {
                self.hierarchy_state_dirty = true;
                self.persist_orchestrator_state().await?;
            }
            // Keep the known execution hydrated with the same detailed issue
            // that just drove hierarchy reconciliation. The later known-
            // candidate pass must not reintroduce an older child-edge list.
            self.refresh_execution_issue(&issue_id, normalized.clone())?;

            if normalized.sub_issues.is_empty() && self.parent_issue_ids.contains(&normalized.id) {
                continue;
            }

            // Check the ancestor fence before parent eligibility can persist a
            // dispatch intent and acquire leases. A nested parent may be
            // reactivated while a higher parent is integrating retained
            // evidence; leaving the intent behind would make the nested
            // parent permanently undispatchable after the higher lease ends.
            if self
                .hierarchy_state
                .has_active_dispatched_ancestor(&issue_id)
            {
                continue;
            }

            let parent_repair_retry = self
                .hierarchy_state
                .parent_integrations
                .get(&issue_id)
                .is_some_and(|controller| {
                    matches!(
                        controller.state,
                        super::ParentIntegrationState::Fixing { .. }
                    )
                });
            if !normalized.sub_issues.is_empty() && !parent_repair_retry {
                let parent_retry = self
                    .executions
                    .get(&issue_id)
                    .is_some_and(|execution| execution.status() == SchedulerStatus::RetryQueued);
                if !self
                    .parent_dispatch_is_eligible(
                        &tracker_issue,
                        &normalized,
                        observed_at,
                        parent_retry,
                        reachable_child_edges,
                    )
                    .await?
                {
                    continue;
                }
            }

            if let Some(hierarchy_generation) = self
                .hierarchy_state
                .hierarchy
                .values()
                .filter(|snapshot| {
                    snapshot
                        .required_child_edges
                        .iter()
                        .any(|edge| edge.required && edge.child_id == issue_id)
                })
                .map(|snapshot| snapshot.generation)
                .next()
            {
                let previous_state = self.hierarchy_state.clone();
                self.hierarchy_state
                    .run_hierarchy_generations
                    .insert(issue_id.clone(), hierarchy_generation);
                if let Err(error) = self.persist_orchestrator_state().await {
                    self.hierarchy_state = previous_state;
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error)
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            }

            let workspace = match self
                .workspace
                .ensure_workspace(&normalized, observed_at)
                .await
            {
                Ok(workspace) => workspace,
                Err(error) => {
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(
                            &issue_id,
                            SchedulerError::Workspace {
                                detail: error.to_string(),
                            },
                        )
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };

            if !normalized.sub_issues.is_empty() && !parent_repair_retry {
                let targets = match self
                    .workspace
                    .parent_workspace_targets(&normalized, &workspace)
                    .await
                {
                    Ok(targets) => targets,
                    Err(error) => {
                        let error = self
                            .clear_parent_dispatch_intent_after_preparation_failure(
                                &issue_id,
                                SchedulerError::Workspace {
                                    detail: error.to_string(),
                                },
                            )
                            .await;
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                        continue;
                    }
                };
                let input_version = parent_targets_input_version(
                    self.hierarchy_state
                        .hierarchy
                        .get(&issue_id)
                        .map_or(0, |snapshot| snapshot.generation),
                    &targets,
                );
                let previous_state = self.hierarchy_state.clone();
                let result = self
                    .hierarchy_state
                    .parent_integrations
                    .get_mut(&issue_id)
                    .ok_or_else(|| SchedulerError::Workspace {
                        detail: "parent workspace has no durable integration controller".to_owned(),
                    })?
                    .record_workspace_prepared(targets, &input_version, observed_at);
                if let Err(error) = result {
                    self.hierarchy_state = previous_state;
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(
                            &issue_id,
                            error.into(),
                        )
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
                if let Err(error) = self.persist_orchestrator_state().await {
                    self.hierarchy_state = previous_state;
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error)
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            }

            if let Some(normal_retry_count) = self
                .executions
                .get(&issue_id)
                .and_then(IssueExecution::retry)
                .map(|retry| retry.normal_retry_count)
            {
                // Keep the durable pending-retry marker intact until the
                // worker's start_run preparation writes the replacement run
                // manifest. A crash before start_workers must recover the
                // queued retry rather than an advanced, unqueued count.
                if let Err(error) = self
                    .workspace
                    .persist_retry_count(&workspace, normal_retry_count)
                    .await
                {
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(
                            &issue_id,
                            SchedulerError::Workspace {
                                detail: error.to_string(),
                            },
                        )
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            }

            let worker_id = match self.next_worker_id() {
                Ok(worker_id) => worker_id,
                Err(error) => {
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error)
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };
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
            )
            .with_repository_binding(
                normalized
                    .repository_binding
                    .as_ref()
                    .and_then(RepositoryBindingOutcome::resolved_binding)
                    .cloned(),
            );
            let route = match decide_issue_route(&normalized, &self.config) {
                Ok(route) => route,
                Err(error) => {
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error)
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };

            let mut execution = self
                .remove_execution(&issue_id)
                .unwrap_or_else(|| IssueExecution::new(normalized.clone(), observed_at));
            if let Err(error) = execution.refresh_issue(normalized.clone()) {
                self.insert_execution(issue_id.clone(), execution);
                let error = self
                    .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error.into())
                    .await;
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
            if let Err(error) = execution.attach_workspace(workspace.clone()) {
                self.insert_execution(issue_id.clone(), execution);
                let error = self
                    .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error.into())
                    .await;
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
            let execution_before_claim = execution.clone();
            let unclaimed_execution = execution_before_claim.clone();
            execution = match execution.claim(run.clone()) {
                Ok(execution) => execution,
                Err(error) => {
                    self.insert_execution(issue_id.clone(), execution_before_claim);
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(
                            &issue_id,
                            error.into(),
                        )
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };
            let claimed_run = execution
                .current_run()
                .cloned()
                .expect("claimed execution must expose the claimed run");

            let start_request = WorkerStartRequest {
                issue: normalized.clone(),
                workspace,
                run: claimed_run.clone(),
                route,
                memory_grant_registry_recovered: self
                    .recovered_memory_issue_ids
                    .contains(&issue_id),
                expected_parent_conversation_id: (!normalized.sub_issues.is_empty())
                    .then(|| {
                        self.hierarchy_state
                            .parent_integrations
                            .get(&issue_id)
                            .and_then(|controller| controller.conversation_id.clone())
                    })
                    .flatten(),
                parent_repair: self
                    .hierarchy_state
                    .parent_integrations
                    .get(&issue_id)
                    .and_then(|controller| {
                        controller
                            .repair_attempts
                            .iter()
                            .rev()
                            .find(|repair| {
                                repair.status == ParentRepairStatus::ChangesRequested
                                    || (repair.status == ParentRepairStatus::Implementing
                                        && !repair.implementation_completed)
                            })
                            .cloned()
                    }),
            };

            if !normalized.sub_issues.is_empty() && !start_request.route.dry_run {
                let previous_state = self.hierarchy_state.clone();
                let input_version = self
                    .hierarchy_state
                    .parent_integrations
                    .get(&issue_id)
                    .map(parent_controller_input_version)
                    .unwrap_or_default();
                let result = self
                    .hierarchy_state
                    .parent_integrations
                    .get_mut(&issue_id)
                    .ok_or_else(|| SchedulerError::Workspace {
                        detail: "parent launch has no durable integration controller".to_owned(),
                    })?
                    .start_attempt_intent(
                        "parent integration harness run",
                        format!("parent-run:{}", claimed_run.worker_id),
                        ParentAttemptRoot::ParentRoot,
                        effective_stall_timeout(self.config.stall_timeout_ms).as_u64(),
                        input_version,
                        observed_at,
                    );
                if let Err(error) = result {
                    self.hierarchy_state = previous_state;
                    self.insert_execution(issue_id.clone(), unclaimed_execution);
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(
                            &issue_id,
                            error.into(),
                        )
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
                if let Err(error) = self.persist_orchestrator_state().await {
                    self.hierarchy_state = previous_state;
                    self.insert_execution(issue_id.clone(), unclaimed_execution);
                    let error = self
                        .clear_parent_dispatch_intent_after_preparation_failure(&issue_id, error)
                        .await;
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            }

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
            pending_launches.into_iter().zip(start_results)
        {
            match result {
                Ok(launch) => {
                    self.recovered_memory_issue_ids.remove(&issue_id);
                    let started_at = launch.started_at.unwrap_or(observed_at);
                    if !execution.issue().sub_issues.is_empty() && !start_request.route.dry_run {
                        let conversation_id = launch.conversation.conversation_id.to_string();
                        let attempt_id = self
                            .hierarchy_state
                            .parent_integrations
                            .get(&issue_id)
                            .and_then(|controller| controller.current_attempt_id())
                            .map(str::to_owned)
                            .ok_or_else(|| SchedulerError::Workspace {
                                detail: "parent launch has no durable attempt intent".to_owned(),
                            })?;
                        self.hierarchy_state
                            .parent_integrations
                            .get_mut(&issue_id)
                            .ok_or_else(|| SchedulerError::Workspace {
                                detail: "parent launch has no durable integration controller"
                                    .to_owned(),
                            })?
                            .attach_attempt_conversation(&attempt_id, &conversation_id)?;
                    }
                    execution = execution.start_running(
                        observed_at,
                        effective_stall_timeout(self.config.stall_timeout_ms),
                        Some(launch.conversation),
                    )?;
                    execution.record_turn_started(observed_at)?;
                    self.worker_metadata.insert(
                        claimed_run.worker_id.clone(),
                        WorkerMetadata::new(
                            issue_id.clone(),
                            Some(start_request.route.harness_kind),
                        )
                        .with_dry_run(start_request.route.dry_run),
                    );
                    let run_start_changed = self
                        .hierarchy_state
                        .run_started_at_by_issue
                        .insert(issue_id.clone(), started_at)
                        .is_none_or(|previous| previous != started_at);
                    self.hierarchy_state
                        .terminal_orchestrator_issues
                        .remove(&issue_id);
                    if !execution.issue().sub_issues.is_empty()
                        && self.hierarchy_state.hierarchy.contains_key(&issue_id)
                    {
                        let nested_child_ids = self
                            .hierarchy_state
                            .hierarchy
                            .get(&issue_id)
                            .map(|snapshot| {
                                snapshot
                                    .required_child_edges
                                    .iter()
                                    .filter(|edge| {
                                        edge.required
                                            && self
                                                .hierarchy_state
                                                .hierarchy
                                                .contains_key(&edge.child_id)
                                    })
                                    .map(|edge| edge.child_id.clone())
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(&issue_id) {
                            snapshot.mark_dispatched();
                        }
                        self.hierarchy_state
                            .release_leaf_leases_for_parent(&issue_id, observed_at.as_u64());
                        self.hierarchy_state.release_ancestor_leases_for_children(
                            &nested_child_ids,
                            observed_at.as_u64(),
                        );
                        if let Err(error) = self.persist_orchestrator_state().await
                            && first_error.is_none()
                        {
                            self.hierarchy_state_dirty = true;
                            first_error = Some(error);
                        }
                    }
                    if run_start_changed
                        && (execution.issue().sub_issues.is_empty()
                            || !self.hierarchy_state.hierarchy.contains_key(&issue_id))
                        && let Err(error) = self.persist_orchestrator_state().await
                        && first_error.is_none()
                    {
                        self.hierarchy_state_dirty = true;
                        first_error = Some(error);
                    }
                    if let Err(error) = self.workspace.clear_retry_pending(&issue_id).await {
                        if first_error.is_none() {
                            first_error = Some(SchedulerError::Workspace {
                                detail: error.to_string(),
                            });
                        }
                    } else {
                        debug!(issue_id = %issue_id, "dispatched scheduler worker");
                    }
                }
                Err(error) => {
                    let detail = error.to_string();
                    warn!(issue_id = %issue_id, error = %detail, "failed to launch scheduler worker");
                    if !execution.issue().sub_issues.is_empty()
                        && let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(&issue_id)
                    {
                        snapshot.clear_dispatch_intent();
                        if let Err(persist_error) = self.persist_orchestrator_state().await
                            && first_error.is_none()
                        {
                            first_error = Some(persist_error);
                        }
                    }
                    let outcome = WorkerOutcomeRecord::from_run(
                        &claimed_run,
                        WorkerOutcomeKind::Failed,
                        observed_at,
                        Some("failed to start worker".to_string()),
                        Some(detail),
                    );
                    execution = self
                        .resolve_finished_execution(execution, outcome, observed_at)
                        .await?;
                }
            }

            self.insert_execution(issue_id.clone(), execution);
            if let Err(error) = self.persist_retry_if_queued(&issue_id).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    async fn record_parent_worker_outcome(
        &mut self,
        issue_id: &IssueId,
        execution: &IssueExecution,
        outcome: &mut WorkerOutcomeRecord,
    ) -> Result<bool, SchedulerError> {
        let previous_state = self.hierarchy_state.clone();
        let merging_continuation = tracker_merging_interrupt_cancelled(execution, outcome);
        let Some(controller) = self.hierarchy_state.parent_integrations.get_mut(issue_id) else {
            return Ok(false);
        };
        let Some(attempt_id) = controller.current_attempt_id().map(str::to_owned) else {
            return Ok(false);
        };
        let launch_never_attached = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .is_some_and(|attempt| attempt.conversation_id.is_none());
        let deadline_reached = controller
            .current_attempt_deadline()
            .is_some_and(|deadline| outcome.finished_at.as_u64() >= deadline.as_u64());
        if execution
            .interrupt()
            .is_some_and(|interrupt| interrupt.status == HarnessInterruptStatus::Acknowledged)
        {
            controller.observe_harness_stopped(
                &attempt_id,
                "harness interrupt reconciled a stopped state",
                outcome.finished_at,
            )?;
        }
        let mut verification_evidence_accepted = false;
        let verification_passed = if deadline_reached {
            controller.append_log(
                &attempt_id,
                "worker outcome arrived at or after the absolute parent attempt deadline",
            )?;
            false
        } else {
            match outcome.parent_verification.as_ref() {
                Some(evidence) => {
                    match controller.record_verification_evidence(&attempt_id, evidence) {
                        Ok(passed) => {
                            verification_evidence_accepted = true;
                            passed
                        }
                        Err(error) => {
                            controller.append_log(
                                &attempt_id,
                                &format!("verification receipt rejected: {error}"),
                            )?;
                            false
                        }
                    }
                }
                None => false,
            }
        };
        enforce_parent_outcome_trust(outcome, deadline_reached, verification_passed);
        if outcome.harness_stopped {
            controller.observe_harness_stopped(
                &attempt_id,
                "harness adapter observed a terminal runtime state",
                outcome.finished_at,
            )?;
        }
        let observed_cleanup = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .and_then(|attempt| attempt.cleanup.clone());
        let harness_stopped = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .is_some_and(|attempt| attempt.harness_stopped_at.is_some());
        let status = if !launch_never_attached && !harness_stopped {
            ParentAttemptStatus::Indeterminate
        } else {
            match outcome.outcome {
                WorkerOutcomeKind::Succeeded if verification_passed => ParentAttemptStatus::Passed,
                WorkerOutcomeKind::Succeeded | WorkerOutcomeKind::Failed => {
                    ParentAttemptStatus::Failed
                }
                WorkerOutcomeKind::TimedOut | WorkerOutcomeKind::Stalled => {
                    ParentAttemptStatus::TimedOut
                }
                WorkerOutcomeKind::Cancelled => ParentAttemptStatus::Canceled,
                WorkerOutcomeKind::Detached | WorkerOutcomeKind::CancelFailed => {
                    ParentAttemptStatus::Indeterminate
                }
            }
        };
        let cleanup = observed_cleanup.unwrap_or_else(|| ParentCleanupReceipt {
            status: if launch_never_attached {
                ParentCleanupStatus::Succeeded
            } else {
                ParentCleanupStatus::Pending
            },
            occurred_at: outcome.finished_at,
            detail: Some(if launch_never_attached {
                "worker launch failed before a parent command could start".to_owned()
            } else {
                "parent command teardown receipt was not available".to_owned()
            }),
        });
        let exit_code = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .and_then(|attempt| attempt.exit_code);
        controller.finish_attempt(&attempt_id, status, exit_code, cleanup, outcome.finished_at)?;
        let repair_implementation_turn = matches!(
            controller.state,
            super::ParentIntegrationState::Fixing { .. }
        ) && controller.repair_attempts.iter().any(|repair| {
            repair.status == ParentRepairStatus::ChangesRequested
                || (repair.status == ParentRepairStatus::Implementing
                    && !repair.implementation_completed)
        });
        let inactive_repair_implementation = repair_implementation_turn
            && execution.issue().state.category != IssueStateCategory::Active;
        if status == ParentAttemptStatus::Passed
            && !inactive_repair_implementation
            && matches!(
                controller.state,
                super::ParentIntegrationState::Fixing { .. }
            )
            && let Some(repair_id) = controller
                .repair_attempts
                .iter()
                .rev()
                .find(|repair| {
                    repair.status == ParentRepairStatus::ChangesRequested
                        || (repair.status == ParentRepairStatus::Implementing
                            && !repair.implementation_completed)
                })
                .map(|repair| repair.id.clone())
        {
            controller.record_repair_implementation_completed(&repair_id)?;
        }
        let input_version = parent_controller_input_version(controller);
        let repair_requested = verification_evidence_accepted
            && status == ParentAttemptStatus::Failed
            && outcome
                .parent_verification
                .as_ref()
                .and_then(|evidence| evidence.repair_repository_id.as_ref())
                .is_some();
        if inactive_repair_implementation {
            controller.cancel_without_harness(
                "tracker became inactive before the repair implementation turn could be published",
                &input_version,
                outcome.finished_at,
            )?;
        } else if status == ParentAttemptStatus::Passed
            && execution.issue().state.category == IssueStateCategory::Terminal
        {
            controller.complete(&attempt_id, &input_version, outcome.finished_at)?;
        } else if status == ParentAttemptStatus::Passed {
            // The harness evidence is final, but only the tracker can prove
            // that the parent reached its terminal workflow state. Preserve
            // this passed attempt so a later successful tracker refresh can
            // finalize it without rerunning or losing its exact receipts.
        } else if merging_continuation {
            controller.prepare_retry(
                &attempt_id,
                "tracker merging superseded human-review polling; refresh before continuing",
                outcome.finished_at,
            )?;
        } else if status == ParentAttemptStatus::Indeterminate {
            controller.prepare_retry(
                &attempt_id,
                "parent harness outcome is indeterminate; retain ownership until terminal state is reconciled",
                outcome.finished_at,
            )?;
        } else if status == ParentAttemptStatus::Canceled
            && (execution.issue().state.category != IssueStateCategory::Active
                || acknowledged_operator_cancel_terminal(execution, outcome))
        {
            controller.cancel(
                outcome
                    .summary
                    .clone()
                    .unwrap_or_else(|| "parent integration harness was canceled".to_owned()),
                &input_version,
                status == ParentAttemptStatus::Canceled,
                outcome.finished_at,
            )?;
        } else if repair_requested {
            // The failed, harness-observed check selected a verified repository
            // for repair. Keep the controller in Integrating so the
            // orchestrator can atomically create the repair attempt below.
        } else if repair_implementation_turn {
            // Keep the durable repair in Fixing. The execution retry policy
            // controls another implementation turn without publishing an
            // incomplete branch.
        } else {
            controller.prepare_retry(
                &attempt_id,
                outcome
                    .error
                    .as_deref()
                    .or(outcome.summary.as_deref())
                    .unwrap_or("parent integration turn finished before terminal tracker state"),
                outcome.finished_at,
            )?;
        }
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_state;
            self.hierarchy_state_dirty = true;
            return Err(error);
        }
        Ok(repair_requested)
    }

    async fn clear_parent_dispatch_intent_after_preparation_failure(
        &mut self,
        issue_id: &IssueId,
        error: SchedulerError,
    ) -> SchedulerError {
        let should_clear = self
            .hierarchy_state
            .hierarchy
            .get(issue_id)
            .is_some_and(HierarchySnapshot::dispatch_intended);
        if !should_clear {
            return error;
        }
        if let Some(snapshot) = self.hierarchy_state.hierarchy.get_mut(issue_id) {
            snapshot.clear_dispatch_intent();
        }
        if let Err(persist_error) = self.persist_orchestrator_state().await {
            warn!(
                issue = %issue_id,
                error = %persist_error,
                "failed to clear parent dispatch intent after launch preparation failure"
            );
            return persist_error;
        }
        error
    }

    async fn apply_worker_updates(
        &mut self,
        updates: Vec<WorkerUpdate>,
    ) -> Result<(), SchedulerError> {
        let mut first_error = None;
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
                    let Some(issue_id) = self
                        .worker_metadata
                        .get(&worker_id)
                        .map(|metadata| metadata.issue_id.clone())
                    else {
                        continue;
                    };
                    let parent_log = format!(
                        "{}: {}{}",
                        event_kind.as_deref().unwrap_or("runtime_event"),
                        summary.as_deref().unwrap_or_default(),
                        payload
                            .as_ref()
                            .map(|payload| format!(" {payload}"))
                            .unwrap_or_default()
                    );
                    let parent_workspace_path = self
                        .executions
                        .get(&issue_id)
                        .and_then(IssueExecution::workspace)
                        .map(|workspace| workspace.path.clone());
                    if let Some(controller) =
                        self.hierarchy_state.parent_integrations.get_mut(&issue_id)
                        && let Some(attempt_id) = controller.current_attempt_id().map(str::to_owned)
                    {
                        if let Err(error) = observe_parent_command_event(
                            controller,
                            &attempt_id,
                            parent_workspace_path.as_deref(),
                            observed_at,
                            event_id.as_deref(),
                            event_kind.as_deref(),
                            payload.as_ref(),
                        ) {
                            controller.append_log(
                                &attempt_id,
                                &format!("runtime command evidence rejected: {error}"),
                            )?;
                        }
                        controller
                            .append_log(&attempt_id, &redact_runtime_diagnostic(&parent_log))?;
                        self.hierarchy_state_dirty = true;
                    }
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
                    let Some(metadata) = self.worker_metadata.remove(&worker_id) else {
                        continue;
                    };
                    let issue_id = metadata.issue_id;
                    let Some(execution) = self.remove_execution(&issue_id) else {
                        continue;
                    };
                    if metadata.dry_run {
                        let execution = execution.release(
                            outcome.finished_at,
                            ReleaseReason::Completed,
                            Some(outcome),
                        )?;
                        self.insert_execution(issue_id, execution);
                        continue;
                    }
                    let finished_at = outcome.finished_at;
                    let original_execution = execution.clone();
                    let finished_outcome = outcome.clone();
                    let execution = match self
                        .resolve_finished_execution(execution, outcome, finished_at)
                        .await
                    {
                        Ok(execution) => execution,
                        Err(error) => {
                            let mut retained_execution = original_execution;
                            retained_execution.retain_worker_outcome(finished_outcome.clone());
                            self.insert_execution(issue_id.clone(), retained_execution.clone());
                            self.pending_finished_updates
                                .insert(issue_id.clone(), (retained_execution, finished_outcome));
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                            continue;
                        }
                    };
                    if let Err(error) = self
                        .rebind_finished_child_hierarchy_generation(&issue_id)
                        .await
                    {
                        // The outcome has already been applied by
                        // resolve_finished_execution. If rebinding its
                        // hierarchy generation cannot be durably persisted,
                        // retry the original running execution so the next
                        // flush applies the outcome exactly once.
                        self.insert_execution(issue_id.clone(), original_execution.clone());
                        self.pending_finished_updates
                            .insert(issue_id.clone(), (original_execution, finished_outcome));
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                        continue;
                    }
                    self.insert_execution(issue_id.clone(), execution);
                    if let Err(error) = self.persist_retry_if_queued(&issue_id).await
                        && first_error.is_none()
                    {
                        first_error = Some(error);
                    }
                }
                WorkerUpdate::ConversationMetadataUpdate {
                    worker_id,
                    conversation,
                } => {
                    let Some(issue_id) = self
                        .worker_metadata
                        .get(&worker_id)
                        .map(|metadata| metadata.issue_id.clone())
                    else {
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
                    let Some(issue_id) = self
                        .worker_metadata
                        .get(&worker_id)
                        .map(|metadata| metadata.issue_id.clone())
                    else {
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

        if self.hierarchy_state_dirty
            && let Err(error) = self.persist_orchestrator_state().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }

        first_error.map_or(Ok(()), Err)
    }

    async fn rebind_finished_child_hierarchy_generation(
        &mut self,
        issue_id: &IssueId,
    ) -> Result<(), SchedulerError> {
        let Some(generation) = self
            .hierarchy_state
            .hierarchy
            .values()
            .filter(|snapshot| {
                snapshot
                    .required_child_edges
                    .iter()
                    .any(|edge| edge.required && edge.child_id == *issue_id)
            })
            .map(|snapshot| snapshot.generation)
            .next()
        else {
            return Ok(());
        };
        if self
            .hierarchy_state
            .run_hierarchy_generations
            .get(issue_id)
            .is_none_or(|current| *current == generation)
        {
            return Ok(());
        }

        let previous_state = self.hierarchy_state.clone();
        self.hierarchy_state
            .run_hierarchy_generations
            .insert(issue_id.clone(), generation);
        self.hierarchy_state
            .rebind_leaf_leases(&BTreeSet::from([issue_id.clone()]), generation);
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_state;
            return Err(error);
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
            .filter_map(|(issue_id, execution)| {
                let absolute_timeout = self
                    .hierarchy_state
                    .parent_integrations
                    .get(issue_id)
                    .and_then(ParentIntegrationController::current_attempt_deadline)
                    .is_some_and(|deadline| deadline <= observed_at);
                match execution.state() {
                    crate::opensymphony_domain::SchedulerState::Running { stall, .. }
                        if stall.stalled_at <= observed_at || absolute_timeout =>
                    {
                        Some((issue_id.clone(), absolute_timeout))
                    }
                    _ => None,
                }
            })
            .collect::<Vec<_>>();

        for (issue_id, absolute_timeout) in stalled {
            let Some(mut execution) = self.remove_execution(&issue_id) else {
                continue;
            };
            let Some(run) = execution.current_run().cloned() else {
                self.insert_execution(issue_id, execution);
                continue;
            };

            let remote_stopped = self
                .abort_worker(
                    &mut execution,
                    &run,
                    WorkerAbortReason::Stalled,
                    observed_at,
                )
                .await?;
            if !remote_stopped {
                // The local worker and execution remain owned by the scheduler;
                // keep the run in Running so the next stall pass retries the
                // same stop request instead of releasing a still-live remote run.
                self.insert_execution(issue_id, execution);
                continue;
            }
            let outcome = WorkerOutcomeRecord::from_run(
                &run,
                if absolute_timeout {
                    WorkerOutcomeKind::TimedOut
                } else if remote_stopped {
                    WorkerOutcomeKind::Stalled
                } else {
                    WorkerOutcomeKind::Detached
                },
                observed_at,
                Some(if absolute_timeout {
                    "parent integration attempt exceeded its absolute timeout".to_owned()
                } else {
                    "worker exceeded the configured stall timeout".to_owned()
                }),
                Some(if absolute_timeout {
                    "parent integration command deadline reached".to_owned()
                } else {
                    "scheduler stall timeout reached".to_owned()
                }),
            );
            execution = self
                .resolve_finished_execution(execution, outcome, observed_at)
                .await?;
            self.insert_execution(issue_id.clone(), execution);
            self.persist_retry_if_queued(&issue_id).await?;
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

        // Do not reopen executions that were released due to terminal worker outcomes.
        // These represent either runs that could not be safely stopped or explicit
        // operator cancels, so reopening would duplicate or restart unwanted work.
        let retry_exhausted_can_reopen = retry_exhausted_release(&execution)
            && execution
                .retry_count_override()
                .is_some_and(|count| !self.retry_limit_reached(count));
        let retry_exhausted_marker_only_reopen =
            retry_exhausted_can_reopen && recovered_workspace.is_none();
        let retry_count_override = execution.retry_count_override();
        let was_terminal_outcome =
            terminal_worker_outcome_prevents_reopen(&execution) && !retry_exhausted_can_reopen;
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
        if retry_exhausted_marker_only_reopen {
            let normal_retry_count = retry_count_override
                .expect("retry-exhausted recovery should record its consumed retry count")
                .saturating_add(1);
            let retry = RetryEntry {
                issue_id: issue.id.clone(),
                identifier: issue.identifier.clone(),
                attempt: RetryAttempt::new(normal_retry_count)?,
                normal_retry_count,
                scheduled_at: observed_at,
                due_at: observed_at,
                reason: RetryReason::Reconciliation,
                error: None,
            };
            execution = execution.restore_retry(retry)?;
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

        if let Err(error) = execution.refresh_issue(issue) {
            self.insert_execution(issue_id, execution);
            return Err(error.into());
        }
        let abort_requested = abort_reason.is_some();
        let mut remote_stopped = true;
        if let Some(run) = execution.current_run().cloned()
            && let Some(abort_reason) = abort_reason
        {
            remote_stopped = match self
                .abort_worker(&mut execution, &run, abort_reason, observed_at)
                .await
            {
                Ok(remote_stopped) => remote_stopped,
                Err(error) => {
                    self.insert_execution(issue_id, execution);
                    return Err(error);
                }
            };
        }
        if abort_requested && !remote_stopped {
            warn!(
                issue = %issue_id,
                "retaining execution because the harness did not acknowledge its stop request"
            );
            self.insert_execution(issue_id, execution);
            return Ok(());
        }
        let previous_parent_state = self.hierarchy_state.clone();
        let parent_update = (|| -> Result<bool, SchedulerError> {
            let Some(controller) = self.hierarchy_state.parent_integrations.get_mut(&issue_id)
            else {
                return Ok(false);
            };
            if execution.issue().state.category == IssueStateCategory::Terminal
                && !controller.state.terminal()
                && matches!(controller.state, super::ParentIntegrationState::Integrating)
                && controller.current_attempt_id().is_none()
                && let Some(attempt_id) = controller
                    .attempts
                    .iter()
                    .rev()
                    .find(|attempt| attempt.status == ParentAttemptStatus::Passed)
                    .map(|attempt| attempt.id.clone())
            {
                let input_version = parent_controller_input_version(controller);
                match controller.complete(&attempt_id, &input_version, observed_at) {
                    Ok(()) => return Ok(true),
                    Err(ParentIntegrationError::FinalVerificationIncomplete) => {
                        // A legacy or malformed Passed marker is insufficient
                        // to release the parent. Keep its controller and
                        // leases intact for operator-visible recovery.
                        return Ok(false);
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            if controller.state.terminal() {
                return Ok(false);
            }
            let Some(attempt_id) = controller.current_attempt_id().map(str::to_owned) else {
                if !controller.can_cancel_without_harness() {
                    // An indeterminate attempt or incomplete teardown can still
                    // own a live harness. Retain the controller and its leases
                    // for stop reconciliation or operator recovery.
                    return Ok(false);
                }
                // Provider-side repair stages deliberately have no live
                // harness turn. A terminal tracker transition still owns the
                // controller lifecycle, so finish it durably without issuing
                // a fictitious worker interrupt.
                let input_version = parent_controller_input_version(controller);
                controller.cancel_without_harness(
                    format!("scheduler released parent integration: {reason:?}"),
                    &input_version,
                    observed_at,
                )?;
                return Ok(true);
            };
            if execution
                .interrupt()
                .is_some_and(|interrupt| interrupt.status == HarnessInterruptStatus::Acknowledged)
            {
                controller.observe_harness_stopped(
                    &attempt_id,
                    "harness interrupt reconciled a stopped state",
                    observed_at,
                )?;
            }
            let observed_cleanup = controller
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .and_then(|attempt| attempt.cleanup.clone());
            let observed_exit_code = controller
                .attempts
                .iter()
                .find(|attempt| attempt.id == attempt_id)
                .and_then(|attempt| attempt.exit_code);
            controller.finish_attempt(
                &attempt_id,
                ParentAttemptStatus::Canceled,
                observed_exit_code,
                observed_cleanup.unwrap_or_else(|| ParentCleanupReceipt {
                    status: ParentCleanupStatus::Pending,
                    occurred_at: observed_at,
                    detail: Some(format!(
                        "scheduler release acknowledged harness stop but has no command teardown receipt: {reason:?}"
                    )),
                }),
                observed_at,
            )?;
            let input_version = parent_controller_input_version(controller);
            controller.cancel(
                format!("scheduler released parent integration: {reason:?}"),
                &input_version,
                true,
                observed_at,
            )?;
            Ok(true)
        })();
        let parent_changed = match parent_update {
            Ok(changed) => changed,
            Err(error) => {
                self.hierarchy_state = previous_parent_state;
                self.insert_execution(issue_id, execution);
                return Err(error);
            }
        };
        if parent_changed && let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_parent_state;
            self.hierarchy_state_dirty = true;
            self.insert_execution(issue_id, execution);
            return Err(error);
        }
        self.workspace
            .revoke_issue_resources(execution.issue().identifier.as_str());
        let was_retry_exhausted = retry_exhausted_release(&execution);
        let retain_failed = was_retry_exhausted && self.workspace.retain_failed_workspaces();
        if execution.status() == SchedulerStatus::Released
            && reason == ReleaseReason::TrackerTerminal
            && !was_retry_exhausted
        {
            execution = execution.replace_release_reason(observed_at, reason)?;
        } else if execution.status() != SchedulerStatus::Released {
            execution = execution.release(
                observed_at,
                if was_retry_exhausted {
                    ReleaseReason::RetryExhausted
                } else {
                    reason
                },
                None,
            )?;
        }
        // A retry-exhausted release can be reconciled after the tracker moves
        // to a terminal state. Preserve its failed-cleanup policy even though
        // the externally visible release reason becomes TrackerTerminal.
        let cleanup_reason = if was_retry_exhausted {
            ReleaseReason::RetryExhausted
        } else {
            reason
        };
        let successful_terminal_outcome = reason == ReleaseReason::Completed
            || (reason == ReleaseReason::TrackerTerminal
                && execution
                    .last_worker_outcome()
                    .is_some_and(|outcome| outcome.outcome == WorkerOutcomeKind::Succeeded));
        let parent_finalized =
            self.parent_finalized_success(&issue_id, !execution.issue().sub_issues.is_empty());
        if successful_terminal_outcome && parent_finalized {
            self.record_terminal_orchestrator_success(&issue_id).await?;
        }
        if cleanup_terminal
            && parent_finalized
            && let Err(error) = self.retain_terminal_child_lease(&execution).await
        {
            self.insert_execution(issue_id, execution);
            return Err(error);
        }
        let parent_workspace = self
            .is_parent_integration_workspace(&issue_id, !execution.issue().sub_issues.is_empty());
        let mut retry_cleanup_succeeded = retain_failed;
        if !parent_workspace
            && cleanup_terminal
            && parent_finalized
            && remote_stopped
            && !retain_failed
            && let Some(workspace) = execution.workspace().cloned()
            && !self.workspace_has_active_lease(&workspace).await?
        {
            match if cleanup_reason == ReleaseReason::RetryExhausted {
                self.workspace.cleanup_failed_workspace(&workspace).await
            } else {
                self.workspace.cleanup_workspace(&workspace, true).await
            } {
                Ok(()) => {
                    retry_cleanup_succeeded = true;
                    execution.clear_workspace();
                }
                Err(error) => {
                    tracing::warn!(
                        issue = %issue_id,
                        %error,
                        "retaining released execution while terminal workspace cleanup retries"
                    );
                }
            }
        } else if was_retry_exhausted && (!cleanup_terminal || execution.workspace().is_none()) {
            retry_cleanup_succeeded = true;
        }
        if was_retry_exhausted && retry_cleanup_succeeded {
            if let Err(error) = self
                .workspace
                .clear_retry_exhaustion(execution.issue().identifier.as_str())
                .await
            {
                self.insert_execution(issue_id, execution);
                return Err(SchedulerError::Workspace {
                    detail: error.to_string(),
                });
            }
            execution = execution.replace_release_reason(observed_at, reason)?;
        }
        self.insert_execution(issue_id, execution);
        Ok(())
    }

    async fn abort_worker(
        &mut self,
        execution: &mut IssueExecution,
        run: &RunAttempt,
        reason: WorkerAbortReason,
        observed_at: TimestampMs,
    ) -> Result<bool, SchedulerError> {
        let harness_kind = self
            .worker_metadata
            .get(&run.worker_id)
            .and_then(|metadata| metadata.harness_kind.clone())
            .unwrap_or_else(|| "<unknown>".to_string());
        let mut remote_stopped = execution.conversation().is_none();
        if execution.conversation().is_some() {
            let (command, queued) = execution.request_interrupt(
                harness_kind,
                None,
                HarnessInterruptReason::SchedulerAbort,
                HarnessInterruptExpectedNextState::Released,
                observed_at,
            )?;
            self.persist_interrupt_intent(execution).await?;
            if queued {
                match self.worker.interrupt_worker(command).await {
                    Ok(acknowledgement) if acknowledgement.timed_out => {
                        execution.timeout_interrupt(
                            observed_at,
                            acknowledgement.detail.unwrap_or_else(|| {
                                "worker interrupt acknowledgement timed out".to_string()
                            }),
                        )?;
                    }
                    Ok(acknowledgement) if acknowledgement.accepted => {
                        execution.acknowledge_interrupt(observed_at)?;
                        remote_stopped = true;
                    }
                    Ok(acknowledgement) => {
                        execution.fail_interrupt(
                            observed_at,
                            acknowledgement.detail.unwrap_or_else(|| {
                                "worker interrupt request was not accepted".to_string()
                            }),
                        )?;
                    }
                    Err(error) => {
                        execution.fail_interrupt(observed_at, error.to_string())?;
                    }
                }
            } else {
                remote_stopped = execution.interrupt().is_some_and(|interrupt| {
                    interrupt.status == HarnessInterruptStatus::Acknowledged
                });
            }
        }
        if !remote_stopped {
            // Keep the local worker metadata/task until the harness confirms
            // that it stopped. The next reconciliation can retry the same
            // interrupt instead of leaving a remote run alive after its local
            // task was discarded.
            return Ok(false);
        }
        self.worker
            .abort_worker(&run.worker_id, reason)
            .await
            .map_err(|error| SchedulerError::Worker {
                detail: error.to_string(),
            })?;
        self.worker_metadata.remove(&run.worker_id);
        Ok(remote_stopped)
    }

    async fn resolve_finished_execution(
        &mut self,
        mut execution: IssueExecution,
        mut outcome: WorkerOutcomeRecord,
        observed_at: TimestampMs,
    ) -> Result<IssueExecution, SchedulerError> {
        let issue_id = execution.issue().id.clone();
        if let Some(reason) = non_active_release_reason(execution.issue().state.category.clone()) {
            self.record_parent_worker_outcome(&issue_id, &execution, &mut outcome)
                .await?;
            if self
                .hierarchy_state
                .parent_integrations
                .get(&issue_id)
                .is_some_and(ParentIntegrationController::has_unreconciled_harness)
            {
                return self
                    .queue_retry_for_outcome(execution, outcome, observed_at)
                    .await;
            }
            return self
                .release_finished_execution(execution, observed_at, reason, Some(outcome))
                .await;
        }

        // Detached and CancelFailed are terminal outcomes: release the execution instead of
        // queuing a retry. Operator cancels are also terminal from the scheduler's perspective
        // for completed or cancelled outcomes, even when the worker outcome races a failed or
        // timed-out acknowledgement, because retrying would restart work the operator stopped.
        if matches!(
            outcome.outcome,
            WorkerOutcomeKind::Detached | WorkerOutcomeKind::CancelFailed
        ) {
            self.record_parent_worker_outcome(&issue_id, &execution, &mut outcome)
                .await?;
            if self
                .hierarchy_state
                .parent_integrations
                .get(&issue_id)
                .is_some_and(ParentIntegrationController::has_unreconciled_harness)
            {
                return self
                    .queue_retry_for_outcome(execution, outcome, observed_at)
                    .await;
            }
            return self
                .release_finished_execution(
                    execution,
                    observed_at,
                    ReleaseReason::TrackerInactive,
                    Some(outcome),
                )
                .await;
        }
        if acknowledged_operator_cancel_terminal(&execution, &outcome) {
            self.record_parent_worker_outcome(&issue_id, &execution, &mut outcome)
                .await?;
            return self
                .release_finished_execution(
                    execution,
                    observed_at,
                    ReleaseReason::Cancelled,
                    Some(outcome),
                )
                .await;
        }

        if let Some(state) = self
            .refresh_finished_issue_state(&issue_id, observed_at)
            .await
        {
            let mut issue = execution.issue().clone();
            issue.state = state;
            execution.refresh_issue(issue)?;
            if let Some(reason) =
                non_active_release_reason(execution.issue().state.category.clone())
            {
                self.record_parent_worker_outcome(&issue_id, &execution, &mut outcome)
                    .await?;
                if self
                    .hierarchy_state
                    .parent_integrations
                    .get(&issue_id)
                    .is_some_and(ParentIntegrationController::has_unreconciled_harness)
                {
                    return self
                        .queue_retry_for_outcome(execution, outcome, observed_at)
                        .await;
                }
                return self
                    .release_finished_execution(execution, observed_at, reason, Some(outcome))
                    .await;
            }
        }

        let repair_request_accepted = self
            .record_parent_worker_outcome(&issue_id, &execution, &mut outcome)
            .await?;
        let repair_implementation_ready = self
            .hierarchy_state
            .parent_integrations
            .get(&issue_id)
            .is_some_and(|controller| {
                matches!(
                    controller.state,
                    super::ParentIntegrationState::Fixing { .. }
                ) && controller.repair_attempts.iter().any(|repair| {
                    repair.status == ParentRepairStatus::Implementing
                        && repair.implementation_completed
                })
            });
        if repair_implementation_ready {
            return self
                .release_finished_execution(
                    execution,
                    observed_at,
                    ReleaseReason::Completed,
                    Some(outcome),
                )
                .await;
        }
        let repair_repository_id = repair_request_accepted
            .then(|| {
                outcome
                    .parent_verification
                    .as_ref()
                    .and_then(|evidence| evidence.repair_repository_id.clone())
            })
            .flatten();
        if let Some(repository_id) = repair_repository_id {
            let evidence = outcome
                .parent_verification
                .as_ref()
                .expect("repair repository was selected from verification evidence");
            let defect_key = format!(
                "parent-repair:{}:{}:{}:{}",
                issue_id, evidence.run_id, evidence.attempt, repository_id
            );
            self.insert_execution(issue_id.clone(), execution.clone());
            let repair_result = self
                .begin_parent_repair(&issue_id, &repository_id, &defect_key, outcome.finished_at)
                .await;
            let execution = self
                .remove_execution(&issue_id)
                .expect("parent execution was temporarily restored");
            repair_result?;
            return self
                .release_finished_execution(
                    execution,
                    observed_at,
                    ReleaseReason::Completed,
                    Some(outcome),
                )
                .await;
        }
        if self.parent_waiting_for_tracker_confirmation(&issue_id) {
            return self
                .release_finished_execution(
                    execution,
                    observed_at,
                    ReleaseReason::Completed,
                    Some(outcome),
                )
                .await;
        }

        if self
            .hierarchy_state
            .parent_integrations
            .get(&issue_id)
            .is_some_and(ParentIntegrationController::has_unreconciled_harness)
        {
            return self
                .queue_retry_for_outcome(execution, outcome, observed_at)
                .await;
        }

        let retry_count = execution
            .current_run()
            .map(|run| run.normal_retry_count)
            .unwrap_or_default();
        if self
            .config
            .max_retry_attempts
            .is_some_and(|max_attempts| retry_count >= max_attempts)
        {
            // The tracker refresh above is the only authority that can prove
            // the issue is no longer active. If it still appears active (or
            // the refresh failed), park the exhausted run rather than making
            // a successful worker turn look like a completed Linear task.
            let reason = ReleaseReason::RetryExhausted;
            return self
                .release_finished_execution(execution, observed_at, reason, Some(outcome))
                .await;
        }

        self.queue_retry_for_outcome(execution, outcome, observed_at)
            .await
    }

    async fn refresh_finished_issue_state(
        &mut self,
        issue_id: &IssueId,
        observed_at: TimestampMs,
    ) -> Option<IssueState> {
        let issue_ids = vec![issue_id.as_str().to_string()];
        let snapshots = match self.tracker.issue_states_by_ids(&issue_ids).await {
            Ok(snapshots) => snapshots,
            Err(error) => {
                self.set_linear_cooldown_from_tracker_error(&error, observed_at);
                warn!(
                    issue_id = %issue_id,
                    %error,
                    "failed to refresh tracker state after worker finished; falling back to retry policy"
                );
                return None;
            }
        };

        snapshots
            .into_iter()
            .next()
            .map(|snapshot| issue_state_from_name(&snapshot.state.name, &self.config))
    }

    async fn release_finished_execution(
        &mut self,
        execution: IssueExecution,
        observed_at: TimestampMs,
        reason: ReleaseReason,
        outcome: Option<WorkerOutcomeRecord>,
    ) -> Result<IssueExecution, SchedulerError> {
        self.workspace
            .revoke_issue_resources(execution.issue().identifier.as_str());
        let cleanup_terminal = matches!(
            reason,
            ReleaseReason::TrackerTerminal | ReleaseReason::RetryExhausted
        );
        let parent_finalized = self.parent_finalized_success(
            &execution.issue().id,
            !execution.issue().sub_issues.is_empty(),
        );
        let parent_workspace = self.is_parent_integration_workspace(
            &execution.issue().id,
            !execution.issue().sub_issues.is_empty(),
        );
        if cleanup_terminal && parent_finalized {
            self.retain_terminal_child_lease(&execution).await?;
        }
        let successful_terminal_outcome = reason == ReleaseReason::Completed
            || (reason == ReleaseReason::TrackerTerminal
                && outcome
                    .as_ref()
                    .is_some_and(|outcome| outcome.outcome == WorkerOutcomeKind::Succeeded));
        if successful_terminal_outcome && parent_finalized {
            self.record_terminal_orchestrator_success(&execution.issue().id)
                .await?;
        }
        if reason == ReleaseReason::RetryExhausted {
            let normal_retry_count = execution
                .current_run()
                .map(|run| run.normal_retry_count)
                .unwrap_or_default();
            let persisted = self
                .persist_retry_exhaustion(execution.issue(), normal_retry_count)
                .await?;
            // A retry-exhausted execution may have been reopened after the
            // configured limit increased. Keep the durable override aligned
            // with the retry that just exhausted the new budget so the next
            // reconciliation cannot reopen it repeatedly.
            let mut execution = execution.release(observed_at, reason, outcome)?;
            execution.set_retry_count_override(normal_retry_count);
            let retain_failed = self.workspace.retain_failed_workspaces() || !persisted;
            if cleanup_terminal
                && parent_finalized
                && !parent_workspace
                && !retain_failed
                && let Some(workspace) = execution.workspace().cloned()
                && !self.workspace_has_active_lease(&workspace).await?
            {
                let cleanup = self.workspace.cleanup_failed_workspace(&workspace).await;
                match cleanup {
                    Ok(()) => execution.clear_workspace(),
                    Err(error) => {
                        tracing::warn!(
                            issue = %execution.issue().id,
                            %error,
                            "retaining released execution while terminal workspace cleanup retries"
                        );
                    }
                }
            }
            return Ok(execution);
        }
        let mut execution = execution.release(observed_at, reason, outcome)?;
        let retain_failed =
            reason == ReleaseReason::RetryExhausted && self.workspace.retain_failed_workspaces();
        if cleanup_terminal
            && parent_finalized
            && !parent_workspace
            && !retain_failed
            && let Some(workspace) = execution.workspace().cloned()
            && !self.workspace_has_active_lease(&workspace).await?
        {
            let cleanup = if reason == ReleaseReason::RetryExhausted {
                self.workspace.cleanup_failed_workspace(&workspace).await
            } else {
                self.workspace.cleanup_workspace(&workspace, true).await
            };
            match cleanup {
                Ok(()) => execution.clear_workspace(),
                Err(error) => {
                    tracing::warn!(
                        issue = %execution.issue().id,
                        %error,
                        "retaining released execution while terminal workspace cleanup retries"
                    );
                }
            }
        }
        Ok(execution)
    }

    async fn record_terminal_orchestrator_success(
        &mut self,
        issue_id: &IssueId,
    ) -> Result<(), SchedulerError> {
        if self
            .hierarchy_state
            .terminal_orchestrator_issues
            .insert(issue_id.clone())
            && let Err(error) = self.persist_orchestrator_state().await
        {
            self.hierarchy_state
                .terminal_orchestrator_issues
                .remove(issue_id);
            return Err(error);
        }
        Ok(())
    }

    fn parent_finalized_success(&self, issue_id: &IssueId, issue_has_children: bool) -> bool {
        match self.hierarchy_state.parent_integrations.get(issue_id) {
            Some(controller) => {
                controller.state == super::ParentIntegrationState::Completed
                    && self
                        .hierarchy_state
                        .hierarchy
                        .get(issue_id)
                        .is_some_and(|snapshot| {
                            snapshot.accepts_event(controller.hierarchy_generation)
                        })
            }
            None => !issue_has_children,
        }
    }

    // Parent capture retries after daemon restart from the durable runtime
    // envelope, so cleanup remains deferred to the capture-aware lifecycle.
    fn is_parent_integration_workspace(
        &self,
        issue_id: &IssueId,
        issue_has_children: bool,
    ) -> bool {
        issue_has_children
            || self
                .hierarchy_state
                .parent_integrations
                .contains_key(issue_id)
    }

    fn parent_waiting_for_tracker_confirmation(&self, issue_id: &IssueId) -> bool {
        self.hierarchy_state
            .parent_integrations
            .get(issue_id)
            .is_some_and(|controller| {
                !controller.state.terminal()
                    && controller.current_attempt_id().is_none()
                    && controller
                        .attempts
                        .last()
                        .is_some_and(|attempt| attempt.status == ParentAttemptStatus::Passed)
            })
    }

    async fn persist_retry_exhaustion(
        &mut self,
        issue: &NormalizedIssue,
        normal_retry_count: u32,
    ) -> Result<bool, SchedulerError> {
        let record = RetryExhaustionRecord {
            issue: issue.clone(),
            normal_retry_count,
        };
        if let Err(error) = self
            .workspace
            .persist_retry_exhaustion(issue, normal_retry_count)
            .await
        {
            warn!(
                issue_id = %issue.id,
                %error,
                "deferring retry exhaustion persistence"
            );
            self.pending_retry_exhaustion_persistence
                .insert(issue.id.clone(), record);
            return Ok(false);
        }
        Ok(true)
    }

    async fn queue_retry_for_outcome(
        &mut self,
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

    async fn flush_pending_retry_persistence(&mut self) -> Result<(), SchedulerError> {
        let pending = std::mem::take(&mut self.pending_retry_persistence);
        let mut first_error = None;
        for (issue_id, retry) in pending {
            let workspace = self
                .executions
                .get(&issue_id)
                .and_then(|execution| execution.workspace().cloned());
            let Some(workspace) = workspace else {
                continue;
            };
            if let Err(error) = self
                .workspace
                .persist_retry_pending(&workspace, &retry)
                .await
            {
                self.pending_retry_persistence.insert(issue_id, retry);
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
            } else if let Err(error) = self.workspace.clear_retry_pending(&issue_id).await {
                self.pending_retry_persistence.insert(issue_id, retry);
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
            }
        }
        first_error.map_or(Ok(()), |detail| Err(SchedulerError::Workspace { detail }))
    }

    async fn flush_pending_finished_updates(&mut self) -> Result<(), SchedulerError> {
        let pending = std::mem::take(&mut self.pending_finished_updates);
        let mut first_error = None;
        for (issue_id, (execution, outcome)) in pending {
            let finished_at = outcome.finished_at;
            let retry_execution = execution.clone();
            match self
                .resolve_finished_execution(execution, outcome.clone(), finished_at)
                .await
            {
                Ok(execution) => {
                    if let Err(error) = self
                        .rebind_finished_child_hierarchy_generation(&issue_id)
                        .await
                    {
                        self.insert_execution(issue_id.clone(), retry_execution.clone());
                        self.pending_finished_updates
                            .insert(issue_id.clone(), (retry_execution, outcome));
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                        continue;
                    }
                    self.insert_execution(issue_id.clone(), execution);
                    if let Err(error) = self.persist_retry_if_queued(&issue_id).await
                        && first_error.is_none()
                    {
                        first_error = Some(error);
                    }
                }
                Err(error) => {
                    self.insert_execution(issue_id.clone(), retry_execution.clone());
                    self.pending_finished_updates
                        .insert(issue_id.clone(), (retry_execution, outcome));
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    async fn parent_dispatch_is_eligible(
        &mut self,
        tracker_issue: &TrackerIssue,
        normalized: &NormalizedIssue,
        observed_at: TimestampMs,
        allow_retry: bool,
        reachable_child_edges: Option<&BTreeSet<(IssueId, IssueId)>>,
    ) -> Result<bool, SchedulerError> {
        if self
            .hierarchy_state
            .parent_integrations
            .get(&normalized.id)
            .is_some_and(ParentIntegrationController::has_unreconciled_harness)
        {
            return Ok(false);
        }
        if allow_retry
            && self
                .hierarchy_state
                .parent_integrations
                .get(&normalized.id)
                .is_some_and(|controller| {
                    matches!(
                        controller.state,
                        super::ParentIntegrationState::Fixing { .. }
                    ) && controller
                        .repair_attempts
                        .iter()
                        .any(|repair| repair.status == ParentRepairStatus::ChangesRequested)
                })
        {
            return Ok(true);
        }
        if self.linear_cooldown_active(observed_at) {
            return Ok(false);
        }
        let Some(snapshot) = self.hierarchy_state.hierarchy.get(&normalized.id).cloned() else {
            return Ok(false);
        };
        if snapshot.blocked_reason.is_some() {
            return Ok(false);
        }
        // Summary discovery and known-candidate reconciliation do not carry
        // the complete tracker edge set. Keep parents deferred until a full
        // observation can release canceled-subtree evidence safely.
        if reachable_child_edges.is_none() {
            return Ok(false);
        }
        if snapshot.dispatch_claimed() && !allow_retry {
            return Ok(allow_retry);
        }
        if snapshot.dispatch_intended() && !allow_retry {
            return Ok(false);
        }
        if !allow_retry
            && self
                .parent_eligibility_checked_at
                .get(&normalized.id)
                .is_some_and(|checked_at| {
                    checked_at
                        .as_u64()
                        .saturating_add(DISPATCH_DISCOVERY_INTERVAL_MS)
                        > observed_at.as_u64()
                })
        {
            return Ok(false);
        }
        self.parent_eligibility_checked_at
            .insert(normalized.id.clone(), observed_at);
        let eligibility_timeout =
            parent_eligibility_timeout(self.parent_eligibility_work_units(&normalized.id));
        let mut evidence = match timeout(
            eligibility_timeout,
            self.tracker.parent_eligibility(tracker_issue, &snapshot),
        )
        .await
        {
            Err(_) => {
                warn!(
                    parent = %normalized.identifier,
                    timeout = ?eligibility_timeout,
                    "parent eligibility provider lookup timed out; keeping parent blocked"
                );
                self.record_parent_eligibility_block(
                    &normalized.id,
                    HierarchyBlockedReason::UnresolvedFailure(
                        "parent eligibility provider lookup timed out".to_owned(),
                    ),
                )
                .await?;
                return Ok(false);
            }
            Ok(Ok(evidence)) => evidence,
            Ok(Err(error)) => {
                self.set_linear_cooldown_from_tracker_error(&error, observed_at);
                warn!(
                    parent = %normalized.identifier,
                    error = %error,
                    "parent eligibility provider lookup failed; keeping parent blocked"
                );
                self.record_parent_eligibility_block(
                    &normalized.id,
                    HierarchyBlockedReason::UnresolvedFailure(error.to_string()),
                )
                .await?;
                return Ok(false);
            }
        };
        let mut released_canceled_subtree_holds = false;
        for child in &mut evidence.children {
            // A current execution supersedes an older durable success receipt.
            // Provider terminal flags never authorize parent admission.
            child.orchestrator_terminal = self.executions.get(&child.child_id).map_or_else(
                || {
                    self.hierarchy_state
                        .terminal_orchestrator_issues
                        .contains(&child.child_id)
                },
                |execution| {
                    matches!(
                        execution.state(),
                        crate::opensymphony_orchestrator::SchedulerState::Released {
                            reason: ReleaseReason::TrackerTerminal | ReleaseReason::Completed,
                            ..
                        }
                    ) && self.parent_finalized_success(
                        &child.child_id,
                        !execution.issue().sub_issues.is_empty(),
                    )
                },
            );
            if !child.merge_required {
                if !child.orchestrator_terminal {
                    child.unresolved_failure =
                        Some("child orchestrator outcome is not durably terminal".to_owned());
                    continue;
                }
                if let Some(reachable_child_edges) = reachable_child_edges {
                    released_canceled_subtree_holds |= self
                        .hierarchy_state
                        .release_subtree_evidence_for_undispatched_parent_with_reachability(
                            &child.child_id,
                            Some(reachable_child_edges),
                            observed_at.as_u64(),
                        );
                }
                child.resource = None;
                child.resources.clear();
                continue;
            }
            if self.terminal_child_failure_ids.contains(&child.child_id)
                || self
                    .executions
                    .get(&child.child_id)
                    .is_some_and(|execution| {
                        execution.retry().is_some()
                            || execution.last_worker_outcome().is_some_and(|outcome| {
                                matches!(
                                    outcome.outcome,
                                    WorkerOutcomeKind::Failed
                                        | WorkerOutcomeKind::TimedOut
                                        | WorkerOutcomeKind::Stalled
                                        | WorkerOutcomeKind::Detached
                                        | WorkerOutcomeKind::CancelFailed
                                )
                            })
                    })
            {
                child.orchestrator_terminal = false;
                child.unresolved_failure =
                    Some("child has an unresolved worker failure or retry".to_owned());
                continue;
            }
            let direct_provider_evidence_is_stale =
                child.provider_evidence_at.is_some_and(|evidence_at| {
                    !self.hierarchy_state.hierarchy.contains_key(&child.child_id)
                        && self.child_run_started_after(&child.child_id, evidence_at)
                });
            let descendant_provider_evidence_is_stale =
                child.provider_evidence_by_issue.iter().any(|boundary| {
                    self.child_run_started_after(&boundary.issue_id, boundary.evidence_at)
                });
            if child.provider_merge_confirmed
                && (direct_provider_evidence_is_stale || descendant_provider_evidence_is_stale)
            {
                child.orchestrator_terminal = false;
                child.unresolved_failure =
                    Some("provider merge evidence predates the current child run".to_owned());
                continue;
            }
            if child.resource.is_none() {
                let descendant_resources = self
                    .hierarchy_state
                    .descendant_resources_for(&child.child_id);
                if !descendant_resources.is_empty() {
                    child.resource = descendant_resources.first().cloned();
                    child.resources = descendant_resources;
                } else {
                    let retry_resources = self
                        .hierarchy_state
                        .ancestor_resources_for_child(&normalized.id, &child.child_id);
                    let expected_owner = super::LeaseOwner::leaf_worker(&child.child_id);
                    let leaf_resource = self
                        .hierarchy_state
                        .leases
                        .iter()
                        .find(|lease| {
                            lease.active()
                                && lease.kind == super::LeaseKind::LeafWorker
                                && lease.hierarchy_generation == snapshot.generation
                                && lease.owner == expected_owner
                                && lease.resource.issue_id == child.child_id
                        })
                        .map(|lease| lease.resource.clone());
                    if retry_resources.is_empty() {
                        child.resource = leaf_resource;
                    } else {
                        let mut resources = retry_resources;
                        if let Some(leaf_resource) = leaf_resource
                            && !resources.contains(&leaf_resource)
                        {
                            resources.push(leaf_resource);
                        }
                        child.resource = resources.first().cloned();
                        child.resources = resources;
                    }
                }
            }
            if child.resources.is_empty() {
                child.resources = child.resource.clone().into_iter().collect();
            }
        }
        if released_canceled_subtree_holds {
            self.persist_orchestrator_state().await?;
        }
        if let Err(reason) = evidence.eligible_for(&snapshot) {
            self.record_parent_eligibility_block(&normalized.id, reason)
                .await?;
            return Ok(false);
        }
        if evidence.children.iter().any(|child| {
            child.resources().into_iter().any(|resource| {
                if self.hierarchy_state.leases.iter().any(|lease| {
                    lease.active()
                        && lease.kind == super::LeaseKind::AncestorIntegration
                        && lease.owner == super::LeaseOwner::ancestor(&normalized.id)
                        && lease.hierarchy_generation == snapshot.generation
                        && lease.resource == *resource
                }) {
                    return false;
                }
                if self.hierarchy_state.hierarchy.contains_key(&child.child_id) {
                    return !self.hierarchy_state.leases.iter().any(|lease| {
                        lease.active()
                            && lease.kind == super::LeaseKind::AncestorIntegration
                            && lease.owner == super::LeaseOwner::ancestor(&child.child_id)
                            && lease.resource == *resource
                    });
                }
                let expected_owner = super::LeaseOwner::leaf_worker(&resource.issue_id);
                !self.hierarchy_state.leases.iter().any(|lease| {
                    lease.active()
                        && lease.kind == super::LeaseKind::LeafWorker
                        && lease.hierarchy_generation == snapshot.generation
                        && lease.owner == expected_owner
                        && lease.resource == *resource
                })
            })
        }) {
            self.record_parent_eligibility_block(
                &normalized.id,
                HierarchyBlockedReason::MissingCheckoutEvidence,
            )
            .await?;
            return Ok(false);
        }

        self.clear_parent_eligibility_block(&normalized.id).await?;

        let mut next_state = self.hierarchy_state.clone();
        let Some(next_snapshot) = next_state.hierarchy.get_mut(&normalized.id) else {
            return Ok(false);
        };
        let mut required_merge_commits = BTreeSet::new();
        for commit in evidence.children.iter().flat_map(|child| {
            if child.merge_result_commits_by_repository.is_empty() {
                child
                    .merge_result_commit
                    .iter()
                    .chain(child.merge_result_commits.iter())
                    .map(|commit| super::RequiredMergeCommit {
                        repository_id: child.merge_repository_id.clone(),
                        commit: commit.clone(),
                    })
                    .collect::<Vec<_>>()
            } else {
                child.merge_result_commits_by_repository.clone()
            }
        }) {
            if !commit.commit.trim().is_empty() {
                required_merge_commits.insert(commit);
            }
        }
        next_snapshot.dispatch_required_merge_commits =
            required_merge_commits.into_iter().collect();
        if next_snapshot.freeze().is_err() {
            return Ok(false);
        }
        next_snapshot.mark_dispatch_intent();
        let input_version = parent_input_version(next_snapshot);
        let mut required_leases = evidence.integration_leases(&normalized.id, observed_at.as_u64());
        required_leases.extend(evidence.children.iter().flat_map(|child| {
            child
                .resources()
                .into_iter()
                .cloned()
                .map(|resource| LeaseRecord {
                    kind: super::LeaseKind::Review,
                    resource,
                    owner: super::LeaseOwner::review_for_parent(&normalized.id, &child.child_id),
                    hierarchy_generation: evidence.hierarchy_generation,
                    acquired_at: observed_at.as_u64(),
                    expires_at: None,
                    released_at: None,
                })
        }));
        next_state
            .acquire_leases(required_leases)
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?;
        let controller = next_state
            .parent_integrations
            .entry(normalized.id.clone())
            .or_insert(ParentIntegrationController::new(
                normalized.id.clone(),
                snapshot.generation,
            )?);
        let controller_replaced = controller.hierarchy_generation != snapshot.generation;
        if controller_replaced {
            *controller =
                ParentIntegrationController::new(normalized.id.clone(), snapshot.generation)?;
        }
        controller.admit(&input_version, observed_at)?;
        let previous_state = std::mem::replace(&mut self.hierarchy_state, next_state);
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_state;
            return Err(error);
        }
        if controller_replaced {
            self.reopen_completed_parent_for_new_controller(normalized, observed_at)?;
        }
        Ok(true)
    }

    fn child_run_started_after(&self, issue_id: &IssueId, evidence_at: TimestampMs) -> bool {
        self.hierarchy_state
            .run_started_at_by_issue
            .get(issue_id)
            .copied()
            .or_else(|| {
                self.executions
                    .get(issue_id)
                    .and_then(|execution| execution.last_worker_outcome())
                    .map(|outcome| outcome.started_at)
            })
            .is_some_and(|started_at| evidence_at < started_at)
    }

    async fn record_parent_eligibility_block(
        &mut self,
        parent_id: &IssueId,
        reason: HierarchyBlockedReason,
    ) -> Result<(), SchedulerError> {
        let changed = self
            .hierarchy_state
            .hierarchy
            .get_mut(parent_id)
            .is_some_and(|snapshot| {
                if snapshot.blocked_reason.is_some()
                    || snapshot.eligibility_blocked_reason.as_ref() == Some(&reason)
                {
                    false
                } else {
                    snapshot.eligibility_blocked_reason = Some(reason);
                    true
                }
            });
        if changed {
            self.persist_orchestrator_state().await?;
        }
        Ok(())
    }

    async fn clear_parent_eligibility_block(
        &mut self,
        parent_id: &IssueId,
    ) -> Result<(), SchedulerError> {
        let changed = self
            .hierarchy_state
            .hierarchy
            .get_mut(parent_id)
            .is_some_and(|snapshot| snapshot.eligibility_blocked_reason.take().is_some());
        if changed {
            self.persist_orchestrator_state().await?;
        }
        Ok(())
    }

    async fn hydrate_parent_hierarchy_for_terminal_child(
        &mut self,
        issue: &NormalizedIssue,
    ) -> Result<(), SchedulerError> {
        let Some(parent_id) = issue.parent_id.as_ref() else {
            return Ok(());
        };
        if self.hierarchy_state.has_ancestor_edge(&issue.id) {
            return Ok(());
        }
        let parent = self
            .tracker
            .issue_by_id(parent_id.as_str())
            .await
            .map_err(|error| SchedulerError::Tracker {
                detail: format!("failed to hydrate terminal child parent {parent_id}: {error}"),
            })?
            .ok_or_else(|| SchedulerError::Tracker {
                detail: format!("terminal child parent {parent_id} was not found"),
            })?;
        if self.reconcile_hierarchy_issue(&parent, None)? {
            self.hierarchy_state_dirty = true;
            self.persist_orchestrator_state().await?;
        }
        if self.hierarchy_state.has_ancestor_edge(&issue.id) {
            Ok(())
        } else {
            Err(SchedulerError::Tracker {
                detail: format!(
                    "terminal child {issue_id} parent {parent_id} did not contain the child edge",
                    issue_id = issue.id
                ),
            })
        }
    }

    fn parent_eligibility_work_units(&self, parent_id: &IssueId) -> usize {
        self.hierarchy_provider_work_units(parent_id, &mut HashSet::new())
    }

    fn hierarchy_provider_work_units(
        &self,
        issue_id: &IssueId,
        visiting: &mut HashSet<IssueId>,
    ) -> usize {
        if !visiting.insert(issue_id.clone()) {
            return 1;
        }
        let child_ids = self
            .hierarchy_state
            .hierarchy
            .get(issue_id)
            .map(|snapshot| {
                snapshot
                    .required_child_edges
                    .iter()
                    .filter(|edge| edge.required)
                    .map(|edge| edge.child_id.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let work_units = if child_ids.is_empty() {
            1
        } else {
            let descendant_work_units = child_ids
                .iter()
                .map(|child_id| self.hierarchy_provider_work_units(child_id, visiting))
                .sum::<usize>();
            let has_nested_parent = child_ids
                .iter()
                .any(|child_id| self.hierarchy_state.hierarchy.contains_key(child_id));
            descendant_work_units + usize::from(has_nested_parent)
        };
        visiting.remove(issue_id);
        work_units.max(1)
    }

    async fn persist_retry_if_queued(&mut self, issue_id: &IssueId) -> Result<(), SchedulerError> {
        let Some((retry, workspace)) = self.executions.get(issue_id).and_then(|execution| {
            execution
                .retry()
                .cloned()
                .zip(execution.workspace().cloned())
        }) else {
            return Ok(());
        };
        if let Err(error) = self
            .workspace
            .persist_retry_pending(&workspace, &retry)
            .await
        {
            self.pending_retry_persistence
                .insert(issue_id.clone(), retry);
            return Err(SchedulerError::Workspace {
                detail: error.to_string(),
            });
        }
        self.workspace
            .clear_retry_pending(issue_id)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?;
        Ok(())
    }

    async fn load_recovery_state(&mut self) -> Result<(), SchedulerError> {
        if self.pending_recovery.is_some()
            && self.pending_retry_exhaustion.is_some()
            && self.pending_retry_recovery.is_some()
        {
            return Ok(());
        }

        if !self.durable_state_loaded {
            if let Some(raw) = self
                .workspace
                .load_orchestrator_state()
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?
            {
                self.hierarchy_state =
                    serde_json::from_value(raw).map_err(|error| SchedulerError::Workspace {
                        detail: format!("invalid durable hierarchy state: {error}"),
                    })?;
                self.hierarchy_state
                    .validate()
                    .map_err(|detail| SchedulerError::Workspace { detail })?;
            }
            self.durable_state_loaded = true;
        }

        let recoveries = self.workspace.recover_workspaces().await.map_err(|error| {
            SchedulerError::Workspace {
                detail: error.to_string(),
            }
        })?;
        let recovered_run_started_at =
            self.workspace
                .recovered_run_started_at()
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?;
        let retry_exhaustion =
            self.workspace
                .recover_retry_exhaustion()
                .await
                .map_err(|error| SchedulerError::Workspace {
                    detail: error.to_string(),
                })?;
        let retry_pending = self
            .workspace
            .recover_retry_pending()
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?;
        self.recovered_memory_issue_ids
            .extend(recoveries.iter().map(|record| record.issue.id.clone()));
        self.recovered_memory_issue_ids
            .extend(retry_pending.iter().map(|record| record.issue.id.clone()));
        self.recovered_memory_issue_ids
            .extend(recovered_run_started_at.keys().cloned());
        if !recovered_run_started_at.is_empty() {
            self.hierarchy_state
                .run_started_at_by_issue
                .extend(recovered_run_started_at);
            self.persist_orchestrator_state().await?;
        }
        self.pending_recovery = Some(recoveries);
        self.pending_retry_exhaustion = Some(retry_exhaustion);
        self.pending_retry_recovery = Some(retry_pending);
        Ok(())
    }

    async fn persist_orchestrator_state(&mut self) -> Result<(), SchedulerError> {
        let mut retained_issue_ids = self
            .executions
            .iter()
            .filter(|(_, execution)| {
                execution.workspace().is_some()
                    || matches!(
                        execution.status(),
                        SchedulerStatus::Claimed
                            | SchedulerStatus::Running
                            | SchedulerStatus::RetryQueued
                    )
            })
            .map(|(issue_id, _)| issue_id.clone())
            .collect::<BTreeSet<_>>();
        retained_issue_ids.extend(self.recovered_memory_issue_ids.iter().cloned());
        self.hierarchy_state
            .prune_obsolete_run_boundaries(&retained_issue_ids);
        let state = serde_json::to_value(&self.hierarchy_state).map_err(|error| {
            SchedulerError::Workspace {
                detail: format!("failed to encode durable hierarchy state: {error}"),
            }
        })?;
        self.workspace
            .persist_orchestrator_state(&state)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?;
        self.hierarchy_state_dirty = false;
        Ok(())
    }

    async fn retain_terminal_child_lease(
        &mut self,
        execution: &IssueExecution,
    ) -> Result<bool, SchedulerError> {
        let Some(workspace) = execution.workspace() else {
            return Ok(false);
        };
        self.retain_terminal_child_lease_for_workspace(execution.issue(), workspace)
            .await
    }

    async fn parent_is_terminal_and_undispatched(&mut self, parent_id: &IssueId) -> bool {
        let parent_identifier = parent_id.as_str().to_owned();
        let Ok(states) = self
            .tracker
            .issue_states_by_ids(std::slice::from_ref(&parent_identifier))
            .await
        else {
            // A failed refresh must retain evidence. Cleanup can retry after
            // the provider confirms whether the parent is still terminal.
            return false;
        };
        let parent_terminal = states.iter().any(|snapshot| {
            snapshot.id == parent_identifier
                && state_category_from_name(&snapshot.state.name, &self.config)
                    == IssueStateCategory::Terminal
        });
        let undispatched = parent_terminal
            && self
                .hierarchy_state
                .hierarchy
                .get(parent_id)
                .is_some_and(|snapshot| !snapshot.has_dispatched_execution_fence());
        if !undispatched {
            self.terminal_undispatched_parent_ids.remove(parent_id);
        }
        undispatched
    }

    async fn retain_terminal_child_lease_for_workspace(
        &mut self,
        issue: &NormalizedIssue,
        workspace: &WorkspaceRecord,
    ) -> Result<bool, SchedulerError> {
        self.hydrate_parent_hierarchy_for_terminal_child(issue)
            .await?;
        if !self.hierarchy_state.has_ancestor_edge(&issue.id) {
            return Ok(false);
        }
        if self
            .hierarchy_state
            .has_active_dispatched_ancestor(&issue.id)
        {
            return Ok(false);
        }
        if let Some(parent_id) = issue.parent_id.as_ref()
            && self.parent_is_terminal_and_undispatched(parent_id).await
        {
            if self
                .hierarchy_state
                .release_subtree_evidence_for_undispatched_parent(parent_id, current_epoch_millis())
            {
                self.persist_orchestrator_state().await?;
            }
            return Ok(false);
        }
        let Some(resource) = self
            .workspace
            .workspace_lease_resource(issue, workspace)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?
        else {
            return Ok(false);
        };
        if self.hierarchy_state.leases.iter().any(|lease| {
            lease.active()
                && lease.resource == resource
                && lease.kind != super::LeaseKind::LeafWorker
        }) {
            return Ok(false);
        }
        let issue_id = issue.id.clone();
        let leaf_owner = super::LeaseOwner::leaf_worker(&issue_id);
        let hierarchy_generation = self
            .hierarchy_state
            .run_hierarchy_generations
            .get(&issue_id)
            .copied()
            .or_else(|| {
                self.hierarchy_state
                    .hierarchy
                    .values()
                    .filter(|snapshot| {
                        snapshot
                            .required_child_edges
                            .iter()
                            .any(|edge| edge.required && edge.child_id == issue_id)
                    })
                    .map(|snapshot| snapshot.generation)
                    .next()
            })
            // A terminal child can finish before its parent enters the
            // scheduler's configured scan. Seed the first snapshot generation
            // so retained evidence can be consumed when that parent arrives.
            .unwrap_or(1);
        if self.hierarchy_state.leases.iter().any(|lease| {
            lease.active()
                && lease.kind == super::LeaseKind::LeafWorker
                && lease.owner == leaf_owner
                && lease.resource == resource
                && lease.hierarchy_generation == hierarchy_generation
        }) {
            return Ok(true);
        }
        let acquired_at = current_epoch_millis();
        let mut next_state = self.hierarchy_state.clone();
        for lease in &mut next_state.leases {
            if lease.active()
                && lease.kind == super::LeaseKind::LeafWorker
                && lease.owner == leaf_owner
            {
                lease.released_at = Some(acquired_at);
            }
        }
        next_state
            .acquire_leases(vec![LeaseRecord {
                kind: super::LeaseKind::LeafWorker,
                resource: resource.clone(),
                owner: leaf_owner,
                hierarchy_generation,
                acquired_at,
                expires_at: None,
                released_at: None,
            }])
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?;
        let previous_state = std::mem::replace(&mut self.hierarchy_state, next_state);
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_state;
            return Err(error);
        }
        Ok(true)
    }

    async fn workspace_has_active_lease(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<bool, SchedulerError> {
        self.workspace
            .workspace_has_active_lease(workspace)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })
    }

    async fn release_stale_binding_leases(
        &mut self,
        issue: &NormalizedIssue,
        workspace: &WorkspaceRecord,
        released_at: TimestampMs,
    ) -> Result<(), SchedulerError> {
        let Some(resource) = self
            .workspace
            .workspace_lease_resource(issue, workspace)
            .await
            .map_err(|error| SchedulerError::Workspace {
                detail: error.to_string(),
            })?
        else {
            return Ok(());
        };
        let mut next_state = self.hierarchy_state.clone();
        if !next_state.release_resource_leases(&resource, released_at.as_u64()) {
            return Ok(());
        }
        let previous_state = std::mem::replace(&mut self.hierarchy_state, next_state);
        if let Err(error) = self.persist_orchestrator_state().await {
            self.hierarchy_state = previous_state;
            return Err(error);
        }
        Ok(())
    }

    async fn flush_pending_retry_exhaustion_persistence(&mut self) -> Result<(), SchedulerError> {
        let pending = std::mem::take(&mut self.pending_retry_exhaustion_persistence);
        let mut first_error = None;
        for (issue_id, record) in pending {
            if let Err(error) = self
                .workspace
                .persist_retry_exhaustion(&record.issue, record.normal_retry_count)
                .await
            {
                self.pending_retry_exhaustion_persistence
                    .insert(issue_id, record);
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
                continue;
            }
            self.cleanup_retry_exhausted_workspace_if_ready(&issue_id)
                .await;
        }
        first_error.map_or(Ok(()), |detail| Err(SchedulerError::Workspace { detail }))
    }

    async fn cleanup_retry_exhausted_workspace_if_ready(&mut self, issue_id: &IssueId) {
        if self.workspace.retain_failed_workspaces() {
            return;
        }
        let Some(workspace) = self
            .executions
            .get(issue_id)
            .filter(|execution| retry_exhausted_release(execution))
            .and_then(|execution| execution.workspace().cloned())
        else {
            return;
        };
        match self.workspace_has_active_lease(&workspace).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(
                    issue = %issue_id,
                    %error,
                    "unable to check durable lease before retry-exhausted cleanup"
                );
                return;
            }
        }
        match self.workspace.cleanup_failed_workspace(&workspace).await {
            Ok(()) => {
                if let Some(execution) = self.executions.get_mut(issue_id) {
                    execution.clear_workspace();
                }
            }
            Err(error) => {
                tracing::warn!(
                    issue = %issue_id,
                    %error,
                    "retry-exhausted workspace cleanup failed after durable marker persistence"
                );
            }
        }
    }

    fn next_worker_id(&mut self) -> Result<WorkerId, SchedulerError> {
        self.next_worker_ordinal = self.next_worker_ordinal.saturating_add(1);
        WorkerId::new(format!("scheduler-worker-{}", self.next_worker_ordinal))
            .map_err(SchedulerError::Identifier)
    }

    fn reserve_recovered_worker_id(&mut self, worker_id: &WorkerId) {
        if let Some(ordinal) = recovered_worker_ordinal(worker_id) {
            self.next_worker_ordinal = self.next_worker_ordinal.max(ordinal);
        }
    }

    fn remove_execution(&mut self, issue_id: &IssueId) -> Option<IssueExecution> {
        let execution = self.executions.remove(issue_id)?;
        self.decrement_running_count(&execution);
        self.debug_assert_running_counts();
        Some(execution)
    }

    fn insert_execution(&mut self, issue_id: IssueId, execution: IssueExecution) {
        // Reopened/recovered work invalidates old success before launch
        // preparation can temporarily remove the execution from this map.
        if execution.status() != SchedulerStatus::Released {
            self.hierarchy_state_dirty |= self
                .hierarchy_state
                .terminal_orchestrator_issues
                .remove(&issue_id);
        }
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
    terminal: Vec<TrackerIssue>,
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

    fn terminal_issue(&self, issue_id: &IssueId) -> Option<&TrackerIssue> {
        self.terminal
            .iter()
            .find(|issue| issue.id == issue_id.as_str())
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

fn recovered_route(
    mut route: HarnessRouteDecision,
    persisted_harness_kind: Option<&str>,
) -> Result<HarnessRouteDecision, SchedulerError> {
    let persisted_harness_kind = persisted_harness_kind
        .filter(|kind| !kind.trim().is_empty())
        .ok_or_else(|| SchedulerError::InvalidConfiguration {
            detail: "recovered run is missing its persisted harness kind".to_string(),
        })?;
    if route.harness_kind == persisted_harness_kind {
        return Ok(route);
    }
    let capability = harness_capability(persisted_harness_kind)?;
    if !capability.available || !capability.actions.start_run {
        return Err(SchedulerError::InvalidConfiguration {
            detail: format!(
                "persisted recovery harness `{persisted_harness_kind}` cannot start issue execution"
            ),
        });
    }
    route.harness_kind = persisted_harness_kind.to_owned();
    route.reason = format!(
        "recovered persisted harness `{persisted_harness_kind}`; {}",
        route.reason
    );
    route.user_override = false;
    Ok(route)
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

fn recovered_worker_ordinal(worker_id: &WorkerId) -> Option<u64> {
    worker_id
        .as_str()
        .strip_prefix("scheduler-worker-")
        .and_then(|value| value.parse::<u64>().ok())
}

fn harness_capability(kind: &str) -> Result<HarnessCapability, SchedulerError> {
    HarnessKind::parse(kind)
        .map(HarnessKind::capability)
        .ok_or_else(|| SchedulerError::InvalidConfiguration {
            detail: format!("unknown routing harness `{kind}`"),
        })
}

fn tracker_issue_belongs_to_configured_project(
    issue: &TrackerIssue,
    config: &SchedulerConfig,
) -> bool {
    project_belongs_to_configured_project(
        config,
        issue.project_id.as_deref(),
        issue.project_slug.as_deref(),
        true,
    )
}

fn project_belongs_to_configured_project(
    config: &SchedulerConfig,
    project_id: Option<&str>,
    project_slug: Option<&str>,
    project_identity_known: bool,
) -> bool {
    if config.tracker_project_id.is_none()
        && config.tracker_project_slug.is_none()
        && config.tracker_project_ids.is_empty()
        && config.tracker_project_slugs.is_empty()
    {
        return true;
    }
    // Older tracker adapters may not expose project identity on a state-only
    // response. Preserve that compatibility path; a live Linear response now
    // carries the identity so moved issues are still fenced immediately.
    if !project_identity_known && project_id.is_none() && project_slug.is_none() {
        return true;
    }
    if let Some(config_project_id) = config.tracker_project_id.as_deref()
        && project_id
            .is_some_and(|issue_project_id| issue_project_id.trim() == config_project_id.trim())
    {
        return true;
    }
    if project_id.is_some_and(|issue_project_id| {
        config
            .tracker_project_ids
            .iter()
            .enumerate()
            .any(|(index, project_id)| {
                !config
                    .tracker_project_id_slug_fallbacks
                    .get(index)
                    .copied()
                    .unwrap_or(false)
                    && issue_project_id.trim() == project_id.trim()
            })
    }) {
        return true;
    }
    let singular_slug_matches =
        config
            .tracker_project_slug
            .as_deref()
            .is_some_and(|config_project_slug| {
                let singular_matches_first_vector_entry = config
                    .tracker_project_slugs
                    .first()
                    .is_none_or(|first_project_slug| {
                        first_project_slug
                            .trim()
                            .eq_ignore_ascii_case(config_project_slug.trim())
                    });
                singular_matches_first_vector_entry
                    && project_slug.is_some_and(|issue_project_slug| {
                        issue_project_slug
                            .trim()
                            .eq_ignore_ascii_case(config_project_slug.trim())
                    })
            });
    singular_slug_matches
        || project_slug.is_some_and(|issue_project_slug| {
            config.tracker_project_slugs.iter().any(|project_slug| {
                issue_project_slug
                    .trim()
                    .eq_ignore_ascii_case(project_slug.trim())
            })
        })
}

fn normalize_tracker_issue(
    issue: &TrackerIssue,
    config: &SchedulerConfig,
) -> Result<NormalizedIssue, SchedulerError> {
    let is_parent = !issue.sub_issues.is_empty();
    let repository_binding = resolve_repository_binding(
        config,
        &issue.labels,
        issue.project_id.as_deref(),
        issue.project_slug.as_deref(),
        is_parent,
    );
    Ok(NormalizedIssue {
        id: IssueId::new(issue.id.clone())?,
        identifier: IssueIdentifier::new(issue.identifier.clone())?,
        title: issue.title.clone(),
        description: issue.description.clone(),
        priority: issue.priority,
        state: issue_state_from_name(&issue.state, config),
        branch_name: issue.branch_name.clone(),
        pr_url: issue.pr_url.clone(),
        pr_urls: issue.pr_urls.clone(),
        url: Some(issue.url.clone()),
        labels: issue.labels.clone(),
        project_id: issue.project_id.clone(),
        project_slug: issue.project_slug.clone(),
        project_name: issue.project_name.clone(),
        parent_id: match &issue.parent_id {
            Some(parent_id) => Some(IssueId::new(parent_id.clone())?),
            None => None,
        },
        repository_binding,
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
        pr_urls: Vec::new(),
        url: None,
        labels: snapshot.labels.clone(),
        project_id: snapshot.project_id.clone(),
        project_slug: snapshot.project_slug.clone(),
        project_name: None,
        parent_id: None,
        repository_binding: resolve_repository_binding(
            config,
            &snapshot.labels,
            snapshot.project_id.as_deref(),
            snapshot.project_slug.as_deref(),
            snapshot.is_parent,
        ),
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: None,
        updated_at: Some(datetime_to_timestamp(snapshot.updated_at)),
    })
}

fn resolve_repository_binding(
    config: &SchedulerConfig,
    labels: &[String],
    project_id: Option<&str>,
    project_slug: Option<&str>,
    is_parent: bool,
) -> Option<RepositoryBindingOutcome> {
    config.repository_routing.as_ref().and_then(|routing| {
        // An unlabeled parent is intentionally repository-neutral. Only a
        // managed label on a parent is invalid; terminal children still use
        // strict routing outcomes for missing or malformed bindings.
        if is_parent && managed_repository_aliases(labels).is_empty() {
            None
        } else {
            Some(routing.resolve(labels, project_id, project_slug, is_parent))
        }
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

fn acknowledged_operator_cancel_terminal(
    execution: &IssueExecution,
    outcome: &WorkerOutcomeRecord,
) -> bool {
    matches!(
        outcome.outcome,
        WorkerOutcomeKind::Succeeded | WorkerOutcomeKind::Cancelled
    ) && execution.interrupt().is_some_and(|interrupt| {
        matches!(
            interrupt.status,
            HarnessInterruptStatus::Requested
                | HarnessInterruptStatus::Acknowledged
                | HarnessInterruptStatus::Failed
                | HarnessInterruptStatus::TimedOut
        ) && interrupt.command.reason == HarnessInterruptReason::OperatorCancel
            && interrupt.command.expected_next_state == HarnessInterruptExpectedNextState::Paused
    })
}

fn terminal_worker_outcome_prevents_reopen(execution: &IssueExecution) -> bool {
    retry_exhausted_release(execution)
        || matches!(
            execution.state(),
            crate::opensymphony_orchestrator::SchedulerState::Released {
                reason: ReleaseReason::Completed | ReleaseReason::Cancelled,
                ..
            }
        )
        || matches!(
            execution
                .last_worker_outcome()
                .map(|outcome| outcome.outcome),
            Some(WorkerOutcomeKind::Detached | WorkerOutcomeKind::CancelFailed)
        )
        || execution
            .last_worker_outcome()
            .is_some_and(|outcome| acknowledged_operator_cancel_terminal(execution, outcome))
}

fn retry_exhausted_release(execution: &IssueExecution) -> bool {
    matches!(
        execution.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    )
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

fn is_canceled_tracker_issue(issue: &TrackerIssue, canceled_states: &[String]) -> bool {
    matches!(issue.state_kind, TrackerIssueStateKind::Canceled)
        || canceled_states.iter().any(|configured_state| {
            configured_state.to_ascii_lowercase().contains("cancel")
                && configured_state
                    .trim()
                    .eq_ignore_ascii_case(issue.state.trim())
        })
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
        pr_urls: issue.pr_urls.clone(),
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
                state_kind: tracker_state_kind_from_name(&child.state),
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

fn hierarchy_depth(
    state: &DurableOrchestratorState,
    parent_id: &IssueId,
    target_id: &IssueId,
) -> usize {
    let mut pending = vec![(parent_id.clone(), 0usize)];
    let mut visited = BTreeSet::new();
    while let Some((issue_id, depth)) = pending.pop() {
        if !visited.insert(issue_id.clone()) {
            continue;
        }
        if &issue_id == target_id {
            return depth;
        }
        if let Some(snapshot) = state.hierarchy.get(&issue_id) {
            pending.extend(
                snapshot
                    .required_child_edges
                    .iter()
                    .filter(|edge| edge.required)
                    .map(|edge| (edge.child_id.clone(), depth.saturating_add(1))),
            );
        }
    }
    0
}

fn conversation_id_suffix(value: &str) -> &str {
    value.get(value.len().saturating_sub(8)..).unwrap_or(value)
}

fn parent_input_version(snapshot: &HierarchySnapshot) -> String {
    let merges = snapshot
        .dispatch_required_merge_commits
        .iter()
        .map(|commit| {
            format!(
                "{}@{}",
                commit
                    .repository_id
                    .as_ref()
                    .map_or("<legacy>", |repository| repository.as_str()),
                commit.commit
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("hierarchy:{};merges:{merges}", snapshot.generation)
}

fn parent_targets_input_version(
    hierarchy_generation: u64,
    targets: &[ParentRepositoryTarget],
) -> String {
    let targets = targets
        .iter()
        .map(|target| {
            format!(
                "{}:{}:{}@{}",
                target.repository_id,
                target.checkout_handle,
                target.relative_path.display(),
                target.target_commit
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("hierarchy:{hierarchy_generation};targets:{targets}")
}

fn parent_controller_input_version(controller: &ParentIntegrationController) -> String {
    parent_targets_input_version(
        controller.hierarchy_generation,
        &controller.targets.values().cloned().collect::<Vec<_>>(),
    )
}

fn unavailable_provider_snapshot(
    repair: &ParentRepairAttempt,
) -> super::ParentRepairProviderSnapshot {
    super::ParentRepairProviderSnapshot {
        pull_request_id: repair.pull_request_id.clone(),
        pull_request_url: repair.pull_request_url.clone(),
        head_commit: repair.pushed_commit.clone(),
        review_head_commit: None,
        review_request_cursor: None,
        open: false,
        checks_passed: false,
        checks_failed: false,
        review_approved: false,
        review_rejected: false,
        changes_requested: false,
        review_feedback: Vec::new(),
        mergeable: false,
        merge_conflict: false,
        merged: false,
        merge_result_commit: None,
        target_contains_merge_result: false,
        provider_available: false,
    }
}

fn observe_parent_command_event(
    controller: &mut ParentIntegrationController,
    attempt_id: &str,
    parent_workspace_path: Option<&Path>,
    observed_at: TimestampMs,
    event_id: Option<&str>,
    event_kind: Option<&str>,
    payload: Option<&serde_json::Value>,
) -> Result<(), ParentIntegrationError> {
    let Some(kind) = event_kind else {
        return Ok(());
    };
    let Some(payload) = payload else {
        return Ok(());
    };
    if kind == "codex.item/started" || kind == "codex.item/completed" {
        let params = payload.get("params").unwrap_or(payload);
        let item = params.get("item").unwrap_or(params);
        let item_type = item
            .get("type")
            .or_else(|| params.get("type"))
            .and_then(serde_json::Value::as_str);
        if item_type != Some("commandExecution") {
            return Ok(());
        }
        let command_id = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .or(event_id);
        let command = item.get("command").and_then(serde_json::Value::as_str);
        if kind == "codex.item/started" {
            if let (Some(command_id), Some(command)) = (command_id, command) {
                let cwd = item
                    .get("cwd")
                    .or_else(|| item.get("workingDirectory"))
                    .or_else(|| params.get("cwd"))
                    .or_else(|| params.get("workingDirectory"))
                    .and_then(serde_json::Value::as_str);
                let root = observed_parent_command_root(controller, parent_workspace_path, cwd)?;
                controller.observe_command_started(
                    attempt_id,
                    command_id,
                    command,
                    root,
                    observed_at,
                )?;
            }
            return Ok(());
        }
        if let (Some(command_id), Some(exit_code)) = (
            command_id,
            item.get("exitCode")
                .and_then(serde_json::Value::as_i64)
                .and_then(|value| i32::try_from(value).ok()),
        ) {
            let output = item
                .get("aggregatedOutput")
                .and_then(serde_json::Value::as_str)
                .map(redact_runtime_diagnostic);
            controller.observe_command_finished(
                attempt_id,
                command_id,
                exit_code,
                output.as_deref(),
                observed_at,
            )?;
        }
        return Ok(());
    }
    if kind.ends_with("ActionEvent") {
        let command = nested_runtime_string(payload, &["command"]);
        let command_id = payload
            .get("action_id")
            .and_then(serde_json::Value::as_str)
            .or(event_id);
        if let (Some(command_id), Some(command)) = (command_id, command) {
            let cwd = nested_runtime_string(payload, &["cwd", "working_dir"]);
            let root = observed_parent_command_root(controller, parent_workspace_path, cwd)?;
            controller.observe_command_started(
                attempt_id,
                command_id,
                command,
                root,
                observed_at,
            )?;
        }
    } else if kind.ends_with("ObservationEvent")
        && let Some(exit_code) = payload
            .get("exit_code")
            .and_then(serde_json::Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
        && let Some(command_id) = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .and_then(|attempt| {
                attempt
                    .commands
                    .iter()
                    .rev()
                    .find(|command| command.finished_at.is_none())
                    .map(|command| command.command_id.clone())
            })
    {
        let output = payload
            .get("preview")
            .and_then(serde_json::Value::as_str)
            .map(redact_runtime_diagnostic);
        controller.observe_command_finished(
            attempt_id,
            &command_id,
            exit_code,
            output.as_deref(),
            observed_at,
        )?;
    }
    Ok(())
}

fn nested_runtime_string<'a>(payload: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    [Some(payload), payload.get("arguments"), payload.get("args")]
        .into_iter()
        .flatten()
        .find_map(|object| {
            keys.iter()
                .find_map(|key| object.get(*key).and_then(serde_json::Value::as_str))
        })
}

fn observed_parent_command_root(
    controller: &ParentIntegrationController,
    parent_workspace_path: Option<&Path>,
    cwd: Option<&str>,
) -> Result<ParentAttemptRoot, ParentIntegrationError> {
    let Some(cwd) = cwd.filter(|cwd| !cwd.trim().is_empty()) else {
        return Ok(ParentAttemptRoot::ParentRoot);
    };
    let cwd = PathBuf::from(cwd);
    let relative = if cwd.is_absolute() {
        let parent = parent_workspace_path.ok_or_else(|| {
            ParentIntegrationError::InvalidVerificationEvidence(
                "runtime command reported an absolute cwd without a parent workspace binding"
                    .to_owned(),
            )
        })?;
        cwd.strip_prefix(parent).map_err(|_| {
            ParentIntegrationError::InvalidVerificationEvidence(
                "runtime command cwd is outside the parent integration workspace".to_owned(),
            )
        })?
    } else {
        cwd.as_path()
    };
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(ParentIntegrationError::InvalidVerificationEvidence(
            "runtime command cwd is not a contained parent workspace path".to_owned(),
        ));
    }
    if relative.as_os_str().is_empty() || relative == Path::new(".") {
        return Ok(ParentAttemptRoot::ParentRoot);
    }
    controller
        .targets
        .values()
        .find(|target| target.relative_path == relative)
        .map(|target| ParentAttemptRoot::CheckoutHandle(target.checkout_handle.clone()))
        .ok_or_else(|| {
            ParentIntegrationError::InvalidVerificationEvidence(
                "runtime command cwd does not match a verified parent checkout".to_owned(),
            )
        })
}

fn enforce_parent_outcome_trust(
    outcome: &mut WorkerOutcomeRecord,
    deadline_reached: bool,
    verification_passed: bool,
) {
    if deadline_reached && outcome.outcome == WorkerOutcomeKind::Succeeded {
        outcome.outcome = WorkerOutcomeKind::TimedOut;
        outcome.summary = Some("parent attempt exceeded its absolute deadline".to_owned());
        outcome.error = Some("parent worker outcome arrived after the attempt deadline".to_owned());
    } else if outcome.outcome == WorkerOutcomeKind::Succeeded && !verification_passed {
        outcome.outcome = WorkerOutcomeKind::Failed;
        outcome.summary = Some("parent final verification did not pass".to_owned());
        outcome.error = Some(match outcome.error.take() {
            Some(existing) => format!(
                "{existing}; parent success lacked matching orchestrator-owned command evidence"
            ),
            None => "parent success lacked matching orchestrator-owned command evidence".to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_orchestrator::ParentRepairPolicy;

    #[test]
    fn conversation_suffix_matches_gateway_alias_shape() {
        assert_eq!(conversation_id_suffix("conv-123456789"), "23456789");
        assert_eq!(conversation_id_suffix("short"), "short");
    }

    #[test]
    fn parent_outcome_trust_enforces_deadline_and_runtime_verification() {
        let outcome = || WorkerOutcomeRecord {
            worker_id: WorkerId::new("parent-worker").expect("worker id"),
            attempt: None,
            outcome: WorkerOutcomeKind::Succeeded,
            started_at: TimestampMs::new(3),
            finished_at: TimestampMs::new(103),
            turn_count: 1,
            summary: Some("claimed success".to_owned()),
            error: None,
            harness_stopped: false,
            parent_verification: None,
        };
        let mut late = outcome();
        enforce_parent_outcome_trust(&mut late, true, true);
        assert_eq!(late.outcome, WorkerOutcomeKind::TimedOut);
        assert!(
            late.error
                .as_deref()
                .is_some_and(|error| error.contains("deadline"))
        );

        let mut unverified = outcome();
        enforce_parent_outcome_trust(&mut unverified, false, false);
        assert_eq!(unverified.outcome, WorkerOutcomeKind::Failed);
        assert!(
            unverified
                .error
                .as_deref()
                .is_some_and(|error| error.contains("orchestrator-owned command evidence"))
        );
    }

    #[test]
    fn canceled_tracker_children_are_not_reachable_for_lease_release() {
        let mut issue = tracker_issue_from_normalized(&issue_with_project(None, None));
        issue.state = "Canceled".to_owned();
        issue.state_kind = TrackerIssueStateKind::Canceled;

        assert!(is_canceled_tracker_issue(&issue, &["Canceled".to_owned()]));
    }

    fn issue_with_project(project_id: Option<&str>, project_slug: Option<&str>) -> NormalizedIssue {
        NormalizedIssue {
            id: IssueId::new("issue-project-drift").expect("issue id should be valid"),
            identifier: IssueIdentifier::new("COE-549").expect("identifier should be valid"),
            title: "project drift".to_owned(),
            description: None,
            priority: None,
            state: IssueState {
                id: None,
                name: "In Progress".to_owned(),
                category: IssueStateCategory::Active,
            },
            branch_name: None,
            pr_url: None,
            pr_urls: Vec::new(),
            url: None,
            labels: Vec::new(),
            project_id: project_id.map(str::to_owned),
            project_slug: project_slug.map(str::to_owned),
            project_name: None,
            parent_id: None,
            repository_binding: None,
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn parent_eligibility_timeout_scales_by_provider_batches() {
        assert_eq!(parent_eligibility_timeout(1), Duration::from_secs(30));
        assert_eq!(
            parent_eligibility_timeout(PARENT_ELIGIBILITY_PROVIDER_CONCURRENCY),
            Duration::from_secs(30)
        );
        assert_eq!(
            parent_eligibility_timeout(PARENT_ELIGIBILITY_PROVIDER_CONCURRENCY + 1),
            Duration::from_secs(60)
        );
        assert_eq!(
            parent_eligibility_timeout(PARENT_ELIGIBILITY_PROVIDER_CONCURRENCY * 2 + 1),
            Duration::from_secs(90)
        );
    }

    #[test]
    fn recovered_scheduler_worker_ids_expose_their_allocator_ordinal() {
        let recovered = WorkerId::new("scheduler-worker-7").expect("worker id should be valid");
        let custom = WorkerId::new("worker-from-legacy").expect("worker id should be valid");

        assert_eq!(recovered_worker_ordinal(&recovered), Some(7));
        assert_eq!(recovered_worker_ordinal(&custom), None);
    }

    #[test]
    fn openhands_command_events_supply_exit_and_teardown_receipts() {
        let mut controller = ParentIntegrationController::new(
            IssueId::new("parent-command-events").expect("parent id"),
            1,
        )
        .expect("controller");
        controller
            .admit("hierarchy:1", TimestampMs::new(1))
            .expect("admit");
        controller
            .record_workspace_prepared(
                [ParentRepositoryTarget {
                    repository_id: crate::opensymphony_domain::CanonicalRepositoryId::new(
                        "github:repository:one",
                    )
                    .expect("repository id"),
                    checkout_handle: "checkout-one".to_owned(),
                    relative_path: PathBuf::from("repositories/one"),
                    target_branch: "develop".to_owned(),
                    target_commit: "abc123".to_owned(),
                    instruction_path: PathBuf::from("AGENTS.md"),
                    instruction_hash: "instructions-1".to_owned(),
                    repair_policy: ParentRepairPolicy {
                        review_profile: "required".to_owned(),
                        review_provider: "github".to_owned(),
                        review_policy_generation: "policy-1".to_owned(),
                        required_checks: true,
                        required_review: true,
                        merge_method: "squash".to_owned(),
                    },
                }],
                "targets:1",
                TimestampMs::new(2),
            )
            .expect("workspace");
        let attempt_id = controller
            .start_attempt(
                "parent harness",
                "attempt:1",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");

        observe_parent_command_event(
            &mut controller,
            &attempt_id,
            Some(Path::new("/parent")),
            TimestampMs::new(10),
            Some("action-1"),
            Some("ActionEvent"),
            Some(&serde_json::json!({
                "action_id": "action-1",
                "tool_name": "terminal",
                "arguments": {
                    "command": "cargo test",
                    "cwd": "/parent/repositories/one"
                }
            })),
        )
        .expect("command start");
        observe_parent_command_event(
            &mut controller,
            &attempt_id,
            None,
            TimestampMs::new(20),
            Some("observation-1"),
            Some("ObservationEvent"),
            Some(&serde_json::json!({
                "observation_id": "observation-1",
                "tool_name": "terminal",
                "exit_code": 0,
                "preview": "passed token=secret"
            })),
        )
        .expect("command completion");

        let attempt = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("attempt receipt");
        assert_eq!(
            attempt
                .commands
                .last()
                .map(|command| command.command.as_str()),
            Some("cargo test")
        );
        assert_eq!(attempt.exit_code, Some(0));
        assert_eq!(
            attempt.commands.last().map(|command| &command.root),
            Some(&ParentAttemptRoot::CheckoutHandle(
                "checkout-one".to_owned()
            ))
        );
        assert_eq!(
            attempt.cleanup.as_ref().map(|cleanup| cleanup.status),
            Some(ParentCleanupStatus::Succeeded)
        );
        assert!(attempt.bounded_log.contains("token=[redacted]"));
        assert!(attempt.resources.iter().any(|resource| {
            resource.kind == "foreground_process"
                && resource.identifier == "action-1"
                && resource.status
                    == crate::opensymphony_orchestrator::ParentResourceStatus::Released
        }));
    }

    #[test]
    fn codex_command_events_cannot_backdate_completion_past_orchestrator_deadline() {
        let mut controller = ParentIntegrationController::new(
            IssueId::new("parent-command-deadline").expect("parent id"),
            1,
        )
        .expect("controller");
        controller
            .admit("hierarchy:1", TimestampMs::new(1))
            .expect("admit");
        controller
            .record_workspace_prepared(
                [ParentRepositoryTarget {
                    repository_id: crate::opensymphony_domain::CanonicalRepositoryId::new(
                        "github:repository:one",
                    )
                    .expect("repository id"),
                    checkout_handle: "checkout-one".to_owned(),
                    relative_path: PathBuf::from("repositories/one"),
                    target_branch: "develop".to_owned(),
                    target_commit: "abc123".to_owned(),
                    instruction_path: PathBuf::from("AGENTS.md"),
                    instruction_hash: "instructions-1".to_owned(),
                    repair_policy: ParentRepairPolicy {
                        review_profile: "required".to_owned(),
                        review_provider: "github".to_owned(),
                        review_policy_generation: "policy-1".to_owned(),
                        required_checks: true,
                        required_review: true,
                        merge_method: "squash".to_owned(),
                    },
                }],
                "targets:1",
                TimestampMs::new(2),
            )
            .expect("workspace");
        let attempt_id = controller
            .start_attempt(
                "parent harness",
                "attempt:1",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");

        observe_parent_command_event(
            &mut controller,
            &attempt_id,
            None,
            TimestampMs::new(10),
            Some("command-1"),
            Some("codex.item/started"),
            Some(&serde_json::json!({
                "params": {
                    "startedAtMs": 1,
                    "item": {
                        "id": "command-1",
                        "type": "commandExecution",
                        "command": "cargo test"
                    }
                }
            })),
        )
        .expect("command start");
        let error = observe_parent_command_event(
            &mut controller,
            &attempt_id,
            None,
            TimestampMs::new(103),
            Some("command-1"),
            Some("codex.item/completed"),
            Some(&serde_json::json!({
                "params": {
                    "completedAtMs": 20,
                    "item": {
                        "id": "command-1",
                        "type": "commandExecution",
                        "exitCode": 0,
                        "aggregatedOutput": "passed"
                    }
                }
            })),
        )
        .expect_err("scheduler observation time enforces the absolute deadline");
        assert!(error.to_string().contains("after the attempt deadline"));

        let attempt = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("attempt receipt");
        assert_eq!(attempt.exit_code, None);
        assert_eq!(
            attempt.cleanup.as_ref().map(|cleanup| cleanup.status),
            Some(ParentCleanupStatus::Pending)
        );
        assert!(attempt.resources.iter().any(|resource| {
            resource.kind == "foreground_process"
                && resource.identifier == "command-1"
                && resource.status
                    == crate::opensymphony_orchestrator::ParentResourceStatus::Allocated
        }));
    }

    #[test]
    fn project_identity_drift_requires_reconciliation_supersession() {
        let previous = issue_with_project(Some("project-a"), Some("project-a"));
        let moved = issue_with_project(Some("project-b"), Some("project-b"));
        assert!(project_identity_changed(&previous, &moved));

        let same_project = issue_with_project(Some("project-a"), Some("renamed-project-a"));
        assert!(!project_identity_changed(&previous, &same_project));
    }

    #[test]
    fn project_identity_can_be_cleared_when_tracker_removes_project() {
        let previous = issue_with_project(Some("project-a"), Some("project-a"));
        let removed = issue_with_project(None, None);

        assert!(project_identity_changed(&previous, &removed));
    }

    #[test]
    fn known_projectless_snapshot_does_not_belong_to_configured_project() {
        let config = SchedulerConfig {
            poll_interval_ms: 1,
            max_concurrent_agents: 1,
            max_turns: 1,
            max_concurrent_agents_by_state: BTreeMap::new(),
            retry_policy: RetryPolicy::default(),
            max_retry_attempts: None,
            stall_timeout_ms: None,
            active_states: vec!["In Progress".to_owned()],
            terminal_states: vec!["Done".to_owned()],
            tracker_project_id: Some("project-a".to_owned()),
            tracker_project_slug: None,
            tracker_project_ids: Vec::new(),
            tracker_project_id_slug_fallbacks: Vec::new(),
            tracker_project_slugs: Vec::new(),
            routing: RoutingConfig {
                harness: "rust_native".to_owned(),
                model: None,
                model_profile: None,
                harness_env: "HARNESS".to_owned(),
                model_env: "MODEL".to_owned(),
                model_profile_env: "MODEL_PROFILE".to_owned(),
                harness_from_env: false,
                model_from_env: false,
                model_profile_from_env: false,
                dry_run: false,
            },
            repository_routing: None,
        };

        assert!(!project_belongs_to_configured_project(
            &config, None, None, true
        ));
        assert!(project_belongs_to_configured_project(
            &config, None, None, false
        ));
        let full_detail = tracker_issue_from_normalized(&issue_with_project(None, None));
        assert!(!tracker_issue_belongs_to_configured_project(
            &full_detail,
            &config
        ));

        let mut fallback_config = config;
        fallback_config.tracker_project_id = None;
        fallback_config.tracker_project_ids = vec!["legacy-project-slug".to_owned()];
        fallback_config.tracker_project_id_slug_fallbacks = vec![true];
        fallback_config.tracker_project_slugs = vec!["configured-project".to_owned()];
        assert!(!project_belongs_to_configured_project(
            &fallback_config,
            Some("legacy-project-slug"),
            Some("unconfigured-project"),
            true,
        ));
    }
}
