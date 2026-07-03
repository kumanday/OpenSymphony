---
id: OSYM-857
title: Init Target Branch Prompt And Flag
milestone: "M12.7: Workflow Target Branch Configuration"
priority: 3
estimate: 3
blockedBy: ["OSYM-856"]
blocks: ["OSYM-859"]
areas:
  - cli
  - workflow
  - target-repo-bootstrap
parent: null
---

## Summary

Expose the workflow target branch through `opensymphony init` with an interactive prompt and a non-interactive flag.

## Scope

### In scope

- Add `opensymphony init --target-branch <branch>`.
- Prompt interactive users for the target branch during workflow customization.
- Surface branch examples in the prompt/help text, such as `develop` (default), `main`, `release/next`, etc.
- Use `develop` as the prompt default when no explicit target branch is supplied.
- Keep non-interactive mode prompt-free and fail fast on invalid explicit values.
- Update init help output tests.

### Out of scope

- Marker-only `opensymphony update` behavior.
- Automatically creating or fetching missing remote target branches.
- Changing the selected Git branch of the target repository.

## Deliverables

- CLI argument and help text for `opensymphony init --target-branch`.
- Prompt/default resolution code in the init flow.
- Init tests for default `develop`, explicit `main`, invalid refs, and no-prompt non-interactive behavior.

## Acceptance Criteria

- [ ] Interactive init asks for the target branch only when `WORKFLOW.md` will be created or customized.
- [ ] Interactive init keeps the branch entry free-form while showing examples for common branch names.
- [ ] Empty interactive input uses `develop`.
- [ ] `opensymphony init --non-interactive --target-branch main` writes `main`.
- [ ] `opensymphony init --target-branch origin/develop` exits with a clear error.
- [ ] Existing review-provider, Linear slug, conflict-policy, and commit/push flows still behave as before.

## Test Plan

- Run `cargo fmt --check`.
- Run `cargo test-system-duckdb --test init`.
- Run `cargo test-system-duckdb --test help`.

## Context

- Depends on OSYM-856.
- Read `docs/specs/workflow-target-branch-update-spec.md` section 6.
- Inspect `prompt_review_provider` and `prompt_with_default` in `crates/opensymphony-cli/src/init_repo.rs`.
- Preserve the existing non-interactive contract from OSYM-738.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not over-detect. The default is `develop`; `main`, `release/next`, and other branch names are explicit choices.
