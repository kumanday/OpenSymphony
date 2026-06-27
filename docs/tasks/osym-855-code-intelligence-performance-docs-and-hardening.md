---
id: OSYM-855
title: Code Intelligence Performance Docs And Hardening
milestone: "M12.6: Tree-sitter Code Intelligence"
priority: 3
estimate: 5
blockedBy: ["OSYM-851", "OSYM-852", "OSYM-853", "OSYM-854"]
blocks: []
areas:
  - code-intelligence
  - documentation
  - operations
parent: null
---

## Summary

Finish V1 by bounding resource use, documenting the agent workflow, and proving concurrency and security behavior.

## Scope

### In scope

- Add bounded document and parsed-tree caches only where benchmarks or repeated context calls need them.
- Use `spawn_blocking` and bounded parallelism for parse/query work from async paths.
- Add concurrency, large-file, generated-directory, symlink, path-containment, query-timeout, and oversized-capture tests.
- Add `docs/code-intelligence.md`.
- Update `README.md`, `docs/tasks/multi-repo-memory-server-with-code-intelligence.md`, `docs/memory.md`, `docs/operations.md`, and workflow guidance as needed.
- Record performance evidence for warm and mixed-cache `memory.context --include-code-intel` calls.

### Out of scope

- Filesystem watcher support.
- Hosted WASM grammar sandboxing.
- Compiler or LSP-backed semantic resolution.
- Public documentation that includes private source snippets.

## Deliverables

- Resource-limit and concurrency hardening in the AST provider.
- Security and performance tests.
- Operator and agent documentation for code intelligence.
- Updated README feature note and memory/operations docs.

## Acceptance Criteria

- [ ] Parallel AST context calls do not corrupt parser, tree, or query state.
- [ ] Generated/vendor directories and oversized files are skipped with clear warnings.
- [ ] Security tests cover path containment, symlink escapes, query limits, trusted grammar policy, and no target-code execution.
- [ ] Documentation explains configuration, freshness, MCP tools, CLI usage, and the recommended agent workflow.
- [ ] V1 acceptance criteria from the spec are either met or explicitly deferred with follow-up tasks.

## Test Plan

- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb`.
- Run `cargo clippy-system-duckdb`.
- Run focused benchmark or timing checks for `memory.context --include-code-intel` on representative Rust and TypeScript paths.
- Run `opensymphony memory lint --public-docs` after documentation updates.

## Context

- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md` sections 11, 13, 17, 18, 19, 20, and 23.
- Inspect `docs/memory.md`, `docs/operations.md`, and `docs/tasks/multi-repo-memory-server-with-code-intelligence.md`.
- Keep docs clear that code intelligence is context; current source files and tests remain authoritative.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Add caches only after integration exists. A bounded cache is useful; a custom cache framework is not.
