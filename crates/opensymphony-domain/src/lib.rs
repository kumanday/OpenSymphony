mod control_plane;
mod identifiers;
mod issue;
mod runtime;
mod snapshot;
mod state_machine;
mod time;

pub const CRATE_NAME: &str = "opensymphony-domain";

pub use control_plane::{
    ControlPlaneAgentServerStatus, ControlPlaneDaemonSnapshot, ControlPlaneDaemonState,
    ControlPlaneDaemonStatus, ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot,
    ControlPlaneMetricsSnapshot, ControlPlaneRecentEvent, ControlPlaneRecentEventKind,
    ControlPlaneWorkerOutcome, SnapshotEnvelope,
};
pub use identifiers::{
    ConversationId, IdentifierError, IssueId, IssueIdentifier, TrackerStateId, WorkerId,
    WorkspaceKey,
};
pub use issue::{BlockerRef, IssueState, IssueStateCategory, NormalizedIssue};
pub use runtime::{
    ConversationMetadata, ReleaseReason, RetryAttempt, RetryCalculationError, RetryEntry,
    RetryPolicy, RetryReason, RunAttempt, RuntimeStreamState, StallMetadata, WorkerOutcomeKind,
    WorkerOutcomeRecord, WorkspaceRecord,
};
pub use snapshot::{
    ComponentHealthSnapshot, DaemonSnapshot, HealthStatus, IssueSnapshot, OrchestratorSnapshot,
    RetrySnapshot, RuntimeStateSnapshot, RuntimeUsageTotals, WorkerAttemptSnapshot,
};
pub use state_machine::{
    IssueExecution, SchedulerState, SchedulerStatus, StateTransitionError, TransitionAction,
};
pub use time::{DurationMs, TimestampMs};

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use serde_json::json;

    use super::{
        ComponentHealthSnapshot, ConversationMetadata, HealthStatus, IssueExecution, IssueId,
        IssueIdentifier, IssueSnapshot, IssueState, IssueStateCategory, NormalizedIssue,
        OrchestratorSnapshot, ReleaseReason, RetryAttempt, RetryEntry, RetryPolicy, RetryReason,
        RunAttempt, RuntimeStreamState, RuntimeUsageTotals, SchedulerStatus, StateTransitionError,
        TimestampMs, WorkerId, WorkerOutcomeKind, WorkerOutcomeRecord, WorkspaceKey,
        WorkspaceRecord,
    };

    fn must<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{error}"),
        }
    }

    fn must_some<T>(value: Option<T>, message: &str) -> T {
        match value {
            Some(value) => value,
            None => panic!("{message}"),
        }
    }

    fn ts(value: u64) -> TimestampMs {
        TimestampMs::new(value)
    }

    fn unique_temp_path(prefix: &str) -> PathBuf {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        std::env::temp_dir().join(format!(
            "opensymphony-domain-{prefix}-{}-{unique_suffix}",
            std::process::id()
        ))
    }

    struct TempPathGuard(PathBuf);

    impl TempPathGuard {
        fn new(path: PathBuf) -> Self {
            Self(path)
        }
    }

    impl Drop for TempPathGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_issue() -> NormalizedIssue {
        NormalizedIssue {
            id: must(IssueId::new("lin_260")),
            identifier: must(IssueIdentifier::new("COE-260")),
            title: "Domain model and orchestrator state machine".to_owned(),
            description: Some("Define the shared orchestration model.".to_owned()),
            priority: Some(1),
            state: IssueState {
                id: None,
                name: "In Progress".to_owned(),
                category: IssueStateCategory::Active,
            },
            branch_name: Some(
                "leonardogonzalez/coe-260-domain-model-and-orchestrator-state-machine".to_owned(),
            ),
            url: Some(
                "https://linear.app/trilogy-ai-coe/issue/COE-260/domain-model-and-orchestrator-state-machine"
                    .to_owned(),
            ),
            labels: vec!["foundation".to_owned(), "contracts".to_owned()],
            blocked_by: Vec::new(),
            created_at: Some(ts(10)),
            updated_at: Some(ts(20)),
        }
    }

    fn sample_workspace() -> WorkspaceRecord {
        WorkspaceRecord {
            path: PathBuf::from("/tmp/workspaces/COE-260"),
            workspace_key: must(WorkspaceKey::new("COE-260")),
            created_now: false,
            created_at: Some(ts(11)),
            updated_at: Some(ts(21)),
            last_seen_tracker_refresh_at: Some(ts(22)),
        }
    }

    fn sample_run(
        issue: &NormalizedIssue,
        workspace: &WorkspaceRecord,
        attempt: Option<RetryAttempt>,
        claimed_at: TimestampMs,
    ) -> RunAttempt {
        RunAttempt::new(
            must(WorkerId::new("worker-1")),
            issue.id.clone(),
            issue.identifier.clone(),
            workspace.path.clone(),
            claimed_at,
            attempt,
            8,
        )
    }

    fn sample_conversation(fresh_conversation: bool) -> ConversationMetadata {
        ConversationMetadata {
            conversation_id: must(super::ConversationId::new("conv_260")),
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            fresh_conversation,
            runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_owned()),
            stream_state: RuntimeStreamState::Ready,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
        }
    }

    #[test]
    fn state_transitions_are_explicit_and_testable() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        assert_eq!(execution.status(), SchedulerStatus::Claimed);

        let mut session = sample_conversation(true);
        session.stream_state = RuntimeStreamState::Attaching;

        let mut execution =
            must(execution.start_running(ts(50), super::DurationMs::new(300_000), Some(session)));
        assert_eq!(execution.status(), SchedulerStatus::Running);

        must(execution.record_turn_started(ts(55)));
        must(execution.observe_runtime_event(
            ts(56),
            Some("evt_1".to_owned()),
            Some("conversation_state_update".to_owned()),
            Some("ready".to_owned()),
        ));

        let running = must_some(execution.current_run(), "running attempt must exist");
        assert_eq!(running.turn_count, 1);
        let outcome = WorkerOutcomeRecord::from_run(
            running,
            WorkerOutcomeKind::Succeeded,
            ts(60),
            Some("worker exited cleanly".to_owned()),
            None,
        );

        let retry = must(RetryEntry::continuation(
            &issue,
            running.attempt,
            0,
            ts(60),
            RetryPolicy::default(),
        ));

        let execution = must(execution.queue_retry(retry.clone(), outcome.clone()));
        assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
        assert_eq!(
            must_some(execution.retry(), "retry metadata must exist").attempt,
            retry.attempt
        );
        let retry_snapshot = execution.snapshot();
        assert_eq!(
            must_some(
                retry_snapshot.conversation,
                "retry-queued snapshots must retain conversation metadata",
            )
            .conversation_id
            .as_str(),
            "conv_260"
        );
        assert_eq!(
            must_some(
                execution.last_worker_outcome(),
                "last worker outcome must be recorded",
            )
            .outcome,
            WorkerOutcomeKind::Succeeded
        );

        let retry_run = sample_run(&issue, &workspace, Some(retry.attempt), ts(61));
        let execution = must(execution.claim(retry_run));
        assert_eq!(execution.status(), SchedulerStatus::Claimed);
        assert_eq!(
            must_some(execution.current_run(), "claimed retry run must exist").normal_retry_count,
            1
        );

        let execution = must(execution.release(ts(70), ReleaseReason::TrackerInactive, None));
        assert_eq!(execution.status(), SchedulerStatus::Released);

        let snapshot = execution.snapshot();
        assert_eq!(snapshot.runtime.state, SchedulerStatus::Released);
        assert_eq!(
            snapshot.runtime.release_reason,
            Some(ReleaseReason::TrackerInactive)
        );
        assert_eq!(snapshot.recent_worker_outcomes.len(), 1);
    }

    #[test]
    fn invalid_transitions_and_attempt_mismatches_are_rejected() {
        let issue = sample_issue();
        let workspace = sample_workspace();

        let execution = IssueExecution::new(issue.clone(), ts(30));
        let error = match execution.start_running(ts(50), super::DurationMs::new(10_000), None) {
            Ok(_) => panic!("starting from unclaimed should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::InvalidTransition { .. }
        ));

        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));
        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let outcome = WorkerOutcomeRecord {
            worker_id: must(WorkerId::new("worker-1")),
            attempt: None,
            outcome: WorkerOutcomeKind::Failed,
            started_at: ts(40),
            finished_at: ts(41),
            turn_count: 0,
            summary: None,
            error: Some("boom".to_owned()),
        };
        let retry = must(RetryEntry::failure(
            &issue,
            None,
            0,
            ts(41),
            RetryReason::Failure,
            Some("boom".to_owned()),
            RetryPolicy::default(),
        ));
        let execution = must(execution.queue_retry(retry.clone(), outcome));

        let wrong_attempt_run = sample_run(&issue, &workspace, None, ts(42));
        let error = match execution.claim(wrong_attempt_run) {
            Ok(_) => panic!("claiming with the wrong retry attempt should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::AttemptMismatch { .. }
        ));
    }

    #[test]
    fn claim_rejects_runs_without_an_attached_workspace() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let execution = IssueExecution::new(issue.clone(), ts(30));
        let run = sample_run(&issue, &workspace, None, ts(40));

        let error = match execution.claim(run) {
            Ok(_) => panic!("claiming without an attached workspace should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::WorkspaceNotAttached { .. }
        ));
    }

    #[test]
    fn start_running_requires_conversation_metadata_for_first_run() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let error = match execution.start_running(ts(50), super::DurationMs::new(300), None) {
            Ok(_) => panic!("starting the first run without conversation metadata should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::ConversationNotAttached
        ));
    }

    #[test]
    fn start_running_can_reuse_retained_conversation_metadata() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let execution = must(execution.start_running(
            ts(50),
            super::DurationMs::new(300),
            Some(sample_conversation(false)),
        ));
        let execution = must(execution.release(ts(60), ReleaseReason::TrackerInactive, None));
        let execution = must(execution.reopen(ts(70)));

        let run = sample_run(&issue, &workspace, None, ts(80));
        let execution = must(execution.claim(run));
        let execution = must(execution.start_running(ts(90), super::DurationMs::new(300), None));

        assert_eq!(
            must_some(
                execution.conversation(),
                "retained conversation metadata should be reused",
            )
            .conversation_id
            .as_str(),
            "conv_260"
        );
        assert_eq!(execution.status(), SchedulerStatus::Running);
    }

    #[test]
    fn claim_accepts_equivalent_normalized_workspace_paths() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let mut run = sample_run(&issue, &workspace, None, ts(40));
        run.workspace_path = PathBuf::from("/tmp/workspaces/../workspaces/COE-260");

        let execution = must(execution.claim(run));
        assert_eq!(execution.status(), SchedulerStatus::Claimed);
    }

    #[cfg(unix)]
    #[test]
    fn claim_accepts_workspace_paths_with_equivalent_symlink_roots() {
        let issue = sample_issue();
        let temp_root = unique_temp_path("workspace-symlink");
        let _temp_root_guard = TempPathGuard::new(temp_root.clone());
        let canonical_root = temp_root.join("canonical-root");
        let canonical_workspace = canonical_root.join("COE-260");
        let symlink_root = temp_root.join("symlink-root");

        must(fs::create_dir_all(&canonical_workspace));
        must(symlink(&canonical_root, &symlink_root));

        let workspace = WorkspaceRecord {
            path: symlink_root.join("COE-260"),
            ..sample_workspace()
        };

        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let mut run = sample_run(&issue, &workspace, None, ts(40));
        run.workspace_path = canonical_workspace;

        let execution = must(execution.claim(run));
        assert_eq!(execution.status(), SchedulerStatus::Claimed);
    }

    #[test]
    fn workspace_keys_are_sanitized_on_creation() {
        assert_eq!(must(WorkspaceKey::new("feature/42")).as_str(), "feature_42");
        assert_eq!(must(WorkspaceKey::new("../tmp")).as_str(), ".._tmp");
        assert_eq!(
            must(WorkspaceKey::new("Bug: weird path")).as_str(),
            "Bug__weird_path"
        );
    }

    #[test]
    fn retry_delay_math_matches_continuation_and_failure_rules() {
        let issue = sample_issue();
        let policy = RetryPolicy::default();

        let continuation = must(RetryEntry::continuation(&issue, None, 0, ts(100), policy));
        assert_eq!(continuation.attempt.get(), 1);
        assert_eq!(continuation.normal_retry_count, 1);
        assert_eq!(continuation.due_at, ts(1_100));

        let first_failure = must(RetryEntry::failure(
            &issue,
            None,
            1,
            ts(100),
            RetryReason::Failure,
            Some("first failure".to_owned()),
            policy,
        ));
        assert_eq!(first_failure.attempt.get(), 1);
        assert_eq!(first_failure.due_at, ts(10_100));

        let capped_policy = RetryPolicy {
            continuation_delay_ms: super::DurationMs::new(1_000),
            failure_base_delay_ms: super::DurationMs::new(10_000),
            max_backoff_ms: super::DurationMs::new(25_000),
        };
        let fifth_attempt = must(RetryAttempt::new(5));
        assert_eq!(
            capped_policy.failure_delay(fifth_attempt),
            super::DurationMs::new(25_000)
        );
    }

    #[test]
    fn reopen_preserves_workspace_and_conversation_after_inactive_release() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let execution = must(execution.start_running(
            ts(50),
            super::DurationMs::new(300_000),
            Some(sample_conversation(false)),
        ));
        let execution = must(execution.release(ts(60), ReleaseReason::TrackerInactive, None));
        let execution = must(execution.reopen(ts(70)));

        assert_eq!(execution.status(), SchedulerStatus::Unclaimed);
        assert_eq!(execution.workspace(), Some(&workspace));
        assert!(execution.conversation().is_some());
    }

    #[test]
    fn reopen_clears_workspace_and_conversation_after_terminal_release() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let execution = must(execution.start_running(
            ts(50),
            super::DurationMs::new(300_000),
            Some(sample_conversation(false)),
        ));
        let execution = must(execution.release(ts(60), ReleaseReason::TrackerTerminal, None));
        let execution = must(execution.reopen(ts(70)));

        assert_eq!(execution.status(), SchedulerStatus::Unclaimed);
        assert!(execution.workspace().is_none());
        assert!(execution.conversation().is_none());
    }

    #[test]
    fn recent_worker_outcomes_are_bounded_to_latest_window() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let mut next_attempt = None;

        for index in 0_u64..12 {
            let claimed_at = ts(40 + index * 10);
            let run = sample_run(&issue, &workspace, next_attempt, claimed_at);
            execution = must(execution.claim(run));

            let current_run = must_some(execution.current_run(), "claimed run must exist");
            let summary = format!("outcome {index}");
            let finished_at = ts(45 + index * 10);
            let outcome = WorkerOutcomeRecord::from_run(
                current_run,
                WorkerOutcomeKind::Failed,
                finished_at,
                Some(summary.clone()),
                Some("boom".to_owned()),
            );
            let retry = must(RetryEntry::failure(
                &issue,
                current_run.attempt,
                0,
                finished_at,
                RetryReason::Failure,
                Some("boom".to_owned()),
                RetryPolicy::default(),
            ));

            next_attempt = Some(retry.attempt);
            execution = must(execution.queue_retry(retry, outcome));
        }

        let snapshot = execution.snapshot();
        assert_eq!(
            snapshot
                .last_worker_outcome
                .as_ref()
                .and_then(|outcome| outcome.summary.as_deref()),
            Some("outcome 11")
        );
        assert_eq!(snapshot.recent_worker_outcomes.len(), 10);
        assert_eq!(
            snapshot.recent_worker_outcomes[0].summary.as_deref(),
            Some("outcome 2")
        );
        assert_eq!(
            snapshot.recent_worker_outcomes[9].summary.as_deref(),
            Some("outcome 11")
        );
    }

    #[test]
    fn snapshot_models_serialize_stably() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));
        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let issue_snapshot = IssueSnapshot::from(&execution);

        let snapshot = OrchestratorSnapshot::new(
            ts(100),
            super::DaemonSnapshot::new(
                HealthStatus::Healthy,
                30_000,
                4,
                Some(ts(90)),
                ComponentHealthSnapshot {
                    status: HealthStatus::Healthy,
                    detail: Some("ready".to_owned()),
                    updated_at: Some(ts(95)),
                },
                RuntimeUsageTotals::default(),
            ),
            vec![issue_snapshot],
        );

        let json = must(serde_json::to_value(&snapshot));
        assert_eq!(json["generated_at"], json!(100));
        assert_eq!(json["daemon"]["health"], json!("healthy"));
        assert_eq!(json["daemon"]["running_issue_count"], json!(0));
        assert_eq!(json["issues"][0]["issue"]["identifier"], json!("COE-260"));
        assert_eq!(json["issues"][0]["runtime"]["state"], json!("claimed"));
        assert_eq!(
            json["issues"][0]["workspace"]["path"],
            json!("/tmp/workspaces/COE-260")
        );
        assert_eq!(json["issues"][0]["retry"], serde_json::Value::Null);
    }

    #[test]
    fn replayed_runtime_events_do_not_hide_existing_stalls() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let mut execution = must(execution.start_running(
            ts(50),
            super::DurationMs::new(300),
            Some(sample_conversation(false)),
        ));

        must(execution.observe_runtime_event(
            ts(60),
            Some("evt_latest".to_owned()),
            Some("conversation_state_update".to_owned()),
            Some("ready".to_owned()),
        ));
        must(execution.observe_runtime_event(
            ts(55),
            Some("evt_old".to_owned()),
            Some("tool_call".to_owned()),
            Some("replayed".to_owned()),
        ));

        let conversation = must_some(
            execution.conversation(),
            "running execution must keep conversation metadata",
        );
        assert_eq!(conversation.last_event_at, Some(ts(60)));
        assert_eq!(conversation.last_event_id.as_deref(), Some("evt_latest"));

        let snapshot = execution.snapshot();
        assert_eq!(snapshot.runtime.last_event_at, Some(ts(60)));
        assert_eq!(snapshot.runtime.stalled_at, Some(ts(360)));
    }

    #[test]
    fn attach_workspace_rejects_rebinding_to_different_identity() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue, ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let rebound_workspace = WorkspaceRecord {
            path: PathBuf::from("/tmp/workspaces/COE-260-alt"),
            workspace_key: must(WorkspaceKey::new("COE-260-alt")),
            ..workspace.clone()
        };

        let error = match execution.attach_workspace(rebound_workspace) {
            Ok(_) => panic!("rebinding a different workspace identity should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::WorkspaceIdentityMismatch { .. }
        ));
        assert_eq!(execution.workspace(), Some(&workspace));
    }

    #[test]
    fn attach_workspace_rejects_first_binding_for_the_wrong_issue_path() {
        let issue = sample_issue();
        let workspace = WorkspaceRecord {
            path: PathBuf::from("/tmp/workspaces/COE-261"),
            workspace_key: must(WorkspaceKey::new("COE-260")),
            ..sample_workspace()
        };
        let mut execution = IssueExecution::new(issue, ts(30));

        let error = match execution.attach_workspace(workspace) {
            Ok(_) => panic!("first workspace attachment for a different issue path should fail"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::WorkspaceIssueMismatch { .. }
        ));
        assert!(execution.workspace().is_none());
    }

    #[test]
    fn attach_workspace_allows_refresh_for_same_identity() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue, ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let refreshed_workspace = WorkspaceRecord {
            updated_at: Some(ts(99)),
            last_seen_tracker_refresh_at: Some(ts(100)),
            ..workspace.clone()
        };

        must(execution.attach_workspace(refreshed_workspace.clone()));
        assert_eq!(execution.workspace(), Some(&refreshed_workspace));
    }

    #[test]
    fn running_snapshot_last_event_at_stays_none_without_runtime_events() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let mut execution = must(execution.start_running(
            ts(50),
            super::DurationMs::new(300),
            Some(sample_conversation(false)),
        ));

        must(execution.record_turn_started(ts(55)));

        let snapshot = execution.snapshot();
        assert_eq!(snapshot.runtime.last_event_at, None);
        assert_eq!(snapshot.runtime.stalled_at, Some(ts(355)));
        assert_eq!(
            snapshot
                .runtime
                .worker
                .as_ref()
                .map(|worker| worker.normal_retry_count),
            Some(0)
        );
    }

    #[test]
    fn queue_retry_rejects_outcomes_from_a_different_worker() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let outcome = WorkerOutcomeRecord {
            worker_id: must(WorkerId::new("worker-2")),
            attempt: None,
            outcome: WorkerOutcomeKind::Failed,
            started_at: ts(40),
            finished_at: ts(41),
            turn_count: 0,
            summary: Some("stale worker".to_owned()),
            error: Some("boom".to_owned()),
        };
        let retry = must(RetryEntry::failure(
            &issue,
            None,
            0,
            ts(41),
            RetryReason::Failure,
            Some("boom".to_owned()),
            RetryPolicy::default(),
        ));

        let error = match execution.queue_retry(retry, outcome) {
            Ok(_) => panic!("queue_retry should reject stale worker outcomes"),
            Err(error) => error,
        };
        assert!(matches!(error, StateTransitionError::WorkerMismatch { .. }));
    }

    #[test]
    fn queue_retry_rejects_outcomes_from_a_different_attempt() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        must(execution.attach_workspace(workspace.clone()));

        let first_run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(first_run));
        let first_outcome = WorkerOutcomeRecord::from_run(
            must_some(execution.current_run(), "claimed run must exist"),
            WorkerOutcomeKind::Succeeded,
            ts(50),
            Some("completed".to_owned()),
            None,
        );
        let first_retry = must(RetryEntry::continuation(
            &issue,
            None,
            0,
            ts(50),
            RetryPolicy::default(),
        ));
        let execution = must(execution.queue_retry(first_retry.clone(), first_outcome));

        let retry_run = sample_run(&issue, &workspace, Some(first_retry.attempt), ts(60));
        let execution = must(execution.claim(retry_run));
        let stale_outcome = WorkerOutcomeRecord {
            worker_id: must(WorkerId::new("worker-1")),
            attempt: None,
            outcome: WorkerOutcomeKind::Failed,
            started_at: ts(40),
            finished_at: ts(61),
            turn_count: 1,
            summary: Some("old attempt".to_owned()),
            error: Some("boom".to_owned()),
        };
        let retry = must(RetryEntry::failure(
            &issue,
            Some(first_retry.attempt),
            1,
            ts(61),
            RetryReason::Failure,
            Some("boom".to_owned()),
            RetryPolicy::default(),
        ));

        let error = match execution.queue_retry(retry, stale_outcome) {
            Ok(_) => panic!("queue_retry should reject stale attempt outcomes"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            StateTransitionError::AttemptMismatch { .. }
        ));
    }
}
