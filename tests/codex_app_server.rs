use chrono::{TimeZone, Utc};
use opensymphony::opensymphony_codex::{
    CodexAppServerAdapter, CodexAppServerLaunch, CodexAppServerSchemaValidator,
    CodexApprovalDecision, CodexApprovalPolicy, CodexContractArtifact, CodexContractGeneration,
    CodexJsonRpcSession, CodexLifecycleRequest, CodexSandboxPolicy, CodexThreadSandboxMode,
    CodexThreadStartParams, CodexTokenUsage, CodexTurnStartParams, CodexUserInput,
    CodexWebSocketAuth, NormalizedCodexEventKind, codex_approval_decision_audit_record,
    codex_approval_request_from_event, codex_event_summary, normalize_server_notification,
    normalized_event_to_journal_record, websocket_benchmark_requirements,
};
use opensymphony::opensymphony_domain::HarnessAdapter;
use opensymphony::opensymphony_gateway_schema::approval::{
    ApprovalKind, ApprovalRiskLevel, ApprovalStatus,
};
use opensymphony::opensymphony_gateway_schema::event_journal::EventKind;
use opensymphony::opensymphony_gateway_schema::model_settings::{
    CredentialReferenceKind, CredentialStorageMode, ModelSettingsResponse,
};
use serde_json::json;

async fn terminate_codex_child(
    mut child: tokio::process::Child,
    stderr_task: tokio::task::JoinHandle<String>,
) -> (Option<std::process::ExitStatus>, String) {
    child.kill().await.ok();
    let status = child.wait().await.ok();
    let stderr = stderr_task.await.unwrap_or_default();
    (status, stderr)
}

#[tokio::test]
async fn codex_live_stdio_initializes_when_requested() {
    if std::env::var_os("OPENSYMPHONY_CODEX_LIVE_STDIO").is_none() {
        eprintln!("set OPENSYMPHONY_CODEX_LIVE_STDIO=1 to launch the local Codex CLI");
        return;
    }

    let codex = std::env::var("OPENSYMPHONY_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let adapter = CodexAppServerAdapter::local_stdio(&codex, "opensymphony-live-test", "0.0.0");
    let (program, args) = adapter.launch().to_command();
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("launch codex app-server stdio");

    let mut session = adapter.session();
    let initialize = session.initialize();
    let line = CodexJsonRpcSession::encode_line(&initialize).expect("encode initialize");
    let mut stdin = child.stdin.take().expect("child stdin");
    let mut stderr = child.stderr.take().expect("child stderr");
    let stderr_task = tokio::spawn(async move {
        let mut output = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut output).await;
        output
    });
    tokio::io::AsyncWriteExt::write_all(&mut stdin, line.as_bytes())
        .await
        .expect("write initialize");

    let stdout = child.stdout.take().expect("child stdout");
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut response = String::new();
    let read = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut response),
    )
    .await;
    match read {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            let (status, stderr) = terminate_codex_child(child, stderr_task).await;
            panic!("read initialize response: {error}; status={status:?}; stderr={stderr}");
        }
        Err(error) => {
            let (status, stderr) = terminate_codex_child(child, stderr_task).await;
            panic!("initialize response timeout: {error}; status={status:?}; stderr={stderr}");
        }
    }

    drop(stdin);
    let (status, stderr) = terminate_codex_child(child, stderr_task).await;
    assert!(
        !response.trim().is_empty(),
        "initialize should emit a JSON-RPC response line; status={status:?}; stderr={stderr}"
    );
    let response: serde_json::Value = serde_json::from_str(response.trim())
        .unwrap_or_else(|error| panic!("initialize response is json: {error}; stderr={stderr}"));
    assert_eq!(response["id"], initialize.id);
    assert!(
        response.get("result").is_some(),
        "initialize should return a JSON-RPC result: {response}; status={status:?}; stderr={stderr}"
    );
}

#[test]
fn codex_stdio_launch_and_json_rpc_request_shape_are_stable() {
    let launch = CodexAppServerLaunch::stdio_with_program("codex-test");
    let (program, args) = launch.to_command();
    assert_eq!(program, "codex-test");
    assert_eq!(
        args,
        vec!["--dangerously-bypass-hook-trust", "app-server", "--stdio"]
    );
    assert_eq!(launch.program(), "codex-test");
    assert_eq!(
        launch.command_args(),
        vec!["--dangerously-bypass-hook-trust", "app-server", "--stdio"]
    );

    let mut session = CodexJsonRpcSession::new("opensymphony-test", "0.0.0");
    let initialize = session.initialize();
    assert_eq!(initialize.id, 1);
    assert_eq!(initialize.method, "initialize");
    assert_eq!(initialize.params["clientInfo"]["name"], "opensymphony-test");

    let thread = session
        .thread_start(CodexThreadStartParams {
            approval_policy: Some(CodexApprovalPolicy::Never),
            cwd: Some("/tmp/issue-workspace".into()),
            model: Some("gpt-5-codex".into()),
            model_provider: Some("openai".into()),
            base_instructions: Some("OpenSymphony workflow prompt".into()),
            developer_instructions: None,
            ephemeral: Some(true),
            sandbox: Some(CodexThreadSandboxMode::DangerFullAccess),
            config: Some(json!({ "model": "gpt-5-codex" })),
        })
        .expect("serialize thread/start request");
    assert_eq!(thread.id, 2);
    assert_eq!(thread.method, "thread/start");
    assert_eq!(thread.params["approvalPolicy"], "never");
    assert_eq!(thread.params["cwd"], "/tmp/issue-workspace");
    assert_eq!(thread.params["model"], "gpt-5-codex");
    assert_eq!(thread.params["sandbox"], "danger-full-access");

    let turn = session
        .turn_start(CodexTurnStartParams {
            thread_id: "thread-1".into(),
            input: vec![CodexUserInput::Text {
                text: "continue".into(),
                text_elements: Vec::new(),
            }],
            approval_policy: Some(CodexApprovalPolicy::Never),
            cwd: Some("/tmp/issue-workspace".into()),
            model: Some("gpt-5-codex".into()),
            sandbox_policy: Some(CodexSandboxPolicy::danger_full_access()),
            client_user_message_id: Some("client-msg-1".into()),
        })
        .expect("serialize turn/start request");
    assert_eq!(turn.id, 3);
    assert_eq!(turn.method, "turn/start");
    assert_eq!(turn.params["threadId"], "thread-1");
    assert_eq!(turn.params["approvalPolicy"], "never");
    assert_eq!(turn.params["sandboxPolicy"]["type"], "dangerFullAccess");
    assert!(turn.params["sandboxPolicy"].get("networkAccess").is_none());

    let encoded = CodexJsonRpcSession::encode_line(&turn).expect("encode JSON-RPC request");
    assert!(encoded.ends_with('\n'));
    assert!(encoded.contains("\"jsonrpc\":\"2.0\""));
}

#[test]
fn codex_schema_validator_rejects_drifted_automation_payloads() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "definitions": {
            "ClientRequest": {
                "oneOf": [
                    {
                        "type": "object",
                        "required": ["jsonrpc", "id", "method", "params"],
                        "properties": {
                            "jsonrpc": { "const": "2.0" },
                            "id": { "type": "integer" },
                            "method": { "enum": ["turn/start"] },
                            "params": {
                                "type": "object",
                                "required": ["approvalPolicy", "sandboxPolicy", "threadId", "input"],
                                "properties": {
                                    "approvalPolicy": { "enum": ["never"] },
                                    "threadId": { "type": "string" },
                                    "input": { "type": "array" },
                                    "sandboxPolicy": {
                                        "type": "object",
                                        "required": ["type"],
                                        "properties": {
                                            "type": { "enum": ["dangerFullAccess"] }
                                        },
                                        "additionalProperties": false
                                    }
                                }
                            }
                        }
                    }
                ]
            }
        }
    });
    let validator =
        CodexAppServerSchemaValidator::from_schema_json(schema).expect("schema should compile");
    let mut session = CodexJsonRpcSession::new("opensymphony-test", "0.0.0");
    let turn = session
        .turn_start(CodexTurnStartParams {
            thread_id: "thread-1".into(),
            input: vec![CodexUserInput::Text {
                text: "continue".into(),
                text_elements: Vec::new(),
            }],
            approval_policy: Some(CodexApprovalPolicy::Never),
            cwd: Some("/tmp/issue-workspace".into()),
            model: Some("gpt-5-codex".into()),
            sandbox_policy: Some(CodexSandboxPolicy::danger_full_access()),
            client_user_message_id: None,
        })
        .expect("turn/start serializes");
    validator
        .validate_request(&turn)
        .expect("maximum-permission turn/start shape should match schema");

    let mut drifted = turn.clone();
    drifted.params["sandboxPolicy"]["networkAccess"] = json!(true);
    let error = validator
        .validate_request(&drifted)
        .expect_err("dangerFullAccess must not carry networkAccess");
    assert!(error.to_string().contains("Update Codex"));
}

#[test]
fn codex_schema_validator_accepts_defs_client_request_shape() {
    let schema = json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "$defs": {
            "JsonRpcVersion": { "const": "2.0" },
            "ClientRequest": {
                "type": "object",
                "required": ["jsonrpc", "id", "method", "params"],
                "properties": {
                    "jsonrpc": { "$ref": "#/$defs/JsonRpcVersion" },
                    "id": { "type": "integer" },
                    "method": { "enum": ["initialize"] },
                    "params": { "type": "object" }
                }
            }
        }
    });
    let validator = CodexAppServerSchemaValidator::from_schema_json(schema)
        .expect("$defs schema should compile");
    let mut session = CodexJsonRpcSession::new("opensymphony-test", "0.0.0");
    let initialize = session.initialize();

    validator
        .validate_request(&initialize)
        .expect("initialize should match $defs ClientRequest schema");
}

#[test]
fn codex_installed_schema_accepts_automation_payloads_when_requested() {
    if std::env::var_os("OPENSYMPHONY_CODEX_LIVE_SCHEMA").is_none() {
        eprintln!("set OPENSYMPHONY_CODEX_LIVE_SCHEMA=1 to validate against installed Codex");
        return;
    }

    let codex = std::env::var("OPENSYMPHONY_CODEX_BIN").unwrap_or_else(|_| "codex".into());
    let schema_dir = tempfile::tempdir().expect("schema tempdir should exist");
    let generation = CodexContractGeneration::json_schema_with_program(&codex, schema_dir.path());
    let (program, args) = generation.to_command();
    let output = std::process::Command::new(&program)
        .args(&args)
        .output()
        .expect("codex schema generation should launch");
    assert!(
        output.status.success(),
        "schema generation should succeed; status={}; stderr-bytes={}",
        output.status,
        output.stderr.len()
    );
    let validator = CodexAppServerSchemaValidator::from_schema_file(
        schema_dir
            .path()
            .join("codex_app_server_protocol.v2.schemas.json"),
    )
    .expect("installed schema should compile");

    let adapter = CodexAppServerAdapter::local_stdio(&codex, "opensymphony-live-test", "0.0.0");
    let mut session = adapter.session();
    let initialize = session.initialize();
    validator
        .validate_request(&initialize)
        .expect("initialize should match installed schema");
    let thread = adapter
        .start_issue_thread_request(
            &mut session,
            "/tmp/issue-workspace",
            Some("gpt-5-codex".into()),
            json!({ "opensymphonyRoute": { "harness": "codex_app_server" } }),
        )
        .expect("thread/start should serialize");
    assert!(
        thread.request.params.get("modelProvider").is_none(),
        "selected model should not force a Codex provider"
    );
    validator
        .validate_request(&thread.request)
        .expect("thread/start should match installed schema");
    let turn = adapter
        .start_issue_turn_request(
            &mut session,
            "thread-1",
            "/tmp/issue-workspace",
            Some("gpt-5-codex".into()),
            "workflow prompt",
        )
        .expect("turn/start should serialize");
    validator
        .validate_request(&turn.request)
        .expect("turn/start should match installed schema");
    let default_thread = adapter
        .start_issue_thread_request(
            &mut session,
            "/tmp/issue-workspace",
            None,
            json!({ "opensymphonyRoute": { "harness": "codex_app_server" } }),
        )
        .expect("thread/start without selected model should serialize");
    assert!(
        default_thread.request.params.get("model").is_none(),
        "omitted model should leave Codex model selection to its own config"
    );
    assert!(
        default_thread.request.params.get("modelProvider").is_none(),
        "omitted model should not force a Codex provider"
    );
    validator
        .validate_request(&default_thread.request)
        .expect("thread/start without model should match installed schema");
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
fn codex_token_usage_notification_maps_to_normalized_usage_payload() {
    let event = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": {
                "last": {
                    "cachedInputTokens": 4,
                    "inputTokens": 10,
                    "outputTokens": 20,
                    "reasoningOutputTokens": 2,
                    "totalTokens": 30
                },
                "total": {
                    "cachedInputTokens": 40,
                    "inputTokens": 100,
                    "outputTokens": 200,
                    "reasoningOutputTokens": 20,
                    "totalTokens": 300
                }
            }
        }
    }))
    .expect("token usage notification normalizes");

    assert_eq!(event.kind, NormalizedCodexEventKind::TokenUsageUpdated);
    assert_eq!(event.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(
        event.token_usage,
        Some(CodexTokenUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 40,
            total_tokens: 300,
        })
    );

    let record = normalized_event_to_journal_record("COE-482", 21, &event);
    let payload = record.payload.expect("journal record should carry payload");
    assert_eq!(payload["usage"]["input_tokens"], 100);
    assert_eq!(payload["usage"]["output_tokens"], 200);
    assert_eq!(payload["usage"]["cache_read_tokens"], 40);
    assert_eq!(payload["usage"]["total_tokens"], 300);
    assert_eq!(
        payload["raw_payload"]["params"]["tokenUsage"]["total"]["cachedInputTokens"],
        40
    );

    let sparse = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": { "total": { "totalTokens": 9 } }
        }
    }))
    .expect("sparse token usage notification normalizes");
    assert_eq!(
        sparse.token_usage,
        Some(CodexTokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 9,
        })
    );

    let missing_total = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": {
                "total": {
                    "cachedInputTokens": 3,
                    "inputTokens": 5,
                    "outputTokens": 7
                }
            }
        }
    }))
    .expect("token usage without explicit total normalizes");
    assert_eq!(
        missing_total.token_usage,
        Some(CodexTokenUsage {
            input_tokens: 5,
            output_tokens: 7,
            cache_read_tokens: 3,
            total_tokens: 15,
        })
    );

    let empty = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "tokenUsage": { "total": {} }
        }
    }))
    .expect("missing token fields should not panic");
    assert_eq!(empty.token_usage, None);
}

#[test]
fn codex_event_summaries_extract_bounded_redacted_previews() {
    let message = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/agentMessage/delta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "item-1",
            "delta": "Here is the answer\napi_key=sk-live-secret"
        }
    }))
    .expect("message delta normalizes");
    assert_eq!(
        codex_event_summary(&message),
        "Codex assistant: Here is the answer api_key=[redacted]"
    );
    let message_record = normalized_event_to_journal_record("COE-483", 1, &message);
    assert_eq!(
        message_record.summary,
        "Codex assistant: Here is the answer api_key=[redacted]"
    );

    let command = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-1",
            "delta": "running tests with Authorization: bearer-token"
        }
    }))
    .expect("command output normalizes");
    assert_eq!(
        codex_event_summary(&command),
        "Codex command output: running tests with Authorization:[redacted]"
    );

    let authorization_scheme = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-authorization",
            "delta": "curl -H Authorization: Bearer abc123"
        }
    }))
    .expect("authorization scheme command output normalizes");
    assert_eq!(
        codex_event_summary(&authorization_scheme),
        "Codex command output: curl -H Authorization:[redacted]"
    );

    let custom_authorization_scheme = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-custom-authorization",
            "delta": "curl -H Authorization: Digest abc123"
        }
    }))
    .expect("custom authorization scheme command output normalizes");
    assert_eq!(
        codex_event_summary(&custom_authorization_scheme),
        "Codex command output: curl -H Authorization:[redacted]"
    );

    let token_authorization_scheme = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-token-authorization",
            "delta": "curl -H Authorization: Token abc123"
        }
    }))
    .expect("token authorization scheme command output normalizes");
    assert_eq!(
        codex_event_summary(&token_authorization_scheme),
        "Codex command output: curl -H Authorization:[redacted]"
    );

    let compact_authorization_scheme = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-compact-authorization",
            "delta": "curl -H Authorization:Token abc123"
        }
    }))
    .expect("compact authorization scheme command output normalizes");
    assert_eq!(
        codex_event_summary(&compact_authorization_scheme),
        "Codex command output: curl -H Authorization:[redacted]"
    );

    let bare_authorization_scheme = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-bare-authorization",
            "delta": "curl -H Authorization Bearer abc123"
        }
    }))
    .expect("bare authorization scheme command output normalizes");
    assert_eq!(
        codex_event_summary(&bare_authorization_scheme),
        "Codex command output: curl -H Authorization [redacted]"
    );

    let password = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-password",
            "delta": "login password: hunter2"
        }
    }))
    .expect("password command output normalizes");
    assert_eq!(
        codex_event_summary(&password),
        "Codex command output: login password:[redacted]"
    );

    let multi_word_password = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-password-phrase",
            "delta": "login password: my secret value"
        }
    }))
    .expect("multi-word password command output normalizes");
    assert_eq!(
        codex_event_summary(&multi_word_password),
        "Codex command output: login password:[redacted]"
    );

    let bare_multi_word_password = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-bare-password-phrase",
            "delta": "login password my secret value"
        }
    }))
    .expect("bare multi-word password command output normalizes");
    assert_eq!(
        codex_event_summary(&bare_multi_word_password),
        "Codex command output: login password [redacted]"
    );

    let spaced_token_assignment = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-spaced-token-assignment",
            "delta": "request token = abc123"
        }
    }))
    .expect("spaced token assignment command output normalizes");
    assert_eq!(
        codex_event_summary(&spaced_token_assignment),
        "Codex command output: request token [redacted]"
    );

    let long_output = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/commandExecution/outputDelta",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "cmd-2",
            "delta": "x".repeat(220)
        }
    }))
    .expect("long command output normalizes");
    let summary = codex_event_summary(&long_output);
    assert!(summary.starts_with("Codex command output: "));
    assert!(summary.ends_with("..."));
    assert!(summary.len() < 210);

    let diff = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "turn/diff/updated",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "files": ["src/main.rs", "README.md"]
        }
    }))
    .expect("diff update normalizes");
    assert_eq!(codex_event_summary(&diff), "Codex diff updated: 2 file(s)");

    let usage = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thread-1",
            "usage": {
                "input_tokens": 12,
                "output_tokens": 8,
                "cache_read_tokens": 4
            }
        }
    }))
    .expect("token usage normalizes");
    assert_eq!(
        codex_event_summary(&usage),
        "Codex token usage: 12 input, 8 output, 4 cache"
    );

    let unknown = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "future/event",
        "params": { "threadId": "thread-2" }
    }))
    .expect("unknown normalizes");
    assert_eq!(codex_event_summary(&unknown), "Codex event: future/event");
}

#[test]
fn codex_event_summaries_cover_lifecycle_and_item_branches() {
    let thread_started = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/started",
        "params": { "threadId": "thread-1" }
    }))
    .expect("thread start normalizes");
    assert_eq!(
        codex_event_summary(&thread_started),
        "Codex thread started thread-1"
    );

    let turn_started = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "turn/started",
        "params": { "turnId": "turn-1" }
    }))
    .expect("turn start normalizes");
    assert_eq!(
        codex_event_summary(&turn_started),
        "Codex turn started turn-1"
    );

    let turn_completed = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "turn/completed",
        "params": { "turnId": "turn-1" }
    }))
    .expect("turn completed normalizes");
    assert_eq!(
        codex_event_summary(&turn_completed),
        "Codex turn completed turn-1"
    );

    let turn_cancelled = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "turn/cancelled",
        "params": { "turnId": "turn-1" }
    }))
    .expect("turn cancelled normalizes");
    assert_eq!(
        codex_event_summary(&turn_cancelled),
        "Codex turn cancelled turn-1"
    );

    let status = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/status/changed",
        "params": { "status": "running" }
    }))
    .expect("thread status normalizes");
    assert_eq!(codex_event_summary(&status), "Codex thread status: running");

    let item_started = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/started",
        "params": { "itemId": "item-1", "type": "commandExecution" }
    }))
    .expect("item start normalizes");
    assert_eq!(
        codex_event_summary(&item_started),
        "Codex item started: commandExecution"
    );

    let item_completed = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/completed",
        "params": { "itemId": "item-1", "type": "commandExecution" }
    }))
    .expect("item completion normalizes");
    assert_eq!(
        codex_event_summary(&item_completed),
        "Codex item completed: commandExecution"
    );

    let plan = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/plan/delta",
        "params": { "delta": "1. inspect\n2. patch" }
    }))
    .expect("plan delta normalizes");
    assert_eq!(
        codex_event_summary(&plan),
        "Codex plan: 1. inspect 2. patch"
    );

    let approval_requested = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "item/permissions/requestApproval",
        "params": { "itemId": "approval-1" }
    }))
    .expect("approval request normalizes");
    assert_eq!(
        codex_event_summary(&approval_requested),
        "Codex requested approval approval-1"
    );

    let approval_completed = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": { "itemId": "approval-1", "decision": "approve" }
    }))
    .expect("approval completion normalizes");
    assert_eq!(
        codex_event_summary(&approval_completed),
        "Codex approval completed approval-1"
    );

    let error = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "error",
        "params": { "message": "auth failed for token=abc123" }
    }))
    .expect("error normalizes");
    assert_eq!(
        codex_event_summary(&error),
        "Codex app-server error: auth failed for token=[redacted]"
    );
}

#[test]
fn codex_adapter_exposes_supported_local_harness_capabilities() {
    let adapter = CodexAppServerAdapter::local_stdio("codex-test", "opensymphony-test", "1.10.1");
    assert_eq!(adapter.harness_kind(), "codex_app_server");
    assert_eq!(
        adapter.launch().command_args(),
        vec!["--dangerously-bypass-hook-trust", "app-server", "--stdio"]
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
    assert!(!capabilities.actions.approve);
    assert!(!capabilities.actions.reject);
    assert!(!capabilities.actions.comment);
    assert!(capabilities.approvals.tool_approval);
    assert!(!capabilities.history.fetch_history);
    assert!(!capabilities.history.reconcile_after_ready);
    assert!(!capabilities.history.reconnect_and_replay);
    assert!(capabilities.history.preserve_unknown_events);
    assert!(!capabilities.transport.remote);
    assert!(
        capabilities
            .feature_gaps
            .iter()
            .any(|gap| gap.contains("history fetch"))
    );
}

#[test]
fn codex_lifecycle_requests_cover_start_resume_cancel_and_approval() {
    let adapter = CodexAppServerAdapter::local_stdio("codex-test", "opensymphony-test", "1.10.1");
    let mut session = adapter.session();

    let thread_start = adapter
        .start_issue_thread_request(
            &mut session,
            "/tmp/issue-workspace",
            Some("gpt-5-codex".into()),
            json!({ "opensymphonyRoute": { "harness": "codex_app_server" } }),
        )
        .expect("thread/start request serializes");
    assert_eq!(thread_start.lifecycle, CodexLifecycleRequest::Start);
    assert_eq!(thread_start.request.method, "thread/start");
    assert_eq!(thread_start.request.params["approvalPolicy"], "never");
    assert_eq!(thread_start.request.params["cwd"], "/tmp/issue-workspace");
    assert!(
        thread_start
            .request
            .params
            .get("baseInstructions")
            .is_none()
    );
    assert_eq!(thread_start.request.params["model"], "gpt-5-codex");
    assert!(
        thread_start.request.params.get("modelProvider").is_none(),
        "selected Codex model should not force a provider"
    );
    assert_eq!(
        thread_start.request.params["sandbox"],
        json!("danger-full-access")
    );

    let turn_start = adapter
        .start_issue_turn_request(
            &mut session,
            "thread-1",
            "/tmp/issue-workspace",
            Some("gpt-5-codex".into()),
            "workflow prompt",
        )
        .expect("turn/start request serializes");
    assert_eq!(turn_start.lifecycle, CodexLifecycleRequest::Start);
    assert_eq!(turn_start.request.method, "turn/start");
    assert_eq!(turn_start.request.params["threadId"], "thread-1");
    assert_eq!(
        turn_start.request.params["input"][0]["text"],
        "workflow prompt"
    );
    assert_eq!(turn_start.request.params["approvalPolicy"], "never");
    assert_eq!(
        turn_start.request.params["sandboxPolicy"],
        json!({ "type": "dangerFullAccess" })
    );

    let start_with_codex_default = adapter
        .start_issue_thread_request(&mut session, "/tmp/issue-workspace", None, json!({}))
        .expect("start request without selected model serializes");
    assert!(
        start_with_codex_default
            .request
            .params
            .get("model")
            .is_none(),
        "omitting selected model should let Codex use its own configured default"
    );
    assert!(
        start_with_codex_default
            .request
            .params
            .get("modelProvider")
            .is_none(),
        "omitting selected model should not force Codex's configured provider"
    );

    let resume = adapter
        .resume_issue_request(&mut session, "thread-1", "/tmp/issue-workspace", "continue")
        .expect("resume request serializes");
    assert_eq!(resume.lifecycle, CodexLifecycleRequest::Resume);
    assert_eq!(resume.request.method, "turn/start");
    assert_eq!(resume.request.params["threadId"], "thread-1");
    assert_eq!(resume.request.params["approvalPolicy"], "never");
    assert_eq!(
        resume.request.params["sandboxPolicy"],
        json!({ "type": "dangerFullAccess" })
    );

    let cancel = adapter.cancel_turn_request(&mut session, "turn-1");
    assert_eq!(cancel.lifecycle, CodexLifecycleRequest::Cancel);
    assert_eq!(cancel.request.method, "turn/cancel");
    assert_eq!(cancel.request.params["turnId"], "turn-1");

    let approve = adapter.approval_response(
        &mut session,
        "approval-1",
        CodexApprovalDecision::Approve,
        Some("operator approved".into()),
    );
    assert_eq!(approve.lifecycle, CodexLifecycleRequest::Approval);
    assert_eq!(approve.request.method, "approval/respond");
    assert_eq!(approve.request.params["decision"], "approve");

    let reject = adapter.approval_response(
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
    assert_eq!(
        completed_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "turn/completed".into()
        }
    );

    let terminal = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/status/changed",
        "params": {
            "threadId": "thread-1",
            "status": "completed"
        }
    }))
    .expect("terminal thread status normalizes");
    let terminal_record = normalized_event_to_journal_record("COE-476", 14, &terminal);
    assert_eq!(terminal_record.kind, EventKind::RunCompleted);

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

    let error_field_only = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "error",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "error": "future nested diagnostic"
        }
    }))
    .expect("error notification normalizes");
    let error_field_record = normalized_event_to_journal_record("COE-476", 19, &error_field_only);
    assert_eq!(error_field_record.kind, EventKind::RunFailed);
    assert_eq!(
        error_field_record.summary,
        "Codex app-server reported an error"
    );

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
    assert_eq!(
        canceled_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "turn/cancelled".into()
        }
    );
}

#[test]
fn codex_approval_notification_maps_to_approval_center_contract() {
    let raw = json!({
        "jsonrpc": "2.0",
        "method": "item/permissions/requestApproval",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-1",
            "title": "Run shell command",
            "description": "Codex wants to inspect the repo",
            "command": "rg approval crates"
        }
    });
    let event = normalize_server_notification(raw).expect("approval notification normalizes");
    let requested_at = Utc
        .timestamp_millis_opt(1_720_000_000_000)
        .single()
        .expect("timestamp");

    let approval =
        codex_approval_request_from_event("run-1", "lin-429", "COE-429", requested_at, &event)
            .expect("approval request should map");

    assert_eq!(approval.approval_id, "approval-1");
    assert_eq!(approval.run_id, "run-1");
    assert_eq!(approval.issue_id, "lin-429");
    assert_eq!(approval.kind, ApprovalKind::CommandExecution);
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(approval.correlation_id, "thread-1:turn-1:approval-1");
    assert_eq!(
        approval.actor.as_ref().expect("actor").actor_id,
        "codex_app_server"
    );
    assert_eq!(
        approval
            .target_context
            .as_ref()
            .expect("target context")
            .command
            .as_deref(),
        Some("rg approval crates")
    );
    assert_eq!(
        approval.risk_summary.as_ref().expect("risk").level,
        ApprovalRiskLevel::Medium
    );
    assert!(approval.proposed_action.is_some());
}

#[test]
fn codex_file_write_approval_has_medium_risk() {
    let raw = json!({
        "jsonrpc": "2.0",
        "method": "item/permissions/requestApproval",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-2",
            "title": "Write file",
            "description": "Codex wants to edit source",
            "filePath": "src/lib.rs"
        }
    });
    let event = normalize_server_notification(raw).expect("approval notification normalizes");
    let requested_at = Utc
        .timestamp_millis_opt(1_720_000_000_000)
        .single()
        .expect("timestamp");

    let approval =
        codex_approval_request_from_event("run-1", "lin-429", "COE-429", requested_at, &event)
            .expect("approval request should map");

    assert_eq!(approval.kind, ApprovalKind::FileWrite);
    assert_eq!(
        approval.risk_summary.as_ref().expect("risk").level,
        ApprovalRiskLevel::Medium
    );
    assert_eq!(
        approval
            .target_context
            .as_ref()
            .expect("target context")
            .file_path
            .as_deref(),
        Some("src/lib.rs")
    );
}

#[test]
fn codex_approval_decision_request_and_audit_record_stay_correlated() {
    let adapter = CodexAppServerAdapter::local_stdio("codex-test", "opensymphony-test", "1.10.1");
    let mut session = adapter.session();

    let response = adapter.approval_response(
        &mut session,
        "approval-1",
        CodexApprovalDecision::Approve,
        Some("operator accepted command".into()),
    );
    let audit = codex_approval_decision_audit_record(
        "run-1",
        42,
        "approval-1",
        CodexApprovalDecision::Approve,
        Some("operator accepted command".into()),
    );

    assert_eq!(response.request.method, "approval/respond");
    assert_eq!(response.request.params["approvalId"], "approval-1");
    assert_eq!(response.request.params["decision"], "approve");
    assert_eq!(audit.kind, EventKind::ApprovalGranted);
    assert_eq!(audit.actor.actor_id(), "opensymphony_approval_bridge");
    assert_eq!(
        audit.raw_payload_ref.as_deref(),
        Some("codex:run-1:approval-decision:42")
    );
}

#[test]
fn codex_approval_completed_maps_decisions_without_guessing() {
    let approved = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-1",
            "decision": "approve"
        }
    }))
    .expect("approved completion normalizes");
    let approved_record = normalized_event_to_journal_record("COE-476", 11, &approved);
    assert_eq!(approved_record.kind, EventKind::ApprovalGranted);

    let rejected = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-2",
            "decision": "reject"
        }
    }))
    .expect("rejected completion normalizes");
    let rejected_record = normalized_event_to_journal_record("COE-476", 12, &rejected);
    assert_eq!(rejected_record.kind, EventKind::ApprovalDenied);

    let unknown = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-3"
        }
    }))
    .expect("unknown completion normalizes");
    let unknown_record = normalized_event_to_journal_record("COE-476", 13, &unknown);
    assert_eq!(
        unknown_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "approval/completed".into()
        }
    );

    let cancelled = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-4",
            "decision": "cancelled"
        }
    }))
    .expect("cancelled completion normalizes");
    let cancelled_record = normalized_event_to_journal_record("COE-476", 15, &cancelled);
    assert_eq!(
        cancelled_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "approval/completed".into()
        }
    );

    let alias = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-5",
            "decision": "approved"
        }
    }))
    .expect("alias completion normalizes");
    let alias_record = normalized_event_to_journal_record("COE-476", 16, &alias);
    assert_eq!(
        alias_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "approval/completed".into()
        }
    );

    let result_field_only = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/completed",
        "params": {
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "approval-6",
            "result": "approve"
        }
    }))
    .expect("completion with non-contract result field normalizes");
    let result_field_record = normalized_event_to_journal_record("COE-476", 17, &result_field_only);
    assert_eq!(
        result_field_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "approval/completed".into()
        }
    );

    let alternate_method = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "approval/responded",
        "params": {
            "decision": "approve"
        }
    }))
    .expect("unknown alternate method is retained");
    assert_eq!(alternate_method.kind, NormalizedCodexEventKind::Unknown);
}

#[test]
fn codex_thread_status_changed_uses_only_status_field() {
    let completed = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/status/changed",
        "params": {
            "threadId": "thread-1",
            "status": "completed"
        }
    }))
    .expect("thread status change normalizes");
    let completed_record = normalized_event_to_journal_record("COE-476", 18, &completed);
    assert_eq!(completed_record.kind, EventKind::RunCompleted);

    let state_field_only = normalize_server_notification(json!({
        "jsonrpc": "2.0",
        "method": "thread/status/changed",
        "params": {
            "threadId": "thread-1",
            "state": "completed"
        }
    }))
    .expect("thread status with non-contract state field normalizes");
    let state_field_record = normalized_event_to_journal_record("COE-476", 20, &state_field_only);
    assert_eq!(
        state_field_record.kind,
        EventKind::HarnessEventNormalized {
            source_kind: "thread/status/changed".into()
        }
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
            && profile.model_reference == "gpt-5.5"
            && profile.config_overrides.get("model").map(String::as_str) == Some("gpt-5.5")
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
        vec!["--dangerously-bypass-hook-trust", "app-server", "--stdio"]
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
