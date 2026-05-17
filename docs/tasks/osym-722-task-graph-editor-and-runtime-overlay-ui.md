---
id: OSYM-722
title: Task Graph Editor And Runtime Overlay UI
milestone: "M8: Task Graph Operations And OpenHands Run UI"
priority: 2
estimate: 8
blockedBy: ["OSYM-712", "OSYM-720", "OSYM-721"]
blocks: ["OSYM-735"]
parent: null
---

## Summary

Build editable task graph views that combine Linear project structure with OpenSymphony runtime overlays.

## Scope

### In scope

- Add editable milestone, issue, and sub-issue views.
- Add create dialogs and safe inline editing.
- Add dependency editor.
- Add comments and evidence editor.
- Show failed, blocked, queued, running, complete, and stale runtime states.
- Add workspace, harness, diff summary, validation, retry, and blocker badges.
- Add filters for task and runtime state.

### Out of scope

- Planning conversation UI.
- Hosted admin UI.
- Deep Linear project settings.

## Deliverables

- Task graph editor UI.
- Runtime overlay UI.
- Dependency editor.
- UI tests for edits, filters, and server acknowledgement behavior.

## Acceptance Criteria

- [ ] Users can browse and edit milestones, issues, and sub-issues through gateway-mediated actions.
- [ ] Runtime overlay badges link task nodes to active or historical runs.
- [ ] Edits reconcile with server acknowledgements and event updates.

## Test Plan

- Run component tests for edit forms, dependency editor, filters, and overlay states.
- Replay task graph update events after mutation receipts.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.5.2 and `docs/host-client-implementation_plan.md` P4.6/P4.7.
- UI state must remain transport-agnostic and event-reduced.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Optimistic UI behavior should follow the acknowledgement rules chosen for gateway action receipts.
