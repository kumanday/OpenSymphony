//! Runtime config loading for the `opensymphony run` command.

use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use crate::opensymphony_memory::DEFAULT_PRIVATE_MEMORY_CONFIG_FILE;
use crate::opensymphony_openhands::OpenHandsConversationStorePaths;
use crate::opensymphony_workflow::{ResolvedWorkflow, WorkflowDefinition};
use serde::Deserialize;
use tokio::fs;

use super::{RunArgs, RunCommandError};

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const DEFAULT_CONTROL_PLANE_BIND: &str = "127.0.0.1:2468";
const DEFAULT_MEMORY_SERVER_BIND: &str = "127.0.0.1:0";
const DEFAULT_MEMORY_TOKEN_ENV: &str = "OPENSYMPHONY_MEMORY_TOKEN";

#[derive(Debug, Default, Deserialize)]
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

#[derive(Debug, Default, Deserialize)]
struct ControlPlaneConfigFile {
    #[serde(default)]
    bind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RunOpenHandsConfigFile {
    #[serde(default)]
    tool_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
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
    pub(super) target_repo: PathBuf,
    pub(super) workflow_path: PathBuf,
    pub(super) workflow: ResolvedWorkflow,
    pub(super) bind: SocketAddr,
    pub(super) tool_dir: Option<PathBuf>,
    pub(super) openhands_conversation_store: Option<OpenHandsConversationStorePaths>,
    pub(super) memory: RunMemoryConfig,
}

pub(super) async fn resolve_runtime_config(
    args: &RunArgs,
) -> Result<RunRuntimeConfig, RunCommandError> {
    let cwd = env::current_dir().map_err(RunCommandError::CurrentDir)?;
    let config_path = match &args.config {
        Some(path) => Some(resolve_relative_to(&cwd, path)),
        None => {
            let candidate = cwd.join(DEFAULT_CONFIG_FILE);
            candidate.exists().then_some(candidate)
        }
    };

    let config = match &config_path {
        Some(path) => load_run_config(path).await?,
        None => RunConfigFile::default(),
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
    let mut workflow = workflow
        .resolve_with_process_env(&target_repo)
        .map_err(|source| RunCommandError::ResolveWorkflow {
            path: workflow_path.clone(),
            source,
        })?;
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
    validate_memory_bootstrap(&target_repo, &memory)?;

    Ok(RunRuntimeConfig {
        config_path,
        target_repo,
        workflow_path,
        workflow,
        bind,
        tool_dir,
        openhands_conversation_store,
        memory,
    })
}

fn validate_memory_bootstrap(
    target_repo: &Path,
    memory: &RunMemoryConfig,
) -> Result<(), RunCommandError> {
    if !memory.auto_capture && memory.server.is_none() {
        return Ok(());
    }
    let path = target_repo.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE);
    if path.is_file() {
        return Ok(());
    }
    Err(RunCommandError::MissingMemoryConfig { path })
}

async fn load_run_config(path: &Path) -> Result<RunConfigFile, RunCommandError> {
    let raw = fs::read_to_string(path)
        .await
        .map_err(|source| RunCommandError::ReadConfig {
            path: path.to_path_buf(),
            source,
        })?;
    let config = serde_yaml::from_str::<RunConfigFile>(&raw).map_err(|source| {
        RunCommandError::ParseConfig {
            path: path.to_path_buf(),
            source,
        }
    })?;
    resolve_run_config(path, config)
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

        let result = validate_memory_bootstrap(repo.path(), &memory);

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

        validate_memory_bootstrap(repo.path(), &memory).expect("disabled auto-capture should pass");
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

        validate_memory_bootstrap(repo.path(), &memory).expect("memory config should satisfy run");
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

        let result = validate_memory_bootstrap(repo.path(), &memory);

        assert!(matches!(
            result,
            Err(RunCommandError::MissingMemoryConfig { .. })
        ));
    }
}
