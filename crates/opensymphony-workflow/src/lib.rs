//! `WORKFLOW.md` loading, typed config resolution, and strict prompt rendering.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Component;
use std::path::{Path, PathBuf};

use opensymphony_domain::Issue;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tera::{Context, Tera};
use thiserror::Error;

const DEFAULT_TRACKER_ENDPOINT: &str = "https://api.linear.app/graphql";
const DEFAULT_ACTIVE_STATES: [&str; 2] = ["Todo", "In Progress"];
const DEFAULT_TERMINAL_STATES: [&str; 5] = ["Closed", "Cancelled", "Canceled", "Duplicate", "Done"];
const DEFAULT_POLL_INTERVAL_MS: u64 = 30_000;
const DEFAULT_WORKSPACE_ROOT: &str = "/symphony_workspaces";
const DEFAULT_HOOK_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MAX_CONCURRENT_AGENTS: usize = 10;
const DEFAULT_MAX_TURNS: u32 = 20;
const DEFAULT_MAX_RETRY_BACKOFF_MS: u64 = 300_000;
const DEFAULT_STALL_TIMEOUT_MS: u64 = 300_000;
const DEFAULT_OPENHANDS_BASE_URL: &str = "http://127.0.0.1:8000";
const DEFAULT_LOCAL_SERVER_COMMAND: [&str; 3] = ["python", "-m", "openhands.agent_server"];
const DEFAULT_STARTUP_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_READINESS_PROBE_PATH: &str = "/openapi.json";
const DEFAULT_PERSISTENCE_DIR_RELATIVE: &str = ".opensymphony/openhands";
const DEFAULT_MAX_ITERATIONS: u32 = 500;
const DEFAULT_CONFIRMATION_POLICY_KIND: &str = "NeverConfirm";
const DEFAULT_AGENT_KIND: &str = "Agent";
const DEFAULT_LINEAR_API_KEY_ENV: &str = "LINEAR_API_KEY";
const DEFAULT_READY_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_RECONNECT_INITIAL_MS: u64 = 1_000;
const DEFAULT_RECONNECT_MAX_MS: u64 = 30_000;
const DEFAULT_QUERY_PARAM_NAME: &str = "session_api_key";
const DEFAULT_PROMPT_TEMPLATE: &str = "You are working on an issue from Linear.";

/// Environment access abstraction used for deterministic tests.
pub trait EnvProvider {
    fn get_var(&self, name: &str) -> Option<String>;
}

/// The current process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessEnv;

impl EnvProvider for ProcessEnv {
    fn get_var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|value| !value.is_empty())
    }
}

impl EnvProvider for BTreeMap<String, String> {
    fn get_var(&self, name: &str) -> Option<String> {
        self.get(name).cloned().filter(|value| !value.is_empty())
    }
}

/// Raw workflow payload returned by front matter parsing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub config: Map<String, Value>,
    pub prompt_template: String,
}

/// Fully typed workflow contract used by downstream crates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub definition: WorkflowDefinition,
    pub config: WorkflowConfig,
}

impl Workflow {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, WorkflowError> {
        Self::load_from_path_with_env(path, &ProcessEnv)
    }

    pub fn load_from_path_with_env(
        path: impl AsRef<Path>,
        env: &impl EnvProvider,
    ) -> Result<Self, WorkflowError> {
        let path = path.as_ref();
        let contents =
            fs::read_to_string(path).map_err(|source| WorkflowError::MissingWorkflowFile {
                path: path.to_path_buf(),
                source,
            })?;
        Self::load_from_str_with_env(&contents, env)
    }

    pub fn load_from_str(contents: &str) -> Result<Self, WorkflowError> {
        Self::load_from_str_with_env(contents, &ProcessEnv)
    }

    pub fn load_from_str_with_env(
        contents: &str,
        env: &impl EnvProvider,
    ) -> Result<Self, WorkflowError> {
        let definition = parse_workflow_definition(contents)?;
        let config = WorkflowConfig::from_definition_with_env(&definition, env)?;
        Ok(Self { definition, config })
    }

    pub fn render_prompt(
        &self,
        issue: &Issue,
        attempt: Option<u32>,
    ) -> Result<String, WorkflowError> {
        let template = if self.definition.prompt_template.is_empty() {
            DEFAULT_PROMPT_TEMPLATE
        } else {
            &self.definition.prompt_template
        };

        #[derive(Serialize)]
        struct RenderContext<'a> {
            issue: &'a Issue,
            attempt: Option<u32>,
        }

        let mut tera = Tera::default();
        tera.autoescape_on(Vec::<&'static str>::new());
        tera.add_raw_template("workflow", template)
            .map_err(WorkflowError::TemplateParseError)?;

        let context = Context::from_serialize(RenderContext { issue, attempt })
            .map_err(WorkflowError::TemplateRenderError)?;
        tera.render("workflow", &context)
            .map_err(WorkflowError::TemplateRenderError)
    }
}

/// Typed configuration view resolved from workflow front matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowConfig {
    pub tracker: TrackerConfig,
    pub polling: PollingConfig,
    pub workspace: WorkspaceConfig,
    pub hooks: HooksConfig,
    pub agent: AgentConfig,
    pub openhands: OpenHandsConfig,
}

impl WorkflowConfig {
    pub fn from_definition(definition: &WorkflowDefinition) -> Result<Self, WorkflowError> {
        Self::from_definition_with_env(definition, &ProcessEnv)
    }

    pub fn from_definition_with_env(
        definition: &WorkflowDefinition,
        env: &impl EnvProvider,
    ) -> Result<Self, WorkflowError> {
        let raw: RawWorkflowConfig =
            serde_json::from_value(Value::Object(definition.config.clone()))
                .map_err(WorkflowError::WorkflowParseError)?;

        Ok(Self {
            tracker: TrackerConfig::from_raw(raw.tracker.unwrap_or_default(), env)?,
            polling: PollingConfig::from_raw(raw.polling.unwrap_or_default())?,
            workspace: WorkspaceConfig::from_raw(raw.workspace.unwrap_or_default(), env)?,
            hooks: HooksConfig::from_raw(raw.hooks.unwrap_or_default())?,
            agent: AgentConfig::from_raw(raw.agent.unwrap_or_default())?,
            openhands: OpenHandsConfig::from_raw(raw.openhands.unwrap_or_default(), env)?,
        })
    }
}

/// Supported tracker kinds for the local MVP.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackerKind {
    Linear,
}

/// Tracker settings required for orchestration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerConfig {
    pub kind: Option<TrackerKind>,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub project_slug: Option<String>,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
}

impl TrackerConfig {
    fn from_raw(raw: RawTrackerConfig, env: &impl EnvProvider) -> Result<Self, WorkflowError> {
        let kind = match raw.kind.as_deref() {
            None => None,
            Some("linear") | Some("Linear") => Some(TrackerKind::Linear),
            Some(other) => {
                return Err(WorkflowError::invalid_config(
                    "tracker.kind",
                    format!("unsupported tracker kind `{other}`"),
                ));
            }
        };

        let api_key = match raw.api_key.as_deref() {
            Some(value) => Some(resolve_explicit_config_value(
                value,
                env,
                "tracker.api_key",
            )?),
            None if matches!(kind, Some(TrackerKind::Linear)) => {
                env.get_var(DEFAULT_LINEAR_API_KEY_ENV)
            }
            None => None,
        }
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        let project_slug = raw.project_slug.and_then(|slug| {
            let trimmed = slug.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });

        if matches!(kind, Some(TrackerKind::Linear)) && project_slug.is_none() {
            return Err(WorkflowError::invalid_config(
                "tracker.project_slug",
                "must be set for the Linear tracker",
            ));
        }
        if matches!(kind, Some(TrackerKind::Linear)) && api_key.is_none() {
            return Err(WorkflowError::invalid_config(
                "tracker.api_key",
                format!(
                    "must be set for the Linear tracker via `tracker.api_key` or `{DEFAULT_LINEAR_API_KEY_ENV}`"
                ),
            ));
        }

        Ok(Self {
            kind,
            endpoint: raw
                .endpoint
                .unwrap_or_else(|| DEFAULT_TRACKER_ENDPOINT.to_string()),
            api_key,
            project_slug,
            active_states: normalize_states(
                raw.active_states.unwrap_or_else(default_active_states),
            ),
            terminal_states: normalize_states(
                raw.terminal_states.unwrap_or_else(default_terminal_states),
            ),
        })
    }
}

/// Poll loop settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollingConfig {
    pub interval_ms: u64,
}

impl PollingConfig {
    fn from_raw(raw: RawPollingConfig) -> Result<Self, WorkflowError> {
        Ok(Self {
            interval_ms: parse_u64(
                raw.interval_ms,
                DEFAULT_POLL_INTERVAL_MS,
                "polling.interval_ms",
            )?,
        })
    }
}

/// Workspace path settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub root: PathBuf,
}

impl WorkspaceConfig {
    fn from_raw(raw: RawWorkspaceConfig, env: &impl EnvProvider) -> Result<Self, WorkflowError> {
        Ok(Self {
            root: resolve_path_like(
                raw.root.as_deref(),
                DEFAULT_WORKSPACE_ROOT,
                env,
                "workspace.root",
            )?,
        })
    }
}

/// Repository-owned workspace hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HooksConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    pub timeout_ms: u64,
}

impl HooksConfig {
    fn from_raw(raw: RawHooksConfig) -> Result<Self, WorkflowError> {
        Ok(Self {
            after_create: raw.after_create,
            before_run: raw.before_run,
            after_run: raw.after_run,
            before_remove: raw.before_remove,
            timeout_ms: parse_non_positive_as_default(
                raw.timeout_ms,
                DEFAULT_HOOK_TIMEOUT_MS,
                "hooks.timeout_ms",
            )?,
        })
    }
}

/// Scheduler-relevant agent limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub max_concurrent_agents: usize,
    pub max_turns: u32,
    pub max_retry_backoff_ms: u64,
    pub stall_timeout_ms: Option<u64>,
    pub max_concurrent_agents_by_state: BTreeMap<String, usize>,
}

impl AgentConfig {
    fn from_raw(raw: RawAgentConfig) -> Result<Self, WorkflowError> {
        Ok(Self {
            max_concurrent_agents: parse_usize(
                raw.max_concurrent_agents,
                DEFAULT_MAX_CONCURRENT_AGENTS,
                "agent.max_concurrent_agents",
            )?,
            max_turns: parse_u32(raw.max_turns, DEFAULT_MAX_TURNS, "agent.max_turns")?,
            max_retry_backoff_ms: parse_u64(
                raw.max_retry_backoff_ms,
                DEFAULT_MAX_RETRY_BACKOFF_MS,
                "agent.max_retry_backoff_ms",
            )?,
            stall_timeout_ms: parse_optional_positive_u64(
                raw.stall_timeout_ms,
                DEFAULT_STALL_TIMEOUT_MS,
                "agent.stall_timeout_ms",
            )?,
            max_concurrent_agents_by_state: normalize_state_limits(
                raw.max_concurrent_agents_by_state,
            )?,
        })
    }
}

/// OpenHands-specific runtime namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsConfig {
    pub transport: OpenHandsTransportConfig,
    pub local_server: OpenHandsLocalServerConfig,
    pub conversation: OpenHandsConversationConfig,
    pub websocket: OpenHandsWebSocketConfig,
    pub mcp: OpenHandsMcpConfig,
}

impl OpenHandsConfig {
    fn from_raw(raw: RawOpenHandsConfig, env: &impl EnvProvider) -> Result<Self, WorkflowError> {
        let websocket = OpenHandsWebSocketConfig::from_raw(raw.websocket.unwrap_or_default())?;

        Ok(Self {
            transport: OpenHandsTransportConfig::from_raw(raw.transport.unwrap_or_default()),
            local_server: OpenHandsLocalServerConfig::from_raw(
                raw.local_server.unwrap_or_default(),
            )?,
            conversation: OpenHandsConversationConfig::from_raw(
                raw.conversation.unwrap_or_default(),
                env,
            )?,
            websocket,
            mcp: OpenHandsMcpConfig::from_raw(raw.mcp.unwrap_or_default())?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsTransportConfig {
    pub base_url: String,
    pub session_api_key_env: Option<String>,
}

impl OpenHandsTransportConfig {
    fn from_raw(raw: RawOpenHandsTransportConfig) -> Self {
        Self {
            base_url: raw
                .base_url
                .unwrap_or_else(|| DEFAULT_OPENHANDS_BASE_URL.to_string()),
            session_api_key_env: raw.session_api_key_env,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsLocalServerConfig {
    pub enabled: bool,
    pub command: Vec<String>,
    pub startup_timeout_ms: u64,
    pub readiness_probe_path: String,
    pub env: BTreeMap<String, String>,
}

impl OpenHandsLocalServerConfig {
    fn from_raw(raw: RawOpenHandsLocalServerConfig) -> Result<Self, WorkflowError> {
        Ok(Self {
            enabled: raw.enabled.unwrap_or(false),
            command: normalize_command(
                raw.command,
                DEFAULT_LOCAL_SERVER_COMMAND.as_slice(),
                "openhands.local_server.command",
            )?,
            startup_timeout_ms: parse_u64(
                raw.startup_timeout_ms,
                DEFAULT_STARTUP_TIMEOUT_MS,
                "openhands.local_server.startup_timeout_ms",
            )?,
            readiness_probe_path: raw
                .readiness_probe_path
                .unwrap_or_else(|| DEFAULT_READINESS_PROBE_PATH.to_string()),
            env: raw.env.unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConversationReusePolicy {
    PerIssue,
    Fresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsConversationConfig {
    pub reuse_policy: ConversationReusePolicy,
    pub persistence_dir_relative: PathBuf,
    pub max_iterations: u32,
    pub stuck_detection: bool,
    pub confirmation_policy_kind: String,
    pub agent: OpenHandsAgentProfile,
}

impl OpenHandsConversationConfig {
    fn from_raw(
        raw: RawOpenHandsConversationConfig,
        env: &impl EnvProvider,
    ) -> Result<Self, WorkflowError> {
        let reuse_policy = match raw.reuse_policy.as_deref() {
            None | Some("per_issue") => ConversationReusePolicy::PerIssue,
            Some("fresh") => ConversationReusePolicy::Fresh,
            Some(other) => {
                return Err(WorkflowError::invalid_config(
                    "openhands.conversation.reuse_policy",
                    format!("unsupported reuse policy `{other}`"),
                ));
            }
        };

        let persistence_dir_relative = PathBuf::from(
            raw.persistence_dir_relative
                .unwrap_or_else(|| DEFAULT_PERSISTENCE_DIR_RELATIVE.to_string()),
        );
        if persistence_dir_relative.is_absolute()
            || persistence_dir_relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(WorkflowError::invalid_config(
                "openhands.conversation.persistence_dir_relative",
                "must remain relative to the issue workspace",
            ));
        }

        Ok(Self {
            reuse_policy,
            persistence_dir_relative,
            max_iterations: parse_u32(
                raw.max_iterations,
                DEFAULT_MAX_ITERATIONS,
                "openhands.conversation.max_iterations",
            )?,
            stuck_detection: raw.stuck_detection.unwrap_or(true),
            confirmation_policy_kind: raw
                .confirmation_policy
                .and_then(|policy| policy.kind)
                .unwrap_or_else(|| DEFAULT_CONFIRMATION_POLICY_KIND.to_string()),
            agent: OpenHandsAgentProfile::from_raw(raw.agent.unwrap_or_default(), env)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsAgentProfile {
    pub kind: String,
    pub llm: OpenHandsLlmConfig,
    pub log_completions: bool,
}

impl OpenHandsAgentProfile {
    fn from_raw(
        raw: RawOpenHandsAgentProfile,
        env: &impl EnvProvider,
    ) -> Result<Self, WorkflowError> {
        Ok(Self {
            kind: raw.kind.unwrap_or_else(|| DEFAULT_AGENT_KIND.to_string()),
            llm: OpenHandsLlmConfig::from_raw(raw.llm.unwrap_or_default(), env)?,
            log_completions: raw.log_completions.unwrap_or(false),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsLlmConfig {
    pub model: Option<String>,
    pub api_key_env: Option<String>,
    pub base_url_env: Option<String>,
}

impl OpenHandsLlmConfig {
    fn from_raw(raw: RawOpenHandsLlmConfig, env: &impl EnvProvider) -> Result<Self, WorkflowError> {
        Ok(Self {
            model: resolve_optional_explicit_config_value(
                raw.model.as_deref(),
                env,
                "openhands.conversation.agent.llm.model",
            )?,
            api_key_env: raw.api_key_env,
            base_url_env: raw.base_url_env,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketAuthMode {
    Auto,
    Header,
    QueryParam,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsWebSocketConfig {
    pub enabled: bool,
    pub ready_timeout_ms: u64,
    pub reconnect_initial_ms: u64,
    pub reconnect_max_ms: u64,
    pub auth_mode: WebSocketAuthMode,
    pub query_param_name: String,
}

impl OpenHandsWebSocketConfig {
    fn from_raw(raw: RawOpenHandsWebSocketConfig) -> Result<Self, WorkflowError> {
        let reconnect_initial_ms = parse_u64(
            raw.reconnect_initial_ms,
            DEFAULT_RECONNECT_INITIAL_MS,
            "openhands.websocket.reconnect_initial_ms",
        )?;
        let reconnect_max_ms = parse_u64(
            raw.reconnect_max_ms,
            DEFAULT_RECONNECT_MAX_MS,
            "openhands.websocket.reconnect_max_ms",
        )?;
        if reconnect_max_ms < reconnect_initial_ms {
            return Err(WorkflowError::invalid_config(
                "openhands.websocket.reconnect_max_ms",
                "must be greater than or equal to reconnect_initial_ms",
            ));
        }

        let auth_mode = match raw.auth_mode.as_deref() {
            None | Some("auto") => WebSocketAuthMode::Auto,
            Some("header") => WebSocketAuthMode::Header,
            Some("query_param") => WebSocketAuthMode::QueryParam,
            Some("none") => WebSocketAuthMode::None,
            Some(other) => {
                return Err(WorkflowError::invalid_config(
                    "openhands.websocket.auth_mode",
                    format!("unsupported auth mode `{other}`"),
                ));
            }
        };

        Ok(Self {
            enabled: raw.enabled.unwrap_or(true),
            ready_timeout_ms: parse_u64(
                raw.ready_timeout_ms,
                DEFAULT_READY_TIMEOUT_MS,
                "openhands.websocket.ready_timeout_ms",
            )?,
            reconnect_initial_ms,
            reconnect_max_ms,
            auth_mode,
            query_param_name: raw
                .query_param_name
                .unwrap_or_else(|| DEFAULT_QUERY_PARAM_NAME.to_string()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenHandsMcpConfig {
    pub stdio_servers: Vec<StdioMcpServerConfig>,
}

impl OpenHandsMcpConfig {
    fn from_raw(raw: RawOpenHandsMcpConfig) -> Result<Self, WorkflowError> {
        Ok(Self {
            stdio_servers: raw
                .stdio_servers
                .unwrap_or_default()
                .into_iter()
                .map(StdioMcpServerConfig::from_raw)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StdioMcpServerConfig {
    pub name: String,
    pub command: Vec<String>,
}

impl StdioMcpServerConfig {
    fn from_raw(raw: RawStdioMcpServerConfig) -> Result<Self, WorkflowError> {
        if raw.name.trim().is_empty() {
            return Err(WorkflowError::invalid_config(
                "openhands.mcp.stdio_servers.name",
                "must not be empty",
            ));
        }

        Ok(Self {
            name: raw.name,
            command: normalize_command(
                Some(raw.command),
                &[],
                "openhands.mcp.stdio_servers.command",
            )?,
        })
    }
}

/// Workflow parsing and config errors.
#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("missing workflow file `{path}`")]
    MissingWorkflowFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("workflow parse error")]
    WorkflowParseError(#[source] serde_json::Error),
    #[error("workflow parse error: {0}")]
    WorkflowStructure(String),
    #[error("workflow front matter must decode to a map")]
    WorkflowFrontMatterNotAMap,
    #[error("template parse error")]
    TemplateParseError(#[source] tera::Error),
    #[error("template render error")]
    TemplateRenderError(#[source] tera::Error),
    #[error("invalid config for `{field}`: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

impl WorkflowError {
    fn invalid_config(field: &'static str, reason: impl Into<String>) -> Self {
        Self::InvalidConfig {
            field,
            reason: reason.into(),
        }
    }
}

/// Parse a `WORKFLOW.md` payload into raw front matter and prompt template text.
pub fn parse_workflow_definition(contents: &str) -> Result<WorkflowDefinition, WorkflowError> {
    let (front_matter, body) = split_front_matter(contents)?;
    let config = if let Some(front_matter) = front_matter {
        let value: serde_yaml::Value = serde_yaml::from_str(front_matter)
            .map_err(|error| WorkflowError::WorkflowStructure(error.to_string()))?;
        let mapping = match value {
            serde_yaml::Value::Mapping(mapping) => mapping,
            _ => return Err(WorkflowError::WorkflowFrontMatterNotAMap),
        };

        match serde_json::to_value(mapping)
            .map_err(|error| WorkflowError::WorkflowStructure(error.to_string()))?
        {
            Value::Object(object) => object,
            _ => return Err(WorkflowError::WorkflowFrontMatterNotAMap),
        }
    } else {
        Map::new()
    };

    Ok(WorkflowDefinition {
        config,
        prompt_template: body.trim().to_string(),
    })
}

fn split_front_matter(contents: &str) -> Result<(Option<&str>, &str), WorkflowError> {
    let Some(first_line) = contents.split_inclusive('\n').next() else {
        return Ok((None, ""));
    };
    if trim_line(first_line) != "---" {
        return Ok((None, contents));
    }

    let mut offset = first_line.len();
    let front_matter_start = offset;
    for line in contents[offset..].split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if trim_line(line) == "---" {
            return Ok((
                Some(&contents[front_matter_start..line_start]),
                &contents[offset..],
            ));
        }
    }

    Err(WorkflowError::WorkflowStructure(
        "front matter started with `---` but no closing delimiter was found".to_string(),
    ))
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}

fn parse_i64(value: IntLike, field: &'static str) -> Result<i64, WorkflowError> {
    match value {
        IntLike::Integer(number) => Ok(number),
        IntLike::String(number) => number.parse::<i64>().map_err(|_| {
            WorkflowError::invalid_config(field, format!("expected an integer, got `{number}`"))
        }),
    }
}

fn parse_u64(
    value: Option<IntLike>,
    default: u64,
    field: &'static str,
) -> Result<u64, WorkflowError> {
    match value {
        Some(value) => {
            let parsed = parse_i64(value, field)?;
            u64::try_from(parsed)
                .map_err(|_| WorkflowError::invalid_config(field, "must be a non-negative integer"))
        }
        None => Ok(default),
    }
}

fn parse_u32(
    value: Option<IntLike>,
    default: u32,
    field: &'static str,
) -> Result<u32, WorkflowError> {
    let parsed = parse_u64(value, u64::from(default), field)?;
    u32::try_from(parsed)
        .map_err(|_| WorkflowError::invalid_config(field, "value does not fit in u32"))
}

fn parse_usize(
    value: Option<IntLike>,
    default: usize,
    field: &'static str,
) -> Result<usize, WorkflowError> {
    let parsed = parse_u64(value, default as u64, field)?;
    usize::try_from(parsed)
        .map_err(|_| WorkflowError::invalid_config(field, "value does not fit in usize"))
}

fn parse_non_positive_as_default(
    value: Option<IntLike>,
    default: u64,
    field: &'static str,
) -> Result<u64, WorkflowError> {
    match value {
        Some(raw) => {
            let parsed = parse_i64(raw, field)?;
            if parsed <= 0 {
                Ok(default)
            } else {
                u64::try_from(parsed).map_err(|_| {
                    WorkflowError::invalid_config(field, "must be a non-negative integer")
                })
            }
        }
        None => Ok(default),
    }
}

fn parse_optional_positive_u64(
    value: Option<IntLike>,
    default: u64,
    field: &'static str,
) -> Result<Option<u64>, WorkflowError> {
    match value {
        Some(raw) => {
            let parsed = parse_i64(raw, field)?;
            if parsed <= 0 {
                Ok(None)
            } else {
                Ok(Some(u64::try_from(parsed).map_err(|_| {
                    WorkflowError::invalid_config(field, "must be a non-negative integer")
                })?))
            }
        }
        None => Ok(Some(default)),
    }
}

fn normalize_states(states: Vec<String>) -> Vec<String> {
    states
        .into_iter()
        .map(|state| state.trim().to_lowercase())
        .filter(|state| !state.is_empty())
        .collect()
}

fn normalize_state_limits(
    raw: Option<BTreeMap<String, IntLike>>,
) -> Result<BTreeMap<String, usize>, WorkflowError> {
    let mut normalized = BTreeMap::new();
    for (state, value) in raw.unwrap_or_default() {
        let limit = parse_i64(value, "agent.max_concurrent_agents_by_state")?;
        if limit > 0 {
            normalized.insert(
                state.trim().to_lowercase(),
                usize::try_from(limit).map_err(|_| {
                    WorkflowError::invalid_config(
                        "agent.max_concurrent_agents_by_state",
                        "value does not fit in usize",
                    )
                })?,
            );
        }
    }
    Ok(normalized)
}

fn normalize_command(
    raw: Option<Vec<String>>,
    default: &[&str],
    field: &'static str,
) -> Result<Vec<String>, WorkflowError> {
    let command = raw.unwrap_or_else(|| default.iter().map(|part| part.to_string()).collect());
    if command.is_empty() || command.iter().all(|part| part.trim().is_empty()) {
        return Err(WorkflowError::invalid_config(
            field,
            "command must not be empty",
        ));
    }
    Ok(command)
}

fn resolve_explicit_config_value(
    raw: &str,
    env: &impl EnvProvider,
    field: &'static str,
) -> Result<String, WorkflowError> {
    if let Some(name) = parse_env_token(raw) {
        return env.get_var(name).ok_or_else(|| {
            WorkflowError::invalid_config(
                field,
                format!("environment variable `{name}` is required"),
            )
        });
    }
    Ok(raw.to_string())
}

fn resolve_optional_explicit_config_value(
    raw: Option<&str>,
    env: &impl EnvProvider,
    field: &'static str,
) -> Result<Option<String>, WorkflowError> {
    raw.map(|value| resolve_explicit_config_value(value, env, field))
        .transpose()
}

fn parse_env_token(raw: &str) -> Option<&str> {
    if let Some(stripped) = raw
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        return Some(stripped);
    }
    raw.strip_prefix('$')
}

fn resolve_path_like(
    raw: Option<&str>,
    default: &str,
    env: &impl EnvProvider,
    field: &'static str,
) -> Result<PathBuf, WorkflowError> {
    let raw = match raw {
        Some(value) => resolve_explicit_config_value(value, env, field)?,
        None => default.to_string(),
    };
    let expanded = if raw == "~" {
        env.get_var("HOME")
            .ok_or_else(|| WorkflowError::invalid_config(field, "HOME is required to expand `~`"))?
    } else if let Some(rest) = raw.strip_prefix("~/") {
        let home = env.get_var("HOME").ok_or_else(|| {
            WorkflowError::invalid_config(field, "HOME is required to expand `~`")
        })?;
        format!("{home}/{rest}")
    } else {
        raw
    };
    Ok(PathBuf::from(expanded))
}

fn default_active_states() -> Vec<String> {
    DEFAULT_ACTIVE_STATES
        .into_iter()
        .map(String::from)
        .collect()
}

fn default_terminal_states() -> Vec<String> {
    DEFAULT_TERMINAL_STATES
        .into_iter()
        .map(String::from)
        .collect()
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawWorkflowConfig {
    #[serde(default)]
    tracker: Option<RawTrackerConfig>,
    #[serde(default)]
    polling: Option<RawPollingConfig>,
    #[serde(default)]
    workspace: Option<RawWorkspaceConfig>,
    #[serde(default)]
    hooks: Option<RawHooksConfig>,
    #[serde(default)]
    agent: Option<RawAgentConfig>,
    #[serde(default)]
    openhands: Option<RawOpenHandsConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawTrackerConfig {
    kind: Option<String>,
    endpoint: Option<String>,
    api_key: Option<String>,
    project_slug: Option<String>,
    active_states: Option<Vec<String>>,
    terminal_states: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawPollingConfig {
    interval_ms: Option<IntLike>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawWorkspaceConfig {
    root: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawHooksConfig {
    after_create: Option<String>,
    before_run: Option<String>,
    after_run: Option<String>,
    before_remove: Option<String>,
    timeout_ms: Option<IntLike>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RawAgentConfig {
    max_concurrent_agents: Option<IntLike>,
    max_turns: Option<IntLike>,
    max_retry_backoff_ms: Option<IntLike>,
    stall_timeout_ms: Option<IntLike>,
    max_concurrent_agents_by_state: Option<BTreeMap<String, IntLike>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsConfig {
    transport: Option<RawOpenHandsTransportConfig>,
    local_server: Option<RawOpenHandsLocalServerConfig>,
    conversation: Option<RawOpenHandsConversationConfig>,
    websocket: Option<RawOpenHandsWebSocketConfig>,
    mcp: Option<RawOpenHandsMcpConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsTransportConfig {
    base_url: Option<String>,
    session_api_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsLocalServerConfig {
    enabled: Option<bool>,
    command: Option<Vec<String>>,
    startup_timeout_ms: Option<IntLike>,
    readiness_probe_path: Option<String>,
    env: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsConversationConfig {
    reuse_policy: Option<String>,
    persistence_dir_relative: Option<String>,
    max_iterations: Option<IntLike>,
    stuck_detection: Option<bool>,
    confirmation_policy: Option<RawConfirmationPolicy>,
    agent: Option<RawOpenHandsAgentProfile>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfirmationPolicy {
    kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsAgentProfile {
    kind: Option<String>,
    llm: Option<RawOpenHandsLlmConfig>,
    log_completions: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsLlmConfig {
    model: Option<String>,
    api_key_env: Option<String>,
    base_url_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsWebSocketConfig {
    enabled: Option<bool>,
    ready_timeout_ms: Option<IntLike>,
    reconnect_initial_ms: Option<IntLike>,
    reconnect_max_ms: Option<IntLike>,
    auth_mode: Option<String>,
    query_param_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawOpenHandsMcpConfig {
    stdio_servers: Option<Vec<RawStdioMcpServerConfig>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStdioMcpServerConfig {
    name: String,
    command: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum IntLike {
    Integer(i64),
    String(String),
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    fn env() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("HOME".into(), "/home/tester".into()),
            ("LINEAR_KEY".into(), "linear-secret".into()),
            ("OPENHANDS_MODEL".into(), "gpt-5.4".into()),
        ])
    }

    fn issue() -> Issue {
        Issue::new(
            "1",
            "OSYM-1",
            "Foundation",
            "In Progress",
            Utc.timestamp_opt(1, 0).single().unwrap(),
        )
        .with_description("Implement the contracts")
        .with_labels(["foundation", "urgent"])
    }

    #[test]
    fn parses_front_matter_and_prompt() {
        let workflow = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $LINEAR_KEY
---

# Assignment
Hello {{ issue.identifier }}
"#,
            &env(),
        )
        .unwrap();

        assert_eq!(
            workflow.definition.config["tracker"]["project_slug"],
            "opensymphony"
        );
        assert_eq!(
            workflow.definition.prompt_template,
            "# Assignment\nHello {{ issue.identifier }}"
        );
        assert_eq!(workflow.config.tracker.kind, Some(TrackerKind::Linear));
    }

    #[test]
    fn allows_workflows_without_front_matter() {
        let workflow = Workflow::load_from_str_with_env("Hello {{ issue.title }}", &env()).unwrap();

        assert!(workflow.definition.config.is_empty());
        assert_eq!(
            workflow.definition.prompt_template,
            "Hello {{ issue.title }}"
        );
    }

    #[test]
    fn rejects_non_map_front_matter() {
        let error = Workflow::load_from_str_with_env(
            r#"---
  - invalid
---
body
"#,
            &env(),
        )
        .unwrap_err();

        assert!(matches!(error, WorkflowError::WorkflowFrontMatterNotAMap));
    }

    #[test]
    fn resolves_env_indirection_and_normalizes_paths() {
        let workflow = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $LINEAR_KEY
workspace:
  root: ~/opensymphony/workspaces
agent:
  max_concurrent_agents_by_state:
    In Progress: 2
    Todo: "0"
openhands:
  conversation:
    persistence_dir_relative: .opensymphony/openhands
    agent:
      llm:
        model: ${OPENHANDS_MODEL}
---
Hello
"#,
            &env(),
        )
        .unwrap();

        assert_eq!(
            workflow.config.tracker.api_key.as_deref(),
            Some("linear-secret")
        );
        assert_eq!(
            workflow.config.workspace.root,
            PathBuf::from("/home/tester/opensymphony/workspaces")
        );
        assert_eq!(
            workflow.config.agent.max_concurrent_agents_by_state,
            BTreeMap::from([("in progress".into(), 2usize)])
        );
        assert_eq!(
            workflow
                .config
                .openhands
                .conversation
                .agent
                .llm
                .model
                .as_deref(),
            Some("gpt-5.4")
        );
    }

    #[test]
    fn rejects_missing_openhands_model_env_reference() {
        let mut env = env();
        env.remove("OPENHANDS_MODEL");

        let error = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $LINEAR_KEY
openhands:
  conversation:
    agent:
      llm:
        model: ${OPENHANDS_MODEL}
---
Hello
"#,
            &env,
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "openhands.conversation.agent.llm.model")
        );
    }

    #[test]
    fn rejects_unknown_top_level_workflow_sections() {
        let error = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $LINEAR_KEY
workpace:
  root: ~/wrong
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(matches!(error, WorkflowError::WorkflowParseError(_)));
    }

    #[test]
    fn strict_rendering_rejects_unknown_variables() {
        let workflow =
            Workflow::load_from_str_with_env("Hello {{ issue.missing_field }}", &env()).unwrap();
        let error = workflow.render_prompt(&issue(), None).unwrap_err();

        assert!(matches!(error, WorkflowError::TemplateRenderError(_)));
    }

    #[test]
    fn strict_rendering_rejects_unknown_filters() {
        let workflow =
            Workflow::load_from_str_with_env("Hello {{ issue.identifier | unknown }}", &env())
                .unwrap();
        let error = workflow.render_prompt(&issue(), None).unwrap_err();

        assert!(matches!(error, WorkflowError::TemplateRenderError(_)));
    }

    #[test]
    fn renders_continuation_context() {
        let workflow = Workflow::load_from_str_with_env(
            r#"{% if attempt %}attempt={{ attempt }}{% endif %} {{ issue.identifier }}"#,
            &env(),
        )
        .unwrap();

        let rendered = workflow.render_prompt(&issue(), Some(2)).unwrap();

        assert_eq!(rendered, "attempt=2 OSYM-1");
    }

    #[test]
    fn rejects_linear_tracker_without_project_slug() {
        let error = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "tracker.project_slug")
        );
    }

    #[test]
    fn accepts_linear_tracker_with_process_env_api_key() {
        let mut env = env();
        env.insert("LINEAR_API_KEY".into(), "process-linear-key".into());

        let workflow = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
---
Hello
"#,
            &env,
        )
        .unwrap();

        assert_eq!(
            workflow.config.tracker.api_key.as_deref(),
            Some("process-linear-key")
        );
    }

    #[test]
    fn rejects_linear_tracker_with_unresolved_api_key_env() {
        let error = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $MISSING_LINEAR_KEY
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "tracker.api_key")
        );
    }

    #[test]
    fn rejects_missing_env_backed_workspace_root() {
        let error = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $LINEAR_KEY
workspace:
  root: $WORKSPACE_ROOT
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "workspace.root")
        );
    }

    #[test]
    fn rejects_unknown_openhands_keys() {
        let error = Workflow::load_from_str_with_env(
            r#"---
tracker:
  kind: linear
  project_slug: opensymphony
  api_key: $LINEAR_KEY
openhands:
  websocket:
    reconnect_inital_ms: 1000
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::WorkflowParseError(_))
                || matches!(error, WorkflowError::InvalidConfig { .. })
        );
    }

    #[test]
    fn rejects_invalid_openhands_extension_values() {
        let error = Workflow::load_from_str_with_env(
            r#"---
openhands:
  websocket:
    reconnect_initial_ms: 5000
    reconnect_max_ms: 1000
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "openhands.websocket.reconnect_max_ms")
        );
    }

    #[test]
    fn rejects_absolute_openhands_persistence_path() {
        let error = Workflow::load_from_str_with_env(
            r#"---
openhands:
  conversation:
    persistence_dir_relative: /tmp/absolute
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "openhands.conversation.persistence_dir_relative")
        );
    }

    #[test]
    fn rejects_parent_directory_escape_in_persistence_path() {
        let error = Workflow::load_from_str_with_env(
            r#"---
openhands:
  conversation:
    persistence_dir_relative: ../shared
---
Hello
"#,
            &env(),
        )
        .unwrap_err();

        assert!(
            matches!(error, WorkflowError::InvalidConfig { field, .. } if field == "openhands.conversation.persistence_dir_relative")
        );
    }
}
