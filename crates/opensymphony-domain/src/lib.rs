//! Shared OpenSymphony domain types that stay independent from runtime transports.

use serde::{Deserialize, Serialize};

/// Tracker-owned issue identity normalized for runtime consumers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IssueRef {
    /// Stable tracker-side issue ID.
    pub issue_id: String,
    /// Human-facing identifier such as `COE-253`.
    pub identifier: String,
    /// Current issue title.
    pub title: String,
}

/// Prompt variants used across fresh and continuation turns.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PromptKind {
    /// Full workflow prompt for a brand-new or reset conversation.
    Fresh,
    /// Continuation guidance for a reused conversation.
    Continuation,
}

/// The normalized execution state the orchestrator cares about.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    /// No background run is active yet.
    Idle,
    /// The conversation is currently executing.
    Running,
    /// The conversation reached a successful terminal state.
    Success,
    /// The conversation reached an error terminal state.
    Error,
    /// The conversation was cancelled.
    Cancelled,
    /// A future status not yet modeled by this crate.
    Unknown,
}

impl ExecutionStatus {
    /// Returns `true` when the status is terminal.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Success | Self::Error | Self::Cancelled)
    }
}

/// Prompt pair carried into the session runner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PromptSet {
    /// Full first-turn prompt.
    pub full_prompt: String,
    /// Continuation guidance prompt.
    pub continuation_prompt: String,
}

/// Stable summary returned by the OpenHands issue session runner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionOutcome {
    /// Conversation used by the attempt.
    pub conversation_id: String,
    /// Prompt variant used for the turn.
    pub prompt_kind: PromptKind,
    /// Final execution status seen by the runtime adapter.
    pub execution_status: ExecutionStatus,
    /// Number of cached runtime events after reconcile.
    pub event_count: usize,
}
