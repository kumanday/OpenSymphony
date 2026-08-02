---
id: OSYM-888
title: Scoped Cross-Repository Memory And Leaf Overlays
milestone: "M12.95: Multi-Repository Foundations"
priority: 2
estimate: 13
blockedBy: ["OSYM-886", "OSYM-887"]
blocks: ["OSYM-891"]
areas:
  - memory
  - code-intelligence
  - security
  - mcp
parent: null
---

## Summary

Complete the leaf memory-authorization contract so a terminal worker can
retrieve persisted context from authorized sibling repositories while live
workspace evidence remains bound to its own verified run and checkout
generation.

## Existing Code Baseline

Extend these ownership points rather than creating parallel grant, filter, or
overlay paths:

- `crates/opensymphony-memory/src/lib.rs::{MemoryScopeFilter, MemorySourceRef,
  RegisteredMemorySource, MemoryRepositorySource}`, the `*_with_scope` queries
  in `crates/opensymphony-memory/src/query.rs`, and source registration,
  withdrawal, and `persist_scope_refs` in
  `crates/opensymphony-memory/src/catalog.rs` already provide canonical source
  provenance and project/repository/area-correlated filtering.
- `crates/opensymphony-memory/src/code_graph.rs` already separates
  `indexed_baseline` from `workspace_overlay` evidence and returns repository,
  revision, path/symbol, freshness, and overlay provenance. Preserve that result
  contract while adding authorization.
- `crates/opensymphony-cli/src/memory.rs` owns `MemoryScopeGrantRegistry`,
  `MemoryScopeGrant`, `authorize_memory_request_with_scoped_grant`,
  `validate_worker_memory_scope`, `find_verified_checkout_for_code_intel`, and
  `resolve_code_graph_overlay`. The current leaf scaffold carries one project,
  execution repository, authorized sibling repositories, issue, and checkout
  generation; it fails closed, requires an explicit repository, and limits
  live code tools to the execution repository.
- `crates/opensymphony-cli/src/orchestrator_run/backends.rs` issues worker
  credentials from the resolved project and repository binding.
  `crates/opensymphony-openhands/src/session.rs::MemoryWorkerAccess` injects the
  scoped MCP endpoint and bearer credential and supersedes conversations whose
  process-scoped credential cannot be refreshed. The Codex backend injects the
  equivalent CLI scope environment.
- `crates/opensymphony-workspace/src/models.rs::{CheckoutManifest,
  TerminalRuntimeEnvelope}` already records immutable repository binding,
  config/inventory/policy generations, checkout generation and path, target
  commit, instruction provenance, harness/model, conversation binding, and
  cleanup intent. Derive grants and overlay ownership from that durable
  envelope instead of reconstructing them from mutable tracker state.

Preserve the fail-closed intent and secret-redaction coverage around
`worker_memory_grant_rejects_foreign_and_unscoped_requests`,
`strict_memory_context_requires_a_worker_scope_grant`,
`memory_env_injection_sets_worker_cli_scope`, and
`memory_worker_access_builds_a_scoped_mcp_server_config`, updating the first
test's blanket `allAccessible` rejection to the grant-bounded semantics below.
The process-local registry is a leaf scaffold, not the final claim lifecycle.

## Scope

### In scope

- Extend or replace the existing leaf grant scaffold with claims for run and
  attempt identity, project set, Linear projects, work item, canonical
  repositories, visibility, and administrative capabilities (empty for
  ordinary workers) while preserving its fail-closed behavior.
- Reconstruct claims from the durable terminal envelope after restart and
  expire or revoke process credentials when a run, attempt, checkout
  generation, binding, or conversation is superseded. Do not persist bearer
  secrets.
- Keep authorization claims independent of query filters; every filter narrows
  the grant and `all_accessible` means only all records inside the grant.
- Permit persisted memory and exact-commit target-branch code snapshots for
  repositories associated with the worker's Linear project, including an
  authorized sibling repository.
- Require an explicit canonical repository filter when querying another
  authorized repository.
- Split persisted code-snapshot authorization from live-overlay authorization:
  an authorized sibling may supply persisted code, but a live overlay must
  match the worker's execution repository, issue, run/attempt, and verified
  checkout generation, target commit, and checkout HEAD.
- Record and return overlay commit, dirtiness, run owner, source type,
  freshness, and persisted-versus-live provenance.
- Deny unrelated repositories and every other worker's dirty overlay even when
  the caller requests all accessible records.
- Keep memory retrieval distinct from filesystem grants: knowing another
  repository's memory does not expose its checkout.
- Authorize every memory and code-intelligence tool before applying its scoped
  query filter, including work-item and visibility checks for persisted
  records.
- Bind leaf capture to the immutable execution envelope and record project,
  issue, execution repository, commits, `InstructionProvenance::content_hash`,
  and source refs without using the existing project-association inference as
  ownership authority when several repositories are valid.
- Sync docs only to an explicitly owning repository under its current
  instructions and review policy.
- Surface a visible degraded or blocked memory state when the central service is
  unavailable; never fall back silently to another repository's local store.

### Out of scope

- Parent descendant grants and parent-owned integration overlays (OSYM-891).
- Hosted organization/tenant RBAC.
- Filesystem access to a non-execution repository.

## Deliverables

- Spec-complete leaf claims and credential lifecycle built on the existing
  grant registry and harness injection paths.
- Authorization-before-filter enforcement across memory and code-intelligence
  tools.
- Canonical cross-repository persisted memory and code-snapshot retrieval.
- Leaf overlay ownership and provenance checks.
- Capture/docs-sync ownership and degraded-state behavior.
- Negative-scope and cross-repository integration tests.

## Acceptance Criteria

- [ ] A worker bound to repository A can explicitly query persisted memory and
      target-branch code for associated repository B without receiving or
      resolving repository B's checkout.
- [ ] The same worker cannot query unrelated repository C, even with
      `all_accessible`.
- [ ] Project-set, project, work-item, repository, visibility, and capability
      claims are enforced before area/path filters; filters cannot expand the
      grant, and ordinary worker grants carry no administrative capability.
- [ ] Grant credentials are revoked or replaced across run/attempt,
      binding/generation, and conversation supersession without persisting raw
      tokens.
- [ ] Repository A's live overlay is available only when its issue, run/attempt,
      checkout generation, target commit/HEAD, and execution repository match
      the durable checkout and runtime records.
- [ ] No worker can retrieve repository B's dirty overlay from another issue.
- [ ] Cross-repository code results cite canonical repository ID, exact commit,
      source path or symbol, freshness, and snapshot/overlay origin.
- [ ] Memory service failure is visible and never triggers unsafe direct-store
      fallback.
- [ ] Leaf capture and docs sync preserve the correct repository owner.

## Test Plan

- Add positive A-to-B persisted retrieval and negative A-to-C tests.
- Add `all_accessible`, visibility, admin-capability, and filter-widening tests.
- Add two concurrent dirty workspace overlays and prove strict run/generation
  isolation, including two runs of the same issue.
- Add restart and supersession tests for claim reconstruction, token
  replacement/revocation, and stale-token denial.
- Test OpenHands and Codex receive equivalent memory claims without raw tokens in
  manifests or diagnostics.
- Preserve the existing scoped-query, source-withdrawal, worker-grant,
  OpenHands MCP, and Codex environment regressions.
- Run focused memory MCP, code-graph, worker, and secret-canary tests plus
  `cargo test-system-duckdb --test memory` and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 15, 18.5, and 19.
- Inspect `crates/opensymphony-cli/src/orchestrator_run/backends.rs`,
  `crates/opensymphony-openhands/src/session.rs`,
  `crates/opensymphony-workspace/src/{manager,models}.rs`,
  `crates/opensymphony-memory/src/{lib,catalog,query,code_graph,capture,docs_sync}.rs`,
  and memory MCP dispatch in `crates/opensymphony-cli/src/memory.rs`.
- Reuse OSYM-879 target snapshots and OSYM-880/OSYM-881 overlay and indexed
  retrieval contracts.
- Trace the named baseline types and regressions before changing grant,
  filtering, overlay, credential, capture, or conversation-reuse behavior.

## Definition of Ready

- [x] The landed filter, source, leaf-grant, harness-injection, and verified
      envelope baseline is explicit.
- [x] The remaining claim lifecycle, sibling snapshot, overlay isolation,
      capture, and degradation work is measurable.
- [x] Parent grant and integration-overlay ownership is assigned to OSYM-891.

## Notes

The default leaf repository grant is the repositories associated with its
Linear project. Persisted sibling context is broader than live workspace truth:
only the execution repository's verified generation can contribute a live
overlay.
OSYM-891 derives descendant repository sets and parent grants, reconciles
multiple parent-owned integration overlays, and owns parent capture and
controller receipts; it reuses this task's enforcement primitives.
