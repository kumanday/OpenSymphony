//! Runtime config loading for the `opensymphony run` command.

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use crate::opensymphony_memory::DEFAULT_PRIVATE_MEMORY_CONFIG_FILE;
use crate::opensymphony_openhands::OpenHandsConversationStorePaths;
use crate::opensymphony_workflow::{
    AgentFrontMatter, HooksFrontMatter, IntegerLike, OpenHandsFrontMatter, PollingFrontMatter,
    ResolvedWorkflow, RoutingFrontMatter, TrackerFrontMatter, WorkflowDefinition,
    WorkflowFrontMatter, WorkspaceFrontMatter,
};
use serde::Deserialize;
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

#[derive(Debug, Clone, Deserialize)]
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
    retain_failed: bool,
    #[serde(default)]
    cleanup_after_parent_finalization: bool,
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Default, Deserialize)]
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
    #[serde(default)]
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
    pub memory_catalog_root: Option<PathBuf>,
    pub mode: CentralRoutingMode,
    pub repository: Option<String>,
    pub integration_instructions: Option<ResolvedIntegrationInstructions>,
    pub generation: String,
    pub retry_max_attempts: Option<u32>,
    runtime: RunConfigFile,
    pub workflow_front_matter: WorkflowFrontMatter,
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
    #[error("central config roots overlap: `{left}` and `{right}`")]
    OverlappingRoots { left: PathBuf, right: PathBuf },
    #[error("central config repository remote contains credentials")]
    CredentialBearingRemote,
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
    pub(super) config_generation: String,
    pub(super) target_repo: PathBuf,
    pub(super) workflow_path: PathBuf,
    pub(super) workflow: ResolvedWorkflow,
    pub(super) bind: SocketAddr,
    pub(super) tool_dir: Option<PathBuf>,
    pub(super) openhands_conversation_store: Option<OpenHandsConversationStorePaths>,
    pub(super) retry_max_attempts: Option<u32>,
    pub(super) memory_catalog_root: Option<PathBuf>,
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
        central_workspace_root,
        central_memory_catalog_root,
        central_workflow_front_matter,
        retry_max_attempts,
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
                if central.mode == CentralRoutingMode::ProjectSet {
                    return Err(RunCommandError::StrictRoutingDisabled {
                        generation: central.generation,
                    });
                }
                (
                    central.runtime,
                    central.generation,
                    central.workspace_root,
                    central.memory_catalog_root,
                    Some(central.workflow_front_matter),
                    central.retry_max_attempts,
                )
            } else {
                (
                    parse_legacy_run_config(path, &raw)?,
                    generation_hash(raw.as_bytes()),
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
        ),
    };
    let config_root = config_path
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(cwd.as_path());
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|path| super::super::resolve_path(config_root, path))
        .unwrap_or_else(|| cwd.clone());
    let workflow_path = target_repo.join("WORKFLOW.md");
    let workflow = WorkflowDefinition::load_from_path(&workflow_path).map_err(|source| {
        RunCommandError::LoadWorkflow {
            path: workflow_path.clone(),
            source,
        }
    })?;
    let workflow = central_workflow_front_matter
        .map(|front_matter| WorkflowDefinition {
            front_matter,
            prompt_template: workflow.prompt_template.clone(),
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
        config_generation,
        target_repo,
        workflow_path,
        workflow,
        bind,
        tool_dir,
        openhands_conversation_store,
        retry_max_attempts,
        memory_catalog_root: central_memory_catalog_root,
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

pub fn looks_like_central_config(raw: &str) -> bool {
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(raw) else {
        return false;
    };
    let Some(mapping) = value.as_mapping() else {
        return false;
    };
    mapping.contains_key(serde_yaml::Value::String("schema_version".to_owned()))
        || mapping.contains_key(serde_yaml::Value::String("instance".to_owned()))
        || mapping
            .get(serde_yaml::Value::String("routing".to_owned()))
            .and_then(serde_yaml::Value::as_mapping)
            .is_some_and(|routing| {
                routing.contains_key(serde_yaml::Value::String("mode".to_owned()))
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
        if !is_contained(&state_root, &memory_root) {
            return Err(CentralConfigError::InvalidRoot);
        }
        let _ = (memory.auto_capture, memory.auto_archive, memory.serve);
        Some(memory_root)
    } else {
        None
    };

    let mut aliases = BTreeSet::new();
    let mut checkout_roots: Vec<PathBuf> = Vec::new();
    for (repository_id, repository) in &config.repositories {
        validate_repository(repository_id, repository, config_root)?;
        if !config.credentials.contains_key(&repository.credential) {
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
            if !aliases.insert(alias.clone()) {
                return Err(CentralConfigError::DuplicateAlias {
                    alias: alias.clone(),
                });
            }
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
            required_literal(reference, "credentials.reference")?;
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
            required_literal(merge_method, "review_profiles.merge_method")?;
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
    }
    for (project_id, project) in &config.linear_projects {
        required_literal(project_id, "linear_projects.id")?;
        required_literal(
            &project.provider_project_id,
            "linear_projects.provider_project_id",
        )?;
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
    }
    let retry_max_attempts = if let Some(scheduler) = config.scheduler.as_ref() {
        if scheduler.max_concurrent_tasks == 0
            || scheduler
                .retry
                .max_attempts
                .is_some_and(|attempts| attempts == 0)
        {
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
    let _ = config.compatibility.allow_repo_local_config;

    let mode = match config.routing.mode.trim() {
        "legacy_single" => CentralRoutingMode::LegacySingle,
        "project_set" => CentralRoutingMode::ProjectSet,
        _ => {
            return Err(CentralConfigError::InvalidReference {
                field: "routing.mode".to_owned(),
            });
        }
    };
    if mode == CentralRoutingMode::ProjectSet && config.compatibility.allow_repo_local_config {
        return Err(CentralConfigError::InvalidReference {
            field: "compatibility.allow_repo_local_config".to_owned(),
        });
    }
    let active_project_set = config.routing.active_project_set.as_deref();
    let mut integration_instructions = None;
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

    let mut generation_input = raw.as_bytes().to_vec();
    if let Some(instructions) = integration_instructions.as_ref() {
        generation_input.extend_from_slice(instructions.content_hash.as_bytes());
    }
    let runtime = central_legacy_runtime_config(&config, config_root)?;
    let workflow_front_matter = central_workflow_front_matter(&config, Some(&workspace_root))?;
    Ok(ResolvedCentralConfig {
        instance_id,
        state_root,
        workspace_root: Some(workspace_root),
        memory_catalog_root,
        mode,
        repository: config.routing.repository,
        integration_instructions,
        generation: generation_hash(&generation_input),
        retry_max_attempts,
        runtime,
        workflow_front_matter,
    })
}

fn central_workflow_front_matter(
    config: &CentralConfigFile,
    workspace_root: Option<&Path>,
) -> Result<WorkflowFrontMatter, CentralConfigError> {
    let (tracker, project_slug) = match config.routing.mode.trim() {
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
            (
                config
                    .tracker_profiles
                    .values()
                    .next()
                    .expect("length checked"),
                config
                    .linear_projects
                    .values()
                    .next()
                    .expect("length checked")
                    .provider_project_id
                    .clone(),
            )
        }
        "project_set" => {
            let project_set_id = config
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
            let project_id = project_set.projects.first().ok_or_else(|| {
                CentralConfigError::InvalidReference {
                    field: format!("project_sets.{project_set_id}.projects"),
                }
            })?;
            let project = config.linear_projects.get(project_id).ok_or_else(|| {
                CentralConfigError::InvalidReference {
                    field: format!("project_sets.{project_set_id}.projects"),
                }
            })?;
            (tracker, project.provider_project_id.clone())
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
            project_slug: Some(project_slug),
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

fn validate_repository(
    repository_id: &str,
    repository: &CentralRepositoryFile,
    config_root: &Path,
) -> Result<(), CentralConfigError> {
    required_literal(repository_id, "repositories.id")?;
    required_literal(&repository.remote.provider, "repositories.remote.provider")?;
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
    }
    Ok(())
}

fn validate_remote_clone(value: &str) -> Result<(), CentralConfigError> {
    if let Ok(url) = Url::parse(value) {
        let conventional_ssh_user =
            url.scheme().eq_ignore_ascii_case("ssh") && url.username().eq_ignore_ascii_case("git");
        if (!url.username().is_empty() && !conventional_ssh_user)
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
        && (user != "git" || host.is_empty())
    {
        return Err(CentralConfigError::CredentialBearingRemote);
    }
    Ok(())
}

fn required_literal(value: &str, field: &'static str) -> Result<String, CentralConfigError> {
    let value = value.trim();
    (!value.is_empty())
        .then(|| value.to_owned())
        .ok_or(CentralConfigError::EmptyField { field })
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
    }

    #[test]
    fn central_config_rejects_duplicate_aliases() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source =
            central_fixture(root.path()).replace("aliases: [core]", "aliases: [core, core]");

        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("duplicate aliases should fail");
        assert!(matches!(error, CentralConfigError::DuplicateAlias { .. }));
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
    fn central_config_rejects_tracker_credentials_without_environment_variable() {
        let root = tempfile::tempdir().expect("central config root should exist");
        let source = central_fixture(root.path()).replace("    variable: LINEAR_API_KEY\n", "");
        let error = resolve_central_config(&root.path().join("config.yaml"), &source)
            .expect_err("tracker credentials without a variable should fail");
        assert!(matches!(error, CentralConfigError::InvalidReference { .. }));
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
