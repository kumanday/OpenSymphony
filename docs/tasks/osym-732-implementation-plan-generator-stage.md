---
id: OSYM-732
title: Implementation Plan Generator Stage
milestone: "M9: Collaborative Planning Alpha"
priority: 1
estimate: 5
blockedBy: ["OSYM-731"]
blocks: ["OSYM-733"]
parent: null
---

## Summary

Wrap the existing `create-implementation-plan` skill as a structured planning-stage generator that uses intake, research, Linear state, and codebase analysis artifacts.

## Scope

### In scope

- Adapt the skill into a generator stage within a planning session.
- Feed repository analysis, Linear graph context, requirements, constraints, and research findings into generation.
- Store generated milestone, issue, sub-issue, acceptance criteria, verification, and dependency outputs as structured artifacts.
- Store a generated task package projection with `planningWave`, exact Linear milestone names, and an explicit task file list.
- Preserve Linear-native terminology in generated artifacts.
- Support selective regeneration of specific artifacts.

### Out of scope

- Linear GraphQL publishing.
- Autonomous implementation of generated work.
- OpenSymphony runtime execution changes.

## Deliverables

- Implementation plan generator integration.
- Task package projection for `docs/tasks/task-package.yaml`.
- Prompt/context assembly logic.
- Artifact output tests.
- Selective regeneration path.

## Acceptance Criteria

- [ ] The generator produces milestone-level goals, issue-level vertical deliverables, and sub-issue-level execution units.
- [ ] The generator emits a manifest-driven task package from the explicit task paths in `docs/tasks/task-package.yaml`.
- [ ] Generated artifacts include acceptance criteria, verification expectations, and initial dependencies.
- [ ] Regeneration preserves human-reviewed artifacts outside the selected regeneration scope.

## Test Plan

- Run generator tests with fixture intake, research, codebase analysis, and Linear context.
- Verify generated artifacts validate against planning schemas.

## Context

- Source skill: `.agents/skills/create-implementation-plan/SKILL.md`.
- Source sections: `PRODUCT.md` section 8 and `docs/host-client-implementation_plan.md` P6.4.
- GSD-2 inspiration is limited to task creation flow: interview, research, analysis, planning, decomposition, dependency graph, review, and publish.
- Current skill output includes `docs/tasks/task-package.yaml` and `docs/tasks/milestones.md`.
- Downstream publish state is recorded in `docs/tasks/linear-publish.yaml`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This stage should produce reviewable artifacts before any Linear entity is created or updated.
