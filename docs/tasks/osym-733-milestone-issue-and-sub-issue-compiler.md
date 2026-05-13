---
id: OSYM-733
title: Milestone, Issue, And Sub-Issue Compiler
milestone: "M4: Collaborative Planning Alpha"
priority: 1
estimate: 5
blockedBy: ["OSYM-732"]
blocks: ["OSYM-734", "OSYM-736"]
parent: null
---

## Summary

Compile planning artifacts into a Linear-native hierarchy of milestones, issues, and sub-issues with acceptance criteria, verification expectations, and dependency metadata.

## Scope

### In scope

- Convert planning artifacts into a milestone/issue/sub-issue hierarchy.
- Enforce taxonomy: milestone equals Linear project milestone, issue equals Linear issue, sub-issue equals Linear sub-issue.
- Compile the approved hierarchy into a task package manifest with exact milestone strings and explicit task file references.
- Carry `planningWave` through the task package manifest and Linear publish receipt fields.
- Map GSD-2 phase or milestone-level planning to milestone-level planning.
- Map GSD-2 slices to Linear issues and GSD-2 tasks to Linear sub-issues.
- Require issue acceptance criteria, sub-issue verification expectations, and dependency fields where applicable.
- Flag underspecified sub-issues.

### Out of scope

- Linear mutation execution.
- Task scheduler changes.
- Completion review workflow.

## Deliverables

- Plan compiler.
- Manifest compiler for `docs/tasks/task-package.yaml`.
- Publish receipt field compiler for `docs/tasks/linear-publish.yaml`.
- Taxonomy validation.
- Sub-issue readiness checks.
- Dependency metadata output.

## Acceptance Criteria

- [ ] Compiler output can be rendered as a Linear milestone, issue, and sub-issue tree.
- [ ] Compiler output can be rendered as a manifest-driven task package.
- [ ] Compiler output includes stable fields for `planningWave`, source task files, and Linear publish mappings.
- [ ] Invalid taxonomy or missing required planning fields produce actionable validation messages.
- [ ] Output preserves links back to source artifacts and review comments.

## Test Plan

- Run compiler tests for complete plans, missing criteria, missing verification expectations, invalid parentage, and underspecified sub-issues.
- Verify output can feed the Linear draft preview task.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.6.3 and `docs/host-client-implementation_plan.md` P6.5.
- Use direct Linear terminology in user-facing labels and payloads.
- The compiler should preserve the `planningWave` identity from the planning session.
- The publish flow records created or updated Linear entities in `docs/tasks/linear-publish.yaml`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The compiler is the transition from planning artifacts to a task graph draft.
