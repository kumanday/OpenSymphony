# Stream Benchmark Report

## Methodology

Benchmarks use representative terminal/log JSON payloads (~380 bytes/frame)
measured on a macOS development host. All tests run in debug mode; release
mode numbers would be significantly higher.

## Results

| Transport | Messages | Throughput Gate | Latency | Reconnect | Replay | Binary Frames |
|-----------|----------|-----------------|---------|-----------|--------|---------------|
| In-process tokio mpsc | 100,000 | > 50 MB/s | ~0 µs | N/A (in-process) | N/A | Yes |
| Unix domain socket | 50,000 | > 10 MB/s | ~1-5 µs | Manual | N/A | Yes |
| Loopback WebSocket | 10,000 | > 5 MB/s | ~50-200 µs | Supported | Via cursor | Yes |
| SSE loopback | n/a | > 5 MB/s (estimated) | ~100-300 µs | Supported | Via cursor | No |
| JSON-RPC 2.0 over WS | n/a | Same as WS + ~15-25% overhead | Same as WS | Supported | Via cursor | Yes |

## Overhead measurements

- **JSON-RPC 2.0 envelope**: ~15-25% overhead for typical terminal frames.
- **SSE line framing**: ~10-15% overhead for typical terminal frames.

## Recommendations

### Local desktop transport order

1. **In-process Rust channels** — fastest, zero-copy when embedded.
2. **Native IPC** (Unix domain sockets / named pipes) — separate process, still loopback.
3. **Tauri channels** — from Rust backend into webview for high-volume frames.
4. **Loopback HTTP/WebSocket** — compatibility path and baseline contract test path.

All four paths expose the same gateway DTOs, event envelopes, cursors, and action
receipts. Local native transport is a performance optimization; it must not bypass
the event journal, permission checks, or orchestrator-owned state transitions.

Codex app-server loopback WebSocket is a feature-gated future harness transport,
not an OpenSymphony gateway transport. It speaks the Codex JSON-RPC contract
rather than gateway DTOs, so production exposure must be justified with
`scripts/codex_app_server_benchmark.mjs` evidence for throughput, queue behavior,
reconnect, and secure exposure.

### Remote hosted transport strategy

- **Primary**: WebSocket with JSON text frames for control events and detail reads.
- **High-volume streams**: Binary WebSocket frames for terminal/log output.
- **Optional control envelope**: JSON-RPC 2.0 over WebSocket for bidirectional
  request/response and notification routing. Use it when:
  - Multiple concurrent subscriptions per connection are needed.
  - Request/response correlation (action receipts, approval decisions) must be
    explicit.
  - A standard envelope simplifies client SDK generation.
- **Fallback**: SSE for simple snapshot streams where WebSocket is unavailable.

For Codex app-server specifically, stdio remains the preferred local prototype
transport. Loopback WebSocket is experimental and must keep secure exposure
controls enabled before any non-local use: localhost-only by default,
capability-token or signed-bearer authentication for exposed listeners, and
repeatable reconnect/queue throughput evidence for the supported Codex version
range.

### Stream split

| Data type | Transport | Notes |
|-----------|-----------|-------|
| Control events (state changes, run lifecycle) | WebSocket JSON or JSON-RPC | Small, ordered, idempotent |
| Terminal/log frames | WebSocket binary or Tauri channel | High volume, lossy acceptable |
| Snapshots | REST + SSE or WebSocket push | Periodic, full replacement |
| Detail reads (events, files, diffs) | REST with pagination | On demand, cursor replay |
| JSON-RPC control sessions | WebSocket JSON-RPC | Optional, for hosted mode |

## Hosted requirements

Remote hosted mode must preserve:

- **Cursor replay**: clients reconnect and resume from last `sequence`.
- **Action receipts**: every mutation returns a correlation ID and expected events.
- **Idempotency**: `ActionDispatch.idempotency_key` guards duplicate submits.
- **Monotonic sequences**: per-partition `sequence` never decreases.
- **RBAC hooks**: every stream attach and mutation checks auth before delivery.
