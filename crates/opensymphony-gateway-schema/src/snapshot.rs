use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{cursor::PageCursor, version::SchemaVersion};

/// Dashboard-level snapshot delivered over REST or SSE.
///
/// This is the v1 public contract. It deliberately avoids leaking
/// orchestrator internals (e.g. raw `IssueExecution`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub schema_version: SchemaVersion,
    pub generated_at: DateTime<Utc>,
    pub sequence: u64,
    pub health: GatewayHealth,
    pub metrics: GatewayMetrics,
    pub projects: Vec<ProjectSummary>,
    pub recent_events: Vec<SnapshotEventSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayHealth {
    Healthy,
    Degraded,
    Failed,
    Starting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayMetrics {
    pub running_issue_count: u32,
    pub retry_queue_depth: u32,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cost_micros: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub milestone_count: u32,
    pub issue_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
}

/// List of projects returned by `GET /api/v1/projects`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectList {
    pub schema_version: SchemaVersion,
    pub projects: Vec<ProjectSummary>,
}

/// Project detail returned by `GET /api/v1/projects/{project_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDetail {
    pub schema_version: SchemaVersion,
    pub project_id: String,
    pub name: String,
    pub milestone_count: u32,
    pub issue_count: u32,
    pub running_count: u32,
    pub completed_count: u32,
    pub failed_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub milestones: Vec<ProjectMilestoneSummary>,
}

/// Compact milestone summary inside a project detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMilestoneSummary {
    pub milestone_id: String,
    pub name: String,
    pub issue_count: u32,
}

/// Paged sub-issue list returned by `GET /api/v1/projects/{project_id}/issues`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIssuesPage {
    pub schema_version: SchemaVersion,
    pub project_id: String,
    pub next_cursor: Option<PageCursor>,
    pub issues: Vec<ProjectIssueSummary>,
}

/// Single issue summary inside a project issues list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIssueSummary {
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub priority: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEventSummary {
    pub happened_at: DateTime<Utc>,
    pub issue_identifier: Option<String>,
    pub kind: SnapshotEventKind,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotEventKind {
    WorkerStarted,
    WorkspacePrepared,
    StreamAttached,
    SnapshotPublished,
    WorkerCompleted,
    RetryScheduled,
    ClientAttached,
    ClientDetached,
    Warning,
}
