use std::{
    env,
    ffi::OsString,
    fs,
    fs::File,
    io,
    io::Read,
    path::{Path, PathBuf},
    process::{self, Command, ExitCode},
    time::{Duration, SystemTime, UNIX_EPOCH},
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
    executable: PathBuf,
    manifest: DesktopBundleManifest,
}

pub async fn run_command(args: AppArgs) -> ExitCode {
    let dry_run = args.dry_run;
    match tokio::task::spawn_blocking(move || run_app(args)).await {
        Ok(Ok(executable)) => {
            if !dry_run {
                println!("OpenSymphony desktop ready: {}", executable.display());
            }
            ExitCode::SUCCESS
        }
        Ok(Err(error)) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
        Err(error) => {
            eprintln!("desktop launcher task failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn run_app(args: AppArgs) -> Result<PathBuf, DesktopLauncherError> {
    let cache_root =
        normalize_cache_root(selected_install_root(args.install_path, args.cache_root)?)?;
    let cache_dir = cache_root.join(desktop_version());
    validate_cache_dir(&cache_root, &cache_dir)?;
    let bundle_dir = args.bundle_dir.as_deref();
    let release_index_url = selected_release_index_url(args.release_index_url);
    let verified = ensure_verified_bundle(
        &cache_root,
        &cache_dir,
        bundle_dir,
        release_index_url.as_str(),
    )?;

    if args.dry_run {
        println!(
            "Dry run: would launch cached OpenSymphony desktop at {}",
            verified.executable.display()
        );
        return Ok(verified.executable);
    }

    Command::new(&verified.executable)
        .current_dir(&cache_dir)
        .spawn()
        .map_err(|source| DesktopLauncherError::Launch {
            path: verified.executable.clone(),
            source,
        })?;
    Ok(verified.executable)
}

fn ensure_verified_bundle(
    cache_root: &Path,
    cache_dir: &Path,
    bundle_dir: Option<&Path>,
    release_index_url: &str,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    match verify_bundle(cache_dir) {
        Ok(bundle) => Ok(bundle),
        Err(first_error) => {
            if let Some(bundle_dir) = bundle_dir {
                if cache_dir.exists() {
                    validate_cache_dir(cache_root, cache_dir)?;
                    fs::remove_dir_all(cache_dir).map_err(|source| {
                        DesktopLauncherError::Repair {
                            path: cache_dir.to_path_buf(),
                            source,
                        }
                    })?;
                }
                copy_dir_all(bundle_dir, cache_dir)?;
                verify_bundle(cache_dir)
            } else {
                install_release_bundle(cache_root, cache_dir, release_index_url, first_error)
            }
        }
    }
}

fn verify_bundle(cache_dir: &Path) -> Result<VerifiedBundle, DesktopLauncherError> {
    let manifest_path = cache_dir.join(MANIFEST_FILE);
    let manifest = read_manifest(&manifest_path)?;

    if manifest.version != desktop_version() {
        return Err(DesktopLauncherError::WrongVersion {
            path: manifest_path,
            expected: desktop_version().to_string(),
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

fn install_release_bundle(
    cache_root: &Path,
    cache_dir: &Path,
    release_index_url: &str,
    _cached_error: DesktopLauncherError,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    let index = download_release_index(release_index_url)?;
    let asset = compatible_release_asset(&index, release_index_url)?;
    let stage_dir = unique_stage_dir(cache_root);
    let archive_path = stage_dir.join("bundle.tar.gz");

    let result = (|| {
        fs::create_dir_all(&stage_dir).map_err(|source| DesktopLauncherError::Repair {
            path: stage_dir.clone(),
            source,
        })?;
        download_file(&asset.url, &archive_path)?;
        verify_archive_checksum(&archive_path, asset)?;
        extract_release_archive(&archive_path, &stage_dir)?;
        let verified = verify_bundle_matches_asset(&stage_dir, asset)?;
        promote_verified_bundle(cache_root, cache_dir, &stage_dir)?;
        verify_bundle(cache_dir).map(|promoted| VerifiedBundle {
            executable: promoted.executable,
            manifest: verified.manifest,
        })
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(&stage_dir);
    }
    result
}

fn download_release_index(url: &str) -> Result<DesktopReleaseIndex, DesktopLauncherError> {
    let client =
        release_http_client().map_err(|source| DesktopLauncherError::ReleaseIndexDownload {
            url: url.to_string(),
            source,
        })?;
    let body = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text())
        .map_err(|source| DesktopLauncherError::ReleaseIndexDownload {
            url: url.to_string(),
            source,
        })?;
    let index =
        parse_release_index(&body).map_err(|source| DesktopLauncherError::InvalidReleaseIndex {
            url: url.to_string(),
            source,
        })?;
    if index.schema_version != 1 {
        return Err(DesktopLauncherError::UnsupportedReleaseIndex {
            url: url.to_string(),
            schema_version: index.schema_version,
        });
    }
    Ok(index)
}

fn compatible_release_asset<'a>(
    index: &'a DesktopReleaseIndex,
    release_index_url: &str,
) -> Result<&'a DesktopReleaseAsset, DesktopLauncherError> {
    index
        .assets
        .iter()
        .find(|asset| {
            asset.version == desktop_version()
                && asset.platform == current_platform()
                && asset.arch == current_arch()
        })
        .ok_or_else(|| DesktopLauncherError::NoCompatibleReleaseAsset {
            url: release_index_url.to_string(),
            version: desktop_version().to_string(),
            platform: current_platform().to_string(),
            arch: current_arch().to_string(),
        })
        .and_then(|asset| {
            if asset.checksum.algorithm != "sha256" {
                return Err(DesktopLauncherError::UnsupportedReleaseChecksum {
                    url: asset.url.clone(),
                    algorithm: asset.checksum.algorithm.clone(),
                });
            }
            if !asset.launch_target.args.is_empty() {
                return Err(DesktopLauncherError::UnsupportedLaunchArgs {
                    url: asset.url.clone(),
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
            Ok(asset)
        })
}

fn download_file(url: &str, destination: &Path) -> Result<(), DesktopLauncherError> {
    let client =
        release_http_client().map_err(|source| DesktopLauncherError::ReleaseAssetDownload {
            url: url.to_string(),
            source,
        })?;
    let mut response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|source| DesktopLauncherError::ReleaseAssetDownload {
            url: url.to_string(),
            source,
        })?;
    let mut file =
        File::create(destination).map_err(|source| DesktopLauncherError::ReleaseArchiveRead {
            path: destination.to_path_buf(),
            source,
        })?;
    io::copy(&mut response, &mut file).map_err(|source| {
        DesktopLauncherError::ReleaseArchiveRead {
            path: destination.to_path_buf(),
            source,
        }
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
    let verified = verify_bundle(stage_dir)?;
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
) -> Result<(), DesktopLauncherError> {
    validate_cache_dir(cache_root, cache_dir)?;
    if cache_dir.exists() {
        fs::remove_dir_all(cache_dir).map_err(|source| DesktopLauncherError::Repair {
            path: cache_dir.to_path_buf(),
            source,
        })?;
    }
    fs::rename(stage_dir, cache_dir).map_err(|source| DesktopLauncherError::Repair {
        path: cache_dir.to_path_buf(),
        source,
    })
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

fn unique_stage_dir(cache_root: &Path) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    cache_root.join(format!(".tmp-desktop-install-{}-{millis}", process::id()))
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
    reject_parent_components(cache_root)?;
    reject_parent_components(cache_dir)?;
    reject_symlink(cache_root)?;
    reject_symlink(cache_dir)?;
    if cache_root.parent().is_none()
        || !cache_dir.starts_with(cache_root)
        || cache_dir.file_name().and_then(|name| name.to_str()) != Some(desktop_version())
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
    #[error("failed to repair desktop cache at {path}: {source}")]
    Repair { path: PathBuf, source: io::Error },
    #[error("failed to launch desktop app at {path}: {source}")]
    Launch { path: PathBuf, source: io::Error },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use flate2::{Compression, write::GzEncoder};
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };
    use tempfile::TempDir;

    use super::super::{Cli, Command as CliCommand};

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
    fn remote_release_installs_to_custom_path_and_dry_runs() {
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

        let executable = run_app(AppArgs {
            bundle_dir: None,
            install_path: Some(install_root.path().to_path_buf()),
            cache_root: None,
            release_index_url: Some(server.url("/index.json")),
            dry_run: true,
        })
        .expect("remote release should install and dry-run");

        let cache_dir = install_root.path().join(desktop_version());
        assert_eq!(
            executable,
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

    #[test]
    fn remote_checksum_failure_does_not_promote_bundle() {
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
        )
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

    #[test]
    fn existing_verified_bundle_skips_release_download() {
        let install_root = TempDir::new().expect("install root");
        let cache_dir = install_root.path().join(desktop_version());
        write_installed_bundle(&cache_dir, b"fake desktop");

        let verified = ensure_verified_bundle(
            install_root.path(),
            &cache_dir,
            None,
            "http://127.0.0.1:9/index.json",
        )
        .expect("verified cache should launch without remote discovery");

        assert_eq!(
            verified.executable,
            cache_dir
                .join("OpenSymphony")
                .canonicalize()
                .expect("installed executable")
        );
    }

    #[test]
    fn platform_selection_uses_current_target() {
        assert_eq!(current_platform(), env::consts::OS);
        assert_eq!(current_arch(), env::consts::ARCH);
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

    #[test]
    fn local_bundle_materializes_and_verifies_from_manifest() {
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
        )
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
    #[test]
    fn local_bundle_copy_preserves_executable_mode() {
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
        )
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
        let manifest = DesktopBundleManifest {
            version: desktop_version().to_string(),
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
        serde_json::json!({
            "schema_version": 1,
            "assets": [{
                "version": desktop_version(),
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
