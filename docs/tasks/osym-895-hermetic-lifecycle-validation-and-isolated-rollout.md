---
id: OSYM-895
title: Hermetic Lifecycle Validation And Isolated Rollout
milestone: "M12.97: Multi-Repository Operations And Rollout"
priority: 2
estimate: 13
blockedBy: ["OSYM-894"]
blocks: []
areas:
  - testing
  - operations
  - documentation
  - release
parent: null
---

## Summary

Prove the complete multi-repository lifecycle with deterministic local fixtures,
systematic restart/fault injection, migration rollback, and one bounded
non-production live rollout before strict mode can be activated.

## Scope

### In scope

- Build a hermetic fixture with at least three local bare repositories,
  contradictory repository instructions, fake Linear hierarchy, fake provider,
  fake harness, isolated memory catalog, and unique local resources.
- Route and execute terminal children, create provider-confirmed merge results,
  retain checkout generations, freeze parent hierarchy, acquire ancestor leases,
  and prepare contained parent integration worktrees without network clones.
- Run bounded cross-repository checks, exercise persisted and live memory scopes,
  seed one integration defect, and complete one requested-change repair/merge
  loop in only the affected repository.
- Refresh every repository, run final verification, remove integration
  worktrees, release leases bottom-up, and delete the complete eligible subtree.
- Inject restart before and after every durable transition and external side
  effect; prove convergence without duplicate branches, pull requests, reviews,
  merges, captures, or cleanup.
- Add negative fixtures for every binding outcome, wrong remote, dirty state,
  stale handles, hierarchy mutation, missing merge evidence, provider outage,
  check/review failure, memory scope widening, process timeout, and cleanup
  failure.
- Test migration preflight/apply/activation/rollback from legacy mode and verify
  strict mode remains default-off.
- Add an opt-in disposable live test with bounded credentials, budgets, timeouts,
  unique repositories/issues/ports, and complete teardown evidence.
- Capture release evidence at one immutable commit and config hash before
  enabling one non-production project set.
- Document rollback and staged expansion; do not automatically activate
  production projects.

### Out of scope

- Production activation.
- Hosted multi-tenant isolation.
- Performance tuning without a measured fixture failure.

## Deliverables

- Hermetic multi-repository lifecycle test harness and fault matrix.
- Migration/rollback and negative-scope test suite.
- Opt-in bounded live-test script and teardown report.
- Release checklist, evidence template, and isolated rollout runbook.
- Updated testing, operations, and implementation-plan documentation.

## Acceptance Criteria

- [ ] The hermetic scenario completes the full three-repository child, parent,
      repair, memory, verification, and cleanup lifecycle with no external
      services.
- [ ] Restart at every numbered boundary reaches the same result without
      duplicate side effects or lost leases/evidence.
- [ ] Every binding, workspace, merge, memory, hierarchy, process, provider, and
      cleanup negative case reaches the specified typed state.
- [ ] The fixture proves no fresh parent clone, no cross-repository instruction
      leak, no unauthorized live overlay, and no cleanup under active lease.
- [ ] Legacy mode remains green and migration rollback restores its prior config
      and workspace behavior.
- [ ] The live test is opt-in, bounded, uniquely named, and leaves no repository,
      issue, branch, pull request, process, port, credential, or workspace behind.
- [ ] Strict mode activates first for one explicit non-production project set
      only after evidence is captured at one commit/config hash.
- [ ] Production expansion remains a separate operator decision.

## Test Plan

- Run the hermetic suite with systematic restart and fault injection.
- Run focused config, Linear, workspace, orchestrator, provider, harness, memory,
  gateway, TUI, web, and desktop tests.
- Run `cargo fmt --check`, `cargo clippy-system-duckdb`, relevant
  `cargo test-system-duckdb` suites, TypeScript tests/type checks, and
  `git diff --check`.
- Before release-sensitive rollout, also run default bundled-mode clippy/tests
  and the opt-in disposable live scenario.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 20 through 24 and
  its complete acceptance criteria.
- Reuse existing testkit fake tracker/server patterns and the M12.9 temporary Git
  snapshot/overlay fixtures.
- The delegated fork and its PR #20 live harness are reference evidence only;
  they are not sufficient release validation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The hermetic suite is the release gate. The live test confirms integration with
real systems only after deterministic behavior is already proven.
