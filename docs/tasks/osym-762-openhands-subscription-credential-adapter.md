---
id: OSYM-762
title: OpenHands Subscription Credential Adapter
milestone: "M10.3: Codex And Subscription Readiness"
priority: 2
estimate: 8
blockedBy: ["OSYM-761"]
blocks: ["OSYM-763"]
parent: null
---

## Summary

Add a feature-gated OpenHands subscription credential adapter using documented OpenHands SDK or provider client flows, starting with OpenAI ChatGPT/Codex.

## Scope

### In scope

- Support OpenHands SDK `LLM.subscription_login(vendor="openai", ...)` where the pinned SDK supports it.
- Support browser login and device-code flows where documented and available.
- Store credentials through the selected local or hosted storage provider.
- Construct a subscription-backed `LLM` for OpenHands `Agent` and `Conversation` creation.
- Expose login status, account identity where available, auth mode, and expiration state.
- Add feature gating and integration tests.

### Out of scope

- Undocumented auth implementation.
- Codex app-server credential ownership.
- Additional subscription providers.

## Deliverables

- Feature-gated OpenHands subscription credential adapter.
- Auth integration tests.
- Credential status updates.
- Documentation for supported login flows.

## Acceptance Criteria

- [ ] A configured OpenAI ChatGPT/Codex subscription credential can construct the OpenHands `LLM` path documented for the pinned SDK.
- [ ] Tokens and refresh material are stored through selected credential storage and redacted everywhere else.
- [ ] API-key OpenHands configuration continues to work independently.

## Test Plan

- Run mocked subscription credential tests.
- Run live integration tests only when required credentials and SDK support are available.
- Verify redaction in logs, events, and diagnostic payloads.

## Context

- Source sections: `PRODUCT.md` section 6 and `docs/host-client-implementation_plan.md` P9.3.
- Use documented SDK/client behavior and respect account/workspace policy boundaries.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep this integration behind an explicit feature flag until the flow is proven end to end.
