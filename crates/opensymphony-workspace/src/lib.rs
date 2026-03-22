mod error;
mod manager;
mod models;
mod paths;

pub use error::{WorkspaceError, WorkspaceOwnershipConflictDetails};
pub use manager::WorkspaceManager;
pub use models::{
    CleanupConfig, CleanupDecision, CleanupOutcome, EnsureWorkspaceResult, HookConfig,
    HookDefinition, HookExecutionRecord, HookExecutionStatus, HookKind, IssueDescriptor,
    IssueLifecycleState, IssueManifest, RunDescriptor, RunManifest, RunStatus, WorkspaceHandle,
    WorkspaceManagerConfig,
};
pub use paths::{resolve_path_within_root, sanitize_workspace_key, workspace_path_for_root};
