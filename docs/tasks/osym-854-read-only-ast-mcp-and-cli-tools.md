---
id: OSYM-854
title: Read-Only AST MCP And CLI Tools
milestone: "M12.6: Tree-sitter Code Intelligence"
priority: 3
estimate: 5
blockedBy: ["OSYM-851", "OSYM-852", "OSYM-853"]
blocks: ["OSYM-855"]
areas:
  - code-intelligence
  - mcp
  - cli
parent: null
---

## Summary

Expose targeted read-only AST exploration through the memory MCP server and optional debug CLI commands.

## Scope

### In scope

- Add `code.ast.status`, `code.ast.outline`, `code.ast.symbols`, `code.ast.references`, `code.ast.query`, `code.ast.context`, and `code.ast.diagnostics` tools when AST code intelligence is enabled.
- Add optional `opensymphony code ast status|outline|query|ingest` debug commands if they reuse the same provider code.
- Enforce path containment, file limits, query limits, capture-size limits, and ad hoc query policy.
- Render traces with paths, line ranges, parser versions, query-pack versions, freshness, truncation, and fallback decisions.

### Out of scope

- Gateway HTTP endpoints.
- Hosted-mode remote query execution beyond existing auth policy.
- Write or refactor tools.

## Deliverables

- MCP tool descriptors and handlers for read-only AST tools.
- CLI debug command coverage where it avoids duplicating MCP logic.
- Tool-contract tests for enabled/disabled states, read access, admin-only ad hoc queries when configured, and limit enforcement.

## Acceptance Criteria

- [ ] AST tools appear in `tools/list` only when code-intelligence AST support is enabled.
- [ ] Read-only tools work with normal read access.
- [ ] Ad hoc query execution respects local/admin policy and deterministic limits.
- [ ] Tool responses cite paths, line ranges, content hashes, parser versions, and query-pack versions.
- [ ] `memory.context` remains the recommended agent kickoff path.

## Test Plan

- Run focused MCP memory server tests.
- Run CLI parser tests for new `opensymphony code ast` commands if added.
- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb --test memory`.

## Context

- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md` section 9.
- Inspect MCP tool descriptor and dispatch code in `crates/opensymphony-cli/src/memory.rs`.
- Reuse existing access-token handling and admin-gated ingestion behavior.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not add write tools. AST context is evidence for agents, not an edit API.
