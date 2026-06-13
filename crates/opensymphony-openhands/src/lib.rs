mod client;
mod conversation_store;
mod events;
mod models;
mod normalization;
mod runtime_mirror;
mod session;
mod supervisor;
mod tooling;

pub use client::{
    ApiKeyAuth, AuthConfig, HttpAuth, OpenHandsClient, OpenHandsError, OpenHandsProbeResult,
    RuntimeEventStream, RuntimeStreamConfig, TransportAuthKind, TransportConfig,
    TransportDiagnostics, TransportTargetKind, WebSocketAuth,
};
pub use conversation_store::{
    ConversationMoveOutcome, ConversationStoreError, ConversationStoreKind, LocatedConversation,
    OPENHANDS_CONVERSATIONS_PATH_ENV, OpenHandsConversationStorePaths,
};
pub use events::{
    ActionEventPayload, ActivityKind, ActivitySummary, ConversationErrorEvent,
    ConversationStateMirror, EventCache, KnownEvent, LlmCompletionLogEvent, MessageEventPayload,
    ObservationEventPayload, TerminalExecutionStatus, UnknownEvent,
};
pub use models::{
    AcceptedResponse, AgentConfig, CondenserConfig, ConfirmationPolicy, Conversation,
    ConversationCreateRequest, ConversationRunRequest, ConversationStateUpdatePayload,
    DoctorProbeConfig, EventEnvelope, LLM_SUMMARIZING_CONDENSER_KIND, LlmConfig,
    SearchConversationEventsResponse, SendMessageRequest, TextContent, ToolConfig, WorkspaceConfig,
};
pub use normalization::{
    NormalizationContext, NormalizationError, NormalizedEvent, UNKNOWN_RAW_REF_PREFIX,
    normalize_action, normalize_conversation_error, normalize_event, normalize_llm_completion,
    normalize_message, normalize_observation, normalize_state_update, normalize_unknown,
};
pub use runtime_mirror::{
    MirrorConfig, NO_EVENT_CURSOR_MARKER, RuntimeMirror, TERMINAL_CURSOR_MARKER,
};
pub use session::{
    ConversationLaunchProfile, IssueConversationManifest, IssueSessionContext, IssueSessionError,
    IssueSessionObserver, IssueSessionPromptKind, IssueSessionResult, IssueSessionReusePolicy,
    IssueSessionRunner, IssueSessionRunnerConfig, LlmConfigFingerprint, MemoryWorkerAccess,
    RUNTIME_CONTRACT_VERSION, RehydrationOptions, RehydrationResult, WorkpadComment,
    WorkpadCommentSource,
};
pub use supervisor::{
    ExternalServerConfig, LaunchOwnership, LocalServerSupervisor, ProbeConfig, ServerMode,
    ServerState, ServerStatus, SupervisedServerConfig, SupervisorConfig, SupervisorError,
};
pub use tooling::{
    LocalServerTooling, LocalToolingError, LocalToolingLayout, PinStatus, ResolvedLaunch,
    ToolingMetadata,
};

pub const CRATE_NAME: &str = "opensymphony-openhands";

pub fn crate_summary() -> &'static str {
    "REST client, WebSocket event stream, event cache/state mirror, OpenHands → OpenSymphony event normalization, runtime state mirror with progress-based idle detection, local server supervisor, repo-local tooling resolution, conservative readiness probes, doctor diagnostics, issue session runner, and protocol error mapping"
}

pub fn placeholder_summary() -> &'static str {
    crate_summary()
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, crate_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-openhands");
        assert!(crate_summary().contains("event normalization"));
        assert!(crate_summary().contains("runtime state mirror"));
    }
}
