---
id: OSYM-760
title: Harness Adapter And Capability Model
milestone: "M7: Provider, Harness, And Model Readiness"
priority: 2
estimate: 5
blockedBy: ["OSYM-701", "OSYM-723"]
blocks: ["OSYM-761", "OSYM-764", "OSYM-765"]
parent: null
---

## Summary

Finalize the harness adapter interface and capability model so OpenHands, Codex app-server, and future Rust-native adapters can share run contracts.

## Scope

### In scope

- Confirm the OpenHands adapter fits the shared interface.
- Confirm Codex app-server can fit through JSON-RPC request/response and notification handling.
- Confirm future Rust-native or in-process harness adapters can fit.
- Add harness capability DTOs for actions, event streams, approvals, model settings, transport, cancellation, pause/resume, and history.
- Expose capability metadata through gateway capabilities.

### Out of scope

- Codex production implementation.
- Subscription login.
- Cross-harness routing policy.

## Deliverables

- Stable `HarnessAdapter` interface or equivalent Rust boundary.
- Harness capability DTO.
- Capability tests.
- Adapter compatibility memo.

## Acceptance Criteria

- [ ] OpenHands works through the finalized adapter shape.
- [ ] Codex app-server and Rust-native adapters have documented fit notes and feature gaps.
- [ ] Clients can discover harness capabilities without knowing private adapter types.

## Test Plan

- Run existing OpenHands adapter tests through the finalized interface.
- Run capability serialization tests.
- Add fake harness tests for optional capability combinations.

## Context

- Source sections: `docs/host-client-architecture.md` 4.4 and `docs/host-client-implementation_plan.md` P9.1.
- Future harnesses should normalize into the same run, event, approval, and evidence contracts.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep the adapter interface shaped by current OpenHands needs and explicit future extension points.
