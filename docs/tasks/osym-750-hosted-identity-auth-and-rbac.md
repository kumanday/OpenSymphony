---
id: OSYM-750
title: Hosted Identity, Auth, And RBAC
milestone: "M6: Hosted Alpha"
priority: 1
estimate: 8
blockedBy: ["OSYM-705", "OSYM-741", "OSYM-742"]
blocks: ["OSYM-751", "OSYM-752", "OSYM-753", "OSYM-754"]
parent: null
---

## Summary

Add hosted user identity, organization/tenant membership, sessions, and role-based permission checks for APIs, streams, and actions.

## Scope

### In scope

- Add users, organizations, memberships, roles, project access rules, and tenant-scoped entity IDs.
- Select and implement the hosted auth provider strategy.
- Add login, logout, and session endpoints.
- Add API, WebSocket, and JSON-RPC-over-WebSocket auth middleware when that remote protocol is selected.
- Add trusted local development auth bypass for explicit dev mode.
- Enforce permission checks for gateway actions and stream subscriptions.

### Out of scope

- Secret storage implementation.
- Hosted workspace isolation.
- Billing and commercial plans.

## Deliverables

- Hosted identity schema.
- Auth middleware.
- Web client login flow integration.
- RBAC tests.

## Acceptance Criteria

- [ ] Hosted requests and streams carry authenticated user and tenant context.
- [ ] Permissions protect project, run, planning session, secret, and action access.
- [ ] Local development auth bypass is explicit and unavailable in production configuration.

## Test Plan

- Run auth and RBAC unit tests.
- Run API and WebSocket permission tests for allowed and denied access.
- Verify action receipts include permission rejection reasons.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.10 and 4.11, `docs/host-client-architecture.md` 4.9 and 9.
- Hosted mode changes deployment, auth, tenancy, isolation, and policy enforcement while preserving gateway contracts.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Every externally visible hosted mutation needs a permission decision and audit-ready context.
