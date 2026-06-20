use opensymphony::opensymphony_codex::{
    CodexAppServerAdapter, CodexAppServerLaunch, CodexApprovalDecision, CodexContractArtifact,
    CodexContractGeneration, CodexJsonRpcSession, CodexLifecycleRequest, CodexThreadStartParams,
    CodexTurnStartParams, CodexUserInput, CodexWebSocketAuth, NormalizedCodexEventKind,
    normalize_server_notification, normalized_event_to_journal_record,
    websocket_benchmark_requirements,
};
use opensymphony::opensymphony_domain::HarnessAdapter;
use opensymphony::opensymphony_gateway_schema::event_journal::EventKind;
use opensymphony::opensymphony_gateway_schema::model_settings::{
    CredentialReferenceKind, CredentialStorageMode, ModelSettingsResponse,
};
use serde_json::json;

#[test]
fn codex_stdio_launch_and_json_rpc_request_shape_are_stable() {
    let launch = CodexAppServerLaunch::stdio_with_program("codex-test");
    let (program, args) = launch.to_command();
    assert_eq!(program, "codex-test");
    assert_eq!(args, vec!["app-server", "--stdio"]);
    assert_eq!(launch.program(), "codex-test");
    assert_eq!(launch.command_args(), vec!["app-server", "--stdio"]);

    let mut session = CodexJsonRpcSession::new("opensymphony-test", "0.0.0");
    let initialize = session.initialize();
    assert_eq!(initialize.id, 1);
    assert_eq!(initialize.method, "initialize");
    assert_eq!(initialize.params["clientInfo"]["name"], "opensymphony-test");

    let thread = session
        .thread_start(CodexThreadStartParams {
            cwd: Some("/tmp/issue-workspace".into()),
            model: Some("gpt-5-codex".into()),
            model_provider: Some("openai".into()),
            base_instructions: Some("OpenSymphony workflow prompt".into()),
            developer_instructions: None,
            ephemeral: Some(true),
            config: Some(json!({ "model": "gpt-5-codex" })),
        })
        .expect("serialize thread/start request");
    assert_eq!(thread.id, 2);
    assert_eq!(thread.method, "thread/start");
    assert_eq!(thread.params["cwd"], "/tmp/issue-workspace");
    assert_eq!(thread.params["model"], "gpt-5-codex");

    let turn = session
        .turn_start(CodexTurnStartParams {
            thread_id: "thread-1".into(),
            input: vec![CodexUserInput::Text {
                text: "continue".into(),
                text_elements: Vec::new(),
            }],
            cwd: Some("/tmp/issue-workspace".into()),
            model: Some("gpt-5-codex".into()),
            client_user_message_id: Some("client-msg-1".into()),
        })
        .expect("serialize turn/start request");
    assert_eq!(turn.id, 3);
    assert_eq!(turn.method, "turn/start");
    assert_eq!(turn.params["threadId"], "thread-1");

    let encoded = CodexJsonRpcSession::encode_line(&turn).expect("encode JSON-RPC request");
    assert!(encoded.ends_with('\n'));
    assert!(encoded.contains("\"jsonrpc\":\"2.0\""));
}

#[test]
fn codex_notification_normalization_preserves_thread_turn_and_raw_payload() {
    let raw = json!({
        "jsonrpc": "2.0",
        "method": "item/agentMessage/delta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "item-1",
            "delta": "hello"
        }
    });

    let event = normalize_server_notification(raw.clone()).expect("notification normalizes");
    assert_eq!(event.kind, NormalizedCodexEventKind::AgentMessageDelta);
    assert_eq!(event.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(event.item_id.as_deref(), Some("item-1"));
    assert_eq!(event.message_delta.as_deref(), Some("hello"));
    assert_eq!(event.raw, raw);

    let unknown = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "future/event",
        "params": { "threadId": "thread-2" }
    }))
    .expect("unknown notifications are retained");
    assert_eq!(unknown.kind, NormalizedCodexEventKind::Unknown);
    assert_eq!(unknown.thread_id.as_deref(), Some("thread-2"));

    assert!(
        normalize_server_notification(json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "item/permissions/requestApproval",
            "params": {}
        }))
        .is_none()
    );
}

#[test]
fn codex_adapter_exposes_supported_local_harness_capabilities() {
    let adapter = CodexAppServerAdapter::local_stdio("codex-test", "opensymphony-test", "1.10.1");
    assert_eq!(adapter.harness_kind(), "codex_app_server");
    assert_eq!(
        adapter.launch().command_args(),
        vec!["app-server", "--stdio"]
    );

    let capabilities = adapter.capabilities();
    assert!(capabilities.available);
    assert_eq!(
        capabilities.runtime_contract_version.as_deref(),
        Some("codex-app-server-json-rpc-v2")
    );
    assert_eq!(capabilities.transport.modes, vec!["stdio"]);
    assert!(capabilities.actions.start_run);
    assert!(capabilities.actions.cancel);
    assert!(capabilities.actions.approve);
    assert!(capabilities.approvals.tool_approval);
    assert!(capabilities.history.preserve_unknown_events);
    assert!(!capabilities.transport.remote);
}

#[test]
fn codex_lifecycle_requests_cover_start_resume_cancel_and_approval() {
    let adapter = CodexAppServerAdapter::local_stdio("codex-test", "opensymphony-test", "1.10.1");
    let mut session = adapter.session();

    let start = adapter
        .start_issue_request(
            &mut session,
            "/tmp/issue-workspace",
            "gpt-5-codex",
            "workflow prompt",
            json!({ "approvalPolicy": "on-request" }),
        )
        .expect("start request serializes");
    assert_eq!(start.lifecycle, CodexLifecycleRequest::Start);
    assert_eq!(start.request.method, "thread/start");
    assert_eq!(start.request.params["cwd"], "/tmp/issue-workspace");
    assert_eq!(start.request.params["baseInstructions"], "workflow prompt");

    let resume = adapter
        .resume_issue_request(&mut session, "thread-1", "/tmp/issue-workspace", "continue")
        .expect("resume request serializes");
    assert_eq!(resume.lifecycle, CodexLifecycleRequest::Resume);
    assert_eq!(resume.request.method, "turn/start");
    assert_eq!(resume.request.params["threadId"], "thread-1");

    let cancel = adapter.cancel_turn_request(&mut session, "turn-1");
    assert_eq!(cancel.lifecycle, CodexLifecycleRequest::Cancel);
    assert_eq!(cancel.request.method, "turn/cancel");
    assert_eq!(cancel.request.params["turnId"], "turn-1");

    let approve = adapter.approval_request(
        &mut session,
        "approval-1",
        CodexApprovalDecision::Approve,
        Some("operator approved".into()),
    );
    assert_eq!(approve.lifecycle, CodexLifecycleRequest::Approval);
    assert_eq!(approve.request.method, "approval/respond");
    assert_eq!(approve.request.params["decision"], "approve");

    let reject = adapter.approval_request(
        &mut session,
        "approval-2",
        CodexApprovalDecision::Reject,
        None,
    );
    assert_eq!(reject.request.params["decision"], "reject");
}

#[test]
fn codex_contract_generation_commands_are_explicit() {
    let schema = CodexContractGeneration::json_schema_with_program(
        "codex-test",
        std::env::temp_dir().join("opensymphony-codex-schema"),
    );
    let (program, schema_args) = schema.to_command();
    assert_eq!(program, "codex-test");
    assert_eq!(schema.artifact(), CodexContractArtifact::JsonSchema);
    assert_eq!(schema_args[0], "app-server");
    assert_eq!(schema_args[1], "generate-json-schema");
    assert_eq!(schema_args[2], "--out");

    let ts = CodexContractGeneration::typescript_with_program(
        "codex-test",
        std::env::temp_dir().join("opensymphony-codex-ts"),
    );
    let (_, ts_args) = ts.to_command();
    assert_eq!(ts.artifact(), CodexContractArtifact::TypeScript);
    assert_eq!(ts_args[1], "generate-ts");
}

#[test]
fn codex_events_map_to_journal_surfaces_with_raw_payload_refs() {
    let approval_raw = json!({
        "jsonrpc": "2.0",
        "method": "item/permissions/requestApproval",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-1",
            "title": "Run shell command"
        }
    });
    let approval =
        normalize_server_notification(approval_raw).expect("approval notification normalizes");
    assert_eq!(approval.kind, NormalizedCodexEventKind::ApprovalRequested);

    let approval_record = normalized_event_to_journal_record("COE-476", 7, &approval);
    assert_eq!(approval_record.kind, EventKind::ApprovalRequested);
    assert_eq!(approval_record.actor.actor_id(), "codex_app_server");
    assert_eq!(
        approval_record.raw_payload_ref.as_deref(),
        Some("codex:COE-476:7")
    );
    assert!(approval_record.summary.contains("Codex requested approval"));

    let completed = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "turn/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1"
        }
    }))
    .expect("completion normalizes");
    let completed_record = normalized_event_to_journal_record("COE-476", 8, &completed);
    assert_eq!(completed_record.kind, EventKind::RunCompleted);

    let failed = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "error",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "message": "missing login"
        }
    }))
    .expect("error normalizes");
    let failed_record = normalized_event_to_journal_record("COE-476", 9, &failed);
    assert_eq!(failed_record.kind, EventKind::RunFailed);
    assert!(failed_record.summary.contains("missing login"));

    let canceled = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "turn/cancelled",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1"
        }
    }))
    .expect("cancel notification normalizes");
    let canceled_record = normalized_event_to_journal_record("COE-476", 10, &canceled);
    assert_eq!(canceled_record.kind, EventKind::RunCancelled);
}

#[test]
fn codex_model_and_credential_reuse_maps_existing_settings_profiles() {
    let settings = ModelSettingsResponse::local_default(true);
    let codex_profiles = settings
        .profiles
        .iter()
        .filter_map(opensymphony::opensymphony_codex::CodexModelCredentialReuse::from_profile)
        .collect::<Vec<_>>();

    assert!(codex_profiles.iter().any(|profile| {
        profile.profile_id == "codex-chatgpt-local-keychain"
            && profile.can_supply_subscription_credentials
            && profile.credential_reference_kind == CredentialReferenceKind::CodexCliLogin
            && profile.storage_mode == CredentialStorageMode::CodexCliHome
    }));
    assert!(codex_profiles.iter().any(|profile| {
        profile.profile_id == "hosted-openai-subscription-broker"
            && profile.credential_reference_kind == CredentialReferenceKind::HostedBrokerReference
            && profile.storage_mode == CredentialStorageMode::HostedBroker
    }));
    assert!(
        codex_profiles
            .iter()
            .all(|profile| !profile.credential_reference_id.is_empty())
    );
}

#[test]
fn codex_websocket_auth_and_benchmark_dimensions_are_explicit() {
    let mut stdio_with_auth = CodexAppServerLaunch::stdio();
    stdio_with_auth.websocket_auth = Some(CodexWebSocketAuth::CapabilityToken {
        token_file: std::env::temp_dir().join("opensymphony-codex-stdio-token"),
        token_sha256: "1".repeat(64),
    });
    assert_eq!(
        stdio_with_auth.command_args(),
        vec!["app-server", "--stdio"]
    );

    let mut launch = CodexAppServerLaunch::loopback_websocket(18765);
    launch.extra_args = vec!["--stdio".into()];
    launch.websocket_auth = Some(CodexWebSocketAuth::CapabilityToken {
        token_file: std::env::temp_dir().join("opensymphony-codex-token"),
        token_sha256: "0".repeat(64),
    });
    let args = launch.command_args();
    assert!(args.contains(&"--listen".to_owned()));
    assert!(args.contains(&"ws://127.0.0.1:18765".to_owned()));
    assert!(args.contains(&"--ws-auth".to_owned()));
    assert!(args.contains(&"capability-token".to_owned()));
    assert!(args.contains(&"--ws-token-file".to_owned()));
    assert!(args.contains(&"--ws-token-sha256".to_owned()));
    let extra_stdio = args
        .iter()
        .position(|arg| arg == "--stdio")
        .expect("extra stdio arg is present");
    let selected_transport = args
        .iter()
        .position(|arg| arg == "--listen")
        .expect("selected websocket transport is present");
    assert!(selected_transport > extra_stdio);

    let mut signed_bearer = CodexAppServerLaunch::loopback_websocket(18766);
    signed_bearer.websocket_auth = Some(CodexWebSocketAuth::SignedBearerToken {
        shared_secret_file: std::env::temp_dir().join("opensymphony-codex-jwt-secret"),
        issuer: "opensymphony".into(),
        audience: "codex-app-server".into(),
        max_clock_skew_seconds: Some(30),
    });
    let signed_bearer_args = signed_bearer.command_args();
    assert!(signed_bearer_args.contains(&"--ws-auth".to_owned()));
    assert!(signed_bearer_args.contains(&"signed-bearer-token".to_owned()));
    assert!(signed_bearer_args.contains(&"--ws-shared-secret-file".to_owned()));
    assert!(signed_bearer_args.contains(&"--ws-issuer".to_owned()));
    assert!(signed_bearer_args.contains(&"opensymphony".to_owned()));
    assert!(signed_bearer_args.contains(&"--ws-audience".to_owned()));
    assert!(signed_bearer_args.contains(&"codex-app-server".to_owned()));
    assert!(signed_bearer_args.contains(&"--ws-max-clock-skew-seconds".to_owned()));
    assert!(signed_bearer_args.contains(&"30".to_owned()));

    let dimensions = websocket_benchmark_requirements()
        .into_iter()
        .map(|requirement| requirement.dimension)
        .collect::<Vec<_>>();
    assert_eq!(
        dimensions,
        vec![
            "throughput",
            "queue behavior",
            "reconnect",
            "secure exposure"
        ]
    );
}
