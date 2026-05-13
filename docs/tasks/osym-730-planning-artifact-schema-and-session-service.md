---
id: OSYM-730
title: Planning Artifact Schema And Session Service
milestone: "M4: Collaborative Planning Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-701", "OSYM-702"]
blocks: ["OSYM-731", "OSYM-732", "OSYM-733", "OSYM-735"]
parent: null
---

## Summary

Define planning artifacts and implement the planning session service that stores conversation turns, artifact revisions, diffs, review comments, and projections.

## Scope

### In scope

- Define artifacts for intake, project context, requirements, research brief, codebase analysis, architecture notes, risk register, milestone plan, issue plan, sub-issue plan, dependency graph, verification plan, plan validation, Linear draft, review comments, and publish receipt.
- Define a planning-wave artifact that can project to a manifest-driven `docs/tasks/task-package.yaml`.
- Define a Linear publish receipt artifact compatible with `docs/tasks/linear-publish.yaml`.
- Create planning sessions through the gateway.
- Store conversation turns, artifact revisions, artifact diffs, review comments, and session state.
- Render structured and markdown projections for review, prompt context, audit history, and diffs.

### Out of scope

- Research execution.
- Linear publishing.
- Full planning UI.

## Deliverables

- Planning artifact schema.
- Task package and publish receipt schemas.
- Planning session APIs.
- Artifact validation tests.
- Session persistence and diff tests.

## Acceptance Criteria

- [ ] Planning sessions can store and retrieve all required artifact types.
- [ ] Planning sessions can store a stable `planningWave` identity and render a task package projection with explicit source task paths.
- [ ] Artifact revisions are diffable and reviewable.
- [ ] Session APIs expose enough state for a conversational planning workspace.

## Test Plan

- Run artifact schema validation tests.
- Run planning session create, update, diff, and projection tests.

## Context

- Source sections: `PRODUCT.md` section 8, `docs/hosted-client-PRD.md` 4.6, and `docs/host-client-implementation_plan.md` P6.1/P6.2.
- The adapted GSD-2 workflow for OpenSymphony covers interview, research, codebase analysis, planning, decomposition, dependency graph, review, and Linear publishing.
- The current skill contract uses `docs/tasks/task-package.yaml` as the task package manifest and `docs/tasks/linear-publish.yaml` as publish state.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep user-facing planning terminology aligned with Linear milestones, issues, and sub-issues.
