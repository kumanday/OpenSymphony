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

## COE-547 Code Baseline

Build the inherited-evidence inventory from these suites:

- Central configuration and secret/path validation:
  `crates/opensymphony-cli/src/orchestrator_run/config.rs`, especially the
  `central_config_*` tests.
- Migration, interruption, conflict preflight, source generations, activation,
  and rollback: `crates/opensymphony-cli/src/migration.rs`, especially
  `interrupted_catalog_copy_resumes_before_promotion`,
  `migration_preflights_all_legacy_memory_conflicts_before_copying`,
  `migration_rejects_edited_legacy_sources_before_promotion`, and
  `apply_and_rollback_restore_legacy_files`.
- Process-incarnation and runtime-root ownership:
  `crates/opensymphony-cli/src/orchestrator_run/mod.rs`, especially the
  `runtime_root_ownership_*` and `strict_run_marker_*` tests.
- Harness and durable-run recovery:
  `crates/opensymphony-cli/src/orchestrator_run/backends.rs`,
  `crates/opensymphony-openhands/src/session.rs`, and
  `crates/opensymphony-orchestrator/tests/scheduler.rs`, including prepared
  OpenHands/Codex reattachment, retry exhaustion, interrupt acknowledgement,
  tracker reactivation, and retention cases.
- Catalog coordination:
  `crates/opensymphony-cli/src/memory.rs`, especially
  `central_memory_writers_share_a_catalog_coordination_lock`,
  `memory_server_writer_gate_keeps_filesystem_lock_until_guards_drain`, and
  `memory_server_health_reports_pinned_config_generation`.
- Operator projections:
  `crates/opensymphony-cli/src/orchestrator_run/snapshot.rs`,
  `crates/opensymphony-gateway/tests/gateway.rs`, and
  `crates/opensymphony-tui/tests/reducer.rs`.

Treat those tests as inherited subsystem evidence. This task owns the
cross-subsystem, three-repository lifecycle and fault matrix; compose the
existing fixtures and add only missing multi-repository transitions rather than
duplicating the unit matrices.

## Scope

### In scope

- Inventory the COE-547 regression cases and map them to the lifecycle fault
  matrix before adding new fixtures.
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
- Compose the COE-547 migration preflight/apply/activation/rollback fixtures
  into the full lifecycle and verify strict mode remains default-off.
- Add an opt-in disposable live test with bounded credentials, budgets, timeouts,
  unique repositories/issues/ports, and complete teardown evidence.
- Capture release evidence at one immutable commit and config hash before
  enabling one non-production project set.
- Document rollback and staged expansion; do not automatically activate
  production projects.

### Out of scope

- Recreating COE-547's subsystem-level config, migration, marker, retry,
  reattachment, memory-lock, or generic operator-projection test matrices.
- Production activation.
- Hosted multi-tenant isolation.
- Performance tuning without a measured fixture failure.

## Deliverables

- Hermetic multi-repository lifecycle test harness and a fault matrix that maps
  inherited COE-547 evidence plus remaining cross-subsystem gaps.
- Composed migration/rollback and negative-scope integration suite.
- Opt-in bounded live-test script and teardown report.
- Release checklist, evidence template, and isolated rollout runbook.
- Updated testing, operations, and implementation-plan documentation.

## Acceptance Criteria

- [ ] The hermetic scenario completes the full three-repository child, parent,
      repair, memory, verification, and cleanup lifecycle with no external
      services.
- [ ] Restart at every numbered boundary reaches the same result without
      duplicate side effects or lost leases/evidence.
- [ ] The fault matrix cites each inherited COE-547 regression and adds new
      coverage only where the complete multi-repository lifecycle has an
      uncovered transition or interaction.
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

- Inventory and run the inherited COE-547 subsystem regressions, then run the
  hermetic suite with systematic restart and cross-subsystem fault injection.
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
- Build the inherited-evidence map from the named suites before writing the
  hermetic harness.
- After COE-547 closeout is indexed, `memory.context` scoped to issue `COE-547`
  and areas `testing`, `operations`, `configuration`, `memory`, and
  `orchestrator` may help recover rationale and evidence links; verify them
  against the named source and tests.
- The delegated fork and its PR #20 live harness are reference evidence only;
  they are not sufficient release validation.

## Definition of Ready

- [x] The COE-547 inherited evidence and remaining hermetic lifecycle boundary
      are explicit.
- [x] Required fixtures, docs, and release evidence are explicitly referenced.
- [ ] OSYM-894 is merged and the complete implemented state/event surface is
      frozen for the release candidate.

## Notes

The hermetic suite is the release gate. The live test confirms integration with
real systems only after deterministic behavior is already proven.
The fixture inventory is part of the deliverable: inherited subsystem evidence
must remain traceable without being copied or silently reauthored.
