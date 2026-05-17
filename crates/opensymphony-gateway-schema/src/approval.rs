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
    pub requested_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub status: ApprovalStatus,
    pub correlation_id: String,
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
}

/// Action receipt returned after a mutation is accepted or rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub schema_version: SchemaVersion,
    pub action_id: String,
    pub correlation_id: String,
    pub status: ActionReceiptStatus,
    pub reason: Option<String>,
    pub expected_events: Vec<String>,
    pub result: Option<serde_json::Value>,
    pub issued_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionReceiptStatus {
    Accepted,
    Rejected,
    Queued,
    Completed,
}
