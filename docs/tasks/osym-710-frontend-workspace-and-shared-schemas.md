---
id: OSYM-710
title: Frontend Workspace And Shared Schemas
milestone: "M7: Shared Client And Desktop Alpha"
priority: 1
estimate: 5
blockedBy: ["OSYM-701", "OSYM-702"]
blocks: ["OSYM-711", "OSYM-712", "OSYM-713", "OSYM-714", "OSYM-740"]
parent: null
---

## Summary

Create the shared TypeScript frontend workspace and schema layer used by both Tauri desktop and browser clients.

## Scope

### In scope

- Add frontend package structure for app shells, UI core, state, API client, terminal renderer, task graph UI, run UI, and planning UI.
- Configure TypeScript build, lint, test, and fixture workflows.
- Generate or define TypeScript models aligned with gateway v1 DTOs.
- Add schema version constants and runtime validation for stream payloads where needed.

### Out of scope

- Full page implementations.
- Tauri packaging.
- Browser deployment.

## Deliverables

- Frontend workspace skeleton.
- Shared schema package.
- Build and test commands.
- Schema compatibility tests.

## Acceptance Criteria

- [ ] Desktop and web entrypoints can depend on the same shared packages.
- [ ] Schema types cover capability, dashboard, task graph, run, event, terminal/log, approval, and planning payloads.
- [ ] Generated or hand-maintained schemas have a documented update path.

## Test Plan

- Run the frontend type check and unit test commands added by this task.
- Run schema fixture tests against gateway fixture payloads.

## Context

- Source sections: `docs/host-client-architecture.md` 5.1 and 10.2.
- Keep framework-specific state out of transport and schema packages where practical.
- The repo may need a new `apps/` and `packages/` workspace alongside the Rust crates.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Prefer a layout that keeps desktop-only and web-only code behind adapters.
