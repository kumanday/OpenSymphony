use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::opensymphony_domain::{TrackerErrorCategory, TrackerIssueStateKind};
use crate::opensymphony_linear::{LinearClient, LinearConfig, LinearError, RetryPolicy};
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Response, StatusCode},
    routing::post,
};
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
    assert_eq!(
        first.branch_name.as_deref(),
        Some("feat/coe-260-domain-model")
    );
    assert_eq!(
        first.pr_url.as_deref(),
        Some("https://github.com/kumanday/OpenSymphony/pull/260")
    );
    assert_eq!(first.labels, vec!["backend", "urgent"]);
    assert_eq!(first.project_id.as_deref(), Some("proj-open"));
    assert_eq!(
        first.project_slug.as_deref(),
        Some("opensymphony-bootstrap")
    );
    assert_eq!(first.project_name.as_deref(), Some("OpenSymphony"));
    assert_eq!(first.parent_id, None);
    assert!(first.sub_issues.is_empty());
    assert_eq!(first.blocked_by.len(), 1);
    assert!(first.blocked_by[0].is_terminal());
    assert_eq!(first.blocked_by[0].state.tracker_type, "completed");

    let second = &issues[1];
    assert_eq!(second.identifier, "COE-264");
    assert_eq!(
        second.url,
        "https://linear.app/trilogy-ai-coe/issue/COE-264/linear-read-adapter-and-issue-normalization"
    );
    assert_eq!(second.priority, None);
    assert_eq!(second.state, "In Progress");
    assert_eq!(second.project_id, None);
    assert_eq!(second.project_slug, None);
    assert_eq!(second.project_name, None);
    assert_eq!(second.parent_id.as_deref(), Some("issue-254"));
    assert_eq!(second.sub_issues.len(), 2);
    assert_eq!(second.sub_issues[0].identifier, "COE-266");
    assert_eq!(second.sub_issues[0].state, "Done");
    assert_eq!(second.sub_issues[1].identifier, "COE-277");
    assert_eq!(second.sub_issues[1].state, "Todo");
    assert_eq!(second.blocked_by.len(), 1);
    assert_eq!(second.blocked_by[0].identifier, "COE-261");
    assert_eq!(
        second.blocked_by[0].state.kind,
        TrackerIssueStateKind::Started
    );
    assert_eq!(second.blocked_by[0].state.tracker_type, "started");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].authorization.as_deref(), Some("test-token"));
    assert_eq!(
        requests[0].body["variables"]["projectSlug"],
        Value::String("e7b957855cb7".to_string())
    );
    assert_eq!(
        requests[0].body["variables"]["stateNames"],
        serde_json::json!(["In Progress"])
    );
    assert_eq!(
        requests[0].body["variables"]["relationFirst"],
        serde_json::json!(10)
    );
    assert_eq!(
        requests[0].body["variables"]["labelFirst"],
        serde_json::json!(10)
    );
    assert_eq!(
        requests[0].body["variables"]["includeArchived"],
        Value::Bool(false)
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("includeArchived: $includeArchived")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("labels(first: $labelFirst)")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("children(includeArchived: true, first: 100)")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("parent {")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("project {")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("branchName")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("attachments {")
    );
}

#[tokio::test]
async fn configured_project_id_resolves_to_the_linear_project_slug_for_issue_queries() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(
            r#"{"data":{"projects":{"nodes":[{"id":"proj-open","name":"OpenSymphony","slugId":"e7b957855cb7","url":null,"content":null}]}}}"#,
        ),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.project_id = Some("proj-open".to_string());
    let client = LinearClient::new(config).expect("client configuration should be valid");

    client
        .candidate_issues()
        .await
        .expect("project-id candidate query should succeed");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["variables"]["id"], "proj-open");
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("project lookup query should be present")
            .contains("filter: { id: { eq: $id } }")
    );
    assert_eq!(requests[1].body["variables"]["projectSlug"], "e7b957855cb7");
}

#[tokio::test]
async fn unresolved_configured_project_id_fails_closed() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(
        r#"{"data":{"projects":{"nodes":[]}}}"#,
    )])
    .await;
    let mut config = test_config(server.base_url());
    config.project_id = Some("missing-project-id".to_string());
    config.project_slug = "missing-project-id".to_string();
    let client = LinearClient::new(config).expect("client configuration should be valid");

    let error = client
        .candidate_issues()
        .await
        .expect_err("an unresolved provider project ID should fail");
    assert!(
        matches!(error, LinearError::InvalidConfiguration(message) if message.contains("missing-project-id"))
    );
    assert_eq!(server.recorded_requests().await.len(), 1);
}

#[tokio::test]
async fn candidate_issue_summaries_use_lightweight_dispatch_query() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/candidate_issues_page.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issue_summaries()
        .await
        .expect("candidate summary query should succeed");

    assert_eq!(issues.len(), 2);
    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssueSummariesByState")
    );
    assert!(
        !requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("labels(first:")
    );
    assert!(
        !requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("projectMilestone")
    );
    assert!(requests[0].body["variables"].get("labelFirst").is_none());
}

#[tokio::test]
async fn candidate_issue_summaries_do_not_expand_relation_pages() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/candidate_issues_with_relation_paging.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issue_summaries()
        .await
        .expect("candidate summary query should succeed without relation expansion");

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].identifier, "COE-264");
    assert_eq!(server.recorded_requests().await.len(), 1);
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
    assert!(
        requests[1].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssueInverseRelationsPage")
    );
    assert!(
        requests[1].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("$issueId: String!")
    );
    assert_eq!(
        requests[1].body["variables"]["issueId"],
        Value::String("issue-264".to_string())
    );
    assert_eq!(
        requests[1].body["variables"]["after"],
        Value::String("relations-cursor-1".to_string())
    );
    assert_eq!(
        requests[0].body["variables"]["relationFirst"],
        serde_json::json!(10)
    );
    assert_eq!(
        requests[0].body["variables"]["labelFirst"],
        serde_json::json!(10)
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("labels(first: $labelFirst)")
    );
}

#[tokio::test]
async fn project_issues_by_identifiers_fetches_project_issue_details_in_one_query() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(project_issues_response(&[
        (
            "issue-260",
            "COE-260",
            "Domain model and orchestrator state machine",
        ),
        (
            "issue-264",
            "COE-264",
            "Linear read adapter and issue normalization",
        ),
    ]))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should work");

    let issues = client
        .project_issues_by_identifiers(&["COE-260", "COE-264"])
        .await
        .expect("identifier lookup should succeed");

    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].identifier, "COE-260");
    assert_eq!(issues[1].identifier, "COE-264");
    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body["variables"]["projectSlug"],
        serde_json::json!("e7b957855cb7")
    );
    assert_eq!(
        requests[0].body["variables"]["includeArchived"],
        serde_json::json!(false)
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query ProjectIssues")
    );
    assert!(
        !requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssueByIdentifier")
    );
}

#[tokio::test]
async fn project_task_graph_issues_return_requested_backlog_and_active_from_one_scan() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(
        project_issues_response_with_states(&[
            (
                "issue-260",
                "COE-260",
                "Domain model and orchestrator state machine",
                "In Progress",
                "started",
            ),
            (
                "issue-300",
                "COE-300",
                "Deferred backlog polish",
                "Backlog",
                "backlog",
            ),
            (
                "issue-310",
                "COE-310",
                "Unrequested todo issue",
                "Todo",
                "unstarted",
            ),
            (
                "issue-320",
                "COE-320",
                "Unrequested human review issue",
                "Human Review",
                "started",
            ),
            (
                "issue-330",
                "COE-330",
                "Parked unstarted issue",
                "Ready for Spec",
                "unstarted",
            ),
        ]),
    )])
    .await;
    let mut config = test_config(server.base_url());
    config.active_states = vec!["Todo".to_string(), "In Progress".to_string()];
    let client = LinearClient::new(config).expect("client configuration should work");

    let issues = client
        .project_task_graph_issues(&["COE-260"])
        .await
        .expect("task graph lookup should succeed");

    // Requested identifiers first, then unrequested backlog, dispatchable,
    // and started-kind issues. COE-310 is Todo but untracked by the control
    // plane (e.g. just promoted from Backlog, not yet dispatched) — dropping
    // it would make the issue vanish from every pane until the orchestrator
    // picks it up. COE-320 is in-flight (started kind) so it belongs in the
    // Current pane even though "Human Review" is not dispatchable. COE-330
    // sits in a parked unstarted state outside `active_states`, which the
    // scheduler will never dispatch, so it stays out of the task graph.
    assert_eq!(
        issues
            .iter()
            .map(|issue| issue.identifier.as_str())
            .collect::<Vec<_>>(),
        vec!["COE-260", "COE-300", "COE-310", "COE-320"],
    );
    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1, "one project scan serves both buckets");
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query ProjectIssues")
    );
}

#[tokio::test]
async fn project_task_graph_issues_resolve_out_of_project_ids_by_identifier() {
    // Project scan returns only COE-260. COE-999 is tracked but sits outside
    // the project (moved / no project metadata); it resolves via the
    // per-identifier fallback rather than failing the whole task graph.
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(project_issues_response_with_states(&[(
            "issue-260",
            "COE-260",
            "In-project issue",
            "In Progress",
            "started",
        )])),
        QueuedResponse::json(issue_by_identifier_response(
            "issue-999",
            "COE-999",
            "Out-of-project issue",
            "In Progress",
            "started",
        )),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should work");

    let issues = client
        .project_task_graph_issues(&["COE-260", "COE-999"])
        .await
        .expect("task graph lookup should resolve the out-of-project id");

    let identifiers = issues
        .iter()
        .map(|issue| issue.identifier.as_str())
        .collect::<Vec<_>>();
    assert!(identifiers.contains(&"COE-260"));
    assert!(identifiers.contains(&"COE-999"));
}

#[tokio::test]
async fn project_task_graph_issues_omit_unresolvable_ids_instead_of_failing() {
    // COE-404 is neither in the project scan nor resolvable by identifier
    // (deleted/inaccessible); it is omitted so the rest of the task graph
    // still renders instead of the endpoint 502-ing.
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(project_issues_response_with_states(&[(
            "issue-260",
            "COE-260",
            "In-project issue",
            "In Progress",
            "started",
        )])),
        QueuedResponse::json(issue_not_found_response()),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should work");

    let issues = client
        .project_task_graph_issues(&["COE-260", "COE-404"])
        .await
        .expect("task graph lookup should not fail on an unresolvable id");

    let identifiers = issues
        .iter()
        .map(|issue| issue.identifier.as_str())
        .collect::<Vec<_>>();
    assert_eq!(identifiers, vec!["COE-260"]);
}

#[tokio::test]
async fn candidate_issues_fetch_all_label_pages() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(include_str!(
            "fixtures/candidate_issues_with_label_paging.json"
        )),
        QueuedResponse::json(include_str!("fixtures/issue_labels_page_2.json")),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("candidate query should succeed");

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].identifier, "COE-260");
    assert_eq!(issues[0].labels, vec!["backend", "urgent"]);

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssueLabelsPage")
    );
    assert!(
        requests[1].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("$issueId: String!")
    );
    assert_eq!(
        requests[1].body["variables"]["issueId"],
        Value::String("issue-260".to_string())
    );
    assert_eq!(
        requests[1].body["variables"]["after"],
        Value::String("labels-cursor-1".to_string())
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
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssuesByState")
    );
    assert_eq!(
        requests[0].body["variables"]["relationFirst"],
        serde_json::json!(2)
    );
    assert_eq!(
        requests[0].body["variables"]["labelFirst"],
        serde_json::json!(2)
    );
    assert_eq!(
        requests[0].body["variables"]["includeArchived"],
        Value::Bool(false)
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("includeArchived: $includeArchived")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("labels(first: $labelFirst)")
    );
    assert_eq!(requests[0].body["variables"]["after"], Value::Null);
    assert_eq!(
        requests[1].body["variables"]["after"],
        Value::String("cursor-1".to_string())
    );
}

#[tokio::test]
async fn terminal_issues_include_archived_for_cleanup() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/candidate_issues_page.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .terminal_issues()
        .await
        .expect("terminal cleanup query should succeed");

    assert_eq!(issues.len(), 2);

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body["variables"]["stateNames"],
        serde_json::json!(["Done", "Canceled"])
    );
    assert_eq!(
        requests[0].body["variables"]["includeArchived"],
        Value::Bool(true)
    );
    assert_eq!(
        requests[0].body["variables"]["labelFirst"],
        serde_json::json!(10)
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("includeArchived: $includeArchived")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("labels(first: $labelFirst)")
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
    assert_eq!(snapshots[0].state.tracker_type, "completed");
    assert_eq!(snapshots[1].identifier, "COE-264");
    assert_eq!(snapshots[1].state.kind, TrackerIssueStateKind::Canceled);
    assert_eq!(snapshots[1].state.tracker_type, "canceled");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssueStatesByIds")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("includeArchived: true")
    );
    assert_eq!(
        requests[0].body["variables"]["issueIds"],
        serde_json::json!(["issue-260", "issue-264"])
    );
    assert_eq!(
        requests[0].body["variables"]["projectSlug"],
        Value::String("e7b957855cb7".to_string())
    );
}

#[tokio::test]
async fn issue_states_by_ids_omits_missing_ids_for_cross_project_recovery() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(include_str!(
        "fixtures/issue_states_missing_id.json"
    ))])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let snapshots = client
        .issue_states_by_ids(&["issue-260".to_string(), "issue-264".to_string()])
        .await
        .expect("missing issue ids should be ignored during recovery reconciliation");

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].identifier, "COE-260");
}

#[tokio::test]
async fn fetch_workpad_comment_returns_latest_active_workpad_comment() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::json(
            r###"{
              "data": {
                "issue": {
                  "id": "issue-260",
                  "comments": {
                    "nodes": [
                      {
                        "id": "comment-ignore",
                        "body": "Ordinary issue comment",
                        "updatedAt": "2026-03-25T22:00:00.000Z",
                        "resolvedAt": null
                      },
                      {
                        "id": "comment-resolved",
                        "body": "## Agent Harness Workpad\n\nResolved progress",
                        "updatedAt": "2026-03-25T22:01:00.000Z",
                        "resolvedAt": "2026-03-25T22:02:00.000Z"
                      },
                      {
                        "id": "comment-old-workpad",
                        "body": "## Agent Harness Workpad\n\nOlder progress",
                        "updatedAt": "2026-03-25T22:03:00.000Z",
                        "resolvedAt": null
                      }
                    ],
                    "pageInfo": {
                      "hasNextPage": true,
                      "endCursor": "comments-cursor-1"
                    }
                  }
                }
              }
            }"###,
        ),
        QueuedResponse::json(
            r###"{
              "data": {
                "issue": {
                  "id": "issue-260",
                  "comments": {
                    "nodes": [
                      {
                        "id": "comment-latest-workpad",
                        "body": "## Agent Harness Workpad\n\nLatest active progress",
                        "updatedAt": "2026-03-25T22:05:00.000Z",
                        "resolvedAt": null
                      }
                    ],
                    "pageInfo": {
                      "hasNextPage": false,
                      "endCursor": null
                    }
                  }
                }
              }
            }"###,
        ),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let comment = client
        .fetch_workpad_comment("issue-260")
        .await
        .expect("workpad lookup should succeed")
        .expect("workpad comment should be found");

    assert_eq!(comment.id, "comment-latest-workpad");
    assert!(comment.body.contains("Latest active progress"));

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("query IssueCommentsPage")
    );
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("$issueId: String!")
    );
    assert_eq!(
        requests[0].body["variables"]["issueId"],
        Value::String("issue-260".to_string())
    );
    assert_eq!(
        requests[1].body["variables"]["after"],
        Value::String("comments-cursor-1".to_string())
    );
}

#[tokio::test]
async fn fetch_workpad_comment_returns_none_when_no_active_marker_exists() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(
        r###"{
          "data": {
            "issue": {
              "id": "issue-260",
              "comments": {
                "nodes": [
                  {
                    "id": "comment-ignore",
                    "body": "Ordinary issue comment",
                    "updatedAt": "2026-03-25T22:00:00.000Z",
                    "resolvedAt": null
                  },
                  {
                    "id": "comment-resolved",
                    "body": "## Agent Harness Workpad\n\nResolved progress",
                    "updatedAt": "2026-03-25T22:01:00.000Z",
                    "resolvedAt": "2026-03-25T22:02:00.000Z"
                  }
                ],
                "pageInfo": {
                  "hasNextPage": false,
                  "endCursor": null
                }
              }
            }
          }
        }"###,
    )])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let comment = client
        .fetch_workpad_comment("issue-260")
        .await
        .expect("workpad lookup should succeed");

    assert!(comment.is_none());
}

#[test]
fn client_configuration_requires_active_states() {
    let mut config = LinearConfig::new("test-token", "e7b957855cb7");
    config.terminal_states = vec!["Done".to_string()];

    let error = match LinearClient::new(config) {
        Ok(_) => panic!("missing active states should fail"),
        Err(error) => error,
    };

    match error {
        LinearError::InvalidConfiguration(message) => {
            assert!(message.contains("tracker.active_states"));
        }
        other => panic!("expected invalid configuration error, got {other:?}"),
    }
}

#[test]
fn client_configuration_requires_terminal_states() {
    let mut config = LinearConfig::new("test-token", "e7b957855cb7");
    config.active_states = vec!["In Progress".to_string()];
    config.terminal_states = vec![" ".to_string()];

    let error = match LinearClient::new(config) {
        Ok(_) => panic!("blank terminal states should fail"),
        Err(error) => error,
    };

    match error {
        LinearError::InvalidConfiguration(message) => {
            assert!(message.contains("tracker.terminal_states"));
        }
        other => panic!("expected invalid configuration error, got {other:?}"),
    }
}

#[test]
fn client_configuration_requires_project_slug() {
    let mut config = LinearConfig::new("test-token", "   ");
    config.active_states = vec!["In Progress".to_string()];
    config.terminal_states = vec!["Done".to_string()];

    let error = match LinearClient::new(config) {
        Ok(_) => panic!("blank project slug should fail"),
        Err(error) => error,
    };

    match error {
        LinearError::InvalidConfiguration(message) => {
            assert!(message.contains("tracker.project_slug"));
        }
        other => panic!("expected invalid configuration error, got {other:?}"),
    }
}

#[test]
fn client_configuration_requires_api_key() {
    let mut config = LinearConfig::new("   ", "e7b957855cb7");
    config.active_states = vec!["In Progress".to_string()];
    config.terminal_states = vec!["Done".to_string()];

    let error = match LinearClient::new(config) {
        Ok(_) => panic!("blank api key should fail"),
        Err(error) => error,
    };

    match error {
        LinearError::InvalidConfiguration(message) => {
            assert!(message.contains("LINEAR_API_KEY"));
        }
        other => panic!("expected invalid configuration error, got {other:?}"),
    }
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
async fn rate_limited_429_with_long_retry_after_returns_immediately() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\":\"rate limited\"}",
        )
        .with_header("retry-after", "3600"),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.retry_policy.max_backoff = Duration::from_secs(5);
    let client = LinearClient::new(config).expect("client configuration should be valid");
    let start = tokio::time::Instant::now();

    let error = client
        .candidate_issues()
        .await
        .expect_err("long retry-after should return without sleeping");

    assert!(error.is_rate_limited());
    assert_eq!(server.recorded_requests().await.len(), 1);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "long retry-after should not be slept inside the Linear client"
    );
}

#[tokio::test]
async fn rate_limited_429_above_inline_cap_returns_immediately() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::TOO_MANY_REQUESTS,
            "{\"error\":\"rate limited\"}",
        )
        .with_header("retry-after", "60"),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.retry_policy.max_backoff = Duration::from_secs(120);
    let client = LinearClient::new(config).expect("client configuration should be valid");
    let start = tokio::time::Instant::now();

    let error = client
        .candidate_issues()
        .await
        .expect_err("retry-after above inline cap should return without sleeping");

    assert!(error.is_rate_limited());
    assert_eq!(server.recorded_requests().await.len(), 1);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "retry-after above inline cap should not be slept inside the Linear client"
    );
}

#[tokio::test]
async fn graphql_rate_limited_bad_request_retries() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::BAD_REQUEST,
            r#"{"errors":[{"message":"rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#,
        )
        .with_header("content-type", "application/json")
        .with_header("retry-after", "0"),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("GraphQL rate-limited requests should retry");

    assert_eq!(issues.len(), 2);
    assert_eq!(server.recorded_requests().await.len(), 2);
}

#[tokio::test]
async fn graphql_server_error_envelope_retries() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"errors":[{"message":"temporary upstream failure"}]}"#,
        )
        .with_header("content-type", "application/json"),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("5xx GraphQL envelopes should stay retryable");

    assert_eq!(issues.len(), 2);
    assert_eq!(server.recorded_requests().await.len(), 2);
}

#[tokio::test]
async fn graphql_rate_limited_bad_request_retries_using_reset_header() {
    let reset_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time should be after epoch")
        .as_millis()
        .to_string();
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::BAD_REQUEST,
            r#"{"errors":[{"message":"rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#,
        )
        .with_header("content-type", "application/json")
        .with_header("x-ratelimit-requests-reset", &reset_ms),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.retry_policy.initial_backoff = Duration::from_secs(5);
    config.retry_policy.max_backoff = Duration::from_secs(5);
    let client = LinearClient::new(config).expect("client configuration should be valid");
    let start = tokio::time::Instant::now();

    let issues = client
        .candidate_issues()
        .await
        .expect("GraphQL rate-limited requests should honor reset headers");

    assert_eq!(issues.len(), 2);
    assert_eq!(server.recorded_requests().await.len(), 2);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "retry should use the reset header instead of the exponential backoff"
    );
}

#[tokio::test]
async fn graphql_rate_limited_bad_request_with_long_reset_returns_immediately() {
    let reset_ms = (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time should be after epoch")
        + Duration::from_secs(60 * 60))
    .as_millis()
    .to_string();
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::BAD_REQUEST,
            r#"{"errors":[{"message":"rate limit exceeded","extensions":{"code":"RATELIMITED"}}]}"#,
        )
        .with_header("content-type", "application/json")
        .with_header("x-ratelimit-requests-reset", &reset_ms),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.retry_policy.initial_backoff = Duration::from_secs(5);
    config.retry_policy.max_backoff = Duration::from_secs(5);
    let client = LinearClient::new(config).expect("client configuration should be valid");
    let start = tokio::time::Instant::now();

    let error = client
        .candidate_issues()
        .await
        .expect_err("long reset should return a rate-limit error without sleeping");

    assert!(error.is_rate_limited());
    assert!(
        error
            .retry_after()
            .is_some_and(|delay| delay > Duration::from_secs(5))
    );
    assert_eq!(server.recorded_requests().await.len(), 1);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "long reset header should not be slept inside the Linear client"
    );
}

#[tokio::test]
async fn graphql_internal_server_error_retries_even_with_graphql_envelope() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(
            StatusCode::BAD_GATEWAY,
            r#"{"errors":[{"message":"temporary upstream failure","extensions":{"code":"INTERNAL_SERVER_ERROR"}}]}"#,
        )
        .with_header("content-type", "application/json"),
        QueuedResponse::json(include_str!("fixtures/candidate_issues_page.json")),
    ])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should be valid");

    let issues = client
        .candidate_issues()
        .await
        .expect("5xx GraphQL envelopes should stay retryable");

    assert_eq!(issues.len(), 2);
    assert_eq!(server.recorded_requests().await.len(), 2);
}

#[tokio::test]
#[ignore = "requires LINEAR_API_KEY and live Linear access"]
async fn live_linear_client_reads_opensymphony_project() {
    let api_key = std::env::var("LINEAR_API_KEY").expect("LINEAR_API_KEY must be set");
    let mut config = LinearConfig::new(api_key, "e7b957855cb7");
    config.active_states = vec![
        "Todo".to_string(),
        "In Progress".to_string(),
        "Human Review".to_string(),
        "Rework".to_string(),
    ];
    config.terminal_states = vec!["Done".to_string(), "Canceled".to_string()];
    let client = LinearClient::new(config).expect("live client configuration should be valid");

    let summaries = client
        .candidate_issue_summaries()
        .await
        .expect("live summary query should succeed");
    println!("live candidate summaries fetched: {}", summaries.len());

    let issues = client
        .project_issues_by_identifiers(&["COE-504"])
        .await
        .expect("live project-scoped identifier lookup should succeed");
    let issue = issues
        .iter()
        .find(|issue| issue.identifier == "COE-504")
        .expect("COE-504 should be visible through the live project detail path");
    println!(
        "live issue detail: {} {} {}",
        issue.identifier, issue.state, issue.title
    );
    assert_eq!(issue.identifier, "COE-504");
    assert_eq!(issue.state, "Done");
}

#[tokio::test]
async fn invalid_json_response_includes_operation_and_response_metadata() {
    let server = MockGraphqlServer::start(vec![
        QueuedResponse::new(StatusCode::OK, "<html>not graphql json</html>")
            .with_header("content-type", "text/html")
            .with_header("content-length", "29"),
    ])
    .await;
    let mut config = test_config(server.base_url());
    config.retry_policy.max_attempts = 1;
    let client = LinearClient::new(config).expect("client configuration should be valid");

    let error = client
        .candidate_issues()
        .await
        .expect_err("invalid JSON response should fail");
    let rendered = error.to_string();

    assert!(matches!(
        error.category(),
        TrackerErrorCategory::InvalidResponse
    ));
    assert!(
        rendered.contains("IssuesByState"),
        "error should name the GraphQL operation: {rendered}"
    );
    assert!(
        rendered.contains("HTTP 200 OK"),
        "error should include the HTTP status: {rendered}"
    );
    assert!(
        rendered.contains("content-type=text/html"),
        "error should include safe response metadata: {rendered}"
    );
    assert!(
        rendered.contains("body_bytes="),
        "error should include response length without logging the body: {rendered}"
    );
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

#[tokio::test]
async fn archive_issue_uses_issue_archive_mutation() {
    let server = MockGraphqlServer::start(vec![QueuedResponse::json(
        r#"{"data":{"issueArchive":{"success":true}}}"#,
    )])
    .await;
    let client = LinearClient::new(test_config(server.base_url()))
        .expect("client configuration should work");

    client
        .archive_issue("COE-123")
        .await
        .expect("archive mutation should succeed");

    let requests = server.recorded_requests().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body["variables"]["id"], "COE-123");
    assert_eq!(requests[0].body["variables"]["trash"], false);
    assert!(
        requests[0].body["query"]
            .as_str()
            .expect("query should be a string")
            .contains("issueArchive")
    );
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

fn project_issues_response(issues: &[(&str, &str, &str)]) -> String {
    project_issues_response_with_states(
        &issues
            .iter()
            .map(|(issue_id, identifier, title)| {
                (*issue_id, *identifier, *title, "Done", "completed")
            })
            .collect::<Vec<_>>(),
    )
}

fn project_issues_response_with_states(issues: &[(&str, &str, &str, &str, &str)]) -> String {
    let nodes = issues
        .iter()
        .map(|(issue_id, identifier, title, state_name, state_type)| {
            format!(
                r#"{{
      "id": "{issue_id}",
      "identifier": "{identifier}",
      "url": "https://linear.app/example/issue/{identifier}",
      "title": "{title}",
      "description": "Issue looked up by identifier.",
      "priority": 0.0,
      "createdAt": "2026-03-20T10:00:00Z",
      "updatedAt": "2026-03-21T12:00:00Z",
      "state": {{
        "id": "state-{identifier}",
        "name": "{state_name}",
        "type": "{state_type}"
      }},
      "parent": null,
      "children": {{
        "nodes": []
      }},
      "labels": {{
        "nodes": [],
        "pageInfo": {{
          "hasNextPage": false,
          "endCursor": null
        }}
      }},
      "inverseRelations": {{
        "nodes": [],
        "pageInfo": {{
          "hasNextPage": false,
          "endCursor": null
        }}
      }}
    }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"{{
  "data": {{
    "issues": {{
      "nodes": [
        {nodes}
      ],
      "pageInfo": {{
        "hasNextPage": false,
        "endCursor": null
      }}
    }}
  }}
}}"#
    )
}

fn issue_by_identifier_response(
    issue_id: &str,
    identifier: &str,
    title: &str,
    state_name: &str,
    state_type: &str,
) -> String {
    format!(
        r#"{{
  "data": {{
    "issue": {{
      "id": "{issue_id}",
      "identifier": "{identifier}",
      "url": "https://linear.app/example/issue/{identifier}",
      "title": "{title}",
      "description": "Issue resolved by identifier fallback.",
      "priority": 0.0,
      "createdAt": "2026-03-20T10:00:00Z",
      "updatedAt": "2026-03-21T12:00:00Z",
      "state": {{ "id": "state-{identifier}", "name": "{state_name}", "type": "{state_type}" }},
      "parent": null,
      "children": {{ "nodes": [] }},
      "labels": {{ "nodes": [], "pageInfo": {{ "hasNextPage": false, "endCursor": null }} }},
      "inverseRelations": {{ "nodes": [], "pageInfo": {{ "hasNextPage": false, "endCursor": null }} }}
    }}
  }}
}}"#
    )
}

fn issue_not_found_response() -> String {
    r#"{"errors":[{"message":"Entity not found: Issue","extensions":{"code":"INPUT_ERROR","statusCode":400,"type":"invalid input","userError":true,"userPresentableMessage":"Could not find referenced Issue."},"path":["issue"]}],"data":{"issue":null}}"#.to_string()
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
