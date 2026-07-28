---
id: OSYM-889
title: Hierarchy Generations And Ancestor Workspace Leases
milestone: "M12.96: Parent Integration Lifecycle"
priority: 1
estimate: 8
blockedBy: ["OSYM-885", "OSYM-886"]
blocks: ["OSYM-890"]
areas:
  - orchestrator
  - workspace-lifecycle
  - linear
parent: null
---

## Summary

Replace terminal child cleanup and simple parent deferral with versioned
hierarchy reconciliation, provider-backed parent eligibility, and durable
owner-identified workspace leases.

## Scope

### In scope

- Persist hierarchy generations and the required child-edge snapshot used for
  each parent eligibility decision.
- Reconcile child additions/removals while a parent waits for children or
  required merges.
- Freeze required edges before ancestor lease acquisition; block with
  `HierarchyChanged` instead of silently changing scope after preparation starts.
- Ignore late child events whose hierarchy generation cannot satisfy the active
  parent generation.
- Add owner-identified leaf-worker, review, ancestor-integration, repair, and
  bounded diagnostic leases.
- Acquire all required review and ancestor leases before releasing a terminal
  child's worker lease or making its checkout cleanup-eligible.
- Support several lease owners and deeper ancestors even when the current UI
  exposes a tree.
- Make parent eligibility require terminal orchestrator outcomes, provider-
  confirmed required merges, recorded merge-result commits, retained checkout
  generations, and no unresolved retry/review/check/merge failure.
- Dispatch an eligible parent exactly once instead of retaining permanent
  `ParentDeferred` behavior.
- Rebuild ancestor leases before cleanup reconciliation after restart.

### Out of scope

- Creating the parent execution root.
- Running integration checks or repair pull requests.
- General DAG/shared-descendant product behavior.

## Deliverables

- Versioned hierarchy snapshot and reconciliation events.
- Durable lease records, owners, acquisition/release rules, and blocked reasons.
- Provider-backed parent eligibility and one-time dispatch.
- Restart reconciliation for hierarchy, leases, and terminal child retention.
- Hierarchy mutation, nested-parent, and lease-race tests.

## Acceptance Criteria

- [ ] A terminal child's checkout remains present while any review or ancestor
      lease exists.
- [ ] A parent cannot become eligible from tracker terminal state without
      provider merge evidence and recorded target commits.
- [ ] All required leases exist before the leaf-worker lease is released.
- [ ] Child changes while waiting create a new hierarchy generation and
      recompute eligibility.
- [ ] Child changes after scope freeze block for explicit re-planning without
      releasing retained evidence.
- [ ] Late child events cannot satisfy a newer hierarchy generation.
- [ ] An eligible parent is dispatched exactly once across restart.
- [ ] Higher-ancestor leases survive intermediate parent completion.

## Test Plan

- Extend hierarchy-selection tests for waiting reconciliation, freeze,
  `HierarchyChanged`, stale events, and nested parents.
- Add concurrent lease acquisition/release and crash-boundary tests.
- Add fake-provider eligibility tests for tracker-only completion, missing merge
  evidence, failed checks, and recorded merge-result commits.
- Run focused orchestrator and workspace tests, `cargo clippy-system-duckdb`,
  and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 9.5 through 9.6,
  12, 13.1, 15.3, and 16.
- Inspect `crates/opensymphony-orchestrator/src/{selection,scheduler}.rs`,
  `crates/opensymphony-orchestrator/tests/hierarchy_selection.rs`, and
  `crates/opensymphony-workspace/src/{manager,models}.rs`.
- The orchestrator remains the sole owner of scheduling and parent lifecycle
  state.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not implement leases as an anonymous reference count. Recovery and operator
diagnostics require durable owner identity.
