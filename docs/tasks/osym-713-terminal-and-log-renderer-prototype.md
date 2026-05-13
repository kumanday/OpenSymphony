---
id: OSYM-713
title: Terminal And Log Renderer Prototype
milestone: "M2: Shared Client And Desktop Alpha"
priority: 2
estimate: 5
blockedBy: ["OSYM-701", "OSYM-711"]
blocks: ["OSYM-717", "OSYM-724"]
parent: null
---

## Summary

Prototype a high-throughput terminal and log renderer that can support bursty OpenHands output without blocking the main UI.

## Scope

### In scope

- Implement worker-based decode and render loop.
- Support text log frames first.
- Add terminal cell delta frame support when the gateway schema provides it.
- Add scrollback, search, copy, and jump-to-latest.
- Measure frame rate, memory growth, and UI responsiveness.

### Out of scope

- Hosted stream broker implementation.
- Approval UI.
- Full terminal emulator parity.

## Deliverables

- Terminal/log pane prototype.
- Renderer benchmark harness.
- Fixture payloads for bursty logs and terminal output.

## Acceptance Criteria

- [ ] Representative log bursts render without blocking primary navigation interactions.
- [ ] Scrollback remains stable while live output arrives.
- [ ] Renderer benchmarks produce repeatable throughput and responsiveness numbers.

## Test Plan

- Run frontend renderer tests and benchmarks.
- Replay representative terminal/log fixtures from the stream feasibility task.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.2.3 and `docs/host-client-architecture.md` 4.8.
- High-volume streams may use binary frames for hosted/web and local native/Tauri channels for desktop.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep frame schemas versioned and decodable by TypeScript.
