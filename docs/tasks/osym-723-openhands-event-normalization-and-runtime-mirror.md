---
id: OSYM-723
title: OpenHands Event Normalization And Runtime Mirror
milestone: "M3: Task Graph Operations And OpenHands Run UI"
priority: 1
estimate: 8
blockedBy: ["OSYM-703"]
blocks: ["OSYM-724", "OSYM-725", "OSYM-752"]
parent: null
---

## Summary

Normalize OpenHands agent-server runtime events into OpenSymphony event envelopes and maintain a runtime state mirror for run detail views.

## Scope

### In scope

- Add typed handling for high-value OpenHands events.
- Preserve unknown OpenHands events as raw JSON references.
- Map OpenHands events to OpenSymphony run lifecycle, terminal/log streams, and diagnostics.
- Maintain active conversation/session state, execution status, readiness, reconnect status, history sync status, and last known event cursor.
- Add contract fixtures from fake and pinned live server behavior where available.

### Out of scope

- Client timeline UI.
- Hosted runtime pool.
- Codex app-server normalization.

## Deliverables

- Event normalization module.
- Runtime state mirror.
- OpenHands contract tests.
- Unknown-event retention tests.

## Acceptance Criteria

- [ ] Known OpenHands events produce typed OpenSymphony event envelopes.
- [ ] Unknown event types are retained and visible through diagnostics without failing the run.
- [ ] Runtime mirror state matches REST history plus WebSocket event reconciliation.

## Test Plan

- Run fake OpenHands server tests for create, send, run, event history, WebSocket readiness, reconnect, and unknown events.
- Run compatibility tests against the pinned OpenHands version when the live fixture path is available.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.7 and `docs/host-client-architecture.md` 4.4.
- Preserve existing OpenHands invariants from AGENTS.md: HTTP operations, WebSocket runtime, readiness barrier, reconciliation, event dedupe, and conversation reuse.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Runtime attachment remains server-owned.
