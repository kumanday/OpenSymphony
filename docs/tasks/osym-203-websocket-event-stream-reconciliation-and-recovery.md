---
id: OSYM-203
title: Implement WebSocket event stream, reconciliation, and recovery
type: feature
area: agent-runtime
priority: P0
estimate: 6d
milestone: M2 OpenHands runtime adapter
parent: OSYM-200
depends_on:
  - OSYM-201
  - OSYM-202
blocks:
  - OSYM-204
  - OSYM-401
  - OSYM-501
  - OSYM-601
project_context:
  - AGENTS.md
  - README.md
  - docs/websocket-runtime.md
  - docs/openhands-agent-server.md
  - docs/testing-and-operations.md
repo_paths:
  - crates/opensymphony-openhands/
  - crates/opensymphony-testkit/
definition_of_ready:
  - OSYM-201 and OSYM-202 are merged
  - Pinned WebSocket assumptions are documented
---

# OSYM-203: Implement WebSocket event stream, reconciliation, and recovery

## Summary
Implement the WebSocket-first runtime stream with the same high-level safety behavior as the current OpenHands remote client: initial sync, readiness barrier, post-ready reconcile, deduplication, timestamp ordering, and reconnect recovery.

## Scope
- Connect to `/sockets/events/{conversation_id}` with configurable auth modes
- Wait for the first `ConversationStateUpdateEvent` as the readiness barrier
- Run reconcile-after-ready through `events/search`
- Maintain an event cache ordered by timestamp and deduplicated by event ID
- Keep a conversation state mirror from streamed state updates plus REST refresh fallback
- Reconnect with bounded exponential backoff and re-reconcile after reconnect

## Out of scope
- Sending control operations through the WebSocket
- Implementing against the web-app Socket.IO protocol

## Deliverables
- WebSocket client
- Event decoder with raw unknown-event retention
- Reconciliation algorithm
- Reconnect and recovery logic

## Acceptance criteria
- Out-of-order events are stored correctly
- Duplicate events do not cause duplicated state transitions
- Disconnect and reconnect do not lose events across the attach window
- Terminal execution state can be detected from the streamed state model

## Test plan
- Contract tests for known event decoding
- Fake-server tests for attach, readiness, reconcile, and reconnect
- Live local run with intentional connection interruption if practical

## Notes
This is the highest-risk adapter task in the whole MVP. Keep the behavior small, explicit, and heavily tested.
