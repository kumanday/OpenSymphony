use std::{path::Path, process::Stdio};

use chrono::Utc;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    fs,
    process::Command,
    time::{timeout, Instant},
};

use crate::{
    paths::{normalize_absolute_path, resolve_path_within_root, sanitize_workspace_key},
    CleanupDecision, CleanupOutcome, EnsureWorkspaceResult, HookDefinition, HookExecutionRecord,
    HookExecutionStatus, HookKind, IssueDescriptor, IssueLifecycleState, IssueManifest,
    RunDescriptor, RunManifest, RunStatus, WorkspaceError, WorkspaceHandle, WorkspaceManagerConfig,
    WorkspaceOwnershipConflictDetails,
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

        self.create_directory(&workspace_path).await?;
        let canonical_workspace = self.canonicalize_path(&workspace_path).await?;
        ensure_descendant(&canonical_root, &canonical_workspace)?;

        let handle = WorkspaceHandle::new(
            issue.issue_id.clone(),
            issue.identifier.clone(),
            workspace_key,
            canonical_workspace,
        );
        let existing_manifest = self.inspect_issue_manifest_state(issue, &handle).await?;
        if let ExistingIssueManifestState::Conflict(manifest) = &existing_manifest {
            return Err(WorkspaceError::WorkspaceOwnershipConflict {
                details: Box::new(WorkspaceOwnershipConflictDetails {
                    workspace: handle.workspace_path().to_path_buf(),
                    workspace_key: handle.workspace_key().to_string(),
                    existing_issue_id: manifest.issue_id.clone(),
                    existing_identifier: manifest.identifier.clone(),
                    requested_issue_id: issue.issue_id.clone(),
                    requested_identifier: issue.identifier.clone(),
                }),
            });
        }

        let created = !matches!(existing_manifest, ExistingIssueManifestState::Owned(_));
        let after_create = if created {
            match self.execute_hook(HookKind::AfterCreate, &handle).await {
                Ok(record) => record,
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
        let path = self
            .validate_managed_metadata_path(workspace, &path)
            .await?;
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
            Err(error) if !self.bootstrap_layout_exists(workspace).await? => {
                Ok(ExistingIssueManifestState::ForeignArtifact)
            }
            Err(error) => Err(WorkspaceError::DecodeManifest {
                path,
                source: error,
            }),
        }
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
        command
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let started_at = Utc::now();
        let started = Instant::now();
        let output = timeout(self.config.hooks.timeout, command.output()).await;
        let finished_at = Utc::now();
        let duration_ms = started.elapsed().as_millis() as u64;

        match output {
            Ok(Ok(output)) => {
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
            Ok(Err(error)) => Err(Box::new(HookFailure {
                error: WorkspaceError::LaunchHook {
                    hook: kind,
                    cwd: cwd.clone(),
                    source: error,
                },
                record: HookExecutionRecord {
                    kind,
                    command: hook.command.clone(),
                    cwd,
                    best_effort: !kind.is_required(),
                    status: HookExecutionStatus::Failed,
                    started_at,
                    finished_at,
                    duration_ms,
                    exit_code: None,
                    stdout: String::new(),
                    stderr: String::new(),
                },
            })),
            Err(_) => Err(Box::new(HookFailure {
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
                    stdout: String::new(),
                    stderr: String::new(),
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
        let canonical_root = self.canonicalize_path(&self.config.root).await?;
        let canonical_workspace = self.canonicalize_path(workspace.workspace_path()).await?;
        ensure_descendant(&canonical_root, &canonical_workspace)
    }

    async fn bootstrap_layout_exists(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<bool, WorkspaceError> {
        for directory in [
            workspace.metadata_dir(),
            workspace.logs_dir(),
            workspace.generated_dir(),
            workspace.openhands_dir(),
            workspace.prompts_dir(),
            workspace.runs_dir(),
        ] {
            let path = self
                .validate_managed_metadata_path(workspace, &directory)
                .await?;
            match fs::metadata(&path).await {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => {
                    return Err(WorkspaceError::Canonicalize {
                        path,
                        source: error,
                    });
                }
            }
        }

        Ok(true)
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
        let path = self.validate_managed_metadata_path(workspace, path).await?;
        self.create_directory(&path).await?;
        self.validate_managed_metadata_path(workspace, &path)
            .await?;
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

    async fn load_manifest<T>(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
    ) -> Result<Option<T>, WorkspaceError>
    where
        T: DeserializeOwned,
    {
        let path = self.validate_managed_metadata_path(workspace, path).await?;
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
        let path = self.validate_managed_metadata_path(workspace, path).await?;

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

    async fn validate_managed_metadata_path(
        &self,
        workspace: &WorkspaceHandle,
        path: &Path,
    ) -> Result<std::path::PathBuf, WorkspaceError> {
        let normalized = normalize_absolute_path(path)?;
        ensure_descendant(workspace.workspace_path(), &normalized)?;

        let relative = normalized
            .strip_prefix(workspace.workspace_path())
            .expect("managed metadata paths should remain within the workspace");
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

fn issue_manifest_claims_workspace(workspace: &WorkspaceHandle, manifest: &IssueManifest) -> bool {
    if manifest.sanitized_workspace_key != workspace.workspace_key() {
        return false;
    }

    match normalize_absolute_path(&manifest.workspace_path) {
        Ok(path) => path == workspace.workspace_path(),
        Err(_) => false,
    }
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
