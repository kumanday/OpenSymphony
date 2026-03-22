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
const REQUIRED_PIN_PACKAGES: [&str; 4] = [
    "openhands-agent-server",
    "openhands-sdk",
    "openhands-tools",
    "openhands-workspace",
];

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
    pub dependency_matches_version: bool,
    pub lockfile_resolved: bool,
    pub lockfile_matches_version: bool,
}

impl PinStatus {
    pub fn is_ready(&self) -> bool {
        self.version_pinned
            && self.dependency_pinned
            && self.dependency_matches_version
            && self.lockfile_resolved
            && self.lockfile_matches_version
    }

    pub fn blocking_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if !self.version_pinned {
            issues.push("version.txt still contains the bootstrap placeholder".to_string());
        }

        if !self.dependency_pinned {
            issues.push(
                "pyproject.toml is missing one or more required OpenHands package pins".to_string(),
            );
        }

        if self.version_pinned && !self.dependency_matches_version {
            issues
                .push("pyproject.toml OpenHands package pins do not match version.txt".to_string());
        }

        if !self.lockfile_resolved {
            issues.push(
                "uv.lock does not contain a verifiable resolved OpenHands package set".to_string(),
            );
        }

        if self.version_pinned && !self.lockfile_matches_version {
            issues.push("uv.lock OpenHands package versions do not match version.txt".to_string());
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
        let dependency_versions = required_pin_versions(agent_server_dependencies);
        let lockfile_versions = resolved_lockfile_versions(&lockfile_contents);
        let pin_status = PinStatus {
            version_pinned: version != PLACEHOLDER_VERSION,
            dependency_pinned: dependency_versions.is_some()
                && !agent_server_dependencies
                    .iter()
                    .any(|dependency| dependency == PLACEHOLDER_REQUIREMENT),
            dependency_matches_version: dependency_versions
                .as_ref()
                .is_some_and(|pins| pins.values().all(|pin| pin == &version)),
            lockfile_resolved: !lockfile_contents.contains(PLACEHOLDER_LOCK_SNIPPET)
                && lockfile_versions.is_some(),
            lockfile_matches_version: lockfile_versions
                .as_ref()
                .is_some_and(|pins| pins.values().all(|pin| pin == &version)),
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

#[derive(Debug, Deserialize)]
struct UvLock {
    #[serde(default)]
    package: Vec<UvLockPackage>,
}

#[derive(Debug, Deserialize)]
struct UvLockPackage {
    name: String,
    version: String,
}

fn required_pin_versions(requirements: &[String]) -> Option<BTreeMap<String, String>> {
    let pins: BTreeMap<String, String> = requirements
        .iter()
        .filter_map(|requirement| {
            let (name, version) = requirement.split_once("==")?;
            Some((name.trim().to_string(), version.trim().to_string()))
        })
        .collect();

    REQUIRED_PIN_PACKAGES
        .iter()
        .all(|package| pins.contains_key(*package))
        .then(|| {
            REQUIRED_PIN_PACKAGES
                .iter()
                .map(|package| (package.to_string(), pins[*package].clone()))
                .collect()
        })
}

fn resolved_lockfile_versions(lockfile_contents: &str) -> Option<BTreeMap<String, String>> {
    let parsed: UvLock = toml::from_str(lockfile_contents).ok()?;
    let pins: BTreeMap<String, String> = parsed
        .package
        .into_iter()
        .filter(|package| REQUIRED_PIN_PACKAGES.contains(&package.name.as_str()))
        .map(|package| (package.name, package.version))
        .collect();

    REQUIRED_PIN_PACKAGES
        .iter()
        .all(|package| pins.contains_key(*package))
        .then_some(pins)
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

    #[test]
    fn marks_pin_unready_when_dependency_versions_drift_from_version_file() {
        let temp_dir = TempDir::new().expect("temp dir");
        write_tooling_fixture(
            temp_dir.path(),
            "1.2.4",
            "openhands-agent-server==1.2.3",
            resolved_lockfile("1.2.3"),
            "127.0.0.1",
        );

        let tooling = LocalServerTooling::load(temp_dir.path()).expect("load should succeed");

        assert!(!tooling.pin_status.is_ready());
        assert!(tooling.pin_status.blocking_issues().iter().any(|issue| {
            issue.contains("pyproject.toml OpenHands package pins do not match version.txt")
        }));
        assert!(tooling.pin_status.blocking_issues().iter().any(|issue| {
            issue.contains("uv.lock OpenHands package versions do not match version.txt")
        }));
    }

    fn write_tooling_fixture(
        tool_dir: &Path,
        version: &str,
        dependency: &str,
        lockfile: impl AsRef<str>,
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
                "[project]\nname = \"fixture\"\nversion = \"0.0.0\"\n\n[project.optional-dependencies]\nagent-server = [\n  \"{dependency}\",\n  \"openhands-sdk==1.2.3\",\n  \"openhands-tools==1.2.3\",\n  \"openhands-workspace==1.2.3\",\n]\n\n[tool.opensymphony.openhands_server]\nmodule = \"openhands.agent_server\"\nruntime_env = \"RUNTIME\"\nruntime = \"process\"\nhost = \"{host}\"\ndefault_port = 8000\nport_env = \"OPENHANDS_SERVER_PORT\"\nlauncher = \"RUNTIME=process uv run --module openhands.agent_server --host {host} --port 8000\"\n"
            ),
        )
        .expect("pyproject");
        fs::write(tool_dir.join("uv.lock"), lockfile.as_ref()).expect("uv.lock");
        fs::write(tool_dir.join("version.txt"), version).expect("version");
    }

    fn resolved_lockfile(version: &str) -> String {
        format!(
            "version = 1\n\n[[package]]\nname = \"fixture\"\nversion = \"0.0.0\"\nsource = {{ virtual = \".\" }}\n\n[package.optional-dependencies]\nagent-server = [\n  {{ name = \"openhands-agent-server\" }},\n  {{ name = \"openhands-sdk\" }},\n  {{ name = \"openhands-tools\" }},\n  {{ name = \"openhands-workspace\" }},\n]\n\n[[package]]\nname = \"openhands-agent-server\"\nversion = \"{version}\"\nsource = {{ registry = \"https://pypi.org/simple\" }}\n\n[[package]]\nname = \"openhands-sdk\"\nversion = \"{version}\"\nsource = {{ registry = \"https://pypi.org/simple\" }}\n\n[[package]]\nname = \"openhands-tools\"\nversion = \"{version}\"\nsource = {{ registry = \"https://pypi.org/simple\" }}\n\n[[package]]\nname = \"openhands-workspace\"\nversion = \"{version}\"\nsource = {{ registry = \"https://pypi.org/simple\" }}\n"
        )
    }
}
