use std::{collections::VecDeque, sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    routing::post,
    Json, Router,
};
use opensymphony_domain::{TrackerErrorCategory, TrackerIssueStateKind};
use opensymphony_linear::{LinearClient, LinearConfig, RetryPolicy};
use serde_json::Value;
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

#[tokio::test]
async fn candidate_issues_normalize_fixture_payloads() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/candidate_issues_page.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");

    assert_eq!(issues.len(), 2);

    let first = &issues[0];
    assert_eq!(first.identifier, "COE-260");
    assert_eq!(
        first.url,
        "https://linear.app/trilogy-ai-coe/issue/COE-260/domain-model-and-orchestrator-state-machine"
    );
    assert_eq!(first.priority, Some(1));
    assert_eq!(first.state, "In Progress");
    assert_eq!(first.labels, vec!["backend", "urgent"]);
    assert_eq!(first.blocked_by.len(), 1);
    assert!(first.blocked_by[0].is_terminal());

    let second = &issues[1];
    assert_eq!(second.identifier, "COE-264");
    assert_eq!(
        second.url,
        "https://linear.app/trilogy-ai-coe/issue/COE-264/linear-read-adapter-and-issue-normalization"
    );
    assert_eq!(second.priority, None);
    assert_eq!(second.state, "In Progress");
    assert_eq!(second.blocked_by.len(), 1);
    assert_eq!(second.blocked_by[0].identifier, "COE-261");
    assert_eq!(
        second.blocked_by[0].state.kind,
        TrackerIssueStateKind::Started
    );

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer test-token")
    );
    assert_eq!(
        requests[0].body["variables"]["projectSlug"],
        Value::String("e7b957855cb7".to_string())
    );
    assert_eq!(
        requests[0].body["variables"]["stateNames"],
        serde_json::json!(["In Progress"])
    );
}

#[tokio::test]
async fn candidate_issues_fetch_all_inverse_relation_pages() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(include_str!(
            "fixtures/candidate_issues_with_relation_paging.json"
        )),
        QueuedResponse::json(include_str!("fixtures/issue_inverse_relations_page_2.json")),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].identifier, "COE-264");
    assert_eq!(issues[0].blocked_by.len(), 1);
    assert_eq!(issues[0].blocked_by[0].identifier, "COE-258");
    assert!(issues[0].blocked_by[0].is_terminal());

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[1].body["query"]
        .as_str()
        .expect("query should be a string")
        .contains("query IssueInverseRelationsPage"));
    assert_eq!(
        requests[1].body["variables"]["issueId"],
        Value::String("issue-264".to_string())
    );
    assert_eq!(
        requests[1].body["variables"]["after"],
        Value::String("relations-cursor-1".to_string())
    );
}

#[tokio::test]
async fn issues_by_state_walk_pagination() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(include_str!("fixtures/issues_page_1.json")),
        QueuedResponse::json(include_str!("fixtures/issues_page_2.json")),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.page_size = 2;
    let client = LinearClient::new(config).expect("client configuration should be valid");

    let issues = client
        .issues_by_state_names(&["Todo".to_string(), "In Progress".to_string()])
        .await
        .expect("pagination query should succeed");

    assert_eq!(issues.len(), 3);
    assert_eq!(issues[0].identifier, "COE-260");
    assert_eq!(issues[0].priority, Some(1));
    assert_eq!(issues[1].priority, Some(2));
    assert_eq!(issues[2].identifier, "COE-264");
    assert_eq!(issues[2].priority, Some(1));

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].body["query"]
        .as_str()
        .expect("query should be a string")
        .contains("query IssuesByState"));
    assert_eq!(requests[0].body["variables"]["after"], Value::Null);
    assert_eq!(
        requests[1].body["variables"]["after"],
        Value::String("cursor-1".to_string())
    );
}

#[tokio::test]
async fn issue_states_by_ids_return_normalized_snapshots() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/issue_states_page.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let snapshots = client
        .issue_states_by_ids(&["issue-260".to_string(), "issue-264".to_string()])
        .await
        .expect("issue state query should succeed");

    assert_eq!(snapshots.len(), 2);
    assert_eq!(snapshots[0].identifier, "COE-260");
    assert_eq!(snapshots[0].state.kind, TrackerIssueStateKind::Completed);
    assert_eq!(snapshots[1].identifier, "COE-264");
    assert_eq!(snapshots[1].state.kind, TrackerIssueStateKind::Canceled);

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].body["query"]
        .as_str()
        .expect("query should be a string")
        .contains("query IssueStatesByIds"));
    assert_eq!(
        requests[0].body["variables"]["issueIds"],
        serde_json::json!(["issue-260", "issue-264"])
    );
}

#[tokio::test]
async fn issue_states_by_ids_fail_when_linear_omits_requested_ids() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/issue_states_missing_id.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let error = client
        .issue_states_by_ids(&["issue-260".to_string(), "issue-264".to_string()])
        .await
        .expect_err("missing issue ids should fail reconciliation");

    assert_eq!(error.category(), TrackerErrorCategory::NotFound);
    assert!(error.to_string().contains("issue-264"));
}

#[tokio::test]
async fn rate_limited_requests_retry_using_retry_after() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\":\"rate limited\"}",
        )
        .with_header("retry-after", "0"),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("client should retry the rate-limited request");

    assert_eq!(issues.len(), 2);
    assert_eq!(server.recorded_requests().await.len(), 2);
}

#[tokio::test]
async fn permission_denied_maps_to_tracker_error_category() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::new(
        StatusCode::FORBIDDEN,
        "{\"error\":\"forbidden\"}",
    )])
    .await;
    let mut config = test_config(server.base_url());
    config.retry_policy.max_attempts = 1;
    let client = LinearClient::new(config).expect("client configuration should be valid");

    let error = client
        .candidate_issues()
        .await
        .expect_err("permission denied response should fail");

    assert_eq!(error.category(), TrackerErrorCategory::PermissionDenied);
}

fn test_config(base_url: &str) -> LinearConfig {
    let mut config = LinearConfig::new("test-token", "e7b957855cb7");
    config.base_url = base_url.to_string();
    config.active_states = vec!["In Progress".to_string()];
    config.terminal_states = vec!["Done".to_string(), "Canceled".to_string()];
    config.request_timeout = Duration::from_secs(2);
    config.retry_policy = RetryPolicy {
        max_attempts: 2,
        initial_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(1),
    };
    config
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    authorization: Option<String>,
    body: Value,
}

#[derive(Clone)]
struct AppState {
    responses: Arc<Mutex<VecDeque<QueuedResponse>>>,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

struct MockGraphqlServer {
    base_url: String,
    state: AppState,
    task: JoinHandle<()>,
}

impl MockGraphqlServer {
    async fn start(responses: Vec<QueuedResponse>) -> Self {
        let state = AppState {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
            requests: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new()
            .route("/graphql", post(handle_graphql))
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
            base_url: format!("http://{address}/graphql"),
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

impl Drop for MockGraphqlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_graphql(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response<Body> {
    let request = CapturedRequest {
        authorization: headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
        body,
    };
    state.requests.lock().await.push(request);

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
    fn json(body: &str) -> Self {
        Self::new(StatusCode::OK, body).with_header("content-type", "application/json")
    }

    fn new(status: StatusCode, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: Vec::new(),
        }
    }

    fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }
}
