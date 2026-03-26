mod client;
mod events;
mod models;
mod session;
mod supervisor;
mod tooling;

pub use client::{
    ApiKeyAuth, AuthConfig, HttpAuth, OpenHandsClient, OpenHandsError, OpenHandsProbeResult,
    RuntimeEventStream, RuntimeStreamConfig, TransportAuthKind, TransportConfig,
    TransportDiagnostics, TransportTargetKind, WebSocketAuth,
};
pub use events::{
    ActionEventPayload, ActivityKind, ActivitySummary, ConversationErrorEvent,
    ConversationStateMirror, EventCache, KnownEvent, LlmCompletionLogEvent, MessageEventPayload,
    ObservationEventPayload, TerminalExecutionStatus, UnknownEvent,
};
pub use models::{
    AcceptedResponse, AgentConfig, CondenserConfig, ConfirmationPolicy, Conversation,
    ConversationCreateRequest, ConversationRunRequest, ConversationStateUpdatePayload,
    DoctorProbeConfig, EventEnvelope, LLM_SUMMARIZING_CONDENSER_KIND, LlmConfig, McpConfig,
    McpStdioServerConfig, SearchConversationEventsResponse, SendMessageRequest, TextContent,
    ToolConfig, WorkspaceConfig,
};
pub use session::{
    ConversationLaunchProfile, IssueConversationManifest, IssueSessionContext, IssueSessionError,
    IssueSessionObserver, IssueSessionPromptKind, IssueSessionResult, IssueSessionReusePolicy,
    IssueSessionRunner, IssueSessionRunnerConfig, LlmConfigFingerprint, RUNTIME_CONTRACT_VERSION,
    WorkpadComment, WorkpadCommentSource,
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
    "REST client, WebSocket event stream, event cache/state mirror, local server supervisor, repo-local tooling resolution, conservative readiness probes, doctor diagnostics, issue session runner, and protocol error mapping"
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
        assert!(crate_summary().contains("local server supervisor"));
    }
}
