// OSYM-721 / COE-405: integration tests for the gateway-mediated Linear
// mutation pipeline.
//
// The host client must only reach Linear through the gateway. These tests
// confirm that the `/api/v1/taskgraph/milestones`, `/issues`,
// `/sub-issues`, `/relations`, and `/evidence` routes:
//   * forward the request body and correlation_id to a fake
//     `LinearMutationClient`;
//   * return an `ActionReceipt` whose `status` reflects the Linear result;
//   * tag the receipt with the expected task-graph-update follow-up event.

#![allow(clippy::unwrap_used)]

use chrono::Utc;
use opensymphony::opensymphony_control::SnapshotStore;
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome,
};
use opensymphony::opensymphony_gateway::{
    GatewayServer, IssueOp, LinearMutationClient, MilestoneOp, MutationError, SubIssueOp,
    TaskGraphEvidenceRequest, TaskGraphEvidenceResponse, TaskGraphIssueRequest,
    TaskGraphIssueResponse, TaskGraphMilestoneRequest, TaskGraphMilestoneResponse,
    TaskGraphRelationRequest, TaskGraphRelationResponse, TaskGraphSubIssueRequest,
    TaskGraphSubIssueResponse,
};
use opensymphony::opensymphony_gateway_schema::action::{
    ActionKind, ActionReceipt, ActionStatus, ExpectedFollowup,
};
use opensymphony::opensymphony_gateway_schema::envelope::EntityKind;
use opensymphony::opensymphony_gateway_schema::event_journal::EventKind as JournalEventKind;
use reqwest::Client;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep};
use uuid::Uuid;

#[path = "support/mod.rs"]
mod compat;
pub use compat::*;

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
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
            conversation_count: 2,
            status_line: "healthy".to_owned(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 1,
            retry_queue_depth: 0,
            input_tokens: 2048,
            output_tokens: 2048,
            cache_read_tokens: 512,
            total_tokens: 4096 + step,
            total_cost_micros: 120_000,
        },
        issues: vec![IssueSnapshot {
            identifier: "COE-405".to_owned(),
            title: "Linear Milestone, Issue, And Sub-Issue Mutations".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Idle,
            last_outcome: WorkerOutcome::Completed,
            last_event_at: now,
            conversation_id_suffix: "c0e405".to_owned(),
            workspace_path_suffix: "COE-405".to_owned(),
            retry_count: 0,
            claimed_at: None,
            started_at: None,
            finished_at: None,
            turn_count: 0,
            max_turns: 0,
            runtime_seconds: 0,
            blocked: false,
            blocked_by: Vec::new(),
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            transport_target: Some("loopback".to_owned()),
            http_auth_mode: Some("none".to_owned()),
            websocket_auth_mode: Some("none".to_owned()),
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            cancel_acknowledged: false,
            cancel_failed: false,
            detached: false,
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-405".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: format!("published step {step}"),
        }],
    }
}

#[derive(Default)]
struct RecordedCalls {
    milestone: Mutex<Vec<(TaskGraphMilestoneRequest, String)>>,
    issue: Mutex<Vec<(TaskGraphIssueRequest, String)>>,
    sub_issue: Mutex<Vec<(TaskGraphSubIssueRequest, String)>>,
    relation: Mutex<Vec<(TaskGraphRelationRequest, String)>>,
    evidence: Mutex<Vec<(TaskGraphEvidenceRequest, String)>>,
}

struct FakeLinearClient {
    calls: RecordedCalls,
}

impl FakeLinearClient {
    fn new() -> Self {
        Self {
            calls: RecordedCalls::default(),
        }
    }
}

#[async_trait::async_trait]
impl LinearMutationClient for FakeLinearClient {
    async fn create_or_update_project_milestone(
        &self,
        request: TaskGraphMilestoneRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphMilestoneResponse, MutationError> {
        if matches!(request.op, MilestoneOp::Update) && request.milestone_id.is_none() {
            return Err(MutationError::Validation(
                "milestone_id required for update".into(),
            ));
        }
        self.calls
            .milestone
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphMilestoneResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphMilestone,
            ),
            milestone_id: Some(match request.op {
                MilestoneOp::Update => request
                    .milestone_id
                    .clone()
                    .unwrap_or_else(|| "ms_fake".to_owned()),
                MilestoneOp::Create => "ms_fake".into(),
            }),
            milestone_name: Some(request.name),
            project_id: Some(request.project_id),
        })
    }

    async fn create_or_update_issue(
        &self,
        request: TaskGraphIssueRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphIssueResponse, MutationError> {
        self.calls
            .issue
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphIssueResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphIssue,
            ),
            issue_id: Some(match request.op {
                IssueOp::Update => request
                    .issue_id
                    .clone()
                    .unwrap_or_else(|| "iss_fake".to_owned()),
                IssueOp::Create => "iss_fake".into(),
            }),
            issue_identifier: Some(request.title),
            state_id: None,
            project_milestone_id: request.project_milestone_id,
        })
    }

    async fn create_or_update_sub_issue(
        &self,
        request: TaskGraphSubIssueRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphSubIssueResponse, MutationError> {
        if matches!(request.op, SubIssueOp::Update)
            && request.sub_issue_id.as_deref().is_none_or(str::is_empty)
        {
            return Err(MutationError::Validation(
                "sub_issue_id required for update".into(),
            ));
        }
        self.calls
            .sub_issue
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        let sub_issue_id = match request.op {
            SubIssueOp::Update => request
                .sub_issue_id
                .clone()
                .unwrap_or_else(|| "sub_fake".to_owned()),
            SubIssueOp::Create => "sub_fake".into(),
        };
        Ok(TaskGraphSubIssueResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphSubIssue,
            ),
            sub_issue_id: Some(sub_issue_id),
            sub_issue_identifier: Some(request.title),
            parent_identifier: Some(request.parent_identifier),
            state_id: None,
        })
    }

    async fn create_issue_relation(
        &self,
        request: TaskGraphRelationRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphRelationResponse, MutationError> {
        self.calls
            .relation
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphRelationResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphRelation,
            ),
            relation_id: Some("rel_fake".into()),
            relation_type: Some(request.relation_type),
            related_issue_id: Some(request.related_issue_id),
        })
    }

    async fn create_evidence_comment(
        &self,
        request: TaskGraphEvidenceRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphEvidenceResponse, MutationError> {
        self.calls
            .evidence
            .lock()
            .await
            .push((request.clone(), correlation_id.to_string()));
        Ok(TaskGraphEvidenceResponse {
            receipt: ActionReceipt::accepted(
                Uuid::new_v4().to_string(),
                correlation_id,
                ActionKind::TaskGraphEvidence,
            ),
            comment_id: Some("c_fake".into()),
            issue_id: Some(request.issue_id),
            issue_identifier: None,
        })
    }
}

async fn start_test_server(client: Arc<FakeLinearClient>) -> (JoinHandle<()>, SocketAddr) {
    let (handle, addr, _journal) = start_test_server_with_journal(client).await;
    (handle, addr)
}

/// Same as `start_test_server_with_journal`, but skips the
/// `with_linear_mutations(...)` installation so the
/// `/api/v1/taskgraph/*` routes return 503 on every request.
async fn start_test_server_without_linear_mutations() -> (JoinHandle<()>, SocketAddr) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = opensymphony::opensymphony_domain::InMemoryEventJournal::new(1024, 64);
    // Note: do NOT call `.with_linear_mutations(...)`; we rely on the
    // production wiring's `mutation_client_unavailable()` short-circuit.
    let server = GatewayServer::with_journal(
        store,
        journal,
        opensymphony::opensymphony_domain::StreamBroker::new(
            opensymphony::opensymphony_domain::InMemoryEventJournal::new(1024, 64),
        ),
    );
    let handle = tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    sleep(Duration::from_millis(25)).await;
    (handle, addr)
}

async fn start_test_server_with_journal(
    client: Arc<FakeLinearClient>,
) -> (
    JoinHandle<()>,
    SocketAddr,
    opensymphony::opensymphony_domain::InMemoryEventJournal,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = opensymphony::opensymphony_domain::InMemoryEventJournal::new(1024, 64);
    let journal_handle = journal.clone();
    let server = GatewayServer::with_journal(
        store,
        journal.clone(),
        opensymphony::opensymphony_domain::StreamBroker::new(journal.clone()),
    )
    .with_linear_mutations(Some(client as Arc<dyn LinearMutationClient>));
    let handle = tokio::spawn(async move {
        let _ = server.serve(listener).await;
    });
    sleep(Duration::from_millis(25)).await;
    (handle, addr, journal_handle)
}

#[tokio::test]
async fn milestones_create_returns_accepted_receipt_with_correlation_id() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphMilestoneRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-milestone-create".into(),
        op: MilestoneOp::Create,
        idempotency_key: None,
        project_id: "proj_1".into(),
        milestone_id: None,
        name: "M1 demo".into(),
        description: Some("desc".into()),
        target_date: None,
        sort_order: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphMilestoneResponse = resp.json().await.unwrap();
    assert_eq!(body.milestone_id.as_deref(), Some("ms_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert_eq!(body.receipt.correlation_id, "corr-milestone-create");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.milestone.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].1, "corr-milestone-create");
    handle.abort();
}

#[tokio::test]
async fn milestones_update_forwards_existing_id_to_fake_client() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphMilestoneRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-milestone-update".into(),
        op: MilestoneOp::Update,
        idempotency_key: None,
        project_id: "proj_1".into(),
        milestone_id: Some("ms_existing".into()),
        name: "M1 renamed".into(),
        description: None,
        target_date: None,
        sort_order: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphMilestoneResponse = resp.json().await.unwrap();
    assert_eq!(body.milestone_id.as_deref(), Some("ms_existing"));
    assert_eq!(body.receipt.correlation_id, "corr-milestone-update");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.milestone.lock().await;
    let (forwarded, cid) = &calls[0];
    assert_eq!(cid, "corr-milestone-update");
    assert_eq!(forwarded.milestone_id.as_deref(), Some("ms_existing"));
    assert!(matches!(forwarded.op, MilestoneOp::Update));

    // The audit event must be `Updated` for an Update op (not the
    // pre-fix `Created` shape); see COE-405 review feedback round 4.
    let events = journal.recent_events(10).await;
    assert!(
        events.iter().any(|rec| matches!(
            rec.kind,
            JournalEventKind::TaskGraphMilestoneUpdated { ref milestone_id } if milestone_id == "ms_existing"
        )),
        "expected TaskGraphMilestoneUpdated{{milestone_id=\"ms_existing\"}} in {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|rec| matches!(rec.kind, JournalEventKind::TaskGraphMilestoneCreated { .. })),
        "did not expect a TaskGraphMilestoneCreated event for an Update op: {events:?}"
    );
    handle.abort();
}

#[tokio::test]
async fn milestones_create_emits_created_event_in_journal() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphMilestoneRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-milestone-create-audit".into(),
        op: MilestoneOp::Create,
        idempotency_key: None,
        project_id: "proj_1".into(),
        milestone_id: None,
        name: "M1 created".into(),
        description: Some("desc".into()),
        target_date: None,
        sort_order: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let events = journal.recent_events(10).await;
    assert!(
        events.iter().any(|rec| matches!(
            rec.kind,
            JournalEventKind::TaskGraphMilestoneCreated { ref milestone_id } if milestone_id == "ms_fake"
        )),
        "expected TaskGraphMilestoneCreated{{milestone_id=\"ms_fake\"}} in {events:?}"
    );
    assert!(
        events
            .iter()
            .all(|rec| !matches!(rec.kind, JournalEventKind::TaskGraphMilestoneUpdated { .. })),
        "did not expect any TaskGraphMilestoneUpdated events for a Create op: {events:?}"
    );
    handle.abort();
}

#[tokio::test]
async fn milestones_update_without_id_is_rejected() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, _journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphMilestoneRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-milestone-update-missing-id".into(),
        op: MilestoneOp::Update,
        idempotency_key: None,
        project_id: "proj_1".into(),
        milestone_id: None, // missing for an Update → should be rejected
        name: "M1 anonymous".into(),
        description: None,
        target_date: None,
        sort_order: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: TaskGraphMilestoneResponse = resp.json().await.unwrap();
    assert_eq!(body.receipt.status, ActionStatus::Rejected);

    // The fake client never sees Update requests that fail validation.
    let calls = fake.calls.milestone.lock().await;
    assert!(calls.is_empty());
    handle.abort();
}

#[tokio::test]
async fn issues_create_forwards_request_and_returns_receipt() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-issue-create".into(),
        op: IssueOp::Create,
        idempotency_key: None,
        team_id: "team_1".into(),
        issue_id: None,
        title: "Demo issue".into(),
        description: Some("body".into()),
        priority: Some(2.0),
        estimate: Some(3.0),
        assignee_id: Some("user_1".into()),
        project_id: Some("proj_1".into()),
        project_milestone_id: None,
        label_ids: Some(vec!["label_a".into()]),
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphIssueResponse = resp.json().await.unwrap();
    assert_eq!(body.issue_id.as_deref(), Some("iss_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert_eq!(body.receipt.correlation_id, "corr-issue-create");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.issue.lock().await;
    let (forwarded, cid) = &calls[0];
    assert_eq!(cid, "corr-issue-create");
    assert_eq!(forwarded.title, "Demo issue");
    assert_eq!(forwarded.team_id, "team_1");
    assert!(matches!(forwarded.op, IssueOp::Create));

    // Create op must emit the `TaskGraphIssueCreated` event variant.
    let events = journal.recent_events(10).await;
    assert!(
        events.iter().any(|rec| matches!(
            rec.kind,
            JournalEventKind::TaskGraphIssueCreated { ref issue_id, .. } if issue_id == "iss_fake"
        )),
        "expected TaskGraphIssueCreated{{issue_id=\"iss_fake\"}} in {events:?}"
    );
    handle.abort();
}

#[tokio::test]
async fn issues_update_forwards_issue_id_to_fake_client() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-issue-update".into(),
        op: IssueOp::Update,
        idempotency_key: None,
        team_id: "team_1".into(),
        issue_id: Some("iss_existing".into()),
        title: "Renamed issue".into(),
        description: None,
        priority: None,
        estimate: None,
        assignee_id: None,
        project_id: None,
        project_milestone_id: None,
        label_ids: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphIssueResponse = resp.json().await.unwrap();
    assert_eq!(body.receipt.correlation_id, "corr-issue-update");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.issue.lock().await;
    let (forwarded, cid) = &calls[0];
    assert_eq!(cid, "corr-issue-update");
    assert_eq!(forwarded.issue_id.as_deref(), Some("iss_existing"));
    assert!(matches!(forwarded.op, IssueOp::Update));

    // Update op must emit the `TaskGraphIssueUpdated` event variant
    // (not `TaskGraphIssueCreated`).
    let events = journal.recent_events(10).await;
    assert!(
        events.iter().any(|rec| matches!(
            rec.kind,
            JournalEventKind::TaskGraphIssueUpdated { ref issue_id } if issue_id == "iss_existing"
        )),
        "expected TaskGraphIssueUpdated{{issue_id=\"iss_existing\"}} in {events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|rec| matches!(rec.kind, JournalEventKind::TaskGraphIssueCreated { .. })),
        "did not expect a TaskGraphIssueCreated event for an Update op: {events:?}"
    );
    handle.abort();
}

#[tokio::test]
async fn sub_issue_create_forwards_parent_identifier_and_returns_receipt() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphSubIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-sub-issue-create".into(),
        op: SubIssueOp::Create,
        idempotency_key: None,
        team_id: "team_1".into(),
        parent_id: "parent_1".into(),
        sub_issue_id: None,
        parent_identifier: "COE-405".into(),
        title: "Sub issue".into(),
        description: None,
        priority: Some(3.0),
        estimate: None,
        assignee_id: None,
        project_id: None,
        project_milestone_id: None,
        label_ids: None,
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/sub-issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphSubIssueResponse = resp.json().await.unwrap();
    assert_eq!(body.sub_issue_id.as_deref(), Some("sub_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.sub_issue.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.parent_identifier, "COE-405");
    assert!(matches!(calls[0].0.op, SubIssueOp::Create));

    // Create op must emit `TaskGraphSubIssueCreated` and carry the parent
    // identifier so downstream caches can correlate the new node.
    let events = journal.recent_events(10).await;
    assert!(
        events.iter().any(|rec| matches!(
            rec.kind,
            JournalEventKind::TaskGraphSubIssueCreated { ref sub_issue_id, ref parent_identifier }
            if sub_issue_id == "sub_fake" && parent_identifier == "COE-405"
        )),
        "expected TaskGraphSubIssueCreated{{sub_issue_id=\"sub_fake\", parent_identifier=\"COE-405\"}} in {events:?}"
    );
    handle.abort();
}

#[tokio::test]
async fn relations_create_preserves_dependency_metadata() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphRelationRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-relation".into(),
        idempotency_key: None,
        relation_type: "blocks".into(),
        issue_id: "COE-405".into(),
        related_issue_id: "COE-411".into(),
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/relations"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphRelationResponse = resp.json().await.unwrap();
    assert_eq!(body.relation_id.as_deref(), Some("rel_fake"));
    assert_eq!(body.related_issue_id.as_deref(), Some("COE-411"));
    assert_eq!(body.relation_type.as_deref(), Some("blocks"));
    assert_eq!(body.receipt.correlation_id, "corr-relation");
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.relation.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0.related_issue_id, "COE-411");
    handle.abort();
}

#[tokio::test]
async fn evidence_create_returns_comment_receipt_with_taskgraph_followup() {
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr) = start_test_server(fake.clone()).await;

    let req = TaskGraphEvidenceRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-evidence".into(),
        idempotency_key: None,
        issue_id: "COE-405".into(),
        body: "evidence body".into(),
    };

    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/evidence"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: TaskGraphEvidenceResponse = resp.json().await.unwrap();
    assert_eq!(body.comment_id.as_deref(), Some("c_fake"));
    assert_eq!(body.receipt.status, ActionStatus::Accepted);
    assert!(
        body.receipt
            .expected_followup
            .contains(&ExpectedFollowup::TaskGraphUpdate)
    );

    let calls = fake.calls.evidence.lock().await;
    assert_eq!(calls.len(), 1);
    handle.abort();
}

#[tokio::test]
async fn evidence_journal_entity_ref_correlates_by_issue_id() {
    // Regression guard for the round-4 audit-journal finding: an evidence
    // event was stamped with `entity_ref.id = comment_id` even though
    // `entity_ref.kind = Issue`, so future journal queries of the form
    // "events for issue X" silently missed the comment. The fix pins
    // `entity_ref.id` to `issue_id` and keeps `comment_id` in the typed
    // `EventKind::TaskGraphCommentCreated` payload.
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphEvidenceRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-evidence-corr-id".into(),
        idempotency_key: None,
        issue_id: "COE-405".into(),
        body: "evidence body".into(),
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/evidence"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let events = journal.recent_events(10).await;
    let comment_event = events
        .iter()
        .find(|rec| matches!(rec.kind, JournalEventKind::TaskGraphCommentCreated { .. }))
        .expect("expected TaskGraphCommentCreated event in journal");
    let entity_ref = comment_event
        .entity_refs
        .first()
        .expect("event should carry an entity_ref");
    assert_eq!(
        entity_ref.id, "COE-405",
        "entity_ref.id must be the issue id so 'events for issue X' filters find the comment"
    );
    assert_eq!(entity_ref.kind, EntityKind::Issue);
    handle.abort();
}

#[tokio::test]
async fn relation_journal_entity_ref_correlates_by_from_issue_id() {
    // Regression guard for the round-4 audit-journal finding: the relation
    // event stamped `entity_ref.id = relation_id` while `kind = Issue`, so
    // issue-keyed queries dropped it. The fix uses the request's
    // `issue_id` (the "from" issue of the relation) as `entity_ref.id`.
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphRelationRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-relation-corr-id".into(),
        idempotency_key: None,
        relation_type: "blocks".into(),
        issue_id: "COE-405".into(),
        related_issue_id: "COE-411".into(),
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/relations"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let events = journal.recent_events(10).await;
    let relation_event = events
        .iter()
        .find(|rec| matches!(rec.kind, JournalEventKind::TaskGraphRelationCreated { .. }))
        .expect("expected TaskGraphRelationCreated event in journal");
    let entity_ref = relation_event
        .entity_refs
        .first()
        .expect("event should carry an entity_ref");
    assert_eq!(
        entity_ref.id, "COE-405",
        "entity_ref.id must be the from-issue id so 'events for issue X' filters find the relation"
    );
    assert_eq!(entity_ref.kind, EntityKind::Issue);
    handle.abort();
}

#[tokio::test]
async fn issue_journal_payload_renames_milestone_field() {
    // Regression guard for the round-4 review item pointing out that the
    // audit payload field called `parent_identifier` actually carried
    // `project_milestone_id` (a milestone id, not an issue identifier).
    // The fix renames the payload field to `milestone_id`.
    let fake = Arc::new(FakeLinearClient::new());
    let (handle, addr, journal) = start_test_server_with_journal(fake.clone()).await;

    let req = TaskGraphIssueRequest {
        schema_version: "1.0.0".into(),
        correlation_id: "corr-issue-milestone-field".into(),
        op: IssueOp::Create,
        idempotency_key: None,
        team_id: "team_1".into(),
        issue_id: None,
        title: "M-rename test issue".into(),
        description: None,
        priority: None,
        estimate: None,
        assignee_id: None,
        project_id: None,
        project_milestone_id: Some("ms_target".into()),
        label_ids: None,
    };
    let resp = Client::new()
        .post(format!("http://{addr}/api/v1/taskgraph/issues"))
        .json(&req)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let events = journal.recent_events(10).await;
    let issue_event = events
        .iter()
        .find(|rec| matches!(rec.kind, JournalEventKind::TaskGraphIssueCreated { .. }))
        .expect("expected TaskGraphIssueCreated event in journal");
    let payload = issue_event.payload.clone().expect("payload should be set");
    assert_eq!(
        payload.get("milestone_id").and_then(|v| v.as_str()),
        Some("ms_target"),
        "milestone id must be carried under the `milestone_id` payload field"
    );
    assert!(
        payload.get("parent_identifier").is_none(),
        "renamed `milestone_id` payload field must replace the misleading `parent_identifier`"
    );
    handle.abort();
}

#[tokio::test]
async fn taskgraph_routes_return_503_when_no_linear_mutation_client_is_wired() {
    // When `GatewayServer::with_linear_mutations(...)` is never called, every
    // `/api/v1/taskgraph/*` endpoint must short-circuit with 503 rather than
    // attempting to dereference an absent `LinearMutationClient`.
    let (handle, addr) = start_test_server_without_linear_mutations().await;
    let client = Client::new();

    // Milestones
    let resp = client
        .post(format!("http://{addr}/api/v1/taskgraph/milestones"))
        .json(&TaskGraphMilestoneRequest {
            schema_version: "1.0.0".into(),
            correlation_id: "corr-unavail".into(),
            op: MilestoneOp::Create,
            idempotency_key: None,
            project_id: "proj_1".into(),
            milestone_id: None,
            name: "ignored".into(),
            description: None,
            target_date: None,
            sort_order: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    // Issues
    let resp = client
        .post(format!("http://{addr}/api/v1/taskgraph/issues"))
        .json(&TaskGraphIssueRequest {
            schema_version: "1.0.0".into(),
            correlation_id: "corr-unavail".into(),
            op: IssueOp::Create,
            idempotency_key: None,
            team_id: "team_1".into(),
            issue_id: None,
            title: "ignored".into(),
            description: None,
            priority: None,
            estimate: None,
            assignee_id: None,
            project_id: None,
            project_milestone_id: None,
            label_ids: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    // Sub-issues
    let resp = client
        .post(format!("http://{addr}/api/v1/taskgraph/sub-issues"))
        .json(&TaskGraphSubIssueRequest {
            schema_version: "1.0.0".into(),
            correlation_id: "corr-unavail".into(),
            op: SubIssueOp::Create,
            idempotency_key: None,
            team_id: "team_1".into(),
            parent_id: "iss_1".into(),
            sub_issue_id: None,
            parent_identifier: "COE-405".into(),
            title: "ignored".into(),
            description: None,
            priority: None,
            estimate: None,
            assignee_id: None,
            project_id: None,
            project_milestone_id: None,
            label_ids: None,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    // Relations
    let resp = client
        .post(format!("http://{addr}/api/v1/taskgraph/relations"))
        .json(&TaskGraphRelationRequest {
            schema_version: "1.0.0".into(),
            correlation_id: "corr-unavail".into(),
            idempotency_key: None,
            relation_type: "blocks".into(),
            issue_id: "iss_1".into(),
            related_issue_id: "iss_2".into(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    // Evidence / comments
    let resp = client
        .post(format!("http://{addr}/api/v1/taskgraph/evidence"))
        .json(&TaskGraphEvidenceRequest {
            schema_version: "1.0.0".into(),
            correlation_id: "corr-unavail".into(),
            idempotency_key: None,
            issue_id: "iss_1".into(),
            body: "ignored".into(),
        })
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 503);

    handle.abort();
}
