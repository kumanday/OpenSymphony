//! Error types for the OpenHands runtime adapter.

use std::time::Duration;

use thiserror::Error;

use opensymphony_workspace::WorkspaceError;

/// Convenience result alias for the OpenHands adapter.
pub type Result<T> = std::result::Result<T, OpenHandsError>;

/// Stable runtime adapter failures exposed to the rest of the workspace.
#[derive(Debug, Error)]
pub enum OpenHandsError {
    /// The adapter configuration is invalid.
    #[error("invalid OpenHands configuration: {message}")]
    InvalidConfig {
        /// Human-readable validation detail.
        message: String,
    },
    /// Filesystem work failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON serialization or parsing failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// URL parsing or joining failed.
    #[error(transparent)]
    Url(#[from] url::ParseError),
    /// Workspace-owned manifest or path handling failed.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// The HTTP transport failed before the server returned a response.
    #[error("OpenHands HTTP transport error for {method} {url}: {source}")]
    HttpTransport {
        /// HTTP method in use.
        method: String,
        /// Fully resolved request URL.
        url: String,
        /// Underlying transport error.
        #[source]
        source: reqwest::Error,
    },
    /// The server returned a non-success HTTP status.
    #[error("OpenHands HTTP {status} for {method} {url}: {body}")]
    HttpStatus {
        /// HTTP method in use.
        method: String,
        /// Fully resolved request URL.
        url: String,
        /// HTTP status code returned by the server.
        status: reqwest::StatusCode,
        /// Response body captured for diagnostics.
        body: String,
    },
    /// The WebSocket transport failed.
    #[error("OpenHands WebSocket error for {url}: {source}")]
    WebSocket {
        /// Fully resolved socket URL.
        url: String,
        /// Underlying websocket error.
        #[source]
        source: tokio_tungstenite::tungstenite::Error,
    },
    /// The adapter timed out while waiting for a runtime condition.
    #[error("timed out waiting for {operation} after {} ms", .timeout.as_millis())]
    Timeout {
        /// Operation that timed out.
        operation: &'static str,
        /// Timeout that expired.
        timeout: Duration,
    },
    /// The requested conversation is missing.
    #[error("OpenHands conversation not found: {conversation_id}")]
    ConversationNotFound {
        /// Missing conversation identifier.
        conversation_id: String,
    },
    /// A supervised process exited before the adapter could use it.
    #[error("OpenHands server process exited unexpectedly: {message}")]
    ProcessExited {
        /// Short diagnostic text.
        message: String,
    },
    /// A protocol assumption was violated.
    #[error("OpenHands protocol error: {message}")]
    Protocol {
        /// Human-readable detail.
        message: String,
    },
    /// A background task failed to join cleanly.
    #[error("OpenHands background task failed: {message}")]
    Join {
        /// Human-readable join detail.
        message: String,
    },
}

impl From<tokio_tungstenite::tungstenite::Error> for OpenHandsError {
    fn from(source: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket {
            url: "<unknown>".to_string(),
            source,
        }
    }
}
