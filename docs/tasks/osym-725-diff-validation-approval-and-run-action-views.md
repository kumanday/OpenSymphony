---
id: OSYM-725
title: Diff, Validation, Approval, And Run Action Views
milestone: "M8: Task Graph Operations And OpenHands Run UI"
priority: 2
estimate: 8
blockedBy: ["OSYM-703", "OSYM-705", "OSYM-712", "OSYM-724"]
blocks: ["OSYM-752", "OSYM-770"]
parent: null
---

## Summary

Complete the rich run detail experience with diffs, validation evidence, approval handling, and supported run actions.

## Scope

### In scope

- Show changed files and per-file diffs.
- Show validation commands, results, and evidence summaries.
- Normalize approval requests where supported.
- Add pending approval list and approve, deny, and explain actions.
- Add run action bar for retry, cancel, rehydrate, comment, follow-up issue/sub-issue, workspace open, and debug view where available.
- Add audit trail rendering for decisions and actions.

### Out of scope

- Hosted user management.
- Codex approval bridge.
- Linear planning publish flow.

## Deliverables

- Diff viewer.
- Validation evidence UI.
- Approval center v1.
- Run action bar.
- Action UI tests.

## Acceptance Criteria

- [ ] Users can inspect changed files, diffs, and validation outcomes for a run.
- [ ] Approval requests show actor, target, command/file/issue/run context, and risk summary when available.
- [ ] Run actions use gateway action receipts and show correlated event outcomes.

## Test Plan

- Run UI fixture tests for diff, validation, approval, and action states.
- Run gateway action integration tests with fake run data.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.1.4, 4.11, and `docs/host-client-implementation_plan.md` P5.5 through P5.7.
- Approval prompts must be clear enough for hosted audit and local operator use.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Completion evidence should be easy to attach to Linear comments in later workflows.
