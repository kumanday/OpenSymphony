//! Planning-stage analysis and generation for repository structure, Linear graph context,
//! research artifacts, and implementation plans.
//!
//! This module provides the full planning pipeline used during collaborative planning sessions:
//!
//! - **Codebase Analysis** (`codebase`): Scans a repository for structure, conventions,
//!   integration points, ownership boundaries, and risks.
//! - **Linear Graph Analysis** (`linear_graph`): Produces a summary of existing Linear
//!   project state including milestones, blocker chains, and issue distributions.
//! - **Research Briefs** (`research`): Captures targeted research findings with citations
//!   for review before plan generation.
//! - **Implementation Plan Generator** (`generator`): Transforms planning session context
//!   into structured milestones, issues, sub-issues, task packages, and dependency graphs.
//! - **Plan Compiler** (`compiler`): Enforces Linear-native taxonomy (milestone/issue/
//!   sub-issue) on `PlanArtifacts` and emits the manifest and publish-receipt projections.

pub mod codebase;
pub mod compiler;
pub mod domain;
pub mod generator;
pub mod graph_validate;
pub mod linear_graph;
pub mod research;

pub use codebase::{
    AnalysisRisk, CodebaseAnalysis, CodebaseAnalysisError, CodebaseAnalyzer, Convention,
    IntegrationPoint, LanguageSignature, OwnershipSignal, PackageInfo, PackageKind, RiskCategory,
    RiskSeverity,
};
pub use compiler::{
    AppliedHierarchy, CompilationResult, CompiledIssue, CompiledMilestone, CompiledSubIssue,
    DependencyEdge, DependencyMetadata, DependencyRelation, LinearPublishEntity,
    LinearPublishReceipt, MilestoneReceipt, PlanCompiler, TaskKind, TaxonomyViolation,
    UnderspecifiedSubIssue, ValidationMessage, ValidationSeverity,
};
pub use generator::{
    AcceptanceCriterion, GenerationError, IntakeContext, ManifestTask, PlanArtifacts,
    PlanGenerator, PlannedIssue, PlannedMilestone, PlannedSubIssue, PlanningSession,
    RegenerationScope, TaskId, TaskPackageManifest, TaskPriority, validate_dependency_graph,
};
pub use graph_validate::{
    DependencyGraph, DependencyGraphBuilder, GraphEdge, GraphEdgeReason, GraphNode, GraphNodeKind,
    ManifestTaskEntry, ManifestValidationResult, ManifestValidator, ManifestValidatorError,
    MissingTaskFile, ParsedTaskFile, PlanCheckCategory, PlanCheckFinding, PlanCheckSeverity,
    PlanQualityChecker, PlanValidationReport, SelfBlock, TaskFrontmatter, TaskFrontmatterError,
    TaskPackageManifestFile, UnknownDependency, UnknownMilestone, attach_manifest_validation,
    build_blocker_inverse, build_in_memory_report, creation_order_waves, load_manifest,
    parse_task_file, parse_task_text,
};
pub use linear_graph::{
    BlockerChain, BlockerSnapshot, ChildRef, IssueSnapshot, LinearGraphAnalysis,
    LinearGraphAnalyzer, MilestoneSummary, ParentChildRelationship,
};
pub use research::{
    ConfidenceLevel, ResearchArtifactStore, ResearchBrief, ResearchBriefBuilder, ResearchError,
    ResearchFinding,
};
