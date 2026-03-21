---
id: OSYM-303
title: Implement Linear MCP write surface
type: feature
area: tracker-tools
priority: P1
estimate: 4d
milestone: M3 Symphony orchestration core
parent: OSYM-300
depends_on:
  - OSYM-101
  - OSYM-302
blocks:
  - OSYM-305
  - OSYM-502
project_context:
  - AGENTS.md
  - README.md
  - docs/linear-and-tools.md
  - docs/architecture.md
repo_paths:
  - crates/opensymphony-linear-mcp/
  - crates/opensymphony-cli/
definition_of_ready:
  - OSYM-101 and OSYM-302 are merged
  - The initial write operations are selected
---

# OSYM-303: Implement Linear MCP write surface

## Summary
Implement a small stdio MCP server that lets the coding agent write back to Linear without giving the orchestrator direct responsibility for those writes.

## Scope
- Expose a minimal set of Linear write tools through MCP
- Keep the tool contracts narrow and auditable
- Document how the tool is attached to OpenHands conversations
- Package the MCP server behind a CLI subcommand

## Out of scope
- A full Linear SDK
- Scheduler logic that depends on MCP writes succeeding

## Deliverables
- Runnable MCP server
- Tool schemas and docs
- CLI entrypoint such as `opensymphony linear-mcp`

## Acceptance criteria
- The MCP server starts locally and advertises the documented tools
- Agent-side writes can be performed through the MCP layer
- Failure of MCP writes does not break scheduler correctness

## Test plan
- Tool schema tests
- Local stdio integration tests
- Optional live Linear write tests on a safe sandbox project
