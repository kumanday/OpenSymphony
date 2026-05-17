use std::{convert::Infallible, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use tokio::{net::TcpListener, sync::broadcast};

use crate::opensymphony_domain::{InMemoryEventJournal, StreamBroker};
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
        let journal = InMemoryEventJournal::new(GATEWAY_JOURNAL_CAPACITY, GATEWAY_SUBSCRIBER_CAPACITY);
        Self {
            journal: journal.clone(),
            broker: StreamBroker::new(journal),
            store,
        }
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

/// SSE snapshot stream: `GET /api/v1/events`
async fn events_sse(
    State(state): State<GatewayState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = state.store.subscribe();
    let initial = state.store.current().await;
    let store = state.store.clone();
    let stream = stream! {
        let mut last_sent_sequence = initial.sequence;
        yield Ok(snapshot_event(&initial));
        while let Some(envelope) =
            next_snapshot_envelope(&store, &mut receiver, &mut last_sent_sequence).await
        {
            yield Ok(snapshot_event(&envelope));
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
    let limit = params.limit.max(1).min(GATEWAY_EVENT_PAGE_LIMIT);
    match state.journal.query_after(&cursor, limit).await {
        Ok(page) => Ok(Json(page)),
        Err(err) => {
            let status = match &err {
                JournalError::InvalidCursor { .. } => axum::http::StatusCode::GONE,
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

            // Read optional init message for cursor/partition.
            let (cursor, partition) = if let Some(Ok(init_msg)) = socket.recv().await {
                match parse_init_message(&init_msg) {
                    Ok((c, p)) => (c, p),
                    Err(_) => (StreamCursor::new(0, "events"), "events".to_string()),
                }
            } else {
                (StreamCursor::new(0, "events"), "events".to_string())
            };

            // Deliver backlog events.
            let query_cursor = StreamCursor::new(cursor.sequence, &partition);
            if let Ok(page) = journal.query_after(&query_cursor, GATEWAY_EVENT_PAGE_LIMIT).await {
                for event in page.events {
                    if let Ok(json) = serde_json::to_string(&event) {
                        let _ = socket
                            .send(Message::Text(format!("__event__ {}", json).into()))
                            .await;
                    }
                }
            }

            // Stream live events.
            let mut event_stream = match broker.create_stream(&cursor) {
                Ok(s) => s,
                Err(err) => {
                    if let Ok(json) = serde_json::to_string(&err) {
                        let _ = socket
                            .send(Message::Text(format!("__error__ {}", json).into()))
                            .await;
                    }
                    return;
                }
            };

            loop {
                match event_stream.recv().await {
                    Some(Ok(event)) => {
                        if let Ok(json) = serde_json::to_string(&event) {
                            if socket
                                .send(Message::Text(format!("__event__ {}", json).into()))
                                .await
                                .is_err()
                            {
                                break;
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
    Ok((StreamCursor::new(init.cursor, &init.partition), init.partition))
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

fn snapshot_event(envelope: &SnapshotEnvelope) -> Event {
    let dashboard = control_plane_to_dashboard_snapshot(envelope);
    let payload =
        serde_json::to_string(&dashboard).expect("DashboardSnapshot is always serializable");
    Event::default()
        .event("snapshot")
        .id(envelope.sequence.to_string())
        .data(payload)
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
                if let Some(envelope) = latest_from_store(store, *last_sent_sequence).await {
                    *last_sent_sequence = envelope.sequence;
                    return Some(envelope);
                }
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

async fn latest_from_store(
    store: &SnapshotStore,
    last_sent_sequence: u64,
) -> Option<SnapshotEnvelope> {
    let latest = store.current().await;
    (latest.sequence > last_sent_sequence).then_some(latest)
}
