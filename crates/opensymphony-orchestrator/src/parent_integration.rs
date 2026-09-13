use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, IssueId, ParentVerificationEvidence, TimestampMs,
};
use crate::opensymphony_workspace::redact_runtime_diagnostic;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MAX_TRANSITIONS: usize = 256;
const MAX_ATTEMPTS: usize = 64;
const MAX_LOG_BYTES: usize = 16 * 1024;
const MAX_COMMAND_BYTES: usize = 4 * 1024;
const MAX_COMMAND_ID_BYTES: usize = 256;
const MAX_REPAIR_FAILURE_BYTES: usize = 4 * 1024;
const MAX_RESOURCE_RECEIPTS: usize = 64;
const MAX_COMMAND_RECEIPTS: usize = MAX_RESOURCE_RECEIPTS / 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentIntegrationState {
    WaitingForChildren,
    WaitingForChildMerges,
    AcquiringChildLeases,
    PreparingIntegrationWorkspace,
    RefreshingRepositories,
    Integrating,
    Fixing {
        repository_id: CanonicalRepositoryId,
        repair_attempt: u64,
    },
    AwaitingFixReview {
        repository_id: CanonicalRepositoryId,
        repair_attempt: u64,
        pull_request_id: String,
    },
    AwaitingFixMerge {
        repository_id: CanonicalRepositoryId,
        repair_attempt: u64,
        pull_request_id: String,
    },
    RefreshingAfterFixes,
    FinalVerification,
    Finalizing,
    Completed,
    Blocked {
        reason: String,
    },
    Failed {
        reason: String,
    },
    Canceled {
        reason: String,
    },
}

impl ParentIntegrationState {
    pub fn terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed { .. } | Self::Canceled { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentRetryClassification {
    None,
    Retryable,
    CleanupThenRetry,
    OperatorAction,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentSideEffectIntent {
    pub kind: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentSideEffectReceipt {
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentTransition {
    pub state_version: u64,
    pub previous: ParentIntegrationState,
    pub next: ParentIntegrationState,
    pub reason: String,
    pub idempotency_key: String,
    pub input_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effect_intent: Option<ParentSideEffectIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_receipt: Option<ParentSideEffectReceipt>,
    pub retry_classification: ParentRetryClassification,
    pub occurred_at: TimestampMs,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentAttemptRoot {
    #[default]
    ParentRoot,
    CheckoutHandle(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentAttemptStatus {
    Running,
    Passed,
    Failed,
    TimedOut,
    Canceled,
    Indeterminate,
}

impl ParentAttemptStatus {
    fn terminal(self) -> bool {
        self != Self::Running
    }

    pub fn passed(self) -> bool {
        self == Self::Passed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentResourceStatus {
    Allocated,
    Released,
    Collision,
    CleanupFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentResourceReceipt {
    pub kind: String,
    pub identifier: String,
    pub status: ParentResourceStatus,
    pub occurred_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentCleanupStatus {
    Pending,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentCleanupReceipt {
    pub status: ParentCleanupStatus,
    pub occurred_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentCommandReceipt {
    pub command_id: String,
    /// Bounded redacted display for diagnostics.
    pub command: String,
    /// SHA-256 identity of the exact transient harness command.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command_hash: String,
    #[serde(default)]
    pub root: ParentAttemptRoot,
    pub started_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<TimestampMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentVerificationAttempt {
    pub id: String,
    pub name: String,
    pub idempotency_key: String,
    pub root: ParentAttemptRoot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub started_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<TimestampMs>,
    pub timeout_ms: u64,
    pub status: ParentAttemptStatus,
    #[serde(default)]
    pub commands: Vec<ParentCommandReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub bounded_log: String,
    #[serde(default)]
    pub log_truncated: bool,
    #[serde(default)]
    pub resources: Vec<ParentResourceReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<ParentCleanupReceipt>,
    pub input_version: String,
    #[serde(default)]
    pub verified_repository_commits: BTreeMap<CanonicalRepositoryId, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRepositoryTarget {
    pub repository_id: CanonicalRepositoryId,
    pub checkout_handle: String,
    #[serde(default)]
    pub relative_path: PathBuf,
    #[serde(default = "default_parent_target_branch")]
    pub target_branch: String,
    pub target_commit: String,
    #[serde(default)]
    pub instruction_path: PathBuf,
    #[serde(default)]
    pub instruction_hash: String,
    #[serde(default)]
    pub repair_policy: ParentRepairPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRepairPolicy {
    pub review_profile: String,
    pub review_provider: String,
    pub review_policy_generation: String,
    pub required_checks: bool,
    pub required_review: bool,
    pub merge_method: String,
}

fn default_parent_target_branch() -> String {
    String::new()
}

impl Default for ParentRepairPolicy {
    fn default() -> Self {
        Self {
            review_profile: "legacy-default".to_owned(),
            review_provider: "github".to_owned(),
            review_policy_generation: "schema-1-default".to_owned(),
            required_checks: true,
            required_review: true,
            merge_method: "merge".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentRepairStatus {
    PreparingBranch,
    Implementing,
    AwaitingPullRequest,
    AwaitingReview,
    ChangesRequested,
    AwaitingMerge,
    Refreshing,
    Completed,
    FailedChecks,
    ReviewRejected,
    ProviderUnavailable,
    ExternallyClosed,
    ForcePushed,
    MergeConflict,
}

impl ParentRepairStatus {
    pub fn resumable(self) -> bool {
        !matches!(self, Self::Completed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentProviderOperationKind {
    ReconcileBranch,
    CreateBranch,
    ReconcilePush,
    Push,
    ReconcilePullRequest,
    CreatePullRequest,
    ReconcileReview,
    RequestReview,
    ReconcileMerge,
    Merge,
    RefreshTarget,
}

impl ParentProviderOperationKind {
    fn prerequisite(self) -> Option<Self> {
        match self {
            Self::CreateBranch => Some(Self::ReconcileBranch),
            Self::Push => Some(Self::ReconcilePush),
            Self::CreatePullRequest => Some(Self::ReconcilePullRequest),
            Self::RequestReview => Some(Self::ReconcileReview),
            Self::Merge => Some(Self::ReconcileMerge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentProviderOperation {
    pub kind: ParentProviderOperationKind,
    pub idempotency_key: String,
    pub input_version: String,
    pub intended_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ParentSideEffectReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRepairAttempt {
    pub id: String,
    pub number: u64,
    pub repository_id: CanonicalRepositoryId,
    pub checkout_handle: String,
    pub target_branch: String,
    pub target_commit: String,
    pub instruction_path: PathBuf,
    pub instruction_hash: String,
    pub branch: String,
    pub lease_id: String,
    pub policy: ParentRepairPolicy,
    #[serde(default)]
    pub implementation_completed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_advancement_failure: Option<ParentRepairAdvancementFailure>,
    pub status: ParentRepairStatus,
    pub created_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_result_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_target_commit: Option<String>,
    #[serde(default)]
    pub requested_change_count: u32,
    #[serde(default)]
    pub operations: Vec<ParentProviderOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRepairAdvancementFailure {
    pub occurred_at: TimestampMs,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentRepairProviderSnapshot {
    pub pull_request_id: Option<String>,
    pub pull_request_url: Option<String>,
    pub head_commit: Option<String>,
    pub review_head_commit: Option<String>,
    /// Provider cursor observed before a review request is written. GitHub
    /// uses the highest issue-comment ID so a crash after an exact trigger can
    /// reconcile the write without relying on second-precision timestamps.
    pub review_request_cursor: Option<String>,
    pub open: bool,
    pub checks_passed: bool,
    pub checks_failed: bool,
    pub review_approved: bool,
    pub review_rejected: bool,
    pub changes_requested: bool,
    pub mergeable: bool,
    pub merge_conflict: bool,
    pub merged: bool,
    pub merge_result_commit: Option<String>,
    pub target_contains_merge_result: bool,
    pub provider_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentFinalEvidence {
    pub attempt_id: String,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub input_version: String,
    pub repository_commits: BTreeMap<CanonicalRepositoryId, String>,
    pub recorded_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentIntegrationController {
    pub parent_id: IssueId,
    pub hierarchy_generation: u64,
    pub state_version: u64,
    pub state: ParentIntegrationState,
    // Transition history is bounded, so admission identity must survive pruning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_input_version: Option<String>,
    #[serde(default)]
    pub next_attempt_sequence: u64,
    #[serde(default)]
    pub next_repair_sequence: u64,
    #[serde(default)]
    pub transitions: Vec<ParentTransition>,
    #[serde(default)]
    pub targets: BTreeMap<CanonicalRepositoryId, ParentRepositoryTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub attempts: Vec<ParentVerificationAttempt>,
    #[serde(default)]
    pub repair_attempts: Vec<ParentRepairAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_evidence: Option<ParentFinalEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParentIntegrationError {
    #[error("parent controller generation must be greater than zero")]
    InvalidGeneration,
    #[error("invalid parent transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: Box<ParentIntegrationState>,
        to: Box<ParentIntegrationState>,
    },
    #[error("idempotency key `{0}` was reused with different transition facts")]
    IdempotencyConflict(String),
    #[error("parent controller is bound to conversation `{expected}`, not `{actual}`")]
    ConversationMismatch { expected: String, actual: String },
    #[error("checkout handle `{0}` is not present in the verified parent target map")]
    UnknownCheckoutHandle(String),
    #[error("verification attempt `{0}` was not found")]
    UnknownAttempt(String),
    #[error("repair attempt `{0}` was not found")]
    UnknownRepairAttempt(String),
    #[error("repository `{0}` is not present in the verified parent target map")]
    UnknownRepairRepository(String),
    #[error("repair attempt `{0}` does not match its verified checkout target")]
    RepairTargetMismatch(String),
    #[error("provider operation `{operation:?}` requires a completed `{prerequisite:?}` lookup")]
    ProviderReconciliationRequired {
        operation: ParentProviderOperationKind,
        prerequisite: ParentProviderOperationKind,
    },
    #[error("verification attempt `{0}` is already terminal")]
    AttemptAlreadyTerminal(String),
    #[error("verification attempt `{0}` cannot start before cleanup and baseline verification")]
    AttemptNotReady(String),
    #[error("resource `{kind}:{identifier}` is already allocated")]
    ResourceCollision { kind: String, identifier: String },
    #[error("final verification requires a passed attempt for the current repository targets")]
    FinalVerificationIncomplete,
    #[error("parent verification evidence is invalid: {0}")]
    InvalidVerificationEvidence(String),
}

impl ParentIntegrationController {
    pub fn new(
        parent_id: IssueId,
        hierarchy_generation: u64,
    ) -> Result<Self, ParentIntegrationError> {
        if hierarchy_generation == 0 {
            return Err(ParentIntegrationError::InvalidGeneration);
        }
        Ok(Self {
            parent_id,
            hierarchy_generation,
            state_version: 0,
            state: ParentIntegrationState::WaitingForChildren,
            admission_input_version: None,
            next_attempt_sequence: 0,
            next_repair_sequence: 0,
            transitions: Vec::new(),
            targets: BTreeMap::new(),
            conversation_id: None,
            attempts: Vec::new(),
            repair_attempts: Vec::new(),
            final_evidence: None,
        })
    }

    pub fn migrate_legacy_repair_target(
        &mut self,
        current: ParentRepositoryTarget,
    ) -> Result<bool, ParentIntegrationError> {
        let existing = self
            .targets
            .get_mut(&current.repository_id)
            .ok_or_else(|| {
                ParentIntegrationError::UnknownRepairRepository(current.repository_id.to_string())
            })?;
        let legacy = existing.target_branch.is_empty()
            && existing.repair_policy == ParentRepairPolicy::default();
        if !legacy {
            return Ok(false);
        }
        if existing.checkout_handle != current.checkout_handle
            || existing.relative_path != current.relative_path
            || existing.target_commit != current.target_commit
            || (!existing.instruction_path.as_os_str().is_empty()
                && existing.instruction_path != current.instruction_path)
            || (!existing.instruction_hash.is_empty()
                && existing.instruction_hash != current.instruction_hash)
        {
            return Err(ParentIntegrationError::RepairTargetMismatch(
                current.repository_id.to_string(),
            ));
        }
        *existing = current;
        Ok(true)
    }

    pub fn admit(
        &mut self,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        if let Some(admitted_input) = self.admission_input_version.as_deref() {
            return if admitted_input == input_version {
                Ok(())
            } else {
                Err(ParentIntegrationError::IdempotencyConflict(format!(
                    "parent:{}:{}:admission",
                    self.parent_id, self.hierarchy_generation
                )))
            };
        }
        let previous = self.clone();
        let result = (|| {
            self.transition(
                ParentIntegrationState::WaitingForChildMerges,
                "required children reached terminal orchestrator outcomes",
                format!(
                    "parent:{}:{}:children",
                    self.parent_id, self.hierarchy_generation
                ),
                input_version,
                None,
                Some(receipt("verified", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
            self.transition(
                ParentIntegrationState::AcquiringChildLeases,
                "provider merge evidence is complete",
                format!(
                    "parent:{}:{}:merges",
                    self.parent_id, self.hierarchy_generation
                ),
                input_version,
                None,
                Some(receipt("verified", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
            self.transition(
                ParentIntegrationState::PreparingIntegrationWorkspace,
                "generation-bound descendant leases were acquired",
                format!(
                    "parent:{}:{}:leases",
                    self.parent_id, self.hierarchy_generation
                ),
                input_version,
                Some(intent(
                    "acquire_descendant_leases",
                    format!(
                        "parent:{}:{}:leases",
                        self.parent_id, self.hierarchy_generation
                    ),
                )),
                Some(receipt("succeeded", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
            Ok(())
        })();
        if let Err(error) = result {
            *self = previous;
            return Err(error);
        }
        self.admission_input_version = Some(input_version.to_owned());
        Ok(())
    }

    pub fn record_workspace_prepared(
        &mut self,
        targets: impl IntoIterator<Item = ParentRepositoryTarget>,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let targets = targets
            .into_iter()
            .map(|target| (target.repository_id.clone(), target))
            .collect::<BTreeMap<_, _>>();
        self.targets = targets;
        if self.state == ParentIntegrationState::PreparingIntegrationWorkspace {
            self.transition(
                ParentIntegrationState::RefreshingRepositories,
                "parent integration root and checkout map were verified",
                format!(
                    "parent:{}:{}:workspace",
                    self.parent_id, self.hierarchy_generation
                ),
                input_version,
                Some(intent(
                    "prepare_parent_workspace",
                    format!(
                        "parent:{}:{}:workspace",
                        self.parent_id, self.hierarchy_generation
                    ),
                )),
                Some(receipt("succeeded", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
        }
        self.record_baseline_verified(input_version, occurred_at)
    }

    pub fn record_baseline_verified(
        &mut self,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        if !self.cleanup_complete() {
            return Err(ParentIntegrationError::AttemptNotReady(
                "baseline-refresh".to_owned(),
            ));
        }
        let refresh_cycle = self
            .attempts
            .last()
            .map_or_else(|| "initial".to_owned(), |attempt| attempt.id.clone());
        self.transition(
            ParentIntegrationState::Integrating,
            "all parent integration checkouts retain their recorded target commits as verified baselines",
            format!(
                "parent:{}:{}:refresh:{refresh_cycle}:{input_version}",
                self.parent_id, self.hierarchy_generation,
            ),
            input_version,
            Some(intent(
                "refresh_verified_baseline",
                format!(
                    "parent:{}:{}:refresh:{refresh_cycle}:{input_version}",
                    self.parent_id, self.hierarchy_generation,
                ),
            )),
            Some(receipt("succeeded", None)),
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start_attempt(
        &mut self,
        name: impl Into<String>,
        idempotency_key: impl Into<String>,
        root: ParentAttemptRoot,
        conversation_id: impl Into<String>,
        timeout_ms: u64,
        input_version: impl Into<String>,
        started_at: TimestampMs,
    ) -> Result<String, ParentIntegrationError> {
        let conversation_id = conversation_id.into();
        let id = self.start_attempt_intent(
            name,
            idempotency_key,
            root,
            timeout_ms,
            input_version,
            started_at,
        )?;
        self.attach_attempt_conversation(&id, &conversation_id)?;
        Ok(id)
    }

    pub fn start_attempt_intent(
        &mut self,
        name: impl Into<String>,
        idempotency_key: impl Into<String>,
        root: ParentAttemptRoot,
        timeout_ms: u64,
        input_version: impl Into<String>,
        started_at: TimestampMs,
    ) -> Result<String, ParentIntegrationError> {
        let name = name.into();
        let idempotency_key = idempotency_key.into();
        let input_version = input_version.into();
        if let Some(existing) = self
            .attempts
            .iter()
            .find(|attempt| attempt.idempotency_key == idempotency_key)
        {
            if existing.name == name
                && existing.root == root
                && existing.timeout_ms == timeout_ms
                && existing.input_version == input_version
            {
                return Ok(existing.id.clone());
            }
            return Err(ParentIntegrationError::IdempotencyConflict(idempotency_key));
        }
        if !matches!(
            self.state,
            ParentIntegrationState::Integrating
                | ParentIntegrationState::FinalVerification
                | ParentIntegrationState::Fixing { .. }
        ) || !self.cleanup_complete()
        {
            return Err(ParentIntegrationError::AttemptNotReady(name));
        }
        if let ParentAttemptRoot::CheckoutHandle(handle) = &root
            && !self
                .targets
                .values()
                .any(|target| target.checkout_handle == *handle)
        {
            return Err(ParentIntegrationError::UnknownCheckoutHandle(
                handle.clone(),
            ));
        }
        if self.attempts.len() >= MAX_ATTEMPTS {
            let Some(removable) = self
                .attempts
                .iter()
                .position(|attempt| attempt.status.terminal())
            else {
                return Err(ParentIntegrationError::InvalidVerificationEvidence(
                    "nonterminal parent attempt receipts exceed their bound".to_owned(),
                ));
            };
            self.attempts.remove(removable);
        }
        self.next_attempt_sequence = self.next_attempt_sequence.saturating_add(1);
        let id = format!("parent-attempt-{}", self.next_attempt_sequence);
        self.attempts.push(ParentVerificationAttempt {
            id: id.clone(),
            name,
            idempotency_key,
            root,
            conversation_id: None,
            started_at,
            finished_at: None,
            timeout_ms,
            status: ParentAttemptStatus::Running,
            commands: Vec::new(),
            exit_code: None,
            bounded_log: String::new(),
            log_truncated: false,
            resources: Vec::new(),
            cleanup: None,
            input_version,
            verified_repository_commits: BTreeMap::new(),
        });
        Ok(id)
    }

    pub fn attach_attempt_conversation(
        &mut self,
        attempt_id: &str,
        conversation_id: &str,
    ) -> Result<(), ParentIntegrationError> {
        if let Some(expected) = &self.conversation_id {
            if expected != conversation_id {
                return Err(ParentIntegrationError::ConversationMismatch {
                    expected: expected.clone(),
                    actual: conversation_id.to_owned(),
                });
            }
        } else {
            self.conversation_id = Some(conversation_id.to_owned());
        }
        let attempt = self.attempt_mut(attempt_id)?;
        match &attempt.conversation_id {
            Some(expected) if expected != conversation_id => {
                Err(ParentIntegrationError::ConversationMismatch {
                    expected: expected.clone(),
                    actual: conversation_id.to_owned(),
                })
            }
            Some(_) => Ok(()),
            None => {
                attempt.conversation_id = Some(conversation_id.to_owned());
                Ok(())
            }
        }
    }

    pub fn record_verification_evidence(
        &mut self,
        attempt_id: &str,
        evidence: &ParentVerificationEvidence,
    ) -> Result<bool, ParentIntegrationError> {
        if evidence.schema_version != 1 {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "unsupported schema version".to_owned(),
            ));
        }
        let expected_commits = self
            .targets
            .iter()
            .map(|(repository_id, target)| (repository_id.clone(), target.target_commit.clone()))
            .collect::<BTreeMap<_, _>>();
        if evidence.repository_commits != expected_commits {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "repository commits do not match the current target map".to_owned(),
            ));
        }
        let attempt = self.attempt_mut(attempt_id)?;
        let selected_command = attempt.commands.iter().rev().find(|command| {
            !evidence.command_hash.is_empty()
                && command.command_hash == evidence.command_hash
                && match &command.root {
                    ParentAttemptRoot::ParentRoot => evidence.root == "parent_root",
                    ParentAttemptRoot::CheckoutHandle(handle) => evidence.root == *handle,
                }
                && command.finished_at.is_some()
                && command.exit_code.is_some()
        });
        let selected_exit_code = selected_command.and_then(|command| command.exit_code);
        if evidence.command_hash.is_empty() || selected_exit_code.is_none() {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "selected command does not match a completed runtime command".to_owned(),
            ));
        }
        attempt.exit_code = selected_exit_code;
        attempt.verified_repository_commits = evidence.repository_commits.clone();
        Ok(selected_exit_code == Some(0)
            && attempt
                .cleanup
                .as_ref()
                .is_some_and(|cleanup| cleanup.status == ParentCleanupStatus::Succeeded)
            && active_resource_keys(attempt).is_empty())
    }

    pub fn observe_command_started(
        &mut self,
        attempt_id: &str,
        command_id: &str,
        command: &str,
        root: ParentAttemptRoot,
        observed_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        if command_id.trim().is_empty() || command.trim().is_empty() {
            return Ok(());
        }
        if command_id.len() > MAX_COMMAND_ID_BYTES || command.len() > MAX_COMMAND_BYTES {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "foreground command identity exceeds its evidence bound".to_owned(),
            ));
        }
        let attempt = self.attempt_mut(attempt_id)?;
        if observed_at < attempt.started_at {
            return Ok(());
        }
        let command_hash = parent_command_identity(command);
        let command = redact_runtime_diagnostic(command);
        if attempt.status != ParentAttemptStatus::Running {
            return Err(ParentIntegrationError::AttemptAlreadyTerminal(
                attempt_id.to_owned(),
            ));
        }
        if let Some(existing) = attempt
            .commands
            .iter()
            .find(|receipt| receipt.command_id == command_id)
        {
            return if existing.command_hash == command_hash && existing.root == root {
                Ok(())
            } else {
                Err(ParentIntegrationError::InvalidVerificationEvidence(
                    "a command id was replayed with different command text".to_owned(),
                ))
            };
        }
        if observed_at.as_u64()
            >= attempt
                .started_at
                .as_u64()
                .saturating_add(attempt.timeout_ms)
        {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "foreground command started after the attempt deadline".to_owned(),
            ));
        }
        if let Some(active_command_id) = attempt
            .commands
            .iter()
            .rev()
            .find(|receipt| receipt.finished_at.is_none())
            .map(|receipt| receipt.command_id.as_str())
        {
            return Err(ParentIntegrationError::ResourceCollision {
                kind: "foreground_process".to_owned(),
                identifier: active_command_id.to_owned(),
            });
        }
        if active_resource_keys(attempt)
            .into_iter()
            .any(|(kind, identifier)| kind == "foreground_process" && identifier == command_id)
        {
            return Err(ParentIntegrationError::ResourceCollision {
                kind: "foreground_process".to_owned(),
                identifier: command_id.to_owned(),
            });
        }
        if attempt.commands.len() >= MAX_COMMAND_RECEIPTS {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "attempt command receipts exceed their bound".to_owned(),
            ));
        }
        if attempt.resources.len() >= MAX_RESOURCE_RECEIPTS.saturating_sub(1) {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "attempt resource receipts cannot reserve a teardown receipt".to_owned(),
            ));
        }
        attempt.commands.push(ParentCommandReceipt {
            command_id: command_id.to_owned(),
            command,
            command_hash,
            root,
            started_at: observed_at,
            finished_at: None,
            exit_code: None,
        });
        attempt.exit_code = None;
        attempt.cleanup = Some(ParentCleanupReceipt {
            status: ParentCleanupStatus::Pending,
            occurred_at: observed_at,
            detail: Some("foreground command is running".to_owned()),
        });
        self.allocate_resource(attempt_id, "foreground_process", command_id, observed_at)
    }

    pub fn observe_command_finished(
        &mut self,
        attempt_id: &str,
        command_id: &str,
        exit_code: i32,
        redacted_output: Option<&str>,
        observed_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let attempt = self.attempt_mut(attempt_id)?;
        if observed_at < attempt.started_at {
            return Ok(());
        }
        let Some(command_index) = attempt
            .commands
            .iter()
            .rposition(|command| command.command_id == command_id)
        else {
            return Ok(());
        };
        if observed_at < attempt.commands[command_index].started_at {
            return Ok(());
        }
        if let Some(finished_at) = attempt.commands[command_index].finished_at {
            return if attempt.commands[command_index].exit_code == Some(exit_code) {
                Ok(())
            } else {
                Err(ParentIntegrationError::InvalidVerificationEvidence(
                    format!(
                        "command {command_id} completion replay changed its exit code after {finished_at:?}"
                    ),
                ))
            };
        }
        if observed_at.as_u64()
            >= attempt
                .started_at
                .as_u64()
                .saturating_add(attempt.timeout_ms)
        {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "foreground command completed after the attempt deadline".to_owned(),
            ));
        }
        attempt.commands[command_index].finished_at = Some(observed_at);
        attempt.commands[command_index].exit_code = Some(exit_code);
        attempt.exit_code = Some(exit_code);
        attempt.cleanup = Some(ParentCleanupReceipt {
            status: ParentCleanupStatus::Succeeded,
            occurred_at: observed_at,
            detail: Some("harness reported foreground process completion".to_owned()),
        });
        if let Some(output) = redacted_output {
            self.append_log(attempt_id, output)?;
        }
        self.release_resource(
            attempt_id,
            "foreground_process",
            command_id,
            true,
            Some("harness reported foreground process completion".to_owned()),
            observed_at,
        )
    }

    pub fn observe_harness_stopped(
        &mut self,
        attempt_id: &str,
        detail: &str,
        observed_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let foreground_processes = {
            let attempt = self.attempt_mut(attempt_id)?;
            active_resource_keys(attempt)
                .into_iter()
                .filter_map(|(kind, identifier)| {
                    (kind == "foreground_process").then_some(identifier)
                })
                .collect::<Vec<_>>()
        };
        for identifier in foreground_processes {
            self.release_resource(
                attempt_id,
                "foreground_process",
                &identifier,
                true,
                Some(detail.to_owned()),
                observed_at,
            )?;
        }
        let attempt = self.attempt_mut(attempt_id)?;
        let remaining = active_resource_keys(attempt);
        attempt.cleanup = Some(ParentCleanupReceipt {
            status: if remaining.is_empty() {
                ParentCleanupStatus::Succeeded
            } else {
                ParentCleanupStatus::Pending
            },
            occurred_at: observed_at,
            detail: Some(if remaining.is_empty() {
                detail.to_owned()
            } else {
                format!(
                    "{detail}; {} non-process resource(s) still require cleanup",
                    remaining.len()
                )
            }),
        });
        Ok(())
    }

    pub fn append_log(
        &mut self,
        attempt_id: &str,
        redacted: &str,
    ) -> Result<(), ParentIntegrationError> {
        let attempt = self.attempt_mut(attempt_id)?;
        if attempt.log_truncated || redacted.is_empty() {
            return Ok(());
        }
        if !attempt.bounded_log.is_empty() {
            attempt.bounded_log.push('\n');
        }
        let remaining = MAX_LOG_BYTES.saturating_sub(attempt.bounded_log.len());
        if redacted.len() <= remaining {
            attempt.bounded_log.push_str(redacted);
        } else {
            let mut boundary = remaining.min(redacted.len());
            while boundary > 0 && !redacted.is_char_boundary(boundary) {
                boundary -= 1;
            }
            attempt.bounded_log.push_str(&redacted[..boundary]);
            attempt.log_truncated = true;
        }
        Ok(())
    }

    pub fn allocate_resource(
        &mut self,
        attempt_id: &str,
        kind: impl Into<String>,
        identifier: impl Into<String>,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let kind = kind.into();
        let identifier = identifier.into();
        let collision = self.attempts.iter().any(|attempt| {
            active_resource_keys(attempt)
                .into_iter()
                .any(|(active_kind, active_identifier)| {
                    active_kind == kind && active_identifier == identifier
                })
        });
        if collision {
            let attempt = self.attempt_mut(attempt_id)?;
            if attempt.resources.len() < MAX_RESOURCE_RECEIPTS.saturating_sub(1) {
                attempt.resources.push(ParentResourceReceipt {
                    kind: kind.clone(),
                    identifier: identifier.clone(),
                    status: ParentResourceStatus::Collision,
                    occurred_at,
                    detail: Some("resource is owned by another parent attempt".to_owned()),
                });
            }
            return Err(ParentIntegrationError::ResourceCollision { kind, identifier });
        }
        let attempt = self.attempt_mut(attempt_id)?;
        if attempt.resources.len() >= MAX_RESOURCE_RECEIPTS.saturating_sub(1) {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "attempt resource receipts cannot reserve a teardown receipt".to_owned(),
            ));
        }
        attempt.resources.push(ParentResourceReceipt {
            kind,
            identifier,
            status: ParentResourceStatus::Allocated,
            occurred_at,
            detail: None,
        });
        Ok(())
    }

    pub fn release_resource(
        &mut self,
        attempt_id: &str,
        kind: &str,
        identifier: &str,
        succeeded: bool,
        detail: Option<String>,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let attempt = self.attempt_mut(attempt_id)?;
        if attempt.resources.len() >= MAX_RESOURCE_RECEIPTS {
            return Err(ParentIntegrationError::InvalidVerificationEvidence(
                "attempt resource receipts exceed their bound".to_owned(),
            ));
        }
        attempt.resources.push(ParentResourceReceipt {
            kind: kind.to_owned(),
            identifier: identifier.to_owned(),
            status: if succeeded {
                ParentResourceStatus::Released
            } else {
                ParentResourceStatus::CleanupFailed
            },
            occurred_at,
            detail,
        });
        Ok(())
    }

    pub fn finish_attempt(
        &mut self,
        attempt_id: &str,
        status: ParentAttemptStatus,
        exit_code: Option<i32>,
        cleanup: ParentCleanupReceipt,
        finished_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        if !status.terminal() {
            return Err(ParentIntegrationError::AttemptAlreadyTerminal(
                attempt_id.to_owned(),
            ));
        }
        let attempt = self.attempt_mut(attempt_id)?;
        if attempt.status.terminal() {
            return Err(ParentIntegrationError::AttemptAlreadyTerminal(
                attempt_id.to_owned(),
            ));
        }
        attempt.status = status;
        attempt.exit_code = exit_code;
        attempt.finished_at = Some(finished_at);
        attempt.cleanup = Some(cleanup);
        Ok(())
    }

    pub fn prepare_retry(
        &mut self,
        attempt_id: &str,
        reason: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let attempt = self
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .ok_or_else(|| ParentIntegrationError::UnknownAttempt(attempt_id.to_owned()))?;
        let input_version = attempt.input_version.clone();
        let passed = attempt.status.passed();
        let cleanup = attempt.cleanup.clone();
        let attempt_key = attempt.idempotency_key.clone();
        self.transition(
            ParentIntegrationState::RefreshingRepositories,
            reason,
            format!("{}:retry", attempt_key),
            &input_version,
            Some(intent(
                "cleanup_and_refresh",
                format!("{}:retry", attempt_key),
            )),
            cleanup.as_ref().map(|cleanup| {
                receipt(
                    match cleanup.status {
                        ParentCleanupStatus::Succeeded => "succeeded",
                        ParentCleanupStatus::Pending => "pending",
                        ParentCleanupStatus::Failed => "failed",
                    },
                    cleanup.detail.clone(),
                )
            }),
            if passed {
                ParentRetryClassification::Retryable
            } else {
                ParentRetryClassification::CleanupThenRetry
            },
            occurred_at,
        )?;
        Ok(())
    }

    pub fn reconcile_restart(
        &mut self,
        harness_reconciled: bool,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let Some(index) = self
            .attempts
            .iter()
            .rposition(|attempt| attempt.status == ParentAttemptStatus::Running)
        else {
            return Ok(());
        };
        if harness_reconciled {
            return Ok(());
        }
        let attempt_id = self.attempts[index].id.clone();
        let launch_never_attached = self.attempts[index].conversation_id.is_none()
            && self.attempts[index].commands.is_empty()
            && active_resource_keys(&self.attempts[index]).is_empty();
        self.attempts[index].status = ParentAttemptStatus::Indeterminate;
        self.attempts[index].finished_at = Some(occurred_at);
        self.attempts[index].cleanup = Some(ParentCleanupReceipt {
            status: if launch_never_attached {
                ParentCleanupStatus::Succeeded
            } else {
                ParentCleanupStatus::Pending
            },
            occurred_at,
            detail: Some(if launch_never_attached {
                "attempt intent persisted before worker launch; no conversation, command, or resource was attached".to_owned()
            } else {
                "harness state and attempt-owned teardown were unavailable after restart".to_owned()
            }),
        });
        if self.repair_implementation_in_progress() {
            Ok(())
        } else {
            self.prepare_retry(
                &attempt_id,
                "nonterminal parent attempt became indeterminate after restart",
                occurred_at,
            )
        }
    }

    pub fn reconcile_terminal_run_after_restart(
        &mut self,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let Some(index) = self
            .attempts
            .iter()
            .rposition(|attempt| attempt.status == ParentAttemptStatus::Running)
        else {
            return Ok(());
        };
        let attempt_id = self.attempts[index].id.clone();
        let resources_released = active_resource_keys(&self.attempts[index]).is_empty();
        self.attempts[index].status = ParentAttemptStatus::Indeterminate;
        self.attempts[index].finished_at = Some(occurred_at);
        self.attempts[index].cleanup = Some(ParentCleanupReceipt {
            status: if resources_released {
                ParentCleanupStatus::Succeeded
            } else {
                ParentCleanupStatus::Pending
            },
            occurred_at,
            detail: Some(if resources_released {
                "durable run manifest recorded a terminal harness state before controller outcome persistence".to_owned()
            } else {
                "durable run manifest is terminal but named attempt resources still require cleanup"
                    .to_owned()
            }),
        });
        if self.repair_implementation_in_progress() {
            Ok(())
        } else {
            self.prepare_retry(
                &attempt_id,
                "terminal harness outcome was not durably applied before restart",
                occurred_at,
            )
        }
    }

    fn repair_implementation_in_progress(&self) -> bool {
        matches!(self.state, ParentIntegrationState::Fixing { .. })
            && self.repair_attempts.iter().any(|repair| {
                matches!(
                    repair.status,
                    ParentRepairStatus::Implementing | ParentRepairStatus::ChangesRequested
                ) && !repair.implementation_completed
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn begin_repair(
        &mut self,
        idempotency_key: &str,
        repository_id: CanonicalRepositoryId,
        checkout_handle: &str,
        target_commit: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<String, ParentIntegrationError> {
        if let Some(existing) = self.repair_attempts.iter().find(|attempt| {
            attempt
                .operations
                .first()
                .is_some_and(|operation| operation.idempotency_key == idempotency_key)
        }) {
            return if existing.repository_id == repository_id
                && existing.checkout_handle == checkout_handle
                && existing.target_commit == target_commit
            {
                Ok(existing.id.clone())
            } else {
                Err(ParentIntegrationError::IdempotencyConflict(
                    idempotency_key.to_owned(),
                ))
            };
        }
        if self.state != ParentIntegrationState::Integrating {
            return Err(ParentIntegrationError::InvalidTransition {
                from: Box::new(self.state.clone()),
                to: Box::new(ParentIntegrationState::Fixing {
                    repository_id,
                    repair_attempt: self.next_repair_sequence.saturating_add(1),
                }),
            });
        }
        let target = self.targets.get(&repository_id).ok_or_else(|| {
            ParentIntegrationError::UnknownRepairRepository(repository_id.to_string())
        })?;
        if target.checkout_handle != checkout_handle || target.target_commit != target_commit {
            return Err(ParentIntegrationError::RepairTargetMismatch(
                repository_id.to_string(),
            ));
        }
        let instruction_path = target.instruction_path.clone();
        let instruction_hash = target.instruction_hash.clone();
        let target_branch = target.target_branch.clone();
        let policy = target.repair_policy.clone();
        self.next_repair_sequence = self.next_repair_sequence.saturating_add(1);
        let number = self.next_repair_sequence;
        let id = format!("parent-repair-{number}");
        let branch_parent = self
            .parent_id
            .as_str()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_owned();
        let branch = format!(
            "fix/{branch_parent}-g{}-repair-{number}",
            self.hierarchy_generation
        );
        let lease_id = format!(
            "repair:{}:{}:{}",
            self.parent_id, self.hierarchy_generation, number
        );
        self.transition(
            ParentIntegrationState::Fixing {
                repository_id: repository_id.clone(),
                repair_attempt: number,
            },
            "integration defect requires a repository repair",
            idempotency_key,
            input_version,
            Some(intent("acquire_repair_lease", idempotency_key)),
            None,
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        self.repair_attempts.push(ParentRepairAttempt {
            id: id.clone(),
            number,
            repository_id,
            checkout_handle: checkout_handle.to_owned(),
            target_branch,
            target_commit: target_commit.to_owned(),
            instruction_path,
            instruction_hash,
            branch,
            lease_id,
            policy,
            implementation_completed: false,
            last_advancement_failure: None,
            status: ParentRepairStatus::PreparingBranch,
            created_at: occurred_at,
            pushed_commit: None,
            pull_request_id: None,
            pull_request_url: None,
            merge_result_commit: None,
            refreshed_target_commit: None,
            requested_change_count: 0,
            operations: vec![ParentProviderOperation {
                kind: ParentProviderOperationKind::ReconcileBranch,
                idempotency_key: idempotency_key.to_owned(),
                input_version: input_version.to_owned(),
                intended_at: occurred_at,
                receipt: None,
                completed_at: None,
            }],
        });
        Ok(id)
    }

    pub fn record_repair_advancement_failure(
        &mut self,
        repair_id: &str,
        detail: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let mut boundary = detail.len().min(MAX_REPAIR_FAILURE_BYTES);
        while boundary > 0 && !detail.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.repair_mut(repair_id)?.last_advancement_failure =
            Some(ParentRepairAdvancementFailure {
                occurred_at,
                detail: detail[..boundary].to_owned(),
            });
        Ok(())
    }

    pub fn begin_provider_operation(
        &mut self,
        repair_id: &str,
        kind: ParentProviderOperationKind,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<String, ParentIntegrationError> {
        let repair = self.repair_mut(repair_id)?;
        if let Some(prerequisite) = kind.prerequisite()
            && !repair
                .operations
                .iter()
                .any(|operation| operation.kind == prerequisite && operation.receipt.is_some())
        {
            return Err(ParentIntegrationError::ProviderReconciliationRequired {
                operation: kind,
                prerequisite,
            });
        }
        if let Some(existing) = repair
            .operations
            .iter()
            .rev()
            .find(|operation| operation.kind == kind && operation.receipt.is_none())
        {
            return if existing.input_version == input_version {
                Ok(existing.idempotency_key.clone())
            } else {
                Err(ParentIntegrationError::IdempotencyConflict(
                    existing.idempotency_key.clone(),
                ))
            };
        }
        let sequence = repair
            .operations
            .iter()
            .filter(|operation| operation.kind == kind)
            .count()
            .saturating_add(1);
        let key = format!("{}:{kind:?}:{sequence}", repair.id).to_ascii_lowercase();
        repair.operations.push(ParentProviderOperation {
            kind,
            idempotency_key: key.clone(),
            input_version: input_version.to_owned(),
            intended_at: occurred_at,
            receipt: None,
            completed_at: None,
        });
        Ok(key)
    }

    pub fn record_provider_operation(
        &mut self,
        repair_id: &str,
        idempotency_key: &str,
        status: &str,
        detail: Option<String>,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let operation = self
            .repair_mut(repair_id)?
            .operations
            .iter_mut()
            .find(|operation| operation.idempotency_key == idempotency_key)
            .ok_or_else(|| {
                ParentIntegrationError::IdempotencyConflict(idempotency_key.to_owned())
            })?;
        let next = receipt(status, detail);
        if let Some(existing) = &operation.receipt {
            return if existing == &next {
                Ok(())
            } else {
                Err(ParentIntegrationError::IdempotencyConflict(
                    idempotency_key.to_owned(),
                ))
            };
        }
        operation.receipt = Some(next);
        operation.completed_at = Some(occurred_at);
        Ok(())
    }

    pub fn record_repair_branch_ready(
        &mut self,
        repair_id: &str,
    ) -> Result<(), ParentIntegrationError> {
        self.repair_mut(repair_id)?.status = ParentRepairStatus::Implementing;
        Ok(())
    }

    pub fn record_repair_implementation_completed(
        &mut self,
        repair_id: &str,
    ) -> Result<(), ParentIntegrationError> {
        let repair = self.repair_mut(repair_id)?;
        if !matches!(
            repair.status,
            ParentRepairStatus::Implementing | ParentRepairStatus::ChangesRequested
        ) {
            return Err(ParentIntegrationError::RepairTargetMismatch(
                repair_id.to_owned(),
            ));
        }
        repair.status = ParentRepairStatus::Implementing;
        repair.implementation_completed = true;
        Ok(())
    }

    pub fn record_repair_push(
        &mut self,
        repair_id: &str,
        commit: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let was_fixing = matches!(self.state, ParentIntegrationState::Fixing { .. });
        let review_state = {
            let repair = self.repair_mut(repair_id)?;
            let unchanged = repair.pushed_commit.as_deref() == Some(commit);
            repair.pushed_commit = Some(commit.to_owned());
            repair.status = if repair.pull_request_id.is_some() {
                ParentRepairStatus::AwaitingReview
            } else {
                ParentRepairStatus::AwaitingPullRequest
            };
            (!unchanged || was_fixing)
                .then(|| repair.pull_request_id.clone())
                .flatten()
                .map(
                    |pull_request_id| ParentIntegrationState::AwaitingFixReview {
                        repository_id: repair.repository_id.clone(),
                        repair_attempt: repair.number,
                        pull_request_id,
                    },
                )
        };
        if let Some(review_state) = review_state {
            self.transition(
                review_state,
                "requested changes were pushed to the existing repair pull request",
                format!("{repair_id}:push:{commit}"),
                input_version,
                Some(intent("push_repair", format!("{repair_id}:push:{commit}"))),
                Some(receipt("pushed", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
        }
        Ok(())
    }

    pub fn record_repair_pull_request(
        &mut self,
        repair_id: &str,
        pull_request_id: &str,
        pull_request_url: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let (repository_id, number) = {
            let repair = self.repair_mut(repair_id)?;
            repair.pull_request_id = Some(pull_request_id.to_owned());
            repair.pull_request_url = Some(pull_request_url.to_owned());
            repair.status = ParentRepairStatus::AwaitingReview;
            (repair.repository_id.clone(), repair.number)
        };
        self.transition(
            ParentIntegrationState::AwaitingFixReview {
                repository_id,
                repair_attempt: number,
                pull_request_id: pull_request_id.to_owned(),
            },
            "repair pull request is awaiting configured checks and review",
            format!("{repair_id}:pull-request:{pull_request_id}"),
            input_version,
            Some(intent(
                "reconcile_pull_request",
                format!("{repair_id}:pull-request:{pull_request_id}"),
            )),
            Some(receipt("found_or_created", None)),
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        Ok(())
    }

    pub fn reconcile_repair_provider(
        &mut self,
        repair_id: &str,
        snapshot: ParentRepairProviderSnapshot,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let current = self.repair(repair_id)?.clone();
        if !snapshot.provider_available {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::ProviderUnavailable;
            return self.block_repair(
                repair_id,
                "provider unavailable",
                input_version,
                occurred_at,
            );
        }
        if snapshot.pull_request_id.is_none()
            && current.pull_request_id.is_none()
            && current.pushed_commit.is_some()
            && matches!(self.state, ParentIntegrationState::Blocked { .. })
        {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::AwaitingPullRequest;
            self.transition(
                ParentIntegrationState::Fixing {
                    repository_id: current.repository_id,
                    repair_attempt: current.number,
                },
                "provider recovered before repair pull-request creation",
                format!("{repair_id}:pull-request-recovered:{}", self.state_version),
                input_version,
                Some(intent(
                    "reconcile_pull_request",
                    format!("{repair_id}:pull-request-recovered:{}", self.state_version),
                )),
                Some(receipt("missing", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
            return Ok(());
        }
        if snapshot.merged {
            let merge_commit = snapshot
                .merge_result_commit
                .filter(|commit| !commit.trim().is_empty())
                .ok_or_else(|| {
                    ParentIntegrationError::RepairTargetMismatch(repair_id.to_owned())
                })?;
            if !snapshot.target_contains_merge_result {
                self.repair_mut(repair_id)?.status = ParentRepairStatus::ForcePushed;
                return self.block_repair(
                    repair_id,
                    "provider merge result is not reachable from target",
                    input_version,
                    occurred_at,
                );
            }
            let repair = self.repair_mut(repair_id)?;
            repair.status = ParentRepairStatus::Refreshing;
            repair.merge_result_commit = Some(merge_commit);
            self.transition(
                ParentIntegrationState::RefreshingAfterFixes,
                "provider reports a reachable repair merge result",
                format!("{repair_id}:merged"),
                input_version,
                Some(intent(
                    "refresh_after_repair",
                    format!("{repair_id}:merged"),
                )),
                Some(receipt("merged", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
            return Ok(());
        }
        if !snapshot.open && snapshot.pull_request_id.is_some() {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::ExternallyClosed;
            return self.block_repair(
                repair_id,
                "repair pull request was closed externally",
                input_version,
                occurred_at,
            );
        }
        if snapshot
            .head_commit
            .as_deref()
            .zip(current.pushed_commit.as_deref())
            .is_some_and(|(actual, expected)| actual != expected)
        {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::ForcePushed;
            return self.block_repair(
                repair_id,
                "repair branch was force-pushed outside the recorded attempt",
                input_version,
                occurred_at,
            );
        }
        if snapshot.merge_conflict {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::MergeConflict;
            return self.block_repair(
                repair_id,
                "repair merge conflict",
                input_version,
                occurred_at,
            );
        }
        if snapshot.checks_failed {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::FailedChecks;
            return self.block_repair(
                repair_id,
                "required checks failed",
                input_version,
                occurred_at,
            );
        }
        let review_is_current =
            snapshot.review_head_commit.as_deref() == current.pushed_commit.as_deref();
        if snapshot.changes_requested && review_is_current {
            if current.status == ParentRepairStatus::ChangesRequested {
                return Ok(());
            }
            let (repository_id, number, change_number) = {
                let repair = self.repair_mut(repair_id)?;
                repair.status = ParentRepairStatus::ChangesRequested;
                repair.implementation_completed = false;
                repair.requested_change_count = repair.requested_change_count.saturating_add(1);
                (
                    repair.repository_id.clone(),
                    repair.number,
                    repair.requested_change_count,
                )
            };
            self.transition(
                ParentIntegrationState::Fixing {
                    repository_id,
                    repair_attempt: number,
                },
                "provider review requested changes in the existing repair attempt",
                format!("{repair_id}:changes-requested:{change_number}"),
                input_version,
                None,
                Some(receipt("changes_requested", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
            return Ok(());
        }
        if snapshot.review_rejected && review_is_current {
            self.repair_mut(repair_id)?.status = ParentRepairStatus::ReviewRejected;
            return self.block_repair(
                repair_id,
                "repair review was rejected",
                input_version,
                occurred_at,
            );
        }
        let checks_satisfied = !current.policy.required_checks || snapshot.checks_passed;
        let review_satisfied =
            !current.policy.required_review || (snapshot.review_approved && review_is_current);
        if checks_satisfied && review_satisfied && snapshot.mergeable {
            let pull_request_id = current.pull_request_id.ok_or_else(|| {
                ParentIntegrationError::RepairTargetMismatch(repair_id.to_owned())
            })?;
            self.repair_mut(repair_id)?.status = ParentRepairStatus::AwaitingMerge;
            self.transition(
                ParentIntegrationState::AwaitingFixMerge {
                    repository_id: current.repository_id,
                    repair_attempt: current.number,
                    pull_request_id,
                },
                "central repair review policy is satisfied",
                format!("{repair_id}:merge-ready"),
                input_version,
                Some(intent("merge_repair", format!("{repair_id}:merge-ready"))),
                Some(receipt("eligible", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
        } else if matches!(
            self.state,
            ParentIntegrationState::Blocked { .. }
                | ParentIntegrationState::AwaitingFixMerge { .. }
        ) && snapshot.open
        {
            let pull_request_id = current.pull_request_id.ok_or_else(|| {
                ParentIntegrationError::RepairTargetMismatch(repair_id.to_owned())
            })?;
            self.repair_mut(repair_id)?.status = ParentRepairStatus::AwaitingReview;
            self.transition(
                ParentIntegrationState::AwaitingFixReview {
                    repository_id: current.repository_id,
                    repair_attempt: current.number,
                    pull_request_id,
                },
                "provider reconciliation requires a current eligible review snapshot",
                format!("{repair_id}:provider-recovered:{}", self.state_version),
                input_version,
                Some(intent(
                    "reconcile_provider",
                    format!("{repair_id}:provider-recovered:{}", self.state_version),
                )),
                Some(receipt("recovered", None)),
                ParentRetryClassification::Retryable,
                occurred_at,
            )?;
        }
        Ok(())
    }

    pub fn record_repair_refresh(
        &mut self,
        repair_id: &str,
        refreshed_target_commit: &str,
        refreshed_instruction_path: &Path,
        refreshed_instruction_hash: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let repository_id = {
            let repair = self.repair_mut(repair_id)?;
            if repair.merge_result_commit.is_none() || refreshed_target_commit.trim().is_empty() {
                return Err(ParentIntegrationError::RepairTargetMismatch(
                    repair_id.to_owned(),
                ));
            }
            repair.status = ParentRepairStatus::Completed;
            repair.refreshed_target_commit = Some(refreshed_target_commit.to_owned());
            repair.repository_id.clone()
        };
        let target = self.targets.get_mut(&repository_id).ok_or_else(|| {
            ParentIntegrationError::UnknownRepairRepository(repository_id.to_string())
        })?;
        target.target_commit = refreshed_target_commit.to_owned();
        target.instruction_path = refreshed_instruction_path.to_path_buf();
        target.instruction_hash = refreshed_instruction_hash.to_owned();
        self.transition(
            ParentIntegrationState::Integrating,
            "affected integration checkout refreshed to the provider merge result",
            format!("{repair_id}:refreshed:{refreshed_target_commit}"),
            input_version,
            Some(intent(
                "verify_refreshed_target",
                format!("{repair_id}:refreshed:{refreshed_target_commit}"),
            )),
            Some(receipt("reachable", None)),
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        Ok(())
    }

    pub fn repair(&self, repair_id: &str) -> Result<&ParentRepairAttempt, ParentIntegrationError> {
        self.repair_attempts
            .iter()
            .find(|repair| repair.id == repair_id)
            .ok_or_else(|| ParentIntegrationError::UnknownRepairAttempt(repair_id.to_owned()))
    }

    fn repair_mut(
        &mut self,
        repair_id: &str,
    ) -> Result<&mut ParentRepairAttempt, ParentIntegrationError> {
        self.repair_attempts
            .iter_mut()
            .find(|repair| repair.id == repair_id)
            .ok_or_else(|| ParentIntegrationError::UnknownRepairAttempt(repair_id.to_owned()))
    }

    fn block_repair(
        &mut self,
        repair_id: &str,
        reason: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        self.transition(
            ParentIntegrationState::Blocked {
                reason: reason.to_owned(),
            },
            reason,
            format!("{repair_id}:blocked:{reason}"),
            input_version,
            Some(intent(
                "reconcile_provider",
                format!("{repair_id}:blocked:{reason}"),
            )),
            Some(receipt("blocked", Some(reason.to_owned()))),
            ParentRetryClassification::OperatorAction,
            occurred_at,
        )?;
        Ok(())
    }

    pub fn complete(
        &mut self,
        attempt_id: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let expected_commits = self
            .targets
            .iter()
            .map(|(repository_id, target)| (repository_id.clone(), target.target_commit.clone()))
            .collect::<BTreeMap<_, _>>();
        if !self.attempts.iter().any(|attempt| {
            attempt.id == attempt_id
                && attempt.status.passed()
                && attempt.input_version == input_version
                && attempt.exit_code == Some(0)
                && attempt
                    .cleanup
                    .as_ref()
                    .is_some_and(|cleanup| cleanup.status == ParentCleanupStatus::Succeeded)
                && attempt.verified_repository_commits == expected_commits
        }) {
            return Err(ParentIntegrationError::FinalVerificationIncomplete);
        }
        self.transition(
            ParentIntegrationState::FinalVerification,
            "parent integration attempt passed",
            format!("{attempt_id}:final-verification"),
            input_version,
            Some(intent(
                "accept_harness_final_verification",
                format!("{attempt_id}:final-verification"),
            )),
            Some(receipt("passed", None)),
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        self.final_evidence = Some(ParentFinalEvidence {
            attempt_id: attempt_id.to_owned(),
            conversation_id: self.conversation_id.clone().unwrap_or_default(),
            input_version: input_version.to_owned(),
            repository_commits: expected_commits,
            recorded_at: occurred_at,
        });
        self.transition(
            ParentIntegrationState::Finalizing,
            "final verification evidence was written durably",
            format!("{attempt_id}:finalize"),
            input_version,
            Some(intent(
                "persist_final_evidence",
                format!("{attempt_id}:finalize"),
            )),
            Some(receipt("succeeded", None)),
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        self.transition(
            ParentIntegrationState::Completed,
            "all recorded repository targets passed final verification",
            format!("{attempt_id}:complete"),
            input_version,
            None,
            Some(receipt("succeeded", None)),
            ParentRetryClassification::Terminal,
            occurred_at,
        )?;
        Ok(())
    }

    pub fn cancel(
        &mut self,
        reason: impl Into<String>,
        input_version: &str,
        acknowledged: bool,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        let reason = reason.into();
        self.transition(
            ParentIntegrationState::Canceled {
                reason: reason.clone(),
            },
            reason,
            format!(
                "parent:{}:{}:cancel",
                self.parent_id, self.hierarchy_generation
            ),
            input_version,
            Some(intent(
                "interrupt_parent_harness",
                format!(
                    "parent:{}:{}:cancel",
                    self.parent_id, self.hierarchy_generation
                ),
            )),
            Some(receipt(
                if acknowledged {
                    "acknowledged"
                } else {
                    "uncertain"
                },
                None,
            )),
            ParentRetryClassification::Terminal,
            occurred_at,
        )?;
        Ok(())
    }

    pub fn current_attempt_id(&self) -> Option<&str> {
        self.attempts
            .iter()
            .rev()
            .find(|attempt| attempt.status == ParentAttemptStatus::Running)
            .map(|attempt| attempt.id.as_str())
    }

    pub fn current_attempt_deadline(&self) -> Option<TimestampMs> {
        self.attempts
            .iter()
            .rev()
            .find(|attempt| attempt.status == ParentAttemptStatus::Running)
            .map(|attempt| {
                TimestampMs::new(
                    attempt
                        .started_at
                        .as_u64()
                        .saturating_add(attempt.timeout_ms),
                )
            })
    }

    fn cleanup_complete(&self) -> bool {
        self.attempts.iter().all(|attempt| match attempt.status {
            ParentAttemptStatus::Running => false,
            ParentAttemptStatus::Passed
            | ParentAttemptStatus::Failed
            | ParentAttemptStatus::TimedOut
            | ParentAttemptStatus::Canceled
            | ParentAttemptStatus::Indeterminate => {
                attempt
                    .cleanup
                    .as_ref()
                    .is_some_and(|cleanup| cleanup.status == ParentCleanupStatus::Succeeded)
                    && active_resource_keys(attempt).is_empty()
            }
        })
    }

    fn attempt_mut(
        &mut self,
        attempt_id: &str,
    ) -> Result<&mut ParentVerificationAttempt, ParentIntegrationError> {
        self.attempts
            .iter_mut()
            .find(|attempt| attempt.id == attempt_id)
            .ok_or_else(|| ParentIntegrationError::UnknownAttempt(attempt_id.to_owned()))
    }

    #[allow(clippy::too_many_arguments)]
    fn transition(
        &mut self,
        next: ParentIntegrationState,
        reason: impl Into<String>,
        idempotency_key: impl Into<String>,
        input_version: &str,
        side_effect_intent: Option<ParentSideEffectIntent>,
        result_receipt: Option<ParentSideEffectReceipt>,
        retry_classification: ParentRetryClassification,
        occurred_at: TimestampMs,
    ) -> Result<bool, ParentIntegrationError> {
        let reason = reason.into();
        let idempotency_key = idempotency_key.into();
        if let Some(existing) = self
            .transitions
            .iter()
            .find(|transition| transition.idempotency_key == idempotency_key)
        {
            if existing.next == next
                && existing.reason == reason
                && existing.input_version == input_version
                && existing.side_effect_intent == side_effect_intent
                && existing.result_receipt == result_receipt
                && existing.retry_classification == retry_classification
            {
                return Ok(false);
            }
            return Err(ParentIntegrationError::IdempotencyConflict(idempotency_key));
        }
        if !allowed_transition(&self.state, &next) {
            return Err(ParentIntegrationError::InvalidTransition {
                from: Box::new(self.state.clone()),
                to: Box::new(next),
            });
        }
        self.state_version = self.state_version.saturating_add(1);
        let previous = std::mem::replace(&mut self.state, next.clone());
        self.transitions.push(ParentTransition {
            state_version: self.state_version,
            previous,
            next,
            reason,
            idempotency_key,
            input_version: input_version.to_owned(),
            side_effect_intent,
            result_receipt,
            retry_classification,
            occurred_at,
        });
        if self.transitions.len() > MAX_TRANSITIONS {
            self.transitions.remove(0);
        }
        Ok(true)
    }
}

fn active_resource_keys(attempt: &ParentVerificationAttempt) -> Vec<(String, String)> {
    let mut latest = BTreeMap::new();
    for resource in &attempt.resources {
        if resource.status != ParentResourceStatus::Collision {
            latest.insert(
                (resource.kind.clone(), resource.identifier.clone()),
                resource.status,
            );
        }
    }
    latest
        .into_iter()
        .filter_map(|(key, status)| {
            matches!(
                status,
                ParentResourceStatus::Allocated | ParentResourceStatus::CleanupFailed
            )
            .then_some(key)
        })
        .collect()
}

fn allowed_transition(from: &ParentIntegrationState, to: &ParentIntegrationState) -> bool {
    use ParentIntegrationState as State;
    matches!(
        (from, to),
        (State::WaitingForChildren, State::WaitingForChildMerges)
            | (State::WaitingForChildMerges, State::AcquiringChildLeases)
            | (
                State::AcquiringChildLeases,
                State::PreparingIntegrationWorkspace
            )
            | (
                State::PreparingIntegrationWorkspace,
                State::RefreshingRepositories
            )
            | (State::RefreshingRepositories, State::Integrating)
            | (State::Integrating, State::RefreshingRepositories)
            | (State::Integrating, State::Fixing { .. })
            | (State::Fixing { .. }, State::AwaitingFixReview { .. })
            | (State::AwaitingFixReview { .. }, State::Fixing { .. })
            | (
                State::AwaitingFixReview { .. },
                State::AwaitingFixMerge { .. }
            )
            | (
                State::AwaitingFixMerge { .. },
                State::AwaitingFixReview { .. }
            )
            | (State::AwaitingFixMerge { .. }, State::Fixing { .. })
            | (State::AwaitingFixReview { .. }, State::RefreshingAfterFixes)
            | (State::AwaitingFixMerge { .. }, State::RefreshingAfterFixes)
            | (State::RefreshingAfterFixes, State::Integrating)
            | (State::Integrating, State::FinalVerification)
            | (State::FinalVerification, State::Integrating)
            | (State::FinalVerification, State::Finalizing)
            | (State::Finalizing, State::Completed)
            | (State::Blocked { .. }, State::RefreshingRepositories)
            | (State::Blocked { .. }, State::Fixing { .. })
            | (State::Blocked { .. }, State::AwaitingFixReview { .. })
            | (State::Blocked { .. }, State::AwaitingFixMerge { .. })
            | (State::Blocked { .. }, State::RefreshingAfterFixes)
            | (_, State::Blocked { .. })
            | (_, State::Failed { .. })
            | (_, State::Canceled { .. })
    ) && !from.terminal()
}

fn intent(kind: impl Into<String>, idempotency_key: impl Into<String>) -> ParentSideEffectIntent {
    ParentSideEffectIntent {
        kind: kind.into(),
        idempotency_key: idempotency_key.into(),
    }
}

fn receipt(status: impl Into<String>, detail: Option<String>) -> ParentSideEffectReceipt {
    ParentSideEffectReceipt {
        status: status.into(),
        detail,
    }
}

pub fn parent_command_identity(command: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(command.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repair_policy() -> ParentRepairPolicy {
        ParentRepairPolicy {
            review_profile: "required".to_owned(),
            review_provider: "github".to_owned(),
            review_policy_generation: "policy-1".to_owned(),
            required_checks: true,
            required_review: true,
            merge_method: "squash".to_owned(),
        }
    }

    fn controller() -> ParentIntegrationController {
        let mut controller =
            ParentIntegrationController::new(IssueId::new("parent").expect("parent id"), 7)
                .expect("controller");
        controller
            .admit("hierarchy:7", TimestampMs::new(1))
            .expect("admit");
        controller
            .record_workspace_prepared(
                ["a", "b", "c"]
                    .into_iter()
                    .map(|repo| ParentRepositoryTarget {
                        repository_id: CanonicalRepositoryId::new(format!(
                            "github:repository:{repo}"
                        ))
                        .expect("repository"),
                        checkout_handle: format!("checkout-{repo}"),
                        relative_path: PathBuf::from(format!("repositories/{repo}")),
                        target_branch: "develop".to_owned(),
                        target_commit: format!("commit-{repo}"),
                        instruction_path: PathBuf::from("AGENTS.md"),
                        instruction_hash: format!("instruction-{repo}"),
                        repair_policy: repair_policy(),
                    }),
                "targets:1",
                TimestampMs::new(2),
            )
            .expect("workspace");
        controller
    }

    fn finish_passed(controller: &mut ParentIntegrationController, id: &str) {
        controller
            .observe_command_started(
                id,
                "command-final",
                "cargo test",
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(3),
            )
            .expect("command start");
        controller
            .observe_command_finished(
                id,
                "command-final",
                0,
                Some("all checks passed"),
                TimestampMs::new(4),
            )
            .expect("command completion");
        let evidence = ParentVerificationEvidence {
            schema_version: 1,
            run_id: "run-parent".to_owned(),
            attempt: 1,
            hierarchy_generation: controller.hierarchy_generation,
            repository_commits: controller
                .targets
                .iter()
                .map(|(repository_id, target)| {
                    (repository_id.clone(), target.target_commit.clone())
                })
                .collect(),
            command: "cargo test".to_owned(),
            command_hash: parent_command_identity("cargo test"),
            root: "parent_root".to_owned(),
            repair_repository_id: None,
        };
        assert!(
            controller
                .record_verification_evidence(id, &evidence)
                .expect("verification receipt")
        );
        controller
            .finish_attempt(
                id,
                ParentAttemptStatus::Passed,
                Some(0),
                ParentCleanupReceipt {
                    status: ParentCleanupStatus::Succeeded,
                    occurred_at: TimestampMs::new(4),
                    detail: None,
                },
                TimestampMs::new(4),
            )
            .expect("finish");
    }

    #[test]
    fn no_repair_lifecycle_is_idempotent_and_records_exact_targets() {
        let mut controller = controller();
        let attempt = controller
            .start_attempt(
                "integration",
                "attempt:1",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                30_000,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        assert_eq!(
            controller
                .start_attempt(
                    "integration",
                    "attempt:1",
                    ParentAttemptRoot::ParentRoot,
                    "conversation-1",
                    30_000,
                    "targets:1",
                    TimestampMs::new(3),
                )
                .expect("idempotent attempt"),
            attempt
        );
        finish_passed(&mut controller, &attempt);
        controller
            .complete(&attempt, "targets:1", TimestampMs::new(5))
            .expect("complete");

        assert_eq!(controller.state, ParentIntegrationState::Completed);
        assert_eq!(
            controller.state_version as usize,
            controller.transitions.len()
        );
        assert_eq!(
            controller
                .final_evidence
                .as_ref()
                .expect("evidence")
                .repository_commits
                .len(),
            3
        );
    }

    #[test]
    fn checkout_roots_are_topology_neutral_and_generation_bound() {
        let mut valid_root = controller();
        assert!(
            valid_root
                .start_attempt(
                    "repo check",
                    "attempt:repo",
                    ParentAttemptRoot::CheckoutHandle("checkout-b".to_owned()),
                    "conversation-1",
                    1_000,
                    "targets:1",
                    TimestampMs::new(3),
                )
                .is_ok()
        );
        let mut unknown_root = controller();
        assert_eq!(
            unknown_root.start_attempt(
                "unknown repo check",
                "attempt:unknown",
                ParentAttemptRoot::CheckoutHandle("frontend".to_owned()),
                "conversation-1",
                1_000,
                "targets:1",
                TimestampMs::new(3),
            ),
            Err(ParentIntegrationError::UnknownCheckoutHandle(
                "frontend".to_owned()
            ))
        );
    }

    #[test]
    fn timeout_and_indeterminate_attempts_require_cleanup_before_rerun() {
        let mut timed_out_controller = controller();
        let attempt = timed_out_controller
            .start_attempt(
                "integration",
                "attempt:1",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        timed_out_controller
            .allocate_resource(&attempt, "port", "24001", TimestampMs::new(3))
            .expect("port");
        timed_out_controller
            .finish_attempt(
                &attempt,
                ParentAttemptStatus::TimedOut,
                None,
                ParentCleanupReceipt {
                    status: ParentCleanupStatus::Failed,
                    occurred_at: TimestampMs::new(4),
                    detail: Some("process remained visible".to_owned()),
                },
                TimestampMs::new(4),
            )
            .expect("timeout");
        timed_out_controller
            .prepare_retry(&attempt, "timeout", TimestampMs::new(5))
            .expect("retry intent");
        assert!(
            timed_out_controller
                .record_baseline_verified("targets:2", TimestampMs::new(6))
                .is_err()
        );

        let mut failed_without_teardown = controller();
        let failed = failed_without_teardown
            .start_attempt(
                "integration",
                "attempt:failed",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        failed_without_teardown
            .finish_attempt(
                &failed,
                ParentAttemptStatus::Failed,
                None,
                ParentCleanupReceipt {
                    status: ParentCleanupStatus::Pending,
                    occurred_at: TimestampMs::new(4),
                    detail: Some("no runtime completion event".to_owned()),
                },
                TimestampMs::new(4),
            )
            .expect("failed attempt");
        failed_without_teardown
            .prepare_retry(&failed, "failed", TimestampMs::new(5))
            .expect("retry intent");
        failed_without_teardown
            .record_baseline_verified("targets:2", TimestampMs::new(6))
            .expect_err("failed command without teardown blocks baseline refresh");

        let mut stopped_by_harness = controller();
        let stopped = stopped_by_harness
            .start_attempt(
                "integration",
                "attempt:stopped",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        stopped_by_harness
            .observe_command_started(
                &stopped,
                "command-running",
                "cargo test",
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(4),
            )
            .expect("command start");
        stopped_by_harness
            .observe_harness_stopped(
                &stopped,
                "interrupt observed paused state",
                TimestampMs::new(103),
            )
            .expect("stopped-state cleanup");
        let cleanup = stopped_by_harness.attempts[0]
            .cleanup
            .clone()
            .expect("cleanup receipt");
        stopped_by_harness
            .finish_attempt(
                &stopped,
                ParentAttemptStatus::TimedOut,
                None,
                cleanup,
                TimestampMs::new(103),
            )
            .expect("timed out attempt");
        stopped_by_harness
            .prepare_retry(&stopped, "timeout", TimestampMs::new(104))
            .expect("retry intent");
        stopped_by_harness
            .record_baseline_verified("targets:2", TimestampMs::new(105))
            .expect("stopped-state receipt permits baseline refresh");

        let mut restarted = controller();
        let running = restarted
            .start_attempt(
                "integration",
                "attempt:restart",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        restarted
            .reconcile_restart(false, TimestampMs::new(4))
            .expect("restart reconciliation");
        assert_eq!(
            restarted
                .attempts
                .iter()
                .find(|attempt| attempt.id == running)
                .expect("attempt")
                .status,
            ParentAttemptStatus::Indeterminate
        );
        restarted
            .record_baseline_verified("targets:2", TimestampMs::new(5))
            .expect_err("unknown teardown blocks baseline refresh");
        restarted
            .attempts
            .iter_mut()
            .find(|attempt| attempt.id == running)
            .expect("attempt")
            .cleanup = Some(ParentCleanupReceipt {
            status: ParentCleanupStatus::Succeeded,
            occurred_at: TimestampMs::new(6),
            detail: Some("recovery cleanup verified".to_owned()),
        });
        restarted
            .record_baseline_verified("targets:2", TimestampMs::new(7))
            .expect("explicit cleanup permits verified rerun");

        let mut prelaunch = controller();
        let intent = prelaunch
            .start_attempt_intent(
                "integration",
                "attempt:prelaunch",
                ParentAttemptRoot::ParentRoot,
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("persisted launch intent");
        prelaunch
            .reconcile_restart(false, TimestampMs::new(4))
            .expect("metadata-only intent reconciliation");
        let attempt = prelaunch
            .attempts
            .iter()
            .find(|attempt| attempt.id == intent)
            .expect("attempt");
        assert_eq!(attempt.status, ParentAttemptStatus::Indeterminate);
        assert_eq!(
            attempt.cleanup.as_ref().map(|cleanup| cleanup.status),
            Some(ParentCleanupStatus::Succeeded)
        );
        prelaunch
            .record_baseline_verified("targets:2", TimestampMs::new(5))
            .expect("metadata-only crash does not block the retry baseline");
    }

    #[test]
    fn terminal_run_manifest_recovers_lost_controller_outcome_without_rotating_conversation() {
        let mut controller = controller();
        let attempt_id = controller
            .start_attempt(
                "integration",
                "attempt:terminal-before-outcome",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");

        controller
            .reconcile_terminal_run_after_restart(TimestampMs::new(10))
            .expect("terminal run reconciliation");

        let attempt = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == attempt_id)
            .expect("attempt");
        assert_eq!(attempt.status, ParentAttemptStatus::Indeterminate);
        assert_eq!(
            attempt.cleanup.as_ref().map(|cleanup| cleanup.status),
            Some(ParentCleanupStatus::Succeeded)
        );
        assert_eq!(
            controller.conversation_id.as_deref(),
            Some("conversation-1")
        );
        controller
            .record_baseline_verified("targets:2", TimestampMs::new(11))
            .expect("terminal harness evidence permits baseline refresh");
        let next_attempt = controller
            .start_attempt(
                "integration",
                "attempt:after-terminal-recovery",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:2",
                TimestampMs::new(12),
            )
            .expect("retry stays on the authoritative conversation");
        assert_ne!(next_attempt, attempt_id);
    }

    #[test]
    fn verification_selects_real_completed_command_before_receipt_write() {
        let mut controller = controller();
        let attempt = controller
            .start_attempt(
                "integration",
                "attempt:commands",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        controller
            .observe_command_started(
                &attempt,
                "command-verify",
                "cargo test",
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(4),
            )
            .expect("verification start");
        controller
            .observe_command_started(
                &attempt,
                "command-verify",
                "cargo test",
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(4),
            )
            .expect("duplicate start is idempotent");
        controller
            .observe_command_finished(
                &attempt,
                "command-verify",
                0,
                Some("passed"),
                TimestampMs::new(5),
            )
            .expect("verification finish");
        controller
            .observe_command_finished(
                &attempt,
                "command-verify",
                0,
                Some("passed"),
                TimestampMs::new(5),
            )
            .expect("duplicate finish is idempotent");
        controller
            .observe_command_started(
                &attempt,
                "command-receipt",
                "write final-verification.json",
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(6),
            )
            .expect("receipt write start");
        controller
            .observe_command_finished(&attempt, "command-receipt", 0, None, TimestampMs::new(7))
            .expect("receipt write finish");

        let evidence = ParentVerificationEvidence {
            schema_version: 1,
            run_id: "run-parent".to_owned(),
            attempt: 1,
            hierarchy_generation: controller.hierarchy_generation,
            repository_commits: controller
                .targets
                .iter()
                .map(|(repository_id, target)| {
                    (repository_id.clone(), target.target_commit.clone())
                })
                .collect(),
            command: "cargo test".to_owned(),
            command_hash: parent_command_identity("cargo test"),
            root: "parent_root".to_owned(),
            repair_repository_id: None,
        };
        assert!(
            controller
                .record_verification_evidence(&attempt, &evidence)
                .expect("select observed verification command")
        );
        let attempt = controller
            .attempts
            .iter()
            .find(|candidate| candidate.id == attempt)
            .expect("attempt receipt");
        assert_eq!(attempt.commands.len(), 2);
        assert_eq!(attempt.resources.len(), 4);
        assert_eq!(attempt.exit_code, Some(0));
    }

    #[test]
    fn verification_uses_exact_command_identity_while_persisting_redacted_text() {
        let mut controller = controller();
        let attempt = controller
            .start_attempt(
                "integration",
                "attempt:exact-command",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(10),
            )
            .expect("attempt");
        let exact_command = "cargo   test token=secret";
        controller
            .observe_command_started(
                &attempt,
                "command-exact",
                exact_command,
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(11),
            )
            .expect("command start");
        controller
            .observe_command_finished(
                &attempt,
                "command-exact",
                0,
                Some("passed"),
                TimestampMs::new(12),
            )
            .expect("command finish");
        let receipt = controller
            .attempts
            .last()
            .expect("attempt")
            .commands
            .last()
            .expect("command");
        assert_ne!(receipt.command, exact_command);
        assert_eq!(receipt.command_hash, parent_command_identity(exact_command));

        let evidence = ParentVerificationEvidence {
            schema_version: 1,
            run_id: "run-parent".to_owned(),
            attempt: 1,
            hierarchy_generation: controller.hierarchy_generation,
            repository_commits: controller
                .targets
                .iter()
                .map(|(repository_id, target)| {
                    (repository_id.clone(), target.target_commit.clone())
                })
                .collect(),
            command: redact_runtime_diagnostic(exact_command),
            command_hash: parent_command_identity(exact_command),
            root: "parent_root".to_owned(),
            repair_repository_id: None,
        };
        assert!(
            controller
                .record_verification_evidence(&attempt, &evidence)
                .expect("exact command hash selects the observed command")
        );
    }

    #[test]
    fn command_events_before_the_current_attempt_are_ignored() {
        let mut controller = controller();
        let attempt = controller
            .start_attempt(
                "integration",
                "attempt:current-turn",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(10),
            )
            .expect("attempt");
        controller
            .observe_command_started(
                &attempt,
                "command-old-turn",
                "cargo test",
                ParentAttemptRoot::ParentRoot,
                TimestampMs::new(9),
            )
            .expect("stale start is ignored");
        controller
            .observe_command_finished(
                &attempt,
                "command-old-turn",
                0,
                Some("old completion"),
                TimestampMs::new(9),
            )
            .expect("stale completion is ignored");
        assert!(
            controller
                .attempts
                .last()
                .expect("attempt")
                .commands
                .is_empty()
        );
    }

    #[test]
    fn resources_and_logs_are_bounded_and_attributable() {
        let mut controller = controller();
        let first = controller
            .start_attempt(
                "integration",
                "attempt:1",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("first");
        controller
            .allocate_resource(&first, "port", "24001", TimestampMs::new(3))
            .expect("port");
        assert!(matches!(
            controller.allocate_resource(&first, "port", "24001", TimestampMs::new(3)),
            Err(ParentIntegrationError::ResourceCollision { .. })
        ));
        controller
            .append_log(&first, &"x".repeat(MAX_LOG_BYTES + 10))
            .expect("log");
        let attempt = controller
            .attempts
            .iter()
            .find(|attempt| attempt.id == first)
            .expect("attempt");
        assert_eq!(attempt.bounded_log.len(), MAX_LOG_BYTES);
        assert!(attempt.log_truncated);

        controller
            .release_resource(&first, "port", "24001", true, None, TimestampMs::new(4))
            .expect("release port");
        assert!(
            controller
                .allocate_resource(&first, "port", "24001", TimestampMs::new(5))
                .is_ok()
        );
        controller
            .release_resource(
                &first,
                "port",
                "24001",
                false,
                Some("still listening".to_owned()),
                TimestampMs::new(6),
            )
            .expect("record failed cleanup");
        assert!(matches!(
            controller.allocate_resource(&first, "port", "24001", TimestampMs::new(7)),
            Err(ParentIntegrationError::ResourceCollision { .. })
        ));
    }

    #[test]
    fn attempt_ids_remain_monotonic_after_retention() {
        let mut controller = controller();
        controller.next_attempt_sequence = 64;
        let attempt = controller
            .start_attempt(
                "integration",
                "attempt:65",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                1_000,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        assert_eq!(attempt, "parent-attempt-65");
        assert_eq!(controller.next_attempt_sequence, 65);
    }

    #[test]
    fn admission_idempotency_survives_transition_history_compaction() {
        let mut controller = controller();
        for sequence in 1..=(MAX_TRANSITIONS / 2 + 8) {
            let attempt = controller
                .start_attempt(
                    format!("integration {sequence}"),
                    format!("attempt:{sequence}"),
                    ParentAttemptRoot::ParentRoot,
                    "conversation-1",
                    1_000,
                    "targets:1",
                    TimestampMs::new(sequence as u64 * 3),
                )
                .expect("attempt");
            controller
                .finish_attempt(
                    &attempt,
                    ParentAttemptStatus::Failed,
                    Some(1),
                    ParentCleanupReceipt {
                        status: ParentCleanupStatus::Succeeded,
                        occurred_at: TimestampMs::new(sequence as u64 * 3 + 1),
                        detail: None,
                    },
                    TimestampMs::new(sequence as u64 * 3 + 1),
                )
                .expect("finish attempt");
            controller
                .prepare_retry(&attempt, "retry", TimestampMs::new(sequence as u64 * 3 + 1))
                .expect("prepare retry");
            controller
                .record_baseline_verified("targets:1", TimestampMs::new(sequence as u64 * 3 + 2))
                .expect("baseline");
        }
        assert_eq!(controller.transitions.len(), MAX_TRANSITIONS);
        assert!(
            !controller
                .transitions
                .iter()
                .any(|transition| { transition.idempotency_key.ends_with(":children") })
        );

        let mut restored: ParentIntegrationController = serde_json::from_value(
            serde_json::to_value(&controller).expect("serialize controller"),
        )
        .expect("restore controller");
        let state_version = restored.state_version;
        restored
            .admit("hierarchy:7", TimestampMs::new(10_000))
            .expect("persisted admission remains idempotent");
        assert_eq!(restored.state_version, state_version);
        assert_eq!(
            restored.admission_input_version.as_deref(),
            Some("hierarchy:7")
        );
    }

    #[test]
    fn repository_neutral_parent_can_complete_with_no_checkout_targets() {
        let mut controller =
            ParentIntegrationController::new(IssueId::new("parent-empty").expect("parent id"), 1)
                .expect("controller");
        controller
            .admit("hierarchy:1", TimestampMs::new(1))
            .expect("admit");
        controller
            .record_workspace_prepared([], "targets:empty", TimestampMs::new(2))
            .expect("empty parent workspace");
        let attempt = controller
            .start_attempt(
                "repository-neutral verification",
                "attempt:empty",
                ParentAttemptRoot::ParentRoot,
                "conversation-empty",
                100,
                "targets:empty",
                TimestampMs::new(3),
            )
            .expect("attempt");
        finish_passed(&mut controller, &attempt);
        controller
            .complete(&attempt, "targets:empty", TimestampMs::new(5))
            .expect("empty target set is verified by the runtime command");

        assert_eq!(controller.state, ParentIntegrationState::Completed);
        assert!(
            controller
                .final_evidence
                .as_ref()
                .expect("final evidence")
                .repository_commits
                .is_empty()
        );
    }

    #[test]
    fn unchanged_targets_refresh_again_before_a_retry_attempt() {
        let mut controller = controller();
        let first = controller
            .start_attempt(
                "integration",
                "attempt:first",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                1_000,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("first attempt");
        controller
            .finish_attempt(
                &first,
                ParentAttemptStatus::Failed,
                Some(1),
                ParentCleanupReceipt {
                    status: ParentCleanupStatus::Succeeded,
                    occurred_at: TimestampMs::new(4),
                    detail: None,
                },
                TimestampMs::new(4),
            )
            .expect("failed attempt");
        controller
            .prepare_retry(&first, "retry", TimestampMs::new(5))
            .expect("retry transition");
        controller
            .record_baseline_verified("targets:1", TimestampMs::new(6))
            .expect("refresh unchanged targets");

        assert!(
            controller
                .start_attempt(
                    "integration retry",
                    "attempt:second",
                    ParentAttemptRoot::ParentRoot,
                    "conversation-1",
                    1_000,
                    "targets:1",
                    TimestampMs::new(7),
                )
                .is_ok()
        );
    }

    #[test]
    fn completed_intermediate_parent_keeps_checkout_evidence_for_ancestor() {
        let mut controller = controller();
        let attempt = controller
            .start_attempt(
                "integration",
                "attempt:1",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                30_000,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("attempt");
        finish_passed(&mut controller, &attempt);
        controller
            .complete(&attempt, "targets:1", TimestampMs::new(5))
            .expect("complete");
        let encoded = serde_json::to_value(&controller).expect("encode");
        let recovered: ParentIntegrationController =
            serde_json::from_value(encoded).expect("decode");
        assert_eq!(recovered.targets.len(), 3);
        assert_eq!(recovered.final_evidence, controller.final_evidence);
    }

    fn begin_repair(controller: &mut ParentIntegrationController) -> String {
        controller
            .begin_repair(
                "repair:defect-a",
                CanonicalRepositoryId::new("github:repository:a").expect("repository"),
                "checkout-a",
                "commit-a",
                "targets:1",
                TimestampMs::new(10),
            )
            .expect("begin repair")
    }

    fn review_snapshot() -> ParentRepairProviderSnapshot {
        ParentRepairProviderSnapshot {
            pull_request_id: Some("42".to_owned()),
            pull_request_url: Some("https://github.com/example/a/pull/42".to_owned()),
            head_commit: Some("repair-commit-1".to_owned()),
            review_head_commit: Some("repair-commit-1".to_owned()),
            review_request_cursor: None,
            open: true,
            checks_passed: false,
            checks_failed: false,
            review_approved: false,
            review_rejected: false,
            changes_requested: false,
            mergeable: true,
            merge_conflict: false,
            merged: false,
            merge_result_commit: None,
            target_contains_merge_result: false,
            provider_available: true,
        }
    }

    #[test]
    fn repair_attempts_use_verified_target_policy_and_survive_recovery() {
        let mut controller = controller();
        let repair_id = begin_repair(&mut controller);
        let replay = begin_repair(&mut controller);

        assert_eq!(replay, repair_id);
        let repair = controller.repair(&repair_id).expect("repair");
        assert_eq!(repair.branch, "fix/parent-g7-repair-1");
        assert_eq!(repair.instruction_hash, "instruction-a");
        assert_eq!(repair.policy, repair_policy());
        assert_eq!(repair.target_commit, "commit-a");

        let recovered: ParentIntegrationController = serde_json::from_value(
            serde_json::to_value(controller).expect("serialize repair controller"),
        )
        .expect("recover repair controller");
        assert_eq!(recovered.repair_attempts.len(), 1);
        assert_eq!(recovered.next_repair_sequence, 1);
    }

    #[test]
    fn schema_one_parent_targets_without_repair_policy_remain_readable() {
        let controller = controller();
        let current = controller.targets.values().next().expect("target").clone();
        let mut value = serde_json::to_value(controller).expect("serialize controller");
        for target in value["targets"]
            .as_object_mut()
            .expect("target map")
            .values_mut()
        {
            target
                .as_object_mut()
                .expect("target")
                .remove("repair_policy");
            target
                .as_object_mut()
                .expect("target")
                .remove("target_branch");
            target
                .as_object_mut()
                .expect("target")
                .remove("instruction_path");
            target
                .as_object_mut()
                .expect("target")
                .remove("instruction_hash");
        }

        let mut recovered: ParentIntegrationController =
            serde_json::from_value(value).expect("schema-one controller remains readable");
        assert!(recovered.targets.values().all(|target| {
            target.target_branch.is_empty()
                && target.instruction_path.as_os_str().is_empty()
                && target.instruction_hash.is_empty()
                && target.repair_policy == ParentRepairPolicy::default()
        }));
        assert!(
            recovered
                .migrate_legacy_repair_target(current.clone())
                .expect("verified provenance migrates")
        );
        assert_eq!(
            recovered.targets.get(&current.repository_id),
            Some(&current)
        );
    }

    #[test]
    fn mutating_provider_operations_require_search_before_create() {
        let mut controller = controller();
        let repair_id = begin_repair(&mut controller);

        assert!(matches!(
            controller.begin_provider_operation(
                &repair_id,
                ParentProviderOperationKind::CreateBranch,
                "targets:1",
                TimestampMs::new(11),
            ),
            Err(ParentIntegrationError::ProviderReconciliationRequired { .. })
        ));
        controller
            .record_provider_operation(
                &repair_id,
                "repair:defect-a",
                "missing",
                None,
                TimestampMs::new(11),
            )
            .expect("branch lookup receipt");
        let create_key = controller
            .begin_provider_operation(
                &repair_id,
                ParentProviderOperationKind::CreateBranch,
                "targets:1",
                TimestampMs::new(12),
            )
            .expect("create intent");
        assert_eq!(
            controller
                .begin_provider_operation(
                    &repair_id,
                    ParentProviderOperationKind::CreateBranch,
                    "targets:1",
                    TimestampMs::new(13),
                )
                .expect("replayed create intent"),
            create_key
        );
        assert_eq!(
            controller
                .repair(&repair_id)
                .expect("repair")
                .operations
                .len(),
            2
        );
    }

    #[test]
    fn requested_changes_keep_one_attempt_and_reachable_merge_refreshes_target() {
        let mut controller = controller();
        let repair_id = begin_repair(&mut controller);
        controller
            .record_repair_branch_ready(&repair_id)
            .expect("branch ready");
        controller
            .record_repair_push(
                &repair_id,
                "repair-commit-1",
                "targets:1",
                TimestampMs::new(11),
            )
            .expect("push");
        controller
            .record_repair_pull_request(
                &repair_id,
                "42",
                "https://github.com/example/a/pull/42",
                "targets:1",
                TimestampMs::new(12),
            )
            .expect("pull request");

        let mut changes = review_snapshot();
        changes.changes_requested = true;
        controller
            .reconcile_repair_provider(
                &repair_id,
                changes.clone(),
                "targets:1",
                TimestampMs::new(13),
            )
            .expect("requested changes");
        controller
            .reconcile_repair_provider(
                &repair_id,
                changes.clone(),
                "targets:1",
                TimestampMs::new(14),
            )
            .expect("repeated requested changes are idempotent before a push");
        assert_eq!(
            controller
                .repair(&repair_id)
                .expect("repair")
                .requested_change_count,
            1
        );
        controller
            .record_repair_push(
                &repair_id,
                "repair-commit-2",
                "targets:1",
                TimestampMs::new(15),
            )
            .expect("same PR repair push");
        controller
            .reconcile_repair_provider(&repair_id, changes, "targets:1", TimestampMs::new(16))
            .expect("stale requested changes do not apply to the new head");

        let mut approved = review_snapshot();
        approved.head_commit = Some("repair-commit-2".to_owned());
        approved.review_head_commit = Some("repair-commit-2".to_owned());
        approved.checks_passed = true;
        approved.review_approved = true;
        controller
            .reconcile_repair_provider(&repair_id, approved, "targets:1", TimestampMs::new(17))
            .expect("merge ready");
        let mut merged = review_snapshot();
        merged.head_commit = Some("repair-commit-2".to_owned());
        merged.merged = true;
        merged.open = false;
        merged.merge_result_commit = Some("squash-result".to_owned());
        merged.target_contains_merge_result = true;
        controller
            .reconcile_repair_provider(&repair_id, merged, "targets:1", TimestampMs::new(18))
            .expect("merged");
        controller
            .record_repair_refresh(
                &repair_id,
                "target-after-squash-result",
                Path::new("AGENTS.md"),
                "instruction-a-after",
                "targets:2",
                TimestampMs::new(19),
            )
            .expect("refresh");

        let repair = controller.repair(&repair_id).expect("repair");
        assert_eq!(controller.repair_attempts.len(), 1);
        assert_eq!(repair.requested_change_count, 1);
        assert_eq!(repair.pull_request_id.as_deref(), Some("42"));
        assert_eq!(repair.status, ParentRepairStatus::Completed);
        assert_eq!(
            controller.targets[&repair.repository_id].target_commit,
            "target-after-squash-result"
        );
        assert_eq!(controller.state, ParentIntegrationState::Integrating);
    }

    #[test]
    fn provider_failures_are_typed_and_resumable() {
        for (mut snapshot, expected) in [
            {
                let mut snapshot = review_snapshot();
                snapshot.provider_available = false;
                (snapshot, ParentRepairStatus::ProviderUnavailable)
            },
            {
                let mut snapshot = review_snapshot();
                snapshot.open = false;
                (snapshot, ParentRepairStatus::ExternallyClosed)
            },
            {
                let mut snapshot = review_snapshot();
                snapshot.head_commit = Some("foreign".to_owned());
                (snapshot, ParentRepairStatus::ForcePushed)
            },
            {
                let mut snapshot = review_snapshot();
                snapshot.merge_conflict = true;
                (snapshot, ParentRepairStatus::MergeConflict)
            },
            {
                let mut snapshot = review_snapshot();
                snapshot.checks_failed = true;
                (snapshot, ParentRepairStatus::FailedChecks)
            },
            {
                let mut snapshot = review_snapshot();
                snapshot.review_rejected = true;
                (snapshot, ParentRepairStatus::ReviewRejected)
            },
        ] {
            let mut controller = controller();
            let repair_id = begin_repair(&mut controller);
            controller
                .record_repair_push(
                    &repair_id,
                    "repair-commit-1",
                    "targets:1",
                    TimestampMs::new(11),
                )
                .expect("push");
            controller
                .record_repair_pull_request(
                    &repair_id,
                    "42",
                    "https://github.com/example/a/pull/42",
                    "targets:1",
                    TimestampMs::new(12),
                )
                .expect("pull request");
            snapshot.pull_request_id = Some("42".to_owned());
            controller
                .reconcile_repair_provider(&repair_id, snapshot, "targets:1", TimestampMs::new(13))
                .expect("typed block");
            let status = controller.repair(&repair_id).expect("repair").status;
            assert_eq!(status, expected);
            assert!(status.resumable());
            assert!(matches!(
                controller.state,
                ParentIntegrationState::Blocked { .. }
            ));
        }
    }

    #[test]
    fn provider_outage_recovers_to_the_same_pull_request() {
        let mut controller = controller();
        let repair_id = begin_repair(&mut controller);
        controller
            .record_repair_push(
                &repair_id,
                "repair-commit-1",
                "targets:1",
                TimestampMs::new(11),
            )
            .expect("push");
        controller
            .record_repair_pull_request(
                &repair_id,
                "42",
                "https://github.com/example/a/pull/42",
                "targets:1",
                TimestampMs::new(12),
            )
            .expect("pull request");
        let mut unavailable = review_snapshot();
        unavailable.provider_available = false;
        controller
            .reconcile_repair_provider(&repair_id, unavailable, "targets:1", TimestampMs::new(13))
            .expect("outage");
        controller
            .reconcile_repair_provider(
                &repair_id,
                review_snapshot(),
                "targets:1",
                TimestampMs::new(14),
            )
            .expect("provider recovery");
        assert_eq!(
            controller.repair(&repair_id).expect("repair").status,
            ParentRepairStatus::AwaitingReview
        );
        assert!(matches!(
            controller.state,
            ParentIntegrationState::AwaitingFixReview {
                ref pull_request_id,
                ..
            } if pull_request_id == "42"
        ));
    }

    #[test]
    fn provider_outage_before_pull_request_recovers_to_publication() {
        let mut controller = controller();
        let repair_id = begin_repair(&mut controller);
        controller
            .record_repair_push(
                &repair_id,
                "repair-commit-1",
                "targets:1",
                TimestampMs::new(11),
            )
            .expect("push");
        let mut unavailable = review_snapshot();
        unavailable.provider_available = false;
        unavailable.pull_request_id = None;
        unavailable.pull_request_url = None;
        unavailable.open = false;
        controller
            .reconcile_repair_provider(&repair_id, unavailable, "targets:1", TimestampMs::new(12))
            .expect("outage");
        let mut recovered = review_snapshot();
        recovered.pull_request_id = None;
        recovered.pull_request_url = None;
        recovered.open = false;
        controller
            .reconcile_repair_provider(&repair_id, recovered, "targets:1", TimestampMs::new(13))
            .expect("provider recovery");
        assert_eq!(
            controller.repair(&repair_id).expect("repair").status,
            ParentRepairStatus::AwaitingPullRequest
        );
        assert!(matches!(
            controller.state,
            ParentIntegrationState::Fixing { .. }
        ));
    }

    #[test]
    fn restart_during_repair_implementation_stays_in_fixing() {
        for terminal_manifest in [false, true] {
            let mut controller = controller();
            let repair_id = begin_repair(&mut controller);
            controller
                .record_repair_branch_ready(&repair_id)
                .expect("repair branch");
            let attempt_id = controller
                .start_attempt_intent(
                    "repair implementation",
                    format!("repair-turn-{terminal_manifest}"),
                    ParentAttemptRoot::CheckoutHandle("checkout-a".to_owned()),
                    1_000,
                    "targets:1",
                    TimestampMs::new(11),
                )
                .expect("repair attempt");

            if terminal_manifest {
                controller
                    .reconcile_terminal_run_after_restart(TimestampMs::new(12))
                    .expect("terminal manifest recovery");
            } else {
                controller
                    .reconcile_restart(false, TimestampMs::new(12))
                    .expect("missing harness recovery");
            }

            assert!(matches!(
                controller.state,
                ParentIntegrationState::Fixing { .. }
            ));
            assert_eq!(
                controller
                    .attempts
                    .iter()
                    .find(|attempt| attempt.id == attempt_id)
                    .map(|attempt| attempt.status),
                Some(ParentAttemptStatus::Indeterminate)
            );
            assert_eq!(
                controller.repair(&repair_id).expect("repair").status,
                ParentRepairStatus::Implementing
            );
        }
    }

    #[test]
    fn repairs_across_repositories_preserve_prior_attempt_evidence() {
        let mut controller = controller();
        let first_id = begin_repair(&mut controller);
        controller
            .record_repair_push(&first_id, "repair-a", "targets:1", TimestampMs::new(11))
            .expect("push a");
        controller
            .record_repair_pull_request(
                &first_id,
                "42",
                "https://github.com/example/a/pull/42",
                "targets:1",
                TimestampMs::new(12),
            )
            .expect("pr a");
        let mut merged = review_snapshot();
        merged.head_commit = Some("repair-a".to_owned());
        merged.open = false;
        merged.merged = true;
        merged.merge_result_commit = Some("squash-a".to_owned());
        merged.target_contains_merge_result = true;
        controller
            .reconcile_repair_provider(&first_id, merged, "targets:1", TimestampMs::new(13))
            .expect("merge a");
        controller
            .record_repair_refresh(
                &first_id,
                "squash-a",
                Path::new("AGENTS.md"),
                "instruction-a-2",
                "targets:2",
                TimestampMs::new(14),
            )
            .expect("refresh a");

        let second_id = controller
            .begin_repair(
                "repair:defect-b",
                CanonicalRepositoryId::new("github:repository:b").expect("repository b"),
                "checkout-b",
                "commit-b",
                "targets:2",
                TimestampMs::new(15),
            )
            .expect("begin b");
        assert_ne!(first_id, second_id);
        assert_eq!(controller.repair_attempts.len(), 2);
        assert_eq!(
            controller.repair_attempts[0].status,
            ParentRepairStatus::Completed
        );
        assert_eq!(
            controller.repair_attempts[0].pull_request_id.as_deref(),
            Some("42")
        );
        assert_eq!(
            controller.repair_attempts[1].repository_id.as_str(),
            "github:repository:b"
        );
    }
}
