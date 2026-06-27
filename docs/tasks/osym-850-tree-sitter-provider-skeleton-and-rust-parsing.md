---
id: OSYM-850
title: Tree-sitter Provider Skeleton And Rust Parsing
milestone: "M12.6: Tree-sitter Code Intelligence"
priority: 2
estimate: 5
blockedBy: []
blocks: ["OSYM-851", "OSYM-852"]
areas:
  - code-intelligence
  - memory
  - rust
parent: null
---

## Summary

Add the internal `opensymphony_code_intel` module tree and prove the first trusted Tree-sitter parsing path with Rust fixtures.

## Scope

### In scope

- Add `crates/opensymphony-code-intel` as an internal module included from the root crate with `#[path = ...]`.
- Pin `tree-sitter`, `tree-sitter-rust`, and only the dependency surface needed for Rust parsing.
- Implement language detection, source identity, one-based source spans, parsed document summaries, and AST diagnostics for Rust.
- Add a Rust definitions query pack and fixture tests for functions, structs, enums, traits, impl methods, tests, and malformed code.

### Out of scope

- TypeScript, JavaScript, Python, or Markdown query packs.
- DuckDB persistence.
- MCP tools and gateway endpoints.
- Full type checking or compiler-backed call graphs.

## Deliverables

- `crates/opensymphony-code-intel/src/` provider skeleton and Rust parser implementation.
- `crates/opensymphony-code-intel/queries/rust/` initial query files and metadata.
- `crates/opensymphony-code-intel/fixtures/rust/` fixture files.
- Root `src/lib.rs` and `Cargo.toml` updates.

## Acceptance Criteria

- [ ] A Rust source file can be parsed without executing target-repo code.
- [ ] Rust fixture tests extract function, struct, enum, trait, method, and test symbols with one-based rendered spans.
- [ ] Tree-sitter `ERROR` and `MISSING` nodes produce diagnostics instead of hard failures.
- [ ] Parser and query-pack versions are included in returned summaries or records.
- [ ] The current heuristic `CodebaseAnalyzer` behavior is untouched.

## Test Plan

- Run `cargo fmt --check`.
- Run focused `opensymphony_code_intel` unit tests with `cargo test-system-duckdb`.
- Run `cargo check-system-duckdb`.

## Context

- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md`.
- Inspect `src/lib.rs` for internal module inclusion style.
- Inspect `Cargo.toml` workspace dependency conventions.
- Inspect `crates/opensymphony-memory/src/lib.rs` for `CodeIntelArtifact`, `CodeIntelIndex`, `MemoryFreshness`, and scope/source reference types.
- Keep trusted built-in grammars only; do not load parser code from target repositories.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Start with Rust only. Add no cache abstraction beyond what the parser tests need.
