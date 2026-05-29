//! Minimal domain types needed for Linear graph analysis.
//!
//! These mirror the corresponding types in `opensymphony_domain` so that
//! `opensymphony-planning` can be built as a standalone workspace member
//! without introducing a circular dependency on the root `opensymphony` crate.
//!
//! When compiled via `#[path = ...]` inside the root crate, the real domain
//! types are used instead (see `linear_graph.rs`).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Mirrors `opensymphony_domain::TrackerIssue`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssue {
    pub id: String,
    pub identifier: String,
    #[serde(default)]
    pub url: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub state: String,
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TrackerIssueRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_milestone: Option<TrackerProjectMilestone>,
    pub blocked_by: Vec<TrackerIssueBlocker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_issues: Vec<TrackerIssueRef>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssueRef {
    pub id: String,
    pub identifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerProjectMilestone {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssueBlocker {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_kind: Option<TrackerIssueStateKind>,
}

impl TrackerIssueBlocker {
    pub fn is_terminal(&self) -> bool {
        if let Some(ref kind) = self.state_kind {
            return kind.is_terminal();
        }
        TrackerIssueStateKind::from_tracker_type(&self.state).is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerIssueStateKind {
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    Triage,
    Unknown(String),
}

impl TrackerIssueStateKind {
    pub fn from_tracker_type(value: impl AsRef<str>) -> Self {
        match value.as_ref().trim().to_ascii_lowercase().as_str() {
            "backlog" => Self::Backlog,
            "unstarted" => Self::Unstarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            "canceled" => Self::Canceled,
            "triage" | "triaged" => Self::Triage,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Canceled)
    }
}
