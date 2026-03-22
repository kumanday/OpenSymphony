use serde::{Deserialize, Serialize};

use crate::{IssueId, IssueIdentifier, TimestampMs, TrackerStateId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStateCategory {
    Active,
    NonActive,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueState {
    pub id: Option<TrackerStateId>,
    pub name: String,
    pub category: IssueStateCategory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerRef {
    pub id: Option<IssueId>,
    pub identifier: Option<IssueIdentifier>,
    pub state: Option<String>,
    pub created_at: Option<TimestampMs>,
    pub updated_at: Option<TimestampMs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedIssue {
    pub id: IssueId,
    pub identifier: IssueIdentifier,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub state: IssueState,
    pub branch_name: Option<String>,
    pub url: Option<String>,
    pub labels: Vec<String>,
    pub blocked_by: Vec<BlockerRef>,
    pub created_at: Option<TimestampMs>,
    pub updated_at: Option<TimestampMs>,
}
