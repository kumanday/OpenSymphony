use std::{
    cmp::Ordering,
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command as StdCommand, ExitCode, Stdio},
};

use clap::Args;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

use super::memory_init_summary::memory_init_change_lists;
use crate::opensymphony_cli::init_repo::{
    self, InitCommandError, ReviewProviderArg, TargetBranch, WORKFLOW_AUTOMATED_REVIEW_HEADING,
};
use crate::opensymphony_memory::{MemoryInitApplyReport, ensure_memory_initialized};

const DEFAULT_CRATE_METADATA_URL: &str = "https://crates.io/api/v1/crates/opensymphony";
const WORKFLOW_TARGET_BRANCH_HEADING: &str = "## Branch target";
const WORKFLOW_TARGET_BRANCH_MARKER: &str = "Target branch:";
const WORKFLOW_REVIEW_PROVIDER_MARKER: &str = "Active review provider:";
const OPENHANDS_REVIEW_WORKFLOW_PATH: &str = ".github/workflows/ai-pr-review.yml";
const OPENHANDS_REVIEW_WORKFLOW_FILE: &str = "ai-pr-review.yml";
const LEGACY_WORKFLOW_TARGET_REMOTE_REF: &str = "origin/main";
const LEGACY_BRANCH_CONTROL_PHRASES: &[&str] = &[
    "Keep feature branches current with `origin/main`.",
    "latest origin/main before handoff",
    "latest `origin/main` before handoff",
    "branch from origin/main and restart",
    "branch from `origin/main` and restart",
    "sync with latest origin/main before",
    "sync with latest `origin/main` before",
    "Merge latest origin/main into branch",
    "Merge latest `origin/main` into branch",
    "Create a fresh branch from origin/main.",
    "Create a fresh branch from `origin/main`.",
    "merged origin/main clean",
    "merged `origin/main` clean",
];

#[derive(Debug, Args, Clone)]
pub struct UpdateArgs {
    #[arg(
        long,
        value_name = "BRANCH",
        value_parser = TargetBranch::parse,
        help = "Patch the managed WORKFLOW.md target branch marker without reinstalling or refreshing template skills"
    )]
    target_branch: Option<TargetBranch>,
    #[arg(
        long,
        value_enum,
        value_name = "PROVIDER",
        help = "Patch the managed WORKFLOW.md review provider marker and toggle any existing OpenHands review workflow: codex, openhands, or none"
    )]
    code_review: Option<ReviewProviderArg>,
}

impl UpdateArgs {
    fn workflow_settings_mode(&self) -> bool {
        self.target_branch.is_some() || self.code_review.is_some()
    }
}

#[derive(Debug, Error)]
enum UpdateCommandError {
    #[error("failed to determine the current working directory: {0}")]
    CurrentDir(#[source] io::Error),
    #[error("failed to build the update client: {0}")]
    HttpClient(#[source] reqwest::Error),
    #[error("invalid update metadata URL `{value}`: {source}")]
    InvalidMetadataUrl {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("failed to fetch the latest published OpenSymphony version from {url}: {source}")]
    FetchLatestVersion {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to fetch the latest published OpenSymphony version from {url}: HTTP {status}")]
    FetchLatestVersionStatus { url: String, status: StatusCode },
    #[error("latest-version response from {url} was not valid JSON: {source}")]
    DecodeLatestVersion {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to run `cargo install opensymphony`: {0}")]
    CargoInstall(#[source] io::Error),
    #[error("`cargo install opensymphony` exited with {status}")]
    CargoInstallFailed { status: String },
    #[error("{0}")]
    Template(#[from] InitCommandError),
    #[error("failed to initialize project memory: {0}")]
    MemoryInit(#[from] crate::opensymphony_memory::MemoryError),
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
    #[error("workflow settings mode requires an OpenSymphony target repo; missing {missing}")]
    MissingTargetRepoMarkers { missing: String },
    #[error("WORKFLOW.md has multiple managed `{marker}` markers")]
    MultipleWorkflowMarkers { marker: &'static str },
    #[error("malformed managed `{marker}` marker in WORKFLOW.md; expected `{example}`")]
    MalformedWorkflowMarker {
        marker: &'static str,
        example: String,
    },
}

#[derive(Debug, Deserialize)]
struct CrateMetadataResponse {
    #[serde(rename = "crate")]
    krate: PublishedCrate,
}

#[derive(Debug, Deserialize)]
struct PublishedCrate {
    max_version: String,
}

#[derive(Debug)]
enum SelfUpdateAction {
    SkipUpToDate,
    SkipCurrentNewer,
    Install,
}

#[derive(Debug)]
struct SelfUpdatePlan {
    current_version: String,
    latest_version: String,
    action: SelfUpdateAction,
}

#[derive(Debug)]
struct TargetRepoMarkers {
    has_workflow: bool,
    has_config: bool,
}

impl TargetRepoMarkers {
    fn looks_like_target_repo(&self) -> bool {
        self.has_workflow && self.has_config
    }

    fn missing_markers(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.has_workflow {
            missing.push("WORKFLOW.md");
        }
        if !self.has_config {
            missing.push("config.yaml");
        }
        missing
    }
}

#[derive(Debug, Default)]
struct SkillSyncReport {
    created: Vec<String>,
    updated: Vec<String>,
    unchanged_count: usize,
}

pub async fn run_command(args: UpdateArgs) -> ExitCode {
    match run_update(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("opensymphony update failed: {error}");
            ExitCode::from(1)
        }
    }
}

async fn run_update(args: UpdateArgs) -> Result<(), UpdateCommandError> {
    let current_dir = env::current_dir().map_err(UpdateCommandError::CurrentDir)?;
    println!("Updating OpenSymphony from {}", current_dir.display());

    if args.workflow_settings_mode() {
        update_workflow_settings(&current_dir, &args)?;
        println!("OpenSymphony workflow settings update complete.");
        return Ok(());
    }

    let client = Client::builder()
        .user_agent(concat!("opensymphony-cli/", env!("CARGO_PKG_VERSION")))
        .timeout(init_repo::template_fetch_timeout())
        .build()
        .map_err(UpdateCommandError::HttpClient)?;

    let update_plan = plan_self_update(&client).await?;
    run_self_update(&update_plan).await?;

    let target_repo = detect_target_repo_markers(&current_dir);
    if !target_repo.looks_like_target_repo() {
        let missing = target_repo.missing_markers();
        println!(
            "Skipped template skill refresh because this directory is missing {}.",
            join_for_display(&missing)
        );
        println!("OpenSymphony update complete.");
        return Ok(());
    }

    println!("Detected an OpenSymphony target repo; refreshing template-managed skill files.");
    let report = sync_template_skills(&current_dir, &client).await?;
    let memory_report = ensure_memory_initialized(&current_dir, None)?;

    println!("Skill refresh summary:");
    print_paths("Created", &report.created);
    print_paths("Updated", &report.updated);
    println!("Unchanged: {} file(s)", report.unchanged_count);
    print_memory_init_summary(&current_dir, &memory_report);
    println!("OpenSymphony update complete.");
    Ok(())
}

fn update_workflow_settings(
    current_dir: &Path,
    args: &UpdateArgs,
) -> Result<(), UpdateCommandError> {
    let target_repo = detect_target_repo_markers(current_dir);
    if !target_repo.looks_like_target_repo() {
        return Err(UpdateCommandError::MissingTargetRepoMarkers {
            missing: join_for_display(&target_repo.missing_markers()),
        });
    }

    let workflow_path = current_dir.join("WORKFLOW.md");
    let workflow =
        fs::read_to_string(&workflow_path).map_err(|source| UpdateCommandError::ReadFile {
            path: workflow_path.clone(),
            source,
        })?;
    let patched =
        patch_workflow_settings(&workflow, args.target_branch.as_ref(), args.code_review)?;

    if patched == workflow {
        println!("WORKFLOW.md already matches requested settings.");
    } else {
        write_file(&workflow_path, &patched)?;
        println!("Updated WORKFLOW.md managed settings.");
    }

    if let Some(code_review) = args.code_review {
        sync_openhands_review_workflow(current_dir, code_review);
    }

    Ok(())
}

fn sync_openhands_review_workflow(current_dir: &Path, code_review: ReviewProviderArg) {
    if !current_dir.join(OPENHANDS_REVIEW_WORKFLOW_PATH).is_file() {
        if matches!(code_review, ReviewProviderArg::Openhands) {
            eprintln!(
                "Warning: `--code-review openhands` updated WORKFLOW.md but {OPENHANDS_REVIEW_WORKFLOW_PATH} is missing; update mode did not install or enable the OpenHands GitHub Actions review workflow."
            );
        }
        return;
    }

    let action = match code_review {
        ReviewProviderArg::Openhands => "enable",
        ReviewProviderArg::Codex | ReviewProviderArg::None => "disable",
    };
    let command = format!("gh workflow {action} {OPENHANDS_REVIEW_WORKFLOW_FILE}");
    match StdCommand::new("gh")
        .args(["workflow", action, OPENHANDS_REVIEW_WORKFLOW_FILE])
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .output()
    {
        Ok(output) if output.status.success() => {
            println!("{action}d existing OpenHands GitHub Actions review workflow.");
        }
        Ok(output) => {
            eprintln!(
                "Warning: `{command}` exited with {}; WORKFLOW.md marker remains updated, but the existing OpenHands GitHub Actions review workflow state was not synchronized.",
                render_exit_status(output.status)
            );
        }
        Err(source) => {
            eprintln!(
                "Warning: failed to run `{command}`: {source}; WORKFLOW.md marker remains updated, but the existing OpenHands GitHub Actions review workflow state was not synchronized."
            );
        }
    }
}

fn patch_workflow_settings(
    workflow: &str,
    target_branch: Option<&TargetBranch>,
    code_review: Option<ReviewProviderArg>,
) -> Result<String, UpdateCommandError> {
    let had_crlf = workflow.contains("\r\n");
    let mut patched = workflow.replace("\r\n", "\n");

    if let Some(target_branch) = target_branch {
        let marker_patch = patch_target_branch_marker(patched, target_branch)?;
        patched = replace_legacy_branch_control_phrases(
            marker_patch.workflow,
            marker_patch.previous_value.as_deref(),
            target_branch,
        );
    }

    if let Some(code_review) = code_review {
        patched = patch_review_provider_marker(patched, code_review)?.workflow;
    }

    if had_crlf {
        Ok(patched.replace('\n', "\r\n"))
    } else {
        Ok(patched)
    }
}

struct MarkerPatch {
    workflow: String,
    previous_value: Option<String>,
}

fn patch_target_branch_marker(
    workflow: String,
    target_branch: &TargetBranch,
) -> Result<MarkerPatch, UpdateCommandError> {
    patch_marker_line(
        workflow,
        WORKFLOW_TARGET_BRANCH_HEADING,
        WORKFLOW_TARGET_BRANCH_MARKER,
        target_branch.local(),
        || target_branch_section(target_branch),
    )
}

fn patch_review_provider_marker(
    workflow: String,
    code_review: ReviewProviderArg,
) -> Result<MarkerPatch, UpdateCommandError> {
    patch_marker_line(
        workflow,
        WORKFLOW_AUTOMATED_REVIEW_HEADING,
        WORKFLOW_REVIEW_PROVIDER_MARKER,
        code_review.as_str(),
        || review_provider_section(code_review),
    )
}

fn patch_marker_line<F>(
    workflow: String,
    section_heading: &'static str,
    marker: &'static str,
    value: &str,
    missing_section: F,
) -> Result<MarkerPatch, UpdateCommandError>
where
    F: FnOnce() -> String,
{
    let mut matches = Vec::new();
    let mut offset = 0;
    let mut in_section = false;
    for line in workflow.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = line_without_newline.trim();
        if trimmed == section_heading {
            in_section = true;
        } else if in_section {
            if trimmed.starts_with('#') {
                in_section = false;
            } else if let Some(value) = marker_line_value(line_without_newline, marker)? {
                matches.push((
                    offset,
                    offset + line_without_newline.len(),
                    value.to_string(),
                ));
            }
        }
        offset += line.len();
    }

    match matches.as_slice() {
        [] => Ok(MarkerPatch {
            workflow: insert_managed_section(workflow, &missing_section(), marker),
            previous_value: None,
        }),
        [(start, end, previous_value)] => {
            let mut patched = workflow;
            patched.replace_range(*start..*end, &format!("{marker} `{value}`"));
            Ok(MarkerPatch {
                workflow: patched,
                previous_value: Some(previous_value.clone()),
            })
        }
        _ => Err(UpdateCommandError::MultipleWorkflowMarkers { marker }),
    }
}

fn marker_line_value<'a>(
    line: &'a str,
    marker: &'static str,
) -> Result<Option<&'a str>, UpdateCommandError> {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix(marker) else {
        return Ok(None);
    };
    let rest = rest.trim();
    let Some(value) = rest
        .strip_prefix('`')
        .and_then(|rest| rest.strip_suffix('`'))
    else {
        return Err(UpdateCommandError::MalformedWorkflowMarker {
            marker,
            example: marker_example(marker),
        });
    };
    if value.trim().is_empty() || value.contains('`') {
        return Err(UpdateCommandError::MalformedWorkflowMarker {
            marker,
            example: marker_example(marker),
        });
    }
    Ok(Some(value))
}

fn marker_example(marker: &'static str) -> String {
    let value = if marker == WORKFLOW_REVIEW_PROVIDER_MARKER {
        "codex"
    } else {
        "develop"
    };
    format!("{marker} `{value}`")
}

fn insert_managed_section(mut workflow: String, section: &str, marker: &str) -> String {
    if marker == WORKFLOW_TARGET_BRANCH_MARKER
        && let Some(index) = workflow.find(WORKFLOW_AUTOMATED_REVIEW_HEADING)
    {
        workflow.insert_str(index, section);
        return workflow;
    }
    if marker == WORKFLOW_REVIEW_PROVIDER_MARKER
        && let Some(index) = workflow.find(WORKFLOW_AUTOMATED_REVIEW_HEADING)
    {
        let mut insert_at = workflow[index..]
            .find('\n')
            .map(|line_end| index + line_end + 1)
            .unwrap_or_else(|| workflow.len());
        if insert_at == workflow.len() && !workflow.ends_with('\n') {
            workflow.push('\n');
            insert_at = workflow.len();
        }
        let marker_only = section
            .strip_prefix(WORKFLOW_AUTOMATED_REVIEW_HEADING)
            .unwrap_or(section)
            .trim_start();
        workflow.insert_str(insert_at, marker_only);
        return workflow;
    }

    if !workflow.is_empty() && !workflow.ends_with('\n') {
        workflow.push('\n');
    }
    if !workflow.is_empty() && !workflow.ends_with("\n\n") {
        workflow.push('\n');
    }
    workflow.push_str(section);
    workflow
}

fn target_branch_section(target_branch: &TargetBranch) -> String {
    format!(
        "{WORKFLOW_TARGET_BRANCH_HEADING}\n\nTarget branch: `{}`\n\n<!-- Set by `opensymphony init` or `opensymphony update --target-branch`.\n     Value is a local branch name, not an `origin/...` ref. Agents should use\n     `origin/<target-branch>` when syncing, creating replacement branches, and\n     preparing PRs. -->\n\n",
        target_branch.local()
    )
}

fn review_provider_section(code_review: ReviewProviderArg) -> String {
    format!(
        "{WORKFLOW_AUTOMATED_REVIEW_HEADING}\n\nActive review provider: `{}`\n\n<!-- Set by `opensymphony init` or `opensymphony update --code-review`; valid values: `openhands`, `codex`, `none`. -->\n",
        code_review.as_str()
    )
}

fn replace_legacy_branch_control_phrases(
    workflow: String,
    previous_branch: Option<&str>,
    target_branch: &TargetBranch,
) -> String {
    let remote_ref = target_branch.remote_ref();
    let mut workflow = workflow;
    let mut source_refs = vec![LEGACY_WORKFLOW_TARGET_REMOTE_REF.to_string()];
    if let Some(previous_branch) = previous_branch {
        let previous_ref = format!("origin/{previous_branch}");
        if previous_ref != LEGACY_WORKFLOW_TARGET_REMOTE_REF {
            source_refs.push(previous_ref);
        }
    }
    for phrase in LEGACY_BRANCH_CONTROL_PHRASES {
        for source_ref in &source_refs {
            let source_phrase = phrase.replace(LEGACY_WORKFLOW_TARGET_REMOTE_REF, source_ref);
            let target_phrase = phrase.replace(LEGACY_WORKFLOW_TARGET_REMOTE_REF, &remote_ref);
            workflow = workflow.replace(&source_phrase, &target_phrase);
        }
    }
    workflow
}

async fn plan_self_update(client: &Client) -> Result<SelfUpdatePlan, UpdateCommandError> {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let latest_version = fetch_latest_version(client).await?;
    let action = match compare_versions(&current_version, &latest_version) {
        Some(Ordering::Less) => SelfUpdateAction::Install,
        Some(Ordering::Equal) => SelfUpdateAction::SkipUpToDate,
        Some(Ordering::Greater) => SelfUpdateAction::SkipCurrentNewer,
        None => SelfUpdateAction::Install,
    };

    Ok(SelfUpdatePlan {
        current_version,
        latest_version,
        action,
    })
}

async fn fetch_latest_version(client: &Client) -> Result<String, UpdateCommandError> {
    let metadata_url = env::var("OPENSYMPHONY_UPDATE_CRATE_METADATA_URL")
        .unwrap_or_else(|_| DEFAULT_CRATE_METADATA_URL.to_string());
    let metadata_url =
        Url::parse(&metadata_url).map_err(|source| UpdateCommandError::InvalidMetadataUrl {
            value: metadata_url.clone(),
            source,
        })?;

    let response = client
        .get(metadata_url.clone())
        .send()
        .await
        .map_err(|source| UpdateCommandError::FetchLatestVersion {
            url: metadata_url.to_string(),
            source,
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(UpdateCommandError::FetchLatestVersionStatus {
            url: metadata_url.to_string(),
            status,
        });
    }

    let metadata = response
        .json::<CrateMetadataResponse>()
        .await
        .map_err(|source| UpdateCommandError::DecodeLatestVersion {
            url: metadata_url.to_string(),
            source,
        })?;

    Ok(metadata.krate.max_version)
}

async fn run_self_update(plan: &SelfUpdatePlan) -> Result<(), UpdateCommandError> {
    println!("Current CLI version: {}", plan.current_version);
    println!("Latest published version: {}", plan.latest_version);

    match plan.action {
        SelfUpdateAction::SkipUpToDate => {
            println!(
                "Current version matches the latest published release; skipping `cargo install opensymphony`."
            );
            Ok(())
        }
        SelfUpdateAction::SkipCurrentNewer => {
            println!(
                "Current version is newer than the latest published release; skipping `cargo install opensymphony`."
            );
            Ok(())
        }
        SelfUpdateAction::Install => {
            println!("Running `cargo install opensymphony`...");
            let status = Command::new("cargo")
                .args(["install", "opensymphony"])
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .await
                .map_err(UpdateCommandError::CargoInstall)?;

            if !status.success() {
                return Err(UpdateCommandError::CargoInstallFailed {
                    status: render_exit_status(status),
                });
            }

            println!(
                "Installed published OpenSymphony {}. The next `opensymphony` invocation will use it.",
                plan.latest_version
            );
            Ok(())
        }
    }
}

async fn sync_template_skills(
    target_repo: &Path,
    client: &Client,
) -> Result<SkillSyncReport, UpdateCommandError> {
    let assets = init_repo::fetch_template_skill_assets(client).await?;
    let mut report = SkillSyncReport::default();

    for asset in assets {
        let destination = target_repo.join(&asset.path);
        match fs::read_to_string(&destination) {
            Ok(existing) => {
                if comparable_text(&existing) == comparable_text(&asset.contents) {
                    report.unchanged_count += 1;
                    continue;
                }

                write_file(&destination, &asset.contents)?;
                report.updated.push(asset.path);
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                write_file(&destination, &asset.contents)?;
                report.created.push(asset.path);
            }
            Err(source) => {
                return Err(UpdateCommandError::ReadFile {
                    path: destination,
                    source,
                });
            }
        }
    }

    Ok(report)
}

fn write_file(path: &Path, contents: &str) -> Result<(), UpdateCommandError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| UpdateCommandError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    fs::write(path, contents).map_err(|source| UpdateCommandError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

fn detect_target_repo_markers(repo_root: &Path) -> TargetRepoMarkers {
    TargetRepoMarkers {
        has_workflow: repo_root.join("WORKFLOW.md").is_file(),
        has_config: repo_root.join("config.yaml").is_file(),
    }
}

fn comparable_text(value: &str) -> String {
    value.replace("\r\n", "\n").trim_end().to_owned()
}

fn print_paths(label: &str, paths: &[String]) {
    if paths.is_empty() {
        return;
    }

    println!("{label}:");
    for path in paths {
        println!("- {path}");
    }
}

fn print_memory_init_summary(target_repo: &Path, report: &MemoryInitApplyReport) {
    let (created, updated, unchanged) = memory_init_change_lists(report, target_repo);

    println!("Memory init summary:");
    print_paths("Created", &created);
    print_paths("Updated", &updated);
    println!("Unchanged: {} file(s)", unchanged.len());
}

fn join_for_display(items: &[&str]) -> String {
    match items {
        [] => "nothing".to_string(),
        [only] => format!("`{only}`"),
        [first, second] => format!("`{first}` and `{second}`"),
        _ => {
            let mut formatted = items
                .iter()
                .map(|item| format!("`{item}`"))
                .collect::<Vec<_>>();
            let last = formatted.pop().expect("there should be at least one item");
            format!("{}, and {}", formatted.join(", "), last)
        }
    }
}

fn compare_versions(current: &str, latest: &str) -> Option<Ordering> {
    if current == latest {
        return Some(Ordering::Equal);
    }

    let current = parse_version_components(current)?;
    let latest = parse_version_components(latest)?;
    Some(compare_components(&current, &latest))
}

fn parse_version_components(version: &str) -> Option<Vec<u64>> {
    let core = version
        .split_once('+')
        .map(|(core, _)| core)
        .unwrap_or(version);
    let core = core.split_once('-').map(|(core, _)| core).unwrap_or(core);

    if core.trim().is_empty() {
        return None;
    }

    core.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()
}

fn compare_components(current: &[u64], latest: &[u64]) -> Ordering {
    let max_len = current.len().max(latest.len());
    for index in 0..max_len {
        let left = current.get(index).copied().unwrap_or_default();
        let right = latest.get(index).copied().unwrap_or_default();
        match left.cmp(&right) {
            Ordering::Equal => continue,
            non_equal => return non_equal,
        }
    }

    Ordering::Equal
}

fn render_exit_status(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "termination by signal".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use crate::opensymphony_cli::init_repo::{ReviewProviderArg, TargetBranch};

    use super::{
        TargetRepoMarkers, compare_components, compare_versions, join_for_display,
        parse_version_components, patch_workflow_settings,
    };

    #[test]
    fn compare_versions_handles_equal_older_and_newer_releases() {
        assert_eq!(compare_versions("1.2.2", "1.2.2"), Some(Ordering::Equal));
        assert_eq!(compare_versions("1.2.2", "1.2.3"), Some(Ordering::Less));
        assert_eq!(compare_versions("1.3.0", "1.2.9"), Some(Ordering::Greater));
    }

    #[test]
    fn compare_versions_ignores_semver_suffixes() {
        assert_eq!(
            compare_versions("1.2.3-dev.1", "1.2.3"),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare_versions("1.2.3+build.5", "1.2.4"),
            Some(Ordering::Less)
        );
    }

    #[test]
    fn compare_versions_returns_none_for_non_numeric_versions() {
        assert_eq!(compare_versions("main", "1.2.3"), None);
    }

    #[test]
    fn parse_version_components_splits_numeric_core() {
        assert_eq!(
            parse_version_components("1.2.3-dev+build"),
            Some(vec![1, 2, 3])
        );
    }

    #[test]
    fn compare_components_pads_shorter_versions_with_zeroes() {
        assert_eq!(compare_components(&[1, 2], &[1, 2, 0]), Ordering::Equal);
        assert_eq!(compare_components(&[1, 2, 1], &[1, 2]), Ordering::Greater);
    }

    #[test]
    fn join_for_display_formats_one_or_two_items_cleanly() {
        assert_eq!(join_for_display(&["WORKFLOW.md"]), "`WORKFLOW.md`");
        assert_eq!(
            join_for_display(&["WORKFLOW.md", "config.yaml"]),
            "`WORKFLOW.md` and `config.yaml`"
        );
    }

    #[test]
    fn target_repo_markers_require_both_workflow_and_config() {
        assert!(
            TargetRepoMarkers {
                has_workflow: true,
                has_config: true
            }
            .looks_like_target_repo()
        );
        assert!(
            !TargetRepoMarkers {
                has_workflow: true,
                has_config: false
            }
            .looks_like_target_repo()
        );
    }

    #[test]
    fn patch_workflow_settings_updates_existing_markers_and_legacy_branch_text() {
        let target_branch = TargetBranch::parse("release/next").expect("branch should parse");
        let workflow = r#"## Branch target

Target branch: `main`

Keep feature branches current with `origin/main`.
Run the pull skill to sync with latest `origin/main` before code edits.
Do not delete `origin/main`.
Leave https://github.com/origin/main.git alone.

## Automated AI PR review

Active review provider: `openhands`
"#;

        let patched = patch_workflow_settings(
            workflow,
            Some(&target_branch),
            Some(ReviewProviderArg::Codex),
        )
        .expect("workflow should patch");

        assert!(patched.contains("Target branch: `release/next`"));
        assert!(patched.contains("Active review provider: `codex`"));
        assert!(patched.contains("Keep feature branches current with `origin/release/next`."));
        assert!(patched.contains("sync with latest `origin/release/next` before"));
        assert!(patched.contains("Do not delete `origin/main`."));
        assert!(patched.contains("https://github.com/origin/main.git"));
    }

    #[test]
    fn patch_workflow_settings_updates_previous_branch_control_text() {
        let target_branch = TargetBranch::parse("release/next").expect("branch should parse");
        let workflow = r#"## Branch target

Target branch: `develop`

Keep feature branches current with `origin/develop`.
Run the pull skill to sync with latest origin/develop before code edits.
Run the pull skill to sync with latest `origin/develop` before code edits.
Do not delete `origin/develop`.

## Automated AI PR review

Active review provider: `codex`
"#;

        let patched = patch_workflow_settings(workflow, Some(&target_branch), None)
            .expect("workflow should patch");

        assert!(patched.contains("Target branch: `release/next`"));
        assert!(patched.contains("Keep feature branches current with `origin/release/next`."));
        assert!(patched.contains("sync with latest origin/release/next before"));
        assert!(patched.contains("sync with latest `origin/release/next` before"));
        assert!(patched.contains("Do not delete `origin/develop`."));
    }

    #[test]
    fn patch_workflow_settings_inserts_missing_managed_markers() {
        let target_branch = TargetBranch::parse("develop").expect("branch should parse");
        let workflow = "# Existing workflow\n\nKeep this prose.\n";

        let patched = patch_workflow_settings(
            workflow,
            Some(&target_branch),
            Some(ReviewProviderArg::None),
        )
        .expect("workflow should patch");

        assert!(patched.contains("## Branch target"));
        assert!(patched.contains("Target branch: `develop`"));
        assert!(patched.contains("## Automated AI PR review"));
        assert!(patched.contains("Active review provider: `none`"));
        assert!(patched.contains("Keep this prose."));
    }

    #[test]
    fn patch_workflow_settings_rejects_malformed_marker() {
        let target_branch = TargetBranch::parse("develop").expect("branch should parse");
        let error = patch_workflow_settings(
            "## Branch target\n\nTarget branch: develop\n",
            Some(&target_branch),
            None,
        )
        .expect_err("malformed marker should fail");

        assert!(
            error
                .to_string()
                .contains("malformed managed `Target branch:` marker"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn patch_workflow_settings_uses_provider_example_for_malformed_provider_marker() {
        let error = patch_workflow_settings(
            "## Automated AI PR review\n\nActive review provider: codex\n",
            None,
            Some(ReviewProviderArg::Openhands),
        )
        .expect_err("malformed provider marker should fail");
        let error = error.to_string();

        assert!(
            error.contains("expected `Active review provider: `codex``"),
            "unexpected error: {error}"
        );
        assert!(
            !error.contains("Active review provider: `develop`"),
            "provider marker error should not show branch examples: {error}"
        );
    }

    #[test]
    fn patch_workflow_settings_accepts_trailing_marker_whitespace() {
        let target_branch = TargetBranch::parse("develop").expect("branch should parse");
        let workflow = "## Branch target\n\nTarget branch: `main`  \n\n## Automated AI PR review\n\nActive review provider: `openhands`\t\n";

        let patched = patch_workflow_settings(
            workflow,
            Some(&target_branch),
            Some(ReviewProviderArg::Codex),
        )
        .expect("trailing whitespace should be tolerated");

        assert!(patched.contains("Target branch: `develop`"));
        assert!(patched.contains("Active review provider: `codex`"));
    }

    #[test]
    fn patch_workflow_settings_rejects_duplicate_marker_in_managed_section() {
        let target_branch = TargetBranch::parse("develop").expect("branch should parse");
        let error = patch_workflow_settings(
            "## Branch target\n\nTarget branch: `main`\nTarget branch: `release`\n",
            Some(&target_branch),
            None,
        )
        .expect_err("duplicate marker should fail");

        assert!(
            error
                .to_string()
                .contains("multiple managed `Target branch:` markers"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn patch_workflow_settings_ignores_marker_like_prose_outside_managed_slot() {
        let target_branch = TargetBranch::parse("develop").expect("branch should parse");
        let workflow = r#"# Notes

Target branch: `example`
Active review provider: `example`

## Branch target

Target branch: `main`

## Automated AI PR review

Active review provider: `openhands`
"#;

        let patched = patch_workflow_settings(
            workflow,
            Some(&target_branch),
            Some(ReviewProviderArg::Codex),
        )
        .expect("workflow should patch");

        assert_eq!(patched.matches("Target branch: `example`").count(), 1);
        assert_eq!(
            patched.matches("Active review provider: `example`").count(),
            1
        );
        assert_eq!(patched.matches("Target branch: `develop`").count(), 1);
        assert_eq!(
            patched.matches("Active review provider: `codex`").count(),
            1
        );
    }
}
