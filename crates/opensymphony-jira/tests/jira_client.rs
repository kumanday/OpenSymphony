use std::{collections::VecDeque, sync::Arc, time::Duration};

use crate::opensymphony_domain::{TrackerErrorCategory, TrackerIssueStateKind};
use crate::opensymphony_jira::{JiraClient, JiraConfig, JiraError, RetryPolicy};
use axum::{
    Router,
    body::Body,
    extract::{Request, State},
    http::{Response, StatusCode},
};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

#[tokio::test]
async fn candidate_issues_paginate_and_normalize_fixture_payloads() {
    let server = MockRestServer::start(vec![
        QueuedResponse::json(include_str!("fixtures/search_active_page_1.json")),
        QueuedResponse::json(include_str!("fixtures/search_active_page_2.json")),
    ])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");

    assert_eq!(issues.len(), 2);

    let first = &issues[0];
    assert_eq!(first.id, "10001");
    assert_eq!(first.identifier, "OSYM-1");
    assert_eq!(first.url, format!("{}/browse/OSYM-1", server.base_url()));
    assert_eq!(first.title, "Bootstrap the orchestrator");
    assert_eq!(
        first.description.as_deref(),
        Some("Stand up the scheduler loop.")
    );
    assert_eq!(first.priority, Some(1));
    assert_eq!(first.state, "In Progress");
    assert_eq!(first.state_kind, TrackerIssueStateKind::Started);
    assert_eq!(first.branch_name, None);
    assert_eq!(first.pr_url, None);
    assert_eq!(first.labels, vec!["backend", "urgent"]);
    assert_eq!(first.project_id.as_deref(), Some("20000"));
    assert_eq!(first.project_slug.as_deref(), Some("OSYM"));
    assert_eq!(first.project_name.as_deref(), Some("OpenSymphony"));
    assert_eq!(
        first
            .project_milestone
            .as_ref()
            .map(|milestone| milestone.name.as_str()),
        Some("Milestone 1")
    );
    assert_eq!(
        first.blocked_by.len(),
        1,
        "only `is blocked by` links count"
    );
    assert_eq!(first.blocked_by[0].identifier, "OSYM-9");
    assert!(first.blocked_by[0].is_terminal());
    assert_eq!(first.blocked_by[0].state.tracker_type, "done");

    let second = &issues[1];
    assert_eq!(second.identifier, "OSYM-2");
    assert_eq!(second.description, None);
    assert_eq!(second.priority, None);
    assert_eq!(second.state_kind, TrackerIssueStateKind::Unstarted);
    assert_eq!(second.parent_id.as_deref(), Some("10001"));
    assert_eq!(
        second
            .parent
            .as_ref()
            .map(|parent| parent.identifier.as_str()),
        Some("OSYM-1")
    );
    assert_eq!(second.sub_issues.len(), 2);
    assert_eq!(second.sub_issues[0].identifier, "OSYM-21");
    assert_eq!(second.sub_issues[0].state, "To Do");
    assert_eq!(second.sub_issues[1].identifier, "OSYM-22");
    assert_eq!(second.sub_issues[1].state, "Done");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].path, "/rest/api/3/search/jql");
    assert_eq!(
        requests[0].authorization.as_deref(),
        // base64("bot@example.com:test-token")
        Some("Basic Ym90QGV4YW1wbGUuY29tOnRlc3QtdG9rZW4=")
    );
    let first_body: serde_json::Value =
        serde_json::from_str(&requests[0].body).expect("search request body should be JSON");
    assert_eq!(
        first_body["jql"],
        "project = \"OSYM\" AND status in (\"To Do\", \"In Progress\") ORDER BY created ASC"
    );
    assert!(first_body.get("nextPageToken").is_none());
    let second_body: serde_json::Value =
        serde_json::from_str(&requests[1].body).expect("search request body should be JSON");
    assert_eq!(second_body["nextPageToken"], "PAGE-2");
}

#[tokio::test]
async fn terminal_issues_query_terminal_states() {
    let server = MockRestServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/search_active_page_2.json"
    ))])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    client
        .terminal_issues()
        .await
        .expect("terminal query should succeed");

    let requests = server.recorded_requests().await;
    let body: serde_json::Value =
        serde_json::from_str(&requests[0].body).expect("search request body should be JSON");
    assert_eq!(
        body["jql"],
        "project = \"OSYM\" AND status in (\"Done\") ORDER BY created ASC"
    );
}

#[tokio::test]
async fn project_issues_by_identifiers_report_missing_issues() {
    let server = MockRestServer::start(vec![
        QueuedResponse::json(include_str!("fixtures/issue_osym_2.json")),
        QueuedResponse::new(StatusCode::NOT_FOUND, r#"{"errorMessages":["not found"]}"#),
    ])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let error = client
        .project_issues_by_identifiers(&["OSYM-2", "OSYM-404"])
        .await
        .expect_err("missing issues should surface as MissingIssueIds");

    match error {
        JiraError::MissingIssueIds { issue_ids } => {
            assert_eq!(issue_ids, vec!["OSYM-404".to_string()]);
        }
        other => panic!("expected MissingIssueIds, got {other:?}"),
    }

    let requests = server.recorded_requests().await;
    assert_eq!(requests[0].method, "GET");
    assert!(
        requests[0]
            .path
            .starts_with("/rest/api/3/issue/OSYM-2?fields=")
    );
}

#[tokio::test]
async fn project_issues_by_identifiers_treat_foreign_projects_as_missing() {
    let server = MockRestServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/issue_foreign_project.json"
    ))])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let error = client
        .project_issues_by_identifiers(&["OTHER-1"])
        .await
        .expect_err("issues outside the configured project should be missing");

    assert!(matches!(
        error,
        JiraError::MissingIssueIds { issue_ids } if issue_ids == vec!["OTHER-1".to_string()]
    ));
}

#[tokio::test]
async fn issues_by_identifiers_do_not_filter_by_project() {
    let server = MockRestServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/issue_foreign_project.json"
    ))])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .issues_by_identifiers(&["OTHER-1"])
        .await
        .expect("unscoped lookup should succeed");

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].identifier, "OTHER-1");
}

#[tokio::test]
async fn issue_states_by_ids_normalize_status_snapshots() {
    let server = MockRestServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/issue_states_page.json"
    ))])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let snapshots = client
        .issue_states_by_ids(&["10001", "10002"])
        .await
        .expect("state refresh should succeed");

    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].id, "10001");
    assert_eq!(snapshots[0].identifier, "OSYM-1");
    assert_eq!(snapshots[0].state.name, "In Progress");
    assert!(!snapshots[0].state.is_terminal());
    assert_eq!(snapshots[1].id, "10002");
    assert!(snapshots[1].state.is_terminal());

    let requests = server.recorded_requests().await;
    let body: serde_json::Value =
        serde_json::from_str(&requests[0].body).expect("search request body should be JSON");
    assert_eq!(body["jql"], "id in (\"10001\", \"10002\")");
}

#[tokio::test]
async fn issue_states_by_ids_reject_jql_injection() {
    let server = MockRestServer::start(Vec::new()).await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let error = client
        .issue_states_by_ids(&["10001) OR (id > 0"])
        .await
        .expect_err("hostile identifiers should be rejected before any request");

    assert!(matches!(error, JiraError::InvalidConfiguration(_)));
    assert!(server.recorded_requests().await.is_empty());
}

#[tokio::test]
async fn fetch_workpad_comment_returns_latest_marker_comment() {
    let server = MockRestServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/comments_page.json"
    ))])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let comment = client
        .fetch_workpad_comment("10001")
        .await
        .expect("comment fetch should succeed")
        .expect("workpad comment should be found");

    assert_eq!(comment.id, "50003");
    assert_eq!(
        comment.body,
        "## Agent Harness Workpad\nResume from step 5."
    );

    let requests = server.recorded_requests().await;
    assert_eq!(requests[0].method, "GET");
    assert!(
        requests[0]
            .path
            .starts_with("/rest/api/3/issue/10001/comment?startAt=0")
    );
}

#[tokio::test]
async fn rate_limited_requests_retry_with_retry_after() {
    let server = MockRestServer::start(vec![
        QueuedResponse::new(StatusCode::TOO_MANY_REQUESTS, r#"{"errorMessages":[]}"#)
            .with_header("retry-after", "0"),
        QueuedResponse::json(include_str!("fixtures/search_active_page_2.json")),
    ])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("rate-limited request should retry and succeed");

    assert_eq!(issues.len(), 1);
    assert_eq!(server.recorded_requests().await.len(), 2);
}

#[tokio::test]
async fn auth_failures_map_to_tracker_categories_without_retry() {
    let server = MockRestServer::start(vec![QueuedResponse::new(
        StatusCode::UNAUTHORIZED,
        r#"{"errorMessages":["bad token"]}"#,
    )])
    .await;
    let client = JiraClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let error = client
        .candidate_issues()
        .await
        .expect_err("auth failures should not be retried");

    assert_eq!(error.category(), TrackerErrorCategory::Auth);
    assert_eq!(server.recorded_requests().await.len(), 1);
}

#[tokio::test]
async fn bearer_tokens_are_sent_when_no_email_is_configured() {
    let server = MockRestServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/search_active_page_2.json"
    ))])
    .await;
    let mut config = test_config(server.base_url());
    config.auth_email = None;
    let client = JiraClient::new(config).expect("client configuration should be valid");

    client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");

    let requests = server.recorded_requests().await;
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer test-token")
    );
}

#[tokio::test]
async fn invalid_configuration_is_rejected() {
    assert!(matches!(
        JiraClient::new(JiraConfig::new("", "token", "OSYM")),
        Err(JiraError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        JiraClient::new(JiraConfig::new("https://acme.atlassian.net", " ", "OSYM")),
        Err(JiraError::InvalidConfiguration(_))
    ));
    assert!(matches!(
        JiraClient::new(JiraConfig::new("https://acme.atlassian.net", "token", "")),
        Err(JiraError::InvalidConfiguration(_))
    ));

    let mut missing_states = JiraConfig::new("https://acme.atlassian.net", "token", "OSYM");
    missing_states.terminal_states = vec!["Done".to_string()];
    assert!(matches!(
        JiraClient::new(missing_states),
        Err(JiraError::InvalidConfiguration(_))
    ));
}

fn test_config(base_url: &str) -> JiraConfig {
    let mut config = JiraConfig::new(base_url, "test-token", "OSYM");
    config.auth_email = Some("bot@example.com".to_string());
    config.active_states = vec!["To Do".to_string(), "In Progress".to_string()];
    config.terminal_states = vec!["Done".to_string()];
    config.retry_policy = RetryPolicy {
        max_attempts: 3,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(2),
    };
    config
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: String,
}

#[derive(Clone)]
struct AppState {
    responses: Arc<Mutex<VecDeque<QueuedResponse>>>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

struct MockRestServer {
    base_url: String,
    state: AppState,
    task: JoinHandle<()>,
}

impl MockRestServer {
    async fn start(responses: Vec<QueuedResponse>) -> Self {
        let state = AppState {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .fallback(handle_request)
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should stay up");
        });

        Self {
            base_url: format!("http://{address}"),
            state,
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn recorded_requests(&self) -> Vec<CapturedRequest> {
        self.state.requests.lock().await.clone()
    }
}

impl Drop for MockRestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_request(State(state): State<AppState>, request: Request) -> Response<Body> {
    let method = request.method().to_string();
    let path = request
        .uri()
        .path_and_query()
        .map(|path_and_query| path_and_query.to_string())
        .unwrap_or_else(|| request.uri().path().to_string());
    let authorization = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let body_bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .expect("request body should be readable");
    state.requests.lock().await.push(CapturedRequest {
        method,
        path,
        authorization,
        body: String::from_utf8_lossy(&body_bytes).into_owned(),
    });

    let response = state
        .responses
        .lock()
        .await
        .pop_front()
        .expect("test did not queue enough responses");

    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(Body::from(response.body))
        .expect("response should be valid")
}

struct QueuedResponse {
    status: StatusCode,
    body: String,
    headers: Vec<(String, String)>,
}

impl QueuedResponse {
    fn json(body: impl Into<String>) -> Self {
        Self::new(StatusCode::OK, body).with_header("content-type", "application/json")
    }

    fn new(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into(),
            headers: Vec::new(),
        }
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}
