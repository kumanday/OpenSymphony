//! One durable conversation record and one stable cross-process owner lock.
use std::{fs::File, io, path::Path};

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::AcpProfile;
use crate::opensymphony_workspace::{
    AcpProcessState, AcpRecovery, AcpSessionIdentity, AcpSessionState, AcpSessionStatus,
    ConversationManifest, WorkspaceError, WorkspaceHandle, WorkspaceManager,
};

const MAX_MANIFEST_BYTES: usize = 2 * 1024 * 1024;
const MAX_ID_BYTES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub(super) enum DurabilityError {
    #[error("ACP session already has a live owner")]
    Owned,
    #[error("ACP prior prompt outcome is uncertain; resolve execution risk before another attempt")]
    Uncertain,
    #[error(
        "ACP owner was lost during process launch; resolve process ownership before another launch"
    )]
    ProcessUncertain,
    #[error("ACP cannot replace a native conversation manifest")]
    NativeManifest,
    #[error("ACP durable session binding mismatch: {0}")]
    Binding(&'static str),
    #[error("ACP durable metadata exceeds its bounds or contains an invalid identifier")]
    InvalidMetadata,
    #[error("ACP durable filesystem operation failed ({0:?})")]
    Io(io::ErrorKind),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

impl From<io::Error> for DurabilityError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.kind())
    }
}

pub(super) fn profile_fingerprint(
    profile: &AcpProfile,
    services: &super::HostServices,
    limits: &super::ClientLimits,
) -> Result<String, DurabilityError> {
    // Persist only the digest: resolved MCP grants are part of session reuse
    // identity but must never enter durable manifests in cleartext.
    let encoded = serde_json::to_vec(&(
        profile,
        services.read_files,
        services.write_files,
        services.terminals,
        &services.mcp_servers,
        limits,
    ))
    .map_err(|_| DurabilityError::InvalidMetadata)?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

pub(super) struct Durability {
    manager: WorkspaceManager,
    workspace: WorkspaceHandle,
    manifest: ConversationManifest,
    // Never unlink this file: another process may already be waiting on its inode.
    _owner_lock: File,
}

impl Drop for Durability {
    fn drop(&mut self) {
        // Release explicitly: a concurrent fork can temporarily inherit an open
        // descriptor before exec closes it, delaying close-only flock release.
        let _ = self._owner_lock.unlock();
    }
}

impl Durability {
    pub(super) fn process_is_absent(&self) -> bool {
        #[cfg(unix)]
        {
            verify_prior_process(self.state().process).is_ok()
        }
        #[cfg(not(unix))]
        {
            // Windows owner-death recovery relies on closing the JobObject;
            // that assumption does not prove a live owner's failed stop.
            self.state().process == AcpProcessState::Stopped
        }
    }

    /// Zero generation is the trusted host's request to claim the latest record.
    /// A nonzero generation requires an exact match before reserving its successor.
    pub(super) async fn open(
        manager: WorkspaceManager,
        workspace: WorkspaceHandle,
        identity: AcpSessionIdentity,
    ) -> Result<Self, DurabilityError> {
        Self::claim(manager, workspace, identity, true).await
    }

    pub(super) async fn retire_persisted(
        manager: WorkspaceManager,
        workspace: WorkspaceHandle,
        identity: AcpSessionIdentity,
    ) -> Result<(), DurabilityError> {
        // The claim verifies both the exclusive owner lock and process absence;
        // reserve no launch, so a cleanup crash cannot leave LaunchPending behind.
        let _guard = Self::claim(manager, workspace, identity, false).await?;
        Ok(())
    }

    async fn claim(
        manager: WorkspaceManager,
        workspace: WorkspaceHandle,
        identity: AcpSessionIdentity,
        reserve_launch: bool,
    ) -> Result<Self, DurabilityError> {
        validate_identity(&identity)?;
        let issue = manager
            .load_issue_manifest(&workspace)
            .await?
            .ok_or(DurabilityError::Binding("issue manifest missing"))?;
        if issue.issue_id != workspace.issue_id()
            || issue.identifier != workspace.identifier()
            || issue.sanitized_workspace_key != workspace.workspace_key()
            || issue.workspace_path != workspace.workspace_path()
            || identity.workspace_path != workspace.workspace_path()
        {
            return Err(DurabilityError::Binding("workspace"));
        }
        if issue
            .repository_binding
            .as_ref()
            .and_then(|binding| binding.resolved_binding())
            != identity.repository_binding.as_ref()
        {
            return Err(DurabilityError::Binding("repository"));
        }
        if workspace.checkout_generation() != identity.checkout_generation.as_deref() {
            return Err(DurabilityError::Binding("checkout generation"));
        }

        let lock_path = workspace.metadata_dir().join("acp-owner.lock");
        let lock_path = manager
            .validate_workspace_owned_path(&workspace, &lock_path)
            .await?;
        let owner_lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;
        match owner_lock.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(DurabilityError::Owned),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        match tokio::fs::symlink_metadata(workspace.conversation_manifest_path()).await {
            Ok(metadata) if metadata.len() > MAX_MANIFEST_BYTES as u64 => {
                return Err(DurabilityError::InvalidMetadata);
            }
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let mut manifest = match manager.load_conversation_manifest(&workspace).await? {
            Some(manifest) => manifest,
            None => {
                let mut manifest = ConversationManifest::new(
                    &workspace,
                    "",
                    "acp://stdio",
                    workspace.metadata_dir(),
                    "acp-v1",
                );
                manifest.acp = Some(AcpSessionState {
                    harness: "acp".into(),
                    identity: AcpSessionIdentity {
                        generation: 0,
                        ..identity.clone()
                    },
                    session_id: None,
                    initialization: Value::Null,
                    model_selection: false,
                    status: AcpSessionStatus::Ready,
                    stop_reason: None,
                    workflow_prompt_seeded: Some(false),
                    recovery: AcpRecovery::Fresh,
                    owner_id: String::new(),
                    process: AcpProcessState::Stopped,
                });
                manifest
            }
        };
        if manifest.issue_id != workspace.issue_id()
            || manifest.identifier != workspace.identifier()
        {
            return Err(DurabilityError::Binding("issue"));
        }
        if reserve_launch && let Some(run) = manager.load_run_manifest(&workspace).await? {
            if run.run_id != identity.run_id || run.attempt != identity.attempt {
                return Err(DurabilityError::Binding("run identity"));
            }
            if let Some(envelope) = run.parent_runtime_envelope {
                if envelope.harness != "acp" || envelope.workspace_path != identity.workspace_path {
                    return Err(DurabilityError::Binding("parent runtime envelope"));
                }
                manifest.parent_runtime_envelope = Some(envelope);
            }
            if let Some(envelope) = run.runtime_envelope {
                if envelope.harness != "acp"
                    || envelope.checkout_path != identity.workspace_path
                    || Some(&envelope.repository_binding) != identity.repository_binding.as_ref()
                    || Some(envelope.checkout_generation.as_str())
                        != identity.checkout_generation.as_deref()
                {
                    return Err(DurabilityError::Binding("runtime envelope"));
                }
                manifest.runtime_envelope = Some(envelope);
            }
        }
        let state = manifest
            .acp
            .as_mut()
            .ok_or(DurabilityError::NativeManifest)?;
        validate_binding(&state.identity, &identity)?;
        if state.harness != "acp"
            || manifest.conversation_id != state.session_id.as_deref().unwrap_or_default()
        {
            return Err(DurabilityError::Binding("harness/session"));
        }
        if matches!(
            state.status,
            AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain
        ) {
            return Err(DurabilityError::Uncertain);
        }
        verify_prior_process(state.process)?;
        // Migrate before a new run clears the old terminal status and stop reason.
        if state.workflow_prompt_seeded.is_none() {
            state.workflow_prompt_seeded = Some(state.workflow_prompt_seeded());
        }
        if identity.generation != 0 && identity.generation != state.identity.generation {
            return Err(DurabilityError::Binding("connection generation"));
        }
        if reserve_launch {
            let new_run = state.identity.run_id != identity.run_id
                || state.identity.attempt != identity.attempt;
            state.identity.generation = state
                .identity
                .generation
                .checked_add(1)
                .ok_or(DurabilityError::Binding("connection generation exhausted"))?;
            state.identity.run_id = identity.run_id;
            state.identity.attempt = identity.attempt;
            if new_run {
                state.status = AcpSessionStatus::Ready;
                state.stop_reason = None;
            }
            state.owner_id = uuid::Uuid::new_v4().to_string();
            state.process = AcpProcessState::LaunchPending;
        } else {
            state.process = AcpProcessState::Stopped;
        }
        let durable = Self {
            manager,
            workspace,
            manifest,
            _owner_lock: owner_lock,
        };
        // Claim the generation before launching a process or establishing a connection.
        durable.persist().await?;
        Ok(durable)
    }

    pub(super) fn state(&self) -> &AcpSessionState {
        self.manifest
            .acp
            .as_ref()
            .expect("ACP manifest initialized on open")
    }

    pub(super) fn state_mut(&mut self) -> &mut AcpSessionState {
        self.manifest
            .acp
            .as_mut()
            .expect("ACP manifest initialized on open")
    }

    pub(super) async fn persist(&self) -> Result<(), DurabilityError> {
        validate_identity(&self.state().identity)?;
        if self
            .state()
            .session_id
            .as_deref()
            .is_some_and(|id| !valid_id(id))
            || serde_json::to_vec_pretty(&self.manifest)
                .map_err(|_| DurabilityError::InvalidMetadata)?
                .len()
                > MAX_MANIFEST_BYTES
        {
            return Err(DurabilityError::InvalidMetadata);
        }
        let path = self.workspace.conversation_manifest_path();
        let mut manifest = self.manifest.clone();
        if let Some(envelope) = &mut manifest.runtime_envelope {
            envelope.acp_session = Some(self.state().identity.clone());
            envelope.conversation_binding = self.state().session_id.clone();
            envelope.run_id.clone_from(&self.state().identity.run_id);
            envelope.attempt = self.state().identity.attempt;
        }
        if let Some(envelope) = &mut manifest.parent_runtime_envelope {
            envelope.conversation_binding = self.state().session_id.clone();
            envelope.run_id.clone_from(&self.state().identity.run_id);
            envelope.attempt = self.state().identity.attempt;
        }
        self.manager
            .write_json_artifact_atomically(&self.workspace, &path, &manifest)
            .await?;
        // Atomic replacement alone does not establish the pre-send durability barrier.
        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .await?;
        file.sync_all().await?;
        sync_directory(&self.workspace.metadata_dir()).await?;
        sync_directory(self.workspace.workspace_path()).await?;
        Ok(())
    }

    pub(super) async fn launched(&mut self, pid: u32) -> Result<(), DurabilityError> {
        if self.state().process != AcpProcessState::LaunchPending || pid == 0 {
            return Err(DurabilityError::InvalidMetadata);
        }
        self.state_mut().process = AcpProcessState::Running { pid };
        self.persist().await
    }

    /// Called only after supervised termination and reaping prove local quiescence.
    pub(super) async fn stopped(&mut self) -> Result<(), DurabilityError> {
        self.state_mut().process = AcpProcessState::Stopped;
        self.persist().await
    }

    pub(super) async fn ready(
        &mut self,
        session_id: String,
        initialization: Value,
        model_selection: bool,
        recovery: AcpRecovery,
        reset_reason: Option<String>,
    ) -> Result<(), DurabilityError> {
        self.require_quiescent()?;
        if !valid_id(&session_id) {
            return Err(DurabilityError::InvalidMetadata);
        }
        self.manifest.conversation_id = session_id.clone();
        self.manifest.fresh_conversation = recovery == AcpRecovery::Fresh;
        self.manifest.reset_reason = reset_reason;
        self.manifest.last_attached_at = Some(chrono::Utc::now());
        let state = self.state_mut();
        state.session_id = Some(session_id);
        state.initialization = initialization;
        state.model_selection = model_selection;
        state.recovery = recovery;
        if recovery == AcpRecovery::Fresh {
            state.workflow_prompt_seeded = Some(false);
        }
        state.status = AcpSessionStatus::Ready;
        state.stop_reason = None;
        self.persist().await
    }

    pub(super) async fn set_model_selection(
        &mut self,
        model_selection: bool,
    ) -> Result<(), DurabilityError> {
        self.require_quiescent()?;
        if self.state().model_selection != model_selection {
            self.state_mut().model_selection = model_selection;
            self.persist().await?;
        }
        Ok(())
    }

    pub(super) async fn submitted(
        &mut self,
        run_id: String,
        attempt: u32,
    ) -> Result<(), DurabilityError> {
        self.require_quiescent()?;
        if !valid_id(&run_id) || self.state().session_id.is_none() {
            return Err(DurabilityError::InvalidMetadata);
        }
        let state = self.state_mut();
        state.identity.run_id = run_id;
        state.identity.attempt = attempt;
        state.status = AcpSessionStatus::Submitted;
        state.stop_reason = None;
        self.persist().await
    }

    pub(super) async fn finished(&mut self, stop_reason: String) -> Result<(), DurabilityError> {
        if self.state().status != AcpSessionStatus::Submitted || !valid_id(&stop_reason) {
            return Err(DurabilityError::InvalidMetadata);
        }
        self.manifest.fresh_conversation = false;
        let state = self.state_mut();
        state.status = AcpSessionStatus::Finished;
        if stop_reason != "cancelled_before_prompt" {
            state.workflow_prompt_seeded = Some(true);
        }
        state.stop_reason = Some(stop_reason);
        self.persist().await
    }

    pub(super) async fn uncertain(&mut self) -> Result<(), DurabilityError> {
        self.state_mut().status = AcpSessionStatus::Uncertain;
        self.persist().await
    }

    fn require_quiescent(&self) -> Result<(), DurabilityError> {
        match self.state().status {
            AcpSessionStatus::Ready | AcpSessionStatus::Finished => Ok(()),
            AcpSessionStatus::Submitted | AcpSessionStatus::Uncertain => {
                Err(DurabilityError::Uncertain)
            }
        }
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_BYTES && !id.chars().any(char::is_control)
}

fn verify_prior_process(process: AcpProcessState) -> Result<(), DurabilityError> {
    let pid = match process {
        AcpProcessState::Stopped => return Ok(()),
        AcpProcessState::LaunchPending => return Err(DurabilityError::ProcessUncertain),
        AcpProcessState::Running { pid } => pid,
    };
    #[cfg(unix)]
    {
        let group = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
            .ok_or(DurabilityError::InvalidMetadata)?;
        // Only a missing process group proves local quiescence. EPERM and PID
        // reuse remain fenced; the lock alone does not prove an orphan exited.
        match rustix::process::test_kill_process_group(group) {
            Err(rustix::io::Errno::SRCH) => Ok(()),
            _ => Err(DurabilityError::Owned),
        }
    }
    #[cfg(windows)]
    {
        // The mandatory kill-on-close JobObject ends children on owner death.
        let _ = pid;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Err(DurabilityError::Owned)
    }
}

fn validate_identity(identity: &AcpSessionIdentity) -> Result<(), DurabilityError> {
    if !valid_id(&identity.profile_id)
        || !valid_id(&identity.profile_fingerprint)
        || !valid_id(&identity.credential_scope)
        || !valid_id(&identity.run_id)
        || identity
            .checkout_generation
            .as_deref()
            .is_some_and(|id| !valid_id(id))
        || !identity.workspace_path.is_absolute()
    {
        return Err(DurabilityError::InvalidMetadata);
    }
    Ok(())
}

fn validate_binding(
    stored: &AcpSessionIdentity,
    requested: &AcpSessionIdentity,
) -> Result<(), DurabilityError> {
    for (matches, name) in [
        (stored.profile_id == requested.profile_id, "profile"),
        (
            stored.profile_fingerprint == requested.profile_fingerprint,
            "profile fingerprint",
        ),
        (
            stored.credential_scope == requested.credential_scope,
            "credential scope",
        ),
        (
            stored.workspace_path == requested.workspace_path,
            "workspace",
        ),
        (
            stored.repository_binding == requested.repository_binding,
            "repository",
        ),
        (
            stored.checkout_generation == requested.checkout_generation,
            "checkout generation",
        ),
    ] {
        if !matches {
            return Err(DurabilityError::Binding(name));
        }
    }
    Ok(())
}

async fn sync_directory(path: &Path) -> Result<(), DurabilityError> {
    #[cfg(unix)]
    tokio::fs::File::open(path).await?.sync_all().await?;
    // Windows workspace replacement uses MoveFileExW(MOVEFILE_WRITE_THROUGH).
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_workspace::{
        CleanupConfig, HookConfig, IssueDescriptor, WorkspaceManagerConfig,
    };
    use serde_json::json;

    fn manager(root: &Path) -> WorkspaceManager {
        WorkspaceManager::new(WorkspaceManagerConfig {
            root: root.to_path_buf(),
            hooks: HookConfig::default(),
            cleanup: CleanupConfig::default(),
        })
        .expect("manager")
    }

    async fn workspace(root: &Path) -> WorkspaceHandle {
        manager(root)
            .ensure(&IssueDescriptor {
                issue_id: "issue-609".into(),
                identifier: "COE-609".into(),
                title: "durability".into(),
                current_state: "In Progress".into(),
                last_seen_tracker_refresh_at: None,
                repository_binding: None,
            })
            .await
            .expect("workspace")
            .handle
    }

    fn identity(workspace: &WorkspaceHandle) -> AcpSessionIdentity {
        AcpSessionIdentity {
            profile_id: "agent".into(),
            profile_fingerprint: "a".repeat(64),
            credential_scope: "grant-revision-1".into(),
            workspace_path: workspace.workspace_path().to_path_buf(),
            repository_binding: None,
            checkout_generation: None,
            generation: 0,
            run_id: "run-1".into(),
            attempt: 1,
        }
    }

    #[tokio::test]
    async fn owner_claim_is_exclusive_across_processes_and_reserves_generation() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let mut first = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("owner");
        assert_eq!(first.state().identity.generation, 1);
        assert!(matches!(
            Durability::open(manager(root.path()), workspace.clone(), input.clone()).await,
            Err(DurabilityError::Owned)
        ));
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new(std::env::current_exe().expect("test binary"))
                .args([
                    "--exact",
                    "opensymphony_acp::durable::tests::owner_lock_child",
                    "--nocapture",
                ])
                .env(
                    "ACP_TEST_OWNER_LOCK",
                    workspace.metadata_dir().join("acp-owner.lock"),
                )
                .output(),
        )
        .await
        .expect("child deadline")
        .expect("child");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        first.stopped().await.expect("first owner quiescent");
        drop(first);
        let mut second = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("next owner");
        assert_eq!(second.state().identity.generation, 2);
        second.stopped().await.expect("second owner quiescent");
        drop(second);
        let mut stale = input;
        stale.generation = 1;
        let stale_result = Durability::open(manager(root.path()), workspace, stale)
            .await
            .map(|_| ());
        assert!(
            matches!(
                stale_result,
                Err(DurabilityError::Binding("connection generation"))
            ),
            "{stale_result:?}"
        );
    }

    #[tokio::test]
    async fn new_claim_clears_prior_terminal_outcome_before_launch() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let first_identity = identity(&workspace);
        let mut first = Durability::open(
            manager(root.path()),
            workspace.clone(),
            first_identity.clone(),
        )
        .await
        .expect("first owner");
        first
            .ready(
                "session-1".into(),
                Value::Null,
                false,
                AcpRecovery::Fresh,
                None,
            )
            .await
            .expect("ready first turn");
        first
            .submitted(first_identity.run_id, first_identity.attempt)
            .await
            .expect("submit first turn");
        first
            .finished("end_turn".into())
            .await
            .expect("first terminal outcome");
        first.stopped().await.expect("quiescent");
        drop(first);

        let mut next_identity = identity(&workspace);
        next_identity.run_id = "run-2".into();
        next_identity.attempt = 2;
        let next = Durability::open(manager(root.path()), workspace.clone(), next_identity)
            .await
            .expect("next owner");
        assert_eq!(next.state().process, AcpProcessState::LaunchPending);
        assert_eq!(next.state().status, AcpSessionStatus::Ready);
        assert_eq!(next.state().stop_reason, None);
        assert!(next.state().workflow_prompt_seeded());
        let persisted = manager(root.path())
            .load_conversation_manifest(&workspace)
            .await
            .expect("manifest")
            .expect("manifest")
            .acp
            .expect("ACP state");
        assert_eq!(persisted.identity.run_id, "run-2");
        assert_eq!(persisted.identity.attempt, 2);
        assert_eq!(persisted.status, AcpSessionStatus::Ready);
        assert_eq!(persisted.stop_reason, None);
        assert_eq!(persisted.workflow_prompt_seeded, Some(true));
    }

    #[tokio::test]
    async fn legacy_finished_prompt_is_migrated_before_new_run_claim() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let mut first = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("first owner");
        first
            .ready(
                "session-1".into(),
                Value::Null,
                false,
                AcpRecovery::Fresh,
                None,
            )
            .await
            .expect("ready");
        first
            .submitted(input.run_id, input.attempt)
            .await
            .expect("submitted");
        first.finished("end_turn".into()).await.expect("finished");
        first.stopped().await.expect("stopped");
        drop(first);
        let mut legacy = manager(root.path())
            .load_conversation_manifest(&workspace)
            .await
            .expect("manifest")
            .expect("conversation");
        legacy.acp.as_mut().expect("ACP").workflow_prompt_seeded = None;
        manager(root.path())
            .write_json_artifact_atomically(
                &workspace,
                &workspace.conversation_manifest_path(),
                &legacy,
            )
            .await
            .expect("legacy manifest");

        let mut next = identity(&workspace);
        next.run_id = "run-2".into();
        next.attempt = 2;
        let claimed = Durability::open(manager(root.path()), workspace, next)
            .await
            .expect("second owner");
        assert_eq!(claimed.state().status, AcpSessionStatus::Ready);
        assert_eq!(claimed.state().workflow_prompt_seeded, Some(true));
    }

    #[tokio::test]
    async fn pre_prompt_cancellation_does_not_seed_workflow_context() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let mut owner = Durability::open(manager(root.path()), workspace, input.clone())
            .await
            .expect("owner");
        owner
            .ready(
                "session-1".into(),
                Value::Null,
                false,
                AcpRecovery::Fresh,
                None,
            )
            .await
            .expect("ready");
        owner
            .submitted(input.run_id, input.attempt)
            .await
            .expect("submitted");
        owner
            .finished("cancelled_before_prompt".into())
            .await
            .expect("cancelled");
        assert!(!owner.state().workflow_prompt_seeded());
    }

    #[test]
    fn owner_lock_child() {
        let Some(path) = std::env::var_os("ACP_TEST_OWNER_LOCK") else {
            return;
        };
        let file = File::options()
            .read(true)
            .write(true)
            .open(path)
            .expect("lock file");
        assert!(matches!(
            file.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
    }

    #[tokio::test]
    async fn crash_during_launch_retains_an_explicit_ownership_fence() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let owner = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("owner");
        assert_eq!(owner.state().process, AcpProcessState::LaunchPending);
        drop(owner);
        let result = Durability::open(manager(root.path()), workspace, input)
            .await
            .map(|_| ());
        assert!(
            matches!(result, Err(DurabilityError::ProcessUncertain)),
            "{result:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn owner_loss_refuses_a_surviving_process_group_and_allows_a_dead_one() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let mut owner = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("owner");
        let mut command = tokio::process::Command::new("sleep");
        command.arg("30").kill_on_drop(true);
        crate::opensymphony_workspace::configure_process_group(&mut command);
        let mut child = command.spawn().expect("process group");
        owner
            .launched(child.id().expect("child pid"))
            .await
            .expect("launch checkpoint");
        drop(owner);
        assert!(matches!(
            Durability::open(manager(root.path()), workspace.clone(), input.clone()).await,
            Err(DurabilityError::Owned)
        ));
        child.kill().await.expect("stop and reap orphan");
        let mut recovered = Durability::open(manager(root.path()), workspace, input)
            .await
            .expect("dead group can recover");
        assert_eq!(recovered.state().identity.generation, 2);
        recovered.stopped().await.expect("release reservation");
    }

    #[tokio::test]
    async fn submission_is_durable_and_blocks_recovery_until_terminal_evidence() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let mut owner = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("owner");
        owner
            .ready(
                "opaque/session:1".into(),
                json!({"protocolVersion": 1}),
                false,
                AcpRecovery::Fresh,
                None,
            )
            .await
            .expect("ready");
        owner
            .submitted("run-2".into(), 2)
            .await
            .expect("submission barrier");
        let stored = manager(root.path())
            .load_conversation_manifest(&workspace)
            .await
            .expect("load")
            .expect("manifest")
            .acp
            .expect("ACP");
        assert_eq!(stored.status, AcpSessionStatus::Submitted);
        assert_eq!(stored.identity.run_id, "run-2");
        assert_eq!(stored.identity.attempt, 2);
        assert!(matches!(
            owner
                .ready(
                    "replacement".into(),
                    Value::Null,
                    false,
                    AcpRecovery::Fresh,
                    Some("reset".into())
                )
                .await,
            Err(DurabilityError::Uncertain)
        ));
        drop(owner);
        assert!(matches!(
            Durability::open(manager(root.path()), workspace.clone(), input.clone()).await,
            Err(DurabilityError::Uncertain)
        ));

        // An explicit host resolution updates the existing record, preserving identity.
        let mut manifest = manager(root.path())
            .load_conversation_manifest(&workspace)
            .await
            .expect("load")
            .expect("manifest");
        manifest.acp.as_mut().expect("ACP").status = AcpSessionStatus::Finished;
        manifest.acp.as_mut().expect("ACP").process = AcpProcessState::Stopped;
        manager(root.path())
            .write_json_artifact_atomically(
                &workspace,
                &workspace.conversation_manifest_path(),
                &manifest,
            )
            .await
            .expect("resolve");
        let mut restored = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("resolved owner");
        assert_eq!(
            restored.state().session_id.as_deref(),
            Some("opaque/session:1")
        );
        restored
            .ready(
                "opaque/session:1".into(),
                json!({"protocolVersion": 1}),
                false,
                AcpRecovery::RestoredLoad,
                None,
            )
            .await
            .expect("load ready");
        restored
            .submitted("run-3".into(), 3)
            .await
            .expect("next submission");
        restored
            .finished("end_turn".into())
            .await
            .expect("terminal evidence");
        restored.stopped().await.expect("supervised process reaped");
        drop(restored);
        let mut finished_identity = input.clone();
        finished_identity.run_id = "run-3".into();
        finished_identity.attempt = 3;
        let mut reopened =
            Durability::open(manager(root.path()), workspace.clone(), finished_identity)
                .await
                .expect("finished recovery");
        assert_eq!(reopened.state().status, AcpSessionStatus::Finished);
        assert_eq!(reopened.state().stop_reason.as_deref(), Some("end_turn"));
        reopened.uncertain().await.expect("explicit risk fence");
        drop(reopened);
        assert!(matches!(
            Durability::open(manager(root.path()), workspace, input).await,
            Err(DurabilityError::Uncertain)
        ));
    }

    #[tokio::test]
    async fn reuse_checks_profile_scope_workspace_repository_and_checkout_binding() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let input = identity(&workspace);
        let mut owner = Durability::open(manager(root.path()), workspace.clone(), input.clone())
            .await
            .expect("owner");
        owner.stopped().await.expect("owner quiescent");
        drop(owner);
        for field in [
            "profile",
            "profile fingerprint",
            "credential scope",
            "workspace",
            "repository",
            "checkout generation",
        ] {
            let mut changed = input.clone();
            match field {
                "profile" => changed.profile_id.push('2'),
                "profile fingerprint" => changed.profile_fingerprint.push('2'),
                "credential scope" => changed.credential_scope.push('2'),
                "workspace" => changed.workspace_path.push("other"),
                "repository" => changed.repository_binding = Some(serde_json::from_value(json!({
                    "alias": "other",
                    "repository": {"id": "github:repository:other", "safe_remote_fingerprint": "sha256:other"},
                    "config_generation": "config-1",
                    "inventory_generation": "inventory-1"
                })).expect("repository binding")),
                "checkout generation" => changed.checkout_generation = Some("checkout-2".into()),
                _ => unreachable!(),
            }
            assert!(
                matches!(Durability::open(manager(root.path()), workspace.clone(), changed).await, Err(DurabilityError::Binding(name)) if name == field),
                "{field}"
            );
        }
        let mut manifest = manager(root.path())
            .load_conversation_manifest(&workspace)
            .await
            .expect("load")
            .expect("manifest");
        manifest.issue_id = "another-issue".into();
        manager(root.path())
            .write_conversation_manifest(&workspace, &manifest)
            .await
            .expect("foreign issue");
        assert!(matches!(
            Durability::open(manager(root.path()), workspace, input).await,
            Err(DurabilityError::Binding("issue"))
        ));
    }

    #[tokio::test]
    async fn native_manifests_remain_readable_and_are_not_overwritten() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let native = ConversationManifest::new(
            &workspace,
            "native-session",
            "http://localhost",
            workspace.metadata_dir(),
            "native-v1",
        );
        let raw = serde_json::to_value(&native).expect("native serialization");
        assert!(raw.get("acp").is_none());
        let decoded: ConversationManifest = serde_json::from_value(raw).expect("legacy manifest");
        assert!(decoded.acp.is_none());
        manager(root.path())
            .write_conversation_manifest(&workspace, &native)
            .await
            .expect("native write");
        assert!(matches!(
            Durability::open(
                manager(root.path()),
                workspace.clone(),
                identity(&workspace)
            )
            .await,
            Err(DurabilityError::NativeManifest)
        ));
        assert_eq!(
            manager(root.path())
                .load_conversation_manifest(&workspace)
                .await
                .expect("read")
                .expect("native"),
            native
        );
    }

    #[tokio::test]
    async fn invalid_or_oversized_metadata_never_replaces_the_durable_record() {
        let root = tempfile::tempdir().expect("temp");
        let workspace = workspace(root.path()).await;
        let mut owner = Durability::open(
            manager(root.path()),
            workspace.clone(),
            identity(&workspace),
        )
        .await
        .expect("owner");
        let before = std::fs::read(workspace.conversation_manifest_path()).expect("reservation");
        assert!(matches!(
            owner
                .ready(
                    "bad\nsession".into(),
                    Value::Null,
                    false,
                    AcpRecovery::Fresh,
                    None
                )
                .await,
            Err(DurabilityError::InvalidMetadata)
        ));
        assert!(matches!(
            owner
                .ready(
                    "session".into(),
                    json!("x".repeat(MAX_MANIFEST_BYTES)),
                    false,
                    AcpRecovery::Fresh,
                    None
                )
                .await,
            Err(DurabilityError::InvalidMetadata)
        ));
        assert_eq!(
            std::fs::read(workspace.conversation_manifest_path()).expect("reservation remains"),
            before
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlinked_lock_and_manifest_without_touching_the_target() {
        for filename in ["acp-owner.lock", "conversation.json"] {
            let root = tempfile::tempdir().expect("temp");
            let workspace = workspace(root.path()).await;
            let outside = root.path().join("outside");
            std::fs::write(&outside, "unchanged").expect("outside");
            std::os::unix::fs::symlink(&outside, workspace.metadata_dir().join(filename))
                .expect("symlink");
            assert!(matches!(
                Durability::open(
                    manager(root.path()),
                    workspace.clone(),
                    identity(&workspace)
                )
                .await,
                Err(DurabilityError::Workspace(
                    WorkspaceError::ManagedPathSymlink { .. }
                ))
            ));
            assert_eq!(
                std::fs::read_to_string(outside).expect("outside remains"),
                "unchanged"
            );
        }
    }
}
