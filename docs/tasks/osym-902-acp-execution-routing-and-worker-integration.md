---
id: OSYM-902
title: ACP Execution Routing And Worker Integration
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 8
blockedBy:
- OSYM-901
blocks:
- OSYM-904
- OSYM-906
- OSYM-840
areas:
- acp
- orchestrator
- cli
parent: null
---

## Summary

Route real opensymphony run work to ACP profiles and wire the complete worker lifecycle. Capability discovery alone cannot satisfy this task.

## Scope

### In scope

- Add harness kind acp and profile identity to route resolution and persisted HarnessRouteDecision; propagate central/workflow precedence and model selection without reusing OpenHands credential assumptions.
- Wire WorkerBackend start/recover/poll/abort/interrupt and CLI run startup into the session owner. Replace Codex-versus-other branches in dispatch, session switching and recovery with explicit adapter identity matching.
- Reuse verified task/repository binding, cwd, hooks, instruction provenance, scoped memory grants, review context and runtime envelope; ACP-only dispatch must not start an OpenHands server.
- Normalize live runtime updates/outcomes/usage through worker messages while retaining a separate redacted ACP source stream for IDE forwarding; apply partial tool updates and replay accounting correctly.
- Implement scheduler-visible waiting/failed/cancelled/uncertain outcomes and cleanup gates. Public capability discovery reports adapter, profile and negotiated run support in Rust and TypeScript.

### Out of scope

- New scheduling policy, provider-specific enum variants and IDE presentation. Operator response completion is OSYM-904.

## Deliverables

- Concrete ACP backend routing in production run paths.
- Cross-language capability/config projections and native harness regression coverage.

## Acceptance Criteria

- [ ] An isolated real opensymphony run dispatches a tracked fixture issue to a configured fake ACP executable, receives updates and a terminal outcome, and never launches OpenHands.
- [ ] Two ACP profiles route independently; changing profile cannot reuse incompatible context; recovery honors persisted profile identity instead of the current default.
- [ ] Interrupt, abort, continuation, failure/retry and terminal cleanup exercise the ACP owner and preserve scheduler authority and workspace invariants.
- [ ] Adapter availability, profile preflight readiness and effective run capabilities are distinguishable; absent usage stays absent and replay is not charged twice.
- [ ] OpenHands and Codex routes retain their existing launch, interrupt and recovery behavior.

## Test Plan

- Drive the actual CLI/backend with fake tracker and ACP peers; assert hooks, exact cwd, memory scope, profile switches, recovery and no OpenHands startup.
- Run adapter-boundary, scheduler, gateway capabilities, Rust/TypeScript round-trip and native routing tests.

## Context

- crates/opensymphony-orchestrator/src/scheduler.rs: WorkerBackend, HarnessRouteDecision, WorkerUpdate.
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` and `crates/opensymphony-cli/src/orchestrator_run/mod.rs`; crates/opensymphony-domain/src/harness.rs; crates/opensymphony-gateway-schema/src/capability.rs; packages/gateway-schema/src/capability.ts.
- docs/specs/acp-harness-adapter.md; docs/harness-adapter-compatibility.md; docs/configuration.md; docs/architecture.md.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

Keep protocol types inside opensymphony-acp. HarnessAdapter currently discovers capabilities; actual execution integration is mandatory here.
