---
id: OSYM-851
title: Memory Context AST Provider Integration
milestone: "M12.6: Tree-sitter Code Intelligence"
priority: 2
estimate: 5
blockedBy: ["OSYM-850"]
blocks: ["OSYM-853", "OSYM-854", "OSYM-855"]
areas:
  - code-intelligence
  - memory
  - cli
parent: null
---

## Summary

Wire the Tree-sitter provider into the existing `CodeIntelIndex` and `memory.context --include-code-intel` path while preserving the current repository-summary fallback.

## Scope

### In scope

- Implement an `AstCodeIntelProvider` adapter for `opensymphony_memory::CodeIntelIndex`.
- Add a minimal composite provider that returns AST artifacts first and falls back to `CodebaseAnalyzer` for unsupported paths or degraded parser state.
- Replace direct `CodebaseAnalyzer::new(repo_root).code_context(...)` calls in memory context code with the composite provider.
- Enforce repo-boundary path resolution, file-size limits, and unsupported-language fallback behavior.
- Include parse/query/fallback trace lines in rendered code-intelligence context.

### Out of scope

- Persisting code documents, symbols, edges, or diagnostics to DuckDB.
- New MCP tool names beyond the existing `memory.context` behavior.
- Gateway code-intelligence endpoints.

## Deliverables

- Composite code-intelligence provider construction used by `opensymphony memory context --include-code-intel`.
- Updated memory CLI and MCP context rendering for AST artifacts.
- Tests covering requested paths, unsupported files, parser degradation, and fallback traces.

## Acceptance Criteria

- [ ] `opensymphony memory context --include-code-intel --paths <rust-file>` renders AST-derived structural evidence.
- [ ] Existing memory context tests still pass.
- [ ] Unsupported files still return useful repository-summary artifacts.
- [ ] Paths outside the configured repo root are rejected.
- [ ] Fallback to `CodebaseAnalyzer` is visible in the trace when used.

## Test Plan

- Run focused memory CLI tests for `--include-code-intel`.
- Run focused MCP `memory.context` tests with `includeCodeIntel=true`.
- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb --test memory`.

## Context

- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md` sections 9, 10, and 15.
- Inspect `crates/opensymphony-cli/src/memory.rs` functions `append_code_intel_context`, `append_code_intel_context_blocking`, and `call_memory_context_tool`.
- Inspect `crates/opensymphony-planning/src/codebase.rs` for the current `CodebaseAnalyzer` `CodeIntelIndex` implementation.
- Do not change the public `memory.context` contract except to enrich the optional code-intelligence section.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep the first integration synchronous behind the existing blocking wrapper. Add broader async architecture only if tests prove it is needed.
