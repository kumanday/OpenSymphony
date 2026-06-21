use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{cursor::PageCursor, version::SchemaVersion};

/// Run lifecycle states that reflect the full lifecycle including eligibility
/// and workspace state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLifecycleState {
    Eligible,
    Queued,
    Claimed,
    Running,
    Paused,
    Releasing,
    Completed,
    Failed,
    Canceled,
    RetryExhausted,
}

/// Run detail exposed by `/api/v1/runs/{run_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDetail {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub worker_id: String,
    pub status: RunStatus,
    pub lifecycle_state: RunLifecycleState,
    pub claimed_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub release_reason: Option<ReleaseReason>,
    pub turn_count: u32,
    /// Configured turn budget. A value of 0 means the budget is unknown.
    pub max_turns: u32,
    pub retry_attempt: Option<u32>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    /// Elapsed runtime in whole seconds. A value of 0 means runtime is unknown
    /// unless the run is actively running and has a start timestamp.
    pub runtime_seconds: u64,
    pub conversation_id: Option<String>,
    /// Logical workspace identifier for hosted mode.
    pub workspace_id: Option<String>,
    /// Local filesystem path (absent in hosted mode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Harness type (e.g. "openhands").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_type: Option<String>,
    /// Brief human-readable summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Blocker description when the run is blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    pub error: Option<String>,
    /// Actions the client may perform on this run.
    #[serde(default)]
    pub allowed_actions: Vec<RunAction>,
    /// Liveness envelope describing the phase, stream health, and latest progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub liveness: Option<RunLivenessEnvelope>,
    /// Diagnostic hints surfaced when multiple subsystems disagree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<RunDiagnostics>,
    /// Actions the client may safely invoke in the current state.
    #[serde(default)]
    pub safe_actions: SafeActions,
    /// True when the harness session has been detached from the run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detached: bool,
    /// True when the harness acknowledged a cancel/force-stop request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_acknowledged: bool,
    /// True when a cancel/force-stop request was not acknowledged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_failed: bool,
}

/// Action a client may dispatch on a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAction {
    Retry,
    Cancel,
    Pause,
    Resume,
    Rehydrate,
    Detach,
    Comment,
    CreateFollowup,
    OpenWorkspace,
    Debug,
}

/// Operational phase observed by the client for a long-running run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Active,
    Quiet,
    Degraded,
    Stalled,
    RetryQueued,
    Cancelled,
    Detached,
    Completed,
}

/// Stream-level liveness classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStreamLiveness {
    Healthy,
    Stale,
    Dead,
    Detached,
    Degraded,
    Stalled,
}

/// Compact snapshot of the current run liveness surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLivenessEnvelope {
    pub phase: RunPhase,
    pub stream: RunStreamLiveness,
    pub latest_progress: Option<RunProgress>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub harness_acknowledged: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_failed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detached: bool,
}

/// Progress event emitted during a long-running run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunProgress {
    pub sequence: u64,
    pub event_id: String,
    pub happened_at: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
}

/// Diagnostic hints for operator-facing tooling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDiagnostics {
    /// True when the scheduler reports retry queued but the harness session
    /// still appears active for the same issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_scheduler_disagreement: Option<HarnessSchedulerDisagreement>,
    /// True when the harness acknowledged a cancel/force-stop request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_acknowledged: bool,
    /// True when a cancel/force-stop request was not acknowledged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_failed: bool,
}

/// Details of a harness/scheduler disagreement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessSchedulerDisagreement {
    pub scheduler_status: RunStatus,
    pub harness_status: String,
    pub detected_at: DateTime<Utc>,
    pub resolution_path: String,
}

/// Actions the client may safely invoke in the current run state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeActions {
    #[serde(default)]
    pub retry: bool,
    #[serde(default)]
    pub cancel: bool,
    #[serde(default)]
    pub rehydrate: bool,
    #[serde(default)]
    pub detach: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Unclaimed,
    Claimed,
    Running,
    Paused,
    RetryQueued,
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    Completed,
    TrackerInactive,
    TrackerTerminal,
    Cancelled,
    CancelFailed,
    RetryExhausted,
}

/// Paged run events for `/api/v1/runs/{run_id}/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEventPage {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub next_cursor: Option<PageCursor>,
    pub events: Vec<RunEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEvent {
    pub sequence: u64,
    pub event_id: String,
    pub happened_at: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
    /// Typed payload when the kind is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// Original raw payload for forward compatibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<serde_json::Value>,
}

/// File change kind inside a changed-files entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Created,
    Modified,
    Removed,
}

/// Single changed-file entry for `/api/v1/runs/{run_id}/files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFileEntry {
    /// Workspace-relative path (never a raw absolute local path).
    pub path: String,
    pub change_kind: FileChangeKind,
    pub lines_added: u32,
    pub lines_removed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

/// Paged changed-files response for `/api/v1/runs/{run_id}/files`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFilesPage {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub next_cursor: Option<PageCursor>,
    pub files: Vec<ChangedFileEntry>,
}

/// Single line inside a diff hunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiffLine {
    Context { line: String },
    Addition { line: String },
    Deletion { line: String },
}

/// A contiguous hunk inside a unified diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffHunk {
    /// Path of the file this hunk belongs to, relative to the workspace root.
    pub file_path: String,
    pub header: String,
    pub start_line: u32,
    pub old_line_count: u32,
    pub new_line_count: u32,
    pub lines: Vec<DiffLine>,
}

/// Paged diff response for `/api/v1/runs/{run_id}/diffs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileDiffPage {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub file_path: String,
    pub next_cursor: Option<PageCursor>,
    pub hunks: Vec<DiffHunk>,
    pub total_lines_added: u32,
    pub total_lines_removed: u32,
}
