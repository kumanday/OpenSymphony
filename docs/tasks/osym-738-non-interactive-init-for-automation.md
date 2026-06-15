---
id: OSYM-738
title: Non-Interactive Init For Automation
milestone: "M9.5: Developer Build Acceleration"
priority: 2
estimate: 2
blockedBy: []
blocks: []
areas:
  - cli
  - developer-experience
  - operations
parent: null
---

## Summary

Make `opensymphony init` usable in non-interactive automation by accepting explicit flags for every prompt-driven decision. This lets power users provision separate OpenSymphony target repositories or VMs without a terminal prompt loop while preserving the existing guided default for humans.

## Scope

### In scope

- Add a non-interactive `opensymphony init` mode that fails fast when a required decision is missing instead of reading stdin.
- Add flags for AI PR review setup, AI review provider settings, Linear project slug, existing-file conflict behavior, missing LLM environment guidance values, and optional bootstrap commit/push.
- Keep the current interactive flow and defaults unchanged unless the new mode or flags are explicitly provided.
- Document automation-oriented examples for VM/provisioning scripts.
- Add CLI tests that prove non-interactive init succeeds without stdin prompts and fails with a clear error when a required conflict decision is omitted.

### Out of scope

- Changing template-managed file contents beyond values already customized by init.
- Adding multi-repo orchestration support.
- Changing `opensymphony update` behavior.
- Replacing the existing interactive first-run experience.

## Deliverables

- Updated `InitArgs` and init command flow in `crates/opensymphony-cli/src/init_repo.rs`.
- Focused tests in `crates/opensymphony-cli/tests/init.rs`.
- Updated `docs/operations.md` and `docs/DEVELOPMENT.md` examples for automation.
- Updated task package and Linear publish metadata for M9.5.

## Acceptance Criteria

- [ ] `opensymphony init --non-interactive` never prompts on stdin.
- [ ] Non-interactive init accepts explicit flags for all user choices needed by the current prompt flow.
- [ ] Non-interactive init succeeds in a new target repository when all required automation flags are supplied.
- [ ] Non-interactive init fails before writing when it encounters an unresolved prompt-only decision such as an existing-file conflict without a conflict-policy flag.
- [ ] Existing interactive init tests continue to pass.
- [ ] Docs include a copyable automation example for provisioning a target repository or VM.

## Test Plan

- `cargo fmt --check`
- `cargo test-system-duckdb --test init`
- `cargo test-system-duckdb --test help`
- `cargo test-system-duckdb --test update`

## Context

- The current init flow lives in `crates/opensymphony-cli/src/init_repo.rs` and already centralizes prompt reads through `InitUi`.
- Existing coverage in `crates/opensymphony-cli/tests/init.rs` exercises the interactive defaults, AI PR review setup, conflict handling, and commit/push behavior.
- M9.5 is the right milestone because this is a developer and power-user acceleration feature that should land before the patch release while the release prep is already open.
- The motivating user provisions separate VMs per OpenSymphony orchestrator until the future multi-repo orchestrator is implemented.

## Definition of Ready

- [x] Hidden assumptions from prior discussion are written down.
- [x] Required files, docs, and dependencies are explicitly referenced.
- [x] A coding agent could begin execution without additional planning context.

## Notes

Treat the automation mode as a thin CLI surface over the existing init behavior. Prefer explicit flags and deterministic failure over inferred behavior when a missing answer could overwrite files, configure GitHub, or commit/push changes.
