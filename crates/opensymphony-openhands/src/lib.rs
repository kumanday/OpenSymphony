use async_trait::async_trait;
use opensymphony_domain::{Issue, WorkerOutcome};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PromptMode {
    Fresh,
    Continuation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueRunRequest {
    pub issue: Issue,
    pub attempt: u32,
    pub workspace_path: PathBuf,
    pub prompt_mode: PromptMode,
    pub prompt: String,
    pub max_turns: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueRunResult {
    pub outcome: WorkerOutcome,
    pub conversation_id: String,
    pub prompt_mode: PromptMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueRunProgress {
    pub issue_id: String,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IssueSessionError {
    #[error("runtime transport error: {detail}")]
    Transport {
        detail: String,
        conversation_id: Option<String>,
    },
    #[error("runtime protocol error: {detail}")]
    Protocol {
        detail: String,
        conversation_id: Option<String>,
    },
    #[error("runtime timeout: {detail}")]
    Timeout {
        detail: String,
        conversation_id: Option<String>,
    },
    #[error("runtime execution error: {detail}")]
    Execution {
        detail: String,
        conversation_id: Option<String>,
    },
}

impl IssueSessionError {
    pub fn transport(detail: impl Into<String>) -> Self {
        Self::Transport {
            detail: detail.into(),
            conversation_id: None,
        }
    }

    pub fn protocol(detail: impl Into<String>) -> Self {
        Self::Protocol {
            detail: detail.into(),
            conversation_id: None,
        }
    }

    pub fn timeout(detail: impl Into<String>) -> Self {
        Self::Timeout {
            detail: detail.into(),
            conversation_id: None,
        }
    }

    pub fn execution(detail: impl Into<String>) -> Self {
        Self::Execution {
            detail: detail.into(),
            conversation_id: None,
        }
    }

    pub fn with_conversation_id(mut self, conversation_id: impl Into<String>) -> Self {
        let conversation_id = Some(conversation_id.into());
        match &mut self {
            Self::Transport {
                conversation_id: slot,
                ..
            }
            | Self::Protocol {
                conversation_id: slot,
                ..
            }
            | Self::Timeout {
                conversation_id: slot,
                ..
            }
            | Self::Execution {
                conversation_id: slot,
                ..
            } => *slot = conversation_id,
        }
        self
    }

    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::Transport {
                conversation_id, ..
            }
            | Self::Protocol {
                conversation_id, ..
            }
            | Self::Timeout {
                conversation_id, ..
            }
            | Self::Execution {
                conversation_id, ..
            } => conversation_id.as_deref(),
        }
    }
}

#[async_trait]
pub trait IssueSessionRunner: Send + Sync {
    async fn run_issue(
        &self,
        request: IssueRunRequest,
        progress_tx: mpsc::UnboundedSender<IssueRunProgress>,
    ) -> Result<IssueRunResult, IssueSessionError>;
}
