---
id: OSYM-814
title: FrankenTUI Project Headers And Dependency Gutter
milestone: "M10.7: Project Grouping And Dependency Signals"
priority: 2
estimate: 5
blockedBy: ["OSYM-813"]
blocks: ["OSYM-816"]
areas:
  - tui
  - control
parent: null
---

## Summary

Implement the TUI issue-list grouping and compact dependency display from the TUI dependency gutter spec.

## Scope

### In scope

- Render one project header per visible project in project-set mode.
- Keep project headers non-selectable and preserve selected-issue windowing context.
- Add a fixed-width dependency gutter before each issue identifier.
- Add a short dependency suffix only when width allows.
- Show expanded dependency detail for the selected issue.
- Preserve one visual line per issue row.

### Out of scope

- New keyboard interactions.
- Collapsible groups.
- Reordering issues beyond the current issue order plus project grouping.
- Mutating Linear, scheduler, or orchestrator state from the TUI.

## Deliverables

- TUI reducer and renderer changes for project headers, dependency gutters, suffixes, and selected detail.
- Narrow-terminal fitting behavior that drops suffixes before corrupting titles or pane separators.
- Reducer and rendering tests for ready, blocked, downstream, hidden, single-project, and multi-project cases.

## Acceptance Criteria

- [ ] Project-set mode renders a one-line project header for every visible project.
- [ ] Project headers identify the project without repeating it on every issue row.
- [ ] Selection skips project headers.
- [ ] Active and Todo rows can show upstream and downstream dependency hints when width allows.
- [ ] Completed blockers are omitted from compact issue-list suffixes.
- [ ] The selected issue detail pane can show completed blocker detail.
- [ ] Missing dependency data renders blank markers and no suffix.

## Test Plan

- Run focused `opensymphony-tui` reducer and rendering tests.
- Add narrow-width rendering tests for suffix truncation and pane boundary preservation.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/tui-dependency-gutter-spec.md`.
- Inspect `crates/opensymphony-tui/src/lib.rs`.
- Inspect `docs/ui-frankentui.md`.
- Depends on `OSYM-813`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep this read-only. The gutter is display only, not scheduling logic.

