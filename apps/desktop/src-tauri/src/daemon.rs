//! Local daemon supervisor for OpenSymphony desktop.
//!
//! Manages the lifecycle of a local OpenSymphony daemon process,
//! including startup, health checking, monitoring, and graceful shutdown.
//! Only stops processes that it explicitly owns.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{error, info, warn};

/// Configuration for a supervised daemon process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Path to the daemon executable.
    pub executable: PathBuf,
    /// Arguments to pass to the daemon.
    pub args: Vec<String>,
    /// Environment variables to set for the daemon process.
    pub env: Vec<(String, String)>,
    /// Maximum time to wait for the daemon to become healthy.
    pub startup_timeout: Duration,
    /// Whether to automatically restart the daemon if it exits.
    pub auto_restart: bool,
    /// Gateway URL where the daemon listens.
    pub gateway_url: String,
    /// Skip health check after startup (for testing or daemons without HTTP).
    pub skip_health_check: bool,
}

/// Current state of the supervised daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonState {
    /// Daemon is not running.
    Stopped,
    /// Daemon is starting up.
    Starting,
    /// Daemon is running and healthy.
    Running,
    /// Daemon is running but unhealthy.
    Unhealthy,
    /// Daemon is shutting down.
    Stopping,
    /// Daemon has crashed or failed to start.
    Failed(String),
}

impl DaemonState {
    /// Return a simple snake_case string representation for API responses.
    pub fn as_str(&self) -> &'static str {
        match self {
            DaemonState::Stopped => "stopped",
            DaemonState::Starting => "starting",
            DaemonState::Running => "running",
            DaemonState::Unhealthy => "unhealthy",
            DaemonState::Stopping => "stopping",
            DaemonState::Failed(_) => "failed",
        }
    }
}

/// Result of a daemon startup attempt.
#[derive(Debug, Serialize)]
pub struct StartupResult {
    /// Whether startup succeeded.
    pub success: bool,
    /// Process ID if started successfully.
    pub pid: Option<u32>,
    /// Error message if startup failed.
    pub error: Option<String>,
    /// Time taken to start up in milliseconds.
    pub elapsed_ms: u64,
}

/// Error type for daemon operations.
#[derive(Error, Debug)]
pub enum DaemonError {
    #[error("daemon failed to start: {0}")]
    StartFailed(String),
    #[error("daemon is not running")]
    NotRunning,
    #[error("daemon health check failed: {0}")]
    HealthCheckFailed(String),
    #[error("timeout waiting for daemon to start")]
    StartupTimeout,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Handle to a supervised daemon process.
///
/// Tracks process ownership and provides lifecycle management.
/// Only stops processes that this handle explicitly owns.
pub struct DaemonHandle {
    /// The child process.
    child: Option<Child>,
    /// Process ID of the daemon.
    pid: Option<u32>,
    /// Current state of the daemon.
    state: DaemonState,
    /// Whether this handle owns the process (and should stop it on drop).
    owns_process: bool,
    /// Configuration used to start this daemon.
    config: DaemonConfig,
}

impl DaemonHandle {
    /// Create a new daemon handle with the given configuration.
    pub fn new(config: DaemonConfig) -> Self {
        Self {
            child: None,
            pid: None,
            state: DaemonState::Stopped,
            owns_process: false,
            config,
        }
    }

    /// Start the daemon process.
    ///
    /// Only starts if not already running. Returns a StartupResult with
    /// the outcome and timing information.
    pub async fn start(&mut self) -> StartupResult {
        if self.is_running() {
            warn!("daemon already running, pid={:?}", self.pid);
            return StartupResult {
                success: true,
                pid: self.pid,
                error: None,
                elapsed_ms: 0,
            };
        }

        let start_time = Instant::now();
        info!(
            executable = ?self.config.executable,
            args = ?self.config.args,
            "starting supervised daemon",
        );

        self.state = DaemonState::Starting;

        let mut cmd = Command::new(&self.config.executable);
        cmd.args(&self.config.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // Create a new process group so we can kill the entire group on cleanup.
        // This prevents orphaned child processes when the parent is killed.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }

        for (key, value) in &self.config.env {
            cmd.env(key, value);
        }

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id();
                self.child = Some(child);
                self.pid = Some(pid);
                self.owns_process = true;

                info!(pid, "daemon process started");

                // Wait for health check unless explicitly skipped
                let health_result = if self.config.skip_health_check {
                    Ok(())
                } else {
                    self.wait_for_health().await
                };

                let elapsed = start_time.elapsed().as_millis() as u64;

                match health_result {
                    Ok(()) => {
                        self.state = DaemonState::Running;
                        info!(pid, elapsed_ms = elapsed, "daemon healthy");
                        StartupResult {
                            success: true,
                            pid: Some(pid),
                            error: None,
                            elapsed_ms: elapsed,
                        }
                    }
                    Err(e) => {
                        self.state = DaemonState::Failed(e.to_string());
                        error!(pid, error = %e, "daemon failed health check");
                        // Kill the spawned process since startup failed.
                        // This prevents orphaned daemon processes.
                        self.kill_process_only();
                        self.child = None;
                        self.pid = None;
                        self.owns_process = false;
                        StartupResult {
                            success: false,
                            pid: None,
                            error: Some(e.to_string()),
                            elapsed_ms: elapsed,
                        }
                    }
                }
            }
            Err(e) => {
                self.state = DaemonState::Failed(e.to_string());
                error!(error = %e, "failed to spawn daemon process");
                StartupResult {
                    success: false,
                    pid: None,
                    error: Some(e.to_string()),
                    elapsed_ms: start_time.elapsed().as_millis() as u64,
                }
            }
        }
    }

    /// Wait for the daemon to become healthy.
    ///
    /// Polls the health endpoint until it responds or the timeout is reached.
    async fn wait_for_health(&self) -> Result<(), DaemonError> {
        let deadline = Instant::now() + self.config.startup_timeout;
        let health_url = format!("{}/healthz", self.config.gateway_url.trim_end_matches('/'));

        info!(url = %health_url, "waiting for daemon health check");

        // Use a client with per-request timeout to avoid blocking indefinitely
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| DaemonError::StartFailed(format!("Failed to build HTTP client: {}", e)))?;

        while Instant::now() < deadline {
            match client.get(&health_url).send().await {
                Ok(response) if response.status().is_success() => {
                    return Ok(());
                }
                Ok(response) => {
                    warn!(status = %response.status(), "daemon not yet ready");
                }
                Err(e) => {
                    warn!(error = %e, "health check failed, retrying");
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        Err(DaemonError::StartupTimeout)
    }

    /// Check if the daemon is currently running.
    ///
    /// Verifies both internal state and OS-level process liveness to detect
    /// crashes or external kills. Updates internal state if the OS process
    /// has died but our state still says running.
    pub fn is_running(&mut self) -> bool {
        if self.pid.is_none()
            || !matches!(
                self.state,
                DaemonState::Running
                    | DaemonState::Starting
                    | DaemonState::Unhealthy
                    | DaemonState::Stopping
            )
        {
            return false;
        }
        if !self.is_process_alive() {
            // OS process died but state still says running/stopping - update to reflect reality.
            // This prevents stale-state hazards where is_running() returns false
            // but daemon_status() still reports state as "running" or "stopping".
            warn!(pid = ?self.pid, "daemon process detected dead, updating state");
            self.state = DaemonState::Stopped;
            self.pid = None;
            self.child = None;
            self.owns_process = false;
            false
        } else {
            true
        }
    }

    /// Check if the child process is still alive.
    ///
    /// `try_wait()` is non-blocking and reaps the child if it has already
    /// exited, which keeps liveness checks accurate on every platform.
    fn is_process_alive(&mut self) -> bool {
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(None) => true,
                Ok(Some(status)) => {
                    info!(pid = ?self.pid, status = ?status, "daemon process exited");
                    false
                }
                Err(e) => {
                    if Self::is_child_already_reaped_error(&e) {
                        info!(pid = ?self.pid, error = %e, "daemon process was already reaped");
                        false
                    } else {
                        warn!(pid = ?self.pid, error = %e, "failed to check daemon process status");
                        true
                    }
                }
            }
        } else {
            false
        }
    }

    /// Get the current state of the daemon.
    pub fn state(&self) -> &DaemonState {
        &self.state
    }

    /// Get the process ID of the daemon.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Get the gateway URL for this daemon.
    pub fn gateway_url(&self) -> &str {
        &self.config.gateway_url
    }

    /// Stop the daemon process gracefully.
    ///
    /// Only stops if this handle owns the process. Sends SIGTERM first,
    /// waits up to 5 seconds for exit, then escalates to SIGKILL if needed.
    /// Uses async sleep to avoid blocking the tokio worker thread.
    pub async fn stop(&mut self) -> Result<(), DaemonError> {
        if !self.owns_process {
            warn!("attempted to stop daemon that we don't own");
            return Ok(());
        }

        if self.child.is_some() {
            info!(pid = ?self.pid, "stopping daemon process");
            self.state = DaemonState::Stopping;

            #[cfg(unix)]
            {
                if let Some(pid) = self.pid {
                    // Send SIGTERM to the entire process group for graceful shutdown.
                    // Since we use setsid(), all child processes are in the same group.
                    let _ = unsafe { libc::kill(-(pid as i32), libc::SIGTERM) };
                }
            }

            #[cfg(windows)]
            {
                if let Some(pid) = self.pid {
                    Self::spawn_taskkill(pid, false);
                }
            }

            // Async wait loop: poll for exit without blocking the tokio worker thread.
            // If the process doesn't exit within 5 seconds, escalate to SIGKILL.
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut exited = false;
            while Instant::now() < deadline {
                if let Some(ref mut child) = self.child {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            info!(pid = ?self.pid, status = ?status, "daemon stopped gracefully");
                            exited = true;
                            break;
                        }
                        Ok(None) => {
                            // Process still running; use async sleep so we
                            // don't block the tokio worker thread.
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        Err(e) => {
                            warn!(pid = ?self.pid, error = %e, "error checking daemon status");
                            break;
                        }
                    }
                }
            }

            // Escalate to SIGKILL if process didn't exit within timeout
            if !exited {
                warn!(pid = ?self.pid, "daemon did not exit within 5s, force-killing");
                self.kill_process_only();
            }
        }

        self.child = None;
        self.pid = None;
        self.state = DaemonState::Stopped;
        self.owns_process = false;

        Ok(())
    }

    /// Internal helper to kill just the process without updating state fields.
    ///
    /// Sends a force-kill signal and performs only an immediate `try_wait()` on
    /// the caller's thread. If the child has not exited yet, a background thread
    /// owns the final blocking wait so `Drop` and async callers never stall.
    fn kill_process_only(&mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = self.pid;
            Self::force_kill_child(pid, &mut child);

            match child.try_wait() {
                Ok(Some(status)) => {
                    info!(pid = ?pid, status = ?status, "daemon process reaped after force kill");
                }
                Ok(None) => {
                    Self::reap_child_in_background(child, pid);
                }
                Err(e) => {
                    warn!(pid = ?pid, error = %e, "failed to reap daemon after force kill");
                    Self::reap_child_in_background(child, pid);
                }
            }
        }
    }

    /// Force-kill the daemon process.
    pub fn kill(&mut self) -> Result<(), DaemonError> {
        info!(pid = ?self.pid, "force-killing daemon");
        self.kill_process_only();
        self.pid = None;
        self.state = DaemonState::Stopped;
        self.owns_process = false;
        Ok(())
    }

    fn force_kill_child(pid: Option<u32>, child: &mut Child) {
        #[cfg(unix)]
        {
            if let Some(pid) = pid {
                // Kill the entire process group (negative PID means process group ID).
                // This ensures all child processes are also terminated, preventing
                // orphaned processes when the parent is killed.
                let _ = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
        }
        #[cfg(windows)]
        {
            if let Some(pid) = pid {
                Self::spawn_taskkill(pid, true);
            }
        }
        let _ = child.kill();
    }

    #[cfg(windows)]
    fn spawn_taskkill(pid: u32, force: bool) {
        let pid_arg = pid.to_string();
        let mut command = Command::new("taskkill");
        command.args(["/PID", pid_arg.as_str(), "/T"]);
        if force {
            command.arg("/F");
        }
        if let Err(e) = command.spawn() {
            warn!(pid, error = %e, "failed to spawn taskkill");
        }
    }

    fn is_child_already_reaped_error(error: &std::io::Error) -> bool {
        #[cfg(unix)]
        {
            error.raw_os_error() == Some(libc::ECHILD)
        }
        #[cfg(not(unix))]
        {
            let _ = error;
            false
        }
    }

    fn reap_child_in_background(mut child: Child, pid: Option<u32>) {
        if let Err(e) = std::thread::Builder::new()
            .name("os-reaper".to_string())
            .spawn(move || {
                let status = child.wait();
                match status {
                    Ok(status) => {
                        info!(pid = ?pid, status = ?status, "daemon process reaped in background");
                    }
                    Err(e) => {
                        warn!(pid = ?pid, error = %e, "background daemon reap failed");
                    }
                }
            })
        {
            warn!(pid = ?pid, error = %e, "failed to spawn daemon reaper thread");
        }
    }
}

impl Drop for DaemonHandle {
    fn drop(&mut self) {
        if self.owns_process {
            info!(
                pid = ?self.pid,
                "daemon handle dropped, cleaning up owned process",
            );
            // Non-blocking cleanup: kill without waiting to avoid blocking
            // the thread during drop, especially in async contexts.
            self.kill_process_only();
            self.child = None;
            self.pid = None;
            self.state = DaemonState::Stopped;
            self.owns_process = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn test_config() -> DaemonConfig {
        DaemonConfig {
            executable: PathBuf::from("/bin/sleep"),
            args: vec!["300".to_string()],
            env: vec![("TEST_VAR".to_string(), "test_value".to_string())],
            startup_timeout: Duration::from_secs(5),
            auto_restart: true,
            gateway_url: "http://127.0.0.1:2468".to_string(),
            skip_health_check: false,
        }
    }

    #[test]
    fn test_daemon_handle_creation() {
        let config = test_config();
        let mut handle = DaemonHandle::new(config);
        assert_eq!(handle.state(), &DaemonState::Stopped);
        assert!(handle.pid().is_none());
        assert!(!handle.is_running());
    }

    #[tokio::test]
    async fn test_daemon_start_stop_with_fake_command() {
        // Create a simple script that exits immediately
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("fake_daemon.sh");
        fs::write(
            &script_path,
            "#!/bin/bash\nsleep 0.1\necho 'daemon started'\nwhile true; do sleep 1; done\n",
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }

        let mut config = test_config();
        config.executable = script_path.clone();
        config.args = vec![];
        config.startup_timeout = Duration::from_secs(2);
        config.gateway_url = format!("file://{}", dir.path().display());
        config.skip_health_check = true;

        let mut handle = DaemonHandle::new(config);

        // Start the daemon
        let result = handle.start().await;
        // With skip_health_check = true, startup succeeds without waiting for health check.
        assert!(result.success);
        // The spawned process is not killed since health check was skipped
        assert!(result.pid.is_some());

        // Clean up
        let _ = handle.stop().await;
    }

    #[test]
    fn test_daemon_ownership_tracking() {
        let config = test_config();
        let handle = DaemonHandle::new(config);
        assert!(!handle.owns_process);

        // After start, owns_process would be true
        // After stop, owns_process would be false
    }

    #[test]
    fn test_daemon_config_serialization() {
        let config = test_config();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: DaemonConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.executable, config.executable);
        assert_eq!(deserialized.args, config.args);
        assert_eq!(deserialized.gateway_url, config.gateway_url);
    }
}
