use chrono::{TimeZone, Utc};
use opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome, SnapshotEnvelope,
};
use opensymphony_tui::{ConnectionState, FocusPane, TimelineMode, TuiAction, TuiState};

fn fixture(sequence: u64, issue_count: usize) -> SnapshotEnvelope {
    let identifiers = (0..issue_count)
        .map(|index| format!("COE-{}", 255 + index))
        .collect::<Vec<_>>();
    fixture_with_identifiers(sequence, &identifiers)
}

fn fixture_with_identifiers(sequence: u64, identifiers: &[String]) -> SnapshotEnvelope {
    let now = Utc
        .with_ymd_and_hms(2026, 3, 21, 20, 0, 0)
        .single()
        .expect("valid fixed test timestamp")
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
                input_tokens: 512,
                output_tokens: 512,
                cache_read_tokens: 256,
                total_tokens: 1024,
                total_cost_micros: 50_000,
            },
            issues: identifiers
                .iter()
                .enumerate()
                .map(|(index, identifier)| IssueSnapshot {
                    identifier: identifier.clone(),
                    title: format!("Issue {identifier}"),
                    tracker_state: "In Progress".to_owned(),
                    runtime_state: IssueRuntimeState::Running,
                    last_outcome: WorkerOutcome::Running,
                    last_event_at: now,
                    conversation_id_suffix: format!("conv-{identifier}"),
                    workspace_path_suffix: format!("workspace-{index}"),
                    retry_count: index as u32,
                    blocked: false,
                    server_base_url: Some("http://127.0.0.1:3000".to_owned()),
                    transport_target: Some("loopback".to_owned()),
                    http_auth_mode: Some("none".to_owned()),
                    websocket_auth_mode: Some("none".to_owned()),
                    websocket_query_param_name: None,
                    recent_events: Vec::new(),
                    modified_files: Vec::new(),
                    input_tokens: 1024 + (index as u64 * 100),
                    output_tokens: 512 + (index as u64 * 50),
                    cache_read_tokens: 256 + (index as u64 * 25),
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

fn reordered_fixture(sequence: u64, identifiers: &[&str]) -> SnapshotEnvelope {
    let mut snapshot = fixture(sequence, identifiers.len());
    snapshot.snapshot.issues = identifiers
        .iter()
        .enumerate()
        .map(|(index, identifier)| IssueSnapshot {
            identifier: (*identifier).to_owned(),
            title: format!("Issue {index}"),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Running,
            last_outcome: WorkerOutcome::Running,
            last_event_at: snapshot.snapshot.generated_at,
            conversation_id_suffix: format!("conv-{index}"),
            workspace_path_suffix: format!("workspace-{index}"),
            retry_count: index as u32,
            blocked: false,
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            transport_target: Some("loopback".to_owned()),
            http_auth_mode: Some("none".to_owned()),
            websocket_auth_mode: Some("none".to_owned()),
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
            input_tokens: 1024 + (index as u64 * 100),
            output_tokens: 512 + (index as u64 * 50),
            cache_read_tokens: 256 + (index as u64 * 25),
        })
        .collect();
    snapshot
}

#[test]
fn applies_snapshot_and_renders_selected_issue() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));

    assert_eq!(state.connection, ConnectionState::Connecting);
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("conn=connecting"));
    assert!(rendered.contains("focus=issues"));
    assert!(rendered.contains("[x] ISSUES"));
    assert!(rendered.contains("[ ] ISSUE + WORKSPACE DETAIL"));
    assert!(rendered.contains("COE-255"));
    assert!(rendered.contains("Issue COE-255"));
    assert!(rendered.contains("RECENT EVENTS"));
}

#[test]
fn marks_the_ui_live_after_the_stream_attaches() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));
    state.reduce(TuiAction::StreamAttached);

    assert_eq!(state.connection, ConnectionState::Live);
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("conn=live"));
    assert!(rendered.contains("COE-255"));
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
fn preserves_selected_issue_when_snapshot_reorders() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(1, 3))));
    state.reduce(TuiAction::MoveSelectionDown);

    let reordered = vec![
        "COE-257".to_owned(),
        "COE-255".to_owned(),
        "COE-256".to_owned(),
    ];
    state.reduce(TuiAction::SnapshotReceived(Box::new(
        fixture_with_identifiers(2, &reordered),
    )));

    assert_eq!(state.selected_issue, 2);

    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("> COE-256 [running / In Progress]"));
    assert!(
        rendered.contains("workspace:"),
        "rendered output was: {}",
        rendered
    );
    assert!(
        rendered.contains("conv:"),
        "rendered output was: {}",
        rendered
    );
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
    assert!(rendered.contains("workspace: workspace-0"));
}

#[test]
fn keeps_selected_issue_visible_when_issue_list_is_windowed() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 12))));
    for _ in 0..9 {
        state.reduce(TuiAction::MoveSelectionDown);
    }

    let rendered = state.render_text(70, 22);

    assert!(rendered.contains("> COE-264 [running / In Progress]"));
    assert!(rendered.contains("workspace: workspace-9"));
    assert!(!rendered.contains("> COE-255 [running / In Progress]"));
}

#[test]
fn keeps_rendering_latest_snapshot_while_reconnecting() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));
    state.reduce(TuiAction::StreamAttached);
    state.reduce(TuiAction::ConnectionLost("stream closed".to_owned()));

    let rendered = state.render_text(100, 20);

    assert!(rendered.contains("conn=reconnecting"));
    assert!(rendered.contains("COE-255"));
    assert!(rendered.contains("workspace: workspace-0"));
}

#[test]
fn refreshed_snapshots_do_not_claim_live_before_stream_reattaches() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 2))));
    state.reduce(TuiAction::StreamAttached);
    state.reduce(TuiAction::ConnectionLost("stream closed".to_owned()));
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(4, 2))));

    assert_eq!(
        state.connection,
        ConnectionState::Reconnecting("stream closed".to_owned())
    );
    let rendered = state.render_text(100, 20);
    assert!(rendered.contains("conn=reconnecting"));
    assert!(rendered.contains("seq=4"));
}

#[test]
fn preserves_selected_issue_when_snapshots_reorder() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(reordered_fixture(
        1,
        &["COE-255", "COE-256", "COE-257"],
    ))));
    state.reduce(TuiAction::MoveSelectionDown);

    state.reduce(TuiAction::SnapshotReceived(Box::new(reordered_fixture(
        2,
        &["COE-257", "COE-255", "COE-256", "COE-258"],
    ))));

    assert_eq!(state.selected_issue, 2);
    let rendered = state.render_text(100, 22);
    assert!(rendered.contains("COE-256 Issue 2"));
}

#[test]
fn keeps_selected_issue_visible_in_long_issue_lists() {
    let mut state = TuiState::default();
    state.reduce(TuiAction::SnapshotReceived(Box::new(fixture(3, 12))));
    for _ in 0..8 {
        state.reduce(TuiAction::MoveSelectionDown);
    }

    let rendered = state.render_text(100, 22);

    assert!(rendered.contains("> COE-263 [running / In Progress]"));
    assert!(rendered.contains("workspace: workspace-8"));
}
