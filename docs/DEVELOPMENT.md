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

```bash
# 1. Build the shared frontend first (workspace root).
npm install
npm run build --workspace=@opensymphony/gateway-schema
npm run build --workspace=@opensymphony/ui-core
npm run build --workspace=@opensymphony/desktop

# 2. Run the desktop frontend against a loopback gateway.
#    The bundle auto-attaches via HttpGatewayTransport if the
#    Rust gateway isn't reachable.
(cd apps/desktop && npm run dev)            # vite dev server on 127.0.0.1:1420
# 3. (optional) Launch the Tauri shell once dependencies are installed.
cargo install --path apps/desktop/src-tauri --locked
(cd apps/desktop/src-tauri && cargo run)
```

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

When you change shared target-repo assets, update the template first and then
make sure the `init` and `update` flows still copy the full tree.

Provisioning scripts can initialize a target repo without stdin prompts:

```bash
opensymphony init \
  --non-interactive \
  --linear-project-slug my-linear-project \
  --conflict-policy overwrite \
  --commit-and-push
```

Use `cargo test-system-duckdb --test init` after changing the init flow.

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

- COE-429 contributed: PR #137: Implement Codex approvals and harness/model selection (merge `ba53548`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-429: Codex Approvals And Harness/Model Selection

## Source refs

- COE-429

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
