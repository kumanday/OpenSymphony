---
id: OSYM-840
title: Debug Attachment Core Refactor
milestone: "M12.5: ACP Debugging And IDE Attach"
priority: 2
estimate: 8
blockedBy: []
blocks: ["OSYM-841", "OSYM-843", "OSYM-844", "OSYM-845"]
areas:
  - debugging
  - openhands-runtime
  - cli
parent: null
---

## Summary

Extract reusable debug attachment primitives from the current terminal debug command so CLI and ACP paths share workspace resolution, conversation attach, stream, and turn execution logic.

## Scope

### In scope

- Introduce a `DebugAttachment` core type or equivalent internal abstraction.
- Extract issue workspace resolution, manifest loading, OpenHands store selection, client construction, stream attach, rehydrate, wait, and run-turn behavior.
- Preserve current terminal debug behavior behind `opensymphony debug <issue-key> --cli`.
- Keep active, archived, and legacy OpenHands conversation store lookup support.

### Out of scope

- ACP JSON-RPC protocol handling.
- Tauri or Zed launch UI.

## Deliverables

- Shared debug attachment core.
- CLI terminal debug path wired through the shared core.
- Regression tests for existing debug workflows.

## Acceptance Criteria

- [ ] Existing terminal debug behavior is preserved through `--cli`.
- [ ] Debug attachment loads `.opensymphony/issue.json` and `.opensymphony/conversation.json` from the exact issue workspace.
- [ ] Active, archived, and legacy OpenHands conversation stores resolve through existing store logic.
- [ ] Shared run-turn logic sends the prompt, runs the conversation, waits for terminal or idle state, and redacts sensitive details.

## Test Plan

- Run debug-session unit tests.
- Add tests for valid workspace attach, manifest parsing, active store, archived store, and legacy flat store lookup.
- Run focused CLI debug tests.

## Context

- Read `docs/opensymphony-acp-debugging-spec.md`.
- Grounding files include `crates/opensymphony-cli/src/debug_session.rs`, `crates/opensymphony-openhands/src/conversation_store.rs`, and `crates/opensymphony-workspace/src/models.rs`.
- Existing debug command COE work is already delivered; this task refactors it for ACP.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not introduce a parallel debug manifest such as `.opensymphony/debug.json`.
