use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::version::SchemaVersion;

// ─── Artifact ────────────────────────────────────────────────────────────────

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

/// All artifact kinds produced or consumed by the planning wave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningArtifactKind {
    /// Initial problem statement and goals captured from stakeholder interview.
    Intake,
    /// Repository-level context (structure, languages, build system).
    ProjectContext,
    /// Functional and non-functional requirements.
    Requirements,
    /// Summary of research findings (external services, APIs, prior art).
    ResearchBrief,
    /// Codebase analysis results (architecture, hot-spots, constraints).
    CodebaseAnalysis,
    /// Architecture decision notes and trade-off analysis.
    ArchitectureNotes,
    /// Known risks and mitigation strategies.
    RiskRegister,
    /// Milestone-level plan (scope, timeline, key deliverables).
    MilestoneDraft,
    /// Issue-level plan (description, acceptance criteria, verification).
    IssueDraft,
    /// Sub-issue plan for decomposition.
    SubIssueDraft,
    /// Dependency map across milestones/issues/sub-issues.
    DependencyMap,
    /// Verification and test plan.
    VerificationPlan,
    /// Acceptance criteria and validation checklist.
    AcceptanceCriteria,
    /// Plan validation result (cycle checks, missing blocker checks, quality).
    PlanValidation,
    /// Linear draft (issues to be created before publishing).
    LinearDraft,
    /// Review comments collected during planning review.
    ReviewComments,
    /// Publish receipt emitted after Linear creation.
    PublishReceipt,
    /// Planning-wave identity and task-package projection.
    PlanningWave,
}

impl std::fmt::Display for PlanningArtifactKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Delegate to serde serialization so the Display output is guaranteed
        // to stay in sync with the `#[serde(rename_all = "snake_case")]` contract.
        let serde_str = serde_json::to_string(self).map_err(|_| std::fmt::Error)?;
        // serde_json serializes unit-variant enums as quoted strings, e.g. `"intake"`.
        write!(f, "{}", serde_str.trim_matches('"'))
    }
}

// ─── Artifact Revision & Diff ────────────────────────────────────────────────

/// Immutable snapshot of an artifact at a specific revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRevision {
    pub revision_id: String,
    pub artifact_id: String,
    pub version: u32,
    pub content_hash: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub authored_by: Option<String>,
    pub change_summary: Option<String>,
}

/// Unified diff between two artifact revisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactDiff {
    pub diff_id: String,
    pub artifact_id: String,
    pub from_version: u32,
    pub to_version: u32,
    pub unified_diff: String,
    pub lines_added: u32,
    pub lines_removed: u32,
    pub summary: Option<String>,
    pub generated_at: DateTime<Utc>,
}

// ─── Review Comment ──────────────────────────────────────────────────────────

/// A review comment attached to a specific artifact revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewComment {
    pub comment_id: String,
    pub session_id: String,
    pub artifact_id: String,
    pub revision_id: Option<String>,
    pub author: String,
    pub body: String,
    pub resolved: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ─── Conversation Turn ───────────────────────────────────────────────────────

/// Role of the participant in a planning conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    User,
    Agent,
    System,
}

impl std::fmt::Display for TurnRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let serde_str = serde_json::to_string(self).map_err(|_| std::fmt::Error)?;
        write!(f, "{}", serde_str.trim_matches('"'))
    }
}

/// A single turn in the planning conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub turn_id: String,
    pub session_id: String,
    pub turn_number: u32,
    pub role: TurnRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// Artifact IDs that were created or updated during this turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts_modified: Vec<String>,
    /// Free-form metadata attached to the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

// ─── Planning Session ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningSessionStatus {
    Draft,
    InReview,
    Approved,
    Published,
    Archived,
}

impl std::fmt::Display for PlanningSessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let serde_str = serde_json::to_string(self).map_err(|_| std::fmt::Error)?;
        write!(f, "{}", serde_str.trim_matches('"'))
    }
}

/// Full planning session state (superset of the summary).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningSession {
    pub schema_version: SchemaVersion,
    pub session_id: String,
    pub project_id: String,
    pub title: String,
    pub status: PlanningSessionStatus,
    pub planning_wave: Option<String>,
    pub created_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub turns: Vec<ConversationTurn>,
    pub artifacts: Vec<PlanningArtifact>,
    /// Free-form key-value metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Lightweight planning session summary for listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningSessionSummary {
    pub schema_version: SchemaVersion,
    pub session_id: String,
    pub project_id: String,
    pub title: String,
    pub status: PlanningSessionStatus,
    pub planning_wave: Option<String>,
    pub turn_count: u32,
    pub artifact_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl PlanningSession {
    /// Render a summary suitable for a compact listing view.
    pub fn summary(&self) -> PlanningSessionSummary {
        PlanningSessionSummary {
            schema_version: self.schema_version.clone(),
            session_id: self.session_id.clone(),
            project_id: self.project_id.clone(),
            title: self.title.clone(),
            status: self.status,
            planning_wave: self.planning_wave.clone(),
            turn_count: self.turns.len() as u32,
            artifact_count: self.artifacts.len() as u32,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    /// Collect review markdown for all artifacts in the session.
    pub fn render_review_markdown(&self) -> String {
        let mut out = String::from("# Planning Review\n\n");
        out.push_str(&format!("**Session:** {}\n\n", self.title));
        out.push_str(&format!("**Status:** {}\n\n", self.status));

        if !self.artifacts.is_empty() {
            out.push_str("## Artifacts\n\n");
            for artifact in &self.artifacts {
                out.push_str(&format!(
                    "### {} ({})\n\n{}\n\n",
                    artifact.title, artifact.kind, artifact.content
                ));
            }
        }

        if !self.turns.is_empty() {
            out.push_str("## Conversation\n\n");
            for turn in &self.turns {
                out.push_str(&format!(
                    "**{}** (turn {})\n\n{}\n\n",
                    turn.role, turn.turn_number, turn.content
                ));
            }
        }

        out
    }

    /// Render a compact prompt context for agent reuse.
    pub fn render_prompt_context(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!("[Session: {}]", self.title));

        if let Some(ref wave) = self.planning_wave {
            parts.push(format!("[Wave: {}]", wave));
        }

        for artifact in &self.artifacts {
            parts.push(format!(
                "[Artifact: {}] {}\n{}",
                artifact.kind, artifact.title, artifact.content
            ));
        }

        parts.join("\n\n")
    }

    /// Render an audit history listing every turn and artifact update.
    pub fn render_audit_history(&self) -> String {
        let mut out = String::from("# Audit History\n\n");
        out.push_str(&format!("Session: {}\n\n", self.session_id));

        for turn in &self.turns {
            let modified = if turn.artifacts_modified.is_empty() {
                String::from("none")
            } else {
                turn.artifacts_modified.join(", ")
            };
            out.push_str(&format!(
                "- {} [{}] turn={} modified=[{}]\n",
                turn.created_at.format("%Y-%m-%dT%H:%M:%SZ"),
                turn.role,
                turn.turn_number,
                modified,
            ));
        }

        out
    }
}

// ─── Planning Wave & Task Package Projection ─────────────────────────────────

/// Represents the planning-wave identity and task-package data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanningWave {
    pub wave_id: String,
    pub wave_name: String,
    pub tasks_dir: String,
    pub milestones: Vec<String>,
    pub task_entries: Vec<TaskEntry>,
}

/// A single task entry inside a planning wave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEntry {
    pub id: String,
    pub file: String,
}

/// Task package projection rendered from a PlanningWave artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPackageProjection {
    pub planning_wave: String,
    pub tasks_dir: String,
    pub milestones: Vec<String>,
    pub tasks: Vec<TaskEntry>,
}

impl PlanningWave {
    /// Render the YAML-compatible task-package projection.
    pub fn to_task_package_projection(&self) -> TaskPackageProjection {
        TaskPackageProjection {
            planning_wave: self.wave_name.clone(),
            tasks_dir: self.tasks_dir.clone(),
            milestones: self.milestones.clone(),
            tasks: self.task_entries.clone(),
        }
    }
}

// ─── Linear Publish Receipt ──────────────────────────────────────────────────

/// Matches the structure of `docs/tasks/linear-publish.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinearPublishReceipt {
    pub planning_wave: String,
    pub linear_project: String,
    pub published_at: DateTime<Utc>,
    pub milestones: Vec<PublishedMilestone>,
    pub tasks: Vec<PublishedTask>,
}

/// A milestone entry inside a publish receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedMilestone {
    pub name: String,
    pub milestone_id: String,
}

/// A task entry inside a publish receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedTask {
    pub task_id: String,
    pub issue: String,
    pub issue_id: String,
    pub url: String,
    pub file: String,
}

// ─── YAML output view structs ────────────────────────────────────────────────

/// CamelCase view of a milestone for YAML serialization.
#[derive(Serialize)]
struct PublishedMilestoneYaml {
    name: String,
    #[serde(rename = "milestoneId")]
    milestone_id: String,
}

/// CamelCase view of a task for YAML serialization.
#[derive(Serialize)]
struct PublishedTaskYaml {
    #[serde(rename = "taskId")]
    task_id: String,
    issue: String,
    #[serde(rename = "issueId")]
    issue_id: String,
    url: String,
    file: String,
}

/// CamelCase view of the full receipt for YAML serialization.
///
/// Using a dedicated Serialize-derived struct guarantees the compiler will
/// catch any field additions or renames — unlike manual Mapping builders.
#[derive(Serialize)]
struct LinearPublishReceiptYaml {
    #[serde(rename = "planningWave")]
    planning_wave: String,
    #[serde(rename = "linearProject")]
    linear_project: String,
    #[serde(rename = "publishedAt")]
    published_at: String,
    milestones: Vec<PublishedMilestoneYaml>,
    tasks: Vec<PublishedTaskYaml>,
}

impl LinearPublishReceipt {
    /// Render YAML string for the publish receipt using serde_yaml.
    ///
    /// Delegates to a Serialize-derived view struct so the compiler
    /// enforces field coverage and rename consistency.
    pub fn render_yaml(&self) -> String {
        let yaml = LinearPublishReceiptYaml {
            planning_wave: self.planning_wave.clone(),
            linear_project: self.linear_project.clone(),
            published_at: self
                .published_at
                .format("%Y-%m-%dT%H:%M:%S%.6f+00:00")
                .to_string(),
            milestones: self
                .milestones
                .iter()
                .map(|ms| PublishedMilestoneYaml {
                    name: ms.name.clone(),
                    milestone_id: ms.milestone_id.clone(),
                })
                .collect(),
            tasks: self
                .tasks
                .iter()
                .map(|t| PublishedTaskYaml {
                    task_id: t.task_id.clone(),
                    issue: t.issue.clone(),
                    issue_id: t.issue_id.clone(),
                    url: t.url.clone(),
                    file: t.file.clone(),
                })
                .collect(),
        };

        serde_yaml::to_string(&yaml)
            .expect("LinearPublishReceipt yaml serialization should never fail")
    }
}
