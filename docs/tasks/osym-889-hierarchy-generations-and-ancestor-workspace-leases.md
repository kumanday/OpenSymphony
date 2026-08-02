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

Layer versioned hierarchy reconciliation and durable owner-identified leases
onto the verified checkout-generation lifecycle. Replace the unconditional
parent skip in `Scheduler::dispatch_ready_issues` with provider-backed,
restart-safe eligibility and one-time dispatch.

## Existing Code Baseline

Build the hierarchy and lease model through these existing ownership points:

- `crates/opensymphony-workspace/src/models.rs` already identifies a leaf
  through `CheckoutManifest::generation`,
  `WorkspaceHandle::checkout_generation`, `TerminalRuntimeEnvelope`, and
  `RunManifest::runtime_envelope`. Hierarchy generation is a separate version
  axis; do not conflate it with config, inventory, policy, or checkout
  generations or introduce another checkout identity.
- `crates/opensymphony-workspace/src/manager.rs` owns
  `ensure_verified_checkout`, `verify_checkout_for_retry`,
  `verify_runtime_envelope_for_retry`, `list_all_workspaces`,
  `find_verified_workspace_by_issue_reference`,
  `recover_abandoned_staging_checkouts`, and `write_json_artifact_atomically`.
  Lease-aware retention and deletion must preserve its ownership validation,
  Git provenance, atomic writes, incomplete-generation recovery, and malformed
  generation quarantine.
- `crates/opensymphony-orchestrator/src/scheduler.rs::{load_recovery_state,
  bootstrap_recovery, dispatch_ready_issues}` owns recovery and dispatch and
  already detects immutable repository-binding changes through
  `RepositoryBindingOutcome::binding_changed_opt` and supersedes incompatible
  runs. Rebuild leases before `bootstrap_recovery` can reach terminal cleanup,
  and compose hierarchy generations and late-event rejection with that
  scheduler-owned state transition path.
- `crates/opensymphony-orchestrator/src/selection.rs::parent_issue_blocked_by_incomplete_children`
  already handles incomplete descendants. Preserve that selector; the missing
  behavior is the scheduler's later unconditional parent skip.
- `crates/opensymphony-linear/src/client.rs` owns configured-project scans,
  latest issue-state reconciliation, and identifier lookup for tracked issues
  outside the current project scan. Parent eligibility should consume that
  provider truth instead of adding a second tracker cache.
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` owns
  `RuntimeWorkspaceBackend::recover_workspaces` and
  `cleanup_workspace_with_policy`, including runtime-envelope verification,
  retry-exhaustion handling, hooks, harness archival, and delegation to
  `WorkspaceManager`. Preserve
  `retained_terminal_cleanup_keeps_openhands_conversation_active`:
  conversation archival follows authorized workspace removal, not terminal
  tracker state alone.

Preserve `verified_checkout_is_atomic_repository_local_and_quarantines_drift`
and the existing terminal-retention regressions. They are the leaf lifecycle
substrate, not the hierarchy-generation or lease model this task owns.

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
- Key each leased resource by the existing issue ID, canonical repository ID,
  and checkout generation; do not persist arbitrary checkout paths in hierarchy
  state.
- Persist lease owners and hierarchy generation beside the existing checkout
  and runtime envelope, and perform receipt updates through
  `WorkspaceManager::write_json_artifact_atomically`.
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
- Keep retained checkout and conversation evidence aligned until lease-aware
  cleanup authorizes removal.
- Prevent ordinary, failed, retry-exhausted, recovery, and forced terminal
  cleanup while an applicable lease exists; after release, resume the existing
  cleanup chain unchanged.

### Out of scope

- Parent execution roots, child-checkout maps, and integration worktrees
  (OSYM-890).
- Parent controller execution and integration checks (OSYM-891).
- Repair pull requests and provider lifecycle (OSYM-892).
- Bottom-up subtree cleanup, tombstones, and recovery sweeps (OSYM-893).
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
- [ ] Lease records identify the existing issue, canonical repository, and
      checkout generation without storing an alternate checkout identity or
      arbitrary path.
- [ ] Retaining a leased checkout retains its conversation evidence; archival
      occurs only when lease-aware workspace removal is authorized.
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
- Preserve existing verified-checkout, quarantine, recovery, runtime-envelope,
  binding-supersession, and cleanup regressions. Add lease behavior primarily
  in hierarchy-selection and scheduler tests instead of duplicating checkout
  lifecycle fixtures or redesigning `WorkspaceManager` cleanup.
- Run focused orchestrator and workspace tests, `cargo clippy-system-duckdb`,
  and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 9.5 through 9.6,
  12, 13.1, 15.3, and 16.
- Inspect `crates/opensymphony-orchestrator/src/{selection,scheduler}.rs`,
  `crates/opensymphony-orchestrator/tests/hierarchy_selection.rs`, and
  `crates/opensymphony-workspace/src/{manager,models}.rs`, plus
  `crates/opensymphony-linear/src/client.rs` and terminal cleanup in
  `crates/opensymphony-cli/src/orchestrator_run/backends.rs`.
- The orchestrator remains the sole owner of scheduling and parent lifecycle
  state.
- Trace the named leaf checkout, scheduler, provider, and retention paths before
  introducing hierarchy or lease persistence.

## Definition of Ready

- [x] The landed leaf checkout, envelope, quarantine, recovery, and retention
      baseline is explicit.
- [x] The remaining hierarchy-generation, eligibility, and durable-lease work
      is measurable.
- [x] Parent workspace, controller, repair, and subtree-cleanup ownership is
      assigned to OSYM-890 through OSYM-893.

## Notes

Do not implement leases as an anonymous reference count. Recovery and operator
diagnostics require durable owner identity.
