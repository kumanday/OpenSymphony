# Project Memory

OpenSymphony project memory turns completed Linear work into durable development
context. During `opensymphony run`, terminal issue transitions are captured
automatically when memory auto-capture is enabled. The capture uses Linear issue
narrative, active Workpad content, issue hierarchy, milestones, GitHub PR
descriptions, reviews, checks, and source refs. It writes private issue capsules
under `.opensymphony/memory/`, updates a DuckDB index, evolves
`.opensymphony/memory/memory.yaml`, and syncs stable topics into public docs.

Related specifications:

- [OKF Memory System Specification](okf-memory-spec.md) describes how the
  Markdown memory corpus should evolve into Open Knowledge Format bundles.
- [LLM Wiki Graph View Specification](llm-wiki-graph-view-spec.md) describes
  the client-side graph explorer for those bundles.

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

## OKF Bundle Compatibility

OpenSymphony treats OKF as the portable Markdown contract for memory, not as a
replacement for the current local store. The logical bundle layout follows
`docs/okf-memory-spec.md`:

```text
bundle-root/
  index.md
  log.md
  projects/
  milestones/
  issues/
  areas/
  repositories/
  code/
  runs/
  references/
```

The current `.opensymphony/memory/` paths stay in place for this compatibility
slice. Issue capsules map to `issues/<issue>.md`, milestone nodes map to
`milestones/<slug>.md`, generated topic docs map to `areas/<slug>.md`, and
repository memory remains a facet under `repositories/` rather than the root
taxonomy.

Every parsed OKF concept requires YAML frontmatter with a non-empty `type` and a
contained bundle-relative Markdown path. Existing legacy top-level fields such
as `issue`, `milestone`, `linear_url`, `areas`, `repository`, `prs`,
`source_refs`, and `docs_sync` are preserved as data during parse/render. The
parser also projects those fields into `opensymphony` extension metadata:
visibility, concept kind, scope refs, source refs, and docs-sync state. Unknown
frontmatter is kept in the raw frontmatter map so future writers can round-trip
documents they do not fully understand. Writers emit canonical YAML and do not
preserve the original frontmatter field order or whitespace.

`opensymphony memory lint --okf [bundle-root]` validates an OKF bundle from the
CLI, and the memory MCP admin path accepts the equivalent `memory.lint` request
with `okf` plus `bundleRoot` arguments. A user-supplied bundle root is
canonicalized and must stay inside the repository root, matching the containment
policy used by other memory admin file arguments. When no bundle root is
provided, linting uses the configured memory root.

`opensymphony memory export-okf --visibility public|private [--output DIR]`
exports the configured memory root as a directory bundle. The output directory
defaults to `okf-export-{visibility}` under the repository root when omitted and
must be new or empty so stale private files cannot survive a public export.
Export writes into a repository-contained staging directory first, runs OKF lint
on the staged bundle, and only then promotes the completed bundle to the
requested output path. If final promotion fails, OpenSymphony preserves the
lint-clean staged bundle for recovery and restores the previous empty output
directory when possible. Public export skips private concepts and fails if any
remaining public concept still references private comments, private memory
paths, or private source snapshots. Private export can include private concepts
but still keeps normal OKF lint errors fatal except for visible links back into
the private memory store, which are expected in private round-trip bundles.

The public export redaction scan is deliberately narrow and explicit: it treats
`linear:comment:`, `.opensymphony/memory/issues`,
`.opensymphony/memory/source*`, `.opensymphony/memory/snapshot*`, and their
Windows-path variants as private material when they appear in exported public
concepts. The scan uses the same markdown-visible text extraction as private
memory link linting, so fenced code blocks, inline code spans, escaped text, and
HTML comments do not create public export false positives.

The memory MCP admin surface exposes the same export operation as
`memory.export_okf` with `visibility` (`public` or `private`) and optional
`output` arguments. It uses the same repository containment, staging, lint, and
public redaction checks as the CLI command.

`opensymphony memory import-okf <bundle-root> [--force]` validates an OKF
directory bundle, copies its Markdown concepts into the configured memory root
without rewriting frontmatter, and rebuilds the derived DuckDB catalog from the
imported bundle. The import source and target memory root are canonicalized,
checked against the repository containment policy, and rejected when they
overlap. Import preflights the full copy set before writing so predictable
target conflicts do not leave partially imported Markdown files. Existing
Markdown files are not overwritten unless `--force` is supplied. Because
`import-okf` restores both public and private bundles, visible private memory
links are allowed during import and preserved in the copied Markdown. Unknown
concept types, unknown frontmatter fields, missing optional fields, broken
links, and missing generated indexes are warning-level import inputs; malformed
concepts remain errors with file paths in the diagnostic.

Import is not transactional after the preflight succeeds. A filesystem write or
DuckDB reindex failure can leave already-copied Markdown files in the memory
root. Fix the underlying failure, inspect the partially copied files, and rerun
with `--force` only when replacing those files is intentional.

The memory MCP admin surface exposes the same import operation as
`memory.import_okf` with `bundleRoot` and optional `force` arguments. Prefer the
CLI or MCP admin tools for normal maintenance; direct file or DuckDB inspection
is an offline fallback for recovery and diagnostics only.

OKF lint diagnostics are intentionally actionable. Errors cover missing or
invalid concept frontmatter, missing `type`, malformed reserved files,
containment failures, and public-export leaks of private memory. Warnings cover
missing recommended fields, unknown types, broken Markdown links, wiki-only
links without Markdown equivalents, missing generated indexes, missing
citations for source-backed claims, and unknown OKF versions. Info diagnostics
call out synthesized title/description data, retained legacy fields, and
OpenSymphony extension metadata. Warning-level findings remain nonfatal;
private-data leakage and containment breakage are reported as errors.

Migration is intentionally incremental:

- Phase 1 enriches and parses existing documents as OKF concepts while keeping
  legacy paths and fields.
- Phase 2 can mirror or move documents into the final bundle layout and rebuild
  the catalog from OKF concepts.
- Phase 3 can expose graph, hosted import/export, and visibility-filtered APIs
  from the OKF-derived catalog.

The default visibility posture is private memory with optional public docs.
Private capsules may include Linear comments, review context, and source
snapshots, while public docs should contain public source refs such as issue
identifiers, PR URLs, and commit SHAs. Public docs must not link directly to
private capsule paths. Public memory is supported only by explicit
configuration, and generated indexes such as DuckDB should remain local unless a
project deliberately publishes them.

Generated memory `indexes/log.md` output groups entries under `## YYYY-MM-DD`
headings with newest dates first. The date comes from indexed completion time
when available, then capture time, and finally a stable ISO sentinel for
malformed legacy rows so regeneration is deterministic.

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
opensymphony memory export-okf --visibility public --output public-okf
opensymphony memory import-okf public-okf
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
`memory.capture`, `memory.sync_docs`, `memory.lint`, `memory.reindex`,
`memory.export_okf`, `memory.import_okf`, and `memory.ingest_code_intel`; these
require `OPENSYMPHONY_MEMORY_ADMIN_TOKEN` or `--admin-token` on
`opensymphony memory serve`. If an admin token is configured without a separate
read token, the admin token also protects read tools.
`memory.context` builds the agent kickoff bundle. Add `--include-code-intel`
to include available codebase-analysis artifacts alongside selected memory.
`opensymphony memory reindex --from-okf [bundle-root]` rebuilds the derived
DuckDB catalog from OKF concept documents, defaulting to the configured memory
root. Broken links and unknown concept types are indexed as warnings; malformed
OKF frontmatter remains fatal because those files are not parseable concepts.
The OKF rebuild clears derived GitHub metadata tables (`pull_requests`,
`changed_files`, `checks`, and `reviews`) because OKF concepts do not currently
carry that capture-enrichment data.

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

When managed local OpenHands is configured, the archive command also moves
matching conversations into the repo-scoped `archived/` store. It first tries
the issue workspace's `.opensymphony/conversation.json` manifest, then scans
managed conversation `meta.json` files for a `workspace.working_dir` ending in
the issue key so repo-scoped active conversations and legacy flat conversations
are covered by the same archive operation.

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
- COE-389: Current Gateway Inventory And Vocabulary
- COE-390: Gateway Schemas And Stream Feasibility
- COE-391: Gateway Module, Capabilities, And Dashboard Snapshot
- COE-392: Task Graph, Run Detail, File, And Diff Read APIs
- COE-393: Event Journal And Stream Broker
- COE-394: Frontend Workspace And Shared Schemas
- COE-395: Planning Artifact Schema And Session Service
- COE-396: Action Receipts And Initial Run Actions
- COE-397: Gateway API Client, Transport Adapters, And Reducers
- COE-398: Tauri Shell And Security Capabilities
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-400: OpenHands Event Normalization And Runtime Mirror
- COE-402: App Shell, Dashboard, Task Graph, And Run Views
- COE-403: Terminal And Log Renderer Prototype
- COE-404: Desktop Connection Profiles And Daemon Management
- COE-405: Linear Milestone, Issue, And Sub-Issue Mutations
- COE-406: Repository, Linear, And Research Analysis
- COE-407: Browser Transport And Remote Stream Protocols
- COE-408: Harness Adapter And Capability Model
- COE-409: Desktop Settings, Keychain, And Native Actions
- COE-410: Desktop Local Stream Optimization
- COE-411: Task Graph Editor And Runtime Overlay UI
- COE-412: Runtime Timeline And Terminal/Log Association
- COE-413: Implementation Plan Generator Stage
- COE-414: Diff, Validation, Approval, And Run Action Views
- COE-415: Milestone, Issue, And Sub-Issue Compiler
- COE-416: Dependency Graph And Plan Checks
- COE-417: Planning Workspace UI
- COE-419: Hosted Auth Placeholders And Web Parity
- COE-423: Model And Credential Settings
- COE-425: OpenHands Subscription Credential Adapter
- COE-426: Codex App-Server Prototype And Benchmarks
- COE-428: Model Configuration UI And Routing Metadata
- COE-429: Codex Approvals And Harness/Model Selection
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics
- COE-448: Multi-repo memory server and deterministic context
- COE-449: Desktop alpha recovery: replace stubs with functional app
- COE-452: DuckDB Prebuilt Developer Build Mode
- COE-453: Non-Interactive Init For Automation
- COE-473: Desktop task graph dependency and run detail parity
- COE-475: ChatGPT OAuth For Codex Harness
- COE-476: Codex Production Harness Enablement
- COE-478: Harden model profile storage and validation follow-ups
- COE-479: Codex Debug Session Resume
- COE-480: Run Detail Metrics And Density
- COE-481: Model Configuration Codex Subscription Follow-Up
- COE-482: TUI Codex Token Usage Accounting
- COE-483: Codex Event Content Summaries
- COE-484: Desktop Live Snapshot And Run Detail Refresh
- COE-485: Harden desktop live event resumption and refresh failure visibility

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
- COE-389
- COE-390
- COE-391
- COE-392
- COE-393
- COE-394
- COE-395
- COE-396
- COE-397
- COE-398
- COE-399
- COE-400
- COE-402
- COE-403
- COE-404
- COE-405
- COE-406
- COE-407
- COE-408
- COE-409
- COE-410
- COE-411
- COE-412
- COE-413
- COE-414
- COE-415
- COE-416
- COE-417
- COE-419
- COE-423
- COE-425
- COE-426
- COE-428
- COE-429
- COE-434
- COE-435
- COE-448
- COE-449
- COE-452
- COE-453
- COE-473
- COE-475
- COE-476
- COE-478
- COE-479
- COE-480
- COE-481
- COE-482
- COE-483
- COE-484
- COE-485

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
