---
id: OSYM-812
title: Desktop Operations Integration Hardening
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 3
estimate: 2
blockedBy: ["OSYM-805", "OSYM-806", "OSYM-807", "OSYM-808", "OSYM-809", "OSYM-810", "OSYM-811"]
blocks: []
areas:
  - desktop
  - orchestrator
  - testing
parent: null
---

## Summary

Add the small cross-cutting verification pass for M10.6 so the interrupt, Run Detail, and launcher changes work together instead of only in isolated unit tests.

## Scope

### In scope

- Add or update integration tests for operator cancel through the gateway to a harness adapter.
- Add a regression covering Human Review to Merging supersede with interrupt diagnostics.
- Add a desktop smoke check or documented manual verification for Debug, Workspace, token display, PR link, and colored file stats.
- Update docs only where operator behavior changed.

### Out of scope

- Full hosted-mode validation.
- A new end-to-end test framework.
- Publishing the planning wave to Linear.

## Deliverables

- Focused integration/regression coverage for the completed M10.6 behavior.
- Operator-facing doc updates if commands or UI behavior changed.
- Final verification notes for the milestone.

## Acceptance Criteria

- [ ] A gateway cancel path reaches at least one fake or real harness interrupt implementation in tests.
- [ ] Human Review to Merging supersede is covered by a regression test.
- [ ] Desktop Run Detail action and TUI-parity changes are covered by UI or smoke checks.
- [ ] `opensymphony app` and `opensymphony desktop` have CLI coverage.
- [ ] Documentation reflects any changed operator commands or behavior.

## Test Plan

- Run the focused Rust tests added by OSYM-805 through OSYM-808 and OSYM-811.
- Run the focused TypeScript tests added by OSYM-809 and OSYM-810.
- Run `git diff --check`.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Review the completed sibling tasks before adding integration coverage.
- Prefer the existing test harnesses; add no new framework unless an existing one cannot exercise the behavior.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This is a final thin hardening slice, not a dumping ground for unfinished implementation.
