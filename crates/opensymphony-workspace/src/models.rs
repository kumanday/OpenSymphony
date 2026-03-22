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

    pub(crate) fn after_create_receipt_path(&self) -> PathBuf {
        self.workspace_path.join(".opensymphony.after_create.json")
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

    pub fn latest_prompt_path(&self, kind: PromptKind) -> PathBuf {
        self.prompts_dir()
            .join(format!("last-{}-prompt.md", kind.file_stem()))
    }

    pub fn latest_prompt_manifest_path(&self, kind: PromptKind) -> PathBuf {
        self.prompts_dir()
            .join(format!("last-{}-prompt.json", kind.file_stem()))
    }

    pub fn run_artifacts_dir(&self, attempt: u32) -> PathBuf {
        self.runs_dir().join(format!("attempt-{attempt:04}"))
    }

    pub fn run_prompt_path(&self, attempt: u32, kind: PromptKind, sequence: u32) -> PathBuf {
        self.run_artifacts_dir(attempt)
            .join(format!("prompt-{}-{sequence:03}.md", kind.file_stem()))
    }

    pub fn run_prompt_manifest_path(
        &self,
        attempt: u32,
        kind: PromptKind,
        sequence: u32,
    ) -> PathBuf {
        self.run_artifacts_dir(attempt)
            .join(format!("prompt-{}-{sequence:03}.json", kind.file_stem()))
    }

    pub fn issue_context_path(&self) -> PathBuf {
        self.generated_dir().join("issue-context.md")
    }

    pub fn session_context_path(&self) -> PathBuf {
        self.generated_dir().join("session-context.json")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AfterCreateBootstrapReceipt {
    pub issue_id: String,
    pub identifier: String,
    pub sanitized_workspace_key: String,
    pub workspace_path: PathBuf,
    pub completed_at: DateTime<Utc>,
}

impl AfterCreateBootstrapReceipt {
    pub(crate) fn new(workspace: &WorkspaceHandle, issue: &IssueDescriptor) -> Self {
        Self {
            issue_id: issue.issue_id.clone(),
            identifier: issue.identifier.clone(),
            sanitized_workspace_key: workspace.workspace_key().to_string(),
            workspace_path: workspace.workspace_path().to_path_buf(),
            completed_at: Utc::now(),
        }
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

impl fmt::Display for RunStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Preparing => "preparing",
            Self::Prepared => "prepared",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::PreparationFailed => "preparation_failed",
        })
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationManifest {
    pub issue_id: String,
    pub identifier: String,
    pub conversation_id: String,
    pub server_base_url: String,
    pub persistence_dir: PathBuf,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attached_at: Option<DateTime<Utc>>,
    pub fresh_conversation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_reason: Option<String>,
    pub runtime_contract_version: String,
}

impl ConversationManifest {
    pub fn new(
        workspace: &WorkspaceHandle,
        conversation_id: impl Into<String>,
        server_base_url: impl Into<String>,
        persistence_dir: impl Into<PathBuf>,
        runtime_contract_version: impl Into<String>,
    ) -> Self {
        Self {
            issue_id: workspace.issue_id().to_string(),
            identifier: workspace.identifier().to_string(),
            conversation_id: conversation_id.into(),
            server_base_url: server_base_url.into(),
            persistence_dir: persistence_dir.into(),
            created_at: Utc::now(),
            last_attached_at: None,
            fresh_conversation: true,
            reset_reason: None,
            runtime_contract_version: runtime_contract_version.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Full,
    Continuation,
}

impl PromptKind {
    pub fn file_stem(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Continuation => "continuation",
        }
    }
}

impl fmt::Display for PromptKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "full",
            Self::Continuation => "continuation",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptCaptureDescriptor {
    pub kind: PromptKind,
    pub sequence: u32,
}

impl PromptCaptureDescriptor {
    pub fn new(kind: PromptKind, sequence: u32) -> Self {
        Self { kind, sequence }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCaptureManifest {
    pub issue_id: String,
    pub identifier: String,
    pub run_id: String,
    pub attempt: u32,
    pub prompt_kind: PromptKind,
    pub sequence: u32,
    pub workspace_path: PathBuf,
    pub archived_prompt_path: PathBuf,
    pub stable_prompt_path: PathBuf,
    pub captured_at: DateTime<Utc>,
    pub prompt_length_bytes: usize,
}

impl PromptCaptureManifest {
    pub fn new(
        workspace: &WorkspaceHandle,
        run: &RunDescriptor,
        descriptor: PromptCaptureDescriptor,
        prompt: &str,
    ) -> Self {
        Self {
            issue_id: workspace.issue_id().to_string(),
            identifier: workspace.identifier().to_string(),
            run_id: run.run_id.clone(),
            attempt: run.attempt,
            prompt_kind: descriptor.kind,
            sequence: descriptor.sequence,
            workspace_path: workspace.workspace_path().to_path_buf(),
            archived_prompt_path: workspace.run_prompt_path(
                run.attempt,
                descriptor.kind,
                descriptor.sequence,
            ),
            stable_prompt_path: workspace.latest_prompt_path(descriptor.kind),
            captured_at: Utc::now(),
            prompt_length_bytes: prompt.len(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueContextArtifact {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub current_state: String,
    pub repo_workflow_path: PathBuf,
    pub repo_agents_path: Option<PathBuf>,
    pub repo_skills_dir: Option<PathBuf>,
    pub last_run_status: Option<RunStatus>,
    pub important_constraints: Vec<String>,
    pub known_blockers: Vec<String>,
}

impl IssueContextArtifact {
    pub fn render_markdown(&self, workspace: &WorkspaceHandle) -> String {
        use std::fmt::Write as _;

        let mut output = String::new();
        let _ = writeln!(output, "# OpenSymphony Issue Context");
        let _ = writeln!(output);
        let _ = writeln!(output, "Repository-owned policy remains authoritative.");
        let _ = writeln!(
            output,
            "These generated notes reference repo-owned files without overwriting them."
        );
        let _ = writeln!(output);
        let _ = writeln!(output, "- issue: {}", self.identifier);
        let _ = writeln!(output, "- issue id: {}", self.issue_id);
        let _ = writeln!(output, "- title: {}", self.title);
        let _ = writeln!(output, "- current state: {}", self.current_state);
        let _ = writeln!(
            output,
            "- last run status: {}",
            self.last_run_status
                .map(|status| status.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        );
        let _ = writeln!(output);
        let _ = writeln!(output, "## Repository Context");
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "- WORKFLOW.md: {}",
            self.repo_workflow_path.display()
        );
        let _ = writeln!(
            output,
            "- AGENTS.md: {}",
            self.repo_agents_path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "absent".to_string())
        );
        let _ = writeln!(
            output,
            "- .agents/skills/: {}",
            self.repo_skills_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "absent".to_string())
        );
        let _ = writeln!(output);
        let _ = writeln!(output, "## OpenSymphony Artifacts");
        let _ = writeln!(output);
        let _ = writeln!(
            output,
            "- issue manifest: {}",
            workspace.issue_manifest_path().display()
        );
        let _ = writeln!(
            output,
            "- run manifest: {}",
            workspace.run_manifest_path().display()
        );
        let _ = writeln!(
            output,
            "- conversation manifest: {}",
            workspace.conversation_manifest_path().display()
        );
        let _ = writeln!(
            output,
            "- latest full prompt: {}",
            workspace.latest_prompt_path(PromptKind::Full).display()
        );
        let _ = writeln!(
            output,
            "- latest continuation prompt: {}",
            workspace
                .latest_prompt_path(PromptKind::Continuation)
                .display()
        );
        let _ = writeln!(
            output,
            "- session context: {}",
            workspace.session_context_path().display()
        );
        if !self.important_constraints.is_empty() {
            let _ = writeln!(output);
            let _ = writeln!(output, "## Important Constraints");
            let _ = writeln!(output);
            for constraint in &self.important_constraints {
                let _ = writeln!(output, "- {constraint}");
            }
        }
        if !self.known_blockers.is_empty() {
            let _ = writeln!(output);
            let _ = writeln!(output, "## Known Blockers");
            let _ = writeln!(output);
            for blocker in &self.known_blockers {
                let _ = writeln!(output, "- {blocker}");
            }
        }

        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContextArtifact {
    pub issue_id: String,
    pub identifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_status: Option<RunStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_kind: Option<PromptKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_path: Option<PathBuf>,
    #[serde(default)]
    pub recent_validation_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_retry_reason: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl SessionContextArtifact {
    pub fn new(workspace: &WorkspaceHandle) -> Self {
        Self {
            issue_id: workspace.issue_id().to_string(),
            identifier: workspace.identifier().to_string(),
            conversation_id: None,
            attempt: None,
            last_run_id: None,
            last_run_status: None,
            last_prompt_kind: None,
            last_prompt_path: None,
            recent_validation_commands: Vec::new(),
            last_retry_reason: None,
            updated_at: Utc::now(),
        }
    }
}
