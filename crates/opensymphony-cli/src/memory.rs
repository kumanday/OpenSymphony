use std::{
    collections::BTreeSet,
    env, fs, io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{self, ExitCode},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[cfg(test)]
use std::io::Write;

use chrono::{NaiveDate, Utc};
use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{
    sync::{Mutex, OwnedMutexGuard, watch},
    task::JoinHandle,
};

use crate::{
    opensymphony_code_intel::{
        AstDiagnosticKind, CaptureRecord, CodeIntelArtifact, CodeIntelProvider, CodeIntelScope,
        CodeIntelScopeKind, CodeIntelSourceRef, CompositeCodeIntelProvider,
        JAVASCRIPT_QUERY_PACK_VERSION, JSX_QUERY_PACK_VERSION, PROVIDER_NAME,
        PYTHON_QUERY_PACK_VERSION, ParsedDocumentSummary, RUST_QUERY_PACK_VERSION, SourceLanguage,
        SymbolKind, TREE_SITTER_VERSION, TSX_QUERY_PACK_VERSION, TYPESCRIPT_QUERY_PACK_VERSION,
        parse_path, run_ad_hoc_query, skipped_directory_name,
    },
    opensymphony_domain::{TrackerIssue, TrackerIssueBlocker, TrackerIssueRef},
    opensymphony_linear::{LinearClient, LinearConfig},
    opensymphony_memory::{
        ArchivePlan, CodeGraphContextQuery, CodeIntelDiagnosticInput, CodeIntelDocumentInput,
        CodeIntelEdgeInput, CodeIntelPersistBatch, CodeIntelSkippedFileInput, CodeIntelSymbolInput,
        CodeWorkspaceOverlay, CommentEvidence, DEFAULT_PRIVATE_MEMORY_CONFIG_FILE, DocsSyncPlan,
        IssueEvidence, IssueLinkEvidence, IssueSelection, LintSeverity, MemoryConfig,
        MemoryContextOptions, MemoryError, MemoryReindexReport, MemoryScopeFilter,
        MemorySourceKind, MemorySourceRegistrationStatus, MemoryVisibility, RegisteredMemorySource,
        SourceFile, archive_blocking_warning_count, brief, code_graph_context,
        code_graph_workspace_context_overlay, code_index_branch, context_for_issue_with_options,
        docs_for_area_with_scope, expand_issue_range, export_okf_bundle, import_okf_bundle, lint,
        lint_okf_bundle, load_source_file, mark_archived, merge_memory_index_from_okf,
        migrate_code_repository_identity, persist_code_intel_documents,
        persist_code_intel_skipped_files, plan_archive, plan_capture, plan_docs_sync,
        plan_memory_init, reconcile_memory_sources, refresh_memory_index,
        refresh_memory_index_from_okf, register_memory_source, registered_memory_sources,
        related_by_area_with_scope, related_by_issue_with_scope, related_by_paths_with_scope,
        render_archive_plan, render_capture_dry_run, search_with_scope, sha256_bytes_hex,
        sha256_hex, status_with_scope, write_capture_plan, write_docs_sync_plan,
        write_memory_init_plan,
    },
    opensymphony_openhands::{
        ConversationMoveOutcome, ConversationStoreKind, IssueConversationManifest,
        OpenHandsConversationStorePaths,
    },
    opensymphony_workflow::{ResolvedWorkflow, WorkflowDefinition},
    opensymphony_workspace::{
        CleanupConfig, HookConfig, IssueManifest, WorkspaceManager, WorkspaceManagerConfig,
        workspace_path_for_root,
    },
};

use super::orchestrator_run::config::{
    CentralRoutingMode, looks_like_central_config, select_config_path, validate_central_config_text,
};

const MEMORY_MCP_TOOL_TIMEOUT: Duration = Duration::from_secs(300);
const REMOTE_MEMORY_TOOL_TIMEOUT: Duration = Duration::from_secs(330);
pub(crate) const MEMORY_ACTIVITY_MARKER: &str = ".opensymphony-memory.active";
pub(crate) const MEMORY_MIGRATION_LOCK: &str = "memory.migration.lock";
static MEMORY_STALE_LOCK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryActivityStatus {
    Absent,
    Live,
    Stale,
}

pub(crate) struct MemoryCoordinationLock {
    path: PathBuf,
}

impl Drop for MemoryCoordinationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

type MemoryWriterGate = Arc<Mutex<Option<MemoryCoordinationLock>>>;

#[allow(dead_code)]
enum MemoryWriterGuard {
    File(MemoryCoordinationLock),
    Shared(OwnedMutexGuard<Option<MemoryCoordinationLock>>),
}
const AST_MCP_TOOL_NAMES: &[&str] = &[
    "code.ast.status",
    "code.ast.outline",
    "code.ast.symbols",
    "code.ast.references",
    "code.ast.query",
    "code.ast.context",
    "code.ast.diagnostics",
];

#[derive(Debug, Args)]
pub struct MemoryArgs {
    #[arg(long, global = true, help = "Memory configuration YAML path")]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: MemoryCommand,
}

#[derive(Debug, Subcommand)]
enum MemoryCommand {
    #[command(about = "Create project memory configuration")]
    Init(InitArgs),
    #[command(about = "Capture completed issue evidence into issue memory")]
    Capture(CaptureArgs),
    #[command(about = "Import deterministic YAML issue evidence into issue memory")]
    Import(ImportArgs),
    #[command(name = "sync-docs", about = "Sync issue memory into topic docs")]
    SyncDocs(SyncDocsArgs),
    #[command(about = "Show capture and docs-sync status")]
    Status(StatusArgs),
    #[command(about = "Show one issue capsule")]
    Show(ShowArgs),
    #[command(about = "Show a compact issue memory brief")]
    Brief(ShowArgs),
    #[command(about = "Search captured issue memory")]
    Search(SearchArgs),
    #[command(about = "Find related issue memory")]
    Related(RelatedArgs),
    #[command(about = "Print topic documentation for an area")]
    Docs(DocsArgs),
    #[command(about = "Build a compact memory context bundle for an issue")]
    Context(ContextArgs),
    #[command(about = "Serve read-only memory tools over local MCP-style HTTP")]
    Serve(ServeArgs),
    #[command(about = "Lint memory and docs for stale or unsafe state")]
    Lint(LintArgs),
    #[command(about = "Refresh memory catalog schema and generated indexes")]
    Reindex(ReindexArgs),
    #[command(name = "export-okf", about = "Export an OKF memory bundle")]
    ExportOkf(ExportOkfArgs),
    #[command(name = "import-okf", about = "Import an OKF memory bundle")]
    ImportOkf(ImportOkfArgs),
}

#[derive(Debug, Args)]
struct InitArgs {
    #[arg(long, help = "Only show the proposed memory configuration")]
    dry_run: bool,
    #[arg(long, help = "Overwrite an existing memory configuration")]
    force: bool,
}

#[derive(Debug, Args)]
struct CaptureArgs {
    #[arg(help = "Issue identifier to capture, e.g. COE-123")]
    issue: Option<String>,
    #[arg(long, help = "Comma-separated issue identifiers")]
    issues: Option<String>,
    #[arg(
        long,
        help = "File containing one issue identifier per line or CSV cell"
    )]
    issues_file: Option<PathBuf>,
    #[arg(long, help = "Inclusive issue range, e.g. COE-100..COE-199")]
    issue_range: Option<String>,
    #[arg(long, help = "Skip default GitHub PR discovery")]
    no_github: bool,
    #[arg(long, help = "Only show the capture plan")]
    dry_run: bool,
    #[arg(long, help = "Overwrite generated or non-generated existing capsules")]
    force: bool,
}

#[derive(Debug, Args)]
struct ImportArgs {
    #[arg(help = "Issue identifier to import, e.g. COE-123")]
    issue: Option<String>,
    #[arg(long, help = "Comma-separated issue identifiers")]
    issues: Option<String>,
    #[arg(
        long,
        help = "File containing one issue identifier per line or CSV cell"
    )]
    issues_file: Option<PathBuf>,
    #[arg(long, help = "Inclusive issue range, e.g. COE-100..COE-199")]
    issue_range: Option<String>,
    #[arg(long, help = "Select source-file issues before this issue key")]
    before_issue: Option<String>,
    #[arg(long, help = "Select source-file issues in this milestone")]
    milestone: Option<String>,
    #[arg(long, help = "Select source-file issues with this state")]
    state: Option<String>,
    #[arg(
        long,
        help = "Select source-file issues completed or updated before YYYY-MM-DD"
    )]
    before_date: Option<NaiveDate>,
    #[arg(long, help = "YAML source evidence file for deterministic import")]
    source_file: PathBuf,
    #[arg(long, help = "Only show the capture plan")]
    dry_run: bool,
    #[arg(long, help = "Overwrite generated or non-generated existing capsules")]
    force: bool,
}

#[derive(Debug, Args)]
struct SyncDocsArgs {
    #[arg(long, help = "Comma-separated issue identifiers")]
    issues: Option<String>,
    #[arg(
        long,
        help = "File containing one issue identifier per line or CSV cell"
    )]
    issues_file: Option<PathBuf>,
    #[arg(long, help = "Only include issue capsules pending docs sync")]
    since_last_sync: bool,
    #[arg(long, help = "Only sync issue capsules for this area")]
    area: Option<String>,
    #[arg(long, help = "Only show the proposed documentation diff")]
    dry_run: bool,
    #[arg(
        long,
        help = "Include simple Mermaid diagrams in managed docs sections"
    )]
    with_diagrams: bool,
}

#[derive(Debug, Args)]
struct StatusArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long, help = "Filter by issue/work item")]
    issue: Option<String>,
    #[arg(long, help = "Filter by milestone")]
    milestone: Option<String>,
    #[arg(long, help = "Filter by area")]
    area: Option<String>,
}

#[derive(Debug, Args)]
struct ShowArgs {
    #[arg(help = "Issue identifier")]
    issue: String,
}

#[derive(Debug, Args)]
struct SearchArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long, help = "Filter by issue/work item")]
    issue: Option<String>,
    #[arg(long, help = "Filter by milestone")]
    milestone: Option<String>,
    #[arg(long, help = "Filter by area")]
    area: Option<String>,
    #[arg(help = "Search query")]
    query: String,
    #[arg(long, default_value = "10", help = "Maximum results")]
    limit: usize,
}

#[derive(Debug, Args)]
struct RelatedArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long, help = "Find memory related to this issue")]
    issue: Option<String>,
    #[arg(long, help = "Filter related memory by milestone")]
    milestone: Option<String>,
    #[arg(long, help = "Find memory related to this area")]
    area: Option<String>,
    #[arg(long, value_delimiter = ',', help = "Find memory related to paths")]
    paths: Vec<PathBuf>,
    #[arg(long, default_value = "10", help = "Maximum results")]
    limit: usize,
}

#[derive(Debug, Args)]
struct DocsArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long, help = "Issue/work item scope identifier")]
    issue: Option<String>,
    #[arg(long, help = "Milestone scope identifier")]
    milestone: Option<String>,
    #[arg(long, help = "Area slug")]
    area: String,
}

#[derive(Debug, Args)]
struct ContextArgs {
    #[command(flatten)]
    scope: ScopeArgs,
    #[arg(long, help = "Issue identifier")]
    issue: String,
    #[arg(long, help = "Milestone scope identifier")]
    milestone: Option<String>,
    #[arg(long, help = "Area scope slug")]
    area: Option<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Explicit issue identifiers to include"
    )]
    include: Vec<String>,
    #[arg(
        long,
        value_delimiter = ',',
        help = "Code paths to use for path-matched memory"
    )]
    paths: Vec<PathBuf>,
    #[arg(long, help = "Append code-intelligence context for --paths")]
    include_code_intel: bool,
    #[arg(long, default_value = "20", help = "Maximum selected memory briefs")]
    limit: usize,
}

#[derive(Debug, Args, Default, Clone)]
struct ScopeArgs {
    #[arg(long, help = "Project set scope identifier")]
    project_set: Option<String>,
    #[arg(long, help = "Project scope identifier")]
    project: Option<String>,
    #[arg(long, help = "Repository scope identifier or path")]
    repo: Option<String>,
    #[arg(
        long,
        help = "Allow queries outside the default current project set scope"
    )]
    all_accessible: bool,
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[arg(long, default_value = "127.0.0.1:8765", help = "Bind address")]
    addr: SocketAddr,
    #[arg(
        long,
        env = "OPENSYMPHONY_MEMORY_TOKEN",
        help = "Optional read-only bearer token"
    )]
    token: Option<String>,
    #[arg(
        long,
        env = "OPENSYMPHONY_MEMORY_ADMIN_TOKEN",
        help = "Optional admin bearer token for capture, sync, lint, and reindex tools"
    )]
    admin_token: Option<String>,
}

#[derive(Debug, Args)]
struct LintArgs {
    #[arg(long, help = "Check public docs for private memory links")]
    public_docs: bool,
    #[arg(long, help = "Lint an OKF bundle")]
    okf: bool,
    #[arg(help = "OKF bundle root; defaults to the configured memory root with --okf")]
    bundle: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "With --from-okf, reindex clears derived GitHub metadata tables \
                  (pull_requests, changed_files, checks, reviews). OKF concepts \
                  do not repopulate that metadata."
)]
struct ReindexArgs {
    #[arg(long, help = "Rebuild the derived catalog from OKF concept documents")]
    from_okf: bool,
    #[arg(help = "OKF bundle root; defaults to the configured memory root")]
    bundle: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ExportOkfArgs {
    #[arg(long, value_enum, help = "Bundle visibility to export")]
    visibility: OkfVisibilityArg,
    #[arg(
        long,
        help = "Output directory; defaults to okf-export-{visibility} under the repo root and must be empty if it already exists"
    )]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct ImportOkfArgs {
    #[arg(help = "OKF bundle directory to import")]
    bundle: PathBuf,
    #[arg(long, help = "Overwrite existing imported OKF Markdown files")]
    force: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum OkfVisibilityArg {
    Public,
    Private,
}

impl From<OkfVisibilityArg> for MemoryVisibility {
    fn from(value: OkfVisibilityArg) -> Self {
        match value {
            OkfVisibilityArg::Public => MemoryVisibility::Public,
            OkfVisibilityArg::Private => MemoryVisibility::Private,
        }
    }
}

#[derive(Debug, Args)]
pub struct LinearArgs {
    #[command(subcommand)]
    command: LinearCommand,
}

#[derive(Debug, Subcommand)]
enum LinearCommand {
    #[command(about = "Archive Linear issues only after memory capture")]
    Archive(ArchiveArgs),
}

#[derive(Debug, Args)]
struct ArchiveArgs {
    #[arg(long, help = "Memory configuration YAML path")]
    config: Option<PathBuf>,
    #[arg(long, help = "Comma-separated issue identifiers")]
    issues: Option<String>,
    #[arg(
        long,
        help = "File containing one issue identifier per line or CSV cell"
    )]
    issues_file: Option<PathBuf>,
    #[arg(long, help = "Inclusive issue range, e.g. COE-100..COE-199")]
    issue_range: Option<String>,
    #[arg(long, help = "Skip default GitHub PR discovery during live capture")]
    no_github: bool,
    #[arg(long, help = "Select archive candidates from captured memory")]
    from_memory: bool,
    #[arg(
        long,
        help = "Filter --from-memory candidates by Linear or memory state"
    )]
    state: Option<String>,
    #[arg(long, help = "Only show archive eligibility")]
    dry_run: bool,
    #[arg(long, help = "Bypass missing or warning capture checks")]
    force: bool,
    #[arg(long, help = "Runtime workflow path for Linear credentials")]
    workflow: Option<PathBuf>,
}

pub async fn run_command(args: MemoryArgs) -> ExitCode {
    match run_memory(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("opensymphony memory failed: {error}");
            ExitCode::from(1)
        }
    }
}

pub async fn run_linear_command(args: LinearArgs) -> ExitCode {
    match run_linear(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("opensymphony linear failed: {error}");
            ExitCode::from(1)
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct AutoMemoryReport {
    pub(crate) completed_issue_keys: Vec<String>,
    pub(crate) captured_issue_keys: Vec<String>,
    pub(crate) archived_issue_keys: Vec<String>,
    pub(crate) docs_written: Vec<PathBuf>,
    pub(crate) capture_completed: bool,
    pub(crate) docs_sync_completed: bool,
    pub(crate) archive_completed: bool,
    pub(crate) warnings: Vec<String>,
}

impl AutoMemoryReport {
    pub(crate) fn workflow_completed(&self) -> bool {
        self.capture_completed && self.docs_sync_completed && self.archive_completed
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn auto_capture_terminal(
    repo_root: &Path,
    workflow_path: &Path,
    resolved_workflow: Option<&ResolvedWorkflow>,
    identifiers: &[String],
    conversation_store: Option<&OpenHandsConversationStorePaths>,
    auto_archive: bool,
    memory_config: Option<&MemoryConfig>,
    writer_gate: Option<MemoryWriterGate>,
) -> Result<AutoMemoryReport, MemoryError> {
    let mut identifiers = identifiers
        .iter()
        .filter_map(|identifier| non_empty(identifier))
        .collect::<Vec<_>>();
    identifiers.sort();
    identifiers.dedup();
    if identifiers.is_empty() {
        return Ok(AutoMemoryReport::default());
    }

    let config = memory_config
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| load_memory_config(repo_root, None))?;
    let _coordination_lock = acquire_memory_writer_guard(&config, writer_gate).await?;
    let client = match resolved_workflow {
        Some(workflow) => linear_client_from_resolved_workflow(workflow)?,
        None => linear_client_from_workflow(repo_root, Some(workflow_path))?,
    };
    let source = load_linear_source_from_client(&client, &identifiers).await?;
    let selection = IssueSelection {
        identifiers,
        ..IssueSelection::default()
    };
    let mut capture_plan = plan_capture(&config, &source, &selection, true, true)?;
    let issue_keys = capture_plan
        .selected
        .iter()
        .map(|issue| issue.issue.identifier.clone())
        .collect::<Vec<_>>();
    capture_plan
        .selected
        .retain(|issue| !issue.already_captured || issue.stale);
    if issue_keys.is_empty() {
        return Ok(AutoMemoryReport::default());
    }

    let captured_issue_keys = capture_plan
        .selected
        .iter()
        .map(|issue| issue.issue.identifier.clone())
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    let mut capture_completed = true;
    let evolved_config = if capture_plan.selected.is_empty() {
        config.clone()
    } else {
        let capture_report = write_capture_plan(&config, &capture_plan, false)?;
        warnings.extend(capture_report.warnings);
        match reload_memory_config(&config) {
            Ok(config) => config,
            Err(error) => {
                capture_completed = false;
                warnings.push(format!(
                    "failed to reload evolved memory config after capture: {error}"
                ));
                let _ = record_auto_memory_status(&config, &issue_keys, &warnings);
                return Ok(AutoMemoryReport {
                    completed_issue_keys: Vec::new(),
                    captured_issue_keys,
                    archived_issue_keys: Vec::new(),
                    docs_written: Vec::new(),
                    capture_completed,
                    docs_sync_completed: false,
                    archive_completed: !auto_archive,
                    warnings,
                });
            }
        }
    };
    let docs_selection = IssueSelection {
        identifiers: issue_keys.clone(),
        since_last_sync: true,
        ..IssueSelection::default()
    };

    let mut archived_issue_keys = Vec::new();
    let mut docs_written = Vec::new();
    let mut docs_sync_completed = false;
    match plan_docs_sync(&evolved_config, &docs_selection, true, false) {
        Ok(docs_plan) => {
            warnings.extend(docs_plan.warnings.clone());
            if !docs_plan.targets.is_empty() {
                match write_docs_sync_plan(&evolved_config, &docs_plan) {
                    Ok(written) => {
                        docs_written = written;
                        docs_sync_completed = true;
                    }
                    Err(error) => {
                        warnings.push(format!("failed to sync captured memory docs: {error}"));
                    }
                }
            } else {
                docs_sync_completed = true;
            }
        }
        Err(error) => {
            warnings.push(format!("failed to plan captured memory docs sync: {error}"));
        }
    }

    let mut archive_completed = !auto_archive;
    if auto_archive {
        match plan_archive(&evolved_config, &issue_keys, false, None, true, false) {
            Ok(archive_plan) => {
                warnings.extend(archive_plan.warnings.clone());
                match archive_in_linear_with_client(&client, &archive_plan).await {
                    Ok(archive_report) => {
                        archive_completed =
                            archive_plan.warnings.is_empty() && archive_report.failures.is_empty();
                        if !archive_report.archived.is_empty()
                            && let Err(error) =
                                mark_archived(&evolved_config, &archive_report.archived)
                        {
                            archive_completed = false;
                            warnings
                                .push(format!("failed to mark archived memory capsules: {error}"));
                        }
                        if !archive_report.archived.is_empty() {
                            match archive_openhands_conversations_for_issues(
                                repo_root,
                                Some(workflow_path),
                                conversation_store,
                                &archive_report.archived,
                                resolved_workflow,
                            )
                            .await
                            {
                                Ok(conversation_report) => {
                                    archive_completed = archive_completed
                                        && conversation_report.failures.is_empty();
                                    warnings.extend(conversation_report.warnings);
                                    warnings.extend(conversation_report.failures);
                                }
                                Err(error) => {
                                    archive_completed = false;
                                    warnings.push(format!(
                                        "failed to archive OpenHands conversations: {error}"
                                    ));
                                }
                            }
                        }
                        archived_issue_keys = archive_report.archived;
                        warnings.extend(archive_report.failures);
                    }
                    Err(error) => {
                        warnings.push(format!("failed to archive captured Linear issues: {error}"));
                    }
                }
            }
            Err(error) => {
                warnings.push(format!(
                    "failed to plan captured Linear issue archive: {error}"
                ));
            }
        }
    }

    if let Err(error) = record_auto_memory_status(&evolved_config, &issue_keys, &warnings) {
        warnings.push(format!(
            "failed to record local memory automation status: {error}"
        ));
    }
    if !warnings.is_empty()
        && let Err(error) = update_linear_memory_status(&client, &issue_keys, &warnings).await
    {
        warnings.push(format!("failed to update Linear memory status: {error}"));
        if let Err(error) = record_auto_memory_status(&evolved_config, &issue_keys, &warnings) {
            warnings.push(format!(
                "failed to record local memory automation status after Linear update failure: {error}"
            ));
        }
    }
    let completed_issue_keys = if capture_completed && docs_sync_completed && archive_completed {
        issue_keys
    } else {
        Vec::new()
    };
    Ok(AutoMemoryReport {
        completed_issue_keys,
        captured_issue_keys,
        archived_issue_keys,
        docs_written,
        capture_completed,
        docs_sync_completed,
        archive_completed,
        warnings,
    })
}

async fn run_memory(args: MemoryArgs) -> Result<(), MemoryError> {
    let repo_root = env::current_dir().map_err(|source| MemoryError::ReadFile {
        path: PathBuf::from("."),
        source,
    })?;
    let MemoryArgs {
        config: config_path,
        command,
    } = args;
    let selected_central_config = selected_central_config_path(&repo_root, config_path.as_deref())?;
    if memory_command_writes(&command) {
        reject_project_set_memory_write(
            selected_central_config.as_deref(),
            command_name(&command),
        )?;
    }
    if let Some(endpoint) = env::var("OPENSYMPHONY_MEMORY_ENDPOINT")
        .ok()
        .and_then(|value| non_empty(&value))
        && let Some((tool_name, arguments)) = remote_memory_tool_request(&command)
    {
        return run_remote_memory_tool(&endpoint, tool_name, arguments).await;
    }
    match command {
        MemoryCommand::Init(args) => run_init(
            &repo_root,
            config_path.as_deref(),
            selected_central_config.as_deref(),
            args,
        ),
        MemoryCommand::Capture(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            let _coordination_lock = (!args.dry_run)
                .then(|| acquire_memory_writer_lock(&config))
                .transpose()?;
            run_capture(
                &repo_root,
                &config,
                selected_central_config.as_deref(),
                args,
            )
            .await
        }
        MemoryCommand::Import(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            let _coordination_lock = (!args.dry_run)
                .then(|| acquire_memory_writer_lock(&config))
                .transpose()?;
            run_import(&config, args)
        }
        MemoryCommand::SyncDocs(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            let _coordination_lock = (!args.dry_run)
                .then(|| acquire_memory_writer_lock(&config))
                .transpose()?;
            run_sync_docs(&config, args)
        }
        MemoryCommand::Status(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_status(&config, args)
        }
        MemoryCommand::Show(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_show(&config, args, ShowMode::Full)
        }
        MemoryCommand::Brief(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_show(&config, args, ShowMode::Brief)
        }
        MemoryCommand::Search(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_search(&config, args)
        }
        MemoryCommand::Related(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_related(&config, args)
        }
        MemoryCommand::Docs(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_docs(&config, args)
        }
        MemoryCommand::Context(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_context(
                &repo_root,
                &config,
                selected_central_config.as_deref(),
                args,
            )
            .await
        }
        MemoryCommand::Serve(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_serve(config, args, selected_central_config).await
        }
        MemoryCommand::Lint(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            run_lint(&config, args)
        }
        MemoryCommand::Reindex(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            let _coordination_lock = acquire_memory_writer_lock(&config)?;
            run_reindex(&config, args)
        }
        MemoryCommand::ExportOkf(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            let _coordination_lock = acquire_memory_writer_lock(&config)?;
            run_export_okf(&config, args)
        }
        MemoryCommand::ImportOkf(args) => {
            let config = load_memory_config(&repo_root, config_path.as_deref())?;
            let _coordination_lock = acquire_memory_writer_lock(&config)?;
            run_import_okf(&config, args)
        }
    }
}

fn memory_command_writes(command: &MemoryCommand) -> bool {
    matches!(
        command,
        MemoryCommand::Capture(_)
            | MemoryCommand::Import(_)
            | MemoryCommand::SyncDocs(_)
            | MemoryCommand::Reindex(_)
            | MemoryCommand::ExportOkf(_)
            | MemoryCommand::ImportOkf(_)
    )
}

fn command_name(command: &MemoryCommand) -> &'static str {
    match command {
        MemoryCommand::Capture(_) => "memory capture",
        MemoryCommand::Import(_) => "memory import",
        MemoryCommand::SyncDocs(_) => "memory sync-docs",
        MemoryCommand::Reindex(_) => "memory reindex",
        MemoryCommand::ExportOkf(_) => "memory export-okf",
        MemoryCommand::ImportOkf(_) => "memory import-okf",
        _ => "memory operation",
    }
}

fn reject_project_set_memory_write(
    central_config_path: Option<&Path>,
    operation: &str,
) -> Result<(), MemoryError> {
    let Some(central_config_path) = central_config_path else {
        return Ok(());
    };
    let raw = fs::read_to_string(central_config_path).map_err(|source| MemoryError::ReadFile {
        path: central_config_path.to_path_buf(),
        source,
    })?;
    let central = validate_central_config_text(central_config_path, &raw)
        .map_err(|error| MemoryError::InvalidInput(format!("invalid central config: {error}")))?;
    if central.mode == CentralRoutingMode::ProjectSet {
        return Err(MemoryError::InvalidInput(format!(
            "memory write operation `{operation}` does not support project_set central routing until strict routing is enabled"
        )));
    }
    Ok(())
}

async fn run_linear(args: LinearArgs) -> Result<(), MemoryError> {
    match args.command {
        LinearCommand::Archive(args) => run_archive(args).await,
    }
}

fn load_memory_config(
    repo_root: &Path,
    config_path: Option<&Path>,
) -> Result<MemoryConfig, MemoryError> {
    if let Some(central_path) = selected_central_config_path(repo_root, config_path)? {
        let memory_repo_root = central_memory_repo_root(repo_root, &central_path)?;
        let mut config = MemoryConfig::load(memory_repo_root, None)?;
        apply_central_memory_root(&mut config, &central_path)?;
        return Ok(config);
    }
    let mut config = MemoryConfig::load(repo_root, config_path)?;
    if config_path.is_some() {
        return Ok(config);
    }
    let Some(central_path) = select_config_path(repo_root, None) else {
        return Ok(config);
    };
    let raw = fs::read_to_string(&central_path).map_err(|source| MemoryError::ReadFile {
        path: central_path.clone(),
        source,
    })?;
    if !looks_like_central_config(&raw) {
        return Ok(config);
    }
    apply_central_memory_root(&mut config, &central_path)?;
    Ok(config)
}

fn selected_central_config_path(
    repo_root: &Path,
    explicit_path: Option<&Path>,
) -> Result<Option<PathBuf>, MemoryError> {
    let Some(config_path) = select_config_path(repo_root, explicit_path) else {
        return Ok(None);
    };
    if !config_path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&config_path).map_err(|source| MemoryError::ReadFile {
        path: config_path.clone(),
        source,
    })?;
    Ok(looks_like_central_config(&raw).then_some(config_path))
}

fn central_memory_repo_root(repo_root: &Path, central_path: &Path) -> Result<PathBuf, MemoryError> {
    let raw = fs::read_to_string(central_path).map_err(|source| MemoryError::ReadFile {
        path: central_path.to_path_buf(),
        source,
    })?;
    let central = validate_central_config_text(central_path, &raw)
        .map_err(|error| MemoryError::InvalidInput(format!("invalid central config: {error}")))?;
    Ok(central
        .target_repo()
        .unwrap_or_else(|| repo_root.to_path_buf()))
}

fn apply_central_memory_root(
    config: &mut MemoryConfig,
    central_path: &Path,
) -> Result<(), MemoryError> {
    let raw = fs::read_to_string(central_path).map_err(|source| MemoryError::ReadFile {
        path: central_path.to_path_buf(),
        source,
    })?;
    let central = validate_central_config_text(central_path, &raw)
        .map_err(|error| MemoryError::InvalidInput(format!("invalid central config: {error}")))?;
    if let Some(memory_root) = &central.memory_catalog_root {
        config.memory_root = memory_root.clone();
        config.index_path = memory_root.join("memory.duckdb");
    }
    for source in central.memory_sources.values() {
        *config = config.clone().with_repository_source(
            crate::opensymphony_memory::MemoryRepositorySource {
                repository_id: source.repository_id.clone(),
                root: source.checkout_path.clone(),
                commit_sha: None,
            },
        );
    }
    let active_repository_id = central.target_repo().and_then(|target| {
        central
            .memory_sources
            .values()
            .find(|source| source.checkout_path == target)
            .map(|source| source.repository_id.clone())
    });
    if let Some(repository_id) = active_repository_id.or_else(|| {
        (central.memory_sources.len() == 1)
            .then(|| central.memory_sources.keys().next().cloned())
            .flatten()
    }) {
        *config = config.clone().with_default_repository_id(repository_id);
    }
    config.containment_root = Some(central.state_root);
    Ok(())
}

fn reload_memory_config(config: &MemoryConfig) -> Result<MemoryConfig, MemoryError> {
    let mut evolved = if config.config_path.is_file() {
        MemoryConfig::load(&config.repo_root, Some(&config.config_path))?
    } else {
        MemoryConfig::load(&config.repo_root, None)?
    };
    evolved.memory_root = config.memory_root.clone();
    evolved.index_path = config.index_path.clone();
    evolved.containment_root = config.containment_root.clone();
    Ok(evolved)
}

fn run_init(
    repo_root: &Path,
    config_path: Option<&Path>,
    central_config_path: Option<&Path>,
    args: InitArgs,
) -> Result<(), MemoryError> {
    if central_config_path.is_some() {
        return Err(MemoryError::InvalidInput(
            "memory init cannot target a central instance config; initialize a repository-local memory config instead"
                .to_string(),
        ));
    }
    let plan = plan_memory_init(repo_root, config_path, args.force)?;
    println!("# Memory Init Plan\n");
    println!("Config: {}", plan.config_path.display());
    println!("Git ignore: {}", plan.gitignore_path.display());
    if args.dry_run {
        println!("\n## Proposed config\n");
        println!("{}", plan.config_contents);
        println!("Dry run only. Re-run without `--dry-run` to create memory configuration.");
        return Ok(());
    }

    let _coordination_lock =
        acquire_memory_coordination_lock(repo_root).map_err(|source| MemoryError::WriteFile {
            path: memory_migration_lock_path(repo_root),
            source,
        })?;
    write_memory_init_plan(&plan)?;
    println!("Wrote memory configuration: {}", plan.config_path.display());
    if plan.gitignore_before.as_deref() == Some(plan.gitignore_after.as_str()) {
        println!("Git ignore already allowed the shared memory config.");
    } else {
        println!("Updated git ignore: {}", plan.gitignore_path.display());
    }
    Ok(())
}

async fn run_capture(
    repo_root: &Path,
    config: &MemoryConfig,
    central_config_path: Option<&Path>,
    args: CaptureArgs,
) -> Result<(), MemoryError> {
    let identifiers = collect_issue_ids(
        args.issue.as_deref(),
        args.issues.as_deref(),
        args.issues_file.as_deref(),
        args.issue_range.as_deref(),
    )?;
    if identifiers.is_empty() {
        return Err(MemoryError::InvalidInput(
            "provide at least one issue identifier for live memory capture".to_string(),
        ));
    }
    let selection = IssueSelection {
        identifiers: identifiers.clone(),
        ..IssueSelection::default()
    };
    let source = load_linear_source(repo_root, None, central_config_path, &identifiers).await?;
    let write = !args.dry_run;
    let plan = plan_capture(config, &source, &selection, write, !args.no_github)?;
    print_or_write_capture_plan(config, &plan, args.force)?;
    Ok(())
}

fn run_import(config: &MemoryConfig, args: ImportArgs) -> Result<(), MemoryError> {
    let selection = IssueSelection {
        identifiers: collect_issue_ids(
            args.issue.as_deref(),
            args.issues.as_deref(),
            args.issues_file.as_deref(),
            args.issue_range.as_deref(),
        )?,
        milestone: args.milestone,
        state: args.state,
        before_date: args.before_date,
        before_issue: args.before_issue,
        area: None,
        since_last_sync: false,
    };
    let source = load_source_file(&args.source_file)?;
    let write = !args.dry_run;
    let plan = plan_capture(config, &source, &selection, write, false)?;
    print_or_write_capture_plan(config, &plan, args.force)?;
    Ok(())
}

fn print_or_write_capture_plan(
    config: &MemoryConfig,
    plan: &crate::opensymphony_memory::CapturePlan,
    force: bool,
) -> Result<(), MemoryError> {
    if !plan.write {
        println!("{}", render_capture_dry_run(config, plan));
        println!(
            "Dry run only. Re-run without `--dry-run` to create capsules and update the index."
        );
        return Ok(());
    }

    let report = write_capture_plan(config, plan, force)?;
    print_capture_write_report(report);
    Ok(())
}

fn print_capture_write_report(report: crate::opensymphony_memory::CaptureWriteReport) {
    println!("Wrote {} capsule(s).", report.written_capsules.len());
    for path in report.written_capsules {
        println!("- {}", path.display());
    }
    println!("Updated DuckDB index: {}", report.index_path.display());
    for path in report.markdown_indexes {
        println!("Updated markdown index: {}", path.display());
    }
    for path in report.milestone_nodes {
        println!("Updated milestone node: {}", path.display());
    }
    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in report.warnings {
            println!("- {warning}");
        }
    }
}

fn run_sync_docs(config: &MemoryConfig, args: SyncDocsArgs) -> Result<(), MemoryError> {
    let selection = IssueSelection {
        identifiers: collect_issue_ids(
            None,
            args.issues.as_deref(),
            args.issues_file.as_deref(),
            None,
        )?,
        area: args.area,
        since_last_sync: args.since_last_sync,
        ..IssueSelection::default()
    };
    let write = !args.dry_run;
    let plan = plan_docs_sync(config, &selection, write, args.with_diagrams)?;
    print_docs_plan(&plan);
    if !write {
        println!("Dry run only. Re-run without `--dry-run` to update topic docs.");
        return Ok(());
    }
    if plan.targets.is_empty() {
        return Ok(());
    }
    let written = write_docs_sync_plan(config, &plan)?;
    println!("Wrote {} topic doc(s).", written.len());
    for path in written {
        println!("- {}", path.display());
    }
    Ok(())
}

fn run_status(config: &MemoryConfig, args: StatusArgs) -> Result<(), MemoryError> {
    let scope = scope_filter(
        &args.scope,
        args.issue.as_deref(),
        args.milestone.as_deref(),
        args.area.as_deref(),
    );
    let report = status_with_scope(
        config,
        &IssueSelection {
            milestone: args.milestone.clone(),
            area: args.area.clone(),
            ..IssueSelection::default()
        },
        &scope,
    )?;
    println!("# Memory Status\n");
    println!("Issues captured: {}", report.issue_count);
    println!("Docs pending: {}", report.docs_pending_count);
    println!("Capture warnings: {}", report.warning_count);
    for issue in report.issues {
        println!(
            "- {}: {} [{}] areas={} warnings={}",
            issue.issue_key,
            issue.title,
            issue.docs_sync_status,
            issue.areas.join(","),
            issue.warning_count
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ShowMode {
    Full,
    Brief,
}

fn run_show(config: &MemoryConfig, args: ShowArgs, mode: ShowMode) -> Result<(), MemoryError> {
    match mode {
        ShowMode::Brief => {
            println!("{}", brief(config, &args.issue)?);
        }
        ShowMode::Full => {
            let path = config.issue_capsule_path(&args.issue);
            let contents = fs::read_to_string(&path).map_err(|source| MemoryError::ReadFile {
                path: path.clone(),
                source,
            })?;
            println!("{contents}");
        }
    }
    Ok(())
}

fn run_search(config: &MemoryConfig, args: SearchArgs) -> Result<(), MemoryError> {
    let scope = scope_filter(
        &args.scope,
        args.issue.as_deref(),
        args.milestone.as_deref(),
        args.area.as_deref(),
    );
    let results = search_with_scope(config, &args.query, args.limit, &scope)?;
    print_search_results(config, &results);
    Ok(())
}

fn run_related(config: &MemoryConfig, args: RelatedArgs) -> Result<(), MemoryError> {
    let scope = scope_filter(
        &args.scope,
        None,
        args.milestone.as_deref(),
        args.area.as_deref(),
    );
    let results = if let Some(issue) = args.issue {
        related_by_issue_with_scope(config, &issue, args.limit, &scope)?
    } else if let Some(area) = args.area {
        related_by_area_with_scope(config, &area, args.limit, &scope)?
    } else if !args.paths.is_empty() {
        related_by_paths_with_scope(config, &args.paths, args.limit, &scope)?
    } else {
        return Err(MemoryError::InvalidInput(
            "provide one of --issue, --area, or --paths".to_string(),
        ));
    };
    print_search_results(config, &results);
    Ok(())
}

fn run_docs(config: &MemoryConfig, args: DocsArgs) -> Result<(), MemoryError> {
    let scope = scope_filter(
        &args.scope,
        args.issue.as_deref(),
        args.milestone.as_deref(),
        Some(args.area.as_str()),
    );
    println!("{}", docs_for_area_with_scope(config, &args.area, &scope)?);
    Ok(())
}

async fn run_context(
    repo_root: &Path,
    config: &MemoryConfig,
    central_config_path: Option<&Path>,
    args: ContextArgs,
) -> Result<(), MemoryError> {
    if central_config_path.is_some() {
        let _ = load_central_resolved_workflow(repo_root, central_config_path)?;
    }
    let mut warnings = Vec::new();
    let source =
        match load_linear_context_source(repo_root, None, central_config_path, &args.issue).await {
            Ok(source) => source,
            Err(error) => {
                warnings.push(format!(
                "live Linear context lookup failed; continuing with indexed memory only: {error}"
            ));
                SourceFile::default()
            }
        };
    let options = MemoryContextOptions {
        issue: args.issue,
        explicit_includes: args.include,
        paths: args.paths,
        limit: args.limit,
    };
    let scope = scope_filter(
        &args.scope,
        Some(options.issue.as_str()),
        args.milestone.as_deref(),
        args.area.as_deref(),
    );
    for warning in warnings {
        println!("> Warning: {warning}\n");
    }
    let mut context = context_for_issue_with_options(config, &source, &options)?;
    if args.include_code_intel {
        append_code_intel_context(config, &mut context, &scope, &options.paths, options.limit)?;
    }
    println!("{context}");
    Ok(())
}

fn remote_memory_tool_request(command: &MemoryCommand) -> Option<(&'static str, Value)> {
    match command {
        MemoryCommand::Capture(args) => Some((
            "memory.capture",
            json!({
                "issue": args.issue.clone(),
                "issues": args.issues.clone(),
                "issuesFile": args.issues_file.as_ref().map(|path| path.display().to_string()),
                "issueRange": args.issue_range.clone(),
                "noGithub": args.no_github,
                "dryRun": args.dry_run,
                "force": args.force
            }),
        )),
        MemoryCommand::Import(args) => Some((
            "memory.capture",
            json!({
                "issue": args.issue.clone(),
                "issues": args.issues.clone(),
                "issuesFile": args.issues_file.as_ref().map(|path| path.display().to_string()),
                "issueRange": args.issue_range.clone(),
                "beforeIssue": args.before_issue.clone(),
                "milestone": args.milestone.clone(),
                "state": args.state.clone(),
                "beforeDate": args.before_date.map(|date| date.to_string()),
                "sourceFile": args.source_file.display().to_string(),
                "dryRun": args.dry_run,
                "force": args.force
            }),
        )),
        MemoryCommand::SyncDocs(args) => Some((
            "memory.sync_docs",
            json!({
                "issues": args.issues.clone(),
                "issuesFile": args.issues_file.as_ref().map(|path| path.display().to_string()),
                "sinceLastSync": args.since_last_sync,
                "area": args.area.clone(),
                "dryRun": args.dry_run,
                "withDiagrams": args.with_diagrams
            }),
        )),
        MemoryCommand::Lint(args) => Some((
            "memory.lint",
            json!({
                "publicDocs": args.public_docs,
                "okf": args.okf,
                "bundleRoot": args.bundle.as_ref().map(|path| path.display().to_string())
            }),
        )),
        MemoryCommand::Reindex(args) => Some((
            "memory.reindex",
            json!({
                "fromOkf": args.from_okf,
                "bundleRoot": args.bundle.as_ref().map(|path| path.display().to_string())
            }),
        )),
        MemoryCommand::ExportOkf(args) => Some((
            "memory.export_okf",
            json!({
                "visibility": MemoryVisibility::from(args.visibility).as_str(),
                "output": args.output.as_ref().map(|path| path.display().to_string())
            }),
        )),
        MemoryCommand::ImportOkf(args) => Some((
            "memory.import_okf",
            json!({
                "bundleRoot": args.bundle.display().to_string(),
                "force": args.force
            }),
        )),
        MemoryCommand::Brief(args) => {
            Some(("memory.brief", json!({ "issue": args.issue.clone() })))
        }
        MemoryCommand::Search(args) => Some((
            "memory.search",
            with_scope_json(
                &args.scope,
                json!({
                    "issue": args.issue.clone(),
                    "milestone": args.milestone.clone(),
                    "area": args.area.clone(),
                    "query": args.query.clone(),
                    "limit": args.limit
                }),
            ),
        )),
        MemoryCommand::Related(args) => Some((
            "memory.related",
            with_scope_json(
                &args.scope,
                json!({
                    "issue": args.issue.clone(),
                    "milestone": args.milestone.clone(),
                    "area": args.area.clone(),
                    "paths": path_strings(&args.paths),
                    "limit": args.limit
                }),
            ),
        )),
        MemoryCommand::Docs(args) => Some((
            "memory.docs",
            with_scope_json(
                &args.scope,
                json!({
                    "issue": args.issue.clone(),
                    "milestone": args.milestone.clone(),
                    "area": args.area.clone()
                }),
            ),
        )),
        MemoryCommand::Status(args) => Some((
            "memory.status",
            with_scope_json(
                &args.scope,
                json!({
                    "issue": args.issue.clone(),
                    "area": args.area.clone(),
                    "milestone": args.milestone.clone()
                }),
            ),
        )),
        MemoryCommand::Context(args) => Some((
            "memory.context",
            with_scope_json(
                &args.scope,
                json!({
                    "issue": args.issue.clone(),
                    "milestone": args.milestone.clone(),
                    "area": args.area.clone(),
                    "include": args.include.clone(),
                    "paths": path_strings(&args.paths),
                    "includeCodeIntel": args.include_code_intel,
                    "limit": args.limit
                }),
            ),
        )),
        _ => None,
    }
}

async fn run_remote_memory_tool(
    endpoint: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<(), MemoryError> {
    let client = reqwest::Client::builder()
        .timeout(REMOTE_MEMORY_TOOL_TIMEOUT)
        .build()
        .map_err(|error| {
            MemoryError::InvalidInput(format!(
                "failed to configure memory server client timeout: {error}"
            ))
        })?;
    let request = json!({
        "jsonrpc": "2.0",
        "id": "opensymphony-cli",
        "method": "tools/call",
        "params": {
            "name": tool_name,
            "arguments": arguments
        }
    });
    let mut builder = client.post(endpoint).json(&request);
    let token = remote_memory_tool_token_from_env(tool_name)?;
    if let Some(token) = token {
        builder = builder.bearer_auth(token);
    }
    let response = builder.send().await.map_err(|error| {
        MemoryError::InvalidInput(format!("failed to call memory server {endpoint}: {error}"))
    })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        MemoryError::InvalidInput(format!(
            "failed to read memory server response body: {error}"
        ))
    })?;
    let result = parse_remote_memory_response(status, &body, tool_name)?;
    print_remote_memory_result(result)?;
    Ok(())
}

fn parse_remote_memory_response(
    status: reqwest::StatusCode,
    body: &str,
    tool_name: &str,
) -> Result<Value, MemoryError> {
    if !status.is_success() {
        return Err(MemoryError::InvalidInput(format!(
            "memory server returned HTTP {status}: {}",
            remote_response_error_detail(body)
        )));
    }
    let payload = serde_json::from_str::<Value>(body).map_err(|error| {
        MemoryError::InvalidInput(format!(
            "memory server response was not valid JSON: {error}"
        ))
    })?;
    if let Some(error) = payload.get("error") {
        return Err(MemoryError::InvalidInput(format!(
            "memory server tool {tool_name} failed: {error}"
        )));
    }
    payload.get("result").cloned().ok_or_else(|| {
        MemoryError::InvalidInput("memory server response omitted result".to_string())
    })
}

fn remote_response_error_detail(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "<empty body>".to_string()
            } else {
                trimmed.to_string()
            }
        })
}

fn remote_memory_tool_token_from_env(tool_name: &str) -> Result<Option<String>, MemoryError> {
    remote_memory_tool_token(tool_name, |name| env::var(name).ok())
}

fn remote_memory_tool_token<F>(
    tool_name: &str,
    mut read_env: F,
) -> Result<Option<String>, MemoryError>
where
    F: FnMut(&str) -> Option<String>,
{
    if is_admin_memory_tool(tool_name) {
        return read_env("OPENSYMPHONY_MEMORY_ADMIN_TOKEN")
            .and_then(|value| non_empty(&value))
            .map(Some)
            .ok_or_else(|| {
                MemoryError::InvalidInput(format!(
                    "OPENSYMPHONY_MEMORY_ADMIN_TOKEN is required for remote admin memory tool `{tool_name}`"
                ))
            });
    }

    Ok(read_env("OPENSYMPHONY_MEMORY_TOKEN")
        .and_then(|value| non_empty(&value))
        .or_else(|| {
            read_env("OPENSYMPHONY_MEMORY_ADMIN_TOKEN").and_then(|value| non_empty(&value))
        }))
}

fn print_remote_memory_result(result: Value) -> Result<(), MemoryError> {
    if let Some(text) = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
    {
        println!("{text}");
        return Ok(());
    }
    let pretty = serde_json::to_string_pretty(&result)?;
    println!("{pretty}");
    Ok(())
}

fn path_strings(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect()
}

fn with_scope_json(scope: &ScopeArgs, mut arguments: Value) -> Value {
    if let Value::Object(map) = &mut arguments {
        map.insert(
            "projectSet".to_string(),
            json!(
                scope
                    .project_set
                    .clone()
                    .or_else(|| env_scope_value("OPENSYMPHONY_MEMORY_PROJECT_SET"))
            ),
        );
        map.insert(
            "project".to_string(),
            json!(
                scope
                    .project
                    .clone()
                    .or_else(|| env_scope_value("OPENSYMPHONY_MEMORY_PROJECT"))
            ),
        );
        map.insert(
            "repo".to_string(),
            json!(
                scope
                    .repo
                    .clone()
                    .or_else(|| env_scope_value("OPENSYMPHONY_MEMORY_EXECUTION_REPO"))
            ),
        );
        map.insert("allAccessible".to_string(), json!(scope.all_accessible));
    }
    arguments
}

fn run_lint(config: &MemoryConfig, args: LintArgs) -> Result<(), MemoryError> {
    let report = if args.okf {
        let bundle_root = args
            .bundle
            .as_deref()
            .map(|path| repo_existing_path_from_path(config, path))
            .transpose()?
            .unwrap_or_else(|| config.memory_root.clone());
        lint_okf_bundle(&bundle_root, args.public_docs)?
    } else {
        lint(config, args.public_docs)?
    };
    if report.findings.is_empty() {
        println!("Memory lint passed.");
        return Ok(());
    }
    for finding in report.findings {
        let severity = match finding.severity {
            LintSeverity::Info => "info",
            LintSeverity::Warn => "warn",
            LintSeverity::Error => "error",
        };
        let path = finding
            .path
            .as_ref()
            .map(|path| format!(" ({})", path.display()))
            .unwrap_or_default();
        println!("[{severity}] {}{path}", finding.message);
        if let Some(command) = finding.next_command {
            println!("  next: {command}");
        }
    }
    Ok(())
}

fn run_reindex(config: &MemoryConfig, args: ReindexArgs) -> Result<(), MemoryError> {
    let report = if args.from_okf {
        let bundle_root = args
            .bundle
            .as_deref()
            .map(|path| repo_existing_path_from_path(config, path))
            .transpose()?
            .unwrap_or_else(|| config.memory_root.clone());
        refresh_memory_index_from_okf(config, &bundle_root)?
    } else {
        refresh_memory_index(config)?
    };
    print_reindex_report(report);
    Ok(())
}

fn run_export_okf(config: &MemoryConfig, args: ExportOkfArgs) -> Result<(), MemoryError> {
    let visibility = MemoryVisibility::from(args.visibility);
    let report = export_okf_bundle(config, visibility, args.output.as_deref())?;
    println!("Exported OKF bundle: {}", report.output_path.display());
    println!("Visibility: {visibility}");
    println!("Copied files: {}", report.copied_files.len());
    println!(
        "Skipped private files: {}",
        report.skipped_private_files.len()
    );
    println!("Lint findings: {}", report.finding_count);
    for path in report.copied_files {
        println!("- {}", path.display());
    }
    Ok(())
}

fn run_import_okf(config: &MemoryConfig, args: ImportOkfArgs) -> Result<(), MemoryError> {
    let report = import_okf_bundle(config, &args.bundle, args.force)?;
    println!("Imported OKF bundle: {}", report.source_path.display());
    println!("Target memory root: {}", report.target_path.display());
    println!("Copied files: {}", report.copied_files.len());
    println!("Lint findings: {}", report.finding_count);
    print_reindex_report(report.reindex);
    Ok(())
}

fn print_reindex_report(report: MemoryReindexReport) {
    println!("Updated DuckDB index: {}", report.index_path.display());
    println!("Indexed records: {}", report.issue_count);
    println!("Indexed warnings: {}", report.warning_count);
    for path in report.markdown_indexes {
        println!("Updated markdown index: {}", path.display());
    }
}

#[derive(Clone)]
struct MemoryServerState {
    config: MemoryConfig,
    auth: MemoryServerAuth,
    workspace_root: Option<PathBuf>,
    central_config_path: Option<PathBuf>,
    resolved_workflow: Option<ResolvedWorkflow>,
    config_generation: Option<String>,
    writer_gate: MemoryWriterGate,
}

#[derive(Clone, Default)]
pub(crate) struct MemoryServerAuth {
    read_token: Option<String>,
    admin_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemoryServerAccess {
    Read,
    Admin,
}

#[derive(Debug, Deserialize)]
struct MemoryMcpRequest {
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

async fn run_serve(
    config: MemoryConfig,
    args: ServeArgs,
    central_config_path: Option<PathBuf>,
) -> Result<(), MemoryError> {
    let handle = start_memory_server_with_auth(
        config,
        args.addr,
        MemoryServerAuth {
            read_token: args.token,
            admin_token: args.admin_token,
        },
        None,
        central_config_path,
        None,
        None,
    )
    .await?;
    println!(
        "OpenSymphony memory server listening on {}",
        handle.endpoint()
    );
    handle.wait().await
}

pub(crate) struct MemoryServerHandle {
    endpoint: String,
    task: Option<JoinHandle<Result<(), String>>>,
    shutdown: watch::Sender<bool>,
    writer_gate: MemoryWriterGate,
}

impl Drop for MemoryServerHandle {
    fn drop(&mut self) {
        self.abort();
    }
}

impl MemoryServerHandle {
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn writer_gate(&self) -> Option<MemoryWriterGate> {
        (!self.is_finished()).then(|| Arc::clone(&self.writer_gate))
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.task.as_ref().is_some_and(JoinHandle::is_finished)
    }

    pub(crate) fn abort(&self) {
        let _ = self.shutdown.send(true);
    }

    pub(crate) async fn wait(mut self) -> Result<(), MemoryError> {
        let task = self
            .task
            .take()
            .expect("memory server task should only be awaited once");
        match task.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(MemoryError::InvalidInput(error)),
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(MemoryError::InvalidInput(format!(
                "memory server task failed: {error}"
            ))),
        }
    }
}

#[cfg(test)]
pub(crate) async fn start_memory_server_with_workspace_root(
    config: MemoryConfig,
    addr: SocketAddr,
    token: Option<String>,
    workspace_root: Option<PathBuf>,
) -> Result<MemoryServerHandle, MemoryError> {
    start_memory_server_with_auth(
        config,
        addr,
        MemoryServerAuth {
            read_token: token,
            admin_token: None,
        },
        workspace_root,
        None,
        None,
        None,
    )
    .await
}

#[allow(dead_code)]
pub(crate) async fn start_memory_server_with_central_config(
    config: MemoryConfig,
    addr: SocketAddr,
    token: Option<String>,
    workspace_root: Option<PathBuf>,
    central_config_path: Option<PathBuf>,
) -> Result<MemoryServerHandle, MemoryError> {
    start_memory_server_with_resolved_config(
        config,
        addr,
        token,
        workspace_root,
        central_config_path,
        None,
        None,
    )
    .await
}

pub(crate) async fn start_memory_server_with_resolved_config(
    config: MemoryConfig,
    addr: SocketAddr,
    token: Option<String>,
    workspace_root: Option<PathBuf>,
    central_config_path: Option<PathBuf>,
    resolved_workflow: Option<ResolvedWorkflow>,
    config_generation: Option<String>,
) -> Result<MemoryServerHandle, MemoryError> {
    start_memory_server_with_auth(
        config,
        addr,
        MemoryServerAuth {
            read_token: token,
            admin_token: None,
        },
        workspace_root,
        central_config_path,
        resolved_workflow,
        config_generation,
    )
    .await
}

async fn start_memory_server_with_auth(
    config: MemoryConfig,
    addr: SocketAddr,
    auth: MemoryServerAuth,
    workspace_root: Option<PathBuf>,
    central_config_path: Option<PathBuf>,
    resolved_workflow: Option<ResolvedWorkflow>,
    config_generation: Option<String>,
) -> Result<MemoryServerHandle, MemoryError> {
    let activity_marker = memory_activity_marker_path(&config.memory_root);
    let coordination_root = memory_coordination_root(&config);
    let coordination_lock =
        acquire_memory_coordination_lock(&coordination_root).map_err(|source| {
            MemoryError::InvalidInput(format!(
                "memory migration or server activity is already active at {}; {source}",
                memory_migration_lock_path(&coordination_root).display()
            ))
        })?;
    register_configured_memory_sources(&config)?;
    fs::create_dir_all(&config.memory_root).map_err(|source| MemoryError::CreateDir {
        path: config.memory_root.clone(),
        source,
    })?;
    match memory_activity_status(&config.memory_root).map_err(|source| {
        MemoryError::InvalidInput(format!(
            "failed to inspect memory server activity marker {}: {source}",
            activity_marker.display()
        ))
    })? {
        MemoryActivityStatus::Absent => {}
        MemoryActivityStatus::Stale => {
            fs::remove_file(&activity_marker).map_err(|source| {
                MemoryError::InvalidInput(format!(
                    "failed to remove stale memory activity marker {}: {source}",
                    activity_marker.display()
                ))
            })?;
        }
        MemoryActivityStatus::Live => {
            return Err(MemoryError::InvalidInput(format!(
                "memory server is already active at {}",
                activity_marker.display()
            )));
        }
    }
    super::orchestrator_run::publish_initialized_marker(
        &activity_marker,
        &super::orchestrator_run::process_marker_fields(),
    )
    .map_err(|source| {
        MemoryError::InvalidInput(format!(
            "memory server is already active or cannot claim {}; {source}",
            activity_marker.display()
        ))
    })?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|error| {
        let _ = fs::remove_file(&activity_marker);
        MemoryError::InvalidInput(format!("failed to bind memory server {addr}: {error}"))
    })?;
    let local_addr = listener.local_addr().map_err(|error| {
        let _ = fs::remove_file(&activity_marker);
        MemoryError::InvalidInput(format!("failed to read memory server address: {error}"))
    })?;
    let writer_gate = Arc::new(Mutex::new(Some(coordination_lock)));
    let state = MemoryServerState {
        config,
        auth,
        workspace_root,
        central_config_path,
        resolved_workflow,
        config_generation,
        writer_gate: Arc::clone(&writer_gate),
    };
    let app = axum::Router::new()
        .route("/health", axum::routing::get(memory_server_health))
        .route("/mcp", axum::routing::post(memory_server_mcp))
        .with_state(state);
    let (shutdown, mut shutdown_rx) = watch::channel(false);
    let writer_gate_for_task = Arc::clone(&writer_gate);
    let task = tokio::spawn(async move {
        let result = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await
            .map_err(|error| format!("memory server failed: {error}"));
        let _ = fs::remove_file(&activity_marker);
        let mut lock = writer_gate_for_task.lock().await;
        lock.take();
        result
    });
    Ok(MemoryServerHandle {
        endpoint: format!("http://{local_addr}/mcp"),
        task: Some(task),
        shutdown,
        writer_gate,
    })
}

fn register_configured_memory_sources(config: &MemoryConfig) -> Result<(), MemoryError> {
    let mut source_ids = BTreeSet::new();
    for source in config.repository_sources.values() {
        if let Some(legacy_repo_id) = source.root.file_name().and_then(|name| name.to_str()) {
            migrate_code_repository_identity(config, legacy_repo_id, &source.repository_id)?;
        }
        let commit_sha = source
            .commit_sha
            .clone()
            .or_else(|| git_commit_sha_for_repo(&source.root))
            .ok_or_else(|| {
                MemoryError::InvalidInput(format!(
                    "cannot resolve an exact Git commit for configured repository `{}` at {}",
                    source.repository_id,
                    source.root.display()
                ))
            })?;
        let local_config = MemoryConfig::load(&source.root, None)?;
        let mut roots = vec![
            (
                MemorySourceKind::Policy,
                source.root.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE),
            ),
            (MemorySourceKind::PublicDocs, local_config.docs.public_root),
            (MemorySourceKind::LegacyStore, local_config.memory_root),
        ];
        for export_name in ["okf-export-public", "okf-export-private"] {
            roots.push((MemorySourceKind::OkfBundle, source.root.join(export_name)));
        }
        for (kind, root) in roots {
            if !root.exists() {
                continue;
            }
            let source_id = if kind == MemorySourceKind::OkfBundle {
                format!(
                    "{}:{}:{}",
                    source.repository_id,
                    kind.as_str(),
                    root.display()
                )
            } else {
                format!("{}:{}", source.repository_id, kind.as_str())
            };
            source_ids.insert(source_id.clone());
            let registration = RegisteredMemorySource {
                source_id,
                repository_id: source.repository_id.clone(),
                commit_sha: commit_sha.clone(),
                kind,
                root: root.clone(),
                status: MemorySourceRegistrationStatus::Pending,
                generation: memory_source_generation(&root)?,
            };
            let already_imported = registered_memory_sources(config)?.iter().any(|existing| {
                existing.source_id == registration.source_id
                    && existing.generation == registration.generation
                    && existing.status == MemorySourceRegistrationStatus::Registered
            });
            if !already_imported {
                register_memory_source(config, &registration)?;
                if matches!(
                    kind,
                    MemorySourceKind::LegacyStore | MemorySourceKind::OkfBundle
                ) {
                    let mut import_config = config.clone();
                    import_config.repo_root = source.root.clone();
                    merge_memory_index_from_okf(&import_config, &root, &source.repository_id)?;
                }
                register_memory_source(
                    config,
                    &RegisteredMemorySource {
                        status: MemorySourceRegistrationStatus::Registered,
                        ..registration
                    },
                )?;
            }
        }
    }
    reconcile_memory_sources(config, &source_ids)?;
    Ok(())
}

fn memory_source_generation(root: &Path) -> Result<String, MemoryError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| MemoryError::ReadFile {
        path: root.to_path_buf(),
        source,
    })?;
    let mut entries = Vec::new();
    if metadata.file_type().is_file() {
        entries.push((
            root.file_name()
                .map(|name| name.to_string_lossy().as_bytes().to_vec())
                .unwrap_or_default(),
            fs::read(root).map_err(|source| MemoryError::ReadFile {
                path: root.to_path_buf(),
                source,
            })?,
        ));
    } else if metadata.file_type().is_dir() {
        collect_memory_source_entries(root, root, &mut entries)?;
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut material = Vec::new();
    for (relative, contents) in entries {
        material.extend_from_slice(&relative);
        material.push(0);
        material.extend_from_slice(&contents);
        material.push(0);
    }
    Ok(format!("sha256:{}", sha256_bytes_hex(&material)))
}

fn collect_memory_source_entries(
    root: &Path,
    current: &Path,
    entries: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<(), MemoryError> {
    for entry in fs::read_dir(current).map_err(|source| MemoryError::ReadFile {
        path: current.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| MemoryError::ReadFile {
            path: current.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| MemoryError::ReadFile {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            collect_memory_source_entries(root, &path, entries)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .as_bytes()
                .to_vec();
            let contents = fs::read(&path).map_err(|source| MemoryError::ReadFile {
                path: path.clone(),
                source,
            })?;
            entries.push((relative, contents));
        }
    }
    Ok(())
}

pub(crate) fn memory_activity_marker_path(memory_root: &Path) -> PathBuf {
    memory_root.join(MEMORY_ACTIVITY_MARKER)
}

pub(crate) fn memory_migration_lock_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".opensymphony").join(MEMORY_MIGRATION_LOCK)
}

pub(crate) fn acquire_memory_coordination_lock(
    repo_root: &Path,
) -> io::Result<MemoryCoordinationLock> {
    let path = memory_migration_lock_path(repo_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    loop {
        match super::orchestrator_run::publish_initialized_marker(
            &path,
            &super::orchestrator_run::process_marker_fields(),
        ) {
            Ok(_) => {
                return Ok(MemoryCoordinationLock {
                    path: path.to_path_buf(),
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if memory_lock_owner_is_stale(&path) {
                    let quarantine = stale_memory_lock_path(&path);
                    match fs::rename(&path, &quarantine) {
                        Ok(()) => {
                            fs::remove_file(&quarantine)?;
                            continue;
                        }
                        Err(rename_error) if rename_error.kind() == io::ErrorKind::NotFound => {
                            continue;
                        }
                        Err(rename_error) => return Err(rename_error),
                    }
                }
                return Err(error);
            }
            Err(error) => return Err(error),
        }
    }
}

fn memory_coordination_root(config: &MemoryConfig) -> PathBuf {
    let local_default = config.repo_root.join(".opensymphony/memory");
    if config.memory_root == local_default {
        config.repo_root.clone()
    } else {
        config.memory_root.clone()
    }
}

#[cfg(test)]
fn initialize_memory_coordination_lock(
    mut file: fs::File,
    path: &Path,
) -> io::Result<MemoryCoordinationLock> {
    if let Err(error) = file.write_all(super::orchestrator_run::process_marker_fields().as_bytes())
    {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(MemoryCoordinationLock {
        path: path.to_path_buf(),
    })
}

fn acquire_memory_writer_lock(
    config: &MemoryConfig,
) -> Result<MemoryCoordinationLock, MemoryError> {
    let coordination_root = memory_coordination_root(config);
    acquire_memory_coordination_lock(&coordination_root).map_err(|source| MemoryError::WriteFile {
        path: memory_migration_lock_path(&coordination_root),
        source,
    })
}

async fn acquire_memory_writer_guard(
    config: &MemoryConfig,
    writer_gate: Option<MemoryWriterGate>,
) -> Result<MemoryWriterGuard, MemoryError> {
    if let Some(writer_gate) = writer_gate {
        let guard = writer_gate.lock_owned().await;
        if guard.is_some() {
            return Ok(MemoryWriterGuard::Shared(guard));
        }
        drop(guard);
    }
    Ok(MemoryWriterGuard::File(acquire_memory_writer_lock(config)?))
}

fn stale_memory_lock_path(path: &Path) -> PathBuf {
    let sequence = MEMORY_STALE_LOCK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let incarnation = Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(MEMORY_MIGRATION_LOCK);
    path.with_file_name(format!(
        ".{name}.stale-{}-{incarnation}-{sequence}",
        process::id()
    ))
}

pub(crate) fn memory_lock_is_stale(repo_root: &Path) -> bool {
    let path = memory_migration_lock_path(repo_root);
    path.exists() && memory_lock_owner_is_stale(&path)
}

pub(crate) fn memory_activity_status(memory_root: &Path) -> io::Result<MemoryActivityStatus> {
    let path = memory_activity_marker_path(memory_root);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(MemoryActivityStatus::Absent);
        }
        Err(error) => return Err(error),
    };
    Ok(if memory_marker_owner_alive(&raw) {
        MemoryActivityStatus::Live
    } else {
        MemoryActivityStatus::Stale
    })
}

fn memory_lock_owner_is_stale(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    !memory_marker_owner_alive(&raw)
}

fn memory_marker_owner_alive(raw: &str) -> bool {
    let Some(pid) = raw.lines().find_map(|line| {
        line.strip_prefix("pid=")
            .and_then(|value| value.parse::<u32>().ok())
    }) else {
        return true;
    };
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    super::orchestrator_run::process_owner_alive(
        pid,
        raw.lines()
            .find_map(|line| line.strip_prefix("start=").map(str::trim)),
    )
}

async fn memory_server_health(
    axum::extract::State(state): axum::extract::State<MemoryServerState>,
) -> axum::Json<Value> {
    let mut payload = memory_server_health_payload(&state.auth);
    if let Some(generation) = state.config_generation.as_deref() {
        payload["configGeneration"] = json!(generation);
    }
    axum::Json(payload)
}

fn memory_server_health_payload(auth: &MemoryServerAuth) -> Value {
    let admin_tools = non_empty_str(auth.admin_token.as_deref()).is_some();
    json!({
        "status": "ok",
        "protocol": "mcp-streamable-http-2025-06-18",
        "mode": if admin_tools { "read_write" } else { "read_only" },
        "catalog": "per_instance",
        "adminTools": admin_tools
    })
}

async fn memory_server_mcp(
    axum::extract::State(state): axum::extract::State<MemoryServerState>,
    headers: axum::http::HeaderMap,
    axum::Json(request): axum::Json<MemoryMcpRequest>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    if let Err(response) = authorize_memory_request(
        &headers,
        &state.auth,
        required_access_for_request(&request, &state.auth),
    ) {
        return response;
    }
    let id = request.id.clone();
    let writer_request = request.method == "tools/call"
        && request
            .params
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(is_memory_writer_tool);
    let _writer_guard = if writer_request {
        Some(state.writer_gate.clone().lock_owned().await)
    } else {
        None
    };
    let result = match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": { "name": "opensymphony-memory", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": {} }
        })),
        "tools/list" => Ok(json!({
            "tools": memory_tool_descriptors(&state.config, &state.auth)
        })),
        "tools/call" => match tokio::time::timeout(
            MEMORY_MCP_TOOL_TIMEOUT,
            call_memory_tool_with_workspace(
                &state.config,
                request.params,
                state.workspace_root.as_deref(),
                state.central_config_path.as_deref(),
                state.resolved_workflow.as_ref(),
            ),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(MemoryError::InvalidInput(format!(
                "memory tool call exceeded {} second timeout",
                MEMORY_MCP_TOOL_TIMEOUT.as_secs()
            ))),
        },
        other => Err(MemoryError::InvalidInput(format!(
            "unsupported MCP method `{other}`"
        ))),
    };

    match result {
        Ok(value) => (
            axum::http::StatusCode::OK,
            axum::Json(json!({ "jsonrpc": "2.0", "id": id, "result": value })),
        ),
        Err(error) => (
            axum::http::StatusCode::OK,
            axum::Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32000, "message": error.to_string() }
            })),
        ),
    }
}

fn required_access_for_request(
    request: &MemoryMcpRequest,
    auth: &MemoryServerAuth,
) -> MemoryServerAccess {
    if request.method == "tools/call"
        && request
            .params
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| required_access_for_tool(name, auth) == MemoryServerAccess::Admin)
    {
        MemoryServerAccess::Admin
    } else {
        MemoryServerAccess::Read
    }
}

fn memory_tool_descriptors(config: &MemoryConfig, auth: &MemoryServerAuth) -> Vec<Value> {
    let mut tools = vec![
        json!({ "name": "memory.context", "description": "Build a pre-implementation memory context bundle", "access": "read" }),
        json!({ "name": "memory.search", "description": "Search captured issue memory", "access": "read" }),
        json!({ "name": "memory.related", "description": "Find related issue memory by issue, area, or paths", "access": "read" }),
        json!({ "name": "memory.brief", "description": "Return a compact issue memory brief", "access": "read" }),
        json!({ "name": "memory.docs", "description": "Return topic documentation for an area", "access": "read" }),
        json!({ "name": "memory.status", "description": "Return capture and docs-sync status", "access": "read" }),
        json!({ "name": "memory.capture", "description": "Capture completed issue evidence into memory", "access": "admin" }),
        json!({ "name": "memory.sync_docs", "description": "Sync captured memory into topic docs", "access": "admin" }),
        json!({ "name": "memory.lint", "description": "Lint memory and docs", "access": "admin" }),
        json!({ "name": "memory.reindex", "description": "Refresh memory catalog schema and generated indexes", "access": "admin" }),
        json!({ "name": "memory.export_okf", "description": "Export an OKF memory bundle", "access": "admin" }),
        json!({ "name": "memory.import_okf", "description": "Import an OKF memory bundle", "access": "admin" }),
        json!({ "name": "memory.ingest_code_intel", "description": "Generate code-intelligence artifacts for future ingestion", "access": "admin" }),
    ];
    if config.code_intel.enabled {
        tools.push(json!({
            "name": "code.graph.context",
            "description": "Bounded read-only indexed code discovery with optional workspace overlay",
            "access": "read",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "repository": { "type": "string", "description": "Indexed repository id" },
                    "query": { "type": "string", "description": "Case-insensitive symbol/path search" },
                    "path": { "type": "string", "description": "Repository-relative path or directory" },
                    "symbol": { "type": "string", "description": "Symbol name or stable symbol key" },
                    "runId": { "type": "string", "description": "Optional issue run identity for workspace overlay reads" },
                    "depth": { "type": "integer", "minimum": 0, "maximum": 8, "default": 1 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 20 }
                }
            }
        }));
    }
    if ast_tools_enabled(config) {
        tools.extend(AST_MCP_TOOL_NAMES.iter().map(|name| {
            json!({
                "name": name,
                "description": "Read-only Tree-sitter AST code intelligence",
                "access": match required_access_for_tool(name, auth) {
                    MemoryServerAccess::Read => "read",
                    MemoryServerAccess::Admin => "admin",
                }
            })
        }));
    }
    tools
}

fn is_admin_memory_tool(name: &str) -> bool {
    matches!(
        name,
        "memory.capture"
            | "memory.sync_docs"
            | "memory.lint"
            | "memory.reindex"
            | "memory.export_okf"
            | "memory.import_okf"
            | "memory.ingest_code_intel"
    )
}

fn is_memory_writer_tool(name: &str) -> bool {
    matches!(
        name,
        "memory.capture"
            | "memory.sync_docs"
            | "memory.reindex"
            | "memory.export_okf"
            | "memory.import_okf"
            | "memory.ingest_code_intel"
    )
}

fn required_access_for_tool(name: &str, auth: &MemoryServerAuth) -> MemoryServerAccess {
    if is_admin_memory_tool(name)
        || (name == "code.ast.query" && non_empty_str(auth.admin_token.as_deref()).is_some())
    {
        MemoryServerAccess::Admin
    } else {
        MemoryServerAccess::Read
    }
}

fn ast_tools_enabled(config: &MemoryConfig) -> bool {
    config.code_intel.enabled && config.code_intel.ast.enabled
}

fn authorize_memory_request(
    headers: &axum::http::HeaderMap,
    auth: &MemoryServerAuth,
    required_access: MemoryServerAccess,
) -> Result<(), (axum::http::StatusCode, axum::Json<Value>)> {
    if let Some(origin) = headers
        .get(axum::http::header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        && !origin_is_localhost(origin)
    {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(json!({
                "error": {
                    "code": "forbidden_origin",
                    "message": "memory server only accepts localhost origins"
                }
            })),
        ));
    }
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let authorized = match required_access {
        MemoryServerAccess::Read => {
            let read_token = non_empty_str(auth.read_token.as_deref());
            let admin_token = non_empty_str(auth.admin_token.as_deref());
            match (read_token, admin_token) {
                (Some(read_token), Some(admin_token)) => {
                    bearer == Some(read_token) || bearer == Some(admin_token)
                }
                (Some(read_token), None) => bearer == Some(read_token),
                (None, Some(admin_token)) => bearer == Some(admin_token),
                (None, None) => true,
            }
        }
        MemoryServerAccess::Admin => {
            let Some(admin_token) = non_empty_str(auth.admin_token.as_deref()) else {
                return Err((
                    axum::http::StatusCode::FORBIDDEN,
                    axum::Json(json!({
                        "error": {
                            "code": "admin_token_required",
                            "message": "memory server admin token is required for admin tools"
                        }
                    })),
                ));
            };
            bearer == Some(admin_token)
        }
    };
    if authorized {
        Ok(())
    } else {
        Err((
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(json!({
                "error": {
                    "code": "unauthorized",
                    "message": "memory server token is required for this tool"
                }
            })),
        ))
    }
}

fn non_empty_str(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn origin_is_localhost(origin: &str) -> bool {
    let Ok(origin) = url::Url::parse(origin.trim()) else {
        return false;
    };
    if !matches!(origin.scheme(), "http" | "https") {
        return false;
    }
    matches!(
        origin.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
    )
}

#[cfg(test)]
async fn call_memory_tool(config: &MemoryConfig, params: Value) -> Result<Value, MemoryError> {
    call_memory_tool_with_workspace(config, params, None, None, None).await
}

async fn call_memory_tool_with_workspace(
    config: &MemoryConfig,
    params: Value,
    workspace_root: Option<&Path>,
    central_config_path: Option<&Path>,
    resolved_workflow: Option<&ResolvedWorkflow>,
) -> Result<Value, MemoryError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| MemoryError::InvalidInput("tools/call requires params.name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
    if central_config_path.is_some() && resolved_workflow.is_none() && name == "memory.context" {
        let _ = load_central_resolved_workflow(&config.repo_root, central_config_path)?;
    }
    if is_memory_writer_tool(name) {
        reject_project_set_memory_write(central_config_path, name)?;
    }
    if AST_MCP_TOOL_NAMES.contains(&name) && !ast_tools_enabled(config) {
        return Err(MemoryError::InvalidInput(
            "AST code-intelligence tools are disabled".to_string(),
        ));
    }
    if name == "code.graph.context" && !config.code_intel.enabled {
        return Err(MemoryError::InvalidInput(
            "indexed code graph tools are disabled".to_string(),
        ));
    }
    match name {
        "memory.context" => {
            let issue = required_string_arg(&arguments, "issue")?;
            let options = MemoryContextOptions {
                issue: issue.clone(),
                explicit_includes: string_list_arg(&arguments, "include"),
                paths: string_list_arg(&arguments, "paths")
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
                limit: usize_arg(&arguments, "limit", 20),
            };
            let source = context_source_from_mcp(&arguments);
            let mut text = context_for_issue_with_options(config, &source, &options)?;
            if bool_arg(&arguments, "includeCodeIntel")
                || bool_arg(&arguments, "include_code_intel")
            {
                text = append_code_intel_context_blocking(
                    config.clone(),
                    text,
                    scope_filter_from_mcp(&arguments, true),
                    options.paths.clone(),
                    options.limit,
                )
                .await?;
            }
            Ok(mcp_text(text))
        }
        "memory.search" => {
            let query = required_string_arg(&arguments, "query")?;
            let scope = scope_filter_from_mcp(&arguments, true);
            let results =
                search_with_scope(config, &query, usize_arg(&arguments, "limit", 10), &scope)?;
            Ok(json!({ "results": search_results_json(config, &results) }))
        }
        "memory.related" => {
            let limit = usize_arg(&arguments, "limit", 10);
            let scope = scope_filter_from_mcp(&arguments, false);
            let results = if let Some(issue) = optional_string_arg(&arguments, "issue") {
                related_by_issue_with_scope(config, &issue, limit, &scope)?
            } else if let Some(area) = optional_string_arg(&arguments, "area") {
                related_by_area_with_scope(config, &area, limit, &scope)?
            } else {
                let paths = string_list_arg(&arguments, "paths")
                    .into_iter()
                    .map(PathBuf::from)
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    return Err(MemoryError::InvalidInput(
                        "memory.related requires issue, area, or paths".to_string(),
                    ));
                }
                related_by_paths_with_scope(config, &paths, limit, &scope)?
            };
            Ok(json!({ "results": search_results_json(config, &results) }))
        }
        "memory.brief" => {
            let issue = required_string_arg(&arguments, "issue")?;
            Ok(mcp_text(brief(config, &issue)?))
        }
        "memory.docs" => {
            let area = required_string_arg(&arguments, "area")?;
            Ok(mcp_text(docs_for_area_with_scope(
                config,
                &area,
                &scope_filter_from_mcp(&arguments, false),
            )?))
        }
        "memory.status" => {
            let scope = scope_filter_from_mcp(&arguments, true);
            let report = status_with_scope(
                config,
                &IssueSelection {
                    area: optional_string_arg(&arguments, "area"),
                    milestone: optional_string_arg(&arguments, "milestone"),
                    ..IssueSelection::default()
                },
                &scope,
            )?;
            let sources = registered_memory_sources(config)?
                .into_iter()
                .filter(|source| {
                    scope
                        .repo
                        .as_deref()
                        .is_none_or(|repository_id| source.repository_id == repository_id)
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "issueCount": report.issue_count,
                "warningCount": report.warning_count,
                "docsPendingCount": report.docs_pending_count,
                "sources": sources.into_iter().map(|source| json!({
                    "sourceId": source.source_id,
                    "repositoryId": source.repository_id,
                    "commit": source.commit_sha,
                    "kind": source.kind.as_str(),
                    "status": source.status,
                    "generation": source.generation,
                })).collect::<Vec<_>>(),
                "issues": report.issues.into_iter().map(|issue| json!({
                    "issueKey": issue.issue_key,
                    "title": issue.title,
                    "state": issue.state,
                    "milestone": issue.milestone,
                    "areas": issue.areas,
                    "docsSyncStatus": issue.docs_sync_status,
                    "warningCount": issue.warning_count,
                    "capsulePath": path_for_json(config, &issue.capsule_path)
                })).collect::<Vec<_>>()
            }))
        }
        "code.graph.context" => {
            call_code_graph_context_tool(
                config.clone(),
                arguments.clone(),
                workspace_root.map(Path::to_path_buf),
            )
            .await
        }
        "code.ast.status" => call_code_ast_status_tool(config),
        "code.ast.outline" => call_code_ast_outline_tool(config.clone(), arguments.clone()).await,
        "code.ast.symbols" => call_code_ast_symbols_tool(config.clone(), arguments.clone()).await,
        "code.ast.references" => {
            call_code_ast_references_tool(config.clone(), arguments.clone()).await
        }
        "code.ast.query" => call_code_ast_query_tool(config.clone(), arguments.clone()).await,
        "code.ast.context" => call_code_ast_context_tool(config, &arguments).await,
        "code.ast.diagnostics" => {
            call_code_ast_diagnostics_tool(config.clone(), arguments.clone()).await
        }
        "memory.capture" => {
            call_memory_capture_tool(config, &arguments, central_config_path, resolved_workflow)
                .await
        }
        "memory.sync_docs" => call_memory_sync_docs_tool(config, &arguments),
        "memory.lint" => call_memory_lint_tool(config, &arguments),
        "memory.reindex" => call_memory_reindex_tool(config, &arguments),
        "memory.export_okf" => call_memory_export_okf_tool(config, &arguments),
        "memory.import_okf" => call_memory_import_okf_tool(config, &arguments),
        "memory.ingest_code_intel" => call_memory_ingest_code_intel_tool(config, &arguments).await,
        other => Err(MemoryError::InvalidInput(format!(
            "unsupported memory tool `{other}`"
        ))),
    }
}

async fn call_code_graph_context_tool(
    config: MemoryConfig,
    arguments: Value,
    workspace_root: Option<PathBuf>,
) -> Result<Value, MemoryError> {
    ast_mcp_tool_blocking("code.graph.context", move || {
        let repo_id = optional_string_arg(&arguments, "repository")
            .or_else(|| optional_string_arg(&arguments, "repo"))
            .or_else(|| config.default_repository_id.clone())
            .unwrap_or_else(|| {
                config
                    .repo_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("repo")
                    .to_string()
            });
        let context_query = CodeGraphContextQuery {
            repo_id: repo_id.clone(),
            query: optional_string_arg(&arguments, "query"),
            path: optional_string_arg(&arguments, "path"),
            symbol: optional_string_arg(&arguments, "symbol"),
            depth: usize_arg_allow_zero(&arguments, "depth", 1).min(8),
            limit: usize_arg(&arguments, "limit", 20)
                .min(config.code_intel.ast.max_matches_per_request.max(1)),
        };
        let overlay = optional_string_arg(&arguments, "runId")
            .or_else(|| optional_string_arg(&arguments, "run"))
            .map(|run_id| {
                resolve_code_graph_overlay(
                    &config,
                    workspace_root.as_deref(),
                    &repo_id,
                    &run_id,
                    &context_query,
                )
            })
            .transpose()?;
        code_graph_context(&config, context_query, overlay.as_ref())
            .map_err(|error| MemoryError::InvalidInput(error.to_string()))
    })
    .await
}

fn resolve_code_graph_overlay(
    config: &MemoryConfig,
    workspace_root: Option<&Path>,
    repo_id: &str,
    run_id: &str,
    context_query: &CodeGraphContextQuery,
) -> Result<CodeWorkspaceOverlay, MemoryError> {
    let workspace_root = workspace_root.ok_or_else(|| {
        MemoryError::InvalidInput(
            "run-scoped code graph context requires a configured workspace root".to_string(),
        )
    })?;
    let workspace_root =
        workspace_root
            .canonicalize()
            .map_err(|source| MemoryError::ResolvePath {
                path: workspace_root.to_path_buf(),
                source,
            })?;
    let workspace_candidate = workspace_path_for_root(&workspace_root, run_id)
        .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
    if let Ok(metadata) = fs::symlink_metadata(&workspace_candidate)
        && metadata.file_type().is_symlink()
    {
        return Err(MemoryError::InvalidInput(
            "run workspace root must not be a symlink".to_string(),
        ));
    }
    let workspace_path =
        workspace_candidate
            .canonicalize()
            .map_err(|source| MemoryError::ResolvePath {
                path: workspace_candidate.clone(),
                source,
            })?;
    if !workspace_path.starts_with(&workspace_root) {
        return Err(MemoryError::PathOutsideRepo {
            path: workspace_path,
            repo_root: workspace_root,
        });
    }
    let manifest_path = workspace_path.join(".opensymphony/issue.json");
    let manifest = fs::read_to_string(&manifest_path).map_err(|source| MemoryError::ReadFile {
        path: manifest_path.clone(),
        source,
    })?;
    let manifest: IssueManifest = serde_json::from_str(&manifest).map_err(|source| {
        MemoryError::InvalidInput(format!(
            "invalid run workspace ownership manifest: {source}"
        ))
    })?;
    let workspace_key = workspace_candidate
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let manifest_workspace_path =
        manifest
            .workspace_path
            .canonicalize()
            .map_err(|source| MemoryError::ResolvePath {
                path: manifest.workspace_path.clone(),
                source,
            })?;
    if (manifest.identifier != run_id && manifest.issue_id != run_id)
        || manifest.sanitized_workspace_key != workspace_key
        || manifest_workspace_path != workspace_path
    {
        return Err(MemoryError::InvalidInput(
            "run workspace ownership manifest does not match the requested run".to_string(),
        ));
    }
    let branch = code_index_branch(&config.repo_root)
        .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
    let base_revision = workspace_merge_base(&workspace_path, &branch)?;
    code_graph_workspace_context_overlay(
        config,
        repo_id,
        &workspace_path,
        run_id,
        &base_revision,
        context_query,
    )
    .map_err(|error| MemoryError::InvalidInput(error.to_string()))
}

fn workspace_merge_base(workspace_path: &Path, branch: &str) -> Result<String, MemoryError> {
    for reference in [format!("origin/{branch}"), branch.to_string()] {
        let verified = process::Command::new("git")
            .current_dir(workspace_path)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(&reference)
            .output()
            .map_err(|source| {
                MemoryError::InvalidInput(format!("failed to inspect git workspace: {source}"))
            })?;
        if !verified.status.success() {
            continue;
        }
        let output = process::Command::new("git")
            .current_dir(workspace_path)
            .args(["merge-base", "HEAD"])
            .arg(&reference)
            .output()
            .map_err(|source| {
                MemoryError::InvalidInput(format!("failed to resolve git merge base: {source}"))
            })?;
        if output.status.success() {
            let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !revision.is_empty() {
                return Ok(revision);
            }
        }
    }
    Err(MemoryError::InvalidInput(format!(
        "no usable git comparison base found for branch `{branch}`"
    )))
}

struct AstDocument {
    display: String,
    source: String,
    summary: ParsedDocumentSummary,
}

struct AstDocuments {
    documents: Vec<AstDocument>,
    warnings: Vec<String>,
}

fn call_code_ast_status_tool(config: &MemoryConfig) -> Result<Value, MemoryError> {
    let _ = resolve_code_intel_repo(config, None)?;
    Ok(json!({
        "provider": "tree-sitter-ast",
        "available": true,
        "languages": ast_language_ids(),
        "parserVersion": TREE_SITTER_VERSION,
        "queryPackVersions": ast_query_pack_versions(),
        "limits": ast_limits_json(config)
    }))
}

async fn call_code_ast_outline_tool(
    config: MemoryConfig,
    arguments: Value,
) -> Result<Value, MemoryError> {
    ast_mcp_tool_blocking("code.ast.outline", move || {
        let ast_documents = ast_documents(&config, &arguments)?;
        let limit = ast_limit(&config, &arguments);
        let mut remaining = limit;
        let mut truncated = false;
        let mut response_documents = Vec::new();
        for document in &ast_documents.documents {
            let selected_symbols = document
                .summary
                .symbols
                .iter()
                .take(remaining)
                .map(|symbol| json!({
                    "kind": symbol_kind_id(&symbol.kind),
                    "name": symbol.name,
                    "span": span_json(&symbol.span),
                    "selectionSpan": line_span_json(symbol.span.start_line, symbol.span.end_line),
                    "parserVersion": symbol.parser_version,
                    "queryPackVersion": symbol.query_pack_version
                }))
                .collect::<Vec<_>>();
            remaining = remaining.saturating_sub(selected_symbols.len());
            truncated |= document.summary.symbols.len() > selected_symbols.len();
            response_documents.push(json!({
                "path": document.display,
                "language": document.summary.source.language.id(),
                "contentSha256": document.summary.source.sha256,
                "parserVersion": parser_version_string(&document.summary),
                "queryPackVersion": document.summary.versions.query_pack,
                "symbols": selected_symbols,
                "diagnostics": diagnostics_json(&document.summary)
            }));
        }
        Ok(json!({
            "documents": response_documents,
            "limit": limit,
            "trace": ast_trace_json(&config, &ast_documents, truncated)
        }))
    })
    .await
}

async fn call_code_ast_symbols_tool(
    config: MemoryConfig,
    arguments: Value,
) -> Result<Value, MemoryError> {
    ast_mcp_tool_blocking("code.ast.symbols", move || {
        let query =
            optional_string_arg(&arguments, "query").map(|value| value.to_ascii_lowercase());
        let kinds = normalized_string_set_args(&arguments, &["kinds"]);
        let limit = ast_limit(&config, &arguments);
        let mut symbols = Vec::new();
        let ast_documents = ast_documents(&config, &arguments)?;
        for document in &ast_documents.documents {
            for symbol in document.summary.symbols.iter().filter(|symbol| {
                query
                    .as_ref()
                    .is_none_or(|query| symbol.name.to_ascii_lowercase().contains(query))
                    && (kinds.is_empty() || kinds.contains(symbol_kind_id(&symbol.kind)))
            }) {
                if symbols.len() >= limit {
                    break;
                }
                symbols.push(json!({
                    "id": format!("{}:{}:{}", document.display, symbol.name, symbol.rendered_span),
                    "kind": symbol_kind_id(&symbol.kind),
                    "name": symbol.name,
                    "path": document.display,
                    "span": span_json(&symbol.span),
                    "selectionSpan": line_span_json(symbol.span.start_line, symbol.span.end_line),
                    "source": source_json(&document.summary)
                }));
            }
            if symbols.len() >= limit {
                break;
            }
        }
        Ok(json!({
            "symbols": symbols,
            "limit": limit,
            "trace": ast_trace_json(&config, &ast_documents, symbols.len() >= limit)
        }))
    })
    .await
}

async fn call_code_ast_references_tool(
    config: MemoryConfig,
    arguments: Value,
) -> Result<Value, MemoryError> {
    ast_mcp_tool_blocking("code.ast.references", move || {
        let symbol = required_string_arg(&arguments, "symbol")?;
        let limit = ast_limit(&config, &arguments);
        let mut references = Vec::new();
        let ast_documents = ast_documents(&config, &arguments)?;
        for document in &ast_documents.documents {
            for capture in document
                .summary
                .captures
                .iter()
                .filter(|capture| capture.capture_name.starts_with("reference."))
                .filter(|capture| capture_matches_symbol(&capture.text, &symbol))
            {
                if references.len() >= limit {
                    break;
                }
                let (snippet, truncated) =
                    truncate_capture(&capture.text, config.code_intel.ast.max_capture_bytes);
                references.push(json!({
                    "kind": capture.capture_name,
                    "path": document.display,
                    "span": span_json(&capture.span),
                    "snippet": snippet,
                    "truncated": truncated,
                    "source": source_json(&document.summary)
                }));
            }
            if references.len() >= limit {
                break;
            }
        }
        Ok(json!({
            "references": references,
            "confidence": "syntactic",
            "limit": limit,
            "trace": ast_trace_json(&config, &ast_documents, references.len() >= limit)
        }))
    })
    .await
}

async fn call_code_ast_query_tool(
    config: MemoryConfig,
    arguments: Value,
) -> Result<Value, MemoryError> {
    ast_mcp_tool_blocking("code.ast.query", move || {
        let language = required_string_arg(&arguments, "language")?.to_ascii_lowercase();
        let language = SourceLanguage::from_id(&language).ok_or_else(|| {
            MemoryError::InvalidInput(format!("unsupported AST query language `{language}`"))
        })?;
        if !language.supports_ast_queries() {
            return Err(MemoryError::InvalidInput(format!(
                "language `{}` does not support Tree-sitter ad hoc queries",
                language.id()
            )));
        }
        let query = required_string_arg(&arguments, "query")?;
        let limit = ast_limit(&config, &arguments);
        let ast_documents = ast_documents(&config, &arguments)?;
        let mut matches = Vec::new();
        for document in ast_documents
            .documents
            .iter()
            .filter(|document| document.summary.source.language == language)
        {
            let query_matches = run_ad_hoc_query(language, &document.source, &query, limit)
                .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
            for query_match in query_matches {
                if matches.len() >= limit {
                    break;
                }
                matches.push(json!({
                    "path": document.display,
                    "captures": query_match.captures.iter().map(|capture| {
                        let (text, truncated) = truncate_capture(
                            &capture.text,
                            config.code_intel.ast.max_capture_bytes,
                        );
                        json!({
                            "name": capture.capture_name,
                            "text": text,
                            "truncated": truncated,
                            "span": span_json(&capture.span)
                        })
                    }).collect::<Vec<_>>(),
                    "source": source_json(&document.summary)
                }));
            }
            if matches.len() >= limit {
                break;
            }
        }
        Ok(json!({
            "matches": matches,
            "limit": limit,
            "trace": ast_trace_json(&config, &ast_documents, matches.len() >= limit)
        }))
    })
    .await
}

async fn call_code_ast_context_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
    let scope = scope_filter_from_mcp(arguments, true);
    let paths = ast_path_args(arguments)?;
    let limit = ast_limit(config, arguments);
    let symbol_kinds = normalized_string_set_args(arguments, &["symbols"]);
    let repo_root = resolve_code_intel_repo(config, scope.repo.as_deref())?;
    let scope_refs = scope_refs_for_context(&scope, &paths);
    let artifacts = code_intel_artifacts_with_symbol_kinds_blocking(
        repo_root,
        paths,
        scope_refs,
        limit,
        symbol_kinds,
    )
    .await?;
    let trace = artifacts
        .iter()
        .filter(|artifact| artifact.kind == "trace")
        .flat_map(|artifact| artifact.summary.lines().map(str::to_string))
        .collect::<Vec<_>>();
    let mut markdown = String::from("## Structural Context\n\n");
    append_code_intel_artifacts(config, &mut markdown, artifacts);
    Ok(json!({ "markdown": markdown, "trace": trace }))
}

async fn call_code_ast_diagnostics_tool(
    config: MemoryConfig,
    arguments: Value,
) -> Result<Value, MemoryError> {
    ast_mcp_tool_blocking("code.ast.diagnostics", move || {
        let ast_documents = ast_documents(&config, &arguments)?;
        let limit = ast_limit(&config, &arguments);
        let mut diagnostics = Vec::new();
        for document in &ast_documents.documents {
            for diagnostic in &document.summary.diagnostics {
                if diagnostics.len() >= limit {
                    break;
                }
                diagnostics.push(json!({
                    "path": document.display,
                    "kind": diagnostic_kind_id(&diagnostic.kind),
                    "nodeKind": diagnostic.node_kind,
                    "span": span_json(&diagnostic.span),
                    "source": source_json(&document.summary)
                }));
            }
            if diagnostics.len() >= limit {
                break;
            }
        }
        let truncated = ast_documents
            .documents
            .iter()
            .map(|document| document.summary.diagnostics.len())
            .sum::<usize>()
            > diagnostics.len();
        Ok(json!({
            "diagnostics": diagnostics,
            "limit": limit,
            "trace": ast_trace_json(&config, &ast_documents, truncated)
        }))
    })
    .await
}

async fn ast_mcp_tool_blocking<F>(tool_name: &'static str, task: F) -> Result<Value, MemoryError>
where
    F: FnOnce() -> Result<Value, MemoryError> + Send + 'static,
{
    tokio::task::spawn_blocking(task).await.map_err(|error| {
        MemoryError::InvalidInput(format!("{tool_name} analysis task failed: {error}"))
    })?
}

fn ast_documents(config: &MemoryConfig, arguments: &Value) -> Result<AstDocuments, MemoryError> {
    let scope = scope_filter_from_mcp(arguments, false);
    let repo_root = resolve_code_intel_repo(config, scope.repo.as_deref())?;
    let paths = ast_path_args(arguments)?;
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    for path in paths {
        collect_ast_files(
            &repo_root,
            &path,
            config.code_intel.ast.max_files_per_request,
            &mut files,
            &mut warnings,
        )?;
    }
    let mut documents = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(&repo_root)
            .map_err(|_| MemoryError::PathOutsideRepo {
                path: path.clone(),
                repo_root: repo_root.clone(),
            })?
            .to_path_buf();
        let metadata = fs::metadata(&path).map_err(|source| MemoryError::ReadFile {
            path: path.clone(),
            source,
        })?;
        if metadata.len() > config.code_intel.ast.max_file_bytes {
            warnings.push(format!(
                "{} exceeds AST max_file_bytes {}",
                relative.display(),
                config.code_intel.ast.max_file_bytes
            ));
            continue;
        }
        let source = fs::read_to_string(&path).map_err(|source| MemoryError::ReadFile {
            path: path.clone(),
            source,
        })?;
        let summary = parse_path(&relative, &source)
            .map_err(|error| MemoryError::InvalidInput(error.to_string()))?;
        documents.push(AstDocument {
            display: relative.display().to_string(),
            source,
            summary,
        });
    }
    Ok(AstDocuments {
        documents,
        warnings,
    })
}

fn ast_path_args(arguments: &Value) -> Result<Vec<PathBuf>, MemoryError> {
    let paths = string_list_arg(arguments, "paths")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Err(MemoryError::InvalidInput(
            "code.ast tools require at least one path".to_string(),
        ));
    }
    Ok(paths)
}

fn collect_ast_files(
    repo_root: &Path,
    path: &Path,
    max_files: usize,
    files: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) -> Result<(), MemoryError> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|source| MemoryError::ResolvePath {
            path: candidate.clone(),
            source,
        })?;
    if !resolved.starts_with(repo_root) {
        return Err(MemoryError::PathOutsideRepo {
            path: resolved,
            repo_root: repo_root.to_path_buf(),
        });
    }
    if resolved.is_file() {
        if ast_file_is_supported(repo_root, &resolved)? && files.len() < max_files {
            files.push(resolved);
        }
        return Ok(());
    }
    if !resolved.is_dir() {
        return Ok(());
    }
    if files.len() >= max_files {
        return Ok(());
    }
    let relative = resolved
        .strip_prefix(repo_root)
        .map_err(|_| MemoryError::PathOutsideRepo {
            path: resolved.clone(),
            repo_root: repo_root.to_path_buf(),
        })?;
    if let Some(component) = skipped_directory_name(&resolved) {
        warnings.push(format!(
            "{} skipped directory `{component}`",
            relative.display()
        ));
        return Ok(());
    }
    let mut entries = fs::read_dir(&resolved)
        .map_err(|source| MemoryError::ReadFile {
            path: resolved.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::ReadFile {
            path: resolved.clone(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        if files.len() >= max_files {
            break;
        }
        let file_type = entry.file_type().map_err(|source| MemoryError::ReadFile {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() {
            collect_ast_files(repo_root, &entry.path(), max_files, files, warnings)?;
        } else if file_type.is_file() && ast_file_is_supported(repo_root, &entry.path())? {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn ast_file_is_supported(repo_root: &Path, path: &Path) -> Result<bool, MemoryError> {
    let relative = path
        .strip_prefix(repo_root)
        .map_err(|_| MemoryError::PathOutsideRepo {
            path: path.to_path_buf(),
            repo_root: repo_root.to_path_buf(),
        })?;
    Ok(crate::opensymphony_code_intel::detect_language(relative).is_some())
}

fn capture_matches_symbol(capture_text: &str, symbol: &str) -> bool {
    capture_text.match_indices(symbol).any(|(start, value)| {
        let end = start + value.len();
        let before = capture_text[..start].chars().next_back();
        let after = capture_text[end..].chars().next();
        before.is_none_or(|value| !is_identifier_char(value))
            && after.is_none_or(|value| !is_identifier_char(value))
    })
}

fn is_identifier_char(value: char) -> bool {
    value == '_' || value.is_alphanumeric()
}

fn ast_limit(config: &MemoryConfig, arguments: &Value) -> usize {
    usize_arg(arguments, "limit", 50).min(config.code_intel.ast.max_matches_per_request)
}

fn ast_language_ids() -> Vec<&'static str> {
    vec!["rust", "typescript", "tsx", "javascript", "jsx", "python"]
}

fn ast_query_pack_versions() -> Value {
    json!({
        "rust": RUST_QUERY_PACK_VERSION,
        "typescript": TYPESCRIPT_QUERY_PACK_VERSION,
        "tsx": TSX_QUERY_PACK_VERSION,
        "javascript": JAVASCRIPT_QUERY_PACK_VERSION,
        "jsx": JSX_QUERY_PACK_VERSION,
        "python": PYTHON_QUERY_PACK_VERSION
    })
}

fn ast_limits_json(config: &MemoryConfig) -> Value {
    json!({
        "maxFileBytes": config.code_intel.ast.max_file_bytes,
        "maxFilesPerRequest": config.code_intel.ast.max_files_per_request,
        "maxMatchesPerRequest": config.code_intel.ast.max_matches_per_request,
        "maxCaptureBytes": config.code_intel.ast.max_capture_bytes
    })
}

fn ast_trace_json(
    config: &MemoryConfig,
    ast_documents: &AstDocuments,
    truncated: bool,
) -> Vec<String> {
    let documents = &ast_documents.documents;
    let mut trace = vec![
        format!("parsed {} file(s)", documents.len()),
        format!(
            "max files per request {}",
            config.code_intel.ast.max_files_per_request
        ),
        format!(
            "max matches per request {}",
            config.code_intel.ast.max_matches_per_request
        ),
    ];
    if truncated {
        trace.push("truncated by limit".to_string());
    }
    trace.extend(
        ast_documents
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}")),
    );
    trace.extend(documents.iter().map(|document| {
        format!(
            "{} lines {}-{} parser {} query-pack {} content sha256:{}",
            document.display,
            1,
            document.source.lines().count().max(1),
            parser_version_string(&document.summary),
            document.summary.versions.query_pack,
            document.summary.source.sha256
        )
    }));
    trace
}

fn source_json(summary: &ParsedDocumentSummary) -> Value {
    json!({
        "contentSha256": summary.source.sha256,
        "parserVersion": parser_version_string(summary),
        "queryPackVersion": summary.versions.query_pack
    })
}

fn parser_version_string(summary: &ParsedDocumentSummary) -> String {
    format!(
        "{}:{}",
        summary.versions.grammar, summary.versions.tree_sitter
    )
}

fn diagnostics_json(summary: &ParsedDocumentSummary) -> Vec<Value> {
    summary
        .diagnostics
        .iter()
        .map(|diagnostic| {
            json!({
                "kind": diagnostic_kind_id(&diagnostic.kind),
                "nodeKind": diagnostic.node_kind,
                "span": span_json(&diagnostic.span)
            })
        })
        .collect()
}

fn span_json(span: &crate::opensymphony_code_intel::SourceSpan) -> Value {
    json!({
        "startLine": span.start_line,
        "startColumn": span.start_column,
        "endLine": span.end_line,
        "endColumn": span.end_column,
        "startByte": span.start_byte,
        "endByte": span.end_byte
    })
}

fn line_span_json(start_line: usize, end_line: usize) -> Value {
    json!({ "startLine": start_line, "endLine": end_line })
}

fn truncate_capture(text: &str, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text.to_string(), false);
    }
    let end = text
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= max_bytes)
        .last()
        .unwrap_or(0);
    (text[..end].to_string(), true)
}

async fn call_memory_capture_tool(
    config: &MemoryConfig,
    arguments: &Value,
    central_config_path: Option<&Path>,
    resolved_workflow: Option<&ResolvedWorkflow>,
) -> Result<Value, MemoryError> {
    if central_config_path.is_some() && resolved_workflow.is_none() {
        let _ = load_central_resolved_workflow(&config.repo_root, central_config_path)?;
    }
    let identifiers = issue_ids_from_mcp(config, arguments)?;
    if identifiers.is_empty() {
        return Err(MemoryError::InvalidInput(
            "memory.capture requires issue, issues, issuesFile, or issueRange".to_string(),
        ));
    }
    let source = if let Some(source_file) = optional_string_arg(arguments, "sourceFile")
        .or_else(|| optional_string_arg(arguments, "source_file"))
    {
        load_source_file(&repo_existing_path(config, &source_file)?)?
    } else if let Some(workflow) = resolved_workflow {
        let client = linear_client_from_resolved_workflow(workflow)?;
        load_linear_source_from_client(&client, &identifiers).await?
    } else {
        load_linear_source(&config.repo_root, None, central_config_path, &identifiers).await?
    };
    let selection = IssueSelection {
        identifiers,
        milestone: optional_string_arg(arguments, "milestone"),
        state: optional_string_arg(arguments, "state"),
        before_date: optional_string_arg(arguments, "beforeDate")
            .or_else(|| optional_string_arg(arguments, "before_date"))
            .map(|value| NaiveDate::parse_from_str(&value, "%Y-%m-%d"))
            .transpose()
            .map_err(|error| MemoryError::InvalidInput(format!("invalid beforeDate: {error}")))?,
        before_issue: optional_string_arg(arguments, "beforeIssue")
            .or_else(|| optional_string_arg(arguments, "before_issue")),
        area: optional_string_arg(arguments, "area"),
        since_last_sync: false,
    };
    let write = !bool_arg(arguments, "dryRun") && !bool_arg(arguments, "dry_run");
    let discover_github = !bool_arg(arguments, "noGithub") && !bool_arg(arguments, "no_github");
    let plan = plan_capture(config, &source, &selection, write, discover_github)?;
    if !write {
        return Ok(json!({
            "dryRun": true,
            "plan": capture_plan_json(config, &plan)
        }));
    }
    let report = write_capture_plan(config, &plan, bool_arg(arguments, "force"))?;
    Ok(json!({
        "dryRun": false,
        "plan": capture_plan_json(config, &plan),
        "write": capture_write_report_json(config, report)
    }))
}

fn call_memory_sync_docs_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
    let selection = IssueSelection {
        identifiers: issue_ids_from_mcp(config, arguments)?,
        area: optional_string_arg(arguments, "area"),
        since_last_sync: bool_arg(arguments, "sinceLastSync")
            || bool_arg(arguments, "since_last_sync"),
        ..IssueSelection::default()
    };
    let write = !bool_arg(arguments, "dryRun") && !bool_arg(arguments, "dry_run");
    let with_diagrams = bool_arg(arguments, "withDiagrams") || bool_arg(arguments, "with_diagrams");
    let plan = plan_docs_sync(config, &selection, write, with_diagrams)?;
    if !write {
        return Ok(json!({
            "dryRun": true,
            "plan": docs_sync_plan_json(config, &plan),
            "written": []
        }));
    }
    let written = write_docs_sync_plan(config, &plan)?;
    Ok(json!({
        "dryRun": false,
        "plan": docs_sync_plan_json(config, &plan),
        "written": paths_for_json(config, &written)
    }))
}

fn call_memory_lint_tool(config: &MemoryConfig, arguments: &Value) -> Result<Value, MemoryError> {
    let public_docs = bool_arg(arguments, "publicDocs") || bool_arg(arguments, "public_docs");
    let report = if bool_arg(arguments, "okf") {
        let bundle_root = optional_string_arg(arguments, "bundleRoot")
            .or_else(|| optional_string_arg(arguments, "bundle_root"))
            .map(|path| repo_existing_path(config, &path))
            .transpose()?
            .unwrap_or_else(|| config.memory_root.clone());
        lint_okf_bundle(&bundle_root, public_docs)?
    } else {
        lint(config, public_docs)?
    };
    Ok(json!({
        "findingCount": report.findings.len(),
        "findings": report.findings.into_iter().map(|finding| {
            json!({
                "severity": match finding.severity {
                    LintSeverity::Info => "info",
                    LintSeverity::Warn => "warn",
                    LintSeverity::Error => "error",
                },
                "path": finding.path.as_ref().map(|path| path_for_json(config, path)),
                "message": finding.message,
                "nextCommand": finding.next_command
            })
        }).collect::<Vec<_>>()
    }))
}

fn call_memory_reindex_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
    let report = if bool_arg(arguments, "fromOkf") || bool_arg(arguments, "from_okf") {
        let bundle_root = optional_string_arg(arguments, "bundleRoot")
            .or_else(|| optional_string_arg(arguments, "bundle_root"))
            .map(|path| repo_existing_path(config, &path))
            .transpose()?
            .unwrap_or_else(|| config.memory_root.clone());
        refresh_memory_index_from_okf(config, &bundle_root)?
    } else {
        refresh_memory_index(config)?
    };
    Ok(memory_reindex_report_json(config, report))
}

fn call_memory_export_okf_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
    let visibility = memory_visibility_arg(arguments)?;
    let output = optional_string_arg(arguments, "output").map(PathBuf::from);
    let report = export_okf_bundle(config, visibility, output.as_deref())?;
    Ok(okf_export_report_json(config, visibility, report))
}

fn call_memory_import_okf_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
    let bundle = optional_string_arg(arguments, "bundleRoot")
        .map(PathBuf::from)
        .ok_or_else(|| {
            MemoryError::InvalidInput("missing string argument `bundleRoot`".to_string())
        })?;
    let report = import_okf_bundle(config, &bundle, bool_arg(arguments, "force"))?;
    Ok(okf_import_report_json(config, report))
}

async fn call_memory_ingest_code_intel_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
    let scope = scope_filter_from_mcp(arguments, false);
    let paths = string_list_arg(arguments, "paths")
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let limit = usize_arg(arguments, "limit", 10);
    let scope_refs = scope_refs_for_context(&scope, &paths);
    let persist = bool_arg(arguments, "persist");
    let (artifacts, persist_report) = if persist {
        let languages = normalized_string_set_args(arguments, &["languages"]);
        let symbols = normalized_string_set_args(arguments, &["symbols"]);
        let query_packs = string_set_args(
            arguments,
            &["queryPack", "queryPacks", "query_pack", "query_packs"],
        );
        let resolved_config = memory_config_for_repository(config, scope.repo.as_deref())?;
        let (artifacts, report) = code_intel_persist_artifacts_blocking(CodeIntelPersistRequest {
            config: resolved_config,
            scope: scope.clone(),
            paths,
            scope_refs,
            limit,
            languages,
            symbols,
            query_packs,
        })
        .await?;
        (artifacts, Some(report))
    } else {
        let repo_root = resolve_code_intel_repo(config, scope.repo.as_deref())?;
        let artifacts = code_intel_artifacts_blocking(repo_root, paths, scope_refs, limit).await?;
        (artifacts, None)
    };
    let (parsed_files, persisted_rows, stale_rows, skipped_files, diagnostics) =
        if let Some(report) = persist_report {
            (
                report.parsed_files,
                report.persisted_documents
                    + report.persisted_symbols
                    + report.persisted_edges
                    + report.persisted_diagnostics,
                report.stale_rows,
                report.skipped_files,
                report.diagnostics,
            )
        } else {
            (0, 0, 0, Vec::new(), Vec::new())
        };
    Ok(json!({
        "persisted": persist,
        "parsedFiles": parsed_files,
        "persistedRows": persisted_rows,
        "staleRows": stale_rows,
        "skippedFiles": skipped_files,
        "diagnostics": diagnostics,
        "artifactCount": artifacts.len(),
        "artifacts": artifacts.into_iter().map(|artifact| json!({
            "provider": artifact.provider,
            "kind": artifact.kind,
            "title": artifact.title,
            "path": artifact.path.as_ref().map(|path| path_for_json(config, path)),
            "commitSha": artifact.commit_sha,
            "summary": artifact.summary,
            "sourceRefs": artifact.source_refs.into_iter().map(|source| json!({
                "kind": source.kind,
                "id": source.id,
                "repoId": source.repo_id,
                "symbolKey": source.symbol_key
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }))
}

struct CodeIntelPersistencePlan {
    artifacts: Vec<CodeIntelArtifact>,
    documents: Vec<CodeIntelDocumentInput>,
    skipped_files: Vec<String>,
    skipped_file_inputs: Vec<CodeIntelSkippedFileInput>,
    diagnostics: Vec<String>,
}

struct CodeIntelPersistRequest {
    config: MemoryConfig,
    scope: MemoryScopeFilter,
    paths: Vec<PathBuf>,
    scope_refs: Vec<CodeIntelScope>,
    limit: usize,
    languages: BTreeSet<String>,
    symbols: BTreeSet<String>,
    query_packs: BTreeSet<String>,
}

async fn code_intel_persist_artifacts_blocking(
    request: CodeIntelPersistRequest,
) -> Result<
    (
        Vec<CodeIntelArtifact>,
        crate::opensymphony_memory::CodeIntelPersistReport,
    ),
    MemoryError,
> {
    tokio::task::spawn_blocking(move || {
        let repo_root = resolve_code_intel_repo(&request.config, request.scope.repo.as_deref())?;
        let plan = code_intel_documents_for_persistence(&request)?;
        let repo_id = repo_id_for_code_intel(&request.config, &request.scope);
        let commit_sha = git_commit_sha_for_repo(&repo_root);
        let worktree_dirty = git_worktree_dirty(&repo_root);
        let mut report = persist_code_intel_documents(
            &request.config,
            CodeIntelPersistBatch {
                repo_id: repo_id.clone(),
                commit_sha: commit_sha.clone(),
                worktree_dirty,
                documents: plan.documents,
            },
        )?;
        persist_code_intel_skipped_files(
            &request.config,
            &repo_id,
            commit_sha.as_deref(),
            worktree_dirty,
            &plan.skipped_file_inputs,
        )?;
        report.skipped_files = plan.skipped_files;
        report.diagnostics = plan.diagnostics;
        Ok((plan.artifacts, report))
    })
    .await
    .map_err(|error| {
        MemoryError::InvalidInput(format!(
            "code-intelligence persistence task failed: {error}"
        ))
    })?
}

fn code_intel_documents_for_persistence(
    request: &CodeIntelPersistRequest,
) -> Result<CodeIntelPersistencePlan, MemoryError> {
    let repo_root = resolve_code_intel_repo(&request.config, request.scope.repo.as_deref())?;
    let repo_id = repo_id_for_code_intel(&request.config, &request.scope);
    let mut artifacts = Vec::new();
    let mut documents = Vec::new();
    let mut skipped_files = Vec::new();
    let mut skipped_file_inputs = Vec::new();
    let mut diagnostics = Vec::new();
    let mut parsed_files = 0usize;
    let mut query_runs = 0usize;
    let mut remaining_symbols = request.limit;
    let commit_sha = git_commit_sha_for_repo(&repo_root);
    for path in &request.paths {
        let resolved = repo_existing_path_from_path(&request.config, path)?;
        let relative = resolved
            .strip_prefix(&repo_root)
            .map_err(|_| MemoryError::PathOutsideRepo {
                path: resolved.clone(),
                repo_root: repo_root.clone(),
            })?
            .to_path_buf();
        let relative_display = relative.to_string_lossy().to_string();
        if resolved.is_dir() {
            skipped_files.push(format!("{relative_display}: directory"));
            continue;
        }
        let Some(language) = crate::opensymphony_code_intel::detect_language(&relative) else {
            record_skipped_code_intel_file(
                &mut skipped_files,
                &mut skipped_file_inputs,
                &relative,
                &relative_display,
                &resolved,
                "unsupported language",
            )?;
            continue;
        };
        let language_id = source_language_id(language);
        if !request.languages.is_empty() && !request.languages.contains(language_id) {
            record_skipped_code_intel_file(
                &mut skipped_files,
                &mut skipped_file_inputs,
                &relative,
                &relative_display,
                &resolved,
                &format!("language `{language_id}` not selected"),
            )?;
            continue;
        }
        let source = fs::read_to_string(&resolved).map_err(|source| MemoryError::ReadFile {
            path: resolved.clone(),
            source,
        })?;
        let summary = match parse_path(&relative, &source) {
            Ok(summary) => summary,
            Err(error) => {
                record_skipped_code_intel_file(
                    &mut skipped_files,
                    &mut skipped_file_inputs,
                    &relative,
                    &relative_display,
                    &resolved,
                    "parse failed",
                )?;
                diagnostics.push(format!("{relative_display}: {error}"));
                continue;
            }
        };
        if !request.query_packs.is_empty()
            && !request.query_packs.contains(&summary.versions.query_pack)
        {
            record_skipped_code_intel_file(
                &mut skipped_files,
                &mut skipped_file_inputs,
                &relative,
                &relative_display,
                &resolved,
                &format!("query pack `{}` not selected", summary.versions.query_pack),
            )?;
            continue;
        }
        for diagnostic in &summary.diagnostics {
            diagnostics.push(format!(
                "{relative_display}: {} at {}",
                diagnostic.node_kind, diagnostic.rendered_span
            ));
        }
        parsed_files += 1;
        query_runs += 1;
        let (summary_artifacts, used_symbols) = code_intel_artifacts_for_summary(
            &summary,
            &relative,
            &repo_id,
            &request.scope_refs,
            commit_sha.clone(),
            &request.symbols,
            remaining_symbols,
        );
        remaining_symbols = remaining_symbols.saturating_sub(used_symbols);
        artifacts.extend(summary_artifacts);
        documents.push(code_intel_document_input(
            relative,
            source,
            summary,
            &request.symbols,
            used_symbols,
        ));
    }
    artifacts.push(code_intel_trace_artifact(
        &request.scope_refs,
        parsed_files,
        query_runs,
        &skipped_files,
    ));
    Ok(CodeIntelPersistencePlan {
        artifacts,
        documents,
        skipped_files,
        skipped_file_inputs,
        diagnostics,
    })
}

fn record_skipped_code_intel_file(
    skipped_files: &mut Vec<String>,
    skipped_file_inputs: &mut Vec<CodeIntelSkippedFileInput>,
    relative: &Path,
    relative_display: &str,
    resolved: &Path,
    reason: &str,
) -> Result<(), MemoryError> {
    let bytes = fs::read(resolved).map_err(|source| MemoryError::ReadFile {
        path: resolved.to_path_buf(),
        source,
    })?;
    skipped_files.push(format!("{relative_display}: {reason}"));
    skipped_file_inputs.push(CodeIntelSkippedFileInput {
        path: relative.to_path_buf(),
        reason: reason.to_string(),
        content_sha256: sha256_bytes_hex(&bytes),
    });
    Ok(())
}

fn code_intel_artifacts_for_summary(
    summary: &ParsedDocumentSummary,
    relative_path: &Path,
    repo_id: &str,
    scope_refs: &[CodeIntelScope],
    commit_sha: Option<String>,
    symbols: &BTreeSet<String>,
    symbol_limit: usize,
) -> (Vec<CodeIntelArtifact>, usize) {
    let relative_display = relative_path.to_string_lossy().to_string();
    let diagnostic_summary = diagnostics_summary(&summary.diagnostics);
    let mut artifacts = vec![CodeIntelArtifact {
        provider: summary.versions.provider.clone(),
        kind: "ast-summary".to_string(),
        scope_refs: scope_refs.to_vec(),
        source_refs: vec![CodeIntelSourceRef {
            kind: "path".to_string(),
            id: relative_display.clone(),
            url: None,
            repo_id: Some(repo_id.to_string()),
            symbol_key: None,
        }],
        path: Some(relative_path.to_path_buf()),
        commit_sha: commit_sha.clone(),
        title: relative_display.to_string(),
        summary: format!(
            "- Language: {}\n- Content hash: sha256:{}\n- Parser: {} ({}, {})\n- Query pack: {}\n- Diagnostics: {diagnostic_summary}",
            source_language_id(summary.source.language),
            summary.source.sha256,
            summary.versions.provider,
            summary.versions.grammar,
            summary.versions.tree_sitter,
            summary.versions.query_pack,
        ),
    }];

    let mut key_counts = BTreeSet::new();
    let symbols_with_keys = summary
        .symbols
        .iter()
        .map(|symbol| {
            let base_key = sha256_hex(
                &[
                    repo_id,
                    &relative_display,
                    source_language_id(summary.source.language),
                    symbol_kind_id(&symbol.kind),
                    &symbol.container_chain.join("\u{1f}"),
                    &symbol.name,
                ]
                .join("\u{1f}"),
            );
            let mut symbol_key = base_key.clone();
            let mut ordinal = 1usize;
            while !key_counts.insert(symbol_key.clone()) {
                ordinal += 1;
                symbol_key = format!("{base_key}#{ordinal}");
            }
            (symbol, symbol_key)
        })
        .filter(|(symbol, _)| symbols.is_empty() || symbols.contains(symbol_kind_id(&symbol.kind)))
        .take(symbol_limit)
        .collect::<Vec<_>>();

    if symbols_with_keys.is_empty() {
        return (artifacts, 0);
    }

    let used_symbols = symbols_with_keys.len();
    let rendered_symbols = symbols_with_keys
        .iter()
        .map(|(symbol, _)| {
            format!(
                "- {} `{}` at {}:{}",
                symbol_kind_id(&symbol.kind),
                symbol.name,
                relative_display,
                symbol.rendered_span
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    artifacts.push(CodeIntelArtifact {
        provider: PROVIDER_NAME.to_string(),
        kind: "ast-symbols".to_string(),
        scope_refs: scope_refs.to_vec(),
        source_refs: symbols_with_keys
            .iter()
            .map(|(symbol, symbol_key)| CodeIntelSourceRef {
                kind: "code-symbol".to_string(),
                id: format!("{relative_display}:{}", symbol.rendered_span),
                url: None,
                repo_id: Some(repo_id.to_string()),
                symbol_key: Some(symbol_key.clone()),
            })
            .collect(),
        path: Some(relative_path.to_path_buf()),
        commit_sha,
        title: format!("Symbols in {relative_display}"),
        summary: rendered_symbols,
    });
    (artifacts, used_symbols)
}

fn diagnostics_summary(diagnostics: &[crate::opensymphony_code_intel::AstDiagnostic]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == AstDiagnosticKind::Error)
        .count();
    let missing = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == AstDiagnosticKind::Missing)
        .count();
    format!("{errors} ERROR, {missing} MISSING")
}

fn code_intel_trace_artifact(
    scope_refs: &[CodeIntelScope],
    parsed_files: usize,
    query_runs: usize,
    skipped_files: &[String],
) -> CodeIntelArtifact {
    let fallback = if skipped_files.is_empty() {
        "fallback: CodebaseAnalyzer not used".to_string()
    } else {
        format!(
            "fallback: CodebaseAnalyzer not used in persistent ingest ({})",
            skipped_files.join("; ")
        )
    };
    CodeIntelArtifact {
        provider: "composite-code-intel".to_string(),
        kind: "trace".to_string(),
        scope_refs: scope_refs.to_vec(),
        source_refs: Vec::new(),
        path: None,
        commit_sha: None,
        title: "Code-intelligence trace".to_string(),
        summary: format!(
            "- parse: parsed {parsed_files} file(s)\n- query: ran {query_runs} Tree-sitter query pack(s)\n- {fallback}"
        ),
    }
}

fn code_intel_document_input(
    path: PathBuf,
    source: String,
    summary: ParsedDocumentSummary,
    symbols: &BTreeSet<String>,
    _symbol_limit: usize,
) -> CodeIntelDocumentInput {
    let language = source_language_id(summary.source.language).to_string();
    let parser_version = format!(
        "{}:{}",
        summary.versions.grammar, summary.versions.tree_sitter
    );
    let all_symbols = summary
        .symbols
        .iter()
        .filter(|symbol| symbols.is_empty() || symbols.contains(symbol_kind_id(&symbol.kind)))
        .map(|symbol| {
            let snippet = source
                .get(symbol.span.start_byte..symbol.span.end_byte)
                .unwrap_or(symbol.name.as_str());
            CodeIntelSymbolInput {
                kind: symbol_kind_id(&symbol.kind).to_string(),
                name: symbol.name.clone(),
                container_chain: symbol.container_chain.clone(),
                signature: None,
                start_line: symbol.span.start_line,
                start_col: symbol.span.start_column,
                end_line: symbol.span.end_line,
                end_col: symbol.span.end_column,
                start_byte: symbol.span.start_byte,
                end_byte: symbol.span.end_byte,
                selection_start_line: symbol.span.start_line,
                selection_end_line: symbol.span.end_line,
                snippet_sha256: sha256_hex(snippet),
            }
        })
        .collect::<Vec<_>>();
    CodeIntelDocumentInput {
        path,
        language,
        content_sha256: summary.source.sha256.clone(),
        parser_id: summary.versions.provider.clone(),
        parser_version: parser_version.clone(),
        query_pack_version: summary.versions.query_pack.clone(),
        byte_len: summary.source.bytes,
        line_count: source.lines().count(),
        symbols: all_symbols,
        edges: summary
            .captures
            .iter()
            .filter_map(code_intel_edge_input)
            .collect(),
        diagnostics: summary
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let (kind, severity) = match diagnostic.kind {
                    AstDiagnosticKind::Error => ("error", "error"),
                    AstDiagnosticKind::Missing => ("missing", "warning"),
                };
                CodeIntelDiagnosticInput {
                    kind: kind.to_string(),
                    severity: severity.to_string(),
                    message: format!("{} parse diagnostic", diagnostic.node_kind),
                    start_line: diagnostic.span.start_line,
                    start_col: diagnostic.span.start_column,
                    end_line: diagnostic.span.end_line,
                    end_col: diagnostic.span.end_column,
                    start_byte: diagnostic.span.start_byte,
                    end_byte: diagnostic.span.end_byte,
                }
            })
            .collect(),
    }
}

fn code_intel_edge_input(capture: &CaptureRecord) -> Option<CodeIntelEdgeInput> {
    if !matches!(
        capture.capture_name.split('.').next(),
        Some("reference" | "import" | "export" | "test")
    ) {
        return None;
    }
    Some(CodeIntelEdgeInput {
        edge_kind: capture.capture_name.clone(),
        target_hint: Some(capture.text.clone()),
        confidence: format!("query_pack:{}", capture.query_name),
        start_line: capture.span.start_line,
        start_col: capture.span.start_column,
        end_line: capture.span.end_line,
        end_col: capture.span.end_column,
        start_byte: capture.span.start_byte,
        end_byte: capture.span.end_byte,
    })
}

fn string_set_args(arguments: &Value, keys: &[&str]) -> BTreeSet<String> {
    keys.iter()
        .flat_map(|key| string_list_arg(arguments, key))
        .collect()
}

fn normalized_string_set_args(arguments: &Value, keys: &[&str]) -> BTreeSet<String> {
    keys.iter()
        .flat_map(|key| string_list_arg(arguments, key))
        .map(|value| value.to_ascii_lowercase())
        .collect()
}

fn repo_id_for_code_intel(config: &MemoryConfig, scope: &MemoryScopeFilter) -> String {
    scope
        .repo
        .clone()
        .or_else(|| config.default_repository_id.clone())
        .unwrap_or_else(|| {
            config
                .repo_root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("repo")
                .to_string()
        })
}

fn git_commit_sha_for_repo(repo_root: &Path) -> Option<String> {
    let output = process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_worktree_dirty(repo_root: &Path) -> bool {
    process::Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=no"])
        .current_dir(repo_root)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

fn source_language_id(language: SourceLanguage) -> &'static str {
    match language {
        SourceLanguage::Rust => "rust",
        SourceLanguage::TypeScript => "typescript",
        SourceLanguage::Tsx => "tsx",
        SourceLanguage::JavaScript => "javascript",
        SourceLanguage::Jsx => "jsx",
        SourceLanguage::Python => "python",
        SourceLanguage::Json => "json",
        SourceLanguage::Yaml => "yaml",
        SourceLanguage::Toml => "toml",
        SourceLanguage::Markdown => "markdown",
    }
}

fn symbol_kind_id(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "module",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Type => "type",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Constructor => "constructor",
        SymbolKind::Field => "field",
        SymbolKind::Variable => "variable",
        SymbolKind::Constant => "constant",
        SymbolKind::Test => "test",
        SymbolKind::Document => "document",
    }
}

fn diagnostic_kind_id(kind: &AstDiagnosticKind) -> &'static str {
    match kind {
        AstDiagnosticKind::Error => "error",
        AstDiagnosticKind::Missing => "missing",
    }
}

fn mcp_text(text: String) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn search_results_json(
    config: &MemoryConfig,
    results: &[crate::opensymphony_memory::SearchResult],
) -> Vec<Value> {
    results
        .iter()
        .map(|result| {
            json!({
                "issueKey": result.issue_key.clone(),
                "title": result.title.clone(),
                "capsulePath": path_for_json(config, &result.capsule_path),
                "areas": result.areas.clone(),
                "snippet": result.snippet.clone()
            })
        })
        .collect()
}

fn capture_plan_json(
    config: &MemoryConfig,
    plan: &crate::opensymphony_memory::CapturePlan,
) -> Value {
    json!({
        "write": plan.write,
        "selected": plan.selected.iter().map(|issue| json!({
            "issueKey": issue.issue.identifier.clone(),
            "title": issue.issue.title.clone(),
            "capsulePath": path_for_json(config, &issue.capsule_path),
            "areas": issue.areas.clone(),
            "docsTargets": paths_for_json(config, &issue.docs_targets),
            "alreadyCaptured": issue.already_captured,
            "stale": issue.stale,
            "warningCount": issue.warnings.len(),
            "warnings": issue.warnings.clone()
        })).collect::<Vec<_>>(),
        "warnings": plan.warnings.clone()
    })
}

fn capture_write_report_json(
    config: &MemoryConfig,
    report: crate::opensymphony_memory::CaptureWriteReport,
) -> Value {
    json!({
        "writtenCapsules": paths_for_json(config, &report.written_capsules),
        "indexPath": path_for_json(config, &report.index_path),
        "markdownIndexes": paths_for_json(config, &report.markdown_indexes),
        "milestoneNodes": paths_for_json(config, &report.milestone_nodes),
        "warnings": report.warnings
    })
}

fn docs_sync_plan_json(config: &MemoryConfig, plan: &DocsSyncPlan) -> Value {
    json!({
        "write": plan.write,
        "selectedIssueKeys": plan.selected_issue_keys.clone(),
        "warnings": plan.warnings.clone(),
        "targets": plan.targets.iter().map(|target| json!({
            "area": target.area.clone(),
            "title": target.title.clone(),
            "path": path_for_json(config, &target.path),
            "visibility": target.visibility.as_str(),
            "create": target.create,
            "issueKeys": target.issue_keys.clone(),
            "diff": target.diff.clone()
        })).collect::<Vec<_>>()
    })
}

fn memory_reindex_report_json(config: &MemoryConfig, report: MemoryReindexReport) -> Value {
    json!({
        "issueCount": report.issue_count,
        "warningCount": report.warning_count,
        "indexPath": path_for_json(config, &report.index_path),
        "markdownIndexes": paths_for_json(config, &report.markdown_indexes)
    })
}

fn okf_export_report_json(
    config: &MemoryConfig,
    visibility: MemoryVisibility,
    report: crate::opensymphony_memory::OkfExportReport,
) -> Value {
    json!({
        "outputPath": path_for_json(config, &report.output_path),
        "visibility": visibility.as_str(),
        "copiedFiles": paths_for_json(config, &report.copied_files),
        "skippedPrivateFiles": paths_for_json(config, &report.skipped_private_files),
        "findingCount": report.finding_count
    })
}

fn okf_import_report_json(
    config: &MemoryConfig,
    report: crate::opensymphony_memory::OkfImportReport,
) -> Value {
    json!({
        "sourcePath": path_for_json(config, &report.source_path),
        "targetPath": path_for_json(config, &report.target_path),
        "copiedFiles": paths_for_json(config, &report.copied_files),
        "findingCount": report.finding_count,
        "reindex": memory_reindex_report_json(config, report.reindex)
    })
}

fn issue_ids_from_mcp(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Vec<String>, MemoryError> {
    let issue = optional_string_arg(arguments, "issue")
        .or_else(|| optional_string_arg(arguments, "workItem"))
        .or_else(|| optional_string_arg(arguments, "work_item"));
    let issues = arguments.get("issues").and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Array(_) => Some(string_list_arg(arguments, "issues").join(",")),
        _ => None,
    });
    let issues_file = optional_string_arg(arguments, "issuesFile")
        .or_else(|| optional_string_arg(arguments, "issues_file"))
        .map(|path| repo_existing_path(config, &path))
        .transpose()?;
    let issue_range = optional_string_arg(arguments, "issueRange")
        .or_else(|| optional_string_arg(arguments, "issue_range"));
    collect_issue_ids(
        issue.as_deref(),
        issues.as_deref(),
        issues_file.as_deref(),
        issue_range.as_deref(),
    )
}

fn paths_for_json(config: &MemoryConfig, paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path_for_json(config, path))
        .collect()
}

fn repo_existing_path(config: &MemoryConfig, value: &str) -> Result<PathBuf, MemoryError> {
    repo_existing_path_from_path(config, Path::new(value))
}

fn repo_existing_path_from_path(
    config: &MemoryConfig,
    path: &Path,
) -> Result<PathBuf, MemoryError> {
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        config.repo_root.join(path)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|source| MemoryError::ResolvePath {
            path: candidate.clone(),
            source,
        })?;
    let repo_root = config
        .repo_root
        .canonicalize()
        .map_err(|source| MemoryError::ResolvePath {
            path: config.repo_root.clone(),
            source,
        })?;
    if !resolved.starts_with(&repo_root) {
        return Err(MemoryError::PathOutsideRepo {
            path: resolved,
            repo_root,
        });
    }
    Ok(resolved)
}

fn memory_config_for_repository(
    config: &MemoryConfig,
    repository_id: Option<&str>,
) -> Result<MemoryConfig, MemoryError> {
    let repository_id = repository_id
        .filter(|value| !value.trim().is_empty())
        .or(config.default_repository_id.as_deref());
    let Some(repository_id) = repository_id else {
        return Ok(config.clone());
    };
    let Some(source) = config.repository_sources.get(repository_id) else {
        if config.repository_sources.is_empty() {
            return Ok(config.clone());
        }
        return Err(MemoryError::InvalidInput(format!(
            "unknown canonical repository id `{repository_id}`"
        )));
    };
    let mut resolved = config.clone();
    resolved.repo_root = source.root.clone();
    Ok(resolved)
}

fn context_source_from_mcp(arguments: &Value) -> SourceFile {
    let Some(current_issue) = arguments.get("currentIssue") else {
        return SourceFile::default();
    };
    let identifier = optional_string_arg(current_issue, "identifier")
        .or_else(|| optional_string_arg(arguments, "issue"))
        .unwrap_or_default();
    if identifier.is_empty() {
        return SourceFile::default();
    }
    SourceFile {
        issues: vec![IssueEvidence {
            id: optional_string_arg(current_issue, "id"),
            identifier,
            title: optional_string_arg(current_issue, "title").unwrap_or_default(),
            description: optional_string_arg(current_issue, "description"),
            state: optional_string_arg(current_issue, "state"),
            labels: string_list_arg(current_issue, "labels"),
            children: issue_links_arg(current_issue, "children"),
            blocked_by: issue_links_arg(current_issue, "blockedBy"),
            ..IssueEvidence::default()
        }],
        ..SourceFile::default()
    }
}

fn issue_links_arg(arguments: &Value, key: &str) -> Vec<IssueLinkEvidence> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let identifier = optional_string_arg(value, "identifier")?;
            Some(IssueLinkEvidence {
                id: optional_string_arg(value, "id"),
                identifier,
                state: optional_string_arg(value, "state"),
                ..IssueLinkEvidence::default()
            })
        })
        .collect()
}

fn append_code_intel_context(
    config: &MemoryConfig,
    output: &mut String,
    scope: &MemoryScopeFilter,
    paths: &[PathBuf],
    limit: usize,
) -> Result<(), MemoryError> {
    let repo_root = resolve_code_intel_repo(config, scope.repo.as_deref())?;
    let scope_refs = scope_refs_for_context(scope, paths);
    let artifacts =
        CompositeCodeIntelProvider::new(repo_root).code_context(paths, &scope_refs, limit)?;
    append_code_intel_artifacts(config, output, artifacts);
    Ok(())
}

async fn append_code_intel_context_blocking(
    config: MemoryConfig,
    mut output: String,
    scope: MemoryScopeFilter,
    paths: Vec<PathBuf>,
    limit: usize,
) -> Result<String, MemoryError> {
    let repo_root = resolve_code_intel_repo(&config, scope.repo.as_deref())?;
    let scope_refs = scope_refs_for_context(&scope, &paths);
    let artifacts = code_intel_artifacts_blocking(repo_root, paths, scope_refs, limit).await?;
    append_code_intel_artifacts(&config, &mut output, artifacts);
    Ok(output)
}

async fn code_intel_artifacts_blocking(
    repo_root: PathBuf,
    paths: Vec<PathBuf>,
    scope_refs: Vec<CodeIntelScope>,
    limit: usize,
) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
    tokio::task::spawn_blocking(move || {
        CompositeCodeIntelProvider::new(repo_root).code_context(&paths, &scope_refs, limit)
    })
    .await
    .map_err(|error| {
        MemoryError::InvalidInput(format!("code-intelligence analysis task failed: {error}"))
    })?
    .map_err(MemoryError::from)
}

async fn code_intel_artifacts_with_symbol_kinds_blocking(
    repo_root: PathBuf,
    paths: Vec<PathBuf>,
    scope_refs: Vec<CodeIntelScope>,
    limit: usize,
    symbol_kinds: BTreeSet<String>,
) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
    tokio::task::spawn_blocking(move || {
        CompositeCodeIntelProvider::new(repo_root).code_context_with_symbol_kinds(
            &paths,
            &scope_refs,
            limit,
            &symbol_kinds,
        )
    })
    .await
    .map_err(|error| {
        MemoryError::InvalidInput(format!("code-intelligence analysis task failed: {error}"))
    })?
    .map_err(MemoryError::from)
}

fn append_code_intel_artifacts(
    config: &MemoryConfig,
    output: &mut String,
    artifacts: Vec<CodeIntelArtifact>,
) {
    output.push_str("\n## Code Intelligence\n\n");
    if artifacts.is_empty() {
        output.push_str("- No code-intelligence artifacts found.\n");
        return;
    }
    for artifact in artifacts {
        output.push_str(&format!("### {}: {}\n\n", artifact.kind, artifact.title));
        output.push_str(&format!("- Provider: {}\n", artifact.provider));
        if let Some(path) = &artifact.path {
            output.push_str(&format!("- Path: {}\n", path_for_json(config, path)));
        }
        if let Some(commit_sha) = &artifact.commit_sha {
            output.push_str(&format!("- Commit: {commit_sha}\n"));
        }
        if !artifact.source_refs.is_empty() {
            let sources = artifact
                .source_refs
                .iter()
                .map(|source| format!("{}:{}", source.kind, source.id))
                .collect::<Vec<_>>()
                .join(", ");
            output.push_str(&format!("- Sources: {sources}\n"));
        }
        output.push('\n');
        output.push_str(&artifact.summary);
        output.push_str("\n\n");
    }
}

fn resolve_code_intel_repo(
    config: &MemoryConfig,
    repo: Option<&str>,
) -> Result<PathBuf, MemoryError> {
    let Some(repo) = repo.and_then(non_empty) else {
        return config
            .default_repository_id
            .as_deref()
            .and_then(|id| config.repository_sources.get(id))
            .map(|source| source.root.clone())
            .map(Ok)
            .unwrap_or_else(|| repo_existing_path(config, "."));
    };
    if let Some(source) = config.repository_sources.get(&repo) {
        return Ok(source.root.clone());
    }
    if !config.repository_sources.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "unknown canonical repository id `{repo}`"
        )));
    }
    let resolved = repo_existing_path(config, &repo)?;
    if !resolved.is_dir() {
        return Err(MemoryError::InvalidInput(format!(
            "context repo `{repo}` did not resolve to a directory at {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn scope_refs_for_context(scope: &MemoryScopeFilter, paths: &[PathBuf]) -> Vec<CodeIntelScope> {
    let mut refs = Vec::new();
    push_scope_ref(
        &mut refs,
        CodeIntelScopeKind::ProjectSet,
        scope.project_set.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        CodeIntelScopeKind::Project,
        scope.project.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        CodeIntelScopeKind::Milestone,
        scope.milestone.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        CodeIntelScopeKind::WorkItem,
        scope.issue.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        CodeIntelScopeKind::Repository,
        scope.repo.as_deref(),
    );
    push_scope_ref(&mut refs, CodeIntelScopeKind::Area, scope.area.as_deref());
    for path in paths {
        refs.push(CodeIntelScope {
            kind: CodeIntelScopeKind::CodePath,
            id: path.display().to_string(),
            label: None,
        });
    }
    refs
}

fn push_scope_ref(refs: &mut Vec<CodeIntelScope>, kind: CodeIntelScopeKind, id: Option<&str>) {
    if let Some(id) = id.and_then(non_empty) {
        refs.push(CodeIntelScope {
            kind,
            id,
            label: None,
        });
    }
}

fn scope_filter(
    scope: &ScopeArgs,
    issue: Option<&str>,
    milestone: Option<&str>,
    area: Option<&str>,
) -> MemoryScopeFilter {
    MemoryScopeFilter {
        project_set: scope
            .project_set
            .as_deref()
            .and_then(non_empty)
            .or_else(|| env_scope_value("OPENSYMPHONY_MEMORY_PROJECT_SET")),
        project: scope
            .project
            .as_deref()
            .and_then(non_empty)
            .or_else(|| env_scope_value("OPENSYMPHONY_MEMORY_PROJECT")),
        milestone: milestone.and_then(non_empty),
        issue: issue.and_then(non_empty),
        repo: scope
            .repo
            .as_deref()
            .and_then(non_empty)
            .or_else(|| env_scope_value("OPENSYMPHONY_MEMORY_EXECUTION_REPO")),
        area: area.and_then(non_empty),
        all_accessible: scope.all_accessible,
    }
}

fn env_scope_value(name: &str) -> Option<String> {
    env::var(name).ok().and_then(|value| non_empty(&value))
}

fn scope_filter_from_mcp(arguments: &Value, include_issue: bool) -> MemoryScopeFilter {
    MemoryScopeFilter {
        project_set: optional_string_arg(arguments, "projectSet"),
        project: optional_string_arg(arguments, "project"),
        milestone: optional_string_arg(arguments, "milestone"),
        issue: include_issue
            .then(|| optional_string_arg(arguments, "issue"))
            .flatten(),
        repo: optional_string_arg(arguments, "repo"),
        area: optional_string_arg(arguments, "area"),
        all_accessible: bool_arg(arguments, "allAccessible")
            || bool_arg(arguments, "all_accessible"),
    }
}

fn path_for_json(config: &MemoryConfig, path: &Path) -> String {
    path.strip_prefix(&config.repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn required_string_arg(arguments: &Value, key: &str) -> Result<String, MemoryError> {
    optional_string_arg(arguments, key)
        .ok_or_else(|| MemoryError::InvalidInput(format!("missing string argument `{key}`")))
}

fn memory_visibility_arg(arguments: &Value) -> Result<MemoryVisibility, MemoryError> {
    match required_string_arg(arguments, "visibility")?
        .to_ascii_lowercase()
        .as_str()
    {
        "public" => Ok(MemoryVisibility::Public),
        "private" => Ok(MemoryVisibility::Private),
        value => Err(MemoryError::InvalidInput(format!(
            "invalid visibility `{value}`; expected public or private"
        ))),
    }
}

fn optional_string_arg(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty)
}

fn string_list_arg(arguments: &Value, key: &str) -> Vec<String> {
    match arguments.get(key) {
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_str)
            .filter_map(non_empty)
            .collect(),
        Some(Value::String(value)) => parse_issue_cells(value),
        _ => Vec::new(),
    }
}

fn usize_arg(arguments: &Value, key: &str, default: usize) -> usize {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn usize_arg_allow_zero(arguments: &Value, key: &str, default: usize) -> usize {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(default)
}

fn bool_arg(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

async fn run_archive(args: ArchiveArgs) -> Result<(), MemoryError> {
    let repo_root = env::current_dir().map_err(|source| MemoryError::ReadFile {
        path: PathBuf::from("."),
        source,
    })?;
    let selected_central_config = selected_central_config_path(&repo_root, args.config.as_deref())?;
    let config = load_memory_config(&repo_root, args.config.as_deref())?;
    let identifiers = collect_issue_ids(
        None,
        args.issues.as_deref(),
        args.issues_file.as_deref(),
        args.issue_range.as_deref(),
    )?;
    if args.from_memory && !identifiers.is_empty() {
        return Err(MemoryError::InvalidInput(
            "choose either --from-memory or explicit issue selectors, not both".to_string(),
        ));
    }
    if args.state.is_some() && !args.from_memory {
        return Err(MemoryError::InvalidInput(
            "--state only applies with --from-memory".to_string(),
        ));
    }
    if args.no_github && args.from_memory {
        return Err(MemoryError::InvalidInput(
            "--no-github only applies when archive performs live capture for explicit issues"
                .to_string(),
        ));
    }
    let write = !args.dry_run;
    let _coordination_lock = write
        .then(|| acquire_memory_writer_lock(&config))
        .transpose()?;

    if !args.from_memory {
        if identifiers.is_empty() {
            return Err(MemoryError::InvalidInput(
                "provide explicit issues or use --from-memory".to_string(),
            ));
        }
        return run_archive_with_live_capture(&repo_root, &config, args, identifiers, write).await;
    }

    let plan = plan_archive(
        &config,
        &identifiers,
        args.from_memory,
        args.state.as_deref(),
        write,
        args.force,
    )?;
    if !write {
        println!("{}", render_archive_plan(&config, &plan));
        println!("Dry run only. Re-run without `--dry-run` to archive eligible Linear issues.");
        return Ok(());
    }
    let report = archive_in_linear(
        &repo_root,
        args.workflow.as_deref(),
        selected_central_config.as_deref(),
        &plan,
    )
    .await?;
    if !report.archived.is_empty() {
        mark_archived(&config, &report.archived)?;
    }
    let conversation_report = archive_openhands_conversations_from_config(
        &repo_root,
        args.workflow.as_deref(),
        &report.archived,
        selected_central_config.as_deref(),
    )
    .await?;
    println!("Archived {} Linear issue(s).", report.archived.len());
    for issue_key in &report.archived {
        println!("- {issue_key}");
    }
    print_conversation_archive_report(&conversation_report);
    if !report.failures.is_empty() {
        for failure in &report.failures {
            eprintln!("- {failure}");
        }
        return Err(MemoryError::Linear(format!(
            "archived {} issue(s), failed to archive {} issue(s)",
            report.archived.len(),
            report.failures.len()
        )));
    }
    if !conversation_report.failures.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "archived {} Linear issue(s), failed to archive {} OpenHands conversation(s)",
            report.archived.len(),
            conversation_report.failures.len()
        )));
    }
    Ok(())
}

async fn run_archive_with_live_capture(
    repo_root: &Path,
    config: &MemoryConfig,
    args: ArchiveArgs,
    identifiers: Vec<String>,
    write: bool,
) -> Result<(), MemoryError> {
    let selected_central_config = selected_central_config_path(repo_root, args.config.as_deref())?;
    let selection = IssueSelection {
        identifiers: identifiers.clone(),
        ..IssueSelection::default()
    };
    let source = load_linear_source(
        repo_root,
        args.workflow.as_deref(),
        selected_central_config.as_deref(),
        &identifiers,
    )
    .await?;
    let capture_plan = plan_capture(config, &source, &selection, write, !args.no_github)?;

    if !write {
        println!("{}", render_capture_dry_run(config, &capture_plan));
        let archive_plan = archive_plan_after_capture(config, &capture_plan, false, args.force);
        println!("\n{}", render_archive_plan(config, &archive_plan));
        println!(
            "Dry run only. Re-run without `--dry-run` to capture memory and archive eligible Linear issues."
        );
        return Ok(());
    }

    let capture_report = write_capture_plan(config, &capture_plan, args.force)?;
    print_capture_write_report(capture_report);

    let archive_plan = archive_plan_after_capture(config, &capture_plan, true, args.force);
    if archive_plan.issues.iter().all(|issue| !issue.eligible) {
        println!("\n{}", render_archive_plan(config, &archive_plan));
        return Err(MemoryError::InvalidInput(
            "no archive-eligible issues after memory capture".to_string(),
        ));
    }
    if !archive_plan.warnings.is_empty() {
        println!("\n{}", render_archive_plan(config, &archive_plan));
    }

    let report = archive_in_linear(
        repo_root,
        args.workflow.as_deref(),
        selected_central_config.as_deref(),
        &archive_plan,
    )
    .await?;
    finish_archive_write(
        repo_root,
        args.workflow.as_deref(),
        selected_central_config.as_deref(),
        config,
        report,
    )
    .await
}

fn archive_plan_after_capture(
    config: &MemoryConfig,
    capture_plan: &crate::opensymphony_memory::CapturePlan,
    write: bool,
    force: bool,
) -> ArchivePlan {
    let mut issues = Vec::new();
    let mut warnings = Vec::new();
    let mut selected = capture_plan.selected.iter().collect::<Vec<_>>();
    selected.sort_by(|left, right| {
        left.issue
            .children
            .len()
            .cmp(&right.issue.children.len())
            .then_with(|| left.issue.identifier.cmp(&right.issue.identifier))
    });
    for issue in selected {
        let issue_key = issue.issue.identifier.clone();
        let capture_warnings = issue
            .warnings
            .iter()
            .chain(capture_plan.warnings.iter())
            .cloned()
            .collect::<Vec<_>>();
        let warning_count = archive_blocking_warning_count(&capture_warnings);
        let (eligible, reason) = if force {
            (
                true,
                "eligible because --force bypasses capture warning checks after live capture"
                    .to_string(),
            )
        } else if warning_count == 0 {
            (
                true,
                "eligible after live capture writes fresh memory with no unresolved warnings"
                    .to_string(),
            )
        } else {
            (
                false,
                format!(
                    "blocked: live capture would produce {warning_count} unresolved warning(s); rerun capture or use --force"
                ),
            )
        };
        if !eligible {
            warnings.push(format!("{issue_key}: {reason}"));
        }
        issues.push(crate::opensymphony_memory::ArchiveIssuePlan {
            issue_key,
            eligible,
            reason,
            capsule_path: Some(config.issue_capsule_path(&issue.issue.identifier)),
        });
    }
    ArchivePlan {
        write,
        force,
        issues,
        warnings,
    }
}

async fn finish_archive_write(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
    config: &MemoryConfig,
    report: LinearArchiveReport,
) -> Result<(), MemoryError> {
    if !report.archived.is_empty() {
        mark_archived(config, &report.archived)?;
    }
    let conversation_report = archive_openhands_conversations_from_config(
        repo_root,
        workflow_path,
        &report.archived,
        central_config_path,
    )
    .await?;
    println!("Archived {} Linear issue(s).", report.archived.len());
    for issue_key in &report.archived {
        println!("- {issue_key}");
    }
    print_conversation_archive_report(&conversation_report);
    if !report.failures.is_empty() {
        for failure in &report.failures {
            eprintln!("- {failure}");
        }
        return Err(MemoryError::Linear(format!(
            "archived {} issue(s), failed to archive {} issue(s)",
            report.archived.len(),
            report.failures.len()
        )));
    }
    if !conversation_report.failures.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "archived {} Linear issue(s), failed to archive {} OpenHands conversation(s)",
            report.archived.len(),
            conversation_report.failures.len()
        )));
    }
    Ok(())
}

#[derive(Debug, Default, Deserialize)]
struct ConversationArchiveRuntimeConfig {
    #[serde(default)]
    target_repo: Option<String>,
    #[serde(default)]
    openhands: ConversationArchiveOpenHandsConfig,
}

#[derive(Debug, Default, Deserialize)]
struct ConversationArchiveOpenHandsConfig {
    #[serde(default)]
    tool_dir: Option<String>,
}

#[derive(Debug, Default)]
struct ConversationArchiveReport {
    moved: Vec<ConversationArchiveEntry>,
    already_archived: Vec<ConversationArchiveEntry>,
    warnings: Vec<String>,
    failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConversationArchiveEntry {
    issue_key: String,
    conversation_id: String,
}

struct ConversationArchiveContext<'a> {
    conversation_store: &'a OpenHandsConversationStorePaths,
    manager: WorkspaceManager,
}

async fn archive_openhands_conversations_from_config(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    issue_keys: &[String],
    central_config_path: Option<&Path>,
) -> Result<ConversationArchiveReport, MemoryError> {
    let store = conversation_store_from_run_config(repo_root, workflow_path, central_config_path)?;
    let context = conversation_archive_context(
        repo_root,
        workflow_path,
        central_config_path,
        None,
        store.as_ref(),
    )?;
    archive_openhands_conversations_for_issues_with_context(context.as_ref(), issue_keys).await
}

async fn archive_openhands_conversations_for_issues(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    conversation_store: Option<&OpenHandsConversationStorePaths>,
    issue_keys: &[String],
    resolved_workflow: Option<&ResolvedWorkflow>,
) -> Result<ConversationArchiveReport, MemoryError> {
    let context = conversation_archive_context(
        repo_root,
        workflow_path,
        None,
        resolved_workflow,
        conversation_store,
    )?;
    archive_openhands_conversations_for_issues_with_context(context.as_ref(), issue_keys).await
}

fn conversation_archive_context<'a>(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
    resolved_workflow: Option<&ResolvedWorkflow>,
    conversation_store: Option<&'a OpenHandsConversationStorePaths>,
) -> Result<Option<ConversationArchiveContext<'a>>, MemoryError> {
    let Some(conversation_store) = conversation_store else {
        return Ok(None);
    };
    let workflow = match resolved_workflow {
        Some(workflow) => workflow.clone(),
        None => load_resolved_workflow_with_config(repo_root, workflow_path, central_config_path)?,
    };
    let manager = WorkspaceManager::new(WorkspaceManagerConfig {
        root: workflow.config.workspace.root.clone(),
        hooks: HookConfig::default(),
        cleanup: CleanupConfig {
            remove_terminal_workspaces: false,
        },
    })
    .map_err(|error| {
        MemoryError::InvalidInput(format!("failed to build workspace manager: {error}"))
    })?;
    Ok(Some(ConversationArchiveContext {
        conversation_store,
        manager,
    }))
}

async fn archive_openhands_conversations_for_issues_with_context(
    context: Option<&ConversationArchiveContext<'_>>,
    issue_keys: &[String],
) -> Result<ConversationArchiveReport, MemoryError> {
    let mut report = ConversationArchiveReport::default();
    if issue_keys.is_empty() {
        return Ok(report);
    }
    let Some(context) = context else {
        report.warnings.push(
            "skipped OpenHands conversation archive: no managed tool_dir configured".to_string(),
        );
        return Ok(report);
    };

    for issue_key in issue_keys {
        let mut candidate_ids = Vec::new();
        let mut deferred_warning = None;

        let workspace = context
            .manager
            .find_workspace_by_issue_reference(issue_key)
            .await
            .map_err(|error| {
                MemoryError::InvalidInput(format!(
                    "failed to find workspace for {issue_key}: {error}"
                ))
            })?;

        if let Some(workspace) = workspace {
            let manifest_path = workspace.conversation_manifest_path();
            let raw_manifest = context
                .manager
                .read_text_artifact(&workspace, &manifest_path)
                .await
                .map_err(|error| {
                    MemoryError::InvalidInput(format!(
                        "failed to read conversation manifest for {issue_key}: {error}"
                    ))
                })?;
            if let Some(raw_manifest) = raw_manifest {
                match serde_json::from_str::<IssueConversationManifest>(&raw_manifest) {
                    Ok(manifest) => {
                        candidate_ids.push(manifest.conversation_id.to_string());
                    }
                    Err(error) => {
                        deferred_warning = Some(format!(
                            "{issue_key}: skipped workspace conversation manifest {}; decode failed: {error}",
                            manifest_path.display()
                        ));
                    }
                }
            } else {
                deferred_warning = Some(format!(
                    "{issue_key}: workspace exists but no conversation manifest was found"
                ));
            }
        } else {
            deferred_warning = Some(format!(
                "{issue_key}: no managed workspace was found; scanning OpenHands stores by workspace metadata"
            ));
        }

        let scan_report = context
            .conversation_store
            .find_conversations_by_workspace_issue(issue_key);
        report.warnings.extend(
            scan_report
                .warnings
                .into_iter()
                .map(|warning| format!("{issue_key}: {warning}")),
        );
        candidate_ids.extend(
            scan_report
                .conversations
                .into_iter()
                .map(|conversation| conversation.conversation_id),
        );

        if candidate_ids.is_empty() {
            report.warnings.push(deferred_warning.unwrap_or_else(|| {
                format!(
                    "{issue_key}: no OpenHands conversations matched the issue workspace metadata"
                )
            }));
            continue;
        }

        let mut seen = BTreeSet::new();
        for conversation_id in candidate_ids {
            let key = conversation_archive_dedupe_key(&conversation_id);
            if !seen.insert(key) {
                continue;
            }
            archive_one_openhands_conversation(
                context.conversation_store,
                &mut report,
                issue_key,
                &conversation_id,
            );
        }
    }

    Ok(report)
}

fn archive_one_openhands_conversation(
    conversation_store: &OpenHandsConversationStorePaths,
    report: &mut ConversationArchiveReport,
    issue_key: &str,
    conversation_id: &str,
) {
    match conversation_store.move_conversation_to(conversation_id, ConversationStoreKind::Archived)
    {
        Ok(ConversationMoveOutcome::Moved { .. }) => {
            report.moved.push(ConversationArchiveEntry {
                issue_key: issue_key.to_string(),
                conversation_id: conversation_id.to_string(),
            });
        }
        Ok(ConversationMoveOutcome::AlreadyInTarget { .. }) => {
            report.already_archived.push(ConversationArchiveEntry {
                issue_key: issue_key.to_string(),
                conversation_id: conversation_id.to_string(),
            });
        }
        Ok(ConversationMoveOutcome::Missing) => {
            report.warnings.push(format!(
                "{issue_key}: OpenHands conversation {conversation_id} was not found in the active, archived, or legacy stores"
            ));
        }
        Err(error) => {
            report.failures.push(format!(
                "{issue_key}: failed to archive OpenHands conversation {conversation_id}: {error}"
            ));
        }
    }
}

fn conversation_archive_dedupe_key(conversation_id: &str) -> String {
    conversation_id
        .trim()
        .chars()
        .filter(|character| *character != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

fn print_conversation_archive_report(report: &ConversationArchiveReport) {
    if !report.moved.is_empty() {
        println!("Archived {} OpenHands conversation(s).", report.moved.len());
        for entry in &report.moved {
            println!("- {}: {}", entry.issue_key, entry.conversation_id);
        }
    }
    if !report.already_archived.is_empty() {
        println!(
            "{} OpenHands conversation(s) were already archived.",
            report.already_archived.len()
        );
        for entry in &report.already_archived {
            println!("- {}: {}", entry.issue_key, entry.conversation_id);
        }
    }
    for warning in &report.warnings {
        eprintln!("- {warning}");
    }
    for failure in &report.failures {
        eprintln!("- {failure}");
    }
}

fn conversation_store_from_run_config(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
) -> Result<Option<OpenHandsConversationStorePaths>, MemoryError> {
    let config_path = central_config_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_root.join("config.yaml"));
    if !config_path.is_file() {
        return Ok(None);
    }
    let central = if central_config_path.is_some() {
        let raw = fs::read_to_string(&config_path).map_err(|source| MemoryError::ReadFile {
            path: config_path.clone(),
            source,
        })?;
        Some(
            validate_central_config_text(&config_path, &raw).map_err(|error| {
                MemoryError::InvalidInput(format!("invalid central config: {error}"))
            })?,
        )
    } else {
        None
    };
    let config = if central.is_none() {
        let raw = fs::read_to_string(&config_path).map_err(|source| MemoryError::ReadFile {
            path: config_path.clone(),
            source,
        })?;
        Some(
            serde_yaml::from_str::<ConversationArchiveRuntimeConfig>(&raw).map_err(|source| {
                MemoryError::ParseYaml {
                    path: config_path.clone(),
                    source,
                }
            })?,
        )
    } else {
        None
    };
    let config_root = config_path.parent().unwrap_or(repo_root);
    let target_repo = if let Some(workflow_root) = workflow_path.and_then(Path::parent) {
        workflow_root.to_path_buf()
    } else if let Some(central) = central.as_ref() {
        central
            .target_repo()
            .unwrap_or_else(|| repo_root.to_path_buf())
    } else if let Some(config) = config.as_ref() {
        config
            .target_repo
            .as_deref()
            .map(|value| expand_config_path(&config_path, config_root, value))
            .transpose()?
            .unwrap_or_else(|| repo_root.to_path_buf())
    } else {
        repo_root.to_path_buf()
    };
    let tool_dir = if let Some(central) = central.as_ref() {
        central.tool_dir()
    } else if let Some(config) = config.as_ref() {
        config
            .openhands
            .tool_dir
            .as_deref()
            .map(|value| expand_config_path(&config_path, config_root, value))
            .transpose()?
    } else {
        None
    };
    let Some(tool_dir) = tool_dir else {
        return Ok(None);
    };
    OpenHandsConversationStorePaths::for_tool_dir(tool_dir, target_repo)
        .map(Some)
        .map_err(|error| MemoryError::InvalidInput(error.to_string()))
}

fn expand_config_path(
    config_path: &Path,
    config_root: &Path,
    raw: &str,
) -> Result<PathBuf, MemoryError> {
    let expanded = super::expand_env_tokens(raw).map_err(|error| {
        MemoryError::InvalidInput(format!(
            "failed to expand {}: {error}",
            config_path.display()
        ))
    })?;
    Ok(super::resolve_path(config_root, &expanded))
}

fn load_resolved_workflow_with_config(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
) -> Result<ResolvedWorkflow, MemoryError> {
    if let Some(workflow) = load_central_resolved_workflow(repo_root, central_config_path)? {
        return Ok(workflow);
    }
    let workflow_path = workflow_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_root.join("WORKFLOW.md"));
    let workflow = WorkflowDefinition::load_from_path(&workflow_path)
        .map_err(|error| MemoryError::InvalidInput(format!("failed to load workflow: {error}")))?;
    let workflow_root = workflow_path.parent().unwrap_or(repo_root);
    workflow
        .resolve_with_process_env(workflow_root)
        .map_err(|error| MemoryError::InvalidInput(format!("failed to resolve workflow: {error}")))
}

const AUTO_MEMORY_STATUS_LOG_LIMIT: usize = 100;
const AUTO_MEMORY_STATUS_LOG_MAX_BYTES: usize = 64 * 1024;

fn record_auto_memory_status(
    config: &MemoryConfig,
    issue_keys: &[String],
    warnings: &[String],
) -> Result<(), MemoryError> {
    if issue_keys.is_empty() && warnings.is_empty() {
        return Ok(());
    }
    let path = config.memory_root.join("indexes/automation.md");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| MemoryError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let mut contents = fs::read_to_string(&path)
        .unwrap_or_else(|_| "# OpenSymphony Memory Automation Log\n\n".to_string());
    contents = trim_auto_memory_status_log(
        &contents,
        AUTO_MEMORY_STATUS_LOG_LIMIT,
        AUTO_MEMORY_STATUS_LOG_MAX_BYTES,
    );
    contents.push_str(&format!("## {}\n\n", Utc::now().to_rfc3339()));
    if !issue_keys.is_empty() {
        contents.push_str(&format!("- Issues: {}\n", issue_keys.join(", ")));
    }
    if warnings.is_empty() {
        contents.push_str("- Status: completed without blocking warnings\n");
    } else {
        contents.push_str("- Warnings:\n");
        for warning in warnings {
            contents.push_str(&format!("  - {warning}\n"));
        }
    }
    contents.push('\n');
    let contents = trim_auto_memory_status_log(
        &contents,
        AUTO_MEMORY_STATUS_LOG_LIMIT,
        AUTO_MEMORY_STATUS_LOG_MAX_BYTES,
    );
    atomic_write_auto_memory_status(&path, &contents)
}

fn atomic_write_auto_memory_status(path: &Path, contents: &str) -> Result<(), MemoryError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("automation.md");
    let temp_path = parent.join(format!(
        ".{file_name}.tmp-{}-{}",
        process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::write(&temp_path, contents).map_err(|source| MemoryError::WriteFile {
        path: temp_path.clone(),
        source,
    })?;
    fs::rename(&temp_path, path).map_err(|source| {
        let _ = fs::remove_file(&temp_path);
        MemoryError::WriteFile {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn trim_auto_memory_status_log(contents: &str, max_entries: usize, max_bytes: usize) -> String {
    let mut entries = Vec::new();
    let mut current = Vec::new();
    for line in contents.lines() {
        if line.starts_with("## ") {
            if !current.is_empty() {
                entries.push(current.join("\n"));
            }
            current = vec![line.to_string()];
        } else if !current.is_empty() {
            current.push(line.to_string());
        }
    }
    if !current.is_empty() {
        entries.push(current.join("\n"));
    }

    let start = entries.len().saturating_sub(max_entries);
    let mut retained = entries.into_iter().skip(start).collect::<Vec<_>>();
    loop {
        let rendered = render_auto_memory_status_log(&retained);
        if rendered.len() <= max_bytes || retained.len() <= 1 {
            return rendered;
        }
        retained.remove(0);
    }
}

fn render_auto_memory_status_log(entries: &[String]) -> String {
    let mut output = "# OpenSymphony Memory Automation Log\n\n".to_string();
    for entry in entries {
        output.push_str(entry.trim_end());
        output.push_str("\n\n");
    }
    output
}

const LINEAR_MEMORY_STATUS_BEGIN: &str = "<!-- BEGIN OPENSYMPHONY MANAGED MEMORY STATUS -->";
const LINEAR_MEMORY_STATUS_END: &str = "<!-- END OPENSYMPHONY MANAGED MEMORY STATUS -->";

async fn update_linear_memory_status(
    client: &LinearClient,
    issue_keys: &[String],
    warnings: &[String],
) -> Result<(), MemoryError> {
    let Some(project) = client
        .project_overview()
        .await
        .map_err(|error| MemoryError::Linear(format!("Linear project lookup failed: {error}")))?
    else {
        return Ok(());
    };
    let existing = project.content.unwrap_or_default();
    let section = render_linear_memory_status_section(issue_keys, warnings);
    let updated = replace_or_append_managed_section(
        &existing,
        LINEAR_MEMORY_STATUS_BEGIN,
        LINEAR_MEMORY_STATUS_END,
        &section,
    );
    client
        .update_project_content(&project.id, &updated)
        .await
        .map_err(|error| MemoryError::Linear(format!("Linear project update failed: {error}")))
}

fn render_linear_memory_status_section(issue_keys: &[String], warnings: &[String]) -> String {
    let mut section = String::new();
    section.push_str(LINEAR_MEMORY_STATUS_BEGIN);
    section.push_str("\n\n## OpenSymphony Memory Status\n\n");
    section.push_str(&format!("- Updated: {}\n", Utc::now().to_rfc3339()));
    if !issue_keys.is_empty() {
        section.push_str(&format!("- Captured: {}\n", issue_keys.join(", ")));
    }
    section.push_str("- Attention needed:\n");
    for warning in warnings.iter().take(10) {
        section.push_str(&format!("  - {warning}\n"));
    }
    if warnings.len() > 10 {
        section.push_str(&format!("  - ...and {} more\n", warnings.len() - 10));
    }
    section.push('\n');
    section.push_str(LINEAR_MEMORY_STATUS_END);
    section
}

fn replace_or_append_managed_section(
    existing: &str,
    begin: &str,
    end: &str,
    replacement: &str,
) -> String {
    if let Some(begin_index) = existing.find(begin) {
        // A missing end marker means the managed block was truncated; replace
        // from BEGIN to the end so repeated updates cannot append duplicates.
        let end_index = existing[begin_index..]
            .find(end)
            .map(|relative_end| begin_index + relative_end + end.len())
            .unwrap_or(existing.len());
        let mut output = String::new();
        output.push_str(existing[..begin_index].trim_end());
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(replacement.trim_end());
        let tail = existing[end_index..].trim_start();
        if !tail.is_empty() {
            output.push_str("\n\n");
            output.push_str(tail);
        }
        output
    } else {
        let mut output = existing.trim_end().to_string();
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str(replacement.trim_end());
        output
    }
}

#[derive(Debug, Default)]
struct LinearArchiveReport {
    archived: Vec<String>,
    failures: Vec<String>,
}

async fn archive_in_linear(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
    plan: &ArchivePlan,
) -> Result<LinearArchiveReport, MemoryError> {
    let client =
        linear_client_from_workflow_with_config(repo_root, workflow_path, central_config_path)?;
    archive_in_linear_with_client(&client, plan).await
}

async fn archive_in_linear_with_client(
    client: &LinearClient,
    plan: &ArchivePlan,
) -> Result<LinearArchiveReport, MemoryError> {
    let mut report = LinearArchiveReport::default();
    for issue in plan.issues.iter().filter(|issue| issue.eligible) {
        match client.archive_issue(&issue.issue_key).await {
            Ok(()) => report.archived.push(issue.issue_key.clone()),
            Err(error) => report
                .failures
                .push(format!("failed to archive {}: {error}", issue.issue_key)),
        }
    }
    Ok(report)
}

fn linear_client_from_workflow(
    repo_root: &Path,
    workflow_path: Option<&Path>,
) -> Result<LinearClient, MemoryError> {
    linear_client_from_workflow_with_config(repo_root, workflow_path, None)
}

fn linear_client_from_workflow_with_config(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
) -> Result<LinearClient, MemoryError> {
    if let Some(resolved) = load_central_resolved_workflow(repo_root, central_config_path)? {
        return linear_client_from_resolved_workflow(&resolved);
    }
    let workflow_path = workflow_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_root.join("WORKFLOW.md"));
    let workflow_result = match WorkflowDefinition::load_from_path(&workflow_path) {
        Ok(workflow) => {
            let workflow_root = workflow_path.parent().unwrap_or(repo_root);
            workflow
                .resolve_with_process_env(workflow_root)
                .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    };
    match workflow_result {
        Ok(resolved) => linear_client_from_resolved_workflow(&resolved),
        Err(workflow_error) => Err(MemoryError::InvalidInput(format!(
            "failed to resolve workflow: {workflow_error}"
        ))),
    }
}

fn load_central_resolved_workflow(
    repo_root: &Path,
    explicit_path: Option<&Path>,
) -> Result<Option<ResolvedWorkflow>, MemoryError> {
    let Some(config_path) = selected_central_config_path(repo_root, explicit_path)? else {
        return Ok(None);
    };
    let raw = fs::read_to_string(&config_path).map_err(|source| MemoryError::ReadFile {
        path: config_path.clone(),
        source,
    })?;
    let central = validate_central_config_text(&config_path, &raw)
        .map_err(|error| MemoryError::InvalidInput(format!("invalid central config: {error}")))?;
    if central.mode == CentralRoutingMode::ProjectSet {
        return Err(MemoryError::InvalidInput(
            "memory commands do not support project_set central routing until strict routing is enabled"
                .to_string(),
        ));
    }
    let workflow = WorkflowDefinition {
        front_matter: central.workflow_front_matter,
        prompt_template: String::new(),
    };
    workflow
        .resolve_with_process_env(config_path.parent().unwrap_or(repo_root))
        .map(Some)
        .map_err(|error| {
            MemoryError::InvalidInput(format!("failed to resolve central config tracker: {error}"))
        })
}

fn linear_client_from_resolved_workflow(
    resolved: &ResolvedWorkflow,
) -> Result<LinearClient, MemoryError> {
    let mut linear_config = LinearConfig::new(
        resolved.config.tracker.api_key.clone(),
        resolved.config.tracker.project_slug.clone(),
    );
    linear_config.base_url = resolved.config.tracker.endpoint.clone();
    linear_config.project_id = resolved.config.tracker.project_id.clone();
    linear_config.active_states = resolved.config.tracker.active_states.clone();
    linear_config.terminal_states = resolved.config.tracker.terminal_states.clone();
    LinearClient::new(linear_config)
        .map_err(|error| MemoryError::Linear(format!("invalid Linear config: {error}")))
}

async fn load_linear_source(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
    identifiers: &[String],
) -> Result<SourceFile, MemoryError> {
    let client =
        linear_client_from_workflow_with_config(repo_root, workflow_path, central_config_path)?;
    load_linear_source_from_client(&client, identifiers).await
}

async fn load_linear_context_source(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    central_config_path: Option<&Path>,
    issue_key: &str,
) -> Result<SourceFile, MemoryError> {
    let client =
        linear_client_from_workflow_with_config(repo_root, workflow_path, central_config_path)?;
    let normalized_issue = issue_key.trim();
    if normalized_issue.is_empty() {
        return Err(MemoryError::InvalidInput(
            "--issue must not be empty".to_string(),
        ));
    }
    let current = client
        .issues_by_identifiers(&[normalized_issue])
        .await
        .map_err(|error| MemoryError::Linear(format!("Linear issue lookup failed: {error}")))?;
    let issue = current
        .iter()
        .find(|issue| issue.identifier.eq_ignore_ascii_case(normalized_issue))
        .ok_or_else(|| {
            MemoryError::Linear(format!(
                "Linear issue lookup did not return {normalized_issue}"
            ))
        })?;
    let mut identifiers = BTreeSet::from([issue.identifier.clone()]);
    if let Some(parent) = &issue.parent {
        identifiers.insert(parent.identifier.clone());
    }
    for child in &issue.sub_issues {
        identifiers.insert(child.identifier.clone());
    }
    for blocker in &issue.blocked_by {
        identifiers.insert(blocker.identifier.clone());
    }
    let identifiers = identifiers.into_iter().collect::<Vec<_>>();
    load_linear_source_from_client(&client, &identifiers).await
}

async fn load_linear_source_from_client(
    client: &LinearClient,
    identifiers: &[String],
) -> Result<SourceFile, MemoryError> {
    let tracker_issues = load_linear_issue_tree(client, identifiers).await?;

    let mut issues = Vec::new();
    for issue in tracker_issues {
        let workpad = client
            .fetch_workpad_comment(&issue.id)
            .await
            .map_err(|error| {
                MemoryError::Linear(format!(
                    "Linear workpad comment lookup failed for {}: {error}",
                    issue.identifier
                ))
            })?;
        issues.push(issue_evidence_from_tracker(issue, workpad));
    }

    Ok(SourceFile {
        issues,
        ..SourceFile::default()
    })
}

async fn load_linear_issue_tree(
    client: &LinearClient,
    identifiers: &[String],
) -> Result<Vec<TrackerIssue>, MemoryError> {
    let mut seen = BTreeSet::new();
    let mut pending = identifiers
        .iter()
        .map(|identifier| identifier.trim().to_string())
        .filter(|identifier| !identifier.is_empty())
        .collect::<BTreeSet<_>>();
    let mut issues = Vec::new();

    while !pending.is_empty() {
        let batch = pending.iter().cloned().collect::<Vec<_>>();
        pending.clear();
        let tracker_issues = client
            .issues_by_identifiers(&batch)
            .await
            .map_err(|error| MemoryError::Linear(format!("Linear issue lookup failed: {error}")))?;
        for issue in tracker_issues {
            let issue_key = issue.identifier.clone();
            if !seen.insert(issue_key) {
                continue;
            }
            for child in &issue.sub_issues {
                if !seen.contains(&child.identifier) {
                    pending.insert(child.identifier.clone());
                }
            }
            issues.push(issue);
        }
    }

    issues.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    Ok(issues)
}

fn issue_evidence_from_tracker(
    issue: TrackerIssue,
    workpad: Option<crate::opensymphony_linear::WorkpadComment>,
) -> IssueEvidence {
    let parent = issue.parent.as_ref().map(issue_link_from_tracker_ref);
    let children = issue
        .sub_issues
        .iter()
        .map(issue_link_from_tracker_ref)
        .collect::<Vec<_>>();
    let blocked_by = issue
        .blocked_by
        .iter()
        .map(issue_link_from_tracker_blocker)
        .collect::<Vec<_>>();
    let milestone = issue.project_milestone.clone();
    IssueEvidence {
        id: Some(issue.id),
        identifier: issue.identifier,
        title: issue.title,
        url: Some(issue.url),
        description: issue.description,
        state: Some(issue.state),
        milestone: milestone.as_ref().map(|milestone| milestone.name.clone()),
        milestone_id: milestone.map(|milestone| milestone.id),
        parent,
        children,
        blocked_by,
        labels: issue.labels,
        comments: workpad
            .map(|comment| {
                vec![CommentEvidence {
                    id: Some(comment.id),
                    body: comment.body,
                    updated_at: Some(comment.updated_at),
                    source: Some("linear:workpad".to_string()),
                    ..CommentEvidence::default()
                }]
            })
            .unwrap_or_default(),
        updated_at: Some(issue.updated_at),
        ..IssueEvidence::default()
    }
}

fn issue_link_from_tracker_ref(issue: &TrackerIssueRef) -> IssueLinkEvidence {
    IssueLinkEvidence {
        id: Some(issue.id.clone()),
        identifier: issue.identifier.clone(),
        title: issue.title.clone(),
        url: issue.url.clone(),
        state: Some(issue.state.clone()),
    }
}

fn issue_link_from_tracker_blocker(issue: &TrackerIssueBlocker) -> IssueLinkEvidence {
    IssueLinkEvidence {
        id: Some(issue.id.clone()),
        identifier: issue.identifier.clone(),
        title: Some(issue.title.clone()),
        url: None,
        state: Some(issue.state.name.clone()),
    }
}

fn collect_issue_ids(
    positional: Option<&str>,
    comma_separated: Option<&str>,
    issues_file: Option<&Path>,
    issue_range: Option<&str>,
) -> Result<Vec<String>, MemoryError> {
    let mut issues = Vec::new();
    if let Some(issue) = positional.and_then(non_empty) {
        issues.push(issue);
    }
    if let Some(raw) = comma_separated {
        issues.extend(parse_issue_cells(raw));
    }
    if let Some(path) = issues_file {
        let contents = fs::read_to_string(path).map_err(|source| MemoryError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        issues.extend(parse_issue_cells(&contents));
    }
    if let Some(range) = issue_range {
        issues.extend(expand_issue_range(range)?);
    }
    issues.sort();
    issues.dedup();
    Ok(issues)
}

fn parse_issue_cells(raw: &str) -> Vec<String> {
    raw.split([',', '\n', '\r', '\t', ' '])
        .filter_map(non_empty)
        .collect()
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn print_docs_plan(plan: &DocsSyncPlan) {
    println!("# Docs Sync Summary\n");
    println!("Selected issues: {}", plan.selected_issue_keys.join(", "));
    if plan.targets.is_empty() {
        println!("No stable topic docs selected for writing.");
    }
    for target in &plan.targets {
        println!(
            "\n## {} ({})\n{}",
            target.title,
            if target.create { "create" } else { "update" },
            target.diff
        );
    }
    if !plan.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in &plan.warnings {
            println!("- {warning}");
        }
    }
}

fn print_search_results(
    config: &MemoryConfig,
    results: &[crate::opensymphony_memory::SearchResult],
) {
    if results.is_empty() {
        println!("No matching memory found.");
        return;
    }
    for result in results {
        let path = result
            .capsule_path
            .strip_prefix(&config.repo_root)
            .unwrap_or(&result.capsule_path);
        println!(
            "- {}: {} [{}]\n  {}\n  {}",
            result.issue_key,
            result.title,
            result.areas.join(", "),
            path.display(),
            result.snippet
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LINEAR_MEMORY_STATUS_BEGIN, LINEAR_MEMORY_STATUS_END, MemoryMcpRequest, MemoryServerAccess,
        MemoryServerAuth, MemoryServerState, RUST_QUERY_PACK_VERSION, acquire_memory_writer_lock,
        authorize_memory_request, call_code_graph_context_tool, call_memory_ingest_code_intel_tool,
        call_memory_tool, context_source_from_mcp, load_memory_config, memory_server_health,
        memory_server_health_payload, memory_tool_descriptors, origin_is_localhost,
        parse_remote_memory_response, remote_memory_tool_token, replace_or_append_managed_section,
        required_access_for_request, resolve_code_graph_overlay, resolve_code_intel_repo, run_init,
        trim_auto_memory_status_log,
    };
    use crate::opensymphony_memory::{
        CodeGraphContextQuery, CodeIntelDiagnosticInput, CodeIntelDocumentInput,
        CodeIntelEdgeInput, CodeIntelPersistBatch, CodeIntelSymbolInput, CodeSymbolDiffStatus,
        MemoryConfig, MemoryError, code_symbol_detail, code_symbol_neighborhood,
        code_symbols_containing_span, compare_code_symbols, persist_code_intel_documents,
    };
    use crate::opensymphony_workspace::IssueManifest;
    use axum::http::{HeaderMap, HeaderValue, header};
    use chrono::Utc;
    use duckdb::{Connection, params};
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn mcp_tool_list_exposes_context_admin_and_ast_tools_when_enabled() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let names = memory_tool_descriptors(&config, &MemoryServerAuth::default())
            .into_iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(|name| name.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"memory.context".to_string()));
        assert!(names.contains(&"memory.capture".to_string()));
        assert!(names.contains(&"memory.sync_docs".to_string()));
        assert!(names.contains(&"memory.reindex".to_string()));
        assert!(names.contains(&"memory.export_okf".to_string()));
        assert!(names.contains(&"memory.import_okf".to_string()));
        let graph_tool = memory_tool_descriptors(&config, &MemoryServerAuth::default())
            .into_iter()
            .find(|tool| tool["name"] == "code.graph.context")
            .expect("indexed graph context tool");
        assert_eq!(graph_tool["access"], "read");
        assert_eq!(graph_tool["inputSchema"]["additionalProperties"], false);
        assert!(
            graph_tool["inputSchema"]["properties"]
                .get("visibility")
                .is_none()
        );
        assert!(
            graph_tool["inputSchema"]["properties"]
                .get("snippet")
                .is_none()
        );
        assert!(
            graph_tool["inputSchema"]["properties"]
                .get("repoRoot")
                .is_none()
        );
        assert!(names.contains(&"code.ast.status".to_string()));
        assert!(names.contains(&"code.ast.outline".to_string()));
        assert!(names.contains(&"code.ast.symbols".to_string()));
        assert!(names.contains(&"code.ast.references".to_string()));
        assert!(names.contains(&"code.ast.query".to_string()));
        assert!(names.contains(&"code.ast.context".to_string()));
        assert!(names.contains(&"code.ast.diagnostics".to_string()));
    }

    #[test]
    fn memory_init_rejects_a_selected_central_config() {
        let repo = TempDir::new().expect("repo should exist");
        let central = repo.path().join("central.yaml");
        let error = run_init(
            repo.path(),
            Some(&central),
            Some(&central),
            super::InitArgs {
                dry_run: true,
                force: false,
            },
        )
        .expect_err("central config must not be overwritten by memory init");
        assert!(
            error
                .to_string()
                .contains("cannot target a central instance config")
        );
    }

    #[test]
    fn central_memory_load_uses_typed_checkout_for_repository_policy() {
        let root = TempDir::new().expect("central config root should exist");
        let checkout = root.path().join("checkout");
        let central = root.path().join("central.yaml");
        std::fs::create_dir_all(checkout.join(".opensymphony/memory"))
            .expect("checkout memory config directory should exist");
        std::fs::write(
            checkout.join(".opensymphony/memory/memory.yaml"),
            "docs:\n  public_root: repository-docs\nareas:\n  ops:\n    docs_target: repository-ops.md\nredaction:\n  deny_patterns: [checkout-secret]\n",
        )
        .expect("checkout memory config should be written");
        std::fs::write(
            &central,
            format!(
                "schema_version: 1\ninstance:\n  id: typed-checkout\n  state_root: {0}/state\nrouting:\n  mode: legacy_single\n  repository: repo\ntracker_profiles:\n  linear:\n    provider: linear\n    credential: linear-key\n    active_states: [Todo]\n    terminal_states: [Done]\nlinear_projects:\n  project:\n    provider_project_id: project-id\n    repositories: [repo]\nrepositories:\n  repo:\n    aliases: [repo]\n    remote:\n      provider: git\n      locator: github.com/example/repo\n      clone: git@github.com:example/repo.git\n    target_branch: develop\n    credential: git-key\n    review_profile: review\n    instructions:\n      path: AGENTS.md\n    checkout_path: {0}/checkout\ncredentials:\n  linear-key:\n    kind: environment\n    variable: LINEAR_API_KEY\n  git-key:\n    kind: ssh-agent\nreview_profiles:\n  review:\n    provider: git\n    credential: git-key\nworkspace:\n  root: {0}/workspace\nmemory:\n  catalog_root: {0}/state/memory\n",
                root.path().display()
            ),
        )
        .expect("central config should be written");

        let config = load_memory_config(root.path(), Some(&central))
            .expect("central memory config should load");
        let canonical_root = std::fs::canonicalize(root.path()).expect("canonical root");
        assert_eq!(config.repo_root, canonical_root.join("checkout"));
        assert_eq!(config.memory_root, canonical_root.join("state/memory"));
        assert_eq!(
            config.docs.public_root,
            canonical_root.join("checkout/repository-docs")
        );
        assert_eq!(
            config
                .areas
                .get("ops")
                .expect("checkout area should load")
                .docs_target,
            canonical_root.join("checkout/repository-ops.md")
        );
        assert_eq!(config.redaction.deny_patterns, vec!["checkout-secret"]);
    }

    #[tokio::test]
    async fn project_set_memory_commands_are_rejected_before_tracker_access() {
        let repo = TempDir::new().expect("repo should exist");
        let central = repo.path().join("central.yaml");
        std::fs::write(
            &central,
            format!(
                "schema_version: 1\ninstance:\n  id: project-set\n  state_root: {0}/state\nrouting:\n  mode: project_set\n  active_project_set: suite\ntracker_profiles:\n  linear:\n    provider: linear\n    credential: linear-key\nproject_sets:\n  suite:\n    tracker_profile: linear\n    projects: [project]\nlinear_projects:\n  project:\n    provider_project_id: project-id\n    repositories: [repo]\nrepositories:\n  repo:\n    aliases: [repo]\n    remote:\n      provider: git\n      locator: github.com/example/repo\n      clone: git@github.com:example/repo.git\n    target_branch: develop\n    credential: git-key\n    review_profile: review\n    instructions:\n      path: AGENTS.md\ncredentials:\n  linear-key:\n    kind: environment\n    variable: LINEAR_API_KEY\n  git-key:\n    kind: ssh-agent\nreview_profiles:\n  review:\n    provider: git\n    credential: git-key\nworkspace:\n  root: {0}/workspace\nmemory:\n  catalog_root: {0}/state/memory\n",
                repo.path().display()
            ),
        )
        .expect("central config should be written");

        let error = super::load_central_resolved_workflow(repo.path(), Some(&central))
            .expect_err("project_set memory commands must be gated");
        assert!(error.to_string().contains("do not support project_set"));
        let error = super::reject_project_set_memory_write(Some(&central), "memory sync-docs")
            .expect_err("project_set memory writes must be rejected");
        assert!(error.to_string().contains("memory sync-docs"));

        let config = MemoryConfig::load(repo.path(), None).expect("memory config should load");
        let error = super::run_context(
            repo.path(),
            &config,
            Some(&central),
            super::ContextArgs {
                scope: super::ScopeArgs::default(),
                issue: "COE-547".to_string(),
                milestone: None,
                area: None,
                include: Vec::new(),
                paths: Vec::new(),
                include_code_intel: false,
                limit: 20,
            },
        )
        .await
        .expect_err("CLI context must reject project_set memory commands");
        assert!(error.to_string().contains("do not support project_set"));
    }

    #[test]
    fn mcp_tool_list_hides_ast_tools_when_code_intel_disabled() {
        let repo = TempDir::new().expect("temp repo");
        let config_path = repo.path().join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  enabled: false\n  ast:\n    enabled: true\n",
        )
        .expect("config");
        let config = MemoryConfig::load(repo.path(), Some(&config_path)).expect("memory config");
        let names = memory_tool_descriptors(&config, &MemoryServerAuth::default())
            .into_iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(|name| name.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"memory.context".to_string()));
        assert!(!names.contains(&"code.graph.context".to_string()));
        assert!(!names.iter().any(|name| name.starts_with("code.ast.")));

        std::fs::write(
            &config_path,
            "code_intel:\n  enabled: true\n  ast:\n    enabled: false\n",
        )
        .expect("config");
        let config = MemoryConfig::load(repo.path(), Some(&config_path)).expect("memory config");
        let names = memory_tool_descriptors(&config, &MemoryServerAuth::default())
            .into_iter()
            .filter_map(|tool| {
                tool.get("name")
                    .and_then(|name| name.as_str())
                    .map(str::to_string)
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"memory.context".to_string()));
        assert!(!names.iter().any(|name| name.starts_with("code.ast.")));
    }

    #[tokio::test]
    async fn code_graph_context_finds_symbols_callers_and_bounds_evidence() {
        let repo = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\nfn caller() -> u8 { answer() }\n",
        )
        .expect("source");
        init_test_git_repo(repo.path(), "develop");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        call_memory_ingest_code_intel_tool(
            &config,
            &json!({ "paths": ["src/lib.rs"], "persist": true, "limit": 20 }),
        )
        .await
        .expect("persist indexed source");
        let repo_id = repo
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("repo id")
            .to_string();

        let response = call_memory_tool(
            &config,
            json!({
                "name": "code.graph.context",
                "arguments": {
                    "repository": repo_id,
                    "symbol": "answer",
                    "depth": 1,
                    "limit": 10
                }
            }),
        )
        .await
        .expect("graph context");
        let evidence = response["evidence"].as_array().expect("evidence array");
        assert!(evidence.iter().any(|item| {
            item["kind"] == "symbol"
                && item["name"] == "answer"
                && item["provenance"] == "indexed_baseline"
        }));
        assert!(evidence.iter().any(|item| item["relation"] == "caller"));
        assert!(evidence.iter().all(|item| item.get("snippet").is_none()));
        assert!(evidence.iter().all(|item| {
            item["path"].is_string()
                && item["span"].is_object()
                && item["parserVersion"].is_string()
                && item["queryPackVersion"].is_string()
                && item["freshness"].is_string()
                && item["sourceRef"].is_string()
        }));

        let bounded = call_memory_tool(
            &config,
            json!({
                "name": "code.graph.context",
                "arguments": { "repository": repo_id, "query": "lib", "limit": 1 }
            }),
        )
        .await
        .expect("bounded graph context");
        assert_eq!(bounded["limit"], 1);
        assert_eq!(
            bounded["evidence"]
                .as_array()
                .expect("bounded evidence")
                .len(),
            1
        );
        assert_eq!(bounded["truncated"], true);
        assert!(bounded["dropped"].as_u64().expect("dropped count") > 0);

        let caller_key = evidence
            .iter()
            .find(|item| item["kind"] == "symbol" && item["name"] == "caller")
            .and_then(|item| item["symbolKey"].as_str())
            .expect("caller symbol key");
        assert!(
            evidence
                .iter()
                .any(|item| { item["kind"] == "edge" && item["sourceSymbolKey"] == caller_key })
        );

        let zero_depth = call_memory_tool(
            &config,
            json!({
                "name": "code.graph.context",
                "arguments": { "repository": repo_id, "symbol": "answer", "depth": 0, "limit": 10 }
            }),
        )
        .await
        .expect("zero-depth graph context");
        assert_eq!(zero_depth["depth"], 0);
        assert!(
            zero_depth["evidence"]
                .as_array()
                .expect("zero-depth evidence")
                .iter()
                .all(|item| item["kind"] != "symbol" || item["relation"] != "caller")
        );

        let outside = call_memory_tool(
            &config,
            json!({
                "name": "code.graph.context",
                "arguments": { "repository": repo_id, "path": "../outside" }
            }),
        )
        .await
        .expect_err("path traversal must be rejected");
        assert!(outside.to_string().contains("parent traversal"));
    }

    #[tokio::test]
    async fn code_graph_context_replaces_baseline_records_with_run_overlay() {
        let target = TempDir::new().expect("target repo");
        std::fs::create_dir_all(target.path().join("src")).expect("src dir");
        std::fs::write(
            target.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("baseline source");
        init_test_git_repo(target.path(), "develop");
        let config = MemoryConfig::load(target.path(), None).expect("memory config");
        call_memory_ingest_code_intel_tool(
            &config,
            &json!({ "paths": ["src/lib.rs"], "persist": true, "limit": 20 }),
        )
        .await
        .expect("persist baseline");
        let repo_id = target
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("repo id")
            .to_string();

        let workspaces = TempDir::new().expect("workspace root");
        let workspace = workspaces.path().join("COE-544");
        assert!(
            std::process::Command::new("git")
                .args(["clone", "--quiet"])
                .arg(target.path())
                .arg(&workspace)
                .status()
                .expect("git clone")
                .success()
        );
        std::fs::create_dir_all(workspace.join(".opensymphony")).expect("workspace metadata");
        let now = Utc::now();
        std::fs::write(
            workspace.join(".opensymphony/issue.json"),
            serde_json::to_vec(&IssueManifest {
                issue_id: "issue-544".to_string(),
                identifier: "COE-544".to_string(),
                title: "Indexed Agent Code Context And Retrieval".to_string(),
                current_state: "started".to_string(),
                sanitized_workspace_key: "COE-544".to_string(),
                workspace_path: workspace.clone(),
                created_at: now,
                updated_at: now,
                last_seen_tracker_refresh_at: None,
                repository_binding: None,
            })
            .expect("workspace manifest json"),
        )
        .expect("workspace manifest");
        std::fs::write(
            workspace.join("src/lib.rs"),
            "pub fn replacement() -> u8 { 43 }\n",
        )
        .expect("workspace edit");

        let response = call_code_graph_context_tool(
            config.clone(),
            json!({
                "repository": repo_id,
                "symbol": "replacement",
                "runId": "COE-544",
                "depth": 1,
                "limit": 10
            }),
            Some(workspaces.path().to_path_buf()),
        )
        .await
        .expect("overlay graph context");
        assert_eq!(response["provenance"]["kind"], "workspace_overlay");
        assert!(
            response["overlayDigest"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        let evidence = response["evidence"].as_array().expect("overlay evidence");
        assert!(evidence.iter().any(|item| {
            item["kind"] == "symbol"
                && item["name"] == "replacement"
                && item["provenance"] == "workspace_overlay"
                && item["freshness"] == "current"
        }));
        assert!(evidence.iter().all(|item| item.get("snippet").is_none()));

        std::fs::write(
            workspace.join("src/lib.rs"),
            "pub fn replacement() -> u8 { let = ; 43 }\n",
        )
        .expect("workspace diagnostic edit");
        let diagnostics = call_code_graph_context_tool(
            config.clone(),
            json!({
                "repository": repo_id,
                "symbol": "replacement",
                "runId": "COE-544",
                "depth": 0,
                "limit": 10
            }),
            Some(workspaces.path().to_path_buf()),
        )
        .await
        .expect("overlay diagnostic graph context");
        assert!(
            diagnostics["evidence"]
                .as_array()
                .expect("diagnostic evidence")
                .iter()
                .any(|item| item["kind"] == "diagnostic"
                    && item["provenance"] == "workspace_overlay"
                    && item["parserVersion"].is_string()
                    && item["queryPackVersion"].is_string()
                    && item["overlayDigest"].is_string())
        );

        std::fs::write(workspace.join("src/lib.rs"), "let = ;\n")
            .expect("workspace diagnostic-only edit");
        let diagnostic_only = call_code_graph_context_tool(
            config.clone(),
            json!({
                "repository": repo_id,
                "path": "src/lib.rs",
                "runId": "COE-544",
                "depth": 0,
                "limit": 10
            }),
            Some(workspaces.path().to_path_buf()),
        )
        .await
        .expect("diagnostic-only overlay graph context");
        assert!(
            diagnostic_only["evidence"]
                .as_array()
                .expect("diagnostic-only evidence")
                .iter()
                .any(|item| {
                    item["kind"] == "diagnostic"
                        && item["path"] == "src/lib.rs"
                        && item["symbolKey"].is_null()
                        && item["provenance"] == "workspace_overlay"
                        && item["parserVersion"].is_string()
                        && item["queryPackVersion"].is_string()
                })
        );

        std::fs::write(
            workspace.join("src/lib.rs"),
            "pub fn stale_baseline_should_not_survive() {}\n".repeat(64),
        )
        .expect("oversized workspace edit");
        let mut unanalyzed_config = config;
        unanalyzed_config.code_intel.ast.max_file_bytes = 128;
        let unanalyzed = call_code_graph_context_tool(
            unanalyzed_config,
            json!({
                "repository": repo_id,
                "symbol": "answer",
                "runId": "COE-544",
                "depth": 0,
                "limit": 10
            }),
            Some(workspaces.path().to_path_buf()),
        )
        .await
        .expect("unanalyzed overlay graph context");
        assert!(
            unanalyzed["evidence"]
                .as_array()
                .expect("unanalyzed evidence")
                .iter()
                .all(|item| !(item["kind"] == "symbol" && item["name"] == "answer"))
        );
        assert!(
            unanalyzed["unanalyzedFiles"]
                .as_array()
                .expect("unanalyzed files")
                .iter()
                .any(|path| path == "src/lib.rs")
        );
    }

    #[cfg(unix)]
    #[test]
    fn code_graph_context_rejects_foreign_and_symlinked_workspaces() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let context_query = CodeGraphContextQuery {
            repo_id: "repo".to_string(),
            query: Some("answer".to_string()),
            path: None,
            symbol: None,
            depth: 0,
            limit: 1,
        };
        let workspace_root = TempDir::new().expect("workspace root");
        let workspace = workspace_root.path().join("COE-544");
        std::fs::create_dir_all(workspace.join(".opensymphony")).expect("metadata");
        let now = Utc::now();
        std::fs::write(
            workspace.join(".opensymphony/issue.json"),
            serde_json::to_vec(&IssueManifest {
                issue_id: "issue-544".to_string(),
                identifier: "COE-545".to_string(),
                title: "foreign".to_string(),
                current_state: "started".to_string(),
                sanitized_workspace_key: "COE-544".to_string(),
                workspace_path: workspace.clone(),
                created_at: now,
                updated_at: now,
                last_seen_tracker_refresh_at: None,
                repository_binding: None,
            })
            .expect("manifest json"),
        )
        .expect("manifest");
        let foreign = resolve_code_graph_overlay(
            &config,
            Some(workspace_root.path()),
            "repo",
            "COE-544",
            &context_query,
        )
        .expect_err("foreign workspace must be rejected");
        assert!(foreign.to_string().contains("ownership"));

        std::fs::remove_dir_all(&workspace).expect("remove foreign workspace");
        let symlink_target = TempDir::new().expect("symlink target");
        std::fs::create_dir_all(symlink_target.path()).expect("symlink target directory");
        std::os::unix::fs::symlink(symlink_target.path(), &workspace).expect("workspace symlink");
        let symlinked = resolve_code_graph_overlay(
            &config,
            Some(workspace_root.path()),
            "repo",
            "COE-544",
            &context_query,
        )
        .expect_err("symlinked workspace must be rejected");
        assert!(symlinked.to_string().contains("symlink"));
    }

    #[tokio::test]
    async fn live_memory_context_revalidates_source_after_indexed_discovery() {
        let repo = TempDir::new().expect("repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("baseline source");
        init_test_git_repo(repo.path(), "develop");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        call_memory_ingest_code_intel_tool(
            &config,
            &json!({ "paths": ["src/lib.rs"], "persist": true, "limit": 20 }),
        )
        .await
        .expect("persist baseline");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn replacement() -> u8 { 43 }\n",
        )
        .expect("edited source");

        let context = call_memory_tool(
            &config,
            json!({
                "name": "memory.context",
                "arguments": {
                    "issue": "COE-544",
                    "paths": ["src/lib.rs"],
                    "includeCodeIntel": true
                }
            }),
        )
        .await
        .expect("live context");
        let text = context["content"][0]["text"]
            .as_str()
            .expect("context text");
        assert!(text.contains("function `replacement`"));
        assert!(!text.contains("function `answer`"));
    }

    #[tokio::test]
    async fn memory_server_http_exposes_graph_discovery_and_live_ast_context() {
        let repo = TempDir::new().expect("repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("source");
        init_test_git_repo(repo.path(), "develop");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        call_memory_ingest_code_intel_tool(
            &config,
            &json!({ "paths": ["src/lib.rs"], "persist": true, "limit": 20 }),
        )
        .await
        .expect("persist baseline");
        let repo_id = repo
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("repo id")
            .to_string();
        let activity_marker = super::memory_activity_marker_path(&config.memory_root);
        let handle = super::start_memory_server_with_workspace_root(
            config,
            "127.0.0.1:0".parse().expect("address"),
            None,
            None,
        )
        .await
        .expect("start memory server");
        assert!(activity_marker.is_file());
        let client = reqwest::Client::new();
        let list = client
            .post(handle.endpoint())
            .json(&json!({ "id": 1, "method": "tools/list", "params": {} }))
            .send()
            .await
            .expect("tools list request")
            .json::<serde_json::Value>()
            .await
            .expect("tools list response");
        let tools = list["result"]["tools"].as_array().expect("tools");
        assert!(
            tools
                .iter()
                .any(|tool| tool["name"] == "code.graph.context")
        );
        assert!(tools.iter().any(|tool| tool["name"] == "code.ast.context"));

        let graph = client
            .post(handle.endpoint())
            .json(&json!({
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "code.graph.context",
                    "arguments": { "repository": repo_id, "symbol": "answer", "limit": 5 }
                }
            }))
            .send()
            .await
            .expect("graph request")
            .json::<serde_json::Value>()
            .await
            .expect("graph response");
        assert!(graph["result"]["evidence"].as_array().is_some());

        let ast = client
            .post(handle.endpoint())
            .json(&json!({
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "code.ast.context",
                    "arguments": { "paths": ["src/lib.rs"], "limit": 5 }
                }
            }))
            .send()
            .await
            .expect("AST request")
            .json::<serde_json::Value>()
            .await
            .expect("AST response");
        assert!(ast["result"]["markdown"].as_str().is_some());
        handle.abort();
        handle
            .wait()
            .await
            .expect("memory server should shut down gracefully");
        assert!(!activity_marker.exists());
    }

    #[tokio::test]
    async fn memory_server_refuses_to_start_during_migration() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let lock_path = super::memory_migration_lock_path(&config.repo_root);
        std::fs::create_dir_all(lock_path.parent().expect("lock parent"))
            .expect("lock parent should exist");
        std::fs::write(&lock_path, "active\n").expect("migration lock should exist");

        let result = super::start_memory_server_with_workspace_root(
            config,
            "127.0.0.1:0".parse().expect("address"),
            None,
            None,
        )
        .await;
        let error = match result {
            Err(error) => error,
            Ok(handle) => {
                handle.abort();
                panic!("memory server must honor migration lock");
            }
        };
        assert!(
            matches!(error, MemoryError::InvalidInput(message) if message.contains("migration"))
        );
    }

    #[tokio::test]
    async fn memory_server_reclaims_stale_activity_marker() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let marker_path = super::memory_activity_marker_path(&config.memory_root);
        std::fs::create_dir_all(&config.memory_root).expect("memory root");
        std::fs::write(&marker_path, "pid=2000000000\n").expect("stale marker");

        let handle = super::start_memory_server_with_workspace_root(
            config,
            "127.0.0.1:0".parse().expect("address"),
            None,
            None,
        )
        .await
        .expect("stale marker should be reclaimed");
        assert!(marker_path.is_file());
        handle.abort();
        handle
            .wait()
            .await
            .expect("memory server should shut down gracefully");
        assert!(!marker_path.exists());
    }

    #[test]
    fn memory_markers_reject_reused_pid_incarnation() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        std::fs::create_dir_all(&config.memory_root).expect("memory root");
        let marker_path = super::memory_activity_marker_path(&config.memory_root);
        std::fs::write(
            &marker_path,
            format!(
                "pid={}\nstart=not-the-current-process\n",
                std::process::id()
            ),
        )
        .expect("activity marker");
        assert_eq!(
            super::memory_activity_status(&config.memory_root).expect("activity status"),
            super::MemoryActivityStatus::Stale
        );

        let lock_path = super::memory_migration_lock_path(repo.path());
        std::fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("lock parent");
        std::fs::write(
            &lock_path,
            format!(
                "pid={}\nstart=not-the-current-process\n",
                std::process::id()
            ),
        )
        .expect("coordination marker");
        assert!(super::memory_lock_owner_is_stale(&lock_path));
    }

    #[test]
    fn stale_memory_lock_quarantine_names_are_unique() {
        let repo = TempDir::new().expect("repo");
        let path = super::memory_migration_lock_path(repo.path());
        let first = super::stale_memory_lock_path(&path);
        let second = super::stale_memory_lock_path(&path);

        assert_ne!(first, second);
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(&std::process::id().to_string()))
        );
    }

    #[test]
    fn failed_memory_lock_owner_initialization_removes_partial_lock() {
        let repo = TempDir::new().expect("repo");
        let path = super::memory_migration_lock_path(repo.path());
        std::fs::create_dir_all(path.parent().expect("lock parent")).expect("lock parent");
        std::fs::File::create(&path).expect("lock file");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .expect("read-only lock file");

        assert!(super::initialize_memory_coordination_lock(file, &path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn central_memory_reload_uses_defaults_when_local_config_is_absent() {
        let repo = TempDir::new().expect("repo");
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.memory_root = repo.path().join("central-catalog");
        config.index_path = config.memory_root.join("memory.duckdb");
        assert!(!config.config_path.is_file());

        let evolved = super::reload_memory_config(&config).expect("reload should use defaults");

        assert_eq!(evolved.memory_root, config.memory_root);
        assert_eq!(evolved.index_path, config.index_path);
    }

    #[test]
    fn central_memory_writers_share_a_catalog_coordination_lock() {
        let first_repo = TempDir::new().expect("first repo");
        let second_repo = TempDir::new().expect("second repo");
        let catalog = first_repo.path().join("central-catalog");
        let mut first = MemoryConfig::load(first_repo.path(), None).expect("first config");
        let mut second = MemoryConfig::load(second_repo.path(), None).expect("second config");
        first.memory_root = catalog.clone();
        second.memory_root = catalog.clone();

        let lock = acquire_memory_writer_lock(&first).expect("first catalog lock");
        assert!(matches!(
            acquire_memory_writer_lock(&second),
            Err(MemoryError::WriteFile { .. })
        ));
        drop(lock);
        acquire_memory_writer_lock(&second)
            .expect("catalog lock should be released after the first writer exits");
    }

    #[tokio::test]
    async fn memory_server_writer_gate_keeps_filesystem_lock_until_guards_drain() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let coordination_lock = super::acquire_memory_coordination_lock(&config.repo_root)
            .expect("coordination lock should be acquired");
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(Some(coordination_lock)));
        let writer_guard = gate.clone().lock_owned().await;
        let releasing_gate = gate.clone();
        let release_task = tokio::spawn(async move {
            let mut lock = releasing_gate.lock().await;
            lock.take();
        });

        assert!(
            super::acquire_memory_coordination_lock(&config.repo_root).is_err(),
            "the filesystem lock must remain held while a writer guard is active"
        );
        drop(writer_guard);
        release_task
            .await
            .expect("the server shutdown lock release should complete");
        super::acquire_memory_coordination_lock(&config.repo_root)
            .expect("the filesystem lock should release after guards drain");
    }

    #[tokio::test]
    async fn memory_writer_gate_falls_back_after_server_consumes_lock() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(None));

        let guard = super::acquire_memory_writer_guard(&config, Some(gate))
            .await
            .expect("writer should fall back to the filesystem lock");
        assert!(matches!(guard, super::MemoryWriterGuard::File(_)));
    }

    #[test]
    fn mcp_tool_list_marks_ast_query_admin_when_admin_token_is_configured() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let tools = memory_tool_descriptors(
            &config,
            &MemoryServerAuth {
                read_token: Some("read-token".to_string()),
                admin_token: Some("admin-token".to_string()),
            },
        );
        let query_tool = tools
            .iter()
            .find(|tool| tool["name"] == "code.ast.query")
            .expect("query tool");
        let outline_tool = tools
            .iter()
            .find(|tool| tool["name"] == "code.ast.outline")
            .expect("outline tool");

        assert_eq!(query_tool["access"], "admin");
        assert_eq!(outline_tool["access"], "read");
    }

    #[test]
    fn workflow_guidance_discovers_from_index_before_live_revalidation() {
        let workflow = include_str!("../../../WORKFLOW.md");
        let indexed = workflow
            .find("code.graph.context")
            .expect("indexed discovery guidance");
        let live = workflow
            .find("--include-code-intel")
            .expect("live revalidation guidance");
        assert!(
            indexed < live,
            "indexed discovery must precede live AST guidance"
        );
        assert!(workflow.contains("before edits and after every touched-file change"));
    }

    #[test]
    fn mcp_admin_tools_require_admin_access() {
        let read_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({ "name": "memory.context" }),
        };
        let admin_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({ "name": "memory.capture" }),
        };
        let okf_export_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({ "name": "memory.export_okf" }),
        };
        let persistent_code_ingest_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({
                "name": "memory.ingest_code_intel",
                "arguments": { "persist": true }
            }),
        };

        assert_eq!(
            required_access_for_request(&read_request, &MemoryServerAuth::default()),
            MemoryServerAccess::Read
        );
        assert_eq!(
            required_access_for_request(&admin_request, &MemoryServerAuth::default()),
            MemoryServerAccess::Admin
        );
        assert_eq!(
            required_access_for_request(&okf_export_request, &MemoryServerAuth::default()),
            MemoryServerAccess::Admin
        );
        assert_eq!(
            required_access_for_request(
                &persistent_code_ingest_request,
                &MemoryServerAuth::default()
            ),
            MemoryServerAccess::Admin
        );

        let ast_outline_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({ "name": "code.ast.outline" }),
        };
        let ast_query_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({ "name": "code.ast.query" }),
        };
        let graph_request = MemoryMcpRequest {
            id: json!("test"),
            method: "tools/call".to_string(),
            params: json!({ "name": "code.graph.context" }),
        };
        assert_eq!(
            required_access_for_request(&ast_outline_request, &MemoryServerAuth::default()),
            MemoryServerAccess::Read
        );
        assert_eq!(
            required_access_for_request(&ast_query_request, &MemoryServerAuth::default()),
            MemoryServerAccess::Read
        );
        assert_eq!(
            required_access_for_request(&graph_request, &MemoryServerAuth::default()),
            MemoryServerAccess::Read
        );
        assert_eq!(
            required_access_for_request(
                &graph_request,
                &MemoryServerAuth {
                    read_token: Some("read-token".to_string()),
                    admin_token: Some("admin-token".to_string()),
                },
            ),
            MemoryServerAccess::Read
        );
        assert_eq!(
            required_access_for_request(
                &ast_query_request,
                &MemoryServerAuth {
                    read_token: Some("read-token".to_string()),
                    admin_token: Some("admin-token".to_string()),
                },
            ),
            MemoryServerAccess::Admin
        );
    }

    #[tokio::test]
    async fn memory_ingest_code_intel_persists_structured_rows() {
        let repo = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "use std::fmt::Debug;\npub fn answer() -> u8 { helper() }\nfn helper() -> u8 { 42 }\n",
        )
        .expect("valid source");
        std::fs::write(repo.path().join("src/bad.rs"), "pub fn broken( {\n").expect("bad source");
        std::fs::write(repo.path().join("notes.txt"), "not code\n").expect("notes source");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        let result = call_memory_ingest_code_intel_tool(
            &config,
            &json!({
                "paths": ["src/lib.rs", "src/bad.rs", "notes.txt"],
                "persist": true,
                "limit": 20
            }),
        )
        .await
        .expect("ingest succeeds");

        assert_eq!(result["persisted"], true);
        assert_eq!(result["parsedFiles"], 2);
        assert!(result["persistedRows"].as_u64().expect("rows") > 2);
        assert!(
            result["skippedFiles"][0]
                .as_str()
                .expect("skipped file")
                .contains("unsupported language")
        );
        assert!(
            result["diagnostics"][0]
                .as_str()
                .expect("diagnostic")
                .contains("src/bad.rs")
        );
        let symbol_source = result["artifacts"]
            .as_array()
            .expect("artifacts")
            .iter()
            .flat_map(|artifact| artifact["sourceRefs"].as_array().into_iter().flatten())
            .find(|source| source["kind"] == "code-symbol")
            .expect("persisted symbol source ref");
        assert!(
            symbol_source["repoId"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
        );
        assert!(
            symbol_source["symbolKey"]
                .as_str()
                .is_some_and(|key| !key.is_empty())
        );
        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_documents", "current"), 2);
        assert!(count_rows(&connection, "code_symbols", "current") > 0);
        assert!(count_rows(&connection, "code_edges", "current") > 0);
        assert!(count_rows(&connection, "code_diagnostics", "current") > 0);
    }

    #[tokio::test]
    async fn memory_ingest_code_intel_skips_requested_directories() {
        let repo = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("valid source");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        let result = call_memory_ingest_code_intel_tool(
            &config,
            &json!({
                "paths": ["src", "src/lib.rs"],
                "persist": true,
                "limit": 20
            }),
        )
        .await
        .expect("directory path should be skipped without aborting ingest");

        assert_eq!(result["persisted"], true);
        assert_eq!(result["parsedFiles"], 1);
        assert!(
            result["skippedFiles"]
                .as_array()
                .expect("skipped files")
                .iter()
                .any(|file| file.as_str() == Some("src: directory"))
        );
        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_documents", "current"), 1);
        assert_eq!(
            connection
                .query_row::<i64, _, _>("SELECT count(*) FROM code_skipped_files", [], |row| row
                    .get(0))
                .expect("skipped file count"),
            0
        );
    }

    #[tokio::test]
    async fn memory_ingest_code_intel_defaults_to_artifacts_without_persistence() {
        let repo = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("source");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        let result = call_memory_ingest_code_intel_tool(
            &config,
            &json!({
                "paths": ["src/lib.rs"],
                "limit": 20
            }),
        )
        .await
        .expect("ingest succeeds");

        assert_eq!(result["persisted"], false);
        assert!(result["artifactCount"].as_u64().expect("artifacts") > 0);
        assert!(
            !repo
                .path()
                .join(".opensymphony/memory/memory.duckdb")
                .exists(),
            "non-persistent ingest should not create the DuckDB index"
        );
    }

    #[tokio::test]
    async fn memory_ingest_code_intel_limit_does_not_cap_persisted_symbols() {
        let repo = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        std::fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn one() -> u8 { two() }\nfn two() -> u8 { 2 }\n",
        )
        .expect("source");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        let result = call_memory_ingest_code_intel_tool(
            &config,
            &json!({
                "paths": ["src/lib.rs"],
                "persist": true,
                "limit": 1
            }),
        )
        .await
        .expect("ingest succeeds");

        assert_eq!(result["persisted"], true);
        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_symbols", "current"), 2);
        let resolved_edges: i64 = connection
            .query_row(
                "SELECT count(*) FROM code_edges WHERE freshness = 'current' AND source_symbol_key IS NOT NULL AND target_symbol_key IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .expect("resolved edge count");
        assert_eq!(resolved_edges, 1);
    }

    #[tokio::test]
    async fn memory_ingest_code_intel_stales_content_and_query_pack_changes() {
        let repo = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(repo.path().join("src")).expect("src dir");
        let source_path = repo.path().join("src/lib.rs");
        std::fs::write(&source_path, "pub fn answer() -> u8 { 42 }\n").expect("source");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        call_memory_ingest_code_intel_tool(
            &config,
            &json!({
                "paths": ["src/lib.rs"],
                "persist": true
            }),
        )
        .await
        .expect("initial ingest");
        std::fs::write(&source_path, "pub fn answer() -> u8 { 43 }\n").expect("edited source");
        let edited = call_memory_ingest_code_intel_tool(
            &config,
            &json!({
                "paths": ["src/lib.rs"],
                "persist": true
            }),
        )
        .await
        .expect("edited ingest");
        assert!(edited["staleRows"].as_u64().expect("stale rows") > 0);

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        let (content_sha256, parser_version): (String, String) = connection
            .query_row(
                "SELECT content_sha256, parser_version FROM code_documents WHERE freshness = 'current' AND path = 'src/lib.rs' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("current document");
        drop(connection);

        let report = persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: repo
                    .path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("repo")
                    .to_string(),
                commit_sha: None,
                worktree_dirty: false,
                documents: vec![CodeIntelDocumentInput {
                    path: "src/lib.rs".into(),
                    language: "rust".to_string(),
                    content_sha256,
                    parser_id: "tree-sitter".to_string(),
                    parser_version,
                    query_pack_version: "rust-query-pack-v999".to_string(),
                    byte_len: 28,
                    line_count: 1,
                    symbols: Vec::new(),
                    edges: Vec::new(),
                    diagnostics: Vec::new(),
                }],
            },
        )
        .expect("manual query-pack persist");
        assert!(
            report.stale_rows > 0,
            "query-pack version drift should mark prior rows stale"
        );

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert!(count_rows(&connection, "code_documents", "stale") >= 2);
        assert_eq!(count_rows(&connection, "code_documents", "current"), 1);
    }

    #[test]
    fn code_intel_dirty_worktree_freshness_is_consistent_for_child_rows() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let clean_batch = CodeIntelPersistBatch {
            repo_id: "repo".to_string(),
            commit_sha: Some("old".to_string()),
            worktree_dirty: false,
            documents: vec![sample_code_intel_document("hash-a", "pack-a")],
        };
        persist_code_intel_documents(&config, clean_batch).expect("clean persist");

        let dirty_batch = CodeIntelPersistBatch {
            repo_id: "repo".to_string(),
            commit_sha: Some("new".to_string()),
            worktree_dirty: true,
            documents: vec![sample_code_intel_document("hash-a", "pack-a")],
        };
        let report =
            persist_code_intel_documents(&config, dirty_batch).expect("dirty same-content persist");
        assert_eq!(
            report.stale_rows, 0,
            "dirty reingest carve-out should apply to parent and child rows"
        );

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_documents", "current"), 1);
        assert_eq!(count_rows(&connection, "code_symbols", "current"), 2);
        assert_eq!(count_rows(&connection, "code_edges", "current"), 1);
        assert_eq!(count_rows(&connection, "code_diagnostics", "current"), 1);
        assert_eq!(count_rows(&connection, "code_symbols", "stale"), 0);
        assert_eq!(count_rows(&connection, "code_edges", "stale"), 0);
        assert_eq!(count_rows(&connection, "code_diagnostics", "stale"), 0);
    }

    #[test]
    fn code_intel_clean_commit_only_reingest_does_not_report_stale_rows() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let first_batch = CodeIntelPersistBatch {
            repo_id: "repo".to_string(),
            commit_sha: Some("old".to_string()),
            worktree_dirty: false,
            documents: vec![sample_code_intel_document("hash-a", "pack-a")],
        };
        persist_code_intel_documents(&config, first_batch).expect("first persist");

        let second_batch = CodeIntelPersistBatch {
            repo_id: "repo".to_string(),
            commit_sha: Some("new".to_string()),
            worktree_dirty: false,
            documents: vec![sample_code_intel_document("hash-a", "pack-a")],
        };
        let report = persist_code_intel_documents(&config, second_batch)
            .expect("same artifact, new commit persist");
        assert_eq!(
            report.stale_rows, 0,
            "commit-only reingest should replace provenance without reporting phantom stale rows"
        );

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_documents", "current"), 1);
        assert_eq!(count_rows(&connection, "code_symbols", "current"), 2);
        assert_eq!(count_rows(&connection, "code_edges", "current"), 1);
        assert_eq!(count_rows(&connection, "code_diagnostics", "current"), 1);
        assert_eq!(count_rows(&connection, "code_documents", "stale"), 0);
        assert_eq!(count_rows(&connection, "code_symbols", "stale"), 0);
        assert_eq!(count_rows(&connection, "code_edges", "stale"), 0);
        assert_eq!(count_rows(&connection, "code_diagnostics", "stale"), 0);
    }

    #[test]
    fn code_intel_parser_version_drift_keeps_stale_child_rows() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let first_batch = CodeIntelPersistBatch {
            repo_id: "repo".to_string(),
            commit_sha: Some("same".to_string()),
            worktree_dirty: false,
            documents: vec![sample_code_intel_document("hash-a", "pack-a")],
        };
        persist_code_intel_documents(&config, first_batch).expect("first persist");

        let mut changed_parser = sample_code_intel_document("hash-a", "pack-a");
        changed_parser.parser_version = "tree-sitter-rust:0.27.0".to_string();
        let second_batch = CodeIntelPersistBatch {
            repo_id: "repo".to_string(),
            commit_sha: Some("same".to_string()),
            worktree_dirty: false,
            documents: vec![changed_parser],
        };
        let report =
            persist_code_intel_documents(&config, second_batch).expect("parser drift persist");
        assert!(
            report.stale_rows > 0,
            "parser-version drift should report stale rows"
        );

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_documents", "current"), 1);
        assert_eq!(count_rows(&connection, "code_symbols", "current"), 1);
        assert_eq!(count_rows(&connection, "code_edges", "current"), 1);
        assert_eq!(count_rows(&connection, "code_diagnostics", "current"), 1);
        assert_eq!(count_rows(&connection, "code_documents", "stale"), 1);
        assert_eq!(count_rows(&connection, "code_symbols", "stale"), 1);
        assert_eq!(count_rows(&connection, "code_edges", "stale"), 1);
        assert_eq!(count_rows(&connection, "code_diagnostics", "stale"), 1);
    }

    #[test]
    fn code_intel_diagnostic_severity_tracks_diagnostic_kind() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut document = sample_code_intel_document("hash-a", "pack-a");
        document.diagnostics = vec![
            CodeIntelDiagnosticInput {
                kind: "error".to_string(),
                severity: "error".to_string(),
                message: "ERROR parse diagnostic".to_string(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 5,
                start_byte: 0,
                end_byte: 5,
            },
            CodeIntelDiagnosticInput {
                kind: "missing".to_string(),
                severity: "warning".to_string(),
                message: "MISSING parse diagnostic".to_string(),
                start_line: 2,
                start_col: 0,
                end_line: 2,
                end_col: 5,
                start_byte: 6,
                end_byte: 11,
            },
        ];

        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("same".to_string()),
                worktree_dirty: false,
                documents: vec![document],
            },
        )
        .expect("persist diagnostics");

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        let severities = connection
            .prepare("SELECT kind, severity FROM code_diagnostics ORDER BY kind")
            .expect("prepare diagnostics")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query diagnostics")
            .collect::<Result<Vec<_>, _>>()
            .expect("diagnostics rows");
        assert_eq!(
            severities,
            vec![
                ("error".to_string(), "error".to_string()),
                ("missing".to_string(), "warning".to_string())
            ]
        );
    }

    #[test]
    fn code_intel_same_line_edges_and_diagnostics_keep_distinct_rows() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut document = sample_code_intel_document("hash-a", "pack-a");
        document.edges = vec![
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("answer".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 6,
                start_byte: 0,
                end_byte: 6,
            },
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("answer".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 1,
                start_col: 8,
                end_line: 1,
                end_col: 14,
                start_byte: 8,
                end_byte: 14,
            },
        ];
        document.diagnostics = vec![
            CodeIntelDiagnosticInput {
                kind: "missing".to_string(),
                severity: "warning".to_string(),
                message: "MISSING parse diagnostic".to_string(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 1,
                start_byte: 0,
                end_byte: 1,
            },
            CodeIntelDiagnosticInput {
                kind: "missing".to_string(),
                severity: "warning".to_string(),
                message: "MISSING parse diagnostic".to_string(),
                start_line: 1,
                start_col: 2,
                end_line: 1,
                end_col: 3,
                start_byte: 2,
                end_byte: 3,
            },
        ];

        let report = persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("same".to_string()),
                worktree_dirty: false,
                documents: vec![document],
            },
        )
        .expect("persist same-line records");
        assert_eq!(report.persisted_edges, 2);
        assert_eq!(report.persisted_diagnostics, 2);

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        assert_eq!(count_rows(&connection, "code_edges", "current"), 2);
        assert_eq!(count_rows(&connection, "code_diagnostics", "current"), 2);
    }

    #[test]
    fn code_intel_symbol_key_is_stable_beside_revision_bound_symbol_id() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let base = sample_code_intel_document("hash-a", "pack-a");
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("base".to_string()),
                worktree_dirty: false,
                documents: vec![base],
            },
        )
        .expect("base persist");

        let mut shifted = sample_code_intel_document("hash-b", "pack-a");
        shifted.symbols[0].start_line = 5;
        shifted.symbols[0].end_line = 5;
        shifted.symbols[0].start_byte = 40;
        shifted.symbols[0].end_byte = 52;
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("head".to_string()),
                worktree_dirty: false,
                documents: vec![shifted],
            },
        )
        .expect("head persist");

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        let rows = connection
            .prepare("SELECT symbol_id, symbol_key FROM code_symbols ORDER BY commit_sha")
            .expect("prepare symbols")
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .expect("query symbols")
            .collect::<Result<Vec<_>, _>>()
            .expect("symbol rows");
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].0, rows[1].0);
        assert_eq!(rows[0].1, rows[1].1);
    }

    #[test]
    fn code_intel_duplicate_symbol_keys_get_deterministic_ordinals() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut document = sample_code_intel_document("hash-a", "pack-a");
        document.symbols.push(CodeIntelSymbolInput {
            kind: "function".to_string(),
            name: "answer".to_string(),
            container_chain: Vec::new(),
            signature: None,
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 12,
            start_byte: 20,
            end_byte: 32,
            selection_start_line: 2,
            selection_end_line: 2,
            snippet_sha256: "snippet-2".to_string(),
        });
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("same".to_string()),
                worktree_dirty: false,
                documents: vec![document],
            },
        )
        .expect("persist duplicate symbols");

        let connection = Connection::open(repo.path().join(".opensymphony/memory/memory.duckdb"))
            .expect("index opens");
        let keys = connection
            .prepare("SELECT symbol_key FROM code_symbols ORDER BY start_byte")
            .expect("prepare keys")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query keys")
            .collect::<Result<Vec<_>, _>>()
            .expect("keys");
        assert_eq!(keys.len(), 2);
        assert!(!keys[0].ends_with("#2"));
        assert_eq!(keys[1], format!("{}#2", keys[0]));
    }

    #[test]
    fn code_intel_container_chain_survives_partial_symbol_indexes() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut document = sample_code_intel_document("hash-a", "pack-a");
        document.symbols = vec![CodeIntelSymbolInput {
            kind: "method".to_string(),
            name: "run".to_string(),
            container_chain: vec!["Widget".to_string()],
            signature: None,
            start_line: 3,
            start_col: 5,
            end_line: 5,
            end_col: 6,
            start_byte: 20,
            end_byte: 80,
            selection_start_line: 3,
            selection_end_line: 5,
            snippet_sha256: "run-v1".to_string(),
        }];
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("base".to_string()),
                worktree_dirty: false,
                documents: vec![document],
            },
        )
        .expect("persist partial symbols");

        let containing = code_symbols_containing_span(&config, "repo", "src/lib.rs", 4, 10, 10)
            .expect("span containment");
        assert_eq!(containing.len(), 1);
        assert_eq!(containing[0].container_symbol_id, None);
        assert_eq!(containing[0].container_chain, vec!["Widget"]);

        let detail = code_symbol_detail(&config, &containing[0].symbol_key)
            .expect("detail query")
            .expect("method detail");
        assert_eq!(detail.container_chain, vec!["Widget"]);
    }

    #[test]
    fn code_intel_read_helpers_do_not_create_index() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        assert!(
            code_symbol_detail(&config, "missing")
                .expect("detail read")
                .is_none()
        );
        assert!(
            code_symbols_containing_span(&config, "repo", "src/lib.rs", 1, 1, 10)
                .expect("span read")
                .is_empty()
        );
        assert!(
            code_symbol_neighborhood(&config, "missing", 1, 10)
                .expect("neighborhood read")
                .is_none()
        );
        assert!(
            compare_code_symbols(&config, "repo", "base", "head", 10)
                .expect("compare read")
                .diffs
                .is_empty()
        );
        assert!(
            !repo
                .path()
                .join(".opensymphony/memory/memory.duckdb")
                .exists()
        );
    }

    #[test]
    fn code_intel_read_helpers_schema_gate_legacy_index() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        std::fs::create_dir_all(config.index_path.parent().expect("index parent"))
            .expect("index dir");
        let connection = Connection::open(&config.index_path).expect("legacy index opens");
        connection
            .execute_batch(
                r#"
CREATE TABLE code_symbols (
  symbol_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  commit_sha TEXT,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  freshness TEXT NOT NULL
);
CREATE TABLE code_edges (
  edge_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  path TEXT NOT NULL,
  edge_kind TEXT NOT NULL,
  freshness TEXT NOT NULL
);
"#,
            )
            .expect("legacy schema");
        drop(connection);

        assert!(
            code_symbol_detail(&config, "missing")
                .expect("legacy detail read")
                .is_none()
        );
        assert!(
            code_symbols_containing_span(&config, "repo", "src/lib.rs", 1, 1, 10)
                .expect("legacy span read")
                .is_empty()
        );
        assert!(
            code_symbol_neighborhood(&config, "missing", 1, 10)
                .expect("legacy neighborhood read")
                .is_none()
        );
        assert!(
            compare_code_symbols(&config, "repo", "base", "head", 10)
                .expect("legacy compare read")
                .diffs
                .is_empty()
        );

        let connection = Connection::open(&config.index_path).expect("legacy index reopens");
        let symbol_key_columns: i64 = connection
            .query_row(
                "SELECT count(*) FROM pragma_table_info('code_symbols') WHERE name = 'symbol_key'",
                [],
                |row| row.get(0),
            )
            .expect("symbol_key column count");
        assert_eq!(symbol_key_columns, 0);
    }

    #[test]
    fn code_intel_impl_methods_link_to_owner_symbol_by_chain() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut document = sample_code_intel_document("hash-a", "pack-a");
        document.symbols = vec![
            CodeIntelSymbolInput {
                kind: "module".to_string(),
                name: "m".to_string(),
                container_chain: Vec::new(),
                signature: None,
                start_line: 1,
                start_col: 1,
                end_line: 8,
                end_col: 2,
                start_byte: 0,
                end_byte: 120,
                selection_start_line: 1,
                selection_end_line: 8,
                snippet_sha256: "module".to_string(),
            },
            CodeIntelSymbolInput {
                kind: "struct".to_string(),
                name: "Widget".to_string(),
                container_chain: vec!["m".to_string()],
                signature: None,
                start_line: 2,
                start_col: 1,
                end_line: 2,
                end_col: 15,
                start_byte: 16,
                end_byte: 30,
                selection_start_line: 2,
                selection_end_line: 2,
                snippet_sha256: "widget".to_string(),
            },
            CodeIntelSymbolInput {
                kind: "method".to_string(),
                name: "run".to_string(),
                container_chain: vec!["m".to_string(), "Widget".to_string()],
                signature: None,
                start_line: 3,
                start_col: 5,
                end_line: 3,
                end_col: 16,
                start_byte: 32,
                end_byte: 43,
                selection_start_line: 3,
                selection_end_line: 3,
                snippet_sha256: "run".to_string(),
            },
            CodeIntelSymbolInput {
                kind: "method".to_string(),
                name: "fmt".to_string(),
                container_chain: vec!["m".to_string(), "Widget".to_string(), "Display".to_string()],
                signature: None,
                start_line: 5,
                start_col: 5,
                end_line: 5,
                end_col: 16,
                start_byte: 64,
                end_byte: 75,
                selection_start_line: 5,
                selection_end_line: 5,
                snippet_sha256: "fmt".to_string(),
            },
        ];
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("base".to_string()),
                worktree_dirty: false,
                documents: vec![document],
            },
        )
        .expect("persist impl symbols");

        let method = code_symbols_containing_span(&config, "repo", "src/lib.rs", 3, 8, 10)
            .expect("span containment")
            .into_iter()
            .find(|symbol| symbol.name == "run")
            .expect("method symbol");
        assert!(method.container_symbol_id.is_some());
        assert_eq!(method.container_chain, vec!["m", "Widget"]);

        let widget = code_symbols_containing_span(&config, "repo", "src/lib.rs", 2, 8, 10)
            .expect("widget span containment")
            .into_iter()
            .find(|symbol| symbol.name == "Widget")
            .expect("scoped widget symbol");
        let trait_method = code_symbols_containing_span(&config, "repo", "src/lib.rs", 5, 8, 10)
            .expect("trait impl span containment")
            .into_iter()
            .find(|symbol| symbol.name == "fmt")
            .expect("trait impl method symbol");
        assert_eq!(
            trait_method.container_symbol_id.as_deref(),
            Some(widget.symbol_id.as_str())
        );
        assert_eq!(trait_method.container_chain, vec!["m", "Widget", "Display"]);
    }

    #[test]
    fn code_intel_read_model_resolves_containers_edges_bounds_and_diff() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut base = sample_code_intel_document("hash-a", "pack-a");
        base.symbols = vec![
            CodeIntelSymbolInput {
                kind: "struct".to_string(),
                name: "Widget".to_string(),
                container_chain: Vec::new(),
                signature: None,
                start_line: 1,
                start_col: 1,
                end_line: 8,
                end_col: 1,
                start_byte: 0,
                end_byte: 120,
                selection_start_line: 1,
                selection_end_line: 8,
                snippet_sha256: "widget".to_string(),
            },
            CodeIntelSymbolInput {
                kind: "method".to_string(),
                name: "run".to_string(),
                container_chain: vec!["Widget".to_string()],
                signature: None,
                start_line: 3,
                start_col: 5,
                end_line: 5,
                end_col: 6,
                start_byte: 20,
                end_byte: 80,
                selection_start_line: 3,
                selection_end_line: 5,
                snippet_sha256: "run-v1".to_string(),
            },
            CodeIntelSymbolInput {
                kind: "function".to_string(),
                name: "helper".to_string(),
                container_chain: Vec::new(),
                signature: None,
                start_line: 10,
                start_col: 1,
                end_line: 10,
                end_col: 20,
                start_byte: 140,
                end_byte: 160,
                selection_start_line: 10,
                selection_end_line: 10,
                snippet_sha256: "helper".to_string(),
            },
            CodeIntelSymbolInput {
                kind: "function".to_string(),
                name: "removed".to_string(),
                container_chain: Vec::new(),
                signature: None,
                start_line: 12,
                start_col: 1,
                end_line: 12,
                end_col: 20,
                start_byte: 170,
                end_byte: 190,
                selection_start_line: 12,
                selection_end_line: 12,
                snippet_sha256: "removed".to_string(),
            },
        ];
        base.edges = vec![
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("helper()".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 4,
                start_col: 9,
                end_line: 4,
                end_col: 17,
                start_byte: 40,
                end_byte: 48,
            },
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("Widget::helper()".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 4,
                start_col: 20,
                end_line: 4,
                end_col: 28,
                start_byte: 50,
                end_byte: 58,
            },
        ];
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("base".to_string()),
                worktree_dirty: false,
                documents: vec![base.clone()],
            },
        )
        .expect("base persist");

        let containing = code_symbols_containing_span(&config, "repo", "src/lib.rs", 4, 10, 10)
            .expect("span containment");
        assert_eq!(
            containing
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Widget", "run"]
        );
        let run_key = containing
            .iter()
            .find(|symbol| symbol.name == "run")
            .expect("run symbol")
            .symbol_key
            .clone();
        let detail = code_symbol_detail(&config, &run_key)
            .expect("detail query")
            .expect("run detail");
        assert_eq!(detail.container_chain, vec!["Widget"]);
        let endpoint = code_symbols_containing_span(&config, "repo", "src/lib.rs", 5, 6, 10)
            .expect("exclusive end containment");
        assert_eq!(
            endpoint
                .iter()
                .map(|symbol| symbol.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Widget"]
        );

        let neighborhood = code_symbol_neighborhood(&config, &run_key, 1, 10)
            .expect("neighborhood")
            .expect("center exists");
        assert!(!neighborhood.truncated);
        assert!(
            neighborhood
                .edges
                .iter()
                .any(|edge| edge.confidence == "syntactic"
                    && !edge.unresolved
                    && edge.target_symbol_key.is_some())
        );
        assert!(
            neighborhood
                .edges
                .iter()
                .any(
                    |edge| edge.target_hint.as_deref() == Some("Widget::helper()")
                        && edge.unresolved
                        && edge.target_symbol_key.is_none()
                )
        );
        let truncated = code_symbol_neighborhood(&config, &run_key, 1, 1)
            .expect("bounded neighborhood")
            .expect("center exists");
        assert!(truncated.truncated);
        assert!(
            truncated.edges.iter().all(|edge| [
                edge.source_symbol_key.as_deref(),
                edge.target_symbol_key.as_deref()
            ]
            .into_iter()
            .flatten()
            .all(|key| truncated
                .symbols
                .iter()
                .any(|symbol| symbol.symbol_key == key))),
            "bounded neighborhoods must not expose dangling edges"
        );

        let mut head = base;
        head.content_sha256 = "hash-b".to_string();
        head.symbols[1].snippet_sha256 = "run-v2".to_string();
        head.symbols.retain(|symbol| symbol.name != "removed");
        head.symbols.push(CodeIntelSymbolInput {
            kind: "function".to_string(),
            name: "added".to_string(),
            container_chain: Vec::new(),
            signature: None,
            start_line: 12,
            start_col: 1,
            end_line: 12,
            end_col: 20,
            start_byte: 170,
            end_byte: 190,
            selection_start_line: 12,
            selection_end_line: 12,
            snippet_sha256: "added".to_string(),
        });
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("head".to_string()),
                worktree_dirty: false,
                documents: vec![head],
            },
        )
        .expect("head persist");

        let comparison =
            compare_code_symbols(&config, "repo", "base", "head", 10).expect("compare revisions");
        assert!(
            comparison
                .diffs
                .iter()
                .any(|diff| matches!(diff.status, CodeSymbolDiffStatus::Added))
        );
        assert!(
            comparison
                .diffs
                .iter()
                .any(|diff| matches!(diff.status, CodeSymbolDiffStatus::Removed))
        );
        assert!(
            comparison
                .diffs
                .iter()
                .any(|diff| matches!(diff.status, CodeSymbolDiffStatus::Modified))
        );
        let exact_cap =
            compare_code_symbols(&config, "repo", "base", "head", comparison.diffs.len())
                .expect("exact capped compare");
        assert!(
            !exact_cap.truncated,
            "exact-sized diff pages must not truncate on unchanged trailing keys"
        );
        let capped =
            compare_code_symbols(&config, "repo", "base", "head", 1).expect("capped compare");
        assert!(capped.truncated);
    }

    #[test]
    fn code_intel_revision_comparison_preserves_unchanged_file_rows() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let document = sample_code_intel_document("hash-a", "pack-a");
        for revision in ["base", "head"] {
            persist_code_intel_documents(
                &config,
                CodeIntelPersistBatch {
                    repo_id: "repo".to_string(),
                    commit_sha: Some(revision.to_string()),
                    worktree_dirty: false,
                    documents: vec![document.clone()],
                },
            )
            .expect("persist revision");
        }

        {
            let connection = Connection::open(&config.index_path).expect("index opens");
            let rows = connection
                .prepare(
                    "SELECT commit_sha, symbol_id, symbol_key FROM code_symbols ORDER BY commit_sha",
                )
                .expect("prepare symbols")
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .expect("query symbols")
                .collect::<Result<Vec<_>, _>>()
                .expect("symbol rows");
            assert_eq!(rows.len(), 2);
            assert_ne!(rows[0].1, rows[1].1);
            assert_eq!(rows[0].2, rows[1].2);
        }

        let comparison =
            compare_code_symbols(&config, "repo", "base", "head", 10).expect("compare revisions");
        assert!(
            comparison.diffs.is_empty(),
            "unchanged symbols must not become added or removed"
        );

        let containing = code_symbols_containing_span(&config, "repo", "src/lib.rs", 1, 2, 10)
            .expect("span containment");
        assert_eq!(
            containing.len(),
            1,
            "current span lookups should collapse unchanged revision duplicates"
        );

        let mut dirty = document.clone();
        dirty.symbols[0].snippet_sha256 = "dirty-snippet".to_string();
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("head".to_string()),
                worktree_dirty: true,
                documents: vec![dirty],
            },
        )
        .expect("dirty revision persist");
        let connection =
            Connection::open(&config.index_path).expect("index reopens after dirty persist");
        for table in [
            "code_documents",
            "code_symbols",
            "code_edges",
            "code_diagnostics",
        ] {
            let dirty_key_rows: i64 = connection
                .query_row(
                    &format!("SELECT count(*) FROM {table} WHERE commit_sha LIKE '%+dirty'"),
                    [],
                    |row| row.get(0),
                )
                .expect("dirty revision key count");
            assert_eq!(
                dirty_key_rows, 0,
                "{table} must keep commit_sha as real HEAD"
            );
        }
        let comparison =
            compare_code_symbols(&config, "repo", "base", "head", 10).expect("compare revisions");
        assert!(
            comparison.diffs.is_empty(),
            "dirty worktree rows must not be compared as committed revisions"
        );
    }

    #[test]
    fn code_intel_neighborhood_skips_edges_without_source_symbols() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let mut document = sample_code_intel_document("hash-a", "pack-a");
        document.edges = vec![CodeIntelEdgeInput {
            edge_kind: "reference.call".to_string(),
            target_hint: Some("answer".to_string()),
            confidence: "query_pack:calls".to_string(),
            start_line: 2,
            start_col: 1,
            end_line: 2,
            end_col: 7,
            start_byte: 20,
            end_byte: 26,
        }];
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "repo".to_string(),
                commit_sha: Some("base".to_string()),
                worktree_dirty: false,
                documents: vec![document],
            },
        )
        .expect("persist source-less edge");

        let symbol = code_symbols_containing_span(&config, "repo", "src/lib.rs", 1, 2, 10)
            .expect("span containment")
            .into_iter()
            .next()
            .expect("answer symbol");
        let neighborhood = code_symbol_neighborhood(&config, &symbol.symbol_key, 1, 10)
            .expect("neighborhood")
            .expect("center exists");
        assert!(neighborhood.edges.is_empty());
        assert!(
            neighborhood.truncated,
            "dropped edges with missing source endpoints must be explicit"
        );
    }

    fn sample_code_intel_document(hash: &str, query_pack: &str) -> CodeIntelDocumentInput {
        CodeIntelDocumentInput {
            path: "src/lib.rs".into(),
            language: "rust".to_string(),
            content_sha256: hash.to_string(),
            parser_id: "tree-sitter".to_string(),
            parser_version: "tree-sitter-rust:0.26.9".to_string(),
            query_pack_version: query_pack.to_string(),
            byte_len: 24,
            line_count: 1,
            symbols: vec![CodeIntelSymbolInput {
                kind: "function".to_string(),
                name: "answer".to_string(),
                container_chain: Vec::new(),
                signature: None,
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 12,
                start_byte: 0,
                end_byte: 12,
                selection_start_line: 1,
                selection_end_line: 1,
                snippet_sha256: "snippet".to_string(),
            }],
            edges: vec![CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("answer".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 7,
                start_byte: 0,
                end_byte: 6,
            }],
            diagnostics: vec![CodeIntelDiagnosticInput {
                kind: "error".to_string(),
                severity: "error".to_string(),
                message: "ERROR parse diagnostic".to_string(),
                start_line: 1,
                start_col: 1,
                end_line: 1,
                end_col: 7,
                start_byte: 0,
                end_byte: 6,
            }],
        }
    }

    fn init_test_git_repo(root: &std::path::Path, branch: &str) {
        assert!(
            std::process::Command::new("git")
                .args(["init", "-b", branch])
                .current_dir(root)
                .status()
                .expect("git init")
                .success()
        );
        for (key, value) in [("user.email", "test@example.com"), ("user.name", "Test")] {
            assert!(
                std::process::Command::new("git")
                    .args(["config", key, value])
                    .current_dir(root)
                    .status()
                    .expect("git config")
                    .success()
            );
        }
        assert!(
            std::process::Command::new("git")
                .args(["add", "."])
                .current_dir(root)
                .status()
                .expect("git add")
                .success()
        );
        assert!(
            std::process::Command::new("git")
                .args(["commit", "-m", "baseline"])
                .current_dir(root)
                .status()
                .expect("git commit")
                .success()
        );
    }

    fn count_rows(connection: &Connection, table: &str, freshness: &str) -> i64 {
        connection
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE freshness = ?"),
                params![freshness],
                |row| row.get(0),
            )
            .expect("row count")
    }

    #[tokio::test]
    async fn okf_export_import_tools_dispatch_through_mcp() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        let memory_dir = repo_root.join(".opensymphony/memory/issues");
        std::fs::create_dir_all(&memory_dir).expect("memory dir");
        std::fs::write(
            memory_dir.join("COE-1.md"),
            r#"---
type: topic-doc
title: "COE-1: Public OKF concept"
description: Public concept.
tags: [memory, okf]
timestamp: 2026-06-23T10:00:00Z
opensymphony:
  visibility: public
  scope_refs:
    - kind: work_item
      id: COE-1
---

# COE-1: Public OKF concept

Public memory concept.
"#,
        )
        .expect("concept");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let export = call_memory_tool(
            &config,
            json!({
                "name": "memory.export_okf",
                "arguments": {
                    "visibility": "public",
                    "output": "public-okf"
                }
            }),
        )
        .await
        .expect("export okf tool");

        assert_eq!(export["outputPath"], "public-okf");
        assert_eq!(export["visibility"], "public");
        assert!(
            export["copiedFiles"]
                .as_array()
                .expect("copied files")
                .iter()
                .any(|path| path == "issues/COE-1.md")
        );

        let import = call_memory_tool(
            &config,
            json!({
                "name": "memory.import_okf",
                "arguments": {
                    "bundleRoot": "public-okf",
                    "force": true
                }
            }),
        )
        .await
        .expect("import okf tool");

        assert_eq!(import["sourcePath"], "public-okf");
        assert_eq!(import["targetPath"], ".opensymphony/memory");
        assert!(
            import["copiedFiles"]
                .as_array()
                .expect("copied files")
                .iter()
                .any(|path| path == "issues/COE-1.md")
        );
    }

    #[tokio::test]
    async fn mcp_memory_context_can_include_ast_code_intelligence() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(
            repo_root.join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("source file");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let response = call_memory_tool(
            &config,
            json!({
                "name": "memory.context",
                "arguments": {
                    "issue": "COE-999",
                    "paths": ["src/lib.rs"],
                    "includeCodeIntel": true
                }
            }),
        )
        .await
        .expect("context tool");

        let text = response["content"][0]["text"]
            .as_str()
            .expect("text content");
        assert!(text.contains("## Code Intelligence"));
        assert!(text.contains("ast-summary: src/lib.rs"));
        assert!(text.contains("function `answer`"));
        assert!(text.contains("fallback: CodebaseAnalyzer not used"));
    }

    #[tokio::test]
    async fn code_ast_status_returns_provider_versions_and_limits() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let status = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.status",
                "arguments": {}
            }),
        )
        .await
        .expect("status");

        assert_eq!(status["provider"], "tree-sitter-ast");
        assert_eq!(status["available"], true);
        assert!(
            status["languages"]
                .as_array()
                .expect("languages")
                .iter()
                .any(|language| language == "rust")
        );
        assert_eq!(status["queryPackVersions"]["rust"], RUST_QUERY_PACK_VERSION);
        assert_eq!(status["limits"]["maxMatchesPerRequest"], 2000);
    }

    #[tokio::test]
    async fn code_ast_context_returns_markdown_and_trace() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(
            repo_root.join("src/lib.rs"),
            "pub struct Thing {\n    value: u8,\n}\npub fn answer() -> u8 { 42 }\n",
        )
        .expect("source");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let context = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.context",
                "arguments": { "paths": ["src/lib.rs"], "symbols": ["struct"], "limit": 5 }
            }),
        )
        .await
        .expect("context");

        assert!(
            context["markdown"]
                .as_str()
                .expect("markdown")
                .contains("## Structural Context")
        );
        assert!(
            context["markdown"]
                .as_str()
                .expect("markdown")
                .contains("struct `Thing`")
        );
        assert!(
            !context["markdown"]
                .as_str()
                .expect("markdown")
                .contains("function `answer`")
        );
        assert!(
            context["trace"]
                .as_array()
                .expect("trace")
                .iter()
                .any(|line| line
                    .as_str()
                    .expect("trace line")
                    .contains("fallback: CodebaseAnalyzer not used"))
        );
    }

    #[tokio::test]
    async fn code_ast_outline_symbols_and_query_return_source_citations() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(
            repo_root.join("src/lib.rs"),
            "pub fn answer() -> u8 { helper() }\nfn helper() -> u8 { 42 }\n",
        )
        .expect("source");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let outline = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.outline",
                "arguments": { "paths": ["src/lib.rs"], "limit": 1 }
            }),
        )
        .await
        .expect("outline");
        assert_eq!(outline["documents"][0]["path"], "src/lib.rs");
        assert_eq!(outline["documents"][0]["language"], "rust");
        assert!(outline["documents"][0]["contentSha256"].as_str().is_some());
        assert!(
            outline["documents"][0]["parserVersion"]
                .as_str()
                .expect("parser version")
                .contains("tree-sitter-rust")
        );
        assert!(
            outline["documents"][0]["queryPackVersion"]
                .as_str()
                .expect("query pack")
                .starts_with("rust-query-pack")
        );
        assert_eq!(
            outline["documents"][0]["symbols"][0]["span"]["startLine"],
            1
        );
        assert_eq!(outline["limit"], 1);
        assert!(
            outline["trace"]
                .as_array()
                .expect("outline trace")
                .iter()
                .any(|line| line == "truncated by limit")
        );

        let symbols = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": {
                    "paths": ["src/lib.rs"],
                    "query": "answer",
                    "kinds": ["function"],
                    "limit": 10
                }
            }),
        )
        .await
        .expect("symbols");
        assert_eq!(symbols["symbols"][0]["name"], "answer");
        assert_eq!(
            symbols["symbols"][0]["source"]["queryPackVersion"],
            RUST_QUERY_PACK_VERSION
        );
        assert!(
            symbols["trace"]
                .as_array()
                .expect("symbols trace")
                .iter()
                .any(|line| line.as_str().expect("trace line").contains("src/lib.rs"))
        );

        let query = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.query",
                "arguments": {
                    "paths": ["src/lib.rs"],
                    "language": "rust",
                    "query": "(function_item name: (identifier) @definition.function)",
                    "limit": 1
                }
            }),
        )
        .await
        .expect("query");
        assert_eq!(query["matches"].as_array().expect("matches").len(), 1);
        assert_eq!(query["matches"][0]["captures"][0]["text"], "answer");
        assert_eq!(query["matches"][0]["captures"][0]["span"]["startLine"], 1);
        assert!(
            query["matches"][0]["source"]["contentSha256"]
                .as_str()
                .is_some()
        );
    }

    #[tokio::test]
    async fn code_ast_references_returns_span_and_source_citation() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(
            repo_root.join("src/lib.rs"),
            "fn helper() {}\nfn myhelper() {}\npub fn answer() { helper(); myhelper(); }\n",
        )
        .expect("source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_capture_bytes: 3\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let references = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.references",
                "arguments": { "paths": ["src/lib.rs"], "symbol": "helper" }
            }),
        )
        .await
        .expect("references");
        assert_eq!(
            references["references"]
                .as_array()
                .expect("references")
                .len(),
            1
        );
        let first = &references["references"][0];

        assert_eq!(first["path"], "src/lib.rs");
        assert_eq!(first["kind"], "reference.call");
        assert_eq!(first["span"]["startLine"], 3);
        assert_eq!(first["snippet"], "hel");
        assert_eq!(first["truncated"], true);
        assert!(first["source"]["contentSha256"].as_str().is_some());
        assert_eq!(first["source"]["queryPackVersion"], RUST_QUERY_PACK_VERSION);
        assert!(
            references["trace"]
                .as_array()
                .expect("references trace")
                .iter()
                .any(|line| line.as_str().expect("trace line").contains("src/lib.rs"))
        );
    }

    #[tokio::test]
    async fn code_ast_diagnostics_returns_parser_diagnostics_with_source() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn broken( {\n").expect("source");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let diagnostics = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.diagnostics",
                "arguments": { "paths": ["src/lib.rs"] }
            }),
        )
        .await
        .expect("diagnostics");
        let first = &diagnostics["diagnostics"][0];

        assert_eq!(first["path"], "src/lib.rs");
        assert!(
            matches!(
                first["kind"].as_str().expect("diagnostic kind"),
                "error" | "missing"
            ),
            "diagnostic kind should use the serialized AST diagnostic vocabulary"
        );
        assert!(first["span"]["startLine"].as_u64().expect("line") >= 1);
        assert!(first["source"]["contentSha256"].as_str().is_some());
        assert_eq!(first["source"]["queryPackVersion"], RUST_QUERY_PACK_VERSION);
    }

    #[tokio::test]
    async fn code_ast_tools_enforce_configured_match_limits() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(
            repo_root.join("src/lib.rs"),
            "pub fn one() -> u8 { 1 }\npub fn two() -> u8 { 2 }\n",
        )
        .expect("source");
        std::fs::write(
            repo_root.join("src/more.rs"),
            "pub fn three() -> u8 { 3 }\npub fn four() -> u8 { 4 }\n",
        )
        .expect("source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_matches_per_request: 1\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let symbols = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": { "paths": ["src/lib.rs"], "limit": 10 }
            }),
        )
        .await
        .expect("symbols");

        assert_eq!(symbols["symbols"].as_array().expect("symbols").len(), 1);
        assert_eq!(symbols["limit"], 1);

        let outline = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.outline",
                "arguments": { "paths": ["src"], "limit": 10 }
            }),
        )
        .await
        .expect("outline");
        let outline_symbol_count = outline["documents"]
            .as_array()
            .expect("documents")
            .iter()
            .map(|document| document["symbols"].as_array().expect("symbols").len())
            .sum::<usize>();
        assert_eq!(outline_symbol_count, 1);
        assert_eq!(outline["limit"], 1);
        assert!(
            outline["trace"]
                .as_array()
                .expect("outline trace")
                .iter()
                .any(|line| line == "truncated by limit")
        );
    }

    #[tokio::test]
    async fn code_ast_diagnostics_enforces_request_limit() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/bad.rs"), "pub fn broken( {\n").expect("source");
        std::fs::write(repo_root.join("src/worse.rs"), "pub fn worse( {\n").expect("source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_matches_per_request: 1\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let diagnostics = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.diagnostics",
                "arguments": { "paths": ["src"], "limit": 10 }
            }),
        )
        .await
        .expect("diagnostics");

        assert_eq!(
            diagnostics["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .len(),
            1
        );
        assert_eq!(diagnostics["limit"], 1);
        assert!(
            diagnostics["trace"]
                .as_array()
                .expect("diagnostics trace")
                .iter()
                .any(|line| line == "truncated by limit")
        );
    }

    #[tokio::test]
    async fn code_ast_file_limit_counts_supported_source_files() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/aaa.bin"), b"notes").expect("notes");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn answer() {}\n").expect("source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_files_per_request: 1\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let symbols = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": { "paths": ["src"], "limit": 10 }
            }),
        )
        .await
        .expect("symbols");

        assert_eq!(symbols["symbols"][0]["name"], "answer");
        assert!(
            symbols["trace"]
                .as_array()
                .expect("trace")
                .iter()
                .any(|line| line.as_str().expect("trace line").contains("parsed 1 file"))
        );
    }

    #[tokio::test]
    async fn code_ast_tools_skip_generated_and_vendor_directories_with_trace() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src/generated")).expect("src dir");
        std::fs::create_dir_all(repo_root.join("node_modules/pkg")).expect("node_modules");
        std::fs::create_dir_all(repo_root.join("vendor/pkg")).expect("vendor");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn answer() {}\n").expect("source");
        std::fs::write(
            repo_root.join("src/generated/mod.rs"),
            "pub fn generated_mod() {}\n",
        )
        .expect("explicit generated source");
        std::fs::write(
            repo_root.join("node_modules/pkg/lib.rs"),
            "pub fn generated_dep() {}\n",
        )
        .expect("generated source");
        std::fs::write(
            repo_root.join("vendor/pkg/lib.rs"),
            "pub fn vendored() {}\n",
        )
        .expect("vendor source");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let symbols = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": { "paths": ["src", "node_modules", "vendor"], "limit": 10 }
            }),
        )
        .await
        .expect("symbols");

        assert_eq!(symbols["symbols"][0]["name"], "answer");
        assert_eq!(symbols["symbols"].as_array().expect("symbols").len(), 1);
        let trace = symbols["trace"].as_array().expect("trace");
        assert!(trace.iter().any(|line| {
            line.as_str()
                .expect("trace line")
                .contains("warning: node_modules skipped directory `node_modules`")
        }));
        assert!(trace.iter().any(|line| {
            line.as_str()
                .expect("trace line")
                .contains("warning: vendor skipped directory `vendor`")
        }));
        assert!(trace.iter().any(|line| {
            line.as_str()
                .expect("trace line")
                .contains("warning: src/generated skipped directory `generated`")
        }));

        let explicit_generated = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": { "paths": ["src/generated/mod.rs"], "limit": 10 }
            }),
        )
        .await
        .expect("explicit generated symbols");

        assert_eq!(explicit_generated["symbols"][0]["name"], "generated_mod");
        assert!(
            !explicit_generated["trace"]
                .as_array()
                .expect("trace")
                .iter()
                .any(|line| line
                    .as_str()
                    .expect("trace line")
                    .contains("skipped directory"))
        );
    }

    #[tokio::test]
    async fn code_ast_tools_skip_oversized_files_with_trace() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/big.rs"), "pub fn oversized() {}\n")
            .expect("big source");
        std::fs::write(repo_root.join("src/small.rs"), "pub fn ok() {}\n").expect("small source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_file_bytes: 15\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let symbols = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": { "paths": ["src"], "limit": 10 }
            }),
        )
        .await
        .expect("symbols");

        assert_eq!(symbols["symbols"][0]["name"], "ok");
        assert_eq!(symbols["symbols"].as_array().expect("symbols").len(), 1);
        assert!(
            symbols["trace"]
                .as_array()
                .expect("trace")
                .iter()
                .any(|line| line
                    .as_str()
                    .expect("trace line")
                    .contains("warning: src/big.rs exceeds AST max_file_bytes 15"))
        );
    }

    #[tokio::test]
    async fn parallel_code_ast_tool_calls_do_not_share_parser_or_query_state() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(
            repo_root.join("src/lib.rs"),
            "pub fn answer() -> u8 { helper() }\nfn helper() -> u8 { 42 }\n",
        )
        .expect("source");
        std::fs::write(repo_root.join("src/bad.rs"), "pub fn broken( {\n").expect("bad source");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let symbols_config = config.clone();
        let diagnostics_config = config.clone();
        let query_config = config.clone();
        let (symbols, diagnostics, query) = tokio::join!(
            call_memory_tool(
                &symbols_config,
                json!({
                    "name": "code.ast.symbols",
                    "arguments": { "paths": ["src/lib.rs"], "limit": 10 }
                }),
            ),
            call_memory_tool(
                &diagnostics_config,
                json!({
                    "name": "code.ast.diagnostics",
                    "arguments": { "paths": ["src/bad.rs"], "limit": 1 }
                }),
            ),
            call_memory_tool(
                &query_config,
                json!({
                    "name": "code.ast.query",
                    "arguments": {
                        "paths": ["src/lib.rs"],
                        "language": "rust",
                        "query": "(function_item name: (identifier) @definition.function)",
                        "limit": 10
                    }
                }),
            )
        );

        let symbols = symbols.expect("symbols");
        let diagnostics = diagnostics.expect("diagnostics");
        let query = query.expect("query");
        assert!(
            symbols["symbols"]
                .as_array()
                .expect("symbols")
                .iter()
                .any(|symbol| symbol["name"] == "answer")
        );
        assert_eq!(
            diagnostics["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .len(),
            1
        );
        assert_eq!(
            query["matches"]
                .as_array()
                .expect("matches")
                .iter()
                .flat_map(|item| item["captures"].as_array().expect("captures"))
                .filter(|capture| capture["name"] == "definition.function")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn code_ast_query_truncates_large_captures() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn answer() {}\n").expect("source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_capture_bytes: 3\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let query = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.query",
                "arguments": {
                    "paths": ["src/lib.rs"],
                    "language": "rust",
                    "query": "(function_item name: (identifier) @definition.function)"
                }
            }),
        )
        .await
        .expect("query");

        assert_eq!(query["matches"][0]["captures"][0]["text"], "ans");
        assert_eq!(query["matches"][0]["captures"][0]["truncated"], true);
    }

    #[tokio::test]
    async fn code_ast_query_accepts_custom_capture_names() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn answer() {}\n").expect("source");
        let config = MemoryConfig::load(&repo_root, None).expect("config");

        let query = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.query",
                "arguments": {
                    "paths": ["src/lib.rs"],
                    "language": "rust",
                    "query": "(function_item name: (identifier) @my_capture)"
                }
            }),
        )
        .await
        .expect("query");

        assert_eq!(query["matches"][0]["captures"][0]["name"], "my_capture");
        assert_eq!(query["matches"][0]["captures"][0]["text"], "answer");
    }

    #[tokio::test]
    async fn code_ast_tools_reject_paths_outside_repo() {
        let repo = TempDir::new().expect("temp repo");
        let outside = TempDir::new().expect("outside repo");
        let outside_path = outside.path().join("lib.rs");
        std::fs::write(&outside_path, "pub fn outside() {}\n").expect("outside source");
        let config = MemoryConfig::load(repo.path(), None).expect("config");

        let error = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.outline",
                "arguments": { "paths": [outside_path] }
            }),
        )
        .await
        .expect_err("outside path should fail");

        assert!(matches!(error, MemoryError::PathOutsideRepo { .. }));
    }

    #[tokio::test]
    async fn code_ast_tools_validate_all_requested_paths_before_budget_skip() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn answer() {}\n").expect("source");
        let outside = TempDir::new().expect("outside repo");
        let outside_path = outside.path().join("lib.rs");
        std::fs::write(&outside_path, "pub fn outside() {}\n").expect("outside source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(
            &config_path,
            "code_intel:\n  ast:\n    max_files_per_request: 1\n",
        )
        .expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let error = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.symbols",
                "arguments": { "paths": ["src", outside_path] }
            }),
        )
        .await
        .expect_err("outside path should fail even after file budget is full");

        assert!(matches!(error, MemoryError::PathOutsideRepo { .. }));
    }

    #[tokio::test]
    async fn code_ast_tool_calls_fail_when_disabled() {
        let repo = TempDir::new().expect("temp repo");
        let repo_root = repo.path().canonicalize().expect("canonical repo");
        std::fs::create_dir_all(repo_root.join("src")).expect("src dir");
        std::fs::write(repo_root.join("src/lib.rs"), "pub fn answer() {}\n").expect("source");
        let config_path = repo_root.join("opensymphony-memory.yaml");
        std::fs::write(&config_path, "code_intel:\n  ast:\n    enabled: false\n").expect("config");
        let config = MemoryConfig::load(&repo_root, Some(&config_path)).expect("config");

        let error = call_memory_tool(
            &config,
            json!({
                "name": "code.ast.outline",
                "arguments": { "paths": ["src/lib.rs"] }
            }),
        )
        .await
        .expect_err("disabled AST tools should fail");

        assert!(matches!(error, MemoryError::InvalidInput(message)
            if message.contains("AST code-intelligence tools are disabled")));
    }

    #[test]
    fn admin_authorization_does_not_accept_worker_read_token() {
        let auth = MemoryServerAuth {
            read_token: Some("read-token".to_string()),
            admin_token: Some("admin-token".to_string()),
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer read-token"),
        );

        assert!(authorize_memory_request(&headers, &auth, MemoryServerAccess::Read).is_ok());
        let blocked = authorize_memory_request(&headers, &auth, MemoryServerAccess::Admin)
            .expect_err("admin tools need admin token");
        assert_eq!(blocked.0, axum::http::StatusCode::UNAUTHORIZED);

        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer admin-token"),
        );
        assert!(authorize_memory_request(&headers, &auth, MemoryServerAccess::Admin).is_ok());
    }

    #[test]
    fn read_authorization_requires_admin_token_when_only_admin_auth_is_configured() {
        let auth = MemoryServerAuth {
            read_token: None,
            admin_token: Some("admin-token".to_string()),
        };
        let headers = HeaderMap::new();

        let blocked = authorize_memory_request(&headers, &auth, MemoryServerAccess::Read)
            .expect_err("admin-only auth should protect read tools too");
        assert_eq!(blocked.0, axum::http::StatusCode::UNAUTHORIZED);

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer admin-token"),
        );
        assert!(authorize_memory_request(&headers, &auth, MemoryServerAccess::Read).is_ok());
    }

    #[test]
    fn health_reports_admin_tools_only_for_non_empty_admin_token() {
        let empty_admin = MemoryServerAuth {
            read_token: Some("read-token".to_string()),
            admin_token: Some("   ".to_string()),
        };
        let empty_payload = memory_server_health_payload(&empty_admin);
        assert_eq!(empty_payload["mode"], "read_only");
        assert_eq!(empty_payload["adminTools"], false);

        let configured_admin = MemoryServerAuth {
            read_token: Some("read-token".to_string()),
            admin_token: Some("admin-token".to_string()),
        };
        let configured_payload = memory_server_health_payload(&configured_admin);
        assert_eq!(configured_payload["mode"], "read_write");
        assert_eq!(configured_payload["adminTools"], true);
    }

    #[tokio::test]
    async fn memory_server_health_reports_pinned_config_generation() {
        let repo = TempDir::new().expect("repo");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let state = MemoryServerState {
            config,
            auth: MemoryServerAuth::default(),
            workspace_root: None,
            central_config_path: None,
            resolved_workflow: None,
            config_generation: Some("sha256:pinned-generation".to_string()),
            writer_gate: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        };

        let axum::Json(payload) = memory_server_health(axum::extract::State(state)).await;
        assert_eq!(payload["configGeneration"], "sha256:pinned-generation");
    }

    #[test]
    fn localhost_origin_check_rejects_prefix_spoofing() {
        assert!(origin_is_localhost("http://localhost:3333"));
        assert!(origin_is_localhost("https://127.0.0.1"));
        assert!(origin_is_localhost("http://[::1]:3333"));

        assert!(!origin_is_localhost("http://localhost.evil.com"));
        assert!(!origin_is_localhost("https://127.0.0.1.evil.com"));
        assert!(!origin_is_localhost("ftp://localhost"));
    }

    #[test]
    fn code_intel_repo_resolution_stays_inside_repo_root() {
        let repo = TempDir::new().expect("temp repo");
        let config = MemoryConfig::load(repo.path(), None).expect("config");
        std::fs::create_dir(repo.path().join("service")).expect("service dir");
        let resolved = resolve_code_intel_repo(&config, Some("service")).expect("inside repo");
        assert!(resolved.starts_with(repo.path().canonicalize().expect("canonical repo")));

        let outside = TempDir::new().expect("outside repo");
        let error = resolve_code_intel_repo(
            &config,
            Some(outside.path().to_str().expect("outside path")),
        )
        .expect_err("outside repo must be rejected");
        assert!(matches!(error, MemoryError::PathOutsideRepo { .. }));
    }

    #[test]
    fn code_intel_repo_resolution_uses_registered_canonical_identity() {
        let instance = TempDir::new().expect("instance");
        let repository = TempDir::new().expect("repository");
        let config = MemoryConfig::load(instance.path(), None)
            .expect("config")
            .with_repository_source(crate::opensymphony_memory::MemoryRepositorySource {
                repository_id: "github:repository:123".to_string(),
                root: repository.path().to_path_buf(),
                commit_sha: Some("abc123".to_string()),
            })
            .with_default_repository_id("github:repository:123");

        assert_eq!(
            resolve_code_intel_repo(&config, Some("github:repository:123"))
                .expect("canonical source should resolve"),
            repository.path()
        );
        let error = resolve_code_intel_repo(&config, Some("/tmp/not-a-repository"))
            .expect_err("local paths must not select a central source");
        assert!(
            matches!(error, MemoryError::InvalidInput(message) if message.contains("canonical repository"))
        );
    }

    #[test]
    fn remote_admin_tool_requires_admin_token_without_read_fallback() {
        let error = remote_memory_tool_token("memory.export_okf", |name| match name {
            "OPENSYMPHONY_MEMORY_TOKEN" => Some("read-token".to_string()),
            _ => None,
        })
        .expect_err("admin tool should fail before sending read token");
        assert!(
            matches!(error, MemoryError::InvalidInput(message) if message.contains("OPENSYMPHONY_MEMORY_ADMIN_TOKEN"))
        );

        let token = remote_memory_tool_token("memory.context", |name| match name {
            "OPENSYMPHONY_MEMORY_ADMIN_TOKEN" => Some("admin-token".to_string()),
            _ => None,
        })
        .expect("read tool can use admin token when no read token exists");
        assert_eq!(token, Some("admin-token".to_string()));
    }

    #[test]
    fn remote_client_timeout_outlasts_server_tool_timeout() {
        assert!(super::REMOTE_MEMORY_TOOL_TIMEOUT > super::MEMORY_MCP_TOOL_TIMEOUT);
    }

    #[test]
    fn remote_response_reports_http_status_before_json_parse_errors() {
        let error = parse_remote_memory_response(
            reqwest::StatusCode::BAD_GATEWAY,
            "upstream unavailable",
            "memory.context",
        )
        .expect_err("HTTP failure should report status");

        assert!(matches!(error, MemoryError::InvalidInput(message)
                if message.contains("HTTP 502 Bad Gateway")
                    && message.contains("upstream unavailable")
                    && !message.contains("not valid JSON")));
    }

    #[test]
    fn mcp_context_source_preserves_worker_issue_graph() {
        let source = context_source_from_mcp(&json!({
            "issue": "COE-999",
            "currentIssue": {
                "id": "issue-999",
                "identifier": "COE-999",
                "title": "Memory context",
                "description": "Use deterministic facts.",
                "state": "In Progress",
                "labels": ["area:memory"],
                "children": [
                    { "id": "issue-101", "identifier": "COE-101", "state": "Done" }
                ],
                "blockedBy": [
                    { "id": "issue-100", "identifier": "COE-100", "state": "Done" }
                ]
            }
        }));

        assert_eq!(source.issues.len(), 1);
        assert_eq!(source.issues[0].identifier, "COE-999");
        assert_eq!(source.issues[0].labels, vec!["area:memory"]);
        assert_eq!(source.issues[0].children[0].identifier, "COE-101");
        assert_eq!(source.issues[0].blocked_by[0].identifier, "COE-100");
    }

    #[test]
    fn managed_linear_memory_status_replaces_existing_section() {
        let existing = format!(
            "Intro\n\n{LINEAR_MEMORY_STATUS_BEGIN}\nold\n{LINEAR_MEMORY_STATUS_END}\n\nTail"
        );
        let replacement = format!("{LINEAR_MEMORY_STATUS_BEGIN}\nnew\n{LINEAR_MEMORY_STATUS_END}");

        let updated = replace_or_append_managed_section(
            &existing,
            LINEAR_MEMORY_STATUS_BEGIN,
            LINEAR_MEMORY_STATUS_END,
            &replacement,
        );

        assert!(updated.contains("Intro"));
        assert!(updated.contains("new"));
        assert!(updated.contains("Tail"));
        assert!(!updated.contains("old"));
    }

    #[test]
    fn managed_linear_memory_status_replaces_truncated_section() {
        let existing = format!("Intro\n\n{LINEAR_MEMORY_STATUS_BEGIN}\nold without end marker");
        let replacement = format!("{LINEAR_MEMORY_STATUS_BEGIN}\nnew\n{LINEAR_MEMORY_STATUS_END}");

        let updated = replace_or_append_managed_section(
            &existing,
            LINEAR_MEMORY_STATUS_BEGIN,
            LINEAR_MEMORY_STATUS_END,
            &replacement,
        );

        assert!(updated.contains("Intro"));
        assert!(updated.contains("new"));
        assert_eq!(updated.matches(LINEAR_MEMORY_STATUS_BEGIN).count(), 1);
        assert!(!updated.contains("old without end marker"));
    }

    #[test]
    fn auto_memory_status_log_keeps_recent_entries() {
        let contents = "\
# OpenSymphony Memory Automation Log

## 2026-05-16T00:00:00Z

- Captured: COE-1

## 2026-05-16T00:01:00Z

- Captured: COE-2

## 2026-05-16T00:02:00Z

- Captured: COE-3
";

        let trimmed = trim_auto_memory_status_log(contents, 2, usize::MAX);

        assert!(!trimmed.contains("COE-1"));
        assert!(trimmed.contains("COE-2"));
        assert!(trimmed.contains("COE-3"));
        assert_eq!(trimmed.matches("## ").count(), 2);
    }

    #[test]
    fn auto_memory_status_log_respects_size_limit() {
        let contents = "\
# OpenSymphony Memory Automation Log

## 2026-05-16T00:00:00Z

- Captured: COE-1

## 2026-05-16T00:01:00Z

- Captured: COE-2 with a longer status line

## 2026-05-16T00:02:00Z

- Captured: COE-3 with a longer status line
";

        let trimmed = trim_auto_memory_status_log(contents, 100, 120);

        assert!(!trimmed.contains("COE-1"));
        assert!(!trimmed.contains("COE-2"));
        assert!(trimmed.contains("COE-3"));
        assert!(trimmed.len() <= 120);
    }
}
