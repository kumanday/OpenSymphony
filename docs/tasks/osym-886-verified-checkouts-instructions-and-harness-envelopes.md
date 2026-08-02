---
id: OSYM-886
title: Verified Checkouts Instructions And Harness Envelopes
milestone: "M12.95: Multi-Repository Foundations"
priority: 1
estimate: 13
blockedBy: ["OSYM-885"]
blocks: ["OSYM-888", "OSYM-889", "OSYM-890"]
areas:
  - workspace-lifecycle
  - workflow
  - openhands-runtime
  - codex-runtime
parent: null
---

## Summary

Create immutable, verified checkout generations for bound terminal tasks and
launch OpenHands or Codex with only that checkout's pinned instructions and
runtime envelope.

## COE-547 Code Baseline

Start from these code and test ownership points:

- `crates/opensymphony-cli/src/orchestrator_run/config.rs` owns
  `CentralRepositoryFile`, `CentralInstructionsFile`, `ResolvedCentralConfig`,
  and `RunRuntimeConfig`. Extend their resolved repository and instruction
  identities after OSYM-885 binding; do not add a parallel configuration path.
- `crates/opensymphony-workspace/src/models.rs` owns `WorkspaceHandle`,
  `RunManifest`, and `ConversationManifest`; add checkout generation,
  instruction provenance, binding, and policy compatibility to those durable
  records.
- `crates/opensymphony-cli/src/orchestrator_run/mod.rs` owns runtime-root
  acquisition and process-incarnation marker recovery. Checkout generations
  must compose with that ownership rather than introducing another process-root
  lock.
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` owns recovered run
  selection, persisted retry/interrupt state, Codex turn recovery, and
  OpenHands attach. Gate those paths on the expanded durable envelope instead
  of forking their recovery state machines.
- `crates/opensymphony-openhands/src/session.rs::recover_with_observer` and
  `recovery_baseline_event_ids` are the OpenHands prepared/trigger-pending
  reconciliation boundary.

Preserve the regressions
`central_config_rejects_repository_instruction_symlink_escape`,
`explicit_config_selection_does_not_depend_on_repository_checkout`,
`recover_workspaces_reattaches_ambiguous_prepared_openhands_runs`,
`recover_workspaces_reattaches_prepared_codex_run_with_active_turn`, and
`codex_stdio_worker_recovery_reconciles_without_starting_a_new_turn`.
These are compatibility foundations, not the terminal multi-repository
envelope this task still owns.

## Scope

### In scope

- Derive contained workspace keys and checkout generations from issue/run and
  canonical repository identity without permitting path or Unicode collisions.
- Resolve credentials privately, materialize through staging, check out the
  centrally configured target branch, and publish atomically.
- Verify canonical remote fingerprint, branch, HEAD, Git integrity, history
  depth, cleanliness, generation manifest, and instruction commit before attach.
- Quarantine wrong remote, corrupt, dirty, incomplete, or mismatched generations
  rather than resetting or adopting them silently.
- Run identity-changing checkout operations through typed workspace-manager
  methods; run non-identity lifecycle hooks only after verification.
- Select the configured instruction file, root `AGENTS.md`, or legacy
  `WORKFLOW.md` body after verification and persist path, hash, and source
  commit.
- Compose per-job prompts without instructions from another inventory
  repository.
- Extend the existing run and conversation manifests into the terminal runtime
  envelope: binding, config/inventory generations, checkout provenance,
  instruction hash, target commit, harness/model profile, conversation binding,
  and cleanup intent.
- Launch both current harness adapters with `cwd` equal to the verified checkout
  and record the requested execution scope and effective containment.
- Gate the inherited reattachment paths on repository, checkout generation,
  policy, and instruction compatibility for both harnesses.
- Route terminal retention and deletion decisions through `WorkspaceManager`.

### Out of scope

- Reworking COE-547 central-config migration, generic retry accounting,
  interrupt acknowledgement, or legacy single-repository recovery.
- Parent execution roots or multi-checkout harness scopes.
- Cross-repository integration checks.
- Hosted workspace isolation.

## Deliverables

- Typed checkout-generation and verification operations.
- Durable provenance and terminal runtime-envelope extensions to the existing
  run and conversation manifests.
- Repository-specific instruction loader and prompt composition.
- OpenHands and Codex launch/reattach integration gated by the complete terminal
  envelope.
- Quarantine, crash-recovery, instruction-isolation, and secret-canary tests.

## Acceptance Criteria

- [ ] Each strict terminal task runs with `cwd` equal to the verified checkout
      for its canonical binding.
- [ ] Two repositories with contradictory instruction markers receive only
      their own pinned instructions.
- [ ] An interrupted clone never becomes attachable and retry publishes a new
      generation atomically.
- [ ] Wrong remote, target branch, HEAD, dirty state, instruction commit, or
      generation identity blocks before worker attach.
- [ ] Repository credentials never appear in remotes, manifests, prompts, logs,
      errors, or process-display strings.
- [ ] Restart reuses only a compatible checkout and harness conversation;
      mismatches enter a typed blocked or superseded state.
- [ ] The inherited COE-547 prepared/trigger-pending and retry-recovery
      regressions remain green without duplicating prompts or redefining legacy
      terminal semantics.
- [ ] OpenHands and Codex record their effective containment without claiming
      sandboxing in the current trusted-host profiles.
- [ ] No backend deletes a terminal workspace directly.

## Test Plan

- Add temporary-repository tests for staged creation, retry, wrong remotes,
  shallow history, target branches, dirty state, symlink escapes, and quarantine.
- Add prompt tests with contradictory repository instructions and native nested
  `AGENTS.md` discovery metadata.
- Extend the COE-547 recovery fixtures with OpenHands and Codex launch/resume
  tests for `cwd`, full-envelope compatibility, conversation reuse, mismatch
  quarantine, and containment receipts.
- Run focused workspace, workflow, orchestrator-run, OpenHands, and Codex tests,
  `cargo fmt --check`, `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 10, 11, 15, 16,
  and 19.
- Inspect `crates/opensymphony-workspace/src/{manager,models,paths}.rs`,
  `crates/opensymphony-cli/src/orchestrator_run/{backends,mod}.rs`,
  `crates/opensymphony-openhands/src/session.rs`, and
  `crates/opensymphony-codex/src/lib.rs`.
- Trace the named COE-547 types and regressions before changing run, retry,
  reattachment, marker, or retention state.
- After COE-547 closeout is indexed, `memory.context` scoped to issue `COE-547`
  and areas `workspace-lifecycle`, `workflow`, `openhands-runtime`, and
  `codex-runtime` may supply provenance and rationale; verify it against the
  named source and tests.
- Preserve current one-conversation-per-issue behavior when its full envelope is
  compatible.

## Definition of Ready

- [x] The COE-547 compatibility baseline and non-reimplementation boundary are
      explicit.
- [x] Required files, docs, and remaining terminal-envelope ownership are
      explicitly referenced.
- [ ] OSYM-885 is merged and its canonical binding contract is available to the
      checkout and harness-envelope implementation.

## Notes

Do not add a command broker. Terminal workers need one checkout, so the existing
native harness tools plus truthful containment metadata are sufficient.
Treat a review request to redesign generic legacy recovery as COE-547 follow-up
work unless the terminal envelope cannot be implemented through a narrow
compatibility extension.
