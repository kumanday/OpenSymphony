---
id: OSYM-823
title: Three.js Graph Renderer And Worker Layouts
milestone: "M11.5: LLM Wiki Graph View"
priority: 2
estimate: 13
blockedBy: ["OSYM-821", "OSYM-822"]
blocks: ["OSYM-826"]
areas:
  - frontend
  - graph-view
  - threejs
parent: null
---

## Summary

Build the graph canvas renderer with Three.js, worker-based layouts, GPU-friendly picking, and responsive pan, zoom, selection, and label behavior.

## Scope

### In scope

- Implement the default 2.5D orthographic graph renderer.
- Add worker-based force, hierarchical, radial neighborhood, and timeline layouts.
- Use instanced node geometry, batched edge geometry, level-of-detail labels, and GPU-friendly picking or spatial indexes.
- Provide busy, stabilizing, empty, and error states.

### Out of scope

- Graph editing.
- Full 3D perspective mode unless it is cheap after the 2.5D path is stable.

## Deliverables

- Three.js renderer component.
- Layout worker and layout adapters.
- Interaction tests or component tests for pan, zoom, hover, selection, and keyboard focus handoff.

## Acceptance Criteria

- [ ] The graph canvas is nonblank with fixture data on desktop and mobile viewports.
- [ ] Layout work does not block inspector or filter UI responsiveness.
- [ ] Labels remain readable through level-of-detail rules.
- [ ] Selection updates the shared graph state within the target latency for loaded fixtures.

## Test Plan

- Run frontend tests for renderer state transitions.
- Run Playwright screenshot checks for desktop and mobile graph viewports.
- Run canvas nonblank pixel checks.

## Context

- Builds on OSYM-821 and OSYM-822.
- Read `docs/specs/llm-wiki-graph-view-spec.md` sections 8, 9, and 14.
- Follow frontend guidance for Three.js scenes and visual verification.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Large graphs should default to community aggregation instead of a full-label hairball.
