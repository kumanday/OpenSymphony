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

## Scope

### In scope

- Start one memory catalog and MCP service from the instance state root rather
  than `runtime.target_repo`.
- Add durable `scope_refs` for instance, project set, Linear project, milestone,
  work item, canonical repository, code path, and area; allow records with zero,
  one, or several repository refs.
- Register repository-local memory policy, public docs, and portable OKF bundles
  as repository-owned sources identified by canonical repository ID and commit.
- Keep private issue, parent, and cross-repository records in the central
  catalog while preserving repository-owned public artifacts.
- Import or register existing `.opensymphony/memory` stores with source
  provenance, conflict preflight, resumable status, and no authoritative dual
  writes.
- Keep normal worker and CLI access on one injected MCP endpoint; preserve
  direct file/database access only as an explicit offline administrative path.
- Reuse the current DuckDB catalog, Markdown/OKF support, MCP tools, Tree-sitter
  provider, target-branch snapshots, and workspace-overlay machinery.
- Resolve persisted code-intelligence snapshots by canonical repository ID and
  exact commit instead of treating a local root path as repository identity.
- Keep migrations on startup/write paths and normal reads non-mutating.

### Out of scope

- New vector databases, fusion retrievers, or storage-provider abstractions.
- Per-run authorization and live-overlay access rules.
- Hosted tenant identity or remote object storage.

## Deliverables

- Per-instance memory catalog configuration and supervised service lifecycle.
- Scope-reference and registered-source persistence.
- Repository-store migration/registration command and recovery report.
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
- [ ] Persisted code snapshots are addressed by canonical repository ID and
      commit, not a caller-supplied local path.
- [ ] Ordinary MCP reads do not mutate DuckDB schema state.
- [ ] The existing COE-448 memory commands and M12.9 code-index behavior remain
      compatible for one registered repository.

## Test Plan

- Add fixtures with one project set, two projects, three repositories,
  repo-neutral memory, multi-repository records, and existing repo-local stores.
- Test clean migration, conflicts, interruption, restart, repeat migration, and
  prohibition of dual writes.
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

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Repository is a memory facet and source owner, not the catalog root. Avoid the
speculative Qdrant/vector/provider work from the older plan.
