use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::opensymphony_workflow::WorkflowDefinition;

use super::orchestrator_run::config::{
    CentralConfigError, load_central_config, looks_like_central_config,
};

#[derive(Debug, Args)]
pub struct MigrationArgs {
    #[command(subcommand)]
    command: MigrationCommand,
}

#[derive(Debug, Subcommand)]
enum MigrationCommand {
    #[command(about = "Report migration risks without writing files")]
    Preflight(MigrationPaths),
    #[command(about = "Apply a staged, recoverable central-config migration")]
    Apply(MigrationPaths),
    #[command(about = "Restore the last backed-up runnable configuration")]
    Rollback(RollbackArgs),
}

#[derive(Debug, Args, Clone)]
struct MigrationPaths {
    #[arg(long, help = "Legacy config path; defaults to <repo>/config.yaml")]
    config: Option<PathBuf>,
    #[arg(long, default_value = ".", help = "Legacy repository root")]
    repo: PathBuf,
    #[arg(long, help = "Central config destination; apply defaults to --config")]
    output: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct RollbackArgs {
    #[arg(long, help = "Central config path; defaults to ./config.yaml")]
    config: Option<PathBuf>,
}

#[derive(Debug, Error)]
enum MigrationError {
    #[error("migration input {path} does not exist")]
    MissingInput { path: PathBuf },
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse legacy config {path}: {source}")]
    ParseConfig {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to parse workflow {path}: {source}")]
    ParseWorkflow {
        path: PathBuf,
        #[source]
        source: crate::opensymphony_workflow::WorkflowLoadError,
    },
    #[error("migration cannot serialize a literal tracker secret")]
    LiteralSecret,
    #[error("migration cannot preserve a credential-bearing repository remote")]
    CredentialBearingRemote,
    #[error("failed to serialize generated central config: {0}")]
    SerializeConfig(#[source] serde_yaml::Error),
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("migration has no activation marker at {path}")]
    MissingActivation { path: PathBuf },
    #[error("rollback is blocked while an active strict run marker exists at {path}")]
    ActiveStrictRun { path: PathBuf },
    #[error("invalid migration activation marker {path}: {source}")]
    ParseActivation {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("central config validation failed: {0}")]
    CentralConfig(#[from] CentralConfigError),
    #[error("git did not provide a usable repository remote")]
    MissingRemote,
}

#[derive(Debug, Serialize)]
struct MigrationReport {
    operation: &'static str,
    source_config: PathBuf,
    target_repo: PathBuf,
    workflow: PathBuf,
    target_config: Option<PathBuf>,
    central_config_already_active: bool,
    preflight_only: bool,
    config_generation: Option<String>,
    recognized_front_matter: Vec<&'static str>,
    hardcoded_clone_hooks: Vec<&'static str>,
    literal_secret_detected: bool,
    credential_bearing_remote_detected: bool,
    backup: Option<PathBuf>,
    activation_marker: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct LegacyConfigProbe {
    #[serde(default)]
    target_repo: Option<String>,
    #[serde(default)]
    control_plane: LegacyControlPlaneProbe,
    #[serde(default)]
    openhands: LegacyOpenHandsProbe,
    #[serde(default)]
    memory: LegacyMemoryProbe,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyControlPlaneProbe {
    bind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyOpenHandsProbe {
    tool_dir: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct LegacyMemoryProbe {
    auto_capture: Option<bool>,
    auto_archive: Option<bool>,
    serve: Option<bool>,
    bind: Option<String>,
    token_env: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ActivationMarker {
    config_path: PathBuf,
    workflow_path: PathBuf,
    backup_dir: PathBuf,
    generation: String,
    had_config: bool,
    had_workflow: bool,
}

#[derive(Debug)]
struct SourceContext {
    source_config: PathBuf,
    config_source: String,
    target_repo: PathBuf,
    workflow_path: PathBuf,
    workflow_source: String,
    workflow: WorkflowDefinition,
    config: LegacyConfigProbe,
    remote: String,
}

pub async fn run(args: MigrationArgs) -> std::process::ExitCode {
    let result = match args.command {
        MigrationCommand::Preflight(paths) => preflight(paths).await,
        MigrationCommand::Apply(paths) => apply(paths).await,
        MigrationCommand::Rollback(args) => rollback(args).await,
    };
    match result {
        Ok(report) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).expect("migration report is serializable")
            );
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("migration failed: {error}");
            std::process::ExitCode::from(1)
        }
    }
}

async fn preflight(paths: MigrationPaths) -> Result<MigrationReport, MigrationError> {
    let source = load_source(&paths)?;
    if looks_like_central_config(&source.config_source) {
        let central = load_central_config(&source.source_config).await?;
        let _ = (
            &central.instance_id,
            &central.state_root,
            &central.workspace_root,
            &central.mode,
            &central.repository,
            &central.integration_instructions,
        );
        return Ok(MigrationReport {
            operation: "preflight",
            source_config: source.source_config,
            target_repo: source.target_repo,
            workflow: source.workflow_path,
            target_config: None,
            central_config_already_active: true,
            preflight_only: true,
            config_generation: Some(central.generation),
            recognized_front_matter: Vec::new(),
            hardcoded_clone_hooks: Vec::new(),
            literal_secret_detected: false,
            credential_bearing_remote_detected: false,
            backup: None,
            activation_marker: None,
        });
    }
    Ok(build_report("preflight", &source, None, None, true))
}

async fn apply(paths: MigrationPaths) -> Result<MigrationReport, MigrationError> {
    let source = load_source(&paths)?;
    if looks_like_central_config(&source.config_source) {
        let central = load_central_config(&source.source_config).await?;
        let _ = (
            &central.instance_id,
            &central.state_root,
            &central.workspace_root,
            &central.mode,
            &central.repository,
            &central.integration_instructions,
        );
        return Ok(MigrationReport {
            operation: "apply",
            source_config: source.source_config,
            target_repo: source.target_repo,
            workflow: source.workflow_path,
            target_config: None,
            central_config_already_active: true,
            preflight_only: false,
            config_generation: Some(central.generation),
            recognized_front_matter: Vec::new(),
            hardcoded_clone_hooks: Vec::new(),
            literal_secret_detected: false,
            credential_bearing_remote_detected: false,
            backup: None,
            activation_marker: None,
        });
    }

    let report = build_report("apply", &source, None, None, false);
    if report.literal_secret_detected {
        return Err(MigrationError::LiteralSecret);
    }
    if report.credential_bearing_remote_detected {
        return Err(MigrationError::CredentialBearingRemote);
    }
    if !report.hardcoded_clone_hooks.is_empty() {
        return Err(MigrationError::Write {
            path: source.workflow_path,
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "recognized clone hook requires explicit operator migration",
            ),
        });
    }

    let cwd = current_dir()?;
    let target_config = paths
        .output
        .map(|path| absolute_path(&cwd, &path))
        .unwrap_or_else(|| source.source_config.clone());
    let generated = generate_central_config(&source)?;
    let generation = sha256(generated.as_bytes());
    let migration_root = target_config
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".opensymphony/migration");
    let backup_dir = migration_root
        .join("backups")
        .join(generation.trim_start_matches("sha256:"));
    fs::create_dir_all(&backup_dir).map_err(|source_error| MigrationError::Write {
        path: backup_dir.clone(),
        source: source_error,
    })?;
    let had_config = target_config.is_file();
    let had_workflow = source.workflow_path.is_file();
    if had_config {
        fs::copy(&target_config, backup_dir.join("config.yaml")).map_err(|source_error| {
            MigrationError::Write {
                path: backup_dir.join("config.yaml"),
                source: source_error,
            }
        })?;
    }
    if had_workflow {
        fs::copy(&source.workflow_path, backup_dir.join("WORKFLOW.md")).map_err(
            |source_error| MigrationError::Write {
                path: backup_dir.join("WORKFLOW.md"),
                source: source_error,
            },
        )?;
    }

    let central_stage = stage_path(&target_config, &generation);
    let workflow_stage = stage_path(&source.workflow_path, &generation);
    write_file(&central_stage, generated.as_bytes())?;
    load_central_config(&central_stage).await?;
    write_file(&workflow_stage, workflow_body(&source))?;

    let marker = ActivationMarker {
        config_path: target_config.clone(),
        workflow_path: source.workflow_path.clone(),
        backup_dir: backup_dir.clone(),
        generation: generation.clone(),
        had_config,
        had_workflow,
    };
    let marker_path = migration_root.join("activation.yaml");
    let marker_stage = stage_path(&marker_path, &generation);
    let marker_raw = serde_yaml::to_string(&marker).map_err(MigrationError::SerializeConfig)?;
    write_file(&marker_stage, marker_raw.as_bytes())?;
    fs::rename(&marker_stage, &marker_path).map_err(|source_error| MigrationError::Write {
        path: marker_path.clone(),
        source: source_error,
    })?;

    fs::rename(&central_stage, &target_config).map_err(|source_error| MigrationError::Write {
        path: target_config.clone(),
        source: source_error,
    })?;
    fs::rename(&workflow_stage, &source.workflow_path).map_err(|source_error| {
        MigrationError::Write {
            path: source.workflow_path.clone(),
            source: source_error,
        }
    })?;

    Ok(MigrationReport {
        operation: "apply",
        source_config: source.source_config,
        target_repo: source.target_repo,
        workflow: source.workflow_path,
        target_config: Some(target_config),
        central_config_already_active: false,
        preflight_only: false,
        config_generation: Some(generation),
        recognized_front_matter: report.recognized_front_matter,
        hardcoded_clone_hooks: report.hardcoded_clone_hooks,
        literal_secret_detected: false,
        credential_bearing_remote_detected: false,
        backup: Some(backup_dir),
        activation_marker: Some(marker_path),
    })
}

async fn rollback(args: RollbackArgs) -> Result<MigrationReport, MigrationError> {
    let cwd = current_dir()?;
    let config_path = args
        .config
        .map(|path| absolute_path(&cwd, &path))
        .unwrap_or_else(|| cwd.join("config.yaml"));
    let migration_root = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".opensymphony/migration");
    let marker_path = migration_root.join("activation.yaml");
    if !marker_path.is_file() {
        return Err(MigrationError::MissingActivation { path: marker_path });
    }
    let marker_raw = fs::read_to_string(&marker_path).map_err(|source| MigrationError::Read {
        path: marker_path.clone(),
        source,
    })?;
    let marker: ActivationMarker =
        serde_yaml::from_str(&marker_raw).map_err(|source| MigrationError::ParseActivation {
            path: marker_path.clone(),
            source,
        })?;
    let active_run_marker = migration_root.join("strict-run.active");
    if active_run_marker.exists() {
        return Err(MigrationError::ActiveStrictRun {
            path: active_run_marker,
        });
    }

    if marker.had_config {
        restore_file(&marker.backup_dir.join("config.yaml"), &marker.config_path)?;
    } else if marker.config_path.is_file() {
        fs::remove_file(&marker.config_path).map_err(|source| MigrationError::Write {
            path: marker.config_path.clone(),
            source,
        })?;
    }
    if marker.had_workflow {
        restore_file(
            &marker.backup_dir.join("WORKFLOW.md"),
            &marker.workflow_path,
        )?;
    }
    fs::remove_file(&marker_path).map_err(|source| MigrationError::Write {
        path: marker_path.clone(),
        source,
    })?;

    Ok(MigrationReport {
        operation: "rollback",
        source_config: marker.config_path.clone(),
        target_repo: marker
            .workflow_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
        workflow: marker.workflow_path,
        target_config: Some(marker.config_path),
        central_config_already_active: false,
        preflight_only: false,
        config_generation: Some(marker.generation),
        recognized_front_matter: Vec::new(),
        hardcoded_clone_hooks: Vec::new(),
        literal_secret_detected: false,
        credential_bearing_remote_detected: false,
        backup: Some(marker.backup_dir),
        activation_marker: Some(marker_path),
    })
}

fn load_source(paths: &MigrationPaths) -> Result<SourceContext, MigrationError> {
    let cwd = current_dir()?;
    let repo = absolute_path(&cwd, &paths.repo);
    let source_config = paths
        .config
        .as_ref()
        .map(|path| absolute_path(&cwd, path))
        .unwrap_or_else(|| repo.join("config.yaml"));
    let config_source = if source_config.is_file() {
        fs::read_to_string(&source_config).map_err(|source| MigrationError::Read {
            path: source_config.clone(),
            source,
        })?
    } else {
        String::new()
    };
    let config = if config_source.is_empty() {
        LegacyConfigProbe {
            target_repo: None,
            control_plane: LegacyControlPlaneProbe::default(),
            openhands: LegacyOpenHandsProbe::default(),
            memory: LegacyMemoryProbe::default(),
        }
    } else {
        serde_yaml::from_str(&config_source).map_err(|source| MigrationError::ParseConfig {
            path: source_config.clone(),
            source,
        })?
    };
    let config_root = source_config.parent().unwrap_or_else(|| Path::new("."));
    let target_repo = config
        .target_repo
        .as_deref()
        .map(|path| resolve_repo_path(config_root, path))
        .unwrap_or(repo);
    let workflow_path = target_repo.join("WORKFLOW.md");
    let workflow_source = fs::read_to_string(&workflow_path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            MigrationError::MissingInput {
                path: workflow_path.clone(),
            }
        } else {
            MigrationError::Read {
                path: workflow_path.clone(),
                source,
            }
        }
    })?;
    let workflow = WorkflowDefinition::parse(&workflow_source).map_err(|source| {
        MigrationError::ParseWorkflow {
            path: workflow_path.clone(),
            source,
        }
    })?;
    let remote = git_remote(&target_repo)?;
    Ok(SourceContext {
        source_config,
        config_source,
        target_repo,
        workflow_path,
        workflow_source,
        workflow,
        config,
        remote,
    })
}

fn build_report(
    operation: &'static str,
    source: &SourceContext,
    target_config: Option<PathBuf>,
    backup: Option<PathBuf>,
    preflight_only: bool,
) -> MigrationReport {
    let front_matter = &source.workflow.front_matter;
    let mut recognized = Vec::new();
    if front_matter.tracker.project_slug.is_some() {
        recognized.push("tracker");
    }
    if front_matter.polling.interval_ms.is_some() {
        recognized.push("polling");
    }
    if front_matter.workspace.root.is_some() {
        recognized.push("workspace");
    }
    if front_matter.hooks.after_create.is_some()
        || front_matter.hooks.before_run.is_some()
        || front_matter.hooks.after_run.is_some()
        || front_matter.hooks.before_remove.is_some()
    {
        recognized.push("hooks");
    }
    if front_matter.agent.max_concurrent_agents.is_some() || front_matter.agent.max_turns.is_some()
    {
        recognized.push("agent");
    }
    if front_matter.routing.harness.is_some() {
        recognized.push("routing");
    }
    if front_matter.openhands.transport.base_url.is_some() {
        recognized.push("openhands");
    }
    let hardcoded_clone_hooks = [
        ("after_create", front_matter.hooks.after_create.as_deref()),
        ("before_run", front_matter.hooks.before_run.as_deref()),
        ("after_run", front_matter.hooks.after_run.as_deref()),
        ("before_remove", front_matter.hooks.before_remove.as_deref()),
    ]
    .into_iter()
    .filter_map(|(name, value)| {
        value
            .filter(|value| value.to_ascii_lowercase().contains("git clone"))
            .map(|_| name)
    })
    .collect();
    MigrationReport {
        operation,
        source_config: source.source_config.clone(),
        target_repo: source.target_repo.clone(),
        workflow: source.workflow_path.clone(),
        target_config,
        central_config_already_active: false,
        preflight_only,
        config_generation: None,
        recognized_front_matter: recognized,
        hardcoded_clone_hooks,
        literal_secret_detected: source
            .workflow
            .front_matter
            .tracker
            .api_key
            .as_deref()
            .is_some_and(|value| !value.contains("${")),
        credential_bearing_remote_detected: remote_has_credentials(&source.remote),
        backup,
        activation_marker: None,
    }
}

fn generate_central_config(source: &SourceContext) -> Result<String, MigrationError> {
    if source
        .workflow
        .front_matter
        .tracker
        .api_key
        .as_deref()
        .is_some_and(|value| !value.contains("${"))
    {
        return Err(MigrationError::LiteralSecret);
    }
    if remote_has_credentials(&source.remote) {
        return Err(MigrationError::CredentialBearingRemote);
    }
    let project = source
        .workflow
        .front_matter
        .tracker
        .project_slug
        .clone()
        .unwrap_or_else(|| "legacy-project".to_owned());
    let target_branch = target_branch(&source.workflow.prompt_template);
    let workspace_root = source
        .workflow
        .front_matter
        .workspace
        .root
        .clone()
        .unwrap_or_else(|| "~/.opensymphony/workspaces/legacy-migrated".to_owned());
    let instruction_path = if source.target_repo.join("AGENTS.md").is_file() {
        "AGENTS.md"
    } else {
        "WORKFLOW.md"
    };
    let remote_locator = source.remote.clone();
    let linear_projects = BTreeMap::from([(
        project.clone(),
        json!({
            "provider_project_id": project.clone(),
            "repositories": ["legacy-repository"]
        }),
    )]);
    let max_concurrent_tasks = source
        .workflow
        .front_matter
        .agent
        .max_concurrent_agents
        .as_ref()
        .and_then(|value| integer_value(value).parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(10);
    let root = json!({
        "schema_version": 1,
        "instance": {
            "id": format!("legacy-{}", safe_id(source.target_repo.file_name().and_then(|name| name.to_str()).unwrap_or("repository"))),
            "state_root": "~/.opensymphony/state/legacy-migrated",
        },
        "routing": {
            "mode": "legacy_single",
            "repository": "legacy-repository",
            "harness": source.workflow.front_matter.routing.harness.clone(),
            "model": source.workflow.front_matter.routing.model.clone(),
            "model_profile": source.workflow.front_matter.routing.model_profile.clone(),
        },
        "tracker_profiles": {
            "legacy-linear": {
                "provider": "linear",
                "endpoint": source.workflow.front_matter.tracker.endpoint.clone().unwrap_or_else(|| "https://api.linear.app/graphql".to_owned()),
                "credential": "linear-api-key",
                "active_states": source.workflow.front_matter.tracker.active_states.clone().unwrap_or_else(|| vec!["Todo".to_owned(), "In Progress".to_owned(), "Rework".to_owned()]),
                "terminal_states": source.workflow.front_matter.tracker.terminal_states.clone().unwrap_or_else(|| vec!["Done".to_owned(), "Canceled".to_owned()]),
            }
        },
        "project_sets": {
            "legacy-project-set": {"tracker_profile": "legacy-linear", "projects": [project.clone()]}
        },
        "linear_projects": linear_projects,
        "repositories": {
            "legacy-repository": {
                "aliases": ["legacy"],
                "remote": {"provider": "git", "locator": remote_locator, "clone": source.remote.clone()},
                "target_branch": target_branch,
                "credential": "legacy-git",
                "review_profile": "legacy-review",
                "instructions": {"path": instruction_path},
                "checkout_path": source.target_repo.display().to_string(),
            }
        },
        "credentials": {
            "linear-api-key": {"kind": "environment", "variable": "LINEAR_API_KEY"},
            "legacy-git": {"kind": "ssh-agent"},
        },
        "review_profiles": {
            "legacy-review": {"provider": "git", "credential": "legacy-git", "required_checks": false, "required_review": false, "merge_method": "squash"}
        },
        "workspace": {"root": workspace_root, "retain_failed": true, "cleanup_after_parent_finalization": false},
        "scheduler": {
            "max_concurrent_tasks": max_concurrent_tasks,
            "max_turns": source.workflow.front_matter.agent.max_turns.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
            "max_retry_backoff_ms": source.workflow.front_matter.agent.max_retry_backoff_ms.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
            "stall_timeout_ms": source.workflow.front_matter.agent.stall_timeout_ms.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
            "poll_interval_ms": source.workflow.front_matter.polling.interval_ms.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
            "retry": {"max_attempts": 3}
        },
        "hooks": {
            "after_create": source.workflow.front_matter.hooks.after_create.clone(),
            "before_run": source.workflow.front_matter.hooks.before_run.clone(),
            "after_run": source.workflow.front_matter.hooks.after_run.clone(),
            "before_remove": source.workflow.front_matter.hooks.before_remove.clone(),
            "timeout_ms": source.workflow.front_matter.hooks.timeout_ms.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
        },
        "integration": {"policy": "builtin:legacy-single", "use_shared_git_worktrees": false},
        "memory": {
            "catalog_root": "~/.opensymphony/state/legacy-migrated/memory",
            "auto_capture": source.config.memory.auto_capture.unwrap_or(true),
            "auto_archive": source.config.memory.auto_archive.unwrap_or(false),
            "serve": source.config.memory.serve.unwrap_or_else(|| source.target_repo.join(".opensymphony/memory").is_dir()),
            "bind": source.config.memory.bind.clone(),
            "token_env": source.config.memory.token_env.clone(),
        },
        "control_plane": {"bind": source.config.control_plane.bind.clone().unwrap_or_else(|| "127.0.0.1:2468".to_owned())},
        "openhands": {
            "tool_dir": source.config.openhands.tool_dir.clone(),
            "transport_base_url": source.workflow.front_matter.openhands.transport.base_url.clone(),
            "transport_session_api_key_env": source.workflow.front_matter.openhands.transport.session_api_key_env.clone(),
        },
        "compatibility": {"allow_repo_local_config": false},
    });
    serde_yaml::to_string(&root).map_err(MigrationError::SerializeConfig)
}

fn workflow_body(source: &SourceContext) -> &[u8] {
    if source.workflow_source.trim_start().starts_with("---")
        && source.workflow_source.match_indices("---").count() >= 2
    {
        source.workflow.prompt_template.as_bytes()
    } else {
        source.workflow_source.as_bytes()
    }
}

fn target_branch(prompt: &str) -> String {
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix("Target branch:"))
        .map(|value| value.trim().trim_matches('`').to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "develop".to_owned())
}

fn integer_value(value: &crate::opensymphony_workflow::IntegerLike) -> String {
    match value {
        crate::opensymphony_workflow::IntegerLike::Integer(value) => value.to_string(),
        crate::opensymphony_workflow::IntegerLike::String(value) => value.clone(),
    }
}

fn git_remote(repo: &Path) -> Result<String, MigrationError> {
    let output = Command::new("git")
        .args([
            "-C",
            &repo.display().to_string(),
            "config",
            "--get",
            "remote.origin.url",
        ])
        .output()
        .map_err(|source| MigrationError::Read {
            path: repo.to_path_buf(),
            source,
        })?;
    let remote = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if remote.is_empty() {
        return Err(MigrationError::MissingRemote);
    }
    Ok(remote)
}

fn remote_has_credentials(value: &str) -> bool {
    if let Ok(url) = url::Url::parse(value) {
        return !url.username().is_empty() || url.password().is_some();
    }
    value
        .split_once('@')
        .is_some_and(|(user, host)| user != "git" || host.is_empty())
}

fn safe_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

fn stage_path(path: &Path, generation: &str) -> PathBuf {
    let suffix = generation.trim_start_matches("sha256:");
    PathBuf::from(format!("{}.staging-{suffix}", path.display()))
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), MigrationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| MigrationError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, contents).map_err(|source| MigrationError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn restore_file(backup: &Path, target: &Path) -> Result<(), MigrationError> {
    let contents = fs::read(backup).map_err(|source| MigrationError::Read {
        path: backup.to_path_buf(),
        source,
    })?;
    let stage = stage_path(target, &sha256(&contents));
    write_file(&stage, &contents)?;
    fs::rename(&stage, target).map_err(|source| MigrationError::Write {
        path: target.to_path_buf(),
        source,
    })
}

fn resolve_repo_path(base: &Path, value: &str) -> PathBuf {
    let value = if value == "~" {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
    } else if let Some(value) = value.strip_prefix("~/") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(value)
    } else {
        PathBuf::from(value)
    };
    absolute_path(base, &value)
}

fn absolute_path(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn current_dir() -> Result<PathBuf, MigrationError> {
    std::env::current_dir().map_err(|source| MigrationError::Read {
        path: PathBuf::from("."),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_never_include_literal_tracker_secrets() {
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  api_key: super-secret-canary\n  project_slug: project\n---\nbody\n",
        )
        .expect("workflow should parse");
        let source = SourceContext {
            source_config: PathBuf::from("config.yaml"),
            config_source: String::new(),
            target_repo: PathBuf::from("repo"),
            workflow_path: PathBuf::from("repo/WORKFLOW.md"),
            workflow_source: String::new(),
            workflow,
            config: LegacyConfigProbe {
                target_repo: None,
                control_plane: LegacyControlPlaneProbe::default(),
                openhands: LegacyOpenHandsProbe::default(),
                memory: LegacyMemoryProbe::default(),
            },
            remote: "git@github.com:example/repo.git".to_owned(),
        };

        let report = build_report("preflight", &source, None, None, true);
        let serialized = serde_json::to_string(&report).expect("report should serialize");
        assert!(report.literal_secret_detected);
        assert!(!serialized.contains("super-secret-canary"));
    }

    #[tokio::test]
    async fn apply_and_rollback_restore_legacy_files() {
        let root = tempfile::tempdir().expect("migration root should exist");
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.path())
            .status()
            .expect("git init should run");
        Command::new("git")
            .args(["remote", "add", "origin", "git@github.com:example/repo.git"])
            .current_dir(root.path())
            .status()
            .expect("git remote should be configured");
        let config_path = root.path().join("config.yaml");
        let workflow_path = root.path().join("WORKFLOW.md");
        let old_config = "control_plane:\n  bind: 127.0.0.1:2468\n";
        let old_workflow = "---\ntracker:\n  kind: linear\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n\n# Implementation instructions\n";
        fs::write(&config_path, old_config).expect("legacy config should be written");
        fs::write(&workflow_path, old_workflow).expect("legacy workflow should be written");

        let report = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: None,
        })
        .await
        .expect("migration should apply");
        assert!(report.activation_marker.is_some());
        let migrated_config =
            fs::read_to_string(&config_path).expect("central config should exist");
        let migrated_workflow = fs::read_to_string(&workflow_path).expect("workflow should exist");
        assert!(migrated_config.contains("legacy_single"));
        assert!(migrated_workflow.contains("Implementation instructions"));
        assert!(!migrated_config.contains("super-secret"));

        let strict_run_marker = root
            .path()
            .join(".opensymphony/migration/strict-run.active");
        fs::write(&strict_run_marker, "active\n").expect("strict run marker should be written");
        let blocked = rollback(RollbackArgs {
            config: Some(config_path.clone()),
        })
        .await
        .expect_err("rollback should be blocked by an active strict run");
        assert!(matches!(blocked, MigrationError::ActiveStrictRun { .. }));
        fs::remove_file(strict_run_marker).expect("strict run marker should be removed");

        let repeated = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: None,
        })
        .await
        .expect("repeat apply should be idempotent");
        assert!(repeated.central_config_already_active);

        rollback(RollbackArgs {
            config: Some(config_path.clone()),
        })
        .await
        .expect("rollback should restore the prior files");
        assert_eq!(
            fs::read_to_string(config_path).expect("config should restore"),
            old_config
        );
        assert_eq!(
            fs::read_to_string(workflow_path).expect("workflow should restore"),
            old_workflow
        );
    }
}
