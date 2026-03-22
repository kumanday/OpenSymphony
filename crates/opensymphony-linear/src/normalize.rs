use opensymphony_domain::{
    TrackerIssue, TrackerIssueBlocker, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot,
};

use crate::error::LinearError;
use crate::graphql::{
    LinearBlockerNode, LinearIssueNode, LinearIssueStateNode, LinearLabelNode, LinearRelationNode,
    LinearWorkflowState,
};

pub(super) fn normalize_issue(node: LinearIssueNode) -> Result<TrackerIssue, LinearError> {
    Ok(TrackerIssue {
        id: node.id,
        identifier: node.identifier,
        title: node.title,
        description: node.description,
        priority: normalize_priority(node.priority)?,
        state: normalize_state(node.state),
        labels: normalize_labels(node.labels.nodes),
        blocked_by: normalize_blockers(node.inverse_relations.nodes),
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
        value if value <= u8::MAX as u64 => Ok(Some(value as u8)),
        value => Err(LinearError::InvalidResponse(format!(
            "Linear priority exceeds u8 range: {value}"
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
}
