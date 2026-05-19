//! Planning-stage analysis for repository structure, Linear graph context, and research artifacts.
//!
//! This module provides the analysis pass used during collaborative planning sessions:
//!
//! - **Codebase Analysis** (`codebase`): Scans a repository for structure, conventions,
//!   integration points, ownership boundaries, and risks.
//! - **Linear Graph Analysis** (`linear_graph`): Produces a summary of existing Linear
//!   project state including milestones, blocker chains, and issue distributions.
//! - **Research Briefs** (`research`): Captures targeted research findings with citations
//!   for review before plan generation.

pub mod codebase;
pub mod domain;
pub mod linear_graph;
pub mod research;

pub use codebase::{
    AnalysisRisk, CodebaseAnalysis, CodebaseAnalysisError, CodebaseAnalyzer, Convention,
    IntegrationPoint, LanguageSignature, OwnershipSignal, PackageInfo, PackageKind,
};
pub use linear_graph::{
    BlockerChain, BlockerSnapshot, ChildRef, IssueSnapshot, LinearGraphAnalysis,
    LinearGraphAnalyzer, MilestoneSummary, ParentChildRelationship,
};
pub use research::{
    ConfidenceLevel, ResearchArtifactStore, ResearchBrief, ResearchBriefBuilder, ResearchError,
    ResearchFinding,
};
