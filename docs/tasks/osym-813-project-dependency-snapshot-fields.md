---
id: OSYM-813
title: Project Metadata For Operator Issue Snapshots
milestone: "M10.7: Project Grouping And Dependency Signals"
priority: 2
estimate: 3
blockedBy: []
blocks: ["OSYM-814", "OSYM-815"]
areas:
  - control
  - linear
  - gateway
parent: null
---

## Summary

Expose the missing project identity fields operator clients need to group visible issues by Linear project.

## Scope

### In scope

- Ensure control-plane issue snapshots include project slug or identifier, project display name when available, and repository or workspace label when available.
- Preserve the existing `blocked` and `blocked_by` fields and add only minimal blocker status metadata if the TUI cannot distinguish completed blockers from unfinished blockers with current data.
- Keep missing project or dependency data non-fatal for older snapshots and fake fixtures.
- Update schema, fixture, and serialization tests for the added fields.

### Out of scope

- Full dependency graph rendering.
- Desktop task graph dependency gutters, suffixes, or edge visualization.
- Duplicating existing `TaskGraphSnapshot.blocked_by` dependency derivation.
- Dependency-aware scheduling changes.
- TUI or desktop rendering.

## Deliverables

- Control-plane and gateway schema updates for operator-list project metadata.
- Fake snapshot fixtures that cover single-project, multi-project, and missing-project-data cases.
- Documentation note if a public DTO changes.

## Acceptance Criteria

- [ ] Operator clients can identify which Linear project each visible issue belongs to when project data exists.
- [ ] Existing blocked/blocked-by behavior remains backward-compatible.
- [ ] Minimal blocker status metadata is added only if required for the TUI list and is covered by tests.
- [ ] Missing project data degrades to absent display metadata rather than a snapshot error.
- [ ] Serialization and fixture tests cover the new fields.

## Test Plan

- Run focused gateway schema and control-plane serialization tests.
- Run fake snapshot tests that exercise missing project metadata.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/tui-dependency-gutter-spec.md`.
- Inspect `crates/opensymphony-control`.
- Inspect `crates/opensymphony-gateway-schema`.
- Inspect Linear normalization in `crates/opensymphony-linear`.
- The desktop app already derives dependency gutters, suffixes, hidden blockers, completed blockers, and graph edges from `TaskGraphSnapshot.blocked_by`; do not rebuild that path here.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not precompute a full graph. This task only fills the operator issue snapshot fields missing for project grouping.
