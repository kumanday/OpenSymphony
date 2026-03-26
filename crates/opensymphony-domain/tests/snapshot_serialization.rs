use chrono::{TimeZone, Utc};
use opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
};

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
            metrics: MetricsSnapshot {
                running_issues: 1,
                retry_queue_depth: 0,
                total_tokens: 8_192,
                total_cost_micros: 250_000,
            },
            issues: vec![IssueSnapshot {
                identifier: "COE-269".to_owned(),
                title: "Control-plane API and snapshot store".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::RetryQueued,
                last_outcome: WorkerOutcome::Continued,
                last_event_at: now,
                conversation_id_suffix: "269-live".to_owned(),
                workspace_path_suffix: "COE-269".to_owned(),
                retry_count: 1,
                blocked: false,
                server_base_url: Some("https://agent.example.com/runtime".to_owned()),
                transport_target: Some("remote".to_owned()),
                http_auth_mode: Some("header".to_owned()),
                websocket_auth_mode: Some("query_param".to_owned()),
                websocket_query_param_name: Some("session_api_key".to_owned()),
                recent_events: Vec::new(),
                modified_files: Vec::new(),
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
    assert_eq!(encoded["snapshot"]["issues"][0]["http_auth_mode"], "header");
    assert_eq!(
        encoded["snapshot"]["recent_events"][0]["kind"],
        "snapshot_published"
    );

    let decoded: SnapshotEnvelope =
        serde_json::from_value(encoded).expect("deserialize snapshot envelope from json");
    assert_eq!(decoded, envelope);
}
