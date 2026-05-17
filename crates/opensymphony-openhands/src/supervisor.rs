use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    process::{Child, Command, Stdio},
    thread,
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime},
};

#[cfg(unix)]
use rustix::{
    io::Errno,
    process::{Pid, Signal, kill_process_group},
};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use reqwest::{blocking::Client, redirect::Policy};
use thiserror::Error;
use url::Url;

use super::tooling::{LocalServerTooling, LocalToolingError, ResolvedLaunch};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMode {
    Supervised,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchOwnership {
    Launched,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerState {
    Stopped,
    Ready,
    Unreachable,
    Exited { code: Option<i32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeConfig {
    pub path: String,
    pub poll_interval: Duration,
    pub connect_timeout: Duration,
}

impl Default for ProbeConfig {
    fn default() -> Self {
        Self {
            path: "/openapi.json".to_string(),
            poll_interval: Duration::from_millis(100),
            connect_timeout: Duration::from_millis(250),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisedServerConfig {
    pub tooling: LocalServerTooling,
    pub command: Option<Vec<String>>,
    pub port_override: Option<u16>,
    pub extra_env: BTreeMap<String, String>,
    pub startup_timeout: Duration,
    pub probe: ProbeConfig,
}

impl SupervisedServerConfig {
    pub fn new(tooling: LocalServerTooling) -> Self {
        Self {
            tooling,
            command: None,
            port_override: None,
            extra_env: BTreeMap::new(),
            startup_timeout: Duration::from_secs(10),
            probe: ProbeConfig::default(),
        }
    }

    fn base_url(&self) -> String {
        self.tooling.base_url(self.port_override)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalServerConfig {
    pub base_url: String,
    pub probe: ProbeConfig,
}

impl ExternalServerConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            probe: ProbeConfig::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorConfig {
    Supervised(Box<SupervisedServerConfig>),
    External(ExternalServerConfig),
}

impl SupervisorConfig {
    pub fn supervised(tooling: LocalServerTooling) -> Self {
        Self::Supervised(Box::new(SupervisedServerConfig::new(tooling)))
    }

    pub fn external(base_url: impl Into<String>) -> Self {
        Self::External(ExternalServerConfig::new(base_url))
    }

    fn mode(&self) -> ServerMode {
        match self {
            Self::Supervised(_) => ServerMode::Supervised,
            Self::External(_) => ServerMode::External,
        }
    }

    fn base_url(&self) -> String {
        match self {
            Self::Supervised(config) => config.base_url(),
            Self::External(config) => config.base_url.clone(),
        }
    }

    fn probe(&self) -> &ProbeConfig {
        match self {
            Self::Supervised(config) => &config.probe,
            Self::External(config) => &config.probe,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub mode: ServerMode,
    pub ownership: LaunchOwnership,
    pub state: ServerState,
    pub base_url: String,
    pub version: Option<String>,
    pub pid: Option<u32>,
    pub launched_at: Option<SystemTime>,
    pub launcher: Option<String>,
}

pub struct LocalServerSupervisor {
    config: SupervisorConfig,
    launched: Option<LaunchedProcess>,
    last_exit: Option<ExitedProcess>,
}

struct LaunchedProcess {
    child: Child,
    launch: ResolvedLaunch,
    launched_at: SystemTime,
    stderr_reader: Option<JoinHandle<()>>,
}

struct ExitedProcess {
    launch: ResolvedLaunch,
    launched_at: SystemTime,
    pid: u32,
    code: Option<i32>,
}

impl LocalServerSupervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self {
            config,
            launched: None,
            last_exit: None,
        }
    }

    pub fn start(&mut self) -> Result<ServerStatus, SupervisorError> {
        self.reap_exited_child()?;

        if self.launched.is_some() {
            return self.status();
        }

        self.last_exit = None;

        match &self.config {
            SupervisorConfig::External(config) => {
                if probe_external_ready(&config.base_url, &config.probe)? {
                    Ok(ServerStatus {
                        mode: ServerMode::External,
                        ownership: LaunchOwnership::External,
                        state: ServerState::Ready,
                        base_url: config.base_url.clone(),
                        version: None,
                        pid: None,
                        launched_at: None,
                        launcher: None,
                    })
                } else {
                    Err(SupervisorError::ExternalServerUnavailable {
                        base_url: config.base_url.clone(),
                        path: config.probe.path.clone(),
                    })
                }
            }
            SupervisorConfig::Supervised(config) => {
                let launch = config.tooling.resolve_launch(
                    config.port_override,
                    &config.extra_env,
                    config.command.as_deref(),
                )?;
                if probe_local_ready(&launch.base_url, &config.probe)? {
                    return Err(SupervisorError::ExistingReadyServer {
                        base_url: launch.base_url,
                        path: config.probe.path.clone(),
                    });
                }
                let mut command = Command::new(&launch.program);
                command
                    .args(&launch.args)
                    .current_dir(&launch.working_dir)
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped())
                    .envs(&launch.env);
                configure_server_command(&mut command);

                let mut child = command.spawn().map_err(|source| SupervisorError::Spawn {
                    program: format!("{} {}", launch.program, launch.args.join(" "))
                        .trim()
                        .to_string(),
                    source,
                })?;
                let mut stderr_reader = child.stderr.take().map(spawn_filtered_stderr_forwarder);
                let launched_at = SystemTime::now();
                let deadline = Instant::now() + config.startup_timeout;

                loop {
                    if let Some(status) =
                        child.try_wait().map_err(|source| SupervisorError::Wait {
                            pid: child.id(),
                            source,
                        })?
                    {
                        detach_output_reader(&mut stderr_reader);
                        return Err(SupervisorError::UnexpectedExit {
                            pid: child.id(),
                            code: status.code(),
                            base_url: launch.base_url.clone(),
                        });
                    }

                    if probe_local_ready(&launch.base_url, &config.probe)? {
                        let pid = child.id();
                        self.launched = Some(LaunchedProcess {
                            child,
                            launch: launch.clone(),
                            launched_at,
                            stderr_reader,
                        });

                        return Ok(ServerStatus {
                            mode: ServerMode::Supervised,
                            ownership: LaunchOwnership::Launched,
                            state: ServerState::Ready,
                            base_url: launch.base_url,
                            version: Some(launch.version),
                            pid: Some(pid),
                            launched_at: Some(launched_at),
                            launcher: Some(launch.launcher_summary),
                        });
                    }

                    if Instant::now() >= deadline {
                        let pid = child.id();
                        kill_child(&mut child)?;
                        join_output_reader(&mut stderr_reader);
                        return Err(SupervisorError::StartupTimeout {
                            base_url: launch.base_url,
                            path: config.probe.path.clone(),
                            timeout: config.startup_timeout,
                            pid,
                        });
                    }

                    thread::sleep(config.probe.poll_interval);
                }
            }
        }
    }

    pub fn stop(&mut self) -> Result<ServerStatus, SupervisorError> {
        if let Some(mut launched) = self.launched.take() {
            let pid = launched.child.id();
            self.last_exit = None;
            if launched
                .child
                .try_wait()
                .map_err(|source| SupervisorError::Wait { pid, source })?
                .is_none()
            {
                kill_child(&mut launched.child)?;
            }
            join_child_output(&mut launched);

            return Ok(ServerStatus {
                mode: ServerMode::Supervised,
                ownership: LaunchOwnership::Launched,
                state: ServerState::Stopped,
                base_url: launched.launch.base_url,
                version: Some(launched.launch.version),
                pid: Some(pid),
                launched_at: Some(launched.launched_at),
                launcher: Some(launched.launch.launcher_summary),
            });
        }

        self.status()
    }

    pub fn status(&mut self) -> Result<ServerStatus, SupervisorError> {
        self.reap_exited_child()?;

        if let Some(launched) = self.launched.as_mut() {
            let ready = probe_local_ready(&launched.launch.base_url, self.config.probe())?;
            return Ok(ServerStatus {
                mode: ServerMode::Supervised,
                ownership: LaunchOwnership::Launched,
                state: if ready {
                    ServerState::Ready
                } else {
                    ServerState::Unreachable
                },
                base_url: launched.launch.base_url.clone(),
                version: Some(launched.launch.version.clone()),
                pid: Some(launched.child.id()),
                launched_at: Some(launched.launched_at),
                launcher: Some(launched.launch.launcher_summary.clone()),
            });
        }

        if let Some(exited) = self.last_exit.as_ref() {
            return Ok(ServerStatus {
                mode: ServerMode::Supervised,
                ownership: LaunchOwnership::Launched,
                state: ServerState::Exited { code: exited.code },
                base_url: exited.launch.base_url.clone(),
                version: Some(exited.launch.version.clone()),
                pid: Some(exited.pid),
                launched_at: Some(exited.launched_at),
                launcher: Some(exited.launch.launcher_summary.clone()),
            });
        }

        let ready = match &self.config {
            SupervisorConfig::Supervised(_) => {
                probe_local_ready(&self.config.base_url(), self.config.probe())?
            }
            SupervisorConfig::External(_) => {
                probe_external_ready(&self.config.base_url(), self.config.probe())?
            }
        };
        Ok(ServerStatus {
            mode: self.config.mode(),
            ownership: match self.config {
                SupervisorConfig::Supervised(_) => LaunchOwnership::Launched,
                SupervisorConfig::External(_) => LaunchOwnership::External,
            },
            state: match self.config {
                SupervisorConfig::Supervised(_) => ServerState::Stopped,
                SupervisorConfig::External(_) if ready => ServerState::Ready,
                SupervisorConfig::External(_) => ServerState::Unreachable,
            },
            base_url: self.config.base_url(),
            version: match &self.config {
                SupervisorConfig::Supervised(config) => Some(config.tooling.version.clone()),
                SupervisorConfig::External(_) => None,
            },
            pid: None,
            launched_at: None,
            launcher: match &self.config {
                SupervisorConfig::Supervised(config) => {
                    Some(config.tooling.metadata.launcher.clone())
                }
                SupervisorConfig::External(_) => None,
            },
        })
    }

    fn reap_exited_child(&mut self) -> Result<(), SupervisorError> {
        if let Some(launched) = self.launched.as_mut() {
            let pid = launched.child.id();
            if let Some(status) = launched
                .child
                .try_wait()
                .map_err(|source| SupervisorError::Wait { pid, source })?
            {
                let launched = self.launched.take().expect("launched process should exist");
                let mut launched = launched;
                join_child_output(&mut launched);
                self.last_exit = Some(ExitedProcess {
                    launch: launched.launch,
                    launched_at: launched.launched_at,
                    pid,
                    code: status.code(),
                });
            }
        }

        Ok(())
    }

    fn stop_best_effort(&mut self) {
        if let Some(mut launched) = self.launched.take() {
            let _ = launched.child.try_wait();
            let _ = kill_child(&mut launched.child);
            join_child_output(&mut launched);
        }
        self.last_exit = None;
    }
}

fn spawn_filtered_stderr_forwarder(stderr: impl Read + Send + 'static) -> JoinHandle<()> {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut filter = OpenHandsLogFilter::default();
        let stderr = std::io::stderr();
        let mut output = stderr.lock();

        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if filter.should_forward(&line) {
                let _ = writeln!(output, "{line}");
            }
        }
    })
}

fn join_child_output(launched: &mut LaunchedProcess) {
    join_output_reader(&mut launched.stderr_reader);
}

fn join_output_reader(stderr_reader: &mut Option<JoinHandle<()>>) {
    if let Some(handle) = stderr_reader.take() {
        let _ = handle.join();
    }
}

fn detach_output_reader(stderr_reader: &mut Option<JoinHandle<()>>) {
    let _ = stderr_reader.take();
}

#[derive(Default)]
struct OpenHandsLogFilter {
    auto_title_suppression_remaining: usize,
}

impl OpenHandsLogFilter {
    fn should_forward(&mut self, line: &str) -> bool {
        if is_successful_openapi_access_log(line) {
            return false;
        }

        if self.auto_title_suppression_remaining > 0 {
            if line.contains("ValueError: No user messages found in conversation events") {
                self.auto_title_suppression_remaining = 0;
            } else {
                self.auto_title_suppression_remaining -= 1;
            }
            return false;
        }

        if line.contains("Auto-title generation failed for conversation") {
            self.auto_title_suppression_remaining = 80;
            return false;
        }

        true
    }
}

fn is_successful_openapi_access_log(line: &str) -> bool {
    line.contains("\"GET ")
        && line.contains("/openapi.json HTTP/1.1\" 200")
        && !line.contains(" 404 ")
        && !line.contains(" 500 ")
}

impl Drop for LocalServerSupervisor {
    fn drop(&mut self) {
        self.stop_best_effort();
    }
}

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error(transparent)]
    Tooling(#[from] LocalToolingError),
    #[error("OpenHands base URL must use http://host:port with no path, found `{base_url}`")]
    InvalidBaseUrl { base_url: String },
    #[error(
        "OpenHands external base URL must be an absolute http or https URL without query or fragment, found `{base_url}`"
    )]
    InvalidExternalBaseUrl { base_url: String },
    #[error("failed to resolve socket address for `{base_url}`: {source}")]
    ResolveAddress {
        base_url: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to spawn local OpenHands server with `{program}`: {source}")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "local OpenHands server exited before readiness at {base_url} (pid {pid}, exit code {code:?})"
    )]
    UnexpectedExit {
        pid: u32,
        code: Option<i32>,
        base_url: String,
    },
    #[error(
        "local OpenHands server did not become ready within {timeout:?} at {base_url}{path} (pid {pid})"
    )]
    StartupTimeout {
        base_url: String,
        path: String,
        timeout: Duration,
        pid: u32,
    },
    #[error("external OpenHands server is not ready at {base_url}{path}")]
    ExternalServerUnavailable { base_url: String, path: String },
    #[error(
        "refusing to launch supervised OpenHands server because another ready server is already responding at {base_url}{path}"
    )]
    ExistingReadyServer { base_url: String, path: String },
    #[error("failed to wait for local OpenHands server pid {pid}: {source}")]
    Wait {
        pid: u32,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to stop local OpenHands server pid {pid}: {source}")]
    Kill {
        pid: u32,
        #[source]
        source: std::io::Error,
    },
    #[error("failed readiness probe against {base_url}{path}: {source}")]
    ProbeIo {
        base_url: String,
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed readiness probe against {base_url}{path}: {source}")]
    ProbeHttp {
        base_url: String,
        path: String,
        #[source]
        source: reqwest::Error,
    },
}

fn kill_child(child: &mut Child) -> Result<(), SupervisorError> {
    let pid = child.id();
    if child
        .try_wait()
        .map_err(|source| SupervisorError::Wait { pid, source })?
        .is_none()
    {
        terminate_child_process_tree(child, pid)?;
    }

    child
        .wait()
        .map_err(|source| SupervisorError::Wait { pid, source })?;
    Ok(())
}

#[cfg(unix)]
fn configure_server_command(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_server_command(_command: &mut Command) {}

#[cfg(unix)]
fn terminate_child_process_tree(child: &mut Child, pid: u32) -> Result<(), SupervisorError> {
    let process_id = i32::try_from(pid).map_err(|source| SupervisorError::Kill {
        pid,
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("local OpenHands server pid {pid} does not fit in i32: {source}"),
        ),
    })?;
    let process_group = Pid::from_raw(process_id).ok_or_else(|| SupervisorError::Kill {
        pid,
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("local OpenHands server pid {pid} is not a valid Unix pid"),
        ),
    })?;

    match kill_process_group(process_group, Signal::KILL) {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(source) => Err(SupervisorError::Kill {
            pid,
            source: source.into(),
        }),
    }?;

    if child
        .try_wait()
        .map_err(|source| SupervisorError::Wait { pid, source })?
        .is_none()
    {
        child
            .kill()
            .map_err(|source| SupervisorError::Kill { pid, source })?;
    }

    Ok(())
}

#[cfg(not(unix))]
fn terminate_child_process_tree(child: &mut Child, pid: u32) -> Result<(), SupervisorError> {
    child
        .kill()
        .map_err(|source| SupervisorError::Kill { pid, source })
}

fn probe_local_ready(base_url: &str, probe: &ProbeConfig) -> Result<bool, SupervisorError> {
    let endpoint = HttpEndpoint::parse(base_url)?;
    let addresses = endpoint.socket_addresses(base_url)?;
    probe_resolved_addresses(
        base_url,
        &endpoint.host,
        normalized_probe_path(&probe.path),
        &addresses,
        probe.connect_timeout,
    )
}

fn probe_external_ready(base_url: &str, probe: &ProbeConfig) -> Result<bool, SupervisorError> {
    let endpoint = external_probe_url(base_url, &probe.path)?;
    let client = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(probe.connect_timeout)
        .build()
        .map_err(|source| SupervisorError::ProbeHttp {
            base_url: base_url.to_string(),
            path: endpoint.path().to_string(),
            source,
        })?;

    match client.get(endpoint.clone()).send() {
        Ok(response) => Ok(response.status().is_success()),
        Err(source) if source.is_timeout() => Ok(false),
        Err(source) if source.is_connect() => Ok(false),
        Err(source) => Err(SupervisorError::ProbeHttp {
            base_url: base_url.to_string(),
            path: endpoint.path().to_string(),
            source,
        }),
    }
}

fn external_probe_url(base_url: &str, probe_path: &str) -> Result<Url, SupervisorError> {
    let mut url = Url::parse(base_url).map_err(|_| SupervisorError::InvalidExternalBaseUrl {
        base_url: base_url.to_string(),
    })?;

    match url.scheme() {
        "http" | "https" => {}
        _ => {
            return Err(SupervisorError::InvalidExternalBaseUrl {
                base_url: base_url.to_string(),
            });
        }
    }

    if url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(SupervisorError::InvalidExternalBaseUrl {
            base_url: base_url.to_string(),
        });
    }

    let base_path = url.path().trim_end_matches('/');
    let probe_path = normalized_probe_path(probe_path);
    let path = if base_path.is_empty() {
        probe_path.to_string()
    } else {
        format!("{base_path}{probe_path}")
    };
    url.set_path(&path);
    Ok(url)
}

fn probe_resolved_addresses(
    base_url: &str,
    host: &str,
    path: &str,
    addresses: &[std::net::SocketAddr],
    timeout: Duration,
) -> Result<bool, SupervisorError> {
    let mut first_fatal = None;

    for address in addresses {
        match probe_address(base_url, host, path, *address, timeout) {
            Ok(true) => return Ok(true),
            Ok(false) => continue,
            Err(ProbeAttempt::Transient) => continue,
            Err(ProbeAttempt::Fatal(error)) if first_fatal.is_none() => {
                first_fatal = Some(error);
            }
            Err(ProbeAttempt::Fatal(_)) => {}
        }
    }

    if let Some(error) = first_fatal {
        Err(error)
    } else {
        Ok(false)
    }
}

fn probe_address(
    base_url: &str,
    host: &str,
    path: &str,
    address: std::net::SocketAddr,
    timeout: Duration,
) -> Result<bool, ProbeAttempt> {
    let stream = match TcpStream::connect_timeout(&address, timeout) {
        Ok(stream) => stream,
        Err(source) if is_transient_connection_error(&source) => {
            return Err(ProbeAttempt::Transient);
        }
        Err(source) => {
            return Err(ProbeAttempt::Fatal(SupervisorError::ProbeIo {
                base_url: base_url.to_string(),
                path: path.to_string(),
                source,
            }));
        }
    };

    stream.set_read_timeout(Some(timeout)).map_err(|source| {
        ProbeAttempt::Fatal(SupervisorError::ProbeIo {
            base_url: base_url.to_string(),
            path: path.to_string(),
            source,
        })
    })?;
    stream.set_write_timeout(Some(timeout)).map_err(|source| {
        ProbeAttempt::Fatal(SupervisorError::ProbeIo {
            base_url: base_url.to_string(),
            path: path.to_string(),
            source,
        })
    })?;

    let mut stream = stream;
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host
    )
    .map_err(|source| transient_probe_error(base_url, path, source))?;

    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|source| transient_probe_error(base_url, path, source))?;

    let status_line = response.lines().next().unwrap_or_default();
    Ok(status_line.contains(" 200 "))
}

enum ProbeAttempt {
    Transient,
    Fatal(SupervisorError),
}

fn normalized_probe_path(path: &str) -> &str {
    if path.starts_with('/') {
        path
    } else {
        "/openapi.json"
    }
}

fn is_transient_connection_error(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::AddrNotAvailable
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::WouldBlock
    )
}

fn transient_probe_error(base_url: &str, path: &str, source: std::io::Error) -> ProbeAttempt {
    if is_transient_connection_error(&source) {
        ProbeAttempt::Transient
    } else {
        ProbeAttempt::Fatal(SupervisorError::ProbeIo {
            base_url: base_url.to_string(),
            path: path.to_string(),
            source,
        })
    }
}

struct HttpEndpoint {
    host: String,
    port: u16,
}

impl HttpEndpoint {
    fn parse(base_url: &str) -> Result<Self, SupervisorError> {
        let without_scheme =
            base_url
                .strip_prefix("http://")
                .ok_or_else(|| SupervisorError::InvalidBaseUrl {
                    base_url: base_url.to_string(),
                })?;

        if without_scheme.contains('/') {
            return Err(SupervisorError::InvalidBaseUrl {
                base_url: base_url.to_string(),
            });
        }

        let (host, port) =
            without_scheme
                .rsplit_once(':')
                .ok_or_else(|| SupervisorError::InvalidBaseUrl {
                    base_url: base_url.to_string(),
                })?;
        let port = port
            .parse::<u16>()
            .map_err(|_| SupervisorError::InvalidBaseUrl {
                base_url: base_url.to_string(),
            })?;

        Ok(Self {
            host: host.to_string(),
            port,
        })
    }

    fn socket_addresses(
        &self,
        base_url: &str,
    ) -> Result<Vec<std::net::SocketAddr>, SupervisorError> {
        let addresses: Vec<_> = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(|source| SupervisorError::ResolveAddress {
                base_url: base_url.to_string(),
                source,
            })?
            .collect();
        if addresses.is_empty() {
            return Err(SupervisorError::InvalidBaseUrl {
                base_url: base_url.to_string(),
            });
        }

        Ok(addresses)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, SocketAddr, TcpListener},
        thread,
        time::Duration,
    };

    use super::{OpenHandsLogFilter, is_successful_openapi_access_log, probe_resolved_addresses};

    #[test]
    fn probe_resolved_addresses_tries_later_candidates_after_transient_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let port = listener
            .local_addr()
            .expect("listener address should resolve")
            .port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 256];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let bytes_read = stream.read(&mut chunk).expect("request should read");
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..bytes_read]);
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
            .expect("response should write");
            stream.flush().expect("response should flush");
            stream
                .shutdown(Shutdown::Both)
                .expect("stream should shut down");
        });

        let unreachable = SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port));
        let reachable = SocketAddr::from(([127, 0, 0, 1], port));
        let ready = probe_resolved_addresses(
            &format!("http://localhost:{port}"),
            "localhost",
            "/openapi.json",
            &[unreachable, reachable],
            Duration::from_millis(250),
        )
        .expect("probe should succeed");

        assert!(ready);
        server.join().expect("server thread should finish");
    }

    #[test]
    fn openapi_access_filter_drops_successful_probe_noise_only() {
        assert!(is_successful_openapi_access_log(
            r#"INFO:     127.0.0.1:60000 - "GET /openapi.json HTTP/1.1" 200 OK"#
        ));
        assert!(is_successful_openapi_access_log(
            r#"INFO:     127.0.0.1:60000 - "GET /runtime/openapi.json HTTP/1.1" 200 OK"#
        ));
        assert!(!is_successful_openapi_access_log(
            r#"INFO:     127.0.0.1:60000 - "GET /openapi.json HTTP/1.1" 500 Internal Server Error"#
        ));
        assert!(!is_successful_openapi_access_log(
            r#"INFO:     127.0.0.1:60000 - "GET /api/conversations HTTP/1.1" 200 OK"#
        ));
    }

    #[test]
    fn openhands_log_filter_suppresses_empty_conversation_auto_title_traceback() {
        let mut filter = OpenHandsLogFilter::default();
        let lines = [
            "WARNING  Auto-title generation failed for conversation 87b36f15-7bf8-4740-8877-ccd2924d98a8",
            "Traceback (most recent call last):",
            "  File \"/path/to/title.py\", line 10, in generate",
            "ValueError: No user messages found in conversation events",
            "INFO:     server still healthy",
        ];
        let forwarded = lines
            .iter()
            .filter(|line| filter.should_forward(line))
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(forwarded, vec!["INFO:     server still healthy"]);
    }
}
