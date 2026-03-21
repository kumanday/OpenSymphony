use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PROMPT_TEMPLATE: &str = "You are working on an issue from Linear.";
pub const DEFAULT_LINEAR_ENDPOINT: &str = "https://api.linear.app/graphql";
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 30_000;
pub const DEFAULT_WORKSPACE_ROOT: &str = "/symphony_workspaces";
pub const DEFAULT_HOOK_TIMEOUT_MS: u64 = 60_000;
pub const DEFAULT_MAX_CONCURRENT_AGENTS: u64 = 10;
pub const DEFAULT_MAX_TURNS: u64 = 20;
pub const DEFAULT_MAX_RETRY_BACKOFF_MS: u64 = 300_000;
pub const DEFAULT_STALL_TIMEOUT_MS: u64 = 300_000;
pub const DEFAULT_OPENHANDS_BASE_URL: &str = "http://127.0.0.1:8000";
pub const DEFAULT_OPENHANDS_STARTUP_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_OPENHANDS_READINESS_PROBE_PATH: &str = "/openapi.json";
pub const DEFAULT_OPENHANDS_PERSISTENCE_DIR: &str = ".opensymphony/openhands";
pub const DEFAULT_OPENHANDS_MAX_ITERATIONS: u64 = 500;
pub const DEFAULT_OPENHANDS_READY_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS: u64 = 1_000;
pub const DEFAULT_OPENHANDS_RECONNECT_MAX_MS: u64 = 30_000;
pub const DEFAULT_OPENHANDS_AUTH_MODE: &str = "auto";
pub const DEFAULT_OPENHANDS_QUERY_PARAM_NAME: &str = "session_api_key";

pub fn default_active_states() -> Vec<String> {
    vec!["Todo".to_owned(), "In Progress".to_owned()]
}

pub fn default_terminal_states() -> Vec<String> {
    vec![
        "Closed".to_owned(),
        "Cancelled".to_owned(),
        "Canceled".to_owned(),
        "Duplicate".to_owned(),
        "Done".to_owned(),
    ]
}

pub fn default_openhands_local_server_command() -> Vec<String> {
    vec![
        "python".to_owned(),
        "-m".to_owned(),
        "openhands.agent_server".to_owned(),
        "--host".to_owned(),
        "127.0.0.1".to_owned(),
        "--port".to_owned(),
        "8000".to_owned(),
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowDefinition {
    pub front_matter: WorkflowFrontMatter,
    pub prompt_template: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct WorkflowFrontMatter {
    #[serde(default)]
    pub tracker: TrackerFrontMatter,
    #[serde(default)]
    pub polling: PollingFrontMatter,
    #[serde(default)]
    pub workspace: WorkspaceFrontMatter,
    #[serde(default)]
    pub hooks: HooksFrontMatter,
    #[serde(default)]
    pub agent: AgentFrontMatter,
    #[serde(default)]
    pub openhands: OpenHandsFrontMatter,
    #[serde(flatten)]
    pub extensions: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrackerFrontMatter {
    pub kind: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub project_slug: Option<String>,
    pub active_states: Option<Vec<String>>,
    pub terminal_states: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PollingFrontMatter {
    pub interval_ms: Option<IntegerLike>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFrontMatter {
    pub root: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HooksFrontMatter {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    pub timeout_ms: Option<IntegerLike>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentFrontMatter {
    pub max_concurrent_agents: Option<IntegerLike>,
    pub max_turns: Option<IntegerLike>,
    pub max_retry_backoff_ms: Option<IntegerLike>,
    pub stall_timeout_ms: Option<IntegerLike>,
    pub max_concurrent_agents_by_state: Option<BTreeMap<String, IntegerLike>>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsFrontMatter {
    #[serde(default)]
    pub transport: OpenHandsTransportFrontMatter,
    #[serde(default)]
    pub local_server: OpenHandsLocalServerFrontMatter,
    #[serde(default)]
    pub conversation: OpenHandsConversationFrontMatter,
    #[serde(default)]
    pub websocket: OpenHandsWebSocketFrontMatter,
    #[serde(default)]
    pub mcp: OpenHandsMcpFrontMatter,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsTransportFrontMatter {
    pub base_url: Option<String>,
    pub session_api_key_env: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsLocalServerFrontMatter {
    pub enabled: Option<bool>,
    pub command: Option<Vec<String>>,
    pub startup_timeout_ms: Option<IntegerLike>,
    pub readiness_probe_path: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsConversationFrontMatter {
    pub reuse_policy: Option<String>,
    pub persistence_dir_relative: Option<String>,
    pub max_iterations: Option<IntegerLike>,
    pub stuck_detection: Option<bool>,
    pub confirmation_policy: Option<OpenHandsConfirmationPolicy>,
    pub agent: Option<OpenHandsConversationAgentFrontMatter>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OpenHandsConfirmationPolicy {
    pub kind: String,
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct OpenHandsConversationAgentFrontMatter {
    pub kind: Option<String>,
    pub llm: Option<OpenHandsLlmFrontMatter>,
    pub log_completions: Option<bool>,
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct OpenHandsLlmFrontMatter {
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url_env: Option<String>,
    #[serde(flatten)]
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsWebSocketFrontMatter {
    pub enabled: Option<bool>,
    pub ready_timeout_ms: Option<IntegerLike>,
    pub reconnect_initial_ms: Option<IntegerLike>,
    pub reconnect_max_ms: Option<IntegerLike>,
    pub auth_mode: Option<String>,
    pub query_param_name: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsMcpFrontMatter {
    pub stdio_servers: Option<Vec<OpenHandsStdioServerFrontMatter>>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsStdioServerFrontMatter {
    pub name: String,
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum IntegerLike {
    Integer(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedWorkflow {
    pub config: WorkflowConfig,
    pub extensions: WorkflowExtensions,
    pub prompt_template: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowConfig {
    pub tracker: TrackerConfig,
    pub polling: PollingConfig,
    pub workspace: WorkspaceConfig,
    pub hooks: HooksConfig,
    pub agent: AgentConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowExtensions {
    pub openhands: OpenHandsConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerKind {
    Linear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerConfig {
    pub kind: TrackerKind,
    pub endpoint: String,
    pub api_key: String,
    pub project_slug: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollingConfig {
    pub interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    pub max_concurrent_agents: u64,
    pub max_turns: u64,
    pub max_retry_backoff_ms: u64,
    pub stall_timeout_ms: Option<u64>,
    pub max_concurrent_agents_by_state: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsConfig {
    pub transport: OpenHandsTransportConfig,
    pub local_server: OpenHandsLocalServerConfig,
    pub conversation: OpenHandsConversationConfig,
    pub websocket: OpenHandsWebSocketConfig,
    pub mcp: OpenHandsMcpConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsTransportConfig {
    pub base_url: String,
    pub session_api_key_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsLocalServerConfig {
    pub enabled: bool,
    pub command: Vec<String>,
    pub startup_timeout_ms: u64,
    pub readiness_probe_path: String,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsConversationConfig {
    pub reuse_policy: String,
    pub persistence_dir_relative: PathBuf,
    pub max_iterations: u64,
    pub stuck_detection: bool,
    pub confirmation_policy: Option<OpenHandsConfirmationPolicy>,
    pub agent: Option<OpenHandsConversationAgentConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsConversationAgentConfig {
    pub kind: Option<String>,
    pub llm: Option<OpenHandsLlmConfig>,
    pub log_completions: bool,
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenHandsLlmConfig {
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url_env: Option<String>,
    pub options: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsWebSocketConfig {
    pub enabled: bool,
    pub ready_timeout_ms: u64,
    pub reconnect_initial_ms: u64,
    pub reconnect_max_ms: u64,
    pub auth_mode: String,
    pub query_param_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsMcpConfig {
    pub stdio_servers: Vec<OpenHandsStdioServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsStdioServerConfig {
    pub name: String,
    pub command: Vec<String>,
}

pub trait Environment {
    fn get(&self, name: &str) -> Option<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnvironment;

impl Environment for ProcessEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        std::env::var_os(name).map(|value| value.to_string_lossy().into_owned())
    }
}

impl Environment for BTreeMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

impl Environment for HashMap<String, String> {
    fn get(&self, name: &str) -> Option<String> {
        self.get(name).cloned()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PromptContext<'a, T>
where
    T: Serialize,
{
    pub issue: &'a T,
    pub attempt: Option<u32>,
}
