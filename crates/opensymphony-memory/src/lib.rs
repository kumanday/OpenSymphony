use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use chrono::{DateTime, NaiveDate, Utc};
use duckdb::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const DEFAULT_MEMORY_CONFIG_FILE: &str = "opensymphony-memory.yaml";
pub const FALLBACK_PRIVATE_MEMORY_CONFIG_FILE: &str = ".opensymphony/memory/config.yaml";
pub const DEFAULT_MEMORY_ROOT: &str = ".opensymphony/memory";
pub const DEFAULT_INDEX_FILE_NAME: &str = "memory.duckdb";
pub const DEFAULT_PUBLIC_DOCS_ROOT: &str = "docs";
pub const ISSUE_CAPSULE_BEGIN: &str = "<!-- BEGIN OPENSYMPHONY MANAGED ISSUE CAPSULE -->";
pub const ISSUE_CAPSULE_END: &str = "<!-- END OPENSYMPHONY MANAGED ISSUE CAPSULE -->";
pub const TOPIC_DOC_BEGIN: &str = "<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->";
pub const TOPIC_DOC_END: &str = "<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->";

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
    pub repo_root: PathBuf,
    pub memory_root: PathBuf,
    pub visibility: MemoryVisibility,
    pub index_path: PathBuf,
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
    pub path_hints: Vec<String>,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RedactionConfig {
    pub deny_patterns: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct MemoryConfigFile {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    memory_root: Option<PathBuf>,
    #[serde(default)]
    visibility: Option<MemoryVisibility>,
    #[serde(default)]
    index_path: Option<PathBuf>,
    #[serde(default)]
    source_snapshots: Option<SourceSnapshotPolicy>,
    #[serde(default)]
    markdown_indexes: Option<bool>,
    #[serde(default)]
    docs: Option<DocsConfigFile>,
    #[serde(default)]
    areas: BTreeMap<String, AreaConfigFile>,
    #[serde(default)]
    redaction: Option<RedactionConfigFile>,
}

#[derive(Debug, Default, Deserialize)]
struct DocsConfigFile {
    #[serde(default)]
    public_root: Option<PathBuf>,
    #[serde(default)]
    default_visibility: Option<MemoryVisibility>,
    #[serde(default)]
    deny_private_links: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct AreaConfigFile {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    docs_target: Option<PathBuf>,
    #[serde(default)]
    visibility: Option<MemoryVisibility>,
    #[serde(default)]
    path_hints: Vec<String>,
    #[serde(default)]
    labels: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
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
pub struct CommentEvidence {
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
    pub stale_count: usize,
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

impl MemoryConfig {
    pub fn load(
        repo_root: impl AsRef<Path>,
        config_path: Option<&Path>,
    ) -> Result<Self, MemoryError> {
        let repo_root = normalize_path(repo_root.as_ref());
        let config_file = match config_path {
            Some(path) => Some(resolve_path(&repo_root, path)),
            None => default_config_path(&repo_root),
        };

        let parsed = match config_file {
            Some(path) => {
                let contents = read_to_string(&path)?;
                serde_yaml::from_str::<MemoryConfigFile>(&contents).map_err(|source| {
                    MemoryError::ParseYaml {
                        path: path.clone(),
                        source,
                    }
                })?
            }
            None => MemoryConfigFile::default(),
        };

        let memory_root = resolve_path(
            &repo_root,
            parsed
                .memory_root
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_MEMORY_ROOT)),
        );
        let index_path = parsed
            .index_path
            .as_deref()
            .map(|path| resolve_path(&repo_root, path))
            .unwrap_or_else(|| memory_root.join(DEFAULT_INDEX_FILE_NAME));
        let visibility = parsed.visibility.unwrap_or_default();
        let docs_file = parsed.docs.unwrap_or_default();
        let public_root = resolve_path(
            &repo_root,
            docs_file
                .public_root
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_PUBLIC_DOCS_ROOT)),
        );
        let default_doc_visibility = docs_file
            .default_visibility
            .unwrap_or(MemoryVisibility::Public);
        let mut areas = BTreeMap::new();
        for (slug, area) in parsed.areas {
            let slug = slugify(&slug);
            areas.insert(
                slug.clone(),
                AreaConfig {
                    title: area.title.unwrap_or_else(|| titleize_slug(&slug)),
                    docs_target: area
                        .docs_target
                        .as_deref()
                        .map(|path| resolve_path(&repo_root, path))
                        .unwrap_or_else(|| public_root.join(format!("{slug}.md"))),
                    visibility: area.visibility.unwrap_or(default_doc_visibility),
                    path_hints: normalize_list(area.path_hints),
                    labels: normalize_list(area.labels),
                    slug,
                },
            );
        }

        Ok(Self {
            enabled: parsed.enabled.unwrap_or(true),
            repo_root,
            memory_root,
            visibility,
            index_path,
            source_snapshot_policy: parsed.source_snapshots.unwrap_or_default(),
            markdown_indexes: parsed.markdown_indexes.unwrap_or(true),
            docs: DocsConfig {
                public_root,
                default_visibility: default_doc_visibility,
                deny_private_links: docs_file.deny_private_links.unwrap_or(true),
            },
            areas,
            redaction: parsed
                .redaction
                .map_or_else(RedactionConfig::default, |redaction| RedactionConfig {
                    deny_patterns: normalize_list(redaction.deny_patterns),
                }),
        })
    }

    pub fn issue_capsule_path(&self, issue_key: &str) -> PathBuf {
        self.memory_root
            .join("issues")
            .join(format!("{}.md", sanitize_issue_key(issue_key)))
    }

    pub fn area_or_default(&self, slug: &str) -> AreaConfig {
        let slug = slugify(slug);
        self.areas
            .get(&slug)
            .cloned()
            .unwrap_or_else(|| AreaConfig {
                title: titleize_slug(&slug),
                docs_target: self.docs.public_root.join(format!("{slug}.md")),
                visibility: self.docs.default_visibility,
                path_hints: Vec::new(),
                labels: Vec::new(),
                slug,
            })
    }
}

pub fn load_source_file(path: impl AsRef<Path>) -> Result<SourceFile, MemoryError> {
    let path = path.as_ref();
    let contents = read_to_string(path)?;
    serde_yaml::from_str::<SourceFile>(&contents).map_err(|source| MemoryError::ParseYaml {
        path: path.to_path_buf(),
        source,
    })
}

pub fn plan_capture(
    config: &MemoryConfig,
    source: &SourceFile,
    selection: &IssueSelection,
    write: bool,
    discover_github: bool,
) -> Result<CapturePlan, MemoryError> {
    if !config.enabled {
        return Err(MemoryError::InvalidInput(
            "memory is disabled in configuration".to_string(),
        ));
    }

    let mut selected = select_issues(source, selection);
    let mut warnings = Vec::new();

    if selected.is_empty() && !selection.identifiers.is_empty() {
        selected = selection
            .identifiers
            .iter()
            .map(|identifier| placeholder_issue(identifier))
            .collect();
        warnings.push(
            "no source file issue records matched; using issue identifiers with missing-source warnings"
                .to_string(),
        );
    }

    if selected.is_empty() {
        return Err(MemoryError::InvalidInput(
            "no issues selected for memory capture".to_string(),
        ));
    }

    let mut plans = Vec::new();
    let indexed = load_indexed_issues(config).unwrap_or_default();
    for issue in selected {
        let issue_key = normalize_issue_key(&issue.identifier);
        let mut issue_warnings = Vec::new();
        if issue.title.trim().is_empty() {
            issue_warnings.push("Linear issue title was not available".to_string());
        }
        if issue.url.is_none() {
            issue_warnings.push("Linear issue URL was not available".to_string());
        }

        let mut prs = matched_prs(source, &issue, &issue_key);
        if discover_github {
            match discover_github_prs(&config.repo_root, &issue_key) {
                Ok(discovered) => merge_prs(&mut prs, discovered),
                Err(error) => issue_warnings.push(error),
            }
        }
        if prs.is_empty() {
            issue_warnings.push("no GitHub PR source was matched".to_string());
        }

        let areas = infer_areas(config, source, &issue, &prs);
        let docs_targets = areas
            .iter()
            .map(|area| config.area_or_default(area).docs_target)
            .collect::<Vec<_>>();
        let source_hash = source_hash(&issue, &prs)?;
        let already_captured = indexed
            .iter()
            .any(|indexed| indexed.issue_key.eq_ignore_ascii_case(&issue_key));
        let stale = indexed
            .iter()
            .find(|indexed| indexed.issue_key.eq_ignore_ascii_case(&issue_key))
            .is_some_and(|indexed| indexed.source_hash != source_hash);
        let capsule_path = config.issue_capsule_path(&issue_key);

        plans.push(CaptureIssuePlan {
            issue,
            prs,
            capsule_path,
            areas,
            docs_targets,
            source_hash,
            already_captured,
            stale,
            warnings: issue_warnings,
        });
    }

    plans.sort_by(|left, right| left.issue.identifier.cmp(&right.issue.identifier));

    Ok(CapturePlan {
        write,
        selected: plans,
        warnings,
    })
}

pub fn write_capture_plan(
    config: &MemoryConfig,
    plan: &CapturePlan,
    force: bool,
) -> Result<CaptureWriteReport, MemoryError> {
    let issue_dir = config.memory_root.join("issues");
    create_dir_all(&issue_dir)?;
    create_dir_all(config.index_path.parent().unwrap_or(&config.memory_root))?;

    let mut written_capsules = Vec::new();
    let mut warnings = plan.warnings.clone();
    for issue_plan in &plan.selected {
        let markdown = render_issue_capsule(config, issue_plan)?;
        if issue_plan.capsule_path.exists() {
            let existing = read_to_string(&issue_plan.capsule_path)?;
            if !force && !existing.contains(ISSUE_CAPSULE_BEGIN) {
                return Err(MemoryError::InvalidInput(format!(
                    "{} already exists and does not look generated; rerun with --force to overwrite it",
                    issue_plan.capsule_path.display()
                )));
            }
        }

        write_file(&issue_plan.capsule_path, &markdown)?;
        written_capsules.push(issue_plan.capsule_path.clone());
    }

    index_capture_plan(config, plan)?;
    let markdown_indexes = if config.markdown_indexes {
        write_markdown_indexes(config)?
    } else {
        Vec::new()
    };

    for issue_plan in &plan.selected {
        warnings.extend(issue_plan.warnings.clone());
    }

    Ok(CaptureWriteReport {
        written_capsules,
        index_path: config.index_path.clone(),
        markdown_indexes,
        warnings,
    })
}

pub fn render_capture_dry_run(config: &MemoryConfig, plan: &CapturePlan) -> String {
    let mut output = String::new();
    output.push_str("# Memory Capture Dry Run\n\n");
    output.push_str(&format!(
        "Memory root: {}\n\n",
        display_path(&config.repo_root, &config.memory_root)
    ));
    if plan.selected.is_empty() {
        output.push_str("No issues selected.\n");
        return output;
    }

    output.push_str("## Selected Issues\n\n");
    for issue in &plan.selected {
        output.push_str(&format!(
            "- {}: {}\n",
            issue.issue.identifier,
            issue_title(&issue.issue)
        ));
        output.push_str(&format!(
            "  Capsule: {}\n",
            display_path(&config.repo_root, &issue.capsule_path)
        ));
        output.push_str(&format!(
            "  Linear source: {}\n",
            issue.issue.url.as_deref().unwrap_or("missing")
        ));
        let prs = if issue.prs.is_empty() {
            "none".to_string()
        } else {
            issue
                .prs
                .iter()
                .map(|pr| format!("#{}", pr.number))
                .collect::<Vec<_>>()
                .join(", ")
        };
        output.push_str(&format!("  GitHub PRs: {prs}\n"));
        output.push_str(&format!("  Areas: {}\n", issue.areas.join(", ")));
        output.push_str(&format!(
            "  Docs impact: {}\n",
            issue
                .docs_targets
                .iter()
                .map(|path| display_path(&config.repo_root, path))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        output.push_str(&format!(
            "  Existing capsule: {}\n",
            if issue.already_captured {
                if issue.stale { "stale" } else { "fresh" }
            } else {
                "missing"
            }
        ));
        if !issue.warnings.is_empty() {
            output.push_str("  Warnings:\n");
            for warning in &issue.warnings {
                output.push_str(&format!("  - {warning}\n"));
            }
        }
    }

    if !plan.warnings.is_empty() {
        output.push_str("\n## Plan Warnings\n\n");
        for warning in &plan.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }
    output
}

pub fn brief(config: &MemoryConfig, issue_key: &str) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let indexed = find_indexed_issue(config, &issue_key)?
        .ok_or_else(|| MemoryError::InvalidInput(format!("no capsule found for {issue_key}")))?;
    let mut output = String::new();
    output.push_str(&format!("# {}: {}\n\n", indexed.issue_key, indexed.title));
    output.push_str(&format!(
        "- Capsule: {}\n",
        display_path(&config.repo_root, &indexed.capsule_path)
    ));
    output.push_str(&format!("- Visibility: {}\n", indexed.visibility));
    if !indexed.areas().is_empty() {
        output.push_str(&format!("- Areas: {}\n", indexed.areas().join(", ")));
    }
    output.push('\n');
    output.push_str(&compact_capsule_body(&indexed.body));
    Ok(output)
}

pub fn search(
    config: &MemoryConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let terms = normalize_query_terms(query);
    if terms.is_empty() {
        return Err(MemoryError::InvalidInput(
            "search query must not be empty".to_string(),
        ));
    }

    let mut scored = Vec::new();
    for indexed in load_indexed_issues(config)? {
        let haystack = format!(
            "{} {} {} {}",
            indexed.issue_key,
            indexed.title,
            indexed.labels.join(" "),
            indexed.body
        )
        .to_ascii_lowercase();
        let score = terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        if score > 0 {
            scored.push((
                score,
                SearchResult {
                    issue_key: indexed.issue_key.clone(),
                    title: indexed.title.clone(),
                    capsule_path: indexed.capsule_path.clone(),
                    areas: indexed.areas(),
                    snippet: snippet_for_terms(&indexed.body, &terms),
                },
            ));
        }
    }
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.issue_key.cmp(&right.1.issue_key))
    });
    Ok(scored
        .into_iter()
        .take(limit.max(1))
        .map(|(_, result)| result)
        .collect())
}

pub fn related_by_issue(
    config: &MemoryConfig,
    issue_key: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let indexed = find_indexed_issue(config, &issue_key)?
        .ok_or_else(|| MemoryError::InvalidInput(format!("no capsule found for {issue_key}")))?;
    let mut related = Vec::new();
    let indexed_areas = indexed.areas();
    for candidate in load_indexed_issues(config)? {
        if candidate.issue_key == issue_key {
            continue;
        }
        let candidate_areas = candidate.areas();
        let overlap = candidate_areas
            .iter()
            .filter(|area| indexed_areas.contains(area))
            .count();
        if overlap > 0 {
            related.push((
                overlap,
                SearchResult {
                    issue_key: candidate.issue_key.clone(),
                    title: candidate.title.clone(),
                    capsule_path: candidate.capsule_path.clone(),
                    areas: candidate_areas,
                    snippet: first_interesting_line(&candidate.body),
                },
            ));
        }
    }
    related.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.issue_key.cmp(&right.1.issue_key))
    });
    Ok(related
        .into_iter()
        .take(limit.max(1))
        .map(|(_, result)| result)
        .collect())
}

pub fn related_by_area(
    config: &MemoryConfig,
    area: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let area = slugify(area);
    let mut results = Vec::new();
    for candidate in load_indexed_issues(config)? {
        let areas = candidate.areas();
        if areas.iter().any(|candidate_area| candidate_area == &area) {
            results.push(SearchResult {
                issue_key: candidate.issue_key.clone(),
                title: candidate.title.clone(),
                capsule_path: candidate.capsule_path.clone(),
                areas,
                snippet: first_interesting_line(&candidate.body),
            });
        }
    }
    results.sort_by(|left, right| left.issue_key.cmp(&right.issue_key));
    results.truncate(limit.max(1));
    Ok(results)
}

pub fn related_by_paths(
    config: &MemoryConfig,
    paths: &[PathBuf],
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let terms = paths
        .iter()
        .flat_map(|path| {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .filter_map(|value| normalize_optional(&value))
        .collect::<Vec<_>>();
    search(config, &terms.join(" "), limit)
}

pub fn docs_for_area(config: &MemoryConfig, area: &str) -> Result<String, MemoryError> {
    let area = config.area_or_default(area);
    if !area.docs_target.exists() {
        return Err(MemoryError::InvalidInput(format!(
            "no topic doc exists for area `{}` at {}",
            area.slug,
            area.docs_target.display()
        )));
    }
    read_to_string(&area.docs_target)
}

pub fn context_for_issue(
    config: &MemoryConfig,
    source: &SourceFile,
    issue_key: &str,
    limit: usize,
) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let mut output = String::new();
    output.push_str(&format!("# Memory Context: {issue_key}\n\n"));
    if let Some(issue) = source
        .issues
        .iter()
        .find(|issue| normalize_issue_key(&issue.identifier) == issue_key)
    {
        output.push_str(&format!("## Current Issue\n\n{}\n\n", issue_title(issue)));
        if let Some(description) = issue.description.as_deref().and_then(normalize_optional) {
            output.push_str(&format!("{}\n\n", summarize_text(&description, 600)));
        }
    }

    let query = source
        .issues
        .iter()
        .find(|issue| normalize_issue_key(&issue.identifier) == issue_key)
        .map(|issue| {
            format!(
                "{} {} {}",
                issue.title,
                issue.labels.join(" "),
                issue.description.clone().unwrap_or_default()
            )
        })
        .unwrap_or_else(|| issue_key.clone());
    let results = search(config, &query, limit).unwrap_or_default();
    output.push_str("## Related Memory\n\n");
    if results.is_empty() {
        output.push_str("- No related captured memory found.\n");
    } else {
        for result in results {
            output.push_str(&format!(
                "- {}: {} ({})\n",
                result.issue_key,
                result.title,
                result.areas.join(", ")
            ));
        }
    }
    output.push_str("\n## Guidance\n\n");
    output.push_str("- Treat memory as context, not as authority over current code.\n");
    output.push_str("- Inspect the referenced docs and current files before editing.\n");
    output.push_str("- Use `opensymphony debug ");
    output.push_str(&issue_key);
    output.push_str("` only when the original agent conversation is needed.\n");
    Ok(output)
}

pub fn status(
    config: &MemoryConfig,
    selection: &IssueSelection,
) -> Result<StatusReport, MemoryError> {
    let mut issues = load_indexed_issues(config)?;
    if let Some(area) = selection.area.as_ref().map(|area| slugify(area)) {
        issues.retain(|issue| issue.areas().contains(&area));
    }
    if let Some(milestone) = selection
        .milestone
        .as_ref()
        .and_then(|value| normalize_optional(value))
    {
        issues.retain(|issue| issue.milestone.as_deref() == Some(milestone.as_str()));
    }

    let stale_count = issues
        .iter()
        .filter(|issue| issue.docs_sync_status == "pending")
        .count();
    let warning_count = issues.iter().map(|issue| issue.warning_count).sum();
    let docs_pending_count = issues
        .iter()
        .filter(|issue| issue.docs_sync_status == "pending")
        .count();
    let status_issues = issues
        .into_iter()
        .map(|issue| {
            let areas = issue.areas();
            StatusIssue {
                issue_key: issue.issue_key,
                title: issue.title,
                state: issue.state,
                milestone: issue.milestone,
                capsule_path: issue.capsule_path,
                visibility: issue.visibility,
                areas,
                docs_sync_status: issue.docs_sync_status,
                warning_count: issue.warning_count,
            }
        })
        .collect::<Vec<_>>();

    Ok(StatusReport {
        issue_count: status_issues.len(),
        stale_count,
        warning_count,
        docs_pending_count,
        issues: status_issues,
    })
}

pub fn lint(config: &MemoryConfig, public_docs: bool) -> Result<LintReport, MemoryError> {
    let mut findings = Vec::new();
    let issues = load_indexed_issues(config).unwrap_or_default();
    for issue in &issues {
        if issue.warning_count > 0 {
            findings.push(LintFinding {
                severity: LintSeverity::Warn,
                path: Some(issue.capsule_path.clone()),
                message: format!(
                    "{} has {} unresolved capture warning(s)",
                    issue.issue_key, issue.warning_count
                ),
                next_command: Some(format!("opensymphony memory show {}", issue.issue_key)),
            });
        }
        if issue.areas().is_empty() {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                path: Some(issue.capsule_path.clone()),
                message: format!("{} has no area mapping", issue.issue_key),
                next_command: Some(format!(
                    "opensymphony memory capture {} --write --force",
                    issue.issue_key
                )),
            });
        }
    }

    if public_docs && config.docs.deny_private_links {
        for area in all_known_areas(config, &issues) {
            let path = area.docs_target;
            if !path.exists() {
                continue;
            }
            let contents = read_to_string(&path)?;
            if contains_private_memory_link(&contents) {
                findings.push(LintFinding {
                    severity: LintSeverity::Error,
                    path: Some(path),
                    message: "public docs contain a private memory path".to_string(),
                    next_command: Some("opensymphony memory sync-docs --dry-run".to_string()),
                });
            }
        }
    }

    Ok(LintReport { findings })
}

pub fn plan_docs_sync(
    config: &MemoryConfig,
    selection: &IssueSelection,
    write: bool,
    with_diagrams: bool,
) -> Result<DocsSyncPlan, MemoryError> {
    let selected = select_indexed_issues_for_docs(config, selection)?;
    if selected.is_empty() {
        return Err(MemoryError::InvalidInput(
            "no captured issues selected for docs sync".to_string(),
        ));
    }

    let mut by_area: BTreeMap<String, Vec<IndexedIssue>> = BTreeMap::new();
    for issue in selected {
        for area in issue.areas() {
            if selection
                .area
                .as_ref()
                .is_some_and(|selected_area| slugify(selected_area) != area)
            {
                continue;
            }
            by_area.entry(area).or_default().push(issue.clone());
        }
    }

    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    for (area_slug, issues) in by_area {
        let area = config.area_or_default(&area_slug);
        let before = if area.docs_target.exists() {
            Some(read_to_string(&area.docs_target)?)
        } else {
            None
        };
        let after = render_topic_doc(config, &area, &issues, before.as_deref(), with_diagrams);
        if area.visibility == MemoryVisibility::Public
            && config.docs.deny_private_links
            && contains_private_memory_link(&after)
        {
            warnings.push(format!(
                "{} would contain private memory links",
                display_path(&config.repo_root, &area.docs_target)
            ));
        }
        let diff = render_diff(before.as_deref().unwrap_or(""), &after, &area.docs_target);
        targets.push(DocsTargetPlan {
            area: area.slug,
            title: area.title,
            path: area.docs_target,
            visibility: area.visibility,
            create: before.is_none(),
            before,
            after,
            diff,
            issue_keys: issues.into_iter().map(|issue| issue.issue_key).collect(),
        });
    }

    let selected_issue_keys = targets
        .iter()
        .flat_map(|target| target.issue_keys.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    Ok(DocsSyncPlan {
        write,
        selected_issue_keys,
        targets,
        warnings,
    })
}

pub fn write_docs_sync_plan(
    config: &MemoryConfig,
    plan: &DocsSyncPlan,
) -> Result<Vec<PathBuf>, MemoryError> {
    let mut written = Vec::new();
    for target in &plan.targets {
        ensure_repo_contained(&config.repo_root, &target.path)?;
        write_file(&target.path, &target.after)?;
        written.push(target.path.clone());
    }
    mark_docs_synced(config, plan)?;
    Ok(written)
}

pub fn plan_archive(
    config: &MemoryConfig,
    identifiers: &[String],
    from_memory: bool,
    state: Option<&str>,
    write: bool,
    force: bool,
) -> Result<ArchivePlan, MemoryError> {
    let issues = load_indexed_issues(config).unwrap_or_default();
    let mut selected_keys = identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    if from_memory {
        for issue in &issues {
            if state.is_none_or(|state| archive_state_matches(issue, state)) {
                selected_keys.insert(issue.issue_key.clone());
            }
        }
    }
    if selected_keys.is_empty() {
        return Err(MemoryError::InvalidInput(
            "no Linear issues selected for archive".to_string(),
        ));
    }

    let mut plans = Vec::new();
    let mut warnings = Vec::new();
    for issue_key in selected_keys {
        let indexed = issues
            .iter()
            .find(|issue| issue.issue_key == issue_key)
            .cloned();
        let (eligible, reason, capsule_path) = match indexed {
            Some(issue) if force => (
                true,
                "eligible because --force bypasses capture freshness checks".to_string(),
                Some(issue.capsule_path),
            ),
            Some(issue) if issue.warning_count == 0 => (
                true,
                "eligible: fresh captured memory exists with no unresolved warnings".to_string(),
                Some(issue.capsule_path),
            ),
            Some(issue) => (
                false,
                format!(
                    "blocked: captured memory has {} unresolved warning(s); rerun capture or use --force",
                    issue.warning_count
                ),
                Some(issue.capsule_path),
            ),
            None if force => (
                true,
                "eligible because --force bypasses missing memory checks".to_string(),
                None,
            ),
            None => (
                false,
                "blocked: no captured memory found; run `opensymphony memory capture --write` first"
                    .to_string(),
                None,
            ),
        };
        if !eligible {
            warnings.push(format!("{issue_key}: {reason}"));
        }
        plans.push(ArchiveIssuePlan {
            issue_key,
            eligible,
            reason,
            capsule_path,
        });
    }

    Ok(ArchivePlan {
        write,
        force,
        issues: plans,
        warnings,
    })
}

fn archive_state_matches(issue: &IndexedIssue, state: &str) -> bool {
    let state = state.trim();
    state.eq_ignore_ascii_case("captured")
        || issue.docs_sync_status.eq_ignore_ascii_case(state)
        || issue
            .state
            .as_deref()
            .is_some_and(|issue_state| issue_state.eq_ignore_ascii_case(state))
}

pub fn render_archive_plan(config: &MemoryConfig, plan: &ArchivePlan) -> String {
    let mut output = String::new();
    output.push_str("# Linear Archive Dry Run\n\n");
    for issue in &plan.issues {
        output.push_str(&format!(
            "- {}: {} ({})\n",
            issue.issue_key,
            if issue.eligible {
                "eligible"
            } else {
                "blocked"
            },
            issue.reason
        ));
        if let Some(path) = &issue.capsule_path {
            output.push_str(&format!(
                "  Capsule: {}\n",
                display_path(&config.repo_root, path)
            ));
        }
    }
    if !plan.warnings.is_empty() {
        output.push_str("\n## Warnings\n\n");
        for warning in &plan.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }
    output
}

pub fn mark_archived(config: &MemoryConfig, issue_keys: &[String]) -> Result<(), MemoryError> {
    if !config.index_path.exists() {
        return Ok(());
    }
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    for issue_key in issue_keys {
        connection
            .execute(
                "UPDATE issues SET archive_status = 'archived' WHERE issue_key = ?",
                params![normalize_issue_key(issue_key)],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    Ok(())
}

pub fn expand_issue_range(range: &str) -> Result<Vec<String>, MemoryError> {
    let Some((start, end)) = range.split_once("..") else {
        return Err(MemoryError::InvalidInput(format!(
            "issue range `{range}` must look like COE-100..COE-199"
        )));
    };
    let (start_prefix, start_number) = split_issue_key(start)?;
    let (end_prefix, end_number) = split_issue_key(end)?;
    if start_prefix != end_prefix {
        return Err(MemoryError::InvalidInput(format!(
            "issue range `{range}` must use the same prefix on both ends"
        )));
    }
    if start_number > end_number {
        return Err(MemoryError::InvalidInput(format!(
            "issue range `{range}` must be ascending"
        )));
    }
    Ok((start_number..=end_number)
        .map(|number| format!("{start_prefix}-{number}"))
        .collect())
}

impl IndexedIssue {
    fn areas(&self) -> Vec<String> {
        self.areas.clone()
    }
}

fn default_config_path(repo_root: &Path) -> Option<PathBuf> {
    let public = repo_root.join(DEFAULT_MEMORY_CONFIG_FILE);
    if public.exists() {
        return Some(public);
    }
    let private = repo_root.join(FALLBACK_PRIVATE_MEMORY_CONFIG_FILE);
    if private.exists() {
        Some(private)
    } else {
        None
    }
}

fn select_issues(source: &SourceFile, selection: &IssueSelection) -> Vec<IssueEvidence> {
    let selected_identifiers = selection
        .identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    let mut issues = source.issues.clone();
    issues.retain(|issue| {
        let issue_key = normalize_issue_key(&issue.identifier);
        if !selected_identifiers.is_empty() && !selected_identifiers.contains(&issue_key) {
            return false;
        }
        if let Some(milestone) = selection
            .milestone
            .as_ref()
            .and_then(|value| normalize_optional(value))
            && issue.milestone.as_deref() != Some(milestone.as_str())
        {
            return false;
        }
        if let Some(state) = selection
            .state
            .as_ref()
            .and_then(|value| normalize_optional(value))
            && issue
                .state
                .as_deref()
                .is_none_or(|issue_state| !issue_state.eq_ignore_ascii_case(&state))
        {
            return false;
        }
        if let Some(before_date) = selection.before_date {
            let issue_date = issue
                .completed_at
                .or(issue.updated_at)
                .map(|timestamp| timestamp.date_naive());
            if issue_date.is_none_or(|date| date >= before_date) {
                return false;
            }
        }
        if let Some(before_issue) = selection.before_issue.as_deref()
            && !issue_is_before(&issue_key, before_issue)
        {
            return false;
        }
        true
    });
    issues.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    issues
}

fn matched_prs(
    source: &SourceFile,
    issue: &IssueEvidence,
    issue_key: &str,
) -> Vec<PullRequestEvidence> {
    let override_prs = source
        .overrides
        .get(issue_key)
        .or_else(|| source.overrides.get(&issue.identifier))
        .map(|override_record| override_record.prs.clone())
        .unwrap_or_default();
    let explicit = issue
        .linked_prs
        .iter()
        .chain(override_prs.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut prs = source
        .prs
        .iter()
        .filter(|pr| {
            explicit.contains(&pr.number)
                || contains_issue_key(&pr.title, issue_key)
                || pr
                    .body
                    .as_deref()
                    .is_some_and(|body| contains_issue_key(body, issue_key))
                || pr
                    .branch
                    .as_deref()
                    .is_some_and(|branch| contains_issue_key(branch, issue_key))
        })
        .cloned()
        .collect::<Vec<_>>();
    prs.sort_by_key(|pr| pr.number);
    prs.dedup_by_key(|pr| pr.number);
    prs
}

fn merge_prs(target: &mut Vec<PullRequestEvidence>, incoming: Vec<PullRequestEvidence>) {
    for pr in incoming {
        if !target.iter().any(|existing| existing.number == pr.number) {
            target.push(pr);
        }
    }
    target.sort_by_key(|pr| pr.number);
}

fn infer_areas(
    config: &MemoryConfig,
    source: &SourceFile,
    issue: &IssueEvidence,
    prs: &[PullRequestEvidence],
) -> Vec<String> {
    let issue_key = normalize_issue_key(&issue.identifier);
    if let Some(overrides) = source
        .overrides
        .get(&issue_key)
        .or_else(|| source.overrides.get(&issue.identifier))
        && !overrides.areas.is_empty()
    {
        return normalize_list(overrides.areas.clone())
            .into_iter()
            .map(|area| slugify(&area))
            .collect();
    }

    let mut areas = BTreeSet::new();
    let labels = normalize_list(issue.labels.clone());
    for (slug, area) in &config.areas {
        if labels.iter().any(|label| area.labels.contains(label)) {
            areas.insert(slug.clone());
        }
    }
    let changed_files = prs
        .iter()
        .flat_map(|pr| pr.changed_files.iter())
        .map(|file| file.path.to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();
    for (slug, area) in &config.areas {
        if area.path_hints.iter().any(|hint| {
            changed_files
                .iter()
                .any(|file| file.contains(&hint.to_ascii_lowercase()))
        }) {
            areas.insert(slug.clone());
        }
    }

    let configured_labels = config
        .areas
        .values()
        .flat_map(|area| area.labels.iter().cloned())
        .collect::<BTreeSet<_>>();
    for label in labels {
        if label != "done" && label != "bug" && label != "feature" {
            let label_slug = slugify(&label);
            if !configured_labels.contains(&label) && !areas.contains(&label_slug) {
                areas.insert(label_slug);
            }
        }
    }

    if areas.is_empty() {
        let first_path_area = prs
            .iter()
            .flat_map(|pr| pr.changed_files.iter())
            .find_map(|file| file.path.components().next())
            .map(|component| slugify(&component.as_os_str().to_string_lossy()));
        areas.insert(first_path_area.unwrap_or_else(|| "general".to_string()));
    }

    areas.into_iter().collect()
}

fn source_hash(issue: &IssueEvidence, prs: &[PullRequestEvidence]) -> Result<String, MemoryError> {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(issue)?);
    hasher.update(serde_json::to_vec(prs)?);
    Ok(format!("{:x}", hasher.finalize()))
}

fn render_issue_capsule(
    config: &MemoryConfig,
    plan: &CaptureIssuePlan,
) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(&plan.issue.identifier);
    let frontmatter = IssueCapsuleFrontmatter {
        capsule_type: "issue-capsule",
        visibility: config.visibility,
        issue: issue_key.clone(),
        title: issue_title(&plan.issue),
        state: plan.issue.state.clone(),
        milestone: plan.issue.milestone.clone(),
        linear_url: plan.issue.url.clone(),
        prs: plan
            .prs
            .iter()
            .map(|pr| CapsulePr {
                number: pr.number,
                url: pr.url.clone(),
                merge_sha: pr.merge_sha.clone(),
            })
            .collect(),
        areas: plan.areas.clone(),
        source_refs: SourceRefs {
            linear_issue: plan
                .issue
                .url
                .as_ref()
                .map(|_| format!("linear:{issue_key}")),
            github_prs: plan
                .prs
                .iter()
                .map(|pr| format!("github:pr:{}", pr.number))
                .collect(),
        },
        captured_at: Utc::now(),
        docs_sync: DocsSyncFrontmatter {
            status: "pending".to_string(),
        },
    };
    let frontmatter =
        serde_yaml::to_string(&frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: plan.capsule_path.clone(),
            source,
        })?;

    let mut markdown = String::new();
    markdown.push_str("---\n");
    markdown.push_str(&frontmatter);
    markdown.push_str("---\n\n");
    markdown.push_str(ISSUE_CAPSULE_BEGIN);
    markdown.push_str("\n\n");
    markdown.push_str(&format!("# {issue_key}: {}\n\n", issue_title(&plan.issue)));
    markdown.push_str("## Original intent\n\n");
    markdown.push_str(&render_original_intent(&plan.issue));
    markdown.push_str("\n\n## Outcome\n\n");
    markdown.push_str(&render_outcome(plan));
    markdown.push_str("\n\n## Decisions and actions\n\n");
    markdown.push_str(&render_decisions(plan));
    markdown.push_str("\n\n## Validation evidence\n\n");
    markdown.push_str(&render_validation(plan));
    markdown.push_str("\n\n## Review and rework\n\n");
    markdown.push_str(&render_reviews(plan));
    markdown.push_str("\n\n## Follow-ups and risks\n\n");
    markdown.push_str(&render_followups(plan));
    markdown.push_str("\n\n## Documentation impact\n\n");
    for target in &plan.docs_targets {
        markdown.push_str(&format!("- {}\n", display_path(&config.repo_root, target)));
    }
    if !plan.warnings.is_empty() {
        markdown.push_str("\n## Capture warnings\n\n");
        for warning in &plan.warnings {
            markdown.push_str(&format!("- {warning}\n"));
        }
    }
    markdown.push_str("\n## Provenance\n\n");
    match &plan.issue.url {
        Some(url) => markdown.push_str(&format!("- Linear: {url}\n")),
        None => markdown.push_str(&format!("- Linear: {issue_key}\n")),
    }
    for pr in &plan.prs {
        let label = pr.url.as_deref().map_or_else(
            || format!("#{}", pr.number),
            |url| format!("[#{}]({url})", pr.number),
        );
        markdown.push_str(&format!("- PR: {label}\n"));
    }
    markdown.push_str(&format!("- Debug: `opensymphony debug {issue_key}`\n"));
    markdown.push('\n');
    markdown.push_str(ISSUE_CAPSULE_END);
    markdown.push('\n');

    Ok(markdown)
}

#[derive(Debug, Serialize)]
struct IssueCapsuleFrontmatter {
    #[serde(rename = "type")]
    capsule_type: &'static str,
    visibility: MemoryVisibility,
    issue: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    milestone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_url: Option<String>,
    prs: Vec<CapsulePr>,
    areas: Vec<String>,
    source_refs: SourceRefs,
    captured_at: DateTime<Utc>,
    docs_sync: DocsSyncFrontmatter,
}

#[derive(Debug, Serialize)]
struct CapsulePr {
    number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_sha: Option<String>,
}

#[derive(Debug, Serialize)]
struct SourceRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_issue: Option<String>,
    github_prs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DocsSyncFrontmatter {
    status: String,
}

fn render_original_intent(issue: &IssueEvidence) -> String {
    issue.description.as_deref().map_or_else(
        || "- Source issue description was not available.".to_string(),
        |description| summarize_markdown(description, 900),
    )
}

fn render_outcome(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    if plan.prs.is_empty() {
        lines.push("- No merged PR source was matched during capture.".to_string());
    } else {
        for pr in &plan.prs {
            let mut line = format!(
                "- PR #{}: {}",
                pr.number,
                fallback_title(&pr.title, "untitled PR")
            );
            if let Some(sha) = pr.merge_sha.as_deref() {
                line.push_str(&format!(" (merge `{}`)", short_sha(sha)));
            }
            lines.push(line);
        }
    }
    let changed_files = plan
        .prs
        .iter()
        .flat_map(|pr| pr.changed_files.iter())
        .take(8)
        .map(|file| format!("  - {}", file.path.display()))
        .collect::<Vec<_>>();
    if !changed_files.is_empty() {
        lines.push("- Notable changed files:".to_string());
        lines.extend(changed_files);
    }
    lines.join("\n")
}

fn render_decisions(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    for comment in &plan.issue.comments {
        if should_copy_comment_summary(&comment.body) {
            lines.push(format!("- {}", summarize_text(&comment.body, 260)));
        }
    }
    for pr in &plan.prs {
        if let Some(body) = pr.body.as_deref().and_then(normalize_optional) {
            lines.push(format!(
                "- PR #{} summary: {}",
                pr.number,
                summarize_text(&body, 260)
            ));
        }
    }
    if lines.is_empty() {
        lines.push("- No explicit decision notes were found in source evidence.".to_string());
    }
    lines.join("\n")
}

fn render_validation(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    for pr in &plan.prs {
        for check in &pr.checks {
            lines.push(format!(
                "- PR #{} `{}`: {}",
                pr.number,
                check.name,
                check.conclusion.as_deref().unwrap_or("unknown")
            ));
        }
    }
    if lines.is_empty() {
        lines.push("- No check summary source was found.".to_string());
    }
    lines.join("\n")
}

fn render_reviews(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    for pr in &plan.prs {
        for review in &pr.reviews {
            let reviewer = review.reviewer.as_deref().unwrap_or("reviewer");
            let state = review.state.as_deref().unwrap_or("reviewed");
            let disposition = review
                .disposition
                .as_deref()
                .map(|value| format!(": {}", summarize_text(value, 180)))
                .unwrap_or_default();
            lines.push(format!(
                "- PR #{} {reviewer} {state}{disposition}",
                pr.number
            ));
        }
    }
    if lines.is_empty() {
        lines.push("- No review summary source was found.".to_string());
    }
    lines.join("\n")
}

fn render_followups(plan: &CaptureIssuePlan) -> String {
    let followups = plan
        .issue
        .comments
        .iter()
        .filter(|comment| {
            let body = comment.body.to_ascii_lowercase();
            body.contains("follow-up") || body.contains("follow up") || body.contains("risk")
        })
        .map(|comment| format!("- {}", summarize_text(&comment.body, 240)))
        .collect::<Vec<_>>();
    if followups.is_empty() {
        "- No unresolved follow-ups were identified during capture.".to_string()
    } else {
        followups.join("\n")
    }
}

fn index_capture_plan(config: &MemoryConfig, plan: &CapturePlan) -> Result<(), MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    for issue_plan in &plan.selected {
        let issue_key = normalize_issue_key(&issue_plan.issue.identifier);
        let body = read_to_string(&issue_plan.capsule_path)?;
        let labels_json = serde_json::to_string(&issue_plan.issue.labels)?;
        connection
            .execute("DELETE FROM issues WHERE issue_key = ?", params![issue_key])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, state, milestone, labels_json, completion_time, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    issue_key,
                    issue_title(&issue_plan.issue),
                    issue_plan.issue.state.clone(),
                    issue_plan.issue.milestone.clone(),
                    labels_json,
                    issue_plan
                        .issue
                        .completed_at
                        .or(issue_plan.issue.updated_at)
                        .map(|value| value.to_rfc3339()),
                    "not_archived",
                    issue_plan.capsule_path.to_string_lossy().to_string(),
                    config.visibility.as_str(),
                    issue_plan.source_hash.clone(),
                    issue_plan.warnings.len() as i64,
                    "pending",
                    body,
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;

        connection
            .execute(
                "DELETE FROM issue_areas WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for area in &issue_plan.areas {
            connection
                .execute(
                    "INSERT INTO issue_areas (issue_key, area) VALUES (?, ?)",
                    params![issue_key, area],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }

        connection
            .execute(
                "DELETE FROM pull_requests WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        connection
            .execute(
                "DELETE FROM changed_files WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        connection
            .execute("DELETE FROM checks WHERE issue_key = ?", params![issue_key])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        connection
            .execute(
                "DELETE FROM reviews WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;

        for pr in &issue_plan.prs {
            connection
                .execute(
                    "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        issue_key,
                        pr.number as i64,
                        pr.title.clone(),
                        pr.url.clone(),
                        pr.branch.clone(),
                        pr.merge_sha.clone(),
                        pr.merged_at.map(|value| value.to_rfc3339()),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            for file in &pr.changed_files {
                connection
                    .execute(
                        "INSERT INTO changed_files (issue_key, pr_number, file_path, change_kind) VALUES (?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            file.path.to_string_lossy().to_string(),
                            file.change_kind.clone(),
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
            for check in &pr.checks {
                connection
                    .execute(
                        "INSERT INTO checks (issue_key, pr_number, name, conclusion, completed_at) VALUES (?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            check.name.clone(),
                            check.conclusion.clone(),
                            check.completed_at.map(|value| value.to_rfc3339()),
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
            for review in &pr.reviews {
                connection
                    .execute(
                        "INSERT INTO reviews (issue_key, pr_number, reviewer, state, submitted_at, disposition) VALUES (?, ?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            review.reviewer.clone(),
                            review.state.clone(),
                            review.submitted_at.map(|value| value.to_rfc3339()),
                            review.disposition.clone(),
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }

        for area in &issue_plan.areas {
            let area_config = config.area_or_default(area);
            connection
                .execute("DELETE FROM areas WHERE area = ?", params![area])
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            connection
                .execute(
                    "INSERT INTO areas (area, display_name, docs_target) VALUES (?, ?, ?)",
                    params![
                        area,
                        area_config.title,
                        area_config.docs_target.to_string_lossy().to_string(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
    }

    Ok(())
}

fn open_index(config: &MemoryConfig) -> Result<Connection, MemoryError> {
    if let Some(parent) = config.index_path.parent() {
        create_dir_all(parent)?;
    }
    Connection::open(&config.index_path).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })
}

fn migrate_index(connection: &Connection) -> Result<(), duckdb::Error> {
    connection.execute_batch(
        r#"
CREATE TABLE IF NOT EXISTS issues (
  issue_key TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  state TEXT,
  milestone TEXT,
  labels_json TEXT NOT NULL,
  completion_time TEXT,
  archive_status TEXT NOT NULL,
  capsule_path TEXT NOT NULL,
  visibility TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  warning_count BIGINT NOT NULL,
  docs_sync_status TEXT NOT NULL,
  body TEXT NOT NULL,
  captured_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS pull_requests (
  issue_key TEXT NOT NULL,
  number BIGINT NOT NULL,
  title TEXT NOT NULL,
  url TEXT,
  branch TEXT,
  merge_sha TEXT,
  merged_at TEXT
);
CREATE TABLE IF NOT EXISTS changed_files (
  issue_key TEXT NOT NULL,
  pr_number BIGINT NOT NULL,
  file_path TEXT NOT NULL,
  change_kind TEXT
);
CREATE TABLE IF NOT EXISTS checks (
  issue_key TEXT NOT NULL,
  pr_number BIGINT NOT NULL,
  name TEXT NOT NULL,
  conclusion TEXT,
  completed_at TEXT
);
CREATE TABLE IF NOT EXISTS reviews (
  issue_key TEXT NOT NULL,
  pr_number BIGINT NOT NULL,
  reviewer TEXT,
  state TEXT,
  submitted_at TEXT,
  disposition TEXT
);
CREATE TABLE IF NOT EXISTS areas (
  area TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  docs_target TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS issue_areas (
  issue_key TEXT NOT NULL,
  area TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS doc_sync_runs (
  run_id TEXT PRIMARY KEY,
  selected_issues_json TEXT NOT NULL,
  target_docs_json TEXT NOT NULL,
  generated_at TEXT NOT NULL,
  status TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS doc_memory_links (
  topic_doc TEXT NOT NULL,
  issue_key TEXT NOT NULL,
  visibility TEXT NOT NULL
);
"#,
    )
}

fn load_indexed_issues(config: &MemoryConfig) -> Result<Vec<IndexedIssue>, MemoryError> {
    if !config.index_path.exists() {
        return Ok(Vec::new());
    }
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;

    let mut statement = connection
        .prepare(
            "SELECT issue_key, title, state, milestone, labels_json, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body FROM issues ORDER BY issue_key",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map([], |row| {
            let labels_json: String = row.get(4)?;
            Ok(IndexedIssue {
                issue_key: row.get(0)?,
                title: row.get(1)?,
                state: row.get(2)?,
                milestone: row.get(3)?,
                labels: serde_json::from_str::<Vec<String>>(&labels_json).unwrap_or_default(),
                areas: Vec::new(),
                capsule_path: PathBuf::from(row.get::<_, String>(5)?),
                visibility: match row.get::<_, String>(6)?.as_str() {
                    "public" => MemoryVisibility::Public,
                    _ => MemoryVisibility::Private,
                },
                source_hash: row.get(7)?,
                warning_count: row.get::<_, i64>(8)? as usize,
                docs_sync_status: row.get(9)?,
                body: row.get(10)?,
            })
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;

    let mut issues = Vec::new();
    for row in rows {
        issues.push(row.map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?);
    }
    drop(statement);

    for issue in &mut issues {
        issue.areas = load_issue_areas(&connection, &issue.issue_key).map_err(|source| {
            MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            }
        })?;
    }
    Ok(issues)
}

fn load_issue_areas(
    connection: &Connection,
    issue_key: &str,
) -> Result<Vec<String>, duckdb::Error> {
    let mut statement =
        connection.prepare("SELECT area FROM issue_areas WHERE issue_key = ? ORDER BY area")?;
    let rows = statement.query_map(params![issue_key], |row| row.get::<_, String>(0))?;
    let mut areas = Vec::new();
    for row in rows {
        areas.push(row?);
    }
    Ok(areas)
}

fn find_indexed_issue(
    config: &MemoryConfig,
    issue_key: &str,
) -> Result<Option<IndexedIssue>, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    Ok(load_indexed_issues(config)?
        .into_iter()
        .find(|issue| issue.issue_key == issue_key))
}

fn write_markdown_indexes(config: &MemoryConfig) -> Result<Vec<PathBuf>, MemoryError> {
    create_dir_all(&config.memory_root.join("indexes"))?;
    let issues = load_indexed_issues(config)?;
    let index_path = config.memory_root.join("indexes/index.md");
    let log_path = config.memory_root.join("indexes/log.md");

    let mut index = String::new();
    index.push_str("# OpenSymphony Memory Index\n\n");
    for issue in &issues {
        index.push_str(&format!(
            "- [{}: {}]({}) ({})\n",
            issue.issue_key,
            issue.title,
            path_relative_to(&config.memory_root, &issue.capsule_path),
            issue.areas().join(", ")
        ));
    }
    write_file(&index_path, &index)?;

    let mut log = String::new();
    log.push_str("# OpenSymphony Memory Log\n\n");
    for issue in issues.iter().rev() {
        log.push_str(&format!(
            "- {}: {} [{}]\n",
            issue.issue_key, issue.title, issue.docs_sync_status
        ));
    }
    write_file(&log_path, &log)?;

    Ok(vec![index_path, log_path])
}

fn select_indexed_issues_for_docs(
    config: &MemoryConfig,
    selection: &IssueSelection,
) -> Result<Vec<IndexedIssue>, MemoryError> {
    let mut issues = load_indexed_issues(config)?;
    let selected_identifiers = selection
        .identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    if !selected_identifiers.is_empty() {
        issues.retain(|issue| selected_identifiers.contains(&issue.issue_key));
    }
    if selection.since_last_sync {
        issues.retain(|issue| issue.docs_sync_status == "pending");
    }
    if let Some(area) = selection.area.as_ref().map(|area| slugify(area)) {
        issues.retain(|issue| issue.areas().contains(&area));
    }
    Ok(issues)
}

fn render_topic_doc(
    config: &MemoryConfig,
    area: &AreaConfig,
    issues: &[IndexedIssue],
    before: Option<&str>,
    with_diagrams: bool,
) -> String {
    let frontmatter = format!(
        "---\ntype: topic-doc\narea: {}\nvisibility: {}\nlast_memory_sync: {}\n---\n\n",
        area.slug,
        area.visibility,
        Utc::now().to_rfc3339()
    );
    let mut managed = String::new();
    managed.push_str(TOPIC_DOC_BEGIN);
    managed.push_str("\n\n");
    managed.push_str("## Current model\n\n");
    managed.push_str(&current_model_from_issues(issues));
    managed.push_str("\n\n## Important invariants\n\n");
    managed.push_str(&invariants_from_issues(issues));
    managed.push_str("\n\n## Operational flow\n\n");
    if with_diagrams {
        managed.push_str(&format!(
            "```mermaid\nflowchart TD\n  memory[\"Captured issue memory\"] --> area[\"{}\"]\n  area --> docs[\"{}\"]\n```\n",
            area.title,
            display_path(&config.repo_root, &area.docs_target)
        ));
    } else {
        managed.push_str("- No generated diagram requested for this sync.\n");
    }
    managed.push_str("\n## Known gotchas\n\n");
    managed.push_str(&gotchas_from_issues(issues));
    managed.push_str("\n\n## Recent changes\n\n");
    for issue in issues {
        managed.push_str(&format!("- {}: {}\n", issue.issue_key, issue.title));
    }
    managed.push_str("\n## Provenance\n\n");
    for issue in issues {
        managed.push_str(&format!("- {}\n", issue.issue_key));
    }
    managed.push('\n');
    managed.push_str(TOPIC_DOC_END);
    managed.push('\n');

    let title = format!("# {}\n\n", area.title);
    match before {
        Some(existing)
            if existing.contains(TOPIC_DOC_BEGIN) && existing.contains(TOPIC_DOC_END) =>
        {
            replace_managed_block(existing, TOPIC_DOC_BEGIN, TOPIC_DOC_END, &managed)
        }
        Some(existing) => {
            let mut output = existing.trim_end().to_string();
            output.push_str("\n\n");
            output.push_str(&managed);
            output
        }
        None => {
            let mut output = frontmatter;
            output.push_str(&title);
            output.push_str(&managed);
            output
        }
    }
}

fn current_model_from_issues(issues: &[IndexedIssue]) -> String {
    let mut lines = Vec::new();
    for issue in issues.iter().take(6) {
        lines.push(format!(
            "- {} contributed: {}",
            issue.issue_key,
            first_section_line(&issue.body, "## Outcome").unwrap_or_else(|| issue.title.clone())
        ));
    }
    if lines.is_empty() {
        "- No captured issue memory selected.".to_string()
    } else {
        lines.join("\n")
    }
}

fn invariants_from_issues(issues: &[IndexedIssue]) -> String {
    let mut lines = Vec::new();
    for issue in issues {
        if issue.body.to_ascii_lowercase().contains("invariant") {
            lines.push(format!(
                "- Recheck invariant notes in {} before changing this area.",
                issue.issue_key
            ));
        }
    }
    if lines.is_empty() {
        "- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.\n- Use capsule provenance to inspect the original PR or Linear issue when context is ambiguous.".to_string()
    } else {
        lines.join("\n")
    }
}

fn gotchas_from_issues(issues: &[IndexedIssue]) -> String {
    let mut lines = Vec::new();
    for issue in issues {
        if issue.warning_count > 0 {
            lines.push(format!(
                "- {} had capture warnings; verify source evidence before relying on it.",
                issue.issue_key
            ));
        }
    }
    if lines.is_empty() {
        "- No area-specific gotchas were inferred from the selected memory.".to_string()
    } else {
        lines.join("\n")
    }
}

fn mark_docs_synced(config: &MemoryConfig, plan: &DocsSyncPlan) -> Result<(), MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let run_id = format!("doc-sync-{}", Utc::now().timestamp_millis());
    let target_docs = plan
        .targets
        .iter()
        .map(|target| target.path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    connection
        .execute(
            "INSERT INTO doc_sync_runs (run_id, selected_issues_json, target_docs_json, generated_at, status) VALUES (?, ?, ?, ?, ?)",
            params![
                run_id,
                serde_json::to_string(&plan.selected_issue_keys)?,
                serde_json::to_string(&target_docs)?,
                Utc::now().to_rfc3339(),
                "written",
            ],
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    for issue_key in &plan.selected_issue_keys {
        connection
            .execute(
                "UPDATE issues SET docs_sync_status = 'synced' WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    for target in &plan.targets {
        connection
            .execute(
                "DELETE FROM doc_memory_links WHERE topic_doc = ?",
                params![target.path.to_string_lossy().to_string()],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for issue_key in &target.issue_keys {
            connection
                .execute(
                    "INSERT INTO doc_memory_links (topic_doc, issue_key, visibility) VALUES (?, ?, ?)",
                    params![
                        target.path.to_string_lossy().to_string(),
                        issue_key,
                        target.visibility.as_str(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
    }
    Ok(())
}

fn render_diff(before: &str, after: &str, path: &Path) -> String {
    if before == after {
        return format!("diff -- {}\n(no changes)\n", path.display());
    }
    let mut diff = String::new();
    diff.push_str(&format!("--- {}\n+++ {}\n", path.display(), path.display()));
    diff.push_str("@@\n");
    if !before.is_empty() {
        for line in before.lines().take(80) {
            diff.push_str(&format!("-{line}\n"));
        }
        if before.lines().count() > 80 {
            diff.push_str("-...\n");
        }
    }
    for line in after.lines().take(120) {
        diff.push_str(&format!("+{line}\n"));
    }
    if after.lines().count() > 120 {
        diff.push_str("+...\n");
    }
    diff
}

fn all_known_areas(config: &MemoryConfig, issues: &[IndexedIssue]) -> Vec<AreaConfig> {
    let mut slugs = config.areas.keys().cloned().collect::<BTreeSet<_>>();
    for issue in issues {
        for area in issue.areas() {
            slugs.insert(area);
        }
    }
    slugs
        .into_iter()
        .map(|slug| config.area_or_default(&slug))
        .collect()
}

fn discover_github_prs(
    repo_root: &Path,
    issue_key: &str,
) -> Result<Vec<PullRequestEvidence>, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--search",
            issue_key,
            "--json",
            "number,title,url,headRefName,mergedAt,body,mergeCommit",
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|error| format!("GitHub PR discovery skipped: failed to run gh: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "GitHub PR discovery skipped: gh exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let values =
        serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).map_err(|error| {
            format!("GitHub PR discovery skipped: failed to parse gh JSON: {error}")
        })?;
    let mut prs = Vec::new();
    for value in values {
        let Some(number) = value.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        if !contains_issue_key(
            value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            issue_key,
        ) && !contains_issue_key(
            value
                .get("body")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            issue_key,
        ) && !contains_issue_key(
            value
                .get("headRefName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            issue_key,
        ) {
            continue;
        }
        let merge_sha = value
            .get("mergeCommit")
            .and_then(|commit| commit.get("oid"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let merged_at = value
            .get("mergedAt")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let mut pr = PullRequestEvidence {
            number,
            title: value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            url: value
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            branch: value
                .get("headRefName")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            body: value
                .get("body")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            merge_sha,
            merged_at,
            ..PullRequestEvidence::default()
        };
        enrich_pr_from_gh(repo_root, &mut pr);
        prs.push(pr);
    }
    Ok(prs)
}

fn enrich_pr_from_gh(repo_root: &Path, pr: &mut PullRequestEvidence) {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr.number.to_string(),
            "--json",
            "files,commits,reviews,statusCheckRollup,mergeCommit",
        ])
        .current_dir(repo_root)
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return;
    };
    if pr.merge_sha.is_none() {
        pr.merge_sha = value
            .get("mergeCommit")
            .and_then(|commit| commit.get("oid"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
    }
    pr.changed_files = value
        .get("files")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            file.get("path")
                .and_then(serde_json::Value::as_str)
                .map(|path| ChangedFileEvidence {
                    path: PathBuf::from(path),
                    change_kind: file
                        .get("changeType")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                })
        })
        .collect();
    pr.commits = value
        .get("commits")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|commit| {
            let sha = commit
                .get("oid")
                .or_else(|| commit.get("sha"))
                .and_then(serde_json::Value::as_str)?;
            Some(CommitEvidence {
                sha: sha.to_string(),
                summary: commit
                    .get("messageHeadline")
                    .or_else(|| commit.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                author: None,
                timestamp: None,
            })
        })
        .collect();
    pr.reviews = value
        .get("reviews")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|review| ReviewEvidence {
            reviewer: review
                .get("author")
                .and_then(|author| author.get("login"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            state: review
                .get("state")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            submitted_at: review
                .get("submittedAt")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc)),
            disposition: review
                .get("body")
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_optional),
        })
        .collect();
    pr.checks = value
        .get("statusCheckRollup")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|check| {
            let name = check
                .get("name")
                .or_else(|| check.get("context"))
                .and_then(serde_json::Value::as_str)?;
            Some(CheckEvidence {
                name: name.to_string(),
                conclusion: check
                    .get("conclusion")
                    .or_else(|| check.get("state"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                completed_at: check
                    .get("completedAt")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc)),
            })
        })
        .collect();
}

fn read_to_string(path: &Path) -> Result<String, MemoryError> {
    fs::read_to_string(path).map_err(|source| MemoryError::ReadFile {
        path: path.to_path_buf(),
        source,
    })
}

fn create_dir_all(path: &Path) -> Result<(), MemoryError> {
    fs::create_dir_all(path).map_err(|source| MemoryError::CreateDir {
        path: path.to_path_buf(),
        source,
    })
}

fn write_file(path: &Path, contents: &str) -> Result<(), MemoryError> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    fs::write(path, contents).map_err(|source| MemoryError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn resolve_path(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn ensure_repo_contained(repo_root: &Path, path: &Path) -> Result<(), MemoryError> {
    let normalized = normalize_components(path);
    let repo_root = normalize_components(repo_root);
    if normalized.starts_with(&repo_root) {
        Ok(())
    } else {
        Err(MemoryError::PathOutsideRepo {
            path: normalized,
            repo_root,
        })
    }
}

fn normalize_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn normalize_issue_key(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn sanitize_issue_key(value: &str) -> String {
    normalize_issue_key(value)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn issue_title(issue: &IssueEvidence) -> String {
    fallback_title(&issue.title, &issue.identifier)
}

fn fallback_title(value: &str, fallback: &str) -> String {
    normalize_optional(value).unwrap_or_else(|| fallback.to_string())
}

fn placeholder_issue(identifier: &str) -> IssueEvidence {
    let identifier = normalize_issue_key(identifier);
    IssueEvidence {
        identifier: identifier.clone(),
        title: format!("{identifier} (source details unavailable)"),
        ..IssueEvidence::default()
    }
}

fn normalize_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = values
        .into_iter()
        .filter_map(|value| normalize_optional(&value))
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn contains_issue_key(text: &str, issue_key: &str) -> bool {
    text.to_ascii_uppercase()
        .contains(&normalize_issue_key(issue_key))
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in value.trim().to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn titleize_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn summarize_markdown(value: &str, limit: usize) -> String {
    let summary = summarize_text(value, limit);
    if summary.starts_with('-') || summary.starts_with('#') {
        summary
    } else {
        format!("{summary}\n")
    }
}

fn summarize_text(value: &str, limit: usize) -> String {
    let collapsed = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() <= limit {
        collapsed
    } else {
        format!(
            "{}...",
            collapsed
                .chars()
                .take(limit.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn should_copy_comment_summary(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    !lower.contains("full transcript")
        && !lower.contains("assistant:")
        && !lower.contains("user:")
        && body.split_whitespace().count() < 400
}

fn short_sha(sha: &str) -> &str {
    sha.get(0..7).unwrap_or(sha)
}

fn compact_capsule_body(body: &str) -> String {
    let mut output = String::new();
    let mut include = false;
    for line in body.lines() {
        if line.starts_with("## Outcome")
            || line.starts_with("## Decisions")
            || line.starts_with("## Validation")
            || line.starts_with("## Follow-ups")
            || line.starts_with("## Documentation")
        {
            include = true;
            output.push_str(line);
            output.push('\n');
            continue;
        }
        if line.starts_with("## ") && include {
            include = false;
        }
        if include && output.lines().count() < 80 {
            output.push_str(line);
            output.push('\n');
        }
    }
    if output.trim().is_empty() {
        first_interesting_line(body)
    } else {
        output
    }
}

fn first_interesting_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("---")
                && !line.starts_with("<!--")
                && !line.starts_with("type:")
        })
        .unwrap_or("No summary available.")
        .to_string()
}

fn first_section_line(body: &str, section: &str) -> Option<String> {
    let mut in_section = false;
    for line in body.lines() {
        if line.starts_with(section) {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            return None;
        }
        if in_section {
            let trimmed = line.trim().trim_start_matches("- ").trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn normalize_query_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter_map(normalize_optional)
        .map(|term| term.to_ascii_lowercase())
        .collect()
}

fn snippet_for_terms(body: &str, terms: &[String]) -> String {
    body.lines()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            terms.iter().any(|term| lower.contains(term))
        })
        .map(|line| summarize_text(line, 240))
        .unwrap_or_else(|| first_interesting_line(body))
}

fn contains_private_memory_link(contents: &str) -> bool {
    contents.contains(".opensymphony/memory")
        || contents.contains(".opensymphony\\memory")
        || contents.contains("../.opensymphony")
}

fn display_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn path_relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn replace_managed_block(existing: &str, begin: &str, end: &str, replacement: &str) -> String {
    let Some(begin_index) = existing.find(begin) else {
        return existing.to_string();
    };
    let Some(end_index) = existing.find(end) else {
        return existing.to_string();
    };
    let end_index = end_index + end.len();
    let mut output = String::new();
    output.push_str(existing[..begin_index].trim_end());
    output.push_str("\n\n");
    output.push_str(replacement.trim_end());
    output.push('\n');
    output.push_str(existing[end_index..].trim_start_matches('\n'));
    output
}

fn split_issue_key(value: &str) -> Result<(String, u64), MemoryError> {
    let value = normalize_issue_key(value);
    let Some((prefix, number)) = value.rsplit_once('-') else {
        return Err(MemoryError::InvalidInput(format!(
            "issue key `{value}` must look like PREFIX-123"
        )));
    };
    let number = number.parse::<u64>().map_err(|_| {
        MemoryError::InvalidInput(format!("issue key `{value}` has an invalid numeric suffix"))
    })?;
    Ok((prefix.to_string(), number))
}

fn issue_is_before(issue_key: &str, before_issue: &str) -> bool {
    match (split_issue_key(issue_key), split_issue_key(before_issue)) {
        (Ok((issue_prefix, issue_number)), Ok((before_prefix, before_number))) => {
            issue_prefix == before_prefix && issue_number < before_number
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

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

    fn config_for(repo_root: &Path) -> MemoryConfig {
        let config_path = repo_root.join("opensymphony-memory.yaml");
        fs::write(
            &config_path,
            r#"
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-runtime.md
    path_hints:
      - openhands
    labels:
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
