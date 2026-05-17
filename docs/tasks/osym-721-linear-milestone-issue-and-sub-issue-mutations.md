---
id: OSYM-721
title: Linear Milestone, Issue, And Sub-Issue Mutations
milestone: "M8: Task Graph Operations And OpenHands Run UI"
priority: 1
estimate: 8
blockedBy: ["OSYM-705", "OSYM-720"]
blocks: ["OSYM-722", "OSYM-736"]
parent: null
---

## Summary

Implement gateway-mediated Linear mutations for creating and updating milestones, issues, sub-issues, comments, evidence notes, and dependency relations.

## Scope

### In scope

- Create and update Linear project milestones.
- Create and update Linear issues under projects and milestones.
- Create and update Linear sub-issues under issues.
- Support status, priority, labels, assignee, estimate, relations, blocker links, comments, and evidence notes where available.
- Return action receipts and publish correlated task graph update events.
- Add fake Linear mutation tests.

### Out of scope

- Planning draft preview UI.
- Hosted Linear credential management.
- Tracker adapters beyond Linear.

## Deliverables

- `/api/v1/taskgraph/milestones` mutation endpoint.
- `/api/v1/taskgraph/issues` mutation endpoint.
- `/api/v1/taskgraph/sub-issues` mutation endpoint.
- GraphQL mutation assets or service methods.
- Mutation tests with fake Linear.

## Acceptance Criteria

- [ ] A gateway client can create and update a Linear milestone, issue, and sub-issue.
- [ ] Mutation responses include action receipt fields and expected follow-up events.
- [ ] Relation/blocker mutations preserve the dependency graph expected by the task graph UI and planning flow.

## Test Plan

- Run fake Linear mutation tests for success, validation failure, permission failure placeholder, and schema drift cases.
- Verify emitted task graph update events carry correlation IDs from receipts.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.5.3 and `docs/host-client-architecture.md` 7.3.
- Keep Linear GraphQL helper/query assets compatible until a gateway service fully replaces them.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use Linear-native names in request and response shapes.
