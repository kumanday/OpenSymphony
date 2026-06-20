#![cfg(feature = "codex-app-server-prototype")]

use opensymphony::opensymphony_codex::{
    CodexAppServerLaunch, CodexJsonRpcSession, CodexThreadStartParams, CodexTurnStartParams,
    CodexUserInput, CodexWebSocketAuth, NormalizedCodexEventKind, normalize_server_notification,
    websocket_benchmark_requirements,
};
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
