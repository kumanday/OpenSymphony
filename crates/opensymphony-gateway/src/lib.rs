use std::{
    convert::Infallible,
    path::{Path, Path as StdPath, PathBuf},
    time::Duration,
};

use chrono::Utc;
use serde_json::json;

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path as AxumPath, Path as ExtractPath, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    response::{IntoResponse, Response},
    routing::get,
};
use tokio::{net::TcpListener, sync::broadcast};

pub use crate::opensymphony_control::SnapshotStore;
pub use crate::opensymphony_domain::{
    ControlPlaneAgentServerStatus, ControlPlaneDaemonSnapshot, ControlPlaneDaemonState,
    ControlPlaneDaemonStatus, ControlPlaneFileChange, ControlPlaneFileChangeKind,
    ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot, ControlPlaneMetricsSnapshot,
    ControlPlaneRecentEvent, ControlPlaneRecentEventKind, ControlPlaneWorkerOutcome,
    SnapshotEnvelope,
};
pub use crate::opensymphony_gateway_schema::{
    capability::{AuthMode, FeatureCapability, GatewayCapabilities, TransportCapability},
    cursor::PageCursor,
    run::{
        ChangedFileEntry, DiffHunk, DiffLine, FileChangeKind, FileDiffPage, ReleaseReason,
        RunAction, RunDetail, RunEvent, RunEventPage, RunFilesPage, RunLifecycleState, RunStatus,
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

/// Combined state for the gateway router.
#[derive(Debug, Clone)]
struct GatewayState {
    store: SnapshotStore,
    /// Optional path to the built web app static assets directory.
    web_assets_dir: Option<String>,
}

/// V1 gateway server that exposes stable public DTO endpoints
/// on top of the internal control-plane `SnapshotStore`.
#[derive(Debug, Clone)]
pub struct GatewayServer {
    state: GatewayState,
}

impl GatewayServer {
    pub fn new(store: SnapshotStore) -> Self {
        Self {
            state: GatewayState {
                store,
                web_assets_dir: None,
            },
        }
    }

    /// Enable serving of the built web client from the given directory.
    pub fn with_web_assets(mut self, dir: impl Into<String>) -> Self {
        self.state.web_assets_dir = Some(dir.into());
        self
    }

    pub fn router(&self) -> Router {
        let mut router = Router::new()
            .route("/api/v1/capabilities", get(capabilities))
            .route("/api/v1/dashboard/snapshot", get(dashboard_snapshot))
            .route("/api/v1/events", get(events))
            .route("/api/v1/projects", get(list_projects))
            .route("/api/v1/projects/{project_id}", get(get_project))
            .route(
                "/api/v1/projects/{project_id}/taskgraph",
                get(get_task_graph),
            )
            .route("/api/v1/runs/{run_id}", get(get_run_detail))
            .route("/api/v1/runs/{run_id}/events", get(get_run_events))
            .route("/api/v1/runs/{run_id}/files", get(get_run_files))
            .route("/api/v1/runs/{run_id}/diffs", get(get_run_diffs));

        // Attach static web asset routes if configured.
        if self.state.web_assets_dir.is_some() {
            router = router
                .route("/app", get(web_asset_handler))
                .route("/app/", get(web_asset_handler))
                .route("/app/{*path}", get(web_asset_handler));
        }

        router.with_state(self.state.clone())
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

async fn dashboard_snapshot(State(store): State<GatewayState>) -> Json<DashboardSnapshot> {
    let envelope = store.store.current().await;
    Json(control_plane_to_dashboard_snapshot(&envelope))
}

async fn events(
    State(store): State<GatewayState>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mut receiver = store.store.subscribe();
    let initial = store.store.current().await;
    let stream = stream! {
        let mut last_sent_sequence = initial.sequence;
        yield Ok(snapshot_event(&initial));
        while let Some(envelope) =
            next_snapshot_envelope(&store.store, &mut receiver, &mut last_sent_sequence).await
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

fn map_file_change_kind(kind: ControlPlaneFileChangeKind) -> FileChangeKind {
    match kind {
        ControlPlaneFileChangeKind::Created => FileChangeKind::Created,
        ControlPlaneFileChangeKind::Modified => FileChangeKind::Modified,
        ControlPlaneFileChangeKind::Removed => FileChangeKind::Removed,
    }
}

// ── Project endpoints ─────────────────────────────────────────────────────────

async fn list_projects(State(store): State<GatewayState>) -> Json<ProjectList> {
    let envelope = store.store.current().await;
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
    State(store): State<GatewayState>,
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

    let envelope = store.store.current().await;
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
    State(store): State<GatewayState>,
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

    let envelope = store.store.current().await;
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
    State(store): State<GatewayState>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.store.current().await;
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
                }),
            );
        }
    };

    let (status, lifecycle_state) = match issue.runtime_state {
        ControlPlaneIssueRuntimeState::Idle => (RunStatus::Unclaimed, RunLifecycleState::Eligible),
        ControlPlaneIssueRuntimeState::Running => (RunStatus::Running, RunLifecycleState::Running),
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
        }),
    )
}

async fn get_run_events(
    State(store): State<GatewayState>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.store.current().await;
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
    State(store): State<GatewayState>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.store.current().await;
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
    State(store): State<GatewayState>,
    AxumPath(run_id): AxumPath<String>,
) -> impl IntoResponse {
    let envelope = store.store.current().await;
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

// ---------------------------------------------------------------------------

/// Resolve the requested path and verify it stays inside the assets directory.
/// Returns the resolved absolute path if safe, or `None` if the request is
/// outside the assets directory.
fn resolve_safe_path(assets_dir: &str, rest: &str) -> Option<PathBuf> {
    // Reject absolute paths early to avoid Path::new().join() discarding the base.
    // is_absolute() is cross-platform: it catches Unix (/) and Windows (C:) paths.
    if Path::new(rest).is_absolute() {
        return None;
    }

    let base = Path::new(assets_dir);
    let candidate = base.join(rest);
    match (candidate.canonicalize(), base.canonicalize()) {
        (Ok(resolved), Ok(base_resolved)) => {
            if resolved == base_resolved || resolved.starts_with(&base_resolved) {
                Some(resolved)
            } else {
                None
            }
        }
        // If canonicalize fails (file doesn't exist), do a static check.
        // Reject any path that contains ParentDir components (handles both
        // forward-slash and backslash separators cross-platform).
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

/// Serve `index.html` from the given assets directory, returning `None` if not found.
async fn serve_index_html(assets_dir: &str) -> Option<Response> {
    let index_path = Path::new(assets_dir).join("index.html");
    serve_file(&index_path).await.ok()
}

/// Serve a static file from the web assets directory, or fall back to
/// `index.html` for SPA routes.
async fn web_asset_handler(
    State(state): State<GatewayState>,
    path: Option<ExtractPath<String>>,
) -> Response {
    // If web assets are not configured, return 404.
    let assets_dir = match &state.web_assets_dir {
        Some(dir) => dir,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let rest = path.map(|p| p.0).unwrap_or_default();

    // If the path is empty (root /app/), serve index.html directly.
    if rest.is_empty() {
        return serve_index_html(assets_dir)
            .await
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response());
    }

    // Resolve the joined path and verify it stays inside the assets directory.
    let safe_path = match resolve_safe_path(assets_dir, &rest) {
        Some(p) => p,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    // Try the exact file path first.
    if safe_path.is_file() {
        return match serve_file(&safe_path).await {
            Ok(resp) => resp,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
    }

    // SPA fallback: if the path does not look like a static asset request,
    // serve index.html so client-side routing works.
    if !path_has_known_extension(&rest) {
        return serve_index_html(assets_dir)
            .await
            .unwrap_or_else(|| StatusCode::NOT_FOUND.into_response());
    }

    StatusCode::NOT_FOUND.into_response()
}

/// Return true if the URL path segment looks like a request for a known static
/// asset file.  Paths that do not match these extensions are treated as SPA
/// routes and should fall back to `index.html`.
fn path_has_known_extension(path: &str) -> bool {
    if let Some(dot_pos) = path.rfind('.')
        && let Some(ext) = path.get(dot_pos + 1..)
    {
        return matches!(
            ext.to_lowercase().as_str(),
            "html"
                | "css"
                | "js"
                | "json"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "svg"
                | "ico"
                | "woff"
                | "woff2"
                | "ttf"
                | "eot"
                | "otf"
                | "map"
                | "txt"
                | "xml"
                | "webp"
                | "mp4"
                | "webm"
                | "mp3"
                | "wav"
                | "flac"
                | "pdf"
                | "zip"
                | "gz"
                | "tar"
                | "bz2"
        );
    }
    false
}

/// Read a file from disk and return it as an HTTP response with the correct
/// content type.
async fn serve_file(path: &Path) -> Result<Response, std::io::Error> {
    let bytes = tokio::fs::read(path).await?;
    let content_type = mime_type(path);
    Ok(([(axum::http::header::CONTENT_TYPE, content_type)], bytes).into_response())
}

/// Return a conservative MIME type for the given file extension.
fn mime_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("eot") => "application/vnd.ms-fontobject",
        Some("otf") => "font/otf",
        Some("map") => "application/json; charset=utf-8",
        Some("txt") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml; charset=utf-8",
        Some(_) | None => "application/octet-stream",
    }
}
