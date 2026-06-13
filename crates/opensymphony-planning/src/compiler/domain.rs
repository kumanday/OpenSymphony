//! Domain types for the milestone/issue/sub-issue plan compiler.
//!
//! These types carry the compiler output, validation diagnostics, and the
//! Linear-published metadata fields the downstream publish stage fills in.
//! They use direct Linear terminology (milestone, issue, sub-issue) and are
//! serialised to the `task-package.yaml` and `linear-publish.yaml` shapes
//! consumed by `convert-tasks-to-linear`.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::generator::domain::{PlannedIssue, PlannedSubIssue, TaskId};

/// Linear taxonomy applied by the compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    /// Linear project milestone -- mapped from GSD-2 phase or milestone-level planning.
    Milestone,
    /// Linear issue -- mapped from a GSD-2 slice.
    Issue,
    /// Linear sub-issue -- mapped from a GSD-2 task.
    SubIssue,
}

impl TaskKind {
    /// Returns true when the kind corresponds to a leaf that the
    /// `convert-tasks-to-linear` publish stage creates or updates.
    pub fn is_publishable(self) -> bool {
        matches!(self, Self::Issue | Self::SubIssue)
    }
}

/// Directory containing task files consumed by the downstream converter.
/// Sourced from `PlanningSession.tasks_dir` so receipts always reference
/// the same files the converter reads.
pub type TasksDir = String;

/// A taxonomy violation raised when a planning artifact cannot be classified
/// into the Linear milestone/issue/sub-issue hierarchy. Violations are
/// blocking but always actionable: the [`TaxonomyViolation::reason`] field
/// states what is wrong and the source task id is preserved for follow-up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxonomyViolation {
    pub task_id: Option<TaskId>,
    pub task_kind: Option<TaskKind>,
    pub reason: String,
    pub actionable: String,
}

/// Severity of a [`ValidationMessage`]. Errors block publish, warnings
/// produce flagged entries but do not block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationSeverity {
    Error,
    Warning,
}

/// A single validation message. `field` names the schema field that is
/// missing or invalid so downstream tooling can produce a precise fix
/// instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationMessage {
    pub severity: ValidationSeverity,
    pub task_id: Option<TaskId>,
    pub field: String,
    pub message: String,
}

impl ValidationMessage {
    pub fn error(
        task_id: Option<TaskId>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: ValidationSeverity::Error,
            task_id,
            field: field.into(),
            message: message.into(),
        }
    }

    pub fn warning(
        task_id: Option<TaskId>,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: ValidationSeverity::Warning,
            task_id,
            field: field.into(),
            message: message.into(),
        }
    }
}

/// A sub-issue flagged as underspecified. The compiler records the exact
/// field counts already present on the sub-issue so the planning stage can
/// recover without re-running the entire pipeline. A `*_count` of `0`
/// indicates the corresponding field is missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnderspecifiedSubIssue {
    pub sub_issue_id: TaskId,
    pub parent_issue_id: TaskId,
    pub acceptance_criteria_count: usize,
    pub verification_steps_count: usize,
    pub deliverables_count: usize,
    pub scope_in_count: usize,
    pub reasons: Vec<String>,
}

/// Linear entity reference recorded in the publish receipt. The
/// `issue`/`issueId`/`url` fields are populated by the downstream publish
/// step. Planning-stage output leaves them as `None` so callers can detect
/// which entries still need to be published.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearPublishEntity {
    pub source_task_id: TaskId,
    pub source_file: String,
    pub linear_kind: TaskKind,
    pub linear_milestone: String,
    pub parent_task_id: Option<TaskId>,
    pub blocked_by: Vec<TaskId>,
    pub blocks: Vec<TaskId>,
    /// Review comment lanes preserved so draft preview can render them.
    /// Stored as opaque markers; the publish stage does not interpret them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_comments: Vec<String>,
    pub issue: Option<String>,
    pub issue_id: Option<String>,
    pub url: Option<String>,
}

/// Milestone receipt stored in `docs/tasks/linear-publish.yaml`. The
/// `linked_issues` list is sourced from the compiled hierarchy milestones so
/// the publish step can quickly detect when an issue has not been attached.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneReceipt {
    pub name: String,
    pub milestone_id: Option<String>,
    pub linked_issues: Vec<TaskId>,
}

/// Publish-receipt fields emitted for `docs/tasks/linear-publish.yaml`.
/// The compiler fills the planning-side fields (`planningWave`, milestones,
/// tasks) and leaves Linear-side fields (`linearProject`, `publishedAt`,
/// entity ids, urls) as `None` to be populated by the publish step.
///
/// `milestones` is keyed by exact milestone name (matching the existing
/// `linear-publish.yaml` fixture) so the compiler's slice can be merged
/// into the live receipt without rewriting existing milestone entries.
///
/// The serialised shape preserves camelCase keys so the eventual
/// `linear-publish.yaml` artefact matches both the existing fixture layout
/// and the `convert-tasks-to-linear` validator that already loads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearPublishReceipt {
    pub planning_wave: String,
    pub linear_project: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub milestones: BTreeMap<String, MilestoneReceipt>,
    pub tasks: BTreeMap<TaskId, LinearPublishEntity>,
}

/// Applied hierarchy view used by the Linear draft preview consumer. The
/// tree shape preserves direct Linear terminology and is *separate* from the
/// receipt payload so preview consumers can render the tree without having
/// to deserialize the full publish receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedHierarchy {
    pub planning_wave: String,
    pub milestones: Vec<CompiledMilestone>,
}

/// Linear-native milestone projection carrying only the fields that survive
/// downstream compilation. Input fields that are not part of the Linear
/// milestone type are intentionally dropped here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledMilestone {
    pub name: String,
    pub goal: String,
    pub notes: Option<String>,
    pub issues: Vec<CompiledIssue>,
}

/// Linear-native issue projection carrying acceptance criteria counts so
/// the draft preview can flag gaps without re-reading the original artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledIssue {
    pub task_id: TaskId,
    pub title: String,
    pub summary: String,
    pub source_file: String,
    pub milestones: String,
    pub priority: u8,
    pub estimate: Option<u8>,
    pub blocked_by: Vec<TaskId>,
    pub blocks: Vec<TaskId>,
    pub acceptance_criteria_count: usize,
    pub acceptance_criteria_descriptions: Vec<String>,
    pub verification_count: usize,
    pub sub_issues: Vec<CompiledSubIssue>,
}

/// Linear-native sub-issue projection. `underspecified_reasons` lists the
/// exact field gaps so the planning stage can recover without rerunning the
/// generator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledSubIssue {
    pub task_id: TaskId,
    pub title: String,
    pub summary: String,
    pub source_file: String,
    pub parent_task_id: TaskId,
    pub milestones: String,
    pub priority: u8,
    pub estimate: Option<u8>,
    pub blocked_by: Vec<TaskId>,
    pub blocks: Vec<TaskId>,
    pub acceptance_criteria_count: usize,
    pub verification_count: usize,
    pub verification_steps: Vec<String>,
    pub underspecified_reasons: Vec<String>,
}

/// Dependency relation kind. The metadata output preserves explicit kinds so
/// downstream consumers can render `parent -> sub-issue` hierarchy edges
/// separately from `blocked_by` blocker edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyRelation {
    /// Source task blocks the target task (mirrors `blocked_by`).
    Blocks,
    /// Source task is the parent issue of the target sub-issue.
    ParentOf,
}

/// A single dependency edge in the compiled metadata. Edges are emitted in
/// the same order they appear in the compiled artifacts so the dependency
/// graph view in the draft preview reproduces deterministic output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub source: TaskId,
    pub target: TaskId,
    pub milestone: String,
    pub relation: DependencyRelation,
}

/// Compact dependency metadata emitted for `linear-publish.yaml` consumers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyMetadata {
    pub planning_wave: String,
    pub total_nodes: usize,
    pub milestone_count: usize,
    pub issue_count: usize,
    pub sub_issue_count: usize,
    pub edges: Vec<DependencyEdge>,
}

/// Final compiler output. `manifest_yaml` and `publish_receipt_yaml` are
/// the textual projections; `applied_hierarchy`, `taxonomy_violations`,
/// `validation_messages`, `underspecified_sub_issues`, and
/// `dependency_metadata` are the structured projections consumed by tests,
/// the Linear draft preview task, and the publish step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilationResult {
    pub planning_wave: String,
    pub manifest_yaml: String,
    pub publish_receipt_yaml: String,
    pub applied_hierarchy: AppliedHierarchy,
    pub taxonomy_violations: Vec<TaxonomyViolation>,
    pub validation_messages: Vec<ValidationMessage>,
    pub underspecified_sub_issues: Vec<UnderspecifiedSubIssue>,
    pub dependency_metadata: DependencyMetadata,
}

impl CompilationResult {
    /// Returns true when there are no blocking validation errors and no
    /// taxonomy violations. Underspecified sub-issues are surfaced as
    /// warnings; they do not block publish when the rest of the output is
    /// valid.
    pub fn is_publishable(&self) -> bool {
        self.taxonomy_violations.is_empty()
            && self
                .validation_messages
                .iter()
                .all(|m| m.severity != ValidationSeverity::Error)
    }
}

/// Helper conversions used by the compiler body. They never mutate the input
/// artefacts; they only project fields into the Linear-native shape.
pub(crate) fn issue_to_compiled(issue: &PlannedIssue, milestone_name: &str) -> CompiledIssue {
    CompiledIssue {
        task_id: issue.id.clone(),
        title: issue.title.clone(),
        summary: issue.summary.clone(),
        source_file: issue.task_file.clone().unwrap_or_default(),
        milestones: milestone_name.to_string(),
        priority: issue.priority.as_linear_priority(),
        estimate: issue.estimate,
        blocked_by: issue.blocked_by.clone(),
        blocks: issue.blocks.clone(),
        acceptance_criteria_count: issue.acceptance_criteria.len(),
        acceptance_criteria_descriptions: issue
            .acceptance_criteria
            .iter()
            .map(|c| c.description.clone())
            .collect(),
        verification_count: issue.verification_steps.len(),
        sub_issues: issue
            .sub_issues
            .iter()
            .map(|s| sub_issue_to_compiled(s, issue.id.clone(), milestone_name))
            .collect(),
    }
}

/// Helper conversions for sub-issues.
pub(crate) fn sub_issue_to_compiled(
    sub_issue: &PlannedSubIssue,
    parent_task_id: TaskId,
    milestone_name: &str,
) -> CompiledSubIssue {
    let underspecified_reasons = classify_underspecified_sub_issue(sub_issue);
    CompiledSubIssue {
        task_id: sub_issue.id.clone(),
        title: sub_issue.title.clone(),
        summary: sub_issue.summary.clone(),
        source_file: sub_issue.task_file.clone().unwrap_or_default(),
        parent_task_id,
        milestones: milestone_name.to_string(),
        priority: sub_issue.priority.as_linear_priority(),
        estimate: sub_issue.estimate,
        blocked_by: sub_issue.blocked_by.clone(),
        blocks: sub_issue.blocks.clone(),
        acceptance_criteria_count: sub_issue.acceptance_criteria.len(),
        verification_count: sub_issue.verification_steps.len(),
        verification_steps: sub_issue.verification_steps.clone(),
        underspecified_reasons,
    }
}

/// Classify the fields that make a sub-issue underspecified. Each reason is
/// a single literal human-readable string the draft preview can surface.
pub(crate) fn classify_underspecified_sub_issue(sub_issue: &PlannedSubIssue) -> Vec<String> {
    let mut reasons = Vec::new();
    if sub_issue.acceptance_criteria.is_empty() {
        reasons.push("missing acceptance criteria".to_string());
    }
    if sub_issue.verification_steps.is_empty() {
        reasons.push("missing verification expectations".to_string());
    }
    if sub_issue.deliverables.is_empty() {
        reasons.push("missing deliverables".to_string());
    }
    if sub_issue.scope_in.is_empty() {
        reasons.push("missing in-scope items".to_string());
    }
    reasons
}
