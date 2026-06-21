---
id: OSYM-780
title: Model Configuration Codex Subscription Follow-Up
milestone: "M10.4: Desktop Live Operations And Model Polish"
priority: 3
estimate: 5
blockedBy: []
blocks: []
areas:
  - model-configuration
  - codex
  - desktop
parent: null
---

## Summary

Clean up model configuration defaults and subscription-auth UX after local Codex harness support landed.

## Scope

### In scope

- Replace the default `OpenAI API-compatible` model from `gpt-4.1` to `gpt-5.5` unless a current compatibility reason requires a different value.
- Replace the default `OpenAI subscription` profile model from `codex` to `gpt-5.5` unless the current Codex app-server contract requires a provider-prefixed variant.
- Audit how desktop model profiles are read, written, persisted, and passed into orchestrator/harness selection for Codex-backed runs.
- Investigate the collapsed `Auth: Not configured` display and propose the smallest operator-facing status that reflects API-key, OpenHands subscription, and Codex CLI login cases.
- Investigate whether showing `OpenHands Auth Directory Env` for the `OpenAI subscription` profile is still correct for Codex-backed subscription usage.
- Check with the operator before changing subscription-auth field semantics if the investigation finds multiple plausible UX/config models.

### Out of scope

- Implementing a hosted credential broker.
- Copying raw ChatGPT OAuth tokens into desktop settings.
- Adding automatic model recommendation or cost routing.

## Deliverables

- Updated model profile defaults and tests.
- Short investigation note in the PR or task comment covering auth-status and subscription-field findings.
- Any approved UI/config changes needed so Codex subscription use does not imply OpenHands-only auth configuration.
- Documentation updates if default models or auth/profile behavior changes.

## Acceptance Criteria

- [ ] Default model profiles use `gpt-5.5` for API-compatible and subscription-backed OpenAI profiles, or the implementation documents a concrete contract reason for a different exact string.
- [ ] Existing saved profiles are preserved unless the user explicitly edits them.
- [ ] Desktop model profile persistence and orchestrator launch wiring are verified for Codex-backed runs.
- [ ] `Auth: Not configured` is either replaced with a meaningful status or documented as intentionally scoped to one credential path.
- [ ] The operator is consulted before changing the meaning of `OpenHands Auth Directory Env` for subscription profiles.

## Test Plan

- Run model configuration schema/default tests.
- Run desktop/web model profile persistence tests.
- Run a focused manual or automated check that selecting the Codex subscription profile produces the expected harness/model routing metadata without exposing raw credentials.

## Context

- `packages/gateway-schema/src/model_config.ts` currently defaults `OpenAI API-compatible` to `gpt-4.1` and `OpenAI subscription` to `codex`.
- The desktop UI currently labels subscription credential input as `OpenHands Auth Directory Env` and collapses credential status to `Auth: ...`.
- Local Codex harness authentication is based on the Codex CLI login/readiness path rather than an OpenHands auth directory.

## Definition of Ready

- [ ] Current model-settings and desktop persistence code paths are identified.
- [ ] The auth-status question is investigated before implementation changes are finalized.
- [ ] A coding agent can start from this task without additional planning context.
