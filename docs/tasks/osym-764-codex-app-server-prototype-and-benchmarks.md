---
id: OSYM-764
title: Codex App-Server Prototype And Benchmarks
milestone: "M10.3: Codex And Subscription Readiness"
priority: 3
estimate: 8
blockedBy: ["OSYM-760", "OSYM-761"]
blocks: ["OSYM-765", "OSYM-766", "OSYM-767"]
parent: null
---

## Summary

Build a feature-gated Codex app-server local stdio prototype and benchmark loopback WebSocket behavior before production enablement.

## Scope

### In scope

- Launch `codex app-server` over stdio.
- Initialize JSON-RPC session.
- Start a thread and turn.
- Read notifications and normalize basic events.
- Add schema generation to CI or dev tooling where supported.
- Reuse model and credential settings.
- Benchmark loopback WebSocket throughput, queue behavior, reconnect behavior, auth flags, and exposure controls.

### Out of scope

- Production Codex harness enablement.
- Hosted Codex runtime pool.
- Cross-harness routing.

## Deliverables

- Feature-gated Codex local prototype.
- Basic Codex contract tests.
- Codex WebSocket benchmark report.
- Production readiness recommendation.

## Acceptance Criteria

- [ ] Local stdio prototype can start a JSON-RPC session and normalize basic thread/turn notifications.
- [ ] WebSocket benchmark covers throughput, reconnect, queue behavior, and secure exposure requirements.
- [ ] Codex reuse of model and credential settings is demonstrated or documented with gaps.

## Test Plan

- Run feature-gated Codex fake or local contract tests.
- Run benchmark scripts against stdio and loopback WebSocket where the installed Codex version supports them.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.8 and `docs/host-client-architecture.md` 8.4.
- Codex app-server support is future scope; this task establishes fit and evidence.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep WebSocket behavior feature-gated until benchmark and security results support use.
