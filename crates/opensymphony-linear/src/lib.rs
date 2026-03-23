use async_trait::async_trait;
use chrono::{DateTime, Utc};
use opensymphony_domain::{
    compare_issues, Issue, IssueBlocker, IssueStateSnapshot, IssueTracker, TrackerError,
};
use reqwest::{
    blocking::Client as BlockingClient, header::AUTHORIZATION, Client as AsyncClient, StatusCode,
};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::env;
use std::time::Duration;
use thiserror::Error;

pub const DEFAULT_LINEAR_API_URL: &str = "https://api.linear.app/graphql";
const DEFAULT_LINEAR_TIMEOUT_MS: u64 = 30_000;
const WORKFLOW_STATE_PAGE_SIZE: usize = 100;

const QUERY_CANDIDATE_ISSUES: &str = r#"
query CandidateIssues($projectSlug: String!, $states: [String!]!, $first: Int!, $after: String) {
  issues(
    filter: { project: { slugId: { eq: $projectSlug } }, state: { name: { in: $states } } }
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
      inverseRelations(first: 100) {
        nodes {
          type
          issue { id identifier state { name } }
          relatedIssue { id }
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

const QUERY_ISSUE_STATES: &str = r#"
query IssueStates($issueIds: [ID!]!, $first: Int!, $after: String) {
  issues(
    filter: { id: { in: $issueIds } }
    first: $first
    after: $after
    includeArchived: true
  ) {
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
    filter: { project: { slugId: { eq: $projectSlug } }, state: { name: { in: $states } } }
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
      inverseRelations(first: 100) {
        nodes {
          type
          issue { id identifier state { name } }
          relatedIssue { id }
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

const QUERY_ISSUE_BY_ID: &str = r#"
query IssueById($id: ID!) {
  issues(filter: { id: { eq: $id } }, first: 1) {
    nodes {
      id
      identifier
      title
      description
      priority
      createdAt
      updatedAt
      state { id name }
      labels { nodes { name } }
      team { id key name }
      project { id name slugId }
    }
  }
}
"#;

const QUERY_ISSUE_BY_IDENTIFIER: &str = r#"
query IssueByIdentifier($teamKey: String!, $number: Float!) {
  issues(
    filter: { number: { eq: $number }, team: { key: { eq: $teamKey } } }
    first: 1
  ) {
    nodes {
      id
      identifier
      title
      description
      priority
      createdAt
      updatedAt
      state { id name }
      labels { nodes { name } }
      team { id key name }
      project { id name slugId }
    }
  }
}
"#;

const QUERY_PROJECT_BY_SLUG: &str = r#"
query ProjectBySlug($projectSlug: String!) {
  projects(filter: { slugId: { eq: $projectSlug } }, first: 1) {
    nodes {
      id
      name
      slugId
      teams {
        nodes {
          id
          key
          name
        }
      }
    }
  }
}
"#;

const QUERY_WORKFLOW_STATES_FOR_TEAM: &str = r#"
query WorkflowStatesForTeam($teamId: ID!, $first: Int!, $after: String) {
  workflowStates(
    filter: { team: { id: { eq: $teamId } } }
    first: $first
    after: $after
  ) {
    nodes {
      id
      name
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"#;

const MUTATION_COMMENT_ISSUE: &str = r#"
mutation CommentIssue($issueId: String!, $body: String!) {
  commentCreate(input: { issueId: $issueId, body: $body }) {
    success
    comment {
      issue {
        id
        identifier
        title
        description
        priority
        createdAt
        updatedAt
        state { id name }
        labels { nodes { name } }
      }
    }
  }
}
"#;

const MUTATION_TRANSITION_ISSUE: &str = r#"
mutation TransitionIssue($id: String!, $stateId: String!) {
  issueUpdate(id: $id, input: { stateId: $stateId }) {
    success
    issue {
      id
      identifier
      title
      description
      priority
      createdAt
      updatedAt
      state { id name }
      labels { nodes { name } }
    }
  }
}
"#;

const MUTATION_LINK_PR: &str = r#"
mutation LinkPr($issueId: String!, $url: String!, $title: String) {
  attachmentLinkURL(issueId: $issueId, url: $url, title: $title) {
    success
    attachment {
      issue {
        id
        identifier
        title
        description
        priority
        createdAt
        updatedAt
        state { id name }
        labels { nodes { name } }
      }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearApiConfig {
    pub base_url: String,
    pub api_key: String,
    pub timeout_ms: u64,
}

impl LinearApiConfig {
    pub fn from_env() -> Result<Self, LinearError> {
        let api_key = env::var("LINEAR_API_KEY").map_err(|_| {
            LinearError::Auth("LINEAR_API_KEY environment variable is required".to_string())
        })?;
        Ok(Self {
            base_url: env::var("OPENSYMPHONY_LINEAR_API_URL")
                .unwrap_or_else(|_| DEFAULT_LINEAR_API_URL.to_string()),
            api_key,
            timeout_ms: DEFAULT_LINEAR_TIMEOUT_MS,
        })
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
pub struct ReqwestLinearTransport {
    client: AsyncClient,
    config: LinearApiConfig,
}

impl ReqwestLinearTransport {
    pub fn new(config: LinearApiConfig) -> Result<Self, LinearError> {
        let client = AsyncClient::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|error| LinearError::Transport(error.to_string()))?;
        Ok(Self { client, config })
    }

    pub fn from_env() -> Result<Self, LinearError> {
        Self::new(LinearApiConfig::from_env()?)
    }
}

#[async_trait]
impl LinearTransport for ReqwestLinearTransport {
    async fn execute(&self, query: &str, variables: Value) -> Result<Value, LinearError> {
        let response = self
            .client
            .post(&self.config.base_url)
            .header(
                AUTHORIZATION,
                personal_api_key_authorization(&self.config.api_key),
            )
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await
            .map_err(classify_reqwest_error)?;
        let status = response.status();
        let body = response.text().await.map_err(classify_reqwest_error)?;
        decode_graphql_value(status, &body)
    }
}

#[derive(Debug, Clone)]
pub struct LinearGraphqlWriteClient {
    client: BlockingClient,
    config: LinearApiConfig,
}

impl LinearGraphqlWriteClient {
    pub fn new(config: LinearApiConfig) -> Result<Self, LinearError> {
        let client = BlockingClient::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|error| LinearError::Transport(error.to_string()))?;
        Ok(Self { client, config })
    }

    pub fn from_env() -> Result<Self, LinearError> {
        Self::new(LinearApiConfig::from_env()?)
    }

    fn execute_graphql<T>(&self, query: &str, variables: Value) -> Result<T, LinearError>
    where
        T: DeserializeOwned,
    {
        let response = self
            .client
            .post(&self.config.base_url)
            .header(
                AUTHORIZATION,
                personal_api_key_authorization(&self.config.api_key),
            )
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .map_err(classify_reqwest_error)?;
        let status = response.status();
        let body = response.text().map_err(classify_reqwest_error)?;
        extract_graphql_data(decode_graphql_value(status, &body)?)
    }

    fn fetch_issue_record(&self, query: &str) -> Result<LinearIssueNode, LinearError> {
        if let Some((team_key, number)) = parse_issue_identifier(query) {
            let response = self.execute_graphql::<LinearIssuesData>(
                QUERY_ISSUE_BY_IDENTIFIER,
                json!({ "teamKey": team_key, "number": number }),
            )?;
            return first_issue_or_not_found(response.issues.nodes, query);
        }

        let response =
            self.execute_graphql::<LinearIssuesData>(QUERY_ISSUE_BY_ID, json!({ "id": query }))?;
        first_issue_or_not_found(response.issues.nodes, query)
    }

    fn fetch_project_record(&self, project_slug: &str) -> Result<LinearProjectNode, LinearError> {
        let response = self.execute_graphql::<LinearProjectsData>(
            QUERY_PROJECT_BY_SLUG,
            json!({ "projectSlug": project_slug }),
        )?;
        response
            .projects
            .nodes
            .into_iter()
            .next()
            .ok_or_else(|| LinearError::NotFound(project_slug.to_string()))
    }

    fn fetch_team_states(
        &self,
        team_id: &str,
    ) -> Result<Vec<LinearWorkflowStateNode>, LinearError> {
        let mut cursor = None::<String>;
        let mut states = Vec::new();

        loop {
            let response = self.execute_graphql::<LinearWorkflowStatesData>(
                QUERY_WORKFLOW_STATES_FOR_TEAM,
                json!({
                    "teamId": team_id,
                    "first": WORKFLOW_STATE_PAGE_SIZE,
                    "after": cursor,
                }),
            )?;
            states.extend(response.workflow_states.nodes);

            if !response.workflow_states.page_info.has_next_page {
                return Ok(states);
            }
            cursor = response.workflow_states.page_info.end_cursor;
        }
    }
}

impl LinearWriteOperations for LinearGraphqlWriteClient {
    fn get_issue(&self, query: &str) -> Result<Issue, LinearError> {
        self.fetch_issue_record(query)
            .map(|issue| normalize_issue(issue, &[]))
    }

    fn comment_issue(&self, issue_id: &str, body: &str) -> Result<Issue, LinearError> {
        let response = self.execute_graphql::<CommentCreateData>(
            MUTATION_COMMENT_ISSUE,
            json!({ "issueId": issue_id, "body": body }),
        )?;
        let payload = response.comment_create;
        if !payload.success {
            return Err(LinearError::InvalidResponse(
                "commentCreate returned success=false".to_string(),
            ));
        }
        let issue = payload
            .comment
            .and_then(|comment| comment.issue)
            .ok_or_else(|| {
                LinearError::InvalidResponse("commentCreate response missing issue".to_string())
            })?;
        Ok(normalize_issue(issue, &[]))
    }

    fn transition_issue(&self, issue_id: &str, state_name: &str) -> Result<Issue, LinearError> {
        let issue = self.fetch_issue_record(issue_id)?;
        let team = issue.team.as_ref().ok_or_else(|| {
            LinearError::InvalidResponse("issue response missing team".to_string())
        })?;
        let state = self
            .fetch_team_states(&team.id)?
            .into_iter()
            .find(|candidate| candidate.name == state_name)
            .ok_or_else(|| LinearError::InvalidStateTransition(state_name.to_string()))?;
        let response = self.execute_graphql::<IssueUpdateData>(
            MUTATION_TRANSITION_ISSUE,
            json!({ "id": issue.id, "stateId": state.id }),
        )?;
        let payload = response.issue_update;
        if !payload.success {
            return Err(LinearError::InvalidResponse(
                "issueUpdate returned success=false".to_string(),
            ));
        }
        let issue = payload.issue.ok_or_else(|| {
            LinearError::InvalidResponse("issueUpdate response missing issue".to_string())
        })?;
        Ok(normalize_issue(issue, &[]))
    }

    fn link_pr(
        &self,
        issue_id: &str,
        url: &str,
        title: Option<&str>,
    ) -> Result<Issue, LinearError> {
        let response = self.execute_graphql::<AttachmentLinkData>(
            MUTATION_LINK_PR,
            json!({ "issueId": issue_id, "url": url, "title": title }),
        )?;
        let payload = response.attachment_link_url;
        if !payload.success {
            return Err(LinearError::InvalidResponse(
                "attachmentLinkURL returned success=false".to_string(),
            ));
        }
        let issue = payload
            .attachment
            .and_then(|attachment| attachment.issue)
            .ok_or_else(|| {
                LinearError::InvalidResponse("attachmentLinkURL response missing issue".to_string())
            })?;
        Ok(normalize_issue(issue, &[]))
    }

    fn list_project_states(&self, project_slug: &str) -> Result<Vec<String>, LinearError> {
        let project = self.fetch_project_record(project_slug)?;
        let mut states = BTreeSet::new();
        for team in project.teams.nodes {
            for state in self.fetch_team_states(&team.id)? {
                states.insert(state.name);
            }
        }
        Ok(states.into_iter().collect())
    }
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

        let mut cursor = None::<String>;
        let mut states = Vec::new();

        loop {
            let response = self
                .execute_with_retries(
                    QUERY_ISSUE_STATES,
                    json!({
                        "issueIds": issue_ids,
                        "first": self.config.page_size,
                        "after": cursor,
                    }),
                )
                .await?;
            let page = serde_json::from_value::<LinearIssuesResponse>(response)
                .map_err(|error| LinearError::InvalidResponse(error.to_string()))?;

            states.extend(
                page.data
                    .issues
                    .nodes
                    .into_iter()
                    .map(|issue| IssueStateSnapshot {
                        id: issue.id,
                        identifier: issue.identifier,
                        state: issue.state.name.clone(),
                        is_active: self.config.active_states.contains(&issue.state.name),
                        is_terminal: self.config.terminal_states.contains(&issue.state.name),
                    }),
            );

            if !page.data.issues.page_info.has_next_page {
                return Ok(states);
            }
            cursor = page.data.issues.page_info.end_cursor;
        }
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

fn decode_graphql_value(status: StatusCode, body: &str) -> Result<Value, LinearError> {
    let response = match serde_json::from_str::<Value>(body) {
        Ok(response) => response,
        Err(error) => {
            if !status.is_success() {
                return Err(classify_status_error(status, body));
            }
            return Err(LinearError::InvalidResponse(error.to_string()));
        }
    };

    if status_overrides_graphql_errors(status) {
        return Err(classify_status_error(status, body));
    }

    let errors = response
        .get("errors")
        .cloned()
        .map(serde_json::from_value::<Vec<GraphqlError>>)
        .transpose()
        .map_err(|error| LinearError::InvalidResponse(error.to_string()))?
        .unwrap_or_default();
    if !errors.is_empty() {
        return Err(classify_graphql_errors(&errors));
    }

    if !status.is_success() {
        return Err(classify_status_error(status, body));
    }

    if response.get("data").is_none() {
        return Err(LinearError::InvalidResponse(
            "linear response missing `data`".to_string(),
        ));
    }

    Ok(response)
}

fn extract_graphql_data<T>(response: Value) -> Result<T, LinearError>
where
    T: DeserializeOwned,
{
    serde_json::from_value::<GraphqlDataEnvelope<T>>(response)
        .map(|envelope| envelope.data)
        .map_err(|error| LinearError::InvalidResponse(error.to_string()))
}

fn classify_status_error(status: StatusCode, body: &str) -> LinearError {
    let detail = body.to_string();
    match status {
        StatusCode::UNAUTHORIZED => LinearError::Auth(detail),
        StatusCode::FORBIDDEN => LinearError::PermissionDenied(detail),
        StatusCode::TOO_MANY_REQUESTS => LinearError::RateLimited(detail),
        StatusCode::REQUEST_TIMEOUT | StatusCode::GATEWAY_TIMEOUT => LinearError::Timeout(detail),
        status if status.is_server_error() => LinearError::Transport(detail),
        _ => LinearError::InvalidResponse(detail),
    }
}

fn status_overrides_graphql_errors(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED
            | StatusCode::FORBIDDEN
            | StatusCode::TOO_MANY_REQUESTS
            | StatusCode::REQUEST_TIMEOUT
            | StatusCode::GATEWAY_TIMEOUT
    ) || status.is_server_error()
}

fn classify_reqwest_error(error: reqwest::Error) -> LinearError {
    if error.is_timeout() {
        LinearError::Timeout(error.to_string())
    } else {
        LinearError::Transport(error.to_string())
    }
}

fn personal_api_key_authorization(api_key: &str) -> &str {
    api_key
}

fn classify_graphql_errors(errors: &[GraphqlError]) -> LinearError {
    let detail = errors
        .iter()
        .map(graphql_error_message)
        .collect::<Vec<_>>()
        .join("; ");

    if errors.iter().any(|error| {
        matches!(
            error.extensions.code.as_deref(),
            Some("AUTHENTICATION_ERROR" | "AUTHENTICATION_REQUIRED" | "UNAUTHENTICATED")
        )
    }) {
        return LinearError::Auth(detail);
    }

    if errors.iter().any(|error| {
        matches!(
            error.extensions.code.as_deref(),
            Some("FORBIDDEN" | "PERMISSION_REQUIRED")
        )
    }) {
        return LinearError::PermissionDenied(detail);
    }

    if errors.iter().any(|error| {
        matches!(
            error.extensions.code.as_deref(),
            Some("RATELIMITED" | "RATE_LIMITED")
        )
    }) {
        return LinearError::RateLimited(detail);
    }

    if errors.iter().any(|error| {
        let message = graphql_error_message(error).to_ascii_lowercase();
        message.contains("not found")
    }) {
        return LinearError::NotFound(detail);
    }

    if errors.iter().any(|error| {
        let message = graphql_error_message(error).to_ascii_lowercase();
        message.contains("invalid state")
            || message.contains("stateid")
            || message.contains("state id")
    }) {
        return LinearError::InvalidStateTransition(detail);
    }

    if errors
        .iter()
        .any(|error| error.extensions.code.as_deref() == Some("INVALID_INPUT"))
    {
        return LinearError::InvalidResponse(detail);
    }

    LinearError::InvalidResponse(detail)
}

fn graphql_error_message(error: &GraphqlError) -> String {
    error
        .extensions
        .user_presentable_message
        .clone()
        .unwrap_or_else(|| error.message.clone())
}

fn first_issue_or_not_found(
    issues: Vec<LinearIssueNode>,
    query: &str,
) -> Result<LinearIssueNode, LinearError> {
    issues
        .into_iter()
        .next()
        .ok_or_else(|| LinearError::NotFound(query.to_string()))
}

fn parse_issue_identifier(query: &str) -> Option<(String, f64)> {
    let (team_key, number) = query.rsplit_once('-')?;
    if team_key.is_empty() {
        return None;
    }
    let number = number.parse::<u32>().ok()? as f64;
    Some((team_key.to_string(), number))
}

fn normalize_issue(issue: LinearIssueNode, terminal_states: &[String]) -> Issue {
    let blockers = normalize_blockers(
        &issue.id,
        &issue.blocked_by_issues.nodes,
        &issue.inverse_relations.nodes,
        terminal_states,
    );

    Issue {
        id: issue.id,
        identifier: issue.identifier,
        title: issue.title,
        description: issue.description,
        priority: normalize_priority(issue.priority),
        state: issue.state.name.clone(),
        labels: issue
            .labels
            .nodes
            .into_iter()
            .map(|label| label.name)
            .collect(),
        blocked_by: blockers,
        created_at: issue.created_at,
        updated_at: issue.updated_at,
    }
}

fn normalize_priority(priority: Option<f64>) -> Option<u8> {
    priority
        .and_then(|value| u8::try_from(value as i64).ok())
        .filter(|value| *value > 0)
}

fn normalize_blockers(
    issue_id: &str,
    blocked_by_issues: &[LinearBlockerNode],
    inverse_relations: &[LinearIssueRelationNode],
    terminal_states: &[String],
) -> Vec<IssueBlocker> {
    let mut seen = BTreeSet::new();
    let mut blockers = Vec::new();

    for blocker in blocked_by_issues {
        if seen.insert(blocker.id.clone()) {
            blockers.push(IssueBlocker {
                id: blocker.id.clone(),
                identifier: blocker.identifier.clone(),
                is_terminal: terminal_states.contains(&blocker.state.name),
                state: blocker.state.name.clone(),
            });
        }
    }

    for relation in inverse_relations {
        if relation.relation_type != "blocks" || relation.related_issue.id != issue_id {
            continue;
        }

        if seen.insert(relation.issue.id.clone()) {
            blockers.push(IssueBlocker {
                id: relation.issue.id.clone(),
                identifier: relation.issue.identifier.clone(),
                is_terminal: terminal_states.contains(&relation.issue.state.name),
                state: relation.issue.state.name.clone(),
            });
        }
    }

    blockers
}

fn epoch() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid")
}

#[derive(Debug, Deserialize)]
struct GraphqlDataEnvelope<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
    #[serde(default)]
    extensions: GraphqlErrorExtensions,
}

#[derive(Debug, Default, Deserialize)]
struct GraphqlErrorExtensions {
    code: Option<String>,
    #[serde(rename = "userPresentableMessage")]
    user_presentable_message: Option<String>,
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
struct LinearProjectsData {
    projects: LinearProjectConnection,
}

#[derive(Debug, Deserialize)]
struct LinearWorkflowStatesData {
    #[serde(rename = "workflowStates")]
    workflow_states: LinearWorkflowStateConnection,
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
struct CommentCreatePayload {
    success: bool,
    comment: Option<LinearCommentNode>,
}

#[derive(Debug, Deserialize)]
struct IssueUpdatePayload {
    success: bool,
    issue: Option<LinearIssueNode>,
}

#[derive(Debug, Deserialize)]
struct AttachmentLinkPayload {
    success: bool,
    attachment: Option<LinearAttachmentNode>,
}

#[derive(Debug, Deserialize)]
struct LinearCommentNode {
    issue: Option<LinearIssueNode>,
}

#[derive(Debug, Deserialize)]
struct LinearAttachmentNode {
    issue: Option<LinearIssueNode>,
}

#[derive(Debug, Deserialize)]
struct LinearIssueConnection {
    #[serde(default)]
    nodes: Vec<LinearIssueNode>,
    #[serde(rename = "pageInfo")]
    page_info: LinearPageInfo,
}

#[derive(Debug, Deserialize)]
struct LinearProjectConnection {
    #[serde(default)]
    nodes: Vec<LinearProjectNode>,
}

#[derive(Debug, Deserialize)]
struct LinearWorkflowStateConnection {
    #[serde(default)]
    nodes: Vec<LinearWorkflowStateNode>,
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
    priority: Option<f64>,
    #[serde(rename = "createdAt", default = "epoch")]
    created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt", default = "epoch")]
    updated_at: DateTime<Utc>,
    state: LinearStateNode,
    #[serde(default)]
    labels: LinearLabelsConnection,
    #[serde(rename = "blockedByIssues", default)]
    blocked_by_issues: LinearBlockerConnection,
    #[serde(rename = "inverseRelations", default)]
    inverse_relations: LinearRelationConnection,
    #[serde(default)]
    team: Option<LinearTeamNode>,
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

#[derive(Debug, Deserialize, Default)]
struct LinearRelationConnection {
    #[serde(default)]
    nodes: Vec<LinearIssueRelationNode>,
}

#[derive(Debug, Deserialize)]
struct LinearIssueRelationNode {
    #[serde(rename = "type")]
    relation_type: String,
    issue: LinearBlockerNode,
    #[serde(rename = "relatedIssue")]
    related_issue: LinearRelatedIssueNode,
}

#[derive(Debug, Deserialize)]
struct LinearRelatedIssueNode {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LinearTeamNode {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LinearProjectNode {
    #[serde(default)]
    teams: LinearTeamConnection,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LinearTeamConnection {
    #[serde(default)]
    nodes: Vec<LinearTeamNode>,
}

#[derive(Debug, Deserialize)]
struct LinearWorkflowStateNode {
    id: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::sync::{Arc, Mutex};

    type FakeResponses = HashMap<String, VecDeque<Result<Value, LinearError>>>;
    type Shared<T> = Arc<Mutex<T>>;

    #[derive(Clone, Default)]
    struct FakeTransport {
        responses: Shared<FakeResponses>,
        calls: Shared<Vec<(String, Value)>>,
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

        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().expect("lock should succeed").clone()
        }
    }

    #[async_trait]
    impl LinearTransport for FakeTransport {
        async fn execute(&self, query: &str, variables: Value) -> Result<Value, LinearError> {
            self.calls
                .lock()
                .expect("lock should succeed")
                .push((query.to_string(), variables));
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
            "inverseRelations": { "nodes": [] }
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
    async fn maps_relation_backers_and_optional_fields() {
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
                            "inverseRelations": {
                                "nodes": [{
                                    "type": "blocks",
                                    "issue": { "id": "2", "identifier": "ABC-2", "state": { "name": "Cancelled" } },
                                    "relatedIssue": { "id": "1" }
                                }]
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
    async fn paginates_state_refreshes() {
        let transport = FakeTransport::default();
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
                        "pageInfo": { "hasNextPage": true, "endCursor": "next-page" }
                    }
                }
            })),
        );
        transport.push(
            "IssueStates",
            Ok(json!({
                "data": {
                    "issues": {
                        "nodes": [{
                            "id": "2",
                            "identifier": "ABC-2",
                            "state": { "name": "Done" }
                        }],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                    }
                }
            })),
        );
        let adapter = LinearAdapter::new(
            transport.clone(),
            LinearConfig {
                page_size: 1,
                active_states: vec!["Todo".to_string()],
                terminal_states: vec!["Done".to_string()],
                ..LinearConfig::default()
            },
        );

        let states = adapter
            .fetch_states_by_issue_ids(&["1".to_string(), "2".to_string()])
            .await
            .expect("state refresh should paginate");
        let identifiers = states
            .into_iter()
            .map(|state| state.identifier)
            .collect::<Vec<_>>();
        assert_eq!(identifiers, vec!["ABC-1", "ABC-2"]);
        let calls = transport.calls();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].0.contains("includeArchived: true"));
        assert_eq!(calls[0].1["issueIds"], json!(["1", "2"]));
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

    #[test]
    fn candidate_queries_match_live_linear_schema_fields() {
        assert!(QUERY_CANDIDATE_ISSUES.contains("slugId"));
        assert!(QUERY_CANDIDATE_ISSUES.contains("inverseRelations"));
        assert!(!QUERY_CANDIDATE_ISSUES.contains("blockedByIssues"));
    }

    #[test]
    fn classifies_non_json_rate_limits_as_retryable() {
        let error = decode_graphql_value(StatusCode::TOO_MANY_REQUESTS, "slow down")
            .expect_err("429 should be retryable even without JSON");
        assert!(matches!(error, LinearError::RateLimited(detail) if detail == "slow down"));
    }

    #[test]
    fn classifies_non_json_server_errors_as_transport() {
        let error = decode_graphql_value(StatusCode::SERVICE_UNAVAILABLE, "<html>retry</html>")
            .expect_err("5xx should be retryable even without JSON");
        assert!(matches!(error, LinearError::Transport(detail) if detail == "<html>retry</html>"));
    }

    #[test]
    fn classifies_json_rate_limits_by_http_status_before_graphql_errors() {
        let error = decode_graphql_value(
            StatusCode::TOO_MANY_REQUESTS,
            r#"{"errors":[{"message":"slow down","extensions":{"code":"BAD_USER_INPUT"}}]}"#,
        )
        .expect_err("429 JSON payload should still be retryable");
        assert!(matches!(error, LinearError::RateLimited(detail) if detail.contains("slow down")));
    }

    #[test]
    fn classifies_graphql_ratelimited_errors_as_rate_limited() {
        let error = decode_graphql_value(
            StatusCode::BAD_REQUEST,
            r#"{"errors":[{"message":"slow down","extensions":{"code":"RATELIMITED"}}]}"#,
        )
        .expect_err("RATELIMITED GraphQL payload should be retryable");
        assert!(matches!(error, LinearError::RateLimited(detail) if detail.contains("slow down")));
    }

    #[test]
    fn uses_plain_authorization_header_for_personal_api_keys() {
        assert_eq!(
            personal_api_key_authorization("linear-api-key"),
            "linear-api-key"
        );
    }
}
