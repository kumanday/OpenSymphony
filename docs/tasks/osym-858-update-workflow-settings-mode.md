---
id: OSYM-858
title: Update Workflow Settings Mode
milestone: "M12.7: Workflow Target Branch Configuration"
priority: 2
estimate: 5
blockedBy: ["OSYM-856"]
blocks: ["OSYM-859"]
areas:
  - cli
  - workflow
  - code-review
parent: null
---

## Summary

Teach `opensymphony update` to patch managed workflow settings for existing target repositories without reinstalling the CLI or refreshing template skills.

## Scope

### In scope

- Add `opensymphony update --target-branch <branch>`.
- Add `opensymphony update --code-review codex|openhands|none`.
- Enter workflow settings mode when either flag is present.
- Require target-repo markers before patching settings.
- Patch only the target-branch marker, the active review provider marker, and exact known legacy branch-control phrases.
- Skip Cargo self-update, template skill refresh, and memory bootstrap in marker-only settings mode.
- If `.github/workflows/ai-pr-review.yml` exists, attempt to enable it for `--code-review openhands`.
- If `.github/workflows/ai-pr-review.yml` exists, attempt to disable it for `--code-review codex` and `--code-review none`.
- Print a warning for `--code-review openhands` when OpenHands review workflow files are absent.
- Print an actionable warning when `gh workflow enable` or `gh workflow disable` cannot synchronize the existing workflow state.

### Out of scope

- Installing or repairing the full OpenHands PR-review GitHub Actions workflow.
- Rewriting unrelated `WORKFLOW.md` prose.
- Downloading template assets in marker-only settings mode.

## Deliverables

- Update CLI args and settings-mode routing.
- Workflow marker patch helper with tests for existing, missing, and malformed markers.
- Existing OpenHands review workflow enable/disable helper using GitHub CLI.
- Integration tests proving no template fetch occurs in marker-only mode.
- Integration tests proving existing OpenHands review workflow enable/disable is attempted for each provider state.
- Help output updates for the new flags.

## Acceptance Criteria

- [ ] `opensymphony update --target-branch develop` changes the branch marker and known legacy `origin/main` workflow-control text only.
- [ ] `opensymphony update --code-review codex` changes only the active review provider marker.
- [ ] `opensymphony update --target-branch develop --code-review codex` performs both changes in one write.
- [ ] Marker-only update mode does not call `cargo install opensymphony`.
- [ ] Marker-only update mode does not fetch template skill assets.
- [ ] `opensymphony update --code-review openhands` attempts to enable an existing `.github/workflows/ai-pr-review.yml`.
- [ ] `opensymphony update --code-review codex` attempts to disable an existing `.github/workflows/ai-pr-review.yml`.
- [ ] `opensymphony update --code-review none` attempts to disable an existing `.github/workflows/ai-pr-review.yml`.
- [ ] Missing OpenHands workflow files are warned about but not scaffolded.
- [ ] GitHub CLI workflow toggle failures leave the marker update in place and print the failed command.
- [ ] No-flag `opensymphony update` still performs the current self-update and skill refresh behavior.

## Test Plan

- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb --test update`.
- Run `cargo test-system-duckdb --test help`.

## Context

- Depends on OSYM-856.
- Read `docs/specs/workflow-target-branch-update-spec.md` section 7.
- Inspect `crates/opensymphony-cli/src/update_repo.rs` for target repo detection and current update sequencing.
- Inspect `crates/opensymphony-cli/tests/update.rs` for the fake crate/template server.
- Reuse the existing review-provider vocabulary from init.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

One patch helper is enough. Do not build a general Markdown rewrite engine, and
do not install OpenHands review assets from update settings mode.
