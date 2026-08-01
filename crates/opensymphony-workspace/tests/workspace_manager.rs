use std::{collections::BTreeMap, process::Command, time::Duration};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, RepositoryBinding, RepositoryBindingOutcome, RepositoryIdentity,
    SafeRemoteFingerprint,
};
use crate::opensymphony_workspace::{
    CheckoutManifest, CheckoutRepository, CleanupConfig, CleanupDecision, ConversationManifest,
    HookConfig, HookDefinition, HookExecutionRecord, HookExecutionStatus, HookKind,
    IssueContextArtifact, IssueDescriptor, IssueLifecycleState, PromptCaptureDescriptor,
    PromptKind, RunDescriptor, RunManifest, RunStatus, SessionContextArtifact, WorkspaceError,
    WorkspaceManager, WorkspaceManagerConfig, compose_terminal_prompt,
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
        repository_binding: None,
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

fn git(path: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

#[test]
fn terminal_prompt_keeps_repository_instructions_in_one_section() {
    let prompt = compose_terminal_prompt(
        "central policy",
        "issue facts",
        "verified checkout",
        Some("repository-only instruction"),
        "trusted host",
    );

    assert!(prompt.contains("## Central Execution Procedure\n\ncentral policy"));
    assert!(prompt.contains("## Repository Instructions\n\nrepository-only instruction"));
    assert!(!prompt.contains("other repository"));
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
async fn checkout_timeout_does_not_override_legacy_hook_timeout() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig {
            after_create: Some(HookDefinition::shell(timeout_command())),
            timeout: Duration::from_millis(200),
            ..HookConfig::default()
        },
        CleanupConfig::default(),
    ))
    .expect("manager should build");

    let error = manager
        .ensure_with_checkout_timeout(
            &sample_issue("COE-549-hook-timeout"),
            Duration::from_millis(1),
        )
        .await
        .expect_err("legacy hook should use its configured timeout");

    assert!(matches!(
        error,
        WorkspaceError::HookTimedOut {
            hook: HookKind::AfterCreate,
            ..
        }
    ));
}

#[tokio::test]
async fn verified_checkout_is_atomic_repository_local_and_quarantines_drift() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let source = temp_dir.path().join("source");
    let origin = temp_dir.path().join("origin.git");
    std::fs::create_dir_all(&source).expect("source should exist");
    std::fs::create_dir_all(&origin).expect("origin should exist");
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.email", "test@example.invalid"]);
    git(&source, &["config", "user.name", "OpenSymphony Test"]);
    std::fs::write(source.join("AGENTS.md"), "source-only instructions\n")
        .expect("instructions should be written");
    std::fs::create_dir(source.join("configured-dir"))
        .expect("non-file instruction path should be written");
    std::fs::write(source.join("configured-dir/marker"), "not instructions\n")
        .expect("non-file instruction marker should be written");
    std::fs::write(source.join("README.md"), "clean\n").expect("readme should be written");
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "initial"]);
    git(&origin, &["init", "--bare"]);
    git(
        &source,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    git(&source, &["push", "-u", "origin", "main"]);

    let binding = RepositoryBinding {
        alias: "source".to_owned(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::from_remote(
                "local",
                None,
                origin.to_str().expect("origin path"),
            )
            .expect("repository id should be valid"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "local",
                None,
                origin.to_str().expect("origin path"),
            )
            .expect("fingerprint should be valid"),
        },
        config_generation: "config-1".to_owned(),
        inventory_generation: "inventory-1".to_owned(),
    };
    let repository = CheckoutRepository {
        provider: "local".to_owned(),
        provider_id: None,
        remote_locator: origin.to_str().expect("origin path").to_owned(),
        remote: origin.to_str().expect("origin path").to_owned(),
        target_branch: "main".to_owned(),
        credential_kind: "ssh-agent".to_owned(),
        credential_reference: None,
        credential_env: Some("CHECKOUT_SECRET_CANARY".to_owned()),
        instructions_path: "AGENTS.md".into(),
        review_profile: "local".to_owned(),
    };
    let missing_policy_manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("missing-policy manager should be constructed")
    .with_repository_checkouts(BTreeMap::from([(
        "other-repository".to_owned(),
        repository.clone(),
    )]));
    let mut missing_policy_issue = sample_issue("COE-549/missing-policy");
    missing_policy_issue.repository_binding =
        Some(RepositoryBindingOutcome::Resolved(binding.clone()));
    let missing_policy_error = missing_policy_manager
        .ensure_with_run_id(&missing_policy_issue, Some("run-missing-policy"))
        .await
        .expect_err("resolved bindings without policies must not fall back to legacy workspaces");
    assert!(matches!(
        missing_policy_error,
        WorkspaceError::CheckoutVerification { reason, .. }
            if reason == "resolved repository binding has no configured checkout policy"
    ));
    assert!(
        !temp_dir
            .path()
            .join("workspaces/COE-549-missing-policy")
            .exists(),
        "missing checkout policy must not create a legacy workspace"
    );
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build")
    .with_repository_checkouts(BTreeMap::from([(
        binding.repository_id().to_string(),
        repository.clone(),
    )]));
    let mut issue = sample_issue("COE-549/terminal");
    issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(binding.clone()));

    let first = manager
        .ensure_with_run_id(&issue, Some("run-terminal-1"))
        .await
        .expect("checkout should publish");
    assert!(first.created);
    assert!(first.handle.checkout_generation().is_some());
    assert!(first.handle.workspace_path().join("AGENTS.md").is_file());
    assert_eq!(
        manager
            .read_checkout_instructions(&first.handle)
            .await
            .expect("instructions should load")
            .expect("instructions should exist")
            .trim(),
        "source-only instructions"
    );
    let manifest = tokio::fs::read_to_string(first.handle.checkout_manifest_path())
        .await
        .expect("checkout manifest should exist");
    let manifest_record: CheckoutManifest =
        serde_json::from_str(&manifest).expect("checkout manifest should decode");
    assert_eq!(manifest_record.run_id, "run-terminal-1");
    assert_eq!(
        manifest_record.workspace_path,
        first.handle.workspace_path()
    );
    assert!(!manifest.contains("CHECKOUT_SECRET_CANARY"));
    assert!(!manifest.contains(origin.to_str().expect("origin path")));

    let reused = manager.ensure(&issue).await.expect("checkout should reuse");
    assert!(!reused.created);
    assert_eq!(
        reused.handle.workspace_path(),
        first.handle.workspace_path()
    );

    let mut non_file_repository = repository.clone();
    non_file_repository.instructions_path = "configured-dir".into();
    let non_file_manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("non-file manager should build")
    .with_repository_checkouts(BTreeMap::from([(
        binding.repository_id().to_string(),
        non_file_repository,
    )]));
    let mut non_file_issue = sample_issue("COE-549/non-file");
    non_file_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(binding.clone()));
    let error = non_file_manager
        .ensure_with_run_id(&non_file_issue, Some("run-non-file"))
        .await
        .expect_err("configured instruction directories must block publication");
    assert!(matches!(
        error,
        WorkspaceError::CheckoutVerification { reason, .. }
            if reason == "configured instruction path is not a regular file"
    ));

    git(
        first.handle.workspace_path(),
        &[
            "remote",
            "set-url",
            "origin",
            source.to_str().expect("source path"),
        ],
    );
    let repaired = manager
        .ensure(&issue)
        .await
        .expect("wrong remote should quarantine and retry");
    assert!(repaired.created);
    assert_ne!(
        repaired.handle.workspace_path(),
        first.handle.workspace_path()
    );
    assert!(
        temp_dir
            .path()
            .join("workspaces/.opensymphony-quarantine")
            .read_dir()
            .expect("quarantine should exist")
            .next()
            .is_some()
    );

    std::fs::write(
        repaired.handle.workspace_path().join("dirty.txt"),
        "dirty\n",
    )
    .expect("dirty marker should be written");
    let mut prepared_run = RunManifest::new(
        &repaired.handle,
        &RunDescriptor::new("run-prepared-trigger-pending", 1),
    );
    prepared_run.status = RunStatus::Prepared;
    manager
        .write_run_manifest(&repaired.handle, &prepared_run)
        .await
        .expect("prepared run manifest should be written");
    manager
        .write_json_artifact(
            &repaired.handle,
            &repaired.handle.conversation_manifest_path(),
            &json!({
                "active_run_id": "run-prepared-trigger-pending",
                "trigger_pending_run_id": "run-prepared-trigger-pending"
            }),
        )
        .await
        .expect("trigger-pending conversation markers should be written");
    let clean_retry = manager
        .ensure(&issue)
        .await
        .expect("prepared trigger-pending checkout should be retained");
    assert!(!clean_retry.created);
    assert_eq!(
        clean_retry.handle.workspace_path(),
        repaired.handle.workspace_path()
    );
    std::fs::write(
        clean_retry
            .handle
            .workspace_path()
            .join("after-trigger-clear.txt"),
        "worker edit after trigger\n",
    )
    .expect("post-trigger worker edit should be written");
    manager
        .write_json_artifact(
            &clean_retry.handle,
            &clean_retry.handle.conversation_manifest_path(),
            &json!({
                "active_run_id": "run-prepared-trigger-pending",
                "trigger_pending_run_id": null
            }),
        )
        .await
        .expect("cleared trigger marker should be written");
    let retained_after_trigger_clear = manager
        .ensure(&issue)
        .await
        .expect("active prepared checkout should survive cleared trigger marker");
    assert!(!retained_after_trigger_clear.created);
    assert_eq!(
        retained_after_trigger_clear.handle.workspace_path(),
        clean_retry.handle.workspace_path()
    );

    let mut run_manifest = RunManifest::new(
        &clean_retry.handle,
        &RunDescriptor::new("run-feature-branch", 1),
    );
    run_manifest.status = RunStatus::Running;
    manager
        .write_run_manifest(&clean_retry.handle, &run_manifest)
        .await
        .expect("running manifest should be written");
    git(
        clean_retry.handle.workspace_path(),
        &["checkout", "-b", "worker-feature"],
    );
    manager
        .verify_checkout_for_retry(&clean_retry.handle)
        .await
        .expect("worker feature branches should remain attachable");
    git(
        clean_retry.handle.workspace_path(),
        &["checkout", "--detach", "HEAD"],
    );

    git(
        clean_retry.handle.workspace_path(),
        &["checkout", "--orphan", "rewritten"],
    );
    git(clean_retry.handle.workspace_path(), &["rm", "-rf", "."]);
    git(
        clean_retry.handle.workspace_path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(
        clean_retry.handle.workspace_path(),
        &["config", "user.name", "OpenSymphony Test"],
    );
    std::fs::write(
        clean_retry.handle.workspace_path().join("AGENTS.md"),
        "rewritten instructions\n",
    )
    .expect("rewritten instructions should be written");
    git(clean_retry.handle.workspace_path(), &["add", "AGENTS.md"]);
    git(
        clean_retry.handle.workspace_path(),
        &["commit", "-m", "unrelated rewrite"],
    );
    git(
        clean_retry.handle.workspace_path(),
        &["branch", "-M", "main"],
    );
    let ancestry_error = manager
        .verify_checkout_for_retry(&clean_retry.handle)
        .await
        .expect_err("unrelated retained HEAD should be rejected");
    assert!(matches!(
        ancestry_error,
        WorkspaceError::CheckoutVerification { reason, .. }
            if reason.contains("no longer descends")
    ));

    let checkout_manifest_path = clean_retry.handle.checkout_manifest_path();
    let mut tampered_manifest: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(&checkout_manifest_path)
            .await
            .expect("checkout manifest should be readable"),
    )
    .expect("checkout manifest should decode");
    tampered_manifest["target_commit"] = serde_json::Value::String("tampered".to_owned());
    tokio::fs::write(
        &checkout_manifest_path,
        serde_json::to_vec_pretty(&tampered_manifest).expect("tampered manifest should encode"),
    )
    .await
    .expect("tampered manifest should be writable");
    assert!(matches!(
        manager.verify_checkout(&clean_retry.handle).await,
        Err(WorkspaceError::CheckoutVerification { .. })
    ));
    assert!(matches!(
        manager
            .find_verified_workspace_by_issue_reference(&issue.identifier)
            .await,
        Err(WorkspaceError::CheckoutVerification { .. })
    ));

    let renamed_issue = IssueDescriptor {
        issue_id: issue.issue_id.clone(),
        identifier: "COE-549/renamed".to_owned(),
        title: "Issue COE-549/renamed".to_owned(),
        current_state: issue.current_state.clone(),
        last_seen_tracker_refresh_at: None,
        repository_binding: issue.repository_binding.clone(),
    };
    let renamed = manager
        .ensure(&renamed_issue)
        .await
        .expect("renamed issue should publish a new generation");
    assert!(renamed.created);
    assert_ne!(
        renamed.handle.workspace_path(),
        clean_retry.handle.workspace_path()
    );

    assert!(!clean_retry.handle.workspace_path().exists());
    assert!(
        temp_dir
            .path()
            .join("workspaces/.opensymphony-quarantine")
            .read_dir()
            .expect("quarantine should exist")
            .next()
            .is_some()
    );
}

#[tokio::test]
async fn legacy_workspace_lookup_skips_malformed_generation_manifests() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-549-malformed-generation");
    let ensured = manager
        .ensure(&issue)
        .await
        .expect("legacy workspace should exist");

    let malformed_path = workspace_root.join("malformed-generation");
    tokio::fs::create_dir_all(malformed_path.join(".opensymphony"))
        .await
        .expect("malformed generation directory should exist");
    let mut issue_manifest: serde_json::Value = serde_json::from_str(
        &tokio::fs::read_to_string(ensured.handle.issue_manifest_path())
            .await
            .expect("issue manifest should be readable"),
    )
    .expect("issue manifest should decode");
    issue_manifest["sanitized_workspace_key"] =
        serde_json::Value::String("malformed-generation".to_owned());
    issue_manifest["workspace_path"] =
        serde_json::Value::String(malformed_path.display().to_string());
    tokio::fs::write(
        malformed_path.join(".opensymphony/issue.json"),
        serde_json::to_vec_pretty(&issue_manifest).expect("issue manifest should encode"),
    )
    .await
    .expect("malformed issue manifest should be written");
    tokio::fs::write(
        malformed_path.join(".opensymphony/checkout.json"),
        b"not-json",
    )
    .await
    .expect("malformed checkout manifest should be written");

    let malformed_issue_path = workspace_root.join("malformed-issue-generation");
    tokio::fs::create_dir_all(malformed_issue_path.join(".opensymphony"))
        .await
        .expect("malformed issue generation directory should exist");
    tokio::fs::write(
        malformed_issue_path.join(".opensymphony/issue.json"),
        b"not-json",
    )
    .await
    .expect("malformed issue manifest should be written");
    tokio::fs::write(
        malformed_issue_path.join(".opensymphony/checkout.json"),
        b"{}",
    )
    .await
    .expect("strict generation marker should be written");

    let missing_checkout_path =
        workspace_root.join("malformed-generation--missing-checkout-manifest");
    tokio::fs::create_dir_all(missing_checkout_path.join(".opensymphony"))
        .await
        .expect("missing checkout manifest directory should exist");
    let mut missing_checkout_issue = issue_manifest.clone();
    missing_checkout_issue["sanitized_workspace_key"] =
        serde_json::Value::String("malformed-generation".to_owned());
    missing_checkout_issue["workspace_path"] =
        serde_json::Value::String(missing_checkout_path.display().to_string());
    tokio::fs::write(
        missing_checkout_path.join(".opensymphony/issue.json"),
        serde_json::to_vec_pretty(&missing_checkout_issue)
            .expect("missing checkout issue manifest should encode"),
    )
    .await
    .expect("missing checkout issue manifest should be written");

    let missing_issue_path = workspace_root.join("missing-issue-generation");
    tokio::fs::create_dir_all(missing_issue_path.join(".opensymphony"))
        .await
        .expect("missing issue manifest directory should exist");
    tokio::fs::write(
        missing_issue_path.join(".opensymphony/checkout.json"),
        b"{}",
    )
    .await
    .expect("checkout metadata without an issue manifest should be written");

    let found = manager
        .find_workspace_by_issue_reference(&issue.issue_id)
        .await
        .expect("malformed generations should not abort legacy lookup")
        .expect("legacy workspace should still be found");
    assert_eq!(found.workspace_path(), ensured.handle.workspace_path());
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
async fn find_workspace_by_issue_reference_returns_identifier_match() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-287"))
        .await
        .expect("workspace should exist");

    let found = manager
        .find_workspace_by_issue_reference("COE-287")
        .await
        .expect("lookup should succeed")
        .expect("workspace should be found");

    assert_eq!(found.issue_id(), ensured.handle.issue_id());
    assert_eq!(found.identifier(), ensured.handle.identifier());
    assert_eq!(found.workspace_path(), ensured.handle.workspace_path());
}

#[tokio::test]
async fn find_workspace_by_issue_reference_scans_issue_ids() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let issue = sample_issue("COE-288");
    let ensured = manager
        .ensure(&issue)
        .await
        .expect("workspace should exist");

    let found = manager
        .find_workspace_by_issue_reference(&issue.issue_id)
        .await
        .expect("lookup should succeed")
        .expect("workspace should be found");

    assert_eq!(found.issue_id(), ensured.handle.issue_id());
    assert_eq!(found.identifier(), ensured.handle.identifier());
    assert_eq!(found.workspace_path(), ensured.handle.workspace_path());
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
    let mut issue = sample_issue("COE-263-after-create-receipt");
    issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(RepositoryBinding {
        alias: "core".to_string(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::new("github:repository:core")
                .expect("repository id should be valid"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("core"),
                "owner/repository",
            )
            .expect("fingerprint should be valid"),
        },
        config_generation: "config-1".to_string(),
        inventory_generation: "inventory-1".to_string(),
    }));

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
    let receipt = tokio::fs::read_to_string(workspace_path.join(".opensymphony.after_create.json"))
        .await
        .expect("after_create receipt should be readable");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&receipt).expect("receipt should be valid JSON")
            ["repository_binding"]["repository"]["id"],
        "github:repository:core"
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
        .start_run(
            &ensured.handle,
            &RunDescriptor::new("run-1", 1).with_normal_retry_count(2),
        )
        .await
        .expect("before_run hook should succeed");

    assert_eq!(run_manifest.status, RunStatus::Prepared);
    assert_eq!(run_manifest.normal_retry_count, 2);
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
    assert_eq!(persisted.normal_retry_count, 2);
    assert_eq!(persisted.sanitized_workspace_key, "feature_42");
}

#[tokio::test]
async fn repository_binding_is_persisted_before_and_during_a_run_claim() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let binding = RepositoryBinding {
        alias: "core".to_string(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::new("github:repository:42").expect("repository id"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("42"),
                "owner/repository",
            )
            .expect("fingerprint"),
        },
        config_generation: "config-1".to_string(),
        inventory_generation: "inventory-1".to_string(),
    };
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build")
    .with_legacy_repository(Some(binding.repository.id.clone()));
    let mut issue = sample_issue("COE-548");
    issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(binding.clone()));
    let ensured = manager
        .ensure(&issue)
        .await
        .expect("workspace should exist");
    assert_eq!(
        ensured.issue_manifest.repository_binding,
        Some(RepositoryBindingOutcome::Resolved(binding.clone()))
    );

    let manifest = manager
        .start_run(
            &ensured.handle,
            &RunDescriptor::new("run-binding", 1).with_repository_binding(Some(binding.clone())),
        )
        .await
        .expect("run manifest should be written");
    assert_eq!(manifest.repository_binding, Some(binding));
}

#[tokio::test]
async fn existing_workspace_rejects_a_changed_repository_identity() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let binding = |alias: &str, id: &str| RepositoryBinding {
        alias: alias.to_string(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::new(id).expect("repository id"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some(id),
                "owner/repository",
            )
            .expect("fingerprint"),
        },
        config_generation: "config-1".to_string(),
        inventory_generation: "inventory-1".to_string(),
    };
    let first_binding = binding("core", "github:repository:core");
    let second_binding = binding("web", "github:repository:web");
    let mut first_issue = sample_issue("COE-548-rebind");
    first_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(first_binding));
    manager
        .ensure(&first_issue)
        .await
        .expect("initial workspace should exist");

    let mut changed_issue = first_issue;
    changed_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(second_binding));
    let error = manager
        .ensure(&changed_issue)
        .await
        .expect_err("changed repository identity must not reuse the old workspace");

    assert!(matches!(
        error,
        WorkspaceError::RepositoryBindingMismatch { .. }
    ));
}

#[tokio::test]
async fn legacy_workspace_backfills_a_new_repository_identity() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let binding = RepositoryBinding {
        alias: "core".to_string(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::new("github:repository:core").expect("repository id"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("core"),
                "owner/repository",
            )
            .expect("fingerprint"),
        },
        config_generation: "config-1".to_string(),
        inventory_generation: "inventory-1".to_string(),
    };
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build")
    .with_legacy_repository(Some(binding.repository.id.clone()));
    let legacy_issue = sample_issue("COE-548-legacy");
    let legacy_workspace = manager
        .ensure(&legacy_issue)
        .await
        .expect("legacy workspace should exist");
    manager
        .start_run(
            &legacy_workspace.handle,
            &RunDescriptor::new("legacy-proof", 1).with_repository_binding(Some(binding.clone())),
        )
        .await
        .expect("legacy run should persist repository proof");

    let mut upgraded_issue = legacy_issue;
    upgraded_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(binding.clone()));
    let ensured = manager
        .ensure(&upgraded_issue)
        .await
        .expect("legacy workspace should accept a safe repository backfill");

    assert_eq!(
        ensured.issue_manifest.repository_binding,
        Some(RepositoryBindingOutcome::Resolved(binding))
    );
}

#[tokio::test]
async fn legacy_workspace_rejects_unproven_repository_backfill() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let binding = RepositoryBinding {
        alias: "core".to_string(),
        repository: RepositoryIdentity {
            id: CanonicalRepositoryId::new("github:repository:core").expect("repository id"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("core"),
                "owner/repository",
            )
            .expect("fingerprint"),
        },
        config_generation: "config-1".to_string(),
        inventory_generation: "inventory-1".to_string(),
    };
    let legacy_issue = sample_issue("COE-548-unproven-legacy");
    manager
        .ensure(&legacy_issue)
        .await
        .expect("legacy workspace should exist");

    let mut upgraded_issue = legacy_issue;
    upgraded_issue.repository_binding = Some(RepositoryBindingOutcome::Resolved(binding));
    let error = manager
        .ensure(&upgraded_issue)
        .await
        .expect_err("unproven legacy repository must not be backfilled");

    assert!(matches!(
        error,
        WorkspaceError::RepositoryBindingMismatch { .. }
    ));
}

#[tokio::test]
async fn run_manifest_redacts_hook_credentials_before_persisting() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let manager = WorkspaceManager::new(manager_config(
        &temp_dir.path().join("workspaces"),
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-547-hook-redaction"))
        .await
        .expect("workspace should exist");
    let mut manifest = manager
        .start_run(
            &ensured.handle,
            &RunDescriptor::new("run-hook-redaction", 1),
        )
        .await
        .expect("run should start");
    let now = chrono::Utc::now();
    manifest.hooks.push(HookExecutionRecord {
        kind: HookKind::BeforeRun,
        command: "echo access_token=sk-live-hook".to_string(),
        cwd: ensured.handle.workspace_path().to_path_buf(),
        best_effort: false,
        status: HookExecutionStatus::Succeeded,
        started_at: now,
        finished_at: now,
        duration_ms: 1,
        exit_code: Some(0),
        stdout: "{\"account_id\":\"acct_hook\"}".to_string(),
        stderr: "refresh_token: rt-hook".to_string(),
    });
    manager
        .write_run_manifest(&ensured.handle, &manifest)
        .await
        .expect("manifest should persist");

    let persisted = manager
        .load_run_manifest(&ensured.handle)
        .await
        .expect("run manifest should load")
        .expect("run manifest should exist");
    let hook = persisted.hooks.last().expect("hook should persist");
    for value in [&hook.command, &hook.stdout, &hook.stderr] {
        assert!(!value.contains("sk-live-hook"));
        assert!(!value.contains("acct_hook"));
        assert!(!value.contains("rt-hook"));
    }
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
            timeout: Duration::from_millis(500),
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
async fn conversation_manifest_artifacts_round_trip_inside_workspace() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-266-conversation-artifacts"))
        .await
        .expect("workspace should exist");

    manager
        .write_json_artifact(
            &ensured.handle,
            &ensured.handle.conversation_manifest_path(),
            &json!({
                "conversation_id": "conv_266",
                "workflow_prompt_seeded": true,
            }),
        )
        .await
        .expect("conversation manifest artifact should be writable");
    manager
        .write_text_artifact(
            &ensured.handle,
            &ensured.handle.prompts_dir().join("last-full-prompt.md"),
            "Ticket COE-266",
        )
        .await
        .expect("prompt artifact should be writable");

    let conversation_manifest = manager
        .read_text_artifact(
            &ensured.handle,
            &ensured.handle.conversation_manifest_path(),
        )
        .await
        .expect("conversation manifest artifact should be readable")
        .expect("conversation manifest artifact should exist");
    let prompt = manager
        .read_text_artifact(
            &ensured.handle,
            &ensured.handle.prompts_dir().join("last-full-prompt.md"),
        )
        .await
        .expect("prompt artifact should be readable")
        .expect("prompt artifact should exist");

    assert!(conversation_manifest.contains("\"conversation_id\": \"conv_266\""));
    assert_eq!(prompt, "Ticket COE-266");
}

#[tokio::test]
async fn conversation_manifest_and_generated_context_artifacts_are_persisted() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-270-artifacts"))
        .await
        .expect("workspace should exist");

    let mut conversation = ConversationManifest::new(
        &ensured.handle,
        "conv-270",
        "http://127.0.0.1:8000",
        ensured.handle.openhands_dir(),
        "agent-server-v1",
    );
    conversation.last_attached_at = Some(chrono::Utc::now());
    conversation.fresh_conversation = false;
    manager
        .write_conversation_manifest(&ensured.handle, &conversation)
        .await
        .expect("conversation manifest should write");

    let loaded = manager
        .load_conversation_manifest(&ensured.handle)
        .await
        .expect("conversation manifest should load")
        .expect("conversation manifest should exist");
    assert_eq!(loaded, conversation);

    let issue_context = IssueContextArtifact {
        issue_id: ensured.handle.issue_id().to_string(),
        identifier: ensured.handle.identifier().to_string(),
        title: "Repository harness and generated context artifacts".to_string(),
        current_state: "In Progress".to_string(),
        repo_workflow_path: ensured.handle.workspace_path().join("WORKFLOW.md"),
        repo_agents_path: Some(ensured.handle.workspace_path().join("AGENTS.md")),
        repo_skills_dir: Some(ensured.handle.workspace_path().join(".agents/skills")),
        last_run_status: Some(RunStatus::Prepared),
        important_constraints: vec![
            "Repo-owned policy remains authoritative.".to_string(),
            "Generated artifacts stay under .opensymphony/.".to_string(),
        ],
        known_blockers: vec!["None".to_string()],
    };
    manager
        .write_issue_context(&ensured.handle, &issue_context)
        .await
        .expect("issue context should write");

    let rendered_issue_context = tokio::fs::read_to_string(ensured.handle.issue_context_path())
        .await
        .expect("issue context should exist");
    assert!(rendered_issue_context.contains("Repository-owned policy remains authoritative."));
    assert!(rendered_issue_context.contains("WORKFLOW.md"));
    assert!(rendered_issue_context.contains(".agents/skills/"));
    assert!(rendered_issue_context.contains(".opensymphony/conversation.json"));

    let mut session_context = SessionContextArtifact::new(&ensured.handle);
    session_context.conversation_id = Some("conv-270".to_string());
    session_context.attempt = Some(4);
    session_context.last_run_id = Some("run-4".to_string());
    session_context.last_run_status = Some(RunStatus::Succeeded);
    session_context.last_prompt_kind = Some(PromptKind::Continuation);
    session_context.last_prompt_path =
        Some(ensured.handle.latest_prompt_path(PromptKind::Continuation));
    session_context.recent_validation_commands =
        vec!["cargo test -p opensymphony-workspace".to_string()];
    manager
        .write_session_context(&ensured.handle, &session_context)
        .await
        .expect("session context should write");

    let loaded_session = manager
        .load_session_context(&ensured.handle)
        .await
        .expect("session context should load")
        .expect("session context should exist");
    assert_eq!(loaded_session, session_context);
}

#[tokio::test]
async fn prompt_capture_writes_latest_and_per_run_artifacts() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-270-prompts"))
        .await
        .expect("workspace should exist");
    let run = RunDescriptor::new("run-17", 17);

    let first = manager
        .write_prompt_capture(
            &ensured.handle,
            &run,
            PromptCaptureDescriptor::new(PromptKind::Full, 1),
            "Initial full prompt",
        )
        .await
        .expect("first prompt capture should write");
    let second = manager
        .write_prompt_capture(
            &ensured.handle,
            &run,
            PromptCaptureDescriptor::new(PromptKind::Full, 2),
            "Updated full prompt",
        )
        .await
        .expect("second prompt capture should write");
    manager
        .write_prompt_capture(
            &ensured.handle,
            &run,
            PromptCaptureDescriptor::new(PromptKind::Continuation, 1),
            "Continuation guidance",
        )
        .await
        .expect("continuation prompt capture should write");

    assert_eq!(
        tokio::fs::read_to_string(first.archived_prompt_path.clone())
            .await
            .expect("archived first prompt should exist"),
        "Initial full prompt"
    );
    assert_eq!(
        tokio::fs::read_to_string(second.archived_prompt_path.clone())
            .await
            .expect("archived second prompt should exist"),
        "Updated full prompt"
    );
    assert_eq!(
        tokio::fs::read_to_string(ensured.handle.latest_prompt_path(PromptKind::Full))
            .await
            .expect("latest full prompt should exist"),
        "Updated full prompt"
    );
    assert_eq!(
        tokio::fs::read_to_string(ensured.handle.latest_prompt_path(PromptKind::Continuation))
            .await
            .expect("latest continuation prompt should exist"),
        "Continuation guidance"
    );

    let latest_full_manifest = serde_json::from_str::<
        crate::opensymphony_workspace::PromptCaptureManifest,
    >(
        &tokio::fs::read_to_string(ensured.handle.latest_prompt_manifest_path(PromptKind::Full))
            .await
            .expect("latest full prompt manifest should exist"),
    )
    .expect("latest full prompt manifest should decode");
    assert_eq!(latest_full_manifest.sequence, 2);
    assert_eq!(latest_full_manifest.attempt, 17);
    assert_eq!(latest_full_manifest.prompt_kind, PromptKind::Full);

    let archived_run_manifest =
        serde_json::from_str::<crate::opensymphony_workspace::PromptCaptureManifest>(
            &tokio::fs::read_to_string(ensured.handle.run_prompt_manifest_path(
                17,
                PromptKind::Continuation,
                1,
            ))
            .await
            .expect("archived continuation prompt manifest should exist"),
        )
        .expect("archived continuation prompt manifest should decode");
    assert_eq!(archived_run_manifest.run_id, "run-17");
    assert_eq!(archived_run_manifest.sequence, 1);
    assert_eq!(
        archived_run_manifest.prompt_length_bytes,
        "Continuation guidance".len()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn generated_issue_context_rejects_symlinked_output_paths() {
    let temp_dir = TempDir::new().expect("temp dir should exist");
    let workspace_root = temp_dir.path().join("workspaces");
    let manager = WorkspaceManager::new(manager_config(
        &workspace_root,
        HookConfig::default(),
        CleanupConfig::default(),
    ))
    .expect("manager should build");
    let ensured = manager
        .ensure(&sample_issue("COE-270-generated-symlink"))
        .await
        .expect("workspace should exist");

    let outside_issue_context = temp_dir.path().join("outside-issue-context.md");
    tokio::fs::write(&outside_issue_context, "outside")
        .await
        .expect("outside issue context should exist");
    symlink(&outside_issue_context, ensured.handle.issue_context_path())
        .expect("issue context symlink should be created");

    let error = manager
        .write_issue_context(
            &ensured.handle,
            &IssueContextArtifact {
                issue_id: ensured.handle.issue_id().to_string(),
                identifier: ensured.handle.identifier().to_string(),
                title: "Symlink rejection".to_string(),
                current_state: "In Progress".to_string(),
                repo_workflow_path: ensured.handle.workspace_path().join("WORKFLOW.md"),
                repo_agents_path: None,
                repo_skills_dir: None,
                last_run_status: None,
                important_constraints: Vec::new(),
                known_blockers: Vec::new(),
            },
        )
        .await
        .expect_err("symlinked issue context should be rejected");

    assert!(matches!(error, WorkspaceError::ManagedPathSymlink { .. }));
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

    let outside_conversation_manifest = temp_dir.path().join("outside-conversation.json");
    tokio::fs::write(&outside_conversation_manifest, "{}")
        .await
        .expect("outside conversation manifest should exist");
    symlink(
        &outside_conversation_manifest,
        ensured.handle.conversation_manifest_path(),
    )
    .expect("conversation manifest symlink should be created");

    let conversation_read_error = manager
        .read_text_artifact(
            &ensured.handle,
            &ensured.handle.conversation_manifest_path(),
        )
        .await
        .expect_err("symlinked conversation manifest should be rejected");
    assert!(matches!(
        conversation_read_error,
        WorkspaceError::ManagedPathSymlink { .. }
    ));

    let outside_prompt = temp_dir.path().join("outside-prompt.md");
    tokio::fs::write(&outside_prompt, "outside")
        .await
        .expect("outside prompt should exist");
    let prompt_path = ensured.handle.prompts_dir().join("last-full-prompt.md");
    symlink(&outside_prompt, &prompt_path).expect("prompt symlink should be created");

    let prompt_write_error = manager
        .write_text_artifact(&ensured.handle, &prompt_path, "prompt")
        .await
        .expect_err("symlinked prompt artifact should be rejected");
    assert!(matches!(
        prompt_write_error,
        WorkspaceError::ManagedPathSymlink { .. }
    ));
}
