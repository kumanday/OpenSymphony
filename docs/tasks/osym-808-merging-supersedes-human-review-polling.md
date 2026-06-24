---
id: OSYM-808
title: Merging Supersedes Human Review Polling
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 2
estimate: 3
blockedBy: ["OSYM-805", "OSYM-806", "OSYM-807"]
blocks: ["OSYM-812"]
areas:
  - orchestrator
  - linear
  - workflow
parent: null
---

## Summary

When Linear moves an active issue from `Human Review` to `Merging`, interrupt the current review polling turn in the same conversation and continue the issue toward land or closeout.

## Scope

### In scope

- Detect tracker state changes from `Human Review` to `Merging` for active runs.
- Send the shared interrupt command with reason `tracker_merging_supersedes_human_review`.
- Stop enqueueing Human Review polling prompts for that issue.
- Continue the same issue run/conversation through the existing land or closeout path after acknowledgement or timeout diagnostics.
- Reconcile Linear and PR state before deciding whether to move the issue to `Done`.

### Out of scope

- Creating a second worker or conversation.
- Replacing the tracker polling loop.
- Changing `WORKFLOW.md` semantics beyond invoking the existing land/closeout behavior.

## Deliverables

- Orchestrator transition handling for Human Review to Merging.
- Run events documenting the supersede reason.
- Regression tests for same-conversation interrupt and closeout routing.

## Acceptance Criteria

- [ ] A Human Review polling turn is interrupted when Linear reports `Merging`.
- [ ] The same issue conversation continues toward land/closeout after interruption handling.
- [ ] Repeated tracker observations of `Merging` do not send duplicate interrupts.
- [ ] The run event stream records why the review loop stopped.

## Test Plan

- Run orchestrator tests with fake tracker state transitions.
- Run adapter-backed tests for both OpenHands and Codex interrupt acknowledgement paths when feasible.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Read `WORKFLOW.md`.
- Inspect `crates/opensymphony-cli/src/orchestrator_run/`.
- Inspect Linear state normalization in `crates/opensymphony-linear/` and orchestrator tracker polling.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task is about one active issue conversation. Do not phrase or implement it as a handoff between workers.
