---
id: OSYM-100
title: Foundation and Contracts
type: parent
area: foundation
priority: P0
estimate: 2w
milestone: M1 Foundation and contracts
children:
  - OSYM-101
  - OSYM-102
  - OSYM-103
project_context:
  - AGENTS.md
  - README.md
  - docs/architecture.md
  - docs/repository-layout.md
  - docs/implementation-plan.md
repo_paths:
  - Cargo.toml
  - crates/
  - docs/
definition_of_ready:
  - Project docs are present and reviewed
  - Repository path ownership is agreed
  - Milestone M1 boundaries are accepted
---

# OSYM-100: Foundation and Contracts

## Summary
Establish the repository skeleton, core configuration and workflow contracts, and the orchestrator state model that every later milestone depends on.

## Scope
- Create the Cargo workspace and crate boundaries
- Define workflow and config parsing rules
- Define domain models and scheduler states
- Set lint, test, and documentation conventions for later tasks

## Out of scope
- OpenHands transport implementation
- Linear API calls
- FrankenTUI rendering

## Child issues
- OSYM-101
- OSYM-102
- OSYM-103

## Deliverables
- Bootstrap repo with agreed crate layout
- Typed workflow/config crate
- Shared domain model crate
- State-machine tests and docs updates

## Acceptance criteria
- All child issues are merged
- Later tasks can compile against stable crate boundaries
- State transitions are documented and covered by tests

## Test plan
- Run all M1 unit tests in CI
- Verify downstream crates can depend on M1 interfaces without touching implementation details
