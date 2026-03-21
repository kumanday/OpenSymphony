---
id: OSYM-401
title: Implement control-plane API and snapshot store
type: feature
area: ui-observability
priority: P1
estimate: 4d
milestone: M4 Operator UX and repo harness
parent: OSYM-400
depends_on:
  - OSYM-103
  - OSYM-203
  - OSYM-304
blocks:
  - OSYM-402
  - OSYM-503
  - OSYM-601
project_context:
  - AGENTS.md
  - README.md
  - docs/architecture.md
  - docs/ui-frankentui.md
  - docs/testing-and-operations.md
repo_paths:
  - crates/opensymphony-control/
  - crates/opensymphony-orchestrator/
definition_of_ready:
  - Dependencies are merged
  - Snapshot schema is reviewed
---

# OSYM-401: Implement control-plane API and snapshot store

## Summary
Implement the local control plane that publishes daemon snapshots and selected runtime events to read-only clients such as FrankenTUI.

## Scope
- Define stable snapshot payloads
- Publish snapshots from orchestrator-owned state changes
- Expose a local HTTP API for current state
- Expose a local update stream for incremental UI refresh
- Keep the surface read-only in the MVP

## Out of scope
- Remote multi-tenant auth
- Bidirectional control actions from the UI

## Deliverables
- Control-plane server
- Snapshot store and publisher
- Client-facing serialization models

## Acceptance criteria
- A UI client can attach and render current daemon state without reaching into daemon internals
- Snapshot updates reflect worker and issue state changes promptly
- The API remains functional without the UI attached

## Test plan
- Snapshot reducer tests
- Serialization tests
- Integration tests with a mock control-plane client
