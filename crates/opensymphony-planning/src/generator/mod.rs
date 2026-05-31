//! Implementation plan generator stage for collaborative planning sessions.
//!
//! This module wraps the existing `create-implementation-plan` skill as a
//! structured generator that uses intake, research, Linear state, and
//! codebase analysis artifacts to produce:
//!
//! - Planned milestones with goals, issues, and sub-issues
//! - Task package manifest (task-package.yaml equivalent)
//! - Human-readable milestone index (milestones.md equivalent)
//! - Individual task files with Linear-compatible frontmatter
//! - Acceptance criteria, verification steps, and dependency graphs
//!
//! The generator supports selective regeneration of specific artifact types,
//! preserving human-reviewed artifacts outside the selected regeneration scope.

pub mod domain;
#[allow(clippy::module_inception)]
pub mod generator;
pub mod session;

pub use domain::{
    AcceptanceCriterion, ManifestTask, PlanArtifacts, PlannedIssue, PlannedMilestone,
    PlannedSubIssue, RegenerationScope, TaskId, TaskPackageManifest, TaskPriority,
};
pub use generator::{GenerationError, PlanGenerator, validate_dependency_graph};
pub use session::{IntakeContext, PlanningSession};
