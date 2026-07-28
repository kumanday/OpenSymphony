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

Issue per-run memory grants that let a terminal worker retrieve persisted context
from authorized sibling repositories while exposing a live overlay only for its
own verified checkout.

## Scope

### In scope

- Define per-run memory claims for project set, Linear projects, work item,
  canonical repositories, visibility, and admin capabilities.
- Keep authorization claims independent of query filters; every filter narrows
  the grant and `all_accessible` never widens it.
- Give a terminal child persisted memory and target-branch code snapshots for
  repositories associated with its Linear project.
- Require an explicit canonical repository filter when querying another
  authorized repository.
- Expose a live code/memory overlay only for the worker's own verified checkout
  generation and record commit, dirtiness, and run owner.
- Deny unrelated repositories and every other worker's dirty overlay even when
  the caller requests all accessible records.
- Keep memory retrieval distinct from filesystem grants: knowing another
  repository's memory does not expose its checkout.
- Route code-intelligence lookup through canonical IDs and return source type,
  commit, path/symbol refs, freshness, and persisted-versus-live provenance.
- Capture leaf results with project, issue, execution repository, commits, and
  instruction hash.
- Sync docs only to an explicitly owning repository under its current
  instructions and review policy.
- Surface a visible degraded or blocked memory state when the central service is
  unavailable; never fall back silently to another repository's local store.

### Out of scope

- Parent descendant grants and integration overlays.
- Hosted organization/tenant RBAC.
- Filesystem access to a non-execution repository.

## Deliverables

- Memory claim and filter enforcement model.
- Leaf worker credential/env injection for OpenHands and Codex.
- Canonical cross-repository memory and code-intelligence retrieval.
- Leaf overlay ownership and provenance checks.
- Capture/docs-sync ownership and degraded-state behavior.
- Negative-scope and cross-repository integration tests.

## Acceptance Criteria

- [ ] A worker bound to repository A can explicitly query persisted memory and
      target-branch code for associated repository B.
- [ ] The same worker cannot query unrelated repository C, even with
      `all_accessible`.
- [ ] Project-set, project, work-item, repository, visibility, and area/path
      filters are enforced and cannot expand token claims.
- [ ] Repository A's live overlay is available only when its checkout generation
      and owning run match the grant.
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
  isolation.
- Test OpenHands and Codex receive equivalent memory claims without raw tokens in
  manifests or diagnostics.
- Run focused memory MCP, code-graph, worker, and secret-canary tests plus
  `cargo test-system-duckdb --test memory` and `git diff --check`.

## Context

- Read `docs/specs/multi-repo-orchestration-spec.md` sections 15, 18.5, and 19.
- Inspect `crates/opensymphony-cli/src/orchestrator_run/backends.rs`,
  `crates/opensymphony-openhands/src/session.rs`,
  `crates/opensymphony-memory/src/{query,code_graph,capture,docs_sync}.rs`, and
  memory MCP dispatch in `crates/opensymphony-cli/src/memory.rs`.
- Reuse OSYM-879 target snapshots and OSYM-880/OSYM-881 overlay and indexed
  retrieval contracts.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

The default leaf repository grant is the repositories associated with its Linear
project. Live workspace truth remains narrower than persisted context.
