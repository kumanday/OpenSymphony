use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// Read-only task graph node exposed by the gateway.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskGraphNode {
    pub schema_version: SchemaVersion,
    pub node_id: String,
    pub kind: TaskGraphNodeKind,
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub state_category: TaskGraphStateCategory,
    pub priority: Option<u8>,
    /// Parent node identifier when the parent is present in this task graph snapshot.
    pub parent_id: Option<String>,
    /// Child node identifiers that are present in this task graph snapshot.
    pub children: Vec<String>,
    /// Blocker node identifiers that are present in this task graph snapshot.
    pub blocked_by: Vec<String>,
    pub url: Option<String>,
    pub branch_name: Option<String>,
    pub labels: Vec<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// Estimated effort in minutes.
    pub estimate_minutes: Option<u32>,
    /// Runtime overlay when the node has active or recent run data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_overlay: Option<TaskGraphRuntimeOverlay>,
}

/// Runtime overlay attached to a task graph node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskGraphRuntimeOverlay {
    /// Whether the node is eligible for execution.
    pub eligible: bool,
    /// Whether the node is queued for execution.
    pub queued: bool,
    /// Active run identifier when a worker is currently executing this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_run_id: Option<String>,
    /// Last known outcome (e.g. "completed", "failed", "canceled").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<String>,
    /// Number of retry attempts so far.
    pub retry_count: u32,
    /// Logical workspace identifier for hosted mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    /// Harness type (e.g. "openhands").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_type: Option<String>,
    /// Conversation/session identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Timestamp of the last observed event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    /// High-level diff summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<DiffSummary>,
    /// Validation status when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_status: Option<String>,
    /// Blocker or dependency summary when the node is blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker_summary: Option<String>,
}

/// Compact summary of file changes produced by a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffSummary {
    pub files_added: u32,
    pub files_modified: u32,
    pub files_removed: u32,
    pub lines_added: u32,
    pub lines_removed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskGraphNodeKind {
    Milestone,
    Issue,
    SubIssue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskGraphStateCategory {
    Backlog,
    Todo,
    InProgress,
    Done,
    Canceled,
}

/// Flat list response for a project task graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskGraphSnapshot {
    pub schema_version: SchemaVersion,
    pub project_id: String,
    pub generated_at: DateTime<Utc>,
    pub nodes: Vec<TaskGraphNode>,
    pub root_ids: Vec<String>,
}
