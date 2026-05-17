---
id: OSYM-703
title: Task Graph, Run Detail, File, And Diff Read APIs
milestone: "M6: Gateway And Stream Contract"
priority: 1
estimate: 8
blockedBy: ["OSYM-701", "OSYM-702"]
blocks: ["OSYM-720", "OSYM-723", "OSYM-724", "OSYM-725"]
parent: null
---

## Summary

Expose read APIs for projects, task graph nodes, run details, run event history, changed files, and diffs.

## Scope

### In scope

- Implement project list and project detail endpoints.
- Implement project task graph reads with milestones, issues, sub-issues, and runtime overlays.
- Implement run detail reads with issue context, workspace, harness session, lifecycle state, summaries, and action capabilities.
- Implement event history, changed-files, and per-file diff reads with cursor or pagination support where needed.
- Add safety checks for hosted path abstraction and local workspace containment.

### Out of scope

- Linear mutations.
- Runtime stream delivery.
- Full diff viewer UI.

## Deliverables

- `/api/v1/projects`.
- `/api/v1/projects/{project_id}`.
- `/api/v1/projects/{project_id}/taskgraph`.
- `/api/v1/runs/{run_id}`.
- `/api/v1/runs/{run_id}/events`.
- `/api/v1/runs/{run_id}/files`.
- `/api/v1/runs/{run_id}/diffs`.

## Acceptance Criteria

- [ ] Task graph responses use Linear-native project, milestone, issue, and sub-issue names.
- [ ] Runtime overlays include eligibility, queue, active run, last outcome, retry, workspace, harness, diff, validation, and blocker summaries when available.
- [ ] File and diff reads avoid exposing unsafe local paths and can map to logical workspace identifiers for hosted mode.

## Test Plan

- Run gateway fixture tests for task graph, run detail, event history, files, and diffs.
- Run workspace containment tests for file and diff paths.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.1.2 and 4.5, `docs/host-client-architecture.md` 4.5 and 7.1.
- Related existing areas: `crates/opensymphony-linear`, `crates/opensymphony-control`, `crates/opensymphony-workspace`, and orchestrator state projection code.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

These APIs are the read foundation for the desktop app, web app, task graph editor, and run detail view.
