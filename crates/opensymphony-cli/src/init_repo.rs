use std::{
    env, fs,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Args;
use reqwest::{Client, StatusCode, Url};
use thiserror::Error;

const DEFAULT_TEMPLATE_BASE_URL: &str =
    "https://raw.githubusercontent.com/kumanday/OpenSymphony-template/refs/heads/main/";
const DEFAULT_LLM_MODEL: &str = "openai/accounts/fireworks/models/glm-5p1";
const DEFAULT_LLM_BASE_URL: &str = "https://api.fireworks.ai/inference/v1";
const PRESERVED_AGENTS_MARKER: &str = "## Preserved Existing AGENTS.md";
const WORKFLOW_PROJECT_SLUG_PLACEHOLDER: &str = "\"YOUR-PROJECT-SLUG\"";
const WORKFLOW_GIT_REMOTE_PLACEHOLDER: &str = "https://github.com/YOUR-ORG/YOUR-REPO.git";

#[derive(Debug, Args, Clone)]
pub struct InitArgs {}

#[derive(Debug, Error)]
enum InitCommandError {
    #[error("failed to determine the current working directory: {0}")]
    CurrentDir(#[source] io::Error),
    #[error("failed to build the template fetch client: {0}")]
    HttpClient(#[source] reqwest::Error),
    #[error("invalid template base URL `{value}`: {source}")]
    InvalidTemplateBaseUrl {
        value: String,
        #[source]
        source: url::ParseError,
    },
    #[error("failed to fetch template asset {path} from {url}: {source}")]
    FetchTemplate {
        path: &'static str,
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("failed to fetch template asset {path} from {url}: HTTP {status}")]
    FetchTemplateStatus {
        path: &'static str,
        url: String,
        status: StatusCode,
    },
    #[error("template asset {path} from {url} was not valid UTF-8: {source}")]
    DecodeTemplate {
        path: &'static str,
        url: String,
        #[source]
        source: reqwest::Error,
    },
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
    #[error("failed to read interactive input: {0}")]
    PromptIo(#[source] io::Error),
    #[error("input closed while waiting for a response")]
    PromptClosed,
    #[error("initialization aborted")]
    AbortedByUser,
}

#[derive(Clone, Copy)]
struct TemplateAsset {
    path: &'static str,
    kind: AssetKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    Standard,
    Agents,
    Workflow,
}

#[derive(Clone)]
struct FetchedAsset {
    definition: TemplateAsset,
    contents: String,
}

enum PlannedAction {
    Create,
    Prompt,
    Overwrite,
    Skip,
    Unchanged,
    MergeAgents,
    CustomizeWorkflow,
}

struct PlannedAsset {
    asset: FetchedAsset,
    existing: Option<String>,
    action: PlannedAction,
}

enum AppliedChange {
    Created,
    Overwritten,
    Updated,
    Merged,
    Skipped,
    Unchanged,
}

enum GitRemoteDetection {
    Selected { remote_name: String, url: String },
    None,
    Ambiguous(Vec<String>),
}

trait EnvLookup {
    fn get(&self, name: &str) -> Option<String>;
}

struct ProcessEnvironment;

impl EnvLookup for ProcessEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }
}

struct PromptUi<R, W> {
    reader: R,
    writer: W,
}

impl<R, W> PromptUi<R, W>
where
    R: BufRead,
    W: Write,
{
    fn new(reader: R, writer: W) -> Self {
        Self { reader, writer }
    }

    fn line(&mut self, message: impl AsRef<str>) -> Result<(), InitCommandError> {
        writeln!(self.writer, "{}", message.as_ref()).map_err(InitCommandError::PromptIo)
    }

    fn blank_line(&mut self) -> Result<(), InitCommandError> {
        writeln!(self.writer).map_err(InitCommandError::PromptIo)
    }

    fn prompt(&mut self, prompt: &str) -> Result<String, InitCommandError> {
        write!(self.writer, "{prompt}").map_err(InitCommandError::PromptIo)?;
        self.writer.flush().map_err(InitCommandError::PromptIo)?;

        let mut response = String::new();
        let bytes = self
            .reader
            .read_line(&mut response)
            .map_err(InitCommandError::PromptIo)?;
        if bytes == 0 {
            return Err(InitCommandError::PromptClosed);
        }

        while response.ends_with('\n') || response.ends_with('\r') {
            response.pop();
        }
        Ok(response)
    }
}

const TEMPLATE_ASSETS: &[TemplateAsset] = &[
    TemplateAsset {
        path: "WORKFLOW.md",
        kind: AssetKind::Workflow,
    },
    TemplateAsset {
        path: "AGENTS.md",
        kind: AssetKind::Agents,
    },
    TemplateAsset {
        path: "config.yaml",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".gitignore",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/commit/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/convert-tasks-to-linear/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/create-implementation-plan/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/land/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/linear/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/pull/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".agents/skills/push/SKILL.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".github/CODEOWNERS",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".github/pull_request_template.md",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: ".github/workflows/ai-pr-review.yml",
        kind: AssetKind::Standard,
    },
    TemplateAsset {
        path: "docs/tasks/README.md",
        kind: AssetKind::Standard,
    },
];

pub async fn run_command(args: InitArgs) -> ExitCode {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut ui = PromptUi::new(stdin.lock(), stdout.lock());

    match run_init(args, &ProcessEnvironment, &mut ui).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = ui.blank_line();
            let _ = ui.line(format!("opensymphony init failed: {error}"));
            ExitCode::from(1)
        }
    }
}

async fn run_init<R, W, E>(
    args: InitArgs,
    env_lookup: &E,
    ui: &mut PromptUi<R, W>,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    let _ = args;

    let target_repo = env::current_dir().map_err(InitCommandError::CurrentDir)?;
    let client = Client::builder()
        .user_agent(concat!("opensymphony-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(InitCommandError::HttpClient)?;

    ui.line(format!(
        "Initializing OpenSymphony files in {}",
        target_repo.display()
    ))?;
    ui.line("Fetching the current template payload from GitHub...")?;

    let fetched_assets = fetch_template_assets(&client).await?;
    let mut planned_assets = plan_assets(&target_repo, fetched_assets)?;
    resolve_conflicts(&mut planned_assets, ui)?;

    let workflow_will_change = planned_assets.iter().any(|planned| {
        planned.asset.definition.kind == AssetKind::Workflow
            && matches!(
                planned.action,
                PlannedAction::Create | PlannedAction::Overwrite | PlannedAction::CustomizeWorkflow
            )
    });

    let git_remote = detect_git_remote_url(&target_repo);
    match &git_remote {
        GitRemoteDetection::Selected { remote_name, url } => {
            ui.line(format!(
                "Detected git remote `{remote_name}` -> {url}; `WORKFLOW.md` will use it for the clone hook."
            ))?;
        }
        GitRemoteDetection::None => {
            ui.line(
                "No git remote URL detected; `WORKFLOW.md` will keep its clone URL placeholder.",
            )?;
        }
        GitRemoteDetection::Ambiguous(remotes) => {
            ui.line(format!(
                "Found multiple git remotes without `origin` ({}); `WORKFLOW.md` will keep its clone URL placeholder.",
                remotes.join(", ")
            ))?;
        }
    }

    let linear_project_slug = if workflow_will_change {
        let response =
            ui.prompt("Enter your Linear project slug/key (leave blank to set it later): ")?;
        let response = response.trim();
        (!response.is_empty()).then(|| response.to_owned())
    } else {
        None
    };

    let mut created = Vec::new();
    let mut overwritten = Vec::new();
    let mut updated = Vec::new();
    let mut merged = Vec::new();
    let mut skipped = Vec::new();
    let mut unchanged = Vec::new();
    let mut wrote_config = false;

    for planned in planned_assets {
        let destination = target_repo.join(planned.asset.definition.path);
        let relative_path = planned.asset.definition.path.to_owned();

        let final_result = apply_asset(
            &destination,
            planned,
            git_remote_url(&git_remote),
            linear_project_slug.as_deref(),
        )?;

        match final_result {
            AppliedChange::Created => {
                if relative_path == "config.yaml" {
                    wrote_config = true;
                }
                created.push(relative_path);
            }
            AppliedChange::Overwritten => {
                if relative_path == "config.yaml" {
                    wrote_config = true;
                }
                overwritten.push(relative_path);
            }
            AppliedChange::Updated => {
                if relative_path == "config.yaml" {
                    wrote_config = true;
                }
                updated.push(relative_path);
            }
            AppliedChange::Merged => merged.push(relative_path),
            AppliedChange::Skipped => skipped.push(relative_path),
            AppliedChange::Unchanged => unchanged.push(relative_path),
        }
    }

    ui.blank_line()?;
    ui.line("Initialization summary:")?;
    print_group(ui, "Created", &created)?;
    print_group(ui, "Overwritten", &overwritten)?;
    print_group(ui, "Updated", &updated)?;
    print_group(ui, "Merged", &merged)?;
    print_group(ui, "Skipped", &skipped)?;
    print_group(ui, "Unchanged", &unchanged)?;

    if wrote_config {
        ui.blank_line()?;
        ui.line(
            "Review `config.yaml` and update `openhands.tool_dir` if the copied template path does not match your machine.",
        )?;
    }

    prompt_for_missing_llm_env(env_lookup, ui)?;

    ui.blank_line()?;
    ui.line("OpenSymphony init complete.")?;
    Ok(())
}

async fn fetch_template_assets(client: &Client) -> Result<Vec<FetchedAsset>, InitCommandError> {
    let base_url = env::var("OPENSYMPHONY_TEMPLATE_BASE_URL")
        .unwrap_or_else(|_| DEFAULT_TEMPLATE_BASE_URL.to_string());
    let base_url =
        Url::parse(&base_url).map_err(|source| InitCommandError::InvalidTemplateBaseUrl {
            value: base_url.clone(),
            source,
        })?;

    let mut fetched = Vec::with_capacity(TEMPLATE_ASSETS.len());
    for definition in TEMPLATE_ASSETS {
        let url = base_url.join(definition.path).map_err(|source| {
            InitCommandError::InvalidTemplateBaseUrl {
                value: format!("{base_url}{}", definition.path),
                source,
            }
        })?;
        let response = client.get(url.clone()).send().await.map_err(|source| {
            InitCommandError::FetchTemplate {
                path: definition.path,
                url: url.to_string(),
                source,
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            return Err(InitCommandError::FetchTemplateStatus {
                path: definition.path,
                url: url.to_string(),
                status,
            });
        }

        let contents =
            response
                .text()
                .await
                .map_err(|source| InitCommandError::DecodeTemplate {
                    path: definition.path,
                    url: url.to_string(),
                    source,
                })?;

        fetched.push(FetchedAsset {
            definition: *definition,
            contents,
        });
    }

    Ok(fetched)
}

fn plan_assets(
    target_repo: &Path,
    assets: Vec<FetchedAsset>,
) -> Result<Vec<PlannedAsset>, InitCommandError> {
    let mut planned = Vec::with_capacity(assets.len());

    for asset in assets {
        let destination = target_repo.join(asset.definition.path);
        match fs::read_to_string(&destination) {
            Ok(existing) => {
                let action = match asset.definition.kind {
                    AssetKind::Agents => {
                        if comparable_text(&existing) == comparable_text(&asset.contents)
                            || agents_already_initialized(&existing, &asset.contents)
                        {
                            PlannedAction::Unchanged
                        } else {
                            PlannedAction::MergeAgents
                        }
                    }
                    AssetKind::Workflow => {
                        if comparable_text(&existing) == comparable_text(&asset.contents) {
                            PlannedAction::CustomizeWorkflow
                        } else {
                            PlannedAction::Prompt
                        }
                    }
                    AssetKind::Standard => {
                        if comparable_text(&existing) == comparable_text(&asset.contents) {
                            PlannedAction::Unchanged
                        } else {
                            PlannedAction::Prompt
                        }
                    }
                };

                planned.push(PlannedAsset {
                    asset,
                    existing: Some(existing),
                    action,
                });
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                planned.push(PlannedAsset {
                    asset,
                    existing: None,
                    action: PlannedAction::Create,
                });
            }
            Err(source) => {
                return Err(InitCommandError::ReadFile {
                    path: destination,
                    source,
                });
            }
        }
    }

    Ok(planned)
}

fn resolve_conflicts<R, W>(
    planned_assets: &mut [PlannedAsset],
    ui: &mut PromptUi<R, W>,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    for planned in planned_assets {
        if !matches!(planned.action, PlannedAction::Prompt) {
            continue;
        }

        let relative_path = Path::new(planned.asset.definition.path);
        let display_path = relative_path.display();

        loop {
            ui.blank_line()?;
            ui.line(format!("`{display_path}` already exists."))?;
            let response = ui.prompt("Choose [s]kip, [o]verwrite, or [a]bort: ")?;
            match response.trim().to_ascii_lowercase().as_str() {
                "s" | "skip" => {
                    planned.action = PlannedAction::Skip;
                    break;
                }
                "o" | "overwrite" => {
                    planned.action = PlannedAction::Overwrite;
                    break;
                }
                "a" | "abort" => return Err(InitCommandError::AbortedByUser),
                _ => {
                    ui.line("Please answer with `skip`, `overwrite`, or `abort`.")?;
                }
            }
        }
    }

    Ok(())
}

fn apply_asset(
    destination: &Path,
    planned: PlannedAsset,
    git_remote_url: Option<&str>,
    linear_project_slug: Option<&str>,
) -> Result<AppliedChange, InitCommandError> {
    let existing = planned.existing.as_deref();

    let Some(final_contents) = build_final_contents(
        &planned.asset,
        &planned.action,
        existing,
        git_remote_url,
        linear_project_slug,
    ) else {
        return Ok(match planned.action {
            PlannedAction::Skip => AppliedChange::Skipped,
            PlannedAction::Unchanged => AppliedChange::Unchanged,
            PlannedAction::Create
            | PlannedAction::Overwrite
            | PlannedAction::MergeAgents
            | PlannedAction::CustomizeWorkflow
            | PlannedAction::Prompt => AppliedChange::Unchanged,
        });
    };

    if let Some(existing) = existing
        && comparable_text(existing) == comparable_text(&final_contents)
    {
        return Ok(AppliedChange::Unchanged);
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|source| InitCommandError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(destination, final_contents).map_err(|source| InitCommandError::WriteFile {
        path: destination.to_path_buf(),
        source,
    })?;

    Ok(match planned.action {
        PlannedAction::Create => AppliedChange::Created,
        PlannedAction::Overwrite => AppliedChange::Overwritten,
        PlannedAction::CustomizeWorkflow => AppliedChange::Updated,
        PlannedAction::MergeAgents => AppliedChange::Merged,
        PlannedAction::Skip => AppliedChange::Skipped,
        PlannedAction::Unchanged => AppliedChange::Unchanged,
        PlannedAction::Prompt => unreachable!("conflicts should be resolved before apply"),
    })
}

fn build_final_contents(
    asset: &FetchedAsset,
    action: &PlannedAction,
    existing: Option<&str>,
    git_remote_url: Option<&str>,
    linear_project_slug: Option<&str>,
) -> Option<String> {
    match action {
        PlannedAction::Create | PlannedAction::Overwrite => Some(match asset.definition.kind {
            AssetKind::Workflow => {
                customize_workflow(&asset.contents, git_remote_url, linear_project_slug)
            }
            _ => asset.contents.clone(),
        }),
        PlannedAction::CustomizeWorkflow => Some(customize_workflow(
            &asset.contents,
            git_remote_url,
            linear_project_slug,
        )),
        PlannedAction::MergeAgents => Some(match existing {
            Some(existing) if !existing.trim().is_empty() => {
                merge_agents(&asset.contents, existing)
            }
            _ => asset.contents.clone(),
        }),
        PlannedAction::Skip | PlannedAction::Unchanged => None,
        PlannedAction::Prompt => None,
    }
}

fn prompt_for_missing_llm_env<R, W, E>(
    env_lookup: &E,
    ui: &mut PromptUi<R, W>,
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
    E: EnvLookup,
{
    let mut exports = Vec::new();

    if env_lookup.get("LLM_MODEL").is_none() {
        let response = ui.prompt(&format!(
            "LLM_MODEL is not set. Enter a model now, or press Enter to use `{DEFAULT_LLM_MODEL}`: "
        ))?;
        let value = match response.trim() {
            "" => DEFAULT_LLM_MODEL.to_string(),
            custom => custom.to_string(),
        };
        exports.push(("LLM_MODEL", value));
    }

    if env_lookup.get("LLM_API_KEY").is_none() {
        let response = ui.prompt(
            "LLM_API_KEY is not set. Press Enter to use the placeholder `<your-llm-api-key>` in the export snippet, or type a different placeholder label: ",
        )?;
        let value = match response.trim() {
            "" => "<your-llm-api-key>".to_string(),
            custom => custom.to_string(),
        };
        exports.push(("LLM_API_KEY", value));
    }

    if env_lookup.get("LLM_BASE_URL").is_none() {
        let response = ui.prompt(&format!(
            "LLM_BASE_URL is not set. Enter a base URL now, or press Enter to use `{DEFAULT_LLM_BASE_URL}`: "
        ))?;
        let value = match response.trim() {
            "" => DEFAULT_LLM_BASE_URL.to_string(),
            custom => custom.to_string(),
        };
        exports.push(("LLM_BASE_URL", value));
    }

    if exports.is_empty() {
        return Ok(());
    }

    ui.blank_line()?;
    ui.line("Before `opensymphony run`, export these in your shell:")?;
    for (name, value) in exports {
        ui.line(format!("export {name}={}", shell_single_quote(&value)))?;
    }
    Ok(())
}

fn detect_git_remote_url(target_repo: &Path) -> GitRemoteDetection {
    let output = std::process::Command::new("git")
        .args(["remote"])
        .current_dir(target_repo)
        .output();
    let Ok(output) = output else {
        return GitRemoteDetection::None;
    };
    if !output.status.success() {
        return GitRemoteDetection::None;
    }

    let remotes = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let Some(remote_name) = select_remote_name(&remotes) else {
        return if remotes.len() > 1 {
            GitRemoteDetection::Ambiguous(remotes)
        } else {
            GitRemoteDetection::None
        };
    };

    let get_url = std::process::Command::new("git")
        .args(["remote", "get-url", &remote_name])
        .current_dir(target_repo)
        .output();
    let Ok(get_url) = get_url else {
        return GitRemoteDetection::None;
    };
    if !get_url.status.success() {
        return GitRemoteDetection::None;
    }

    let url = String::from_utf8_lossy(&get_url.stdout).trim().to_owned();
    if url.is_empty() {
        GitRemoteDetection::None
    } else {
        GitRemoteDetection::Selected { remote_name, url }
    }
}

fn select_remote_name(remotes: &[String]) -> Option<String> {
    if remotes.iter().any(|remote| remote == "origin") {
        Some("origin".to_string())
    } else if remotes.len() == 1 {
        remotes.first().cloned()
    } else {
        None
    }
}

fn git_remote_url(detection: &GitRemoteDetection) -> Option<&str> {
    match detection {
        GitRemoteDetection::Selected { url, .. } => Some(url.as_str()),
        GitRemoteDetection::None | GitRemoteDetection::Ambiguous(_) => None,
    }
}

fn agents_already_initialized(existing: &str, template: &str) -> bool {
    comparable_text(existing).starts_with(&comparable_text(template))
        || existing.contains(PRESERVED_AGENTS_MARKER)
}

fn merge_agents(template: &str, existing: &str) -> String {
    let mut merged = template.trim_end().to_string();
    if existing.trim().is_empty() {
        merged.push('\n');
        return merged;
    }

    merged.push_str("\n\n");
    merged.push_str(PRESERVED_AGENTS_MARKER);
    merged.push_str("\n\n");
    merged.push_str(
        "The following content was preserved from the repository's previous `AGENTS.md` during `opensymphony init`.\n\n",
    );
    merged.push_str(existing.trim());
    merged.push('\n');
    merged
}

fn customize_workflow(
    template: &str,
    git_remote_url: Option<&str>,
    linear_project_slug: Option<&str>,
) -> String {
    let mut customized = template.to_string();

    if let Some(url) = git_remote_url {
        customized = customized.replace(WORKFLOW_GIT_REMOTE_PLACEHOLDER, &shell_single_quote(url));
    }

    if let Some(slug) = linear_project_slug
        .map(str::trim)
        .filter(|slug| !slug.is_empty())
    {
        customized =
            customized.replace(WORKFLOW_PROJECT_SLUG_PLACEHOLDER, &yaml_double_quote(slug));
    }

    customized
}

fn comparable_text(value: &str) -> String {
    value.replace("\r\n", "\n").trim_end().to_owned()
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn yaml_double_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn print_group<R, W>(
    ui: &mut PromptUi<R, W>,
    label: &str,
    items: &[String],
) -> Result<(), InitCommandError>
where
    R: BufRead,
    W: Write,
{
    if items.is_empty() {
        return Ok(());
    }

    ui.line(format!("{label}:"))?;
    for item in items {
        ui.line(format!("- {item}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        DEFAULT_LLM_BASE_URL, DEFAULT_LLM_MODEL, GitRemoteDetection, PRESERVED_AGENTS_MARKER,
        PromptUi, agents_already_initialized, comparable_text, customize_workflow, git_remote_url,
        merge_agents, prompt_for_missing_llm_env, select_remote_name, shell_single_quote,
    };

    struct StubEnvironment {
        values: BTreeMap<String, String>,
    }

    impl super::EnvLookup for StubEnvironment {
        fn get(&self, name: &str) -> Option<String> {
            self.values.get(name).cloned()
        }
    }

    #[test]
    fn customize_workflow_replaces_repo_url_and_project_slug() {
        let workflow = r#"---
tracker:
  project_slug: "YOUR-PROJECT-SLUG"
hooks:
  after_create: |
    git clone --depth 1 https://github.com/YOUR-ORG/YOUR-REPO.git .
---
"#;

        let customized = customize_workflow(
            workflow,
            Some("git@github.com:kumanday/demo.git"),
            Some("demo-project"),
        );

        assert!(customized.contains("project_slug: \"demo-project\""));
        assert!(customized.contains("git clone --depth 1 'git@github.com:kumanday/demo.git' ."));
    }

    #[test]
    fn customize_workflow_replaces_placeholders_without_exact_line_matching() {
        let workflow = r#"---
tracker:
  project_slug:    "YOUR-PROJECT-SLUG"
hooks:
  after_create: |
        if [ ! -d .git ]; then git clone --depth 1 https://github.com/YOUR-ORG/YOUR-REPO.git .; fi
---
"#;

        let customized = customize_workflow(
            workflow,
            Some("https://github.com/example/demo.git"),
            Some("demo-project"),
        );

        assert!(customized.contains("project_slug:    \"demo-project\""));
        assert!(customized.contains("git clone --depth 1 'https://github.com/example/demo.git' ."));
    }

    #[test]
    fn merge_agents_appends_existing_content_under_marker() {
        let template = "# AGENTS.md\n\nTemplate\n";
        let existing = "# Existing\n\nRules";

        let merged = merge_agents(template, existing);

        assert!(merged.starts_with("# AGENTS.md"));
        assert!(merged.contains(PRESERVED_AGENTS_MARKER));
        assert!(merged.contains("# Existing\n\nRules"));
    }

    #[test]
    fn agents_initialized_detection_handles_prefixed_template_content() {
        let template = "# AGENTS.md\n\nTemplate\n";
        let existing = format!("{template}\n\n## More Notes\n\nCustom");

        assert!(agents_already_initialized(&existing, template));
    }

    #[test]
    fn comparable_text_ignores_crlf_and_trailing_newlines() {
        assert_eq!(comparable_text("a\r\nb\r\n"), comparable_text("a\nb\n\n"));
    }

    #[test]
    fn select_remote_prefers_origin_then_single_remote() {
        assert_eq!(
            select_remote_name(&["fork".to_string(), "origin".to_string()]),
            Some("origin".to_string())
        );
        assert_eq!(
            select_remote_name(&["upstream".to_string()]),
            Some("upstream".to_string())
        );
        assert_eq!(
            select_remote_name(&["fork".to_string(), "upstream".to_string()]),
            None
        );
    }

    #[test]
    fn git_remote_url_returns_selected_only() {
        assert_eq!(
            git_remote_url(&GitRemoteDetection::Selected {
                remote_name: "origin".to_string(),
                url: "https://github.com/kumanday/OpenSymphony-template.git".to_string(),
            }),
            Some("https://github.com/kumanday/OpenSymphony-template.git")
        );
        assert_eq!(git_remote_url(&GitRemoteDetection::None), None);
        assert_eq!(
            git_remote_url(&GitRemoteDetection::Ambiguous(vec!["fork".to_string()])),
            None
        );
    }

    #[test]
    fn shell_single_quote_escapes_embedded_single_quotes() {
        assert_eq!(shell_single_quote("abc'def"), "'abc'\\''def'");
    }

    #[test]
    fn llm_defaults_match_fireworks_examples() {
        assert_eq!(
            DEFAULT_LLM_MODEL,
            "openai/accounts/fireworks/models/glm-5p1"
        );
        assert_eq!(
            DEFAULT_LLM_BASE_URL,
            "https://api.fireworks.ai/inference/v1"
        );
    }

    #[test]
    fn missing_llm_env_prompts_render_fireworks_defaults() {
        let env = StubEnvironment {
            values: BTreeMap::new(),
        };
        let mut output = Vec::new();
        let mut ui = PromptUi::new(&b"\n\n\n"[..], &mut output);

        prompt_for_missing_llm_env(&env, &mut ui).expect("prompt should succeed");

        let rendered = String::from_utf8(output).expect("prompt output should be utf-8");
        assert!(rendered.contains("LLM_MODEL is not set."));
        assert!(rendered.contains("openai/accounts/fireworks/models/glm-5p1"));
        assert!(rendered.contains("https://api.fireworks.ai/inference/v1"));
        assert!(rendered.contains("export LLM_API_KEY='<your-llm-api-key>'"));
    }
}
