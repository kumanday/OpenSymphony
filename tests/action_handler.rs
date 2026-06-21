use chrono::Utc;
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneWorkerOutcome as WorkerOutcome, InMemoryEventJournal, SnapshotEnvelope,
};
use opensymphony::opensymphony_gateway::action_handler::ActionHandler;
use opensymphony::opensymphony_gateway_schema::{
    action::{ActionDispatch, ActionKind, ActionStatus, ActionTarget},
    envelope::EntityKind,
    event_journal::EventKind,
    version::SchemaVersion,
};

fn fixture_snapshot(
    identifier: &str,
    runtime_state: IssueRuntimeState,
    last_outcome: WorkerOutcome,
) -> DaemonSnapshot {
    let now = Utc::now();
    DaemonSnapshot {
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
            conversation_count: 1,
            status_line: "healthy".to_owned(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 0,
            retry_queue_depth: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            total_cost_micros: 0,
        },
        issues: vec![IssueSnapshot {
            identifier: identifier.to_owned(),
            title: "Test Issue".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state,
            last_outcome,
            last_event_at: now,
            conversation_id_suffix: "c0e255".to_owned(),
            workspace_path_suffix: "COE-255".to_owned(),
            retry_count: 0,
            claimed_at: None,
            started_at: None,
            finished_at: None,
            turn_count: 0,
            max_turns: 0,
            runtime_seconds: 0,
            blocked: false,
            blocked_by: Vec::new(),
            server_base_url: None,
            transport_target: None,
            http_auth_mode: None,
            websocket_auth_mode: None,
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cancel_acknowledged: false,
            cancel_failed: false,
            detached: false,
        }],
        recent_events: Vec::new(),
    }
}

fn fixture_envelope(
    identifier: &str,
    runtime_state: IssueRuntimeState,
    last_outcome: WorkerOutcome,
) -> SnapshotEnvelope {
    SnapshotEnvelope {
        sequence: 1,
        published_at: Utc::now(),
        snapshot: fixture_snapshot(identifier, runtime_state, last_outcome),
    }
}

fn action_dispatch(
    action_kind: ActionKind,
    entity_id: &str,
    correlation_id: &str,
) -> ActionDispatch {
    ActionDispatch {
        schema_version: SchemaVersion::v1(),
        correlation_id: correlation_id.to_owned(),
        action_kind,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Issue,
            entity_id: entity_id.to_owned(),
        },
        payload: None,
        idempotency_key: None,
    }
}

#[tokio::test]
async fn action_handler_returns_accepted_receipt_with_correlation_id() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Cancel, "COE-255", "corr_001");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Accepted);
    assert_eq!(receipt.correlation_id, "corr_001");
    assert!(!receipt.action_id.is_empty());
    assert!(receipt.reason.is_none());
}

#[tokio::test]
async fn action_handler_returns_rejected_receipt_for_unknown_issue() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Cancel, "COE-999", "corr_002");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    assert_eq!(receipt.correlation_id, "corr_002");
    assert!(
        receipt
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("not found")
    );
}

#[tokio::test]
async fn action_handler_rejects_retry_on_active_run() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Retry, "COE-255", "corr_003");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    assert!(
        receipt
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("already active")
    );
}

#[tokio::test]
async fn action_handler_accepts_cancel_on_running_issue() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Cancel, "COE-255", "corr_004");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Accepted);
    assert!(receipt.reason.is_none());
}

#[tokio::test]
async fn action_handler_rejects_rehydrate_on_running_issue() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Rehydrate, "COE-255", "corr_005");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    assert!(
        receipt
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("Rehydrate is only available")
    );
}

#[tokio::test]
async fn action_handler_accepts_rehydrate_on_completed_with_failed_outcome() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Completed,
        WorkerOutcome::Failed,
    );
    let dispatch = action_dispatch(ActionKind::Rehydrate, "COE-255", "corr_006");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Accepted);
    assert!(receipt.reason.is_none());
}

#[tokio::test]
async fn action_handler_rejects_rehydrate_on_completed_with_running_outcome() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Completed,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Rehydrate, "COE-255", "corr_007");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    assert!(
        receipt
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("Rehydrate is only available")
    );
}

#[tokio::test]
async fn action_handler_publishes_accepted_event_to_journal() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Cancel, "COE-255", "corr_008");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Accepted);
    let cursor = opensymphony::opensymphony_gateway_schema::cursor::StreamCursor::new(0, "events");
    let page = journal.query_after(&cursor, 10).await.expect("query");
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].correlation_id, Some("corr_008".to_owned()));
    assert_eq!(
        page.events[0].kind,
        EventKind::GatewayActionDispatched {
            action: "cancel".into()
        }
    );
}

#[tokio::test]
async fn action_handler_publishes_rejected_event_to_journal() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Retry, "COE-255", "corr_009");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    let cursor = opensymphony::opensymphony_gateway_schema::cursor::StreamCursor::new(0, "events");
    let page = journal.query_after(&cursor, 10).await.expect("query");
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].correlation_id, Some("corr_009".to_owned()));
    assert!(matches!(
        page.events[0].kind,
        EventKind::GatewayActionFailed { .. }
    ));
}

#[tokio::test]
async fn action_handler_rejects_duplicate_idempotency_key() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let mut dispatch = action_dispatch(ActionKind::Cancel, "COE-255", "corr_010");
    dispatch.idempotency_key = Some("key_001".to_owned());
    let receipt1 = handler.dispatch(dispatch.clone(), &envelope).await;
    assert_eq!(receipt1.status, ActionStatus::Accepted);
    let receipt2 = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt2.status, ActionStatus::Rejected);
    assert!(
        receipt2
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("duplicate idempotency key")
    );
}

#[tokio::test]
async fn action_handler_returns_expected_followups_for_retry() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Completed,
        WorkerOutcome::Failed,
    );
    let dispatch = action_dispatch(ActionKind::Retry, "COE-255", "corr_011");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Accepted);
    use opensymphony::opensymphony_gateway_schema::action::ExpectedFollowup;
    assert!(
        receipt
            .expected_followup
            .iter()
            .any(|f| matches!(f, ExpectedFollowup::ActionCompletion))
    );
    assert!(
        receipt
            .expected_followup
            .iter()
            .any(|f| matches!(f, ExpectedFollowup::RunLifecycle))
    );
}

#[tokio::test]
async fn action_handler_comment_is_accepted_on_any_state() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    for (state, outcome) in [
        (IssueRuntimeState::Running, WorkerOutcome::Running),
        (IssueRuntimeState::Idle, WorkerOutcome::Unknown),
        (IssueRuntimeState::Completed, WorkerOutcome::Completed),
        (IssueRuntimeState::Failed, WorkerOutcome::Failed),
    ] {
        let envelope = fixture_envelope("COE-255", state, outcome);
        let dispatch = action_dispatch(ActionKind::Comment, "COE-255", "corr_comment");
        let receipt = handler.dispatch(dispatch, &envelope).await;
        assert_eq!(
            receipt.status,
            ActionStatus::Accepted,
            "comment should be accepted in {:?}",
            state
        );
    }
}

#[tokio::test]
async fn action_handler_pause_rejected_on_non_running_issue() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope("COE-255", IssueRuntimeState::Idle, WorkerOutcome::Unknown);
    let dispatch = action_dispatch(ActionKind::Pause, "COE-255", "corr_pause");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    assert!(
        receipt
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("pause only valid on a running issue")
    );
}

#[tokio::test]
async fn action_handler_resume_rejected_on_non_paused_issue() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let dispatch = action_dispatch(ActionKind::Resume, "COE-255", "corr_resume");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Rejected);
    assert!(
        receipt
            .reason
            .as_ref()
            .expect("should not be None")
            .contains("resume only valid on a paused issue")
    );
}

#[tokio::test]
async fn action_handler_resume_accepted_on_paused_issue() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope("COE-255", IssueRuntimeState::Paused, WorkerOutcome::Running);
    let dispatch = action_dispatch(ActionKind::Resume, "COE-255", "corr_resume_ok");
    let receipt = handler.dispatch(dispatch, &envelope).await;
    assert_eq!(receipt.status, ActionStatus::Accepted);
    assert!(receipt.reason.is_none());
}

/// Concurrent idempotency test: two tasks with the same idempotency key
/// should result in exactly one accepted receipt and one rejected receipt.
#[tokio::test]
async fn action_handler_concurrent_idempotency_only_one_accepted() {
    let journal = InMemoryEventJournal::new(1000, 64);
    let handler = ActionHandler::new(journal.clone());
    let envelope = fixture_envelope(
        "COE-255",
        IssueRuntimeState::Running,
        WorkerOutcome::Running,
    );
    let mut dispatch = action_dispatch(ActionKind::Cancel, "COE-255", "corr_concurrent");
    dispatch.idempotency_key = Some("concurrent_key_001".to_owned());

    let h1 = handler.clone();
    let h2 = handler.clone();
    let env1 = envelope.clone();
    let env2 = envelope.clone();
    let d1 = dispatch.clone();
    let d2 = dispatch.clone();

    let (r1, r2) = tokio::join!(h1.dispatch(d1, &env1), h2.dispatch(d2, &env2),);

    let statuses = vec![r1.status, r2.status];
    assert!(
        statuses.contains(&ActionStatus::Accepted),
        "exactly one accepted receipt expected; got {:?}",
        statuses
    );
    assert!(
        statuses.contains(&ActionStatus::Rejected),
        "exactly one rejected receipt expected; got {:?}",
        statuses
    );

    // Verify exactly one audit event was appended to the journal (no
    // duplicate-race event from the rejected dispatch).
    let cursor = opensymphony::opensymphony_gateway_schema::cursor::StreamCursor::new(0, "events");
    let page = journal.query_after(&cursor, 10).await.expect("query");
    assert_eq!(
        page.events.len(),
        1,
        "journal should contain exactly one event after concurrent dispatch"
    );
}
