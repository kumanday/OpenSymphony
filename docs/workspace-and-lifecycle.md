# Workspace and Lifecycle

## 1. Goal

Preserve the Symphony workspace contract while adapting it to OpenHands conversation persistence and local MVP safety constraints.

## 2. Workspace mapping

Each issue maps to exactly one workspace path:

```text
<workspace.root>/<sanitized_issue_identifier>
```

Sanitization rule:

- keep `[A-Za-z0-9._-]`
- replace every other character with `_`

Examples:

- `ABC-123` -> `ABC-123`
- `feature/42` -> `feature_42`
- `Bug: weird path` -> `Bug__weird_path`

Because this sanitization is not injective, workspace reuse must be gated by the persisted issue manifest for the current path. If an existing current-path manifest claims the same sanitized key for a different issue, OpenSymphony must refuse reuse instead of silently aliasing two issues onto one workspace.

## 3. Hard safety invariants

- The resolved workspace path must stay under `workspace.root`.
- The issue workspace path itself must not be a symlink when OpenSymphony reuses or validates it.
- `cwd` for all hook commands and all OpenHands runs must equal the resolved issue workspace path unless an explicit per-command `cwd` override inside the same workspace is required.
- When `openhands.local_server.command` is omitted, the runtime-owned local tooling layer must resolve the pinned launcher from the OpenSymphony checkout before that `cwd` switch happens. Workflow resolution must not bake a compile-time checkout path into config defaults.
- Explicit workflow-owned `openhands.local_server.command` overrides are currently rejected until the runtime supervisor can honor them instead of always launching the pinned repo-local server wrapper.
- OpenSymphony must never run agent work directly in `workspace.root`.
- Path checks must operate on canonicalized paths when possible.

## 4. Workspace directory layout

Recommended layout inside each issue workspace:

```text
<issue_workspace>/
  .opensymphony.after_create.json
  .opensymphony/
    issue.json
    run.json
    conversation.json
    prompts/
      last-full-prompt.md
      last-continuation-prompt.md
    logs/
      worker.log
      hook.log
    openhands/
      create-conversation-request.json
      last-conversation-state.json
    generated/
      issue-context.md
      session-context.json
```

Notes:

- `.opensymphony/` is OpenSymphony-owned metadata.
- `.opensymphony.after_create.json` is an internal OpenSymphony bootstrap receipt written at the workspace root immediately after a successful first-time `after_create` hook and before `.opensymphony/` metadata bootstrap.
- The workspace layer bootstraps `issue.json`, `run.json`, and the supporting metadata directories after a successful first-time `after_create` hook so clone/worktree hooks still see a fresh workspace directory.
- `conversation.json` remains reserved for the OpenHands session layer even though the workspace handle exposes its deterministic path.
- The repository working tree remains otherwise untouched except by normal agent work.
- OpenSymphony must never overwrite repository-owned `AGENTS.md`.

## 5. Workspace ownership model

The workspace manager owns:

- workspace path resolution
- existence checks
- creation
- optional initial population through hooks
- metadata bootstrap
- cleanup

The OpenHands runtime owns only the conversation execution inside that path.

## 6. Lifecycle hooks

Preserve the Symphony hook model.

## 6.1 `after_create`

Runs once after a brand-new issue workspace is created.

On first bootstrap, this hook runs before OpenSymphony creates `.opensymphony/` so repository bootstrap commands such as `git clone <repo> .` or `git worktree add <path>` can target an otherwise empty workspace directory.

Use for:

- cloning the repo
- adding a git worktree
- bootstrapping local tooling
- creating ignored helper files

Do not rerun it on every worker attempt.

If the first `after_create` attempt fails before bootstrap completes, the next `ensure` attempt should retry `after_create` instead of treating the partially initialized workspace directory as fully reusable.

After a successful first-time `after_create`, OpenSymphony must persist a root-scoped bootstrap receipt before it starts creating `.opensymphony/` metadata. If later bootstrap steps fail, the next `ensure` should resume metadata bootstrap without rerunning `after_create`.

Steady-state workspace ownership is still determined by a decodable OpenSymphony-owned `issue.json` whose workspace path and sanitized key match the current workspace, not by raw file existence. Repository-provided, copied, or undecodable `.opensymphony/issue.json` or `.opensymphony.after_create.json` artifacts must not suppress a required first-bootstrap retry.

## 6.2 `before_run`

Runs before each worker lifetime.

Use for:

- syncing or fetching changes
- checking workspace health
- generating run-scoped metadata
- recording diagnostic info

## 6.3 `after_run`

Runs after each worker lifetime regardless of outcome, best effort.

Use for:

- capturing status
- collecting logs
- updating generated context artifacts
- cleaning temporary files

## 6.4 `before_remove`

Runs before workspace deletion for terminal issues.

Use for:

- final log or artifact collection
- archiving evidence
- safe cleanup steps

## 6.5 Hook execution rules

- Hooks execute inside the issue workspace unless explicitly documented otherwise.
- Workspace-handle validation must reject symlinked workspace roots before hook execution, cleanup, or manifest I/O can proceed.
- Any explicit hook `cwd` override must still resolve inside the same issue workspace.
- Containment checks for explicit hook `cwd` overrides should use canonicalized paths so symlinked subdirectories cannot escape the workspace.
- OpenSymphony-managed metadata paths under `.opensymphony/` must reject symlinked directories or files before any manifest read or write.
- Unix hook commands should run via a non-login `sh -c` shell so host profile startup files cannot change `cwd` or fail the hook before the configured command runs.
- Hook timeouts use the configured `hooks.timeout_ms`.
- When a hook times out, OpenSymphony must terminate the entire spawned process tree rather than only the direct shell wrapper process.
- Hook failures are categorized and surfaced with issue context.
- `after_run` and `before_remove` are best effort by default.
- `after_create` and `before_run` failures fail the current worker attempt.

## 7. Issue metadata manifest

Persist a small issue manifest under `.opensymphony/issue.json`.

Suggested fields:

- `issue_id`
- `identifier`
- `title`
- `current_state`
- `sanitized_workspace_key`
- `workspace_path`
- `created_at`
- `updated_at`
- `last_seen_tracker_refresh_at`

Use cases:

- restart recovery
- operator debugging
- workspace introspection
- authoritative ownership check for non-injective sanitized workspace keys

## 7.1 Run metadata manifest

Persist the latest worker-lifetime manifest under `.opensymphony/run.json`.

Suggested fields:

- `run_id`
- `attempt`
- `issue_id`
- `identifier`
- `sanitized_workspace_key`
- `workspace_path`
- `status`
- `status_detail`
- `hooks`
- `created_at`
- `updated_at`

Use cases:

- capture `before_run` and `after_run` hook outcomes with stdout/stderr for diagnostics
- explain the latest worker-lifetime state during restart recovery
- make cleanup and retry decisions inspectable without daemon memory

## 8. Conversation metadata manifest

Persist `.opensymphony/conversation.json`.

Suggested fields:

- `issue_id`
- `identifier`
- `conversation_id`
- `server_base_url`
- `persistence_dir`
- `created_at`
- `last_attached_at`
- `fresh_conversation`
- `reset_reason`
- `runtime_contract_version`

This file is the bridge between Symphony issue ownership and OpenHands conversation reuse.

## 9. Generated context artifacts

OpenSymphony should generate additive helper files under `.opensymphony/generated/`.

Recommended files:

### `issue-context.md`

Human-readable summary for the agent and operator:

- issue identifier and title
- current state
- last worker outcome
- important constraints
- known blockers
- location of OpenSymphony metadata files

### `session-context.json`

Machine-readable runtime summary:

- conversation ID
- attempt number
- last worker timestamps
- last known execution status
- recent validation commands
- last retry reason if any

These files help continuity without altering the repository's own guidance files.

## 10. Prompt artifacts

Persist the last prompts sent by OpenSymphony.

Why:

- debugging render issues
- replaying a failed run
- comparing full vs continuation prompt logic
- making live tests and regressions easier to inspect

Store at minimum:

- last full prompt
- last continuation prompt
- timestamp metadata

## 11. Conversation lifetime policy inside the workspace

Default policy:

- one conversation per issue
- conversation persistence is stored under the issue workspace
- reused across worker lifetimes
- reset only on explicit error or incompatible-version policy

Reset handling:

- archive old metadata if useful
- write a reset reason
- resend full prompt on the next fresh run

## 12. Clean exit and continuation

Symphony requires a short continuation retry after normal worker exit.

OpenSymphony implementation:

- worker may already have run multiple in-process turns on the same conversation
- when the worker finally exits cleanly, the orchestrator schedules the short retry
- the next worker reattaches to the same workspace and usually the same conversation
- because the conversation already contains the original assignment, the next worker sends continuation guidance instead of replaying the full prompt

## 13. Cleanup policy

## 13.1 Terminal issues

When the tracker says an issue is terminal:

- cancel any active worker
- run `before_remove` best effort
- delete the workspace if configured to do so

Keep cleanup policy configurable enough to allow retention during debugging.

## 13.2 Non-active, non-terminal issues

When the tracker says the issue is not active and not terminal:

- cancel active work
- do not delete the workspace by default
- preserve metadata and conversation state for possible later reactivation

## 13.3 Startup sweep

On daemon startup, optionally sweep known workspaces against terminal tracker states and remove those that no longer need to exist.

## 14. Local MVP safety posture

The local MVP assumes host access.

Implications:

- hook commands run on the host
- OpenHands tool execution may run on the host
- workspace root selection matters
- docs must strongly recommend trusted repositories only

A future hosted mode can keep the same workspace ownership model while moving actual execution into a remote or container-backed environment.

## 15. Suggested implementation API

```rust
trait WorkspaceManager {
    fn workspace_path_for(&self, issue_identifier: &str) -> Result<PathBuf>;
    async fn ensure(&self, issue: &IssueDescriptor) -> Result<EnsureWorkspaceResult>;
    async fn start_run(
        &self,
        workspace: &WorkspaceHandle,
        run: &RunDescriptor,
    ) -> Result<RunManifest>;
    async fn finish_run(
        &self,
        workspace: &WorkspaceHandle,
        run: &mut RunManifest,
        status: RunStatus,
    ) -> Result<()>;
    async fn cleanup(
        &self,
        workspace: &WorkspaceHandle,
        state: IssueLifecycleState,
    ) -> Result<CleanupOutcome>;
}
```

`WorkspaceHandle` should expose:

- `issue_id`
- `identifier`
- `workspace_path`
- `metadata_dir`
- `issue_manifest_path`
- `run_manifest_path`
- `conversation_manifest_path`

## 16. Tests required

- sanitize identifier edge cases
- canonical path containment
- create vs reuse
- `after_create` only once
- clone/worktree-compatible fresh bootstrap before `.opensymphony/` exists
- retry `after_create` after a failed first bootstrap
- `before_run` every worker lifetime
- timeout on hook
- hook stderr capture
- canonical `cwd` containment for symlinked subdirectories
- terminal cleanup
- issue and run metadata file write and reload
- conversation reset path preserves workspace safety
