# Project Memory

OpenSymphony project memory turns completed Linear work into durable development
context. During `opensymphony run`, terminal issue transitions are captured
automatically when memory auto-capture is enabled. The capture uses Linear issue
narrative, active Workpad content, issue hierarchy, milestones, GitHub PR
descriptions, reviews, checks, and source refs. It writes private issue capsules
under `.opensymphony/memory/`, updates a DuckDB index, evolves
`.opensymphony/memory/memory.yaml`, and syncs stable topics into public docs.

## Rationale and Model

Linear remains OpenSymphony's short-term planning and coordination surface, but
completed issues still carry implementation knowledge that should survive queue
cleanup and archival. GitHub PRs preserve code review history, and Workpad
comments preserve useful audit details, but neither is a convenient
component-oriented project memory on its own.

OpenSymphony memory is the completed-work distillation of what mattered: the
intent, shipped outcome, decisions, validation evidence, relevant review
feedback, follow-ups, risks, and documentation impact. Implementation agents
should keep their focus on code, validation, PRs, and Workpad accuracy; long-term
memory is produced by capture and docs sync rather than by ad hoc edits during
ordinary feature work.

The memory system has two different outputs:

- Issue capsules record what happened for one completed Linear issue. They are
  compact source-referenced closeout documents, not run transcripts.
- Topic docs describe what is now true about a subsystem. They capture the
  current model, invariants, gotchas, and recent changes without requiring
  readers to know which issue introduced the knowledge.

The default visibility posture is private memory with optional public docs.
Private capsules may include Linear comments, review context, and source
snapshots, while public docs should contain public source refs such as issue
identifiers, PR URLs, and commit SHAs. Public docs must not link directly to
private capsule paths. Public memory is supported only by explicit
configuration, and generated indexes such as DuckDB should remain local unless a
project deliberately publishes them.

Areas bridge issue memory and topic docs. Area inference uses Linear narrative,
labels, milestones, active Workpad content, PR narrative, review summaries, and
existing learned aliases in `.opensymphony/memory/memory.yaml`. GitHub changed
files are indexed for path-based lookup, but they do not create areas or appear
in capsule or docs prose. Merge SHAs are immutable audit refs, not inference
signals.

The CLI remains useful for setup, historical backfill, inspection, and manual
operator actions:

```bash
opensymphony memory init
opensymphony memory capture COE-123
opensymphony memory context --issue COE-456
opensymphony memory brief COE-123
opensymphony memory related --area openhands-runtime
opensymphony memory sync-docs --since-last-sync
opensymphony memory serve --addr 127.0.0.1:8765
opensymphony linear archive --issues COE-123
```

Use `--dry-run` on write commands when you want a non-writing preview.

## Configuration

`config.yaml` controls run-loop automation:

```yaml
memory:
  auto_capture: true
  auto_archive: false
  serve: true
  bind: 127.0.0.1:0
```

`auto_capture` defaults to `true`. `auto_archive` defaults to `false`; when it
is enabled, OpenSymphony archives only after fresh capture succeeds with no
blocking warnings. `serve` starts the local memory server during
`opensymphony run` when memory is initialized. The default bind address uses an
ephemeral loopback port, and workers receive the resulting MCP endpoint through
`OPENSYMPHONY_MEMORY_ENDPOINT`. Workers receive only the normal read token;
admin tools require a separate `OPENSYMPHONY_MEMORY_ADMIN_TOKEN`.

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

Area inference treats Linear labels named `area:<slug>` as canonical. Existing
label aliases and narrative evidence from Linear and GitHub still work as
fallbacks. Merge SHA is not used for inference or search; it is stored only
under `source_refs` as the immutable audit pointer to the exact merged code
state. GitHub changed files are indexed for later lookup such as "which issues
touched this file?", but they are not rendered into capsules or docs and do not
infer areas.

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
opensymphony memory context --issue COE-456
opensymphony memory brief COE-123
opensymphony memory related --area openhands-runtime
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory search "reconnect recovery"
opensymphony memory docs --area openhands-runtime
```

`memory context` is a pre-implementation context compiler, not a capture
command. It fetches live Linear facts when available, excludes the current issue
capsule, and selects captured memory from deterministic buckets: explicit
includes, blocking predecessors, completed children, completed siblings, path
matches, and canonical area matches. It strips each selected brief's
`Documentation impact` section and appends one deduplicated section at the end.
When `opensymphony run` starts a worker, it asks the supervised memory server
for the same style of kickoff bundle and writes it to
`.opensymphony/generated/memory-context.md` inside the issue workspace. If the
server is disabled, the runner falls back to direct local memory reads.

Read commands open the DuckDB index in read-only mode and do not run migrations.
Startup and write paths own schema creation or migration.

`memory serve` exposes the memory command set through a local MCP-style
Streamable HTTP JSON-RPC endpoint at `/mcp`. CLI commands call that endpoint
when `OPENSYMPHONY_MEMORY_ENDPOINT` is set; otherwise they use offline direct
mode. Read tools are `memory.context`, `memory.search`, `memory.related`,
`memory.brief`, `memory.docs`, and `memory.status`. Admin tools are
`memory.capture`, `memory.sync_docs`, `memory.lint`, `memory.reindex`, and
`memory.ingest_code_intel`; these require `OPENSYMPHONY_MEMORY_ADMIN_TOKEN` or
`--admin-token` on `opensymphony memory serve`. If an admin token is configured
without a separate read token, the admin token also protects read tools.
`memory.context` builds the agent kickoff bundle. Add `--include-code-intel`
to include available codebase-analysis artifacts alongside selected memory.

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

- COE-252 contributed: PR #10: Implement foundation workflow and scheduler contracts
- COE-253 contributed: PR #19: COE-253: OpenHands Runtime Adapter (merge `911b0b4`)
- COE-254 contributed: PR #6: COE-254: bootstrap tracker, workspace, and orchestration core
- COE-255 contributed: PR #4: COE-255: add control plane and FrankenTUI slice
- COE-256 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-258 contributed: PR #83: Add memory init and mapped docs sync

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-252: Foundation and Contracts
- COE-253: OpenHands Runtime Adapter
- COE-254: Tracker, Workspaces, and Orchestration
- COE-255: Observability and FrankenTUI
- COE-256: Validation and Local Operations
- COE-258: Bootstrap workspace and crate boundaries
- COE-259: Workflow loader and typed config
- COE-260: Domain model and orchestrator state machine
- COE-261: Local agent-server supervisor
- COE-262: REST client and conversation contract
- COE-263: Workspace manager and lifecycle hooks
- COE-264: Linear read adapter and issue normalization
- COE-265: WebSocket event stream, reconciliation, and recovery
- COE-266: Issue session runner
- COE-267: Linear MCP write surface
- COE-268: Orchestrator scheduler, retries, and reconciliation
- COE-269: Control-plane API and snapshot store
- COE-270: Repository harness and generated context artifacts
- COE-271: FrankenTUI operator client
- COE-272: Fake OpenHands server and protocol contract suite
- COE-273: Live local end-to-end suite
- COE-274: CLI packaging, doctor, and local operations docs
- COE-277: Implement hierarchy-aware task selection
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-284: Add orchestrator run command to CLI and make it installable
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-393: Event Journal And Stream Broker
- COE-396: Action Receipts And Initial Run Actions
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-403: Terminal And Log Renderer Prototype
- COE-410: Desktop Local Stream Optimization
- COE-413: Implementation Plan Generator Stage
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-448: Multi-repo memory server and deterministic context

## Source refs

- COE-252
- COE-253
- COE-254
- COE-255
- COE-256
- COE-258
- COE-259
- COE-260
- COE-261
- COE-262
- COE-263
- COE-264
- COE-265
- COE-266
- COE-267
- COE-268
- COE-269
- COE-270
- COE-271
- COE-272
- COE-273
- COE-274
- COE-277
- COE-280
- COE-281
- COE-282
- COE-284
- COE-287
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-393
- COE-396
- COE-399
- COE-403
- COE-410
- COE-413
- COE-434
- COE-448

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
