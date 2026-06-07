//! Tauri native commands orchestration.
//!
//! Every command uses narrow, strongly-typed request and response structs so
//! that the capability matrix stays auditable and the attack surface is small.

use crate::daemon::{DaemonConfig, DaemonHandle, StartupResult};
use crate::types::{CommandResult, DesktopError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tauri::State;
use tauri::command;
use thiserror::Error;
use tokio::sync::Mutex;
use tracing::warn;

const DEFAULT_GATEWAY_HTTP_URL: &str = "http://127.0.0.1:8000";
const DEFAULT_GATEWAY_HTTP_LOCALHOST_URL: &str = "http://localhost:8000";
const DEFAULT_GATEWAY_WS_URL: &str = "ws://127.0.0.1:8000";

// ─── Executable validation ─────────────────────────────────────────────────

/// Validate that a daemon executable path is safe to run.
///
/// Rejects paths that don't exist, aren't regular files, or lack execute
/// permission on Unix systems. Also rejects world-writable paths to prevent
/// tampering by other local users.
///
/// Note: Group-writable paths are NOT rejected because in a desktop environment,
/// the user's primary group typically contains only that user, so rejecting
/// group-writable paths would break legitimate executables (e.g., `~/bin/`).
///
/// In production deployments, this should be restricted to bundled
/// executables within the app's resource directory.
fn validate_executable_path(path: &PathBuf) -> Result<(), DaemonPathError> {
    if !path.exists() {
        return Err(DaemonPathError::NotFound);
    }

    let metadata = std::fs::metadata(path).map_err(|e| DaemonPathError::AccessDenied {
        detail: e.to_string(),
    })?;
    if !metadata.is_file() {
        return Err(DaemonPathError::NotAFile);
    }

    // On Unix, verify execute permission and reject world-writable paths
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = metadata.permissions();
        let mode = perms.mode();

        // Reject world-writable paths (prevents tampering by any local user)
        if mode & 0o002 != 0 {
            return Err(DaemonPathError::WorldWritable);
        }

        if mode & 0o111 == 0 {
            return Err(DaemonPathError::NotExecutable);
        }
    }

    Ok(())
}

/// Error returned when a daemon executable path fails validation.
#[derive(Error, Debug)]
enum DaemonPathError {
    #[error("daemon executable path does not exist")]
    NotFound,
    #[error("daemon executable path is not a regular file")]
    NotAFile,
    #[error("daemon executable path is not executable")]
    NotExecutable,
    #[error("daemon executable path is world-writable")]
    WorldWritable,
    #[error("daemon executable path cannot be inspected: {detail}")]
    AccessDenied { detail: String },
}

impl DaemonPathError {
    fn kind(&self) -> &'static str {
        match self {
            DaemonPathError::NotFound => "not_found",
            DaemonPathError::NotAFile => "not_a_file",
            DaemonPathError::NotExecutable => "not_executable",
            DaemonPathError::WorldWritable => "world_writable",
            DaemonPathError::AccessDenied { .. } => "access_denied",
        }
    }
}

// ─── Shared desktop state ───────────────────────────────────────────────────

/// Shared application state managed by Tauri.
pub struct DesktopState {
    /// The supervised daemon handle, if any.
    pub daemon_handle: Arc<Mutex<Option<DaemonHandle>>>,
    /// Whether the daemon is currently supervised by this app instance.
    pub daemon_supervised: Arc<AtomicBool>,
}

impl DesktopState {
    pub fn new() -> Self {
        Self {
            daemon_handle: Arc::new(Mutex::new(None)),
            daemon_supervised: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Default for DesktopState {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Connection Profiles ───────────────────────────────────────────────────

/// Connection profile kind discriminant.
#[derive(Debug, Deserialize, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    LocalDaemon,
    SupervisedLocalDaemon,
    EmbeddedHost,
    ExternalGateway,
    HostedGateway,
}

impl ProfileKind {
    fn as_str(&self) -> &'static str {
        match self {
            ProfileKind::LocalDaemon => "local_daemon",
            ProfileKind::SupervisedLocalDaemon => "supervised_local_daemon",
            ProfileKind::EmbeddedHost => "embedded_host",
            ProfileKind::ExternalGateway => "external_gateway",
            ProfileKind::HostedGateway => "hosted_gateway",
        }
    }
}

/// Request to create or update a connection profile.
#[derive(Debug, Deserialize)]
pub struct ProfileRequest {
    pub id: Option<String>,
    pub label: String,
    pub kind: ProfileKind,
    pub gateway_url: String,
    pub daemon_path: Option<String>,
    pub daemon_args: Option<Vec<String>>,
    pub auto_restart: Option<bool>,
    pub startup_timeout_secs: Option<u64>,
}

/// Response with profile details.
#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub gateway_url: String,
    pub managed: bool,
    pub daemon_path: Option<String>,
}

/// Store a connection profile.
#[command]
pub async fn store_profile(_req: ProfileRequest) -> CommandResult<ProfileResponse> {
    // Stub implementation - real persistence will be added in COE-409.
    // Generate a timestamp-based unique ID to prevent collisions when
    // multiple profiles are stored without explicit IDs.
    let profile_id = _req.id.unwrap_or_else(|| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("profile-{}", ts)
    });
    Ok(ProfileResponse {
        id: profile_id,
        label: _req.label,
        kind: _req.kind.as_str().to_string(),
        gateway_url: _req.gateway_url,
        managed: matches!(
            _req.kind,
            ProfileKind::SupervisedLocalDaemon | ProfileKind::EmbeddedHost
        ),
        daemon_path: _req.daemon_path,
    })
}

/// List all stored connection profiles.
#[command]
pub async fn list_profiles() -> CommandResult<Vec<ProfileResponse>> {
    // Real implementation reads from local storage
    Ok(vec![])
}

/// Set the active connection profile.
#[command]
pub async fn set_active_profile(_profile_id: String) -> CommandResult<ProfileResponse> {
    // Real implementation updates active profile in storage
    Err(DesktopError::NotFound)
}

// ─── Gateway Discovery ──────────────────────────────────────────────────────

/// Result of a gateway discovery probe.
#[derive(Debug, Serialize)]
pub struct DiscoveryResult {
    pub healthy: bool,
    pub compatible: bool,
    pub gateway_url: String,
    pub latency_ms: u64,
    pub error: Option<String>,
    pub capabilities: Option<serde_json::Value>,
}

/// Probe a gateway URL for health and capabilities.
#[command]
pub async fn probe_gateway(gateway_url: String) -> CommandResult<DiscoveryResult> {
    let start = std::time::Instant::now();
    let health_url = format!("{}/healthz", gateway_url.trim_end_matches('/'));
    let capabilities_url = format!("{}/api/v1/capabilities", gateway_url.trim_end_matches('/'));

    // Use a client with a timeout to avoid blocking the async runtime
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| DesktopError::Internal {
            message: format!("Failed to build HTTP client: {}", e),
        })?;

    // Probe health
    match client.get(&health_url).send().await {
        Ok(response) if response.status().is_success() => {
            let _health_latency = start.elapsed().as_millis() as u64;

            // Probe capabilities
            match client.get(&capabilities_url).send().await {
                Ok(cap_response) if cap_response.status().is_success() => {
                    let capabilities: Option<serde_json::Value> = cap_response.json().await.ok();
                    let total_latency = start.elapsed().as_millis() as u64;

                    Ok(DiscoveryResult {
                        healthy: true,
                        compatible: true,
                        gateway_url,
                        latency_ms: total_latency,
                        error: None,
                        capabilities,
                    })
                }
                Ok(cap_response) => Ok(DiscoveryResult {
                    healthy: true,
                    compatible: false,
                    gateway_url,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!(
                        "Capabilities endpoint returned {}",
                        cap_response.status()
                    )),
                    capabilities: None,
                }),
                Err(e) => Ok(DiscoveryResult {
                    healthy: true,
                    compatible: false,
                    gateway_url,
                    latency_ms: start.elapsed().as_millis() as u64,
                    error: Some(format!("Capabilities probe failed: {}", e)),
                    capabilities: None,
                }),
            }
        }
        Ok(response) => Ok(DiscoveryResult {
            healthy: false,
            compatible: false,
            gateway_url,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("Health check returned {}", response.status())),
            capabilities: None,
        }),
        Err(e) => Ok(DiscoveryResult {
            healthy: false,
            compatible: false,
            gateway_url,
            latency_ms: start.elapsed().as_millis() as u64,
            error: Some(format!("Health probe failed: {}", e)),
            capabilities: None,
        }),
    }
}

/// Discover gateway on default loopback address.
#[command]
pub async fn discover_default_gateway() -> CommandResult<DiscoveryResult> {
    let default_urls = [DEFAULT_GATEWAY_HTTP_URL, DEFAULT_GATEWAY_HTTP_LOCALHOST_URL];

    for url in &default_urls {
        let result = probe_gateway(url.to_string()).await?;
        if result.healthy && result.compatible {
            return Ok(result);
        }
    }

    // Return last result if none succeeded
    probe_gateway(default_urls[0].to_string()).await
}

// ─── Daemon Supervision ─────────────────────────────────────────────────────

/// Request to start a supervised daemon.
#[derive(Debug, Deserialize)]
pub struct StartDaemonRequest {
    /// Path to the daemon executable.
    pub executable: String,
    /// Arguments to pass to the daemon.
    pub args: Option<Vec<String>>,
    /// Environment variables for the daemon.
    pub env: Option<Vec<(String, String)>>,
    /// Gateway URL where the daemon listens.
    pub gateway_url: Option<String>,
    /// Startup timeout in seconds.
    pub startup_timeout_secs: Option<u64>,
    /// Whether to auto-restart on failure.
    pub auto_restart: Option<bool>,
}

/// Start and supervise a local daemon.
///
/// Acquires the daemon mutex for the entire start sequence to prevent
/// concurrent starts that could orphan processes.
#[command]
pub async fn start_daemon(
    state: State<'_, DesktopState>,
    req: StartDaemonRequest,
) -> CommandResult<StartupResult> {
    // Atomically check-and-start by holding the mutex for the entire operation
    let mut handle_guard = state.daemon_handle.lock().await;

    if handle_guard.is_some() {
        warn!("daemon already supervised, rejecting start request");
        return Err(DesktopError::Internal {
            message: "Daemon already supervised by this instance".to_string(),
        });
    }

    let exec_path = PathBuf::from(&req.executable);

    // Validate executable path for safety
    if let Err(err) = validate_executable_path(&exec_path) {
        warn!(?err, path = ?exec_path, "daemon executable path validation failed");
        return Err(DesktopError::DaemonPath {
            kind: err.kind().to_string(),
            detail: err.to_string(),
        });
    }

    let config = DaemonConfig {
        executable: exec_path,
        args: req.args.unwrap_or_default(),
        env: req.env.unwrap_or_default(),
        startup_timeout: Duration::from_secs(req.startup_timeout_secs.unwrap_or(30)),
        auto_restart: req.auto_restart.unwrap_or(true),
        gateway_url: req
            .gateway_url
            .unwrap_or_else(|| DEFAULT_GATEWAY_HTTP_URL.to_string()),
        skip_health_check: false,
    };

    let mut handle = DaemonHandle::new(config);
    let result = handle.start().await;

    if result.success {
        state.daemon_supervised.store(true, Ordering::SeqCst);
        *handle_guard = Some(handle);
    } else {
        warn!(error = ?result.error, "daemon startup failed");
    }

    Ok(result)
}

/// Stop the supervised daemon.
///
/// Only stops if this app instance owns the process.
#[command]
pub async fn stop_daemon(state: State<'_, DesktopState>) -> CommandResult<serde_json::Value> {
    if !state.daemon_supervised.load(Ordering::SeqCst) {
        return Ok(serde_json::json!({
            "stopped": false,
            "reason": "no daemon supervised"
        }));
    }

    let mut handle_guard = state.daemon_handle.lock().await;
    if let Some(ref mut handle) = *handle_guard {
        match handle.stop().await {
            Ok(()) => {
                state.daemon_supervised.store(false, Ordering::SeqCst);
                *handle_guard = None;
                Ok(serde_json::json!({
                    "stopped": true,
                    "reason": null
                }))
            }
            Err(e) => Ok(serde_json::json!({
                "stopped": false,
                "reason": e.to_string()
            })),
        }
    } else {
        Ok(serde_json::json!({
            "stopped": false,
            "reason": "no daemon handle"
        }))
    }
}

/// Query the status of the supervised daemon.
#[command]
pub async fn daemon_status(state: State<'_, DesktopState>) -> CommandResult<ProcessStatus> {
    let mut handle_guard = state.daemon_handle.lock().await;
    if let Some(ref mut handle) = *handle_guard {
        let is_running = handle.is_running();
        // Derive state string from actual liveness check to avoid stale
        // enum values when the daemon crashes or is killed externally.
        let state_str = if is_running {
            handle.state().as_str().to_string()
        } else {
            "stopped".to_string()
        };
        Ok(ProcessStatus {
            pid: handle.pid(),
            running: is_running,
            state: state_str,
            supervised: state.daemon_supervised.load(Ordering::SeqCst),
        })
    } else {
        Ok(ProcessStatus {
            pid: None,
            running: false,
            state: "stopped".to_string(),
            supervised: false,
        })
    }
}

/// Response for daemon process status.
#[derive(Debug, Serialize)]
pub struct ProcessStatus {
    pub pid: Option<u32>,
    pub running: bool,
    pub state: String,
    pub supervised: bool,
}

// ─── Gateway Transport Commands (COE-410) ───────────────────────────────────
//
// These commands implement the desktop local transport adapter, allowing the
// Tauri webview frontend to communicate with the local OpenSymphony gateway
// using the same GatewayEnvelope and schema types as HTTP/WebSocket transports.
//
// Transport priority (per architecture doc 3.1):
// 1. In-process Rust channels (embedded host) - not available in webview
// 2. Native local IPC (Unix sockets/named pipes) - via loopback fallback
// 3. Tauri channels (this implementation) - high-volume frames to webview
// 4. Loopback HTTP/WebSocket - compatibility baseline

/// Gateway connection state managed by the Tauri app.
#[derive(Debug)]
pub struct GatewayConnection {
    pub base_url: String,
    pub auth_token: Option<String>,
    pub connected: bool,
}

impl Default for GatewayConnection {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_GATEWAY_HTTP_URL.to_string(),
            auth_token: None,
            connected: false,
        }
    }
}

fn gateway_profile_for_url(base_url: &str) -> &'static str {
    let Ok(parsed) = url::Url::parse(base_url) else {
        return "loopback_http";
    };

    let is_loopback = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    };

    match parsed.scheme() {
        "ws" | "wss" if is_loopback => "loopback_websocket",
        "ws" | "wss" => "websocket",
        _ if is_loopback => "loopback_http",
        _ => "websocket",
    }
}

/// Request to attach to a local gateway instance.
#[derive(Debug, Deserialize)]
pub struct AttachGatewayRequest {
    /// Gateway base URL (e.g., "http://127.0.0.1:8000").
    pub base_url: String,
    /// Optional auth token for hosted or secured gateways.
    pub auth_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AttachGatewayResponse {
    pub connected: bool,
    pub profile: String,
}

/// Attach to a local or remote gateway instance.
#[command]
pub async fn attach_gateway(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
    req: AttachGatewayRequest,
) -> CommandResult<AttachGatewayResponse> {
    // Validate URL using proper parser
    let parsed = url::Url::parse(&req.base_url).map_err(|e| DesktopError::Gateway {
        message: format!("Invalid gateway URL: {}", e),
    })?;

    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(DesktopError::Gateway {
            message: "Gateway URL must use http or https scheme".to_string(),
        });
    }

    let profile = gateway_profile_for_url(parsed.as_str());

    // Mutate connection state to record the attachment
    let mut conn = state.write().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    conn.base_url = req.base_url.clone();
    conn.auth_token = req.auth_token.clone();
    conn.connected = true;
    drop(conn);

    Ok(AttachGatewayResponse {
        connected: true,
        profile: profile.to_string(),
    })
}

/// Get dashboard snapshot from gateway.
#[command]
pub async fn dashboard_snapshot(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
) -> CommandResult<serde_json::Value> {
    // Read attached gateway URL to prove state is not dead (COE-404 will wire
    // actual gateway calls using this value).
    let conn = state.read().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "projects": [],
        "runs": [],
        "events": [],
        "base_url": conn.base_url.clone(),
    }))
}

/// Get task graph for a project.
#[command]
pub async fn task_graph(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
    project_id: String,
) -> CommandResult<serde_json::Value> {
    let conn = state.read().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "project_id": project_id,
        "nodes": [],
        "root_ids": [],
        "base_url": conn.base_url.clone(),
    }))
}

/// Get run details.
#[command]
pub async fn run_detail(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
    run_id: String,
) -> CommandResult<serde_json::Value> {
    let conn = state.read().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "run_id": run_id,
        "status": "idle",
        "events": [],
        "base_url": conn.base_url.clone(),
    }))
}

/// Get run events with cursor support.
#[command]
pub async fn run_events(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
    run_id: String,
    cursor: Option<u64>,
    page_size: Option<u64>,
) -> CommandResult<serde_json::Value> {
    let conn = state.read().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "run_id": run_id,
        "cursor": cursor.unwrap_or(0),
        "page_size": page_size.unwrap_or(100),
        "events": [],
        "has_more": false,
        "base_url": conn.base_url.clone(),
    }))
}

/// Get terminal snapshot.
#[command]
pub async fn terminal_snapshot(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
    run_id: String,
    terminal_id: String,
) -> CommandResult<serde_json::Value> {
    let conn = state.read().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    Ok(serde_json::json!({
        "schema_version": {"major": 1, "minor": 0, "patch": 0},
        "run_id": run_id,
        "terminal_id": terminal_id,
        "content": "",
        "cursor": 0,
        "base_url": conn.base_url.clone(),
    }))
}

/// Connection profile for local gateway discovery.
#[derive(Debug, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub name: String,
    pub profile_type: String,
    pub base_url: String,
    pub auth_mode: String,
    pub available: bool,
}

/// Get available connection profiles for the desktop app.
#[command]
pub async fn get_connection_profiles() -> CommandResult<Vec<ConnectionProfile>> {
    Ok(vec![
        ConnectionProfile {
            name: "Local Daemon".to_string(),
            profile_type: "loopback_http".to_string(),
            base_url: DEFAULT_GATEWAY_HTTP_URL.to_string(),
            auth_mode: "none".to_string(),
            available: true,
        },
        ConnectionProfile {
            name: "Local Gateway (WebSocket)".to_string(),
            profile_type: "loopback_websocket".to_string(),
            base_url: DEFAULT_GATEWAY_WS_URL.to_string(),
            auth_mode: "none".to_string(),
            available: true,
        },
        ConnectionProfile {
            name: "Tauri Native".to_string(),
            profile_type: "tauri_channel".to_string(),
            base_url: "tauri://local".to_string(),
            auth_mode: "none".to_string(),
            available: true,
        },
    ])
}

// ─── Gateway Local Stream Transport (COE-410) ──────────────────────────────

use crate::opensymphony_gateway_schema::{
    capability::{
        AuthMode, FeatureCapability as GatewayFeatureCapability, GatewayCapabilities,
        TransportCapability as GatewayTransportCapability,
    },
    envelope::GatewayEnvelope,
    version::SchemaVersion,
};

/// Request to subscribe to the gateway event stream via Tauri channel.
#[derive(Debug, Deserialize)]
pub struct SubscribeEventsRequest {
    /// Optional cursor to resume from (sequence number).
    pub cursor: Option<u64>,
    /// Optional cursor partition to resume within.
    pub partition: Option<String>,
}

/// Request to subscribe to terminal frames for a specific run.
#[derive(Debug, Deserialize)]
pub struct SubscribeTerminalRequest {
    pub run_id: String,
    /// Optional cursor to resume from (sequence number).
    pub cursor: Option<u64>,
}

/// Gateway health status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum GatewayHealthStatus {
    #[serde(rename = "healthy")]
    Healthy,
    #[serde(rename = "degraded")]
    Degraded,
    #[serde(rename = "unavailable")]
    Unavailable,
}

/// Local gateway connection info.
#[derive(Debug, Serialize)]
pub struct GatewayConnectionInfo {
    pub status: GatewayHealthStatus,
    pub profile: String,
    pub base_uri: String,
    pub transports: Vec<String>,
}

/// Query gateway capabilities.
/// Used by the frontend transport factory to select the optimal profile.
#[command]
pub async fn gateway_capabilities() -> CommandResult<GatewayCapabilities> {
    Ok(GatewayCapabilities {
        schema_version: SchemaVersion::v1(),
        gateway_version: env!("CARGO_PKG_VERSION").to_string(),
        supported_api_versions: vec!["1.0.0".to_string()],
        transports: vec![
            GatewayTransportCapability {
                transport: "tauri_channel".to_string(),
                modes: vec!["json".to_string()],
                supported_encodings: vec!["utf-8".to_string()],
                bidirectional: true,
            },
            GatewayTransportCapability {
                transport: "loopback_http".to_string(),
                modes: vec!["json".to_string()],
                supported_encodings: vec!["utf-8".to_string()],
                bidirectional: false,
            },
            GatewayTransportCapability {
                transport: "loopback_websocket".to_string(),
                modes: vec!["json".to_string(), "binary".to_string()],
                supported_encodings: vec!["utf-8".to_string(), "base64".to_string()],
                bidirectional: true,
            },
        ],
        features: vec![
            GatewayFeatureCapability {
                feature: "task_graph".to_string(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            GatewayFeatureCapability {
                feature: "terminal_stream".to_string(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
        ],
        auth_modes: vec![AuthMode::None, AuthMode::ApiKey],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    })
}

/// Query the local gateway health and connection info.
#[command]
pub async fn gateway_connection_info(
    state: tauri::State<'_, std::sync::RwLock<GatewayConnection>>,
) -> CommandResult<GatewayConnectionInfo> {
    let conn = state.read().map_err(|e| DesktopError::Gateway {
        message: format!("Failed to acquire connection state lock: {}", e),
    })?;
    let status = if conn.connected {
        GatewayHealthStatus::Healthy
    } else {
        GatewayHealthStatus::Degraded
    };
    let base_uri = conn.base_url.clone();
    let profile = gateway_profile_for_url(&base_uri).to_string();
    drop(conn);

    Ok(GatewayConnectionInfo {
        status,
        profile,
        base_uri,
        transports: vec![
            "tauri_channel".to_string(),
            "loopback_http".to_string(),
            "loopback_websocket".to_string(),
        ],
    })
}

/// Subscribe to the gateway event stream via a Tauri channel.
///
/// This provides a high-throughput, zero-copy path from the local gateway
/// to the webview frontend. The channel carries GatewayEnvelope instances
/// that are identical in structure to those delivered via HTTP/SSE or
/// WebSocket transports.
///
/// The caller provides a `tauri::ipc::Channel` through which envelopes
/// are streamed. This enables backpressure handling and avoids the
/// HTTP/WebSocket overhead for local desktop mode.
#[command]
pub async fn subscribe_events(
    _req: SubscribeEventsRequest,
    _tx: tauri::ipc::Channel<GatewayEnvelope>,
    _state: tauri::State<'_, SubscriptionState>,
) -> CommandResult<()> {
    _state
        .event_subscribers
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // COE-409 will wire this to the actual gateway event stream.
    // The channel transport enables high-throughput local delivery.
    Ok(())
}

/// Subscribe to terminal frames for a specific run via a Tauri channel.
///
/// Terminal frames are high-volume and benefit from the zero-copy-friendly
/// Rust frame buffer path. This command establishes a dedicated channel
/// for terminal streaming.
#[command]
pub async fn subscribe_terminal(
    _req: SubscribeTerminalRequest,
    _tx: tauri::ipc::Channel<GatewayEnvelope>,
    _state: tauri::State<'_, SubscriptionState>,
) -> CommandResult<()> {
    _state
        .terminal_subscribers
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // COE-409 will wire this to the actual gateway terminal stream.
    Ok(())
}

/// Active subscriptions tracked for cleanup.
/// COE-409 will wire this to actual gateway subscription management.
#[derive(Debug, Default)]
pub struct SubscriptionState {
    pub event_subscribers: AtomicUsize,
    pub terminal_subscribers: AtomicUsize,
}

fn decrement_subscription_count(counter: &AtomicUsize) -> usize {
    let prev = counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            Some(current.saturating_sub(1))
        })
        .unwrap_or(0);
    prev.saturating_sub(1)
}

/// Unsubscribe from the gateway event stream.
/// Clean up the channel and release resources.
#[command]
pub async fn unsubscribe_events(_state: tauri::State<'_, SubscriptionState>) -> CommandResult<()> {
    eprintln!(
        "unsubscribe_events: {} remaining subscribers",
        decrement_subscription_count(&_state.event_subscribers)
    );
    Ok(())
}

/// Unsubscribe from terminal frame streaming.
#[command]
pub async fn unsubscribe_terminal(
    _run_id: String,
    _state: tauri::State<'_, SubscriptionState>,
) -> CommandResult<()> {
    eprintln!(
        "unsubscribe_terminal({}): {} remaining subscribers",
        _run_id,
        decrement_subscription_count(&_state.terminal_subscribers)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrement_subscription_count_saturates_at_zero() {
        let counter = AtomicUsize::new(0);

        assert_eq!(decrement_subscription_count(&counter), 0);
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        counter.store(2, Ordering::Relaxed);
        assert_eq!(decrement_subscription_count(&counter), 1);
        assert_eq!(counter.load(Ordering::Relaxed), 1);
        assert_eq!(decrement_subscription_count(&counter), 0);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        assert_eq!(decrement_subscription_count(&counter), 0);
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn gateway_profile_for_url_matches_loopback_and_remote_urls() {
        assert_eq!(
            gateway_profile_for_url("http://127.0.0.1:8000"),
            "loopback_http"
        );
        assert_eq!(
            gateway_profile_for_url("ws://localhost:8000"),
            "loopback_websocket"
        );
        assert_eq!(gateway_profile_for_url("https://example.com"), "websocket");
    }
}
