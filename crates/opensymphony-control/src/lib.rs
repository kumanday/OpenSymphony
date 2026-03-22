use std::{convert::Infallible, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use reqwest_eventsource::{Event as EventSourceEvent, EventSource, ReadyState};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{RwLock, broadcast},
};
use tracing::warn;
use url::Url;

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

#[derive(Debug, Clone)]
pub struct SnapshotStore {
    inner: Arc<StoreState>,
}

#[derive(Debug)]
struct StoreState {
    current: RwLock<SnapshotEnvelope>,
    updates: broadcast::Sender<SnapshotEnvelope>,
}

impl SnapshotStore {
    pub fn new(initial_snapshot: DaemonSnapshot) -> Self {
        let initial = SnapshotEnvelope {
            sequence: 1,
            published_at: Utc::now(),
            snapshot: initial_snapshot,
        };
        let (updates, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(StoreState {
                current: RwLock::new(initial),
                updates,
            }),
        }
    }

    pub async fn current(&self) -> SnapshotEnvelope {
        self.inner.current.read().await.clone()
    }

    pub async fn publish(&self, snapshot: DaemonSnapshot) -> SnapshotEnvelope {
        let mut current = self.inner.current.write().await;
        let next = SnapshotEnvelope {
            sequence: current.sequence + 1,
            published_at: Utc::now(),
            snapshot,
        };
        *current = next.clone();
        let _ = self.inner.updates.send(next.clone());
        next
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SnapshotEnvelope> {
        self.inner.updates.subscribe()
    }
}

#[derive(Debug, Clone)]
pub struct ControlPlaneServer {
    store: SnapshotStore,
}

impl ControlPlaneServer {
    pub fn new(store: SnapshotStore) -> Self {
        Self { store }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/healthz", get(health))
            .route("/api/v1/snapshot", get(snapshot))
            .route("/api/v1/events", get(events))
            .with_state(self.store.clone())
    }

    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        axum::serve(listener, self.router()).await
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthResponse {
    pub status: String,
    pub current_sequence: u64,
    pub published_at: DateTime<Utc>,
    pub issue_count: usize,
}

async fn health(State(store): State<SnapshotStore>) -> Json<HealthResponse> {
    let envelope = store.current().await;
    Json(HealthResponse {
        status: health_status(envelope.snapshot.daemon.state).to_owned(),
        current_sequence: envelope.sequence,
        published_at: envelope.published_at,
        issue_count: envelope.snapshot.issue_count(),
    })
}

fn health_status(state: DaemonState) -> &'static str {
    match state {
        DaemonState::Starting => "starting",
        DaemonState::Ready => "ok",
        DaemonState::Degraded => "degraded",
        DaemonState::Stopped => "stopped",
    }
}

async fn snapshot(State(store): State<SnapshotStore>) -> Json<SnapshotEnvelope> {
    Json(store.current().await)
}

async fn events(
    State(store): State<SnapshotStore>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = store.subscribe();
    let initial = store.current().await;
    let stream = stream! {
        let mut last_sequence = initial.sequence;
        if let Some(event) = snapshot_event(&initial) {
            yield Ok(event);
        }
        loop {
            let Some(envelope) =
                recv_monotonic_snapshot(&store, &mut receiver, &mut last_sequence).await
            else {
                break;
            };

            if let Some(event) = snapshot_event(&envelope) {
                yield Ok(event);
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

async fn recv_monotonic_snapshot(
    store: &SnapshotStore,
    receiver: &mut broadcast::Receiver<SnapshotEnvelope>,
    last_sequence: &mut u64,
) -> Option<SnapshotEnvelope> {
    loop {
        match receiver.recv().await {
            Ok(envelope) if envelope.sequence > *last_sequence => {
                *last_sequence = envelope.sequence;
                return Some(envelope);
            }
            Ok(_) => continue,
            Err(broadcast::error::RecvError::Lagged(_)) => {
                let envelope = store.current().await;
                if envelope.sequence > *last_sequence {
                    *last_sequence = envelope.sequence;
                    return Some(envelope);
                }
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

fn snapshot_event(envelope: &SnapshotEnvelope) -> Option<Event> {
    let payload = serde_json::to_string(envelope).ok()?;
    Some(
        Event::default()
            .event("snapshot")
            .id(envelope.sequence.to_string())
            .data(payload),
    )
}

#[derive(Debug, Clone)]
pub struct ControlPlaneClient {
    base_url: Url,
    http: reqwest::Client,
    snapshot_timeout: Duration,
    stream_connect_timeout: Duration,
}

impl ControlPlaneClient {
    const DEFAULT_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
    const DEFAULT_STREAM_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    const DEFAULT_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(45);

    pub fn new(base_url: Url) -> Self {
        Self::with_timeouts(
            base_url,
            Self::DEFAULT_SNAPSHOT_TIMEOUT,
            Self::DEFAULT_STREAM_CONNECT_TIMEOUT,
            Self::DEFAULT_STREAM_IDLE_TIMEOUT,
        )
    }

    pub fn with_snapshot_timeout(base_url: Url, snapshot_timeout: Duration) -> Self {
        Self::with_timeouts(
            base_url,
            snapshot_timeout,
            Self::DEFAULT_STREAM_CONNECT_TIMEOUT,
            Self::DEFAULT_STREAM_IDLE_TIMEOUT,
        )
    }

    pub fn with_timeouts(
        base_url: Url,
        snapshot_timeout: Duration,
        stream_connect_timeout: Duration,
        stream_idle_timeout: Duration,
    ) -> Self {
        Self {
            base_url,
            http: reqwest::Client::builder()
                .connect_timeout(stream_connect_timeout)
                .read_timeout(stream_idle_timeout)
                .build()
                .expect("control-plane reqwest client should build"),
            snapshot_timeout,
            stream_connect_timeout,
        }
    }

    pub async fn fetch_snapshot(&self) -> Result<SnapshotEnvelope, ControlPlaneClientError> {
        let snapshot_url = self.join_path("api/v1/snapshot")?;
        let response = self
            .http
            .get(snapshot_url)
            .timeout(self.snapshot_timeout)
            .send()
            .await?;
        Ok(response.error_for_status()?.json().await?)
    }

    pub fn stream_updates(&self) -> Result<ControlPlaneEventStream, ControlPlaneClientError> {
        let events_url = self.join_path("api/v1/events")?;
        let request = self.http.get(events_url);
        let inner = EventSource::new(request).map_err(ControlPlaneClientError::StreamRequest)?;
        Ok(ControlPlaneEventStream {
            inner,
            stream_connect_timeout: self.stream_connect_timeout,
            observed_open: false,
        })
    }

    fn join_path(&self, path: &'static str) -> Result<Url, ControlPlaneClientError> {
        self.base_url
            .join(path)
            .map_err(|source| ControlPlaneClientError::InvalidBaseUrl {
                base_url: self.base_url.to_string(),
                path,
                source,
            })
    }
}

pub struct ControlPlaneEventStream {
    inner: EventSource,
    stream_connect_timeout: Duration,
    observed_open: bool,
}

#[derive(Debug)]
pub enum ControlPlaneStreamUpdate {
    Snapshot(SnapshotEnvelope),
    Reconnecting(ControlPlaneClientError),
}

impl ControlPlaneEventStream {
    pub async fn next(
        &mut self,
    ) -> Option<Result<ControlPlaneStreamUpdate, ControlPlaneClientError>> {
        loop {
            let event = if self.observed_open {
                match self.inner.next().await {
                    Some(event) => event,
                    None => return None,
                }
            } else {
                match tokio::time::timeout(self.stream_connect_timeout, self.inner.next()).await {
                    Ok(Some(event)) => event,
                    Ok(None) => return None,
                    Err(_) => {
                        return Some(Err(ControlPlaneClientError::StreamConnectTimeout(
                            self.stream_connect_timeout,
                        )));
                    }
                }
            };
            match event {
                Ok(EventSourceEvent::Open) => {
                    self.observed_open = true;
                    continue;
                }
                Ok(EventSourceEvent::Message(message)) => {
                    self.observed_open = true;
                    return Some(
                        serde_json::from_str(&message.data)
                            .map(ControlPlaneStreamUpdate::Snapshot)
                            .map_err(ControlPlaneClientError::Decode),
                    );
                }
                Err(error) if self.inner.ready_state() == ReadyState::Connecting => {
                    self.observed_open = false;
                    return Some(Ok(ControlPlaneStreamUpdate::Reconnecting(
                        ControlPlaneClientError::Stream(Box::new(error)),
                    )));
                }
                Err(error) => {
                    self.observed_open = false;
                    return Some(Err(ControlPlaneClientError::Stream(Box::new(error))));
                }
            }
        }
    }

    pub fn close(&mut self) {
        self.inner.close();
    }
}

#[derive(Debug, Error)]
pub enum ControlPlaneClientError {
    #[error("failed to resolve control-plane path `{path}` against `{base_url}`: {source}")]
    InvalidBaseUrl {
        base_url: String,
        path: &'static str,
        #[source]
        source: url::ParseError,
    },
    #[error("control-plane HTTP request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("control-plane update stream failed: {0}")]
    Stream(#[source] Box<reqwest_eventsource::Error>),
    #[error("control-plane update stream did not establish within {0:?}")]
    StreamConnectTimeout(Duration),
    #[error("control-plane update request could not be cloned: {0}")]
    StreamRequest(reqwest_eventsource::CannotCloneRequestError),
    #[error("failed to decode control-plane payload: {0}")]
    Decode(#[from] serde_json::Error),
}

impl From<reqwest_eventsource::Error> for ControlPlaneClientError {
    fn from(error: reqwest_eventsource::Error) -> Self {
        Self::Stream(Box::new(error))
    }
}

pub fn log_stream_error(error: &ControlPlaneClientError) {
    warn!(error = %error, "control-plane stream error");
}

#[cfg(test)]
mod tests {
    use super::{
        AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState,
        IssueSnapshot, MetricsSnapshot, RecentEvent, RecentEventKind, WorkerOutcome,
    };
    use super::{SnapshotStore, recv_monotonic_snapshot};
    use chrono::{TimeZone, Utc};
    use std::time::Duration;

    fn fixture_snapshot(step: u64) -> DaemonSnapshot {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
            .single()
            .expect("fixture timestamp should be valid")
            + chrono::Duration::seconds(step as i64);
        DaemonSnapshot {
            generated_at: now,
            daemon: DaemonStatus {
                state: DaemonState::Ready,
                last_poll_at: now,
                workspace_root: "/tmp/opensymphony".to_owned(),
                status_line: "ready".to_owned(),
            },
            agent_server: AgentServerStatus {
                reachable: true,
                base_url: "http://127.0.0.1:3000".to_owned(),
                conversation_count: 1,
                status_line: "healthy".to_owned(),
            },
            metrics: MetricsSnapshot {
                running_issues: 1,
                retry_queue_depth: 0,
                total_tokens: 4096 + step,
                total_cost_micros: 120_000,
            },
            issues: vec![IssueSnapshot {
                identifier: "COE-271".to_owned(),
                title: "FrankenTUI operator client".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::Running,
                last_outcome: WorkerOutcome::Running,
                last_event_at: now,
                conversation_id_suffix: "c0e271".to_owned(),
                workspace_path_suffix: "COE-271".to_owned(),
                retry_count: 0,
                blocked: false,
            }],
            recent_events: vec![RecentEvent {
                happened_at: now,
                issue_identifier: Some("COE-271".to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: format!("published step {step}"),
            }],
        }
    }

    #[tokio::test]
    async fn lagged_receivers_resume_with_monotonic_sequences() {
        let store = SnapshotStore::new(fixture_snapshot(0));
        let mut receiver = store.subscribe();
        let mut last_sequence = 1;

        for step in 1..=80 {
            store.publish(fixture_snapshot(step)).await;
        }

        let latest = store.current().await;
        let recovered = tokio::time::timeout(
            Duration::from_secs(1),
            recv_monotonic_snapshot(&store, &mut receiver, &mut last_sequence),
        )
        .await
        .expect("lagged receiver should recover within timeout")
        .expect("lagged receiver should yield the latest snapshot");

        assert_eq!(recovered.sequence, latest.sequence);
        assert_eq!(last_sequence, latest.sequence);

        let next = store.publish(fixture_snapshot(81)).await;
        let resumed = tokio::time::timeout(
            Duration::from_secs(1),
            recv_monotonic_snapshot(&store, &mut receiver, &mut last_sequence),
        )
        .await
        .expect("receiver should resume within timeout")
        .expect("receiver should yield the next snapshot");

        assert_eq!(resumed.sequence, next.sequence);
        assert!(resumed.sequence > recovered.sequence);
    }
}
