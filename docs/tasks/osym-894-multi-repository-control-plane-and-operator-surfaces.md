---
id: OSYM-894
title: Multi-Repository Control Plane And Operator Surfaces
milestone: "M12.97: Multi-Repository Operations And Rollout"
priority: 3
estimate: 8
blockedBy: ["OSYM-893"]
blocks: ["OSYM-895"]
areas:
  - gateway
  - tui
  - desktop
  - web
  - documentation
parent: null
---

## Summary

Expose truthful, sanitized multi-repository routing, parent, lease, repair,
memory, containment, and cleanup state consistently across the control plane,
CLI, TUI, web, and desktop clients.

## COE-547 Code Baseline

Extend this projection chain:

- `crates/opensymphony-domain/src/snapshot.rs` owns
  `WorkerAttemptSnapshot`, `RetrySnapshot`, `RuntimeStateSnapshot`, and
  `IssueSnapshot`, including retry-count overrides, interrupts, and explicit
  release reasons.
- `crates/opensymphony-domain/src/control_plane.rs` owns
  `ControlPlaneIssueSnapshot`, including retry, release, cancellation, harness,
  path-suffix, and token projections.
- `crates/opensymphony-cli/src/orchestrator_run/snapshot.rs::map_issue` maps
  authoritative scheduler state into the control plane and enforces
  tracker-terminal precedence.
- `crates/opensymphony-gateway/src/lib.rs` maps that snapshot into gateway run
  detail and now receives configured terminal states through
  `GatewayServer::with_terminal_states`.
- `crates/opensymphony-workspace/src/models.rs::redact_runtime_diagnostic` is
  the shared credential and bounded-diagnostic redaction boundary.
- Existing FrankenTUI projections consume the Rust control-plane model. COE-547
  did not add matching TypeScript/web/desktop multi-repository schemas, so
  those remain explicit deliverables here.

Preserve the regressions
`retry_exhausted_release_preserves_explicit_reason`,
`terminal_tracker_state_overrides_a_failed_worker_outcome`,
`gateway_run_detail_exposes_retry_exhausted_lifecycle`,
`gateway_run_detail_terminal_tracker_state_overrides_stale_exhaustion_reason`,
and the `runtime_diagnostics_redact_*` tests. This baseline covers generic
single-issue runtime truth, not repository binding, checkout provenance,
parents, leases, repairs, memory sources, cleanup progress, support bundles, or
cross-client parity.

## Scope

### In scope

- Extend the existing Rust control-plane fields plus gateway and FrankenTUI
  projections with active project set, Linear project, binding outcome,
  canonical repository display, inventory/checkouts, instruction hash, target
  commits, and provenance; add matching TypeScript schemas.
- Add parent state, pinned hierarchy generation, descendant repository summary,
  checkout handles, effective harness containment, verification attempts,
  leases, repairs, provider status, memory scope/source freshness, and cleanup.
- Keep blocked, quarantined, degraded, failed, canceled, and cleanup states
  distinct from completed.
- Update CLI, FrankenTUI, web, and desktop views so operators can explain why a
  task is blocked, what repository/commit it uses, what a parent integrates,
  which leases prevent cleanup, what repair is pending, and what final evidence
  remains.
- Expose exact local paths only on trusted local diagnostic surfaces.
- Add structured events and support bundles with stable semantic IDs, config and
  instruction hashes, safe remote fingerprints, and bounded evidence refs.
- Redact credentials, credential-bearing remotes, secret env values, private
  instruction bodies, memory tokens, and unrestricted paths.
- Keep Tauri IPC capabilities, gateway operator authorization, harness
  approvals, and worker execution containment as separate concepts.
- Update architecture, configuration, workspace lifecycle, memory, harness,
  operations, and testing documentation with implemented behavior.

### Out of scope

- Redesigning COE-547 retry, terminal-release, tracker-completion, config
  generation, or generic diagnostic-redaction semantics.
- New scheduling or provider behavior.
- Hosted tenant RBAC or sandbox implementation.
- A generic permissions UI.

## Deliverables

- Rust and TypeScript schema extensions layered on the existing COE-547 runtime
  projections, with round-trip fixtures.
- Gateway snapshots/events and sanitized diagnostic bundles.
- CLI, TUI, web, and desktop multi-repository projections.
- Cross-client parity, redaction, and blocked-state rendering tests.
- Updated operator and architecture documentation.

## Acceptance Criteria

- [ ] Every client agrees on routing, repository, parent, lease, repair, memory,
      containment, provider, verification, and cleanup states.
- [ ] Existing COE-547 retry, exhaustion, interrupt, release-reason, completion,
      config-generation, and diagnostic-redaction fixtures remain stable across
      the new cross-client projections.
- [ ] Blocked, degraded, quarantined, failed, canceled, and cleaning work never
      renders as completed.
- [ ] An operator can identify the canonical repository, target commit,
      instruction hash, lease owner, pending provider step, and cleanup blocker.
- [ ] Parent views list verified descendant repositories without exposing raw
      local paths remotely.
- [ ] Trusted-host execution is labeled honestly and never presented as
      workspace confinement.
- [ ] Secret-canary tests find no credential, token, private instruction body, or
      credential-bearing remote in schemas, events, logs, or support bundles.
- [ ] Tauri/gateway/harness “permission” surfaces do not claim to enforce worker
      filesystem access.

## Test Plan

- Inventory the COE-547 Rust DTO and FrankenTUI fixtures first, then add
  Rust/TypeScript round trips only for the remaining multi-repository fields.
- Add gateway snapshot and event tests for leaf, waiting parent, integration,
  repair, blocked, cleaning, and completed states.
- Add TUI/web/desktop rendering and reducer parity tests.
- Add local-versus-remote path projection and secret-canary tests.
- Run focused gateway/control/TUI tests, `npm test`, TypeScript checks,
  `cargo clippy-system-duckdb`, and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 15, 18, 19, and
  21.6.
- Inspect `crates/opensymphony-domain/src/control_plane.rs`,
  `crates/opensymphony-gateway-schema`, `crates/opensymphony-gateway`,
  `crates/opensymphony-tui`, `packages/gateway-schema`, `packages/ui-core`,
  `apps/web`, and `apps/desktop`.
- Follow the named Rust snapshot-to-gateway projection chain, then compare it
  with the TypeScript and client surfaces listed above.
- After COE-547 closeout is indexed, `memory.context` scoped to issue `COE-547`
  and areas `gateway`, `tui`, and `workspace-lifecycle` may supply provenance
  and rationale; verify it against the named source and fixtures.
- Treat the generic runtime projections as compatibility inputs, not as a
  reason to broaden this task into scheduling or recovery changes.
- Public repository identity uses canonical IDs and safe display aliases, never
  credential-bearing clone URLs.

## Definition of Ready

- [x] The COE-547 projection baseline and remaining multi-repository operator
      surface are explicit.
- [x] Required Rust, TypeScript, client, and documentation paths are explicitly
      referenced.
- [ ] OSYM-893 is merged and its final cleanup and lease states are available
      for projection.

## Notes

This task exposes implemented facts. It must not invent containment or permission
guarantees to make the UI appear complete.
Do not reimplement generic runtime recovery to make a view convenient; adapt
the clients to the authoritative domain state delivered by earlier slices.
