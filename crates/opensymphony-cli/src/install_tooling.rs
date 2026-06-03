use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitStatus,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::opensymphony_openhands::LocalServerTooling;
use tempfile::NamedTempFile;
use thiserror::Error;
use tokio::process::Command;

pub(crate) const DEFAULT_MANAGED_OPENHANDS_TOOL_DIR: &str = "~/.opensymphony/openhands-server";

const OPENHANDS_VERSION: &str = include_str!("../../../tools/openhands-server/version.txt");

struct EmbeddedToolingFile {
    relative_path: &'static str,
    contents: &'static [u8],
    executable: bool,
}

const EMBEDDED_OPENHANDS_FILES: &[EmbeddedToolingFile] = &[
    EmbeddedToolingFile {
        relative_path: ".python-version",
        contents: include_bytes!("../../../tools/openhands-server/.python-version"),
        executable: false,
    },
    EmbeddedToolingFile {
        relative_path: "README.md",
        contents: include_bytes!("../../../tools/openhands-server/README.md"),
        executable: false,
    },
    EmbeddedToolingFile {
        relative_path: "install.sh",
        contents: include_bytes!("../../../tools/openhands-server/install.sh"),
        executable: true,
    },
    EmbeddedToolingFile {
        relative_path: "pyproject.toml",
        contents: include_bytes!("../../../tools/openhands-server/pyproject.toml"),
        executable: false,
    },
    EmbeddedToolingFile {
        relative_path: "run-local.sh",
        contents: include_bytes!("../../../tools/openhands-server/run-local.sh"),
        executable: true,
    },
    EmbeddedToolingFile {
        relative_path: "uv.lock",
        contents: include_bytes!("../../../tools/openhands-server/uv.lock"),
        executable: false,
    },
    EmbeddedToolingFile {
        relative_path: "version.txt",
        contents: include_bytes!("../../../tools/openhands-server/version.txt"),
        executable: false,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolingInstallAction {
    Ready,
    Installed,
    Updated,
    Repaired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolingInstallReport {
    pub(crate) action: ToolingInstallAction,
    pub(crate) tool_dir: PathBuf,
    pub(crate) version: String,
}

impl ToolingInstallReport {
    pub(crate) fn summary(&self) -> String {
        match self.action {
            ToolingInstallAction::Ready => format!(
                "pinned OpenHands tooling {} is already available at {}",
                self.version,
                self.tool_dir.display()
            ),
            ToolingInstallAction::Installed => format!(
                "installed pinned OpenHands tooling {} at {}",
                self.version,
                self.tool_dir.display()
            ),
            ToolingInstallAction::Updated => format!(
                "updated pinned OpenHands tooling {} at {}",
                self.version,
                self.tool_dir.display()
            ),
            ToolingInstallAction::Repaired => format!(
                "repaired pinned OpenHands tooling {} at {}",
                self.version,
                self.tool_dir.display()
            ),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum InstallToolingError {
    #[error("HOME or USERPROFILE must be set to resolve {display_path}")]
    MissingHomeDirectory { display_path: &'static str },
    #[error("failed to create {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to prepare temporary file in {path}: {source}")]
    CreateTempFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to persist {path}: {source}")]
    PersistFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to set executable permissions on {path}: {source}")]
    SetPermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to launch {path}: {source}")]
    LaunchInstaller {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("bundled OpenHands installer {path} failed with {status}: {detail}")]
    InstallerFailed {
        path: PathBuf,
        status: String,
        detail: String,
    },
}

pub(crate) async fn ensure_openhands_tooling(
    tool_dir: &Path,
) -> Result<ToolingInstallReport, InstallToolingError> {
    let action = current_install_action(tool_dir);
    if matches!(action, ToolingInstallAction::Ready) {
        return Ok(ToolingInstallReport {
            action,
            tool_dir: tool_dir.to_path_buf(),
            version: embedded_openhands_version().to_string(),
        });
    }

    materialize_embedded_tooling(tool_dir, action)?;
    prepare_openhands_tooling(tool_dir).await?;
    Ok(ToolingInstallReport {
        action,
        tool_dir: tool_dir.to_path_buf(),
        version: embedded_openhands_version().to_string(),
    })
}

pub(crate) fn default_managed_openhands_tool_dir() -> Result<PathBuf, InstallToolingError> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or(InstallToolingError::MissingHomeDirectory {
            display_path: DEFAULT_MANAGED_OPENHANDS_TOOL_DIR,
        })?;
    Ok(home.join(".opensymphony").join("openhands-server"))
}

pub(crate) fn embedded_openhands_version() -> &'static str {
    OPENHANDS_VERSION.trim()
}

fn current_install_action(tool_dir: &Path) -> ToolingInstallAction {
    match LocalServerTooling::load(tool_dir) {
        Ok(tooling)
            if tooling.pin_status.is_ready() && tooling.version == embedded_openhands_version() =>
        {
            ToolingInstallAction::Ready
        }
        Ok(tooling) if tooling.pin_status.is_ready() => {
            debug_assert_ne!(tooling.version, embedded_openhands_version());
            ToolingInstallAction::Updated
        }
        _ if tool_dir.exists() => ToolingInstallAction::Repaired,
        _ => ToolingInstallAction::Installed,
    }
}

fn materialize_embedded_tooling(
    tool_dir: &Path,
    _action: ToolingInstallAction,
) -> Result<(), InstallToolingError> {
    fs::create_dir_all(tool_dir).map_err(|source| InstallToolingError::CreateDir {
        path: tool_dir.to_path_buf(),
        source,
    })?;

    for asset in EMBEDDED_OPENHANDS_FILES {
        let destination = tool_dir.join(asset.relative_path);
        let parent = destination.parent().unwrap_or(tool_dir);
        fs::create_dir_all(parent).map_err(|source| InstallToolingError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;

        let mut temp_file = NamedTempFile::new_in(parent).map_err(|source| {
            InstallToolingError::CreateTempFile {
                path: parent.to_path_buf(),
                source,
            }
        })?;
        temp_file
            .write_all(asset.contents)
            .map_err(|source| InstallToolingError::WriteFile {
                path: destination.clone(),
                source,
            })?;
        temp_file
            .flush()
            .map_err(|source| InstallToolingError::WriteFile {
                path: destination.clone(),
                source,
            })?;
        apply_file_permissions(temp_file.as_file(), &destination, asset.executable)?;

        temp_file
            .persist(&destination)
            .map_err(|source| InstallToolingError::PersistFile {
                path: destination.clone(),
                source: source.error,
            })?;
    }

    Ok(())
}

#[cfg(unix)]
fn apply_file_permissions(
    file: &fs::File,
    path: &Path,
    executable: bool,
) -> Result<(), InstallToolingError> {
    let mut permissions = file
        .metadata()
        .map_err(|source| InstallToolingError::SetPermissions {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    permissions.set_mode(if executable { 0o755 } else { 0o644 });
    file.set_permissions(permissions)
        .map_err(|source| InstallToolingError::SetPermissions {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn apply_file_permissions(
    _file: &std::fs::File,
    _path: &Path,
    _executable: bool,
) -> Result<(), InstallToolingError> {
    Ok(())
}

async fn prepare_openhands_tooling(tool_dir: &Path) -> Result<(), InstallToolingError> {
    let installer = tool_dir.join("install.sh");
    let output = Command::new("bash")
        .arg(&installer)
        .current_dir(tool_dir)
        .output()
        .await
        .map_err(|source| InstallToolingError::LaunchInstaller {
            path: installer.clone(),
            source,
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(InstallToolingError::InstallerFailed {
        path: installer,
        status: render_status(output.status),
        detail: render_command_output(&output.stdout, &output.stderr),
    })
}

fn render_status(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| status.to_string())
}

fn render_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();

    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout: {stdout}; stderr: {stderr}"),
        (false, true) => format!("stdout: {stdout}"),
        (true, false) => format!("stderr: {stderr}"),
        (true, true) => "no output".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf};

    use tempfile::TempDir;

    use super::{
        DEFAULT_MANAGED_OPENHANDS_TOOL_DIR, EMBEDDED_OPENHANDS_FILES,
        default_managed_openhands_tool_dir, embedded_openhands_version,
        materialize_embedded_tooling,
    };

    #[test]
    fn default_tool_dir_uses_home() {
        let tool_dir = default_managed_openhands_tool_dir().expect("tool dir should resolve");
        let expected_home = env::var_os("HOME")
            .or_else(|| env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .expect("test environment should expose a home directory");

        assert_eq!(
            tool_dir,
            expected_home.join(".opensymphony/openhands-server")
        );
        assert_eq!(
            DEFAULT_MANAGED_OPENHANDS_TOOL_DIR,
            "~/.opensymphony/openhands-server"
        );
    }

    #[test]
    fn embedded_files_match_repo_tooling_source() {
        let temp_dir = TempDir::new().expect("temp dir");
        materialize_embedded_tooling(temp_dir.path(), super::ToolingInstallAction::Installed)
            .expect("tooling should materialize");

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_tooling_root = if manifest_dir.join("tools/openhands-server").is_dir() {
            manifest_dir.join("tools/openhands-server")
        } else {
            manifest_dir.join("../../tools/openhands-server")
        };
        if !repo_tooling_root.is_dir() {
            return;
        }

        for asset in EMBEDDED_OPENHANDS_FILES {
            if asset.relative_path == "README.md" {
                continue;
            }
            let embedded = std::fs::read(temp_dir.path().join(asset.relative_path))
                .expect("embedded file should exist");
            let repo_copy = std::fs::read(repo_tooling_root.join(asset.relative_path))
                .expect("repo tooling source should exist");
            assert_eq!(
                embedded, repo_copy,
                "embedded asset {} should match the repo tooling source",
                asset.relative_path
            );
        }

        assert_eq!(embedded_openhands_version(), "1.24.0");
    }
}
