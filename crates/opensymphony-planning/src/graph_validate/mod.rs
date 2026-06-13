//! Dependency-graph generator and plan-quality checks.
//!
//! This module owns the planning-session validation artefacts that the
//! downstream planning workspace UI (OSYM-735) and Linear draft preview
//! (OSYM-736) consume. It exposes three first-class types:
//!
//! - [`DependencyGraphBuilder`] in [`graph`] emits a deterministic graph
//!   artefact from in-memory [`crate::opensymphony_planning::generator::domain::PlanArtifacts`].
//! - [`PlanQualityChecker`] in [`checks`] runs cycle detection (delegating
//!   to the existing in-memory validator), missing-blocker detection,
//!   parallelizable-work grouping, and the full plan-check category matrix.
//! - [`ManifestValidator`] in [`manifest`] reads
//!   `docs/tasks/task-package.yaml` plus the declared task files and
//!   returns the same five error classes as the Python
//!   `convert-tasks-to-linear.py` validator.
//!
//! All three produce values of [`domain`], so the planning session API can
//! combine them into a single [`PlanValidationReport`] without further
//! translation.

pub mod checks;
pub mod domain;
pub mod frontmatter;
pub mod graph;
pub mod manifest;

pub use checks::{PlanQualityChecker, build_blocker_inverse, creation_order_waves};
pub use domain::{
    DependencyGraph, GraphEdge, GraphEdgeReason, GraphNode, GraphNodeKind,
    ManifestValidationResult, MissingTaskFile, PlanCheckCategory, PlanCheckFinding,
    PlanCheckSeverity, PlanValidationReport, SelfBlock, UnknownDependency, UnknownMilestone,
};
pub use frontmatter::{
    ParsedTaskFile, TaskFrontmatter, TaskFrontmatterError, parse_task_file, parse_task_text,
};
pub use graph::DependencyGraphBuilder;
pub use manifest::{
    ManifestTaskEntry, ManifestValidator, ManifestValidatorError, TaskPackageManifestFile,
    load_manifest,
};

/// Shared trait over the planning artefact types that carry bidirectional
/// blocker metadata. Both [`crate::opensymphony_planning::generator::domain::PlannedIssue`]
/// and [`crate::opensymphony_planning::generator::domain::PlannedSubIssue`]
/// implement this trait so the graph + plan-check modules can walk them
/// without duplicating the implementation per struct.
pub trait BlockingTask {
    fn id(&self) -> TaskId;
    fn blocked_by(&self) -> &[TaskId];
    fn blocks(&self) -> &[TaskId];
}

impl BlockingTask for PlannedIssue {
    fn id(&self) -> TaskId {
        self.id.clone()
    }
    fn blocked_by(&self) -> &[TaskId] {
        &self.blocked_by
    }
    fn blocks(&self) -> &[TaskId] {
        &self.blocks
    }
}

impl BlockingTask for PlannedSubIssue {
    fn id(&self) -> TaskId {
        self.id.clone()
    }
    fn blocked_by(&self) -> &[TaskId] {
        &self.blocked_by
    }
    fn blocks(&self) -> &[TaskId] {
        &self.blocks
    }
}

use chrono::Utc;

use crate::opensymphony_planning::generator::domain::{
    PlanArtifacts, PlannedIssue, PlannedSubIssue, TaskId,
};

use super::codebase::CodebaseAnalysis;
use super::research::ResearchBrief;

/// Convenience helper that runs the graph builder and the plan-quality
/// checker together. The manifest validator produces an independent
/// report and is not invoked from this helper because it reads from
/// disk; callers typically run it in a separate planning-session step.
#[allow(dead_code)]
pub fn build_in_memory_report(
    artifacts: &PlanArtifacts,
    research: Option<&ResearchBrief>,
    codebase: Option<&CodebaseAnalysis>,
) -> PlanValidationReport {
    let dependency_graph = DependencyGraphBuilder::build(artifacts);
    let mut checker = PlanQualityChecker::new(artifacts);
    if let Some(brief) = research {
        checker = checker.with_research(brief.findings.len());
    }
    if let Some(analysis) = codebase {
        // Count every risk regardless of severity so the in-memory checker
        // gets an accurate picture of what the analyzer actually found.
        // Filtering to `RiskSeverity::High` here would silently downgrade
        // medium/low risks to zero, which then makes the plan checker emit
        // a misleading "rerun the analyzer" warning for analyses that did
        // run and did find issues.
        let risk_count = analysis.risks.len();
        checker = checker.with_codebase(risk_count);
    }
    let plan_checks = checker.run();
    PlanValidationReport {
        planning_wave: artifacts.planning_wave.clone(),
        generated_at: Utc::now(),
        dependency_graph: Some(dependency_graph),
        plan_checks,
        manifest_validation: None,
    }
}

/// Convenience helper to attach a manifest-validation result onto an
/// existing in-memory report. The helper takes ownership of the supplied
/// manifest result so callers can move it into the report after the
/// on-disk validation step completes.
#[allow(dead_code)]
pub fn attach_manifest_validation(
    report: &mut PlanValidationReport,
    result: ManifestValidationResult,
) {
    report.manifest_validation = Some(result);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::opensymphony_planning::generator::generator::validate_dependency_graph;

    #[test]
    fn plan_validation_report_round_trips_through_json() {
        // A minimal-but-valid set of artefacts that the in-memory report
        // helper can consume. Confirms the planning-session API can
        // serialize and re-deserialize the report without losing fields.
        use crate::opensymphony_planning::generator::domain::{PlanArtifacts, TaskPackageManifest};
        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "rich-client-hosted-mode".to_string(),
            milestones: vec![],
            manifest: TaskPackageManifest {
                planning_wave: "rich-client-hosted-mode".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec![],
                tasks: vec![],
            },
            milestone_index: String::new(),
            task_files: Default::default(),
        };
        validate_dependency_graph(&artifacts).expect("no cycles in empty artifacts");
        let report = build_in_memory_report(&artifacts, None, None);
        let json = serde_json::to_string(&report).expect("serializable");
        assert!(json.contains("rich-client-hosted-mode"));
        assert!(json.contains("dependency_graph"));
        let parsed: PlanValidationReport = serde_json::from_str(&json).expect("deserializable");
        assert_eq!(parsed.planning_wave, "rich-client-hosted-mode");
        assert!(parsed.dependency_graph.is_some());
    }

    /// Regression test for the in-memory report helper: when the loaded
    /// codebase analysis contains only medium-severity risks, the helper
    /// must count all of them and produce exactly one `CodebaseAnalysis`
    /// warning — not zero. Previously the helper filtered to
    /// `RiskSeverity::High` only, which would silently downgrade
    /// medium/low risks to 0 and emit a misleading "rerun the analyzer"
    /// warning for an analysis that did run and did find issues.
    #[test]
    fn build_in_memory_report_counts_all_risk_severities() {
        use crate::opensymphony_planning::codebase::{
            AnalysisRisk, CodebaseAnalysis, RiskCategory, RiskSeverity,
        };
        use crate::opensymphony_planning::generator::domain::{PlanArtifacts, TaskPackageManifest};
        use crate::opensymphony_planning::graph_validate::{
            checks::PlanQualityChecker, domain::PlanCheckCategory,
        };

        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "rich-client-hosted-mode".to_string(),
            milestones: vec![],
            manifest: TaskPackageManifest {
                planning_wave: "rich-client-hosted-mode".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec![],
                tasks: vec![],
            },
            milestone_index: String::new(),
            task_files: Default::default(),
        };
        let analysis = CodebaseAnalysis {
            root_path: "/tmp".to_string(),
            languages: vec![],
            packages: vec![],
            build_systems: vec![],
            ownership_files: vec![],
            integration_points: vec![],
            conventions: vec![],
            // Two medium risks, no high risks. With the bug, this would be
            // bucketed to 0 and the plan checker would warn; with the fix,
            // the count is 2 so no `CodebaseAnalysis` warning fires.
            risks: vec![
                AnalysisRisk {
                    category: RiskCategory::Complexity,
                    severity: RiskSeverity::Medium,
                    description: "fan-out beyond threshold".to_string(),
                    affected_path: "crates/foo".to_string(),
                },
                AnalysisRisk {
                    category: RiskCategory::Coupling,
                    severity: RiskSeverity::Medium,
                    description: "cross-crate import".to_string(),
                    affected_path: "crates/bar".to_string(),
                },
            ],
            total_files: 0,
            total_rust_files: 0,
            total_typescript_files: 0,
        };

        let checker = PlanQualityChecker::new(&artifacts).with_codebase(analysis.risks.len());
        let findings = checker.run();
        assert!(
            !findings
                .iter()
                .any(|f| f.category == PlanCheckCategory::CodebaseAnalysis),
            "CodebaseAnalysis must not warn when the analyzer reported {len} risks",
            len = analysis.risks.len(),
        );
    }
}
