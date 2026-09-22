//! Connection-owned client facilities. Callback containment is not a process sandbox.
use super::{CallbackOutput, ClientError, ClientLimits};
#[cfg(not(windows))]
use crate::opensymphony_workspace::{configure_process_group, terminate_process_tree};
use crate::opensymphony_workspace::{
    environment_variable_names_equal, has_environment_name_collision, resolve_path_within_root,
};
use agent_client_protocol::{
    Error, Responder,
    schema::v1::{ClientCapabilities, FileSystemCapabilities, McpServer},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

/// Immutable host policy and resolved, scoped MCP grants. Never populated from IDE attachment.
#[derive(Default, Clone)]
pub struct HostServices {
    pub read_files: bool,
    pub write_files: bool,
    pub terminals: bool,
    pub mcp_servers: Vec<McpServer>,
}
impl HostServices {
    pub(super) fn capabilities(&self) -> ClientCapabilities {
        ClientCapabilities::default()
            .fs(FileSystemCapabilities::default()
                .read_text_file(self.read_files)
                .write_text_file(self.write_files))
            .terminal(self.terminals)
    }
}

pub(super) struct Callback {
    pub method: String,
    pub params: Value,
    pub responder: Responder,
    pub permit: OwnedSemaphorePermit,
    pub bytes: OwnedSemaphorePermit,
}
enum ServiceCommand {
    Callback(Box<Callback>),
    BeginTurn(CancellationToken, oneshot::Sender<Result<(), Error>>),
}
#[derive(Clone)]
pub(super) struct CallbackSender {
    tx: mpsc::Sender<ServiceCommand>,
    permits: Arc<Semaphore>,
    bytes: Arc<Semaphore>,
}
impl CallbackSender {
    /// Retire the previous callback epoch before a retained owner submits another prompt.
    pub async fn begin_turn(
        &self,
        cancellation: CancellationToken,
        timeout: Duration,
    ) -> Result<(), ClientError> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(ServiceCommand::BeginTurn(cancellation.clone(), tx))
            .map_err(|_| ClientError::Teardown)?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(ClientError::CancelledBeforePrompt),
            result = tokio::time::timeout(timeout, rx) => {
                result.map_err(|_| ClientError::SetupTimeout)?
                    .map_err(|_| ClientError::Teardown)?
                    .map_err(|_| ClientError::Teardown)
            }
        }
    }
    pub fn enqueue(
        &self,
        method: String,
        params: Value,
        responder: Responder,
    ) -> Result<(), Error> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::internal_error())?;
        let size = serde_json::to_vec(&params).map_err(Error::from)?.len()
            + serde_json::to_vec(responder.id())
                .map_err(Error::from)?
                .len()
            + method.len();
        let bytes = self
            .bytes
            .clone()
            .try_acquire_many_owned(u32::try_from(size).map_err(|_| Error::internal_error())?)
            .map_err(|_| Error::internal_error())?;
        self.tx
            .try_send(ServiceCommand::Callback(Box::new(Callback {
                method,
                params,
                responder,
                permit,
                bytes,
            })))
            .map_err(|_| Error::internal_error())
    }
}

pub(super) struct Services {
    root: PathBuf,
    environment: BTreeMap<String, String>,
    policy: HostServices,
    limits: ClientLimits,
    rx: mpsc::Receiver<ServiceCommand>,
    terminals: BTreeMap<String, Terminal>,
    processes: JoinSet<bool>,
    processes_ok: bool,
    replies: JoinSet<()>,
    output: Arc<Mutex<CallbackOutput>>,
    resource_failure: Arc<std::sync::atomic::AtomicBool>,
    fatal: CancellationToken,
    cancellation: CancellationToken,
    shutdown: CancellationToken,
}
impl Services {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        environment: BTreeMap<String, String>,
        policy: HostServices,
        limits: ClientLimits,
        output: Arc<Mutex<CallbackOutput>>,
        resource_failure: Arc<std::sync::atomic::AtomicBool>,
        fatal: CancellationToken,
        cancellation: CancellationToken,
        shutdown: CancellationToken,
    ) -> (CallbackSender, Self) {
        let (tx, rx) = mpsc::channel(limits.pending_callbacks);
        let sender = CallbackSender {
            tx,
            permits: Arc::new(Semaphore::new(limits.pending_callbacks)),
            bytes: Arc::new(Semaphore::new(limits.queued_bytes)),
        };
        (
            sender,
            Self {
                root,
                environment,
                policy,
                limits,
                rx,
                terminals: BTreeMap::new(),
                processes: JoinSet::new(),
                processes_ok: true,
                replies: JoinSet::new(),
                output,
                resource_failure,
                fatal,
                cancellation,
                shutdown,
            },
        )
    }

    pub async fn run(mut self) -> bool {
        loop {
            tokio::select! {
                biased;
                _ = self.shutdown.cancelled() => break,
                Some(result) = self.processes.join_next(), if !self.processes.is_empty() => { self.processes_ok &= matches!(result, Ok(true)); },
                Some(_) = self.replies.join_next(), if !self.replies.is_empty() => {},
                request = self.rx.recv() => {
                    let request = match request {
                        None => break,
                        Some(ServiceCommand::Callback(request)) => *request,
                        Some(ServiceCommand::BeginTurn(cancellation, reply)) => {
                            let result = self.retire_turn().await.map(|()| self.cancellation = cancellation.child_token());
                            if result.is_err() { self.fatal.cancel(); }
                            let _ = reply.send(result);
                            continue;
                        }
                    };
                    let shutdown = self.shutdown.clone();
                    let result = tokio::select! {
                        biased;
                        _ = shutdown.cancelled() => break,
                        result = self.handle_callback(&request.method, request.params) => result,
                    };
                    let output = self.output.clone();
                    let resource_failure = self.resource_failure.clone();
                    let fatal = self.fatal.clone();
                    let cancellation = self.cancellation.clone();
                    let limits = self.limits.clone();
                    self.replies.spawn(async move {
                        let _permit = request.permit;
                        let _bytes = request.bytes;
                        let result = match result {
                            Ok(Reply::Ready(value)) => Ok(value),
                            Ok(Reply::Wait(mut terminal, release)) => tokio::select! {
                                result = tokio::time::timeout(limits.callback_timeout, terminal.wait_for(|state| state.exit.is_some())) => {
                                    match result {
                                        Ok(Ok(state)) => state.exit.clone().unwrap_or_else(|| Err(Error::internal_error())).map(|exit| if release { json!({}) } else { exit }),
                                        _ => Err(Error::new(-32603, "terminal wait expired")),
                                    }
                                },
                                _ = cancellation.cancelled() => Err(Error::new(-32603, "session cancelled")),
                            },
                            Err(error) => Err(error),
                        };
                        let result = bounded_response(result, request.responder.id(), &limits);
                        let frame = serde_json::to_string(&agent_client_protocol::RawJsonRpcMessage::response(request.responder.id().clone(), result.clone()));
                        // Reservation and SDK enqueue are one critical section: asynchronous
                        // waits may finish together, but the writer must see matching FIFO order.
                        let mut output = output.lock().unwrap_or_else(|e| e.into_inner());
                        if frame.is_ok_and(|frame| output.admit(frame, &limits)) {
                            if request.responder.respond_with_result(result).is_err() { fatal.cancel(); }
                        } else {
                            resource_failure.store(true, std::sync::atomic::Ordering::Release);
                            fatal.cancel();
                        }
                    });
                }
            }
        }
        self.rx.close();
        for terminal in self.terminals.values() {
            terminal.kill.cancel();
        }
        self.replies.abort_all();
        // Wait for every owned process task to kill its tree and reap its child.
        let drained =
            tokio::time::timeout(self.limits.reap_timeout + self.limits.reap_timeout, async {
                while let Some(result) = self.processes.join_next().await {
                    self.processes_ok &= matches!(result, Ok(true));
                }
            })
            .await
            .is_ok();
        drained
            && self.processes_ok
            && self
                .terminals
                .values()
                .all(|t| t.state.borrow().exit.as_ref().is_some_and(Result::is_ok))
    }

    async fn retire_turn(&mut self) -> Result<(), Error> {
        if !self.processes_ok {
            return Err(Error::internal_error());
        }
        self.cancellation.cancel();
        for terminal in self.terminals.values() {
            terminal.kill.cancel();
        }
        tokio::time::timeout(self.limits.reap_timeout + self.limits.reap_timeout, async {
            while let Some(result) = self.processes.join_next().await {
                if !matches!(result, Ok(true)) {
                    return Err(Error::internal_error());
                }
            }
            while let Some(result) = self.replies.join_next().await {
                if result.is_err() {
                    return Err(Error::internal_error());
                }
            }
            if self.terminals.values().any(|terminal| {
                !terminal
                    .state
                    .borrow()
                    .exit
                    .as_ref()
                    .is_some_and(Result::is_ok)
            }) {
                return Err(Error::internal_error());
            }
            self.terminals.clear();
            Ok(())
        })
        .await
        .map_err(|_| Error::internal_error())?
    }

    async fn handle_callback(&mut self, method: &str, params: Value) -> Result<Reply, Error> {
        let cancellation = self.cancellation.clone();
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(Error::new(-32603, "session cancelled")),
            result = tokio::time::timeout(self.limits.callback_timeout, self.handle(method, params)) =>
                result.unwrap_or_else(|_| Err(Error::new(-32603, "callback deadline exceeded"))),
        }
    }

    async fn handle(&mut self, method: &str, params: Value) -> Result<Reply, Error> {
        match method {
            "fs/read_text_file" if self.policy.read_files => {
                let request: ReadFile =
                    serde_json::from_value(params).map_err(|_| Error::invalid_params())?;
                let path = self.path(&request.path, false).await?;
                #[cfg(windows)]
                let (mut file, _path_guards) = super::windows_path::open_file(&path, false)
                    .await
                    .map_err(io_error)?;
                #[cfg(not(windows))]
                let mut file = self.open_file(&path, false).await?;
                if !file.metadata().await.map_err(io_error)?.is_file() {
                    return Err(Error::invalid_params());
                }
                let mut bytes = Vec::new();
                (&mut file)
                    .take(self.limits.file_bytes as u64 + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(io_error)?;
                if bytes.len() > self.limits.file_bytes || request.line == Some(0) {
                    return Err(Error::invalid_params());
                }
                let text = String::from_utf8(bytes).map_err(|_| Error::invalid_params())?;
                let content = if request.line.is_none() && request.limit.is_none() {
                    text
                } else {
                    text.split_inclusive('\n')
                        .skip(request.line.unwrap_or(1) as usize - 1)
                        .take(request.limit.map_or(usize::MAX, |n| n as usize))
                        .collect()
                };
                Ok(Reply::Ready(json!({"content":content})))
            }
            "fs/write_text_file" if self.policy.write_files => {
                let request: WriteFile =
                    serde_json::from_value(params).map_err(|_| Error::invalid_params())?;
                if request.content.len() > self.limits.file_bytes {
                    return Err(Error::invalid_params());
                }
                let path = self.path(&request.path, true).await?;
                // Check the complete path before creating parents; nonexistent leaves are
                // validated against the nearest existing ancestor, including dangling links.
                #[cfg(windows)]
                let (mut file, _path_guards) = super::windows_path::open_file(&path, true)
                    .await
                    .map_err(io_error)?;
                #[cfg(not(windows))]
                let mut file = self.open_file(&path, true).await?;
                file.set_len(0).await.map_err(io_error)?;
                file.write_all(request.content.as_bytes())
                    .await
                    .map_err(io_error)?;
                file.flush().await.map_err(io_error)?;
                Ok(Reply::Ready(json!({})))
            }
            "terminal/create" if self.policy.terminals => {
                let request: CreateTerminal =
                    serde_json::from_value(params).map_err(|_| Error::invalid_params())?;
                if self.terminals.len() >= self.limits.terminal_count
                    || self.processes.len() >= self.limits.terminal_count
                    || request.command.is_empty()
                    || request.command.len() > 8192
                    || request.command.contains('\0')
                    || request.args.len() > 128
                    || request
                        .args
                        .iter()
                        .any(|v| v.len() > 8192 || v.contains('\0'))
                    || request.env.len() > 128
                    || has_environment_name_collision(request.env.iter().map(|v| v.name.as_str()))
                {
                    return Err(Error::invalid_params());
                }
                // Per-command overrides may only repeat host-owned values. This keeps
                // scoped grants, credentials and executable search paths immutable.
                if request.env.iter().any(|v| {
                    !self.environment.iter().any(|(name, value)| {
                        environment_variable_names_equal(name, &v.name) && value == &v.value
                    })
                }) {
                    return Err(Error::invalid_params());
                }
                let cwd = self
                    .path(request.cwd.as_deref().unwrap_or(&self.root), false)
                    .await?;
                if !tokio::fs::metadata(&cwd).await.map_err(io_error)?.is_dir() {
                    return Err(Error::invalid_params());
                }
                let output_limit = request
                    .output_byte_limit
                    .unwrap_or(self.limits.terminal_output_bytes as u64);
                if output_limit > self.limits.terminal_output_bytes as u64 {
                    return Err(Error::invalid_params());
                }
                #[cfg(windows)]
                let _cwd_guards = super::windows_path::pin_directory(&cwd, false)
                    .await
                    .map_err(io_error)?;
                let mut command = Command::new(request.command);
                command
                    .args(request.args)
                    .env_clear()
                    .envs(&self.environment)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                #[cfg(not(unix))]
                command.current_dir(&cwd);
                #[cfg(unix)]
                pin_terminal_cwd(&mut command, &self.root, &cwd).map_err(io_error)?;
                #[cfg(not(windows))]
                configure_process_group(&mut command);
                #[cfg(not(windows))]
                let child = command.spawn().map_err(io_error)?;
                #[cfg(windows)]
                let child =
                    super::windows_process::WindowsChild::spawn(command).map_err(io_error)?;
                let id = uuid::Uuid::new_v4().to_string();
                let kill = self.cancellation.child_token();
                let (tx, state) = watch::channel(TerminalState::default());
                self.processes.spawn(run_terminal(
                    #[cfg(unix)]
                    crate::opensymphony_workspace::ProcessGroupGuard::new(child.id()),
                    child,
                    output_limit as usize,
                    kill.clone(),
                    self.shutdown.clone(),
                    tx,
                    self.limits.reap_timeout,
                ));
                self.terminals.insert(id.clone(), Terminal { state, kill });
                Ok(Reply::Ready(json!({"terminalId":id})))
            }
            "terminal/output" | "terminal/wait_for_exit" | "terminal/kill" | "terminal/release"
                if self.policy.terminals =>
            {
                let request: TerminalRequest =
                    serde_json::from_value(params).map_err(|_| Error::invalid_params())?;
                let terminal = self
                    .terminals
                    .get(&request.terminal_id)
                    .ok_or_else(Error::invalid_params)?;
                match method {
                    "terminal/output" => {
                        let state = terminal.state.borrow();
                        if state.exit.as_ref().is_some_and(Result::is_err) {
                            return Err(Error::internal_error());
                        }
                        Ok(Reply::Ready(
                            json!({"output":state.output,"truncated":state.truncated,"exitStatus":state.exit.as_ref().and_then(|v|v.as_ref().ok())}),
                        ))
                    }
                    "terminal/kill" => {
                        terminal.kill.cancel();
                        Ok(Reply::Wait(terminal.state.clone(), true))
                    }
                    "terminal/release" => {
                        let terminal = self
                            .terminals
                            .remove(&request.terminal_id)
                            .ok_or_else(Error::invalid_params)?;
                        terminal.kill.cancel();
                        Ok(Reply::Wait(terminal.state, true))
                    }
                    _ => Ok(Reply::Wait(terminal.state.clone(), false)),
                }
            }
            _ => Err(Error::method_not_found()),
        }
    }

    #[cfg(not(windows))]
    async fn open_file(&self, path: &Path, write: bool) -> Result<tokio::fs::File, Error> {
        #[cfg(unix)]
        {
            use rustix::fs::{FileType, Mode, OFlags, fstat, mkdirat, open, openat};
            let mut directory = open(
                &self.root,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|e| io_error(e.into()))?;
            let relative = path
                .strip_prefix(&self.root)
                .map_err(|_| Error::invalid_params())?;
            let mut parts = relative.components().peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    let flags = OFlags::NOFOLLOW
                        | OFlags::NONBLOCK
                        | OFlags::CLOEXEC
                        | if write {
                            OFlags::WRONLY | OFlags::CREATE
                        } else {
                            OFlags::RDONLY
                        };
                    let file = openat(
                        &directory,
                        part.as_os_str(),
                        flags,
                        Mode::from_bits_truncate(0o600),
                    )
                    .map_err(|e| io_error(e.into()))?;
                    if FileType::from_raw_mode(
                        fstat(&file).map_err(|e| io_error(e.into()))?.st_mode,
                    ) != FileType::RegularFile
                    {
                        return Err(Error::invalid_params());
                    }
                    return Ok(tokio::fs::File::from_std(std::fs::File::from(file)));
                }
                if write {
                    match mkdirat(
                        &directory,
                        part.as_os_str(),
                        Mode::from_bits_truncate(0o700),
                    ) {
                        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
                        Err(error) => return Err(io_error(error.into())),
                    }
                }
                directory = openat(
                    &directory,
                    part.as_os_str(),
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|e| io_error(e.into()))?;
            }
            Err(Error::invalid_params())
        }
        #[cfg(not(unix))]
        {
            if write {
                tokio::fs::create_dir_all(path.parent().ok_or_else(Error::invalid_params)?)
                    .await
                    .map_err(io_error)?;
            }
            self.path(path, write).await?;
            if tokio::fs::metadata(path).await.is_ok_and(|m| !m.is_file()) {
                return Err(Error::invalid_params());
            }
            tokio::fs::OpenOptions::new()
                .read(!write)
                .write(write)
                .create(write)
                .open(path)
                .await
                .map_err(io_error)
        }
    }

    async fn path(&self, path: &Path, missing: bool) -> Result<PathBuf, Error> {
        if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(Error::invalid_params());
        }
        let path =
            resolve_path_within_root(&self.root, path).map_err(|_| Error::invalid_params())?;
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| Error::invalid_params())?;
        let mut current = self.root.clone();
        for part in relative.components() {
            current.push(part);
            match tokio::fs::symlink_metadata(&current).await {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let canonical = tokio::fs::canonicalize(&current).await.map_err(io_error)?;
                    if !canonical.starts_with(&self.root) {
                        return Err(Error::invalid_params());
                    }
                }
                Ok(_) => {}
                Err(error) if missing && error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(io_error(error)),
            }
        }
        Ok(path)
    }
}

#[cfg(unix)]
fn pin_terminal_cwd(command: &mut Command, root: &Path, cwd: &Path) -> std::io::Result<()> {
    use rustix::fs::{Mode, OFlags, open, openat};
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = open(root, flags, Mode::empty())?;
    let relative = cwd
        .strip_prefix(root)
        .map_err(|_| std::io::ErrorKind::InvalidInput)?;
    for component in relative.components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        directory = openat(&directory, component.as_os_str(), flags, Mode::empty())?;
    }
    set_child_directory(command, directory);
    Ok(())
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn set_child_directory(command: &mut Command, directory: std::os::fd::OwnedFd) {
    // SAFETY: the closure only invokes async-signal-safe fchdir on the owned,
    // already-contained directory. It allocates nothing, takes no locks and
    // changes only the forked child's cwd. Command owns the descriptor through
    // spawn; CLOEXEC closes the child copy after the cwd has been established.
    unsafe {
        command.pre_exec(move || rustix::process::fchdir(&directory).map_err(std::io::Error::from));
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadFile {
    path: PathBuf,
    line: Option<u32>,
    limit: Option<u32>,
}
#[derive(Deserialize)]
struct WriteFile {
    path: PathBuf,
    content: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateTerminal {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: Vec<Env>,
    cwd: Option<PathBuf>,
    output_byte_limit: Option<u64>,
}
#[derive(Deserialize)]
struct Env {
    name: String,
    value: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminalRequest {
    terminal_id: String,
}
// Bound the complete encoded frame, including the peer's request ID and JSON
// escaping. An oversized file is a callback error; terminal tails are truncatable.
fn bounded_response(
    mut result: Result<Value, Error>,
    id: &agent_client_protocol::schema::v1::RequestId,
    limits: &ClientLimits,
) -> Result<Value, Error> {
    let budget = limits
        .frame_bytes
        .min(limits.callback_bytes.saturating_sub(1));
    let fits = |result: &Result<Value, Error>| {
        serde_json::to_vec(&agent_client_protocol::RawJsonRpcMessage::response(
            id.clone(),
            result.clone(),
        ))
        .is_ok_and(|frame| frame.len() <= budget)
    };
    if fits(&result) {
        return result;
    }
    if let Ok(value) = &mut result
        && let Some(output) = value.get("output").and_then(Value::as_str)
    {
        let output = output.to_owned();
        value["output"] = json!("");
        value["truncated"] = json!(true);
        let overhead = serde_json::to_vec(&agent_client_protocol::RawJsonRpcMessage::response(
            id.clone(),
            Ok(value.clone()),
        ))
        .map_or(budget, |frame| frame.len());
        // Any UTF-8 byte needs at most six JSON bytes (e.g. a NUL).
        let keep = budget.saturating_sub(overhead) / 6;
        let mut start = output.len().saturating_sub(keep);
        while !output.is_char_boundary(start) {
            start += 1;
        }
        value["output"] = json!(&output[start..]);
        if fits(&result) {
            return result;
        }
    }
    Err(Error::new(
        -32602,
        "callback response exceeds configured byte budget",
    ))
}

fn io_error(error: std::io::Error) -> Error {
    Error::new(
        -32603,
        format!("host facility I/O failed ({:?})", error.kind()),
    )
}
enum Reply {
    Ready(Value),
    Wait(watch::Receiver<TerminalState>, bool),
}
struct Terminal {
    state: watch::Receiver<TerminalState>,
    kill: CancellationToken,
}
#[derive(Clone, Default)]
struct TerminalState {
    output: String,
    truncated: bool,
    exit: Option<Result<Value, Error>>,
}

// Decoding each stream independently preserves split UTF-8 code points. The bounded
// tail is valid UTF-8 even when its byte limit falls inside a multi-byte character.
fn append_output(
    state: &mut TerminalState,
    pending: &mut Vec<u8>,
    bytes: &[u8],
    eof: bool,
    limit: usize,
) {
    pending.extend_from_slice(bytes);
    loop {
        match std::str::from_utf8(pending) {
            Ok(text) => {
                state.output.push_str(text);
                pending.clear();
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                state
                    .output
                    .push_str(std::str::from_utf8(&pending[..valid]).unwrap_or_default());
                pending.drain(..valid);
                match error.error_len() {
                    Some(n) => {
                        state.output.push('\u{fffd}');
                        pending.drain(..n);
                    }
                    None if eof => {
                        state.output.push('\u{fffd}');
                        pending.clear();
                        break;
                    }
                    None => break,
                }
            }
        }
    }
    if state.output.len() > limit {
        let mut start = state.output.len() - limit;
        while !state.output.is_char_boundary(start) {
            start += 1;
        }
        state.output.drain(..start);
        state.truncated = true;
    }
}

#[cfg(not(windows))]
type TerminalChild = tokio::process::Child;
#[cfg(windows)]
type TerminalChild = super::windows_process::WindowsChild;
async fn run_terminal(
    #[cfg(unix)] mut guard: crate::opensymphony_workspace::ProcessGroupGuard,
    mut child: TerminalChild,
    limit: usize,
    kill: CancellationToken,
    shutdown: CancellationToken,
    tx: watch::Sender<TerminalState>,
    timeout: Duration,
) -> bool {
    #[cfg(not(windows))]
    let pid = child.id();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let mut out = [0u8; 4096];
    let mut err = [0u8; 4096];
    let mut pending_out = Vec::new();
    let mut pending_err = Vec::new();
    let mut out_open = true;
    let mut err_open = true;
    let mut state = TerminalState::default();
    let mut output_failed = false;
    let status = loop {
        tokio::select! {
            biased;
            _ = kill.cancelled() => break None,
            _ = shutdown.cancelled() => break None,
            result = child.wait() => break Some(result),
            read = stdout.read(&mut out), if out_open => {
                output_failed |= read.is_err();
                let n = read.unwrap_or(0); out_open = n != 0;
                append_output(&mut state, &mut pending_out, &out[..n], !out_open, limit); tx.send_replace(state.clone());
            },
            read = stderr.read(&mut err), if err_open => {
                output_failed |= read.is_err();
                let n = read.unwrap_or(0); err_open = n != 0;
                append_output(&mut state, &mut pending_err, &err[..n], !err_open, limit); tx.send_replace(state.clone());
            },
        }
    };
    #[cfg(not(windows))]
    let signalled = terminate_process_tree(&mut child, pid).await.is_ok();
    #[cfg(windows)]
    let signalled = child.start_kill().is_ok();
    let status = match status {
        Some(status) => status,
        None => tokio::time::timeout(timeout, child.wait())
            .await
            .unwrap_or_else(|_| Err(std::io::ErrorKind::TimedOut.into())),
    };
    let drained = tokio::time::timeout(timeout, async {
        while out_open || err_open {
            tokio::select! {
                read = stdout.read(&mut out), if out_open => {
                    let n = read.map_err(io_error)?; out_open = n != 0;
                    append_output(&mut state, &mut pending_out, &out[..n], !out_open, limit);
                },
                read = stderr.read(&mut err), if err_open => {
                    let n = read.map_err(io_error)?; err_open = n != 0;
                    append_output(&mut state, &mut pending_err, &err[..n], !err_open, limit);
                },
            }
        }
        Ok::<_, Error>(())
    })
    .await;
    #[cfg(unix)]
    if signalled && status.is_ok() {
        guard.disarm();
    }
    state.exit = Some(match status {
        Ok(status) if signalled && !output_failed && matches!(drained, Ok(Ok(()))) => {
            #[cfg(unix)]
            let signal = {
                use std::os::unix::process::ExitStatusExt;
                status.signal().map(|signal| signal.to_string())
            };
            #[cfg(not(unix))]
            let signal: Option<String> = None;
            Ok(json!({"exitCode":status.code(),"signal":signal}))
        }
        _ => Err(Error::new(-32603, "terminal teardown failed")),
    });
    let ok = state.exit.as_ref().is_some_and(Result::is_ok);
    tx.send_replace(state);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn terminal_cwd_uses_pinned_directory_after_rename_and_symlink_swap() {
        let root = tempfile::tempdir().expect("root");
        let root = root.path().canonicalize().expect("canonical");
        let cwd = root.join("cwd");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::create_dir(&cwd).expect("cwd");
        std::fs::write(cwd.join("marker"), "original").expect("marker");
        std::fs::write(outside.path().join("marker"), "outside").expect("outside marker");
        let mut command = Command::new("/bin/cat");
        command.arg("marker");
        pin_terminal_cwd(&mut command, &root, &cwd).expect("pin");
        std::fs::rename(&cwd, root.join("moved")).expect("rename");
        std::os::unix::fs::symlink(outside.path(), &cwd).expect("swap");
        let output = command.output().await.expect("spawn");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"original");
        assert!(pin_terminal_cwd(&mut Command::new("/bin/cat"), &root, &cwd).is_err());
    }

    #[test]
    fn facility_responses_fit_encoded_budgets_without_resource_failure() {
        for frame_bytes in [256, 1024] {
            for callback_bytes in [256, 1024] {
                let limits = ClientLimits {
                    frame_bytes,
                    callback_bytes,
                    ..Default::default()
                };
                let id =
                    agent_client_protocol::schema::v1::RequestId::from("callback-id".to_owned());
                for content in ["x".repeat(4096), "\0".repeat(4096), "😀".repeat(1024)] {
                    let file = bounded_response(Ok(json!({"content":content})), &id, &limits);
                    assert!(file.is_err());
                    let terminal = bounded_response(
                        Ok(json!({"output":content,"truncated":false,"exitStatus":{"exitCode":0}})),
                        &id,
                        &limits,
                    );
                    assert_eq!(terminal.as_ref().expect("tail")["truncated"], true);
                    for result in [file, terminal] {
                        let encoded = serde_json::to_string(
                            &agent_client_protocol::RawJsonRpcMessage::response(id.clone(), result),
                        )
                        .expect("encode");
                        assert!(CallbackOutput::default().admit(encoded, &limits));
                    }
                }
            }
        }
    }

    #[test]
    fn active_filesystem_callback_observes_turn_cancellation() {
        let root = tempfile::tempdir().expect("root");
        let root = root.path().canonicalize().expect("canonical");
        let path = root.join("file");
        std::fs::write(&path, "original").expect("file");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let cancellation = CancellationToken::new();
            let (_, mut service) = Services::new(
                root,
                BTreeMap::new(),
                HostServices {
                    write_files: true,
                    ..Default::default()
                },
                ClientLimits::default(),
                Arc::new(Mutex::new(CallbackOutput::default())),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
                CancellationToken::new(),
                cancellation.clone(),
                CancellationToken::new(),
            );
            let (started, ready) = oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel();
            let blocking = tokio::task::spawn_blocking(move || {
                let _ = started.send(());
                blocked.recv().expect("release");
            });
            ready.await.expect("blocking pool occupied");
            let request = service.handle_callback(
                "fs/write_text_file",
                json!({"path":path,"content":"changed"}),
            );
            tokio::pin!(request);
            // Poll the production path until its filesystem lookup is queued
            // behind the occupied blocking pool, then cancel before I/O resumes.
            std::future::poll_fn(|cx| {
                assert!(request.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            cancellation.cancel();
            let result = tokio::time::timeout(Duration::from_millis(100), &mut request).await;
            release.send(()).expect("release");
            blocking.await.expect("blocking task");
            let error = match result {
                Ok(Err(error)) => error,
                _ => panic!("active callback ignored cancellation"),
            };
            assert_eq!(error.message, "session cancelled");
        });
        assert_eq!(std::fs::read_to_string(path).expect("file"), "original");
    }

    #[tokio::test]
    async fn retained_turn_epoch_reaps_prior_terminals_without_cancelling_caller_tokens() {
        let root = tempfile::tempdir().expect("root");
        let shutdown = CancellationToken::new();
        let (sender, mut service) = Services::new(
            root.path().canonicalize().expect("root"),
            BTreeMap::from([("PATH".into(), std::env::var("PATH").expect("path"))]),
            HostServices {
                terminals: true,
                ..Default::default()
            },
            ClientLimits::default(),
            Arc::new(Mutex::new(CallbackOutput::default())),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            CancellationToken::new(),
            CancellationToken::new(),
            shutdown.clone(),
        );
        service
            .handle(
                "terminal/create",
                json!({"command":"python3","args":["-c","import time; time.sleep(60)"]}),
            )
            .await
            .expect("terminal");
        let mut terminal = service
            .terminals
            .values()
            .next()
            .expect("terminal")
            .state
            .clone();
        let task = tokio::spawn(service.run());
        let first = CancellationToken::new();
        sender
            .begin_turn(first.clone(), Duration::from_secs(5))
            .await
            .expect("prior process quiescent");
        assert!(
            terminal
                .wait_for(|state| state.exit.is_some())
                .await
                .expect("exit")
                .exit
                .as_ref()
                .expect("exit")
                .is_ok()
        );
        let second = CancellationToken::new();
        sender
            .begin_turn(second.clone(), Duration::from_secs(5))
            .await
            .expect("new epoch");
        assert!(!first.is_cancelled());
        assert!(!second.is_cancelled());
        shutdown.cancel();
        assert!(task.await.expect("actor"));
    }

    #[tokio::test]
    async fn queued_epoch_handoff_obeys_cancellation_and_deadline() {
        // Retain the receiver without draining it: the epoch is admitted behind
        // outstanding work, but the service cannot acknowledge it yet.
        let (tx, _rx) = mpsc::channel(2);
        let sender = CallbackSender {
            tx,
            permits: Arc::new(Semaphore::new(2)),
            bytes: Arc::new(Semaphore::new(1024)),
        };
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let pending = sender.begin_turn(cancellation, Duration::from_secs(60));
        let (result, ()) = tokio::join!(pending, async {
            tokio::task::yield_now().await;
            cancel.cancel();
        });
        assert!(matches!(result, Err(ClientError::CancelledBeforePrompt)));
        assert!(matches!(
            sender
                .begin_turn(CancellationToken::new(), Duration::from_millis(1))
                .await,
            Err(ClientError::SetupTimeout)
        ));
    }

    #[test]
    fn terminal_tail_preserves_fragmented_utf8_and_byte_bounds() {
        for limit in 0..16 {
            let mut state = TerminalState::default();
            let mut pending = Vec::new();
            for byte in "start😀end".as_bytes() {
                append_output(&mut state, &mut pending, &[*byte], false, limit);
                assert!(state.output.len() <= limit);
            }
            append_output(&mut state, &mut pending, &[], true, limit);
            assert!("start😀end".ends_with(&state.output));
            assert!(pending.is_empty());
            assert_eq!(state.truncated, limit < "start😀end".len());
        }
        let mut state = TerminalState::default();
        let mut pending = Vec::new();
        append_output(&mut state, &mut pending, &[0xff, 0xf0, 0x9f], false, 16);
        append_output(&mut state, &mut pending, &[], true, 16);
        assert_eq!(state.output, "��");
    }
}
