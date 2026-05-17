use chrono::Utc;
use opensymphony::opensymphony_gateway_schema::{
    action::{ActionDispatch, ActionKind, ActionTarget},
    approval::{ActionReceipt, ActionReceiptStatus, ApprovalKind, ApprovalRequest, ApprovalStatus},
    capability::{AuthMode, FeatureCapability, GatewayCapabilities, TransportCapability},
    cursor::{PageCursor, StreamCursor},
    envelope::{EntityKind, EntityRef, GatewayEnvelope},
    planning::{
        PlanningArtifact, PlanningArtifactKind, PlanningSessionStatus, PlanningSessionSummary,
    },
    run::{ReleaseReason, RunDetail, RunEvent, RunEventPage, RunStatus},
    snapshot::{
        DashboardSnapshot, GatewayHealth, GatewayMetrics, ProjectSummary, SnapshotEventKind,
        SnapshotEventSummary,
    },
    task_graph::{TaskGraphNode, TaskGraphNodeKind, TaskGraphStateCategory},
    terminal::{TerminalEncoding, TerminalFrame, TerminalFrameKind, TerminalSnapshot},
    transport::{TransportProfile, TransportRecommendation},
    version::{GATEWAY_SCHEMA_VERSION, SchemaVersion},
};
use serde_json::json;

fn must_serialize<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("must serialize")
}

fn must_deserialize<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("must deserialize")
}

fn sample_schema_version() -> SchemaVersion {
    SchemaVersion::v1()
}

#[test]
fn schema_version_roundtrips() {
    let v = sample_schema_version();
    let json = must_serialize(&v);
    let back: SchemaVersion = must_deserialize(&json);
    assert_eq!(v, back);
    assert_eq!(v.as_str(), "1.0.0");
    assert_eq!(format!("{v}"), "1.0.0");
}

#[test]
fn gateway_schema_version_constant_matches() {
    assert_eq!(GATEWAY_SCHEMA_VERSION, "1.0.0");
}

#[test]
fn stream_cursor_roundtrips() {
    let cursor = StreamCursor::new(42, "events").with_timestamp_anchor(1_700_000_000);
    let json = must_serialize(&cursor);
    let back: StreamCursor = must_deserialize(&json);
    assert_eq!(cursor, back);
    assert!(json.contains("\"sequence\":42"));
    assert!(json.contains("\"partition\":\"events\""));
    assert!(json.contains("\"timestamp_anchor\":1700000000"));
}

#[test]
fn page_cursor_roundtrips() {
    let cursor = PageCursor::first(50);
    let json = must_serialize(&cursor);
    let back: PageCursor = must_deserialize(&json);
    assert_eq!(cursor, back);
}

#[test]
fn gateway_envelope_roundtrips_with_raw_payload() {
    let payload = json!({"content": "hello"});
    let envelope = GatewayEnvelope::new(
        StreamCursor::new(7, "terminal:run-1"),
        EntityRef::terminal("term-1"),
        "terminal_frame",
        payload,
    );
    let json = must_serialize(&envelope);
    let back: GatewayEnvelope = must_deserialize(&json);
    assert_eq!(back.schema_version, SchemaVersion::v1());
    assert_eq!(back.cursor.sequence, 7);
    assert_eq!(back.entity_ref.kind, EntityKind::TerminalSession);
    assert_eq!(back.entity_ref.id, "term-1");
    assert_eq!(back.event_kind, "terminal_frame");
    assert!(back.payload.is_some());
    assert!(back.raw_payload.is_some());
    assert_eq!(back.payload, back.raw_payload);
}

#[test]
fn gateway_envelope_from_raw_payload_roundtrips() {
    let envelope = GatewayEnvelope::from_raw_payload(
        StreamCursor::new(8, "unknown:run-2"),
        EntityRef::run("run-2"),
        "future_event",
        json!({"unknown_field": 42}),
    );
    let json = must_serialize(&envelope);
    let back: GatewayEnvelope = must_deserialize(&json);
    assert_eq!(back.schema_version, SchemaVersion::v1());
    assert_eq!(back.cursor.sequence, 8);
    assert_eq!(back.entity_ref.kind, EntityKind::Run);
    assert_eq!(back.entity_ref.id, "run-2");
    assert_eq!(back.event_kind, "future_event");
    assert!(back.payload.is_none());
    assert!(back.raw_payload.is_some());
    assert_eq!(back.raw_payload, Some(json!({"unknown_field": 42})));
}

#[test]
fn dashboard_snapshot_roundtrips() {
    let snapshot = DashboardSnapshot {
        schema_version: SchemaVersion::v1(),
        generated_at: Utc::now(),
        sequence: 1,
        health: GatewayHealth::Healthy,
        metrics: GatewayMetrics {
            running_issue_count: 2,
            retry_queue_depth: 0,
            total_input_tokens: 1024,
            total_output_tokens: 512,
            total_cache_read_tokens: 256,
            total_cost_micros: 0,
        },
        projects: vec![ProjectSummary {
            project_id: "proj-1".into(),
            name: "OpenSymphony".into(),
            milestone_count: 3,
            issue_count: 12,
            running_count: 2,
            completed_count: 5,
            failed_count: 0,
        }],
        recent_events: vec![SnapshotEventSummary {
            sequence: 1,
            happened_at: Utc::now(),
            issue_identifier: Some("COE-390".into()),
            kind: SnapshotEventKind::WorkerStarted,
            summary: "Run started".into(),
        }],
    };
    let json = must_serialize(&snapshot);
    let back: DashboardSnapshot = must_deserialize(&json);
    assert_eq!(back.schema_version, SchemaVersion::v1());
    assert_eq!(back.sequence, 1);
    assert_eq!(back.health, GatewayHealth::Healthy);
    assert_eq!(back.projects.len(), 1);
    assert_eq!(back.projects[0].project_id, "proj-1");
}

#[test]
fn task_graph_node_roundtrips() {
    let node = TaskGraphNode {
        schema_version: SchemaVersion::v1(),
        node_id: "node-1".into(),
        kind: TaskGraphNodeKind::Issue,
        identifier: "COE-390".into(),
        title: "Gateway Schemas".into(),
        state: "In Progress".into(),
        state_category: TaskGraphStateCategory::InProgress,
        priority: Some(1),
        parent_id: Some("milestone-1".into()),
        children: vec!["sub-1".into()],
        blocked_by: vec![],
        url: Some("https://linear.app/trilogy-ai-coe/issue/COE-390".into()),
        branch_name: Some("leonardogonzalez/coe-390".into()),
        labels: vec!["foundation".into(), "contracts".into()],
        created_at: Some(Utc::now()),
        updated_at: Some(Utc::now()),
        estimate_minutes: Some(300),
    };
    let json = must_serialize(&node);
    let back: TaskGraphNode = must_deserialize(&json);
    assert_eq!(back.node_id, "node-1");
    assert_eq!(back.kind, TaskGraphNodeKind::Issue);
    assert_eq!(back.state_category, TaskGraphStateCategory::InProgress);
    assert_eq!(back.children.len(), 1);
}

#[test]
fn run_detail_roundtrips() {
    let run = RunDetail {
        schema_version: SchemaVersion::v1(),
        run_id: "run-1".into(),
        issue_id: "issue-1".into(),
        issue_identifier: "COE-390".into(),
        worker_id: "worker-1".into(),
        status: RunStatus::Running,
        claimed_at: Utc::now(),
        started_at: Some(Utc::now()),
        finished_at: None,
        release_reason: None,
        turn_count: 3,
        max_turns: 8,
        retry_attempt: None,
        input_tokens: 1024,
        output_tokens: 512,
        cache_read_tokens: 256,
        runtime_seconds: 120,
        conversation_id: Some("conv-1".into()),
        workspace_path: Some("/tmp/workspaces/COE-390".into()),
        error: None,
    };
    let json = must_serialize(&run);
    let back: RunDetail = must_deserialize(&json);
    assert_eq!(back.status, RunStatus::Running);
    assert_eq!(back.issue_identifier, "COE-390");
    assert_eq!(back.turn_count, 3);
}

#[test]
fn run_event_page_roundtrips() {
    let page = RunEventPage {
        schema_version: SchemaVersion::v1(),
        run_id: "run-1".into(),
        next_cursor: Some(PageCursor {
            page_token: "page-2".into(),
            page_size: 50,
        }),
        events: vec![RunEvent {
            sequence: 1,
            event_id: "evt-1".into(),
            happened_at: Utc::now(),
            kind: "ConversationStateUpdateEvent".into(),
            summary: "ready".into(),
            payload: Some(json!({"execution_status":"ready"})),
            raw_payload: Some(json!({"execution_status":"ready"})),
        }],
    };
    let json = must_serialize(&page);
    let back: RunEventPage = must_deserialize(&json);
    assert_eq!(back.events.len(), 1);
    assert_eq!(back.events[0].sequence, 1);
    assert!(back.next_cursor.is_some());
}

#[test]
fn terminal_frame_roundtrips() {
    let frame = TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: 1,
        stream_id: "stream-1".into(),
        run_id: "run-1".into(),
        terminal_session_id: "term-1".into(),
        frame_kind: TerminalFrameKind::Stdout,
        encoding: TerminalEncoding::Utf8,
        content: "hello world\n".into(),
        timestamp: Utc::now(),
    };
    let json = must_serialize(&frame);
    let back: TerminalFrame = must_deserialize(&json);
    assert_eq!(back.frame_sequence, 1);
    assert_eq!(back.frame_kind, TerminalFrameKind::Stdout);
    assert_eq!(back.encoding, TerminalEncoding::Utf8);
    assert_eq!(back.content, "hello world\n");
}

#[test]
fn terminal_snapshot_roundtrips() {
    let snapshot = TerminalSnapshot {
        schema_version: SchemaVersion::v1(),
        terminal_session_id: "term-1".into(),
        run_id: "run-1".into(),
        frames: vec![],
        total_frames: 0,
        truncated: false,
        cursor: 0,
    };
    let json = must_serialize(&snapshot);
    let back: TerminalSnapshot = must_deserialize(&json);
    assert!(!back.truncated);
    assert_eq!(back.total_frames, 0);
}

#[test]
fn approval_request_roundtrips() {
    let req = ApprovalRequest {
        schema_version: SchemaVersion::v1(),
        approval_id: "apr-1".into(),
        run_id: "run-1".into(),
        issue_id: "issue-1".into(),
        kind: ApprovalKind::ToolUse,
        title: "Approve file write".into(),
        description: "Agent wants to write to src/main.rs".into(),
        proposed_action: Some(json!({"path": "src/main.rs", "content": "fn main() {}"})),
        requested_at: Utc::now(),
        expires_at: None,
        status: ApprovalStatus::Pending,
        correlation_id: "corr-1".into(),
    };
    let json = must_serialize(&req);
    let back: ApprovalRequest = must_deserialize(&json);
    assert_eq!(back.kind, ApprovalKind::ToolUse);
    assert_eq!(back.status, ApprovalStatus::Pending);
    assert_eq!(back.correlation_id, "corr-1");
}

#[test]
fn action_receipt_roundtrips() {
    let receipt = ActionReceipt {
        schema_version: SchemaVersion::v1(),
        action_id: "act-1".into(),
        correlation_id: "corr-1".into(),
        status: ActionReceiptStatus::Accepted,
        reason: None,
        expected_events: vec!["snapshot_updated".into(), "run_started".into()],
        result: Some(json!({"run_id": "run-1"})),
        issued_at: Utc::now(),
    };
    let json = must_serialize(&receipt);
    let back: ActionReceipt = must_deserialize(&json);
    assert_eq!(back.status, ActionReceiptStatus::Accepted);
    assert_eq!(back.expected_events.len(), 2);
}

#[test]
fn planning_artifact_roundtrips() {
    let artifact = PlanningArtifact {
        schema_version: SchemaVersion::v1(),
        artifact_id: "art-1".into(),
        session_id: "sess-1".into(),
        kind: PlanningArtifactKind::MilestoneDraft,
        title: "M1: Gateway Contract".into(),
        content: "# Milestone\n\nDraft gateway schemas.".into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        generated_by: Some("planning-agent".into()),
        approved: false,
        published_to_tracker: false,
    };
    let json = must_serialize(&artifact);
    let back: PlanningArtifact = must_deserialize(&json);
    assert_eq!(back.kind, PlanningArtifactKind::MilestoneDraft);
    assert!(!back.approved);
}

#[test]
fn planning_session_summary_roundtrips() {
    let sess = PlanningSessionSummary {
        schema_version: SchemaVersion::v1(),
        session_id: "sess-1".into(),
        project_id: "proj-1".into(),
        title: "Q3 Planning".into(),
        status: PlanningSessionStatus::Draft,
        artifact_count: 3,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let json = must_serialize(&sess);
    let back: PlanningSessionSummary = must_deserialize(&json);
    assert_eq!(back.status, PlanningSessionStatus::Draft);
}

#[test]
fn gateway_capabilities_roundtrips() {
    let caps = GatewayCapabilities {
        schema_version: SchemaVersion::v1(),
        gateway_version: "1.6.0".into(),
        supported_api_versions: vec!["1.0.0".into()],
        transports: vec![TransportCapability {
            transport: "websocket".into(),
            modes: vec!["json".into(), "binary".into()],
            supported_encodings: vec!["utf-8".into(), "base64".into()],
            bidirectional: true,
        }],
        features: vec![FeatureCapability {
            feature: "task_graph".into(),
            available: true,
            requires_auth: false,
            requires_plan: None,
        }],
        auth_modes: vec![AuthMode::None, AuthMode::ApiKey],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    };
    let json = must_serialize(&caps);
    let back: GatewayCapabilities = must_deserialize(&json);
    assert_eq!(back.gateway_version, "1.6.0");
    assert_eq!(back.max_event_page_size, 1000);
    assert_eq!(back.auth_modes.len(), 2);
}

#[test]
fn action_dispatch_roundtrips() {
    let action = ActionDispatch {
        schema_version: SchemaVersion::v1(),
        correlation_id: "corr-1".into(),
        action_kind: ActionKind::Retry,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Run,
            entity_id: "run-1".into(),
        },
        payload: None,
        idempotency_key: Some("idem-1".into()),
    };
    let json = must_serialize(&action);
    let back: ActionDispatch = must_deserialize(&json);
    assert_eq!(back.action_kind, ActionKind::Retry);
    assert_eq!(back.correlation_id, "corr-1");
    assert_eq!(back.idempotency_key, Some("idem-1".into()));
}

#[test]
fn transport_recommendation_roundtrips() {
    let rec = TransportRecommendation {
        profile: TransportProfile::InProcessChannel,
        priority: 1,
        description: "Fastest local path".into(),
        expected_latency_ms: 0,
        expected_throughput_kbps: 1_000_000,
        reconnect_support: false,
        replay_support: false,
        binary_frame_support: true,
        auth_required: false,
    };
    let json = must_serialize(&rec);
    let back: TransportRecommendation = must_deserialize(&json);
    assert_eq!(back.profile, TransportProfile::InProcessChannel);
    assert_eq!(back.priority, 1);
}

#[test]
fn entity_ref_helpers_work() {
    let issue = EntityRef::issue("issue-1", Some("COE-390".into()));
    assert_eq!(issue.kind, EntityKind::Issue);
    assert_eq!(issue.id, "issue-1");
    assert_eq!(issue.identifier, Some("COE-390".into()));

    let run = EntityRef::run("run-1");
    assert_eq!(run.kind, EntityKind::Run);
    assert_eq!(run.identifier, None);
}

#[test]
fn all_schema_modules_compile_and_export() {
    // This test exists primarily as a compile-time gate.
    let _ = SchemaVersion::v1();
    let _ = GatewayHealth::Healthy;
    let _ = TaskGraphNodeKind::Issue;
    let _ = RunStatus::Running;
    let _ = TerminalFrameKind::Stdout;
    let _ = ApprovalKind::ToolUse;
    let _ = PlanningArtifactKind::MilestoneDraft;
    let _ = TransportProfile::WebSocket;
    let _ = AuthMode::BearerToken;
    let _ = ActionReceiptStatus::Accepted;
    let _ = SnapshotEventKind::WorkerStarted;
    let _ = TaskGraphStateCategory::InProgress;
    let _ = ReleaseReason::Completed;
    let _ = TerminalEncoding::Utf8;
    let _ = PlanningSessionStatus::Draft;
}
