---
id: OSYM-600
title: Hosted Deployment Follow-on
type: parent
area: hosted-follow-on
priority: P2
estimate: 2w
milestone: M6 Hosted deployment follow-on
depends_on:
  - OSYM-500
children:
  - OSYM-601
  - OSYM-602
project_context:
  - AGENTS.md
  - README.md
  - docs/deployment-modes.md
  - docs/implementation-plan.md
repo_paths:
  - crates/opensymphony-openhands/
  - docs/
  - scripts/
definition_of_ready:
  - M5 is stable
  - Local MVP is validated
  - Hosted security requirements are documented
---

# OSYM-600: Hosted Deployment Follow-on

## Summary
Add the first hosted-mode capability and documentation without destabilizing the local MVP architecture.

## Scope
- Remote server transport and auth hardening
- Hosted topology guidance and rollout notes

## Out of scope
- Full production SRE automation
- Autoscaling and fleet orchestration implementation

## Child issues
- OSYM-601
- OSYM-602

## Deliverables
- Remote mode transport config
- Auth test coverage
- Hosted deployment guidance

## Acceptance criteria
- All child issues are merged
- The same orchestrator can target a remote agent-server with configuration changes rather than architectural changes

## Test plan
- Remote-mode integration tests against a pinned external server
