---
id: OSYM-805
title: Harness Interrupt Contract And Run Diagnostics
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 2
estimate: 3
blockedBy: []
blocks: ["OSYM-806", "OSYM-807", "OSYM-808", "OSYM-809", "OSYM-812"]
areas:
  - orchestrator
  - gateway
  - harness-runtime
parent: null
---

## Summary

Add the shared orchestrator and gateway contract for interrupting an active run and surfacing cancel diagnostics without hard-coding a harness-specific protocol into the UI.

## Scope

### In scope

- Define a harness-neutral interrupt command with run id, issue id, harness kind, conversation/thread id, optional turn id, reason, and expected next state.
- Surface cancel requested, acknowledged, failed, timeout, and reason fields through run diagnostics.
- Make repeated interrupt requests idempotent for repeated tracker observations or button clicks.
- Keep scheduler state ownership inside the orchestrator.

### Out of scope

- Implementing the OpenHands HTTP call.
- Implementing the Codex JSON-RPC call.
- Redesigning unrelated run actions.

## Deliverables

- Domain/orchestrator interrupt command and status model.
- Gateway schema updates for run diagnostics.
- Focused tests for idempotent interrupt command state.

## Acceptance Criteria

- [ ] The orchestrator can record an interrupt request with reason `operator_cancel`.
- [ ] The orchestrator can record an interrupt request with reason `tracker_merging_supersedes_human_review`.
- [ ] Run Detail data exposes pending, acknowledged, and failed cancel states without relying on desktop-local state.
- [ ] Repeated interrupt requests for the same active run do not enqueue duplicate harness interrupts.

## Test Plan

- Run focused orchestrator/domain tests for interrupt state transitions.
- Run gateway schema round-trip tests for the new diagnostic fields.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Inspect `packages/gateway-schema/src/run.ts`.
- Inspect orchestrator run state in `crates/opensymphony-cli/src/orchestrator_run/`.
- Keep the desktop and TUI as clients of orchestrator-owned state.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task should create the narrow contract used by later adapter and UI tasks. Do not add a new abstraction beyond the existing harness boundary.
