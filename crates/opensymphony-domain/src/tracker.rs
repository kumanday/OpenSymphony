use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssue {
    pub id: String,
    pub identifier: String,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<u8>,
    pub state: String,
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub blocked_by: Vec<TrackerIssueBlocker>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sub_issues: Vec<TrackerIssueRef>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssueStateSnapshot {
    pub id: String,
    pub identifier: String,
    pub state: TrackerIssueState,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssueState {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub tracker_type: String,
    pub kind: TrackerIssueStateKind,
}

impl TrackerIssueState {
    pub fn is_terminal(&self) -> bool {
        self.kind.is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssueBlocker {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: TrackerIssueState,
}

impl TrackerIssueBlocker {
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackerIssueRef {
    pub id: String,
    pub identifier: String,
    pub state: String,
}

impl TrackerIssueRef {
    pub fn is_terminal(&self, terminal_states: &HashSet<String>) -> bool {
        let state = self.state.trim();
        terminal_states
            .iter()
            .any(|terminal_state| terminal_state.trim().eq_ignore_ascii_case(state))
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackerErrorCategory {
    Auth,
    RateLimited,
    Transport,
    Timeout,
    InvalidResponse,
    NotFound,
    InvalidStateTransition,
    PermissionDenied,
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{TrackerErrorCategory, TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind};

    #[test]
    fn tracker_state_kind_maps_known_linear_types() {
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("started"),
            TrackerIssueStateKind::Started
        );
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("completed"),
            TrackerIssueStateKind::Completed
        );
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("triaged"),
            TrackerIssueStateKind::Triage
        );
    }

    #[test]
    fn tracker_state_kind_preserves_unknown_values() {
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("custom-state"),
            TrackerIssueStateKind::Unknown("custom-state".to_string())
        );
    }

    #[test]
    fn tracker_state_helpers_report_terminal_only() {
        let non_terminal = TrackerIssueState {
            id: "state-started".to_string(),
            name: "In Progress".to_string(),
            tracker_type: "started".to_string(),
            kind: TrackerIssueStateKind::Started,
        };
        let terminal = TrackerIssueState {
            id: "state-done".to_string(),
            name: "Done".to_string(),
            tracker_type: "completed".to_string(),
            kind: TrackerIssueStateKind::Completed,
        };

        assert!(!non_terminal.is_terminal());
        assert!(terminal.is_terminal());
    }

    #[test]
    fn tracker_state_serialization_preserves_raw_tracker_type() {
        let state = TrackerIssueState {
            id: "state-triaged".to_string(),
            name: "Triage".to_string(),
            tracker_type: "triaged".to_string(),
            kind: TrackerIssueStateKind::from_tracker_type("triaged"),
        };

        let json = serde_json::to_value(&state).expect("state should serialize");

        assert_eq!(json["type"], serde_json::json!("triaged"));
        assert_eq!(json["kind"], serde_json::json!("triage"));
    }

    #[test]
    fn tracker_error_category_variants_remain_stable() {
        let categories = [
            TrackerErrorCategory::Auth,
            TrackerErrorCategory::RateLimited,
            TrackerErrorCategory::Transport,
            TrackerErrorCategory::Timeout,
            TrackerErrorCategory::InvalidResponse,
            TrackerErrorCategory::NotFound,
            TrackerErrorCategory::InvalidStateTransition,
            TrackerErrorCategory::PermissionDenied,
        ];

        assert_eq!(categories.len(), 8);
    }

    #[test]
    fn tracker_issue_ref_matches_terminal_states_case_insensitively() {
        let issue = TrackerIssueRef {
            id: "issue-1".to_string(),
            identifier: "COE-1".to_string(),
            state: "done".to_string(),
        };
        let terminal_states = HashSet::from([String::from("Done"), String::from("Canceled")]);

        assert!(issue.is_terminal(&terminal_states));
    }
}
