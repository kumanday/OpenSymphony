---
id: OSYM-765
title: Codex Approvals And Cross-Harness Routing
milestone: "M10.3: Codex And Subscription Readiness"
priority: 3
estimate: 8
blockedBy: ["OSYM-763", "OSYM-764", "OSYM-767"]
blocks: ["OSYM-770", "OSYM-771"]
parent: null
---

## Summary

Map Codex approval events into the OpenSymphony approval center and add an alpha routing policy model across harnesses and model profiles.

## Scope

### In scope

- Map Codex approval requests to OpenSymphony approval requests.
- Send approval decisions back to Codex.
- Audit approval decisions.
- Define routing rules by task type, model profile, harness capability, cost, speed, and user policy.
- Add explicit user override.
- Add route decision audit events.
- Add dry-run route preview.

### Out of scope

- Hosted Codex production pool.
- Automatic cost optimization.
- Additional tracker adapters.

## Deliverables

- Codex approval bridge.
- Approval contract tests.
- Routing policy engine alpha.
- Route decision tests.
- Dry-run route preview.

## Acceptance Criteria

- [ ] Codex approval requests appear in the same approval center contract as OpenHands-supported approvals.
- [ ] Approval decisions are delivered back to Codex and audited.
- [ ] Routing dry-runs explain selected harness, model profile, and policy reason.

## Test Plan

- Run Codex approval bridge tests with fake JSON-RPC notifications.
- Run route decision tests for task type, capability, user override, and missing-capability cases.

## Context

- Source sections: `docs/host-client-implementation_plan.md` P9.7/P9.8.
- Routing should use configured base URL, model string, credential reference, and harness capabilities.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Treat this as alpha behavior with explicit user visibility.
