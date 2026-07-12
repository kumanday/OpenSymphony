---
id: OSYM-883
title: Code Graph Bootstrap UX And End-To-End Validation
milestone: "M12.9: Code Graph View"
priority: 3
estimate: 5
blockedBy: ["OSYM-881", "OSYM-882"]
blocks: []
areas:
  - graph-view
  - desktop
  - web
  - testing
  - documentation
  - release
parent: null
---

## Summary

Expose repository indexing in the product and prove the complete agent and human
Code Graph workflow from an empty database through a packaged desktop build.

## Scope

### In scope

- Add `indexRepo` to the shared Code Graph adapter with fixture, HTTP, and
  Tauri-native implementations.
- Replace the unindexed/empty placeholder with an `Index repository` action,
  progress and coverage status, failure diagnostics, retry, and refresh from
  `code_graph_updated` events.
- Show the selected target revision and whether the current view is baseline,
  workspace-composed, stale, truncated, or partially analyzed.
- Add a deterministic local end-to-end fixture that starts from an empty DuckDB,
  indexes a target branch, creates workspace edits, and exercises indexed agent
  retrieval plus human symbol and topology diffs.
- Verify web and packaged Tauri desktop parity, including the real production
  adapters rather than `?fixtures` only.
- Update desktop, graph-view, code-intelligence, memory, operations, testing, and
  release documentation.
- Complete the post-M12.9 desktop release handoff with version-parity and
  `package:release --dry-run` evidence so the feature reaches installed bundles.

### Out of scope

- Publishing a signed installer for new platforms.
- Hosted repository cloning policy.
- New graph engines or visualization dependencies.

## Deliverables

- Product indexing action and status/error states.
- Empty-database agent and operator end-to-end test.
- Packaged desktop non-placeholder smoke evidence.
- Updated user, operator, testing, and release documentation.

## Acceptance Criteria

- [ ] From an empty database, a local operator can index the configured repository
      and reach a nonempty Code Graph without invoking an admin MCP tool manually.
- [ ] The same indexed baseline is discoverable through `code.graph.context`, and
      a dirty workspace query returns overlay provenance and changed symbols.
- [ ] A workspace that adds a cross-module call renders an added topology edge and
      module-connection delta against the configured target branch.
- [ ] HTTP and native adapters return equivalent index, snapshot, overlay, and
      topology-delta results.
- [ ] The packaged desktop uses production adapters and contains no M12.9
      follow-on placeholder for the Code Graph surface.
- [ ] Version metadata stays in lock-step and the desktop release dry-run passes.

## Test Plan

- Add a local end-to-end test covering empty index, target snapshot, agent query,
  dirty workspace overlay, topology delta, event refresh, and cleanup.
- Add UI tests for indexing progress, retry, partial coverage, stale/truncated
  status, and keyboard-accessible empty states.
- Run relevant Rust and TypeScript suites, `cargo clippy-system-duckdb`, the
  packaged desktop smoke, and
  `npm run package:release --workspace=@opensymphony/desktop -- --dry-run`.
- Run default bundled-mode release-sensitive validation before publishing.
- Run `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md`, especially sections 6.6, 8.4, 10,
  12, and 14.
- Inspect production adapter selection in `apps/desktop/src/index.ts` and
  `apps/web/src/main.ts`.
- Reuse OSYM-879 through OSYM-882; do not create another indexing or graph path.
- Follow the desktop version-parity and release rules in `AGENTS.md`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This is the release gate for the M12.9 follow-on wave. Passing fixture-only tests
is insufficient.
