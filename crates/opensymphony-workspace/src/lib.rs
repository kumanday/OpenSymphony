mod environment;
mod error;
mod manager;
mod models;
mod paths;
mod sensitive_fields;

pub(crate) use sensitive_fields::{normalize_secret_field_name, runtime_field_is_sensitive};

pub use environment::environment_variable_names_equal;
pub(crate) use environment::{has_environment_name_collision, insert_environment_value};
pub use error::{WorkspaceError, WorkspaceOwnershipConflictDetails};
pub use manager::{
    WorkspaceManager, compose_parent_continuation_prompt, compose_parent_prompt,
    compose_terminal_prompt,
};
pub use models::{
    CheckoutManifest, CheckoutRepository, CleanupConfig, CleanupDecision, CleanupIntent,
    CleanupOutcome, CleanupRequest, CleanupTarget, CleanupTerminalOutcome, CleanupTombstone,
    ConversationManifest, EnsureWorkspaceResult, HookConfig, HookDefinition, HookExecutionRecord,
    HookExecutionStatus, HookKind, InstructionProvenance, IssueContextArtifact, IssueDescriptor,
    IssueLifecycleState, IssueManifest, ParentCheckoutRequest, ParentChildCheckoutMap,
    ParentExecutionManifest, ParentExecutionRoot, ParentIntegrationCheckout,
    ParentRetainedCheckout, ParentRuntimeCheckout, ParentRuntimeDescriptor, ParentRuntimeEnvelope,
    PromptCaptureDescriptor, PromptCaptureManifest, PromptKind, RunDescriptor, RunManifest,
    RunStatus, SSH_AUTH_SOCK_ENV, SessionContextArtifact, TerminalRuntimeEnvelope, WorkspaceHandle,
    WorkspaceManagerConfig, checkout_credential_environment_variables, redact_runtime_diagnostic,
};
pub use paths::{
    checkout_workspace_key, parent_workspace_key, resolve_path_within_root, sanitize_workspace_key,
    workspace_path_for_root,
};

#[cfg(unix)]
pub(crate) use manager::ProcessGroupGuard;
pub(crate) use manager::{configure_process_group, terminate_process_tree};
