---
id: OSYM-841
title: Multi-Harness ACP IDE Bridge
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 2
estimate: 13
blockedBy:
- OSYM-907
- OSYM-903
- OSYM-905
blocks:
- OSYM-842
- OSYM-845
areas:
- debugging
- acp
- cli
parent: null
---

## Summary

Implement opensymphony debug --acp-stdio as an ACP server that connects an IDE to the existing host-owned runtime session for any supported ACP profile.

## Scope

### In scope

- Use the official SDK in server role for initialize, session/new, prompt, cancel and advertised close; keep one attachment per spawned bridge process and stdout protocol-only.
- Negotiate IDE and harness legs independently; initialize uses conservative capabilities before cwd selects a profile, then expose accurate session options and diagnostics.
- Forward ACP-native content/tool updates, config/modes, permitted metadata and registered extensions from the ordered source stream. Normalize native OpenHands events at its boundary; do not reconstruct ACP from lossy scheduler summaries.
- Remap outer session/RPC IDs to bound inner IDs, including bidirectional blocking requests and concurrent updates; route IDE permission/input responses through the same owner validation as operator clients.
- Keep filesystem, terminals, environment and scoped MCP bound to the existing host. Define explicit unsupported/fallback behavior when an IDE lacks a callback or extension already required by the harness.
- Implement writer acquisition, cancellation, completion ordering and close/disconnect through OSYM-907. Optional outer load/resume/list remain unadvertised until implemented.

### Out of scope

- A new native Codex ACP bridge, ACP v2, remote IDE hosting and multiple sessions in one bridge process.

## Deliverables

- Production ACP server/bridge command and source-event forwarding.
- Bidirectional ID/capability mapping and actionable protocol errors.

## Acceptance Criteria

- [ ] The same bridge attaches to two distinct ACP harness profiles and prompts their existing sessions without vendor-specific bridge branches.
- [ ] IDE callbacks return exactly once to the originating inner request; zero/string IDs, metadata and partial tool updates survive correct mapping.
- [ ] An unsupported IDE feature fails or takes the documented host-operator fallback; attaching cannot expand previously negotiated downstream capabilities.
- [ ] Prompt completion follows the bound terminal result; cancel and close remain responsive under output load and stop attachment-owned work before release.
- [ ] Logs/secrets stay off stdout; invalid cwd/binding, unavailable owner, busy control and unsupported restoration produce actionable errors.

## Test Plan

- Drive an outer fake ACP IDE and inner fake ACP harness through the actual host owner with interleaved prompts, callbacks, extensions, cancellation and EOF.
- Assert payload fidelity, version/capability negotiation, ID isolation, bounded buffering and native OpenHands mapping.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md; official ACP v1 initialize, prompt-turn, session-setup and extensibility contracts.
- Shared attachment and handoff from OSYM-840/OSYM-907; ACP source stream, callbacks and extension registry from M12.99.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

The IDE-facing ACP session ID identifies an attachment; it is distinct from the harness session and control lease. session/close must cancel owned ongoing work.
