use std::{io, path::Path, process::Stdio};

use chrono::Utc;
use serde::{Serialize, de::DeserializeOwned};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::{Instant, timeout},
};

use crate::{
    CleanupDecision, CleanupOutcome, ConversationManifest, EnsureWorkspaceResult, HookDefinition,
    HookExecutionRecord, HookExecutionStatus, HookKind, IssueContextArtifact, IssueDescriptor,
    IssueLifecycleState, IssueManifest, PromptCaptureDescriptor, PromptCaptureManifest,
    RunDescriptor, RunManifest, RunStatus, SessionContextArtifact, WorkspaceError, WorkspaceHandle,
    WorkspaceManagerConfig, WorkspaceOwnershipConflictDetails,
    models::AfterCreateBootstrapReceipt,
    paths::{normalize_absolute_path, resolve_path_within_root, sanitize_workspace_key},
};

pub struct WorkspaceManager {
    config: WorkspaceManagerConfig,
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
    Conflict(AfterCreateBootstrapReceipt),
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

enum HookCommandOutput {
    Completed(std::process::Output),
    TimedOut { stdout: String, stderr: String },
}

impl WorkspaceManager {
    pub fn new(mut config: WorkspaceManagerConfig) -> Result<Self, WorkspaceError> {
        config.root = normalize_absolute_path(&config.root)?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &WorkspaceManagerConfig {
        &self.config
    }

    pub fn workspace_path_for(
        &self,
        issue_identifier: &str,
    ) -> Result<std::path::PathBuf, WorkspaceError> {
        crate::workspace_path_for_root(&self.config.root, issue_identifier)
    }

    pub async fn ensure(
        &self,
        issue: &IssueDescriptor,
    ) -> Result<EnsureWorkspaceResult, WorkspaceError> {
        self.create_directory(&self.config.root).await?;
        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let workspace_key = sanitize_workspace_key(&issue.identifier)?;
        let workspace_path = crate::workspace_path_for_root(&canonical_root, &issue.identifier)?;

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
        if !path_exists(workspace.workspace_path()).await? {
            return Ok(CleanupOutcome {
                decision: self.cleanup_decision(state),
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
        let decision = self.cleanup_decision(state);

        if decision == CleanupDecision::Remove {
            match fs::remove_dir_all(workspace.workspace_path()).await {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
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

        match crate::workspace_path_for_root(&self.config.root, issue_reference) {
            Ok(candidate) => {
                if let Some((handle, manifest)) =
                    self.load_workspace_from_directory(&candidate).await?
                    && workspace_matches_issue_reference(&manifest, issue_reference)
                {
                    return Ok(Some(handle));
                }
            }
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

            if let Some((handle, manifest)) =
                self.load_workspace_from_directory(&entry.path()).await?
                && workspace_matches_issue_reference(&manifest, issue_reference)
            {
                return Ok(Some(handle));
            }
        }

        Ok(None)
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

            if let Some((handle, manifest)) =
                self.load_workspace_from_directory(&entry.path()).await?
            {
                workspaces.push((handle, manifest));
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
        self.write_manifest(workspace, &workspace.run_manifest_path(), manifest)
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
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
            workspace_path: workspace.workspace_path().to_path_buf(),
            created_at: existing
                .as_ref()
                .map(|manifest| manifest.created_at)
                .unwrap_or(now),
            updated_at: now,
            last_seen_tracker_refresh_at: issue.last_seen_tracker_refresh_at,
        };

        self.write_manifest(workspace, &workspace.issue_manifest_path(), &manifest)
            .await?;
        Ok(manifest)
    }

    async fn write_after_create_receipt(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<(), WorkspaceError> {
        let receipt = AfterCreateBootstrapReceipt::new(workspace, issue);
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
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
                ownership_claim_from_after_create_receipt(receipt),
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(WorkspaceError::ReadManifest {
                    path: issue_manifest_path,
                    source,
                });
            }
        };
        let manifest = match serde_json::from_str::<IssueManifest>(&raw) {
            Ok(manifest) => manifest,
            Err(_) => return Ok(None),
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
    ) -> Result<std::path::PathBuf, Box<HookFailure>> {
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

    async fn canonicalize_path(&self, path: &Path) -> Result<std::path::PathBuf, WorkspaceError> {
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(WorkspaceError::Canonicalize {
                path: path.to_path_buf(),
                source: error,
            }),
        }
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
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
    ) -> Result<std::path::PathBuf, WorkspaceError> {
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
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
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

async fn run_hook_command(
    mut command: Command,
    timeout_duration: std::time::Duration,
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
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
}

#[cfg(not(unix))]
fn configure_hook_command(_command: &mut Command) {}

#[cfg(unix)]
async fn terminate_hook_process_tree(
    _child: &mut tokio::process::Child,
    process_id: Option<u32>,
) -> io::Result<()> {
    if let Some(process_id) = process_id {
        let result = unsafe { libc::kill(-(process_id as i32), libc::SIGKILL) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(error);
            }
        }
    }

    Ok(())
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
        ExistingReceiptState::Conflict(receipt)
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
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(WorkspaceError::Canonicalize {
            path: path.to_path_buf(),
            source: error,
        }),
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

    use super::build_shell_command;

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
}
