//! Typed configuration for the OpenHands transport and local supervisor.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{OpenHandsError, Result};
use crate::wire::{AgentConfig, ConfirmationPolicy};

const DEFAULT_READY_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_RECONNECT_INITIAL_MS: u64 = 1_000;
const DEFAULT_RECONNECT_MAX_MS: u64 = 30_000;
const DEFAULT_POLL_INTERVAL_MS: u64 = 1_000;
const DEFAULT_STARTUP_TIMEOUT_MS: u64 = 30_000;

/// Top-level runtime configuration grouped by transport, supervision, and conversation policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpenHandsConfig {
    /// REST and WebSocket endpoint configuration.
    pub transport: TransportConfig,
    /// Optional local server supervision settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_server: Option<LocalServerConfig>,
    /// Conversation creation defaults.
    pub conversation: ConversationConfig,
    /// WebSocket-first attachment behavior.
    #[serde(default)]
    pub websocket: WebSocketConfig,
}

/// HTTP authentication configuration for the server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum HttpAuth {
    /// No REST authentication.
    #[default]
    None,
    /// Send `X-Session-API-Key: <key>` on REST requests.
    SessionApiKey(String),
}

/// WebSocket authentication mode supported by the pinned server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub enum WebSocketAuthMode {
    /// Do not send any WebSocket authentication material.
    None,
    /// Prefer the non-browser `X-Session-API-Key` header.
    #[default]
    Auto,
    /// Always send the session API key as a query parameter.
    QueryParam,
    /// Always send the session API key as a header.
    Header,
}

/// Base transport settings shared by the REST client and the WebSocket stream.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Public base URL for the server.
    pub base_url: Url,
    /// REST authentication behavior.
    #[serde(default)]
    pub http_auth: HttpAuth,
    /// WebSocket authentication behavior.
    #[serde(default)]
    pub websocket_auth: WebSocketAuthMode,
    /// Query parameter used when query-string auth is enabled.
    #[serde(default = "default_session_api_key_query_param")]
    pub websocket_query_param_name: String,
}

impl TransportConfig {
    /// Builds the public server root URL, stripping a trailing `/api` when present.
    fn root_base_url(&self) -> Result<Url> {
        if base_path_has_api_suffix(&self.base_url) {
            strip_path_suffix(&self.base_url, "/api")
        } else {
            Ok(self.base_url.clone())
        }
    }

    /// Returns the configured session API key when one exists.
    #[must_use]
    pub fn session_api_key(&self) -> Option<&str> {
        match &self.http_auth {
            HttpAuth::None => None,
            HttpAuth::SessionApiKey(key) => Some(key.as_str()),
        }
    }

    /// Builds the REST base URL, preserving any configured path prefix.
    pub fn rest_base_url(&self) -> Result<Url> {
        join_url(&self.root_base_url()?, "/api")
    }

    /// Joins a path fragment onto the public root URL.
    pub fn join_root_path(&self, suffix: &str) -> Result<Url> {
        join_url(&self.root_base_url()?, suffix)
    }

    /// Joins a path fragment onto the REST base URL.
    pub fn join_rest_path(&self, suffix: &str) -> Result<Url> {
        join_url(&self.rest_base_url()?, suffix)
    }
}

/// Local supervisor settings for a single shared `openhands-agent-server` subprocess.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalServerConfig {
    /// Full command line used to launch the server.
    pub command: Vec<String>,
    /// Optional working directory for the child process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<PathBuf>,
    /// Extra environment variables applied to the child process.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Time budget for readiness probing after launch.
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    /// Preferred readiness endpoint path.
    #[serde(default = "default_readiness_probe_path")]
    pub readiness_probe_path: String,
}

impl LocalServerConfig {
    /// Validates that a runnable command is present.
    pub fn validate(&self) -> Result<()> {
        if self.command.is_empty() {
            return Err(OpenHandsError::InvalidConfig {
                message: "local server command must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

/// Conversation defaults mirrored into the minimal create request subset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationConfig {
    /// Stable runtime contract label persisted in workspace manifests.
    pub runtime_contract_version: String,
    /// Relative persistence directory recorded in workspace metadata.
    pub persistence_dir_relative: String,
    /// Default OpenHands agent payload.
    pub agent: AgentConfig,
    /// Confirmation behavior sent to the server.
    #[serde(default = "ConfirmationPolicy::never_confirm")]
    pub confirmation_policy: ConfirmationPolicy,
    /// Maximum iterations per run.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    /// Whether the server should enable stuck detection.
    #[serde(default = "default_stuck_detection")]
    pub stuck_detection: bool,
    /// Whether the server should auto-generate a title.
    #[serde(default)]
    pub autotitle: bool,
    /// Optional server-side hook configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_config: Option<serde_json::Value>,
    /// Optional plugins loaded by the server.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<serde_json::Value>,
    /// Optional secrets passed on creation.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secrets: BTreeMap<String, serde_json::Value>,
}

/// WebSocket-first attachment configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// Maximum time to wait for the first `ConversationStateUpdateEvent`.
    #[serde(default = "default_ready_timeout_ms")]
    pub ready_timeout_ms: u64,
    /// Initial reconnect delay used after disconnects.
    #[serde(default = "default_reconnect_initial_ms")]
    pub reconnect_initial_ms: u64,
    /// Maximum reconnect delay.
    #[serde(default = "default_reconnect_max_ms")]
    pub reconnect_max_ms: u64,
    /// REST poll interval used while waiting for a terminal status.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

impl Default for WebSocketConfig {
    fn default() -> Self {
        Self {
            ready_timeout_ms: default_ready_timeout_ms(),
            reconnect_initial_ms: default_reconnect_initial_ms(),
            reconnect_max_ms: default_reconnect_max_ms(),
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

impl WebSocketConfig {
    /// Returns the ready timeout as a standard duration.
    #[must_use]
    pub fn ready_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.ready_timeout_ms)
    }

    /// Returns the initial reconnect delay as a duration.
    #[must_use]
    pub fn reconnect_initial(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.reconnect_initial_ms)
    }

    /// Returns the reconnect cap as a duration.
    #[must_use]
    pub fn reconnect_max(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.reconnect_max_ms)
    }

    /// Returns the REST poll interval as a duration.
    #[must_use]
    pub fn poll_interval(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.poll_interval_ms)
    }
}

fn join_url(base: &Url, suffix: &str) -> Result<Url> {
    let mut url = base.clone();
    let prefix = url.path().trim_end_matches('/');
    let suffix = suffix.trim_start_matches('/');
    let path = if prefix.is_empty() {
        format!("/{suffix}")
    } else if suffix.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}/{suffix}")
    };
    url.set_path(&path);
    Ok(url)
}

fn base_path_has_api_suffix(base: &Url) -> bool {
    base.path().trim_end_matches('/').ends_with("/api")
}

fn strip_path_suffix(base: &Url, suffix: &str) -> Result<Url> {
    let mut url = base.clone();
    let path = url.path().trim_end_matches('/').to_string();
    let stripped = path
        .strip_suffix(suffix)
        .ok_or_else(|| OpenHandsError::InvalidConfig {
            message: format!("URL path does not end with expected suffix {suffix}: {path}"),
        })?;
    url.set_path(if stripped.is_empty() { "/" } else { stripped });
    Ok(url)
}

fn default_session_api_key_query_param() -> String {
    "session_api_key".to_string()
}

fn default_startup_timeout_ms() -> u64 {
    DEFAULT_STARTUP_TIMEOUT_MS
}

fn default_readiness_probe_path() -> String {
    "/ready".to_string()
}

fn default_max_iterations() -> u32 {
    500
}

fn default_stuck_detection() -> bool {
    true
}

fn default_ready_timeout_ms() -> u64 {
    DEFAULT_READY_TIMEOUT_MS
}

fn default_reconnect_initial_ms() -> u64 {
    DEFAULT_RECONNECT_INITIAL_MS
}

fn default_reconnect_max_ms() -> u64 {
    DEFAULT_RECONNECT_MAX_MS
}

fn default_poll_interval_ms() -> u64 {
    DEFAULT_POLL_INTERVAL_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_base_url_adds_api_prefix_once() {
        let root = TransportConfig {
            base_url: Url::parse("https://example.com/runtime/123")
                .expect("static test URL must parse"),
            http_auth: HttpAuth::None,
            websocket_auth: WebSocketAuthMode::Auto,
            websocket_query_param_name: default_session_api_key_query_param(),
        };
        assert_eq!(
            root.rest_base_url()
                .expect("REST base URL should be derivable")
                .as_str(),
            "https://example.com/runtime/123/api"
        );

        let pre_scoped = TransportConfig {
            base_url: Url::parse("https://example.com/runtime/123/api")
                .expect("static test URL must parse"),
            ..root
        };
        assert_eq!(
            pre_scoped
                .rest_base_url()
                .expect("REST base URL should preserve existing /api")
                .as_str(),
            "https://example.com/runtime/123/api"
        );
    }

    #[test]
    fn join_root_path_strips_api_suffix_for_root_endpoints() {
        let transport = TransportConfig {
            base_url: Url::parse("https://example.com/runtime/123/api")
                .expect("static test URL must parse"),
            http_auth: HttpAuth::None,
            websocket_auth: WebSocketAuthMode::Auto,
            websocket_query_param_name: default_session_api_key_query_param(),
        };

        assert_eq!(
            transport
                .join_root_path("/ready")
                .expect("root endpoint should strip /api")
                .as_str(),
            "https://example.com/runtime/123/ready"
        );
        assert_eq!(
            transport
                .join_root_path("/server_info")
                .expect("diagnostic endpoint should strip /api")
                .as_str(),
            "https://example.com/runtime/123/server_info"
        );
    }
}
