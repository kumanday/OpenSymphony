use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// Association context for a terminal/log frame so every frame can be traced
/// back to its run, workspace, command, issue, and sub-issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TerminalLogAssociation {
    pub run_id: String,
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_issue_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_session_id: Option<String>,
}

/// Terminal or log frame delivered over a high-volume stream.
///
/// Supports both text and binary payloads. Binary frames are base64-encoded
/// in JSON mode; WebSocket binary mode sends raw bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalFrame {
    pub schema_version: SchemaVersion,
    pub frame_sequence: u64,
    pub stream_id: String,
    pub run_id: String,
    pub terminal_session_id: String,
    pub frame_kind: TerminalFrameKind,
    pub encoding: TerminalEncoding,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub association: TerminalLogAssociation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalFrameKind {
    Stdout,
    Stderr,
    Log,
    Prompt,
    Status,
    EndOfStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalEncoding {
    Utf8,
    Base64,
}

/// Terminal snapshot for `/api/v1/runs/{run_id}/terminal/{terminal_id}/snapshot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub schema_version: SchemaVersion,
    pub terminal_session_id: String,
    pub run_id: String,
    pub frames: Vec<TerminalFrame>,
    pub total_frames: u64,
    pub truncated: bool,
    pub cursor: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<TerminalSession>,
}

/// Metadata for a terminal/log session associated with a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSession {
    pub schema_version: SchemaVersion,
    pub terminal_session_id: String,
    pub run_id: String,
    pub association: TerminalLogAssociation,
    pub frame_count: u64,
    pub total_bytes: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_cursor: u64,
}
