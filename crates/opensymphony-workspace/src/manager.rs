use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use chrono::Utc;
#[cfg(unix)]
use rustix::{
    io::Errno,
    process::{Pid, Signal, kill_process_group},
};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::{Instant, timeout},
};
use url::Url;
use uuid::Uuid;

use super::{
    CheckoutManifest, CheckoutRepository, CleanupDecision, CleanupOutcome, ConversationManifest,
    EnsureWorkspaceResult, HookDefinition, HookExecutionRecord, HookExecutionStatus, HookKind,
    IssueContextArtifact, IssueDescriptor, IssueLifecycleState, IssueManifest,
    PromptCaptureDescriptor, PromptCaptureManifest, RunDescriptor, RunManifest, RunStatus,
    SessionContextArtifact, TerminalRuntimeEnvelope, WorkspaceError, WorkspaceHandle,
    WorkspaceManagerConfig, WorkspaceOwnershipConflictDetails,
    models::{AfterCreateBootstrapReceipt, InstructionProvenance, redact_runtime_diagnostic},
    paths::{
        checkout_workspace_key, normalize_absolute_path, resolve_path_within_root,
        sanitize_workspace_key,
    },
};
use crate::opensymphony_domain::{RepositoryBinding, SafeRemoteFingerprint};

pub struct WorkspaceManager {
    config: WorkspaceManagerConfig,
    legacy_repository: Option<crate::opensymphony_domain::CanonicalRepositoryId>,
    checkout_repositories: BTreeMap<String, CheckoutRepository>,
    checkout_credential_envs: BTreeSet<String>,
}

struct HookFailure {
    error: WorkspaceError,
    record: HookExecutionRecord,
}

enum ExistingIssueManifestState {
    Missing,
    Owned(IssueManifest),
    ForeignArtifact,
    Conflict(IssueManifest),
}

enum ExistingReceiptState {
    Missing,
    Owned,
    ForeignArtifact,
    Conflict(Box<AfterCreateBootstrapReceipt>),
}

enum ExistingWorkspaceState {
    Missing,
    Owned,
    AfterCreateCompleted,
    ForeignArtifact,
    Conflict(WorkspaceOwnershipClaim),
}

struct WorkspaceOwnershipClaim {
    issue_id: String,
    identifier: String,
}

async fn checkout_operation_with_timeout<T, F>(
    checkout_timeout: Option<Duration>,
    path: &Path,
    operation: &str,
    future: F,
) -> Result<T, WorkspaceError>
where
    F: Future<Output = Result<T, WorkspaceError>>,
{
    match checkout_timeout {
        Some(timeout_duration) => match timeout(timeout_duration, future).await {
            Ok(result) => result,
            Err(_) => Err(WorkspaceError::CheckoutOperation {
                operation: operation.to_owned(),
                path: path.to_path_buf(),
                detail: format!("checkout acquisition timed out after {timeout_duration:?}"),
            }),
        },
        None => future.await,
    }
}

fn checkout_deadline(timeout_duration: Option<Duration>) -> Option<Instant> {
    timeout_duration.map(|duration| Instant::now() + duration)
}

fn checkout_time_remaining(deadline: Option<Instant>) -> Option<Duration> {
    deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()))
}

enum HookCommandOutput {
    Completed(std::process::Output),
    TimedOut { stdout: String, stderr: String },
}

struct StagingCleanupGuard {
    paths: Vec<PathBuf>,
}

impl StagingCleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self { paths: vec![path] }
    }

    fn register(&mut self, path: PathBuf) {
        self.paths.push(path);
    }

    fn disarm(&mut self) {
        self.paths.clear();
    }
}

impl Drop for StagingCleanupGuard {
    fn drop(&mut self) {
        let paths = std::mem::take(&mut self.paths);
        if paths.is_empty() {
            return;
        }
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            drop(handle.spawn_blocking(move || {
                for path in paths {
                    let _ = std::fs::remove_file(&path);
                    let _ = std::fs::remove_dir_all(&path);
                }
            }));
        } else {
            for path in paths {
                let _ = std::fs::remove_file(&path);
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }
}

impl WorkspaceManager {
    pub fn new(mut config: WorkspaceManagerConfig) -> Result<Self, WorkspaceError> {
        config.root = normalize_absolute_path(&config.root)?;
        Ok(Self {
            config,
            legacy_repository: None,
            checkout_repositories: BTreeMap::new(),
            checkout_credential_envs: BTreeSet::new(),
        })
    }

    pub fn with_legacy_repository(
        mut self,
        legacy_repository: Option<crate::opensymphony_domain::CanonicalRepositoryId>,
    ) -> Self {
        self.legacy_repository = legacy_repository;
        self
    }

    pub fn with_repository_checkouts(
        mut self,
        repositories: BTreeMap<String, CheckoutRepository>,
    ) -> Self {
        self.checkout_credential_envs =
            super::checkout_credential_environment_variables(&repositories);
        self.checkout_repositories = repositories;
        self
    }

    pub fn config(&self) -> &WorkspaceManagerConfig {
        &self.config
    }

    pub fn checkout_credential_envs(&self) -> &BTreeSet<String> {
        &self.checkout_credential_envs
    }

    pub fn workspace_path_for(&self, issue_identifier: &str) -> Result<PathBuf, WorkspaceError> {
        super::workspace_path_for_root(&self.config.root, issue_identifier)
    }

    pub async fn ensure(
        &self,
        issue: &IssueDescriptor,
    ) -> Result<EnsureWorkspaceResult, WorkspaceError> {
        self.ensure_with_run_id(issue, None).await
    }

    pub async fn ensure_with_checkout_timeout(
        &self,
        issue: &IssueDescriptor,
        checkout_timeout: Duration,
    ) -> Result<EnsureWorkspaceResult, WorkspaceError> {
        if let Some(binding) = issue
            .repository_binding
            .as_ref()
            .and_then(crate::opensymphony_domain::RepositoryBindingOutcome::resolved_binding)
            && !self.checkout_repositories.is_empty()
        {
            let repository = self.checkout_repository_for_binding(binding)?;
            return self
                .ensure_verified_checkout_for_run(
                    issue,
                    binding,
                    repository,
                    None,
                    Some(checkout_timeout),
                )
                .await;
        }

        self.ensure(issue).await
    }

    pub async fn ensure_with_run_id(
        &self,
        issue: &IssueDescriptor,
        run_id: Option<&str>,
    ) -> Result<EnsureWorkspaceResult, WorkspaceError> {
        if let Some(binding) = issue
            .repository_binding
            .as_ref()
            .and_then(crate::opensymphony_domain::RepositoryBindingOutcome::resolved_binding)
            && !self.checkout_repositories.is_empty()
        {
            let repository = self.checkout_repository_for_binding(binding)?;
            return self
                .ensure_verified_checkout_for_run(issue, binding, repository, run_id, None)
                .await;
        }

        self.create_directory(&self.config.root).await?;
        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let workspace_key = sanitize_workspace_key(&issue.identifier)?;
        let workspace_path = super::workspace_path_for_root(&canonical_root, &issue.identifier)?;

        self.reject_symlinked_workspace_root(&workspace_path)
            .await?;
        self.create_directory(&workspace_path).await?;
        self.reject_symlinked_workspace_root(&workspace_path)
            .await?;
        let canonical_workspace = self.canonicalize_path(&workspace_path).await?;
        ensure_descendant(&canonical_root, &canonical_workspace)?;

        let handle = WorkspaceHandle::new(
            issue.issue_id.clone(),
            issue.identifier.clone(),
            workspace_key,
            canonical_workspace,
        );
        let existing_state = self.inspect_workspace_state(issue, &handle).await?;
        if let ExistingWorkspaceState::Conflict(claim) = &existing_state {
            return Err(WorkspaceError::WorkspaceOwnershipConflict {
                details: Box::new(WorkspaceOwnershipConflictDetails {
                    workspace: handle.workspace_path().to_path_buf(),
                    workspace_key: handle.workspace_key().to_string(),
                    existing_issue_id: claim.issue_id.clone(),
                    existing_identifier: claim.identifier.clone(),
                    requested_issue_id: issue.issue_id.clone(),
                    requested_identifier: issue.identifier.clone(),
                }),
            });
        }

        if matches!(
            &existing_state,
            ExistingWorkspaceState::Owned | ExistingWorkspaceState::AfterCreateCompleted
        ) {
            let existing_repository = self
                .load_issue_manifest(&handle)
                .await?
                .and_then(|manifest| manifest.repository_binding)
                .and_then(|binding| binding.repository_id().cloned())
                .map(|repository| repository.to_string());
            let historical_repository = self
                .load_run_manifest(&handle)
                .await?
                .and_then(|manifest| manifest.repository_binding)
                .map(|binding| binding.repository_id().to_string());
            let receipt_repository = self
                .load_manifest::<AfterCreateBootstrapReceipt>(
                    &handle,
                    &handle.after_create_receipt_path(),
                )
                .await?
                .and_then(|receipt| receipt.repository_binding)
                .map(|binding| binding.repository_id().to_string());
            let historical_repository = historical_repository.or(receipt_repository);
            let requested_repository = issue
                .repository_binding
                .as_ref()
                .and_then(|binding| binding.repository_id().cloned())
                .map(|repository| repository.to_string());
            let configured_legacy_repository =
                self.legacy_repository.as_ref().map(ToString::to_string);
            let legacy_repository_mismatch = configured_legacy_repository
                .as_ref()
                .is_some_and(|configured| Some(configured) != requested_repository.as_ref());
            if existing_repository != requested_repository
                && requested_repository.is_some()
                && (existing_repository.is_some()
                    || historical_repository != requested_repository
                    || legacy_repository_mismatch)
            {
                return Err(WorkspaceError::RepositoryBindingMismatch {
                    workspace: handle.workspace_path().to_path_buf(),
                    existing_repository,
                    requested_repository,
                });
            }
        }

        let created = matches!(
            existing_state,
            ExistingWorkspaceState::Missing | ExistingWorkspaceState::ForeignArtifact
        );
        let after_create = if created {
            match self.execute_hook(HookKind::AfterCreate, &handle).await {
                Ok(record) => {
                    if record.is_some() {
                        self.write_after_create_receipt(issue, &handle).await?;
                    }
                    record
                }
                Err(failure) => return Err(failure.error),
            }
        } else {
            None
        };
        self.bootstrap_workspace_layout(&handle).await?;
        let issue_manifest = self.upsert_issue_manifest(issue, &handle).await?;

        Ok(EnsureWorkspaceResult {
            handle,
            issue_manifest,
            created,
            after_create,
        })
    }

    fn checkout_repository_for_binding(
        &self,
        binding: &RepositoryBinding,
    ) -> Result<&CheckoutRepository, WorkspaceError> {
        self.checkout_repositories
            .get(binding.repository_id().as_str())
            .ok_or_else(|| {
                checkout_verification(
                    &self.config.root,
                    "resolved repository binding has no configured checkout policy",
                )
            })
    }

    /// Create or reuse one immutable, verified checkout for a bound issue.
    pub async fn ensure_verified_checkout(
        &self,
        issue: &IssueDescriptor,
        binding: &RepositoryBinding,
        repository: &CheckoutRepository,
    ) -> Result<EnsureWorkspaceResult, WorkspaceError> {
        self.ensure_verified_checkout_for_run(issue, binding, repository, None, None)
            .await
    }

    async fn ensure_verified_checkout_for_run(
        &self,
        issue: &IssueDescriptor,
        binding: &RepositoryBinding,
        repository: &CheckoutRepository,
        run_id: Option<&str>,
        checkout_timeout: Option<Duration>,
    ) -> Result<EnsureWorkspaceResult, WorkspaceError> {
        let mut checkout_deadline = checkout_deadline(checkout_timeout);
        self.create_directory(&self.config.root).await?;
        let workspace_key = checkout_workspace_key(
            &issue.identifier,
            &issue.issue_id,
            binding.repository_id().as_str(),
        )?;

        let compatible = checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            &self.config.root,
            "scan retained checkout generations",
            self.find_compatible_checkout(issue, binding, checkout_deadline),
        )
        .await?;
        if let Some(existing) = compatible {
            if let Some(run_id) = run_id {
                self.update_checkout_run_id(&existing.handle, run_id)
                    .await?;
            }
            return Ok(existing);
        }

        let generation = Uuid::new_v4().simple().to_string();
        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let published_path = canonical_root.join(format!("{workspace_key}--{generation}"));
        let staging_root = canonical_root.join(".opensymphony-staging");
        self.reject_symlinked_workspace_root(&staging_root).await?;
        self.create_directory(&staging_root).await?;
        let canonical_staging_root = self.canonicalize_path(&staging_root).await?;
        ensure_descendant(
            &self.canonicalize_path(&self.config.root).await?,
            &canonical_staging_root,
        )?;
        let staging_path = staging_root.join(format!("{workspace_key}--{generation}"));
        self.reject_symlinked_workspace_root(&staging_path).await?;
        let mut staging_cleanup = StagingCleanupGuard::new(staging_path.clone());
        let clone_result = match checkout_time_remaining(checkout_deadline) {
            Some(timeout_duration) => match timeout(
                timeout_duration,
                self.run_git_clone(repository, &staging_path, &mut staging_cleanup),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err(WorkspaceError::CheckoutOperation {
                    operation: "acquire verified checkout".to_owned(),
                    path: staging_path.clone(),
                    detail: format!("checkout acquisition timed out after {timeout_duration:?}"),
                }),
            },
            None => {
                self.run_git_clone(repository, &staging_path, &mut staging_cleanup)
                    .await
            }
        };
        if clone_result.is_err() {
            let _ = fs::remove_dir_all(&staging_path).await;
        }
        clone_result?;

        let facts = match checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            &staging_path,
            "verify acquired checkout",
            self.verify_git_checkout(&staging_path, binding, repository, true, true),
        )
        .await
        {
            Ok(facts) => facts,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_path).await;
                return Err(error);
            }
        };
        let instruction = match checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            &staging_path,
            "discover checkout instructions",
            self.load_instruction_provenance(&staging_path, repository, &facts.head),
        )
        .await
        {
            Ok(instruction) => instruction,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_path).await;
                return Err(error);
            }
        };
        if let Err(error) = checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            &staging_path,
            "prepare checkout metadata",
            self.exclude_metadata_from_git(&staging_path),
        )
        .await
        {
            let _ = fs::remove_dir_all(&staging_path).await;
            return Err(error);
        }

        let staging_path = match self.canonicalize_path(&staging_path).await {
            Ok(path) => path,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging_path).await;
                return Err(error);
            }
        };
        let staging_workspace = WorkspaceHandle::new(
            issue.issue_id.clone(),
            issue.identifier.clone(),
            workspace_key,
            staging_path,
        )
        .with_checkout_generation(generation.clone());
        let after_create_started = Instant::now();
        let after_create = match self
            .execute_hook(HookKind::AfterCreate, &staging_workspace)
            .await
        {
            Ok(record) => record,
            Err(failure) => {
                let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
                return Err(failure.error);
            }
        };
        if let Some(deadline) = checkout_deadline.as_mut() {
            *deadline += after_create_started.elapsed();
        }
        if let Err(error) = checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            staging_workspace.workspace_path(),
            "verify checkout after creation hook",
            self.verify_git_checkout(
                staging_workspace.workspace_path(),
                binding,
                repository,
                true,
                true,
            ),
        )
        .await
        {
            let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
            return Err(error);
        }
        if let Err(error) = self.bootstrap_workspace_layout(&staging_workspace).await {
            let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
            return Err(error);
        }

        let now = Utc::now();
        let checkout_manifest = CheckoutManifest {
            schema_version: 1,
            generation: generation.clone(),
            issue_id: issue.issue_id.clone(),
            identifier: issue.identifier.clone(),
            run_id: run_id.unwrap_or(&issue.issue_id).to_owned(),
            sanitized_workspace_key: staging_workspace.workspace_key().to_owned(),
            workspace_path: published_path.clone(),
            repository_binding: binding.clone(),
            remote_fingerprint: binding
                .repository
                .safe_remote_fingerprint
                .as_str()
                .to_owned(),
            target_branch: facts.branch.clone(),
            target_commit: facts.head.clone(),
            current_branch: facts.branch,
            head: facts.head,
            shallow: facts.shallow,
            clean: facts.clean,
            instruction,
            created_at: now,
            verified_at: now,
            quarantined: false,
            quarantine_reason: None,
        };
        if let Err(error) = self
            .write_manifest(
                &staging_workspace,
                &staging_workspace.checkout_manifest_path(),
                &checkout_manifest,
            )
            .await
        {
            let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
            return Err(error);
        }
        let issue_manifest = match self
            .upsert_issue_manifest_at_path(issue, &staging_workspace, &published_path)
            .await
        {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
                return Err(error);
            }
        };
        if after_create.is_some()
            && let Err(error) = self
                .write_after_create_receipt_at_path(issue, &staging_workspace, &published_path)
                .await
        {
            let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
            return Err(error);
        }

        if let Err(source) = fs::rename(staging_workspace.workspace_path(), &published_path).await {
            let _ = fs::remove_dir_all(staging_workspace.workspace_path()).await;
            return Err(WorkspaceError::CheckoutOperation {
                operation: "publish checkout generation".to_owned(),
                path: published_path.clone(),
                detail: source.to_string(),
            });
        }
        staging_cleanup.disarm();
        let workspace = WorkspaceHandle::new(
            issue.issue_id.clone(),
            issue.identifier.clone(),
            staging_workspace.workspace_key().to_owned(),
            self.canonicalize_path(&published_path).await?,
        )
        .with_checkout_generation(generation);
        let after_create = after_create.map(|mut record| {
            if let Ok(relative) = record.cwd.strip_prefix(staging_workspace.workspace_path()) {
                record.cwd = workspace.workspace_path().join(relative);
            }
            record
        });
        if let Err(error) = checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            workspace.workspace_path(),
            "verify published checkout",
            self.verify_checkout(&workspace),
        )
        .await
        {
            self.remove_published_checkout(&workspace).await;
            return Err(error);
        }

        Ok(EnsureWorkspaceResult {
            handle: workspace,
            issue_manifest,
            created: true,
            after_create,
        })
    }

    pub async fn verify_checkout(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        self.verify_checkout_with_worker_changes(workspace, false)
            .await
    }

    pub async fn verify_runtime_envelope(
        &self,
        workspace: &WorkspaceHandle,
        expected: &TerminalRuntimeEnvelope,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        self.verify_runtime_envelope_with_worker_changes(workspace, expected, false)
            .await
    }

    pub async fn verify_runtime_envelope_for_retry(
        &self,
        workspace: &WorkspaceHandle,
        expected: &TerminalRuntimeEnvelope,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        self.verify_runtime_envelope_with_worker_changes(workspace, expected, true)
            .await
    }

    async fn verify_runtime_envelope_with_worker_changes(
        &self,
        workspace: &WorkspaceHandle,
        expected: &TerminalRuntimeEnvelope,
        allow_worker_changes: bool,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        let manifest = self
            .verify_checkout_with_worker_changes(workspace, allow_worker_changes)
            .await?;
        if manifest.repository_binding != expected.repository_binding
            || manifest.generation != expected.checkout_generation
            || workspace.workspace_path() != expected.checkout_path
            || manifest.target_branch != expected.target_branch
            || manifest.target_commit != expected.target_commit
            || manifest.instruction != expected.instruction
        {
            return Err(WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: manifest.generation,
                reason: "runtime envelope does not match the verified checkout".to_owned(),
            });
        }
        Ok(manifest)
    }

    async fn verify_checkout_with_worker_changes(
        &self,
        workspace: &WorkspaceHandle,
        allow_worker_changes: bool,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        self.verify_checkout_with_worker_changes_timeout(workspace, allow_worker_changes, None)
            .await
    }

    async fn verify_checkout_with_worker_changes_timeout(
        &self,
        workspace: &WorkspaceHandle,
        allow_worker_changes: bool,
        checkout_deadline: Option<Instant>,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        let manifest = self
            .load_manifest::<CheckoutManifest>(workspace, &workspace.checkout_manifest_path())
            .await?
            .ok_or_else(|| WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: workspace
                    .checkout_generation()
                    .unwrap_or("unknown")
                    .to_owned(),
                reason: "generation manifest is missing".to_owned(),
            })?;
        if workspace
            .checkout_generation()
            .is_some_and(|generation| generation != manifest.generation)
        {
            return Err(WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: manifest.generation,
                reason: "workspace handle generation does not match manifest".to_owned(),
            });
        }
        let repository = self
            .checkout_repositories
            .get(manifest.repository_binding.repository_id().as_str())
            .ok_or_else(|| WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: manifest.generation.clone(),
                reason: "repository policy is unavailable".to_owned(),
            })?;
        let facts = checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            workspace.workspace_path(),
            "verify retained checkout",
            self.verify_git_checkout(
                workspace.workspace_path(),
                &manifest.repository_binding,
                repository,
                !allow_worker_changes,
                !allow_worker_changes,
            ),
        )
        .await?;
        if manifest.schema_version != 1
            || manifest.remote_fingerprint
                != manifest
                    .repository_binding
                    .repository
                    .safe_remote_fingerprint
                    .as_str()
            || manifest.target_branch != repository.target_branch
            || (!allow_worker_changes && manifest.target_branch != manifest.current_branch)
            || manifest.target_commit != manifest.head
        {
            return Err(WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: manifest.generation,
                reason: "checkout manifest provenance is inconsistent".to_owned(),
            });
        }
        if (!allow_worker_changes && facts.head != manifest.head)
            || (!allow_worker_changes && facts.branch != manifest.current_branch)
            || facts.shallow != manifest.shallow
            || (!allow_worker_changes && (!facts.clean || manifest.clean != facts.clean))
        {
            return Err(WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: manifest.generation,
                reason: "Git state does not match the recorded generation".to_owned(),
            });
        }
        if allow_worker_changes {
            let ancestry = checkout_operation_with_timeout(
                checkout_time_remaining(checkout_deadline),
                workspace.workspace_path(),
                "verify retained checkout ancestry",
                self.git(
                    workspace.workspace_path(),
                    &[
                        "merge-base",
                        "--is-ancestor",
                        &manifest.target_commit,
                        &facts.head,
                    ],
                ),
            )
            .await;
            if ancestry.is_err() {
                return Err(WorkspaceError::CheckoutVerification {
                    path: workspace.workspace_path().to_path_buf(),
                    generation: manifest.generation,
                    reason: "retained checkout HEAD no longer descends from the verified target"
                        .to_owned(),
                });
            }
        }
        let instruction = checkout_operation_with_timeout(
            checkout_time_remaining(checkout_deadline),
            workspace.workspace_path(),
            "verify checkout instructions",
            self.load_instruction_provenance(
                workspace.workspace_path(),
                repository,
                &manifest.head,
            ),
        )
        .await?;
        if instruction != manifest.instruction {
            return Err(WorkspaceError::CheckoutVerification {
                path: workspace.workspace_path().to_path_buf(),
                generation: manifest.generation,
                reason: "instruction provenance does not match the recorded commit".to_owned(),
            });
        }
        Ok(manifest)
    }

    pub async fn read_checkout_instructions(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<String>, WorkspaceError> {
        self.read_checkout_instructions_with_worker_changes(workspace, false)
            .await
    }

    pub async fn verify_checkout_for_retry(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<CheckoutManifest, WorkspaceError> {
        self.verify_checkout_with_worker_changes(workspace, true)
            .await
    }

    pub async fn read_checkout_instructions_for_retry(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<String>, WorkspaceError> {
        self.read_checkout_instructions_with_worker_changes(workspace, true)
            .await
    }

    async fn read_checkout_instructions_with_worker_changes(
        &self,
        workspace: &WorkspaceHandle,
        allow_worker_changes: bool,
    ) -> Result<Option<String>, WorkspaceError> {
        let manifest = self
            .verify_checkout_with_worker_changes(workspace, allow_worker_changes)
            .await?;
        if manifest.instruction.path.as_os_str().is_empty() {
            return Ok(None);
        }
        let path =
            resolve_path_within_root(workspace.workspace_path(), &manifest.instruction.path)?;
        let path = self.validate_workspace_owned_path(workspace, &path).await?;
        let bytes = fs::read(&path)
            .await
            .map_err(|source| WorkspaceError::ReadManagedFile {
                path: path.clone(),
                source,
            })?;
        let content = if is_workflow_instruction_path(&manifest.instruction.path) {
            workflow_body(&bytes)
        } else {
            bytes
        };
        Ok(Some(String::from_utf8_lossy(&content).into_owned()))
    }

    async fn find_compatible_checkout(
        &self,
        issue: &IssueDescriptor,
        binding: &RepositoryBinding,
        checkout_deadline: Option<Instant>,
    ) -> Result<Option<EnsureWorkspaceResult>, WorkspaceError> {
        let mut entries = fs::read_dir(&self.config.root).await.map_err(|source| {
            WorkspaceError::ReadDirectory {
                path: self.config.root.clone(),
                source,
            }
        })?;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| WorkspaceError::ReadDirectory {
                    path: self.config.root.clone(),
                    source,
                })?
        {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            let file_type =
                entry
                    .file_type()
                    .await
                    .map_err(|source| WorkspaceError::ReadDirectory {
                        path: entry.path(),
                        source,
                    })?;
            if !file_type.is_dir() {
                continue;
            }
            let Some((handle, manifest)) =
                (match self.load_workspace_from_directory(&entry.path()).await {
                    Ok(workspace) => workspace,
                    Err(WorkspaceError::DecodeManifest { path, source }) => {
                        tracing::warn!(
                            path = %path.display(),
                            %source,
                            "skipping retained checkout with a malformed generation manifest"
                        );
                        continue;
                    }
                    Err(error) => return Err(error),
                })
            else {
                continue;
            };
            if manifest.issue_id != issue.issue_id {
                continue;
            }
            let expected_workspace_key = checkout_workspace_key(
                &issue.identifier,
                &issue.issue_id,
                binding.repository_id().as_str(),
            )?;
            if manifest.identifier != issue.identifier {
                self.quarantine_checkout(
                    &handle,
                    "checkout identifier changed for the same tracker issue".to_owned(),
                )
                .await?;
                continue;
            }
            if manifest.sanitized_workspace_key != expected_workspace_key
                || manifest.workspace_path != handle.workspace_path()
            {
                continue;
            }
            let checkout = match self
                .load_manifest::<CheckoutManifest>(&handle, &handle.checkout_manifest_path())
                .await
            {
                Ok(Some(checkout)) => checkout,
                Ok(None) => continue,
                Err(error) => {
                    self.quarantine_checkout(&handle, error.to_string()).await?;
                    continue;
                }
            };
            if manifest.identifier != issue.identifier
                || manifest.sanitized_workspace_key != expected_workspace_key
                || manifest.workspace_path != handle.workspace_path()
                || checkout.issue_id != issue.issue_id
                || checkout.identifier != issue.identifier
                || checkout.sanitized_workspace_key != expected_workspace_key
                || checkout.workspace_path != handle.workspace_path()
            {
                self.quarantine_checkout(
                    &handle,
                    "checkout ownership manifest does not match the requested issue".to_owned(),
                )
                .await?;
                continue;
            }
            let entry_name = entry.file_name();
            let expected_generation = entry_name
                .to_str()
                .and_then(|name| {
                    name.strip_prefix(&format!("{}--", manifest.sanitized_workspace_key))
                })
                .filter(|generation| !generation.is_empty());
            if expected_generation != Some(checkout.generation.as_str()) {
                self.quarantine_checkout(
                    &handle,
                    "published checkout path does not match generation manifest".to_owned(),
                )
                .await?;
                continue;
            }
            if checkout.repository_binding != *binding {
                self.quarantine_checkout(
                    &handle,
                    "repository binding or policy generation mismatch".to_owned(),
                )
                .await?;
                continue;
            }
            let allow_worker_changes = self.load_run_manifest(&handle).await?.is_some_and(|run| {
                run.pending_retry
                    || matches!(
                        run.status,
                        RunStatus::Running
                            | RunStatus::Succeeded
                            | RunStatus::Failed
                            | RunStatus::Cancelled
                    )
            });
            match self
                .verify_checkout_with_worker_changes_timeout(
                    &handle,
                    allow_worker_changes,
                    checkout_deadline,
                )
                .await
            {
                Ok(_) => {
                    if self.config.hooks.after_create.is_some()
                        && !matches!(
                            self.inspect_after_create_receipt_state(issue, &handle)
                                .await?,
                            ExistingReceiptState::Owned
                        )
                    {
                        self.quarantine_checkout(
                            &handle,
                            "after_create hook completion receipt is missing or invalid".to_owned(),
                        )
                        .await?;
                        continue;
                    }
                    let issue_manifest = self.upsert_issue_manifest(issue, &handle).await?;
                    return Ok(Some(EnsureWorkspaceResult {
                        handle,
                        issue_manifest,
                        created: false,
                        after_create: None,
                    }));
                }
                Err(error) => {
                    if is_proven_checkout_invalid(&error) {
                        self.quarantine_checkout(&handle, error.to_string()).await?;
                    } else {
                        return Err(error);
                    }
                }
            }
        }
        Ok(None)
    }

    async fn quarantine_checkout(
        &self,
        workspace: &WorkspaceHandle,
        reason: String,
    ) -> Result<(), WorkspaceError> {
        let quarantine_root = self.config.root.join(".opensymphony-quarantine");
        self.reject_symlinked_workspace_root(&quarantine_root)
            .await?;
        self.create_directory(&quarantine_root).await?;
        self.reject_symlinked_workspace_root(&quarantine_root)
            .await?;
        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let canonical_quarantine_root = self.canonicalize_path(&quarantine_root).await?;
        ensure_descendant(&canonical_root, &canonical_quarantine_root)?;
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let destination = quarantine_root.join(format!(
            "{}-{suffix}",
            workspace
                .workspace_path()
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("checkout")
        ));
        self.reject_symlinked_workspace_root(&destination).await?;
        match self
            .load_manifest::<CheckoutManifest>(workspace, &workspace.checkout_manifest_path())
            .await
        {
            Ok(Some(mut manifest)) => {
                manifest.quarantined = true;
                manifest.quarantine_reason = Some(redact_runtime_diagnostic(&reason));
                self.write_json_artifact(workspace, &workspace.checkout_manifest_path(), &manifest)
                    .await?;
            }
            Ok(None) | Err(WorkspaceError::DecodeManifest { .. }) => {}
            Err(error) => return Err(error),
        }
        fs::rename(workspace.workspace_path(), &destination)
            .await
            .map_err(|source| WorkspaceError::CheckoutQuarantined {
                path: workspace.workspace_path().to_path_buf(),
                reason: format!("{reason}; quarantine failed: {source}"),
            })
    }

    async fn remove_published_checkout(&self, workspace: &WorkspaceHandle) {
        let _ = fs::remove_dir_all(workspace.workspace_path()).await;
    }

    async fn run_git_clone(
        &self,
        repository: &CheckoutRepository,
        destination: &Path,
        staging_cleanup: &mut StagingCleanupGuard,
    ) -> Result<(), WorkspaceError> {
        let environment_credential = repository.credential_kind == "environment";
        let ssh_agent_credential = repository.credential_kind == "ssh-agent";
        if (!environment_credential && !ssh_agent_credential)
            || (environment_credential
                && repository
                    .credential_env
                    .as_deref()
                    .is_none_or(|variable| std::env::var_os(variable).is_none()))
        {
            return Err(WorkspaceError::CheckoutOperation {
                operation: "resolve repository credential provider".to_owned(),
                path: destination.to_path_buf(),
                detail: "repository credential provider is unavailable".to_owned(),
            });
        }
        let askpass_path = if let Some(variable) = repository.credential_env.as_deref()
            && std::env::var_os(variable).is_some()
        {
            let path = destination
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(format!(".opensymphony-askpass-{}", Uuid::new_v4().simple()));
            staging_cleanup.register(path.clone());
            fs::write(
                &path,
                b"#!/bin/sh\ncase \"$1\" in\n  *Username*) printf '%s\\n' x-access-token ;;\n  *) printf '%s\\n' \"$OPENSYMPHONY_CHECKOUT_CREDENTIAL\" ;;\nesac\n",
            )
            .await
            .map_err(|source| WorkspaceError::CheckoutOperation {
                operation: "prepare Git credential helper".to_owned(),
                path: path.clone(),
                detail: source.to_string(),
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                    .await
                    .map_err(|source| WorkspaceError::CheckoutOperation {
                        operation: "prepare Git credential helper".to_owned(),
                        path: path.clone(),
                        detail: source.to_string(),
                    })?;
            }
            Some(path)
        } else {
            None
        };
        let mut command = Command::new("git");
        command
            .arg("clone")
            .arg("--single-branch")
            .arg("--branch")
            .arg(&repository.target_branch)
            .arg(&repository.remote)
            .arg(destination);
        if let Some(variable) = repository.credential_env.as_deref()
            && let Ok(value) = std::env::var(variable)
        {
            command.env("OPENSYMPHONY_CHECKOUT_CREDENTIAL", value);
        }
        if let Some(path) = askpass_path.as_ref() {
            command
                .env("GIT_ASKPASS", path)
                .env("GIT_TERMINAL_PROMPT", "0");
        }
        command.kill_on_drop(true);
        let output = match command.output().await {
            Ok(output) => output,
            Err(source) => {
                if let Some(path) = askpass_path {
                    let _ = fs::remove_file(path).await;
                }
                return Err(WorkspaceError::CheckoutOperation {
                    operation: "clone repository".to_owned(),
                    path: destination.to_path_buf(),
                    detail: source.to_string(),
                });
            }
        };
        if let Some(path) = askpass_path {
            let _ = fs::remove_file(path).await;
        }
        if output.status.success() {
            return Ok(());
        }
        Err(WorkspaceError::CheckoutOperation {
            operation: "clone repository".to_owned(),
            path: destination.to_path_buf(),
            detail: redact_runtime_diagnostic(&String::from_utf8_lossy(&output.stderr)),
        })
    }

    async fn verify_git_checkout(
        &self,
        checkout: &Path,
        binding: &RepositoryBinding,
        repository: &CheckoutRepository,
        enforce_worktree_state: bool,
        require_remote_head: bool,
    ) -> Result<GitFacts, WorkspaceError> {
        let inside = self
            .git(checkout, &["rev-parse", "--is-inside-work-tree"])
            .await?;
        if inside != "true" {
            return Err(checkout_verification(checkout, "not a Git worktree"));
        }
        let canonical_checkout = self
            .canonicalize_path(checkout)
            .await
            .map_err(|error| checkout_verification(checkout, &error.to_string()))?;
        for git_path in ["--git-dir", "--git-common-dir"] {
            let git_path = self.git(checkout, &["rev-parse", git_path]).await?;
            let git_path = PathBuf::from(&git_path);
            let git_path = if git_path.is_absolute() {
                git_path
            } else {
                canonical_checkout.join(git_path)
            };
            let canonical_git_path = self
                .canonicalize_path(&git_path)
                .await
                .map_err(|error| checkout_verification(checkout, &error.to_string()))?;
            if ensure_descendant(&canonical_checkout, &canonical_git_path).is_err() {
                return Err(checkout_verification(
                    checkout,
                    "Git directory escapes the checkout generation",
                ));
            }
        }
        let expected = SafeRemoteFingerprint::from_remote(
            &repository.provider,
            repository.provider_id.as_deref(),
            &repository.remote,
        )
        .map_err(|error| checkout_verification(checkout, &error.to_string()))?;
        let expected_locator = SafeRemoteFingerprint::from_remote(
            &repository.provider,
            None,
            &repository.remote_locator,
        )
        .map_err(|error| checkout_verification(checkout, &error.to_string()))?;
        for (kind, args) in [
            ("fetch", vec!["remote", "get-url", "--all", "origin"]),
            (
                "push",
                vec!["remote", "get-url", "--all", "--push", "origin"],
            ),
        ] {
            let remotes = self
                .git(checkout, &args)
                .await
                .map_err(|_| checkout_verification(checkout, "origin remote is unavailable"))?;
            let mut found = false;
            for remote in remotes
                .lines()
                .map(str::trim)
                .filter(|remote| !remote.is_empty())
            {
                found = true;
                if remote_contains_credentials(remote) {
                    return Err(checkout_verification(
                        checkout,
                        "observed remote contains credentials",
                    ));
                }
                let actual = SafeRemoteFingerprint::from_remote(
                    &repository.provider,
                    repository.provider_id.as_deref(),
                    remote,
                )
                .map_err(|error| checkout_verification(checkout, &error.to_string()))?;
                let actual_locator =
                    SafeRemoteFingerprint::from_remote(&repository.provider, None, remote)
                        .map_err(|error| checkout_verification(checkout, &error.to_string()))?;
                if actual != expected
                    || actual != binding.repository.safe_remote_fingerprint
                    || actual_locator != expected_locator
                {
                    return Err(checkout_verification(
                        checkout,
                        &format!("{kind} remote fingerprint mismatch"),
                    ));
                }
            }
            if !found {
                return Err(checkout_verification(
                    checkout,
                    &format!("origin {kind} remote is unavailable"),
                ));
            }
        }
        let branch = self.git(checkout, &["branch", "--show-current"]).await?;
        if enforce_worktree_state && branch != repository.target_branch {
            return Err(checkout_verification(checkout, "target branch mismatch"));
        }
        let remote_ref = format!("refs/remotes/origin/{}", repository.target_branch);
        self.git(checkout, &["show-ref", "--verify", &remote_ref])
            .await
            .map_err(|_| {
                checkout_verification(checkout, "target branch is not present on origin")
            })?;
        let head = self.git(checkout, &["rev-parse", "HEAD"]).await?;
        if require_remote_head {
            let remote_head = self.git(checkout, &["rev-parse", &remote_ref]).await?;
            if head != remote_head {
                return Err(checkout_verification(
                    checkout,
                    "HEAD does not match the configured target branch tip",
                ));
            }
        }
        let shallow = self
            .git(checkout, &["rev-parse", "--is-shallow-repository"])
            .await?
            == "true";
        if shallow {
            return Err(checkout_verification(checkout, "history is shallow"));
        }
        let status = self
            .git(
                checkout,
                &["status", "--porcelain", "--untracked-files=all"],
            )
            .await?;
        if enforce_worktree_state && !status.is_empty() {
            return Err(checkout_verification(checkout, "worktree is dirty"));
        }
        self.git(checkout, &["fsck", "--no-dangling"])
            .await
            .map_err(|_| checkout_verification(checkout, "Git integrity check failed"))?;
        Ok(GitFacts {
            branch,
            head,
            shallow,
            clean: status.is_empty(),
        })
    }

    async fn git(&self, checkout: &Path, args: &[&str]) -> Result<String, WorkspaceError> {
        let mut command = Command::new("git");
        command.arg("-C").arg(checkout).args(args);
        for variable in &self.checkout_credential_envs {
            command.env_remove(variable);
        }
        let output = command
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|source| WorkspaceError::CheckoutOperation {
                operation: args.first().copied().unwrap_or("git").to_owned(),
                path: checkout.to_path_buf(),
                detail: source.to_string(),
            })?;
        if !output.status.success() {
            return Err(WorkspaceError::CheckoutOperation {
                operation: args.first().copied().unwrap_or("git").to_owned(),
                path: checkout.to_path_buf(),
                detail: redact_runtime_diagnostic(&String::from_utf8_lossy(&output.stderr)),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    async fn load_instruction_provenance(
        &self,
        checkout: &Path,
        repository: &CheckoutRepository,
        source_commit: &str,
    ) -> Result<InstructionProvenance, WorkspaceError> {
        let candidates = [
            (
                !repository.instructions_path.as_os_str().is_empty(),
                repository.instructions_path.clone(),
                "configured",
            ),
            (true, PathBuf::from("AGENTS.md"), "agents"),
            (true, PathBuf::from("WORKFLOW.md"), "workflow"),
        ];
        let mut selected = None;
        for (enabled, relative, source) in candidates {
            if !enabled {
                continue;
            }
            self.reject_symlinked_path_components(checkout, &relative)
                .await?;
            let path = resolve_path_within_root(checkout, &relative)?;
            let metadata = match fs::symlink_metadata(&path).await {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if source == "configured" {
                        return Err(WorkspaceError::MissingInstruction { path });
                    }
                    continue;
                }
                Err(source) => return Err(WorkspaceError::ReadManagedFile { path, source }),
            };
            if metadata.file_type().is_symlink() {
                return Err(WorkspaceError::InstructionPathEscape { path });
            }
            if source == "configured" && !metadata.is_file() {
                return Err(WorkspaceError::CheckoutVerification {
                    path,
                    generation: source_commit.to_owned(),
                    reason: "configured instruction path is not a regular file".to_owned(),
                });
            }
            if metadata.is_file() {
                selected = Some((relative, path, source));
                break;
            }
        }
        let agents = discover_agents(checkout).await?;
        let native_discovery_hashes = self.hash_discovered_instructions(checkout, &agents).await?;
        let Some((relative, path, source)) = selected else {
            return Ok(InstructionProvenance {
                path: PathBuf::new(),
                content_hash: hash_bytes(&[]),
                source_commit: source_commit.to_owned(),
                source: "none".to_owned(),
                native_discovery_paths: agents,
                native_discovery_hashes,
            });
        };
        let mut content =
            fs::read(&path)
                .await
                .map_err(|source| WorkspaceError::ReadManagedFile {
                    path: path.clone(),
                    source,
                })?;
        if is_workflow_instruction_path(&relative) {
            content = workflow_body(&content);
        }
        Ok(InstructionProvenance {
            path: relative,
            content_hash: hash_bytes(&content),
            source_commit: source_commit.to_owned(),
            source: source.to_owned(),
            native_discovery_paths: agents,
            native_discovery_hashes,
        })
    }

    async fn hash_discovered_instructions(
        &self,
        checkout: &Path,
        paths: &[PathBuf],
    ) -> Result<BTreeMap<PathBuf, String>, WorkspaceError> {
        let mut hashes = BTreeMap::new();
        for relative in paths {
            self.reject_symlinked_path_components(checkout, relative)
                .await?;
            let path = resolve_path_within_root(checkout, relative)?;
            let content =
                fs::read(&path)
                    .await
                    .map_err(|source| WorkspaceError::ReadManagedFile {
                        path: path.clone(),
                        source,
                    })?;
            hashes.insert(relative.clone(), hash_bytes(&content));
        }
        Ok(hashes)
    }

    async fn update_checkout_run_id(
        &self,
        workspace: &WorkspaceHandle,
        run_id: &str,
    ) -> Result<(), WorkspaceError> {
        let Some(mut manifest) = self
            .load_manifest::<CheckoutManifest>(workspace, &workspace.checkout_manifest_path())
            .await?
        else {
            return Ok(());
        };
        if manifest.run_id != run_id {
            manifest.run_id = run_id.to_owned();
            self.write_manifest_atomically(
                workspace,
                &workspace.checkout_manifest_path(),
                &manifest,
            )
            .await?;
        }
        Ok(())
    }

    async fn exclude_metadata_from_git(&self, checkout: &Path) -> Result<(), WorkspaceError> {
        let exclude = checkout.join(".git").join("info").join("exclude");
        let mut contents = fs::read_to_string(&exclude).await.map_err(|source| {
            WorkspaceError::ReadManagedFile {
                path: exclude.clone(),
                source,
            }
        })?;
        let required = [".opensymphony/", ".opensymphony.after_create.json"];
        let missing = required
            .iter()
            .filter(|entry| !contents.lines().any(|line| line.trim() == **entry))
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            if !contents.ends_with('\n') {
                contents.push('\n');
            }
            for entry in missing {
                contents.push_str(entry);
                contents.push('\n');
            }
            fs::write(&exclude, contents).await.map_err(|source| {
                WorkspaceError::WriteArtifact {
                    path: exclude,
                    source,
                }
            })?;
        }
        Ok(())
    }

    pub async fn start_run(
        &self,
        workspace: &WorkspaceHandle,
        run: &RunDescriptor,
    ) -> Result<RunManifest, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;

        let mut manifest = RunManifest::new(workspace, run);
        self.write_run_manifest(workspace, &manifest).await?;

        match self.execute_hook(HookKind::BeforeRun, workspace).await {
            Ok(Some(record)) => manifest.hooks.push(record),
            Ok(None) => {}
            Err(failure) => {
                manifest.status = RunStatus::PreparationFailed;
                manifest.status_detail = Some(failure.error.to_string());
                manifest.updated_at = Utc::now();
                manifest.hooks.push(failure.record);
                self.write_run_manifest(workspace, &manifest).await?;
                return Err(failure.error);
            }
        }

        manifest.status = RunStatus::Prepared;
        manifest.updated_at = Utc::now();
        self.write_run_manifest(workspace, &manifest).await?;
        Ok(manifest)
    }

    pub async fn finish_run(
        &self,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        status: RunStatus,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;

        run_manifest.status = status;
        run_manifest.updated_at = Utc::now();
        self.write_run_manifest(workspace, run_manifest).await?;

        match self.execute_hook(HookKind::AfterRun, workspace).await {
            Ok(Some(record)) => run_manifest.hooks.push(record),
            Ok(None) => {}
            Err(failure) => run_manifest.hooks.push(failure.record),
        }

        run_manifest.updated_at = Utc::now();
        self.write_run_manifest(workspace, run_manifest).await
    }

    pub fn cleanup_decision(&self, state: IssueLifecycleState) -> CleanupDecision {
        match (state, self.config.cleanup.remove_terminal_workspaces) {
            (IssueLifecycleState::Terminal, true) => CleanupDecision::Remove,
            _ => CleanupDecision::Retain,
        }
    }

    pub async fn cleanup(
        &self,
        workspace: &WorkspaceHandle,
        state: IssueLifecycleState,
    ) -> Result<CleanupOutcome, WorkspaceError> {
        self.cleanup_with_terminal_removal(
            workspace,
            state,
            self.config.cleanup.remove_terminal_workspaces,
        )
        .await
    }

    /// Remove a failed terminal workspace even when ordinary terminal
    /// workspaces are retained by configuration. Retry exhaustion has its own
    /// `retain_failed` policy at the scheduler layer.
    pub async fn cleanup_failed_terminal_workspace(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<CleanupOutcome, WorkspaceError> {
        self.cleanup_with_terminal_removal(workspace, IssueLifecycleState::Terminal, true)
            .await
    }

    async fn cleanup_with_terminal_removal(
        &self,
        workspace: &WorkspaceHandle,
        state: IssueLifecycleState,
        remove_terminal_workspaces: bool,
    ) -> Result<CleanupOutcome, WorkspaceError> {
        if !path_exists(workspace.workspace_path()).await? {
            return Ok(CleanupOutcome {
                decision: if state == IssueLifecycleState::Terminal && remove_terminal_workspaces {
                    CleanupDecision::Remove
                } else {
                    CleanupDecision::Retain
                },
                before_remove: None,
            });
        }

        self.validate_workspace_handle(workspace).await?;
        if state != IssueLifecycleState::Terminal {
            return Ok(CleanupOutcome {
                decision: CleanupDecision::Retain,
                before_remove: None,
            });
        }

        let before_remove = match self.execute_hook(HookKind::BeforeRemove, workspace).await {
            Ok(record) => record,
            Err(failure) => Some(failure.record),
        };
        let decision = if remove_terminal_workspaces {
            CleanupDecision::Remove
        } else {
            CleanupDecision::Retain
        };

        if decision == CleanupDecision::Remove {
            match fs::remove_dir_all(workspace.workspace_path()).await {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(WorkspaceError::RemoveWorkspace {
                        path: workspace.workspace_path().to_path_buf(),
                        source: error,
                    });
                }
            }
        }

        Ok(CleanupOutcome {
            decision,
            before_remove,
        })
    }

    pub async fn load_issue_manifest(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<IssueManifest>, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.load_manifest(workspace, &workspace.issue_manifest_path())
            .await
    }

    pub async fn find_workspace_by_issue_reference(
        &self,
        issue_reference: &str,
    ) -> Result<Option<WorkspaceHandle>, WorkspaceError> {
        self.create_directory(&self.config.root).await?;

        match super::workspace_path_for_root(&self.config.root, issue_reference) {
            Ok(candidate) => match self.load_workspace_from_directory(&candidate).await {
                Ok(Some((handle, manifest)))
                    if workspace_matches_issue_reference(&manifest, issue_reference) =>
                {
                    return Ok(Some(handle));
                }
                Ok(_) => {}
                Err(WorkspaceError::DecodeManifest { .. }) => {}
                Err(error) => return Err(error),
            },
            Err(WorkspaceError::EmptyIdentifier | WorkspaceError::InvalidWorkspaceKey { .. }) => {}
            Err(error) => return Err(error),
        }

        let mut entries = fs::read_dir(&self.config.root).await.map_err(|source| {
            WorkspaceError::ReadDirectory {
                path: self.config.root.clone(),
                source,
            }
        })?;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| WorkspaceError::ReadDirectory {
                    path: self.config.root.clone(),
                    source,
                })?
        {
            let file_type =
                entry
                    .file_type()
                    .await
                    .map_err(|source| WorkspaceError::ReadDirectory {
                        path: entry.path(),
                        source,
                    })?;
            if !file_type.is_dir() {
                continue;
            }

            match self.load_workspace_from_directory(&entry.path()).await {
                Ok(Some((handle, manifest)))
                    if workspace_matches_issue_reference(&manifest, issue_reference) =>
                {
                    return Ok(Some(handle));
                }
                Ok(_) => {}
                Err(WorkspaceError::DecodeManifest { .. }) => {}
                Err(error) => return Err(error),
            }
        }

        Ok(None)
    }

    pub async fn find_verified_workspace_by_issue_reference(
        &self,
        issue_reference: &str,
    ) -> Result<Option<WorkspaceHandle>, WorkspaceError> {
        let mut verification_error = None;
        for (handle, manifest) in self.list_all_workspaces().await? {
            if !workspace_matches_issue_reference(&manifest, issue_reference)
                || handle.checkout_generation().is_none()
            {
                continue;
            }
            match self.verify_checkout_for_retry(&handle).await {
                Ok(checkout) if issue_manifest_owns_checkout(&handle, &manifest, &checkout) => {
                    return Ok(Some(handle));
                }
                Ok(_) => {
                    verification_error.get_or_insert(checkout_verification(
                        handle.workspace_path(),
                        "checkout ownership manifest does not match the issue manifest",
                    ));
                }
                Err(error) => {
                    verification_error.get_or_insert(error);
                }
            }
        }
        verification_error.map_or(Ok(None), Err)
    }

    /// List all valid workspaces in the workspace root.
    pub async fn list_all_workspaces(
        &self,
    ) -> Result<Vec<(WorkspaceHandle, IssueManifest)>, WorkspaceError> {
        self.create_directory(&self.config.root).await?;

        let mut workspaces = Vec::new();
        let mut entries = fs::read_dir(&self.config.root).await.map_err(|source| {
            WorkspaceError::ReadDirectory {
                path: self.config.root.clone(),
                source,
            }
        })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| WorkspaceError::ReadDirectory {
                    path: self.config.root.clone(),
                    source,
                })?
        {
            let file_type =
                entry
                    .file_type()
                    .await
                    .map_err(|source| WorkspaceError::ReadDirectory {
                        path: entry.path(),
                        source,
                    })?;
            if !file_type.is_dir() {
                continue;
            }

            match self.load_workspace_from_directory(&entry.path()).await {
                Ok(Some((handle, manifest))) if handle.checkout_generation().is_some() => {
                    match self
                        .load_manifest::<CheckoutManifest>(
                            &handle,
                            &handle.checkout_manifest_path(),
                        )
                        .await?
                    {
                        Some(checkout)
                            if issue_manifest_owns_checkout(&handle, &manifest, &checkout) =>
                        {
                            workspaces.push((handle, manifest));
                        }
                        Some(_) => tracing::warn!(
                            path = %handle.workspace_path().display(),
                            "skipping workspace with mismatched issue and checkout ownership"
                        ),
                        None => {}
                    }
                }
                Ok(Some((handle, manifest))) => workspaces.push((handle, manifest)),
                Ok(None) => {}
                Err(WorkspaceError::DecodeManifest { path, source }) => {
                    tracing::warn!(
                        path = %path.display(),
                        %source,
                        "skipping workspace with malformed checkout manifest during recovery"
                    );
                }
                Err(error) => return Err(error),
            }
        }

        Ok(workspaces)
    }

    pub async fn write_issue_manifest(
        &self,
        workspace: &WorkspaceHandle,
        manifest: &IssueManifest,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_manifest(workspace, &workspace.issue_manifest_path(), manifest)
            .await
    }

    pub async fn load_run_manifest(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<RunManifest>, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.load_manifest(workspace, &workspace.run_manifest_path())
            .await
    }

    pub async fn write_run_manifest(
        &self,
        workspace: &WorkspaceHandle,
        manifest: &RunManifest,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        let mut sanitized = manifest.clone();
        sanitized.status_detail = sanitized
            .status_detail
            .as_deref()
            .map(redact_runtime_diagnostic);
        sanitized.retry_error = sanitized
            .retry_error
            .as_deref()
            .map(redact_runtime_diagnostic);
        for hook in &mut sanitized.hooks {
            hook.command = redact_runtime_diagnostic(&hook.command);
            hook.stdout = redact_runtime_diagnostic(&hook.stdout);
            hook.stderr = redact_runtime_diagnostic(&hook.stderr);
        }
        self.write_manifest(workspace, &workspace.run_manifest_path(), &sanitized)
            .await
    }

    pub async fn read_text_artifact(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
    ) -> Result<Option<String>, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        let path = self.validate_workspace_owned_path(workspace, path).await?;
        match fs::read_to_string(&path).await {
            Ok(raw) => Ok(Some(raw)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(WorkspaceError::ReadManagedFile {
                path,
                source: error,
            }),
        }
    }

    pub async fn write_text_artifact(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
        contents: &str,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_bytes_artifact(workspace, path, contents.as_bytes())
            .await
    }

    pub async fn write_json_artifact<T>(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
        artifact: &T,
    ) -> Result<(), WorkspaceError>
    where
        T: Serialize,
    {
        self.validate_workspace_handle(workspace).await?;
        let path = normalize_absolute_path(path)?;
        let payload = serde_json::to_vec_pretty(artifact).map_err(|error| {
            WorkspaceError::EncodeJsonArtifact {
                path: path.clone(),
                source: error,
            }
        })?;
        self.write_bytes_artifact(workspace, &path, &payload).await
    }

    pub async fn load_conversation_manifest(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<ConversationManifest>, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.load_manifest(workspace, &workspace.conversation_manifest_path())
            .await
    }

    pub async fn write_conversation_manifest(
        &self,
        workspace: &WorkspaceHandle,
        manifest: &ConversationManifest,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_manifest(workspace, &workspace.conversation_manifest_path(), manifest)
            .await
    }

    pub async fn write_prompt_capture(
        &self,
        workspace: &WorkspaceHandle,
        run: &RunDescriptor,
        descriptor: PromptCaptureDescriptor,
        prompt: &str,
    ) -> Result<PromptCaptureManifest, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;

        let manifest = PromptCaptureManifest::new(workspace, run, descriptor, prompt);
        let archived_manifest_path =
            workspace.run_prompt_manifest_path(run.attempt, descriptor.kind, descriptor.sequence);
        let stable_manifest_path = workspace.latest_prompt_manifest_path(descriptor.kind);

        self.write_text_artifact(workspace, &manifest.archived_prompt_path, prompt)
            .await?;
        self.write_text_artifact(workspace, &manifest.stable_prompt_path, prompt)
            .await?;
        self.write_manifest(workspace, &archived_manifest_path, &manifest)
            .await?;
        self.write_manifest(workspace, &stable_manifest_path, &manifest)
            .await?;

        Ok(manifest)
    }

    pub async fn write_issue_context(
        &self,
        workspace: &WorkspaceHandle,
        artifact: &IssueContextArtifact,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_text_artifact(
            workspace,
            &workspace.issue_context_path(),
            &artifact.render_markdown(workspace),
        )
        .await
    }

    pub async fn load_session_context(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<SessionContextArtifact>, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.load_manifest(workspace, &workspace.session_context_path())
            .await
    }

    pub async fn write_session_context(
        &self,
        workspace: &WorkspaceHandle,
        artifact: &SessionContextArtifact,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_manifest(workspace, &workspace.session_context_path(), artifact)
            .await
    }

    async fn upsert_issue_manifest(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<IssueManifest, WorkspaceError> {
        self.upsert_issue_manifest_at_path(issue, workspace, workspace.workspace_path())
            .await
    }

    async fn upsert_issue_manifest_at_path(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
        manifest_workspace_path: &Path,
    ) -> Result<IssueManifest, WorkspaceError> {
        let existing = match self.inspect_issue_manifest_state(issue, workspace).await? {
            ExistingIssueManifestState::Owned(manifest) => Some(manifest),
            ExistingIssueManifestState::Conflict(manifest) => {
                return Err(WorkspaceError::WorkspaceOwnershipConflict {
                    details: Box::new(WorkspaceOwnershipConflictDetails {
                        workspace: workspace.workspace_path().to_path_buf(),
                        workspace_key: workspace.workspace_key().to_string(),
                        existing_issue_id: manifest.issue_id,
                        existing_identifier: manifest.identifier,
                        requested_issue_id: issue.issue_id.clone(),
                        requested_identifier: issue.identifier.clone(),
                    }),
                });
            }
            ExistingIssueManifestState::Missing | ExistingIssueManifestState::ForeignArtifact => {
                None
            }
        };
        let now = Utc::now();
        let manifest = IssueManifest {
            issue_id: issue.issue_id.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
            current_state: issue.current_state.clone(),
            sanitized_workspace_key: workspace.workspace_key().to_string(),
            workspace_path: manifest_workspace_path.to_path_buf(),
            created_at: existing
                .as_ref()
                .map(|manifest| manifest.created_at)
                .unwrap_or(now),
            updated_at: now,
            last_seen_tracker_refresh_at: issue.last_seen_tracker_refresh_at,
            repository_binding: issue.repository_binding.clone(),
        };

        self.write_manifest_atomically(workspace, &workspace.issue_manifest_path(), &manifest)
            .await?;
        Ok(manifest)
    }

    async fn write_after_create_receipt(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<(), WorkspaceError> {
        self.write_after_create_receipt_at_path(issue, workspace, workspace.workspace_path())
            .await
    }

    async fn write_after_create_receipt_at_path(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
        manifest_workspace_path: &Path,
    ) -> Result<(), WorkspaceError> {
        let mut receipt = AfterCreateBootstrapReceipt::new(workspace, issue);
        receipt.workspace_path = manifest_workspace_path.to_path_buf();
        self.write_manifest(workspace, &workspace.after_create_receipt_path(), &receipt)
            .await
    }

    async fn bootstrap_workspace_layout(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<(), WorkspaceError> {
        for directory in [
            workspace.metadata_dir(),
            workspace.logs_dir(),
            workspace.generated_dir(),
            workspace.openhands_dir(),
            workspace.prompts_dir(),
            workspace.runs_dir(),
        ] {
            self.create_managed_directory(workspace, &directory).await?;
        }

        Ok(())
    }

    async fn inspect_issue_manifest_state(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<ExistingIssueManifestState, WorkspaceError> {
        let path = workspace.issue_manifest_path();
        let path = self.validate_workspace_owned_path(workspace, &path).await?;
        let raw = match fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ExistingIssueManifestState::Missing);
            }
            Err(error) => {
                return Err(WorkspaceError::ReadManifest {
                    path,
                    source: error,
                });
            }
        };

        match serde_json::from_str::<IssueManifest>(&raw) {
            Ok(manifest) => Ok(classify_issue_manifest_ownership(
                issue, workspace, manifest,
            )),
            Err(_) => Ok(ExistingIssueManifestState::ForeignArtifact),
        }
    }

    async fn inspect_after_create_receipt_state(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<ExistingReceiptState, WorkspaceError> {
        let path = workspace.after_create_receipt_path();
        let path = self.validate_workspace_owned_path(workspace, &path).await?;
        let raw = match fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ExistingReceiptState::Missing);
            }
            Err(error) => {
                return Err(WorkspaceError::ReadManifest {
                    path,
                    source: error,
                });
            }
        };

        match serde_json::from_str::<AfterCreateBootstrapReceipt>(&raw) {
            Ok(receipt) => Ok(classify_after_create_receipt_ownership(
                issue, workspace, receipt,
            )),
            Err(_) => Ok(ExistingReceiptState::ForeignArtifact),
        }
    }

    async fn inspect_workspace_state(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<ExistingWorkspaceState, WorkspaceError> {
        let issue_manifest_state = self.inspect_issue_manifest_state(issue, workspace).await?;
        let issue_manifest_is_foreign = matches!(
            issue_manifest_state,
            ExistingIssueManifestState::ForeignArtifact
        );
        match issue_manifest_state {
            ExistingIssueManifestState::Owned(_) => return Ok(ExistingWorkspaceState::Owned),
            ExistingIssueManifestState::Conflict(manifest) => {
                return Ok(ExistingWorkspaceState::Conflict(
                    ownership_claim_from_issue_manifest(manifest),
                ));
            }
            ExistingIssueManifestState::Missing | ExistingIssueManifestState::ForeignArtifact => {}
        }

        let receipt_state = self
            .inspect_after_create_receipt_state(issue, workspace)
            .await?;
        match receipt_state {
            ExistingReceiptState::Owned => Ok(ExistingWorkspaceState::AfterCreateCompleted),
            ExistingReceiptState::Conflict(receipt) => Ok(ExistingWorkspaceState::Conflict(
                ownership_claim_from_after_create_receipt(*receipt),
            )),
            ExistingReceiptState::ForeignArtifact => Ok(ExistingWorkspaceState::ForeignArtifact),
            ExistingReceiptState::Missing => {
                if issue_manifest_is_foreign {
                    Ok(ExistingWorkspaceState::ForeignArtifact)
                } else {
                    Ok(ExistingWorkspaceState::Missing)
                }
            }
        }
    }

    async fn load_workspace_from_directory(
        &self,
        workspace_path: &Path,
    ) -> Result<Option<(WorkspaceHandle, IssueManifest)>, WorkspaceError> {
        self.reject_symlinked_workspace_root(workspace_path).await?;
        if !path_exists(workspace_path).await? {
            return Ok(None);
        }

        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let canonical_workspace = self.canonicalize_path(workspace_path).await?;
        ensure_descendant(&canonical_root, &canonical_workspace)?;

        let issue_manifest_path = canonical_workspace.join(".opensymphony").join("issue.json");
        let raw = match fs::read_to_string(&issue_manifest_path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(WorkspaceError::ReadManifest {
                    path: issue_manifest_path,
                    source,
                });
            }
        };
        let manifest = match serde_json::from_str::<IssueManifest>(&raw) {
            Ok(manifest) => manifest,
            Err(source) => {
                let checkout_manifest_path = canonical_workspace
                    .join(".opensymphony")
                    .join("checkout.json");
                if path_exists(&checkout_manifest_path).await? {
                    return Err(WorkspaceError::DecodeManifest {
                        path: issue_manifest_path,
                        source,
                    });
                }
                return Ok(None);
            }
        };

        let handle = WorkspaceHandle::new(
            manifest.issue_id.clone(),
            manifest.identifier.clone(),
            manifest.sanitized_workspace_key.clone(),
            canonical_workspace,
        );
        if !issue_manifest_claims_workspace(&handle, &manifest) {
            return Ok(None);
        }

        let handle = match self
            .load_manifest::<CheckoutManifest>(&handle, &handle.checkout_manifest_path())
            .await
        {
            Ok(Some(checkout)) => handle.with_checkout_generation(checkout.generation),
            Ok(None)
                if is_generation_shaped_directory(
                    workspace_path,
                    &manifest.sanitized_workspace_key,
                ) =>
            {
                return Err(missing_checkout_manifest_error(
                    handle.checkout_manifest_path(),
                ));
            }
            Ok(None) => handle,
            Err(error @ WorkspaceError::DecodeManifest { .. })
                if workspace_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&format!("{}--", manifest.sanitized_workspace_key))
                    }) =>
            {
                return Err(error);
            }
            Err(WorkspaceError::DecodeManifest { .. }) => handle,
            Err(error) => return Err(error),
        };

        Ok(Some((handle, manifest)))
    }

    async fn execute_hook(
        &self,
        kind: HookKind,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<HookExecutionRecord>, Box<HookFailure>> {
        let Some(hook) = self.hook_definition(kind) else {
            return Ok(None);
        };
        let cwd = self.resolve_hook_cwd(workspace, kind, hook).await?;
        let mut command = build_shell_command(&hook.command);
        configure_hook_command(&mut command);
        command
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for variable in &self.checkout_credential_envs {
            command.env_remove(variable);
        }

        let started_at = Utc::now();
        let started = Instant::now();
        let output = run_hook_command(command, self.config.hooks.timeout)
            .await
            .map_err(|error| {
                Box::new(HookFailure {
                    error: WorkspaceError::LaunchHook {
                        hook: kind,
                        cwd: cwd.clone(),
                        source: error,
                    },
                    record: HookExecutionRecord {
                        kind,
                        command: hook.command.clone(),
                        cwd: cwd.clone(),
                        best_effort: !kind.is_required(),
                        status: HookExecutionStatus::Failed,
                        started_at,
                        finished_at: Utc::now(),
                        duration_ms: started.elapsed().as_millis() as u64,
                        exit_code: None,
                        stdout: String::new(),
                        stderr: String::new(),
                    },
                })
            })?;
        let finished_at = Utc::now();
        let duration_ms = started.elapsed().as_millis() as u64;

        match output {
            HookCommandOutput::Completed(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                let exit_code = output.status.code();

                if output.status.success() {
                    Ok(Some(HookExecutionRecord {
                        kind,
                        command: hook.command.clone(),
                        cwd,
                        best_effort: !kind.is_required(),
                        status: HookExecutionStatus::Succeeded,
                        started_at,
                        finished_at,
                        duration_ms,
                        exit_code,
                        stdout,
                        stderr,
                    }))
                } else {
                    let record = HookExecutionRecord {
                        kind,
                        command: hook.command.clone(),
                        cwd,
                        best_effort: !kind.is_required(),
                        status: HookExecutionStatus::Failed,
                        started_at,
                        finished_at,
                        duration_ms,
                        exit_code,
                        stdout: stdout.clone(),
                        stderr: stderr.clone(),
                    };
                    Err(Box::new(HookFailure {
                        error: WorkspaceError::HookFailed {
                            hook: kind,
                            command: hook.command.clone(),
                            exit_code,
                            stdout,
                            stderr,
                        },
                        record,
                    }))
                }
            }
            HookCommandOutput::TimedOut { stdout, stderr } => Err(Box::new(HookFailure {
                error: WorkspaceError::HookTimedOut {
                    hook: kind,
                    command: hook.command.clone(),
                    timeout: self.config.hooks.timeout,
                },
                record: HookExecutionRecord {
                    kind,
                    command: hook.command.clone(),
                    cwd,
                    best_effort: !kind.is_required(),
                    status: HookExecutionStatus::TimedOut,
                    started_at,
                    finished_at,
                    duration_ms,
                    exit_code: None,
                    stdout,
                    stderr,
                },
            })),
        }
    }

    fn hook_definition(&self, kind: HookKind) -> Option<&HookDefinition> {
        match kind {
            HookKind::AfterCreate => self.config.hooks.after_create.as_ref(),
            HookKind::BeforeRun => self.config.hooks.before_run.as_ref(),
            HookKind::AfterRun => self.config.hooks.after_run.as_ref(),
            HookKind::BeforeRemove => self.config.hooks.before_remove.as_ref(),
        }
    }

    async fn resolve_hook_cwd(
        &self,
        workspace: &WorkspaceHandle,
        kind: HookKind,
        hook: &HookDefinition,
    ) -> Result<PathBuf, Box<HookFailure>> {
        let workspace_path = workspace.workspace_path().to_path_buf();
        let cwd = match hook.cwd.as_ref() {
            Some(cwd) => {
                let lexical_cwd =
                    resolve_path_within_root(&workspace_path, cwd).map_err(|error| {
                        let escaped = match &error {
                            WorkspaceError::PathEscape { path, .. } => path.clone(),
                            _ => cwd.clone(),
                        };

                        Box::new(HookFailure {
                            error: WorkspaceError::HookPathEscape {
                                hook: kind,
                                workspace: workspace_path.clone(),
                                cwd: escaped.clone(),
                            },
                            record: HookExecutionRecord {
                                kind,
                                command: hook.command.clone(),
                                cwd: escaped,
                                best_effort: !kind.is_required(),
                                status: HookExecutionStatus::Failed,
                                started_at: Utc::now(),
                                finished_at: Utc::now(),
                                duration_ms: 0,
                                exit_code: None,
                                stdout: String::new(),
                                stderr: String::new(),
                            },
                        })
                    })?;

                let canonical_cwd = fs::canonicalize(&lexical_cwd).await.map_err(|error| {
                    Box::new(HookFailure {
                        error: WorkspaceError::LaunchHook {
                            hook: kind,
                            cwd: lexical_cwd.clone(),
                            source: error,
                        },
                        record: HookExecutionRecord {
                            kind,
                            command: hook.command.clone(),
                            cwd: lexical_cwd.clone(),
                            best_effort: !kind.is_required(),
                            status: HookExecutionStatus::Failed,
                            started_at: Utc::now(),
                            finished_at: Utc::now(),
                            duration_ms: 0,
                            exit_code: None,
                            stdout: String::new(),
                            stderr: String::new(),
                        },
                    })
                })?;

                ensure_descendant(&workspace_path, &canonical_cwd).map_err(|_| {
                    Box::new(HookFailure {
                        error: WorkspaceError::HookPathEscape {
                            hook: kind,
                            workspace: workspace_path.clone(),
                            cwd: canonical_cwd.clone(),
                        },
                        record: HookExecutionRecord {
                            kind,
                            command: hook.command.clone(),
                            cwd: canonical_cwd.clone(),
                            best_effort: !kind.is_required(),
                            status: HookExecutionStatus::Failed,
                            started_at: Utc::now(),
                            finished_at: Utc::now(),
                            duration_ms: 0,
                            exit_code: None,
                            stdout: String::new(),
                            stderr: String::new(),
                        },
                    })
                })?;

                canonical_cwd
            }
            None => workspace_path,
        };

        Ok(cwd)
    }

    async fn validate_workspace_handle(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<(), WorkspaceError> {
        self.reject_symlinked_workspace_root(workspace.workspace_path())
            .await?;
        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let canonical_workspace = self.canonicalize_path(workspace.workspace_path()).await?;
        ensure_descendant(&canonical_root, &canonical_workspace)
    }

    async fn create_directory(&self, path: &Path) -> Result<(), WorkspaceError> {
        fs::create_dir_all(path)
            .await
            .map_err(|error| WorkspaceError::CreateDirectory {
                path: path.to_path_buf(),
                source: error,
            })
    }

    async fn create_managed_directory(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
    ) -> Result<(), WorkspaceError> {
        let path = self.validate_workspace_owned_path(workspace, path).await?;
        self.create_directory(&path).await?;
        self.validate_workspace_owned_path(workspace, &path).await?;
        Ok(())
    }

    async fn canonicalize_path(&self, path: &Path) -> Result<PathBuf, WorkspaceError> {
        fs::canonicalize(path)
            .await
            .map_err(|error| WorkspaceError::Canonicalize {
                path: path.to_path_buf(),
                source: error,
            })
    }

    async fn reject_symlinked_workspace_root(&self, path: &Path) -> Result<(), WorkspaceError> {
        match fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err(WorkspaceError::WorkspacePathSymlink {
                    path: path.to_path_buf(),
                })
            }
            Ok(_) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(WorkspaceError::Canonicalize {
                path: path.to_path_buf(),
                source: error,
            }),
        }
    }

    async fn reject_symlinked_path_components(
        &self,
        root: &Path,
        relative: &Path,
    ) -> Result<(), WorkspaceError> {
        let mut current = root.to_path_buf();
        for component in relative.components() {
            let std::path::Component::Normal(component) = component else {
                continue;
            };
            current.push(component);
            match fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(WorkspaceError::InstructionPathEscape { path: current });
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(source) => {
                    return Err(WorkspaceError::ReadManagedFile {
                        path: current,
                        source,
                    });
                }
            }
        }
        Ok(())
    }

    async fn load_manifest<T>(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
    ) -> Result<Option<T>, WorkspaceError>
    where
        T: DeserializeOwned,
    {
        let path = self.validate_workspace_owned_path(workspace, path).await?;
        let raw = match fs::read_to_string(&path).await {
            Ok(raw) => raw,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(WorkspaceError::ReadManifest {
                    path: path.to_path_buf(),
                    source: error,
                });
            }
        };

        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|error| WorkspaceError::DecodeManifest {
                path: path.to_path_buf(),
                source: error,
            })
    }

    async fn write_manifest<T>(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
        manifest: &T,
    ) -> Result<(), WorkspaceError>
    where
        T: Serialize,
    {
        if let Some(parent) = path.parent() {
            self.create_managed_directory(workspace, parent).await?;
        }
        let path = self.validate_workspace_owned_path(workspace, path).await?;

        let payload = serde_json::to_vec_pretty(manifest).map_err(|error| {
            WorkspaceError::EncodeManifest {
                path: path.clone(),
                source: error,
            }
        })?;

        fs::write(&path, payload)
            .await
            .map_err(|error| WorkspaceError::WriteManifest {
                path,
                source: error,
            })
    }

    async fn write_manifest_atomically<T>(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
        manifest: &T,
    ) -> Result<(), WorkspaceError>
    where
        T: Serialize,
    {
        if let Some(parent) = path.parent() {
            self.create_managed_directory(workspace, parent).await?;
        }
        let path = self.validate_workspace_owned_path(workspace, path).await?;
        let payload = serde_json::to_vec_pretty(manifest).map_err(|error| {
            WorkspaceError::EncodeManifest {
                path: path.clone(),
                source: error,
            }
        })?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("manifest.json");
        let temporary_path = path.with_file_name(format!(".{file_name}.tmp-{}", Uuid::new_v4()));
        let temporary_path = self
            .validate_workspace_owned_path(workspace, &temporary_path)
            .await?;
        if let Err(source) = fs::write(&temporary_path, payload).await {
            let _ = fs::remove_file(&temporary_path).await;
            return Err(WorkspaceError::WriteManifest { path, source });
        }
        if let Err(source) = fs::rename(&temporary_path, &path).await {
            let _ = fs::remove_file(&temporary_path).await;
            return Err(WorkspaceError::WriteManifest { path, source });
        }
        Ok(())
    }

    async fn write_bytes_artifact(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
        payload: &[u8],
    ) -> Result<(), WorkspaceError> {
        if let Some(parent) = path.parent() {
            self.create_managed_directory(workspace, parent).await?;
        }
        let path = self.validate_workspace_owned_path(workspace, path).await?;

        fs::write(&path, payload)
            .await
            .map_err(|error| WorkspaceError::WriteArtifact {
                path,
                source: error,
            })
    }

    async fn validate_workspace_owned_path(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let normalized = normalize_absolute_path(path)?;
        ensure_descendant(workspace.workspace_path(), &normalized)?;

        let relative = normalized
            .strip_prefix(workspace.workspace_path())
            .expect("managed workspace paths should remain within the workspace");
        let mut current = workspace.workspace_path().to_path_buf();

        for component in relative.components() {
            current.push(component.as_os_str());

            match fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(WorkspaceError::ManagedPathSymlink {
                        path: current.clone(),
                    });
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Err(error) => {
                    return Err(WorkspaceError::Canonicalize {
                        path: current.clone(),
                        source: error,
                    });
                }
            }
        }

        Ok(normalized)
    }
}

pub fn compose_terminal_prompt(
    central_procedure: &str,
    task_facts: &str,
    checkout_facts: &str,
    repository_instructions: Option<&str>,
    capabilities: &str,
) -> String {
    let repository_section = repository_instructions
        .filter(|instructions| !instructions.trim().is_empty())
        .unwrap_or("No repository-specific instructions were selected.");
    format!(
        "## Central Execution Procedure\n\n{central_procedure}\n\n## Task Facts\n\n{task_facts}\n\n## Verified Checkout\n\n{checkout_facts}\n\n## Repository Instructions\n\n{repository_section}\n\n## Runtime Capabilities\n\n{capabilities}\n"
    )
}

fn is_workflow_instruction_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("WORKFLOW.md"))
}

async fn run_hook_command(
    mut command: Command,
    timeout_duration: Duration,
) -> io::Result<HookCommandOutput> {
    let mut child = command.spawn()?;
    let process_id = child.id();
    let stdout_task = tokio::spawn(read_child_pipe(child.stdout.take()));
    let stderr_task = tokio::spawn(read_child_pipe(child.stderr.take()));

    match timeout(timeout_duration, child.wait()).await {
        Ok(status) => {
            let status = status?;
            let stdout = join_child_pipe(stdout_task).await?;
            let stderr = join_child_pipe(stderr_task).await?;

            Ok(HookCommandOutput::Completed(std::process::Output {
                status,
                stdout,
                stderr,
            }))
        }
        Err(_) => {
            terminate_hook_process_tree(&mut child, process_id).await?;
            let _ = child.wait().await?;
            let stdout = join_child_pipe(stdout_task).await?;
            let stderr = join_child_pipe(stderr_task).await?;

            Ok(HookCommandOutput::TimedOut {
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            })
        }
    }
}

#[derive(Debug)]
struct GitFacts {
    branch: String,
    head: String,
    shallow: bool,
    clean: bool,
}

fn checkout_verification(path: &Path, reason: &str) -> WorkspaceError {
    WorkspaceError::CheckoutVerification {
        path: path.to_path_buf(),
        generation: "unknown".to_owned(),
        reason: reason.to_owned(),
    }
}

fn remote_contains_credentials(remote: &str) -> bool {
    if let Ok(url) = Url::parse(remote) {
        if url.password().is_some() {
            return true;
        }
        if !url.username().is_empty() && !url.scheme().eq_ignore_ascii_case("ssh") {
            return true;
        }
    }

    let Some((authority, _path)) = remote.split_once(':') else {
        return false;
    };
    if authority.contains('/') || authority.contains('\\') {
        return false;
    }
    let Some((username, host)) = authority.split_once('@') else {
        return false;
    };
    host.is_empty() || username.contains(':') || !username.eq_ignore_ascii_case("git")
}

fn is_proven_checkout_invalid(error: &WorkspaceError) -> bool {
    matches!(
        error,
        WorkspaceError::CheckoutVerification { .. }
            | WorkspaceError::InstructionPathEscape { .. }
            | WorkspaceError::MissingInstruction { .. }
            | WorkspaceError::PathEscape { .. }
    )
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn workflow_body(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let Some(rest) = text.strip_prefix("---") else {
        return bytes.to_vec();
    };
    let Some((_, body)) = rest.split_once("\n---") else {
        return bytes.to_vec();
    };
    body.trim_start_matches('\n').as_bytes().to_vec()
}

async fn discover_agents(root: &Path) -> Result<Vec<PathBuf>, WorkspaceError> {
    let mut pending = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    let mut instruction_candidates = 0;
    while let Some(directory) = pending.pop() {
        let mut entries =
            fs::read_dir(&directory)
                .await
                .map_err(|source| WorkspaceError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?;
        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|source| WorkspaceError::ReadDirectory {
                    path: directory.clone(),
                    source,
                })?
        {
            let path = entry.path();
            let file_type =
                entry
                    .file_type()
                    .await
                    .map_err(|source| WorkspaceError::ReadDirectory {
                        path: path.clone(),
                        source,
                    })?;
            if file_type.is_symlink() {
                if entry.file_name() == "AGENTS.md" {
                    return Err(WorkspaceError::InstructionPathEscape { path });
                }
                continue;
            }
            if file_type.is_dir() {
                if !matches!(entry.file_name().to_str(), Some(".git" | ".opensymphony")) {
                    pending.push(path);
                }
            } else if file_type.is_file()
                && entry.file_name() == "AGENTS.md"
                && let Ok(relative) = path.strip_prefix(root)
            {
                instruction_candidates += 1;
                if instruction_candidates > 10_000 {
                    return Err(checkout_verification(
                        root,
                        "instruction discovery exceeded the candidate limit",
                    ));
                }
                paths.push(relative.to_path_buf());
            }
        }
    }
    paths.sort();
    Ok(paths)
}

async fn read_child_pipe<R>(pipe: Option<R>) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let Some(mut pipe) = pipe else {
        return Ok(Vec::new());
    };
    let mut buffer = Vec::new();
    pipe.read_to_end(&mut buffer).await?;
    Ok(buffer)
}

async fn join_child_pipe(
    task: tokio::task::JoinHandle<io::Result<Vec<u8>>>,
) -> io::Result<Vec<u8>> {
    match task.await {
        Ok(result) => result,
        Err(error) => Err(io::Error::other(error)),
    }
}

#[cfg(unix)]
fn configure_hook_command(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_hook_command(_command: &mut Command) {}

#[cfg(unix)]
async fn terminate_hook_process_tree(
    _child: &mut tokio::process::Child,
    process_id: Option<u32>,
) -> io::Result<()> {
    let Some(process_id) = process_id else {
        return Ok(());
    };

    let process_id = i32::try_from(process_id).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("hook process id {process_id} does not fit in i32"),
        )
    })?;
    let process_group = Pid::from_raw(process_id).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("hook process id {process_id} is not a valid Unix pid"),
        )
    })?;

    match kill_process_group(process_group, Signal::KILL) {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(windows)]
async fn terminate_hook_process_tree(
    child: &mut tokio::process::Child,
    process_id: Option<u32>,
) -> io::Result<()> {
    let Some(process_id) = process_id else {
        return child.kill().await;
    };

    let status = Command::new("taskkill")
        .arg("/T")
        .arg("/F")
        .arg("/PID")
        .arg(process_id.to_string())
        .status()
        .await?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "taskkill /T /F /PID {process_id} exited with {status}"
        )))
    }
}

#[cfg(not(any(unix, windows)))]
async fn terminate_hook_process_tree(
    child: &mut tokio::process::Child,
    _process_id: Option<u32>,
) -> io::Result<()> {
    child.kill().await
}

fn classify_issue_manifest_ownership(
    issue: &IssueDescriptor,
    workspace: &WorkspaceHandle,
    manifest: IssueManifest,
) -> ExistingIssueManifestState {
    if !issue_manifest_claims_workspace(workspace, &manifest) {
        return ExistingIssueManifestState::ForeignArtifact;
    }

    if manifest.issue_id == issue.issue_id && manifest.identifier == issue.identifier {
        ExistingIssueManifestState::Owned(manifest)
    } else {
        ExistingIssueManifestState::Conflict(manifest)
    }
}

fn classify_after_create_receipt_ownership(
    issue: &IssueDescriptor,
    workspace: &WorkspaceHandle,
    receipt: AfterCreateBootstrapReceipt,
) -> ExistingReceiptState {
    if !after_create_receipt_claims_workspace(workspace, &receipt) {
        return ExistingReceiptState::ForeignArtifact;
    }

    if receipt.issue_id == issue.issue_id && receipt.identifier == issue.identifier {
        ExistingReceiptState::Owned
    } else {
        ExistingReceiptState::Conflict(Box::new(receipt))
    }
}

fn issue_manifest_claims_workspace(workspace: &WorkspaceHandle, manifest: &IssueManifest) -> bool {
    workspace_path_claim_matches(
        workspace,
        &manifest.sanitized_workspace_key,
        &manifest.workspace_path,
    )
}

fn after_create_receipt_claims_workspace(
    workspace: &WorkspaceHandle,
    receipt: &AfterCreateBootstrapReceipt,
) -> bool {
    workspace_path_claim_matches(
        workspace,
        &receipt.sanitized_workspace_key,
        &receipt.workspace_path,
    )
}

fn workspace_path_claim_matches(
    workspace: &WorkspaceHandle,
    claimed_workspace_key: &str,
    claimed_workspace_path: &Path,
) -> bool {
    if claimed_workspace_key != workspace.workspace_key() {
        return false;
    }

    match normalize_absolute_path(claimed_workspace_path) {
        Ok(path) => path == workspace.workspace_path(),
        Err(_) => false,
    }
}

fn ownership_claim_from_issue_manifest(manifest: IssueManifest) -> WorkspaceOwnershipClaim {
    WorkspaceOwnershipClaim {
        issue_id: manifest.issue_id,
        identifier: manifest.identifier,
    }
}

fn ownership_claim_from_after_create_receipt(
    receipt: AfterCreateBootstrapReceipt,
) -> WorkspaceOwnershipClaim {
    WorkspaceOwnershipClaim {
        issue_id: receipt.issue_id,
        identifier: receipt.identifier,
    }
}

fn workspace_matches_issue_reference(manifest: &IssueManifest, issue_reference: &str) -> bool {
    manifest.identifier == issue_reference || manifest.issue_id == issue_reference
}

fn issue_manifest_binding_matches_checkout(
    manifest: &IssueManifest,
    checkout: &CheckoutManifest,
) -> bool {
    manifest
        .repository_binding
        .as_ref()
        .and_then(crate::opensymphony_domain::RepositoryBindingOutcome::resolved_binding)
        .is_some_and(|binding| binding == &checkout.repository_binding)
}

fn issue_manifest_owns_checkout(
    handle: &WorkspaceHandle,
    manifest: &IssueManifest,
    checkout: &CheckoutManifest,
) -> bool {
    checkout.issue_id == manifest.issue_id
        && checkout.identifier == manifest.identifier
        && checkout.sanitized_workspace_key == manifest.sanitized_workspace_key
        && checkout.workspace_path == handle.workspace_path()
        && issue_manifest_binding_matches_checkout(manifest, checkout)
}

fn ensure_descendant(root: &Path, candidate: &Path) -> Result<(), WorkspaceError> {
    if candidate.starts_with(root) {
        Ok(())
    } else {
        Err(WorkspaceError::PathEscape {
            root: root.to_path_buf(),
            path: candidate.to_path_buf(),
        })
    }
}

async fn path_exists(path: &Path) -> Result<bool, WorkspaceError> {
    match fs::metadata(path).await {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(WorkspaceError::Canonicalize {
            path: path.to_path_buf(),
            source: error,
        }),
    }
}

fn is_generation_shaped_directory(path: &Path, workspace_key: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix(&format!("{workspace_key}--")))
        .is_some_and(|generation| !generation.is_empty())
}

fn missing_checkout_manifest_error(path: PathBuf) -> WorkspaceError {
    WorkspaceError::DecodeManifest {
        path,
        source: serde_json::from_str::<serde_json::Value>("")
            .expect_err("empty JSON must produce a decode error"),
    }
}

#[cfg(unix)]
fn build_shell_command(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.arg("-c").arg(command);
    process
}

#[cfg(windows)]
fn build_shell_command(command: &str) -> Command {
    let mut process = Command::new("cmd");
    process.arg("/C").arg(command);
    process
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::ffi::OsString;

    use super::{
        WorkspaceError, build_shell_command, discover_agents, remote_contains_credentials,
    };

    #[cfg(unix)]
    #[tokio::test]
    async fn discover_agents_rejects_symlinked_nested_agents_files() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("checkout root should exist");
        let nested = root.path().join("nested");
        tokio::fs::create_dir(&nested)
            .await
            .expect("nested directory should exist");
        let outside = root.path().join("outside.md");
        tokio::fs::write(&outside, "outside instructions")
            .await
            .expect("outside instructions should exist");
        symlink(&outside, nested.join("AGENTS.md")).expect("nested symlink should exist");

        let error = discover_agents(root.path())
            .await
            .expect_err("symlinked nested instructions must block discovery");
        assert!(matches!(
            error,
            WorkspaceError::InstructionPathEscape { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unix_hook_commands_use_non_login_shell() {
        let command = build_shell_command("echo hook");
        let std_command = command.as_std();
        let args: Vec<OsString> = std_command.get_args().map(|arg| arg.to_owned()).collect();

        assert_eq!(std_command.get_program(), "sh");
        assert_eq!(
            args,
            vec![OsString::from("-c"), OsString::from("echo hook")]
        );
    }

    #[test]
    fn remote_credential_detection_handles_urls_and_scp_locators() {
        assert!(remote_contains_credentials(
            "https://token@example.com/org/repo.git"
        ));
        assert!(remote_contains_credentials(
            "TOKEN@example.com:org/repo.git"
        ));
        assert!(!remote_contains_credentials(
            "ssh://deploy@example.com/org/repo.git"
        ));
        assert!(!remote_contains_credentials("git@example.com:org/repo.git"));
        assert!(!remote_contains_credentials(
            "https://example.com/org/repo.git"
        ));
    }
}
