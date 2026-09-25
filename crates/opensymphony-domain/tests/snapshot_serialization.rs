use crate::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
};
use chrono::{TimeZone, Utc};

fn fixture() -> SnapshotEnvelope {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("valid fixed test timestamp");
    SnapshotEnvelope {
        sequence: 7,
        published_at: now,
        snapshot: DaemonSnapshot {
            generated_at: now,
            daemon: DaemonStatus {
                state: DaemonState::Ready,
                last_poll_at: now,
                workspace_root: "/tmp/opensymphony/workspaces".to_owned(),
                status_line: "scheduler healthy".to_owned(),
            },
            agent_server: AgentServerStatus {
                reachable: true,
                base_url: "http://127.0.0.1:3002".to_owned(),
                conversation_count: 2,
                status_line: "healthy".to_owned(),
            },
            memory_server: Default::default(),
            metrics: MetricsSnapshot {
                running_issues: 1,
                retry_queue_depth: 0,
                input_tokens: 4096,
                output_tokens: 4096,
                cache_read_tokens: 1024,
                total_tokens: 8_192,
                total_cost_micros: 250_000,
            },
            issues: vec![IssueSnapshot {
                operator_interactions: Vec::new(),
                harness_capability: None,
                identifier: "COE-269".to_owned(),
                title: "Control-plane API and snapshot store".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::RetryQueued,
                last_outcome: WorkerOutcome::Continued,
                last_event_at: now,
                conversation_id_suffix: "269-live".to_owned(),
                codex_thread_id: None,
                workspace_path_suffix: "COE-269".to_owned(),
                branch_name: None,
                pr_url: None,
                project_id: Some("proj-open".to_owned()),
                project_slug: Some("opensymphony-bootstrap".to_owned()),
                project_name: Some("OpenSymphony".to_owned()),
                workspace_label: Some("COE-269".to_owned()),
                retry_count: 1,
                release_reason: None,
                claimed_at: None,
                started_at: None,
                finished_at: None,
                turn_count: 0,
                max_turns: 0,
                runtime_seconds: 0,
                blocked: false,
                hierarchy_generation: None,
                hierarchy_blocked_reason: None,
                repository_binding: None,
                blocked_by: Vec::new(),
                server_base_url: Some("https://agent.example.com/runtime".to_owned()),
                transport_target: Some("remote".to_owned()),
                http_auth_mode: Some("header".to_owned()),
                websocket_auth_mode: Some("query_param".to_owned()),
                websocket_query_param_name: Some("session_api_key".to_owned()),
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 2048,
                output_tokens: 1024,
                cache_read_tokens: 512,
                total_tokens: 3072,
                cancel_requested: false,
                cancel_acknowledged: false,
                cancel_failed: false,
                cancel_timed_out: false,
                cancel_reason: None,
                detached: false,
                operator: None,
            }],
            recent_events: vec![RecentEvent {
                happened_at: now,
                issue_identifier: Some("COE-269".to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: "snapshot sequence advanced".to_owned(),
            }],
        },
    }
}

#[test]
fn snapshot_envelope_round_trips_through_json() {
    let envelope = fixture();

    let encoded = serde_json::to_value(&envelope).expect("serialize snapshot envelope to json");
    assert_eq!(encoded["snapshot"]["daemon"]["state"], "ready");
    assert_eq!(
        encoded["snapshot"]["memory_server"]["status_line"],
        "disabled"
    );
    assert_eq!(
        encoded["snapshot"]["issues"][0]["runtime_state"],
        "retry_queued"
    );
    assert_eq!(
        encoded["snapshot"]["issues"][0]["last_outcome"],
        "continued"
    );
    assert_eq!(
        encoded["snapshot"]["issues"][0]["transport_target"],
        "remote"
    );
    assert_eq!(
        encoded["snapshot"]["issues"][0]["project_slug"],
        "opensymphony-bootstrap"
    );
    assert_eq!(
        encoded["snapshot"]["issues"][0]["project_name"],
        "OpenSymphony"
    );
    assert_eq!(
        encoded["snapshot"]["issues"][0]["workspace_label"],
        "COE-269"
    );
    assert_eq!(encoded["snapshot"]["issues"][0]["http_auth_mode"], "header");
    assert_eq!(
        encoded["snapshot"]["recent_events"][0]["kind"],
        "snapshot_published"
    );

    let decoded: SnapshotEnvelope =
        serde_json::from_value(encoded).expect("deserialize snapshot envelope from json");
    assert_eq!(decoded, envelope);
}

#[test]
fn older_issue_snapshots_without_project_metadata_still_decode() {
    let mut encoded = serde_json::to_value(fixture()).expect("serialize fixture");
    let issue = encoded["snapshot"]["issues"][0]
        .as_object_mut()
        .expect("issue object");
    issue.remove("project_id");
    issue.remove("project_slug");
    issue.remove("project_name");
    issue.remove("workspace_label");

    let decoded: SnapshotEnvelope =
        serde_json::from_value(encoded).expect("missing project metadata stays optional");
    let issue = &decoded.snapshot.issues[0];
    assert_eq!(issue.project_id, None);
    assert_eq!(issue.project_slug, None);
    assert_eq!(issue.project_name, None);
    assert_eq!(issue.workspace_label, None);
    assert_eq!(issue.blocked_by, Vec::<String>::new());
}

#[test]
fn operator_projection_round_trips_without_secret_or_path_fields() {
    use crate::opensymphony_domain::{
        ControlPlaneContainmentSnapshot, ControlPlaneOperatorSnapshot,
        ControlPlaneRepositorySnapshot,
    };

    let projection = ControlPlaneOperatorSnapshot {
        routing_mode: Some("project_set".to_owned()),
        active_project_set: vec!["project-a".to_owned()],
        linear_project: Some("project-a".to_owned()),
        binding_status: Some("resolved".to_owned()),
        parent: None,
        repository: Some(ControlPlaneRepositorySnapshot {
            canonical_id: "github:repository:123".to_owned(),
            display_alias: "backend".to_owned(),
            safe_remote_fingerprint: Some("sha256:fingerprint".to_owned()),
            config_generation: Some("config-1".to_owned()),
            inventory_generation: Some("inventory-1".to_owned()),
            checkout_generation: None,
            target_branch: Some("develop".to_owned()),
            target_commit: Some("abc123".to_owned()),
            instruction_source: Some("AGENTS.md".to_owned()),
            instruction_hash: Some("sha256:instructions".to_owned()),
        }),
        leases: Vec::new(),
        repairs: Vec::new(),
        memory: None,
        containment: Some(ControlPlaneContainmentSnapshot {
            requested_scope: Some("trusted_host".to_owned()),
            effective_containment: "trusted_host".to_owned(),
        }),
        provider: None,
        verification: None,
        cleanup: None,
    };
    let encoded = serde_json::to_value(&projection).expect("serialize operator projection");
    assert!(encoded.get("workspace_path").is_none());
    assert!(encoded.get("remote_url").is_none());
    let decoded: ControlPlaneOperatorSnapshot =
        serde_json::from_value(encoded).expect("deserialize operator projection");
    assert_eq!(decoded, projection);
}
