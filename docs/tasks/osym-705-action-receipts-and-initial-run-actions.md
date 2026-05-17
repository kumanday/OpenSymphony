---
id: OSYM-705
title: Action Receipts And Initial Run Actions
milestone: "M6: Gateway And Stream Contract"
priority: 1
estimate: 5
blockedBy: ["OSYM-702", "OSYM-704"]
blocks: ["OSYM-721", "OSYM-725", "OSYM-736", "OSYM-750"]
parent: null
---

## Summary

Add an action envelope and receipt framework, then implement the first safe run and issue actions through the gateway.

## Scope

### In scope

- Define action request and action receipt envelopes.
- Generate correlation IDs and publish accepted/rejected action events.
- Add hosted-permission placeholders to the action layer.
- Implement retry, cancel where supported, rehydrate, issue comment, and debug-session metadata actions.
- Ensure action responses identify expected follow-up events.

### Out of scope

- Full hosted RBAC.
- Linear milestone/issue/sub-issue creation.
- Approval center UI.

## Deliverables

- Action envelope types.
- Action audit records in the event journal.
- Initial `/api/v1/actions/*` endpoints.
- Tests for idempotency behavior where practical.

## Acceptance Criteria

- [ ] Each action returns accepted or rejected status, reason, action ID, and correlation ID.
- [ ] Accepted and rejected actions appear in the event stream.
- [ ] Actions call orchestrator-owned commands or messages instead of private state mutation.

## Test Plan

- Run action handler unit tests and fake-orchestrator integration tests.
- Verify emitted event correlation IDs match action receipts.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.1.4 and `docs/host-client-architecture.md` 7.3.
- Respect AGENTS.md invariants for orchestrator-owned scheduling state and UI separation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

These initial actions should be small and auditable so later Linear, planning, and hosted actions can reuse the envelope.
