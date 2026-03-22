# WebSocket Runtime Contract

This document describes the WebSocket-first integration between OpenSymphony and OpenHands agent-server.

It is intentionally detailed because the runtime stream is the highest-risk adapter surface in the project.

## 1. Scope

This document covers:

- where the WebSocket fits into the runtime lifecycle
- which parts of the OpenHands contract are REST vs WebSocket
- how to attach, recover, and reconcile
- how to decode and store events
- how to translate runtime stream state into Symphony worker outcomes

It does not replace the Symphony poll loop. Linear polling remains a separate orchestrator concern.

## 2. Current contract to implement

## 2.1 Directionality

For the SDK agent-server contract used here:

- REST performs operations
- WebSocket delivers real-time events and state updates

Do not design the MVP around sending agent actions over the WebSocket.

## 2.2 Path shape

Use the SDK client behavior as the pinned wire-level reference.

Current observed path shape:

```text
ws://<host>/sockets/events/{conversation_id}
wss://<host>/sockets/events/{conversation_id}
```

If the host contains a base path, preserve it.

## 2.3 Auth modes

Support these modes in the Rust client:

- none
- query-param session API key fallback
- optional header-based auth for versions that support it

Default local MVP behavior: none.

## 3. Event model

## 3.1 High-value typed events

Typed decoding should exist at least for:

- `ConversationStateUpdateEvent`
- `LLMCompletionLogEvent`
- `ConversationErrorEvent` if present in the pinned version
- minimal generic base event envelope with:
  - `id`
  - `timestamp`
  - `source`
  - `kind` or discriminant
  - raw JSON payload

## 3.2 Unknown events

Unknown events must not crash the runtime.

Rules:

- preserve raw JSON
- assign a generic `UnknownEvent`
- store it in the event journal
- ignore it for state transitions unless a later version adds typed handling

## 3.3 State updates

`ConversationStateUpdateEvent` is the most important event type for this adapter.

Use it for:

- readiness barrier
- incremental conversation state mirror
- `execution_status` change tracking
- live snapshot updates without extra REST calls

## 4. WebSocket-first attachment algorithm

Use this sequence whenever attaching to a conversation, both fresh and existing.

### 4.1 Attachment sequence

1. Ensure `conversation_id` is known.
2. Perform an initial full event sync with `GET /api/conversations/{id}/events/search`.
3. Start the WebSocket connection.
4. Wait for the readiness barrier.
5. Reconcile events again with `GET /events/search`.
6. Only after readiness and reconcile is the stream considered attached.

### 4.2 Why the double sync exists

There is a race window between:

- initial REST event sync
- successful WebSocket subscription

Without reconciliation after readiness, events emitted in that window can be missed.

This is why the Rust runtime should intentionally copy the high-level behavior of the SDK remote client rather than simplifying it away.

## 5. Readiness barrier

The current SDK client treats the first `ConversationStateUpdateEvent` received after subscription as proof that the subscription is ready.

Implement the same rule.

Do not require that the first WebSocket frame be the readiness event.

The client should:

- keep waiting across ping and pong traffic
- ignore unrelated event kinds until a `ConversationStateUpdateEvent` arrives
- ignore one malformed or forward-compatible frame and continue waiting until timeout or socket close

Suggested API in Rust:

```rust
enum StreamReady {
    Ready,
    Timeout,
    Closed,
}
```

Configurable timeout:

- default `30000 ms`

If readiness is not achieved, fail the attach attempt and surface a transport error.

Current repository implementation:

- `opensymphony-openhands::OpenHandsClient::wait_for_readiness` loops until a `ConversationStateUpdateEvent` arrives from `/sockets/events/{conversation_id}`, while tolerating control frames and unrelated or undecodable events before readiness
- `opensymphony-openhands::OpenHandsClient::attach_runtime_stream` performs the full attach sequence: initial REST sync, WebSocket connect, readiness barrier, and post-ready reconcile before returning a live `RuntimeEventStream`
- the readiness frame is retained on `RuntimeEventStream::ready_event` as an attach barrier and diagnostic snapshot, but replayable runtime events still come from the reconciled event cache rather than the barrier frame itself
- `opensymphony-testkit` sends a state-update event immediately on WebSocket attach so readiness behavior is deterministic in CI
- `crates/opensymphony-openhands/tests/fake_server_contract.rs`, `crates/opensymphony-openhands/tests/client_resilience.rs`, and `crates/opensymphony-cli/tests/doctor.rs` cover the readiness, attach, and reconcile path

## 6. Event cache and reconciliation

## 6.1 Required behavior

The local event cache must:

- hold events in timestamp order
- deduplicate by event ID
- support full sync and incremental add
- allow replay into snapshot derivation
- survive reconnect cycles during one worker lifetime

## 6.2 Ordering

Do not assume WebSocket delivery order is strictly monotonic.

The cache should insert events by timestamp, not just append blindly.

## 6.3 Reconciliation API

Use `GET /api/conversations/{conversation_id}/events/search` with pagination.

The reconcile pass should:

- fetch all pages until no `next_page_id`
- merge only unseen event IDs
- update ordering
- return the number of new events added
- tolerate partial failure by preserving already-cached events

Current repository implementation:

- `OpenHandsClient::search_all_events` paginates until `next_page_id` is absent
- `EventCache` deduplicates by event ID, inserts by timestamp order, and can return the newly merged events from reconcile or reconnect passes
- `RuntimeEventStream` preserves one cache across reconnect cycles and replays ordered state updates into the state mirror after late arrivals
- the contract suite includes multi-page reconciliation, out-of-order insertion, and reconnect-recovery tests

## 6.4 Conversation state mirror

Maintain a conversation state mirror alongside the event cache.

Sources of truth:

- WebSocket `ConversationStateUpdateEvent` for fast incremental state
- `GET /api/conversations/{id}` for authoritative refresh on startup and reconnect

Wire-level compatibility note:

- the pinned source may emit both full-state snapshots and single-key state updates
- the Rust client should support both without depending on undocumented fields leaking into orchestrator code

Current repository implementation:

- `KnownEvent` now distinguishes `ConversationStateUpdateEvent`, `LLMCompletionLogEvent`, `ConversationErrorEvent`, and `UnknownEvent`
- unknown event kinds retain raw JSON in the event journal instead of failing the stream
- `ConversationStateMirror::rebuild_from` replays the timestamp-ordered cache so late state updates do not regress terminal detection
- `ConversationStateMirror::terminal_status` provides the current finished/error/stuck classification used by the probe and future workers

## 7. Run lifecycle over REST plus WebSocket

## 7.1 Sending a turn

For each turn:

1. Select prompt shape:
   - full prompt on fresh conversation
   - continuation guidance on resumed conversation or later turns
2. `POST /api/conversations/{id}/events`
   - user role
   - prompt content
   - `run=false`
3. `POST /api/conversations/{id}/run`
4. Observe progress through the WebSocket event stream

## 7.2 Waiting for completion

Primary mechanism:

- watch `ConversationStateUpdateEvent`
- detect `execution_status` entering a terminal state

Fallback mechanism:

- refresh `GET /api/conversations/{id}` if stream health is uncertain
- reconcile events after refresh
- classify the worker if the authoritative state is terminal

## 7.3 Terminal state queue

Implementation recommendation:

- maintain a small internal channel that receives terminal execution-status transitions
- let the worker await this channel with timeout and cancellation support
- keep REST fallback as backup, not as the main loop
- do not report success while a queued `ConversationErrorEvent` still exists in the pending stream buffer, even if the mirrored state has already reached `finished`

## 8. Disconnect and reconnect behavior

## 8.1 Failure modes to handle

- server restart
- network drop
- idle timeout
- daemon reconnect after a transient failure
- temporary auth mismatch
- event decode failure for one message

## 8.2 Reconnect policy

Use bounded exponential backoff:

- initial delay: `1000 ms`
- max delay: `30000 ms`

On reconnect:

1. refresh conversation info with REST
2. reconnect WebSocket
3. wait for readiness
4. reconcile events
5. resume streaming

If reconnection exhausts policy limits or the worker deadline, fail the worker and let the orchestrator schedule retry.

Current repository implementation:

- `RuntimeStreamConfig` carries readiness timeout, bounded exponential backoff, and max reconnect attempts
- `RuntimeEventStream::next_event` reconnects on both clean socket close and transport resets, then re-runs readiness plus reconcile before resuming
- reconnect readiness snapshots remain barriers only; they refresh `ready_event` but are not replayed as synthetic runtime events unless `/events/search` also returns them
- `opensymphony-testkit` can now force live socket drops so reconnect coverage is deterministic in CI

## 8.3 Decode failures

A single malformed or unknown event must not kill the stream unless the connection itself is corrupted.

Policy:

- log decode failure with raw payload hash or truncated payload
- keep connection alive if possible
- continue processing subsequent messages

## 9. Cancellation

Worker cancellation must close the WebSocket cleanly and stop waiting on terminal status.

Cancellation sources:

- orchestrator reconciliation
- daemon shutdown
- terminal issue state
- operator stop command in future control-plane versions

## 10. Suggested Rust internal API

## 10.1 Types

Suggested modules and types in `opensymphony-openhands`:

- `WsUrlBuilder`
- `WsAuthMode`
- `EventEnvelope`
- `KnownEvent`
- `UnknownEvent`
- `EventCache`
- `ConversationStateMirror`
- `RuntimeEventStream`
- `RunWatcher`
- `TerminalStatus`

## 10.2 Trait sketch

```rust
trait RuntimeEventStream {
    async fn attach(&mut self) -> Result<()>;
    async fn wait_ready(&mut self, timeout: Duration) -> Result<()>;
    async fn next_event(&mut self) -> Result<Option<RuntimeEvent>>;
    async fn reconcile(&mut self) -> Result<usize>;
    async fn close(&mut self) -> Result<()>;
}
```

Current repository implementation:

- `OpenHandsClient::attach_runtime_stream(conversation_id, RuntimeStreamConfig)` returns a live `RuntimeEventStream`
- `RuntimeEventStream` currently exposes `ready_event`, `event_cache`, `state_mirror`, `next_event`, and `close`
- `OpenHandsClient::wait_for_readiness` remains available as the narrow readiness helper used by lower-level tests and diagnostics

## 11. Relationship to Symphony worker state

The runtime stream informs, but does not define, Symphony outcomes.

Mapping examples:

- terminal `execution_status` `finished` with clean completion:
  - worker may continue another in-process turn or exit normally
- transport failure:
  - abnormal worker exit, schedule backoff retry
- no events for longer than `stall_timeout_ms`:
  - classify as stalled, terminate worker, schedule retry
- issue becomes terminal in Linear while stream is healthy:
  - orchestrator cancels worker regardless of OpenHands status

## 12. What not to do

- Do not use a polling-only runtime adapter and plan to refactor later.
- Do not use the OpenHands web-app Socket.IO examples for this stream.
- Do not assume the WebSocket alone makes the runtime state authoritative.
- Do not discard unknown events.
- Do not skip the post-ready reconcile step.
- Do not let TUI-specific streaming requirements leak into the runtime contract.

## 13. Test matrix for this component

Required automated scenarios:

- fresh attach with immediate readiness
- attach timeout
- out-of-order WebSocket events
- duplicate event IDs across REST and WebSocket
- disconnect before terminal status
- reconnect plus reconcile catches missed events
- terminal `execution_status` observed over WebSocket
- failure-only events such as `ConversationErrorEvent` do not count as successful completion
- REST fallback after stream uncertainty
- unknown event kind does not crash the stream

Live test scenarios:

- real local agent-server attach
- send prompt, trigger run, receive progress
- clean completion
- forced server restart and reconnect behavior
