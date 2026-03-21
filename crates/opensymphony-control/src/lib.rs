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
        if let Some(event) = snapshot_event(&initial) {
            yield Ok(event);
        }
        loop {
            match receiver.recv().await {
                Ok(envelope) => {
                    if let Some(event) = snapshot_event(&envelope) {
                        yield Ok(event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let envelope = store.current().await;
                    if let Some(event) = snapshot_event(&envelope) {
                        yield Ok(event);
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
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
}

impl ControlPlaneClient {
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    pub async fn fetch_snapshot(&self) -> Result<SnapshotEnvelope, ControlPlaneClientError> {
        let snapshot_url = self.join_path("api/v1/snapshot")?;
        let response = self.http.get(snapshot_url).send().await?;
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
