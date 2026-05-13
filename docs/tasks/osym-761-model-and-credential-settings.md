---
id: OSYM-761
title: Model And Credential Settings
milestone: "M7: Provider, Harness, And Model Readiness"
priority: 2
estimate: 8
blockedBy: ["OSYM-716", "OSYM-751", "OSYM-760"]
blocks: ["OSYM-762", "OSYM-763", "OSYM-764"]
parent: null
---

## Summary

Implement model and credential settings that preserve API-compatible OpenHands configuration and add subscription credential references.

## Scope

### In scope

- Preserve `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY` settings for API-compatible OpenHands use.
- Add subscription-backed credential settings with OpenAI ChatGPT/Codex as the first provider type.
- Represent owner scope, credential storage, base URL, model string, credential reference, provider, and harness compatibility.
- Add credential status endpoint and UI hooks.
- Support local keychain or isolated OpenHands auth-directory storage.
- Support hosted secret-store or credential-broker references when hosted secrets exist.

### Out of scope

- OpenHands subscription login execution.
- Codex app-server execution.
- Dynamic routing policy.

## Deliverables

- Model and credential settings model.
- Credential status endpoint.
- Credential status UI hooks.
- Redaction tests.

## Acceptance Criteria

- [ ] API-key mode maps to existing OpenHands `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY` behavior.
- [ ] Subscription mode stores credential references separately from API keys.
- [ ] Model settings identify which harnesses can consume each configuration.

## Test Plan

- Run settings serialization and redaction tests.
- Run local and hosted credential reference tests.
- Verify OpenHands API-compatible settings still flow into existing harness configuration.

## Context

- Source sections: `PRODUCT.md` sections 5 and 6, `docs/hosted-client-PRD.md` 4.9, and `docs/host-client-architecture.md` 8.
- Subscription credentials are a credential capability that can serve OpenHands now and future harnesses later.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep raw refresh tokens out of workspaces and frontend payloads.
