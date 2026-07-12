---
id: OSYM-880
title: Workspace Code Overlay And Composite Graph
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-879"]
blocks: ["OSYM-881", "OSYM-882"]
areas:
  - code-intelligence
  - memory
  - gateway
  - orchestrator
  - workspace-lifecycle
parent: null
---

## Summary

Compose each issue workspace's live Tree-sitter results over an immutable target-
branch baseline so agents and operators can query branch and uncommitted changes
without creating a second whole-repository index.

## Scope

### In scope

- Resolve the configured target ref and the run's merge-base through the same
  target-branch contract used by workspace diffs and PR operations.
- Enumerate committed, staged, unstaged, untracked, and deleted workspace paths;
  treat rename and move detection as remove plus add in V1.
- Parse only changed and new supported files from the issue workspace, using
  content-hash caching and existing Tree-sitter limits.
- Represent deleted files as overlay tombstones.
- Isolate overlay state by repository, base revision, run/workspace identity, and
  workspace content digest so concurrent issue workspaces cannot mark one
  another stale.
- Build a virtual workspace graph by replacing baseline records for changed
  paths, removing tombstoned paths, adding live records, and re-resolving affected
  edge endpoints against the composed symbol set.
- Update run-scoped outline, graph, and diff-overlay reads to consume the
  composite graph and report unsupported or failed paths honestly.
- Retain or rebuild overlays across process restart while the owning workspace
  remains recoverable, and remove them with workspace lifecycle cleanup.

### Out of scope

- A separate full DuckDB index per workspace.
- Continuous watch-mode parsing.
- Agent-facing retrieval UX and topology-delta rendering.

## Deliverables

- Workspace overlay model and bounded live-scan service.
- Baseline-plus-overlay graph composition and edge re-resolution.
- Run-scoped gateway/native reads backed by the composed graph.
- Workspace concurrency, restart, and cleanup tests.

## Acceptance Criteria

- [ ] Editing an uncommitted supported file changes the run's symbol and edge
      graph without first persisting a fake commit revision.
- [ ] Adding and deleting supported files produces added records and tombstones
      in the composed graph.
- [ ] Unchanged baseline callers and references remain available for blast-radius
      traversal into changed or removed symbols.
- [ ] Two concurrent workspaces based on the same repository produce independent
      overlays and cannot change each other's freshness or results.
- [ ] The comparison base follows the configured target branch and remains pinned
      for a run even when that branch advances later.
- [ ] Unsupported, oversized, and failed paths appear in coverage diagnostics.

## Test Plan

- Add composite-graph tests for modified, added, deleted, untracked, unsupported,
  and failed files.
- Add a two-workspace concurrency fixture with conflicting edits to the same path.
- Add restart/rebuild and workspace-removal cleanup tests.
- Add gateway tests proving a dirty workspace changes the overlay instead of only
  appending paths to `unanalyzed_files`.
- Run `cargo fmt --check`, `cargo clippy-system-duckdb`, focused code-intel,
  memory, gateway, orchestrator, and workspace tests, and `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 10.1 through 10.4.
- Inspect `workspace_comparison_base`, `get_run_code_diff_overlay`, and run-file
  scanning in `crates/opensymphony-gateway/src/lib.rs`.
- Reuse OSYM-879 snapshots and the existing `AstCodeIntelProvider`; do not add a
  second parser or per-workspace database.
- Preserve orchestrator ownership and workspace containment invariants.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Physical storage may remain centralized. Workspace isolation is logical and must
be explicit in every overlay key and query.
