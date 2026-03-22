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
use opensymphony_domain::SnapshotEnvelope;
use reqwest_eventsource::{Event as EventSourceEvent, EventSource};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    net::TcpListener,
    sync::{broadcast, RwLock},
    time::timeout,
};
use tracing::warn;
use url::Url;

pub use opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as PublicDaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome,
};

pub type DaemonSnapshot = PublicDaemonSnapshot;

const CONTROL_PLANE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const CONTROL_PLANE_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_PLANE_STREAM_ATTACH_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_PLANE_STREAM_READ_TIMEOUT: Duration = Duration::from_secs(35);

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
        let mut last_sent_sequence = initial.sequence;
        if let Some(event) = snapshot_event(&initial) {
            yield Ok(event);
        }
        while let Some(envelope) =
            next_snapshot_envelope(&store, &mut receiver, &mut last_sent_sequence).await
        {
            if let Some(event) = snapshot_event(&envelope) {
                yield Ok(event);
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(CONTROL_PLANE_KEEPALIVE_INTERVAL)
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

async fn next_snapshot_envelope(
    store: &SnapshotStore,
    receiver: &mut broadcast::Receiver<SnapshotEnvelope>,
    last_sent_sequence: &mut u64,
) -> Option<SnapshotEnvelope> {
    loop {
        match receiver.recv().await {
            Ok(envelope) => {
                if envelope.sequence <= *last_sent_sequence {
                    continue;
                }
                *last_sent_sequence = envelope.sequence;
                return Some(envelope);
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if let Some(envelope) =
                    catch_up_lagged_receiver(store, receiver, *last_sent_sequence).await
                {
                    *last_sent_sequence = envelope.sequence;
                    return Some(envelope);
                }
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

async fn catch_up_lagged_receiver(
    store: &SnapshotStore,
    _receiver: &mut broadcast::Receiver<SnapshotEnvelope>,
    last_sent_sequence: u64,
) -> Option<SnapshotEnvelope> {
    // Fast-forward lagged subscribers straight to the latest published snapshot instead of
    // draining the retained broadcast backlog. Under sustained publication that backlog may
    // never go empty, which would otherwise stall SSE delivery indefinitely.
    let latest = store.current().await;
    (latest.sequence > last_sent_sequence).then_some(latest)
}

#[derive(Debug, Clone)]
pub struct ControlPlaneClient {
    base_url: Url,
    http: reqwest::Client,
    stream_http: reqwest::Client,
    stream_attach_timeout: Duration,
}

impl ControlPlaneClient {
    pub fn new(base_url: Url) -> Self {
        Self::with_timeouts(
            base_url,
            CONTROL_PLANE_SNAPSHOT_TIMEOUT,
            CONTROL_PLANE_STREAM_ATTACH_TIMEOUT,
            CONTROL_PLANE_STREAM_READ_TIMEOUT,
        )
    }

    fn with_timeouts(
        base_url: Url,
        snapshot_timeout: Duration,
        stream_attach_timeout: Duration,
        stream_read_timeout: Duration,
    ) -> Self {
        Self {
            base_url,
            http: build_snapshot_http_client(snapshot_timeout),
            stream_http: build_stream_http_client(stream_attach_timeout, stream_read_timeout),
            stream_attach_timeout,
        }
    }

    pub async fn fetch_snapshot(&self) -> Result<SnapshotEnvelope, ControlPlaneClientError> {
        let snapshot_url = self.join_path("api/v1/snapshot")?;
        let response = self.http.get(snapshot_url).send().await?;
        Ok(response.error_for_status()?.json().await?)
    }

    pub fn stream_updates(&self) -> Result<ControlPlaneEventStream, ControlPlaneClientError> {
        let events_url = self.join_path("api/v1/events")?;
        let request = self.stream_http.get(events_url);
        let inner = EventSource::new(request).map_err(ControlPlaneClientError::StreamRequest)?;
        Ok(ControlPlaneEventStream {
            inner,
            attach_timeout: self.stream_attach_timeout,
            awaiting_first_snapshot: true,
        })
    }

    fn join_path(&self, path: &'static str) -> Result<Url, ControlPlaneClientError> {
        normalized_base_url(&self.base_url)
            .join(path)
            .map_err(|source| ControlPlaneClientError::InvalidBaseUrl {
                base_url: self.base_url.to_string(),
                path,
                source,
            })
    }
}

fn build_snapshot_http_client(snapshot_timeout: Duration) -> reqwest::Client {
    // Bootstrap and reconnect fetches should fail fast so the UI can retry instead of hanging
    // forever behind a partial `/api/v1/snapshot` response.
    reqwest::Client::builder()
        .timeout(snapshot_timeout)
        .read_timeout(snapshot_timeout)
        .build()
        .expect("static snapshot timeout config should produce a reqwest client")
}

fn build_stream_http_client(
    stream_attach_timeout: Duration,
    stream_read_timeout: Duration,
) -> reqwest::Client {
    // Reuse the attach watchdog budget for the underlying TCP connect so both the socket setup
    // and the first-snapshot wait fail fast before the longer steady-state idle timeout applies.
    reqwest::Client::builder()
        .connect_timeout(stream_attach_timeout)
        .read_timeout(stream_read_timeout)
        .build()
        .expect("static stream read timeout config should produce a reqwest client")
}

fn normalized_base_url(base_url: &Url) -> Url {
    let mut normalized = base_url.clone();
    let path = normalized.path();
    if path.is_empty() || path.ends_with('/') {
        return normalized;
    }

    normalized.set_path(&format!("{path}/"));
    normalized
}

pub struct ControlPlaneEventStream {
    inner: EventSource,
    attach_timeout: Duration,
    awaiting_first_snapshot: bool,
}

impl ControlPlaneEventStream {
    pub async fn next(&mut self) -> Option<Result<SnapshotEnvelope, ControlPlaneClientError>> {
        while let Some(event) = self.next_event().await {
            match event {
                Ok(EventSourceEvent::Open) => continue,
                Ok(EventSourceEvent::Message(message)) => {
                    self.awaiting_first_snapshot = false;
                    return Some(
                        serde_json::from_str(&message.data)
                            .map_err(ControlPlaneClientError::Decode),
                    );
                }
                Err(error) => return Some(Err(error)),
            }
        }
        None
    }

    async fn next_event(&mut self) -> Option<Result<EventSourceEvent, ControlPlaneClientError>> {
        let next = if self.awaiting_first_snapshot {
            match timeout(self.attach_timeout, self.inner.next()).await {
                Ok(next) => next,
                Err(_) => {
                    self.inner.close();
                    return Some(Err(ControlPlaneClientError::StreamAttachTimeout(
                        self.attach_timeout,
                    )));
                }
            }
        } else {
            self.inner.next().await
        };

        next.map(|event| event.map_err(ControlPlaneClientError::from))
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
    #[error("control-plane update stream did not attach within {0:?}")]
    StreamAttachTimeout(Duration),
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
    use chrono::{TimeZone, Utc};
    use opensymphony_domain::{
        ControlPlaneAgentServerStatus as AgentServerStatus,
        ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
        ControlPlaneDaemonStatus as DaemonStatus,
        ControlPlaneIssueRuntimeState as IssueRuntimeState,
        ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
        ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
        ControlPlaneWorkerOutcome as WorkerOutcome,
    };
    use std::time::Duration;
    use tokio::{
        io::AsyncWriteExt,
        net::TcpListener,
        sync::broadcast,
        time::{sleep, timeout},
    };
    use url::Url;

    use super::{
        catch_up_lagged_receiver, next_snapshot_envelope, ControlPlaneClient,
        ControlPlaneClientError, SnapshotStore,
    };

    fn fixture_snapshot(step: u64) -> DaemonSnapshot {
        let now = Utc
            .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
            .single()
            .expect("valid fixed test timestamp")
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
                conversation_count: 2,
                status_line: "healthy".to_owned(),
            },
            metrics: MetricsSnapshot {
                running_issues: 1,
                retry_queue_depth: 0,
                total_tokens: 4096 + step,
                total_cost_micros: 120_000,
            },
            issues: vec![IssueSnapshot {
                identifier: "COE-255".to_owned(),
                title: "Observability and FrankenTUI".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::Running,
                last_outcome: WorkerOutcome::Running,
                last_event_at: now,
                conversation_id_suffix: "c0e255".to_owned(),
                workspace_path_suffix: "COE-255".to_owned(),
                retry_count: 0,
                blocked: false,
            }],
            recent_events: vec![RecentEvent {
                happened_at: now,
                issue_identifier: Some("COE-255".to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: format!("published step {step}"),
            }],
        }
    }

    #[tokio::test]
    async fn lagged_receivers_resume_from_the_latest_snapshot_without_regressing() {
        let store = SnapshotStore::new(fixture_snapshot(0));
        let mut receiver = store.subscribe();
        let mut last_sent_sequence = store.current().await.sequence;

        for step in 1..=80 {
            store.publish(fixture_snapshot(step)).await;
        }

        let latest = next_snapshot_envelope(&store, &mut receiver, &mut last_sent_sequence)
            .await
            .expect("latest snapshot after lag");
        assert_eq!(latest.sequence, 81);

        let expected = store.publish(fixture_snapshot(81)).await;
        let resumed = timeout(
            Duration::from_secs(1),
            next_snapshot_envelope(&store, &mut receiver, &mut last_sent_sequence),
        )
        .await
        .expect("resumed snapshot")
        .expect("open stream");

        assert_eq!(resumed.sequence, expected.sequence);
        assert_eq!(
            resumed.snapshot.recent_events[0].summary,
            "published step 81"
        );
    }

    #[tokio::test]
    async fn lagged_catch_up_returns_before_the_broadcast_backlog_drains() {
        let store = SnapshotStore::new(fixture_snapshot(0));
        let mut receiver = store.subscribe();
        let last_sent_sequence = store.current().await.sequence;

        for step in 1..=80 {
            store.publish(fixture_snapshot(step)).await;
        }

        let latest = catch_up_lagged_receiver(&store, &mut receiver, last_sent_sequence)
            .await
            .expect("latest snapshot after lag");

        assert_eq!(latest.sequence, 81);

        match receiver.try_recv() {
            Ok(buffered) => assert!(buffered.sequence < latest.sequence),
            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(other) => {
                panic!("expected catch-up to return before draining the backlog, got {other:?}")
            }
        }
    }

    #[test]
    fn control_plane_client_preserves_path_prefixes_without_trailing_slashes() {
        let client = ControlPlaneClient::new(
            Url::parse("http://proxy/opensymphony").expect("valid prefixed control-plane base url"),
        );
        let snapshot_url = client
            .join_path("api/v1/snapshot")
            .expect("snapshot path should resolve beneath the prefix");
        let events_url = client
            .join_path("api/v1/events")
            .expect("events path should resolve beneath the prefix");

        assert_eq!(
            snapshot_url.as_str(),
            "http://proxy/opensymphony/api/v1/snapshot"
        );
        assert_eq!(
            events_url.as_str(),
            "http://proxy/opensymphony/api/v1/events"
        );
    }

    async fn write_sse_headers(socket: &mut tokio::net::TcpStream) {
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: keep-alive\r\n\r\n",
            )
            .await
            .expect("mock SSE server should write headers");
    }

    fn fixture_snapshot_event(sequence: u64) -> String {
        let snapshot = fixture_snapshot(sequence);
        let envelope = super::SnapshotEnvelope {
            sequence,
            published_at: snapshot.generated_at,
            snapshot,
        };
        let payload = serde_json::to_string(&envelope).expect("serialize snapshot");
        format!(
            "event: snapshot\nid: {}\ndata: {payload}\n\n",
            envelope.sequence
        )
    }

    #[tokio::test]
    async fn fetch_snapshot_times_out_when_response_body_never_arrives() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock snapshot listener");
        let base_url = Url::parse(&format!(
            "http://{}/",
            listener.local_addr().expect("mock listener address")
        ))
        .expect("valid control-plane base url");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept snapshot client");
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: keep-alive\r\n\r\n",
                )
                .await
                .expect("mock snapshot server should write headers");
            sleep(Duration::from_millis(250)).await;
        });

        let client = ControlPlaneClient::with_timeouts(
            base_url,
            Duration::from_millis(75),
            Duration::from_secs(1),
            Duration::from_secs(1),
        );
        let result = timeout(Duration::from_secs(1), client.fetch_snapshot())
            .await
            .expect("fetch should surface the snapshot timeout")
            .expect_err("snapshot fetch should time out");

        match result {
            ControlPlaneClientError::Request(error) => assert!(error.is_timeout()),
            other => panic!("expected a request timeout, got {other:?}"),
        }

        server
            .await
            .expect("mock snapshot server should exit cleanly");
    }

    #[tokio::test]
    async fn stream_updates_time_out_when_the_event_stream_goes_idle_after_the_first_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock SSE listener");
        let base_url = Url::parse(&format!(
            "http://{}/",
            listener.local_addr().expect("mock listener address")
        ))
        .expect("valid control-plane base url");
        let first_snapshot = fixture_snapshot_event(6);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept SSE client");
            write_sse_headers(&mut socket).await;
            socket
                .write_all(first_snapshot.as_bytes())
                .await
                .expect("mock SSE server should write initial snapshot");
            sleep(Duration::from_millis(250)).await;
        });

        let client = ControlPlaneClient::with_timeouts(
            base_url,
            super::CONTROL_PLANE_SNAPSHOT_TIMEOUT,
            Duration::from_millis(75),
            Duration::from_millis(75),
        );
        let mut stream = client.stream_updates().expect("open event stream");
        let first = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream should yield the first attached snapshot")
            .expect("stream should yield a result")
            .expect("stream should decode the first snapshot");
        assert_eq!(first.sequence, 6);
        let result = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream should surface the idle timeout")
            .expect("stream should report an error after going idle");

        match result {
            Err(ControlPlaneClientError::Stream(error)) => match error.as_ref() {
                reqwest_eventsource::Error::Transport(reqwest_error) => {
                    assert!(reqwest_error.is_timeout());
                }
                other => panic!("expected a transport timeout, got {other:?}"),
            },
            other => panic!("expected a stream timeout error, got {other:?}"),
        }

        stream.close();
        server.await.expect("mock SSE server should exit cleanly");
    }

    #[tokio::test]
    async fn stream_updates_stay_alive_when_keepalive_comments_arrive_before_a_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock SSE listener");
        let base_url = Url::parse(&format!(
            "http://{}/",
            listener.local_addr().expect("mock listener address")
        ))
        .expect("valid control-plane base url");
        let expected_snapshot = {
            let snapshot = fixture_snapshot(5);
            super::SnapshotEnvelope {
                sequence: 6,
                published_at: snapshot.generated_at,
                snapshot,
            }
        };
        let payload = serde_json::to_string(&expected_snapshot).expect("serialize snapshot");
        let event = format!(
            "event: snapshot\nid: {}\ndata: {payload}\n\n",
            expected_snapshot.sequence
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept SSE client");
            write_sse_headers(&mut socket).await;
            for _ in 0..3 {
                sleep(Duration::from_millis(40)).await;
                socket
                    .write_all(b": keepalive\n\n")
                    .await
                    .expect("mock SSE server should write keepalive");
            }
            sleep(Duration::from_millis(40)).await;
            socket
                .write_all(event.as_bytes())
                .await
                .expect("mock SSE server should write snapshot event");
        });

        let client = ControlPlaneClient::with_timeouts(
            base_url,
            super::CONTROL_PLANE_SNAPSHOT_TIMEOUT,
            Duration::from_secs(1),
            Duration::from_millis(75),
        );
        let mut stream = client.stream_updates().expect("open event stream");
        let result = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream should stay alive through keepalives")
            .expect("stream should yield the next snapshot")
            .expect("snapshot event should decode");

        assert_eq!(result, expected_snapshot);

        stream.close();
        server.await.expect("mock SSE server should exit cleanly");
    }

    #[tokio::test]
    async fn stream_updates_time_out_when_only_keepalive_comments_arrive_before_a_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock SSE listener");
        let base_url = Url::parse(&format!(
            "http://{}/",
            listener.local_addr().expect("mock listener address")
        ))
        .expect("valid control-plane base url");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept SSE client");
            write_sse_headers(&mut socket).await;
            for _ in 0..8 {
                sleep(Duration::from_millis(20)).await;
                if socket.write_all(b": keepalive\n\n").await.is_err() {
                    break;
                }
            }
        });

        let attach_timeout = Duration::from_millis(75);
        let client = ControlPlaneClient::with_timeouts(
            base_url,
            super::CONTROL_PLANE_SNAPSHOT_TIMEOUT,
            attach_timeout,
            Duration::from_secs(1),
        );
        let mut stream = client.stream_updates().expect("open event stream");
        let result = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream should surface the attach timeout")
            .expect("stream should report an error after timing out")
            .expect_err("stream attach should time out");

        match result {
            ControlPlaneClientError::StreamAttachTimeout(timeout) => {
                assert_eq!(timeout, attach_timeout);
            }
            other => panic!("expected a stream attach timeout, got {other:?}"),
        }

        stream.close();
        server.await.expect("mock SSE server should exit cleanly");
    }

    #[tokio::test]
    async fn stream_updates_time_out_when_the_first_snapshot_never_arrives_after_open() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock SSE listener");
        let base_url = Url::parse(&format!(
            "http://{}/",
            listener.local_addr().expect("mock listener address")
        ))
        .expect("valid control-plane base url");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept SSE client");
            write_sse_headers(&mut socket).await;
            sleep(Duration::from_millis(250)).await;
        });

        let attach_timeout = Duration::from_millis(75);
        let client = ControlPlaneClient::with_timeouts(
            base_url,
            super::CONTROL_PLANE_SNAPSHOT_TIMEOUT,
            attach_timeout,
            Duration::from_secs(1),
        );
        let mut stream = client.stream_updates().expect("open event stream");
        let result = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream should surface the attach timeout")
            .expect("stream should report an error after timing out")
            .expect_err("stream attach should time out");

        match result {
            ControlPlaneClientError::StreamAttachTimeout(timeout) => {
                assert_eq!(timeout, attach_timeout);
            }
            other => panic!("expected a stream attach timeout, got {other:?}"),
        }

        stream.close();
        server.await.expect("mock SSE server should exit cleanly");
    }

    #[tokio::test]
    async fn stream_updates_time_out_when_the_event_stream_never_opens() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock SSE listener");
        let base_url = Url::parse(&format!(
            "http://{}/",
            listener.local_addr().expect("mock listener address")
        ))
        .expect("valid control-plane base url");
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.expect("accept SSE client");
            sleep(Duration::from_millis(250)).await;
        });

        let attach_timeout = Duration::from_millis(75);
        let client = ControlPlaneClient::with_timeouts(
            base_url,
            super::CONTROL_PLANE_SNAPSHOT_TIMEOUT,
            attach_timeout,
            Duration::from_secs(1),
        );
        let mut stream = client.stream_updates().expect("open event stream");
        let result = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("stream should surface the open timeout")
            .expect("stream should report an error after timing out")
            .expect_err("stream open should time out");

        match result {
            ControlPlaneClientError::StreamAttachTimeout(timeout) => {
                assert_eq!(timeout, attach_timeout);
            }
            other => panic!("expected a stream attach timeout, got {other:?}"),
        }

        stream.close();
        server.await.expect("mock SSE server should exit cleanly");
    }
}
