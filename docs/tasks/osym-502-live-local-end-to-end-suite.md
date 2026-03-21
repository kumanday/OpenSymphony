---
id: OSYM-502
title: Implement live local end-to-end suite
type: feature
area: quality-ops
priority: P1
estimate: 4d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-201
  - OSYM-204
  - OSYM-303
  - OSYM-304
  - OSYM-305
  - OSYM-501
blocks:
  - OSYM-503
project_context:
  - AGENTS.md
  - README.md
  - docs/testing-and-operations.md
  - WORKFLOW.example.md
repo_paths:
  - scripts/
  - examples/
  - crates/opensymphony-cli/
  - crates/opensymphony-testkit/
definition_of_ready:
  - Dependencies are merged
  - Example target repo exists
  - Safe local test credentials are documented
---

# OSYM-502: Implement live local end-to-end suite

## Summary
Implement the opt-in live local suite that proves the MVP really works against a pinned local OpenHands server in a trusted environment.

## Scope
- Add scripts or test harnesses to stand up a local supervised daemon run
- Exercise a minimal target repo and test issue set
- Verify conversation reuse, workspace artifacts, and WebSocket reconnect behavior where practical

## Out of scope
- Always-on CI execution of live tests
- Hosted deployment tests

## Deliverables
- Live local scripts
- Live test docs
- Scenario assertions and expected outputs

## Acceptance criteria
- A new developer can run the documented local suite on a prepared machine
- The suite verifies at least one full issue lifecycle
- The suite captures logs and artifacts for debugging failures

## Test plan
- Manual and scripted execution of the live suite
- Artifact verification after each scenario
