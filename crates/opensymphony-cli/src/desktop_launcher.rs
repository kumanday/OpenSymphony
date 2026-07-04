use std::{
    env,
    ffi::OsString,
    fs,
    fs::File,
    io,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
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

// ponytail: parsed contract lives ahead of download logic; remove allow once COE-528 uses it.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopReleaseIndex {
    schema_version: u32,
    assets: Vec<DesktopReleaseAsset>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopReleaseAsset {
    version: String,
    platform: String,
    arch: String,
    url: String,
    checksum: DesktopReleaseChecksum,
    launch_target: DesktopLaunchTarget,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopReleaseChecksum {
    algorithm: String,
    value: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DesktopLaunchTarget {
    executable: PathBuf,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug)]
struct VerifiedBundle {
    executable: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopCommand {
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
}

impl DesktopCommand {
    fn new(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_string(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
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
            format!("{} {}", self.program, self.args.join(" "))
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
    let cache_root =
        normalize_cache_root(selected_install_root(args.install_path, args.cache_root)?)?;
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
    let mut runner = RealCommandRunner;
    ensure_verified_bundle_with(cache_root, cache_dir, bundle_dir, &mut runner)
}

fn ensure_verified_bundle_with<R: DesktopCommandRunner>(
    cache_root: &Path,
    cache_dir: &Path,
    bundle_dir: Option<&Path>,
    runner: &mut R,
) -> Result<VerifiedBundle, DesktopLauncherError> {
    match verify_bundle(cache_dir) {
        Ok(bundle) => Ok(bundle),
        Err(_first_error) => {
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
                build_source_fallback(cache_root, cache_dir, runner)
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

#[allow(dead_code)]
fn parse_release_index(contents: &str) -> Result<DesktopReleaseIndex, serde_json::Error> {
    serde_json::from_str(contents)
}

fn build_source_fallback<R: DesktopCommandRunner>(
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
    ensure_source_build_prerequisites(current_platform(), runner)?;

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

    let archive = staging.path().join("opensymphony-source.tar.gz");
    let url = source_archive_url();
    println!("Downloading OpenSymphony source archive: {url}");
    download_source_archive(&url, &archive)?;

    println!("Extracting OpenSymphony source archive.");
    runner.run(&DesktopCommand::new(
        "tar",
        &[
            "-xzf",
            archive
                .to_str()
                .ok_or_else(|| DesktopLauncherError::NonUtf8Path {
                    path: archive.clone(),
                })?,
            "-C",
            source_dir
                .to_str()
                .ok_or_else(|| DesktopLauncherError::NonUtf8Path {
                    path: source_dir.clone(),
                })?,
            "--strip-components",
            "1",
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

fn download_source_archive(url: &str, path: &Path) -> Result<(), DesktopLauncherError> {
    let mut response =
        reqwest::blocking::get(url).map_err(|source| DesktopLauncherError::SourceDownload {
            url: url.to_string(),
            source,
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(DesktopLauncherError::SourceDownloadStatus {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    let mut file = File::create(path).map_err(|source| DesktopLauncherError::Repair {
        path: path.to_path_buf(),
        source,
    })?;
    response
        .copy_to(&mut file)
        .map_err(|source| DesktopLauncherError::SourceDownload {
            url: url.to_string(),
            source,
        })?;
    Ok(())
}

fn ensure_source_build_prerequisites<R: DesktopCommandRunner>(
    platform: &str,
    runner: &mut R,
) -> Result<(), DesktopLauncherError> {
    let initial = probe_source_build_prerequisites(platform, runner);
    if initial
        .iter()
        .any(|issue| issue.name == platform_prerequisite_name(platform))
        && let Some(commands) = platform_install_plan(platform, runner)
    {
        println!("Installing desktop platform prerequisites with detected package manager.");
        for command in commands {
            println!("Running prerequisite installer: {}", command.display());
            runner.run(&command)?;
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
            manual_commands: vec![
                "curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh".to_string(),
                "restart your terminal, then run: cargo --version && rustc --version".to_string(),
            ],
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

fn platform_desktop_dependencies_ready<R: DesktopCommandRunner>(
    platform: &str,
    runner: &R,
) -> bool {
    match platform {
        "macos" => runner.command_succeeds("xcode-select", &["-p"]),
        "linux" => {
            runner.program_exists("pkg-config")
                && runner.command_succeeds(
                    "pkg-config",
                    &["--exists", "webkit2gtk-4.1", "openssl", "librsvg-2.0"],
                )
        }
        "windows" => runner.program_exists("cl"),
        _ => false,
    }
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
            DesktopCommand::new("sudo", &["apt-get", "update"]),
            DesktopCommand::new(
                "sudo",
                &[
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
        return Some(vec![
            DesktopCommand::new("sudo", &["pacman", "-Syu", "--noconfirm"]),
            DesktopCommand::new(
                "sudo",
                &[
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
            ),
        ]);
    }
    if runner.program_exists("dnf") {
        return Some(vec![
            DesktopCommand::new(
                "sudo",
                &[
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
            DesktopCommand::new("sudo", &["dnf", "group", "install", "-y", "c-development"]),
        ]);
    }
    None
}

fn platform_manual_commands<R: DesktopCommandRunner>(platform: &str, runner: &R) -> Vec<String> {
    if let Some(commands) = platform_install_plan(platform, runner) {
        return commands
            .into_iter()
            .map(|command| command.display())
            .collect();
    }
    match platform {
        "macos" => vec!["xcode-select --install".to_string()],
        "windows" => vec![
            "winget install --id Microsoft.VisualStudio.2022.BuildTools".to_string(),
            "install Microsoft Edge WebView2 Evergreen Runtime".to_string(),
        ],
        "linux" => vec![
            "sudo apt-get update".to_string(),
            "sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config".to_string(),
            "or use the equivalent Tauri prerequisite packages for your Linux distribution".to_string(),
        ],
        _ => vec![
            "install the Tauri desktop prerequisites for this platform before building"
                .to_string(),
        ],
    }
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
        validate_cache_dir(cache_root, cache_dir)?;
        fs::remove_dir_all(cache_dir).map_err(|source| DesktopLauncherError::Repair {
            path: cache_dir.to_path_buf(),
            source,
        })?;
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
    println!("Installing desktop frontend dependencies.");
    runner.run(&DesktopCommand::new("npm", &["install"]).in_dir(source_dir))?;
    let tauri_dir = source_dir.join("apps/desktop/src-tauri");
    println!("Building desktop app from source.");
    runner.run(&DesktopCommand::new("cargo", &["build", "--release"]).in_dir(&tauri_dir))?;

    let built_executable = source_dir.join(source_build_executable_relative(current_platform()));
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

fn source_build_executable_relative(platform: &str) -> PathBuf {
    Path::new("apps/desktop/src-tauri/target/release").join(source_build_executable_name(platform))
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
    #[error("failed to repair desktop cache at {path}: {source}")]
    Repair { path: PathBuf, source: io::Error },
    #[error(
        "desktop source build prerequisites are missing.\nInstall the missing prerequisites, then rerun `opensymphony app`:\n{details}"
    )]
    MissingPrerequisites { details: String },
    #[error("desktop source archive path is not valid UTF-8: {path}")]
    NonUtf8Path { path: PathBuf },
    #[error("failed to download desktop source archive from {url}: {source}")]
    SourceDownload { url: String, source: reqwest::Error },
    #[error("failed to download desktop source archive from {url}: HTTP {status}")]
    SourceDownloadStatus { url: String, status: u16 },
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
    use std::collections::BTreeSet;
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
        let raw = r#"{
            "schema_version": 1,
            "assets": [
                {
                    "version": "2.7.0",
                    "platform": "macos",
                    "arch": "aarch64",
                    "url": "https://downloads.example.invalid/opensymphony/2.7.0/macos-aarch64.zip",
                    "checksum": {
                        "algorithm": "sha256",
                        "value": "0123456789abcdef"
                    },
                    "launch_target": {
                        "executable": "OpenSymphony.app/Contents/MacOS/OpenSymphony",
                        "args": []
                    }
                }
            ]
        }"#;

        let index = parse_release_index(raw).expect("release index should parse");
        let asset = &index.assets[0];

        assert_eq!(index.schema_version, 1);
        assert_eq!(asset.version, "2.7.0");
        assert_eq!(asset.platform, "macos");
        assert_eq!(asset.arch, "aarch64");
        assert_eq!(
            asset.url,
            "https://downloads.example.invalid/opensymphony/2.7.0/macos-aarch64.zip"
        );
        assert_eq!(asset.checksum.algorithm, "sha256");
        assert_eq!(asset.checksum.value, "0123456789abcdef");
        assert_eq!(
            asset.launch_target.executable,
            PathBuf::from("OpenSymphony.app/Contents/MacOS/OpenSymphony")
        );
        assert!(asset.launch_target.args.is_empty());
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
    fn apt_prerequisite_plan_uses_tauri_packages() {
        let runner = FakeRunner::with_programs(&["sudo", "apt-get"]);
        let commands = platform_install_plan("linux", &runner).expect("apt plan");
        let displays: Vec<_> = commands.iter().map(|command| command.display()).collect();

        assert_eq!(displays[0], "sudo apt-get update");
        assert!(displays[1].contains("libwebkit2gtk-4.1-dev"));
        assert!(displays[1].contains("libayatana-appindicator3-dev"));
        assert!(displays[1].contains("pkg-config"));
    }

    #[test]
    fn source_build_failure_reports_failed_command() {
        let source = TempDir::new().expect("source tempdir");
        let bundle = TempDir::new().expect("bundle tempdir");
        fs::create_dir_all(source.path().join("apps/desktop/src-tauri"))
            .expect("create fake source tree");
        let mut runner = FakeRunner::default().fail_run("cargo", &["build", "--release"]);

        let error = build_source_bundle_from_source_dir(source.path(), bundle.path(), &mut runner)
            .expect_err("cargo build failure should abort source build");

        assert!(error.to_string().contains("cargo build --release"));
        assert!(!bundle.path().join(MANIFEST_FILE).exists());
    }

    #[test]
    fn source_build_promotes_fake_build_and_verifies() {
        let source = TempDir::new().expect("source tempdir");
        let cache_root = TempDir::new().expect("cache tempdir");
        let built_executable = source
            .path()
            .join(source_build_executable_relative(current_platform()));
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
        assert_eq!(runner.runs, ["npm install", "cargo build --release"]);
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
