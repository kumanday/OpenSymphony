---
id: OSYM-601
title: Add remote agent-server mode and auth hardening
type: feature
area: hosted-follow-on
priority: P2
estimate: 4d
milestone: M6 Hosted deployment follow-on
parent: OSYM-600
depends_on:
  - OSYM-202
  - OSYM-203
  - OSYM-204
  - OSYM-401
blocks:
  - OSYM-602
project_context:
  - AGENTS.md
  - README.md
  - docs/deployment-modes.md
  - docs/openhands-agent-server.md
repo_paths:
  - crates/opensymphony-openhands/
  - crates/opensymphony-control/
definition_of_ready:
  - Dependencies are merged
  - Hosted auth expectations are documented
  - `crates/opensymphony-openhands/` and `crates/opensymphony-control/` exist in the working branch
---

# OSYM-601: Add remote agent-server mode and auth hardening

## Summary
Extend the runtime adapter so the same orchestrator can connect to a remote agent-server with hardened auth and transport settings.

## Scope
- Support explicit external base URLs cleanly
- Reuse the existing `openhands.transport`, `openhands.local_server`, and `openhands.websocket` config surface instead of inventing a hosted-only runtime path
- Support configured auth modes for HTTP and WebSocket
- Add stricter transport validation for remote mode
- Ensure the control plane exposes enough metadata for remote troubleshooting

## Out of scope
- Full enterprise auth provider integration
- Autoscaling and tenancy control

## Deliverables
- Remote-mode transport config
- Documented mapping from `WORKFLOW.example.md` transport fields to remote mode
- Auth-capable HTTP and WebSocket connection logic
- Remote-mode docs and tests

## Acceptance criteria
- The daemon can connect to a remote server without code changes outside config
- HTTP and WebSocket auth behave consistently for the pinned version
- Local supervised mode remains unaffected

## Test plan
- Integration tests against an external pinned server
- Exercise `transport.base_url`, `session_api_key_env`, `local_server.enabled=false`, and `websocket.auth_mode` against the pinned server
- Auth failure and success tests
