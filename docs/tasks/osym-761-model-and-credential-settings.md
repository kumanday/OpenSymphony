---
id: OSYM-761
title: Model And Credential Settings
milestone: "M10.3: Codex And Subscription Readiness"
priority: 2
estimate: 8
blockedBy: ["OSYM-760"]
blocks: ["OSYM-762", "OSYM-764", "OSYM-766"]
parent: null
---

## Summary

Implement local and harness-orthogonal model and credential settings that preserve API-compatible OpenHands configuration while adding subscription credential references for Codex and future harnesses. Hosted secret-store implementation is tracked separately in OSYM-755.

## Scope

### In scope

- Preserve `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY` settings for API-compatible OpenHands use.
- Add API-key profiles and subscription-backed credential references with OpenAI ChatGPT/Codex as the first subscription provider type.
- Represent owner scope, credential storage mode, base URL, model string, credential reference, provider, harness compatibility, and credential status.
- Support local keychain references and isolated OpenHands auth-directory references without copying raw secrets into workspaces, browser payloads, logs, or Linear content.
- Add credential status endpoint and UI hooks that can report installed, logged-out, expired, unsupported, and permission-denied states.
- Model hosted credential-broker references as a future-compatible reference type without implementing hosted broker storage here.

### Out of scope

- Hosted multi-tenant secret-store implementation, refresh-token broker, or hosted credential injection. That production hosted gap is tracked by OSYM-755.
- OpenHands subscription login execution.
- Codex app-server execution.
- Dynamic routing policy.

## Deliverables

- Model and credential settings model.
- Credential reference and status DTOs.
- Credential status endpoint.
- Credential status UI hooks.
- Redaction and secret-leakage tests.
- Harness compatibility fit notes for OpenHands and Codex.

## Acceptance Criteria

- [ ] API-key mode maps to existing OpenHands `LLM_BASE_URL`, `LLM_MODEL`, and `LLM_API_KEY` behavior.
- [ ] Subscription mode stores credential references and status metadata separately from API keys and raw token material.
- [ ] Model settings identify which harnesses can consume each configuration.
- [ ] Local Codex/ChatGPT subscription readiness can build on this settings seam without depending on hosted secrets.
- [ ] Hosted credential broker references are represented in a way that OSYM-755 can implement later without changing the public settings shape.

## Test Plan

- Run settings serialization and redaction tests.
- Run local credential reference and status-rendering tests.
- Verify OpenHands API-compatible settings still flow into existing harness configuration.
- Verify Codex credential references can be represented without raw token persistence in OpenSymphony.

## Context

- Source sections: `PRODUCT.md` sections 5 and 6, `docs/hosted-client-PRD.md` 4.9, and `docs/host-client-architecture.md` 8.
- Subscription credentials are a credential capability that can serve OpenHands now and Codex next, while future hosted production storage is handled separately.
- This retargeting removes hosted-only secret storage from the local Codex critical path.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep raw refresh tokens out of workspaces, frontend payloads, logs, and Linear content. Treat hosted storage as a reference shape only until OSYM-755 implements the hosted broker.
