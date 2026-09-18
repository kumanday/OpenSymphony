use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ReleaseReason, RepositoryBindingOutcome};

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
    /// Full Codex thread id for Codex-app-server runs, so operators can open
    /// the `codex://threads/<id>` deep link directly. `None` for OpenHands
    /// runs (their debug entry point is the workspace, not a Codex thread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_thread_id: Option<String>,
    pub workspace_path_suffix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_label: Option<String>,
    pub retry_count: u32,
    /// Preserve the scheduler's explicit release reason for consumers that
    /// need to distinguish a parked exhausted retry from a first-attempt
    /// terminal failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_reason: Option<ReleaseReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub turn_count: u32,
    #[serde(default)]
    pub max_turns: u32,
    #[serde(default)]
    pub runtime_seconds: u64,
    pub blocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy_blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_binding: Option<RepositoryBindingOutcome>,
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
    #[serde(default)]
    pub total_tokens: u64,
    /// True when the harness has been detached from the run (local operator or
    /// host requested disconnect without a clean terminal state).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub detached: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_requested: bool,
    /// True when the harness acknowledged a cancel/force-stop request.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_acknowledged: bool,
    /// True when a cancel/force-stop request was not acknowledged and the run
    /// ended in a cancel-failed state.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_failed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_timed_out: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
    /// Sanitized multi-repository and lifecycle facts. Missing means the
    /// authoritative runtime has not provided that fact; clients must not
    /// infer completion or containment from its absence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<ControlPlaneOperatorSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneOperatorSnapshot {
    pub routing_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_project_set: Vec<String>,
    pub linear_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_status: Option<String>,
    pub parent: Option<ControlPlaneParentSnapshot>,
    pub repository: Option<ControlPlaneRepositorySnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leases: Vec<ControlPlaneLeaseSnapshot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repairs: Vec<ControlPlaneRepairSnapshot>,
    pub memory: Option<ControlPlaneMemorySnapshot>,
    pub containment: Option<ControlPlaneContainmentSnapshot>,
    pub provider: Option<ControlPlaneProviderSnapshot>,
    pub verification: Option<ControlPlaneVerificationSnapshot>,
    pub cleanup: Option<ControlPlaneCleanupSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneRepositorySnapshot {
    pub canonical_id: String,
    pub display_alias: String,
    pub safe_remote_fingerprint: Option<String>,
    pub config_generation: Option<String>,
    pub inventory_generation: Option<String>,
    pub checkout_generation: Option<String>,
    pub target_branch: Option<String>,
    pub target_commit: Option<String>,
    pub instruction_source: Option<String>,
    pub instruction_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneParentSnapshot {
    pub parent_id: String,
    pub state: Option<String>,
    pub hierarchy_generation: Option<u64>,
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub descendant_repositories: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkout_handles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneLeaseSnapshot {
    pub owner_id: String,
    pub owner_kind: String,
    pub repository_id: String,
    pub checkout_generation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneRepairSnapshot {
    pub id: String,
    pub repository_id: Option<String>,
    pub status: String,
    pub pull_request_url: Option<String>,
    pub target_commit: Option<String>,
    pub instruction_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneMemorySnapshot {
    pub scope: String,
    pub source_freshness: Option<String>,
    pub degraded: bool,
    pub overlay_provenance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneContainmentSnapshot {
    pub requested_scope: Option<String>,
    pub effective_containment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneProviderSnapshot {
    pub provider: String,
    pub step: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneVerificationSnapshot {
    pub attempts: u32,
    pub status: String,
    pub final_evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlPlaneCleanupSnapshot {
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blockers: Vec<String>,
    pub retry_count: u32,
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
