---
id: OSYM-304
title: Implement orchestrator scheduler, retries, and reconciliation
type: feature
area: orchestration
priority: P0
estimate: 6d
milestone: M3 Symphony orchestration core
parent: OSYM-300
depends_on:
  - OSYM-102
  - OSYM-103
  - OSYM-204
  - OSYM-301
  - OSYM-302
blocks:
  - OSYM-401
  - OSYM-501
  - OSYM-502
project_context:
  - AGENTS.md
  - README.md
  - docs/architecture.md
  - docs/symphony-spec-alignment.md
  - docs/workspace-and-lifecycle.md
repo_paths:
  - crates/opensymphony-orchestrator/
definition_of_ready:
  - Dependencies are merged
  - Concurrency and retry policies are documented
---

# OSYM-304: Implement orchestrator scheduler, retries, and reconciliation

## Summary
Implement the long-running Symphony scheduler loop that claims work, dispatches issue workers, handles retry policy, reconciles running tasks, and releases terminal work.

## Scope
- Periodic poll loop over Linear active states
- Candidate sorting and bounded dispatch
- Claim, run, retry, and release transitions
- Failure backoff and fixed continuation retry
- Running-issue reconciliation and stall detection
- Restart recovery from manifests and tracker state

## Out of scope
- UI presentation logic
- Hosted multi-daemon sharding

## Deliverables
- Scheduler loop
- Retry queue
- Running worker registry
- Reconciliation logic

## Acceptance criteria
- Normal worker exit schedules a continuation retry when the issue remains active
- Failures schedule exponential backoff according to config
- Terminal issues trigger cleanup and release
- The scheduler can restart and recover tracked work without corrupting state

## Test plan
- State-machine tests for all transition paths
- Fake Linear plus fake OpenHands integration tests
- Recovery tests with persisted manifests
