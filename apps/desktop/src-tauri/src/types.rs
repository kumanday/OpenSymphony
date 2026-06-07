//! Shared types for desktop commands.

use serde::Serialize;
use thiserror::Error;

/// Structured error type returned by desktop native commands.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "type")]
pub enum DesktopError {
    /// The requested resource does not exist.
    #[error("resource not found")]
    NotFound,
    /// Insufficient permissions to perform the operation.
    #[error("permission denied")]
    PermissionDenied,
    /// Daemon executable path validation failed with a specific reason.
    #[error("daemon path error ({kind}): {detail}")]
    DaemonPath { kind: String, detail: String },
    /// Gateway command failed.
    #[error("gateway error: {message}")]
    Gateway { message: String },
    /// Generic internal error with a human-readable message.
    #[error("internal error: {message}")]
    Internal { message: String },
    /// Keychain error with a non-leaking message.
    #[error("keychain error: {message}")]
    Keychain { message: String },
    /// Settings persistence error with a non-leaking message.
    #[error("settings error: {message}")]
    Settings { message: String },
}

pub type CommandResult<T> = Result<T, DesktopError>;
