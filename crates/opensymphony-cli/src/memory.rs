use std::{
    collections::BTreeSet,
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{self, ExitCode},
    time::Duration,
};

use chrono::{NaiveDate, Utc};
use clap::{Args, Subcommand};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::task::JoinHandle;

use crate::{
    opensymphony_domain::{TrackerIssue, TrackerIssueBlocker, TrackerIssueRef},
    opensymphony_linear::{LinearClient, LinearConfig},
    opensymphony_memory::{
        ArchivePlan, CodeIntelArtifact, CodeIntelIndex, CommentEvidence, DocsSyncPlan,
        IssueEvidence, IssueLinkEvidence, IssueSelection, KnowledgeScope, KnowledgeScopeKind,
        LintSeverity, MemoryConfig, MemoryContextOptions, MemoryError, MemoryReindexReport,
        MemoryScopeFilter, SourceFile, archive_blocking_warning_count, brief,
        context_for_issue_with_options, docs_for_area_with_scope, expand_issue_range, lint,
        lint_okf_bundle, load_source_file, mark_archived, plan_archive, plan_capture,
        plan_docs_sync, plan_memory_init, refresh_memory_index, related_by_area_with_scope,
        related_by_issue_with_scope, related_by_paths_with_scope, render_archive_plan,
        render_capture_dry_run, search_with_scope, status_with_scope, write_capture_plan,
        write_docs_sync_plan, write_memory_init_plan,
    },
    opensymphony_openhands::{
        ConversationMoveOutcome, ConversationStoreKind, IssueConversationManifest,
        OpenHandsConversationStorePaths,
    },
    opensymphony_planning::CodebaseAnalyzer,
    opensymphony_workflow::WorkflowDefinition,
    opensymphony_workspace::{CleanupConfig, HookConfig, WorkspaceManager, WorkspaceManagerConfig},
};

const MEMORY_MCP_TOOL_TIMEOUT: Duration = Duration::from_secs(300);
const REMOTE_MEMORY_TOOL_TIMEOUT: Duration = Duration::from_secs(330);

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

pub(crate) async fn auto_capture_terminal(
    repo_root: &Path,
    workflow_path: &Path,
    identifiers: &[String],
    conversation_store: Option<&OpenHandsConversationStorePaths>,
    auto_archive: bool,
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

    let config = MemoryConfig::load(repo_root, None)?;
    let client = linear_client_from_workflow(repo_root, Some(workflow_path))?;
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
        match MemoryConfig::load(repo_root, None) {
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
                match archive_in_linear(repo_root, Some(workflow_path), &archive_plan).await {
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
    if let Some(endpoint) = env::var("OPENSYMPHONY_MEMORY_ENDPOINT")
        .ok()
        .and_then(|value| non_empty(&value))
        && let Some((tool_name, arguments)) = remote_memory_tool_request(&command)
    {
        return run_remote_memory_tool(&endpoint, tool_name, arguments).await;
    }
    match command {
        MemoryCommand::Init(args) => run_init(&repo_root, config_path.as_deref(), args),
        MemoryCommand::Capture(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_capture(&repo_root, &config, args).await
        }
        MemoryCommand::Import(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_import(&config, args)
        }
        MemoryCommand::SyncDocs(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_sync_docs(&config, args)
        }
        MemoryCommand::Status(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_status(&config, args)
        }
        MemoryCommand::Show(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_show(&config, args, ShowMode::Full)
        }
        MemoryCommand::Brief(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_show(&config, args, ShowMode::Brief)
        }
        MemoryCommand::Search(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_search(&config, args)
        }
        MemoryCommand::Related(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_related(&config, args)
        }
        MemoryCommand::Docs(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_docs(&config, args)
        }
        MemoryCommand::Context(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_context(&repo_root, &config, args).await
        }
        MemoryCommand::Serve(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_serve(config, args).await
        }
        MemoryCommand::Lint(args) => {
            let config = MemoryConfig::load(&repo_root, config_path.as_deref())?;
            run_lint(&config, args)
        }
    }
}

async fn run_linear(args: LinearArgs) -> Result<(), MemoryError> {
    match args.command {
        LinearCommand::Archive(args) => run_archive(args).await,
    }
}

fn run_init(
    repo_root: &Path,
    config_path: Option<&Path>,
    args: InitArgs,
) -> Result<(), MemoryError> {
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
    let source = load_linear_source(repo_root, None, &identifiers).await?;
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
    args: ContextArgs,
) -> Result<(), MemoryError> {
    let mut warnings = Vec::new();
    let source = match load_linear_context_source(repo_root, None, &args.issue).await {
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

#[derive(Clone)]
struct MemoryServerState {
    config: MemoryConfig,
    auth: MemoryServerAuth,
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

async fn run_serve(config: MemoryConfig, args: ServeArgs) -> Result<(), MemoryError> {
    let handle = start_memory_server_with_auth(
        config,
        args.addr,
        MemoryServerAuth {
            read_token: args.token,
            admin_token: args.admin_token,
        },
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
    task: JoinHandle<Result<(), String>>,
}

impl MemoryServerHandle {
    pub(crate) fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    pub(crate) fn abort(&self) {
        self.task.abort();
    }

    pub(crate) async fn wait(self) -> Result<(), MemoryError> {
        match self.task.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(MemoryError::InvalidInput(error)),
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(MemoryError::InvalidInput(format!(
                "memory server task failed: {error}"
            ))),
        }
    }
}

pub(crate) async fn start_memory_server(
    config: MemoryConfig,
    addr: SocketAddr,
    token: Option<String>,
) -> Result<MemoryServerHandle, MemoryError> {
    start_memory_server_with_auth(
        config,
        addr,
        MemoryServerAuth {
            read_token: token,
            admin_token: None,
        },
    )
    .await
}

async fn start_memory_server_with_auth(
    config: MemoryConfig,
    addr: SocketAddr,
    auth: MemoryServerAuth,
) -> Result<MemoryServerHandle, MemoryError> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|error| {
        MemoryError::InvalidInput(format!("failed to bind memory server {addr}: {error}"))
    })?;
    let local_addr = listener.local_addr().map_err(|error| {
        MemoryError::InvalidInput(format!("failed to read memory server address: {error}"))
    })?;
    let state = MemoryServerState { config, auth };
    let app = axum::Router::new()
        .route("/health", axum::routing::get(memory_server_health))
        .route("/mcp", axum::routing::post(memory_server_mcp))
        .with_state(state);
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .map_err(|error| format!("memory server failed: {error}"))
    });
    Ok(MemoryServerHandle {
        endpoint: format!("http://{local_addr}/mcp"),
        task,
    })
}

async fn memory_server_health(
    axum::extract::State(state): axum::extract::State<MemoryServerState>,
) -> axum::Json<Value> {
    axum::Json(memory_server_health_payload(&state.auth))
}

fn memory_server_health_payload(auth: &MemoryServerAuth) -> Value {
    let admin_tools = non_empty_str(auth.admin_token.as_deref()).is_some();
    json!({
        "status": "ok",
        "protocol": "mcp-streamable-http-2025-06-18",
        "mode": if admin_tools { "read_write" } else { "read_only" },
        "adminTools": admin_tools
    })
}

async fn memory_server_mcp(
    axum::extract::State(state): axum::extract::State<MemoryServerState>,
    headers: axum::http::HeaderMap,
    axum::Json(request): axum::Json<MemoryMcpRequest>,
) -> (axum::http::StatusCode, axum::Json<Value>) {
    if let Err(response) =
        authorize_memory_request(&headers, &state.auth, required_access_for_request(&request))
    {
        return response;
    }
    let id = request.id.clone();
    let result = match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "serverInfo": { "name": "opensymphony-memory", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": {} }
        })),
        "tools/list" => Ok(json!({
            "tools": memory_tool_descriptors()
        })),
        "tools/call" => match tokio::time::timeout(
            MEMORY_MCP_TOOL_TIMEOUT,
            call_memory_tool(&state.config, request.params),
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

fn required_access_for_request(request: &MemoryMcpRequest) -> MemoryServerAccess {
    if request.method == "tools/call"
        && request
            .params
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(is_admin_memory_tool)
    {
        MemoryServerAccess::Admin
    } else {
        MemoryServerAccess::Read
    }
}

fn memory_tool_descriptors() -> Vec<Value> {
    vec![
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
        json!({ "name": "memory.ingest_code_intel", "description": "Generate code-intelligence artifacts for future ingestion", "access": "admin" }),
    ]
}

fn is_admin_memory_tool(name: &str) -> bool {
    matches!(
        name,
        "memory.capture"
            | "memory.sync_docs"
            | "memory.lint"
            | "memory.reindex"
            | "memory.ingest_code_intel"
    )
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

async fn call_memory_tool(config: &MemoryConfig, params: Value) -> Result<Value, MemoryError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| MemoryError::InvalidInput("tools/call requires params.name".to_string()))?;
    let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
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
            Ok(json!({
                "issueCount": report.issue_count,
                "warningCount": report.warning_count,
                "docsPendingCount": report.docs_pending_count,
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
        "memory.capture" => call_memory_capture_tool(config, &arguments).await,
        "memory.sync_docs" => call_memory_sync_docs_tool(config, &arguments),
        "memory.lint" => call_memory_lint_tool(config, &arguments),
        "memory.reindex" => call_memory_reindex_tool(config),
        "memory.ingest_code_intel" => call_memory_ingest_code_intel_tool(config, &arguments).await,
        other => Err(MemoryError::InvalidInput(format!(
            "unsupported memory tool `{other}`"
        ))),
    }
}

async fn call_memory_capture_tool(
    config: &MemoryConfig,
    arguments: &Value,
) -> Result<Value, MemoryError> {
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
    } else {
        load_linear_source(&config.repo_root, None, &identifiers).await?
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

fn call_memory_reindex_tool(config: &MemoryConfig) -> Result<Value, MemoryError> {
    Ok(memory_reindex_report_json(
        config,
        refresh_memory_index(config)?,
    ))
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
    let repo_root = resolve_code_intel_repo(config, scope.repo.as_deref())?;
    let scope_refs = scope_refs_for_context(&scope, &paths);
    let artifacts = code_intel_artifacts_blocking(repo_root, paths, scope_refs, limit).await?;
    Ok(json!({
        "persisted": false,
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
                "id": source.id
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    }))
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
        "indexPath": path_for_json(config, &report.index_path),
        "markdownIndexes": paths_for_json(config, &report.markdown_indexes)
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
    let artifacts = CodebaseAnalyzer::new(repo_root).code_context(paths, &scope_refs, limit)?;
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
    scope_refs: Vec<KnowledgeScope>,
    limit: usize,
) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
    tokio::task::spawn_blocking(move || {
        CodebaseAnalyzer::new(repo_root).code_context(&paths, &scope_refs, limit)
    })
    .await
    .map_err(|error| {
        MemoryError::InvalidInput(format!("code-intelligence analysis task failed: {error}"))
    })?
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
        return Ok(config.repo_root.clone());
    };
    let resolved = repo_existing_path(config, &repo)?;
    if !resolved.is_dir() {
        return Err(MemoryError::InvalidInput(format!(
            "context repo `{repo}` did not resolve to a directory at {}",
            resolved.display()
        )));
    }
    Ok(resolved)
}

fn scope_refs_for_context(scope: &MemoryScopeFilter, paths: &[PathBuf]) -> Vec<KnowledgeScope> {
    let mut refs = Vec::new();
    push_scope_ref(
        &mut refs,
        KnowledgeScopeKind::ProjectSet,
        scope.project_set.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        KnowledgeScopeKind::Project,
        scope.project.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        KnowledgeScopeKind::Milestone,
        scope.milestone.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        KnowledgeScopeKind::WorkItem,
        scope.issue.as_deref(),
    );
    push_scope_ref(
        &mut refs,
        KnowledgeScopeKind::Repository,
        scope.repo.as_deref(),
    );
    push_scope_ref(&mut refs, KnowledgeScopeKind::Area, scope.area.as_deref());
    for path in paths {
        refs.push(KnowledgeScope {
            kind: KnowledgeScopeKind::CodePath,
            id: path.display().to_string(),
            label: None,
        });
    }
    refs
}

fn push_scope_ref(refs: &mut Vec<KnowledgeScope>, kind: KnowledgeScopeKind, id: Option<&str>) {
    if let Some(id) = id.and_then(non_empty) {
        refs.push(KnowledgeScope {
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

fn bool_arg(arguments: &Value, key: &str) -> bool {
    arguments.get(key).and_then(Value::as_bool).unwrap_or(false)
}

async fn run_archive(args: ArchiveArgs) -> Result<(), MemoryError> {
    let repo_root = env::current_dir().map_err(|source| MemoryError::ReadFile {
        path: PathBuf::from("."),
        source,
    })?;
    let config = MemoryConfig::load(&repo_root, args.config.as_deref())?;
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
    let report = archive_in_linear(&repo_root, args.workflow.as_deref(), &plan).await?;
    if !report.archived.is_empty() {
        mark_archived(&config, &report.archived)?;
    }
    let conversation_report = archive_openhands_conversations_from_config(
        &repo_root,
        args.workflow.as_deref(),
        &report.archived,
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
    let selection = IssueSelection {
        identifiers: identifiers.clone(),
        ..IssueSelection::default()
    };
    let source = load_linear_source(repo_root, args.workflow.as_deref(), &identifiers).await?;
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

    let report = archive_in_linear(repo_root, args.workflow.as_deref(), &archive_plan).await?;
    finish_archive_write(repo_root, args.workflow.as_deref(), config, report).await
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
    config: &MemoryConfig,
    report: LinearArchiveReport,
) -> Result<(), MemoryError> {
    if !report.archived.is_empty() {
        mark_archived(config, &report.archived)?;
    }
    let conversation_report =
        archive_openhands_conversations_from_config(repo_root, workflow_path, &report.archived)
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
) -> Result<ConversationArchiveReport, MemoryError> {
    let store = conversation_store_from_run_config(repo_root, workflow_path)?;
    let context = conversation_archive_context(repo_root, workflow_path, store.as_ref())?;
    archive_openhands_conversations_for_issues_with_context(context.as_ref(), issue_keys).await
}

async fn archive_openhands_conversations_for_issues(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    conversation_store: Option<&OpenHandsConversationStorePaths>,
    issue_keys: &[String],
) -> Result<ConversationArchiveReport, MemoryError> {
    let context = conversation_archive_context(repo_root, workflow_path, conversation_store)?;
    archive_openhands_conversations_for_issues_with_context(context.as_ref(), issue_keys).await
}

fn conversation_archive_context<'a>(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    conversation_store: Option<&'a OpenHandsConversationStorePaths>,
) -> Result<Option<ConversationArchiveContext<'a>>, MemoryError> {
    let Some(conversation_store) = conversation_store else {
        return Ok(None);
    };
    let workflow = load_resolved_workflow(repo_root, workflow_path)?;
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
) -> Result<Option<OpenHandsConversationStorePaths>, MemoryError> {
    let config_path = repo_root.join("config.yaml");
    if !config_path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&config_path).map_err(|source| MemoryError::ReadFile {
        path: config_path.clone(),
        source,
    })?;
    let config =
        serde_yaml::from_str::<ConversationArchiveRuntimeConfig>(&raw).map_err(|source| {
            MemoryError::ParseYaml {
                path: config_path.clone(),
                source,
            }
        })?;
    let config_root = config_path.parent().unwrap_or(repo_root);
    let target_repo = match workflow_path.and_then(Path::parent) {
        Some(workflow_root) => workflow_root.to_path_buf(),
        None => config
            .target_repo
            .as_deref()
            .map(|value| expand_config_path(&config_path, config_root, value))
            .transpose()?
            .unwrap_or_else(|| repo_root.to_path_buf()),
    };
    let Some(tool_dir) = config
        .openhands
        .tool_dir
        .as_deref()
        .map(|value| expand_config_path(&config_path, config_root, value))
        .transpose()?
    else {
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

fn load_resolved_workflow(
    repo_root: &Path,
    workflow_path: Option<&Path>,
) -> Result<crate::opensymphony_workflow::ResolvedWorkflow, MemoryError> {
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
    plan: &ArchivePlan,
) -> Result<LinearArchiveReport, MemoryError> {
    let client = linear_client_from_workflow(repo_root, workflow_path)?;
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
    let workflow_path = workflow_path
        .map(Path::to_path_buf)
        .unwrap_or_else(|| repo_root.join("WORKFLOW.md"));
    if !workflow_path.exists() {
        return Err(MemoryError::InvalidInput(format!(
            "{} not found",
            workflow_path.display()
        )));
    }
    let workflow = WorkflowDefinition::load_from_path(&workflow_path)
        .map_err(|error| MemoryError::InvalidInput(format!("failed to load workflow: {error}")))?;
    let workflow_root = workflow_path.parent().unwrap_or(repo_root);
    let resolved = workflow
        .resolve_with_process_env(workflow_root)
        .map_err(|error| {
            MemoryError::InvalidInput(format!("failed to resolve workflow: {error}"))
        })?;
    let mut linear_config = LinearConfig::new(
        resolved.config.tracker.api_key,
        resolved.config.tracker.project_slug,
    );
    linear_config.base_url = resolved.config.tracker.endpoint;
    linear_config.active_states = resolved.config.tracker.active_states;
    linear_config.terminal_states = resolved.config.tracker.terminal_states;
    LinearClient::new(linear_config)
        .map_err(|error| MemoryError::Linear(format!("invalid Linear config: {error}")))
}

async fn load_linear_source(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    identifiers: &[String],
) -> Result<SourceFile, MemoryError> {
    let client = linear_client_from_workflow(repo_root, workflow_path)?;
    load_linear_source_from_client(&client, identifiers).await
}

async fn load_linear_context_source(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    issue_key: &str,
) -> Result<SourceFile, MemoryError> {
    let client = linear_client_from_workflow(repo_root, workflow_path)?;
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
        MemoryServerAuth, authorize_memory_request, context_source_from_mcp,
        memory_server_health_payload, memory_tool_descriptors, origin_is_localhost,
        parse_remote_memory_response, remote_memory_tool_token, replace_or_append_managed_section,
        required_access_for_request, resolve_code_intel_repo, trim_auto_memory_status_log,
    };
    use crate::opensymphony_memory::{MemoryConfig, MemoryError};
    use axum::http::{HeaderMap, HeaderValue, header};
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn mcp_tool_list_exposes_context_and_admin_tools_without_code_context() {
        let names = memory_tool_descriptors()
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
        assert!(!names.iter().any(|name| name.contains("code_context")));
        assert!(!names.iter().any(|name| name.contains("code-context")));
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

        assert_eq!(
            required_access_for_request(&read_request),
            MemoryServerAccess::Read
        );
        assert_eq!(
            required_access_for_request(&admin_request),
            MemoryServerAccess::Admin
        );
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
    fn remote_admin_tool_requires_admin_token_without_read_fallback() {
        let error = remote_memory_tool_token("memory.capture", |name| match name {
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
