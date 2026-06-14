---
id: OSYM-737
title: DuckDB Prebuilt Developer Build Mode
milestone: "M9.5: Developer Build Acceleration"
priority: 2
estimate: 3
blockedBy: []
blocks: []
areas:
  - build
  - developer-experience
  - memory
parent: null
---

## Summary

Add an opt-in developer build mode that links DuckDB from a downloaded prebuilt native library while keeping bundled DuckDB as the default user and release path. Provide stable cargo aliases and agent instructions so OpenSymphony development can avoid repeatedly compiling DuckDB in hot workspaces.

## Scope

### In scope

- Split the DuckDB dependency into explicit `duckdb-bundled` and `duckdb-prebuilt` Cargo feature modes, with bundled remaining the default.
- Add `cargo check-dev`, `cargo test-dev`, and `cargo clippy-dev` aliases that set `DUCKDB_DOWNLOAD_LIB=1`, disable default features, and enable the prebuilt DuckDB mode.
- Document the developer-mode commands in `AGENTS.md` and local operations/testing documentation.
- Verify the prebuilt mode against focused memory tests and standard developer validation commands.
- Preserve release and `cargo install opensymphony` behavior for users who do not opt into the developer mode.

### Out of scope

- Replacing DuckDB as the memory backend.
- Introducing the future `MemoryCatalog`, `DocumentStore`, or provider-shaped memory architecture.
- Changing memory server APIs, capture semantics, schema layout, or runtime behavior.
- Requiring a system-wide DuckDB installation for normal users.

## Deliverables

- Updated `Cargo.toml` feature declarations and DuckDB dependency configuration.
- Updated `.cargo/config.toml` aliases for `cargo check-dev`, `cargo test-dev`, and `cargo clippy-dev` without setting a repo-wide DuckDB env var.
- Updated `AGENTS.md`, `README.md`, `docs/operations.md`, and `docs/testing-and-operations.md` instructions.
- Validation evidence for prebuilt developer mode and default bundled mode.

## Acceptance Criteria

- [ ] `cargo install opensymphony` and default local builds still use bundled DuckDB without requiring a system DuckDB install.
- [ ] `DUCKDB_DOWNLOAD_LIB=1 cargo check --no-default-features --features duckdb-prebuilt` succeeds.
- [ ] `cargo test-dev --test memory` succeeds and exercises the memory DuckDB path in prebuilt mode.
- [ ] `cargo clippy-dev` succeeds under the repository lint policy.
- [ ] Documentation tells OpenSymphony agents to prefer `cargo test-dev` and `cargo clippy-dev` for iterative development, and to run default bundled-mode validation before release-sensitive changes.
- [ ] The implementation does not alter memory server APIs, memory schema, or capture/query behavior.

## Test Plan

- `cargo fmt --check`
- `DUCKDB_DOWNLOAD_LIB=1 cargo check --no-default-features --features duckdb-prebuilt`
- `cargo test-dev --test memory`
- `cargo clippy-dev`
- `cargo test --test memory`

## Context

- Before this task, `Cargo.toml` enabled `duckdb` with the `bundled` feature through the workspace dependency.
- `libduckdb-sys` supports `DUCKDB_DOWNLOAD_LIB=1` for downloaded prebuilt native libraries in non-bundled mode.
- `.cargo/config.toml` already centralizes repository cargo aliases, but the DuckDB download env var should stay scoped to the developer aliases.
- `AGENTS.md` should steer OpenSymphony development tasks toward the faster dev aliases once this work lands.
- `README.md`, `docs/operations.md`, and `docs/testing-and-operations.md` currently describe bundled DuckDB and validation commands.

## Definition of Ready

- [x] Hidden assumptions from prior discussion are written down.
- [x] Required files, docs, and dependencies are explicitly referenced.
- [x] A coding agent could begin execution without additional planning context.

## Notes

Keep this story intentionally narrow. It is complementary to, but distinct from, the later provider-shaped memory backend work that would make DuckDB optional at the API boundary.
