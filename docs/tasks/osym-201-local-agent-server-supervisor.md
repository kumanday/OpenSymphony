---
id: OSYM-201
title: Implement local agent-server supervisor
type: feature
area: agent-runtime
priority: P0
estimate: 4d
milestone: M2 OpenHands runtime adapter
parent: OSYM-200
depends_on:
  - OSYM-101
blocks:
  - OSYM-203
  - OSYM-204
  - OSYM-502
project_context:
  - AGENTS.md
  - README.md
  - docs/openhands-agent-server.md
  - docs/deployment-modes.md
  - docs/testing-and-operations.md
repo_paths:
  - crates/opensymphony-openhands/
  - tools/openhands-server/
  - crates/opensymphony-cli/
definition_of_ready:
  - OSYM-101 is merged
  - Pinned OpenHands server version is selected
---

# OSYM-201: Implement local agent-server supervisor

## Summary
Implement the local supervised-mode lifecycle for one shared OpenHands agent-server subprocess per daemon.

## Scope
- Resolve the pinned server command and environment from config
- Launch the subprocess bound to loopback only
- Probe readiness through a documented or conservative HTTP check
- Expose start, stop, and status methods to the daemon
- Log version and process metadata for diagnostics

## Out of scope
- Remote or hosted auth hardening
- Multiple server instances for load balancing

## Deliverables
- Supervisor abstraction
- Process startup and shutdown logic
- Healthcheck probe logic
- Basic diagnostics for the doctor command

## Acceptance criteria
- The daemon can start and stop the server reliably
- The supervisor never kills an external server it did not launch
- Loopback binding is the default in local mode

## Test plan
- Integration tests with a fake or lightweight local server process
- Failure tests for startup timeout and unexpected exit
- Manual smoke run against the pinned real server
