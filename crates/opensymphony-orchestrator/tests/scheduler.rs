use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
};

use chrono::{TimeZone, Utc};
use opensymphony_orchestrator::{
    ConversationId, ConversationMetadata, IssueId, IssueIdentifier, IssueRef, IssueState,
    IssueStateCategory, NormalizedIssue, RecoveryRecord, ReleaseReason, RetryReason,
    RuntimeStreamState, Scheduler, SchedulerConfig, SchedulerStatus, TimestampMs, TrackerBackend,
    TrackerIssue, TrackerIssueState, TrackerIssueStateKind, TrackerIssueStateSnapshot,
    WorkerAbortReason, WorkerBackend, WorkerId, WorkerLaunch, WorkerOutcomeKind,
    WorkerOutcomeRecord, WorkerStartRequest, WorkerUpdate, WorkspaceBackend, WorkspaceKey,
    WorkspaceRecord,
};

fn ts(value: u64) -> TimestampMs {
    TimestampMs::new(value)
}

fn dt(value: u64) -> chrono::DateTime<Utc> {
    Utc.timestamp_millis_opt(value as i64)
        .single()
        .expect("timestamp should be valid")
}

fn scheduler_config() -> SchedulerConfig {
    SchedulerConfig {
        poll_interval_ms: 1_000,
        max_concurrent_agents: 2,
        max_turns: 4,
        max_concurrent_agents_by_state: BTreeMap::new(),
        retry_policy: Default::default(),
        stall_timeout_ms: Some(100),
        active_states: vec!["In Progress".to_string()],
        terminal_states: vec!["Done".to_string(), "Canceled".to_string()],
    }
}

fn tracker_issue(id: &str, identifier: &str, state: &str, created_at: u64) -> TrackerIssue {
    TrackerIssue {
        id: id.to_string(),
        identifier: identifier.to_string(),
        url: format!("https://linear.app/example/{identifier}"),
        title: format!("Issue {identifier}"),
        description: Some("scheduler test fixture".to_string()),
        priority: Some(1),
        state: state.to_string(),
        labels: Vec::new(),
        parent_id: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: dt(created_at),
        updated_at: dt(created_at),
    }
}

fn normalized_issue(id: &str, identifier: &str, state: &str) -> NormalizedIssue {
    NormalizedIssue {
        id: IssueId::new(id).expect("issue id should be valid"),
        identifier: IssueIdentifier::new(identifier).expect("issue identifier should be valid"),
        title: format!("Issue {identifier}"),
        description: None,
        priority: Some(1),
        state: IssueState {
            id: None,
            name: state.to_string(),
            category: if state == "In Progress" {
                IssueStateCategory::Active
            } else if matches!(state, "Done" | "Canceled") {
                IssueStateCategory::Terminal
            } else {
                IssueStateCategory::NonActive
            },
        },
        branch_name: None,
        url: Some(format!("https://linear.app/example/{identifier}")),
        labels: Vec::new(),
        parent_id: None,
        blocked_by: Vec::new(),
        sub_issues: vec![IssueRef {
            id: IssueId::new(format!("{id}-child")).expect("child id should be valid"),
            identifier: IssueIdentifier::new(format!("{identifier}-child"))
                .expect("child identifier should be valid"),
            state: "Done".to_string(),
        }],
        created_at: Some(ts(0)),
        updated_at: Some(ts(0)),
    }
}

fn tracker_state_snapshot(
    id: &str,
    identifier: &str,
    state: &str,
    tracker_type: &str,
    updated_at: u64,
) -> TrackerIssueStateSnapshot {
    TrackerIssueStateSnapshot {
        id: id.to_string(),
        identifier: identifier.to_string(),
        state: TrackerIssueState {
            id: state.to_ascii_lowercase().replace(' ', "-"),
            name: state.to_string(),
            tracker_type: tracker_type.to_string(),
            kind: TrackerIssueStateKind::from_tracker_type(tracker_type),
        },
        updated_at: dt(updated_at),
    }
}

fn workspace_record(identifier: &str, path: &str) -> WorkspaceRecord {
    WorkspaceRecord {
        path: PathBuf::from(path),
        workspace_key: WorkspaceKey::new(identifier).expect("workspace key should be valid"),
        created_now: false,
        created_at: Some(ts(0)),
        updated_at: Some(ts(0)),
        last_seen_tracker_refresh_at: Some(ts(0)),
    }
}

fn conversation(worker_id: &WorkerId) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: ConversationId::new(format!("conv-{}", worker_id.as_str()))
            .expect("conversation id should be valid"),
        server_base_url: Some("http://127.0.0.1:8000".to_string()),
        transport_target: Some("loopback".to_string()),
        http_auth_mode: Some("none".to_string()),
        websocket_auth_mode: Some("none".to_string()),
        websocket_query_param_name: None,
        fresh_conversation: true,
        runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_string()),
        stream_state: RuntimeStreamState::Ready,
        last_event_id: None,
        last_event_kind: None,
        last_event_at: None,
        last_event_summary: None,
        recent_activity: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
        runtime_seconds: 0,
    }
}

#[derive(Debug, Clone)]
struct FakeError(String);

impl std::fmt::Display for FakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for FakeError {}

#[derive(Default)]
struct FakeTracker {
    active: Vec<TrackerIssue>,
    terminal: Vec<TrackerIssue>,
    states: HashMap<String, TrackerIssueStateSnapshot>,
    state_requests: Vec<Vec<String>>,
}

impl TrackerBackend for FakeTracker {
    type Error = FakeError;

    async fn candidate_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        Ok(self.active.clone())
    }

    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        Ok(self.terminal.clone())
    }

    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, Self::Error> {
        self.state_requests.push(issue_ids.to_vec());
        Ok(issue_ids
            .iter()
            .filter_map(|id| self.states.get(id).cloned())
            .collect())
    }
}

#[derive(Default)]
struct FakeWorkspace {
    recoveries: Vec<RecoveryRecord>,
    ensured: Vec<String>,
    cleaned: Vec<(String, bool)>,
    records: HashMap<String, WorkspaceRecord>,
}

impl WorkspaceBackend for FakeWorkspace {
    type Error = FakeError;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        _observed_at: TimestampMs,
    ) -> Result<WorkspaceRecord, Self::Error> {
        self.ensured.push(issue.identifier.to_string());
        let record = self
            .records
            .entry(issue.id.to_string())
            .or_insert_with(|| {
                workspace_record(
                    issue.identifier.as_str(),
                    &format!("/tmp/workspaces/{}", issue.identifier),
                )
            })
            .clone();
        Ok(record)
    }

    async fn recover_workspaces(&mut self) -> Result<Vec<RecoveryRecord>, Self::Error> {
        Ok(self.recoveries.clone())
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error> {
        self.cleaned
            .push((workspace.workspace_key.to_string(), terminal));
        Ok(())
    }
}

#[derive(Default)]
struct FakeWorker {
    launches: Vec<WorkerStartRequest>,
    updates: VecDeque<WorkerUpdate>,
    aborted: Vec<(String, WorkerAbortReason)>,
    launch_results: VecDeque<Result<WorkerLaunch, FakeError>>,
}

impl WorkerBackend for FakeWorker {
    type Error = FakeError;

    async fn start_worker(
        &mut self,
        request: WorkerStartRequest,
    ) -> Result<WorkerLaunch, Self::Error> {
        self.launches.push(request.clone());
        match self.launch_results.pop_front() {
            Some(result) => result,
            None => Ok(WorkerLaunch {
                conversation: conversation(&request.run.worker_id),
            }),
        }
    }

    async fn poll_updates(&mut self) -> Result<Vec<WorkerUpdate>, Self::Error> {
        Ok(self.updates.drain(..).collect())
    }

    async fn abort_worker(
        &mut self,
        worker_id: &WorkerId,
        reason: WorkerAbortReason,
    ) -> Result<(), Self::Error> {
        self.aborted.push((worker_id.to_string(), reason));
        Ok(())
    }
}

#[tokio::test]
async fn successful_worker_exit_queues_continuation_retry_for_active_issue() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-268", "COE-268", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-268").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("execution should exist")
            .status(),
        SchedulerStatus::Running
    );
    assert_eq!(scheduler.worker().launches.len(), 1);

    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Succeeded,
                ts(200),
                Some("worker exited cleanly".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("second tick should succeed");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    let retry = execution.retry().expect("retry metadata should exist");
    assert_eq!(retry.reason, RetryReason::Continuation);
    assert_eq!(retry.due_at, ts(1_200));

    scheduler
        .tick(ts(1_300))
        .await
        .expect("third tick should redispatch the issue");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(scheduler.worker().launches.len(), 2);
    let second_run = &scheduler.worker().launches[1].run;
    assert_eq!(
        second_run
            .attempt
            .expect("retry run should carry a retry attempt")
            .get(),
        1
    );
    assert_eq!(second_run.normal_retry_count, 1);
}

#[tokio::test]
async fn failures_schedule_exponential_backoff() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-269", "COE-269", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    let issue_id = IssueId::new("lin-269").expect("issue id should be valid");
    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Failed,
                ts(200),
                Some("worker failed".to_string()),
                Some("boom".to_string()),
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("failure tick should succeed");

    let retry = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .retry()
        .expect("retry should exist")
        .clone();
    assert_eq!(retry.reason, RetryReason::Failure);
    assert_eq!(retry.due_at, ts(10_200));

    scheduler
        .tick(ts(10_200))
        .await
        .expect("first retry dispatch should succeed");

    let second_run = scheduler.worker().launches[1].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: second_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &second_run,
                WorkerOutcomeKind::Failed,
                ts(10_400),
                Some("worker failed again".to_string()),
                Some("still broken".to_string()),
            ),
        });

    scheduler
        .tick(ts(10_400))
        .await
        .expect("second failure tick should succeed");

    let retry = scheduler
        .execution(&issue_id)
        .expect("execution should exist")
        .retry()
        .expect("retry should exist")
        .clone();
    assert_eq!(
        retry.attempt.get(),
        2,
        "second retry should increment the retry attempt"
    );
    assert_eq!(retry.due_at, ts(30_400));
}

#[tokio::test]
async fn per_state_capacity_releases_slot_after_worker_finishes() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-275", "COE-275", "In Progress", 0),
            tracker_issue("lin-276", "COE-276", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch the first issue");

    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Succeeded,
                ts(200),
                Some("worker exited cleanly".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("finish tick should free the state slot for the next issue");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-276"
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-275").expect("issue id should be valid"))
            .expect("finished issue should still exist")
            .status(),
        SchedulerStatus::RetryQueued
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-276").expect("issue id should be valid"))
            .expect("second issue should be running")
            .status(),
        SchedulerStatus::Running
    );
}

#[tokio::test]
async fn terminal_reconciliation_aborts_running_worker_and_cleans_up_workspace() {
    let issue = tracker_issue("lin-270", "COE-270", "In Progress", 0);
    let tracker = FakeTracker {
        active: vec![
            issue.clone(),
            tracker_issue("lin-270-b", "COE-270-B", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should succeed");

    scheduler.tracker_mut().active =
        vec![tracker_issue("lin-270-b", "COE-270-B", "In Progress", 1)];
    scheduler.tracker_mut().terminal = vec![tracker_issue("lin-270", "COE-270", "Done", 0)];

    scheduler
        .tick(ts(200))
        .await
        .expect("terminal reconciliation should succeed");

    let issue_id = IssueId::new("lin-270").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("released execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    match execution.state() {
        opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerTerminal);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::TrackerTerminal
    );
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-270".to_string(), true)]
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-270-B"
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-270-b").expect("issue id should be valid"))
            .expect("replacement issue should be running")
            .status(),
        SchedulerStatus::Running
    );
}

#[tokio::test]
async fn runtime_events_extend_stall_deadlines_before_retrying_a_stalled_worker() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-271", "COE-271", "In Progress", 0),
            tracker_issue("lin-271-b", "COE-271-B", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(0))
        .await
        .expect("first tick should succeed");

    let running = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::RuntimeEvent {
            worker_id: running.worker_id.clone(),
            observed_at: ts(50),
            event_id: Some("evt-1".to_string()),
            event_kind: Some("conversation_state_update".to_string()),
            summary: Some("agent still making progress".to_string()),
        });

    scheduler
        .tick(ts(50))
        .await
        .expect("runtime event tick should succeed");
    let snapshot = scheduler.snapshot(ts(50));
    assert_eq!(snapshot.issues[0].runtime.stalled_at, Some(ts(150)));

    scheduler
        .tick(ts(120))
        .await
        .expect("pre-stall tick should succeed");
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-271").expect("issue id should be valid"))
            .expect("execution should exist")
            .status(),
        SchedulerStatus::Running
    );

    scheduler
        .tick(ts(160))
        .await
        .expect("stall tick should succeed");

    let execution = scheduler
        .execution(&IssueId::new("lin-271").expect("issue id should be valid"))
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(scheduler.worker().aborted[0].1, WorkerAbortReason::Stalled);
    assert_eq!(
        execution.retry().expect("retry should exist").reason,
        RetryReason::Stalled
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-271-B"
    );
}

#[tokio::test]
async fn recovery_reuses_manifest_workspace_for_active_issue_dispatch() {
    let recovered_workspace = workspace_record("COE-272", "/tmp/recovered/COE-272");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-272", "COE-272", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-272", "COE-272", "In Progress"),
            workspace: recovered_workspace.clone(),
            had_in_flight_run: true,
        }],
        records: HashMap::from([("lin-272".to_string(), recovered_workspace.clone())]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery tick should succeed");

    let issue_id = IssueId::new("lin-272").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should exist after recovery");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(
        execution
            .workspace()
            .expect("workspace should be attached")
            .path,
        recovered_workspace.path
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].workspace.path,
        recovered_workspace.path
    );
    assert!(scheduler.workspace().cleaned.is_empty());
}

#[tokio::test]
async fn tracker_inactive_release_frees_the_per_state_slot() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-277", "COE-277", "In Progress", 0),
            tracker_issue("lin-278", "COE-278", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch the first issue");

    scheduler.tracker_mut().active = vec![tracker_issue("lin-278", "COE-278", "In Progress", 1)];
    scheduler.tracker_mut().states.insert(
        "lin-277".to_string(),
        tracker_state_snapshot("lin-277", "COE-277", "Todo", "unstarted", 200),
    );

    scheduler
        .tick(ts(200))
        .await
        .expect("inactive reconciliation should release and replace the running issue");

    let released = scheduler
        .execution(&IssueId::new("lin-277").expect("issue id should be valid"))
        .expect("released issue should still exist");
    assert_eq!(released.status(), SchedulerStatus::Released);
    match released.state() {
        opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerInactive);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::TrackerInactive
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-278"
    );
}

#[tokio::test]
async fn running_count_follows_active_state_reconciliation() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-280", "COE-280", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_concurrent_agents = 3;
    config.stall_timeout_ms = None;
    config.active_states.push("Code Review".to_string());
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    config
        .max_concurrent_agents_by_state
        .insert("Code Review".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch the initial issue");

    scheduler.tracker_mut().active = vec![
        tracker_issue("lin-280", "COE-280", "Code Review", 0),
        tracker_issue("lin-281", "COE-281", "In Progress", 1),
        tracker_issue("lin-282", "COE-282", "Code Review", 2),
    ];

    scheduler
        .tick(ts(200))
        .await
        .expect("active-state reconciliation should update running counts");

    let refreshed = scheduler
        .execution(&IssueId::new("lin-280").expect("issue id should be valid"))
        .expect("original issue should still be running");
    assert_eq!(refreshed.status(), SchedulerStatus::Running);
    assert_eq!(refreshed.issue().state.name, "Code Review");
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-281"
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-282").expect("issue id should be valid"))
            .expect("reconciled active issue should exist")
            .status(),
        SchedulerStatus::Unclaimed
    );
}

#[tokio::test]
async fn recovery_does_not_count_released_issues_as_running_capacity() {
    let recovered_workspace = workspace_record("COE-283-A", "/tmp/recovered/COE-283-A");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-283-b", "COE-283-B", "In Progress", 1)],
        states: HashMap::from([(
            "lin-283-a".to_string(),
            tracker_state_snapshot("lin-283-a", "COE-283-A", "Todo", "unstarted", 100),
        )]),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-283-a", "COE-283-A", "In Progress"),
            workspace: recovered_workspace,
            had_in_flight_run: true,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery tick should not reserve running capacity for released issues");

    let recovered = scheduler
        .execution(&IssueId::new("lin-283-a").expect("issue id should be valid"))
        .expect("recovered issue should still exist");
    assert_eq!(recovered.status(), SchedulerStatus::Released);
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].issue.identifier.as_str(),
        "COE-283-B"
    );
}

#[tokio::test]
async fn per_state_capacity_limits_dispatches_even_when_multiple_issues_are_ready() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-273", "COE-273", "In Progress", 0),
            tracker_issue("lin-274", "COE-274", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config
        .max_concurrent_agents_by_state
        .insert("In Progress".to_string(), 1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler.tick(ts(100)).await.expect("tick should succeed");

    assert_eq!(scheduler.worker().launches.len(), 1);
    let running = scheduler
        .executions()
        .values()
        .filter(|execution| execution.status() == SchedulerStatus::Running)
        .count();
    let unclaimed = scheduler
        .executions()
        .values()
        .filter(|execution| execution.status() == SchedulerStatus::Unclaimed)
        .count();
    assert_eq!(running, 1);
    assert_eq!(unclaimed, 1);
}
