---
id: OSYM-901
title: ACP Session Ownership And Durable Recovery
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 8
blockedBy:
- OSYM-900
blocks:
- OSYM-902
areas:
- acp
- runtime
- workspace
parent: null
---

## Summary

Own each ACP session in the OpenSymphony runtime host across worker attempts and IDE attachments, with durable identity and truthful recovery after process or host failure.

## Scope

### In scope

- Implement one supervised per-issue process/connection owner with command and ordered event channels; a worker borrows the session instead of defining its lifetime.
- Retain idle sessions according to explicit bounded host retention policy, including sessions without persistence. Existing attachment/control leases pin the session; resource limits refuse new launches or visibly end eligible idle sessions.
- Persist additive harness kind/profile/fingerprint, opaque session, workspace/repository/run/generation, negotiated capabilities and submission/outcome markers in existing manifests/runtime envelopes.
- Implement optional load/resume with replay separation, profile/grant compatibility checks, reset policy, uncertain-submission fencing, owner loss and cleanup. Publish attach-ready identity/state for M13 without implementing IDE protocol handling.
- Expose host commands/events through the existing control-plane boundary so a separate debug process can reach the owner. Define single-owner generation fencing and refuse a second process for a live session.

### Out of scope

- IDE writer transfer and scheduler holds are OSYM-907. Cross-issue process pools and generic plugin infrastructure.

## Deliverables

- Host-owned session actor, durable session identity and control-plane command/event seam.
- Recovery and retention policy with diagnostics distinguishing live attach, restored context, transcript inspection and fresh reset.

## Acceptance Criteria

- [ ] A finished worker attempt leaves an eligible live session available to the next attempt and IDE attachment; releasing one subscriber does not kill shared execution.
- [ ] Nonpersistent sessions attach while alive; after owner death the system never describes a fresh session or recorded transcript as restored agent context.
- [ ] Possible prompt delivery without a terminal result is fenced as uncertain and never automatically replayed, including across restart.
- [ ] Profile, repository, workspace, generation and credential-scope mismatches prevent unsafe reuse; legacy native manifests remain readable.
- [ ] Cleanup waits for execution quiescence and releases owned terminal/process resources; EOF alone is not proof that a harness delegating remote work stopped.

## Test Plan

- Test concurrent sessions and repeated worker attempts, retention expiry with/without attachments, host crash before/after prompt submission, load replay, resume without replay and unsupported persistence.
- Test cross-process duplicate owner rejection and stale generation commands; run workspace/session persistence regressions.

## Context

- docs/specs/acp-harness-adapter.md: protocol/session lifecycle and persistence; docs/specs/opensymphony-acp-debugging-spec.md: shared owner and binding.
- crates/opensymphony-workspace/src/models.rs; crates/opensymphony-openhands/src/session.rs; crates/opensymphony-control/src/; existing runtime envelopes and cleanup fences.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

Use existing persistence and control surfaces. No .opensymphony/debug.json, sidecar session database, or independent scheduling owner.
