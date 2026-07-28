---
id: OSYM-891
title: Restart-Safe Parent Integration Controller
milestone: "M12.96: Parent Integration Lifecycle"
priority: 1
estimate: 13
blockedBy: ["OSYM-888", "OSYM-890"]
blocks: ["OSYM-892"]
areas:
  - orchestrator
  - harness
  - memory
  - testing
parent: null
---

## Summary

Run topology-neutral parent integration and final verification through one
durable orchestrator-owned state machine, with bounded command evidence,
cross-repository memory, and deterministic restart behavior.

## Scope

### In scope

- Implement durable parent states from waiting and lease acquisition through
  preparation, refresh, integration, final verification, finalization, blocked,
  failed, and canceled outcomes.
- Record every transition with state version, reason, idempotency key, input
  versions, side-effect intent, result receipt, and retry classification.
- Keep one parent controller authoritative over scheduling and external side
  effects while one parent harness conversation performs implementation and
  verification.
- Support bounded foreground integration commands/checks in the parent root or a
  named checkout without assuming repository roles.
- Require each command to own readiness, descendants, temporary resources, and
  teardown before exit; do not support unmanaged background services.
- Persist named verification attempts with handle/root, conversation, start and
  terminal state, timeout, exit result, bounded logs, allocated ports/resources,
  and cleanup receipt.
- On cancellation or timeout, request harness stop and record acknowledgement or
  uncertainty rather than assuming processes stopped.
- On restart, reconcile the harness when supported; otherwise mark nonterminal
  attempts indeterminate, clean attempt-owned resources, refresh the verified
  baseline, and rerun.
- Give the parent memory access to repo-neutral authorized context, persisted
  snapshots for exactly its descendant repositories, and live overlays only for
  its active integration checkouts.
- Capture parent memory with all verified repository commits used for final
  integration.
- Propagate intermediate parent results and leases to higher ancestors without
  collapsing them into one repository binding.
- Complete only after final verification passes against all recorded target
  commits and durable evidence is written.

### Out of scope

- Creating repair branches, pull requests, or review loops.
- Durable long-running process supervision.
- Per-repository parent controllers.

## Deliverables

- Durable parent controller, transitions, and restart reconciliation.
- Bounded integration-command and resource-lifecycle receipts.
- Parent memory grant, overlays, capture, and negative-scope enforcement.
- Final-verification evidence and higher-parent propagation.
- Crash-injection tests through the no-repair lifecycle.

## Acceptance Criteria

- [ ] An eligible parent starts exactly once and reaches final verification with
      one authoritative controller and one harness conversation.
- [ ] Checks can run from the parent root or any verified checkout handle without
      topology-specific behavior.
- [ ] A timed-out, canceled, or restart-indeterminate attempt never counts as
      passed and is rerun only after cleanup and baseline verification.
- [ ] Attempt logs and resources are bounded, attributable, and contain no
      secrets.
- [ ] The parent can query persisted and live memory for exactly its descendant
      repositories; an unrelated repository remains denied with
      `all_accessible`.
- [ ] Final verification records the exact target commit for every repository.
- [ ] Restart at every no-repair parent state converges without duplicate
      transitions, lost leases, or duplicate side effects.
- [ ] A higher ancestor retains lower checkout evidence after intermediate parent
      completion.

## Test Plan

- Add a fake harness that records parent commands, cancellation, uncertain stop,
  bounded logs, and resource cleanup.
- Inject restart before/after every transition and side-effect receipt.
- Test timeout, port collision, cleanup failure, unknown harness state, and final
  verification failure.
- Add parent memory positive/negative-scope, overlay, capture, and restart tests.
- Run focused orchestrator, worker, memory, and workspace tests,
  `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 13.5 through 13.8,
  14.4, 15, 16, and 18.5.
- Inspect actor ownership in `crates/opensymphony-orchestrator/src/scheduler.rs`,
  worker event normalization in
  `crates/opensymphony-cli/src/orchestrator_run/backends.rs`, and current
  harness interruption contracts.
- Use one bounded foreground command as the first process model. Add a supervisor
  only after a real workflow proves it necessary.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The state machine may enter a repair-required state, but OSYM-892 owns branch,
provider, review, and merge side effects.
