//! Dependency-graph builder for in-memory planning artefacts.
//!
//! The builder consumes a [`PlanArtifacts`] and emits a
//! [`DependencyGraph`] that downstream consumers (planning workspace UI,
//! Linear draft preview) can render directly. The output is deterministic:
//!
//! - nodes sort by `(milestone, kind, id)` so the milestone tree renders
//!   in a stable order across runs;
//! - edges sort by `(milestone, from, relation, to)` so blockers stack
//!   consistently;
//! - parallelizable waves are already produced in topological order so the
//!   UI can render them as columns of work without re-running Kahn's
//!   algorithm on the client side.
//!
//! Each edge carries a reason and an optional source-artifact reference
//! so reviewers can link the rendered arrow back to the document that
//! introduced the metadata.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;

use crate::opensymphony_planning::generator::domain::{PlanArtifacts, PlannedMilestone, TaskId};

use super::checks::creation_order_waves;
use super::domain::{DependencyGraph, GraphEdge, GraphEdgeReason, GraphNode, GraphNodeKind};

/// Builds a [`DependencyGraph`] from in-memory planning artefacts.
#[derive(Debug, Default, Clone)]
pub struct DependencyGraphBuilder;

impl DependencyGraphBuilder {
    /// Builds a deterministic graph from the supplied planning artefacts.
    /// The returned graph's `parallelizable_waves` field already reflects a
    /// topological ordering, so callers do not need to re-sort.
    #[allow(dead_code)]
    pub fn build(artifacts: &PlanArtifacts) -> DependencyGraph {
        let mut nodes: Vec<GraphNode> = Vec::new();
        let mut edges: Vec<GraphEdge> = Vec::new();

        for milestone in &artifacts.milestones {
            collect_milestone_nodes(milestone, &mut nodes);
            collect_milestone_edges(milestone, &mut edges);
        }

        nodes.sort_by(|a, b| {
            a.milestone
                .cmp(&b.milestone)
                .then_with(|| kind_rank(a.kind).cmp(&kind_rank(b.kind)))
                .then_with(|| a.id.cmp(&b.id))
        });
        edges.sort_by(|a, b| {
            a.milestone
                .cmp(&b.milestone)
                .then_with(|| a.from.cmp(&b.from))
                .then_with(|| relation_rank(a.relation).cmp(&relation_rank(b.relation)))
                .then_with(|| a.to.cmp(&b.to))
        });

        let parallelizable_waves = parallelizable_waves_deterministic(artifacts);

        DependencyGraph {
            planning_wave: artifacts.planning_wave.clone(),
            generated_at: Utc::now(),
            nodes,
            edges,
            parallelizable_waves,
        }
    }
}

fn kind_rank(kind: GraphNodeKind) -> u8 {
    match kind {
        GraphNodeKind::Milestone => 0,
        GraphNodeKind::Issue => 1,
        GraphNodeKind::SubIssue => 2,
    }
}

fn relation_rank(relation: GraphEdgeReason) -> u8 {
    match relation {
        GraphEdgeReason::ParentOf => 0,
        GraphEdgeReason::BlockedBy => 1,
        GraphEdgeReason::BlocksInvariant => 2,
        GraphEdgeReason::MissingInverse => 3,
        GraphEdgeReason::UnknownTarget => 4,
    }
}

fn collect_milestone_nodes(milestone: &PlannedMilestone, nodes: &mut Vec<GraphNode>) {
    nodes.push(GraphNode {
        id: milestone.id.clone(),
        kind: GraphNodeKind::Milestone,
        title: milestone.name.clone(),
        milestone: milestone.name.clone(),
        acceptance_criteria_count: milestone.acceptance_criteria.len(),
        verification_count: milestone.verification_steps.len(),
        source_artifact_ref: None,
    });
    for issue in &milestone.issues {
        nodes.push(GraphNode {
            id: issue.id.clone(),
            kind: GraphNodeKind::Issue,
            title: issue.title.clone(),
            milestone: milestone.name.clone(),
            acceptance_criteria_count: issue.acceptance_criteria.len(),
            verification_count: issue.verification_steps.len(),
            source_artifact_ref: issue.task_file.clone(),
        });
        for sub in &issue.sub_issues {
            nodes.push(GraphNode {
                id: sub.id.clone(),
                kind: GraphNodeKind::SubIssue,
                title: sub.title.clone(),
                milestone: milestone.name.clone(),
                acceptance_criteria_count: sub.acceptance_criteria.len(),
                verification_count: sub.verification_steps.len(),
                source_artifact_ref: sub.task_file.clone(),
            });
        }
    }
}

fn collect_milestone_edges(milestone: &PlannedMilestone, edges: &mut Vec<GraphEdge>) {
    let declared_ids: BTreeSet<TaskId> = collect_all_task_ids(milestone);
    let source_for = build_task_file_lookup(milestone);
    for issue in &milestone.issues {
        for sub in &issue.sub_issues {
            push_parent_edge(&sub.id, &issue.id, milestone, edges);
            push_blocker_edges(sub, milestone, &declared_ids, &source_for, edges);
        }
        push_blocker_edges(issue, milestone, &declared_ids, &source_for, edges);
    }
}

fn push_parent_edge(
    child: &TaskId,
    parent: &TaskId,
    milestone: &PlannedMilestone,
    edges: &mut Vec<GraphEdge>,
) {
    edges.push(GraphEdge {
        from: parent.clone(),
        to: child.clone(),
        relation: GraphEdgeReason::ParentOf,
        milestone: milestone.name.clone(),
        source_artifact_ref: None,
    });
}

fn push_blocker_edges(
    task: &dyn super::BlockingTask,
    milestone: &PlannedMilestone,
    declared_ids: &BTreeSet<TaskId>,
    source_for: &BTreeMap<TaskId, Option<String>>,
    edges: &mut Vec<GraphEdge>,
) {
    for blocker in task.blocked_by() {
        let reason = if !declared_ids.contains(blocker) {
            GraphEdgeReason::UnknownTarget
        } else {
            GraphEdgeReason::BlockedBy
        };
        edges.push(GraphEdge {
            from: blocker.clone(),
            to: task.id(),
            relation: reason,
            milestone: milestone.name.clone(),
            source_artifact_ref: source_for.get(blocker).and_then(|file| file.clone()),
        });
    }
    let mut blocks_pairs: Vec<(TaskId, TaskId)> = task
        .blocks()
        .iter()
        .map(|blocked| (task.id(), blocked.clone()))
        .collect();
    blocks_pairs.sort();
    for (source, target) in &blocks_pairs {
        let reason = if !declared_ids.contains(target) {
            GraphEdgeReason::UnknownTarget
        } else {
            GraphEdgeReason::BlocksInvariant
        };
        edges.push(GraphEdge {
            from: source.clone(),
            to: target.clone(),
            relation: reason,
            milestone: milestone.name.clone(),
            source_artifact_ref: source_for.get(source).and_then(|file| file.clone()),
        });
    }
}

fn collect_all_task_ids(milestone: &PlannedMilestone) -> BTreeSet<TaskId> {
    let mut ids: BTreeSet<TaskId> = BTreeSet::new();
    ids.insert(milestone.id.clone());
    for issue in &milestone.issues {
        ids.insert(issue.id.clone());
        for sub in &issue.sub_issues {
            ids.insert(sub.id.clone());
        }
    }
    ids
}

/// Build a lookup from `TaskId` -> task file for every task referenced inside
/// the milestone, so edges can attach the FROM-side artifact reference
/// regardless of which side of the relationship was iterated.
fn build_task_file_lookup(milestone: &PlannedMilestone) -> BTreeMap<TaskId, Option<String>> {
    // BTreeMap keeps entries in sorted order so the graph-edge attributes
    // stream is deterministic for the JSON plan-validation artifact (matches
    // the rest of this module, see COE-416 review).
    let mut lookup: BTreeMap<TaskId, Option<String>> = BTreeMap::new();
    for issue in &milestone.issues {
        lookup.insert(issue.id.clone(), issue.task_file.clone());
        for sub in &issue.sub_issues {
            lookup.insert(sub.id.clone(), sub.task_file.clone());
        }
    }
    lookup
}

fn parallelizable_waves_deterministic(artifacts: &PlanArtifacts) -> Vec<Vec<TaskId>> {
    let mut waves = creation_order_waves(artifacts);
    for wave in &mut waves {
        wave.sort();
    }
    waves
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::opensymphony_planning::generator::domain::{
        AcceptanceCriterion, ManifestTask, PlannedIssue, PlannedMilestone, PlannedSubIssue,
        TaskPackageManifest, TaskPriority,
    };

    fn artifacts_for(issues: Vec<PlannedIssue>) -> PlanArtifacts {
        let milestone = PlannedMilestone {
            id: TaskId::new("M9"),
            name: "M9: Wave".to_string(),
            goal: "Goal".to_string(),
            issues,
            acceptance_criteria: vec![],
            verification_steps: vec![],
            notes: None,
        };
        PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "test-wave".to_string(),
            milestones: vec![milestone.clone()],
            manifest: TaskPackageManifest {
                planning_wave: "test-wave".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec![milestone.name.clone()],
                tasks: vec![ManifestTask {
                    id: milestone.id.clone(),
                    file: "docs/tasks/m9.md".to_string(),
                }],
            },
            milestone_index: String::new(),
            task_files: Default::default(),
        }
    }

    fn issue(id: &str, blocked_by: Vec<&str>, blocks: Vec<&str>) -> PlannedIssue {
        PlannedIssue {
            id: TaskId::new(id),
            title: format!("Issue {}", id),
            summary: "S".to_string(),
            scope_in: vec!["in".to_string()],
            scope_out: vec![],
            deliverables: vec!["d".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: "AC".to_string(),
                verification_command: None,
            }],
            verification_steps: vec![],
            context: vec![],
            definition_of_ready: vec![],
            notes: None,
            priority: TaskPriority::Normal,
            estimate: None,
            blocked_by: blocked_by.iter().map(|s| TaskId::new(*s)).collect(),
            blocks: blocks.iter().map(|s| TaskId::new(*s)).collect(),
            sub_issues: vec![],
            task_file: Some(format!("docs/tasks/{}.md", id)),
        }
    }

    fn sub_issue(id: &str, blocked_by: Vec<&str>) -> PlannedSubIssue {
        PlannedSubIssue {
            id: TaskId::new(id),
            title: format!("Sub {}", id),
            summary: "S".to_string(),
            scope_in: vec!["in".to_string()],
            scope_out: vec![],
            deliverables: vec!["d".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: "AC".to_string(),
                verification_command: None,
            }],
            verification_steps: vec!["verify".to_string()],
            context: vec![],
            definition_of_ready: vec![],
            notes: None,
            priority: TaskPriority::Normal,
            estimate: None,
            blocked_by: blocked_by.iter().map(|s| TaskId::new(*s)).collect(),
            blocks: vec![],
            task_file: Some(format!("docs/tasks/{}.md", id)),
        }
    }

    #[test]
    fn builder_emits_acyclic_graph_with_sources_and_reasons() {
        // Two real issues; A blocks B and B's blockedBy reciprocates so the
        // builder emits a BlocksInvariant edge (not an UnknownTarget one).
        let a = issue("OSYM-734", vec![], vec!["OSYM-735"]);
        let b = issue("OSYM-735", vec!["OSYM-734"], vec![]);
        let artifacts = artifacts_for(vec![a, b]);
        let graph = DependencyGraphBuilder::build(&artifacts);
        assert!(graph.edges.iter().any(|e| {
            e.relation == GraphEdgeReason::BlocksInvariant
                && e.from == TaskId::new("OSYM-734")
                && e.to == TaskId::new("OSYM-735")
        }));
        assert!(
            graph
                .parallelizable_waves
                .first()
                .is_some_and(|w| w.contains(&TaskId::new("OSYM-734")))
        );
        let blocker_edge = graph
            .edges
            .iter()
            .find(|e| {
                e.from == TaskId::new("OSYM-734")
                    && e.to == TaskId::new("OSYM-735")
                    && e.relation == GraphEdgeReason::BlocksInvariant
            })
            .expect("expected a BlocksInvariant edge from OSYM-734 to OSYM-735");
        assert_eq!(
            blocker_edge.source_artifact_ref.as_deref(),
            Some("docs/tasks/OSYM-734.md")
        );
    }

    #[test]
    fn builder_marks_unknown_target_reason() {
        let parent_issue = issue("OSYM-734", vec!["OSYM-DOES-NOT-EXIST"], vec![]);
        let artifacts = artifacts_for(vec![parent_issue]);
        let graph = DependencyGraphBuilder::build(&artifacts);
        let unknown_edge = graph
            .edges
            .iter()
            .find(|e| e.to == TaskId::new("OSYM-734"))
            .expect("expected an incoming edge for OSYM-734");
        assert_eq!(unknown_edge.relation, GraphEdgeReason::UnknownTarget);
        assert_eq!(unknown_edge.from, TaskId::new("OSYM-DOES-NOT-EXIST"));
    }

    #[test]
    fn builder_emits_parent_of_edges_for_sub_issues() {
        let parent_issue = PlannedIssue {
            sub_issues: vec![sub_issue("OSYM-734.SUB", vec![])],
            ..issue("OSYM-734", vec![], vec![])
        };
        let artifacts = artifacts_for(vec![parent_issue]);
        let graph = DependencyGraphBuilder::build(&artifacts);
        assert!(
            graph
                .edges
                .iter()
                .any(|e| e.relation == GraphEdgeReason::ParentOf
                    && e.from == TaskId::new("OSYM-734")
                    && e.to == TaskId::new("OSYM-734.SUB"))
        );
        // Sub-issue node has a verification count sourced from verification_steps.
        let sub_node = graph
            .nodes
            .iter()
            .find(|n| n.id == TaskId::new("OSYM-734.SUB"))
            .expect("sub-issue node present");
        assert_eq!(sub_node.verification_count, 1);
        assert_eq!(sub_node.kind, GraphNodeKind::SubIssue);
    }
}
