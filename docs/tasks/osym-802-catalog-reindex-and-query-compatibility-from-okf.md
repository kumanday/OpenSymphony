---
id: OSYM-802
title: Catalog Reindex And Query Compatibility From OKF
milestone: "M10.5: OKF Memory Bundle Foundation"
priority: 2
estimate: 8
blockedBy: ["OSYM-800", "OSYM-801"]
blocks: ["OSYM-803", "OSYM-804", "OSYM-820", "OSYM-821", "OSYM-825"]
areas:
  - memory
  - okf
  - duckdb
parent: null
---

## Summary

Make the memory catalog rebuildable from OKF concepts while preserving existing capture, context, related, docs, and search behavior.

## Scope

### In scope

- Implement reindexing from OKF concept documents into the existing catalog or DuckDB-derived index.
- Extract concept IDs, type, title, description, tags, timestamp, visibility, scope refs, source refs, links, citations, and body text.
- Preserve current memory query command behavior after deleting and rebuilding the derived catalog.
- Add freshness and capture warning metadata to the derived catalog where available.

### Out of scope

- Vector search or hosted search expansion.
- Rendering graph views.

## Deliverables

- `opensymphony memory reindex --from-okf` or equivalent internal reindex path.
- Catalog extraction tests for OKF fixtures.
- Compatibility tests for existing memory query commands.

## Acceptance Criteria

- [ ] The catalog can be deleted and rebuilt from an OKF bundle fixture.
- [ ] Existing `memory context`, `related`, `search`, and `docs` commands continue to return equivalent results for migrated fixtures.
- [ ] Broken links are indexed as warnings, not fatal failures.
- [ ] Unknown concept types are indexed as generic concepts.

## Test Plan

- Run catalog rebuild fixture tests.
- Run focused CLI memory tests for context, related, search, and docs commands.
- Run `cargo test --test memory`.

## Context

- Builds on OSYM-800 and OSYM-801.
- Read `docs/okf-memory-spec.md` sections 7 and 8.
- Current memory server behavior is described in `docs/memory.md`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The catalog remains derived; reads must not mutate schema state.
