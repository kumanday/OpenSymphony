use std::collections::{BTreeMap, BTreeSet};

use crate::opensymphony_domain::{CanonicalRepositoryId, IssueId, TimestampMs};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_TRANSITIONS: usize = 256;
const MAX_ATTEMPTS: usize = 64;
const MAX_LOG_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentIntegrationState {
    WaitingForChildren,
    WaitingForChildMerges,
    AcquiringChildLeases,
    PreparingIntegrationWorkspace,
    RefreshingRepositories,
    Integrating,
    FinalVerification,
    Finalizing,
    Completed,
    Blocked { reason: String },
    Failed { reason: String },
    Canceled { reason: String },
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParentAttemptRoot {
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
pub struct ParentVerificationAttempt {
    pub id: String,
    pub name: String,
    pub idempotency_key: String,
    pub root: ParentAttemptRoot,
    pub conversation_id: String,
    pub started_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<TimestampMs>,
    pub timeout_ms: u64,
    pub status: ParentAttemptStatus,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentRepositoryTarget {
    pub repository_id: CanonicalRepositoryId,
    pub checkout_handle: String,
    pub target_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentFinalEvidence {
    pub attempt_id: String,
    pub repository_commits: BTreeMap<CanonicalRepositoryId, String>,
    pub recorded_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentIntegrationController {
    pub parent_id: IssueId,
    pub hierarchy_generation: u64,
    pub state_version: u64,
    pub state: ParentIntegrationState,
    #[serde(default)]
    pub transitions: Vec<ParentTransition>,
    #[serde(default)]
    pub targets: BTreeMap<CanonicalRepositoryId, ParentRepositoryTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub attempts: Vec<ParentVerificationAttempt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_evidence: Option<ParentFinalEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParentIntegrationError {
    #[error("parent controller generation must be greater than zero")]
    InvalidGeneration,
    #[error("invalid parent transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: ParentIntegrationState,
        to: ParentIntegrationState,
    },
    #[error("idempotency key `{0}` was reused with different transition facts")]
    IdempotencyConflict(String),
    #[error("parent controller is bound to conversation `{expected}`, not `{actual}`")]
    ConversationMismatch { expected: String, actual: String },
    #[error("checkout handle `{0}` is not present in the verified parent target map")]
    UnknownCheckoutHandle(String),
    #[error("verification attempt `{0}` was not found")]
    UnknownAttempt(String),
    #[error("verification attempt `{0}` is already terminal")]
    AttemptAlreadyTerminal(String),
    #[error("verification attempt `{0}` cannot start before cleanup and baseline verification")]
    AttemptNotReady(String),
    #[error("resource `{kind}:{identifier}` is already allocated")]
    ResourceCollision { kind: String, identifier: String },
    #[error("final verification requires a passed attempt and at least one repository target")]
    FinalVerificationIncomplete,
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
            transitions: Vec::new(),
            targets: BTreeMap::new(),
            conversation_id: None,
            attempts: Vec::new(),
            final_evidence: None,
        })
    }

    pub fn admit(
        &mut self,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
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
        self.transition(
            ParentIntegrationState::Integrating,
            "all parent integration checkouts match their recorded target commits",
            format!(
                "parent:{}:{}:refresh:{input_version}",
                self.parent_id, self.hierarchy_generation
            ),
            input_version,
            Some(intent(
                "refresh_verified_baseline",
                format!(
                    "parent:{}:{}:refresh:{input_version}",
                    self.parent_id, self.hierarchy_generation
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
        let name = name.into();
        if !matches!(
            self.state,
            ParentIntegrationState::Integrating | ParentIntegrationState::FinalVerification
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
        let conversation_id = conversation_id.into();
        if let Some(expected) = &self.conversation_id {
            if expected != &conversation_id {
                return Err(ParentIntegrationError::ConversationMismatch {
                    expected: expected.clone(),
                    actual: conversation_id,
                });
            }
        } else {
            self.conversation_id = Some(conversation_id.clone());
        }
        let idempotency_key = idempotency_key.into();
        if let Some(existing) = self
            .attempts
            .iter()
            .find(|attempt| attempt.idempotency_key == idempotency_key)
        {
            return Ok(existing.id.clone());
        }
        let id = format!("parent-attempt-{}", self.attempts.len().saturating_add(1));
        self.attempts.push(ParentVerificationAttempt {
            id: id.clone(),
            name,
            idempotency_key,
            root,
            conversation_id,
            started_at,
            finished_at: None,
            timeout_ms,
            status: ParentAttemptStatus::Running,
            exit_code: None,
            bounded_log: String::new(),
            log_truncated: false,
            resources: Vec::new(),
            cleanup: None,
            input_version: input_version.into(),
        });
        if self.attempts.len() > MAX_ATTEMPTS {
            let removable = self
                .attempts
                .iter()
                .position(|attempt| attempt.status.terminal())
                .unwrap_or(0);
            self.attempts.remove(removable);
        }
        Ok(id)
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
            attempt.resources.iter().rev().any(|resource| {
                resource.kind == kind
                    && resource.identifier == identifier
                    && resource.status == ParentResourceStatus::Allocated
            })
        });
        if collision {
            self.attempt_mut(attempt_id)?
                .resources
                .push(ParentResourceReceipt {
                    kind: kind.clone(),
                    identifier: identifier.clone(),
                    status: ParentResourceStatus::Collision,
                    occurred_at,
                    detail: Some("resource is owned by another parent attempt".to_owned()),
                });
            return Err(ParentIntegrationError::ResourceCollision { kind, identifier });
        }
        self.attempt_mut(attempt_id)?
            .resources
            .push(ParentResourceReceipt {
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
        self.attempt_mut(attempt_id)?
            .resources
            .push(ParentResourceReceipt {
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
        let has_allocated_resources = active_resource_keys(&self.attempts[index]).next().is_some();
        self.attempts[index].status = ParentAttemptStatus::Indeterminate;
        self.attempts[index].finished_at = Some(occurred_at);
        self.attempts[index].cleanup = Some(ParentCleanupReceipt {
            status: if has_allocated_resources {
                ParentCleanupStatus::Pending
            } else {
                ParentCleanupStatus::Succeeded
            },
            occurred_at,
            detail: Some("harness state was unavailable after restart".to_owned()),
        });
        self.prepare_retry(
            &attempt_id,
            "nonterminal parent attempt became indeterminate after restart",
            occurred_at,
        )
    }

    pub fn complete(
        &mut self,
        attempt_id: &str,
        input_version: &str,
        occurred_at: TimestampMs,
    ) -> Result<(), ParentIntegrationError> {
        if self.targets.is_empty()
            || !self
                .attempts
                .iter()
                .any(|attempt| attempt.id == attempt_id && attempt.status.passed())
        {
            return Err(ParentIntegrationError::FinalVerificationIncomplete);
        }
        self.transition(
            ParentIntegrationState::FinalVerification,
            "parent integration attempt passed",
            format!("{attempt_id}:final-verification"),
            input_version,
            Some(intent(
                "run_final_verification",
                format!("{attempt_id}:final-verification"),
            )),
            Some(receipt("passed", None)),
            ParentRetryClassification::Retryable,
            occurred_at,
        )?;
        let repository_commits = self
            .targets
            .iter()
            .map(|(repository_id, target)| (repository_id.clone(), target.target_commit.clone()))
            .collect();
        self.final_evidence = Some(ParentFinalEvidence {
            attempt_id: attempt_id.to_owned(),
            repository_commits,
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

    fn cleanup_complete(&self) -> bool {
        self.attempts.iter().all(|attempt| {
            if matches!(
                attempt.status,
                ParentAttemptStatus::TimedOut
                    | ParentAttemptStatus::Canceled
                    | ParentAttemptStatus::Indeterminate
            ) {
                attempt
                    .cleanup
                    .as_ref()
                    .is_some_and(|cleanup| cleanup.status == ParentCleanupStatus::Succeeded)
                    && active_resource_keys(attempt).next().is_none()
            } else {
                true
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
                from: self.state.clone(),
                to: next,
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

fn active_resource_keys(attempt: &ParentVerificationAttempt) -> impl Iterator<Item = (&str, &str)> {
    let released = attempt
        .resources
        .iter()
        .filter(|resource| {
            matches!(
                resource.status,
                ParentResourceStatus::Released | ParentResourceStatus::CleanupFailed
            )
        })
        .map(|resource| (resource.kind.as_str(), resource.identifier.as_str()))
        .collect::<BTreeSet<_>>();
    attempt
        .resources
        .iter()
        .filter(|resource| resource.status == ParentResourceStatus::Allocated)
        .map(|resource| (resource.kind.as_str(), resource.identifier.as_str()))
        .filter(move |resource| !released.contains(resource))
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
            | (State::Integrating, State::FinalVerification)
            | (State::FinalVerification, State::Integrating)
            | (State::FinalVerification, State::Finalizing)
            | (State::Finalizing, State::Completed)
            | (State::Blocked { .. }, State::RefreshingRepositories)
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

#[cfg(test)]
mod tests {
    use super::*;

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
                        target_commit: format!("commit-{repo}"),
                    }),
                "targets:1",
                TimestampMs::new(2),
            )
            .expect("workspace");
        controller
    }

    fn finish_passed(controller: &mut ParentIntegrationController, id: &str) {
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
        let mut controller = controller();
        assert!(
            controller
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
        assert_eq!(
            controller.start_attempt(
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
            .expect("resource-free cleanup permits verified rerun");
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
        let second = controller
            .start_attempt(
                "parallel check",
                "attempt:2",
                ParentAttemptRoot::ParentRoot,
                "conversation-1",
                100,
                "targets:1",
                TimestampMs::new(3),
            )
            .expect("second");
        assert!(matches!(
            controller.allocate_resource(&second, "port", "24001", TimestampMs::new(3)),
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
}
