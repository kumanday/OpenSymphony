---
id: OSYM-815
title: Desktop Project Grouping And Collapse
milestone: "M10.7: Project Grouping And Dependency Signals"
priority: 3
estimate: 5
blockedBy: ["OSYM-813"]
blocks: ["OSYM-816"]
areas:
  - desktop
  - ui
  - gateway
parent: null
---

## Summary

Add desktop task-list grouping by Linear project, with per-project collapse state, using the same project and dependency metadata exposed for the TUI.

## Scope

### In scope

- Group the desktop task list by project when a snapshot includes project-set data.
- Show compact project headings with project identifier, name, and visible counts.
- Add per-project collapse and expand state in the client.
- Preserve current single-project behavior when project-set data is absent.
- Reuse existing shared UI state patterns and avoid a new persistence layer.

### Out of scope

- Full graph visualization.
- Editing Linear project settings.
- Persisting collapsed state across app restarts.
- Rebuilding the Run Detail layout.

## Deliverables

- Shared frontend or desktop UI changes for project headings and collapse state.
- UI tests for grouped, collapsed, expanded, and single-project fallback states.
- Small documentation update if operator behavior changes.

## Acceptance Criteria

- [ ] Desktop task lists can show one heading per visible Linear project.
- [ ] Collapsing a project hides its issue rows without losing the selected run detail when possible.
- [ ] Expanding a project restores its issue rows in the previous order.
- [ ] Single-project snapshots preserve the existing compact layout unless explicit project-set data is present.
- [ ] Tests cover grouped, collapsed, expanded, and missing-project-data states.

## Test Plan

- Run focused TypeScript UI tests for the shared app shell or desktop task list.
- Run the desktop/web build touched by the UI package.
- Run `git diff --check`.

## Context

- Read `docs/specs/tui-dependency-gutter-spec.md` for grouping semantics.
- Inspect `packages/ui-core/src/app-shell.ts`.
- Inspect `apps/desktop/src/index.ts`.
- Depends on `OSYM-813`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Collapse is desktop-only in this wave; the first TUI slice remains display-only.

