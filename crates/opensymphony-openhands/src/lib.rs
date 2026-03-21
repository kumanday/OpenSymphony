//! Direct Rust integration with the OpenHands SDK agent-server.

mod cache;
mod client;
mod config;
mod error;
mod session;
mod stream;
mod supervisor;
mod wire;

pub use cache::EventCache;
pub use client::{OpenHandsClient, RunConversationResponse};
pub use config::{
    ConversationConfig, HttpAuth, LocalServerConfig, OpenHandsConfig, TransportConfig,
    WebSocketAuthMode, WebSocketConfig,
};
pub use error::{OpenHandsError, Result};
pub use session::{IssueSessionRequest, IssueSessionRunner};
pub use stream::{AttachedConversation, ConversationStateMirror};
pub use supervisor::{LocalAgentServerSupervisor, ServerStatus};
pub use wire::{
    AgentConfig, ConfirmationPolicy, ContentBlock, ConversationInfo, CreateConversationRequest,
    EventPage, LlmConfig, OpenHandsWorkspace, RemoteExecutionStatus, RuntimeEventEnvelope,
    RuntimeEventPayload, SendMessageRequest, ServerInfo, ToolConfig,
};
