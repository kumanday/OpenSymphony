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

## Scope

### In scope

- Extend domain, gateway, and TypeScript schemas with routing mode, active
  project set, Linear project, binding outcome, canonical repository display,
  config/inventory/checkouts, instruction hash, target commits, and provenance.
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

- New scheduling or provider behavior.
- Hosted tenant RBAC or sandbox implementation.
- A generic permissions UI.

## Deliverables

- Rust and TypeScript schema extensions and round-trip fixtures.
- Gateway snapshots/events and sanitized diagnostic bundles.
- CLI, TUI, web, and desktop multi-repository projections.
- Cross-client parity, redaction, and blocked-state rendering tests.
- Updated operator and architecture documentation.

## Acceptance Criteria

- [ ] Every client agrees on routing, repository, parent, lease, repair, memory,
      containment, provider, verification, and cleanup states.
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

- Add Rust/TypeScript schema round trips for every new enum and optional field.
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
- Public repository identity uses canonical IDs and safe display aliases, never
  credential-bearing clone URLs.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task exposes implemented facts. It must not invent containment or permission
guarantees to make the UI appear complete.
