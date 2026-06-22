use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

use duckdb::Connection;
use tempfile::TempDir;

#[test]
fn memory_capture_write_query_and_sync_docs_are_reviewable() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_fixture(repo.path());

    let dry_run = run(
        repo.path(),
        [
            "memory",
            "import",
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
            "import",
            "COE-123",
            "--source-file",
            "source.yaml",
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
    assert!(stdout.contains("Docs Sync Summary"));
    assert!(stdout.contains("COE-123"));
    assert!(!stdout.contains(".opensymphony/memory/issues"));

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
    assert_success(&archive, "archive dry-run");
    let stdout = String::from_utf8_lossy(&archive.stdout);
    assert!(stdout.contains("eligible"));
}

#[test]
fn memory_init_creates_private_config_and_gitignore_policy() {
    let repo = TempDir::new().expect("temp repo should exist");
    fs::create_dir_all(repo.path().join("docs/tasks")).expect("tasks dir should write");
    fs::write(
        repo.path().join("docs/tasks/task.md"),
        "---\narea: agent-runtime\n---\n# Task\n",
    )
    .expect("task should write");
    fs::write(
        repo.path().join("docs/agent-runtime.md"),
        "# Agent Runtime\n",
    )
    .expect("doc should write");
    fs::write(repo.path().join(".gitignore"), ".opensymphony*\n").expect("gitignore should write");

    let output = run(repo.path(), ["memory", "init"]);
    assert_success(&output, "memory init");

    let config_path = repo.path().join(".opensymphony/memory/memory.yaml");
    let config = fs::read_to_string(&config_path).expect("memory config should exist");
    assert!(config.contains("agent-runtime:"));
    assert!(config.contains("docs_target: docs/agent-runtime.md"));
    assert!(config.contains("status: stable"));
    assert!(config.contains("confidence: 85"));
    assert!(
        !repo
            .path()
            .join(".opensymphony/memory/memory.duckdb")
            .exists()
    );

    let gitignore =
        fs::read_to_string(repo.path().join(".gitignore")).expect("gitignore should be readable");
    assert!(gitignore.contains("!.opensymphony/memory/memory.yaml"));
    assert!(gitignore.contains(".opensymphony/memory/*"));

    let duplicate = run(repo.path(), ["memory", "init"]);
    assert_failure(&duplicate, "memory init duplicate");
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already exists"));

    let forced = run(repo.path(), ["memory", "init", "--force"]);
    assert_success(&forced, "memory init force");
}

#[test]
fn memory_init_dry_run_does_not_write_files() {
    let repo = TempDir::new().expect("temp repo should exist");

    let output = run(repo.path(), ["memory", "init", "--dry-run"]);
    assert_success(&output, "memory init dry-run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Proposed config"));
    assert!(stdout.contains("areas:"));
    assert!(!stdout.contains("general:"));
    assert!(
        !repo
            .path()
            .join(".opensymphony/memory/memory.yaml")
            .exists()
    );
    assert!(!repo.path().join(".gitignore").exists());
}

#[test]
fn memory_context_can_include_code_intelligence_without_a_separate_cli_command() {
    let repo = TempDir::new().expect("temp repo should exist");
    fs::create_dir_all(repo.path().join("src")).expect("src dir should write");
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn answer() -> u8 { 42 }\n",
    )
    .expect("source file should write");

    let output = run(
        repo.path(),
        [
            "memory",
            "context",
            "--issue",
            "COE-999",
            "--paths",
            "src/lib.rs",
            "--include-code-intel",
        ],
    );

    assert_success(&output, "context with code intelligence");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Memory Context: COE-999"));
    assert!(stdout.contains("## Code Intelligence"));
    assert!(stdout.contains("Repository summary"));

    let help = run(repo.path(), ["memory", "--help"]);
    assert_success(&help, "memory help");
    assert!(
        !String::from_utf8_lossy(&help.stdout).contains("code-context"),
        "memory help should keep context as the agent-facing command"
    );
}

#[test]
fn memory_read_commands_use_mcp_endpoint_when_configured() {
    let repo = TempDir::new().expect("temp repo should exist");
    let server = TinyGraphqlServer::start([
        r#"{"jsonrpc":"2.0","id":"opensymphony-cli","result":{"content":[{"type":"text","text":"remote brief"}]}}"#,
    ]);

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["memory", "brief", "COE-123"])
        .current_dir(repo.path())
        .env("OPENSYMPHONY_MEMORY_ENDPOINT", &server.base_url)
        .output()
        .expect("command should run");

    assert_success(&output, "remote memory brief");
    assert!(String::from_utf8_lossy(&output.stdout).contains("remote brief"));
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("\"method\":\"tools/call\""));
    assert!(requests[0].contains("\"name\":\"memory.brief\""));
    assert!(requests[0].contains("\"issue\":\"COE-123\""));
}

#[test]
fn memory_read_commands_forward_worker_scope_to_mcp_endpoint() {
    let repo = TempDir::new().expect("temp repo should exist");
    let server = TinyGraphqlServer::start([
        r#"{"jsonrpc":"2.0","id":"opensymphony-cli","result":{"results":[]}}"#,
    ]);

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["memory", "search", "shared"])
        .current_dir(repo.path())
        .env("OPENSYMPHONY_MEMORY_ENDPOINT", &server.base_url)
        .env("OPENSYMPHONY_MEMORY_PROJECT", "project-alpha")
        .env("OPENSYMPHONY_MEMORY_EXECUTION_REPO", "services/api")
        .output()
        .expect("command should run");

    assert_success(&output, "remote memory search");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("\"name\":\"memory.search\""));
    assert!(requests[0].contains("\"query\":\"shared\""));
    assert!(requests[0].contains("\"project\":\"project-alpha\""));
    assert!(requests[0].contains("\"repo\":\"services/api\""));
}

#[test]
fn memory_admin_commands_can_use_mcp_endpoint() {
    let repo = TempDir::new().expect("temp repo should exist");
    let server = TinyGraphqlServer::start([
        r#"{"jsonrpc":"2.0","id":"opensymphony-cli","result":{"findingCount":0,"findings":[]}}"#,
        r#"{"jsonrpc":"2.0","id":"opensymphony-cli","result":{"findingCount":0,"findings":[]}}"#,
    ]);

    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["memory", "lint", "--public-docs"])
        .current_dir(repo.path())
        .env("OPENSYMPHONY_MEMORY_ENDPOINT", &server.base_url)
        .env("OPENSYMPHONY_MEMORY_ADMIN_TOKEN", "admin-token")
        .output()
        .expect("command should run");

    assert_success(&output, "remote memory lint");
    let okf_output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["memory", "lint", "--okf", "fixtures/okf-migration"])
        .current_dir(repo.path())
        .env("OPENSYMPHONY_MEMORY_ENDPOINT", &server.base_url)
        .env("OPENSYMPHONY_MEMORY_ADMIN_TOKEN", "admin-token")
        .output()
        .expect("command should run");

    assert_success(&okf_output, "remote okf memory lint");
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("\"name\":\"memory.lint\""));
    assert!(requests[0].contains("\"publicDocs\":true"));
    assert!(requests[1].contains("\"name\":\"memory.lint\""));
    assert!(requests[1].contains("\"okf\":true"));
    assert!(requests[1].contains("\"bundleRoot\":\"fixtures/okf-migration\""));
}

#[test]
fn memory_lint_okf_reports_fixture_diagnostics() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let fixture = repo.path().join("fixtures/okf-migration");
    copy_dir_recursive(&okf_fixture("okf-migration"), &fixture);
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["memory", "lint", "--okf", "fixtures/okf-migration"])
        .current_dir(repo.path())
        .output()
        .expect("command should run");

    assert_success(&output, "okf lint fixture");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[error]"));
    assert!(stdout.contains("frontmatter lacks non-empty `type`"));
    assert!(stdout.contains("reserved log.md must use ISO date headings"));
    assert!(stdout.contains("private export leak"));
    assert!(stdout.contains("[warn]"));
    assert!(stdout.contains("missing recommended field(s)"));
    assert!(stdout.contains("broken Markdown link"));
    assert!(stdout.contains("wiki-only link"));
    assert!(stdout.contains("missing generated index.md"));
    assert!(stdout.contains("citation section missing"));
    assert!(stdout.contains("[info]"));
    assert!(stdout.contains("legacy field(s) retained"));
}

#[test]
fn memory_search_defaults_cross_repo_and_repo_filters_by_changed_paths() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    fs::write(repo.path().join("source.yaml"), multi_repo_source())
        .expect("source evidence should write");

    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "import",
                "--issues",
                "COE-201,COE-202",
                "--source-file",
                "source.yaml",
            ],
        ),
        "capture multi-repo fixture",
    );

    let cross_repo = run(repo.path(), ["memory", "search", "shared"]);
    assert_success(&cross_repo, "cross-repo search");
    let stdout = String::from_utf8_lossy(&cross_repo.stdout);
    assert!(stdout.contains("COE-201"));
    assert!(stdout.contains("COE-202"));

    let api_only = run(
        repo.path(),
        ["memory", "search", "--repo", "services/api", "shared"],
    );
    assert_success(&api_only, "repo-scoped search");
    let stdout = String::from_utf8_lossy(&api_only.stdout);
    assert!(stdout.contains("COE-201"));
    assert!(!stdout.contains("COE-202"));
}

#[test]
fn memory_docs_applies_repo_scope_before_returning_area_doc() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    fs::write(repo.path().join("source.yaml"), multi_repo_source())
        .expect("source evidence should write");
    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "import",
                "--issues",
                "COE-201,COE-202",
                "--source-file",
                "source.yaml",
            ],
        ),
        "capture multi-repo fixture",
    );
    assert_success(
        &run(
            repo.path(),
            ["memory", "sync-docs", "--area", "openhands-runtime"],
        ),
        "sync scoped docs",
    );

    let api_docs = run(
        repo.path(),
        [
            "memory",
            "docs",
            "--area",
            "openhands-runtime",
            "--repo",
            "services/api",
        ],
    );
    assert_success(&api_docs, "docs with matching repo scope");
    assert!(String::from_utf8_lossy(&api_docs.stdout).contains("# OpenHands Runtime"));

    let mobile_docs = run(
        repo.path(),
        [
            "memory",
            "docs",
            "--area",
            "openhands-runtime",
            "--repo",
            "services/mobile",
        ],
    );
    assert_failure(&mobile_docs, "docs with non-matching repo scope");
    assert!(
        String::from_utf8_lossy(&mobile_docs.stderr)
            .contains("no captured memory for area `openhands-runtime`")
    );
}

#[test]
fn sync_docs_requires_configured_area_mapping() {
    let repo = TempDir::new().expect("temp repo should exist");
    fs::write(repo.path().join("source.yaml"), candidate_only_source())
        .expect("source evidence should write");
    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "import",
                "COE-123",
                "--source-file",
                "source.yaml",
            ],
        ),
        "capture without config",
    );

    let output = run(repo.path(), ["memory", "sync-docs"]);
    assert_success(&output, "sync-docs without stable mappings");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("candidate or unmapped docs areas"));
    assert!(stdout.contains("confidence improves"));
    assert!(!repo.path().join("docs/readme-md.md").exists());
}

#[test]
fn sync_docs_writes_only_configured_areas_and_dry_run_writes_nothing() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_general_memory_config(repo.path());
    fs::write(repo.path().join("source.yaml"), noisy_file_source())
        .expect("source evidence should write");
    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "import",
                "COE-123",
                "--source-file",
                "source.yaml",
            ],
        ),
        "capture noisy file source",
    );

    let dry_run = run(repo.path(), ["memory", "sync-docs", "--dry-run"]);
    assert_success(&dry_run, "sync-docs dry-run");
    assert!(
        !repo.path().join("docs/general.md").exists(),
        "dry run should not write docs"
    );

    let write = run(repo.path(), ["memory", "sync-docs"]);
    assert_success(&write, "sync-docs write");
    assert!(repo.path().join("docs/general.md").is_file());
    for bad_path in ["docs/readme-md.md", "docs/cargo.md", "docs/cargo-lock.md"] {
        assert!(
            !repo.path().join(bad_path).exists(),
            "{bad_path} should not be generated from changed filenames"
        );
    }
}

#[test]
fn memory_import_reports_missing_source_file() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());

    let output = run(
        repo.path(),
        [
            "memory",
            "import",
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
fn memory_import_force_overwrites_non_generated_capsule() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_fixture(repo.path());
    let issue_dir = repo.path().join(".opensymphony/memory/issues");
    fs::create_dir_all(&issue_dir).expect("issue dir should write");
    fs::write(issue_dir.join("COE-123.md"), "operator note").expect("capsule should write");

    let blocked = run(
        repo.path(),
        [
            "memory",
            "import",
            "COE-123",
            "--source-file",
            "source.yaml",
        ],
    );
    assert_failure(&blocked, "capture without force");
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("does not look generated"));

    let forced = run(
        repo.path(),
        [
            "memory",
            "import",
            "COE-123",
            "--source-file",
            "source.yaml",
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
                "import",
                "COE-123",
                "--source-file",
                "source.yaml",
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
fn memory_import_generates_date_grouped_log_newest_first() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    fs::write(repo.path().join("source.yaml"), sample_two_issue_source())
        .expect("source evidence should write");

    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "import",
                "--issues",
                "COE-123,COE-124",
                "--source-file",
                "source.yaml",
            ],
        ),
        "capture two dated issues",
    );

    let log = fs::read_to_string(repo.path().join(".opensymphony/memory/indexes/log.md"))
        .expect("log should be readable");
    assert!(log.contains("## 2026-06-14"));
    assert!(log.contains("## 2026-06-13"));
    assert!(
        log.find("## 2026-06-14").expect("newer heading")
            < log.find("## 2026-06-13").expect("older heading")
    );
}

#[test]
fn memory_capture_discovers_github_by_default_and_reports_missing_gh() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let output = run_with_path(
        repo.path(),
        ["memory", "capture", "COE-123", "--dry-run"],
        "",
    );

    assert_failure(&output, "discover github without gh");
    assert!(String::from_utf8_lossy(&output.stderr).contains("gh CLI was not found"));
}

#[test]
fn memory_capture_can_skip_github_discovery() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let output = run_with_path(
        repo.path(),
        ["memory", "capture", "COE-123", "--no-github", "--dry-run"],
        "",
    );

    assert_success(&output, "capture without github discovery");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GitHub PRs: none"));
    assert!(stdout.contains("no GitHub PR source was matched"));
}

#[test]
fn memory_capture_discovers_matching_github_prs_by_default() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let bin_dir = repo.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir should write");
    let gh_log = repo.path().join("gh.log");
    write_fake_gh_discovery(bin_dir.join("gh"), &gh_log);

    let output = run_with_path(
        repo.path(),
        ["memory", "capture", "COE-123", "--dry-run"],
        bin_dir.to_str().expect("bin path should be utf-8"),
    );

    assert_success(&output, "capture with github discovery");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GitHub PRs: #456"));
    assert!(
        fs::read_to_string(gh_log)
            .expect("gh log should be readable")
            .contains("pr list")
    );
}

#[test]
fn memory_capture_reports_github_enrichment_warnings() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let bin_dir = repo.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir should write");
    write_fake_gh_enrichment_failure(bin_dir.join("gh"));

    let output = run_with_path(
        repo.path(),
        ["memory", "capture", "COE-123", "--dry-run"],
        bin_dir.to_str().expect("bin path should be utf-8"),
    );

    assert_success(&output, "capture with partial github evidence");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("GitHub PRs: #456"));
    assert!(stdout.contains("GitHub PR enrichment for PR #456 failed"));
}

#[test]
fn memory_capture_expands_linear_children_and_links_graph() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_with_children_response("COE-123", "Parent capability", ["COE-124"]),
        linear_child_issue_response("COE-124", "Child implementation", "COE-123"),
        linear_empty_comments("issue-COE-123"),
        linear_empty_comments("issue-COE-124"),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let output = run(repo.path(), ["memory", "capture", "COE-123", "--no-github"]);

    assert_success(&output, "capture parent and child");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Wrote 2 capsule(s)."));
    let parent = fs::read_to_string(repo.path().join(".opensymphony/memory/issues/COE-123.md"))
        .expect("parent capsule should be readable");
    let child = fs::read_to_string(repo.path().join(".opensymphony/memory/issues/COE-124.md"))
        .expect("child capsule should be readable");
    assert!(parent.contains("[[COE-124|COE-124: Child implementation]]"));
    assert!(child.contains("[[COE-123|COE-123: Parent capability]]"));
    assert!(
        parent.contains(
            "[[milestones/m3-symphony-orchestration-core|M3: Symphony orchestration core]]"
        )
    );
    assert!(
        repo.path()
            .join(".opensymphony/memory/milestones/m3-symphony-orchestration-core.md")
            .is_file()
    );
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn linear_archive_live_capture_archives_expanded_children() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_with_children_response("COE-123", "Parent capability", ["COE-124"]),
        linear_child_issue_response("COE-124", "Child implementation", "COE-123"),
        linear_empty_comments("issue-COE-123"),
        linear_empty_comments("issue-COE-124"),
        r#"{"data":{"issueArchive":{"success":true}}}"#.to_string(),
        r#"{"data":{"issueArchive":{"success":true}}}"#.to_string(),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let archive = run(
        repo.path(),
        [
            "linear",
            "archive",
            "--issues",
            "COE-123",
            "--no-github",
            "--force",
        ],
    );

    assert_success(&archive, "archive parent and expanded child");
    let stdout = String::from_utf8_lossy(&archive.stdout);
    assert!(stdout.contains("Wrote 2 capsule(s)."));
    assert!(stdout.contains("Archived 2 Linear issue(s)."));
    assert_eq!(archive_status(repo.path(), "COE-123"), "archived");
    assert_eq!(archive_status(repo.path(), "COE-124"), "archived");
    let requests = server.requests();
    assert_eq!(requests.len(), 6);
    assert!(requests[4].contains("\"id\":\"COE-124\""));
    assert!(requests[5].contains("\"id\":\"COE-123\""));
}

#[test]
fn linear_archive_with_explicit_issues_captures_before_archiving() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
        r#"{"data":{"issueArchive":{"success":true}}}"#.to_string(),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let bin_dir = repo.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir should write");
    let gh_log = repo.path().join("gh.log");
    write_fake_gh_discovery(bin_dir.join("gh"), &gh_log);

    let archive = run_with_path(
        repo.path(),
        ["linear", "archive", "--issues", "COE-123"],
        bin_dir.to_str().expect("bin path should be utf-8"),
    );

    assert_success(&archive, "archive with live capture");
    assert!(String::from_utf8_lossy(&archive.stdout).contains("Wrote 1 capsule(s)."));
    assert_eq!(archive_status(repo.path(), "COE-123"), "archived");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].contains("\"id\":\"COE-123\""));
}

#[test]
fn linear_archive_does_not_block_when_no_github_pr_matches() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
        r#"{"data":{"issueArchive":{"success":true}}}"#.to_string(),
    ]);
    write_workflow(repo.path(), &server.base_url);

    let bin_dir = repo.path().join("bin");
    fs::create_dir_all(&bin_dir).expect("bin dir should write");
    write_fake_gh_no_matches(bin_dir.join("gh"));

    let archive = run_with_path(
        repo.path(),
        ["linear", "archive", "--issues", "COE-123"],
        bin_dir.to_str().expect("bin path should be utf-8"),
    );

    assert_success(&archive, "archive without matched GitHub PR");
    let stdout = String::from_utf8_lossy(&archive.stdout);
    assert!(stdout.contains("no GitHub PR source was matched"));
    assert!(!stdout.contains("Linear Archive Dry Run"));
    assert!(stdout.contains("Archived 1 Linear issue(s)."));
    assert_eq!(archive_status(repo.path(), "COE-123"), "archived");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].contains("\"id\":\"COE-123\""));
}

#[test]
fn archive_from_memory_does_not_block_when_only_warning_is_no_github_pr() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    let server = TinyGraphqlServer::start([
        linear_issue_response("COE-123", "WebSocket reconnect recovery"),
        linear_empty_comments("issue-COE-123"),
        r#"{"data":{"issueArchive":{"success":true}}}"#.to_string(),
    ]);
    write_workflow(repo.path(), &server.base_url);

    assert_success(
        &run(repo.path(), ["memory", "capture", "COE-123", "--no-github"]),
        "capture without matched GitHub PR",
    );
    let archive = run(repo.path(), ["linear", "archive", "--from-memory"]);

    assert_success(&archive, "archive from memory without matched GitHub PR");
    assert!(String::from_utf8_lossy(&archive.stdout).contains("Archived 1 Linear issue(s)."));
    assert_eq!(archive_status(repo.path(), "COE-123"), "archived");
}

#[test]
fn linear_archive_write_marks_successes_before_reporting_partial_failure() {
    let repo = TempDir::new().expect("temp repo should exist");
    write_memory_config(repo.path());
    fs::write(repo.path().join("source.yaml"), sample_two_issue_source())
        .expect("source evidence should write");
    assert_success(
        &run(
            repo.path(),
            [
                "memory",
                "import",
                "--issues",
                "COE-123,COE-124",
                "--source-file",
                "source.yaml",
            ],
        ),
        "capture two issues",
    );

    let server = TinyGraphqlServer::start([
        r#"{"data":{"issueArchive":{"success":true}}}"#,
        r#"{"data":{"issueArchive":{"success":false}}}"#,
    ]);
    write_workflow(repo.path(), &server.base_url);

    let archive = run(
        repo.path(),
        ["linear", "archive", "--from-memory", "--state", "captured"],
    );

    assert_failure(&archive, "partial archive failure");
    assert!(String::from_utf8_lossy(&archive.stdout).contains("Archived 1 Linear issue(s)."));
    assert!(String::from_utf8_lossy(&archive.stderr).contains("failed to archive COE-124"));
    assert_eq!(
        archive_status(repo.path(), "COE-123"),
        "archived",
        "successful mutation should be recorded locally",
    );
    assert_eq!(
        archive_status(repo.path(), "COE-124"),
        "not_archived",
        "failed mutation should remain eligible for retry",
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("\"id\":\"COE-123\""));
    assert!(requests[1].contains("\"id\":\"COE-124\""));
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

fn okf_fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/opensymphony-memory/tests/fixtures")
        .join(name)
}

fn copy_dir_recursive(source: &std::path::Path, destination: &std::path::Path) {
    fs::create_dir_all(destination).expect("destination directory should be created");
    for entry in fs::read_dir(source).expect("source directory should be readable") {
        let entry = entry.expect("source entry should be readable");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("source entry type should be readable")
            .is_dir()
        {
            copy_dir_recursive(&source_path, &destination_path);
        } else {
            fs::copy(&source_path, &destination_path).expect("fixture file should copy");
        }
    }
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
    status: stable
    confidence: 90
    aliases:
      - runtime
      - OpenHands Runtime
    source_refs:
      linear_labels:
        - runtime
"#,
    )
    .expect("memory config should write");
}

fn write_general_memory_config(repo: &std::path::Path) {
    fs::create_dir_all(repo.join(".opensymphony/memory")).expect("memory dir should write");
    fs::write(
        repo.join(".opensymphony/memory/memory.yaml"),
        r#"
areas:
  general:
    title: General
    docs_target: docs/general.md
    status: stable
    confidence: 90
    aliases:
      - general
    source_refs:
      linear_labels:
        - general
"#,
    )
    .expect("memory config should write");
}

fn write_workflow(repo: &std::path::Path, linear_endpoint: &str) {
    fs::write(
        repo.join("WORKFLOW.md"),
        format!(
            r#"---
tracker:
  kind: linear
  endpoint: {linear_endpoint}
  api_key: test-token
  project_slug: test-project
  active_states:
    - Todo
  terminal_states:
    - Done
---
{{{{ issue.identifier }}}}
"#
        ),
    )
    .expect("workflow should write");
}

fn linear_issue_response(identifier: &str, title: &str) -> String {
    let issue_id = format!("issue-{identifier}");
    format!(
        r#"{{
  "data": {{
    "issue": {{
      "id": "{issue_id}",
      "identifier": "{identifier}",
      "url": "https://linear.app/example/issue/{identifier}",
      "title": "{title}",
      "description": "Captured from Linear.",
      "priority": 0.0,
      "createdAt": "2026-03-20T10:00:00Z",
      "updatedAt": "2026-03-21T12:00:00Z",
      "state": {{
        "id": "state-done",
        "name": "Done",
        "type": "completed"
      }},
      "parent": null,
      "children": {{
        "nodes": []
      }},
      "labels": {{
        "nodes": [
          {{ "name": "runtime" }}
        ],
        "pageInfo": {{
          "hasNextPage": false,
          "endCursor": null
        }}
      }},
      "inverseRelations": {{
        "nodes": [],
        "pageInfo": {{
        "hasNextPage": false,
        "endCursor": null
        }}
      }}
    }}
  }}
}}"#
    )
}

fn linear_issue_with_children_response<const N: usize>(
    identifier: &str,
    title: &str,
    children: [&str; N],
) -> String {
    linear_issue_tree_response(identifier, title, None, &children)
}

fn linear_child_issue_response(identifier: &str, title: &str, parent: &str) -> String {
    linear_issue_tree_response(identifier, title, Some(parent), &[])
}

fn linear_issue_tree_response(
    identifier: &str,
    title: &str,
    parent: Option<&str>,
    children: &[&str],
) -> String {
    let issue_id = format!("issue-{identifier}");
    let parent_json = parent.map_or_else(
        || "null".to_string(),
        |parent| {
            format!(
                r#"{{
        "id": "issue-{parent}",
        "identifier": "{parent}",
        "url": "https://linear.app/example/issue/{parent}",
        "title": "Parent capability",
        "state": {{ "name": "Done" }}
      }}"#
            )
        },
    );
    let children_json = children
        .iter()
        .map(|child| {
            format!(
                r#"{{
          "id": "issue-{child}",
          "identifier": "{child}",
          "url": "https://linear.app/example/issue/{child}",
          "title": "Child implementation",
          "state": {{ "name": "Done" }}
        }}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{
  "data": {{
    "issue": {{
      "id": "{issue_id}",
      "identifier": "{identifier}",
      "url": "https://linear.app/example/issue/{identifier}",
      "title": "{title}",
      "description": "Captured from Linear.",
      "priority": 0.0,
      "createdAt": "2026-03-20T10:00:00Z",
      "updatedAt": "2026-03-21T12:00:00Z",
      "state": {{
        "id": "state-done",
        "name": "Done",
        "type": "completed"
      }},
      "parent": {parent_json},
      "projectMilestone": {{
        "id": "milestone-m3",
        "name": "M3: Symphony orchestration core"
      }},
      "children": {{
        "nodes": [{children_json}]
      }},
      "labels": {{
        "nodes": [
          {{ "name": "runtime" }}
        ],
        "pageInfo": {{
          "hasNextPage": false,
          "endCursor": null
        }}
      }},
      "inverseRelations": {{
        "nodes": [],
        "pageInfo": {{
        "hasNextPage": false,
        "endCursor": null
        }}
      }}
    }}
  }}
}}"#
    )
}

fn linear_empty_comments(issue_id: &str) -> String {
    format!(
        r#"{{
  "data": {{
    "issue": {{
      "id": "{issue_id}",
      "comments": {{
        "nodes": [],
        "pageInfo": {{
          "hasNextPage": false,
          "endCursor": null
        }}
      }}
    }}
  }}
}}"#
    )
}

fn write_fake_gh_discovery(path: std::path::PathBuf, log_path: &std::path::Path) {
    write_executable(
        path,
        &format!(
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "{}"
if [ "${{1-}}" = "pr" ] && [ "${{2-}}" = "list" ]; then
  printf '%s\n' '[{{"number":456,"title":"COE-123 recover websocket reconnects","url":"https://github.com/example/repo/pull/456","headRefName":"coe-123-reconnect","mergedAt":"2026-03-22T10:00:00Z","body":"Fixes COE-123","mergeCommit":{{"oid":"abcdef1234567890"}}}}]'
  exit 0
fi
if [ "${{1-}}" = "pr" ] && [ "${{2-}}" = "view" ]; then
  printf '%s\n' '{{"files":[{{"path":"crates/opensymphony-openhands/src/client.rs","changeType":"MODIFIED"}}],"commits":[{{"oid":"abcdef1234567890","messageHeadline":"COE-123 recover websocket reconnects"}}],"reviews":[{{"author":{{"login":"reviewer"}},"state":"APPROVED","submittedAt":"2026-03-22T11:00:00Z","body":"Looks correct."}}],"statusCheckRollup":[{{"name":"cargo test","conclusion":"SUCCESS","completedAt":"2026-03-22T11:30:00Z"}}],"mergeCommit":{{"oid":"abcdef1234567890"}}}}'
  exit 0
fi
printf 'unexpected gh command: %s\n' "$*" >&2
exit 1
"#,
            log_path.display(),
        ),
    );
}

fn write_fake_gh_enrichment_failure(path: std::path::PathBuf) {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
if [ "${1-}" = "pr" ] && [ "${2-}" = "list" ]; then
  printf '%s\n' '[{"number":456,"title":"COE-123 recover websocket reconnects","url":"https://github.com/example/repo/pull/456","headRefName":"coe-123-reconnect","mergedAt":"2026-03-22T10:00:00Z","body":"Fixes COE-123","mergeCommit":{"oid":"abcdef1234567890"}}]'
  exit 0
fi
if [ "${1-}" = "pr" ] && [ "${2-}" = "view" ]; then
  printf '%s\n' 'simulated gh view failure' >&2
  exit 2
fi
printf 'unexpected gh command: %s\n' "$*" >&2
exit 1
"#,
    );
}

fn write_fake_gh_no_matches(path: std::path::PathBuf) {
    write_executable(
        path,
        r#"#!/bin/sh
set -eu
if [ "${1-}" = "pr" ] && [ "${2-}" = "list" ]; then
  printf '%s\n' '[]'
  exit 0
fi
printf 'unexpected gh command: %s\n' "$*" >&2
exit 1
"#,
    );
}

fn write_executable(path: std::path::PathBuf, contents: &str) {
    fs::write(&path, contents).expect("executable should write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&path)
            .expect("executable metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("executable should be executable");
    }
}

fn archive_status(repo: &std::path::Path, issue_key: &str) -> String {
    let connection = Connection::open(repo.join(".opensymphony/memory/memory.duckdb"))
        .expect("memory index should open");
    connection
        .query_row(
            "SELECT archive_status FROM issues WHERE issue_key = ?",
            [issue_key],
            |row| row.get::<_, String>(0),
        )
        .expect("archive status should exist")
}

struct TinyGraphqlServer {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl TinyGraphqlServer {
    fn start<const N: usize, S>(responses: [S; N]) -> Self
    where
        S: Into<String>,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("server should bind");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded_requests = Arc::clone(&requests);
        let responses = responses.map(Into::into);
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("request should connect");
                let body = read_http_body(&mut stream);
                recorded_requests
                    .lock()
                    .expect("requests lock should be healthy")
                    .push(body);
                let http_response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    response.len(),
                    response
                );
                stream
                    .write_all(http_response.as_bytes())
                    .expect("response should write");
            }
        });

        Self { base_url, requests }
    }

    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("requests lock should be healthy")
            .clone()
    }
}

fn read_http_body(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("request should read");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let headers_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
        .expect("headers should end");
    let headers = String::from_utf8_lossy(&buffer[..headers_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().expect("content length"))
        })
        .unwrap_or(0);
    let already_read = buffer.len() - headers_end;
    let remaining = content_length.saturating_sub(already_read);
    if remaining > 0 {
        let mut body_tail = vec![0; remaining];
        stream
            .read_exact(&mut body_tail)
            .expect("request body should read");
        buffer.extend_from_slice(&body_tail);
    }
    String::from_utf8_lossy(&buffer[headers_end..]).to_string()
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

fn noisy_file_source() -> &'static str {
    r#"
issues:
  - identifier: COE-123
    title: Repo metadata cleanup
    url: https://linear.app/example/issue/COE-123
    description: Update root repository metadata.
    state: Done
    labels:
      - general
    linked_prs:
      - 456
prs:
  - number: 456
    title: COE-123 update repository metadata
    url: https://github.com/example/repo/pull/456
    branch: coe-123-metadata
    merge_sha: abcdef1234567890
    changed_files:
      - path: README.md
        change_kind: modified
      - path: Cargo.toml
        change_kind: modified
      - path: Cargo.lock
        change_kind: modified
"#
}

fn candidate_only_source() -> &'static str {
    r#"
issues:
  - identifier: COE-123
    title: Repo metadata cleanup
    url: https://linear.app/example/issue/COE-123
    description: Update root repository metadata.
    state: Done
    linked_prs:
      - 456
prs:
  - number: 456
    title: COE-123 update repository metadata
    url: https://github.com/example/repo/pull/456
    branch: coe-123-metadata
    merge_sha: abcdef1234567890
    changed_files:
      - path: README.md
        change_kind: modified
"#
}

fn multi_repo_source() -> &'static str {
    r#"
issues:
  - identifier: COE-201
    title: Shared broker API work
    url: https://linear.app/example/issue/COE-201
    description: Shared broker update in the API service.
    state: Done
    labels:
      - runtime
    linked_prs:
      - 201
  - identifier: COE-202
    title: Shared broker web work
    url: https://linear.app/example/issue/COE-202
    description: Shared broker update in the web service.
    state: Done
    labels:
      - runtime
    linked_prs:
      - 202
prs:
  - number: 201
    title: COE-201 shared broker api
    url: https://github.com/example/repo/pull/201
    changed_files:
      - path: services/api/src/broker.rs
        change_kind: modified
  - number: 202
    title: COE-202 shared broker web
    url: https://github.com/example/repo/pull/202
    changed_files:
      - path: services/web/src/broker.ts
        change_kind: modified
"#
}

fn sample_two_issue_source() -> &'static str {
    r#"
issues:
  - identifier: COE-123
    title: First archive candidate
    url: https://linear.app/example/issue/COE-123
    description: First completed issue.
    state: Done
    completed_at: 2026-06-13T17:00:00Z
    labels:
      - runtime
    linked_prs:
      - 456
  - identifier: COE-124
    title: Second archive candidate
    url: https://linear.app/example/issue/COE-124
    description: Second completed issue.
    state: Done
    completed_at: 2026-06-14T17:00:00Z
    labels:
      - runtime
    linked_prs:
      - 457
prs:
  - number: 456
    title: COE-123 first archive candidate
    url: https://github.com/example/repo/pull/456
    changed_files:
      - path: crates/opensymphony-openhands/src/client.rs
        change_kind: modified
  - number: 457
    title: COE-124 second archive candidate
    url: https://github.com/example/repo/pull/457
    changed_files:
      - path: crates/opensymphony-openhands/src/client.rs
        change_kind: modified
"#
}
