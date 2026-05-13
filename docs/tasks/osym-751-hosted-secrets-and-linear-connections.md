---
id: OSYM-751
title: Hosted Secrets And Linear Connections
milestone: "M6: Hosted Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-750"]
blocks: ["OSYM-752", "OSYM-761"]
parent: null
---

## Summary

Implement hosted secret references and tenant-scoped Linear connections for server-side project sync and mutations.

## Scope

### In scope

- Add encrypted secret references for Linear credentials, provider credentials, repository credentials, and harness credentials.
- Scope secrets by user, organization, project, and environment.
- Add rotation and revocation paths.
- Add redaction in logs, events, terminal summaries, issue comments, and diagnostics.
- Support hosted Linear connections by organization or project using API-key mode initially or OAuth if selected.
- Configure tenant-scoped Linear sync workers.

### Out of scope

- Subscription credential adapter implementation.
- Hosted workspace runtime pool.
- Billing or quota UI.

## Deliverables

- Hosted secret store integration.
- Redaction tests.
- Hosted Linear connection service.
- Tenant isolation tests.

## Acceptance Criteria

- [ ] Raw secrets never appear in frontend payloads, logs, events, comments, or diagnostics.
- [ ] Linear connections are scoped to permitted organizations/projects.
- [ ] Secret rotation and revocation update credential status and prevent future use.

## Test Plan

- Run secret storage and redaction tests.
- Run hosted Linear connection tests with tenant-scoped fake credentials.
- Run negative tests for cross-tenant access.

## Context

- Source sections: `docs/host-client-architecture.md` 4.9 and 9.3, `docs/host-client-implementation_plan.md` P8.3/P8.4.
- Server-side credentials must stay out of agent workspaces.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task also prepares hosted storage for model and subscription credentials.
