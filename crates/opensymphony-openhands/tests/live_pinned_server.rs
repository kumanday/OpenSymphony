use std::{
    env,
    net::TcpListener,
    path::Path,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use opensymphony_openhands::{
    AgentConfig, ApiKeyAuth, AuthConfig, CondenserConfig, ConfirmationPolicy, Conversation,
    ConversationCreateRequest, HttpAuth, LlmConfig, OpenHandsClient, OpenHandsError,
    RuntimeStreamConfig, SendMessageRequest, TransportConfig, WebSocketAuth, WorkspaceConfig,
};
use tempfile::TempDir;
use tokio::time::{Instant, sleep};
use uuid::Uuid;

const LIVE_GATE_ENV: &str = "OPENSYMPHONY_LIVE_OPENHANDS";
const SESSION_API_KEY: &str = "live-secret-token";
const SESSION_API_KEY_HEADER: &str = "x-session-api-key";
const SESSION_API_KEY_QUERY_PARAM: &str = "session_api_key";
const LONG_HISTORY_BATCHES: usize = 6;
const LONG_HISTORY_BATCH_SIZE: usize = 50;
const LONG_HISTORY_MAX_SIZE: u64 = 40;
const LONG_HISTORY_KEEP_FIRST: u64 = 2;

#[tokio::test]
async fn live_pinned_server_authenticates_external_http_and_websocket_paths() {
    if env::var(LIVE_GATE_ENV).as_deref() != Ok("1") {
        eprintln!("skipping live pinned-server test; set {LIVE_GATE_ENV}=1 to enable it");
        return;
    }

    let server = PinnedServer::start(SESSION_API_KEY).await;
    let workspace = TempDir::new().expect("workspace temp dir should be created");
    let request = doctor_probe_request(workspace.path());

    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()).with_auth(
        AuthConfig::header_api_key_with_websocket_query_fallback(
            SESSION_API_KEY_HEADER,
            SESSION_API_KEY_QUERY_PARAM,
            SESSION_API_KEY,
        ),
    ));

    let conversation = client
        .create_conversation(&request)
        .await
        .expect("create conversation should authenticate");
    let attached = client
        .get_conversation(conversation.conversation_id)
        .await
        .expect("get conversation should authenticate");
    assert_eq!(attached.conversation_id, conversation.conversation_id);

    let stream = client
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(5),
                reconnect_initial_backoff: Duration::from_millis(100),
                reconnect_max_backoff: Duration::from_millis(250),
                max_reconnect_attempts: 1,
            },
        )
        .await
        .expect("websocket readiness should authenticate");

    assert_eq!(
        stream.conversation().conversation_id,
        conversation.conversation_id
    );
    assert_eq!(
        stream.state_mirror().execution_status(),
        Some("idle"),
        "new conversations should attach in the idle state"
    );
}

#[tokio::test]
async fn live_pinned_server_rejects_missing_http_auth_and_wrong_websocket_auth() {
    if env::var(LIVE_GATE_ENV).as_deref() != Ok("1") {
        eprintln!("skipping live pinned-server test; set {LIVE_GATE_ENV}=1 to enable it");
        return;
    }

    let server = PinnedServer::start(SESSION_API_KEY).await;
    let workspace = TempDir::new().expect("workspace temp dir should be created");
    let request = doctor_probe_request(workspace.path());

    let authenticated = OpenHandsClient::new(TransportConfig::new(server.base_url()).with_auth(
        AuthConfig::header_api_key_with_websocket_query_fallback(
            SESSION_API_KEY_HEADER,
            SESSION_API_KEY_QUERY_PARAM,
            SESSION_API_KEY,
        ),
    ));
    let conversation = authenticated
        .create_conversation(&request)
        .await
        .expect("setup conversation should authenticate");

    let missing_http_auth = OpenHandsClient::new(TransportConfig::new(server.base_url()));
    let error = missing_http_auth
        .create_conversation(&request)
        .await
        .expect_err("missing HTTP auth should be rejected");
    assert!(
        matches!(
            error,
            OpenHandsError::HttpStatus {
                status_code: 401,
                ..
            }
        ),
        "expected stable HTTP 401 mapping, got {error:?}"
    );

    let wrong_websocket_auth = OpenHandsClient::new(
        TransportConfig::new(server.base_url()).with_auth(AuthConfig {
            http: HttpAuth::Header(ApiKeyAuth::new(SESSION_API_KEY_HEADER, SESSION_API_KEY)),
            websocket: WebSocketAuth::QueryParam(ApiKeyAuth::new(
                SESSION_API_KEY_QUERY_PARAM,
                "wrong-secret-token",
            )),
        }),
    );
    let error = match wrong_websocket_auth
        .attach_runtime_stream(
            conversation.conversation_id,
            RuntimeStreamConfig {
                readiness_timeout: Duration::from_secs(5),
                reconnect_initial_backoff: Duration::from_millis(100),
                reconnect_max_backoff: Duration::from_millis(250),
                max_reconnect_attempts: 1,
            },
        )
        .await
    {
        Ok(_) => panic!("wrong websocket auth should fail the attach handshake"),
        Err(error) => error,
    };
    assert!(
        matches!(error, OpenHandsError::WebSocketTransport { .. }),
        "expected stable websocket transport mapping, got {error:?}"
    );
}

#[tokio::test]
#[ignore = "requires a prepared local machine with live OpenHands credentials"]
async fn live_pinned_server_condenses_long_history_without_prompt_overflow() {
    if env::var(LIVE_GATE_ENV).as_deref() != Ok("1") {
        eprintln!("skipping live pinned-server test; set {LIVE_GATE_ENV}=1 to enable it");
        return;
    }

    let model = env::var("OPENSYMPHONY_OPENHANDS_MODEL")
        .or_else(|_| env::var("LLM_MODEL"))
        .expect("live condenser test requires OPENSYMPHONY_OPENHANDS_MODEL or LLM_MODEL");
    let server = PinnedServer::start(SESSION_API_KEY).await;
    let workspace = TempDir::new().expect("workspace temp dir should be created");
    let request = long_history_request(workspace.path(), &model);

    let client = OpenHandsClient::new(TransportConfig::new(server.base_url()).with_auth(
        AuthConfig::header_api_key_with_websocket_query_fallback(
            SESSION_API_KEY_HEADER,
            SESSION_API_KEY_QUERY_PARAM,
            SESSION_API_KEY,
        ),
    ));

    let conversation = client
        .create_conversation(&request)
        .await
        .expect("create conversation should authenticate");

    for batch in 0..LONG_HISTORY_BATCHES {
        for turn in 0..LONG_HISTORY_BATCH_SIZE {
            let ordinal = batch * LONG_HISTORY_BATCH_SIZE + turn + 1;
            let prompt = if turn + 1 == LONG_HISTORY_BATCH_SIZE {
                format!(
                    "History marker {ordinal}. Reply with the exact text `batch-{batch}-ok` and then finish."
                )
            } else {
                format!(
                    "History marker {ordinal}. Do not answer yet; this is backlog context only."
                )
            };

            client
                .send_message(
                    conversation.conversation_id,
                    &SendMessageRequest::user_text(prompt),
                )
                .await
                .expect("sending backlog messages should succeed");
        }

        client
            .run_conversation(conversation.conversation_id)
            .await
            .expect("conversation run should start");

        wait_for_terminal_status(
            &client,
            conversation.conversation_id,
            Duration::from_secs(120),
        )
        .await
        .expect("conversation run should finish cleanly");
    }

    let events = client
        .search_all_events(conversation.conversation_id)
        .await
        .expect("events should remain searchable after long history");
    let condensation_events = events
        .items()
        .iter()
        .filter(|event| event.kind.contains("Condensation"))
        .count();
    let overflow_errors = events
        .items()
        .iter()
        .filter(|event| event.kind == "ConversationErrorEvent")
        .filter(|event| event.payload.to_string().contains("prompt is too long"))
        .count();
    let message_events = events
        .items()
        .iter()
        .filter(|event| event.kind == "MessageEvent")
        .count();

    assert!(
        message_events >= LONG_HISTORY_BATCHES * LONG_HISTORY_BATCH_SIZE,
        "expected at least {} message events, got {message_events}",
        LONG_HISTORY_BATCHES * LONG_HISTORY_BATCH_SIZE
    );
    assert!(
        condensation_events > 0,
        "expected at least one condensation event after long history, got none"
    );
    assert_eq!(
        overflow_errors, 0,
        "expected no prompt-overflow ConversationErrorEvent, found {overflow_errors}"
    );
}

struct PinnedServer {
    child: Child,
    base_url: String,
}

impl PinnedServer {
    async fn start(session_api_key: &str) -> Self {
        let port = free_port();
        let base_url = format!("http://127.0.0.1:{port}");
        let repo_root = workspace_root();
        let mut child = Command::new(repo_root.join("tools/openhands-server/run-local.sh"))
            .current_dir(&repo_root)
            .env("OPENHANDS_SERVER_PORT", port.to_string())
            .env("SESSION_API_KEY", session_api_key)
            .env("OPENHANDS_SUPPRESS_BANNER", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("pinned OpenHands server should start");

        wait_for_ready(&base_url, session_api_key, &mut child).await;

        Self { child, base_url }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for PinnedServer {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir should have workspace parent")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn doctor_probe_request(workspace_root: &std::path::Path) -> ConversationCreateRequest {
    let working_dir = workspace_root.join("repo");
    let persistence_dir = working_dir.join(".opensymphony/openhands");
    std::fs::create_dir_all(&persistence_dir).expect("probe directories should be created");
    ConversationCreateRequest::doctor_probe(
        working_dir.display().to_string(),
        persistence_dir.display().to_string(),
        Some("gpt-5.4-mini".to_string()),
        None,
    )
}

fn long_history_request(workspace_root: &Path, model: &str) -> ConversationCreateRequest {
    let working_dir = workspace_root.join("repo");
    let persistence_dir = working_dir.join(".opensymphony/openhands");
    std::fs::create_dir_all(&persistence_dir).expect("probe directories should be created");

    let llm = LlmConfig {
        model: model.to_string(),
        api_key: env::var("LLM_API_KEY").ok(),
        base_url: env::var("LLM_BASE_URL").ok(),
        usage_id: None,
    };

    ConversationCreateRequest {
        conversation_id: Uuid::new_v4(),
        workspace: WorkspaceConfig {
            working_dir: working_dir.display().to_string(),
            kind: "LocalWorkspace".to_string(),
        },
        persistence_dir: persistence_dir.display().to_string(),
        max_iterations: 20,
        stuck_detection: true,
        confirmation_policy: ConfirmationPolicy {
            kind: "NeverConfirm".to_string(),
        },
        agent: AgentConfig {
            kind: "Agent".to_string(),
            llm: llm.clone(),
            condenser: Some(CondenserConfig::llm_summarizing(
                llm,
                LONG_HISTORY_MAX_SIZE,
                LONG_HISTORY_KEEP_FIRST,
            )),
            tools: None,
            include_default_tools: None,
        },
    }
}

async fn wait_for_terminal_status(
    client: &OpenHandsClient,
    conversation_id: Uuid,
    wait_timeout: Duration,
) -> Result<Conversation, String> {
    let deadline = Instant::now() + wait_timeout;

    loop {
        let conversation = client
            .get_conversation(conversation_id)
            .await
            .map_err(|error| format!("failed to poll conversation state: {error}"))?;
        match conversation.execution_status.as_str() {
            "finished" => return Ok(conversation),
            "error" | "stuck" => {
                return Err(format!(
                    "terminal execution_status `{}`",
                    conversation.execution_status
                ));
            }
            _ => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "timed out waiting {wait_timeout:?} for terminal status"
                    ));
                }
                sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

async fn wait_for_ready(base_url: &str, session_api_key: &str, child: &mut Child) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(250))
        .build()
        .expect("probe client should build");

    for _ in 0..120 {
        if let Some(status) = child.try_wait().expect("child status should be readable") {
            panic!("pinned OpenHands server exited before readiness: status={status:?}");
        }

        match client
            .get(format!("{base_url}/openapi.json"))
            .header(SESSION_API_KEY_HEADER, session_api_key)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return,
            Ok(_) | Err(_) => sleep(Duration::from_millis(100)).await,
        }
    }

    panic!("pinned OpenHands server did not become ready at {base_url}");
}
