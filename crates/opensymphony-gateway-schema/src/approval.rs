use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// A live ACP callback. This is deliberately ephemeral: a restart cannot
/// restore an operator decision without the same live RPC responder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorInteraction {
    pub request_id: String,
    pub run_id: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub session_id: String,
    pub generation: u64,
    /// Generated public binding token; the original peer RPC ID stays private
    /// inside the ACP responder. Clients echo this token unchanged.
    pub rpc_id: String,
    pub kind: OperatorInteractionKind,
    pub title: String,
    pub options: Vec<OperatorOption>,
    pub questions: Vec<OperatorQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub requested_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorInteractionKind {
    Permission,
    Question,
    PlanApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorOption {
    pub id: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorQuestion {
    pub id: String,
    pub prompt: String,
    pub options: Vec<OperatorOption>,
    pub allow_multiple: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorQuestionAnswer {
    pub question_id: String,
    pub selected_option_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperatorAnswer {
    Permission {
        option_id: String,
    },
    Question {
        answers: Vec<OperatorQuestionAnswer>,
    },
    Plan {
        accepted: bool,
    },
    Decline,
    Cancel,
}

/// Approval request exposed by the gateway for human-in-the-loop actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub schema_version: SchemaVersion,
    pub approval_id: String,
    pub run_id: String,
    pub issue_id: String,
    pub kind: ApprovalKind,
    pub title: String,
    pub description: String,
    pub proposed_action: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_interaction: Option<OperatorInteraction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ApprovalActor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_context: Option<ApprovalTargetContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_summary: Option<ApprovalRiskSummary>,
    pub requested_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub status: ApprovalStatus,
    pub correlation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_at: Option<DateTime<Utc>>,
}

/// Actor requesting an approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalActor {
    pub actor_id: String,
    pub actor_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Context the approval applies to (file, command, issue, or run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalTargetContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue_identifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// Risk level for an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRiskLevel {
    Low,
    Medium,
    High,
    Unknown,
}

/// Risk summary associated with an approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRiskSummary {
    pub level: ApprovalRiskLevel,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    ToolUse,
    FileWrite,
    CommandExecution,
    PlanPublish,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Expired,
    Cancelled,
    Passed,
    Failed,
}

// NOTE: ActionReceipt and ActionStatus are defined in the action module.
// Use `opensymphony_gateway_schema::action::{ActionReceipt, ActionStatus}`.
