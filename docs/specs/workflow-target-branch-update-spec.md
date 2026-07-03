# Workflow Target Branch And Update Settings Specification

Status: draft

Reader: an OpenSymphony engineer updating the target-repo bootstrap and update
flows.

Post-read action: implement configurable workflow target branches and marker-only
workflow updates for existing target repositories.

## 1. Summary

OpenSymphony target repositories currently assume that feature branches are kept
current with `origin/main` and that pull requests merge directly into `main`.
The new default should be `develop`, while operators still need a repo-level
setting for common alternatives:

- repositories whose default integration branch is `master`
- gitflow-style repositories where feature PRs target `develop`
- release-train repositories where operators intentionally choose another
  long-lived integration branch

The branch target is workflow guidance for agents and repo-local skills. It is
not orchestrator scheduling state.

## 2. Current State

- `opensymphony init` fetches the current template payload, customizes
  `WORKFLOW.md`, and already records the selected AI review provider by replacing
  a stable marker.
- `opensymphony update` currently updates the installed CLI, refreshes
  template-managed skills, and initializes memory files when run inside a target
  repo. It intentionally does not rewrite `WORKFLOW.md`.
- The generated workflow text has several hard-coded `origin/main` references
  for fresh branches, pull-sync work, final merge readiness, rework resets, and
  workpad examples.
- Repo-local `pull`, `push`, and `land` skills also mention `main` or
  `origin/main`, so the workflow must make the configured target branch
  authoritative over those generic examples.

## 3. Goals

1. Use `develop` as the default target branch.
2. Let `opensymphony init` prompt for a target branch and accept
   `--target-branch <branch>` for non-interactive setup.
3. Let existing target repos change workflow markers with:
   - `opensymphony update --target-branch develop`
   - `opensymphony update --code-review codex`
   - `opensymphony update --target-branch develop --code-review codex`
4. Patch only managed workflow markers and known branch-control text when update
   flags are present.
5. Do not refresh or re-download template skills in marker-only update mode.
6. Do not replace user-edited workflow content.

## 4. Non-Goals

- Do not add gitflow automation for release branches.
- Do not create, protect, or retarget remote branches through GitHub settings.
- Do not change the orchestrator state machine or Linear polling.
- Do not make the TUI or desktop clients own merge target decisions.
- Do not scaffold or repair missing OpenHands PR-review GitHub Actions from
  marker-only update mode.

## 5. Workflow Model

Add one managed branch marker near the existing automated review provider marker:

```markdown
## Branch target

Target branch: `develop`

<!-- Set by `opensymphony init` or `opensymphony update --target-branch`.
     Value is a local branch name, not an `origin/...` ref. Agents should use
     `origin/<target-branch>` when syncing, creating replacement branches, and
     preparing PRs. -->
```

Generated workflow guidance should refer to the configured target branch instead
of spelling `origin/main` as a fixed policy. The default generated text should
show `origin/develop`, but the marker is the single value update code edits
later.

When repo-local skills mention `main` or `origin/main` as the integration branch,
agents should treat that as the workflow target branch for this repository.
Future template skill text should say "configured target branch" and read the
marker, not bake a per-repo branch name into each skill file.

## 6. Init UX

Interactive `opensymphony init` adds one prompt:

```text
Target branch for feature PRs and syncs (default develop):
```

The prompt remains free-form text, but the surrounding help text should surface
examples in this style: `develop` (default), `main`, `release/next`, etc.

Default resolution:

1. explicit `--target-branch`
2. non-empty interactive response
3. `develop`

Non-interactive setup accepts:

```bash
opensymphony init --non-interactive --target-branch main
```

Branch validation should be boring and strict:

- trim surrounding whitespace
- reject empty values
- reject `origin/foo` and `refs/heads/foo`; the flag takes `foo`
- reject names Git rejects via `git check-ref-format --branch`
- allow normal slash branches such as `release/next`

## 7. Update UX

When neither `--target-branch` nor `--code-review` is present,
`opensymphony update` keeps its current behavior.

When either marker flag is present, `opensymphony update` enters workflow
settings mode:

1. require target-repo markers (`WORKFLOW.md` and `config.yaml`)
2. read the existing `WORKFLOW.md`
3. update or insert only the managed target-branch and review-provider markers
4. patch exact known legacy `origin/main` workflow-control phrases when changing
   the target branch
5. write `WORKFLOW.md` only if content changed
6. skip Cargo self-update, template skill refresh, and memory bootstrap

Supported code review values should reuse the existing provider vocabulary:

```text
codex | openhands | none
```

`--code-review` is marker-first, not marker-only. It must patch the workflow
provider marker and then synchronize any already-installed OpenHands PR-review
GitHub Actions workflow:

- `--code-review openhands`: if `.github/workflows/ai-pr-review.yml` already
  exists, attempt to enable that workflow for the repository.
- `--code-review codex` or `--code-review none`: if
  `.github/workflows/ai-pr-review.yml` already exists, attempt to disable that
  workflow so an outdated OpenHands review job does not keep running after the
  marker switches away from `openhands`.
- If the workflow file is missing, do not create it. Print a clear warning for
  `--code-review openhands` explaining that update mode did not install the
  OpenHands review workflow.
- If GitHub CLI access is unavailable or workflow enable/disable fails, keep the
  marker change and print an actionable warning with the failed command. Do not
  silently pretend the GitHub Actions state was synchronized.

If a marker is missing, insert the minimal managed section instead of fetching
the whole template. If a marker is malformed, fail with a clear message and do
not guess.

## 8. Implementation Notes

- Reuse the existing review provider enum instead of adding a second provider
  model.
- Reuse `gh workflow enable` / `gh workflow disable` for already-installed
  OpenHands review workflow toggles. Do not edit workflow YAML by hand just to
  toggle it.
- Add a small target-branch value type or helper, not a new config subsystem.
- Extend workflow customization to replace both review provider and target
  branch during init.
- Add a marker patch helper for update settings mode. It should operate on
  normalized line endings and preserve the rest of the file byte-for-byte where
  possible.
- Keep no-flag update behavior unchanged.
- Update the template repository before relying on generated target-repo files.

## 9. Validation

Add focused tests for:

- init writes `Target branch: ` followed by `develop` by default
- init writes `main` with `--target-branch main`
- init rejects `origin/develop`
- update with `--target-branch develop` changes only the branch marker and known
  legacy `origin/main` workflow-control text
- update with `--code-review codex` changes only the active review provider
  marker
- combined update changes both markers in one write
- settings update does not fetch template skill assets
- `--code-review openhands` attempts to enable an existing
  `.github/workflows/ai-pr-review.yml`
- `--code-review codex` and `--code-review none` attempt to disable an existing
  `.github/workflows/ai-pr-review.yml`
- no-flag update still refreshes template-managed skills
- help output documents both new flags

Run at least:

```bash
cargo fmt --check
cargo test-system-duckdb --test init
cargo test-system-duckdb --test update
cargo test-system-duckdb --test help
```

Use the dev fallback aliases if system DuckDB is unavailable.

## 10. Deferred

If operators later want `opensymphony update --code-review openhands` to install
or repair the full OpenHands review workflow when it is missing or stale, add an
explicit flag for that scaffold. Do not hide creation or repair inside workflow
settings mode.
