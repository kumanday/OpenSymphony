use chrono::{TimeZone, Utc};
use opensymphony_control::{
    AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot,
    MetricsSnapshot, RecentEvent, RecentEventKind, SnapshotEnvelope, WorkerOutcome,
};
use opensymphony_tui::{ConnectionState, FocusPane, TimelineMode, TuiAction, TuiState};

fn fixture(sequence: u64, issue_count: usize) -> SnapshotEnvelope {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("fixture timestamp should be valid")
        + chrono::Duration::seconds(sequence as i64);
    SnapshotEnvelope {
        sequence,
        published_at: now,
        snapshot: DaemonSnapshot {
            generated_at: now,
            daemon: DaemonStatus {
                state: DaemonState::Ready,
                last_poll_at: now,
                workspace_root: "/tmp/opensymphony".to_owned(),
                status_line: "ready".to_owned(),
            },
            agent_server: AgentServerStatus {
                reachable: true,
                base_url: "http://127.0.0.1:3000".to_owned(),
                conversation_count: issue_count as u32,
                status_line: "healthy".to_owned(),
            },
            metrics: MetricsSnapshot {
                running_issues: 1,
                retry_queue_depth: 0,
                total_tokens: 1024,
                total_cost_micros: 50_000,
            },
            issues: (0..issue_count)
                .map(|index| IssueSnapshot {
                    identifier: format!("COE-{}", 255 + index),
                    title: format!("Issue {index}"),
                    tracker_state: "In Progress".to_owned(),
                    runtime_state: IssueRuntimeState::Running,
                    last_outcome: WorkerOutcome::Running,
                    last_event_at: now,
                    conversation_id_suffix: format!("conv-{index}"),
                    workspace_path_suffix: format!("workspace-{index}"),
                    retry_count: index as u32,
                    blocked: false,
                })
                .collect(),
            recent_events: vec![RecentEvent {
                happened_at: now,
                issue_identifier: Some("COE-255".to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: "snapshot updated".to_owned(),
            }],
        },
    }
}

fn fixture_with_identifiers(sequence: u64, identifiers: &[&str]) -> SnapshotEnvelope {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("fixture timestamp should be valid")
        + chrono::Duration::seconds(sequence as i64);
    SnapshotEnvelope {
        sequence,
        published_at: now,
        snapshot: DaemonSnapshot {
            generated_at: now,
            daemon: DaemonStatus {
                state: DaemonState::Ready,
                last_poll_at: now,
                workspace_root: "/tmp/opensymphony".to_owned(),
                status_line: "ready".to_owned(),
            },
            agent_server: AgentServerStatus {
                reachable: true,
                base_url: "http://127.0.0.1:3000".to_owned(),
                conversation_count: identifiers.len() as u32,
                status_line: "healthy".to_owned(),
            },
            metrics: MetricsSnapshot {
                running_issues: 1,
                retry_queue_depth: 0,
                total_tokens: 1024,
                total_cost_micros: 50_000,
            },
            issues: identifiers
                .iter()
                .enumerate()
                .map(|(index, identifier)| IssueSnapshot {
                    identifier: (*identifier).to_owned(),
                    title: format!("Issue {index}"),
                    tracker_state: "In Progress".to_owned(),
                    runtime_state: IssueRuntimeState::Running,
                    last_outcome: WorkerOutcome::Running,
                    last_event_at: now,
                    conversation_id_suffix: format!("conv-{index}"),
                    workspace_path_suffix: format!("workspace-{index}"),
                    retry_count: index as u32,
                    blocked: false,
                })
                .collect(),
            recent_events: vec![RecentEvent {
                happened_at: now,
                issue_identifier: identifiers.first().map(|value| (*value).to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: "snapshot updated".to_owned(),
            }],
        },
    }
}

fn retime(mut envelope: SnapshotEnvelope, seconds_from_base: i64) -> SnapshotEnvelope {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("fixture timestamp should be valid")
        + chrono::Duration::seconds(seconds_from_base);
    envelope.published_at = now;
    envelope.snapshot.generated_at = now;
    envelope.snapshot.daemon.last_poll_at = now;
    for issue in &mut envelope.snapshot.issues {
        issue.last_event_at = now;
    }
    for event in &mut envelope.snapshot.recent_events {
        event.happened_at = now;
    }
    envelope
}

#[test]
fn applies_snapshot_and_renders_selected_issue() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));

    assert_eq!(state.connection, ConnectionState::Live);
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("focus=issues"));
    assert!(rendered.contains("[x] ISSUES"));
    assert!(rendered.contains("[ ] ISSUE + WORKSPACE DETAIL"));
    assert!(rendered.contains("COE-255"));
    assert!(rendered.contains("Issue 0"));
    assert!(rendered.contains("RECENT EVENTS"));
}

#[test]
fn bootstrap_snapshot_keeps_connecting_status_until_stream_updates_arrive() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::BootstrapSnapshotReceived(Box::new(fixture(
        3, 2,
    ))));

    assert_eq!(state.connection, ConnectionState::Connecting);
    assert_eq!(
        state
            .latest_snapshot
            .as_ref()
            .map(|snapshot| snapshot.sequence),
        Some(3)
    );
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("conn=connecting"));
    assert!(rendered.contains("COE-255"));

    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));

    assert_eq!(state.connection, ConnectionState::Live);
}

#[test]
fn clamps_selection_when_new_snapshot_has_fewer_issues() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(1, 3))));
    state.reduce(TuiAction::MoveSelectionDown);
    state.reduce(TuiAction::MoveSelectionDown);

    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(2, 1))));

    assert_eq!(state.selected_issue, 0);
}

#[test]
fn cycles_focus_and_timeline_mode() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::FocusNext);
    state.reduce(TuiAction::FocusNext);
    state.reduce(TuiAction::ToggleTimelineMode);

    assert_eq!(state.focus, FocusPane::Timeline);
    assert_eq!(state.timeline_mode, TimelineMode::Metrics);

    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("focus=timeline"));
    assert!(rendered.contains("bottom=metrics"));
    assert!(rendered.contains("[x] METRICS"));
}

#[test]
fn preserves_selected_issue_when_snapshot_order_changes() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(1, &["COE-255", "COE-256", "COE-257"]),
    )));
    state.reduce(TuiAction::MoveSelectionDown);

    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(2, &["COE-257", "COE-256", "COE-255"]),
    )));

    assert_eq!(state.selected_issue, 1);
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("COE-256 Issue 1"));
}

#[test]
fn ignores_regressing_snapshot_sequences() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(5, &["COE-255", "COE-256"]),
    )));
    state.reduce(TuiAction::MoveSelectionDown);
    state.reduce(TuiAction::ConnectionLost("stream closed".to_owned()));

    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(4, &["COE-256", "COE-255"]),
    )));

    assert_eq!(
        state.connection,
        ConnectionState::Reconnecting("stream closed".to_owned())
    );
    assert_eq!(state.selected_issue, 1);
    assert_eq!(
        state
            .latest_snapshot
            .as_ref()
            .map(|snapshot| snapshot.sequence),
        Some(5)
    );
}

#[test]
fn accepts_lower_sequence_after_reconnect_when_snapshot_is_newer() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(5, &["COE-255", "COE-256"]),
    )));
    state.reduce(TuiAction::ConnectionLost("stream closed".to_owned()));

    let restarted = retime(fixture_with_identifiers(1, &["COE-301", "COE-302"]), 30);
    state.reduce(TuiAction::SnapshotReceived(Box::new(restarted.clone())));

    assert_eq!(state.connection, ConnectionState::Live);
    assert_eq!(
        state
            .latest_snapshot
            .as_ref()
            .map(|snapshot| snapshot.sequence),
        Some(1)
    );
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("COE-301"));
}

#[test]
fn accepts_lower_sequence_after_restart_without_connection_loss_transition() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(5, &["COE-255", "COE-256"]),
    )));

    let restarted = retime(fixture_with_identifiers(1, &["COE-401", "COE-402"]), 30);
    state.reduce(TuiAction::SnapshotReceived(Box::new(restarted.clone())));

    assert_eq!(state.connection, ConnectionState::Live);
    assert_eq!(
        state
            .latest_snapshot
            .as_ref()
            .map(|snapshot| snapshot.sequence),
        Some(1)
    );
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("COE-401"));
}

#[test]
fn keeps_selected_issue_visible_when_issue_list_exceeds_pane_height() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(1, 12))));
    for _ in 0..8 {
        state.reduce(TuiAction::MoveSelectionDown);
    }

    let rendered = state.render_text(100, 22);

    assert!(rendered.contains("> COE-263 [running / In Progress]"));
    assert!(!rendered.contains("> COE-255 [running / In Progress]"));
}
