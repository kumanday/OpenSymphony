---
id: OSYM-871
title: Symbol Identity Container Chain And Code Read Model
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: []
blocks: ["OSYM-872", "OSYM-874", "OSYM-875"]
areas:
  - code-intelligence
  - memory
  - graph-view
parent: null
---

## Summary

Add stable logical symbol identity and the read-model primitives the Code Graph needs without replacing the existing revision-bound code-intelligence records.

## Scope

### In scope

- Extract enclosing-symbol relationships during ingest and expose container chains root to leaf.
- Add `symbol_key` as a stable logical identity tier beside the existing `symbol_id`.
- Use deterministic ordinal suffixes for duplicate logical symbols within one document.
- Add a `symbol_key` index and migration for `code_symbols`.
- Carry resolved `symbol_key` endpoints where edge targets can be resolved.
- Normalize edge confidence at the shared DTO/read-model boundary.
- Add bounded neighborhood traversal and span-containment query helpers over the existing code tables.

### Out of scope

- Rename or move detection across `symbol_key` boundaries.
- Type-checker or LSP-backed call graph precision.
- HTTP gateway routes, native commands, or frontend rendering.

## Deliverables

- `opensymphony_code_intel` extraction updates for container ownership.
- Memory index migration and persistence updates for `symbol_key`.
- Read-model helpers for symbol lookup, neighborhood traversal, edge target resolution, and span containment.
- Fixture coverage for stable identity, duplicates, containers, and confidence handling.

## Acceptance Criteria

- [ ] `symbol_id` remains revision-bound and still changes when exact content or span identity changes.
- [ ] `symbol_key` survives content edits and line shifts that keep repo, path, language, kind, container chain, and name stable.
- [ ] Duplicate logical symbols in one document receive deterministic ordinal suffixes.
- [ ] Container chains are available for symbol detail and file containment rendering.
- [ ] Neighborhood traversal is bounded and reports truncation metadata instead of silently dropping records.
- [ ] Edge target resolution marks exact, syntactic, heuristic, and unresolved relationships honestly.

## Test Plan

- Add code-intelligence fixture tests for container extraction and `symbol_key` stability.
- Add memory migration and persistence tests for the new indexed column.
- Add read-model tests for span containment, bounded neighborhoods, and edge confidence.
- Run focused `cargo test-system-duckdb` tests for code-intelligence and memory.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 7.2 and 7.3.
- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md` for the existing code-intelligence data model.
- Inspect `crates/opensymphony-code-intel`.
- Inspect `crates/opensymphony-memory/src/index.rs`.
- The graph must not depend on row IDs as stable UI identity.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep this as a compatible amendment to the AST layer. Do not add a parallel ontology.
