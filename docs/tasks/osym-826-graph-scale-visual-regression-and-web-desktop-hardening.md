---
id: OSYM-826
title: Graph Scale, Visual Regression, And Web/Desktop Hardening
milestone: "M11.5: LLM Wiki Graph View"
priority: 3
estimate: 8
blockedBy: ["OSYM-823", "OSYM-824", "OSYM-825"]
blocks: []
areas:
  - frontend
  - graph-view
  - testing
parent: null
---

## Summary

Harden the graph view across scale, browser, desktop, accessibility, and visual regression targets.

## Scope

### In scope

- Validate graph performance at 500, 5,000, and aggregation-oriented 20,000 node fixtures.
- Add Playwright screenshot checks across desktop and mobile web viewports.
- Add desktop smoke coverage for the shared graph package where the Tauri app is available.
- Add accessibility checks for keyboard flow, list fallback, semantic inspector content, reduced motion, and color independence.

### Out of scope

- Editing memory concepts or graph links.
- New graph modes beyond those already defined in the graph view milestone.

## Deliverables

- Scale fixture suite.
- Visual regression and WebGL nonblank checks.
- Accessibility and web/desktop parity test coverage.

## Acceptance Criteria

- [ ] A 500-node graph is interactive within the target load budget.
- [ ] A 5,000-node graph remains usable with progressive layout and level-of-detail labels.
- [ ] A 20,000-node fixture defaults to aggregation or filtered overview.
- [ ] Web and desktop clients pass screenshot, nonblank canvas, and accessibility checks.

## Test Plan

- Run frontend unit and integration tests.
- Run Playwright visual checks for web and desktop targets available in CI.
- Run graph scale benchmarks or scripted performance checks.

## Context

- Builds on OSYM-823, OSYM-824, and OSYM-825.
- Read `docs/specs/llm-wiki-graph-view-spec.md` sections 14, 16, and 17.
- This milestone prepares the graph view for M13 release-quality validation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not let performance fixtures become a source of fake product data in normal runtime paths.
