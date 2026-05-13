---
id: OSYM-701
title: Gateway Schemas And Stream Feasibility
milestone: "M1: Gateway And Stream Contract"
priority: 1
estimate: 5
blockedBy: ["OSYM-700"]
blocks: ["OSYM-702", "OSYM-703", "OSYM-704", "OSYM-710", "OSYM-713", "OSYM-717", "OSYM-741"]
parent: null
---

## Summary

Draft the gateway v1 schemas and benchmark the stream options needed for browser, hosted, and high-throughput Tauri desktop operation.

## Scope

### In scope

- Define draft Rust structs or JSON schemas for snapshots, events, task graph nodes, runs, terminal frames, approvals, planning artifacts, capabilities, cursors, and action receipts.
- Benchmark SSE, WebSocket, binary terminal/log frames, JSON-RPC 2.0 over WebSocket, Tauri channels, local native IPC, and in-process Rust channel delivery.
- Recommend the stream split for control events, terminal/log frames, snapshots, detail reads, and optional JSON-RPC control sessions.
- Record throughput, latency, replay, and reconnect expectations.

### Out of scope

- Production stream broker.
- Frontend rendering implementation.
- Hosted authentication middleware.

## Deliverables

- `gateway-schema-v1` draft.
- Stream benchmark report.
- Recommended local desktop transport order and remote hosted transport strategy.

## Acceptance Criteria

- [ ] Schema drafts include version fields, event sequence fields, cursor fields, entity references, and raw payload preservation references.
- [ ] Benchmarks compare loopback gateway, local native, Tauri channel, and hosted remote candidates with representative terminal/log output.
- [ ] The recommendation covers JSON-RPC 2.0 over WebSocket as an evaluated hosted control envelope.

## Test Plan

- Add benchmark commands or scripts with documented input fixtures.
- Run schema serialization tests for representative snapshot, event, terminal frame, and planning artifact payloads.

## Context

- Source sections: `docs/host-client-implementation_plan.md` P0.3 and P0.4, `docs/host-client-architecture.md` sections 4.7, 4.8, and 7.
- Desktop local mode should prefer in-process Rust channels, native IPC, Tauri channels, then loopback HTTP/WebSocket when supported by packaging.
- Hosted remote mode should preserve cursor replay, action receipts, idempotency, monotonic sequences, and RBAC hooks.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use stable public schemas even when the first implementation stores data through existing local mechanisms.
