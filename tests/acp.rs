use opensymphony::opensymphony_acp::{
    AcpProfile, ClientError, ClientLimits, LaunchContext, run_turn,
};
use opensymphony::opensymphony_workflow::{AcpAuth, WorkflowDefinition};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

fn profile(mode: &str) -> AcpProfile {
    AcpProfile {
        command: "python3".into(),
        args: vec![
            format!("{}/tests/fixtures/acp_peer.py", env!("CARGO_MANIFEST_DIR")),
            mode.into(),
        ],
        transport: "stdio".into(),
        protocol_versions: vec![1],
        env_refs: BTreeMap::new(),
        auth: Some(AcpAuth {
            method_id: "test_auth".into(),
        }),
        required_capabilities: vec![],
        extensions: vec![],
        session: Default::default(),
    }
}
fn context(root: &Path) -> LaunchContext {
    let root = root.canonicalize().expect("canonical root");
    let cwd = root.join("COE-608");
    std::fs::create_dir_all(&cwd).expect("create issue workspace");
    LaunchContext {
        workspace_root: root,
        workspace_key: "COE-608".into(),
        issue_workspace: cwd,
        environment: BTreeMap::from([
            ("PATH".into(), std::env::var("PATH").expect("PATH")),
            ("CHECKOUT_SECRET".into(), "checkout-secret-value".into()),
        ]),
        excluded_environment: BTreeSet::from(["CHECKOUT_SECRET".into()]),
        services: Default::default(),
    }
}
fn limits() -> ClientLimits {
    ClientLimits {
        setup_timeout: Duration::from_secs(2),
        prompt_timeout: Duration::from_secs(2),
        cancel_timeout: Duration::from_millis(150),
        reap_timeout: Duration::from_secs(2),
        ..ClientLimits::default()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn acp_turn_without_client_deadline_survives_five_minutes_of_virtual_time() {
    let root = tempfile::tempdir().expect("temp");
    let (tx, mut updates) = tokio::sync::mpsc::channel(4);
    let cancellation = CancellationToken::new();
    let turn = tokio::spawn({
        let cancellation = cancellation.clone();
        let profile = profile("cancel");
        let context = context(root.path());
        async move {
            run_turn(
                &profile,
                context,
                "long running work".into(),
                cancellation,
                Some(tx),
                ClientLimits {
                    prompt_timeout: Duration::ZERO,
                    ..limits()
                },
            )
            .await
        }
    });
    tokio::time::timeout(Duration::from_secs(5), updates.recv())
        .await
        .expect("peer update before clock advance")
        .expect("peer update");
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    assert!(
        !turn.is_finished(),
        "active turn must not hit a fixed client deadline"
    );
    tokio::time::resume();
    cancellation.cancel();
    let result = tokio::time::timeout(Duration::from_secs(5), turn)
        .await
        .expect("cancelled turn completes")
        .expect("turn task")
        .expect("launch");
    assert!(
        result
            .outcome
            .expect("cancel acknowledged")
            .cancellation_acknowledged
    );
}

#[tokio::test]
async fn acp_process_completes_ordered_callbacks_and_redacts_evidence() {
    let root = tempfile::tempdir().expect("temp");
    let mut launch = context(root.path());
    launch
        .environment
        .insert("AUTH_SOURCE".into(), "fake-sensitive-auth-material".into());
    let mut config = profile("complete");
    config
        .env_refs
        .insert("TEST_AUTH".into(), "AUTH_SOURCE".into());
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let run = run_turn(
        &config,
        launch,
        "hello".into(),
        CancellationToken::new(),
        Some(tx),
        limits(),
    )
    .await
    .expect("launch");
    assert!(!format!("{run:?}").contains("fake-sensitive-auth-material"));
    let report = run.outcome.expect("completed protocol");
    assert_eq!(report.stop_reason, "end_turn");
    assert!(!report.cancellation_acknowledged);
    assert_eq!(report.session_id, "opaque/session:zero");
    assert!(run.process_reaped);
    let mut updates = Vec::new();
    while let Ok(update) = rx.try_recv() {
        updates.push(update.update);
    }
    assert_eq!(updates.last().expect("last")["content"]["text"], "last");
    assert!(
        updates
            .iter()
            .any(|u| u["sessionUpdate"] == "future_update")
    );
    let evidence = serde_json::to_string(&run.evidence).expect("JSON");
    assert!(!evidence.contains("fake-sensitive-auth-material"));
    assert!(!run.stderr.contains("fake-sensitive-auth-material"));
    assert!(evidence.contains("opaque-request"));
    assert!(evidence.contains("_future/notice"));
    assert!(run.evidence.iter().any(|e| e.direction == "outgoing"
        && e.payload["id"] == 0
        && e.payload["error"]["code"] == -32601));
    assert_eq!(
        run.evidence
            .iter()
            .filter(|e| e.direction == "outgoing" && e.payload.get("result").is_some())
            .count(),
        1
    );
}

#[tokio::test(flavor = "current_thread")]
async fn acp_binds_new_session_before_adjacent_update_dispatch() {
    for _ in 0..12 {
        let root = tempfile::tempdir().expect("temp");
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let run = run_turn(
            &profile("adjacent_session_update"),
            context(root.path()),
            "hello".into(),
            CancellationToken::new(),
            Some(tx),
            limits(),
        )
        .await
        .expect("launch");
        assert!(run.outcome.expect("ordered session binding").succeeded());
        assert!(run.process_reaped);
        let update = rx.recv().await.expect("adjacent update delivered");
        assert_eq!(update.session_id, "opaque/session:zero");
        assert_eq!(update.update["sessionUpdate"], "current_mode_update");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn acp_bounds_callback_output_before_sdk_enqueue_when_stdin_is_blocked() {
    for mode in [
        "blocked_callbacks_unknown",
        "blocked_callbacks_permission",
        "blocked_callbacks_invalid",
    ] {
        for (callback_frames, callback_bytes) in [(3, 1024 * 1024), (4096, 64 * 1024)] {
            let root = tempfile::tempdir().expect("temp");
            let run = tokio::time::timeout(
                Duration::from_secs(5),
                run_turn(
                    &profile(mode),
                    context(root.path()),
                    "hello".into(),
                    CancellationToken::new(),
                    None,
                    ClientLimits {
                        callback_frames,
                        callback_bytes,
                        // The entire fixture fits ingress; only callback egress can saturate.
                        queued_frames: 4096,
                        queued_bytes: 64 * 1024 * 1024,
                        evidence_frames: 0,
                        ..limits()
                    },
                ),
            )
            .await
            .expect("saturation never blocks dispatch")
            .expect("launch");
            assert_eq!(
                run.outcome.expect_err("output must saturate"),
                ClientError::ResourceLimit { submitted: true },
                "{mode}: {callback_frames}/{callback_bytes}"
            );
            assert!(run.process_reaped);
        }
    }
}

#[tokio::test]
async fn acp_releases_callback_budget_after_transport_flush() {
    let root = tempfile::tempdir().expect("temp");
    let run = run_turn(
        &profile("paced_queue"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        None,
        ClientLimits {
            callback_frames: 1,
            callback_bytes: 256,
            ..limits()
        },
    )
    .await
    .expect("launch");
    assert!(
        run.outcome
            .expect("sixteen callbacks reuse one reservation")
            .succeeded()
    );
    assert!(run.process_reaped);
}

#[tokio::test(flavor = "current_thread")]
async fn acp_cancel_waits_for_prompt_response_and_keeps_tokio_responsive() {
    for (mode, acknowledged) in [("cancel", true), ("ignore_cancel", false)] {
        let root = tempfile::tempdir().expect("temp");
        let token = CancellationToken::new();
        let cancel = token.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let cancel_after_update = async {
            rx.recv().await.expect("update before cancel");
            cancel.cancel();
        };
        let config = profile(mode);
        let (run, ()) = tokio::join!(
            run_turn(
                &config,
                context(root.path()),
                "hello".into(),
                token,
                Some(tx),
                limits()
            ),
            cancel_after_update
        );
        let run = run.expect("launch");
        assert!(run.process_reaped);
        if acknowledged {
            let report = run.outcome.expect("response");
            assert!(report.cancellation_acknowledged);
            assert_eq!(
                rx.recv().await.expect("last update").update["content"]["text"],
                "before cancellation response"
            );
        } else {
            assert_eq!(
                run.outcome.expect_err("must not ack send"),
                ClientError::CancelTimeout
            );
        }
    }
}

#[tokio::test]
async fn acp_adversarial_frames_deadlines_eof_and_unknown_outcomes() {
    for mode in [
        "foreign_session",
        "missing_update",
        "null_update",
        "missing_session_id",
        "malformed",
        "oversized",
        "eof",
        "crash",
        "flood",
        "setup_hang",
        "hang",
        "v2",
    ] {
        let root = tempfile::tempdir().expect("temp");
        let mut bound = limits();
        bound.setup_timeout = Duration::from_millis(500);
        bound.prompt_timeout = Duration::from_millis(500);
        let config = profile(mode);
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_turn(
                &config,
                context(root.path()),
                "hello".into(),
                CancellationToken::new(),
                None,
                bound,
            ),
        )
        .await
        .expect("bounded run")
        .expect("spawn");
        assert!(result.outcome.is_err(), "mode {mode}: {:?}", result);
        assert!(result.process_reaped, "mode {mode}: {result:?}");
        if mode == "v2" {
            assert!(
                result
                    .outcome
                    .expect_err("v2 rejected")
                    .to_string()
                    .contains("negotiate ACP v1")
            );
        }
    }
    let root = tempfile::tempdir().expect("temp");
    let result = run_turn(
        &profile("unknown_stop"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        None,
        limits(),
    )
    .await
    .expect("spawn");
    assert_eq!(
        result.outcome.expect("wire response").stop_reason,
        "future_stop_reason"
    );
}

#[tokio::test]
async fn acp_bounds_update_delivery_stderr_and_evidence() {
    let root = tempfile::tempdir().expect("temp");
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let result = run_turn(
        &profile("complete"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        Some(tx),
        limits(),
    )
    .await
    .expect("spawn");
    assert!(
        matches!(
            result.outcome,
            Err(ClientError::ResourceLimit { submitted: true })
        ),
        "{:?}",
        result
    );
    let result = run_turn(
        &profile("stderr_flood"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        None,
        ClientLimits {
            evidence_frames: 2,
            ..limits()
        },
    )
    .await
    .expect("spawn");
    assert_eq!(result.outcome.expect("turn").stop_reason, "end_turn");
    assert_eq!(result.evidence.len(), 2);
    assert!(result.evidence_truncated);
    assert_eq!(result.stderr, "[stderr capture limit exceeded]");
}

#[tokio::test]
async fn acp_invalid_launch_inputs_fail_before_execution() {
    let root = tempfile::tempdir().expect("temp");
    for case in 0..11 {
        let mut config = profile("complete");
        let mut launch = context(root.path());
        let mut bound = limits();
        match case {
            0 => launch.issue_workspace = launch.workspace_root.clone(),
            1 => {
                config
                    .env_refs
                    .insert("TEST_AUTH".into(), "CHECKOUT_SECRET".into());
            }
            2 => {
                config
                    .env_refs
                    .insert("TEST_AUTH".into(), "MISSING_ENV".into());
            }
            3 => {
                config
                    .env_refs
                    .insert("TEST_AUTH".into(), "AUTH_SOURCE".into());
                launch
                    .environment
                    .insert("AUTH_SOURCE".into(), "known-secret".into());
                config.args.push("known-secret".into());
            }
            4 => config.args.push("--token=secret".into()),
            5 => bound.evidence_bytes = 16 * 1024 * 1024 + 1,
            6 => bound.queued_bytes = 64 * 1024 * 1024 + 1,
            7 => bound.callback_frames = 0,
            8 => bound.callback_frames = 4097,
            9 => bound.callback_bytes = 255,
            _ => bound.callback_bytes = 64 * 1024 * 1024 + 1,
        }
        assert!(
            run_turn(
                &config,
                launch,
                "hello".into(),
                CancellationToken::new(),
                None,
                bound
            )
            .await
            .is_err()
        );
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().expect("outside");
        let mut launch = context(root.path());
        let link = launch.workspace_root.join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).expect("link");
        launch.workspace_key = "escape".into();
        launch.issue_workspace = link;
        assert!(matches!(
            run_turn(
                &profile("complete"),
                launch,
                "hello".into(),
                CancellationToken::new(),
                None,
                limits()
            )
            .await,
            Err(ClientError::InvalidWorkspace)
        ));
    }
}

#[test]
fn acp_workflow_profiles_validate_and_preserve_environment_references() {
    let source = "---\ntracker:\n  kind: linear\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\nrouting:\n  harness: acp\n  harness_profile: fake\nacp:\n  profiles:\n    fake:\n      command: python3\n      args: [agent.py]\n      env_refs: {TEST_AUTH: AUTH_SOURCE}\n---\nPrompt";
    let env = BTreeMap::from([
        ("LINEAR_API_KEY".into(), "linear-secret".into()),
        ("AUTH_SOURCE".into(), "auth-secret".into()),
    ]);
    let resolved = WorkflowDefinition::parse(source)
        .expect("parse")
        .resolve(Path::new("/repo"), &env)
        .expect("resolve");
    assert_eq!(
        resolved.config.routing.harness_profile.as_deref(),
        Some("fake")
    );
    let selected_model = WorkflowDefinition::parse(&source.replace(
        "harness_profile: fake",
        "harness_profile: fake\n  model: workflow-model",
    ))
    .expect("model selection");
    assert_eq!(
        selected_model
            .resolve(Path::new("/repo"), &env)
            .expect("workflow model")
            .config
            .routing
            .model
            .as_deref(),
        Some("workflow-model")
    );
    let mut model_environment = env.clone();
    model_environment.insert("OPENSYMPHONY_MODEL".into(), "environment-model".into());
    let model = selected_model
        .resolve(Path::new("/repo"), &model_environment)
        .expect("environment model")
        .config
        .routing;
    assert_eq!(model.model.as_deref(), Some("environment-model"));
    assert!(model.model_from_env);
    for harness in ["acp", "openhands_agent_server", "codex_app_server"] {
        let mut overrides = env.clone();
        overrides.insert("OPENSYMPHONY_HARNESS".into(), harness.into());
        let overridden = WorkflowDefinition::parse(source)
            .expect("parse override")
            .resolve(Path::new("/repo"), &overrides)
            .expect("resolve explicit harness override");
        assert_eq!(overridden.config.routing.harness, harness);
        assert!(overridden.config.routing.harness_from_env);
        assert_eq!(
            overridden.config.routing.harness_profile.as_deref(),
            if harness == "acp" { Some("fake") } else { None }
        );
    }
    let rendered = serde_yaml::to_string(&resolved.extensions.acp).expect("render");
    assert!(rendered.contains("AUTH_SOURCE"));
    assert!(!rendered.contains("auth-secret"));
    for (old, new) in [
        ("harness_profile: fake", "harness_profile: missing"),
        ("args: [agent.py]", "args: ['--TOKEN=secret']"),
        ("args: [agent.py]", "args: ['--Api-Key', 'secret']"),
        ("args: [agent.py]", "args: ['--PaSsWoRd=secret']"),
        ("args: [agent.py]", "args: ['--SECRET', 'secret']"),
        ("args: [agent.py]", "transport: http"),
        ("args: [agent.py]", "protocol_versions: [2]"),
        ("args: [agent.py]", "extensions: [cursor]"),
        ("args: [agent.py]", "required_capabilities: [unknown]"),
        ("AUTH_SOURCE", "literal-secret-value"),
        ("args: [agent.py]", "auth: {method_id: ''}"),
        ("harness: acp", "harness: openhands_agent_server"),
        (
            "harness_profile: fake",
            "harness_profile: fake\n  model_profile: openhands-profile",
        ),
    ] {
        let workflow =
            WorkflowDefinition::parse(&source.replace(old, new)).expect("parse invalid semantics");
        assert!(workflow.resolve(Path::new("/repo"), &env).is_err(), "{new}");
    }
    for flag in [
        "--access-token",
        "--oauth2-bearer",
        "--client-secret",
        "--pat",
    ] {
        for args in [
            format!("['{flag}', 'literal-secret']"),
            format!("['{}=literal-secret']", flag.to_ascii_uppercase()),
        ] {
            let workflow = WorkflowDefinition::parse(
                &source.replace("args: [agent.py]", &format!("args: {args}")),
            )
            .expect("parse credential flags");
            let error = workflow
                .resolve(Path::new("/repo"), &env)
                .expect_err("credential flags rejected");
            assert!(!error.to_string().contains("literal-secret"));
        }
    }
    let unsupported = WorkflowDefinition::parse(&source.replace("harness: acp", "harness: absent"))
        .expect("parse")
        .resolve(Path::new("/repo"), &env)
        .expect_err("unknown harness");
    assert!(unsupported.to_string().contains("`acp`"));
    assert!(WorkflowDefinition::parse(&source.replace("args: [agent.py]", "cwd: /tmp")).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn acp_unresponsive_process_tree_is_terminated() {
    let root = tempfile::tempdir().expect("temp");
    let launch = context(root.path());
    let pid_path = launch.issue_workspace.join("grandchild.pid");
    let result = run_turn(
        &profile("hang_tree"),
        launch,
        "hello".into(),
        CancellationToken::new(),
        None,
        ClientLimits {
            prompt_timeout: Duration::from_millis(500),
            ..limits()
        },
    )
    .await
    .expect("launch");
    assert_eq!(
        result.outcome.expect_err("deadline"),
        ClientError::PromptTimeout
    );
    assert!(result.process_reaped);
    let pid: i32 = std::fs::read_to_string(pid_path)
        .expect("descendant launched")
        .parse()
        .expect("PID");
    let pid = rustix::process::Pid::from_raw(pid).expect("PID");
    // A killed orphan may briefly remain a zombie until init reaps it.
    let state = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.as_raw_nonzero().get().to_string()])
        .output()
        .expect("ps");
    let state = String::from_utf8_lossy(&state.stdout);
    assert!(
        state.trim().is_empty() || state.trim().starts_with('Z'),
        "descendant still running: {state}"
    );
}

#[tokio::test]
async fn acp_required_capability_and_authentication_fail_before_prompt() {
    for capability in [
        Some("prompt.image"),
        Some("prompt.audio"),
        Some("prompt.embedded_context"),
        None,
    ] {
        let root = tempfile::tempdir().expect("temp");
        let mut config = profile("complete");
        if let Some(capability) = capability {
            config.required_capabilities = vec![capability.into()];
        } else {
            config.auth = Some(AcpAuth {
                method_id: "not-advertised".into(),
            });
        }
        let result = run_turn(
            &config,
            context(root.path()),
            "hello".into(),
            CancellationToken::new(),
            None,
            limits(),
        )
        .await
        .expect("spawn");
        assert!(matches!(result.outcome, Err(ClientError::Setup(_))));
        if let Some(capability) = capability {
            assert!(
                result
                    .outcome
                    .as_ref()
                    .expect_err("missing capability")
                    .to_string()
                    .contains(capability)
            );
        }
        assert!(
            !result
                .evidence
                .iter()
                .any(|frame| frame.payload["method"] == "session/prompt")
        );
    }
}

#[tokio::test]
async fn acp_missing_login_reports_an_actionable_error() {
    let root = tempfile::tempdir().expect("temp");
    let mut config = profile("auth_required");
    config.auth = None;
    let result = run_turn(
        &config,
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        None,
        limits(),
    )
    .await
    .expect("spawn");
    assert_eq!(
        result.outcome.expect_err("auth"),
        ClientError::AuthenticationRequired { submitted: false }
    );
    assert!(
        !result
            .evidence
            .iter()
            .any(|frame| frame.payload["method"] == "session/prompt")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn acp_setup_cancellation_stops_before_prompt_and_pre_cancelled_tokens_do_not_spawn() {
    let root = tempfile::tempdir().expect("temp");
    let token = CancellationToken::new();
    token.cancel();
    let mut config = profile("complete");
    config.command = "nonexistent-acp-executable".into();
    assert_eq!(
        run_turn(
            &config,
            context(root.path()),
            "hello".into(),
            token,
            None,
            limits()
        )
        .await
        .expect_err("pre-cancelled launch"),
        ClientError::CancelledBeforePrompt
    );
    for mode in ["setup_hang", "auth_hang", "new_hang"] {
        let root = tempfile::tempdir().expect("temp");
        let launch = context(root.path());
        let marker = launch.issue_workspace.join("setup-waiting");
        let token = CancellationToken::new();
        let cancel = token.clone();
        let config = profile(mode);
        let bound = ClientLimits {
            setup_timeout: Duration::from_secs(30),
            ..limits()
        };
        let cancel_at_setup = async {
            while !tokio::fs::try_exists(&marker).await.expect("marker") {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            cancel.cancel();
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(
                run_turn(&config, launch, "hello".into(), token, None, bound),
                cancel_at_setup
            )
        })
        .await
        .expect("setup cancellation is prompt");
        let result = result.expect("spawn");
        assert_eq!(
            result.outcome.expect_err("cancelled setup"),
            ClientError::CancelledBeforePrompt
        );
        assert!(result.process_reaped);
        assert!(
            !result
                .evidence
                .iter()
                .any(|frame| frame.payload["method"] == "session/prompt")
        );
    }
}

#[tokio::test]
async fn acp_live_updates_preserve_whitespace_and_long_content() {
    let root = tempfile::tempdir().expect("temp");
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let result = run_turn(
        &profile("rich_update"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        Some(tx),
        limits(),
    )
    .await
    .expect("spawn");
    assert!(result.outcome.expect("completed").succeeded());
    assert_eq!(
        rx.recv().await.expect("space").update["content"]["text"],
        " "
    );
    assert_eq!(
        rx.recv().await.expect("code").update["content"]["text"],
        format!(
            "def example():\n\treturn '  spaced  '\n{}",
            "x".repeat(1024)
        )
    );
}

#[tokio::test]
async fn acp_rpc_errors_preserve_redacted_context_without_evidence() {
    let root = tempfile::tempdir().expect("temp");
    for method in [
        "initialize",
        "authenticate",
        "session/new",
        "session/prompt",
    ] {
        let mut config = profile(&format!("rpc_{}", method.replace('/', "_")));
        config
            .env_refs
            .insert("TEST_AUTH".into(), "AUTH_SOURCE".into());
        let mut launch = context(root.path());
        launch
            .environment
            .insert("AUTH_SOURCE".into(), "rpc-known-sensitive".into());
        let result = run_turn(
            &config,
            launch,
            "hello".into(),
            CancellationToken::new(),
            None,
            ClientLimits {
                evidence_bytes: 0,
                ..limits()
            },
        )
        .await
        .expect("launch");
        assert!(result.evidence.is_empty());
        assert!(result.process_reaped);
        let error = result.outcome.expect_err("peer RPC error");
        assert!(!format!("{error:?} {error}").contains("rpc-known-sensitive"));
        assert!(!format!("{error:?} {error}").contains("acct-rpc-sensitive"));
        match error {
            ClientError::Rpc {
                method: failed_method,
                code,
                message,
                submitted,
            } => {
                assert_eq!(failed_method, method);
                assert_eq!(code, -32602);
                assert_eq!(submitted, method == "session/prompt");
                assert!(message.starts_with("bad params [redacted]"));
                assert!(message.chars().count() <= 515);
            }
            other => panic!("missing RPC context: {other:?}"),
        }
    }
}

#[tokio::test]
async fn acp_configured_environment_targets_follow_platform_name_rules() {
    let root = tempfile::tempdir().expect("temp");
    let mut config = profile("complete");
    config.env_refs = BTreeMap::from([
        ("AGENT_TOKEN".into(), "SOURCE_A".into()),
        ("agent_token".into(), "SOURCE_B".into()),
    ]);
    let mut launch = context(root.path());
    launch.environment.extend([
        ("SOURCE_A".into(), "credential-a".into()),
        ("SOURCE_B".into(), "credential-b".into()),
    ]);
    let result = run_turn(
        &config,
        launch,
        "hello".into(),
        CancellationToken::new(),
        None,
        limits(),
    )
    .await;
    if cfg!(windows) {
        assert!(matches!(result, Err(ClientError::InvalidConfiguration(_))));
    } else {
        assert!(
            result
                .expect("POSIX targets are distinct")
                .outcome
                .expect("turn")
                .succeeded()
        );
    }
}

#[tokio::test]
async fn acp_profile_cannot_map_unscoped_credential_into_memory_scope() {
    let root = tempfile::tempdir().expect("temp");
    for (target, source) in [
        ("OPENSYMPHONY_MEMORY_ADMIN_TOKEN", "AGENT_TOKEN"),
        ("AGENT_TOKEN", "OPENSYMPHONY_MEMORY_TOKEN"),
    ] {
        let mut config = profile("complete");
        config.env_refs.insert(target.into(), source.into());
        let mut launch = context(root.path());
        launch
            .environment
            .insert("AGENT_TOKEN".into(), "unscoped-bearer".into());
        launch
            .environment
            .insert("OPENSYMPHONY_MEMORY_TOKEN".into(), "managed-grant".into());
        let result = run_turn(
            &config,
            launch,
            "hello".into(),
            CancellationToken::new(),
            None,
            limits(),
        )
        .await;
        assert!(matches!(result, Err(ClientError::InvalidConfiguration(_))));
    }
}

#[tokio::test]
async fn acp_live_input_queue_bounds_bytes_and_releases_dispatched_charges() {
    let root = tempfile::tempdir().expect("temp");
    for (mode, succeeds) in [("evidence_flood", false), ("paced_queue", true)] {
        let result = run_turn(
            &profile(mode),
            context(root.path()),
            "hello".into(),
            CancellationToken::new(),
            None,
            ClientLimits {
                frame_bytes: 16384,
                queued_frames: 4096,
                queued_bytes: 1024,
                evidence_frames: 0,
                ..limits()
            },
        )
        .await
        .expect("launch");
        if succeeds {
            assert!(
                result
                    .outcome
                    .expect("repeated round trips release queue bytes")
                    .succeeded()
            );
        } else {
            assert_eq!(
                result
                    .outcome
                    .expect_err("legal frame exceeds live queue byte budget"),
                ClientError::ResourceLimit { submitted: true }
            );
        }
        assert!(result.process_reaped);
    }
}

#[tokio::test]
async fn acp_account_identity_fields_are_redacted_in_evidence_and_live_updates() {
    let root = tempfile::tempdir().expect("temp");
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let result = run_turn(
        &profile("account_identity"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        Some(tx),
        limits(),
    )
    .await
    .expect("launch");
    assert!(result.outcome.as_ref().expect("turn").succeeded());
    let evidence = serde_json::to_string(&result.evidence).expect("JSON");
    assert!(!evidence.contains("acct-sensitive"));
    assert!(!format!("{result:?}").contains("acct-sensitive"));
    let update = rx.recv().await.expect("update");
    assert!(!format!("{update:?}").contains("acct-sensitive"));
    let serialized = serde_json::to_string(&update).expect("JSON");
    assert!(!serialized.contains("acct-sensitive"));
    let identities = &update.update["identities"][0];
    for key in [
        "account_id",
        "account-id",
        "accountIdentity",
        "accountIdentifier",
        "accountID",
        "chatgpt-account-id",
        "providerAccountIdentity",
    ] {
        assert_eq!(identities[key], "[redacted]", "{key}");
    }
    assert_eq!(identities["account_display_name"], "safe display");
}

#[tokio::test]
async fn acp_plain_prompt_above_half_frame_budget_is_admitted() {
    let root = tempfile::tempdir().expect("temp");
    let result = run_turn(
        &profile("complete"),
        context(root.path()),
        "x".repeat(2500),
        CancellationToken::new(),
        None,
        ClientLimits {
            frame_bytes: 4096,
            ..limits()
        },
    )
    .await
    .expect("prompt fits encoded frame");
    assert!(result.outcome.expect("completed turn").succeeded());
    assert!(result.process_reaped);
    assert!(
        result
            .evidence
            .iter()
            .any(|frame| frame.payload["method"] == "session/prompt")
    );
}

#[tokio::test]
async fn acp_retained_evidence_bytes_are_bounded_across_legal_frames() {
    let root = tempfile::tempdir().expect("temp");
    for budget in [0, 4096, 16384] {
        let result = run_turn(
            &profile("evidence_flood"),
            context(root.path()),
            "hello".into(),
            CancellationToken::new(),
            None,
            ClientLimits {
                evidence_bytes: budget,
                ..limits()
            },
        )
        .await
        .expect("launch");
        assert!(
            result
                .outcome
                .expect("evidence truncation preserves completion")
                .succeeded()
        );
        let retained_bytes: usize = result
            .evidence
            .iter()
            .map(|frame| serde_json::to_vec(frame).expect("frame JSON").len())
            .sum();
        assert!(retained_bytes <= budget);
        assert!(result.evidence_truncated);
        assert!(result.evidence.len() < limits().evidence_frames);
        assert!(result.process_reaped);
        if budget == 0 {
            assert!(result.evidence.is_empty());
        } else {
            assert!(!result.evidence.is_empty());
        }
    }
}

#[tokio::test]
async fn acp_escaped_prompt_overflow_is_rejected_before_submission() {
    let root = tempfile::tempdir().expect("temp");
    let result = run_turn(
        &profile("complete"),
        context(root.path()),
        "\0".repeat(100),
        CancellationToken::new(),
        None,
        ClientLimits {
            frame_bytes: 512,
            ..limits()
        },
    )
    .await
    .expect("setup can launch");
    assert_eq!(
        result.outcome.expect_err("encoded prompt too large"),
        ClientError::ResourceLimit { submitted: false }
    );
    assert!(result.process_reaped);
    assert!(
        !result
            .evidence
            .iter()
            .any(|frame| frame.payload["method"] == "session/prompt")
    );
}

#[tokio::test]
async fn acp_host_credentials_cannot_be_copied_into_literal_argv() {
    let root = tempfile::tempdir().expect("temp");
    let mut config = profile("complete");
    config.args.push("checkout-secret-value".into());
    assert!(matches!(
        run_turn(
            &config,
            context(root.path()),
            "hello".into(),
            CancellationToken::new(),
            None,
            limits()
        )
        .await,
        Err(ClientError::InvalidConfiguration(_))
    ));
}

#[tokio::test]
async fn acp_authentication_errors_retain_post_submission_uncertainty() {
    let root = tempfile::tempdir().expect("temp");
    let result = run_turn(
        &profile("prompt_auth_required"),
        context(root.path()),
        "hello".into(),
        CancellationToken::new(),
        None,
        limits(),
    )
    .await
    .expect("spawn");
    assert_eq!(
        result.outcome.expect_err("auth expired"),
        ClientError::AuthenticationRequired { submitted: true }
    );
    assert!(
        result
            .evidence
            .iter()
            .any(|frame| frame.payload["method"] == "session/prompt")
    );
}

#[tokio::test]
async fn acp_live_update_debug_hides_opaque_session_credentials() {
    let root = tempfile::tempdir().expect("temp");
    let mut launch = context(root.path());
    launch
        .environment
        .insert("AUTH_SOURCE".into(), "opaque-session-secret".into());
    let mut config = profile("secret_session");
    config
        .env_refs
        .insert("TEST_AUTH".into(), "AUTH_SOURCE".into());
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let result = run_turn(
        &config,
        launch,
        "hello".into(),
        CancellationToken::new(),
        Some(tx),
        limits(),
    )
    .await
    .expect("spawn");
    assert!(!format!("{result:?}").contains("opaque-session-secret"));
    assert!(result.outcome.expect("completed").succeeded());
    let update = rx.recv().await.expect("update");
    assert_eq!(update.session_id, "opaque-session-secret");
    assert!(!format!("{update:?}").contains("opaque-session-secret"));
}

#[tokio::test]
async fn acp_env_reference_sources_are_retained_only_when_explicitly_targeted() {
    for mode in ["complete", "preserve_source"] {
        let root = tempfile::tempdir().expect("temp");
        let mut launch = context(root.path());
        launch
            .environment
            .insert("AUTH_SOURCE".into(), "scoped-auth-secret".into());
        let mut config = profile(mode);
        config
            .env_refs
            .insert("TEST_AUTH".into(), "AUTH_SOURCE".into());
        if mode == "preserve_source" {
            config
                .env_refs
                .insert("AUTH_SOURCE".into(), "AUTH_SOURCE".into());
        }
        let result = run_turn(
            &config,
            launch,
            "hello".into(),
            CancellationToken::new(),
            None,
            limits(),
        )
        .await
        .expect("spawn");
        assert!(
            result
                .outcome
                .expect("child verifies scoped environment")
                .succeeded()
        );
    }
}

#[test]
fn acp_adapter_exposes_execution_and_explicit_gaps() {
    use opensymphony::opensymphony_domain::HarnessAdapter;
    let adapter = opensymphony::opensymphony_acp::AcpAdapter;
    assert_eq!(adapter.harness_kind(), "acp");
    let capability = adapter.capabilities();
    assert!(
        capability.available
            && capability.actions.start_run
            && capability.cancellation.acknowledges_cancel
    );
    assert!(!capability.actions.approve && !capability.pause_resume.resume);
    assert!(!capability.feature_gaps.is_empty());
}

fn services_profile(mode: &str) -> AcpProfile {
    let mut profile = profile(mode);
    profile.args[0] = format!(
        "{}/tests/fixtures/acp_services_peer.py",
        env!("CARGO_MANIFEST_DIR")
    );
    profile.auth = None;
    profile
}
fn services_context(root: &Path) -> LaunchContext {
    let mut context = context(root);
    context.services = opensymphony::opensymphony_acp::HostServices {
        read_files: true,
        write_files: true,
        terminals: true,
        ..Default::default()
    };
    context
        .environment
        .insert("HOST_SCOPE".into(), "COE-610".into());
    context
}
fn services_limits() -> ClientLimits {
    ClientLimits {
        file_bytes: 1024,
        terminal_output_bytes: 2048,
        prompt_timeout: Duration::from_secs(10),
        ..limits()
    }
}
#[tokio::test]
async fn acp_client_services_files_enforce_text_limits_and_containment() {
    let root = tempfile::tempdir().expect("root");
    let context = services_context(root.path());
    std::fs::write(context.issue_workspace.join("oversized"), vec![b'x'; 1025]).expect("file");
    std::fs::write(context.issue_workspace.join("invalid-utf8"), [0xff]).expect("file");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.path(), context.issue_workspace.join("outside"))
            .expect("link");
        std::os::unix::fs::symlink(
            root.path().join("nonexistent"),
            context.issue_workspace.join("dangling"),
        )
        .expect("link");
    }
    let run = run_turn(
        &services_profile("files"),
        context,
        "files".into(),
        CancellationToken::new(),
        None,
        services_limits(),
    )
    .await
    .expect("launch");
    assert!(
        run.outcome.expect("file callbacks").succeeded(),
        "{}",
        run.stderr
    );
}
#[tokio::test]
async fn acp_client_services_terminal_output_exit_release_and_host_policy() {
    for mode in ["terminal", "release", "shutdown", "wait_timeout"] {
        let root = tempfile::tempdir().expect("root");
        let run = run_turn(
            &services_profile(mode),
            services_context(root.path()),
            "terminal".into(),
            CancellationToken::new(),
            None,
            ClientLimits {
                callback_timeout: Duration::from_millis(500),
                ..services_limits()
            },
        )
        .await
        .expect("launch");
        assert!(
            run.outcome.expect("terminal callbacks").succeeded(),
            "{}",
            run.stderr
        );
        assert!(run.process_reaped);
    }
}
#[tokio::test]
async fn acp_client_services_cancel_reaps_terminal_without_blocking_dispatch() {
    let root = tempfile::tempdir().expect("root");
    let context = services_context(root.path());
    let marker = context.issue_workspace.join("terminal.pid");
    let cancel = CancellationToken::new();
    let cancellation = cancel.clone();
    let cancelling = async {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("terminal started");
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancellation.cancel();
    };
    let config = services_profile("cancel");
    let (run, ()) = tokio::join!(
        run_turn(
            &config,
            context,
            "cancel".into(),
            cancel,
            None,
            services_limits()
        ),
        cancelling
    );
    let run = run.expect("launch");
    assert!(
        run.outcome.expect("cancelled").cancellation_acknowledged,
        "{}",
        run.stderr
    );
    assert!(run.process_reaped);
    #[cfg(unix)]
    {
        let pid = std::fs::read_to_string(marker)
            .expect("pid")
            .parse::<i32>()
            .expect("pid");
        assert!(
            std::process::Command::new("kill")
                .args(["-0", &pid.to_string()])
                .output()
                .expect("probe")
                .status
                .code()
                .is_some_and(|code| code != 0)
        );
    }
}
#[tokio::test]
async fn acp_client_services_absence_and_session_configuration_are_negotiated() {
    for mode in ["disabled", "config", "legacy_mode", "unsupported_config"] {
        let root = tempfile::tempdir().expect("root");
        let mut profile = services_profile(mode);
        if mode == "config" {
            profile.session.model = Some("second".into());
        }
        if mode == "unsupported_config" {
            profile.session.model = Some("unknown".into());
        }
        if mode == "legacy_mode" {
            profile.session.mode = Some("code".into());
        }
        let run = run_turn(
            &profile,
            context(root.path()),
            "configuration".into(),
            CancellationToken::new(),
            None,
            services_limits(),
        )
        .await
        .expect("launch");
        if mode == "unsupported_config" {
            assert!(matches!(run.outcome, Err(ClientError::Setup(_))));
            assert!(
                !run.evidence
                    .iter()
                    .any(|f| f.payload["method"] == "session/prompt")
            );
        } else {
            let report = run.outcome.expect("configured");
            assert!(report.succeeded(), "{}", run.stderr);
            if mode == "config" {
                assert_eq!(report.configuration.options[0]["currentValue"], "first");
            }
            if mode == "legacy_mode" {
                assert_eq!(report.configuration.current_mode.as_deref(), Some("ask"));
            }
        }
    }
}
#[tokio::test]
async fn acp_scoped_mcp_attachment_requires_negotiated_transport_and_redacts_grants() {
    use agent_client_protocol::schema::v1::{HttpHeader, McpServer, McpServerHttp};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listen");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("peer MCP access");
        let mut data = vec![0; 4096];
        let size = socket.read(&mut data).await.expect("request");
        assert!(
            String::from_utf8_lossy(&data[..size])
                .contains("Authorization: Bearer scoped-grant-610")
        );
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nCOE-610")
            .await
            .expect("response");
    });
    for mode in ["missing_mcp", "mcp"] {
        let root = tempfile::tempdir().expect("root");
        let mut context = context(root.path());
        context.services.mcp_servers.push(McpServer::Http(
            McpServerHttp::new("opensymphony-memory", format!("http://{address}/mcp")).headers(
                vec![
                    HttpHeader::new("Authorization", "Bearer scoped-grant-610"),
                    HttpHeader::new("X-Debug", "3"),
                ],
            ),
        ));
        let run = run_turn(
            &services_profile(mode),
            context,
            "memory".into(),
            CancellationToken::new(),
            None,
            services_limits(),
        )
        .await
        .expect("launch");
        if mode == "missing_mcp" {
            assert!(matches!(run.outcome, Err(ClientError::Setup(_))));
        } else {
            assert!(
                run.outcome.expect("MCP attached").succeeded(),
                "{}",
                run.stderr
            );
        }
        assert!(
            !serde_json::to_string(&run.evidence)
                .expect("evidence")
                .contains("scoped-grant-610")
        );
    }
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("MCP accessed")
        .expect("server");
}

#[tokio::test]
async fn acp_mcp_credential_arguments_are_redacted_from_requests_echoes_and_stderr() {
    use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
    let root = tempfile::tempdir().expect("root");
    let mut context = context(root.path());
    context.services.mcp_servers.push(McpServer::Stdio(
        McpServerStdio::new("memory", "memory-server").args(vec![
            "--token".into(),
            "standalone-oauth-value".into(),
            "--api-key=inline-api-value".into(),
            "--header".into(),
            "Authorization: Bearer generic-header-value".into(),
            "--custom".into(),
            "opaque-generic-arg".into(),
        ]),
    ));
    let run = run_turn(
        &services_profile("mcp_argv"),
        context,
        "memory".into(),
        CancellationToken::new(),
        None,
        services_limits(),
    )
    .await
    .expect("launch");
    assert!(
        run.outcome.expect("stdio attachment").succeeded(),
        "{}",
        run.stderr
    );
    let evidence = serde_json::to_string(&run.evidence).expect("evidence");
    for secret in [
        "standalone-oauth-value",
        "inline-api-value",
        "generic-header-value",
    ] {
        assert!(!evidence.contains(secret));
        assert!(!run.stderr.contains(secret));
    }
    assert!(!evidence.contains("opaque-generic-arg"));
    assert!(evidence.contains("[redacted]"));
}

#[tokio::test]
async fn acp_one_turn_rejects_write_adjacent_to_prompt_response() {
    let root = tempfile::tempdir().expect("root");
    let context = services_context(root.path());
    let marker = context.issue_workspace.join("late-written");
    let run = run_turn(
        &services_profile("late_after_prompt"),
        context,
        "finish".into(),
        CancellationToken::new(),
        None,
        services_limits(),
    )
    .await
    .expect("launch");
    assert!(
        run.outcome.expect("completed").succeeded(),
        "{}",
        run.stderr
    );
    assert!(
        !marker.exists(),
        "post-response callback wrote into workspace"
    );
}

#[test]
fn acp_one_turn_cancels_callback_pending_when_prompt_response_arrives() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let root = tempfile::tempdir().expect("root");
        let context = services_context(root.path());
        let marker = context.issue_workspace.join("late-written");
        let response_sent = context.issue_workspace.join("response-sent");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = started_tx.send(());
            release_rx.recv().expect("release filesystem worker");
        });
        started_rx.await.expect("filesystem worker occupied");
        let run = tokio::spawn(async move {
            run_turn(
                &services_profile("pending_at_response"),
                context,
                "finish".into(),
                CancellationToken::new(),
                None,
                services_limits(),
            )
            .await
        });
        let sent = tokio::time::timeout(Duration::from_secs(3), async {
            while !response_sent.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        release_tx.send(()).expect("release worker");
        blocker.await.expect("blocking worker");
        sent.expect("peer sent adjacent response");
        let run = run.await.expect("run task").expect("launch");
        assert!(
            run.outcome.expect("completed").succeeded(),
            "{}",
            run.stderr
        );
        assert!(!marker.exists(), "callback committed after prompt response");
    });
}

#[tokio::test]
async fn acp_client_services_callback_admission_bounds_pending_waits() {
    let root = tempfile::tempdir().expect("root");
    let run = run_turn(
        &services_profile("wait_flood"),
        services_context(root.path()),
        "flood".into(),
        CancellationToken::new(),
        None,
        ClientLimits {
            pending_callbacks: 2,
            ..services_limits()
        },
    )
    .await
    .expect("launch");
    assert!(matches!(
        run.outcome,
        Err(ClientError::ResourceLimit { submitted: true })
    ));
    assert!(run.process_reaped);
}
#[tokio::test]
async fn acp_client_services_handles_are_connection_scoped_even_with_equal_session_ids() {
    let first_root = tempfile::tempdir().expect("first");
    let first = services_context(first_root.path());
    let marker = first.issue_workspace.join("terminal.id");
    let cancel = CancellationToken::new();
    let cancellation = cancel.clone();
    let owner_profile = services_profile("owner");
    let other = async {
        let id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(id) = tokio::fs::read_to_string(&marker).await
                    && !id.is_empty()
                {
                    break id;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("owner terminal");
        let second_root = tempfile::tempdir().expect("second");
        let mut second = services_context(second_root.path());
        second.environment.insert("OTHER_TERMINAL".into(), id);
        let result = run_turn(
            &services_profile("foreign_handle"),
            second,
            "foreign".into(),
            CancellationToken::new(),
            None,
            services_limits(),
        )
        .await
        .expect("second launch");
        cancellation.cancel();
        assert!(
            result.outcome.expect("foreign denied").succeeded(),
            "{}",
            result.stderr
        );
    };
    let (run, ()) = tokio::join!(
        run_turn(
            &owner_profile,
            first,
            "owner".into(),
            cancel,
            None,
            services_limits()
        ),
        other
    );
    assert!(
        run.expect("owner launch")
            .outcome
            .expect("owner cancelled")
            .cancellation_acknowledged
    );
}

#[test]
fn acp_explicit_session_profile_selections_are_bounded_and_round_trip() {
    let mut profile = services_profile("config");
    profile.session.model = Some("provider/model:revision".into());
    profile
        .session
        .options
        .insert("reasoning".into(), "high".into());
    profile.validate().expect("opaque selections");
    let encoded = serde_yaml::to_string(&profile).expect("serialize");
    let decoded: AcpProfile = serde_yaml::from_str(&encoded).expect("deserialize");
    assert_eq!(decoded.session, profile.session);
    profile.session.model = Some(String::new());
    assert!(profile.validate().is_err());
    profile.session.model = Some("x".repeat(1025));
    assert!(profile.validate().is_err());
    profile.session.model = None;
    profile.session.options = (0..33)
        .map(|i| (format!("option-{i}"), "value".into()))
        .collect();
    assert!(profile.validate().is_err());
}

#[tokio::test]
async fn acp_configuration_applies_model_before_dependent_options() {
    let root = tempfile::tempdir().expect("root");
    let mut profile = services_profile("config_dependent");
    profile.session.model = Some("second".into());
    profile
        .session
        .options
        .insert("a-reasoning".into(), "high".into());
    let run = run_turn(
        &profile,
        context(root.path()),
        "configured".into(),
        CancellationToken::new(),
        None,
        services_limits(),
    )
    .await
    .expect("launch");
    let report = run.outcome.expect("dependent selection applied");
    assert!(report.succeeded(), "{}", run.stderr);
    assert!(
        report
            .configuration
            .options
            .iter()
            .any(|option| option["id"] == "a-reasoning" && option["currentValue"] == "high")
    );
}

#[tokio::test]
async fn acp_configuration_rpc_errors_preserve_classification_and_redact_context() {
    for mode in ["config_auth_error", "config_rpc_error", "legacy_rpc_error"] {
        let root = tempfile::tempdir().expect("root");
        let mut profile = services_profile(mode);
        let mut context = context(root.path());
        context
            .environment
            .insert("TEST_TOKEN".into(), "rpc-secret-610".into());
        if mode.starts_with("legacy") {
            profile.session.mode = Some("code".into());
        } else {
            profile.session.model = Some("second".into());
        }
        let run = run_turn(
            &profile,
            context,
            "unsubmitted".into(),
            CancellationToken::new(),
            None,
            services_limits(),
        )
        .await
        .expect("launch");
        let error = run.outcome.expect_err("configuration RPC failed");
        if mode == "config_auth_error" {
            assert_eq!(
                error,
                ClientError::AuthenticationRequired { submitted: false }
            );
        } else {
            let ClientError::Rpc {
                method,
                code,
                message,
                submitted,
            } = error
            else {
                panic!("expected contextual RPC error: {error:?}");
            };
            assert_eq!(
                method,
                if mode.starts_with("legacy") {
                    "session/set_mode"
                } else {
                    "session/set_config_option"
                }
            );
            assert_eq!(code, -32603);
            assert!(!submitted);
            assert!(message.contains("unavailable"));
            assert!(!message.contains("rpc-secret-610"));
        }
    }
}

#[tokio::test]
async fn acp_configuration_resolves_mode_category_after_model_prerequisite() {
    let root = tempfile::tempdir().expect("root");
    let mut profile = services_profile("config_dependent_mode");
    profile.session.model = Some("second".into());
    profile.session.mode = Some("code".into());
    let run = run_turn(
        &profile,
        context(root.path()),
        "configured".into(),
        CancellationToken::new(),
        None,
        services_limits(),
    )
    .await
    .expect("launch");
    let report = run.outcome.expect("dependent mode applied");
    assert!(report.succeeded(), "{}", run.stderr);
    assert!(
        report
            .configuration
            .options
            .iter()
            .any(|option| option["category"] == "mode" && option["currentValue"] == "code")
    );
}

#[tokio::test]
async fn acp_mcp_urls_reject_embedded_credentials_before_launch() {
    use agent_client_protocol::schema::v1::{McpServer, McpServerHttp, McpServerSse};
    for url in [
        "https://user:password@example.com/mcp",
        "https://example.com/mcp?access_token=url-secret",
        "https://example.com/mcp?api%5fkey=url-secret",
        "https://example.com/mcp?oauth=url-secret",
        "https://example.com/mcp?bearer=url-secret",
        "https://example.com/mcp?key=url-secret",
        "https://example.com/mcp?signature=url-secret",
        "https://example.com/mcp?ordinary=query-value",
        "https://example.com/mcp?",
        "https://example.com/mcp#url-secret",
    ] {
        for sse in [false, true] {
            let root = tempfile::tempdir().expect("root");
            let mut context = context(root.path());
            context.services.mcp_servers.push(if sse {
                McpServer::Sse(McpServerSse::new("memory", url))
            } else {
                McpServer::Http(McpServerHttp::new("memory", url))
            });
            let error = run_turn(
                &services_profile("mcp"),
                context,
                "unused".into(),
                CancellationToken::new(),
                None,
                services_limits(),
            )
            .await
            .expect_err("URL rejected before source capture");
            assert!(matches!(error, ClientError::InvalidConfiguration(_)));
            assert!(!error.to_string().contains("url-secret"));
        }
    }
}

#[tokio::test]
async fn acp_filesystem_payloads_stay_on_wire_and_out_of_evidence() {
    let root = tempfile::tempdir().expect("root");
    let context = services_context(root.path());
    std::fs::write(
        context.issue_workspace.join("private-file"),
        "opaque_workspace_payload_610",
    )
    .expect("private file");
    let run = run_turn(
        &services_profile("file_privacy"),
        context,
        "files".into(),
        CancellationToken::new(),
        None,
        services_limits(),
    )
    .await
    .expect("launch");
    assert!(
        run.outcome
            .expect("peer received original content")
            .succeeded()
    );
    let evidence = serde_json::to_string(&run.evidence).expect("evidence");
    assert!(!evidence.contains("opaque_workspace_payload_610"));
    assert!(
        run.evidence
            .iter()
            .any(|frame| frame.payload["result"]["content"] == "[redacted]")
    );
    assert!(
        run.evidence
            .iter()
            .any(|frame| frame.payload["method"] == "fs/write_text_file"
                && frame.payload["params"]["content"] == "[redacted]")
    );
    assert!(
        run.evidence
            .iter()
            .any(|frame| frame.payload["result"]["output"] == "[redacted]")
    );
}

#[tokio::test]
async fn acp_mcp_aliases_cannot_forward_resolved_excluded_checkout_values() {
    use agent_client_protocol::schema::v1::{
        EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerSse, McpServerStdio,
    };
    for server in [
        McpServer::Stdio(McpServerStdio::new("memory", "memory-server").env(vec![
            EnvVariable::new("ACCESS_TOKEN", "checkout-secret-value"),
        ])),
        McpServer::Stdio(
            McpServerStdio::new("memory", "memory-server")
                .args(vec!["--token=checkout-secret-value".into()]),
        ),
        McpServer::Http(
            McpServerHttp::new("memory", "https://example.test/mcp").headers(vec![
                HttpHeader::new("Authorization", "Bearer checkout-secret-value"),
            ]),
        ),
        McpServer::Sse(
            McpServerSse::new("memory", "https://example.test/mcp")
                .headers(vec![HttpHeader::new("X-Grant", "checkout-secret-value")]),
        ),
    ] {
        let root = tempfile::tempdir().expect("root");
        let mut context = context(root.path());
        context.services.mcp_servers.push(server);
        let error = run_turn(
            &services_profile("mcp"),
            context,
            "unused".into(),
            CancellationToken::new(),
            None,
            services_limits(),
        )
        .await
        .expect_err("excluded value rejected before launch");
        assert!(matches!(error, ClientError::InvalidConfiguration(_)));
        assert!(!error.to_string().contains("checkout-secret-value"));
    }
}

#[tokio::test]
async fn acp_encoded_facility_output_respects_small_response_budgets() {
    let root = tempfile::tempdir().expect("root");
    let context = services_context(root.path());
    std::fs::write(context.issue_workspace.join("escaped"), vec![0; 2048]).expect("escaped");
    std::fs::write(context.issue_workspace.join("small"), "ok").expect("small");
    let run = run_turn(
        &services_profile("response_budget"),
        context,
        "bounded".into(),
        CancellationToken::new(),
        None,
        ClientLimits {
            file_bytes: 4096,
            terminal_output_bytes: 4096,
            frame_bytes: 1024,
            callback_bytes: 1024,
            ..services_limits()
        },
    )
    .await
    .expect("launch");
    assert!(
        run.outcome
            .expect("oversized callbacks do not tear down transport")
            .succeeded(),
        "{}",
        run.stderr
    );
    assert!(run.process_reaped);
}
