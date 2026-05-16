use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use chrono::{DateTime, NaiveDate, Utc};
use duckdb::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const DEFAULT_PRIVATE_MEMORY_CONFIG_FILE: &str = ".opensymphony/memory/memory.yaml";
pub const DEFAULT_MEMORY_CONFIG_FILE: &str = "opensymphony-memory.yaml";
pub const FALLBACK_PRIVATE_MEMORY_CONFIG_FILE: &str = ".opensymphony/memory/config.yaml";
pub const DEFAULT_MEMORY_ROOT: &str = ".opensymphony/memory";
pub const DEFAULT_INDEX_FILE_NAME: &str = "memory.duckdb";
pub const DEFAULT_PUBLIC_DOCS_ROOT: &str = "docs";
pub const ISSUE_CAPSULE_BEGIN: &str = "<!-- BEGIN OPENSYMPHONY MANAGED ISSUE CAPSULE -->";
pub const ISSUE_CAPSULE_END: &str = "<!-- END OPENSYMPHONY MANAGED ISSUE CAPSULE -->";
pub const TOPIC_DOC_BEGIN: &str = "<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->";
pub const TOPIC_DOC_END: &str = "<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->";
const MEMORY_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to create {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write {path}: {source}")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse YAML from {path}: {source}")]
    ParseYaml {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to encode JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to update DuckDB index {path}: {source}")]
    DuckDb {
        path: PathBuf,
        #[source]
        source: duckdb::Error,
    },
    #[error("failed to resolve {path}: {source}")]
    ResolvePath {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("Linear operation failed: {0}")]
    Linear(String),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{path} is outside the repository root {repo_root}")]
    PathOutsideRepo { path: PathBuf, repo_root: PathBuf },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryVisibility {
    #[default]
    Private,
    Public,
}

impl MemoryVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}

impl fmt::Display for MemoryVisibility {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSnapshotPolicy {
    Disabled,
    #[default]
    Hashes,
    PrivateSnapshots,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub config_path: PathBuf,
    pub repo_root: PathBuf,
    pub memory_root: PathBuf,
    pub visibility: MemoryVisibility,
    pub index_path: PathBuf,
    pub confidence_threshold: u8,
    pub source_snapshot_policy: SourceSnapshotPolicy,
    pub markdown_indexes: bool,
    pub docs: DocsConfig,
    pub areas: BTreeMap<String, AreaConfig>,
    pub redaction: RedactionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsConfig {
    pub public_root: PathBuf,
    pub default_visibility: MemoryVisibility,
    pub deny_private_links: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaConfig {
    pub slug: String,
    pub title: String,
    pub docs_target: PathBuf,
    pub visibility: MemoryVisibility,
    pub status: AreaStatus,
    pub confidence: u8,
    pub aliases: Vec<String>,
    pub source_refs: AreaSourceRefs,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaStatus {
    #[default]
    Candidate,
    Stable,
}

impl AreaStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Candidate => "candidate",
            Self::Stable => "stable",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaSourceRefs {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub docs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linear_labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linear_milestones: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linear_issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub github_prs: Vec<String>,
}

impl AreaSourceRefs {
    fn is_empty(&self) -> bool {
        self.docs.is_empty()
            && self.linear_labels.is_empty()
            && self.linear_milestones.is_empty()
            && self.linear_issues.is_empty()
            && self.github_prs.is_empty()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionConfig {
    pub deny_patterns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInitPlan {
    pub config_path: PathBuf,
    pub config_contents: String,
    pub gitignore_path: PathBuf,
    pub gitignore_before: Option<String>,
    pub gitignore_after: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryInitFileChange {
    Created,
    Updated,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryInitApplyReport {
    pub config_path: PathBuf,
    pub config: MemoryInitFileChange,
    pub gitignore_path: PathBuf,
    pub gitignore: MemoryInitFileChange,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct MemoryConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    memory_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visibility: Option<MemoryVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    index_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confidence_threshold: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_snapshots: Option<SourceSnapshotPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    markdown_indexes: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    docs: Option<DocsConfigFile>,
    #[serde(default)]
    areas: BTreeMap<String, AreaConfigFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    redaction: Option<RedactionConfigFile>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct DocsConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_root: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_visibility: Option<MemoryVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deny_private_links: Option<bool>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct AreaConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    docs_target: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    visibility: Option<MemoryVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<AreaStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confidence: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "AreaSourceRefs::is_empty")]
    source_refs: AreaSourceRefs,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct RedactionConfigFile {
    #[serde(default)]
    deny_patterns: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    #[serde(default)]
    pub issues: Vec<IssueEvidence>,
    #[serde(default)]
    pub prs: Vec<PullRequestEvidence>,
    #[serde(default)]
    pub overrides: BTreeMap<String, IssueOverride>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueEvidence {
    #[serde(default)]
    pub id: Option<String>,
    pub identifier: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub milestone: Option<String>,
    #[serde(default)]
    pub milestone_id: Option<String>,
    #[serde(default)]
    pub parent: Option<IssueLinkEvidence>,
    #[serde(default)]
    pub children: Vec<IssueLinkEvidence>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub comments: Vec<CommentEvidence>,
    #[serde(default)]
    pub linked_prs: Vec<u64>,
    #[serde(default)]
    pub task_files: Vec<PathBuf>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueLinkEvidence {
    #[serde(default)]
    pub id: Option<String>,
    pub identifier: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentEvidence {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestEvidence {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub merge_sha: Option<String>,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub commits: Vec<CommitEvidence>,
    #[serde(default)]
    pub changed_files: Vec<ChangedFileEvidence>,
    #[serde(default)]
    pub checks: Vec<CheckEvidence>,
    #[serde(default)]
    pub reviews: Vec<ReviewEvidence>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitEvidence {
    pub sha: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFileEvidence {
    pub path: PathBuf,
    #[serde(default)]
    pub change_kind: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckEvidence {
    pub name: String,
    #[serde(default)]
    pub conclusion: Option<String>,
    #[serde(default)]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEvidence {
    #[serde(default)]
    pub reviewer: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub disposition: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueOverride {
    #[serde(default)]
    pub prs: Vec<u64>,
    #[serde(default)]
    pub areas: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IssueSelection {
    pub identifiers: Vec<String>,
    pub milestone: Option<String>,
    pub state: Option<String>,
    pub before_date: Option<NaiveDate>,
    pub before_issue: Option<String>,
    pub area: Option<String>,
    pub since_last_sync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePlan {
    pub write: bool,
    pub selected: Vec<CaptureIssuePlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureIssuePlan {
    pub issue: IssueEvidence,
    pub prs: Vec<PullRequestEvidence>,
    pub capsule_path: PathBuf,
    pub areas: Vec<String>,
    pub docs_targets: Vec<PathBuf>,
    pub source_hash: String,
    pub already_captured: bool,
    pub stale: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureWriteReport {
    pub written_capsules: Vec<PathBuf>,
    pub index_path: PathBuf,
    pub markdown_indexes: Vec<PathBuf>,
    pub milestone_nodes: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub issue_key: String,
    pub title: String,
    pub capsule_path: PathBuf,
    pub areas: Vec<String>,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub issue_count: usize,
    pub warning_count: usize,
    pub docs_pending_count: usize,
    pub issues: Vec<StatusIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusIssue {
    pub issue_key: String,
    pub title: String,
    pub state: Option<String>,
    pub milestone: Option<String>,
    pub capsule_path: PathBuf,
    pub visibility: MemoryVisibility,
    pub areas: Vec<String>,
    pub docs_sync_status: String,
    pub warning_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintReport {
    pub findings: Vec<LintFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintFinding {
    pub severity: LintSeverity,
    pub path: Option<PathBuf>,
    pub message: String,
    pub next_command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintSeverity {
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsSyncPlan {
    pub write: bool,
    pub selected_issue_keys: Vec<String>,
    pub targets: Vec<DocsTargetPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocsTargetPlan {
    pub area: String,
    pub title: String,
    pub path: PathBuf,
    pub visibility: MemoryVisibility,
    pub create: bool,
    pub before: Option<String>,
    pub after: String,
    pub diff: String,
    pub issue_keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivePlan {
    pub write: bool,
    pub force: bool,
    pub issues: Vec<ArchiveIssuePlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveIssuePlan {
    pub issue_key: String,
    pub eligible: bool,
    pub reason: String,
    pub capsule_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IndexedIssue {
    issue_key: String,
    title: String,
    state: Option<String>,
    milestone: Option<String>,
    labels: Vec<String>,
    areas: Vec<String>,
    capsule_path: PathBuf,
    visibility: MemoryVisibility,
    source_hash: String,
    warning_count: usize,
    docs_sync_status: String,
    body: String,
}

include!("config.rs");
include!("capture.rs");
include!("query.rs");
include!("docs_sync.rs");
include!("archive.rs");
include!("capture_render.rs");
include!("index.rs");
include!("github.rs");
include!("util.rs");

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn ensure_memory_initialized_creates_config_and_gitignore_policy_once() {
        let repo = TempDir::new().expect("temp repo");

        let first = ensure_memory_initialized(repo.path(), None).expect("memory init");

        assert_eq!(first.config, MemoryInitFileChange::Created);
        assert_eq!(first.gitignore, MemoryInitFileChange::Created);
        assert!(
            repo.path()
                .join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE)
                .is_file()
        );
        assert_eq!(
            fs::read_to_string(repo.path().join(".gitignore")).expect(".gitignore"),
            ".opensymphony*\n!.opensymphony/\n.opensymphony/*\n!.opensymphony/memory/\n.opensymphony/memory/*\n!.opensymphony/memory/memory.yaml\n"
        );

        let second = ensure_memory_initialized(repo.path(), None).expect("memory init idempotent");

        assert_eq!(second.config, MemoryInitFileChange::Unchanged);
        assert_eq!(second.gitignore, MemoryInitFileChange::Unchanged);
    }

    #[test]
    fn capture_plan_matches_prs_and_infers_areas() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let source = sample_source();
        let selection = IssueSelection {
            identifiers: vec!["COE-123".to_string()],
            ..IssueSelection::default()
        };

        let plan = plan_capture(&config, &source, &selection, false, false).expect("plan");

        assert_eq!(plan.selected.len(), 1);
        let issue = &plan.selected[0];
        assert_eq!(issue.prs[0].number, 456);
        assert!(issue.areas.contains(&"openhands-runtime".to_string()));
        assert!(issue.docs_targets[0].ends_with("docs/openhands-runtime.md"));
    }

    #[test]
    fn capsule_generation_omits_transcript_like_comments() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let mut source = sample_source();
        source.issues[0].comments.push(CommentEvidence {
            body: "assistant: a full transcript should not be copied".to_string(),
            ..CommentEvidence::default()
        });
        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            false,
            false,
        )
        .expect("plan");

        let markdown = render_issue_capsule(&config, &plan.selected[0]).expect("capsule");

        assert!(markdown.contains("WebSocket reconnect recovery"));
        assert!(!markdown.contains("assistant: a full transcript"));
        assert!(markdown.contains("opensymphony debug COE-123"));
    }

    #[test]
    fn capsule_generation_filters_low_signal_review_noise() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let mut source = sample_source();
        source.prs[0].reviews = vec![
            ReviewEvidence {
                reviewer: Some("chatgpt-codex-connector".to_string()),
                state: Some("COMMENTED".to_string()),
                disposition: Some(
                    r#"
### Codex Review
https://github.com/example/repo/blob/abc/src/lib.rs#L10
**<sub><sub>![P2 Badge](https://img.shields.io/badge/P2-yellow?style=flat)</sub></sub>  Fail doctor config when env placeholders are unset**
Missing env-backed config should be surfaced as an explicit doctor failure.
"#
                    .to_string(),
                ),
                ..ReviewEvidence::default()
            },
            ReviewEvidence {
                reviewer: Some("chatgpt-codex-connector".to_string()),
                state: Some("COMMENTED".to_string()),
                disposition: Some(
                    r#"
### Codex Review

Here are some automated review suggestions for this pull request.

**Reviewed commit:** `abc1234`

<details> <summary>About Codex in GitHub</summary>

[Your team has set up Codex to review pull requests in this repo](https://example.com).
Reviews are triggered when you open a pull request for review.
"#
                    .to_string(),
                ),
                ..ReviewEvidence::default()
            },
            ReviewEvidence {
                reviewer: Some("kumanday".to_string()),
                state: Some("COMMENTED".to_string()),
                ..ReviewEvidence::default()
            },
            ReviewEvidence {
                reviewer: Some("github-actions".to_string()),
                state: Some("COMMENTED".to_string()),
                disposition: Some(
                    "Good taste. The changes address the remaining unresolved threads.".to_string(),
                ),
                ..ReviewEvidence::default()
            },
            ReviewEvidence {
                reviewer: Some("github-actions".to_string()),
                state: Some("COMMENTED".to_string()),
                disposition: Some(
                    "Good taste. The changes address the remaining unresolved threads.".to_string(),
                ),
                ..ReviewEvidence::default()
            },
            ReviewEvidence {
                reviewer: Some("reviewer".to_string()),
                state: Some("APPROVED".to_string()),
                ..ReviewEvidence::default()
            },
        ];
        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            false,
            false,
        )
        .expect("plan");

        let markdown = render_issue_capsule(&config, &plan.selected[0]).expect("capsule");

        assert!(!markdown.contains("Codex Review"));
        assert!(!markdown.contains("About Codex"));
        assert!(!markdown.contains("github.com/example/repo/blob"));
        assert!(!markdown.contains("P2 Badge"));
        assert!(markdown.contains("Fail doctor config when env placeholders are unset"));
        assert!(!markdown.contains("kumanday COMMENTED"));
        assert_eq!(
            markdown.matches("github-actions COMMENTED").count(),
            1,
            "duplicate automated summaries should collapse: {markdown}",
        );
        assert!(markdown.contains("reviewer APPROVED"));
    }

    #[test]
    fn write_capture_indexes_capsule_in_duckdb() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let source = sample_source();
        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            true,
            false,
        )
        .expect("plan");

        let report = write_capture_plan(&config, &plan, false).expect("write");
        let results = search(&config, "reconnect recovery", 10).expect("search");

        assert_eq!(report.written_capsules.len(), 1);
        assert!(config.index_path.exists());
        assert_eq!(results[0].issue_key, "COE-123");
    }

    #[test]
    fn capture_evolves_memory_config_and_keeps_changed_files_index_only() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("default config");
        let source = sample_source();
        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            true,
            false,
        )
        .expect("plan");

        write_capture_plan(&config, &plan, false).expect("write");

        let evolved = MemoryConfig::load(repo.path(), None).expect("evolved config");
        let area = evolved.areas.get("runtime").expect("runtime area");
        assert_eq!(area.status, AreaStatus::Stable);
        assert!(area.confidence >= evolved.confidence_threshold);
        assert!(
            area.source_refs
                .linear_labels
                .contains(&"runtime".to_string())
        );
        assert!(
            area.source_refs.linear_issues.is_empty(),
            "per-issue inventory belongs in capsules and DuckDB, not tracked memory.yaml"
        );
        assert!(
            area.source_refs.github_prs.is_empty(),
            "per-PR inventory belongs in capsules and DuckDB, not tracked memory.yaml"
        );

        let capsule =
            fs::read_to_string(evolved.issue_capsule_path("COE-123")).expect("capsule should read");
        assert!(capsule.contains("github_merge_shas"));
        assert!(capsule.contains("abcdef1234567890"));
        assert!(
            !capsule.contains("crates/opensymphony-openhands/src/client.rs"),
            "changed files should stay out of capsule prose and frontmatter"
        );

        let connection = Connection::open(&evolved.index_path).expect("index should open");
        let changed_file: String = connection
            .query_row(
                "SELECT file_path FROM changed_files WHERE issue_key = 'COE-123'",
                [],
                |row| row.get(0),
            )
            .expect("changed file should be indexed");
        assert_eq!(changed_file, "crates/opensymphony-openhands/src/client.rs");
    }

    #[test]
    fn capture_creates_candidate_area_from_linear_and_pr_narrative() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("default config");
        let mut source = sample_source();
        source.issues[0].title = "OpenHands runtime adapter".to_string();
        source.issues[0].milestone = None;
        source.issues[0].labels.clear();
        source.prs[0].title = "COE-123 support OpenHands runtime adapter".to_string();
        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            true,
            false,
        )
        .expect("plan");

        assert_eq!(plan.selected[0].areas, vec!["openhands-runtime-adapter"]);
        write_capture_plan(&config, &plan, false).expect("write");

        let evolved = MemoryConfig::load(repo.path(), None).expect("evolved config");
        let area = evolved
            .areas
            .get("openhands-runtime-adapter")
            .expect("candidate area");
        assert_eq!(area.status, AreaStatus::Candidate);
        assert!(area.confidence < evolved.confidence_threshold);
        assert!(
            area.source_refs.linear_issues.is_empty(),
            "candidate areas should not accumulate issue inventory in tracked config"
        );
    }

    #[test]
    fn area_evidence_matching_requires_whole_tokens() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let mut source = sample_source();
        source.issues[0].title = "OpenHands gruntimeerror handling".to_string();
        source.issues[0].description =
            Some("Fix gruntimeerror handling without ownership changes.".to_string());
        source.issues[0].labels.clear();
        source.prs[0].title = "COE-123 harden gruntimeerror handling".to_string();
        source.prs[0].body = Some("No ownership area changed.".to_string());

        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            true,
            false,
        )
        .expect("plan");

        assert!(
            !plan.selected[0]
                .areas
                .contains(&"openhands-runtime".to_string())
        );
    }

    #[test]
    fn capture_index_rolls_back_when_a_later_issue_fails() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let mut source = sample_source();
        source.issues.push(IssueEvidence {
            identifier: "COE-124".to_string(),
            title: "Missing capsule should abort".to_string(),
            url: Some("https://linear.app/example/issue/COE-124".to_string()),
            state: Some("Done".to_string()),
            labels: vec!["runtime".to_string()],
            ..IssueEvidence::default()
        });
        let plan = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string(), "COE-124".to_string()],
                ..IssueSelection::default()
            },
            true,
            false,
        )
        .expect("plan");
        let first_issue = plan
            .selected
            .iter()
            .find(|issue| issue.issue.identifier == "COE-123")
            .expect("first issue should be planned");
        fs::create_dir_all(first_issue.capsule_path.parent().expect("capsule parent"))
            .expect("capsule dir should write");
        fs::write(
            &first_issue.capsule_path,
            render_issue_capsule(&config, first_issue).expect("capsule should render"),
        )
        .expect("first capsule should write");

        let result = index_capture_plan(&config, &plan);

        assert!(
            matches!(result, Err(MemoryError::ReadFile { .. })),
            "missing second capsule should fail indexing: {result:?}",
        );
        assert!(
            load_indexed_issues(&config)
                .expect("index should load")
                .is_empty(),
            "first issue writes should roll back when a later issue fails",
        );
    }

    #[test]
    fn docs_sync_omits_private_capsule_links_for_public_docs() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());
        let source = sample_source();
        let capture = plan_capture(
            &config,
            &source,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            true,
            false,
        )
        .expect("plan");
        write_capture_plan(&config, &capture, false).expect("write capture");

        let docs = plan_docs_sync(
            &config,
            &IssueSelection {
                identifiers: vec!["COE-123".to_string()],
                ..IssueSelection::default()
            },
            false,
            false,
        )
        .expect("docs plan");

        assert_eq!(docs.targets.len(), 1);
        assert!(!docs.targets[0].after.contains(".opensymphony/memory"));
        assert!(docs.targets[0].after.contains("COE-123"));
    }

    #[test]
    fn private_link_guard_allows_tracked_memory_config_path() {
        assert!(!contains_private_memory_link(
            "Commit .opensymphony/memory/memory.yaml"
        ));
        assert!(contains_private_memory_link(
            "See .opensymphony/memory/issues/COE-123.md"
        ));
        assert!(!contains_private_memory_link(
            "Do not publish .opensymphony/memory/memory.duckdb"
        ));
    }

    #[test]
    fn docs_sync_summary_reports_changed_line_counts() {
        let diff = render_diff_stat(
            "alpha\nshared\nold\nomega\n",
            "alpha\nshared\nnew\nomega\n",
            Path::new("docs/topic.md"),
        );

        assert!(diff.contains("docs/topic.md"));
        assert!(diff.contains("4 -> 4 lines"));
        assert!(diff.contains("+1 -1"));
    }

    #[test]
    fn docs_sync_summary_for_new_docs_reports_only_adds() {
        let diff = render_diff_stat("", "alpha\nbeta\n", Path::new("docs/topic.md"));

        assert!(diff.contains("0 -> 2 lines"));
        assert!(diff.contains("+2 -0"));
    }

    #[test]
    fn archive_blocks_missing_memory_unless_forced() {
        let repo = TempDir::new().expect("temp repo");
        let config = config_for(repo.path());

        let blocked = plan_archive(
            &config,
            &[String::from("COE-999")],
            false,
            None,
            false,
            false,
        )
        .expect("archive plan");
        let forced = plan_archive(
            &config,
            &[String::from("COE-999")],
            false,
            None,
            false,
            true,
        )
        .expect("forced archive plan");

        assert!(!blocked.issues[0].eligible);
        assert!(forced.issues[0].eligible);
    }

    #[cfg(unix)]
    #[test]
    fn repo_containment_rejects_symlink_escape() {
        let repo = TempDir::new().expect("temp repo");
        let outside = TempDir::new().expect("outside dir");
        std::os::unix::fs::symlink(outside.path(), repo.path().join("docs"))
            .expect("symlink should be created");

        let result = ensure_repo_contained(repo.path(), &repo.path().join("docs/escape.md"));

        assert!(matches!(result, Err(MemoryError::PathOutsideRepo { .. })));
    }

    #[test]
    fn sanitized_issue_keys_avoid_separator_collisions() {
        assert_ne!(sanitize_issue_key("COE_123"), sanitize_issue_key("COE-123"));
    }

    fn config_for(repo_root: &Path) -> MemoryConfig {
        let config_path = repo_root.join("opensymphony-memory.yaml");
        fs::write(
            &config_path,
            r#"
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-runtime.md
    status: stable
    confidence: 90
    aliases:
      - runtime
      - OpenHands Runtime
    source_refs:
      linear_labels:
        - runtime
"#,
        )
        .expect("config");
        MemoryConfig::load(repo_root, Some(&config_path)).expect("memory config")
    }

    fn sample_source() -> SourceFile {
        SourceFile {
            issues: vec![IssueEvidence {
                identifier: "COE-123".to_string(),
                title: "WebSocket reconnect recovery".to_string(),
                url: Some("https://linear.app/example/issue/COE-123".to_string()),
                description: Some("Recover OpenHands runtime streams after reconnect.".to_string()),
                state: Some("Done".to_string()),
                milestone: Some("M3".to_string()),
                labels: vec!["runtime".to_string()],
                comments: vec![CommentEvidence {
                    body: "Decision: reconcile REST event backlog after readiness.".to_string(),
                    ..CommentEvidence::default()
                }],
                linked_prs: vec![456],
                ..IssueEvidence::default()
            }],
            prs: vec![PullRequestEvidence {
                number: 456,
                title: "COE-123 recover websocket reconnects".to_string(),
                url: Some("https://github.com/example/repo/pull/456".to_string()),
                branch: Some("coe-123-reconnect".to_string()),
                merge_sha: Some("abcdef1234567890".to_string()),
                changed_files: vec![ChangedFileEvidence {
                    path: PathBuf::from("crates/opensymphony-openhands/src/client.rs"),
                    change_kind: Some("modified".to_string()),
                }],
                checks: vec![CheckEvidence {
                    name: "cargo test".to_string(),
                    conclusion: Some("success".to_string()),
                    ..CheckEvidence::default()
                }],
                reviews: vec![ReviewEvidence {
                    reviewer: Some("reviewer".to_string()),
                    state: Some("APPROVED".to_string()),
                    disposition: Some("Reconnect ordering looked correct.".to_string()),
                    ..ReviewEvidence::default()
                }],
                ..PullRequestEvidence::default()
            }],
            ..SourceFile::default()
        }
    }
}
