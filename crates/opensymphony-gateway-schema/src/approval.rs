use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

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
