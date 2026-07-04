---
id: OSYM-875
title: Cross Graph Code Memory And Work Chips
milestone: "M12.9: Code Graph View"
priority: 3
estimate: 5
blockedBy: ["OSYM-871", "OSYM-872", "OSYM-873"]
blocks: ["OSYM-876"]
areas:
  - memory
  - graph-view
  - gateway
parent: null
---

## Summary

Connect Code Graph symbols to related work items and memory concepts through lazy inspector chips, while keeping graph DTOs separate and visibility boundaries intact.

## Scope

### In scope

- Add `symbol_key` to newly written code source refs where code-intelligence records cite symbols.
- Resolve Code Graph inspector issue chips from `scope_refs`.
- Resolve Code Graph inspector memory-concept chips from source refs and citations.
- Add Knowledge Graph inspector code chips for concepts whose source refs cite code symbols or paths.
- Navigate chips to their home surface without merging Task, Knowledge, and Code Graph DTOs.
- Preserve private memory visibility and hosted snippet policy during chip resolution.

### Out of scope

- A merged tri-graph snapshot.
- Editing memory, work items, or code through chips.
- Backfilling all old memory capsules with `symbol_key`.

## Deliverables

- Source-ref write updates for new code records.
- Lazy chip resolution endpoints or adapter calls using existing memory and graph contracts.
- Code Graph inspector issue and memory chip rendering.
- Knowledge Graph inspector code chip rendering.
- Tests for visibility, navigation, missing targets, and stale symbols.

## Acceptance Criteria

- [ ] Selecting a symbol shows related issues when ingested records carry matching `scope_refs`.
- [ ] Selecting a symbol shows related memory concepts when source refs or citations mention the symbol or path.
- [ ] Selecting a Knowledge Graph concept can show code chips when its source refs cite code.
- [ ] Chip activation switches to the owning graph surface and restores the target selection.
- [ ] Missing, stale, or unauthorized targets are shown honestly instead of causing broken navigation.
- [ ] Cross-graph chips do not introduce a merged graph DTO or graph editing path.

## Test Plan

- Add memory/source-ref fixture tests for `symbol_key` writes.
- Add inspector chip resolution tests for issue, memory, and code targets.
- Add navigation tests for Code Graph to Task Graph and Knowledge Graph transitions.
- Add visibility tests covering private memory and hosted snippet denial.
- Run focused memory, gateway, graph, and ui-core tests.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 7.4 and 11.3.
- Read `docs/specs/okf-memory-spec.md` for scope and source refs.
- Inspect the existing Knowledge Graph inspector and memory detail DTOs.
- Cross-graph relationships are chips, not a combined graph.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Backfill is not required for v1. New records should carry enough identity for links going forward.
