//! Gateway-mediated task-graph mutation endpoints (`/api/v1/taskgraph/*`).
//!
//! These endpoints are designed to be honest about the fact that the gateway is
//! a *mediator*, not the source of truth, for the Linear task graph. They
//! share the same action envelope (`ActionKind`, `ActionReceipt`) as
//! `POST /api/v1/actions/dispatch` so that the journal can correlate them
//! with follow-up `TaskGraph*` events under a single `correlation_id`.
//!
//! The actual Linear mutation is performed by a `LinearMutationClient`
//! abstraction (an in-process trait). Production wiring uses a thin shim that
//! forwards to `opensymphony_linear::LinearClient`; tests inject a fake that
//! speaks to a `MockGraphqlServer` so the test plan can exercise success,
//! validation failure, permission failure, and schema drift cases without
//! hitting the live tracker.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::opensymphony_domain::InMemoryEventJournal;
use crate::opensymphony_gateway_schema::action::{
    ActionKind, ActionReceipt, ExpectedFollowup, PermissionResult,
};
use crate::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};
use crate::opensymphony_gateway_schema::event_journal::{EventActor, EventKind, EventRecord};

// =============================================================================
// Request DTOs (Linear-native shapes).
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneOp {
    Create,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphMilestoneRequest {
    pub schema_version: String,
    pub correlation_id: String,
    pub op: MilestoneOp,
    pub idempotency_key: Option<String>,
    pub project_id: String,
    /// Required for `Update`; forwarded as the URL id and validated by the
    /// adapter. Ignored for `Create` (the milestone id is chosen by Linear).
    pub milestone_id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueOp {
    Create,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphIssueRequest {
    pub schema_version: String,
    pub correlation_id: String,
    pub op: IssueOp,
    pub idempotency_key: Option<String>,
    pub team_id: String,
    /// Required for `Update`.
    pub issue_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<f64>,
    pub estimate: Option<f64>,
    pub assignee_id: Option<String>,
    pub project_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub label_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubIssueOp {
    Create,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphSubIssueRequest {
    pub schema_version: String,
    pub correlation_id: String,
    pub op: SubIssueOp,
    pub idempotency_key: Option<String>,
    pub team_id: String,
    pub parent_id: String,
    /// Required for `Update`.
    pub sub_issue_id: Option<String>,
    pub parent_identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<f64>,
    pub estimate: Option<f64>,
    pub assignee_id: Option<String>,
    pub project_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub label_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphRelationRequest {
    pub schema_version: String,
    pub correlation_id: String,
    pub idempotency_key: Option<String>,
    /// "blocks" / "blocked_by" / "related" / "duplicate".
    pub relation_type: String,
    pub issue_id: String,
    pub related_issue_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphEvidenceRequest {
    pub schema_version: String,
    pub correlation_id: String,
    pub idempotency_key: Option<String>,
    pub issue_id: String,
    pub body: String,
}

// =============================================================================
// Response DTOs.
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphMilestoneResponse {
    pub receipt: ActionReceipt,
    pub milestone_id: Option<String>,
    pub milestone_name: Option<String>,
    pub project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphIssueResponse {
    pub receipt: ActionReceipt,
    pub issue_id: Option<String>,
    pub issue_identifier: Option<String>,
    pub state_id: Option<String>,
    pub project_milestone_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphSubIssueResponse {
    pub receipt: ActionReceipt,
    pub sub_issue_id: Option<String>,
    pub sub_issue_identifier: Option<String>,
    pub parent_identifier: Option<String>,
    pub state_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphRelationResponse {
    pub receipt: ActionReceipt,
    pub relation_id: Option<String>,
    pub relation_type: Option<String>,
    pub related_issue_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskGraphEvidenceResponse {
    pub receipt: ActionReceipt,
    pub comment_id: Option<String>,
    pub issue_id: Option<String>,
    pub issue_identifier: Option<String>,
}

// =============================================================================
// Errors (translated into `ActionReceipt::rejected` reasons).
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MutationError {
    Validation(String),
    PermissionDenied(String),
    SchemaDrift(String),
    Upstream(String),
    /// The mutation client is not configured (i.e. no `LinearMutationClient`
    /// was wired into the gateway). Surfaced as HTTP 503 Service Unavailable.
    Unavailable(String),
}

impl MutationError {
    pub fn as_reason(&self) -> String {
        match self {
            MutationError::Validation(reason) => format!("validation failed: {reason}"),
            MutationError::PermissionDenied(reason) => format!("permission denied: {reason}"),
            MutationError::SchemaDrift(reason) => format!("schema drift: {reason}"),
            MutationError::Upstream(reason) => format!("upstream error: {reason}"),
            MutationError::Unavailable(reason) => {
                format!("mutation client unavailable: {reason}")
            }
        }
    }
}

/// Trait used to expose the shared `IssueCreateInput` field set on both
/// `TaskGraphIssueRequest` and `TaskGraphSubIssueRequest`. Lets the issue and
/// sub-issue adapters route through one helper instead of duplicating the
/// struct construction.
trait IssueCreateInputFields {
    fn description(&self) -> Option<String>;
    fn priority(&self) -> Option<f64>;
    fn estimate(&self) -> Option<f64>;
    fn assignee_id(&self) -> Option<String>;
    fn project_id(&self) -> Option<String>;
    fn project_milestone_id(&self) -> Option<String>;
    fn label_ids(&self) -> Option<Vec<String>>;
}

impl IssueCreateInputFields for TaskGraphIssueRequest {
    fn description(&self) -> Option<String> {
        self.description.clone()
    }
    fn priority(&self) -> Option<f64> {
        self.priority
    }
    fn estimate(&self) -> Option<f64> {
        self.estimate
    }
    fn assignee_id(&self) -> Option<String> {
        self.assignee_id.clone()
    }
    fn project_id(&self) -> Option<String> {
        self.project_id.clone()
    }
    fn project_milestone_id(&self) -> Option<String> {
        self.project_milestone_id.clone()
    }
    fn label_ids(&self) -> Option<Vec<String>> {
        self.label_ids.clone()
    }
}

impl IssueCreateInputFields for TaskGraphSubIssueRequest {
    fn description(&self) -> Option<String> {
        self.description.clone()
    }
    fn priority(&self) -> Option<f64> {
        self.priority
    }
    fn estimate(&self) -> Option<f64> {
        self.estimate
    }
    fn assignee_id(&self) -> Option<String> {
        self.assignee_id.clone()
    }
    fn project_id(&self) -> Option<String> {
        self.project_id.clone()
    }
    fn project_milestone_id(&self) -> Option<String> {
        self.project_milestone_id.clone()
    }
    fn label_ids(&self) -> Option<Vec<String>> {
        self.label_ids.clone()
    }
}

fn build_issue_create_input<R: IssueCreateInputFields>(
    team_id: String,
    title: String,
    request: &R,
    parent_id: Option<String>,
) -> IssueCreateInput {
    IssueCreateInput {
        team_id,
        title,
        description: request.description(),
        priority: request.priority(),
        estimate: request.estimate(),
        state_id: None,
        assignee_id: request.assignee_id(),
        project_id: request.project_id(),
        project_milestone_id: request.project_milestone_id(),
        parent_id,
        label_ids: request.label_ids(),
    }
}

// =============================================================================
// LinearMutationClient trait — the seam between gateway handlers and the
// real (or fake) Linear client.
// =============================================================================

#[async_trait]
pub trait LinearMutationClient: Send + Sync + 'static {
    /// Dispatch a milestone mutation. The `op` field on `request` selects
    /// which underlying Linear mutation the adapter performs.
    async fn create_or_update_project_milestone(
        &self,
        request: TaskGraphMilestoneRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphMilestoneResponse, MutationError>;

    async fn create_or_update_issue(
        &self,
        request: TaskGraphIssueRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphIssueResponse, MutationError>;

    async fn create_or_update_sub_issue(
        &self,
        request: TaskGraphSubIssueRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphSubIssueResponse, MutationError>;

    async fn create_issue_relation(
        &self,
        request: TaskGraphRelationRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphRelationResponse, MutationError>;

    async fn create_evidence_comment(
        &self,
        request: TaskGraphEvidenceRequest,
        correlation_id: &str,
    ) -> Result<TaskGraphEvidenceResponse, MutationError>;
}

// =============================================================================
// Default LinearMutationClient implementation that round-trips through the
// real `opensymphony_linear::LinearClient`. Wrapped in a single trait object
// so tests can substitute fakes.
// =============================================================================

use crate::opensymphony_linear::{
    IssueCreateInput, IssueUpdateInput, LinearClient, LinearCommentMutationResult,
    LinearIssueMutationResult, LinearIssueRelationMutationResult, LinearMilestoneMutationResult,
    ProjectMilestoneCreateInput, ProjectMilestoneUpdateInput,
};

pub struct LinearClientMutationAdapter {
    client: Arc<LinearClient>,
}

impl LinearClientMutationAdapter {
    pub fn new(client: Arc<LinearClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LinearMutationClient for LinearClientMutationAdapter {
    async fn create_or_update_project_milestone(
        &self,
        request: TaskGraphMilestoneRequest,
        _correlation_id: &str,
    ) -> Result<TaskGraphMilestoneResponse, MutationError> {
        // The adapter ultimately threads `request.correlation_id` into the
        // generated `ActionReceipt`, so we don't need this parameter here.
        let project_id = request.project_id.trim().to_string();
        if project_id.is_empty() {
            return Err(MutationError::Validation("project_id is required".into()));
        }
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err(MutationError::Validation("name is required".into()));
        }
        let action_id = Uuid::new_v4().to_string();
        let result: LinearMilestoneMutationResult = match request.op {
            MilestoneOp::Create => {
                let input = ProjectMilestoneCreateInput {
                    project_id,
                    name,
                    description: request.description,
                    target_date: request.target_date,
                    sort_order: request.sort_order,
                };
                self.client
                    .create_project_milestone(input)
                    .await
                    .map_err(map_linear_err)?
            }
            MilestoneOp::Update => {
                let milestone_id = request.milestone_id.clone().ok_or_else(|| {
                    MutationError::Validation("milestone_id required for update".into())
                })?;
                let input = ProjectMilestoneUpdateInput {
                    name: Some(name),
                    description: request.description,
                    target_date: request.target_date,
                    sort_order: request.sort_order,
                };
                self.client
                    .update_project_milestone(&milestone_id, input)
                    .await
                    .map_err(map_linear_err)?
            }
        };
        Ok(build_milestone_response_accepted(
            &action_id,
            &result,
            &request.correlation_id,
        ))
    }

    async fn create_or_update_issue(
        &self,
        request: TaskGraphIssueRequest,
        _correlation_id: &str,
    ) -> Result<TaskGraphIssueResponse, MutationError> {
        // The adapter ultimately threads `request.correlation_id` into the
        // generated `ActionReceipt`, so we don't need this parameter here.
        let team_id = request.team_id.trim().to_string();
        let title = request.title.trim().to_string();
        if team_id.is_empty() {
            return Err(MutationError::Validation("team_id is required".into()));
        }
        if title.is_empty() {
            return Err(MutationError::Validation("title is required".into()));
        }
        let action_id = Uuid::new_v4().to_string();
        let issue_id = request.issue_id.clone();
        let input = build_issue_create_input(team_id, title, &request, None);
        let result: LinearIssueMutationResult = match request.op {
            IssueOp::Create => self
                .client
                .create_issue(input)
                .await
                .map_err(map_linear_err)?,
            IssueOp::Update => {
                let id = issue_id.clone().ok_or_else(|| {
                    MutationError::Validation("issue_id required for update".into())
                })?;
                let update_input = IssueUpdateInput {
                    title: Some(input.title),
                    description: input.description,
                    priority: input.priority,
                    estimate: input.estimate,
                    state_id: None,
                    assignee_id: input.assignee_id,
                    project_id: input.project_id,
                    project_milestone_id: input.project_milestone_id,
                    label_ids: input.label_ids,
                };
                self.client
                    .update_issue(&id, update_input)
                    .await
                    .map_err(map_linear_err)?
            }
        };
        Ok(build_issue_response_accepted(
            &action_id,
            &result,
            &request.correlation_id,
        ))
    }

    async fn create_or_update_sub_issue(
        &self,
        request: TaskGraphSubIssueRequest,
        _correlation_id: &str,
    ) -> Result<TaskGraphSubIssueResponse, MutationError> {
        // The adapter ultimately threads `request.correlation_id` into the
        // generated `ActionReceipt`, so we don't need this parameter here.
        let team_id = request.team_id.trim().to_string();
        let title = request.title.trim().to_string();
        let parent_id = request.parent_id.trim().to_string();
        if team_id.is_empty() {
            return Err(MutationError::Validation("team_id is required".into()));
        }
        if parent_id.is_empty() {
            return Err(MutationError::Validation("parent_id is required".into()));
        }
        if title.is_empty() {
            return Err(MutationError::Validation("title is required".into()));
        }
        let action_id = Uuid::new_v4().to_string();
        let input = build_issue_create_input(team_id, title, &request, Some(parent_id));
        let result: LinearIssueMutationResult = match request.op {
            SubIssueOp::Create => self
                .client
                .create_issue(input)
                .await
                .map_err(map_linear_err)?,
            SubIssueOp::Update => {
                let id = request.sub_issue_id.clone().ok_or_else(|| {
                    MutationError::Validation("sub_issue_id required for update".into())
                })?;
                let update_input = IssueUpdateInput {
                    title: Some(input.title),
                    description: input.description,
                    priority: input.priority,
                    estimate: input.estimate,
                    state_id: None,
                    assignee_id: input.assignee_id,
                    project_id: input.project_id,
                    project_milestone_id: input.project_milestone_id,
                    label_ids: input.label_ids,
                };
                self.client
                    .update_issue(&id, update_input)
                    .await
                    .map_err(map_linear_err)?
            }
        };
        Ok(LinearResponseForSubIssue(&result).into_response(&action_id, &request.correlation_id))
    }

    async fn create_issue_relation(
        &self,
        request: TaskGraphRelationRequest,
        _correlation_id: &str,
    ) -> Result<TaskGraphRelationResponse, MutationError> {
        // The adapter ultimately threads `request.correlation_id` into the
        // generated `ActionReceipt`, so we don't need this parameter here.
        let relation_type = request.relation_type.trim().to_string();
        if request.issue_id.trim().is_empty() {
            return Err(MutationError::Validation("issue_id is required".into()));
        }
        if request.related_issue_id.trim().is_empty() {
            return Err(MutationError::Validation(
                "related_issue_id is required".into(),
            ));
        }
        if relation_type.is_empty() {
            return Err(MutationError::Validation(
                "relation_type is required".into(),
            ));
        }
        let action_id = Uuid::new_v4().to_string();
        let result: LinearIssueRelationMutationResult = self
            .client
            .create_issue_relation(&request.issue_id, &request.related_issue_id, &relation_type)
            .await
            .map_err(map_linear_err)?;
        Ok(TaskGraphRelationResponse {
            receipt: build_accepted_receipt(
                &action_id,
                &request.correlation_id,
                ActionKind::TaskGraphRelation,
            ),
            relation_id: Some(result.id.clone()),
            relation_type: Some(result.relation_type.clone()),
            related_issue_id: Some(request.related_issue_id.clone()),
        })
    }

    async fn create_evidence_comment(
        &self,
        request: TaskGraphEvidenceRequest,
        _correlation_id: &str,
    ) -> Result<TaskGraphEvidenceResponse, MutationError> {
        // The adapter ultimately threads `request.correlation_id` into the
        // generated `ActionReceipt`, so we don't need this parameter here.
        if request.issue_id.trim().is_empty() {
            return Err(MutationError::Validation("issue_id is required".into()));
        }
        let body = request.body.trim().to_string();
        if body.is_empty() {
            return Err(MutationError::Validation("body is required".into()));
        }
        let action_id = Uuid::new_v4().to_string();
        let result: LinearCommentMutationResult = self
            .client
            .create_comment(&request.issue_id, &body)
            .await
            .map_err(map_linear_err)?;
        Ok(TaskGraphEvidenceResponse {
            receipt: build_accepted_receipt(
                &action_id,
                &request.correlation_id,
                ActionKind::TaskGraphEvidence,
            ),
            comment_id: Some(result.id.clone()),
            issue_id: Some(result.issue_id.clone()),
            issue_identifier: Some(result.issue_identifier.clone()),
        })
    }
}

// Helper adapter to bind permission result for sub-issue responses (needed
// only because the conversion has more fields than the free function form).
struct LinearResponseForSubIssue<'a>(&'a LinearIssueMutationResult);

impl<'a> LinearResponseForSubIssue<'a> {
    fn into_response(self, action_id: &str, correlation_id: &str) -> TaskGraphSubIssueResponse {
        TaskGraphSubIssueResponse {
            receipt: build_accepted_receipt_with_followups(
                action_id,
                correlation_id,
                ActionKind::TaskGraphSubIssue,
            )
            .with_permission(PermissionResult::local()),
            sub_issue_id: Some(self.0.id.clone()),
            sub_issue_identifier: Some(self.0.identifier.clone()),
            parent_identifier: self.0.parent_identifier.clone(),
            state_id: Some(self.0.state_id.clone()),
        }
    }
}

fn map_linear_err(err: crate::opensymphony_linear::LinearError) -> MutationError {
    use crate::opensymphony_linear::LinearError;
    match err {
        LinearError::InvalidConfiguration(detail) => MutationError::Validation(detail),
        LinearError::HttpStatus { status, body, .. } => {
            // Compare against the raw `u16` we already have so we don't have
            // to allocate a `reqwest::StatusCode` just to ask "is it 401/403?".
            if status.as_u16() == 401 || status.as_u16() == 403 {
                MutationError::PermissionDenied(format!("linear returned HTTP {status}: {body}"))
            } else {
                MutationError::Upstream(format!("linear returned HTTP {status}: {body}"))
            }
        }
        LinearError::Graphql { errors, .. } => {
            // Linear GraphQL error envelope. Treat any well-known schema/code
            // marker as a schema-drift signal or as a permission signal so
            // existing observability tooling can correlate.
            let summary = errors
                .iter()
                .map(|e| match &e.code {
                    Some(code) => format!("{code}: {}", e.message),
                    None => e.message.clone(),
                })
                .collect::<Vec<_>>()
                .join("; ");
            let lowered = summary.to_lowercase();
            if lowered.contains("forbidden")
                || lowered.contains("unauthorized")
                || lowered.contains("permission")
            {
                MutationError::PermissionDenied(summary)
            } else if lowered.contains("field") && lowered.contains("not found") {
                MutationError::SchemaDrift(summary)
            } else {
                MutationError::Upstream(summary)
            }
        }
        LinearError::MissingIssueIds { .. } => {
            MutationError::Upstream("linear response missing issue ids".into())
        }
        LinearError::InvalidResponse(detail) => MutationError::Upstream(detail),
        other => MutationError::Upstream(other.to_string()),
    }
}

fn build_milestone_response_accepted(
    action_id: &str,
    milestone: &LinearMilestoneMutationResult,
    correlation_id: &str,
) -> TaskGraphMilestoneResponse {
    TaskGraphMilestoneResponse {
        receipt: build_accepted_receipt(action_id, correlation_id, ActionKind::TaskGraphMilestone)
            .with_permission(PermissionResult::local()),
        milestone_id: Some(milestone.id.clone()),
        milestone_name: Some(milestone.name.clone()),
        project_id: Some(milestone.project_id.clone()),
    }
}

fn build_issue_response_accepted(
    action_id: &str,
    issue: &LinearIssueMutationResult,
    correlation_id: &str,
) -> TaskGraphIssueResponse {
    TaskGraphIssueResponse {
        receipt: build_accepted_receipt(action_id, correlation_id, ActionKind::TaskGraphIssue)
            .with_permission(PermissionResult::local()),
        issue_id: Some(issue.id.clone()),
        issue_identifier: Some(issue.identifier.clone()),
        state_id: Some(issue.state_id.clone()),
        project_milestone_id: issue.project_milestone_id.clone(),
    }
}

fn build_accepted_receipt(
    action_id: &str,
    correlation_id: &str,
    kind: ActionKind,
) -> ActionReceipt {
    build_accepted_receipt_with_followups(action_id, correlation_id, kind)
}

fn build_accepted_receipt_with_followups(
    action_id: &str,
    correlation_id: &str,
    kind: ActionKind,
) -> ActionReceipt {
    let mut receipt = ActionReceipt::accepted(action_id.to_owned(), correlation_id, kind);
    if !receipt
        .expected_followup
        .contains(&ExpectedFollowup::TaskGraphUpdate)
    {
        receipt
            .expected_followup
            .push(ExpectedFollowup::TaskGraphUpdate);
    }
    receipt
}

fn build_audit_event_inner(
    actor_id: &str,
    correlation_id: &str,
    entity_ref: EntityRef,
    kind: EventKind,
    payload: Value,
) -> EventRecord {
    EventRecord::builder()
        .actor(EventActor::system(actor_id))
        .correlation_id(correlation_id.to_owned())
        .kind(kind)
        .entity_ref(entity_ref)
        .summary("task graph mutation")
        .payload(payload)
        .build()
}

pub fn entity_kind_for(kind: ActionKind) -> EntityKind {
    match kind {
        ActionKind::TaskGraphMilestone => EntityKind::Milestone,
        ActionKind::TaskGraphIssue => EntityKind::Issue,
        ActionKind::TaskGraphSubIssue => EntityKind::SubIssue,
        ActionKind::TaskGraphRelation | ActionKind::TaskGraphEvidence => EntityKind::Issue,
        _ => EntityKind::Unknown,
    }
}

/// Append the per-mutation event to the journal so the client can subscribe.
///
/// This is intentionally minimal: the dispatcher at `/api/v1/actions/dispatch`
/// already handles general-purpose action auditing, and this helper just
/// ensures the task-graph-specific variants land in the journal too.
///
/// Evidence/comment events are built inline in `task_graph_evidence_handler`
/// because they need to carry the real `issue_id` alongside the comment id.
pub async fn append_mutation_event(
    journal: &InMemoryEventJournal,
    correlation_id: &str,
    kind: ActionKind,
    entity_ref: EntityRef,
    payload: Value,
) -> Result<(), String> {
    append_mutation_event_with_op(
        journal,
        correlation_id,
        MutationOp::Upsert,
        kind,
        entity_ref,
        payload,
    )
    .await
}

/// Like [`append_mutation_event`] but lets the caller pick between the
/// `Created` and `Updated` event variants on the resulting `TaskGraph*`
/// event. Default behavior for relation events and any unknown action kind
/// is unchanged.
pub async fn append_mutation_event_with_op(
    journal: &InMemoryEventJournal,
    correlation_id: &str,
    op: MutationOp,
    kind: ActionKind,
    entity_ref: EntityRef,
    payload: Value,
) -> Result<(), String> {
    use MutationOp::*;
    let event_kind = match (kind, op) {
        (ActionKind::TaskGraphMilestone, Created) => EventKind::TaskGraphMilestoneCreated {
            milestone_id: entity_ref.id.clone(),
        },
        (ActionKind::TaskGraphMilestone, Updated) => EventKind::TaskGraphMilestoneUpdated {
            milestone_id: entity_ref.id.clone(),
        },
        (ActionKind::TaskGraphIssue, Created) => {
            // Top-level issues don't have a parent issue by definition
            // (sub-issues do, and that's a separate variant). Surface only
            // the issue_id so consumers don't accidentally key off a
            // placeholder parent identifier that the issue handler never
            // sets in the payload.
            EventKind::TaskGraphIssueCreated {
                issue_id: entity_ref.id.clone(),
            }
        }
        (ActionKind::TaskGraphIssue, _) => EventKind::TaskGraphIssueUpdated {
            issue_id: entity_ref.id.clone(),
        },
        (ActionKind::TaskGraphSubIssue, Created) => {
            // Sub-issue Created events carry the parent identifier
            // explicitly (sub-issues always have a parent).
            let parent_identifier = payload
                .get("parent_identifier")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned();
            EventKind::TaskGraphSubIssueCreated {
                sub_issue_id: entity_ref.id.clone(),
                parent_identifier,
            }
        }
        (ActionKind::TaskGraphSubIssue, _) => EventKind::TaskGraphSubIssueUpdated {
            sub_issue_id: entity_ref.id.clone(),
        },
        (ActionKind::TaskGraphRelation, _) => EventKind::TaskGraphRelationCreated {
            relation_id: entity_ref.id.clone(),
            relation_type: entity_ref.identifier.clone().unwrap_or_default(),
        },
        (other, _) => {
            return Err(format!(
                "append_mutation_event_with_op called with non-taskgraph action {other} (use the inline builder for evidence)"
            ));
        }
    };
    let record =
        build_audit_event_inner("gateway", correlation_id, entity_ref, event_kind, payload);
    match journal.append(record).await {
        Ok(_) => Ok(()),
        Err(err) => {
            // We deliberately do not propagate this: the journal is an audit
            // mirror, not the source of truth for the mutation, and the
            // already-built response is on its way to the caller. But we
            // must not silently drop the failure — surfaced via tracing.
            tracing::warn!(
                correlation_id = %correlation_id,
                action = ?kind,
                error = ?err,
                "failed to append task-graph mutation event to journal",
            );
            Err(format!("{err:?}"))
        }
    }
}

/// `Created` vs `Updated` modifier for [`append_mutation_event_with_op`].
///
/// `Upsert` is the backwards-compatible default that picks the `Updated`
/// variant for issue / milestone / sub-issue actions (matching the pre-fix
/// behaviour, kept so callers that genuinely don't know the op semantic
/// fall back to something sensible).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationOp {
    Created,
    Updated,
    Upsert,
}

// =============================================================================
// Axum state for the mutation handlers.
//
// We define a dedicated state type so the router for `/api/v1/taskgraph/*`
// can be assembled independently of the rest of the gateway and merged in.
// The wrapper keeps the handler code from depending on `crate::GatewayState`
// directly, which would import the rest of the dependency tree.
// =============================================================================

/// State shared by the task graph mutation handlers. Only fields that the
/// handlers actually consume live here to keep the dependency surface
/// minimal and the state struct honest about what it holds.
#[derive(Clone)]
pub struct TaskGraphMutationState {
    pub journal: InMemoryEventJournal,
    pub linear_mutations: Option<Arc<dyn LinearMutationClient>>,
}

impl axum::extract::FromRef<super::GatewayState> for TaskGraphMutationState {
    fn from_ref(state: &super::GatewayState) -> Self {
        Self {
            journal: state.journal.clone(),
            linear_mutations: state.linear_mutations.clone(),
        }
    }
}

fn ensure_correlation_id(supplied: &str) -> String {
    if supplied.trim().is_empty() {
        Uuid::new_v4().to_string()
    } else {
        supplied.to_string()
    }
}

fn mutation_client_unavailable() -> Response {
    (
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "status": "unavailable",
            "reason": "mutation client not configured",
        })),
    )
        .into_response()
}

fn status_for_mutation_error(err: &MutationError) -> axum::http::StatusCode {
    use axum::http::StatusCode;
    match err {
        MutationError::Validation(_) => StatusCode::BAD_REQUEST,
        MutationError::PermissionDenied(_) => StatusCode::FORBIDDEN,
        MutationError::SchemaDrift(_) => StatusCode::UNPROCESSABLE_ENTITY,
        MutationError::Upstream(_) => StatusCode::BAD_GATEWAY,
        MutationError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

fn empty_milestone_response(receipt: ActionReceipt) -> TaskGraphMilestoneResponse {
    TaskGraphMilestoneResponse {
        receipt,
        milestone_id: None,
        milestone_name: None,
        project_id: None,
    }
}

fn empty_issue_response(receipt: ActionReceipt) -> TaskGraphIssueResponse {
    TaskGraphIssueResponse {
        receipt,
        issue_id: None,
        issue_identifier: None,
        state_id: None,
        project_milestone_id: None,
    }
}

fn empty_sub_issue_response(receipt: ActionReceipt) -> TaskGraphSubIssueResponse {
    TaskGraphSubIssueResponse {
        receipt,
        sub_issue_id: None,
        sub_issue_identifier: None,
        parent_identifier: None,
        state_id: None,
    }
}

fn empty_relation_response(receipt: ActionReceipt) -> TaskGraphRelationResponse {
    TaskGraphRelationResponse {
        receipt,
        relation_id: None,
        related_issue_id: None,
        relation_type: None,
    }
}

fn empty_evidence_response(receipt: ActionReceipt) -> TaskGraphEvidenceResponse {
    TaskGraphEvidenceResponse {
        receipt,
        comment_id: None,
        issue_id: None,
        issue_identifier: None,
    }
}

/// Build the axum Router for `/api/v1/taskgraph/*` from a
/// `TaskGraphMutationState` so the parent gateway can merge it.
pub fn task_graph_router() -> axum::Router<TaskGraphMutationState> {
    use axum::routing::post;
    axum::Router::new()
        .route("/milestones", post(task_graph_milestone_handler))
        .route("/issues", post(task_graph_issue_handler))
        .route("/sub-issues", post(task_graph_sub_issue_handler))
        .route("/relations", post(task_graph_relation_handler))
        .route("/evidence", post(task_graph_evidence_handler))
}

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

async fn task_graph_milestone_handler(
    State(state): State<TaskGraphMutationState>,
    Json(mut request): Json<TaskGraphMilestoneRequest>,
) -> Response {
    let Some(client) = state.linear_mutations.clone() else {
        return mutation_client_unavailable();
    };
    let correlation_id = ensure_correlation_id(&request.correlation_id);
    let op = request.op.clone();
    request.correlation_id = correlation_id.clone();
    let journal = state.journal.clone();
    let result = client
        .create_or_update_project_milestone(request, &correlation_id)
        .await;
    let response = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                correlation_id = %correlation_id,
                action = "task_graph.milestone",
                error = ?err,
                "rejected task-graph milestone mutation",
            );
            let receipt = ActionReceipt::rejected(
                Uuid::new_v4().to_string(),
                correlation_id.clone(),
                ActionKind::TaskGraphMilestone,
                err.as_reason(),
            );
            let status = status_for_mutation_error(&err);
            return (status, Json(empty_milestone_response(receipt))).into_response();
        }
    };
    let milestone_id = response.milestone_id.clone().unwrap_or_default();
    let milestone_name = response.milestone_name.clone();
    let project_id = response.project_id.clone();
    let event_op = match op {
        MilestoneOp::Create => MutationOp::Created,
        MilestoneOp::Update => MutationOp::Updated,
    };
    if let Err(err) = append_mutation_event_with_op(
        &journal,
        &correlation_id,
        event_op,
        ActionKind::TaskGraphMilestone,
        EntityRef {
            kind: entity_kind_for(ActionKind::TaskGraphMilestone),
            id: milestone_id.clone(),
            identifier: milestone_name.clone(),
        },
        serde_json::json!({
            "milestone_id": milestone_id,
            "milestone_name": milestone_name,
            "project_id": project_id,
        }),
    )
    .await
    {
        tracing::warn!(
            correlation_id = %correlation_id,
            action = "task_graph.milestone",
            error = %err,
            "failed to record milestone mutation in audit journal",
        );
    }
    (axum::http::StatusCode::OK, Json(response)).into_response()
}

async fn task_graph_issue_handler(
    State(state): State<TaskGraphMutationState>,
    Json(mut request): Json<TaskGraphIssueRequest>,
) -> Response {
    let Some(client) = state.linear_mutations.clone() else {
        return mutation_client_unavailable();
    };
    let correlation_id = ensure_correlation_id(&request.correlation_id);
    let op = request.op.clone();
    request.correlation_id = correlation_id.clone();
    let journal = state.journal.clone();
    let result = client
        .create_or_update_issue(request, &correlation_id)
        .await;
    let response = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                correlation_id = %correlation_id,
                action = "task_graph.issue",
                error = ?err,
                "rejected task-graph issue mutation",
            );
            let receipt = ActionReceipt::rejected(
                Uuid::new_v4().to_string(),
                correlation_id.clone(),
                ActionKind::TaskGraphIssue,
                err.as_reason(),
            );
            let status = status_for_mutation_error(&err);
            return (status, Json(empty_issue_response(receipt))).into_response();
        }
    };
    let issue_id = response.issue_id.clone().unwrap_or_default();
    let issue_identifier = response.issue_identifier.clone();
    let event_op = match op {
        IssueOp::Create => MutationOp::Created,
        IssueOp::Update => MutationOp::Updated,
    };
    if let Err(err) = append_mutation_event_with_op(
        &journal,
        &correlation_id,
        event_op,
        ActionKind::TaskGraphIssue,
        EntityRef {
            kind: entity_kind_for(ActionKind::TaskGraphIssue),
            id: issue_id.clone(),
            identifier: issue_identifier.clone(),
        },
        serde_json::json!({
            "issue_id": issue_id,
            "issue_identifier": issue_identifier,
            "milestone_id": response.project_milestone_id.clone(),
        }),
    )
    .await
    {
        tracing::warn!(
            correlation_id = %correlation_id,
            action = "task_graph.issue",
            error = %err,
            "failed to record issue mutation in audit journal",
        );
    }
    (axum::http::StatusCode::OK, Json(response)).into_response()
}

async fn task_graph_sub_issue_handler(
    State(state): State<TaskGraphMutationState>,
    Json(mut request): Json<TaskGraphSubIssueRequest>,
) -> Response {
    let Some(client) = state.linear_mutations.clone() else {
        return mutation_client_unavailable();
    };
    let correlation_id = ensure_correlation_id(&request.correlation_id);
    let op = request.op.clone();
    request.correlation_id = correlation_id.clone();
    let journal = state.journal.clone();
    let result = client
        .create_or_update_sub_issue(request, &correlation_id)
        .await;
    let response = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                correlation_id = %correlation_id,
                action = "task_graph.sub_issue",
                error = ?err,
                "rejected task-graph sub-issue mutation",
            );
            let receipt = ActionReceipt::rejected(
                Uuid::new_v4().to_string(),
                correlation_id.clone(),
                ActionKind::TaskGraphSubIssue,
                err.as_reason(),
            );
            let status = status_for_mutation_error(&err);
            return (status, Json(empty_sub_issue_response(receipt))).into_response();
        }
    };
    let sub_issue_id = response.sub_issue_id.clone().unwrap_or_default();
    let sub_issue_identifier = response.sub_issue_identifier.clone();
    let parent_identifier = response.parent_identifier.clone();
    let event_op = match op {
        SubIssueOp::Create => MutationOp::Created,
        SubIssueOp::Update => MutationOp::Updated,
    };
    if let Err(err) = append_mutation_event_with_op(
        &journal,
        &correlation_id,
        event_op,
        ActionKind::TaskGraphSubIssue,
        EntityRef {
            kind: entity_kind_for(ActionKind::TaskGraphSubIssue),
            id: sub_issue_id.clone(),
            identifier: sub_issue_identifier.clone(),
        },
        serde_json::json!({
            "sub_issue_id": sub_issue_id,
            "sub_issue_identifier": sub_issue_identifier,
            "parent_identifier": parent_identifier,
        }),
    )
    .await
    {
        tracing::warn!(
            correlation_id = %correlation_id,
            action = "task_graph.sub_issue",
            error = %err,
            "failed to record sub-issue mutation in audit journal",
        );
    }
    (axum::http::StatusCode::OK, Json(response)).into_response()
}

async fn task_graph_relation_handler(
    State(state): State<TaskGraphMutationState>,
    Json(mut request): Json<TaskGraphRelationRequest>,
) -> Response {
    let Some(client) = state.linear_mutations.clone() else {
        return mutation_client_unavailable();
    };
    let correlation_id = ensure_correlation_id(&request.correlation_id);
    request.correlation_id = correlation_id.clone();
    // Capture the "from" issue id up front: `EntityKind` has no `Relation`
    // variant, so the journal entity ref must identify the issue the relation
    // originated from so downstream filters "events for issue X" include it.
    let from_issue_id = request.issue_id.clone();
    let relation_type = request.relation_type.clone();
    let journal = state.journal.clone();
    let result = client.create_issue_relation(request, &correlation_id).await;
    let response = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                correlation_id = %correlation_id,
                action = "task_graph.relation",
                error = ?err,
                "rejected task-graph relation mutation",
            );
            let receipt = ActionReceipt::rejected(
                Uuid::new_v4().to_string(),
                correlation_id.clone(),
                ActionKind::TaskGraphRelation,
                err.as_reason(),
            );
            let status = status_for_mutation_error(&err);
            return (status, Json(empty_relation_response(receipt))).into_response();
        }
    };
    let relation_id = response.relation_id.clone().unwrap_or_default();
    let related_issue_id = response.related_issue_id.clone().unwrap_or_default();
    let response_relation_type = response
        .relation_type
        .clone()
        .unwrap_or_else(|| relation_type.clone());
    if let Err(err) = append_mutation_event(
        &journal,
        &correlation_id,
        ActionKind::TaskGraphRelation,
        EntityRef {
            kind: entity_kind_for(ActionKind::TaskGraphRelation),
            id: from_issue_id.clone(),
            identifier: Some(response_relation_type.clone()),
        },
        serde_json::json!({
            "relation_id": relation_id,
            "related_issue_id": related_issue_id,
            "relation_type": response_relation_type,
            "from_issue_id": from_issue_id,
        }),
    )
    .await
    {
        tracing::warn!(
            correlation_id = %correlation_id,
            action = "task_graph.relation",
            error = %err,
            "failed to record relation mutation in audit journal",
        );
    }
    (axum::http::StatusCode::OK, Json(response)).into_response()
}

async fn task_graph_evidence_handler(
    State(state): State<TaskGraphMutationState>,
    Json(mut request): Json<TaskGraphEvidenceRequest>,
) -> Response {
    let Some(client) = state.linear_mutations.clone() else {
        return mutation_client_unavailable();
    };
    let correlation_id = ensure_correlation_id(&request.correlation_id);
    request.correlation_id = correlation_id.clone();
    let journal = state.journal.clone();
    let result = client
        .create_evidence_comment(request, &correlation_id)
        .await;
    let response = match result {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(
                correlation_id = %correlation_id,
                action = "task_graph.evidence",
                error = ?err,
                "rejected task-graph evidence mutation",
            );
            let receipt = ActionReceipt::rejected(
                Uuid::new_v4().to_string(),
                correlation_id.clone(),
                ActionKind::TaskGraphEvidence,
                err.as_reason(),
            );
            let status = status_for_mutation_error(&err);
            return (status, Json(empty_evidence_response(receipt))).into_response();
        }
    };
    let comment_id = response.comment_id.clone().unwrap_or_default();
    let issue_id = response.issue_id.clone().unwrap_or_default();
    let issue_identifier = response.issue_identifier.clone();

    // Build the audit event directly so we can carry both `comment_id` AND
    // the real `issue_id` (the shared helper stamped `issue_id = ""` for
    // evidence, which lost correlation). The `entity_ref.id` also uses the
    // issue id so journal queries of the form "events for issue X" include
    // this comment — `comment_id` is preserved in the payload and the
    // `EventKind` variant so consumers can still pivot to it.
    let event_kind = EventKind::TaskGraphCommentCreated {
        comment_id: comment_id.clone(),
        issue_id: issue_id.clone(),
    };
    let entity_ref = EntityRef {
        kind: entity_kind_for(ActionKind::TaskGraphEvidence),
        id: issue_id.clone(),
        identifier: issue_identifier
            .clone()
            .or_else(|| Some(comment_id.clone())),
    };
    let payload = serde_json::json!({
        "comment_id": comment_id,
        "issue_id": issue_id,
        "issue_identifier": issue_identifier,
    });
    let record =
        build_audit_event_inner("gateway", &correlation_id, entity_ref, event_kind, payload);
    if let Err(err) = journal.append(record).await {
        tracing::warn!(
            correlation_id = %correlation_id,
            action = "task_graph.evidence",
            error = ?err,
            "failed to append TaskGraphCommentCreated event to journal",
        );
    }
    (axum::http::StatusCode::OK, Json(response)).into_response()
}
