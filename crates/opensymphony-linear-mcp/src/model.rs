use opensymphony_domain::TrackerIssueStateKind;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TeamSummary {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkflowStateSummary {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub tracker_type: String,
    pub kind: TrackerIssueStateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueBlockerSnapshot {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: WorkflowStateSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueSnapshot {
    pub id: String,
    pub identifier: String,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub state: WorkflowStateSummary,
    pub labels: Vec<String>,
    pub blocked_by: Vec<IssueBlockerSnapshot>,
    pub team: TeamSummary,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommentSnapshot {
    pub id: String,
    pub body: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttachmentSnapshot {
    pub id: String,
    pub title: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}
