//! Plan compiler for OSYM-733 / COE-415.
//!
//! The compiler is the transition layer between `PlanGenerator` output
//! (`PlanArtifacts`) and the downstream `convert-tasks-to-linear` publish flow.
//! It enforces Linear-native taxonomy (milestone == Linear project milestone,
//! issue == Linear issue, sub-issue == Linear sub-issue), validates that each
//! issue carries acceptance criteria, that each sub-issue carries verification
//! expectations, and that dependency metadata is shaped correctly. It then
//! emits the manifest shape consumed by `docs/tasks/task-package.yaml` and the
//! publish-receipt fields consumed by `docs/tasks/linear-publish.yaml`.
//!
//! Mapping GSD-2 inputs onto Linear entities:
//! - GSD-2 phase or milestone-level planning  -> Linear milestone
//! - GSD-2 slice                              -> Linear issue
//! - GSD-2 task                               -> Linear sub-issue
//!
//! Validation errors are *actionable*: each [`ValidationMessage`] names the
//! offending field and the fix that should be applied. Taxonomy violations
//! are reported separately through [`TaxonomyViolation`].

#[allow(clippy::module_inception)]
pub mod compiler;
#[allow(clippy::module_inception)]
pub mod domain;

pub use compiler::PlanCompiler;
pub use domain::{
    AppliedHierarchy, CompilationResult, CompiledIssue, CompiledMilestone, CompiledSubIssue,
    DependencyEdge, DependencyMetadata, DependencyRelation, LinearPublishEntity,
    LinearPublishReceipt, MilestoneReceipt, TaskKind, TaxonomyViolation, UnderspecifiedSubIssue,
    ValidationMessage, ValidationSeverity,
};
