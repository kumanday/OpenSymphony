use std::{
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDescriptor {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub current_state: String,
    pub last_seen_tracker_refresh_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDescriptor {
    pub run_id: String,
    pub attempt: u32,
}

impl RunDescriptor {
    pub fn new(run_id: impl Into<String>, attempt: u32) -> Self {
        Self {
            run_id: run_id.into(),
            attempt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDefinition {
    pub command: String,
    pub cwd: Option<PathBuf>,
}

impl HookDefinition {
    pub fn shell(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
        }
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookConfig {
    pub after_create: Option<HookDefinition>,
    pub before_run: Option<HookDefinition>,
    pub after_run: Option<HookDefinition>,
    pub before_remove: Option<HookDefinition>,
    pub timeout: Duration,
}

impl Default for HookConfig {
    fn default() -> Self {
        Self {
            after_create: None,
            before_run: None,
            after_run: None,
            before_remove: None,
            timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CleanupConfig {
    pub remove_terminal_workspaces: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceManagerConfig {
    pub root: PathBuf,
    pub hooks: HookConfig,
    pub cleanup: CleanupConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHandle {
    issue_id: String,
    identifier: String,
    workspace_key: String,
    workspace_path: PathBuf,
}

impl WorkspaceHandle {
    pub(crate) fn new(
        issue_id: impl Into<String>,
        identifier: impl Into<String>,
        workspace_key: impl Into<String>,
        workspace_path: PathBuf,
    ) -> Self {
        Self {
            issue_id: issue_id.into(),
            identifier: identifier.into(),
            workspace_key: workspace_key.into(),
            workspace_path,
        }
    }

    pub fn issue_id(&self) -> &str {
        &self.issue_id
    }

    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    pub fn workspace_key(&self) -> &str {
        &self.workspace_key
    }

    pub fn workspace_path(&self) -> &Path {
        &self.workspace_path
    }

    pub fn metadata_dir(&self) -> PathBuf {
        self.workspace_path.join(".opensymphony")
    }

    pub fn issue_manifest_path(&self) -> PathBuf {
        self.metadata_dir().join("issue.json")
    }

    pub fn run_manifest_path(&self) -> PathBuf {
        self.metadata_dir().join("run.json")
    }

    pub fn conversation_manifest_path(&self) -> PathBuf {
        self.metadata_dir().join("conversation.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.metadata_dir().join("logs")
    }

    pub fn generated_dir(&self) -> PathBuf {
        self.metadata_dir().join("generated")
    }

    pub fn openhands_dir(&self) -> PathBuf {
        self.metadata_dir().join("openhands")
    }

    pub fn prompts_dir(&self) -> PathBuf {
        self.metadata_dir().join("prompts")
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.metadata_dir().join("runs")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsureWorkspaceResult {
    pub handle: WorkspaceHandle,
    pub issue_manifest: IssueManifest,
    pub created: bool,
    pub after_create: Option<HookExecutionRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueLifecycleState {
    Active,
    Inactive,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupDecision {
    Retain,
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupOutcome {
    pub decision: CleanupDecision,
    pub before_remove: Option<HookExecutionRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookKind {
    AfterCreate,
    BeforeRun,
    AfterRun,
    BeforeRemove,
}

impl HookKind {
    pub fn is_required(self) -> bool {
        matches!(self, Self::AfterCreate | Self::BeforeRun)
    }
}

impl fmt::Display for HookKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AfterCreate => "after_create",
            Self::BeforeRun => "before_run",
            Self::AfterRun => "after_run",
            Self::BeforeRemove => "before_remove",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookExecutionStatus {
    Succeeded,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookExecutionRecord {
    pub kind: HookKind,
    pub command: String,
    pub cwd: PathBuf,
    pub best_effort: bool,
    pub status: HookExecutionStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueManifest {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub current_state: String,
    pub sanitized_workspace_key: String,
    pub workspace_path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_tracker_refresh_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Preparing,
    Prepared,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    PreparationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunManifest {
    pub run_id: String,
    pub issue_id: String,
    pub identifier: String,
    pub sanitized_workspace_key: String,
    pub workspace_path: PathBuf,
    pub attempt: u32,
    pub status: RunStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_detail: Option<String>,
    #[serde(default)]
    pub hooks: Vec<HookExecutionRecord>,
}

impl RunManifest {
    pub fn new(workspace: &WorkspaceHandle, run: &RunDescriptor) -> Self {
        let now = Utc::now();
        Self {
            run_id: run.run_id.clone(),
            issue_id: workspace.issue_id().to_string(),
            identifier: workspace.identifier().to_string(),
            sanitized_workspace_key: workspace.workspace_key().to_string(),
            workspace_path: workspace.workspace_path().to_path_buf(),
            attempt: run.attempt,
            status: RunStatus::Preparing,
            created_at: now,
            updated_at: now,
            status_detail: None,
            hooks: Vec::new(),
        }
    }
}
