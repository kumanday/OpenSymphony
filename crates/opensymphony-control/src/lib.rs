use std::{convert::Infallible, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use opensymphony_domain::{DaemonSnapshot, SnapshotEnvelope};
use reqwest_eventsource::{Event as EventSourceEvent, EventSource};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{broadcast, RwLock},
};
use tracing::warn;
use url::Url;

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
        status: "ok".to_owned(),
        current_sequence: envelope.sequence,
        published_at: envelope.published_at,
        issue_count: envelope.snapshot.issue_count(),
    })
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
}

impl ControlPlaneClient {
    const DEFAULT_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

    pub fn new(base_url: Url) -> Self {
        Self::with_snapshot_timeout(base_url, Self::DEFAULT_SNAPSHOT_TIMEOUT)
    }

    pub fn with_snapshot_timeout(base_url: Url, snapshot_timeout: Duration) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
            snapshot_timeout,
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
        Ok(ControlPlaneEventStream { inner })
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
}

impl ControlPlaneEventStream {
    pub async fn next(&mut self) -> Option<Result<SnapshotEnvelope, ControlPlaneClientError>> {
        while let Some(event) = self.inner.next().await {
            match event {
                Ok(EventSourceEvent::Open) => continue,
                Ok(EventSourceEvent::Message(message)) => {
                    return Some(
                        serde_json::from_str(&message.data)
                            .map_err(ControlPlaneClientError::Decode),
                    );
                }
                Err(error) => return Some(Err(ControlPlaneClientError::Stream(Box::new(error)))),
            }
        }
        None
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
    use super::{recv_monotonic_snapshot, SnapshotStore};
    use chrono::{TimeZone, Utc};
    use opensymphony_domain::{
        AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState,
        IssueSnapshot, MetricsSnapshot, RecentEvent, RecentEventKind, WorkerOutcome,
    };
    use std::time::Duration;

    fn fixture_snapshot(step: u64) -> DaemonSnapshot {
        let now = Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap()
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
        .unwrap()
        .unwrap();

        assert_eq!(recovered.sequence, latest.sequence);
        assert_eq!(last_sequence, latest.sequence);

        let next = store.publish(fixture_snapshot(81)).await;
        let resumed = tokio::time::timeout(
            Duration::from_secs(1),
            recv_monotonic_snapshot(&store, &mut receiver, &mut last_sequence),
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(resumed.sequence, next.sequence);
        assert!(resumed.sequence > recovered.sequence);
    }
}
