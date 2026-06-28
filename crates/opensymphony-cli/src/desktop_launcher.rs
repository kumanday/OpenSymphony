use std::{
    env,
    ffi::OsString,
    fs,
    fs::File,
    io,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::Args;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MANIFEST_FILE: &str = "opensymphony-desktop-manifest.json";
const DEFAULT_CACHE_RELATIVE: &str = ".opensymphony/desktop";

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
        env = "OPENSYMPHONY_DESKTOP_CACHE_ROOT",
        hide = true,
        help = "Override the desktop cache root; primarily for smoke tests"
    )]
    cache_root: Option<PathBuf>,
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

#[derive(Debug)]
struct VerifiedBundle {
    executable: PathBuf,
}

pub async fn run_command(args: AppArgs) -> ExitCode {
    let dry_run = args.dry_run;
    match run_app(args) {
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

fn run_app(args: AppArgs) -> Result<PathBuf, DesktopLauncherError> {
    let cache_root = normalize_cache_root(args.cache_root.unwrap_or(default_cache_root()?))?;
    let cache_dir = cache_root.join(desktop_version());
    validate_cache_dir(&cache_root, &cache_dir)?;
    let bundle_dir = args.bundle_dir.as_deref();
    let verified = ensure_verified_bundle(&cache_root, &cache_dir, bundle_dir)?;

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
                Err(first_error)
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

    Ok(VerifiedBundle { executable })
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
    let mut file = File::open(path).map_err(|source| DesktopLauncherError::MissingExecutable {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];
    loop {
        let read =
            file.read(&mut buffer)
                .map_err(|source| DesktopLauncherError::MissingExecutable {
                    path: path.to_path_buf(),
                    source,
                })?;
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
        "could not find HOME; set OPENSYMPHONY_DESKTOP_CACHE_ROOT to choose a desktop cache directory"
    )]
    MissingHome,
    #[error("failed to determine current directory for desktop cache root: {0}")]
    CurrentDir(io::Error),
    #[error(
        "refusing unsafe desktop cache root {path}\nRepair: choose a non-root cache directory under ~/.opensymphony/desktop or another app-owned directory."
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
    #[error("failed to repair desktop cache at {path}: {source}")]
    Repair { path: PathBuf, source: io::Error },
    #[error("failed to launch desktop app at {path}: {source}")]
    Launch { path: PathBuf, source: io::Error },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
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
        let verified = ensure_verified_bundle(cache.path(), &cache_dir, Some(source.path()))
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
        let verified = ensure_verified_bundle(cache.path(), &cache_dir, Some(source.path()))
            .expect("bundle should materialize and verify");
        let mode = fs::metadata(&verified.executable)
            .expect("copied executable metadata")
            .permissions()
            .mode();

        assert_ne!(mode & 0o111, 0);
    }
}
