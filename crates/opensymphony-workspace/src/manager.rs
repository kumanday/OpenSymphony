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
};

pub struct WorkspaceManager {
    config: WorkspaceManagerConfig,
}

struct HookFailure {
    error: WorkspaceError,
    record: HookExecutionRecord,
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
        let created = !path_exists(&workspace_path).await?;

        self.create_directory(&workspace_path).await?;
        let canonical_workspace = self.canonicalize_path(&workspace_path).await?;
        ensure_descendant(&canonical_root, &canonical_workspace)?;

        let handle = WorkspaceHandle::new(
            issue.issue_id.clone(),
            issue.identifier.clone(),
            workspace_key,
            canonical_workspace,
        );
        self.bootstrap_workspace_layout(&handle).await?;
        let issue_manifest = self.upsert_issue_manifest(issue, &handle).await?;
        let after_create = if created {
            match self.execute_hook(HookKind::AfterCreate, &handle).await {
                Ok(record) => record,
                Err(failure) => return Err(failure.error),
            }
        } else {
            None
        };

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
        self.load_manifest(&workspace.issue_manifest_path()).await
    }

    pub async fn write_issue_manifest(
        &self,
        workspace: &WorkspaceHandle,
        manifest: &IssueManifest,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_manifest(&workspace.issue_manifest_path(), manifest)
            .await
    }

    pub async fn load_run_manifest(
        &self,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<RunManifest>, WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.load_manifest(&workspace.run_manifest_path()).await
    }

    pub async fn write_run_manifest(
        &self,
        workspace: &WorkspaceHandle,
        manifest: &RunManifest,
    ) -> Result<(), WorkspaceError> {
        self.validate_workspace_handle(workspace).await?;
        self.write_manifest(&workspace.run_manifest_path(), manifest)
            .await
    }

    async fn upsert_issue_manifest(
        &self,
        issue: &IssueDescriptor,
        workspace: &WorkspaceHandle,
    ) -> Result<IssueManifest, WorkspaceError> {
        let existing = self
            .load_manifest::<IssueManifest>(&workspace.issue_manifest_path())
            .await?;
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

        self.write_manifest(&workspace.issue_manifest_path(), &manifest)
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
            self.create_directory(&directory).await?;
        }

        Ok(())
    }

    async fn execute_hook(
        &self,
        kind: HookKind,
        workspace: &WorkspaceHandle,
    ) -> Result<Option<HookExecutionRecord>, Box<HookFailure>> {
        let Some(hook) = self.hook_definition(kind) else {
            return Ok(None);
        };
        let cwd = self.resolve_hook_cwd(workspace, kind, hook)?;
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

    fn resolve_hook_cwd(
        &self,
        workspace: &WorkspaceHandle,
        kind: HookKind,
        hook: &HookDefinition,
    ) -> Result<std::path::PathBuf, Box<HookFailure>> {
        let workspace_path = workspace.workspace_path().to_path_buf();
        let cwd = match hook.cwd.as_ref() {
            Some(cwd) => resolve_path_within_root(&workspace_path, cwd).map_err(|error| {
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
            })?,
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

    async fn create_directory(&self, path: &Path) -> Result<(), WorkspaceError> {
        fs::create_dir_all(path)
            .await
            .map_err(|error| WorkspaceError::CreateDirectory {
                path: path.to_path_buf(),
                source: error,
            })
    }

    async fn canonicalize_path(&self, path: &Path) -> Result<std::path::PathBuf, WorkspaceError> {
        fs::canonicalize(path)
            .await
            .map_err(|error| WorkspaceError::Canonicalize {
                path: path.to_path_buf(),
                source: error,
            })
    }

    async fn load_manifest<T>(&self, path: &Path) -> Result<Option<T>, WorkspaceError>
    where
        T: DeserializeOwned,
    {
        let raw = match fs::read_to_string(path).await {
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

    async fn write_manifest<T>(&self, path: &Path, manifest: &T) -> Result<(), WorkspaceError>
    where
        T: Serialize,
    {
        if let Some(parent) = path.parent() {
            self.create_directory(parent).await?;
        }

        let payload = serde_json::to_vec_pretty(manifest).map_err(|error| {
            WorkspaceError::EncodeManifest {
                path: path.to_path_buf(),
                source: error,
            }
        })?;

        fs::write(path, payload)
            .await
            .map_err(|error| WorkspaceError::WriteManifest {
                path: path.to_path_buf(),
                source: error,
            })
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
    process.arg("-lc").arg(command);
    process
}

#[cfg(windows)]
fn build_shell_command(command: &str) -> Command {
    let mut process = Command::new("cmd");
    process.arg("/C").arg(command);
    process
}
