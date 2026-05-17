use std::{
    fs,
    path::{Path, PathBuf},
    process::{self, ExitCode},
};

use chrono::{NaiveDate, Utc};
use clap::{Args, Subcommand};
use serde::Deserialize;

use crate::{
    opensymphony_domain::{TrackerIssue, TrackerIssueRef},
    opensymphony_linear::{LinearClient, LinearConfig},
    opensymphony_memory::{
        ArchivePlan, CommentEvidence, DocsSyncPlan, IssueEvidence, IssueSelection, LintSeverity,
        MemoryConfig, MemoryError, SourceFile, archive_blocking_warning_count, brief,
        context_for_issue, docs_for_area, expand_issue_range, lint, load_source_file,
        mark_archived, plan_archive, plan_capture, plan_docs_sync, plan_memory_init,
        related_by_area, related_by_issue, related_by_paths, render_archive_plan,
        render_capture_dry_run, search, status, write_capture_plan, write_docs_sync_plan,
        write_memory_init_plan,
    },
    opensymphony_openhands::{
        ConversationMoveOutcome, IssueConversationManifest, OpenHandsConversationStorePaths,
    },
    opensymphony_workflow::WorkflowDefinition,
    opensymphony_workspace::{CleanupConfig, HookConfig, WorkspaceManager, WorkspaceManagerConfig},
};

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
    #[arg(help = "Search query")]
    query: String,
    #[arg(long, default_value = "10", help = "Maximum results")]
    limit: usize,
}

#[derive(Debug, Args)]
struct RelatedArgs {
    #[arg(long, help = "Find memory related to this issue")]
    issue: Option<String>,
    #[arg(long, help = "Find memory related to this area")]
    area: Option<String>,
    #[arg(long, value_delimiter = ',', help = "Find memory related to paths")]
    paths: Vec<PathBuf>,
    #[arg(long, default_value = "10", help = "Maximum results")]
    limit: usize,
}

#[derive(Debug, Args)]
struct DocsArgs {
    #[arg(long, help = "Area slug")]
    area: String,
}

#[derive(Debug, Args)]
struct ContextArgs {
    #[arg(long, help = "Issue identifier")]
    issue: String,
    #[arg(long, default_value = "8", help = "Maximum related memories")]
    limit: usize,
}

#[derive(Debug, Args)]
struct LintArgs {
    #[arg(long, help = "Check public docs for private memory links")]
    public_docs: bool,
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
    let repo_root = std::env::current_dir().map_err(|source| MemoryError::ReadFile {
        path: PathBuf::from("."),
        source,
    })?;
    let MemoryArgs {
        config: config_path,
        command,
    } = args;
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
            run_context(&config, args)
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
    let report = status(
        config,
        &IssueSelection {
            milestone: args.milestone,
            area: args.area,
            ..IssueSelection::default()
        },
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
    let results = search(config, &args.query, args.limit)?;
    print_search_results(config, &results);
    Ok(())
}

fn run_related(config: &MemoryConfig, args: RelatedArgs) -> Result<(), MemoryError> {
    let results = if let Some(issue) = args.issue {
        related_by_issue(config, &issue, args.limit)?
    } else if let Some(area) = args.area {
        related_by_area(config, &area, args.limit)?
    } else if !args.paths.is_empty() {
        related_by_paths(config, &args.paths, args.limit)?
    } else {
        return Err(MemoryError::InvalidInput(
            "provide one of --issue, --area, or --paths".to_string(),
        ));
    };
    print_search_results(config, &results);
    Ok(())
}

fn run_docs(config: &MemoryConfig, args: DocsArgs) -> Result<(), MemoryError> {
    println!("{}", docs_for_area(config, &args.area)?);
    Ok(())
}

fn run_context(config: &MemoryConfig, args: ContextArgs) -> Result<(), MemoryError> {
    let source = SourceFile::default();
    println!(
        "{}",
        context_for_issue(config, &source, &args.issue, args.limit)?
    );
    Ok(())
}

fn run_lint(config: &MemoryConfig, args: LintArgs) -> Result<(), MemoryError> {
    let report = lint(config, args.public_docs)?;
    if report.findings.is_empty() {
        println!("Memory lint passed.");
        return Ok(());
    }
    for finding in report.findings {
        let severity = match finding.severity {
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

async fn run_archive(args: ArchiveArgs) -> Result<(), MemoryError> {
    let repo_root = std::env::current_dir().map_err(|source| MemoryError::ReadFile {
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
    moved: Vec<String>,
    already_archived: Vec<String>,
    warnings: Vec<String>,
    failures: Vec<String>,
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
        let Some(workspace) = context
            .manager
            .find_workspace_by_issue_reference(issue_key)
            .await
            .map_err(|error| {
                MemoryError::InvalidInput(format!(
                    "failed to find workspace for {issue_key}: {error}"
                ))
            })?
        else {
            report.warnings.push(format!(
                "{issue_key}: skipped OpenHands conversation archive; no managed workspace found"
            ));
            continue;
        };
        let manifest_path = workspace.conversation_manifest_path();
        let Some(raw_manifest) = context
            .manager
            .read_text_artifact(&workspace, &manifest_path)
            .await
            .map_err(|error| {
                MemoryError::InvalidInput(format!(
                    "failed to read conversation manifest for {issue_key}: {error}"
                ))
            })?
        else {
            report.warnings.push(format!(
                "{issue_key}: skipped OpenHands conversation archive; no conversation manifest found"
            ));
            continue;
        };
        let manifest =
            serde_json::from_str::<IssueConversationManifest>(&raw_manifest).map_err(|error| {
                MemoryError::InvalidInput(format!(
                    "failed to decode conversation manifest {} for {issue_key}: {error}",
                    manifest_path.display()
                ))
            })?;

        match context
            .conversation_store
            .move_conversation_to(
                manifest.conversation_id.as_str(),
                crate::opensymphony_openhands::ConversationStoreKind::Archived,
            )
            .map_err(|error| error.to_string())
        {
            Ok(ConversationMoveOutcome::Moved { .. }) => report.moved.push(issue_key.clone()),
            Ok(ConversationMoveOutcome::AlreadyInTarget { .. }) => {
                report.already_archived.push(issue_key.clone());
            }
            Ok(ConversationMoveOutcome::Missing) => {
                report.warnings.push(format!(
                    "{issue_key}: OpenHands conversation {} was not found in the active, archived, or legacy stores",
                    manifest.conversation_id
                ));
            }
            Err(error) => {
                report.failures.push(format!(
                    "{issue_key}: failed to archive OpenHands conversation {}: {error}",
                    manifest.conversation_id
                ));
            }
        }
    }

    Ok(report)
}

fn print_conversation_archive_report(report: &ConversationArchiveReport) {
    if !report.moved.is_empty() {
        println!("Archived {} OpenHands conversation(s).", report.moved.len());
        for issue_key in &report.moved {
            println!("- {issue_key}");
        }
    }
    if !report.already_archived.is_empty() {
        println!(
            "{} OpenHands conversation(s) were already archived.",
            report.already_archived.len()
        );
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
    let mut seen = std::collections::BTreeSet::new();
    let mut pending = identifiers
        .iter()
        .map(|identifier| identifier.trim().to_string())
        .filter(|identifier| !identifier.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
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

fn issue_link_from_tracker_ref(
    issue: &TrackerIssueRef,
) -> crate::opensymphony_memory::IssueLinkEvidence {
    crate::opensymphony_memory::IssueLinkEvidence {
        id: Some(issue.id.clone()),
        identifier: issue.identifier.clone(),
        title: issue.title.clone(),
        url: issue.url.clone(),
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
        LINEAR_MEMORY_STATUS_BEGIN, LINEAR_MEMORY_STATUS_END, replace_or_append_managed_section,
        trim_auto_memory_status_log,
    };

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
