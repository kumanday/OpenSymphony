use chrono::Utc;
use opensymphony::opensymphony_gateway_schema::{
    action::{ActionDispatch, ActionKind, ActionReceipt, ActionStatus, ActionTarget},
    approval::{ApprovalKind, ApprovalRequest, ApprovalStatus},
    capability::{
        AuthMode, FeatureCapability, GatewayCapabilities, HarnessCapability, TransportCapability,
    },
    cursor::{PageCursor, StreamCursor},
    envelope::{EntityKind, EntityRef, GatewayEnvelope},
    model_settings::{
        CredentialStatusKind, CredentialStatusResponse, CredentialStorageMode,
        ModelSettingsResponse,
    },
    planning::{
        ArtifactDiff, ArtifactRevision, ConversationTurn, LinearPublishReceipt, PlanningArtifact,
        PlanningArtifactKind, PlanningSession, PlanningSessionStatus, PlanningSessionSummary,
        PlanningWave, PublishedMilestone, PublishedTask, ReviewComment, TaskEntry,
        TaskPackageProjection, TurnRole,
    },
    run::{
        HarnessSchedulerDisagreement, ReleaseReason, RunDetail, RunEvent, RunEventPage,
        RunLifecycleState, RunLivenessEnvelope, RunPhase, RunProgress, RunStatus,
        RunStreamLiveness, SafeActions,
    },
    snapshot::{
        DashboardSnapshot, GatewayHealth, GatewayMetrics, ProjectSummary, SnapshotEventKind,
        SnapshotEventSummary,
    },
    task_graph::{TaskGraphNode, TaskGraphNodeKind, TaskGraphStateCategory},
    terminal::{
        TerminalEncoding, TerminalFrame, TerminalFrameKind, TerminalLogAssociation,
        TerminalSnapshot,
    },
    transport::{TransportProfile, TransportRecommendation},
    version::{GATEWAY_SCHEMA_VERSION, SchemaVersion},
};
use serde_json::{Value, json};

fn must_serialize<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("must serialize")
}

fn must_deserialize<T: serde::de::DeserializeOwned>(json: &str) -> T {
    serde_json::from_str(json).expect("must deserialize")
}

fn assert_no_raw_secret_field_names(value: &Value) {
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                let key = key.to_ascii_lowercase();
                assert_ne!(key, "value", "raw value field must not be serialized");
                assert!(
                    !key.contains("token"),
                    "token-bearing field must not be serialized"
                );
                assert!(
                    !key.contains("secret"),
                    "secret-bearing field must not be serialized"
                );
                assert_no_raw_secret_field_names(nested);
            }
        }
        Value::Array(values) => {
            for nested in values {
                assert_no_raw_secret_field_names(nested);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
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
fn model_settings_roundtrip_and_redact_secret_material() {
    let settings = ModelSettingsResponse::local_default(false);
    let json = must_serialize(&settings);
    let back: ModelSettingsResponse = must_deserialize(&json);

    assert_eq!(back.schema_version, sample_schema_version());
    assert!(back.profiles.iter().any(|profile| {
        profile.id == "openhands-env-api-key"
            && profile.credential_reference.reference == "LLM_API_KEY"
            && profile.model.reference == "LLM_MODEL"
            && profile
                .base_url
                .as_ref()
                .is_some_and(|base_url| base_url.reference == "LLM_BASE_URL")
            && profile.compatible_harnesses == vec!["openhands_agent_server"]
            && profile.status == CredentialStatusKind::LoggedOut
    }));
    assert!(back.profiles.iter().any(|profile| {
        profile.storage_mode == CredentialStorageMode::HostedBroker
            && profile.credential_reference.redacted
            && profile.status == CredentialStatusKind::Unsupported
    }));
    assert!(back.profiles.iter().any(|profile| {
        profile.storage_mode == CredentialStorageMode::LocalKeychain
            && profile.credential_reference.redacted
            && profile.compatible_harnesses == vec!["codex_app_server"]
    }));
    assert_no_raw_secret_field_names(
        &serde_json::to_value(&back.profiles).expect("profiles serialize as JSON value"),
    );
    assert!(
        back.supported_credential_statuses
            .contains(&CredentialStatusKind::Installed)
    );
    assert!(
        back.supported_credential_statuses
            .contains(&CredentialStatusKind::Expired)
    );
    assert!(
        back.supported_credential_statuses
            .contains(&CredentialStatusKind::PermissionDenied)
    );
    assert!(!json.contains("sk-live-secret"));
    assert!(!json.contains("refresh_token"));
}

#[test]
fn credential_status_response_supports_ui_status_states() {
    let settings = ModelSettingsResponse::local_default(true);
    let statuses = CredentialStatusResponse::from_model_settings(&settings);
    let json = must_serialize(&statuses);
    let back: CredentialStatusResponse = must_deserialize(&json);

    assert_eq!(back.schema_version, sample_schema_version());
    assert!(
        back.statuses
            .iter()
            .any(
                |status| status.credential_reference_id == "credential:env:LLM_API_KEY"
                    && status.status == CredentialStatusKind::Installed
            )
    );
    assert_eq!(
        back.supported_statuses,
        vec![
            CredentialStatusKind::Installed,
            CredentialStatusKind::LoggedOut,
            CredentialStatusKind::Expired,
            CredentialStatusKind::Unsupported,
            CredentialStatusKind::PermissionDenied,
            CredentialStatusKind::Unknown,
        ]
    );
    assert!(json.contains("\"installed\""));
    assert!(json.contains("\"permission_denied\""));
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
        runtime_overlay: None,
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
        lifecycle_state: RunLifecycleState::Running,
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
        workspace_id: Some("COE-390".into()),
        harness_type: Some("openhands".into()),
        summary: Some("Processing run".into()),
        blocker: None,
        error: None,
        allowed_actions: vec![],
        liveness: None,
        diagnostics: None,
        safe_actions: SafeActions::default(),
        detached: false,
        cancel_acknowledged: false,
        cancel_failed: false,
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
        association: TerminalLogAssociation {
            run_id: "run-1".into(),
            workspace_id: "workspace-1".into(),
            command_id: None,
            issue_id: None,
            sub_issue_id: None,
            harness_session_id: None,
        },
        correlation_id: None,
        source_event_id: Some("evt-1".into()),
        frame_id: Some("fid-1".into()),
    };
    let json = must_serialize(&frame);
    let back: TerminalFrame = must_deserialize(&json);
    assert_eq!(back.frame_sequence, 1);
    assert_eq!(back.frame_kind, TerminalFrameKind::Stdout);
    assert_eq!(back.encoding, TerminalEncoding::Utf8);
    assert_eq!(back.content, "hello world\n");
    assert_eq!(back.source_event_id.as_deref(), Some("evt-1"));
    assert_eq!(back.frame_id.as_deref(), Some("fid-1"));
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
        session: None,
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
        actor: None,
        target_context: None,
        risk_summary: None,
        requested_at: Utc::now(),
        expires_at: None,
        status: ApprovalStatus::Pending,
        correlation_id: "corr-1".into(),
        decided_at: None,
    };
    let json = must_serialize(&req);
    let back: ApprovalRequest = must_deserialize(&json);
    assert_eq!(back.kind, ApprovalKind::ToolUse);
    assert_eq!(back.status, ApprovalStatus::Pending);
    assert_eq!(back.correlation_id, "corr-1");
}

#[test]
fn action_receipt_roundtrips() {
    let receipt =
        ActionReceipt::accepted("act-1".to_string(), "corr-1".to_string(), ActionKind::Retry);
    let json = must_serialize(&receipt);
    let back: ActionReceipt = must_deserialize(&json);
    assert_eq!(back.status, ActionStatus::Accepted);
    assert_eq!(back.correlation_id, "corr-1");
    assert_eq!(back.action_id, "act-1");
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
        planning_wave: None,
        turn_count: 0,
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
        harnesses: vec![HarnessCapability::openhands_agent_server()],
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
    assert_eq!(back.harnesses[0].kind, "openhands_agent_server");
    assert!(back.harnesses[0].history.reconnect_and_replay);
}

#[test]
fn harness_capability_roundtrips_future_adapters() {
    let caps = vec![
        HarnessCapability::openhands_agent_server(),
        HarnessCapability::codex_app_server_future(),
        HarnessCapability::rust_native_future(),
    ];

    let json = must_serialize(&caps);
    let back: Vec<HarnessCapability> = must_deserialize(&json);

    assert_eq!(back.len(), 3);
    assert!(back[0].available);
    assert_eq!(back[1].kind, "codex_app_server");
    assert_eq!(back[1].transport.protocol, "json_rpc_2_0");
    assert!(back[1].approvals.tool_approval);
    assert_eq!(back[2].kind, "rust_native");
    assert!(back[2].pause_resume.pause);
}

#[test]
fn fake_harness_optional_capability_combinations_roundtrip() {
    let mut fake = HarnessCapability::rust_native_future();
    fake.kind = "fake_minimal".into();
    fake.display_name = "Fake minimal harness".into();
    fake.available = true;
    fake.actions.pause = false;
    fake.actions.resume = false;
    fake.approvals.tool_approval = false;
    fake.approvals.human_decision = false;
    fake.model_settings.subscription_credentials = false;
    fake.model_settings.credential_reference_kinds.clear();
    fake.event_streams.delivery_modes = vec!["test_fixture".into()];
    fake.feature_gaps = vec!["No approval or pause/resume support in this fake harness.".into()];

    let json = must_serialize(&fake);
    let back: HarnessCapability = must_deserialize(&json);

    assert_eq!(back.kind, "fake_minimal");
    assert!(!back.actions.pause);
    assert!(!back.approvals.tool_approval);
    assert!(back.history.preserve_unknown_events);
    assert_eq!(back.event_streams.delivery_modes, vec!["test_fixture"]);
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
    let _ = ActionStatus::Accepted;
    let _ = SnapshotEventKind::WorkerStarted;
    let _ = TaskGraphStateCategory::InProgress;
    let _ = ReleaseReason::Completed;
    let _ = TerminalEncoding::Utf8;
    let _ = PlanningSessionStatus::Draft;
}

// ---------------------------------------------------------------------------
// Planning artifact kind exhaustive roundtrip
// ---------------------------------------------------------------------------

#[test]
fn all_planning_artifact_kinds_roundtrip() {
    let kinds = [
        PlanningArtifactKind::Intake,
        PlanningArtifactKind::ProjectContext,
        PlanningArtifactKind::Requirements,
        PlanningArtifactKind::ResearchBrief,
        PlanningArtifactKind::CodebaseAnalysis,
        PlanningArtifactKind::ArchitectureNotes,
        PlanningArtifactKind::RiskRegister,
        PlanningArtifactKind::MilestoneDraft,
        PlanningArtifactKind::IssueDraft,
        PlanningArtifactKind::SubIssueDraft,
        PlanningArtifactKind::DependencyMap,
        PlanningArtifactKind::VerificationPlan,
        PlanningArtifactKind::AcceptanceCriteria,
        PlanningArtifactKind::PlanValidation,
        PlanningArtifactKind::LinearDraft,
        PlanningArtifactKind::ReviewComments,
        PlanningArtifactKind::PublishReceipt,
        PlanningArtifactKind::PlanningWave,
    ];

    assert_eq!(kinds.len(), 18, "expected exactly 18 artifact kinds");

    for kind in kinds {
        let artifact = PlanningArtifact {
            schema_version: SchemaVersion::v1(),
            artifact_id: "art-kind-test".into(),
            session_id: "sess-1".into(),
            kind,
            title: format!("{:?}", kind),
            content: "test content".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            generated_by: Some("test".into()),
            approved: false,
            published_to_tracker: false,
        };
        let json = must_serialize(&artifact);
        let back: PlanningArtifact = must_deserialize(&json);
        assert_eq!(back.kind, kind, "kind {:?} did not roundtrip", kind);
    }
}

// ---------------------------------------------------------------------------
// Planning session full roundtrip
// ---------------------------------------------------------------------------

#[test]
fn planning_session_full_roundtrip() {
    let session = PlanningSession {
        schema_version: SchemaVersion::v1(),
        session_id: "sess-1".into(),
        project_id: "proj-1".into(),
        title: "Q4 Planning".into(),
        status: PlanningSessionStatus::Draft,
        planning_wave: Some("rich-client-hosted-mode".into()),
        created_by: Some("alice".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        turns: vec![ConversationTurn {
            turn_id: "turn-1".into(),
            session_id: "sess-1".into(),
            turn_number: 1,
            role: TurnRole::User,
            content: "Let's plan the rich client.".into(),
            created_at: Utc::now(),
            artifacts_modified: vec!["art-1".into()],
            metadata: None,
        }],
        artifacts: vec![PlanningArtifact {
            schema_version: SchemaVersion::v1(),
            artifact_id: "art-1".into(),
            session_id: "sess-1".into(),
            kind: PlanningArtifactKind::Intake,
            title: "Intake".into(),
            content: "Build a rich client.".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            generated_by: Some("agent".into()),
            approved: false,
            published_to_tracker: false,
        }],
        metadata: None,
    };
    let json = must_serialize(&session);
    let back: PlanningSession = must_deserialize(&json);
    assert_eq!(back.session_id, "sess-1");
    assert_eq!(back.planning_wave, Some("rich-client-hosted-mode".into()));
    assert_eq!(back.turns.len(), 1);
    assert_eq!(back.artifacts.len(), 1);
    assert_eq!(back.artifacts[0].kind, PlanningArtifactKind::Intake);
}

// ---------------------------------------------------------------------------
// Planning session summary derivation
// ---------------------------------------------------------------------------

#[test]
fn planning_session_summary_is_derived_correctly() {
    let session = PlanningSession {
        schema_version: SchemaVersion::v1(),
        session_id: "sess-1".into(),
        project_id: "proj-1".into(),
        title: "Planning".into(),
        status: PlanningSessionStatus::InReview,
        planning_wave: Some("wave-1".into()),
        created_by: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        turns: vec![
            ConversationTurn {
                turn_id: "t1".into(),
                session_id: "sess-1".into(),
                turn_number: 1,
                role: TurnRole::User,
                content: "hi".into(),
                created_at: Utc::now(),
                artifacts_modified: vec![],
                metadata: None,
            },
            ConversationTurn {
                turn_id: "t2".into(),
                session_id: "sess-1".into(),
                turn_number: 2,
                role: TurnRole::Agent,
                content: "ok".into(),
                created_at: Utc::now(),
                artifacts_modified: vec![],
                metadata: None,
            },
        ],
        artifacts: vec![
            PlanningArtifact {
                schema_version: SchemaVersion::v1(),
                artifact_id: "a1".into(),
                session_id: "sess-1".into(),
                kind: PlanningArtifactKind::Intake,
                title: "Intake".into(),
                content: "c".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                generated_by: None,
                approved: false,
                published_to_tracker: false,
            },
            PlanningArtifact {
                schema_version: SchemaVersion::v1(),
                artifact_id: "a2".into(),
                session_id: "sess-1".into(),
                kind: PlanningArtifactKind::Requirements,
                title: "Requirements".into(),
                content: "c".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                generated_by: None,
                approved: false,
                published_to_tracker: false,
            },
        ],
        metadata: None,
    };

    let summary = session.summary();
    assert_eq!(summary.turn_count, 2);
    assert_eq!(summary.artifact_count, 2);
    assert_eq!(summary.planning_wave, Some("wave-1".into()));
    assert_eq!(summary.status, PlanningSessionStatus::InReview);
}

// ---------------------------------------------------------------------------
// Artifact revision and diff roundtrips
// ---------------------------------------------------------------------------

#[test]
fn artifact_revision_roundtrips() {
    let rev = ArtifactRevision {
        revision_id: "rev-1".into(),
        artifact_id: "art-1".into(),
        version: 3,
        content_hash: "sha256-abc".into(),
        content: "revised content".into(),
        created_at: Utc::now(),
        authored_by: Some("agent".into()),
        change_summary: Some("Updated milestone timeline".into()),
    };
    let json = must_serialize(&rev);
    let back: ArtifactRevision = must_deserialize(&json);
    assert_eq!(back.version, 3);
    assert_eq!(back.content_hash, "sha256-abc");
    assert!(back.change_summary.is_some());
}

#[test]
fn artifact_diff_roundtrips() {
    let diff = ArtifactDiff {
        diff_id: "diff-1".into(),
        artifact_id: "art-1".into(),
        from_version: 2,
        to_version: 3,
        unified_diff: "--- a\n+++ b\n@@ -1 +1 @@\n-old\n+new\n".into(),
        lines_added: 1,
        lines_removed: 1,
        summary: Some("Updated milestone timeline".into()),
        generated_at: Utc::now(),
    };
    let json = must_serialize(&diff);
    let back: ArtifactDiff = must_deserialize(&json);
    assert_eq!(back.from_version, 2);
    assert_eq!(back.to_version, 3);
    assert_eq!(back.lines_added, 1);
}

#[test]
fn review_comment_roundtrips() {
    let comment = ReviewComment {
        comment_id: "rc-1".into(),
        session_id: "sess-1".into(),
        artifact_id: "art-1".into(),
        revision_id: Some("rev-1".into()),
        author: "reviewer-1".into(),
        body: "Milestone timeline seems optimistic.".into(),
        resolved: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let json = must_serialize(&comment);
    let back: ReviewComment = must_deserialize(&json);
    assert_eq!(back.author, "reviewer-1");
    assert!(!back.resolved);
}

#[test]
fn conversation_turn_roundtrips() {
    let turn = ConversationTurn {
        turn_id: "turn-5".into(),
        session_id: "sess-1".into(),
        turn_number: 5,
        role: TurnRole::Agent,
        content: "I've generated the intake artifact.".into(),
        created_at: Utc::now(),
        artifacts_modified: vec!["art-1".into(), "art-2".into()],
        metadata: Some(json!({"tokens": 128})),
    };
    let json = must_serialize(&turn);
    let back: ConversationTurn = must_deserialize(&json);
    assert_eq!(back.role, TurnRole::Agent);
    assert_eq!(back.artifacts_modified.len(), 2);
}

#[test]
fn conversation_turn_empty_metadata_omitted() {
    let turn = ConversationTurn {
        turn_id: "turn-6".into(),
        session_id: "sess-1".into(),
        turn_number: 6,
        role: TurnRole::System,
        content: "Session created.".into(),
        created_at: Utc::now(),
        artifacts_modified: vec![],
        metadata: None,
    };
    let json = must_serialize(&turn);
    assert!(!json.contains("\"metadata\""));
    assert!(!json.contains("\"artifacts_modified\""));
}

// ─── Planning Session ─────────────────────────────────────────────────────

fn sample_planning_session() -> PlanningSession {
    PlanningSession {
        schema_version: SchemaVersion::v1(),
        session_id: "sess-1".into(),
        project_id: "proj-1".into(),
        title: "Q3 Planning Session".into(),
        status: PlanningSessionStatus::Draft,
        planning_wave: Some("rich-client-hosted-mode".into()),
        created_by: Some("agent-planner".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        turns: vec![ConversationTurn {
            turn_id: "turn-1".into(),
            session_id: "sess-1".into(),
            turn_number: 1,
            role: TurnRole::Agent,
            content: "Starting planning session.".into(),
            created_at: Utc::now(),
            artifacts_modified: vec![],
            metadata: None,
        }],
        artifacts: vec![PlanningArtifact {
            schema_version: SchemaVersion::v1(),
            artifact_id: "art-1".into(),
            session_id: "sess-1".into(),
            kind: PlanningArtifactKind::MilestoneDraft,
            title: "M1: Gateway Contract".into(),
            content: "Draft gateway schemas.".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            generated_by: Some("planner".into()),
            approved: false,
            published_to_tracker: false,
        }],
        metadata: None,
    }
}

#[test]
fn planning_session_roundtrips() {
    let session = sample_planning_session();
    let json = must_serialize(&session);
    let back: PlanningSession = must_deserialize(&json);
    assert_eq!(back.session_id, "sess-1");
    assert_eq!(back.status, PlanningSessionStatus::Draft);
    assert_eq!(
        back.planning_wave.as_deref(),
        Some("rich-client-hosted-mode")
    );
    assert_eq!(back.turns.len(), 1);
    assert_eq!(back.artifacts.len(), 1);
}

#[test]
fn planning_session_render_review_markdown_contains_artifacts() {
    let session = sample_planning_session();
    let markdown = session.render_review_markdown();
    assert!(markdown.contains("# Planning Review"));
    assert!(markdown.contains("**Session:** Q3 Planning Session"));
    assert!(markdown.contains("## Artifacts"));
    assert!(markdown.contains("M1: Gateway Contract"));
}

#[test]
fn planning_session_render_prompt_context_includes_wave() {
    let session = sample_planning_session();
    let ctx = session.render_prompt_context();
    assert!(ctx.contains("[Session: Q3 Planning Session]"));
    assert!(ctx.contains("[Wave: rich-client-hosted-mode]"));
    assert!(ctx.contains("[Artifact: milestone_draft]"));
}

#[test]
fn planning_session_render_audit_history_lists_turns() {
    let session = sample_planning_session();
    let audit = session.render_audit_history();
    assert!(audit.contains("# Audit History"));
    assert!(audit.contains("[agent] turn=1"));
}

#[test]
fn planning_session_empty_render_is_valid() {
    let session = PlanningSession {
        schema_version: SchemaVersion::v1(),
        session_id: "sess-empty".into(),
        project_id: "proj-1".into(),
        title: "Empty Session".into(),
        status: PlanningSessionStatus::Draft,
        planning_wave: None,
        created_by: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        turns: vec![],
        artifacts: vec![],
        metadata: None,
    };
    let markdown = session.render_review_markdown();
    assert!(markdown.contains("# Planning Review"));
    assert!(!markdown.contains("## Artifacts"));
    assert!(!markdown.contains("## Conversation"));

    let ctx = session.render_prompt_context();
    assert!(ctx.contains("[Session: Empty Session]"));
    assert!(!ctx.contains("[Wave:"));
}

#[test]
fn planning_session_status_roundtrips() {
    for status in [
        PlanningSessionStatus::Draft,
        PlanningSessionStatus::InReview,
        PlanningSessionStatus::Approved,
        PlanningSessionStatus::Published,
        PlanningSessionStatus::Archived,
    ] {
        let json = must_serialize(&status);
        let back: PlanningSessionStatus = must_deserialize(&json);
        assert_eq!(status, back);
    }
}

// ─── Planning Wave & Task Package Projection ──────────────────────────────

#[test]
fn planning_wave_roundtrips() {
    let wave = PlanningWave {
        wave_id: "wave-1".into(),
        wave_name: "rich-client-hosted-mode".into(),
        tasks_dir: "docs/tasks".into(),
        milestones: vec![
            "M6: Gateway And Stream Contract".into(),
            "M9: Collaborative Planning Alpha".into(),
        ],
        task_entries: vec![
            TaskEntry {
                id: "OSYM-700".into(),
                file: "docs/tasks/osym-700-current-gateway-inventory-and-vocabulary.md".into(),
            },
            TaskEntry {
                id: "OSYM-730".into(),
                file: "docs/tasks/osym-730-planning-artifact-schema-and-session-service.md".into(),
            },
        ],
    };
    let json = must_serialize(&wave);
    let back: PlanningWave = must_deserialize(&json);
    assert_eq!(back.wave_name, "rich-client-hosted-mode");
    assert_eq!(back.milestones.len(), 2);
    assert_eq!(back.task_entries.len(), 2);
}

#[test]
fn task_package_projection_is_derived_from_wave() {
    let wave = PlanningWave {
        wave_id: "wave-1".into(),
        wave_name: "rich-client-hosted-mode".into(),
        tasks_dir: "docs/tasks".into(),
        milestones: vec!["M9: Collaborative Planning Alpha".into()],
        task_entries: vec![TaskEntry {
            id: "OSYM-730".into(),
            file: "docs/tasks/osym-730-planning-artifact-schema-and-session-service.md".into(),
        }],
    };
    let projection = wave.to_task_package_projection();
    assert_eq!(projection.planning_wave, "rich-client-hosted-mode");
    assert_eq!(projection.tasks_dir, "docs/tasks");
    assert_eq!(projection.milestones.len(), 1);
    assert_eq!(projection.tasks.len(), 1);
    assert_eq!(projection.tasks[0].id, "OSYM-730");
}

#[test]
fn task_package_projection_roundtrips() {
    let proj = TaskPackageProjection {
        planning_wave: "rich-client-hosted-mode".into(),
        tasks_dir: "docs/tasks".into(),
        milestones: vec!["M9".into()],
        tasks: vec![TaskEntry {
            id: "OSYM-730".into(),
            file: "docs/tasks/osym-730.md".into(),
        }],
    };
    let json = must_serialize(&proj);
    let back: TaskPackageProjection = must_deserialize(&json);
    assert_eq!(back.planning_wave, proj.planning_wave);
    assert_eq!(back.tasks[0].id, "OSYM-730");
}

// ─── Linear Publish Receipt ───────────────────────────────────────────────

#[test]
fn linear_publish_receipt_roundtrips() {
    let receipt = LinearPublishReceipt {
        planning_wave: "rich-client-hosted-mode".into(),
        linear_project: "e7b957855cb7".into(),
        published_at: Utc::now(),
        milestones: vec![PublishedMilestone {
            name: "M9: Collaborative Planning Alpha".into(),
            milestone_id: "806afecc-4a9f-4862-8330-6ce70d606058".into(),
        }],
        tasks: vec![PublishedTask {
            task_id: "OSYM-730".into(),
            issue: "COE-395".into(),
            issue_id: "f95c6539-7582-44b8-a508-5822c2346033".into(),
            url: "https://linear.app/trilogy-ai-coe/issue/COE-395/planning-artifact-schema-and-session-service"
                .into(),
            file: "docs/tasks/osym-730-planning-artifact-schema-and-session-service.md".into(),
        }],
    };
    let json = must_serialize(&receipt);
    let back: LinearPublishReceipt = must_deserialize(&json);
    assert_eq!(back.planning_wave, "rich-client-hosted-mode");
    assert_eq!(back.linear_project, "e7b957855cb7");
    assert_eq!(back.milestones.len(), 1);
    assert_eq!(back.tasks.len(), 1);
    assert_eq!(back.tasks[0].issue, "COE-395");
}

#[test]
fn linear_publish_receipt_render_yaml_contains_expected_fields() {
    let receipt = LinearPublishReceipt {
        planning_wave: "rich-client-hosted-mode".into(),
        linear_project: "proj-123".into(),
        published_at: Utc::now(),
        milestones: vec![PublishedMilestone {
            name: "M9: Collaborative Planning Alpha".into(),
            milestone_id: "ms-1".into(),
        }],
        tasks: vec![PublishedTask {
            task_id: "OSYM-730".into(),
            issue: "COE-395".into(),
            issue_id: "issue-1".into(),
            url: "https://linear.app/trilogy-ai-coe/issue/COE-395".into(),
            file: "docs/tasks/osym-730.md".into(),
        }],
    };
    let yaml = receipt.render_yaml();
    assert!(yaml.contains("planningWave: rich-client-hosted-mode"));
    assert!(yaml.contains("linearProject: proj-123"));
    assert!(yaml.contains("milestones:"));
    assert!(yaml.contains("M9: Collaborative Planning Alpha"));
    assert!(yaml.contains("tasks:"));
    assert!(yaml.contains("OSYM-730"));
    assert!(yaml.contains("issue: COE-395"));
}

#[test]
fn linear_publish_receipt_render_yaml_is_valid_yaml() {
    let receipt = LinearPublishReceipt {
        planning_wave: "rich-client-hosted-mode".into(),
        linear_project: "proj-123".into(),
        published_at: Utc::now(),
        milestones: vec![PublishedMilestone {
            name: "M9: Collaborative Planning Alpha".into(),
            milestone_id: "ms-1".into(),
        }],
        tasks: vec![PublishedTask {
            task_id: "OSYM-730".into(),
            issue: "COE-395".into(),
            issue_id: "issue-1".into(),
            url: "https://linear.app/trilogy-ai-coe/issue/COE-395".into(),
            file: "docs/tasks/osym-730.md".into(),
        }],
    };
    let yaml = receipt.render_yaml();
    // Verify the output is actually valid parseable YAML
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&yaml).expect("render_yaml must produce valid YAML");
    assert_eq!(
        parsed["planningWave"].as_str(),
        Some("rich-client-hosted-mode")
    );
    assert_eq!(parsed["linearProject"].as_str(), Some("proj-123"));
    assert_eq!(
        parsed["milestones"][0]["name"].as_str(),
        Some("M9: Collaborative Planning Alpha")
    );
    assert_eq!(
        parsed["milestones"][0]["milestoneId"].as_str(),
        Some("ms-1")
    );
    assert_eq!(parsed["tasks"][0]["taskId"].as_str(), Some("OSYM-730"));
    assert_eq!(parsed["tasks"][0]["issue"].as_str(), Some("COE-395"));
    // Assert publishedAt is present
    assert!(
        parsed["publishedAt"].as_str().is_some(),
        "publishedAt must be present"
    );
    // Ensure no unexpected keys are present
    let top_keys: std::collections::HashSet<&str> = parsed
        .as_mapping()
        .expect("top-level YAML must be a mapping")
        .keys()
        .filter_map(|k| k.as_str())
        .collect();
    let expected_keys: std::collections::HashSet<&str> = [
        "planningWave",
        "linearProject",
        "publishedAt",
        "milestones",
        "tasks",
    ]
    .into();
    assert_eq!(top_keys, expected_keys, "unexpected YAML keys");
}

// ─── Compile-time gate for all planning types ─────────────────────────────

#[test]
fn all_planning_types_compile_and_export() {
    let _ = PlanningArtifactKind::Intake;
    let _ = PlanningArtifactKind::ProjectContext;
    let _ = PlanningArtifactKind::Requirements;
    let _ = PlanningArtifactKind::ResearchBrief;
    let _ = PlanningArtifactKind::CodebaseAnalysis;
    let _ = PlanningArtifactKind::ArchitectureNotes;
    let _ = PlanningArtifactKind::RiskRegister;
    let _ = PlanningArtifactKind::MilestoneDraft;
    let _ = PlanningArtifactKind::IssueDraft;
    let _ = PlanningArtifactKind::SubIssueDraft;
    let _ = PlanningArtifactKind::DependencyMap;
    let _ = PlanningArtifactKind::VerificationPlan;
    let _ = PlanningArtifactKind::AcceptanceCriteria;
    let _ = PlanningArtifactKind::PlanValidation;
    let _ = PlanningArtifactKind::LinearDraft;
    let _ = PlanningArtifactKind::ReviewComments;
    let _ = PlanningArtifactKind::PublishReceipt;
    let _ = PlanningArtifactKind::PlanningWave;
    let _ = TurnRole::User;
    let _ = TurnRole::Agent;
    let _ = TurnRole::System;
}

// ─── Long-running run liveness fixtures ────────────────────────────────────

#[test]
fn run_phase_roundtrips() {
    let phases = [
        RunPhase::Active,
        RunPhase::Quiet,
        RunPhase::Degraded,
        RunPhase::Stalled,
        RunPhase::RetryQueued,
        RunPhase::Cancelled,
        RunPhase::Detached,
        RunPhase::Completed,
    ];
    for phase in phases {
        let json = must_serialize(&phase);
        let back: RunPhase = must_deserialize(&json);
        assert_eq!(phase, back);
    }
}

#[test]
fn run_stream_liveness_roundtrips() {
    let statuses = [
        RunStreamLiveness::Healthy,
        RunStreamLiveness::Stale,
        RunStreamLiveness::Dead,
    ];
    for status in statuses {
        let json = must_serialize(&status);
        let back: RunStreamLiveness = must_deserialize(&json);
        assert_eq!(status, back);
    }
}

#[test]
fn run_progress_roundtrips() {
    let progress = RunProgress {
        sequence: 42,
        event_id: "evt-progress-42".into(),
        happened_at: Utc::now(),
        kind: "ConversationStateUpdateEvent".into(),
        summary: "Turn 3 completing".into(),
    };
    let json = must_serialize(&progress);
    let back: RunProgress = must_deserialize(&json);
    assert_eq!(progress.sequence, back.sequence);
    assert_eq!(progress.event_id, back.event_id);
    assert_eq!(progress.kind, back.kind);
}

#[test]
fn run_liveness_envelope_roundtrips() {
    let envelope = RunLivenessEnvelope {
        phase: RunPhase::Active,
        stream: RunStreamLiveness::Healthy,
        latest_progress: None,
        harness_acknowledged: false,
        cancel_failed: false,
        detached: false,
    };
    let json = must_serialize(&envelope);
    let back: RunLivenessEnvelope = must_deserialize(&json);
    assert_eq!(envelope.phase, back.phase);
    assert_eq!(envelope.stream, back.stream);
}

#[test]
fn safe_actions_roundtrips() {
    let actions = SafeActions {
        retry: true,
        cancel: false,
        rehydrate: true,
        detach: false,
    };
    let json = must_serialize(&actions);
    let back: SafeActions = must_deserialize(&json);
    assert_eq!(actions, back);
}

#[test]
fn safe_actions_defaults_to_all_false() {
    let actions = SafeActions::default();
    assert!(!actions.retry);
    assert!(!actions.cancel);
    assert!(!actions.rehydrate);
    assert!(!actions.detach);
}

#[test]
fn harness_scheduler_disagreement_roundtrips() {
    let diag = HarnessSchedulerDisagreement {
        scheduler_status: RunStatus::RetryQueued,
        harness_status: "running".into(),
        detected_at: Utc::now(),
        resolution_path: "cancel harness session, then retry".into(),
    };
    let json = must_serialize(&diag);
    let back: HarnessSchedulerDisagreement = must_deserialize(&json);
    assert_eq!(diag.scheduler_status, back.scheduler_status);
    assert_eq!(diag.harness_status, back.harness_status);
}
