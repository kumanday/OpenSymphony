use std::{collections::BTreeMap, process::Stdio, sync::Arc};

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use tempfile::TempDir;
use tokio::{io::AsyncWriteExt, net::TcpListener, process::Command};

#[tokio::test]
async fn init_copies_template_files_and_customizes_workflow() {
    let server = TemplateServer::start().await;
    let repo = TempDir::new().expect("temp repo should exist");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    let mut child = spawn_init_child(repo.path(), server.base_url(), &[]);
    write_stdin(&mut child, "demo-project\n").await;

    let output = child
        .wait_with_output()
        .await
        .expect("init command should finish");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "init should succeed: stdout={stdout}, stderr={stderr}",
    );

    let workflow =
        std::fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should exist");
    assert!(workflow.contains("project_slug: \"demo-project\""));
    assert!(workflow.contains("git clone --depth 1 'https://github.com/example/demo.git' ."));

    assert!(
        repo.path().join("AGENTS.md").is_file(),
        "AGENTS.md should be created"
    );
    assert!(
        repo.path().join(".agents/skills/pull/SKILL.md").is_file(),
        "skill file should be created"
    );
    assert!(
        repo.path().join("config.yaml").is_file(),
        "config.yaml should be created"
    );
    assert!(
        stdout.contains("Initialization summary"),
        "stdout should contain a summary: {stdout}",
    );
}

#[tokio::test]
async fn init_merges_agents_and_skips_conflicting_file_when_requested() {
    let server = TemplateServer::start().await;
    let repo = TempDir::new().expect("temp repo should exist");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    std::fs::write(
        repo.path().join("AGENTS.md"),
        "# Existing Agents\n\nKeep me.\n",
    )
    .expect("existing AGENTS should write");
    std::fs::create_dir_all(repo.path().join(".github")).expect(".github should exist");
    std::fs::write(
        repo.path().join(".github/pull_request_template.md"),
        "keep this template\n",
    )
    .expect("existing PR template should write");

    let mut child = spawn_init_child(repo.path(), server.base_url(), &[]);
    write_stdin(&mut child, "skip\ndemo-project\n").await;

    let output = child
        .wait_with_output()
        .await
        .expect("init command should finish");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "init should succeed: stdout={stdout}, stderr={stderr}",
    );

    let agents =
        std::fs::read_to_string(repo.path().join("AGENTS.md")).expect("AGENTS.md should exist");
    assert!(
        agents.contains("## Preserved Existing AGENTS.md"),
        "existing AGENTS content should be preserved: {agents}",
    );
    assert!(
        agents.contains("# Existing Agents\n\nKeep me."),
        "existing AGENTS text should be appended: {agents}",
    );

    let pr_template = std::fs::read_to_string(repo.path().join(".github/pull_request_template.md"))
        .expect("PR template should exist");
    assert_eq!(pr_template, "keep this template\n");
    assert!(
        stdout.contains("- .github/pull_request_template.md"),
        "skipped file should appear in summary: {stdout}",
    );
}

#[tokio::test]
async fn init_aborts_before_writing_when_user_requests_abort() {
    let server = TemplateServer::start().await;
    let repo = TempDir::new().expect("temp repo should exist");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    std::fs::write(repo.path().join("WORKFLOW.md"), "user workflow\n")
        .expect("existing workflow should write");

    let mut child = spawn_init_child(repo.path(), server.base_url(), &[]);
    write_stdin(&mut child, "abort\n").await;

    let output = child
        .wait_with_output()
        .await
        .expect("init command should finish");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "init should fail when aborted: stdout={stdout}, stderr={stderr}",
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("WORKFLOW.md"))
            .expect("workflow should still exist"),
        "user workflow\n"
    );
    assert!(
        !repo.path().join("AGENTS.md").exists(),
        "no additional files should be written after abort",
    );
}

fn spawn_init_child(
    repo_root: &std::path::Path,
    template_base_url: &str,
    extra_args: &[&str],
) -> tokio::process::Child {
    let mut command = Command::new(env!("CARGO_BIN_EXE_opensymphony"));
    command
        .arg("init")
        .args(extra_args)
        .current_dir(repo_root)
        .env("OPENSYMPHONY_TEMPLATE_BASE_URL", template_base_url)
        .env("LLM_MODEL", "already-set-model")
        .env("LLM_API_KEY", "already-set-key")
        .env("LLM_BASE_URL", "https://example.com/llm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.spawn().expect("init command should spawn")
}

async fn write_stdin(child: &mut tokio::process::Child, input: &str) {
    let mut stdin = child.stdin.take().expect("stdin should exist");
    stdin
        .write_all(input.as_bytes())
        .await
        .expect("stdin should accept scripted input");
    drop(stdin);
}

fn init_git_repo(repo_root: &std::path::Path, remote_url: &str) {
    run_git(repo_root, &["init", "-q"]);
    run_git(repo_root, &["remote", "add", "origin", remote_url]);
}

fn run_git(repo_root: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .status()
        .expect("git should run");
    assert!(status.success(), "git {:?} should succeed", args);
}

struct TemplateServer {
    base_url: String,
    task: tokio::task::JoinHandle<()>,
}

impl TemplateServer {
    async fn start() -> Self {
        let assets = Arc::new(template_assets());
        let app = Router::new()
            .fallback(get(template_handler))
            .with_state(assets);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("template server should bind");
        let address = listener
            .local_addr()
            .expect("template server should have an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("template server should run");
        });

        Self {
            base_url: format!("http://{address}/"),
            task,
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Drop for TemplateServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn template_handler(
    State(assets): State<Arc<BTreeMap<String, String>>>,
    uri: Uri,
    _request: Request,
) -> Response {
    let path = uri.path().trim_start_matches('/');
    match assets.get(path) {
        Some(content) => (StatusCode::OK, content.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, format!("missing asset {path}")).into_response(),
    }
}

fn template_assets() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "WORKFLOW.md".to_string(),
            r#"---
tracker:
  kind: linear
  project_slug: "YOUR-PROJECT-SLUG"
hooks:
  after_create: |
    git clone --depth 1 https://github.com/YOUR-ORG/YOUR-REPO.git .
openhands:
  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
---
"#
            .to_string(),
        ),
        (
            "AGENTS.md".to_string(),
            "# AGENTS.md\n\nTemplate agents.\n".to_string(),
        ),
        (
            "config.yaml".to_string(),
            "control_plane:\n  bind: 127.0.0.1:2468\n".to_string(),
        ),
        (".gitignore".to_string(), ".opensymphony/\n".to_string()),
        (
            ".agents/skills/commit/SKILL.md".to_string(),
            "# commit\n".to_string(),
        ),
        (
            ".agents/skills/convert-tasks-to-linear/SKILL.md".to_string(),
            "# convert\n".to_string(),
        ),
        (
            ".agents/skills/create-implementation-plan/SKILL.md".to_string(),
            "# plan\n".to_string(),
        ),
        (
            ".agents/skills/land/SKILL.md".to_string(),
            "# land\n".to_string(),
        ),
        (
            ".agents/skills/linear/SKILL.md".to_string(),
            "# linear\n".to_string(),
        ),
        (
            ".agents/skills/pull/SKILL.md".to_string(),
            "# pull\n".to_string(),
        ),
        (
            ".agents/skills/push/SKILL.md".to_string(),
            "# push\n".to_string(),
        ),
        (".github/CODEOWNERS".to_string(), "* @example\n".to_string()),
        (
            ".github/pull_request_template.md".to_string(),
            "template body\n".to_string(),
        ),
        (
            ".github/workflows/ai-pr-review.yml".to_string(),
            "name: ai-pr-review\n".to_string(),
        ),
        ("docs/tasks/README.md".to_string(), "# Tasks\n".to_string()),
    ])
}
