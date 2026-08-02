---
id: OSYM-887
title: Per-Instance Memory Catalog And Source Migration
milestone: "M12.95: Multi-Repository Foundations"
priority: 2
estimate: 13
blockedBy: ["OSYM-885"]
blocks: ["OSYM-888"]
areas:
  - memory
  - mcp
  - configuration
parent: null
---

## Summary

Evolve the completed repository-rooted memory server into one per-orchestrator
catalog and MCP service that registers repository-owned sources by canonical
identity without recreating the existing memory or code-index engines.

## COE-547 Code Baseline

Extend these code ownership points:

- `crates/opensymphony-cli/src/orchestrator_run/config.rs` resolves
  `CentralMemoryFile`, `ResolvedCentralConfig::memory_catalog_root`, and the
  corresponding `RunRuntimeConfig` fields. Keep one configured catalog root and
  extend it with registered canonical sources.
- `crates/opensymphony-cli/src/memory.rs` owns `MemoryCoordinationLock`,
  `MemoryWriterGate`, `acquire_memory_writer_guard`, supervised `run_serve`
  shutdown, stale-owner recovery, and pinned `config_generation` health.
  Extend this service rather than starting one server or lock domain per
  repository.
- `crates/opensymphony-cli/src/migration.rs` owns
  `resume_in_progress_catalog_copy`, `acquire_partial_apply_catalog_guard`,
  `memory_catalog_generation`, `preserve_legacy_memory`, and rollback
  verification. Source registration must follow that staged/resumable model;
  it must not duplicate the literal directory-copy implementation.
- `crates/opensymphony-memory/src/{lib,config,okf}.rs` remain the DuckDB,
  Markdown, scope-filter, and OKF implementation to evolve with canonical
  source identity.

Preserve the regressions
`central_memory_writers_share_a_catalog_coordination_lock`,
`memory_server_writer_gate_keeps_filesystem_lock_until_guards_drain`,
`memory_server_health_reports_pinned_config_generation`,
`interrupted_catalog_copy_resumes_before_promotion`,
`migration_preflights_all_legacy_memory_conflicts_before_copying`, and
`partial_apply_catalog_guard_rejects_post_migration_captures`. They protect the
existing store but do not provide the multi-repository source registry,
`scope_refs`, or canonical repository provenance this task owns.

## Scope

### In scope

- Extend the COE-547 central catalog and supervised MCP lifecycle from one
  migrated repository store to an instance-wide multi-repository service.
- Add durable `scope_refs` for instance, project set, Linear project, milestone,
  work item, canonical repository, code path, and area; allow records with zero,
  one, or several repository refs.
- Register repository-local memory policy, public docs, and portable OKF bundles
  as repository-owned sources identified by canonical repository ID and commit.
- Keep private issue, parent, and cross-repository records in the central
  catalog while preserving repository-owned public artifacts.
- Import or register existing `.opensymphony/memory` stores with source
  provenance and resumable registration status, reusing COE-547 conflict and
  source-generation checks, with no authoritative dual writes.
- Keep normal worker and CLI access on one injected MCP endpoint; preserve
  direct file/database access only as an explicit offline administrative path.
- Reuse the current DuckDB catalog, Markdown/OKF support, MCP tools, Tree-sitter
  provider, target-branch snapshots, and workspace-overlay machinery.
- Resolve persisted code-intelligence snapshots by canonical repository ID and
  exact commit instead of treating a local root path as repository identity.
- Keep migrations on startup/write paths and normal reads non-mutating.

### Out of scope

- Reimplementing COE-547 catalog-root selection, generic writer locking,
  graceful shutdown, or literal legacy-store copy/rollback behavior.
- New vector databases, fusion retrievers, or storage-provider abstractions.
- Per-run authorization and live-overlay access rules.
- Hosted tenant identity or remote object storage.

## Deliverables

- Multi-repository extensions to the existing per-instance catalog and
  supervised service lifecycle.
- Scope-reference and registered-source persistence.
- Repository-source registration and migration recovery layered on the
  COE-547 store-copy baseline.
- Canonical-ID code-snapshot resolution.
- Compatibility, migration, restart, and concurrent-read tests.

## Acceptance Criteria

- [ ] One OpenSymphony instance exposes one memory MCP endpoint regardless of
      how many inventory repositories it manages.
- [ ] Repo-neutral project or parent records and records referencing several
      repositories coexist with single-repository issue records.
- [ ] Repository memory policy, public docs, and OKF stay repository-owned while
      private runtime records live under the instance state root.
- [ ] Existing repository stores migrate or register once with provenance and
      never receive concurrent authoritative writes from both old and new paths.
- [ ] COE-547 catalog locking, shutdown, interrupted-copy, repeat-apply, and
      rollback regressions remain green while multiple canonical repository
      sources are registered.
- [ ] Persisted code snapshots are addressed by canonical repository ID and
      commit, not a caller-supplied local path.
- [ ] Ordinary MCP reads do not mutate DuckDB schema state.
- [ ] The existing COE-448 memory commands and M12.9 code-index behavior remain
      compatible for one registered repository.

## Test Plan

- Add fixtures with one project set, two projects, three repositories,
  repo-neutral memory, multi-repository records, and existing repo-local stores.
- Extend the COE-547 migration fixtures with source registration, provenance,
  resumable status, conflicts between canonical sources, and prohibition of
  dual writes; do not duplicate its literal store-copy matrix.
- Run parallel MCP reads during capture/reindex and assert one writer owns
  migrations without lock leakage.
- Run `cargo test-system-duckdb --test memory`, focused orchestrator-run tests,
  `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 8 and 18.5.
- Treat completed COE-448 and
  `docs/tasks/multi-repo-memory-server-with-code-intelligence.md` as behavior to
  evolve, not a reason to add duplicate provider layers.
- Reuse the target snapshots and overlays delivered by OSYM-879 through
  OSYM-881.
- Inspect `crates/opensymphony-memory/src/{config,index,query,okf}.rs`,
  `crates/opensymphony-cli/src/memory.rs`, and
  `crates/opensymphony-cli/src/orchestrator_run/{config,mod}.rs`.
- Trace the named catalog, migration, and shutdown symbols and tests before
  extending their lifecycle.
- After COE-547 closeout is indexed, `memory.context` scoped to issue `COE-547`
  and areas `memory` and `configuration` may supply provenance and rationale;
  verify it against the named source and tests.

## Definition of Ready

- [x] The COE-547 catalog/migration baseline and remaining source-registry
      boundary are explicit.
- [x] Required files, docs, and multi-repository memory deliverables are
      explicitly referenced.
- [ ] OSYM-885 is merged and its canonical repository identity is available to
      source registration and snapshot lookup.

## Notes

Repository is a memory facet and source owner, not the catalog root. Avoid the
speculative Qdrant/vector/provider work from the older plan.
Do not treat the COE-547 bulk-copy destination as repository identity; this task
must replace that temporary migration association with durable canonical source
provenance.
