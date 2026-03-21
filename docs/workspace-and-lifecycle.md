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

## 3. Hard safety invariants

- The resolved workspace path must stay under `workspace.root`.
- `cwd` for all hook commands and all OpenHands runs must equal the resolved issue workspace path unless an explicit per-command `cwd` override inside the same workspace is required.
- OpenSymphony must never run agent work directly in `workspace.root`.
- Path checks must operate on canonicalized paths when possible.

## 4. Workspace directory layout

Recommended layout inside each issue workspace:

```text
<issue_workspace>/
  .opensymphony/
    bootstrap.ok
    issue.json
    conversation.json
    retry.json
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

Runs once after a workspace has been bootstrapped successfully.

Use for:

- cloning the repo
- adding a git worktree
- bootstrapping local tooling
- creating ignored helper files

Do not rerun it on every worker attempt.
If the first bootstrap attempt fails after the directory already exists, the next attempt must rerun `after_create` until bootstrap completes. A successful bootstrap should persist a local marker under `.opensymphony/` so later worker attempts can reuse the workspace without replaying creation hooks.

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
- Hook timeouts use the configured `hooks.timeout_ms`.
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
    async fn ensure(&self, issue: &Issue) -> Result<WorkspaceHandle>;
    async fn run_hook(&self, hook: HookKind, workspace: &WorkspaceHandle) -> Result<()>;
    async fn remove(&self, workspace: &WorkspaceHandle) -> Result<()>;
}
```

`WorkspaceHandle` should expose:

- `issue_id`
- `identifier`
- `workspace_path`
- `metadata_dir`
- `conversation_manifest_path`

## 16. Tests required

- sanitize identifier edge cases
- canonical path containment
- create vs reuse
- `after_create` only once
- `before_run` every worker lifetime
- timeout on hook
- terminal cleanup
- metadata file write and reload
- conversation reset path preserves workspace safety
