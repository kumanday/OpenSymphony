---
id: OSYM-882
title: Edge Delta And Module Topology Diff
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-880"]
blocks: ["OSYM-883"]
areas:
  - code-intelligence
  - graph-view
  - gateway
  - frontend
  - desktop
parent: null
---

## Summary

Make Code Graph diffs represent topology changes, including new and removed
symbol edges and new connections between modules or communities.

## Scope

### In scope

- Add a stable logical `edge_key` separate from revision-bound `edge_id`, based
  on edge kind, stable source identity, stable target identity or normalized
  unresolved hint, and a deterministic duplicate ordinal.
- Diff baseline and composite workspace edge sets into added, removed,
  retargeted, and confidence-changed relationships without treating line shifts
  as topology changes.
- Aggregate symbol-edge deltas into directory/module/community connection deltas
  with counts by edge kind and confidence.
- Extend the shared DTOs, gateway routes, native commands, HTTP/native/fixture
  adapters, deep links, and update events with topology-delta data.
- Expand blast-radius details to identify unchanged inbound callers/references,
  their paths, confidence, and distance rather than counts alone.
- Render added/removed edges, module-connection deltas, confidence, and accessible
  list summaries in Code Graph Diff mode without turning topology into a policy
  or pass/fail judgment.

### Out of scope

- Architecture conformance rules, layering gates, or cycle policing.
- Compiler-grade call resolution.
- Rename/move inference beyond V1 remove-plus-add behavior.

## Deliverables

- Stable edge identity and revision-aware edge comparison.
- Edge, module-connection, and detailed blast-radius DTOs.
- Web/desktop graph rendering, filters, inspector details, and accessibility list.
- Edge-heavy fixtures and regression tests.

## Acceptance Criteria

- [ ] Adding a call or import between previously unconnected modules produces one
      added edge and one added module-connection delta.
- [ ] Removing that relationship produces the corresponding removed deltas.
- [ ] Moving an unchanged relationship to another line does not create a false
      added/removed topology change.
- [ ] Retargeted and confidence-changed relationships are distinguishable from
      added/removed edges.
- [ ] Unchanged baseline callers into modified or removed workspace symbols appear
      as source-cited blast-radius entries.
- [ ] Confidence is visible without relying on color, and unresolved edges remain
      explicitly labeled.

## Test Plan

- Add read-model tests for stable edge identity, duplicate relationships, line
  shifts, added/removed/retargeted edges, and confidence changes.
- Add module aggregation and inbound blast-radius fixtures across multiple files.
- Add gateway-schema round trips and HTTP/native parity tests.
- Add renderer DOM/snapshot, accessibility-list, and edge-heavy scale tests.
- Run focused Rust and TypeScript suites, `cargo clippy-system-duckdb`,
  `npm test`, and `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 7.2, 9, 10, and 13.
- Inspect revision-bound edge ID construction and `code_edge_revisions` in
  `crates/opensymphony-memory/src/index.rs`.
- Inspect `CodeDiffOverlay` and blast-radius projection in
  `crates/opensymphony-memory/src/code_graph.rs`.
- Reuse OSYM-880's baseline-plus-overlay composition.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task surfaces structural facts with extraction confidence. It does not claim
that a new module connection is good or bad.
