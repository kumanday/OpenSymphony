use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::opensymphony_workflow::WorkflowDefinition;

use super::memory::{
    MemoryActivityStatus, acquire_memory_coordination_lock, memory_activity_marker_path,
    memory_activity_status, memory_lock_is_stale, memory_migration_lock_path,
};
use super::orchestrator_run::config::{
    CentralConfigError, load_central_config, looks_like_central_config,
    validate_central_config_text,
};

#[cfg(unix)]
use rustix::process::{Pid, test_kill_process};

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
    #[error("failed to expand legacy config value in {path}: {detail}")]
    ResolveConfig { path: PathBuf, detail: String },
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
    #[error("central config destination {path} is not an activation of this migration")]
    DestinationConflict { path: PathBuf },
    #[error("activated migration file changed after apply: {path}")]
    ActivatedFileChanged { path: PathBuf },
    #[error(
        "migrated memory catalog changed after apply; rollback is blocked to preserve it: {path}"
    )]
    MemoryCatalogChanged { path: PathBuf },
    #[error("workflow {path} does not declare an exact `Target branch:`")]
    MissingTargetBranch { path: PathBuf },
    #[error("workflow field {field} is not a valid unsigned integer: {value}")]
    InvalidNumericSetting { field: String, value: String },
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
    #[error("legacy memory writers are active at {path}; stop them before migration")]
    MemoryActive { path: PathBuf },
    #[error("memory migration is already active at {path}")]
    MemoryMigrationActive { path: PathBuf },
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
    #[serde(default)]
    source_config: PathBuf,
    config_path: PathBuf,
    workflow_path: PathBuf,
    backup_dir: PathBuf,
    generation: String,
    #[serde(default)]
    workflow_generation: String,
    had_config: bool,
    had_workflow: bool,
    #[serde(default)]
    config_mode: Option<u32>,
    #[serde(default)]
    workflow_mode: Option<u32>,
    #[serde(default)]
    memory_catalog_root: Option<PathBuf>,
    #[serde(default)]
    memory_catalog_generation: Option<String>,
}

impl ActivationMarker {
    fn target_repo(&self) -> PathBuf {
        self.workflow_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }
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

struct ActiveCentralConfig {
    source_config: PathBuf,
    target_config: PathBuf,
    target_repo: PathBuf,
    workflow: PathBuf,
    generation: String,
    activation_marker: PathBuf,
}

enum ActiveMigrationResolution {
    Complete,
    Restored,
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

async fn active_target_central_config(
    paths: &MigrationPaths,
) -> Result<Option<ActiveCentralConfig>, MigrationError> {
    let cwd = current_dir()?;
    let repo = absolute_path(&cwd, &paths.repo);
    ensure_memory_migration_inactive(&repo)?;
    let source_config = paths
        .config
        .as_ref()
        .map(|path| absolute_path(&cwd, path))
        .unwrap_or_else(|| repo.join("config.yaml"));
    let target_config = migration_target_config(paths, &cwd, &source_config);
    if !target_config.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&target_config).map_err(|source| MigrationError::Read {
        path: target_config.clone(),
        source,
    })?;
    if !looks_like_central_config(&raw) {
        return Ok(None);
    }
    let marker = load_activation_marker(&target_config)?;
    if target_config != source_config && marker.is_none() {
        return Err(MigrationError::DestinationConflict {
            path: target_config,
        });
    }
    if target_config != source_config
        && let Some((_, marker)) = marker.as_ref()
    {
        let source = load_source(paths)?;
        let source_matches = marker.config_path == target_config
            && marker.workflow_path == source.workflow_path
            && (marker.source_config.as_os_str().is_empty()
                || marker.source_config == source.source_config);
        if !source_matches {
            return Err(MigrationError::DestinationConflict {
                path: target_config,
            });
        }
    }
    let central = load_central_config(&target_config).await?;
    let (activation_marker, marker) = marker.map_or(
        (migration_marker_path(&target_config), None),
        |(path, marker)| (path, Some(marker)),
    );
    let (target_repo, workflow) = marker
        .map(|marker| (marker.target_repo(), marker.workflow_path))
        .unwrap_or_else(|| (repo.clone(), repo.join("WORKFLOW.md")));
    ensure_memory_migration_inactive(&target_repo)?;
    Ok(Some(ActiveCentralConfig {
        source_config,
        target_config,
        target_repo,
        workflow,
        generation: central.generation,
        activation_marker,
    }))
}

fn active_report(
    operation: &'static str,
    active: &ActiveCentralConfig,
    preflight_only: bool,
) -> MigrationReport {
    MigrationReport {
        operation,
        source_config: active.source_config.clone(),
        target_repo: active.target_repo.clone(),
        workflow: active.workflow.clone(),
        target_config: Some(active.target_config.clone()),
        central_config_already_active: true,
        preflight_only,
        config_generation: Some(active.generation.clone()),
        recognized_front_matter: Vec::new(),
        hardcoded_clone_hooks: Vec::new(),
        literal_secret_detected: false,
        credential_bearing_remote_detected: false,
        backup: None,
        activation_marker: Some(active.activation_marker.clone()),
    }
}

fn migration_target_config(paths: &MigrationPaths, cwd: &Path, source: &Path) -> PathBuf {
    let target = paths
        .output
        .as_deref()
        .map(|path| absolute_path(cwd, path))
        .unwrap_or_else(|| source.to_path_buf());
    normalize_path(&target)
}

fn migration_root(target_config: &Path) -> PathBuf {
    normalize_path(target_config)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".opensymphony/migration")
}

pub(crate) fn strict_run_marker_path(target_config: &Path) -> PathBuf {
    let target_config = normalize_path(target_config);
    let key = sha256(target_config.display().to_string().as_bytes());
    migration_root(&target_config).join(format!(
        "strict-run-{}.active",
        &key.trim_start_matches("sha256:")[..16]
    ))
}

pub(crate) struct StrictRunMarkerGuard {
    path: PathBuf,
}

impl Drop for StrictRunMarkerGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub(crate) fn claim_strict_run_marker(
    target_config: &Path,
    generation: &str,
) -> std::io::Result<StrictRunMarkerGuard> {
    let marker = strict_run_marker_path(target_config);
    if marker.exists() && !strict_run_marker_owner_alive(&marker) {
        match fs::remove_file(&marker) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)?;
    if let Err(error) = writeln!(file, "pid={}\ngeneration={generation}", std::process::id()) {
        let _ = fs::remove_file(&marker);
        return Err(error);
    }
    Ok(StrictRunMarkerGuard { path: marker })
}

fn strict_run_marker_owner_alive(marker: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(marker) else {
        return true;
    };
    let Some(pid) = contents
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.trim().parse::<i32>().ok())
    else {
        return true;
    };

    #[cfg(unix)]
    {
        let Some(pid) = Pid::from_raw(pid) else {
            return true;
        };
        match test_kill_process(pid) {
            Ok(()) => true,
            Err(error) if error == rustix::io::Errno::SRCH => false,
            Err(_) => true,
        }
    }
    #[cfg(not(unix))]
    {
        let Ok(output) = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
        else {
            return true;
        };
        if !output.status.success() {
            return true;
        }
        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u32>().ok())
                == Some(pid as u32)
        })
    }
}

fn migration_marker_path(target_config: &Path) -> PathBuf {
    let target_config = normalize_path(target_config);
    let key = sha256(target_config.display().to_string().as_bytes());
    migration_root(&target_config).join(format!(
        "activation-{}.yaml",
        &key.trim_start_matches("sha256:")[..16]
    ))
}

fn parse_activation_marker(path: &Path) -> Result<ActivationMarker, MigrationError> {
    let marker_raw = fs::read_to_string(path).map_err(|source| MigrationError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_yaml::from_str::<ActivationMarker>(&marker_raw).map_err(|source| {
        MigrationError::ParseActivation {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn load_activation_marker(
    target_config: &Path,
) -> Result<Option<(PathBuf, ActivationMarker)>, MigrationError> {
    let target_path = migration_marker_path(target_config);
    if target_path.is_file() {
        return Ok(Some((
            target_path.clone(),
            parse_activation_marker(&target_path)?,
        )));
    }

    // Keep reading the pre-namespace marker for one-way compatibility, but never
    // let a marker for another central config control this target's rollback.
    let legacy_path = migration_root(target_config).join("activation.yaml");
    if !legacy_path.is_file() {
        return Ok(None);
    }
    let marker = parse_activation_marker(&legacy_path)?;
    if marker.config_path == target_config {
        Ok(Some((legacy_path, marker)))
    } else {
        Ok(None)
    }
}

fn resume_partial_apply(
    active: &ActiveCentralConfig,
) -> Result<ActiveMigrationResolution, MigrationError> {
    let Some((marker_path, marker)) = load_activation_marker(&active.target_config)? else {
        return Ok(ActiveMigrationResolution::Complete);
    };
    let workflow_stage = stage_path(&marker.workflow_path, &marker.generation);
    if workflow_stage.is_file() {
        let staged_workflow = fs::read(&workflow_stage).map_err(|source| MigrationError::Read {
            path: workflow_stage.clone(),
            source,
        })?;
        if marker.workflow_generation.is_empty()
            || sha256(&staged_workflow) != marker.workflow_generation
        {
            restore_or_remove_after_failed_apply(
                &marker.config_path,
                &marker.backup_dir.join("config.yaml"),
                marker.had_config,
                marker.config_mode,
            )?;
            restore_or_remove_after_failed_apply(
                &marker.workflow_path,
                &marker.backup_dir.join("WORKFLOW.md"),
                marker.had_workflow,
                marker.workflow_mode,
            )?;
            remove_staged_files(&[
                &stage_path(&marker.config_path, &marker.generation),
                &workflow_stage,
                &stage_path(&marker_path, &marker.generation),
            ]);
            fs::remove_file(&marker_path).map_err(|source| MigrationError::Write {
                path: marker_path,
                source,
            })?;
            return Ok(ActiveMigrationResolution::Restored);
        }
        replace_staged_file(&workflow_stage, &marker.workflow_path)?;
        return Ok(ActiveMigrationResolution::Complete);
    }

    let workflow_source =
        fs::read_to_string(&marker.workflow_path).map_err(|source| MigrationError::Read {
            path: marker.workflow_path.clone(),
            source,
        })?;
    if !workflow_has_orchestration_front_matter(&workflow_source) {
        return Ok(ActiveMigrationResolution::Complete);
    }

    restore_or_remove_after_failed_apply(
        &marker.config_path,
        &marker.backup_dir.join("config.yaml"),
        marker.had_config,
        marker.config_mode,
    )?;
    restore_or_remove_after_failed_apply(
        &marker.workflow_path,
        &marker.backup_dir.join("WORKFLOW.md"),
        marker.had_workflow,
        marker.workflow_mode,
    )?;
    fs::remove_file(&marker_path).map_err(|source| MigrationError::Write {
        path: marker_path,
        source,
    })?;
    Ok(ActiveMigrationResolution::Restored)
}

async fn preflight(paths: MigrationPaths) -> Result<MigrationReport, MigrationError> {
    if let Some(active) = active_target_central_config(&paths).await? {
        return Ok(active_report("preflight", &active, true));
    }
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
    let target_config = migration_target_config(&paths, &current_dir()?, &source.source_config);
    let report = build_report(
        "preflight",
        &source,
        Some(target_config.clone()),
        None,
        true,
    );
    if report.literal_secret_detected || report.credential_bearing_remote_detected {
        return Ok(report);
    }
    let generated = generate_central_config(&source)?;
    validate_central_config_text(&target_config, &generated)?;
    Ok(report)
}

async fn apply(paths: MigrationPaths) -> Result<MigrationReport, MigrationError> {
    let _ = preflight(paths.clone()).await?;
    if let Some(active) = active_target_central_config(&paths).await? {
        return match resume_partial_apply(&active)? {
            ActiveMigrationResolution::Complete => Ok(active_report("apply", &active, false)),
            ActiveMigrationResolution::Restored => {
                // The interrupted generation was safely restored.  Continue this
                // invocation from the legacy source so repeat apply completes the
                // migration instead of returning a false success.
                let _ = preflight(paths.clone()).await?;
                apply_legacy_source(paths).await
            }
        };
    }
    apply_legacy_source(paths).await
}

async fn apply_legacy_source(paths: MigrationPaths) -> Result<MigrationReport, MigrationError> {
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

    let _memory_lock = acquire_memory_migration_lock(&source.target_repo)?;
    let cwd = current_dir()?;
    let target_config = migration_target_config(&paths, &cwd, &source.source_config);
    let generated = generate_central_config(&source)?;
    let generation = sha256(generated.as_bytes());
    let migration_root = migration_root(&target_config);
    let backup_dir = migration_root
        .join("backups")
        .join(generation.trim_start_matches("sha256:"));
    fs::create_dir_all(&backup_dir).map_err(|source_error| MigrationError::Write {
        path: backup_dir.clone(),
        source: source_error,
    })?;
    let had_config = target_config.is_file();
    let had_workflow = source.workflow_path.is_file();
    let config_mode = file_mode(&target_config).map_err(|source_error| MigrationError::Read {
        path: target_config.clone(),
        source: source_error,
    })?;
    let workflow_mode =
        file_mode(&source.workflow_path).map_err(|source_error| MigrationError::Read {
            path: source.workflow_path.clone(),
            source: source_error,
        })?;
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
    write_file_with_mode(&central_stage, generated.as_bytes(), config_mode)?;
    let central = load_central_config(&central_stage).await?;
    preserve_legacy_memory(&source.target_repo, central.memory_catalog_root.as_deref())?;
    let memory_catalog_root = central.memory_catalog_root.clone();
    let memory_catalog_generation = memory_catalog_root
        .as_deref()
        .map(memory_catalog_generation)
        .transpose()?;
    let workflow_body = workflow_body(&source)?;
    write_file_with_mode(&workflow_stage, &workflow_body, workflow_mode)?;

    let marker = ActivationMarker {
        source_config: source.source_config.clone(),
        config_path: target_config.clone(),
        workflow_path: source.workflow_path.clone(),
        backup_dir: backup_dir.clone(),
        generation: generation.clone(),
        workflow_generation: sha256(&workflow_body),
        had_config,
        had_workflow,
        config_mode,
        workflow_mode,
        memory_catalog_root,
        memory_catalog_generation,
    };
    let marker_path = migration_marker_path(&target_config);
    let marker_stage = stage_path(&marker_path, &generation);
    let marker_raw = serde_yaml::to_string(&marker).map_err(MigrationError::SerializeConfig)?;
    write_file(&marker_stage, marker_raw.as_bytes())?;

    // Publish the activation record before replacing either runnable file.  A
    // process interruption after this point leaves the recoverable backup and
    // marker available to rollback rather than an untracked mixed generation.
    if let Err(error) = replace_staged_file(&marker_stage, &marker_path) {
        remove_staged_files(&[&central_stage, &workflow_stage, &marker_stage]);
        return Err(error);
    }
    if let Err(error) = replace_staged_file(&central_stage, &target_config) {
        return recover_failed_apply(
            &marker_path,
            &[&central_stage, &workflow_stage, &marker_stage],
            error,
            vec![restore_or_remove_after_failed_apply(
                &target_config,
                &backup_dir.join("config.yaml"),
                had_config,
                config_mode,
            )],
        );
    }
    if let Err(error) = replace_staged_file(&workflow_stage, &source.workflow_path) {
        return recover_failed_apply(
            &marker_path,
            &[&central_stage, &workflow_stage, &marker_stage],
            error,
            vec![
                restore_or_remove_after_failed_apply(
                    &target_config,
                    &backup_dir.join("config.yaml"),
                    had_config,
                    config_mode,
                ),
                restore_or_remove_after_failed_apply(
                    &source.workflow_path,
                    &backup_dir.join("WORKFLOW.md"),
                    had_workflow,
                    workflow_mode,
                ),
            ],
        );
    }

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
    let Some((marker_path, marker)) = load_activation_marker(&config_path)? else {
        return Err(MigrationError::MissingActivation {
            path: migration_marker_path(&config_path),
        });
    };
    let active_run_marker = strict_run_marker_path(&config_path);
    let _strict_run_marker = match claim_strict_run_marker(&config_path, "rollback") {
        Ok(marker) => marker,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(MigrationError::ActiveStrictRun {
                path: active_run_marker,
            });
        }
        Err(source) => {
            return Err(MigrationError::Write {
                path: active_run_marker,
                source,
            });
        }
    };

    let _memory_lock = acquire_memory_migration_lock(&marker.target_repo())?;
    verify_activated_files(&marker)?;
    if let (Some(root), Some(expected)) = (
        marker.memory_catalog_root.as_deref(),
        marker.memory_catalog_generation.as_deref(),
    ) && memory_catalog_generation(root)? != expected
    {
        return Err(MigrationError::MemoryCatalogChanged {
            path: root.to_path_buf(),
        });
    }

    // Restore the repository workflow first. The central config remains
    // active while this succeeds, so an interrupted rollback still leaves a
    // runnable central generation instead of a stripped legacy workflow.
    if marker.had_workflow {
        restore_file(
            &marker.backup_dir.join("WORKFLOW.md"),
            &marker.workflow_path,
            marker.workflow_mode,
        )?;
    }
    if marker.had_config {
        restore_file(
            &marker.backup_dir.join("config.yaml"),
            &marker.config_path,
            marker.config_mode,
        )?;
    } else if marker.config_path.is_file() {
        fs::remove_file(&marker.config_path).map_err(|source| MigrationError::Write {
            path: marker.config_path.clone(),
            source,
        })?;
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

fn verify_activated_files(marker: &ActivationMarker) -> Result<(), MigrationError> {
    verify_activated_file(
        &marker.config_path,
        &marker.generation,
        &marker.backup_dir.join("config.yaml"),
        marker.had_config,
    )?;
    if !marker.workflow_generation.is_empty() {
        verify_activated_file(
            &marker.workflow_path,
            &marker.workflow_generation,
            &marker.backup_dir.join("WORKFLOW.md"),
            marker.had_workflow,
        )?;
    }
    Ok(())
}

fn verify_activated_file(
    path: &Path,
    generation: &str,
    backup_path: &Path,
    had_original: bool,
) -> Result<(), MigrationError> {
    let Ok(contents) = fs::read(path) else {
        if !had_original {
            return Ok(());
        }
        return Err(MigrationError::ActivatedFileChanged {
            path: path.to_path_buf(),
        });
    };
    let current_generation = sha256(&contents);
    if current_generation == generation {
        return Ok(());
    }
    if had_original
        && fs::read(backup_path)
            .ok()
            .is_some_and(|backup| sha256(&backup) == current_generation)
    {
        return Ok(());
    }
    Err(MigrationError::ActivatedFileChanged {
        path: path.to_path_buf(),
    })
}

fn memory_catalog_generation(root: &Path) -> Result<String, MigrationError> {
    if !root.exists() {
        return Ok(sha256(b"<absent>"));
    }
    let mut entries = Vec::new();
    collect_memory_catalog_entries(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut digest_input = Vec::new();
    for (relative, kind, contents) in entries {
        digest_input.extend_from_slice(relative.as_bytes());
        digest_input.push(0);
        digest_input.push(kind);
        digest_input.push(0);
        digest_input.extend_from_slice(&contents);
        digest_input.push(0);
    }
    Ok(sha256(&digest_input))
}

fn collect_memory_catalog_entries(
    root: &Path,
    current: &Path,
    entries: &mut Vec<(String, u8, Vec<u8>)>,
) -> Result<(), MigrationError> {
    let read_dir = fs::read_dir(current).map_err(|source| MigrationError::Read {
        path: current.to_path_buf(),
        source,
    })?;
    for entry in read_dir {
        let entry = entry.map_err(|source| MigrationError::Read {
            path: current.to_path_buf(),
            source,
        })?;
        if entry.file_name() == super::memory::MEMORY_ACTIVITY_MARKER {
            continue;
        }
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        let file_type = entry.file_type().map_err(|source| MigrationError::Read {
            path: path.clone(),
            source,
        })?;
        if file_type.is_symlink() {
            return Err(MigrationError::Write {
                path,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "memory catalog contains a symlink",
                ),
            });
        }
        if file_type.is_dir() {
            entries.push((relative, b'd', Vec::new()));
            collect_memory_catalog_entries(root, &path, entries)?;
        } else if file_type.is_file() {
            let contents = fs::read(&path).map_err(|source| MigrationError::Read {
                path: path.clone(),
                source,
            })?;
            entries.push((relative, b'f', contents));
        }
    }
    Ok(())
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
        .map(|path| {
            super::expand_env_tokens(path)
                .map_err(|error| MigrationError::ResolveConfig {
                    path: source_config.clone(),
                    detail: error.to_string(),
                })
                .map(|path| resolve_repo_path(config_root, &path))
        })
        .transpose()?
        .unwrap_or(repo);
    ensure_memory_migration_inactive(&target_repo)?;
    ensure_legacy_memory_quiescent(&target_repo)?;
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
            .filter(|value| hook_creates_repository(value))
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
            .is_some_and(|value| credential_variable(value).is_none()),
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
        .is_some_and(|value| credential_variable(value).is_none())
    {
        return Err(MigrationError::LiteralSecret);
    }
    if remote_has_credentials(&source.remote) {
        return Err(MigrationError::CredentialBearingRemote);
    }
    let linear_credential_variable =
        tracker_credential_variable(source.workflow.front_matter.tracker.api_key.as_deref())?;
    let project = source
        .workflow
        .front_matter
        .tracker
        .project_slug
        .clone()
        .unwrap_or_else(|| "legacy-project".to_owned());
    let target_branch = target_branch(&source.workflow.prompt_template).ok_or_else(|| {
        MigrationError::MissingTargetBranch {
            path: source.workflow_path.clone(),
        }
    })?;
    let instance_id = format!(
        "legacy-{}-{}",
        safe_id(
            source
                .target_repo
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("repository")
        ),
        &sha256(source.target_repo.display().to_string().as_bytes())[7..23]
    );
    let workspace_root = match source.workflow.front_matter.workspace.root.as_deref() {
        Some(value) => {
            let value =
                super::expand_env_tokens(value).map_err(|error| MigrationError::ResolveConfig {
                    path: source.workflow_path.clone(),
                    detail: format!("workspace.root: {error}"),
                })?;
            let resolved = resolve_repo_path(&source.target_repo, &value);
            if paths_overlap(&resolved, &source.target_repo) {
                format!("~/.opensymphony/workspaces/{instance_id}")
            } else {
                resolved.display().to_string()
            }
        }
        None => format!("~/.opensymphony/workspaces/{instance_id}"),
    };
    let instruction_path = if source.target_repo.join("AGENTS.md").is_file() {
        "AGENTS.md"
    } else {
        "WORKFLOW.md"
    };
    let remote_locator = source.remote.clone();
    let linear_projects = BTreeMap::from([(
        project.clone(),
        json!({
            "provider_project_id": source
                .workflow
                .front_matter
                .tracker
                .project_id
                .clone()
                .unwrap_or_else(|| project.clone()),
            "provider_project_slug": project.clone(),
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
    let state_root = format!("~/.opensymphony/state/{instance_id}");
    let max_concurrent_agents_by_state = source
        .workflow
        .front_matter
        .agent
        .max_concurrent_agents_by_state
        .as_ref()
        .map(|limits| {
            limits
                .iter()
                .map(|(state, value)| {
                    integer_value(value)
                        .parse::<u64>()
                        .map(|value| (state.clone(), value))
                        .map_err(|_| MigrationError::InvalidNumericSetting {
                            field: format!("agent.max_concurrent_agents_by_state.{state}"),
                            value: integer_value(value),
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let control_plane_bind = expand_legacy_bind(
        source.config.control_plane.bind.as_deref(),
        &source.source_config,
        "control_plane.bind",
    )?;
    let memory_bind = expand_legacy_bind(
        source.config.memory.bind.as_deref(),
        &source.source_config,
        "memory.bind",
    )?;
    let memory_token_env = expand_legacy_bind(
        source.config.memory.token_env.as_deref(),
        &source.source_config,
        "memory.token_env",
    )?;
    let root = json!({
        "schema_version": 1,
        "instance": {
            "id": instance_id,
            "state_root": state_root,
        },
        "routing": {
            "mode": "legacy_single",
            "repository": "legacy-repository",
            "harness": source.workflow.front_matter.routing.harness.clone(),
            "model": source.workflow.front_matter.routing.model.clone(),
            "model_profile": source.workflow.front_matter.routing.model_profile.clone(),
            "harness_env": source.workflow.front_matter.routing.harness_env.clone(),
            "model_env": source.workflow.front_matter.routing.model_env.clone(),
            "model_profile_env": source.workflow.front_matter.routing.model_profile_env.clone(),
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
            "linear-api-key": {"kind": "environment", "variable": linear_credential_variable},
            "legacy-git": {"kind": "ssh-agent"},
        },
        "review_profiles": {
            "legacy-review": {"provider": "git", "credential": "legacy-git", "required_checks": false, "required_review": false, "merge_method": "squash"}
        },
        "workspace": {"root": workspace_root, "retain_failed": true, "cleanup_after_parent_finalization": false},
        "scheduler": {
            "max_concurrent_tasks": max_concurrent_tasks,
            "max_concurrent_agents_by_state": max_concurrent_agents_by_state,
            "max_turns": source.workflow.front_matter.agent.max_turns.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
            "max_retry_backoff_ms": source.workflow.front_matter.agent.max_retry_backoff_ms.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
            "stall_timeout_ms": source.workflow.front_matter.agent.stall_timeout_ms.as_ref().and_then(migrated_stall_timeout_ms),
            "poll_interval_ms": source.workflow.front_matter.polling.interval_ms.as_ref().and_then(|value| integer_value(value).parse::<u64>().ok()),
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
            "catalog_root": format!("{state_root}/memory"),
            "auto_capture": source.config.memory.auto_capture.unwrap_or(true),
            "auto_archive": source.config.memory.auto_archive.unwrap_or(false),
            "serve": source.config.memory.serve.unwrap_or_else(|| source.target_repo.join(".opensymphony/memory/memory.yaml").is_file()),
            "bind": memory_bind,
            "token_env": memory_token_env,
        },
        "control_plane": {"bind": control_plane_bind.unwrap_or_else(|| "127.0.0.1:2468".to_owned())},
        "openhands": {
            "tool_dir": expand_legacy_bind(
                source.config.openhands.tool_dir.as_deref(),
                &source.source_config,
                "openhands.tool_dir",
            )?
            .map(|value| {
                resolve_repo_path(
                    source.source_config.parent().unwrap_or_else(|| Path::new(".")),
                    &value,
                )
                .display()
                .to_string()
            }),
            "transport_base_url": source.workflow.front_matter.openhands.transport.base_url.clone(),
            "transport_session_api_key_env": source.workflow.front_matter.openhands.transport.session_api_key_env.clone(),
            "front_matter": source.workflow.front_matter.openhands.clone(),
        },
        "compatibility": {"allow_repo_local_config": false},
    });
    serde_yaml::to_string(&root).map_err(MigrationError::SerializeConfig)
}

fn workflow_has_front_matter(source: &str) -> bool {
    source.trim_start().starts_with("---") && source.match_indices("---").count() >= 2
}

fn workflow_has_orchestration_front_matter(source: &str) -> bool {
    if !workflow_has_front_matter(source) {
        return false;
    }
    let Ok(workflow) = WorkflowDefinition::parse(source) else {
        return true;
    };
    let front_matter = workflow.front_matter;
    front_matter.tracker != Default::default()
        || front_matter.polling != Default::default()
        || front_matter.workspace != Default::default()
        || front_matter.hooks != Default::default()
        || front_matter.agent != Default::default()
        || front_matter.openhands != Default::default()
        || front_matter.routing != Default::default()
}

fn workflow_body(source: &SourceContext) -> Result<Vec<u8>, MigrationError> {
    if !workflow_has_front_matter(&source.workflow_source) {
        return Ok(source.workflow_source.as_bytes().to_vec());
    }

    let mut local_front_matter = BTreeMap::<String, serde_yaml::Value>::new();
    if let Some(codex) = source.workflow.front_matter.codex.as_ref() {
        local_front_matter.insert(
            "codex".to_owned(),
            serde_yaml::to_value(codex).map_err(MigrationError::SerializeConfig)?,
        );
    }
    if let Some(logging) = source.workflow.front_matter.logging.as_ref() {
        local_front_matter.insert(
            "logging".to_owned(),
            serde_yaml::to_value(logging).map_err(MigrationError::SerializeConfig)?,
        );
    }
    for (name, value) in &source.workflow.front_matter.extensions {
        local_front_matter.insert(name.clone(), value.clone());
    }

    if local_front_matter.is_empty() {
        if source
            .workflow
            .prompt_template
            .trim_start()
            .starts_with("---")
        {
            let mut body = String::from("---\n---\n\n");
            body.push_str(&source.workflow.prompt_template);
            return Ok(body.into_bytes());
        }
        return Ok(source.workflow.prompt_template.as_bytes().to_vec());
    }

    let serialized =
        serde_yaml::to_string(&local_front_matter).map_err(MigrationError::SerializeConfig)?;
    let mut body = String::from("---\n");
    body.push_str(&serialized);
    body.push_str("---\n\n");
    body.push_str(&source.workflow.prompt_template);
    Ok(body.into_bytes())
}

fn target_branch(prompt: &str) -> Option<String> {
    prompt
        .lines()
        .find_map(|line| line.trim().strip_prefix("Target branch:"))
        .map(|value| value.trim().trim_matches('`').to_owned())
        .filter(|value| !value.is_empty())
}

fn integer_value(value: &crate::opensymphony_workflow::IntegerLike) -> String {
    match value {
        crate::opensymphony_workflow::IntegerLike::Integer(value) => value.to_string(),
        crate::opensymphony_workflow::IntegerLike::String(value) => value.trim().to_owned(),
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn preserve_legacy_memory(
    target_repo: &Path,
    central_memory_root: Option<&Path>,
) -> Result<(), MigrationError> {
    ensure_legacy_memory_quiescent(target_repo)?;
    let Some(destination) = central_memory_root else {
        return Ok(());
    };
    let source = target_repo.join(".opensymphony/memory");
    if !source.is_dir() || paths_overlap(&source, destination) {
        return Ok(());
    }
    let has_entries = fs::read_dir(&source)
        .map_err(|source_error| MigrationError::Read {
            path: source.clone(),
            source: source_error,
        })?
        .next()
        .is_some();
    if !has_entries {
        return Ok(());
    }
    copy_directory_contents(&source, destination)
}

fn ensure_legacy_memory_quiescent(target_repo: &Path) -> Result<(), MigrationError> {
    let memory_root = target_repo.join(".opensymphony/memory");
    let marker = memory_activity_marker_path(&memory_root);
    match memory_activity_status(&memory_root).map_err(|source| MigrationError::Read {
        path: marker.clone(),
        source,
    })? {
        MemoryActivityStatus::Absent | MemoryActivityStatus::Stale => {}
        MemoryActivityStatus::Live => return Err(MigrationError::MemoryActive { path: marker }),
    }
    Ok(())
}

fn ensure_memory_migration_inactive(target_repo: &Path) -> Result<(), MigrationError> {
    let path = memory_migration_lock_path(target_repo);
    if path.exists() && !memory_lock_is_stale(target_repo) {
        return Err(MigrationError::MemoryMigrationActive { path });
    }
    Ok(())
}

struct MemoryMigrationLock {
    _lock: super::memory::MemoryCoordinationLock,
}

fn acquire_memory_migration_lock(
    target_repo: &Path,
) -> Result<MemoryMigrationLock, MigrationError> {
    let path = memory_migration_lock_path(target_repo);
    let lock = acquire_memory_coordination_lock(target_repo).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            MigrationError::MemoryMigrationActive { path: path.clone() }
        } else {
            MigrationError::Write {
                path: path.clone(),
                source,
            }
        }
    })?;
    if let Err(error) = ensure_legacy_memory_quiescent(target_repo) {
        drop(lock);
        return Err(error);
    }
    let memory_root = target_repo.join(".opensymphony/memory");
    let marker = memory_activity_marker_path(&memory_root);
    if matches!(
        memory_activity_status(&memory_root).map_err(|source| MigrationError::Read {
            path: marker.clone(),
            source,
        })?,
        MemoryActivityStatus::Stale
    ) {
        fs::remove_file(&marker).map_err(|source| MigrationError::Write {
            path: marker,
            source,
        })?;
    }
    Ok(MemoryMigrationLock { _lock: lock })
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    validate_directory_contents(source, destination)?;
    copy_directory_contents_unchecked(source, destination)
}

fn validate_directory_contents(source: &Path, destination: &Path) -> Result<(), MigrationError> {
    let entries = fs::read_dir(source).map_err(|source_error| MigrationError::Read {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source_error| MigrationError::Read {
            path: source.to_path_buf(),
            source: source_error,
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|source_error| MigrationError::Read {
                path: source_path.clone(),
                source: source_error,
            })?;
        if file_type.is_symlink() {
            return Err(MigrationError::Write {
                path: source_path,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "legacy memory store contains a symlink",
                ),
            });
        }
        if file_type.is_dir() {
            match fs::symlink_metadata(&destination_path) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() =>
                {
                    validate_directory_contents(&source_path, &destination_path)?;
                }
                Ok(_) => {
                    return Err(MigrationError::Write {
                        path: destination_path,
                        source: std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "legacy memory directory conflicts with an existing central entry",
                        ),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    validate_directory_contents(&source_path, &destination_path)?;
                }
                Err(source_error) => {
                    return Err(MigrationError::Read {
                        path: destination_path,
                        source: source_error,
                    });
                }
            }
        } else if file_type.is_file() {
            match fs::symlink_metadata(&destination_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(metadata)
                    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
                {
                    let source_contents =
                        fs::read(&source_path).map_err(|source_error| MigrationError::Read {
                            path: source_path.clone(),
                            source: source_error,
                        })?;
                    let destination_contents =
                        fs::read(&destination_path).map_err(|source_error| {
                            MigrationError::Read {
                                path: destination_path.clone(),
                                source: source_error,
                            }
                        })?;
                    if source_contents != destination_contents {
                        return Err(MigrationError::Write {
                            path: destination_path,
                            source: std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                "legacy memory file conflicts with an existing central catalog entry",
                            ),
                        });
                    }
                }
                Ok(_) => {
                    return Err(MigrationError::Write {
                        path: destination_path,
                        source: std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "legacy memory file conflicts with an existing central entry",
                        ),
                    });
                }
                Err(source_error) => {
                    return Err(MigrationError::Read {
                        path: destination_path,
                        source: source_error,
                    });
                }
            }
        }
    }
    Ok(())
}

fn copy_directory_contents_unchecked(
    source: &Path,
    destination: &Path,
) -> Result<(), MigrationError> {
    fs::create_dir_all(destination).map_err(|source_error| MigrationError::Write {
        path: destination.to_path_buf(),
        source: source_error,
    })?;
    let entries = fs::read_dir(source).map_err(|source_error| MigrationError::Read {
        path: source.to_path_buf(),
        source: source_error,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source_error| MigrationError::Read {
            path: source.to_path_buf(),
            source: source_error,
        })?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|source_error| MigrationError::Read {
                path: source_path.clone(),
                source: source_error,
            })?;
        if file_type.is_symlink() {
            return Err(MigrationError::Write {
                path: source_path,
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "legacy memory store contains a symlink",
                ),
            });
        }
        if file_type.is_dir() {
            match fs::symlink_metadata(&destination_path) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() =>
                {
                    copy_directory_contents_unchecked(&source_path, &destination_path)?;
                }
                Ok(_) => {
                    return Err(MigrationError::Write {
                        path: destination_path,
                        source: std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "legacy memory directory conflicts with an existing central entry",
                        ),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    copy_directory_contents_unchecked(&source_path, &destination_path)?;
                }
                Err(source_error) => {
                    return Err(MigrationError::Read {
                        path: destination_path,
                        source: source_error,
                    });
                }
            }
        } else if file_type.is_file() {
            match fs::symlink_metadata(&destination_path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::copy(&source_path, &destination_path).map_err(|source_error| {
                        MigrationError::Write {
                            path: destination_path.clone(),
                            source: source_error,
                        }
                    })?;
                }
                Ok(metadata)
                    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
                {
                    let source_contents =
                        fs::read(&source_path).map_err(|source_error| MigrationError::Read {
                            path: source_path.clone(),
                            source: source_error,
                        })?;
                    let destination_contents =
                        fs::read(&destination_path).map_err(|source_error| {
                            MigrationError::Read {
                                path: destination_path.clone(),
                                source: source_error,
                            }
                        })?;
                    if source_contents != destination_contents {
                        return Err(MigrationError::Write {
                            path: destination_path,
                            source: std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                "legacy memory file conflicts with an existing central catalog entry",
                            ),
                        });
                    }
                }
                Ok(_) => {
                    return Err(MigrationError::Write {
                        path: destination_path,
                        source: std::io::Error::new(
                            std::io::ErrorKind::AlreadyExists,
                            "legacy memory file conflicts with an existing central entry",
                        ),
                    });
                }
                Err(source_error) => {
                    return Err(MigrationError::Read {
                        path: destination_path,
                        source: source_error,
                    });
                }
            }
        }
    }
    Ok(())
}

fn migrated_stall_timeout_ms(value: &crate::opensymphony_workflow::IntegerLike) -> Option<u64> {
    let parsed = integer_value(value).parse::<i64>().ok()?;
    Some(if parsed <= 0 { 0 } else { parsed as u64 })
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
    if value.contains('?') || value.contains('#') {
        return true;
    }
    if let Ok(url) = url::Url::parse(value) {
        let conventional_ssh_user =
            url.scheme().eq_ignore_ascii_case("ssh") && url.username().eq_ignore_ascii_case("git");
        return (!url.username().is_empty() && !conventional_ssh_user)
            || url.password().is_some()
            || !url.query().unwrap_or_default().is_empty()
            || !url.fragment().unwrap_or_default().is_empty();
    }
    value
        .split_once('@')
        .is_some_and(|(user, host)| user != "git" || host.is_empty())
}

fn hook_creates_repository(value: &str) -> bool {
    let normalized = value
        .replace("\\\r\n", " ")
        .replace("\\\n", " ")
        .replace("&&", ";")
        .replace("||", ";")
        .to_ascii_lowercase();
    normalized
        .split([';', '\n'])
        .map(|command| {
            command
                .split_whitespace()
                .map(|token| {
                    token.trim_matches(|character: char| {
                        matches!(character, '&' | '|' | ';' | '(' | ')' | '\'' | '"' | '`')
                    })
                })
                .filter(|token| !token.is_empty())
                .collect::<Vec<_>>()
        })
        .any(|tokens| {
            let git_index = tokens
                .iter()
                .position(|token| token.rsplit('/').next().is_some_and(|name| name == "git"));
            if let Some(index) = git_index {
                return tokens[index + 1..]
                    .iter()
                    .any(|token| matches!(*token, "clone" | "init"));
            }
            if tokens
                .iter()
                .position(|token| token.rsplit('/').next().is_some_and(|name| name == "gh"))
                .is_some_and(|index| {
                    tokens.get(index + 1..index + 3) == Some(["repo", "clone"].as_slice())
                })
            {
                return true;
            }

            let command = tokens
                .first()
                .and_then(|token| token.rsplit('/').next())
                .unwrap_or_default();
            match command {
                "hg" | "svn" | "bzr" | "darcs" | "fossil" | "pijul" => tokens
                    .iter()
                    .skip(1)
                    .any(|token| matches!(*token, "clone" | "checkout" | "co" | "branch" | "get")),
                "cp" => tokens
                    .iter()
                    .any(|token| matches!(*token, "-a" | "-r" | "-R" | "--archive")),
                "rsync" => true,
                _ => false,
            }
        })
}

fn expand_legacy_bind(
    value: Option<&str>,
    path: &Path,
    field: &str,
) -> Result<Option<String>, MigrationError> {
    value
        .map(super::expand_env_tokens)
        .transpose()
        .map_err(|error| MigrationError::ResolveConfig {
            path: path.to_path_buf(),
            detail: format!("{field}: {error}"),
        })
}

fn tracker_credential_variable(value: Option<&str>) -> Result<String, MigrationError> {
    let Some(value) = value else {
        return Ok("LINEAR_API_KEY".to_owned());
    };
    let Some(variable) = credential_variable(value) else {
        return Err(MigrationError::LiteralSecret);
    };
    Ok(variable.to_owned())
}

fn credential_variable(value: &str) -> Option<&str> {
    let variable = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'))?;
    if variable.is_empty()
        || !variable.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
        || !variable
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase() || character == '_')
    {
        return None;
    }
    Some(variable)
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
    write_file_with_mode(path, contents, None)
}

fn write_file_with_mode(
    path: &Path,
    contents: &[u8],
    mode: Option<u32>,
) -> Result<(), MigrationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| MigrationError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, contents).map_err(|source| MigrationError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    set_file_mode(path, mode).map_err(|source| MigrationError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn restore_file(backup: &Path, target: &Path, mode: Option<u32>) -> Result<(), MigrationError> {
    let contents = fs::read(backup).map_err(|source| MigrationError::Read {
        path: backup.to_path_buf(),
        source,
    })?;
    let stage = stage_path(target, &sha256(&contents));
    write_file_with_mode(&stage, &contents, mode)?;
    replace_staged_file(&stage, target)
}

fn replace_staged_file(stage: &Path, target: &Path) -> Result<(), MigrationError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing_name: *const u16, new_name: *const u16, flags: u32) -> i32;
        }

        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        let existing = stage
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let replacement = target
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // MoveFileExW replaces the destination in one filesystem operation;
        // unlike remove-then-rename it cannot leave a runnable file absent.
        let replaced = unsafe {
            MoveFileExW(
                existing.as_ptr(),
                replacement.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if replaced == 0 {
            return Err(MigrationError::Write {
                path: target.to_path_buf(),
                source: std::io::Error::last_os_error(),
            });
        }
        return Ok(());
    }
    fs::rename(stage, target).map_err(|source| MigrationError::Write {
        path: target.to_path_buf(),
        source,
    })
}

fn remove_staged_files(paths: &[&Path]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn restore_or_remove_after_failed_apply(
    target: &Path,
    backup: &Path,
    had_original: bool,
    mode: Option<u32>,
) -> Result<(), MigrationError> {
    if had_original {
        restore_file(backup, target, mode)
    } else if target.is_file() {
        fs::remove_file(target).map_err(|source| MigrationError::Write {
            path: target.to_path_buf(),
            source,
        })
    } else {
        Ok(())
    }
}

fn recover_failed_apply(
    marker_path: &Path,
    staged_files: &[&Path],
    original_error: MigrationError,
    restorations: Vec<Result<(), MigrationError>>,
) -> Result<MigrationReport, MigrationError> {
    remove_staged_files(staged_files);
    if let Some(error) = restorations.into_iter().find_map(Result::err) {
        // Keep the marker so a later rollback can retry recovery after the
        // filesystem problem is resolved.
        return Err(error);
    }
    fs::remove_file(marker_path).map_err(|source| MigrationError::Write {
        path: marker_path.to_path_buf(),
        source,
    })?;
    Err(original_error)
}

fn resolve_repo_path(base: &Path, value: &str) -> PathBuf {
    let value = if value == "~" {
        super::open_user_home_dir().unwrap_or_default()
    } else if let Some(value) = value.strip_prefix("~/") {
        super::open_user_home_dir().unwrap_or_default().join(value)
    } else {
        PathBuf::from(value)
    };
    let path = absolute_path(base, &value);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn file_mode(path: &Path) -> Result<Option<u32>, std::io::Error> {
    if !path.is_file() {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(Some(fs::metadata(path)?.permissions().mode()))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}

fn set_file_mode(path: &Path, mode: Option<u32>) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    let _ = (path, mode);
    Ok(())
}

fn absolute_path(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
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

    #[test]
    fn migration_keeps_delimiter_leading_prompt_outside_front_matter() {
        let workflow_source = "---\ntracker:\n  kind: linear\n  project_slug: project\n---\n\n---\nTarget branch: develop\n---\n";
        let workflow =
            WorkflowDefinition::parse(workflow_source).expect("legacy workflow should parse");
        let source = SourceContext {
            source_config: PathBuf::from("config.yaml"),
            config_source: String::new(),
            target_repo: PathBuf::from("repo"),
            workflow_path: PathBuf::from("repo/WORKFLOW.md"),
            workflow_source: workflow_source.to_string(),
            workflow,
            config: LegacyConfigProbe {
                target_repo: None,
                control_plane: LegacyControlPlaneProbe::default(),
                openhands: LegacyOpenHandsProbe::default(),
                memory: LegacyMemoryProbe::default(),
            },
            remote: "git@github.com:example/repo.git".to_owned(),
        };

        let migrated = String::from_utf8(workflow_body(&source).expect("body should serialize"))
            .expect("migrated workflow should be UTF-8");
        assert!(
            migrated.starts_with("---\n---\n\n")
                && migrated.contains("---\nTarget branch: develop"),
            "migrated workflow: {migrated:?}"
        );
        WorkflowDefinition::parse(&migrated)
            .expect("delimiter-leading migrated prompt should remain loadable");
    }

    #[test]
    fn migration_accepts_braced_and_unbraced_credential_references() {
        assert_eq!(
            tracker_credential_variable(Some("${LINEAR_API_KEY}"))
                .expect("braced references should be supported"),
            "LINEAR_API_KEY"
        );
        assert_eq!(
            tracker_credential_variable(Some("$LINEAR_API_KEY"))
                .expect("unbraced references should be supported"),
            "LINEAR_API_KEY"
        );
        assert!(tracker_credential_variable(Some("literal-secret")).is_err());
    }

    #[test]
    fn activation_markers_use_normalized_destination_paths() {
        let dotted = Path::new("/tmp/coe-547/./config.yaml");
        let normalized = Path::new("/tmp/coe-547/config.yaml");
        assert_eq!(
            migration_marker_path(dotted),
            migration_marker_path(normalized)
        );
    }

    #[test]
    fn strict_run_markers_reclaim_stale_owners_and_are_destination_namespaced() {
        let root = tempfile::tempdir().expect("marker root should exist");
        let first = root.path().join("first.yaml");
        let second = root.path().join("second.yaml");
        assert_ne!(
            strict_run_marker_path(&first),
            strict_run_marker_path(&second)
        );

        let marker = strict_run_marker_path(&first);
        fs::create_dir_all(marker.parent().expect("marker parent should exist"))
            .expect("marker parent should be created");
        fs::write(&marker, "pid=2000000000\ngeneration=stale\n")
            .expect("stale marker should be written");
        let guard = claim_strict_run_marker(&first, "generation").expect("stale marker claim");
        assert!(marker.is_file());
        drop(guard);
        assert!(!marker.exists());
    }

    #[test]
    fn rollback_verification_accepts_the_backed_up_partial_generation() {
        let root = tempfile::tempdir().expect("verification root should exist");
        let current = root.path().join("config.yaml");
        let backup = root.path().join("backup.yaml");
        fs::write(&current, "legacy\n").expect("legacy file should be written");
        fs::write(&backup, "legacy\n").expect("backup file should be written");
        verify_activated_file(&current, &sha256(b"central\n"), &backup, true)
            .expect("the backed-up generation should be accepted during rollback");
        fs::write(&current, "unexpected\n").expect("changed file should be written");
        assert!(matches!(
            verify_activated_file(&current, &sha256(b"central\n"), &backup, true),
            Err(MigrationError::ActivatedFileChanged { .. })
        ));
    }

    #[test]
    fn migration_copies_existing_legacy_memory_without_overwriting_destination() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let source = root.path().join("repo/.opensymphony/memory");
        let destination = root.path().join("state/memory");
        fs::create_dir_all(&source).expect("legacy memory root should exist");
        fs::create_dir_all(source.join("issues")).expect("issue directory should exist");
        fs::write(source.join("issues/COE-1.md"), "capsule\n")
            .expect("legacy capsule should be written");
        preserve_legacy_memory(&root.path().join("repo"), Some(&destination))
            .expect("legacy memory should be copied");
        assert_eq!(
            fs::read_to_string(destination.join("issues/COE-1.md"))
                .expect("copied capsule should exist"),
            "capsule\n"
        );
        preserve_legacy_memory(&root.path().join("repo"), Some(&destination))
            .expect("identical repeat preservation should be safe");
        fs::write(destination.join("issues/COE-1.md"), "newer\n")
            .expect("destination capsule should be writable");
        assert!(preserve_legacy_memory(&root.path().join("repo"), Some(&destination)).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("issues/COE-1.md"))
                .expect("destination capsule should remain"),
            "newer\n"
        );
    }

    #[test]
    fn migration_preflights_all_legacy_memory_conflicts_before_copying() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let source = repo.join(".opensymphony/memory");
        let destination = root.path().join("state/memory");
        fs::create_dir_all(source.join("nested")).expect("legacy memory tree should exist");
        fs::write(source.join("first.md"), "first\n").expect("first capsule should exist");
        fs::write(source.join("nested/second.md"), "second\n")
            .expect("second capsule should exist");
        fs::create_dir_all(destination.join("nested")).expect("destination tree should exist");
        fs::write(destination.join("nested/second.md"), "conflicting\n")
            .expect("conflicting capsule should exist");

        assert!(preserve_legacy_memory(&repo, Some(&destination)).is_err());
        assert!(
            !destination.join("first.md").exists(),
            "a late conflict must not leave earlier entries copied"
        );
    }

    #[test]
    fn migration_rejects_active_legacy_memory_writer_before_copying() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let source = repo.join(".opensymphony/memory");
        let destination = root.path().join("state/memory");
        fs::create_dir_all(&source).expect("legacy memory root should exist");
        fs::write(
            memory_activity_marker_path(&source),
            format!("pid={}\n", std::process::id()),
        )
        .expect("activity marker should be written");
        fs::write(source.join("issue.md"), "capsule\n").expect("legacy capsule should exist");

        let error = preserve_legacy_memory(&repo, Some(&destination))
            .expect_err("active memory writers must block migration");
        assert!(matches!(error, MigrationError::MemoryActive { .. }));
        assert!(
            !destination.exists(),
            "blocked migration must not copy memory"
        );
    }

    #[test]
    fn migration_lock_is_exclusive_and_released_after_apply_scope() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let first = acquire_memory_migration_lock(&repo).expect("first lock should succeed");
        assert!(matches!(
            acquire_memory_migration_lock(&repo),
            Err(MigrationError::MemoryMigrationActive { .. })
        ));
        drop(first);
        let second = acquire_memory_migration_lock(&repo).expect("released lock should succeed");
        drop(second);
        assert!(!memory_migration_lock_path(&repo).exists());
    }

    #[test]
    fn migration_reclaims_a_stale_memory_lock() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let lock_path = memory_migration_lock_path(&repo);
        fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("lock parent");
        fs::write(&lock_path, "pid=2000000000\n").expect("stale lock should be written");

        let lock = acquire_memory_migration_lock(&repo).expect("stale lock should be reclaimed");
        drop(lock);
        assert!(!lock_path.exists());
    }

    #[test]
    fn clone_hook_detection_covers_git_and_gh_variants() {
        assert!(hook_creates_repository(
            "git -c protocol.version=2 clone URL ."
        ));
        assert!(hook_creates_repository("gh repo \\\nclone owner/repo ."));
        assert!(hook_creates_repository("git \\\nclone URL ."));
        assert!(hook_creates_repository("cd repo && git clone URL ."));
        assert!(hook_creates_repository("/usr/bin/git clone URL ."));
        assert!(hook_creates_repository(
            "env GIT_SSH_COMMAND=ssh git clone URL ."
        ));
        assert!(hook_creates_repository("sh -c 'git clone URL .'"));
        assert!(!hook_creates_repository("npm init -y"));
        assert!(!hook_creates_repository("cargo init --name clone"));
        assert!(!hook_creates_repository("printf '%s\\n' ready"));
        assert!(hook_creates_repository(
            "hg clone https://example.invalid/repo ."
        ));
        assert!(hook_creates_repository("cp -a /other/repository/. ."));
        assert!(hook_creates_repository("rsync -a /other/repository/ ."));
    }

    #[test]
    fn migration_relocates_repo_relative_workspace_roots() {
        assert!(paths_overlap(
            Path::new("/repo/var/workspaces"),
            Path::new("/repo")
        ));
        assert!(!paths_overlap(
            Path::new("/var/workspaces"),
            Path::new("/repo")
        ));
        assert_eq!(
            integer_value(&crate::opensymphony_workflow::IntegerLike::String(
                " 4 ".to_owned()
            )),
            "4"
        );

        let target_repo = PathBuf::from("/repo");
        let workflow_path = target_repo.join("WORKFLOW.md");
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\nworkspace:\n  root: ./var/workspaces\n---\n\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            target_repo,
            workflow_path,
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
        let generated = generate_central_config(&source).expect("migration should generate");
        assert!(!generated.contains("/repo/var/workspaces"));
        assert!(generated.contains("~/.opensymphony/workspaces/legacy-repo-"));
    }

    #[test]
    fn migration_preserves_linear_project_id_separately_from_slug() {
        let target_repo = PathBuf::from("/repo");
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_id: immutable-project-id\n  project_slug: human-project-slug\n---\n\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            workflow_path: target_repo.join("WORKFLOW.md"),
            workflow_source: String::new(),
            target_repo,
            workflow,
            config: LegacyConfigProbe {
                target_repo: None,
                control_plane: LegacyControlPlaneProbe::default(),
                openhands: LegacyOpenHandsProbe::default(),
                memory: LegacyMemoryProbe::default(),
            },
            remote: "git@github.com:example/repo.git".to_owned(),
        };

        let generated = generate_central_config(&source).expect("migration should generate");
        let generated: serde_yaml::Value =
            serde_yaml::from_str(&generated).expect("generated config should parse");
        let project = &generated["linear_projects"]["human-project-slug"];
        assert_eq!(
            project["provider_project_id"].as_str(),
            Some("immutable-project-id")
        );
        assert_eq!(
            project["provider_project_slug"].as_str(),
            Some("human-project-slug")
        );
    }

    #[test]
    fn migration_preserves_supported_credential_indirection() {
        assert_eq!(
            tracker_credential_variable(Some("${MY_LINEAR_TOKEN}"))
                .expect("custom credential variable should migrate"),
            "MY_LINEAR_TOKEN"
        );
        assert!(matches!(
            tracker_credential_variable(Some("literal-secret")),
            Err(MigrationError::LiteralSecret)
        ));
    }

    #[test]
    fn migration_preserves_disabled_negative_stall_timeout() {
        assert_eq!(
            migrated_stall_timeout_ms(&crate::opensymphony_workflow::IntegerLike::String(
                "-1".to_owned()
            )),
            Some(0)
        );
        assert_eq!(
            migrated_stall_timeout_ms(&crate::opensymphony_workflow::IntegerLike::Integer(-1)),
            Some(0)
        );
    }

    #[test]
    fn migration_rejects_query_credentials_in_remotes() {
        assert!(remote_has_credentials(
            "https://example.com/repo.git?access_token=secret"
        ));
        assert!(remote_has_credentials(
            "git@github.com:example/repo.git#secret"
        ));
        assert!(!remote_has_credentials(
            "ssh://git@github.com/example/repo.git"
        ));
        assert!(remote_has_credentials(
            "ssh://deploy@github.com/example/repo.git"
        ));
        assert!(!remote_has_credentials("git@github.com:example/repo.git"));
    }

    #[tokio::test]
    async fn preflight_reports_redacted_unsafe_inputs_without_writing() {
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
        fs::write(
            root.path().join("WORKFLOW.md"),
            "---\ntracker:\n  kind: linear\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\n  api_key: secret-canary\n---\n\nTarget branch: develop\n",
        )
        .expect("workflow should be written");

        let report = preflight(MigrationPaths {
            config: None,
            repo: root.path().to_path_buf(),
            output: None,
        })
        .await
        .expect("unsafe preflight should return a report");
        let serialized = serde_json::to_string(&report).expect("report should serialize");
        assert!(report.literal_secret_detected);
        assert!(!report.credential_bearing_remote_detected);
        assert!(!serialized.contains("secret-canary"));
        assert!(!root.path().join("config.yaml").exists());
        assert!(!root.path().join(".opensymphony").exists());
    }

    #[tokio::test]
    async fn apply_and_rollback_restore_legacy_files() {
        use std::io::Write;

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
        let output_path = root.path().join("central/central.yaml");
        let workflow_path = root.path().join("WORKFLOW.md");
        let old_config = "control_plane:\n  bind: 127.0.0.1:2468\nopenhands:\n  tool_dir: ./managed-tools\nmemory:\n  token_env: MEMORY_TOKEN_ENV\n";
        let old_workflow = "---\ntracker:\n  kind: linear\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\nworkspace:\n  root: ../.legacy-workspaces\nagent:\n  max_concurrent_agents_by_state:\n    In Progress: 1\nrouting:\n  harness_env: CUSTOM_HARNESS\n  model_env: CUSTOM_MODEL\n  model_profile_env: CUSTOM_MODEL_PROFILE\nopenhands:\n  local_server:\n    enabled: true\n    command: [custom-openhands]\n  conversation:\n    agent:\n      llm:\n        model: custom/model\n        api_key_env: CUSTOM_OPENAI_KEY\n  websocket:\n    reconnect_max_ms: 9876\ncodex:\n  command: codex app-server\nlogging:\n  level: debug\nrepository_local:\n  preserve: true\n---\n\nTarget branch: develop\n\n# Implementation instructions\n";
        let old_workflow = old_workflow.replace("repository_local:\n  preserve: true\n", "");
        fs::write(&config_path, old_config).expect("legacy config should be written");
        fs::write(&workflow_path, &old_workflow).expect("legacy workflow should be written");
        fs::create_dir_all(root.path().join(".opensymphony/memory"))
            .expect("legacy memory directory should be created");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))
                .expect("config permissions should be set");
            fs::set_permissions(&workflow_path, fs::Permissions::from_mode(0o640))
                .expect("workflow permissions should be set");
        }

        let report = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: Some(output_path.clone()),
        })
        .await
        .expect("migration should apply");
        assert!(report.activation_marker.is_some());
        let migrated_config =
            fs::read_to_string(&output_path).expect("central config should exist");
        assert_eq!(
            fs::read_to_string(&config_path).expect("legacy source config should remain"),
            old_config
        );
        let migrated_workflow = fs::read_to_string(&workflow_path).expect("workflow should exist");
        assert!(migrated_config.contains("legacy_single"));
        assert!(migrated_config.contains("custom-openhands"));
        assert!(migrated_config.contains("CUSTOM_OPENAI_KEY"));
        assert!(migrated_config.contains("CUSTOM_HARNESS"));
        assert!(migrated_config.contains("CUSTOM_MODEL_PROFILE"));
        assert!(migrated_config.contains("MEMORY_TOKEN_ENV"));
        assert!(migrated_config.contains(&root.path().join("managed-tools").display().to_string()));
        assert!(!migrated_config.contains("max_attempts"));
        assert!(migrated_config.contains("serve: false"));
        assert!(migrated_config.contains("reconnect_max_ms"));
        assert!(migrated_config.contains("max_concurrent_agents_by_state"));
        assert!(
            migrated_config.contains(
                &root
                    .path()
                    .parent()
                    .expect("temporary root should have a parent")
                    .join(".legacy-workspaces")
                    .display()
                    .to_string()
            )
        );
        assert!(migrated_workflow.contains("Implementation instructions"));
        assert!(migrated_workflow.contains("codex:"));
        assert!(migrated_workflow.contains("logging:"));
        assert!(!migrated_config.contains("super-secret"));

        let marker_path = report
            .activation_marker
            .clone()
            .expect("migration should publish an activation marker");
        let marker = parse_activation_marker(&marker_path).expect("activation marker should parse");
        fs::write(
            stage_path(&workflow_path, &marker.generation),
            "truncated staged workflow",
        )
        .expect("corrupt staged workflow should be written");
        let resumed_corrupt = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: Some(output_path.clone()),
        })
        .await
        .expect("mismatched staged workflow should recover safely");
        assert!(!resumed_corrupt.central_config_already_active);
        assert!(
            fs::read_to_string(&workflow_path)
                .expect("recovered workflow should exist")
                .contains("codex:")
        );
        restore_file(
            &marker.backup_dir.join("WORKFLOW.md"),
            &workflow_path,
            marker.workflow_mode,
        )
        .expect("partial apply should restore the legacy workflow");
        let resumed = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: Some(output_path.clone()),
        })
        .await
        .expect("repeat apply should recover a partially published migration");
        assert!(!resumed.central_config_already_active);
        assert!(
            fs::read_to_string(&workflow_path)
                .expect("recovered workflow should exist")
                .contains("codex:")
        );

        let conflicting_output = root.path().join("central/conflicting.yaml");
        fs::copy(&output_path, &conflicting_output)
            .expect("conflicting central destination should be copied");
        let conflict = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: Some(conflicting_output),
        })
        .await
        .expect_err("unmarked central destination should conflict");
        assert!(matches!(
            conflict,
            MigrationError::DestinationConflict { .. }
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&config_path)
                    .expect("migrated config should exist")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&workflow_path)
                    .expect("migrated workflow should exist")
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
        }

        let strict_run_marker = strict_run_marker_path(&output_path);
        fs::create_dir_all(
            strict_run_marker
                .parent()
                .expect("strict run marker should have a parent"),
        )
        .expect("migration marker directory should exist");
        fs::write(
            &strict_run_marker,
            format!("pid={}\ngeneration=active\n", std::process::id()),
        )
        .expect("strict run marker should be written");
        let blocked = rollback(RollbackArgs {
            config: Some(output_path.clone()),
        })
        .await
        .expect_err("rollback should be blocked by an active strict run");
        assert!(matches!(blocked, MigrationError::ActiveStrictRun { .. }));
        fs::remove_file(strict_run_marker).expect("strict run marker should be removed");

        let repeated = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: Some(output_path.clone()),
        })
        .await
        .expect("repeat apply should be idempotent");
        assert!(repeated.central_config_already_active);

        let active_config = fs::read(&output_path).expect("active config should be readable");
        fs::OpenOptions::new()
            .append(true)
            .open(&output_path)
            .expect("active config should be writable")
            .write_all(b"# changed after activation\n")
            .expect("active config should be changed");
        let changed = rollback(RollbackArgs {
            config: Some(output_path.clone()),
        })
        .await
        .expect_err("rollback should refuse changed activated files");
        assert!(matches!(
            changed,
            MigrationError::ActivatedFileChanged { .. }
        ));
        fs::write(&output_path, active_config).expect("active config should be restored");

        let memory_root = marker
            .memory_catalog_root
            .clone()
            .expect("generated migrations should record the central memory root");
        fs::create_dir_all(&memory_root).expect("central memory root should be creatable");
        fs::write(memory_root.join("post-migration.md"), "new evidence\n")
            .expect("post-migration memory should be writable");
        let changed_memory = rollback(RollbackArgs {
            config: Some(output_path.clone()),
        })
        .await
        .expect_err("rollback must preserve post-migration memory");
        assert!(matches!(
            changed_memory,
            MigrationError::MemoryCatalogChanged { .. }
        ));
        assert!(
            output_path.is_file(),
            "blocked rollback must keep central config"
        );
        fs::remove_dir_all(&memory_root).expect("test memory catalog should be removed");

        rollback(RollbackArgs {
            config: Some(output_path.clone()),
        })
        .await
        .expect("rollback should restore the prior files");
        assert_eq!(
            fs::read_to_string(&config_path).expect("config should restore"),
            old_config
        );
        assert!(
            !output_path.is_file(),
            "separate central output should be removed on rollback"
        );
        assert_eq!(
            fs::read_to_string(&workflow_path).expect("workflow should restore"),
            old_workflow
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&config_path)
                    .expect("config should restore")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&workflow_path)
                    .expect("workflow should restore")
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
            );
        }
    }
}
