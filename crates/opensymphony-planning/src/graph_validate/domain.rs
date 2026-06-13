//! Domain types for the dependency-graph generator and plan-quality checks.
//!
//! These types are the JSON-shaped planning-session artefacts produced by
//! the `graph_validate` module. They use direct Linear terminology
//! (milestone, issue, sub-issue) and stay serde-friendly so the planning
//! session API can serialize them without further transformation.
//!
//! The graph hierarchy is `Milestone -> Issue -> SubIssue` and edges are
//! emitted in the same deterministic order produced by the compiler
//! (`DependencyMetadata`): sorted by milestone then source id then
//! target id. Stable ordering keeps diff-friendly output across runs.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::opensymphony_planning::generator::domain::TaskId;

/// Node kind in the dependency graph. Mirrors the Linear-native taxonomy the
/// `compiler` module already emits so the planning workspace UI can render
/// the same node labels when it consumes this artefact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphNodeKind {
    Milestone,
    Issue,
    SubIssue,
}

/// A single node in the dependency graph. The optional fields below let
/// downstream consumers (planning workspace UI, draft preview) build the
/// review view without re-reading the original planning artefacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: TaskId,
    pub kind: GraphNodeKind,
    pub title: String,
    pub milestone: String,
    pub acceptance_criteria_count: usize,
    pub verification_count: usize,
    /// Relative task file path when known (`docs/tasks/*.md`). Empty when
    /// the node was loaded from in-memory planning artefacts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_artifact_ref: Option<String>,
}

/// A reason attached to every graph edge so reviewers can immediately tell
/// why the planner inserted the edge (parent-child vs blocker). The four
/// variants match the actions the compiler records in `DependencyEdge` —
/// `ParentOf` for hierarchy and `Blocks` for blockers — plus the missing /
/// inconsistent metadata cases the graph validator surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeReason {
    /// `source` is the parent of `target` in the milestone/issue/sub-issue
    /// hierarchy. Always present for sub-issue nodes.
    ParentOf,
    /// `source` is in the `blocked_by` list of `target` (forward blocker).
    BlockedBy,
    /// `source` is in the `blocks` list of `target` (inverse metadata). Kept
    /// to make symmetric blocker metadata visible in the graph.
    BlocksInvariant,
    /// `source` is declared as a blocker for `target` but the original
    /// artifact does not record the matching inverse on `source`. Surfaced
    /// to flag inconsistent blocker metadata before publish.
    MissingInverse,
    /// The edge references a task that is not declared in the artefact.
    /// Captured when the manifest validator discovers an unknown dependency.
    UnknownTarget,
}

/// A single dependency edge in the produced graph artefact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: TaskId,
    pub to: TaskId,
    pub relation: GraphEdgeReason,
    /// Milestone the edge belongs to. Empty when the relation comes from
    /// the manifest-level validator (where milestone classification is only
    /// available via the task frontmatter).
    pub milestone: String,
    /// Source artifact reference (task file path or planning artefact key).
    /// Empty when the planner did not record a provenance path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_artifact_ref: Option<String>,
}

/// Top-level dependency graph artefact. Edges and nodes are sorted
/// deterministically so consumers can render diffs between revisions
/// without reordering themselves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub planning_wave: String,
    pub generated_at: DateTime<Utc>,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Groups of tasks that can be executed in parallel. Each entry is one
    /// creation-order wave ordered topologically: every task in wave `N`
    /// has all of its blockers satisfied by tasks in waves `0..N`.
    pub parallelizable_waves: Vec<Vec<TaskId>>,
}

/// Severity of a [`PlanCheckFinding`]. Mirrors the existing
/// `compiler::ValidationSeverity` so consumers that already understand
/// that enum can swallow the new one without change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanCheckSeverity {
    Error,
    Warning,
}

/// Categories of plan checks. They cover the full acceptance criteria list
/// from `docs/tasks/osym-734-dependency-graph-and-plan-checks.md`: scope
/// clarity, research coverage, codebase analysis, dependencies, acceptance
/// criteria, and verification expectations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanCheckCategory {
    /// Plan is missing or has empty in-scope / out-of-scope items.
    ScopeClarity,
    /// Plan or intake referenced research but no research findings were
    /// loaded into the planning session.
    ResearchCoverage,
    /// Plan or intake referenced codebase analysis but no `CodebaseAnalysis`
    /// was loaded into the planning session.
    CodebaseAnalysis,
    /// Plan has cycles / missing blockers / unknown dependencies.
    Dependencies,
    /// Issue or sub-issue is missing acceptance criteria.
    AcceptanceCriteria,
    /// Sub-issue is missing verification expectations.
    VerificationExpectations,
}

/// A single plan-check finding. `task_id` is optional so the checker can
/// surface artefact-wide issues (empty scope, missing milestone list, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanCheckFinding {
    pub severity: PlanCheckSeverity,
    pub category: PlanCheckCategory,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    pub field: String,
    pub message: String,
}

impl PlanCheckFinding {
    pub fn error(
        category: PlanCheckCategory,
        task_id: Option<TaskId>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: PlanCheckSeverity::Error,
            category,
            task_id,
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn warning(
        category: PlanCheckCategory,
        task_id: Option<TaskId>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: PlanCheckSeverity::Warning,
            category,
            task_id,
            field: field.into(),
            message: message.into(),
        }
    }
}

/// Manifest validation result. Mirrors the legacy Python
/// `convert_tasks_to_linear.py` `validate_task_graph` checks so an on-disk
/// `task-package.yaml` is validated identically regardless of which entry
/// point triggered the check.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ManifestValidationResult {
    pub planning_wave: String,
    /// IDs declared in the manifest's `tasks` list. Mirrors the manifest so
    /// downstream tooling can reconcile manifest entries with findings.
    pub declared_task_ids: Vec<TaskId>,
    /// Files declared in the manifest but not found on disk. Distinct from
    /// [`Self::invalid_task_files`] — a missing file is an authoritative
    /// `NotFound`, whereas an invalid file exists on disk but failed to
    /// parse or read.
    pub missing_task_files: Vec<MissingTaskFile>,
    /// Files declared in the manifest that exist on disk but cannot be
    /// loaded as a valid task file (YAML syntax error, missing frontmatter,
    /// permission denied, etc). Surfaced as a separate class so users
    /// trying to fix the root cause do not chase a phantom "missing" file.
    pub invalid_task_files: Vec<InvalidTaskFile>,
    pub unknown_milestones: Vec<UnknownMilestone>,
    pub unknown_dependencies: Vec<UnknownDependency>,
    pub creation_order_cycles: Vec<Vec<TaskId>>,
    pub self_blocks: Vec<SelfBlock>,
    pub duplicate_task_ids: Vec<TaskId>,
}

impl ManifestValidationResult {
    /// Returns true when there are no error-class findings.
    pub fn is_ok(&self) -> bool {
        self.missing_task_files.is_empty()
            && self.invalid_task_files.is_empty()
            && self.unknown_milestones.is_empty()
            && self.unknown_dependencies.is_empty()
            && self.creation_order_cycles.is_empty()
            && self.self_blocks.is_empty()
            && self.duplicate_task_ids.is_empty()
    }

    /// Total error finding count. Useful for test assertions.
    pub fn error_count(&self) -> usize {
        self.missing_task_files.len()
            + self.invalid_task_files.len()
            + self.unknown_milestones.len()
            + self.unknown_dependencies.len()
            + self
                .creation_order_cycles
                .iter()
                .map(|cycle| cycle.len())
                .sum::<usize>()
            + self.self_blocks.len()
            + self.duplicate_task_ids.len()
    }
}

/// One entry per task file that the manifest references but is missing
/// from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissingTaskFile {
    pub task_id: TaskId,
    pub file_path: String,
}

/// One entry per task file that exists on disk but failed to load. The
/// `reason` field carries the underlying parse/read error so users can fix
/// the root cause without having to re-run the validator with logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidTaskFile {
    pub task_id: TaskId,
    pub file_path: String,
    pub reason: String,
}

/// One entry per task whose `milestone` field is not in the manifest's
/// `milestones` list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownMilestone {
    pub task_id: TaskId,
    pub declared_milestone: String,
}

/// One entry per task whose `blockedBy` lists a task that is not declared
/// in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownDependency {
    pub from_task_id: TaskId,
    pub unknown_dependency: TaskId,
}

/// One entry per task that lists itself in `blockedBy` or `blocks`. The
/// manifest validator treats this as an error because a self-block cannot
/// be satisfied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelfBlock {
    pub task_id: TaskId,
}

/// The full planning-session validation artefact: graph + plan checks +
/// manifest validation, produced independently by `dependency_graph_builder`,
/// `plan_quality_checker`, and `manifest_validator`. Combined into one
/// report so planning sessions can serialise a single record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanValidationReport {
    pub planning_wave: String,
    pub generated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_graph: Option<DependencyGraph>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_checks: Vec<PlanCheckFinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_validation: Option<ManifestValidationResult>,
}

impl PlanValidationReport {
    /// True when none of the optional sections reports an error-severity
    /// finding. Manifest validation considers all its findings to be
    /// errors; plan checks filter by severity; dependency graph has no
    /// embedded severity so its presence is informational.
    pub fn has_errors<F>(&self, severity_filter: F) -> bool
    where
        F: Fn(PlanCheckSeverity) -> bool,
    {
        self.plan_checks.iter().any(|c| severity_filter(c.severity))
            || self
                .manifest_validation
                .as_ref()
                .is_some_and(|m| !m.is_ok())
    }

    /// Bumps counts into a small map keyed by category. Useful when the
    /// planner wants to render a dashboard view of "how many issues per
    /// category" without iterating the full finding list upstream.
    pub fn category_counts(&self) -> BTreeMap<PlanCheckCategory, usize> {
        let mut counts: BTreeMap<PlanCheckCategory, usize> = BTreeMap::new();
        for finding in &self.plan_checks {
            *counts.entry(finding.category).or_insert(0) += 1;
        }
        counts
    }
}
