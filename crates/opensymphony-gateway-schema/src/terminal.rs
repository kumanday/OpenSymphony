use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

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
}
