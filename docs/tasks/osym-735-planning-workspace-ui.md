---
id: OSYM-735
title: Planning Workspace UI
milestone: "M9: Collaborative Planning Alpha"
priority: 2
estimate: 8
blockedBy: ["OSYM-712", "OSYM-722", "OSYM-730", "OSYM-734"]
blocks: ["OSYM-736", "OSYM-742"]
parent: null
---

## Summary

Build the collaborative planning workspace for conversation, artifacts, hierarchy editing, dependency graph review, validation, and plan diffs.

## Scope

### In scope

- Add conversation pane.
- Add structured artifact pane.
- Add repository analysis and research panes.
- Add milestone/issue/sub-issue hierarchy editor.
- Add dependency editor and graph view.
- Add acceptance criteria and verification editors.
- Add plan validation UI.
- Add diff view between artifact revisions.

### Out of scope

- Linear mutation execution.
- Hosted team collaboration controls.
- Full AI model configuration UI.

## Deliverables

- Planning workspace.
- Artifact editors.
- Dependency graph view.
- Plan validation UI.
- UI tests for editing, diffing, and validation states.

## Acceptance Criteria

- [ ] Users can review and edit intake, research, codebase analysis, requirements, hierarchy, dependencies, acceptance criteria, and verification expectations.
- [ ] Users can see plan diffs between artifact revisions.
- [ ] Validation messages link to the artifact fields that need review.

## Test Plan

- Run planning UI component tests with fixture sessions and artifact revisions.
- Run keyboard navigation and focus checks for the planning workspace.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.6.2 and `docs/host-client-architecture.md` 5.4.
- The planning workspace should feel like a task-creation tool, with Linear as the publishing target.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep the first UI dense, editable, and review-oriented.
