use chrono::{TimeZone, Utc};
use opensymphony_domain::{
    AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot,
    MetricsSnapshot, RecentEvent, RecentEventKind, SnapshotEnvelope, WorkerOutcome,
};

fn fixture() -> SnapshotEnvelope {
    let now = Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap();
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

    let encoded = serde_json::to_value(&envelope).unwrap();
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
        encoded["snapshot"]["recent_events"][0]["kind"],
        "snapshot_published"
    );

    let decoded: SnapshotEnvelope = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, envelope);
}

#[test]
fn snapshot_envelope_decodes_unknown_recent_event_kinds() {
    let mut encoded = serde_json::to_value(fixture()).unwrap();
    encoded["snapshot"]["recent_events"][0]["kind"] =
        serde_json::Value::String("tool_call_summary".to_owned());

    let decoded: SnapshotEnvelope = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        decoded.snapshot.recent_events[0].kind,
        RecentEventKind::Other("tool_call_summary".to_owned())
    );

    let reencoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        reencoded["snapshot"]["recent_events"][0]["kind"],
        "tool_call_summary"
    );
}
