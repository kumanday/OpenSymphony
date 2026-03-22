use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(super) const ISSUES_BY_STATE_QUERY: &str = r#"
query IssuesByState($projectSlug: String!, $stateNames: [String!], $first: Int!, $after: String, $relationFirst: Int!) {
  issues(
    filter: {
      project: { slugId: { eq: $projectSlug } }
      state: { name: { in: $stateNames } }
    }
    includeArchived: true
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
      labels {
        nodes {
          name
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
query IssueStatesByIds($projectSlug: String!, $issueIds: [String!], $first: Int!, $after: String) {
  issues(
    filter: {
      id: { in: $issueIds }
      project: { slugId: { eq: $projectSlug } }
    }
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
    pub first: usize,
    pub after: Option<String>,
    pub relation_first: usize,
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
pub(super) struct IssueInverseRelationsVariables {
    pub issue_id: String,
    pub first: usize,
    pub after: Option<String>,
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
pub(super) struct IssueInverseRelationsData {
    pub issue: Option<LinearIssueRelationsNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IssuesConnection<T> {
    pub nodes: Vec<T>,
    #[serde(rename = "pageInfo")]
    pub page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
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
pub(super) struct LinearLabelConnection {
    pub nodes: Vec<LinearLabelNode>,
}

#[derive(Debug, Deserialize)]
pub(super) struct LinearLabelNode {
    pub name: String,
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
