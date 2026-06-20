---
id: OSYM-766
title: ChatGPT OAuth For Codex Harness
milestone: "M10.3: Codex And Subscription Readiness"
priority: 2
estimate: 5
blockedBy: ["OSYM-761", "OSYM-764"]
blocks: ["OSYM-767"]
parent: null
---

## Summary

Add the local ChatGPT subscription credential path needed for Codex harness support. The implementation should use Codex-supported authentication surfaces such as `codex login --device-auth`, stored Codex credentials, or the current supported equivalent, and expose credential status through OpenSymphony without copying raw access or refresh tokens into workspaces, logs, browser payloads, or Linear content.

## Scope

### In scope

- Detect whether a compatible Codex CLI/app-server installation is available.
- Detect local Codex login status and subscription-backed account readiness.
- Support the Codex-supported login initiation path, including device-auth guidance where available.
- Represent Codex subscription credentials as model/credential setting references rather than raw token material.
- Surface account, status, and failure information where Codex exposes it safely.
- Document logout, revocation, and unsupported-version behavior.
- Add tests or fakes around credential-status detection and failure rendering.

### Out of scope

- Hosted multi-tenant credential storage.
- Scraping unsupported private Codex token formats.
- Cross-harness routing policy.
- Hosted production deployment of subscription credentials.

## Deliverables

- Codex install and login-status detector.
- Subscription credential reference mapping for Codex.
- Login/logout guidance surfaces.
- Failure-state rendering tests.
- Operator documentation for the supported Codex auth path.

## Acceptance Criteria

- [ ] A local operator can identify whether Codex is installed, logged in, and usable for subscription-backed runs.
- [ ] OpenSymphony stores only credential references and status metadata for Codex subscription auth, not raw access or refresh tokens.
- [ ] Unsupported, logged-out, expired, and permission-denied states are explicit in CLI/gateway/client surfaces.
- [ ] The implementation documents the current supported Codex login path and version assumptions.

## Test Plan

- Run fake Codex credential-status tests.
- Run redaction tests to ensure token material is never serialized to OpenSymphony surfaces.
- Run a local smoke test against the documented supported Codex login path when available.

## Context

- This task exists so Codex app-server support can satisfy the business requirement of using ChatGPT subscriptions locally without waiting for full hosted mode.
- It should build on OSYM-761 and OSYM-764 rather than creating a separate credential subsystem.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not scrape or persist private Codex token formats. Use supported Codex authentication behavior and preserve raw credential secrecy.
