---
id: OSYM-809
title: Desktop Run Detail Action Wiring And Cleanup
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 2
estimate: 3
blockedBy: ["OSYM-805"]
blocks: ["OSYM-812"]
areas:
  - desktop
  - ui
  - gateway
parent: null
---

## Summary

Make Run Detail actions real: wire Cancel, Debug, and Workspace to concrete behavior and remove buttons that only pretend to work.

## Scope

### In scope

- Wire `Cancel` to the gateway cancel action and shared diagnostics.
- For OpenHands runs, copy `cd <path-to-target-repo> && opensymphony debug <issue-key>` to the clipboard.
- For Codex runs, open `codex://threads/<session-id>` when possible and copy it as fallback.
- Make `Workspace` copy the workspace path to the clipboard.
- Remove or hide `Retry`, `Detach`, `Comment`, and `Follow-up` until each has a real API or command.
- Hide `Rehydrate` unless the gateway exposes clear tested semantics for it.

### Out of scope

- Implementing the harness interrupt adapters.
- Adding new Linear comment or follow-up mutation flows.
- Replacing the whole Run Detail layout.

## Deliverables

- Shared UI action-bar update.
- Desktop native command wiring for clipboard and deeplink/open fallback.
- Tests for visible actions and action dispatch behavior.

## Acceptance Criteria

- [ ] Run Detail shows only backed actions for the selected run state.
- [ ] Cancel disables while pending and reports acknowledgement or failure from run diagnostics.
- [ ] OpenHands Debug copies the shell-safe debug command and shows operator feedback.
- [ ] Codex Debug opens the deeplink or copies it with operator feedback.
- [ ] Workspace copies the workspace path and confirms the copy.

## Test Plan

- Run affected TypeScript UI tests for action visibility and dispatch.
- Run focused Tauri/native action tests for clipboard and deeplink fallback.
- Manually verify Debug and Workspace behavior in the desktop app if the shell is available.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Inspect `packages/ui-core/src/run-actions.ts`.
- Inspect `packages/ui-core/src/app-shell.ts`.
- Inspect `apps/desktop/src-tauri/src/actions.rs`.
- Inspect `apps/desktop/src/index.ts`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Prefer deleting fake actions over wiring placeholder receipts.
