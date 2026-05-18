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
    pub max_turns: u32,
    pub retry_attempt: Option<u32>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Unclaimed,
    Claimed,
    Running,
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
