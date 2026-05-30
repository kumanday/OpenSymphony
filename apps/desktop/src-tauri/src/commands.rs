//! Tauri native commands exposed to the frontend.
//!
//! Every command uses narrow, strongly-typed request and response structs so
//! that the capability matrix stays auditable and the attack surface is small.
//!
//! Fields are stubbed and unused until COE-404/COE-409 implement real backends.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use tauri::command;
use thiserror::Error;

// ─── Error type ─────────────────────────────────────────────────────────────

/// Structured error type returned by desktop native commands.
/// Replaces opaque `String` errors so the frontend can distinguish
/// permission denied, not found, cancelled, and internal failure.
///
/// Uses internally-tagged serialization so every variant produces a uniform
/// JSON shape: `{"type":"Cancelled"}`, `{"type":"Internal","message":"..."}`.
#[derive(Error, Debug, Serialize)]
#[serde(tag = "type")]
pub enum DesktopError {
    /// The user cancelled the operation (e.g., closed a file picker).
    #[error("operation cancelled")]
    Cancelled,
    /// The requested resource does not exist.
    #[error("resource not found")]
    NotFound,
    /// Insufficient permissions to perform the operation.
    #[error("permission denied")]
    PermissionDenied,
    /// The local daemon is not running.
    #[error("daemon unavailable")]
    DaemonUnavailable,
    /// Generic internal error with a human-readable message.
    #[error("internal error: {message}")]
    Internal { message: String },
}

/// Alias for ergonomic command return types.
type CommandResult<T> = Result<T, DesktopError>;

// ─── File / Folder Selection ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct OpenFileRequest {
    /// Human-readable title shown in the native dialog.
    pub title: Option<String>,
    /// Allowed MIME types (empty means all).
    pub accepts: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct OpenFileResponse {
    /// Absolute path chosen by the user, or `None` on cancel.
    pub path: Option<String>,
}

/// Stub: open a single-file picker dialog.
#[command]
pub async fn open_file(_req: OpenFileRequest) -> CommandResult<OpenFileResponse> {
    // Real implementation uses `tauri_plugin_dialog::ask` / `open`.
    Ok(OpenFileResponse { path: None })
}

#[derive(Debug, Serialize)]
pub struct OpenFolderResponse {
    pub path: Option<String>,
}

/// Stub: open a folder picker dialog.
#[command]
pub async fn open_folder(_title: Option<String>) -> CommandResult<OpenFolderResponse> {
    Ok(OpenFolderResponse { path: None })
}

// ─── Notification ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct NotifyRequest {
    pub title: String,
    pub body: String,
    /// Optional severity hint.
    pub level: Option<NotifyLevel>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Serialize)]
pub struct NotifyResponse {
    pub acknowledged: bool,
}

/// Stub: request a native OS notification.
#[command]
pub async fn notify(_req: NotifyRequest) -> CommandResult<NotifyResponse> {
    // Real implementation uses `tauri_plugin_notification::Notification`.
    Ok(NotifyResponse {
        acknowledged: false,
    })
}

// ─── Settings (local, non-secret) ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct GetSettingRequest {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum SettingValue {
    Text(String),
    Flag(bool),
    Number(f64),
}

#[derive(Debug, Serialize)]
pub struct GetSettingResponse {
    pub value: Option<SettingValue>,
}

/// Stub: read a local setting by key.
#[command]
pub async fn get_setting(_req: GetSettingRequest) -> CommandResult<GetSettingResponse> {
    Ok(GetSettingResponse { value: None })
}

#[derive(Debug, Deserialize)]
pub struct SetSettingRequest {
    pub key: String,
    pub value: SettingValue,
}

#[derive(Debug, Serialize)]
pub struct SetSettingResponse {
    pub persisted: bool,
}

/// Stub: write a local setting by key.
#[command]
pub async fn set_setting(_req: SetSettingRequest) -> CommandResult<SetSettingResponse> {
    Ok(SetSettingResponse { persisted: false })
}

// ─── Local Process Supervision (stencil only) ───────────────────────────────

/// The shell plugin is loaded at minimal baseline (`shell:default`) to allow
/// future process supervision without requiring a capability redesign.
/// `shell:default` grants only the `open` helper (launch default app for a URL/path).
/// No `shell:execute` or `shell:kill` permissions are active.
/// COE-404 will implement whitelisted executable paths, PID tracking, and
/// input sanitization before any execute/kill permissions are added.

#[derive(Debug, Serialize)]
pub struct ProcessStatus {
    pub pid: Option<u32>,
    pub running: bool,
}

/// Stub: query whether the locally-supervised daemon process is running.
#[command]
pub async fn daemon_status() -> CommandResult<ProcessStatus> {
    // COE-404 will implement actual discovery + supervision.
    Ok(ProcessStatus {
        pid: None,
        running: false,
    })
}
