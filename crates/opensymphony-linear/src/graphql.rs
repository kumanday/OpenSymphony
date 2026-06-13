use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(super) const ISSUES_BY_STATE_QUERY: &str = r#"
query IssuesByState($projectSlug: String!, $stateNames: [String!], $includeArchived: Boolean!, $first: Int!, $after: String, $relationFirst: Int!, $labelFirst: Int!) {
  issues(
    filter: {
      project: { slugId: { eq: $projectSlug } }
      state: { name: { in: $stateNames } }
    }
    includeArchived: $includeArchived
    first: $first
    after: $after
  ) {
    nodes {
      id
      identifier
      url
      title
      description
      priority
      createdAt
      updatedAt
      state {
        id
        name
        type
      }
      parent {
        id
        identifier
        url
        title
        state {
          name
        }
      }
      projectMilestone {
        id
        name
      }
      children(includeArchived: true, first: 100) {
        nodes {
          id
          identifier
          url
          title
          state {
            name
          }
        }
      }
      labels(first: $labelFirst) {
        nodes {
          name
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
      inverseRelations(first: $relationFirst) {
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
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

pub(super) const ISSUE_LABELS_QUERY: &str = r#"
query IssueLabelsPage($issueId: String!, $first: Int!, $after: String) {
  issue(id: $issueId) {
    id
    labels(first: $first, after: $after) {
      nodes {
        name
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"#;

pub(super) const ISSUE_INVERSE_RELATIONS_QUERY: &str = r#"
query IssueInverseRelationsPage($issueId: String!, $first: Int!, $after: String) {
  issue(id: $issueId) {
    id
    inverseRelations(first: $first, after: $after) {
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
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"#;

pub(super) const ISSUE_STATES_BY_IDS_QUERY: &str = r#"
query IssueStatesByIds($projectSlug: String!, $issueIds: [ID!], $first: Int!, $after: String) {
  issues(
    filter: {
      id: { in: $issueIds }
      project: { slugId: { eq: $projectSlug } }
    }
    includeArchived: true
    first: $first
    after: $after
  ) {
    nodes {
      id
      identifier
      updatedAt
      state {
        id
        name
        type
      }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

pub(super) const ISSUE_BY_IDENTIFIER_QUERY: &str = r#"
query IssueByIdentifier($identifier: String!, $relationFirst: Int!, $labelFirst: Int!) {
  issue(id: $identifier) {
    id
    identifier
    url
    title
    description
    priority
    createdAt
    updatedAt
    state {
      id
      name
      type
    }
    parent {
      id
      identifier
      url
      title
      state {
        name
      }
    }
    projectMilestone {
      id
      name
    }
    children(includeArchived: true, first: 100) {
      nodes {
        id
        identifier
        url
        title
        state {
          name
        }
      }
    }
    labels(first: $labelFirst) {
      nodes {
        name
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
    inverseRelations(first: $relationFirst) {
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
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"#;

pub(super) const ISSUE_COMMENTS_QUERY: &str = r#"
query IssueCommentsPage($issueId: String!, $first: Int!, $after: String) {
  issue(id: $issueId) {
    id
    comments(first: $first, after: $after) {
      nodes {
        id
        body
        updatedAt
        resolvedAt
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
"#;

pub(super) const ISSUE_ARCHIVE_MUTATION: &str = r#"
mutation IssueArchive($id: String!, $trash: Boolean) {
  issueArchive(id: $id, trash: $trash) {
    success
  }
}
"#;

pub(super) const PROJECT_MILESTONE_CREATE_MUTATION: &str = r#"
mutation ProjectMilestoneCreate($input: ProjectMilestoneCreateInput!) {
  projectMilestoneCreate(input: $input) {
    success
    projectMilestone {
      id
      name
      description
      targetDate
      sortOrder
      project {
        id
        name
        slugId
      }
    }
  }
}
"#;

pub(super) const PROJECT_MILESTONE_UPDATE_MUTATION: &str = r#"
mutation ProjectMilestoneUpdate($id: String!, $input: ProjectMilestoneUpdateInput!) {
  projectMilestoneUpdate(id: $id, input: $input) {
    success
    projectMilestone {
      id
      name
      description
      targetDate
      sortOrder
      project {
        id
        name
        slugId
      }
    }
  }
}
"#;

pub(super) const ISSUE_CREATE_MUTATION: &str = r#"
mutation IssueCreate($input: IssueCreateInput!) {
  issueCreate(input: $input) {
    success
    issue {
      id
      identifier
      url
      title
      description
      priority
      estimate
      createdAt
      updatedAt
      state {
        id
        name
        type
      }
      project {
        id
        name
        slugId
      }
      projectMilestone {
        id
        name
      }
      parent {
        id
        identifier
      }
      assignee {
        id
        name
        email
      }
      labels(first: 25) {
        nodes {
          id
          name
        }
      }
    }
  }
}
"#;

pub(super) const ISSUE_UPDATE_MUTATION: &str = r#"
mutation IssueUpdate($id: String!, $input: IssueUpdateInput!) {
  issueUpdate(id: $id, input: $input) {
    success
    issue {
      id
      identifier
      url
      title
      description
      priority
      estimate
      createdAt
      updatedAt
      state {
        id
        name
        type
      }
      project {
        id
        name
        slugId
      }
      projectMilestone {
        id
        name
      }
      parent {
        id
        identifier
      }
      assignee {
        id
        name
        email
      }
      labels(first: 25) {
        nodes {
          id
          name
        }
      }
    }
  }
}
"#;

pub(super) const COMMENT_CREATE_MUTATION: &str = r#"
mutation CommentCreate($input: CommentCreateInput!) {
  commentCreate(input: $input) {
    success
    comment {
      id
      body
      url
      createdAt
      updatedAt
      issue {
        id
        identifier
      }
    }
  }
}
"#;

pub(super) const ISSUE_RELATION_CREATE_MUTATION: &str = r#"
mutation IssueRelationCreate($input: IssueRelationCreateInput!) {
  issueRelationCreate(input: $input) {
    success
    issueRelation {
      id
      type
      issue {
        id
        identifier
      }
      relatedIssue {
        id
        identifier
      }
    }
  }
}
"#;

pub(super) const PROJECT_BY_SLUG_QUERY: &str = r#"
query ProjectBySlug($slug: String!) {
  projects(filter: { slugId: { eq: $slug } }, first: 1) {
    nodes {
      id
      name
      slugId
      url
      content
    }
  }
}
"#;

pub(super) const PROJECT_UPDATE_CONTENT_MUTATION: &str = r#"
mutation UpdateProjectContent($id: String!, $content: String!) {
  projectUpdate(id: $id, input: { content: $content }) {
    success
    project {
      id
      name
      slugId
      content
      updatedAt
    }
  }
}
"#;

#[derive(Debug, Deserialize)]
pub(super) struct GraphqlEnvelope<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphqlErrorPayload>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GraphqlErrorPayload {
    pub message: String,
    pub extensions: Option<GraphqlErrorExtensions>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GraphqlErrorExtensions {
    pub code: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssuesByStateVariables {
    pub project_slug: String,
    pub state_names: Vec<String>,
    pub include_archived: bool,
    pub first: usize,
    pub after: Option<String>,
    pub relation_first: usize,
    pub label_first: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueStatesByIdsVariables {
    pub project_slug: String,
    pub issue_ids: Vec<String>,
    pub first: usize,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueByIdentifierVariables {
    pub identifier: String,
    pub relation_first: usize,
    pub label_first: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueInverseRelationsVariables {
    pub issue_id: String,
    pub first: usize,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueLabelsVariables {
    pub issue_id: String,
    pub first: usize,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueCommentsVariables {
    pub issue_id: String,
    pub first: usize,
    pub after: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueArchiveVariables {
    pub id: String,
    pub trash: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectMilestoneCreateVariables {
    pub input: ProjectMilestoneCreateInput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectMilestoneUpdateVariables {
    pub id: String,
    pub input: ProjectMilestoneUpdateInput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueCreateVariables {
    pub input: IssueCreateInput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueUpdateVariables {
    pub id: String,
    pub input: IssueUpdateInput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentCreateVariables {
    pub input: CommentCreateInput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueRelationCreateVariables {
    pub input: IssueRelationCreateInput,
}

#[derive(Debug, Serialize)]
pub(super) struct ProjectBySlugVariables {
    pub slug: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ProjectUpdateContentVariables {
    pub id: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssuesByStateData {
    pub issues: IssuesConnection<LinearIssueNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueStatesByIdsData {
    pub issues: IssuesConnection<LinearIssueStateNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueByIdentifierData {
    pub issue: Option<LinearIssueNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueInverseRelationsData {
    pub issue: Option<LinearIssueRelationsNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueLabelsData {
    pub issue: Option<LinearIssueLabelsNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueCommentsData {
    pub issue: Option<LinearIssueCommentsNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueArchiveData {
    pub issue_archive: IssueArchivePayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProjectBySlugData {
    pub projects: ProjectsConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectUpdateContentData {
    pub project_update: ProjectUpdatePayload,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueArchivePayload {
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProjectUpdatePayload {
    pub success: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProjectsConnection {
    pub nodes: Vec<LinearProjectNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearProjectNode {
    pub id: String,
    pub name: String,
    pub slug_id: String,
    pub url: String,
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssuesConnection<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct PageInfo {
    #[serde(rename = "hasNextPage")]
    pub has_next_page: bool,
    #[serde(rename = "endCursor")]
    pub end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearIssueNode {
    pub id: String,
    pub identifier: String,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state: LinearWorkflowState,
    #[serde(default)]
    pub parent: Option<LinearParentNode>,
    #[serde(default)]
    pub project_milestone: Option<LinearProjectMilestoneNode>,
    #[serde(default)]
    pub children: LinearChildConnection,
    pub labels: LinearLabelConnection,
    pub inverse_relations: LinearRelationConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearIssueRelationsNode {
    pub id: String,
    pub inverse_relations: LinearRelationConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearIssueLabelsNode {
    pub id: String,
    pub labels: LinearLabelConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearIssueCommentsNode {
    pub id: String,
    pub comments: LinearCommentConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearIssueStateNode {
    pub id: String,
    pub identifier: String,
    pub updated_at: DateTime<Utc>,
    pub state: LinearWorkflowState,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearWorkflowState {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearParentNode {
    pub id: String,
    #[serde(default)]
    pub identifier: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub state: Option<LinearIssueRefState>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearProjectMilestoneNode {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct LinearChildConnection {
    pub nodes: Vec<LinearChildNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearChildNode {
    pub id: String,
    pub identifier: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub state: LinearIssueRefState,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearIssueRefState {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearLabelConnection {
    pub nodes: Vec<LinearLabelNode>,
    #[serde(default, rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearLabelNode {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearCommentConnection {
    pub nodes: Vec<LinearCommentNode>,
    #[serde(default, rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearCommentNode {
    pub id: String,
    pub body: String,
    pub updated_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearRelationConnection {
    pub nodes: Vec<LinearRelationNode>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearRelationNode {
    #[serde(rename = "type")]
    pub relation_type: String,
    pub issue: LinearBlockerNode,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearBlockerNode {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: LinearWorkflowState,
}

// =============================================================================
// Mutation input DTOs (Linear-native names).
//
// These structs are serialized directly into GraphQL variables. Everything that
// is optional comes through as `null` (not omitted) so the Linear schema's
// "argument requires non-null" behavior stays intact.
// =============================================================================

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMilestoneCreateInput {
    pub project_id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMilestoneUpdateInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueCreateInput {
    pub team_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_milestone_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueUpdateInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_milestone_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentCreateInput {
    pub issue_id: String,
    pub body: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueRelationCreateInput {
    pub issue_id: String,
    pub related_issue_id: String,
    #[serde(rename = "type")]
    pub relation_type: String,
}

// =============================================================================
// Mutation response DTOs.
// =============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectMilestoneCreateData {
    pub project_milestone_create: ProjectMilestoneMutationPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectMilestoneUpdateData {
    pub project_milestone_update: ProjectMilestoneMutationPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueCreateData {
    pub issue_create: IssueMutationPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueUpdateData {
    pub issue_update: IssueMutationPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentCreateData {
    pub comment_create: CommentMutationPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueRelationCreateData {
    pub issue_relation_create: IssueRelationMutationPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectMilestoneMutationPayload {
    pub success: bool,
    #[serde(default)]
    pub project_milestone: Option<ProjectMilestoneMutationNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectMilestoneMutationNode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub target_date: Option<String>,
    #[serde(default)]
    pub sort_order: Option<f64>,
    pub project: LinearProjectNode,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueMutationPayload {
    pub success: bool,
    #[serde(default)]
    pub issue: Option<IssueMutationNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueMutationNode {
    pub id: String,
    pub identifier: String,
    #[serde(default)]
    pub url: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<f64>,
    #[serde(default)]
    pub estimate: Option<f64>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub state: LinearWorkflowState,
    #[serde(default)]
    pub project: Option<LinearProjectNode>,
    #[serde(default)]
    pub project_milestone: Option<LinearProjectMilestoneNode>,
    #[serde(default)]
    pub parent: Option<LinearParentRefNode>,
    #[serde(default)]
    pub assignee: Option<LinearAssigneeNode>,
    #[serde(default, rename = "labels")]
    pub labels: LinearLabelMutationConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearParentRefNode {
    pub id: String,
    pub identifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LinearAssigneeNode {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(super) struct LinearLabelMutationConnection {
    pub nodes: Vec<LinearLabelRefNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearLabelRefNode {
    #[serde(default)]
    #[allow(dead_code)]
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentMutationPayload {
    pub success: bool,
    #[serde(default)]
    pub comment: Option<CommentMutationNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CommentMutationNode {
    pub id: String,
    pub body: String,
    #[serde(default)]
    pub url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub issue: IssueRefNode,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssueRefNode {
    pub id: String,
    pub identifier: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueRelationMutationPayload {
    pub success: bool,
    #[serde(default)]
    pub issue_relation: Option<IssueRelationMutationNode>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IssueRelationMutationNode {
    pub id: String,
    #[serde(rename = "type")]
    pub relation_type: String,
    pub issue: IssueRefNode,
    pub related_issue: IssueRefNode,
}
