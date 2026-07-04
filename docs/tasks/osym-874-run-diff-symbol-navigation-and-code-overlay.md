---
id: OSYM-874
title: Run Diff Symbol Navigation And Code Overlay
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-870", "OSYM-871", "OSYM-872", "OSYM-873"]
blocks: ["OSYM-876"]
areas:
  - run-detail
  - graph-view
  - code-intelligence
parent: null
---

## Summary

Make Run Detail the primary Code Graph entry by mapping changed diff lines to symbols, opening symbol neighborhoods, and rendering run-scoped delta and blast-radius overlays.

## Scope

### In scope

- Fetch and cache run-scoped file outlines for supported changed files.
- Resolve diff lines to the innermost enclosing symbol using outline spans.
- Render one visible gutter glyph per changed symbol region with hover, keyboard, and context-menu access.
- Add file-header navigation into Code Graph File mode.
- Compute added, removed, and modified symbols between merge-base and head snapshots.
- Compute inbound blast radius for changed symbols using `calls` and `references` edges.
- Expose the run diff-overlay route/native command backed by the shared DTO.
- Render the Run Detail summary strip and graph overlay styling.
- Provide a delta list fallback for accessibility.

### Out of scope

- Rename or move detection.
- Agent workpad or PR-body summaries.
- Live agent-attention visualization.

## Deliverables

- Diff pane symbol affordance UI and navigation into Code Graph Query mode.
- Server-side diff overlay computation and route/native command implementation.
- Run Detail summary strip wired to the overlay DTO.
- Graph overlay styling for added, removed, modified, and blast-radius symbols.
- Tests for graceful absence when code intelligence is disabled or unsupported.

## Acceptance Criteria

- [ ] A supported changed line can open the enclosing symbol neighborhood with the run overlay active in two interactions or fewer.
- [ ] Activating a diff affordance changes only the hero surface; Run Detail and Inspector lower-column state are preserved.
- [ ] The summary strip numbers match the diff-overlay DTO exactly.
- [ ] Unsupported or unanalyzed files appear in `unanalyzed_files` and do not silently disappear.
- [ ] Removed symbols render from the base snapshot as distinct ghosted nodes.
- [ ] Glyphs are keyboard-focusable and have accessible names including the target symbol.

## Test Plan

- Add unit tests for outline span containment and one-glyph-per-symbol-region behavior.
- Add server tests for added, removed, modified, blast-radius, and `unanalyzed_files` classification.
- Add UI tests for diff affordance activation and state preservation.
- Add accessibility checks for glyph labels and delta list fallback.
- Run focused gateway, code-intelligence, and ui-core tests.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 10 and 11.1.
- Inspect `packages/ui-core/src/diff.ts`.
- Inspect `packages/gateway-schema/src/run.ts`.
- Inspect `crates/opensymphony-gateway/src/lib.rs` diff computation helpers.
- The overlay must use the same merge-base comparison as the existing diff pane.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use line containment over the file outline for v1. A symbol-at-position endpoint can come later if benchmarks require it.
