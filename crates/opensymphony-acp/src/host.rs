//! Bounded runtime ownership. Scheduler decisions remain outside this actor.
use super::{
    AcpProfile, ClientError, ClientLimits, LaunchContext, SharedCapture, SourceFrame, TurnReport,
    durable::{Durability, DurabilityError, profile_fingerprint},
};
use crate::opensymphony_workspace::{
    AcpRecovery, AcpSessionIdentity, AcpSessionState, AcpSessionStatus, WorkspaceHandle,
    WorkspaceManager,
};
use agent_client_protocol::{
    Agent, ConnectionTo, UntypedMessage,
    schema::v1::{
        CancelNotification, ContentBlock, InitializeResponse, PromptRequest, SessionId, TextContent,
    },
};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    pub max_sessions: usize,
    pub idle_timeout: Duration,
    pub attachment_ttl: Duration,
    pub max_attachments: usize,
}
impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_sessions: 8,
            idle_timeout: Duration::from_secs(900),
            attachment_ttl: Duration::from_secs(60),
            max_attachments: 16,
        }
    }
}

/// Retire a durable, known-quiescent session after its original host is gone.
/// The owner lock and prior process check must both succeed; this never launches
/// a peer or resolves an ambiguous prompt by assuming its process exited.
pub async fn retire_persisted_session(
    manager: WorkspaceManager,
    workspace: WorkspaceHandle,
) -> Result<(), HostError> {
    let state = manager
        .load_conversation_manifest(&workspace)
        .await
        .map_err(|_| HostError::Persistence)?
        .and_then(|manifest| manifest.acp)
        .ok_or(HostError::NativeManifest)?;
    Durability::retire_persisted(manager, workspace, state.identity).await?;
    Ok(())
}

/// Host callers supply the scheduler's verified workspace and current credential/grant revision.
pub struct SessionLaunch {
    pub manager: WorkspaceManager,
    pub workspace: WorkspaceHandle,
    pub identity: AcpSessionIdentity,
    pub profile: AcpProfile,
    pub context: LaunchContext,
    pub limits: ClientLimits,
    pub require_persistence: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum HostError {
    #[error("ACP owner is unavailable; inspect persisted state before recovery")]
    Unavailable,
    #[error("ACP session identity or connection generation does not match")]
    IdentityMismatch,
    #[error("ACP session has active work or attachment leases")]
    Busy,
    #[error("ACP host or subscriber resource limit reached")]
    ResourceLimit,
    #[error("ACP durable session reservation or checkpoint failed")]
    Persistence,
    #[error("ACP session has a live owner or prior process; attach through that host")]
    AlreadyOwned,
    #[error(
        "ACP prior prompt delivery is uncertain; reconcile execution risk before another attempt"
    )]
    UncertainSubmission,
    #[error("ACP host was lost during process launch; reconcile process ownership before retrying")]
    UncertainLaunch,
    #[error("ACP cannot replace a native conversation manifest")]
    NativeManifest,
    #[error("ACP session cannot restore context with negotiated capabilities")]
    PersistenceUnsupported,
    #[error("ACP session launch or setup failed: {0}")]
    Client(String),
}

impl From<DurabilityError> for HostError {
    fn from(error: DurabilityError) -> Self {
        match error {
            DurabilityError::Owned => Self::AlreadyOwned,
            DurabilityError::Uncertain => Self::UncertainSubmission,
            DurabilityError::ProcessUncertain => Self::UncertainLaunch,
            DurabilityError::NativeManifest => Self::NativeManifest,
            DurabilityError::Binding(_) => Self::IdentityMismatch,
            _ => Self::Persistence,
        }
    }
}

/// Read recorded evidence without claiming that agent context or a live owner was restored.
pub async fn inspect_recorded_session(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
) -> Result<SessionSnapshot, HostError> {
    let manifest = manager
        .load_conversation_manifest(workspace)
        .await
        .map_err(|_| HostError::Persistence)?
        .ok_or(HostError::Unavailable)?;
    if manifest.issue_id != workspace.issue_id() || manifest.identifier != workspace.identifier() {
        return Err(HostError::IdentityMismatch);
    }
    let mut state = manifest.acp.ok_or(HostError::NativeManifest)?;
    if state.identity.workspace_path != workspace.workspace_path() {
        return Err(HostError::IdentityMismatch);
    }
    state.recovery = AcpRecovery::TranscriptOnly;
    Ok(SessionSnapshot {
        state,
        live: false,
        attachment_count: 0,
        retirement_eligible: false,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub state: AcpSessionState,
    pub live: bool,
    pub attachment_count: usize,
    pub retirement_eligible: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    Source {
        generation: u64,
        run_id: String,
        replay: bool,
        frame: SourceFrame,
    },
    State {
        snapshot: Box<SessionSnapshot>,
    },
    Gap {
        generation: u64,
        sequence: u64,
    },
    Ended {
        generation: u64,
        cleanup_ready: bool,
        detail: Option<String>,
    },
}

#[derive(Clone)]
pub(super) struct EventPublisher {
    tx: broadcast::Sender<SessionEvent>,
    binding: Arc<Mutex<(u64, String, bool)>>,
    replay_request: Arc<Mutex<Option<serde_json::Value>>>,
    history: Arc<Mutex<EventHistory>>,
}
impl EventPublisher {
    pub(super) fn end_replay(&self) {
        self.binding.lock().unwrap_or_else(|e| e.into_inner()).2 = false;
    }
    pub(super) fn publish(&self, frame: SourceFrame) {
        if frame.direction == "outgoing" && frame.payload["method"] == "session/load" {
            *self
                .replay_request
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = frame.payload.get("id").cloned();
        }
        let ends_replay = frame.direction == "incoming"
            && frame.payload.get("method").is_none()
            && frame.payload.get("id").is_some_and(|id| {
                self.replay_request
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_ref()
                    == Some(id)
            });
        let (generation, run_id, replay) = self
            .binding
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let event = SessionEvent::Source {
            generation,
            run_id,
            replay,
            frame,
        };
        let admitted = self
            .history
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(event.clone());
        let event = if admitted {
            event
        } else {
            let SessionEvent::Source { frame, .. } = event else {
                unreachable!()
            };
            SessionEvent::Gap {
                generation,
                sequence: frame.sequence,
            }
        };
        let _ = self.tx.send(event);
        if ends_replay {
            self.end_replay();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceHistory {
    pub events: Vec<SessionEvent>,
    pub truncated: bool,
}
struct EventHistory {
    events: VecDeque<(SessionEvent, usize)>,
    bytes: usize,
    max_bytes: usize,
    max_frames: usize,
    truncated: bool,
}
impl EventHistory {
    fn retain(&mut self, event: SessionEvent) -> bool {
        let bytes = serde_json::to_vec(&event).map_or(usize::MAX, |v| v.len());
        if bytes > self.max_bytes {
            self.truncated = true;
            return false;
        }
        while self.events.len() >= self.max_frames
            || self.bytes.saturating_add(bytes) > self.max_bytes
        {
            let Some((_, removed)) = self.events.pop_front() else {
                break;
            };
            self.bytes -= removed;
            self.truncated = true;
        }
        self.bytes += bytes;
        self.events.push_back((event, bytes));
        true
    }
}

#[derive(Clone)]
pub struct SessionHandle {
    pub owner_id: String,
    pub generation: u64,
    commands: mpsc::Sender<Command>,
    events: broadcast::Sender<SessionEvent>,
    history: Arc<Mutex<EventHistory>>,
    persistence_supported: bool,
}
impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}
impl SessionHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }
    /// Subscribe first, then read history and deduplicate source sequence numbers.
    pub fn source_history(&self) -> SourceHistory {
        let history = self.history.lock().unwrap_or_else(|e| e.into_inner());
        SourceHistory {
            events: history
                .events
                .iter()
                .map(|(event, _)| event.clone())
                .collect(),
            truncated: history.truncated,
        }
    }
    pub async fn inspect(&self) -> Result<SessionSnapshot, HostError> {
        match self.control(SessionControl::Inspect).await? {
            ControlResult::Snapshot(s) => Ok(*s),
            _ => Err(HostError::Unavailable),
        }
    }
    /// Observation pins the live process but grants no prompt/write authority.
    pub async fn control(&self, action: SessionControl) -> Result<ControlResult, HostError> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .try_send(Command::Control {
                generation: self.generation,
                action,
                reply,
            })
            .map_err(channel_error)?;
        receive.await.map_err(|_| HostError::Unavailable)?
    }
    pub async fn prompt(
        &self,
        run_id: String,
        attempt: u32,
        prompt: String,
        cancellation: CancellationToken,
    ) -> Result<TurnReport, HostError> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .try_send(Command::Prompt {
                generation: self.generation,
                run_id,
                attempt,
                prompt,
                cancellation,
                reply,
            })
            .map_err(channel_error)?;
        receive.await.map_err(|_| HostError::Unavailable)?
    }
}
fn channel_error<T>(error: mpsc::error::TrySendError<T>) -> HostError {
    match error {
        mpsc::error::TrySendError::Closed(_) => HostError::Unavailable,
        mpsc::error::TrySendError::Full(_) => HostError::ResourceLimit,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum SessionControl {
    Inspect,
    Attach,
    Renew { lease_id: String },
    Release { lease_id: String },
    Retire,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ControlResult {
    Snapshot(Box<SessionSnapshot>),
    Lease { lease_id: String, ttl_ms: u64 },
    Released,
}

enum Command {
    Control {
        generation: u64,
        action: SessionControl,
        reply: oneshot::Sender<Result<ControlResult, HostError>>,
    },
    Prompt {
        generation: u64,
        run_id: String,
        attempt: u32,
        prompt: String,
        cancellation: CancellationToken,
        reply: oneshot::Sender<Result<TurnReport, HostError>>,
    },
}
enum HostCommand {
    Open {
        launch: Box<SessionLaunch>,
        reply: oneshot::Sender<Result<SessionHandle, HostError>>,
    },
    Lookup {
        owner_id: String,
        generation: u64,
        reply: oneshot::Sender<Result<SessionHandle, HostError>>,
    },
}
type LaunchCompletion = (
    String,
    oneshot::Sender<Result<SessionHandle, HostError>>,
    Result<(AcpSessionIdentity, SessionHandle), HostError>,
);

#[derive(Clone, Debug)]
pub struct SessionHost {
    commands: mpsc::Sender<HostCommand>,
}
impl SessionHost {
    pub fn new(policy: RetentionPolicy) -> Result<Self, HostError> {
        if !(1..=128).contains(&policy.max_sessions)
            || !(1..=128).contains(&policy.max_attachments)
            || policy.idle_timeout.is_zero()
            || policy.attachment_ttl.is_zero()
            || policy.idle_timeout > Duration::from_secs(86400)
            || policy.attachment_ttl > Duration::from_secs(3600)
        {
            return Err(HostError::ResourceLimit);
        }
        let (commands, mut receive) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut sessions: BTreeMap<String, (AcpSessionIdentity, SessionHandle)> =
                BTreeMap::new();
            let mut starting = FuturesUnordered::new();
            let mut pending = std::collections::BTreeSet::new();
            loop {
                let command = tokio::select! {
                    completed = starting.next(), if !starting.is_empty() => {
                        let (key, reply, result): LaunchCompletion = completed.expect("pending launch");
                        pending.remove(&key);
                        if let Ok((identity, handle)) = &result { sessions.insert(key, (identity.clone(), handle.clone())); }
                        let _ = reply.send(result.map(|(_, handle)| handle));
                        continue;
                    }
                    command = receive.recv() => match command { Some(command) => command, None => break },
                };
                sessions.retain(|_, (_, handle)| !handle.commands.is_closed());
                match command {
                    HostCommand::Lookup {
                        owner_id,
                        generation,
                        reply,
                    } => {
                        let result = sessions
                            .values()
                            .find(|(_, h)| h.owner_id == owner_id)
                            .ok_or(HostError::Unavailable)
                            .and_then(|(_, h)| {
                                if h.generation == generation {
                                    Ok(h.clone())
                                } else {
                                    Err(HostError::IdentityMismatch)
                                }
                            });
                        let _ = reply.send(result);
                    }
                    HostCommand::Open { mut launch, reply } => {
                        let key = launch.workspace.issue_id().to_owned();
                        let fingerprint = profile_fingerprint(
                            &launch.profile,
                            &launch.context.services,
                            &launch.limits,
                        );
                        let Ok(fingerprint) = fingerprint else {
                            let _ = reply.send(Err(HostError::Persistence));
                            continue;
                        };
                        launch.identity.profile_fingerprint = fingerprint;
                        if let Err(error) =
                            super::validate_launch(&launch.profile, &launch.context, &launch.limits)
                        {
                            let _ = reply.send(Err(HostError::Client(error.to_string())));
                            continue;
                        }
                        if launch.context.issue_workspace != launch.identity.workspace_path
                            || Some(launch.context.workspace_key.as_str())
                                != launch
                                    .workspace
                                    .workspace_path()
                                    .file_name()
                                    .and_then(|name| name.to_str())
                        {
                            let _ = reply.send(Err(HostError::IdentityMismatch));
                            continue;
                        }
                        if pending.contains(&key) {
                            let _ = reply.send(Err(HostError::Busy));
                            continue;
                        }
                        if let Some((identity, handle)) = sessions.get(&key) {
                            if launch.require_persistence && !handle.persistence_supported {
                                let _ = reply.send(Err(HostError::PersistenceUnsupported));
                                continue;
                            }
                            let mut expected = launch.identity.clone();
                            expected.generation = identity.generation;
                            expected.run_id.clone_from(&identity.run_id);
                            expected.attempt = identity.attempt;
                            let result = if expected == *identity
                                && (launch.identity.generation == 0
                                    || launch.identity.generation == handle.generation)
                            {
                                Ok(handle.clone())
                            } else {
                                Err(HostError::IdentityMismatch)
                            };
                            let _ = reply.send(result);
                            continue;
                        }
                        if sessions.len() + pending.len() >= policy.max_sessions {
                            let _ = reply.send(Err(HostError::ResourceLimit));
                            continue;
                        }
                        pending.insert(key.clone());
                        let policy = policy.clone();
                        starting.push(async move { (key, reply, start(*launch, policy).await) });
                    }
                }
            }
        });
        Ok(Self { commands })
    }
    pub async fn open(&self, launch: SessionLaunch) -> Result<SessionHandle, HostError> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .try_send(HostCommand::Open {
                launch: Box::new(launch),
                reply,
            })
            .map_err(channel_error)?;
        receive.await.map_err(|_| HostError::Unavailable)?
    }
    pub async fn lookup(
        &self,
        owner_id: String,
        generation: u64,
    ) -> Result<SessionHandle, HostError> {
        let (reply, receive) = oneshot::channel();
        self.commands
            .try_send(HostCommand::Lookup {
                owner_id,
                generation,
                reply,
            })
            .map_err(channel_error)?;
        receive.await.map_err(|_| HostError::Unavailable)?
    }
}

async fn start(
    launch: SessionLaunch,
    policy: RetentionPolicy,
) -> Result<(AcpSessionIdentity, SessionHandle), HostError> {
    super::validate_launch(&launch.profile, &launch.context, &launch.limits)
        .map_err(|e| HostError::Client(e.to_string()))?;
    if launch.context.issue_workspace != launch.workspace.workspace_path()
        || Some(launch.context.workspace_key.as_str())
            != launch
                .workspace
                .workspace_path()
                .file_name()
                .and_then(|name| name.to_str())
        || launch.identity.workspace_path != launch.workspace.workspace_path()
    {
        return Err(HostError::IdentityMismatch);
    }
    let durable = Durability::open(launch.manager, launch.workspace, launch.identity)
        .await
        .map_err(HostError::from)?;
    let identity = durable.state().identity.clone();
    let (commands, receive) = mpsc::channel(16);
    let (events, _) = broadcast::channel(
        launch
            .limits
            .queued_frames
            .min((64 * 1024 * 1024 / launch.limits.queued_bytes).max(1)),
    );
    let history = Arc::new(Mutex::new(EventHistory {
        events: VecDeque::new(),
        bytes: 0,
        max_bytes: launch.limits.queued_bytes,
        max_frames: launch.limits.queued_frames,
        truncated: false,
    }));
    let mut handle = SessionHandle {
        owner_id: durable.state().owner_id.clone(),
        generation: identity.generation,
        commands,
        events: events.clone(),
        history: history.clone(),
        persistence_supported: false,
    };
    let (ready, ready_receive) = oneshot::channel();
    let publisher = EventPublisher {
        tx: events,
        binding: Arc::new(Mutex::new((
            identity.generation,
            identity.run_id.clone(),
            false,
        ))),
        replay_request: Arc::new(Mutex::new(None)),
        history,
    };
    let mut driver = SessionDriver {
        durable,
        receive,
        publisher,
        policy,
        require_persistence: launch.require_persistence,
        ready: Some(ready),
        attachments: BTreeMap::new(),
        idle_since: tokio::time::Instant::now(),
        retire_reply: None,
        reset_reason: None,
    };
    tokio::spawn(async move {
        let result = super::run_connection(
            &launch.profile,
            launch.context,
            String::new(),
            CancellationToken::new(),
            None,
            launch.limits,
            Some(&mut driver),
        )
        .await;
        let error = match &result {
            Ok(result) => result.outcome.as_ref().err().map(ToString::to_string),
            Err(error) => Some(error.to_string()),
        };
        // A failed connection cannot establish remote quiescence. A submitted marker
        // survives even if the outcome checkpoint fails, and prevents another prompt.
        let checkpointed = if result.as_ref().is_ok_and(|r| {
            r.process_reaped
                && (r.process_tree_signal_error.is_none() || driver.durable.process_is_absent())
        }) || matches!(
            result,
            Err(ClientError::InvalidConfiguration(_)
                | ClientError::InvalidWorkspace
                | ClientError::Launch(_)
                | ClientError::CancelledBeforePrompt)
        ) {
            driver.durable.stopped().await.is_ok()
        } else {
            false
        };
        if driver.durable.state().status == AcpSessionStatus::Submitted {
            let _ = driver.durable.uncertain().await;
        }
        if let Some(ready) = driver.ready.take() {
            let _ = ready.send(Err(HostError::Client(
                error
                    .clone()
                    .unwrap_or_else(|| "owner exited during setup".into()),
            )));
        }
        let cleanup_ready = error.is_none()
            && checkpointed
            && matches!(
                driver.durable.state().status,
                AcpSessionStatus::Ready | AcpSessionStatus::Finished
            );
        let mut ended_snapshot = driver.snapshot(false);
        ended_snapshot.live = false;
        ended_snapshot.retirement_eligible = false;
        ended_snapshot.state.recovery = AcpRecovery::TranscriptOnly;
        let _ = driver.publisher.tx.send(SessionEvent::State {
            snapshot: Box::new(ended_snapshot),
        });
        let reply = driver.retire_reply.take();
        let publisher = driver.publisher.clone();
        let generation = driver.durable.state().identity.generation;
        drop(driver); // Release the owner lock and close commands before acknowledging retirement.
        let _ = publisher.tx.send(SessionEvent::Ended {
            generation,
            cleanup_ready,
            detail: error,
        });
        if let Some(reply) = reply {
            let _ = reply.send(if cleanup_ready {
                Ok(ControlResult::Released)
            } else {
                Err(HostError::Unavailable)
            });
        }
    });
    handle.persistence_supported = ready_receive.await.map_err(|_| HostError::Unavailable)??;
    Ok((identity, handle))
}

type PromptFuture = Pin<Box<dyn Future<Output = Result<TurnReport, ClientError>> + Send>>;
struct ActivePrompt {
    result: PromptFuture,
    reply: oneshot::Sender<Result<TurnReport, HostError>>,
    cancellation: CancellationToken,
}

struct PreparingPrompt {
    result: Pin<Box<dyn Future<Output = Result<(), ClientError>> + Send>>,
    run_id: String,
    attempt: u32,
    prompt: String,
    cancellation: CancellationToken,
    forced: CancellationToken,
    callback_epoch: CancellationToken,
    reply: oneshot::Sender<Result<TurnReport, HostError>>,
}

pub(super) struct SessionDriver {
    durable: Durability,
    receive: mpsc::Receiver<Command>,
    pub(super) publisher: EventPublisher,
    policy: RetentionPolicy,
    require_persistence: bool,
    ready: Option<oneshot::Sender<Result<bool, HostError>>>,
    attachments: BTreeMap<String, tokio::time::Instant>,
    idle_since: tokio::time::Instant,
    retire_reply: Option<oneshot::Sender<Result<ControlResult, HostError>>>,
    reset_reason: Option<String>,
}
impl SessionDriver {
    pub(super) async fn launched(&mut self, pid: Option<u32>) -> Result<(), ClientError> {
        self.durable
            .launched(pid.ok_or(ClientError::Teardown)?)
            .await
            .map_err(|_| ClientError::Setup("process ownership checkpoint failed".into()))
    }
    pub(super) fn reset_missing_session(&mut self, error: &agent_client_protocol::Error) -> bool {
        if self.require_persistence
            || error.code != agent_client_protocol::schema::v1::ErrorCode::ResourceNotFound
        {
            return false;
        }
        self.durable.state_mut().recovery = AcpRecovery::Fresh;
        self.reset_reason = Some(
            "agent no longer has persisted context; fresh full-context prompt required".into(),
        );
        self.publisher.end_replay();
        true
    }
    pub(super) fn restoration(
        &mut self,
        initialization: &InitializeResponse,
    ) -> Result<Option<(String, bool)>, HostError> {
        let capabilities = &initialization.agent_capabilities;
        let has_resume = capabilities.session_capabilities.resume.is_some();
        let has_load = capabilities.load_session;
        if self.require_persistence && !has_resume && !has_load {
            return Err(HostError::PersistenceUnsupported);
        }
        let Some(id) = self.durable.state().session_id.clone() else {
            return Ok(None);
        };
        let recovery = if has_resume {
            AcpRecovery::RestoredResume
        } else if has_load {
            AcpRecovery::RestoredLoad
        } else {
            AcpRecovery::Fresh
        };
        self.durable.state_mut().recovery = recovery;
        let replay = recovery == AcpRecovery::RestoredLoad;
        self.publisher
            .binding
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .2 = replay;
        if has_resume || has_load {
            Ok(Some((id, replay)))
        } else {
            Ok(None)
        }
    }
    fn snapshot(&self, active: bool) -> SessionSnapshot {
        let mut state = self.durable.state().clone();
        if state.status == AcpSessionStatus::Finished {
            state.recovery = AcpRecovery::LiveAttach;
        }
        SessionSnapshot {
            state,
            live: true,
            attachment_count: self.attachments.len(),
            retirement_eligible: !active
                && self.attachments.is_empty()
                && matches!(
                    self.durable.state().status,
                    AcpSessionStatus::Ready | AcpSessionStatus::Finished
                ),
        }
    }
    fn publish(&self, active: bool) {
        let _ = self.publisher.tx.send(SessionEvent::State {
            snapshot: Box::new(self.snapshot(active)),
        });
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn drive(
        &mut self,
        connection: ConnectionTo<Agent>,
        initialization: InitializeResponse,
        session_id: SessionId,
        capture: &SharedCapture,
        limits: &ClientLimits,
        services: &super::services::CallbackSender,
        configuration: &Arc<Mutex<super::SessionConfiguration>>,
        profile: &AcpProfile,
    ) -> Result<TurnReport, ClientError> {
        let mut metadata = serde_json::to_value(&initialization)
            .map_err(|_| ClientError::Setup("invalid negotiation metadata".into()))?;
        {
            let capture = capture.lock().unwrap_or_else(|e| e.into_inner());
            if session_id.0.len() > 8192
                || session_id.0.is_empty()
                || capture.secrets.contains_secret(&session_id.0)
            {
                return Err(ClientError::Setup(
                    "invalid or credential-bearing session identity".into(),
                ));
            }
            capture.redact(&mut metadata, false);
        }
        let recovery = self.durable.state().recovery;
        let reset_reason = if self.reset_reason.is_some() {
            self.reset_reason.take()
        } else if self.durable.state().session_id.is_some() && recovery == AcpRecovery::Fresh {
            Some("agent context unavailable: persistence unsupported; fresh full-context prompt required".into())
        } else {
            None
        };
        let model_selection = configuration
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .supports_model_selection();
        self.durable
            .ready(
                session_id.0.to_string(),
                metadata,
                model_selection,
                recovery,
                reset_reason,
            )
            .await
            .map_err(|_| ClientError::Setup("durable session checkpoint failed".into()))?;
        self.publisher
            .binding
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .2 = false;
        self.idle_since = tokio::time::Instant::now();
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(Ok(initialization.agent_capabilities.load_session
                || initialization
                    .agent_capabilities
                    .session_capabilities
                    .resume
                    .is_some()));
        }
        self.publish(false);
        let mut active: Option<ActivePrompt> = None;
        let mut preparing: Option<PreparingPrompt> = None;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        let mut commands_closed = false;
        loop {
            tokio::select! {
                result = async { preparing.as_mut().expect("guarded preparation").result.as_mut().await }, if preparing.is_some() => {
                    let pending = preparing.take().expect("preparing prompt");
                    let result = result.and_then(|()| {
                        if pending.cancellation.is_cancelled() || pending.forced.is_cancelled() { Err(ClientError::CancelledBeforePrompt) } else { Ok(()) }
                    });
                    if let Err(error) = result {
                        let _ = pending.reply.send(Err(HostError::Client(error.to_string())));
                        return Err(error);
                    }
                    let model_selection = configuration
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .supports_model_selection();
                    if self.durable.set_model_selection(model_selection).await.is_err() {
                        let _ = pending.reply.send(Err(HostError::Persistence));
                        return Err(ClientError::Setup("model capability checkpoint failed".into()));
                    }
                    if self.durable.submitted(pending.run_id.clone(), pending.attempt).await.is_err() {
                        let _ = pending.reply.send(Err(HostError::Persistence));
                        return Err(ClientError::Setup("submission checkpoint failed".into()));
                    }
                    if pending.cancellation.is_cancelled() || pending.forced.is_cancelled() {
                        pending.callback_epoch.cancel();
                        if self.durable.finished("cancelled_before_prompt".into()).await.is_err() {
                            let _ = pending.reply.send(Err(HostError::Persistence));
                            return Err(ClientError::Setup("cancellation checkpoint failed".into()));
                        }
                        let error = ClientError::CancelledBeforePrompt;
                        let _ = pending.reply.send(Err(HostError::Client(error.to_string())));
                        return Err(error);
                    }
                    active = Some(ActivePrompt {
                        result: Box::pin(prompt_rpc(connection.clone(), initialization.clone(), session_id.clone(), pending.prompt, pending.cancellation, pending.forced.clone(), capture.clone(), limits.clone(), configuration.clone(), pending.callback_epoch, services.clone())),
                        reply: pending.reply, cancellation: pending.forced,
                    });
                    self.publish(true);
                }
                result = async { active.as_mut().expect("guarded active prompt").result.as_mut().await }, if active.is_some() => {
                    let pending = active.take().expect("active prompt");
                    let report = match result {
                        Ok(report) => report,
                        Err(ClientError::CancelledBeforePrompt) => {
                            if self.durable.finished("cancelled_before_prompt".into()).await.is_err() {
                                let _ = pending.reply.send(Err(HostError::Persistence));
                                return Err(ClientError::Setup("cancellation checkpoint failed".into()));
                            }
                            let error = ClientError::CancelledBeforePrompt;
                            let _ = pending.reply.send(Err(HostError::Client(error.to_string())));
                            return Err(error);
                        }
                        Err(error) => { let _ = self.durable.uncertain().await; let _ = pending.reply.send(Err(HostError::Client(error.to_string()))); return Err(error); }
                    };
                    if self.durable.finished(report.stop_reason.clone()).await.is_err() {
                        let _ = pending.reply.send(Err(HostError::Persistence));
                        return Err(ClientError::Setup("terminal checkpoint failed; submission remains fenced".into()));
                    }
                    let _ = pending.reply.send(Ok(report));
                    self.idle_since = tokio::time::Instant::now();
                    self.publish(false);
                }
                command = self.receive.recv(), if !commands_closed => {
                    let Some(command) = command else {
                        commands_closed = true;
                        if let Some(active) = &active { active.cancellation.cancel(); }
                        if let Some(preparing) = &preparing { preparing.forced.cancel(); }
                        continue;
                    };
                    match command {
                        Command::Prompt { generation, run_id, attempt, prompt, cancellation, reply } => {
                            if generation != self.durable.state().identity.generation { let _ = reply.send(Err(HostError::IdentityMismatch)); continue; }
                            if active.is_some() || preparing.is_some() { let _ = reply.send(Err(HostError::Busy)); continue; }
                            if prompt.len() > limits.frame_bytes / 6 || run_id.is_empty() || run_id.len() > 1024 { let _ = reply.send(Err(HostError::ResourceLimit)); continue; }
                            if cancellation.is_cancelled() { let _ = reply.send(Err(HostError::Client(ClientError::CancelledBeforePrompt.to_string()))); continue; }
                            let forced = cancellation.child_token();
                            let callback_epoch = forced.child_token();
                            let sender = services.clone();
                            let epoch = callback_epoch.clone();
                            let timeout = limits.setup_timeout;
                            let connection = connection.clone();
                            let profile = profile.clone();
                            let session_id = session_id.clone();
                            let configuration = configuration.clone();
                            let capture = capture.clone();
                            // Preparation frames belong to this accepted run even if setup
                            // fails before any durable prompt submission occurs.
                            self.publisher.binding.lock().unwrap_or_else(|e| e.into_inner()).1 = run_id.clone();
                            preparing = Some(PreparingPrompt {
                                result: Box::pin(async move {
                                    tokio::select! {
                                        biased;
                                        _ = epoch.cancelled() => Err(ClientError::CancelledBeforePrompt),
                                        result = tokio::time::timeout(timeout, async {
                                            sender.begin_turn(epoch.clone(), timeout).await?;
                                            super::session_config::apply(&connection, &profile, session_id.0.as_ref(), &configuration, &capture).await
                                        }) => result.map_err(|_| ClientError::SetupTimeout)?,
                                    }
                                }),
                                run_id, attempt, prompt, cancellation, forced, callback_epoch, reply,
                            });
                            self.publish(true);
                        }
                        Command::Control { generation, action, reply } => {
                            if generation != self.durable.state().identity.generation { let _ = reply.send(Err(HostError::IdentityMismatch)); continue; }
                            self.expire_leases();
                            let result = match action {
                                SessionControl::Inspect => Ok(ControlResult::Snapshot(Box::new(self.snapshot(active.is_some() || preparing.is_some())))),
                                SessionControl::Attach => {
                                    if self.attachments.len() >= self.policy.max_attachments { Err(HostError::ResourceLimit) } else {
                                        let lease_id = uuid::Uuid::new_v4().to_string();
                                        self.attachments.insert(lease_id.clone(), tokio::time::Instant::now() + self.policy.attachment_ttl);
                                        Ok(ControlResult::Lease { lease_id, ttl_ms: self.policy.attachment_ttl.as_millis() as u64 })
                                    }
                                }
                                SessionControl::Renew { lease_id } => match self.attachments.get_mut(&lease_id) {
                                    Some(expiry) => { *expiry = tokio::time::Instant::now() + self.policy.attachment_ttl; Ok(ControlResult::Lease { lease_id, ttl_ms: self.policy.attachment_ttl.as_millis() as u64 }) }
                                    None => Err(HostError::IdentityMismatch),
                                },
                                SessionControl::Release { lease_id } => if self.attachments.remove(&lease_id).is_some() { self.idle_since = tokio::time::Instant::now(); Ok(ControlResult::Released) } else { Err(HostError::IdentityMismatch) },
                                SessionControl::Retire => {
                                    if active.is_some() || preparing.is_some() || !self.attachments.is_empty() { Err(HostError::Busy) } else { self.retire_reply = Some(reply); break; }
                                }
                            };
                            let _ = reply.send(result);
                        }
                    }
                }
                _ = tick.tick() => {
                    self.expire_leases();
                    if active.is_none() && preparing.is_none() && self.attachments.is_empty() && (commands_closed || self.idle_since.elapsed() >= self.policy.idle_timeout) { break; }
                }
            }
        }
        Ok(TurnReport {
            stop_reason: "session_retired".into(),
            cancellation_requested: false,
            cancellation_acknowledged: false,
            session_id: session_id.0.to_string(),
            initialization,
            configuration: configuration
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        })
    }
    fn expire_leases(&mut self) {
        self.attachments
            .retain(|_, expiry| *expiry > tokio::time::Instant::now());
    }
}

#[allow(clippy::too_many_arguments)]
async fn prompt_rpc(
    connection: ConnectionTo<Agent>,
    initialization: InitializeResponse,
    session_id: SessionId,
    prompt: String,
    cancellation: CancellationToken,
    forced: CancellationToken,
    capture: SharedCapture,
    limits: ClientLimits,
    configuration: Arc<Mutex<super::SessionConfiguration>>,
    callback_epoch: CancellationToken,
    services: super::services::CallbackSender,
) -> Result<TurnReport, ClientError> {
    // The submitted marker may finish persisting after cancellation. This
    // check runs again when the active future is first polled, immediately
    // before transport submission.
    if cancellation.is_cancelled() || forced.is_cancelled() {
        callback_epoch.cancel();
        return Err(ClientError::CancelledBeforePrompt);
    }
    let request = UntypedMessage::new(
        "session/prompt",
        PromptRequest::new(
            session_id.clone(),
            vec![ContentBlock::Text(TextContent::new(prompt))],
        ),
    )
    .map_err(|_| ClientError::Protocol { submitted: false })?;
    let (response_tx, response_rx) = oneshot::channel();
    connection
        .send_request(request)
        .on_receiving_result(async move |result| {
            // Revoke callback authority in ordered response dispatch, before an
            // adjacent late callback can run. This does not cancel the prompt token.
            callback_epoch.cancel();
            let _ = response_tx.send(result);
            Ok(())
        })
        .map_err(|_| ClientError::Protocol { submitted: true })?;
    let response = async {
        response_rx
            .await
            .map_err(|_| agent_client_protocol::Error::internal_error())?
    };
    tokio::pin!(response);
    let mut cancellation_requested = false;
    let result = tokio::select! {
        result = &mut response => result,
        _ = async { tokio::select! { _ = cancellation.cancelled() => {}, _ = forced.cancelled() => {} } } => {
            cancellation_requested = true;
            forced.cancel();
            connection.send_notification(CancelNotification::new(session_id.clone())).map_err(|_| ClientError::Protocol { submitted: true })?;
            tokio::time::timeout(limits.cancel_timeout, &mut response).await.map_err(|_| ClientError::CancelTimeout)?
        },
        _ = super::wait_prompt_timeout(limits.prompt_timeout) => return Err(ClientError::PromptTimeout),
    }.map_err(|error| super::rpc_failure("session/prompt", &error, &capture, true).unwrap_or(ClientError::Protocol { submitted: true }))?;
    let stop_reason = result
        .get("stopReason")
        .and_then(serde_json::Value::as_str)
        .filter(|reason| reason.len() <= 1024)
        .ok_or(ClientError::Protocol { submitted: true })?
        .to_owned();
    // Unknown reasons do not establish the tested stop contract.
    if ![
        "end_turn",
        "max_tokens",
        "max_turn_requests",
        "refusal",
        "cancelled",
    ]
    .contains(&stop_reason.as_str())
    {
        return Err(ClientError::Protocol { submitted: true });
    }
    services.end_turn(limits.setup_timeout).await?;
    Ok(TurnReport {
        cancellation_acknowledged: cancellation_requested && stop_reason == "cancelled",
        cancellation_requested,
        stop_reason,
        session_id: session_id.0.to_string(),
        initialization,
        configuration: configuration
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    })
}
