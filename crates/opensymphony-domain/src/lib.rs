mod identifiers;
mod issue;
mod runtime;
mod snapshot;
mod state_machine;
mod time;

pub const CRATE_NAME: &str = "opensymphony-domain";

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
    use std::path::PathBuf;

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

    #[test]
    fn state_transitions_are_explicit_and_testable() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        execution.attach_workspace(workspace.clone());

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        assert_eq!(execution.status(), SchedulerStatus::Claimed);

        let session = ConversationMetadata {
            conversation_id: must(super::ConversationId::new("conv_260")),
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            fresh_conversation: true,
            runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_owned()),
            stream_state: RuntimeStreamState::Attaching,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
        };

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
        execution.attach_workspace(workspace.clone());
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
    fn snapshot_models_serialize_stably() {
        let issue = sample_issue();
        let workspace = sample_workspace();
        let mut execution = IssueExecution::new(issue.clone(), ts(30));
        execution.attach_workspace(workspace.clone());
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
        execution.attach_workspace(workspace.clone());

        let run = sample_run(&issue, &workspace, None, ts(40));
        let execution = must(execution.claim(run));
        let session = ConversationMetadata {
            conversation_id: must(super::ConversationId::new("conv_260")),
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            fresh_conversation: false,
            runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_owned()),
            stream_state: RuntimeStreamState::Ready,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
        };

        let mut execution =
            must(execution.start_running(ts(50), super::DurationMs::new(300), Some(session)));

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
}
