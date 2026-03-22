use std::{process::Stdio, sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    process::Command,
    sync::Mutex,
    task::JoinHandle,
    time::timeout,
};

#[tokio::test]
async fn linear_mcp_stdio_server_advertises_tools_and_executes_writes() {
    let server = MockLinearGraphqlServer::start().await;

    let mut child = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("linear-mcp")
        .arg("--stdio")
        .env("LINEAR_API_KEY", "test-linear-key")
        .env("LINEAR_BASE_URL", server.base_url())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("linear-mcp command should spawn");

    let mut stdin = child.stdin.take().expect("child stdin should exist");
    let stdout = child.stdout.take().expect("child stdout should exist");
    let mut stdout_lines = BufReader::new(stdout).lines();

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {
                    "name": "opensymphony-test",
                    "version": "0.1.0"
                }
            }
        }),
    )
    .await;

    let initialize = read_json(&mut stdout_lines).await;
    assert_eq!(initialize["result"]["protocolVersion"], json!("2025-11-25"));

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    )
    .await;

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
    )
    .await;

    let tools = read_json(&mut stdout_lines).await;
    let tool_names = tools["result"]["tools"]
        .as_array()
        .expect("tools should be an array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        tool_names,
        vec![
            "linear_get_issue",
            "linear_comment_issue",
            "linear_transition_issue",
            "linear_link_pr",
            "linear_list_project_states",
        ]
    );

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "linear_get_issue",
                "arguments": {
                    "issue": "COE-267"
                }
            }
        }),
    )
    .await;
    let get_issue = read_json(&mut stdout_lines).await;
    let get_issue_payload = tool_payload(&get_issue["result"]);
    assert_eq!(get_issue_payload["issue"]["identifier"], json!("COE-267"));
    assert_eq!(get_issue_payload["issue"]["team"]["key"], json!("COE"));

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "linear_comment_issue",
                "arguments": {
                    "issue": "COE-267",
                    "body": "Comment from test"
                }
            }
        }),
    )
    .await;
    let comment_issue = read_json(&mut stdout_lines).await;
    let comment_payload = tool_payload(&comment_issue["result"]);
    assert_eq!(comment_payload["comment"]["id"], json!("comment-1"));
    assert_eq!(
        comment_payload["comment"]["body"],
        json!("Comment from test")
    );

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "linear_transition_issue",
                "arguments": {
                    "issue": "COE-267",
                    "state": "Done"
                }
            }
        }),
    )
    .await;
    let transition_issue = read_json(&mut stdout_lines).await;
    let transition_payload = tool_payload(&transition_issue["result"]);
    assert_eq!(transition_payload["issue"]["state"]["name"], json!("Done"));
    assert_eq!(
        transition_payload["issue"]["state"]["type"],
        json!("completed")
    );

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "linear_link_pr",
                "arguments": {
                    "issue": "COE-267",
                    "url": "https://github.com/kumanday/OpenSymphony/pull/99",
                    "title": "OpenSymphony PR 99"
                }
            }
        }),
    )
    .await;
    let link_pr = read_json(&mut stdout_lines).await;
    let link_payload = tool_payload(&link_pr["result"]);
    assert_eq!(
        link_payload["attachment"]["url"],
        json!("https://github.com/kumanday/OpenSymphony/pull/99")
    );

    send_json(
        &mut stdin,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "linear_list_project_states",
                "arguments": {
                    "issue": "COE-267"
                }
            }
        }),
    )
    .await;
    let list_states = read_json(&mut stdout_lines).await;
    let states_payload = tool_payload(&list_states["result"]);
    assert_eq!(states_payload["team"]["key"], json!("COE"));
    assert_eq!(states_payload["states"].as_array().unwrap().len(), 3);

    drop(stdin);
    let status = timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("linear-mcp should exit after stdin closes")
        .expect("linear-mcp child should wait successfully");
    assert!(status.success(), "linear-mcp should exit successfully");

    let requests = server.recorded_requests().await;
    assert!(
        requests
            .iter()
            .all(|request| request.authorization.as_deref() == Some("test-linear-key")),
        "all GraphQL requests should carry the configured API key"
    );
}

async fn send_json(stdin: &mut tokio::process::ChildStdin, value: Value) {
    let encoded = serde_json::to_vec(&value).expect("message should encode");
    stdin
        .write_all(&encoded)
        .await
        .expect("stdin write should succeed");
    stdin
        .write_all(b"\n")
        .await
        .expect("stdin newline should succeed");
    stdin.flush().await.expect("stdin flush should succeed");
}

async fn read_json(
    stdout_lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> Value {
    let line = stdout_lines
        .next_line()
        .await
        .expect("stdout should remain readable")
        .expect("server should emit a response line");
    serde_json::from_str(&line).expect("response line should be valid JSON")
}

fn tool_payload(result: &Value) -> Value {
    if !result["structuredContent"].is_null() {
        return result["structuredContent"].clone();
    }

    let text = result["content"][0]["text"]
        .as_str()
        .expect("tool result should include a text payload");
    serde_json::from_str(text).expect("tool text payload should be valid JSON")
}

#[derive(Clone)]
struct MockState {
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
}

struct MockLinearGraphqlServer {
    base_url: String,
    state: MockState,
    task: JoinHandle<()>,
}

impl MockLinearGraphqlServer {
    async fn start() -> Self {
        let state = MockState {
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

impl Drop for MockLinearGraphqlServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone)]
struct CapturedRequest {
    authorization: Option<String>,
}

async fn handle_graphql(
    State(state): State<MockState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response<Body> {
    let captured = CapturedRequest {
        authorization: headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned),
    };
    state.requests.lock().await.push(captured);

    let query = body["query"].as_str().unwrap_or_default();
    let variables = &body["variables"];

    let response = if query.contains("ResolveIssueByIdentifier") {
        json!({
            "data": {
                "issues": {
                    "nodes": [issue_payload("state-in-progress", "In Progress", "started")]
                }
            }
        })
    } else if query.contains("ResolveIssueById") {
        json!({
            "data": {
                "issue": issue_payload("state-in-progress", "In Progress", "started")
            }
        })
    } else if query.contains("WorkflowStatesByTeamId") || query.contains("WorkflowStatesByTeamKey")
    {
        json!({
            "data": {
                "workflowStates": {
                    "nodes": [
                        workflow_state_payload("state-todo", "Todo", "unstarted", 1.0),
                        workflow_state_payload("state-in-progress", "In Progress", "started", 2.0),
                        workflow_state_payload("state-done", "Done", "completed", 3.0)
                    ]
                }
            }
        })
    } else if query.contains("CommentIssue") {
        assert_eq!(variables["issueId"], json!("issue-267"));
        assert_eq!(variables["body"], json!("Comment from test"));
        json!({
            "data": {
                "commentCreate": {
                    "success": true,
                    "comment": {
                        "id": "comment-1",
                        "body": variables["body"],
                        "url": "https://linear.app/comment/comment-1"
                    }
                }
            }
        })
    } else if query.contains("TransitionIssue") {
        assert_eq!(variables["id"], json!("issue-267"));
        let state_id = variables["stateId"].as_str().unwrap_or_default();
        let (state_name, state_type) = match state_id {
            "state-done" => ("Done", "completed"),
            "state-in-progress" => ("In Progress", "started"),
            "state-todo" => ("Todo", "unstarted"),
            other => panic!("unexpected state id `{other}`"),
        };
        json!({
            "data": {
                "issueUpdate": {
                    "success": true,
                    "issue": issue_payload(state_id, state_name, state_type)
                }
            }
        })
    } else if query.contains("LinkIssueUrl") {
        assert_eq!(variables["issueId"], json!("issue-267"));
        json!({
            "data": {
                "attachmentLinkURL": {
                    "success": true,
                    "attachment": {
                        "id": "attachment-1",
                        "title": variables["title"],
                        "url": variables["url"]
                    }
                }
            }
        })
    } else {
        panic!("unexpected GraphQL operation: {query}");
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(Body::from(response.to_string()))
        .expect("response should be valid")
}

fn issue_payload(state_id: &str, state_name: &str, state_type: &str) -> Value {
    json!({
        "id": "issue-267",
        "identifier": "COE-267",
        "url": "https://linear.app/trilogy-ai-coe/issue/COE-267/linear-mcp-write-surface",
        "title": "Linear MCP write surface",
        "description": "Implement the Linear MCP server.",
        "priority": 2,
        "state": {
            "id": state_id,
            "name": state_name,
            "type": state_type
        },
        "team": {
            "id": "team-coe",
            "key": "COE",
            "name": "Trilogy AI COE"
        },
        "labels": {
            "nodes": [
                { "name": "tracker-tools" }
            ]
        },
        "inverseRelations": {
            "nodes": [
                {
                    "type": "blocks",
                    "issue": {
                        "id": "issue-258",
                        "identifier": "COE-258",
                        "title": "Bootstrap workspace and crate boundaries",
                        "state": {
                            "id": "state-done",
                            "name": "Done",
                            "type": "completed"
                        }
                    }
                }
            ]
        },
        "createdAt": "2026-03-21T19:59:51.661Z",
        "updatedAt": "2026-03-22T07:01:00.884Z"
    })
}

fn workflow_state_payload(id: &str, name: &str, tracker_type: &str, position: f64) -> Value {
    json!({
        "id": id,
        "name": name,
        "type": tracker_type,
        "position": position,
        "team": {
            "id": "team-coe",
            "key": "COE",
            "name": "Trilogy AI COE"
        }
    })
}
