---
id: OSYM-890
title: Parent Execution Roots And Child Workspace Reuse
milestone: "M12.96: Parent Integration Lifecycle"
priority: 1
estimate: 13
blockedBy: ["OSYM-886", "OSYM-889"]
blocks: ["OSYM-891"]
areas:
  - workspace-lifecycle
  - orchestrator
  - harness
parent: null
---

## Summary

Build one repository-neutral parent execution root whose contained integration
worktrees reuse retained child Git storage and launch through a generic,
truthfully reported harness execution scope.

## Existing Code Baseline

Extend the landed leaf-generation path instead of introducing alternate
checkout or launch identities:

- `crates/opensymphony-workspace/src/models.rs::{WorkspaceHandle,
  CheckoutManifest, TerminalRuntimeEnvelope}` already bind an issue to a
  verified checkout generation, canonical repository, target commit,
  instruction provenance, review-policy generations, harness/model,
  conversation, requested execution scope, effective containment, and cleanup
  intent.
- `crates/opensymphony-workspace/src/manager.rs` owns verified checkout
  materialization, retry verification, generation quarantine, recovery, and
  atomic artifact writes. Parent checkout handles must resolve through this
  ownership boundary and retain its remote, cleanliness, containment, and
  generation checks.
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` composes leaf
  runtime envelopes and launches OpenHands or Codex with the verified checkout
  as `cwd`. Add the parent-root scope to that shared launch path; do not add a
  parent-only harness backend or reuse mutable tracker paths as checkout
  handles.

The remaining work is the repository-neutral parent root, contained
integration worktrees, and multi-checkout launch envelope. It does not include
another leaf clone, recovery, quarantine, prompt-instruction, or conversation
lifecycle.

## Scope

### In scope

- Create the parent manifest, child-checkout map, integration plan, evidence
  directory, and `repositories/<checkout-handle>/` layout under configured roots.
- Group descendants by canonical repository ID while retaining every child
  generation unchanged for evidence.
- Select one verified Git storage source per repository and create parent-owned
  shared-object worktrees without a fresh network clone.
- Fetch/deepen the configured target branch and reset each integration worktree
  to the recorded provider merge-result target commit.
- For several children in one repository, prove every provider merge-result
  commit is reachable from the selected target commit; do not require replaced
  feature commits after squash or rebase merge.
- Persist generation-bound checkout handles and reject arbitrary paths, stale
  generations, wrong remotes, dirty state, and containment escapes.
- Compose the parent prompt from generic lifecycle policy, optional project-set
  integration instructions, parent acceptance criteria, child-checkout map, and
  repository instructions keyed by canonical ID.
- Launch one parent conversation with the parent root as `cwd` and the complete
  relative checkout map.
- Let harness-native shell and file tools perform implementation/verification;
  require typed checkout/lease/cleanup/provider operations to use handles.
- Record requested execution scope and effective `trusted_host` or
  `workspace_confined` containment for OpenHands, Codex, and future adapters.

### Out of scope

- A command broker or topology-specific repository roles.
- Parent state-machine execution and integration checks.
- Repair branches and pull requests.

## Deliverables

- Parent execution-root and child-checkout-map schemas.
- Shared-object integration-worktree creation and verification.
- Checkout-handle resolver and containment checks.
- Cross-harness parent launch envelope and prompt composition.
- Multi-repository, same-repository, squash-merge, and no-reclone tests.

## Acceptance Criteria

- [ ] A parent spanning at least three repositories gets one non-Git root and one
      verified contained integration worktree per canonical repository.
- [ ] Every child checkout remains unchanged and leased as evidence.
- [ ] Integration worktrees reuse child repository storage and perform no fresh
      clone.
- [ ] Several children in one repository produce one handle at a target commit
      containing every provider merge result, including squash/rebase cases.
- [ ] Stale handles, arbitrary paths, wrong remotes, dirty state, and symlink
      escapes fail before an orchestrator-owned repository operation.
- [ ] The parent prompt has no encoded repository roles and clearly associates
      each instruction source with its canonical repository.
- [ ] OpenHands and Codex receive the same logical execution scope and record
      their actual containment without claiming a prompt-enforced sandbox.

## Test Plan

- Build temporary Git fixtures with three repositories, several children in one
  repository, shallow history, and squash/rebase merge results.
- Assert worktree object sharing, exact target commits, child evidence
  immutability, and zero clone calls during parent preparation.
- Add stale-generation, traversal, symlink, dirty-state, and wrong-remote tests.
- Add OpenHands/Codex parent-envelope and prompt-isolation tests.
- Run focused workspace, workflow, orchestrator-run, OpenHands, and Codex tests
  plus `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 4, 10.3, 13.2
  through 13.6, 14.1, and 19.
- Inspect `crates/opensymphony-workspace`, current worker construction in
  `crates/opensymphony-cli/src/orchestrator_run/backends.rs`, and the harness
  capability boundary in `crates/opensymphony-domain/src/harness.rs`.
- Current trusted-host execution is acceptable only when labeled honestly.

## Definition of Ready

- [x] The verified leaf-generation and harness-envelope substrate is explicit.
- [x] Parent-root, handle, worktree, and launch ownership is measurable.
- [x] Controller and repair side effects remain assigned to OSYM-891 and
      OSYM-892.

## Notes

The parent root is the portability seam for future sandbox/container adapters.
Do not build that hosted isolation in this task.
