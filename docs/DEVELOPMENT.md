# Development Guide

This document is for contributors working on OpenSymphony itself. For user
setup and operator flows, start with the [README](../README.md) and the docs
linked there.

If you are developing OpenSymphony itself, clone the repository and install from the checkout instead:

```bash
git clone https://github.com/kumanday/OpenSymphony.git && cd OpenSymphony
cargo install --path .
```

## Repository structure

```text
OpenSymphony/
├── Cargo.toml
├── crates/
│   ├── opensymphony-cli/
│   ├── opensymphony-control/
│   ├── opensymphony-domain/
│   ├── opensymphony-linear/
│   ├── opensymphony-openhands/
│   ├── opensymphony-orchestrator/
│   ├── opensymphony-testkit/
│   ├── opensymphony-tui/
│   ├── opensymphony-workflow/
│   └── opensymphony-workspace/
├── docs/
├── examples/
├── scripts/
├── tools/
│   └── openhands-server/
├── AGENTS.md
└── README.md
```

Only the repository-root `Cargo.toml` is a package manifest. The
`crates/opensymphony-*` directories are internal subsystem module trees that
compile into the one public `opensymphony` package.

## Design summary

OpenSymphony is the Rust implementation of the Symphony orchestration model.

Key choices:

- Rust owns orchestration, retries, workspace lifecycle, and tracker
  reconciliation
- OpenHands is the execution substrate
- Linear reads happen through the internal `opensymphony_linear` module
- agent-side Linear writes use the repo-local GraphQL helper assets copied by
  `opensymphony init`
- FrankenTUI is optional and must not affect correctness

## Desktop alpha (COE-449)

The Tauri desktop wrapper now mounts the same shared `OpenSymphonyApp`
shell as the web bundle, instead of the historical stub renderer. Both
entry points live under `apps/`:

- `apps/desktop` — Tauri wrapper; frontend ships from `apps/desktop/dist`.
- `apps/web` — browser bundle served by the gateway or deployed as a
  static site.

### Running the desktop alpha locally

The desktop app is currently a repo-clone development flow. There is no
released `opensymphony app` or `opensymphony desktop` installer/launcher yet;
that work is tracked by COE-488 / OSYM-811. Until then, run the Tauri app from
this checkout.

```bash
# 1. Install frontend dependencies once from the workspace root.
npm install

# 2. Launch the Tauri shell.
cd apps/desktop/src-tauri
cargo run
```

The frontend workspace intentionally does not commit an npm lockfile today:
`package-lock.json` is listed in `.gitignore`, and CI installs from the
workspace package manifests with `npm install`. Revisit that policy before
release packaging or other supply-chain-sensitive frontend dependency changes.

`cargo run` rebuilds the desktop frontend first, so local source changes under
`apps/desktop` and shared frontend packages are reflected in the Tauri shell.

For frontend hot reload, run `npm run dev --workspace=@opensymphony/desktop`
from the workspace root in one terminal and `cargo run` from
`apps/desktop/src-tauri` in another. Plain `cargo run` rebuilds and reads the
current `apps/desktop/dist` bundle; it does not start Vite.

To see live OpenSymphony state instead of the empty or disconnected UI states,
run an OpenSymphony control plane separately, usually with `opensymphony run` in
the target repository. The desktop client connects to the loopback gateway at
`http://127.0.0.1:2468`.

The desktop entry detects the Tauri runtime via
`globalThis.__TAURI__` and uses the native `list_profiles`,
`store_profile`, and `set_active_profile` commands for connection
profile persistence. Outside Tauri (vite dev, `npm run build` preview)
the entry falls back to a loopback HTTP transport against
`http://127.0.0.1:2468` and renders the same `OpenSymphony Desktop`
shell.

The shared shell subscribes to the gateway event stream when the active
transport exposes it. Desktop local mode keeps using the loopback HTTP/SSE
transport for live dashboard, task graph, and Run Detail refreshes; the
unimplemented Tauri channel stream remains a future optimization.

### Verification artifacts

Every release-blocking check below is wired to a single command and is
expected to pass on every pull request:

| Check | Command | Verifies |
|---|---|---|
| TypeScript types | `npm run type-check` | Shared frontend compiles end-to-end |
| Frontend tests | `npx jest --config jest.config.js` (or `npm test`) | Includes the route contract, app-shell render, transport contract, reducer, profile, and discovery suites |
| Desktop bundle | `npm run build --workspace=@opensymphony/desktop` | `dist/index.html` + `dist/assets/main-*.js` contain the real app shell markup (no stub placeholder text) |
| Desktop smoke | `npx jest --config jest.config.js --testPathPattern apps/desktop` | `build-smoke.test.ts` and `app-shell-render.test.ts` both pass |
| Rust desktop | `cd apps/desktop/src-tauri && cargo test` | 36 unit tests + 5 process-ownership integration tests |
| Rust Lint | `cd apps/desktop/src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings` | Formatting + clippy on the desktop crate |

### Acceptance reminders

- Capability discovery (`alphaCapabilities()` in `packages/ui-core/src/app-shell.ts`) only advertises `loopback_http` and explicitly marks `terminal_stream` as `available: false`. No stub native transport is marked as ready.
- Connection profiles persist via the `settings` capability and the
  `fs:allow-read-text-file` / `fs:allow-write-text-file` permission set,
  scoped to `$HOME/.config/opensymphony` by fs-plugin config.
- The frontend's `routes-contract.test.ts` keeps the TS API client in
  lock-step with the Rust axum router declared in
  `crates/opensymphony-gateway/src/lib.rs` (`pub fn router(&self) -> Router`).

## Milestones

### M1 Foundation and contracts

Workspace bootstrap, workflow/config loading, domain model, state machine.

### M2 OpenHands runtime adapter

Local server supervisor, REST client, WebSocket stream, session runner.

### M3 Symphony orchestration core

Workspace lifecycle, Linear adapter, scheduler, GraphQL-backed repo harness.

### M4 Operator UX and repo harness

Control plane, FrankenTUI, generated issue context artifacts.

### M5 Validation and local packaging

Fake server, live tests, doctor command, packaging.

## Required checks

Fast iterative checks on a macOS/Homebrew development machine should use the
system-linked DuckDB aliases. They build with `--no-default-features --features
duckdb-prebuilt` and point Cargo at `/opt/homebrew/opt/duckdb` for the aliased
command. The expected native DuckDB version is `1.5.3`, matching the pinned
Rust `duckdb` and `libduckdb-sys` dependency line.

```bash
cargo fmt --check
cargo check-system-duckdb
cargo test-system-duckdb
cargo clippy-system-duckdb
```

Install and pin DuckDB once on the host:

```bash
brew install duckdb
brew pin duckdb
```

Homebrew does not currently provide a versioned `duckdb@...` formula. Pinning
prevents routine Homebrew upgrades from moving the system library after it has
been verified. If Homebrew DuckDB is later unpinned or upgraded, run
`duckdb --version` and verify it is still DuckDB `1.5.3` before trusting
system-linked checks. If Homebrew DuckDB is unavailable, use the portable
downloaded fallback aliases:

```bash
cargo check-dev
cargo test-dev
cargo clippy-dev
```

The fallback aliases download and reuse a prebuilt DuckDB library inside the
checkout's Cargo target directory. If you override `CARGO_TARGET_DIR` for a
fallback command, use an absolute path; the normal target directory does not
need an override.

Before release-sensitive, packaging, or dependency changes, also run the default
bundled-mode checks:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Useful commands

```bash
# Format and lint
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo clippy-dev
cargo clippy-system-duckdb

# Full tests
cargo test
cargo test-dev
cargo test-system-duckdb

# CLI-focused checks
cargo test --test init
cargo test --test help

# Doctor
cargo run -- doctor --config examples/configs/local-dev.yaml

# Install and smoke-test
cargo install --path . --locked
./scripts/smoke_local.sh
```

## Template ownership

`opensymphony init` bootstraps target repositories from
`OpenSymphony-template`.

Important rule:

- copy `.agents/skills/` recursively, not file-by-file, so helper scripts,
  query assets, and reference docs survive intact
- keep `opensymphony update` aligned with the same recursive copy rule so
  existing target repos can refresh the template-managed skill tree without
  rerunning the full bootstrap flow
- keep `opensymphony init --non-interactive` aligned with the interactive
  bootstrap flow. Every prompt-driven decision should have an explicit flag,
  and unresolved file conflicts must fail before writing.
- keep generated `WORKFLOW.md` branch-target guidance and template-managed
  skills aligned with the `Target branch:` marker instead of hard-coding one
  remote branch.

When you change shared target-repo assets, update the template first and then
make sure the `init` and `update` flows still copy the full tree.
If the current PR cannot modify `OpenSymphony-template`, leave a follow-up that
updates that repo's `WORKFLOW.md` plus `.agents/skills/pull/SKILL.md`,
`.agents/skills/push/SKILL.md`, and `.agents/skills/land/SKILL.md` with the
same branch-marker wording before relying on fresh template fetches.

For configurable target branches and marker-only workflow update flags, see
[Workflow Target Branch And Update Settings Specification](specs/workflow-target-branch-update-spec.md).
The default target branch is `develop`, but automation should also cover
explicit `main` and slash branch names such as `release/next`.

Provisioning scripts can initialize a target repo without stdin prompts:

```bash
opensymphony init \
  --non-interactive \
  --linear-project-slug my-linear-project \
  --target-branch release/next \
  --conflict-policy overwrite \
  --commit-and-push
```

Existing target repos can patch only workflow settings without refreshing the
template-managed skill tree:

```bash
opensymphony update --target-branch develop
opensymphony update --target-branch main
opensymphony update --target-branch release/next
opensymphony update --target-branch release/next --code-review openhands
```

`--code-review openhands` records the marker and attempts to enable an existing
`.github/workflows/ai-pr-review.yml` through `gh workflow`; update settings
mode warns instead of installing or repairing that workflow when it is missing.
`codex` and `none` record the marker and attempt to disable an existing
OpenHands review workflow. If `gh` is unavailable, unauthorized, or cannot
access Actions, verify or adjust the workflow state manually.

Use `cargo test-system-duckdb --test init`,
`cargo test-system-duckdb --test update`, and
`cargo test-system-duckdb --test help` after changing this flow.

## Linear development rules

- keep orchestrator-side Linear logic inside the `opensymphony_linear` module tree
- keep agent-side Linear usage in the template-owned `.agents/skills/linear/`
  tree
- prefer checked-in GraphQL query files over inline ad hoc mutations
- do not reintroduce a separate bridge layer for agent-side Linear writes

## Versioning

OpenSymphony `1.0.0` is the compatibility boundary for the GraphQL-only Linear
rewrite.

Breaking changes in this line include:

- removal of the old workflow-owned Linear bridge configuration
- removal of the bridge CLI entrypoint
- provider-agnostic AI review configuration via `AI_REVIEW_API_KEY`

## Document map

- `AGENTS.md`
- `docs/architecture.md`
- `docs/build.md`
- `docs/configuration.md`
- `docs/developer-experience.md`
- `docs/openhands-agent-server.md`
- `docs/linear-and-tools.md`
- `docs/operations.md`
- `docs/testing-and-operations.md`
- `docs/repository-layout.md`
- `docs/migration-1.0.0.md`

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
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-284: Add orchestrator run command to CLI and make it installable
- COE-285: Refactor orchestrator_run.rs into smaller CLI runtime modules
- COE-287: Add opensymphony debug command for conversational session debugging
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
- COE-407: Browser Transport And Remote Stream Protocols
- COE-408: Harness Adapter And Capability Model
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
- COE-423: Model And Credential Settings
- COE-425: OpenHands Subscription Credential Adapter
- COE-426: Codex App-Server Prototype And Benchmarks
- COE-428: Model Configuration UI And Routing Metadata
- COE-429: Codex Approvals And Cross-Harness Routing
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics
- COE-448: Multi-repo memory server and deterministic context
- COE-449: Desktop alpha recovery: replace stubs with functional app
- COE-452: DuckDB Prebuilt Developer Build Mode
- COE-453: Non-Interactive Init For Automation
- COE-454: OKF Bundle Schema And Legacy Capsule Mapping
- COE-456: OKF Writer, Lint, And Migration Fixtures
- COE-458: Catalog Reindex And Query Compatibility From OKF
- COE-460: OKF Export, Import, And Visibility Boundaries
- COE-463: Docs Sync And MCP Admin Parity For OKF
- COE-465: Shared Graph Frontend Package And Reducers
- COE-467: Three.js Graph Renderer And Worker Layouts
- COE-468: Concept Inspector, Search, Filters, And Accessibility Fallback
- COE-469: Live Memory Graph Integration And Privacy Gates
- COE-471: Graph Scale, Visual Regression, And Web/Desktop Hardening
- COE-473: Desktop task graph dependency and run detail parity
- COE-475: ChatGPT OAuth For Codex Harness
- COE-476: Codex Production Harness Enablement
- COE-478: Harden model profile storage and validation follow-ups
- COE-479: Codex Debug Session Resume
- COE-480: Run Detail Metrics And Density
- COE-481: Model Configuration Codex Subscription Follow-Up
- COE-482: TUI Codex Token Usage Accounting
- COE-483: Codex Event Content Summaries
- COE-484: Desktop Live Snapshot And Run Detail Refresh
- COE-505: Add scheduler-side Codex stdio interrupt channel

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
- COE-280
- COE-281
- COE-282
- COE-284
- COE-285
- COE-287
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
- COE-407
- COE-408
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
- COE-423
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
- COE-454
- COE-456
- COE-458
- COE-460
- COE-463
- COE-465
- COE-467
- COE-468
- COE-469
- COE-471
- COE-473
- COE-475
- COE-476
- COE-478
- COE-479
- COE-480
- COE-481
- COE-482
- COE-483
- COE-484
- COE-505

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
