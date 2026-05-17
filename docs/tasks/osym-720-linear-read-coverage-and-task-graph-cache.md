---
id: OSYM-720
title: Linear Read Coverage And Task Graph Cache
milestone: "M8: Task Graph Operations And OpenHands Run UI"
priority: 1
estimate: 5
blockedBy: ["OSYM-703"]
blocks: ["OSYM-721", "OSYM-722", "OSYM-731"]
parent: null
---

## Summary

Expand Linear read coverage and build the cache that joins Linear task graph data with OpenSymphony runtime overlays.

## Scope

### In scope

- Ensure GraphQL reads cover projects, project milestones, issues, sub-issues, relations, labels, priorities, statuses, assignees, estimates, comments, and attachments where needed.
- Add Linear schema drift validation.
- Cache Linear entities with sync timestamps.
- Cache OpenSymphony runtime overlays.
- Join cached tracker data and runtime data into task graph DTOs.
- Add invalidation and refresh behavior.

### Out of scope

- Linear create/update mutations.
- Planning artifact generation.
- Hosted Linear OAuth.

## Deliverables

- Linear query coverage matrix.
- GraphQL schema drift tests.
- Task graph cache.
- Sync and join tests.

## Acceptance Criteria

- [ ] Task graph reads can include Linear projects, milestones, issues, sub-issues, and relations.
- [ ] Runtime overlays are joined without requiring UI access to orchestrator internals.
- [ ] Schema drift is detected with a clear failure and remediation path.

## Test Plan

- Run Linear fake-server tests and schema drift checks.
- Run task graph cache join tests with mixed Linear and runtime fixture data.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.5 and `docs/host-client-implementation_plan.md` P4.1/P4.2.
- Relevant repo paths include `crates/opensymphony-linear`, checked-in GraphQL query assets, and gateway task graph DTOs.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This cache feeds both task graph editing and planning-session analysis.
