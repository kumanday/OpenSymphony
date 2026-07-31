mod error;
mod manager;
mod models;
mod paths;

pub use error::{WorkspaceError, WorkspaceOwnershipConflictDetails};
pub use manager::{WorkspaceManager, compose_terminal_prompt};
pub use models::{
    CheckoutManifest, CheckoutRepository, CleanupConfig, CleanupDecision, CleanupOutcome,
    ConversationManifest, EnsureWorkspaceResult, HookConfig, HookDefinition, HookExecutionRecord,
    HookExecutionStatus, HookKind, InstructionProvenance, IssueContextArtifact, IssueDescriptor,
    IssueLifecycleState, IssueManifest, PromptCaptureDescriptor, PromptCaptureManifest, PromptKind,
    RunDescriptor, RunManifest, RunStatus, SessionContextArtifact, TerminalRuntimeEnvelope,
    WorkspaceHandle, WorkspaceManagerConfig, redact_runtime_diagnostic,
};
pub use paths::{
    checkout_workspace_key, resolve_path_within_root, sanitize_workspace_key,
    workspace_path_for_root,
};
