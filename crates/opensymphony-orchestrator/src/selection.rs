use std::collections::HashSet;

use opensymphony_domain::TrackerIssue;

pub fn issue_blocked_by_non_terminal_blockers(issue: &TrackerIssue) -> bool {
    issue
        .blocked_by
        .iter()
        .any(|blocker| !blocker.is_terminal())
}

pub fn parent_issue_blocked_by_incomplete_children(
    issue: &TrackerIssue,
    terminal_states: &HashSet<String>,
) -> bool {
    !issue.sub_issues.is_empty()
        && issue
            .sub_issues
            .iter()
            .any(|sub_issue| !sub_issue.is_terminal(terminal_states))
}

pub fn should_dispatch_issue(issue: &TrackerIssue, terminal_states: &HashSet<String>) -> bool {
    !issue_blocked_by_non_terminal_blockers(issue)
        && !parent_issue_blocked_by_incomplete_children(issue, terminal_states)
}

pub fn filter_issues_for_dispatch<I>(
    issues: I,
    terminal_states: &HashSet<String>,
) -> Vec<TrackerIssue>
where
    I: IntoIterator<Item = TrackerIssue>,
{
    let mut filtered = issues
        .into_iter()
        .filter(|issue| should_dispatch_issue(issue, terminal_states))
        .collect::<Vec<_>>();
    sort_issues_for_dispatch(&mut filtered);
    filtered
}

pub fn sort_issues_for_dispatch(issues: &mut [TrackerIssue]) {
    issues.sort_by(|left, right| {
        priority_rank(left)
            .cmp(&priority_rank(right))
            .then_with(|| left.sub_issues.len().cmp(&right.sub_issues.len()))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.identifier.cmp(&right.identifier))
    });
}

fn priority_rank(issue: &TrackerIssue) -> u8 {
    issue.priority.unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{DateTime, Utc};
    use opensymphony_domain::{
        TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState,
        TrackerIssueStateKind,
    };

    use super::{
        filter_issues_for_dispatch, issue_blocked_by_non_terminal_blockers,
        parent_issue_blocked_by_incomplete_children, should_dispatch_issue,
        sort_issues_for_dispatch,
    };

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("timestamp should parse")
            .with_timezone(&Utc)
    }

    fn terminal_states() -> HashSet<String> {
        HashSet::from([String::from("Done"), String::from("Canceled")])
    }

    fn state(name: &str, kind: TrackerIssueStateKind) -> TrackerIssueState {
        TrackerIssueState {
            id: format!("state-{}", name.to_ascii_lowercase().replace(' ', "-")),
            name: name.to_string(),
            tracker_type: match &kind {
                TrackerIssueStateKind::Completed => "completed",
                TrackerIssueStateKind::Canceled => "canceled",
                TrackerIssueStateKind::Started => "started",
                TrackerIssueStateKind::Unstarted => "unstarted",
                TrackerIssueStateKind::Backlog => "backlog",
                TrackerIssueStateKind::Triage => "triage",
                TrackerIssueStateKind::Unknown(_) => "unknown",
            }
            .to_string(),
            kind,
        }
    }

    fn blocker(identifier: &str, state: TrackerIssueState) -> TrackerIssueBlocker {
        TrackerIssueBlocker {
            id: format!("issue-{}", identifier.to_ascii_lowercase()),
            identifier: identifier.to_string(),
            title: format!("Issue {identifier}"),
            state,
        }
    }

    fn child(identifier: &str, state: &str) -> TrackerIssueRef {
        TrackerIssueRef {
            id: format!("issue-{}", identifier.to_ascii_lowercase()),
            identifier: identifier.to_string(),
            state: state.to_string(),
        }
    }

    fn issue(
        identifier: &str,
        priority: Option<u8>,
        created_at: &str,
        blocked_by: Vec<TrackerIssueBlocker>,
        sub_issues: Vec<TrackerIssueRef>,
    ) -> TrackerIssue {
        TrackerIssue {
            id: format!("issue-{}", identifier.to_ascii_lowercase()),
            identifier: identifier.to_string(),
            url: format!("https://linear.app/example/{identifier}"),
            title: format!("Issue {identifier}"),
            description: None,
            priority,
            state: "In Progress".to_string(),
            labels: Vec::new(),
            parent_id: None,
            blocked_by,
            sub_issues,
            created_at: ts(created_at),
            updated_at: ts(created_at),
        }
    }

    #[test]
    fn parent_issue_is_blocked_when_any_child_is_non_terminal() {
        let issue = issue(
            "COE-277",
            Some(1),
            "2026-03-22T00:00:00Z",
            Vec::new(),
            vec![child("COE-278", "In Progress"), child("COE-279", "Done")],
        );

        assert!(parent_issue_blocked_by_incomplete_children(
            &issue,
            &terminal_states()
        ));
    }

    #[test]
    fn parent_issue_is_ready_when_all_children_are_terminal() {
        let issue = issue(
            "COE-277",
            Some(1),
            "2026-03-22T00:00:00Z",
            Vec::new(),
            vec![child("COE-278", "Done"), child("COE-279", "Canceled")],
        );

        assert!(!parent_issue_blocked_by_incomplete_children(
            &issue,
            &terminal_states()
        ));
    }

    #[test]
    fn blocker_check_composes_with_hierarchy_check() {
        let issue = issue(
            "COE-277",
            Some(1),
            "2026-03-22T00:00:00Z",
            vec![blocker(
                "COE-260",
                state("In Progress", TrackerIssueStateKind::Started),
            )],
            vec![child("COE-278", "Done")],
        );

        assert!(issue_blocked_by_non_terminal_blockers(&issue));
        assert!(!should_dispatch_issue(&issue, &terminal_states()));
    }

    #[test]
    fn sort_prefers_leaf_issues_before_parents_when_priorities_match() {
        let mut issues = vec![
            issue(
                "COE-277",
                Some(1),
                "2026-03-20T00:00:00Z",
                Vec::new(),
                vec![child("COE-278", "Done")],
            ),
            issue(
                "COE-278",
                Some(1),
                "2026-03-21T00:00:00Z",
                Vec::new(),
                Vec::new(),
            ),
        ];

        sort_issues_for_dispatch(&mut issues);

        assert_eq!(
            issues
                .iter()
                .map(|issue| issue.identifier.as_str())
                .collect::<Vec<_>>(),
            vec!["COE-278", "COE-277"]
        );
    }

    #[test]
    fn filter_skips_parent_until_children_finish() {
        let issues = vec![
            issue(
                "COE-277",
                Some(1),
                "2026-03-20T00:00:00Z",
                Vec::new(),
                vec![child("COE-278", "In Progress")],
            ),
            issue(
                "COE-278",
                Some(1),
                "2026-03-21T00:00:00Z",
                Vec::new(),
                Vec::new(),
            ),
        ];

        let filtered = filter_issues_for_dispatch(issues, &terminal_states());

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].identifier, "COE-278");
    }

    #[test]
    fn nested_hierarchy_dispatches_only_the_leaf_issue() {
        let issues = vec![
            issue(
                "COE-P1",
                Some(1),
                "2026-03-20T00:00:00Z",
                Vec::new(),
                vec![child("COE-S1", "In Progress")],
            ),
            issue(
                "COE-S1",
                Some(1),
                "2026-03-21T00:00:00Z",
                Vec::new(),
                vec![child("COE-SS1", "In Progress")],
            ),
            issue(
                "COE-SS1",
                Some(1),
                "2026-03-22T00:00:00Z",
                Vec::new(),
                Vec::new(),
            ),
        ];

        let filtered = filter_issues_for_dispatch(issues, &terminal_states());

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].identifier, "COE-SS1");
    }

    #[test]
    fn adding_a_new_child_reblocks_the_parent_on_the_next_snapshot() {
        let terminal_states = terminal_states();
        let mut parent = issue(
            "COE-277",
            Some(1),
            "2026-03-20T00:00:00Z",
            Vec::new(),
            Vec::new(),
        );

        assert!(should_dispatch_issue(&parent, &terminal_states));

        parent.sub_issues.push(child("COE-278", "Todo"));

        assert!(!should_dispatch_issue(&parent, &terminal_states));
    }
}
