---
id: OSYM-905
title: ACP Extensions And Harness Operations
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 8
blockedBy:
- OSYM-904
blocks:
- OSYM-906
- OSYM-841
areas:
- acp
- gateway
- runtime
parent: null
---

## Summary

Support configured custom ACP requests, notifications and metadata in both directions, including operator-invoked harness operations and a verified vendor callback.

## Scope

### In scope

- Use SDK typed/raw hooks for a small registered extension table: namespace/version, direction, capability predicate, method, parameter/result validation, deadline and effect policy.
- Preserve permitted _meta and unknown redacted evidence; implement explicit handling for inbound requests and notifications. Use descriptors for simple operations and Rust handlers for stateful/domain behavior.
- Wire a registered HarnessOperation operator action through authorization, orchestrator/session binding, runtime invocation and result/error receipt. Never accept an arbitrary wire method/session from clients.
- Implement version-tested Cursor compatibility for its documented non-underscore method names using captured envelopes; route blocking questions/plans into OSYM-904 and known notifications into activity/artifact projections.
- Expose enabled operation schemas and capability predicates for downstream IDE negotiation. Peer support never implies permission, and timeouts on mutations remain outcome-unknown.

### Out of scope

- Dynamic plugin loading, a scripting runtime, invented Devin operations or implementation of an Exo ACP facade.

## Deliverables

- Bidirectional extension registry, at least one real vendor handler and a complete outbound operator operation path.
- Capability/schema fixtures and redacted wire evidence for supported versions.

## Acceptance Criteria

- [ ] A blocking vendor request reaches an operator, validates the reply and responds exactly once; notifications emit no RPC response and request ID 0 remains valid.
- [ ] An enabled outbound operation is discoverable and invoked through the production operator action path with the correct bound session and structured result.
- [ ] Unknown requests fail with method-not-found; unknown notifications remain bounded evidence; malformed/disabled/version-mismatched methods cannot bypass policy or hang the harness.
- [ ] Namespaced metadata survives supported forwarding without becoming executable input; lifecycle mutations still pass through orchestrator commands.
- [ ] At least one vendor interaction is validated with a pinned real harness, while a deterministic fake extension tests the outbound path when no live safe operation exists.

## Test Plan

- Test inbound/outbound requests, notifications, metadata, schema validation, ID correlation, deadlines, cancellation, rejected effects and no unsafe retry.
- Capture redacted Cursor frames and verify actual request/notification envelopes before enabling compatibility registrations.

## Context

- docs/specs/acp-harness-adapter.md: custom extensions and concrete vendor handling.
- https://agentclientprotocol.com/protocol/v1/extensibility; https://cursor.com/docs/cli/acp; existing operator action authorization/receipts.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

Use the generic acp harness kind for every vendor profile. Supporting an extension means implementing its response/lifecycle semantics, not merely logging its name.
