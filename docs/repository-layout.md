# Repository Layout

This document records the intended package, module, and directory ownership for
the OpenSymphony implementation repo.

## 1. Top-level layout

```text
OpenSymphony/
  AGENTS.md
  README.md
  WORKFLOW.example.md
  Cargo.toml
  crates/
  docs/
  examples/
  scripts/
  tools/
  .github/
```

`Cargo.toml` at the repository root is the only Cargo package manifest.

OpenSymphony publishes one crates.io package, `opensymphony`.

The `crates/opensymphony-*` directories remain because they are useful internal
subsystem boundaries, but they are source directories compiled into the main
package, not standalone published crates.

## 2. Internal subsystem boundaries

### `opensymphony_domain`

- shared domain types
- scheduler state and transitions
- snapshot models

### `opensymphony_workflow`

- `WORKFLOW.md` loading
- typed front-matter resolution
- strict prompt rendering
- environment and path resolution
- migration errors for removed workflow fields

### `opensymphony_workspace`

- workspace path resolution
- containment and sanitization
- lifecycle hooks
- issue and conversation manifests

### `opensymphony_linear`

- Linear GraphQL read adapter
- pagination and normalization
- tracker reconciliation helpers
- guarded operator-side issue archival for memory cleanup

### `opensymphony_memory`

- issue capsule generation
- DuckDB memory index and markdown indexes
- memory search, related-context lookup, and compact briefs
- docs sync planning and public/private link checks
- archive eligibility checks

### `opensymphony_openhands`

- local server supervision
- REST client
- WebSocket event stream
- issue session runner

### `opensymphony_codex`

- local Codex app-server stdio adapter
- Codex JSON-RPC lifecycle request and notification normalization helpers
- model credential reuse mapping from gateway model settings
- benchmark requirement descriptors for experimental transports

### `opensymphony_orchestrator`

- scheduler loop
- retry queue
- reconciliation
- worker supervision

### `opensymphony_control`

- control-plane HTTP API
- snapshot publication

### `opensymphony_cli`

- `init`
- `run`
- `debug`
- `memory`
- `linear archive`
- `daemon`
- `tui`
- `doctor`
- `rehydrate`

### `opensymphony_tui`

- FrankenTUI operator UI

### `opensymphony_testkit`

- fake OpenHands helpers
- fake Linear fixtures
- contract-test utilities

## 3. Shared non-module assets

### `tools/openhands-server/`

Owns the pinned local OpenHands package and launch scripts that the published
CLI embeds for `opensymphony install openhands`.

### `examples/`

Holds sample configs and target-repo fixtures.

### `docs/`

Owns design, operations, and migration documentation.
Build and developer workflow notes live in `docs/build.md` and
`docs/developer-experience.md`; keep those discoverable from the broader
operations/development docs when generated memory sync creates or refreshes
them.

### `.agents/skills/` in the template repo

Owns target-repo agent guidance. The most important Linear assets now live in
the template skill tree instead of a separate bridge crate:

- `SKILL.md`
- `scripts/linear_graphql.py`
- `queries/*.graphql`
- `references/*.md`

## 4. Template skill propagation rule

`opensymphony init` and `opensymphony update` must copy `.agents/skills/`
recursively so that target repos receive the complete skill payload, including
helper scripts and query assets.

That rule is now part of the supported public behavior.

## 5. Versioning note

OpenSymphony `1.0.0` removed the old agent-side Linear bridge layer. The
internal module layout above is the post-removal structure and should stay free
of dead bridge code.

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
- COE-426: Codex App-Server Prototype And Benchmarks
- COE-428: Model Configuration UI And Routing Metadata
- COE-429: Codex Approvals And Harness/Model Selection
- COE-452: DuckDB Prebuilt Developer Build Mode
- COE-453: Non-Interactive Init For Automation
- COE-475: ChatGPT OAuth For Codex Harness
- COE-476: Codex Production Harness Enablement
- COE-478: Harden model profile storage and validation follow-ups
- COE-479: Codex Debug Session Resume

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
- COE-426
- COE-428
- COE-429
- COE-452
- COE-453
- COE-475
- COE-476
- COE-478
- COE-479

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
