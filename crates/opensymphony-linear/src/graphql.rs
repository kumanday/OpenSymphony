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
pub(super) struct IssueArchivePayload {
    pub success: bool,
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
