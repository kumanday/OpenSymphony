use std::{
    cmp::Ordering,
    env, fs, io,
    path::{Path, PathBuf},
    process::{ExitCode, Stdio},
};

use clap::Args;
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

use crate::opensymphony_cli::init_repo::{self, InitCommandError};

const DEFAULT_CRATE_METADATA_URL: &str = "https://crates.io/api/v1/crates/opensymphony";

#[derive(Debug, Args, Clone)]
pub struct UpdateArgs {}

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
    let _ = args;

    let current_dir = env::current_dir().map_err(UpdateCommandError::CurrentDir)?;
    println!("Updating OpenSymphony from {}", current_dir.display());

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

    println!("Skill refresh summary:");
    print_paths("Created", &report.created);
    print_paths("Updated", &report.updated);
    println!("Unchanged: {} file(s)", report.unchanged_count);
    println!("OpenSymphony update complete.");
    Ok(())
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

    use super::{
        TargetRepoMarkers, compare_components, compare_versions, join_for_display,
        parse_version_components,
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
}
