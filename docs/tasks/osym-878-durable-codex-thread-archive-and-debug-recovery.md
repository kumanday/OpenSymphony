---
id: OSYM-878
title: Durable Codex Thread Archive And Debug Recovery
milestone: "M12.85: Codex Thread Lifecycle"
priority: 2
estimate: 8
blockedBy: ["OSYM-877"]
blocks: []
areas:
  - codex-runtime
  - workspace-lifecycle
  - debugging
parent: null
---

## Summary

Archive the one canonical Codex thread only after an issue is terminal, then
unarchive that same thread before debug or reopened work resumes it.

## Scope

### In scope

- Add a Serde-defaulted `codex_archive_state` (`active`, `archiving`,
  `archived`, `unarchiving`) and narrow manifest transition helpers that retain
  the canonical id and original creation timestamp.
- Add schema-validated `thread/list`, `thread/archive`, and `thread/unarchive`
  operations plus a paginated state inspector using the exact workspace CWD,
  `useStateDbOnly`, archived-first lookup, and only normal top-level sources.
- Reconcile pending archive transitions at lifecycle boundaries without reading
  Codex's private SQLite database.
- Archive active canonical threads after terminal scheduler reconciliation;
  treat already-archived threads as no-ops and retry failures on later ticks
  without blocking unrelated issues.
- Unarchive before Codex debug, `--app` deep links, or reopened-issue resumes;
  stop before interactive resume when unarchive fails and print the id plus the
  manual recovery command.
- Update Codex harness and operations documentation with lifecycle behavior and
  manual recovery guidance.

### Out of scope

- Replacing missing threads, an automatic history registry, or automatic
  rearchive immediately after interactive debug.
- Changes to OpenHands archival or debug behavior.
- Codex sidebar repair or SQLite inspection.

## Deliverables

- Durable archive-state manifest model, lifecycle request builders, and state
  inspection helper.
- Terminal archive reconciliation and pending-state recovery.
- Debug/deep-link/reopen unarchive behavior that preserves the same id.
- Operator recovery documentation and end-to-end fake-app-server coverage.

## Acceptance Criteria

- [ ] A terminal active thread transitions through `archiving` to `archived`
  once; repeated terminal ticks and daemon restart are no-ops after success.
- [ ] Archive failures and missing canonical threads fail closed, retain the
  workspace and manifest, report the id, and create no replacement thread.
- [ ] Pending archive and unarchive states reconcile correctly after simulated
  interruption using Codex app-server state rather than SQLite or error text.
- [ ] Debug and `--app` unarchive an archived thread before launching or
  returning it; active threads have no archive mutation, and a failed unarchive
  prevents resume with manual recovery guidance.
- [ ] A reopened issue unarchives and resumes the original id, while existing
  OpenHands debug and Codex interrupt behavior remain unchanged.
- [ ] One fake end-to-end scenario covers initial run, successful and failed
  retries, restart, terminal archive, debug unarchive, and reopen with one
  canonical id and one started Codex thread.

## Test Plan

- Add generated-schema contract tests for list, archive, unarchive, pagination,
  source filtering, and state-response handling.
- Add terminal reconciliation and pending-transition restart tests using the
  fake app-server, including no-op, failure, and missing-thread paths.
- Add debug and deep-link tests for active, archived, and unarchive-failure
  paths; rerun existing OpenHands debug and Codex interrupt coverage.
- Run focused `codex_app_server`, `run`, and `debug` test targets through
  `cargo test-system-duckdb`, then `cargo fmt --check` and `git diff --check`.

## Context

- Read `docs/specs/codex-thread-lifecycle-spec.md` from Durable Lifecycle State
  through Operational Recovery, especially the terminal, debug, and end-to-end
  requirements.
- Build on OSYM-877's canonical manifest path; this task is blocked until
  retries cannot create replacement threads.
- Inspect `crates/opensymphony-cli/src/orchestrator_run/backends.rs` terminal
  reconciliation and `crates/opensymphony-cli/src/debug_session.rs` Codex debug
  and deep-link paths.
- Preserve `crates/opensymphony-workspace` retention as the storage guarantee
  for recovery artifacts.
- Update `docs/codex-app-server-harness.md` and `docs/operations.md`; the
  lifecycle specification is the authority for behavior.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Use app-server state only at lifecycle boundaries. A terminal archive must not
become an event-loop query or a reason to invent a second conversation store.
