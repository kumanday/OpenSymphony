---
id: OSYM-806
title: OpenHands Agent-Server Interrupt Adapter
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 2
estimate: 2
blockedBy: ["OSYM-805"]
blocks: ["OSYM-808", "OSYM-812"]
areas:
  - openhands-runtime
  - harness-runtime
parent: null
---

## Summary

Wire OpenHands cancellation to the agent-server `/interrupt` endpoint so active turns stop promptly instead of waiting for pause semantics.

## Scope

### In scope

- Add an OpenHands client method for `POST /api/conversations/{conversation_id}/interrupt` or the configured equivalent route.
- Treat `/interrupt` as the primary mid-turn stop mechanism.
- Record fallback diagnostics if an older server lacks `/interrupt` and the adapter must use `/pause`.
- Reconcile events/state before reporting acknowledgement to the orchestrator.

### Out of scope

- Changing OpenHands conversation reuse rules.
- Implementing Codex interruption.
- Adding a second worker or conversation for Human Review.

## Deliverables

- OpenHands adapter interrupt method.
- Request-shape and acknowledgement tests.
- Documentation note if the supported OpenHands agent-server contract changes.

## Acceptance Criteria

- [ ] OpenHands-backed runs send `/interrupt` for harness interrupts.
- [ ] The adapter distinguishes `/interrupt` from `/pause` in code and diagnostics.
- [ ] The adapter reports acknowledgement only after state/event reconciliation or a documented timeout path.
- [ ] Tests cover successful interrupt and fallback/error handling.

## Test Plan

- Run OpenHands client unit tests for interrupt request construction.
- Run focused session/orchestrator tests using a fake OpenHands agent-server response.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Inspect `crates/opensymphony-openhands/src/client.rs`.
- Inspect `crates/opensymphony-openhands/src/session.rs`.
- The installed agent-server documents `/interrupt` as immediate cancellation; do not use `/pause` as the default.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use the existing REST client style; no new transport layer.
