pub const CRATE_NAME: &str = "opensymphony-orchestrator";

pub use opensymphony_domain::{
    ComponentHealthSnapshot, ConversationId, ConversationMetadata, DaemonSnapshot, DurationMs,
    HealthStatus, IdentifierError, IssueExecution, IssueId, IssueIdentifier, IssueSnapshot,
    IssueState, IssueStateCategory, NormalizedIssue, OrchestratorSnapshot, ReleaseReason,
    RetryAttempt, RetryCalculationError, RetryEntry, RetryPolicy, RetryReason, RunAttempt,
    RuntimeStreamState, RuntimeUsageTotals, SchedulerState, SchedulerStatus, StallMetadata,
    StateTransitionError, TimestampMs, TrackerStateId, TransitionAction, WorkerAttemptSnapshot,
    WorkerId, WorkerOutcomeKind, WorkerOutcomeRecord, WorkspaceKey, WorkspaceRecord,
};

pub fn boundary_summary() -> &'static str {
    "poll tick, runtime state machine, worker supervision, retry queue, cancellation/reconciliation, and snapshot derivation inputs"
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        boundary_summary, ConversationId, ConversationMetadata, DurationMs, IssueExecution,
        IssueId, IssueIdentifier, IssueState, IssueStateCategory, NormalizedIssue, ReleaseReason,
        RunAttempt, RuntimeStreamState, SchedulerStatus, TimestampMs, WorkerId, WorkspaceKey,
        WorkspaceRecord,
    };

    fn must<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn exposes_domain_state_machine_as_the_orchestrator_boundary() {
        let issue = NormalizedIssue {
            id: must(IssueId::new("lin_260")),
            identifier: must(IssueIdentifier::new("COE-260")),
            title: "Domain model and orchestrator state machine".to_owned(),
            description: None,
            priority: Some(1),
            state: IssueState {
                id: None,
                name: "In Progress".to_owned(),
                category: IssueStateCategory::Active,
            },
            branch_name: None,
            url: None,
            labels: Vec::new(),
            blocked_by: Vec::new(),
            created_at: None,
            updated_at: None,
        };

        let workspace = WorkspaceRecord {
            path: PathBuf::from("/tmp/workspaces/COE-260"),
            workspace_key: must(WorkspaceKey::new("COE-260")),
            created_now: false,
            created_at: None,
            updated_at: None,
            last_seen_tracker_refresh_at: None,
        };

        let run = RunAttempt::new(
            must(WorkerId::new("worker-1")),
            issue.id.clone(),
            issue.identifier.clone(),
            workspace.path.clone(),
            TimestampMs::new(10),
            None,
            8,
        );

        let mut execution = IssueExecution::new(issue, TimestampMs::new(0));
        must(execution.attach_workspace(workspace));
        let execution = must(execution.claim(run));
        let execution = must(execution.start_running(
            TimestampMs::new(11),
            DurationMs::new(300_000),
            Some(ConversationMetadata {
                conversation_id: must(ConversationId::new("conv_260")),
                server_base_url: Some("http://127.0.0.1:3000".to_owned()),
                fresh_conversation: true,
                runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_owned()),
                stream_state: RuntimeStreamState::Ready,
                last_event_id: None,
                last_event_kind: None,
                last_event_at: None,
                last_event_summary: None,
            }),
        ));
        let execution =
            must(execution.release(TimestampMs::new(12), ReleaseReason::TrackerInactive, None));

        assert_eq!(execution.status(), SchedulerStatus::Released);
        assert!(boundary_summary().contains("runtime state machine"));
    }
}
