use async_trait::async_trait;
use chrono::{DateTime, Utc};
use opensymphony_domain::{
    compare_issues, Issue, IssueBlocker, IssueStateSnapshot, IssueTracker, TrackerError,
};
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

const QUERY_CANDIDATE_ISSUES: &str = r#"
query CandidateIssues($projectSlug: String!, $states: [String!]!, $first: Int!, $after: String) {
  issues(
    filter: { project: { slug: { eq: $projectSlug } }, state: { name: { in: $states } } }
    first: $first
    after: $after
  ) {
    nodes {
      id
      identifier
      title
      description
      priority
      createdAt
      updatedAt
      state { name }
      labels { nodes { name } }
      blockedByIssues { nodes { id identifier state { name } } }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

const QUERY_ISSUE_STATES: &str = r#"
query IssueStates($issueIds: [String!]!) {
  issues(filter: { id: { in: $issueIds } }) {
    nodes {
      id
      identifier
      state { name }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

const QUERY_TERMINAL_ISSUES: &str = r#"
query TerminalIssues($projectSlug: String!, $states: [String!]!, $first: Int!, $after: String) {
  issues(
    filter: { project: { slug: { eq: $projectSlug } }, state: { name: { in: $states } } }
    first: $first
    after: $after
  ) {
    nodes {
      id
      identifier
      title
      description
      priority
      createdAt
      updatedAt
      state { name }
      labels { nodes { name } }
      blockedByIssues { nodes { id identifier state { name } } }
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearConfig {
    pub project_slug: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub page_size: usize,
    pub max_retries: u32,
}

impl Default for LinearConfig {
    fn default() -> Self {
        Self {
            project_slug: String::new(),
            active_states: vec!["Todo".to_string(), "In Progress".to_string()],
            terminal_states: vec!["Done".to_string(), "Cancelled".to_string()],
            page_size: 50,
            max_retries: 2,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum LinearError {
    #[error("linear auth error: {0}")]
    Auth(String),
    #[error("linear rate limited: {0}")]
    RateLimited(String),
    #[error("linear transport error: {0}")]
    Transport(String),
    #[error("linear timeout: {0}")]
    Timeout(String),
    #[error("linear invalid response: {0}")]
    InvalidResponse(String),
    #[error("linear not found: {0}")]
    NotFound(String),
    #[error("linear invalid state transition: {0}")]
    InvalidStateTransition(String),
    #[error("linear permission denied: {0}")]
    PermissionDenied(String),
}

impl From<LinearError> for TrackerError {
    fn from(value: LinearError) -> Self {
        match value {
            LinearError::Auth(message) => TrackerError::Auth(message),
            LinearError::RateLimited(message) => TrackerError::RateLimited(message),
            LinearError::Transport(message) => TrackerError::Transport(message),
            LinearError::Timeout(message) => TrackerError::Timeout(message),
            LinearError::InvalidResponse(message) => TrackerError::InvalidResponse(message),
            LinearError::NotFound(message) => TrackerError::NotFound(message),
            LinearError::InvalidStateTransition(message) => TrackerError::InvalidResponse(message),
            LinearError::PermissionDenied(message) => TrackerError::PermissionDenied(message),
        }
    }
}

#[async_trait]
pub trait LinearTransport: Send + Sync {
    async fn execute(&self, query: &str, variables: Value) -> Result<Value, LinearError>;
}

pub trait LinearWriteOperations: Send + Sync {
    fn get_issue(&self, query: &str) -> Result<Issue, LinearError>;
    fn comment_issue(&self, issue_id: &str, body: &str) -> Result<Issue, LinearError>;
    fn transition_issue(&self, issue_id: &str, state_name: &str) -> Result<Issue, LinearError>;
    fn link_pr(&self, issue_id: &str, url: &str, title: Option<&str>)
        -> Result<Issue, LinearError>;
    fn list_project_states(&self, project_slug: &str) -> Result<Vec<String>, LinearError>;
}

#[derive(Debug, Clone)]
pub struct LinearAdapter<T> {
    transport: T,
    config: LinearConfig,
}

impl<T> LinearAdapter<T> {
    pub fn new(transport: T, config: LinearConfig) -> Self {
        Self { transport, config }
    }
}

impl<T> LinearAdapter<T>
where
    T: LinearTransport,
{
    pub async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, LinearError> {
        let mut issues = self
            .fetch_issue_pages(QUERY_CANDIDATE_ISSUES, &self.config.active_states)
            .await?;
        issues.sort_by(compare_issues);
        Ok(issues)
    }

    pub async fn fetch_terminal_issues(&self) -> Result<Vec<Issue>, LinearError> {
        self.fetch_issue_pages(QUERY_TERMINAL_ISSUES, &self.config.terminal_states)
            .await
    }

    pub async fn fetch_states_by_issue_ids(
        &self,
        issue_ids: &[String],
    ) -> Result<Vec<IssueStateSnapshot>, LinearError> {
        if issue_ids.is_empty() {
            return Ok(vec![]);
        }

        let response = self
            .execute_with_retries(QUERY_ISSUE_STATES, json!({ "issueIds": issue_ids }))
            .await?;
        let page = serde_json::from_value::<LinearIssuesResponse>(response)
            .map_err(|error| LinearError::InvalidResponse(error.to_string()))?;
        Ok(page
            .data
            .issues
            .nodes
            .into_iter()
            .map(|issue| IssueStateSnapshot {
                id: issue.id,
                identifier: issue.identifier,
                state: issue.state.name.clone(),
                is_active: self.config.active_states.contains(&issue.state.name),
                is_terminal: self.config.terminal_states.contains(&issue.state.name),
            })
            .collect())
    }

    async fn fetch_issue_pages(
        &self,
        query: &str,
        states: &[String],
    ) -> Result<Vec<Issue>, LinearError> {
        let mut cursor = None::<String>;
        let mut issues = Vec::new();

        loop {
            let response = self
                .execute_with_retries(
                    query,
                    json!({
                        "projectSlug": self.config.project_slug,
                        "states": states,
                        "first": self.config.page_size,
                        "after": cursor,
                    }),
                )
                .await?;
            let page = serde_json::from_value::<LinearIssuesResponse>(response)
                .map_err(|error| LinearError::InvalidResponse(error.to_string()))?;

            issues.extend(
                page.data
                    .issues
                    .nodes
                    .into_iter()
                    .map(|issue| normalize_issue(issue, &self.config.terminal_states)),
            );

            if !page.data.issues.page_info.has_next_page {
                return Ok(issues);
            }
            cursor = page.data.issues.page_info.end_cursor;
        }
    }

    async fn execute_with_retries(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<Value, LinearError> {
        let mut attempt = 0;
        loop {
            match self.transport.execute(query, variables.clone()).await {
                Ok(response) => return Ok(response),
                Err(error @ LinearError::RateLimited(_))
                | Err(error @ LinearError::Transport(_))
                | Err(error @ LinearError::Timeout(_)) => {
                    if attempt >= self.config.max_retries {
                        return Err(error);
                    }
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[async_trait]
impl<T> IssueTracker for LinearAdapter<T>
where
    T: LinearTransport + Send + Sync,
{
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, TrackerError> {
        Self::fetch_candidate_issues(self).await.map_err(Into::into)
    }

    async fn fetch_states_by_issue_ids(
        &self,
        issue_ids: &[String],
    ) -> Result<Vec<IssueStateSnapshot>, TrackerError> {
        Self::fetch_states_by_issue_ids(self, issue_ids)
            .await
            .map_err(Into::into)
    }

    async fn fetch_terminal_issues(&self) -> Result<Vec<Issue>, TrackerError> {
        Self::fetch_terminal_issues(self).await.map_err(Into::into)
    }
}

#[derive(Debug, Deserialize)]
struct LinearIssuesResponse {
    data: LinearIssuesData,
}

#[derive(Debug, Deserialize)]
struct LinearIssuesData {
    issues: LinearIssueConnection,
}

#[derive(Debug, Deserialize)]
struct LinearIssueConnection {
    #[serde(default)]
    nodes: Vec<LinearIssueNode>,
    #[serde(rename = "pageInfo")]
    page_info: LinearPageInfo,
}

#[derive(Debug, Deserialize)]
struct LinearPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LinearIssueNode {
    id: String,
    identifier: String,
    #[serde(default)]
    title: String,
    description: Option<String>,
    priority: Option<u8>,
    #[serde(rename = "createdAt", default = "epoch")]
    created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt", default = "epoch")]
    updated_at: DateTime<Utc>,
    state: LinearStateNode,
    #[serde(default)]
    labels: LinearLabelsConnection,
    #[serde(rename = "blockedByIssues", default)]
    blocked_by_issues: LinearBlockerConnection,
}

#[derive(Debug, Deserialize)]
struct LinearStateNode {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct LinearLabelsConnection {
    #[serde(default)]
    nodes: Vec<LinearLabelNode>,
}

#[derive(Debug, Deserialize)]
struct LinearLabelNode {
    name: String,
}

#[derive(Debug, Deserialize, Default)]
struct LinearBlockerConnection {
    #[serde(default)]
    nodes: Vec<LinearBlockerNode>,
}

#[derive(Debug, Deserialize)]
struct LinearBlockerNode {
    id: String,
    identifier: String,
    state: LinearStateNode,
}

fn normalize_issue(issue: LinearIssueNode, terminal_states: &[String]) -> Issue {
    Issue {
        id: issue.id,
        identifier: issue.identifier,
        title: issue.title,
        description: issue.description,
        priority: issue.priority,
        state: issue.state.name.clone(),
        labels: issue
            .labels
            .nodes
            .into_iter()
            .map(|label| label.name)
            .collect(),
        blocked_by: issue
            .blocked_by_issues
            .nodes
            .into_iter()
            .map(|blocker| IssueBlocker {
                id: blocker.id,
                identifier: blocker.identifier,
                is_terminal: terminal_states.contains(&blocker.state.name),
                state: blocker.state.name,
            })
            .collect(),
        created_at: issue.created_at,
        updated_at: issue.updated_at,
    }
}

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeTransport {
        responses: Mutex<HashMap<String, VecDeque<Result<Value, LinearError>>>>,
    }

    impl FakeTransport {
        fn push(&self, operation_name: &str, response: Result<Value, LinearError>) {
            self.responses
                .lock()
                .expect("lock should succeed")
                .entry(operation_name.to_string())
                .or_default()
                .push_back(response);
        }
    }

    #[async_trait]
    impl LinearTransport for FakeTransport {
        async fn execute(&self, query: &str, _variables: Value) -> Result<Value, LinearError> {
            let operation = if query.contains("CandidateIssues") {
                "CandidateIssues"
            } else if query.contains("TerminalIssues") {
                "TerminalIssues"
            } else {
                "IssueStates"
            };

            self.responses
                .lock()
                .expect("lock should succeed")
                .get_mut(operation)
                .and_then(|responses| responses.pop_front())
                .unwrap_or_else(|| Err(LinearError::Transport("missing fake response".to_string())))
        }
    }

    fn issue_node(
        id: &str,
        identifier: &str,
        state: &str,
        priority: Option<u8>,
        created_at: &str,
    ) -> Value {
        json!({
            "id": id,
            "identifier": identifier,
            "title": format!("Issue {identifier}"),
            "description": "Fixture issue",
            "priority": priority,
            "createdAt": created_at,
            "updatedAt": created_at,
            "state": { "name": state },
            "labels": { "nodes": [{ "name": "fixture" }] },
            "blockedByIssues": { "nodes": [] }
        })
    }

    #[tokio::test]
    async fn paginates_and_sorts_candidates() {
        let transport = FakeTransport::default();
        transport.push(
            "CandidateIssues",
            Ok(json!({
                "data": {
                    "issues": {
                        "nodes": [issue_node("2", "ABC-2", "Todo", Some(3), "2026-03-21T20:00:02Z")],
                        "pageInfo": { "hasNextPage": true, "endCursor": "page-2" }
                    }
                }
            })),
        );
        transport.push(
            "CandidateIssues",
            Ok(json!({
                "data": {
                    "issues": {
                        "nodes": [issue_node("1", "ABC-1", "Todo", Some(1), "2026-03-21T20:00:01Z")],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            })),
        );
        let adapter = LinearAdapter::new(
            transport,
            LinearConfig {
                project_slug: "demo".to_string(),
                page_size: 1,
                ..LinearConfig::default()
            },
        );

        let issues = adapter
            .fetch_candidate_issues()
            .await
            .expect("fetch should succeed");
        let identifiers = issues
            .into_iter()
            .map(|issue| issue.identifier)
            .collect::<Vec<_>>();
        assert_eq!(identifiers, vec!["ABC-1", "ABC-2"]);
    }

    #[tokio::test]
    async fn maps_blockers_and_optional_fields() {
        let transport = FakeTransport::default();
        transport.push(
            "CandidateIssues",
            Ok(json!({
                "data": {
                    "issues": {
                        "nodes": [{
                            "id": "1",
                            "identifier": "ABC-1",
                            "title": "Blocked issue",
                            "description": null,
                            "priority": null,
                            "createdAt": "2026-03-21T20:00:00Z",
                            "updatedAt": "2026-03-21T20:00:00Z",
                            "state": { "name": "Todo" },
                            "labels": { "nodes": [] },
                            "blockedByIssues": {
                                "nodes": [{ "id": "2", "identifier": "ABC-2", "state": { "name": "Cancelled" } }]
                            }
                        }],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            })),
        );
        let adapter = LinearAdapter::new(
            transport,
            LinearConfig {
                project_slug: "demo".to_string(),
                terminal_states: vec!["Cancelled".to_string()],
                ..LinearConfig::default()
            },
        );

        let issues = adapter
            .fetch_candidate_issues()
            .await
            .expect("fetch should succeed");
        assert_eq!(issues.len(), 1);
        assert!(issues[0].blocked_by[0].is_terminal);
    }

    #[tokio::test]
    async fn retries_transient_transport_errors() {
        let transport = FakeTransport::default();
        transport.push(
            "IssueStates",
            Err(LinearError::Transport("temporary".to_string())),
        );
        transport.push(
            "IssueStates",
            Ok(json!({
                "data": {
                    "issues": {
                        "nodes": [{
                            "id": "1",
                            "identifier": "ABC-1",
                            "state": { "name": "Todo" }
                        }],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            })),
        );
        let adapter = LinearAdapter::new(
            transport,
            LinearConfig {
                max_retries: 1,
                ..LinearConfig::default()
            },
        );

        let states = adapter
            .fetch_states_by_issue_ids(&["1".to_string()])
            .await
            .expect("state refresh should retry and succeed");
        assert_eq!(states[0].identifier, "ABC-1");
    }

    #[tokio::test]
    async fn rejects_invalid_responses() {
        let transport = FakeTransport::default();
        transport.push("CandidateIssues", Ok(json!({ "data": {} })));
        let adapter = LinearAdapter::new(transport, LinearConfig::default());

        let error = adapter
            .fetch_candidate_issues()
            .await
            .expect_err("invalid payload should fail");
        assert!(matches!(error, LinearError::InvalidResponse(_)));
    }
}
