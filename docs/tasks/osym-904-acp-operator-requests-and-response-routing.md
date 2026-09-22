---
id: OSYM-904
title: ACP Operator Requests And Response Routing
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 13
blockedBy:
- OSYM-902
- OSYM-903
blocks:
- OSYM-905
- OSYM-906
- OSYM-907
areas:
- acp
- orchestrator
- gateway
- ui
parent: null
---

## Summary

Complete the round trip from a harness permission or information request to a real operator response and back to the blocked RPC.

## Scope

### In scope

- Add worker request/response channels and orchestrator-owned pending interactions bound to run, session, connection generation and request ID; preserve option IDs/kinds and validate structured questions.
- Populate run approvals/input requests in snapshots and gateway APIs. Wire existing ApprovalDecision/ActionReceipt plus typed input responses through authorization, command dispatch and the session owner.
- Expose actionable web/desktop and terminal operator controls through existing clients; support approve/deny, choice/form submit, decline and cancel with response deadlines and receipts.
- Implement explicit operator, deny and authorized allow_once policies; unattended operator mode without a response path fails visibly. Elicitation capabilities remain disabled until their actual flow is implemented.
- Handle timeout, disconnect, turn cancellation, duplicate/stale replies and ownership changes without hanging a harness. Make waiting-for-input visible to stall detection and concurrency accounting.

### Out of scope

- Arbitrary RPC passthrough and vendor method registration, which belongs to OSYM-905.

## Deliverables

- Working pending-interaction projection, operator commands and worker response delivery.
- Reviewable operator controls and interaction diagnostics across configured clients.

## Acceptance Criteria

- [ ] A real opensymphony run receives an offered permission, exposes it via the gateway/UI, accepts a valid operator decision, and emits exactly one matching ACP response with the original option and RPC IDs.
- [ ] The same production route completes a structured question; permissions, questions and plan approval have distinct typed semantics.
- [ ] Wrong run/session/generation, invalid option, expired and duplicate responses are rejected; no old approval is replayed after restart.
- [ ] Cancellation answers pending requests using the appropriate cancellation result, and timeout cannot leave the worker or RPC permanently blocked.
- [ ] Other issues continue running while an issue awaits input. Secrets are not elicited through forms or leaked into public snapshots; URL elicitation requires a consented out-of-band flow if enabled.

## Test Plan

- Exercise worker -> orchestrator -> gateway/client -> command -> worker -> ACP in one integration test, including terminal and web/desktop response clients.
- Test deny/allow_once/operator policies, zero request IDs, stale responses, duplicate submissions, deadlines, disconnect and cancellation races.

## Context

- crates/opensymphony-gateway/src/lib.rs: get_run_approvals currently returns no runtime requests.
- ApprovalRequest, ApprovalDecision and ActionReceipt contracts; scheduler worker messages; control-plane actions; packages/api-client and shared operator UI.
- docs/specs/acp-harness-adapter.md: client callbacks and operator responses.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

A DTO, empty endpoint, optimistic receipt, or UI-only approval is not completion. Evidence must show the harness receives the response.
