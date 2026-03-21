//! Workspace ownership helpers and persistent manifests.

use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Workspace-specific failure modes.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Filesystem access failed.
    #[error("filesystem error at {path}: {source}")]
    Io {
        /// Path tied to the failure.
        path: String,
        /// Original IO error.
        #[source]
        source: std::io::Error,
    },
    /// Serialized data was invalid.
    #[error("failed to parse manifest at {path}: {source}")]
    Json {
        /// Path tied to the failure.
        path: String,
        /// Original JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// Path resolution escaped the configured workspace root.
    #[error("workspace path escaped the configured root: {path}")]
    PathEscape {
        /// Escaping path.
        path: String,
    },
}

/// Resolved OpenSymphony-owned paths within an issue workspace.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceLayout {
    /// Canonical workspace root.
    pub workspace_root: PathBuf,
    /// Sanitized issue workspace path.
    pub issue_workspace: PathBuf,
    /// `.opensymphony` metadata directory.
    pub metadata_dir: PathBuf,
    /// `.opensymphony/prompts`
    pub prompts_dir: PathBuf,
    /// `.opensymphony/logs`
    pub logs_dir: PathBuf,
    /// `.opensymphony/openhands`
    pub openhands_dir: PathBuf,
    /// `.opensymphony/conversation.json`
    pub conversation_manifest_path: PathBuf,
}

impl WorkspaceLayout {
    /// Resolves the deterministic issue workspace path under `workspace_root`.
    pub fn new(workspace_root: impl AsRef<Path>, identifier: &str) -> Result<Self, WorkspaceError> {
        let workspace_root = ensure_dir(workspace_root.as_ref())?;
        let sanitized = sanitize_issue_identifier(identifier);
        if !workspace_key_is_normal_component(&sanitized) {
            return Err(WorkspaceError::PathEscape {
                path: workspace_root.join(&sanitized).display().to_string(),
            });
        }
        let issue_workspace = workspace_root.join(&sanitized);
        let metadata_dir = issue_workspace.join(".opensymphony");
        let prompts_dir = metadata_dir.join("prompts");
        let logs_dir = metadata_dir.join("logs");
        let openhands_dir = metadata_dir.join("openhands");
        let conversation_manifest_path = metadata_dir.join("conversation.json");

        let candidate = issue_workspace.strip_prefix(&workspace_root).map_err(|_| {
            WorkspaceError::PathEscape {
                path: issue_workspace.display().to_string(),
            }
        })?;
        if candidate.components().next().is_none() {
            return Err(WorkspaceError::PathEscape {
                path: issue_workspace.display().to_string(),
            });
        }

        Ok(Self {
            workspace_root,
            issue_workspace,
            metadata_dir,
            prompts_dir,
            logs_dir,
            openhands_dir,
            conversation_manifest_path,
        })
    }

    /// Creates the issue workspace plus owned metadata directories.
    pub fn create(&self) -> Result<(), WorkspaceError> {
        let issue_workspace = ensure_dir(&self.issue_workspace)?;
        ensure_contained(&issue_workspace, &self.workspace_root)?;
        for path in [
            &self.metadata_dir,
            &self.prompts_dir,
            &self.logs_dir,
            &self.openhands_dir,
        ] {
            let resolved = ensure_dir(path)?;
            ensure_contained(&resolved, &issue_workspace)?;
        }
        Ok(())
    }
}

/// Minimal persisted conversation metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConversationManifest {
    /// Stable tracker-side issue ID.
    pub issue_id: String,
    /// Human-facing identifier such as `COE-253`.
    pub identifier: String,
    /// OpenHands conversation ID.
    pub conversation_id: String,
    /// Base URL for the server used when the conversation was created.
    pub server_base_url: String,
    /// Persistence directory inside the issue workspace.
    pub persistence_dir: String,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Latest successful attachment timestamp.
    pub last_attached_at: DateTime<Utc>,
    /// Whether the conversation was freshly created for the current run.
    pub fresh_conversation: bool,
    /// Optional reset reason.
    pub reset_reason: Option<String>,
    /// Runtime contract version tracked by OpenSymphony.
    pub runtime_contract_version: String,
}

impl ConversationManifest {
    /// Reads the manifest if it exists.
    pub fn load(path: impl AsRef<Path>) -> Result<Option<Self>, WorkspaceError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read_to_string(path).map_err(|source| WorkspaceError::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&data)
            .map(Some)
            .map_err(|source| WorkspaceError::Json {
                path: path.display().to_string(),
                source,
            })
    }

    /// Writes the manifest to disk.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), WorkspaceError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            ensure_dir(parent)?;
        }
        let data = serde_json::to_vec_pretty(self).map_err(|source| WorkspaceError::Json {
            path: path.display().to_string(),
            source,
        })?;
        fs::write(path, data).map_err(|source| WorkspaceError::Io {
            path: path.display().to_string(),
            source,
        })
    }
}

/// Stores the last rendered prompt sent by the session runner.
pub fn write_prompt_artifact(path: impl AsRef<Path>, prompt: &str) -> Result<(), WorkspaceError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(path, prompt).map_err(|source| WorkspaceError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Sanitizes an issue identifier into a stable workspace key.
#[must_use]
pub fn sanitize_issue_identifier(identifier: &str) -> String {
    let sanitized: String = identifier
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    normalize_workspace_key(sanitized)
}

fn normalize_workspace_key(key: String) -> String {
    if workspace_key_is_normal_component(&key) {
        return key;
    }

    let normalized: String = key
        .chars()
        .map(|ch| if ch == '.' { '_' } else { ch })
        .collect();
    if workspace_key_is_normal_component(&normalized) {
        normalized
    } else {
        "_".to_string()
    }
}

fn workspace_key_is_normal_component(key: &str) -> bool {
    let mut components = Path::new(key).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn ensure_contained(path: &Path, root: &Path) -> Result<(), WorkspaceError> {
    path.strip_prefix(root)
        .map_err(|_| WorkspaceError::PathEscape {
            path: path.display().to_string(),
        })?;
    Ok(())
}

fn ensure_dir(path: &Path) -> Result<PathBuf, WorkspaceError> {
    fs::create_dir_all(path).map_err(|source| WorkspaceError::Io {
        path: path.display().to_string(),
        source,
    })?;
    path.canonicalize().map_err(|source| WorkspaceError::Io {
        path: path.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn sanitize_replaces_unsafe_characters() {
        assert_eq!(
            sanitize_issue_identifier("Bug: weird/path"),
            "Bug__weird_path"
        );
    }

    #[test]
    fn sanitize_rewrites_dot_only_identifiers() {
        assert_eq!(sanitize_issue_identifier("."), "_");
        assert_eq!(sanitize_issue_identifier(".."), "__");
        assert_eq!(sanitize_issue_identifier(""), "_");
        assert_eq!(sanitize_issue_identifier("..."), "...");
    }

    #[cfg(unix)]
    #[test]
    fn create_rejects_metadata_symlink_escape() {
        let temp_dir = TempDir::new().expect("temp dir should exist");
        let workspace = WorkspaceLayout::new(temp_dir.path(), "COE-253")
            .expect("workspace layout should build");
        std::fs::create_dir_all(&workspace.issue_workspace)
            .expect("issue workspace should be creatable");
        let escape = temp_dir.path().join("escape");
        std::fs::create_dir_all(&escape).expect("escape dir should be creatable");
        symlink(&escape, &workspace.metadata_dir).expect("metadata symlink should be creatable");

        let error = workspace
            .create()
            .expect_err("metadata symlink escaping the workspace must be rejected");
        assert!(matches!(error, WorkspaceError::PathEscape { .. }));
    }
}
