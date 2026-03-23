use std::{
    env,
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};

use opensymphony_openhands::{
    ApiKeyAuth, AuthConfig, ConversationCreateRequest, HttpAuth, OpenHandsClient, OpenHandsError,
    RuntimeStreamConfig, TransportConfig, WebSocketAuth,
};
use tempfile::TempDir;
use tokio::time::sleep;

const LIVE_GATE_ENV: &str = "OPENSYMPHONY_LIVE_OPENHANDS";
const SESSION_API_KEY: &str = "live-secret-token";
const SESSION_API_KEY_HEADER: &str = "x-session-api-key";
const SESSION_API_KEY_QUERY_PARAM: &str = "session_api_key";

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
        Some("gpt-4.1-mini".to_string()),
        None,
    )
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
