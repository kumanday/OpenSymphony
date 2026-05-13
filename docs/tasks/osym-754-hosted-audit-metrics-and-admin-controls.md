---
id: OSYM-754
title: Hosted Audit, Metrics, And Admin Controls
milestone: "M6: Hosted Alpha"
priority: 2
estimate: 8
blockedBy: ["OSYM-750", "OSYM-753"]
blocks: ["OSYM-771", "OSYM-772"]
parent: null
---

## Summary

Add hosted audit logging, operational metrics, and alpha admin controls for users, projects, runs, credentials, capacity, and quotas.

## Scope

### In scope

- Audit login/logout, project access changes, secret changes, action mutations, approval decisions, and admin actions.
- Track resource usage, stream metrics, run metrics, retry counters, queue depth, and backpressure.
- Add alpha admin views for users, projects, active runs, runtime capacity, quotas, credential rotation/revocation, and run cancellation/kill controls.
- Enforce permission boundaries on admin actions.

### Out of scope

- Billing.
- Enterprise policy engine.
- Full observability vendor integration.

## Deliverables

- Hosted audit log.
- Metrics dashboard foundation.
- Admin UI alpha.
- Admin action tests.

## Acceptance Criteria

- [ ] Sensitive hosted actions produce audit events with actor, target, timestamp, and outcome.
- [ ] Admin controls require explicit permission checks.
- [ ] Metrics expose stream health, queue depth, run latency, retries, failures, and resource usage.

## Test Plan

- Run audit event tests for hosted actions and sensitive configuration changes.
- Run admin permission tests.
- Run metrics emission tests for stream and run fixtures.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.10, 4.11, and 4.12.
- Audit and metrics data support hosted diagnostics and operator trust.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Admin actions should emit ordinary gateway action receipts when they mutate run or project state.
