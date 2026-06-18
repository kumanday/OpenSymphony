---
id: OSYM-825
title: Live Memory Graph Integration And Privacy Gates
milestone: "M11.5: LLM Wiki Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-803", "OSYM-820", "OSYM-821"]
blocks: ["OSYM-826"]
areas:
  - gateway
  - graph-view
  - memory
  - security
parent: null
---

## Summary

Connect the graph view to live memory data, update events, stale-state warnings, and authorization-aware visibility filtering.

## Scope

### In scope

- Connect graph client adapters to gateway or memory-server DTOs.
- Handle `memory_graph_updated` events and stale graph cursor state.
- Enforce visibility filtering for private, public, local desktop, and hosted token scopes.
- Redact secret-like frontmatter values and local paths before rendering.

### Out of scope

- Hosted billing, tenant management, or broader auth implementation.
- Direct local filesystem graph browsing outside approved desktop capability gates.

## Deliverables

- Live graph data adapter.
- Privacy and redaction tests.
- Stale graph and reindex warning UI states.

## Acceptance Criteria

- [ ] Hosted or scoped clients cannot widen graph visibility through filters.
- [ ] Local paths are hidden or normalized unless the client is authorized for local desktop mode.
- [ ] Graph updates respond to memory graph events without requiring full app reload.
- [ ] Stale and warning-heavy graph states are visible to the operator.

## Test Plan

- Run visibility filtering tests for graph DTOs.
- Run frontend integration tests with update events.
- Run web and desktop adapter tests.

## Context

- Builds on OSYM-803, OSYM-820, and OSYM-821.
- Read `docs/llm-wiki-graph-view-spec.md` sections 7, 12, and 15.
- M11 hosted auth work provides the broader authorization foundation outside this package.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The graph view must not mutate orchestrator, Linear, or memory state.
