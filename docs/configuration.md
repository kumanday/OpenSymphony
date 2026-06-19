# Configuration

This document covers target-repo bootstrap, generated files, and the runtime
configuration that `opensymphony run` expects.

## Bootstrap

Use `opensymphony init` from the target repository root:

```bash
cd /path/to/target-repo
opensymphony init
```

`opensymphony init` is the primary setup path for existing repositories. It:

- fetches the current starter files from the template repo's raw GitHub URLs
- copies missing files into the target repo
- leaves an existing `AGENTS.md` untouched and writes starter guidance to
  `AGENTS-example.md` during first-time setup
- prompts before overwriting other conflicting files
- fills the `WORKFLOW.md` clone hook from `git remote` when possible
- offers to fill the Linear project slug/key in `WORKFLOW.md`
- creates or updates `.gitignore` so local OpenSymphony runtime state stays untracked
- can optionally scaffold OpenHands AI PR review
- can configure the GitHub Actions variables, label, and optional review secret
  automatically when `gh` is installed and can access the repository
- prompts whether to commit and push the generated OpenSymphony files so shared
  skills and, when selected, AI PR Review setup are present in the remote
  repository before story work starts

For repositories that are already initialized, `opensymphony update` is the
maintenance path for template-owned skills:

```bash
cd /path/to/target-repo
opensymphony update
```

The command first checks whether the installed CLI is older than the newest
published `opensymphony` release and only runs `cargo install opensymphony`
when it actually needs to. If the current directory already looks like an
OpenSymphony target repo because it has both `WORKFLOW.md` and `config.yaml`,
the command then refreshes changed or new files under `.agents/skills/`.

The template repository is still the upstream source of those starter assets,
but it is an implementation detail of `opensymphony init`, not a required
manual setup step:

- [kumanday/OpenSymphony-template](https://github.com/kumanday/OpenSymphony-template)
- [Raw template base](https://raw.githubusercontent.com/kumanday/OpenSymphony-template/refs/heads/main/WORKFLOW.md)

## Files Added By `init`

Core bootstrap payload:

- `WORKFLOW.md`
- `AGENTS.md`
- `AGENTS-example.md` when `AGENTS.md` already existed before first-time setup
- `config.yaml`
- `.gitignore` created or updated to ignore OpenSymphony runtime state
- `.agents/skills/` copied recursively, including skill-local `references/`, `scripts/`, and similar helper files
- `.agents/skills/linear/references/`
- `.github/CODEOWNERS`
- `.github/pull_request_template.md`

## Refreshing Template Skills

`opensymphony update` only refreshes template-managed files under
`.agents/skills/`.

It does not:

- rerun the interactive `init` prompts
- modify `WORKFLOW.md`
- merge or rewrite `AGENTS.md`
- create `AGENTS-example.md` after `config.yaml` exists
- copy `.github/*` bootstrap files
- delete repo-local extra skills that are not in the template tree

Optional AI PR review scaffolding:

- `.github/workflows/ai-pr-review.yml`
- `.agents/skills/custom-codereview-guide.md`

## Labels

If you enable AI PR review and `gh` is available with repository access,
`opensymphony init` can create the `review-this` label for you. If automation is
skipped, create it once per repository:

```bash
gh label create "review-this" --description "Trigger AI PR review" --color "d73a4a" --force
```

## Review The Generated Workflow

After `init`, review `WORKFLOW.md` and `config.yaml`.

If you accept the final commit/push prompt, `init` stages only the files it
created or updated, commits them as `chore: bootstrap OpenSymphony`, and pushes
`HEAD` to the detected git remote. If the repository already has staged changes
or no single remote can be detected, `init` leaves git alone and prints a
reminder to commit and push manually.

Important fields:

| Field | Description | Env Var | Example |
|-------|-------------|---------|---------|
| `tracker.project_slug` | Linear `Project.slugId` from the project URL | - | `my-project-5250e49b61f4` |
| `workspace.root` | Where to store per-issue workspaces | - | `~/.opensymphony/workspaces` |
| `openhands.conversation.agent.llm.model` | LLM model to use | `LLM_MODEL` | `openai/accounts/fireworks/models/glm-5p1` |

For Linear trackers, `tracker.project_slug` should store the project's
`slugId`, not a `team/project` path.

## Environment Variables

OpenSymphony uses standard OpenHands environment variable names.

Fireworks example via the OpenAI-compatible provider adapter:

```bash
export LLM_MODEL="openai/accounts/fireworks/models/glm-5p1"
export LLM_API_KEY="fw-..."
export LLM_BASE_URL="https://api.fireworks.ai/inference/v1"
```

The workflow supports `${VAR}` syntax for environment variable substitution in
the front matter:

```yaml
openhands:
  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
```

## Conversation Condensation

Optional conversation condensation is enabled by default per workflow to reduce
long-history context pressure before the agent-server hits the model window:

```yaml
openhands:
  conversation:
    agent:
      condenser:
        max_size: 240
        keep_first: 2
```

OpenSymphony forwards an OpenHands `LLMSummarizingCondenser` that reuses the
conversation agent's LLM settings. The condenser is enabled by default with
`max_size: 240` and `keep_first: 2`. To disable it, set `enabled: false`.

## Runtime Config

`opensymphony init` also copies a starter `config.yaml` next to the target
repository `WORKFLOW.md`.

Minimal local-supervised example:

```yaml
control_plane:
  bind: 127.0.0.1:2468

openhands:
  tool_dir: ~/.opensymphony/openhands-server

memory:
  auto_capture: true
  auto_archive: false
```

The bind address is the single local HTTP surface for both the gateway API used
by the web/desktop clients (`/api/v1/capabilities`,
`/api/v1/dashboard/snapshot`, and related `/api/v1/*` routes) and the
control-plane compatibility routes used by the TUI (`/healthz`,
`/api/v1/snapshot`, and `/api/v1/control/events`).

Provision that app-managed directory with:

```bash
opensymphony install openhands
```

For managed local OpenHands, OpenSymphony derives a repository-scoped
conversation store from `openhands.tool_dir` and the target repo path:

```text
<tool_dir>/workspace/conversations/repos/<repo-key>/
  active/
  archived/
```

`opensymphony run` first moves known terminal issue conversations from existing
workspace manifests into `archived/`, then prepares `active/` from current
Linear candidate issue manifests before launching the managed server with
`OH_CONVERSATIONS_PATH` pointing at `active/`. The terminal-workspace sweep is a
temporary compatibility shim for older flat stores. This keeps completed or
manually archived issue history out of normal server startup while preserving it
for `opensymphony debug`.

When your workflow points at an external OpenHands agent-server with
`openhands.transport.session_api_key_env`, `config.yaml` can omit
`openhands.tool_dir`.

Use [examples/target-repo/config.yaml](../examples/target-repo/config.yaml) as
the starting template if you want to inspect the checked-in example.

[examples/configs/local-dev.yaml](../examples/configs/local-dev.yaml) is a
developer-facing doctor fixture for this repository. It is not the runtime
config that `opensymphony run` looks for in a target repo.

## Planning Workspace

The planning workspace is a dense, editable, review-oriented UI for the
hosted-client mode. It renders from the local planning workspace state and is
intended to feel like a task-creation tool with Linear as the publishing
target.

### Intentional MVP limitations

- The fixture planning session is intentionally reused across project switches
  in the local app shell. The workspace is not yet keyed per project, so
  switching projects keeps the same conversation, artifacts, and hierarchy
  until the gateway provides real planning sessions or a per-project session
  loader is implemented. This is documented behavior, not a bug.

## Memory Configuration

Project memory stores runtime state under `.opensymphony/memory` and can be
captured automatically by `opensymphony run`. Runtime automation is controlled
by `config.yaml`:

```yaml
memory:
  auto_capture: true
  auto_archive: false
```

`auto_capture` defaults to `true`. It captures terminal issue transitions
observed by the run loop. `auto_archive` defaults to `false`; when enabled, it
archives only after successful capture with no blocking warnings.
When archive succeeds and the repo uses the managed local OpenHands server,
OpenSymphony also moves the issue's persisted conversation from the repo-scoped
`active/` store to `archived/`.

Initialize the shared memory policy and learned ontology file with:

```bash
opensymphony memory init
```

This creates `.opensymphony/memory/memory.yaml` and updates `.gitignore` so only
that shared config is tracked. Capsules, markdown indexes, DuckDB, source
snapshots, and runtime logs remain local:

```text
.opensymphony/memory/
  memory.yaml
  issues/
  indexes/
  memory.duckdb
```

`memory.yaml` contains policy plus learned structure. `memory init` seeds stable
areas from existing top-level `docs/*.md` files when present; otherwise it
starts with an empty `areas` map and capture evolves it from Linear and PR
narrative evidence:

```yaml
memory_root: .opensymphony/memory
visibility: private
index_path: .opensymphony/memory/memory.duckdb
confidence_threshold: 75
markdown_indexes: true
docs:
  public_root: docs
  default_visibility: public
  deny_private_links: true
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-runtime.md
    visibility: public
    status: stable
    confidence: 85
    aliases:
      - OpenHands Runtime
    source_refs:
      docs:
        - docs/openhands-runtime.md
      linear_labels:
        - runtime
```

Private memory should stay out of source control. Commit
`.opensymphony/memory/memory.yaml` and generated public docs when appropriate;
do not commit issue capsules, markdown indexes, DuckDB, source snapshots, or
runtime state.

## OpenHands PR Review

If you opt into OpenHands PR review during `init`, the CLI will try to
configure the GitHub Actions variables, label, and optional review secret for
you when:

- `gh` is installed
- `gh` can access the target repository
- you approve the automation prompt

If any of those are missing, `init` falls back to a short checklist plus the
manual `gh` commands. The full verification and branch-protection guidance
lives in the OpenSymphony docs at
[ai-pr-review-human-setup.md](ai-pr-review-human-setup.md); `init` does not
copy that guide into the target repository.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-252 contributed: PR #10: Implement foundation workflow and scheduler contracts
- COE-253 contributed: PR #19: COE-253: OpenHands Runtime Adapter (merge `911b0b4`)
- COE-254 contributed: PR #6: COE-254: bootstrap tracker, workspace, and orchestration core
- COE-255 contributed: PR #4: COE-255: add control plane and FrankenTUI slice
- COE-256 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-258 contributed: PR #83: Add memory init and mapped docs sync

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-252: Foundation and Contracts
- COE-253: OpenHands Runtime Adapter
- COE-254: Tracker, Workspaces, and Orchestration
- COE-255: Observability and FrankenTUI
- COE-256: Validation and Local Operations
- COE-258: Bootstrap workspace and crate boundaries
- COE-259: Workflow loader and typed config
- COE-260: Domain model and orchestrator state machine
- COE-261: Local agent-server supervisor
- COE-262: REST client and conversation contract
- COE-263: Workspace manager and lifecycle hooks
- COE-264: Linear read adapter and issue normalization
- COE-265: WebSocket event stream, reconciliation, and recovery
- COE-266: Issue session runner
- COE-267: Linear MCP write surface
- COE-268: Orchestrator scheduler, retries, and reconciliation
- COE-269: Control-plane API and snapshot store
- COE-270: Repository harness and generated context artifacts
- COE-271: FrankenTUI operator client
- COE-272: Fake OpenHands server and protocol contract suite
- COE-273: Live local end-to-end suite
- COE-274: CLI packaging, doctor, and local operations docs
- COE-277: Implement hierarchy-aware task selection
- COE-278: Doctor live probe resolves repo-local OpenHands launcher paths reliably
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-284: Add orchestrator run command to CLI and make it installable
- COE-286: Abort active CLI worker tasks on graceful orchestrator shutdown
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-288: Add context condenser support to prevent LLM context window overflow
- COE-293: OpenHands agent has no filesystem tools - only FinishTool and ThinkTool
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-394: Frontend Workspace And Shared Schemas
- COE-395: Planning Artifact Schema And Session Service
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-400: OpenHands Event Normalization And Runtime Mirror
- COE-401: Web App Entry And Deployment Modes
- COE-402: App Shell, Dashboard, Task Graph, And Run Views
- COE-403: Terminal And Log Renderer Prototype
- COE-404: Desktop Connection Profiles And Daemon Management
- COE-405: Linear Milestone, Issue, And Sub-Issue Mutations
- COE-406: Repository, Linear, And Research Analysis
- COE-409: Desktop Settings, Keychain, And Native Actions
- COE-410: Desktop Local Stream Optimization
- COE-411: Task Graph Editor And Runtime Overlay UI
- COE-412: Runtime Timeline And Terminal/Log Association
- COE-413: Implementation Plan Generator Stage
- COE-414: Diff, Validation, Approval, And Run Action Views
- COE-415: Milestone, Issue, And Sub-Issue Compiler
- COE-416: Dependency Graph And Plan Checks
- COE-417: Planning Workspace UI
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics
- COE-449: Desktop alpha recovery: replace stubs with functional app
- COE-452: DuckDB Prebuilt Developer Build Mode
- COE-453: Non-Interactive Init For Automation
- COE-473: Desktop task graph dependency and run detail parity

## Source refs

- COE-252
- COE-253
- COE-254
- COE-255
- COE-256
- COE-258
- COE-259
- COE-260
- COE-261
- COE-262
- COE-263
- COE-264
- COE-265
- COE-266
- COE-267
- COE-268
- COE-269
- COE-270
- COE-271
- COE-272
- COE-273
- COE-274
- COE-277
- COE-278
- COE-280
- COE-281
- COE-282
- COE-284
- COE-286
- COE-287
- COE-288
- COE-293
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-394
- COE-395
- COE-397
- COE-398
- COE-399
- COE-400
- COE-401
- COE-402
- COE-403
- COE-404
- COE-405
- COE-406
- COE-409
- COE-410
- COE-411
- COE-412
- COE-413
- COE-414
- COE-415
- COE-416
- COE-417
- COE-434
- COE-435
- COE-449
- COE-452
- COE-453
- COE-473

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
