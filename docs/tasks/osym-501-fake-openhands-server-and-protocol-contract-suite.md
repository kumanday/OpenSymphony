---
id: OSYM-501
title: Build fake OpenHands server and protocol contract suite
type: feature
area: quality-ops
priority: P0
estimate: 5d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-202
  - OSYM-203
  - OSYM-204
  - OSYM-302
  - OSYM-304
blocks:
  - OSYM-502
  - OSYM-503
project_context:
  - AGENTS.md
  - README.md
  - docs/testing-and-operations.md
  - docs/websocket-runtime.md
repo_paths:
  - crates/opensymphony-testkit/
  - crates/opensymphony-openhands/
  - crates/opensymphony-linear/
definition_of_ready:
  - Dependencies are merged
  - Fake-server scope is agreed
---

# OSYM-501: Build fake OpenHands server and protocol contract suite

## Summary
Create the deterministic fake runtime and contract suite that make the highest-risk adapter paths testable in CI without relying on a real local OpenHands server.

## Scope
- Implement a fake HTTP and WebSocket server for the minimum runtime contract
- Allow scripted event sequences for success, failure, disconnect, and out-of-order cases
- Add protocol-focused integration tests for the Rust client and scheduler

## Out of scope
- A full simulation of every OpenHands feature
- UI rendering tests

## Deliverables
- Fake OpenHands server
- Fixture builders for event streams and conversation states
- CI integration suites for adapter and scheduler behavior

## Acceptance criteria
- Core adapter behavior can be validated in CI without a live server
- Race windows around initial sync and readiness can be reproduced deterministically
- Event ordering and deduplication logic is covered by tests

## Test plan
- CI runs of the fake-server suite
- Protocol contract assertions for all required endpoints
