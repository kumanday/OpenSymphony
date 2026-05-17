use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{cursor::PageCursor, version::SchemaVersion};

/// Run detail exposed by `/api/v1/runs/{run_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunDetail {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub worker_id: String,
    pub status: RunStatus,
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
    pub workspace_path: Option<String>,
    pub error: Option<String>,
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
