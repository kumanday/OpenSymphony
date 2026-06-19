//! Tauri native commands orchestration.
//!
//! Every command uses narrow, strongly-typed request and response structs so
//! that the capability matrix stays auditable and the attack surface is small.

use crate::daemon::{DaemonConfig, DaemonHandle, StartupResult};
use crate::types::{CommandResult, DesktopError};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tauri::State;
use tauri::command;
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

const DEFAULT_GATEWAY_HTTP_URL: &str = "http://127.0.0.1:2468";
const DEFAULT_GATEWAY_HTTP_LOCALHOST_URL: &str = "http://localhost:2468";
const LEGACY_DEFAULT_GATEWAY_HTTP_URLS: &[&str] =
    &["http://127.0.0.1:8000", "http://localhost:8000"];

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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProfileResponse {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub gateway_url: String,
    pub transport: String,
    pub managed: bool,
    pub active: bool,
    pub daemon_path: Option<String>,
    pub daemon_args: Vec<String>,
    pub auto_restart: bool,
    pub startup_timeout_secs: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct ProfileStore {
    profiles: Vec<ProfileResponse>,
    active_profile_id: Option<String>,
}

/// Store a connection profile.
#[command]
pub async fn store_profile(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    req: ProfileRequest,
) -> CommandResult<ProfileResponse> {
    validate_profile_gateway_url(&req.gateway_url)?;
    let stored_response = {
        let _guard = profile_store_lock().lock().await;
        let path = profile_store_path()?;
        let mut store = load_profile_store_async(path.clone()).await?;
        normalize_profile_store_without_default(&mut store);

        let profile_id = req.id.clone().unwrap_or_else(new_profile_id);
        let was_active = store
            .active_profile_id
            .as_ref()
            .is_some_and(|active_id| active_id == &profile_id);
        let make_active =
            was_active || store.active_profile_id.is_none() || store.profiles.is_empty();
        let response = profile_response_from_request(req, profile_id.clone(), make_active);

        store.profiles.retain(|profile| profile.id != profile_id);
        store.profiles.push(response.clone());
        if make_active {
            store.active_profile_id = Some(profile_id);
        }
        normalize_profile_store_without_default(&mut store);
        let active_id = store.active_profile_id.clone();
        let stored_response = store
            .profiles
            .iter()
            .find(|profile| profile.id == response.id)
            .map(|profile| with_profile_active(profile, active_id.as_deref()))
            .unwrap_or_else(|| with_profile_active(&response, active_id.as_deref()));
        save_profile_store_async(path, store).await?;
        stored_response
    };

    if stored_response.active {
        update_gateway_connection(&state, stored_response.gateway_url.clone(), None).await;
    }

    Ok(stored_response)
}

/// List all stored connection profiles.
#[command]
pub async fn list_profiles() -> CommandResult<Vec<ProfileResponse>> {
    let _guard = profile_store_lock().lock().await;
    let path = profile_store_path()?;
    let store = load_profile_store_async(path).await?;
    Ok(normalized_profiles_for_read(store))
}

/// Set the active connection profile.
#[command]
pub async fn set_active_profile(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    profile_id: String,
) -> CommandResult<ProfileResponse> {
    let active_profile = {
        let _guard = profile_store_lock().lock().await;
        let path = profile_store_path()?;
        let mut store = load_profile_store_async(path.clone()).await?;
        normalize_profile_store(&mut store);

        let Some(profile) = store
            .profiles
            .iter()
            .find(|candidate| candidate.id == profile_id)
            .cloned()
        else {
            return Err(DesktopError::NotFound);
        };

        store.active_profile_id = Some(profile_id);
        normalize_profile_store(&mut store);
        let active_id = store.active_profile_id.clone();
        save_profile_store_async(path, store).await?;
        with_profile_active(&profile, active_id.as_deref())
    };

    update_gateway_connection(&state, active_profile.gateway_url.clone(), None).await;

    Ok(active_profile)
}

fn new_profile_id() -> String {
    static PROFILE_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let nonce = PROFILE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("profile-{ts}-{nonce}")
}

fn profile_response_from_request(
    req: ProfileRequest,
    profile_id: String,
    active: bool,
) -> ProfileResponse {
    let managed = matches!(
        req.kind,
        ProfileKind::SupervisedLocalDaemon | ProfileKind::EmbeddedHost
    );
    let transport = profile_transport(&req.kind, &req.gateway_url);
    ProfileResponse {
        id: profile_id,
        label: req.label,
        kind: req.kind.as_str().to_string(),
        gateway_url: req.gateway_url,
        transport,
        managed,
        active,
        daemon_path: req.daemon_path,
        daemon_args: req.daemon_args.unwrap_or_default(),
        auto_restart: req.auto_restart.unwrap_or(false),
        startup_timeout_secs: req.startup_timeout_secs.unwrap_or(30),
    }
}

fn profile_transport(kind: &ProfileKind, gateway_url: &str) -> String {
    match kind {
        ProfileKind::EmbeddedHost => "loopback_http".to_string(),
        ProfileKind::HostedGateway => "websocket".to_string(),
        ProfileKind::ExternalGateway
        | ProfileKind::LocalDaemon
        | ProfileKind::SupervisedLocalDaemon => gateway_profile_for_url(gateway_url).to_string(),
    }
}

fn validate_profile_gateway_url(gateway_url: &str) -> CommandResult<()> {
    let parsed = url::Url::parse(gateway_url).map_err(|e| DesktopError::Gateway {
        message: format!("Invalid gateway URL: {e}"),
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(DesktopError::Gateway {
            message: "Gateway URL must use http or https scheme".to_string(),
        });
    }
    Ok(())
}

fn default_profile() -> ProfileResponse {
    ProfileResponse {
        id: "local-daemon".to_string(),
        label: "Local Daemon".to_string(),
        kind: ProfileKind::LocalDaemon.as_str().to_string(),
        gateway_url: DEFAULT_GATEWAY_HTTP_URL.to_string(),
        transport: "loopback_http".to_string(),
        managed: false,
        active: true,
        daemon_path: None,
        daemon_args: vec![],
        auto_restart: false,
        startup_timeout_secs: 30,
    }
}

fn profile_store_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn profile_store_path() -> CommandResult<PathBuf> {
    let project_dirs =
        directories::ProjectDirs::from("dev", "opensymphony", "app").ok_or_else(|| {
            DesktopError::Settings {
                message: "could not determine project directories".to_string(),
            }
        })?;
    Ok(project_dirs.config_dir().join("profiles.json"))
}

async fn load_profile_store_async(path: PathBuf) -> CommandResult<ProfileStore> {
    tokio::task::spawn_blocking(move || load_profile_store(&path))
        .await
        .map_err(|e| DesktopError::Internal {
            message: format!("profile store load task failed: {e}"),
        })?
}

async fn save_profile_store_async(path: PathBuf, store: ProfileStore) -> CommandResult<()> {
    tokio::task::spawn_blocking(move || save_profile_store(&path, &store))
        .await
        .map_err(|e| DesktopError::Internal {
            message: format!("profile store save task failed: {e}"),
        })?
}

fn load_profile_store(path: &Path) -> CommandResult<ProfileStore> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content).map_err(|e| DesktopError::Settings {
            message: format!("failed to parse profile store at {}: {e}", path.display()),
        }),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(ProfileStore::default()),
        Err(e) => Err(DesktopError::Settings {
            message: format!("failed to read profile store at {}: {e}", path.display()),
        }),
    }
}

fn save_profile_store(path: &Path, store: &ProfileStore) -> CommandResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| DesktopError::Settings {
            message: format!("failed to create profile store directory: {e}"),
        })?;
    }
    let content = serde_json::to_string_pretty(store).map_err(|e| DesktopError::Settings {
        message: format!("failed to serialize profile store: {e}"),
    })?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).map_err(|e| DesktopError::Settings {
        message: format!("failed to write profile store: {e}"),
    })?;
    fs::rename(&tmp, path).map_err(|e| DesktopError::Settings {
        message: format!("failed to persist profile store: {e}"),
    })?;
    Ok(())
}

fn normalize_profile_store(store: &mut ProfileStore) {
    if store.profiles.is_empty() {
        store.profiles.push(default_profile());
    }
    normalize_profile_store_without_default(store);
}

fn normalize_profile_store_without_default(store: &mut ProfileStore) {
    migrate_legacy_default_gateway_profiles(store);

    let active_exists = store.active_profile_id.as_ref().is_some_and(|active_id| {
        store
            .profiles
            .iter()
            .any(|profile| profile.id.as_str() == active_id)
    });
    if !active_exists {
        store.active_profile_id = store.profiles.first().map(|profile| profile.id.clone());
    }
    let active_id = store.active_profile_id.as_deref();
    for profile in &mut store.profiles {
        profile.active = active_id.is_some_and(|id| id == profile.id.as_str());
        profile.transport = profile_transport_for_response(profile);
    }
}

fn migrate_legacy_default_gateway_profiles(store: &mut ProfileStore) {
    for profile in &mut store.profiles {
        if is_legacy_default_gateway_profile(profile) {
            profile.gateway_url = DEFAULT_GATEWAY_HTTP_URL.to_string();
        }
    }
}

fn is_legacy_default_gateway_profile(profile: &ProfileResponse) -> bool {
    profile.kind == ProfileKind::LocalDaemon.as_str()
        && is_legacy_default_gateway_url(&profile.gateway_url)
        && matches!(
            profile.label.as_str(),
            "Local Gateway" | "Local Daemon" | "Local daemon"
        )
}

fn is_legacy_default_gateway_url(gateway_url: &str) -> bool {
    let normalized = gateway_url.trim_end_matches('/');
    LEGACY_DEFAULT_GATEWAY_HTTP_URLS.contains(&normalized)
}

fn normalized_profiles_for_read(mut store: ProfileStore) -> Vec<ProfileResponse> {
    normalize_profile_store(&mut store);
    profiles_with_active(&store)
}

fn profile_transport_for_response(profile: &ProfileResponse) -> String {
    match profile.kind.as_str() {
        "hosted_gateway" => "websocket".to_string(),
        "embedded_host" => "loopback_http".to_string(),
        _ => gateway_profile_for_url(&profile.gateway_url).to_string(),
    }
}

fn profiles_with_active(store: &ProfileStore) -> Vec<ProfileResponse> {
    let active_id = store.active_profile_id.as_deref();
    store
        .profiles
        .iter()
        .map(|profile| with_profile_active(profile, active_id))
        .collect()
}

fn with_profile_active(profile: &ProfileResponse, active_id: Option<&str>) -> ProfileResponse {
    let mut profile = profile.clone();
    profile.active = active_id.is_some_and(|id| id == profile.id.as_str());
    profile
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

    let client = gateway_http_client();

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
    pub client: reqwest::Client,
}

impl Default for GatewayConnection {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_GATEWAY_HTTP_URL.to_string(),
            auth_token: None,
            connected: false,
            client: gateway_http_client(),
        }
    }
}

async fn update_gateway_connection(
    state: &tauri::State<'_, RwLock<GatewayConnection>>,
    base_url: String,
    auth_token: Option<String>,
) {
    let mut conn = state.write().await;
    conn.base_url = base_url;
    conn.auth_token = auth_token;
    conn.connected = false;
}

async fn set_gateway_connected_for_url(
    state: &tauri::State<'_, RwLock<GatewayConnection>>,
    base_url: &str,
    connected: bool,
) {
    let mut conn = state.write().await;
    if conn.base_url == base_url {
        conn.connected = connected;
    }
}

fn gateway_http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("gateway HTTP client configuration should be valid")
        })
        .clone()
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

    match (parsed.scheme(), is_loopback) {
        ("http" | "https", true) => "loopback_http",
        ("http" | "https", false) => "websocket",
        _ => "loopback_http",
    }
}

async fn gateway_get_json(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    path: &str,
) -> CommandResult<serde_json::Value> {
    let (base_url, auth_token, client) = {
        let conn = state.read().await;
        (
            conn.base_url.clone(),
            conn.auth_token.clone(),
            conn.client.clone(),
        )
    };

    let parsed = match url::Url::parse(&base_url) {
        Ok(parsed) => parsed,
        Err(e) => {
            set_gateway_connected_for_url(&state, &base_url, false).await;
            return Err(DesktopError::Gateway {
                message: format!("Invalid gateway URL: {e}"),
            });
        }
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        set_gateway_connected_for_url(&state, &base_url, false).await;
        return Err(DesktopError::Gateway {
            message: "Gateway URL must use http or https scheme".to_string(),
        });
    }
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let mut request = client.get(&url);
    if let Some(token) = auth_token {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            set_gateway_connected_for_url(&state, &base_url, false).await;
            return Err(DesktopError::Gateway {
                message: format!("Gateway request failed for {path}: {e}"),
            });
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        set_gateway_connected_for_url(&state, &base_url, false).await;
        return Err(DesktopError::Gateway {
            message: format!("Gateway returned {status} for {path}: {body}"),
        });
    }
    let value = match response.json::<serde_json::Value>().await {
        Ok(value) => value,
        Err(e) => {
            set_gateway_connected_for_url(&state, &base_url, false).await;
            return Err(DesktopError::Gateway {
                message: format!("Gateway returned invalid JSON for {path}: {e}"),
            });
        }
    };
    set_gateway_connected_for_url(&state, &base_url, true).await;
    Ok(value)
}

/// Request to attach to a local gateway instance.
#[derive(Debug, Deserialize)]
pub struct AttachGatewayRequest {
    /// Gateway base URL (e.g., "http://127.0.0.1:2468").
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
    state: tauri::State<'_, RwLock<GatewayConnection>>,
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

    update_gateway_connection(&state, req.base_url.clone(), req.auth_token.clone()).await;

    Ok(AttachGatewayResponse {
        connected: false,
        profile: profile.to_string(),
    })
}

/// Get dashboard snapshot from gateway.
#[command]
pub async fn dashboard_snapshot(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(state, "/api/v1/dashboard/snapshot").await
}

/// Get task graph for a project.
#[command]
pub async fn task_graph(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    project_id: String,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(
        state,
        &format!(
            "/api/v1/projects/{}/taskgraph",
            urlencoding::encode(&project_id)
        ),
    )
    .await
}

/// Get run details.
#[command]
pub async fn run_detail(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(
        state,
        &format!("/api/v1/runs/{}", urlencoding::encode(&run_id)),
    )
    .await
}

/// Get changed files for a run.
#[command]
pub async fn run_files(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(
        state,
        &format!("/api/v1/runs/{}/files", urlencoding::encode(&run_id)),
    )
    .await
}

/// Get a diff page for a run.
#[command]
pub async fn run_diffs(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
    file_path: Option<String>,
) -> CommandResult<serde_json::Value> {
    let mut path = format!("/api/v1/runs/{}/diffs", urlencoding::encode(&run_id));
    if let Some(file_path) = file_path.filter(|path| !path.is_empty()) {
        path.push_str("?file_path=");
        path.push_str(&urlencoding::encode(&file_path));
    }
    gateway_get_json(state, &path).await
}

/// Get run validation summary.
#[command]
pub async fn run_validation(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(
        state,
        &format!("/api/v1/runs/{}/validation", urlencoding::encode(&run_id)),
    )
    .await
}

/// Get pending run approvals.
#[command]
pub async fn run_approvals(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(
        state,
        &format!("/api/v1/runs/{}/approvals", urlencoding::encode(&run_id)),
    )
    .await
}

/// Get run events with cursor support.
#[command]
pub async fn run_events(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
    page_token: Option<String>,
    page_size: Option<u64>,
) -> CommandResult<serde_json::Value> {
    let mut path = format!("/api/v1/runs/{}/events", urlencoding::encode(&run_id));
    let mut params = Vec::new();
    if let Some(page_token) = page_token {
        params.push(format!(
            "page_token={}",
            urlencoding::encode(&page_token)
        ));
    }
    if let Some(page_size) = page_size {
        params.push(format!("page_size={page_size}"));
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    gateway_get_json(state, &path).await
}

/// Get terminal snapshot.
#[command]
pub async fn terminal_snapshot(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
    run_id: String,
    terminal_id: String,
) -> CommandResult<serde_json::Value> {
    gateway_get_json(
        state,
        &format!(
            "/api/v1/runs/{}/terminals/{}/snapshot",
            urlencoding::encode(&run_id),
            urlencoding::encode(&terminal_id)
        ),
    )
    .await
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
pub async fn get_connection_profiles(
    state: tauri::State<'_, RwLock<GatewayConnection>>,
) -> CommandResult<Vec<ConnectionProfile>> {
    let profiles = list_profiles().await?;
    let (active_base_url, gateway_connected) = {
        let conn = state.read().await;
        (conn.base_url.clone(), conn.connected)
    };
    let mut available_profiles: Vec<ConnectionProfile> = profiles
        .into_iter()
        .map(|profile| {
            let available =
                profile_is_current_gateway_available(&profile, &active_base_url, gateway_connected);
            ConnectionProfile {
                name: profile.label,
                profile_type: profile.transport,
                base_url: profile.gateway_url,
                auth_mode: "none".to_string(),
                available,
            }
        })
        .collect();
    available_profiles.push(ConnectionProfile {
        name: "Tauri Native".to_string(),
        profile_type: "tauri_channel".to_string(),
        base_url: "tauri://local".to_string(),
        auth_mode: "none".to_string(),
        available: false,
    });
    Ok(available_profiles)
}

fn profile_is_current_gateway_available(
    profile: &ProfileResponse,
    active_base_url: &str,
    gateway_connected: bool,
) -> bool {
    gateway_connected
        && profile.active
        && profile.gateway_url.trim_end_matches('/') == active_base_url.trim_end_matches('/')
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
        transports: vec![GatewayTransportCapability {
            transport: "loopback_http".to_string(),
            modes: vec!["json".to_string()],
            supported_encodings: vec!["utf-8".to_string()],
            bidirectional: false,
        }],
        features: vec![
            GatewayFeatureCapability {
                feature: "task_graph".to_string(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
            GatewayFeatureCapability {
                feature: "terminal_stream".to_string(),
                available: false,
                requires_auth: false,
                requires_plan: None,
            },
            GatewayFeatureCapability {
                feature: "tauri_channel".to_string(),
                available: false,
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
    state: tauri::State<'_, RwLock<GatewayConnection>>,
) -> CommandResult<GatewayConnectionInfo> {
    let conn = state.read().await;
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
        transports: vec!["loopback_http".to_string()],
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
    Err(DesktopError::Gateway {
        message:
            "Tauri channel event streams are not available; use loopback HTTP/WebSocket transport"
                .to_string(),
    })
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
    Err(DesktopError::Gateway {
        message: "Tauri channel terminal streams are not available; use loopback HTTP/WebSocket transport".to_string(),
    })
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
            gateway_profile_for_url("http://127.0.0.1:2468"),
            "loopback_http"
        );
        assert_eq!(
            gateway_profile_for_url("ws://localhost:2468"),
            "loopback_http"
        );
        assert_eq!(gateway_profile_for_url("https://example.com"), "websocket");
    }

    #[tokio::test]
    async fn gateway_capabilities_advertises_http_only_native_transport() {
        let capabilities = gateway_capabilities().await.unwrap();
        let transports: Vec<&str> = capabilities
            .transports
            .iter()
            .map(|transport| transport.transport.as_str())
            .collect();

        assert_eq!(transports, vec!["loopback_http"]);
    }

    #[test]
    fn normalized_profiles_for_read_does_not_persist_default_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let profiles = normalized_profiles_for_read(ProfileStore::default());

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].id, "local-daemon");
        assert!(!path.exists());
    }

    #[test]
    fn normalize_profile_store_without_default_keeps_empty_store_empty() {
        let mut store = ProfileStore::default();

        normalize_profile_store_without_default(&mut store);

        assert!(store.profiles.is_empty());
        assert!(store.active_profile_id.is_none());
    }

    #[test]
    fn normalize_profile_store_migrates_legacy_local_gateway_default_port() {
        let mut store = ProfileStore {
            profiles: vec![
                ProfileResponse {
                    id: "local-daemon".to_string(),
                    label: "Local Gateway".to_string(),
                    kind: ProfileKind::LocalDaemon.as_str().to_string(),
                    gateway_url: "http://127.0.0.1:8000".to_string(),
                    transport: "loopback_http".to_string(),
                    managed: false,
                    active: true,
                    daemon_path: None,
                    daemon_args: vec![],
                    auto_restart: false,
                    startup_timeout_secs: 30,
                },
                ProfileResponse {
                    id: "custom-local".to_string(),
                    label: "Custom Local 8000".to_string(),
                    kind: ProfileKind::LocalDaemon.as_str().to_string(),
                    gateway_url: "http://127.0.0.1:8000".to_string(),
                    transport: "loopback_http".to_string(),
                    managed: false,
                    active: false,
                    daemon_path: None,
                    daemon_args: vec![],
                    auto_restart: false,
                    startup_timeout_secs: 30,
                },
                ProfileResponse {
                    id: "external".to_string(),
                    label: "Local Gateway".to_string(),
                    kind: ProfileKind::ExternalGateway.as_str().to_string(),
                    gateway_url: "http://127.0.0.1:8000".to_string(),
                    transport: "loopback_http".to_string(),
                    managed: false,
                    active: false,
                    daemon_path: None,
                    daemon_args: vec![],
                    auto_restart: false,
                    startup_timeout_secs: 30,
                },
            ],
            active_profile_id: Some("local-daemon".to_string()),
        };

        normalize_profile_store(&mut store);

        assert_eq!(store.profiles[0].gateway_url, DEFAULT_GATEWAY_HTTP_URL);
        assert_eq!(store.profiles[1].gateway_url, "http://127.0.0.1:8000");
        assert_eq!(store.profiles[2].gateway_url, "http://127.0.0.1:8000");
        assert!(store.profiles[0].active);
    }

    #[test]
    fn profile_availability_requires_active_connected_matching_gateway() {
        let active = default_profile();
        let mut inactive = active.clone();
        inactive.active = false;

        assert!(profile_is_current_gateway_available(
            &active,
            DEFAULT_GATEWAY_HTTP_URL,
            true
        ));
        assert!(!profile_is_current_gateway_available(
            &inactive,
            DEFAULT_GATEWAY_HTTP_URL,
            true
        ));
        assert!(!profile_is_current_gateway_available(
            &active,
            DEFAULT_GATEWAY_HTTP_LOCALHOST_URL,
            true
        ));
        assert!(!profile_is_current_gateway_available(
            &active,
            DEFAULT_GATEWAY_HTTP_URL,
            false
        ));
    }

    #[test]
    fn profile_store_normalizes_defaults_and_persists_active_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        let mut store = ProfileStore::default();

        normalize_profile_store(&mut store);

        assert_eq!(store.profiles.len(), 1);
        assert_eq!(store.active_profile_id.as_deref(), Some("local-daemon"));
        assert!(store.profiles[0].active);

        let profile = profile_response_from_request(
            ProfileRequest {
                id: Some("external-dev".to_string()),
                label: "External Dev".to_string(),
                kind: ProfileKind::ExternalGateway,
                gateway_url: "http://localhost:9000".to_string(),
                daemon_path: None,
                daemon_args: None,
                auto_restart: None,
                startup_timeout_secs: None,
            },
            "external-dev".to_string(),
            true,
        );
        store.profiles.push(profile);
        store.active_profile_id = Some("external-dev".to_string());
        normalize_profile_store(&mut store);
        save_profile_store(&path, &store).unwrap();

        let mut loaded = load_profile_store(&path).unwrap();
        normalize_profile_store(&mut loaded);
        let profiles = profiles_with_active(&loaded);

        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().any(|candidate| {
            candidate.id == "external-dev"
                && candidate.active
                && candidate.gateway_url == "http://localhost:9000"
        }));
        assert!(
            profiles
                .iter()
                .any(|candidate| { candidate.id == "local-daemon" && !candidate.active })
        );
    }

    #[test]
    fn validate_profile_gateway_url_rejects_non_http_schemes() {
        assert!(validate_profile_gateway_url("http://127.0.0.1:2468").is_ok());
        assert!(validate_profile_gateway_url("https://gateway.example").is_ok());
        assert!(validate_profile_gateway_url("ws://localhost:2468").is_err());
        assert!(validate_profile_gateway_url("wss://gateway.example").is_err());
        assert!(validate_profile_gateway_url("tauri://local").is_err());
    }

    #[test]
    fn load_profile_store_reports_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");
        fs::write(&path, "{not json").unwrap();

        let err = load_profile_store(&path).unwrap_err();

        assert!(matches!(
            err,
            DesktopError::Settings { message }
                if message.contains("failed to parse profile store")
        ));
    }

    #[test]
    fn new_profile_id_adds_process_unique_nonce() {
        let mut ids = std::collections::HashSet::new();

        for _ in 0..1000 {
            assert!(ids.insert(new_profile_id()));
        }
    }

    #[tokio::test]
    async fn profile_store_lock_serializes_concurrent_async_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.json");

        let write_profile = |id: &'static str| {
            let path = path.clone();
            async move {
                let _guard = profile_store_lock().lock().await;
                let mut store = load_profile_store_async(path.clone()).await.unwrap();
                normalize_profile_store(&mut store);
                let profile = profile_response_from_request(
                    ProfileRequest {
                        id: Some(id.to_string()),
                        label: format!("Profile {id}"),
                        kind: ProfileKind::ExternalGateway,
                        gateway_url: format!("http://localhost:{}", 9000 + id.len()),
                        daemon_path: None,
                        daemon_args: None,
                        auto_restart: None,
                        startup_timeout_secs: None,
                    },
                    id.to_string(),
                    false,
                );
                store.profiles.retain(|existing| existing.id != id);
                store.profiles.push(profile);
                normalize_profile_store(&mut store);
                save_profile_store_async(path, store).await.unwrap();
            }
        };

        let ((), ()) = tokio::join!(write_profile("external-one"), write_profile("external-two"));
        let mut loaded = load_profile_store(&path).unwrap();
        normalize_profile_store(&mut loaded);

        assert!(
            loaded
                .profiles
                .iter()
                .any(|profile| profile.id == "external-one")
        );
        assert!(
            loaded
                .profiles
                .iter()
                .any(|profile| profile.id == "external-two")
        );
    }
}
