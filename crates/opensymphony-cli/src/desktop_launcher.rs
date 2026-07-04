use std::{
    cmp::Ordering,
    env,
    ffi::OsString,
    fs,
    fs::File,
    io,
    io::{BufRead, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::Duration,
};

use clap::Args;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MANIFEST_FILE: &str = "opensymphony-desktop-manifest.json";
const RELEASE_INDEX_FILE: &str = "opensymphony-desktop-release-index.json";
const DEFAULT_CACHE_RELATIVE: &str = ".opensymphony/desktop";
const RELEASE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
const SOURCE_ARCHIVE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const SOURCE_BUILD_TARGET_DIR: &str = "target-opensymphony-desktop";
const LINUX_TAURI_PKG_CONFIG_MODULES: &[&[&str]] = &[
    &["webkit2gtk-4.1"],
    &["openssl"],
    &["librsvg-2.0"],
    &["xdo", "libxdo"],
    &["ayatana-appindicator3-0.1", "appindicator3-0.1"],
];

#[derive(Debug, Args)]
pub struct AppArgs {
    #[arg(
        long,
        env = "OPENSYMPHONY_DESKTOP_BUNDLE_DIR",
        help = "Local desktop bundle directory to install into the versioned cache"
    )]
    bundle_dir: Option<PathBuf>,
    #[arg(
        long,
        env = "OPENSYMPHONY_DESKTOP_INSTALL_PATH",
        value_name = "DIR",
        help = "Install root for versioned desktop bundles"
    )]
    install_path: Option<PathBuf>,
    #[arg(
        long,
        env = "OPENSYMPHONY_DESKTOP_CACHE_ROOT",
        hide = true,
        help = "Override the desktop cache root; primarily for smoke tests"
    )]
    cache_root: Option<PathBuf>,
    #[arg(
        long,
        env = "OPENSYMPHONY_DESKTOP_RELEASE_INDEX_URL",
        hide = true,
        help = "Override the desktop release index URL; primarily for smoke tests"
    )]
    release_index_url: Option<String>,
    #[arg(
        long,
        help = "Verify the bundle and print the launch target without starting it"
    )]
    dry_run: bool,
    #[arg(
        long,
        help = "Launch the cached desktop bundle without checking for updates first"
    )]
    no_update: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopBundleManifest {
    version: String,
    platform: String,
    arch: String,
    executable: PathBuf,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopReleaseIndex {
    schema_version: u32,
    assets: Vec<DesktopReleaseAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopReleaseAsset {
    version: String,
    platform: String,
    arch: String,
    url: String,
    checksum: DesktopReleaseChecksum,
    launch_target: DesktopLaunchTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopReleaseChecksum {
    algorithm: String,
    value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopLaunchTarget {
    executable: PathBuf,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug)]
struct VerifiedBundle {
    bundle_dir: PathBuf,
    executable: PathBuf,
    manifest: DesktopBundleManifest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemanticVersion {
    major: u64,
    minor: u64,
    patch: u64,
}

impl SemanticVersion {
    fn parse(value: &str) -> Option<Self> {
        let core = value.split_once('+').map_or(value, |(core, _)| core);
        let core = core.split_once('-').map_or(core, |(core, _)| core);
        let mut parts = core.split('.');
        let major = parse_version_part(parts.next()?)?;
        let minor = parse_version_part(parts.next()?)?;
        let patch = parse_version_part(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl Ord for SemanticVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl PartialOrd for SemanticVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_version_part(part: &str) -> Option<u64> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopCommand {
    program: String,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
}

impl DesktopCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        Self::new_os(
            program,
            args.iter().map(|arg| OsString::from(*arg)).collect(),
        )
    }

    fn new_os(program: &str, args: Vec<OsString>) -> Self {
        Self {
            program: program.to_string(),
            args,
            cwd: None,
        }
    }

    fn in_dir(mut self, cwd: &Path) -> Self {
        self.cwd = Some(cwd.to_path_buf());
        self
    }

    fn display(&self) -> String {
        if self.args.is_empty() {
            self.program.clone()
        } else {
            let args = self
                .args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            format!("{} {args}", self.program)
        }
    }
}

trait DesktopCommandRunner {
    fn program_exists(&self, program: &str) -> bool;
    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool;
    fn run(&mut self, command: &DesktopCommand) -> Result<(), DesktopLauncherError>;
}

struct RealCommandRunner;

impl DesktopCommandRunner for RealCommandRunner {
    fn program_exists(&self, program: &str) -> bool {
        Command::new(program)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    }

    fn command_succeeds(&self, program: &str, args: &[&str]) -> bool {
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn run(&mut self, command: &DesktopCommand) -> Result<(), DesktopLauncherError> {
        let mut child = Command::new(&command.program);
        child.args(&command.args);
        if let Some(cwd) = &command.cwd {
            child.current_dir(cwd);
        }
        let status =
            child
                .status()
                .map_err(|source| DesktopLauncherError::SourceBuildCommandIo {
                    command: command.display(),
                    source,
                })?;
        if status.success() {
            Ok(())
        } else {
            Err(DesktopLauncherError::SourceBuildCommandFailed {
                command: command.display(),
                status: status.to_string(),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrerequisiteIssue {
    name: &'static str,
    manual_commands: Vec<String>,
}

pub async fn run_command(args: AppArgs) -> ExitCode {
    let dry_run = args.dry_run;
    match run_app(args).await {
        Ok(executable) => {
            if !dry_run {
                println!("OpenSymphony desktop ready: {}", executable.display());
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

async fn run_app(args: AppArgs) -> Result<PathBuf, DesktopLauncherError> {
    let cache_root =
        normalize_cache_root(selected_install_root(args.install_path, args.cache_root)?)?;
    let cache_dir = cache_root.join(desktop_version());
    validate_cache_dir(&cache_root, &cache_dir)?;
    let bundle_dir = args.bundle_dir.as_deref();
    let no_update = args.no_update;
    let skip_update = no_update || bundle_dir.is_some();
    if args.dry_run {
        let verified = dry_run_verified_bundle(&cache_dir, bundle_dir)?;
        println!(
            "Dry run: would launch OpenSymphony desktop at {}",
            verified.executable.display()
        );
        return Ok(verified.executable);
    }

    let release_index_url = selected_release_index_url(args.release_index_url);

    let verified = ensure_verified_bundle(
        &cache_root,
        &cache_dir,
        bundle_dir,
        release_index_url.as_str(),
        true,
    )
    .await?;
    let verified = maybe_update_verified_bundle(
        &cache_root,
        verified,
        release_index_url.as_str(),
        skip_update,
    )
    .await;
    Command::new(&verified.executable)
        .current_dir(&verified.bundle_dir)
        .spawn()
        .map_err(|source| DesktopLauncherError::Launch {
            path: verified.executable.clone(),
            source,
        })?;
    Ok(verified.executable)
}

fn dry_run_verified_bundle(
    cache_dir: &Path,
    bundle_dir: Option<&Path>,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    if let Ok(bundle) = verify_bundle(cache_dir) {
        return Ok(bundle);
    }
    if let Some(bundle_dir) = bundle_dir {
        return verify_bundle(bundle_dir);
    }
    Err(DesktopLauncherError::DryRunSourceBuildRequired {
        version: desktop_version().to_string(),
        platform: current_platform().to_string(),
        arch: current_arch().to_string(),
    })
}

async fn ensure_verified_bundle(
    cache_root: &Path,
    cache_dir: &Path,
    bundle_dir: Option<&Path>,
    release_index_url: &str,
    allow_source_build: bool,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let mut runner = RealCommandRunner;
    ensure_verified_bundle_with(
        cache_root,
        cache_dir,
        bundle_dir,
        release_index_url,
        allow_source_build,
        &mut runner,
    )
    .await
}

async fn maybe_update_verified_bundle(
    cache_root: &Path,
    verified: VerifiedBundle,
    release_index_url: &str,
    no_update: bool,
) -> VerifiedBundle {
    if no_update {
        return verified;
    }
    let candidate = match latest_update_candidate(
        release_index_url,
        verified.manifest.version.as_str(),
    )
    .await
    {
        Ok(candidate) => candidate,
        Err(error) => {
            eprintln!("Desktop update check failed; launching installed bundle: {error}");
            return verified;
        }
    };
    let Some(asset) = candidate else {
        return verified;
    };
    let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
    if interactive {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut reader = stdin.lock();
        let mut writer = stdout.lock();
        match prompt_update_decision(
            &mut reader,
            &mut writer,
            true,
            verified.manifest.version.as_str(),
            asset.version.as_str(),
        ) {
            Ok(true) => {}
            Ok(false) => return verified,
            Err(source) => {
                eprintln!("Desktop update prompt failed; launching installed bundle: {source}");
                return verified;
            }
        }
    }
    match install_release_asset(cache_root, asset).await {
        Ok(updated) => updated,
        Err(error) => {
            eprintln!("Desktop update failed; launching installed bundle: {error}");
            verified
        }
    }
}

async fn ensure_verified_bundle_with<R: DesktopCommandRunner>(
    cache_root: &Path,
    cache_dir: &Path,
    bundle_dir: Option<&Path>,
    release_index_url: &str,
    allow_source_build: bool,
    runner: &mut R,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let cached = if bundle_dir.is_none() {
        latest_verified_cached_bundle(cache_root).or_else(|| verify_bundle(cache_dir).ok())
    } else {
        verify_bundle(cache_dir).ok()
    };
    if let Some(bundle) = cached {
        return Ok(bundle);
    }
    if let Some(bundle_dir) = bundle_dir {
        if cache_dir.exists() {
            remove_cache_entry(cache_root, cache_dir)?;
        }
        copy_dir_all(bundle_dir, cache_dir)?;
        match verify_bundle(cache_dir) {
            Ok(bundle) => Ok(bundle),
            Err(source) => {
                remove_cache_entry(cache_root, cache_dir)?;
                Err(source)
            }
        }
    } else {
        match install_release_bundle(cache_root, cache_dir, release_index_url).await {
            Ok(bundle) => Ok(bundle),
            Err(error) if allow_source_build && release_error_allows_source_fallback(&error) => {
                build_source_fallback(cache_root, cache_dir, runner).await
            }
            Err(error) if release_error_allows_source_fallback(&error) => {
                Err(DesktopLauncherError::DryRunSourceBuildRequired {
                    version: desktop_version().to_string(),
                    platform: current_platform().to_string(),
                    arch: current_arch().to_string(),
                })
            }
            Err(error) => Err(error),
        }
    }
}

fn release_error_allows_source_fallback(error: &DesktopLauncherError) -> bool {
    matches!(
        error,
        DesktopLauncherError::ReleaseIndexDownload { .. }
            | DesktopLauncherError::InvalidReleaseIndex { .. }
            | DesktopLauncherError::UnsupportedReleaseIndex { .. }
            | DesktopLauncherError::NoCompatibleReleaseAsset { .. }
            | DesktopLauncherError::ReleaseAssetDownload { .. }
    )
}

async fn latest_update_candidate(
    release_index_url: &str,
    installed_version: &str,
) -> Result<Option<DesktopReleaseAsset>, DesktopLauncherError> {
    let index = download_release_index_task(release_index_url).await?;
    Ok(newest_compatible_release_asset(&index, release_index_url, installed_version)?.cloned())
}

async fn download_release_index_task(
    release_index_url: &str,
) -> Result<DesktopReleaseIndex, DesktopLauncherError> {
    let release_index_url = release_index_url.to_string();
    tokio::task::spawn_blocking(move || download_release_index(&release_index_url))
        .await
        .map_err(|source| DesktopLauncherError::ReleaseInstallTask { source })?
}

fn newest_compatible_release_asset<'a>(
    index: &'a DesktopReleaseIndex,
    _release_index_url: &str,
    installed_version: &str,
) -> Result<Option<&'a DesktopReleaseAsset>, DesktopLauncherError> {
    let Some(installed_version) = SemanticVersion::parse(installed_version) else {
        return Ok(None);
    };
    let asset = index
        .assets
        .iter()
        .filter(|asset| asset.platform == current_platform() && asset.arch == current_arch())
        .filter_map(|asset| Some((SemanticVersion::parse(&asset.version)?, asset)))
        .filter(|(version, _)| *version > installed_version)
        .max_by_key(|(version, _)| *version)
        .map(|(_, asset)| asset);
    if let Some(asset) = asset {
        validate_release_asset(asset)?;
    }
    Ok(asset)
}

fn prompt_update_decision<R, W>(
    reader: &mut R,
    writer: &mut W,
    interactive: bool,
    installed_version: &str,
    update_version: &str,
) -> io::Result<bool>
where
    R: BufRead,
    W: Write,
{
    if !interactive {
        return Ok(true);
    }
    writeln!(
        writer,
        "OpenSymphony desktop {update_version} is available; installed version is {installed_version}."
    )?;
    loop {
        write!(writer, "Update before launch? [Y/n] ")?;
        writer.flush()?;
        let mut response = String::new();
        if reader.read_line(&mut response)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "update prompt closed before a response",
            ));
        }
        match response.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => writeln!(writer, "Please answer with `yes` or `no`.")?,
        }
    }
}

fn latest_verified_cached_bundle(cache_root: &Path) -> Option<VerifiedBundle> {
    fs::read_dir(cache_root)
        .ok()?
        .filter_map(|entry| {
            let cache_dir = entry.ok()?.path();
            reject_symlink(cache_root).ok()?;
            reject_symlink(&cache_dir).ok()?;
            let manifest = read_manifest(&cache_dir.join(MANIFEST_FILE)).ok()?;
            validate_cache_dir_for_version(cache_root, &cache_dir, &manifest.version).ok()?;
            let version = SemanticVersion::parse(&manifest.version)?;
            let verified = verify_bundle_for_version(&cache_dir, Some(&manifest.version)).ok()?;
            Some((version, verified))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, verified)| verified)
}

fn verify_bundle(cache_dir: &Path) -> Result<VerifiedBundle, DesktopLauncherError> {
    verify_bundle_for_version(cache_dir, Some(desktop_version()))
}

fn verify_bundle_for_version(
    cache_dir: &Path,
    expected_version: Option<&str>,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let manifest_path = cache_dir.join(MANIFEST_FILE);
    let manifest = read_manifest(&manifest_path)?;

    if let Some(expected) = expected_version
        && manifest.version != expected
    {
        return Err(DesktopLauncherError::WrongVersion {
            path: manifest_path,
            expected: expected.to_string(),
            actual: manifest.version,
        });
    }
    if manifest.platform != current_platform() {
        return Err(DesktopLauncherError::WrongPlatform {
            path: manifest_path,
            expected: current_platform().to_string(),
            actual: manifest.platform,
        });
    }
    if manifest.arch != current_arch() {
        return Err(DesktopLauncherError::WrongArch {
            path: manifest_path,
            expected: current_arch().to_string(),
            actual: manifest.arch,
        });
    }

    let executable = resolve_manifest_executable(cache_dir, &manifest.executable)?;
    let actual = file_sha256(&executable)?;
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err(DesktopLauncherError::BadChecksum {
            path: executable,
            expected: manifest.sha256,
            actual,
        });
    }

    Ok(VerifiedBundle {
        bundle_dir: cache_dir.to_path_buf(),
        executable,
        manifest,
    })
}

fn read_manifest(path: &Path) -> Result<DesktopBundleManifest, DesktopLauncherError> {
    let contents =
        fs::read_to_string(path).map_err(|source| DesktopLauncherError::MissingBundle {
            path: path.to_path_buf(),
            source,
        })?;
    serde_json::from_str(&contents).map_err(|source| DesktopLauncherError::InvalidManifest {
        path: path.to_path_buf(),
        source,
    })
}

#[allow(dead_code)]
fn parse_release_index(contents: &str) -> Result<DesktopReleaseIndex, serde_json::Error> {
    serde_json::from_str(contents)
}

fn selected_release_index_url(override_url: Option<String>) -> String {
    override_url.unwrap_or_else(default_release_index_url)
}

fn default_release_index_url() -> String {
    format!(
        "https://github.com/kumanday/OpenSymphony/releases/download/v{}/{}",
        desktop_version(),
        RELEASE_INDEX_FILE
    )
}

fn release_url_display_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return "<invalid release URL>".to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

async fn install_release_bundle(
    cache_root: &Path,
    cache_dir: &Path,
    release_index_url: &str,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let cache_root = cache_root.to_path_buf();
    let cache_dir = cache_dir.to_path_buf();
    let release_index_url = release_index_url.to_string();
    tokio::task::spawn_blocking(move || {
        install_release_bundle_blocking(&cache_root, &cache_dir, &release_index_url)
    })
    .await
    .map_err(|source| DesktopLauncherError::ReleaseInstallTask { source })?
}

fn install_release_bundle_blocking(
    cache_root: &Path,
    cache_dir: &Path,
    release_index_url: &str,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let index = download_release_index(release_index_url)?;
    let asset = compatible_release_asset(&index, release_index_url)?;
    install_release_asset_blocking(cache_root, cache_dir, asset)
}

async fn install_release_asset(
    cache_root: &Path,
    asset: DesktopReleaseAsset,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let cache_root = cache_root.to_path_buf();
    let cache_dir = cache_root.join(&asset.version);
    tokio::task::spawn_blocking(move || {
        install_release_asset_blocking(&cache_root, &cache_dir, &asset)
    })
    .await
    .map_err(|source| DesktopLauncherError::ReleaseInstallTask { source })?
}

fn install_release_asset_blocking(
    cache_root: &Path,
    cache_dir: &Path,
    asset: &DesktopReleaseAsset,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    validate_cache_dir_for_version(cache_root, cache_dir, &asset.version)?;
    fs::create_dir_all(cache_root).map_err(|source| DesktopLauncherError::Repair {
        path: cache_root.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".release-install-")
        .tempdir_in(cache_root)
        .map_err(|source| DesktopLauncherError::Repair {
            path: cache_root.to_path_buf(),
            source,
        })?;
    let stage_dir = staging.path().to_path_buf();
    let archive_path = stage_dir.join("bundle.tar.gz");

    (|| {
        download_file(&asset.url, &archive_path)?;
        verify_archive_checksum(&archive_path, asset)?;
        extract_release_archive(&archive_path, &stage_dir)?;
        let verified = verify_bundle_matches_asset(&stage_dir, asset)?;
        promote_verified_bundle(cache_root, cache_dir, &stage_dir, &asset.version)?;
        verify_bundle_for_version(cache_dir, Some(&asset.version)).map(|promoted| VerifiedBundle {
            bundle_dir: promoted.bundle_dir,
            executable: promoted.executable,
            manifest: verified.manifest,
        })
    })()
}

fn download_release_index(url: &str) -> Result<DesktopReleaseIndex, DesktopLauncherError> {
    let display_url = release_url_display_url(url);
    let client =
        release_http_client().map_err(|source| DesktopLauncherError::ReleaseIndexDownload {
            url: display_url.clone(),
            source: source.without_url(),
        })?;
    let body = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .map_err(|source| DesktopLauncherError::ReleaseIndexDownload {
            url: display_url.clone(),
            source: source.without_url(),
        })?;
    let index =
        parse_release_index(&body).map_err(|source| DesktopLauncherError::InvalidReleaseIndex {
            url: display_url.clone(),
            source,
        })?;
    if index.schema_version != 1 {
        return Err(DesktopLauncherError::UnsupportedReleaseIndex {
            url: display_url,
            schema_version: index.schema_version,
        });
    }
    Ok(index)
}

fn compatible_release_asset<'a>(
    index: &'a DesktopReleaseIndex,
    release_index_url: &str,
) -> Result<&'a DesktopReleaseAsset, DesktopLauncherError> {
    let display_index_url = release_url_display_url(release_index_url);
    index
        .assets
        .iter()
        .find(|asset| {
            asset.version == desktop_version()
                && asset.platform == current_platform()
                && asset.arch == current_arch()
        })
        .ok_or_else(|| DesktopLauncherError::NoCompatibleReleaseAsset {
            url: display_index_url,
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
        })
        .and_then(|asset| {
            validate_release_asset(asset)?;
            Ok(asset)
        })
}

fn validate_release_asset(asset: &DesktopReleaseAsset) -> Result<(), DesktopLauncherError> {
    let display_asset_url = release_url_display_url(&asset.url);
    if asset.checksum.algorithm != "sha256" {
        return Err(DesktopLauncherError::UnsupportedReleaseChecksum {
            url: display_asset_url,
            algorithm: asset.checksum.algorithm.clone(),
        });
    }
    if !asset.launch_target.args.is_empty() {
        return Err(DesktopLauncherError::UnsupportedLaunchArgs {
            url: display_asset_url,
        });
    }
    if asset.launch_target.executable.is_absolute()
        || asset
            .launch_target
            .executable
            .components()
            .any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
    {
        return Err(DesktopLauncherError::UnsafeExecutablePath {
            path: asset.launch_target.executable.clone(),
        });
    }
    Ok(())
}

fn download_file(url: &str, destination: &Path) -> Result<(), DesktopLauncherError> {
    let display_url = release_url_display_url(url);
    let client =
        release_http_client().map_err(|source| DesktopLauncherError::ReleaseAssetDownload {
            url: display_url.clone(),
            source: source.without_url(),
        })?;
    let response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|source| DesktopLauncherError::ReleaseAssetDownload {
            url: display_url.clone(),
            source: source.without_url(),
        })?;
    let body = response
        .bytes()
        .map_err(|source| DesktopLauncherError::ReleaseAssetDownload {
            url: display_url,
            source: source.without_url(),
        })?;
    fs::write(destination, body).map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
        path: destination.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn release_http_client() -> Result<reqwest::blocking::Client, reqwest::Error> {
    reqwest::blocking::Client::builder()
        .timeout(RELEASE_DOWNLOAD_TIMEOUT)
        .build()
}

fn verify_archive_checksum(
    archive_path: &Path,
    asset: &DesktopReleaseAsset,
) -> Result<(), DesktopLauncherError> {
    let actual = raw_file_sha256(archive_path).map_err(|source| {
        DesktopLauncherError::ReleaseArchiveRead {
            path: archive_path.to_path_buf(),
            source,
        }
    })?;
    if !actual.eq_ignore_ascii_case(&asset.checksum.value) {
        return Err(DesktopLauncherError::ReleaseArchiveChecksum {
            path: archive_path.to_path_buf(),
            expected: asset.checksum.value.clone(),
            actual,
        });
    }
    Ok(())
}

fn extract_release_archive(
    archive_path: &Path,
    stage_dir: &Path,
) -> Result<(), DesktopLauncherError> {
    let file =
        File::open(archive_path).map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
            path: archive_path.to_path_buf(),
            source,
        })?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
            path: archive_path.to_path_buf(),
            source,
        })?
    {
        let mut entry = entry.map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
            path: archive_path.to_path_buf(),
            source,
        })?;
        let entry_path = entry
            .path()
            .map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
                path: archive_path.to_path_buf(),
                source,
            })?
            .to_path_buf();
        if entry_path.is_absolute()
            || entry_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(DesktopLauncherError::UnsafeArchiveEntry { path: entry_path });
        }
        let entry_type = entry.header().entry_type();
        if !(entry_type.is_file() || entry_type.is_dir()) {
            return Err(DesktopLauncherError::UnsupportedBundleEntry { path: entry_path });
        }
        let unpacked = entry.unpack_in(stage_dir).map_err(|source| {
            DesktopLauncherError::ReleaseArchiveRead {
                path: archive_path.to_path_buf(),
                source,
            }
        })?;
        if !unpacked {
            return Err(DesktopLauncherError::UnsafeArchiveEntry { path: entry_path });
        }
    }
    fs::remove_file(archive_path).map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
        path: archive_path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn verify_bundle_matches_asset(
    stage_dir: &Path,
    asset: &DesktopReleaseAsset,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let verified = verify_bundle_for_version(stage_dir, Some(&asset.version))?;
    if verified.manifest.executable != asset.launch_target.executable {
        return Err(DesktopLauncherError::LaunchTargetMismatch {
            expected: asset.launch_target.executable.clone(),
            actual: verified.manifest.executable.clone(),
        });
    }
    Ok(verified)
}

fn promote_verified_bundle(
    cache_root: &Path,
    cache_dir: &Path,
    stage_dir: &Path,
    version: &str,
) -> Result<(), DesktopLauncherError> {
    validate_cache_dir_for_version(cache_root, cache_dir, version)?;
    if cache_dir.exists() {
        remove_cache_entry_for_version(cache_root, cache_dir, version)?;
    }
    fs::rename(stage_dir, cache_dir).map_err(|source| DesktopLauncherError::Repair {
        path: cache_dir.to_path_buf(),
        source,
    })
}

async fn build_source_fallback<R: DesktopCommandRunner>(
    cache_root: &Path,
    cache_dir: &Path,
    runner: &mut R,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    println!(
        "No compatible prebuilt desktop bundle is installed for OpenSymphony {} on {}/{}; building from source.",
        desktop_version(),
        current_platform(),
        current_arch()
    );
    validate_cache_dir(cache_root, cache_dir)?;
    fs::create_dir_all(cache_root).map_err(|source| DesktopLauncherError::Repair {
        path: cache_root.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".source-build-")
        .tempdir_in(cache_root)
        .map_err(|source| DesktopLauncherError::Repair {
            path: cache_root.to_path_buf(),
            source,
        })?;
    let source_dir = staging.path().join("source");
    fs::create_dir_all(&source_dir).map_err(|source| DesktopLauncherError::Repair {
        path: source_dir.clone(),
        source,
    })?;
    ensure_source_build_prerequisites(current_platform(), runner)?;

    let archive = staging.path().join("opensymphony-source.tar.gz");
    let url = source_archive_url();
    println!(
        "Downloading OpenSymphony source archive: {}",
        source_archive_display_url(&url)
    );
    download_source_archive(&url, &archive).await?;

    println!("Extracting OpenSymphony source archive.");
    runner.run(&DesktopCommand::new_os(
        "tar",
        vec![
            OsString::from("-xzf"),
            archive.as_os_str().to_os_string(),
            OsString::from("-C"),
            source_dir.as_os_str().to_os_string(),
            OsString::from("--strip-components"),
            OsString::from("1"),
        ],
    ))?;
    install_source_built_bundle_from_source_dir(cache_root, cache_dir, &source_dir, runner)
}

fn source_archive_url() -> String {
    env::var("OPENSYMPHONY_DESKTOP_SOURCE_ARCHIVE_URL").unwrap_or_else(|_| {
        format!(
            "https://github.com/kumanday/OpenSymphony/archive/refs/tags/v{}.tar.gz",
            desktop_version()
        )
    })
}

fn source_archive_display_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return "<invalid source archive URL>".to_string();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.to_string()
}

async fn download_source_archive(url: &str, path: &Path) -> Result<(), DesktopLauncherError> {
    let display_url = source_archive_display_url(url);
    let client = reqwest::Client::builder()
        .timeout(SOURCE_ARCHIVE_DOWNLOAD_TIMEOUT)
        .build()
        .map_err(|source| DesktopLauncherError::SourceDownload {
            url: display_url.clone(),
            source: source.without_url(),
        })?;
    let response =
        client
            .get(url)
            .send()
            .await
            .map_err(|source| DesktopLauncherError::SourceDownload {
                url: display_url.clone(),
                source: source.without_url(),
            })?;
    let status = response.status();
    if !status.is_success() {
        return Err(DesktopLauncherError::SourceDownloadStatus {
            url: display_url,
            status: status.as_u16(),
        });
    }
    let body = response
        .bytes()
        .await
        .map_err(|source| DesktopLauncherError::SourceDownload {
            url: source_archive_display_url(url),
            source: source.without_url(),
        })?;
    fs::write(path, body).map_err(|source| DesktopLauncherError::Repair {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn ensure_source_build_prerequisites<R: DesktopCommandRunner>(
    platform: &str,
    runner: &mut R,
) -> Result<(), DesktopLauncherError> {
    let initial = probe_source_build_prerequisites(platform, runner);
    let platform_name = platform_prerequisite_name(platform);
    if initial.iter().any(|issue| issue.name != platform_name) {
        return Err(DesktopLauncherError::MissingPrerequisites {
            details: format_prerequisite_issues(&initial),
        });
    }
    if initial.iter().any(|issue| issue.name == platform_name)
        && let Some(commands) = platform_install_plan(platform, runner)
    {
        println!("Installing desktop platform prerequisites with detected package manager.");
        for command in commands {
            println!("Running prerequisite installer: {}", command.display());
            if runner.run(&command).is_err() {
                return Err(platform_missing_prerequisites(platform, runner));
            }
        }
    }

    let missing = probe_source_build_prerequisites(platform, runner);
    if missing.is_empty() {
        Ok(())
    } else {
        Err(DesktopLauncherError::MissingPrerequisites {
            details: format_prerequisite_issues(&missing),
        })
    }
}

fn probe_source_build_prerequisites<R: DesktopCommandRunner>(
    platform: &str,
    runner: &R,
) -> Vec<PrerequisiteIssue> {
    let mut missing = Vec::new();
    if !runner.program_exists("cargo") || !runner.program_exists("rustc") {
        missing.push(PrerequisiteIssue {
            name: "Rust/Cargo",
            manual_commands: rust_manual_commands(platform),
        });
    }
    if !runner.program_exists("node") || !runner.program_exists("npm") {
        missing.push(PrerequisiteIssue {
            name: "Node/npm",
            manual_commands: vec![
                "install the Node.js LTS release from https://nodejs.org/".to_string(),
                "restart your terminal, then run: node -v && npm -v".to_string(),
            ],
        });
    }
    if !runner.program_exists("tar") {
        missing.push(PrerequisiteIssue {
            name: "source archive extraction tool",
            manual_commands: vec![
                "install GNU tar or bsdtar with your system package manager".to_string(),
            ],
        });
    }
    if !platform_desktop_dependencies_ready(platform, runner) {
        missing.push(PrerequisiteIssue {
            name: platform_prerequisite_name(platform),
            manual_commands: platform_manual_commands(platform, runner),
        });
    }
    missing
}

fn rust_manual_commands(platform: &str) -> Vec<String> {
    match platform {
        "windows" => vec![
            "winget install --id Rustlang.Rustup --source winget".to_string(),
            "rustup default stable-msvc".to_string(),
            "restart PowerShell, then run: cargo --version; rustc --version".to_string(),
        ],
        _ => vec![
            "curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh".to_string(),
            "restart your terminal, then run: cargo --version && rustc --version".to_string(),
        ],
    }
}

fn platform_desktop_dependencies_ready<R: DesktopCommandRunner>(
    platform: &str,
    runner: &R,
) -> bool {
    match platform {
        "macos" => runner.command_succeeds("xcode-select", &["-p"]),
        "linux" => {
            runner.program_exists("pkg-config")
                && linux_build_toolchain_ready(runner)
                && linux_pkg_config_modules_ready(runner)
        }
        // Windows Build Tools are often installed without exposing cl.exe in a
        // normal shell. Let Cargo's MSVC probe report the precise toolchain
        // error instead of blocking source fallback on a brittle PATH check.
        "windows" => true,
        _ => false,
    }
}

fn linux_build_toolchain_ready<R: DesktopCommandRunner>(runner: &R) -> bool {
    runner.program_exists("cc") || runner.program_exists("gcc") || runner.program_exists("clang")
}

fn linux_pkg_config_modules_ready<R: DesktopCommandRunner>(runner: &R) -> bool {
    LINUX_TAURI_PKG_CONFIG_MODULES.iter().all(|alternatives| {
        alternatives
            .iter()
            .any(|module| runner.command_succeeds("pkg-config", &["--exists", module]))
    })
}

fn platform_prerequisite_name(platform: &str) -> &'static str {
    match platform {
        "macos" => "macOS desktop/Tauri dependencies",
        "linux" => "Linux desktop/Tauri dependencies",
        "windows" => "Windows desktop/Tauri dependencies",
        _ => "platform desktop/Tauri dependencies",
    }
}

fn platform_install_plan<R: DesktopCommandRunner>(
    platform: &str,
    runner: &R,
) -> Option<Vec<DesktopCommand>> {
    if platform != "linux" || !runner.program_exists("sudo") {
        return None;
    }

    if runner.program_exists("apt-get") {
        return Some(vec![
            DesktopCommand::new("sudo", &["-n", "apt-get", "update"]),
            DesktopCommand::new(
                "sudo",
                &[
                    "-n",
                    "apt-get",
                    "install",
                    "-y",
                    "libwebkit2gtk-4.1-dev",
                    "build-essential",
                    "curl",
                    "wget",
                    "file",
                    "libxdo-dev",
                    "libssl-dev",
                    "libayatana-appindicator3-dev",
                    "librsvg2-dev",
                    "pkg-config",
                ],
            ),
        ]);
    }
    if runner.program_exists("pacman") {
        return Some(vec![DesktopCommand::new(
            "sudo",
            &[
                "-n",
                "pacman",
                "-S",
                "--needed",
                "--noconfirm",
                "webkit2gtk-4.1",
                "base-devel",
                "curl",
                "wget",
                "file",
                "openssl",
                "appmenu-gtk-module",
                "libappindicator-gtk3",
                "librsvg",
                "xdotool",
                "pkgconf",
            ],
        )]);
    }
    if runner.program_exists("dnf") {
        return Some(vec![
            DesktopCommand::new(
                "sudo",
                &[
                    "-n",
                    "dnf",
                    "install",
                    "-y",
                    "webkit2gtk4.1-devel",
                    "openssl-devel",
                    "curl",
                    "wget",
                    "file",
                    "libappindicator-gtk3-devel",
                    "librsvg2-devel",
                    "libxdo-devel",
                    "pkgconf-pkg-config",
                ],
            ),
            DesktopCommand::new(
                "sudo",
                &["-n", "dnf", "group", "install", "-y", "c-development"],
            ),
        ]);
    }
    None
}

fn platform_missing_prerequisites<R: DesktopCommandRunner>(
    platform: &str,
    runner: &R,
) -> DesktopLauncherError {
    DesktopLauncherError::MissingPrerequisites {
        details: format_prerequisite_issues(&[PrerequisiteIssue {
            name: platform_prerequisite_name(platform),
            manual_commands: platform_manual_commands(platform, runner),
        }]),
    }
}

fn platform_manual_commands<R: DesktopCommandRunner>(platform: &str, runner: &R) -> Vec<String> {
    match platform {
        "macos" => vec!["xcode-select --install".to_string()],
        "windows" => vec![
            "winget install --id Microsoft.VisualStudio.2022.BuildTools".to_string(),
            "install Microsoft Edge WebView2 Evergreen Runtime".to_string(),
        ],
        "linux" => linux_manual_commands(runner),
        _ => vec![
            "install the Tauri desktop prerequisites for this platform before building".to_string(),
        ],
    }
}

fn linux_manual_commands<R: DesktopCommandRunner>(runner: &R) -> Vec<String> {
    if runner.program_exists("apt-get") {
        return vec![
            "sudo apt-get update".to_string(),
            "sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config".to_string(),
        ];
    }
    if runner.program_exists("pacman") {
        return vec![
            "sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool pkgconf".to_string(),
        ];
    }
    if runner.program_exists("dnf") {
        return vec![
            "sudo dnf install -y webkit2gtk4.1-devel openssl-devel curl wget file libappindicator-gtk3-devel librsvg2-devel libxdo-devel pkgconf-pkg-config".to_string(),
            "sudo dnf group install -y c-development".to_string(),
        ];
    }
    vec![
        "sudo apt-get update".to_string(),
        "sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config".to_string(),
        "or use the equivalent Tauri prerequisite packages for your Linux distribution".to_string(),
    ]
}

fn format_prerequisite_issues(issues: &[PrerequisiteIssue]) -> String {
    let mut details = String::new();
    for issue in issues {
        details.push_str("- ");
        details.push_str(issue.name);
        details.push('\n');
        for command in &issue.manual_commands {
            details.push_str("  run: ");
            details.push_str(command);
            details.push('\n');
        }
    }
    details
}

fn validate_source_archive_version(source_dir: &Path) -> Result<(), DesktopLauncherError> {
    validate_source_metadata_version(
        &source_dir.join("Cargo.toml"),
        toml_version(
            &source_dir.join("Cargo.toml"),
            &["workspace", "package", "version"],
        )?,
    )?;
    validate_source_metadata_version(
        &source_dir.join("apps/desktop/src-tauri/Cargo.toml"),
        toml_version(
            &source_dir.join("apps/desktop/src-tauri/Cargo.toml"),
            &["package", "version"],
        )?,
    )?;
    validate_source_metadata_version(
        &source_dir.join("apps/desktop/package.json"),
        json_package_version(&source_dir.join("apps/desktop/package.json"))?,
    )?;
    validate_source_metadata_version(
        &source_dir.join("apps/desktop/src-tauri/tauri.conf.json"),
        json_package_version(&source_dir.join("apps/desktop/src-tauri/tauri.conf.json"))?,
    )
}

fn toml_version(path: &Path, keys: &[&str]) -> Result<String, DesktopLauncherError> {
    let contents = fs::read_to_string(path).map_err(|source| DesktopLauncherError::Repair {
        path: path.to_path_buf(),
        source,
    })?;
    let parsed = contents.parse::<toml::Value>().map_err(|source| {
        DesktopLauncherError::InvalidSourceMetadata {
            path: path.to_path_buf(),
            details: source.to_string(),
        }
    })?;
    let mut value = &parsed;
    for key in keys {
        value = value
            .get(*key)
            .ok_or_else(|| DesktopLauncherError::MissingSourceVersion {
                path: path.to_path_buf(),
            })?;
    }
    value.as_str().map(ToOwned::to_owned).ok_or_else(|| {
        DesktopLauncherError::MissingSourceVersion {
            path: path.to_path_buf(),
        }
    })
}

fn json_package_version(path: &Path) -> Result<String, DesktopLauncherError> {
    let contents = fs::read_to_string(path).map_err(|source| DesktopLauncherError::Repair {
        path: path.to_path_buf(),
        source,
    })?;
    let parsed = serde_json::from_str::<serde_json::Value>(&contents).map_err(|source| {
        DesktopLauncherError::InvalidSourceMetadata {
            path: path.to_path_buf(),
            details: source.to_string(),
        }
    })?;
    parsed
        .get("version")
        .and_then(|version| version.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| DesktopLauncherError::MissingSourceVersion {
            path: path.to_path_buf(),
        })
}

fn validate_source_metadata_version(
    path: &Path,
    actual: String,
) -> Result<(), DesktopLauncherError> {
    if actual == desktop_version() {
        Ok(())
    } else {
        Err(DesktopLauncherError::SourceVersionMismatch {
            path: path.to_path_buf(),
            expected: desktop_version().to_string(),
            actual,
        })
    }
}

fn install_source_built_bundle_from_source_dir<R: DesktopCommandRunner>(
    cache_root: &Path,
    cache_dir: &Path,
    source_dir: &Path,
    runner: &mut R,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let staging = tempfile::Builder::new()
        .prefix(".source-bundle-")
        .tempdir_in(cache_root)
        .map_err(|source| DesktopLauncherError::Repair {
            path: cache_root.to_path_buf(),
            source,
        })?;
    let staged_bundle = staging.path().join("bundle");
    build_source_bundle_from_source_dir(source_dir, &staged_bundle, runner)?;

    if cache_dir.exists() {
        remove_cache_entry(cache_root, cache_dir)?;
    }
    fs::rename(&staged_bundle, cache_dir).map_err(|source| DesktopLauncherError::Repair {
        path: cache_dir.to_path_buf(),
        source,
    })?;
    println!(
        "Installed source-built desktop bundle at {}",
        cache_dir.display()
    );
    verify_bundle(cache_dir)
}

fn build_source_bundle_from_source_dir<R: DesktopCommandRunner>(
    source_dir: &Path,
    bundle_dir: &Path,
    runner: &mut R,
) -> Result<(), DesktopLauncherError> {
    validate_source_archive_version(source_dir)?;
    validate_frontend_lockfile(source_dir)?;
    println!("Installing desktop frontend dependencies.");
    runner.run(&DesktopCommand::new("npm", &["ci", "--include=dev"]).in_dir(source_dir))?;
    let tauri_dir = source_dir.join("apps/desktop/src-tauri");
    let source_target_dir = source_build_target_dir(source_dir);
    println!("Building desktop app from source.");
    runner.run(
        &DesktopCommand::new_os(
            "cargo",
            vec![
                OsString::from("build"),
                OsString::from("--release"),
                OsString::from("--locked"),
                OsString::from("--target-dir"),
                source_target_dir.as_os_str().to_os_string(),
            ],
        )
        .in_dir(&tauri_dir),
    )?;

    let built_executable = source_target_dir
        .join("release")
        .join(source_build_executable_name(current_platform()));
    let installed_executable = source_build_executable_name(current_platform());
    fs::create_dir_all(bundle_dir).map_err(|source| DesktopLauncherError::Repair {
        path: bundle_dir.to_path_buf(),
        source,
    })?;
    let target = bundle_dir.join(&installed_executable);
    fs::copy(&built_executable, &target).map_err(|source| {
        DesktopLauncherError::MissingExecutable {
            path: built_executable.clone(),
            source,
        }
    })?;
    let permissions = fs::metadata(&built_executable)
        .map_err(|source| DesktopLauncherError::MissingExecutable {
            path: built_executable.clone(),
            source,
        })?
        .permissions();
    fs::set_permissions(&target, permissions).map_err(|source| DesktopLauncherError::Repair {
        path: target.clone(),
        source,
    })?;

    let manifest = DesktopBundleManifest {
        version: desktop_version().to_string(),
        platform: current_platform().to_string(),
        arch: current_arch().to_string(),
        executable: installed_executable,
        sha256: file_sha256(&target)?,
    };
    fs::write(
        bundle_dir.join(MANIFEST_FILE),
        serde_json::to_vec_pretty(&manifest).expect("manifest should serialize"),
    )
    .map_err(|source| DesktopLauncherError::Repair {
        path: bundle_dir.to_path_buf(),
        source,
    })?;
    verify_bundle(bundle_dir)?;
    Ok(())
}

fn validate_frontend_lockfile(source_dir: &Path) -> Result<(), DesktopLauncherError> {
    let path = source_dir.join("package-lock.json");
    if path.is_file() {
        Ok(())
    } else {
        Err(DesktopLauncherError::MissingFrontendLockfile { path })
    }
}

fn source_build_target_dir(source_dir: &Path) -> PathBuf {
    source_dir.join(SOURCE_BUILD_TARGET_DIR)
}

fn source_build_executable_name(platform: &str) -> PathBuf {
    if platform == "windows" {
        PathBuf::from("OpenSymphony.exe")
    } else {
        PathBuf::from("OpenSymphony")
    }
}

fn copy_dir_all(from: &Path, to: &Path) -> Result<(), DesktopLauncherError> {
    fs::create_dir_all(to).map_err(|source| DesktopLauncherError::Repair {
        path: to.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(from).map_err(|source| DesktopLauncherError::MissingBundle {
        path: from.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| DesktopLauncherError::Repair {
            path: to.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let target = to.join(entry.file_name());
        let metadata =
            fs::symlink_metadata(&source_path).map_err(|source| DesktopLauncherError::Repair {
                path: source_path.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(DesktopLauncherError::UnsupportedBundleEntry { path: source_path });
        }
        if metadata.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            let permissions = metadata.permissions();
            fs::copy(entry.path(), &target).map_err(|source| DesktopLauncherError::Repair {
                path: target.clone(),
                source,
            })?;
            fs::set_permissions(&target, permissions).map_err(|source| {
                DesktopLauncherError::Repair {
                    path: target,
                    source,
                }
            })?;
        }
    }
    Ok(())
}

fn remove_cache_entry(cache_root: &Path, cache_dir: &Path) -> Result<(), DesktopLauncherError> {
    remove_cache_entry_for_version(cache_root, cache_dir, desktop_version())
}

fn remove_cache_entry_for_version(
    cache_root: &Path,
    cache_dir: &Path,
    version: &str,
) -> Result<(), DesktopLauncherError> {
    validate_cache_dir_for_version(cache_root, cache_dir, version)?;
    let metadata =
        fs::symlink_metadata(cache_dir).map_err(|source| DesktopLauncherError::Repair {
            path: cache_dir.to_path_buf(),
            source,
        })?;
    if metadata.is_dir() {
        fs::remove_dir_all(cache_dir)
    } else {
        fs::remove_file(cache_dir)
    }
    .map_err(|source| DesktopLauncherError::Repair {
        path: cache_dir.to_path_buf(),
        source,
    })
}

fn reject_parent_components(path: &Path) -> Result<(), DesktopLauncherError> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(DesktopLauncherError::DangerousCacheRoot {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, DesktopLauncherError> {
    raw_file_sha256(path).map_err(|source| DesktopLauncherError::MissingExecutable {
        path: path.to_path_buf(),
        source,
    })
}

fn raw_file_sha256(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn default_cache_root() -> Result<PathBuf, DesktopLauncherError> {
    let home = home_dir().ok_or(DesktopLauncherError::MissingHome)?;
    Ok(home.join(DEFAULT_CACHE_RELATIVE))
}

fn selected_install_root(
    install_path: Option<PathBuf>,
    cache_root: Option<PathBuf>,
) -> Result<PathBuf, DesktopLauncherError> {
    match install_path.or(cache_root) {
        Some(path) => Ok(path),
        None => default_cache_root(),
    }
}

fn home_dir() -> Option<PathBuf> {
    home_dir_from_vars(
        env::var_os("HOME"),
        env::var_os("USERPROFILE"),
        env::var_os("HOMEDRIVE"),
        env::var_os("HOMEPATH"),
    )
}

fn home_dir_from_vars(
    home: Option<OsString>,
    userprofile: Option<OsString>,
    homedrive: Option<OsString>,
    homepath: Option<OsString>,
) -> Option<PathBuf> {
    home.filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            userprofile
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let mut drive = homedrive.filter(|value| !value.is_empty())?;
            let path = homepath.filter(|value| !value.is_empty())?;
            drive.push(path);
            Some(PathBuf::from(drive))
        })
}

fn normalize_cache_root(path: PathBuf) -> Result<PathBuf, DesktopLauncherError> {
    reject_parent_components(&path)?;
    let absolute = if path.is_absolute() {
        path
    } else {
        env::current_dir()
            .map_err(DesktopLauncherError::CurrentDir)?
            .join(path)
    };
    reject_parent_components(&absolute)?;
    if absolute.parent().is_none() {
        return Err(DesktopLauncherError::DangerousCacheRoot { path: absolute });
    }
    Ok(absolute)
}

fn validate_cache_dir(cache_root: &Path, cache_dir: &Path) -> Result<(), DesktopLauncherError> {
    validate_cache_dir_for_version(cache_root, cache_dir, desktop_version())
}

fn validate_cache_dir_for_version(
    cache_root: &Path,
    cache_dir: &Path,
    version: &str,
) -> Result<(), DesktopLauncherError> {
    reject_parent_components(cache_root)?;
    reject_parent_components(cache_dir)?;
    reject_symlink(cache_root)?;
    reject_symlink(cache_dir)?;
    if cache_root.parent().is_none()
        || !cache_dir.starts_with(cache_root)
        || cache_dir.file_name().and_then(|name| name.to_str()) != Some(version)
    {
        return Err(DesktopLauncherError::DangerousCacheRoot {
            path: cache_root.to_path_buf(),
        });
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), DesktopLauncherError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(DesktopLauncherError::DangerousCacheRoot {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn resolve_manifest_executable(
    cache_dir: &Path,
    executable: &Path,
) -> Result<PathBuf, DesktopLauncherError> {
    if executable.is_absolute()
        || executable.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return Err(DesktopLauncherError::UnsafeExecutablePath {
            path: executable.to_path_buf(),
        });
    }

    let candidate = cache_dir.join(executable);
    let canonical_cache =
        cache_dir
            .canonicalize()
            .map_err(|source| DesktopLauncherError::Repair {
                path: cache_dir.to_path_buf(),
                source,
            })?;
    let canonical_executable =
        candidate
            .canonicalize()
            .map_err(|source| DesktopLauncherError::MissingExecutable {
                path: candidate.clone(),
                source,
            })?;
    if !canonical_executable.starts_with(&canonical_cache) {
        return Err(DesktopLauncherError::UnsafeExecutablePath { path: candidate });
    }
    Ok(canonical_executable)
}

fn desktop_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn current_platform() -> &'static str {
    env::consts::OS
}

fn current_arch() -> &'static str {
    env::consts::ARCH
}

#[derive(Debug, Error)]
enum DesktopLauncherError {
    #[error(
        "could not find HOME; pass --install-path or set OPENSYMPHONY_DESKTOP_INSTALL_PATH to choose a desktop install root"
    )]
    MissingHome,
    #[error("failed to determine current directory for desktop install root: {0}")]
    CurrentDir(io::Error),
    #[error(
        "refusing unsafe desktop install root {path}\nRepair: choose a non-root install directory under ~/.opensymphony/desktop or another app-owned directory."
    )]
    DangerousCacheRoot { path: PathBuf },
    #[error(
        "desktop bundle is not installed at {path}: {source}\nRepair: rerun with --bundle-dir <path> or set OPENSYMPHONY_DESKTOP_BUNDLE_DIR to a verified OpenSymphony desktop bundle."
    )]
    MissingBundle { path: PathBuf, source: io::Error },
    #[error(
        "desktop executable is missing at {path}: {source}\nRepair: remove the cached version directory and rerun with --bundle-dir <path>."
    )]
    MissingExecutable { path: PathBuf, source: io::Error },
    #[error(
        "desktop manifest {path} is invalid: {source}\nRepair: rerun with a freshly built desktop bundle."
    )]
    InvalidManifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error(
        "desktop manifest executable path {path} is unsafe\nRepair: use a relative executable path that stays inside the cached bundle."
    )]
    UnsafeExecutablePath { path: PathBuf },
    #[error(
        "desktop bundle entry {path} is a symlink, which local bundle materialization does not yet preserve\nRepair: pass an expanded bundle without symlinks or use a future signed/downloaded bundle format."
    )]
    UnsupportedBundleEntry { path: PathBuf },
    #[error(
        "cached desktop bundle has version {actual}, expected {expected} in {path}\nRepair: remove the cached version directory and rerun with a matching bundle."
    )]
    WrongVersion {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error(
        "cached desktop bundle has platform {actual}, expected {expected} in {path}\nRepair: install a bundle built for this platform."
    )]
    WrongPlatform {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error(
        "cached desktop bundle has architecture {actual}, expected {expected} in {path}\nRepair: install a bundle built for this CPU architecture."
    )]
    WrongArch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error(
        "cached desktop checksum mismatch for {path}: expected {expected}, got {actual}\nRepair: remove the cached version directory and rerun with --bundle-dir <path>."
    )]
    BadChecksum {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error(
        "failed to download desktop release index from {url}: {source}\nRepair: check network access, set OPENSYMPHONY_DESKTOP_RELEASE_INDEX_URL to a compatible release index, or use --bundle-dir <path> for a local bundle."
    )]
    ReleaseIndexDownload { url: String, source: reqwest::Error },
    #[error(
        "desktop release index {url} is invalid: {source}\nRepair: publish a schema_version 1 desktop release index or set OPENSYMPHONY_DESKTOP_RELEASE_INDEX_URL to one."
    )]
    InvalidReleaseIndex {
        url: String,
        source: serde_json::Error,
    },
    #[error(
        "desktop release index {url} has unsupported schema version {schema_version}\nRepair: publish a schema_version 1 desktop release index."
    )]
    UnsupportedReleaseIndex { url: String, schema_version: u32 },
    #[error(
        "desktop release index {url} has no asset for version {version} platform {platform} architecture {arch}\nRepair: publish a compatible desktop asset or use --bundle-dir <path> for a local bundle."
    )]
    NoCompatibleReleaseAsset {
        url: String,
        version: String,
        platform: String,
        arch: String,
    },
    #[error(
        "desktop release asset {url} uses unsupported checksum algorithm {algorithm}\nRepair: publish sha256 release metadata."
    )]
    UnsupportedReleaseChecksum { url: String, algorithm: String },
    #[error(
        "desktop release asset {url} declares launch arguments this launcher does not support yet\nRepair: publish an asset with an empty launch_target.args array."
    )]
    UnsupportedLaunchArgs { url: String },
    #[error(
        "failed to download desktop release asset from {url}: {source}\nRepair: check network access, publish the archive before the release index, or use --bundle-dir <path> for a local bundle."
    )]
    ReleaseAssetDownload { url: String, source: reqwest::Error },
    #[error("failed to read desktop release archive {path}: {source}")]
    ReleaseArchiveRead { path: PathBuf, source: io::Error },
    #[error(
        "desktop release archive checksum mismatch for {path}: expected {expected}, got {actual}\nRepair: publish a release index whose checksum matches the uploaded archive."
    )]
    ReleaseArchiveChecksum {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error(
        "desktop release archive entry {path} is unsafe\nRepair: publish an archive whose entries stay inside the bundle root."
    )]
    UnsafeArchiveEntry { path: PathBuf },
    #[error(
        "desktop release launch target mismatch: metadata expected {expected}, installed manifest used {actual}\nRepair: publish matching launch_target metadata and bundle manifest."
    )]
    LaunchTargetMismatch { expected: PathBuf, actual: PathBuf },
    #[error("desktop release installer task failed: {source}")]
    ReleaseInstallTask { source: tokio::task::JoinError },
    #[error("failed to repair desktop cache at {path}: {source}")]
    Repair { path: PathBuf, source: io::Error },
    #[error(
        "desktop source build prerequisites are missing.\nInstall the missing prerequisites, then rerun `opensymphony app`:\n{details}"
    )]
    MissingPrerequisites { details: String },
    #[error(
        "no compatible desktop bundle is installed for OpenSymphony {version} on {platform}/{arch}.\nDry run did not install prerequisites, download sources, or build locally. Rerun without --dry-run to build from source."
    )]
    DryRunSourceBuildRequired {
        version: String,
        platform: String,
        arch: String,
    },
    #[error("failed to download desktop source archive from {url}: {source}")]
    SourceDownload { url: String, source: reqwest::Error },
    #[error("failed to download desktop source archive from {url}: HTTP {status}")]
    SourceDownloadStatus { url: String, status: u16 },
    #[error(
        "desktop source metadata {path} is invalid: {details}\nRepair: use the OpenSymphony source archive matching CLI version {expected}.",
        expected = desktop_version()
    )]
    InvalidSourceMetadata { path: PathBuf, details: String },
    #[error(
        "desktop source metadata {path} is missing a version field\nRepair: use the OpenSymphony source archive matching CLI version {expected}.",
        expected = desktop_version()
    )]
    MissingSourceVersion { path: PathBuf },
    #[error(
        "desktop source archive version {actual} in {path} does not match CLI version {expected}\nRepair: use the OpenSymphony source archive matching CLI version {expected}."
    )]
    SourceVersionMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("desktop source archive is missing pinned frontend dependencies at {path}")]
    MissingFrontendLockfile { path: PathBuf },
    #[error("failed to run desktop source build command `{command}`: {source}")]
    SourceBuildCommandIo { command: String, source: io::Error },
    #[error("desktop source build command failed: `{command}` exited with {status}")]
    SourceBuildCommandFailed { command: String, status: String },
    #[error("failed to launch desktop app at {path}: {source}")]
    Launch { path: PathBuf, source: io::Error },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use flate2::{Compression, write::GzEncoder};
    use std::{
        collections::{BTreeSet, HashMap},
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };
    use tempfile::TempDir;

    use super::super::{Cli, Command as CliCommand};

    #[derive(Default)]
    struct FakeRunner {
        programs: BTreeSet<String>,
        successful_checks: BTreeSet<String>,
        failing_runs: BTreeSet<String>,
        runs: Vec<String>,
    }

    impl FakeRunner {
        fn with_programs(programs: &[&str]) -> Self {
            Self {
                programs: programs
                    .iter()
                    .map(|program| (*program).to_string())
                    .collect(),
                ..Self::default()
            }
        }

        fn fail_run(mut self, program: &str, args: &[&str]) -> Self {
            self.failing_runs.insert(command_key(program, args));
            self
        }
    }

    impl DesktopCommandRunner for FakeRunner {
        fn program_exists(&self, program: &str) -> bool {
            self.programs.contains(program)
        }

        fn command_succeeds(&self, program: &str, args: &[&str]) -> bool {
            self.successful_checks.contains(&command_key(program, args))
        }

        fn run(&mut self, command: &DesktopCommand) -> Result<(), DesktopLauncherError> {
            let display = command.display();
            self.runs.push(display.clone());
            if self.failing_runs.contains(&display) {
                Err(DesktopLauncherError::SourceBuildCommandFailed {
                    command: display,
                    status: "exit status: 1".to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    fn command_key(program: &str, args: &[&str]) -> String {
        if args.is_empty() {
            program.to_string()
        } else {
            format!("{} {}", program, args.join(" "))
        }
    }

    fn mark_pkg_config_module_ready(runner: &mut FakeRunner, module: &str) {
        runner
            .successful_checks
            .insert(command_key("pkg-config", &["--exists", module]));
    }

    fn mark_linux_tauri_modules_ready(runner: &mut FakeRunner) {
        mark_pkg_config_module_ready(runner, "webkit2gtk-4.1");
        mark_pkg_config_module_ready(runner, "openssl");
        mark_pkg_config_module_ready(runner, "librsvg-2.0");
        mark_pkg_config_module_ready(runner, "libxdo");
        mark_pkg_config_module_ready(runner, "appindicator3-0.1");
    }

    fn write_source_metadata(source: &Path, version: &str) {
        fs::create_dir_all(source.join("apps/desktop/src-tauri")).expect("create fake tauri dir");
        fs::create_dir_all(source.join("apps/desktop")).expect("create fake desktop dir");
        fs::write(
            source.join("Cargo.toml"),
            format!("[workspace.package]\nversion = \"{version}\"\n"),
        )
        .expect("write root cargo metadata");
        fs::write(
            source.join("apps/desktop/src-tauri/Cargo.toml"),
            format!("[package]\nname = \"opensymphony-desktop\"\nversion = \"{version}\"\n"),
        )
        .expect("write tauri cargo metadata");
        fs::write(
            source.join("apps/desktop/package.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .expect("write desktop package metadata");
        fs::write(
            source.join("apps/desktop/src-tauri/tauri.conf.json"),
            format!(r#"{{"version":"{version}"}}"#),
        )
        .expect("write tauri config metadata");
        fs::write(
            source.join("package-lock.json"),
            r#"{"name":"opensymphony-frontend","lockfileVersion":3,"packages":{}}"#,
        )
        .expect("write frontend lockfile");
    }

    #[test]
    fn app_and_desktop_alias_parse_to_same_command() {
        let app = Cli::try_parse_from(["opensymphony", "app", "--dry-run"])
            .expect("app command should parse");
        let desktop = Cli::try_parse_from(["opensymphony", "desktop", "--dry-run"])
            .expect("desktop alias should parse");

        assert!(matches!(app.command, CliCommand::App(_)));
        assert!(matches!(desktop.command, CliCommand::App(_)));
    }

    #[test]
    fn cache_path_is_versioned_under_opensymphony_desktop() {
        let home = Path::new("/tmp/example-home");
        let cache = home.join(DEFAULT_CACHE_RELATIVE).join(desktop_version());

        assert_eq!(
            cache,
            PathBuf::from(format!(
                "/tmp/example-home/.opensymphony/desktop/{}",
                desktop_version()
            ))
        );
    }

    #[test]
    fn install_path_is_a_versioned_bundle_root() {
        let cli = Cli::try_parse_from([
            "opensymphony",
            "app",
            "--install-path",
            "/tmp/opensymphony-desktop",
            "--dry-run",
        ])
        .expect("app command should parse");
        let CliCommand::App(args) = cli.command else {
            panic!("app command expected");
        };
        let root = normalize_cache_root(args.install_path.expect("install path"))
            .expect("install path should normalize");

        assert_eq!(
            root.join(desktop_version()),
            PathBuf::from(format!("/tmp/opensymphony-desktop/{}", desktop_version()))
        );
    }

    #[test]
    fn explicit_install_path_skips_default_home_lookup() {
        let root = selected_install_root(Some(PathBuf::from("/tmp/custom-desktop")), None)
            .expect("explicit install path should be accepted before HOME lookup");

        assert_eq!(root, PathBuf::from("/tmp/custom-desktop"));
    }

    #[test]
    fn release_index_parses_download_contract() {
        let raw = include_str!("../tests/fixtures/desktop-release-index.json");

        let index = parse_release_index(raw).expect("release index should parse");
        let asset = &index.assets[0];

        assert_eq!(index.schema_version, 1);
        assert_eq!(asset.version, "2.7.0");
        assert_eq!(asset.platform, "macos");
        assert_eq!(asset.arch, "aarch64");
        assert_eq!(
            asset.url,
            "https://github.com/kumanday/OpenSymphony/releases/download/v2.7.0/opensymphony-desktop-v2.7.0-macos-aarch64.tar.gz"
        );
        assert_eq!(asset.checksum.algorithm, "sha256");
        assert_eq!(asset.checksum.value.len(), 64);
        assert_eq!(
            asset.launch_target.executable,
            PathBuf::from("OpenSymphony")
        );
        assert!(asset.launch_target.args.is_empty());
    }

    #[test]
    fn release_url_display_url_redacts_credentials_query_and_fragment() {
        let display =
            release_url_display_url("https://user:secret@example.com/index.json?token=secret#frag");

        assert_eq!(display, "https://example.com/index.json");
    }

    #[test]
    fn release_index_download_error_redacts_reqwest_url() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused port");
        let addr = listener.local_addr().expect("local address");
        drop(listener);
        let url = format!("http://user:secret@{addr}/index.json?token=secret#fragment");

        let error = download_release_index(&url).expect_err("closed port should fail");
        let message = error.to_string();

        assert!(message.contains(&format!("http://{addr}/index.json")));
        assert!(!message.contains("user:secret"));
        assert!(!message.contains("token=secret"));
        assert!(!message.contains("#fragment"));
    }

    #[test]
    fn semver_selection_treats_numeric_segments_as_numbers() {
        let index = DesktopReleaseIndex {
            schema_version: 1,
            assets: vec![
                desktop_release_asset("2.9.9", "http://example.com/2.9.9.tar.gz", "0"),
                desktop_release_asset("2.10.0", "http://example.com/2.10.0.tar.gz", "0"),
            ],
        };

        let asset =
            newest_compatible_release_asset(&index, "http://example.com/index.json", "2.8.0")
                .expect("candidate lookup should succeed")
                .expect("newer asset should exist");

        assert_eq!(asset.version, "2.10.0");
    }

    #[test]
    fn update_prompt_defaults_enter_to_yes() {
        let mut output = Vec::new();
        let accepted = prompt_update_decision(&mut &b"\n"[..], &mut output, true, "2.7.0", "2.7.1")
            .expect("prompt should succeed");
        let rendered = String::from_utf8(output).expect("prompt output utf8");

        assert!(accepted);
        assert!(rendered.contains("Update before launch? [Y/n]"));
    }

    #[test]
    fn update_prompt_accepts_explicit_no() {
        let mut output = Vec::new();
        let accepted =
            prompt_update_decision(&mut &b"n\n"[..], &mut output, true, "2.7.0", "2.7.1")
                .expect("prompt should succeed");

        assert!(!accepted);
    }

    #[test]
    fn noninteractive_update_prompt_defaults_to_yes_without_reading() {
        let mut output = Vec::new();
        let accepted = prompt_update_decision(&mut &b""[..], &mut output, false, "2.7.0", "2.7.1")
            .expect("noninteractive default should succeed");

        assert!(accepted);
        assert!(output.is_empty());
    }

    #[test]
    fn release_asset_download_error_redacts_reqwest_url() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind unused port");
        let addr = listener.local_addr().expect("local address");
        drop(listener);
        let temp = TempDir::new().expect("tempdir");
        let url = format!("http://user:secret@{addr}/bundle.tar.gz?token=secret#fragment");

        let error = download_file(&url, &temp.path().join("bundle.tar.gz"))
            .expect_err("closed port should fail");
        let message = error.to_string();

        assert!(message.contains(&format!("http://{addr}/bundle.tar.gz")));
        assert!(!message.contains("user:secret"));
        assert!(!message.contains("token=secret"));
        assert!(!message.contains("#fragment"));
    }

    #[test]
    fn interrupted_release_asset_body_is_download_error() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake server");
        let addr = listener.local_addr().expect("local address");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept request");
            let mut reader = BufReader::new(stream.try_clone().expect("clone stream"));
            let mut line = String::new();
            loop {
                line.clear();
                let read = reader.read_line(&mut line).expect("read request line");
                if read == 0 || line == "\r\n" {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort")
                .expect("write partial response");
        });
        let temp = TempDir::new().expect("tempdir");

        let error = download_file(
            &format!("http://{addr}/bundle.tar.gz"),
            &temp.path().join("bundle.tar.gz"),
        )
        .expect_err("short response body should fail");
        handle.join().expect("server thread should finish");

        assert!(matches!(
            error,
            DesktopLauncherError::ReleaseAssetDownload { .. }
        ));
    }

    #[tokio::test]
    async fn remote_release_installs_to_custom_path() {
        let archive = desktop_release_archive(b"fake desktop");
        let archive_sha = sha256_bytes(&archive);
        let server = FakeReleaseServer::start(|base_url| {
            vec![
                (
                    "/index.json",
                    release_index_body(base_url, archive_sha.as_str()).into_bytes(),
                ),
                ("/bundle.tar.gz", archive),
            ]
        });
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());

        let verified = ensure_verified_bundle(
            install_root.path(),
            &cache_dir,
            None,
            &server.url("/index.json"),
            false,
        )
        .await
        .expect("remote release should install");

        assert_eq!(
            verified.executable,
            cache_dir
                .join("OpenSymphony")
                .canonicalize()
                .expect("installed executable")
        );
        assert!(cache_dir.join(MANIFEST_FILE).is_file());
        assert_eq!(
            server.requests(),
            vec!["/index.json".to_string(), "/bundle.tar.gz".to_string()]
        );
    }

    #[tokio::test]
    async fn remote_promotion_repairs_file_cache_entry() {
        let archive = desktop_release_archive(b"fake desktop");
        let archive_sha = sha256_bytes(&archive);
        let server = FakeReleaseServer::start(|base_url| {
            vec![
                (
                    "/index.json",
                    release_index_body(base_url, archive_sha.as_str()).into_bytes(),
                ),
                ("/bundle.tar.gz", archive),
            ]
        });
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        fs::write(&cache_dir, b"stale cache file").expect("write stale file");

        ensure_verified_bundle(
            install_root.path(),
            &cache_dir,
            None,
            &server.url("/index.json"),
            false,
        )
        .await
        .expect("remote release should replace stale file cache entry");

        assert!(cache_dir.is_dir());
        assert!(cache_dir.join(MANIFEST_FILE).is_file());
    }

    #[tokio::test]
    async fn remote_checksum_failure_does_not_promote_bundle() {
        let archive = desktop_release_archive(b"fake desktop");
        let server = FakeReleaseServer::start(|base_url| {
            vec![
                (
                    "/index.json",
                    release_index_body(
                        base_url,
                        "0000000000000000000000000000000000000000000000000000000000000000",
                    )
                    .into_bytes(),
                ),
                ("/bundle.tar.gz", archive),
            ]
        });
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());

        let error = ensure_verified_bundle(
            install_root.path(),
            &cache_dir,
            None,
            &server.url("/index.json"),
            false,
        )
        .await
        .expect_err("checksum mismatch should fail");

        assert!(matches!(
            error,
            DesktopLauncherError::ReleaseArchiveChecksum { .. }
        ));
        assert!(
            !cache_dir.exists(),
            "failed verification must not promote a versioned bundle"
        );
    }

    #[tokio::test]
    async fn release_index_download_failure_falls_back_to_source_build() {
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        let mut runner = FakeRunner::default();

        let error = ensure_verified_bundle_with(
            install_root.path(),
            &cache_dir,
            None,
            "http://127.0.0.1:9/index.json",
            true,
            &mut runner,
        )
        .await
        .expect_err("source fallback should report missing prerequisites");

        assert!(matches!(
            error,
            DesktopLauncherError::MissingPrerequisites { .. }
        ));
    }

    #[tokio::test]
    async fn existing_verified_bundle_skips_release_download() {
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        write_installed_bundle(&cache_dir, b"fake desktop");

        let verified = ensure_verified_bundle(
            install_root.path(),
            &cache_dir,
            None,
            "http://127.0.0.1:9/index.json",
            false,
        )
        .await
        .expect("verified cache should launch without remote discovery");

        assert_eq!(
            verified.executable,
            cache_dir
                .join("OpenSymphony")
                .canonicalize()
                .expect("installed executable")
        );
    }

    #[tokio::test]
    async fn current_cached_bundle_checks_index_without_asset_download() {
        let archive = desktop_release_archive(b"fake desktop");
        let archive_sha = sha256_bytes(&archive);
        let server = FakeReleaseServer::start(|base_url| {
            vec![(
                "/index.json",
                release_index_body(base_url, archive_sha.as_str()).into_bytes(),
            )]
        });
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        write_installed_bundle(&cache_dir, b"fake desktop");
        let verified = verify_bundle(&cache_dir).expect("cache should verify");

        let launched = maybe_update_verified_bundle(
            install_root.path(),
            verified,
            &server.url("/index.json"),
            false,
        )
        .await;

        assert_eq!(launched.manifest.version, desktop_version());
        assert_eq!(server.requests(), vec!["/index.json".to_string()]);
    }

    #[tokio::test]
    async fn failed_update_launches_existing_verified_bundle() {
        let archive = desktop_release_archive_for_version("2.7.1", b"updated desktop");
        let server = FakeReleaseServer::start(|base_url| {
            vec![
                (
                    "/index.json",
                    release_index_body_for_version(
                        base_url,
                        "2.7.1",
                        "0000000000000000000000000000000000000000000000000000000000000000",
                    )
                    .into_bytes(),
                ),
                ("/bundle.tar.gz", archive),
            ]
        });
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        write_installed_bundle(&cache_dir, b"fake desktop");
        let verified = verify_bundle(&cache_dir).expect("cache should verify");

        let launched = maybe_update_verified_bundle(
            install_root.path(),
            verified,
            &server.url("/index.json"),
            false,
        )
        .await;

        assert_eq!(launched.manifest.version, desktop_version());
        assert!(cache_dir.join(MANIFEST_FILE).is_file());
        assert!(!install_root.path().join("2.7.1").exists());
    }

    #[tokio::test]
    async fn newer_release_promotes_and_launches_updated_bundle() {
        let archive = desktop_release_archive_for_version("2.7.1", b"updated desktop");
        let archive_sha = sha256_bytes(&archive);
        let server = FakeReleaseServer::start(|base_url| {
            vec![
                (
                    "/index.json",
                    release_index_body_for_version(base_url, "2.7.1", archive_sha.as_str())
                        .into_bytes(),
                ),
                ("/bundle.tar.gz", archive),
            ]
        });
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        write_installed_bundle(&cache_dir, b"fake desktop");
        let verified = verify_bundle(&cache_dir).expect("cache should verify");

        let launched = maybe_update_verified_bundle(
            install_root.path(),
            verified,
            &server.url("/index.json"),
            false,
        )
        .await;

        let updated_dir = install_root.path().join("2.7.1");
        assert_eq!(launched.manifest.version, "2.7.1");
        assert_eq!(launched.bundle_dir, updated_dir);
        assert!(updated_dir.join(MANIFEST_FILE).is_file());
        assert_eq!(
            server.requests(),
            vec!["/index.json".to_string(), "/bundle.tar.gz".to_string()]
        );
    }

    #[test]
    fn platform_selection_uses_current_target() {
        assert_eq!(current_platform(), env::consts::OS);
        assert_eq!(current_arch(), env::consts::ARCH);
    }

    #[test]
    fn prerequisite_probe_reports_missing_build_tools() {
        let runner = FakeRunner::default();
        let missing = probe_source_build_prerequisites("linux", &runner);
        let names: Vec<_> = missing.iter().map(|issue| issue.name).collect();

        assert!(names.contains(&"Rust/Cargo"));
        assert!(names.contains(&"Node/npm"));
        assert!(names.contains(&"source archive extraction tool"));
        assert!(names.contains(&"Linux desktop/Tauri dependencies"));
        assert!(
            format_prerequisite_issues(&missing)
                .contains("curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh")
        );
    }

    #[test]
    fn windows_prerequisite_probe_uses_windows_rust_guidance() {
        let runner = FakeRunner::default();
        let missing = probe_source_build_prerequisites("windows", &runner);
        let details = format_prerequisite_issues(&missing);

        assert!(details.contains("winget install --id Rustlang.Rustup --source winget"));
        assert!(details.contains("rustup default stable-msvc"));
        assert!(!details.contains("https://sh.rustup.rs"));
    }

    #[test]
    fn windows_prerequisite_probe_does_not_require_cl_on_path() {
        let runner = FakeRunner::with_programs(&["cargo", "rustc", "node", "npm", "tar"]);
        let missing = probe_source_build_prerequisites("windows", &runner);

        assert!(
            missing.is_empty(),
            "Cargo should surface MSVC Build Tools errors instead of a brittle cl.exe PATH probe"
        );
    }

    #[test]
    fn apt_prerequisite_plan_uses_tauri_packages() {
        let runner = FakeRunner::with_programs(&["sudo", "apt-get"]);
        let commands = platform_install_plan("linux", &runner).expect("apt plan");
        let displays: Vec<_> = commands.iter().map(|command| command.display()).collect();

        assert_eq!(displays[0], "sudo -n apt-get update");
        assert!(displays[1].contains("libwebkit2gtk-4.1-dev"));
        assert!(displays[1].contains("libayatana-appindicator3-dev"));
        assert!(displays[1].contains("pkg-config"));
    }

    #[test]
    fn platform_install_plan_uses_noninteractive_sudo() {
        for manager in ["apt-get", "pacman", "dnf"] {
            let runner = FakeRunner::with_programs(&["sudo", manager]);
            let commands = platform_install_plan("linux", &runner).expect("linux plan");

            assert!(
                commands
                    .iter()
                    .all(|command| command.args.first() == Some(&OsString::from("-n"))),
                "{manager} plan should run sudo in noninteractive mode"
            );
        }
    }

    #[test]
    fn pacman_prerequisite_plan_does_not_upgrade_system() {
        let runner = FakeRunner::with_programs(&["sudo", "pacman"]);
        let commands = platform_install_plan("linux", &runner).expect("pacman plan");
        let displays: Vec<_> = commands.iter().map(|command| command.display()).collect();

        assert_eq!(displays.len(), 1);
        assert!(displays[0].contains("sudo -n pacman -S --needed --noconfirm"));
        assert!(!displays[0].contains("-Syu"));
    }

    #[test]
    fn platform_manual_commands_allow_interactive_sudo() {
        let runner = FakeRunner::with_programs(&["sudo", "apt-get"]);
        let commands = platform_manual_commands("linux", &runner);

        assert_eq!(commands[0], "sudo apt-get update");
        assert!(
            commands.iter().all(|command| !command.contains("sudo -n")),
            "manual prerequisite commands should not force noninteractive sudo"
        );
    }

    #[test]
    fn automatic_platform_install_waits_for_other_prerequisites() {
        let mut runner = FakeRunner::with_programs(&["sudo", "apt-get"]);
        let error = ensure_source_build_prerequisites("linux", &mut runner)
            .expect_err("missing build tools should stop before platform install");

        assert!(runner.runs.is_empty());
        assert!(error.to_string().contains("Rust/Cargo"));
        assert!(error.to_string().contains("Node/npm"));
    }

    #[test]
    fn automatic_platform_install_failure_reports_manual_commands() {
        let mut runner =
            FakeRunner::with_programs(&["cargo", "rustc", "node", "npm", "tar", "sudo", "apt-get"])
                .fail_run("sudo", &["-n", "apt-get", "update"]);

        let error = ensure_source_build_prerequisites("linux", &mut runner)
            .expect_err("failed noninteractive installer should print manual guidance");
        let details = error.to_string();

        assert!(matches!(
            error,
            DesktopLauncherError::MissingPrerequisites { .. }
        ));
        assert!(details.contains("Linux desktop/Tauri dependencies"));
        assert!(details.contains("sudo apt-get update"));
        assert!(!details.contains("sudo -n apt-get update"));
        assert_eq!(runner.runs, vec!["sudo -n apt-get update"]);
    }

    #[test]
    fn linux_prerequisite_probe_requires_full_tauri_inputs() {
        let mut runner =
            FakeRunner::with_programs(&["cargo", "rustc", "node", "npm", "tar", "pkg-config"]);
        mark_linux_tauri_modules_ready(&mut runner);
        let missing = probe_source_build_prerequisites("linux", &runner);

        assert!(
            missing
                .iter()
                .any(|issue| issue.name == "Linux desktop/Tauri dependencies"),
            "missing C compiler should keep Linux prerequisites incomplete"
        );

        runner.programs.insert("cc".to_string());
        let missing = probe_source_build_prerequisites("linux", &runner);

        assert!(missing.is_empty());
    }

    #[test]
    fn linux_prerequisite_probe_accepts_distro_pkg_config_names() {
        let mut runner = FakeRunner::with_programs(&[
            "cargo",
            "rustc",
            "node",
            "npm",
            "tar",
            "pkg-config",
            "cc",
        ]);
        mark_linux_tauri_modules_ready(&mut runner);

        let missing = probe_source_build_prerequisites("linux", &runner);

        assert!(missing.is_empty());
    }

    #[test]
    fn source_archive_display_url_redacts_credentials_query_and_fragment() {
        let display = source_archive_display_url(
            "https://user:secret@example.com/archive.tar.gz?token=secret#frag",
        );

        assert_eq!(display, "https://example.com/archive.tar.gz");
    }

    #[cfg(unix)]
    #[test]
    fn desktop_command_preserves_non_utf8_path_arguments() {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let raw_path = b"/tmp/opensymphony-\xFF".to_vec();
        let command = DesktopCommand::new_os(
            "tar",
            vec![OsString::from("-C"), OsString::from_vec(raw_path.clone())],
        );

        assert_eq!(command.args[1].as_os_str().as_bytes(), raw_path);
    }

    #[tokio::test]
    async fn dry_run_without_cached_bundle_does_not_build_from_source() {
        let cache = TempDir::new().expect("cache tempdir");
        let server = FakeReleaseServer::start(|_| {
            vec![(
                "/index.json",
                br#"{"schema_version":1,"assets":[]}"#.to_vec(),
            )]
        });
        let error = run_app(AppArgs {
            bundle_dir: None,
            install_path: Some(cache.path().to_path_buf()),
            cache_root: None,
            release_index_url: Some(server.url("/index.json")),
            dry_run: true,
            no_update: false,
        })
        .await
        .expect_err("dry run should report missing source-build target without building");

        assert!(matches!(
            error,
            DesktopLauncherError::DryRunSourceBuildRequired { .. }
        ));
        assert!(!cache.path().join(desktop_version()).exists());
        assert!(
            server.requests().is_empty(),
            "dry run must not download or install remote bundles"
        );
    }

    #[tokio::test]
    async fn source_fallback_validates_cache_root_before_installing_prerequisites() {
        let parent = TempDir::new().expect("cache parent tempdir");
        let cache_root = parent.path().join("desktop-cache-file");
        fs::write(&cache_root, b"not a directory").expect("write cache root file");
        let cache_dir = cache_root.join(desktop_version());
        let mut runner =
            FakeRunner::with_programs(&["cargo", "rustc", "node", "npm", "tar", "sudo", "apt-get"]);

        let error = build_source_fallback(&cache_root, &cache_dir, &mut runner)
            .await
            .expect_err("file cache root should fail before installing packages");

        assert!(matches!(
            error,
            DesktopLauncherError::Repair { ref path, .. } if path == &cache_root
        ));
        assert!(
            runner.runs.is_empty(),
            "package installers must not run before cache root is writable"
        );
    }

    #[tokio::test]
    async fn invalid_local_bundle_reports_verification_error_without_source_fallback() {
        let local = TempDir::new().expect("local bundle tempdir");
        let cache = TempDir::new().expect("cache tempdir");
        let manifest = DesktopBundleManifest {
            version: "0.0.0".to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: "bad".to_string(),
        };
        fs::write(
            local.path().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write invalid manifest");

        let cache_dir = cache.path().join(desktop_version());
        let mut runner = FakeRunner::default();
        let error = ensure_verified_bundle_with(
            cache.path(),
            &cache_dir,
            Some(local.path()),
            "http://127.0.0.1:9/index.json",
            false,
            &mut runner,
        )
        .await
        .expect_err("invalid local bundle should report its verification error");

        assert!(matches!(
            error,
            DesktopLauncherError::WrongVersion { ref actual, .. } if actual == "0.0.0"
        ));
        assert!(
            !cache_dir.exists(),
            "failed local bundle copy should not remain cached"
        );
        assert!(
            runner.runs.is_empty(),
            "explicit local bundle failure should not trigger source fallback"
        );
    }

    #[test]
    fn source_build_failure_reports_failed_command() {
        let source = TempDir::new().expect("source tempdir");
        let bundle = TempDir::new().expect("bundle tempdir");
        write_source_metadata(source.path(), desktop_version());
        let target_dir = source_build_target_dir(source.path());
        let target_dir_arg = target_dir.to_str().expect("target dir utf8");
        let mut runner = FakeRunner::default().fail_run(
            "cargo",
            &[
                "build",
                "--release",
                "--locked",
                "--target-dir",
                target_dir_arg,
            ],
        );

        let error = build_source_bundle_from_source_dir(source.path(), bundle.path(), &mut runner)
            .expect_err("cargo build failure should abort source build");

        assert!(error.to_string().contains("cargo build --release"));
        assert!(!bundle.path().join(MANIFEST_FILE).exists());
    }

    #[test]
    fn source_build_promotes_fake_build_and_verifies() {
        let source = TempDir::new().expect("source tempdir");
        let cache_root = TempDir::new().expect("cache tempdir");
        write_source_metadata(source.path(), desktop_version());
        let built_executable = source_build_target_dir(source.path())
            .join("release")
            .join(source_build_executable_name(current_platform()));
        fs::create_dir_all(built_executable.parent().expect("target parent"))
            .expect("create fake target dir");
        fs::write(&built_executable, b"fake source-built desktop")
            .expect("write fake built executable");

        let cache_dir = cache_root.path().join(desktop_version());
        let mut runner = FakeRunner::default();
        let verified = install_source_built_bundle_from_source_dir(
            cache_root.path(),
            &cache_dir,
            source.path(),
            &mut runner,
        )
        .expect("source-built bundle should promote and verify");

        assert!(cache_dir.join(MANIFEST_FILE).exists());
        assert_eq!(
            verified.executable,
            cache_dir
                .join(source_build_executable_name(current_platform()))
                .canonicalize()
                .expect("canonical installed executable")
        );
        assert_eq!(runner.runs[0], "npm ci --include=dev");
        assert!(runner.runs[1].starts_with("cargo build --release --locked --target-dir "));
        assert!(runner.runs[1].contains(SOURCE_BUILD_TARGET_DIR));
    }

    #[test]
    fn source_build_requires_frontend_lockfile_before_installing() {
        let source = TempDir::new().expect("source tempdir");
        let bundle = TempDir::new().expect("bundle tempdir");
        write_source_metadata(source.path(), desktop_version());
        fs::remove_file(source.path().join("package-lock.json")).expect("remove lockfile");
        let mut runner = FakeRunner::default();

        let error = build_source_bundle_from_source_dir(source.path(), bundle.path(), &mut runner)
            .expect_err("missing package lock should abort source build");

        assert!(matches!(
            error,
            DesktopLauncherError::MissingFrontendLockfile { .. }
        ));
        assert!(
            runner.runs.is_empty(),
            "source fallback should not install dependencies without a lockfile"
        );
    }

    #[test]
    fn source_build_rejects_mismatched_source_version_before_caching() {
        let source = TempDir::new().expect("source tempdir");
        let bundle = TempDir::new().expect("bundle tempdir");
        write_source_metadata(source.path(), "0.0.0");
        let mut runner = FakeRunner::default();

        let error = build_source_bundle_from_source_dir(source.path(), bundle.path(), &mut runner)
            .expect_err("mismatched source metadata should abort source build");

        assert!(matches!(
            error,
            DesktopLauncherError::SourceVersionMismatch { .. }
        ));
        assert!(runner.runs.is_empty());
        assert!(!bundle.path().join(MANIFEST_FILE).exists());
    }

    #[tokio::test]
    async fn source_archive_download_runs_inside_tokio_runtime() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("test server addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let mut buffer = [0; 512];
            let _ = stream.read(&mut buffer).await.expect("read request");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\narchive")
                .await
                .expect("write response");
        });
        let temp = TempDir::new().expect("download tempdir");
        let archive = temp.path().join("archive.tar.gz");

        download_source_archive(&format!("http://{addr}/archive.tar.gz"), &archive)
            .await
            .expect("archive should download");
        server.await.expect("server task should finish");

        assert_eq!(fs::read(archive).expect("read archive"), b"archive");
    }

    #[tokio::test]
    async fn source_archive_download_error_redacts_credentials_and_query() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("test server addr");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let mut buffer = [0; 512];
            let _ = stream.read(&mut buffer).await.expect("read request");
            stream
                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("write response");
        });
        let temp = TempDir::new().expect("download tempdir");
        let archive = temp.path().join("archive.tar.gz");
        let error = download_source_archive(
            &format!("http://user:secret@{addr}/archive.tar.gz?token=secret"),
            &archive,
        )
        .await
        .expect_err("403 should fail");
        server.await.expect("server task should finish");
        let message = error.to_string();

        assert!(message.contains(&format!("http://{addr}/archive.tar.gz")));
        assert!(!message.contains("user:secret"));
        assert!(!message.contains("token=secret"));
    }

    #[tokio::test]
    async fn source_archive_transport_error_redacts_reqwest_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind unused port");
        let addr = listener.local_addr().expect("test server addr");
        drop(listener);
        let temp = TempDir::new().expect("download tempdir");
        let archive = temp.path().join("archive.tar.gz");
        let error = download_source_archive(
            &format!("http://user:secret@{addr}/archive.tar.gz?token=secret"),
            &archive,
        )
        .await
        .expect_err("closed listener should fail before status");
        let message = error.to_string();

        assert!(message.contains(&format!("http://{addr}/archive.tar.gz")));
        assert!(!message.contains("user:secret"));
        assert!(!message.contains("token=secret"));
    }

    #[test]
    fn source_build_rejects_mismatched_tauri_config_version_before_caching() {
        let source = TempDir::new().expect("source tempdir");
        let bundle = TempDir::new().expect("bundle tempdir");
        write_source_metadata(source.path(), desktop_version());
        fs::write(
            source.path().join("apps/desktop/src-tauri/tauri.conf.json"),
            r#"{"version":"0.0.0"}"#,
        )
        .expect("write mismatched tauri config");
        let mut runner = FakeRunner::default();

        let error = build_source_bundle_from_source_dir(source.path(), bundle.path(), &mut runner)
            .expect_err("mismatched tauri metadata should abort source build");

        assert!(matches!(
            error,
            DesktopLauncherError::SourceVersionMismatch { ref path, .. }
                if path.ends_with("tauri.conf.json")
        ));
        assert!(runner.runs.is_empty());
        assert!(!bundle.path().join(MANIFEST_FILE).exists());
    }

    #[test]
    fn source_build_promotion_repairs_file_cache_entry() {
        let source = TempDir::new().expect("source tempdir");
        let cache_root = TempDir::new().expect("cache tempdir");
        write_source_metadata(source.path(), desktop_version());
        let built_executable = source_build_target_dir(source.path())
            .join("release")
            .join(source_build_executable_name(current_platform()));
        fs::create_dir_all(built_executable.parent().expect("target parent"))
            .expect("create fake target dir");
        fs::write(&built_executable, b"fake source-built desktop")
            .expect("write fake built executable");
        let cache_dir = cache_root.path().join(desktop_version());
        fs::write(&cache_dir, b"corrupt cache entry").expect("write file cache entry");
        let mut runner = FakeRunner::default();

        let verified = install_source_built_bundle_from_source_dir(
            cache_root.path(),
            &cache_dir,
            source.path(),
            &mut runner,
        )
        .expect("file cache entry should be repaired by source promotion");

        assert!(cache_dir.is_dir());
        assert_eq!(
            verified.executable,
            cache_dir
                .join(source_build_executable_name(current_platform()))
                .canonicalize()
                .expect("canonical installed executable")
        );
    }

    #[test]
    fn home_dir_uses_unix_home_first() {
        let home = home_dir_from_vars(
            Some(OsString::from("/home/alice")),
            Some(OsString::from("C:\\Users\\alice")),
            None,
            None,
        )
        .expect("home should resolve");

        assert_eq!(home, PathBuf::from("/home/alice"));
    }

    #[test]
    fn home_dir_falls_back_to_windows_userprofile() {
        let home = home_dir_from_vars(None, Some(OsString::from("C:\\Users\\alice")), None, None)
            .expect("home should resolve");

        assert_eq!(home, PathBuf::from("C:\\Users\\alice"));
    }

    #[test]
    fn home_dir_falls_back_to_windows_drive_and_path() {
        let home = home_dir_from_vars(
            None,
            None,
            Some(OsString::from("C:")),
            Some(OsString::from("\\Users\\alice")),
        )
        .expect("home should resolve");

        assert_eq!(home, PathBuf::from("C:\\Users\\alice"));
    }

    #[tokio::test]
    async fn local_bundle_materializes_and_verifies_from_manifest() {
        let source = TempDir::new().expect("source tempdir");
        let cache = TempDir::new().expect("cache tempdir");
        let executable = source.path().join("OpenSymphony");
        fs::write(&executable, b"fake desktop").expect("write fake executable");
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: file_sha256(&executable).expect("hash fake executable"),
        };
        fs::write(
            source.path().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let cache_dir = cache.path().join(desktop_version());
        let verified = ensure_verified_bundle(
            cache.path(),
            &cache_dir,
            Some(source.path()),
            "http://127.0.0.1:9/index.json",
            false,
        )
        .await
        .expect("bundle should materialize and verify");

        assert_eq!(
            verified.executable,
            cache_dir
                .join("OpenSymphony")
                .canonicalize()
                .expect("canonical fake executable")
        );
    }

    #[test]
    fn checksum_mismatch_reports_repair_guidance() {
        let cache = TempDir::new().expect("cache tempdir");
        let executable = cache.path().join("OpenSymphony");
        fs::write(&executable, b"fake desktop").expect("write fake executable");
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: "bad".into(),
        };
        fs::write(
            cache.path().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let error = verify_bundle(cache.path()).expect_err("checksum should fail");

        assert!(error.to_string().contains("Repair:"));
    }

    #[test]
    fn checksum_accepts_uppercase_manifest_hex() {
        let cache = TempDir::new().expect("cache tempdir");
        let executable = cache.path().join("OpenSymphony");
        fs::write(&executable, b"fake desktop").expect("write fake executable");
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: file_sha256(&executable)
                .expect("hash fake executable")
                .to_uppercase(),
        };
        fs::write(
            cache.path().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        verify_bundle(cache.path()).expect("uppercase checksum should verify");
    }

    #[test]
    fn manifest_executable_must_stay_inside_cache() {
        let cache = TempDir::new().expect("cache tempdir");
        let executable = cache.path().join("OpenSymphony");
        fs::write(&executable, b"fake desktop").expect("write fake executable");
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("../OpenSymphony"),
            sha256: file_sha256(&executable).expect("hash fake executable"),
        };
        fs::write(
            cache.path().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let error = verify_bundle(cache.path()).expect_err("traversal should fail");

        assert!(matches!(
            error,
            DesktopLauncherError::UnsafeExecutablePath { .. }
        ));
    }

    #[test]
    fn cache_delete_requires_versioned_child_of_cache_root() {
        let cache_root = TempDir::new().expect("cache root");
        let wrong_dir = TempDir::new().expect("wrong dir");

        let error = validate_cache_dir(cache_root.path(), wrong_dir.path())
            .expect_err("wrong cache dir should fail");

        assert!(matches!(
            error,
            DesktopLauncherError::DangerousCacheRoot { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn cache_delete_rejects_versioned_symlink() {
        use std::os::unix::fs::symlink;

        let cache_root = TempDir::new().expect("cache root");
        let outside = TempDir::new().expect("outside target");
        let cache_dir = cache_root.path().join(desktop_version());
        symlink(outside.path(), &cache_dir).expect("create versioned cache symlink");

        let error = validate_cache_dir(cache_root.path(), &cache_dir)
            .expect_err("versioned cache symlink should fail");

        assert!(matches!(
            error,
            DesktopLauncherError::DangerousCacheRoot { .. }
        ));
        assert!(
            outside.path().exists(),
            "validation must not remove symlink target"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cache_delete_rejects_cache_root_symlink() {
        use std::os::unix::fs::symlink;

        let outside = TempDir::new().expect("outside target");
        let link_parent = TempDir::new().expect("link parent");
        let cache_root = link_parent.path().join("desktop");
        let cache_dir = cache_root.join(desktop_version());
        symlink(outside.path(), &cache_root).expect("create cache root symlink");

        let error = validate_cache_dir(&cache_root, &cache_dir)
            .expect_err("cache root symlink should fail");

        assert!(matches!(
            error,
            DesktopLauncherError::DangerousCacheRoot { .. }
        ));
        assert!(
            outside.path().exists(),
            "validation must not remove symlink target"
        );
    }

    #[test]
    fn cache_root_rejects_parent_directory_components() {
        let error = normalize_cache_root(PathBuf::from("cache/../outside"))
            .expect_err("parent-directory cache roots should fail");

        assert!(matches!(
            error,
            DesktopLauncherError::DangerousCacheRoot { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_bundle_copy_preserves_executable_mode() {
        use std::os::unix::fs::PermissionsExt;

        let source = TempDir::new().expect("source tempdir");
        let cache = TempDir::new().expect("cache tempdir");
        let executable = source.path().join("OpenSymphony");
        fs::write(&executable, b"fake desktop").expect("write fake executable");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("set executable mode");
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: file_sha256(&executable).expect("hash fake executable"),
        };
        fs::write(
            source.path().join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let cache_dir = cache.path().join(desktop_version());
        let verified = ensure_verified_bundle(
            cache.path(),
            &cache_dir,
            Some(source.path()),
            "http://127.0.0.1:9/index.json",
            false,
        )
        .await
        .expect("bundle should materialize and verify");
        let mode = fs::metadata(&verified.executable)
            .expect("copied executable metadata")
            .permissions()
            .mode();

        assert_ne!(mode & 0o111, 0);
    }

    fn write_installed_bundle(cache_dir: &Path, executable_contents: &[u8]) {
        fs::create_dir_all(cache_dir).expect("create cache dir");
        let executable = cache_dir.join("OpenSymphony");
        fs::write(&executable, executable_contents).expect("write fake executable");
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: file_sha256(&executable).expect("hash fake executable"),
        };
        fs::write(
            cache_dir.join(MANIFEST_FILE),
            serde_json::to_vec(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");
    }

    fn desktop_release_archive(executable_contents: &[u8]) -> Vec<u8> {
        desktop_release_archive_for_version(desktop_version(), executable_contents)
    }

    fn desktop_release_archive_for_version(version: &str, executable_contents: &[u8]) -> Vec<u8> {
        let manifest = DesktopBundleManifest {
            version: version.to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            executable: PathBuf::from("OpenSymphony"),
            sha256: sha256_bytes(executable_contents),
        };
        let manifest_bytes = serde_json::to_vec(&manifest).expect("serialize manifest");
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut archive = tar::Builder::new(encoder);
        append_bytes(&mut archive, MANIFEST_FILE, manifest_bytes.as_slice());
        append_bytes(&mut archive, "OpenSymphony", executable_contents);
        let encoder = archive.into_inner().expect("finish tar archive");
        encoder.finish().expect("finish gzip archive")
    }

    fn append_bytes(archive: &mut tar::Builder<GzEncoder<Vec<u8>>>, path: &str, bytes: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, path, bytes)
            .expect("append archive entry");
    }

    fn release_index_body(base_url: &str, archive_sha: &str) -> String {
        release_index_body_for_version(base_url, desktop_version(), archive_sha)
    }

    fn release_index_body_for_version(base_url: &str, version: &str, archive_sha: &str) -> String {
        serde_json::json!({
            "schema_version": 1,
            "assets": [{
                "version": version,
                "platform": current_platform(),
                "arch": current_arch(),
                "url": format!("{base_url}/bundle.tar.gz"),
                "checksum": {
                    "algorithm": "sha256",
                    "value": archive_sha
                },
                "launch_target": {
                    "executable": "OpenSymphony",
                    "args": []
                }
            }]
        })
        .to_string()
    }

    fn desktop_release_asset(version: &str, url: &str, checksum: &str) -> DesktopReleaseAsset {
        DesktopReleaseAsset {
            version: version.to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
            url: url.to_string(),
            checksum: DesktopReleaseChecksum {
                algorithm: "sha256".to_string(),
                value: checksum.to_string(),
            },
            launch_target: DesktopLaunchTarget {
                executable: PathBuf::from("OpenSymphony"),
                args: Vec::new(),
            },
        }
    }

    fn sha256_bytes(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        format!("{:x}", hasher.finalize())
    }

    struct FakeReleaseServer {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl FakeReleaseServer {
        fn start<F>(routes: F) -> Self
        where
            F: FnOnce(&str) -> Vec<(&'static str, Vec<u8>)>,
        {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake release server");
            let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
            let routes: HashMap<String, Vec<u8>> = routes(&base_url)
                .into_iter()
                .map(|(path, body)| (path.to_string(), body))
                .collect();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let thread_requests = Arc::clone(&requests);
            thread::spawn(move || {
                for stream in listener.incoming().take(routes.len()) {
                    let Ok(mut stream) = stream else { continue };
                    let mut reader = BufReader::new(stream.try_clone().expect("clone fake stream"));
                    let mut first_line = String::new();
                    if reader.read_line(&mut first_line).is_err() {
                        continue;
                    }
                    let path = first_line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .to_string();
                    thread_requests
                        .lock()
                        .expect("requests lock")
                        .push(path.clone());
                    let body = routes.get(&path);
                    match body {
                        Some(body) => {
                            write!(
                                stream,
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                                body.len()
                            )
                            .expect("write response header");
                            stream.write_all(body).expect("write response body");
                        }
                        None => {
                            stream
                                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                                .expect("write 404");
                        }
                    }
                }
            });
            Self { base_url, requests }
        }

        fn url(&self, path: &str) -> String {
            format!("{}{}", self.base_url, path)
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().expect("requests lock").clone()
        }
    }
}
