use std::{
    env,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Duration,
};

use clap::{Args, Parser, Subcommand};
use opensymphony_openhands::{
    ConversationCreateRequest, DoctorProbeConfig, LocalServerSupervisor, LocalServerTooling,
    OpenHandsClient, SupervisedServerConfig, SupervisorConfig, TransportConfig,
};
use opensymphony_workflow::{Environment, ResolvedWorkflow, WorkflowDefinition};
use serde::{Deserialize, Serialize};
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
    LinearMcp,
    Doctor(DoctorArgs),
}

#[derive(Args)]
pub struct DoctorArgs {
    #[arg(long, default_value = "examples/configs/local-dev.yaml")]
    config: PathBuf,
    #[arg(long)]
    live_openhands: bool,
}

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
        Command::LinearMcp => {
            println!("`opensymphony linear-mcp` is scaffolded but not implemented in this branch.");
            ExitCode::SUCCESS
        }
    }
}

async fn run_doctor(args: DoctorArgs) -> ExitCode {
    run_doctor_command(args.config, args.live_openhands).await
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
            checks.push(check_loopback_base_url(
                &runtime.workflow.extensions.openhands.transport.base_url,
            ));
            checks.push(check_linear(&config.linear, &runtime.workflow));
            (runtime, rendered_probe_prompt)
        }
        Err(error) => {
            let target_repo = config
                .target_repo
                .as_deref()
                .map(|target_repo| resolve_path(config_root, target_repo))
                .unwrap_or_else(|| repo_root.join("examples/target-repo"));
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
        probe_model: normalized_option(&config.openhands.probe_model),
        probe_api_key_env: normalized_option(&config.openhands.probe_api_key_env),
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

    let base_url = &runtime.workflow.extensions.openhands.transport.base_url;
    let mut managed_supervisor = None;
    let mut http_detail = format!("{base_url} responded to /openapi.json");
    let client = OpenHandsClient::new(TransportConfig::new(base_url.clone()));
    if let Err(error) = client.openapi_probe().await {
        match maybe_start_local_supervisor(runtime, tooling) {
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
                    base_url
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

    let client = OpenHandsClient::new(TransportConfig::new(base_url.clone()));
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

    let request =
        match build_doctor_probe_request(runtime, &probe_workspace, &persistence_dir, api_key) {
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
            "workflow max_iterations {} exceeds the current doctor probe limit type",
            conversation.max_iterations
        )
    })?;

    Ok(ConversationCreateRequest::doctor_probe_with_config(
        probe_workspace.display().to_string(),
        persistence_dir.display().to_string(),
        DoctorProbeConfig {
            max_iterations,
            stuck_detection: conversation.stuck_detection,
            confirmation_policy_kind: conversation.confirmation_policy.kind.clone(),
            agent_kind: conversation.agent.kind.clone(),
            model,
            api_key,
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

    let base_url = &runtime.workflow.extensions.openhands.transport.base_url;
    let url = Url::parse(base_url)
        .map_err(|error| format!("invalid OpenHands base URL `{base_url}`: {error}"))?;
    match url.host_str() {
        Some("127.0.0.1") | Some("localhost") => {}
        _ => return Ok(None),
    }

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
    supervisor_config.port_override = Some(
        url.port_or_known_default()
            .ok_or_else(|| format!("OpenHands base URL `{base_url}` does not include a port"))?,
    );

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
