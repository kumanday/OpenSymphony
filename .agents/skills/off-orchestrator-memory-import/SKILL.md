---
name: off-orchestrator-memory-import
description: Import OpenSymphony memory for work completed outside the orchestrator and Linear tracker. Use when asked to capture GitHub PRs, merge commits, direct commits, or other off-run-loop work into project memory by creating a temporary YAML evidence file, running `opensymphony memory import`, and syncing docs.
---

# Off-Orchestrator Memory Import

Create deterministic memory evidence for completed work that did not have a
Linear issue/run-loop capture, then import it through the normal memory tools.

## Workflow

1. Confirm the repo is clean enough to work in:

```sh
git status -sb
```

Do not touch unrelated dirty files.

2. Gather immutable source evidence:

```sh
gh pr view <number> --json number,title,url,body,headRefName,mergeCommit,mergedAt,commits,files,statusCheckRollup,reviews,state
git show --stat --oneline --find-renames <merge-or-direct-commit>
```

For commit-only work, use `git show --no-patch --format='%H%n%s%n%ad' <sha>`
and inspect the changed files with `git show --stat`.

3. Pick a synthetic issue identifier:

- GitHub PR work: `PR-<number>` (example: `PR-196`).
- Commit-only work: `LOCAL-YYYYMMDD-<short-slug>`.

4. Write a temp YAML file under `/tmp`, for example
`/tmp/opensymphony-pr196-memory.yaml`.

Use `url` on the synthetic issue so capture has a source URL. For PR work,
point it at the PR URL. Do not add area names as `labels`; use `overrides`
instead, otherwise `memory.yaml` can learn noisy aliases like every area name
on every selected area.

```yaml
issues:
  - identifier: PR-196
    title: Desktop task and diff latency plus live-refresh UX fixes
    url: https://github.com/kumanday/OpenSymphony/pull/196
    description: >
      Off-orchestrator GitHub PR work summarized in one or two sentences.
    state: Done
    milestone: "M12.9: Code Graph View"
    linked_prs:
      - 196
    completed_at: 2026-07-04T23:35:23Z
    comments:
      - source: git:commit:428acc99458df421d4b9d68b2020b6167c57e68a
        body: >
          Merged outside the OpenSymphony run-loop/Linear tracker. Capture
          preserves the GitHub PR, merge SHA, checks, commits, and changed files
          as durable source refs.

overrides:
  PR-196:
    areas:
      - desktop
      - frontend
      - gateway
      - graph-view
      - testing
      - ui

prs:
  - number: 196
    title: "perf(desktop): fix task/diff click latency and live-refresh selection clobbering"
    url: https://github.com/kumanday/OpenSymphony/pull/196
    branch: perf/desktop-latency-live-refresh
    merge_sha: 428acc99458df421d4b9d68b2020b6167c57e68a
    merged_at: 2026-07-04T23:35:23Z
    body: >
      Summarize the PR body. Keep the durable design/behavior facts, not review
      boilerplate or transcripts.
    commits:
      - sha: 428acc99458df421d4b9d68b2020b6167c57e68a
        timestamp: 2026-07-04T23:35:23Z
        summary: "perf(desktop): fix task/diff click latency and live-refresh selection clobbering (#196)"
    changed_files:
      - path: crates/opensymphony-gateway/src/lib.rs
        change_kind: modified
      - path: packages/ui-core/src/app-shell.ts
        change_kind: modified
    checks:
      - name: rust
        conclusion: success
      - name: typescript
        conclusion: success
```

5. Dry-run, then import:

```sh
opensymphony memory import PR-196 --source-file /tmp/opensymphony-pr196-memory.yaml --dry-run
opensymphony memory import PR-196 --source-file /tmp/opensymphony-pr196-memory.yaml
```

Use `--force` only when intentionally replacing an existing generated capsule.
If the dry-run shows unexpected areas or warnings, fix the YAML before import.

6. Sync docs through the memory tool:

```sh
opensymphony memory sync-docs --issues PR-196
opensymphony memory status
opensymphony memory brief PR-196
```

Do not hand-edit managed doc sections.

7. Verify and commit if requested:

```sh
git diff --check
git diff --stat
git status --short
```

Commit only tracked outputs that belong to the import, usually topic docs and
possibly `Cargo.toml`/`Cargo.lock` if the same request included a release bump.

## Notes

- Keep the temp YAML outside the repo unless the user asks to keep it.
- Do not copy full agent transcripts, review boilerplate, or private memory
  capsule paths into public docs.
- For commit-only work with no PR, prefer grouping it with the nearest real PR.
  If there is truly no PR, import a `LOCAL-*` issue with commit comments and
  report any capture warning instead of inventing a PR number.
