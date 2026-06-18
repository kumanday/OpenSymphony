---
id: OSYM-804
title: Docs Sync And MCP Admin Parity For OKF
milestone: "M10.5: OKF Memory Bundle Foundation"
priority: 3
estimate: 5
blockedBy: ["OSYM-801", "OSYM-802", "OSYM-803"]
blocks: []
areas:
  - memory
  - okf
  - docs
  - mcp
parent: null
---

## Summary

Route docs sync and memory admin surfaces through OKF-derived data where appropriate while preserving current private/public boundaries.

## Scope

### In scope

- Update docs sync to read from OKF concepts or the catalog derived from them.
- Add MCP/admin parity for lint, export, import, reindex, and sync-docs operations where the memory server owns the equivalent capability.
- Update operator docs for OKF workflows and safety boundaries.
- Keep direct file or DB access as an offline admin fallback only.

### Out of scope

- Graph viewer endpoints.
- Hosted UI for memory administration.

## Deliverables

- Docs sync compatibility updates.
- Memory MCP/admin command parity where applicable.
- Documentation updates in `docs/memory.md` and `docs/operations.md`.

## Acceptance Criteria

- [ ] Existing docs sync commands continue to work after OKF reindex.
- [ ] Public docs never expose private bundle paths or source snapshots.
- [ ] MCP/admin tooling exposes equivalent OKF maintenance operations where appropriate.
- [ ] Operator docs describe OKF lint, export, import, reindex, and sync-docs usage.

## Test Plan

- Run docs sync tests and public-doc lint tests.
- Run memory MCP/admin tests if present.
- Run `opensymphony memory lint --okf` on generated fixtures.

## Context

- Builds on OSYM-801, OSYM-802, and OSYM-803.
- Read `docs/okf-memory-spec.md` sections 6, 9, and 12.
- Existing docs sync behavior is documented in `docs/memory.md`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Scheduler correctness must not depend on memory capture, docs sync, or graph rendering.
