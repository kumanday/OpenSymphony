use futures_util::future::join_all;
use opensymphony::opensymphony_gateway_schema::approval::{
    OperatorAnswer, OperatorInteractionKind, OperatorQuestionAnswer,
};
use opensymphony::opensymphony_workflow::AcpPermissionPolicy;
use opensymphony::{opensymphony_acp::*, opensymphony_workspace::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[test]
fn retained_source_history_distinguishes_old_eviction_from_current_loss() {
    let source = |sequence| SessionEvent::Source {
        generation: 7,
        run_id: "current-run".into(),
        replay: false,
        frame: SourceFrame {
            sequence,
            direction: "incoming".into(),
            observed_at: chrono::Utc::now(),
            payload: serde_json::json!({"method":"session/update"}),
        },
    };
    let history = SourceHistory {
        events: (129..=140).map(source).collect(),
        truncated: true,
        latest_cursor: Some((7, 140)),
    };
    assert!(
        history.covers_since((7, 128)),
        "ancient eviction is harmless"
    );
    assert!(
        history.covers_since((7, 130)),
        "processed frames may remain in history"
    );
    assert!(
        !history.covers_since((7, 127)),
        "missing current frame must fence"
    );

    let oversized_tail = SourceHistory {
        latest_cursor: Some((7, 141)),
        ..history
    };
    assert!(
        !oversized_tail.covers_since((7, 140)),
        "a dropped oversized frame must fence even without later frames"
    );
}

async fn launch(root: &Path, issue: &str, mode: &str) -> SessionLaunch {
    let root = root.canonicalize().expect("root");
    let manager = WorkspaceManager::new(WorkspaceManagerConfig {
        root: root.clone(),
        hooks: HookConfig::default(),
        cleanup: CleanupConfig::default(),
    })
    .expect("manager");
    let workspace = manager
        .ensure(&IssueDescriptor {
            issue_id: issue.into(),
            identifier: issue.into(),
            title: issue.into(),
            current_state: "In Progress".into(),
            last_seen_tracker_refresh_at: None,
            repository_binding: None,
        })
        .await
        .expect("ensure")
        .handle;
    let identity = AcpSessionIdentity {
        profile_id: "test".into(),
        profile_fingerprint: String::new(),
        credential_scope: "grant-generation-1".into(),
        workspace_path: workspace.workspace_path().into(),
        repository_binding: None,
        checkout_generation: workspace.checkout_generation().map(str::to_owned),
        generation: 0,
        run_id: "first".into(),
        attempt: 1,
    };
    SessionLaunch {
        context: LaunchContext {
            workspace_root: root,
            workspace_key: workspace.workspace_key().into(),
            issue_workspace: workspace.workspace_path().into(),
            environment: BTreeMap::from([("PATH".into(), std::env::var("PATH").expect("PATH"))]),
            excluded_environment: BTreeSet::new(),
            services: Default::default(),
        },
        manager,
        workspace,
        identity,
        profile: AcpProfile {
            permissions: Default::default(),
            command: "python3".into(),
            args: vec![
                format!(
                    "{}/tests/fixtures/acp_session_peer.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                mode.into(),
            ],
            transport: "stdio".into(),
            protocol_versions: vec![1],
            env_refs: BTreeMap::new(),
            auth: None,
            required_capabilities: vec![],
            extensions: vec![],
            session: Default::default(),
        },
        limits: ClientLimits {
            setup_timeout: Duration::from_secs(3),
            prompt_timeout: Duration::from_secs(3),
            cancel_timeout: Duration::from_secs(1),
            ..ClientLimits::default()
        },
        require_persistence: false,
        expected_session_id: None,
    }
}
async fn retire(handle: &SessionHandle) {
    handle
        .control(SessionControl::Retire)
        .await
        .expect("quiescent retirement");
}

#[tokio::test]
async fn registered_outbound_operation_binds_session_and_preserves_metadata() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "EXTENSION-ECHO", "extension_echo").await;
    request.profile.extensions.push("fixture_echo@1".into());
    let handle = host.open(request).await.expect("open");
    let initial = handle.inspect().await.expect("inspect");
    assert_eq!(initial.state.enabled_operations.len(), 1);
    let handle_for_prompt = handle.clone();
    let prompt = tokio::spawn(async move {
        handle_for_prompt
            .prompt(
                "echo-run".into(),
                1,
                "extension-echo".into(),
                CancellationToken::new(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.path().join("EXTENSION-ECHO/extension-ready").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer ready");
    assert!(matches!(
        handle
            .operation(
                "wrong-run".into(),
                "fixture.echo".into(),
                serde_json::json!({"value":"hello"})
            )
            .await,
        Err(HostError::IdentityMismatch)
    ));
    assert!(
        handle
            .operation(
                "echo-run".into(),
                "other".into(),
                serde_json::json!({"value":"hello"})
            )
            .await
            .is_err()
    );
    assert!(
        handle
            .operation(
                "echo-run".into(),
                "fixture.echo".into(),
                serde_json::json!({"value":"hello","method":"unsafe"})
            )
            .await
            .is_err()
    );
    let result = handle
        .operation(
            "echo-run".into(),
            "fixture.echo".into(),
            serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}}),
        )
        .await
        .expect("registered operation");
    assert_eq!(
        result,
        serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}})
    );
    assert!(
        prompt
            .await
            .expect("prompt task")
            .expect("prompt")
            .succeeded()
    );
    retire(&handle).await;
}

#[tokio::test]
async fn outbound_response_preceding_prompt_completion_is_never_lost() {
    // The peer writes the operation result and prompt completion back-to-back.
    // Repetition catches scheduling differences between SDK waiter delivery
    // and the retained host's prompt-completion branch.
    for trial in 0..24 {
        let root = tempfile::tempdir().expect("temp");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        let mut request = launch(root.path(), "EXTENSION-RACE", "extension_echo").await;
        request.profile.extensions.push("fixture_echo@1".into());
        let handle = host.open(request).await.expect("open");
        let prompt_handle = handle.clone();
        let prompt = tokio::spawn(async move {
            prompt_handle
                .prompt(
                    "echo-race-run".into(),
                    1,
                    "extension-echo".into(),
                    CancellationToken::new(),
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while !root.path().join("EXTENSION-RACE/extension-ready").exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("peer ready");
        let result = handle
            .operation(
                "echo-race-run".into(),
                "fixture.echo".into(),
                serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}}),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("trial {trial} lost correlated operation result: {error}")
            });
        assert_eq!(result["value"], "hello", "trial {trial}");
        assert!(
            prompt
                .await
                .expect("prompt task")
                .expect("prompt")
                .succeeded()
        );
        retire(&handle).await;
    }
}

#[tokio::test]
async fn outbound_operation_redacts_known_secrets_in_value_and_metadata() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "EXTENSION-SECRET", "extension_echo_secret").await;
    request.profile.extensions.push("fixture_echo@1".into());
    request.context.environment.insert(
        "ACCESS_TOKEN".into(),
        "synthetic-operation-secret-613".into(),
    );
    let handle = host.open(request).await.expect("open");
    let prompt_handle = handle.clone();
    let prompt = tokio::spawn(async move {
        prompt_handle
            .prompt(
                "echo-secret-run".into(),
                1,
                "extension-echo".into(),
                CancellationToken::new(),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root
            .path()
            .join("EXTENSION-SECRET/extension-ready")
            .exists()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer ready");
    let result = handle
        .operation(
            "echo-secret-run".into(),
            "fixture.echo".into(),
            serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}}),
        )
        .await
        .expect("registered operation");
    assert_eq!(
        result,
        serde_json::json!({"value":"[redacted]","_meta":{"traceparent":"[redacted]"}})
    );
    assert!(
        !serde_json::to_string(&result)
            .expect("result")
            .contains("synthetic-operation-secret-613")
    );
    assert!(
        prompt
            .await
            .expect("prompt task")
            .expect("prompt")
            .succeeded()
    );
    retire(&handle).await;
}

#[tokio::test]
async fn outbound_operation_timeout_reports_unknown_without_retry() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "EXTENSION-TIMEOUT", "extension_echo_timeout").await;
    request.profile.extensions.push("fixture_echo@1".into());
    request.limits.prompt_timeout = Duration::from_secs(8);
    let handle = host.open(request).await.expect("open");
    let cancellation = CancellationToken::new();
    let handle_for_prompt = handle.clone();
    let prompt_cancel = cancellation.clone();
    let prompt = tokio::spawn(async move {
        handle_for_prompt
            .prompt(
                "echo-timeout-run".into(),
                1,
                "extension-echo".into(),
                prompt_cancel,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root
            .path()
            .join("EXTENSION-TIMEOUT/extension-ready")
            .exists()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer ready");
    let result = handle
        .operation(
            "echo-timeout-run".into(),
            "fixture.echo".into(),
            serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}}),
        )
        .await;
    assert!(
        matches!(result, Err(HostError::Client(message)) if message.contains("outcome is unknown"))
    );
    let count = std::fs::read_to_string(
        root.path()
            .join("EXTENSION-TIMEOUT/extension-request-count"),
    )
    .expect("request marker");
    assert_eq!(
        count.lines().count(),
        1,
        "deadline must not retry a request"
    );
    cancellation.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(3), prompt)
        .await
        .expect("prompt completion");
}

#[tokio::test]
async fn outbound_operation_inflight_limit_rejects_ninth_request() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "EXTENSION-LIMIT", "extension_echo_timeout").await;
    request.profile.extensions.push("fixture_echo@1".into());
    request.limits.prompt_timeout = Duration::from_secs(8);
    let handle = host.open(request).await.expect("open");
    let cancellation = CancellationToken::new();
    let prompt_handle = handle.clone();
    let prompt_cancel = cancellation.clone();
    let prompt = tokio::spawn(async move {
        prompt_handle
            .prompt(
                "echo-limit-run".into(),
                1,
                "extension-echo".into(),
                prompt_cancel,
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.path().join("EXTENSION-LIMIT/extension-ready").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer ready");
    let results = join_all((0..9).map(|_| {
        handle.operation(
            "echo-limit-run".into(),
            "fixture.echo".into(),
            serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}}),
        )
    }))
    .await;
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(HostError::ResourceLimit)))
            .count(),
        1
    );
    assert_eq!(results.iter().filter(|result| matches!(result, Err(HostError::Client(message)) if message.contains("outcome is unknown"))).count(), 8);
    let count =
        std::fs::read_to_string(root.path().join("EXTENSION-LIMIT/extension-request-count"))
            .expect("request marker");
    assert_eq!(count.lines().count(), 8);
    let later = join_all((0..8).map(|_| {
        handle.operation(
            "echo-limit-run".into(),
            "fixture.echo".into(),
            serde_json::json!({"value":"hello","_meta":{"traceparent":"trace-echo"}}),
        )
    }))
    .await;
    assert!(
        later
            .iter()
            .all(|result| matches!(result, Err(HostError::ResourceLimit)))
    );
    let count =
        std::fs::read_to_string(root.path().join("EXTENSION-LIMIT/extension-request-count"))
            .expect("request marker");
    assert_eq!(
        count.lines().count(),
        8,
        "timed-out SDK replies still consume permits"
    );
    cancellation.cancel();
    let _ = tokio::time::timeout(Duration::from_secs(3), prompt)
        .await
        .expect("prompt completion");
}

#[tokio::test]
#[ignore = "Cursor workflow registration awaits an authenticated pinned callback capture"]
async fn cursor_request_id_zero_uses_vendor_result_and_notification_has_no_response() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "CURSOR-EXTENSION", "none").await;
    request
        .profile
        .extensions
        .push("cursor@2026.09.08-6caf4ff".into());
    let handle = host.open(request).await.expect("open");
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let handle_for_prompt = handle.clone();
    let prompt = tokio::spawn(async move {
        handle_for_prompt
            .prompt_with_operator(
                "cursor-run".into(),
                1,
                "cursor-extension".into(),
                CancellationToken::new(),
                Some(tx),
            )
            .await
    });
    let opened = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(AcpOperatorEvent::Opened(opened)) = rx.recv().await {
                break opened;
            }
        }
    })
    .await
    .expect("operator event");
    assert_eq!(opened.interaction.kind, OperatorInteractionKind::Question);
    let (acknowledgement, delivered) = tokio::sync::oneshot::channel();
    assert!(
        opened
            .reply
            .send(AcpOperatorReply {
                answer: OperatorAnswer::Question {
                    answers: vec![OperatorQuestionAnswer {
                        question_id: "mode".into(),
                        selected_option_ids: vec!["plan".into()],
                    }],
                },
                acknowledgement,
                delivery: AcpOperatorDeliveryFence::default(),
            })
            .is_ok()
    );
    assert!(delivered.await.expect("acknowledged"));
    assert!(
        prompt
            .await
            .expect("prompt task")
            .expect("prompt")
            .succeeded()
    );
    let methods =
        std::fs::read_to_string(root.path().join("CURSOR-EXTENSION/methods")).expect("method log");
    assert!(
        !methods.contains("None"),
        "notifications must not prompt a peer response"
    );
    retire(&handle).await;
}
async fn prompt(handle: &SessionHandle, run: &str, text: &str) -> TurnReport {
    handle
        .prompt(run.into(), 1, text.into(), CancellationToken::new())
        .await
        .expect("prompt")
}

#[tokio::test]
async fn session_prompt_without_operator_route_cancels_form_and_completes() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(root.path(), "ISSUE-612-NO-ROUTE", "none").await)
        .await
        .expect("open");
    assert!(
        prompt(&handle, "run-no-route", "form-no-route")
            .await
            .succeeded()
    );
    retire(&handle).await;
}

#[tokio::test]
async fn automatic_permission_policy_is_fenced_by_prompt_epoch() {
    for routed in [false, true] {
        let root = tempfile::tempdir().expect("temp");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        let mut request = launch(root.path(), "PERMISSION-EPOCH", "none").await;
        request.profile.permissions.mode = AcpPermissionPolicy::AllowOnce;
        let handle = host.open(request).await.expect("open");
        let report = if routed {
            let (tx, mut rx) = tokio::sync::mpsc::channel(2);
            let report = handle
                .prompt_with_operator(
                    "policy-route".into(),
                    1,
                    "permission-epoch".into(),
                    CancellationToken::new(),
                    Some(tx),
                )
                .await
                .expect("prompt");
            assert!(rx.try_recv().is_err(), "automatic decisions must not route");
            report
        } else {
            prompt(&handle, "policy-direct", "permission-epoch").await
        };
        assert!(report.succeeded());
        let marker = root.path().join("PERMISSION-EPOCH/permission-epoch.json");
        tokio::time::timeout(Duration::from_secs(3), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("peer observed both permission decisions");
        let decisions: serde_json::Value =
            serde_json::from_slice(&std::fs::read(marker).expect("marker")).expect("decisions");
        assert_eq!(
            decisions["active"],
            serde_json::json!({"outcome":{"outcome":"selected","optionId":"allow-opaque"}})
        );
        assert_eq!(
            decisions["late"],
            serde_json::json!({"outcome":{"outcome":"cancelled"}})
        );
        retire(&handle).await;
    }
}

#[tokio::test]
async fn saturated_operator_event_channel_delivers_callback_closures() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "CLOSE-SATURATION", "none").await;
    request.limits.callback_timeout = Duration::from_millis(100);
    request.limits.prompt_timeout = Duration::from_secs(8);
    let handle = host.open(request).await.expect("open");
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    let prompt = tokio::spawn({
        let handle = handle.clone();
        async move {
            handle
                .prompt_with_operator(
                    "close-saturation".into(),
                    1,
                    "operator-close-saturation".into(),
                    CancellationToken::new(),
                    Some(tx),
                )
                .await
                .expect("prompt")
        }
    });
    let workspace = root.path().join("CLOSE-SATURATION");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !workspace.join("operator-close-timed-out").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("peer observed both timeout cancellations");
    assert!(
        !prompt.is_finished(),
        "prompt remains active during cleanup"
    );
    assert_eq!(rx.len(), 2, "both Opened events saturate the channel");
    let mut opened = BTreeSet::new();
    let mut closed = BTreeSet::new();
    for _ in 0..4 {
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("every callback closure must arrive")
            .expect("operator event");
        match event {
            AcpOperatorEvent::Opened(request) => {
                opened.insert(request.interaction.request_id);
            }
            AcpOperatorEvent::Closed(request_id) => {
                closed.insert(request_id);
            }
        }
    }
    assert_eq!(opened.len(), 2);
    assert_eq!(closed, opened);
    std::fs::write(workspace.join("release-operator-close-prompt"), b"").expect("release peer");
    assert!(prompt.await.expect("prompt task").succeeded());
    retire(&handle).await;
}

#[tokio::test]
async fn native_peer_operator_permission_and_form_question_round_trip() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(root.path(), "ISSUE-612", "none").await)
        .await
        .expect("open");
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let prompt = tokio::spawn({
        let handle = handle.clone();
        async move {
            handle
                .prompt_with_operator(
                    "run-612".into(),
                    1,
                    "operator-roundtrip".into(),
                    CancellationToken::new(),
                    Some(tx),
                )
                .await
                .expect("prompt")
        }
    });
    let mut observed = Vec::new();
    while observed.len() < 2 {
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("operator request timeout")
            .expect("operator request");
        let AcpOperatorEvent::Opened(request) = event else {
            continue;
        };
        assert!(uuid::Uuid::parse_str(&request.interaction.rpc_id).is_ok());
        assert!(!request.interaction.rpc_id.contains("9007199254740993"));
        observed.push(request.interaction.kind);
        let answer = match request.interaction.kind {
            OperatorInteractionKind::Permission => OperatorAnswer::Permission {
                option_id: "allow-opaque".into(),
            },
            OperatorInteractionKind::Question => OperatorAnswer::Question {
                answers: vec![OperatorQuestionAnswer {
                    question_id: "region".into(),
                    selected_option_ids: vec!["west".into()],
                }],
            },
            OperatorInteractionKind::PlanApproval => panic!("vendor plan callback is not enabled"),
        };
        let (acknowledgement, delivered) = tokio::sync::oneshot::channel();
        assert!(
            request
                .reply
                .send(AcpOperatorReply {
                    answer,
                    acknowledgement,
                    delivery: AcpOperatorDeliveryFence::default(),
                })
                .is_ok()
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), delivered)
                .await
                .expect("callback acknowledgement")
                .expect("acknowledgement")
        );
    }
    let report = tokio::time::timeout(Duration::from_secs(5), prompt)
        .await
        .expect("native prompt timeout")
        .expect("task");
    assert!(report.succeeded());
    assert!(
        run_capability(&handle.inspect().await.expect("run capability").state).operator_responses
    );
    assert_eq!(
        observed,
        vec![
            OperatorInteractionKind::Permission,
            OperatorInteractionKind::Question
        ]
    );
    for (run, text, answer) in [
        ("run-decline", "operator-decline", OperatorAnswer::Decline),
        ("run-cancel", "operator-cancel", OperatorAnswer::Cancel),
    ] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let request = tokio::spawn({
            let handle = handle.clone();
            async move {
                handle
                    .prompt_with_operator(
                        run.into(),
                        1,
                        text.into(),
                        CancellationToken::new(),
                        Some(tx),
                    )
                    .await
                    .expect("form outcome prompt")
            }
        });
        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("form outcome timeout")
            .expect("form outcome event");
        let AcpOperatorEvent::Opened(interaction) = event else {
            panic!("expected form request");
        };
        assert_eq!(
            interaction.interaction.kind,
            OperatorInteractionKind::Question
        );
        let (acknowledgement, delivered) = tokio::sync::oneshot::channel();
        assert!(
            interaction
                .reply
                .send(AcpOperatorReply {
                    answer,
                    acknowledgement,
                    delivery: AcpOperatorDeliveryFence::default(),
                })
                .is_ok()
        );
        assert!(delivered.await.expect("form acknowledgement"));
        assert!(request.await.expect("form task").succeeded());
    }
    retire(&handle).await;
}

#[tokio::test]
async fn future_stop_reason_is_durable_terminal_evidence() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(root.path(), "ISSUE-1", "none").await)
        .await
        .expect("open");
    let report = prompt(&handle, "future-run", "unknown-stop").await;
    assert_eq!(report.stop_reason, "future_stop_reason");
    assert!(!report.succeeded());
    let snapshot = handle.inspect().await.expect("inspect");
    assert_eq!(snapshot.state.status, AcpSessionStatus::Finished);
    assert_eq!(
        snapshot.state.stop_reason.as_deref(),
        Some("future_stop_reason")
    );
    retire(&handle).await;
}

#[tokio::test]
async fn retained_owner_reuses_one_process_across_attempts_and_releasing_subscribers() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let first = host
        .open(launch(root.path(), "ISSUE-1", "none").await)
        .await
        .expect("open");
    let ControlResult::Lease { lease_id, .. } =
        first.control(SessionControl::Attach).await.expect("attach")
    else {
        panic!("lease")
    };
    let events = first.subscribe();
    let id = prompt(&first, "attempt-1", "first").await.session_id;
    drop(events);
    first
        .control(SessionControl::Release { lease_id })
        .await
        .expect("release subscriber");
    let next = host
        .open(launch(root.path(), "ISSUE-1", "none").await)
        .await
        .expect("borrow existing");
    assert_eq!(first.owner_id, next.owner_id);
    drop(first);
    assert_eq!(prompt(&next, "attempt-2", "second").await.session_id, id);
    assert_eq!(
        std::fs::read_to_string(root.path().join("ISSUE-1/launches"))
            .expect("launches")
            .lines()
            .count(),
        1
    );
    let state = next.inspect().await.expect("snapshot");
    assert_eq!(state.state.identity.run_id, "attempt-2");
    assert_eq!(state.state.status, AcpSessionStatus::Finished);
    retire(&next).await;
}

#[tokio::test]
async fn concurrent_sessions_busy_prompt_fence_and_cancellation_keep_other_commands_responsive() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let (a, b) = tokio::join!(
        host.open(launch(root.path(), "ISSUE-1", "none").await),
        host.open(launch(root.path(), "ISSUE-2", "none").await)
    );
    let (a, b) = (a.expect("a"), b.expect("b"));
    let cancel = CancellationToken::new();
    let pending = tokio::spawn({
        let a = a.clone();
        let cancel = cancel.clone();
        async move { a.prompt("hang-run".into(), 1, "hang".into(), cancel).await }
    });
    for _ in 0..100 {
        if a.inspect().await.expect("inspect").state.status == AcpSessionStatus::Submitted {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(
        a.prompt("overlap".into(), 1, "no".into(), CancellationToken::new())
            .await,
        Err(HostError::Busy)
    ));
    assert!(matches!(
        a.control(SessionControl::Retire).await,
        Err(HostError::Busy)
    ));
    assert!(prompt(&b, "independent", "other issue").await.succeeded());
    cancel.cancel();
    assert!(
        pending
            .await
            .expect("task")
            .expect("cancelled")
            .cancellation_acknowledged
    );
    retire(&a).await;
    retire(&b).await;
}

#[tokio::test(flavor = "current_thread")]
async fn retained_turn_without_client_deadline_survives_five_minutes_of_virtual_time() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "LONG", "none").await;
    request.limits.prompt_timeout = Duration::ZERO;
    let handle = host.open(request).await.expect("open");
    let cancellation = CancellationToken::new();
    let turn = tokio::spawn({
        let handle = handle.clone();
        let cancellation = cancellation.clone();
        async move {
            handle
                .prompt("long-run".into(), 1, "hang".into(), cancellation)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if handle
                .source_history()
                .events
                .iter()
                .any(|event| matches!(event, SessionEvent::Source { frame, .. } if frame.payload["method"] == "session/prompt"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("prompt submitted");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    assert!(
        !turn.is_finished(),
        "retained turn must not hit a fixed client deadline"
    );
    tokio::time::resume();
    cancellation.cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), turn)
            .await
            .expect("cancelled turn completes")
            .expect("turn task")
            .expect("cancelled prompt")
            .cancellation_acknowledged
    );
    retire(&handle).await;
}

#[test]
fn cancellation_while_submission_sync_is_blocked_never_sends_prompt() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let root = tempfile::tempdir().expect("root");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        let request = launch(root.path(), "SUBMIT", "none").await;
        let workspace = request.workspace.workspace_path().to_path_buf();
        let handle = host.open(request).await.expect("open");

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = started_tx.send(());
            release_rx
                .recv()
                .expect("release blocking filesystem worker");
        });
        started_rx.await.expect("filesystem worker occupied");

        let cancellation = CancellationToken::new();
        let pending = tokio::spawn({
            let handle = handle.clone();
            let cancellation = cancellation.clone();
            async move {
                handle
                    .prompt(
                        "barrier-run".into(),
                        1,
                        "must-not-send".into(),
                        cancellation,
                    )
                    .await
            }
        });
        let barrier = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                // Preparation remains command-responsive. A stalled inspect
                // identifies the actor awaiting the durable filesystem write.
                if tokio::time::timeout(Duration::from_millis(20), handle.inspect())
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        cancellation.cancel();
        release_tx.send(()).expect("release worker");
        blocker.await.expect("blocking worker");
        barrier.expect("submission reached filesystem barrier");
        let error = pending.await.expect("prompt task").expect_err("cancelled");
        assert!(
            error
                .to_string()
                .contains("cancelled before prompt submission")
        );
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(workspace.join(".opensymphony/conversation.json")).expect("manifest"),
        )
        .expect("manifest JSON");
        assert_eq!(manifest["acp"]["status"], "finished");
        assert_eq!(manifest["acp"]["stop_reason"], "cancelled_before_prompt");
        assert!(
            !std::fs::read_to_string(workspace.join("methods"))
                .expect("peer method log")
                .contains("session/prompt"),
            "cancelled prompt crossed the transport barrier"
        );
    });
}

#[tokio::test]
async fn retention_leases_expire_and_resource_limits_refuse_active_owners() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy {
        max_sessions: 1,
        idle_timeout: Duration::from_millis(100),
        // Leave room for parallel workspace setup before the renewal; expiry
        // is asserted after the full lease interval below.
        attachment_ttl: Duration::from_secs(2),
        ..RetentionPolicy::default()
    })
    .expect("host");
    let a = host
        .open(launch(root.path(), "ISSUE-1", "none").await)
        .await
        .expect("open");
    let ControlResult::Lease { lease_id, .. } =
        a.control(SessionControl::Attach).await.expect("attach")
    else {
        panic!("lease")
    };
    tokio::time::sleep(Duration::from_millis(160)).await;
    assert!(a.inspect().await.is_ok(), "attachment pins idle owner");
    assert!(matches!(
        host.open(launch(root.path(), "ISSUE-2", "none").await)
            .await,
        Err(HostError::ResourceLimit)
    ));
    a.control(SessionControl::Renew { lease_id })
        .await
        .expect("renew");
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(
        a.inspect().await.is_err(),
        "expired attachment allows visible retirement"
    );
    let b = host
        .open(launch(root.path(), "ISSUE-2", "none").await)
        .await
        .expect("slot reclaimed");
    tokio::time::sleep(Duration::from_millis(180)).await;
    assert!(
        b.inspect().await.is_err(),
        "unattached idle session expires"
    );
}

#[tokio::test]
async fn restoration_uses_only_negotiated_load_or_resume_and_tags_replay() {
    for mode in ["load", "resume", "none"] {
        let root = tempfile::tempdir().expect("temp");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        let first = host
            .open(launch(root.path(), "ISSUE-1", mode).await)
            .await
            .expect("open");
        let original = prompt(&first, "first", "hello").await.session_id;
        retire(&first).await;
        let next = host
            .open(launch(root.path(), "ISSUE-1", mode).await)
            .await
            .expect("recovery");
        let state = next.inspect().await.expect("snapshot").state;
        let expected = match mode {
            "load" => AcpRecovery::RestoredLoad,
            "resume" => AcpRecovery::RestoredResume,
            _ => AcpRecovery::Fresh,
        };
        assert_eq!(state.recovery, expected);
        assert!(next.generation > first.generation);
        if mode == "none" {
            assert_ne!(state.session_id.as_deref(), Some(original.as_str()));
        } else {
            assert_eq!(state.session_id.as_deref(), Some(original.as_str()));
        }
        // Allow adjacent live notification through the ordered dispatcher.
        tokio::time::sleep(Duration::from_millis(30)).await;
        let history = next.source_history();
        assert!(!history.truncated);
        let replayed = history.events.iter().filter(|event| matches!(event, SessionEvent::Source { replay: true, frame, .. } if frame.payload["params"]["update"]["content"]["text"] == "old history")).count();
        assert_eq!(replayed, usize::from(mode == "load"));
        assert!(!history.events.iter().any(|event| matches!(event, SessionEvent::Source { replay: true, frame, .. } if frame.payload["params"]["update"]["content"]["text"] == "live adjacent to restore response")));
        let methods =
            std::fs::read_to_string(root.path().join("ISSUE-1/methods")).expect("methods");
        assert_eq!(
            methods
                .lines()
                .filter(|line| *line == "session/prompt")
                .count(),
            1,
            "restoration never replays prompt"
        );
        retire(&next).await;
    }
}

#[tokio::test]
async fn uncertain_submission_survives_owner_loss_and_rejects_new_process() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let a = host
        .open(launch(root.path(), "ISSUE-1", "load").await)
        .await
        .expect("open");
    let mut events = a.subscribe();
    assert!(
        a.prompt(
            "crash-run".into(),
            1,
            "crash".into(),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    let ended = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let SessionEvent::Ended {
                cleanup_ready,
                detail,
                ..
            } = events.recv().await.expect("owner events")
            {
                break (cleanup_ready, detail);
            }
        }
    })
    .await
    .expect("ended event");
    assert!(!ended.0, "EOF/reaping is not remote quiescence");
    assert!(ended.1.is_some());
    tokio::time::sleep(Duration::from_millis(80)).await;
    let next = SessionHost::new(RetentionPolicy::default()).expect("new host");
    assert!(matches!(
        next.open(launch(root.path(), "ISSUE-1", "load").await)
            .await,
        Err(HostError::UncertainSubmission)
    ));
    let state: ConversationManifest = serde_json::from_slice(
        &std::fs::read(root.path().join("ISSUE-1/.opensymphony/conversation.json"))
            .expect("manifest"),
    )
    .expect("JSON");
    assert!(matches!(
        state.acp.expect("ACP").status,
        AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain
    ));
    assert_eq!(
        std::fs::read_to_string(root.path().join("ISSUE-1/launches"))
            .expect("launches")
            .lines()
            .count(),
        1
    );
}

#[tokio::test]
async fn duplicate_hosts_and_identity_or_generation_changes_cannot_reuse_session() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let a = host
        .open(launch(root.path(), "ISSUE-1", "none").await)
        .await
        .expect("open");
    let second = SessionHost::new(RetentionPolicy::default()).expect("other host");
    assert!(matches!(
        second
            .open(launch(root.path(), "ISSUE-1", "none").await)
            .await,
        Err(HostError::AlreadyOwned)
    ));
    for field in ["profile", "grant", "workspace", "generation"] {
        let mut changed = launch(root.path(), "ISSUE-1", "none").await;
        match field {
            "profile" => changed.identity.profile_id = "other".into(),
            "grant" => changed.identity.credential_scope = "changed".into(),
            "workspace" => changed.identity.workspace_path = root.path().into(),
            _ => changed.identity.generation = a.generation + 1,
        }
        assert!(
            matches!(host.open(changed).await, Err(HostError::IdentityMismatch)),
            "{field}"
        );
    }
    let mut stale = a.clone();
    stale.generation += 1;
    assert!(matches!(
        stale.inspect().await,
        Err(HostError::IdentityMismatch)
    ));
    assert!(matches!(
        stale
            .prompt("stale".into(), 1, "no".into(), CancellationToken::new())
            .await,
        Err(HostError::IdentityMismatch)
    ));
    retire(&a).await;
}

#[tokio::test]
async fn explicit_persistence_requirement_fails_before_prompt() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "ISSUE-1", "none").await;
    request.require_persistence = true;
    assert!(host.open(request).await.is_err());
    assert!(
        !std::fs::read_to_string(root.path().join("ISSUE-1/methods"))
            .expect("methods")
            .contains("session/prompt")
    );
}

#[tokio::test]
async fn host_control_plane_authenticates_and_reaches_same_owner_over_http() {
    use opensymphony::{
        opensymphony_control::{ControlPlaneServer, SnapshotStore},
        opensymphony_domain::*,
    };
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(root.path(), "ISSUE-1", "none").await)
        .await
        .expect("open");
    let now = chrono::Utc::now();
    let snapshot = ControlPlaneDaemonSnapshot {
        generated_at: now,
        daemon: ControlPlaneDaemonStatus {
            state: ControlPlaneDaemonState::Ready,
            last_poll_at: now,
            workspace_root: String::new(),
            status_line: String::new(),
        },
        agent_server: ControlPlaneAgentServerStatus {
            reachable: false,
            base_url: String::new(),
            conversation_count: 0,
            status_line: String::new(),
        },
        memory_server: Default::default(),
        metrics: ControlPlaneMetricsSnapshot {
            running_issues: 0,
            retry_queue_depth: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            total_cost_micros: 0,
        },
        issues: vec![],
        recent_events: vec![],
    };
    let token = "private-host-bearer-for-hermetic-test-only";
    let server = ControlPlaneServer::new(SnapshotStore::new(snapshot))
        .with_acp_host(host, token.into())
        .expect("private routes");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listen");
    let addr = listener.local_addr().expect("address");
    let task = tokio::spawn(server.serve(listener));
    let url = format!("http://{addr}/api/v1/acp/{}", handle.owner_id);
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .post(&url)
            .json(&serde_json::json!({"generation": handle.generation, "action": "inspect"}))
            .send()
            .await
            .expect("response")
            .status(),
        401
    );
    let observed = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({"generation": handle.generation, "action": "inspect"}))
        .send()
        .await
        .expect("response");
    assert!(observed.status().is_success());
    let value: ControlResult = observed.json().await.expect("DTO");
    assert!(
        matches!(value, ControlResult::Snapshot(snapshot) if snapshot.state.owner_id == handle.owner_id)
    );
    let stale = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({"generation": handle.generation+1, "action": "attach"}))
        .send()
        .await
        .expect("stale");
    assert_eq!(stale.status(), 409);
    let retired = client
        .post(&url)
        .bearer_auth(token)
        .json(&serde_json::json!({"generation": handle.generation, "action": "retire"}))
        .send()
        .await
        .expect("retire denied");
    assert_eq!(retired.status(), 403);
    let mut stream = client
        .get(format!("{url}/events?generation={}", handle.generation))
        .bearer_auth(token)
        .send()
        .await
        .expect("events");
    let first = tokio::time::timeout(Duration::from_secs(2), stream.chunk())
        .await
        .expect("bounded stream")
        .expect("read")
        .expect("state");
    assert!(String::from_utf8_lossy(&first).contains("event: state"));
    retire(&handle).await;
    task.abort();
}

// Invoked only by the crash test as a separate runtime host process.
#[tokio::test]
async fn host_process_fixture() {
    let Ok(root) = std::env::var("OSYM_ACP_CRASH_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(&root, "ISSUE-1", "resume").await)
        .await
        .expect("open");
    if std::env::var("OSYM_ACP_CRASH_PHASE").expect("phase") == "submitted" {
        let pending = handle.clone();
        tokio::spawn(async move {
            pending
                .prompt(
                    "uncertain-run".into(),
                    1,
                    "hang".into(),
                    CancellationToken::new(),
                )
                .await
        });
        loop {
            if handle.inspect().await.expect("inspect").state.status == AcpSessionStatus::Submitted
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    std::fs::write(root.join("host-ready"), "ready").expect("barrier");
    tokio::time::sleep(Duration::from_secs(60)).await;
}

#[cfg(unix)]
#[tokio::test]
async fn real_host_crash_before_and_after_submission_preserves_recovery_boundary() {
    for phase in ["ready", "submitted"] {
        let root = tempfile::tempdir().expect("temp");
        let mut child =
            std::process::Command::new(std::env::current_exe().expect("test executable"))
                .args(["--exact", "host_process_fixture", "--nocapture"])
                .env("OSYM_ACP_CRASH_ROOT", root.path())
                .env("OSYM_ACP_CRASH_PHASE", phase)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("separate host");
        tokio::time::timeout(Duration::from_secs(10), async {
            while !root.path().join("host-ready").exists() {
                assert!(
                    child.try_wait().expect("poll").is_none(),
                    "host exited before checkpoint"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("host barrier");
        child.kill().expect("crash host");
        child.wait().expect("reap host");
        let manifest: ConversationManifest = serde_json::from_slice(
            &std::fs::read(root.path().join("ISSUE-1/.opensymphony/conversation.json"))
                .expect("manifest"),
        )
        .expect("JSON");
        let AcpProcessState::Running { pid } = manifest.acp.expect("ACP").process else {
            panic!("persisted child")
        };
        let pid = rustix::process::Pid::from_raw(pid as i32).expect("pid");
        // Orphan process may still be winding down after inherited transport EOF.
        tokio::time::timeout(Duration::from_secs(5), async {
            while rustix::process::test_kill_process_group(pid) != Err(rustix::io::Errno::SRCH) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("peer observes host transport loss");
        let host = SessionHost::new(RetentionPolicy::default()).expect("replacement host");
        let recovered = host
            .open(launch(root.path(), "ISSUE-1", "resume").await)
            .await;
        if phase == "ready" {
            let recovered = recovered.expect("known quiescent state can restore");
            assert_eq!(
                recovered.inspect().await.expect("state").state.recovery,
                AcpRecovery::RestoredResume
            );
            retire(&recovered).await;
        } else {
            assert!(
                matches!(recovered, Err(HostError::UncertainSubmission)),
                "ambiguous prompt stays fenced"
            );
            assert_eq!(
                std::fs::read_to_string(root.path().join("ISSUE-1/launches"))
                    .expect("launch count")
                    .lines()
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn recorded_inspection_and_bounded_source_history_are_truthful() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let request = launch(root.path(), "ISSUE-1", "none").await;
    let manager =
        WorkspaceManager::new(request.manager.config().clone()).expect("inspection manager");
    let workspace = request.workspace.clone();
    let handle = host.open(request).await.expect("open");
    let text = format!(" code\n\t{}  ", "x".repeat(4096));
    prompt(&handle, "live", &text).await;
    assert!(handle.source_history().events.iter().any(|event| matches!(event, SessionEvent::Source { frame, .. } if frame.payload["params"]["update"]["content"]["text"] == text)), "source payload preserves long whitespace-bearing content");
    let state = handle.inspect().await.expect("live");
    assert_eq!(state.state.recovery, AcpRecovery::LiveAttach);
    let mut requires = launch(root.path(), "ISSUE-1", "none").await;
    requires.require_persistence = true;
    assert!(matches!(
        host.open(requires).await,
        Err(HostError::PersistenceUnsupported)
    ));
    retire(&handle).await;
    let recorded = inspect_recorded_session(&manager, &workspace)
        .await
        .expect("recorded");
    assert!(!recorded.live && !recorded.retirement_eligible);
    assert_eq!(recorded.state.recovery, AcpRecovery::TranscriptOnly);
    // Changed limits identify a different owner; exercise bounded history on
    // a fresh workspace rather than changing the recorded owner contract.
    let mut bounded = launch(root.path(), "BOUNDED", "none").await;
    bounded.limits.queued_frames = 2;
    bounded.limits.queued_bytes = 256;
    let bounded = host.open(bounded).await.expect("bounded owner");
    let history = bounded.source_history();
    assert!(history.truncated);
    assert!(history.events.len() <= 2);
    retire(&bounded).await;
}

#[tokio::test]
async fn missing_persisted_context_resets_only_when_persistence_is_optional() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(root.path(), "ISSUE-1", "load").await)
        .await
        .expect("open");
    prompt(&handle, "finished", "hello").await;
    retire(&handle).await;
    std::fs::write(root.path().join("ISSUE-1/forget-session"), "").expect("forget");
    let mut required = launch(root.path(), "ISSUE-1", "load").await;
    required.require_persistence = true;
    assert!(
        host.open(required).await.is_err(),
        "required persistence never resets"
    );
    let reset = host
        .open(launch(root.path(), "ISSUE-1", "load").await)
        .await
        .expect("optional reset");
    assert_eq!(
        reset.inspect().await.expect("fresh state").state.recovery,
        AcpRecovery::Fresh
    );
    retire(&reset).await;
    let manifest: ConversationManifest = serde_json::from_slice(
        &std::fs::read(root.path().join("ISSUE-1/.opensymphony/conversation.json"))
            .expect("manifest"),
    )
    .expect("JSON");
    assert!(manifest.fresh_conversation);
    assert!(
        manifest
            .reset_reason
            .expect("reset diagnostic")
            .contains("persisted context")
    );
}

#[tokio::test]
async fn missing_restoration_revokes_old_callbacks_before_fresh_session_response() {
    use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
    for mode in ["services_load", "services_resume"] {
        let root = tempfile::tempdir().expect("root");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        for attempt in 0..2 {
            let mut request = launch(root.path(), "RESET", mode).await;
            request.context.services.write_files = true;
            request.context.services.mcp_servers.push(McpServer::Stdio(
                McpServerStdio::new("memory", "memory-server").args(vec![
                    "--token".into(),
                    "retained-scope-grant".into(),
                    "--custom".into(),
                    "opaque-generic-arg".into(),
                ]),
            ));
            let handle = host.open(request).await.expect("open or reset");
            assert!(
                prompt(&handle, &format!("attempt-{attempt}"), "hello")
                    .await
                    .succeeded()
            );
            retire(&handle).await;
            std::fs::write(root.path().join("RESET/forget-session"), "").expect("forget");
        }
        assert!(
            root.path()
                .join("RESET/abandoned-callback-rejected")
                .exists()
        );
        assert!(!root.path().join("RESET/abandoned-write").exists());
    }
}

async fn configured_launch(root: &Path) -> SessionLaunch {
    let mut request = launch(root, "CONFIG", "services_config").await;
    request.profile.session.model = Some("second".into());
    request.profile.session.mode = Some("execute".into());
    request
        .profile
        .session
        .options
        .insert("verbosity".into(), "loud".into());
    request
}

async fn ended(events: &mut tokio::sync::broadcast::Receiver<SessionEvent>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match events.recv().await {
                Ok(SessionEvent::Ended { .. }) => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("owner closed before terminal event: {error}"),
            }
        }
    })
    .await
    .expect("owner ended");
}

#[tokio::test]
async fn retained_preparation_reapplies_explicit_choices_and_rejects_removed_choices() {
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(configured_launch(root.path()).await)
        .await
        .expect("open");
    let mut events = handle.subscribe();
    let drifted = prompt(&handle, "drift", "config-drift").await;
    assert_eq!(drifted.configuration.options[0]["currentValue"], "first");
    let configured = prompt(&handle, "configured", "config-assert").await;
    for (option, expected) in configured
        .configuration
        .options
        .iter()
        .zip(["second", "execute", "loud"])
    {
        assert_eq!(option["currentValue"], expected);
    }
    let history = handle.source_history();
    let configured_ids = history
        .events
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Source { run_id, frame, .. }
                if frame.payload["method"] == "session/set_config_option"
                    && run_id == "configured" =>
            {
                Some(frame.payload["id"].to_string())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        configured_ids.len(),
        3,
        "model/mode/option preparation belongs to pending run"
    );
    for event in &history.events {
        if let SessionEvent::Source { run_id, frame, .. } = event
            && frame.direction == "incoming"
            && configured_ids.contains(&frame.payload["id"].to_string())
        {
            assert_eq!(run_id, "configured", "configuration response attribution");
        }
    }
    prompt(&handle, "remove", "config-remove").await;
    assert!(
        handle
            .prompt(
                "must-not-submit".into(),
                1,
                "config-assert".into(),
                CancellationToken::new()
            )
            .await
            .is_err()
    );
    ended(&mut events).await;
    let manifest: ConversationManifest = serde_json::from_slice(
        &std::fs::read(root.path().join("CONFIG/.opensymphony/conversation.json"))
            .expect("manifest"),
    )
    .expect("JSON");
    assert_eq!(manifest.acp.expect("ACP").identity.run_id, "remove");
    assert_eq!(
        std::fs::read_to_string(root.path().join("CONFIG/methods"))
            .expect("methods")
            .lines()
            .filter(|method| *method == "session/prompt")
            .count(),
        3
    );
}

#[tokio::test]
async fn retained_configuration_preparation_remains_cancellable_and_deadline_bounded() {
    for cancel in [false, true] {
        let root = tempfile::tempdir().expect("root");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        let mut request = configured_launch(root.path()).await;
        request.limits.setup_timeout = Duration::from_secs(2);
        let handle = host.open(request).await.expect("open");
        let mut events = handle.subscribe();
        prompt(&handle, "stall", "config-stall").await;
        let cancellation = CancellationToken::new();
        let marker = root.path().join("CONFIG/preparing-config");
        let (result, ()) = tokio::join!(
            handle.prompt(
                "must-not-submit".into(),
                1,
                "config-assert".into(),
                cancellation.clone()
            ),
            async {
                tokio::time::timeout(Duration::from_secs(3), async {
                    while !marker.exists() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("preparation started");
                let state = handle.inspect().await.expect("owner stays responsive");
                assert_eq!(state.state.status, AcpSessionStatus::Finished);
                assert!(handle.source_history().events.iter().any(|event| matches!(event, SessionEvent::Source {run_id, frame, ..} if run_id == "must-not-submit" && frame.payload["method"] == "session/set_config_option")), "preparation attribution does not depend on later submission");
                if cancel {
                    cancellation.cancel();
                }
            }
        );
        let error = result.expect_err("preparation must stop").to_string();
        assert!(
            error.contains(if cancel { "cancel" } else { "deadline" }),
            "{error}"
        );
        ended(&mut events).await;
        let manifest: ConversationManifest = serde_json::from_slice(
            &std::fs::read(root.path().join("CONFIG/.opensymphony/conversation.json"))
                .expect("manifest"),
        )
        .expect("JSON");
        assert_eq!(manifest.acp.expect("ACP").identity.run_id, "stall");
    }
}

#[tokio::test]
async fn retained_callbacks_reset_after_cancellation_and_keep_live_configuration() {
    let root = tempfile::tempdir().expect("root");
    let mut launch = launch(root.path(), "SERVICES", "services").await;
    launch.context.services = HostServices {
        read_files: true,
        write_files: true,
        terminals: true,
        ..Default::default()
    };
    launch.profile.session.model = Some("second".into());
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host.open(launch).await.expect("open");
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    let marker = root.path().join("SERVICES/callback.pid");
    let (first, ()) = tokio::join!(
        handle.prompt(
            "cancel-turn".into(),
            1,
            "services-cancel".into(),
            cancellation
        ),
        async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !marker.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("terminal started");
            cancel.cancel();
        }
    );
    let first = first.expect("cancel acknowledgement");
    assert!(first.cancellation_acknowledged);
    assert_eq!(first.configuration.options[0]["currentValue"], "second");
    let second = prompt(&handle, "next-turn", "services-next").await;
    assert!(second.succeeded());
    assert_eq!(first.session_id, second.session_id);
    assert_eq!(second.configuration.options[0]["currentValue"], "first");
    assert_eq!(
        std::fs::read_to_string(root.path().join("SERVICES/launches"))
            .expect("launches")
            .lines()
            .count(),
        1
    );
    retire(&handle).await;
}

#[tokio::test]
async fn retained_identity_fingerprints_host_policy_and_resolved_mcp_grants() {
    use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let server = |token: &str| {
        McpServer::Stdio(
            McpServerStdio::new("memory", "memory-server")
                .args(vec!["--token".into(), token.into()]),
        )
    };
    let mut first = launch(root.path(), "GRANTS", "none").await;
    first.context.services.mcp_servers = vec![server("original-scoped-grant")];
    let handle = host.open(first).await.expect("open");
    for changed_policy in [true, false] {
        let mut changed = launch(root.path(), "GRANTS", "none").await;
        changed.context.services.read_files = changed_policy;
        changed.context.services.mcp_servers = vec![server(if changed_policy {
            "original-scoped-grant"
        } else {
            "replacement-scoped-grant"
        })];
        assert!(matches!(
            host.open(changed).await,
            Err(HostError::IdentityMismatch)
        ));
    }
    let manifest =
        std::fs::read_to_string(root.path().join("GRANTS/.opensymphony/conversation.json"))
            .expect("manifest");
    assert!(!manifest.contains("original-scoped-grant"));
    assert!(!manifest.contains("replacement-scoped-grant"));
    retire(&handle).await;
}

#[tokio::test]
async fn restored_sessions_receive_scoped_mcp_and_apply_returned_configuration() {
    use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
    for mode in ["services_load", "services_resume"] {
        let root = tempfile::tempdir().expect("root");
        let host = SessionHost::new(RetentionPolicy::default()).expect("host");
        let mut session_id = None;
        for attempt in 0..2 {
            let mut launch = launch(root.path(), "RESTORE", mode).await;
            launch.profile.session.model = Some("second".into());
            launch.context.services.mcp_servers.push(McpServer::Stdio(
                McpServerStdio::new("memory", "memory-server").args(vec![
                    "--token".into(),
                    "retained-scope-grant".into(),
                    "--custom".into(),
                    "opaque-generic-arg".into(),
                ]),
            ));
            let handle = host.open(launch).await.expect("open or restore");
            let history = handle.source_history();
            let mut scoped_requests = 0;
            for event in &history.events {
                if let SessionEvent::Source { frame, .. } = event
                    && matches!(
                        frame.payload["method"].as_str(),
                        Some("session/new" | "session/load" | "session/resume")
                    )
                {
                    scoped_requests += 1;
                    let captured = frame.payload.to_string();
                    assert!(!captured.contains("opaque-generic-arg"));
                    assert!(!captured.contains("retained-scope-grant"));
                    assert_eq!(
                        frame.payload["params"]["mcpServers"][0]["args"],
                        serde_json::json!(["[redacted]", "[redacted]", "[redacted]", "[redacted]"])
                    );
                }
            }
            assert!(scoped_requests > 0, "source history includes MCP setup");
            let report = prompt(&handle, &format!("attempt-{attempt}"), "hello").await;
            assert_eq!(report.configuration.options[0]["currentValue"], "second");
            if let Some(id) = &session_id {
                assert_eq!(&report.session_id, id);
            } else {
                session_id = Some(report.session_id);
            }
            retire(&handle).await;
        }
        let methods =
            std::fs::read_to_string(root.path().join("RESTORE/methods")).expect("methods");
        assert!(methods.contains(if mode == "services_load" {
            "session/load"
        } else {
            "session/resume"
        }));
    }
}

#[tokio::test]
async fn retained_identity_rejects_changed_callback_and_response_limits() {
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let handle = host
        .open(launch(root.path(), "LIMITS", "none").await)
        .await
        .expect("open");
    for kind in 0..8 {
        let mut changed = launch(root.path(), "LIMITS", "none").await;
        match kind {
            0 => changed.limits.file_bytes = 1024,
            1 => changed.limits.terminal_output_bytes = 1024,
            2 => changed.limits.terminal_count = 1,
            3 => changed.limits.pending_callbacks = 1,
            4 => changed.limits.callback_timeout = Duration::from_secs(1),
            5 => changed.limits.frame_bytes = 4096,
            6 => changed.limits.callback_bytes = 4096,
            _ => changed.limits.callback_frames = 1,
        }
        assert!(
            matches!(host.open(changed).await, Err(HostError::IdentityMismatch)),
            "kind {kind}"
        );
    }
    retire(&handle).await;
}

#[tokio::test]
async fn ordinary_mcp_environment_does_not_poison_launch_or_retained_source_values() {
    use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "MCPENV", "none").await;
    request.context.services.mcp_servers.push(McpServer::Stdio(
        McpServerStdio::new("memory", "memory-server").env(vec![
            EnvVariable::new("DEBUG", "3"),
            EnvVariable::new("ACCESS_TOKEN", "retained-env-secret"),
        ]),
    ));
    let handle = host
        .open(request)
        .await
        .expect("DEBUG=3 must not reject python3");
    prompt(&handle, "env", "debug 3 retained-env-secret").await;
    let history = handle.source_history();
    let request = history
        .events
        .iter()
        .find_map(|event| match event {
            SessionEvent::Source { frame, .. } if frame.payload["method"] == "session/new" => {
                Some(&frame.payload)
            }
            _ => None,
        })
        .expect("source request");
    assert_eq!(
        request["params"]["mcpServers"][0]["env"][0]["value"],
        "[redacted]"
    );
    assert!(history.events.iter().any(|event| matches!(event, SessionEvent::Source { frame, .. } if frame.payload["params"]["update"]["content"]["text"] == "debug 3 [redacted]")));
    assert!(
        !serde_json::to_string(&history.events)
            .expect("source")
            .contains("retained-env-secret")
    );
    retire(&handle).await;
}

#[tokio::test]
async fn retained_source_redacts_file_payloads_without_changing_callback_wire_content() {
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "PRIVATE", "services").await;
    request.context.services.read_files = true;
    request.context.services.write_files = true;
    request.context.services.terminals = true;
    std::fs::write(
        request.workspace.workspace_path().join("private-file"),
        "opaque_workspace_payload_610",
    )
    .expect("private file");
    let handle = host.open(request).await.expect("open");
    assert!(prompt(&handle, "private", "file-privacy").await.succeeded());
    assert_eq!(
        std::fs::read_to_string(root.path().join("PRIVATE/private-copy"))
            .expect("peer received and wrote original"),
        "opaque_workspace_payload_610"
    );
    let history = handle.source_history();
    assert!(
        !serde_json::to_string(&history.events)
            .expect("history")
            .contains("opaque_workspace_payload_610")
    );
    assert!(history.events.iter().any(|event| matches!(event, SessionEvent::Source {frame, ..} if frame.payload["result"]["content"] == "[redacted]")));
    assert!(history.events.iter().any(|event| matches!(event, SessionEvent::Source {frame, ..} if frame.payload["method"] == "fs/write_text_file" && frame.payload["params"]["content"] == "[redacted]")));
    assert!(history.events.iter().any(|event| matches!(event, SessionEvent::Source {frame, ..} if frame.payload["result"]["output"] == "[redacted]")));
    retire(&handle).await;
}

#[tokio::test]
async fn retained_pre_prompt_callbacks_are_rejected_until_begin_turn() {
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "BEFORE", "services_pre_prompt").await;
    request.context.services.write_files = true;
    request.context.services.terminals = true;
    request.profile.session.model = Some("second".into());
    let handle = host.open(request).await.expect("open");
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.path().join("BEFORE/pre-prompt-rejected").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pre-prompt callbacks rejected");
    assert!(!root.path().join("BEFORE/pre-prompt-write").exists());
    assert!(!root.path().join("BEFORE/pre-prompt-process").exists());
    assert!(prompt(&handle, "first", "first").await.succeeded());
    retire(&handle).await;
}

#[tokio::test]
async fn normal_retained_completion_reaps_callbacks_and_rejects_adjacent_idle_work() {
    let root = tempfile::tempdir().expect("root");
    let host = SessionHost::new(RetentionPolicy::default()).expect("host");
    let mut request = launch(root.path(), "LATE", "services").await;
    request.context.services = HostServices {
        read_files: true,
        write_files: true,
        terminals: true,
        ..Default::default()
    };
    let handle = host.open(request).await.expect("open");
    let report = prompt(&handle, "normal", "late-callbacks").await;
    assert!(report.succeeded());
    assert!(
        !report.cancellation_requested,
        "callback closure is separate from prompt cancellation"
    );
    #[cfg(unix)]
    {
        let pid: i32 = std::fs::read_to_string(root.path().join("LATE/normal-child.pid"))
            .expect("child")
            .parse()
            .expect("PID");
        assert_eq!(
            rustix::process::test_kill_process(rustix::process::Pid::from_raw(pid).expect("PID")),
            Err(rustix::io::Errno::SRCH),
            "child must be reaped before completion reply"
        );
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while !root.path().join("LATE/late-rejected").exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("adjacent idle callbacks rejected");
    assert!(!root.path().join("LATE/late-write").exists());
    assert!(!root.path().join("LATE/late-process").exists());
    assert_eq!(
        handle.inspect().await.expect("idle owner").state.status,
        AcpSessionStatus::Finished
    );
    // Repeated short-lived terminals exercise Darwin's exit/group-reap race
    // while other retained-host cases run in parallel.
    for attempt in 0..if cfg!(target_os = "macos") { 12 } else { 1 } {
        assert!(
            prompt(&handle, &format!("next-{attempt}"), "services-next")
                .await
                .succeeded()
        );
    }
    retire(&handle).await;
}
