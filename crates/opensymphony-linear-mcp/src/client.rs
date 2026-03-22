use std::{env, time::Duration};

use opensymphony_domain::TrackerIssueStateKind;
use reqwest::{
    Client,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    error::{GraphqlError, LinearMcpError},
    model::{
        AttachmentSnapshot, CommentSnapshot, IssueBlockerSnapshot, IssueSnapshot, TeamSummary,
        WorkflowStateSummary,
    },
};

const DEFAULT_BASE_URL: &str = "https://api.linear.app/graphql";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const RESOLVE_ISSUE_BY_ID_QUERY: &str = r#"
query ResolveIssueById($id: String!) {
  issue(id: $id) {
    id
    identifier
    url
    title
    description
    priority
    state {
      id
      name
      type
    }
    team {
      id
      key
      name
    }
    labels(first: 20) {
      nodes {
        name
      }
    }
    inverseRelations(first: 20) {
      nodes {
        type
        issue {
          id
          identifier
          title
          state {
            id
            name
            type
          }
        }
      }
    }
    createdAt
    updatedAt
  }
}
"#;

const RESOLVE_ISSUE_BY_IDENTIFIER_QUERY: &str = r#"
query ResolveIssueByIdentifier($teamKey: String!, $issueNumber: Float!) {
  issues(
    filter: { team: { key: { eq: $teamKey } }, number: { eq: $issueNumber } }
    first: 1
  ) {
    nodes {
      id
      identifier
      url
      title
      description
      priority
      state {
        id
        name
        type
      }
      team {
        id
        key
        name
      }
      labels(first: 20) {
        nodes {
          name
        }
      }
      inverseRelations(first: 20) {
        nodes {
          type
          issue {
            id
            identifier
            title
            state {
              id
              name
              type
            }
          }
        }
      }
      createdAt
      updatedAt
    }
  }
}
"#;

const WORKFLOW_STATES_BY_TEAM_ID_QUERY: &str = r#"
query WorkflowStatesByTeamId($teamId: String!) {
  workflowStates(filter: { team: { id: { eq: $teamId } } }, first: 50) {
    nodes {
      id
      name
      type
      position
      team {
        id
        key
        name
      }
    }
  }
}
"#;

const WORKFLOW_STATES_BY_TEAM_KEY_QUERY: &str = r#"
query WorkflowStatesByTeamKey($teamKey: String!) {
  workflowStates(filter: { team: { key: { eq: $teamKey } } }, first: 50) {
    nodes {
      id
      name
      type
      position
      team {
        id
        key
        name
      }
    }
  }
}
"#;

const COMMENT_CREATE_MUTATION: &str = r#"
mutation CommentIssue($issueId: String!, $body: String!) {
  commentCreate(input: { issueId: $issueId, body: $body }) {
    success
    comment {
      id
      body
      url
    }
  }
}
"#;

const TRANSITION_ISSUE_MUTATION: &str = r#"
mutation TransitionIssue($id: String!, $stateId: String!) {
  issueUpdate(id: $id, input: { stateId: $stateId }) {
    success
    issue {
      id
      identifier
      url
      title
      description
      priority
      state {
        id
        name
        type
      }
      team {
        id
        key
        name
      }
      labels(first: 20) {
        nodes {
          name
        }
      }
      inverseRelations(first: 20) {
        nodes {
          type
          issue {
            id
            identifier
            title
            state {
              id
              name
              type
            }
          }
        }
      }
      createdAt
      updatedAt
    }
  }
}
"#;

const ATTACHMENT_LINK_URL_MUTATION: &str = r#"
mutation LinkIssueUrl($issueId: String!, $url: String!, $title: String) {
  attachmentLinkURL(issueId: $issueId, url: $url, title: $title) {
    success
    attachment {
      id
      title
      url
    }
  }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamSelector {
    Id(String),
    Key(String),
}

#[derive(Clone)]
pub struct LinearMcpClient {
    http: Client,
    base_url: String,
    api_key: String,
}

impl LinearMcpClient {
    pub fn from_env() -> Result<Self, LinearMcpError> {
        let api_key = env::var("LINEAR_API_KEY").map_err(|_| {
            LinearMcpError::InvalidConfiguration("LINEAR_API_KEY is required".to_string())
        })?;
        if api_key.trim().is_empty() {
            return Err(LinearMcpError::InvalidConfiguration(
                "LINEAR_API_KEY must not be blank".to_string(),
            ));
        }

        let base_url = env::var("LINEAR_BASE_URL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

        let http = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .map_err(|error| LinearMcpError::InvalidConfiguration(error.to_string()))?;

        Ok(Self {
            http,
            base_url,
            api_key,
        })
    }

    pub async fn resolve_issue(&self, issue: &str) -> Result<IssueSnapshot, LinearMcpError> {
        let issue = normalize_required_string("issue", issue)?;
        if Uuid::parse_str(&issue).is_ok() {
            let data: ResolveIssueByIdData = self
                .execute_graphql(RESOLVE_ISSUE_BY_ID_QUERY, json!({ "id": issue }))
                .await?;
            let node = data
                .issue
                .ok_or_else(|| LinearMcpError::NotFound(format!("Issue `{issue}` not found.")))?;
            return normalize_issue(node);
        }

        let (team_key, issue_number) = parse_issue_identifier(&issue)?;
        let data: ResolveIssueByIdentifierData = self
            .execute_graphql(
                RESOLVE_ISSUE_BY_IDENTIFIER_QUERY,
                json!({
                    "teamKey": team_key,
                    "issueNumber": issue_number as f64,
                }),
            )
            .await?;
        let node = data
            .issues
            .nodes
            .into_iter()
            .next()
            .ok_or_else(|| LinearMcpError::NotFound(format!("Issue `{issue}` not found.")))?;
        normalize_issue(node)
    }

    pub async fn workflow_states_for_issue(
        &self,
        issue: &IssueSnapshot,
    ) -> Result<(TeamSummary, Vec<WorkflowStateSummary>), LinearMcpError> {
        self.workflow_states_for_team(TeamSelector::Id(issue.team.id.clone()))
            .await
    }

    pub async fn workflow_states_for_team(
        &self,
        selector: TeamSelector,
    ) -> Result<(TeamSummary, Vec<WorkflowStateSummary>), LinearMcpError> {
        let data: WorkflowStatesData = match &selector {
            TeamSelector::Id(team_id) => {
                self.execute_graphql(
                    WORKFLOW_STATES_BY_TEAM_ID_QUERY,
                    json!({ "teamId": normalize_required_string("team", team_id)? }),
                )
                .await?
            }
            TeamSelector::Key(team_key) => {
                self.execute_graphql(
                    WORKFLOW_STATES_BY_TEAM_KEY_QUERY,
                    json!({ "teamKey": normalize_required_string("team", team_key)? }),
                )
                .await?
            }
        };

        normalize_states(data.workflow_states.nodes, &selector)
    }

    pub async fn comment_issue(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<CommentSnapshot, LinearMcpError> {
        let data: CommentCreateData = self
            .execute_graphql(
                COMMENT_CREATE_MUTATION,
                json!({
                    "issueId": normalize_required_string("issue", issue_id)?,
                    "body": normalize_required_string("body", body)?,
                }),
            )
            .await?;
        let payload = data.comment_create;
        if !payload.success {
            return Err(LinearMcpError::InvalidResponse(
                "commentCreate returned success=false".to_string(),
            ));
        }
        let comment = payload.comment.ok_or_else(|| {
            LinearMcpError::InvalidResponse(
                "commentCreate returned success=true without a comment".to_string(),
            )
        })?;
        Ok(CommentSnapshot {
            id: comment.id,
            body: comment.body,
            url: comment.url,
        })
    }

    pub async fn transition_issue(
        &self,
        issue_id: &str,
        state_id: &str,
    ) -> Result<IssueSnapshot, LinearMcpError> {
        let data: IssueUpdateData = self
            .execute_graphql(
                TRANSITION_ISSUE_MUTATION,
                json!({
                    "id": normalize_required_string("issue", issue_id)?,
                    "stateId": normalize_required_string("state", state_id)?,
                }),
            )
            .await?;
        let payload = data.issue_update;
        if !payload.success {
            return Err(LinearMcpError::InvalidResponse(
                "issueUpdate returned success=false".to_string(),
            ));
        }
        let issue = payload.issue.ok_or_else(|| {
            LinearMcpError::InvalidResponse(
                "issueUpdate returned success=true without an issue".to_string(),
            )
        })?;
        normalize_issue(issue)
    }

    pub async fn link_issue_url(
        &self,
        issue_id: &str,
        url: &str,
        title: Option<&str>,
    ) -> Result<AttachmentSnapshot, LinearMcpError> {
        let data: AttachmentLinkData = self
            .execute_graphql(
                ATTACHMENT_LINK_URL_MUTATION,
                json!({
                    "issueId": normalize_required_string("issue", issue_id)?,
                    "url": normalize_required_string("url", url)?,
                    "title": normalized_optional_string(title),
                }),
            )
            .await?;
        let payload = data.attachment_link_url;
        if !payload.success {
            return Err(LinearMcpError::InvalidResponse(
                "attachmentLinkURL returned success=false".to_string(),
            ));
        }
        let attachment = payload.attachment.ok_or_else(|| {
            LinearMcpError::InvalidResponse(
                "attachmentLinkURL returned success=true without an attachment".to_string(),
            )
        })?;
        Ok(AttachmentSnapshot {
            id: attachment.id,
            title: attachment.title,
            url: attachment.url,
        })
    }

    async fn execute_graphql<T>(&self, query: &str, variables: Value) -> Result<T, LinearMcpError>
    where
        T: DeserializeOwned,
    {
        let response = self
            .http
            .post(&self.base_url)
            .header(AUTHORIZATION, &self.api_key)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(|error| LinearMcpError::Request(Box::new(error)))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| LinearMcpError::Request(Box::new(error)))?;

        if !status.is_success() {
            if let Ok(envelope) = serde_json::from_str::<GraphqlEnvelope<Value>>(&body)
                && let Some(errors) = envelope.errors
            {
                return Err(LinearMcpError::from_graphql_errors(
                    errors.into_iter().map(GraphqlError::from).collect(),
                ));
            }

            return Err(LinearMcpError::HttpStatus { status, body });
        }

        let envelope: GraphqlEnvelope<T> = serde_json::from_str(&body).map_err(|error| {
            LinearMcpError::InvalidResponse(format!("failed to decode GraphQL response: {error}"))
        })?;

        if let Some(errors) = envelope.errors {
            return Err(LinearMcpError::from_graphql_errors(
                errors.into_iter().map(GraphqlError::from).collect(),
            ));
        }

        envelope.data.ok_or_else(|| {
            LinearMcpError::InvalidResponse(
                "GraphQL response did not include data or errors".to_string(),
            )
        })
    }
}

fn normalize_required_string(field_name: &str, value: &str) -> Result<String, LinearMcpError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(LinearMcpError::InvalidResponse(format!(
            "{field_name} must not be blank"
        )));
    }
    Ok(normalized.to_string())
}

fn normalized_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_issue_identifier(issue: &str) -> Result<(String, u64), LinearMcpError> {
    let (team_key, raw_number) = issue.split_once('-').ok_or_else(|| {
        LinearMcpError::InvalidResponse(format!(
            "issue `{issue}` must be a UUID or identifier like `COE-267`"
        ))
    })?;

    let number = raw_number.parse::<u64>().map_err(|_| {
        LinearMcpError::InvalidResponse(format!(
            "issue `{issue}` must be a UUID or identifier like `COE-267`"
        ))
    })?;

    Ok((team_key.trim().to_string(), number))
}

fn normalize_issue(node: GraphqlIssue) -> Result<IssueSnapshot, LinearMcpError> {
    Ok(IssueSnapshot {
        id: node.id,
        identifier: node.identifier,
        url: node.url,
        title: node.title,
        description: node.description,
        priority: normalize_priority(node.priority)?,
        state: normalize_state(node.state),
        labels: normalize_labels(node.labels.nodes),
        blocked_by: normalize_blockers(node.inverse_relations.nodes),
        team: TeamSummary {
            id: node.team.id,
            key: node.team.key,
            name: node.team.name,
        },
        created_at: node.created_at,
        updated_at: node.updated_at,
    })
}

fn normalize_states(
    nodes: Vec<GraphqlWorkflowStateNode>,
    selector: &TeamSelector,
) -> Result<(TeamSummary, Vec<WorkflowStateSummary>), LinearMcpError> {
    if nodes.is_empty() {
        let team = match selector {
            TeamSelector::Id(value) | TeamSelector::Key(value) => value,
        };
        return Err(LinearMcpError::NotFound(format!(
            "No workflow states were found for team `{team}`."
        )));
    }

    let mut nodes = nodes;
    nodes.sort_by(|left, right| {
        left.position
            .partial_cmp(&right.position)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
    });

    let team = nodes
        .first()
        .map(|node| TeamSummary {
            id: node.team.id.clone(),
            key: node.team.key.clone(),
            name: node.team.name.clone(),
        })
        .ok_or_else(|| {
            LinearMcpError::InvalidResponse("workflow state list unexpectedly empty".to_string())
        })?;

    let states = nodes
        .into_iter()
        .map(|node| {
            normalize_state(GraphqlWorkflowState {
                id: node.id,
                name: node.name,
                tracker_type: node.tracker_type,
            })
        })
        .collect();

    Ok((team, states))
}

fn normalize_state(state: GraphqlWorkflowState) -> WorkflowStateSummary {
    WorkflowStateSummary {
        id: state.id,
        name: state.name,
        tracker_type: state.tracker_type.clone(),
        kind: TrackerIssueStateKind::from_tracker_type(&state.tracker_type),
    }
}

fn normalize_labels(labels: Vec<GraphqlLabelNode>) -> Vec<String> {
    let mut labels = labels
        .into_iter()
        .map(|label| label.name)
        .collect::<Vec<_>>();
    labels.sort_unstable();
    labels.dedup();
    labels
}

fn normalize_blockers(relations: Vec<GraphqlRelationNode>) -> Vec<IssueBlockerSnapshot> {
    let mut blockers = relations
        .into_iter()
        .filter(|relation| relation.relation_type == "blocks")
        .map(|relation| IssueBlockerSnapshot {
            id: relation.issue.id,
            identifier: relation.issue.identifier,
            title: relation.issue.title,
            state: normalize_state(relation.issue.state),
        })
        .collect::<Vec<_>>();
    blockers.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    blockers.dedup_by(|left, right| left.id == right.id);
    blockers
}

const LINEAR_MAX_PRIORITY: u64 = 4;

fn normalize_priority(priority: f64) -> Result<Option<u8>, LinearMcpError> {
    if !priority.is_finite() || priority < 0.0 {
        return Err(LinearMcpError::InvalidResponse(format!(
            "Linear priority must be a finite non-negative number, got {priority}"
        )));
    }

    let rounded = priority.trunc();
    if (priority - rounded).abs() > f64::EPSILON {
        return Err(LinearMcpError::InvalidResponse(format!(
            "Linear priority must be an integer value, got {priority}"
        )));
    }

    match rounded as u64 {
        0 => Ok(None),
        value if value <= LINEAR_MAX_PRIORITY => Ok(Some(value as u8)),
        value => Err(LinearMcpError::InvalidResponse(format!(
            "Linear priority must be between 0 and {LINEAR_MAX_PRIORITY}, got {value}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct GraphqlEnvelope<T> {
    data: Option<T>,
    errors: Option<Vec<GraphqlErrorPayload>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlErrorPayload {
    message: String,
    extensions: Option<GraphqlErrorExtensions>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlErrorExtensions {
    code: Option<String>,
    user_presentable_message: Option<String>,
}

impl From<GraphqlErrorPayload> for GraphqlError {
    fn from(value: GraphqlErrorPayload) -> Self {
        Self {
            message: value.message,
            code: value
                .extensions
                .as_ref()
                .and_then(|extensions| extensions.code.clone()),
            user_presentable_message: value
                .extensions
                .and_then(|extensions| extensions.user_presentable_message),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResolveIssueByIdData {
    issue: Option<GraphqlIssue>,
}

#[derive(Debug, Deserialize)]
struct ResolveIssueByIdentifierData {
    issues: GraphqlIssueConnection,
}

#[derive(Debug, Deserialize)]
struct WorkflowStatesData {
    #[serde(rename = "workflowStates")]
    workflow_states: GraphqlWorkflowStateConnection,
}

#[derive(Debug, Deserialize)]
struct CommentCreateData {
    #[serde(rename = "commentCreate")]
    comment_create: CommentCreatePayload,
}

#[derive(Debug, Deserialize)]
struct IssueUpdateData {
    #[serde(rename = "issueUpdate")]
    issue_update: IssueUpdatePayload,
}

#[derive(Debug, Deserialize)]
struct AttachmentLinkData {
    #[serde(rename = "attachmentLinkURL")]
    attachment_link_url: AttachmentLinkPayload,
}

#[derive(Debug, Deserialize)]
struct GraphqlIssueConnection {
    nodes: Vec<GraphqlIssue>,
}

#[derive(Debug, Deserialize)]
struct GraphqlWorkflowStateConnection {
    nodes: Vec<GraphqlWorkflowStateNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlIssue {
    id: String,
    identifier: String,
    url: String,
    title: String,
    description: Option<String>,
    priority: f64,
    state: GraphqlWorkflowState,
    team: GraphqlTeam,
    labels: GraphqlLabelConnection,
    inverse_relations: GraphqlRelationConnection,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphqlWorkflowStateNode {
    id: String,
    name: String,
    #[serde(rename = "type")]
    tracker_type: String,
    position: f64,
    team: GraphqlTeam,
}

#[derive(Debug, Deserialize)]
struct GraphqlTeam {
    id: String,
    key: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlLabelConnection {
    nodes: Vec<GraphqlLabelNode>,
}

#[derive(Debug, Deserialize)]
struct GraphqlRelationConnection {
    nodes: Vec<GraphqlRelationNode>,
}

#[derive(Debug, Deserialize)]
struct GraphqlLabelNode {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlRelationNode {
    #[serde(rename = "type")]
    relation_type: String,
    issue: GraphqlBlockerIssue,
}

#[derive(Debug, Deserialize)]
struct GraphqlBlockerIssue {
    id: String,
    identifier: String,
    title: String,
    state: GraphqlWorkflowState,
}

#[derive(Debug, Deserialize)]
struct GraphqlWorkflowState {
    id: String,
    name: String,
    #[serde(rename = "type")]
    tracker_type: String,
}

#[derive(Debug, Deserialize)]
struct CommentCreatePayload {
    success: bool,
    comment: Option<GraphqlComment>,
}

#[derive(Debug, Deserialize)]
struct GraphqlComment {
    id: String,
    body: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct IssueUpdatePayload {
    success: bool,
    issue: Option<GraphqlIssue>,
}

#[derive(Debug, Deserialize)]
struct AttachmentLinkPayload {
    success: bool,
    attachment: Option<GraphqlAttachment>,
}

#[derive(Debug, Deserialize)]
struct GraphqlAttachment {
    id: String,
    title: Option<String>,
    url: String,
}
