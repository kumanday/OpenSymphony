use opensymphony::{opensymphony_acp::*, opensymphony_workspace::*};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

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
    }
}
async fn retire(handle: &SessionHandle) {
    handle
        .control(SessionControl::Retire)
        .await
        .expect("quiescent retirement");
}
async fn prompt(handle: &SessionHandle, run: &str, text: &str) -> TurnReport {
    handle
        .prompt(run.into(), 1, text.into(), CancellationToken::new())
        .await
        .expect("prompt")
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

#[tokio::test]
async fn retention_leases_expire_and_resource_limits_refuse_active_owners() {
    let root = tempfile::tempdir().expect("temp");
    let host = SessionHost::new(RetentionPolicy {
        max_sessions: 1,
        idle_timeout: Duration::from_millis(100),
        attachment_ttl: Duration::from_millis(300),
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
    tokio::time::sleep(Duration::from_millis(380)).await;
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
                McpServerStdio::new("memory", "memory-server")
                    .args(vec!["--token".into(), "retained-scope-grant".into()]),
            ));
            let handle = host.open(launch).await.expect("open or restore");
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
