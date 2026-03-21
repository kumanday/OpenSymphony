---
id: OSYM-302
title: Build Linear read adapter and issue normalization
type: feature
area: tracker
priority: P0
estimate: 4d
milestone: M3 Symphony orchestration core
parent: OSYM-300
depends_on:
  - OSYM-101
  - OSYM-103
blocks:
  - OSYM-303
  - OSYM-304
  - OSYM-501
project_context:
  - AGENTS.md
  - README.md
  - docs/linear-and-tools.md
  - docs/symphony-spec-alignment.md
repo_paths:
  - crates/opensymphony-linear/
definition_of_ready:
  - OSYM-101 and OSYM-103 are merged
  - Linear query fields are agreed
---

# OSYM-302: Build Linear read adapter and issue normalization

## Summary
Implement the Linear GraphQL read adapter used by the orchestrator to fetch candidate issues, refresh active states, and reconcile running work.

## Scope
- Fetch candidate issues in configured active states
- Fetch issues by state for startup cleanup logic
- Fetch state by issue IDs for running-work reconciliation
- Normalize Linear issue payloads into the shared domain model
- Handle pagination and rate-aware retries

## Out of scope
- Agent-side writes such as comments or state changes
- Non-Linear trackers in the MVP

## Deliverables
- Linear client
- Query models and normalization layer
- Tracker-facing error categories

## Acceptance criteria
- The orchestrator can obtain normalized issue records from Linear
- Pagination is handled correctly
- The adapter does not expose GraphQL-specific shapes to the scheduler core

## Test plan
- Fixture-based GraphQL response tests
- Pagination tests
- Error mapping tests
