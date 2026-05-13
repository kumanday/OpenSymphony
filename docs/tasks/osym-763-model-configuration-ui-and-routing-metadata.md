---
id: OSYM-763
title: Model Configuration UI And Routing Metadata
milestone: "M7: Provider, Harness, And Model Readiness"
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
- Add optional operator-supplied metadata for context window, reasoning effort, cost profile, and recommended task types.
- Add model configuration UI.
- Preserve arbitrary configured model strings for API-compatible OpenHands usage.
- Add route-decision metadata inputs for future policy rules.

### Out of scope

- Full routing policy engine.
- Hosted billing/cost enforcement.
- Model benchmark automation.

## Deliverables

- Model configuration service.
- Model configuration UI.
- Routing metadata schema.
- UI and service tests.

## Acceptance Criteria

- [ ] Users can view and edit API-compatible and subscription-backed model profiles.
- [ ] Profiles declare usable harnesses and optional task recommendations.
- [ ] Reasoning effort and model metadata can be represented without constraining provider-specific model strings.

## Test Plan

- Run model profile CRUD tests.
- Run UI tests for API-key and subscription-backed profile forms.
- Verify redacted credential display.

## Context

- Source sections: `docs/host-client-architecture.md` 8.2 and 8.3.
- Model selection should be provider-aware and capability-aware.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Profiles should support dynamic routing later without forcing it into the first UI.
