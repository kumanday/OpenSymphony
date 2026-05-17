---
id: OSYM-702
title: Gateway Module, Capabilities, And Dashboard Snapshot
milestone: "M6: Gateway And Stream Contract"
priority: 1
estimate: 5
blockedBy: ["OSYM-700", "OSYM-701"]
blocks: ["OSYM-703", "OSYM-704", "OSYM-705", "OSYM-710"]
parent: null
---

## Summary

Create the versioned gateway module boundary and expose capability discovery plus the first stable dashboard snapshot endpoint.

## Scope

### In scope

- Add or reorganize the gateway module inside the OpenSymphony host.
- Define `/api/v1` version metadata.
- Implement `/api/v1/capabilities`.
- Implement `/api/v1/dashboard/snapshot` from existing control-plane state.
- Add fixtures and compatibility tests for capability and snapshot payloads.

### Out of scope

- Task graph detail reads.
- Run event journal.
- Mutating actions.

## Deliverables

- Gateway module skeleton.
- Public DTO module or crate.
- Capability endpoint.
- Dashboard snapshot endpoint and tests.

## Acceptance Criteria

- [ ] `/api/v1/capabilities` reports API version, stream modes, actions, harness adapters, tracker adapters, planning availability, and experimental flags.
- [ ] `/api/v1/dashboard/snapshot` includes health, Linear sync, harness health, active runs, queue state, retries, recent events, sequence, and timestamp.
- [ ] Existing CLI and TUI behavior remains compatible.

## Test Plan

- Run `cargo test` for gateway DTO and endpoint tests.
- Write JSON fixtures for capability and dashboard snapshot responses.

## Context

- Source sections: `docs/host-client-implementation_plan.md` P1.1 through P1.3.
- Existing control-plane code is the starting point, with public DTOs shielding private orchestrator internals.
- Capabilities drive desktop local, external gateway, browser, hosted, planning, and future harness feature flags.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep capability names stable enough for frontend adapter selection.
