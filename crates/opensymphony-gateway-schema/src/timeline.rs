use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{envelope::EntityKind, run::RunPhase, run::RunStreamLiveness, version::SchemaVersion};

/// High-level category for a grouped timeline entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineEntryKind {
    /// A coarse runtime phase such as waiting-on-prior-turn or stalled.
    Phase,
    /// A single tool call or a collapsed sequence of tool calls.
    ToolCall,
    /// A command executed inside a terminal/log session.
    Command,
    /// A token-usage update from the harness.
    TokenUpdate,
    /// Stream connect, disconnect, or reconnect/reconcile activity.
    Reconnect,
    /// A stall-probe event that explains why a run is stalled or quiet.
    StallProbe,
    /// A generic progress snapshot (worker started, status update, etc.).
    Progress,
    /// A run lifecycle state change (started, completed, failed, cancelled, retry queued).
    State,
    /// A log entry line.
    Log,
    /// A terminal/log frame.
    Terminal,
    /// A file or diff related event.
    File,
    /// Unclassified events.
    Unknown,
}

/// A typed reference to an entity linked to a timeline entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntityRef {
    pub kind: EntityKind,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

impl TimelineEntityRef {
    pub fn run(id: impl Into<String>) -> Self {
        Self {
            kind: EntityKind::Run,
            id: id.into(),
            identifier: None,
        }
    }

    pub fn issue(id: impl Into<String>, identifier: impl Into<String>) -> Self {
        Self {
            kind: EntityKind::Issue,
            id: id.into(),
            identifier: Some(identifier.into()),
        }
    }

    pub fn sub_issue(id: impl Into<String>) -> Self {
        Self {
            kind: EntityKind::SubIssue,
            id: id.into(),
            identifier: None,
        }
    }

    pub fn terminal(id: impl Into<String>) -> Self {
        Self {
            kind: EntityKind::TerminalSession,
            id: id.into(),
            identifier: None,
        }
    }
}

/// Delta of token usage observed in a single timeline entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenDelta {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
}

/// Evidence surfaced for a run state so users can inspect why the run is in a
/// given liveness phase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStateEvidence {
    pub phase: RunPhase,
    pub stream: RunStreamLiveness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stall_deadline_at: Option<DateTime<Utc>>,
    pub explanation: String,
}

/// A single grouped entry in the runtime timeline for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    /// Stable entry id (derived from the first grouped event id).
    pub entry_id: String,
    /// First journal sequence included in this entry.
    pub sequence_start: u64,
    /// Last journal sequence included in this entry.
    pub sequence_end: u64,
    /// Wall-clock timestamp of the first grouped event.
    pub happened_at: DateTime<Utc>,
    pub kind: TimelineEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<RunPhase>,
    /// Human-readable title for the entry.
    pub title: String,
    /// One-line summary of what happened in this group.
    pub summary: String,
    /// Journal event ids that contributed to this entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_ids: Vec<String>,
    /// Entity references (run, issue, sub-issue, terminal session, file, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_refs: Vec<TimelineEntityRef>,
    /// Command id when the entry represents a command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    /// Tool name when the entry represents tool activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Workspace-relative file paths referenced by the entry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_paths: Vec<String>,
    /// Terminal session id when the entry is backed by terminal/log frames.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_session_id: Option<String>,
    /// Log level for log entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    /// Token delta if the entry represents a token update.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_delta: Option<TokenDelta>,
    /// Evidence explaining the run state behind this entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_evidence: Option<RunStateEvidence>,
}

/// Grouped runtime timeline for a single run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTimeline {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<TimelineEntry>,
}

/// Search result returned by terminal/log search endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSearchResult {
    pub schema_version: SchemaVersion,
    pub terminal_session_id: String,
    pub query: String,
    pub matches: Vec<TerminalSearchMatch>,
}

/// A single search match inside a terminal/log session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSearchMatch {
    pub frame_sequence: u64,
    pub frame_timestamp: DateTime<Utc>,
    /// Snippet around the matching text.
    pub snippet: String,
}

/// Result of a jump-to-event query for a terminal/log session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalJumpResult {
    pub schema_version: SchemaVersion,
    pub terminal_session_id: String,
    pub event_id: String,
    pub frame_sequence: Option<u64>,
    pub found: bool,
}

/// Paged log response for `/api/v1/runs/{run_id}/logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLogPage {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub next_cursor: Option<u64>,
    pub entries: Vec<RunLogEntry>,
}

/// A single log entry for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLogEntry {
    pub sequence: u64,
    pub event_id: String,
    pub happened_at: DateTime<Utc>,
    pub level: String,
    pub message: String,
    pub terminal_session_id: Option<String>,
    pub command_id: Option<String>,
}
