---
id: OSYM-736
title: Linear Draft Preview And Publish Flow
milestone: "M4: Collaborative Planning Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-721", "OSYM-733", "OSYM-734", "OSYM-735"]
blocks: ["OSYM-770"]
parent: null
---

## Summary

Implement the review and publish flow that turns approved planning artifacts into Linear milestones, issues, sub-issues, comments, and relations.

## Scope

### In scope

- Generate draft Linear GraphQL mutation payloads.
- Show created and updated milestones, issues, sub-issues, comments, and relations.
- Show warnings, missing fields, and plan validation results.
- Require explicit approval before publish.
- Wrap the manifest-driven `convert-tasks-to-linear` flow as the publish stage where appropriate.
- Read from `docs/tasks/task-package.yaml` and write publish state compatible with `docs/tasks/linear-publish.yaml`.
- Prefer gateway-mediated Linear mutations for the UI flow.
- Store publish receipts.

### Out of scope

- Executing generated implementation work.
- Hosted audit UI.
- Tracker adapters beyond Linear.

## Deliverables

- Linear draft preview UI.
- Draft validation tests.
- Publish flow integration.
- Publish receipt artifacts containing `planningWave`, Linear project, milestone mappings, issue mappings, and source task files.

## Acceptance Criteria

- [ ] Users can preview exact Linear entities and relations before publishing.
- [ ] Publishing creates or updates Linear entities only after approval.
- [ ] Publish receipts identify created/updated entities, source task files, and the planning wave.

## Test Plan

- Run draft generation tests from compiled planning artifacts.
- Run fake Linear publish tests for success, partial failure, validation failure, and retry-safe behavior.

## Context

- Source skill: `.agents/skills/convert-tasks-to-linear/SKILL.md`.
- Source sections: `docs/hosted-client-PRD.md` 4.6.4 and `docs/host-client-implementation_plan.md` P6.8 through P6.10.
- Publishing should produce Linear-backed work ready for OpenSymphony scheduling.
- Current publish state is represented by `docs/tasks/linear-publish.yaml`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The publish flow is the final stage of the collaborative task-creation workflow.
