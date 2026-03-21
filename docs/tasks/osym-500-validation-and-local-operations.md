---
id: OSYM-500
title: Validation and Local Operations
type: parent
area: quality-ops
priority: P0
estimate: 2w
milestone: M5 Validation and local packaging
depends_on:
  - OSYM-200
  - OSYM-300
  - OSYM-400
blocks:
  - OSYM-600
children:
  - OSYM-501
  - OSYM-502
  - OSYM-503
project_context:
  - AGENTS.md
  - README.md
  - docs/testing-and-operations.md
  - docs/implementation-plan.md
repo_paths:
  - crates/opensymphony-testkit/
  - crates/opensymphony-cli/
  - scripts/
  - tools/openhands-server/
definition_of_ready:
  - M2 through M4 are substantially merged
  - Pinned local server environment exists
  - Example target repo exists
---

# OSYM-500: Validation and Local Operations

## Summary
Turn the implementation into a credible local MVP with deterministic fakes, live end-to-end tests, packaging, and a meaningful doctor command.

## Scope
- Fake OpenHands server
- Contract and integration suites
- Live local end-to-end tests
- CLI doctor and operational packaging

## Out of scope
- Hosted fleet automation

## Child issues
- OSYM-501
- OSYM-502
- OSYM-503

## Deliverables
- Testkit crate
- Live E2E scripts and tests
- CLI doctor command and operational docs

## Acceptance criteria
- All child issues are merged
- Local MVP can be validated on a new trusted machine with documented steps
- The repo has a repeatable path to detect regressions in the runtime adapter

## Test plan
- Run fake-server tests in CI
- Run live local suite on a controlled machine
