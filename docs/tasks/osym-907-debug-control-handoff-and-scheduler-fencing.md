---
id: OSYM-907
title: Debug Control Handoff And Scheduler Fencing
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 2
estimate: 8
blockedBy:
- OSYM-840
- OSYM-904
blocks:
- OSYM-841
- OSYM-845
areas:
- debugging
- orchestrator
- acp
parent: null
---

## Summary

Arbitrate exclusive prompt and operator-response ownership between the scheduler and an IDE attachment without allowing concurrent writers or premature cleanup.

## Scope

### In scope

- Add orchestrator-owned observe/acquire/release commands with lease generation fencing and audited outcomes through the existing control plane.
- Acquiring debug control first holds dispatch/retries for the bound issue, then settles or explicitly cancels in-flight work and resolves pending interactions before granting a writer.
- Route permissions/questions to exactly one response owner. Reject late scheduler, IDE and UI decisions after transfer; other issue scheduling continues.
- Release control only after debug-owned work settles. Define cancellation deadlines, EOF, editor crash, stale lease and host restart behavior; retain uncertain execution fences until stopped/reconciled.
- Coordinate workspace retention/cleanup and live-session ownership; terminal and IDE prompts use the same arbitration.

### Out of scope

- Multiple simultaneous writers, collaboration between editors, and independent scheduler mutation from UI processes.

## Deliverables

- Control handoff state machine and typed commands/events/receipts.
- Race tests and operator-visible busy, held, waiting and uncertain states.

## Acceptance Criteria

- [ ] A scheduler turn racing IDE acquisition cannot overlap an IDE prompt; no idle-check/time-of-use window permits a second writer.
- [ ] Only one IDE process obtains control for a session; stale generation commands and duplicate responses fail deterministically.
- [ ] Pending approvals during acquisition/release have a documented single responder and cannot strand a blocked ACP call.
- [ ] EOF, lease expiry or failed cancellation never unblocks scheduler execution merely because the editor disappeared.
- [ ] Closing an attachment cancels its own active work before releasing control/resources; durable workspace/session identity survives, and unrelated work is not cancelled.

## Test Plan

- Run deterministic scheduler/IDE race tests for retries, concurrent attach, cancellation, pending permissions, editor crash, restart and cleanup.
- Verify other issue workers continue during debug hold and that handoff receipts reflect actual quiescence.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md: control ownership and close/disconnect.
- Existing orchestrator command channels, worker interrupts, terminal-runtime envelope and cleanup/lease fencing; OSYM-901 and OSYM-904 seams.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

This is a required safety boundary for multi-harness attachment. Waiting for an idle status without holding the scheduler is insufficient.
