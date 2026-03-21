---
id: OSYM-503
title: Implement CLI packaging, doctor, and local operations docs
type: feature
area: quality-ops
priority: P0
estimate: 4d
milestone: M5 Validation and local packaging
parent: OSYM-500
depends_on:
  - OSYM-401
  - OSYM-402
  - OSYM-501
  - OSYM-502
blocks:
  - OSYM-602
project_context:
  - AGENTS.md
  - README.md
  - docs/testing-and-operations.md
  - docs/deployment-modes.md
repo_paths:
  - crates/opensymphony-cli/
  - docs/
  - tools/openhands-server/
  - scripts/
definition_of_ready:
  - Dependencies are merged
  - Operational commands are agreed
---

# OSYM-503: Implement CLI packaging, doctor, and local operations docs

## Summary
Package the local MVP into a usable CLI with a meaningful doctor command and final operations documentation for trusted-machine deployment.

## Scope
- Implement `daemon`, `tui`, `doctor`, and `linear-mcp` entrypoints cleanly
- Wire the doctor command to real preflight checks
- Document setup, troubleshooting, and local safety posture
- Package pinned local server setup under `tools/openhands-server/`

## Out of scope
- Hosted deployment automation
- Browser UI packaging

## Deliverables
- Usable CLI app
- Doctor command
- Updated README and operations docs
- Pinned-tooling setup instructions

## Acceptance criteria
- A new developer can install prerequisites and get to a passing doctor run using the docs
- CLI help and command structure are coherent
- The local safety limitations are stated clearly in docs and doctor output

## Test plan
- CLI smoke tests
- Doctor command tests with missing and present prerequisites
- Manual first-run verification on a clean machine
