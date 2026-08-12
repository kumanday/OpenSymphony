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

### `opensymphony_code_intel`

- built-in Tree-sitter parser provider skeletons
- trusted language detection, source identity, spans, symbols, and AST diagnostics
- query packs and fixtures for built-in grammars

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

- COE-498 contributed: PR #151: Add Rust tree-sitter code intelligence skeleton (merge `ebd27c2`)
- COE-499 contributed: PR #167: feat(memory): use AST code intel for context (merge `2e0a092`)
- COE-500 contributed: PR #168: feat(code-intel): add query packs for agent languages (merge `3fdc146`)
- COE-501 contributed: PR #172: COE-501: Persist code intelligence ingest rows (merge `f75f0f6`)
- COE-502 contributed: PR #174: Expose read-only AST MCP tools (merge `4de8d58`)
- COE-503 contributed: PR #175: feat(code-intel): harden AST limits and docs (merge `c6241a9`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-498: Tree-sitter Provider Skeleton And Rust Parsing
- COE-499: Memory Context AST Provider Integration
- COE-500: Query Packs For Supported Agent Languages
- COE-501: Code Intelligence Persistence And Ingestion
- COE-502: Read-Only AST MCP And CLI Tools
- COE-503: Code Intelligence Performance Docs And Hardening
- COE-505: Add scheduler-side Codex stdio interrupt channel
- COE-506: Invert CodeIntelIndex trait ownership after AST memory integration
- COE-507: Deduplicate query-pack assets for grammar variants
- COE-508: Cache code-intel parsers and compiled query packs

## Source refs

- COE-498
- COE-499
- COE-500
- COE-501
- COE-502
- COE-503
- COE-505
- COE-506
- COE-507
- COE-508

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
