# Project Memory

OpenSymphony project memory turns completed Linear work into durable development
context. During `opensymphony run`, terminal issue transitions are captured
automatically when memory auto-capture is enabled. The capture uses Linear issue
narrative, active Workpad content, issue hierarchy, milestones, GitHub PR
descriptions, reviews, checks, and source refs. It writes private issue capsules
under `.opensymphony/memory/`, updates a DuckDB index, evolves
`.opensymphony/memory/memory.yaml`, and syncs stable topics into public docs.

The CLI remains useful for setup, historical backfill, inspection, and manual
operator actions:

```bash
opensymphony memory init
opensymphony memory capture COE-123
opensymphony memory brief COE-123
opensymphony memory related --area openhands-runtime
opensymphony memory sync-docs --since-last-sync
opensymphony linear archive --issues COE-123
```

Use `--dry-run` on write commands when you want a non-writing preview.

## Configuration

`config.yaml` controls run-loop automation:

```yaml
memory:
  auto_capture: true
  auto_archive: false
```

`auto_capture` defaults to `true`. `auto_archive` defaults to `false`; when it
is enabled, OpenSymphony archives only after fresh capture succeeds with no
blocking warnings.

Initialize the shared memory policy and learned ontology file once:

```bash
opensymphony memory init
```

This creates `.opensymphony/memory/memory.yaml` and updates `.gitignore` so only
that config is tracked. Capsules, indexes, DuckDB, source snapshots, and
automation logs remain local runtime artifacts.

The config is not a hand-maintained docs map. It is a policy plus learned
structure file that capture can evolve as more work lands:

```yaml
memory_root: .opensymphony/memory
visibility: private
index_path: .opensymphony/memory/memory.duckdb
confidence_threshold: 75
source_snapshots: hashes
markdown_indexes: true
docs:
  public_root: docs
  default_visibility: public
  deny_private_links: true
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-agent-server.md
    visibility: public
    status: stable
    confidence: 85
    aliases:
      - OpenHands Runtime
    source_refs:
      docs:
        - docs/openhands-agent-server.md
      linear_labels:
        - runtime
      linear_issues:
        - COE-123
```

`memory init` seeds stable areas from existing top-level `docs/*.md` files when
they exist. It does not scan `docs/tasks`, `README.md`, Cargo files, source
files, or GitHub changed-file lists to create docs topics. When no docs exist,
the config is still valid and starts with an empty `areas` map.

## Capture Evidence

Live capture requires Linear access from `WORKFLOW.md` and uses GitHub PR
discovery by default through `gh`. For each issue, OpenSymphony reads:

- Linear title, description, labels, state, URL, milestone, parent, children,
  and active Workpad comment
- GitHub PR title, body, branch, checks, review discussion summaries, commits,
  merge SHA, and changed files

Area inference uses narrative evidence from Linear and GitHub. Merge SHA is not
used for inference or search; it is stored only under `source_refs` as the
immutable audit pointer to the exact merged code state. GitHub changed files are
indexed for later lookup such as "which issues touched this file?", but they are
not rendered into capsules or docs and do not infer areas.

Selecting a parent issue also captures its child issue closure. Capsules link
parents, children, and milestones so the Obsidian graph shows the work
structure.

Linear and GitHub are part of the normal live flow. A missing `WORKFLOW.md`,
invalid Linear config, missing issue, Linear API failure, or failing `gh`
command fails capture. Use `--no-github` only for unusual non-PR work.

## Import and Backfill

`memory import` is for deterministic backfills, migrations, tests, or external
exports. It is not the normal path.

```bash
opensymphony memory import --source-file completed.yaml
opensymphony memory import COE-123 --source-file completed.yaml
opensymphony memory import --issue-range COE-120..COE-130 --source-file completed.yaml
```

Top-level source YAML fields:

```yaml
issues: []
prs: []
overrides: {}
```

Important issue fields:

```yaml
issues:
  - id: issue-id
    identifier: COE-123
    title: Issue title
    url: https://linear.app/example/issue/COE-123
    description: Optional issue description
    state: Done
    milestone: M3
    milestone_id: milestone-id
    parent:
      identifier: COE-100
      title: Parent title
    children:
      - identifier: COE-124
        title: Child title
    labels:
      - runtime
    comments:
      - id: comment-id
        author: username
        body: "Decision or summary text"
        updated_at: 2026-03-25T22:05:00Z
        source: linear:workpad
    linked_prs:
      - 456
```

Important PR fields:

```yaml
prs:
  - number: 456
    title: COE-123 implement reconnect recovery
    url: https://github.com/example/repo/pull/456
    branch: coe-123-reconnect
    body: Pull request summary
    merge_sha: abcdef1234567890
    changed_files:
      - path: crates/opensymphony-openhands/src/client.rs
        change_kind: modified
    checks:
      - name: cargo test
        conclusion: success
    reviews:
      - reviewer: reviewer
        state: APPROVED
        disposition: Looks correct.
```

All fields except `issues[].identifier` and `prs[].number` are optional.

## Query and Docs Sync

Useful read commands:

```bash
opensymphony memory status
opensymphony memory brief COE-123
opensymphony memory related --area openhands-runtime
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory search "reconnect recovery"
opensymphony memory docs --area openhands-runtime
```

Docs sync writes stable topic docs by default and prints stat-style output with
file paths, line counts, and changed-line totals:

```bash
opensymphony memory sync-docs --since-last-sync
opensymphony memory sync-docs --issues COE-123
```

Candidate or low-confidence areas remain private until later captures raise
their confidence. Automation records warnings in `.opensymphony/memory/indexes`
so operators can inspect unresolved capture or docs-sync blockers. When the
Linear project overview content is available, OpenSymphony also maintains a
managed memory-status section there for capture warnings that need attention.

## Archive Guard

Archival is guarded by memory capture. For explicit issues,
`opensymphony linear archive` first performs live Linear and GitHub capture, then
archives only eligible issues:

```bash
opensymphony linear archive --issues COE-123
opensymphony linear archive --issue-range COE-120..COE-130
```

An issue is eligible when fresh captured memory exists and has no unresolved
capture warnings. `--force` bypasses the guard for a deliberate operator
recovery. To archive from already captured memory without recapturing, use
`--from-memory`.

## Troubleshooting

- If Linear fails, fix `WORKFLOW.md`, tracker credentials, or issue selection.
  Live capture does not fall back to placeholder records.
- If GitHub discovery fails, install/authenticate `gh` or intentionally rerun
  with `--no-github`.
- If docs sync writes no topic docs, inspect `.opensymphony/memory/memory.yaml`
  for candidate areas below the confidence threshold.
- Use `opensymphony memory capture --help`,
  `opensymphony memory import --help`, and
  `opensymphony linear archive --help` for the current command surface.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-399 contributed: PR #94: COE-399: Linear read coverage, schema drift validation, and task graph cache (merge `db4e2e6`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-399: Linear Read Coverage And Task Graph Cache

## Source refs

- COE-399

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
