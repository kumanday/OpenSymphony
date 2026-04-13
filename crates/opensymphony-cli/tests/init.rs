use std::{collections::BTreeMap, fs, process::Stdio, sync::Arc, time::Duration};

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
    write_stdin(&mut child, "\ndemo-project\n").await;

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
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should exist");
    let config = fs::read_to_string(repo.path().join("config.yaml")).expect("config should exist");
    assert!(workflow.contains("project_slug: \"demo-project\""));
    assert!(workflow.contains("git clone --depth 1 'https://github.com/example/demo.git' ."));
    assert!(config.contains("tool_dir: ~/.opensymphony/openhands-server"));

    assert!(
        repo.path().join("AGENTS.md").is_file(),
        "AGENTS.md should be created"
    );
    assert!(
        repo.path().join(".agents/skills/pull/SKILL.md").is_file(),
        "skill file should be created"
    );
    assert!(
        repo.path()
            .join(".agents/skills/commit/scripts/helper.sh")
            .is_file(),
        "skill helper files should be copied recursively"
    );
    assert!(
        repo.path()
            .join(".agents/skills/linear/references/using-the-helper.md")
            .is_file(),
        "linear reference file should be created"
    );
    assert!(
        repo.path().join("config.yaml").is_file(),
        "config.yaml should be created"
    );
    assert!(
        !repo.path().join("docs/tasks/README.md").exists(),
        "target repos should not receive docs/tasks bootstrap files"
    );
    assert!(
        !repo.path().join(".gitignore").exists(),
        "target repos should not receive the template .gitignore"
    );
    assert!(
        !repo
            .path()
            .join(".github/workflows/ai-pr-review.yml")
            .exists(),
        "AI PR review workflow should not be added unless requested"
    );
    assert!(
        stdout.contains("Initialization summary"),
        "stdout should contain a summary: {stdout}",
    );
}

#[tokio::test]
async fn init_can_scaffold_ai_pr_review_and_print_fallback_commands_when_gh_cannot_access_repo() {
    let server = TemplateServer::start().await;
    let repo = TempDir::new().expect("temp repo should exist");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    let mut child = spawn_init_child(repo.path(), server.base_url(), &[]);
    write_stdin(&mut child, "yes\n\n\n\n\n\ndemo-project\n").await;

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
    assert!(
        repo.path()
            .join(".github/workflows/ai-pr-review.yml")
            .is_file(),
        "AI PR review workflow should be created"
    );
    assert!(
        repo.path()
            .join(".agents/skills/custom-codereview-guide.md")
            .is_file(),
        "starter review guide should be created"
    );
    assert!(
        !repo
            .path()
            .join("docs/ai-pr-review-human-setup.md")
            .exists(),
        "AI PR review should not create repo-local docs setup files"
    );
    assert!(
        stdout.contains(
            "For the managed local OpenHands server, run `opensymphony install openhands`"
        ),
        "stdout should present managed local OpenHands as the normal path: {stdout}",
    );
    assert!(
        stdout.contains("OpenHands PR review scaffolding was added."),
        "stdout should contain AI review guidance: {stdout}",
    );
    assert!(
        stdout.contains(
            "gh variable set AI_REVIEW_MODEL_ID -R example/demo --body 'accounts/fireworks/models/glm-5p1'"
        ),
        "stdout should contain GitHub variable commands: {stdout}",
    );
    assert!(
        stdout.contains(
            "Manual setup guide: https://github.com/kumanday/OpenSymphony/blob/main/docs/ai-pr-review-human-setup.md"
        ),
        "stdout should point to the upstream setup guide: {stdout}",
    );
    assert!(
        stdout.contains("`gh` could not access `example/demo`"),
        "stdout should explain why automation fell back to manual commands: {stdout}",
    );
}

#[tokio::test]
async fn init_can_scaffold_ai_pr_review_and_configure_github_with_gh() {
    let server = TemplateServer::start().await;
    let repo = TempDir::new().expect("temp repo should exist");
    let gh_log = repo.path().join("gh.log");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    let mut child = spawn_init_child_with_env(
        repo.path(),
        server.base_url(),
        &[],
        &[
            ("OPENSYMPHONY_TEST_GH_MODE", "success"),
            (
                "OPENSYMPHONY_TEST_GH_LOG",
                gh_log.to_str().expect("gh log path should be valid"),
            ),
        ],
    );
    write_stdin(&mut child, "yes\n\n\n\n\n\ndemo-project\nyes\nyes\n").await;

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
    assert!(
        stdout.contains("GitHub Actions settings for `example/demo` were configured with `gh`."),
        "stdout should confirm GitHub automation: {stdout}",
    );
    assert!(
        !stdout.contains("gh variable set AI_REVIEW_MODEL_ID"),
        "successful automation should not dump fallback gh commands: {stdout}",
    );

    let gh_log = fs::read_to_string(&gh_log).expect("gh log should exist");
    assert!(
        gh_log.contains("gh --version"),
        "preflight should verify gh exists: {gh_log}",
    );
    assert!(
        gh_log.contains("gh repo view example/demo --json nameWithOwner"),
        "preflight should verify repo access: {gh_log}",
    );
    assert!(
        gh_log.contains(
            "gh variable set AI_REVIEW_PROVIDER_KIND -R example/demo --body openai-compatible"
        ),
        "provider variable should be configured: {gh_log}",
    );
    assert!(
        gh_log.contains("gh label create review-this -R example/demo --description Trigger AI PR review --color d73a4a --force"),
        "label should be ensured: {gh_log}",
    );
    assert!(
        gh_log.contains("gh secret set AI_REVIEW_API_KEY -R example/demo"),
        "secret should be configured when the user reuses LLM_API_KEY: {gh_log}",
    );
}

#[tokio::test]
async fn init_merges_agents_and_skips_conflicting_file_when_requested() {
    let server = TemplateServer::start().await;
    let repo = TempDir::new().expect("temp repo should exist");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    fs::write(
        repo.path().join("AGENTS.md"),
        "# Existing Agents\n\nKeep me.\n",
    )
    .expect("existing AGENTS should write");
    fs::create_dir_all(repo.path().join(".github")).expect(".github should exist");
    fs::write(
        repo.path().join(".github/pull_request_template.md"),
        "keep this template\n",
    )
    .expect("existing PR template should write");

    let mut child = spawn_init_child(repo.path(), server.base_url(), &[]);
    write_stdin(&mut child, "\nskip\ndemo-project\n").await;

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

    let agents = fs::read_to_string(repo.path().join("AGENTS.md")).expect("AGENTS.md should exist");
    assert!(
        agents.contains("## Preserved Existing AGENTS.md"),
        "existing AGENTS content should be preserved: {agents}",
    );
    assert!(
        agents.contains("# Existing Agents\n\nKeep me."),
        "existing AGENTS text should be appended: {agents}",
    );

    let pr_template = fs::read_to_string(repo.path().join(".github/pull_request_template.md"))
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

    fs::write(repo.path().join("WORKFLOW.md"), "user workflow\n")
        .expect("existing workflow should write");

    let mut child = spawn_init_child(repo.path(), server.base_url(), &[]);
    write_stdin(&mut child, "\nabort\n").await;

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
        fs::read_to_string(repo.path().join("WORKFLOW.md")).expect("workflow should still exist"),
        "user workflow\n"
    );
    assert!(
        !repo.path().join("AGENTS.md").exists(),
        "no additional files should be written after abort",
    );
}

#[tokio::test]
async fn init_fails_when_template_fetch_times_out() {
    let server = TemplateServer::start_with_delay(Duration::from_millis(250)).await;
    let repo = TempDir::new().expect("temp repo should exist");
    init_git_repo(repo.path(), "https://github.com/example/demo.git");

    let mut child = spawn_init_child_with_env(
        repo.path(),
        server.base_url(),
        &[],
        &[("OPENSYMPHONY_TEMPLATE_FETCH_TIMEOUT_MS", "50")],
    );
    write_stdin(&mut child, "\n").await;

    let output = child
        .wait_with_output()
        .await
        .expect("init command should finish");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "init should fail on template fetch timeout: stdout={stdout}, stderr={stderr}",
    );
    assert!(
        stdout.contains("opensymphony init failed: failed to fetch template tree"),
        "stdout should report the fetch failure: {stdout}",
    );
    assert!(
        !repo.path().join("WORKFLOW.md").exists(),
        "no files should be written when the template fetch times out",
    );
}

fn spawn_init_child(
    repo_root: &std::path::Path,
    template_base_url: &str,
    extra_args: &[&str],
) -> tokio::process::Child {
    spawn_init_child_with_env(repo_root, template_base_url, extra_args, &[])
}

fn spawn_init_child_with_env(
    repo_root: &std::path::Path,
    template_base_url: &str,
    extra_args: &[&str],
    extra_env: &[(&str, &str)],
) -> tokio::process::Child {
    let gh_bin_dir = repo_root.join(".test-bin");
    fs::create_dir_all(&gh_bin_dir).expect("fake gh bin dir should exist");
    let gh_bin = gh_bin_dir.join("gh");
    fs::write(&gh_bin, fake_gh_script()).expect("fake gh should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(&gh_bin)
            .expect("fake gh metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&gh_bin, permissions).expect("fake gh should be executable");
    }
    let path = format!(
        "{}:{}",
        gh_bin_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut command = Command::new(env!("CARGO_BIN_EXE_opensymphony"));
    command
        .arg("init")
        .args(extra_args)
        .current_dir(repo_root)
        .env("PATH", path)
        .env("OPENSYMPHONY_TEMPLATE_BASE_URL", template_base_url)
        .env("OPENSYMPHONY_TEST_GH_MODE", "deny-repo")
        .env("LLM_MODEL", "already-set-model")
        .env("LLM_API_KEY", "already-set-key")
        .env("LLM_BASE_URL", "https://example.com/llm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (name, value) in extra_env {
        command.env(name, value);
    }
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
        Self::start_with_delay(Duration::ZERO).await
    }

    async fn start_with_delay(delay: Duration) -> Self {
        let assets = Arc::new(template_assets());
        let app = Router::new()
            .fallback(get(template_handler))
            .with_state((assets, delay));
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
    State((assets, delay)): State<(Arc<BTreeMap<String, String>>, Duration)>,
    uri: Uri,
    _request: Request,
) -> Response {
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    let path = uri.path().trim_start_matches('/');
    if path == "__tree.json" {
        let tree = assets
            .keys()
            .map(|path| serde_json::json!({ "path": path, "type": "blob" }))
            .collect::<Vec<_>>();
        return (
            StatusCode::OK,
            serde_json::json!({ "tree": tree }).to_string(),
        )
            .into_response();
    }
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
            "control_plane:\n  bind: 127.0.0.1:2468\n\nopenhands:\n  tool_dir: ~/.opensymphony/openhands-server\n".to_string(),
        ),
        (
            ".agents/skills/commit/SKILL.md".to_string(),
            "# commit\n".to_string(),
        ),
        (
            ".agents/skills/commit/scripts/helper.sh".to_string(),
            "#!/usr/bin/env bash\necho helper\n".to_string(),
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
            ".agents/skills/linear/scripts/linear_graphql.py".to_string(),
            "#!/usr/bin/env python3\n".to_string(),
        ),
        (
            ".agents/skills/linear/references/using-the-helper.md".to_string(),
            "# helper\n".to_string(),
        ),
        (
            ".agents/skills/linear/references/issue-and-comment-operations.md".to_string(),
            "# issue ops\n".to_string(),
        ),
        (
            ".agents/skills/linear/references/project-and-advanced-operations.md".to_string(),
            "# project ops\n".to_string(),
        ),
        (
            ".agents/skills/linear/queries/viewer.graphql".to_string(),
            "query Viewer { viewer { id } }\n".to_string(),
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
    ])
}

fn fake_gh_script() -> &'static str {
    r#"#!/bin/sh
set -eu

mode="${OPENSYMPHONY_TEST_GH_MODE:-deny-repo}"
log_path="${OPENSYMPHONY_TEST_GH_LOG:-}"

log_command() {
  if [ -n "$log_path" ]; then
    printf 'gh %s\n' "$*" >> "$log_path"
  fi
}

case "${1-}" in
  --version)
    log_command "$*"
    printf 'gh version test\n'
    exit 0
    ;;
  repo)
    log_command "$*"
    if [ "$mode" = "success" ]; then
      printf '{"nameWithOwner":"example/demo"}\n'
      exit 0
    fi
    printf 'authentication required or repository access denied\n' >&2
    exit 1
    ;;
  variable)
    log_command "$*"
    if [ "$mode" = "success" ]; then
      exit 0
    fi
    printf 'repository settings access denied\n' >&2
    exit 1
    ;;
  label)
    log_command "$*"
    if [ "$mode" = "success" ]; then
      exit 0
    fi
    printf 'label write access denied\n' >&2
    exit 1
    ;;
  secret)
    log_command "$*"
    cat >/dev/null
    if [ "$mode" = "success" ]; then
      exit 0
    fi
    printf 'secret write access denied\n' >&2
    exit 1
    ;;
  *)
    log_command "$*"
    printf 'unexpected gh invocation: %s\n' "$*" >&2
    exit 1
    ;;
esac
"#
}
