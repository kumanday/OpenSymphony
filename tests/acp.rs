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
    for case in 0..5 {
        let mut config = profile("complete");
        let mut launch = context(root.path());
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
            _ => config.args.push("--token=secret".into()),
        }
        assert!(
            run_turn(
                &config,
                launch,
                "hello".into(),
                CancellationToken::new(),
                None,
                limits()
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
            "harness_profile: fake\n  model: some-model",
        ),
    ] {
        let workflow =
            WorkflowDefinition::parse(&source.replace(old, new)).expect("parse invalid semantics");
        assert!(workflow.resolve(Path::new("/repo"), &env).is_err(), "{new}");
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
    for auth in [false, true] {
        let root = tempfile::tempdir().expect("temp");
        let mut config = profile("complete");
        if auth {
            config.auth = Some(AcpAuth {
                method_id: "not-advertised".into(),
            });
        } else {
            config.required_capabilities = vec!["prompt.image".into()];
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
