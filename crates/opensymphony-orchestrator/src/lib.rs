//! Deterministic scheduler state machine for OpenSymphony.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use opensymphony_domain::{
    BlockerRef, CONTINUATION_RETRY_DELAY_MS, Issue, OrchestrationState, OrchestratorSnapshot,
    RateLimitSnapshot, RetryEntry, RetryQueueSnapshot, RunAttempt, RunningIssueSnapshot,
    RuntimeSession, RuntimeTotals, WorkerOutcome, WorkerOutcomeKind, normalize_state_name,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_ACTIVE_STATES: [&str; 2] = ["todo", "in progress"];
const DEFAULT_TERMINAL_STATES: [&str; 5] = ["closed", "cancelled", "canceled", "duplicate", "done"];
const DEFAULT_POLL_INTERVAL_MS: u64 = 30_000;
const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 10;
const DEFAULT_MAX_RETRY_BACKOFF_MS: u64 = 300_000;
const DEFAULT_STALL_TIMEOUT_MS: u64 = 300_000;

/// Scheduler configuration derived from the workflow crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerConfig {
    pub poll_interval_ms: u64,
    pub max_concurrent_agents: usize,
    pub max_retry_backoff_ms: u64,
    pub stall_timeout_ms: Option<u64>,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub max_concurrent_agents_by_state: BTreeMap<String, usize>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            max_concurrent_agents: DEFAULT_MAX_CONCURRENT_AGENTS,
            max_retry_backoff_ms: DEFAULT_MAX_RETRY_BACKOFF_MS,
            stall_timeout_ms: Some(DEFAULT_STALL_TIMEOUT_MS),
            active_states: DEFAULT_ACTIVE_STATES
                .into_iter()
                .map(String::from)
                .collect(),
            terminal_states: DEFAULT_TERMINAL_STATES
                .into_iter()
                .map(String::from)
                .collect(),
            max_concurrent_agents_by_state: BTreeMap::new(),
        }
    }
}

impl SchedulerConfig {
    fn normalized(mut self) -> Self {
        self.active_states = self
            .active_states
            .into_iter()
            .map(|state| normalize_state_name(&state))
            .collect();
        self.terminal_states = self
            .terminal_states
            .into_iter()
            .map(|state| normalize_state_name(&state))
            .collect();
        self.max_concurrent_agents_by_state = self
            .max_concurrent_agents_by_state
            .into_iter()
            .map(|(state, limit)| (normalize_state_name(&state), limit))
            .collect();
        self
    }

    fn per_state_limit(&self, issue: &Issue) -> usize {
        self.per_state_limit_for_state(&issue.normalized_state())
    }

    fn per_state_limit_for_state(&self, normalized_state: &str) -> usize {
        self.max_concurrent_agents_by_state
            .get(normalized_state)
            .copied()
            .unwrap_or(self.max_concurrent_agents)
    }
}

/// In-memory view of a running issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningIssue {
    pub issue: Issue,
    pub run: RunAttempt,
}

/// Claim-only reservation metadata retained until a run actually starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ClaimedReservation {
    state: String,
    normalized_state: String,
    blocked_by: Vec<BlockerRef>,
    claimed_reconcile_generation: u64,
}

/// Why an issue was released out of the claimed/running set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseReason {
    Terminal,
    Inactive,
    Missing,
    CanceledByReconciliation,
}

/// Result of a reconcile pass for one running issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationOutcome {
    pub issue_id: String,
    pub reason: ReleaseReason,
    pub cleanup_workspace: bool,
}

/// Result of applying a terminal worker outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionResult {
    pub orchestration_state: OrchestrationState,
    pub retry: Option<RetryEntry>,
    pub release_reason: Option<ReleaseReason>,
}

/// Errors raised by invalid state machine usage.
#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("issue `{0}` is already claimed")]
    AlreadyClaimed(String),
    #[error("issue `{0}` is already running")]
    AlreadyRunning(String),
    #[error("issue `{0}` is not claimed")]
    NotClaimed(String),
    #[error("issue `{0}` is not running")]
    NotRunning(String),
    #[error("issue `{issue_id}` retry is not due until {due_at} (attempted start at {started_at})")]
    RetryNotDue {
        issue_id: String,
        due_at: DateTime<Utc>,
        started_at: DateTime<Utc>,
    },
    #[error(
        "issue `{issue_id}` retry attempt mismatch: expected {expected_attempt}, got {actual_attempt:?}"
    )]
    RetryAttemptMismatch {
        issue_id: String,
        expected_attempt: u32,
        actual_attempt: Option<u32>,
    },
    #[error(
        "issue `{issue_id}` cannot start because max_concurrent_agents={limit} is fully reserved"
    )]
    GlobalCapacityReached { issue_id: String, limit: usize },
    #[error(
        "issue `{issue_id}` cannot start in state `{state}` because the state limit {limit} is fully reserved"
    )]
    StateCapacityReached {
        issue_id: String,
        state: String,
        limit: usize,
    },
    #[error("issue `{issue_id}` is no longer dispatch-eligible in state `{state}`")]
    DispatchIneligible { issue_id: String, state: String },
}

/// Single-authority orchestrator state model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerState {
    config: SchedulerConfig,
    running: BTreeMap<String, RunningIssue>,
    claimed: BTreeSet<String>,
    claimed_reservations: BTreeMap<String, ClaimedReservation>,
    retry_attempts: BTreeMap<String, RetryEntry>,
    completed: BTreeSet<String>,
    runtime_totals: RuntimeTotals,
    rate_limits: Option<RateLimitSnapshot>,
    rate_limits_observed_at: Option<DateTime<Utc>>,
    reconcile_generation: u64,
}

impl SchedulerState {
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            config: config.normalized(),
            running: BTreeMap::new(),
            claimed: BTreeSet::new(),
            claimed_reservations: BTreeMap::new(),
            retry_attempts: BTreeMap::new(),
            completed: BTreeSet::new(),
            runtime_totals: RuntimeTotals::default(),
            rate_limits: None,
            rate_limits_observed_at: None,
            reconcile_generation: 0,
        }
    }

    pub fn config(&self) -> &SchedulerConfig {
        &self.config
    }

    pub fn running(&self) -> &BTreeMap<String, RunningIssue> {
        &self.running
    }

    pub fn retry_attempts(&self) -> &BTreeMap<String, RetryEntry> {
        &self.retry_attempts
    }

    pub fn claimed(&self) -> &BTreeSet<String> {
        &self.claimed
    }

    pub fn orchestration_state(&self, issue_id: &str) -> OrchestrationState {
        if self.running.contains_key(issue_id) {
            OrchestrationState::Running
        } else if self.retry_attempts.contains_key(issue_id) {
            OrchestrationState::RetryQueued
        } else if self.claimed.contains(issue_id) {
            OrchestrationState::Claimed
        } else if self.completed.contains(issue_id) {
            OrchestrationState::Released
        } else {
            OrchestrationState::Unclaimed
        }
    }

    /// Sort and claim the next dispatch batch subject to concurrency and blocker rules.
    pub fn claim_candidate_batch(&mut self, issues: &[Issue]) -> Vec<Issue> {
        let mut eligible = issues
            .iter()
            .filter(|issue| self.is_dispatch_eligible(issue))
            .cloned()
            .collect::<Vec<_>>();
        eligible.sort_by(issue_sort_key);

        let current_claimed = self.claimed_capacity_count();
        let claimed_by_state = self.claimed_counts_by_state();
        let mut reserved_global = 0usize;
        let mut reserved_by_state: BTreeMap<String, usize> = BTreeMap::new();

        let mut claimed_batch = Vec::new();
        for issue in eligible {
            let normalized_state = issue.normalized_state();
            let global_limit_reached =
                current_claimed + reserved_global >= self.config.max_concurrent_agents;
            if global_limit_reached {
                break;
            }

            let per_state_limit = self.config.per_state_limit(&issue);
            let already_claimed_in_state = claimed_by_state
                .get(&normalized_state)
                .copied()
                .unwrap_or(0);
            let already_reserved_in_state = reserved_by_state
                .get(&normalized_state)
                .copied()
                .unwrap_or(0);
            if already_claimed_in_state + already_reserved_in_state >= per_state_limit {
                continue;
            }

            self.reserve_claim(&issue, &normalized_state);
            reserved_global += 1;
            *reserved_by_state.entry(normalized_state).or_default() += 1;
            claimed_batch.push(issue);
        }

        claimed_batch
    }

    /// Start a run for a claimed issue.
    pub fn start_run(
        &mut self,
        mut issue: Issue,
        workspace_path: PathBuf,
        attempt: Option<u32>,
        started_at: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        if self.running.contains_key(&issue.id) {
            return Err(SchedulerError::AlreadyRunning(issue.id));
        }
        if !self.claimed.contains(&issue.id) {
            return Err(SchedulerError::NotClaimed(issue.id));
        }
        if let Some(retry) = self.retry_attempts.get(&issue.id) {
            if started_at < retry.due_at {
                return Err(SchedulerError::RetryNotDue {
                    issue_id: issue.id.clone(),
                    due_at: retry.due_at,
                    started_at,
                });
            }
            if attempt != Some(retry.attempt) {
                return Err(SchedulerError::RetryAttemptMismatch {
                    issue_id: issue.id.clone(),
                    expected_attempt: retry.attempt,
                    actual_attempt: attempt,
                });
            }
        }
        let reservation = self.claimed_reservations.get(&issue.id);
        if let Err(state) = self.dispatch_ineligible_state(&issue, reservation) {
            self.release(&issue.id);
            return Err(SchedulerError::DispatchIneligible {
                issue_id: issue.id.clone(),
                state,
            });
        }
        let (state, normalized_state) = reservation
            .map(|reservation| {
                (
                    reservation.state.clone(),
                    reservation.normalized_state.clone(),
                )
            })
            .unwrap_or_else(|| (issue.state.clone(), issue.normalized_state()));
        let reserved_global = self.reserved_capacity_excluding(&issue.id);
        if reserved_global >= self.config.max_concurrent_agents {
            return Err(SchedulerError::GlobalCapacityReached {
                issue_id: issue.id.clone(),
                limit: self.config.max_concurrent_agents,
            });
        }
        let per_state_limit = self.config.per_state_limit_for_state(&normalized_state);
        let reserved_in_state =
            self.reserved_state_capacity_excluding(&issue.id, &normalized_state);
        if reserved_in_state >= per_state_limit {
            return Err(SchedulerError::StateCapacityReached {
                issue_id: issue.id.clone(),
                state: normalized_state,
                limit: per_state_limit,
            });
        }

        issue.state = state;
        self.completed.remove(&issue.id);
        self.claimed_reservations.remove(&issue.id);
        self.retry_attempts.remove(&issue.id);
        self.running.insert(
            issue.id.clone(),
            RunningIssue {
                run: RunAttempt {
                    issue_id: issue.id.clone(),
                    issue_identifier: issue.identifier.clone(),
                    attempt,
                    workspace_path,
                    started_at,
                    session: RuntimeSession::default(),
                },
                issue,
            },
        );
        Ok(())
    }

    /// Restore a running entry during daemon restart recovery.
    pub fn recover_running(&mut self, running_issue: RunningIssue) {
        self.completed.remove(&running_issue.issue.id);
        self.claimed_reservations.remove(&running_issue.issue.id);
        self.claimed.insert(running_issue.issue.id.clone());
        self.running
            .insert(running_issue.issue.id.clone(), running_issue);
    }

    /// Restore a retry entry during daemon restart recovery.
    pub fn recover_retry(&mut self, retry: RetryEntry) {
        self.completed.remove(&retry.issue_id);
        self.claimed_reservations.remove(&retry.issue_id);
        self.claimed.insert(retry.issue_id.clone());
        self.retry_attempts.insert(retry.issue_id.clone(), retry);
    }

    /// Apply a runtime update to an existing running issue.
    pub fn update_runtime_session(
        &mut self,
        issue_id: &str,
        session: RuntimeSession,
    ) -> Result<(), SchedulerError> {
        self.maybe_update_rate_limits(session.rate_limits.clone(), session.last_event_at);
        let running = self
            .running
            .get_mut(issue_id)
            .ok_or_else(|| SchedulerError::NotRunning(issue_id.to_string()))?;
        running.run.session = session;
        Ok(())
    }

    /// Finish a running issue and convert the worker outcome into an explicit scheduler transition.
    pub fn finish_run(
        &mut self,
        issue_id: &str,
        outcome: WorkerOutcome,
        finished_at: DateTime<Utc>,
    ) -> Result<TransitionResult, SchedulerError> {
        let running = self
            .running
            .remove(issue_id)
            .ok_or_else(|| SchedulerError::NotRunning(issue_id.to_string()))?;

        self.completed.remove(issue_id);
        self.maybe_update_rate_limits(
            running.run.session.rate_limits.clone(),
            running.run.session.last_event_at.or(Some(finished_at)),
        );
        self.runtime_totals
            .token_usage
            .add_assign(&running.run.session.token_usage);
        let runtime_seconds = (finished_at - running.run.started_at).num_seconds().max(0) as u64;
        self.runtime_totals.runtime_seconds += runtime_seconds;

        match outcome.kind {
            WorkerOutcomeKind::Succeeded => {
                let retry = self.enqueue_retry(
                    &running,
                    finished_at + Duration::milliseconds(CONTINUATION_RETRY_DELAY_MS),
                    outcome.error,
                );
                Ok(TransitionResult {
                    orchestration_state: OrchestrationState::RetryQueued,
                    retry: Some(retry),
                    release_reason: None,
                })
            }
            WorkerOutcomeKind::Failed
            | WorkerOutcomeKind::TimedOut
            | WorkerOutcomeKind::Stalled => {
                let next_attempt = next_attempt(running.run.attempt);
                let delay_ms = self.failure_backoff_ms(next_attempt);
                let retry = self.enqueue_retry(
                    &running,
                    finished_at
                        + Duration::milliseconds(i64::try_from(delay_ms).unwrap_or(i64::MAX)),
                    outcome.error,
                );
                Ok(TransitionResult {
                    orchestration_state: OrchestrationState::RetryQueued,
                    retry: Some(retry),
                    release_reason: None,
                })
            }
            WorkerOutcomeKind::CanceledByReconciliation => {
                self.release(issue_id);
                Ok(TransitionResult {
                    orchestration_state: OrchestrationState::Released,
                    retry: None,
                    release_reason: Some(ReleaseReason::CanceledByReconciliation),
                })
            }
        }
    }

    /// Release retry state when the retry target is no longer eligible.
    pub fn release(&mut self, issue_id: &str) {
        self.claimed.remove(issue_id);
        self.claimed_reservations.remove(issue_id);
        self.retry_attempts.remove(issue_id);
        self.running.remove(issue_id);
        self.completed.insert(issue_id.to_string());
    }

    /// Return retry entries whose timers are due at or before `now`.
    pub fn due_retries(&self, now: DateTime<Utc>) -> Vec<RetryEntry> {
        self.retry_attempts
            .values()
            .filter(|retry| retry.due_at <= now)
            .cloned()
            .collect()
    }

    /// Reconcile running issues against fresh tracker state snapshots.
    pub fn reconcile_tracker_states(&mut self, refreshed: &[Issue]) -> Vec<ReconciliationOutcome> {
        let refreshed = refreshed
            .iter()
            .cloned()
            .map(|issue| (issue.id.clone(), issue))
            .collect::<BTreeMap<_, _>>();
        let running_ids = self.running.keys().cloned().collect::<Vec<_>>();
        let retry_ids = self.retry_attempts.keys().cloned().collect::<Vec<_>>();
        let claimed_only_ids = self
            .claimed
            .iter()
            .filter(|issue_id| {
                !self.running.contains_key(*issue_id)
                    && !self.retry_attempts.contains_key(*issue_id)
            })
            .cloned()
            .collect::<Vec<_>>();

        let mut outcomes = Vec::new();
        for issue_id in running_ids {
            let Some(current) = self.running.get_mut(&issue_id) else {
                continue;
            };

            match refreshed.get(&issue_id) {
                Some(issue) if issue.is_terminal(&self.config.terminal_states) => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Terminal,
                        cleanup_workspace: true,
                    });
                }
                Some(issue)
                    if issue
                        .is_active(&self.config.active_states, &self.config.terminal_states) =>
                {
                    current.issue = issue.clone();
                }
                Some(_) => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Inactive,
                        cleanup_workspace: false,
                    });
                }
                None => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Missing,
                        cleanup_workspace: false,
                    });
                }
            }
        }

        for issue_id in retry_ids {
            if self.running.contains_key(&issue_id) || !self.retry_attempts.contains_key(&issue_id)
            {
                continue;
            }

            match refreshed.get(&issue_id) {
                Some(issue) if issue.is_terminal(&self.config.terminal_states) => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Terminal,
                        cleanup_workspace: true,
                    });
                }
                Some(issue)
                    if issue
                        .is_active(&self.config.active_states, &self.config.terminal_states) => {}
                Some(_) => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Inactive,
                        cleanup_workspace: false,
                    });
                }
                None => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Missing,
                        cleanup_workspace: false,
                    });
                }
            }
        }

        for issue_id in claimed_only_ids {
            if self.running.contains_key(&issue_id) || self.retry_attempts.contains_key(&issue_id) {
                continue;
            }

            match refreshed.get(&issue_id) {
                Some(issue) if issue.is_terminal(&self.config.terminal_states) => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Terminal,
                        cleanup_workspace: true,
                    });
                }
                Some(issue)
                    if issue
                        .is_active(&self.config.active_states, &self.config.terminal_states) =>
                {
                    if self.claimed_reservation_is_stale(&issue_id) {
                        self.release(&issue_id);
                        outcomes.push(ReconciliationOutcome {
                            issue_id,
                            reason: ReleaseReason::CanceledByReconciliation,
                            cleanup_workspace: false,
                        });
                    } else if let Some(reservation) = self.claimed_reservations.get_mut(&issue_id) {
                        reservation.state = issue.state.clone();
                        reservation.normalized_state = issue.normalized_state();
                        reservation.blocked_by = issue.blocked_by.clone();
                    }
                }
                Some(_) => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Inactive,
                        cleanup_workspace: false,
                    });
                }
                None => {
                    self.release(&issue_id);
                    outcomes.push(ReconciliationOutcome {
                        issue_id,
                        reason: ReleaseReason::Missing,
                        cleanup_workspace: false,
                    });
                }
            }
        }

        self.reconcile_generation = self.reconcile_generation.saturating_add(1);
        outcomes
    }

    /// Force retries for stalled runs based on the configured timeout.
    pub fn reconcile_stalls(&mut self, now: DateTime<Utc>) -> Vec<RetryEntry> {
        let Some(stall_timeout_ms) = self.config.stall_timeout_ms else {
            return Vec::new();
        };
        let stall_timeout =
            Duration::milliseconds(i64::try_from(stall_timeout_ms).unwrap_or(i64::MAX));

        let stalled_issue_ids = self
            .running
            .iter()
            .filter_map(|(issue_id, running)| {
                let last_activity = running
                    .run
                    .session
                    .last_event_at
                    .unwrap_or(running.run.started_at);
                (now - last_activity > stall_timeout).then_some(issue_id.clone())
            })
            .collect::<Vec<_>>();

        let mut retries = Vec::new();
        for issue_id in stalled_issue_ids {
            if let Ok(transition) = self.finish_run(
                &issue_id,
                WorkerOutcome::stalled("stall timeout exceeded"),
                now,
            ) {
                if let Some(retry) = transition.retry {
                    retries.push(retry);
                }
            }
        }

        retries
    }

    /// Compute the current snapshot consumed by the control plane.
    pub fn snapshot(&self, generated_at: DateTime<Utc>) -> OrchestratorSnapshot {
        let mut running = self
            .running
            .values()
            .cloned()
            .map(|entry| RunningIssueSnapshot {
                issue: entry.issue,
                attempt: entry.run.attempt,
                workspace_path: entry.run.workspace_path,
                started_at: entry.run.started_at,
                session: entry.run.session,
                orchestration_state: OrchestrationState::Running,
            })
            .collect::<Vec<_>>();
        running.sort_by(|left, right| issue_sort_key(&left.issue, &right.issue));

        let mut retry_queue = self
            .retry_attempts
            .values()
            .cloned()
            .map(|retry| RetryQueueSnapshot {
                issue_id: retry.issue_id,
                identifier: retry.identifier,
                attempt: retry.attempt,
                due_at: retry.due_at,
                error: retry.error,
            })
            .collect::<Vec<_>>();
        retry_queue.sort_by(|left, right| {
            left.due_at
                .cmp(&right.due_at)
                .then_with(|| left.identifier.cmp(&right.identifier))
        });

        OrchestratorSnapshot {
            generated_at,
            poll_interval_ms: self.config.poll_interval_ms,
            max_concurrent_agents: self.config.max_concurrent_agents,
            claimed_issue_ids: self.claimed.iter().cloned().collect(),
            completed_issue_ids: self.completed.iter().cloned().collect(),
            running,
            retry_queue,
            runtime_totals: self.runtime_totals.clone(),
            rate_limits: self.rate_limits.clone(),
        }
    }

    pub fn failure_backoff_ms(&self, attempt: u32) -> u64 {
        let power = attempt.saturating_sub(1).min(31);
        let delay = 10_000u64.saturating_mul(2u64.saturating_pow(power));
        delay.min(self.config.max_retry_backoff_ms)
    }

    fn is_dispatch_eligible(&self, issue: &Issue) -> bool {
        if self.claimed.contains(&issue.id) || self.running.contains_key(&issue.id) {
            return false;
        }
        self.dispatch_state_is_eligible(&issue.normalized_state(), &issue.blocked_by)
    }

    fn claimed_counts_by_state(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for running in self.running.values() {
            *counts.entry(running.issue.normalized_state()).or_default() += 1;
        }
        for reservation in self.claimed_reservations.values() {
            *counts
                .entry(reservation.normalized_state.clone())
                .or_default() += 1;
        }
        counts
    }

    fn claimed_capacity_count(&self) -> usize {
        self.running.len() + self.claimed_reservations.len()
    }

    fn reserved_capacity_excluding(&self, issue_id: &str) -> usize {
        self.claimed_capacity_count().saturating_sub(usize::from(
            self.claimed_reservations.contains_key(issue_id),
        ))
    }

    fn reserved_state_capacity_excluding(&self, issue_id: &str, normalized_state: &str) -> usize {
        let running_in_state = self
            .running
            .values()
            .filter(|running| running.issue.normalized_state() == normalized_state)
            .count();
        let claimed_only_in_state = self
            .claimed_reservations
            .iter()
            .filter(|(claimed_issue_id, reservation)| {
                claimed_issue_id.as_str() != issue_id
                    && reservation.normalized_state == normalized_state
            })
            .count();
        running_in_state + claimed_only_in_state
    }

    fn reserve_claim(&mut self, issue: &Issue, normalized_state: &str) {
        self.claimed.insert(issue.id.clone());
        self.claimed_reservations.insert(
            issue.id.clone(),
            ClaimedReservation {
                state: issue.state.clone(),
                normalized_state: normalized_state.to_string(),
                blocked_by: issue.blocked_by.clone(),
                claimed_reconcile_generation: self.reconcile_generation,
            },
        );
    }

    fn claimed_reservation_is_stale(&self, issue_id: &str) -> bool {
        let Some(reservation) = self.claimed_reservations.get(issue_id) else {
            return false;
        };
        let poll_interval_ms = self.config.poll_interval_ms.max(1);
        let grace_timeout_ms = self
            .config
            .stall_timeout_ms
            .unwrap_or(DEFAULT_STALL_TIMEOUT_MS)
            .max(poll_interval_ms);
        let grace_generations =
            (grace_timeout_ms.saturating_add(poll_interval_ms - 1)) / poll_interval_ms;

        self.reconcile_generation
            .saturating_sub(reservation.claimed_reconcile_generation)
            >= grace_generations
    }

    fn dispatch_state_is_eligible(&self, normalized_state: &str, blocked_by: &[BlockerRef]) -> bool {
        self.config.active_states.iter().any(|state| state == normalized_state)
            && !self
                .config
                .terminal_states
                .iter()
                .any(|state| state == normalized_state)
            && !(normalized_state == "todo"
                && blocked_by
                    .iter()
                    .any(|blocker| !blocker.is_terminal(&self.config.terminal_states)))
    }

    fn dispatch_ineligible_state(
        &self,
        issue: &Issue,
        reservation: Option<&ClaimedReservation>,
    ) -> Result<(), String> {
        let issue_state = issue.normalized_state();
        if !self.dispatch_state_is_eligible(&issue_state, &issue.blocked_by) {
            return Err(issue_state);
        }
        if let Some(reservation) = reservation {
            if !self.dispatch_state_is_eligible(
                &reservation.normalized_state,
                &reservation.blocked_by,
            ) {
                return Err(reservation.normalized_state.clone());
            }
        }
        Ok(())
    }

    fn maybe_update_rate_limits(
        &mut self,
        rate_limits: Option<RateLimitSnapshot>,
        observed_at: Option<DateTime<Utc>>,
    ) {
        let Some(rate_limits) = rate_limits else {
            return;
        };
        let should_replace = match (self.rate_limits_observed_at, observed_at) {
            (None, _) => true,
            (Some(_), None) => self.rate_limits.is_none(),
            (Some(current), Some(candidate)) => candidate >= current,
        };
        if should_replace {
            self.rate_limits = Some(rate_limits);
            self.rate_limits_observed_at = observed_at;
        }
    }

    fn enqueue_retry(
        &mut self,
        running: &RunningIssue,
        due_at: DateTime<Utc>,
        error: Option<String>,
    ) -> RetryEntry {
        let retry = RetryEntry {
            issue_id: running.issue.id.clone(),
            identifier: running.issue.identifier.clone(),
            attempt: next_attempt(running.run.attempt),
            due_at,
            error,
        };
        self.completed.remove(&running.issue.id);
        self.claimed_reservations.remove(&running.issue.id);
        self.retry_attempts
            .insert(running.issue.id.clone(), retry.clone());
        self.claimed.insert(running.issue.id.clone());
        retry
    }
}

fn next_attempt(current: Option<u32>) -> u32 {
    current.unwrap_or(0) + 1
}

fn issue_sort_key(left: &Issue, right: &Issue) -> std::cmp::Ordering {
    let left_priority = left.priority.unwrap_or(u8::MAX);
    let right_priority = right.priority.unwrap_or(u8::MAX);
    left_priority
        .cmp(&right_priority)
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.identifier.cmp(&right.identifier))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0).single().unwrap()
    }

    fn millis(milliseconds: i64) -> DateTime<Utc> {
        Utc.timestamp_millis_opt(milliseconds).single().unwrap()
    }

    fn issue(
        id: &str,
        identifier: &str,
        state: &str,
        priority: Option<u8>,
        created_at: DateTime<Utc>,
    ) -> Issue {
        let issue = Issue::new(id, identifier, identifier, state, created_at);
        match priority {
            Some(priority) => issue.with_priority(priority),
            None => issue,
        }
    }

    fn claim_and_start_run(
        scheduler: &mut SchedulerState,
        issue: Issue,
        workspace_path: PathBuf,
        attempt: Option<u32>,
        started_at: DateTime<Utc>,
    ) {
        let claimed = scheduler.claim_candidate_batch(std::slice::from_ref(&issue));
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, issue.id);
        scheduler
            .start_run(issue, workspace_path, attempt, started_at)
            .unwrap();
    }

    fn blocker(identifier: &str, state: &str) -> BlockerRef {
        BlockerRef {
            id: Some(format!("blocker-{identifier}")),
            identifier: Some(identifier.into()),
            state: Some(state.into()),
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn candidate_sorting_obeys_priority_age_and_blockers() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let blocked = issue("3", "OSYM-3", "Todo", Some(1), timestamp(3)).with_blockers(vec![
            opensymphony_domain::BlockerRef {
                id: Some("b1".into()),
                identifier: Some("OSYM-4".into()),
                state: Some("In Progress".into()),
                created_at: None,
                updated_at: None,
            },
        ]);
        let candidates = scheduler.claim_candidate_batch(&[
            issue("2", "OSYM-2", "In Progress", Some(2), timestamp(2)),
            blocked,
            issue("1", "OSYM-1", "Todo", Some(1), timestamp(1)),
            issue("4", "OSYM-4", "Todo", Some(1), timestamp(4)),
        ]);

        let identifiers = candidates
            .iter()
            .map(|issue| issue.identifier.as_str())
            .collect::<Vec<_>>();
        assert_eq!(identifiers, vec!["OSYM-1", "OSYM-4", "OSYM-2"]);
    }

    #[test]
    fn bounded_concurrency_honors_per_state_limits() {
        let mut config = SchedulerConfig {
            max_concurrent_agents: 2,
            ..SchedulerConfig::default()
        };
        config
            .max_concurrent_agents_by_state
            .insert("in progress".into(), 1);
        let mut scheduler = SchedulerState::new(config);

        let existing = issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1));
        claim_and_start_run(
            &mut scheduler,
            existing.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            timestamp(1),
        );

        let claimed = scheduler.claim_candidate_batch(&[
            issue("2", "OSYM-2", "In Progress", Some(1), timestamp(2)),
            issue("3", "OSYM-3", "Todo", Some(1), timestamp(3)),
        ]);

        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].identifier, "OSYM-3");
    }

    #[test]
    fn claimed_capacity_is_reserved_across_poll_ticks() {
        let config = SchedulerConfig {
            max_concurrent_agents: 1,
            ..SchedulerConfig::default()
        };
        let mut scheduler = SchedulerState::new(config);

        let first = scheduler.claim_candidate_batch(&[issue(
            "1",
            "OSYM-1",
            "In Progress",
            Some(1),
            timestamp(1),
        )]);
        let second = scheduler.claim_candidate_batch(&[issue(
            "2",
            "OSYM-2",
            "In Progress",
            Some(1),
            timestamp(2),
        )]);

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn reconciliation_releases_active_claimed_only_reservations_without_backing_runs() {
        let config = SchedulerConfig {
            max_concurrent_agents: 1,
            poll_interval_ms: 10,
            stall_timeout_ms: Some(10),
            ..SchedulerConfig::default()
        };
        let mut scheduler = SchedulerState::new(config);

        let first = scheduler.claim_candidate_batch(&[issue(
            "1",
            "OSYM-1",
            "In Progress",
            Some(1),
            timestamp(1),
        )]);
        assert_eq!(first.len(), 1);

        let outcomes = scheduler.reconcile_tracker_states(&[issue(
            "1",
            "OSYM-1",
            "In Progress",
            Some(1),
            timestamp(2),
        )]);

        assert!(outcomes.is_empty());
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Claimed
        );

        let outcomes = scheduler.reconcile_tracker_states(&[issue(
            "1",
            "OSYM-1",
            "In Progress",
            Some(1),
            timestamp(3),
        )]);

        assert_eq!(
            outcomes,
            vec![ReconciliationOutcome {
                issue_id: "1".into(),
                reason: ReleaseReason::CanceledByReconciliation,
                cleanup_workspace: false,
            }]
        );
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Released
        );

        let second = scheduler.claim_candidate_batch(&[issue(
            "2",
            "OSYM-2",
            "In Progress",
            Some(1),
            timestamp(3),
        )]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, "2");
    }

    #[test]
    fn start_run_requires_a_preclaimed_issue() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());

        assert!(matches!(
            scheduler.start_run(
                issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1)),
                PathBuf::from("/tmp/OSYM-1"),
                None,
                timestamp(1),
            ),
            Err(SchedulerError::NotClaimed(id)) if id == "1"
        ));
    }

    #[test]
    fn normal_completion_schedules_fixed_continuation_retry() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let issue = issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1));
        claim_and_start_run(
            &mut scheduler,
            issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            timestamp(10),
        );

        let transition = scheduler
            .finish_run("1", WorkerOutcome::succeeded(), timestamp(20))
            .unwrap();

        assert_eq!(
            transition.orchestration_state,
            OrchestrationState::RetryQueued
        );
        let retry = transition.retry.unwrap();
        assert_eq!(retry.attempt, 1);
        assert_eq!(retry.due_at, timestamp(21));
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::RetryQueued
        );
    }

    #[test]
    fn retry_queued_issues_do_not_appear_completed_in_snapshot() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let issue = issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1));
        claim_and_start_run(
            &mut scheduler,
            issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            timestamp(10),
        );

        scheduler
            .finish_run("1", WorkerOutcome::succeeded(), timestamp(20))
            .unwrap();

        let snapshot = scheduler.snapshot(timestamp(21));

        assert!(snapshot.completed_issue_ids.is_empty());
        assert_eq!(snapshot.retry_queue.len(), 1);
    }

    #[test]
    fn retry_queued_issues_do_not_consume_dispatch_capacity_or_state_slots() {
        let config = SchedulerConfig {
            max_concurrent_agents: 1,
            max_concurrent_agents_by_state: BTreeMap::from([("in progress".into(), 1usize)]),
            ..SchedulerConfig::default()
        };
        let mut scheduler = SchedulerState::new(config);
        claim_and_start_run(
            &mut scheduler,
            issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1)),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            timestamp(10),
        );
        scheduler
            .finish_run("1", WorkerOutcome::succeeded(), timestamp(20))
            .unwrap();

        let claimed = scheduler.claim_candidate_batch(&[issue(
            "2",
            "OSYM-2",
            "In Progress",
            Some(1),
            timestamp(30),
        )]);

        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].id, "2");
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::RetryQueued
        );
        assert_eq!(
            scheduler.orchestration_state("2"),
            OrchestrationState::Claimed
        );
    }

    #[test]
    fn failure_backoff_is_exponential_and_capped() {
        let config = SchedulerConfig {
            max_retry_backoff_ms: 15_000,
            ..SchedulerConfig::default()
        };
        let mut scheduler = SchedulerState::new(config);
        let issue = issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1));
        claim_and_start_run(
            &mut scheduler,
            issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            Some(2),
            timestamp(10),
        );

        let transition = scheduler
            .finish_run("1", WorkerOutcome::failed("boom"), timestamp(20))
            .unwrap();

        let retry = transition.retry.unwrap();
        assert_eq!(retry.attempt, 3);
        assert_eq!(retry.due_at, millis(20_000 + 15_000));
    }

    #[test]
    fn start_run_rejects_retry_attempts_before_due_time() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
        claim_and_start_run(
            &mut scheduler,
            issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            millis(0),
        );

        let retry = scheduler
            .finish_run("1", WorkerOutcome::succeeded(), millis(1_000))
            .unwrap()
            .retry
            .unwrap();

        assert!(matches!(
            scheduler.start_run(
                issue.clone(),
                PathBuf::from("/tmp/OSYM-1"),
                Some(retry.attempt),
                retry.due_at - Duration::milliseconds(1),
            ),
            Err(SchedulerError::RetryNotDue { issue_id, due_at, .. })
                if issue_id == "1" && due_at == retry.due_at
        ));

        scheduler
            .start_run(
                issue,
                PathBuf::from("/tmp/OSYM-1"),
                Some(retry.attempt),
                retry.due_at,
            )
            .unwrap();

        assert_eq!(scheduler.running.len(), 1);
        assert!(scheduler.retry_attempts().is_empty());
    }

    #[test]
    fn start_run_rejects_retry_attempt_mismatch() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
        claim_and_start_run(
            &mut scheduler,
            issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            millis(0),
        );

        let retry = scheduler
            .finish_run("1", WorkerOutcome::succeeded(), millis(1_000))
            .unwrap()
            .retry
            .unwrap();

        assert!(matches!(
            scheduler.start_run(
                issue.clone(),
                PathBuf::from("/tmp/OSYM-1"),
                None,
                retry.due_at,
            ),
            Err(SchedulerError::RetryAttemptMismatch {
                issue_id,
                expected_attempt,
                actual_attempt,
            }) if issue_id == "1" && expected_attempt == retry.attempt && actual_attempt.is_none()
        ));

        assert!(matches!(
            scheduler.start_run(
                issue,
                PathBuf::from("/tmp/OSYM-1"),
                Some(retry.attempt.saturating_sub(1)),
                retry.due_at,
            ),
            Err(SchedulerError::RetryAttemptMismatch {
                issue_id,
                expected_attempt,
                actual_attempt,
            }) if issue_id == "1"
                && expected_attempt == retry.attempt
                && actual_attempt == Some(retry.attempt.saturating_sub(1))
        ));
    }

    #[test]
    fn start_run_rejects_retry_when_global_capacity_is_already_running() {
        let mut scheduler = SchedulerState::new(SchedulerConfig {
            max_concurrent_agents: 1,
            ..SchedulerConfig::default()
        });
        let retry_issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
        claim_and_start_run(
            &mut scheduler,
            retry_issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            millis(0),
        );

        let retry = scheduler
            .finish_run("1", WorkerOutcome::succeeded(), millis(1_000))
            .unwrap()
            .retry
            .unwrap();

        claim_and_start_run(
            &mut scheduler,
            issue("2", "OSYM-2", "In Progress", Some(1), millis(2_000)),
            PathBuf::from("/tmp/OSYM-2"),
            None,
            millis(2_000),
        );

        assert!(
            scheduler
                .start_run(
                    retry_issue,
                    PathBuf::from("/tmp/OSYM-1"),
                    Some(retry.attempt),
                    retry.due_at,
                )
                .is_err()
        );
    }

    #[test]
    fn start_run_rejects_retry_when_state_capacity_is_already_running() {
        let mut scheduler = SchedulerState::new(SchedulerConfig {
            max_concurrent_agents: 2,
            max_concurrent_agents_by_state: BTreeMap::from([("in progress".into(), 1)]),
            ..SchedulerConfig::default()
        });
        let retry_issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
        claim_and_start_run(
            &mut scheduler,
            retry_issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            millis(0),
        );

        let retry = scheduler
            .finish_run("1", WorkerOutcome::succeeded(), millis(1_000))
            .unwrap()
            .retry
            .unwrap();

        claim_and_start_run(
            &mut scheduler,
            issue("2", "OSYM-2", "In Progress", Some(1), millis(2_000)),
            PathBuf::from("/tmp/OSYM-2"),
            None,
            millis(2_000),
        );

        assert!(
            scheduler
                .start_run(
                    retry_issue,
                    PathBuf::from("/tmp/OSYM-1"),
                    Some(retry.attempt),
                    retry.due_at,
                )
                .is_err()
        );
    }

    #[test]
    fn start_run_releases_claims_that_become_non_active_before_launch() {
        for terminal_or_inactive_state in ["Done", "Human Review"] {
            let mut scheduler = SchedulerState::new(SchedulerConfig::default());
            let claimed_issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
            let claimed = scheduler.claim_candidate_batch(std::slice::from_ref(&claimed_issue));

            assert_eq!(claimed.len(), 1);
            assert!(matches!(
                scheduler.start_run(
                    issue("1", "OSYM-1", terminal_or_inactive_state, Some(1), millis(1_000)),
                    PathBuf::from("/tmp/OSYM-1"),
                    None,
                    millis(1_000),
                ),
                Err(SchedulerError::DispatchIneligible { issue_id, .. }) if issue_id == "1"
            ));
            assert_eq!(
                scheduler.orchestration_state("1"),
                OrchestrationState::Released
            );
        }
    }

    #[test]
    fn start_run_releases_todo_claims_that_pick_up_non_terminal_blockers() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let claimed_issue = issue("1", "OSYM-1", "Todo", Some(1), millis(0));
        let claimed = scheduler.claim_candidate_batch(std::slice::from_ref(&claimed_issue));

        assert_eq!(claimed.len(), 1);
        assert!(matches!(
            scheduler.start_run(
                issue("1", "OSYM-1", "Todo", Some(1), millis(1_000))
                    .with_blockers(vec![blocker("OSYM-2", "In Progress")]),
                PathBuf::from("/tmp/OSYM-1"),
                None,
                millis(1_000),
            ),
            Err(SchedulerError::DispatchIneligible { issue_id, state })
                if issue_id == "1" && state == "todo"
        ));
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Released
        );
    }

    #[test]
    fn start_run_uses_reserved_state_for_capacity_checks() {
        let mut scheduler = SchedulerState::new(SchedulerConfig {
            max_concurrent_agents: 2,
            max_concurrent_agents_by_state: BTreeMap::from([("in progress".into(), 1)]),
            ..SchedulerConfig::default()
        });
        let stale_claim = issue("1", "OSYM-1", "Todo", Some(1), millis(0));
        let claimed = scheduler.claim_candidate_batch(std::slice::from_ref(&stale_claim));
        assert_eq!(claimed.len(), 1);

        claim_and_start_run(
            &mut scheduler,
            issue("2", "OSYM-2", "In Progress", Some(1), millis(1_000)),
            PathBuf::from("/tmp/OSYM-2"),
            None,
            millis(1_000),
        );

        let outcomes = scheduler.reconcile_tracker_states(&[
            issue("1", "OSYM-1", "In Progress", Some(1), millis(2_000)),
            issue("2", "OSYM-2", "In Progress", Some(1), millis(2_000)),
        ]);
        assert!(outcomes.is_empty());

        assert!(matches!(
            scheduler.start_run(
                stale_claim,
                PathBuf::from("/tmp/OSYM-1"),
                None,
                millis(3_000),
            ),
            Err(SchedulerError::StateCapacityReached { issue_id, state, limit })
                if issue_id == "1" && state == "in progress" && limit == 1
        ));
    }

    #[test]
    fn reconciliation_releases_terminal_and_non_active_issues() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        claim_and_start_run(
            &mut scheduler,
            issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1)),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            timestamp(1),
        );
        claim_and_start_run(
            &mut scheduler,
            issue("2", "OSYM-2", "In Progress", Some(1), timestamp(2)),
            PathBuf::from("/tmp/OSYM-2"),
            None,
            timestamp(2),
        );

        let outcomes = scheduler.reconcile_tracker_states(&[
            issue("1", "OSYM-1", "Done", Some(1), timestamp(3)),
            issue("2", "OSYM-2", "Human Review", Some(1), timestamp(4)),
        ]);

        assert_eq!(
            outcomes,
            vec![
                ReconciliationOutcome {
                    issue_id: "1".into(),
                    reason: ReleaseReason::Terminal,
                    cleanup_workspace: true,
                },
                ReconciliationOutcome {
                    issue_id: "2".into(),
                    reason: ReleaseReason::Inactive,
                    cleanup_workspace: false,
                },
            ]
        );
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Released
        );
        assert_eq!(
            scheduler.orchestration_state("2"),
            OrchestrationState::Released
        );
    }

    #[test]
    fn reconciliation_releases_retry_queued_terminal_issues_before_due_dispatch() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        claim_and_start_run(
            &mut scheduler,
            issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1)),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            timestamp(10),
        );
        scheduler
            .finish_run("1", WorkerOutcome::succeeded(), timestamp(20))
            .unwrap();

        let outcomes = scheduler.reconcile_tracker_states(&[issue(
            "1",
            "OSYM-1",
            "Done",
            Some(1),
            timestamp(30),
        )]);

        assert_eq!(
            outcomes,
            vec![ReconciliationOutcome {
                issue_id: "1".into(),
                reason: ReleaseReason::Terminal,
                cleanup_workspace: true,
            }]
        );
        assert!(scheduler.due_retries(timestamp(21)).is_empty());
    }

    #[test]
    fn reconciliation_releases_claimed_only_issues_that_never_start_running() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let claimed = scheduler.claim_candidate_batch(&[
            issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1)),
            issue("2", "OSYM-2", "Todo", Some(1), timestamp(2)),
            issue("3", "OSYM-3", "Todo", Some(1), timestamp(3)),
        ]);

        assert_eq!(claimed.len(), 3);
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Claimed
        );
        assert_eq!(
            scheduler.orchestration_state("2"),
            OrchestrationState::Claimed
        );
        assert_eq!(
            scheduler.orchestration_state("3"),
            OrchestrationState::Claimed
        );

        let outcomes = scheduler.reconcile_tracker_states(&[
            issue("1", "OSYM-1", "Done", Some(1), timestamp(4)),
            issue("2", "OSYM-2", "Human Review", Some(1), timestamp(5)),
        ]);

        assert_eq!(
            outcomes,
            vec![
                ReconciliationOutcome {
                    issue_id: "1".into(),
                    reason: ReleaseReason::Terminal,
                    cleanup_workspace: true,
                },
                ReconciliationOutcome {
                    issue_id: "2".into(),
                    reason: ReleaseReason::Inactive,
                    cleanup_workspace: false,
                },
                ReconciliationOutcome {
                    issue_id: "3".into(),
                    reason: ReleaseReason::Missing,
                    cleanup_workspace: false,
                },
            ]
        );
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Released
        );
        assert_eq!(
            scheduler.orchestration_state("2"),
            OrchestrationState::Released
        );
        assert_eq!(
            scheduler.orchestration_state("3"),
            OrchestrationState::Released
        );
    }

    #[test]
    fn disabled_stall_detection_does_not_collapse_claim_grace_to_one_poll_tick() {
        let mut scheduler = SchedulerState::new(SchedulerConfig {
            max_concurrent_agents: 1,
            poll_interval_ms: 100_000,
            stall_timeout_ms: None,
            ..SchedulerConfig::default()
        });
        let claimed_issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
        let claimed = scheduler.claim_candidate_batch(std::slice::from_ref(&claimed_issue));

        assert_eq!(claimed.len(), 1);

        let first_outcomes = scheduler.reconcile_tracker_states(std::slice::from_ref(&issue(
            "1",
            "OSYM-1",
            "In Progress",
            Some(1),
            millis(100_000),
        )));
        let second_outcomes = scheduler.reconcile_tracker_states(std::slice::from_ref(&issue(
            "1",
            "OSYM-1",
            "In Progress",
            Some(1),
            millis(200_000),
        )));

        assert!(first_outcomes.is_empty());
        assert!(second_outcomes.is_empty());
        scheduler
            .start_run(
                claimed_issue,
                PathBuf::from("/tmp/OSYM-1"),
                None,
                millis(250_000),
            )
            .unwrap();
    }

    #[test]
    fn finish_run_preserves_global_rate_limits_when_other_run_has_none() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        claim_and_start_run(
            &mut scheduler,
            issue("1", "OSYM-1", "In Progress", Some(1), millis(0)),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            millis(0),
        );
        claim_and_start_run(
            &mut scheduler,
            issue("2", "OSYM-2", "In Progress", Some(1), millis(1_000)),
            PathBuf::from("/tmp/OSYM-2"),
            None,
            millis(1_000),
        );

        let mut fresher_session =
            RuntimeSession::default().with_event("message", "fresh", millis(10_000));
        let fresher_limits = RateLimitSnapshot {
            requests_remaining: Some(12),
            tokens_remaining: Some(2400),
            resets_at: Some(millis(30_000)),
        };
        fresher_session.rate_limits = Some(fresher_limits.clone());
        scheduler
            .update_runtime_session("1", fresher_session)
            .unwrap();

        scheduler
            .finish_run("2", WorkerOutcome::succeeded(), millis(11_000))
            .unwrap();

        assert_eq!(
            scheduler.snapshot(millis(12_000)).rate_limits,
            Some(fresher_limits)
        );
    }

    #[test]
    fn finish_run_preserves_newer_global_rate_limits_against_older_run_data() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        claim_and_start_run(
            &mut scheduler,
            issue("1", "OSYM-1", "In Progress", Some(1), millis(0)),
            PathBuf::from("/tmp/OSYM-1"),
            None,
            millis(0),
        );
        claim_and_start_run(
            &mut scheduler,
            issue("2", "OSYM-2", "In Progress", Some(1), millis(1_000)),
            PathBuf::from("/tmp/OSYM-2"),
            None,
            millis(1_000),
        );

        let mut fresher_session =
            RuntimeSession::default().with_event("message", "fresh", millis(10_000));
        let fresher_limits = RateLimitSnapshot {
            requests_remaining: Some(12),
            tokens_remaining: Some(2400),
            resets_at: Some(millis(30_000)),
        };
        fresher_session.rate_limits = Some(fresher_limits.clone());
        scheduler
            .update_runtime_session("1", fresher_session)
            .unwrap();

        let mut older_session =
            RuntimeSession::default().with_event("message", "older", millis(5_000));
        older_session.rate_limits = Some(RateLimitSnapshot {
            requests_remaining: Some(3),
            tokens_remaining: Some(600),
            resets_at: Some(millis(15_000)),
        });
        scheduler.running.get_mut("2").unwrap().run.session = older_session;

        scheduler
            .finish_run("2", WorkerOutcome::succeeded(), millis(11_000))
            .unwrap();

        assert_eq!(
            scheduler.snapshot(millis(12_000)).rate_limits,
            Some(fresher_limits)
        );
    }

    #[test]
    fn stall_detection_schedules_retry_from_last_activity() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        let issue = issue("1", "OSYM-1", "In Progress", Some(1), millis(0));
        claim_and_start_run(
            &mut scheduler,
            issue.clone(),
            PathBuf::from("/tmp/OSYM-1"),
            Some(1),
            millis(0),
        );
        scheduler
            .update_runtime_session(
                "1",
                RuntimeSession::default().with_event("message", "still going", millis(100_000)),
            )
            .unwrap();

        let retries = scheduler.reconcile_stalls(millis(450_001));

        assert_eq!(retries.len(), 1);
        assert_eq!(retries[0].attempt, 2);
        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::RetryQueued
        );
    }

    #[test]
    fn restart_recovery_restores_claimed_running_and_retry_state() {
        let mut scheduler = SchedulerState::new(SchedulerConfig::default());
        scheduler.recover_running(RunningIssue {
            issue: issue("1", "OSYM-1", "In Progress", Some(1), timestamp(1)),
            run: RunAttempt {
                issue_id: "1".into(),
                issue_identifier: "OSYM-1".into(),
                attempt: Some(1),
                workspace_path: PathBuf::from("/tmp/OSYM-1"),
                started_at: timestamp(2),
                session: RuntimeSession::default(),
            },
        });
        scheduler.recover_retry(RetryEntry {
            issue_id: "2".into(),
            identifier: "OSYM-2".into(),
            attempt: 3,
            due_at: timestamp(30),
            error: Some("retry me".into()),
        });

        let snapshot = scheduler.snapshot(timestamp(40));

        assert_eq!(
            scheduler.orchestration_state("1"),
            OrchestrationState::Running
        );
        assert_eq!(
            scheduler.orchestration_state("2"),
            OrchestrationState::RetryQueued
        );
        assert_eq!(
            snapshot.claimed_issue_ids,
            vec!["1".to_string(), "2".to_string()]
        );
        assert_eq!(snapshot.retry_queue.len(), 1);
        assert_eq!(snapshot.running.len(), 1);
    }
}
