use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use chrono::NaiveDate;
use clap::{Args, Subcommand};

use crate::{
    opensymphony_domain::TrackerIssue,
    opensymphony_linear::{LinearClient, LinearConfig},
    opensymphony_memory::{
        ArchivePlan, CommentEvidence, DocsSyncPlan, IssueEvidence, IssueSelection, LintSeverity,
        MemoryConfig, MemoryError, SourceFile, brief, context_for_issue, docs_for_area,
        expand_issue_range, lint, load_source_file, mark_archived, plan_archive, plan_capture,
        plan_docs_sync, related_by_area, related_by_issue, related_by_paths, render_archive_plan,
        render_capture_dry_run, search, status, write_capture_plan, write_docs_sync_plan,
    },
    opensymphony_workflow::WorkflowDefinition,
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
    #[command(about = "Capture completed issue evidence into issue memory")]
    Capture(CaptureArgs),
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
    #[arg(long, help = "YAML source evidence file for deterministic capture")]
    source_file: Option<PathBuf>,
    #[arg(long, help = "Only show the capture plan")]
    dry_run: bool,
    #[arg(long, help = "Write capsules and update the DuckDB index")]
    write: bool,
    #[arg(long, help = "Overwrite generated or non-generated existing capsules")]
    force: bool,
    #[arg(long, help = "Use gh to discover matching GitHub PRs")]
    discover_github: bool,
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
    #[arg(long, help = "Write topic docs and mark selected memory synced")]
    write: bool,
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
    #[arg(long, help = "Optional YAML source evidence file")]
    source_file: Option<PathBuf>,
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
    #[arg(long, help = "Select archive candidates from captured memory")]
    from_memory: bool,
    #[arg(
        long,
        help = "Filter --from-memory candidates by Linear or memory state"
    )]
    state: Option<String>,
    #[arg(long, help = "Only show archive eligibility")]
    dry_run: bool,
    #[arg(long, help = "Archive eligible issues in Linear")]
    write: bool,
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

async fn run_memory(args: MemoryArgs) -> Result<(), MemoryError> {
    let repo_root = std::env::current_dir().map_err(|source| MemoryError::ReadFile {
        path: PathBuf::from("."),
        source,
    })?;
    let config = MemoryConfig::load(&repo_root, args.config.as_deref())?;
    match args.command {
        MemoryCommand::Capture(args) => run_capture(&repo_root, &config, args).await,
        MemoryCommand::SyncDocs(args) => run_sync_docs(&config, args),
        MemoryCommand::Status(args) => run_status(&config, args),
        MemoryCommand::Show(args) => run_show(&config, args, false),
        MemoryCommand::Brief(args) => run_show(&config, args, true),
        MemoryCommand::Search(args) => run_search(&config, args),
        MemoryCommand::Related(args) => run_related(&config, args),
        MemoryCommand::Docs(args) => run_docs(&config, args),
        MemoryCommand::Context(args) => run_context(&config, args),
        MemoryCommand::Lint(args) => run_lint(&config, args),
    }
}

async fn run_linear(args: LinearArgs) -> Result<(), MemoryError> {
    match args.command {
        LinearCommand::Archive(args) => run_archive(args).await,
    }
}

async fn run_capture(
    repo_root: &Path,
    config: &MemoryConfig,
    args: CaptureArgs,
) -> Result<(), MemoryError> {
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
    let (source, source_warnings) =
        load_capture_source(repo_root, args.source_file.as_deref(), &selection).await?;
    let write = args.write && !args.dry_run;
    let mut plan = plan_capture(config, &source, &selection, write, args.discover_github)?;
    plan.warnings.extend(source_warnings);
    if !write {
        println!("{}", render_capture_dry_run(config, &plan));
        println!("Dry run only. Re-run with `--write` to create capsules and update the index.");
        return Ok(());
    }

    let report = write_capture_plan(config, &plan, args.force)?;
    println!("Wrote {} capsule(s).", report.written_capsules.len());
    for path in report.written_capsules {
        println!("- {}", path.display());
    }
    println!("Updated DuckDB index: {}", report.index_path.display());
    for path in report.markdown_indexes {
        println!("Updated markdown index: {}", path.display());
    }
    if !report.warnings.is_empty() {
        println!("\nWarnings:");
        for warning in report.warnings {
            println!("- {warning}");
        }
    }
    Ok(())
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
    let write = args.write && !args.dry_run;
    let plan = plan_docs_sync(config, &selection, write, args.with_diagrams)?;
    print_docs_plan(&plan);
    if !write {
        println!("Dry run only. Re-run with `--write` to update topic docs.");
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

fn run_show(config: &MemoryConfig, args: ShowArgs, compact: bool) -> Result<(), MemoryError> {
    if compact {
        println!("{}", brief(config, &args.issue)?);
        return Ok(());
    }
    let path = config.issue_capsule_path(&args.issue);
    let contents = fs::read_to_string(&path).map_err(|source| MemoryError::ReadFile {
        path: path.clone(),
        source,
    })?;
    println!("{contents}");
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
    let source = load_optional_source(args.source_file.as_deref())?;
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
        None,
    )?;
    let write = args.write && !args.dry_run;
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
        println!("Dry run only. Re-run with `--write` to archive eligible Linear issues.");
        return Ok(());
    }
    archive_in_linear(&repo_root, args.workflow.as_deref(), &plan).await?;
    let archived = plan
        .issues
        .iter()
        .filter(|issue| issue.eligible)
        .map(|issue| issue.issue_key.clone())
        .collect::<Vec<_>>();
    mark_archived(&config, &archived)?;
    println!("Archived {} Linear issue(s).", archived.len());
    for issue_key in archived {
        println!("- {issue_key}");
    }
    Ok(())
}

async fn archive_in_linear(
    repo_root: &Path,
    workflow_path: Option<&Path>,
    plan: &ArchivePlan,
) -> Result<(), MemoryError> {
    let client = linear_client_from_workflow(repo_root, workflow_path)?;

    for issue in plan.issues.iter().filter(|issue| issue.eligible) {
        client
            .archive_issue(&issue.issue_key)
            .await
            .map_err(|error| {
                MemoryError::Linear(format!("failed to archive {}: {error}", issue.issue_key))
            })?;
    }
    Ok(())
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

fn load_optional_source(path: Option<&Path>) -> Result<SourceFile, MemoryError> {
    match path {
        Some(path) => load_source_file(path),
        None => Ok(SourceFile::default()),
    }
}

async fn load_capture_source(
    repo_root: &Path,
    source_file: Option<&Path>,
    selection: &IssueSelection,
) -> Result<(SourceFile, Vec<String>), MemoryError> {
    if let Some(path) = source_file {
        return Ok((load_source_file(path)?, Vec::new()));
    }
    if selection.identifiers.is_empty() {
        return Ok((SourceFile::default(), Vec::new()));
    }

    match load_linear_source(repo_root, &selection.identifiers).await {
        Ok(source) => Ok((source, Vec::new())),
        Err(error) => Ok((
            SourceFile::default(),
            vec![format!(
                "Linear source discovery skipped; using supplied issue identifiers only: {error}"
            )],
        )),
    }
}

async fn load_linear_source(
    repo_root: &Path,
    identifiers: &[String],
) -> Result<SourceFile, MemoryError> {
    let client = linear_client_from_workflow(repo_root, None)?;
    let tracker_issues = client
        .issues_by_identifiers(identifiers)
        .await
        .map_err(|error| MemoryError::Linear(format!("Linear issue lookup failed: {error}")))?;

    let mut issues = Vec::new();
    for issue in tracker_issues {
        let workpad = client.fetch_workpad_comment(&issue.id).await.ok().flatten();
        issues.push(issue_evidence_from_tracker(issue, workpad));
    }

    Ok(SourceFile {
        issues,
        ..SourceFile::default()
    })
}

fn issue_evidence_from_tracker(
    issue: TrackerIssue,
    workpad: Option<crate::opensymphony_linear::WorkpadComment>,
) -> IssueEvidence {
    IssueEvidence {
        id: Some(issue.id),
        identifier: issue.identifier,
        title: issue.title,
        url: Some(issue.url),
        description: issue.description,
        state: Some(issue.state),
        labels: issue.labels,
        comments: workpad
            .map(|comment| {
                vec![CommentEvidence {
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
    println!("# Docs Sync Plan\n");
    println!("Selected issues: {}", plan.selected_issue_keys.join(", "));
    for target in &plan.targets {
        println!(
            "\n## {} ({})\n\n{}",
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
