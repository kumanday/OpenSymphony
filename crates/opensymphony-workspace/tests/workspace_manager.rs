use std::time::Duration;

use opensymphony_workspace::{
    CleanupConfig, CleanupDecision, HookConfig, HookDefinition, HookExecutionStatus, HookKind,
    IssueDescriptor, IssueLifecycleState, RunDescriptor, RunStatus, WorkspaceError,
    WorkspaceManager, WorkspaceManagerConfig,
};
use tempfile::TempDir;

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

#[tokio::test]
async fn ensure_creates_reuses_workspace_and_runs_after_create_once() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig {
            after_create: Some(HookDefinition::shell(
                "echo after_create >> .opensymphony/logs/after_create-count.txt",
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

    let after_create_log =
        tokio::fs::read_to_string(first.handle.logs_dir().join("after_create-count.txt"))
            .await
            .expect("after_create hook should have written a marker");
    let run_count = after_create_log.lines().count();
    assert_eq!(run_count, 1);
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
    assert!(tokio::fs::metadata(ensured.handle.workspace_path())
        .await
        .is_ok());
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
    assert!(tokio::fs::metadata(ensured.handle.workspace_path())
        .await
        .is_err());
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
