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
    pub parent_id: Option<String>,
    pub children: Vec<String>,
    pub blocked_by: Vec<String>,
    pub url: Option<String>,
    pub branch_name: Option<String>,
    pub labels: Vec<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    /// Estimated effort in minutes.
    pub estimate_minutes: Option<u32>,
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
