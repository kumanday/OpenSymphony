use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use tempfile::TempDir;
use tokio::{net::TcpListener, process::Command};

#[tokio::test]
async fn update_skips_reinstall_when_current_matches_latest_and_refreshes_skills() {
    let server = UpdateServer::start(env!("CARGO_PKG_VERSION")).await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    fs::write(repo.path().join("WORKFLOW.md"), "# workflow\n").expect("workflow should write");
    fs::write(
        repo.path().join("config.yaml"),
        "openhands:\n  tool_dir: ~/.opensymphony\n",
    )
    .expect("config should write");
    fs::create_dir_all(repo.path().join(".agents/skills/linear"))
        .expect("linear skill dir should exist");
    fs::write(
        repo.path().join(".agents/skills/linear/SKILL.md"),
        "# stale linear\n",
    )
    .expect("stale linear skill should write");
    fs::create_dir_all(repo.path().join(".agents/skills/commit"))
        .expect("commit skill dir should exist");
    fs::write(
        repo.path().join(".agents/skills/commit/SKILL.md"),
        "# commit\n",
    )
    .expect("commit skill should write");
    fs::create_dir_all(repo.path().join(".agents/skills/local-only"))
        .expect("local-only dir should exist");
    fs::write(
        repo.path().join(".agents/skills/local-only/SKILL.md"),
        "# keep me\n",
    )
    .expect("local skill should write");

    let output = run_update(repo.path(), &cargo_log, &server).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        0,
        "cargo should not run when the installed version is current",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/linear/SKILL.md"))
            .expect("linear skill should exist"),
        "# linear\n",
    );
    assert!(
        repo.path().join(".agents/skills/push/SKILL.md").is_file(),
        "new template-managed skills should be created",
    );
    assert!(
        !repo
            .path()
            .join(".agents/skills/opensymphony-memory/SKILL.md")
            .exists(),
        "memory skill should only be refreshed when the template repo provides it",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".agents/skills/local-only/SKILL.md"))
            .expect("local-only skill should survive"),
        "# keep me\n",
    );
    let memory_config = fs::read_to_string(repo.path().join(".opensymphony/memory/memory.yaml"))
        .expect("update should initialize memory config in target repos");
    assert!(
        memory_config.contains("memory_root: .opensymphony/memory"),
        "memory config should contain the default memory root: {memory_config}",
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".gitignore")).expect(".gitignore should exist"),
        memory_gitignore_policy("")
    );
    assert!(
        !repo.path().join("AGENTS.md").exists(),
        "update should not create other bootstrap assets",
    );
    assert!(
        !repo.path().join(".github/CODEOWNERS").exists(),
        "update should not copy .github bootstrap files",
    );
    assert!(
        stdout.contains("skipping `cargo install opensymphony --locked`"),
        "stdout should explain the skipped reinstall: {stdout}",
    );
    assert!(
        stdout.contains("Detected an OpenSymphony target repo"),
        "stdout should explain why skills were refreshed: {stdout}",
    );
    assert!(
        stdout.contains("Updated:") && stdout.contains("- .agents/skills/linear/SKILL.md"),
        "stdout should list updated skill files: {stdout}",
    );
    assert!(
        stdout.contains("Created:")
            && stdout.contains("- .agents/skills/push/SKILL.md")
            && !stdout.contains("- .agents/skills/opensymphony-memory/SKILL.md"),
        "stdout should list created skill files: {stdout}",
    );
    assert!(
        stdout.contains("Memory init summary:")
            && stdout.contains("- .opensymphony/memory/memory.yaml")
            && stdout.contains("- .gitignore"),
        "stdout should list memory initialization files: {stdout}",
    );
}

#[tokio::test]
async fn update_installs_when_latest_is_newer_and_skips_skill_refresh_outside_target_repo() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    let output = run_update(repo.path(), &cargo_log, &server).await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        1,
        "cargo install should run when a newer published version exists",
    );
    let cargo_log = fs::read_to_string(&cargo_log).expect("cargo log should exist");
    assert!(
        cargo_log.contains("ARGS=install opensymphony --locked"),
        "cargo install should use the published lockfile: {cargo_log}",
    );
    assert!(
        stdout.contains("Skipped template skill refresh because this directory is missing `WORKFLOW.md` and `config.yaml`."),
        "stdout should explain why the skill refresh was skipped: {stdout}",
    );
}

#[tokio::test]
async fn marker_only_target_branch_update_skips_reinstall_template_fetch_and_memory_bootstrap() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    fs::write(
        repo.path().join("WORKFLOW.md"),
        r#"## Branch target

Target branch: `main`

Keep feature branches current with `origin/main`.
- `pull`: keep branch updated with latest origin/main before handoff.
- `pull`: keep branch updated with latest `origin/main` before handoff.
Run the pull skill to sync with latest origin/main before any code edits.
Leave https://github.com/origin/main.git unchanged.
Do not delete `origin/main`.

## Automated AI PR review

Active review provider: `openhands`
"#,
    )
    .expect("workflow should write");
    fs::write(repo.path().join("config.yaml"), "tracker: {}\n").expect("config should write");

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--target-branch", "develop"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        0,
        "marker-only mode should not run cargo install",
    );
    assert!(
        server.requested_paths().is_empty(),
        "marker-only mode should not fetch crate metadata or template assets: {:?}",
        server.requested_paths(),
    );
    assert!(
        !repo
            .path()
            .join(".opensymphony/memory/memory.yaml")
            .exists(),
        "marker-only mode should not initialize memory",
    );
    let workflow =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should read");
    assert!(workflow.contains("Target branch: `develop`"));
    assert!(workflow.contains("Active review provider: `openhands`"));
    assert!(workflow.contains("Keep feature branches current with `origin/develop`."));
    assert!(workflow.contains("latest origin/develop before handoff"));
    assert!(workflow.contains("latest `origin/develop` before handoff"));
    assert!(workflow.contains("sync with latest origin/develop before"));
    assert!(workflow.contains("https://github.com/origin/main.git"));
    assert!(workflow.contains("Do not delete `origin/main`."));
}

#[tokio::test]
async fn marker_only_code_review_update_changes_only_provider_marker() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    fs::write(
        repo.path().join("WORKFLOW.md"),
        r#"## Branch target

Target branch: `main`

Keep feature branches current with `origin/main`.

## Automated AI PR review

Active review provider: `openhands`
"#,
    )
    .expect("workflow should write");
    fs::write(repo.path().join("config.yaml"), "tracker: {}\n").expect("config should write");

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--code-review", "codex"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(cargo_invocation_count(&cargo_log), 0);
    assert!(server.requested_paths().is_empty());
    let workflow =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should read");
    assert!(workflow.contains("Target branch: `main`"));
    assert!(workflow.contains("Keep feature branches current with `origin/main`."));
    assert!(workflow.contains("Active review provider: `codex`"));
}

#[tokio::test]
async fn marker_only_combined_update_changes_both_markers() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    fs::write(
        repo.path().join("WORKFLOW.md"),
        r#"## Branch target

Target branch: `main`

## Automated AI PR review

Active review provider: `openhands`
"#,
    )
    .expect("workflow should write");
    fs::write(repo.path().join("config.yaml"), "tracker: {}\n").expect("config should write");

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--target-branch", "develop", "--code-review", "codex"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    let workflow =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should read");
    assert!(workflow.contains("Target branch: `develop`"));
    assert!(workflow.contains("Active review provider: `codex`"));
    assert_eq!(cargo_invocation_count(&cargo_log), 0);
    assert!(server.requested_paths().is_empty());
}

#[tokio::test]
async fn marker_only_openhands_warns_when_review_workflow_file_is_absent() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    fs::write(
        repo.path().join("WORKFLOW.md"),
        "## Automated AI PR review\n\nActive review provider: `none`\n",
    )
    .expect("workflow should write");
    fs::write(repo.path().join("config.yaml"), "tracker: {}\n").expect("config should write");

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--code-review", "openhands"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stderr.contains("Warning: `--code-review openhands` updated WORKFLOW.md"),
        "stderr should warn about missing OpenHands workflow: {stderr}",
    );
}

#[tokio::test]
async fn marker_only_openhands_enables_existing_review_workflow() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    let gh_log = repo.path().join("gh.log");
    write_fake_gh(repo.path().join(".test-bin/gh"), &gh_log, 0);
    write_target_repo_files(repo.path(), "none");
    write_openhands_review_workflow(repo.path());

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--code-review", "openhands"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update should succeed: stdout={stdout}, stderr={stderr}",
    );
    let gh_log = fs::read_to_string(gh_log).expect("gh log should exist");
    assert!(
        gh_log.contains("ARGS=workflow enable ai-pr-review.yml"),
        "update should enable the existing OpenHands workflow: {gh_log}",
    );
    assert!(
        stdout.contains("enabled existing OpenHands GitHub Actions review workflow."),
        "stdout should report workflow sync: {stdout}",
    );
}

#[tokio::test]
async fn marker_only_non_openhands_providers_disable_existing_review_workflow() {
    for provider in ["codex", "none"] {
        let server = UpdateServer::start("9.9.9").await;
        let repo = TempDir::new().expect("temp repo should exist");
        let cargo_log = repo.path().join("cargo.log");
        let gh_log = repo.path().join("gh.log");
        write_fake_gh(repo.path().join(".test-bin/gh"), &gh_log, 0);
        write_target_repo_files(repo.path(), "openhands");
        write_openhands_review_workflow(repo.path());

        let output = run_update_with_args(
            repo.path(),
            &cargo_log,
            &server,
            &["--code-review", provider],
        )
        .await;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
            output.status.success(),
            "update should succeed for {provider}: stdout={stdout}, stderr={stderr}",
        );
        let gh_log = fs::read_to_string(gh_log).expect("gh log should exist");
        assert!(
            gh_log.contains("ARGS=workflow disable ai-pr-review.yml"),
            "update should disable the existing OpenHands workflow for {provider}: {gh_log}",
        );
    }
}

#[tokio::test]
async fn marker_only_workflow_toggle_failure_warns_and_keeps_marker_update() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");
    let gh_log = repo.path().join("gh.log");
    write_fake_gh(repo.path().join(".test-bin/gh"), &gh_log, 42);
    write_target_repo_files(repo.path(), "openhands");
    write_openhands_review_workflow(repo.path());

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--code-review", "codex"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "toggle failure should not fail marker update: stdout={stdout}, stderr={stderr}",
    );
    let workflow =
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should read");
    assert!(workflow.contains("Active review provider: `codex`"));
    assert!(
        stderr.contains("Warning: `gh workflow disable ai-pr-review.yml` exited with exit code 42")
            && stderr.contains("WORKFLOW.md marker remains updated"),
        "stderr should warn with the failed command: {stderr}",
    );
}

#[tokio::test]
async fn marker_only_update_requires_target_repo_markers_before_side_effects() {
    let server = UpdateServer::start("9.9.9").await;
    let repo = TempDir::new().expect("temp repo should exist");
    let cargo_log = repo.path().join("cargo.log");

    let output = run_update_with_args(
        repo.path(),
        &cargo_log,
        &server,
        &["--code-review", "codex"],
    )
    .await;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "update should fail outside a target repo: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        cargo_invocation_count(&cargo_log),
        0,
        "marker-only mode should not run cargo install before marker validation",
    );
    assert!(
        server.requested_paths().is_empty(),
        "marker-only mode should not fetch before marker validation: {:?}",
        server.requested_paths(),
    );
    assert!(
        stderr.contains("workflow settings mode requires an OpenSymphony target repo"),
        "stderr should explain missing target repo markers: {stderr}",
    );
}

async fn run_update(
    repo_root: &Path,
    cargo_log: &Path,
    server: &UpdateServer,
) -> std::process::Output {
    run_update_with_args(repo_root, cargo_log, server, &[]).await
}

async fn run_update_with_args(
    repo_root: &Path,
    cargo_log: &Path,
    server: &UpdateServer,
    args: &[&str],
) -> std::process::Output {
    let fake_bin_dir = repo_root.join(".test-bin");
    fs::create_dir_all(&fake_bin_dir).expect("fake bin dir should exist");
    write_fake_cargo(fake_bin_dir.join("cargo"), cargo_log);

    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("update")
        .args(args)
        .current_dir(repo_root)
        .env("PATH", path_only(fake_bin_dir.as_path()))
        .env("OPENSYMPHONY_TEMPLATE_BASE_URL", server.base_url())
        .env(
            "OPENSYMPHONY_UPDATE_CRATE_METADATA_URL",
            server.crate_metadata_url(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .expect("update command should run")
}

struct UpdateServer {
    base_url: String,
    crate_metadata_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl UpdateServer {
    async fn start(latest_version: &str) -> Self {
        let state = Arc::new(ServerState {
            latest_version: latest_version.to_string(),
            assets: template_assets(),
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let requests = Arc::clone(&state.requests);
        let app = Router::new()
            .fallback(get(update_handler))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("update server should bind");
        let address = listener
            .local_addr()
            .expect("update server should have an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("update server should run");
        });

        Self {
            base_url: format!("http://{address}/"),
            crate_metadata_url: format!("http://{address}/__crate.json"),
            requests,
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn crate_metadata_url(&self) -> &str {
        &self.crate_metadata_url
    }

    fn requested_paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .expect("request log should not be poisoned")
            .clone()
    }
}

impl Drop for UpdateServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ServerState {
    latest_version: String,
    assets: BTreeMap<String, String>,
    requests: Arc<Mutex<Vec<String>>>,
}

async fn update_handler(
    State(state): State<Arc<ServerState>>,
    uri: Uri,
    _request: Request,
) -> Response {
    let path = uri.path().trim_start_matches('/');
    state
        .requests
        .lock()
        .expect("request log should not be poisoned")
        .push(path.to_string());
    if path == "__crate.json" {
        return (
            StatusCode::OK,
            serde_json::json!({
                "crate": {
                    "max_version": state.latest_version,
                }
            })
            .to_string(),
        )
            .into_response();
    }

    if path == "__tree.json" {
        let tree = state
            .assets
            .keys()
            .map(|path| serde_json::json!({ "path": path, "type": "blob" }))
            .collect::<Vec<_>>();
        return (
            StatusCode::OK,
            serde_json::json!({ "tree": tree }).to_string(),
        )
            .into_response();
    }

    match state.assets.get(path) {
        Some(content) => (StatusCode::OK, content.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, format!("missing asset {path}")).into_response(),
    }
}

fn template_assets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            ".agents/skills/commit/SKILL.md".to_string(),
            "# commit\n".to_string(),
        ),
        (
            ".agents/skills/linear/SKILL.md".to_string(),
            "# linear\n".to_string(),
        ),
        (
            ".agents/skills/push/SKILL.md".to_string(),
            "# push\n".to_string(),
        ),
        (
            ".agents/skills/linear/queries/viewer.graphql".to_string(),
            "query Viewer { viewer { id } }\n".to_string(),
        ),
    ])
}

fn cargo_invocation_count(log_path: &Path) -> usize {
    match fs::read_to_string(log_path) {
        Ok(contents) => contents
            .lines()
            .filter(|line| line.starts_with("ARGS="))
            .count(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => 0,
        Err(source) => panic!("cargo log should be readable: {source}"),
    }
}

fn memory_gitignore_policy(prefix: &str) -> String {
    format!(
        "{prefix}.opensymphony*\n!.opensymphony/\n.opensymphony/*\n!.opensymphony/memory/\n.opensymphony/memory/*\n!.opensymphony/memory/memory.yaml\n"
    )
}

fn path_only(path: &Path) -> OsString {
    let mut paths = vec![path.to_path_buf()];
    if let Some(existing) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    std::env::join_paths(paths).expect("path should join")
}

fn write_fake_cargo(path: PathBuf, log_path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fake bin dir should exist");
    }
    write_executable(
        path,
        &format!(
            "#!/bin/sh\nset -eu\nprintf 'PWD=%s\\n' \"$PWD\" >> \"{}\"\nprintf 'ARGS=%s\\n' \"$*\" >> \"{}\"\n",
            log_path.display(),
            log_path.display(),
        ),
    );
}

fn write_fake_gh(path: PathBuf, log_path: &Path, exit_code: i32) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fake bin dir should exist");
    }
    write_executable(
        path,
        &format!(
            "#!/bin/sh\nset -eu\nprintf 'PWD=%s\\n' \"$PWD\" >> \"{}\"\nprintf 'ARGS=%s\\n' \"$*\" >> \"{}\"\nexit {exit_code}\n",
            log_path.display(),
            log_path.display(),
        ),
    );
}

fn write_target_repo_files(repo_root: &Path, provider: &str) {
    fs::write(
        repo_root.join("WORKFLOW.md"),
        format!(
            "## Branch target\n\nTarget branch: `main`\n\n## Automated AI PR review\n\nActive review provider: `{provider}`\n"
        ),
    )
    .expect("workflow should write");
    fs::write(repo_root.join("config.yaml"), "tracker: {}\n").expect("config should write");
}

fn write_openhands_review_workflow(repo_root: &Path) {
    let path = repo_root.join(".github/workflows/ai-pr-review.yml");
    fs::create_dir_all(path.parent().expect("workflow should have parent"))
        .expect("workflow dir should exist");
    fs::write(path, "name: ai-pr-review\n").expect("workflow should write");
}

fn write_executable(path: PathBuf, contents: &str) {
    fs::write(&path, contents).expect("executable should be written");
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
