//! Resolves the Git executable that checkout and memory code spawn.
//!
//! On macOS, `/usr/bin/git` is an `xcrun` trampoline: every invocation looks
//! up the active developer directory before it execs the real binary, which
//! adds roughly 10 ms to each Git process. Checkout verification and code
//! indexing start thousands of short Git processes, so the trampoline cost
//! dominates their wall time. When `git` on `PATH` is that trampoline, the
//! real binary is resolved once with `xcrun --find git`, the same lookup the
//! trampoline performs, and used directly. Every other platform and PATH
//! layout keeps spawning `git` through `PATH`.

use std::{ffi::OsStr, sync::OnceLock};

/// Returns the program to pass to `Command::new` for Git.
pub fn git_program() -> &'static OsStr {
    static PROGRAM: OnceLock<std::ffi::OsString> = OnceLock::new();
    PROGRAM.get_or_init(resolve_git_program).as_os_str()
}

#[cfg(target_os = "macos")]
fn resolve_git_program() -> std::ffi::OsString {
    use std::path::{Path, PathBuf};

    const XCRUN_TRAMPOLINE: &str = "/usr/bin/git";

    fn first_git_on_path() -> Option<PathBuf> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|directory| directory.join("git"))
            .find(|candidate| candidate.is_file())
    }

    let default = std::ffi::OsString::from("git");
    let Some(on_path) = first_git_on_path() else {
        return default;
    };
    if on_path.canonicalize().ok().as_deref() != Some(Path::new(XCRUN_TRAMPOLINE)) {
        return default;
    }
    let Ok(output) = std::process::Command::new("/usr/bin/xcrun")
        .args(["--find", "git"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return default;
    };
    if !output.status.success() {
        return default;
    }
    let Ok(resolved) = String::from_utf8(output.stdout) else {
        return default;
    };
    let resolved = PathBuf::from(resolved.trim());
    if resolved.is_absolute() && resolved.is_file() && resolved != Path::new(XCRUN_TRAMPOLINE) {
        tracing::debug!(git = %resolved.display(), "resolved Git behind the xcrun trampoline");
        resolved.into_os_string()
    } else {
        default
    }
}

#[cfg(not(target_os = "macos"))]
fn resolve_git_program() -> std::ffi::OsString {
    std::ffi::OsString::from("git")
}

#[cfg(test)]
mod tests {
    use super::git_program;

    #[test]
    fn git_program_runs_git() {
        let output = std::process::Command::new(git_program())
            .arg("--version")
            .output()
            .expect("spawn the resolved Git program");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("git version"));
    }
}
