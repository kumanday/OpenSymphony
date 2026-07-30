use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::opensymphony_workflow::{
    DEFAULT_WORKSPACE_ROOT, WorkflowDefinition, WorkflowFrontMatter,
};

use super::memory::{
    MEMORY_MIGRATION_LOCK, MemoryActivityStatus, acquire_memory_coordination_lock,
    memory_activity_marker_path, memory_activity_status, memory_lock_is_stale,
    memory_migration_lock_path,
};
use super::orchestrator_run::config::{
    CentralConfigError, load_central_config, looks_like_central_config,
    validate_central_config_text,
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
    #[error("migration cannot relocate existing legacy workspace state at {path}")]
    WorkspaceRootState { path: PathBuf },
    #[error("migration cannot preserve a credential-bearing repository remote")]
    CredentialBearingRemote,
    #[error("migration refuses symlinked input {path}")]
    SymlinkInput { path: PathBuf },
    #[error("central config destination {path} is not an activation of this migration")]
    DestinationConflict { path: PathBuf },
    #[error("activated migration file changed after apply: {path}")]
    ActivatedFileChanged { path: PathBuf },
    #[error("legacy migration source changed during apply: {path}")]
    LegacySourceChanged { path: PathBuf },
    #[error("migration backup changed after apply: {path}")]
    BackupChanged { path: PathBuf },
    #[error("activation marker does not record a backup generation for {path}")]
    MissingBackupGeneration { path: PathBuf },
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
    #[error("legacy orchestrator owns the workspace root at {path}")]
    RuntimeActive { path: PathBuf },
    #[error("failed to acquire the legacy runtime root lock at {path}: {detail}")]
    RuntimeLock { path: PathBuf, detail: String },
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
    #[serde(default)]
    memory_catalog_copy_in_progress: bool,
    #[serde(default)]
    legacy_workspace_root: Option<PathBuf>,
    #[serde(default)]
    backup_config_generation: Option<String>,
    #[serde(default)]
    backup_workflow_generation: Option<String>,
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
    source_config_present: bool,
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
    enforce_quiescence: bool,
) -> Result<Option<ActiveCentralConfig>, MigrationError> {
    let cwd = current_dir()?;
    let repo = absolute_path(&cwd, &paths.repo);
    if enforce_quiescence {
        ensure_memory_migration_inactive(&repo)?;
    }
    let source_config = paths
        .config
        .as_ref()
        .map(|path| absolute_path(&cwd, path))
        .unwrap_or_else(|| repo.join("config.yaml"));
    let target_config = migration_target_config(paths, &cwd, &source_config);
    let activation = load_activation_marker(&target_config)?;
    let marker_source = if activation.is_some() && target_config != source_config {
        Some(load_source(paths, enforce_quiescence)?)
    } else {
        None
    };
    if let Some((_, marker)) = activation.as_ref() {
        let target_repo = marker_source
            .as_ref()
            .map(|source| source.target_repo.as_path())
            .unwrap_or(repo.as_path());
        let expected_memory_catalog_root =
            central_memory_catalog_root_for_marker(&target_config, marker).await?;
        validate_activation_marker(
            &target_config,
            marker,
            target_repo,
            marker_source
                .as_ref()
                .map(|source| source.source_config.as_path()),
            expected_memory_catalog_root.as_deref(),
        )?;
    }
    if let Some((activation_marker, marker)) = activation.as_ref()
        && marker.memory_catalog_copy_in_progress
    {
        if enforce_quiescence {
            ensure_memory_migration_inactive(&marker.target_repo())?;
        }
        return Ok(Some(ActiveCentralConfig {
            source_config: marker.source_config.clone(),
            target_config: target_config.clone(),
            target_repo: marker.target_repo(),
            workflow: marker.workflow_path.clone(),
            generation: marker.generation.clone(),
            activation_marker: activation_marker.clone(),
        }));
    }
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
    let marker = activation;
    if target_config != source_config && marker.is_none() {
        return Err(MigrationError::DestinationConflict {
            path: target_config,
        });
    }
    if target_config != source_config
        && let Some((_, marker)) = marker.as_ref()
    {
        let source = marker_source
            .as_ref()
            .expect("active marker source is loaded");
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
    if enforce_quiescence {
        ensure_memory_migration_inactive(&target_repo)?;
    }
    Ok(Some(ActiveCentralConfig {
        source_config,
        target_config,
        target_repo,
        workflow,
        generation: central.generation,
        activation_marker,
    }))
}

async fn central_memory_catalog_root_for_marker(
    target_config: &Path,
    marker: &ActivationMarker,
) -> Result<Option<PathBuf>, MigrationError> {
    let staged_config = stage_path(&marker.config_path, &marker.generation);
    let active = if target_config.is_file() {
        let raw = fs::read_to_string(target_config).map_err(|source| MigrationError::Read {
            path: target_config.to_path_buf(),
            source,
        })?;
        if looks_like_central_config(&raw) {
            Some(load_central_config(target_config).await?)
        } else {
            None
        }
    } else {
        None
    };
    let config_path = if active
        .as_ref()
        .is_some_and(|config| config.generation == marker.generation)
    {
        target_config.to_path_buf()
    } else if staged_config.is_file() {
        staged_config
    } else if active.is_some() {
        target_config.to_path_buf()
    } else {
        return Ok(None);
    };
    Ok(load_central_config(&config_path).await?.memory_catalog_root)
}

fn validate_activation_marker(
    target_config: &Path,
    marker: &ActivationMarker,
    target_repo: &Path,
    expected_source_config: Option<&Path>,
    expected_memory_catalog_root: Option<&Path>,
) -> Result<(), MigrationError> {
    let target_config = canonicalize_destination(target_config);
    if canonicalize_destination(&marker.config_path) != target_config {
        return Err(MigrationError::DestinationConflict {
            path: marker.config_path.clone(),
        });
    }

    let target_repo = canonicalize_destination(target_repo);
    let expected_workflow = target_repo.join("WORKFLOW.md");
    if canonicalize_destination(&marker.workflow_path)
        != canonicalize_destination(&expected_workflow)
    {
        return Err(MigrationError::DestinationConflict {
            path: marker.workflow_path.clone(),
        });
    }

    if let Some(expected_source_config) = expected_source_config {
        if canonicalize_destination(&marker.source_config)
            != canonicalize_destination(expected_source_config)
        {
            return Err(MigrationError::DestinationConflict {
                path: marker.source_config.clone(),
            });
        }
    } else if !marker.source_config.as_os_str().is_empty()
        && marker
            .source_config
            .parent()
            .is_none_or(|parent| canonicalize_destination(parent) != target_repo)
    {
        return Err(MigrationError::DestinationConflict {
            path: marker.source_config.clone(),
        });
    }

    let Some(generation) = marker.generation.strip_prefix("sha256:") else {
        return Err(MigrationError::DestinationConflict {
            path: marker.config_path.clone(),
        });
    };
    if generation.len() != 64 || !generation.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(MigrationError::DestinationConflict {
            path: marker.config_path.clone(),
        });
    }
    let expected_backup = migration_root(&target_config)
        .join("backups")
        .join(generation);
    if normalize_path(&marker.backup_dir) != normalize_path(&expected_backup) {
        return Err(MigrationError::DestinationConflict {
            path: marker.backup_dir.clone(),
        });
    }
    reject_symlink_ancestors(&marker.backup_dir)?;

    if let Some(expected_memory_catalog_root) = expected_memory_catalog_root {
        if marker
            .memory_catalog_root
            .as_deref()
            .map(canonicalize_destination)
            != Some(canonicalize_destination(expected_memory_catalog_root))
        {
            return Err(MigrationError::DestinationConflict {
                path: marker
                    .memory_catalog_root
                    .clone()
                    .unwrap_or_else(|| expected_memory_catalog_root.to_path_buf()),
            });
        }
    } else if let Some(memory_catalog_root) = marker.memory_catalog_root.as_ref() {
        return Err(MigrationError::DestinationConflict {
            path: memory_catalog_root.clone(),
        });
    }

    if let Some(workspace_root) = marker.legacy_workspace_root.as_deref() {
        let expected_workspace_root = legacy_workspace_root_from_backup(marker, &target_repo)?;
        if canonicalize_destination(workspace_root)
            != canonicalize_destination(&expected_workspace_root)
        {
            return Err(MigrationError::DestinationConflict {
                path: workspace_root.to_path_buf(),
            });
        }
    }
    Ok(())
}

fn legacy_workspace_root_from_backup(
    marker: &ActivationMarker,
    target_repo: &Path,
) -> Result<PathBuf, MigrationError> {
    let configured = if marker.had_workflow {
        let path = marker.backup_dir.join("WORKFLOW.md");
        let source = fs::read_to_string(&path).map_err(|source| MigrationError::Read {
            path: path.clone(),
            source,
        })?;
        WorkflowDefinition::parse(&source)
            .map_err(|source| MigrationError::ParseWorkflow { path, source })?
            .front_matter
            .workspace
            .root
            .unwrap_or_else(|| DEFAULT_WORKSPACE_ROOT.to_owned())
    } else {
        DEFAULT_WORKSPACE_ROOT.to_owned()
    };
    let configured =
        super::expand_env_tokens(&configured).map_err(|error| MigrationError::ResolveConfig {
            path: marker.workflow_path.clone(),
            detail: format!("workspace.root: {error}"),
        })?;
    Ok(resolve_repo_path(target_repo, &configured))
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
    canonicalize_destination(&target)
}

fn migration_root(target_config: &Path) -> PathBuf {
    canonicalize_destination(target_config)
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".opensymphony/migration")
}

pub(crate) fn strict_run_marker_path(target_config: &Path) -> PathBuf {
    let target_config =
        fs::canonicalize(target_config).unwrap_or_else(|_| normalize_path(target_config));
    let key = sha256(target_config.display().to_string().as_bytes());
    migration_root(&target_config).join(format!(
        "strict-run-{}.active",
        &key.trim_start_matches("sha256:")[..16]
    ))
}

static STRICT_STALE_MARKER_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct StrictRunMarkerGuard {
    path: PathBuf,
}

impl StrictRunMarkerGuard {
    pub(crate) fn update_generation(&self, generation: &str) -> std::io::Result<()> {
        let sequence = STRICT_STALE_MARKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        remove_stale_strict_generation_staging(&self.path)?;
        let temporary = strict_generation_stage_path(&self.path, sequence);
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let owner = super::orchestrator_run::process_marker_fields();
            writeln!(file, "{owner}generation={generation}")?;
            replace_staged_file(&temporary, &self.path)
                .map_err(|error| std::io::Error::other(error.to_string()))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn strict_generation_stage_path(path: &Path, sequence: u64) -> PathBuf {
    path.with_file_name(format!(
        ".{}.generation-{}-{sequence}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("strict-run.active"),
        std::process::id()
    ))
}

fn remove_stale_strict_generation_staging(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("strict-run.active");
    let prefix = format!(".{name}.generation-");
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let entry_name = entry.file_name();
        let entry_name = entry_name.to_string_lossy();
        if entry_name.starts_with(&prefix) && entry_name.ends_with(".tmp") {
            match fs::remove_file(entry.path()) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
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
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    loop {
        if marker.exists() && !strict_run_marker_owner_alive(&marker) {
            let quarantine = stale_strict_run_marker_path(&marker);
            match fs::rename(&marker, &quarantine) {
                Ok(()) => {
                    fs::remove_file(&quarantine)?;
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            }
        }
        match super::orchestrator_run::publish_initialized_marker(
            &marker,
            &format!(
                "{}generation={generation}\n",
                super::orchestrator_run::process_marker_fields()
            ),
        ) {
            Ok(_) => {
                return Ok(StrictRunMarkerGuard { path: marker });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if strict_run_marker_owner_alive(&marker) {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

fn stale_strict_run_marker_path(path: &Path) -> PathBuf {
    let sequence = STRICT_STALE_MARKER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("strict-run.active");
    path.with_file_name(format!(
        ".{name}.stale-{}-{timestamp}-{sequence}",
        std::process::id()
    ))
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

    super::orchestrator_run::process_owner_alive(
        pid,
        contents
            .lines()
            .find_map(|line| line.strip_prefix("start=").map(str::trim)),
    )
}

fn migration_marker_path(target_config: &Path) -> PathBuf {
    let target_config = canonicalize_destination(target_config);
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
        let marker = parse_activation_marker(&target_path)?;
        if canonicalize_destination(&marker.config_path) != canonicalize_destination(target_config)
        {
            return Err(MigrationError::DestinationConflict {
                path: marker.config_path.clone(),
            });
        }
        return Ok(Some((target_path.clone(), marker)));
    }

    // Keep reading the pre-namespace marker for one-way compatibility, but never
    // let a marker for another central config control this target's rollback.
    let legacy_path = migration_root(target_config).join("activation.yaml");
    if !legacy_path.is_file() {
        return Ok(None);
    }
    let marker = parse_activation_marker(&legacy_path)?;
    if canonicalize_destination(&marker.config_path) == canonicalize_destination(target_config) {
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
    if marker.memory_catalog_copy_in_progress && workflow_stage.is_file() {
        let config_stage = stage_path(&marker.config_path, &marker.generation);
        verify_legacy_apply_inputs(&marker)?;
        let config_target = marker.config_path.clone();
        let workflow_target = marker.workflow_path.clone();
        verify_current_generation(&config_stage, &marker.generation)?;
        verify_current_generation(&workflow_stage, &marker.workflow_generation)?;
        resume_in_progress_catalog_copy(&marker_path, marker)?;
        replace_staged_file(&config_stage, &config_target)?;
        replace_staged_file(&workflow_stage, &workflow_target)?;
        return Ok(ActiveMigrationResolution::Complete);
    }
    if workflow_stage.is_file() {
        let staged_workflow = fs::read(&workflow_stage).map_err(|source| MigrationError::Read {
            path: workflow_stage.clone(),
            source,
        })?;
        if marker.workflow_generation.is_empty()
            || sha256(&staged_workflow) != marker.workflow_generation
        {
            let _memory_locks = acquire_partial_apply_catalog_guard(&marker)?;
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
        verify_partial_apply_inputs(&marker)?;
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

    let _memory_locks = acquire_partial_apply_catalog_guard(&marker)?;
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

fn verify_legacy_apply_inputs(marker: &ActivationMarker) -> Result<(), MigrationError> {
    verify_legacy_file(
        &marker.config_path,
        &marker.backup_dir.join("config.yaml"),
        marker.had_config,
    )?;
    verify_legacy_file(
        &marker.workflow_path,
        &marker.backup_dir.join("WORKFLOW.md"),
        marker.had_workflow,
    )
}

fn verify_legacy_file(
    path: &Path,
    backup_path: &Path,
    had_original: bool,
) -> Result<(), MigrationError> {
    if !had_original {
        if path.exists() {
            return Err(MigrationError::ActivatedFileChanged {
                path: path.to_path_buf(),
            });
        }
        return Ok(());
    }
    let backup = fs::read(backup_path).map_err(|source| MigrationError::Read {
        path: backup_path.to_path_buf(),
        source,
    })?;
    verify_current_generation(path, &sha256(&backup))
}

fn resume_in_progress_catalog_copy(
    marker_path: &Path,
    mut marker: ActivationMarker,
) -> Result<(), MigrationError> {
    let mut memory_locks = acquire_memory_migration_lock(&marker.target_repo())?;
    memory_locks
        .acquire_catalog_lock(&marker.target_repo(), marker.memory_catalog_root.as_deref())?;
    preserve_legacy_memory(&marker.target_repo(), marker.memory_catalog_root.as_deref())?;
    marker.memory_catalog_generation = marker
        .memory_catalog_root
        .as_deref()
        .map(memory_catalog_generation)
        .transpose()?;
    marker.memory_catalog_copy_in_progress = false;
    let marker_raw = serde_yaml::to_string(&marker).map_err(MigrationError::SerializeConfig)?;
    let marker_stage = stage_path(marker_path, &marker.generation);
    write_file(&marker_stage, marker_raw.as_bytes())?;
    replace_staged_file(&marker_stage, marker_path)
}

fn acquire_partial_apply_catalog_guard(
    marker: &ActivationMarker,
) -> Result<MemoryMigrationLock, MigrationError> {
    let mut memory_locks = acquire_memory_migration_lock(&marker.target_repo())?;
    memory_locks
        .acquire_catalog_lock(&marker.target_repo(), marker.memory_catalog_root.as_deref())?;
    if let (Some(root), Some(expected)) = (
        marker.memory_catalog_root.as_deref(),
        marker.memory_catalog_generation.as_deref(),
    ) && memory_catalog_generation(root)? != expected
    {
        return Err(MigrationError::MemoryCatalogChanged {
            path: root.to_path_buf(),
        });
    }
    Ok(memory_locks)
}

fn verify_partial_apply_inputs(marker: &ActivationMarker) -> Result<(), MigrationError> {
    verify_current_generation(&marker.config_path, &marker.generation)?;
    if !marker.had_workflow {
        return Err(MigrationError::ActivatedFileChanged {
            path: marker.workflow_path.clone(),
        });
    }
    let backup_workflow =
        fs::read(marker.backup_dir.join("WORKFLOW.md")).map_err(|source| MigrationError::Read {
            path: marker.backup_dir.join("WORKFLOW.md"),
            source,
        })?;
    verify_current_generation(&marker.workflow_path, &sha256(&backup_workflow))
}

fn verify_current_generation(path: &Path, expected_generation: &str) -> Result<(), MigrationError> {
    let contents = fs::read(path).map_err(|_| MigrationError::ActivatedFileChanged {
        path: path.to_path_buf(),
    })?;
    if sha256(&contents) == expected_generation {
        Ok(())
    } else {
        Err(MigrationError::ActivatedFileChanged {
            path: path.to_path_buf(),
        })
    }
}

fn verify_backup_generations(marker: &ActivationMarker) -> Result<(), MigrationError> {
    verify_backup_generation(
        &marker.backup_dir.join("config.yaml"),
        marker.had_config,
        marker.backup_config_generation.as_deref(),
    )?;
    verify_backup_generation(
        &marker.backup_dir.join("WORKFLOW.md"),
        marker.had_workflow,
        marker.backup_workflow_generation.as_deref(),
    )
}

fn verify_backup_generation(
    path: &Path,
    expected: bool,
    generation: Option<&str>,
) -> Result<(), MigrationError> {
    if !expected {
        return Ok(());
    }
    reject_symlink_ancestors(path)?;
    let generation = generation.ok_or_else(|| MigrationError::MissingBackupGeneration {
        path: path.to_path_buf(),
    })?;
    let contents = fs::read(path).map_err(|source| MigrationError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if sha256(&contents) != generation {
        return Err(MigrationError::BackupChanged {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

async fn preflight(paths: MigrationPaths) -> Result<MigrationReport, MigrationError> {
    if let Some(active) = active_target_central_config(&paths, false).await? {
        return Ok(active_report("preflight", &active, true));
    }
    let source = load_source(&paths, false)?;
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
    reject_symlink_input(&target_config)?;
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
    if let Some(active) = active_target_central_config(&paths, true).await? {
        let _strict_run_marker = claim_migration_strict_run_marker(&active.target_config)?;
        let resolution = {
            let _legacy_runtime_ownership = load_activation_marker(&active.target_config)?
                .filter(|(_, marker)| marker.memory_catalog_copy_in_progress)
                .map(|(_, marker)| {
                    recorded_legacy_runtime_workspace_root(&marker)
                        .and_then(|root| acquire_legacy_runtime_ownership(&root))
                })
                .transpose()?;
            resume_partial_apply(&active)?
        };
        return match resolution {
            ActiveMigrationResolution::Complete => Ok(active_report("apply", &active, false)),
            ActiveMigrationResolution::Restored => {
                drop(_strict_run_marker);
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
    let source = load_source(&paths, true)?;
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

    let workspace_root = legacy_runtime_workspace_root(&source)?;
    ensure_legacy_runtime_quiescent(&workspace_root)?;
    let generated = generate_central_config(&source)?;
    let _runtime_ownership = acquire_legacy_runtime_ownership(&workspace_root)?;
    let mut memory_locks = acquire_memory_migration_lock(&source.target_repo)?;
    let cwd = current_dir()?;
    let target_config = migration_target_config(&paths, &cwd, &source.source_config);
    let _strict_run_marker = claim_migration_strict_run_marker(&target_config)?;
    let generation = sha256(generated.as_bytes());
    let migration_root = migration_root(&target_config);
    let backup_dir = migration_root
        .join("backups")
        .join(generation.trim_start_matches("sha256:"));
    reject_symlink_ancestors(&backup_dir)?;
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
    let backup_config_path = backup_dir.join("config.yaml");
    let backup_workflow_path = backup_dir.join("WORKFLOW.md");
    reject_symlink_ancestors(&backup_config_path)?;
    reject_symlink_ancestors(&backup_workflow_path)?;
    let mut backup_config_generation = None;
    let mut backup_workflow_generation = None;
    if had_config {
        fs::copy(&target_config, &backup_config_path).map_err(|source_error| {
            MigrationError::Write {
                path: backup_config_path.clone(),
                source: source_error,
            }
        })?;
        backup_config_generation = Some(sha256(&fs::read(&backup_config_path).map_err(
            |source_error| MigrationError::Read {
                path: backup_config_path.clone(),
                source: source_error,
            },
        )?));
    }
    if had_workflow {
        fs::copy(&source.workflow_path, &backup_workflow_path).map_err(|source_error| {
            MigrationError::Write {
                path: backup_workflow_path.clone(),
                source: source_error,
            }
        })?;
        backup_workflow_generation = Some(sha256(&fs::read(&backup_workflow_path).map_err(
            |source_error| MigrationError::Read {
                path: backup_workflow_path.clone(),
                source: source_error,
            },
        )?));
    }

    let central_stage = stage_path(&target_config, &generation);
    let workflow_stage = stage_path(&source.workflow_path, &generation);
    write_file_with_mode(&central_stage, generated.as_bytes(), config_mode)?;
    let central = load_central_config(&central_stage).await?;
    let memory_catalog_root = central.memory_catalog_root.clone();
    let memory_catalog_generation_before = memory_catalog_root
        .as_deref()
        .map(memory_catalog_generation)
        .transpose()?;
    let workflow_body = workflow_body(&source)?;
    write_file_with_mode(&workflow_stage, &workflow_body, workflow_mode)?;

    let mut marker = ActivationMarker {
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
        memory_catalog_root: memory_catalog_root.clone(),
        memory_catalog_generation: memory_catalog_generation_before,
        memory_catalog_copy_in_progress: true,
        legacy_workspace_root: Some(workspace_root),
        backup_config_generation,
        backup_workflow_generation,
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
    memory_locks.acquire_catalog_lock(&source.target_repo, memory_catalog_root.as_deref())?;
    preserve_legacy_memory(&source.target_repo, memory_catalog_root.as_deref())?;
    marker.memory_catalog_generation = memory_catalog_root
        .as_deref()
        .map(memory_catalog_generation)
        .transpose()?;
    marker.memory_catalog_copy_in_progress = false;
    let marker_raw = serde_yaml::to_string(&marker).map_err(MigrationError::SerializeConfig)?;
    write_file(&marker_stage, marker_raw.as_bytes())?;
    replace_staged_file(&marker_stage, &marker_path)?;
    if let Err(error) = verify_legacy_source_generations(&source) {
        remove_staged_files(&[&central_stage, &workflow_stage, &marker_stage]);
        match fs::remove_file(&marker_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source_error) => {
                return Err(MigrationError::Write {
                    path: marker_path,
                    source: source_error,
                });
            }
        }
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

fn claim_migration_strict_run_marker(
    config_path: &Path,
) -> Result<StrictRunMarkerGuard, MigrationError> {
    let marker = strict_run_marker_path(config_path);
    match claim_strict_run_marker(config_path, "migration") {
        Ok(marker_guard) => Ok(marker_guard),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(MigrationError::ActiveStrictRun { path: marker })
        }
        Err(source) => Err(MigrationError::Write {
            path: marker,
            source,
        }),
    }
}

async fn rollback(args: RollbackArgs) -> Result<MigrationReport, MigrationError> {
    let cwd = current_dir()?;
    let config_path = canonicalize_destination(
        &args
            .config
            .map(|path| absolute_path(&cwd, &path))
            .unwrap_or_else(|| cwd.join("config.yaml")),
    );
    let Some((marker_path, marker)) = load_activation_marker(&config_path)? else {
        return Err(MigrationError::MissingActivation {
            path: migration_marker_path(&config_path),
        });
    };
    if rollback_files_match_backup(&marker)? {
        validate_activation_marker(
            &config_path,
            &marker,
            &marker.target_repo(),
            None,
            marker.memory_catalog_root.as_deref(),
        )?;
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

        let mut memory_locks = acquire_memory_migration_lock(&marker.target_repo())?;
        memory_locks
            .acquire_catalog_lock(&marker.target_repo(), marker.memory_catalog_root.as_deref())?;
        verify_backup_generations(&marker)?;
        if let (Some(root), Some(expected)) = (
            marker.memory_catalog_root.as_deref(),
            marker.memory_catalog_generation.as_deref(),
        ) && memory_catalog_generation(root)? != expected
        {
            return Err(MigrationError::MemoryCatalogChanged {
                path: root.to_path_buf(),
            });
        }
        if !rollback_files_match_backup(&marker)? {
            return Err(MigrationError::ActivatedFileChanged {
                path: marker.config_path.clone(),
            });
        }
        fs::remove_file(&marker_path).map_err(|source| MigrationError::Write {
            path: marker_path.clone(),
            source,
        })?;
        return Ok(rollback_report(marker_path, marker));
    }
    let central = if config_path.is_file() {
        load_central_config(&config_path).await?
    } else {
        load_central_config(&stage_path(&marker.config_path, &marker.generation)).await?
    };
    let target_repo = central.require_legacy_target_repo()?;
    validate_activation_marker(
        &config_path,
        &marker,
        &target_repo,
        None,
        central.memory_catalog_root.as_deref(),
    )?;
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

    let mut memory_locks = acquire_memory_migration_lock(&marker.target_repo())?;
    memory_locks
        .acquire_catalog_lock(&marker.target_repo(), marker.memory_catalog_root.as_deref())?;
    verify_activated_files(&marker)?;
    verify_backup_generations(&marker)?;
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

    Ok(rollback_report(marker_path, marker))
}

fn rollback_report(marker_path: PathBuf, marker: ActivationMarker) -> MigrationReport {
    MigrationReport {
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
    }
}

fn rollback_files_match_backup(marker: &ActivationMarker) -> Result<bool, MigrationError> {
    Ok(rollback_file_matches_backup(
        &marker.config_path,
        &marker.backup_dir.join("config.yaml"),
        marker.had_config,
    )? && rollback_file_matches_backup(
        &marker.workflow_path,
        &marker.backup_dir.join("WORKFLOW.md"),
        marker.had_workflow,
    )?)
}

fn rollback_file_matches_backup(
    target: &Path,
    backup: &Path,
    had_original: bool,
) -> Result<bool, MigrationError> {
    if !had_original {
        return Ok(!target.exists());
    }
    let backup_contents = fs::read(backup).map_err(|source| MigrationError::Read {
        path: backup.to_path_buf(),
        source,
    })?;
    match fs::read(target) {
        Ok(contents) => Ok(contents == backup_contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(MigrationError::ActivatedFileChanged {
            path: target.to_path_buf(),
        }),
    }
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
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if !had_original && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(_) => {
            return Err(MigrationError::ActivatedFileChanged {
                path: path.to_path_buf(),
            });
        }
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
        if entry.file_name() == super::memory::MEMORY_ACTIVITY_MARKER
            || entry.file_name() == MEMORY_MIGRATION_LOCK
        {
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

fn reject_symlink_input(path: &Path) -> Result<(), MigrationError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(MigrationError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() {
        return Err(MigrationError::SymlinkInput {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn reject_symlink_ancestors(path: &Path) -> Result<(), MigrationError> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        match fs::symlink_metadata(candidate) {
            Ok(metadata)
                if metadata.file_type().is_symlink() && !is_safe_system_path_alias(candidate) =>
            {
                return Err(MigrationError::SymlinkInput {
                    path: candidate.to_path_buf(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(MigrationError::Read {
                    path: candidate.to_path_buf(),
                    source,
                });
            }
        }
        current = candidate.parent();
    }
    Ok(())
}

fn is_safe_system_path_alias(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        path == Path::new("/tmp")
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

fn load_source(
    paths: &MigrationPaths,
    enforce_quiescence: bool,
) -> Result<SourceContext, MigrationError> {
    let cwd = current_dir()?;
    let repo = absolute_path(&cwd, &paths.repo);
    let source_config = paths
        .config
        .as_ref()
        .map(|path| absolute_path(&cwd, path))
        .unwrap_or_else(|| repo.join("config.yaml"));
    reject_symlink_input(&source_config)?;
    let source_config_present = source_config.is_file();
    let config_source = if source_config_present {
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
    if enforce_quiescence {
        ensure_memory_migration_inactive(&target_repo)?;
        ensure_legacy_memory_quiescent(&target_repo)?;
    }
    let workflow_path = target_repo.join("WORKFLOW.md");
    reject_symlink_input(&workflow_path)?;
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
        source_config_present,
        target_repo,
        workflow_path,
        workflow_source,
        workflow,
        config,
        remote,
    })
}

fn verify_legacy_source_generations(source: &SourceContext) -> Result<(), MigrationError> {
    let current_config = match fs::read(&source.source_config) {
        Ok(contents) if source.source_config_present => contents,
        Ok(_) => {
            return Err(MigrationError::LegacySourceChanged {
                path: source.source_config.clone(),
            });
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound && !source.source_config_present =>
        {
            Vec::new()
        }
        Err(_) => {
            return Err(MigrationError::LegacySourceChanged {
                path: source.source_config.clone(),
            });
        }
    };
    if source.source_config_present
        && sha256(&current_config) != sha256(source.config_source.as_bytes())
    {
        return Err(MigrationError::LegacySourceChanged {
            path: source.source_config.clone(),
        });
    }

    let current_workflow =
        fs::read(&source.workflow_path).map_err(|_| MigrationError::LegacySourceChanged {
            path: source.workflow_path.clone(),
        })?;
    if sha256(&current_workflow) != sha256(source.workflow_source.as_bytes()) {
        return Err(MigrationError::LegacySourceChanged {
            path: source.workflow_path.clone(),
        });
    }
    Ok(())
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
        literal_secret_detected: workflow_has_literal_secret(&source.workflow.front_matter),
        credential_bearing_remote_detected: remote_has_credentials(&source.remote),
        backup,
        activation_marker: None,
    }
}

fn migrated_workspace_root(
    source: &SourceContext,
    instance_id: &str,
    legacy_default_root: &Path,
) -> Result<String, MigrationError> {
    match source.workflow.front_matter.workspace.root.as_deref() {
        Some(value) => {
            let value =
                super::expand_env_tokens(value).map_err(|error| MigrationError::ResolveConfig {
                    path: source.workflow_path.clone(),
                    detail: format!("workspace.root: {error}"),
                })?;
            let resolved = resolve_repo_path(&source.target_repo, &value);
            if paths_overlap(&resolved, &source.target_repo) {
                reject_workspace_relocation(&resolved)?;
                Ok(format!("~/.opensymphony/workspaces/{instance_id}"))
            } else {
                Ok(resolved.display().to_string())
            }
        }
        None => {
            // The legacy resolver uses this default when front matter omits
            // workspace.root. Do not silently abandon populated state there.
            reject_workspace_relocation(legacy_default_root)?;
            Ok(format!("~/.opensymphony/workspaces/{instance_id}"))
        }
    }
}

fn generate_central_config(source: &SourceContext) -> Result<String, MigrationError> {
    if workflow_has_literal_secret(&source.workflow.front_matter) {
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
    let workspace_root =
        migrated_workspace_root(source, &instance_id, Path::new(DEFAULT_WORKSPACE_ROOT))?;
    // WORKFLOW.md retains the legacy execution prompt after orchestration
    // front matter is moved into the central config. AGENTS.md remains in the
    // checkout as implementation guidance for the worker.
    let instruction_path = "WORKFLOW.md";
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
        .map(|value| migrated_positive_u64(value, "agent.max_concurrent_agents"))
        .transpose()?
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
                    migrated_positive_u64(
                        value,
                        &format!("agent.max_concurrent_agents_by_state.{state}"),
                    )
                    .map(|value| (state.clone(), value))
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
            "max_turns": source.workflow.front_matter.agent.max_turns.as_ref().map(|value| migrated_positive_u64(value, "agent.max_turns")).transpose()?,
            "max_retry_backoff_ms": source.workflow.front_matter.agent.max_retry_backoff_ms.as_ref().map(|value| migrated_positive_u64(value, "agent.max_retry_backoff_ms")).transpose()?,
            "stall_timeout_ms": source.workflow.front_matter.agent.stall_timeout_ms.as_ref().map(migrated_stall_timeout_ms).transpose()?,
            "poll_interval_ms": source.workflow.front_matter.polling.interval_ms.as_ref().map(|value| migrated_positive_u64(value, "polling.interval_ms")).transpose()?,
        },
        "hooks": {
            "after_create": source.workflow.front_matter.hooks.after_create.clone(),
            "before_run": source.workflow.front_matter.hooks.before_run.clone(),
            "after_run": source.workflow.front_matter.hooks.after_run.clone(),
            "before_remove": source.workflow.front_matter.hooks.before_remove.clone(),
            "timeout_ms": source.workflow.front_matter.hooks.timeout_ms.as_ref().map(|value| migrated_non_positive_to_default(value, "hooks.timeout_ms")).transpose()?,
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

fn legacy_runtime_lock_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".opensymphony-instance.lock")
}

fn legacy_runtime_workspace_root(source: &SourceContext) -> Result<PathBuf, MigrationError> {
    let configured = source
        .workflow
        .front_matter
        .workspace
        .root
        .as_deref()
        .unwrap_or(DEFAULT_WORKSPACE_ROOT);
    let configured =
        super::expand_env_tokens(configured).map_err(|error| MigrationError::ResolveConfig {
            path: source.workflow_path.clone(),
            detail: format!("workspace.root: {error}"),
        })?;
    Ok(resolve_repo_path(&source.target_repo, &configured))
}

fn recorded_legacy_runtime_workspace_root(
    marker: &ActivationMarker,
) -> Result<PathBuf, MigrationError> {
    if let Some(root) = marker.legacy_workspace_root.as_ref() {
        return Ok(root.clone());
    }
    let workflow_source =
        fs::read_to_string(&marker.workflow_path).map_err(|source| MigrationError::Read {
            path: marker.workflow_path.clone(),
            source,
        })?;
    let workflow = WorkflowDefinition::parse(&workflow_source).map_err(|source| {
        MigrationError::ParseWorkflow {
            path: marker.workflow_path.clone(),
            source,
        }
    })?;
    let configured = workflow
        .front_matter
        .workspace
        .root
        .as_deref()
        .unwrap_or(DEFAULT_WORKSPACE_ROOT);
    let configured =
        super::expand_env_tokens(configured).map_err(|error| MigrationError::ResolveConfig {
            path: marker.workflow_path.clone(),
            detail: format!("workspace.root: {error}"),
        })?;
    Ok(resolve_repo_path(&marker.target_repo(), &configured))
}

fn ensure_legacy_runtime_quiescent(workspace_root: &Path) -> Result<(), MigrationError> {
    let marker = legacy_runtime_lock_path(workspace_root);
    if marker.exists() && super::orchestrator_run::root_lock_owner_alive(&marker) {
        return Err(MigrationError::RuntimeActive { path: marker });
    }
    Ok(())
}

fn acquire_legacy_runtime_ownership(
    workspace_root: &Path,
) -> Result<super::orchestrator_run::RuntimeRootOwnership, MigrationError> {
    let marker = legacy_runtime_lock_path(workspace_root);
    if marker.exists() && super::orchestrator_run::root_lock_owner_alive(&marker) {
        return Err(MigrationError::RuntimeActive { path: marker });
    }
    match super::orchestrator_run::acquire_root_ownership(vec![workspace_root.to_path_buf()]) {
        Ok(ownership) => Ok(ownership),
        Err(_error)
            if marker.exists() && super::orchestrator_run::root_lock_owner_alive(&marker) =>
        {
            Err(MigrationError::RuntimeActive { path: marker })
        }
        Err(error) => Err(MigrationError::RuntimeLock {
            path: marker,
            detail: error.to_string(),
        }),
    }
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
    _locks: Vec<super::memory::MemoryCoordinationLock>,
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
    Ok(MemoryMigrationLock { _locks: vec![lock] })
}

impl MemoryMigrationLock {
    fn acquire_catalog_lock(
        &mut self,
        target_repo: &Path,
        catalog_root: Option<&Path>,
    ) -> Result<(), MigrationError> {
        let Some(catalog_root) = catalog_root else {
            return Ok(());
        };
        if memory_migration_lock_path(target_repo) == memory_migration_lock_path(catalog_root) {
            return Ok(());
        }
        let path = memory_migration_lock_path(catalog_root);
        let lock = acquire_memory_coordination_lock(catalog_root).map_err(|source| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                MigrationError::MemoryMigrationActive { path: path.clone() }
            } else {
                MigrationError::Write {
                    path: path.clone(),
                    source,
                }
            }
        })?;
        self._locks.push(lock);
        Ok(())
    }
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

fn migrated_positive_u64(
    value: &crate::opensymphony_workflow::IntegerLike,
    field: &str,
) -> Result<u64, MigrationError> {
    let raw = integer_value(value);
    let parsed = raw
        .parse::<i64>()
        .map_err(|_| MigrationError::InvalidNumericSetting {
            field: field.to_owned(),
            value: raw.clone(),
        })?;
    if parsed <= 0 {
        return Err(MigrationError::InvalidNumericSetting {
            field: field.to_owned(),
            value: raw,
        });
    }
    Ok(parsed as u64)
}

fn migrated_non_positive_to_default(
    value: &crate::opensymphony_workflow::IntegerLike,
    field: &str,
) -> Result<u64, MigrationError> {
    let raw = integer_value(value);
    let parsed = raw
        .parse::<i64>()
        .map_err(|_| MigrationError::InvalidNumericSetting {
            field: field.to_owned(),
            value: raw,
        })?;
    Ok(if parsed <= 0 { 0 } else { parsed as u64 })
}

fn migrated_stall_timeout_ms(
    value: &crate::opensymphony_workflow::IntegerLike,
) -> Result<Option<u64>, MigrationError> {
    let raw = integer_value(value);
    let parsed = raw
        .parse::<i64>()
        .map_err(|_| MigrationError::InvalidNumericSetting {
            field: "agent.stall_timeout_ms".to_owned(),
            value: raw,
        })?;
    Ok(Some(if parsed <= 0 { 0 } else { parsed as u64 }))
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
                let git_arguments = &tokens[index + 1..];
                return git_arguments
                    .iter()
                    .any(|token| matches!(*token, "clone" | "init"))
                    || git_arguments
                        .windows(2)
                        .any(|pair| pair == ["worktree", "add"]);
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
                "cp" => tokens.iter().any(|token| {
                    matches!(*token, "-a" | "-r" | "-R" | "--archive" | "--recursive")
                }),
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

fn workflow_has_literal_secret(front_matter: &WorkflowFrontMatter) -> bool {
    front_matter
        .tracker
        .api_key
        .as_deref()
        .is_some_and(|value| credential_variable(value).is_none())
        || openhands_environment_has_literal_secret(&front_matter.openhands.local_server.env)
        || openhands_credential_selector_is_literal(front_matter)
        || [
            front_matter.hooks.after_create.as_deref(),
            front_matter.hooks.before_run.as_deref(),
            front_matter.hooks.after_run.as_deref(),
            front_matter.hooks.before_remove.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(hook_has_literal_secret)
}

pub(crate) fn hook_has_literal_secret(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();

    if literal_value_after_authorization(command, &lower) {
        return true;
    }

    for marker in [
        "api_key",
        "api-key",
        "api_token",
        "api-token",
        "access_token",
        "access-token",
        "oauth_token",
        "oauth-token",
        "client_secret",
        "client-secret",
        "private_key",
        "private-key",
        "password",
        "secret",
        "pat",
        "token",
    ] {
        if literal_assignment_value(command, &lower, marker) {
            return true;
        }
    }
    [
        "--api-key",
        "--access-token",
        "--client-secret",
        "--password",
        "--secret",
        "--pat",
        "--token",
    ]
    .into_iter()
    .any(|marker| literal_value_after_marker(command, &lower, marker))
}

fn literal_value_after_authorization(command: &str, lower: &str) -> bool {
    let marker = "authorization";
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find(marker) {
        let start = search_from + relative;
        let end = start + marker.len();
        if !is_hook_marker_boundary(lower, start, end) {
            search_from = end;
            continue;
        }
        let tail = &command[end..];
        if !tail
            .chars()
            .next()
            .is_some_and(|character| matches!(character, ':' | '='))
        {
            search_from = end;
            continue;
        }
        let tail = trim_hook_value_prefix(tail);
        if let Some(value) = next_hook_word(tail) {
            let literal = if ["bearer", "basic", "token"]
                .into_iter()
                .any(|scheme| value.eq_ignore_ascii_case(scheme))
            {
                let credential = trim_hook_value_prefix(&tail[value.len()..]);
                next_hook_word(credential).is_some_and(|value| credential_variable(value).is_none())
            } else {
                credential_variable(value).is_none()
            };
            if literal {
                return true;
            }
        }
        search_from = end;
    }
    false
}

fn literal_value_after_marker(command: &str, lower: &str, marker: &str) -> bool {
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find(marker) {
        let start = search_from + relative;
        let end = start + marker.len();
        if !is_hook_marker_boundary(lower, start, end) {
            search_from = end;
            continue;
        }
        let mut tail = &command[end..];
        tail = trim_hook_value_prefix(tail);
        if let Some(value) = next_hook_word(tail) {
            let literal = if marker == "authorization"
                && ["bearer", "basic", "token"]
                    .into_iter()
                    .any(|scheme| value.eq_ignore_ascii_case(scheme))
            {
                let credential = trim_hook_value_prefix(&tail[value.len()..]);
                next_hook_word(credential).is_some_and(|value| credential_variable(value).is_none())
            } else {
                credential_variable(value).is_none()
            };
            if literal {
                return true;
            }
        }
        search_from = end;
    }
    false
}

fn literal_assignment_value(command: &str, lower: &str, marker: &str) -> bool {
    let mut search_from = 0;
    while let Some(relative) = lower[search_from..].find(marker) {
        let start = search_from + relative;
        let end = start + marker.len();
        if !is_hook_marker_boundary(lower, start, end) {
            search_from = end;
            continue;
        }
        let before = &lower[..start];
        let preceded_by_flag = before.ends_with("--");
        let tail = &command[end..];
        if preceded_by_flag
            || tail
                .chars()
                .next()
                .is_some_and(|character| matches!(character, ':' | '='))
        {
            let tail = trim_hook_value_prefix(tail);
            if let Some(value) = next_hook_word(tail)
                && credential_variable(value).is_none()
            {
                return true;
            }
        }
        search_from = end;
    }
    false
}

fn is_hook_marker_boundary(value: &str, start: usize, end: usize) -> bool {
    let before = value[..start].chars().next_back();
    let after = value[end..].chars().next();
    before.is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
        && after.is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_')
}

fn trim_hook_value_prefix(value: &str) -> &str {
    value.trim_start_matches(|character: char| {
        character.is_ascii_whitespace() || matches!(character, ':' | '=' | '\'' | '"')
    })
}

fn next_hook_word(value: &str) -> Option<&str> {
    let end = value
        .find(|character: char| {
            character.is_ascii_whitespace() || matches!(character, '\'' | '"' | ',' | ';' | ')')
        })
        .unwrap_or(value.len());
    (end > 0).then(|| &value[..end])
}

fn openhands_credential_selector_is_literal(front_matter: &WorkflowFrontMatter) -> bool {
    let openhands = &front_matter.openhands;
    let mut selectors = vec![openhands.transport.session_api_key_env.as_deref()];
    if let Some(agent) = openhands.conversation.agent.as_ref()
        && let Some(llm) = agent.llm.as_ref()
    {
        selectors.extend([llm.api_key_env.as_deref(), llm.base_url_env.as_deref()]);
        if let Some(subscription) = llm.subscription.as_ref() {
            selectors.extend([
                subscription.access_token_env.as_deref(),
                subscription.account_id_env.as_deref(),
                subscription.auth_directory_env.as_deref(),
            ]);
        }
    }
    selectors
        .into_iter()
        .flatten()
        .any(|value| !is_environment_variable_name(value))
}

fn is_environment_variable_name(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase() || character == '_')
}

fn openhands_environment_has_literal_secret(env: &BTreeMap<String, String>) -> bool {
    env.iter().any(|(name, value)| {
        let name = normalize_secret_field_name(name);
        let secret_name = [
            "access_token",
            "api_key",
            "apikey",
            "authorization",
            "access_key",
            "accesskey",
            "credential",
            "password",
            "pat",
            "secret",
            "token",
        ]
        .iter()
        .any(|part| name == *part || name.ends_with(&format!("_{part}")));
        secret_name && credential_variable(value).is_none()
    })
}

fn normalize_secret_field_name(name: &str) -> String {
    let characters = name.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(name.len());
    for (index, character) in characters.iter().copied().enumerate() {
        if character == '-' {
            if !normalized.ends_with('_') {
                normalized.push('_');
            }
            continue;
        }
        if character.is_ascii_uppercase() {
            let previous_is_lower_or_digit = characters
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous.is_ascii_lowercase() || previous.is_ascii_digit());
            let previous_is_acronym_boundary = characters
                .get(index.wrapping_sub(1))
                .is_some_and(|previous| previous.is_ascii_uppercase())
                && characters
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_lowercase());
            if (previous_is_lower_or_digit || previous_is_acronym_boundary)
                && !normalized.ends_with('_')
            {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}

fn reject_workspace_relocation(path: &Path) -> Result<(), MigrationError> {
    if path.exists()
        && (!path.is_dir()
            || fs::read_dir(path)
                .map_err(|source| MigrationError::Read {
                    path: path.to_path_buf(),
                    source,
                })?
                .next()
                .is_some())
    {
        return Err(MigrationError::WorkspaceRootState {
            path: path.to_path_buf(),
        });
    }
    Ok(())
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
    let mut staged = path.as_os_str().to_os_string();
    staged.push(format!(".staging-{suffix}"));
    PathBuf::from(staged)
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
    reject_symlink_input(path)?;
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
    reject_symlink_ancestors(backup)?;
    reject_symlink_ancestors(target)?;
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

fn canonicalize_destination(path: &Path) -> PathBuf {
    let normalized = normalize_path(path);
    let Some(file_name) = normalized.file_name() else {
        return normalized;
    };
    let parent = normalized.parent().unwrap_or_else(|| Path::new("."));
    canonicalize_existing_prefix(parent)
        .unwrap_or_else(|| normalize_path(parent))
        .join(file_name)
}

fn canonicalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    let mut unresolved = Vec::new();
    let mut existing = path.to_path_buf();
    while !existing.exists() {
        unresolved.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?.to_path_buf();
    }
    let mut resolved = fs::canonicalize(existing).ok()?;
    for component in unresolved.iter().rev() {
        resolved.push(component);
    }
    Some(normalize_path(&resolved))
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
            source_config_present: false,
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
    fn migration_rejects_literal_openhands_secret_environment_values() {
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\nopenhands:\n  local_server:\n    env:\n      OPENAI_API_KEY: raw-secret-canary\n---\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let target_repo = PathBuf::from("repo");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
            target_repo: target_repo.clone(),
            workflow_path: target_repo.join("WORKFLOW.md"),
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
        assert!(report.literal_secret_detected);
        assert!(matches!(
            generate_central_config(&source),
            Err(MigrationError::LiteralSecret)
        ));
    }

    #[test]
    fn migration_rejects_pat_named_literal_secret() {
        let env = BTreeMap::from([(String::from("GITHUB_PAT"), String::from("raw-secret"))]);

        assert!(openhands_environment_has_literal_secret(&env));
    }

    #[test]
    fn migration_rejects_literal_openhands_credential_selector() {
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\nopenhands:\n  conversation:\n    agent:\n      llm:\n        credential_mode: openai_subscription\n        subscription:\n          access_token_env: resolved-access-token\n---\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let target_repo = PathBuf::from("repo");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
            target_repo: target_repo.clone(),
            workflow_path: target_repo.join("WORKFLOW.md"),
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
        assert!(report.literal_secret_detected);
        assert!(matches!(
            generate_central_config(&source),
            Err(MigrationError::LiteralSecret)
        ));
    }

    #[test]
    fn migration_rejects_literal_hook_credentials_but_allows_environment_references() {
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\nhooks:\n  before_run: \"curl -H 'Authorization: Bearer hook-secret-canary' https://example.invalid\"\n---\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let target_repo = PathBuf::from("repo");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
            target_repo: target_repo.clone(),
            workflow_path: target_repo.join("WORKFLOW.md"),
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
        assert!(!serialized.contains("hook-secret-canary"));
        assert!(matches!(
            generate_central_config(&source),
            Err(MigrationError::LiteralSecret)
        ));
        assert!(!hook_has_literal_secret(
            "curl -H 'Authorization: Bearer ${HOOK_TOKEN}' https://example.invalid"
        ));
        assert!(!hook_has_literal_secret(
            "echo 'basic authentication disabled'"
        ));
        assert!(!hook_has_literal_secret("echo 'authorization complete'"));
        assert!(hook_has_literal_secret(
            "echo 'Authorization: Bearer $SAFE'; curl -H 'Authorization: Bearer raw-token' https://example.invalid"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn staging_path_preserves_non_utf8_path_bytes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let path = PathBuf::from(OsString::from_vec(vec![
            b'c', b'o', b'n', b'f', b'i', b'g', 0x80,
        ]));
        let staged = stage_path(&path, "sha256:generation");
        let mut expected = path.as_os_str().to_os_string();
        expected.push(".staging-generation");

        assert_eq!(
            staged.as_os_str().as_bytes(),
            expected.as_os_str().as_bytes()
        );
    }

    #[test]
    fn migration_keeps_delimiter_leading_prompt_outside_front_matter() {
        let workflow_source = "---\ntracker:\n  kind: linear\n  project_slug: project\n---\n\n---\nTarget branch: develop\n---\n";
        let workflow =
            WorkflowDefinition::parse(workflow_source).expect("legacy workflow should parse");
        let source = SourceContext {
            source_config: PathBuf::from("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
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
    fn strict_run_markers_reject_a_reused_pid_with_a_different_incarnation() {
        let root = tempfile::tempdir().expect("marker root should exist");
        let config = root.path().join("config.yaml");
        let marker = strict_run_marker_path(&config);
        fs::create_dir_all(marker.parent().expect("marker parent should exist"))
            .expect("marker parent should be created");
        fs::write(
            &marker,
            format!(
                "pid={}\nstart=incarnation-that-is-not-current\n",
                std::process::id()
            ),
        )
        .expect("marker should be written");

        assert!(!strict_run_marker_owner_alive(&marker));
    }

    #[test]
    fn strict_run_generation_update_reclaims_stale_staging_files() {
        let root = tempfile::tempdir().expect("marker root should exist");
        let config = root.path().join("config.yaml");
        let marker = strict_run_marker_path(&config);
        fs::create_dir_all(marker.parent().expect("marker parent should exist"))
            .expect("marker parent should be created");
        let stale_stage = strict_generation_stage_path(&marker, 0);
        fs::write(&stale_stage, "stale\n").expect("stale stage should be written");

        let guard = claim_strict_run_marker(&config, "initial").expect("marker should claim");
        guard
            .update_generation("next")
            .expect("generation update should reclaim stale staging");
        assert!(!stale_stage.exists());
        drop(guard);
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_symlinked_legacy_inputs_before_apply() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("migration root should exist");
        let real_config = root.path().join("real-config.yaml");
        let config = root.path().join("config.yaml");
        let real_workflow = root.path().join("real-WORKFLOW.md");
        let workflow = root.path().join("WORKFLOW.md");
        fs::write(&real_config, "control_plane:\n  bind: 127.0.0.1:2468\n")
            .expect("real config should be written");
        fs::write(
            &real_workflow,
            "---\ntracker:\n  kind: linear\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n",
        )
        .expect("real workflow should be written");
        symlink(&real_config, &config).expect("config symlink should be created");
        symlink(&real_workflow, &workflow).expect("workflow symlink should be created");

        let error = load_source(
            &MigrationPaths {
                config: Some(config.clone()),
                repo: root.path().to_path_buf(),
                output: None,
            },
            true,
        )
        .expect_err("migration should reject symlinked inputs");
        assert!(matches!(error, MigrationError::SymlinkInput { path } if path == config));
        assert!(
            fs::symlink_metadata(&config)
                .expect("config metadata should remain available")
                .file_type()
                .is_symlink()
        );
        assert!(
            fs::symlink_metadata(&workflow)
                .expect("workflow metadata should remain available")
                .file_type()
                .is_symlink()
        );

        fs::remove_file(&config).expect("config symlink should be removable");
        fs::copy(&real_config, &config).expect("regular config should be restored");
        let error = load_source(
            &MigrationPaths {
                config: Some(config),
                repo: root.path().to_path_buf(),
                output: None,
            },
            true,
        )
        .expect_err("migration should reject a symlinked workflow");
        assert!(matches!(error, MigrationError::SymlinkInput { path } if path == workflow));
    }

    #[cfg(unix)]
    #[test]
    fn strict_run_markers_canonicalize_existing_symlinked_destinations() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("marker root should exist");
        let real = root.path().join("config.yaml");
        let symlinked = root.path().join("config-link.yaml");
        fs::write(&real, "schema_version: 1\n").expect("config should be written");
        symlink(&real, &symlinked).expect("config symlink should be created");

        assert_eq!(
            strict_run_marker_path(&real),
            strict_run_marker_path(&symlinked)
        );
    }

    #[cfg(unix)]
    #[test]
    fn migration_markers_canonicalize_symlinked_destination_parents() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("marker root should exist");
        let real_parent = root.path().join("real");
        let alias_parent = root.path().join("alias");
        fs::create_dir(&real_parent).expect("real destination parent should exist");
        symlink(&real_parent, &alias_parent).expect("destination parent symlink should exist");

        let real = real_parent.join("config.yaml");
        let aliased = alias_parent.join("config.yaml");

        assert_eq!(migration_root(&real), migration_root(&aliased));
        assert_eq!(
            migration_marker_path(&real),
            migration_marker_path(&aliased)
        );
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
    fn rollback_verification_rejects_unreadable_new_activated_files() {
        let root = tempfile::tempdir().expect("verification root should exist");
        let current = root.path().join("config.yaml");
        let backup = root.path().join("backup.yaml");
        fs::create_dir(&current).expect("an unreadable file fixture should exist");
        assert!(matches!(
            verify_activated_file(&current, &sha256(b"central\n"), &backup, false),
            Err(MigrationError::ActivatedFileChanged { path }) if path == current
        ));
    }

    #[test]
    fn partial_apply_catalog_guard_rejects_post_migration_captures() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let catalog = root.path().join("catalog");
        fs::create_dir_all(&repo).expect("repository root should exist");
        let expected = memory_catalog_generation(&catalog).expect("absent catalog should hash");
        fs::create_dir_all(&catalog).expect("catalog should exist");
        fs::write(catalog.join("capture.md"), "post-migration capture\n")
            .expect("capture should be written");
        let marker = ActivationMarker {
            source_config: repo.join("legacy.yaml"),
            config_path: repo.join("config.yaml"),
            workflow_path: repo.join("WORKFLOW.md"),
            backup_dir: repo.join(".migration/backups"),
            generation: "sha256:generation".to_owned(),
            workflow_generation: String::new(),
            had_config: true,
            had_workflow: true,
            config_mode: None,
            workflow_mode: None,
            memory_catalog_root: Some(catalog.clone()),
            memory_catalog_generation: Some(expected),
            memory_catalog_copy_in_progress: false,
            legacy_workspace_root: None,
            backup_config_generation: None,
            backup_workflow_generation: None,
        };

        assert!(matches!(
            acquire_partial_apply_catalog_guard(&marker),
            Err(MigrationError::MemoryCatalogChanged { path }) if path == catalog
        ));
    }

    #[test]
    fn interrupted_catalog_copy_resumes_before_promotion() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let source = repo.join(".opensymphony/memory");
        let catalog = root.path().join("state/memory");
        fs::create_dir_all(&source).expect("legacy memory root should exist");
        fs::write(source.join("capture.md"), "legacy capture\n")
            .expect("legacy capture should be written");
        let config_path = repo.join("config.yaml");
        let marker_path = migration_marker_path(&config_path);
        let marker = ActivationMarker {
            source_config: config_path.clone(),
            config_path,
            workflow_path: repo.join("WORKFLOW.md"),
            backup_dir: repo.join(".opensymphony/migration/backups"),
            generation: "sha256:generation".to_owned(),
            workflow_generation: String::new(),
            had_config: false,
            had_workflow: true,
            config_mode: None,
            workflow_mode: None,
            memory_catalog_root: Some(catalog.clone()),
            memory_catalog_generation: Some(
                memory_catalog_generation(&catalog).expect("empty catalog should hash"),
            ),
            memory_catalog_copy_in_progress: true,
            legacy_workspace_root: None,
            backup_config_generation: None,
            backup_workflow_generation: None,
        };

        resume_in_progress_catalog_copy(&marker_path, marker)
            .expect("interrupted catalog copy should resume");

        assert_eq!(
            fs::read_to_string(catalog.join("capture.md"))
                .expect("legacy capture should be resumed"),
            "legacy capture\n"
        );
        let (_, marker) = load_activation_marker(&repo.join("config.yaml"))
            .expect("activation marker should load")
            .expect("activation marker should be published");
        assert!(!marker.memory_catalog_copy_in_progress);
        assert_eq!(
            marker.memory_catalog_generation,
            Some(memory_catalog_generation(&catalog).expect("catalog generation should hash"))
        );
    }

    #[test]
    fn interrupted_catalog_copy_rejects_edited_legacy_inputs() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let backup_dir = repo.join(".opensymphony/migration/backups");
        let config = repo.join("config.yaml");
        let workflow = repo.join("WORKFLOW.md");
        fs::create_dir_all(&backup_dir).expect("backup directory should exist");
        fs::write(&config, "legacy config\n").expect("legacy config should exist");
        fs::write(&workflow, "legacy workflow\n").expect("legacy workflow should exist");
        fs::write(backup_dir.join("config.yaml"), "legacy config\n")
            .expect("config backup should exist");
        fs::write(backup_dir.join("WORKFLOW.md"), "legacy workflow\n")
            .expect("workflow backup should exist");
        let marker = ActivationMarker {
            source_config: config.clone(),
            config_path: config.clone(),
            workflow_path: workflow,
            backup_dir,
            generation: "sha256:generation".to_owned(),
            workflow_generation: "sha256:workflow".to_owned(),
            had_config: true,
            had_workflow: true,
            config_mode: None,
            workflow_mode: None,
            memory_catalog_root: None,
            memory_catalog_generation: None,
            memory_catalog_copy_in_progress: true,
            legacy_workspace_root: None,
            backup_config_generation: None,
            backup_workflow_generation: None,
        };

        fs::write(&config, "operator edit\n").expect("operator edit should be written");
        assert!(matches!(
            verify_legacy_apply_inputs(&marker),
            Err(MigrationError::ActivatedFileChanged { path }) if path == config
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
    fn migration_refuses_a_live_legacy_runtime_root() {
        let root = tempfile::tempdir().expect("migration root");
        let workspace_root = root.path().join("workspaces");
        let runtime =
            super::super::orchestrator_run::acquire_root_ownership([workspace_root.clone()])
                .expect("legacy runtime should own the workspace root");

        assert!(matches!(
            ensure_legacy_runtime_quiescent(&workspace_root),
            Err(MigrationError::RuntimeActive { .. })
        ));
        assert!(matches!(
            acquire_legacy_runtime_ownership(&workspace_root),
            Err(MigrationError::RuntimeActive { .. })
        ));
        drop(runtime);
        acquire_legacy_runtime_ownership(&workspace_root)
            .expect("migration should acquire the released legacy root");
    }

    #[test]
    fn migration_lock_also_serializes_a_central_catalog_root() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let catalog = root.path().join("central-state/memory");
        let mut first = acquire_memory_migration_lock(&repo).expect("repo lock should succeed");
        first
            .acquire_catalog_lock(&repo, Some(&catalog))
            .expect("catalog lock should succeed");

        assert!(matches!(
            acquire_memory_coordination_lock(&catalog),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists
        ));

        drop(first);
        let catalog_lock = acquire_memory_coordination_lock(&catalog)
            .expect("catalog lock should be released with migration scope");
        drop(catalog_lock);
    }

    #[test]
    fn memory_catalog_generation_ignores_coordination_lock() {
        let root = tempfile::tempdir().expect("catalog root should exist");
        fs::create_dir_all(root.path().join(".opensymphony"))
            .expect("coordination directory should exist");
        fs::write(root.path().join("issue.md"), "capsule\n").expect("catalog entry should exist");
        let before = memory_catalog_generation(root.path()).expect("generation should succeed");
        fs::write(memory_migration_lock_path(root.path()), "pid=123\n")
            .expect("coordination lock should exist");
        let after = memory_catalog_generation(root.path()).expect("generation should succeed");

        assert_eq!(before, after);
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
        assert!(hook_creates_repository("git worktree add ../checkout HEAD"));
        assert!(hook_creates_repository(
            "/usr/bin/git worktree add ../checkout HEAD"
        ));
        assert!(!hook_creates_repository("git worktree list"));
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
        assert!(hook_creates_repository(
            "cp --recursive /other/repository/. ."
        ));
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
        let mut source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
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

        let legacy_default = tempfile::tempdir().expect("legacy default root should exist");
        fs::write(
            legacy_default.path().join("COE-1-run.json"),
            "recoverable state",
        )
        .expect("legacy default state should be written");
        source.workflow.front_matter.workspace.root = None;
        assert!(matches!(
            migrated_workspace_root(&source, "legacy-instance", legacy_default.path()),
            Err(MigrationError::WorkspaceRootState { .. })
        ));
    }

    #[test]
    fn migration_rejects_invalid_numeric_scheduler_settings() {
        let target_repo = PathBuf::from("/repo");
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\n---\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let mut source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
            target_repo: target_repo.clone(),
            workflow_path: target_repo.join("WORKFLOW.md"),
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

        source.workflow.front_matter.agent.max_concurrent_agents =
            Some(crate::opensymphony_workflow::IntegerLike::Integer(0));
        assert!(matches!(
            generate_central_config(&source),
            Err(MigrationError::InvalidNumericSetting { field, .. })
                if field == "agent.max_concurrent_agents"
        ));
        source.workflow.front_matter.agent.max_concurrent_agents = None;

        for field in [
            "agent.max_turns",
            "agent.max_retry_backoff_ms",
            "agent.stall_timeout_ms",
            "polling.interval_ms",
            "hooks.timeout_ms",
        ] {
            let invalid =
                crate::opensymphony_workflow::IntegerLike::String("not-a-number".to_owned());
            match field {
                "agent.max_turns" => source.workflow.front_matter.agent.max_turns = Some(invalid),
                "agent.max_retry_backoff_ms" => {
                    source.workflow.front_matter.agent.max_retry_backoff_ms = Some(invalid)
                }
                "agent.stall_timeout_ms" => {
                    source.workflow.front_matter.agent.stall_timeout_ms = Some(invalid)
                }
                "polling.interval_ms" => {
                    source.workflow.front_matter.polling.interval_ms = Some(invalid)
                }
                "hooks.timeout_ms" => source.workflow.front_matter.hooks.timeout_ms = Some(invalid),
                _ => unreachable!(),
            }
            assert!(matches!(
                generate_central_config(&source),
                Err(MigrationError::InvalidNumericSetting { field: actual, .. })
                    if actual == field
            ));
            source.workflow.front_matter.agent.max_turns = None;
            source.workflow.front_matter.agent.max_retry_backoff_ms = None;
            source.workflow.front_matter.agent.stall_timeout_ms = None;
            source.workflow.front_matter.polling.interval_ms = None;
            source.workflow.front_matter.hooks.timeout_ms = None;
        }
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_symlinked_staging_files() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("migration root should exist");
        let target = root.path().join("central.yaml");
        let outside = root.path().join("outside.txt");
        let stage = stage_path(&target, "sha256:generation");
        fs::write(&outside, "keep me").expect("outside file should be written");
        symlink(&outside, &stage).expect("staging symlink should be created");

        let error = write_file(&stage, b"overwrite me")
            .expect_err("symlinked staging files must be rejected");
        assert!(matches!(error, MigrationError::SymlinkInput { path } if path == stage));
        assert_eq!(
            fs::read_to_string(&outside).expect("outside file should remain readable"),
            "keep me"
        );
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_symlinked_backup_ancestors() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("migration root should exist");
        let real = root.path().join("real-backups");
        let alias = root.path().join("backups");
        fs::create_dir(&real).expect("real backup directory should exist");
        symlink(&real, &alias).expect("backup ancestor symlink should be created");

        let error = reject_symlink_ancestors(&alias.join("generation/config.yaml"))
            .expect_err("backup symlink ancestors must be rejected");
        assert!(matches!(error, MigrationError::SymlinkInput { path } if path == alias));
    }

    #[cfg(unix)]
    #[test]
    fn migration_rejects_symlinked_backup_ancestors_beyond_existing_directory() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let external = root.path().join("external");
        fs::create_dir_all(&repo).expect("repository should exist");
        fs::create_dir_all(external.join("migration/backups/generation"))
            .expect("external backup tree should exist");
        symlink(&external, repo.join(".opensymphony"))
            .expect("opensymphony directory symlink should be created");

        let path = repo.join(".opensymphony/migration/backups/generation/config.yaml");
        let error = reject_symlink_ancestors(&path)
            .expect_err("distant backup symlink ancestors must be rejected");
        assert!(
            matches!(error, MigrationError::SymlinkInput { path } if path == repo.join(".opensymphony"))
        );
    }

    #[tokio::test]
    async fn central_memory_catalog_root_prefers_staged_generation_over_legacy_target() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let config_path = root.path().join("config.yaml");
        let central = format!(
            "schema_version: 1\ninstance:\n  id: staged\n  state_root: {0}/state\nrouting:\n  mode: legacy_single\n  repository: repo\ntracker_profiles:\n  linear:\n    provider: linear\n    credential: linear-key\n    active_states: [Todo]\n    terminal_states: [Done]\nlinear_projects:\n  project:\n    provider_project_id: project\n    repositories: [repo]\nrepositories:\n  repo:\n    aliases: [repo]\n    remote:\n      provider: git\n      locator: example/repo\n      clone: git@github.com:example/repo.git\n    target_branch: develop\n    credential: git-key\n    review_profile: review\n    instructions:\n      path: AGENTS.md\n    checkout_path: {0}/checkout\ncredentials:\n  linear-key:\n    kind: environment\n    variable: LINEAR_API_KEY\n  git-key:\n    kind: ssh-agent\nreview_profiles:\n  review:\n    provider: git\n    credential: git-key\nworkspace:\n  root: {0}/workspace\nmemory:\n  catalog_root: {0}/state/memory\n",
            root.path().display()
        );
        let generation = sha256(central.as_bytes());
        fs::write(&config_path, "legacy: true\n").expect("legacy config should exist");
        let staged = stage_path(&config_path, &generation);
        write_file(&staged, central.as_bytes()).expect("staged config should be written");
        let marker = ActivationMarker {
            source_config: config_path.clone(),
            config_path: config_path.clone(),
            workflow_path: root.path().join("WORKFLOW.md"),
            backup_dir: migration_root(&config_path)
                .join("backups")
                .join(generation.trim_start_matches("sha256:")),
            generation,
            workflow_generation: String::new(),
            had_config: true,
            had_workflow: false,
            config_mode: None,
            workflow_mode: None,
            memory_catalog_root: Some(root.path().join("state/memory")),
            memory_catalog_generation: None,
            memory_catalog_copy_in_progress: true,
            legacy_workspace_root: None,
            backup_config_generation: None,
            backup_workflow_generation: None,
        };

        let catalog = central_memory_catalog_root_for_marker(&config_path, &marker)
            .await
            .expect("staged central config should resolve")
            .expect("staged central config should provide a catalog root");
        assert_eq!(
            catalog,
            canonicalize_destination(&root.path().join("state/memory"))
        );
    }

    #[test]
    fn rollback_rejects_modified_backup_generations() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let backup_dir = root.path().join("backups");
        fs::create_dir_all(&backup_dir).expect("backup directory should exist");
        let config = backup_dir.join("config.yaml");
        let workflow = backup_dir.join("WORKFLOW.md");
        fs::write(&config, "legacy config\n").expect("config backup should exist");
        fs::write(&workflow, "legacy workflow\n").expect("workflow backup should exist");
        let marker = ActivationMarker {
            source_config: root.path().join("legacy.yaml"),
            config_path: root.path().join("config.yaml"),
            workflow_path: root.path().join("WORKFLOW.md"),
            backup_dir,
            generation: "sha256:central".to_owned(),
            workflow_generation: "sha256:workflow".to_owned(),
            had_config: true,
            had_workflow: true,
            config_mode: None,
            workflow_mode: None,
            memory_catalog_root: None,
            memory_catalog_generation: None,
            memory_catalog_copy_in_progress: false,
            legacy_workspace_root: None,
            backup_config_generation: Some(sha256(b"legacy config\n")),
            backup_workflow_generation: Some(sha256(b"legacy workflow\n")),
        };

        fs::write(&config, "tampered\n").expect("tampered backup should be written");
        assert!(matches!(
            verify_backup_generations(&marker),
            Err(MigrationError::BackupChanged { path }) if path == config
        ));
    }

    #[test]
    fn activation_marker_paths_are_bound_to_the_selected_config_and_repository() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let repo = root.path().join("repo");
        let target_config = root.path().join("central/config.yaml");
        let generation = sha256(b"central generation");
        let backup_dir = migration_root(&target_config)
            .join("backups")
            .join(generation.trim_start_matches("sha256:"));
        fs::create_dir_all(&backup_dir).expect("backup directory should exist");
        let marker = ActivationMarker {
            source_config: repo.join("config.yaml"),
            config_path: root.path().join("other/config.yaml"),
            workflow_path: repo.join("WORKFLOW.md"),
            backup_dir,
            generation,
            workflow_generation: String::new(),
            had_config: false,
            had_workflow: false,
            config_mode: None,
            workflow_mode: None,
            memory_catalog_root: None,
            memory_catalog_generation: None,
            memory_catalog_copy_in_progress: false,
            legacy_workspace_root: None,
            backup_config_generation: None,
            backup_workflow_generation: None,
        };

        assert!(matches!(
            validate_activation_marker(&target_config, &marker, &repo, None, None),
            Err(MigrationError::DestinationConflict { path }) if path == marker.config_path
        ));
    }

    #[test]
    fn migration_rejects_edited_legacy_sources_before_promotion() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let config_path = root.path().join("config.yaml");
        let workflow_path = root.path().join("WORKFLOW.md");
        let workflow_source =
            "---\ntracker:\n  kind: linear\n  project_slug: project\n---\nlegacy\n";
        fs::write(&config_path, "legacy config\n").expect("legacy config should exist");
        fs::write(&workflow_path, workflow_source).expect("legacy workflow should exist");
        let workflow = WorkflowDefinition::parse(workflow_source).expect("workflow should parse");
        let source = SourceContext {
            source_config: config_path.clone(),
            config_source: "legacy config\n".to_owned(),
            source_config_present: true,
            target_repo: root.path().to_path_buf(),
            workflow_path: workflow_path.clone(),
            workflow_source: workflow_source.to_owned(),
            workflow,
            config: LegacyConfigProbe {
                target_repo: None,
                control_plane: LegacyControlPlaneProbe::default(),
                openhands: LegacyOpenHandsProbe::default(),
                memory: LegacyMemoryProbe::default(),
            },
            remote: "git@github.com:example/repo.git".to_owned(),
        };
        fs::write(&workflow_path, "operator edit\n").expect("operator edit should be written");

        assert!(matches!(
            verify_legacy_source_generations(&source),
            Err(MigrationError::LegacySourceChanged { path }) if path == workflow_path
        ));
    }

    #[test]
    fn migration_rejects_nonempty_repo_relative_workspace_root() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let target_repo = root.path().join("repo");
        let workspace_root = target_repo.join("var/workspaces");
        fs::create_dir_all(&workspace_root).expect("workspace root should exist");
        fs::write(workspace_root.join("COE-1-run.json"), "recoverable state")
            .expect("workspace state should be written");
        let workflow_path = target_repo.join("WORKFLOW.md");
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\nworkspace:\n  root: ./var/workspaces\n---\n\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
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

        assert!(matches!(
            generate_central_config(&source),
            Err(MigrationError::WorkspaceRootState { .. })
        ));
    }

    #[tokio::test]
    async fn migration_claims_empty_repo_relative_workspace_after_validation() {
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
            "---\ntracker:\n  kind: linear\n  project_slug: project\nworkspace:\n  root: ./var/workspaces\n---\n\nTarget branch: develop\n",
        )
        .expect("legacy workflow should be written");
        let output = root.path().join("central/config.yaml");

        apply(MigrationPaths {
            config: None,
            repo: root.path().to_path_buf(),
            output: Some(output.clone()),
        })
        .await
        .expect("empty repo-relative workspace should be accepted");

        assert!(output.is_file());
        assert!(root.path().join("var/workspaces").is_dir());
        assert!(
            !root
                .path()
                .join("var/workspaces/.opensymphony-instance.lock")
                .exists()
        );
    }

    #[test]
    fn migration_keeps_workflow_prompt_when_agents_guidance_exists() {
        let root = tempfile::tempdir().expect("migration root should exist");
        let target_repo = root.path().join("repo");
        fs::create_dir_all(&target_repo).expect("repository should exist");
        fs::write(target_repo.join("AGENTS.md"), "implementation guidance\n")
            .expect("repository guidance should exist");
        let workflow = WorkflowDefinition::parse(
            "---\ntracker:\n  kind: linear\n  project_slug: project\n---\nTarget branch: develop\n",
        )
        .expect("workflow should parse");
        let source = SourceContext {
            source_config: target_repo.join("config.yaml"),
            config_source: String::new(),
            source_config_present: false,
            target_repo: target_repo.clone(),
            workflow_path: target_repo.join("WORKFLOW.md"),
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
        assert!(generated.contains("path: WORKFLOW.md"));
        assert!(!generated.contains("path: AGENTS.md"));
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
            source_config_present: false,
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
            ))
            .expect("negative stall timeout should be accepted"),
            Some(0)
        );
        assert_eq!(
            migrated_stall_timeout_ms(&crate::opensymphony_workflow::IntegerLike::Integer(-1))
                .expect("negative stall timeout should be accepted"),
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
    async fn preflight_reads_legacy_inputs_while_memory_is_active() {
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
            "---\ntracker:\n  kind: linear\n  project_slug: project\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n\nTarget branch: develop\n",
        )
        .expect("workflow should be written");
        let memory_root = root.path().join(".opensymphony/memory");
        fs::create_dir_all(&memory_root).expect("memory root should exist");
        fs::write(
            memory_activity_marker_path(&memory_root),
            format!("pid={}\n", std::process::id()),
        )
        .expect("active memory marker should be written");

        let report = preflight(MigrationPaths {
            config: None,
            repo: root.path().to_path_buf(),
            output: None,
        })
        .await
        .expect("read-only preflight should not require memory quiescence");
        assert!(report.preflight_only);
        assert!(!root.path().join("config.yaml").exists());
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
        let strict_run_marker = strict_run_marker_path(&output_path);
        fs::create_dir_all(
            strict_run_marker
                .parent()
                .expect("migration marker directory should have a parent"),
        )
        .expect("migration marker directory should be creatable");
        fs::write(
            &strict_run_marker,
            format!("pid={}\ngeneration=active\n", std::process::id()),
        )
        .expect("strict run marker should be written");
        let blocked_apply = apply(MigrationPaths {
            config: Some(config_path.clone()),
            repo: root.path().to_path_buf(),
            output: Some(output_path.clone()),
        })
        .await
        .expect_err("partial apply should be blocked by an active strict run");
        assert!(matches!(
            blocked_apply,
            MigrationError::ActiveStrictRun { .. }
        ));
        fs::remove_file(&strict_run_marker).expect("strict run marker should be removed");
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

        // Simulate an interrupted rollback after both legacy files were
        // restored but before the activation marker was removed.
        restore_file(
            &marker.backup_dir.join("WORKFLOW.md"),
            &workflow_path,
            marker.workflow_mode,
        )
        .expect("interrupted rollback should restore the legacy workflow");
        assert!(!marker.had_config);
        fs::remove_file(&output_path).expect("interrupted rollback should remove central config");

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
