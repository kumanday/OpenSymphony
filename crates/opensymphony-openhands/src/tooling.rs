use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

const PLACEHOLDER_VERSION: &str = "0+bootstrap.placeholder";
const PLACEHOLDER_REQUIREMENT: &str = "openhands-agent-server-placeholder==0+bootstrap.placeholder";
const PLACEHOLDER_LOCK_SNIPPET: &str = "Placeholder bootstrap file.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalToolingLayout {
    pub tool_dir: PathBuf,
    pub run_local_script: PathBuf,
    pub pyproject: PathBuf,
    pub lockfile: PathBuf,
    pub version_file: PathBuf,
}

impl LocalToolingLayout {
    pub fn from_tool_dir(tool_dir: impl Into<PathBuf>) -> Self {
        let tool_dir = tool_dir.into();
        Self {
            run_local_script: tool_dir.join("run-local.sh"),
            pyproject: tool_dir.join("pyproject.toml"),
            lockfile: tool_dir.join("uv.lock"),
            version_file: tool_dir.join("version.txt"),
            tool_dir,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinStatus {
    pub version_pinned: bool,
    pub dependency_pinned: bool,
    pub lockfile_resolved: bool,
}

impl PinStatus {
    pub fn is_ready(&self) -> bool {
        self.version_pinned && self.dependency_pinned && self.lockfile_resolved
    }

    pub fn blocking_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if !self.version_pinned {
            issues.push("version.txt still contains the bootstrap placeholder".to_string());
        }

        if !self.dependency_pinned {
            issues.push(
                "pyproject.toml still contains the placeholder agent-server dependency".to_string(),
            );
        }

        if !self.lockfile_resolved {
            issues.push("uv.lock still contains the bootstrap placeholder content".to_string());
        }

        issues
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ToolingMetadata {
    pub module: String,
    pub runtime_env: String,
    pub runtime: String,
    pub host: String,
    pub default_port: u16,
    pub port_env: String,
    pub launcher: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalServerTooling {
    pub layout: LocalToolingLayout,
    pub metadata: ToolingMetadata,
    pub version: String,
    pub pin_status: PinStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLaunch {
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub working_dir: PathBuf,
    pub base_url: String,
    pub version: String,
    pub launcher_summary: String,
}

impl LocalServerTooling {
    pub fn load(tool_dir: impl Into<PathBuf>) -> Result<Self, LocalToolingError> {
        let layout = LocalToolingLayout::from_tool_dir(tool_dir);
        ensure_exists(&layout.run_local_script)?;
        ensure_exists(&layout.pyproject)?;
        ensure_exists(&layout.lockfile)?;
        ensure_exists(&layout.version_file)?;

        let pyproject_contents = read_to_string(&layout.pyproject)?;
        let parsed: Pyproject = toml::from_str(&pyproject_contents).map_err(|source| {
            LocalToolingError::ParsePyproject {
                path: layout.pyproject.clone(),
                source,
            }
        })?;

        let metadata = parsed
            .tool
            .and_then(|tool| tool.opensymphony)
            .and_then(|opensymphony| opensymphony.openhands_server)
            .ok_or(LocalToolingError::MissingToolingMetadata {
                path: layout.pyproject.clone(),
            })?;

        if metadata.host != "127.0.0.1" {
            return Err(LocalToolingError::NonLoopbackHost {
                host: metadata.host.clone(),
            });
        }

        if metadata.runtime != "process" {
            return Err(LocalToolingError::NonProcessRuntime {
                runtime: metadata.runtime.clone(),
            });
        }

        let agent_server_dependencies = parsed
            .project
            .optional_dependencies
            .get("agent-server")
            .ok_or(LocalToolingError::MissingAgentServerDependency {
                path: layout.pyproject.clone(),
            })?;

        let version = read_to_string(&layout.version_file)?.trim().to_string();
        let lockfile_contents = read_to_string(&layout.lockfile)?;
        let pin_status = PinStatus {
            version_pinned: version != PLACEHOLDER_VERSION,
            dependency_pinned: !agent_server_dependencies
                .iter()
                .any(|dependency| dependency == PLACEHOLDER_REQUIREMENT),
            lockfile_resolved: !lockfile_contents.contains(PLACEHOLDER_LOCK_SNIPPET),
        };

        Ok(Self {
            layout,
            metadata,
            version,
            pin_status,
        })
    }

    pub fn port(&self, port_override: Option<u16>) -> u16 {
        port_override.unwrap_or(self.metadata.default_port)
    }

    pub fn base_url(&self, port_override: Option<u16>) -> String {
        format!("http://{}:{}", self.metadata.host, self.port(port_override))
    }

    pub fn resolve_launch(
        &self,
        port_override: Option<u16>,
        extra_env: &BTreeMap<String, String>,
    ) -> Result<ResolvedLaunch, LocalToolingError> {
        if !self.pin_status.is_ready() {
            return Err(LocalToolingError::UnresolvedPin {
                details: self.pin_status.blocking_issues().join("; "),
            });
        }

        let mut env = BTreeMap::new();
        env.insert(
            self.metadata.port_env.clone(),
            self.port(port_override).to_string(),
        );
        env.insert(
            self.metadata.runtime_env.clone(),
            self.metadata.runtime.clone(),
        );
        env.extend(
            extra_env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );

        Ok(ResolvedLaunch {
            program: "bash".to_string(),
            args: vec![self.layout.run_local_script.display().to_string()],
            env,
            working_dir: self.layout.tool_dir.clone(),
            base_url: self.base_url(port_override),
            version: self.version.clone(),
            launcher_summary: self.metadata.launcher.clone(),
        })
    }
}

#[derive(Debug, Error)]
pub enum LocalToolingError {
    #[error("required local OpenHands tooling file is missing: {path}")]
    MissingFile { path: PathBuf },
    #[error("failed to read local OpenHands tooling file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse local OpenHands pyproject {path}: {source}")]
    ParsePyproject {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("local OpenHands pyproject is missing [tool.opensymphony.openhands_server]: {path}")]
    MissingToolingMetadata { path: PathBuf },
    #[error(
        "local OpenHands pyproject is missing [project.optional-dependencies].agent-server: {path}"
    )]
    MissingAgentServerDependency { path: PathBuf },
    #[error("local OpenHands tooling must default to host 127.0.0.1, found {host}")]
    NonLoopbackHost { host: String },
    #[error("local OpenHands tooling must force RUNTIME=process, found {runtime}")]
    NonProcessRuntime { runtime: String },
    #[error("local OpenHands tooling is not pinned yet: {details}")]
    UnresolvedPin { details: String },
}

#[derive(Debug, Deserialize)]
struct Pyproject {
    project: PyprojectProject,
    tool: Option<PyprojectTool>,
}

#[derive(Debug, Deserialize)]
struct PyprojectProject {
    #[serde(rename = "optional-dependencies", default)]
    optional_dependencies: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PyprojectTool {
    opensymphony: Option<PyprojectOpenSymphony>,
}

#[derive(Debug, Deserialize)]
struct PyprojectOpenSymphony {
    openhands_server: Option<ToolingMetadata>,
}

fn ensure_exists(path: &Path) -> Result<(), LocalToolingError> {
    if path.exists() {
        Ok(())
    } else {
        Err(LocalToolingError::MissingFile {
            path: path.to_path_buf(),
        })
    }
}

fn read_to_string(path: &Path) -> Result<String, LocalToolingError> {
    fs::read_to_string(path).map_err(|source| LocalToolingError::ReadFile {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::Path};

    use tempfile::TempDir;

    use super::{LocalServerTooling, LocalToolingError};

    #[test]
    fn rejects_non_loopback_launcher_metadata() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_tooling_fixture(temp_dir.path(), "0.0.1", "0.0.1", "0.0.1", "0.0.0.0");

        let error = LocalServerTooling::load(temp_dir.path()).expect_err("load should fail");
        assert!(matches!(error, LocalToolingError::NonLoopbackHost { .. }));
    }

    #[test]
    fn refuses_to_resolve_unpinned_tooling() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_tooling_fixture(
            temp_dir.path(),
            "0+bootstrap.placeholder",
            "openhands-agent-server-placeholder==0+bootstrap.placeholder",
            "Placeholder bootstrap file.",
            "127.0.0.1",
        );

        let tooling = LocalServerTooling::load(temp_dir.path()).expect("load should succeed");
        let error = tooling
            .resolve_launch(None, &BTreeMap::new())
            .expect_err("resolve should fail");

        assert!(matches!(error, LocalToolingError::UnresolvedPin { .. }));
    }

    fn write_tooling_fixture(
        tool_dir: &Path,
        version: &str,
        dependency: &str,
        lockfile: &str,
        host: &str,
    ) {
        fs::write(
            tool_dir.join("run-local.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .expect("run-local");
        fs::write(
            tool_dir.join("pyproject.toml"),
            format!(
                "[project]\nname = \"fixture\"\nversion = \"0.0.0\"\n[project.optional-dependencies]\nagent-server = [\"{dependency}\"]\n[tool.opensymphony.openhands_server]\nmodule = \"openhands.agent_server\"\nruntime_env = \"RUNTIME\"\nruntime = \"process\"\nhost = \"{host}\"\ndefault_port = 8000\nport_env = \"OPENHANDS_SERVER_PORT\"\nlauncher = \"RUNTIME=process uv run --module openhands.agent_server --host {host} --port 8000\"\n"
            ),
        )
        .expect("pyproject");
        fs::write(tool_dir.join("uv.lock"), lockfile).expect("uv.lock");
        fs::write(tool_dir.join("version.txt"), version).expect("version");
    }
}
