---
id: OSYM-822
title: Shared Graph Frontend Package And Reducers
milestone: "M11.5: LLM Wiki Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-820"]
blocks: ["OSYM-823", "OSYM-824"]
areas:
  - frontend
  - graph-view
parent: null
---

## Summary

Create the shared frontend graph package used by both web and Tauri clients, with transport-agnostic state, filters, search, and selection reducers.

## Scope

### In scope

- Add a shared graph package or module for web and desktop builds.
- Implement graph state reducers for bundle selection, modes, filters, search, selection, layout status, and deep-link state.
- Provide transport adapter interfaces for gateway, memory server, Tauri native, and fixture data.
- Add fixture data for local web and desktop development.

### Out of scope

- Three.js rendering internals.
- Live memory event handling.

## Deliverables

- Shared frontend package with typed DTO consumption.
- Reducer and adapter tests.
- Fixture graph data aligned with gateway DTOs.

## Acceptance Criteria

- [ ] Web and desktop clients can import the same graph package without Tauri-only dependencies leaking into web builds.
- [ ] Reducers cover atlas, bundle, community, neighborhood, timeline, and evidence modes.
- [ ] Search and filters are deterministic and URL or app-history friendly.
- [ ] Fixture data supports local UI development without bypassing DTO contracts.

## Test Plan

- Run frontend type checks and reducer tests.
- Run web build checks to confirm Tauri-only dependencies are excluded.
- Run desktop build checks if the Tauri app is available.

## Context

- Builds on OSYM-820.
- Read `docs/llm-wiki-graph-view-spec.md` sections 5, 7, 8, and 10.
- Existing shared client work lives in the M7 and M10 task areas.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The primary view is a dense operational workspace, not a landing page.
