---
id: OSYM-845
title: Multi-Harness IDE Attachment Qualification
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 2
estimate: 8
blockedBy:
- OSYM-841
- OSYM-843
- OSYM-906
- OSYM-907
blocks:
- OSYM-844
areas:
- debugging
- acp
- testing
parent: null
---

## Summary

Qualify scheduler-to-IDE attachment across multiple ACP harnesses before the default debug command switches to the IDE flow.

## Scope

### In scope

- Exercise real opensymphony run -> retained runtime owner -> separate ACP bridge -> IDE prompt -> operator callback -> cancellation/release -> scheduler continuation.
- Use an adversarial fake IDE/harness pair plus at least two independently implemented live ACP harnesses with pinned versions, and a real Zed smoke test for both profiles.
- Cover vendor extension requests, metadata fidelity, unsupported IDE fallback, optional persistence differences, archived native debug and exact workspace binding.
- Inject scheduler/IDE races, pending approvals, disconnects, stale lease/replies, host loss, output floods and cleanup attempts.
- Publish compatibility/failure guidance and reproducible evidence separately for fake protocol tests and actual harness/editor runs.

### Out of scope

- Requiring Zed installation for hermetic CI; broad GUI automation or editor multiplexing.

## Deliverables

- End-to-end attachment/race regression suite and live editor evidence.
- Verified capability matrix and troubleshooting for runtime/IDE boundary failures.

## Acceptance Criteria

- [ ] Two real ACP profiles attach to the same session used by the scheduler and exchange debug prompts through Zed; no duplicate agent or fresh-context substitution occurs.
- [ ] A vendor blocking request and its validated IDE/operator response traverse both protocol legs, or a documented capability fallback is explicitly exercised.
- [ ] Ownership tests prove no concurrent scheduler/IDE prompts, no duplicate/stale replies, and no scheduler release before debug work settles.
- [ ] Close cancels owned work before releasing resources while preserving durable workspace/session data; disconnect uncertainty remains fenced.
- [ ] OpenHands store compatibility and native Codex unarchive/resume/--app regressions pass; the qualification report gates OSYM-844.

## Test Plan

- Run fake IDE/ACP/runtime integration, scheduler race and native debug regression tests.
- Run documented live Zed attachment scenarios for both pinned profiles and record exact versions, capabilities, outcomes and limitations.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md: acceptance and failure matrix.
- OSYM-906 runtime qualification evidence, OSYM-907 handoff tests and OSYM-841 bridge.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

The default UX transition depends on this task. A fake OpenHands-only test cannot satisfy multi-harness IDE qualification.
