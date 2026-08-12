---
id: OSYM-893
title: Bottom-Up Subtree Cleanup And Recovery
milestone: "M12.96: Parent Integration Lifecycle"
priority: 2
estimate: 8
blockedBy: ["OSYM-892"]
blocks: ["OSYM-894"]
areas:
  - workspace-lifecycle
  - orchestrator
  - recovery
parent: null
---

## Summary

Finalize parent runs through durable, lease-aware, Git-aware bottom-up cleanup
that survives partial failure and preserves any checkout still needed by a
higher ancestor or diagnostic hold.

## Landed Leaf Cleanup And Recovery Baseline

Begin at these cleanup and recovery boundaries:

- `crates/opensymphony-workspace/src/manager.rs` owns `cleanup`,
  `cleanup_failed_terminal_workspace`, and
  `cleanup_with_terminal_removal`. The last function currently performs the
  final `remove_dir_all`; place lease, worktree, receipt, and tombstone
  eligibility ahead of that destructive boundary. Its verified-generation
  ownership checks, malformed/drifted checkout quarantine, staging recovery,
  and atomic artifact writes must remain the only leaf removal substrate.
- `crates/opensymphony-workspace/src/models.rs` owns `CleanupConfig`,
  `CleanupDecision`, `CleanupOutcome`, hook receipts, and `RunManifest`. Extend
  these durable records for generation-specific cleanup intent rather than
  creating an unrelated cleanup store.
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` owns
  `cleanup_workspace_with_policy`, failed-workspace cleanup, retry-exhaustion
  persistence, recovery from existing run manifests, strict conversation
  archival ordering, and retained-generation reconciliation.
- `crates/opensymphony-orchestrator/src/scheduler.rs` owns retry exhaustion,
  tracker-reactivation, interrupt acknowledgement, terminal recovery, and
  project/binding supersession. It fences the harness before revoking
  issue-scoped resources or authorizing cleanup. Parent cleanup consumes those
  decisions; it must not redefine them.
- `crates/opensymphony-domain/src/state_machine.rs` and
  `crates/opensymphony-cli/src/orchestrator_run/snapshot.rs` preserve release
  reasons and tracker-terminal precedence.

Preserve the regressions
`retry_exhausted_cleanup_policy_survives_terminal_transition`,
`inactive_retry_exhaustion_retries_failed_workspace_cleanup`,
`failed_terminal_interrupt_is_retried_before_cleanup`,
`terminal_recovery_honors_failed_workspace_retention`,
`retained_terminal_cleanup_keeps_openhands_conversation_active`,
`project_scope_drift_supersedes_the_run_without_removing_the_checkout`,
`retry_exhausted_release_preserves_explicit_reason`, and
`terminal_tracker_state_overrides_a_failed_worker_outcome`. They describe one
issue workspace; this task adds ancestor leases, integration worktrees,
bottom-up release, generation tombstones, and subtree cleanup intent.

## Adjacent Task Boundaries

- OSYM-889 owns lease creation, ownership, and hierarchy generations; OSYM-890
  owns parent-root/worktree creation; OSYM-891 owns final verification and
  cleanup-readiness evidence; OSYM-892 owns repair/provider completion. This
  task consumes their terminal records and does not redefine them.
- This task owns cleanup intent, eligibility, ordered lease release, Git-aware
  worktree removal, hook/deletion receipts, tombstones, retries, and subtree
  recovery.
- OSYM-894 owns read-only cleanup/operator projections and support bundles.
  OSYM-895 owns systematic cross-subsystem restart/fault coverage and rollout.
  Review feedback for UI parity, release harnesses, quotas, or hosted deletion
  belongs outside this slice.

## Scope

### In scope

- Record cleanup intent only after required repairs merge, all repositories
  refresh, final verification passes, and durable evidence is written.
- Compute deletion eligibility from owner-identified leases, provider
  operations, repair state, retry state, retention policy, and generation
  identity.
- Remove parent-owned integration worktrees with Git-aware workspace operations
  before deleting their non-Git parent root.
- Release parent and descendant leases bottom-up while preserving higher-
  ancestor owners.
- Execute `before_remove` once per generation intent and persist its receipt.
- Treat hook, permission, worktree removal, filesystem deletion, and tombstone
  writes as retryable cleanup states without releasing unrelated leases.
- Resume only remaining cleanup work after restart.
- Count an already missing path as success only when a matching generation
  tombstone proves the intended prior deletion.
- Apply explicit failed/canceled retention policy and bounded diagnostic holds.
- Ensure scheduler backends and workers never delete workspaces directly.

### Out of scope

- Redesigning the landed retry accounting, tracker-completion semantics,
  interrupt acknowledgement, or generic failed-workspace retention.
- Storage quotas or automatic diagnostic-hold expiry policy tuning.
- Hosted remote workspace deletion.
- Cleaning unrelated legacy workspaces during migration.

## Deliverables

- Parent/subtree cleanup-intent, eligibility, receipt, retry, and tombstone
  persistence layered on the existing single-issue recovery state.
- Git-aware integration-worktree and parent-root removal.
- Bottom-up lease release and higher-ancestor preservation.
- Failed/canceled retention and diagnostic-hold behavior.
- Crash-at-every-boundary cleanup tests.

## Acceptance Criteria

- [ ] No checkout or parent root with an active lease can be deleted.
- [ ] Integration worktrees are removed through Git-aware operations before the
      parent root is deleted.
- [ ] Higher-ancestor leases survive intermediate parent finalization.
- [ ] Hook, permission, worktree, filesystem, or tombstone failure remains
      visible and retryable without losing ownership evidence.
- [ ] Restart resumes only incomplete cleanup steps and does not rerun a
      successfully receipted hook or deletion.
- [ ] Parent cleanup consumes the inherited retry and retention state without
      reclassifying an active Linear issue as completed or duplicating generic
      terminal cleanup.
- [ ] A missing path is accepted only with a matching generation tombstone.
- [ ] Failed and canceled runs follow explicit retention policy.
- [ ] No worker or scheduler backend calls direct recursive deletion.

## Test Plan

- Add temporary-worktree tests for ordered removal, active leases, higher
  ancestors, hook-once behavior, missing paths, and generation mismatch.
- Reuse the landed retry, terminal-state, interrupt, retention, and runtime-root
  regressions as the single-issue baseline.
- Inject failure before/after each cleanup receipt and tombstone write.
- Add failed/canceled/diagnostic-hold cases and disk-permission failures.
- Run focused workspace and orchestrator tests, `cargo fmt --check`,
  `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 12, 13.8, 14.4,
  15.3, 16, and 21.5.
- Inspect `crates/opensymphony-workspace/src/{manager,models}.rs` and every
  `remove_terminal_workspaces`/direct-deletion caller before changing cleanup.
- Trace the named workspace, backend, scheduler, and snapshot symbols and tests
  before adding parent cleanup state.
- Preserve the Codex thread-retention and archive behavior delivered by
  OSYM-877 and OSYM-878.

## Definition of Ready

- [x] The landed single-issue recovery baseline and parent/subtree ownership
      boundary are explicit.
- [x] Required files, docs, and remaining cleanup state are explicitly
      referenced.
- [ ] OSYM-892 is merged and its final parent evidence and repair contract is
      available to cleanup eligibility.

## Notes

Cleanup is the final state-machine phase, not an eager child-worker action.
Generic issue retry and retention are inherited inputs; this task owns only the
lease-aware parent/subtree cleanup protocol.
