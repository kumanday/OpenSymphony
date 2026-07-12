---
id: OSYM-881
title: Indexed Agent Code Context And Retrieval
milestone: "M12.9: Code Graph View"
priority: 2
estimate: 5
blockedBy: ["OSYM-880"]
blocks: ["OSYM-883"]
areas:
  - code-intelligence
  - memory
  - mcp
  - workflow
parent: null
---

## Summary

Give development agents bounded read-only discovery over the persistent baseline
and active workspace overlay while preserving targeted live AST parsing as the
fresh source of truth before edits.

## Scope

### In scope

- Add one read-only `code.graph.context` MCP operation accepting repository,
  query/path/symbol selectors, optional run identity, depth, and result limits.
- Search the indexed baseline for symbols, paths, callers, references, related
  tests, diagnostics, and small neighborhoods before the agent knows exact files.
- When a run is supplied, query the OSYM-880 composite graph so branch and
  uncommitted changes replace the shared baseline.
- Return bounded source-cited evidence with repository, base revision, overlay
  digest, path, span, parser/query-pack versions, confidence, and freshness.
- Keep `memory.context --include-code-intel --paths ...` as the canonical live
  revalidation path after file discovery; do not add a duplicate AST CLI family.
- Update workflow guidance so agents use indexed discovery first when needed,
  then surgical live scanning before edits and after touched-file changes.
- Preserve read-only access, path containment, visibility, admin-token, and
  source-snippet policies.

### Out of scope

- Editing, refactoring, or code-generation tools.
- Injecting the full repository graph into an agent prompt.
- LLM-generated dependency or architecture judgments.

## Deliverables

- `code.graph.context` MCP schema, dispatcher, and bounded query implementation.
- Overlay-aware indexed retrieval and provenance rendering.
- Updated agent workflow and code-intelligence documentation.
- Live MCP HTTP and direct-mode parity tests.

## Acceptance Criteria

- [ ] An agent can find a symbol and its callers/tests from the baseline without
      supplying a file path.
- [ ] Supplying a run identity replaces baseline records with that workspace's
      changed-file overlay and labels the result provenance explicitly.
- [ ] A subsequent targeted `memory.context --include-code-intel` call reads the
      current workspace file and does not persist or trust stale indexed text.
- [ ] Result and depth limits prevent whole-repository prompt dumps and report
      truncation.
- [ ] Hosted visibility and snippet policies cannot be widened by tool arguments.
- [ ] No write/edit tool or duplicate `opensymphony code ast` CLI family is added.

## Test Plan

- Add MCP contract tests for baseline search, symbol neighborhood, related tests,
  overlay replacement, truncation, freshness, and visibility.
- Add a live `memory serve` HTTP proof covering `tools/list` and
  `code.graph.context` alongside existing `code.ast.context` behavior.
- Add workflow prompt tests proving indexed discovery and live revalidation are
  described in the correct order.
- Run focused memory MCP tests, `cargo clippy-system-duckdb`,
  `cargo test-system-duckdb --test memory`, and `git diff --check`.

## Context

- Read `docs/code-intelligence.md` and the agent workflow in `WORKFLOW.md`.
- Inspect the existing `memory.context`, `code.ast.*`, and admin ingest dispatch
  in `crates/opensymphony-cli/src/memory.rs`.
- Reuse OSYM-879 baseline queries and OSYM-880 composite graphs.
- Current source and tests remain authoritative over retrieved evidence.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The index is for discovery and unchanged-repository context. Surgical live AST
scanning is intentionally retained for workspace truth.
