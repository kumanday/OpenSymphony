---
id: OSYM-763
title: Model Configuration UI And Routing Metadata
milestone: "M10.3: Codex And Subscription Readiness"
priority: 3
estimate: 5
blockedBy: ["OSYM-762"]
blocks: ["OSYM-765"]
parent: null
---

## Summary

Add model configuration profiles, UI, and routing metadata for provider-aware and harness-aware execution choices.

## Scope

### In scope

- Add model configuration profiles based on base URL, model string, credential reference, and harness capability.
- Keep the profile contract limited to values the current UI and model-settings surface can represent directly.
- Add model configuration UI.
- Preserve arbitrary configured model strings for API-compatible OpenHands usage.
- Leave context window, cost profile, reasoning controls, and task recommendations for later routing work that can derive or consume them correctly.

### Out of scope

- Full routing policy engine.
- Hosted billing/cost enforcement.
- Model benchmark automation.

## Deliverables

- Model configuration service.
- Model configuration UI.
- Harness compatibility metadata in the profile schema.
- UI and service tests.

## Acceptance Criteria

- [ ] Users can view and edit API-compatible and subscription-backed model profiles.
- [ ] Profiles declare usable harnesses.
- [ ] Arbitrary provider model strings are preserved without adding unwired routing placeholders.

## Test Plan

- Run model profile CRUD tests.
- Run UI tests for API-key and subscription-backed profile forms.
- Verify redacted credential display.

## Context

- Source sections: `docs/host-client-architecture.md` 8.2 and 8.3.
- Model selection should be provider-aware and capability-aware.
- Context window, cost, reasoning, and task recommendation values should come from
  capability discovery, benchmark/policy data, or runtime adapters when those
  consumers exist; they should not be hidden user profile fields.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Profiles should support dynamic routing later without forcing it into the first UI.
