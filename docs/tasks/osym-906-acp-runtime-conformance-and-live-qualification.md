---
id: OSYM-906
title: ACP Runtime Conformance And Live Qualification
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 8
blockedBy:
- OSYM-902
- OSYM-903
- OSYM-904
- OSYM-905
blocks:
- OSYM-845
areas:
- acp
- testing
- docs
parent: null
---

## Summary

Qualify the complete ACP execution path with deterministic failure tests and two independent live harnesses, and publish the supported interoperability contract.

## Scope

### In scope

- Run actual opensymphony run scenarios covering configured profile routing, workspace edits, callbacks, operator responses, cancellation, continuation, restart and cleanup.
- Qualify at least two structurally independent ACP harness implementations with pinned versions, including one supported vendor extension. Use Cursor plus an available DeepSeek Harness or Devin CLI candidate after checking current contracts.
- Exercise optional persistence absent, load with replay, resume without replay, model/config variation and credential failures; record explicit limitations rather than promising universal optional features.
- Complete capabilities endpoint/adapter-boundary and Rust/TypeScript parity checks; audit no ACP route falls through to OpenHands startup.
- Update architecture, configuration, operations, workspace/lifecycle, harness compatibility, testing and sources docs with executable examples, exact versions and redacted evidence.

### Out of scope

- IDE attachment qualification is OSYM-845; ACP v2, generic remote transports, new native vendor adapters and EBO ingestion.

## Deliverables

- Reproducible fake-peer conformance suite and two live harness evidence reports.
- Operator setup/troubleshooting and explicit adapter/profile/negotiated capability matrix.

## Acceptance Criteria

- [ ] Two independently implemented live ACP harnesses complete tracked issue runs through the real routing path; fake-peer tests alone cannot satisfy this gate.
- [ ] The live evidence demonstrates workspace modification, a returned operator decision, cancellation and documented restoration/reset behavior, plus a vendor extension round trip.
- [ ] Failure tests prove no ambiguous prompt resend, cross-session reply, duplicate replay accounting, unsafe cleanup or false cancellation acknowledgement.
- [ ] Examples configure a new compliant profile without adding a vendor-specific scheduler or debug branch.
- [ ] Documentation states version/transport/authentication boundaries and identifies any unavailable live prerequisite as an unmet acceptance criterion.

## Test Plan

- Run relevant system-DuckDB tests, cargo fmt --check, clippy and touched frontend/schema checks; run bundled-mode checks required for the new SDK dependency.
- Run pinned live profile scenarios in isolated issue workspaces with scoped credentials; record commands, versions, outcomes and source-frame references.

## Context

- All M12.99 implementation tasks; docs/specs/acp-harness-adapter.md; docs/testing-and-operations.md; docs/sources.md.
- Reuse fake ACP peer from OSYM-900 and actual run integration from OSYM-902.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

Publish availability truthfully. A configured profile or successful SDK seam test is not live harness qualification.
