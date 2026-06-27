---
id: OSYM-853
title: Code Intelligence Persistence And Ingestion
milestone: "M12.6: Tree-sitter Code Intelligence"
priority: 3
estimate: 5
blockedBy: ["OSYM-851", "OSYM-852"]
blocks: ["OSYM-854", "OSYM-855"]
areas:
  - code-intelligence
  - memory
  - duckdb
parent: null
---

## Summary

Persist query-derived code documents, symbols, edges, and diagnostics in the memory catalog with freshness checks and explicit admin ingestion.

## Scope

### In scope

- Extend memory record kinds for code symbols, edges, and diagnostics.
- Add DuckDB migrations for `code_documents`, `code_symbols`, `code_edges`, and `code_diagnostics`.
- Expand `memory.ingest_code_intel` to accept `persist`, `languages`, `symbols`, and query-pack selectors.
- Mark stale rows when content hashes, parser versions, query-pack versions, or commit identities no longer match.
- Ensure remote ingestion remains admin-only.

### Out of scope

- Persisting full parse trees.
- Persisting large source snippets by default.
- Background filesystem watching.
- Vector embeddings.

## Deliverables

- Memory schema migration and indexes for code-intelligence derived rows.
- Ingestion report showing parsed files, persisted rows, stale rows, skipped files, and diagnostics.
- Freshness helpers shared by context and ingestion paths.
- Tests for current, stale, and query-pack-version invalidation.

## Acceptance Criteria

- [ ] Ingesting a fixture repo writes structured code document, symbol, edge, and diagnostic rows.
- [ ] Editing a file marks prior content-hash rows stale and writes current rows.
- [ ] Query-pack version changes invalidate derived rows.
- [ ] `memory.ingest_code_intel` generates artifacts without persistence by default.
- [ ] Remote persistent ingestion requires admin access.

## Test Plan

- Run focused memory migration and ingestion tests.
- Run `cargo test-system-duckdb --test memory`.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md` sections 10, 11, 13, and 15.
- Inspect `crates/opensymphony-memory/src/index.rs` migration style.
- Inspect `crates/opensymphony-cli/src/memory.rs` for `memory.ingest_code_intel` tool handling and admin access checks.
- Preserve OKF memory compatibility and keep derived code-intelligence tables separate from durable issue capsules.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Persist metadata and hashes first. Render snippets from files on demand unless a later task proves snapshots are needed.
