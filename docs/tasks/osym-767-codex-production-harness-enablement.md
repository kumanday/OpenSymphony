---
id: OSYM-767
title: Codex Production Harness Enablement
milestone: "M10.3: Codex And Subscription Readiness"
priority: 2
estimate: 8
blockedBy: ["OSYM-764", "OSYM-766"]
blocks: ["OSYM-765"]
parent: null
---

## Summary

Graduate the Codex app-server prototype into a supported local OpenSymphony harness path. This task turns the benchmark/prototype work into a production-ready local adapter with version checks, generated schemas, lifecycle handling, event normalization, approval/action support, cancellation/resume behavior, and operator-facing failure modes.

## Scope

### In scope

- Pin or detect compatible Codex CLI/app-server versions.
- Generate or validate the app-server TypeScript/JSON schema contracts used by OpenSymphony.
- Implement the preferred local transport, expected to be stdio unless benchmark evidence selects another supported transport.
- Normalize Codex thread, turn, message, tool/action, approval, and error events into OpenSymphony run detail surfaces.
- Support start/resume/cancel and approval/action flows through the shared harness adapter contract.
- Preserve raw event payloads needed for forward compatibility and diagnostics.
- Add contract tests, fake-server tests, and local smoke documentation.

### Out of scope

- Hosted Codex worker pools.
- Hosted subscription credential broker implementation.
- Cross-harness routing policy beyond the metadata needed by OSYM-765.

## Deliverables

- Production-grade local Codex harness adapter.
- Version and compatibility checks.
- Event normalization and raw-payload retention.
- Approval, cancel, resume, and error surfaces.
- Contract and fake-server tests.
- Local smoke-test documentation.

## Acceptance Criteria

- [ ] OpenSymphony can run a local Codex app-server-backed issue through the shared harness adapter contract.
- [ ] Codex events, approvals, cancellations, and failures are visible through the same run detail/control-plane surfaces as other harnesses.
- [ ] Version incompatibility and missing-login states fail with actionable operator guidance.
- [ ] Contract tests cover the app-server lifecycle and event normalization path.

## Test Plan

- Run Codex adapter contract tests.
- Run fake app-server lifecycle and event-normalization tests.
- Run local smoke tests with a compatible Codex installation when available.

## Context

- This task intentionally follows the Codex prototype/benchmark work and ChatGPT OAuth readiness.
- It should not wait for full hosted mode, but it must respect the shared harness and model/credential settings architecture so hosted mode can reuse the same concepts later.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep the production adapter behind explicit compatibility checks so unsupported Codex versions fail clearly instead of partially running.
