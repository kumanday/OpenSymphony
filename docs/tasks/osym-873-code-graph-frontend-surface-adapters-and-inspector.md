---
id: OSYM-873
title: Code Graph Frontend Surface Adapters And Inspector
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-870", "OSYM-872"]
blocks: ["OSYM-874", "OSYM-875", "OSYM-876"]
areas:
  - graph-view
  - web
  - desktop
parent: null
---

## Summary

Add the Code Graph as a third shared graph surface with fixture, HTTP, and native adapters, code-specific filters, layout presets, styling, deep links, and symbol detail rendering.

## Scope

### In scope

- Register the `Code Graph` surface in the shared graph toolbar.
- Add `CodeGraphAdapter` implementations for fixture, HTTP, and Tauri-native data.
- Add Atlas, File, Neighborhood, and Diff mode state using the existing graph reducer patterns.
- Add code-specific filters for repo, language, symbol kind, edge kind, confidence, freshness, diagnostics, path prefix, community, and delta status.
- Add code node and edge styling using existing renderer extension points.
- Add layout presets for Atlas, File, Neighborhood, and Diff modes.
- Render symbol/file detail sections, raw record toggle, and standalone structure-list fallback.

### Out of scope

- Diff-pane gutter navigation and overlay computation.
- Cross-graph issue and memory chips.
- New renderer or layout engine dependencies.

## Deliverables

- Shared frontend Code Graph surface registration and adapter code.
- Fixture data for Atlas, File, and Neighborhood modes.
- Code-specific renderer styling and filter reducers.
- Inspector and structure-list rendering for code records.
- Deep-link support for `{surface: "code", repo_id, mode, symbol_key?, path?, run_id?, depth, filters}`.

## Acceptance Criteria

- [ ] The Code Graph toggle renders Atlas and Query views from fixture data without code-graph backend availability.
- [ ] HTTP and native adapters can load equivalent snapshots from OSYM-872 contracts.
- [ ] Edge confidence is visible through line style or opacity and filterable without relying on color alone.
- [ ] Node freshness is visible and filterable.
- [ ] Atlas opens aggregated and has no path that renders an unaggregated full repo by default.
- [ ] Deep links restore the target surface, mode, selection, filters, and layout seed.

## Test Plan

- Add frontend reducer and adapter tests for Code Graph modes and filters.
- Add renderer snapshot or DOM checks for code node, edge, confidence, and freshness styling.
- Add deep-link round-trip tests for Atlas, File, and Neighborhood states.
- Run the relevant package tests for `@opensymphony/graph` and `packages/ui-core`.
- Run `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 6, 9, 11.2, and 11.3.
- Inspect `packages/graph`.
- Inspect `packages/ui-core/src/knowledge-graph-renderer.ts`.
- Inspect the existing Knowledge Graph adapter and fixture patterns.
- Use the OSYM-821 renderer and layout decisions; do not restart dependency evaluation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This is a surface extension, not a new graph engine.
