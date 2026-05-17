---
id: OSYM-712
title: App Shell, Dashboard, Task Graph, And Run Views
milestone: "M7: Shared Client And Desktop Alpha"
priority: 2
estimate: 8
blockedBy: ["OSYM-703", "OSYM-710", "OSYM-711"]
blocks: ["OSYM-722", "OSYM-725", "OSYM-735", "OSYM-742"]
parent: null
---

## Summary

Build the first shared UI surfaces for navigation, dashboard, task graph reads, and run detail reads.

## Scope

### In scope

- Implement app navigation shell, project sidebar, resizable panes, command palette placeholder, connection status bar, and keyboard focus model.
- Render dashboard health, active runs, queue, retries, and recent events.
- Render project, milestone, issue, and sub-issue hierarchy with runtime overlay badges.
- Render run summary, event timeline placeholder, workspace metadata, harness metadata, action capability bar, diff placeholder, and validation placeholder.

### Out of scope

- Editable task graph mutations.
- Full terminal/log renderer.
- Planning workspace.

## Deliverables

- Shared layout components.
- Dashboard page.
- Task graph explorer.
- Run detail page.
- UI smoke and fixture tests.

## Acceptance Criteria

- [ ] Users can navigate from project to milestone to issue to sub-issue to run detail.
- [ ] Reconnecting and stale states are visible in dashboard and detail views.
- [ ] Task graph views use Linear milestone, issue, and sub-issue nomenclature.

## Test Plan

- Run component tests against gateway fixtures.
- Run responsive layout checks for desktop and browser viewport sizes.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.4 and `docs/host-client-architecture.md` 5.4.
- Follow the frontend guidance in the repository instructions for dense operational interfaces.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The first UI should be useful with read-only gateway data.
