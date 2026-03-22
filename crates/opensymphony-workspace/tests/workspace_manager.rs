use std::time::Duration;

use opensymphony_workspace::{
    CleanupConfig, CleanupDecision, HookConfig, HookDefinition, HookExecutionStatus, HookKind,
    IssueDescriptor, IssueLifecycleState, RunDescriptor, RunStatus, WorkspaceError,
    WorkspaceManager, WorkspaceManagerConfig,
};
use serde_json::json;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::symlink;

fn sample_issue(identifier: &str) -> IssueDescriptor {
    IssueDescriptor {
        issue_id: format!("id-{identifier}"),
        identifier: identifier.to_string(),
        title: format!("Issue {identifier}"),
        current_state: "In Progress".to_string(),
        last_seen_tracker_refresh_at: None,
    }
}

fn manager_config(
    root: &std::path::Path,
    hooks: HookConfig,
    cleanup: CleanupConfig,
) -> WorkspaceManagerConfig {
    WorkspaceManagerConfig {
        root: root.to_path_buf(),
        hooks,
        cleanup,
    }
}

#[cfg(unix)]
fn current_dir_command(output_path: &str) -> String {
    format!("pwd > {output_path}")
}

#[cfg(windows)]
fn current_dir_command(output_path: &str) -> String {
    format!("cd > {output_path}")
}

#[cfg(unix)]
fn timeout_command() -> &'static str {
    "sleep 1"
}

#[cfg(windows)]
fn timeout_command() -> &'static str {
    "ping 127.0.0.1 -n 2 > NUL"
}

#[cfg(unix)]
fn failing_command() -> &'static str {
    "echo boom 1>&2; exit 7"
}

#[cfg(windows)]
fn failing_command() -> &'static str {
    "echo boom 1>&2 && exit /b 7"
}

#[cfg(unix)]
fn best_effort_failure_command() -> &'static str {
    "echo after-run 1>&2; exit 9"
}

#[cfg(windows)]
fn best_effort_failure_command() -> &'static str {
    "echo after-run 1>&2 && exit /b 9"
}

#[cfg(unix)]
fn after_create_requires_empty_workspace_command() -> &'static str {
    "if [ -e .opensymphony ]; then echo metadata-present 1>&2; exit 17; fi; echo after_create > after_create.txt"
}

#[cfg(windows)]
fn after_create_requires_empty_workspace_command() -> &'static str {
    "if exist .opensymphony\\NUL (echo metadata-present 1>&2 && exit /b 17) else (echo after_create> after_create.txt)"
}

#[cfg(unix)]
fn after_create_retry_command() -> &'static str {
    "if [ ! -f after_create_attempt.txt ]; then echo first > after_create_attempt.txt; echo retry 1>&2; exit 23; fi; echo success > after_create_success.txt"
}

#[cfg(windows)]
fn after_create_retry_command() -> &'static str {
    "if not exist after_create_attempt.txt (echo first> after_create_attempt.txt && echo retry 1>&2 && exit /b 23) else (echo success> after_create_success.txt)"
}

fn foreign_issue_manifest_json(workspace_path: &std::path::Path, key: &str) -> String {
    serde_json::to_string_pretty(&json!({
        "issue_id": "foreign-id",
        "identifier": "foreign-issue",
        "title": "Foreign issue",
        "current_state": "In Progress",
        "sanitized_workspace_key": key,
        "workspace_path": workspace_path,
        "created_at": "2026-03-21T00:00:00Z",
        "updated_at": "2026-03-21T00:00:00Z"
    }))
    .expect("foreign issue manifest JSON should serialize")
}

#[cfg(unix)]
fn shell_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
fn after_create_bootstrap_failure_command(outside_dir: &std::path::Path) -> String {
    format!(
        "if [ -f after_create_success.txt ]; then echo reran > after_create_reran.txt; exit 41; fi; echo success > after_create_success.txt; ln -s {} .opensymphony",
        shell_quote(outside_dir)
    )
}

#[cfg(unix)]
fn timeout_with_background_child_command() -> &'static str {
    "(sleep 1; echo descendant > .opensymphony/logs/descendant.txt) & echo $! > .opensymphony/logs/descendant.pid; sleep 5"
}

#[tokio::test]
async fn ensure_creates_reuses_workspace_and_runs_after_create_once() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell(
                after_create_requires_empty_workspace_command(),
            )),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-263");

    let first = manager
        .ensure(&issue)
        .await
        .expect("first ensure should succeed");
    let second = manager
        .ensure(&issue)
        .await
        .expect("second ensure should reuse workspace");

    assert!(first.created);
    assert!(!second.created);
    assert_eq!(
        first.handle.workspace_path(),
        second.handle.workspace_path()
    );
    assert!(
        tokio::fs::read_to_string(first.handle.issue_manifest_path())
            .await
            .expect("issue manifest should exist")
            .contains("\"sanitized_workspace_key\": \"COE-263\"")
    );
    assert_eq!(
        tokio::fs::read_to_string(first.handle.workspace_path().join("after_create.txt"))
            .await
            .expect("after_create hook should run before metadata bootstrap")
            .trim(),
        "after_create"
    );

    assert!(
        !tokio::fs::try_exists(
            second
                .handle
                .workspace_path()
                .join("after_create_attempt.txt")
        )
        .await
        .expect("attempt marker lookup should succeed")
    );
}

#[tokio::test]
async fn ensure_retries_after_create_after_failed_first_bootstrap() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell(after_create_retry_command())),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-263-retry");

    let first_error = manager
        .ensure(&issue)
        .await
        .expect_err("first ensure should fail its after_create hook");
    assert!(matches!(
        first_error,
        WorkspaceError::HookFailed {
            hook: HookKind::AfterCreate,
            ..
        }
    ));

    let ensured = manager
        .ensure(&issue)
        .await
        .expect("second ensure should retry after_create and succeed");

    assert!(ensured.created);
    assert_eq!(
        tokio::fs::read_to_string(
            ensured
                .handle
                .workspace_path()
                .join("after_create_success.txt")
        )
        .await
        .expect("after_create should succeed on retry")
        .trim(),
        "success"
    );
    assert!(
        tokio::fs::try_exists(ensured.handle.issue_manifest_path())
            .await
            .expect("issue manifest lookup should succeed")
    );
}

#[tokio::test]
async fn ensure_retries_after_create_when_foreign_issue_manifest_preexists() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell(after_create_retry_command())),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-263-foreign-manifest");

    manager
        .ensure(&issue)
        .await
        .expect_err("first ensure should fail its after_create hook");

    let workspace_path = manager
        .workspace_path_for(&issue.identifier)
        .expect("workspace path should resolve");
    let metadata_dir = workspace_path.join(".opensymphony");
    tokio::fs::create_dir_all(&metadata_dir)
        .await
        .expect("metadata dir should exist");
    tokio::fs::write(
        metadata_dir.join("issue.json"),
        foreign_issue_manifest_json(
            temp_dir.path().join("elsewhere").as_path(),
            "COE-263-foreign-manifest",
        ),
    )
    .await
    .expect("foreign issue manifest should be written");

    let ensured = manager
        .ensure(&issue)
        .await
        .expect("second ensure should retry after_create and succeed");

    assert!(ensured.created);
    assert_eq!(
        tokio::fs::read_to_string(
            ensured
                .handle
                .workspace_path()
                .join("after_create_success.txt")
        )
        .await
        .expect("after_create should succeed on retry")
        .trim(),
        "success"
    );
    assert_eq!(
        manager
            .load_issue_manifest(&ensured.handle)
            .await
            .expect("issue manifest should load")
            .expect("issue manifest should exist")
            .issue_id,
        issue.issue_id
    );
}

#[tokio::test]
async fn ensure_retries_after_create_when_copied_malformed_issue_manifest_preexists() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell(after_create_retry_command())),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-263-malformed-manifest");

    manager
        .ensure(&issue)
        .await
        .expect_err("first ensure should fail its after_create hook");

    let workspace_path = manager
        .workspace_path_for(&issue.identifier)
        .expect("workspace path should resolve");
    let metadata_dir = workspace_path.join(".opensymphony");
    for directory in [
        metadata_dir.clone(),
        metadata_dir.join("logs"),
        metadata_dir.join("generated"),
        metadata_dir.join("openhands"),
        metadata_dir.join("prompts"),
        metadata_dir.join("runs"),
    ] {
        tokio::fs::create_dir_all(&directory)
            .await
            .expect("bootstrap directory should exist");
    }
    tokio::fs::write(metadata_dir.join("issue.json"), "{")
        .await
        .expect("malformed issue manifest should be written");

    let ensured = manager
        .ensure(&issue)
        .await
        .expect("second ensure should retry after_create and succeed");

    assert!(ensured.created);
    assert_eq!(
        tokio::fs::read_to_string(
            ensured
                .handle
                .workspace_path()
                .join("after_create_success.txt")
        )
        .await
        .expect("after_create should succeed on retry")
        .trim(),
        "success"
    );
    assert_eq!(
        manager
            .load_issue_manifest(&ensured.handle)
            .await
            .expect("issue manifest should load")
            .expect("issue manifest should exist")
            .issue_id,
        issue.issue_id
    );
}

#[cfg(unix)]
#[tokio::test]
async fn ensure_does_not_rerun_after_create_after_post_hook_bootstrap_failure() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let outside_dir = temp_dir.path().join("outside");
    tokio::fs::create_dir_all(&outside_dir)
        .await
        .expect("outside dir should exist");

    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell(
                after_create_bootstrap_failure_command(&outside_dir),
            )),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-263-after-create-receipt");

    let first_error = manager
        .ensure(&issue)
        .await
        .expect_err("first ensure should fail after after_create succeeds");
    assert!(matches!(
        first_error,
        WorkspaceError::ManagedPathSymlink { .. }
    ));

    let workspace_path = manager
        .workspace_path_for(&issue.identifier)
        .expect("workspace path should resolve");
    assert!(
        tokio::fs::try_exists(workspace_path.join(".opensymphony.after_create.json"))
            .await
            .expect("after_create receipt lookup should succeed")
    );

    tokio::fs::remove_file(workspace_path.join(".opensymphony"))
        .await
        .expect("symlinked metadata dir should be removable");

    let ensured = manager
        .ensure(&issue)
        .await
        .expect("second ensure should resume bootstrap without rerunning after_create");

    assert!(!ensured.created);
    assert!(
        !tokio::fs::try_exists(workspace_path.join("after_create_reran.txt"))
            .await
            .expect("rerun marker lookup should succeed")
    );
    assert_eq!(
        tokio::fs::read_to_string(workspace_path.join("after_create_success.txt"))
            .await
            .expect("first after_create run marker should exist")
            .trim(),
        "success"
    );
}

#[tokio::test]
async fn ensure_rejects_workspace_reuse_for_colliding_sanitized_key() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let first_issue = sample_issue("feature/42");
    let second_issue = sample_issue("feature:42");

    manager
        .ensure(&first_issue)
        .await
        .expect("first workspace should be created");

    let error = manager
        .ensure(&second_issue)
        .await
        .expect_err("colliding sanitized key should be rejected");

    assert!(matches!(
        error,
        WorkspaceError::WorkspaceOwnershipConflict {
            details,
            ..
        } if details.existing_issue_id == first_issue.issue_id
            && details.requested_issue_id == second_issue.issue_id
    ));
}

#[tokio::test]
async fn start_run_executes_before_run_in_workspace_and_persists_manifest() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_run: Some(HookDefinition::shell(current_dir_command(
                ".opensymphony/logs/before_run_cwd.txt",
            ))),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("feature/42");
    let ensured = manager
        .ensure(&issue)
        .await
        .expect("workspace should exist");

    let run_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-1", 1))
        .await
        .expect("before_run hook should succeed");

    assert_eq!(run_manifest.status, RunStatus::Prepared);
    assert_eq!(run_manifest.hooks.len(), 1);
    assert_eq!(run_manifest.hooks[0].kind, HookKind::BeforeRun);
    assert_eq!(run_manifest.hooks[0].status, HookExecutionStatus::Succeeded);

    let cwd = tokio::fs::read_to_string(ensured.handle.logs_dir().join("before_run_cwd.txt"))
        .await
        .expect("hook should have written cwd");
    let normalized = cwd.trim();
    assert_eq!(
        std::path::Path::new(normalized),
        ensured.handle.workspace_path()
    );

    let persisted = manager
        .load_run_manifest(&ensured.handle)
        .await
        .expect("run manifest read should succeed")
        .expect("run manifest should exist");
    assert_eq!(persisted.status, RunStatus::Prepared);
    assert_eq!(persisted.sanitized_workspace_key, "feature_42");
}

#[tokio::test]
async fn before_run_timeout_is_recorded_and_returned() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_run: Some(HookDefinition::shell(timeout_command())),
            timeout: Duration::from_millis(50),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-timeout"))
        .await
        .expect("workspace should exist");

    let error = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-timeout", 1))
        .await
        .expect_err("timeout should fail required hook");

    assert!(matches!(
        error,
        WorkspaceError::HookTimedOut {
            hook: HookKind::BeforeRun,
            ..
        }
    ));

    let persisted = manager
        .load_run_manifest(&ensured.handle)
        .await
        .expect("run manifest read should succeed")
        .expect("run manifest should exist");
    assert_eq!(persisted.status, RunStatus::PreparationFailed);
    assert_eq!(persisted.hooks.len(), 1);
    assert_eq!(persisted.hooks[0].status, HookExecutionStatus::TimedOut);
}

#[cfg(unix)]
#[tokio::test]
async fn before_run_timeout_kills_spawned_descendants() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_run: Some(HookDefinition::shell(
                timeout_with_background_child_command(),
            )),
            timeout: Duration::from_millis(100),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-timeout-tree"))
        .await
        .expect("workspace should exist");

    let error = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-timeout-tree", 1))
        .await
        .expect_err("timeout should fail required hook");
    assert!(matches!(
        error,
        WorkspaceError::HookTimedOut {
            hook: HookKind::BeforeRun,
            ..
        }
    ));

    tokio::time::sleep(Duration::from_millis(1_500)).await;

    assert!(
        tokio::fs::try_exists(ensured.handle.logs_dir().join("descendant.pid"))
            .await
            .expect("descendant pid lookup should succeed")
    );
    assert!(
        !tokio::fs::try_exists(ensured.handle.logs_dir().join("descendant.txt"))
            .await
            .expect("descendant marker lookup should succeed")
    );
}

#[tokio::test]
async fn before_run_failure_captures_stderr() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_run: Some(HookDefinition::shell(failing_command())),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-failure"))
        .await
        .expect("workspace should exist");

    let error = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-failure", 2))
        .await
        .expect_err("non-zero exit should fail required hook");

    assert!(matches!(
        error,
        WorkspaceError::HookFailed {
            hook: HookKind::BeforeRun,
            ..
        }
    ));

    let persisted = manager
        .load_run_manifest(&ensured.handle)
        .await
        .expect("run manifest read should succeed")
        .expect("run manifest should exist");
    assert_eq!(persisted.status, RunStatus::PreparationFailed);
    assert_eq!(persisted.hooks[0].stderr.trim(), "boom");
    assert_eq!(persisted.hooks[0].exit_code, Some(7));
}

#[tokio::test]
async fn after_run_failure_is_best_effort_and_persisted() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_run: Some(HookDefinition::shell(best_effort_failure_command())),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-after-run"))
        .await
        .expect("workspace should exist");
    let mut run_manifest = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-after", 3))
        .await
        .expect("before_run is not configured");

    manager
        .finish_run(&ensured.handle, &mut run_manifest, RunStatus::Succeeded)
        .await
        .expect("after_run should be best effort");

    let persisted = manager
        .load_run_manifest(&ensured.handle)
        .await
        .expect("run manifest read should succeed")
        .expect("run manifest should exist");
    assert_eq!(persisted.status, RunStatus::Succeeded);
    assert_eq!(persisted.hooks.len(), 1);
    assert_eq!(persisted.hooks[0].kind, HookKind::AfterRun);
    assert_eq!(persisted.hooks[0].status, HookExecutionStatus::Failed);
    assert_eq!(persisted.hooks[0].stderr.trim(), "after-run");
}

#[tokio::test]
async fn cleanup_retains_non_terminal_workspaces() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig {
            remove_terminal_workspaces: true,
        },
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-active"))
        .await
        .expect("workspace should exist");

    let outcome = manager
        .cleanup(&ensured.handle, IssueLifecycleState::Inactive)
        .await
        .expect("non-terminal cleanup should succeed");

    assert_eq!(outcome.decision, CleanupDecision::Retain);
    assert!(
        tokio::fs::metadata(ensured.handle.workspace_path())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn terminal_cleanup_can_run_before_remove_without_deleting_workspace() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_remove: Some(HookDefinition::shell(
                "echo before_remove > .opensymphony/logs/before_remove.txt",
            )),
            ..HookConfig::default()
        },
        CleanupConfig {
            remove_terminal_workspaces: false,
        },
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-terminal-retain"))
        .await
        .expect("workspace should exist");

    let outcome = manager
        .cleanup(&ensured.handle, IssueLifecycleState::Terminal)
        .await
        .expect("terminal cleanup should succeed");

    assert_eq!(outcome.decision, CleanupDecision::Retain);
    assert_eq!(
        tokio::fs::read_to_string(ensured.handle.logs_dir().join("before_remove.txt"))
            .await
            .expect("before_remove should have written marker")
            .trim(),
        "before_remove"
    );
}

#[tokio::test]
async fn terminal_cleanup_can_delete_workspace() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig {
            remove_terminal_workspaces: true,
        },
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-terminal-remove"))
        .await
        .expect("workspace should exist");

    let outcome = manager
        .cleanup(&ensured.handle, IssueLifecycleState::Terminal)
        .await
        .expect("terminal cleanup should succeed");

    assert_eq!(outcome.decision, CleanupDecision::Remove);
    assert!(
        tokio::fs::metadata(ensured.handle.workspace_path())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn hook_cwd_override_cannot_escape_workspace() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_run: Some(HookDefinition::shell("echo nope").with_cwd("../outside")),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-cwd"))
        .await
        .expect("workspace should exist");

    let error = manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-cwd", 1))
        .await
        .expect_err("escaping cwd should fail");

    assert!(matches!(
        error,
        WorkspaceError::HookPathEscape {
            hook: HookKind::BeforeRun,
            ..
        }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn workspace_handle_validation_rejects_symlinked_workspace_roots() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let first = manager
        .ensure(&sample_issue("COE-263-root-symlink-a"))
        .await
        .expect("first workspace should exist");
    let second = manager
        .ensure(&sample_issue("COE-263-root-symlink-b"))
        .await
        .expect("second workspace should exist");

    tokio::fs::remove_dir_all(first.handle.workspace_path())
        .await
        .expect("first workspace should be removable");
    symlink(
        second.handle.workspace_path(),
        first.handle.workspace_path(),
    )
    .expect("workspace root symlink should be created");

    let error = manager
        .start_run(&first.handle, &RunDescriptor::new("run-root-symlink", 1))
        .await
        .expect_err("symlinked workspace root should be rejected");
    assert!(matches!(
        error,
        WorkspaceError::WorkspacePathSymlink { ref path }
            if path == first.handle.workspace_path()
    ));
    assert!(
        !tokio::fs::try_exists(second.handle.run_manifest_path())
            .await
            .expect("run manifest lookup should succeed")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn hook_cwd_override_cannot_escape_workspace_through_symlink() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-symlink"))
        .await
        .expect("workspace should exist");

    let outside_dir = temp_dir.path().join("outside");
    tokio::fs::create_dir_all(&outside_dir)
        .await
        .expect("outside dir should exist");
    symlink(
        &outside_dir,
        ensured.handle.workspace_path().join("link-out"),
    )
    .expect("symlink should be created");

    let escaped_manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            before_run: Some(HookDefinition::shell("pwd").with_cwd("link-out")),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");

    let error = escaped_manager
        .start_run(&ensured.handle, &RunDescriptor::new("run-symlink", 1))
        .await
        .expect_err("symlinked cwd should be rejected");

    assert!(matches!(
        error,
        WorkspaceError::HookPathEscape {
            hook: HookKind::BeforeRun,
            ..
        }
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn managed_manifest_paths_reject_symlinked_reads_and_writes() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-263-metadata-symlink"))
        .await
        .expect("workspace should exist");

    let outside_issue_manifest = temp_dir.path().join("outside-issue.json");
    tokio::fs::write(&outside_issue_manifest, "{}")
        .await
        .expect("outside issue manifest should exist");
    tokio::fs::remove_file(ensured.handle.issue_manifest_path())
        .await
        .expect("managed issue manifest should be removable");
    symlink(
        &outside_issue_manifest,
        ensured.handle.issue_manifest_path(),
    )
    .expect("issue manifest symlink should be created");

    let read_error = manager
        .load_issue_manifest(&ensured.handle)
        .await
        .expect_err("symlinked issue manifest should be rejected");
    assert!(matches!(
        read_error,
        WorkspaceError::ManagedPathSymlink { .. }
    ));

    tokio::fs::remove_file(ensured.handle.issue_manifest_path())
        .await
        .expect("issue manifest symlink should be removable");
    let restored = manager
        .ensure(&sample_issue("COE-263-metadata-symlink"))
        .await
        .expect("workspace should remain reusable");
    manager
        .write_issue_manifest(&ensured.handle, &restored.issue_manifest)
        .await
        .expect("issue manifest should be writable after restoring direct path");

    let outside_run_manifest = temp_dir.path().join("outside-run.json");
    tokio::fs::write(&outside_run_manifest, "{}")
        .await
        .expect("outside run manifest should exist");
    symlink(&outside_run_manifest, ensured.handle.run_manifest_path())
        .expect("run manifest symlink should be created");

    let write_error = manager
        .start_run(
            &ensured.handle,
            &RunDescriptor::new("run-symlinked-manifest", 1),
        )
        .await
        .expect_err("symlinked run manifest should be rejected");
    assert!(matches!(
        write_error,
        WorkspaceError::ManagedPathSymlink { .. }
    ));
}
