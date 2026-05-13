use std::{fs, process::Command};

use tempfile::TempDir;

#[test]
fn memory_capture_write_query_and_sync_docs_are_reviewable() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_fixture(repo.path());

    let dry_run = run(
        repo.path(),
        [
            "memory",
            "capture",
            "COE-123",
            "--source-file",
            "source.yaml",
            "--dry-run",
        ],
    );
    assert_success(&dry_run, "capture dry-run");
    let stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(stdout.contains("Memory Capture Dry Run"));
    assert!(stdout.contains("GitHub PRs: #456"));
    assert!(stdout.contains("docs/openhands-runtime.md"));

    let write = run(
        repo.path(),
        [
            "memory",
            "capture",
            "COE-123",
            "--source-file",
            "source.yaml",
            "--write",
        ],
    );
    assert_success(&write, "capture write");
    assert!(
        repo.path()
            .join(".opensymphony/memory/issues/COE-123.md")
            .is_file()
    );
    assert!(
        repo.path()
            .join(".opensymphony/memory/memory.duckdb")
            .is_file()
    );

    let brief = run(repo.path(), ["memory", "brief", "COE-123"]);
    assert_success(&brief, "brief");
    let stdout = String::from_utf8_lossy(&brief.stdout);
    assert!(stdout.contains("WebSocket reconnect recovery"));
    assert!(stdout.contains("Validation evidence"));

    let search = run(repo.path(), ["memory", "search", "reconnect"]);
    assert_success(&search, "search");
    assert!(String::from_utf8_lossy(&search.stdout).contains("COE-123"));

    let docs = run(
        repo.path(),
        ["memory", "sync-docs", "--issues", "COE-123", "--dry-run"],
    );
    assert_success(&docs, "docs dry-run");
    let stdout = String::from_utf8_lossy(&docs.stdout);
    assert!(stdout.contains("Docs Sync Plan"));
    assert!(stdout.contains("COE-123"));
    assert!(!stdout.contains(".opensymphony/memory/issues"));

    let archive = run(
        repo.path(),
        ["linear", "archive", "--issues", "COE-123", "--dry-run"],
    );
    assert_success(&archive, "archive dry-run");
    let stdout = String::from_utf8_lossy(&archive.stdout);
    assert!(stdout.contains("eligible"));
}

#[test]
fn memory_capture_reports_missing_source_file() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());

    let output = run(
        repo.path(),
        [
            "memory",
            "capture",
            "COE-123",
            "--source-file",
            "missing.yaml",
            "--dry-run",
        ],
    );

    assert_failure(&output, "missing source file");
    assert!(String::from_utf8_lossy(&output.stderr).contains("failed to read"));
}

#[test]
fn memory_capture_force_overwrites_non_generated_capsule() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_fixture(repo.path());
    let issue_dir = repo.path().join(".opensymphony/memory/issues");
    fs::create_dir_all(&issue_dir).expect("issue dir should write");
    fs::write(issue_dir.join("COE-123.md"), "operator note").expect("capsule should write");

    let blocked = run(
        repo.path(),
        [
            "memory",
            "capture",
            "COE-123",
            "--source-file",
            "source.yaml",
            "--write",
        ],
    );
    assert_failure(&blocked, "capture without force");
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("does not look generated"));

    let forced = run(
        repo.path(),
        [
            "memory",
            "capture",
            "COE-123",
            "--source-file",
            "source.yaml",
            "--write",
            "--force",
        ],
    );
    assert_success(&forced, "capture with force");
    let capsule =
        fs::read_to_string(issue_dir.join("COE-123.md")).expect("capsule should be readable");
    assert!(capsule.contains("BEGIN OPENSYMPHONY MANAGED ISSUE CAPSULE"));
}

#[test]
fn memory_lint_related_paths_and_from_memory_archive_cover_private_doc_links() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_fixture(repo.path());
    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "capture",
                "COE-123",
                "--source-file",
                "source.yaml",
                "--write",
            ],
        ),
        "capture write",
    );

    fs::write(
        repo.path().join("docs/openhands-runtime.md"),
        "See .opensymphony/memory/issues/COE-123.md for private details.",
    )
    .expect("docs target should write");

    let lint = run(repo.path(), ["memory", "lint", "--public-docs"]);
    assert_success(&lint, "lint private links");
    assert!(
        String::from_utf8_lossy(&lint.stdout).contains("public docs contain a private memory path")
    );

    let related = run(
        repo.path(),
        [
            "memory",
            "related",
            "--paths",
            "crates/opensymphony-openhands/src/client.rs",
        ],
    );
    assert_success(&related, "related by paths");
    assert!(String::from_utf8_lossy(&related.stdout).contains("COE-123"));

    let archive = run(
        repo.path(),
        [
            "linear",
            "archive",
            "--from-memory",
            "--state",
            "captured",
            "--dry-run",
        ],
    );
    assert_success(&archive, "archive from memory captured");
    let stdout = String::from_utf8_lossy(&archive.stdout);
    assert!(stdout.contains("COE-123"));
    assert!(stdout.contains("eligible"));
}

#[test]
fn memory_capture_discover_github_reports_missing_gh() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_fixture(repo.path());

    let output = run_with_path(
        repo.path(),
        [
            "memory",
            "capture",
            "COE-123",
            "--source-file",
            "source.yaml",
            "--discover-github",
            "--dry-run",
        ],
        "",
    );

    assert_success(&output, "discover github without gh");
    assert!(String::from_utf8_lossy(&output.stdout).contains("gh CLI was not found"));
}

fn run<const N: usize>(repo: &std::path::Path, args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("command should run")
}

fn run_with_path<const N: usize>(
    repo: &std::path::Path,
    args: [&str; N],
    path: &str,
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(args)
        .current_dir(repo)
        .env("PATH", path)
        .output()
        .expect("command should run")
}

fn assert_success(output: &std::process::Output, label: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{label} should succeed: stdout={stdout}, stderr={stderr}",
    );
}

fn assert_failure(output: &std::process::Output, label: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "{label} should fail: stdout={stdout}, stderr={stderr}",
    );
}

fn write_memory_fixture(repo: &std::path::Path) {
    write_memory_config(repo);
    fs::write(repo.join("source.yaml"), sample_source()).expect("source evidence should write");
}

fn write_memory_config(repo: &std::path::Path) {
    fs::create_dir_all(repo.join("docs")).expect("docs dir should write");
    fs::write(
        repo.join("opensymphony-memory.yaml"),
        r#"
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-runtime.md
    path_hints:
      - openhands
    labels:
      - runtime
"#,
    )
    .expect("memory config should write");
}

fn sample_source() -> &'static str {
    r#"
issues:
  - identifier: COE-123
    title: WebSocket reconnect recovery
    url: https://linear.app/example/issue/COE-123
    description: Recover OpenHands runtime streams after reconnect.
    state: Done
    milestone: M3
    labels:
      - runtime
    linked_prs:
      - 456
    comments:
      - body: "Decision: reconcile REST event backlog after readiness."
prs:
  - number: 456
    title: COE-123 recover websocket reconnects
    url: https://github.com/example/repo/pull/456
    branch: coe-123-reconnect
    merge_sha: abcdef1234567890
    changed_files:
      - path: crates/opensymphony-openhands/src/client.rs
        change_kind: modified
    checks:
      - name: cargo test
        conclusion: success
    reviews:
      - reviewer: reviewer
        state: APPROVED
        disposition: Reconnect ordering looked correct.
"#
}
