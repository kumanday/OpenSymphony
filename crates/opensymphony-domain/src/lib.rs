use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEnvelope {
    pub sequence: u64,
    pub published_at: DateTime<Utc>,
    pub snapshot: DaemonSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSnapshot {
    pub generated_at: DateTime<Utc>,
    pub daemon: DaemonStatus,
    pub agent_server: AgentServerStatus,
    pub metrics: MetricsSnapshot,
    pub issues: Vec<IssueSnapshot>,
    pub recent_events: Vec<RecentEvent>,
}

impl DaemonSnapshot {
    pub fn issue_count(&self) -> usize {
        self.issues.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonStatus {
    pub state: DaemonState,
    pub last_poll_at: DateTime<Utc>,
    pub workspace_root: String,
    pub status_line: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    Starting,
    Ready,
    Degraded,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentServerStatus {
    pub reachable: bool,
    pub base_url: String,
    pub conversation_count: u32,
    pub status_line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricsSnapshot {
    pub running_issues: u32,
    pub retry_queue_depth: u32,
    pub total_tokens: u64,
    pub total_cost_micros: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueSnapshot {
    pub identifier: String,
    pub title: String,
    pub tracker_state: String,
    pub runtime_state: IssueRuntimeState,
    pub last_outcome: WorkerOutcome,
    pub last_event_at: DateTime<Utc>,
    pub conversation_id_suffix: String,
    pub workspace_path_suffix: String,
    pub retry_count: u32,
    pub blocked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueRuntimeState {
    Idle,
    Running,
    RetryQueued,
    Releasing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutcome {
    Unknown,
    Running,
    Continued,
    Completed,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentEvent {
    pub happened_at: DateTime<Utc>,
    pub issue_identifier: Option<String>,
    pub kind: RecentEventKind,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecentEventKind {
    WorkerStarted,
    WorkspacePrepared,
    StreamAttached,
    SnapshotPublished,
    WorkerCompleted,
    RetryScheduled,
    ClientAttached,
    ClientDetached,
    Warning,
    Other(String),
}

impl RecentEventKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::WorkerStarted => "worker_started",
            Self::WorkspacePrepared => "workspace_prepared",
            Self::StreamAttached => "stream_attached",
            Self::SnapshotPublished => "snapshot_published",
            Self::WorkerCompleted => "worker_completed",
            Self::RetryScheduled => "retry_scheduled",
            Self::ClientAttached => "client_attached",
            Self::ClientDetached => "client_detached",
            Self::Warning => "warning",
            Self::Other(value) => value.as_str(),
        }
    }

    fn from_wire(value: &str) -> Self {
        match value {
            "worker_started" => Self::WorkerStarted,
            "workspace_prepared" => Self::WorkspacePrepared,
            "stream_attached" => Self::StreamAttached,
            "snapshot_published" => Self::SnapshotPublished,
            "worker_completed" => Self::WorkerCompleted,
            "retry_scheduled" => Self::RetryScheduled,
            "client_attached" => Self::ClientAttached,
            "client_detached" => Self::ClientDetached,
            "warning" => Self::Warning,
            other => Self::Other(other.to_owned()),
        }
    }
}

impl Serialize for RecentEventKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RecentEventKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::from_wire(&value))
    }
}
