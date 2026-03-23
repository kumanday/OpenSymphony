use opensymphony_domain::{
    TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot,
};

use crate::error::LinearError;
use crate::graphql::{
    LinearBlockerNode, LinearChildNode, LinearIssueNode, LinearIssueStateNode, LinearLabelNode,
    LinearParentNode, LinearRelationNode, LinearWorkflowState,
};

pub(super) fn normalize_issue(node: LinearIssueNode) -> Result<TrackerIssue, LinearError> {
    Ok(TrackerIssue {
        id: node.id,
        identifier: node.identifier,
        url: node.url,
        title: node.title,
        description: node.description,
        priority: normalize_priority(node.priority)?,
        state: node.state.name,
        labels: normalize_labels(node.labels.nodes),
        parent_id: normalize_parent_id(node.parent),
        blocked_by: normalize_blockers(node.inverse_relations.nodes),
        sub_issues: normalize_sub_issues(node.children.nodes),
        created_at: node.created_at,
        updated_at: node.updated_at,
    })
}

pub(super) fn normalize_issue_state(node: LinearIssueStateNode) -> TrackerIssueStateSnapshot {
    TrackerIssueStateSnapshot {
        id: node.id,
        identifier: node.identifier,
        state: normalize_state(node.state),
        updated_at: node.updated_at,
    }
}

fn normalize_state(state: LinearWorkflowState) -> TrackerIssueState {
    TrackerIssueState {
        id: state.id,
        name: state.name,
        tracker_type: state.kind.clone(),
        kind: TrackerIssueStateKind::from_tracker_type(state.kind),
    }
}

fn normalize_labels(labels: Vec<LinearLabelNode>) -> Vec<String> {
    let mut labels = labels
        .into_iter()
        .map(|label| label.name)
        .collect::<Vec<_>>();
    labels.sort_unstable();
    labels.dedup();
    labels
}

fn normalize_blockers(relations: Vec<LinearRelationNode>) -> Vec<TrackerIssueBlocker> {
    let mut blockers = relations
        .into_iter()
        .filter(|relation| relation.relation_type == "blocks")
        .map(|relation| normalize_blocker(relation.issue))
        .collect::<Vec<_>>();
    blockers.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    blockers.dedup_by(|left, right| left.id == right.id);
    blockers
}

fn normalize_blocker(blocker: LinearBlockerNode) -> TrackerIssueBlocker {
    TrackerIssueBlocker {
        id: blocker.id,
        identifier: blocker.identifier,
        title: blocker.title,
        state: normalize_state(blocker.state),
    }
}

fn normalize_parent_id(parent: Option<LinearParentNode>) -> Option<String> {
    parent.map(|parent| parent.id)
}

fn normalize_sub_issues(children: Vec<LinearChildNode>) -> Vec<TrackerIssueRef> {
    let mut sub_issues = children
        .into_iter()
        .map(|child| TrackerIssueRef {
            id: child.id,
            identifier: child.identifier,
            state: child.state.name,
        })
        .collect::<Vec<_>>();
    sub_issues.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    sub_issues.dedup_by(|left, right| left.id == right.id);
    sub_issues
}

const LINEAR_MAX_PRIORITY: u64 = 4;

fn normalize_priority(priority: f64) -> Result<Option<u8>, LinearError> {
    if !priority.is_finite() || priority < 0.0 {
        return Err(LinearError::InvalidResponse(format!(
            "Linear priority must be a finite non-negative number, got {priority}"
        )));
    }

    let rounded = priority.trunc();
    if (priority - rounded).abs() > f64::EPSILON {
        return Err(LinearError::InvalidResponse(format!(
            "Linear priority must be an integer value, got {priority}"
        )));
    }

    match rounded as u64 {
        0 => Ok(None),
        value if value <= LINEAR_MAX_PRIORITY => Ok(Some(value as u8)),
        value => Err(LinearError::InvalidResponse(format!(
            "Linear priority must be between 0 and {LINEAR_MAX_PRIORITY}, got {value}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use crate::normalize::normalize_priority;

    #[test]
    fn priority_zero_becomes_none() {
        assert_eq!(
            normalize_priority(0.0).expect("priority should normalize"),
            None
        );
    }

    #[test]
    fn fractional_priority_is_rejected() {
        assert!(normalize_priority(1.5).is_err());
    }

    #[test]
    fn linear_priority_is_preserved_for_prompt_consumers() {
        assert_eq!(
            normalize_priority(1.0).expect("priority should normalize"),
            Some(1)
        );
        assert_eq!(
            normalize_priority(4.0).expect("priority should normalize"),
            Some(4)
        );
    }

    #[test]
    fn undocumented_linear_priority_values_are_rejected() {
        assert!(normalize_priority(5.0).is_err());
    }
}
