use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Response, StatusCode},
    routing::post,
};
use opensymphony_linear::{LinearClient, LinearConfig, RetryPolicy};
use opensymphony_orchestrator::filter_issues_for_dispatch;
use serde_json::{Value, json};
use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

#[tokio::test]
async fn fake_linear_hierarchy_keeps_parent_blocked_until_child_completes() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(&candidate_issues_payload(vec![
            active_parent_with_children(
                "issue-p1",
                "COE-277",
                &[child_ref("issue-s1", "COE-278", "In Progress")],
            ),
            active_leaf_issue("issue-s1", "COE-278", Some("issue-p1")),
        ])),
        QueuedResponse::json(&candidate_issues_payload(vec![
            active_parent_with_children(
                "issue-p1",
                "COE-277",
                &[child_ref("issue-s1", "COE-278", "Done")],
            ),
        ])),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");
    let terminal_states = terminal_states();

    let first_snapshot = client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");
    let first_dispatchable = filter_issues_for_dispatch(first_snapshot, &terminal_states);
    assert_eq!(
        first_dispatchable
            .iter()
            .map(|issue| issue.identifier.as_str())
            .collect::<Vec<_>>(),
        vec!["COE-278"]
    );

    let second_snapshot = client
        .candidate_issues()
        .await
        .expect("follow-up candidate query should succeed");
    let second_dispatchable = filter_issues_for_dispatch(second_snapshot, &terminal_states);
    assert_eq!(
        second_dispatchable
            .iter()
            .map(|issue| issue.identifier.as_str())
            .collect::<Vec<_>>(),
        vec!["COE-277"]
    );
}

#[tokio::test]
async fn fake_linear_hierarchy_reblocks_parent_when_new_child_is_added() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(&candidate_issues_payload(vec![
            active_parent_with_children("issue-p1", "COE-277", &[]),
        ])),
        QueuedResponse::json(&candidate_issues_payload(vec![
            active_parent_with_children(
                "issue-p1",
                "COE-277",
                &[child_ref("issue-s1", "COE-278", "Todo")],
            ),
        ])),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");
    let terminal_states = terminal_states();

    let first_snapshot = client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");
    let first_dispatchable = filter_issues_for_dispatch(first_snapshot, &terminal_states);
    assert_eq!(
        first_dispatchable
            .iter()
            .map(|issue| issue.identifier.as_str())
            .collect::<Vec<_>>(),
        vec!["COE-277"]
    );

    let second_snapshot = client
        .candidate_issues()
        .await
        .expect("follow-up candidate query should succeed");
    let second_dispatchable = filter_issues_for_dispatch(second_snapshot, &terminal_states);
    assert!(second_dispatchable.is_empty());
}

fn terminal_states() -> HashSet<String> {
    HashSet::from([String::from("Done"), String::from("Canceled")])
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

fn candidate_issues_payload(nodes: Vec<Value>) -> String {
    json!({
        "data": {
            "issues": {
                "nodes": nodes,
                "pageInfo": {
                    "hasNextPage": false,
                    "endCursor": null
                }
            }
        }
    })
    .to_string()
}

fn active_parent_with_children(id: &str, identifier: &str, children: &[Value]) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "url": format!("https://linear.app/trilogy-ai-coe/issue/{identifier}/parent"),
        "title": format!("Issue {identifier}"),
        "description": null,
        "priority": 1.0,
        "createdAt": "2026-03-22T00:00:00Z",
        "updatedAt": "2026-03-22T00:00:00Z",
        "state": {
            "id": "state-started",
            "name": "In Progress",
            "type": "started"
        },
        "parent": null,
        "children": {
            "nodes": children
        },
        "labels": {
            "nodes": []
        },
        "inverseRelations": {
            "nodes": [],
            "pageInfo": {
                "hasNextPage": false,
                "endCursor": null
            }
        }
    })
}

fn active_leaf_issue(id: &str, identifier: &str, parent_id: Option<&str>) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "url": format!("https://linear.app/trilogy-ai-coe/issue/{identifier}/leaf"),
        "title": format!("Issue {identifier}"),
        "description": null,
        "priority": 1.0,
        "createdAt": "2026-03-23T00:00:00Z",
        "updatedAt": "2026-03-23T00:00:00Z",
        "state": {
            "id": "state-started",
            "name": "In Progress",
            "type": "started"
        },
        "parent": parent_id.map(|value| json!({ "id": value })),
        "children": {
            "nodes": []
        },
        "labels": {
            "nodes": []
        },
        "inverseRelations": {
            "nodes": [],
            "pageInfo": {
                "hasNextPage": false,
                "endCursor": null
            }
        }
    })
}

fn child_ref(id: &str, identifier: &str, state: &str) -> Value {
    json!({
        "id": id,
        "identifier": identifier,
        "state": {
            "name": state
        }
    })
}

#[derive(Clone)]
struct AppState {
    responses: Arc<Mutex<VecDeque<QueuedResponse>>>,
}

struct MockGraphqlServer {
    base_url: String,
    task: JoinHandle<()>,
}

impl MockGraphqlServer {
    async fn start(responses: Vec<QueuedResponse>) -> Self {
        let state = AppState {
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
        };
        let app = Router::new()
            .route("/graphql", post(handle_graphql))
            .with_state(state);
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
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for MockGraphqlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn handle_graphql(State(state): State<AppState>, Json(_body): Json<Value>) -> Response<Body> {
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
