use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::domain::{TrackerIssue, TrackerIssueStateKind};

/// Analysis of the Linear task graph for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinearGraphAnalysis {
    pub project_name: String,
    pub project_id: String,
    pub analyzed_at: DateTime<Utc>,
    pub total_issues: usize,
    pub issues_by_state: BTreeMap<String, usize>,
    pub issues_by_priority: BTreeMap<Option<u8>, usize>,
    pub milestones: Vec<MilestoneSummary>,
    pub blocker_chains: Vec<BlockerChain>,
    pub unblocked_issues: Vec<IssueSnapshot>,
    pub blocked_issues: Vec<IssueSnapshot>,
    pub terminal_issues: Vec<IssueSnapshot>,
    pub active_issues: Vec<IssueSnapshot>,
    pub label_distribution: BTreeMap<String, usize>,
    pub parent_child_relationships: Vec<ParentChildRelationship>,
    pub constraints_summary: String,
}

/// Summary of a Linear milestone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilestoneSummary {
    pub milestone_id: String,
    pub milestone_name: String,
    pub issue_count: usize,
    pub active_issue_count: usize,
    pub completed_issue_count: usize,
    pub canceled_issue_count: usize,
}

/// A chain of blockers for an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockerChain {
    pub issue_id: String,
    pub issue_identifier: String,
    pub issue_title: String,
    pub blockers: Vec<BlockerSnapshot>,
    pub is_resolved: bool,
}

/// Snapshot of a blocker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockerSnapshot {
    pub blocker_id: String,
    pub blocker_identifier: String,
    pub blocker_title: String,
    pub blocker_state: String,
    pub is_terminal: bool,
}

/// Lightweight snapshot of an issue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSnapshot {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub priority: Option<u8>,
}

/// Parent-child relationship between issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentChildRelationship {
    pub parent_id: String,
    pub parent_identifier: String,
    pub children: Vec<ChildRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildRef {
    pub id: String,
    pub identifier: String,
    pub state: String,
}

/// Produces a LinearGraphAnalysis from a set of tracker issues.
pub struct LinearGraphAnalyzer {
    project_name: String,
    project_id: String,
}

impl LinearGraphAnalyzer {
    pub fn new(project_name: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            project_name: project_name.into(),
            project_id: project_id.into(),
        }
    }

    pub fn analyze(&self, issues: &[TrackerIssue]) -> LinearGraphAnalysis {
        let analyzed_at = Utc::now();

        let mut issues_by_state = BTreeMap::new();
        let mut issues_by_priority = BTreeMap::new();
        let mut label_counts: BTreeMap<String, usize> = BTreeMap::new();

        let mut milestones_map: BTreeMap<String, MilestoneSummary> = BTreeMap::new();
        let mut unblocked = Vec::new();
        let mut blocked = Vec::new();
        let mut terminal = Vec::new();
        let mut active = Vec::new();
        let mut parent_map: BTreeMap<String, ParentChildRelationship> = BTreeMap::new();

        for issue in issues {
            let snapshot = IssueSnapshot {
                id: issue.id.clone(),
                identifier: issue.identifier.clone(),
                title: issue.title.clone(),
                state: issue.state.clone(),
                priority: issue.priority,
            };

            // Count by state
            let state_count = issues_by_state.entry(issue.state.clone()).or_insert(0);
            *state_count += 1;

            // Count by priority
            let prio_count = issues_by_priority.entry(issue.priority).or_insert(0);
            *prio_count += 1;

            // Count labels
            for label in &issue.labels {
                let label_count = label_counts.entry(label.clone()).or_insert(0);
                *label_count += 1;
            }

            // Milestone tracking
            if let Some(ref milestone) = issue.project_milestone {
                let ms = milestones_map
                    .entry(milestone.id.clone())
                    .or_insert_with(|| MilestoneSummary {
                        milestone_id: milestone.id.clone(),
                        milestone_name: milestone.name.clone(),
                        issue_count: 0,
                        active_issue_count: 0,
                        completed_issue_count: 0,
                        canceled_issue_count: 0,
                    });
                ms.issue_count += 1;

                let issue_state_kind = TrackerIssueStateKind::from_tracker_type(&issue.state);
                if issue_state_kind.is_terminal() {
                    match issue_state_kind {
                        TrackerIssueStateKind::Completed => ms.completed_issue_count += 1,
                        TrackerIssueStateKind::Canceled => ms.canceled_issue_count += 1,
                        _ => {}
                    }
                } else {
                    ms.active_issue_count += 1;
                }
            }

            // Blocker tracking
            let has_active_blockers = issue.blocked_by.iter().any(|b| !b.is_terminal());
            if has_active_blockers {
                blocked.push(snapshot.clone());
            } else {
                unblocked.push(snapshot.clone());
            }

            // Terminal vs active classification
            let is_terminal = TrackerIssueStateKind::from_tracker_type(&issue.state).is_terminal();
            if is_terminal {
                terminal.push(snapshot.clone());
            } else {
                active.push(snapshot.clone());
            }

            // Parent-child tracking
            if let Some(ref parent) = issue.parent {
                let parent_rel = parent_map.entry(parent.id.clone()).or_insert_with(|| {
                    ParentChildRelationship {
                        parent_id: parent.id.clone(),
                        parent_identifier: parent.identifier.clone(),
                        children: Vec::new(),
                    }
                });
                parent_rel.children.push(ChildRef {
                    id: issue.id.clone(),
                    identifier: issue.identifier.clone(),
                    state: issue.state.clone(),
                });
            }
        }

        // Build blocker chains
        let blocker_chains: Vec<BlockerChain> = issues
            .iter()
            .filter(|i| !i.blocked_by.is_empty())
            .map(|issue| BlockerChain {
                issue_id: issue.id.clone(),
                issue_identifier: issue.identifier.clone(),
                issue_title: issue.title.clone(),
                blockers: issue
                    .blocked_by
                    .iter()
                    .map(|b| BlockerSnapshot {
                        blocker_id: b.id.clone(),
                        blocker_identifier: b.identifier.clone(),
                        blocker_title: b.title.clone(),
                        blocker_state: b.state.clone(),
                        is_terminal: b.is_terminal(),
                    })
                    .collect(),
                is_resolved: issue.blocked_by.iter().all(|b| b.is_terminal()),
            })
            .collect();

        // Build constraints summary
        let constraints_summary =
            Self::build_constraints_summary(&issues_by_state, &blocker_chains, &milestones_map);

        LinearGraphAnalysis {
            project_name: self.project_name.clone(),
            project_id: self.project_id.clone(),
            analyzed_at,
            total_issues: issues.len(),
            issues_by_state,
            issues_by_priority,
            milestones: milestones_map.into_values().collect(),
            blocker_chains,
            unblocked_issues: unblocked,
            blocked_issues: blocked,
            terminal_issues: terminal,
            active_issues: active,
            label_distribution: label_counts,
            parent_child_relationships: parent_map.into_values().collect(),
            constraints_summary,
        }
    }

    fn build_constraints_summary(
        issues_by_state: &BTreeMap<String, usize>,
        blocker_chains: &[BlockerChain],
        milestones: &BTreeMap<String, MilestoneSummary>,
    ) -> String {
        let mut summary = Vec::new();

        let total_active_blockers = blocker_chains.iter().filter(|bc| !bc.is_resolved).count();
        if total_active_blockers > 0 {
            summary.push(format!(
                "{total_active_blockers} issue(s) have unresolved blockers",
            ));
        }

        let total_terminal = issues_by_state
            .iter()
            .filter(|(state, _)| TrackerIssueStateKind::from_tracker_type(state).is_terminal())
            .map(|(_, count)| count)
            .sum::<usize>();
        if total_terminal > 0 {
            summary.push(format!("{total_terminal} terminal issue(s)"));
        }

        let milestone_count = milestones.len();
        if milestone_count > 0 {
            summary.push(format!("{milestone_count} milestone(s) defined"));
        }

        if summary.is_empty() {
            "No active constraints detected".to_string()
        } else {
            summary.join("; ")
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::super::domain::{
        TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerProjectMilestone,
    };
    use super::*;
    use chrono::Utc;

    fn make_issue(
        id: &str,
        identifier: &str,
        title: &str,
        state: &str,
        priority: Option<u8>,
    ) -> TrackerIssue {
        TrackerIssue {
            id: id.to_string(),
            identifier: identifier.to_string(),
            url: format!("https://linear.app/test/issue/{identifier}"),
            title: title.to_string(),
            description: None,
            priority,
            state: state.to_string(),
            labels: vec!["backend".to_string()],
            parent_id: None,
            parent: None,
            project_milestone: Some(TrackerProjectMilestone {
                id: "ms-1".to_string(),
                name: "M1".to_string(),
            }),
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_blocker(
        id: &str,
        identifier: &str,
        title: &str,
        state_name: &str,
        is_terminal: bool,
    ) -> TrackerIssueBlocker {
        TrackerIssueBlocker {
            id: id.to_string(),
            identifier: identifier.to_string(),
            title: title.to_string(),
            state: state_name.to_string(),
            state_kind: Some(if is_terminal {
                TrackerIssueStateKind::Completed
            } else {
                TrackerIssueStateKind::Started
            }),
        }
    }

    #[test]
    fn analyze_counts_issues_by_state() {
        let issues = vec![
            make_issue("1", "COE-1", "Issue 1", "Todo", Some(1)),
            make_issue("2", "COE-2", "Issue 2", "In Progress", Some(2)),
            make_issue("3", "COE-3", "Issue 3", "In Progress", None),
            make_issue("4", "COE-4", "Issue 4", "Completed", Some(1)),
        ];

        let analyzer = LinearGraphAnalyzer::new("TestProject", "proj-1");
        let analysis = analyzer.analyze(&issues);

        assert_eq!(analysis.total_issues, 4);
        assert_eq!(*analysis.issues_by_state.get("Todo").unwrap(), 1);
        assert_eq!(*analysis.issues_by_state.get("In Progress").unwrap(), 2);
        assert_eq!(*analysis.issues_by_state.get("Completed").unwrap(), 1);
    }

    #[test]
    fn analyze_tracks_blocker_chains() {
        let mut issues = vec![make_issue("1", "COE-1", "Blocked Issue", "Todo", Some(1))];
        issues[0].blocked_by = vec![
            make_blocker("b1", "COE-0", "Active Blocker", "In Progress", false),
            make_blocker("b2", "COE-01", "Completed Blocker", "Done", true),
        ];

        let analyzer = LinearGraphAnalyzer::new("TestProject", "proj-1");
        let analysis = analyzer.analyze(&issues);

        assert_eq!(analysis.blocker_chains.len(), 1);
        assert!(!analysis.blocker_chains[0].is_resolved);
        assert_eq!(analysis.blocker_chains[0].blockers.len(), 2);
        assert!(
            analysis
                .blocked_issues
                .iter()
                .any(|i| i.identifier == "COE-1")
        );
    }

    #[test]
    fn analyze_tracks_milestones() {
        let mut issues = vec![
            make_issue("1", "COE-1", "Issue 1", "Todo", Some(1)),
            make_issue("2", "COE-2", "Issue 2", "Completed", Some(2)),
        ];
        issues[0].project_milestone = Some(TrackerProjectMilestone {
            id: "ms-1".to_string(),
            name: "M1".to_string(),
        });
        issues[1].project_milestone = Some(TrackerProjectMilestone {
            id: "ms-2".to_string(),
            name: "M2".to_string(),
        });

        let analyzer = LinearGraphAnalyzer::new("TestProject", "proj-1");
        let analysis = analyzer.analyze(&issues);

        assert_eq!(analysis.milestones.len(), 2);
        let m1 = analysis
            .milestones
            .iter()
            .find(|m| m.milestone_name == "M1")
            .unwrap();
        assert_eq!(m1.issue_count, 1);
        let m2 = analysis
            .milestones
            .iter()
            .find(|m| m.milestone_name == "M2")
            .unwrap();
        assert_eq!(m2.completed_issue_count, 1);
    }

    #[test]
    fn analyze_tracks_parent_child_relationships() {
        let mut issues = vec![
            make_issue("1", "COE-1", "Parent", "Todo", Some(1)),
            make_issue("2", "COE-2", "Child 1", "Completed", Some(2)),
            make_issue("3", "COE-3", "Child 2", "In Progress", None),
        ];

        issues[1].parent_id = Some("1".to_string());
        issues[1].parent = Some(TrackerIssueRef {
            id: "1".to_string(),
            identifier: "COE-1".to_string(),
            title: Some("Parent".to_string()),
            url: None,
            state: "Todo".to_string(),
        });
        issues[2].parent_id = Some("1".to_string());
        issues[2].parent = Some(TrackerIssueRef {
            id: "1".to_string(),
            identifier: "COE-1".to_string(),
            title: Some("Parent".to_string()),
            url: None,
            state: "Todo".to_string(),
        });

        let analyzer = LinearGraphAnalyzer::new("TestProject", "proj-1");
        let analysis = analyzer.analyze(&issues);

        assert_eq!(analysis.parent_child_relationships.len(), 1);
        assert_eq!(analysis.parent_child_relationships[0].children.len(), 2);
    }

    #[test]
    fn analyze_serializes_to_json() {
        let issues = vec![make_issue("1", "COE-1", "Test Issue", "Todo", Some(1))];

        let analyzer = LinearGraphAnalyzer::new("TestProject", "proj-1");
        let analysis = analyzer.analyze(&issues);

        let json = serde_json::to_string(&analysis).expect("should serialize");
        assert!(json.contains("TestProject"));
        assert!(json.contains("COE-1"));

        let deserialized: LinearGraphAnalysis =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized.project_name, analysis.project_name);
        assert_eq!(deserialized.total_issues, analysis.total_issues);
    }

    #[test]
    fn analyze_empty_issueset_returns_zero_counts() {
        let analyzer = LinearGraphAnalyzer::new("EmptyProject", "proj-0");
        let analysis = analyzer.analyze(&[]);

        assert_eq!(analysis.total_issues, 0);
        assert!(analysis.issues_by_state.is_empty());
        assert!(analysis.milestones.is_empty());
        assert!(analysis.blocker_chains.is_empty());
        assert_eq!(
            analysis.constraints_summary,
            "No active constraints detected"
        );
    }

    #[test]
    fn analyze_classifies_terminal_vs_active_issues() {
        let issues = vec![
            make_issue("1", "COE-1", "Active", "In Progress", Some(1)),
            make_issue("2", "COE-2", "Done", "Completed", Some(2)),
            make_issue("3", "COE-3", "Canceled", "Canceled", None),
        ];

        let analyzer = LinearGraphAnalyzer::new("TestProject", "proj-1");
        let analysis = analyzer.analyze(&issues);

        assert_eq!(analysis.active_issues.len(), 1);
        assert_eq!(analysis.terminal_issues.len(), 2);
    }
}
