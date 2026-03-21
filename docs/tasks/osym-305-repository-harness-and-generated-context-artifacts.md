---
id: OSYM-305
title: Implement repository harness and generated context artifacts
type: feature
area: repo-harness
priority: P1
estimate: 3d
milestone: M3 Symphony orchestration core
parent: OSYM-300
depends_on:
  - OSYM-102
  - OSYM-301
  - OSYM-303
blocks:
  - OSYM-502
project_context:
  - AGENTS.md
  - README.md
  - docs/linear-and-tools.md
  - docs/workspace-and-lifecycle.md
repo_paths:
  - examples/
  - crates/opensymphony-workspace/
  - docs/
definition_of_ready:
  - Dependencies are merged
  - Artifact ownership is agreed
---

# OSYM-305: Implement repository harness and generated context artifacts

## Summary
Define and implement the repo-scoped context artifacts that OpenSymphony should generate or reference inside issue workspaces so the agent gets stable, repeatable instructions without mutating user-owned repo policy unexpectedly.

## Scope
- Define `.opensymphony/` workspace artifacts such as issue and conversation manifests
- Generate per-run prompt capture files for auditability
- Document how repo-owned `AGENTS.md` and optional `.agents/skills/` interact with the workflow prompt
- Provide example target-repo artifacts for local development

## Out of scope
- Overwriting repository-owned `AGENTS.md` files
- Inventing a second policy system outside `WORKFLOW.md` and repo context

## Deliverables
- Artifact generation rules
- Example target repo updates
- Docs for context precedence and safe coexistence

## Acceptance criteria
- Generated artifacts are local to the issue workspace and deterministic
- Repo-owned policy files remain the repo owner's responsibility
- Implementers can inspect prompt and manifest artifacts after a run

## Test plan
- Artifact path and content tests
- Temp-workspace integration tests
- Manual verification in the example target repo
