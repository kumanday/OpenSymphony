use std::{fs, process::Command};

use tempfile::TempDir;

#[test]
fn memory_capture_write_query_and_sync_docs_are_reviewable() {
    let repo = TempDir::new().expect("temp repo should exist");
    fs::create_dir_all(repo.path().join("docs")).expect("docs dir should write");
    fs::write(
        repo.path().join("opensymphony-memory.yaml"),
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
    fs::write(repo.path().join("source.yaml"), sample_source())
        .expect("source evidence should write");

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

fn run<const N: usize>(repo: &std::path::Path, args: [&str; N]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(args)
        .current_dir(repo)
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
