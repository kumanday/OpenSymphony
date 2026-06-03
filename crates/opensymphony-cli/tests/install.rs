use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::opensymphony_openhands::LocalServerTooling;
use tempfile::TempDir;

#[test]
fn install_openhands_materializes_the_default_managed_tool_dir() {
    let home_dir = TempDir::new().expect("home dir should be created");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let uv_log = fake_bin_dir.path().join("uv.log");
    let tool_dir = home_dir.path().join(".opensymphony/openhands-server");

    write_fake_uv(fake_bin_dir.path().join("uv"), &uv_log);
    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["install", "openhands"])
        .env("HOME", home_dir.path())
        .env_remove("USERPROFILE")
        .env("PATH", path_only(fake_bin_dir.path()))
        .output()
        .expect("install command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "install should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("installed pinned OpenHands tooling 1.24.0"),
        "install should report the fresh managed-local bootstrap: stdout={stdout}",
    );
    assert!(
        stdout.contains(&tool_dir.display().to_string()),
        "install should announce the managed tool dir: stdout={stdout}",
    );
    assert_embedded_tooling_files(&tool_dir);
    assert_eq!(
        uv_invocation_count(&uv_log),
        1,
        "uv should run once on first install"
    );
    let canonical_tool_dir =
        std::fs::canonicalize(&tool_dir).expect("tool dir should canonicalize after install");
    assert!(
        std::fs::read_to_string(&uv_log)
            .expect("uv log should exist")
            .contains(&format!(
                "ARGS=sync --directory {} --locked --extra agent-server",
                canonical_tool_dir.display()
            )),
        "install should run the pinned sync command: {}",
        std::fs::read_to_string(&uv_log).expect("uv log should exist"),
    );
}

#[test]
fn install_openhands_is_a_no_op_when_the_managed_tooling_is_already_ready() {
    let home_dir = TempDir::new().expect("home dir should be created");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let uv_log = fake_bin_dir.path().join("uv.log");

    write_fake_uv(fake_bin_dir.path().join("uv"), &uv_log);
    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let first = run_install(&home_dir, fake_bin_dir.path());
    assert!(
        first.status.success(),
        "first install should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );
    assert_eq!(
        uv_invocation_count(&uv_log),
        1,
        "uv should run once initially"
    );

    let second = run_install(&home_dir, fake_bin_dir.path());
    let stdout = String::from_utf8_lossy(&second.stdout);
    let stderr = String::from_utf8_lossy(&second.stderr);

    assert!(
        second.status.success(),
        "repeat install should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("already available"),
        "repeat install should report the ready state: stdout={stdout}",
    );
    assert_eq!(
        uv_invocation_count(&uv_log),
        1,
        "repeat install should skip uv when the managed tooling is already ready",
    );
}

#[test]
fn install_openhands_repairs_missing_files_in_the_managed_tool_dir() {
    let home_dir = TempDir::new().expect("home dir should be created");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let uv_log = fake_bin_dir.path().join("uv.log");
    let tool_dir = home_dir.path().join(".opensymphony/openhands-server");

    write_fake_uv(fake_bin_dir.path().join("uv"), &uv_log);
    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let first = run_install(&home_dir, fake_bin_dir.path());
    assert!(
        first.status.success(),
        "first install should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );
    assert_eq!(
        uv_invocation_count(&uv_log),
        1,
        "uv should run once initially"
    );

    std::fs::remove_file(tool_dir.join("run-local.sh"))
        .expect("simulated corruption should remove run-local.sh");

    let repaired = run_install(&home_dir, fake_bin_dir.path());
    let stdout = String::from_utf8_lossy(&repaired.stdout);
    let stderr = String::from_utf8_lossy(&repaired.stderr);

    assert!(
        repaired.status.success(),
        "repair install should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("repaired pinned OpenHands tooling 1.24.0"),
        "repair install should report the repaired state: stdout={stdout}",
    );
    assert!(
        tool_dir.join("run-local.sh").is_file(),
        "repair should restore run-local.sh"
    );
    assert_eq!(
        uv_invocation_count(&uv_log),
        2,
        "repair should rerun uv after fixing the managed-local bundle",
    );
}

#[test]
fn install_openhands_updates_a_stale_but_self_consistent_pin() {
    let home_dir = TempDir::new().expect("home dir should be created");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");
    let uv_log = fake_bin_dir.path().join("uv.log");
    let tool_dir = home_dir.path().join(".opensymphony/openhands-server");

    write_fake_uv(fake_bin_dir.path().join("uv"), &uv_log);
    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let first = run_install(&home_dir, fake_bin_dir.path());
    assert!(
        first.status.success(),
        "first install should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr),
    );

    rewrite_openhands_pin(&tool_dir, "1.24.0", "1.23.0");
    let stale_tooling =
        LocalServerTooling::load(&tool_dir).expect("stale tooling should still parse");
    assert_eq!(stale_tooling.version, "1.23.0");
    assert!(
        stale_tooling.pin_status.is_ready(),
        "stale tooling fixture should remain internally self-consistent",
    );

    let updated = run_install(&home_dir, fake_bin_dir.path());
    let stdout = String::from_utf8_lossy(&updated.stdout);
    let stderr = String::from_utf8_lossy(&updated.stderr);

    assert!(
        updated.status.success(),
        "stale install should update cleanly: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("updated pinned OpenHands tooling 1.24.0"),
        "stale install should report an update, not a ready no-op: stdout={stdout}",
    );
    assert_eq!(
        uv_invocation_count(&uv_log),
        2,
        "updating a stale embedded pin should rerun uv",
    );
    let refreshed = LocalServerTooling::load(&tool_dir).expect("updated tooling should parse");
    assert_eq!(refreshed.version, "1.24.0");
    assert!(refreshed.pin_status.is_ready());
}

#[test]
fn install_openhands_surfaces_a_missing_uv_requirement() {
    let home_dir = TempDir::new().expect("home dir should be created");
    let fake_bin_dir = TempDir::new().expect("fake bin dir should be created");

    write_bash_wrapper(fake_bin_dir.path().join("bash"));

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["install", "openhands"])
        .env("HOME", home_dir.path())
        .env_remove("USERPROFILE")
        .env("PATH", path_only(fake_bin_dir.path()))
        .output()
        .expect("install command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "install should fail when uv is unavailable: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stderr.contains("uv is required to install the pinned OpenHands agent-server environment"),
        "install should surface the missing uv requirement: stderr={stderr}",
    );
}

fn run_install(home_dir: &TempDir, fake_bin_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["install", "openhands"])
        .env("HOME", home_dir.path())
        .env_remove("USERPROFILE")
        .env("PATH", path_only(fake_bin_dir))
        .output()
        .expect("install command should run")
}

fn assert_embedded_tooling_files(tool_dir: &Path) {
    for relative in [
        "README.md",
        "install.sh",
        "run-local.sh",
        "pyproject.toml",
        "uv.lock",
        "version.txt",
    ] {
        assert!(
            tool_dir.join(relative).is_file(),
            "embedded tooling file {relative} should exist under {}",
            tool_dir.display(),
        );
    }
}

fn uv_invocation_count(log_path: &Path) -> usize {
    std::fs::read_to_string(log_path)
        .expect("uv log should exist")
        .lines()
        .filter(|line| line.starts_with("ARGS="))
        .count()
}

fn rewrite_openhands_pin(tool_dir: &Path, from: &str, to: &str) {
    for relative in ["version.txt", "pyproject.toml", "uv.lock"] {
        let path = tool_dir.join(relative);
        let contents = std::fs::read_to_string(&path).expect("tooling file should exist");
        let updated = contents.replace(from, to);
        assert_ne!(
            contents,
            updated,
            "stale pin fixture should rewrite at least one occurrence in {}",
            path.display(),
        );
        std::fs::write(&path, updated).expect("tooling file should rewrite");
    }
}

fn path_only(path: &Path) -> OsString {
    std::env::join_paths([path]).expect("path should join")
}

fn write_fake_uv(path: PathBuf, log_path: &Path) {
    write_executable(
        path,
        &format!(
            "#!/bin/sh\nset -eu\nprintf 'PWD=%s\\n' \"$PWD\" >> \"{}\"\nprintf 'ARGS=%s\\n' \"$*\" >> \"{}\"\n",
            log_path.display(),
            log_path.display(),
        ),
    );
}

fn write_bash_wrapper(path: PathBuf) {
    let bash = real_bash_path();
    write_executable(
        path,
        &format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", bash.display()),
    );
}

fn real_bash_path() -> PathBuf {
    let output = Command::new("bash")
        .args(["-lc", "command -v bash"])
        .output()
        .expect("bash lookup should run");
    assert!(
        output.status.success(),
        "bash lookup should succeed: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("bash lookup output should be UTF-8")
            .trim(),
    )
}

fn write_executable(path: PathBuf, contents: &str) {
    std::fs::write(&path, contents).expect("executable should be written");
    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&path)
            .expect("executable metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).expect("executable should be executable");
    }
}
