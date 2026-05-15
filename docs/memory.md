# Project Memory

OpenSymphony project memory preserves completed-issue knowledge before Linear
issues are archived. The normal workflow captures evidence from Linear and
GitHub, writes private issue capsules under `.opensymphony/memory/`, indexes
them in DuckDB, and can sync selected knowledge into public topic docs.

Project memory has two separate surfaces:

- **CLI commands** such as `opensymphony memory capture` and
  `opensymphony linear archive` create, query, sync, and guard memory.
- **Agent skills** under `.agents/skills/` tell implementation agents when to
  consult memory. Those skills are target-repo template assets, not files
  embedded or injected by the OpenSymphony binary.

## Normal live capture

Live capture requires Linear access from `WORKFLOW.md` and uses GitHub PR
discovery by default through `gh`:

```bash
opensymphony memory capture COE-123 --dry-run
opensymphony memory capture COE-123
opensymphony memory capture --issues COE-123,COE-124
opensymphony memory capture --issue-range COE-120..COE-130 --dry-run
```

For each selected issue, OpenSymphony:

1. reads the Linear issue, state, labels, URL, description, and active workpad
   comment;
2. discovers matching GitHub PRs with `gh pr list --search ISSUE-KEY`;
3. enriches matched PRs with changed files, commits, reviews, checks, and merge
   SHA using `gh pr view`;
4. renders a capture plan or writes the issue capsule and index entry.

Linear is always attempted for live capture. A missing `WORKFLOW.md`, invalid
Linear configuration, missing issue, or Linear API failure is a command failure.
GitHub is also part of the default live flow. A missing or failing `gh` command
is a command failure unless `--no-github` is supplied:

```bash
opensymphony memory capture COE-123 --no-github --dry-run
```

`--no-github` is intended for unusual non-PR work. If GitHub is available but no
matching PR is found, capture records a warning. Warnings keep the issue visible
for review and block archival unless `--force` is used.

## Import and backfill

`memory import` is for deterministic backfills, migrations, tests, or external
exports. Failed Linear or GitHub access should be fixed before live capture is
retried.

```bash
opensymphony memory import --source-file completed.yaml --dry-run
opensymphony memory import --source-file completed.yaml
opensymphony memory import COE-123 --source-file completed.yaml
opensymphony memory import --issue-range COE-120..COE-130 --source-file completed.yaml --dry-run
```

The source file is produced by a user or external export tool. OpenSymphony does
not currently generate this file during the normal live capture flow.

Import selection flags filter records already present in the YAML:

- issue selectors: positional issue, `--issues`, `--issues-file`,
  `--issue-range`
- source filters: `--milestone`, `--state`, `--before-date`, `--before-issue`

If selected records are not present, import fails instead of inventing
placeholder issue evidence.

### Source YAML schema

Top-level fields:

```yaml
issues: []
prs: []
overrides: {}
```

Issue fields:

```yaml
issues:
  - id: issue-id
    identifier: COE-123
    title: Issue title
    url: https://linear.app/example/issue/COE-123
    description: Optional issue description
    state: Done
    milestone: M3
    labels:
      - runtime
    comments:
      - author: username
        body: "Decision or summary text"
        updated_at: 2026-03-25T22:05:00Z
        source: linear:workpad
    linked_prs:
      - 456
    task_files:
      - docs/tasks/COE-123.md
    updated_at: 2026-03-25T22:05:00Z
    completed_at: 2026-03-26T10:00:00Z
```

PR fields:

```yaml
prs:
  - number: 456
    title: COE-123 implement reconnect recovery
    url: https://github.com/example/repo/pull/456
    branch: coe-123-reconnect
    body: Pull request summary
    merge_sha: abcdef1234567890
    merged_at: 2026-03-26T10:30:00Z
    commits:
      - sha: abcdef1234567890
        author: username
        timestamp: 2026-03-26T10:00:00Z
        summary: Implement reconnect recovery
    changed_files:
      - path: crates/opensymphony-openhands/src/client.rs
        change_kind: modified
    checks:
      - name: cargo test
        conclusion: success
        completed_at: 2026-03-26T10:20:00Z
    reviews:
      - reviewer: reviewer
        state: APPROVED
        submitted_at: 2026-03-26T10:25:00Z
        disposition: Looks correct.
```

Overrides are keyed by issue identifier:

```yaml
overrides:
  COE-123:
    prs:
      - 456
    areas:
      - openhands-runtime
```

All fields except `issues[].identifier` and `prs[].number` are optional.
`linked_prs` and `overrides.*.prs` associate issue records with PR records in
the same source file.

## Query and docs sync

Useful read commands:

```bash
opensymphony memory status
opensymphony memory brief COE-123
opensymphony memory related --area openhands-runtime
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory search "reconnect recovery"
opensymphony memory docs --area openhands-runtime
```

Docs sync is review-first. It shows the managed-section diff before writing:

```bash
opensymphony memory sync-docs --issues COE-123 --dry-run
opensymphony memory sync-docs --issues COE-123
opensymphony memory lint --public-docs
```

`opensymphony-memory.yaml` configures memory roots, visibility, area detection,
docs targets, and redaction. See [Configuration](configuration.md#project-memory)
for the config shape.

## Archive guard

Archival is guarded by memory capture. For explicit issues,
`opensymphony linear archive` first performs live Linear and GitHub capture, then
archives only eligible issues:

```bash
opensymphony linear archive --issues COE-123 --dry-run
opensymphony linear archive --issues COE-123
opensymphony linear archive --issue-range COE-120..COE-130 --dry-run
```

An issue is eligible when fresh captured memory exists and has no unresolved
capture warnings. `--force` bypasses the guard when an operator has reviewed the
risk:

```bash
opensymphony linear archive --issues COE-123 --force
```

To archive from already captured memory without recapturing, use
`--from-memory`:

```bash
opensymphony linear archive --from-memory --state captured --dry-run
opensymphony linear archive --from-memory --state pending
```

`--state` only applies to `--from-memory`. Explicit issue archive selectors use
the normal live capture path.

## Troubleshooting

- Use `opensymphony memory capture ... --dry-run` before running the writing command.
- Use `opensymphony memory capture --help`,
  `opensymphony memory import --help`, and
  `opensymphony linear archive --help` for the current command surface.
- If Linear fails, fix `WORKFLOW.md`, tracker credentials, or issue selection.
  Live capture does not fall back to placeholder records.
- If GitHub discovery fails, install/authenticate `gh` or intentionally rerun
  with `--no-github`.
- If archive is blocked by warnings, inspect the capture dry-run or capsule,
  refresh capture, or use `--force` only after review.
