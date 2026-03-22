mod client;
mod events;
mod models;
mod supervisor;
mod tooling;

pub use client::{
    ApiKeyAuth, AuthConfig, HttpAuth, OpenHandsClient, OpenHandsError, OpenHandsProbeResult,
    RuntimeEventStream, RuntimeStreamConfig, TransportConfig, WebSocketAuth,
};
pub use events::{
    ConversationErrorEvent, ConversationStateMirror, EventCache, KnownEvent, LlmCompletionLogEvent,
    TerminalExecutionStatus, UnknownEvent,
};
pub use models::{
    AcceptedResponse, AgentConfig, ConfirmationPolicy, Conversation, ConversationCreateRequest,
    ConversationRunRequest, ConversationStateUpdatePayload, DoctorProbeConfig, EventEnvelope,
    LlmConfig, SearchConversationEventsResponse, SendMessageRequest, TextContent, WorkspaceConfig,
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
    use super::{crate_summary, CRATE_NAME};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-openhands");
        assert!(crate_summary().contains("local server supervisor"));
    }
}
