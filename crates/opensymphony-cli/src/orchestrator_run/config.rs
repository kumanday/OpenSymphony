//! Runtime config loading for the `opensymphony run` command.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, RepositoryIdentity, RepositoryInventoryEntry, RepositoryRouting,
    RepositoryRoutingMode, SafeRemoteFingerprint,
};
use crate::opensymphony_memory::DEFAULT_PRIVATE_MEMORY_CONFIG_FILE;
use crate::opensymphony_openhands::OpenHandsConversationStorePaths;
use crate::opensymphony_workflow::{
    AgentFrontMatter, HooksFrontMatter, IntegerLike, OpenHandsFrontMatter, PollingFrontMatter,
    ResolvedWorkflow, RoutingFrontMatter, TrackerFrontMatter, WorkflowDefinition,
    WorkflowFrontMatter, WorkspaceFrontMatter,
};
use crate::opensymphony_workflow::{
    DEFAULT_ROUTING_HARNESS_ENV, DEFAULT_ROUTING_MODEL_ENV, DEFAULT_ROUTING_MODEL_PROFILE_ENV,
};
use crate::opensymphony_workspace::{
    CheckoutRepository, SSH_AUTH_SOCK_ENV, environment_variable_names_equal,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs;
use url::Url;

use super::{RunArgs, RunCommandError};

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const DEFAULT_USER_CONFIG_DIR: &str = ".opensymphony";
const DEFAULT_CONTROL_PLANE_BIND: &str = "127.0.0.1:2468";
const DEFAULT_MEMORY_SERVER_BIND: &str = "127.0.0.1:0";
const DEFAULT_MEMORY_TOKEN_ENV: &str = "OPENSYMPHONY_MEMORY_TOKEN";

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RunConfigFile {
    #[serde(default)]
    target_repo: Option<String>,
    #[serde(default)]
    control_plane: ControlPlaneConfigFile,
    #[serde(default)]
    openhands: RunOpenHandsConfigFile,
    #[serde(default)]
    memory: RunMemoryConfigFile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralConfigFile {
    #[serde(default)]
    schema_version: u32,
    instance: CentralInstanceFile,
    routing: CentralRoutingFile,
    #[serde(default)]
    tracker_profiles: BTreeMap<String, CentralTrackerProfileFile>,
    #[serde(default)]
    project_sets: BTreeMap<String, CentralProjectSetFile>,
    #[serde(default)]
    linear_projects: BTreeMap<String, CentralLinearProjectFile>,
    #[serde(default)]
    repositories: BTreeMap<String, CentralRepositoryFile>,
    #[serde(default)]
    credentials: BTreeMap<String, CentralCredentialFile>,
    #[serde(default)]
    review_profiles: BTreeMap<String, CentralReviewProfileFile>,
    #[serde(default)]
    workspace: Option<CentralWorkspaceFile>,
    #[serde(default)]
    scheduler: Option<CentralSchedulerFile>,
    #[serde(default)]
    hooks: CentralHooksFile,
    #[serde(default)]
    integration: Option<CentralIntegrationFile>,
    #[serde(default)]
    memory: Option<CentralMemoryFile>,
    #[serde(default)]
    control_plane: CentralControlPlaneFile,
    #[serde(default)]
    openhands: CentralOpenHandsFile,
    #[serde(default)]
    compatibility: CentralCompatibilityFile,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralInstanceFile {
    id: String,
    state_root: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralRoutingFile {
    mode: String,
    #[serde(default)]
    active_project_set: Option<String>,
    #[serde(default)]
    repository: Option<String>,
    #[serde(default)]
    harness: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    model_profile: Option<String>,
    #[serde(default)]
    harness_env: Option<String>,
    #[serde(default)]
    model_env: Option<String>,
    #[serde(default)]
    model_profile_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralTrackerProfileFile {
    provider: String,
    #[serde(default)]
    endpoint: Option<String>,
    credential: String,
    #[serde(default)]
    active_states: Vec<String>,
    #[serde(default)]
    terminal_states: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralProjectSetFile {
    tracker_profile: String,
    #[serde(default)]
    integration_instructions: Option<String>,
    projects: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralLinearProjectFile {
    provider_project_id: String,
    #[serde(default)]
    provider_project_slug: Option<String>,
    #[serde(default)]
    repositories: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralRepositoryFile {
    #[serde(default)]
    aliases: Vec<String>,
    remote: CentralRemoteFile,
    target_branch: String,
    credential: String,
    review_profile: String,
    instructions: CentralInstructionsFile,
    #[serde(default, alias = "path")]
    checkout_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralRemoteFile {
    provider: String,
    #[serde(default)]
    provider_id: Option<String>,
    locator: String,
    clone: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralInstructionsFile {
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralCredentialFile {
    kind: String,
    #[serde(default)]
    variable: Option<String>,
    #[serde(default)]
    reference: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CentralReviewProfileFile {
    provider: String,
    credential: String,
    #[serde(default)]
    required_checks: bool,
    #[serde(default)]
    required_review: bool,
    #[serde(default)]
    merge_method: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralWorkspaceFile {
    root: String,
    #[serde(default)]
    retain_failed: Option<bool>,
    #[serde(default)]
    cleanup_after_parent_finalization: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CentralSchedulerFile {
    max_concurrent_tasks: u64,
    #[serde(default)]
    max_concurrent_agents_by_state: BTreeMap<String, u64>,
    #[serde(default)]
    retry: CentralRetryFile,
    #[serde(default)]
    poll_interval_ms: Option<u64>,
    #[serde(default)]
    max_turns: Option<u64>,
    #[serde(default)]
    max_retry_backoff_ms: Option<u64>,
    #[serde(default)]
    stall_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CentralRetryFile {
    #[serde(default)]
    max_attempts: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralIntegrationFile {
    policy: String,
    #[serde(default)]
    use_shared_git_worktrees: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralHooksFile {
    #[serde(default)]
    after_create: Option<String>,
    #[serde(default)]
    before_run: Option<String>,
    #[serde(default)]
    after_run: Option<String>,
    #[serde(default)]
    before_remove: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralMemoryFile {
    catalog_root: String,
    #[serde(default = "default_true")]
    auto_capture: bool,
    #[serde(default)]
    auto_archive: bool,
    #[serde(default)]
    serve: bool,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    token_env: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralControlPlaneFile {
    #[serde(default)]
    bind: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralOpenHandsFile {
    #[serde(default)]
    tool_dir: Option<String>,
    #[serde(default)]
    transport_base_url: Option<String>,
    #[serde(default)]
    transport_session_api_key_env: Option<String>,
    #[serde(default)]
    front_matter: Option<OpenHandsFrontMatter>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CentralCompatibilityFile {
    #[serde(default)]
    allow_repo_local_config: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CentralRoutingMode {
    LegacySingle,
    ProjectSet,
}

#[derive(Debug, Clone)]
pub struct ResolvedCentralConfig {
    pub instance_id: String,
    pub state_root: PathBuf,
    pub workspace_root: Option<PathBuf>,
    pub retain_failed: bool,
    pub memory_catalog_root: Option<PathBuf>,
    pub project_set_id: Option<String>,
    pub mode: CentralRoutingMode,
    pub repository: Option<String>,
    pub integration_instructions: Option<ResolvedIntegrationInstructions>,
    pub repository_instruction_path: Option<PathBuf>,
    pub generation: String,
    pub repository_routing: RepositoryRouting,
    pub repository_checkouts: BTreeMap<String, CheckoutRepository>,
    pub retry_max_attempts: Option<u32>,
    runtime: RunConfigFile,
    pub workflow_front_matter: WorkflowFrontMatter,
    pub(crate) memory_sources: BTreeMap<String, ResolvedMemorySource>,
}

impl ResolvedCentralConfig {
    pub(crate) fn target_repo(&self) -> Option<PathBuf> {
        self.runtime.target_repo.as_deref().map(PathBuf::from)
    }

    pub(crate) fn require_legacy_target_repo(&self) -> Result<PathBuf, CentralConfigError> {
        if self.mode != CentralRoutingMode::LegacySingle {
            return Err(CentralConfigError::UnsupportedRoutingMode {
                mode: "project_set",
            });
        }
        self.target_repo()
            .ok_or(CentralConfigError::MissingLegacyRepository)
    }

    pub(crate) fn tool_dir(&self) -> Option<PathBuf> {
        self.runtime
            .openhands
            .tool_dir
            .as_deref()
            .map(PathBuf::from)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIntegrationInstructions {
    pub path: PathBuf,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedMemorySource {
    pub(crate) repository_id: String,
    pub(crate) remote_locator: String,
    pub(crate) checkout_path: PathBuf,
    pub(crate) project_scope_ids: BTreeSet<String>,
    pub(crate) target_branch: String,
}

#[derive(Debug, Error)]
pub enum CentralConfigError {
    #[error("failed to read central config {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse central config {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("central config schema_version must be 1")]
    UnsupportedSchema,
    #[error("central config field `{field}` must not be empty")]
    EmptyField { field: &'static str },
    #[error("central config reference `{field}` does not resolve")]
    InvalidReference { field: String },
    #[error("central config aliases must be unique: `{alias}`")]
    DuplicateAlias { alias: String },
    #[error(
        "central config repository policies must be unique for canonical identity `{identity}`"
    )]
    DuplicateRepositoryIdentity { identity: String },
    #[error("central config project routing key is ambiguous: `{key}`")]
    AmbiguousProjectRoutingKey { key: String },
    #[error("central config roots overlap: `{left}` and `{right}`")]
    OverlappingRoots { left: PathBuf, right: PathBuf },
    #[error("central config repository remote contains credentials")]
    CredentialBearingRemote,
    #[error("central config contains a literal secret in OpenHands environment")]
    LiteralSecret,
    #[error("central config repository instruction path must be relative and contained")]
    InvalidInstructionPath,
    #[error("central config integration instructions must be relative to the central config")]
    InvalidIntegrationPath,
    #[error("central config integration instructions cannot be inside a repository checkout")]
    IntegrationInsideCheckout,
    #[error("central config integration instructions could not be read: {path}: {source}")]
    ReadIntegration {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("legacy_single routing requires a configured repository")]
    MissingLegacyRepository,
    #[error("central routing mode `{mode}` is not supported by this command")]
    UnsupportedRoutingMode { mode: &'static str },
    #[error("legacy_single repository `{repository}` must define checkout_path")]
    MissingLegacyCheckout { repository: String },
    #[error("central config path is outside the selected instance roots")]
    InvalidRoot,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct ControlPlaneConfigFile {
    #[serde(default)]
    bind: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RunOpenHandsConfigFile {
    #[serde(default)]
    tool_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct RunMemoryConfigFile {
    #[serde(default)]
    auto_capture: Option<bool>,
    #[serde(default)]
    auto_archive: Option<bool>,
    #[serde(default)]
    serve: Option<bool>,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    token_env: Option<String>,
}

pub(super) struct RunMemoryConfig {
    pub(super) auto_capture: bool,
    pub(super) auto_archive: bool,
    pub(super) server: Option<RunMemoryServerConfig>,
}

#[derive(Debug, Clone)]
pub(super) struct RunMemoryServerConfig {
    pub(super) bind: SocketAddr,
    pub(super) token: Option<String>,
}

pub(super) struct RunRuntimeConfig {
    pub(super) config_path: Option<PathBuf>,
    pub(super) central_config: bool,
    pub(super) config_generation: String,
    pub(super) target_repo: PathBuf,
    pub(super) workflow_path: PathBuf,
    pub(super) workflow: ResolvedWorkflow,
    pub(super) bind: SocketAddr,
    pub(super) tool_dir: Option<PathBuf>,
    pub(super) openhands_conversation_store: Option<OpenHandsConversationStorePaths>,
    pub(super) retry_max_attempts: Option<u32>,
    pub(super) repository_routing: Option<RepositoryRouting>,
    pub(super) repository_checkouts: Option<BTreeMap<String, CheckoutRepository>>,
    pub(super) state_root: Option<PathBuf>,
    pub(super) memory_catalog_root: Option<PathBuf>,
    pub(super) memory_sources: BTreeMap<String, ResolvedMemorySource>,
    pub(super) project_set_id: Option<String>,
    pub(super) retain_failed: bool,
    pub(super) preserve_terminal_workspaces: bool,
    pub(super) memory: RunMemoryConfig,
}

pub(super) async fn resolve_runtime_config(
    args: &RunArgs,
) -> Result<RunRuntimeConfig, RunCommandError> {
    let cwd = env::current_dir().map_err(RunCommandError::CurrentDir)?;
    let config_path = select_config_path(&cwd, args.config.as_deref());
    let (
        config,
        config_generation,
        central_state_root,
        central_workspace_root,
        central_retain_failed,
        central_preserve_terminal_workspaces,
        central_memory_catalog_root,
        central_memory_sources,
        central_project_set_id,
        central_repository_instruction_path,
        central_workflow_front_matter,
        retry_max_attempts,
        central_repository_routing,
        central_repository_checkouts,
    ) = match &config_path {
        Some(path) => {
            let raw =
                fs::read_to_string(path)
                    .await
                    .map_err(|source| RunCommandError::ReadConfig {
                        path: path.clone(),
                        source,
                    })?;
            if looks_like_central_config(&raw) {
                let central = resolve_central_config(path, &raw)?;
                let repository_checkouts = Some(central.repository_checkouts);
                (
                    central.runtime,
                    central.generation,
                    Some(central.state_root),
                    central.workspace_root,
                    Some(central.retain_failed),
                    Some(true),
                    central.memory_catalog_root,
                    central.memory_sources,
                    central.project_set_id,
                    central.repository_instruction_path,
                    Some(central.workflow_front_matter),
                    central.retry_max_attempts,
                    Some(central.repository_routing),
                    repository_checkouts,
                )
            } else {
                (
                    parse_legacy_run_config(path, &raw)?,
                    generation_hash(raw.as_bytes()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    BTreeMap::new(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
        }
        None => (
            RunConfigFile::default(),
            "legacy-unconfigured".to_string(),
            None,
            None,
            None,
            None,
            None,
            BTreeMap::new(),
            None,
            None,
            None,
            None,
            None,
            None,
        ),
    };
    let central_config = central_workflow_front_matter.is_some();
    let config_root = config_path
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(cwd.as_path());
    let central_project_set = central_repository_routing
        .as_ref()
        .is_some_and(|routing| matches!(routing.mode, RepositoryRoutingMode::ProjectSet));
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|path| super::super::resolve_path(config_root, path))
        .unwrap_or_else(|| {
            if central_project_set {
                config_root.to_path_buf()
            } else {
                cwd.clone()
            }
        });
    let central_instruction_configured = central_repository_instruction_path.is_some();
    let workflow_path =
        central_repository_instruction_path.unwrap_or_else(|| target_repo.join("WORKFLOW.md"));
    let workflow = if central_project_set && central_config && !central_instruction_configured {
        WorkflowDefinition::parse(crate::opensymphony_workflow::DEFAULT_PROMPT_TEMPLATE)
            .expect("default central project-set workflow should parse")
    } else {
        WorkflowDefinition::load_from_path(&workflow_path).map_err(|source| {
            RunCommandError::LoadWorkflow {
                path: workflow_path.clone(),
                source,
            }
        })?
    };
    let workflow = central_workflow_front_matter
        .map(|front_matter| {
            // Central config owns orchestration fields, while repository-local
            // codex/logging/extensions remain implementation guidance. Merge
            // those allowed fields instead of dropping them when the central
            // front matter replaces the legacy orchestration values.
            WorkflowDefinition {
                front_matter: merge_repository_local_front_matter(
                    front_matter,
                    &workflow.front_matter,
                ),
                prompt_template: workflow.prompt_template.clone(),
            }
        })
        .unwrap_or(workflow);
    let mut workflow = workflow
        .resolve_with_process_env(&target_repo)
        .map_err(|source| RunCommandError::ResolveWorkflow {
            path: workflow_path.clone(),
            source,
        })?;
    if let Some(workspace_root) = central_workspace_root {
        workflow.config.workspace.root = workspace_root;
    }
    workflow.config.routing.dry_run = args.dry_run;
    let bind_value = config
        .control_plane
        .bind
        .as_deref()
        .unwrap_or(DEFAULT_CONTROL_PLANE_BIND);
    let bind = bind_value
        .parse()
        .map_err(|source| RunCommandError::InvalidBind {
            value: bind_value.to_string(),
            source,
        })?;
    let tool_dir = config
        .openhands
        .tool_dir
        .as_deref()
        .map(|path| super::super::resolve_path(config_root, path));
    let openhands_conversation_store = tool_dir
        .as_ref()
        .map(|tool_dir| OpenHandsConversationStorePaths::for_tool_dir(tool_dir, &target_repo))
        .transpose()?;
    let memory_config_exists = target_repo
        .join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE)
        .is_file();
    let auto_capture = config.memory.auto_capture.unwrap_or(true);
    let serve_memory = config.memory.serve.unwrap_or(memory_config_exists);
    let memory_server = if serve_memory {
        let memory_bind_value = config
            .memory
            .bind
            .as_deref()
            .unwrap_or(DEFAULT_MEMORY_SERVER_BIND);
        let memory_bind =
            memory_bind_value
                .parse()
                .map_err(|source| RunCommandError::InvalidBind {
                    value: memory_bind_value.to_string(),
                    source,
                })?;
        let memory_token_env = config
            .memory
            .token_env
            .as_deref()
            .unwrap_or(DEFAULT_MEMORY_TOKEN_ENV);
        let memory_token = env::var(memory_token_env)
            .ok()
            .and_then(|value| non_empty(&value));
        Some(RunMemoryServerConfig {
            bind: memory_bind,
            token: memory_token,
        })
    } else {
        None
    };
    let memory = RunMemoryConfig {
        auto_capture,
        auto_archive: config.memory.auto_archive.unwrap_or(false),
        server: memory_server,
    };
    validate_memory_bootstrap(
        &target_repo,
        &memory,
        central_memory_catalog_root.as_deref(),
    )?;

    Ok(RunRuntimeConfig {
        config_path,
        central_config,
        config_generation,
        target_repo,
        workflow_path,
        workflow,
        bind,
        tool_dir,
        openhands_conversation_store,
        retry_max_attempts,
        repository_routing: central_repository_routing,
        repository_checkouts: central_repository_checkouts,
        state_root: central_state_root,
        memory_catalog_root: central_memory_catalog_root,
        memory_sources: central_memory_sources,
        project_set_id: central_project_set_id,
        retain_failed: central_retain_failed.unwrap_or(true),
        preserve_terminal_workspaces: central_preserve_terminal_workspaces.unwrap_or(true),
        memory,
    })
}

pub(crate) fn select_config_path(cwd: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(resolve_relative_to(cwd, path));
    }

    if let Some(home) = super::super::open_user_home_dir() {
        let candidate = home.join(DEFAULT_USER_CONFIG_DIR).join(DEFAULT_CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let candidate = cwd.join(DEFAULT_CONFIG_FILE);
    candidate.is_file().then_some(candidate)
}

const CENTRAL_CONFIG_KEYS: &[&str] = &[
    "instance",
    "routing",
    "tracker_profiles",
    "project_sets",
    "linear_projects",
    "repositories",
    "credentials",
    "review_profiles",
    "workspace",
    "scheduler",
    "hooks",
    "integration",
    "memory_catalog",
    "compatibility",
];

const VERSIONED_CENTRAL_SHARED_KEYS: &[&str] = &["control_plane", "openhands"];

fn has_central_top_level_key(raw: &str) -> bool {
    raw.lines().any(|line| {
        if line.starts_with(char::is_whitespace) {
            return false;
        }
        let Some((key, _)) = line.split_once(':') else {
            return false;
        };
        CENTRAL_CONFIG_KEYS
            .iter()
            .any(|candidate| key.trim() == *candidate)
    })
}

fn has_central_memory_catalog_key(raw: &str) -> bool {
    let mut memory_indent = None;
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        if indent == 0 {
            memory_indent = (key.trim() == "memory").then_some(indent);
            if key.trim() == "memory" && value.contains("catalog_root") {
                return true;
            }
            continue;
        }
        if let Some(parent_indent) = memory_indent {
            if indent <= parent_indent {
                memory_indent = None;
            } else if key.trim() == "catalog_root" {
                return true;
            }
        }
    }
    false
}

fn has_schema_versioned_central_shared_key(raw: &str) -> bool {
    let has_schema_version = raw.lines().any(|line| {
        if line.starts_with(char::is_whitespace) {
            return false;
        }
        line.split_once(':')
            .is_some_and(|(key, _)| key.trim() == "schema_version")
    });
    has_schema_version
        && raw.lines().any(|line| {
            if line.starts_with(char::is_whitespace) {
                return false;
            }
            line.split_once(':').is_some_and(|(key, _)| {
                VERSIONED_CENTRAL_SHARED_KEYS
                    .iter()
                    .any(|candidate| key.trim() == *candidate)
            })
        })
}

pub fn looks_like_central_config(raw: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return has_central_top_level_key(raw)
            || has_central_memory_catalog_key(raw)
            || has_schema_versioned_central_shared_key(raw);
    };
    let Some(mapping) = value.as_mapping() else {
        return false;
    };
    CENTRAL_CONFIG_KEYS
        .iter()
        .any(|key| mapping.contains_key(serde_yaml::Value::String((*key).to_owned())))
        || mapping.contains_key(serde_yaml::Value::String("schema_version".to_owned()))
            && VERSIONED_CENTRAL_SHARED_KEYS
                .iter()
                .any(|key| mapping.contains_key(serde_yaml::Value::String((*key).to_owned())))
        || mapping
            .get(serde_yaml::Value::String("memory".to_owned()))
            .and_then(serde_yaml::Value::as_mapping)
            .is_some_and(|memory| {
                memory.contains_key(serde_yaml::Value::String("catalog_root".to_owned()))
            })
}

fn parse_legacy_run_config(path: &Path, raw: &str) -> Result<RunConfigFile, RunCommandError> {
    let config = serde_yaml::from_str::<RunConfigFile>(raw).map_err(|source| {
        RunCommandError::ParseConfig {
            path: path.to_path_buf(),
            source,
        }
    })?;
    resolve_run_config(path, config)
}

pub async fn load_central_config(path: &Path) -> Result<ResolvedCentralConfig, CentralConfigError> {
    let path = absolute_config_path(path)?;
    let raw = fs::read_to_string(&path)
        .await
        .map_err(|source| CentralConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    resolve_central_config(&path, &raw)
}

pub(crate) fn validate_central_config_text(
    path: &Path,
    raw: &str,
) -> Result<ResolvedCentralConfig, CentralConfigError> {
    resolve_central_config(path, raw)
}

fn resolve_central_config(
    path: &Path,
    raw: &str,
) -> Result<ResolvedCentralConfig, CentralConfigError> {
    let path = absolute_config_path(path)?;
    let config = serde_yaml::from_str::<CentralConfigFile>(raw).map_err(|source| {
        CentralConfigError::Parse {
            path: path.clone(),
            source,
        }
    })?;
    if config.schema_version != 1 {
        return Err(CentralConfigError::UnsupportedSchema);
    }
    if config
        .openhands
        .front_matter
        .as_ref()
        .is_some_and(openhands_front_matter_has_literal_secret)
    {
        return Err(CentralConfigError::LiteralSecret);
    }
    for (field, value) in [
        (
            "openhands.transport_base_url",
            config.openhands.transport_base_url.as_deref(),
        ),
        (
            "openhands.front_matter.transport.base_url",
            config
                .openhands
                .front_matter
                .as_ref()
                .and_then(|front_matter| front_matter.transport.base_url.as_deref()),
        ),
    ] {
        if let Some(value) = value {
            validate_openhands_transport_url(value, field)?;
        }
    }
    if [
        config.hooks.after_create.as_deref(),
        config.hooks.before_run.as_deref(),
        config.hooks.after_run.as_deref(),
        config.hooks.before_remove.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(crate::opensymphony_cli::migration::hook_has_literal_secret)
    {
        return Err(CentralConfigError::LiteralSecret);
    }
    for (field, value) in [
        (
            "memory.token_env",
            config
                .memory
                .as_ref()
                .and_then(|memory| memory.token_env.as_deref()),
        ),
        (
            "openhands.transport_session_api_key_env",
            config.openhands.transport_session_api_key_env.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            validate_central_env_name(value).map_err(|_| CentralConfigError::InvalidReference {
                field: field.to_owned(),
            })?;
        }
    }
    if let Some(front_matter) = config.openhands.front_matter.as_ref() {
        let value = serde_yaml::to_value(front_matter).map_err(|_| {
            CentralConfigError::InvalidReference {
                field: "openhands.front_matter".to_owned(),
            }
        })?;
        validate_openhands_env_references(&value, "openhands.front_matter")?;
    }

    let config_root = path.parent().unwrap_or_else(|| Path::new("."));
    let instance_id = required_literal(&config.instance.id, "instance.id")?;
    for (field, value) in [
        ("routing.harness_env", config.routing.harness_env.as_deref()),
        ("routing.model_env", config.routing.model_env.as_deref()),
        (
            "routing.model_profile_env",
            config.routing.model_profile_env.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            validate_central_env_name(value).map_err(|_| CentralConfigError::InvalidReference {
                field: field.to_owned(),
            })?;
        }
    }
    let state_root = resolve_central_path(
        config_root,
        &config.instance.state_root,
        "instance.state_root",
    )?;
    let workspace_root = config
        .workspace
        .as_ref()
        .ok_or_else(|| CentralConfigError::InvalidReference {
            field: "workspace.root".to_owned(),
        })
        .and_then(|workspace| {
            resolve_central_path(config_root, &workspace.root, "workspace.root")
        })?;
    let retain_failed = config
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.retain_failed)
        .unwrap_or(true);
    ensure_non_overlapping(&state_root, &workspace_root)?;
    if let Some(workspace) = config.workspace.as_ref() {
        let _ = (
            workspace.retain_failed,
            workspace.cleanup_after_parent_finalization,
        );
    }
    let memory_catalog_root = if let Some(memory) = config.memory.as_ref() {
        let memory_root =
            resolve_central_path(config_root, &memory.catalog_root, "memory.catalog_root")?;
        if memory_root == state_root || !is_contained(&state_root, &memory_root) {
            return Err(CentralConfigError::InvalidRoot);
        }
        let _ = (memory.auto_capture, memory.auto_archive, memory.serve);
        Some(memory_root)
    } else {
        None
    };
    let mode = match config.routing.mode.trim() {
        "legacy_single" => CentralRoutingMode::LegacySingle,
        "project_set" => CentralRoutingMode::ProjectSet,
        _ => {
            return Err(CentralConfigError::InvalidReference {
                field: "routing.mode".to_owned(),
            });
        }
    };

    let mut checkout_roots: Vec<PathBuf> = Vec::new();
    for (repository_id, repository) in &config.repositories {
        validate_repository(repository_id, repository, config_root)?;
        if !config.credentials.contains_key(&repository.credential) {
            return Err(CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}.credential"),
            });
        }
        let Some(credential) = config.credentials.get(&repository.credential) else {
            return Err(CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}.credential"),
            });
        };
        if mode == CentralRoutingMode::ProjectSet
            && !matches!(credential.kind.as_str(), "ssh-agent")
            && !(credential.kind == "environment" && credential.variable.is_some())
        {
            return Err(CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}.credential"),
            });
        }
        if !config
            .review_profiles
            .contains_key(&repository.review_profile)
        {
            return Err(CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}.review_profile"),
            });
        }
        if repository.aliases.is_empty() {
            return Err(CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}.aliases"),
            });
        }
        for alias in &repository.aliases {
            required_literal(alias, "repositories.aliases")?;
        }
        if let Some(checkout_path) = repository.checkout_path.as_deref() {
            let checkout_path =
                resolve_central_path(config_root, checkout_path, "repositories.checkout_path")?;
            ensure_non_overlapping(&state_root, &checkout_path)?;
            ensure_non_overlapping(&workspace_root, &checkout_path)?;
            for existing in &checkout_roots {
                ensure_non_overlapping(existing, &checkout_path)?;
            }
            checkout_roots.push(checkout_path);
        }
    }

    for (credential_id, credential) in &config.credentials {
        required_literal(credential_id, "credentials.id")?;
        required_literal(&credential.kind, "credentials.kind")?;
        if let Some(variable) = credential.variable.as_deref() {
            validate_central_env_name(variable).map_err(|_| {
                CentralConfigError::InvalidReference {
                    field: format!("credentials.{credential_id}.variable"),
                }
            })?;
        }
        if let Some(reference) = credential.reference.as_deref() {
            let reference = required_literal(reference, "credentials.reference")?;
            if !is_typed_credential_reference(&reference) {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("credentials.{credential_id}.reference"),
                });
            }
        }
    }
    for (profile_id, profile) in &config.review_profiles {
        required_literal(profile_id, "review_profiles.id")?;
        required_literal(&profile.provider, "review_profiles.provider")?;
        if !config.credentials.contains_key(&profile.credential) {
            return Err(CentralConfigError::InvalidReference {
                field: format!("review_profiles.{profile_id}.credential"),
            });
        }
        if let Some(merge_method) = profile.merge_method.as_deref() {
            let merge_method = required_literal(merge_method, "review_profiles.merge_method")?;
            if !matches!(
                merge_method.to_ascii_lowercase().as_str(),
                "merge" | "squash" | "rebase"
            ) {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("review_profiles.{profile_id}.merge_method"),
                });
            }
        }
        let _ = (profile.required_checks, profile.required_review);
    }
    for (tracker_id, tracker) in &config.tracker_profiles {
        required_literal(tracker_id, "tracker_profiles.id")?;
        if !tracker.provider.eq_ignore_ascii_case("linear") {
            return Err(CentralConfigError::InvalidReference {
                field: format!("tracker_profiles.{tracker_id}.provider"),
            });
        }
        if !config.credentials.contains_key(&tracker.credential) {
            return Err(CentralConfigError::InvalidReference {
                field: format!("tracker_profiles.{tracker_id}.credential"),
            });
        }
        if config
            .credentials
            .get(&tracker.credential)
            .and_then(|credential| credential.variable.as_deref())
            .is_none()
        {
            return Err(CentralConfigError::InvalidReference {
                field: format!("tracker_profiles.{tracker_id}.credential.variable"),
            });
        }
        let _ = (
            &tracker.endpoint,
            &tracker.active_states,
            &tracker.terminal_states,
        );
        if let Some(endpoint) = tracker.endpoint.as_deref() {
            validate_tracker_endpoint(
                endpoint,
                &format!("tracker_profiles.{tracker_id}.endpoint"),
            )?;
        }
    }
    if mode == CentralRoutingMode::ProjectSet {
        reject_checkout_credential_env_reuse(&config)?;
        for (repository_id, repository) in &config.repositories {
            let Some(credential) = config.credentials.get(&repository.credential) else {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("repositories.{repository_id}.credential"),
                });
            };
            if (credential.kind == "ssh-agent" && !is_ssh_clone_transport(&repository.remote.clone))
                || (credential.kind == "environment"
                    && !is_https_clone_transport(&repository.remote.clone))
            {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("repositories.{repository_id}.credential"),
                });
            }
        }
    }
    for (project_id, project) in &config.linear_projects {
        required_literal(project_id, "linear_projects.id")?;
        required_literal(
            &project.provider_project_id,
            "linear_projects.provider_project_id",
        )?;
        if let Some(slug) = project.provider_project_slug.as_deref() {
            required_literal(slug, "linear_projects.provider_project_slug")?;
        }
        for repository in &project.repositories {
            if !config.repositories.contains_key(repository) {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("linear_projects.{project_id}.repositories"),
                });
            }
        }
    }
    for (project_set_id, project_set) in &config.project_sets {
        required_literal(project_set_id, "project_sets.id")?;
        if !config
            .tracker_profiles
            .contains_key(&project_set.tracker_profile)
        {
            return Err(CentralConfigError::InvalidReference {
                field: format!("project_sets.{project_set_id}.tracker_profile"),
            });
        }
        for project in &project_set.projects {
            if !config.linear_projects.contains_key(project) {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("project_sets.{project_set_id}.projects"),
                });
            }
        }
        let mut project_ids = BTreeSet::new();
        let mut project_slugs = BTreeSet::new();
        for project_key in &project_set.projects {
            let project = config
                .linear_projects
                .get(project_key)
                .expect("project references were resolved above");
            let (project_id, project_slug) = project_front_matter_identity(project);
            let duplicate_id = project_id
                .as_deref()
                .is_some_and(|id| !project_ids.insert(id.trim().to_owned()));
            let duplicate_slug = !project_slugs.insert(project_slug.trim().to_ascii_lowercase());
            if duplicate_id || duplicate_slug {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("project_sets.{project_set_id}.projects"),
                });
            }
        }
    }
    let retry_max_attempts = if let Some(scheduler) = config.scheduler.as_ref() {
        if scheduler.max_concurrent_tasks == 0 {
            return Err(CentralConfigError::InvalidReference {
                field: "scheduler".to_owned(),
            });
        }
        if scheduler
            .max_concurrent_agents_by_state
            .iter()
            .any(|(state, limit)| state.trim().is_empty() || *limit == 0)
        {
            return Err(CentralConfigError::InvalidReference {
                field: "scheduler.max_concurrent_agents_by_state".to_owned(),
            });
        }
        scheduler
            .retry
            .max_attempts
            .map(|attempts| {
                u32::try_from(attempts).map_err(|_| CentralConfigError::InvalidReference {
                    field: "scheduler.retry.max_attempts".to_owned(),
                })
            })
            .transpose()?
    } else {
        None
    };
    if let Some(integration) = config.integration.as_ref() {
        required_literal(&integration.policy, "integration.policy")?;
        let _ = integration.use_shared_git_worktrees;
    }
    if config.compatibility.allow_repo_local_config {
        return Err(CentralConfigError::InvalidReference {
            field: "compatibility.allow_repo_local_config".to_owned(),
        });
    }

    let active_project_set = config.routing.active_project_set.as_deref();
    let mut integration_instructions = None;
    let mut active_repositories = BTreeSet::new();
    match mode {
        CentralRoutingMode::LegacySingle => {
            let repository = config
                .routing
                .repository
                .clone()
                .ok_or(CentralConfigError::MissingLegacyRepository)?;
            let repository_entry = config.repositories.get(&repository).ok_or_else(|| {
                CentralConfigError::InvalidReference {
                    field: "routing.repository".to_owned(),
                }
            })?;
            if repository_entry.checkout_path.is_none() {
                return Err(CentralConfigError::MissingLegacyCheckout { repository });
            }
            active_repositories.insert(repository.clone());
            if config.linear_projects.len() == 1
                && !config
                    .linear_projects
                    .values()
                    .next()
                    .expect("length checked")
                    .repositories
                    .iter()
                    .any(|allowed| allowed == &repository)
            {
                return Err(CentralConfigError::InvalidReference {
                    field: "routing.repository".to_owned(),
                });
            }
        }
        CentralRoutingMode::ProjectSet => {
            if config.routing.repository.is_some() {
                return Err(CentralConfigError::InvalidReference {
                    field: "routing.repository".to_owned(),
                });
            }
            let project_set_name =
                active_project_set.ok_or_else(|| CentralConfigError::InvalidReference {
                    field: "routing.active_project_set".to_owned(),
                })?;
            let project_set = config.project_sets.get(project_set_name).ok_or_else(|| {
                CentralConfigError::InvalidReference {
                    field: "routing.active_project_set".to_owned(),
                }
            })?;
            if !config
                .tracker_profiles
                .contains_key(&project_set.tracker_profile)
            {
                return Err(CentralConfigError::InvalidReference {
                    field: format!("project_sets.{project_set_name}.tracker_profile"),
                });
            }
            for project in &project_set.projects {
                let project_entry = config.linear_projects.get(project).ok_or_else(|| {
                    CentralConfigError::InvalidReference {
                        field: format!("project_sets.{project_set_name}.projects"),
                    }
                })?;
                for repository in &project_entry.repositories {
                    if !config.repositories.contains_key(repository) {
                        return Err(CentralConfigError::InvalidReference {
                            field: format!("linear_projects.{project}.repositories"),
                        });
                    }
                    active_repositories.insert(repository.clone());
                }
            }
            if let Some(instructions) = project_set.integration_instructions.as_deref() {
                let integration_path = resolve_relative_config_file(config_root, instructions)?;
                let content = std::fs::read(&integration_path).map_err(|source| {
                    CentralConfigError::ReadIntegration {
                        path: integration_path.clone(),
                        source,
                    }
                })?;
                if config.repositories.values().any(|repository| {
                    repository.checkout_path.as_deref().is_some_and(|checkout| {
                        resolve_central_path(config_root, checkout, "repositories.checkout_path")
                            .is_ok_and(|checkout| is_contained(&checkout, &integration_path))
                    })
                }) {
                    return Err(CentralConfigError::IntegrationInsideCheckout);
                }
                integration_instructions = Some(ResolvedIntegrationInstructions {
                    path: integration_path,
                    content_hash: hash_bytes(&content),
                });
            }
        }
    }
    validate_active_repository_aliases(&config, &active_repositories)?;

    let legacy_repository_instruction_path = if mode == CentralRoutingMode::LegacySingle {
        let repository = config
            .routing
            .repository
            .as_ref()
            .ok_or(CentralConfigError::MissingLegacyRepository)?;
        let repository_entry = config.repositories.get(repository).ok_or_else(|| {
            CentralConfigError::InvalidReference {
                field: "routing.repository".to_owned(),
            }
        })?;
        let checkout_path = repository_entry.checkout_path.as_deref().ok_or_else(|| {
            CentralConfigError::MissingLegacyCheckout {
                repository: repository.clone(),
            }
        })?;
        Some(
            resolve_central_path(config_root, checkout_path, "repositories.checkout_path")?
                .join(&repository_entry.instructions.path),
        )
    } else {
        None
    };

    let mut generation_input = raw.as_bytes().to_vec();
    if let Some(instructions) = integration_instructions.as_ref() {
        generation_input.extend_from_slice(instructions.content_hash.as_bytes());
    }
    let runtime = central_legacy_runtime_config(&config, config_root)?;
    let generation = generation_hash(&generation_input);
    let repository_routing = build_repository_routing(
        &config,
        mode.clone(),
        active_repositories,
        generation.clone(),
    )?;
    let repository_checkouts = build_repository_checkouts(&config)?;
    let memory_sources = resolve_memory_sources(&config, config_root, &repository_routing)?;
    let workflow_front_matter = central_workflow_front_matter(&config, Some(&workspace_root))?;
    let repository_instruction_path = legacy_repository_instruction_path;
    Ok(ResolvedCentralConfig {
        instance_id,
        state_root,
        workspace_root: Some(workspace_root),
        retain_failed,
        memory_catalog_root,
        project_set_id: config.routing.active_project_set.clone(),
        mode,
        repository: config.routing.repository,
        integration_instructions,
        repository_instruction_path,
        generation,
        repository_routing,
        repository_checkouts,
        retry_max_attempts,
        runtime,
        workflow_front_matter,
        memory_sources,
    })
}

fn reject_checkout_credential_env_reuse(
    config: &CentralConfigFile,
) -> Result<(), CentralConfigError> {
    let mut checkout_variables = config
        .repositories
        .values()
        .filter_map(|repository| {
            config
                .credentials
                .get(&repository.credential)
                .and_then(|credential| credential.variable.as_deref())
                .map(str::to_owned)
        })
        .collect::<BTreeSet<_>>();
    if config.repositories.values().any(|repository| {
        config
            .credentials
            .get(&repository.credential)
            .is_some_and(|credential| credential.kind == "ssh-agent")
    }) {
        checkout_variables.insert(SSH_AUTH_SOCK_ENV.to_owned());
    }

    for (tracker_id, tracker) in &config.tracker_profiles {
        let Some(variable) = config
            .credentials
            .get(&tracker.credential)
            .and_then(|credential| credential.variable.as_deref())
        else {
            continue;
        };
        if checkout_variables
            .iter()
            .any(|checkout| environment_variable_names_equal(checkout, variable))
        {
            return Err(CentralConfigError::InvalidReference {
                field: format!("tracker_profiles.{tracker_id}.credential"),
            });
        }
    }

    let mut non_checkout_variables = BTreeMap::new();
    if let Some(variable) = config.openhands.transport_session_api_key_env.as_deref() {
        non_checkout_variables.insert(
            variable.to_owned(),
            "openhands.transport_session_api_key_env",
        );
    }
    for (field, variable) in [
        (
            "routing.harness_env",
            config
                .routing
                .harness_env
                .as_deref()
                .unwrap_or(DEFAULT_ROUTING_HARNESS_ENV),
        ),
        (
            "routing.model_env",
            config
                .routing
                .model_env
                .as_deref()
                .unwrap_or(DEFAULT_ROUTING_MODEL_ENV),
        ),
        (
            "routing.model_profile_env",
            config
                .routing
                .model_profile_env
                .as_deref()
                .unwrap_or(DEFAULT_ROUTING_MODEL_PROFILE_ENV),
        ),
    ] {
        non_checkout_variables.insert(variable.to_owned(), field);
    }
    for variable in ["LLM_API_KEY", "LLM_BASE_URL"] {
        non_checkout_variables.insert(variable.to_owned(), "openhands.implicit_llm_env");
    }
    if let Some(variable) = config.memory.as_ref().and_then(|memory| {
        memory
            .token_env
            .as_deref()
            .or_else(|| memory.serve.then_some(DEFAULT_MEMORY_TOKEN_ENV))
    }) {
        non_checkout_variables.insert(variable.to_owned(), "memory.token_env");
    }
    if config.memory.as_ref().is_some_and(|memory| memory.serve) {
        for variable in [
            "OPENSYMPHONY_MEMORY_ENDPOINT",
            "OPENSYMPHONY_MEMORY_PROJECT",
            "OPENSYMPHONY_MEMORY_EXECUTION_REPO",
            DEFAULT_MEMORY_TOKEN_ENV,
        ] {
            non_checkout_variables
                .entry(variable.to_owned())
                .or_insert("memory.runtime");
        }
    }
    for variable in ["LINEAR_CLIENT_ID", "LINEAR_CLIENT_SECRET"] {
        non_checkout_variables.insert(variable.to_owned(), "linear.oauth_client_credentials");
    }
    non_checkout_variables.insert(
        "OPENSYMPHONY_CODEX_BIN".to_owned(),
        "runtime.codex_binary_env",
    );
    if let Some(front_matter) = config.openhands.front_matter.as_ref() {
        let value = serde_yaml::to_value(front_matter).map_err(|_| {
            CentralConfigError::InvalidReference {
                field: "openhands.front_matter".to_owned(),
            }
        })?;
        collect_central_env_references(
            &value,
            "openhands.front_matter",
            &mut non_checkout_variables,
        );
    }
    if let Some((_, field)) = non_checkout_variables.iter().find(|(variable, _)| {
        checkout_variables
            .iter()
            .any(|checkout| environment_variable_names_equal(checkout, variable))
    }) {
        return Err(CentralConfigError::InvalidReference {
            field: (*field).to_owned(),
        });
    }
    Ok(())
}

fn collect_central_env_references(
    value: &serde_yaml::Value,
    path: &str,
    references: &mut BTreeMap<String, &'static str>,
) {
    match value {
        serde_yaml::Value::Mapping(mapping) => {
            for (key, value) in mapping {
                let Some(key) = key.as_str() else {
                    continue;
                };
                let child_path = format!("{path}.{key}");
                if normalize_secret_field_name(key).ends_with("_env")
                    && let Some(variable) = value.as_str()
                {
                    references.insert(variable.to_owned(), "openhands.front_matter");
                }
                collect_central_env_references(value, &child_path, references);
            }
        }
        serde_yaml::Value::Sequence(sequence) => {
            for value in sequence {
                collect_central_env_references(value, path, references);
            }
        }
        _ => {}
    }
}

fn build_repository_checkouts(
    config: &CentralConfigFile,
) -> Result<BTreeMap<String, CheckoutRepository>, CentralConfigError> {
    let mut checkouts = BTreeMap::new();
    let policy_generation =
        generation_hash(&serde_json::to_vec(&config.scheduler).map_err(|_| {
            CentralConfigError::InvalidReference {
                field: "scheduler".to_owned(),
            }
        })?);
    for (repository_id, repository) in &config.repositories {
        let identity = CanonicalRepositoryId::from_remote(
            &repository.remote.provider,
            repository.remote.provider_id.as_deref(),
            &repository.remote.locator,
        )
        .map_err(|_| CentralConfigError::InvalidReference {
            field: "repositories.remote".to_owned(),
        })?;
        let credential_env = config
            .credentials
            .get(&repository.credential)
            .and_then(|credential| credential.variable.clone());
        let review_profile = config
            .review_profiles
            .get(&repository.review_profile)
            .ok_or_else(|| CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}.review_profile"),
            })?;
        let review_policy_generation =
            generation_hash(&serde_json::to_vec(review_profile).map_err(|_| {
                CentralConfigError::InvalidReference {
                    field: format!("review_profiles.{}", repository.review_profile),
                }
            })?);
        let review_credential_env = config
            .credentials
            .get(&review_profile.credential)
            .and_then(|credential| credential.variable.clone());
        let checkout = CheckoutRepository {
            provider: repository.remote.provider.clone(),
            provider_id: repository.remote.provider_id.clone(),
            remote_locator: repository.remote.locator.clone(),
            remote: repository.remote.clone.clone(),
            target_branch: repository.target_branch.clone(),
            credential_kind: config
                .credentials
                .get(&repository.credential)
                .map(|credential| credential.kind.clone())
                .unwrap_or_default(),
            credential_reference: config
                .credentials
                .get(&repository.credential)
                .and_then(|credential| credential.reference.clone()),
            credential_env,
            review_credential_env,
            instructions_path: PathBuf::from(&repository.instructions.path),
            policy_generation: policy_generation.clone(),
            review_profile: repository.review_profile.clone(),
            review_provider: review_profile.provider.clone(),
            review_policy_generation,
            required_checks: review_profile.required_checks,
            required_review: review_profile.required_review,
            merge_method: review_profile.merge_method.clone(),
        };
        if checkouts
            .insert(identity.to_string(), checkout.clone())
            .is_some_and(|existing| existing != checkout)
        {
            return Err(CentralConfigError::DuplicateRepositoryIdentity {
                identity: identity.to_string(),
            });
        }
    }
    Ok(checkouts)
}

fn resolve_memory_sources(
    config: &CentralConfigFile,
    config_root: &Path,
    repository_routing: &RepositoryRouting,
) -> Result<BTreeMap<String, ResolvedMemorySource>, CentralConfigError> {
    let active_repository_ids = match repository_routing.mode {
        RepositoryRoutingMode::LegacySingle => repository_routing
            .legacy_repository
            .as_deref()
            .and_then(|alias| repository_routing.inventory.get(alias))
            .map(|entry| BTreeSet::from([entry.identity.id.clone()]))
            .unwrap_or_default(),
        RepositoryRoutingMode::ProjectSet => repository_routing
            .project_repositories
            .values()
            .flat_map(|repositories| repositories.iter().cloned())
            .collect::<BTreeSet<_>>(),
    };
    let mut sources = BTreeMap::new();
    for (repository_key, repository) in &config.repositories {
        let repository_id = CanonicalRepositoryId::from_remote(
            &repository.remote.provider,
            repository.remote.provider_id.as_deref(),
            &repository.remote.locator,
        )
        .map_err(|_| CentralConfigError::InvalidReference {
            field: "repositories.remote".to_owned(),
        })?;
        let repository_id = repository_id.to_string();
        if !active_repository_ids
            .iter()
            .any(|active| active.to_string() == repository_id)
        {
            continue;
        }
        let Some(checkout_path) = repository.checkout_path.as_deref() else {
            if repository_routing.mode == RepositoryRoutingMode::ProjectSet {
                // Project-set repositories are cloned into verified generation
                // workspaces at dispatch time. A static checkout is optional,
                // so it must not block the dynamic clone path.
                continue;
            }
            return Err(CentralConfigError::MissingLegacyCheckout {
                repository: repository_key.clone(),
            });
        };
        let resolved = ResolvedMemorySource {
            repository_id: repository_id.clone(),
            remote_locator: repository.remote.locator.clone(),
            checkout_path: resolve_central_path(
                config_root,
                checkout_path,
                "repositories.checkout_path",
            )?,
            project_scope_ids: config
                .linear_projects
                .iter()
                .filter(|(_, project)| project.repositories.contains(repository_key))
                .flat_map(|(project_id, project)| {
                    let project_keys = [
                        project_id.clone(),
                        project.provider_project_id.clone(),
                        project.provider_project_slug.clone().unwrap_or_default(),
                    ];
                    let source_project_keys = repository_routing
                        .project_repositories
                        .iter()
                        .filter_map(|(scope_id, repositories)| {
                            repositories
                                .iter()
                                .any(|candidate| candidate.to_string() == repository_id)
                                .then_some(scope_id)
                        })
                        .collect::<BTreeSet<_>>();
                    if !repository_routing.project_repositories.is_empty()
                        && !project_keys
                            .iter()
                            .any(|key| source_project_keys.contains(key))
                    {
                        return Vec::new();
                    }
                    [
                        project_id.clone(),
                        project.provider_project_id.clone(),
                        project.provider_project_slug.clone().unwrap_or_default(),
                    ]
                    .into_iter()
                    .filter(|scope_id| !scope_id.is_empty())
                    .collect::<Vec<_>>()
                })
                .collect(),
            target_branch: repository.target_branch.clone(),
        };
        if sources.insert(repository_id, resolved).is_some() {
            return Err(CentralConfigError::InvalidReference {
                field: "repositories.remote.canonical_id".to_owned(),
            });
        }
    }
    Ok(sources)
}

fn central_workflow_front_matter(
    config: &CentralConfigFile,
    workspace_root: Option<&Path>,
) -> Result<WorkflowFrontMatter, CentralConfigError> {
    if let (Some(top_level), Some(nested)) = (
        config.openhands.transport_base_url.as_deref(),
        config
            .openhands
            .front_matter
            .as_ref()
            .and_then(|front_matter| front_matter.transport.base_url.as_deref()),
    ) && top_level != nested
    {
        return Err(CentralConfigError::InvalidReference {
            field: "openhands.transport_base_url".to_owned(),
        });
    }
    if let (Some(top_level), Some(nested)) = (
        config.openhands.transport_session_api_key_env.as_deref(),
        config
            .openhands
            .front_matter
            .as_ref()
            .and_then(|front_matter| front_matter.transport.session_api_key_env.as_deref()),
    ) && top_level != nested
    {
        return Err(CentralConfigError::InvalidReference {
            field: "openhands.transport_session_api_key_env".to_owned(),
        });
    }
    let (tracker, project_id, project_slug, project_ids, project_slugs, project_id_slug_fallbacks) =
        match config.routing.mode.trim() {
            "legacy_single" => {
                if config.tracker_profiles.len() != 1 {
                    return Err(CentralConfigError::InvalidReference {
                        field: "routing.repository.tracker_profile".to_owned(),
                    });
                }
                if config.linear_projects.len() != 1 {
                    return Err(CentralConfigError::InvalidReference {
                        field: "routing.repository.linear_project".to_owned(),
                    });
                }
                let project = config
                    .linear_projects
                    .values()
                    .next()
                    .expect("length checked");
                let (project_id, project_slug) = project_front_matter_identity(project);
                (
                    config
                        .tracker_profiles
                        .values()
                        .next()
                        .expect("length checked"),
                    project_id.clone(),
                    project_slug.clone(),
                    project_id.as_ref().map(|id| vec![id.clone()]),
                    vec![project_slug],
                    None,
                )
            }
            "project_set" => {
                let project_set_id =
                    config
                        .routing
                        .active_project_set
                        .as_deref()
                        .ok_or_else(|| CentralConfigError::InvalidReference {
                            field: "routing.active_project_set".to_owned(),
                        })?;
                let project_set = config.project_sets.get(project_set_id).ok_or_else(|| {
                    CentralConfigError::InvalidReference {
                        field: "routing.active_project_set".to_owned(),
                    }
                })?;
                let tracker = config
                    .tracker_profiles
                    .get(&project_set.tracker_profile)
                    .ok_or_else(|| CentralConfigError::InvalidReference {
                        field: format!("project_sets.{project_set_id}.tracker_profile"),
                    })?;
                let first_project_id = project_set.projects.first().ok_or_else(|| {
                    CentralConfigError::InvalidReference {
                        field: format!("project_sets.{project_set_id}.projects"),
                    }
                })?;
                let mut project_id = None;
                let mut project_ids = Vec::with_capacity(project_set.projects.len());
                let mut project_slugs = Vec::with_capacity(project_set.projects.len());
                let mut project_id_slug_fallbacks = Vec::with_capacity(project_set.projects.len());
                for project_key in &project_set.projects {
                    let project = config.linear_projects.get(project_key).ok_or_else(|| {
                        CentralConfigError::InvalidReference {
                            field: format!("project_sets.{project_set_id}.projects"),
                        }
                    })?;
                    let (candidate_id, project_slug) = project_front_matter_identity(project);
                    project_ids.push(candidate_id.clone().unwrap_or_else(|| project_slug.clone()));
                    project_id_slug_fallbacks.push(candidate_id.is_none());
                    if project_key == first_project_id {
                        project_id = candidate_id;
                    }
                    project_slugs.push(project_slug);
                }
                let first_project = config
                    .linear_projects
                    .get(first_project_id)
                    .expect("first project was resolved above");
                let (_, project_slug) = project_front_matter_identity(first_project);
                let project_ids = (!project_id_slug_fallbacks.iter().all(|fallback| *fallback))
                    .then_some(project_ids);
                let project_id_slug_fallbacks =
                    project_ids.as_ref().map(|_| project_id_slug_fallbacks);
                (
                    tracker,
                    project_id,
                    project_slug,
                    project_ids,
                    project_slugs,
                    project_id_slug_fallbacks,
                )
            }
            _ => {
                return Err(CentralConfigError::InvalidReference {
                    field: "routing.mode".to_owned(),
                });
            }
        };
    let api_key = config
        .credentials
        .get(&tracker.credential)
        .and_then(|credential| credential.variable.as_deref())
        .map(|variable| format!("${{{variable}}}"));
    let front_matter = WorkflowFrontMatter {
        tracker: TrackerFrontMatter {
            kind: Some(tracker.provider.clone()),
            endpoint: tracker.endpoint.clone(),
            api_key,
            project_id,
            project_slug: Some(project_slug),
            project_ids,
            project_slugs: Some(project_slugs),
            project_id_slug_fallbacks,
            active_states: (!tracker.active_states.is_empty())
                .then(|| tracker.active_states.clone()),
            terminal_states: (!tracker.terminal_states.is_empty())
                .then(|| tracker.terminal_states.clone()),
        },
        polling: PollingFrontMatter {
            interval_ms: config
                .scheduler
                .as_ref()
                .and_then(|scheduler| scheduler.poll_interval_ms)
                .map(|value| central_integer(value, "scheduler.poll_interval_ms"))
                .transpose()?,
        },
        workspace: WorkspaceFrontMatter {
            root: workspace_root.map(|root| root.display().to_string()),
        },
        hooks: HooksFrontMatter {
            after_create: config.hooks.after_create.clone(),
            before_run: config.hooks.before_run.clone(),
            after_run: config.hooks.after_run.clone(),
            before_remove: config.hooks.before_remove.clone(),
            timeout_ms: config
                .hooks
                .timeout_ms
                .map(|value| central_integer(value, "hooks.timeout_ms"))
                .transpose()?,
        },
        agent: AgentFrontMatter {
            max_concurrent_agents: config
                .scheduler
                .as_ref()
                .map(|scheduler| {
                    central_integer(
                        scheduler.max_concurrent_tasks,
                        "scheduler.max_concurrent_tasks",
                    )
                })
                .transpose()?,
            max_turns: config
                .scheduler
                .as_ref()
                .and_then(|scheduler| scheduler.max_turns)
                .map(|value| central_integer(value, "scheduler.max_turns"))
                .transpose()?,
            max_retry_backoff_ms: config
                .scheduler
                .as_ref()
                .and_then(|scheduler| scheduler.max_retry_backoff_ms)
                .map(|value| central_integer(value, "scheduler.max_retry_backoff_ms"))
                .transpose()?,
            stall_timeout_ms: config
                .scheduler
                .as_ref()
                .and_then(|scheduler| scheduler.stall_timeout_ms)
                .map(|value| central_integer(value, "scheduler.stall_timeout_ms"))
                .transpose()?,
            max_concurrent_agents_by_state: config
                .scheduler
                .as_ref()
                .map(|scheduler| {
                    scheduler
                        .max_concurrent_agents_by_state
                        .iter()
                        .map(|(state, limit)| {
                            central_integer(*limit, "scheduler.max_concurrent_agents_by_state")
                                .map(|value| (state.clone(), value))
                        })
                        .collect::<Result<BTreeMap<_, _>, _>>()
                })
                .transpose()?,
        },
        routing: RoutingFrontMatter {
            harness: config.routing.harness.clone(),
            model: config.routing.model.clone(),
            model_profile: config.routing.model_profile.clone(),
            harness_env: config.routing.harness_env.clone(),
            model_env: config.routing.model_env.clone(),
            model_profile_env: config.routing.model_profile_env.clone(),
        },
        openhands: {
            let mut openhands = config.openhands.front_matter.clone().unwrap_or_default();
            if config.openhands.transport_base_url.is_some() {
                openhands.transport.base_url = config.openhands.transport_base_url.clone();
            }
            if config.openhands.transport_session_api_key_env.is_some() {
                openhands.transport.session_api_key_env =
                    config.openhands.transport_session_api_key_env.clone();
            }
            openhands
        },
        ..WorkflowFrontMatter::default()
    };
    Ok(front_matter)
}

fn merge_repository_local_front_matter(
    mut central: WorkflowFrontMatter,
    local: &WorkflowFrontMatter,
) -> WorkflowFrontMatter {
    central.codex = local.codex.clone();
    central.logging = local.logging.clone();
    central.extensions = local.extensions.clone();
    central
}

fn project_front_matter_identity(project: &CentralLinearProjectFile) -> (Option<String>, String) {
    match project.provider_project_slug.clone() {
        Some(slug) if slug == project.provider_project_id => (None, slug),
        Some(slug) => (Some(project.provider_project_id.clone()), slug),
        None => (
            Some(project.provider_project_id.clone()),
            project.provider_project_id.clone(),
        ),
    }
}

fn central_integer(value: u64, field: &'static str) -> Result<IntegerLike, CentralConfigError> {
    i64::try_from(value).map(IntegerLike::Integer).map_err(|_| {
        CentralConfigError::InvalidReference {
            field: field.to_owned(),
        }
    })
}

fn absolute_config_path(path: &Path) -> Result<PathBuf, CentralConfigError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|source| CentralConfigError::Read {
            path: path.to_path_buf(),
            source,
        })
}

fn central_legacy_runtime_config(
    config: &CentralConfigFile,
    config_root: &Path,
) -> Result<RunConfigFile, CentralConfigError> {
    let target_repo = config
        .routing
        .repository
        .as_deref()
        .and_then(|repository| config.repositories.get(repository))
        .and_then(|repository| repository.checkout_path.as_deref())
        .map(|path| resolve_central_path(config_root, path, "repositories.checkout_path"))
        .transpose()?
        .map(|path| path.display().to_string());
    let tool_dir = config
        .openhands
        .tool_dir
        .as_deref()
        .map(|path| expand_central_value(config_root, path))
        .transpose()?
        .map(|path| path.display().to_string());
    let memory = config
        .memory
        .as_ref()
        .map(|memory| RunMemoryConfigFile {
            auto_capture: Some(memory.auto_capture),
            auto_archive: Some(memory.auto_archive),
            serve: Some(memory.serve),
            bind: memory.bind.clone(),
            token_env: memory.token_env.clone(),
        })
        .unwrap_or_default();
    Ok(RunConfigFile {
        target_repo,
        control_plane: ControlPlaneConfigFile {
            bind: config.control_plane.bind.clone(),
        },
        openhands: RunOpenHandsConfigFile { tool_dir },
        memory,
    })
}

fn build_repository_routing(
    config: &CentralConfigFile,
    mode: CentralRoutingMode,
    active_repositories: BTreeSet<String>,
    config_generation: String,
) -> Result<RepositoryRouting, CentralConfigError> {
    let mut inventory = BTreeMap::new();
    let mut identities = BTreeMap::new();
    for repository_id in active_repositories {
        let repository = config.repositories.get(&repository_id).ok_or_else(|| {
            CentralConfigError::InvalidReference {
                field: format!("repositories.{repository_id}"),
            }
        })?;
        let id = CanonicalRepositoryId::from_remote(
            &repository.remote.provider,
            repository.remote.provider_id.as_deref(),
            &repository.remote.locator,
        )
        .map_err(|_| CentralConfigError::InvalidReference {
            field: format!("repositories.{repository_id}.remote"),
        })?;
        let safe_remote_fingerprint = SafeRemoteFingerprint::from_remote(
            &repository.remote.provider,
            repository.remote.provider_id.as_deref(),
            &repository.remote.locator,
        )
        .map_err(|_| CentralConfigError::InvalidReference {
            field: format!("repositories.{repository_id}.remote"),
        })?;
        let identity = RepositoryIdentity {
            id,
            safe_remote_fingerprint,
        };
        identities.insert(repository_id.clone(), identity.clone());
        for alias in &repository.aliases {
            let alias = alias.trim().to_owned();
            inventory.insert(
                alias.clone(),
                RepositoryInventoryEntry {
                    alias,
                    identity: identity.clone(),
                },
            );
        }
    }

    let active_project_ids = match mode {
        CentralRoutingMode::LegacySingle => BTreeSet::new(),
        CentralRoutingMode::ProjectSet => config
            .routing
            .active_project_set
            .as_ref()
            .and_then(|name| config.project_sets.get(name))
            .map(|project_set| project_set.projects.iter().cloned().collect())
            .unwrap_or_default(),
    };
    let mut active_projects = active_project_ids.clone();
    let mut project_repositories: BTreeMap<String, BTreeSet<CanonicalRepositoryId>> =
        BTreeMap::new();
    for project_id in &active_project_ids {
        let project = config.linear_projects.get(project_id).ok_or_else(|| {
            CentralConfigError::InvalidReference {
                field: format!("linear_projects.{project_id}"),
            }
        })?;
        let allowed = project
            .repositories
            .iter()
            .filter_map(|repository| {
                identities
                    .get(repository)
                    .map(|identity| identity.id.clone())
            })
            .collect::<BTreeSet<_>>();
        let keys = [
            Some(project_id.as_str()),
            Some(project.provider_project_id.trim()),
            project.provider_project_slug.as_deref().map(str::trim),
        ];
        for key in keys.into_iter().flatten().filter(|key| !key.is_empty()) {
            if project_repositories
                .get(key)
                .is_some_and(|existing| existing != &allowed)
                || project_repositories.iter().any(|(existing_key, existing)| {
                    existing_key != key
                        && existing_key.eq_ignore_ascii_case(key)
                        && existing != &allowed
                })
            {
                return Err(CentralConfigError::AmbiguousProjectRoutingKey {
                    key: key.to_owned(),
                });
            }
            active_projects.insert(key.to_owned());
            project_repositories.insert(key.to_owned(), allowed.clone());
        }
    }

    let legacy_repository = config
        .routing
        .repository
        .as_ref()
        .and_then(|repository| config.repositories.get(repository))
        .and_then(|repository| repository.aliases.first())
        .map(|alias| alias.trim().to_owned());
    let inventory_generation = generation_hash(&serde_json::to_vec(&inventory).map_err(|_| {
        CentralConfigError::InvalidReference {
            field: "repositories".to_owned(),
        }
    })?);

    Ok(RepositoryRouting {
        mode: match mode {
            CentralRoutingMode::LegacySingle => RepositoryRoutingMode::LegacySingle,
            CentralRoutingMode::ProjectSet => RepositoryRoutingMode::ProjectSet,
        },
        inventory,
        project_repositories,
        active_projects,
        legacy_repository,
        config_generation,
        inventory_generation,
    })
}

fn validate_repository(
    repository_id: &str,
    repository: &CentralRepositoryFile,
    config_root: &Path,
) -> Result<(), CentralConfigError> {
    required_literal(repository_id, "repositories.id")?;
    required_literal(&repository.remote.provider, "repositories.remote.provider")?;
    if let Some(provider_id) = repository.remote.provider_id.as_deref() {
        required_literal(provider_id, "repositories.remote.provider_id")?;
    }
    required_literal(&repository.remote.locator, "repositories.remote.locator")?;
    required_literal(&repository.remote.clone, "repositories.remote.clone")?;
    required_literal(&repository.target_branch, "repositories.target_branch")?;
    required_literal(&repository.credential, "repositories.credential")?;
    required_literal(&repository.review_profile, "repositories.review_profile")?;
    validate_remote_clone(&repository.remote.locator)?;
    validate_remote_clone(&repository.remote.clone)?;
    if repository.instructions.path.is_empty()
        || Path::new(&repository.instructions.path).is_absolute()
        || Path::new(&repository.instructions.path)
            .components()
            .any(|component| component == std::path::Component::ParentDir)
    {
        return Err(CentralConfigError::InvalidInstructionPath);
    }
    if let Some(checkout_path) = repository.checkout_path.as_deref() {
        let checkout_path =
            resolve_central_path(config_root, checkout_path, "repositories.checkout_path")?;
        let instruction_path = checkout_path.join(&repository.instructions.path);
        if !is_contained(&checkout_path, &instruction_path) {
            return Err(CentralConfigError::InvalidInstructionPath);
        }
        if instruction_path.exists() {
            let canonical_checkout = std::fs::canonicalize(&checkout_path)
                .map_err(|_| CentralConfigError::InvalidInstructionPath)?;
            let canonical_instruction = std::fs::canonicalize(&instruction_path)
                .map_err(|_| CentralConfigError::InvalidInstructionPath)?;
            if !canonical_instruction.starts_with(canonical_checkout) {
                return Err(CentralConfigError::InvalidInstructionPath);
            }
        }
    }
    Ok(())
}

fn validate_remote_clone(value: &str) -> Result<(), CentralConfigError> {
    if let Ok(url) = Url::parse(value) {
        if ((!url.username().is_empty() && !url.scheme().eq_ignore_ascii_case("ssh"))
            || is_credential_shaped_username(url.username()))
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(CentralConfigError::CredentialBearingRemote);
        }
        return Ok(());
    }
    if value.contains('?') || value.contains('#') {
        return Err(CentralConfigError::CredentialBearingRemote);
    }
    if let Some((user, host)) = value.split_once('@')
        && (user.is_empty()
            || user.contains(':')
            || is_credential_shaped_username(user)
            || host.is_empty())
    {
        return Err(CentralConfigError::CredentialBearingRemote);
    }
    Ok(())
}

fn is_ssh_clone_transport(value: &str) -> bool {
    if let Ok(url) = Url::parse(value) {
        return matches!(
            url.scheme().to_ascii_lowercase().as_str(),
            "ssh" | "git+ssh"
        );
    }
    let Some((authority, path)) = value.split_once(':') else {
        return false;
    };
    !authority.is_empty()
        && !path.starts_with('/')
        && !authority
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
}

fn is_https_clone_transport(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| url.scheme().eq_ignore_ascii_case("https"))
}

fn is_credential_shaped_username(username: &str) -> bool {
    let username = username.trim().to_ascii_lowercase();
    [
        "bearer ",
        "ghp_",
        "gho_",
        "ghs_",
        "ghu_",
        "ghr_",
        "github_pat_",
        "glpat-",
        "oauth",
        "pat-",
        "pat_",
        "token",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxr-",
    ]
    .iter()
    .any(|prefix| username.starts_with(prefix))
}

fn validate_tracker_endpoint(value: &str, field: &str) -> Result<(), CentralConfigError> {
    let url = Url::parse(value).map_err(|_| CentralConfigError::InvalidReference {
        field: field.to_owned(),
    })?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CentralConfigError::CredentialBearingRemote);
    }
    if !matches!(url.scheme().to_ascii_lowercase().as_str(), "http" | "https")
        || url.host_str().is_none()
    {
        return Err(CentralConfigError::InvalidReference {
            field: field.to_owned(),
        });
    }
    Ok(())
}

fn validate_openhands_transport_url(value: &str, field: &str) -> Result<(), CentralConfigError> {
    let url = Url::parse(value).map_err(|_| CentralConfigError::InvalidReference {
        field: field.to_owned(),
    })?;
    if !matches!(url.scheme().to_ascii_lowercase().as_str(), "http" | "https")
        || url.host_str().is_none()
    {
        return Err(CentralConfigError::InvalidReference {
            field: field.to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(CentralConfigError::InvalidReference {
            field: field.to_owned(),
        });
    }
    if url.query_pairs().any(|(key, _)| {
        matches!(
            key.to_ascii_lowercase().replace(['-', '_'], "").as_str(),
            "accesstoken" | "apikey" | "authorization" | "sessionapikey" | "token"
        )
    }) {
        return Err(CentralConfigError::InvalidReference {
            field: field.to_owned(),
        });
    }
    Ok(())
}

fn required_literal(value: &str, field: &'static str) -> Result<String, CentralConfigError> {
    let value = value.trim();
    (!value.is_empty())
        .then(|| value.to_owned())
        .ok_or(CentralConfigError::EmptyField { field })
}

fn openhands_front_matter_has_literal_secret(front_matter: &OpenHandsFrontMatter) -> bool {
    let Ok(value) = serde_yaml::to_value(front_matter) else {
        return true;
    };
    openhands_yaml_value_has_literal_secret(&value)
}

fn openhands_yaml_value_has_literal_secret(value: &serde_yaml::Value) -> bool {
    if let Some(mapping) = value.as_mapping() {
        return mapping.iter().any(|(key, value)| {
            let command_secret = key.as_str().is_some_and(|key| {
                normalize_secret_field_name(key) == "command"
                    && openhands_command_has_literal_secret(value)
            });
            let secret_name = key.as_str().is_some_and(openhands_secret_field_name)
                && match value.as_str() {
                    Some(value) => !is_central_credential_reference(value),
                    None => !value.is_null(),
                };
            command_secret || secret_name || openhands_yaml_value_has_literal_secret(value)
        });
    }
    value
        .as_sequence()
        .is_some_and(|values| values.iter().any(openhands_yaml_value_has_literal_secret))
}

fn openhands_command_has_literal_secret(value: &serde_yaml::Value) -> bool {
    let Some(values) = value.as_sequence() else {
        return false;
    };
    let Some(values) = values
        .iter()
        .map(serde_yaml::Value::as_str)
        .collect::<Option<Vec<_>>>()
    else {
        return true;
    };
    crate::opensymphony_cli::migration::hook_has_literal_secret(&values.join(" "))
}

fn validate_openhands_env_references(
    value: &serde_yaml::Value,
    path: &str,
) -> Result<(), CentralConfigError> {
    if let Some(mapping) = value.as_mapping() {
        for (key, value) in mapping {
            let Some(key) = key.as_str() else {
                continue;
            };
            let field = format!("{}.{}", path, key);
            let normalized_key = normalize_secret_field_name(key);
            // OpenHands local-server environment entries are literal values,
            // not names of environment variables selected by `*_env` fields.
            if normalized_key == "env" && path.ends_with(".local_server") {
                continue;
            }
            if normalized_key.ends_with("_env") && !value.is_null() {
                let Some(value) = value.as_str() else {
                    return Err(CentralConfigError::InvalidReference { field });
                };
                validate_central_env_name(value).map_err(|_| {
                    CentralConfigError::InvalidReference {
                        field: field.clone(),
                    }
                })?;
            }
            validate_openhands_env_references(value, &field)?;
        }
    } else if let Some(values) = value.as_sequence() {
        for (index, value) in values.iter().enumerate() {
            validate_openhands_env_references(value, &format!("{}[{}]", path, index))?;
        }
    }
    Ok(())
}

fn openhands_secret_field_name(name: &str) -> bool {
    // OpenHands emits some identity headers with hyphens (for example,
    // `chatgpt-account-id`) even though most serialized config uses
    // underscore-separated keys. Normalize the separator before applying the
    // secret-shaped field rules so both spellings fail closed.
    let name = normalize_secret_field_name(name);
    [
        "access_token",
        "api_key",
        "apikey",
        "authorization",
        "access_key",
        "accesskey",
        "account_id",
        "accountid",
        "account_identifier",
        "account_identity",
        "chatgpt_account_id",
        "credential",
        "password",
        "pat",
        "secret",
        "token",
    ]
    .iter()
    .any(|part| name == *part || name.ends_with(&format!("_{part}")))
}

fn normalize_secret_field_name(name: &str) -> String {
    let characters = name.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(name.len());
    for (index, character) in characters.iter().copied().enumerate() {
        if character == '-' {
            if !normalized.ends_with('_') {
                normalized.push('_');
            }
            continue;
        }
        if character.is_ascii_uppercase() {
            let previous_is_lower_or_digit = characters
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous.is_ascii_lowercase() || previous.is_ascii_digit());
            let previous_is_acronym_boundary = characters
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous.is_ascii_uppercase())
                && characters
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_lowercase());
            if (previous_is_lower_or_digit || previous_is_acronym_boundary)
                && !normalized.ends_with('_')
            {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}

fn validate_active_repository_aliases(
    config: &CentralConfigFile,
    active_repositories: &BTreeSet<String>,
) -> Result<(), CentralConfigError> {
    let mut aliases = BTreeSet::new();
    for repository_id in active_repositories {
        let Some(repository) = config.repositories.get(repository_id) else {
            continue;
        };
        for alias in &repository.aliases {
            let alias = alias.trim();
            if !aliases.insert(alias.to_owned()) {
                return Err(CentralConfigError::DuplicateAlias {
                    alias: alias.to_owned(),
                });
            }
        }
    }
    Ok(())
}

fn is_central_credential_reference(value: &str) -> bool {
    let variable = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'));
    variable.is_some_and(|variable| {
        !variable.is_empty()
            && variable.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            })
            && variable
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_uppercase() || character == '_')
    })
}

fn is_typed_credential_reference(value: &str) -> bool {
    let Some((scheme, locator)) = value.split_once(':') else {
        return false;
    };
    if !matches!(
        scheme,
        "broker" | "codex-cli" | "keychain" | "openhands-auth" | "secret-manager" | "vault"
    ) {
        return false;
    }
    !locator.is_empty()
        && locator.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '/' | '_' | '-' | '@')
        })
        && locator.len() <= 128
}

fn validate_central_env_name(value: &str) -> Result<(), ()> {
    let valid = !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase() || character == '_');
    valid.then_some(()).ok_or(())
}

fn resolve_central_path(
    config_root: &Path,
    value: &str,
    field: &'static str,
) -> Result<PathBuf, CentralConfigError> {
    required_literal(value, field)?;
    let value = expand_central_value(config_root, value)?;
    let path = if value.is_absolute() {
        value
    } else {
        config_root.join(value)
    };
    let path = canonicalize_existing_prefix(&normalize_path(&path))
        .ok_or(CentralConfigError::InvalidRoot)?;
    if !path.is_absolute() {
        return Err(CentralConfigError::InvalidRoot);
    }
    required_literal(&path.display().to_string(), field)?;
    Ok(path)
}

fn resolve_relative_config_file(
    config_root: &Path,
    value: &str,
) -> Result<PathBuf, CentralConfigError> {
    if Path::new(value).is_absolute() {
        return Err(CentralConfigError::InvalidIntegrationPath);
    }
    let path = resolve_central_path(config_root, value, "integration_instructions")?;
    if !is_contained(config_root, &path) {
        return Err(CentralConfigError::InvalidIntegrationPath);
    }
    Ok(path)
}

fn expand_central_value(config_root: &Path, value: &str) -> Result<PathBuf, CentralConfigError> {
    let value =
        super::super::expand_env_tokens(value).map_err(|_| CentralConfigError::InvalidRoot)?;
    let value = if value == "~" {
        super::super::open_user_home_dir().ok_or(CentralConfigError::InvalidRoot)?
    } else if let Some(value) = value.strip_prefix("~/") {
        super::super::open_user_home_dir()
            .ok_or(CentralConfigError::InvalidRoot)?
            .join(value)
    } else {
        PathBuf::from(value)
    };
    Ok(if value.is_absolute() {
        value
    } else {
        config_root.join(value)
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn canonicalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    let mut unresolved = Vec::new();
    let mut existing = path.to_path_buf();
    while !existing.exists() {
        unresolved.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?.to_path_buf();
    }
    let mut resolved = std::fs::canonicalize(existing).ok()?;
    for component in unresolved.iter().rev() {
        resolved.push(component);
    }
    Some(normalize_path(&resolved))
}

fn is_contained(parent: &Path, child: &Path) -> bool {
    let Some(parent) = canonicalize_existing_prefix(parent) else {
        return false;
    };
    let Some(child) = canonicalize_existing_prefix(child) else {
        return false;
    };
    child.strip_prefix(parent).is_ok()
}

fn ensure_non_overlapping(left: &Path, right: &Path) -> Result<(), CentralConfigError> {
    if is_contained(left, right) || is_contained(right, left) {
        return Err(CentralConfigError::OverlappingRoots {
            left: left.to_path_buf(),
            right: right.to_path_buf(),
        });
    }
    Ok(())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn generation_hash(bytes: &[u8]) -> String {
    hash_bytes(bytes)
}

fn validate_memory_bootstrap(
    target_repo: &Path,
    memory: &RunMemoryConfig,
    central_catalog_root: Option<&Path>,
) -> Result<(), RunCommandError> {
    if !memory.auto_capture && memory.server.is_none() {
        return Ok(());
    }
    if central_catalog_root.is_some() {
        return Ok(());
    }
    let path = target_repo.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE);
    if path.is_file() {
        return Ok(());
    }
    Err(RunCommandError::MissingMemoryConfig { path })
}

fn resolve_run_config(
    path: &Path,
    mut config: RunConfigFile,
) -> Result<RunConfigFile, RunCommandError> {
    config.target_repo = config
        .target_repo
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    config.control_plane.bind = config
        .control_plane
        .bind
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    config.openhands.tool_dir = config
        .openhands
        .tool_dir
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    config.memory.bind = config
        .memory
        .bind
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    config.memory.token_env = config
        .memory
        .token_env
        .take()
        .map(|value| expand_run_value(path, value))
        .transpose()?;
    Ok(config)
}

fn expand_run_value(path: &Path, value: String) -> Result<String, RunCommandError> {
    super::super::expand_env_tokens(&value).map_err(|error| RunCommandError::ResolveConfig {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })
}

fn resolve_relative_to(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_bootstrap_is_required_when_auto_capture_is_enabled() {
        let repo = tempfile::tempdir().expect("temp repo should exist");
        let memory = RunMemoryConfig {
            auto_capture: true,
            auto_archive: false,
            server: None,
        };

        let result = validate_memory_bootstrap(repo.path(), &memory, None);

        assert!(matches!(
            result,
            Err(RunCommandError::MissingMemoryConfig { .. })
        ));
    }

    #[test]
    fn memory_bootstrap_is_not_required_when_auto_capture_is_disabled() {
        let repo = tempfile::tempdir().expect("temp repo should exist");
        let memory = RunMemoryConfig {
            auto_capture: false,
            auto_archive: false,
            server: None,
        };

        validate_memory_bootstrap(repo.path(), &memory, None)
            .expect("disabled auto-capture should pass");
    }

    #[test]
    fn memory_bootstrap_accepts_initialized_repo() {
        let repo = tempfile::tempdir().expect("temp repo should exist");
        let path = repo.path().join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE);
        std::fs::create_dir_all(path.parent().expect("memory config should have parent"))
            .expect("memory config parent should be created");
        std::fs::write(&path, "memory_root: .opensymphony/memory\n")
            .expect("memory config should be written");
        let memory = RunMemoryConfig {
            auto_capture: true,
            auto_archive: false,
            server: None,
        };

        validate_memory_bootstrap(repo.path(), &memory, None)
            .expect("memory config should satisfy run");
    }

    #[test]
    fn memory_bootstrap_is_required_when_memory_server_is_enabled() {
        let repo = tempfile::tempdir().expect("temp repo should exist");
        let memory = RunMemoryConfig {
            auto_capture: false,
            auto_archive: false,
            server: Some(RunMemoryServerConfig {
                bind: "127.0.0.1:0".parse().expect("valid bind"),
                token: None,
            }),
        };

        let result = validate_memory_bootstrap(repo.path(), &memory, None);

        assert!(matches!(
            result,
            Err(RunCommandError::MissingMemoryConfig { .. })
        ));
    }

    #[test]
    fn memory_bootstrap_accepts_a_central_catalog_without_repo_config() {
        let repo = tempfile::tempdir().expect("temporary repo should exist");
        let memory = RunMemoryConfig {
            auto_capture: true,
            auto_archive: false,
            server: None,
        };
        validate_memory_bootstrap(
            repo.path(),
            &memory,
            Some(&repo.path().join("state/memory")),
        )
        .expect("central memory catalog should replace repo-local bootstrap");
    }

    fn central_fixture(root: &Path) -> String {
        format!(
            r#"schema_version: 1
instance:
  id: test-instance
  state_root: {root}/state
routing:
  mode: project_set
  active_project_set: suite
  harness_env: TEST_HARNESS
  model_env: TEST_MODEL
  model_profile_env: TEST_MODEL_PROFILE
tracker_profiles:
  linear:
    provider: linear
    endpoint: https://api.linear.app/graphql
    credential: linear-key
    active_states: [Todo]
    terminal_states: [Done]
project_sets:
  suite:
    tracker_profile: linear
    integration_instructions: integration.md
    projects: [core]
linear_projects:
  core:
    provider_project_id: core-project
    repositories: [core-repo]
repositories:
  core-repo:
    aliases: [core]
    remote:
      provider: github
      provider_id: repo-42
      locator: kumanday/OpenSymphony
      clone: git@github.com:kumanday/OpenSymphony.git
    target_branch: develop
    credential: github-ssh
    review_profile: github-standard
    instructions:
      path: AGENTS.md
    checkout_path: {root}/checkout
credentials:
  linear-key:
    kind: environment
    variable: LINEAR_API_KEY
  github-ssh:
    kind: ssh-agent
review_profiles:
  github-standard:
    provider: github
    credential: github-ssh
    required_checks: true
    required_review: true
    merge_method: squash
workspace:
  root: {root}/workspace
memory:
  catalog_root: {root}/state/memory
scheduler:
  max_concurrent_tasks: 2
  max_concurrent_agents_by_state:
    Todo: 1
"#,
            root = root.display()
        )
    }

    #[test]
    fn central_config_resolves_relative_config_path_once() {
        let source = r#"schema_version: 1
instance:
  id: relative-instance
  state_root: state
routing:
  mode: legacy_single
  repository: repo
tracker_profiles:
  linear:
    provider: linear
    credential: linear-key
    active_states: [Todo]
    terminal_states: [Done]
linear_projects:
  project:
    provider_project_id: project
    repositories: [repo]
repositories:
  repo:
    aliases: [repo]
    remote:
      provider: git
      locator: example/repo
      clone: git@github.com:example/repo.git
    target_branch: develop
    credential: git-key
    review_profile: review
    instructions:
      path: AGENTS.md
    checkout_path: checkout
credentials:
  linear-key:
    kind: environment
    variable: LINEAR_API_KEY
  git-key:
    kind: ssh-agent
review_profiles:
  review:
    provider: git
    credential: git-key
workspace:
  root: workspace
memory:
  catalog_root: state/memory
scheduler:
  max_concurrent_tasks: 1
"#;
        let resolved = resolve_central_config(Path::new("configs/config.yaml"), source)
            .expect("relative central config path should resolve");
        assert!(resolved.state_root.ends_with("configs/state"));
        assert!(
            resolved
                .memory_catalog_root
                .as_ref()
                .is_some_and(|root| root.ends_with("configs/state/memory"))
        );
        assert_eq!(
            resolved
                .workspace_root
                .as_ref()
                .expect("workspace root should resolve")
                .file_name()
                .and_then(|name| name.to_str()),
            Some("workspace")
        );
        assert!(
            resolved
                .target_repo()
                .expect("legacy repository should resolve")
                .ends_with("configs/checkout")
        );
        assert!(
            resolved
                .repository_instruction_path
                .as_ref()
                .is_some_and(|path| path.ends_with("configs/checkout/AGENTS.md"))
        );
        assert_eq!(resolved.memory_sources.len(), 1);
        assert!(
            resolved
                .memory_sources
                .contains_key("git:repository:example/repo")
        );
        assert_eq!(resolved.runtime.memory.auto_capture, Some(true));
    }

    #[test]
    fn project_set_memory_sources_allow_missing_checkout_paths() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions");
        let source = central_fixture(root.path()).replace(
            &format!("    checkout_path: {}/checkout\n", root.path().display()),
            "",
        );
        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("project-set repositories may rely on verified generation checkouts");
        assert!(resolved.memory_sources.is_empty());
    }

    #[test]
    fn central_config_resolves_paths_and_hashes_integration_instructions() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let path = root.path().join("config.yaml");
        let resolved = resolve_central_config(&path, &central_fixture(root.path()))
            .expect("central fixture should resolve");

        assert_eq!(resolved.mode, CentralRoutingMode::ProjectSet);
        assert!(
            resolved
                .repository_routing
                .active_projects
                .contains("core-project")
        );
        assert_eq!(
            resolved.repository_routing.inventory["core"]
                .identity
                .id
                .as_str(),
            "github:github.com:repository:repo-42"
        );
        assert!(matches!(
            resolved.repository_routing.resolve(
                &["repo:core".to_string()],
                Some("core-project"),
                None,
                false,
            ),
            crate::opensymphony_domain::RepositoryBindingOutcome::Resolved(_)
        ));
        assert!(matches!(
            resolved.require_legacy_target_repo(),
            Err(CentralConfigError::UnsupportedRoutingMode {
                mode: "project_set"
            })
        ));
        assert!(matches!(
            resolved
                .workflow_front_matter
                .agent
                .max_concurrent_agents_by_state
                .as_ref()
                .and_then(|limits| limits.get("Todo")),
            Some(IntegerLike::Integer(1))
        ));
        assert_eq!(
            resolved
                .workflow_front_matter
                .routing
                .harness_env
                .as_deref(),
            Some("TEST_HARNESS")
        );
        assert_eq!(
            resolved.workflow_front_matter.routing.model_env.as_deref(),
            Some("TEST_MODEL")
        );
        assert_eq!(
            resolved
                .workflow_front_matter
                .routing
                .model_profile_env
                .as_deref(),
            Some("TEST_MODEL_PROFILE")
        );
        assert!(resolved.generation.starts_with("sha256:"));
        assert_eq!(
            resolved
                .integration_instructions
                .as_ref()
                .expect("integration instructions should resolve")
                .path,
            root.path()
                .canonicalize()
                .expect("central root should canonicalize")
                .join("integration.md")
        );
        assert!(
            resolved.repository_instruction_path.is_none(),
            "project-set integration instructions must remain parent-only"
        );
    }

    #[test]
    fn central_config_preserves_all_project_set_tracker_identities() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace("projects: [core]", "projects: [core, other]")
            .replace(
                "linear_projects:\n  core:",
                "linear_projects:\n  other:\n    provider_project_id: other-project\n    repositories: [other-repo]\n  core:",
            )
            .replace(
                "repositories:\n  core-repo:",
                "repositories:\n  other-repo:\n    aliases: [other]\n    remote:\n      provider: github\n      locator: example/other\n      clone: git@github.com:example/other.git\n    checkout_path: {root}/other-checkout\n    target_branch: develop\n    credential: github-ssh\n    review_profile: github-standard\n    instructions:\n      path: AGENTS.md\n  core-repo:",
            )
            .replace(
                "{root}/other-checkout",
                &format!("{}/other-checkout", root.path().display()),
            );

        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("multi-project central fixture should resolve");
        assert_eq!(
            resolved.workflow_front_matter.tracker.project_slugs,
            Some(vec!["core-project".to_owned(), "other-project".to_owned()])
        );
        assert_eq!(
            resolved.workflow_front_matter.tracker.project_ids,
            Some(vec!["core-project".to_owned(), "other-project".to_owned()])
        );

        let migrated_source = source.replace(
            "provider_project_id: other-project\n",
            "provider_project_id: other-project\n    provider_project_slug: other-project\n",
        );
        let migrated = resolve_central_config(&root.path().join("config.yaml"), &migrated_source)
            .expect("mixed migrated project fixture should resolve");
        assert_eq!(
            migrated.workflow_front_matter.tracker.project_id,
            Some("core-project".to_owned())
        );
        assert_eq!(
            migrated.workflow_front_matter.tracker.project_ids,
            Some(vec!["core-project".to_owned(), "other-project".to_owned()])
        );
        assert_eq!(
            migrated
                .workflow_front_matter
                .tracker
                .project_id_slug_fallbacks,
            Some(vec![false, true])
        );
        assert_eq!(
            migrated.workflow_front_matter.tracker.project_slugs,
            Some(vec!["core-project".to_owned(), "other-project".to_owned()])
        );
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_tracker() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent",
            "  github-ssh:\n    kind: environment\n    variable: LINEAR_API_KEY",
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("checkout and tracker credential variables must not overlap");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "tracker_profiles.linear.credential"
        ));
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_routing_selectors() {
        let selectors = [
            ("harness_env: TEST_HARNESS", "routing.harness_env"),
            ("model_env: TEST_MODEL", "routing.model_env"),
            (
                "model_profile_env: TEST_MODEL_PROFILE",
                "routing.model_profile_env",
            ),
        ];

        for (selector, field) in selectors {
            let root = tempfile::tempdir().expect("central config root should exist");
            std::fs::write(root.path().join("integration.md"), "integration\n")
                .expect("integration instructions should be written");
            let source = central_fixture(root.path())
                .replace(
                    "  github-ssh:\n    kind: ssh-agent",
                    "  github-ssh:\n    kind: environment\n    variable: GITHUB_TOKEN",
                )
                .replace(selector, &selector.replacen(':', ": GITHUB_TOKEN", 1));

            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("routing selectors must not reuse checkout credentials");
            assert!(matches!(
                error,
                CentralConfigError::InvalidReference { field: actual }
                    if actual == field
            ));
        }
    }

    #[test]
    fn central_config_rejects_ssh_agent_socket_reuse_by_routing_selector() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace("harness_env: TEST_HARNESS", "harness_env: SSH_AUTH_SOCK");

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("SSH agent socket must not be reused by routing selectors");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "routing.harness_env"
        ));
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_memory_token() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace(
                "  github-ssh:\n    kind: ssh-agent",
                "  github-ssh:\n    kind: environment\n    variable: GITHUB_TOKEN",
            )
            .replace(
                &format!("  catalog_root: {}/state/memory", root.path().display()),
                &format!(
                    "  catalog_root: {}/state/memory\n  token_env: GITHUB_TOKEN",
                    root.path().display()
                ),
            );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("memory token must not reuse checkout credentials");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "memory.token_env"
        ));
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_default_model_selector() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace("  harness_env: TEST_HARNESS\n", "")
            .replace("  model_env: TEST_MODEL\n", "")
            .replace("  model_profile_env: TEST_MODEL_PROFILE\n", "")
            .replace(
                "  github-ssh:\n    kind: ssh-agent",
                "  github-ssh:\n    kind: environment\n    variable: OPENSYMPHONY_MODEL",
            );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("the default model selector must not reuse checkout credentials");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "routing.model_env"
        ));
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_default_memory_token() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace(
                "  github-ssh:\n    kind: ssh-agent",
                "  github-ssh:\n    kind: environment\n    variable: OPENSYMPHONY_MEMORY_TOKEN",
            )
            .replace(
                &format!("  catalog_root: {}/state/memory", root.path().display()),
                &format!(
                    "  catalog_root: {}/state/memory\n  serve: true",
                    root.path().display()
                ),
            );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("the default memory token must not reuse checkout credentials");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "memory.token_env"
        ));
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_linear_oauth() {
        for variable in ["LINEAR_CLIENT_ID", "LINEAR_CLIENT_SECRET"] {
            let root = tempfile::tempdir().expect("central config root should exist");
            std::fs::write(root.path().join("integration.md"), "integration\n")
                .expect("integration instructions should be written");
            let source = central_fixture(root.path()).replace(
                "  github-ssh:\n    kind: ssh-agent",
                &format!("  github-ssh:\n    kind: environment\n    variable: {variable}"),
            );

            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("Linear OAuth variables must not reuse checkout credentials");
            assert!(matches!(
                error,
                CentralConfigError::InvalidReference { field }
                    if field == "linear.oauth_client_credentials"
            ));
        }
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_codex_binary() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent",
            "  github-ssh:\n    kind: environment\n    variable: OPENSYMPHONY_CODEX_BIN",
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("Codex binary selector must not reuse checkout credentials");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "runtime.codex_binary_env"
        ));
    }

    #[test]
    fn central_config_preserves_legacy_slug_fallback_without_project_ids() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "provider_project_id: core-project\n",
            "provider_project_id: core-project\n    provider_project_slug: core-project\n",
        );

        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("legacy central fixture should resolve");
        assert_eq!(resolved.workflow_front_matter.tracker.project_id, None);
        assert_eq!(resolved.workflow_front_matter.tracker.project_ids, None);
        assert_eq!(
            resolved.workflow_front_matter.tracker.project_slugs,
            Some(vec!["core-project".to_owned()])
        );
    }

    #[test]
    fn central_config_normalizes_repository_aliases_before_indexing() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source =
            central_fixture(root.path()).replace("aliases: [core]", "aliases: [' core-renamed ']");

        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("whitespace around an alias should be normalized");
        assert!(
            resolved
                .repository_routing
                .inventory
                .contains_key("core-renamed")
        );
        let base = resolve_central_config(
            &root.path().join("base-config.yaml"),
            &central_fixture(root.path()),
        )
        .expect("base fixture should resolve");
        assert_ne!(
            base.repository_routing.inventory_generation,
            resolved.repository_routing.inventory_generation
        );
        assert!(matches!(
            resolved.repository_routing.resolve(
                &["repo:core-renamed".to_string()],
                Some("core-project"),
                None,
                false,
            ),
            crate::opensymphony_domain::RepositoryBindingOutcome::Resolved(_)
        ));
    }

    #[test]
    fn central_config_tracks_scheduler_and_review_policy_generations_separately() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let base_source = central_fixture(root.path());
        let base = resolve_central_config(&root.path().join("base.yaml"), &base_source)
            .expect("base config should resolve");
        let base_checkout = base
            .repository_checkouts
            .values()
            .next()
            .expect("base checkout should exist");
        assert_eq!(base_checkout.review_profile, "github-standard");
        assert_eq!(base_checkout.review_provider, "github");
        assert_ne!(
            base_checkout.policy_generation,
            base.repository_routing.config_generation
        );

        let unrelated_source = base_source.replace("id: test-instance", "id: renamed-instance");
        let unrelated =
            resolve_central_config(&root.path().join("unrelated.yaml"), &unrelated_source)
                .expect("unrelated config edit should resolve");
        let unrelated_checkout = unrelated
            .repository_checkouts
            .values()
            .next()
            .expect("unrelated checkout should exist");
        assert_ne!(
            base.repository_routing.config_generation,
            unrelated.repository_routing.config_generation
        );
        assert_eq!(
            base_checkout.policy_generation,
            unrelated_checkout.policy_generation
        );
        assert_eq!(
            base_checkout.review_policy_generation,
            unrelated_checkout.review_policy_generation
        );

        let scheduler_source =
            base_source.replace("max_concurrent_tasks: 2", "max_concurrent_tasks: 3");
        let scheduler =
            resolve_central_config(&root.path().join("scheduler.yaml"), &scheduler_source)
                .expect("scheduler policy edit should resolve");
        let scheduler_checkout = scheduler
            .repository_checkouts
            .values()
            .next()
            .expect("scheduler checkout should exist");
        assert_ne!(
            base_checkout.policy_generation,
            scheduler_checkout.policy_generation
        );
        assert_eq!(
            base_checkout.review_policy_generation,
            scheduler_checkout.review_policy_generation
        );

        let review_source = base_source.replace(
            "review_profiles:\n  github-standard:\n    provider: github",
            "review_profiles:\n  github-standard:\n    provider: gitlab",
        );
        let review = resolve_central_config(&root.path().join("review.yaml"), &review_source)
            .expect("review policy edit should resolve");
        let review_checkout = review
            .repository_checkouts
            .values()
            .next()
            .expect("review checkout should exist");
        assert_eq!(
            base_checkout.policy_generation,
            review_checkout.policy_generation
        );
        assert_ne!(
            base_checkout.review_policy_generation,
            review_checkout.review_policy_generation
        );
        assert_eq!(review_checkout.review_provider, "gitlab");
    }

    #[test]
    fn central_config_rejects_an_explicitly_blank_provider_id() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source =
            central_fixture(root.path()).replace("provider_id: repo-42", "provider_id: ' '");

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("an explicitly blank provider id should be rejected");

        assert!(matches!(
            error,
            CentralConfigError::EmptyField {
                field: "repositories.remote.provider_id"
            }
        ));
    }

    #[test]
    fn central_config_normalizes_project_routing_keys_before_indexing() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "provider_project_id: core-project",
            "provider_project_id: ' core-project '",
        );

        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("whitespace around a project routing key should be normalized");
        assert!(
            resolved
                .repository_routing
                .active_projects
                .contains("core-project")
        );
        assert!(matches!(
            resolved.repository_routing.resolve(
                &["repo:core".to_string()],
                Some("core-project"),
                None,
                false,
            ),
            crate::opensymphony_domain::RepositoryBindingOutcome::Resolved(_)
        ));
    }

    #[test]
    fn central_config_allows_zero_automatic_retries() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "  max_concurrent_tasks: 2\n",
            "  max_concurrent_tasks: 2\n  retry:\n    max_attempts: 0\n",
        );

        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("zero automatic retries should be valid");
        assert_eq!(resolved.retry_max_attempts, Some(0));
    }

    #[test]
    fn central_config_discriminator_requires_instance_and_routing_mode() {
        // Legacy config files may carry schema metadata without opting into
        // the central parser. Shared runtime sections are central-only and
        // still make malformed central files fail closed.
        assert!(!looks_like_central_config("schema_version: 1\n"));
        assert!(looks_like_central_config(
            "schema_version: 1\ncontrol_plane:\n  bind: 127.0.0.1:2468\n"
        ));
        assert!(looks_like_central_config(
            "schema_version: 1\nopenhands: {}\n"
        ));
        assert!(looks_like_central_config(
            "memory:\n  catalog_root: state/memory\n"
        ));
        assert!(looks_like_central_config("instance:\n  id: legacy\n"));
        assert!(looks_like_central_config(
            "routing:\n  mode: legacy_single\n"
        ));
        assert!(looks_like_central_config("routing:\n  mode: [broken]\n"));
        assert!(looks_like_central_config(
            "schema_version: 1\ninstance:\n  id: central\nrouting:\n  mode: legacy_single\n"
        ));
        assert!(looks_like_central_config("instance:\n  id: [broken\n"));
    }

    #[test]
    fn migrated_equal_project_id_and_slug_use_legacy_slug_fallback() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "provider_project_id: core-project",
            "provider_project_id: legacy-project\n    provider_project_slug: legacy-project",
        );
        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("legacy-compatible central config should resolve");
        assert_eq!(resolved.workflow_front_matter.tracker.project_id, None);
        assert_eq!(
            resolved
                .workflow_front_matter
                .tracker
                .project_slug
                .as_deref(),
            Some("legacy-project")
        );
    }

    #[test]
    fn central_config_rejects_unsupported_repo_local_compatibility() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path())
            .replace(
                "mode: project_set",
                "mode: legacy_single\n  repository: core-repo",
            )
            .replace("  active_project_set: suite\n", "")
            .replace(
                "workspace:\n  root:",
                "compatibility:\n  allow_repo_local_config: true\nworkspace:\n  root:",
            );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("unsupported repository-local compatibility must fail closed");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "compatibility.allow_repo_local_config"
        ));
    }

    #[test]
    fn central_config_preserves_workspace_retention_policy() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should exist");
        let source = central_fixture(root.path()).replace(
            &format!("workspace:\n  root: {}/workspace", root.path().display()),
            &format!(
                "workspace:\n  root: {}/workspace\n  retain_failed: false",
                root.path().display()
            ),
        );
        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("central fixture should resolve");
        assert!(!resolved.retain_failed);
    }

    #[test]
    fn central_config_rejects_duplicate_aliases() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source =
            central_fixture(root.path()).replace("aliases: [core]", "aliases: [core, core]");

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("duplicate aliases should fail");
        assert!(matches!(error, CentralConfigError::DuplicateAlias { .. }));
    }

    #[test]
    fn central_config_rejects_ambiguous_project_routing_keys() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace("projects: [core]", "projects: [core, other]")
            .replace(
                "linear_projects:\n  core:",
                "linear_projects:\n  other:\n    provider_project_id: core\n    repositories: [other-repo]\n  core:",
            )
            .replace(
                "repositories:\n  core-repo:",
                "repositories:\n  other-repo:\n    aliases: [other]\n    remote:\n      provider: github\n      locator: example/other\n      clone: git@github.com:example/other.git\n    target_branch: develop\n    credential: github-ssh\n    review_profile: github-standard\n    instructions:\n      path: AGENTS.md\n  core-repo:",
            );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("colliding project keys should fail closed");
        assert!(matches!(
            error,
            CentralConfigError::AmbiguousProjectRoutingKey { key } if key == "core"
        ));
    }

    #[test]
    fn central_config_rejects_case_insensitive_project_routing_collisions() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace("projects: [core]", "projects: [core, other]")
            .replace(
                "linear_projects:\n  core:\n    provider_project_id: core-project\n    repositories: [core-repo]",
                "linear_projects:\n  other:\n    provider_project_id: CORE\n    repositories: [other-repo]\n  core:\n    provider_project_id: core-project\n    repositories: [core-repo]",
            )
            .replace(
                "repositories:\n  core-repo:",
                "repositories:\n  other-repo:\n    aliases: [other]\n    remote:\n      provider: github\n      locator: example/other\n      clone: git@github.com:example/other.git\n    target_branch: develop\n    credential: github-ssh\n    review_profile: github-standard\n    instructions:\n      path: AGENTS.md\n  core-repo:",
            );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("case-insensitive project keys with different repositories should fail");
        assert!(
            matches!(
                error,
                CentralConfigError::AmbiguousProjectRoutingKey { ref key } if key == "CORE"
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn central_config_allows_alias_reuse_in_inactive_project_sets() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path())
            .replace(
                "  suite:\n    tracker_profile: linear\n    integration_instructions: integration.md\n    projects: [core]",
                "  suite:\n    tracker_profile: linear\n    integration_instructions: integration.md\n    projects: [core]\n  inactive:\n    tracker_profile: linear\n    projects: [inactive]",
            )
            .replace(
                "linear_projects:\n  core:",
                "linear_projects:\n  inactive:\n    provider_project_id: inactive-project\n    repositories: [inactive-repo]\n  core:",
            )
            .replace(
                "repositories:\n  core-repo:",
                "repositories:\n  inactive-repo:\n    aliases: [core]\n    remote:\n      provider: github\n      locator: example/inactive\n      clone: git@github.com:example/inactive.git\n    target_branch: develop\n    credential: github-ssh\n    review_profile: github-standard\n    instructions:\n      path: AGENTS.md\n  core-repo:",
            );

        resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("aliases in inactive project sets should not collide");
    }

    #[test]
    fn central_config_rejects_credentials_in_remote() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            "git@github.com:kumanday/OpenSymphony.git",
            "https://token:secret@example.com/repo.git",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("credential-bearing remote should fail");
        assert!(matches!(error, CentralConfigError::CredentialBearingRemote));
    }

    #[test]
    fn central_config_rejects_credentials_in_remote_locator() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            "locator: kumanday/OpenSymphony",
            "locator: https://token:secret@example.com/repo",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("credential-bearing remote locator should fail");
        assert!(matches!(error, CentralConfigError::CredentialBearingRemote));
    }

    #[test]
    fn central_config_rejects_credential_shaped_ssh_usernames() {
        let root = tempfile::tempdir().expect("central config root should exist");
        for clone in [
            "ssh://ghp_secret@example.com/team/repo.git",
            "github_pat_secret@example.com:team/repo.git",
        ] {
            let source = central_fixture(root.path())
                .replace("git@github.com:kumanday/OpenSymphony.git", clone);
            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("credential-shaped SSH username should fail");
            assert!(matches!(error, CentralConfigError::CredentialBearingRemote));
        }
    }

    #[test]
    fn central_config_allows_ordinary_ssh_usernames_in_remotes() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        for clone in [
            "ssh://deploy@example.com/team/repo.git",
            "deploy@example.com:team/repo.git",
        ] {
            let source = central_fixture(root.path())
                .replace("git@github.com:kumanday/OpenSymphony.git", clone);
            resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect("ordinary SSH usernames should not be treated as credentials");
        }
    }

    #[test]
    fn central_config_rejects_repository_credential_transport_mismatches() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let https_with_ssh_agent = central_fixture(root.path()).replace(
            "git@github.com:kumanday/OpenSymphony.git",
            "https://github.com/kumanday/OpenSymphony.git",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &https_with_ssh_agent)
            .expect_err("SSH credentials should not be paired with HTTPS clones");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "repositories.core-repo.credential"
        ));

        let ssh_with_environment = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent",
            "  github-ssh:\n    kind: environment\n    variable: GITHUB_TOKEN",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &ssh_with_environment)
            .expect_err("environment credentials should not be paired with SSH clones");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "repositories.core-repo.credential"
        ));

        let http_with_environment = ssh_with_environment.replace(
            "git@github.com:kumanday/OpenSymphony.git",
            "http://github.com/kumanday/OpenSymphony.git",
        );
        let error =
            resolve_central_config(&root.path().join("config.yaml"), &http_with_environment)
                .expect_err("environment credentials should not be paired with HTTP clones");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "repositories.core-repo.credential"
        ));
    }

    #[test]
    fn central_legacy_config_skips_verified_checkout_credential_constraints() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let legacy = central_fixture(root.path())
            .replace(
                "mode: project_set\n  active_project_set: suite",
                "mode: legacy_single\n  repository: core-repo",
            )
            .replace("    integration_instructions: integration.md\n", "");
        let typed = legacy.replace(
            "  github-ssh:\n    kind: ssh-agent",
            "  github-ssh:\n    kind: codex_cli_login\n    reference: codex-cli:chatgpt-login",
        );
        resolve_central_config(&root.path().join("typed.yaml"), &typed)
            .expect("legacy routing must not validate unused checkout credential providers");

        let mismatched_transport = legacy.replace(
            "git@github.com:kumanday/OpenSymphony.git",
            "https://github.com/kumanday/OpenSymphony.git",
        );
        resolve_central_config(&root.path().join("mismatched.yaml"), &mismatched_transport)
            .expect("legacy routing must not validate unused clone transport credentials");
    }

    #[test]
    fn central_config_rejects_generated_memory_runtime_variable_reuse() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        for variable in [
            "OPENSYMPHONY_MEMORY_ENDPOINT",
            "OPENSYMPHONY_MEMORY_PROJECT",
            "OPENSYMPHONY_MEMORY_EXECUTION_REPO",
        ] {
            let source = central_fixture(root.path())
                .replace(
                    "  github-ssh:\n    kind: ssh-agent",
                    &format!("  github-ssh:\n    kind: environment\n    variable: {variable}"),
                )
                .replace(
                    &format!("  catalog_root: {}/state/memory", root.path().display()),
                    &format!(
                        "  catalog_root: {}/state/memory\n  serve: true",
                        root.path().display()
                    ),
                );

            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err(
                    "generated memory runtime variables must not reuse checkout credentials",
                );
            assert!(matches!(
                error,
                CentralConfigError::InvalidReference { field }
                    if field == "memory.runtime"
            ));
        }
    }

    #[test]
    fn central_config_rejects_tracker_credentials_without_environment_variable() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace("    variable: LINEAR_API_KEY\n", "");
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("tracker credentials without a variable should fail");
        assert!(matches!(error, CentralConfigError::InvalidReference { .. }));
    }

    #[test]
    fn central_config_rejects_repository_credentials_without_a_provider() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent",
            "  github-ssh:\n    kind: environment",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("repository credentials without a provider should fail");
        assert!(matches!(error, CentralConfigError::InvalidReference { .. }));
    }

    #[test]
    fn central_config_rejects_tracker_endpoint_credentials() {
        let root = tempfile::tempdir().expect("central config root should exist");
        for endpoint in [
            "https://token@example.test/graphql",
            "https://api.example.test/graphql?access_token=secret",
        ] {
            let source = central_fixture(root.path()).replace(
                "endpoint: https://api.linear.app/graphql",
                &format!("endpoint: {endpoint}"),
            );
            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("tracker endpoint credentials should fail");
            assert!(matches!(error, CentralConfigError::CredentialBearingRemote));
        }
    }

    #[test]
    fn central_config_rejects_non_http_tracker_endpoints() {
        let root = tempfile::tempdir().expect("central config root should exist");
        for endpoint in ["ssh://api.example.test/graphql", "api.example.test/graphql"] {
            let source = central_fixture(root.path()).replace(
                "endpoint: https://api.linear.app/graphql",
                &format!("endpoint: {endpoint}"),
            );
            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("tracker endpoints must be HTTP(S) URLs");
            assert!(matches!(
                error,
                CentralConfigError::InvalidReference { field }
                    if field == "tracker_profiles.linear.endpoint"
            ));
        }
    }

    #[test]
    fn central_config_rejects_credential_bearing_openhands_transport_urls() {
        let root = tempfile::tempdir().expect("central config root should exist");
        for (field, suffix) in [
            (
                "openhands.transport_base_url",
                "openhands:\n  transport_base_url: https://token@example.test/api\n",
            ),
            (
                "openhands.front_matter.transport.base_url",
                "openhands:\n  front_matter:\n    transport:\n      base_url: https://api.example.test/api?access_token=secret-canary\n",
            ),
        ] {
            let source = format!("{}\n{suffix}", central_fixture(root.path()));
            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("credential-bearing OpenHands URLs must fail closed");
            assert!(matches!(
                &error,
                CentralConfigError::InvalidReference { field: actual }
                    if actual == field
            ));
            assert!(!error.to_string().contains("secret-canary"));
        }
    }

    #[test]
    fn central_config_rejects_non_http_openhands_transport_urls() {
        let root = tempfile::tempdir().expect("central config root should exist");
        for base_url in [
            "ws://api.example.test",
            "ftp://api.example.test",
            "file:///tmp",
        ] {
            let source = format!(
                "{}\nopenhands:\n  transport_base_url: {base_url}\n",
                central_fixture(root.path())
            );
            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("OpenHands transport must use an HTTP(S) URL with a host");
            assert!(matches!(
                error,
                CentralConfigError::InvalidReference { field }
                    if field == "openhands.transport_base_url"
            ));
        }
    }

    #[test]
    fn central_runtime_preserves_repository_local_front_matter_extensions() {
        let local = WorkflowDefinition::parse(
            "---\ncodex:\n  command: codex app-server\nlogging:\n  level: debug\n---\nImplementation instructions\n",
        )
        .expect("repository-local workflow should parse")
        .front_matter;
        let merged = merge_repository_local_front_matter(WorkflowFrontMatter::default(), &local);

        assert_eq!(merged.codex, local.codex);
        assert_eq!(merged.logging, local.logging);
        assert!(merged.extensions.is_empty());
    }

    #[test]
    fn central_config_rejects_raw_credential_references() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            "    variable: LINEAR_API_KEY\n",
            "    variable: LINEAR_API_KEY\n    reference: sk-raw-oauth-token\n",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("raw credential references should fail");
        assert!(matches!(
            &error,
            CentralConfigError::InvalidReference { field }
                if field == "credentials.linear-key.reference"
        ));
        assert!(!error.to_string().contains("sk-raw-oauth-token"));
    }

    #[test]
    fn central_config_accepts_typed_credential_references() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent\n",
            "  github-ssh:\n    kind: ssh-agent\n  typed-test:\n    kind: codex_cli_login\n    reference: codex-cli:chatgpt-login\n",
        );
        resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("typed credential references should resolve");
    }

    #[test]
    fn central_config_rejects_typed_credential_references_for_repositories() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent\n",
            "  github-ssh:\n    kind: codex_cli_login\n    reference: codex-cli:chatgpt-login\n",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("unsupported repository credential should fail closed");
        assert!(
            matches!(error, CentralConfigError::InvalidReference { field } if field == "repositories.core-repo.credential")
        );
    }

    #[test]
    fn central_config_rejects_remote_query_and_fragment_data() {
        let root = tempfile::tempdir().expect("central config root should exist");
        for suffix in ["?access_token=secret", "#secret"] {
            let source = central_fixture(root.path()).replace(
                "locator: kumanday/OpenSymphony",
                &format!("locator: https://github.com/kumanday/OpenSymphony{suffix}"),
            );
            let error = resolve_central_config(&root.path().join("config.yaml"), &source)
                .expect_err("remote query and fragment data should fail");
            assert!(matches!(error, CentralConfigError::CredentialBearingRemote));
        }
    }

    #[test]
    fn central_legacy_mode_rejects_ambiguous_tracker_and_project_candidates() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path())
            .replace(
                "mode: project_set\n  active_project_set: suite",
                "mode: legacy_single\n  repository: core-repo",
            )
            .replace(
                "project_sets:\n",
                "  another:\n    provider: linear\n    credential: linear-key\nproject_sets:\n",
            );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("legacy mode should reject ambiguous candidates");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "routing.repository.tracker_profile"
        ));
    }

    #[test]
    fn central_legacy_mode_rejects_repository_outside_project_associations() {
        let root = tempfile::tempdir().expect("temporary config root should exist");
        let source = central_fixture(root.path())
            .replace(
                "mode: project_set\n  active_project_set: suite",
                "mode: legacy_single\n  repository: core-repo",
            )
            .replace("    repositories: [core-repo]", "    repositories: []")
            .replace("    integration_instructions: integration.md\n", "");
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("legacy routing must respect project repository associations");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "routing.repository"
        ));
    }

    #[test]
    fn central_config_preserves_complete_openhands_profile() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    local_server:\n      enabled: true\n      command: [custom-openhands]\n    conversation:\n      agent:\n        llm:\n          model: custom/model\n          api_key_env: CUSTOM_OPENAI_KEY\n    websocket:\n      reconnect_max_ms: 9876\n",
            central_fixture(root.path())
        );
        let resolved = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("complete OpenHands profile should resolve");
        assert_eq!(
            resolved
                .workflow_front_matter
                .openhands
                .local_server
                .command,
            Some(vec!["custom-openhands".to_owned()])
        );
        assert_eq!(
            resolved
                .workflow_front_matter
                .openhands
                .conversation
                .agent
                .as_ref()
                .and_then(|agent| agent.llm.as_ref())
                .and_then(|llm| llm.api_key_env.as_deref()),
            Some("CUSTOM_OPENAI_KEY")
        );
    }

    #[test]
    fn central_config_rejects_literal_openhands_environment_secret() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    local_server:\n      env:\n        OPENAI_ACCESS_TOKEN: literal-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("literal OpenHands credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_literal_openhands_local_server_command_secret() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    local_server:\n      command: [curl, --oauth2-bearer, literal-secret]\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("literal OpenHands command credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_literal_hook_credential() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nhooks:\n  before_run: \"curl -H 'Authorization: Bearer hook-secret-canary' https://example.invalid\"\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("literal hook credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("hook-secret-canary"));
    }

    #[test]
    fn central_config_accepts_literal_local_server_environment_values() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    local_server:\n      env:\n        NODE_ENV: development\n",
            central_fixture(root.path())
        );

        resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect("literal local-server environment values should be accepted");
    }

    #[test]
    fn central_config_rejects_pat_named_openhands_secret() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    local_server:\n      env:\n        GITHUB_PAT: literal-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("PAT-shaped OpenHands credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_invalid_openhands_environment_selectors() {
        let root = tempfile::tempdir().expect("temporary config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    transport:\n      session_api_key_env: not-an-environment-name\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("OpenHands environment selectors must use environment names");
        assert!(matches!(error, CentralConfigError::InvalidReference { .. }));
        assert!(!error.to_string().contains("not-an-environment-name"));
    }

    #[test]
    fn central_config_rejects_hyphenated_openhands_environment_selectors() {
        let root = tempfile::tempdir().expect("temporary config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              access-token-env: literal-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("hyphenated environment selectors must use environment names");
        assert!(matches!(error, CentralConfigError::InvalidReference { .. }));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_checkout_credential_reuse_by_camel_case_openhands_selector() {
        let root = tempfile::tempdir().expect("temporary config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = central_fixture(root.path()).replace(
            "  github-ssh:\n    kind: ssh-agent",
            "  github-ssh:\n    kind: environment\n    variable: GITHUB_TOKEN",
        ) + "\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              accessTokenEnv: GITHUB_TOKEN\n";

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("camelCase OpenHands selectors must not reuse checkout credentials");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "openhands.front_matter"
        ));
    }

    #[test]
    fn central_config_rejects_camel_case_openhands_environment_selectors() {
        let root = tempfile::tempdir().expect("temporary config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              accessTokenEnv: literal-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("camelCase environment selectors must use environment names");
        assert!(matches!(error, CentralConfigError::InvalidReference { .. }));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_structured_openhands_secret_values() {
        let root = tempfile::tempdir().expect("temporary config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              access_token:\n                value: nested-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("structured OpenHands credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("nested-secret"));
    }

    #[test]
    fn central_config_rejects_nested_openhands_tool_secret() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              refresh_token: literal-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("nested OpenHands credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_literal_openhands_account_identity() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              chatgpt_account_id: acct_123\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("literal OpenHands account identities must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("acct_123"));
    }

    #[test]
    fn central_config_rejects_hyphenated_openhands_account_identity() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              chatgpt-account-id: acct_456\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("hyphenated OpenHands account identities must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("acct_456"));
    }

    #[test]
    fn central_config_rejects_camel_case_openhands_secret() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(
            root.path().join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  front_matter:\n    conversation:\n      agent:\n        tools:\n          - name: github\n            params:\n              accessToken: literal-secret\n",
            central_fixture(root.path())
        );

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("camelCase OpenHands credentials must be rejected");
        assert!(matches!(error, CentralConfigError::LiteralSecret));
        assert!(!error.to_string().contains("literal-secret"));
    }

    #[test]
    fn central_config_rejects_conflicting_openhands_transport_definitions() {
        let root = tempfile::tempdir().expect("central config root should exist");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        let source = format!(
            "{}\nopenhands:\n  transport_base_url: https://one.example\n  front_matter:\n    transport:\n      base_url: https://two.example\n",
            central_fixture(root.path())
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("conflicting transport definitions should fail");
        assert!(matches!(
            error,
            CentralConfigError::InvalidReference { field }
                if field == "openhands.transport_base_url"
        ));
    }

    #[test]
    fn central_config_rejects_overlapping_state_and_workspace_roots() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            &format!("root: {}/workspace", root.path().display()),
            &format!("root: {}/state/memory", root.path().display()),
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("overlapping instance roots should fail");
        assert!(matches!(error, CentralConfigError::OverlappingRoots { .. }));
    }

    #[test]
    fn central_config_rejects_empty_required_roots_before_expansion() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            &format!("state_root: {}/state", root.path().display()),
            "state_root: \"\"",
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("empty state roots should fail before path expansion");
        assert!(matches!(
            error,
            CentralConfigError::EmptyField {
                field: "instance.state_root"
            }
        ));
    }

    #[test]
    fn central_config_rejects_memory_catalog_at_state_root() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace(
            &format!("catalog_root: {}/state/memory", root.path().display()),
            &format!("catalog_root: {}/state", root.path().display()),
        );
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("memory catalog must have its own state subdirectory");
        assert!(matches!(error, CentralConfigError::InvalidRoot));
    }

    #[test]
    fn central_config_rejects_integration_inside_inventory_checkout() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let integration_path = root.path().join("checkout/integration.md");
        std::fs::create_dir_all(integration_path.parent().expect("integration parent"))
            .expect("checkout should be created");
        std::fs::write(&integration_path, "integration instructions\n")
            .expect("integration instructions should be written");
        let source =
            central_fixture(root.path()).replace("integration.md", "checkout/integration.md");
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("checkout-local integration instructions should fail");
        assert!(matches!(
            error,
            CentralConfigError::IntegrationInsideCheckout
        ));
    }

    #[cfg(unix)]
    #[test]
    fn central_config_resolves_symlinked_integration_paths_before_containment() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("central config root should exist");
        let checkout = root.path().join("checkout");
        std::fs::create_dir_all(&checkout).expect("checkout should be created");
        std::fs::write(
            checkout.join("integration.md"),
            "integration instructions\n",
        )
        .expect("integration instructions should be written");
        symlink(&checkout, root.path().join("checkout-link")).expect("symlink should be created");
        let source =
            central_fixture(root.path()).replace("integration.md", "checkout-link/integration.md");
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("symlinked checkout-local instructions should fail");
        assert!(matches!(
            error,
            CentralConfigError::IntegrationInsideCheckout
        ));
    }

    #[cfg(unix)]
    #[test]
    fn central_config_rejects_repository_instruction_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("central config root should exist");
        let checkout = root.path().join("checkout");
        let external = root.path().join("external.md");
        std::fs::create_dir_all(&checkout).expect("checkout should be created");
        std::fs::write(root.path().join("integration.md"), "integration\n")
            .expect("integration instructions should be written");
        std::fs::write(&external, "external instructions\n")
            .expect("external instructions should be written");
        symlink(&external, checkout.join("AGENTS.md")).expect("instruction symlink should exist");

        let error = resolve_central_config(
            &root.path().join("config.yaml"),
            &central_fixture(root.path()),
        )
        .expect_err("repository instruction symlinks must stay inside the checkout");
        assert!(matches!(error, CentralConfigError::InvalidInstructionPath));
    }

    #[test]
    fn central_config_rejects_unknown_fields() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = format!("{}\nunknown: true\n", central_fixture(root.path()));
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("unknown fields should fail");
        assert!(matches!(error, CentralConfigError::Parse { .. }));
    }

    #[test]
    fn explicit_config_selection_does_not_depend_on_repository_checkout() {
        let cwd = Path::new("/tmp/launch-directory");
        let selected = select_config_path(cwd, Some(Path::new("/tmp/instance/config.yaml")));
        assert_eq!(selected, Some(PathBuf::from("/tmp/instance/config.yaml")));
    }
}
