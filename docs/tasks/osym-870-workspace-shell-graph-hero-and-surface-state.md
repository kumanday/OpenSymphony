---
id: OSYM-870
title: Workspace Shell Graph Hero And Surface State
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 5
blockedBy: []
blocks: ["OSYM-873", "OSYM-874", "OSYM-876"]
areas:
  - desktop
  - web
  - graph-view
parent: null
---

## Summary

Rework the shared workspace shell so graph surfaces live in a full-width hero above two resizable lower columns, while existing Task Graph and Knowledge Graph behavior remains intact.

## Scope

### In scope

- Compact the status pane into a top-bar status strip.
- Move event ticks into a compact mini-view with an expandable full-log modal.
- Move Model Configuration behind a gear action and modal.
- Promote the graph pane to a full-width hero with a surface toolbar ready for Task Graph, Knowledge Graph, and Code Graph.
- Replace the three-column workspace with two resizable lower columns and per-surface content registration.
- Preserve per-surface mode, selection, filter, and column state across surface toggles.

### Out of scope

- Code Graph backend data.
- Code Graph rendering beyond a disabled or fixture-ready surface slot.
- Reworking Knowledge Graph content rendering beyond the shell contract needed here.

## Deliverables

- Updated shared app shell layout and surface registration code.
- Status strip, event-log modal, and model-configuration modal UI.
- State persistence for graph surface toggles and lower-column registration.
- Regression coverage for existing Task Graph and Knowledge Graph flows.

## Acceptance Criteria

- [ ] The graph surface renders as a full-width hero in web and Tauri desktop shells.
- [ ] Task Graph and Knowledge Graph toggles preserve their previous mode, selection, filters, and lower-column state after round trips.
- [ ] Run Detail and Inspector remain visible in the lower columns for Task Graph.
- [ ] Status, connection, event log, and model configuration controls are still reachable without occupying hero space.
- [ ] The shell can register a third Code Graph surface without requiring code-graph data.

## Test Plan

- Run focused ui-core shell and reducer tests.
- Run existing web or desktop shell smoke tests that cover the dashboard.
- Add a visual or DOM regression check for surface toggle restoration and lower-column layout.
- Run `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 6.1, 6.2, and 6.4.
- Inspect `packages/ui-core/src/app-shell.ts`.
- Inspect the existing graph pane toggle behavior for Task Graph and Knowledge Graph.
- OSYM-873 and OSYM-874 depend on this shell geometry for the user-facing Code Graph surface.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Ship this against existing graph surfaces first. Do not wait for Code Graph data.
