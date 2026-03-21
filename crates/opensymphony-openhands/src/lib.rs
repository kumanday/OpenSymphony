use async_trait::async_trait;
use opensymphony_domain::{Issue, WorkerOutcome};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IssueSessionError {
    #[error("runtime transport error: {0}")]
    Transport(String),
    #[error("runtime protocol error: {0}")]
    Protocol(String),
    #[error("runtime timeout: {0}")]
    Timeout(String),
    #[error("runtime execution error: {0}")]
    Execution(String),
}

#[async_trait]
pub trait IssueSessionRunner: Send + Sync {
    async fn run_issue(
        &self,
        request: IssueRunRequest,
    ) -> Result<IssueRunResult, IssueSessionError>;
}
