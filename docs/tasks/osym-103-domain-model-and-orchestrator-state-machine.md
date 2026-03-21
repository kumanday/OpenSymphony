---
id: OSYM-103
title: Define domain model and orchestrator state machine
type: feature
area: domain
priority: P0
estimate: 3d
milestone: M1 Foundation and contracts
parent: OSYM-100
depends_on:
  - OSYM-101
blocks:
  - OSYM-202
  - OSYM-204
  - OSYM-301
  - OSYM-302
  - OSYM-304
  - OSYM-401
project_context:
  - AGENTS.md
  - README.md
  - docs/symphony-spec-alignment.md
  - docs/architecture.md
repo_paths:
  - crates/opensymphony-domain/
  - crates/opensymphony-orchestrator/
definition_of_ready:
  - OSYM-101 is merged
  - Core scheduler states are reviewed
---

# OSYM-103: Define domain model and orchestrator state machine

## Summary
Define the shared issue, workspace, worker, retry, and snapshot models plus the scheduler state machine that the orchestrator will own.

## Scope
- Define normalized issue models independent of Linear and OpenHands wire payloads
- Define orchestration states such as Unclaimed, Claimed, Running, RetryQueued, and Released
- Define retry metadata, turn counters, and stall-detection timestamps
- Define snapshot-facing models that can be published to the control plane

## Out of scope
- Transport-specific serialization details
- Filesystem mutation logic

## Deliverables
- Shared domain crate
- State transition helpers
- Explicit error and outcome enums

## Acceptance criteria
- No OpenHands JSON types leak into domain models
- State transitions are explicit and testable
- The snapshot model is stable enough for M4 to build on

## Test plan
- State transition tests
- Retry calculation tests
- Serialization tests for snapshot models
