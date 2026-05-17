---
id: OSYM-741
title: Browser Transport And Remote Stream Protocols
milestone: "M10: Web Client And External Gateway"
priority: 1
estimate: 8
blockedBy: ["OSYM-704", "OSYM-711", "OSYM-740"]
blocks: ["OSYM-742", "OSYM-750"]
parent: null
---

## Summary

Implement browser HTTP and streaming transport with reconnect, cursor replay, action receipts, and remote protocol evaluation.

## Scope

### In scope

- Use HTTP for reads and mutations.
- Use WebSocket or SSE for event streams based on gateway capabilities.
- Use binary WebSocket for terminal/log streams when enabled.
- Evaluate JSON-RPC 2.0 over WebSocket for hosted bidirectional control and subscriptions.
- Support explicit external gateway/base URL profiles with consistent HTTP and stream auth behavior.
- Document remote daemon-to-server topology choices and transport configuration boundaries.
- Require cursor replay, idempotency keys, action receipts, and monotonic event sequences for any selected remote transport.
- Add reconnect and stale-state behavior.

### Out of scope

- Hosted RBAC middleware.
- Desktop local native transport.
- Final production selection of Codex app-server WebSocket behavior.

## Deliverables

- Browser transport adapter.
- Remote stream protocol decision notes.
- Remote transport configuration notes for local orchestrator, external gateway, and hosted gateway profiles.
- Reconnect tests.
- Origin and CORS preparation notes for separate deployment.

## Acceptance Criteria

- [ ] Browser transport can fetch snapshots/details and apply streamed events with cursor replay.
- [ ] Terminal/log streams can use binary WebSocket frames when the gateway advertises support.
- [ ] JSON-RPC 2.0 over WebSocket evaluation records benefits, constraints, auth requirements, and replay semantics.
- [ ] Remote gateway profiles work through configuration and advertised capabilities without client-side protocol forks.

## Test Plan

- Run browser transport unit and integration tests.
- Simulate disconnect, reconnect, duplicated events, dropped frames, and stale stream states.
- Verify action receipts correlate with streamed events.

## Context

- Source sections: `docs/host-client-architecture.md` 7.2 and `docs/host-client-implementation_plan.md` P7.2.
- Hosted consistency takes priority over raw throughput.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The browser transport is also the desktop remote hosted profile baseline.
