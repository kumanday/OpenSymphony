---
id: OSYM-716
title: Desktop Settings, Keychain, And Native Actions
milestone: "M7: Shared Client And Desktop Alpha"
priority: 3
estimate: 5
blockedBy: ["OSYM-714", "OSYM-715"]
blocks: ["OSYM-761"]
parent: null
---

## Summary

Add desktop-native settings, keychain hooks, redaction helpers, notifications, and convenience actions.

## Scope

### In scope

- Store non-secret desktop settings locally.
- Store sensitive local credentials in OS keychain or the selected local credential storage.
- Add redaction helpers and credential status display.
- Implement native actions for opening repository folders, revealing workspaces where allowed, copying paths or issue links, opening Linear links, and sending notifications.

### Out of scope

- Hosted secret store.
- Subscription login implementation.
- Admin settings UI.

## Deliverables

- Desktop settings service.
- Keychain integration.
- Native action menu.
- Notification integration and tests.

## Acceptance Criteria

- [ ] Secret values are stored outside ordinary settings files.
- [ ] Redacted credential status can be shown without exposing raw secrets.
- [ ] Native actions respect workspace safety and desktop capability scopes.

## Test Plan

- Run settings and redaction tests.
- Run platform-supported keychain integration tests or documented manual verification.
- Verify notification behavior in development mode.

## Context

- Source sections: `docs/hosted-client-PRD.md` 4.2.1 and 4.2.4.
- This task prepares local credential storage for later model and subscription settings.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Credential status should identify configured account/profile state without surfacing token material.
