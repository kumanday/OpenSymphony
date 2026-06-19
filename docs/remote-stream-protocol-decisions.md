# Remote Stream Protocol Decision Notes

> COE-407 / OSYM-741 — Browser Transport And Remote Stream Protocols
> Milestone M10: Web Client And External Gateway
> Source: `docs/host-client-architecture.md` §7.2 and `docs/host-client-implementation_plan.md` P7.2.

This document records the remote transport and stream protocol decisions for the
OpenSymphony web client and the desktop remote/hosted profile baseline. Hosted
consistency takes priority over raw throughput.

## Decision summary

| Concern | Decision | Rationale |
|---|---|---|
| Reads & mutations | REST/HTTP (`HttpGatewayTransport`) | Idempotent, cacheable, well-tooled, matches existing gateway route inventory. |
| Event streams | WebSocket (text JSON) or SSE, selected from advertised capabilities | Ordered, cursor-resumable, lowest common denominator for browsers. |
| Terminal/log streams | Binary WebSocket frames **only when advertised** | High-volume frames; opt-in per gateway so clients never fork. |
| Bidirectional hosted control | JSON-RPC 2.0 over WebSocket — **evaluated, not selected as default** (see below) | Viable candidate; deferred pending benchmark evidence. |
| Consistency guarantees | Cursor replay + idempotency keys + action receipts + monotonic sequences for **every** remote transport | Required regardless of protocol. |

There are no client-side protocol forks: every profile resolves to one of the
shared transport implementations through `TransportFactory`, and optional
features (binary frames, SSE vs WebSocket) are selected from advertised
`GatewayCapabilities`.

## Required guarantees for any selected remote transport

These are non-negotiable and are enforced client-side by `StreamReplayBuffer`
and `StreamCorrelator` so a weak or reordered gateway cannot silently corrupt
state:

- **Cursor replay.** Each stream event carries a `StreamCursor`
  (`{ sequence, partition }`). On reconnect the client resumes from the last
  applied cursor via `events(fromCursor)`; the WebSocket reconnect URL carries
  `cursor_sequence` and `cursor_partition` query params. The replay buffer's
  `nextCursor(partition)` returns the resume point.
- **Monotonic event sequences.** Per partition, the client only emits events
  that advance the frontier (`last + 1`). Events at or below the frontier are
  suppressed as duplicates.
- **Idempotency keys.** Mutations carry an idempotency key so retries after a
  disconnect do not double-apply. Action dispatches include `correlation_id`.
- **Action receipts.** Every action returns an `ActionReceipt` with
  `correlation_id` and `expected_followup`. `StreamCorrelator` links streamed
  follow-up events back to their originating receipt by `correlation_id`.
- **Reconnect and stale-state.** WebSocket reconnect uses exponential backoff
  (capped). A partition is marked stale after a configurable idle window and
  recovered when a fresh event advances the frontier.

## WebSocket vs SSE for event streams

- **WebSocket** is preferred when the gateway advertises a `websocket` or
  `loopback_websocket` transport capability, because it is bidirectional,
  supports binary frames, and gives a single ordered channel.
- **SSE** is the fallback for read-only event streams when only `sse` is
  advertised. SSE is unidirectional, so control/mutations stay on HTTP and
  binary terminal frames are not available on that channel.
- Selection is driven entirely by advertised capabilities — the client does not
  hard-code a profile-to-protocol mapping.

## Binary WebSocket frames for terminal/log streams

Binary frames are used **only** for high-volume terminal/log streams and only
when `binaryFramesAdvertised(capabilities)` is true. The browser sets
`ws.binaryType = "arraybuffer"` on open when support is advertised, and the
transport decodes `ArrayBuffer` (and `Blob`) frames via the shared binary frame
codec. Text JSON envelopes remain the default for control and event streams.

Binary frame wire format (little-endian header, UTF-8 payload JSON):

```
u8   magic         = 0x4F ('O')
u8   version       = 1
u8   frame_type    = 1 (terminal_frame envelope)
u8   reserved      = 0
u32  sequence      (LE)
u16  partition_len (LE)
u8[] partition     (UTF-8)
u32  payload_len   (LE)
u8[] payload       (UTF-8 JSON of the envelope minus the binary header)
```

The header carries `sequence` and `partition` so the client can enforce
monotonic ordering without parsing the full JSON payload first. The codec
(`encodeBinaryFrame` / `decodeBinaryFrame`) is shared between the transport and
tests so the wire format is exercised in both directions, and a versioned magic
byte lets future frame types coexist.

## JSON-RPC 2.0 over WebSocket evaluation

JSON-RPC 2.0 over WebSocket (`json_rpc_over_websocket` profile) was evaluated
as a candidate for hosted bidirectional control and subscriptions. The profile
already exists in `TransportProfile`. The evaluation:

### Benefits
- **Single bidirectional channel** for requests, responses, notifications, and
  subscriptions, avoiding HTTP round-trips for control plane calls.
- **Structured correlation** via JSON-RPC `id` maps cleanly onto OpenSymphony
  `correlation_id`, improving retry/reply matching for hosted control.
- **Resumable subscriptions** can be modeled as JSON-RPC notifications with
  explicit subscription IDs and cursor positions.

### Constraints
- **Browser HTTP/2 vs WebSocket tradeoff**: REST/HTTP already handles reads and
  idempotent mutations well and is cacheable; JSON-RPC over a single WebSocket
  gives up per-request HTTP semantics (caching, load balancing, retry idempotency
  at the HTTP layer).
- **Backpressure and head-of-line blocking**: a single ordered channel can
  delay control messages behind a burst of terminal frames unless streams are
  demultiplexed by partition/frame type.
- **Connection lifecycle**: one WebSocket must carry both control and stream
  traffic, complicating partial reconnect (e.g. re-establishing only the event
  stream without re-handshaking control state).

### Auth requirements
- JSON-RPC must carry auth context on the WebSocket handshake **and** per
  request/notification (defense in depth), mirroring the bearer-token auth used
  on HTTP. Auth must be checked on every message, not just connection open.
- RBAC checks are out of scope for this ticket (hosted RBAC middleware is a
  separate concern) but must be enforceable per JSON-RPC method. This is tracked
  as a follow-up issue ([COE-472](https://linear.app/trilogy-ai-coe/issue/COE-472/hosted-gateway-rbac-enforcement-per-requestmethod),
  blocked by this ticket) so the auth-every-message requirement does not become
  an implicit security gap once hosted JSON-RPC is adopted.

### Replay semantics
- If selected, JSON-RPC **must still use** OpenSymphony event cursors,
  idempotency keys, action receipts, monotonic sequence numbers, and replay
  after reconnect. The protocol envelope does not replace these guarantees.
- Subscription notifications carry cursor positions; resubscribe-after-reconnect
  uses the last applied cursor, exactly like the WebSocket/SSE path.

### Decision
**Not selected as the default in this milestone.** REST/HTTP + WebSocket/SSE
remains the baseline because it is simpler, matches the existing gateway route
inventory, and keeps consistency guarantees identical across transports.
JSON-RPC over WebSocket remains a viable hosted-only upgrade if future
benchmarks show it improves correlation, retries, and subscription management
without weakening the replay/idempotency guarantees above. No client-side
protocol fork is required to adopt it later: it would be selected from
advertised capabilities like any other transport.

## Reconnect and stale-state behavior

- WebSocket reconnect uses exponential backoff starting at 1s, capped at 30s,
  reset to 1s on a successful open. (`scheduleReconnect`.)
- On reconnect, the client resumes the event stream from the last applied
  cursor; replayed events already applied are suppressed by `StreamReplayBuffer`.
- A partition idle longer than `staleAfterMs` is reported via `checkStale` /
  `onStale`; a fresh event that advances the frontier clears the stale flag
  (`onRecovered`). Stale detection is deterministic and testable without real
  timers via `StreamReplayBuffer.checkStale(now, staleAfterMs)`.

## Test coverage

- `packages/api-client/__tests__/stream-replay.test.ts` — replay buffer
  dedup, gap detection, out-of-order flush, seed/reconnect resume, stale
  detection, and action-receipt correlation.
- `packages/api-client/__tests__/reconnect-replay.test.ts` — disconnect →
  reconnect with cursor replay, duplicated events, dropped frames, stale
  states, and action-receipt correlation across the transport + buffer.
- `packages/api-client/__tests__/transport-contract.test.ts` — binary frame
  advertisement, `binaryType` selection, binary decode dispatch, and
  encode/decode round-trip.