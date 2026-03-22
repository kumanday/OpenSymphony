use std::time::Duration;

use opensymphony_domain::{TrackerIssue, TrackerIssueStateSnapshot};
use reqwest::{
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER},
    Client, StatusCode,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::time::sleep;
use tracing::debug;

use crate::error::{GraphqlError, LinearError};
use crate::graphql::{
    GraphqlEnvelope, GraphqlErrorPayload, IssueInverseRelationsData,
    IssueInverseRelationsVariables, IssueStatesByIdsData, IssueStatesByIdsVariables,
    IssuesByStateData, IssuesByStateVariables, LinearIssueNode, LinearRelationConnection,
    ISSUES_BY_STATE_QUERY, ISSUE_INVERSE_RELATIONS_QUERY, ISSUE_STATES_BY_IDS_QUERY,
};
use crate::normalize::{normalize_issue, normalize_issue_state};

const DEFAULT_BASE_URL: &str = "https://api.linear.app/graphql";
const DEFAULT_PAGE_SIZE: usize = 50;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LinearConfig {
    pub api_key: String,
    pub base_url: String,
    pub project_slug: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub page_size: usize,
    pub request_timeout: Duration,
    pub retry_policy: RetryPolicy,
}

impl LinearConfig {
    pub fn new(api_key: impl Into<String>, project_slug: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            project_slug: project_slug.into(),
            active_states: Vec::new(),
            terminal_states: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct LinearClient {
    http: Client,
    config: LinearConfig,
}

impl LinearClient {
    pub fn new(mut config: LinearConfig) -> Result<Self, LinearError> {
        if config.base_url.trim().is_empty() {
            config.base_url = DEFAULT_BASE_URL.to_string();
        }
        if config.page_size == 0 {
            config.page_size = DEFAULT_PAGE_SIZE;
        }
        if config.request_timeout.is_zero() {
            config.request_timeout = DEFAULT_REQUEST_TIMEOUT;
        }
        if config.retry_policy.max_attempts == 0 {
            config.retry_policy.max_attempts = 1;
        }
        if config.retry_policy.initial_backoff.is_zero() {
            config.retry_policy.initial_backoff = Duration::from_millis(1);
        }
        if config.retry_policy.max_backoff < config.retry_policy.initial_backoff {
            config.retry_policy.max_backoff = config.retry_policy.initial_backoff;
        }

        let http = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|error| LinearError::InvalidConfiguration(error.to_string()))?;

        Ok(Self { http, config })
    }

    pub async fn candidate_issues(&self) -> Result<Vec<TrackerIssue>, LinearError> {
        self.issues_by_state_names(&self.config.active_states).await
    }

    pub async fn terminal_issues(&self) -> Result<Vec<TrackerIssue>, LinearError> {
        self.issues_by_state_names(&self.config.terminal_states)
            .await
    }

    pub async fn issues_by_state_names<S>(
        &self,
        state_names: &[S],
    ) -> Result<Vec<TrackerIssue>, LinearError>
    where
        S: AsRef<str>,
    {
        let state_names = normalize_strings(state_names);
        if state_names.is_empty() {
            return Ok(Vec::new());
        }

        let mut after = None;
        let mut issues = Vec::new();

        loop {
            let variables = IssuesByStateVariables {
                project_slug: self.config.project_slug.clone(),
                state_names: state_names.clone(),
                first: self.config.page_size,
                after: after.clone(),
            };
            let response: IssuesByStateData = self
                .execute_graphql(ISSUES_BY_STATE_QUERY, json!(variables))
                .await?;

            let page_info = response.issues.page_info;
            for node in response.issues.nodes {
                issues.push(normalize_issue(self.expand_inverse_relations(node).await?)?);
            }

            if !page_info.has_next_page {
                return Ok(issues);
            }

            after = Some(page_info.end_cursor.ok_or_else(|| {
                LinearError::InvalidResponse(
                    "Linear issues page indicated a next page without an end cursor".to_string(),
                )
            })?);
        }
    }

    pub async fn issue_states_by_ids<S>(
        &self,
        issue_ids: &[S],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, LinearError>
    where
        S: AsRef<str>,
    {
        let issue_ids = normalize_strings(issue_ids);
        if issue_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut after = None;
        let mut snapshots = Vec::new();

        loop {
            let variables = IssueStatesByIdsVariables {
                issue_ids: issue_ids.clone(),
                first: self.config.page_size,
                after: after.clone(),
            };
            let response: IssueStatesByIdsData = self
                .execute_graphql(ISSUE_STATES_BY_IDS_QUERY, json!(variables))
                .await?;

            let page_info = response.issues.page_info;
            for node in response.issues.nodes {
                snapshots.push(normalize_issue_state(node));
            }

            if !page_info.has_next_page {
                return ensure_complete_issue_states(&issue_ids, snapshots);
            }

            after = Some(page_info.end_cursor.ok_or_else(|| {
                LinearError::InvalidResponse(
                    "Linear issue-state page indicated a next page without an end cursor"
                        .to_string(),
                )
            })?);
        }
    }

    async fn expand_inverse_relations(
        &self,
        mut issue: LinearIssueNode,
    ) -> Result<LinearIssueNode, LinearError> {
        issue.inverse_relations = self
            .load_all_inverse_relations(&issue.id, issue.inverse_relations)
            .await?;
        Ok(issue)
    }

    async fn load_all_inverse_relations(
        &self,
        issue_id: &str,
        mut connection: LinearRelationConnection,
    ) -> Result<LinearRelationConnection, LinearError> {
        let mut after = connection.page_info.end_cursor.clone();

        while connection.page_info.has_next_page {
            let cursor = after.clone().ok_or_else(|| {
                LinearError::InvalidResponse(format!(
                    "Linear inverseRelations page for issue {issue_id} indicated a next page without an end cursor"
                ))
            })?;
            let variables = IssueInverseRelationsVariables {
                issue_id: issue_id.to_string(),
                first: self.config.page_size,
                after: Some(cursor),
            };
            let response: IssueInverseRelationsData = self
                .execute_graphql(ISSUE_INVERSE_RELATIONS_QUERY, json!(variables))
                .await?;
            let issue = response.issue.ok_or_else(|| LinearError::MissingIssueIds {
                issue_ids: vec![issue_id.to_string()],
            })?;
            if issue.id != issue_id {
                return Err(LinearError::InvalidResponse(format!(
                    "Linear inverseRelations page returned mismatched issue ID {} for {}",
                    issue.id, issue_id
                )));
            }

            connection.nodes.extend(issue.inverse_relations.nodes);
            connection.page_info = issue.inverse_relations.page_info;
            after = connection.page_info.end_cursor.clone();
        }

        Ok(connection)
    }

    async fn execute_graphql<T>(
        &self,
        query: &'static str,
        variables: Value,
    ) -> Result<T, LinearError>
    where
        T: DeserializeOwned,
    {
        let body = json!({
            "query": query,
            "variables": variables,
        });
        let authorization = format!("Bearer {}", self.config.api_key);
        let mut attempt = 1;

        loop {
            let response = self
                .http
                .post(&self.config.base_url)
                .header(AUTHORIZATION, authorization.as_str())
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json")
                .json(&body)
                .send()
                .await;

            match response {
                Ok(response) => {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers().get(RETRY_AFTER));
                    let payload = response
                        .text()
                        .await
                        .map_err(|error| LinearError::Request(Box::new(error)))?;

                    if !status.is_success() {
                        let error = LinearError::HttpStatus {
                            status,
                            body: payload,
                            retry_after,
                        };
                        if self.should_retry(&error, attempt) {
                            self.sleep_before_retry(&error, attempt).await;
                            attempt += 1;
                            continue;
                        }
                        return Err(error);
                    }

                    let envelope: GraphqlEnvelope<T> =
                        serde_json::from_str(&payload).map_err(|error| {
                            LinearError::InvalidResponse(format!(
                                "failed to decode Linear GraphQL response: {error}"
                            ))
                        })?;

                    if let Some(errors) = envelope.errors {
                        let error =
                            LinearError::from_graphql_errors(convert_graphql_errors(errors));
                        if self.should_retry(&error, attempt) {
                            self.sleep_before_retry(&error, attempt).await;
                            attempt += 1;
                            continue;
                        }
                        return Err(error);
                    }

                    return envelope.data.ok_or_else(|| {
                        LinearError::InvalidResponse(
                            "Linear GraphQL response omitted both data and errors".to_string(),
                        )
                    });
                }
                Err(error) => {
                    let error = LinearError::Request(Box::new(error));
                    if self.should_retry(&error, attempt) {
                        self.sleep_before_retry(&error, attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    fn should_retry(&self, error: &LinearError, attempt: usize) -> bool {
        if attempt >= self.config.retry_policy.max_attempts {
            return false;
        }

        match error {
            LinearError::Request(_) => true,
            LinearError::HttpStatus { status, .. } => {
                *status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            LinearError::Graphql { .. } => error.is_rate_limited(),
            LinearError::MissingIssueIds { .. }
            | LinearError::InvalidConfiguration(_)
            | LinearError::InvalidResponse(_) => false,
        }
    }

    async fn sleep_before_retry(&self, error: &LinearError, attempt: usize) {
        let delay = error
            .retry_after()
            .unwrap_or_else(|| self.exponential_backoff(attempt));
        debug!(
            attempt,
            delay_ms = delay.as_millis(),
            category = ?error.category(),
            "retrying Linear GraphQL request"
        );
        sleep(delay).await;
    }

    fn exponential_backoff(&self, attempt: usize) -> Duration {
        let mut delay = self.config.retry_policy.initial_backoff;
        for _ in 1..attempt {
            match delay.checked_mul(2) {
                Some(next) if next <= self.config.retry_policy.max_backoff => delay = next,
                _ => return self.config.retry_policy.max_backoff,
            }
        }
        delay
    }
}

fn convert_graphql_errors(errors: Vec<GraphqlErrorPayload>) -> Vec<GraphqlError> {
    errors
        .into_iter()
        .map(|error| GraphqlError {
            message: error.message,
            code: error.extensions.and_then(|extensions| extensions.code),
        })
        .collect()
}

fn normalize_strings<S>(values: &[S]) -> Vec<String>
where
    S: AsRef<str>,
{
    let mut normalized = Vec::new();
    for value in values {
        let value = value.as_ref().trim();
        if value.is_empty() {
            continue;
        }
        if !normalized.iter().any(|existing| existing == value) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn ensure_complete_issue_states(
    requested_issue_ids: &[String],
    snapshots: Vec<TrackerIssueStateSnapshot>,
) -> Result<Vec<TrackerIssueStateSnapshot>, LinearError> {
    let mut missing_issue_ids = Vec::new();
    for issue_id in requested_issue_ids {
        if !snapshots.iter().any(|snapshot| snapshot.id == *issue_id) {
            missing_issue_ids.push(issue_id.clone());
        }
    }

    if missing_issue_ids.is_empty() {
        Ok(snapshots)
    } else {
        Err(LinearError::MissingIssueIds {
            issue_ids: missing_issue_ids,
        })
    }
}

fn parse_retry_after(header_value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let seconds = header_value?.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
}
