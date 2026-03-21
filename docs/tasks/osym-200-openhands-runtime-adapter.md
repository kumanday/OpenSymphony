---
id: OSYM-200
title: OpenHands Runtime Adapter
type: parent
area: agent-runtime
priority: P0
estimate: 3w
milestone: M2 OpenHands runtime adapter
depends_on:
  - OSYM-100
blocks:
  - OSYM-300
  - OSYM-500
children:
  - OSYM-201
  - OSYM-202
  - OSYM-203
  - OSYM-204
project_context:
  - AGENTS.md
  - README.md
  - docs/openhands-agent-server.md
  - docs/websocket-runtime.md
  - docs/implementation-plan.md
repo_paths:
  - crates/opensymphony-openhands/
  - tools/openhands-server/
definition_of_ready:
  - M1 is merged
  - Pinned OpenHands version is selected
  - Wire-contract assumptions are reviewed
---

# OSYM-200: OpenHands Runtime Adapter

## Summary
Implement the direct Rust integration with OpenHands agent-server for the local MVP. This includes local server supervision, REST operations, WebSocket-first event handling, and the issue session runner.

## Scope
- Supervise one local agent-server subprocess
- Serialize the minimal conversation request subset
- Attach to runtime events through WebSocket from day one
- Recover from disconnects using reconcile-after-ready behavior
- Expose an orchestrator-facing issue session runner

## Out of scope
- Hosted topology and remote auth hardening beyond basic config hooks
- UI concerns

## Child issues
- OSYM-201
- OSYM-202
- OSYM-203
- OSYM-204

## Deliverables
- Local server supervisor
- REST client
- WebSocket event-stream client
- Session runner facade for the orchestrator

## Acceptance criteria
- All child issues are merged
- A temp-repo issue can be executed through the adapter with one local OpenHands server
- Reconnect and reconcile behavior is covered by tests

## Test plan
- Fake-server contract suite for HTTP and WebSocket behavior
- One live local smoke run against the pinned server
