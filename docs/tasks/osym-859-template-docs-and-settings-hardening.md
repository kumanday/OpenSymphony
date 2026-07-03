---
id: OSYM-859
title: Template Docs And Settings Hardening
milestone: "M12.7: Workflow Target Branch Configuration"
priority: 3
estimate: 3
blockedBy: ["OSYM-856", "OSYM-857", "OSYM-858"]
blocks: []
areas:
  - documentation
  - template
  - workflow
parent: null
---

## Summary

Finish the configurable branch-target wave by aligning target-repo templates, documentation, and validation around the new init and update settings.

## Scope

### In scope

- Update target-repo workflow/template guidance so future `WORKFLOW.md` files expose the target branch marker.
- Update repo-local or template-managed `pull`, `push`, and `land` skill wording so agents follow the configured target branch instead of hard-coding `origin/main`.
- Update README and development docs for `opensymphony init --target-branch` and marker-only `opensymphony update` usage.
- Verify the planning spec's full init/update test matrix.
- Record that `--code-review` toggles existing OpenHands review workflows but does not install or repair missing workflow files.

### Out of scope

- Publishing the task package to Linear.
- Full OpenHands PR-review scaffold install or repair from update mode.
- Release tagging or Cargo publishing.

## Deliverables

- Updated target-repo template assets or template-sync instructions.
- Updated README/development docs for branch target and code review update flags.
- Final validation evidence for init, update, help, and formatting checks.

## Acceptance Criteria

- [ ] Fresh init output and updated existing workflow output use the same marker semantics.
- [ ] Generated or template-managed agent guidance tells agents to sync against the configured target branch.
- [ ] Documentation includes examples for the default `develop`, explicit `main`, `release/next`, and combined `--target-branch` plus `--code-review` update usage.
- [ ] Documentation explains that `--code-review openhands` enables an existing OpenHands review workflow, while `codex` and `none` disable an existing OpenHands review workflow.
- [ ] Documentation explains that missing OpenHands review workflow files are not created or repaired by update settings mode.
- [ ] All required checks from the spec pass or have explicit, reproducible blockers.

## Test Plan

- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb --test init`.
- Run `cargo test-system-duckdb --test update`.
- Run `cargo test-system-duckdb --test help`.
- Run `git diff --check`.

## Context

- Depends on OSYM-856, OSYM-857, and OSYM-858.
- Read `docs/specs/workflow-target-branch-update-spec.md`.
- Inspect `README.md`, `docs/DEVELOPMENT.md`, `WORKFLOW.md`, and `.agents/skills/pull`, `.agents/skills/push`, `.agents/skills/land`.
- The canonical shared Linear query assets live in the separate OpenSymphony-template repo; verify the current template path before changing adjacent repo assets.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep template sync explicit. If the implementation PR cannot modify the template repo, leave exact follow-up instructions instead of pretending it happened.
