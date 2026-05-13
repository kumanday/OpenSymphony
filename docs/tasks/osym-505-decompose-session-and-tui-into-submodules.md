---
id: OSYM-505
title: Decompose oversized session and TUI modules into focused submodules
type: refactor
area: quality-ops
priority: P1
estimate: 3d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-204
  - OSYM-402
blocks: []
project_context:
  - AGENTS.md
  - docs/architecture.md
  - docs/websocket-runtime.md
repo_paths:
  - crates/opensymphony-openhands/src/session.rs
  - crates/opensymphony-tui/src/lib.rs
  - crates/opensymphony-cli/src/lib.rs
  - crates/opensymphony-cli/src/init_repo.rs
definition_of_ready:
  - Baseline tests are green on main
  - Public API surface of the affected crates is frozen for the change
---

# OSYM-505: Decompose oversized session and TUI modules into focused submodules

## Summary
Split the largest single-file modules into cohesive submodules so that turn execution, context-overflow recovery, TUI rendering/reduction, and CLI init-repo concerns can each be read, tested, and modified in isolation without behavior changes.

## Scope
- Split `crates/opensymphony-openhands/src/session.rs` (~3,050 LOC) into submodules such as `session/turn.rs`, `session/overflow.rs`, `session/lifecycle.rs`, and `session/mod.rs`
- Split `crates/opensymphony-tui/src/lib.rs` (~2,799 LOC) into `render.rs`, `reduce.rs`, `panes.rs`, and related submodules
- Split `crates/opensymphony-cli/src/init_repo.rs` (~1,832 LOC) along template fetching, GitHub setup, and file generation boundaries
- Keep all public items re-exported from each crate's `lib.rs` so no external callers change

## Out of scope
- Altering observable behavior, logging, or error types
- Adding new features or changing the WebSocket or CLI contracts
- Reorganizing crate boundaries in the workspace

## Deliverables
- Refactored submodule trees for `opensymphony-openhands::session`, `opensymphony-tui`, and `opensymphony-cli::init_repo`
- Updated imports and, where helpful, file-level doc comments describing each submodule's responsibility
- Git history that preserves blame via `git mv` where possible

## Acceptance criteria
- `cargo fmt --check`, `cargo clippy --workspace -- -D warnings`, and `cargo test --workspace` all pass without new suppressions
- No public item in any affected crate's `lib.rs` changes name or signature
- Each new submodule is meaningfully smaller than the original file and has a single stated responsibility

## Test plan
- Full workspace test run before and after the refactor to confirm no behavior drift
- Live integration suite (`OPENSYMPHONY_LIVE_OPENHANDS=1 cargo test --test live_local_suite -- --ignored`) on a development machine
- Manual TUI smoke check to confirm rendering and focus behavior are unchanged
