---
id: OSYM-204
title: Implement issue session runner
type: feature
area: agent-runtime
priority: P0
estimate: 5d
milestone: M2 OpenHands runtime adapter
parent: OSYM-200
depends_on:
  - OSYM-102
  - OSYM-103
  - OSYM-201
  - OSYM-202
  - OSYM-203
  - OSYM-301
blocks:
  - OSYM-304
  - OSYM-502
  - OSYM-601
project_context:
  - AGENTS.md
  - README.md
  - docs/openhands-agent-server.md
  - docs/websocket-runtime.md
  - docs/workspace-and-lifecycle.md
repo_paths:
  - crates/opensymphony-openhands/
  - crates/opensymphony-orchestrator/
definition_of_ready:
  - Dependencies are merged
  - Conversation reuse policy is agreed
---

# OSYM-204: Implement issue session runner

## Summary
Build the orchestrator-facing issue session runner that binds issue metadata, workspace paths, workflow prompts, and the OpenHands runtime into one coherent attempt execution API.

## Scope
- Create or reuse the issue conversation based on workspace manifests
- Choose full prompt vs continuation guidance correctly
- Post the user event, trigger `run`, and await terminal runtime status
- Return a normalized worker outcome to the orchestrator
- Persist run metadata needed for recovery and observability

## Out of scope
- Scheduler loop ownership
- Linear polling and reconciliation logic

## Deliverables
- Issue session runner facade
- Issue conversation manifest format
- Prompt selection logic for fresh vs continued turns
- Normalized runtime outcome mapping

## Acceptance criteria
- The same issue reuses its conversation by default
- Fresh and continuation prompts are selected according to the documented rules
- The orchestrator receives a stable outcome API without raw transport details

## Test plan
- Temp-workspace tests for conversation manifest behavior
- Fake-server tests for success, failure, and continuation paths
- Live local smoke run in a temp repo
