---
id: OSYM-704
title: Event Journal And Stream Broker
milestone: "M1: Gateway And Stream Contract"
priority: 1
estimate: 8
blockedBy: ["OSYM-701", "OSYM-702"]
blocks: ["OSYM-705", "OSYM-711", "OSYM-724", "OSYM-741", "OSYM-753"]
parent: null
---

## Summary

Implement durable event records and replayable live streams for gateway, orchestrator, task graph, run, terminal/log, approval, and planning activity.

## Scope

### In scope

- Add event records with stable IDs, monotonic sequence numbers, schema version, actor, correlation ID, entity refs, timestamp, summary, payload, and raw payload references.
- Record orchestrator events, gateway action events, normalized harness events, and planning events.
- Support cursor reads and reconnect.
- Implement bounded queues, backpressure behavior, and connection state reporting.
- Split control events from high-volume terminal/log frames where benchmark results justify it.

### Out of scope

- Hosted audit log UI.
- Full terminal frame renderer.
- Planning artifact generation.

## Deliverables

- Event journal storage abstraction or implementation.
- Cursor query API.
- `/api/v1/events?cursor=`.
- Optional WebSocket event stream.
- Stream reconnect tests.

## Acceptance Criteria

- [ ] Clients can resume from a cursor and receive committed events in sequence.
- [ ] Duplicate events are identifiable and reducible by stable ID or sequence.
- [ ] Unknown harness payloads are retained through raw references for diagnostics.
- [ ] Stream errors and degraded states are visible to clients.

## Test Plan

- Run event schema tests, replay tests, reconnect tests, and bounded-queue tests.
- Simulate bursty run and terminal/log events with fake harness input.

## Context

- Source sections: `docs/host-client-architecture.md` 4.7, 4.8, 6.3, and 7.2.
- The frontend state model depends on initial snapshot plus event replay.
- The orchestrator owns execution state; the journal records state transitions for clients and diagnostics.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The event journal is also the hosted-mode reconnect foundation.
