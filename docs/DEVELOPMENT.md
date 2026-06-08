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

# Full tests
cargo test

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

When you change shared target-repo assets, update the template first and then
make sure the `init` and `update` flows still copy the full tree.

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
- `docs/configuration.md`
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
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-401: Web App Entry And Deployment Modes
- COE-402: App Shell, Dashboard, Task Graph, And Run Views
- COE-403: Terminal And Log Renderer Prototype
- COE-404: Desktop Connection Profiles And Daemon Management
- COE-409: Desktop Settings, Keychain, And Native Actions
- COE-410: Desktop Local Stream Optimization
- COE-448: Multi-repo memory server and deterministic context
- COE-449: Desktop alpha recovery: replace stubs with functional app

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
- COE-287
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-397
- COE-398
- COE-401
- COE-402
- COE-403
- COE-404
- COE-409
- COE-410
- COE-448
- COE-449

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
