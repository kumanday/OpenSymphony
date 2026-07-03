---
id: OSYM-856
title: Workflow Target Branch Model And Init Customization
milestone: "M12.7: Workflow Target Branch Configuration"
priority: 2
estimate: 3
blockedBy: []
blocks: ["OSYM-857", "OSYM-858", "OSYM-859"]
areas:
  - cli
  - workflow
  - target-repo-bootstrap
parent: null
---

## Summary

Add the minimal target-branch value handling and workflow customization needed for target repositories to record a branch target without changing orchestrator scheduling state.

## Scope

### In scope

- Add a strict target branch parser/helper for local branch names.
- Extend workflow customization so generated `WORKFLOW.md` records a managed target branch marker.
- Patch the generated workflow's known `origin/main` branch-control text from the selected target branch during init.
- Keep `develop` as the default value when no override is provided.
- Add focused unit tests around marker rendering and branch-name validation.

### Out of scope

- Interactive prompt wiring.
- `opensymphony update` settings mode.
- GitHub branch protection or remote branch creation.
- Orchestrator, Linear, TUI, or desktop state changes.

## Deliverables

- Target branch helper in the CLI bootstrap/update code path.
- `WORKFLOW.md` customization support for the managed target branch marker.
- Unit tests covering default `develop`, explicit `main`, `release/next`, rejected `origin/develop`, and rejected `refs/heads/develop`.

## Acceptance Criteria

- [ ] `opensymphony init --non-interactive` without a target override renders `develop`.
- [ ] Workflow customization can render `develop` and `release/next` as local branch names.
- [ ] Workflow customization uses `origin/<target-branch>` only in generated agent guidance, never as the stored marker value.
- [ ] Invalid branch names fail before files are written.
- [ ] No orchestrator runtime model or scheduler code is touched.

## Test Plan

- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb --test init`.
- Run focused CLI unit tests for workflow customization and branch validation.

## Context

- Read `docs/specs/workflow-target-branch-update-spec.md`.
- Inspect `crates/opensymphony-cli/src/init_repo.rs` for `ReviewProviderArg`, `customize_workflow`, and placeholder replacement style.
- Inspect `crates/opensymphony-cli/tests/init.rs` for the mocked template assets and existing review-provider assertions.
- Treat the branch target as workflow guidance, not orchestrator state.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep this boring: one parser/helper and one workflow customization path.
