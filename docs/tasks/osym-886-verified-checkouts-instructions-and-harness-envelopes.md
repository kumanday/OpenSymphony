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
- Persist the terminal runtime envelope: binding, config/inventory generations,
  checkout provenance, instruction hash, target commit, harness/model profile,
  conversation binding, and cleanup intent.
- Launch both current harness adapters with `cwd` equal to the verified checkout
  and record the requested execution scope and effective containment.
- Resume a conversation only when repository, checkout generation, policy, and
  instruction identity remain compatible.
- Route terminal retention and deletion decisions through `WorkspaceManager`.

### Out of scope

- Parent execution roots or multi-checkout harness scopes.
- Cross-repository integration checks.
- Hosted workspace isolation.

## Deliverables

- Typed checkout-generation and verification operations.
- Durable provenance and terminal runtime-envelope schemas.
- Repository-specific instruction loader and prompt composition.
- OpenHands and Codex launch/reattach integration.
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
- [ ] OpenHands and Codex record their effective containment without claiming
      sandboxing in the current trusted-host profiles.
- [ ] No backend deletes a terminal workspace directly.

## Test Plan

- Add temporary-repository tests for staged creation, retry, wrong remotes,
  shallow history, target branches, dirty state, symlink escapes, and quarantine.
- Add prompt tests with contradictory repository instructions and native nested
  `AGENTS.md` discovery metadata.
- Add OpenHands and Codex launch/resume tests for `cwd`, envelope compatibility,
  conversation reuse, and containment receipts.
- Run focused workspace, workflow, orchestrator-run, OpenHands, and Codex tests,
  `cargo fmt --check`, `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 10, 11, 15, 16,
  and 19.
- Inspect `crates/opensymphony-workspace/src/{manager,models,paths}.rs`,
  `crates/opensymphony-cli/src/orchestrator_run/{backends,mod}.rs`,
  `crates/opensymphony-openhands/src/session.rs`, and
  `crates/opensymphony-codex/src/lib.rs`.
- Preserve current one-conversation-per-issue behavior when its full envelope is
  compatible.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not add a command broker. Terminal workers need one checkout, so the existing
native harness tools plus truthful containment metadata are sufficient.
