use std::{convert::Infallible, time::Duration};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::get,
};
use tokio::{net::TcpListener, sync::broadcast};

pub use crate::opensymphony_control::SnapshotStore;
pub use crate::opensymphony_domain::{
    ControlPlaneAgentServerStatus, ControlPlaneDaemonSnapshot, ControlPlaneDaemonState,
    ControlPlaneDaemonStatus, ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot,
    ControlPlaneMetricsSnapshot, ControlPlaneRecentEvent, ControlPlaneRecentEventKind,
    ControlPlaneWorkerOutcome, SnapshotEnvelope,
};
pub use crate::opensymphony_gateway_schema::{
    capability::{AuthMode, FeatureCapability, GatewayCapabilities, TransportCapability},
    snapshot::{
        DashboardSnapshot, GatewayHealth, GatewayMetrics, ProjectSummary, SnapshotEventKind,
        SnapshotEventSummary,
    },
    version::{GATEWAY_SCHEMA_VERSION, SchemaVersion},
};

const GATEWAY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// V1 gateway server that exposes stable public DTO endpoints
/// on top of the internal control-plane `SnapshotStore`.
#[derive(Debug, Clone)]
pub struct GatewayServer {
    store: SnapshotStore,
}

impl GatewayServer {
    pub fn new(store: SnapshotStore) -> Self {
        Self { store }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/v1/capabilities", get(capabilities))
            .route("/api/v1/dashboard/snapshot", get(dashboard_snapshot))
            .route("/api/v1/events", get(events))
            .with_state(self.store.clone())
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

async fn dashboard_snapshot(State(store): State<SnapshotStore>) -> Json<DashboardSnapshot> {
    let envelope = store.current().await;
    Json(control_plane_to_dashboard_snapshot(&envelope))
}

async fn events(
    State(store): State<SnapshotStore>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = store.subscribe();
    let initial = store.current().await;
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
