//! Runtime config loading for the `opensymphony run` command.

use std::{
    env,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use opensymphony_workflow::{ResolvedWorkflow, WorkflowDefinition};
use serde::Deserialize;
use tokio::fs;

use super::{RunArgs, RunCommandError};

const DEFAULT_CONFIG_FILE: &str = "config.yaml";
const DEFAULT_CONTROL_PLANE_BIND: &str = "127.0.0.1:3000";

#[derive(Debug, Default, Deserialize)]
struct RunConfigFile {
    #[serde(default)]
    target_repo: Option<String>,
    #[serde(default)]
    control_plane: ControlPlaneConfigFile,
    #[serde(default)]
    openhands: RunOpenHandsConfigFile,
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

pub(super) struct RunRuntimeConfig {
    pub(super) config_path: Option<PathBuf>,
    pub(super) target_repo: PathBuf,
    pub(super) workflow_path: PathBuf,
    pub(super) workflow: ResolvedWorkflow,
    pub(super) bind: SocketAddr,
    pub(super) tool_dir: Option<PathBuf>,
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
    let workflow = workflow
        .resolve_with_process_env(&target_repo)
        .map_err(|source| RunCommandError::ResolveWorkflow {
            path: workflow_path.clone(),
            source,
        })?;
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

    Ok(RunRuntimeConfig {
        config_path,
        target_repo,
        workflow_path,
        workflow,
        bind,
        tool_dir,
    })
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
