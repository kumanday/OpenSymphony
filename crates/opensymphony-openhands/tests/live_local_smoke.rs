//! Opt-in live smoke test for the pinned local OpenHands server environment.

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Map;
use tempfile::TempDir;
use url::Url;

use opensymphony_domain::{ExecutionStatus, IssueRef, PromptSet};
use opensymphony_openhands::{
    AgentConfig, ConfirmationPolicy, ConversationConfig, HttpAuth, IssueSessionRequest,
    IssueSessionRunner, LlmConfig, LocalAgentServerSupervisor, LocalServerConfig, OpenHandsClient,
    ToolConfig, TransportConfig, WebSocketAuthMode, WebSocketConfig,
};
use opensymphony_workspace::WorkspaceLayout;

fn env_required(name: &str) -> String {
    env::var(name)
        .unwrap_or_else(|_| panic!("{name} must be set when OPENSYMPHONY_LIVE_OPENHANDS=1"))
}

fn free_loopback_port() -> u16 {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .expect("loopback port allocation should succeed");
    let port = listener
        .local_addr()
        .expect("loopback listener should have a local address")
        .port();
    drop(listener);
    port
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("crate path should resolve to repository root")
}

fn pinned_tools_dir() -> PathBuf {
    repo_root().join("tools/openhands-server")
}

fn live_agent(model: String, api_key: String, base_url: Option<String>) -> AgentConfig {
    AgentConfig {
        kind: Some("Agent".to_string()),
        llm: LlmConfig {
            model,
            api_key: Some(api_key),
            base_url,
            api_version: None,
            usage_id: Some("opensymphony-live-smoke".to_string()),
            max_output_tokens: Some(512),
            log_completions: false,
            log_completions_folder: None,
            extra: Map::new(),
        },
        tools: vec![
            ToolConfig {
                name: "TerminalTool".to_string(),
                params: Map::new(),
            },
            ToolConfig {
                name: "FileEditorTool".to_string(),
                params: Map::new(),
            },
        ],
        include_default_tools: vec!["FinishTool".to_string(), "ThinkTool".to_string()],
        filter_tools_regex: None,
        mcp_config: None,
        extra: Map::new(),
    }
}

#[tokio::test]
async fn live_local_smoke() -> Result<(), Box<dyn std::error::Error>> {
    if env::var("OPENSYMPHONY_LIVE_OPENHANDS").ok().as_deref() != Some("1") {
        eprintln!("skipping live_local_smoke; set OPENSYMPHONY_LIVE_OPENHANDS=1 to enable");
        return Ok(());
    }

    let model = env_required("OPENHANDS_MODEL");
    let api_key = env_required("OPENHANDS_LLM_API_KEY");
    let llm_base_url = env::var("OPENHANDS_LLM_BASE_URL").ok();
    let port = free_loopback_port();
    let transport = TransportConfig {
        base_url: Url::parse(&format!("http://127.0.0.1:{port}"))?,
        http_auth: HttpAuth::None,
        websocket_auth: WebSocketAuthMode::Auto,
        websocket_query_param_name: "session_api_key".to_string(),
    };
    let mut supervisor = LocalAgentServerSupervisor::new(
        transport.clone(),
        LocalServerConfig {
            command: vec![
                "uv".to_string(),
                "run".to_string(),
                "--project".to_string(),
                pinned_tools_dir().display().to_string(),
                "agent-server".to_string(),
                "--host".to_string(),
                "127.0.0.1".to_string(),
                "--port".to_string(),
                port.to_string(),
            ],
            workdir: Some(repo_root()),
            env: BTreeMap::from([
                (
                    "OH_SECRET_KEY".to_string(),
                    "opensymphony-live-local-smoke".to_string(),
                ),
                ("PYTHONUNBUFFERED".to_string(), "1".to_string()),
                ("RUNTIME".to_string(), "process".to_string()),
            ]),
            startup_timeout_ms: 60_000,
            readiness_probe_path: "/ready".to_string(),
        },
    )?;

    let status = supervisor.start().await?;
    assert!(status.ready, "local OpenHands server did not become ready");

    let temp_dir = TempDir::new()?;
    let workspace = WorkspaceLayout::new(temp_dir.path(), "LIVE-OPENHANDS-1")?;
    let runner = IssueSessionRunner::new(
        OpenHandsClient::new(transport.clone()),
        ConversationConfig {
            runtime_contract_version: "openhands-sdk-v1.14.0".to_string(),
            persistence_dir_relative: ".opensymphony/openhands".to_string(),
            agent: live_agent(model, api_key, llm_base_url),
            confirmation_policy: ConfirmationPolicy::never_confirm(),
            max_iterations: 12,
            stuck_detection: true,
            autotitle: false,
            hook_config: None,
            plugins: Vec::new(),
            secrets: BTreeMap::new(),
        },
        WebSocketConfig {
            ready_timeout_ms: 60_000,
            reconnect_initial_ms: 1_000,
            reconnect_max_ms: 10_000,
            poll_interval_ms: 1_000,
        },
    )
    .with_run_timeout(Duration::from_secs(300));

    let outcome = runner
        .execute(&IssueSessionRequest {
            issue: IssueRef {
                issue_id: "live-issue-1".to_string(),
                identifier: "LIVE-OPENHANDS-1".to_string(),
                title: "Live OpenHands smoke".to_string(),
            },
            workspace: workspace.clone(),
            prompts: PromptSet {
                full_prompt: "Create a file named `smoke.txt` in the current working directory with the exact contents `live-smoke-ok`, then finish.".to_string(),
                continuation_prompt: "Resume the existing task. If `smoke.txt` does not contain `live-smoke-ok`, fix it, then finish.".to_string(),
            },
        })
        .await
        ?;

    assert_eq!(outcome.execution_status, ExecutionStatus::Success);
    let file = workspace.issue_workspace.join("smoke.txt");
    assert_eq!(std::fs::read_to_string(file)?.trim(), "live-smoke-ok");

    supervisor.stop().await?;
    Ok(())
}
