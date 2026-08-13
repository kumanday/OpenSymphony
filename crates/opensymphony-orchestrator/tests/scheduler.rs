use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    path::PathBuf,
    time::Duration,
};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, DurationMs, HarnessInterruptCommand, HarnessInterruptReason,
    HarnessInterruptStatus, RepositoryBinding, RepositoryBindingOutcome, RepositoryIdentity,
    RepositoryInventoryEntry, RepositoryRouting, RepositoryRoutingMode, SafeRemoteFingerprint,
    TrackerErrorCategory, TrackerIssueRef,
};
use crate::opensymphony_orchestrator::{
    ChildEligibilityEvidence, ConversationId, ConversationMetadata, HierarchySnapshot, IssueId,
    IssueIdentifier, IssueRef, IssueState, IssueStateCategory, LeaseKind, LeaseOwner, LeaseRecord,
    LeaseResource, NormalizedIssue, ParentEligibilityEvidence, RecoveredRun, RecoveryRecord,
    ReleaseReason, RetryAttempt, RetryEntry, RetryExhaustionRecord, RetryPendingRecord,
    RetryReason, RuntimeStreamState, Scheduler, SchedulerConfig, SchedulerStatus, TimestampMs,
    TrackerBackend, TrackerIssue, TrackerIssueBlocker, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot, TrackerIssueSummary, WorkerAbortReason, WorkerBackend, WorkerId,
    WorkerInterruptAcknowledgement, WorkerLaunch, WorkerOutcomeKind, WorkerOutcomeRecord,
    WorkerStartRequest, WorkerUpdate, WorkspaceBackend, WorkspaceKey, WorkspaceRecord,
    decide_issue_route,
};
use crate::opensymphony_workflow::RoutingConfig;
use chrono::{TimeZone, Utc};

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
        max_retry_attempts: None,
        stall_timeout_ms: Some(100),
        active_states: vec!["In Progress".to_string()],
        terminal_states: vec!["Done".to_string(), "Canceled".to_string()],
        tracker_project_id: None,
        tracker_project_slug: None,
        tracker_project_ids: Vec::new(),
        tracker_project_id_slug_fallbacks: Vec::new(),
        tracker_project_slugs: Vec::new(),
        routing: RoutingConfig {
            harness: "openhands_agent_server".into(),
            model: None,
            model_profile: None,
            harness_env: "OPENSYMPHONY_HARNESS".into(),
            model_env: "OPENSYMPHONY_MODEL".into(),
            model_profile_env: "OPENSYMPHONY_MODEL_PROFILE".into(),
            harness_from_env: false,
            model_from_env: false,
            model_profile_from_env: false,
            dry_run: false,
        },
        repository_routing: None,
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
        state_kind: tracker_issue_state_kind_from_name(state),
        branch_name: None,
        pr_url: None,
        pr_urls: Vec::new(),
        labels: Vec::new(),
        project_id: None,
        project_slug: None,
        project_name: None,
        parent_id: None,
        parent: None,
        project_milestone: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: dt(created_at),
        updated_at: dt(created_at),
    }
}

fn tracker_issue_summary(issue: TrackerIssue) -> TrackerIssueSummary {
    TrackerIssueSummary {
        id: issue.id,
        identifier: issue.identifier,
        url: issue.url,
        title: issue.title,
        priority: issue.priority,
        state: issue.state,
        state_kind: issue.state_kind,
        blocked_by: issue.blocked_by,
        sub_issues: issue.sub_issues,
        created_at: issue.created_at,
        updated_at: issue.updated_at,
    }
}

fn tracker_issue_state_kind_from_name(state: &str) -> TrackerIssueStateKind {
    match state.trim().to_ascii_lowercase().as_str() {
        "backlog" => TrackerIssueStateKind::Backlog,
        "todo" => TrackerIssueStateKind::Unstarted,
        "in progress" | "review" | "human review" | "merging" => TrackerIssueStateKind::Started,
        "done" | "completed" | "closed" => TrackerIssueStateKind::Completed,
        "canceled" | "cancelled" => TrackerIssueStateKind::Canceled,
        other => TrackerIssueStateKind::Unknown(other.to_owned()),
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
        pr_url: None,
        pr_urls: Vec::new(),
        url: Some(format!("https://linear.app/example/{identifier}")),
        labels: Vec::new(),
        project_id: None,
        project_slug: None,
        project_name: None,
        parent_id: None,
        repository_binding: None,
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

fn repository_routing() -> RepositoryRouting {
    let identity = |alias: &str| RepositoryIdentity {
        id: CanonicalRepositoryId::new(format!("github:{alias}"))
            .expect("repository id should be valid"),
        safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
            "github",
            Some(alias),
            "https://github.com/example/repository",
        )
        .expect("fingerprint should be valid"),
    };
    let one = identity("one");
    let two = identity("two");
    let one_id = one.id.clone();
    let two_id = two.id.clone();

    RepositoryRouting {
        mode: RepositoryRoutingMode::ProjectSet,
        inventory: BTreeMap::from([
            (
                "one".to_string(),
                RepositoryInventoryEntry {
                    alias: "one".to_string(),
                    identity: one,
                },
            ),
            (
                "two".to_string(),
                RepositoryInventoryEntry {
                    alias: "two".to_string(),
                    identity: two,
                },
            ),
        ]),
        project_repositories: BTreeMap::from([(
            "project-id".to_string(),
            [one_id, two_id].into_iter().collect(),
        )]),
        active_projects: ["project-id".to_string()].into_iter().collect(),
        legacy_repository: None,
        config_generation: "config-test".to_string(),
        inventory_generation: "inventory-test".to_string(),
    }
}

#[test]
fn selected_route_uses_configured_codex_harness_and_model() {
    let issue = normalized_issue("lin-429", "COE-429", "In Progress");
    let mut config = scheduler_config();
    config.routing.harness = "codex_app_server".into();
    config.routing.model = Some("gpt-5-codex".into());
    config.routing.model_profile = Some("codex-chatgpt-local-keychain".into());
    config.routing.dry_run = true;

    let route = decide_issue_route(&issue, &config).expect("route should resolve");

    assert_eq!(route.harness_kind, "codex_app_server");
    assert_eq!(route.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(
        route.model_profile.as_deref(),
        Some("codex-chatgpt-local-keychain")
    );
    assert!(route.dry_run);
    assert!(route.reason.contains("workflow routing.harness"));
}

#[test]
fn selected_route_rejects_unavailable_harness() {
    let issue = normalized_issue("lin-430", "COE-430", "In Progress");
    let mut config = scheduler_config();
    config.routing.harness = "rust_native".into();

    let error = decide_issue_route(&issue, &config).expect_err("unavailable harness should fail");

    assert!(matches!(
        error,
        crate::opensymphony_orchestrator::SchedulerError::InvalidConfiguration { .. }
    ));
}

#[test]
fn selected_route_records_environment_selection() {
    let issue = normalized_issue("lin-431", "COE-431", "In Progress");
    let mut config = scheduler_config();
    config.routing.harness = "codex_app_server".into();
    config.routing.harness_from_env = true;

    let route = decide_issue_route(&issue, &config).expect("selected route should resolve");

    assert_eq!(route.harness_kind, "codex_app_server");
    assert!(route.user_override);
    assert!(route.reason.contains("OPENSYMPHONY_HARNESS"));
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
        project_id: None,
        project_slug: None,
        project_identity_known: false,
        labels: Vec::new(),
        is_parent: false,
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
        next_activity_sequence: 0,
    }
}

#[derive(Debug, Clone)]
struct FakeError {
    message: String,
    category: Option<TrackerErrorCategory>,
    retry_after: Option<Duration>,
}

impl FakeError {
    fn rate_limited(retry_after: Duration) -> Self {
        Self {
            message: "rate limited".to_string(),
            category: Some(TrackerErrorCategory::RateLimited),
            retry_after: Some(retry_after),
        }
    }

    fn not_found() -> Self {
        Self {
            message: "issue not found".to_string(),
            category: Some(TrackerErrorCategory::NotFound),
            retry_after: None,
        }
    }
}

impl std::fmt::Display for FakeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FakeError {}

#[derive(Default)]
struct FakeTracker {
    active: Vec<TrackerIssue>,
    terminal: Vec<TrackerIssue>,
    states: HashMap<String, TrackerIssueStateSnapshot>,
    detail_issues: Option<Vec<TrackerIssue>>,
    candidate_errors: VecDeque<FakeError>,
    summary_errors: VecDeque<FakeError>,
    terminal_errors: VecDeque<FakeError>,
    detail_errors: VecDeque<FakeError>,
    state_errors: VecDeque<FakeError>,
    active_requests: usize,
    summary_requests: usize,
    terminal_requests: usize,
    detail_requests: Vec<Vec<String>>,
    state_requests: Vec<Vec<String>>,
    parent_evidence: Option<ParentEligibilityEvidence>,
}

impl TrackerBackend for FakeTracker {
    type Error = FakeError;

    async fn candidate_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.active_requests += 1;
        if let Some(error) = self.candidate_errors.pop_front() {
            return Err(error);
        }
        Ok(self.active.clone())
    }

    async fn candidate_issue_summaries(&mut self) -> Result<Vec<TrackerIssueSummary>, Self::Error> {
        self.summary_requests += 1;
        if let Some(error) = self.summary_errors.pop_front() {
            return Err(error);
        }
        Ok(self
            .active
            .clone()
            .into_iter()
            .map(tracker_issue_summary)
            .collect())
    }

    async fn terminal_issues(&mut self) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.terminal_requests += 1;
        if let Some(error) = self.terminal_errors.pop_front() {
            return Err(error);
        }
        Ok(self.terminal.clone())
    }

    async fn issues_by_identifiers(
        &mut self,
        identifiers: &[String],
    ) -> Result<Vec<TrackerIssue>, Self::Error> {
        self.detail_requests.push(identifiers.to_vec());
        if let Some(error) = self.detail_errors.pop_front() {
            return Err(error);
        }
        let requested = identifiers
            .iter()
            .map(|identifier| identifier.to_ascii_uppercase())
            .collect::<std::collections::HashSet<_>>();
        let source = self.detail_issues.as_ref().unwrap_or(&self.active);
        Ok(source
            .iter()
            .filter(|issue| requested.contains(&issue.identifier.to_ascii_uppercase()))
            .cloned()
            .collect())
    }

    async fn issue_states_by_ids(
        &mut self,
        issue_ids: &[String],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, Self::Error> {
        self.state_requests.push(issue_ids.to_vec());
        if let Some(error) = self.state_errors.pop_front() {
            return Err(error);
        }
        Ok(issue_ids
            .iter()
            .filter_map(|id| self.states.get(id).cloned())
            .collect())
    }

    async fn parent_eligibility(
        &mut self,
        _parent: &TrackerIssue,
        _hierarchy: &HierarchySnapshot,
    ) -> Result<ParentEligibilityEvidence, Self::Error> {
        Ok(self
            .parent_evidence
            .clone()
            .unwrap_or_else(|| ParentEligibilityEvidence {
                hierarchy_generation: 0,
                children: Vec::new(),
            }))
    }

    fn error_category(error: &Self::Error) -> Option<TrackerErrorCategory> {
        error.category
    }

    fn retry_after(error: &Self::Error) -> Option<Duration> {
        error.retry_after
    }
}

#[derive(Default)]
struct FakeWorkspace {
    recoveries: Vec<RecoveryRecord>,
    retry_exhaustion: Vec<RetryExhaustionRecord>,
    retry_pending_recoveries: Vec<RetryPendingRecord>,
    cleared_retry_exhaustion: Vec<String>,
    ensured: Vec<String>,
    cleaned: Vec<(String, bool)>,
    failed_cleaned: Vec<String>,
    removed: Vec<String>,
    cleanup_results: VecDeque<Result<(), FakeError>>,
    records: HashMap<String, WorkspaceRecord>,
    persisted_retry_counts: Vec<u32>,
    persisted_retry_exhaustions: Vec<u32>,
    persist_retry_exhaustion_results: VecDeque<Result<(), FakeError>>,
    persisted_retry_pending: usize,
    persisted_retry_pending_without_workspace: usize,
    cleared_retry_pending: Vec<String>,
    revoked_issue_resources: Vec<String>,
    clear_retry_pending_results: VecDeque<Result<(), FakeError>>,
    ensure_results: VecDeque<Result<(), FakeError>>,
    persist_retry_pending_results: VecDeque<Result<(), FakeError>>,
    persisted_interrupt_reasons: Vec<HarnessInterruptReason>,
    persist_interrupt_results: VecDeque<Result<(), FakeError>>,
    retain_failed: bool,
    durable_state: Option<serde_json::Value>,
    persisted_durable_states: Vec<serde_json::Value>,
}

impl WorkspaceBackend for FakeWorkspace {
    type Error = FakeError;

    async fn ensure_workspace(
        &mut self,
        issue: &NormalizedIssue,
        _observed_at: TimestampMs,
    ) -> Result<WorkspaceRecord, Self::Error> {
        self.ensured.push(issue.identifier.to_string());
        if let Some(result) = self.ensure_results.pop_front() {
            result?;
        }
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

    async fn load_orchestrator_state(&mut self) -> Result<Option<serde_json::Value>, Self::Error> {
        Ok(self.durable_state.clone())
    }

    async fn persist_orchestrator_state(
        &mut self,
        state: &serde_json::Value,
    ) -> Result<(), Self::Error> {
        self.durable_state = Some(state.clone());
        self.persisted_durable_states.push(state.clone());
        Ok(())
    }

    async fn recover_retry_exhaustion(
        &mut self,
    ) -> Result<Vec<RetryExhaustionRecord>, Self::Error> {
        Ok(self.retry_exhaustion.clone())
    }

    async fn recover_retry_pending(&mut self) -> Result<Vec<RetryPendingRecord>, Self::Error> {
        Ok(self.retry_pending_recoveries.clone())
    }

    async fn cleanup_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
        terminal: bool,
    ) -> Result<(), Self::Error> {
        self.cleaned
            .push((workspace.workspace_key.to_string(), terminal));
        self.cleanup_results.pop_front().unwrap_or(Ok(()))
    }

    async fn cleanup_failed_workspace(
        &mut self,
        workspace: &WorkspaceRecord,
    ) -> Result<(), Self::Error> {
        self.failed_cleaned
            .push(workspace.workspace_key.to_string());
        self.cleanup_workspace(workspace, true).await
    }

    async fn remove_workspace(&mut self, workspace: &WorkspaceRecord) -> Result<(), Self::Error> {
        self.removed.push(workspace.workspace_key.to_string());
        self.cleanup_workspace(workspace, true).await
    }

    async fn persist_retry_count(
        &mut self,
        _workspace: &WorkspaceRecord,
        normal_retry_count: u32,
    ) -> Result<(), Self::Error> {
        self.persisted_retry_counts.push(normal_retry_count);
        Ok(())
    }

    async fn persist_retry_exhaustion(
        &mut self,
        _issue: &NormalizedIssue,
        normal_retry_count: u32,
    ) -> Result<(), Self::Error> {
        if let Some(result) = self.persist_retry_exhaustion_results.pop_front() {
            result?;
        }
        self.persisted_retry_exhaustions.push(normal_retry_count);
        Ok(())
    }

    async fn persist_retry_pending(
        &mut self,
        _workspace: &WorkspaceRecord,
        _retry: &RetryEntry,
    ) -> Result<(), Self::Error> {
        if let Some(result) = self.persist_retry_pending_results.pop_front() {
            result?;
        }
        self.persisted_retry_pending += 1;
        Ok(())
    }

    async fn persist_retry_pending_without_workspace(
        &mut self,
        _issue: &NormalizedIssue,
        _retry: &RetryEntry,
    ) -> Result<(), Self::Error> {
        self.persisted_retry_pending_without_workspace += 1;
        Ok(())
    }

    async fn clear_retry_pending(&mut self, issue_id: &IssueId) -> Result<(), Self::Error> {
        self.cleared_retry_pending.push(issue_id.to_string());
        self.clear_retry_pending_results
            .pop_front()
            .unwrap_or(Ok(()))
    }

    fn revoke_issue_resources(&mut self, issue_identifier: &str) {
        self.revoked_issue_resources
            .push(issue_identifier.to_owned());
    }

    async fn persist_interrupt_reason(
        &mut self,
        _workspace: &WorkspaceRecord,
        reason: HarnessInterruptReason,
    ) -> Result<(), Self::Error> {
        if let Some(result) = self.persist_interrupt_results.pop_front() {
            result?;
        }
        self.persisted_interrupt_reasons.push(reason);
        Ok(())
    }

    async fn clear_retry_exhaustion(&mut self, identifier: &str) -> Result<(), Self::Error> {
        self.cleared_retry_exhaustion.push(identifier.to_string());
        Ok(())
    }

    fn retain_failed_workspaces(&self) -> bool {
        self.retain_failed
    }
}

#[derive(Default)]
struct FakeWorker {
    launches: Vec<WorkerStartRequest>,
    updates: VecDeque<WorkerUpdate>,
    aborted: Vec<(String, WorkerAbortReason)>,
    abort_results: VecDeque<Result<(), FakeError>>,
    interrupts: Vec<HarnessInterruptCommand>,
    interrupt_results: VecDeque<Result<WorkerInterruptAcknowledgement, FakeError>>,
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
                started_at: None,
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
        self.abort_results.pop_front().unwrap_or(Ok(()))
    }

    async fn interrupt_worker(
        &mut self,
        command: HarnessInterruptCommand,
    ) -> Result<WorkerInterruptAcknowledgement, Self::Error> {
        self.interrupts.push(command);
        self.interrupt_results
            .pop_front()
            .unwrap_or(Ok(WorkerInterruptAcknowledgement {
                accepted: true,
                detail: None,
                timed_out: false,
            }))
    }
}

#[tokio::test]
async fn eligible_parent_dispatches_once_from_durable_claim_after_restart() {
    let child_id = IssueId::new("child-1").expect("child id should be valid");
    let child_identifier = IssueIdentifier::new("COE-1-child").expect("child identifier");
    let parent_id = IssueId::new("parent-1").expect("parent id should be valid");
    let mut parent = tracker_issue("parent-1", "COE-1", "In Progress", 0);
    parent.sub_issues = vec![TrackerIssueRef {
        id: child_id.to_string(),
        identifier: child_identifier.to_string(),
        title: Some("Child".to_owned()),
        url: None,
        state: "Done".to_owned(),
        state_kind: TrackerIssueStateKind::Completed,
    }];
    let snapshot = HierarchySnapshot::new(&parent);
    let resource = LeaseResource {
        issue_id: child_id.clone(),
        repository_id: CanonicalRepositoryId::new("github:repo").expect("repository id"),
        checkout_generation: "checkout-1".to_owned(),
    };
    let mut durable_state = crate::opensymphony_orchestrator::DurableOrchestratorState {
        hierarchy: BTreeMap::from([(parent_id.clone(), snapshot.clone())]),
        leases: vec![LeaseRecord {
            kind: LeaseKind::LeafWorker,
            resource: resource.clone(),
            owner: LeaseOwner::leaf_worker(&child_id),
            hierarchy_generation: snapshot.generation,
            acquired_at: 1,
            expires_at: None,
            released_at: None,
        }],
        ..Default::default()
    };
    let evidence = ParentEligibilityEvidence {
        hierarchy_generation: snapshot.generation,
        children: vec![ChildEligibilityEvidence {
            child_id: child_id.clone(),
            hierarchy_generation: snapshot.generation,
            orchestrator_terminal: true,
            provider_merge_confirmed: true,
            merge_required: true,
            merge_result_commit: Some("merge-commit".to_owned()),
            merge_result_commits: Vec::new(),
            merge_repository_id: None,
            merge_repository_ids: Vec::new(),
            provider_evidence_at: None,
            provider_evidence_by_issue: Vec::new(),
            resource: Some(resource),
            resources: Vec::new(),
            unresolved_failure: None,
        }],
    };
    let tracker = FakeTracker {
        active: vec![parent.clone()],
        parent_evidence: Some(evidence.clone()),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        durable_state: Some(serde_json::to_value(&durable_state).expect("state should encode")),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("eligible parent should dispatch");
    assert_eq!(scheduler.worker().launches.len(), 1);
    durable_state = serde_json::from_value(
        scheduler
            .workspace()
            .durable_state
            .clone()
            .expect("dispatch should persist durable state"),
    )
    .expect("durable state should decode");
    assert!(durable_state.hierarchy[&parent_id].dispatch_claimed());
    assert!(
        durable_state
            .leases
            .iter()
            .any(|lease| { lease.kind == LeaseKind::AncestorIntegration && lease.active() })
    );
    assert!(
        durable_state
            .leases
            .iter()
            .any(|lease| { lease.kind == LeaseKind::LeafWorker && lease.released_at.is_some() })
    );

    let tracker = FakeTracker {
        active: vec![parent],
        parent_evidence: Some(evidence),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        durable_state: Some(serde_json::to_value(&durable_state).expect("state should encode")),
        ..Default::default()
    };
    let mut restarted = Scheduler::new(
        tracker,
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );
    restarted
        .tick(ts(200))
        .await
        .expect("restart should respect the durable dispatch claim");
    assert!(restarted.worker().launches.is_empty());
}

#[tokio::test]
async fn active_ancestor_fence_is_checked_before_nested_parent_eligibility() {
    let root_id = IssueId::new("root-fence").expect("root id");
    let nested_id = IssueId::new("nested-fence").expect("nested id");
    let canceled_child_id = IssueId::new("canceled-fence").expect("child id");
    let mut root = tracker_issue("root-fence", "COE-FENCE-ROOT", "In Progress", 0);
    root.sub_issues = vec![TrackerIssueRef {
        id: nested_id.to_string(),
        identifier: "COE-FENCE-NESTED".to_owned(),
        title: Some("Nested parent".to_owned()),
        url: None,
        state: "In Progress".to_owned(),
        state_kind: TrackerIssueStateKind::Started,
    }];
    let mut nested = tracker_issue("nested-fence", "COE-FENCE-NESTED", "In Progress", 1);
    nested.parent_id = Some(root_id.to_string());
    nested.sub_issues = vec![TrackerIssueRef {
        id: canceled_child_id.to_string(),
        identifier: "COE-FENCE-CANCELED".to_owned(),
        title: Some("Canceled child".to_owned()),
        url: None,
        state: "Canceled".to_owned(),
        state_kind: TrackerIssueStateKind::Canceled,
    }];

    let mut root_snapshot = HierarchySnapshot::new(&root);
    root_snapshot.mark_dispatched();
    let nested_snapshot = HierarchySnapshot::new(&nested);
    let resource = LeaseResource {
        issue_id: nested_id.clone(),
        repository_id: CanonicalRepositoryId::new("github:fence-repo").expect("repository id"),
        checkout_generation: "checkout-fence".to_owned(),
    };
    let durable_state = crate::opensymphony_orchestrator::DurableOrchestratorState {
        hierarchy: BTreeMap::from([
            (root_id.clone(), root_snapshot),
            (nested_id.clone(), nested_snapshot.clone()),
        ]),
        leases: vec![LeaseRecord {
            kind: LeaseKind::AncestorIntegration,
            resource,
            owner: LeaseOwner::ancestor(&root_id),
            hierarchy_generation: 1,
            acquired_at: 1,
            expires_at: None,
            released_at: None,
        }],
        ..Default::default()
    };
    let tracker = FakeTracker {
        active: vec![root, nested],
        parent_evidence: Some(ParentEligibilityEvidence {
            hierarchy_generation: nested_snapshot.generation,
            children: Vec::new(),
        }),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        durable_state: Some(serde_json::to_value(&durable_state).expect("state should encode")),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("ancestor fence should defer nested parent");

    assert!(scheduler.worker().launches.is_empty());
    let persisted =
        serde_json::from_value::<crate::opensymphony_orchestrator::DurableOrchestratorState>(
            scheduler
                .workspace()
                .durable_state
                .clone()
                .expect("state should remain durable"),
        )
        .expect("state should decode");
    assert!(!persisted.hierarchy[&nested_id].dispatch_intended());
    assert!(
        !persisted
            .leases
            .iter()
            .any(|lease| { lease.active() && lease.owner == LeaseOwner::ancestor(&nested_id) })
    );
}

#[tokio::test]
async fn replan_parent_clears_durable_hierarchy_changed_block_without_releasing_leases() {
    let parent_id = IssueId::new("parent-replan").expect("parent id");
    let mut parent = tracker_issue("parent-replan", "COE-REPLAN", "In Progress", 0);
    parent.sub_issues = vec![TrackerIssueRef {
        id: "child-a".to_owned(),
        identifier: "COE-CHILD".to_owned(),
        title: Some("Child".to_owned()),
        url: None,
        state: "Done".to_owned(),
        state_kind: TrackerIssueStateKind::Completed,
    }];
    let mut snapshot = HierarchySnapshot::new(&parent);
    snapshot.freeze().expect("snapshot should freeze");
    snapshot.reconcile(&[TrackerIssueRef {
        id: "child-b".to_owned(),
        identifier: "COE-CHILD-2".to_owned(),
        title: Some("Replacement child".to_owned()),
        url: None,
        state: "Done".to_owned(),
        state_kind: TrackerIssueStateKind::Completed,
    }]);
    let retained_lease = LeaseRecord {
        kind: LeaseKind::AncestorIntegration,
        resource: LeaseResource {
            issue_id: IssueId::new("child-a").expect("child id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository id"),
            checkout_generation: "checkout-1".to_owned(),
        },
        owner: LeaseOwner::ancestor(&parent_id),
        hierarchy_generation: 2,
        acquired_at: 1,
        expires_at: None,
        released_at: None,
    };
    let durable_state = crate::opensymphony_orchestrator::DurableOrchestratorState {
        hierarchy: BTreeMap::from([(parent_id.clone(), snapshot)]),
        leases: vec![retained_lease.clone()],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        durable_state: Some(serde_json::to_value(&durable_state).expect("state should encode")),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        FakeTracker::default(),
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );

    assert!(
        scheduler
            .replan_parent(&parent_id, ts(100))
            .await
            .expect("replan should persist")
    );
    let state: crate::opensymphony_orchestrator::DurableOrchestratorState = serde_json::from_value(
        scheduler
            .workspace()
            .durable_state
            .clone()
            .expect("replanned state should persist"),
    )
    .expect("state should decode");
    assert_eq!(state.hierarchy[&parent_id].generation, 2);
    assert!(!state.hierarchy[&parent_id].frozen);
    assert!(state.hierarchy[&parent_id].blocked_reason.is_none());
    assert_eq!(state.leases, vec![retained_lease]);
}

#[tokio::test]
async fn replan_parent_target_preserves_tracker_failures_for_retry() {
    let tracker = FakeTracker {
        detail_errors: VecDeque::from([FakeError::rate_limited(Duration::from_secs(5))]),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        tracker,
        FakeWorkspace::default(),
        FakeWorker::default(),
        scheduler_config(),
    );

    let result = scheduler
        .replan_parent_target("COE-REPLAN", 1, ts(100))
        .await;
    assert!(matches!(
        result,
        Err(crate::opensymphony_orchestrator::SchedulerError::Tracker { detail })
            if detail == "rate limited"
    ));
}

#[tokio::test]
async fn replan_parent_target_rejects_a_disappeared_issue_without_aborting_dispatch() {
    let tracker = FakeTracker {
        detail_errors: VecDeque::from([FakeError::not_found()]),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        tracker,
        FakeWorkspace::default(),
        FakeWorker::default(),
        scheduler_config(),
    );

    assert!(
        !scheduler
            .replan_parent_target("COE-MISSING", 1, ts(100))
            .await
            .expect("missing replan targets are recorded as rejected outcomes")
    );
}

#[tokio::test]
async fn launch_failure_persists_retry_metadata_before_returning_error() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-267", "COE-267", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        persist_retry_pending_results: VecDeque::from([Err(FakeError {
            message: "launch retry marker write failed".to_string(),
            category: None,
            retry_after: None,
        })]),
        ..Default::default()
    };
    let worker = FakeWorker {
        launch_results: VecDeque::from([Err(FakeError {
            message: "worker launch failed".to_string(),
            category: None,
            retry_after: None,
        })]),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    assert!(scheduler.tick(ts(100)).await.is_err());
    let issue_id = IssueId::new("lin-267").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("launch failure retry should remain tracked")
            .status(),
        SchedulerStatus::RetryQueued
    );
    assert_eq!(scheduler.workspace().persisted_retry_pending, 0);

    scheduler
        .tick(ts(200))
        .await
        .expect("launch retry marker should be retried");
    assert_eq!(scheduler.workspace().persisted_retry_pending, 1);
}

#[tokio::test]
async fn rate_limit_cooldown_skips_linear_reads_but_keeps_worker_updates_flowing() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-500", "COE-500", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch");

    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .tracker_mut()
        .state_errors
        .push_back(FakeError::rate_limited(Duration::from_secs(60)));
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::TokenUsageUpdate {
            worker_id: first_run.worker_id.clone(),
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 5,
            total_tokens: 35,
        });

    let blocked_snapshot = scheduler
        .tick(ts(30_100))
        .await
        .expect("rate-limited state refresh should not fail the tick");

    assert_eq!(
        blocked_snapshot.daemon.health,
        crate::opensymphony_domain::HealthStatus::Degraded
    );
    assert_eq!(scheduler.tracker().state_requests.len(), 1);
    let execution = scheduler
        .execution(&IssueId::new("lin-500").expect("issue id should be valid"))
        .expect("execution should still exist");
    assert_eq!(
        execution
            .conversation()
            .expect("conversation metadata should exist")
            .total_tokens,
        35
    );

    let active_requests = scheduler.tracker().active_requests;
    let summary_requests = scheduler.tracker().summary_requests;
    let terminal_requests = scheduler.tracker().terminal_requests;
    let detail_requests = scheduler.tracker().detail_requests.len();
    let state_requests = scheduler.tracker().state_requests.len();

    let still_blocked = scheduler
        .tick(ts(35_100))
        .await
        .expect("cooldown should skip Linear reads");

    assert_eq!(
        still_blocked.daemon.health,
        crate::opensymphony_domain::HealthStatus::Degraded
    );
    assert_eq!(scheduler.tracker().active_requests, active_requests);
    assert_eq!(scheduler.tracker().summary_requests, summary_requests);
    assert_eq!(scheduler.tracker().terminal_requests, terminal_requests);
    assert_eq!(scheduler.tracker().detail_requests.len(), detail_requests);
    assert_eq!(scheduler.tracker().state_requests.len(), state_requests);
}

#[tokio::test]
async fn rate_limited_running_state_read_retries_after_cooldown() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-505", "COE-505", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch");
    scheduler
        .tracker_mut()
        .state_errors
        .push_back(FakeError::rate_limited(Duration::from_secs(1)));

    scheduler
        .tick(ts(30_100))
        .await
        .expect("rate-limited state refresh should not fail the tick");
    assert_eq!(scheduler.tracker().state_requests.len(), 1);

    scheduler
        .tick(ts(31_100))
        .await
        .expect("state refresh should retry immediately after cooldown");
    assert_eq!(scheduler.tracker().state_requests.len(), 2);
}

#[tokio::test]
async fn rate_limited_terminal_read_retries_after_cooldown() {
    let tracker = FakeTracker::default();
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should succeed");
    scheduler
        .tracker_mut()
        .terminal_errors
        .push_back(FakeError::rate_limited(Duration::from_secs(1)));

    scheduler
        .tick(ts(300_100))
        .await
        .expect("rate-limited terminal refresh should not fail the tick");
    assert_eq!(scheduler.tracker().terminal_requests, 2);

    scheduler
        .tick(ts(301_100))
        .await
        .expect("terminal refresh should retry immediately after cooldown");
    assert_eq!(scheduler.tracker().terminal_requests, 3);
}

#[tokio::test]
async fn rate_limited_dispatch_discovery_retries_after_cooldown() {
    let tracker = FakeTracker::default();
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should succeed");
    scheduler
        .tracker_mut()
        .summary_errors
        .push_back(FakeError::rate_limited(Duration::from_secs(1)));

    scheduler
        .tick(ts(60_100))
        .await
        .expect("rate-limited dispatch discovery should not fail the tick");
    assert_eq!(scheduler.tracker().summary_requests, 1);

    scheduler
        .tick(ts(61_100))
        .await
        .expect("dispatch discovery should retry immediately after cooldown");
    assert_eq!(scheduler.tracker().summary_requests, 2);
}

#[tokio::test]
async fn dispatch_discovery_uses_sixty_second_cadence_and_selected_detail_reads() {
    let tracker = FakeTracker::default();
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    config.max_concurrent_agents = 2;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should succeed");

    assert_eq!(scheduler.tracker().active_requests, 1);
    assert_eq!(scheduler.tracker().summary_requests, 0);
    assert!(scheduler.tracker().detail_requests.is_empty());

    scheduler.tracker_mut().active = vec![
        tracker_issue("lin-501", "COE-501", "In Progress", 0),
        tracker_issue("lin-502", "COE-502", "In Progress", 1),
    ];

    scheduler
        .tick(ts(5_100))
        .await
        .expect("five-second tick should avoid Linear discovery");
    assert_eq!(scheduler.tracker().summary_requests, 0);
    assert!(scheduler.tracker().detail_requests.is_empty());
    assert!(scheduler.worker().launches.is_empty());

    scheduler
        .tick(ts(60_100))
        .await
        .expect("sixty-second dispatch discovery should run");

    assert_eq!(scheduler.tracker().summary_requests, 1);
    assert_eq!(
        scheduler.tracker().detail_requests,
        vec![vec!["COE-501".to_string(), "COE-502".to_string()]]
    );
    assert_eq!(scheduler.worker().launches.len(), 2);

    scheduler
        .tick(ts(65_100))
        .await
        .expect("next five-second tick should avoid another discovery");
    assert_eq!(scheduler.tracker().summary_requests, 1);

    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("hourly full detail refresh should run");
    assert_eq!(scheduler.tracker().active_requests, 2);
}

#[tokio::test]
async fn dispatch_discovery_skips_blocked_bindings_when_filling_capacity() {
    let tracker = FakeTracker::default();
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_concurrent_agents = 1;
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should succeed");

    let mut blocked = tracker_issue(
        "lin-repo-discovery-blocked",
        "COE-548-BLOCKED",
        "In Progress",
        0,
    );
    blocked.priority = Some(1);
    blocked.project_id = Some("project-id".to_string());
    blocked.labels = vec!["repo:missing".to_string()];
    let mut valid = tracker_issue(
        "lin-repo-discovery-valid",
        "COE-548-VALID",
        "In Progress",
        1,
    );
    valid.priority = Some(2);
    valid.project_id = Some("project-id".to_string());
    valid.labels = vec!["repo:one".to_string()];
    scheduler.tracker_mut().active = vec![blocked, valid];

    scheduler
        .tick(ts(60_100))
        .await
        .expect("blocked discovery candidate should not consume capacity");

    assert_eq!(
        scheduler.tracker().detail_requests,
        vec![
            vec!["COE-548-BLOCKED".to_string()],
            vec!["COE-548-VALID".to_string()],
        ]
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].issue.identifier.as_str(),
        "COE-548-VALID"
    );
}

#[tokio::test]
async fn running_state_polling_uses_lightweight_state_refresh_interval() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-502", "COE-502", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    config.active_states.push("Human Review".to_string());
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch");
    scheduler.tracker_mut().states.insert(
        "lin-502".to_string(),
        tracker_state_snapshot("lin-502", "COE-502", "Human Review", "started", 30_000),
    );

    scheduler
        .tick(ts(5_100))
        .await
        .expect("five-second tick should avoid state refresh");
    assert!(scheduler.tracker().state_requests.is_empty());

    scheduler
        .tick(ts(30_100))
        .await
        .expect("thirty-second tick should refresh running state");

    assert_eq!(scheduler.tracker().active_requests, 1);
    assert_eq!(scheduler.tracker().terminal_requests, 1);
    assert_eq!(scheduler.tracker().summary_requests, 0);
    assert_eq!(
        scheduler.tracker().state_requests,
        vec![vec!["lin-502".to_string()]]
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-502").expect("issue id should be valid"))
            .expect("execution should exist")
            .issue()
            .state
            .name,
        "Human Review"
    );
}

#[tokio::test]
async fn running_state_polling_reconciles_repository_binding_changes() {
    let mut issue = tracker_issue("lin-repo-poll", "COE-548-POLL", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial binding should launch");
    let old_binding = scheduler.worker().launches[0]
        .run
        .repository_binding
        .clone()
        .expect("initial run should carry its binding");
    let mut refreshed = tracker_state_snapshot(
        "lin-repo-poll",
        "COE-548-POLL",
        "In Progress",
        "started",
        30_000,
    );
    refreshed.labels = vec!["repo:two".to_string()];
    scheduler
        .tracker_mut()
        .states
        .insert("lin-repo-poll".to_string(), refreshed);

    scheduler
        .tick(ts(30_100))
        .await
        .expect("frequent running refresh should reconcile binding changes");

    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::BindingSuperseded
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_ne!(
        scheduler.worker().launches[1]
            .run
            .repository_binding
            .as_ref()
            .expect("replacement run should carry its binding")
            .repository
            .id,
        old_binding.repository.id
    );
}

#[tokio::test]
async fn operator_cancel_interrupts_active_worker_once() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-493", "COE-493", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("operator cancel acknowledged".to_string()),
            timed_out: false,
        }));
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch active worker");

    assert!(
        scheduler
            .interrupt_operator_cancel("COE-493", ts(200))
            .await
            .expect("operator cancel should be handled")
    );

    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(
        scheduler.worker().interrupts[0].reason,
        HarnessInterruptReason::OperatorCancel
    );
    let execution = scheduler
        .execution(&IssueId::new("lin-493").expect("issue id should be valid"))
        .expect("execution should still be tracked");
    let interrupt = execution.interrupt().expect("interrupt should be recorded");
    assert_eq!(interrupt.status, HarnessInterruptStatus::Acknowledged);
    assert_eq!(
        interrupt.command.reason,
        HarnessInterruptReason::OperatorCancel
    );
    assert!(
        execution
            .conversation()
            .expect("conversation should remain attached")
            .recent_activity
            .iter()
            .any(|event| event.summary.contains("operator_cancel"))
    );
    let activity_ids: Vec<_> = execution
        .conversation()
        .expect("conversation should remain attached")
        .recent_activity
        .iter()
        .map(|event| event.event_id.as_str())
        .collect();
    assert!(
        activity_ids
            .iter()
            .any(|event_id| event_id.starts_with("operator_cancel-interrupt-acknowledged-")),
        "operator cancel acknowledgement should use a reason-specific event id; activity IDs: {activity_ids:?}"
    );

    scheduler
        .interrupt_operator_cancel("COE-493", ts(300))
        .await
        .expect("repeated operator cancel should be idempotent");
    assert_eq!(scheduler.worker().interrupts.len(), 1);
}

#[tokio::test]
async fn acknowledged_operator_cancel_releases_without_retrying_cancelled_run() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-505", "COE-505", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("operator cancel acknowledged".to_string()),
            timed_out: false,
        }));
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch active worker");
    let first_run = scheduler.worker().launches[0].run.clone();

    assert!(
        scheduler
            .interrupt_operator_cancel("COE-505", ts(200))
            .await
            .expect("operator cancel should be handled")
    );
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Cancelled,
                ts(300),
                Some("Codex turn interrupted by operator cancel".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(300))
        .await
        .expect("cancelled worker should release without retry");
    let execution = scheduler
        .execution(&IssueId::new("lin-505").expect("issue id should be valid"))
        .expect("execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    assert!(execution.retry().is_none());
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::Cancelled);
        }
        other => panic!("expected released state, got {other:?}"),
    }

    scheduler
        .tick(ts(1_300))
        .await
        .expect("active tracker refresh should not restart an operator-cancelled run");
    assert_eq!(scheduler.worker().launches.len(), 1);
}

#[tokio::test]
async fn acknowledged_operator_cancel_releases_without_retrying_completed_run() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-506", "COE-506", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("operator cancel acknowledged".to_string()),
            timed_out: false,
        }));
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch active worker");
    let first_run = scheduler.worker().launches[0].run.clone();

    assert!(
        scheduler
            .interrupt_operator_cancel("COE-506", ts(200))
            .await
            .expect("operator cancel should be handled")
    );
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Succeeded,
                ts(300),
                Some("Codex turn completed after operator cancel acknowledgement".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(300))
        .await
        .expect("completed worker after acknowledged cancel should release without retry");
    let execution = scheduler
        .execution(&IssueId::new("lin-506").expect("issue id should be valid"))
        .expect("execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    assert!(execution.retry().is_none());
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::Cancelled);
        }
        other => panic!("expected released state, got {other:?}"),
    }

    scheduler
        .tick(ts(1_300))
        .await
        .expect("active tracker refresh should not restart an operator-cancelled run");
    assert_eq!(scheduler.worker().launches.len(), 1);
}

#[tokio::test]
async fn timed_out_operator_cancel_releases_late_cancelled_run_without_retry() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-507", "COE-507", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("operator cancel acknowledgement timed out".to_string()),
            timed_out: true,
        }));
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch active worker");
    let first_run = scheduler.worker().launches[0].run.clone();

    assert!(
        scheduler
            .interrupt_operator_cancel("COE-507", ts(200))
            .await
            .expect("operator cancel should be handled")
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-507").expect("issue id should be valid"))
            .expect("execution should remain tracked")
            .interrupt()
            .expect("interrupt should be recorded")
            .status,
        HarnessInterruptStatus::TimedOut
    );

    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Cancelled,
                ts(300),
                Some("worker stopped after operator cancel".to_string()),
                None,
            ),
        });
    scheduler
        .tick(ts(300))
        .await
        .expect("late cancellation should release without retry");

    let execution = scheduler
        .execution(&IssueId::new("lin-507").expect("issue id should be valid"))
        .expect("execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    assert!(execution.retry().is_none());
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::Cancelled);
        }
        other => panic!("expected released state, got {other:?}"),
    }
}

#[tokio::test]
async fn merging_supersedes_human_review_polling_once_and_continues_same_issue() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-492", "COE-492", "Human Review", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker {
        interrupt_results: VecDeque::from([
            Ok(WorkerInterruptAcknowledgement {
                accepted: false,
                detail: Some("first interrupt attempt failed".to_string()),
                timed_out: false,
            }),
            Ok(WorkerInterruptAcknowledgement {
                accepted: true,
                detail: None,
                timed_out: false,
            }),
        ]),
        ..Default::default()
    };
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    config.active_states.push("Human Review".to_string());
    config.active_states.push("Merging".to_string());
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should dispatch Human Review polling");
    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler.tracker_mut().states.insert(
        "lin-492".to_string(),
        tracker_state_snapshot("lin-492", "COE-492", "Merging", "started", 30_000),
    );

    scheduler
        .tick(ts(30_100))
        .await
        .expect("Merging state refresh should interrupt Human Review polling");

    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(
        scheduler.worker().interrupts[0].reason,
        HarnessInterruptReason::TrackerMergingSupersedesHumanReview
    );
    let execution = scheduler
        .execution(&IssueId::new("lin-492").expect("issue id should be valid"))
        .expect("execution should still be active");
    assert_eq!(execution.issue().state.name, "Merging");
    let interrupt = execution.interrupt().expect("interrupt should be recorded");
    assert_eq!(interrupt.status, HarnessInterruptStatus::Failed);
    assert_eq!(
        interrupt.command.reason,
        HarnessInterruptReason::TrackerMergingSupersedesHumanReview
    );
    assert!(
        execution
            .conversation()
            .expect("conversation should remain attached")
            .recent_activity
            .iter()
            .any(|event| event
                .summary
                .contains("tracker_merging_supersedes_human_review"))
    );
    assert_eq!(
        execution
            .conversation()
            .expect("conversation should remain attached")
            .recent_activity
            .iter()
            .filter(|event| event.kind == "scheduler.interrupt_requested")
            .count(),
        1
    );

    scheduler
        .tick(ts(60_100))
        .await
        .expect("failed Merging interrupt should be retried");
    assert_eq!(scheduler.worker().interrupts.len(), 2);
    assert_eq!(scheduler.worker().launches.len(), 1);
    let execution = scheduler
        .execution(&IssueId::new("lin-492").expect("issue id should be valid"))
        .expect("execution should still be active");
    assert_eq!(
        execution
            .interrupt()
            .expect("interrupt should remain recorded")
            .status,
        HarnessInterruptStatus::Acknowledged
    );
    assert_eq!(
        execution
            .conversation()
            .expect("conversation should remain attached")
            .recent_activity
            .iter()
            .filter(|event| event.kind == "scheduler.interrupt_requested")
            .count(),
        2
    );

    scheduler
        .tick(ts(90_100))
        .await
        .expect("acknowledged Merging interrupt should stay idempotent");
    assert_eq!(scheduler.worker().interrupts.len(), 2);

    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Cancelled,
                ts(60_200),
                Some("Human Review polling interrupted by Merging".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(60_200))
        .await
        .expect("interrupted worker should queue same-issue continuation");
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-492").expect("issue id should be valid"))
            .expect("execution should remain tracked")
            .status(),
        SchedulerStatus::RetryQueued
    );

    scheduler
        .tick(ts(61_300))
        .await
        .expect("same issue should continue after interrupt handling");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.worker().launches[1].issue.identifier.as_str(),
        "COE-492"
    );
    assert_eq!(
        scheduler.worker().launches[1].issue.state.name.as_str(),
        "Merging"
    );
}

#[tokio::test]
async fn recovered_human_review_run_uses_restored_harness_kind_for_merging_interrupt() {
    let recovered_worker_id =
        WorkerId::new("worker-recovered-codex").expect("worker id should be valid");
    let recovered_workspace = workspace_record("COE-492", "/tmp/recovered/COE-492");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-492", "COE-492", "Human Review", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-492", "COE-492", "Human Review"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("codex_app_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: None,
            }),
        }],
        records: HashMap::from([("lin-492".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    config.active_states.push("Human Review".to_string());
    config.active_states.push("Merging".to_string());
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup recovery should restore active Human Review worker");
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-492").expect("issue id should be valid"))
            .expect("recovered execution should exist")
            .status(),
        SchedulerStatus::Running
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].run.worker_id,
        recovered_worker_id
    );
    assert_eq!(
        scheduler.worker().launches[0].route.harness_kind,
        "codex_app_server"
    );
    assert!(
        scheduler.worker().launches[0].memory_grant_registry_recovered,
        "reattached workers must rotate process-local memory grants"
    );

    scheduler.tracker_mut().states.insert(
        "lin-492".to_string(),
        tracker_state_snapshot("lin-492", "COE-492", "Merging", "started", 30_000),
    );
    scheduler
        .tick(ts(30_100))
        .await
        .expect("Merging state refresh should interrupt recovered Human Review polling");

    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(
        scheduler.worker().interrupts[0].harness_kind,
        "codex_app_server"
    );
    assert_eq!(
        scheduler.worker().interrupts[0].reason,
        HarnessInterruptReason::TrackerMergingSupersedesHumanReview
    );
    assert_eq!(
        scheduler.worker().interrupts[0].conversation_id,
        conversation(&recovered_worker_id).conversation_id
    );
}

#[tokio::test]
async fn recovered_retry_run_restores_retry_state_before_launch() {
    let recovered_worker_id =
        WorkerId::new("worker-recovered-retry").expect("recovered worker id should be valid");
    let recovered_workspace = workspace_record("COE-494", "/tmp/recovered/COE-494");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-494", "COE-494", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-494", "COE-494", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 1,
                repository_binding: None,
            }),
        }],
        records: HashMap::from([("lin-494".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("startup recovery should launch the recovered retry");

    let execution = scheduler
        .execution(&IssueId::new("lin-494").expect("issue id should be valid"))
        .expect("recovered execution should exist");
    let run = execution
        .current_run()
        .expect("recovered execution should have a running attempt");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(run.attempt.map(|attempt| attempt.get()), Some(1));
    assert_eq!(run.normal_retry_count, 1);
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().launches[0].run.attempt, run.attempt);
}

#[tokio::test]
async fn dispatch_discovery_skips_candidates_missing_or_inactive_after_detail_refresh() {
    let tracker = FakeTracker::default();
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup full refresh should succeed");

    scheduler.tracker_mut().active = vec![tracker_issue("lin-503", "COE-503", "In Progress", 0)];
    scheduler.tracker_mut().detail_issues = Some(Vec::new());

    scheduler
        .tick(ts(60_100))
        .await
        .expect("missing detail should skip stale summary candidate");

    assert_eq!(
        scheduler.tracker().detail_requests,
        vec![vec!["COE-503".to_string()]]
    );
    assert!(scheduler.worker().launches.is_empty());

    scheduler.tracker_mut().detail_issues =
        Some(vec![tracker_issue("lin-503", "COE-503", "Todo", 0)]);

    scheduler
        .tick(ts(120_100))
        .await
        .expect("inactive detail should skip stale summary candidate");

    assert_eq!(scheduler.tracker().detail_requests.len(), 2);
    assert!(scheduler.worker().launches.is_empty());
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
    assert!(scheduler.workspace().persisted_retry_counts.is_empty());
    assert_eq!(scheduler.workspace().persisted_retry_pending, 1);

    scheduler
        .tick(ts(1_300))
        .await
        .expect("third tick should redispatch the issue");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(scheduler.workspace().persisted_retry_counts, vec![1]);
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
async fn retry_limit_parks_successful_continuation_while_tracker_is_active() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-269", "COE-269", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        retain_failed: true,
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
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
                Some("first continuation should queue".to_owned()),
                None,
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("first successful run should queue continuation");
    scheduler
        .tick(ts(1_300))
        .await
        .expect("continuation should dispatch once");

    let second_run = scheduler.worker().launches[1].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: second_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &second_run,
                WorkerOutcomeKind::Succeeded,
                ts(1_400),
                Some("successful final continuation should remain successful".to_owned()),
                None,
            ),
        });
    scheduler
        .tick(ts(1_400))
        .await
        .expect("successful final continuation should release as retry exhausted");

    let issue_id = IssueId::new("lin-269").expect("issue id should be valid");
    assert!(matches!(
        scheduler
            .execution(&issue_id)
            .expect("released execution should remain recorded")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
    scheduler
        .tick(ts(61_400))
        .await
        .expect("retry-exhausted continuation should remain parked");
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert!(scheduler.workspace().cleaned.is_empty());
    assert_eq!(scheduler.workspace().persisted_retry_exhaustions, vec![1]);
}

#[tokio::test]
async fn worker_finish_rechecks_tracker_state_before_continuation_retry() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-492", "COE-492", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch");

    scheduler.tracker_mut().states.insert(
        "lin-492".to_string(),
        tracker_state_snapshot("lin-492", "COE-492", "Done", "completed", 200),
    );
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
        .expect("finish tick should release terminal issue");

    let issue_id = IssueId::new("lin-492").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    assert!(execution.retry().is_none());
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerTerminal);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-492".to_string(), true)]
    );
    assert_eq!(
        scheduler.tracker().state_requests,
        vec![vec!["lin-492".to_string()]]
    );
}

#[tokio::test]
async fn retry_state_survives_a_failed_manifest_persistence() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-269", "COE-269", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler.tick(ts(100)).await.expect("initial dispatch");
    let first_run = scheduler.worker().launches[0].run.clone();
    scheduler
        .workspace_mut()
        .persist_retry_pending_results
        .push_back(Err(FakeError {
            message: "manifest write failed".to_string(),
            category: None,
            retry_after: None,
        }));
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
                None,
            ),
        });

    assert!(
        scheduler.tick(ts(200)).await.is_err(),
        "manifest persistence failure should be surfaced"
    );
    let issue_id = IssueId::new("lin-269").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should remain");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(
        execution
            .retry()
            .expect("retry state should remain in memory")
            .normal_retry_count,
        1
    );

    scheduler
        .tick(ts(300))
        .await
        .expect("the next tick should retry persistence");
    assert_eq!(scheduler.workspace().persisted_retry_pending, 1);
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("execution should remain queued")
            .status(),
        SchedulerStatus::RetryQueued
    );
}

#[tokio::test]
async fn exhaustion_persistence_failure_keeps_released_execution_tracked() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-270", "COE-270", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        persist_retry_exhaustion_results: VecDeque::from([Err(FakeError {
            message: "exhaustion marker write failed".to_string(),
            category: None,
            retry_after: None,
        })]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(0);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler.tick(ts(100)).await.expect("initial dispatch");
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
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("released execution should survive deferred marker persistence");
    let issue_id = IssueId::new("lin-270").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("exhausted execution should remain tracked")
            .status(),
        SchedulerStatus::Released
    );
    assert!(scheduler.workspace().persisted_retry_exhaustions.is_empty());
    assert!(
        scheduler.workspace().failed_cleaned.is_empty(),
        "workspace cleanup must wait for durable retry exhaustion"
    );

    scheduler
        .tick(ts(300))
        .await
        .expect("deferred exhaustion marker should retry");
    assert_eq!(scheduler.workspace().persisted_retry_exhaustions, vec![0]);
    assert_eq!(
        scheduler.workspace().failed_cleaned,
        vec!["COE-270".to_string()]
    );
}

#[tokio::test]
async fn worker_updates_after_persistence_failure_are_not_dropped() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-271-a", "COE-271-A", "In Progress", 0),
            tracker_issue("lin-271-b", "COE-271-B", "In Progress", 1),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        persist_retry_pending_results: VecDeque::from([Err(FakeError {
            message: "first pending marker write failed".to_string(),
            category: None,
            retry_after: None,
        })]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler.tick(ts(100)).await.expect("initial dispatch");
    let first_run = scheduler.worker().launches[0].run.clone();
    let second_run = scheduler.worker().launches[1].run.clone();
    scheduler.worker_mut().updates.extend([
        WorkerUpdate::Finished {
            worker_id: first_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &first_run,
                WorkerOutcomeKind::Failed,
                ts(200),
                Some("first worker failed".to_string()),
                None,
            ),
        },
        WorkerUpdate::Finished {
            worker_id: second_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &second_run,
                WorkerOutcomeKind::Failed,
                ts(200),
                Some("second worker failed".to_string()),
                None,
            ),
        },
    ]);

    assert!(scheduler.tick(ts(200)).await.is_err());
    for issue_id in ["lin-271-a", "lin-271-b"] {
        assert_eq!(
            scheduler
                .execution(&IssueId::new(issue_id).expect("issue id should be valid"))
                .expect("both finished executions should remain tracked")
                .status(),
            SchedulerStatus::RetryQueued
        );
    }
    assert_eq!(scheduler.workspace().persisted_retry_pending, 1);

    scheduler
        .tick(ts(300))
        .await
        .expect("the deferred first marker should retry");
    assert_eq!(scheduler.workspace().persisted_retry_pending, 2);
}

#[tokio::test]
async fn terminal_cleanup_failure_after_worker_finish_keeps_execution_for_retry() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-541", "COE-541", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        cleanup_results: VecDeque::from([
            Err(FakeError {
                message: "Codex archive failed".to_string(),
                category: None,
                retry_after: None,
            }),
            Err(FakeError {
                message: "Codex archive still unavailable".to_string(),
                category: None,
                retry_after: None,
            }),
        ]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch");
    scheduler.tracker_mut().states.insert(
        "lin-541".to_string(),
        tracker_state_snapshot("lin-541", "COE-541", "Done", "completed", 200),
    );
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
        .expect("archive failure should not drop the released execution");
    let issue_id = IssueId::new("lin-541").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("execution should remain tracked")
            .status(),
        SchedulerStatus::Released
    );

    scheduler.tracker_mut().active.clear();
    scheduler.tracker_mut().terminal = vec![tracker_issue("lin-541", "COE-541", "Done", 0)];
    scheduler
        .tick(ts(300_200))
        .await
        .expect("terminal reconciliation cleanup failure should be non-fatal");
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-541".to_string(), true), ("COE-541".to_string(), true),]
    );

    scheduler
        .tick(ts(600_300))
        .await
        .expect("terminal reconciliation should retry cleanup again");
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![
            ("COE-541".to_string(), true),
            ("COE-541".to_string(), true),
            ("COE-541".to_string(), true),
        ]
    );
}

#[tokio::test]
async fn retry_exhausted_cleanup_policy_survives_terminal_transition() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-542", "COE-542", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        cleanup_results: VecDeque::from([Ok(())]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler.tick(ts(100)).await.expect("initial dispatch");
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
                Some("queue continuation".to_string()),
                None,
            ),
        });
    scheduler.tick(ts(200)).await.expect("queue continuation");
    scheduler
        .tick(ts(1_300))
        .await
        .expect("dispatch continuation");

    let second_run = scheduler.worker().launches[1].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: second_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &second_run,
                WorkerOutcomeKind::Succeeded,
                ts(1_400),
                Some("successful final run should not exhaust retry budget".to_string()),
                None,
            ),
        });
    scheduler
        .tick(ts(1_400))
        .await
        .expect("successful final run should park as retry exhausted");

    scheduler.tracker_mut().active.clear();
    scheduler.tracker_mut().terminal = vec![tracker_issue("lin-542", "COE-542", "Done", 0)];
    scheduler
        .tick(ts(300_200))
        .await
        .expect("terminal transition should reconcile cleanup");

    assert_eq!(
        scheduler.workspace().failed_cleaned,
        vec!["COE-542".to_string()]
    );
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-542".to_string(), true)]
    );
    assert_eq!(scheduler.workspace().persisted_retry_exhaustions, vec![1]);
    assert_eq!(
        scheduler.workspace().revoked_issue_resources,
        vec!["COE-542".to_string(), "COE-542".to_string()]
    );
    assert_eq!(
        scheduler.workspace().cleared_retry_exhaustion,
        vec!["COE-542".to_string()]
    );
    assert!(matches!(
        scheduler
            .execution(&IssueId::new("lin-542").expect("issue id should be valid"))
            .expect("execution should remain recorded")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::TrackerTerminal,
            ..
        }
    ));
    assert!(
        scheduler
            .execution(&IssueId::new("lin-542").expect("issue id should be valid"))
            .expect("execution should remain recorded")
            .workspace()
            .is_none()
    );

    scheduler
        .tick(ts(600_300))
        .await
        .expect("terminal reconciliation should remain idempotent");
    assert_eq!(
        scheduler.workspace().cleared_retry_exhaustion,
        vec!["COE-542".to_string()]
    );
    assert!(matches!(
        scheduler
            .execution(&IssueId::new("lin-542").expect("issue id should be valid"))
            .expect("execution should remain recorded")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::TrackerTerminal,
            ..
        }
    ));
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
async fn blocked_binding_preserves_a_due_retry_execution() {
    let mut issue = tracker_issue("lin-repo-retry", "COE-548-RETRY", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial repository-bound run should launch");
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
                Some("retry".to_string()),
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("failure should queue retry");
    let retry_before_block = scheduler
        .execution(&IssueId::new("lin-repo-retry").expect("issue id should be valid"))
        .expect("execution should remain tracked")
        .retry()
        .expect("retry should exist")
        .clone();

    scheduler.tracker_mut().active[0].labels = vec!["repo:missing".to_string()];
    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("blocked retry should remain owned");

    let execution = scheduler
        .execution(&IssueId::new("lin-repo-retry").expect("issue id should be valid"))
        .expect("execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(execution.retry(), Some(&retry_before_block));
    assert_eq!(
        scheduler
            .workspace()
            .persisted_retry_pending_without_workspace,
        1
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
}

#[tokio::test]
async fn queued_repository_binding_change_rematerializes_before_retry() {
    let mut issue = tracker_issue(
        "lin-repo-retry-rebind",
        "COE-548-RETRY-REBIND",
        "In Progress",
        0,
    );
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial repository-bound run should launch");
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
                Some("retry".to_string()),
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("failure should queue retry");
    let retry_before_rebind = scheduler
        .execution(&IssueId::new("lin-repo-retry-rebind").expect("issue id should be valid"))
        .expect("execution should remain tracked")
        .retry()
        .expect("retry should exist")
        .clone();

    scheduler.tracker_mut().active[0].labels = vec!["repo:two".to_string()];
    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("binding change should rematerialize the queued retry");

    let execution = scheduler
        .execution(&IssueId::new("lin-repo-retry-rebind").expect("issue id should be valid"))
        .expect("replacement execution should remain tracked");
    assert_eq!(scheduler.workspace().removed, vec!["COE-548-RETRY-REBIND"]);
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(
        execution
            .current_run()
            .expect("replacement run should exist")
            .normal_retry_count,
        retry_before_rebind.normal_retry_count
    );
    assert_eq!(
        execution
            .current_run()
            .and_then(|run| run.repository_binding.as_ref())
            .map(|binding| binding.alias.as_str()),
        Some("two")
    );
    assert_eq!(scheduler.workspace().persisted_retry_pending, 2);
    assert_eq!(
        scheduler.workspace().cleared_retry_pending,
        vec![
            "lin-repo-retry-rebind".to_string(),
            "lin-repo-retry-rebind".to_string(),
            "lin-repo-retry-rebind".to_string(),
            "lin-repo-retry-rebind".to_string()
        ]
    );
}

#[tokio::test]
async fn queued_rebind_keeps_retry_when_replacement_workspace_materialization_fails() {
    let mut issue = tracker_issue(
        "lin-repo-retry-ensure-failure",
        "COE-548-RETRY-ENSURE-FAILURE",
        "In Progress",
        0,
    );
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial repository-bound run should launch");
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
                Some("retry".to_string()),
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("failure should queue retry");
    let retry_before_rebind = scheduler
        .execution(&IssueId::new("lin-repo-retry-ensure-failure").expect("valid issue id"))
        .expect("execution should remain tracked")
        .retry()
        .expect("retry should exist")
        .clone();

    scheduler
        .workspace_mut()
        .ensure_results
        .push_back(Err(FakeError {
            message: "replacement workspace unavailable".to_string(),
            category: None,
            retry_after: None,
        }));
    scheduler.tracker_mut().active[0].labels = vec!["repo:two".to_string()];
    assert!(scheduler.tick(ts(3_600_100)).await.is_err());

    let execution = scheduler
        .execution(&IssueId::new("lin-repo-retry-ensure-failure").expect("valid issue id"))
        .expect("replacement execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(execution.retry(), Some(&retry_before_rebind));
    assert!(execution.workspace().is_none());
    assert_eq!(
        scheduler.workspace().removed,
        vec!["COE-548-RETRY-ENSURE-FAILURE"]
    );
    assert_eq!(
        scheduler
            .workspace()
            .persisted_retry_pending_without_workspace,
        1
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
}

#[tokio::test]
async fn queued_binding_change_reconciles_before_retry_is_due() {
    let mut issue = tracker_issue(
        "lin-repo-retry-poll",
        "COE-548-RETRY-POLL",
        "In Progress",
        0,
    );
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.retry_policy.failure_base_delay_ms = DurationMs::new(120_000);
    config.retry_policy.max_backoff_ms = DurationMs::new(120_000);
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial repository-bound run should launch");
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
                Some("retry".to_string()),
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("failure should queue retry");
    let retry = scheduler
        .execution(&IssueId::new("lin-repo-retry-poll").expect("valid issue id"))
        .expect("execution should remain tracked")
        .retry()
        .expect("retry should exist")
        .clone();
    assert!(retry.due_at > ts(30_100));

    let mut refreshed = tracker_state_snapshot(
        "lin-repo-retry-poll",
        "COE-548-RETRY-POLL",
        "In Progress",
        "started",
        30_000,
    );
    refreshed.labels = vec!["repo:two".to_string()];
    scheduler
        .tracker_mut()
        .states
        .insert("lin-repo-retry-poll".to_string(), refreshed);
    scheduler
        .tick(ts(30_100))
        .await
        .expect("queued binding should reconcile on the frequent refresh");

    let execution = scheduler
        .execution(&IssueId::new("lin-repo-retry-poll").expect("valid issue id"))
        .expect("execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(execution.retry(), Some(&retry));
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.workspace().removed, vec!["COE-548-RETRY-POLL"]);
    assert_eq!(
        execution
            .issue()
            .repository_binding
            .as_ref()
            .and_then(RepositoryBindingOutcome::resolved_binding)
            .map(|binding| binding.alias.as_str()),
        Some("two")
    );

    scheduler
        .tick(ts(60_100))
        .await
        .expect("running retry reconciliation must not starve discovery");
    assert_eq!(scheduler.tracker().summary_requests, 1);
}

#[tokio::test]
async fn same_repository_recovery_supersedes_stale_binding_generations() {
    let recovered_worker_id =
        WorkerId::new("worker-same-repository-generation").expect("worker id should be valid");
    let recovered_workspace = workspace_record(
        "COE-548-SAME-REPOSITORY-GENERATION",
        "/tmp/recovered/COE-548-SAME-REPOSITORY-GENERATION",
    );
    let mut persisted_binding = match repository_routing().resolve(
        &["repo:one".to_string()],
        Some("project-id"),
        None,
        false,
    ) {
        RepositoryBindingOutcome::Resolved(binding) => binding,
        outcome => panic!("expected persisted binding, got {outcome:?}"),
    };
    persisted_binding.config_generation = "config-before-restart".to_string();
    persisted_binding.inventory_generation = "inventory-before-restart".to_string();
    let tracker = FakeTracker {
        active: vec![{
            let mut issue = tracker_issue(
                "lin-same-repository-generation",
                "COE-548-SAME-REPOSITORY-GENERATION",
                "In Progress",
                0,
            );
            issue.project_id = Some("project-id".to_string());
            issue.labels = vec!["repo:one".to_string()];
            issue
        }],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue(
                "lin-same-repository-generation",
                "COE-548-SAME-REPOSITORY-GENERATION",
                "In Progress",
            ),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: Some(persisted_binding.clone()),
            }),
        }],
        records: HashMap::from([(
            "lin-same-repository-generation".to_string(),
            recovered_workspace,
        )]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("same-repository recovery should reuse the persisted run");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    let run = &scheduler.worker().launches[0].run;
    assert_eq!(
        run.repository_binding
            .as_ref()
            .map(|binding| binding.config_generation.as_str()),
        Some("config-before-restart")
    );
    assert_eq!(
        run.repository_binding
            .as_ref()
            .map(|binding| binding.inventory_generation.as_str()),
        Some("inventory-before-restart")
    );
    assert_eq!(
        scheduler.worker().launches[1]
            .run
            .repository_binding
            .as_ref()
            .map(|binding| binding.config_generation.as_str()),
        Some("config-test")
    );
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-same-repository-generation").expect("issue id"))
            .expect("recovered execution should remain tracked")
            .issue()
            .repository_binding
            .as_ref()
            .and_then(RepositoryBindingOutcome::resolved_binding)
            .map(|binding| binding.config_generation.as_str()),
        Some("config-test")
    );
}

#[tokio::test]
async fn bounded_dispatch_detail_rejects_issues_outside_configured_project() {
    let mut issue = tracker_issue("lin-project-move", "COE-548-PROJECT-MOVE", "Todo", 0);
    issue.project_slug = Some("configured-project".to_string());
    issue.blocked_by = vec![TrackerIssueBlocker {
        id: "lin-blocker".to_string(),
        identifier: "COE-548-BLOCKER".to_string(),
        title: "Blocking issue".to_string(),
        state: TrackerIssueState {
            id: "in-progress".to_string(),
            name: "In Progress".to_string(),
            tracker_type: "started".to_string(),
            kind: TrackerIssueStateKind::Started,
        },
    }];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.tracker_project_id = Some("stale-project-id".to_string());
    config.tracker_project_slug = Some("configured-project".to_string());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial full refresh should complete");
    scheduler.tracker_mut().active[0].blocked_by.clear();
    scheduler.tracker_mut().active[0].state = "In Progress".to_string();
    scheduler.tracker_mut().active[0].state_kind = TrackerIssueStateKind::Started;
    scheduler.tracker_mut().active[0].project_slug = Some("other-project".to_string());

    scheduler
        .tick(ts(60_100))
        .await
        .expect("out-of-project detail should be skipped");

    assert_eq!(
        scheduler.tracker().detail_requests,
        vec![vec!["COE-548-PROJECT-MOVE".to_string()]]
    );
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn conflicting_scalar_and_vector_project_slugs_fail_closed() {
    let mut issue = tracker_issue(
        "lin-project-slug-conflict",
        "COE-548-SLUG-CONFLICT",
        "Todo",
        0,
    );
    issue.project_slug = Some("configured-project".to_string());
    issue.blocked_by = vec![TrackerIssueBlocker {
        id: "lin-blocker".to_string(),
        identifier: "COE-548-BLOCKER".to_string(),
        title: "Blocking issue".to_string(),
        state: TrackerIssueState {
            id: "in-progress".to_string(),
            name: "In Progress".to_string(),
            tracker_type: "started".to_string(),
            kind: TrackerIssueStateKind::Started,
        },
    }];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.tracker_project_slug = Some("configured-project".to_string());
    config.tracker_project_slugs = vec!["other-project".to_string()];
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("conflicting project scope should be ignored");

    assert!(scheduler.tracker().detail_requests.is_empty());
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn project_scope_drift_supersedes_the_run_without_removing_the_checkout() {
    let mut issue = tracker_issue(
        "lin-project-scope",
        "COE-548-PROJECT-SCOPE",
        "In Progress",
        0,
    );
    issue.project_id = Some("project-id".to_string());
    issue.project_slug = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    issue.sub_issues.clear();
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut routing = repository_routing();
    let repository_id = routing
        .inventory
        .get("one")
        .expect("repository one should exist")
        .identity
        .id
        .clone();
    routing.project_repositories.insert(
        "project-two".to_string(),
        [repository_id].into_iter().collect(),
    );
    routing.active_projects.insert("project-two".to_string());
    let mut config = scheduler_config();
    config.tracker_project_id = Some("project-id".to_string());
    config.tracker_project_slug = Some("project-id".to_string());
    config.tracker_project_ids = vec!["project-id".to_string(), "project-two".to_string()];
    config.tracker_project_slugs = vec!["project-id".to_string(), "project-two".to_string()];
    config.repository_routing = Some(routing);
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler.tick(ts(100)).await.expect("initial dispatch");
    scheduler.tracker_mut().active[0].project_id = Some("project-two".to_string());
    scheduler.tracker_mut().active[0].project_slug = Some("project-two".to_string());

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
                Some("worker finished before project drift".to_owned()),
                None,
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("finished worker should queue a continuation retry");
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-project-scope").expect("valid issue id"))
            .expect("execution should remain tracked")
            .status(),
        SchedulerStatus::RetryQueued
    );

    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("project-only drift should supersede the idle retry");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert!(scheduler.worker().aborted.is_empty());
    assert!(scheduler.workspace().removed.is_empty());
    assert_eq!(
        scheduler.workspace().revoked_issue_resources,
        vec!["COE-548-PROJECT-SCOPE".to_string()]
    );

    scheduler
        .tick(ts(3_600_200))
        .await
        .expect("replacement should keep using the retained checkout");
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert!(scheduler.workspace().removed.is_empty());
    assert!(
        scheduler
            .execution(&IssueId::new("lin-project-scope").expect("issue id"))
            .expect("replacement execution should remain tracked")
            .workspace()
            .is_some()
    );
}

#[tokio::test]
async fn tracker_inactive_failed_execution_reopens_after_reactivation() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-271", "COE-271", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    let run = scheduler.worker().launches[0].run.clone();
    scheduler.tracker_mut().states.insert(
        "lin-271".to_owned(),
        tracker_state_snapshot("lin-271", "COE-271", "Backlog", "backlog", 200),
    );
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &run,
                WorkerOutcomeKind::Failed,
                ts(200),
                Some("failed while tracker was inactive".to_owned()),
                Some("boom".to_owned()),
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("inactive failure should release the execution");

    let issue_id = IssueId::new("lin-271").expect("issue id should be valid");
    assert!(matches!(
        scheduler
            .execution(&issue_id)
            .expect("execution should remain")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::TrackerInactive,
            ..
        }
    ));

    scheduler.tracker_mut().active = vec![tracker_issue("lin-271", "COE-271", "In Progress", 0)];
    scheduler.tracker_mut().states.insert(
        "lin-271".to_owned(),
        tracker_state_snapshot("lin-271", "COE-271", "In Progress", "started", 60_200),
    );
    scheduler
        .tick(ts(60_200))
        .await
        .expect("reactivated failed execution should dispatch");
    assert_eq!(scheduler.worker().launches.len(), 2);
}

#[tokio::test]
async fn retry_limit_counts_failures_and_prevents_redispatch() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-270", "COE-270", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        cleanup_results: VecDeque::from([
            Err(FakeError {
                message: "archive unavailable".to_string(),
                category: None,
                retry_after: None,
            }),
            Ok(()),
        ]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    let issue_id = IssueId::new("lin-270").expect("issue id should be valid");
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
                Some("first failure".to_owned()),
                Some("boom".to_owned()),
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("first failure should queue retry");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("execution should exist")
            .retry()
            .expect("retry should be queued")
            .normal_retry_count,
        1
    );

    scheduler
        .tick(ts(10_200))
        .await
        .expect("retry dispatch should succeed");
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
                Some("second failure".to_owned()),
                Some("still broken".to_owned()),
            ),
        });
    scheduler
        .tick(ts(10_400))
        .await
        .expect("retry exhaustion should release the execution");

    let released = scheduler
        .execution(&issue_id)
        .expect("released execution should remain recorded");
    assert_eq!(released.status(), SchedulerStatus::Released);
    assert!(matches!(
        released.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-270".to_string(), true)]
    );

    scheduler
        .tick(ts(3_600_200))
        .await
        .expect("full reconciliation should retry exhausted cleanup");
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-270".to_string(), true), ("COE-270".to_string(), true),]
    );
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
        .tick(ts(300_200))
        .await
        .expect("terminal reconciliation should succeed");

    let issue_id = IssueId::new("lin-270").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("released execution should still exist");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerTerminal);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::TrackerTerminal
    );
    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(
        scheduler.worker().interrupts[0].reason,
        HarnessInterruptReason::SchedulerAbort
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
async fn failed_terminal_interrupt_is_retried_before_cleanup() {
    let issue = tracker_issue("lin-271", "COE-271", "In Progress", 0);
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("remote stop was temporarily unavailable".to_string()),
            timed_out: false,
        }));
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("remote stop acknowledged".to_string()),
            timed_out: false,
        }));
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    scheduler.tracker_mut().active.clear();
    scheduler.tracker_mut().terminal = vec![tracker_issue("lin-271", "COE-271", "Done", 0)];

    scheduler
        .tick(ts(300_200))
        .await
        .expect("first terminal reconciliation should retain the run");
    let issue_id = IssueId::new("lin-271").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("run should remain tracked")
            .status(),
        SchedulerStatus::Running
    );
    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert!(scheduler.worker().aborted.is_empty());
    assert!(scheduler.workspace().cleaned.is_empty());
    assert!(scheduler.workspace().revoked_issue_resources.is_empty());

    scheduler
        .tick(ts(600_400))
        .await
        .expect("second terminal reconciliation should retry the stop");
    assert_eq!(scheduler.worker().interrupts.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-271".to_string(), true)]
    );
    assert_eq!(
        scheduler.workspace().revoked_issue_resources,
        vec!["COE-271".to_string()]
    );
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("released run should remain tracked")
            .status(),
        SchedulerStatus::Released
    );
}

#[tokio::test]
async fn interrupt_persistence_failure_keeps_terminal_run_tracked() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-272", "COE-272", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        persist_interrupt_results: VecDeque::from([Err(FakeError {
            message: "run manifest is unwritable".to_string(),
            category: None,
            retry_after: None,
        })]),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    scheduler.tracker_mut().active.clear();
    scheduler.tracker_mut().terminal = vec![tracker_issue("lin-272", "COE-272", "Done", 0)];

    assert!(scheduler.tick(ts(300_200)).await.is_err());
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-272").expect("issue id should be valid"))
            .expect("execution should be reinserted after persistence failure")
            .status(),
        SchedulerStatus::Running
    );
}

#[tokio::test]
async fn failed_nonterminal_interrupt_keeps_execution_owned_until_acknowledged() {
    let issue = tracker_issue("lin-272", "COE-272", "In Progress", 0);
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("remote stop was temporarily unavailable".to_string()),
            timed_out: false,
        }));
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("remote stop acknowledged".to_string()),
            timed_out: false,
        }));
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    scheduler.tracker_mut().active.clear();
    scheduler.tracker_mut().states.insert(
        "lin-272".to_string(),
        tracker_state_snapshot("lin-272", "COE-272", "Todo", "unstarted", 200),
    );

    scheduler
        .tick(ts(30_200))
        .await
        .expect("first inactive reconciliation should retain the run");
    let issue_id = IssueId::new("lin-272").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("run should remain tracked")
            .status(),
        SchedulerStatus::Running
    );
    assert!(scheduler.worker().aborted.is_empty());
    assert!(scheduler.workspace().revoked_issue_resources.is_empty());

    scheduler
        .tick(ts(60_400))
        .await
        .expect("second inactive reconciliation should retry the stop");
    assert_eq!(scheduler.worker().interrupts.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.workspace().revoked_issue_resources,
        vec!["COE-272".to_string()]
    );
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("released run should remain tracked")
            .status(),
        SchedulerStatus::Released
    );
}

#[tokio::test]
async fn failed_stall_interrupt_remains_running_until_stop_is_acknowledged() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-275", "COE-275", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("remote stop was temporarily unavailable".to_string()),
            timed_out: false,
        }));
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("remote stop acknowledged".to_string()),
            timed_out: false,
        }));
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    scheduler
        .tick(ts(250))
        .await
        .expect("first stalled stop attempt should be retained");

    let issue_id = IssueId::new("lin-275").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("failed stalled execution should remain tracked")
            .status(),
        SchedulerStatus::Running
    );
    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert!(scheduler.worker().aborted.is_empty());

    scheduler
        .tick(ts(350))
        .await
        .expect("second stalled stop attempt should succeed");
    assert_eq!(scheduler.worker().interrupts.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("stalled retry should remain tracked")
            .status(),
        SchedulerStatus::RetryQueued
    );
}

#[tokio::test]
async fn timed_out_stall_interrupt_is_recorded_as_timed_out() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-276", "COE-276", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("remote stop acknowledgement timed out".to_string()),
            timed_out: true,
        }));
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some("remote stop acknowledged".to_string()),
            timed_out: false,
        }));
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("initial dispatch should succeed");
    scheduler
        .tick(ts(250))
        .await
        .expect("timed-out stalled stop attempt should be retained");

    let issue_id = IssueId::new("lin-276").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("timed-out execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(
        execution
            .interrupt()
            .expect("interrupt should be recorded")
            .status,
        HarnessInterruptStatus::TimedOut
    );
    assert!(scheduler.worker().aborted.is_empty());

    scheduler
        .tick(ts(350))
        .await
        .expect("acknowledged retry stop attempt should release the worker");
    assert_eq!(scheduler.worker().aborted.len(), 1);
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
            payload: None,
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
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-272".to_string(), recovered_workspace.clone())]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

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
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .and_then(|execution| execution.current_run())
            .map(|run| run.normal_retry_count),
        Some(1)
    );
    assert!(scheduler.workspace().cleaned.is_empty());
}

#[tokio::test]
async fn recovery_keeps_an_active_cancelled_run_released() {
    let recovered_workspace =
        workspace_record("COE-272-CANCELLED", "/tmp/recovered/COE-272-CANCELLED");
    let tracker = FakeTracker {
        active: vec![tracker_issue(
            "lin-272-cancelled",
            "COE-272-CANCELLED",
            "In Progress",
            0,
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-272-cancelled", "COE-272-CANCELLED", "In Progress"),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: true,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("cancelled recovery should succeed");

    let issue_id = IssueId::new("lin-272-cancelled").expect("issue id should be valid");
    assert!(scheduler.worker().launches.is_empty());
    assert!(matches!(
        scheduler
            .execution(&issue_id)
            .expect("execution should remain visible")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::Cancelled,
            ..
        }
    ));
}

#[tokio::test]
async fn recovery_requeues_cancelled_merging_interrupt() {
    let recovered_workspace = workspace_record("COE-272-MERGING", "/tmp/recovered/COE-272-MERGING");
    let tracker = FakeTracker {
        active: vec![tracker_issue(
            "lin-272-merging",
            "COE-272-MERGING",
            "Merging",
            0,
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-272-merging", "COE-272-MERGING", "Merging"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: true,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: Some(HarnessInterruptReason::TrackerMergingSupersedesHumanReview),
            recovered_run: None,
        }],
        records: HashMap::from([("lin-272-merging".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.active_states.push("Merging".to_string());
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("merging cancellation recovery should succeed");

    let issue_id = IssueId::new("lin-272-merging").expect("issue id should be valid");
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .and_then(|execution| execution.current_run())
            .map(|run| run.normal_retry_count),
        Some(1)
    );
}

#[tokio::test]
async fn recovery_parks_cancelled_merging_interrupt_with_consumed_count() {
    let recovered_workspace = workspace_record(
        "COE-272-MERGING-EXHAUSTED",
        "/tmp/recovered/COE-272-MERGING-EXHAUSTED",
    );
    let tracker = FakeTracker {
        active: vec![tracker_issue(
            "lin-272-merging-exhausted",
            "COE-272-MERGING-EXHAUSTED",
            "Merging",
            0,
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue(
                "lin-272-merging-exhausted",
                "COE-272-MERGING-EXHAUSTED",
                "Merging",
            ),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: true,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: Some(HarnessInterruptReason::TrackerMergingSupersedesHumanReview),
            recovered_run: None,
        }],
        records: HashMap::from([("lin-272-merging-exhausted".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.active_states.push("Merging".to_string());
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("exhausted merging cancellation recovery should succeed");

    assert!(scheduler.worker().launches.is_empty());
    assert_eq!(scheduler.workspace().persisted_retry_exhaustions, vec![1]);
}

#[tokio::test]
async fn pre_conversation_recovery_honors_retry_limit() {
    let recovered_workspace = workspace_record("COE-273", "/tmp/recovered/COE-273");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-273", "COE-273", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-273", "COE-273", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-273".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("pre-conversation recovery should succeed");

    let issue_id = IssueId::new("lin-273").expect("issue id should be valid");
    assert_eq!(
        scheduler
            .execution(&issue_id)
            .expect("recovered execution should remain visible")
            .status(),
        SchedulerStatus::Released
    );
    assert_eq!(scheduler.worker().launches.len(), 0);
}

#[tokio::test]
async fn recovery_advances_consumed_retry_budget_before_dispatch() {
    let recovered_workspace = workspace_record("COE-274", "/tmp/recovered/COE-274");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-274", "COE-274", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-274", "COE-274", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-274".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery should restore the retry budget");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0]
            .run
            .attempt
            .map(|attempt| attempt.get()),
        Some(2)
    );
    assert_eq!(scheduler.worker().launches[0].run.normal_retry_count, 2);
}

#[tokio::test]
async fn recovery_queues_an_active_successful_run_for_continuation() {
    let recovered_workspace = workspace_record("COE-278", "/tmp/recovered/COE-278");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-278", "COE-278", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-278", "COE-278", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: true,
            cancelled_run: false,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-278".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("completed initial recovery should succeed");

    let execution = scheduler
        .execution(&IssueId::new("lin-278").expect("issue id should be valid"))
        .expect("recovered execution should remain visible");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(
        execution
            .retry()
            .expect("continuation retry should exist")
            .reason,
        RetryReason::Continuation
    );
    assert_eq!(
        execution
            .retry()
            .expect("continuation retry should exist")
            .due_at,
        ts(1_100)
    );
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn recovery_releases_an_active_successful_run_when_retry_limit_reached() {
    let recovered_workspace = workspace_record("COE-279", "/tmp/recovered/COE-279");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-279", "COE-279", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-279", "COE-279", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: true,
            cancelled_run: false,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-279".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(0);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("active successful recovery should be parked when exhausted");

    assert!(matches!(
        scheduler
            .execution(&IssueId::new("lin-279").expect("issue id should be valid"))
            .expect("recovered execution should remain visible")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn recovery_merges_successful_run_with_pending_retry_exhaustion() {
    let recovered_workspace = workspace_record("COE-281", "/tmp/recovered/COE-281");
    let issue = normalized_issue("lin-281", "COE-281", "In Progress");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-281", "COE-281", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: issue.clone(),
            workspace: recovered_workspace,
            successful_run: true,
            cancelled_run: false,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        retry_exhaustion: vec![RetryExhaustionRecord {
            issue,
            normal_retry_count: 1,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(2);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("successful recovery should merge with pending exhaustion");

    let execution = scheduler
        .execution(&IssueId::new("lin-281").expect("issue id should be valid"))
        .expect("merged execution should remain visible");
    assert_eq!(execution.status(), SchedulerStatus::RetryQueued);
    assert_eq!(
        execution
            .retry()
            .expect("continuation retry")
            .normal_retry_count,
        2
    );
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn recovery_keeps_successful_run_with_active_tracker_and_exhausted_marker_parked() {
    let recovered_workspace = workspace_record("COE-282", "/tmp/recovered/COE-282");
    let issue = normalized_issue("lin-282", "COE-282", "In Progress");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-282", "COE-282", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: issue.clone(),
            workspace: recovered_workspace,
            successful_run: true,
            cancelled_run: false,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        retry_exhaustion: vec![RetryExhaustionRecord {
            issue,
            normal_retry_count: 1,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("successful exhausted recovery should remain parked");

    let execution = scheduler
        .execution(&IssueId::new("lin-282").expect("issue id should be valid"))
        .expect("completed execution should remain visible");
    assert!(matches!(
        execution.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
    assert!(scheduler.workspace().cleared_retry_exhaustion.is_empty());
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn recovery_dispatches_persisted_pending_retry_before_limit() {
    let recovered_workspace = workspace_record("COE-276", "/tmp/recovered/COE-276");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-276", "COE-276", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-276", "COE-276", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: true,
            normal_retry_count: 0,
            retry_scheduled_at: Some(ts(250)),
            retry_due_at: Some(ts(1_200)),
            retry_reason: Some(RetryReason::Continuation),
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-276".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("pending retry recovery should restore schedule");

    assert!(scheduler.worker().launches.is_empty());

    scheduler
        .tick(ts(1_300))
        .await
        .expect("pending retry should dispatch after its restored deadline");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().launches[0].run.normal_retry_count, 1);
}

#[tokio::test]
async fn recovery_parks_pending_retry_when_current_limit_is_lowered() {
    let recovered_workspace = workspace_record("COE-277", "/tmp/recovered/COE-277");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-277", "COE-277", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-277", "COE-277", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: true,
            normal_retry_count: 1,
            retry_scheduled_at: Some(ts(250)),
            retry_due_at: Some(ts(1_200)),
            retry_reason: Some(RetryReason::Failure),
            retry_error: Some("redacted failure".to_string()),
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-277".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("lowered retry limit should be reconciled");

    let execution = scheduler
        .execution(&IssueId::new("lin-277").expect("issue id should be valid"))
        .expect("execution should remain recorded");
    assert!(matches!(
        execution.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
    assert_eq!(scheduler.workspace().persisted_retry_exhaustions, vec![1]);
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn terminal_retry_marker_is_cleared_when_tracker_state_is_terminal() {
    let tracker = FakeTracker {
        terminal: vec![tracker_issue("lin-275", "COE-275", "Done", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        retry_exhaustion: vec![RetryExhaustionRecord {
            issue: normalized_issue("lin-275", "COE-275", "Done"),
            normal_retry_count: 2,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("terminal retry marker recovery should succeed");

    assert_eq!(
        scheduler.workspace().cleared_retry_exhaustion,
        vec!["COE-275".to_string()]
    );
}

#[tokio::test]
async fn retry_exhaustion_stays_parked_while_tracker_issue_is_inactive() {
    let tracker = FakeTracker {
        states: HashMap::from([(
            "lin-277-exhausted".to_string(),
            tracker_state_snapshot(
                "lin-277-exhausted",
                "COE-277-EXHAUSTED",
                "Backlog",
                "backlog",
                0,
            ),
        )]),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        retry_exhaustion: vec![RetryExhaustionRecord {
            issue: normalized_issue("lin-277-exhausted", "COE-277-EXHAUSTED", "Backlog"),
            normal_retry_count: 1,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("inactive exhaustion recovery should succeed");
    let issue_id = IssueId::new("lin-277-exhausted").expect("issue id should be valid");
    let parked = scheduler
        .execution(&issue_id)
        .expect("exhausted issue should remain tracked");
    assert!(matches!(
        parked.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));

    scheduler.tracker_mut().active = vec![tracker_issue(
        "lin-277-exhausted",
        "COE-277-EXHAUSTED",
        "In Progress",
        0,
    )];
    scheduler
        .tick(ts(60_200))
        .await
        .expect("reactivation should not bypass exhaustion");
    assert!(scheduler.worker().launches.is_empty());
    assert!(matches!(
        scheduler
            .execution(&issue_id)
            .expect("exhausted issue should remain tracked")
            .state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
}

#[tokio::test]
async fn inactive_retry_exhaustion_retries_failed_workspace_cleanup() {
    let recovered_workspace =
        workspace_record("COE-278-EXHAUSTED", "/tmp/recovered/COE-278-EXHAUSTED");
    let tracker = FakeTracker {
        states: HashMap::from([(
            "lin-278-exhausted".to_string(),
            tracker_state_snapshot(
                "lin-278-exhausted",
                "COE-278-EXHAUSTED",
                "Backlog",
                "backlog",
                0,
            ),
        )]),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-278-exhausted", "COE-278-EXHAUSTED", "Backlog"),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        cleanup_results: VecDeque::from([
            Err(FakeError {
                message: "cleanup temporarily unavailable".to_string(),
                category: None,
                retry_after: None,
            }),
            Ok(()),
        ]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial inactive exhaustion recovery should succeed");
    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("inactive exhaustion cleanup retry should succeed");

    assert_eq!(
        scheduler.workspace().failed_cleaned,
        vec![
            "COE-278-EXHAUSTED".to_string(),
            "COE-278-EXHAUSTED".to_string()
        ]
    );
    let issue_id = IssueId::new("lin-278-exhausted").expect("issue id should be valid");
    let parked = scheduler
        .execution(&issue_id)
        .expect("exhausted issue should remain tracked");
    assert!(matches!(
        parked.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
}

#[tokio::test]
async fn recovery_restores_exhausted_retry_count_without_dispatching() {
    let recovered_workspace = workspace_record("COE-273", "/tmp/recovered/COE-273");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-273", "COE-273", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-273", "COE-273", "In Progress"),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery should preserve retry exhaustion");

    let execution = scheduler
        .execution(&IssueId::new("lin-273").expect("issue id should be valid"))
        .expect("recovered execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::Released);
    assert_eq!(execution.retry_count_override(), Some(1));
    assert!(matches!(
        execution.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn recovery_reopens_exhausted_execution_after_retry_limit_increase() {
    let recovered_workspace = workspace_record("COE-279", "/tmp/workspaces/COE-279");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-279", "COE-279", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        retain_failed: true,
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-279", "COE-279", "In Progress"),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: false,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        retry_exhaustion: vec![RetryExhaustionRecord {
            issue: normalized_issue("lin-279", "COE-279", "In Progress"),
            normal_retry_count: 1,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(2);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("raising the retry limit should reopen the recovered execution");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().launches[0].run.normal_retry_count, 2);

    let run = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &run,
                WorkerOutcomeKind::Failed,
                ts(200),
                Some("the reopened final retry failed".to_owned()),
                None,
            ),
        });
    scheduler
        .tick(ts(200))
        .await
        .expect("the reopened retry should exhaust the raised limit");

    let execution = scheduler
        .execution(&IssueId::new("lin-279").expect("issue id should be valid"))
        .expect("exhausted execution should remain tracked");
    assert_eq!(execution.retry_count_override(), Some(2));
    assert!(matches!(
        execution.state(),
        crate::opensymphony_orchestrator::SchedulerState::Released {
            reason: ReleaseReason::RetryExhausted,
            ..
        }
    ));
}

#[tokio::test]
async fn recovery_reopens_marker_only_exhaustion_with_consumed_retry_count() {
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-280", "COE-280", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        retain_failed: false,
        retry_exhaustion: vec![RetryExhaustionRecord {
            issue: normalized_issue("lin-280", "COE-280", "In Progress"),
            normal_retry_count: 1,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(2);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("marker-only exhaustion should reopen as a retry");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().launches[0].run.normal_retry_count, 2);
}

#[tokio::test]
async fn terminal_recovery_honors_failed_workspace_retention() {
    let recovered_workspace = workspace_record("COE-274", "/tmp/recovered/COE-274");
    let tracker = FakeTracker {
        terminal: vec![tracker_issue("lin-274", "COE-274", "Done", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        retain_failed: true,
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-274", "COE-274", "In Progress"),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("terminal recovery should succeed");

    assert!(scheduler.workspace().cleaned.is_empty());
}

#[tokio::test]
async fn terminal_recovery_preserves_cancelled_workspace_policy() {
    let recovered_workspace = workspace_record("COE-275", "/tmp/recovered/COE-275");
    let tracker = FakeTracker {
        terminal: vec![tracker_issue("lin-275", "COE-275", "Done", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-275", "COE-275", "In Progress"),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: true,
            completed_run: true,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 1,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.max_retry_attempts = Some(1);
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("cancelled terminal recovery should succeed");

    assert_eq!(
        scheduler.workspace().cleaned,
        vec![("COE-275".to_string(), true)]
    );
    assert!(scheduler.workspace().failed_cleaned.is_empty());
}

#[tokio::test]
async fn parked_recovered_issue_redispatches_when_tracker_reactivates() {
    // A leftover workspace for a non-active (Backlog) issue is recovered and
    // parked at startup. When the issue later moves back into an active
    // state, the 60s dispatch discovery must reopen the released execution
    // and dispatch it instead of waiting for the hourly full detail refresh.
    let recovered_workspace = workspace_record("COE-532", "/tmp/recovered/COE-532");
    let tracker = FakeTracker {
        states: HashMap::from([(
            "lin-532".to_string(),
            tracker_state_snapshot("lin-532", "COE-532", "Backlog", "backlog", 0),
        )]),
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-532", "COE-532", "Backlog"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        records: HashMap::from([("lin-532".to_string(), recovered_workspace.clone())]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config());

    scheduler
        .tick(ts(100))
        .await
        .expect("recovery tick should succeed");

    let issue_id = IssueId::new("lin-532").expect("issue id should be valid");
    let parked = scheduler
        .execution(&issue_id)
        .expect("recovered issue should be parked");
    assert_eq!(parked.status(), SchedulerStatus::Released);
    match parked.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerInactive);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert!(scheduler.worker().launches.is_empty());

    scheduler.tracker_mut().active = vec![tracker_issue("lin-532", "COE-532", "In Progress", 0)];

    scheduler
        .tick(ts(60_200))
        .await
        .expect("dispatch discovery should reopen and dispatch the issue");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0].issue.identifier.as_str(),
        "COE-532"
    );
    assert_eq!(
        scheduler.worker().launches[0].workspace.path,
        recovered_workspace.path
    );
    let running = scheduler
        .execution(&issue_id)
        .expect("dispatched issue should have an execution");
    assert_eq!(running.status(), SchedulerStatus::Running);
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
        .tick(ts(30_200))
        .await
        .expect("inactive reconciliation should release the running issue");

    let released = scheduler
        .execution(&IssueId::new("lin-277").expect("issue id should be valid"))
        .expect("released issue should still exist");
    assert_eq!(released.status(), SchedulerStatus::Released);
    match released.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerInactive);
        }
        other => panic!("expected released state, got {other:?}"),
    }
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::TrackerInactive
    );

    scheduler
        .tick(ts(60_200))
        .await
        .expect("dispatch discovery should replace the released issue");
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
    scheduler.tracker_mut().states.insert(
        "lin-280".to_string(),
        tracker_state_snapshot("lin-280", "COE-280", "Code Review", "started", 200),
    );

    scheduler
        .tick(ts(30_200))
        .await
        .expect("running-state refresh should update running counts");

    scheduler
        .tick(ts(60_200))
        .await
        .expect("running-state refresh should run before dispatch discovery");

    scheduler
        .tick(ts(65_200))
        .await
        .expect("dispatch discovery should use the updated running counts");

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
    assert!(
        scheduler
            .execution(&IssueId::new("lin-282").expect("issue id should be valid"))
            .is_none()
    );
}

#[tokio::test]
async fn recovered_in_flight_run_restores_persisted_interrupt_intent() {
    let recovered_worker_id =
        WorkerId::new("worker-recovered-cancel").expect("worker id should be valid");
    let recovered_workspace = workspace_record("COE-493", "/tmp/recovered/COE-493");
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-493", "COE-493", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-493", "COE-493", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: Some(HarnessInterruptReason::OperatorCancel),
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: None,
            }),
        }],
        records: HashMap::from([("lin-493".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("startup recovery should restore the active run");
    let execution = scheduler
        .execution(&IssueId::new("lin-493").expect("issue id should be valid"))
        .expect("recovered execution should exist");
    let interrupt = execution
        .interrupt()
        .expect("persisted interrupt intent should be restored");
    assert_eq!(
        interrupt.command.reason,
        HarnessInterruptReason::OperatorCancel
    );
    assert_eq!(
        interrupt.command.expected_next_state,
        crate::opensymphony_domain::HarnessInterruptExpectedNextState::Paused
    );
    assert_eq!(interrupt.status, HarnessInterruptStatus::Acknowledged);
    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(
        scheduler.worker().interrupts[0].reason,
        HarnessInterruptReason::OperatorCancel
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
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: None,
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

#[tokio::test]
async fn detached_outcome_does_not_schedule_retry() {
    // When a worker reports a Detached outcome (stop/cancel failed or unsupported),
    // the scheduler should NOT schedule a retry to avoid duplicating still-active work.
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-300", "COE-300", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None; // Disable stall timeout to isolate the test
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch");

    let issue_id = IssueId::new("lin-300").expect("issue id should be valid");
    assert_eq!(
        scheduler.worker().launches.len(),
        1,
        "should have one launch"
    );

    let running = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: running.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &running,
                WorkerOutcomeKind::Detached,
                ts(200),
                Some("underlying run could not be stopped".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("detached outcome tick should succeed");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");

    // Should be Released, not RetryQueued or Running
    assert_eq!(
        execution.status(),
        SchedulerStatus::Released,
        "detached outcome should release the execution"
    );
    match execution.state() {
        crate::opensymphony_orchestrator::SchedulerState::Released { reason, .. } => {
            assert_eq!(*reason, ReleaseReason::TrackerInactive);
        }
        other => panic!("expected released state, got {other:?}"),
    }

    // No retry should be scheduled
    assert!(execution.retry().is_none());
    // No new launches should have occurred
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().revoked_issue_resources,
        vec!["COE-300".to_string()]
    );
}

#[tokio::test]
async fn cancel_failed_outcome_does_not_schedule_retry() {
    // When a worker reports a CancelFailed outcome (cancel/stop was attempted but refused),
    // the scheduler should NOT schedule a retry to avoid duplicating still-active work.
    let tracker = FakeTracker {
        active: vec![tracker_issue("lin-301", "COE-301", "In Progress", 0)],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.stall_timeout_ms = None; // Disable stall timeout to isolate the test
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("first tick should dispatch");

    let issue_id = IssueId::new("lin-301").expect("issue id should be valid");
    let running = scheduler.worker().launches[0].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: running.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &running,
                WorkerOutcomeKind::CancelFailed,
                ts(200),
                Some("cancel/stop was refused by runtime".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("cancel-failed outcome tick should succeed");

    let execution = scheduler
        .execution(&issue_id)
        .expect("execution should still exist");

    // Should be Released, not RetryQueued
    assert_eq!(execution.status(), SchedulerStatus::Released);
    // No retry should be scheduled
    assert!(execution.retry().is_none());
    // No new launches should have occurred
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().revoked_issue_resources,
        vec!["COE-301".to_string()]
    );
}

#[tokio::test]
async fn repository_binding_outcome_blocks_before_workspace_creation() {
    let mut issue = tracker_issue("lin-repo-missing", "COE-548-MISSING", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("missing binding should be a durable blocked candidate");

    let issue_id = IssueId::new("lin-repo-missing").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("blocked issue should remain observable");
    assert!(matches!(
        execution.issue().repository_binding,
        Some(RepositoryBindingOutcome::MissingBinding)
    ));
    assert!(scheduler.workspace().ensured.is_empty());
    assert!(scheduler.worker().launches.is_empty());
}

#[tokio::test]
async fn unlabeled_parent_remains_repository_neutral() {
    let mut issue = tracker_issue("lin-repo-parent", "COE-548-PARENT", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    issue.sub_issues = vec![TrackerIssueRef {
        id: "lin-repo-parent-child".to_string(),
        identifier: "COE-548-CHILD".to_string(),
        title: Some("Child".to_string()),
        url: Some("https://linear.app/trilogy-ai-coe/issue/COE-548-CHILD".to_string()),
        state: "Done".to_string(),
        state_kind: TrackerIssueStateKind::Completed,
    }];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("unlabeled parent should be observable without a binding failure");

    let execution = scheduler
        .execution(&IssueId::new("lin-repo-parent").expect("issue id should be valid"))
        .expect("parent execution should remain observable");
    assert_eq!(execution.issue().repository_binding, None);
}

#[tokio::test]
async fn newly_parented_running_issue_is_superseded_without_relaunch() {
    let mut issue = tracker_issue(
        "lin-repo-parent-refresh",
        "COE-548-PARENT-REFRESH",
        "In Progress",
        0,
    );
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial repository-bound run should launch");

    let mut refreshed = tracker_state_snapshot(
        "lin-repo-parent-refresh",
        "COE-548-PARENT-REFRESH",
        "In Progress",
        "started",
        30_000,
    );
    refreshed.is_parent = true;
    scheduler
        .tracker_mut()
        .states
        .insert("lin-repo-parent-refresh".to_string(), refreshed);

    scheduler
        .tick(ts(30_100))
        .await
        .expect("running parenthood change should reconcile");

    let execution = scheduler
        .execution(&IssueId::new("lin-repo-parent-refresh").expect("issue id should be valid"))
        .expect("replacement execution should remain observable");
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().removed,
        vec!["COE-548-PARENT-REFRESH"]
    );
    assert_eq!(execution.status(), SchedulerStatus::Unclaimed);
    assert!(execution.workspace().is_none());
    assert_eq!(execution.issue().repository_binding, None);
}

#[tokio::test]
async fn newly_parented_legacy_issue_is_neutralized_before_default_routing() {
    let mut issue = tracker_issue(
        "lin-repo-legacy-parent-refresh",
        "COE-548-LEGACY-PARENT-REFRESH",
        "In Progress",
        0,
    );
    issue.project_id = Some("project-id".to_string());
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let routing = repository_routing();
    let mut config = scheduler_config();
    config.repository_routing = Some(RepositoryRouting {
        mode: RepositoryRoutingMode::LegacySingle,
        legacy_repository: Some("one".to_string()),
        ..routing
    });
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("legacy issue should initially use its configured default");
    assert_eq!(scheduler.worker().launches.len(), 1);

    let mut refreshed = tracker_state_snapshot(
        "lin-repo-legacy-parent-refresh",
        "COE-548-LEGACY-PARENT-REFRESH",
        "In Progress",
        "started",
        30_000,
    );
    refreshed.is_parent = true;
    scheduler
        .tracker_mut()
        .states
        .insert("lin-repo-legacy-parent-refresh".to_string(), refreshed);

    scheduler
        .tick(ts(30_100))
        .await
        .expect("legacy parenthood change should reconcile");

    let execution = scheduler
        .execution(
            &IssueId::new("lin-repo-legacy-parent-refresh").expect("issue id should be valid"),
        )
        .expect("neutralized parent should remain observable");
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.workspace().removed,
        vec!["COE-548-LEGACY-PARENT-REFRESH".to_string()]
    );
    assert_eq!(execution.status(), SchedulerStatus::Unclaimed);
    assert!(execution.workspace().is_none());
    assert_eq!(execution.issue().repository_binding, None);
}

#[tokio::test]
async fn binding_supersession_restores_execution_when_abort_fails() {
    let mut issue = tracker_issue("lin-repo-abort", "COE-548-ABORT", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker {
        abort_results: VecDeque::from([Err(FakeError {
            message: "abort failed".to_string(),
            category: None,
            retry_after: None,
        })]),
        ..Default::default()
    };
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("initial repository-bound run should launch");
    let old_worker_id = scheduler.worker().launches[0].run.worker_id.clone();
    scheduler.tracker_mut().active[0].labels = vec!["repo:two".to_string()];
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::RuntimeEvent {
            worker_id: old_worker_id,
            observed_at: ts(3_600_050),
            event_id: Some("abort-failure-late-event".to_string()),
            event_kind: Some("runtime_event".to_string()),
            summary: Some("metadata must remain until local abort succeeds".to_string()),
            payload: None,
        });

    assert!(scheduler.tick(ts(3_600_100)).await.is_err());
    let issue_id = IssueId::new("lin-repo-abort").expect("issue id should be valid");
    let execution = scheduler
        .execution(&issue_id)
        .expect("old execution must remain tracked after abort failure");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(
        execution
            .conversation()
            .and_then(|conversation| conversation.last_event_id.as_deref()),
        Some("abort-failure-late-event")
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().aborted.len(), 1);
}

#[tokio::test]
async fn recovery_fences_persisted_binding_before_superseding_it() {
    let recovered_worker_id =
        WorkerId::new("worker-recovered-binding").expect("worker id should be valid");
    let recovered_workspace =
        workspace_record("COE-548-RECOVERY", "/tmp/recovered/COE-548-RECOVERY");
    let old_binding = match repository_routing().resolve(
        &["repo:two".to_string()],
        Some("project-id"),
        None,
        false,
    ) {
        RepositoryBindingOutcome::Resolved(binding) => binding,
        outcome => panic!("expected persisted binding, got {outcome:?}"),
    };
    let tracker = FakeTracker {
        active: vec![{
            let mut issue =
                tracker_issue("lin-repo-recovery", "COE-548-RECOVERY", "In Progress", 0);
            issue.project_id = Some("project-id".to_string());
            issue.labels = vec!["repo:one".to_string()];
            issue
        }],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue("lin-repo-recovery", "COE-548-RECOVERY", "In Progress"),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: Some(old_binding),
            }),
        }],
        records: HashMap::from([("lin-repo-recovery".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("binding-drift recovery should reattach then supersede");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::BindingSuperseded
    );
    let execution = scheduler
        .execution(&IssueId::new("lin-repo-recovery").expect("issue id should be valid"))
        .expect("replacement execution should remain tracked");
    assert_eq!(execution.status(), SchedulerStatus::Running);
    assert_eq!(
        execution
            .current_run()
            .expect("replacement run should be active")
            .repository_binding
            .as_ref()
            .map(|binding: &RepositoryBinding| binding.alias.as_str()),
        Some("one")
    );
}

#[tokio::test]
async fn legacy_recovery_backfills_binding_before_drift_comparison() {
    let recovered_worker_id =
        WorkerId::new("worker-legacy-recovery").expect("worker id should be valid");
    let recovered_workspace = workspace_record(
        "COE-548-LEGACY-RECOVERY",
        "/tmp/recovered/COE-548-LEGACY-RECOVERY",
    );
    let historical_binding = RepositoryBinding {
        alias: "one".to_string(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::new("github:one").expect("repository id should be valid"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("one"),
                "https://github.com/example/repository",
            )
            .expect("fingerprint should be valid"),
        },
        config_generation: "config-test".to_string(),
        inventory_generation: "inventory-test".to_string(),
    };
    let mut recovered_issue = normalized_issue(
        "lin-legacy-recovery",
        "COE-548-LEGACY-RECOVERY",
        "In Progress",
    );
    recovered_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(
        historical_binding.clone(),
    ));
    let tracker = FakeTracker {
        active: vec![tracker_issue(
            "lin-legacy-recovery",
            "COE-548-LEGACY-RECOVERY",
            "In Progress",
            0,
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: recovered_issue,
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: Some(historical_binding),
            }),
        }],
        records: HashMap::from([("lin-legacy-recovery".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    let routing = repository_routing();
    config.repository_routing = Some(RepositoryRouting {
        mode: RepositoryRoutingMode::LegacySingle,
        legacy_repository: Some("one".to_string()),
        ..routing
    });
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("legacy recovery should reattach without binding drift");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert!(scheduler.worker().aborted.is_empty());
    let execution = scheduler
        .execution(&IssueId::new("lin-legacy-recovery").expect("issue id should be valid"))
        .expect("recovered execution should remain tracked");
    assert_eq!(
        execution
            .current_run()
            .and_then(|run| run.repository_binding.as_ref())
            .map(|binding| binding.alias.as_str()),
        Some("one")
    );
}

#[tokio::test]
async fn externally_persisted_retry_rehydrates_without_a_workspace() {
    let mut tracker_issue = tracker_issue(
        "lin-retry-external",
        "COE-548-RETRY-EXTERNAL",
        "In Progress",
        0,
    );
    tracker_issue.project_id = Some("project-id".to_string());
    tracker_issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![tracker_issue],
        ..Default::default()
    };
    let routing = repository_routing();
    let binding = match routing.resolve(&["repo:one".to_string()], Some("project-id"), None, false)
    {
        RepositoryBindingOutcome::Resolved(binding) => binding,
        outcome => panic!("expected retry binding, got {outcome:?}"),
    };
    let mut pending_issue = normalized_issue(
        "lin-retry-external",
        "COE-548-RETRY-EXTERNAL",
        "In Progress",
    );
    pending_issue.project_id = Some("project-id".to_string());
    pending_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(binding));
    let workspace = FakeWorkspace {
        retry_pending_recoveries: vec![RetryPendingRecord {
            issue: pending_issue.clone(),
            retry: RetryEntry {
                issue_id: pending_issue.id.clone(),
                identifier: pending_issue.identifier.clone(),
                attempt: RetryAttempt::new(1).expect("retry attempt should be valid"),
                normal_retry_count: 1,
                scheduled_at: ts(50),
                due_at: ts(100),
                reason: RetryReason::Failure,
                error: Some("persisted outside the old workspace".to_string()),
            },
        }],
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(routing);
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("external retry state should rehydrate and dispatch");

    assert_eq!(
        scheduler.workspace().ensured,
        vec!["COE-548-RETRY-EXTERNAL"]
    );
    assert_eq!(
        scheduler.workspace().cleared_retry_pending,
        vec!["lin-retry-external".to_string()]
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().launches[0].run.normal_retry_count, 1);
}

#[tokio::test]
async fn external_retry_marker_merges_into_metadata_only_workspace_recovery() {
    let issue_id = IssueId::new("lin-retry-workspace").expect("issue id should be valid");
    let recovered_workspace = workspace_record(
        "COE-548-RETRY-WORKSPACE",
        "/tmp/recovered/COE-548-RETRY-WORKSPACE",
    );
    let pending_issue =
        normalized_issue(issue_id.as_str(), "COE-548-RETRY-WORKSPACE", "In Progress");
    let tracker = FakeTracker {
        active: vec![tracker_issue(
            issue_id.as_str(),
            "COE-548-RETRY-WORKSPACE",
            "In Progress",
            0,
        )],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: pending_issue.clone(),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        retry_pending_recoveries: vec![RetryPendingRecord {
            issue: pending_issue.clone(),
            retry: RetryEntry {
                issue_id: issue_id.clone(),
                identifier: pending_issue.identifier.clone(),
                attempt: RetryAttempt::new(2).expect("retry attempt should be valid"),
                normal_retry_count: 2,
                scheduled_at: ts(50),
                due_at: ts(100),
                reason: RetryReason::Failure,
                error: Some("replacement workspace was prepared".to_string()),
            },
        }],
        records: HashMap::from([(issue_id.to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("metadata-only retry recovery should dispatch");

    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(scheduler.worker().launches[0].run.normal_retry_count, 2);
    assert_eq!(scheduler.workspace().persisted_retry_counts, vec![2]);
    assert_eq!(
        scheduler.workspace().cleared_retry_pending,
        vec![issue_id.to_string()]
    );
}

#[tokio::test]
async fn metadata_only_retry_recovery_rematerializes_after_binding_drift() {
    let issue_id = IssueId::new("lin-retry-drift").expect("issue id should be valid");
    let recovered_workspace =
        workspace_record("COE-548-RETRY-DRIFT", "/tmp/recovered/COE-548-RETRY-DRIFT");
    let routing = repository_routing();
    let old_binding =
        match routing.resolve(&["repo:one".to_string()], Some("project-id"), None, false) {
            RepositoryBindingOutcome::Resolved(binding) => binding,
            outcome => panic!("expected old retry binding, got {outcome:?}"),
        };
    let pending_issue = {
        let mut issue = normalized_issue(issue_id.as_str(), "COE-548-RETRY-DRIFT", "In Progress");
        issue.project_id = Some("project-id".to_string());
        issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(old_binding));
        issue
    };
    let mut tracker_issue =
        tracker_issue(issue_id.as_str(), "COE-548-RETRY-DRIFT", "In Progress", 0);
    tracker_issue.project_id = Some("project-id".to_string());
    tracker_issue.labels = vec!["repo:two".to_string()];
    let tracker = FakeTracker {
        active: vec![tracker_issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: pending_issue.clone(),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: false,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: None,
            interrupt_reason: None,
            recovered_run: None,
        }],
        retry_pending_recoveries: vec![RetryPendingRecord {
            issue: pending_issue.clone(),
            retry: RetryEntry {
                issue_id: issue_id.clone(),
                identifier: pending_issue.identifier.clone(),
                attempt: RetryAttempt::new(1).expect("retry attempt should be valid"),
                normal_retry_count: 1,
                scheduled_at: ts(50),
                due_at: ts(100),
                reason: RetryReason::Failure,
                error: Some("binding changed before start_run".to_string()),
            },
        }],
        records: HashMap::from([(issue_id.to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let mut config = scheduler_config();
    config.repository_routing = Some(routing);
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, FakeWorker::default(), config);

    scheduler
        .tick(ts(100))
        .await
        .expect("binding-drift retry recovery should rematerialize safely");

    assert_eq!(
        scheduler.workspace().removed,
        vec!["COE-548-RETRY-DRIFT".to_string()]
    );
    assert_eq!(
        scheduler.workspace().ensured,
        vec!["COE-548-RETRY-DRIFT".to_string()]
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_eq!(
        scheduler.worker().launches[0]
            .run
            .repository_binding
            .as_ref()
            .map(|binding| binding.alias.as_str()),
        Some("two")
    );
}

#[tokio::test]
async fn external_retry_marker_is_cleared_without_live_tracker_proof() {
    let issue = normalized_issue("lin-retry-stale", "COE-548-RETRY-STALE", "In Progress");
    let workspace = FakeWorkspace {
        retry_pending_recoveries: vec![RetryPendingRecord {
            issue: issue.clone(),
            retry: RetryEntry {
                issue_id: issue.id.clone(),
                identifier: issue.identifier.clone(),
                attempt: RetryAttempt::new(1).expect("retry attempt should be valid"),
                normal_retry_count: 1,
                scheduled_at: ts(50),
                due_at: ts(100),
                reason: RetryReason::Failure,
                error: None,
            },
        }],
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        FakeTracker::default(),
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );

    scheduler
        .tick(ts(100))
        .await
        .expect("stale external retry should be parked");

    assert!(scheduler.execution(&issue.id).is_none());
    assert!(scheduler.worker().launches.is_empty());
    assert_eq!(
        scheduler.workspace().cleared_retry_pending,
        vec![issue.id.to_string()]
    );
}

#[tokio::test]
async fn dispatch_adopts_all_workers_before_returning_clear_error() {
    let tracker = FakeTracker {
        active: vec![
            tracker_issue("lin-first", "COE-548-FIRST", "In Progress", 0),
            tracker_issue("lin-second", "COE-548-SECOND", "In Progress", 0),
        ],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        clear_retry_pending_results: VecDeque::from([
            Err(FakeError {
                message: "first retry marker clear failed".to_string(),
                category: None,
                retry_after: None,
            }),
            Ok(()),
        ]),
        ..Default::default()
    };
    let mut scheduler = Scheduler::new(
        tracker,
        workspace,
        FakeWorker::default(),
        scheduler_config(),
    );

    assert!(scheduler.tick(ts(100)).await.is_err());
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-first").expect("issue id should be valid"))
            .expect("first execution should be retained")
            .status(),
        SchedulerStatus::Running
    );
    let second_run = scheduler.worker().launches[1].run.clone();
    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::Finished {
            worker_id: second_run.worker_id.clone(),
            outcome: WorkerOutcomeRecord::from_run(
                &second_run,
                WorkerOutcomeKind::Succeeded,
                ts(200),
                Some("second worker completed".to_string()),
                None,
            ),
        });

    scheduler
        .tick(ts(200))
        .await
        .expect("adopted second worker should process its event");
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-second").expect("issue id should be valid"))
            .expect("second execution should be retained")
            .status(),
        SchedulerStatus::RetryQueued
    );
}

#[tokio::test]
async fn legacy_recovery_rematerializes_without_persisted_binding_proof() {
    let recovered_worker_id =
        WorkerId::new("worker-unproven-legacy-recovery").expect("worker id should be valid");
    let recovered_workspace = workspace_record(
        "COE-548-LEGACY-UNPROVEN",
        "/tmp/recovered/COE-548-LEGACY-UNPROVEN",
    );
    let tracker = FakeTracker {
        active: vec![{
            let mut issue = tracker_issue(
                "lin-legacy-unproven",
                "COE-548-LEGACY-UNPROVEN",
                "In Progress",
                0,
            );
            issue.project_id = Some("project-id".to_string());
            issue
        }],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue(
                "lin-legacy-unproven",
                "COE-548-LEGACY-UNPROVEN",
                "In Progress",
            ),
            workspace: recovered_workspace.clone(),
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: None,
            }),
        }],
        records: HashMap::from([("lin-legacy-unproven".to_string(), recovered_workspace)]),
        ..Default::default()
    };
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    let routing = repository_routing();
    config.repository_routing = Some(RepositoryRouting {
        mode: RepositoryRoutingMode::LegacySingle,
        legacy_repository: Some("one".to_string()),
        ..routing
    });
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("unproven legacy recovery should rematerialize safely");

    assert_eq!(
        scheduler.workspace().removed,
        vec!["COE-548-LEGACY-UNPROVEN"]
    );
    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::BindingSuperseded
    );
    assert_eq!(scheduler.worker().launches.len(), 1);
    assert_ne!(
        scheduler.worker().launches[0].run.worker_id,
        recovered_worker_id
    );
}

#[tokio::test]
async fn unproven_recovery_keeps_workspace_and_worker_until_stop_acknowledges() {
    let recovered_worker_id =
        WorkerId::new("worker-unproven-stop-pending").expect("worker id should be valid");
    let recovered_workspace = workspace_record(
        "COE-548-LEGACY-STOP-PENDING",
        "/tmp/recovered/COE-548-LEGACY-STOP-PENDING",
    );
    let tracker = FakeTracker {
        active: vec![{
            let mut issue = tracker_issue(
                "lin-legacy-stop-pending",
                "COE-548-LEGACY-STOP-PENDING",
                "In Progress",
                0,
            );
            issue.project_id = Some("project-id".to_string());
            issue
        }],
        ..Default::default()
    };
    let workspace = FakeWorkspace {
        recoveries: vec![RecoveryRecord {
            issue: normalized_issue(
                "lin-legacy-stop-pending",
                "COE-548-LEGACY-STOP-PENDING",
                "In Progress",
            ),
            workspace: recovered_workspace,
            successful_run: false,
            cancelled_run: false,
            completed_run: false,
            had_in_flight_run: true,
            pending_retry: false,
            normal_retry_count: 0,
            retry_scheduled_at: None,
            retry_due_at: None,
            retry_reason: None,
            retry_error: None,
            harness_kind: Some("openhands_agent_server".to_string()),
            interrupt_reason: None,
            recovered_run: Some(RecoveredRun {
                worker_id: recovered_worker_id.clone(),
                conversation: conversation(&recovered_worker_id),
                normal_retry_count: 0,
                repository_binding: None,
            }),
        }],
        ..Default::default()
    };
    let mut worker = FakeWorker::default();
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("stop still pending".to_string()),
            timed_out: false,
        }));
    worker
        .interrupt_results
        .push_back(Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("stop still pending".to_string()),
            timed_out: false,
        }));
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("failed stop should retain the old generation");

    assert!(scheduler.workspace().removed.is_empty());
    assert!(scheduler.worker().launches.is_empty());
    assert_eq!(scheduler.worker().interrupts.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 0);
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-legacy-stop-pending").expect("issue id"))
            .expect("old generation should remain tracked")
            .status(),
        SchedulerStatus::Running
    );
}

#[tokio::test]
async fn alias_only_binding_mutation_supersedes_running_generation() {
    let mut issue = tracker_issue("lin-repo-alias", "COE-548-ALIAS", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut routing = repository_routing();
    let identity = routing
        .inventory
        .get("one")
        .expect("repository one should exist")
        .identity
        .clone();
    routing.inventory.insert(
        "alias".to_string(),
        RepositoryInventoryEntry {
            alias: "alias".to_string(),
            identity: identity.clone(),
        },
    );
    let mut config = scheduler_config();
    config.repository_routing = Some(routing);
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler.tick(ts(100)).await.expect("initial dispatch");
    scheduler.tracker_mut().active[0].labels = vec!["repo:alias".to_string()];
    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("alias-only binding refresh should supersede the run");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler
            .execution(&IssueId::new("lin-repo-alias").expect("issue id should be valid"))
            .expect("execution should remain tracked")
            .issue()
            .repository_binding
            .as_ref()
            .and_then(RepositoryBindingOutcome::resolved_binding)
            .map(|binding| binding.alias.as_str()),
        Some("alias")
    );
}

#[tokio::test]
async fn claimed_repository_binding_is_immutable_and_stale_events_are_ignored() {
    let mut issue = tracker_issue("lin-repo-change", "COE-548-CHANGE", "In Progress", 0);
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("valid repository binding should launch");
    let old_run = scheduler.worker().launches[0].run.clone();
    let old_binding = old_run
        .repository_binding
        .clone()
        .expect("claim should persist repository binding");

    scheduler.tracker_mut().active[0].labels = vec!["repo:two".to_string()];
    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("binding mutation should supersede the old generation");

    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::BindingSuperseded
    );
    assert_ne!(
        scheduler.worker().launches[1]
            .run
            .repository_binding
            .as_ref()
            .expect("replacement claim should persist binding")
            .repository
            .id,
        old_binding.repository.id
    );
    assert_eq!(scheduler.workspace().removed, vec!["COE-548-CHANGE"]);

    scheduler
        .worker_mut()
        .updates
        .push_back(WorkerUpdate::RuntimeEvent {
            worker_id: old_run.worker_id,
            observed_at: ts(3_600_200),
            event_id: Some("late-old-generation-event".to_string()),
            event_kind: Some("late_event".to_string()),
            summary: Some("must be ignored".to_string()),
            payload: None,
        });
    scheduler
        .tick(ts(3_600_200))
        .await
        .expect("late old-generation events should be ignored");

    let issue_id = IssueId::new("lin-repo-change").expect("issue id should be valid");
    let replacement = scheduler
        .execution(&issue_id)
        .expect("replacement execution should remain tracked");
    assert_eq!(
        replacement
            .conversation()
            .expect("replacement conversation should exist")
            .last_event_id,
        None
    );
}

#[tokio::test]
async fn binding_supersession_precedes_simultaneous_merging_transition() {
    let mut issue = tracker_issue("lin-repo-merging", "COE-548-MERGING", "Human Review", 0);
    issue.project_id = Some("project-id".to_string());
    issue.labels = vec!["repo:one".to_string()];
    let tracker = FakeTracker {
        active: vec![issue],
        ..Default::default()
    };
    let workspace = FakeWorkspace::default();
    let worker = FakeWorker::default();
    let mut config = scheduler_config();
    config.repository_routing = Some(repository_routing());
    config.active_states.push("Human Review".to_string());
    config.active_states.push("Merging".to_string());
    config.stall_timeout_ms = None;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, config);

    scheduler
        .tick(ts(100))
        .await
        .expect("Human Review worker should launch");
    scheduler.tracker_mut().active[0].state = "Merging".to_string();
    scheduler.tracker_mut().active[0].labels = vec!["repo:two".to_string()];

    scheduler
        .tick(ts(3_600_100))
        .await
        .expect("binding supersession should handle simultaneous Merging");

    assert_eq!(scheduler.worker().aborted.len(), 1);
    assert_eq!(
        scheduler.worker().aborted[0].1,
        WorkerAbortReason::BindingSuperseded
    );
    assert_eq!(scheduler.worker().interrupts.len(), 1);
    assert_eq!(
        scheduler.worker().interrupts[0].reason,
        HarnessInterruptReason::SchedulerAbort
    );
    assert_eq!(scheduler.worker().launches.len(), 2);
    assert_eq!(scheduler.worker().launches[1].issue.state.name, "Merging");
}
