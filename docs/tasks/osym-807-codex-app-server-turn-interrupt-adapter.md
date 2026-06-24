---
id: OSYM-807
title: Codex App-Server Turn Interrupt Adapter
milestone: "M10.6: Desktop Run Detail Operations And Interrupts"
priority: 2
estimate: 3
blockedBy: ["OSYM-805"]
blocks: ["OSYM-808", "OSYM-812"]
areas:
  - codex-runtime
  - harness-runtime
parent: null
---

## Summary

Replace the stale Codex `turn/cancel` request with the current `turn/interrupt` JSON-RPC contract and retain active turn ids so cancellation can happen mid-turn.

## Scope

### In scope

- Change Codex interrupt requests to `turn/interrupt`.
- Send both `threadId` and `turnId`.
- Retain the latest active Codex turn id in run/session metadata long enough for cancellation.
- Normalize Codex `interrupted` turn status into the shared interrupt acknowledgement path.

### Out of scope

- Changing Codex thread creation or resume behavior.
- Implementing OpenHands interruption.
- Adding hosted Codex worker pools.

## Deliverables

- Codex request builder and session wiring for `turn/interrupt`.
- Event normalization for interrupted status.
- Tests for request shape and interrupted-status handling.

## Acceptance Criteria

- [ ] Codex-backed cancellation sends JSON-RPC method `turn/interrupt`.
- [ ] The request includes both the active `threadId` and active `turnId`.
- [ ] A Codex interrupted turn updates shared run diagnostics as acknowledged.
- [ ] The old `turn/cancel` request path is removed or no longer reachable.

## Test Plan

- Run `crates/opensymphony-codex` unit tests for request construction.
- Run focused adapter tests for event normalization.
- Run `cargo fmt --check`.

## Context

- Read `docs/specs/desktop-run-detail-operations-spec.md`.
- Inspect `crates/opensymphony-codex/src/lib.rs`.
- Inspect `docs/codex-app-server-harness.md`.
- Generate or inspect the current Codex app-server schema before changing protocol strings.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The current app-server schema exposes `turn/interrupt`; do not preserve `turn/cancel` as an undocumented compatibility path.
