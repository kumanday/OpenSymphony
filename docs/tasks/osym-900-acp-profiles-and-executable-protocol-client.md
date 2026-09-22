---
id: OSYM-900
title: ACP Profiles And Executable Protocol Client
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 8
blockedBy: []
blocks:
- OSYM-901
- OSYM-903
areas:
- acp
- configuration
- runtime
parent: null
---

## Summary

Implement a working ACP v1 stdio client and named launch profiles using the official Rust SDK. The deliverable runs a real child process through a complete prompt and cancellation lifecycle.

## Scope

### In scope

- Add the internal opensymphony-acp source module through src/lib.rs; pin the validated agent-client-protocol SDK/schema without creating another Cargo package or copying the Codex RPC client.
- Add typed central/workflow ACP profile configuration and routing.harness_profile. Validate argv, environment references, supported versions/transports, authentication method, required capabilities, and enabled extension names.
- Launch with verified issue cwd, credential exclusions and bounded stderr/stdout; register bidirectional handlers before initialize, negotiate v1, authenticate, create a session, submit prompts, stream ordered updates, and cancel.
- Keep the dispatch loop responsive during callbacks. Preserve opaque IDs, unknown payloads and stop reasons in bounded redacted evidence. Implement explicit errors, submission uncertainty, deadlines and supervised teardown.

### Out of scope

- ACP v2 or network transport implementation; production execution routing is OSYM-902; complete callbacks and operator policy are OSYM-903 and OSYM-904.

## Deliverables

- Executable SDK-backed client and validated profile configuration.
- Adversarial fake ACP subprocess reusable by later runtime and IDE tests.

## Acceptance Criteria

- [ ] A fake child completes initialize, session/new, session/prompt, interleaved updates and terminal response using the same launch path intended for runtime use.
- [ ] A cancelled prompt completes only after its terminal response or an explicit timeout/failure path; sending session/cancel never counts as acknowledgement.
- [ ] Unknown requests receive method-not-found, notifications receive no reply, IDs including 0 correlate correctly, and unsupported versions fail clearly.
- [ ] Invalid profile/env/cwd combinations fail before launch; secrets remain absent from config rendering, arguments and captured diagnostics.
- [ ] Only implemented client capabilities are advertised; SDK integration handles callback concurrency and ordered completion under Tokio without blocking the executor.

## Test Plan

- Run focused config/parser and ACP subprocess tests for malformed/oversized frames, EOF/crash, unknown stop reasons, output floods and an unresponsive child.
- Run cargo fmt --check and the relevant cargo test-system-duckdb and cargo clippy-system-duckdb targets.

## Context

- docs/specs/acp-harness-adapter.md: repository fit, configuration, protocol and cancellation.
- src/lib.rs; Cargo.toml; crates/opensymphony-workflow/src/; central config resolver; crates/opensymphony-codex/src/ for existing supervision patterns.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

This is an implementation task, not an SDK spike. ACP wire v1 and the SDK package version are separate identifiers.
