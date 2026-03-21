---
id: OSYM-300
title: Tracker, Workspaces, and Orchestration
type: parent
area: orchestration
priority: P0
estimate: 3w
milestone: M3 Symphony orchestration core
depends_on:
  - OSYM-100
  - OSYM-200
blocks:
  - OSYM-400
  - OSYM-500
children:
  - OSYM-301
  - OSYM-302
  - OSYM-303
  - OSYM-304
  - OSYM-305
project_context:
  - AGENTS.md
  - README.md
  - docs/workspace-and-lifecycle.md
  - docs/linear-and-tools.md
  - docs/symphony-spec-alignment.md
  - docs/implementation-plan.md
repo_paths:
  - crates/opensymphony-workspace/
  - crates/opensymphony-linear/
  - crates/opensymphony-linear-mcp/
  - crates/opensymphony-orchestrator/
definition_of_ready:
  - M1 and M2 are merged
  - Linear access requirements are documented
  - Workspace root and example repo strategy are agreed
---

# OSYM-300: Tracker, Workspaces, and Orchestration

## Summary
Implement the Symphony-specific heart of the system: issue polling, workspace lifecycle, retry scheduling, and the agent-facing Linear tool path.

## Scope
- Workspace create, reuse, hook execution, and cleanup
- Linear read adapter for orchestration
- Linear MCP server for agent-side writes
- Scheduler loop, retries, stall detection, and reconciliation
- Repository harness files and generated context artifacts

## Out of scope
- FrankenTUI rendering
- Hosted deployment specifics

## Child issues
- OSYM-301
- OSYM-302
- OSYM-303
- OSYM-304
- OSYM-305

## Deliverables
- Workspace manager
- Linear read adapter
- Linear MCP server
- Orchestrator scheduler
- Repo harness artifact generation rules

## Acceptance criteria
- All child issues are merged
- Daemon can claim and execute issues end to end in a temp repo
- Retry and cleanup behavior matches the documented Symphony mapping

## Test plan
- Fake Linear integration tests
- Scheduler behavior tests
- Local temp-repo integration tests
