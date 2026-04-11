---
id: OSYM-303
title: Implement Linear GraphQL agent write path
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
  - crates/opensymphony-cli/
  - .agents/skills/linear/
definition_of_ready:
  - OSYM-101 and OSYM-302 are merged
  - The initial write operations are selected
---

# OSYM-303: Implement Linear GraphQL agent write path

## Summary
Implement the checked-in GraphQL helper, query assets, and documentation that let the coding agent write back to Linear without giving the orchestrator direct responsibility for those writes.

## Scope
- Provide a small checked-in helper for authenticated GraphQL execution
- Check in audited query files for the supported write flows
- Document how target repos should use the helper and query assets
- Keep the orchestrator independent of agent-side write failures

## Out of scope
- A generated full Linear SDK
- Scheduler logic that depends on agent-side writes succeeding

## Deliverables
- Repo-local helper script
- Query assets and reference docs
- `opensymphony init` propagation of the full skill tree

## Acceptance criteria
- Target repos can run the helper with `LINEAR_API_KEY`
- Agent-side writes can be performed through the checked-in GraphQL assets
- Failure of agent-side writes does not break scheduler correctness

## Test plan
- CLI init propagation tests
- Helper smoke tests with checked-in query files
- Optional live Linear write tests on a safe sandbox project
