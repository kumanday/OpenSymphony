//! Harness-neutral debug attachment resolution for host-owned ACP sessions.
//!
//! This module deliberately stops at the adapter boundary.  Callers receive an
//! opaque target and, when the owner is still present, the existing
//! `SessionHandle`.  They do not need to know the downstream protocol session
//! type or start another agent process.

use super::{HostError, SessionHandle, SessionHost, SessionSnapshot};
use crate::opensymphony_workspace::{
    AcpRecovery, AcpSessionIdentity, ConversationManifest, RunManifest, WorkspaceHandle,
    WorkspaceManager,
};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct AttachmentRequest {
    /// The exact path supplied by an IDE or terminal client.
    pub cwd: Option<PathBuf>,
    /// A profile selected by the caller.  `None` means use the persisted
    /// profile and never redirect an existing session to the current default.
    pub profile_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentTarget {
    pub harness: String,
    pub profile_id: String,
    /// Opaque to the attachment core.  ACP adapters may use UUIDs or another
    /// agent-defined identifier without leaking that representation here.
    pub native_session_id: Option<String>,
    pub owner_id: String,
    pub generation: u64,
    pub run_id: String,
    pub attempt: u32,
    pub workspace_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentMode {
    Live,
    TranscriptInspection,
    Unavailable,
}

#[derive(Debug)]
pub struct DebugAttachment {
    pub target: AttachmentTarget,
    pub mode: AttachmentMode,
    pub snapshot: SessionSnapshot,
    pub owner: Option<SessionHandle>,
}

impl DebugAttachment {
    pub fn is_live(&self) -> bool {
        self.mode == AttachmentMode::Live && self.owner.is_some()
    }

    /// A transcript is evidence only.  It cannot be used to submit a prompt.
    pub fn is_transcript_only(&self) -> bool {
        self.mode == AttachmentMode::TranscriptInspection
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AttachmentError {
    #[error("attachment cwd must be the exact issue workspace, not {provided}")]
    WrongWorkspace { provided: PathBuf },
    #[error("issue workspace manifest is missing or does not identify this workspace")]
    InvalidWorkspace,
    #[error("run manifest is missing or does not match the issue workspace")]
    InvalidRun,
    #[error("conversation manifest is missing or does not match the issue workspace")]
    InvalidConversation,
    #[error("persisted ACP binding does not match the current runtime envelope: {0}")]
    BindingMismatch(&'static str),
    #[error("profile `{requested}` does not match the persisted ACP profile `{persisted}`")]
    ProfileMismatch {
        requested: String,
        persisted: String,
    },
    #[error("workspace validation failed: {0}")]
    Workspace(#[from] crate::opensymphony_workspace::WorkspaceError),
    #[error("failed to resolve attachment path: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve an IDE/terminal request against the host-owned ACP session.
///
/// The resolver validates all durable identities before looking up the owner.
/// An owner loss produces a transcript or unavailable status and never starts
/// a replacement conversation.
pub async fn resolve(
    manager: &WorkspaceManager,
    host: &SessionHost,
    workspace: &WorkspaceHandle,
    request: AttachmentRequest,
) -> Result<DebugAttachment, AttachmentError> {
    if let Some(cwd) = request.cwd.as_deref()
        && !same_canonical_path(cwd, workspace.workspace_path()).await?
    {
        return Err(AttachmentError::WrongWorkspace {
            provided: cwd.to_path_buf(),
        });
    }

    let issue = manager
        .load_issue_manifest(workspace)
        .await?
        .filter(|manifest| {
            manifest.issue_id == workspace.issue_id()
                && manifest.identifier == workspace.identifier()
                && manifest.sanitized_workspace_key == workspace.workspace_key()
                && manifest.workspace_path == workspace.workspace_path()
        })
        .ok_or(AttachmentError::InvalidWorkspace)?;
    let run = manager
        .load_run_manifest(workspace)
        .await?
        .filter(|manifest| run_matches_workspace(manifest, workspace))
        .ok_or(AttachmentError::InvalidRun)?;
    let conversation = manager
        .load_conversation_manifest(workspace)
        .await?
        .filter(|manifest| conversation_matches_workspace(manifest, workspace))
        .ok_or(AttachmentError::InvalidConversation)?;
    let state = conversation
        .acp
        .as_ref()
        .ok_or(AttachmentError::BindingMismatch("harness is not ACP"))?;
    let envelope = run
        .runtime_envelope
        .as_ref()
        .ok_or(AttachmentError::BindingMismatch(
            "runtime envelope is missing",
        ))?;
    validate_binding(
        &issue,
        &run,
        &conversation,
        state.identity.clone(),
        state.session_id.as_deref(),
        envelope,
    )?;
    if let Some(requested) = request.profile_id.as_deref()
        && requested != state.identity.profile_id
    {
        return Err(AttachmentError::ProfileMismatch {
            requested: requested.to_owned(),
            persisted: state.identity.profile_id.clone(),
        });
    }

    let target = AttachmentTarget {
        harness: state.harness.clone(),
        profile_id: state.identity.profile_id.clone(),
        native_session_id: state.session_id.clone(),
        owner_id: state.owner_id.clone(),
        generation: state.identity.generation,
        run_id: state.identity.run_id.clone(),
        attempt: state.identity.attempt,
        workspace_path: workspace.workspace_path().to_path_buf(),
    };

    match host
        .lookup(target.owner_id.clone(), target.generation)
        .await
    {
        Ok(owner) => {
            let snapshot = owner
                .inspect()
                .await
                .map_err(|error| AttachmentError::BindingMismatch(host_error_detail(error)))?;
            Ok(DebugAttachment {
                target,
                mode: AttachmentMode::Live,
                snapshot,
                owner: Some(owner),
            })
        }
        Err(HostError::Unavailable) => {
            let snapshot = super::inspect_recorded_session(manager, workspace)
                .await
                .map_err(|error| AttachmentError::BindingMismatch(host_error_detail(error)))?;
            let mode = if snapshot.state.recovery == AcpRecovery::TranscriptOnly {
                AttachmentMode::TranscriptInspection
            } else {
                AttachmentMode::Unavailable
            };
            Ok(DebugAttachment {
                target,
                mode,
                snapshot,
                owner: None,
            })
        }
        Err(error) => Err(AttachmentError::BindingMismatch(host_error_detail(error))),
    }
}

fn run_matches_workspace(manifest: &RunManifest, workspace: &WorkspaceHandle) -> bool {
    manifest.issue_id == workspace.issue_id()
        && manifest.identifier == workspace.identifier()
        && manifest.sanitized_workspace_key == workspace.workspace_key()
        && manifest.workspace_path == workspace.workspace_path()
}

fn conversation_matches_workspace(
    manifest: &ConversationManifest,
    workspace: &WorkspaceHandle,
) -> bool {
    manifest.issue_id == workspace.issue_id() && manifest.identifier == workspace.identifier()
}

fn validate_binding(
    issue: &crate::opensymphony_workspace::IssueManifest,
    run: &RunManifest,
    conversation: &ConversationManifest,
    identity: AcpSessionIdentity,
    session_id: Option<&str>,
    envelope: &crate::opensymphony_workspace::TerminalRuntimeEnvelope,
) -> Result<(), AttachmentError> {
    if issue
        .repository_binding
        .as_ref()
        .and_then(|binding| binding.resolved_binding())
        != Some(&envelope.repository_binding)
    {
        return Err(AttachmentError::BindingMismatch("repository"));
    }
    if envelope.harness != "acp"
        || envelope.checkout_path != run.workspace_path
        || envelope.run_id != run.run_id
        || envelope.attempt != run.attempt
        || identity.workspace_path != run.workspace_path
        || identity.run_id != run.run_id
        || identity.attempt != run.attempt
        || identity.checkout_generation.as_deref() != Some(envelope.checkout_generation.as_str())
        || conversation.conversation_id != session_id.unwrap_or_default()
        || envelope.conversation_binding.as_deref() != session_id
        || envelope.acp_session.as_ref() != Some(&identity)
    {
        return Err(AttachmentError::BindingMismatch("runtime/session"));
    }
    Ok(())
}

async fn same_canonical_path(left: &Path, right: &Path) -> Result<bool, AttachmentError> {
    let left = tokio::fs::canonicalize(left).await?;
    let right = tokio::fs::canonicalize(right).await?;
    Ok(left == right)
}

fn host_error_detail(error: HostError) -> &'static str {
    match error {
        HostError::Unavailable => "owner unavailable",
        HostError::IdentityMismatch => "owner identity",
        HostError::Busy => "owner busy",
        HostError::ResourceLimit => "owner resource limit",
        HostError::Persistence => "owner persistence",
        HostError::AlreadyOwned => "owner already owned",
        HostError::UncertainSubmission => "uncertain submission",
        HostError::UncertainLaunch => "uncertain launch",
        HostError::NativeManifest => "native manifest",
        HostError::PersistenceUnsupported => "persistence unsupported",
        HostError::Client(_) => "owner client",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_workspace::{
        CleanupConfig, HookConfig, IssueDescriptor, WorkspaceManagerConfig,
    };

    async fn workspace(root: &Path) -> (WorkspaceManager, WorkspaceHandle) {
        let manager = WorkspaceManager::new(WorkspaceManagerConfig {
            root: root.to_path_buf(),
            hooks: HookConfig::default(),
            cleanup: CleanupConfig::default(),
        })
        .expect("workspace manager");
        let handle = manager
            .ensure(&IssueDescriptor {
                issue_id: "issue-455".into(),
                identifier: "COE-455".into(),
                title: "debug attachment".into(),
                current_state: "In Progress".into(),
                last_seen_tracker_refresh_at: None,
                repository_binding: None,
            })
            .await
            .expect("workspace")
            .handle;
        (manager, handle)
    }

    #[tokio::test]
    async fn rejects_parent_and_nested_paths_before_manifest_lookup() {
        let root = tempfile::tempdir().expect("temp root");
        let (manager, handle) = workspace(root.path()).await;
        let host = SessionHost::new(Default::default()).expect("host");

        for cwd in [
            root.path().to_path_buf(),
            handle.workspace_path().join(".opensymphony"),
        ] {
            let error = resolve(
                &manager,
                &host,
                &handle,
                AttachmentRequest {
                    cwd: Some(cwd),
                    ..Default::default()
                },
            )
            .await
            .expect_err("non-exact cwd must be rejected");
            assert!(matches!(error, AttachmentError::WrongWorkspace { .. }));
        }
    }

    #[tokio::test]
    async fn exact_workspace_without_runtime_binding_cannot_start_replacement() {
        let root = tempfile::tempdir().expect("temp root");
        let (manager, handle) = workspace(root.path()).await;
        let host = SessionHost::new(Default::default()).expect("host");
        let error = resolve(
            &manager,
            &host,
            &handle,
            AttachmentRequest {
                cwd: Some(handle.workspace_path().to_path_buf()),
                ..Default::default()
            },
        )
        .await
        .expect_err("missing run binding must be rejected");
        assert!(matches!(error, AttachmentError::InvalidRun));
    }
}
