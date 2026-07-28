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

- Storage quotas or automatic diagnostic-hold expiry policy tuning.
- Hosted remote workspace deletion.
- Cleaning unrelated legacy workspaces during migration.

## Deliverables

- Cleanup-intent, eligibility, receipt, retry, and tombstone persistence.
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
- [ ] A missing path is accepted only with a matching generation tombstone.
- [ ] Failed and canceled runs follow explicit retention policy.
- [ ] No worker or scheduler backend calls direct recursive deletion.

## Test Plan

- Add temporary-worktree tests for ordered removal, active leases, higher
  ancestors, hook-once behavior, missing paths, and generation mismatch.
- Inject failure before/after each cleanup receipt and tombstone write.
- Add failed/canceled/diagnostic-hold cases and disk-permission failures.
- Run focused workspace and orchestrator tests, `cargo fmt --check`,
  `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 12, 13.8, 14.4,
  15.3, 16, and 21.5.
- Inspect `crates/opensymphony-workspace/src/{manager,models}.rs` and every
  `remove_terminal_workspaces`/direct-deletion caller before changing cleanup.
- Preserve the Codex thread-retention and archive behavior delivered by
  OSYM-877 and OSYM-878.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Cleanup is the final state-machine phase, not an eager child-worker action.
