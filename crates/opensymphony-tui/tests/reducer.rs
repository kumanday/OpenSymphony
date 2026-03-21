use chrono::{TimeZone, Utc};
use opensymphony_domain::{
    AgentServerStatus, DaemonSnapshot, DaemonState, DaemonStatus, IssueRuntimeState, IssueSnapshot,
    MetricsSnapshot, RecentEvent, RecentEventKind, SnapshotEnvelope, WorkerOutcome,
};
use opensymphony_tui::{ConnectionState, FocusPane, TimelineMode, TuiAction, TuiState};

fn fixture(sequence: u64, issue_count: usize) -> SnapshotEnvelope {
    let now = Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap()
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
fn keeps_timeline_visible_with_many_issues_in_inline_layout() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 12))));

    let rendered = state.render_text(100, 22);

    assert!(rendered.contains("RECENT EVENTS"));
    assert!(rendered.contains("snapshot updated"));
}

#[test]
fn keeps_selected_detail_visible_in_narrow_layout() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 6))));

    let rendered = state.render_text(70, 22);

    assert!(rendered.contains("ISSUE + WORKSPACE DETAIL"));
    assert!(rendered.contains("workspace path: workspace-0"));
}

#[test]
fn keeps_rendering_latest_snapshot_while_reconnecting() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));
    state.reduce(TuiAction::ConnectionLost("stream closed".to_owned()));

    let rendered = state.render_text(100, 20);

    assert!(rendered.contains("conn=reconnecting"));
    assert!(rendered.contains("COE-255"));
    assert!(rendered.contains("workspace path: workspace-0"));
}
