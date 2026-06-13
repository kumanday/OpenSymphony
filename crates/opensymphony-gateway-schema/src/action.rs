use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::envelope::EntityKind;
use super::version::SchemaVersion;

/// Generic action dispatch payload accepted by
/// `POST /api/v1/actions/dispatch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDispatch {
    pub schema_version: SchemaVersion,
    pub correlation_id: String,
    pub action_kind: ActionKind,
    pub target_entity: ActionTarget,
    pub payload: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
}

/// Action receipt returned by the gateway after dispatch validation.
///
/// Every action—whether accepted or rejected—receives a receipt with a
/// stable `action_id` so callers can correlate with follow-up events in
/// the event stream via `correlation_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub schema_version: SchemaVersion,
    pub action_id: String,
    pub correlation_id: String,
    pub status: ActionStatus,
    pub reason: Option<String>,
    /// Timestamp when the receipt was issued (ISO 8601 / RFC 3339).
    pub issued_at: String,
    /// Hosted-mode permission check placeholder.
    /// `None` means local (no permission check); `Some` means the
    /// permission layer was consulted and this is the result.
    pub permission: Option<PermissionResult>,
    /// Hints about expected follow-up events for this action.
    pub expected_followup: Vec<ExpectedFollowup>,
}

/// Status of an action after gateway validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Accepted,
    Rejected,
}

/// Result of a hosted permission check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionResult {
    pub allowed: bool,
    pub required_role: String,
    pub evaluated: bool,
}

/// Hint for follow-up events the client should expect after an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedFollowup {
    /// Expect an orchestrator state transition event.
    StateTransition,
    /// Expect a run lifecycle event (started, completed, failed, etc.).
    RunLifecycle,
    /// Expect an action completion or failure event.
    ActionCompletion,
    /// Expect a journal update (comment, metadata, etc.).
    JournalUpdate,
    /// Expect a task graph update event (milestone/issue/sub-issue/relation
    /// mutations plus evidence notes).
    TaskGraphUpdate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Retry,
    Cancel,
    Pause,
    Resume,
    Rehydrate,
    Comment,
    TransitionIssue,
    CreateFollowup,
    ApprovalDecision,
    PublishPlan,
    /// Create or update a Linear project milestone.
    TaskGraphMilestone,
    /// Create or update a Linear issue (including sub-issue creation).
    TaskGraphIssue,
    /// Update an existing Linear sub-issue.
    TaskGraphSubIssue,
    /// Create or replace dependency/blocker/related relations between two
    /// Linear issues.
    TaskGraphRelation,
    /// Append an evidence comment/note on a Linear issue.
    TaskGraphEvidence,
}

impl std::fmt::Display for ActionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ActionKind::Retry => "retry",
            ActionKind::Cancel => "cancel",
            ActionKind::Pause => "pause",
            ActionKind::Resume => "resume",
            ActionKind::Rehydrate => "rehydrate",
            ActionKind::Comment => "comment",
            ActionKind::TransitionIssue => "transition_issue",
            ActionKind::CreateFollowup => "create_followup",
            ActionKind::ApprovalDecision => "approval_decision",
            ActionKind::PublishPlan => "publish_plan",
            ActionKind::TaskGraphMilestone => "task_graph_milestone",
            ActionKind::TaskGraphIssue => "task_graph_issue",
            ActionKind::TaskGraphSubIssue => "task_graph_sub_issue",
            ActionKind::TaskGraphRelation => "task_graph_relation",
            ActionKind::TaskGraphEvidence => "task_graph_evidence",
        };
        f.write_str(s)
    }
}

impl ActionKind {
    /// Return expected follow-up event types for this action kind.
    pub fn expected_followups(&self) -> Vec<ExpectedFollowup> {
        match self {
            ActionKind::Retry => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::RunLifecycle,
                ExpectedFollowup::StateTransition,
            ],
            ActionKind::Cancel => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::RunLifecycle,
            ],
            ActionKind::Pause => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::StateTransition,
            ],
            ActionKind::Resume => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::StateTransition,
                ExpectedFollowup::RunLifecycle,
            ],
            ActionKind::Rehydrate => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::RunLifecycle,
                ExpectedFollowup::StateTransition,
            ],
            ActionKind::Comment => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::JournalUpdate,
            ],
            ActionKind::TransitionIssue => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::StateTransition,
            ],
            ActionKind::CreateFollowup => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::JournalUpdate,
            ],
            ActionKind::ApprovalDecision => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::StateTransition,
            ],
            ActionKind::PublishPlan => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::JournalUpdate,
            ],
            ActionKind::TaskGraphMilestone => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::TaskGraphUpdate,
            ],
            ActionKind::TaskGraphIssue => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::TaskGraphUpdate,
            ],
            ActionKind::TaskGraphSubIssue => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::TaskGraphUpdate,
            ],
            ActionKind::TaskGraphRelation => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::TaskGraphUpdate,
            ],
            ActionKind::TaskGraphEvidence => vec![
                ExpectedFollowup::ActionCompletion,
                ExpectedFollowup::TaskGraphUpdate,
                ExpectedFollowup::JournalUpdate,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTarget {
    pub entity_kind: EntityKind,
    pub entity_id: String,
}

impl ActionReceipt {
    /// Create an accepted action receipt.
    pub fn accepted(
        action_id: impl Into<String>,
        correlation_id: impl Into<String>,
        action_kind: ActionKind,
    ) -> Self {
        Self {
            schema_version: SchemaVersion::default(),
            action_id: action_id.into(),
            correlation_id: correlation_id.into(),
            status: ActionStatus::Accepted,
            reason: None,
            issued_at: Utc::now().to_rfc3339(),
            permission: None,
            expected_followup: action_kind.expected_followups(),
        }
    }

    /// Create a rejected action receipt with a reason.
    pub fn rejected(
        action_id: impl Into<String>,
        correlation_id: impl Into<String>,
        action_kind: ActionKind,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: SchemaVersion::default(),
            action_id: action_id.into(),
            correlation_id: correlation_id.into(),
            status: ActionStatus::Rejected,
            reason: Some(reason.into()),
            issued_at: Utc::now().to_rfc3339(),
            permission: None,
            expected_followup: action_kind.expected_followups(),
        }
    }

    /// Create a receipt with a permission check result.
    pub fn with_permission(mut self, permission: PermissionResult) -> Self {
        self.permission = Some(permission);
        self
    }
}

impl PermissionResult {
    /// Local-mode placeholder: no permission check required.
    pub fn local() -> Self {
        Self {
            allowed: true,
            required_role: "local".into(),
            evaluated: false,
        }
    }

    /// Hosted-mode permission check: explicitly evaluated.
    pub fn evaluated(allowed: bool, required_role: impl Into<String>) -> Self {
        Self {
            allowed,
            required_role: required_role.into(),
            evaluated: true,
        }
    }
}
