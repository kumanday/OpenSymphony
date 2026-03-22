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
use opensymphony_control::{ControlPlaneServer, SnapshotStore};
use opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome,
};
use opensymphony_openhands::{
    ConversationCreateRequest, LocalServerSupervisor, LocalServerTooling, OpenHandsClient,
    SupervisedServerConfig, SupervisorConfig, TransportConfig,
};
use opensymphony_tui::TuiError;
use serde::Deserialize;
use tempfile::TempDir;
use thiserror::Error;
use tokio::fs;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};
use url::Url;

#[derive(Debug, Parser)]
#[command(name = "opensymphony")]
#[command(about = "OpenSymphony local MVP CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Daemon(DaemonArgs),
    Tui(TuiArgs),
    LinearMcp,
    Doctor(DoctorArgs),
}

#[derive(Debug, Args)]
struct DaemonArgs {
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(long, default_value = "1200")]
    sample_interval_ms: NonZeroU64,
}

#[derive(Debug, Args)]
struct TuiArgs {
    #[arg(long, default_value = "http://127.0.0.1:3000/")]
    url: Url,
    #[arg(long)]
    exit_after_ms: Option<u64>,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    #[arg(long, default_value = "examples/configs/local-dev.yaml")]
    config: PathBuf,
    #[arg(long)]
    live_openhands: bool,
}

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
        Command::Daemon(args) => {
            report_result(run_daemon(args.bind, args.sample_interval_ms).await)
        }
        Command::Tui(args) => report_result(run_tui(args.url, args.exit_after_ms).await),
        Command::LinearMcp => {
            println!("`opensymphony linear-mcp` is scaffolded but not implemented in this branch.");
            ExitCode::SUCCESS
        }
    }
}

async fn run_doctor(args: DoctorArgs) -> ExitCode {
    run_doctor_command(args.config, args.live_openhands).await
}

fn report_result(result: Result<(), CliError>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_daemon(bind: SocketAddr, sample_interval_ms: NonZeroU64) -> Result<(), CliError> {
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
            result.map_err(CliError::Join)??;
            Ok(())
        }
        _ = tokio::signal::ctrl_c() => {
            info!("shutting down control plane");
            Ok(())
        }
    }
}

async fn run_tui(url: Url, exit_after_ms: Option<u64>) -> Result<(), CliError> {
    let exit_after = exit_after_ms.map(Duration::from_millis);
    tokio::task::spawn_blocking(move || opensymphony_tui::run_operator(url, exit_after))
        .await
        .map_err(CliError::Join)?
        .map_err(CliError::Tui)
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
        },
        IssueSnapshot {
            identifier: "OSYM-402".to_owned(),
            title: "FrankenTUI operator client".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: if step % 2 == 0 {
                IssueRuntimeState::Running
            } else {
                IssueRuntimeState::Idle
            },
            last_outcome: if step % 2 == 0 {
                WorkerOutcome::Running
            } else {
                WorkerOutcome::Unknown
            },
            last_event_at: now - ChronoDuration::seconds(10),
            conversation_id_suffix: "402-ui".to_owned(),
            workspace_path_suffix: "OSYM-402".to_owned(),
            retry_count: 0,
            blocked: false,
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
    let tool_dir = resolve_path(config_root, &config.openhands.tool_dir);
    let configured_target_repo = config
        .target_repo
        .as_deref()
        .map(|target_repo| resolve_path(config_root, target_repo));
    let repo_root =
        discover_checkout_root(config_root, configured_target_repo.as_deref(), &tool_dir)
            .or_else(|| {
                env::current_dir()
                    .ok()
                    .and_then(|cwd| find_cargo_workspace_root(&cwd).or(Some(cwd)))
            })
            .unwrap_or_else(|| PathBuf::from("."));
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|target_repo| resolve_path(config_root, target_repo))
        .unwrap_or_else(|| repo_root.join("examples/target-repo"));
    let workspace_root = resolve_path(config_root, &config.workspace_root);

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
    let config = serde_yaml::from_str(&raw)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    resolve_doctor_config(config)
        .map_err(|error| format!("failed to expand {}: {error}", path.display()))
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

fn resolve_doctor_config(
    mut config: DoctorConfig,
) -> Result<DoctorConfig, ResolveDoctorConfigError> {
    config.workspace_root =
        resolve_required_config_value("workspace_root", &config.workspace_root)?;
    if let Some(target_repo) = config.target_repo.take() {
        config.target_repo = Some(resolve_required_config_value("target_repo", &target_repo)?);
    }
    config.openhands.base_url =
        resolve_required_config_value("openhands.base_url", &config.openhands.base_url)?;
    config.openhands.tool_dir =
        resolve_required_config_value("openhands.tool_dir", &config.openhands.tool_dir)?;
    if config.linear.enabled {
        config.linear.api_key_env =
            resolve_required_config_value("linear.api_key_env", &config.linear.api_key_env)?;
    }
    Ok(config)
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

fn resolve_path(base: &Path, raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
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
        .map(|candidate| candidate.to_path_buf())
}

fn effective_openhands_probe_base_url(
    configured_base_url: &str,
    started_supervisor_base_url: Option<&str>,
) -> String {
    started_supervisor_base_url
        .unwrap_or(configured_base_url)
        .to_string()
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
    let probe_api_key_env = match resolve_optional_config_value(
        "openhands.probe_api_key_env",
        config.openhands.probe_api_key_env.as_deref(),
    ) {
        Ok(value) => value,
        Err(error) => {
            checks.push(CheckResult::fail("config", error.to_string()));
            return checks;
        }
    };
    let probe_model = match resolve_optional_config_value(
        "openhands.probe_model",
        config.openhands.probe_model.as_deref(),
    ) {
        Ok(value) => value,
        Err(error) => {
            checks.push(CheckResult::fail("config", error.to_string()));
            return checks;
        }
    };
    let api_key = probe_api_key_env
        .as_ref()
        .and_then(|env_name| env::var(env_name).ok());

    if let Some(env_name) = &probe_api_key_env {
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
    let mut probe_base_url = config.openhands.base_url.clone();
    let mut http_detail = format!("{probe_base_url} responded to /openapi.json");
    let client = OpenHandsClient::new(TransportConfig::new(probe_base_url.clone()));
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
                probe_base_url = effective_openhands_probe_base_url(
                    &config.openhands.base_url,
                    Some(&started.base_url),
                );
                managed_supervisor = Some(supervisor);
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

    let client = OpenHandsClient::new(TransportConfig::new(probe_base_url));
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
        probe_model,
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

#[derive(Debug, Error)]
enum CliError {
    #[error("failed to bind control-plane listener: {0}")]
    Io(#[from] std::io::Error),
    #[error("daemon task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
    #[error("FrankenTUI failed: {0}")]
    Tui(#[from] TuiError),
}

#[cfg(test)]
mod tests {
    use super::{
        discover_checkout_root, effective_openhands_probe_base_url, find_cargo_workspace_root,
        sample_snapshot, spawn_demo_updates, Cli, Command, SnapshotStore,
    };
    use clap::{error::ErrorKind, Parser};
    use opensymphony_domain::{
        ControlPlaneDaemonState as DaemonState, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    };
    use std::{fs, time::Duration};
    use tempfile::TempDir;

    #[test]
    fn daemon_rejects_zero_sample_interval() {
        let error = Cli::try_parse_from(["opensymphony", "daemon", "--sample-interval-ms", "0"])
            .expect_err("zero sample interval should fail CLI parsing");

        assert_eq!(error.kind(), ErrorKind::ValueValidation);
    }

    #[test]
    fn daemon_accepts_positive_sample_interval() {
        let cli = Cli::try_parse_from(["opensymphony", "daemon", "--sample-interval-ms", "250"])
            .expect("parse positive demo sample interval");

        match cli.command {
            Command::Daemon(args) => {
                assert_eq!(args.sample_interval_ms.get(), 250);
            }
            _ => panic!("expected daemon command"),
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

        assert_eq!(
            find_cargo_workspace_root(&config_path),
            Some(repo_root.clone())
        );
        assert_eq!(find_cargo_workspace_root(&tool_dir), Some(repo_root));
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

        let discovered = discover_checkout_root(&config_root, Some(&target_repo), &tool_dir)
            .expect("repo root should be discovered");

        assert_eq!(discovered, repo_root);
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
}
