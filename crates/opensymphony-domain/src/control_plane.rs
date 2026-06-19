use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEnvelope {
    pub sequence: u64,
    pub published_at: DateTime<Utc>,
    pub snapshot: ControlPlaneDaemonSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneDaemonSnapshot {
    pub generated_at: DateTime<Utc>,
    pub daemon: ControlPlaneDaemonStatus,
    pub agent_server: ControlPlaneAgentServerStatus,
    #[serde(default)]
    pub memory_server: ControlPlaneMemoryServerStatus,
    pub metrics: ControlPlaneMetricsSnapshot,
    pub issues: Vec<ControlPlaneIssueSnapshot>,
    pub recent_events: Vec<ControlPlaneRecentEvent>,
}

impl ControlPlaneDaemonSnapshot {
    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneDaemonStatus {
    pub state: ControlPlaneDaemonState,
    pub last_poll_at: DateTime<Utc>,
    pub workspace_root: String,
    pub status_line: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneDaemonState {
    Starting,
    Ready,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneAgentServerStatus {
    pub reachable: bool,
    pub base_url: String,
    pub conversation_count: u32,
    pub status_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneMemoryServerStatus {
    pub enabled: bool,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub status_line: String,
}

impl Default for ControlPlaneMemoryServerStatus {
    fn default() -> Self {
        Self {
            enabled: false,
            reachable: false,
            endpoint: None,
            status_line: "disabled".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneMetricsSnapshot {
    pub running_issues: u32,
    pub retry_queue_depth: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_tokens: u64,
    pub total_cost_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneIssueSnapshot {
    pub identifier: String,
    pub title: String,
    pub tracker_state: String,
    pub runtime_state: ControlPlaneIssueRuntimeState,
    pub last_outcome: ControlPlaneWorkerOutcome,
    pub last_event_at: DateTime<Utc>,
    pub conversation_id_suffix: String,
    pub workspace_path_suffix: String,
    pub retry_count: u32,
    pub blocked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_query_param_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_events: Vec<ControlPlaneConversationEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modified_files: Vec<ControlPlaneFileChange>,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    /// True when the harness has been detached from the run (local operator or
    /// host requested disconnect without a clean terminal state).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detached: bool,
    /// True when the harness acknowledged a cancel/force-stop request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_acknowledged: bool,
    /// True when a cancel/force-stop request was not acknowledged and the run
    /// ended in a cancel-failed state.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_failed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneConversationEvent {
    pub event_id: String,
    pub happened_at: DateTime<Utc>,
    pub kind: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Monotonic sequence number assigned by the event producer. Used by the
    /// gateway to report a stable ordering key even when the snapshot truncates
    /// or reorders events.
    #[serde(default)]
    pub sequence: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneFileChange {
    pub path: String,
    pub change_kind: ControlPlaneFileChangeKind,
    pub lines_added: u32,
    pub lines_removed: u32,
    /// Optional unified diff text for the file. When present, the gateway will
    /// parse it into line-level hunks instead of returning an empty summary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneFileChangeKind {
    Created,
    Modified,
    Removed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneIssueRuntimeState {
    Idle,
    Running,
    Paused,
    RetryQueued,
    Releasing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneWorkerOutcome {
    Unknown,
    Running,
    Continued,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneRecentEvent {
    pub happened_at: DateTime<Utc>,
    pub issue_identifier: Option<String>,
    pub kind: ControlPlaneRecentEventKind,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlPlaneRecentEventKind {
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

impl ControlPlaneRecentEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ControlPlaneRecentEventKind::WorkerStarted => "worker_started",
            ControlPlaneRecentEventKind::WorkspacePrepared => "workspace_prepared",
            ControlPlaneRecentEventKind::StreamAttached => "stream_attached",
            ControlPlaneRecentEventKind::SnapshotPublished => "snapshot_published",
            ControlPlaneRecentEventKind::WorkerCompleted => "worker_completed",
            ControlPlaneRecentEventKind::RetryScheduled => "retry_scheduled",
            ControlPlaneRecentEventKind::ClientAttached => "client_attached",
            ControlPlaneRecentEventKind::ClientDetached => "client_detached",
            ControlPlaneRecentEventKind::Warning => "warning",
        }
    }
}
