//! Production worker wiring. ACP wire types and protocol ownership stay in opensymphony_acp.
use super::*;
use crate::opensymphony_acp::{
    ClientLimits, LaunchContext, RetentionPolicy, RuntimeProjection, SessionControl, SessionEvent,
    SessionHandle, SessionHost, SessionLaunch,
};
use crate::opensymphony_workspace::{AcpProcessState, AcpSessionIdentity, AcpSessionStatus};
use tokio_util::sync::CancellationToken;

pub(super) const KIND: &str = "acp";
#[derive(Clone)]
pub(super) struct ActiveSession {
    pub handle: SessionHandle,
    pub cancellation: CancellationToken,
    pub issue_id: IssueId,
    pub run_id: String,
}
pub(super) type ActiveSessions = Arc<Mutex<HashMap<String, ActiveSession>>>;

pub(super) fn new_host() -> SessionHost {
    SessionHost::new(RetentionPolicy {
        max_sessions: 128,
        ..RetentionPolicy::default()
    })
    .expect("valid production ACP retention policy")
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
                .pointer("/acp/status")
                .and_then(serde_json::Value::as_str)
                != Some("ready")
        );
        if value["conversation_id"].as_str() == Some("") {
            // A launch reservation can precede the peer session ID. This identifies the
            // owner for scheduler fencing, and is never submitted as an ACP session ID.
            value["conversation_id"] = value["acp"]["owner_id"].clone();
        }
    }
    serde_json::from_value(value)
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
    let path = manager
        .validate_workspace_owned_path(workspace, &workspace.conversation_manifest_path())
        .await
        .map_err(|e| e.to_string())?;
    fs::remove_file(path).await.map_err(|e| e.to_string())
}

pub(super) async fn interrupt(
    active: &ActiveSessions,
    command: &crate::opensymphony_domain::HarnessInterruptCommand,
) -> Result<WorkerInterruptAcknowledgement, CliWorkerError> {
    let session = active
        .lock()
        .map_err(|_| {
            CliWorkerError::InterruptFailed("ACP active-session registry unavailable".into())
        })?
        .get(
            command
                .run_id
                .strip_prefix("run-")
                .unwrap_or(&command.run_id),
        )
        .cloned()
        .ok_or_else(|| {
            CliWorkerError::InterruptFailed("ACP worker has no active session".into())
        })?;
    if session.issue_id != command.issue_id || session.run_id != command.run_id {
        return Err(CliWorkerError::InterruptFailed(
            "ACP interrupt run identity mismatch".into(),
        ));
    }
    let ActiveSession {
        handle,
        cancellation,
        ..
    } = session;
    let snapshot = handle
        .inspect()
        .await
        .map_err(|e| CliWorkerError::InterruptFailed(e.to_string()))?;
    if snapshot.state.session_id.as_deref() != Some(command.conversation_id.as_str()) {
        return Err(CliWorkerError::InterruptFailed(
            "ACP interrupt session identity mismatch".into(),
        ));
    }
    cancellation.cancel();
    let stopped = timeout(Duration::from_secs(15), async {
        loop {
            let snapshot = handle
                .inspect()
                .await
                .map_err(|e| CliWorkerError::InterruptFailed(e.to_string()))?;
            match snapshot.state.status {
                AcpSessionStatus::Finished => return Ok(snapshot.state.stop_reason),
                AcpSessionStatus::Ready => return Ok(Some("cancelled_before_submission".into())),
                AcpSessionStatus::Uncertain => {
                    return Err(CliWorkerError::InterruptFailed(
                        "ACP cancellation outcome is uncertain; cleanup remains fenced".into(),
                    ));
                }
                _ => tokio::time::sleep(Duration::from_millis(20)).await,
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
    launch: &mut Option<oneshot::Sender<LaunchReport>>,
    environment: BTreeMap<String, String>,
    excluded_environment: BTreeSet<String>,
    recovered: bool,
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
        launch,
        environment,
        excluded_environment,
        recovered,
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
            "cancelled" => (
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
        if conversation.runtime_envelope.is_some() {
            manifest.runtime_envelope = conversation.runtime_envelope;
        }
        if conversation.parent_runtime_envelope.is_some() {
            manifest.parent_runtime_envelope = conversation.parent_runtime_envelope;
        }
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
    launch: &mut Option<oneshot::Sender<LaunchReport>>,
    environment: BTreeMap<String, String>,
    excluded_environment: BTreeSet<String>,
    recovered: bool,
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
        if previous.status == AcpSessionStatus::Finished {
            return Ok(previous
                .stop_reason
                .clone()
                .unwrap_or_else(|| "unsupported".into()));
        }
    }
    let mut inherited = env::vars().collect::<BTreeMap<_, _>>();
    for (name, value) in environment {
        crate::opensymphony_workspace::insert_environment_value(&mut inherited, name, value);
    }
    let environment = inherited;
    let profile_id = route
        .harness_profile
        .as_ref()
        .ok_or("ACP route has no profile identity")?;
    let profile = workflow
        .extensions
        .acp
        .profiles
        .get(profile_id)
        .cloned()
        .ok_or("persisted ACP profile is unavailable")?;
    // Fingerprint resolved credential scope without persisting credentials or environment.
    use sha2::{Digest, Sha256};
    let mut scope = Sha256::new();
    for name in profile.env_refs.values().map(String::as_str).chain([
        "OPENSYMPHONY_MEMORY_TOKEN",
        "OPENSYMPHONY_MEMORY_ENDPOINT",
        "OPENSYMPHONY_MEMORY_PROJECT",
        "OPENSYMPHONY_MEMORY_EXECUTION_REPO",
        "OPENSYMPHONY_MEMORY_AUTHORIZED_REPOSITORIES",
    ]) {
        if let Some(value) = environment.get(name) {
            scope.update(name.as_bytes());
            scope.update([0]);
            scope.update(value.as_bytes());
            scope.update([0]);
        }
    }
    manager
        .write_json_artifact_atomically(
            workspace,
            &workspace.metadata_dir().join("harness-route.json"),
            route,
        )
        .await
        .map_err(|e| e.to_string())?;
    let memory_prompt = memory_scope_prompt_from_environment(&environment);
    let handle = host
        .open(SessionLaunch {
            manager: manager.clone(),
            workspace: workspace.clone(),
            identity: AcpSessionIdentity {
                profile_id: profile_id.clone(),
                profile_fingerprint: String::new(),
                credential_scope: format!("{:x}", scope.finalize()),
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
            },
            limits: ClientLimits::default(),
            require_persistence: false,
        })
        .await
        .map_err(|e| e.to_string())?;
    let snapshot = handle.inspect().await.map_err(|e| e.to_string())?;
    let fresh = snapshot.state.recovery == crate::opensymphony_workspace::AcpRecovery::Fresh
        && snapshot.state.status == AcpSessionStatus::Ready;
    let mut prompt = if fresh {
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
    metadata.harness_capability = Some(Box::new(crate::opensymphony_acp::run_capability(
        &snapshot.state,
    )));
    let cancellation = CancellationToken::new();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    active.lock().unwrap_or_else(|e| e.into_inner()).insert(
        run.worker_id.to_string(),
        ActiveSession {
            handle: handle.clone(),
            cancellation: cancellation.clone(),
            issue_id: issue.id.clone(),
            run_id: manifest.run_id.clone(),
        },
    );
    if let Some(sender) = launch.take() {
        let _ = sender.send(LaunchReport::Conversation {
            conversation: Box::new(metadata),
            started_at: manifest.started_at.map(datetime_to_timestamp_ms),
        });
    }
    let mut receiver = handle.subscribe();
    let mut projection = RuntimeProjection::default();
    let prompt = handle.prompt(
        manifest.run_id.clone(),
        manifest.attempt,
        prompt,
        cancellation.clone(),
    );
    tokio::pin!(prompt);
    let result = loop {
        tokio::select! {
            result = &mut prompt => break result,
            event = receiver.recv() => match event {
                Ok(SessionEvent::Gap { .. }) => return Err("ACP source stream lost an oversized frame; submission remains fenced".into()),
                Ok(event) => project_event(&event, &mut projection, &manifest.run_id, &run.worker_id, updates),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let history = handle.source_history();
                    if history.truncated { return Err("ACP worker source stream exceeded retention; submission remains fenced".into()); }
                    for event in history.events { project_event(&event, &mut projection, &manifest.run_id, &run.worker_id, updates); }
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
                &manifest.run_id,
                &run.worker_id,
                updates,
            ),
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                let history = handle.source_history();
                if history.truncated {
                    return Err("ACP worker source stream exceeded retention".into());
                }
                for event in history.events {
                    project_event(
                        &event,
                        &mut projection,
                        &manifest.run_id,
                        &run.worker_id,
                        updates,
                    );
                }
            }
            Err(_) => break,
        }
    }
    if result.is_err()
        && cancellation.is_cancelled()
        && handle.inspect().await.is_ok_and(|snapshot| {
            matches!(
                snapshot.state.status,
                AcpSessionStatus::Ready | AcpSessionStatus::Finished
            )
        })
    {
        return Ok("cancelled".into());
    }
    result
        .map(|report| report.stop_reason)
        .map_err(|e| e.to_string())
}

fn project_event(
    event: &SessionEvent,
    projection: &mut RuntimeProjection,
    run_id: &str,
    worker_id: &crate::opensymphony_domain::WorkerId,
    updates: &mpsc::UnboundedSender<WorkerUpdate>,
) {
    if let Some(update) = projection.apply(event, run_id) {
        let _ = updates.send(WorkerUpdate::RuntimeEvent {
            worker_id: worker_id.clone(),
            observed_at: datetime_to_timestamp_ms(update.observed_at),
            event_id: Some(format!("acp-{}-{}", update.generation, update.sequence)),
            event_kind: Some(format!("acp.{}", update.kind)),
            summary: update.summary,
            payload: Some(update.payload),
        });
    }
}
