use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use opensymphony_linear_mcp::run_stdio_server as run_linear_mcp_stdio_server;
use opensymphony_openhands::{
    ConversationCreateRequest, LocalServerSupervisor, LocalServerTooling, OpenHandsClient,
    SupervisedServerConfig, SupervisorConfig, TransportConfig,
};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::fs;
use tracing_subscriber::{fmt, EnvFilter};
use url::Url;

#[derive(Parser)]
#[command(name = "opensymphony")]
#[command(about = "OpenSymphony local MVP CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Daemon,
    Tui,
    LinearMcp(LinearMcpArgs),
    Doctor(DoctorArgs),
}

#[derive(Args)]
pub struct DoctorArgs {
    #[arg(long, default_value = "examples/configs/local-dev.yaml")]
    config: PathBuf,
    #[arg(long)]
    live_openhands: bool,
}

#[derive(Args)]
pub struct LinearMcpArgs {}

#[derive(Debug, Deserialize)]
struct DoctorConfig {
    workspace_root: String,
    target_repo: Option<String>,
    openhands: OpenHandsDoctorConfig,
    linear: LinearDoctorConfig,
}

#[derive(Debug, Deserialize)]
struct OpenHandsDoctorConfig {
    base_url: String,
    tool_dir: String,
    probe_model: Option<String>,
    probe_api_key_env: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LinearDoctorConfig {
    enabled: bool,
    api_key_env: String,
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
        Command::Doctor(args) => run_doctor(args).await,
        Command::Daemon => {
            println!("`opensymphony daemon` is scaffolded but not implemented in this branch.");
            ExitCode::SUCCESS
        }
        Command::Tui => {
            println!("`opensymphony tui` is scaffolded but not implemented in this branch.");
            ExitCode::SUCCESS
        }
        Command::LinearMcp(args) => run_linear_mcp(args).await,
    }
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
    let repo_root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|target_repo| resolve_path(config_root, target_repo))
        .unwrap_or_else(|| repo_root.join("examples/target-repo"));
    let workspace_root = resolve_path(config_root, &config.workspace_root);
    let tool_dir = resolve_path(config_root, &config.openhands.tool_dir);

    checks.push(check_repo_root(&repo_root));
    checks.push(check_target_repo(&target_repo).await);
    checks.push(check_workspace_root(&workspace_root).await);
    checks.push(check_tool_dir(&tool_dir).await);
    checks.push(check_loopback_base_url(&config.openhands.base_url));
    checks.push(check_linear_env(&config.linear));

    let tooling_inspection = inspect_local_tooling(&tool_dir);
    checks.extend(tooling_inspection.checks);

    if live_openhands {
        checks.extend(
            run_live_openhands_checks(
                &config,
                &workspace_root,
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
    let expanded = expand_env_tokens(&raw);
    serde_yaml::from_str(&expanded)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

fn expand_env_tokens(input: &str) -> String {
    let mut expanded = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            let _ = chars.next();
            let mut key = String::new();
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
                key.push(next);
            }
            expanded.push_str(&env::var(key).unwrap_or_default());
        } else {
            expanded.push(ch);
        }
    }

    expanded
}

fn resolve_path(base: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
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
    let runner = tool_dir.join("run-local.sh");

    if version.exists() && pyproject.exists() && runner.exists() {
        CheckResult::pass(
            "openhands-tooling",
            format!("pin files found in {}", tool_dir.display()),
        )
    } else {
        CheckResult::fail(
            "openhands-tooling",
            format!(
                "expected {}, {}, and {}",
                version.display(),
                pyproject.display(),
                runner.display()
            ),
        )
    }
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

fn check_loopback_base_url(base_url: &str) -> CheckResult {
    match Url::parse(base_url) {
        Ok(url) => match url.host_str() {
            Some("127.0.0.1") | Some("localhost") => CheckResult::pass(
                "bind-scope",
                format!("OpenHands base URL is loopback: {base_url}"),
            ),
            Some(host) => CheckResult::warn(
                "bind-scope",
                format!("OpenHands base URL host `{host}` is not loopback in local mode"),
            ),
            None => CheckResult::fail("bind-scope", format!("base URL `{base_url}` has no host")),
        },
        Err(error) => CheckResult::fail(
            "bind-scope",
            format!("invalid base URL `{base_url}`: {error}"),
        ),
    }
}

fn check_linear_env(linear: &LinearDoctorConfig) -> CheckResult {
    if !linear.enabled {
        return CheckResult::skip(
            "linear",
            "Linear checks skipped because `linear.enabled` is false",
        );
    }

    match env::var(&linear.api_key_env) {
        Ok(_) => CheckResult::pass("linear", format!("found {}", linear.api_key_env)),
        Err(_) => CheckResult::warn(
            "linear",
            format!(
                "missing {} while Linear mode is enabled",
                linear.api_key_env
            ),
        ),
    }
}

async fn run_live_openhands_checks(
    config: &DoctorConfig,
    workspace_root: &Path,
    tooling: Option<&LocalServerTooling>,
) -> Vec<CheckResult> {
    let mut checks = Vec::new();
    let api_key = config
        .openhands
        .probe_api_key_env
        .as_ref()
        .and_then(|env_name| env::var(env_name).ok());

    if let Some(env_name) = &config.openhands.probe_api_key_env {
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

    let mut managed_supervisor = None;
    let mut http_detail = format!("{} responded to /openapi.json", config.openhands.base_url);
    let client = OpenHandsClient::new(TransportConfig::new(config.openhands.base_url.clone()));
    if let Err(error) = client.openapi_probe().await {
        match maybe_start_local_supervisor(config, tooling) {
            Ok(Some(mut supervisor)) => {
                let started = match supervisor.status() {
                    Ok(status) => status,
                    Err(status_error) => {
                        checks.push(CheckResult::fail(
                            "openhands-supervisor-status",
                            status_error.to_string(),
                        ));
                        return checks;
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
                managed_supervisor = Some(supervisor);
                http_detail = format!(
                    "started local supervisor and {} responded to /openapi.json",
                    config.openhands.base_url
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

    let client = OpenHandsClient::new(TransportConfig::new(config.openhands.base_url.clone()));
    match client.openapi_probe().await {
        Ok(()) => checks.push(CheckResult::pass("openhands-http", http_detail)),
        Err(error) => {
            checks.push(CheckResult::fail("openhands-http", error.to_string()));
            return stop_managed_supervisor(checks, managed_supervisor);
        }
    }

    let temp_dir = match TempDir::new_in(workspace_root) {
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
    if let Err(error) = fs::create_dir_all(probe_workspace.join(".opensymphony/openhands")).await {
        checks.push(CheckResult::fail(
            "openhands-probe-dir",
            format!("failed to build probe workspace: {error}"),
        ));
        return stop_managed_supervisor(checks, managed_supervisor);
    }

    let request = ConversationCreateRequest::doctor_probe(
        probe_workspace.display().to_string(),
        probe_workspace
            .join(".opensymphony/openhands")
            .display()
            .to_string(),
        normalized_option(&config.openhands.probe_model),
        api_key,
    );

    match client.run_probe(&request, Duration::from_secs(5)).await {
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
    config: &DoctorConfig,
    tooling: Option<&LocalServerTooling>,
) -> Result<Option<LocalServerSupervisor>, String> {
    let Some(tooling) = tooling else {
        return Ok(None);
    };

    if !tooling.pin_status.is_ready() {
        return Err(format!(
            "local tooling is not launchable: {}",
            tooling.pin_status.blocking_issues().join("; ")
        ));
    }

    let url = Url::parse(&config.openhands.base_url).map_err(|error| {
        format!(
            "invalid OpenHands base URL `{}`: {error}",
            config.openhands.base_url
        )
    })?;
    match url.host_str() {
        Some("127.0.0.1") | Some("localhost") => {}
        _ => return Ok(None),
    }

    let mut supervisor_config = SupervisedServerConfig::new(tooling.clone());
    supervisor_config.port_override = Some(url.port_or_known_default().ok_or_else(|| {
        format!(
            "OpenHands base URL `{}` does not include a port",
            config.openhands.base_url
        )
    })?);

    let mut supervisor =
        LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(supervisor_config)));
    supervisor
        .start()
        .map_err(|error| format!("failed to start local OpenHands supervisor: {error}"))?;
    Ok(Some(supervisor))
}

fn normalized_option(value: &Option<String>) -> Option<String> {
    value
        .as_ref()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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
    let _ = fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .try_init();
}
