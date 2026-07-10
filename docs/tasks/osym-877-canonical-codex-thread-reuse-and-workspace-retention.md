---
id: OSYM-877
title: Canonical Codex Thread Reuse And Workspace Retention
milestone: "M12.85: Codex Thread Lifecycle"
priority: 2
estimate: 8
blockedBy: []
blocks: ["OSYM-878"]
areas:
  - codex-runtime
  - workspace-lifecycle
parent: null
---

## Summary

Make the existing conversation manifest the sole canonical Codex thread record
for an issue: start once, resume the same id on every later run, and retain the
workspace that holds that record when the issue becomes terminal.

## Scope

### In scope

- Add typed, installed-schema-validated `thread/resume` and the known-active
  `thread/archive` rollback request; keep `turn/start` as a separate operation.
- Load and validate the Codex conversation manifest before any `thread/start`.
- Start only when no manifest exists; otherwise resume the recorded id, verify
  the returned id matches, and fail closed on an invalid, incompatible, missing,
  or failed-resume manifest path.
- Preserve the canonical id and original creation timestamp; seed the full
  workflow prompt only after Codex accepts `turn/start`, then use the existing
  shared continuation guidance rather than duplicating it.
- Best-effort archive a newly started thread if persisting its first manifest
  fails, then report that id without starting a turn.
- Route terminal cleanup through `WorkspaceManager` so configured retention and
  lifecycle hooks preserve the conversation manifest.

### Out of scope

- Terminal reconciliation that archives an otherwise retained workspace.
- Debug or reopened-issue unarchive behavior.
- A thread registry, automatic reset, or Codex SQLite access.

## Deliverables

- Distinct start, resume, turn, and rollback request builders in the Codex
  adapter with generated-schema coverage.
- Manifest-first Codex worker lifecycle and prompt-seeding behavior.
- Manager-owned terminal workspace cleanup.
- Focused fake-app-server and workspace retention regression coverage.

## Acceptance Criteria

- [ ] Three attempts for one issue produce one `thread/start`, three turns, and
  one unchanged canonical thread id.
- [ ] A valid existing Codex manifest causes `thread/resume`, never a fallback
  `thread/start`, including failed, stalled, cancelled, and recovery retries.
- [ ] An unseeded manifest receives the full prompt; a seeded manifest receives
  the existing continuation guidance, without changing its id or `created_at`.
- [ ] A resume, response-validation, or turn failure leaves the manifest intact
  and reports the issue and canonical id through structured diagnostics.
- [ ] A first-manifest write failure attempts rollback archive, reports the new
  id, and does not send `turn/start`.
- [ ] Terminal cleanup honors the workspace manager's retention decision and
  preserves the conversation manifest and lifecycle-hook behavior.

## Test Plan

- Add installed-schema request and response tests for real `thread/resume` and
  rollback archive serialization.
- Exercise the fake stdio worker across initial, retry, unseeded, seeded,
  model-override, manifest-write-failure, and resume-failure paths.
- Run the focused `codex_app_server` and `run` test targets through
  `cargo test-system-duckdb`, plus workspace cleanup tests.
- Run `cargo fmt --check` and `git diff --check`.

## Context

- Read `docs/specs/codex-thread-lifecycle-spec.md` through Slice One and its
  required worker and terminal-retention tests.
- Inspect `crates/opensymphony-codex/src/lib.rs` request builders and
  `crates/opensymphony-cli/src/orchestrator_run/backends.rs` Codex worker.
- Reuse `crates/opensymphony-openhands/src/session.rs` continuation guidance
  instead of maintaining a Codex-only copy.
- Use `crates/opensymphony-workspace/src/manager.rs` cleanup decisions; do not
  retain the backend's direct terminal directory deletion.
- Existing Codex start and interrupt plumbing from OSYM-767 and OSYM-807 is a
  dependency context, not work to duplicate.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This is the prerequisite slice. Do not add terminal archival until retries no
longer create replacement threads.
