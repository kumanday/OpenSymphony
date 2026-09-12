use std::{collections::BTreeMap, path::PathBuf};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, IssueId, ParentVerificationEvidence, TimestampMs,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

const MAX_TRANSITIONS: usize = 256;
const MAX_ATTEMPTS: usize = 64;
const MAX_LOG_BYTES: usize = 16 * 1024;
const MAX_COMMAND_BYTES: usize = 4 * 1024;
const MAX_COMMAND_ID_BYTES: usize = 256;
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
    pub command: String,
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
    pub target_commit: String,
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
    #[serde(default)]
    pub next_attempt_sequence: u64,
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
            next_attempt_sequence: 0,
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
            command.command == evidence.command
                && match &command.root {
                    ParentAttemptRoot::ParentRoot => evidence.root == "parent_root",
                    ParentAttemptRoot::CheckoutHandle(handle) => evidence.root == *handle,
                }
                && command.finished_at.is_some()
                && command.exit_code.is_some()
        });
        let selected_exit_code = selected_command.and_then(|command| command.exit_code);
        if evidence.command.trim().is_empty() || selected_exit_code.is_none() {
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
            return if existing.command == command && existing.root == root {
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
            command: command.to_owned(),
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
        let Some(command_index) = attempt
            .commands
            .iter()
            .rposition(|command| command.command_id == command_id)
        else {
            return Ok(());
        };
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
        self.prepare_retry(
            &attempt_id,
            "nonterminal parent attempt became indeterminate after restart",
            occurred_at,
        )
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
        self.prepare_retry(
            &attempt_id,
            "terminal harness outcome was not durably applied before restart",
            occurred_at,
        )
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
        if expected_commits.is_empty()
            || !self.attempts.iter().any(|attempt| {
                attempt.id == attempt_id
                    && attempt.status.passed()
                    && attempt.input_version == input_version
                    && attempt.exit_code == Some(0)
                    && attempt
                        .cleanup
                        .as_ref()
                        .is_some_and(|cleanup| cleanup.status == ParentCleanupStatus::Succeeded)
                    && attempt.verified_repository_commits == expected_commits
            })
        {
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
                        relative_path: PathBuf::from(format!("repositories/{repo}")),
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
            root: "parent_root".to_owned(),
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
            root: "parent_root".to_owned(),
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
}
