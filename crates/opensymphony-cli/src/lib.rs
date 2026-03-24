mod debug_session;
mod orchestrator_run;

use std::{
    collections::BTreeSet,
    env,
    net::SocketAddr,
    num::NonZeroU64,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use chrono::{Duration as ChronoDuration, Utc};
use clap::{Args, Parser, Subcommand};
use opensymphony_control::{
    AgentServerStatus, ControlPlaneServer, DaemonSnapshot, DaemonState, DaemonStatus,
    IssueRuntimeState, IssueSnapshot, MetricsSnapshot, RecentEvent, RecentEventKind, SnapshotStore,
    WorkerOutcome,
};
use opensymphony_linear_mcp::run_stdio_server as run_linear_mcp_stdio_server;
use opensymphony_openhands::{
    ConversationCreateRequest, LocalServerSupervisor, LocalServerTooling, McpConfig,
    McpStdioServerConfig, OpenHandsClient, SupervisedServerConfig, SupervisorConfig,
    TransportConfig,
};
use opensymphony_workflow::{
    Environment, ProcessEnvironment, ResolvedWorkflow, WorkflowDefinition,
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use thiserror::Error;
use tokio::fs;
use tracing::info;
use tracing_subscriber::{EnvFilter, fmt};
use url::Url;

#[derive(Debug, Parser)]
#[command(name = "opensymphony")]
#[command(about = "Operate the OpenSymphony local MVP on a trusted machine")]
#[command(
    long_about = "Operate the OpenSymphony local MVP on a trusted machine.\n\nUse this CLI to run the orchestrator, local control-plane demos, preflight checks, and the Linear MCP bridge.\n\nSafety: local OpenSymphony runs agent activity on the host with process-level isolation only. It is not sandboxed."
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Run the real orchestrator against the current project workflow")]
    Run(orchestrator_run::RunArgs),
    #[command(about = "Resume an issue conversation for interactive debugging")]
    Debug(debug_session::DebugArgs),
    #[command(about = "Serve the local control-plane demo stream")]
    Daemon(DaemonArgs),
    #[command(about = "Attach the FrankenTUI operator client to a control plane")]
    Tui(TuiArgs),
    #[command(about = "Start the stdio Linear MCP server for agent-side writes")]
    LinearMcp(LinearMcpArgs),
    #[command(about = "Run local preflight checks for trusted-machine deployment")]
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
struct DaemonArgs {
    #[arg(help = "Socket address for the local control-plane HTTP and SSE server")]
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(help = "Milliseconds between sample snapshot updates")]
    #[arg(long, default_value = "1200")]
    sample_interval_ms: NonZeroU64,
}

#[derive(Debug, Args)]
struct TuiArgs {
    #[arg(help = "Control-plane base URL")]
    #[arg(long, default_value = "http://127.0.0.1:3000/")]
    url: Url,
    #[arg(help = "Exit after the specified number of milliseconds; useful for smoke tests")]
    #[arg(long)]
    exit_after_ms: Option<u64>,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(help = "Doctor config YAML path")]
    #[arg(long, default_value = "examples/configs/local-dev.yaml")]
    config: PathBuf,
    #[arg(help = "Run the live OpenHands probe instead of static preflight only")]
    #[arg(long)]
    live_openhands: bool,
}

#[derive(Debug, Args)]
pub struct LinearMcpArgs {}

#[derive(Debug, Deserialize)]
struct DoctorConfig {
    target_repo: Option<String>,
    openhands: OpenHandsDoctorConfig,
    #[serde(default)]
    linear: LinearDoctorConfig,
}

#[derive(Debug, Deserialize)]
struct OpenHandsDoctorConfig {
    tool_dir: String,
    probe_model: Option<String>,
    probe_api_key_env: Option<String>,
    probe_llm_base_url_env: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LinearDoctorConfig {
    #[serde(default)]
    enabled: bool,
}

struct DoctorRuntimeConfig {
    target_repo: PathBuf,
    workflow_path: PathBuf,
    workflow: ResolvedWorkflow,
    tool_dir: PathBuf,
    probe_model: Option<String>,
    probe_api_key_env: Option<String>,
    probe_llm_base_url_env: Option<String>,
}

struct DoctorWorkflowEnvironment {
    fallback_linear_api_key: bool,
}

impl Environment for DoctorWorkflowEnvironment {
    fn get(&self, name: &str) -> Option<String> {
        env::var_os(name)
            .map(|value| value.to_string_lossy().into_owned())
            .or_else(|| {
                if self.fallback_linear_api_key && name == "LINEAR_API_KEY" {
                    Some("doctor-linear-disabled-placeholder".to_string())
                } else {
                    None
                }
            })
    }
}

#[derive(Debug, Serialize)]
struct DoctorProbeIssue<'a> {
    identifier: &'a str,
    title: &'a str,
    state: &'a str,
    description: Option<&'a str>,
    priority: Option<u8>,
    labels: Vec<&'a str>,
}

#[derive(Debug, Error)]
enum ExpandEnvTokensError {
    #[error("missing environment variable(s): {vars}")]
    MissingVars { vars: String },
    #[error("unterminated environment token `${{{token}}}`")]
    UnterminatedToken { token: String },
}

#[derive(Debug, Error)]
enum ResolveDoctorConfigError {
    #[error("{field}: {source}")]
    Field {
        field: &'static str,
        #[source]
        source: ExpandEnvTokensError,
    },
}

#[derive(Clone, Copy)]
enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

struct CheckResult {
    status: CheckStatus,
    name: &'static str,
    detail: String,
}

impl CheckResult {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Pass,
            name,
            detail: detail.into(),
        }
    }

    fn warn(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Warn,
            name,
            detail: detail.into(),
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Fail,
            name,
            detail: detail.into(),
        }
    }

    fn skip(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Skip,
            name,
            detail: detail.into(),
        }
    }
}

struct ToolingInspection {
    tooling: Option<LocalServerTooling>,
    checks: Vec<CheckResult>,
}

pub async fn run() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Run(args) => orchestrator_run::run_command(args).await,
        Command::Debug(args) => debug_session::run_command(args).await,
        Command::Doctor(args) => run_doctor(args).await,
        Command::Daemon(args) => run_daemon(args).await,
        Command::Tui(args) => run_tui(args).await,
        Command::LinearMcp(args) => run_linear_mcp(args).await,
    }
}

async fn run_daemon(args: DaemonArgs) -> ExitCode {
    match run_daemon_command(args.bind, args.sample_interval_ms).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_daemon_command(
    bind: SocketAddr,
    sample_interval_ms: NonZeroU64,
) -> Result<(), CommandError> {
    let store = SnapshotStore::new(sample_snapshot(0));
    spawn_demo_updates(
        store.clone(),
        Duration::from_millis(sample_interval_ms.get()),
    );
    let server = ControlPlaneServer::new(store);
    let listener = tokio::net::TcpListener::bind(bind).await?;
    info!(%bind, "control plane listening");

    let server_task = tokio::spawn(async move { server.serve(listener).await });
    tokio::select! {
        result = server_task => {
            result.map_err(CommandError::Join)??;
            Ok(())
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down control plane");
            Ok(())
        }
    }
}

async fn run_tui(args: TuiArgs) -> ExitCode {
    match run_tui_command(args.url, args.exit_after_ms).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_tui_command(url: Url, exit_after_ms: Option<u64>) -> Result<(), CommandError> {
    let exit_after = exit_after_ms.map(Duration::from_millis);
    tokio::task::spawn_blocking(move || opensymphony_tui::run_operator(url, exit_after))
        .await
        .map_err(CommandError::Join)?
        .map_err(CommandError::Tui)
}

async fn run_doctor(args: DoctorArgs) -> ExitCode {
    run_doctor_command(args.config, args.live_openhands).await
}

async fn run_linear_mcp(args: LinearMcpArgs) -> ExitCode {
    let _ = args;

    match run_linear_mcp_stdio_server().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("failed to start Linear MCP server: {error}");
            ExitCode::from(1)
        }
    }
}

pub async fn run_doctor_command(config_path: PathBuf, live_openhands: bool) -> ExitCode {
    let mut checks = Vec::new();

    let config = match load_config(&config_path).await {
        Ok(config) => {
            checks.push(CheckResult::pass(
                "config",
                format!("parsed {}", config_path.display()),
            ));
            config
        }
        Err(error) => {
            checks.push(CheckResult::fail("config", error));
            print_checks(&checks);
            return ExitCode::from(1);
        }
    };

    let config_root = config_path.parent().unwrap_or_else(|| Path::new("."));
    let tool_dir = match resolve_required_path(
        config_root,
        "openhands.tool_dir",
        &config.openhands.tool_dir,
    ) {
        Ok(tool_dir) => tool_dir,
        Err(error) => {
            checks.push(CheckResult::fail("config", error.to_string()));
            print_checks(&checks);
            return ExitCode::from(1);
        }
    };
    let configured_target_repo = match config
        .target_repo
        .as_deref()
        .map(|target_repo| resolve_required_path(config_root, "target_repo", target_repo))
        .transpose()
    {
        Ok(target_repo) => target_repo,
        Err(error) => {
            checks.push(CheckResult::fail("config", error.to_string()));
            print_checks(&checks);
            return ExitCode::from(1);
        }
    };
    let repo_root =
        discover_checkout_root(config_root, configured_target_repo.as_deref(), &tool_dir)
            .or_else(|| {
                env::current_dir()
                    .ok()
                    .and_then(|cwd| find_cargo_workspace_root(&cwd).or(Some(cwd)))
            })
            .unwrap_or_else(|| PathBuf::from("."));
    let runtime = resolve_doctor_runtime(&config, config_root, &repo_root);

    checks.push(check_repo_root(&repo_root));

    let (runtime, rendered_probe_prompt) = match runtime {
        Ok(runtime) => {
            checks.push(check_target_repo(&runtime.target_repo).await);
            checks.push(check_workflow(&runtime));

            let rendered_probe_prompt = match render_doctor_probe_prompt(&runtime.workflow) {
                Ok(prompt) => {
                    checks.push(CheckResult::pass(
                        "workflow-prompt",
                        format!(
                            "rendered {} characters from {}",
                            prompt.len(),
                            runtime.workflow_path.display()
                        ),
                    ));
                    prompt
                }
                Err(error) => {
                    checks.push(CheckResult::fail("workflow-prompt", error));
                    print_checks(&checks);
                    return ExitCode::from(1);
                }
            };

            checks.push(check_workspace_root(&runtime.workflow.config.workspace.root).await);
            checks.push(check_tool_dir(&runtime.tool_dir).await);
            checks.extend(check_required_commands());
            checks.push(check_local_safety());
            checks.push(check_openhands_transport(&runtime.workflow));
            checks.push(check_linear(&config.linear, &runtime.workflow));
            (runtime, rendered_probe_prompt)
        }
        Err(error) => {
            let target_repo =
                configured_target_repo.unwrap_or_else(|| repo_root.join("examples/target-repo"));
            checks.push(check_target_repo(&target_repo).await);
            checks.push(CheckResult::fail("workflow", error));
            print_checks(&checks);
            return ExitCode::from(1);
        }
    };

    let tooling_inspection = inspect_local_tooling(&runtime.tool_dir);
    checks.extend(tooling_inspection.checks);

    if live_openhands {
        checks.extend(
            run_live_openhands_checks(
                &runtime,
                &rendered_probe_prompt,
                tooling_inspection.tooling.as_ref(),
            )
            .await,
        );
    } else {
        checks.push(CheckResult::skip(
            "openhands-live",
            "live OpenHands checks skipped; rerun with --live-openhands on a prepared machine",
        ));
    }

    print_checks(&checks);
    if checks
        .iter()
        .any(|check| matches!(check.status, CheckStatus::Fail))
    {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

async fn load_config(path: &Path) -> Result<DoctorConfig, String> {
    let raw = fs::read_to_string(path)
        .await
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let config = serde_yaml::from_str(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    resolve_doctor_config(config)
        .map_err(|error| format!("failed to expand {}: {error}", path.display()))
}

fn resolve_doctor_config(
    mut config: DoctorConfig,
) -> Result<DoctorConfig, ResolveDoctorConfigError> {
    if let Some(target_repo) = config.target_repo.take() {
        config.target_repo = Some(resolve_required_config_value("target_repo", &target_repo)?);
    }

    config.openhands.tool_dir =
        resolve_required_config_value("openhands.tool_dir", &config.openhands.tool_dir)?;

    let probe_model = config.openhands.probe_model.take();
    config.openhands.probe_model =
        resolve_optional_config_value("openhands.probe_model", probe_model.as_deref())?;

    let probe_api_key_env = config.openhands.probe_api_key_env.take();
    config.openhands.probe_api_key_env =
        resolve_optional_config_value("openhands.probe_api_key_env", probe_api_key_env.as_deref())?;

    Ok(config)
}

fn expand_env_tokens(input: &str) -> Result<String, ExpandEnvTokensError> {
    let mut expanded = String::new();
    let mut chars = input.chars().peekable();
    let mut missing = BTreeSet::new();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            let _ = chars.next();
            let mut key = String::new();
            let mut closed = false;
            for next in chars.by_ref() {
                if next == '}' {
                    closed = true;
                    break;
                }
                key.push(next);
            }
            if !closed {
                return Err(ExpandEnvTokensError::UnterminatedToken { token: key });
            }
            match env::var(&key) {
                Ok(value) => expanded.push_str(&value),
                Err(_) => {
                    missing.insert(key);
                }
            }
        } else {
            expanded.push(ch);
        }
    }

    if missing.is_empty() {
        Ok(expanded)
    } else {
        Err(ExpandEnvTokensError::MissingVars {
            vars: missing.into_iter().collect::<Vec<_>>().join(", "),
        })
    }
}

fn resolve_required_config_value(
    field: &'static str,
    raw: &str,
) -> Result<String, ResolveDoctorConfigError> {
    expand_env_tokens(raw).map_err(|source| ResolveDoctorConfigError::Field { field, source })
}

fn resolve_optional_config_value(
    field: &'static str,
    raw: Option<&str>,
) -> Result<Option<String>, ResolveDoctorConfigError> {
    let Some(raw) = raw else {
        return Ok(None);
    };

    match expand_env_tokens(raw) {
        Ok(value) => Ok(match value.trim() {
            "" => None,
            normalized => Some(normalized.to_owned()),
        }),
        Err(ExpandEnvTokensError::MissingVars { .. }) => Ok(None),
        Err(source) => Err(ResolveDoctorConfigError::Field { field, source }),
    }
}

fn resolve_path(base: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn resolve_required_path(
    base: &Path,
    field: &'static str,
    raw: &str,
) -> Result<PathBuf, ResolveDoctorConfigError> {
    resolve_required_config_value(field, raw).map(|value| resolve_path(base, &value))
}

fn discover_checkout_root(
    config_root: &Path,
    target_repo: Option<&Path>,
    tool_dir: &Path,
) -> Option<PathBuf> {
    [Some(config_root), target_repo, Some(tool_dir)]
        .into_iter()
        .flatten()
        .find_map(find_cargo_workspace_root)
}

fn find_cargo_workspace_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_file() { path.parent()? } else { path };
    start
        .ancestors()
        .find(|candidate| candidate.join("Cargo.toml").is_file())
        .map(normalize_workspace_root)
}

fn normalize_workspace_root(path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() {
        env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    } else {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }
}

fn effective_openhands_probe_base_url(
    configured_base_url: &str,
    started_supervisor_base_url: Option<&str>,
) -> String {
    started_supervisor_base_url
        .unwrap_or(configured_base_url)
        .to_string()
}

fn build_doctor_transport(
    runtime: &DoctorRuntimeConfig,
    base_url_override: Option<String>,
) -> Result<TransportConfig, String> {
    let transport = TransportConfig::from_workflow(&runtime.workflow, &ProcessEnvironment)
        .map_err(|error| error.to_string())?;
    Ok(match base_url_override {
        Some(base_url) => TransportConfig::new(base_url).with_auth(transport.auth().clone()),
        None => transport,
    })
}

fn resolve_doctor_runtime(
    config: &DoctorConfig,
    config_root: &Path,
    repo_root: &Path,
) -> Result<DoctorRuntimeConfig, String> {
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|target_repo| resolve_path(config_root, target_repo))
        .unwrap_or_else(|| repo_root.join("examples/target-repo"));
    let workflow_path = target_repo.join("WORKFLOW.md");
    let workflow = WorkflowDefinition::load_from_path(&workflow_path)
        .map_err(|error| format!("failed to load {}: {error}", workflow_path.display()))?;
    let workflow = resolve_doctor_workflow(&workflow, &target_repo, config.linear.enabled)
        .map_err(|error| format!("failed to resolve {}: {error}", workflow_path.display()))?;

    Ok(DoctorRuntimeConfig {
        target_repo,
        workflow_path,
        workflow,
        tool_dir: resolve_path(config_root, &config.openhands.tool_dir),
        probe_model: config.openhands.probe_model.clone(),
        probe_api_key_env: config.openhands.probe_api_key_env.clone(),
        probe_llm_base_url_env: config.openhands.probe_llm_base_url_env.clone(),
    })
}

fn resolve_doctor_workflow(
    workflow: &WorkflowDefinition,
    target_repo: &Path,
    linear_enabled: bool,
) -> Result<ResolvedWorkflow, opensymphony_workflow::WorkflowConfigError> {
    if linear_enabled || workflow.front_matter.tracker.api_key.is_some() {
        workflow.resolve_with_process_env(target_repo)
    } else {
        workflow.resolve(
            target_repo,
            &DoctorWorkflowEnvironment {
                fallback_linear_api_key: true,
            },
        )
    }
}

fn render_doctor_probe_prompt(workflow: &ResolvedWorkflow) -> Result<String, String> {
    let issue = DoctorProbeIssue {
        identifier: "DOCTOR-PROBE",
        title: "Doctor workflow/runtime probe",
        state: "In Progress",
        description: Some(
            "Validate that the target repository workflow resolves and renders inside the doctor runtime path.",
        ),
        priority: Some(3),
        labels: vec!["doctor", "probe"],
    };

    workflow.render_prompt(&issue, None).map_err(|error| {
        format!(
            "failed to render the target repo workflow prompt with the doctor probe issue shape: {error}"
        )
    })
}

fn check_workflow(runtime: &DoctorRuntimeConfig) -> CheckResult {
    let linear_mode =
        if runtime.workflow.config.tracker.api_key == "doctor-linear-disabled-placeholder" {
            "tracker api_key fallback relaxed because `linear.enabled` is false"
        } else {
            "tracker auth resolved"
        };

    CheckResult::pass(
        "workflow",
        format!(
            "resolved {} -> workspace {}, OpenHands {}, project {}, {linear_mode}",
            runtime.workflow_path.display(),
            runtime.workflow.config.workspace.root.display(),
            runtime.workflow.extensions.openhands.transport.base_url,
            runtime.workflow.config.tracker.project_slug,
        ),
    )
}

fn check_repo_root(repo_root: &Path) -> CheckResult {
    if repo_root.join("Cargo.toml").exists() {
        CheckResult::pass(
            "repo",
            format!("found Cargo workspace at {}", repo_root.display()),
        )
    } else {
        CheckResult::fail(
            "repo",
            format!("missing Cargo.toml at {}", repo_root.display()),
        )
    }
}

async fn check_target_repo(target_repo: &Path) -> CheckResult {
    if !target_repo.exists() {
        return CheckResult::fail(
            "target-repo",
            format!("missing target repo {}", target_repo.display()),
        );
    }

    let workflow_path = target_repo.join("WORKFLOW.md");
    if workflow_path.exists() {
        CheckResult::pass("target-repo", format!("found {}", workflow_path.display()))
    } else {
        CheckResult::fail(
            "target-repo",
            format!("missing {}", workflow_path.display()),
        )
    }
}

async fn check_workspace_root(workspace_root: &Path) -> CheckResult {
    match fs::create_dir_all(workspace_root).await {
        Ok(()) => {
            if workspace_root.to_string_lossy().contains("/tmp")
                || workspace_root.to_string_lossy().contains("/Shared")
            {
                CheckResult::warn(
                    "workspace-root",
                    format!(
                        "workspace root {} is usable but looks shared",
                        workspace_root.display()
                    ),
                )
            } else {
                CheckResult::pass(
                    "workspace-root",
                    format!("ready at {}", workspace_root.display()),
                )
            }
        }
        Err(error) => CheckResult::fail(
            "workspace-root",
            format!("failed to create {}: {error}", workspace_root.display()),
        ),
    }
}

async fn check_tool_dir(tool_dir: &Path) -> CheckResult {
    let version = tool_dir.join("version.txt");
    let pyproject = tool_dir.join("pyproject.toml");
    let installer = tool_dir.join("install.sh");
    let runner = tool_dir.join("run-local.sh");

    if version.exists() && pyproject.exists() && installer.exists() && runner.exists() {
        CheckResult::pass(
            "openhands-tooling",
            format!(
                "pin files and helper scripts found in {}",
                tool_dir.display()
            ),
        )
    } else {
        CheckResult::fail(
            "openhands-tooling",
            format!(
                "expected {}, {}, {}, and {}",
                version.display(),
                pyproject.display(),
                installer.display(),
                runner.display()
            ),
        )
    }
}

fn check_required_commands() -> Vec<CheckResult> {
    [
        (
            "cargo",
            "Rust workspace builds, tests, and CLI smoke checks",
        ),
        ("curl", "local control-plane and agent-server smoke probes"),
        ("git", "workspace hooks and local repository operations"),
        (
            "uv",
            "the pinned OpenHands environment under tools/openhands-server",
        ),
    ]
    .into_iter()
    .map(|(name, purpose)| match find_executable(name) {
        Some(path) => CheckResult::pass(
            command_check_name(name),
            format!("found {} at {} ({purpose})", name, path.display()),
        ),
        None => CheckResult::fail(
            command_check_name(name),
            format!("{} is not on PATH ({purpose})", name),
        ),
    })
    .collect()
}

fn command_check_name(name: &'static str) -> &'static str {
    match name {
        "cargo" => "prereq-cargo",
        "curl" => "prereq-curl",
        "git" => "prereq-git",
        "uv" => "prereq-uv",
        _ => "prereq",
    }
}

fn find_executable(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let suffixes = executable_suffixes();

    for directory in env::split_paths(&path) {
        for suffix in &suffixes {
            let candidate = directory.join(format!("{name}{suffix}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn executable_suffixes() -> Vec<String> {
    if cfg!(windows) {
        env::var_os("PATHEXT")
            .map(|value| {
                env::split_paths(&value)
                    .map(|entry| entry.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
            .filter(|suffixes| !suffixes.is_empty())
            .unwrap_or_else(|| vec![".EXE".to_string(), ".BAT".to_string(), ".CMD".to_string()])
    } else {
        vec![String::new()]
    }
}

fn check_local_safety() -> CheckResult {
    CheckResult::warn(
        "local-safety",
        "trusted-machine mode only; agent runs with host filesystem and host process access, with process-level isolation but no sandbox boundary",
    )
}

fn inspect_local_tooling(tool_dir: &Path) -> ToolingInspection {
    match LocalServerTooling::load(tool_dir) {
        Ok(tooling) => {
            let mut checks = vec![
                CheckResult::pass(
                    "openhands-launcher",
                    format!("{} [{}]", tooling.metadata.launcher, tooling.base_url(None)),
                ),
                CheckResult::pass(
                    "openhands-version",
                    format!("version.txt pins {}", tooling.version),
                ),
            ];

            if tooling.pin_status.is_ready() {
                checks.push(CheckResult::pass(
                    "openhands-pin",
                    format!("{} matches pyproject.toml and uv.lock", tooling.version),
                ));
            } else {
                checks.push(CheckResult::fail(
                    "openhands-pin",
                    tooling.pin_status.blocking_issues().join("; "),
                ));
            }

            ToolingInspection {
                tooling: Some(tooling),
                checks,
            }
        }
        Err(error) => ToolingInspection {
            tooling: None,
            checks: vec![CheckResult::fail(
                "openhands-tooling-load",
                error.to_string(),
            )],
        },
    }
}

fn check_openhands_transport(workflow: &ResolvedWorkflow) -> CheckResult {
    let base_url = &workflow.extensions.openhands.transport.base_url;
    match Url::parse(base_url) {
        Ok(url) => {
            let host = url.host_str().unwrap_or("<missing-host>");
            let path = if url.path().trim_matches('/').is_empty() {
                "root path".to_string()
            } else {
                format!("path prefix {}", url.path())
            };
            let auth_detail = workflow
                .extensions
                .openhands
                .transport
                .session_api_key_env
                .as_deref()
                .map(|env_name| format!("auth env {env_name}"))
                .unwrap_or_else(|| "no session API key env".to_string());
            let remote_target = !matches!(host, "127.0.0.1" | "localhost");
            if remote_target {
                CheckResult::warn(
                    "bind-scope",
                    format!(
                        "OpenHands target {base_url} is not loopback; local trusted-machine mode treats it as an external trusted server ({path}, websocket auth {}, {auth_detail})",
                        workflow.extensions.openhands.websocket.auth_mode
                    ),
                )
            } else {
                CheckResult::pass(
                    "bind-scope",
                    format!("OpenHands loopback target {base_url} ({path}, {auth_detail})"),
                )
            }
        }
        Err(error) => CheckResult::fail(
            "bind-scope",
            format!("invalid base URL `{base_url}`: {error}"),
        ),
    }
}

fn check_linear(linear: &LinearDoctorConfig, workflow: &ResolvedWorkflow) -> CheckResult {
    if !linear.enabled {
        return CheckResult::skip(
            "linear",
            format!(
                "Linear checks skipped because `linear.enabled` is false; workflow tracker project {} still resolved",
                workflow.config.tracker.project_slug
            ),
        );
    }

    CheckResult::pass(
        "linear",
        format!(
            "workflow tracker ready for project {} with {} active and {} terminal states",
            workflow.config.tracker.project_slug,
            workflow.config.tracker.active_states.len(),
            workflow.config.tracker.terminal_states.len(),
        ),
    )
}

async fn run_live_openhands_checks(
    runtime: &DoctorRuntimeConfig,
    rendered_probe_prompt: &str,
    tooling: Option<&LocalServerTooling>,
) -> Vec<CheckResult> {
    let mut checks = Vec::new();
    let api_key = runtime
        .probe_api_key_env
        .as_ref()
        .and_then(|env_name| env::var(env_name).ok());
    let llm_base_url = runtime
        .probe_llm_base_url_env
        .as_ref()
        .and_then(|env_name| env::var(env_name).ok());

    if let Some(env_name) = &runtime.probe_api_key_env {
        if api_key.is_none() {
            checks.push(CheckResult::warn(
                "openhands-secret",
                format!(
                    "{} is not set; live probe will rely on server-side defaults",
                    env_name
                ),
            ));
        } else {
            checks.push(CheckResult::pass(
                "openhands-secret",
                format!("found {}", env_name),
            ));
        }
    }

    if let Some(env_name) = &runtime.probe_llm_base_url_env {
        if llm_base_url.is_none() {
            checks.push(CheckResult::warn(
                "openhands-llm-base-url",
                format!(
                    "{} is not set; live probe will rely on provider default endpoint",
                    env_name
                ),
            ));
        } else {
            checks.push(CheckResult::pass(
                "openhands-llm-base-url",
                format!("found {}", env_name),
            ));
        }
    }

    let base_url = &runtime.workflow.extensions.openhands.transport.base_url;
    let mut managed_supervisor = None;
    let mut probe_base_url = base_url.clone();
    let mut http_detail = format!("{probe_base_url} responded to /openapi.json");
    let mut transport = match build_doctor_transport(runtime, None) {
        Ok(transport) => transport,
        Err(error) => {
            checks.push(CheckResult::fail("openhands-auth", error));
            return checks;
        }
    };
    let client = OpenHandsClient::new(transport.clone());
    if let Err(error) = client.openapi_probe().await {
        match maybe_start_local_supervisor(runtime, tooling, &transport) {
            Ok(Some(mut supervisor)) => {
                let started = match supervisor.status() {
                    Ok(status) => status,
                    Err(status_error) => {
                        checks.push(CheckResult::fail(
                            "openhands-supervisor-status",
                            status_error.to_string(),
                        ));
                        return stop_managed_supervisor(checks, Some(supervisor));
                    }
                };
                checks.push(CheckResult::pass(
                    "openhands-supervisor-start",
                    format!(
                        "started pid {} for {}",
                        started.pid.unwrap_or_default(),
                        started.base_url
                    ),
                ));
                probe_base_url =
                    effective_openhands_probe_base_url(base_url, Some(&started.base_url));
                managed_supervisor = Some(supervisor);
                transport = match build_doctor_transport(runtime, Some(probe_base_url.clone())) {
                    Ok(transport) => transport,
                    Err(error) => {
                        checks.push(CheckResult::fail("openhands-auth", error));
                        return stop_managed_supervisor(checks, managed_supervisor);
                    }
                };
                http_detail = format!(
                    "started local supervisor and {probe_base_url} responded to /openapi.json"
                );
            }
            Ok(None) => {
                checks.push(CheckResult::fail("openhands-http", error.to_string()));
                return checks;
            }
            Err(start_error) => {
                checks.push(CheckResult::fail("openhands-supervisor-start", start_error));
                return checks;
            }
        }
    }

    let client = OpenHandsClient::new(transport);
    match client.openapi_probe().await {
        Ok(()) => checks.push(CheckResult::pass("openhands-http", http_detail)),
        Err(error) => {
            checks.push(CheckResult::fail("openhands-http", error.to_string()));
            return stop_managed_supervisor(checks, managed_supervisor);
        }
    }

    let temp_dir = match TempDir::new_in(&runtime.workflow.config.workspace.root) {
        Ok(temp_dir) => temp_dir,
        Err(error) => {
            checks.push(CheckResult::fail(
                "openhands-probe-dir",
                format!("failed to create probe dir: {error}"),
            ));
            return stop_managed_supervisor(checks, managed_supervisor);
        }
    };

    let probe_workspace = temp_dir.path().join("probe-workspace");
    let persistence_dir = probe_workspace.join(
        &runtime
            .workflow
            .extensions
            .openhands
            .conversation
            .persistence_dir_relative,
    );

    if let Err(error) = fs::create_dir_all(&persistence_dir).await {
        checks.push(CheckResult::fail(
            "openhands-probe-dir",
            format!("failed to build probe workspace: {error}"),
        ));
        return stop_managed_supervisor(checks, managed_supervisor);
    }

    let request = match build_doctor_probe_request(
        runtime,
        &probe_workspace,
        &persistence_dir,
        api_key,
        llm_base_url,
    ) {
        Ok(request) => request,
        Err(error) => {
            checks.push(CheckResult::fail("openhands-probe", error));
            return stop_managed_supervisor(checks, managed_supervisor);
        }
    };

    let probe_message = format!(
        "This is an OpenSymphony doctor health check. Do not inspect the repository, do not modify files, and do not call external tools. Confirm that the rendered workflow prompt below arrived successfully, then reply with the exact text `OpenSymphony doctor probe OK` and finish.\n\n--- BEGIN RENDERED WORKFLOW PROMPT ---\n{rendered_probe_prompt}\n--- END RENDERED WORKFLOW PROMPT ---"
    );

    match client
        .run_probe_with_message(&request, &probe_message, Duration::from_secs(5))
        .await
    {
        Ok(result) => {
            checks.push(CheckResult::pass(
                "openhands-conversation",
                format!("created {}", result.conversation.conversation_id),
            ));
            checks.push(CheckResult::pass(
                "openhands-websocket",
                format!("readiness event {}", result.ready_event.id),
            ));
            checks.push(CheckResult::pass(
                "openhands-reconcile",
                format!("reconciled {} event(s)", result.event_cache.items().len()),
            ));
        }
        Err(error) => {
            checks.push(CheckResult::fail("openhands-probe", error.to_string()));
            return stop_managed_supervisor(checks, managed_supervisor);
        }
    }

    stop_managed_supervisor(checks, managed_supervisor)
}

fn build_doctor_probe_request(
    runtime: &DoctorRuntimeConfig,
    probe_workspace: &Path,
    persistence_dir: &Path,
    api_key: Option<String>,
    llm_base_url: Option<String>,
) -> Result<ConversationCreateRequest, String> {
    let conversation = &runtime.workflow.extensions.openhands.conversation;
    let model = runtime.probe_model.clone().or_else(|| {
        conversation
            .agent
            .llm
            .as_ref()
            .and_then(|llm| llm.model.clone())
    });
    let max_iterations = u32::try_from(conversation.max_iterations).map_err(|_| {
        format!(
            "workflow max_iterations {} exceeds u32::MAX ({}), which is the maximum the doctor probe can handle",
            conversation.max_iterations,
            u32::MAX
        )
    })?;
    let mcp_config = McpConfig::from_stdio_servers(
        runtime
            .workflow
            .extensions
            .openhands
            .mcp
            .stdio_servers
            .iter()
            .map(|server| {
                let (command, args) = server
                    .command
                    .split_first()
                    .expect("workflow stdio server commands should be validated during resolution");
                McpStdioServerConfig {
                    name: server.name.clone(),
                    command: command.clone(),
                    args: args.to_vec(),
                    env: Default::default(),
                }
            })
            .collect(),
    );
    Ok(ConversationCreateRequest::doctor_probe_with_config(
        probe_workspace.display().to_string(),
        persistence_dir.display().to_string(),
        opensymphony_openhands::DoctorProbeConfig {
            max_iterations,
            stuck_detection: conversation.stuck_detection,
            confirmation_policy_kind: conversation.confirmation_policy.kind.clone(),
            agent_kind: conversation.agent.kind.clone(),
            model,
            api_key,
            base_url: llm_base_url,
            mcp_config,
        },
    ))
}

fn stop_managed_supervisor(
    mut checks: Vec<CheckResult>,
    managed_supervisor: Option<LocalServerSupervisor>,
) -> Vec<CheckResult> {
    if let Some(mut supervisor) = managed_supervisor {
        match supervisor.stop() {
            Ok(status) => checks.push(CheckResult::pass(
                "openhands-supervisor-stop",
                format!("stopped {}", status.base_url),
            )),
            Err(error) => checks.push(CheckResult::fail(
                "openhands-supervisor-stop",
                error.to_string(),
            )),
        }
    }

    checks
}

fn maybe_start_local_supervisor(
    runtime: &DoctorRuntimeConfig,
    tooling: Option<&LocalServerTooling>,
    transport: &TransportConfig,
) -> Result<Option<LocalServerSupervisor>, String> {
    if !runtime.workflow.extensions.openhands.local_server.enabled {
        return Ok(None);
    }

    let Some(tooling) = tooling else {
        return Ok(None);
    };

    if !tooling.pin_status.is_ready() {
        return Err(format!(
            "local tooling is not launchable: {}",
            tooling.pin_status.blocking_issues().join("; ")
        ));
    }

    let Some(supervisor_base_url) = transport
        .managed_local_server_base_url()
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let url = Url::parse(&supervisor_base_url)
        .map_err(|error| format!("invalid OpenHands base URL `{supervisor_base_url}`: {error}"))?;

    let mut supervisor_config = SupervisedServerConfig::new(tooling.clone());
    supervisor_config.extra_env = runtime
        .workflow
        .extensions
        .openhands
        .local_server
        .env
        .clone();
    supervisor_config.startup_timeout = Duration::from_millis(
        runtime
            .workflow
            .extensions
            .openhands
            .local_server
            .startup_timeout_ms,
    );
    supervisor_config.probe.path = runtime
        .workflow
        .extensions
        .openhands
        .local_server
        .readiness_probe_path
        .clone();
    supervisor_config.port_override = Some(url.port_or_known_default().ok_or_else(|| {
        format!("OpenHands base URL `{supervisor_base_url}` does not include a port")
    })?);

    let mut supervisor =
        LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(supervisor_config)));
    supervisor
        .start()
        .map_err(|error| format!("failed to start local OpenHands supervisor: {error}"))?;
    Ok(Some(supervisor))
}

fn spawn_demo_updates(store: SnapshotStore, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        let mut step = 1_u64;
        loop {
            ticker.tick().await;
            let snapshot = sample_snapshot(step);
            store.publish(snapshot).await;
            step += 1;
        }
    });
}

fn sample_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc::now();
    let runtime = match step % 4 {
        0 => IssueRuntimeState::Running,
        1 => IssueRuntimeState::Running,
        2 => IssueRuntimeState::RetryQueued,
        _ => IssueRuntimeState::Completed,
    };
    let outcome = match step % 4 {
        0 | 1 => WorkerOutcome::Running,
        2 => WorkerOutcome::Continued,
        _ => WorkerOutcome::Completed,
    };
    let daemon_state = if step == 0 {
        DaemonState::Starting
    } else {
        DaemonState::Ready
    };
    let issues = vec![
        IssueSnapshot {
            identifier: "COE-255".to_owned(),
            title: "Observability and FrankenTUI".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: runtime,
            last_outcome: outcome,
            last_event_at: now,
            conversation_id_suffix: "255-live".to_owned(),
            workspace_path_suffix: "COE-255".to_owned(),
            retry_count: if matches!(runtime, IssueRuntimeState::RetryQueued) {
                1
            } else {
                0
            },
            blocked: false,
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            transport_target: Some("loopback".to_owned()),
            http_auth_mode: Some("none".to_owned()),
            websocket_auth_mode: Some("none".to_owned()),
            websocket_query_param_name: None,
        },
        IssueSnapshot {
            identifier: "OSYM-401".to_owned(),
            title: "Control-plane API and snapshot store".to_owned(),
            tracker_state: "Done".to_owned(),
            runtime_state: IssueRuntimeState::Completed,
            last_outcome: WorkerOutcome::Completed,
            last_event_at: now - ChronoDuration::seconds(45),
            conversation_id_suffix: "401-done".to_owned(),
            workspace_path_suffix: "OSYM-401".to_owned(),
            retry_count: 0,
            blocked: false,
            server_base_url: Some("https://agent.example.com/runtime".to_owned()),
            transport_target: Some("remote".to_owned()),
            http_auth_mode: Some("header".to_owned()),
            websocket_auth_mode: Some("query_param".to_owned()),
            websocket_query_param_name: Some("session_api_key".to_owned()),
        },
        IssueSnapshot {
            identifier: "OSYM-402".to_owned(),
            title: "FrankenTUI operator client".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: if step.is_multiple_of(2) {
                IssueRuntimeState::Running
            } else {
                IssueRuntimeState::Idle
            },
            last_outcome: if step.is_multiple_of(2) {
                WorkerOutcome::Running
            } else {
                WorkerOutcome::Unknown
            },
            last_event_at: now - ChronoDuration::seconds(10),
            conversation_id_suffix: "402-ui".to_owned(),
            workspace_path_suffix: "OSYM-402".to_owned(),
            retry_count: 0,
            blocked: false,
            server_base_url: Some("https://agent.example.com/runtime".to_owned()),
            transport_target: Some("remote".to_owned()),
            http_auth_mode: Some("header".to_owned()),
            websocket_auth_mode: Some("header".to_owned()),
            websocket_query_param_name: None,
        },
    ];
    let running_issues = issues
        .iter()
        .filter(|issue| matches!(issue.runtime_state, IssueRuntimeState::Running))
        .count() as u32;
    let retry_queue_depth = issues
        .iter()
        .filter(|issue| matches!(issue.runtime_state, IssueRuntimeState::RetryQueued))
        .count() as u32;

    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: daemon_state,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony/workspaces".to_owned(),
            status_line: "scheduler heartbeat healthy".to_owned(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3002".to_owned(),
            conversation_count: 3,
            status_line: "local agent-server healthy".to_owned(),
        },
        metrics: MetricsSnapshot {
            running_issues,
            retry_queue_depth,
            total_tokens: 8_000 + (step * 240),
            total_cost_micros: 340_000 + (step * 9_500),
        },
        issues,
        recent_events: vec![
            RecentEvent {
                happened_at: now,
                issue_identifier: Some("COE-255".to_owned()),
                kind: RecentEventKind::SnapshotPublished,
                summary: format!("snapshot sequence advanced to step {step}"),
            },
            RecentEvent {
                happened_at: now - ChronoDuration::seconds(5),
                issue_identifier: Some("COE-255".to_owned()),
                kind: RecentEventKind::ClientAttached,
                summary: "FrankenTUI watcher connected to the control plane".to_owned(),
            },
            RecentEvent {
                happened_at: now - ChronoDuration::seconds(12),
                issue_identifier: Some("OSYM-402".to_owned()),
                kind: RecentEventKind::WorkerStarted,
                summary: "operator client reducer refreshed after live update".to_owned(),
            },
        ],
    }
}

fn print_checks(checks: &[CheckResult]) {
    for check in checks {
        let status = match check.status {
            CheckStatus::Pass => "PASS",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Skip => "SKIP",
        };

        println!("[{status}] {}: {}", check.name, check.detail);
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("opensymphony=info,opensymphony_control=info"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}

#[derive(Debug, Error)]
enum CommandError {
    #[error("failed to bind control-plane listener: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("FrankenTUI failed: {0}")]
    Tui(#[from] opensymphony_tui::TuiError),
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::Duration};

    use clap::{Parser, error::ErrorKind};
    use opensymphony_domain::{
        ControlPlaneDaemonState as DaemonState, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    };
    use opensymphony_workflow::WorkflowDefinition;
    use tempfile::TempDir;

    use super::{
        Cli, Command, DoctorRuntimeConfig, SnapshotStore, build_doctor_probe_request,
        command_check_name, discover_checkout_root, effective_openhands_probe_base_url,
        executable_suffixes, find_cargo_workspace_root, resolve_doctor_workflow, sample_snapshot,
        spawn_demo_updates,
    };

    #[test]
    fn daemon_rejects_zero_sample_interval() {
        let error = Cli::try_parse_from(["opensymphony", "daemon", "--sample-interval-ms", "0"])
            .expect_err("zero sample interval should be rejected");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn daemon_accepts_positive_sample_interval() {
        let cli = Cli::try_parse_from(["opensymphony", "daemon", "--sample-interval-ms", "250"])
            .expect("CLI fixture should parse");

        match cli.command {
            Command::Daemon(args) => assert_eq!(args.sample_interval_ms.get(), 250),
            Command::Run(_)
            | Command::Debug(_)
            | Command::Tui(_)
            | Command::LinearMcp(_)
            | Command::Doctor(_) => {
                panic!("expected daemon command")
            }
        }
    }

    #[test]
    fn sample_snapshot_metrics_match_rendered_issue_states() {
        for step in 0..8 {
            let snapshot = sample_snapshot(step);
            let running_issues = snapshot
                .issues
                .iter()
                .filter(|issue| matches!(issue.runtime_state, IssueRuntimeState::Running))
                .count() as u32;
            let retry_queue_depth = snapshot
                .issues
                .iter()
                .filter(|issue| matches!(issue.runtime_state, IssueRuntimeState::RetryQueued))
                .count() as u32;

            assert_eq!(snapshot.metrics.running_issues, running_issues);
            assert_eq!(snapshot.metrics.retry_queue_depth, retry_queue_depth);
        }
    }

    async fn wait_for_sequence(store: &SnapshotStore, target_sequence: u64) {
        loop {
            if store.current().await.sequence >= target_sequence {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn demo_updates_wait_for_the_first_interval_before_publishing() {
        let store = SnapshotStore::new(sample_snapshot(0));
        spawn_demo_updates(store.clone(), Duration::from_millis(120));

        tokio::time::sleep(Duration::from_millis(40)).await;
        let initial = store.current().await;
        assert_eq!(initial.sequence, 1);
        assert!(matches!(
            initial.snapshot.daemon.state,
            DaemonState::Starting
        ));

        tokio::time::timeout(Duration::from_millis(300), wait_for_sequence(&store, 2))
            .await
            .expect("first demo publish should occur after the configured interval");

        let updated = store.current().await;
        assert_eq!(updated.sequence, 2);
        assert!(matches!(updated.snapshot.daemon.state, DaemonState::Ready));
    }

    #[test]
    fn build_doctor_probe_request_reports_u32_limit_for_oversized_iterations() {
        let mut runtime = sample_doctor_runtime();
        runtime
            .workflow
            .extensions
            .openhands
            .conversation
            .max_iterations = u64::from(u32::MAX) + 1;

        let probe_workspace = PathBuf::from("/tmp/doctor-probe-workspace");
        let persistence_dir = probe_workspace.join("sessions");
        let error =
            build_doctor_probe_request(&runtime, &probe_workspace, &persistence_dir, None, None)
                .expect_err("oversized doctor probe max_iterations should fail");

        assert_eq!(
            error,
            format!(
                "workflow max_iterations {} exceeds u32::MAX ({}), which is the maximum the doctor probe can handle",
                u64::from(u32::MAX) + 1,
                u32::MAX
            )
        );
    }

    #[test]
    fn build_doctor_probe_request_forwards_mcp_stdio_servers() {
        let mut runtime = sample_doctor_runtime();
        runtime.workflow.extensions.openhands.mcp.stdio_servers =
            vec![opensymphony_workflow::OpenHandsStdioServerConfig {
                name: "linear".to_string(),
                command: vec![
                    "opensymphony".to_string(),
                    "linear-mcp".to_string(),
                    "--stdio".to_string(),
                ],
            }];

        let probe_workspace = PathBuf::from("/tmp/doctor-probe-workspace");
        let persistence_dir = probe_workspace.join("sessions");
        let request =
            build_doctor_probe_request(&runtime, &probe_workspace, &persistence_dir, None, None)
                .expect("doctor probe request should build");

        assert_eq!(
            request.mcp_config,
            Some(opensymphony_openhands::McpConfig {
                stdio_servers: vec![opensymphony_openhands::McpStdioServerConfig {
                    name: "linear".to_string(),
                    command: "opensymphony".to_string(),
                    args: vec!["linear-mcp".to_string(), "--stdio".to_string()],
                    env: Default::default(),
                }],
            })
        );
    }

    #[test]
    fn find_cargo_workspace_root_walks_up_from_nested_paths() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let config_root = repo_root.join("examples/configs");
        let tool_dir = repo_root.join("tools/openhands-server");
        fs::create_dir_all(&config_root).expect("config root should exist");
        fs::create_dir_all(&tool_dir).expect("tool dir should exist");
        fs::write(repo_root.join("Cargo.toml"), "[workspace]\nmembers = []\n")
            .expect("Cargo.toml should exist");

        let config_path = config_root.join("local-dev.yaml");
        let canonical_repo_root =
            fs::canonicalize(&repo_root).expect("repo root should canonicalize");

        assert_eq!(
            find_cargo_workspace_root(&config_path),
            Some(canonical_repo_root.clone())
        );
        assert_eq!(
            find_cargo_workspace_root(&tool_dir),
            Some(canonical_repo_root)
        );
    }

    #[test]
    fn discover_checkout_root_prefers_repo_anchored_inputs() {
        let temp_dir = TempDir::new().expect("temp dir");
        let repo_root = temp_dir.path().join("repo");
        let config_root = repo_root.join("examples/configs");
        let tool_dir = repo_root.join("tools/openhands-server");
        let target_repo = repo_root.join("examples/target-repo");
        fs::create_dir_all(&config_root).expect("config root should exist");
        fs::create_dir_all(&tool_dir).expect("tool dir should exist");
        fs::create_dir_all(&target_repo).expect("target repo should exist");
        fs::write(repo_root.join("Cargo.toml"), "[workspace]\nmembers = []\n")
            .expect("Cargo.toml should exist");
        let canonical_repo_root =
            fs::canonicalize(&repo_root).expect("repo root should canonicalize");

        let discovered = discover_checkout_root(&config_root, Some(&target_repo), &tool_dir)
            .expect("repo root should be discovered");

        assert_eq!(discovered, canonical_repo_root);
    }

    #[test]
    fn effective_openhands_probe_base_url_prefers_the_started_supervisor() {
        assert_eq!(
            effective_openhands_probe_base_url(
                "http://localhost:8000/opensymphony",
                Some("http://127.0.0.1:8000"),
            ),
            "http://127.0.0.1:8000"
        );
        assert_eq!(
            effective_openhands_probe_base_url("http://localhost:8000/opensymphony", None),
            "http://localhost:8000/opensymphony"
        );
    }

    #[test]
    fn command_check_names_are_stable() {
        assert_eq!(command_check_name("cargo"), "prereq-cargo");
        assert_eq!(command_check_name("curl"), "prereq-curl");
        assert_eq!(command_check_name("git"), "prereq-git");
        assert_eq!(command_check_name("uv"), "prereq-uv");
    }

    #[test]
    fn executable_suffixes_are_non_empty() {
        assert!(
            !executable_suffixes().is_empty(),
            "executable lookup should always have at least one suffix"
        );
    }

    fn sample_doctor_runtime() -> DoctorRuntimeConfig {
        let temp_dir = TempDir::new().expect("temp dir should be created");
        let target_repo = temp_dir.path().join("target-repo");
        fs::create_dir_all(&target_repo).expect("target repo should exist");

        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces
openhands:
  transport:
    base_url: http://127.0.0.1:8000
---

# Doctor Probe
"#,
        )
        .expect("workflow should parse");
        let workflow = resolve_doctor_workflow(&workflow, &target_repo, false)
            .expect("workflow should resolve with Linear disabled");

        DoctorRuntimeConfig {
            target_repo,
            workflow_path: temp_dir.path().join("target-repo/WORKFLOW.md"),
            workflow,
            tool_dir: temp_dir.path().join("tools/openhands-server"),
            probe_model: None,
            probe_api_key_env: None,
            probe_llm_base_url_env: None,
        }
    }
}
