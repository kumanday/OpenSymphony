---
id: OSYM-879
title: Target Branch Code Index And Revision Snapshots
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: []
blocks: ["OSYM-880"]
areas:
  - code-intelligence
  - memory
  - gateway
  - workflow
parent: null
---

## Summary

Turn the existing Code Graph index-report stub into a real bounded indexing
operation that builds immutable whole-repository snapshots for the configured
target branch.

## Scope

### In scope

- Resolve the repository and target branch server-side from workflow and run
  configuration; do not accept arbitrary client filesystem roots.
- Crawl supported source files with the existing containment, skipped-directory,
  language, size, match, and capture limits.
- Parse and persist documents, symbols, edges, diagnostics, and skipped-file
  coverage in bounded batches through the existing Tree-sitter and DuckDB paths.
- Store complete immutable snapshot membership keyed by repository and commit so
  re-indexing identical content at a later commit cannot erase an older revision.
- Make `POST /api/v1/code/repos/{repo_id}/index` and `code_index_repo` perform
  real indexing and return accepted, progress, completed, unavailable, and
  failure diagnostics through the existing contract and event journal.
- Incrementally advance the target-branch snapshot from the prior indexed commit
  by parsing changed paths and recording deletions instead of rescanning unchanged
  files.
- Serialize DuckDB index writes through one owner and keep reads available during
  background indexing.

### Out of scope

- Always-on filesystem watching.
- Hosted remote repository cloning or arbitrary path indexing.
- Compiler, LSP, or type-resolution integration.

## Deliverables

- Working repository-index job behind the gateway and native command.
- Revision-safe DuckDB snapshot schema and migration.
- Progress and completion events with parsed, persisted, skipped, stale, and
  failed counts.
- Updated Code Graph, memory, gateway, configuration, and operations docs.

## Acceptance Criteria

- [ ] Starting with an empty memory DuckDB, indexing the configured repository
      produces nonzero current documents, symbols, and edges for supported files.
- [ ] The indexed revision is the configured target branch commit, not a
      hard-coded `main`, `master`, or `origin/HEAD` choice.
- [ ] Base and later target-branch snapshots remain independently queryable even
      when unchanged files have identical content hashes.
- [ ] A second index after one changed and one deleted file parses only affected
      paths and records correct snapshot membership.
- [ ] Index requests cannot escape the configured repository and do not execute
      target-repository code.
- [ ] Concurrent index requests cannot corrupt DuckDB or create two writers.

## Test Plan

- Add memory tests for empty bootstrap, immutable revisions, identical-content
  commits, changed files, deleted files, skipped coverage, and interrupted jobs.
- Add gateway and Tauri command tests for accepted/progress/completed/error parity.
- Add configured-target-branch regression coverage using a repository where
  `develop` and `main` point at different commits.
- Run `cargo fmt --check`, `cargo clippy-system-duckdb`, focused memory/gateway
  tests, `cargo test-system-duckdb --test memory`, and `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` sections 6.6, 8.4, 10, and 12.
- Inspect `crates/opensymphony-cli/src/memory.rs` persistent ingest handling.
- Inspect `crates/opensymphony-memory/src/index.rs` revision and freshness tables.
- Inspect `crates/opensymphony-memory/src/code_graph.rs::code_graph_index_report`.
- Inspect the configured target-branch model delivered by OSYM-856 through
  OSYM-859.
- This extends the completed OSYM-871 and OSYM-872 read-model and gateway work.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The persistent index is a shared target-branch baseline. It must not claim to be
the live truth for an issue workspace; OSYM-880 adds that isolated overlay.
