use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
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
        stdout.contains("skipping `cargo install opensymphony`"),
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
        cargo_log.contains("ARGS=install opensymphony"),
        "cargo install should use the requested command: {cargo_log}",
    );
    assert!(
        stdout.contains("Skipped template skill refresh because this directory is missing `WORKFLOW.md` and `config.yaml`."),
        "stdout should explain why the skill refresh was skipped: {stdout}",
    );
}

async fn run_update(
    repo_root: &Path,
    cargo_log: &Path,
    server: &UpdateServer,
) -> std::process::Output {
    let fake_bin_dir = repo_root.join(".test-bin");
    fs::create_dir_all(&fake_bin_dir).expect("fake bin dir should exist");
    write_fake_cargo(fake_bin_dir.join("cargo"), cargo_log);

    Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("update")
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
    task: tokio::task::JoinHandle<()>,
}

impl UpdateServer {
    async fn start(latest_version: &str) -> Self {
        let state = Arc::new(ServerState {
            latest_version: latest_version.to_string(),
            assets: template_assets(),
        });
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
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn crate_metadata_url(&self) -> &str {
        &self.crate_metadata_url
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
}

async fn update_handler(
    State(state): State<Arc<ServerState>>,
    uri: Uri,
    _request: Request,
) -> Response {
    let path = uri.path().trim_start_matches('/');
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
    std::env::join_paths([path]).expect("path should join")
}

fn write_fake_cargo(path: PathBuf, log_path: &Path) {
    write_executable(
        path,
        &format!(
            "#!/bin/sh\nset -eu\nprintf 'PWD=%s\\n' \"$PWD\" >> \"{}\"\nprintf 'ARGS=%s\\n' \"$*\" >> \"{}\"\n",
            log_path.display(),
            log_path.display(),
        ),
    );
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
