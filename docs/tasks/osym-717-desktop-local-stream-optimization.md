---
id: OSYM-717
title: Desktop Local Stream Optimization
milestone: "M7: Shared Client And Desktop Alpha"
priority: 3
estimate: 8
blockedBy: ["OSYM-704", "OSYM-711", "OSYM-713", "OSYM-715"]
blocks: ["OSYM-771"]
parent: null
---

## Summary

Implement the selected high-throughput local stream path for Tauri desktop while preserving the shared frontend contract.

## Scope

### In scope

- Connect frontend streams to the selected local host profile.
- Use in-process Rust channels when the host is embedded or directly attached.
- Use native local IPC when the host is a separate local process and benchmark results support it.
- Use Tauri channels from Rust backend to webview for high-volume frames where useful.
- Keep loopback HTTP/WebSocket fallback.
- Use zero-copy-friendly Rust frame buffers internally where practical.

### Out of scope

- Hosted WSS transport implementation.
- Full terminal emulator features.
- Codex app-server transport.

## Deliverables

- Desktop local transport adapter.
- Stream benchmark update.
- Contract tests proving local and remote transports produce equivalent frontend state.

## Acceptance Criteria

- [ ] Desktop local streams can use the best available profile and fall back to loopback gateway.
- [ ] Local and remote transports expose the same frontend event and frame semantics.
- [ ] Copies are limited to trust, process, and webview boundaries where practical.

## Test Plan

- Run local transport integration tests.
- Run renderer benchmarks over local native and fallback transports.
- Verify reconnect and cursor replay semantics remain consistent with gateway streams.

## Context

- Source sections: `docs/host-client-architecture.md` 3.1 and 4.8, `docs/host-client-implementation_plan.md` P3.8.
- The desktop app must support local orchestrator and hosted orchestrator profiles.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The local fast path is a transport optimization over the same gateway DTOs, cursors, frames, and action receipts.
