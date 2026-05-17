use std::{convert::Infallible, sync::Arc, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use tokio::{net::TcpListener, sync::broadcast};

use crate::opensymphony_domain::{EventJournalBackend, InMemoryEventJournal, StreamBroker};
use crate::opensymphony_gateway_schema::{
    cursor::StreamCursor,
    event_journal::{EventPage, JournalError},
};

pub use crate::opensymphony_control::SnapshotStore;
pub use crate::opensymphony_domain::{
    ControlPlaneAgentServerStatus, ControlPlaneDaemonSnapshot, ControlPlaneDaemonState,
    ControlPlaneDaemonStatus, ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot,
    ControlPlaneMetricsSnapshot, ControlPlaneRecentEvent, ControlPlaneRecentEventKind,
    ControlPlaneWorkerOutcome, InMemoryEventJournal as DomainInMemoryEventJournal,
    SnapshotEnvelope, StreamBroker as DomainStreamBroker,
};
pub use crate::opensymphony_gateway_schema::{
    capability::{AuthMode, FeatureCapability, GatewayCapabilities, TransportCapability},
    event_journal::{EventPage as GatewayEventPage, JournalError as EventJournalError},
    snapshot::{
        DashboardSnapshot, GatewayHealth, GatewayMetrics, ProjectSummary, SnapshotEventKind,
        SnapshotEventSummary,
    },
    version::{GATEWAY_SCHEMA_VERSION, SchemaVersion},
};

const GATEWAY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const GATEWAY_JOURNAL_CAPACITY: usize = 10_000;
const GATEWAY_SUBSCRIBER_CAPACITY: usize = 256;
const GATEWAY_EVENT_PAGE_LIMIT: usize = 100;

/// Shared state for the gateway server.
#[derive(Debug, Clone)]
pub struct GatewayState {
    pub store: SnapshotStore,
    pub journal: InMemoryEventJournal,
    pub broker: StreamBroker,
}

/// V1 gateway server that exposes stable public DTO endpoints
/// on top of the internal control-plane `SnapshotStore`.
#[derive(Debug, Clone)]
pub struct GatewayServer {
    store: SnapshotStore,
    journal: InMemoryEventJournal,
    broker: StreamBroker,
}

impl GatewayServer {
    /// Create a new gateway server with the default journal capacity.
    pub fn new(store: SnapshotStore) -> Self {
        let journal =
            InMemoryEventJournal::new(GATEWAY_JOURNAL_CAPACITY, GATEWAY_SUBSCRIBER_CAPACITY);
        Self {
            journal: journal.clone(),
            broker: StreamBroker::new(journal),
            store,
        }
    }

    /// Create a gateway server with a pre-configured journal and broker.
    pub fn with_journal(
        store: SnapshotStore,
        journal: InMemoryEventJournal,
        broker: StreamBroker,
    ) -> Self {
        Self {
            store,
            journal,
            broker,
        }
    }

    /// Extract the journal and broker so the caller can keep clones for testing.
    pub fn journal_and_broker(self) -> (InMemoryEventJournal, StreamBroker) {
        (self.journal, self.broker)
    }

    pub fn router(&self) -> Router {
        let state = GatewayState {
            store: self.store.clone(),
            journal: self.journal.clone(),
            broker: self.broker.clone(),
        };
        Router::new()
            .route("/api/v1/capabilities", get(capabilities))
            .route("/api/v1/dashboard/snapshot", get(dashboard_snapshot))
            .route("/api/v1/events", get(events_sse))
            .route("/api/v1/event-journal", get(event_journal_query))
            .route("/api/v1/streams/events", get(event_stream_ws))
            .with_state(state)
    }

    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        axum::serve(listener, self.router()).await
    }
}

/// Map internal control-plane state into the public dashboard snapshot DTO.
pub fn control_plane_to_dashboard_snapshot(envelope: &SnapshotEnvelope) -> DashboardSnapshot {
    let snapshot = &envelope.snapshot;
    let health = daemon_state_to_gateway_health(snapshot.daemon.state);
    let metrics = GatewayMetrics {
        running_issue_count: snapshot.metrics.running_issues,
        retry_queue_depth: snapshot.metrics.retry_queue_depth,
        total_input_tokens: snapshot.metrics.input_tokens,
        total_output_tokens: snapshot.metrics.output_tokens,
        total_cache_read_tokens: snapshot.metrics.cache_read_tokens,
        total_cost_micros: snapshot.metrics.total_cost_micros,
    };

    // For v1 we flatten all issues into a single synthetic project because the
    // control-plane does not yet expose per-project grouping.
    let projects = if snapshot.issues.is_empty() {
        Vec::new()
    } else {
        let running = snapshot
            .issues
            .iter()
            .filter(|i| matches!(i.runtime_state, ControlPlaneIssueRuntimeState::Running))
            .count() as u32;
        let completed = snapshot
            .issues
            .iter()
            .filter(|i| matches!(i.last_outcome, ControlPlaneWorkerOutcome::Completed))
            .count() as u32;
        let failed = snapshot
            .issues
            .iter()
            .filter(|i| matches!(i.last_outcome, ControlPlaneWorkerOutcome::Failed))
            .count() as u32;

        vec![ProjectSummary {
            project_id: "default".into(),
            name: "OpenSymphony".into(),
            milestone_count: 0,
            issue_count: snapshot.issues.len() as u32,
            running_count: running,
            completed_count: completed,
            failed_count: failed,
        }]
    };

    let recent_events = snapshot
        .recent_events
        .iter()
        .map(|e| SnapshotEventSummary {
            happened_at: e.happened_at,
            issue_identifier: e.issue_identifier.clone(),
            kind: recent_event_kind_to_snapshot_event_kind(&e.kind),
            summary: e.summary.clone(),
        })
        .collect();

    DashboardSnapshot {
        schema_version: SchemaVersion::v1(),
        generated_at: snapshot.generated_at,
        sequence: envelope.sequence,
        health,
        metrics,
        projects,
        recent_events,
    }
}

fn daemon_state_to_gateway_health(state: ControlPlaneDaemonState) -> GatewayHealth {
    match state {
        ControlPlaneDaemonState::Ready => GatewayHealth::Healthy,
        ControlPlaneDaemonState::Degraded => GatewayHealth::Degraded,
        ControlPlaneDaemonState::Starting => GatewayHealth::Starting,
        ControlPlaneDaemonState::Stopped => GatewayHealth::Failed,
    }
}

fn recent_event_kind_to_snapshot_event_kind(
    kind: &ControlPlaneRecentEventKind,
) -> SnapshotEventKind {
    match kind {
        ControlPlaneRecentEventKind::WorkerStarted => SnapshotEventKind::WorkerStarted,
        ControlPlaneRecentEventKind::WorkspacePrepared => SnapshotEventKind::WorkspacePrepared,
        ControlPlaneRecentEventKind::StreamAttached => SnapshotEventKind::StreamAttached,
        ControlPlaneRecentEventKind::SnapshotPublished => SnapshotEventKind::SnapshotPublished,
        ControlPlaneRecentEventKind::WorkerCompleted => SnapshotEventKind::WorkerCompleted,
        ControlPlaneRecentEventKind::RetryScheduled => SnapshotEventKind::RetryScheduled,
        ControlPlaneRecentEventKind::ClientAttached => SnapshotEventKind::ClientAttached,
        ControlPlaneRecentEventKind::ClientDetached => SnapshotEventKind::ClientDetached,
        ControlPlaneRecentEventKind::Warning => SnapshotEventKind::Warning,
    }
}

fn build_capabilities() -> GatewayCapabilities {
    GatewayCapabilities {
        schema_version: SchemaVersion::v1(),
        gateway_version: env!("CARGO_PKG_VERSION").into(),
        supported_api_versions: vec!["1.0.0".into()],
        transports: vec![
            TransportCapability {
                transport: "sse".into(),
                modes: vec!["snapshot".into()],
                supported_encodings: vec!["utf-8".into(), "base64".into()],
                bidirectional: false,
            },
            TransportCapability {
                transport: "websocket".into(),
                modes: vec!["json".into(), "binary".into()],
                supported_encodings: vec!["utf-8".into(), "base64".into()],
                bidirectional: true,
            },
            TransportCapability {
                transport: "http".into(),
                modes: vec!["rest".into()],
                supported_encodings: vec!["utf-8".into()],
                bidirectional: false,
            },
        ],
        features: vec![
            FeatureCapability {
                feature: "task_graph".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "run_detail".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "event_journal".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "terminal_stream".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "action_dispatch".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "planning".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "approval".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "rehydrate".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "linear_sync".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "openhands_harness".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "codex_harness".into(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "hosted_mode".into(),
                available: false,
                requires_auth: true,
                requires_plan: None,
            },
        ],
        auth_modes: vec![AuthMode::None, AuthMode::ApiKey],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    }
}

async fn capabilities() -> Json<GatewayCapabilities> {
    Json(build_capabilities())
}

async fn dashboard_snapshot(State(state): State<GatewayState>) -> Json<DashboardSnapshot> {
    let envelope = state.store.current().await;
    Json(control_plane_to_dashboard_snapshot(&envelope))
}

/// SSE journal event stream: `GET /api/v1/events`
///
/// Streams committed journal events as Server-Sent Events. Unlike the old
/// snapshot-based stream, this endpoint delivers individual journal events
/// with stable IDs, monotonic sequence numbers, and typed payloads.
async fn events_sse(
    State(state): State<GatewayState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let journal = state.journal.clone();
    let stream = stream! {
        let latest_cursor = journal.latest_cursor().await;
        let mut receiver = journal.subscribe();
        let mut last_sequence = latest_cursor.sequence;
        let partition = "events".to_string();
        loop {
            match receiver.recv().await {
                Ok(Ok(event)) => {
                    if event.sequence <= last_sequence {
                        continue;
                    }
                    // Skip events from other partitions to avoid delivering
                    // unrelated data and to prevent advancing last_sequence past
                    // target-partition events.
                    if event.kind.default_partition() != partition {
                        continue;
                    }
                    last_sequence = event.sequence;
                    match serde_json::to_string(&event) {
                        Ok(json) => {
                            yield Ok(Event::default().event("event").data(json));
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                sequence = event.sequence,
                                "Failed to serialize SSE journal event"
                            );
                            // Notify the client so they know data was lost rather
                            // than silently swallowing the event.
                            yield Ok(Event::default()
                                .event("error")
                                .data(
                                    r#"{"error_type":"serialization","message":"Failed to serialize journal event","recoverable":true}"#
                                ));
                        }
                    }
                }
                Ok(Err(ref err)) => {
                    // Forward journal stream errors to the client using the
                    // structured StreamError serialization.
                    match serde_json::to_string(err) {
                        Ok(json) => {
                            yield Ok(Event::default().event("error").data(json));
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to serialize SSE error event"
                            );
                        }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Lag recovery: replay missed events from the journal backlog.
                    match journal.query_after(&StreamCursor::new(last_sequence, &partition), GATEWAY_EVENT_PAGE_LIMIT).await {
                        Ok(page) => {
                            for event in &page.events {
                                if event.sequence > last_sequence {
                                    last_sequence = event.sequence;
                                    if let Ok(json) = serde_json::to_string(event) {
                                        yield Ok(Event::default().event("event").data(json));
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = ?e,
                                cursor = last_sequence,
                                "Lag recovery failed for SSE stream"
                            );
                            // Notify the client that lag recovery failed so they
                            // know their cursor may be stale and should reconnect.
                            yield Ok(Event::default()
                                .event("error")
                                .data(
                                    r#"{"error_type":"cursor_stale","message":"Lag recovery failed; cursor may be stale","recoverable":true}"#
                                ));
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(GATEWAY_KEEPALIVE_INTERVAL)
            .text("keepalive"),
    )
}

/// Cursor-based event journal query: `GET /api/v1/event-journal?cursor=<sequence>&partition=<name>&limit=<n>`
async fn event_journal_query(
    State(state): State<GatewayState>,
    axum::extract::Query(params): axum::extract::Query<EventJournalQueryParams>,
) -> Result<Json<EventPage>, (axum::http::StatusCode, Json<JournalError>)> {
    let cursor = StreamCursor::new(params.cursor, &params.partition);
    let limit = params.limit.clamp(1, GATEWAY_EVENT_PAGE_LIMIT);
    match state.journal.query_after(&cursor, limit).await {
        Ok(page) => Ok(Json(page)),
        Err(err) => {
            let status = match &err {
                JournalError::InvalidCursor { .. } => axum::http::StatusCode::BAD_REQUEST,
                JournalError::PartitionNotFound { .. } => axum::http::StatusCode::NOT_FOUND,
                _ => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err((status, Json(err)))
        }
    }
}

/// WebSocket event stream: `WS /api/v1/streams/events`
async fn event_stream_ws(
    State(state): State<GatewayState>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> impl axum::response::IntoResponse {
    use axum::extract::ws::{Message, WebSocket};
    upgrade.on_upgrade(move |socket: WebSocket| {
        let journal = state.journal.clone();
        let broker = state.broker.clone();
        async move {
            let mut socket = socket;

            // Register the connection with the broker.
            let connection_id: Arc<str> = Arc::from(format!("ws-{}", uuid::Uuid::new_v4()));
            broker.register_connection(connection_id.clone()).await;

            // Read optional init message for cursor/partition with a timeout.
            let init_timeout = tokio::time::timeout(Duration::from_secs(10), socket.recv());
            let (cursor, partition) = match init_timeout.await {
                Ok(Some(Ok(init_msg))) => match parse_init_message(&init_msg) {
                    Ok((c, p)) => (c, p),
                    Err(e) => {
                        tracing::warn!(
                            connection_id = %connection_id,
                            error = %e,
                            "Failed to parse init message, closing connection"
                        );
                        // Send a JSON error event and close the connection.
                        let _ = socket
                            .send(Message::Text(
                                r#"__error__ {"error_type":"invalid_init_message","message":"Failed to parse init message","recoverable":false}"#
                                    .into(),
                            ))
                            .await;
                        broker.unregister_connection(&connection_id).await;
                        return;
                    }
                },
                Ok(Some(Err(e))) => {
                    // WebSocket error while reading init message (client disconnected).
                    tracing::warn!(
                        connection_id = %connection_id,
                        error = %e,
                        "WebSocket error during init read, closing connection"
                    );
                    broker.unregister_connection(&connection_id).await;
                    return;
                }
                Ok(None) => {
                    // Client sent close frame before init.
                    tracing::info!(
                        connection_id = %connection_id,
                        "Client closed connection before sending init message"
                    );
                    broker.unregister_connection(&connection_id).await;
                    return;
                }
                Err(_) => {
                    // Timeout: client didn't send init message. Proceed with defaults
                    // so that clients unaware of the init protocol still work.
                    tracing::info!(
                        connection_id = %connection_id,
                        "Init message timed out; proceeding with defaults"
                    );
                    (StreamCursor::new(0, "events"), "events".to_string())
                }
            };

            // Subscribe to live events FIRST to prevent losing any events that arrive
            // between the backlog query and the live stream subscription.
            let mut event_stream = match broker.create_stream(&cursor) {
                Ok(s) => s,
                Err(err) => {
                    if let Ok(json) = serde_json::to_string(&err) {
                        let _ = socket
                            .send(Message::Text(format!("__error__ {}", json).into()))
                            .await;
                    }
                    broker.unregister_connection(&connection_id).await;
                    return;
                }
            };

            // Deliver backlog events (report errors to the client instead of swallowing).
            let query_cursor = StreamCursor::new(cursor.sequence, &partition);
            let mut last_backlog_sequence = cursor.sequence;
            match journal
                .query_after(&query_cursor, GATEWAY_EVENT_PAGE_LIMIT)
                .await
            {
                Ok(page) => {
                    for event in &page.events {
                        match serde_json::to_string(event) {
                            Ok(json) => {
                                if socket
                                    .send(Message::Text(format!("__event__ {}", json).into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                // Report serialization failure to the client.
                                let _ = socket
                                    .send(Message::Text(
                                        format!(
                                            "__error__ {{\"error_type\":\"serialization\",\"message\":\"Failed to serialize event {}\",\"recoverable\":true}}",
                                            event.event_id
                                        )
                                        .into(),
                                    ))
                                    .await;
                                tracing::warn!(
                                    event_id = %event.event_id,
                                    error = %e,
                                    "Failed to serialize backlog event"
                                );
                            }
                        }
                        last_backlog_sequence = event.sequence.max(last_backlog_sequence);
                    }
                }
                Err(err) => {
                    if let Ok(json) = serde_json::to_string(&err) {
                        let _ = socket
                            .send(Message::Text(format!("__error__ {}", json).into()))
                            .await;
                    }
                    // Stale cursor is unrecoverable via WS; close the connection.
                    broker.unregister_connection(&connection_id).await;
                    return;
                }
            }

            // Advance the event stream's cursor past the backlog so recv() won't
            // re-deliver events already sent above.
            event_stream.set_last_sequence(last_backlog_sequence);

            loop {
                match event_stream.recv().await {
                    Some(Ok(event)) => {
                        match serde_json::to_string(&event) {
                            Ok(json) => {
                                if socket
                                    .send(Message::Text(format!("__event__ {}", json).into()))
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(e) => {
                                // Report serialization failure to the client instead
                                // of silently dropping the event.
                                let _ = socket
                                    .send(Message::Text(
                                        format!(
                                            "__error__ {{\"error_type\":\"serialization\",\"message\":\"Failed to serialize event {}\",\"recoverable\":true}}",
                                            event.event_id
                                        )
                                        .into(),
                                    ))
                                    .await;
                                tracing::warn!(
                                    event_id = %event.event_id,
                                    error = %e,
                                    "Failed to serialize live event"
                                );
                            }
                        }
                    }
                    Some(Err(err)) => {
                        if let Ok(json) = serde_json::to_string(&err) {
                            let _ = socket
                                .send(Message::Text(format!("__error__ {}", json).into()))
                                .await;
                        }
                        if !err.recoverable {
                            break;
                        }
                    }
                    None => break,
                }
            }

            // Unregister the connection when the WebSocket closes.
            broker.unregister_connection(&connection_id).await;
        }
    })
}

fn parse_init_message(
    msg: &axum::extract::ws::Message,
) -> Result<(StreamCursor, String), Box<dyn std::error::Error + Send + Sync>> {
    let text = msg.to_text().map_err(|e: axum::Error| e.to_string())?;
    #[derive(serde::Deserialize)]
    struct InitMessage {
        #[serde(default)]
        cursor: u64,
        #[serde(default = "default_partition")]
        partition: String,
    }
    let init: InitMessage = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok((
        StreamCursor::new(init.cursor, &init.partition),
        init.partition,
    ))
}

/// Query parameters for event journal endpoint.
#[derive(Debug, serde::Deserialize)]
struct EventJournalQueryParams {
    #[serde(default)]
    cursor: u64,
    #[serde(default = "default_partition")]
    partition: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_partition() -> String {
    "events".into()
}

fn default_limit() -> usize {
    50
}
