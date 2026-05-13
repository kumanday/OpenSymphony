---
id: OSYM-731
title: Repository, Linear, And Research Analysis
milestone: "M4: Collaborative Planning Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-720", "OSYM-730"]
blocks: ["OSYM-732"]
parent: null
---

## Summary

Add the planning-stage analysis pass that inspects repository structure, existing Linear state, and relevant public documentation or APIs.

## Scope

### In scope

- Analyze repository structure, architecture, ownership boundaries, conventions, risks, and integration points.
- Read existing Linear project, milestone, issue, sub-issue, and relation state.
- Summarize existing constraints and known work.
- Research public documentation, APIs, ecosystem references, and relevant external sources.
- Attach research and codebase analysis artifacts to the planning session.

### Out of scope

- General background knowledge base.
- Automated execution of implementation tasks.
- Linear publishing.

## Deliverables

- Analysis artifact generator.
- Repository fixture analysis tests.
- Linear fixture analysis tests.
- Research artifact storage and review path.

## Acceptance Criteria

- [ ] A planning session can include codebase analysis with concrete repo paths and integration risks.
- [ ] A planning session can include Linear graph context for existing project work.
- [ ] Research findings are cited or linked enough for review before plan generation.

## Test Plan

- Run fixture repo analysis tests.
- Run fake Linear graph analysis tests.
- Verify generated research artifacts are attached to a planning session.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.6.3 and `docs/host-client-architecture.md` 4.6.
- Research should be targeted to the project being planned and should support task decomposition.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This is the research and analysis stage of the task-creation workflow.
