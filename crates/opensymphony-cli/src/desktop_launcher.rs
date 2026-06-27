use std::{
    env, fs, io,
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
    match run_app(args) {
        Ok(executable) => {
            println!("OpenSymphony desktop ready: {}", executable.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(1)
        }
    }
}

fn run_app(args: AppArgs) -> Result<PathBuf, DesktopLauncherError> {
    let cache_root = args.cache_root.unwrap_or(default_cache_root()?);
    let cache_dir = cache_root.join(desktop_version());
    let bundle_dir = args.bundle_dir.as_deref();
    let verified = ensure_verified_bundle(&cache_dir, bundle_dir)?;

    if args.dry_run {
        println!(
            "Dry run: would launch cached OpenSymphony desktop at {}",
            verified.executable.display()
        );
        return Ok(verified.executable);
    }

    Command::new(&verified.executable)
        .spawn()
        .map_err(|source| DesktopLauncherError::Launch {
            path: verified.executable.clone(),
            source,
        })?;
    Ok(verified.executable)
}

fn ensure_verified_bundle(
    cache_dir: &Path,
    bundle_dir: Option<&Path>,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    match verify_bundle(cache_dir) {
        Ok(bundle) => Ok(bundle),
        Err(first_error) => {
            if let Some(bundle_dir) = bundle_dir {
                if cache_dir.exists() {
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

    let executable = cache_dir.join(&manifest.executable);
    let actual = file_sha256(&executable)?;
    if actual != manifest.sha256 {
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
        let target = to.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|source| DesktopLauncherError::Repair {
                path: entry.path(),
                source,
            })?
            .is_dir()
        {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|source| DesktopLauncherError::Repair {
                path: target,
                source,
            })?;
        }
    }
    Ok(())
}

fn file_sha256(path: &Path) -> Result<String, DesktopLauncherError> {
    let bytes = fs::read(path).map_err(|source| DesktopLauncherError::MissingExecutable {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn default_cache_root() -> Result<PathBuf, DesktopLauncherError> {
    let home = env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or(DesktopLauncherError::MissingHome)?;
    Ok(home.join(DEFAULT_CACHE_RELATIVE))
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
        let verified = ensure_verified_bundle(&cache_dir, Some(source.path()))
            .expect("bundle should materialize and verify");

        assert_eq!(verified.executable, cache_dir.join("OpenSymphony"));
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
}
