---
id: OSYM-816
title: Project Grouping Integration Hardening
milestone: "M10.7: Project Grouping And Dependency Signals"
priority: 3
estimate: 2
blockedBy: ["OSYM-814", "OSYM-815"]
blocks: []
areas:
  - tui
  - desktop
  - testing
parent: null
---

## Summary

Verify the project grouping and dependency-signal work end to end across fake snapshots, TUI rendering, and desktop grouping.

## Scope

### In scope

- Add or update cross-client fixtures that include multiple projects, visible blockers, hidden blockers, and completed blockers.
- Verify TUI and desktop agree on project grouping counts and fallback behavior.
- Update operator docs only where behavior changed.
- Capture final verification commands for the wave.

### Out of scope

- Hosted-mode permissions testing.
- New end-to-end framework.
- Dependency-aware sorting or scheduling.

## Deliverables

- Cross-client fixture coverage for grouped project snapshots.
- Final TUI and desktop smoke or regression checks.
- Documentation updates for operator-visible grouping behavior.

## Acceptance Criteria

- [ ] TUI and desktop fixtures use the same project and dependency metadata shape.
- [ ] Multi-project, single-project, missing-project, hidden-blocker, and completed-blocker cases are covered.
- [ ] TUI and desktop tests pass together for the grouped snapshot cases.
- [ ] Documentation reflects the visible grouping and collapse behavior.

## Test Plan

- Run focused Rust TUI/control-plane tests from `OSYM-813` and `OSYM-814`.
- Run focused TypeScript UI tests from `OSYM-815`.
- Run `git diff --check`.

## Context

- Read `docs/specs/tui-dependency-gutter-spec.md`.
- Review the completed sibling tasks before adding integration coverage.
- Prefer existing fixture and smoke-test paths over a new harness.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This is a thin verification slice, not a place for new features.

