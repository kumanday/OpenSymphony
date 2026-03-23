mod error;
mod manager;
mod models;
mod paths;

pub use error::{WorkspaceError, WorkspaceOwnershipConflictDetails};
pub use manager::WorkspaceManager;
pub use models::{
    CleanupConfig, CleanupDecision, CleanupOutcome, ConversationManifest, EnsureWorkspaceResult,
    HookConfig, HookDefinition, HookExecutionRecord, HookExecutionStatus, HookKind,
    IssueContextArtifact, IssueDescriptor, IssueLifecycleState, IssueManifest,
    PromptCaptureDescriptor, PromptCaptureManifest, PromptKind, RunDescriptor, RunManifest,
    RunStatus, SessionContextArtifact, WorkspaceHandle, WorkspaceManagerConfig,
};
pub use paths::{resolve_path_within_root, sanitize_workspace_key, workspace_path_for_root};
