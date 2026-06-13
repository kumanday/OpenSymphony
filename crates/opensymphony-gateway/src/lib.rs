use std::{
    convert::Infallible,
    path::{Path as StdPath, PathBuf},
    sync::Arc,
    time::Duration,
};

use chrono::Utc;
use serde_json::json;

use async_stream::stream;
use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path as AxumPath, State,
        ws::{Message, WebSocket},
    },
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tokio::{net::TcpListener, sync::broadcast};
use tokio_util::io::ReaderStream;

use crate::opensymphony_domain::{EventStream, InMemoryEventJournal, StreamBroker};
use crate::opensymphony_gateway_schema::{
    cursor::StreamCursor,
    event_journal::{EventPage, EventRecord, JournalError, StreamError},
};

pub mod action_handler;
pub mod task_graph_mutations;
use action_handler::ActionHandler;
// Re-export the task-graph mutation types at the gateway crate level so
// integration tests and host wiring can use them via
// `opensymphony::opensymphony_gateway::TaskGraphMilestoneRequest` etc.
pub use task_graph_mutations::{
    IssueOp, LinearClientMutationAdapter, LinearMutationClient, MilestoneOp, MutationError,
    MutationOp, SubIssueOp, TaskGraphEvidenceRequest, TaskGraphEvidenceResponse,
    TaskGraphIssueRequest, TaskGraphIssueResponse, TaskGraphMilestoneRequest,
    TaskGraphMilestoneResponse, TaskGraphMutationState, TaskGraphRelationRequest,
    TaskGraphRelationResponse, TaskGraphSubIssueRequest, TaskGraphSubIssueResponse,
    append_mutation_event, append_mutation_event_with_op, entity_kind_for, task_graph_router,
};

pub use crate::opensymphony_control::SnapshotStore;
pub use crate::opensymphony_domain::{
    ControlPlaneAgentServerStatus, ControlPlaneDaemonSnapshot, ControlPlaneDaemonState,
    ControlPlaneDaemonStatus, ControlPlaneFileChange, ControlPlaneFileChangeKind,
    ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot, ControlPlaneMetricsSnapshot,
    ControlPlaneRecentEvent, ControlPlaneRecentEventKind, ControlPlaneWorkerOutcome,
    InMemoryEventJournal as DomainInMemoryEventJournal, SnapshotEnvelope,
    StreamBroker as DomainStreamBroker,
};
pub use crate::opensymphony_gateway_schema::{
    action::{
        ActionDispatch, ActionKind, ActionReceipt, ActionStatus, ActionTarget, ExpectedFollowup,
        PermissionResult,
    },
    capability::{AuthMode, FeatureCapability, GatewayCapabilities, TransportCapability},
    cursor::PageCursor,
    event_journal::{EventPage as GatewayEventPage, JournalError as EventJournalError},
    run::{
        ChangedFileEntry, DiffHunk, DiffLine, FileChangeKind, FileDiffPage, ReleaseReason,
        RunAction, RunDetail, RunEvent, RunEventPage, RunFilesPage, RunLifecycleState, RunPhase,
        RunStatus, SafeActions,
    },
    snapshot::{
        DashboardSnapshot, GatewayHealth, GatewayMetrics, ProjectDetail, ProjectIssueSummary,
        ProjectIssuesPage, ProjectList, ProjectMilestoneSummary, ProjectSummary, SnapshotEventKind,
        SnapshotEventSummary,
    },
    task_graph::{DiffSummary, TaskGraphRuntimeOverlay, TaskGraphSnapshot, TaskGraphStateCategory},
    version::{GATEWAY_SCHEMA_VERSION, SchemaVersion},
};

const GATEWAY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);
const GATEWAY_JOURNAL_CAPACITY: usize = 10_000;
const GATEWAY_SUBSCRIBER_CAPACITY: usize = 256;
const GATEWAY_EVENT_PAGE_LIMIT: usize = 100;
const GATEWAY_WS_INIT_TIMEOUT: Duration = Duration::from_secs(10);

fn stream_error_from_journal_error(err: &JournalError, cursor_sequence: u64) -> StreamError {
    match err {
        JournalError::InvalidCursor { .. } => StreamError::cursor_not_found(cursor_sequence),
        JournalError::PartitionNotFound { partition } => {
            StreamError::disconnected(format!("Partition not found: {partition}"))
        }
        JournalError::Backpressure { .. } => StreamError::backpressure(),
        JournalError::NotFound { event_id } => {
            StreamError::disconnected(format!("Event not found: {event_id}"))
        }
    }
}

fn serialize_stream_error(err: &StreamError) -> String {
    serde_json::to_string(err).expect("serialization of derived Serialize type should never fail")
}

fn ws_error_frame(err: &StreamError) -> String {
    format!("__error__ {}", serialize_stream_error(err))
}

fn ws_event_frame(event: &EventRecord) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|json| format!("__event__ {json}"))
}

async fn send_ws_frame(socket: &mut WebSocket, frame: String) -> Result<(), axum::Error> {
    socket.send(Message::Text(frame.into())).await
}

async fn send_ws_stream_error(
    socket: &mut WebSocket,
    err: &StreamError,
) -> Result<(), axum::Error> {
    send_ws_frame(socket, ws_error_frame(err)).await
}

async fn send_ws_server_error(
    socket: &mut WebSocket,
    message: &'static str,
) -> Result<(), axum::Error> {
    let err = StreamError::server_error(message);
    send_ws_stream_error(socket, &err).await
}

#[derive(Debug, Clone, Copy)]
enum WsReplayKind {
    Backlog,
    LagRecovery,
    Live,
}

impl WsReplayKind {
    fn serialize_error_message(self) -> &'static str {
        match self {
            Self::Backlog => "Failed to serialize backlog event",
            Self::LagRecovery => "Failed to serialize lag recovery event",
            Self::Live => "Failed to serialize live event",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Backlog => "backlog",
            Self::LagRecovery => "lag_recovery",
            Self::Live => "live",
        }
    }
}

async fn send_ws_event(
    socket: &mut WebSocket,
    event: &EventRecord,
    replay_kind: WsReplayKind,
) -> bool {
    match ws_event_frame(event) {
        Ok(frame) => send_ws_frame(socket, frame).await.is_ok(),
        Err(err) => {
            let _ = send_ws_server_error(socket, replay_kind.serialize_error_message()).await;
            tracing::warn!(
                event_id = %event.event_id,
                error = %err,
                replay_kind = replay_kind.label(),
                "Failed to serialize WebSocket event"
            );
            true
        }
    }
}

#[derive(Debug)]
struct BrokerConnectionGuard {
    broker: StreamBroker,
    connection_id: Arc<str>,
}

impl BrokerConnectionGuard {
    fn new(broker: StreamBroker, connection_id: Arc<str>) -> Self {
        Self {
            broker,
            connection_id,
        }
    }
}

impl Drop for BrokerConnectionGuard {
    fn drop(&mut self) {
        let broker = self.broker.clone();
        let connection_id = self.connection_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let join = handle.spawn(async move {
                broker.unregister_connection(&connection_id).await;
            });
            drop(join);
        }
    }
}

/// Shared state for the gateway server.
pub struct GatewayState {
    pub store: SnapshotStore,
    pub journal: InMemoryEventJournal,
    pub broker: StreamBroker,
    pub web_assets_dir: Option<String>,
    pub action_handler: ActionHandler,
    pub linear_mutations: Option<Arc<dyn LinearMutationClient>>,
}

impl Clone for GatewayState {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            journal: self.journal.clone(),
            broker: self.broker.clone(),
            web_assets_dir: self.web_assets_dir.clone(),
            action_handler: self.action_handler.clone(),
            linear_mutations: self.linear_mutations.clone(),
        }
    }
}

impl axum::extract::FromRef<GatewayState> for SnapshotStore {
    fn from_ref(state: &GatewayState) -> Self {
        state.store.clone()
    }
}

/// V1 gateway server that exposes stable public DTO endpoints
/// on top of the internal control-plane `SnapshotStore`.
pub struct GatewayServer {
    store: SnapshotStore,
    journal: InMemoryEventJournal,
    broker: StreamBroker,
    web_assets_dir: Option<String>,
    linear_mutations: Option<Arc<dyn LinearMutationClient>>,
}

impl Clone for GatewayServer {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            journal: self.journal.clone(),
            broker: self.broker.clone(),
            web_assets_dir: self.web_assets_dir.clone(),
            linear_mutations: self.linear_mutations.clone(),
        }
    }
}

impl std::fmt::Debug for GatewayServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayServer")
            .field("store", &"<store>")
            .field("journal", &"<journal>")
            .field("broker", &"<broker>")
            .field("web_assets_dir", &self.web_assets_dir)
            .field(
                "linear_mutations",
                &self.linear_mutations.as_ref().map(|_| "..."),
            )
            .finish()
    }
}

impl GatewayServer {
    pub fn new(store: SnapshotStore) -> Self {
        let journal =
            InMemoryEventJournal::new(GATEWAY_JOURNAL_CAPACITY, GATEWAY_SUBSCRIBER_CAPACITY);
        Self {
            journal: journal.clone(),
            broker: StreamBroker::new(journal.clone()),
            store,
            web_assets_dir: None,
            linear_mutations: None,
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
            web_assets_dir: None,
            linear_mutations: None,
        }
    }

    /// Enable serving of the built web client from the given directory.
    pub fn with_web_assets(mut self, dir: impl Into<String>) -> Self {
        self.web_assets_dir = Some(dir.into());
        self
    }

    /// Install a Linear mutation client for the `/api/v1/taskgraph/*`
    /// endpoints. The endpoints respond with 503 until this is configured
    /// because the host client must not call Linear without an explicit
    /// adapter wired in (tests inject fakes; production wires
    /// `LinearClientMutationAdapter`).
    pub fn with_linear_mutations(mut self, client: Option<Arc<dyn LinearMutationClient>>) -> Self {
        self.linear_mutations = client;
        self
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
            web_assets_dir: self.web_assets_dir.clone(),
            action_handler: ActionHandler::new(self.journal.clone()),
            linear_mutations: self.linear_mutations.clone(),
        };
        let mut router = Router::new()
            .route("/api/v1/capabilities", get(capabilities))
            .route("/api/v1/dashboard/snapshot", get(dashboard_snapshot))
            .route("/api/v1/events", get(events))
            .route("/api/v1/event-journal", get(event_journal_query))
            .route("/api/v1/streams/events", get(event_stream_ws))
            .route("/api/v1/projects", get(list_projects))
            .route("/api/v1/projects/{project_id}", get(get_project))
            .route(
                "/api/v1/projects/{project_id}/taskgraph",
                get(get_task_graph),
            )
            .route("/api/v1/runs/{run_id}", get(get_run_detail))
            .route("/api/v1/runs/{run_id}/events", get(get_run_events))
            .route("/api/v1/runs/{run_id}/files", get(get_run_files))
            .route("/api/v1/runs/{run_id}/diffs", get(get_run_diffs))
            .route("/api/v1/actions/dispatch", post(dispatch_action));

        if self.web_assets_dir.is_some() {
            router = router
                .route("/app", get(web_asset_handler))
                .route("/app/", get(web_asset_handler))
                .route("/app/{*path}", get(web_asset_handler));
        }

        // Merge in the `/api/v1/taskgraph/*` mutation routers. They carry
        // their own dedicated state container so the gateway state type
        // doesn't have to expose every internal field to the mutation
        // handlers (which only need the journal and the optional mutation
        // client).
        let mutation_state = TaskGraphMutationState {
            journal: self.journal.clone(),
            linear_mutations: self.linear_mutations.clone(),
        };
        let mutation_router = task_graph_router().with_state(mutation_state);
        router = router.nest("/api/v1/taskgraph", mutation_router);

        router.with_state(state)
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
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "action_dispatch".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "action_receipts".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "run_detail".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            FeatureCapability {
                feature: "event_journal".into(),
                available: true,
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

/// POST /api/v1/actions/dispatch
///
/// Validates the action against the current snapshot state, publishes an audit
/// event to the journal, and returns a receipt so callers can correlate with
/// follow-up events via the event stream.
async fn dispatch_action(
    State(state): State<GatewayState>,
    Json(action): Json<ActionDispatch>,
) -> impl IntoResponse {
    let envelope = state.store.current().await;
    let receipt = state.action_handler.dispatch(action, &envelope).await;

    match receipt.status {
        ActionStatus::Accepted => (StatusCode::OK, Json(receipt)),
        ActionStatus::Rejected => {
            let status = dispatch_rejection_status(&receipt);
            (status, Json(receipt))
        }
    }
}

/// Map rejection reasons to granular HTTP status codes so API consumers can
/// distinguish retryable vs. non-retryable failures without parsing the receipt.
fn dispatch_rejection_status(receipt: &ActionReceipt) -> StatusCode {
    let Some(ref reason) = receipt.reason else {
        return StatusCode::BAD_REQUEST;
    };
    let lower = reason.to_lowercase();
    if lower.contains("permission denied") {
        StatusCode::FORBIDDEN
    } else if lower.contains("duplicate idempotency key") {
        StatusCode::CONFLICT
    } else if lower.contains("not found") {
        StatusCode::NOT_FOUND
    } else if lower.contains("already active")
        || lower.contains("unsafe in state")
        || lower.contains("only valid on")
    {
        StatusCode::UNPROCESSABLE_ENTITY
    } else {
        StatusCode::BAD_REQUEST
    }
}

/// SSE journal event stream: `GET /api/v1/events`
///
/// Streams committed journal events as Server-Sent Events. Unlike the old
/// snapshot-based stream, this endpoint delivers individual journal events
/// with stable IDs, monotonic sequence numbers, and typed payloads.
async fn events(
    State(state): State<GatewayState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let journal = state.journal.clone();
    let stream = stream! {
        // Subscribe first to avoid a race window where events appended between
        // latest_cursor() and subscribe() would be broadcast before the receiver
        // exists and permanently lost.
        let mut receiver = journal.subscribe();
        let mut last_sequence = 0;
        let partition = "events".to_string();

        // Deliver historical events from the backlog before entering the live loop.
        // Query from cursor 0 to get all available events in the journal.
        let mut backlog_cursor = StreamCursor::new(0, &partition);
        let mut backlog_max_sequence: Option<u64> = None;
        loop {
            match journal.query_after(&backlog_cursor, GATEWAY_EVENT_PAGE_LIMIT).await {
                Ok(page) => {
                    for event in &page.events {
                        // Only deliver events that weren't already seen via broadcast.
                        if backlog_max_sequence.is_none_or(|max| event.sequence > max) {
                            match serde_json::to_string(event) {
                                Ok(json) => {
                                    yield Ok(Event::default().event("event").data(json));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        event_id = %event.event_id,
                                        error = %e,
                                        "Failed to serialize SSE backlog event"
                                    );
                                    let error_json = serialize_stream_error(
                                        &StreamError::server_error(
                                            "Failed to serialize SSE backlog event",
                                        ),
                                    );
                                    yield Ok(Event::default().event("error").data(error_json));
                                }
                            }
                        }
                        backlog_max_sequence = Some(
                            backlog_max_sequence.map_or(event.sequence, |max| max.max(event.sequence))
                        );
                    }
                    if !page.has_more {
                        break;
                    }
                    if let Some(ref next) = page.next_cursor {
                        backlog_cursor = next.clone();
                    } else {
                        break;
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        error = ?e,
                        cursor = backlog_cursor.sequence,
                        "Backlog query failed for SSE stream"
                    );
                    let error_json = serialize_stream_error(&stream_error_from_journal_error(
                        &e,
                        backlog_cursor.sequence,
                    ));
                    yield Ok(Event::default().event("error").data(error_json));
                    break;
                }
            }
        }

        // Update last_sequence to the highest backlog sequence delivered,
        // so the live loop skips events we already sent from the backlog.
        if let Some(max_seq) = backlog_max_sequence {
            last_sequence = last_sequence.max(max_seq);
        }

        // Now listen for live events, skipping anything already delivered from backlog.
        loop {
            match receiver.recv().await {
                Ok(Ok(event)) => {
                    if event.sequence <= last_sequence {
                        continue;
                    }
                    // Skip events from other partitions so terminal frames do
                    // not advance the public control-event stream cursor.
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
                            let error_json = serde_json::to_string(&StreamError::server_error(
                                "Failed to serialize journal event",
                            ))
                            .expect("serialization of derived Serialize type should never fail");
                            yield Ok(Event::default().event("error").data(error_json));
                        }
                    }
                }
                Ok(Err(ref err)) => {
                    let err_json = serde_json::to_string(err).expect("serialization of derived Serialize type should never fail");
                    yield Ok(Event::default().event("error").data(err_json));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Paginate through all lagged events to avoid gaps when
                    // the backlog exceeds a single page limit.
                    let mut recovery_cursor =
                        StreamCursor::new(last_sequence, &partition);
                    loop {
                        match journal
                            .query_after(&recovery_cursor, GATEWAY_EVENT_PAGE_LIMIT)
                            .await
                        {
                            Ok(page) => {
                                for event in &page.events {
                                    if event.sequence > last_sequence {
                                        last_sequence = event.sequence;
                                        match serde_json::to_string(event) {
                                            Ok(json) => {
                                                yield Ok(Event::default().event("event").data(json));
                                            }
                                            Err(e) => {
                                                tracing::warn!(
                                                    event_id = %event.event_id,
                                                    error = %e,
                                                    "Failed to serialize SSE lag recovery event"
                                                );
                                                let error_json = serde_json::to_string(&StreamError::server_error(
                                                    "Failed to serialize lag recovery event",
                                                ))
                                                .expect("serialization of derived Serialize type should never fail");
                                                yield Ok(Event::default().event("error").data(error_json));
                                            }
                                        }
                                    }
                                }
                                if !page.has_more {
                                    break;
                                }
                                if let Some(ref next) = page.next_cursor {
                                    recovery_cursor = next.clone();
                                } else {
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = ?e,
                                    cursor = recovery_cursor.sequence,
                                    "Lag recovery failed for SSE stream"
                                );
                                let error_json = serialize_stream_error(&stream_error_from_journal_error(
                                    &e,
                                    recovery_cursor.sequence,
                                ));
                                yield Ok(Event::default().event("error").data(error_json));
                                break;
                            }
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
) -> Result<Json<EventPage>, (StatusCode, Json<JournalError>)> {
    let cursor = StreamCursor::new(params.cursor, &params.partition);
    let limit = params.limit.clamp(1, GATEWAY_EVENT_PAGE_LIMIT);
    match state.journal.query_after(&cursor, limit).await {
        Ok(page) => Ok(Json(page)),
        Err(err) => {
            let status = match &err {
                JournalError::InvalidCursor { .. } => StatusCode::BAD_REQUEST,
                JournalError::PartitionNotFound { .. } => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            Err((status, Json(err)))
        }
    }
}

async fn read_ws_init_message(
    socket: &mut WebSocket,
    connection_id: &str,
) -> Option<(StreamCursor, String)> {
    let init_deadline = tokio::time::Instant::now() + GATEWAY_WS_INIT_TIMEOUT;

    loop {
        let remaining = init_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            tracing::warn!(
                connection_id = %connection_id,
                "Init message timed out; closing WebSocket connection"
            );
            let _ = send_ws_server_error(
                socket,
                "Init message not received within timeout; connection closed",
            )
            .await;
            return None;
        }

        match tokio::time::timeout(remaining, socket.recv()).await {
            Ok(Some(Ok(msg))) => match msg {
                Message::Text(_) => match parse_init_message(&msg) {
                    Ok(init) => return Some(init),
                    Err(err) => {
                        tracing::warn!(
                            connection_id = %connection_id,
                            error = %err,
                            "Failed to parse init message, closing connection"
                        );
                        let _ = send_ws_server_error(socket, "Failed to parse init message").await;
                        return None;
                    }
                },
                Message::Ping(payload) => {
                    // Keep the connection alive while waiting for the init message.
                    let _ = socket.send(Message::Pong(payload)).await;
                }
                Message::Pong(_) | Message::Binary(_) => {}
                Message::Close(_) => {
                    tracing::info!(
                        connection_id = %connection_id,
                        "Client closed connection before sending init message"
                    );
                    return None;
                }
            },
            Ok(Some(Err(err))) => {
                tracing::warn!(
                    connection_id = %connection_id,
                    error = %err,
                    "WebSocket error during init read, closing connection"
                );
                return None;
            }
            Ok(None) => {
                tracing::info!(
                    connection_id = %connection_id,
                    "Client closed connection before sending init message"
                );
                return None;
            }
            Err(_) => {
                tracing::warn!(
                    connection_id = %connection_id,
                    "Init message timed out; closing WebSocket connection"
                );
                let _ = send_ws_server_error(
                    socket,
                    "Init message not received within timeout; connection closed",
                )
                .await;
                return None;
            }
        }
    }
}

async fn create_ws_event_stream(
    socket: &mut WebSocket,
    broker: &StreamBroker,
    cursor: &StreamCursor,
) -> Option<EventStream> {
    match broker.create_stream(cursor) {
        Ok(stream) => Some(stream),
        Err(err) => {
            let _ = send_ws_stream_error(socket, &err).await;
            None
        }
    }
}

async fn replay_ws_events_from_cursor(
    socket: &mut WebSocket,
    journal: &InMemoryEventJournal,
    mut cursor: StreamCursor,
    replay_kind: WsReplayKind,
) -> Option<u64> {
    let mut last_sequence = cursor.sequence;

    loop {
        match journal.query_after(&cursor, GATEWAY_EVENT_PAGE_LIMIT).await {
            Ok(page) => {
                for event in &page.events {
                    if !send_ws_event(socket, event, replay_kind).await {
                        return None;
                    }
                    last_sequence = event.sequence.max(last_sequence);
                }
                if !page.has_more {
                    return Some(last_sequence);
                }
                if let Some(next) = page.next_cursor {
                    cursor = next;
                } else {
                    return Some(last_sequence);
                }
            }
            Err(journal_err) => {
                let stream_err = stream_error_from_journal_error(&journal_err, cursor.sequence);
                let _ = send_ws_stream_error(socket, &stream_err).await;
                tracing::warn!(
                    error = ?journal_err,
                    cursor_sequence = cursor.sequence,
                    replay_kind = replay_kind.label(),
                    "Journal query failed during WebSocket event replay"
                );
                return None;
            }
        }
    }
}

async fn forward_ws_live_events(
    socket: &mut WebSocket,
    journal: &InMemoryEventJournal,
    event_stream: &mut EventStream,
    partition: &str,
) {
    loop {
        match event_stream.recv().await {
            Some(Ok(event)) => {
                if !send_ws_event(socket, &event, WsReplayKind::Live).await {
                    break;
                }
            }
            Some(Err(err)) => {
                let _ = send_ws_stream_error(socket, &err).await;
                if !err.recoverable {
                    break;
                }

                let lag_cursor = StreamCursor::new(event_stream.last_sequence(), partition);
                let Some(last_sequence) = replay_ws_events_from_cursor(
                    socket,
                    journal,
                    lag_cursor,
                    WsReplayKind::LagRecovery,
                )
                .await
                else {
                    return;
                };
                event_stream.set_last_sequence(last_sequence);
            }
            None => break,
        }
    }
}

/// WebSocket event stream: `WS /api/v1/streams/events`
async fn event_stream_ws(
    State(state): State<GatewayState>,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade.on_upgrade(move |socket: WebSocket| {
        let journal = state.journal.clone();
        let broker = state.broker.clone();
        async move {
            let mut socket = socket;
            let connection_id: Arc<str> = Arc::from(format!("ws-{}", uuid::Uuid::new_v4()));
            broker.register_connection(connection_id.clone()).await;
            let _connection_guard =
                BrokerConnectionGuard::new(broker.clone(), connection_id.clone());

            let Some((cursor, partition)) = read_ws_init_message(&mut socket, &connection_id).await
            else {
                broker.unregister_connection(&connection_id).await;
                return;
            };

            let Some(mut event_stream) =
                create_ws_event_stream(&mut socket, &broker, &cursor).await
            else {
                broker.unregister_connection(&connection_id).await;
                return;
            };

            let backlog_cursor = StreamCursor::new(cursor.sequence, &partition);
            let Some(last_backlog_sequence) = replay_ws_events_from_cursor(
                &mut socket,
                &journal,
                backlog_cursor,
                WsReplayKind::Backlog,
            )
            .await
            else {
                broker.unregister_connection(&connection_id).await;
                return;
            };
            event_stream.set_last_sequence(last_backlog_sequence);

            forward_ws_live_events(&mut socket, &journal, &mut event_stream, &partition).await;
            broker.unregister_connection(&connection_id).await;
        }
    })
}

fn parse_init_message(
    msg: &Message,
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

// ── Read API helpers ──────────────────────────────────────────────────────────

fn find_issue_snapshot<'a>(
    envelope: &'a SnapshotEnvelope,
    run_id: &'a str,
) -> Option<&'a ControlPlaneIssueSnapshot> {
    envelope.snapshot.issues.iter().find(|issue| {
        issue.identifier.eq_ignore_ascii_case(run_id)
            || issue.conversation_id_suffix.eq_ignore_ascii_case(run_id)
    })
}

/// Resolve `..` and `.` components in a path without touching the filesystem.
///
/// A crafted path like `/tmp/opensymphony/../etc/passwd` becomes `/tmp/etc/passwd`.
fn normalize_path(path: &StdPath) -> PathBuf {
    let mut components: Vec<_> = path.components().collect();
    let is_absolute = components
        .first()
        .is_some_and(|c| matches!(c, std::path::Component::RootDir));

    let mut stack: Vec<_> = Vec::new();
    if is_absolute {
        // Preserve the leading root dir (first component); skip CurDir entries.
        stack.push(components.remove(0));
    }

    for comp in components {
        match &comp {
            std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => {
                // Pop only if we are not at the root.
                if let Some(last) = stack.last()
                    && matches!(last, std::path::Component::RootDir)
                {
                    continue;
                }
                stack.pop();
            }
            _ => stack.push(comp),
        }
    }
    stack.into_iter().collect()
}

/// Strip the workspace root from a raw absolute path so that the public API
/// never leaks a local filesystem path outside the workspace boundary.
///
/// Normalizes `..` and `.` components in **both** the workspace root and the
/// candidate path before stripping, so that crafted paths such as
/// `/tmp/opensymphony/../etc/passwd` cannot bypass the workspace guard.
pub fn sanitize_file_path(workspace_root: &str, raw_path: &str) -> String {
    let root = normalize_path(StdPath::new(workspace_root));
    let normalized = normalize_path(StdPath::new(raw_path));

    normalized
        .strip_prefix(&root)
        .map(|rel: &StdPath| rel.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            // Out-of-workspace path: use the NORMALIZED path to extract the
            // basename, so that crafted paths like `/tmp/opensymphony/..` do
            // not leak traversal components (`..`) into the public API.
            normalized
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        })
}

/// Resolve the requested path and verify it stays inside the assets directory.
fn resolve_safe_asset_path(assets_dir: &str, rest: &str) -> Option<PathBuf> {
    if StdPath::new(rest).is_absolute() {
        return None;
    }

    let base = StdPath::new(assets_dir);
    let candidate = base.join(rest);
    match (candidate.canonicalize(), base.canonicalize()) {
        (Ok(resolved), Ok(base_resolved)) => {
            if resolved == base_resolved || resolved.starts_with(&base_resolved) {
                Some(resolved)
            } else {
                None
            }
        }
        _ => {
            if candidate
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                None
            } else {
                Some(candidate)
            }
        }
    }
}

async fn serve_index_html(assets_dir: &str) -> Option<Response> {
    let index_path = StdPath::new(assets_dir).join("index.html");
    serve_file(&index_path).await.ok()
}

async fn web_asset_handler(
    State(state): State<GatewayState>,
    path: Option<AxumPath<String>>,
) -> Response {
    let Some(assets_dir) = state.web_assets_dir.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let rest = path.map(|p| p.0).unwrap_or_default();
    if rest.is_empty() {
        return serve_index_html(assets_dir)
            .await
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response());
    }

    let Some(safe_path) = resolve_safe_asset_path(assets_dir, &rest) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if safe_path.is_file() {
        return match serve_file(&safe_path).await {
            Ok(resp) => resp,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }

    if !path_has_known_extension(&rest) {
        return serve_index_html(assets_dir)
            .await
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response());
    }

    StatusCode::NOT_FOUND.into_response()
}

const KNOWN_ASSET_MIME_TYPES: &[(&str, &str)] = &[
    ("html", "text/html; charset=utf-8"),
    ("css", "text/css; charset=utf-8"),
    ("js", "application/javascript; charset=utf-8"),
    ("json", "application/json"),
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("svg", "image/svg+xml"),
    ("ico", "image/x-icon"),
    ("woff", "font/woff"),
    ("woff2", "font/woff2"),
    ("ttf", "font/ttf"),
    ("eot", "application/vnd.ms-fontobject"),
    ("otf", "font/otf"),
    ("map", "application/json"),
    ("txt", "text/plain; charset=utf-8"),
    ("xml", "application/xml"),
    ("webp", "image/webp"),
    ("mp4", "video/mp4"),
    ("webm", "video/webm"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
    ("flac", "audio/flac"),
    ("pdf", "application/pdf"),
    ("zip", "application/zip"),
    ("gz", "application/gzip"),
    ("tar", "application/x-tar"),
    ("bz2", "application/x-bzip2"),
];

fn path_has_known_extension(path: &str) -> bool {
    path.rsplit_once('.')
        .and_then(|(_, ext)| mime_type_for_extension(ext))
        .is_some()
}

async fn serve_file(path: &StdPath) -> Result<Response, std::io::Error> {
    let file = tokio::fs::File::open(path).await?;
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);
    let content_type = mime_type(path);
    Ok(([(axum::http::header::CONTENT_TYPE, content_type)], body).into_response())
}

fn mime_type(path: &StdPath) -> &'static str {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(mime_type_for_extension)
        .unwrap_or("application/octet-stream")
}

fn mime_type_for_extension(extension: &str) -> Option<&'static str> {
    KNOWN_ASSET_MIME_TYPES
        .iter()
        .find_map(|(known, mime)| known.eq_ignore_ascii_case(extension).then_some(*mime))
}

fn map_file_change_kind(kind: ControlPlaneFileChangeKind) -> FileChangeKind {
    match kind {
        ControlPlaneFileChangeKind::Created => FileChangeKind::Created,
        ControlPlaneFileChangeKind::Modified => FileChangeKind::Modified,
        ControlPlaneFileChangeKind::Removed => FileChangeKind::Removed,
    }
}

// ── Project endpoints ─────────────────────────────────────────────────────────

async fn list_projects(State(store): State<SnapshotStore>) -> Json<ProjectList> {
    let envelope = store.current().await;
    let snapshot = &envelope.snapshot;
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

    Json(ProjectList {
        schema_version: SchemaVersion::v1(),
        projects,
    })
}

async fn get_project(
    State(store): State<SnapshotStore>,
    AxumPath(project_id): AxumPath<String>,
) -> impl IntoResponse {
    // Only the "default" project is supported; reject unknown project IDs.
    if project_id != "default" {
        return (
            StatusCode::NOT_FOUND,
            Json(ProjectDetail {
                schema_version: SchemaVersion::v1(),
                project_id,
                name: String::new(),
                milestone_count: 0,
                issue_count: 0,
                running_count: 0,
                completed_count: 0,
                failed_count: 0,
                summary: Some("Project not found".into()),
                milestones: Vec::new(),
            }),
        );
    }

    let envelope = store.current().await;
    let snapshot = &envelope.snapshot;
    let issue_count = snapshot.issues.len() as u32;
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

    (
        StatusCode::OK,
        Json(ProjectDetail {
            schema_version: SchemaVersion::v1(),
            project_id,
            name: "OpenSymphony".into(),
            milestone_count: 0,
            issue_count,
            running_count: running,
            completed_count: completed,
            failed_count: failed,
            summary: Some("Current workspace issues".into()),
            milestones: Vec::new(),
        }),
    )
}

// ── Task Graph endpoint ───────────────────────────────────────────────────────

async fn get_task_graph(
    State(store): State<SnapshotStore>,
    AxumPath(project_id): AxumPath<String>,
) -> impl IntoResponse {
    // Only the "default" project is supported; reject unknown project IDs.
    if project_id != "default" {
        return (
            StatusCode::NOT_FOUND,
            Json(TaskGraphSnapshot {
                schema_version: SchemaVersion::v1(),
                project_id,
                generated_at: Utc::now(),
                nodes: Vec::new(),
                root_ids: Vec::new(),
            }),
        );
    }

    let envelope = store.current().await;
    let snapshot = &envelope.snapshot;
    let generated_at = Utc::now();

    let nodes: Vec<_> = snapshot
        .issues
        .iter()
        .map(|issue| {
            let state_category = map_runtime_state_to_graph_category(&issue.runtime_state);
            let runtime_overlay = build_runtime_overlay(issue);

            crate::opensymphony_gateway_schema::task_graph::TaskGraphNode {
                schema_version: SchemaVersion::v1(),
                node_id: issue.identifier.clone(),
                kind: crate::opensymphony_gateway_schema::task_graph::TaskGraphNodeKind::Issue,
                identifier: issue.identifier.clone(),
                title: issue.title.clone(),
                state: issue.tracker_state.clone(),
                state_category,
                priority: None,
                parent_id: None,
                children: Vec::new(),
                // Dependency info not yet available from the control-plane snapshot;
                // return an empty vector instead of self-referential placeholder data.
                blocked_by: Vec::new(),
                url: None,
                branch_name: None,
                labels: Vec::new(),
                created_at: None,
                updated_at: None,
                estimate_minutes: None,
                runtime_overlay: Some(runtime_overlay),
            }
        })
        .collect();

    // Parent/child relationship data is not yet available from the control-plane
    // snapshot, so every node is treated as a standalone leaf. Returning an empty
    // root_ids prevents clients from building an incorrect flat-forest layout.
    let root_ids: Vec<String> = Vec::new();

    (
        StatusCode::OK,
        Json(TaskGraphSnapshot {
            schema_version: SchemaVersion::v1(),
            project_id,
            generated_at,
            nodes,
            root_ids,
        }),
    )
}

fn map_runtime_state_to_graph_category(
    state: &ControlPlaneIssueRuntimeState,
) -> TaskGraphStateCategory {
    match state {
        ControlPlaneIssueRuntimeState::Idle => TaskGraphStateCategory::Todo,
        ControlPlaneIssueRuntimeState::Running => TaskGraphStateCategory::InProgress,
        ControlPlaneIssueRuntimeState::Paused => TaskGraphStateCategory::InProgress,
        ControlPlaneIssueRuntimeState::RetryQueued => TaskGraphStateCategory::InProgress,
        ControlPlaneIssueRuntimeState::Releasing => TaskGraphStateCategory::InProgress,
        ControlPlaneIssueRuntimeState::Completed => TaskGraphStateCategory::Done,
        ControlPlaneIssueRuntimeState::Failed => TaskGraphStateCategory::Done,
    }
}

fn build_runtime_overlay(issue: &ControlPlaneIssueSnapshot) -> TaskGraphRuntimeOverlay {
    let diff_summary = if issue.modified_files.is_empty() {
        None
    } else {
        let added = issue
            .modified_files
            .iter()
            .filter(|f| f.change_kind == ControlPlaneFileChangeKind::Created)
            .count() as u32;
        let modified = issue
            .modified_files
            .iter()
            .filter(|f| f.change_kind == ControlPlaneFileChangeKind::Modified)
            .count() as u32;
        let removed = issue
            .modified_files
            .iter()
            .filter(|f| f.change_kind == ControlPlaneFileChangeKind::Removed)
            .count() as u32;
        let lines_added: u32 = issue.modified_files.iter().map(|f| f.lines_added).sum();
        let lines_removed: u32 = issue.modified_files.iter().map(|f| f.lines_removed).sum();

        Some(DiffSummary {
            files_added: added,
            files_modified: modified,
            files_removed: removed,
            lines_added,
            lines_removed,
        })
    };

    let outcome = match issue.last_outcome {
        ControlPlaneWorkerOutcome::Unknown => None,
        ControlPlaneWorkerOutcome::Running => Some("running".into()),
        ControlPlaneWorkerOutcome::Continued => Some("continued".into()),
        ControlPlaneWorkerOutcome::Completed => Some("completed".into()),
        ControlPlaneWorkerOutcome::Failed => Some("failed".into()),
        ControlPlaneWorkerOutcome::Canceled => Some("canceled".into()),
    };

    let is_running = matches!(issue.runtime_state, ControlPlaneIssueRuntimeState::Running);
    // An issue is eligible only when it is idle (not yet started) and not
    // blocked.  Completed and failed issues must not appear eligible.
    let is_eligible =
        !issue.blocked && matches!(issue.runtime_state, ControlPlaneIssueRuntimeState::Idle);
    // Queued means the issue is actively waiting to be picked up by a worker.
    // Blocked issues must never appear queued, regardless of state:
    // a blocked Idle issue is not schedulable, and a blocked RetryQueued
    // issue is waiting on its blocker to clear before retry.
    let is_queued = !issue.blocked
        && (matches!(issue.runtime_state, ControlPlaneIssueRuntimeState::Idle)
            || matches!(
                issue.runtime_state,
                ControlPlaneIssueRuntimeState::RetryQueued
            ));

    TaskGraphRuntimeOverlay {
        eligible: is_eligible,
        queued: is_queued,
        // active_run_id maps to the gateway run identifier (the Linear issue
        // identifier), which is the key used by the /runs/{run_id} endpoints.
        active_run_id: is_running.then(|| issue.identifier.clone()),
        last_outcome: outcome,
        retry_count: issue.retry_count,
        workspace_id: (!issue.workspace_path_suffix.is_empty())
            .then(|| issue.workspace_path_suffix.clone()),
        harness_type: issue.server_base_url.is_some().then(|| "openhands".into()),
        conversation_id: (!issue.conversation_id_suffix.is_empty())
            .then(|| format!("conv-{}", issue.conversation_id_suffix)),
        last_event_at: (issue.last_event_at.timestamp() != 0).then_some(issue.last_event_at),
        diff_summary,
        validation_status: None,
        blocker_summary: if issue.blocked {
            Some("Blocked by dependency".into())
        } else {
            None
        },
    }
}

// ── Run endpoints ─────────────────────────────────────────────────────────────

async fn get_run_detail(
    State(store): State<SnapshotStore>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.current().await;
    let issue = match find_issue_snapshot(&envelope, &run_id) {
        Some(issue) => issue,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(RunDetail {
                    schema_version: SchemaVersion::v1(),
                    run_id,
                    issue_id: String::new(),
                    issue_identifier: String::new(),
                    worker_id: String::new(),
                    status: RunStatus::Unclaimed,
                    lifecycle_state: RunLifecycleState::Eligible,
                    claimed_at: Utc::now(),
                    started_at: None,
                    finished_at: None,
                    release_reason: None,
                    turn_count: 0,
                    max_turns: 0,
                    retry_attempt: None,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    runtime_seconds: 0,
                    conversation_id: None,
                    workspace_id: None,
                    workspace_path: None,
                    harness_type: None,
                    summary: None,
                    blocker: None,
                    error: Some("Run not found".into()),
                    allowed_actions: Vec::new(),
                    liveness: None,
                    diagnostics: None,
                    safe_actions: SafeActions::default(),
                }),
            );
        }
    };

    let (status, lifecycle_state) = match issue.runtime_state {
        ControlPlaneIssueRuntimeState::Idle => (RunStatus::Unclaimed, RunLifecycleState::Eligible),
        ControlPlaneIssueRuntimeState::Running => (RunStatus::Running, RunLifecycleState::Running),
        ControlPlaneIssueRuntimeState::Paused => (RunStatus::Paused, RunLifecycleState::Paused),
        ControlPlaneIssueRuntimeState::RetryQueued => {
            (RunStatus::RetryQueued, RunLifecycleState::Queued)
        }
        ControlPlaneIssueRuntimeState::Releasing => {
            (RunStatus::Released, RunLifecycleState::Releasing)
        }
        ControlPlaneIssueRuntimeState::Completed => {
            (RunStatus::Released, RunLifecycleState::Completed)
        }
        ControlPlaneIssueRuntimeState::Failed => (RunStatus::Released, RunLifecycleState::Failed),
    };

    let release_reason = match issue.last_outcome {
        ControlPlaneWorkerOutcome::Completed => Some(ReleaseReason::Completed),
        ControlPlaneWorkerOutcome::Canceled => Some(ReleaseReason::Cancelled),
        // When the snapshot indicates a failure and retries are exhausted
        // (retry_count > 0), treat it as RetryExhausted.  When the issue
        // failed on the first attempt with no retry queued, treat it as a
        // terminal tracker state rather than an exhausted-retry signal.
        ControlPlaneWorkerOutcome::Failed if issue.retry_count > 0 => {
            Some(ReleaseReason::RetryExhausted)
        }
        ControlPlaneWorkerOutcome::Failed => Some(ReleaseReason::TrackerTerminal),
        _ => None,
    };

    (
        StatusCode::OK,
        Json(RunDetail {
            schema_version: SchemaVersion::v1(),
            run_id: issue.identifier.clone(),
            issue_id: issue.identifier.clone(),
            issue_identifier: issue.identifier.clone(),
            worker_id: "default-worker".into(),
            status,
            lifecycle_state,
            // Use published timestamp so it does not drift on every event.
            claimed_at: envelope.published_at,
            // started_at is meaningful only when the run is actively running.
            started_at: matches!(
                issue.runtime_state,
                ControlPlaneIssueRuntimeState::Running | ControlPlaneIssueRuntimeState::Releasing
            )
            .then(|| envelope.published_at),
            // finished_at is set for terminal states using the snapshot timestamp
            // since the control plane does not yet track exact completion time.
            finished_at: matches!(
                issue.runtime_state,
                ControlPlaneIssueRuntimeState::Completed | ControlPlaneIssueRuntimeState::Failed
            )
            .then(|| envelope.published_at),
            release_reason,
            // retry_count and turn_count are distinct concepts; the snapshot
            // currently only tracks retries.
            turn_count: 0,
            max_turns: issue.retry_count.saturating_add(1).max(1),
            retry_attempt: (issue.retry_count > 0).then_some(issue.retry_count),
            input_tokens: issue.input_tokens,
            output_tokens: issue.output_tokens,
            cache_read_tokens: issue.cache_read_tokens,
            runtime_seconds: 0,
            // Emit conversation_id whenever a suffix is available regardless of
            // whether a server URL is configured.
            conversation_id: (!issue.conversation_id_suffix.is_empty())
                .then(|| format!("conv-{}", issue.conversation_id_suffix)),
            workspace_id: (!issue.workspace_path_suffix.is_empty())
                .then(|| issue.workspace_path_suffix.clone()),
            workspace_path: None,
            harness_type: issue.server_base_url.as_ref().map(|_| "openhands".into()),
            summary: None,
            blocker: issue.blocked.then(|| "Blocked by dependency".into()),
            error: None,
            allowed_actions: Vec::new(),
            liveness: None,
            diagnostics: None,
            safe_actions: SafeActions::default(),
        }),
    )
}

async fn get_run_events(
    State(store): State<SnapshotStore>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.current().await;
    let events: Vec<RunEvent> = match find_issue_snapshot(&envelope, &run_id) {
        Some(issue) => issue
            .recent_events
            .iter()
            .enumerate()
            .map(|(idx, evt)| RunEvent {
                sequence: idx as u64 + 1,
                event_id: evt.event_id.clone(),
                happened_at: evt.happened_at,
                kind: evt.kind.clone(),
                summary: evt.summary.clone(),
                payload: Some(json!({"kind": evt.kind})),
                raw_payload: Some(json!({"kind": evt.kind, "summary": evt.summary})),
            })
            .collect(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(RunEventPage {
                    schema_version: SchemaVersion::v1(),
                    run_id,
                    next_cursor: None,
                    events: Vec::new(),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(RunEventPage {
            schema_version: SchemaVersion::v1(),
            run_id,
            next_cursor: None,
            events,
        }),
    )
}

async fn get_run_files(
    State(store): State<SnapshotStore>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.current().await;
    let workspace_root = envelope.snapshot.daemon.workspace_root.clone();
    let files: Vec<ChangedFileEntry> = match find_issue_snapshot(&envelope, &run_id) {
        Some(issue) => issue
            .modified_files
            .iter()
            .map(|fc| ChangedFileEntry {
                path: sanitize_file_path(&workspace_root, &fc.path),
                change_kind: map_file_change_kind(fc.change_kind),
                lines_added: fc.lines_added,
                lines_removed: fc.lines_removed,
                size_bytes: None,
            })
            .collect(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(RunFilesPage {
                    schema_version: SchemaVersion::v1(),
                    run_id,
                    next_cursor: None,
                    files: Vec::new(),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(RunFilesPage {
            schema_version: SchemaVersion::v1(),
            run_id,
            next_cursor: None,
            files,
        }),
    )
}

async fn get_run_diffs(
    State(store): State<SnapshotStore>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.current().await;
    let workspace_root = envelope.snapshot.daemon.workspace_root.clone();
    let files: Vec<&ControlPlaneFileChange> = match find_issue_snapshot(&envelope, &run_id) {
        Some(issue) => issue.modified_files.iter().collect(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(FileDiffPage {
                    schema_version: SchemaVersion::v1(),
                    run_id,
                    file_path: String::new(),
                    next_cursor: None,
                    hunks: Vec::new(),
                    total_lines_added: 0,
                    total_lines_removed: 0,
                }),
            );
        }
    };

    // Build a summary hunk per changed file from the control-plane metadata.
    // Full unified diff content is not yet available from the snapshot;
    // the hunk header and line counts provide callers with change summaries.
    let hunks: Vec<DiffHunk> = files
        .iter()
        .map(|fc| {
            // Build a synthetic hunk header.  When a file is entirely new
            // (0 lines removed) or entirely deleted (0 lines added), the
            // header reflects the correct start,count pairs so parsers won't
            // choke on `@@ -1,0 +1,N @@` or `@@ -1,N +1,0 @@`.
            let old_start = if fc.lines_removed > 0 { 1 } else { 0 };
            let new_start = if fc.lines_added > 0 { 1 } else { 0 };
            DiffHunk {
                header: format!(
                    "@@ -{},{} +{},{} @@",
                    old_start, fc.lines_removed, new_start, fc.lines_added
                ),
                start_line: if fc.lines_removed > 0 { 1 } else { 0 },
                old_line_count: fc.lines_removed,
                new_line_count: fc.lines_added,
                lines: Vec::new(),
            }
        })
        .collect();

    let total_lines_added: u32 = files.iter().map(|f| f.lines_added).sum();
    let total_lines_removed: u32 = files.iter().map(|f| f.lines_removed).sum();

    // When multiple files are present, list all paths so the caller knows the
    // response is an aggregate rather than a single-file diff.
    let file_path = if files.len() == 1 {
        files
            .first()
            .map(|fc| sanitize_file_path(&workspace_root, &fc.path))
    } else {
        Some(format!("[{} files]", files.len()))
    };

    (
        StatusCode::OK,
        Json(FileDiffPage {
            schema_version: SchemaVersion::v1(),
            run_id,
            file_path: file_path.unwrap_or_default(),
            next_cursor: None,
            hunks,
            total_lines_added,
            total_lines_removed,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, StreamErrorType,
    };

    #[test]
    fn journal_error_mapping_preserves_invalid_cursor_sequence() {
        let err = JournalError::InvalidCursor {
            reason: "cursor is older than retained events".into(),
        };

        let stream_err = stream_error_from_journal_error(&err, 37);

        assert_eq!(stream_err.error_type, StreamErrorType::CursorNotFound);
        assert!(stream_err.message.contains("37"));
        assert!(stream_err.recoverable);
    }

    #[test]
    fn journal_error_mapping_keeps_backpressure_recoverable() {
        let err = JournalError::Backpressure { capacity: 100 };

        let stream_err = stream_error_from_journal_error(&err, 12);

        assert_eq!(stream_err.error_type, StreamErrorType::Backpressure);
        assert!(stream_err.recoverable);
    }

    #[test]
    fn serialize_stream_error_matches_flat_error_type_contract() {
        let json = serialize_stream_error(&StreamError::server_error("boom"));
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid json");

        assert_eq!(value["error_type"], "server_error");
        assert_eq!(value["message"], "boom");
        assert_eq!(value["recoverable"], false);
    }

    #[test]
    fn ws_error_frame_prefixes_stream_error_payload() {
        let frame = ws_error_frame(&StreamError::server_error("boom"));
        let payload = frame
            .strip_prefix("__error__ ")
            .expect("frame has error prefix");
        let value: serde_json::Value = serde_json::from_str(payload).expect("valid json");

        assert_eq!(value["error_type"], "server_error");
        assert_eq!(value["message"], "boom");
    }

    #[test]
    fn ws_event_frame_prefixes_event_payload() {
        let event = EventRecord::builder()
            .event_id("evt_ws_frame")
            .sequence(7)
            .actor(EventActor::system("test"))
            .kind(EventKind::RunStarted)
            .summary("frame test")
            .build();

        let frame = ws_event_frame(&event).expect("event serializes");
        let payload = frame
            .strip_prefix("__event__ ")
            .expect("frame has event prefix");
        let value: serde_json::Value = serde_json::from_str(payload).expect("valid json");

        assert_eq!(value["event_id"], "evt_ws_frame");
        assert_eq!(value["sequence"], 7);
    }

    #[test]
    fn web_asset_mime_table_is_the_extension_source_of_truth() {
        let mut seen = std::collections::BTreeSet::new();

        for (extension, mime) in KNOWN_ASSET_MIME_TYPES {
            assert!(!extension.is_empty(), "extension should not be empty");
            assert_ne!(*mime, "application/octet-stream");
            assert!(seen.insert(*extension), "duplicate extension: {extension}");
            assert!(path_has_known_extension(&format!("asset.{extension}")));
            assert_eq!(
                mime_type(StdPath::new(&format!("asset.{extension}"))),
                *mime
            );
        }

        assert!(path_has_known_extension("asset.MP4"));
        assert_eq!(mime_type(StdPath::new("asset.MP4")), "video/mp4");
        assert!(!path_has_known_extension("route/without-extension"));
        assert!(!path_has_known_extension("asset.unknown"));
        assert_eq!(
            mime_type(StdPath::new("asset.unknown")),
            "application/octet-stream"
        );
    }
}
