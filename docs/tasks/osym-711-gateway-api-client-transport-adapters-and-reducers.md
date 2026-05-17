---
id: OSYM-711
title: Gateway API Client, Transport Adapters, And Reducers
milestone: "M7: Shared Client And Desktop Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-704", "OSYM-710"]
blocks: ["OSYM-712", "OSYM-713", "OSYM-717", "OSYM-741"]
parent: null
---

## Summary

Implement the shared gateway client, transport adapter contracts, and event-reduced frontend state model.

## Scope

### In scope

- Implement REST reads, event streams, binary stream placeholders, and action mutation calls.
- Define adapters for desktop local native, desktop local gateway, desktop remote gateway, browser gateway, and tests.
- Add reconnect, cursor replay, stale-data, degraded-stream, and error handling.
- Create reducers for connection state, entity cache, dashboard, task graph, run detail, terminal/log stream state, approvals, and planning sessions.

### Out of scope

- Tauri native channel implementation.
- Browser auth flows.
- Final UI styling.

## Deliverables

- Shared `api-client` package.
- Shared `state` package.
- Mock transport for tests.
- Reducer and reconnect tests.

## Acceptance Criteria

- [ ] All transport adapters reduce to the same state transitions.
- [ ] Snapshot fetch plus stream replay can rebuild dashboard, task graph, and run state from fixtures.
- [ ] Action receipts and correlated events are represented in client state.

## Test Plan

- Run frontend unit tests for transports and reducers.
- Run fixture replay tests with out-of-order, duplicate, reconnect, and stale-stream cases.

## Context

- Source sections: `docs/host-client-architecture.md` 5.2 and 5.3.
- Frontend state should treat the gateway as the contract boundary.
- Desktop local and hosted remote modes share the same product state even when their physical transports differ.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use deterministic fixture replay to make stream bugs reproducible.
