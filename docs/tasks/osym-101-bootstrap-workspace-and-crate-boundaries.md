---
id: OSYM-101
title: Bootstrap workspace and crate boundaries
type: feature
area: foundation
priority: P0
estimate: 3d
milestone: M1 Foundation and contracts
parent: OSYM-100
blocks:
  - OSYM-102
  - OSYM-103
  - OSYM-201
  - OSYM-202
  - OSYM-301
  - OSYM-302
project_context:
  - AGENTS.md
  - README.md
  - docs/repository-layout.md
  - docs/implementation-plan.md
repo_paths:
  - Cargo.toml
  - rust-toolchain.toml
  - crates/
  - tools/openhands-server/
definition_of_ready:
  - Repository has been created
  - The planned crate list is accepted
---

# OSYM-101: Bootstrap workspace and crate boundaries

## Summary
Create the Rust workspace, crate skeletons, repo-level lint settings, and the pinned local OpenHands tooling directory so every later task has stable ownership boundaries.

## Scope
- Create the Cargo workspace and crate directories listed in `docs/repository-layout.md`
- Add shared lint, fmt, clippy, and test configuration
- Add placeholder library or binary entrypoints for each crate
- Create `tools/openhands-server/` with version pin placeholders and local README

## Out of scope
- Real runtime implementation
- Business logic beyond bootstrap

## Deliverables
- Compiling Cargo workspace with placeholder crates
- Pinned Rust toolchain file
- Initial CI checks for fmt, clippy, and tests
- Pinned-tooling directory for the local OpenHands server

## Acceptance criteria
- The whole workspace compiles with placeholder implementations
- Crate boundaries match the documented layout
- CI can run formatting, linting, and basic tests successfully

## Test plan
- Run `cargo fmt --check`
- Run `cargo clippy --workspace --all-targets`
- Run `cargo test --workspace` with bootstrap placeholder tests

## Notes
This task should optimize for clean boundaries, not clever implementation. A stable skeleton saves time later.
