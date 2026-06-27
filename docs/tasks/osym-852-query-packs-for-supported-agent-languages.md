---
id: OSYM-852
title: Query Packs For Supported Agent Languages
milestone: "M12.6: Tree-sitter Code Intelligence"
priority: 3
estimate: 5
blockedBy: ["OSYM-850"]
blocks: ["OSYM-853", "OSYM-854", "OSYM-855"]
areas:
  - code-intelligence
  - query-packs
  - tests
parent: null
---

## Summary

Extend Tree-sitter query-pack coverage beyond Rust to the first agent-useful language set: TypeScript, TSX, JavaScript, JSX, Python, and lightweight config/document files.

## Scope

### In scope

- Add trusted grammar crates for TypeScript/TSX, JavaScript/JSX, and Python.
- Add query-pack metadata loading and capture-name validation.
- Add definitions, imports, calls, tests, docs, locals, injections where practical, and diagnostics query files for supported languages.
- Add fixtures for representative TypeScript React, JavaScript, Python, malformed-code, and import/call/test cases.
- Treat JSON, YAML, TOML, and Markdown as lightweight document/config languages with summaries or fence parsing where cheap.

### Out of scope

- Full semantic resolution across packages.
- WASM or repo-supplied grammar loading.
- HTML script/style injection support unless it falls out naturally from installed grammars.

## Deliverables

- Query packs and metadata under `crates/opensymphony-code-intel/queries/`.
- Fixtures under `crates/opensymphony-code-intel/fixtures/`.
- Query validation tests that fail on invalid node types, invalid field names, and nonstandard capture names.
- Language registry updates.

## Acceptance Criteria

- [ ] Supported source files produce symbols with one-based spans.
- [ ] Import and call captures work for representative Rust, TypeScript, JavaScript, and Python fixtures.
- [ ] Malformed fixtures return diagnostics.
- [ ] Query compilation errors fail tests before runtime.
- [ ] Capture names follow the standard in the spec.

## Test Plan

- Run focused query-pack validation tests.
- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb`.

## Context

- Read `docs/specs/opensymphony_tree_sitter_ast_spec.md` sections 7, 8, and 17.
- Inspect the Rust query-pack shape from OSYM-850 before adding more languages.
- Prefer the shortest useful query per capture; do not chase perfect language-server parity.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Ship conservative syntactic captures. Mark unresolved references as syntactic rather than inventing exact targets.
