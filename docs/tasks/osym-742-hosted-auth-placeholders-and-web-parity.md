---
id: OSYM-742
title: Hosted Auth Placeholders And Web Parity
milestone: "M10: Web Client And External Gateway"
priority: 2
estimate: 5
blockedBy: ["OSYM-712", "OSYM-735", "OSYM-740", "OSYM-741"]
blocks: ["OSYM-750", "OSYM-771"]
parent: null
---

## Summary

Add auth-aware web shell states and verify remote web parity for core dashboard, task graph, run, stream, and planning views.

## Scope

### In scope

- Add login state UI.
- Add unauthorized and forbidden states.
- Add organization and project selection placeholders.
- Keep local unauthenticated mode simple.
- Verify browser views match desktop remote behavior for dashboard, task graph, run detail, terminal/log streams, approvals, and planning drafts.

### Out of scope

- Real hosted auth provider integration.
- Tenant database design.
- Admin console.

## Deliverables

- Auth-aware UI shell.
- Unauthorized and forbidden states.
- Remote parity test fixtures.
- Placeholder tests.

## Acceptance Criteria

- [ ] Web UI renders clear unauthenticated, unauthorized, and forbidden states.
- [ ] Browser remote mode can display the same core project, task graph, run, stream, and planning state as desktop remote mode.
- [ ] Local unauthenticated gateway use remains straightforward for development.

## Test Plan

- Run browser UI tests for auth placeholder states.
- Run fixture parity tests comparing desktop remote and browser state transitions.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.3, 4.10, and release 3.
- Hosted auth is implemented in OSYM-750; this task prepares the user-facing states.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

These placeholders reduce churn when hosted auth arrives.
