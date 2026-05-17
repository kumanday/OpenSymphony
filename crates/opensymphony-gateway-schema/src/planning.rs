use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// Planning session artifact exposed by the gateway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningArtifact {
    pub schema_version: SchemaVersion,
    pub artifact_id: String,
    pub session_id: String,
    pub kind: PlanningArtifactKind,
    pub title: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub generated_by: Option<String>,
    pub approved: bool,
    pub published_to_tracker: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningArtifactKind {
    MilestoneDraft,
    IssueDraft,
    SubIssueDraft,
    DependencyMap,
    AcceptanceCriteria,
    VerificationPlan,
    ResearchSummary,
    CodebaseAnalysis,
}

/// Planning session summary for listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningSessionSummary {
    pub schema_version: SchemaVersion,
    pub session_id: String,
    pub project_id: String,
    pub title: String,
    pub status: PlanningSessionStatus,
    pub artifact_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningSessionStatus {
    Draft,
    InReview,
    Approved,
    Published,
    Archived,
}
