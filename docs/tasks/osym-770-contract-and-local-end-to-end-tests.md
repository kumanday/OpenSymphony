---
id: OSYM-770
title: Contract And Local End-To-End Tests
milestone: "M8: Hardening And Release Quality"
priority: 1
estimate: 8
blockedBy: ["OSYM-724", "OSYM-725", "OSYM-736", "OSYM-765"]
blocks: ["OSYM-771", "OSYM-772"]
parent: null
---

## Summary

Build the contract and local end-to-end test suites for gateway schemas, streams, Linear, OpenHands, planning, and desktop local workflows.

## Scope

### In scope

- Add gateway schema tests.
- Add event replay and stream reconnect tests.
- Add Linear fake server tests.
- Add OpenHands fake server tests.
- Add Codex fake server tests when enabled.
- Add auth and RBAC contract tests where hosted auth exists.
- Add local E2E tests that start OpenSymphony, connect desktop, load dashboard/task graph/run views, render timeline/logs/diffs, and retry or rehydrate a run.

### Out of scope

- Hosted multi-user E2E.
- Performance gates.
- Accessibility review.

## Deliverables

- Contract test suite.
- Local E2E test suite.
- Test fixtures for gateway, Linear, OpenHands, Codex, and planning flows.

## Acceptance Criteria

- [ ] Contract tests cover schema compatibility, event replay, stream reconnect, and fake external services.
- [ ] Local E2E tests prove the desktop can monitor and act on a local OpenSymphony run.
- [ ] Planning draft and publish fixtures validate the milestone/issue/sub-issue workflow.

## Test Plan

- Run `cargo test` for Rust contract and fake-server tests.
- Run frontend and desktop E2E commands added by this task.
- Document any live-service tests that require credentials.

## Context

- Source sections: `docs/host-client-implementation_plan.md` P10.1/P10.2.
- Keep tests compatible with existing CLI and TUI behavior.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use fake services for repeatable CI coverage and live services only for explicit integration runs.
