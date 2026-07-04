---
id: OSYM-872
title: Code Graph DTOs Gateway Routes And Native Commands
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-871"]
blocks: ["OSYM-873", "OSYM-874", "OSYM-875", "OSYM-876"]
areas:
  - gateway
  - desktop
  - graph-view
parent: null
---

## Summary

Define and expose the versioned Code Graph read contracts for web and desktop clients, including HTTP routes, event journal updates, and Tauri-native command parity.

## Scope

### In scope

- Add `packages/gateway-schema/src/code_graph.ts` DTOs for repo list, graph snapshots, symbol detail, file outline, diff overlay shape, index reports, update events, truncation, freshness, and confidence.
- Expose `GET /api/v1/code/repos`, code graph snapshots, symbol detail, run-scoped file outline, and repo indexing routes.
- Emit `code_graph_updated` through the existing event journal envelope after ingest or reindex.
- Add Tauri command mirrors for code repo list, graph snapshot, symbol detail, run outline, and repo indexing.
- Enforce workspace-relative paths, visibility filtering, stale handling, and hosted snippet redaction at the boundary.

### Out of scope

- Diff overlay computation and Run Detail summary strip behavior.
- Code Graph frontend rendering.
- New agent-facing MCP tools.

## Deliverables

- Shared TypeScript gateway-schema DTO module and schema tests.
- Gateway read routes backed by the OSYM-871 read model.
- Tauri native command mirrors using the same DTOs.
- Contract tests for redaction, visibility, stale inclusion, truncation, and event cursors.

## Acceptance Criteria

- [ ] Web clients can list indexed repos and request Atlas, File, and Neighborhood snapshots through schema-versioned DTOs.
- [ ] Desktop native commands return the same DTO shapes as HTTP routes for the same fixtures.
- [ ] Run-scoped outline responses include `symbol_key`, spans, selection spans, kind, path, and container chain.
- [ ] No DTO includes absolute paths, `workspace_path`, or hosted-forbidden snippets.
- [ ] `code_graph_updated` cursors are monotonic and partitioned by repo.
- [ ] Stale records are excluded by default or visibly marked when explicitly included.

## Test Plan

- Add DTO serialization and schema-version tests.
- Add gateway route contract tests with code-intelligence fixtures.
- Add native command parity tests or fixture assertions where desktop commands are tested today.
- Run focused gateway-schema, gateway, and desktop command tests.
- Run `git diff --check`.

## Context

- Read `docs/specs/code-graph-view-spec.md` section 8.
- Inspect `packages/gateway-schema/src/memory_graph.ts`.
- Inspect `crates/opensymphony-gateway/src/lib.rs` and existing run diff routes.
- Inspect `apps/desktop/src-tauri/src/commands.rs`.
- The run-scoped endpoints must resolve worktrees server-side; clients never receive workspace roots.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Define the diff overlay DTO now, but leave overlay computation to OSYM-874.
