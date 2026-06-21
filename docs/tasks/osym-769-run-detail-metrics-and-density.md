---
id: OSYM-769
title: Run Detail Metrics And Density
milestone: "M10.4: Desktop Live Operations And Model Polish"
priority: 3
estimate: 3
blockedBy: []
blocks: []
areas:
  - gateway
  - runtime
  - ui
parent: null
---

## Summary

Make the Run Detail Phase/Stream/Turns/Runtime metrics truthful and visually compact.

## Scope

### In scope

- Replace placeholder `turn_count`, `max_turns`, and `runtime_seconds` values in run detail responses where real data is already available.
- Preserve clear fallback values only when the source data is genuinely unknown.
- Ensure Phase and Stream are derived from current liveness/runtime state, not stale or static placeholders.
- Reduce the visual weight and font size of the Run Detail metric grid so it fits comfortably beside changed files and inspector content.
- Add regression coverage for non-zero runtime and turn values.

### Out of scope

- New long-running observability schema fields.
- A full redesign of Run Detail.
- Runtime accounting for future hosted worker pools.

## Deliverables

- Gateway/runtime wiring for truthful run metric values.
- UI density update for the metric grid.
- Tests covering non-placeholder runtime and turn display.

## Acceptance Criteria

- [ ] Active runs show elapsed runtime based on the run start time or available runtime metadata.
- [ ] Turn counts reflect actual observed turns or a documented unknown/fallback state; they no longer always render as `0 / 1`.
- [ ] Phase and Stream continue to reflect liveness state for OpenHands and Codex-backed runs.
- [ ] The metric grid uses smaller text and stable dimensions without crowding adjacent controls.

## Test Plan

- Run gateway schema/API tests that cover run detail serialization.
- Run runtime/gateway tests with a fixture containing non-zero turns and elapsed runtime.
- Run affected UI tests or a small snapshot-style assertion for compact metric rendering.

## Context

- Current gateway run-detail construction sets `turn_count: 0` and `runtime_seconds: 0` for issue snapshots.
- OpenHands and Codex runtime metadata paths also contain placeholder runtime seconds.
- The desktop screenshot showed Phase/Stream/Turns/Runtime taking too much visual weight while also showing static-looking values.

## Definition of Ready

- [ ] Source of truth for elapsed runtime and turns is identified for each supported harness path.
- [ ] Fallback behavior for unknown metrics is documented in the task implementation notes.
- [ ] A coding agent can start from this task without additional planning context.
