# Operations

This document covers the current local operator workflow for OpenSymphony.

Packaging note: crates.io publishes one package, `opensymphony`. The internal
`crates/opensymphony-*` directories are module trees inside that package, not
separately published dependencies.

## 1. Core commands

Recommended CLI commands:

- `opensymphony init`
- `opensymphony update`
- `opensymphony run`
- `opensymphony debug <issue-id>`
- `opensymphony tui`
- `opensymphony doctor`
- `opensymphony rehydrate <issue-id> --reason "..."`

## 2. First-run flow

```bash
cargo install opensymphony
opensymphony install openhands
opensymphony --help

cd /path/to/target-repo
opensymphony init
opensymphony update
opensymphony run
```

If you already run an external OpenHands agent-server, you can skip
`opensymphony install openhands`.

Important `init` behavior:

- fetches the current template payload
- leaves an existing `AGENTS.md` untouched and writes starter guidance to
  `AGENTS-example.md` during first-time setup
- prompts before overwriting repo-owned files
- optionally scaffolds AI PR review assets
- can configure GitHub Actions variables, the `review-this` label, and the
  optional AI review secret automatically when `gh` is installed and can access
  the target repository
- prompts whether to commit and push the generated OpenSymphony files; when
  accepted, it stages only files it wrote, commits `chore: bootstrap
  OpenSymphony`, and pushes `HEAD` to the detected remote
- supports `--non-interactive` for automation; pass explicit flags for prompt
  decisions and unresolved existing-file conflicts fail before any files are
  written
- copies `.agents/skills/` recursively so helper scripts, query files, and
  reference docs all arrive together
- keeps bootstrap guidance in CLI output and the central OpenSymphony docs
  instead of copying `docs/` files into the target repository

Automation-friendly target repo provisioning can run without stdin prompts:

```bash
cargo install opensymphony
opensymphony install openhands

cd /path/to/target-repo
opensymphony init \
  --non-interactive \
  --linear-project-slug my-linear-project \
  --conflict-policy overwrite \
  --commit-and-push
```

For scripts that scaffold AI PR review too, add the review flags explicitly:

```bash
opensymphony init \
  --non-interactive \
  --ai-pr-review \
  --configure-github \
  --ai-review-provider-kind openai-compatible \
  --ai-review-model-id accounts/fireworks/models/glm-5p1 \
  --ai-review-base-url https://api.fireworks.ai/inference/v1 \
  --ai-review-require-evidence true \
  --ai-review-secret-env LLM_API_KEY \
  --linear-project-slug my-linear-project \
  --conflict-policy overwrite
```

If `--configure-github` is omitted, init still writes the AI PR review files
when `--ai-pr-review` is present, but it prints the manual `gh` commands instead
of mutating repository variables, secrets, or labels. If a non-interactive run
finds an existing generated file and `--conflict-policy` was not supplied, it
fails before applying the template.
When `--ai-review-secret-env` is used, the named environment variable must be
present and non-empty; init fails rather than setting a blank GitHub secret.

For already-initialized repositories, `opensymphony update` is the fast
maintenance path:

- checks the latest published `opensymphony` version and skips
  `cargo install opensymphony` when the running CLI is already current
- refreshes changed or new template-managed files under `.agents/skills/`
- leaves `WORKFLOW.md`, `AGENTS.md`, `.github/*`, and repo-local extra skills
  alone

Normal user installs use bundled DuckDB. This keeps `cargo install
opensymphony` and `opensymphony update` turnkey even when the memory database is
enabled.

Power users who want to avoid compiling bundled DuckDB may install a system
DuckDB development package and build without default features. On the
macOS/Homebrew development host, install and pin DuckDB once:

```bash
brew install duckdb
brew pin duckdb
```

Homebrew currently provides `duckdb`, not a versioned `duckdb@...` formula.
Pinning keeps the verified local version from moving during routine Homebrew
upgrades. The expected version for this release line is DuckDB `1.5.3`. To
build manually against that system library:

```bash
export DUCKDB_LIB_DIR="$(brew --prefix duckdb)/lib"
export DUCKDB_INCLUDE_DIR="$(brew --prefix duckdb)/include"
export DYLD_LIBRARY_PATH="$DUCKDB_LIB_DIR${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
cargo install opensymphony --no-default-features --features duckdb-prebuilt
```

On Linux, set `DUCKDB_LIB_DIR`, `DUCKDB_INCLUDE_DIR`, and `LD_LIBRARY_PATH` to
the matching DuckDB installation. On Windows, set `DUCKDB_LIB_DIR`,
`DUCKDB_INCLUDE_DIR`, and add the DuckDB DLL directory to `PATH` before running
the same Cargo install command. This is a manual optimization path: verify a
memory command after installation, and expect to keep the runtime library
available anywhere the installed binary runs.

To update a power-user system-linked install, run the same Cargo install command
with the same environment first. Then run `opensymphony update` from a target
repository only to refresh template-managed agent assets. Starting with
`opensymphony update` may reinstall the default bundled build when a newer
release exists.

## 3. Recommended validation commands

For fast iterative development inside this repository on the macOS/Homebrew
host, use the system-linked developer aliases:

```bash
cargo fmt --check
cargo check-system-duckdb
cargo test-system-duckdb
cargo test-system-duckdb --test memory
cargo clippy-system-duckdb
```

If system DuckDB is unavailable, use the portable downloaded fallback aliases:

```bash
cargo check-dev
cargo test-dev
cargo clippy-dev
```

The system aliases set `DUCKDB_LIB_DIR`, `DUCKDB_INCLUDE_DIR`, and
`DYLD_LIBRARY_PATH` for the aliased command. The fallback aliases set
`DUCKDB_DOWNLOAD_LIB=1` only for the aliased command. Both alias families use
`--no-default-features --features duckdb-prebuilt`. If a downloaded fallback
command must override `CARGO_TARGET_DIR`, use an absolute path. Release-
sensitive, packaging, and dependency work should still include the default
bundled-mode checks so `cargo install opensymphony` remains turnkey for users:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test init
cargo test --test help
cargo test --test update
./scripts/smoke_local.sh
```

Dependency audit notes:

- COE-429 adds `jsonschema = 0.46.5` as the runtime validator for installed
  Codex app-server JSON Schema payload checks. Release provenance was checked
  against the `Cargo.lock` crates.io source/checksum entries for `jsonschema`
  and its called-out transitive crates (`fancy-regex`, `fluent-uri`, and
  `fraction`), the dependency tree was reviewed with `cargo tree -p jsonschema
  --depth 2`, and `cargo audit` exited successfully against the current lockfile
  on 2026-06-21. Re-run those checks when upgrading `jsonschema`.

Useful runtime checks:

```bash
curl http://127.0.0.1:2468/healthz
curl http://127.0.0.1:2468/api/v1/snapshot
curl http://127.0.0.1:2468/api/v1/capabilities
curl http://127.0.0.1:2468/api/v1/dashboard/snapshot
opensymphony tui --url http://127.0.0.1:2468/ --exit-after-ms 1200
```

## 4. Doctor expectations

`opensymphony doctor` is a real preflight tool.

It is optional troubleshooting/preflight help, not the primary install path for
managed local OpenHands. The normal setup flow is `cargo install opensymphony`
followed by `opensymphony install openhands`.

Current scope:

- loads and resolves the target repo `WORKFLOW.md`
- renders the workflow prompt with a synthetic issue
- validates required local tools
- validates bundled OpenHands tooling
- probes the configured OpenHands transport
- can create a temp conversation and verify runtime readiness

Expected checks include:

- config parses
- target repo exists
- `WORKFLOW.md` resolves cleanly
- required env-backed config values exist
- `cargo`, `curl`, `git`, and `uv` are on `PATH`
- the pinned OpenHands toolchain is present
- loopback/local safety warnings are surfaced

When the configured transport uses managed local OpenHands, `doctor` can
bootstrap the pinned tooling into the configured `openhands.tool_dir` before
continuing the rest of its checks.

## 4.1 Subscription Credential Operations

OpenAI ChatGPT/Codex subscription mode is explicit and feature-gated. Build or
install OpenSymphony with `--features openhands-subscription-credentials`, then
configure the target repo workflow with
`openhands.conversation.agent.llm.credential_mode: openai_subscription`.

Credential establishment belongs to the documented OpenHands SDK flow or to a
future hosted credential broker. For local or self-hosted use, run the
OpenHands SDK browser or device-code login in the environment that owns the
credential store, keep refresh material in the selected auth directory, and
export only the short-lived access-token reference expected by the workflow
before starting `opensymphony run`. Do not place OAuth JSON files, access
tokens, or refresh tokens inside issue workspaces or repository files.
`auth_directory_env`, `auth_method`, `open_browser`, and `force_login` are
operator/bootstrap metadata for that credential setup step; they are preserved
for status and diagnostics, while the runtime conversation request resolves only
the short-lived access token and optional account identity header.

Validation for subscription mode should include:

- mocked subscription request construction tests
- redaction checks for manifests, diagnostics, and debug output
- live integration only when a valid subscription credential and pinned SDK
  support are available

Codex app-server subscription readiness is separate from the OpenHands SDK auth
directory. The gateway reports local Codex readiness through model settings by
running supported Codex CLI checks only:

```bash
codex --version
codex app-server --help
codex login status
```

When `codex login status` is logged out or expired, run
`codex login --device-auth`. Some ChatGPT accounts require enabling
**Security and login -> Enable device code authorization for Codex** before the
device-code flow succeeds. To revoke local Codex access, run `codex logout` and
use ChatGPT account settings for account-side revocation. OpenSymphony must not
read private Codex credential files or copy access/refresh material into
workspaces, logs, workflow files, Linear comments, or browser payloads. Gateway
readiness checks are cached briefly and have bounded per-command timeouts so
operator UI polling cannot hang on a stalled local Codex command.

The local Codex app-server harness path launches
`codex --dangerously-bypass-hook-trust app-server --stdio` and is advertised as
available when clients read `/api/v1/capabilities`. Before starting a run,
OpenSymphony generates the JSON Schema from the installed Codex CLI and
validates its full-automation `thread/start` and `turn/start` payloads. If the
installed schema rejects those payloads, update Codex before running the Codex
harness. Unsupported or logged-out Codex installations must fail with the
readiness guidance above instead of partially starting an issue. Loopback
WebSocket and hosted Codex worker pools remain non-production paths.

For cross-harness route testing, run `opensymphony run --dry-run`.
OpenSymphony will still poll Linear and prepare workspaces, but the worker
returns a route preview instead of launching a model-backed harness. The preview
is recorded as a `routing.decision` runtime event and includes the selected
harness, model, and model profile. To force a local process override without
editing workflow config, start the daemon with `OPENSYMPHONY_HARNESS`, and pass
`OPENSYMPHONY_MODEL` / `OPENSYMPHONY_MODEL_PROFILE` when a launcher wants to use
the active model profile selected in the desktop or web UI.

The Codex local stdio route executes the configured Codex binary with
`cwd == issue_workspace_path`. `OPENSYMPHONY_CODEX_BIN` is a trusted local
operator override and must not be treated as a hosted or multi-tenant input.
Approval requests are surfaced through normalized runtime events and shared
approval-center data models, but approval decisions are not yet forwarded from
the operator action plane into a live Codex stdio session in this alpha route.

The alpha model configuration panel exposed by the web and desktop shells uses
the shared model profile state store, but those entrypoints currently construct
it without durable storage. Treat profile edits as session-local until a
desktop secure-settings backend or hosted settings service is wired in. The UI
may keep model strings, routing hints, subscription bootstrap metadata, and
stored credential references in memory, but raw provider keys and OAuth refresh
material must stay in the selected keychain, OpenHands auth directory, or
hosted secret store.

## 5. Linear operational model

OpenSymphony 1.0.0 is GraphQL-only for agent-side Linear operations.

Operational implications:

- there is no separate local Linear bridge process to start
- initialized target repos rely on `LINEAR_API_KEY`
- the checked-in helper lives at
  `.agents/skills/linear/scripts/linear_graphql.py`
- checked-in query files under `.agents/skills/linear/queries/` are the
  supported mutation/query surface
- issue creation, issue rewrite passes, blocker relations, comments, PR
  attachments, and project updates should all use those checked-in assets

Smoke test:

```bash
cd /path/to/target-repo
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/viewer.graphql
```

## 6. Project memory

Project memory stores policy and learned structure in
`.opensymphony/memory/memory.yaml` and private runtime artifacts under
`.opensymphony/memory/`. `opensymphony run` captures terminal issue transitions
automatically when `memory.auto_capture` is enabled in `config.yaml`:

```yaml
memory:
  auto_capture: true
  auto_archive: false
```

Manual commands remain available for setup, backfill, inspection, and guarded
archive operations:

```bash
opensymphony memory init
opensymphony memory capture COE-123
opensymphony memory status
opensymphony memory brief COE-123
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory sync-docs --since-last-sync
opensymphony memory lint --public-docs
```

Add `--dry-run` to write commands when an operator wants a non-writing preview.

Use `opensymphony memory import --source-file completed.yaml` only for
deterministic imports, migrations, tests, or external exports. Failed Linear or
GitHub access should be fixed before live capture is retried.

`memory capture` creates or refreshes issue capsules, updates
`.opensymphony/memory/memory.duckdb`, and refreshes markdown indexes when
enabled. Normal builds use DuckDB's bundled native library so operators do not
need to install DuckDB separately, at the cost of heavier Rust compile time and
a larger binary. Repository development can opt into the `duckdb-prebuilt`
feature through the system-linked `cargo check-system-duckdb`,
`cargo test-system-duckdb`, and `cargo clippy-system-duckdb` aliases, or the
downloaded fallback `cargo check-dev`, `cargo test-dev`, and `cargo clippy-dev`
aliases. Treat that native dependency as part of the hosted deployment threat
model before enabling memory in a multi-tenant service.
Memory capture does not archive Linear issues.

Read commands such as `memory status`, `memory brief`, `memory related`, and
`memory context` open the DuckDB index in read-only mode and do not run schema
migrations. Run capture, import, docs sync, or reindex-style admin operations
serially if a local DuckDB writer is active.

For worker or tool access, `opensymphony run` starts the read-only memory server
when memory is initialized and `memory.serve` is not disabled. The supervised
server binds to loopback on an ephemeral port by default, reports the endpoint
through the control-plane recent events, and passes
`OPENSYMPHONY_MEMORY_ENDPOINT` into managed local OpenHands workers. Manual
operation is also available with `opensymphony memory serve --addr
127.0.0.1:8765`, which exposes MCP-style `initialize`, `tools/list`, and
`tools/call` JSON-RPC methods at `/mcp`. Set `OPENSYMPHONY_MEMORY_TOKEN` or
pass `--token` to require bearer-token access for read tools. Admin tools
(`memory.capture`, `memory.sync_docs`, `memory.lint`, `memory.reindex`, and
`memory.ingest_code_intel`) require `OPENSYMPHONY_MEMORY_ADMIN_TOKEN` or
`--admin-token`. When only the admin token is configured, it also gates read
tools; do not inject that token into ordinary worker environments.

Linear archival is a separate command and is guarded by captured memory:

```bash
opensymphony linear archive --issues COE-123
```

For explicit issue selectors, the archive command captures live Linear and
GitHub evidence before evaluating the guard. It blocks issues that have no
capsule or unresolved capture warnings unless `--force` is supplied. Normal mode
resolves Linear credentials from `WORKFLOW.md` and calls the Linear GraphQL
archive mutation.

If the repo uses managed local OpenHands, archive also moves the issue's
persisted OpenHands conversation into the repo-scoped `archived/` store. Archive
uses the workspace `.opensymphony/conversation.json` manifest when present and
falls back to scanning managed conversation `meta.json` files for a matching
`workspace.working_dir` issue key, so legacy flat conversations and repo-scoped
`active/` conversations can still be moved even when workspace metadata is
stale. Normal orchestrator runs use the sibling `active/` store, while
`opensymphony debug COE-123` searches active and archived stores and starts the
managed server against the store containing the requested conversation. If
another OpenHands server is already bound to the configured port with a
different store, stop it and retry the debug command.

For issues last run through the local Codex app-server harness,
`opensymphony debug COE-123` reads the recorded Codex thread id from the issue
workspace manifest and runs `codex resume <thread-id>` from that exact issue
workspace. Set `OPENSYMPHONY_CODEX_BIN` to override the Codex binary. Use
`opensymphony debug COE-123 --app` to print `codex://threads/<thread-id>`
without starting OpenHands or launching the Codex CLI.

See [Project Memory](memory.md) for the full command surface, import YAML
schema, and troubleshooting notes.

## 7. Rehydration

Rehydration is the explicit recreation of an OpenHands conversation while
preserving enough history for continuation.

Use it for:

- API key rotation
- broken persisted conversation state
- intentional provider/model changes

Examples:

```bash
opensymphony rehydrate COE-123 --reason "API key rotation"
opensymphony doctor --config ./config.yaml --rehydrate
```

## 8. Local safety

- prefer loopback-only OpenHands targets for local development
- treat target repos and prompts as trusted local input
- do not keep unrelated OpenHands servers running on the same configured port
- stop `opensymphony run` with Ctrl-C so the orchestrator can terminate its
  managed OpenHands process tree; Ctrl-Z only suspends the orchestrator and can
  leave the server bound to the configured port
- do not store provider secrets in checked-in files

## 9. Migration note

If an older target repo still contains `openhands.mcp`, remove that block.
OpenSymphony 1.0.0 expects Linear access through `LINEAR_API_KEY` and the
repo-local GraphQL helper assets copied by `opensymphony init`.

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
- COE-275: Remote agent-server mode and auth hardening
- COE-277: Implement hierarchy-aware task selection
- COE-278: Doctor live probe resolves repo-local OpenHands launcher paths reliably
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-284: Add orchestrator run command to CLI and make it installable
- COE-286: Abort active CLI worker tasks on graceful orchestrator shutdown
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-293: OpenHands agent has no filesystem tools - only FinishTool and ThinkTool
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-389: Current Gateway Inventory And Vocabulary
- COE-390: Gateway Schemas And Stream Feasibility
- COE-391: Gateway Module, Capabilities, And Dashboard Snapshot
- COE-392: Task Graph, Run Detail, File, And Diff Read APIs
- COE-393: Event Journal And Stream Broker
- COE-394: Frontend Workspace And Shared Schemas
- COE-395: Planning Artifact Schema And Session Service
- COE-396: Action Receipts And Initial Run Actions
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-400: OpenHands Event Normalization And Runtime Mirror
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
- COE-419: Hosted Auth Placeholders And Web Parity
- COE-425: OpenHands Subscription Credential Adapter
- COE-426: Codex App-Server Prototype And Benchmarks
- COE-428: Model Configuration UI And Routing Metadata
- COE-429: Codex Approvals And Harness/Model Selection
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics
- COE-448: Multi-repo memory server and deterministic context
- COE-449: Desktop alpha recovery: replace stubs with functional app
- COE-452: DuckDB Prebuilt Developer Build Mode
- COE-453: Non-Interactive Init For Automation
- COE-475: ChatGPT OAuth For Codex Harness
- COE-476: Codex Production Harness Enablement
- COE-478: Harden model profile storage and validation follow-ups
- COE-479: Codex Debug Session Resume
- COE-480: Run Detail Metrics And Density
- COE-481: Model Configuration Codex Subscription Follow-Up
- COE-482: TUI Codex Token Usage Accounting
- COE-483: Codex Event Content Summaries
- COE-484: Desktop Live Snapshot And Run Detail Refresh

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
- COE-275
- COE-277
- COE-278
- COE-280
- COE-281
- COE-282
- COE-284
- COE-286
- COE-287
- COE-293
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-389
- COE-390
- COE-391
- COE-392
- COE-393
- COE-394
- COE-395
- COE-396
- COE-397
- COE-398
- COE-399
- COE-400
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
- COE-419
- COE-425
- COE-426
- COE-428
- COE-429
- COE-434
- COE-435
- COE-448
- COE-449
- COE-452
- COE-453
- COE-475
- COE-476
- COE-478
- COE-479
- COE-480
- COE-481
- COE-482
- COE-483
- COE-484

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
