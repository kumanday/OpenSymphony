use chrono::{DateTime, Utc};
use opensymphony_domain::{Issue, RetryEntry, RetryReason, WorkerOutcome};
use opensymphony_openhands::PromptMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
    #[serde(default)]
    pub cleanup_terminal_workspaces: bool,
    #[serde(default)]
    pub hooks: HookConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    #[serde(default = "default_hook_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            after_create: None,
            before_run: None,
            after_run: None,
            before_remove: None,
            timeout_ms: default_hook_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceContext {
    pub workspace_path: PathBuf,
    pub metadata_dir: PathBuf,
    pub sanitized_workspace_key: String,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueManifest {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub current_state: String,
    pub sanitized_workspace_key: String,
    pub workspace_path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_seen_tracker_refresh_at: DateTime<Utc>,
    pub last_attempt: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationManifest {
    pub issue_id: String,
    pub identifier: String,
    pub conversation_id: String,
    pub server_base_url: String,
    pub persistence_dir: String,
    pub created_at: DateTime<Utc>,
    pub last_attached_at: DateTime<Utc>,
    pub fresh_conversation: bool,
    pub reset_reason: Option<String>,
    pub runtime_contract_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RetryManifest {
    pub issue_id: String,
    pub identifier: String,
    pub attempt: u32,
    pub reason: RetryReason,
    pub scheduled_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastRunManifest {
    pub issue_id: String,
    pub identifier: String,
    pub attempt: u32,
    pub outcome: WorkerOutcome,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContext {
    pub conversation_id: Option<String>,
    pub attempt_number: u32,
    pub last_worker_at: DateTime<Utc>,
    pub last_worker_outcome: String,
    pub last_retry_reason: Option<RetryReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookStage {
    AfterCreate,
    BeforeRun,
    AfterRun,
    BeforeRemove,
}

impl HookStage {
    fn label(self) -> &'static str {
        match self {
            Self::AfterCreate => "after_create",
            Self::BeforeRun => "before_run",
            Self::AfterRun => "after_run",
            Self::BeforeRemove => "before_remove",
        }
    }
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("workspace path escapes configured root: {0}")]
    PathEscape(String),
    #[error("workspace IO failed: {0}")]
    Io(String),
    #[error("workspace JSON failed: {0}")]
    Json(String),
    #[error("hook `{stage}` timed out after {timeout_ms}ms")]
    HookTimeout {
        stage: &'static str,
        timeout_ms: u64,
    },
    #[error("hook `{stage}` failed: {stderr}")]
    HookFailed { stage: &'static str, stderr: String },
}

#[derive(Debug, Clone)]
pub struct WorkspaceManager {
    config: WorkspaceConfig,
}

impl WorkspaceManager {
    pub fn new(config: WorkspaceConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &WorkspaceConfig {
        &self.config
    }

    pub fn sanitize_issue_identifier(identifier: &str) -> String {
        let sanitized = identifier
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();

        match sanitized.as_str() {
            "" => "issue".to_string(),
            "." => "_".to_string(),
            ".." => "__".to_string(),
            _ => sanitized,
        }
    }

    pub fn workspace_path(&self, identifier: &str) -> Result<PathBuf, WorkspaceError> {
        let sanitized = Self::sanitize_issue_identifier(identifier);
        let root = self.resolved_root()?;
        let candidate = root.join(&sanitized);
        if candidate.parent() != Some(root.as_path()) {
            return Err(WorkspaceError::PathEscape(candidate.display().to_string()));
        }
        Ok(candidate)
    }

    pub async fn ensure_workspace(
        &self,
        issue: &Issue,
    ) -> Result<WorkspaceContext, WorkspaceError> {
        fs::create_dir_all(&self.config.root)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;

        let workspace_path = self.workspace_path_for_issue(issue)?;
        let created = !workspace_path.exists();
        fs::create_dir_all(&workspace_path)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;

        let metadata_dir = workspace_path.join(".opensymphony");
        fs::create_dir_all(metadata_dir.join("generated"))
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        fs::create_dir_all(metadata_dir.join("logs"))
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        fs::create_dir_all(metadata_dir.join("openhands"))
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        fs::create_dir_all(metadata_dir.join("prompts"))
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        let needs_bootstrap = !self.workspace_bootstrap_complete(&metadata_dir);

        let context = WorkspaceContext {
            workspace_path,
            metadata_dir,
            sanitized_workspace_key: Self::sanitize_issue_identifier(&issue.id),
            created,
        };

        self.write_issue_manifest(issue, &context, 0)?;

        if needs_bootstrap {
            self.run_hook(HookStage::AfterCreate, &context.workspace_path, false)
                .await?;
            self.mark_workspace_bootstrapped(&context.metadata_dir)?;
        }

        Ok(context)
    }

    pub async fn prepare_issue_workspace(
        &self,
        issue: &Issue,
        attempt: u32,
    ) -> Result<WorkspaceContext, WorkspaceError> {
        let context = self.ensure_workspace(issue).await?;
        self.run_hook(HookStage::BeforeRun, &context.workspace_path, false)
            .await?;
        self.write_issue_manifest(issue, &context, attempt)?;
        Ok(context)
    }

    pub fn write_prompt(
        &self,
        context: &WorkspaceContext,
        mode: PromptMode,
        prompt: &str,
    ) -> Result<(), WorkspaceError> {
        let filename = match mode {
            PromptMode::Fresh => "last-full-prompt.md",
            PromptMode::Continuation => "last-continuation-prompt.md",
        };
        fs::write(context.metadata_dir.join("prompts").join(filename), prompt)
            .map_err(|error| WorkspaceError::Io(error.to_string()))
    }

    pub fn save_conversation_manifest(
        &self,
        issue: &Issue,
        context: &WorkspaceContext,
        conversation_id: &str,
        fresh_conversation: bool,
        reset_reason: Option<String>,
    ) -> Result<(), WorkspaceError> {
        let now = Utc::now();
        let manifest = ConversationManifest {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            conversation_id: conversation_id.to_string(),
            server_base_url: "local".to_string(),
            persistence_dir: context.metadata_dir.join("openhands").display().to_string(),
            created_at: now,
            last_attached_at: now,
            fresh_conversation,
            reset_reason,
            runtime_contract_version: "openhands-agent-server-v1.14.0".to_string(),
        };
        write_json(context.metadata_dir.join("conversation.json"), &manifest)
    }

    pub fn load_conversation_manifest(
        &self,
        issue_id: &str,
    ) -> Result<Option<ConversationManifest>, WorkspaceError> {
        let path = self
            .workspace_path_for_issue_id(issue_id)?
            .join(".opensymphony/conversation.json");
        read_optional_json(path)
    }

    pub fn clear_conversation_manifest(&self, issue_id: &str) -> Result<(), WorkspaceError> {
        let path = self
            .workspace_path_for_issue_id(issue_id)?
            .join(".opensymphony/conversation.json");
        if path.exists() {
            fs::remove_file(path).map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        Ok(())
    }

    pub fn persist_retry(&self, entry: &RetryEntry) -> Result<(), WorkspaceError> {
        let workspace_path = self.workspace_path_for_issue(&entry.issue)?;
        fs::create_dir_all(workspace_path.join(".opensymphony"))
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        let manifest = RetryManifest {
            issue_id: entry.issue.id.clone(),
            identifier: entry.issue.identifier.clone(),
            attempt: entry.attempt,
            reason: entry.reason.clone(),
            scheduled_at: entry.scheduled_at,
        };
        write_json(workspace_path.join(".opensymphony/retry.json"), &manifest)
    }

    pub fn load_retry_manifest(
        &self,
        issue_id: &str,
    ) -> Result<Option<RetryManifest>, WorkspaceError> {
        let path = self
            .workspace_path_for_issue_id(issue_id)?
            .join(".opensymphony/retry.json");
        read_optional_json(path)
    }

    pub fn clear_retry_manifest(&self, issue_id: &str) -> Result<(), WorkspaceError> {
        let path = self
            .workspace_path_for_issue_id(issue_id)?
            .join(".opensymphony/retry.json");
        if path.exists() {
            fs::remove_file(path).map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        Ok(())
    }

    pub async fn finish_attempt(
        &self,
        issue: &Issue,
        attempt: u32,
        outcome: &WorkerOutcome,
        conversation_id: Option<&str>,
        retry_reason: Option<RetryReason>,
    ) -> Result<(), WorkspaceError> {
        let context = self.ensure_workspace(issue).await?;
        let last_run = LastRunManifest {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            attempt,
            outcome: outcome.clone(),
            conversation_id: conversation_id.map(|value| value.to_string()),
        };
        write_json(context.metadata_dir.join("last-run.json"), &last_run)?;
        self.write_issue_manifest(issue, &context, attempt)?;
        self.write_generated_artifacts(
            &context,
            issue,
            attempt,
            outcome,
            conversation_id,
            retry_reason,
        )?;
        self.run_hook(HookStage::AfterRun, &context.workspace_path, true)
            .await?;
        Ok(())
    }

    pub async fn cleanup_terminal_workspace(&self, issue: &Issue) -> Result<(), WorkspaceError> {
        let workspace_path = self.workspace_path_for_issue(issue)?;
        if !workspace_path.exists() {
            return Ok(());
        }
        self.clear_retry_manifest(&issue.id)?;
        if self.config.cleanup_terminal_workspaces {
            self.run_hook(HookStage::BeforeRemove, &workspace_path, true)
                .await?;
            fs::remove_dir_all(&workspace_path)
                .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        Ok(())
    }

    pub fn list_issue_manifests(&self) -> Result<Vec<IssueManifest>, WorkspaceError> {
        let mut manifests: HashMap<String, IssueManifest> = HashMap::new();
        if !self.config.root.exists() {
            return Ok(vec![]);
        }

        for entry in fs::read_dir(&self.config.root)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| WorkspaceError::Io(error.to_string()))?;
            let path = entry.path().join(".opensymphony/issue.json");
            if let Some(manifest) = read_optional_json::<IssueManifest>(path)? {
                let primary_workspace_path = self.workspace_path(&manifest.issue_id)?;
                let replace = match manifests.get(&manifest.issue_id) {
                    None => true,
                    Some(current)
                        if manifest.last_seen_tracker_refresh_at
                            > current.last_seen_tracker_refresh_at =>
                    {
                        true
                    }
                    Some(current)
                        if manifest.last_seen_tracker_refresh_at
                            == current.last_seen_tracker_refresh_at
                            && manifest.workspace_path
                                == primary_workspace_path.display().to_string()
                            && current.workspace_path
                                != primary_workspace_path.display().to_string() =>
                    {
                        true
                    }
                    _ => false,
                };
                if replace {
                    manifests.insert(manifest.issue_id.clone(), manifest);
                }
            }
        }

        Ok(manifests.into_values().collect())
    }

    fn write_generated_artifacts(
        &self,
        context: &WorkspaceContext,
        issue: &Issue,
        attempt: u32,
        outcome: &WorkerOutcome,
        conversation_id: Option<&str>,
        retry_reason: Option<RetryReason>,
    ) -> Result<(), WorkspaceError> {
        let issue_context = format!(
            "# Issue Context\n\n- Identifier: {}\n- Title: {}\n- State: {}\n- Last outcome: {:?}\n- Metadata: {}\n",
            issue.identifier,
            issue.title,
            issue.state,
            outcome.kind,
            context.metadata_dir.display(),
        );
        fs::write(
            context.metadata_dir.join("generated/issue-context.md"),
            issue_context,
        )
        .map_err(|error| WorkspaceError::Io(error.to_string()))?;

        let session_context = SessionContext {
            conversation_id: conversation_id.map(|value| value.to_string()),
            attempt_number: attempt,
            last_worker_at: outcome.observed_at,
            last_worker_outcome: format!("{:?}", outcome.kind),
            last_retry_reason: retry_reason,
        };
        write_json(
            context.metadata_dir.join("generated/session-context.json"),
            &session_context,
        )
    }

    fn write_issue_manifest(
        &self,
        issue: &Issue,
        context: &WorkspaceContext,
        attempt: u32,
    ) -> Result<(), WorkspaceError> {
        let now = Utc::now();
        let manifest = IssueManifest {
            issue_id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
            current_state: issue.state.clone(),
            sanitized_workspace_key: context.sanitized_workspace_key.clone(),
            workspace_path: context.workspace_path.display().to_string(),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            last_seen_tracker_refresh_at: now,
            last_attempt: attempt,
        };
        write_json(context.metadata_dir.join("issue.json"), &manifest)
    }

    async fn run_hook(
        &self,
        stage: HookStage,
        workspace_path: &Path,
        best_effort: bool,
    ) -> Result<(), WorkspaceError> {
        let script = match stage {
            HookStage::AfterCreate => self.config.hooks.after_create.as_deref(),
            HookStage::BeforeRun => self.config.hooks.before_run.as_deref(),
            HookStage::AfterRun => self.config.hooks.after_run.as_deref(),
            HookStage::BeforeRemove => self.config.hooks.before_remove.as_deref(),
        };

        let Some(script) = script else {
            return Ok(());
        };

        let mut command = Command::new("/bin/sh");
        command.arg("-lc").arg(script);
        command.current_dir(workspace_path);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let timeout_ms = self.config.hooks.timeout_ms;
        let mut child = command
            .spawn()
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        let stdout_task = tokio::spawn(read_pipe_to_end(child.stdout.take()));
        let stderr_task = tokio::spawn(read_pipe_to_end(child.stderr.take()));

        let result = timeout(Duration::from_millis(timeout_ms), child.wait()).await;
        match result {
            Ok(Ok(status)) => {
                let stdout = stdout_task
                    .await
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
                let stderr = stderr_task
                    .await
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
                self.append_hook_log(workspace_path, stage, &stdout, &stderr)?;
                if status.success() {
                    Ok(())
                } else {
                    let error = WorkspaceError::HookFailed {
                        stage: stage.label(),
                        stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
                    };
                    if best_effort {
                        Ok(())
                    } else {
                        Err(error)
                    }
                }
            }
            Ok(Err(error)) => {
                let _ = stdout_task.await;
                let _ = stderr_task.await;
                if best_effort {
                    Ok(())
                } else {
                    Err(WorkspaceError::Io(error.to_string()))
                }
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let stdout = stdout_task
                    .await
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
                let stderr = stderr_task
                    .await
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
                self.append_hook_log(workspace_path, stage, &stdout, &stderr)?;
                if best_effort {
                    Ok(())
                } else {
                    Err(WorkspaceError::HookTimeout {
                        stage: stage.label(),
                        timeout_ms,
                    })
                }
            }
        }
    }

    fn append_hook_log(
        &self,
        workspace_path: &Path,
        stage: HookStage,
        stdout: &[u8],
        stderr: &[u8],
    ) -> Result<(), WorkspaceError> {
        let log_path = workspace_path.join(".opensymphony/logs/hook.log");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        writeln!(file, "[{}]", stage.label())
            .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        if !stdout.is_empty() {
            writeln!(file, "stdout: {}", String::from_utf8_lossy(stdout).trim())
                .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        if !stderr.is_empty() {
            writeln!(file, "stderr: {}", String::from_utf8_lossy(stderr).trim())
                .map_err(|error| WorkspaceError::Io(error.to_string()))?;
        }
        Ok(())
    }

    fn workspace_bootstrap_complete(&self, metadata_dir: &Path) -> bool {
        metadata_dir.join("bootstrap.ok").exists()
    }

    fn mark_workspace_bootstrapped(&self, metadata_dir: &Path) -> Result<(), WorkspaceError> {
        fs::write(metadata_dir.join("bootstrap.ok"), b"ok")
            .map_err(|error| WorkspaceError::Io(error.to_string()))
    }

    fn resolved_root(&self) -> Result<PathBuf, WorkspaceError> {
        if self.config.root.exists() {
            return self
                .config
                .root
                .canonicalize()
                .map_err(|error| WorkspaceError::Io(error.to_string()));
        }

        for ancestor in self.config.root.ancestors() {
            if ancestor.exists() {
                let canonical_ancestor = ancestor
                    .canonicalize()
                    .map_err(|error| WorkspaceError::Io(error.to_string()))?;
                let suffix = self
                    .config
                    .root
                    .strip_prefix(ancestor)
                    .map_err(|error| WorkspaceError::PathEscape(error.to_string()))?;
                return Ok(canonical_ancestor.join(suffix));
            }
        }

        Ok(self.config.root.clone())
    }

    fn workspace_path_for_issue(&self, issue: &Issue) -> Result<PathBuf, WorkspaceError> {
        let primary = self.workspace_path(&issue.id)?;
        if primary.exists() {
            return Ok(primary);
        }

        let Some(existing) = self.find_workspace_path_by_issue_id(&issue.id)? else {
            return Ok(primary);
        };
        if existing == primary {
            return Ok(primary);
        }

        fs::rename(&existing, &primary).map_err(|error| WorkspaceError::Io(error.to_string()))?;
        Ok(primary)
    }

    fn workspace_path_for_issue_id(&self, issue_id: &str) -> Result<PathBuf, WorkspaceError> {
        let primary = self.workspace_path(issue_id)?;
        if primary.exists() {
            return Ok(primary);
        }

        Ok(self
            .find_workspace_path_by_issue_id(issue_id)?
            .unwrap_or(primary))
    }

    fn find_workspace_path_by_issue_id(
        &self,
        issue_id: &str,
    ) -> Result<Option<PathBuf>, WorkspaceError> {
        if !self.config.root.exists() {
            return Ok(None);
        }

        for entry in fs::read_dir(&self.config.root)
            .map_err(|error| WorkspaceError::Io(error.to_string()))?
        {
            let entry = entry.map_err(|error| WorkspaceError::Io(error.to_string()))?;
            let path = entry.path();
            let manifest_path = path.join(".opensymphony/issue.json");
            let Some(manifest) = read_optional_json::<IssueManifest>(manifest_path)? else {
                continue;
            };
            if manifest.issue_id == issue_id {
                return Ok(Some(path));
            }
        }

        Ok(None)
    }
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) -> Result<(), WorkspaceError> {
    let json = serde_json::to_vec_pretty(value)
        .map_err(|error| WorkspaceError::Json(error.to_string()))?;
    fs::write(path, json).map_err(|error| WorkspaceError::Io(error.to_string()))
}

async fn read_pipe_to_end<T>(pipe: Option<T>) -> Result<Vec<u8>, std::io::Error>
where
    T: tokio::io::AsyncRead + Unpin,
{
    let Some(mut pipe) = pipe else {
        return Ok(vec![]);
    };

    let mut buffer = Vec::new();
    pipe.read_to_end(&mut buffer).await?;
    Ok(buffer)
}

fn read_optional_json<T: for<'de> Deserialize<'de>>(
    path: PathBuf,
) -> Result<Option<T>, WorkspaceError> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read(path).map_err(|error| WorkspaceError::Io(error.to_string()))?;
    let value = serde_json::from_slice(&contents)
        .map_err(|error| WorkspaceError::Json(error.to_string()))?;
    Ok(Some(value))
}

const fn default_hook_timeout_ms() -> u64 {
    60_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::tempdir;

    fn issue(identifier: &str) -> Issue {
        let timestamp = Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap();
        Issue {
            id: identifier.to_string(),
            identifier: identifier.to_string(),
            title: "Workspace".to_string(),
            description: None,
            priority: Some(1),
            state: "Todo".to_string(),
            labels: vec![],
            blocked_by: vec![],
            created_at: timestamp,
            updated_at: timestamp,
        }
    }

    #[tokio::test]
    async fn sanitizes_identifiers_without_path_escape() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig::default(),
        });

        let path = manager
            .workspace_path("../Bug: weird path")
            .expect("path should resolve");
        let resolved_root = root
            .path()
            .canonicalize()
            .expect("root should canonicalize");
        assert!(path.starts_with(&resolved_root));
        assert_eq!(
            path.file_name().expect("filename exists").to_string_lossy(),
            ".._Bug__weird_path"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canonicalizes_workspace_root_before_resolving_issue_path() {
        use std::os::unix::fs::symlink;

        let root = tempdir().expect("tempdir should exist");
        let real_root = root.path().join("real-root");
        fs::create_dir_all(&real_root).expect("real root should exist");
        let symlink_root = root.path().join("root-link");
        symlink(&real_root, &symlink_root).expect("symlink should exist");

        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: symlink_root,
            cleanup_terminal_workspaces: false,
            hooks: HookConfig::default(),
        });

        let context = manager
            .ensure_workspace(&issue("ABC-1"))
            .await
            .expect("workspace should resolve");

        assert_eq!(
            context.workspace_path,
            real_root
                .canonicalize()
                .expect("real root should canonicalize")
                .join("ABC-1")
        );
        assert!(context.workspace_path.starts_with(
            real_root
                .canonicalize()
                .expect("real root should canonicalize")
        ));
    }

    #[tokio::test]
    async fn runs_after_create_once_and_reuses_workspace() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig {
                after_create: Some(
                    "printf created >> .opensymphony/generated/marker.txt".to_string(),
                ),
                ..HookConfig::default()
            },
        });

        let issue = issue("ABC-1");
        manager
            .ensure_workspace(&issue)
            .await
            .expect("workspace should be created");
        manager
            .ensure_workspace(&issue)
            .await
            .expect("workspace should be reused");

        let marker =
            fs::read_to_string(root.path().join("ABC-1/.opensymphony/generated/marker.txt"))
                .expect("marker should exist");
        assert_eq!(marker, "created");
    }

    #[tokio::test]
    async fn reruns_after_create_until_bootstrap_succeeds() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig {
                after_create: Some(
                    "if [ ! -f .opensymphony/generated/fail-once ]; then touch .opensymphony/generated/fail-once; echo transient >&2; exit 1; fi; printf created >> .opensymphony/generated/marker.txt".to_string(),
                ),
                ..HookConfig::default()
            },
        });

        let issue = issue("ABC-1A");
        let error = manager
            .ensure_workspace(&issue)
            .await
            .expect_err("first bootstrap attempt should fail");
        assert!(matches!(error, WorkspaceError::HookFailed { .. }));

        manager
            .ensure_workspace(&issue)
            .await
            .expect("workspace bootstrap should retry successfully");
        manager
            .ensure_workspace(&issue)
            .await
            .expect("successful bootstrap should not rerun");

        let metadata_dir = root.path().join("ABC-1A/.opensymphony");
        assert!(metadata_dir.join("bootstrap.ok").exists());
        let marker = fs::read_to_string(metadata_dir.join("generated/marker.txt"))
            .expect("marker should exist");
        assert_eq!(marker, "created");
    }

    #[tokio::test]
    async fn runs_before_run_hook_inside_workspace() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig {
                before_run: Some("pwd > .opensymphony/generated/cwd.txt".to_string()),
                ..HookConfig::default()
            },
        });

        let issue = issue("ABC-2");
        let context = manager
            .prepare_issue_workspace(&issue, 1)
            .await
            .expect("workspace should prepare");
        let cwd = fs::read_to_string(context.metadata_dir.join("generated/cwd.txt"))
            .expect("cwd file should exist");
        assert!(cwd.trim().ends_with("/ABC-2"));
    }

    #[tokio::test]
    async fn fails_required_hook_when_timeout_expires() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig {
                before_run: Some(
                    "sleep 0.2; printf leaked > .opensymphony/generated/late.txt".to_string(),
                ),
                timeout_ms: 10,
                ..HookConfig::default()
            },
        });

        let error = manager
            .prepare_issue_workspace(&issue("ABC-3"), 1)
            .await
            .expect_err("timeout should fail");

        assert!(matches!(error, WorkspaceError::HookTimeout { .. }));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!root
            .path()
            .join("ABC-3/.opensymphony/generated/late.txt")
            .exists());
    }

    #[tokio::test]
    async fn migrates_legacy_identifier_workspace_to_stable_issue_id_key() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig::default(),
        });
        let legacy_issue = Issue {
            id: "1".to_string(),
            identifier: "ABC-1".to_string(),
            title: "Workspace".to_string(),
            description: None,
            priority: Some(1),
            state: "Todo".to_string(),
            labels: vec![],
            blocked_by: vec![],
            created_at: Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap(),
        };

        let legacy_workspace = root.path().join("ABC-1");
        let legacy_metadata = legacy_workspace.join(".opensymphony");
        fs::create_dir_all(&legacy_metadata).expect("legacy metadata should exist");
        fs::write(legacy_metadata.join("bootstrap.ok"), b"ok")
            .expect("bootstrap marker should exist");
        fs::write(
            legacy_metadata.join("conversation.json"),
            b"{\"issue_id\":\"1\"}",
        )
        .expect("legacy conversation placeholder should exist");
        write_json(
            legacy_metadata.join("issue.json"),
            &IssueManifest {
                issue_id: legacy_issue.id.clone(),
                identifier: legacy_issue.identifier.clone(),
                title: legacy_issue.title.clone(),
                current_state: legacy_issue.state.clone(),
                sanitized_workspace_key: "ABC-1".to_string(),
                workspace_path: legacy_workspace.display().to_string(),
                created_at: legacy_issue.created_at,
                updated_at: legacy_issue.updated_at,
                last_seen_tracker_refresh_at: legacy_issue.updated_at,
                last_attempt: 1,
            },
        )
        .expect("legacy issue manifest should persist");

        let renamed_issue = Issue {
            identifier: "ABC-99".to_string(),
            ..legacy_issue
        };
        let context = manager
            .ensure_workspace(&renamed_issue)
            .await
            .expect("workspace should migrate to stable issue id key");

        assert_eq!(
            context.workspace_path,
            manager
                .workspace_path("1")
                .expect("stable workspace path should resolve")
        );
        assert!(!legacy_workspace.exists());
        assert!(context.metadata_dir.join("conversation.json").exists());

        let issue_manifest: IssueManifest =
            read_optional_json(context.metadata_dir.join("issue.json"))
                .expect("issue manifest load should succeed")
                .expect("issue manifest should exist");
        assert_eq!(issue_manifest.identifier, "ABC-99");
        assert_eq!(issue_manifest.sanitized_workspace_key, "1");
    }

    #[tokio::test]
    async fn cleans_up_terminal_workspace_when_configured() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: true,
            hooks: HookConfig::default(),
        });
        let issue = issue("ABC-4");
        manager
            .ensure_workspace(&issue)
            .await
            .expect("workspace should exist");

        manager
            .cleanup_terminal_workspace(&issue)
            .await
            .expect("cleanup should succeed");

        assert!(!root.path().join("ABC-4").exists());
    }

    #[tokio::test]
    async fn clears_retry_manifest_when_retaining_terminal_workspace() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig::default(),
        });
        let issue = issue("ABC-5");
        manager
            .ensure_workspace(&issue)
            .await
            .expect("workspace should exist");
        manager
            .persist_retry(&RetryEntry {
                issue: issue.clone(),
                attempt: 2,
                reason: RetryReason::Continuation,
                scheduled_at: Utc::now(),
            })
            .expect("retry should persist");

        manager
            .cleanup_terminal_workspace(&issue)
            .await
            .expect("cleanup should succeed");

        assert!(root.path().join("ABC-5").exists());
        assert!(!root.path().join("ABC-5/.opensymphony/retry.json").exists());
    }

    #[tokio::test]
    async fn does_not_run_before_remove_when_retaining_terminal_workspace() {
        let root = tempdir().expect("tempdir should exist");
        let manager = WorkspaceManager::new(WorkspaceConfig {
            root: root.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig {
                before_remove: Some(
                    "printf removed > .opensymphony/generated/before-remove.txt".to_string(),
                ),
                ..HookConfig::default()
            },
        });
        let issue = issue("ABC-6");
        manager
            .ensure_workspace(&issue)
            .await
            .expect("workspace should exist");

        manager
            .cleanup_terminal_workspace(&issue)
            .await
            .expect("cleanup should succeed");

        assert!(root.path().join("ABC-6").exists());
        assert!(!root
            .path()
            .join("ABC-6/.opensymphony/generated/before-remove.txt")
            .exists());
    }
}
