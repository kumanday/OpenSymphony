//! Production worker wiring. ACP wire types and protocol ownership stay in opensymphony_acp.
use super::*;
use crate::opensymphony_acp::{
    AcpOperatorEvent, ClientLimits, HostServices, LaunchContext, RetentionPolicy,
    RuntimeProjection, SessionControl, SessionEvent, SessionHandle, SessionHost, SessionLaunch,
};
use crate::opensymphony_workspace::{AcpProcessState, AcpSessionIdentity, AcpSessionStatus};
use tokio_util::sync::CancellationToken;

pub(super) const KIND: &str = "acp";
#[derive(Clone)]
pub(super) struct ActiveSession {
    pub handle: SessionHandle,
    pub cancellation: CancellationToken,
    pub prompt_finished: CancellationToken,
    pub workspace: WorkspaceHandle,
    pub issue_id: IssueId,
    pub issue_identifier: IssueIdentifier,
    pub run_id: String,
    pub operator_responses: mpsc::Sender<OperatorResponseCommand>,
}
pub(super) struct OperatorResponseCommand {
    pub request_id: String,
    pub answer: crate::opensymphony_gateway_schema::approval::OperatorAnswer,
    pub acknowledgement: oneshot::Sender<bool>,
    pub delivery: crate::opensymphony_acp::AcpOperatorDeliveryFence,
}
pub(super) type ActiveSessions = Arc<Mutex<HashMap<String, ActiveSession>>>;

pub(super) enum StopObservation {
    Pending,
    Stopped(Option<String>),
    Uncertain,
}

/// A pre-submission cancellation can close the owner after it has persisted a
/// stopped process. In that case the live control channel is gone, so use only
/// the durable record for this exact owner and generation as stop evidence.
pub(super) async fn observe_stop(
    session: &ActiveSession,
    manager: &WorkspaceManager,
) -> Result<StopObservation, CliWorkerError> {
    match session.handle.inspect().await {
        Ok(snapshot) => {
            if snapshot.state.status == AcpSessionStatus::Uncertain {
                return Ok(StopObservation::Uncertain);
            }
            if (snapshot.state.status == AcpSessionStatus::Finished
                && snapshot.state.identity.run_id == session.run_id)
                || (session.prompt_finished.is_cancelled()
                    && snapshot.retirement_eligible
                    && matches!(
                        snapshot.state.status,
                        AcpSessionStatus::Ready | AcpSessionStatus::Finished
                    ))
            {
                return Ok(StopObservation::Stopped(snapshot.state.stop_reason));
            }
            Ok(StopObservation::Pending)
        }
        Err(crate::opensymphony_acp::HostError::Unavailable) => {
            let manifest = manager
                .load_conversation_manifest(&session.workspace)
                .await
                .map_err(|error| CliWorkerError::InterruptFailed(error.to_string()))?
                .ok_or_else(|| {
                    CliWorkerError::InterruptFailed("ACP durable stop evidence is missing".into())
                })?;
            let state = manifest.acp.ok_or_else(|| {
                CliWorkerError::InterruptFailed("ACP durable stop evidence is missing".into())
            })?;
            if manifest.issue_id != session.workspace.issue_id()
                || state.owner_id != session.handle.owner_id
                || state.identity.generation != session.handle.generation
                || (matches!(
                    state.status,
                    AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain
                ) && state.identity.run_id != session.run_id)
                || state.identity.workspace_path != session.workspace.workspace_path()
            {
                return Err(CliWorkerError::InterruptFailed(
                    "ACP durable stop identity mismatch".into(),
                ));
            }
            if state.process != AcpProcessState::Stopped {
                return Ok(StopObservation::Pending);
            }
            match state.status {
                AcpSessionStatus::Ready | AcpSessionStatus::Finished => {
                    Ok(StopObservation::Stopped(state.stop_reason))
                }
                AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain => {
                    Ok(StopObservation::Uncertain)
                }
            }
        }
        Err(error) => Err(CliWorkerError::InterruptFailed(error.to_string())),
    }
}

pub(super) fn new_host() -> SessionHost {
    SessionHost::new(RetentionPolicy {
        max_sessions: 128,
        ..RetentionPolicy::default()
    })
    .expect("valid production ACP retention policy")
}

pub(super) fn production_client_limits() -> ClientLimits {
    // Scheduler stall detection and abort own production turn liveness.
    ClientLimits {
        prompt_timeout: Duration::ZERO,
        ..ClientLimits::default()
    }
}

/// Normalize shared manifest fields for existing scheduler recovery. Never persist this view:
/// the ACP owner is the only writer of its durable conversation record.
pub(super) fn conversation_view(raw: &str) -> Result<IssueConversationManifest, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(raw)?;
    if value.get("acp").is_some_and(|state| !state.is_null()) {
        let created = value["created_at"].clone();
        value["updated_at"] = created.clone();
        if value["last_attached_at"].is_null() {
            value["last_attached_at"] = created;
        }
        value["transport_target"] = serde_json::json!(KIND);
        value["server_base_url"] = serde_json::Value::Null;
        value["last_execution_status"] = value["acp"]["status"].clone();
        value["workflow_prompt_seeded"] = serde_json::json!(
            value
                .pointer("/acp/workflow_prompt_seeded")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or_else(|| value
                    .pointer("/acp/status")
                    .and_then(serde_json::Value::as_str)
                    != Some("ready")
                    && value
                        .pointer("/acp/stop_reason")
                        .and_then(serde_json::Value::as_str)
                        != Some("cancelled_before_prompt"))
        );
        if value["conversation_id"].as_str() == Some("") {
            // A launch reservation can precede the peer session ID. This identifies the
            // owner for scheduler fencing, and is never submitted as an ACP session ID.
            value["conversation_id"] = value["acp"]["owner_id"].clone();
        }
    }
    serde_json::from_value(value)
}

/// Bind the current run to its ACP session without importing an older owner's
/// hierarchy or checkout snapshot from a retained conversation manifest.
pub(super) fn bind_current_run_envelopes(
    run: &mut RunManifest,
    conversation: &crate::opensymphony_workspace::ConversationManifest,
) {
    let Some(state) = conversation.acp.as_ref() else {
        return;
    };
    if state.identity.run_id != run.run_id || state.identity.attempt != run.attempt {
        return;
    }
    if let Some(envelope) = run.runtime_envelope.as_mut() {
        envelope.acp_session = Some(state.identity.clone());
        envelope.conversation_binding = state.session_id.clone();
    }
    if let Some(envelope) = run.parent_runtime_envelope.as_mut() {
        envelope.conversation_binding = state.session_id.clone();
    }
}

pub(super) async fn retire(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    host: Option<&SessionHost>,
) -> Result<(), String> {
    let Some(manifest) = manager
        .load_conversation_manifest(workspace)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let Some(state) = manifest.acp else {
        return Ok(());
    };
    if state.identity.workspace_path != workspace.workspace_path()
        || manifest.issue_id != workspace.issue_id()
        || state.identity.checkout_generation.as_deref() != workspace.checkout_generation()
    {
        return Err("ACP cleanup identity does not match the bound workspace".into());
    }
    if let Some(host) = host {
        match host
            .lookup(state.owner_id.clone(), state.identity.generation)
            .await
        {
            Ok(handle) => {
                handle
                    .control(SessionControl::Retire)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
            Err(crate::opensymphony_acp::HostError::Unavailable) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    crate::opensymphony_acp::retire_persisted_session(manager.clone(), workspace.clone())
        .await
        .map_err(|error| error.to_string())
}

pub(super) async fn retire_and_archive(
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    host: &SessionHost,
    preserve_manifest: bool,
) -> Result<(), String> {
    retire(manager, workspace, Some(host)).await?;
    let manifest = manager
        .load_conversation_manifest(workspace)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("ACP conversation missing")?;
    let state = manifest.acp.as_ref().ok_or("ACP state missing")?;
    use sha2::{Digest, Sha256};
    let owner = format!("{:x}", Sha256::digest(state.owner_id.as_bytes()));
    let path = workspace.metadata_dir().join(format!(
        "acp-retired-{owner}-{}.json",
        state.identity.generation
    ));
    manager
        .write_json_artifact_atomically(workspace, &path, &manifest)
        .await
        .map_err(|e| e.to_string())?;
    if preserve_manifest {
        return Ok(());
    }
    let path = manager
        .validate_workspace_owned_path(workspace, &workspace.conversation_manifest_path())
        .await
        .map_err(|e| e.to_string())?;
    fs::remove_file(path).await.map_err(|e| e.to_string())
}

pub(super) async fn interrupt(
    active: &ActiveSessions,
    manager: &WorkspaceManager,
    command: &crate::opensymphony_domain::HarnessInterruptCommand,
) -> Result<WorkerInterruptAcknowledgement, CliWorkerError> {
    // Scheduler interrupt commands carry the issue identifier in `run_id`,
    // whereas active ACP sessions are keyed by generated worker IDs.
    let session = {
        let active = active.lock().map_err(|_| {
            CliWorkerError::InterruptFailed("ACP active-session registry unavailable".into())
        })?;
        let mut matching = active.values().filter(|session| {
            session.issue_id == command.issue_id
                && session.issue_identifier.as_str() == command.run_id
        });
        let session = matching.next().cloned().ok_or_else(|| {
            CliWorkerError::InterruptFailed("ACP worker has no active session".into())
        })?;
        if matching.next().is_some() {
            return Err(CliWorkerError::InterruptFailed(
                "ACP interrupt matches multiple active sessions".into(),
            ));
        }
        session
    };
    let snapshot = session
        .handle
        .inspect()
        .await
        .map_err(|e| CliWorkerError::InterruptFailed(e.to_string()))?;
    if snapshot.state.session_id.as_deref() != Some(command.conversation_id.as_str())
        || (matches!(
            snapshot.state.status,
            AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain
        ) && snapshot.state.identity.run_id != session.run_id)
        || snapshot.state.owner_id != session.handle.owner_id
        || snapshot.state.identity.generation != session.handle.generation
    {
        return Err(CliWorkerError::InterruptFailed(
            "ACP interrupt session identity mismatch".into(),
        ));
    }
    session.cancellation.cancel();
    let stopped = timeout(Duration::from_secs(15), async {
        loop {
            match observe_stop(&session, manager).await? {
                StopObservation::Stopped(reason) => return Ok(reason),
                StopObservation::Uncertain => {
                    return Err(CliWorkerError::InterruptFailed(
                        "ACP cancellation outcome is uncertain; cleanup remains fenced".into(),
                    ));
                }
                StopObservation::Pending => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await;
    match stopped {
        Ok(Ok(reason)) => Ok(WorkerInterruptAcknowledgement {
            accepted: true,
            detail: Some(format!(
                "ACP prompt stopped: {}",
                reason.unwrap_or_default()
            )),
            timed_out: false,
        }),
        Ok(Err(error)) => Err(error),
        Err(_) => Ok(WorkerInterruptAcknowledgement {
            accepted: false,
            detail: Some("ACP interrupt deadline exceeded; cleanup remains fenced".into()),
            timed_out: true,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_issue(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut RunManifest,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    workflow: &ResolvedWorkflow,
    terminal_prompt: Option<&str>,
    continuation_prompt: Option<&str>,
    host: &SessionHost,
    active: &ActiveSessions,
    updates: &mpsc::UnboundedSender<WorkerUpdate>,
    operator_update_notify: &Notify,
    launch: &mut Option<oneshot::Sender<LaunchReport>>,
    environment: BTreeMap<String, String>,
    excluded_environment: BTreeSet<String>,
    recovered: bool,
    fresh_conversation_grants: Option<&MemoryScopeGrantRegistry>,
) -> WorkerOutcomeRecord {
    let result = try_run(
        route,
        manager,
        workspace,
        manifest,
        issue,
        run,
        workflow,
        terminal_prompt,
        continuation_prompt,
        host,
        active,
        updates,
        operator_update_notify,
        launch,
        environment,
        excluded_environment,
        recovered,
        fresh_conversation_grants,
    )
    .await;
    let (kind, status, stopped, detail) = match result {
        Ok(reason) => match reason.as_str() {
            "end_turn" | "max_tokens" | "max_turn_requests" => (
                WorkerOutcomeKind::Succeeded,
                RunStatus::Succeeded,
                true,
                format!("ACP turn stopped: {reason}"),
            ),
            "cancelled" | "cancelled_before_prompt" => (
                WorkerOutcomeKind::Cancelled,
                RunStatus::Cancelled,
                true,
                "ACP cancellation acknowledged by prompt response".into(),
            ),
            // Refusal and unsupported stop reasons are not eligible for automatic retry.
            _ => (
                WorkerOutcomeKind::Detached,
                RunStatus::Failed,
                true,
                format!("ACP unsuccessful stop reason: {reason}"),
            ),
        },
        Err(detail) => {
            let loaded = manager.load_conversation_manifest(workspace).await;
            let unreadable = loaded.is_err();
            let state = loaded.ok().flatten().and_then(|m| m.acp);
            let uncertain = unreadable
                || (state.is_none() && manifest.started_at.is_some())
                || state.as_ref().is_some_and(|state| {
                    matches!(
                        state.status,
                        AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain
                    )
                });
            let stopped = !uncertain
                && state.as_ref().is_none_or(|state| {
                    state.status == AcpSessionStatus::Finished
                        || state.process == AcpProcessState::Stopped
                });
            (
                if uncertain {
                    WorkerOutcomeKind::Detached
                } else {
                    WorkerOutcomeKind::Failed
                },
                RunStatus::Failed,
                stopped,
                detail,
            )
        }
    };
    if launch.is_some() {
        // Expose uncertain ownership to the scheduler as a terminal fenced outcome instead
        // of treating a possibly delivered prompt as a retryable launch failure.
        if let Ok(Some(raw)) = manager
            .read_text_artifact(workspace, &workspace.conversation_manifest_path())
            .await
            && let Ok(view) = conversation_view(&raw)
        {
            if let Some(sender) = launch.take() {
                let mut metadata = conversation_metadata_from_manifest(&view);
                if let Ok(manifest) = serde_json::from_str::<
                    crate::opensymphony_workspace::ConversationManifest,
                >(&raw)
                    && let Some(state) = manifest.acp
                    && state.initialization["protocolVersion"].as_u64() == Some(1)
                {
                    metadata.harness_capability =
                        Some(Box::new(crate::opensymphony_acp::run_capability(&state)));
                }
                let _ = sender.send(LaunchReport::Conversation {
                    conversation: Box::new(metadata),
                    started_at: manifest.started_at.map(datetime_to_timestamp_ms),
                });
            }
        } else {
            report_launch_failure(launch, detail.clone());
        }
    }
    if let Ok(Some(conversation)) = manager.load_conversation_manifest(workspace).await {
        bind_current_run_envelopes(manifest, &conversation);
    }
    manifest.harness_stopped = stopped;
    manifest.status_detail = Some(detail.clone());
    manifest.status = status;
    let finish = if stopped {
        manager.finish_run(workspace, manifest, status).await
    } else {
        manager.write_run_manifest(workspace, manifest).await
    };
    let mut outcome = WorkerOutcomeRecord::from_run(
        run,
        if finish.is_err() && kind == WorkerOutcomeKind::Succeeded {
            WorkerOutcomeKind::Failed
        } else {
            kind
        },
        now_timestamp(),
        Some(detail),
        finish.err().map(|e| e.to_string()),
    );
    outcome.harness_stopped = stopped;
    active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(run.worker_id.as_str());
    outcome
}

pub(super) fn launch_environment(
    ambient: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    overlay: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut environment = ambient
        .into_iter()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| !crate::opensymphony_acp::is_reserved_memory_environment_name(name))
        .collect::<BTreeMap<_, _>>();
    for (name, value) in overlay {
        crate::opensymphony_workspace::insert_environment_value(
            &mut environment,
            name.clone(),
            value.clone(),
        );
    }
    environment
}

pub(super) struct EffectiveLaunchIdentity {
    pub profile: crate::opensymphony_acp::AcpProfile,
    pub services: HostServices,
    pub profile_fingerprint: String,
    pub credential_scope: String,
}

pub(super) fn effective_launch_identity(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    workflow: &ResolvedWorkflow,
    worker_environment: &BTreeMap<String, String>,
    environment: &BTreeMap<String, String>,
    limits: &ClientLimits,
) -> Result<EffectiveLaunchIdentity, String> {
    let profile_id = route
        .harness_profile
        .as_ref()
        .ok_or("ACP route has no profile identity")?;
    let mut profile = workflow
        .extensions
        .acp
        .profiles
        .get(profile_id)
        .cloned()
        .ok_or("persisted ACP profile is unavailable")?;
    if let Some(model) = &route.model {
        profile.session.model = Some(model.clone());
    }
    profile.validate().map_err(|error| error.to_string())?;
    let mut services = HostServices {
        read_files: true,
        write_files: true,
        terminals: true,
        ..HostServices::default()
    };
    if let Some(endpoint) = worker_environment.get("OPENSYMPHONY_MEMORY_ENDPOINT") {
        services.attach_scoped_memory(
            endpoint,
            worker_environment
                .get("OPENSYMPHONY_MEMORY_TOKEN")
                .map(String::as_str)
                .filter(|token| !token.is_empty()),
        );
    }
    let profile_fingerprint =
        crate::opensymphony_acp::launch_profile_fingerprint(&profile, &services, limits)?;
    let credential_scope = credential_scope(
        profile.env_refs.values().map(String::as_str).chain([
            "OPENSYMPHONY_MEMORY_TOKEN",
            "OPENSYMPHONY_MEMORY_ENDPOINT",
            "OPENSYMPHONY_MEMORY_PROJECT",
            "OPENSYMPHONY_MEMORY_PROJECT_SET",
            "OPENSYMPHONY_MEMORY_EXECUTION_REPO",
            "OPENSYMPHONY_MEMORY_AUTHORIZED_REPOSITORIES",
            "OPENSYMPHONY_MEMORY_RUN_ID",
            "OPENSYMPHONY_MEMORY_ATTEMPT",
        ]),
        environment,
    );
    Ok(EffectiveLaunchIdentity {
        profile,
        services,
        profile_fingerprint,
        credential_scope,
    })
}

fn credential_scope<'a>(
    names: impl IntoIterator<Item = &'a str>,
    environment: &BTreeMap<String, String>,
) -> String {
    // Fingerprint resolved credential scope without persisting credentials or environment.
    use sha2::{Digest, Sha256};
    let mut scope = Sha256::new();
    for name in names {
        if let Some((_, value)) = environment
            .iter()
            .find(|(key, _)| environment_variable_names_equal(key, name))
        {
            scope.update(name.as_bytes());
            scope.update([0]);
            scope.update(value.as_bytes());
            scope.update([0]);
        }
    }
    format!("{:x}", scope.finalize())
}

#[allow(clippy::too_many_arguments)]
async fn try_run(
    route: &crate::opensymphony_orchestrator::HarnessRouteDecision,
    manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    manifest: &mut RunManifest,
    issue: &NormalizedIssue,
    run: &crate::opensymphony_domain::RunAttempt,
    workflow: &ResolvedWorkflow,
    terminal_prompt: Option<&str>,
    continuation_prompt: Option<&str>,
    host: &SessionHost,
    active: &ActiveSessions,
    updates: &mpsc::UnboundedSender<WorkerUpdate>,
    operator_update_notify: &Notify,
    launch: &mut Option<oneshot::Sender<LaunchReport>>,
    environment: BTreeMap<String, String>,
    excluded_environment: BTreeSet<String>,
    recovered: bool,
    fresh_conversation_grants: Option<&MemoryScopeGrantRegistry>,
) -> Result<String, String> {
    let previous = manager
        .load_conversation_manifest(workspace)
        .await
        .map_err(|e| e.to_string())?
        .and_then(|m| m.acp);
    if recovered && let Some(previous) = &previous {
        if matches!(
            previous.status,
            AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain
        ) {
            return Err("ACP prior submission is uncertain; refusing automatic replay".into());
        }
        if previous.status == AcpSessionStatus::Finished
            && previous.identity.run_id == manifest.run_id
            && previous.identity.attempt == manifest.attempt
        {
            return Ok(previous
                .stop_reason
                .clone()
                .unwrap_or_else(|| "unsupported".into()));
        }
    }
    let worker_environment = environment;
    let environment = launch_environment(env::vars_os(), &worker_environment);
    let limits = production_client_limits();
    let EffectiveLaunchIdentity {
        profile,
        services,
        profile_fingerprint,
        credential_scope,
    } = effective_launch_identity(route, workflow, &worker_environment, &environment, &limits)?;
    let cursor_enabled = profile
        .extensions
        .iter()
        .any(|id| id == "cursor@2026.09.08-6caf4ff");
    let profile_id = route
        .harness_profile
        .as_ref()
        .ok_or("ACP route has no profile identity")?;
    manager
        .write_json_artifact_atomically(
            workspace,
            &workspace.metadata_dir().join("harness-route.json"),
            route,
        )
        .await
        .map_err(|e| e.to_string())?;
    let memory_prompt = memory_scope_prompt_from_environment(&worker_environment);
    let handle = host
        .open(SessionLaunch {
            manager: manager.clone(),
            workspace: workspace.clone(),
            identity: AcpSessionIdentity {
                profile_id: profile_id.clone(),
                profile_fingerprint,
                credential_scope,
                workspace_path: workspace.workspace_path().to_path_buf(),
                repository_binding: run.repository_binding.clone(),
                checkout_generation: workspace.checkout_generation().map(str::to_owned),
                generation: 0,
                run_id: manifest.run_id.clone(),
                attempt: manifest.attempt,
            },
            profile,
            context: LaunchContext {
                // The verified handle may be a generation directory or nested parent
                // workspace. Narrow the client boundary to its immediate parent;
                // the owner separately validates the full manager-owned identity.
                workspace_root: workspace
                    .workspace_path()
                    .parent()
                    .ok_or("ACP workspace has no parent")?
                    .to_path_buf(),
                workspace_key: workspace
                    .workspace_path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or("ACP workspace directory is invalid")?
                    .to_owned(),
                issue_workspace: workspace.workspace_path().to_path_buf(),
                environment,
                excluded_environment,
                services,
            },
            limits,
            require_persistence: manifest.parent_runtime_envelope.is_some(),
            expected_session_id: manifest
                .parent_runtime_envelope
                .as_ref()
                .and_then(|envelope| envelope.conversation_binding.clone()),
        })
        .await
        .map_err(|e| e.to_string())?;
    let snapshot = handle.inspect().await.map_err(|e| e.to_string())?;
    // A restored session can still be Ready when no prompt was ever submitted.
    // Recovery kind describes the peer session, not whether workflow context
    // has been seeded. Keep this separate from fresh-owner grant accounting.
    let needs_full_prompt = !snapshot.state.workflow_prompt_seeded();
    let fresh = snapshot.state.recovery == crate::opensymphony_workspace::AcpRecovery::Fresh
        && snapshot.state.status == AcpSessionStatus::Ready;
    let mut prompt = if needs_full_prompt {
        terminal_prompt
            .map(str::to_owned)
            .map(Ok)
            .unwrap_or_else(|| {
                workflow
                    .render_prompt(issue, run.attempt.map(|a| a.get()))
                    .map_err(|e| e.to_string())
            })?
    } else {
        continuation_prompt.unwrap_or("Continue the current issue from its retained workspace and conversation. Reconcile current tracker status and finish the remaining authorized work.").to_owned()
    };
    if let Some(scope) = memory_prompt {
        prompt.push_str(&scope);
    }
    manifest.status = RunStatus::Running;
    manifest.started_at = Some(chrono::Utc::now());
    if let Some(envelope) = manifest.runtime_envelope.as_mut() {
        envelope.acp_session = Some(snapshot.state.identity.clone());
        envelope.conversation_binding = snapshot.state.session_id.clone();
    }
    if let Some(envelope) = manifest.parent_runtime_envelope.as_mut() {
        envelope.conversation_binding = snapshot.state.session_id.clone();
    }
    manager
        .write_run_manifest(workspace, manifest)
        .await
        .map_err(|e| e.to_string())?;
    let raw = manager
        .read_text_artifact(workspace, &workspace.conversation_manifest_path())
        .await
        .map_err(|e| e.to_string())?
        .ok_or("ACP manifest is missing after launch")?;
    let view = conversation_view(&raw).map_err(|e| e.to_string())?;
    let mut metadata = conversation_metadata_from_manifest(&view);
    metadata.stream_state = RuntimeStreamState::Ready;
    let mut capability = crate::opensymphony_acp::run_capability(&snapshot.state);
    metadata.harness_capability = Some(Box::new(capability.clone()));
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    let prompt_finished = CancellationToken::new();
    let _finish_on_drop = prompt_finished.clone().drop_guard();
    let (operator_requests_tx, mut operator_requests_rx) = mpsc::channel(32);
    let (operator_responses_tx, mut operator_responses_rx) = mpsc::channel(32);
    let mut operator_waiters = HashMap::new();
    let active_session = ActiveSession {
        handle: handle.clone(),
        cancellation: cancellation.clone(),
        prompt_finished: prompt_finished.clone(),
        workspace: workspace.clone(),
        issue_id: issue.id.clone(),
        issue_identifier: issue.identifier.clone(),
        run_id: manifest.run_id.clone(),
        operator_responses: operator_responses_tx,
    };
    active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(run.worker_id.to_string(), active_session.clone());
    if let Some(sender) = launch.take() {
        let reported = sender.send(LaunchReport::Conversation {
            conversation: Box::new(metadata),
            started_at: manifest.started_at.map(datetime_to_timestamp_ms),
        });
        if fresh
            && reported.is_ok()
            && let Some(grants) = fresh_conversation_grants
        {
            grants.acknowledge_fresh_conversation(issue.identifier.as_str());
        }
    }
    let mut receiver = handle.subscribe();
    let baseline = handle
        .source_history()
        .latest_cursor
        .unwrap_or((handle.generation, 0));
    let mut projection = RuntimeProjection::after(baseline).with_cursor(
        cursor_enabled,
        snapshot.state.session_id.as_deref(),
        Some(&snapshot.state.identity.workspace_path),
    );
    let prompt = handle.prompt_with_operator(
        manifest.run_id.clone(),
        manifest.attempt,
        prompt,
        cancellation.clone(),
        Some(operator_requests_tx),
    );
    tokio::pin!(prompt);
    let mut operator_requests_open = true;
    let mut operator_responses_open = true;
    let result = loop {
        tokio::select! {
            result = &mut prompt => break result,
            operator = operator_requests_rx.recv(), if operator_requests_open => match operator {
                Some(AcpOperatorEvent::Opened(mut request)) => {
                    request.interaction.run_id = manifest.run_id.clone();
                    request.interaction.issue_id = issue.id.as_str().into();
                    request.interaction.issue_identifier = issue.identifier.to_string();
                    request.interaction.generation = handle.generation;
                    let request_id = request.interaction.request_id.clone();
                    if operator_waiters.insert(request_id.clone(), request.reply).is_some() {
                        return Err("ACP callback repeated its request identity".into());
                    }
                    if updates.send(WorkerUpdate::OperatorRequest { worker_id: run.worker_id.clone(), interaction: request.interaction }).is_ok() {
                        operator_update_notify.notify_one();
                    }
                }
                Some(AcpOperatorEvent::Closed(request_id)) => {
                    operator_waiters.remove(&request_id);
                    if updates.send(WorkerUpdate::OperatorClosed { worker_id: run.worker_id.clone(), request_id }).is_ok() {
                        operator_update_notify.notify_one();
                    }
                }
                // The owner drops this sender while cancelling a prompt that
                // has not been submitted. Keep waiting for the prompt's
                // terminal result so its cancellation evidence wins the race.
                None => operator_requests_open = false,
            },
            response = operator_responses_rx.recv(), if operator_responses_open => match response {
                Some(response) => forward_operator_response(response, &mut operator_waiters),
                None => operator_responses_open = false,
            },
            event = receiver.recv() => match event {
                Ok(SessionEvent::Gap { .. }) => return Err("ACP source stream lost an oversized frame; submission remains fenced".into()),
                Ok(event) => project_event(&event, &mut projection, &mut capability, &manifest.run_id, &run.worker_id, updates)?,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let history = handle.source_history();
                    if !history.covers_since(projection.last_cursor().unwrap_or(baseline)) { return Err("ACP worker source stream exceeded retention; submission remains fenced".into()); }
                    for event in history.events { project_event(&event, &mut projection, &mut capability, &manifest.run_id, &run.worker_id, updates)?; }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break Err(crate::opensymphony_acp::HostError::Unavailable),
            }
        }
    };
    // The owner's prompt response follows earlier updates; drain before publishing Finished.
    loop {
        match receiver.try_recv() {
            Ok(SessionEvent::Gap { .. }) => {
                return Err("ACP source stream lost an oversized frame".into());
            }
            Ok(event) => project_event(
                &event,
                &mut projection,
                &mut capability,
                &manifest.run_id,
                &run.worker_id,
                updates,
            )?,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                let history = handle.source_history();
                if !history.covers_since(projection.last_cursor().unwrap_or(baseline)) {
                    return Err("ACP worker source stream exceeded retention".into());
                }
                for event in history.events {
                    project_event(
                        &event,
                        &mut projection,
                        &mut capability,
                        &manifest.run_id,
                        &run.worker_id,
                        updates,
                    )?;
                }
            }
            Err(_) => break,
        }
    }
    prompt_finished.cancel();
    if result.is_err() && cancellation.is_cancelled() {
        let stopped = timeout(Duration::from_secs(15), async {
            loop {
                match observe_stop(&active_session, manager).await {
                    Ok(StopObservation::Stopped(_)) => return true,
                    Ok(StopObservation::Pending) => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Ok(StopObservation::Uncertain) | Err(_) => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        if stopped {
            return Ok("cancelled".into());
        }
    }
    result
        .map(|report| report.stop_reason)
        .map_err(|e| e.to_string())
}

fn project_event(
    event: &SessionEvent,
    projection: &mut RuntimeProjection,
    capability: &mut crate::opensymphony_gateway_schema::capability::HarnessRunCapability,
    run_id: &str,
    worker_id: &crate::opensymphony_domain::WorkerId,
    updates: &mpsc::UnboundedSender<WorkerUpdate>,
) -> Result<(), String> {
    if let SessionEvent::State { snapshot } = event
        && snapshot.state.identity.run_id == run_id
    {
        let negotiated = crate::opensymphony_acp::run_capability(&snapshot.state);
        if *capability != negotiated {
            *capability = negotiated.clone();
            let _ = updates.send(WorkerUpdate::HarnessCapabilityUpdate {
                worker_id: worker_id.clone(),
                capability: negotiated,
            });
        }
    }
    if let Some(update) = projection.apply(event, run_id) {
        let payload = (!update.payload.is_null()).then_some(update.payload);
        let _ = updates.send(WorkerUpdate::RuntimeEvent {
            worker_id: worker_id.clone(),
            observed_at: datetime_to_timestamp_ms(update.observed_at),
            event_id: Some(format!("acp-{}-{}", update.generation, update.sequence)),
            event_kind: Some(format!("acp.{}", update.kind)),
            summary: update.summary,
            payload,
        });
    }
    if projection.cursor_callback_saturated() {
        return Err("ACP Cursor callback projection saturated; submission remains fenced".into());
    }
    Ok(())
}

fn forward_operator_response(
    response: OperatorResponseCommand,
    waiters: &mut HashMap<String, oneshot::Sender<crate::opensymphony_acp::AcpOperatorReply>>,
) {
    if response.delivery.is_cancelled() {
        let _ = response.acknowledgement.send(false);
        return;
    }
    let reply = crate::opensymphony_acp::AcpOperatorReply {
        answer: response.answer,
        acknowledgement: response.acknowledgement,
        delivery: response.delivery,
    };
    if let Some(waiter) = waiters.remove(&response.request_id) {
        if let Err(reply) = waiter.send(reply) {
            let _ = reply.acknowledgement.send(false);
        }
    } else {
        let _ = reply.acknowledgement.send(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::ffi::OsStringExt;

    #[test]
    fn cursor_todo_projection_saturation_emits_diagnostic_and_fails_run() {
        let mut projection = RuntimeProjection::default().with_cursor(true, Some("session"), None);
        let mut capability = serde_json::from_value(serde_json::json!({
            "harness":"acp","profile_id":"cursor","protocol":"acp","protocol_version":1,
            "rpc":"json_rpc","encoding":"json","framing":"line_delimited","carrier":"stdio",
            "session_restore":false,"history_replay":false,"model_selection":false,
            "cancellation":true,"operator_responses":true
        }))
        .expect("capability");
        let worker = crate::opensymphony_domain::WorkerId::new("worker-cursor").expect("worker");
        let (updates, mut received) = mpsc::unbounded_channel();
        for sequence in 1..=17 {
            let event = SessionEvent::Source {
                generation: 1,
                run_id: "run".into(),
                replay: false,
                frame: crate::opensymphony_acp::SourceFrame {
                    sequence,
                    direction: "incoming".into(),
                    observed_at: chrono::Utc::now(),
                    payload: serde_json::json!({"id":sequence,"method":"cursor/update_todos","params":{
                        "toolCallId":"todo","merge":false,"todos":[]}}),
                },
            };
            let result = project_event(
                &event,
                &mut projection,
                &mut capability,
                "run",
                &worker,
                &updates,
            );
            if sequence < 17 {
                assert!(result.is_ok());
            } else {
                assert!(
                    result
                        .expect_err("saturation fails run")
                        .contains("saturated")
                );
            }
        }
        assert!(
            matches!(received.try_recv(), Ok(WorkerUpdate::RuntimeEvent { event_kind: Some(kind), .. }) if kind == "acp.cursor_todo_saturation")
        );
    }

    #[tokio::test]
    async fn expired_operator_ack_fences_delayed_worker_consume() {
        let delivery = crate::opensymphony_acp::AcpOperatorDeliveryFence::default();
        let (acknowledgement, mut received) = oneshot::channel();
        let response = OperatorResponseCommand {
            request_id: "queued-request".into(),
            answer: crate::opensymphony_gateway_schema::approval::OperatorAnswer::Cancel,
            acknowledgement,
            delivery: delivery.clone(),
        };
        let (waiter, mut callback) = oneshot::channel();
        let mut waiters = HashMap::from([("queued-request".into(), waiter)]);

        assert!(
            timeout(Duration::from_millis(10), &mut received)
                .await
                .is_err()
        );
        assert!(delivery.cancel(), "failure receipt wins the delivery fence");
        forward_operator_response(response, &mut waiters);
        assert!(!received.await.expect("negative acknowledgement"));
        assert!(waiters.contains_key("queued-request"));
        assert!(
            callback.try_recv().is_err(),
            "operator answer never reaches ACP"
        );
    }

    #[test]
    fn credential_scope_uses_platform_environment_name_rules() {
        let first = BTreeMap::from([("AGENT_TOKEN".into(), "first".into())]);
        let changed = BTreeMap::from([("AGENT_TOKEN".into(), "second".into())]);
        let source = ["agent_token"];
        assert_eq!(
            credential_scope(source, &first) != credential_scope(source, &changed),
            cfg!(windows),
            "only Windows resolves the differently cased credential reference"
        );
        assert_ne!(
            credential_scope(["AGENT_TOKEN"], &first),
            credential_scope(["AGENT_TOKEN"], &changed),
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_environment_skips_unrelated_non_utf8_ambient_values() {
        let ambient = [
            ("GOOD".into(), "ambient".into()),
            ("BAD".into(), std::ffi::OsString::from_vec(vec![0xff])),
            (std::ffi::OsString::from_vec(vec![0xfe]), "value".into()),
        ];
        let overlay = BTreeMap::from([
            ("GOOD".into(), "worker".into()),
            ("SCOPED".into(), "token".into()),
        ]);
        let environment = launch_environment(ambient, &overlay);
        assert_eq!(environment.get("GOOD").map(String::as_str), Some("worker"));
        assert_eq!(environment.get("SCOPED").map(String::as_str), Some("token"));
        assert!(!environment.contains_key("BAD"));
        assert_eq!(environment.len(), 2);
    }

    #[test]
    fn launch_environment_replaces_ambient_memory_scope_with_worker_grant() {
        let ambient = [
            (
                "OPENSYMPHONY_MEMORY_ADMIN_TOKEN".into(),
                "admin-bearer".into(),
            ),
            ("OPENSYMPHONY_MEMORY_TOKEN".into(), "stale-grant".into()),
            ("OPENSYMPHONY_MEMORY_PROJECT".into(), "stale-project".into()),
            ("PATH".into(), "/usr/bin".into()),
        ];
        let overlay = BTreeMap::from([
            ("OPENSYMPHONY_MEMORY_TOKEN".into(), "scoped-grant".into()),
            (
                "OPENSYMPHONY_MEMORY_PROJECT".into(),
                "current-project".into(),
            ),
        ]);
        let environment = launch_environment(ambient, &overlay);
        assert!(!environment.contains_key("OPENSYMPHONY_MEMORY_ADMIN_TOKEN"));
        assert_eq!(environment["OPENSYMPHONY_MEMORY_TOKEN"], "scoped-grant");
        assert_eq!(
            environment["OPENSYMPHONY_MEMORY_PROJECT"],
            "current-project"
        );
        assert_eq!(environment["PATH"], "/usr/bin");
    }
}
