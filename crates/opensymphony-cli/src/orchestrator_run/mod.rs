pub(crate) mod backends;
pub(super) mod config;
mod snapshot;

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::Duration,
};

#[cfg(any(
    not(unix),
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd"
))]
use std::process::Command;

use crate::opensymphony_control::{RecentEvent, RecentEventKind, SnapshotStore};
use crate::opensymphony_domain::{InMemoryEventJournal, StreamBroker, TimestampMs};
use crate::opensymphony_gateway::{GatewayServer, LinearTaskGraphClient};
use crate::opensymphony_gateway_schema::event_journal::{EventActor, EventKind, EventRecord};
use crate::opensymphony_linear::LinearError;
use crate::opensymphony_openhands::{OpenHandsError, TransportConfig};
use crate::opensymphony_orchestrator::{
    IssueStateCategory, OrchestratorSnapshot, Scheduler, SchedulerConfig, SchedulerError,
    TrackerBackend, WorkerBackend, WorkspaceBackend,
};
use crate::opensymphony_workflow::ProcessEnvironment;
use crate::opensymphony_workspace::WorkspaceError;
use chrono::{DateTime, Utc};
use clap::Args;
use serde::Deserialize;
use thiserror::Error;
use tokio::{
    net::TcpListener,
    time::{MissedTickBehavior, interval},
};
use tracing::{info, warn};

use self::{
    backends::{
        ManagedLocalPreparation, RuntimeWorkerBackend, RuntimeWorkspaceBackend,
        build_linear_client, build_runtime_transport, build_tracker_backend,
        build_workspace_manager_config_with_retention, prepare_active_conversation_store,
    },
    config::{
        RunRuntimeConfig, looks_like_central_config, resolve_runtime_config, select_config_path,
    },
    snapshot::{
        current_agent_server_status, current_memory_server_status, map_snapshot, push_recent_event,
        terminal_state_set,
    },
};

#[derive(Debug, Args, Clone)]
pub struct RunArgs {
    #[arg(help = "Runtime config YAML path; defaults to ./config.yaml when present")]
    #[arg(long)]
    pub config: Option<PathBuf>,
    #[arg(
        long,
        help = "Preview selected harness/model routing without launching model-backed workers"
    )]
    pub dry_run: bool,
}

#[derive(Debug, Error)]
pub(crate) enum RunCommandError {
    #[error("failed to determine the current working directory: {0}")]
    CurrentDir(#[source] io::Error),
    #[error("failed to acquire configured runtime root ownership: {detail}")]
    RootOwnership { detail: String },
    #[error("failed to acquire the central strict-run marker {path}: {detail}")]
    StrictRunMarker { path: PathBuf, detail: String },
    #[error("failed to read {path}: {source}")]
    ReadConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to expand {path}: {detail}")]
    ResolveConfig { path: PathBuf, detail: String },
    #[error("central config validation failed: {0}")]
    CentralConfig(#[from] config::CentralConfigError),
    #[error(
        "strict multi-repository routing is disabled until its release gates pass (config generation {generation})"
    )]
    StrictRoutingDisabled { generation: String },
    #[error("invalid control-plane bind address `{value}`: {source}")]
    InvalidBind {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("failed to load workflow {path}: {source}")]
    LoadWorkflow {
        path: PathBuf,
        #[source]
        source: crate::opensymphony_workflow::WorkflowLoadError,
    },
    #[error("failed to resolve workflow {path}: {source}")]
    ResolveWorkflow {
        path: PathBuf,
        #[source]
        source: crate::opensymphony_workflow::WorkflowConfigError,
    },
    #[error(
        "memory auto-capture is enabled but {path} is missing; run `opensymphony memory init` or `opensymphony update` from the target repo before `opensymphony run`"
    )]
    MissingMemoryConfig { path: PathBuf },
    #[error("failed to build tracker client: {0}")]
    Tracker(#[from] LinearError),
    #[error("failed to create workspace manager: {0}")]
    WorkspaceManager(#[from] WorkspaceError),
    #[error("failed to prepare OpenHands transport: {0}")]
    Transport(#[from] OpenHandsError),
    #[error("failed to prepare OpenHands conversation store: {0}")]
    ConversationStore(#[from] crate::opensymphony_openhands::ConversationStoreError),
    #[error(
        "managed local OpenHands tooling at {tool_dir} is missing or invalid: {detail}. Run `opensymphony install openhands` or `opensymphony doctor --config <path>`."
    )]
    ToolingSetupRequired { tool_dir: PathBuf, detail: String },
    #[error("failed to start local OpenHands supervisor: {0}")]
    Supervisor(#[from] crate::opensymphony_openhands::SupervisorError),
    #[error("failed to start memory server: {0}")]
    MemoryServer(#[from] crate::opensymphony_memory::MemoryError),
    #[error("failed to build scheduler configuration: {0}")]
    SchedulerConfig(#[from] SchedulerError),
    #[error("failed to bind control-plane listener: {0}")]
    BindListener(#[source] io::Error),
    #[error("control-plane server exited unexpectedly: {0}")]
    Serve(#[source] io::Error),
    #[error(
        "workflow config requires a managed local OpenHands server, but `openhands.tool_dir` is missing from config.yaml (recommended: ~/.opensymphony/openhands-server)"
    )]
    MissingToolDir,
    #[error(
        "OpenHands transport URL `{value}` does not include an explicit port and has no default port"
    )]
    MissingTransportPort { value: String },
    #[error("failed to mint Linear OAuth token: {0}")]
    LinearOAuthToken(String),
}

#[derive(Debug)]
pub(crate) struct RuntimeRootOwnership {
    locks: Vec<RuntimeRootLock>,
}

#[derive(Debug)]
struct RuntimeRootLock {
    marker: PathBuf,
    registry_marker: PathBuf,
    _file: File,
}

struct RootOwnershipSerialization {
    path: PathBuf,
}

static ATOMIC_MARKER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Publish a fully initialized marker without ever exposing an empty final
/// path.  A hard link is used instead of rename so an existing owner cannot
/// be replaced by a concurrent claimant.
pub(crate) fn publish_initialized_marker(path: &Path, contents: &str) -> io::Result<File> {
    let sequence = ATOMIC_MARKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let staging = path.with_file_name(format!(
        ".{}.staging-{}-{sequence}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("marker"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)?;
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(&staging);
        return Err(error);
    }
    match fs::hard_link(&staging, path) {
        Ok(()) => {
            let _ = fs::remove_file(staging);
            Ok(file)
        }
        Err(error) => {
            let _ = fs::remove_file(staging);
            Err(error)
        }
    }
}

impl Drop for RootOwnershipSerialization {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl Drop for RuntimeRootOwnership {
    fn drop(&mut self) {
        for RuntimeRootLock {
            marker,
            registry_marker,
            _file,
        } in self.locks.drain(..)
        {
            drop(_file);
            let _ = fs::remove_file(marker);
            let _ = fs::remove_file(registry_marker);
        }
    }
}

fn acquire_runtime_root_ownership(
    runtime: &RunRuntimeConfig,
) -> Result<RuntimeRootOwnership, RunCommandError> {
    let mut roots = vec![runtime.workflow.config.workspace.root.clone()];
    if let Some(state_root) = &runtime.state_root {
        roots.push(state_root.clone());
    }
    acquire_root_ownership(roots)
}

pub(crate) fn acquire_root_ownership(
    roots: impl IntoIterator<Item = PathBuf>,
) -> Result<RuntimeRootOwnership, RunCommandError> {
    static ROOT_OWNERSHIP_SERIALIZATION: OnceLock<Mutex<()>> = OnceLock::new();
    let _serialization_guard = ROOT_OWNERSHIP_SERIALIZATION
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| RunCommandError::RootOwnership {
            detail: "runtime root ownership serialization was poisoned".to_owned(),
        })?;
    // ponytail: one host-wide file lock serializes cross-process claims; per-root
    // handshakes would add more failure states without improving this local MVP.
    let _filesystem_serialization = acquire_root_ownership_serialization()?;
    let mut canonical_roots = BTreeSet::new();
    for root in roots {
        fs::create_dir_all(&root).map_err(|source| RunCommandError::RootOwnership {
            detail: format!("failed to create {}: {source}", root.display()),
        })?;
        let root = fs::canonicalize(&root).map_err(|source| RunCommandError::RootOwnership {
            detail: format!("failed to resolve {}: {source}", root.display()),
        })?;
        canonical_roots.insert(root);
    }

    let canonical_roots_vec = canonical_roots.iter().collect::<Vec<_>>();
    for (index, root) in canonical_roots_vec.iter().enumerate() {
        for other in canonical_roots_vec.iter().skip(index + 1) {
            if root.starts_with(other) || other.starts_with(*root) {
                return Err(RunCommandError::RootOwnership {
                    detail: format!(
                        "configured roots {} and {} overlap",
                        root.display(),
                        other.display()
                    ),
                });
            }
        }
    }

    let mut ownership = RuntimeRootOwnership {
        locks: Vec::with_capacity(canonical_roots.len()),
    };
    for root in canonical_roots {
        let mut ancestor = root.parent();
        while let Some(path) = ancestor {
            let marker = path.join(".opensymphony-instance.lock");
            if root_marker_blocks(&marker) {
                return Err(RunCommandError::RootOwnership {
                    detail: format!(
                        "{} is nested below the owned root {}",
                        root.display(),
                        path.display()
                    ),
                });
            }
            ancestor = path.parent();
        }
        if let Some((_, owned_root)) = find_live_registered_root_marker(&root, &BTreeSet::new())? {
            return Err(RunCommandError::RootOwnership {
                detail: format!(
                    "{} contains the owned nested root {}",
                    root.display(),
                    owned_root.display()
                ),
            });
        }
        let marker = root.join(".opensymphony-instance.lock");
        let file = loop {
            match initialize_root_marker_atomic(&marker) {
                Ok(file) => break file,
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    if root_lock_owner_alive(&marker) {
                        return Err(RunCommandError::RootOwnership {
                            detail: format!("{} is already owned by another run", root.display()),
                        });
                    }
                    let stale_marker = root.join(format!(
                        ".opensymphony-instance.lock.stale-{}-{}",
                        std::process::id(),
                        Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    ));
                    match fs::rename(&marker, &stale_marker) {
                        Ok(()) => {
                            let _ = fs::remove_file(stale_marker);
                        }
                        Err(rename_error) if rename_error.kind() == io::ErrorKind::NotFound => {}
                        Err(rename_error) => {
                            return Err(RunCommandError::RootOwnership {
                                detail: format!(
                                    "failed to reclaim stale {}: {rename_error}",
                                    marker.display()
                                ),
                            });
                        }
                    }
                }
                Err(source) => {
                    return Err(RunCommandError::RootOwnership {
                        detail: format!("failed to lock {}: {source}", root.display()),
                    });
                }
            }
        };
        let registry_marker = match claim_root_registry_marker(&root) {
            Ok(marker) => marker,
            Err(error) => {
                drop(file);
                let _ = fs::remove_file(&marker);
                return Err(error);
            }
        };
        ownership.locks.push(RuntimeRootLock {
            marker,
            registry_marker,
            _file: file,
        });
    }

    Ok(ownership)
}

fn acquire_root_ownership_serialization() -> Result<RootOwnershipSerialization, RunCommandError> {
    let path = std::env::temp_dir().join("opensymphony-runtime-root-ownership.lock");
    acquire_root_ownership_serialization_at(&path)
}

fn acquire_root_ownership_serialization_at(
    path: &Path,
) -> Result<RootOwnershipSerialization, RunCommandError> {
    loop {
        // Fully initialize a sibling staging file before publishing it with
        // hard_link. Unlike rename, hard_link never replaces an existing
        // destination, so a concurrent contender cannot steal a live lock;
        // unlike create_new on the final path, a crash before initialization
        // leaves only an unreferenced staging file and never an empty lock.
        let staging_path = path.with_file_name(format!(
            "{}.staging-{}-{}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("lock"),
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
        {
            Ok(mut file) => {
                let initialize = file
                    .write_all(process_marker_fields().as_bytes())
                    .and_then(|_| file.sync_all());
                if let Err(source) = initialize {
                    let _ = fs::remove_file(&staging_path);
                    return Err(RunCommandError::RootOwnership {
                        detail: format!("failed to initialize {}: {source}", path.display()),
                    });
                }
                match fs::hard_link(&staging_path, path) {
                    Ok(()) => {
                        let _ = fs::remove_file(staging_path);
                        return Ok(RootOwnershipSerialization {
                            path: path.to_path_buf(),
                        });
                    }
                    Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                        let _ = fs::remove_file(staging_path);
                    }
                    Err(source) => {
                        let _ = fs::remove_file(staging_path);
                        return Err(RunCommandError::RootOwnership {
                            detail: format!("failed to publish {}: {source}", path.display()),
                        });
                    }
                }
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(RunCommandError::RootOwnership {
                    detail: format!("failed to stage {}: {source}", path.display()),
                });
            }
        }

        if !serialization_lock_owner_alive(path) {
            let stale = path.with_file_name(format!(
                "opensymphony-runtime-root-ownership.lock.stale-{}-{}",
                std::process::id(),
                Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ));
            match fs::rename(path, &stale) {
                Ok(()) => {
                    let _ = fs::remove_file(stale);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RunCommandError::RootOwnership {
                        detail: format!("failed to reclaim {}: {error}", path.display()),
                    });
                }
            }
            continue;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn serialization_lock_owner_alive(marker: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(marker) else {
        return true;
    };
    let Some(pid) = contents
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<i32>().ok())
    else {
        // This path is only used for the atomically published host-wide
        // serialization marker. An empty/malformed marker can only be an
        // interrupted pre-publication file from an older implementation and
        // is therefore reclaimable.
        return false;
    };
    process_owner_alive(
        pid,
        contents
            .lines()
            .find_map(|line| line.strip_prefix("start=").map(str::trim)),
    )
}

fn root_marker_blocks(marker: &Path) -> bool {
    let metadata = match fs::symlink_metadata(marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    metadata.file_type().is_symlink() || root_lock_owner_alive(marker)
}

fn root_ownership_registry_path() -> PathBuf {
    std::env::temp_dir().join("opensymphony-runtime-root-ownership-registry")
}

fn claim_root_registry_marker(root: &Path) -> Result<PathBuf, RunCommandError> {
    let registry = root_ownership_registry_path();
    fs::create_dir_all(&registry).map_err(|source| RunCommandError::RootOwnership {
        detail: format!("failed to create {}: {source}", registry.display()),
    })?;
    let sequence = ATOMIC_MARKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let marker = registry.join(format!("root-{}-{sequence}.active", std::process::id()));
    publish_initialized_marker(
        &marker,
        &format!("{}root={}\n", process_marker_fields(), root.display()),
    )
    .map(|_| marker.clone())
    .map_err(|source| RunCommandError::RootOwnership {
        detail: format!("failed to publish {}: {source}", marker.display()),
    })
}

fn find_live_registered_root_marker(
    root: &Path,
    own_registry_markers: &BTreeSet<PathBuf>,
) -> Result<Option<(PathBuf, PathBuf)>, RunCommandError> {
    let registry = root_ownership_registry_path();
    let entries = match fs::read_dir(&registry) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RunCommandError::RootOwnership {
                detail: format!("failed to inspect {}: {source}", registry.display()),
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| RunCommandError::RootOwnership {
            detail: format!("failed to inspect {}: {source}", registry.display()),
        })?;
        let marker = entry.path();
        if own_registry_markers.contains(&marker) {
            continue;
        }
        let Some(name) = marker.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with("root-") || !name.ends_with(".active") {
            continue;
        }
        let metadata =
            fs::symlink_metadata(&marker).map_err(|source| RunCommandError::RootOwnership {
                detail: format!("failed to inspect {}: {source}", marker.display()),
            })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let contents =
            fs::read_to_string(&marker).map_err(|source| RunCommandError::RootOwnership {
                detail: format!("failed to read {}: {source}", marker.display()),
            })?;
        let Some(pid) = contents
            .lines()
            .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<i32>().ok())
        else {
            return Err(RunCommandError::RootOwnership {
                detail: format!("ownership registry marker {} has no PID", marker.display()),
            });
        };
        let start = contents
            .lines()
            .find_map(|line| line.strip_prefix("start=").map(str::trim));
        if !process_owner_alive(pid, start) {
            let _ = fs::remove_file(&marker);
            continue;
        }
        let Some(owned_root) = contents
            .lines()
            .find_map(|line| line.strip_prefix("root=").map(PathBuf::from))
        else {
            return Err(RunCommandError::RootOwnership {
                detail: format!("ownership registry marker {} has no root", marker.display()),
            });
        };
        if paths_overlap(root, &owned_root) {
            return Ok(Some((marker, owned_root)));
        }
    }
    Ok(None)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn initialize_root_marker_atomic(marker: &Path) -> io::Result<File> {
    publish_initialized_marker(marker, &process_marker_fields())
}

#[cfg(test)]
fn initialize_root_marker(mut file: File, marker: &Path) -> Result<File, RunCommandError> {
    if let Err(source) = file.write_all(process_marker_fields().as_bytes()) {
        drop(file);
        let _ = fs::remove_file(marker);
        return Err(RunCommandError::RootOwnership {
            detail: format!("failed to initialize {}: {source}", marker.display()),
        });
    }
    Ok(file)
}

pub(crate) fn root_lock_owner_alive(marker: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(marker) else {
        return true;
    };
    let Some(pid) = contents
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<i32>().ok())
    else {
        return true;
    };

    process_owner_alive(
        pid,
        contents
            .lines()
            .find_map(|line| line.strip_prefix("start=").map(str::trim)),
    )
}

pub(crate) fn process_owner_alive(pid: i32, expected_start: Option<&str>) -> bool {
    let alive = {
        #[cfg(unix)]
        {
            let Some(pid) = rustix::process::Pid::from_raw(pid) else {
                return true;
            };
            match rustix::process::test_kill_process(pid) {
                Ok(()) => true,
                Err(error) if error == rustix::io::Errno::SRCH => false,
                Err(_) => true,
            }
        }
        #[cfg(not(unix))]
        {
            let Ok(output) = Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH"])
                .output()
            else {
                // If process liveness cannot be determined, fail closed.
                return true;
            };
            tasklist_process_is_alive(output.status.success(), &output.stdout, pid)
        }
    };
    alive
        && expected_start.is_none_or(|expected| {
            process_incarnation(pid as u32).is_some_and(|actual| actual == expected)
        })
}

pub(crate) fn process_marker_fields() -> String {
    let mut fields = format!("pid={}\n", std::process::id());
    if let Some(start) = process_incarnation(std::process::id()) {
        fields.push_str("start=");
        fields.push_str(&start);
        fields.push('\n');
    }
    fields
}

fn process_incarnation(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after_command = stat.rsplit_once(')')?.1;
        let start_time = after_command.split_whitespace().nth(19)?;
        Some(format!("linux:{start_time}"))
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "lstart="])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let start = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!start.is_empty()).then(|| format!("ps:{start}"))
    }
    #[cfg(target_os = "windows")]
    {
        use std::ffi::c_void;

        type Handle = *mut c_void;
        #[repr(C)]
        struct FileTime {
            low: u32,
            high: u32,
        }

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> Handle;
            fn GetProcessTimes(
                process: Handle,
                creation: *mut FileTime,
                exit: *mut FileTime,
                kernel: *mut FileTime,
                user: *mut FileTime,
            ) -> i32;
            fn CloseHandle(object: Handle) -> i32;
        }

        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut creation = FileTime { low: 0, high: 0 };
        let mut exit = FileTime { low: 0, high: 0 };
        let mut kernel = FileTime { low: 0, high: 0 };
        let mut user = FileTime { low: 0, high: 0 };
        let result =
            unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) };
        unsafe {
            CloseHandle(handle);
        }
        if result == 0 {
            return None;
        }
        let ticks = (u64::from(creation.high) << 32) | u64::from(creation.low);
        Some(format!("windows:{ticks}"))
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "windows"
    )))]
    {
        let _ = pid;
        None
    }
}

#[cfg(any(test, not(unix)))]
fn tasklist_process_is_alive(status_success: bool, stdout: &[u8], pid: i32) -> bool {
    if !status_success {
        // An unsuccessful probe is unknown, not proof that the owner exited.
        return true;
    }
    String::from_utf8_lossy(stdout).lines().any(|line| {
        line.split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u32>().ok())
            == Some(pid as u32)
    })
}

fn acquire_strict_run_marker(
    runtime: &RunRuntimeConfig,
) -> Result<Option<super::migration::StrictRunMarkerGuard>, RunCommandError> {
    if !runtime.central_config {
        return Ok(None);
    }
    let Some(config_path) = runtime.config_path.as_deref() else {
        return Ok(None);
    };
    let marker = super::migration::strict_run_marker_path(config_path);
    super::migration::claim_strict_run_marker(config_path, &runtime.config_generation)
        .map(Some)
        .map_err(|source| RunCommandError::StrictRunMarker {
            path: marker,
            detail: source.to_string(),
        })
}

async fn preclaim_strict_run_marker(
    args: &RunArgs,
) -> Result<Option<super::migration::StrictRunMarkerGuard>, RunCommandError> {
    let cwd = std::env::current_dir().map_err(RunCommandError::CurrentDir)?;
    let Some(config_path) = select_config_path(&cwd, args.config.as_deref()) else {
        return Ok(None);
    };
    let raw = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|source| RunCommandError::ReadConfig {
            path: config_path.clone(),
            source,
        })?;
    if !looks_like_central_config(&raw) {
        return Ok(None);
    }
    let marker = super::migration::strict_run_marker_path(&config_path);
    super::migration::claim_strict_run_marker(&config_path, "pending-resolution")
        .map(Some)
        .map_err(|source| RunCommandError::StrictRunMarker {
            path: marker,
            detail: source.to_string(),
        })
}

pub async fn run_command(args: RunArgs) -> ExitCode {
    match run_orchestrator(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_orchestrator(args: RunArgs) -> Result<(), RunCommandError> {
    let strict_run_marker = preclaim_strict_run_marker(&args).await?;
    let mut runtime = resolve_runtime_config(&args).await?;
    if let Some(marker) = strict_run_marker.as_ref() {
        marker
            .update_generation(&runtime.config_generation)
            .map_err(|source| RunCommandError::StrictRunMarker {
                path: super::migration::strict_run_marker_path(
                    runtime
                        .config_path
                        .as_deref()
                        .expect("central runtime should retain its config path"),
                ),
                detail: source.to_string(),
            })?;
    }
    let _root_ownership = acquire_runtime_root_ownership(&runtime)?;
    let _strict_run_marker = match strict_run_marker {
        Some(marker) => Some(marker),
        None => acquire_strict_run_marker(&runtime)?,
    };
    let linear_worker_env = apply_linear_oauth_client_credentials(&mut runtime).await?;
    info!(
        config = runtime
            .config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<none>".to_string()),
        config_generation = %runtime.config_generation,
        target_repo = %runtime.target_repo.display(),
        workflow = %runtime.workflow_path.display(),
        bind = %runtime.bind,
        "starting OpenSymphony orchestrator"
    );

    let mut tracker = build_tracker_backend(&runtime.workflow)?;
    let workspace_manager = Arc::new(crate::opensymphony_workspace::WorkspaceManager::new(
        build_workspace_manager_config_with_retention(
            &runtime.workflow,
            runtime.retain_failed,
            runtime.preserve_terminal_workspaces,
        ),
    )?);
    let retry_state_root = runtime.state_root.clone().unwrap_or_else(|| {
        workspace_manager
            .config()
            .root
            .join(".opensymphony-retry-state")
    });
    let workspace = RuntimeWorkspaceBackend::new_with_retention_and_state_root(
        workspace_manager.clone(),
        &runtime.workflow,
        runtime.retain_failed,
        retry_state_root,
    );
    let selected_openhands = selected_openhands_harness(&runtime);
    let managed_local_preparation = if selected_openhands {
        prepare_active_conversation_store(&runtime, &mut tracker, workspace_manager.as_ref())
            .await?
    } else {
        ManagedLocalPreparation::default()
    };
    let active_store_preparation = &managed_local_preparation.active_conversations;
    let legacy_store_migration = &managed_local_preparation.legacy_conversations;
    if legacy_store_migration.moved_to_archived > 0 {
        info!(
            moved_to_archived = legacy_store_migration.moved_to_archived,
            already_archived = legacy_store_migration.already_archived,
            missing = legacy_store_migration.missing,
            skipped_non_terminal = legacy_store_migration.skipped_non_terminal,
            skipped_without_manifest = legacy_store_migration.skipped_without_manifest,
            skipped_invalid_manifest = legacy_store_migration.skipped_invalid_manifest,
            "migrated terminal OpenHands conversations into the repo archived store"
        );
    }
    if active_store_preparation.moved > 0 {
        info!(
            moved = active_store_preparation.moved,
            already_active = active_store_preparation.already_active,
            missing = active_store_preparation.missing,
            skipped_without_workspace = active_store_preparation.skipped_without_workspace,
            skipped_without_manifest = active_store_preparation.skipped_without_manifest,
            skipped_invalid_manifest = active_store_preparation.skipped_invalid_manifest,
            "prepared repo-scoped active OpenHands conversations before server startup"
        );
    }

    let mut memory_server = start_runtime_memory_server(&runtime).await?;
    let memory_env = memory_server.as_ref().map(|server| RuntimeMemoryEnv {
        endpoint: server.endpoint().to_string(),
        token: runtime
            .memory
            .server
            .as_ref()
            .and_then(|server| server.token.clone()),
        project: runtime.workflow.config.tracker.project_slug.clone(),
        execution_repo: runtime.target_repo.display().to_string(),
    });
    if let Some(env) = &memory_env {
        info!(endpoint = %env.endpoint, "started OpenSymphony memory server");
    }

    let (transport, mut supervisor) = if selected_openhands {
        build_runtime_transport(
            &runtime,
            managed_local_preparation.tooling,
            memory_env.as_ref(),
            &linear_worker_env,
        )
        .await?
    } else {
        (
            TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)?,
            None,
        )
    };
    let client = crate::opensymphony_openhands::OpenHandsClient::new(transport);
    if selected_openhands {
        client.openapi_probe().await?;
    }

    let worker = RuntimeWorkerBackend::new(
        client.clone(),
        Arc::new(runtime.workflow.clone()),
        workspace_manager,
        memory_env.clone(),
        linear_worker_env,
    );
    let mut scheduler_config = SchedulerConfig::from_workflow(&runtime.workflow)?;
    scheduler_config.max_retry_attempts = runtime.retry_max_attempts;
    let mut scheduler = Scheduler::new(tracker, workspace, worker, scheduler_config);

    let mut recent_events = VecDeque::new();
    push_recent_event(
        &mut recent_events,
        RecentEventKind::SnapshotPublished,
        None,
        format!(
            "loaded {} (config generation {})",
            runtime.workflow_path.display(),
            runtime.config_generation
        ),
        Utc::now(),
    );
    if let Some(env) = &memory_env {
        push_recent_event(
            &mut recent_events,
            RecentEventKind::SnapshotPublished,
            None,
            format!("memory server listening at {}", env.endpoint),
            Utc::now(),
        );
    }

    let initial_snapshot = map_snapshot(
        &scheduler.snapshot(now_timestamp()),
        runtime.workflow.config.workspace.root.as_path(),
        &terminal_state_set(&runtime.workflow),
        current_agent_server_status(&mut supervisor, client.base_url()),
        current_memory_server_status(memory_server.as_ref()),
        &recent_events,
    );

    let store = SnapshotStore::new(initial_snapshot);
    let listener = TcpListener::bind(runtime.bind)
        .await
        .map_err(RunCommandError::BindListener)?;
    let gateway_journal = InMemoryEventJournal::new(10_000, 256);
    let gateway_broker = StreamBroker::new(gateway_journal.clone());
    let gateway_memory_config = if runtime.memory.server.is_some() || runtime.memory.auto_capture {
        Some(load_runtime_memory_config(&runtime)?)
    } else {
        None
    };
    let server_memory_config = if runtime.memory.server.is_some() {
        gateway_memory_config.clone()
    } else {
        None
    };
    let server =
        GatewayServer::with_journal(store.clone(), gateway_journal.clone(), gateway_broker)
            .with_linear_task_graph(build_optional_task_graph_client(&runtime.workflow))
            .with_memory_config(server_memory_config)
            .with_active_states(
                runtime
                    .workflow
                    .config
                    .tracker
                    .active_states
                    .iter()
                    .cloned(),
            )
            .with_terminal_states(terminal_state_set(&runtime.workflow));
    let mut server_task = tokio::spawn(async move { server.serve(listener).await });
    let mut gateway_action_cursor = 0;

    let bootstrap_snapshot = tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("received shutdown signal");
            server_task.abort();
            shutdown_memory_server(&mut memory_server).await?;
            if let Some(mut supervisor) = supervisor {
                let _ = supervisor.stop();
            }
            return Ok(());
        }
        result = &mut server_task => {
            match result {
                Ok(Ok(())) => {
                    shutdown_memory_server(&mut memory_server).await?;
                    if let Some(mut supervisor) = supervisor {
                        let _ = supervisor.stop();
                    }
                    return Ok(());
                }
                Ok(Err(error)) => {
                    shutdown_memory_server(&mut memory_server).await?;
                    return Err(RunCommandError::Serve(error));
                }
                Err(error) => {
                    shutdown_memory_server(&mut memory_server).await?;
                    return Err(RunCommandError::Serve(io::Error::other(error.to_string())));
                }
            }
        }
        result = scheduler.bootstrap(now_timestamp()) => match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                server_task.abort();
                shutdown_memory_server(&mut memory_server).await?;
                return Err(RunCommandError::SchedulerConfig(error));
            }
        },
    };
    let mut auto_capture_completed_issues = terminal_issue_identifiers(&bootstrap_snapshot);
    push_recent_event(
        &mut recent_events,
        RecentEventKind::SnapshotPublished,
        None,
        format!(
            "recovered startup state; running={}, retry_queue={}",
            bootstrap_snapshot.daemon.running_issue_count,
            bootstrap_snapshot.daemon.retry_queue_count
        ),
        Utc::now(),
    );
    store
        .publish(map_snapshot(
            &bootstrap_snapshot,
            runtime.workflow.config.workspace.root.as_path(),
            &terminal_state_set(&runtime.workflow),
            current_agent_server_status(&mut supervisor, client.base_url()),
            current_memory_server_status(memory_server.as_ref()),
            &recent_events,
        ))
        .await;

    let poll_interval = Duration::from_millis(runtime.workflow.config.polling.interval_ms);
    let mut ticker = interval(poll_interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("received shutdown signal");
                break;
            }
            result = &mut server_task => {
                match result {
                    Ok(Ok(())) => break,
                    Ok(Err(error)) => {
                        shutdown_memory_server(&mut memory_server).await?;
                        return Err(RunCommandError::Serve(error));
                    }
                    Err(error) => {
                        shutdown_memory_server(&mut memory_server).await?;
                        return Err(RunCommandError::Serve(io::Error::other(error.to_string())));
                    }
                }
            }
            result = async {
                ticker.tick().await;
                let observed_at = now_timestamp();
                let result = match apply_gateway_action_events(
                    &mut scheduler,
                    &gateway_journal,
                    &mut gateway_action_cursor,
                    observed_at,
                ).await {
                    Ok(()) => scheduler.tick(observed_at).await,
                    Err(error) => Err(error),
                };
                (observed_at, result)
            } => {
                let (observed_at, result) = result;
                match result {
                    Ok(snapshot) => {
                        let current_terminal_issues = terminal_issue_identifiers(&snapshot);
                        let auto_capture_candidates = auto_capture_candidates(
                            &current_terminal_issues,
                            &mut auto_capture_completed_issues,
                            runtime.memory.auto_capture,
                        );
                        push_recent_event(
                            &mut recent_events,
                            RecentEventKind::SnapshotPublished,
                            None,
                            format!(
                                "polled tracker; running={}, retry_queue={}",
                                snapshot.daemon.running_issue_count,
                                snapshot.daemon.retry_queue_count
                            ),
                            Utc::now(),
                        );
                        store.publish(map_snapshot(
                            &snapshot,
                            runtime.workflow.config.workspace.root.as_path(),
                            &terminal_state_set(&runtime.workflow),
                            current_agent_server_status(&mut supervisor, client.base_url()),
                            current_memory_server_status(memory_server.as_ref()),
                            &recent_events,
                        )).await;
                        if !auto_capture_candidates.is_empty() {
                            let auto_capture_result = super::memory::auto_capture_terminal(
                                &runtime.target_repo,
                                &runtime.workflow_path,
                                Some(&runtime.workflow),
                                &auto_capture_candidates,
                                runtime.openhands_conversation_store.as_ref(),
                                runtime.memory.auto_archive,
                                gateway_memory_config.as_ref(),
                                memory_server
                                    .as_ref()
                                    .and_then(|server| server.writer_gate()),
                            )
                            .await;
                            mark_auto_capture_completed(
                                &mut auto_capture_completed_issues,
                                &auto_capture_candidates,
                                &auto_capture_result,
                            );
                            publish_auto_capture_event(
                                auto_capture_result,
                                &snapshot,
                                &gateway_journal,
                                SnapshotPublishContext {
                                    runtime: &runtime,
                                    supervisor: &mut supervisor,
                                    agent_server_base_url: client.base_url(),
                                    memory_server: memory_server.as_ref(),
                                    memory_config: gateway_memory_config.as_ref(),
                                    recent_events: &mut recent_events,
                                    store: &store,
                                },
                            ).await;
                        }
                    }
                    Err(error) => {
                        warn!(%error, "scheduler tick failed");
                        push_recent_event(
                            &mut recent_events,
                            RecentEventKind::Warning,
                            None,
                            format!("scheduler tick failed: {error}"),
                            Utc::now(),
                        );
                        let snapshot = scheduler.snapshot(observed_at);
                        store.publish(map_snapshot(
                            &snapshot,
                            runtime.workflow.config.workspace.root.as_path(),
                            &terminal_state_set(&runtime.workflow),
                            current_agent_server_status(&mut supervisor, client.base_url()),
                            current_memory_server_status(memory_server.as_ref()),
                            &recent_events,
                        )).await;
                    }
                }
            }
        }
    }

    server_task.abort();
    shutdown_memory_server(&mut memory_server).await?;
    if let Some(mut supervisor) = supervisor {
        let _ = supervisor.stop();
    }

    Ok(())
}

async fn shutdown_memory_server(
    memory_server: &mut Option<super::memory::MemoryServerHandle>,
) -> Result<(), RunCommandError> {
    if let Some(server) = memory_server.take() {
        server.abort();
        server.wait().await?;
    }
    Ok(())
}

fn selected_openhands_harness(runtime: &RunRuntimeConfig) -> bool {
    runtime.workflow.config.routing.harness == "openhands_agent_server"
}

async fn apply_gateway_action_events<T, W, M>(
    scheduler: &mut Scheduler<T, W, M>,
    journal: &InMemoryEventJournal,
    cursor: &mut u64,
    observed_at: TimestampMs,
) -> Result<(), SchedulerError>
where
    T: TrackerBackend,
    W: WorkspaceBackend,
    M: WorkerBackend,
{
    for event in journal.all_events().await {
        if event.sequence <= *cursor {
            continue;
        }
        let sequence = event.sequence;
        let Some(target) = gateway_cancel_target(&event) else {
            *cursor = sequence;
            continue;
        };
        scheduler
            .interrupt_operator_cancel(target, observed_at)
            .await?;
        *cursor = sequence;
    }
    Ok(())
}

fn gateway_cancel_target(event: &EventRecord) -> Option<&str> {
    match &event.kind {
        EventKind::GatewayActionDispatched { action } if action == "cancel" => {}
        _ => return None,
    }
    let payload = event.payload.as_ref()?;
    if payload["status"] != "accepted" {
        return None;
    }
    payload["target_entity"]["id"].as_str()
}

#[derive(Debug, Deserialize)]
struct LinearOAuthTokenResponse {
    access_token: String,
}

async fn apply_linear_oauth_client_credentials(
    runtime: &mut RunRuntimeConfig,
) -> Result<BTreeMap<String, String>, RunCommandError> {
    let Some((client_id, client_secret)) = linear_oauth_credentials_from_env() else {
        return Ok(BTreeMap::new());
    };

    let response = reqwest::Client::new()
        .post("https://api.linear.app/oauth/token")
        .basic_auth(client_id, Some(client_secret))
        .form(&[
            ("grant_type", "client_credentials"),
            ("scope", "read,write"),
        ])
        .send()
        .await
        .map_err(|error| RunCommandError::LinearOAuthToken(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| RunCommandError::LinearOAuthToken(error.to_string()))?;
    if !status.is_success() {
        return Err(RunCommandError::LinearOAuthToken(format!(
            "Linear token endpoint returned HTTP {status}"
        )));
    }
    let token: LinearOAuthTokenResponse = serde_json::from_str(&body)
        .map_err(|error| RunCommandError::LinearOAuthToken(error.to_string()))?;
    let authorization = format!("Bearer {}", token.access_token.trim());
    runtime.workflow.config.tracker.api_key = authorization.clone();

    info!("using Linear OAuth client-credentials token for orchestrator and workers");
    Ok(BTreeMap::from([(
        "LINEAR_API_KEY".to_string(),
        authorization,
    )]))
}

fn linear_oauth_credentials_from_env() -> Option<(String, String)> {
    let client_id = std::env::var("LINEAR_CLIENT_ID").ok()?.trim().to_string();
    let client_secret = std::env::var("LINEAR_CLIENT_SECRET")
        .ok()?
        .trim()
        .to_string();
    (!client_id.is_empty() && !client_secret.is_empty()).then_some((client_id, client_secret))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeMemoryEnv {
    pub(super) endpoint: String,
    pub(super) token: Option<String>,
    pub(super) project: String,
    pub(super) execution_repo: String,
}

async fn start_runtime_memory_server(
    runtime: &RunRuntimeConfig,
) -> Result<Option<super::memory::MemoryServerHandle>, RunCommandError> {
    let Some(server) = runtime.memory.server.as_ref() else {
        return Ok(None);
    };
    let config = load_runtime_memory_config(runtime)?;
    super::memory::start_memory_server_with_resolved_config(
        config,
        server.bind,
        server.token.clone(),
        Some(runtime.workflow.config.workspace.root.clone()),
        runtime.config_path.clone(),
        Some(runtime.workflow.clone()),
        Some(runtime.config_generation.clone()),
    )
    .await
    .map(Some)
    .map_err(RunCommandError::MemoryServer)
}

fn load_runtime_memory_config(
    runtime: &RunRuntimeConfig,
) -> Result<crate::opensymphony_memory::MemoryConfig, crate::opensymphony_memory::MemoryError> {
    let mut config = crate::opensymphony_memory::MemoryConfig::load(&runtime.target_repo, None)?;
    if let Some(memory_root) = runtime.memory_catalog_root.as_ref() {
        config.memory_root = memory_root.clone();
        config.index_path = memory_root.join(crate::opensymphony_memory::DEFAULT_INDEX_FILE_NAME);
        config.containment_root = runtime.state_root.clone();
    }
    Ok(config)
}

async fn publish_auto_capture_event(
    result: Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
    snapshot: &OrchestratorSnapshot,
    journal: &InMemoryEventJournal,
    context: SnapshotPublishContext<'_>,
) {
    if should_publish_memory_graph_update(&result)
        && let Some(config) = context.memory_config
    {
        match append_memory_graph_updated_event(journal, config).await {
            Ok(_) => {}
            Err(error) => {
                warn!(%error, "failed to publish memory graph update event");
                push_recent_event(
                    context.recent_events,
                    RecentEventKind::Warning,
                    None,
                    format!("memory graph update event publish failed: {error}"),
                    Utc::now(),
                );
            }
        }
    }

    if record_auto_capture_recent_event(context.recent_events, result) {
        context
            .store
            .publish(map_snapshot(
                snapshot,
                context.runtime.workflow.config.workspace.root.as_path(),
                &terminal_state_set(&context.runtime.workflow),
                current_agent_server_status(context.supervisor, context.agent_server_base_url),
                current_memory_server_status(context.memory_server),
                context.recent_events,
            ))
            .await;
    }
}

fn should_publish_memory_graph_update(
    result: &Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
) -> bool {
    result.as_ref().is_ok_and(|report| {
        report.capture_completed
            && (!report.captured_issue_keys.is_empty()
                || !report.archived_issue_keys.is_empty()
                || !report.docs_written.is_empty())
    })
}

async fn append_memory_graph_updated_event(
    journal: &InMemoryEventJournal,
    config: &crate::opensymphony_memory::MemoryConfig,
) -> Result<EventRecord, String> {
    let update = crate::opensymphony_memory::memory_graph_updated_event(
        config,
        crate::opensymphony_memory::DEFAULT_MEMORY_GRAPH_BUNDLE_ID,
        crate::opensymphony_memory::MemoryGraphAccess::AllAccessible,
    )
    .map_err(|error| error.to_string())?;
    let record = memory_graph_updated_record(update)?;
    journal
        .append(record)
        .await
        .map_err(|error| format!("{error:?}"))
}

fn memory_graph_updated_record(
    update: crate::opensymphony_gateway_schema::memory_graph::MemoryGraphUpdatedEvent,
) -> Result<EventRecord, String> {
    let bundle_id = update.bundle_id.clone();
    let payload = serde_json::to_value(&update).map_err(|error| error.to_string())?;
    Ok(EventRecord::builder()
        .actor(EventActor::system("memory"))
        .kind(EventKind::MemoryGraphUpdated {
            bundle_id: bundle_id.clone(),
        })
        .summary(format!("memory graph updated for bundle {bundle_id}"))
        .payload(payload)
        .build())
}

struct SnapshotPublishContext<'a> {
    runtime: &'a RunRuntimeConfig,
    supervisor: &'a mut Option<crate::opensymphony_openhands::LocalServerSupervisor>,
    agent_server_base_url: &'a str,
    memory_server: Option<&'a super::memory::MemoryServerHandle>,
    memory_config: Option<&'a crate::opensymphony_memory::MemoryConfig>,
    recent_events: &'a mut VecDeque<RecentEvent>,
    store: &'a SnapshotStore,
}

fn record_auto_capture_recent_event(
    recent_events: &mut VecDeque<RecentEvent>,
    result: Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
) -> bool {
    match result {
        Ok(report) => {
            if report.captured_issue_keys.is_empty() && report.warnings.is_empty() {
                return false;
            }
            let mut summary = if report.captured_issue_keys.is_empty() {
                "memory capture reported no new capsules".to_string()
            } else {
                format!(
                    "memory captured {} issue(s)",
                    report.captured_issue_keys.len()
                )
            };
            if !report.docs_written.is_empty() {
                summary.push_str(&format!(", synced {} doc(s)", report.docs_written.len()));
            }
            if !report.archived_issue_keys.is_empty() {
                summary.push_str(&format!(
                    ", archived {} issue(s)",
                    report.archived_issue_keys.len()
                ));
            }
            if !report.warnings.is_empty() {
                summary.push_str(&format!(", {} warning(s)", report.warnings.len()));
            }
            push_recent_event(
                recent_events,
                if report.warnings.is_empty() {
                    RecentEventKind::SnapshotPublished
                } else {
                    RecentEventKind::Warning
                },
                None,
                summary,
                Utc::now(),
            );
            true
        }
        Err(error) => {
            warn!(%error, "automatic memory capture failed");
            push_recent_event(
                recent_events,
                RecentEventKind::Warning,
                None,
                format!("automatic memory capture failed: {error}"),
                Utc::now(),
            );
            true
        }
    }
}

fn build_optional_task_graph_client(
    workflow: &crate::opensymphony_workflow::ResolvedWorkflow,
) -> Option<Arc<dyn LinearTaskGraphClient>> {
    optional_task_graph_client(build_linear_client(workflow))
}

fn optional_task_graph_client(
    client: Result<crate::opensymphony_linear::LinearClient, LinearError>,
) -> Option<Arc<dyn LinearTaskGraphClient>> {
    match client {
        Ok(client) => Some(Arc::new(client) as Arc<dyn LinearTaskGraphClient>),
        Err(error) => {
            warn!(
                %error,
                "Linear task graph reader unavailable; task graph endpoint will return 503"
            );
            None
        }
    }
}

fn terminal_issue_identifiers(snapshot: &OrchestratorSnapshot) -> BTreeSet<String> {
    snapshot
        .issues
        .iter()
        .filter(|issue| issue.issue.state.category == IssueStateCategory::Terminal)
        .map(|issue| issue.issue.identifier.to_string())
        .collect()
}

fn auto_capture_candidates(
    current_terminal_issues: &BTreeSet<String>,
    completed_issues: &mut BTreeSet<String>,
    auto_capture_enabled: bool,
) -> Vec<String> {
    completed_issues.retain(|issue| current_terminal_issues.contains(issue));
    if !auto_capture_enabled {
        *completed_issues = current_terminal_issues.clone();
        return Vec::new();
    }
    current_terminal_issues
        .difference(completed_issues)
        .cloned()
        .collect()
}

fn mark_auto_capture_completed(
    completed_issues: &mut BTreeSet<String>,
    candidates: &[String],
    result: &Result<super::memory::AutoMemoryReport, crate::opensymphony_memory::MemoryError>,
) {
    match result {
        Ok(report) if report.workflow_completed() && !report.completed_issue_keys.is_empty() => {
            completed_issues.extend(report.completed_issue_keys.iter().cloned());
        }
        Ok(report) if report.workflow_completed() && report.warnings.is_empty() => {
            completed_issues.extend(candidates.iter().cloned());
        }
        Ok(_) | Err(_) => {}
    }
}

pub(super) fn timestamp_to_datetime(value: TimestampMs) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(value.as_u64() as i64).unwrap_or_else(Utc::now)
}

pub(super) fn datetime_to_timestamp_ms(value: DateTime<Utc>) -> TimestampMs {
    TimestampMs::new(value.timestamp_millis().max(0) as u64)
}

pub(super) fn now_timestamp() -> TimestampMs {
    TimestampMs::new(Utc::now().timestamp_millis().max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_memory::MemoryError;

    fn issue_set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|key| key.to_string()).collect()
    }

    #[test]
    fn optional_task_graph_client_returns_none_when_linear_reader_is_unavailable() {
        let client = optional_task_graph_client(Err(LinearError::InvalidConfiguration(
            "missing task graph config".to_owned(),
        )));

        assert!(
            client.is_none(),
            "gateway task graph reader should fail closed instead of aborting run startup",
        );
    }

    #[test]
    fn runtime_root_ownership_rejects_a_second_live_owner_and_releases_on_drop() {
        let root = tempfile::tempdir().expect("runtime root");
        let first = acquire_root_ownership([root.path().to_path_buf()])
            .expect("first owner should acquire the root");
        let second = acquire_root_ownership([root.path().to_path_buf()]);
        assert!(matches!(second, Err(RunCommandError::RootOwnership { .. })));
        drop(first);
        acquire_root_ownership([root.path().to_path_buf()])
            .expect("root should be available after owner drops");
    }

    #[test]
    fn central_runtime_memory_config_uses_state_root_containment() {
        let root = tempfile::tempdir().expect("runtime root");
        let repo = root.path().join("repo");
        let state = root.path().join("state");
        let memory = state.join("memory");
        fs::create_dir_all(&repo).expect("repository should exist");
        fs::create_dir_all(&memory).expect("memory catalog should exist");
        let workflow = crate::opensymphony_workflow::WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  api_key: test-linear-key\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\n---\nTarget branch: develop\n",
        )
        .expect("workflow should parse")
        .resolve(&repo, &BTreeMap::new())
        .expect("workflow should resolve");
        let runtime = RunRuntimeConfig {
            config_path: None,
            central_config: true,
            config_generation: "test-generation".to_owned(),
            target_repo: repo.clone(),
            workflow_path: repo.join("WORKFLOW.md"),
            workflow,
            bind: "127.0.0.1:3000".parse().expect("bind should parse"),
            tool_dir: None,
            openhands_conversation_store: None,
            retry_max_attempts: None,
            state_root: Some(state.clone()),
            memory_catalog_root: Some(memory),
            retain_failed: true,
            preserve_terminal_workspaces: true,
            memory: config::RunMemoryConfig {
                auto_capture: true,
                auto_archive: false,
                server: None,
            },
        };

        let config = load_runtime_memory_config(&runtime).expect("memory config should load");
        assert_eq!(config.containment_root, Some(state));
    }

    #[test]
    fn serialization_lock_reclaims_an_uninitialized_marker() {
        let root = tempfile::tempdir().expect("serialization root");
        let path = root.path().join("serialization.lock");
        fs::write(&path, "").expect("write interrupted marker");

        let owner = acquire_root_ownership_serialization_at(&path)
            .expect("an interrupted marker should be recoverable");
        let marker = fs::read_to_string(&path).expect("published marker should be readable");
        assert!(marker.starts_with(&format!("pid={}\n", std::process::id())));
        assert!(marker.lines().any(|line| line.starts_with("start=")));
        drop(owner);
        assert!(!path.exists());
    }

    #[test]
    fn runtime_ownership_rejects_a_pid_with_a_different_incarnation() {
        let marker = tempfile::NamedTempFile::new().expect("marker should exist");
        fs::write(
            marker.path(),
            format!("pid={}\nstart=stale-incarnation\n", std::process::id()),
        )
        .expect("marker should be written");

        assert!(!root_lock_owner_alive(marker.path()));
    }

    #[test]
    fn runtime_root_ownership_releases_earlier_roots_when_a_later_root_is_busy() {
        let root = tempfile::tempdir().expect("runtime root");
        let first_root = root.path().join("a-first");
        let second_root = root.path().join("b-second");
        let blocker = acquire_root_ownership([second_root.clone()])
            .expect("second root should have a live blocker");

        let result = acquire_root_ownership([first_root.clone(), second_root.clone()]);

        assert!(matches!(result, Err(RunCommandError::RootOwnership { .. })));
        assert!(
            !first_root.join(".opensymphony-instance.lock").exists(),
            "a failed later acquisition must release earlier roots"
        );
        drop(blocker);
    }

    #[test]
    fn runtime_root_ownership_rejects_nested_configured_roots() {
        let root = tempfile::tempdir().expect("runtime root");
        let parent = root.path().join("workspaces");
        let child = parent.join("other-instance");

        let result = acquire_root_ownership([parent.clone(), child.clone()]);

        assert!(matches!(result, Err(RunCommandError::RootOwnership { .. })));
        assert!(!parent.join(".opensymphony-instance.lock").exists());
        assert!(!child.join(".opensymphony-instance.lock").exists());
    }

    #[test]
    fn runtime_root_ownership_rejects_live_nested_owner_in_either_order() {
        let root = tempfile::tempdir().expect("runtime root");
        let parent = root.path().join("workspaces");
        let child = parent.join("other-instance");

        let child_owner = acquire_root_ownership([child.clone()]).expect("child owner");
        assert!(matches!(
            acquire_root_ownership([parent.clone()]),
            Err(RunCommandError::RootOwnership { .. })
        ));
        drop(child_owner);

        let parent_owner = acquire_root_ownership([parent.clone()]).expect("parent owner");
        assert!(matches!(
            acquire_root_ownership([child]),
            Err(RunCommandError::RootOwnership { .. })
        ));
        drop(parent_owner);
    }

    #[test]
    fn runtime_root_ownership_ignores_incomplete_registry_staging_files() {
        let root = tempfile::tempdir().expect("runtime root");
        let registry = root_ownership_registry_path();
        fs::create_dir_all(&registry).expect("ownership registry should exist");
        let staging = registry.join(format!(
            ".root-{}-incomplete.active.staging-test",
            std::process::id()
        ));
        fs::write(&staging, "").expect("incomplete staging marker should be written");

        let ownership = acquire_root_ownership([root.path().join("workspace")])
            .expect("incomplete registry staging files must be ignored");
        drop(ownership);
        fs::remove_file(staging).expect("staging marker should be removed");
    }

    #[tokio::test]
    async fn strict_run_marker_is_claimed_before_runtime_resolution() {
        let root = tempfile::tempdir().expect("config root");
        let config = root.path().join("config.yaml");
        fs::write(&config, "instance:\n  id: test\n")
            .expect("central-shaped config should be written");
        let args = RunArgs {
            config: Some(config.clone()),
            dry_run: false,
        };

        let marker = preclaim_strict_run_marker(&args)
            .await
            .expect("preclaim should succeed")
            .expect("central-shaped config should claim a marker");
        let marker_path = super::super::migration::strict_run_marker_path(&config);
        let marker_contents = fs::read_to_string(&marker_path).expect("marker should exist");
        assert!(marker_contents.contains("generation=pending-resolution"));
        marker
            .update_generation("sha256:resolved")
            .expect("marker generation should be updatable");
        assert!(
            fs::read_to_string(&marker_path)
                .expect("updated marker should exist")
                .contains("generation=sha256:resolved")
        );
        drop(marker);
        assert!(!marker_path.exists());
    }

    #[test]
    fn unsuccessful_tasklist_probe_is_treated_as_live_unknown() {
        assert!(tasklist_process_is_alive(false, b"", 1234));
    }

    #[test]
    fn successful_tasklist_probe_requires_the_requested_pid() {
        assert!(tasklist_process_is_alive(
            true,
            b"Image Name PID Session Name\nworker.exe 1234 Console\n",
            1234
        ));
        assert!(!tasklist_process_is_alive(
            true,
            b"Image Name PID Session Name\nworker.exe 5678 Console\n",
            1234
        ));
    }

    #[test]
    fn failed_root_marker_initialization_removes_marker() {
        let root = tempfile::tempdir().expect("runtime root");
        let marker = root.path().join(".opensymphony-instance.lock");
        let file = File::create(&marker).expect("marker should be created");
        drop(file);
        let file = File::open(&marker).expect("marker should be reopenable");

        assert!(initialize_root_marker(file, &marker).is_err());
        assert!(!marker.exists());
    }

    #[test]
    fn auto_capture_candidates_retry_until_capture_completes() {
        let current = issue_set(&["COE-1", "COE-2"]);
        let mut completed = issue_set(&["COE-1"]);

        let candidates = auto_capture_candidates(&current, &mut completed, true);

        assert_eq!(candidates, vec!["COE-2".to_string()]);
        mark_auto_capture_completed(
            &mut completed,
            &candidates,
            &Err(MemoryError::InvalidInput("capture failed".to_string())),
        );
        assert_eq!(completed, issue_set(&["COE-1"]));

        let retry_candidates = auto_capture_candidates(&current, &mut completed, true);
        assert_eq!(retry_candidates, vec!["COE-2".to_string()]);
    }

    #[test]
    fn auto_capture_candidates_forget_reopened_issues() {
        let current = issue_set(&["COE-2"]);
        let mut completed = issue_set(&["COE-1", "COE-2"]);

        let candidates = auto_capture_candidates(&current, &mut completed, true);

        assert!(candidates.is_empty());
        assert_eq!(completed, issue_set(&["COE-2"]));
    }

    #[test]
    fn auto_capture_result_waits_for_post_capture_steps_before_completing() {
        let mut completed = issue_set(&["COE-1"]);
        let candidates = vec!["COE-2".to_string()];
        let result = Ok(super::super::memory::AutoMemoryReport {
            completed_issue_keys: Vec::new(),
            captured_issue_keys: vec!["COE-2".to_string()],
            archived_issue_keys: Vec::new(),
            docs_written: Vec::new(),
            capture_completed: true,
            docs_sync_completed: false,
            archive_completed: true,
            warnings: vec!["docs sync failed after capture".to_string()],
        });

        mark_auto_capture_completed(&mut completed, &candidates, &result);

        assert_eq!(completed, issue_set(&["COE-1"]));
    }

    #[test]
    fn auto_capture_result_marks_full_workflow_complete() {
        let mut completed = issue_set(&["COE-1"]);
        let candidates = vec!["COE-2".to_string()];
        let result = Ok(super::super::memory::AutoMemoryReport {
            completed_issue_keys: vec!["COE-2".to_string()],
            captured_issue_keys: vec!["COE-2".to_string()],
            archived_issue_keys: Vec::new(),
            docs_written: vec![PathBuf::from("docs/runtime.md")],
            capture_completed: true,
            docs_sync_completed: true,
            archive_completed: true,
            warnings: Vec::new(),
        });

        mark_auto_capture_completed(&mut completed, &candidates, &result);

        assert_eq!(completed, issue_set(&["COE-1", "COE-2"]));
    }

    #[test]
    fn auto_capture_result_does_not_mark_default_noop_complete() {
        let mut completed = issue_set(&["COE-1"]);
        let candidates = vec!["COE-2".to_string()];
        let result = Ok(super::super::memory::AutoMemoryReport::default());

        mark_auto_capture_completed(&mut completed, &candidates, &result);

        assert_eq!(completed, issue_set(&["COE-1"]));
    }

    #[test]
    fn memory_graph_update_publish_requires_completed_capture() {
        let captured = Ok(super::super::memory::AutoMemoryReport {
            completed_issue_keys: vec!["COE-2".to_string()],
            captured_issue_keys: vec!["COE-2".to_string()],
            archived_issue_keys: Vec::new(),
            docs_written: Vec::new(),
            capture_completed: true,
            docs_sync_completed: true,
            archive_completed: true,
            warnings: Vec::new(),
        });
        assert!(should_publish_memory_graph_update(&captured));

        let no_write = Ok(super::super::memory::AutoMemoryReport {
            capture_completed: true,
            ..super::super::memory::AutoMemoryReport::default()
        });
        assert!(!should_publish_memory_graph_update(&no_write));

        let archived = Ok(super::super::memory::AutoMemoryReport {
            archived_issue_keys: vec!["COE-2".to_string()],
            capture_completed: true,
            ..super::super::memory::AutoMemoryReport::default()
        });
        assert!(should_publish_memory_graph_update(&archived));

        let docs_synced = Ok(super::super::memory::AutoMemoryReport {
            docs_written: vec![PathBuf::from("docs/memory.md")],
            capture_completed: true,
            ..super::super::memory::AutoMemoryReport::default()
        });
        assert!(should_publish_memory_graph_update(&docs_synced));

        let failed = Err(MemoryError::InvalidInput("capture failed".to_string()));
        assert!(!should_publish_memory_graph_update(&failed));
    }

    #[test]
    fn memory_graph_updated_record_carries_payload() {
        let update = crate::opensymphony_gateway_schema::memory_graph::MemoryGraphUpdatedEvent {
            schema_version: crate::opensymphony_gateway_schema::version::SchemaVersion::v1(),
            bundle_id: "local-default".to_string(),
            cursor: crate::opensymphony_gateway_schema::cursor::StreamCursor::new(
                42,
                "memory-graph:local-default",
            ),
            updated_at: Utc::now(),
        };

        let record = memory_graph_updated_record(update.clone()).expect("record should build");

        assert_eq!(record.actor, EventActor::system("memory"));
        assert!(matches!(
            record.kind,
            EventKind::MemoryGraphUpdated { ref bundle_id } if bundle_id == "local-default"
        ));
        assert_eq!(
            record.payload,
            Some(serde_json::to_value(update).expect("payload should serialize"))
        );
    }
}
