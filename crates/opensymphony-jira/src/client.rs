use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::opensymphony_domain::{TrackerIssue, TrackerIssueStateSnapshot, TrackerIssueSummary};
use reqwest::{
    Client, Method, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::time::sleep;
use tracing::debug;

use super::adf::document_text;
use super::error::JiraError;
use super::normalize::{
    normalize_issue, normalize_issue_state, normalize_issue_summary, parse_datetime,
};
use super::rest::{CommentsResponse, JiraIssueBean, SearchResponse};

const DEFAULT_PAGE_SIZE: usize = 50;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_INLINE_RATE_LIMIT_RETRY: Duration = Duration::from_secs(30);
const SEARCH_PATH: &str = "/rest/api/3/search/jql";
const ISSUE_FIELDS: &[&str] = &[
    "summary",
    "description",
    "status",
    "priority",
    "labels",
    "created",
    "updated",
    "project",
    "parent",
    "subtasks",
    "issuelinks",
    "fixVersions",
];
const STATE_FIELDS: &[&str] = &["status", "updated"];

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
pub struct JiraConfig {
    /// Jira site base URL, e.g. `https://acme.atlassian.net`.
    pub base_url: String,
    pub api_token: String,
    /// Atlassian account email for Jira Cloud basic auth. When absent the
    /// token is sent as a bearer token (Jira Data Center personal access
    /// tokens).
    pub auth_email: Option<String>,
    pub project_key: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub page_size: usize,
    pub request_timeout: Duration,
    pub retry_policy: RetryPolicy,
}

impl JiraConfig {
    pub fn new(
        base_url: impl Into<String>,
        api_token: impl Into<String>,
        project_key: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_token: api_token.into(),
            auth_email: None,
            project_key: project_key.into(),
            active_states: Vec::new(),
            terminal_states: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkpadComment {
    pub id: String,
    pub body: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone)]
pub struct JiraClient {
    http: Client,
    config: JiraConfig,
    authorization: String,
}

impl JiraClient {
    pub fn new(mut config: JiraConfig) -> Result<Self, JiraError> {
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
        config.base_url = normalize_required_string("tracker.endpoint", &config.base_url)?
            .trim_end_matches('/')
            .to_string();
        config.api_token = normalize_required_string("JIRA_API_TOKEN", &config.api_token)?;
        config.project_key =
            normalize_required_string("tracker.project_slug", &config.project_key)?;
        config.active_states =
            normalize_required_state_names("tracker.active_states", &config.active_states)?;
        config.terminal_states =
            normalize_required_state_names("tracker.terminal_states", &config.terminal_states)?;
        let auth_email = match config.auth_email.take() {
            Some(email) => {
                let email = email.trim().to_string();
                (!email.is_empty()).then_some(email)
            }
            None => None,
        };
        let authorization = match &auth_email {
            Some(email) => format!(
                "Basic {}",
                base64_encode(format!("{email}:{}", config.api_token).as_bytes())
            ),
            None => format!("Bearer {}", config.api_token),
        };
        config.auth_email = auth_email;

        let http = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|error| JiraError::InvalidConfiguration(error.to_string()))?;

        Ok(Self {
            http,
            config,
            authorization,
        })
    }

    pub async fn candidate_issues(&self) -> Result<Vec<TrackerIssue>, JiraError> {
        self.issues_by_state_names(&self.config.active_states).await
    }

    pub async fn candidate_issue_summaries(&self) -> Result<Vec<TrackerIssueSummary>, JiraError> {
        let beans = self
            .search_issues(
                &self.project_states_jql(&self.config.active_states),
                ISSUE_FIELDS,
            )
            .await?;
        beans
            .into_iter()
            .map(|bean| normalize_issue_summary(bean, &self.config.base_url))
            .collect()
    }

    pub async fn terminal_issues(&self) -> Result<Vec<TrackerIssue>, JiraError> {
        self.issues_by_state_names(&self.config.terminal_states)
            .await
    }

    pub async fn issues_by_state_names<S>(
        &self,
        state_names: &[S],
    ) -> Result<Vec<TrackerIssue>, JiraError>
    where
        S: AsRef<str>,
    {
        let state_names = normalize_strings(state_names);
        if state_names.is_empty() {
            return Ok(Vec::new());
        }
        let beans = self
            .search_issues(&self.project_states_jql(&state_names), ISSUE_FIELDS)
            .await?;
        beans
            .into_iter()
            .map(|bean| normalize_issue(bean, &self.config.base_url))
            .collect()
    }

    pub async fn issues_by_identifiers<S>(
        &self,
        identifiers: &[S],
    ) -> Result<Vec<TrackerIssue>, JiraError>
    where
        S: AsRef<str>,
    {
        self.issues_by_identifiers_filtered(identifiers, false)
            .await
    }

    /// Like [`Self::issues_by_identifiers`] but treats issues outside the
    /// configured project as missing, mirroring the Linear client's
    /// project-scoped lookup.
    pub async fn project_issues_by_identifiers<S>(
        &self,
        identifiers: &[S],
    ) -> Result<Vec<TrackerIssue>, JiraError>
    where
        S: AsRef<str>,
    {
        self.issues_by_identifiers_filtered(identifiers, true).await
    }

    async fn issues_by_identifiers_filtered<S>(
        &self,
        identifiers: &[S],
        enforce_project: bool,
    ) -> Result<Vec<TrackerIssue>, JiraError>
    where
        S: AsRef<str>,
    {
        let identifiers = normalize_strings(identifiers);
        if identifiers.is_empty() {
            return Ok(Vec::new());
        }

        let mut issues = Vec::new();
        let mut missing_issue_ids = Vec::new();
        for identifier in &identifiers {
            let bean = match self.fetch_issue(identifier).await {
                Ok(bean) => bean,
                Err(JiraError::HttpStatus {
                    status: StatusCode::NOT_FOUND,
                    ..
                }) => {
                    missing_issue_ids.push(identifier.clone());
                    continue;
                }
                Err(error) => return Err(error),
            };
            if !bean.key.eq_ignore_ascii_case(identifier) {
                return Err(JiraError::InvalidResponse(format!(
                    "Jira issue lookup for {identifier} returned {}",
                    bean.key
                )));
            }
            let in_project =
                bean.fields.project.as_ref().is_some_and(|project| {
                    project.key.eq_ignore_ascii_case(&self.config.project_key)
                });
            if enforce_project && !in_project {
                missing_issue_ids.push(identifier.clone());
                continue;
            }
            issues.push(normalize_issue(bean, &self.config.base_url)?);
        }

        if missing_issue_ids.is_empty() {
            Ok(issues)
        } else {
            Err(JiraError::MissingIssueIds {
                issue_ids: missing_issue_ids,
            })
        }
    }

    pub async fn issue_states_by_ids<S>(
        &self,
        issue_ids: &[S],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, JiraError>
    where
        S: AsRef<str>,
    {
        let issue_ids = normalize_strings(issue_ids);
        if issue_ids.is_empty() {
            return Ok(Vec::new());
        }
        let quoted = issue_ids
            .iter()
            .map(|issue_id| quote_jql_value(issue_id))
            .collect::<Result<Vec<_>, _>>()?;
        let jql = format!("id in ({})", quoted.join(", "));
        let beans = self.search_issues(&jql, STATE_FIELDS).await?;
        beans.into_iter().map(normalize_issue_state).collect()
    }

    pub async fn fetch_workpad_comment(
        &self,
        issue_id: &str,
    ) -> Result<Option<WorkpadComment>, JiraError> {
        let issue_id = normalize_required_string("issue_id", issue_id)?;
        let mut start_at = 0usize;
        let mut latest: Option<WorkpadComment> = None;

        loop {
            let path = format!(
                "/rest/api/3/issue/{issue_id}/comment?startAt={start_at}&maxResults={}",
                self.config.page_size
            );
            let response: CommentsResponse = self
                .execute(Method::GET, &path, "issue comments", None)
                .await?;
            let fetched = response.comments.len();

            for comment in response.comments {
                let Some(body) = document_text(&comment.body) else {
                    continue;
                };
                if !contains_workpad_marker(&body) {
                    continue;
                }
                let updated_at = parse_datetime("updated", Some(&comment.updated))?;
                let candidate = WorkpadComment {
                    id: comment.id,
                    body,
                    updated_at,
                };
                if latest
                    .as_ref()
                    .is_none_or(|existing| candidate.updated_at > existing.updated_at)
                {
                    latest = Some(candidate);
                }
            }

            start_at = response.start_at + fetched;
            if fetched == 0 || start_at >= response.total {
                return Ok(latest);
            }
        }
    }

    async fn fetch_issue(&self, identifier: &str) -> Result<JiraIssueBean, JiraError> {
        let identifier = validate_jql_bare_value(identifier)?;
        let path = format!(
            "/rest/api/3/issue/{identifier}?fields={}",
            ISSUE_FIELDS.join(",")
        );
        self.execute(Method::GET, &path, "issue lookup", None).await
    }

    async fn search_issues(
        &self,
        jql: &str,
        fields: &[&str],
    ) -> Result<Vec<JiraIssueBean>, JiraError> {
        let mut issues = Vec::new();
        let mut next_page_token: Option<String> = None;

        loop {
            let mut body = json!({
                "jql": jql,
                "maxResults": self.config.page_size,
                "fields": fields,
            });
            if let Some(token) = &next_page_token {
                body["nextPageToken"] = json!(token);
            }
            let response: SearchResponse = self
                .execute(Method::POST, SEARCH_PATH, "issue search", Some(body))
                .await?;
            let fetched = response.issues.len();
            issues.extend(response.issues);

            match response.next_page_token {
                Some(token) if fetched > 0 => next_page_token = Some(token),
                _ => return Ok(issues),
            }
        }
    }

    fn project_states_jql<S>(&self, state_names: &[S]) -> String
    where
        S: AsRef<str>,
    {
        let states = state_names
            .iter()
            .map(|state| quote_jql_string(state.as_ref()))
            .collect::<Vec<_>>();
        format!(
            "project = {} AND status in ({}) ORDER BY created ASC",
            quote_jql_string(&self.config.project_key),
            states.join(", ")
        )
    }

    async fn execute<T>(
        &self,
        method: Method,
        path_and_query: &str,
        operation: &str,
        body: Option<Value>,
    ) -> Result<T, JiraError>
    where
        T: DeserializeOwned,
    {
        let url = format!("{}{path_and_query}", self.config.base_url);
        let mut attempt = 1;

        loop {
            let mut request = self
                .http
                .request(method.clone(), &url)
                .header(AUTHORIZATION, &self.authorization)
                .header(ACCEPT, "application/json");
            if let Some(body) = &body {
                request = request.header(CONTENT_TYPE, "application/json").json(body);
            }

            let error = match request.send().await {
                Ok(response) => {
                    let status = response.status();
                    let retry_after = parse_retry_delay(response.headers());
                    match response.text().await {
                        Ok(payload) if status.is_success() => {
                            return serde_json::from_str(&payload).map_err(|error| {
                                JiraError::InvalidResponse(format!(
                                    "failed to decode Jira response for {operation} after HTTP {status}: {error} (body_bytes={})",
                                    payload.len()
                                ))
                            });
                        }
                        Ok(payload) => JiraError::HttpStatus {
                            status,
                            body: payload,
                            retry_after,
                        },
                        Err(error) => JiraError::ResponseBody {
                            operation: operation.to_string(),
                            status,
                            retry_after,
                            source: Box::new(error),
                        },
                    }
                }
                Err(error) => JiraError::Request(Box::new(error)),
            };

            if self.should_retry(&error, attempt) {
                self.sleep_before_retry(&error, attempt).await;
                attempt += 1;
                continue;
            }
            return Err(error);
        }
    }

    fn should_retry(&self, error: &JiraError, attempt: usize) -> bool {
        if attempt >= self.config.retry_policy.max_attempts {
            return false;
        }

        let max_inline_rate_limit_retry = std::cmp::min(
            self.config.retry_policy.max_backoff,
            MAX_INLINE_RATE_LIMIT_RETRY,
        );
        if error.is_rate_limited()
            && error
                .retry_after()
                .is_some_and(|delay| delay > max_inline_rate_limit_retry)
        {
            return false;
        }

        match error {
            JiraError::Request(_) => true,
            JiraError::ResponseBody { .. } => true,
            JiraError::HttpStatus { status, .. } => {
                *status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            JiraError::MissingIssueIds { .. }
            | JiraError::InvalidConfiguration(_)
            | JiraError::InvalidResponse(_) => false,
        }
    }

    async fn sleep_before_retry(&self, error: &JiraError, attempt: usize) {
        let delay = error
            .retry_after()
            .unwrap_or_else(|| self.exponential_backoff(attempt));
        debug!(
            attempt,
            delay_ms = delay.as_millis(),
            category = ?error.category(),
            "retrying Jira request"
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

fn contains_workpad_marker(body: &str) -> bool {
    body.lines()
        .any(|line| line.trim_start().starts_with("## Agent Harness Workpad"))
}

fn normalize_required_string(field_name: &str, value: &str) -> Result<String, JiraError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        Err(JiraError::InvalidConfiguration(format!(
            "{field_name} must be a non-empty string"
        )))
    } else {
        Ok(normalized.to_string())
    }
}

fn normalize_required_state_names(
    field_name: &str,
    values: &[String],
) -> Result<Vec<String>, JiraError> {
    let normalized = normalize_strings(values);
    if normalized.is_empty() {
        Err(JiraError::InvalidConfiguration(format!(
            "{field_name} must contain at least one state name"
        )))
    } else {
        Ok(normalized)
    }
}

fn normalize_strings<S>(values: &[S]) -> Vec<String>
where
    S: AsRef<str>,
{
    let mut normalized = values
        .iter()
        .map(|value| value.as_ref().trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    normalized.dedup();
    normalized
}

fn quote_jql_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

// Issue ids and keys are interpolated into JQL and URL paths, so restrict them
// to the characters Jira itself uses rather than attempting to escape.
fn validate_jql_bare_value(value: &str) -> Result<&str, JiraError> {
    let trimmed = value.trim();
    if !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        Ok(trimmed)
    } else {
        Err(JiraError::InvalidConfiguration(format!(
            "Jira issue id or key `{value}` contains unsupported characters"
        )))
    }
}

fn quote_jql_value(value: &str) -> Result<String, JiraError> {
    validate_jql_bare_value(value).map(quote_jql_string)
}

fn parse_retry_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    parse_rate_limit_reset(headers, SystemTime::now()).or_else(|| {
        let seconds = headers
            .get(RETRY_AFTER)?
            .to_str()
            .ok()?
            .trim()
            .parse::<u64>()
            .ok()?;
        Some(Duration::from_secs(seconds))
    })
}

fn parse_rate_limit_reset(
    headers: &reqwest::header::HeaderMap,
    now: SystemTime,
) -> Option<Duration> {
    // Jira Cloud advertises the end of the throttle window as an RFC 3339
    // timestamp in X-RateLimit-Reset.
    let reset = headers.get("x-ratelimit-reset")?.to_str().ok()?.trim();
    let reset = chrono::DateTime::parse_from_rfc3339(reset).ok()?;
    let now_ms = now.duration_since(UNIX_EPOCH).ok()?.as_millis() as i128;
    let delay_ms = (reset.timestamp_millis() as i128).saturating_sub(now_ms);
    let delay_ms = u64::try_from(delay_ms).unwrap_or(0);
    Some(Duration::from_millis(delay_ms))
}

fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let bytes = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let value = (u32::from(bytes[0]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[2]);
        output.push(ALPHABET[(value >> 18) as usize & 0x3f] as char);
        output.push(ALPHABET[(value >> 12) as usize & 0x3f] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[(value >> 6) as usize & 0x3f] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[value as usize & 0x3f] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    use super::{
        base64_encode, contains_workpad_marker, parse_rate_limit_reset, parse_retry_delay,
        quote_jql_string, validate_jql_bare_value,
    };

    #[test]
    fn base64_encoding_matches_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(
            base64_encode(b"user@example.com:token"),
            "dXNlckBleGFtcGxlLmNvbTp0b2tlbg=="
        );
    }

    #[test]
    fn jql_strings_escape_quotes_and_backslashes() {
        assert_eq!(quote_jql_string("In Progress"), "\"In Progress\"");
        assert_eq!(quote_jql_string("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_jql_string("back\\slash"), "\"back\\\\slash\"");
    }

    #[test]
    fn bare_jql_values_reject_injection_attempts() {
        assert_eq!(validate_jql_bare_value(" OSYM-12 ").ok(), Some("OSYM-12"));
        assert_eq!(validate_jql_bare_value("10001").ok(), Some("10001"));
        assert!(validate_jql_bare_value("OSYM-1) OR (id > 0").is_err());
        assert!(validate_jql_bare_value("").is_err());
    }

    #[test]
    fn workpad_marker_detection_matches_linear_behavior() {
        assert!(contains_workpad_marker(
            "intro\n  ## Agent Harness Workpad\nbody"
        ));
        assert!(!contains_workpad_marker("## Some other heading"));
    }

    #[test]
    fn retry_after_header_parses_as_seconds() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("7"));
        assert_eq!(parse_retry_delay(&headers), Some(Duration::from_secs(7)));
    }

    #[test]
    fn rate_limit_reset_header_parses_as_rfc3339() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-reset",
            HeaderValue::from_static("1970-01-01T00:00:10Z"),
        );
        let now = UNIX_EPOCH + Duration::from_secs(4);
        assert_eq!(
            parse_rate_limit_reset(&headers, now),
            Some(Duration::from_secs(6))
        );
    }
}
