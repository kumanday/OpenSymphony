use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{run::RunAction, version::SchemaVersion};

/// Validation summary for `/api/v1/runs/{run_id}/validation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunValidationSummary {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    pub generated_at: DateTime<Utc>,
    pub overall_status: ValidationStatus,
    pub commands: Vec<ValidationCommand>,
    pub evidence: Vec<ValidationEvidenceItem>,
}

/// A validation command that ran against the run's workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationCommand {
    pub command_id: String,
    pub command: String,
    pub status: ValidationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_summary: Option<String>,
}

/// A single piece of validation evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationEvidenceItem {
    pub evidence_id: String,
    pub label: String,
    pub status: ValidationStatus,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_triggered: Option<RunAction>,
}

/// Status of a validation command or evidence item.
///
/// Kept separate from [`ApprovalStatus`](super::approval::ApprovalStatus) because
/// validation has a distinct lifecycle (running, skipped, error) that does not
/// overlap with approval decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Pending,
    Running,
    Passed,
    Failed,
    Skipped,
    Error,
}

impl fmt::Display for ValidationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Error => "error",
        };
        f.write_str(s)
    }
}
