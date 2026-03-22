use std::{io, path::PathBuf, time::Duration};

use serde_json::Error as JsonError;

use crate::HookKind;

#[derive(Debug)]
pub struct WorkspaceOwnershipConflictDetails {
    pub workspace: PathBuf,
    pub workspace_key: String,
    pub existing_issue_id: String,
    pub existing_identifier: String,
    pub requested_issue_id: String,
    pub requested_identifier: String,
}

impl std::fmt::Display for WorkspaceOwnershipConflictDetails {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "workspace {} with sanitized key {} is already owned by issue {} ({}), cannot reuse it for {} ({})",
            self.workspace.display(),
            self.workspace_key,
            self.existing_identifier,
            self.existing_issue_id,
            self.requested_identifier,
            self.requested_issue_id
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("workspace root must be absolute: {path}")]
    RootNotAbsolute { path: PathBuf },
    #[error("issue identifier cannot be empty")]
    EmptyIdentifier,
    #[error("sanitized workspace key is invalid or reserved: {key}")]
    InvalidWorkspaceKey { key: String },
    #[error("{details}")]
    WorkspaceOwnershipConflict {
        details: Box<WorkspaceOwnershipConflictDetails>,
    },
    #[error("path {path} escapes configured root {root}")]
    PathEscape { root: PathBuf, path: PathBuf },
    #[error("issue workspace path may not be a symlink: {path}")]
    WorkspacePathSymlink { path: PathBuf },
    #[error("OpenSymphony-managed path may not be a symlink: {path}")]
    ManagedPathSymlink { path: PathBuf },
    #[error("failed to create directory {path}: {source}")]
    CreateDirectory { path: PathBuf, source: io::Error },
    #[error("failed to canonicalize {path}: {source}")]
    Canonicalize { path: PathBuf, source: io::Error },
    #[error("failed to read manifest {path}: {source}")]
    ReadManifest { path: PathBuf, source: io::Error },
    #[error("failed to decode manifest {path}: {source}")]
    DecodeManifest { path: PathBuf, source: JsonError },
    #[error("failed to encode manifest {path}: {source}")]
    EncodeManifest { path: PathBuf, source: JsonError },
    #[error("failed to write manifest {path}: {source}")]
    WriteManifest { path: PathBuf, source: io::Error },
    #[error("failed to write artifact {path}: {source}")]
    WriteArtifact { path: PathBuf, source: io::Error },
    #[error("failed to launch hook `{hook}` in {cwd}: {source}")]
    LaunchHook {
        hook: HookKind,
        cwd: PathBuf,
        source: io::Error,
    },
    #[error("hook `{hook}` cwd {cwd} escapes workspace {workspace}")]
    HookPathEscape {
        hook: HookKind,
        workspace: PathBuf,
        cwd: PathBuf,
    },
    #[error("hook `{hook}` timed out after {timeout:?}: {command}")]
    HookTimedOut {
        hook: HookKind,
        command: String,
        timeout: Duration,
    },
    #[error("hook `{hook}` failed with exit code {exit_code:?}: {command}")]
    HookFailed {
        hook: HookKind,
        command: String,
        exit_code: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("failed to remove workspace {path}: {source}")]
    RemoveWorkspace { path: PathBuf, source: io::Error },
}
