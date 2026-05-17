---
id: OSYM-734
title: Dependency Graph And Plan Checks
milestone: "M9: Collaborative Planning Alpha"
priority: 1
estimate: 5
blockedBy: ["OSYM-733"]
blocks: ["OSYM-735", "OSYM-736"]
parent: null
---

## Summary

Generate and validate the dependency graph across milestones, issues, and sub-issues before Linear publishing.

## Scope

### In scope

- Generate dependency graph edges across milestones, issues, and sub-issues.
- Detect cycles and missing blockers.
- Identify parallelizable work.
- Validate `docs/tasks/task-package.yaml`, the explicit task file list, parent references, and blocker references.
- Validate dependency data used by manifest-driven conversion and `docs/tasks/linear-publish.yaml` receipt creation.
- Flag unclear scope, missing acceptance criteria, missing verification expectations, missing research artifacts, and missing codebase analysis artifacts.
- Store graph and check results as planning artifacts.

### Out of scope

- Runtime scheduling policy changes.
- Graph visualization polish.
- Linear relation mutation execution.

## Deliverables

- Dependency graph generator.
- Plan quality checks.
- Task package validation checks.
- Plan validation artifact.
- Tests for cycles, blockers, and missing fields.

## Acceptance Criteria

- [ ] Dependency graph output includes nodes, edges, reasons, and source artifact references.
- [ ] Cycles and missing blockers are detected before publish.
- [ ] Manifest validation reads task paths from `docs/tasks/task-package.yaml` and catches missing task files, unknown milestones, unknown dependencies, and creation-order cycles.
- [ ] Plan checks cover scope clarity, research coverage, codebase analysis, dependencies, acceptance criteria, and verification expectations.

## Test Plan

- Run graph validation tests for acyclic, cyclic, missing blocker, and parallelizable-work fixtures.
- Verify plan validation artifacts render in the planning session API.

## Context

- Source sections: `PRODUCT.md` section 8 and `docs/host-client-implementation_plan.md` P6.6.
- The graph supports planning review and Linear relation creation.
- The graph preserves `planningWave` with task package validation results and publish receipt inputs.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

These checks provide the main quality guard for the task-creation workflow.
