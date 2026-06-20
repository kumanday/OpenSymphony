---
id: OSYM-755
title: Hosted Subscription Credential Broker And Secret Store
milestone: "M11: Hosted Alpha"
priority: 2
estimate: 8
blockedBy: ["OSYM-750", "OSYM-751"]
blocks: []
parent: null
---

## Summary

Add the hosted credential broker and secret-store foundation for subscription-backed harnesses, including ChatGPT OAuth credentials for Codex and OpenHands subscription-backed usage. This task keeps hosted secret storage out of the local Codex critical path while explicitly tracking the production hosted requirement.

## Scope

### In scope

- Store subscription OAuth refresh credentials in encrypted per-user/per-organization hosted secret storage.
- Refresh short-lived credentials safely without exposing raw token material to workspaces, browser clients, Linear content, or logs.
- Support revocation, rotation, and explicit credential health/status surfaces.
- Inject only scoped, short-lived credential material into authorized hosted harness processes.
- Add audit logs for credential use, refresh, revocation, and denied access.
- Preserve tenant isolation and RBAC boundaries for credential reads and writes.
- Document local-vs-hosted subscription credential behavior.

### Out of scope

- Local Codex subscription login support.
- Implementing the Codex harness adapter itself.
- Billing or commercial subscription plan management.

## Deliverables

- Hosted subscription credential broker.
- Encrypted hosted secret-store integration.
- Refresh, revocation, and credential health surfaces.
- Tenant-isolation and RBAC tests for credential access.
- Operator documentation for hosted subscription credential behavior.

## Acceptance Criteria

- [ ] Hosted subscription credentials are encrypted at rest and scoped to the owning user/organization/project policy.
- [ ] Hosted harnesses receive only authorized, short-lived credential material.
- [ ] Revocation and refresh failures are visible to operators without leaking secrets.
- [ ] Audit logs record credential lifecycle and usage decisions.
- [ ] Tests cover cross-tenant denial and authorized same-tenant access.

## Test Plan

- Run hosted credential broker unit tests.
- Run cross-tenant denial tests for credential reads, refreshes, and injections.
- Run audit-log coverage for refresh, revocation, success, and denial paths.

## Context

- This is the hosted production gap intentionally separated from local Codex and ChatGPT subscription readiness.
- Local subscription support proceeds through M10.3 without waiting for this hosted broker.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Hosted subscription credentials must never be exposed in workspace files, browser payloads, Linear comments, or raw logs.
