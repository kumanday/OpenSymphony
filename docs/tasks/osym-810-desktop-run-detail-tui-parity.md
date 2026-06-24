---
id: OSYM-810
title: Desktop Run Detail TUI Parity
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 3
estimate: 3
blockedBy: []
blocks: ["OSYM-812"]
areas:
  - desktop
  - ui
  - gateway
parent: null
---

## Summary

Bring desktop Run Detail and global status closer to the TUI by showing real branch, PR, token, turn, and diff information while removing unbacked placeholder panels.

## Scope

### In scope

- Show Run Detail `Turns` as the observed count, not a `current/max` fraction.
- Remove Run Detail placeholders for validation commands, validation evidence, and pending approvals unless backed by gateway data.
- Add branch and clickable PR URL to Run Detail.
- Add per-run token breakdown: input, cache, output, and total.
- Update global desktop token status to show input, cache, output, and total.
- Render additions green and deletions red in changed-file summaries and rows.

### Out of scope

- Adding new validation or approval subsystems.
- Running GitHub CLI directly from desktop UI if a gateway/workspace detail endpoint can provide the data.
- Redesigning the full desktop dashboard.

## Deliverables

- Gateway/schema updates for branch and PR data if needed.
- Shared UI rendering updates for tokens, turns, branch, PR link, and file stats.
- Tests for the new display behavior.

## Acceptance Criteria

- [ ] Turns render as a single observed count.
- [ ] Branch and PR link appear when known; PR opens as a hyperlink.
- [ ] Per-run and global token displays include input, cache, output, and total.
- [ ] Validation/approval placeholders are absent unless real gateway data exists.
- [ ] Additions are green and deletions are red in changed-file UI.

## Test Plan

- Run gateway schema/API tests for any new Run Detail fields.
- Run affected TypeScript UI tests for token, turn, branch, PR, and diff rendering.
- Compare a local desktop Run Detail against the TUI for a run with branch, PR, and changed files.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Inspect `packages/gateway-schema/src/run.ts`.
- Inspect `packages/gateway-schema/src/snapshot.ts`.
- Inspect `packages/ui-core/src/app-shell.ts`.
- Inspect `packages/ui-core/src/diff.ts`.
- Inspect TUI display code in `crates/opensymphony-tui/src/lib.rs`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use data already exposed by the control plane where possible. Add schema fields only when the desktop cannot get the TUI data without local command hacks.
