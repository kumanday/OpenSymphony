//! Local supervision for one shared OpenHands agent-server subprocess.

use std::collections::BTreeSet;

use tokio::process::{Child, Command};
use tracing::debug;

use crate::client::OpenHandsClient;
use crate::config::{LocalServerConfig, TransportConfig};
use crate::error::{OpenHandsError, Result};
use crate::wire::ServerInfo;

/// Snapshot of the supervised server state.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerStatus {
    /// Whether the process was launched by this supervisor.
    pub launched_by_supervisor: bool,
    /// Child process identifier when one exists.
    pub pid: Option<u32>,
    /// Whether a readiness probe is currently succeeding.
    pub ready: bool,
    /// Public server base URL.
    pub base_url: url::Url,
    /// Launch command used in supervised mode.
    pub command: Vec<String>,
    /// Optional server metadata retrieved from `/server_info`.
    pub server_info: Option<ServerInfo>,
}

/// Supervisor for the local MVP's single shared agent-server process.
#[derive(Debug)]
pub struct LocalAgentServerSupervisor {
    client: OpenHandsClient,
    config: LocalServerConfig,
    child: Option<Child>,
}

impl LocalAgentServerSupervisor {
    /// Builds a new supervisor from transport and local-process settings.
    pub fn new(transport: TransportConfig, config: LocalServerConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            client: OpenHandsClient::new(transport),
            config,
            child: None,
        })
    }

    /// Starts the supervised process and waits for readiness.
    pub async fn start(&mut self) -> Result<ServerStatus> {
        if self.child.is_some() {
            return self.status().await;
        }

        let mut command = Command::new(&self.config.command[0]);
        command.args(&self.config.command[1..]);
        command.kill_on_drop(true);
        if let Some(workdir) = &self.config.workdir {
            command.current_dir(workdir);
        }
        for (key, value) in &self.config.env {
            command.env(key, value);
        }

        let mut child = command.spawn()?;
        let pid = child.id();
        let deadline = tokio::time::Instant::now()
            + std::time::Duration::from_millis(self.config.startup_timeout_ms);

        loop {
            if let Some(exit_status) = child.try_wait()? {
                return Err(OpenHandsError::ProcessExited {
                    message: format!("child exited before readiness with status {exit_status}"),
                });
            }

            if self.probe_ready().await {
                debug!(pid, "local OpenHands server reached readiness");
                self.child = Some(child);
                return self.status().await;
            }

            if tokio::time::Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(OpenHandsError::Timeout {
                    operation: "local OpenHands server startup",
                    timeout: std::time::Duration::from_millis(self.config.startup_timeout_ms),
                });
            }

            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
    }

    /// Stops the supervised process if this supervisor launched it.
    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }

    /// Returns the latest supervisor status.
    pub async fn status(&mut self) -> Result<ServerStatus> {
        let mut launched_by_supervisor = false;
        let mut pid = None;
        if let Some(child) = self.child.as_mut() {
            launched_by_supervisor = true;
            pid = child.id();
            if let Some(exit_status) = child.try_wait()? {
                self.child = None;
                return Err(OpenHandsError::ProcessExited {
                    message: format!("child exited unexpectedly with status {exit_status}"),
                });
            }
        }

        let ready = self.probe_ready().await;
        let server_info = if ready {
            self.client.server_info().await.ok()
        } else {
            None
        };
        Ok(ServerStatus {
            launched_by_supervisor,
            pid,
            ready,
            base_url: self.client.transport().base_url.clone(),
            command: self.config.command.clone(),
            server_info,
        })
    }

    async fn probe_ready(&self) -> bool {
        for path in readiness_probe_candidates(&self.config.readiness_probe_path) {
            if self.client.probe_path(&path).await.is_ok() {
                return true;
            }
        }
        false
    }
}

fn readiness_probe_candidates(preferred: &str) -> Vec<String> {
    let mut unique = BTreeSet::new();
    let mut ordered = Vec::new();
    for candidate in [preferred, "/ready", "/health", "/openapi.json"] {
        let normalized = if candidate.starts_with('/') {
            candidate.to_string()
        } else {
            format!("/{candidate}")
        };
        if unique.insert(normalized.clone()) {
            ordered.push(normalized);
        }
    }
    ordered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_candidates_prefer_configured_path_without_duplicates() {
        let candidates = readiness_probe_candidates("/ready");
        assert_eq!(candidates, vec!["/ready", "/health", "/openapi.json"]);
    }
}
