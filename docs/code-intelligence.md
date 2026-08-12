# Code Intelligence

OpenSymphony code intelligence is local, read-only context for agents. The
persistent code graph provides bounded discovery over the target-branch
baseline and, for a run, its workspace overlay. Targeted current-file parsing
with pinned Tree-sitter grammars remains the revalidation source of truth. The
system extracts symbols, diagnostics, references, and source-cited spans, then
renders that evidence through `memory.context`, `code.graph.context`, and the
read-only `code.ast.*` MCP tools.

Code intelligence does not replace source inspection or tests. Current files,
repository docs, and test results remain authoritative.

## Configuration

Code intelligence is enabled by default when memory is configured:

```yaml
code_intel:
  enabled: true
  ast:
    enabled: true
    max_file_bytes: 2097152
    max_files_per_request: 200
    max_matches_per_request: 2000
    max_capture_bytes: 4096
```

The AST provider only loads built-in pinned grammar crates. Target repositories
cannot supply native grammar binaries.

## Provider Boundary

`opensymphony_code_intel` owns the rendered provider contract:
`CodeIntelProvider`, `CodeIntelArtifact`, `CodeIntelScope`, and
`CodeIntelSourceRef`. `opensymphony_memory` keeps its legacy `CodeIntelIndex`
and `CodeIntelArtifact` compatibility surface as an adapter around that contract
and converts provider errors into `MemoryError`, but AST and composite providers
do not import memory internals. The provider trait is `Send + Sync` for async
actor use. `CodebaseAnalyzer` implements the same provider trait as the
repository-summary fallback.

## Freshness

AST context is generated from the current worktree. Rendered artifacts include
content hash, parser version, query-pack version, path, and line range so agents
can tell what file state produced the context. Persisted code-intelligence rows
store metadata, hashes, spans, freshness, and snippet hashes; source snippets
are rendered from local files on demand.

There is no filesystem watcher in V1. Re-run `memory.context` after editing
source files.

In a central multi-repository catalog, persisted snapshots are addressed by
the configured canonical repository ID and commit. The memory server resolves
that ID to the registered checkout root; local filesystem paths are not
accepted as repository identity. A central catalog also rejects persistence
for repositories that are not present in the configured source registry.

## Target-branch repository snapshots

The Code Graph repository index is an explicit, server-side operation. Graph
MCP calls resolve enablement from the selected registered repository rather
than the catalog's default repository policy; `repo` and `repository` are
accepted as compatibility aliases but conflicting values are rejected before
scope authorization:

```text
POST /api/v1/code/repos/{repo_id}/index
code_index_repo(repoId)
```

The server resolves the configured repository root and target branch from the
target repository's `WORKFLOW.md`. It reads the selected commit through Git's
tree and blob objects, never accepts a client filesystem root, and never runs
target-repository code. The branch marker defaults to `develop` when it is not
present; `main`, `master`, and `origin/HEAD` are not implicit fallbacks.

Each completed run stores an immutable snapshot keyed by repository and commit,
including complete file membership. Parsed documents, symbols, edges,
diagnostics, and skipped-file coverage are persisted in bounded batches. When a
later target-branch commit exists, unchanged files reuse their prior snapshot
membership and only changed, added, or deleted paths are parsed or staled.
DuckDB writes are serialized through the repository index writer; reads remain
available while a gateway request performs the background job.

The gateway returns `accepted`, `progress`, `completed`, `unavailable`, or
`failed` reports and journals progress/failure events plus `code_graph_updated`
after completion. This shared target-branch baseline is not the live truth for
an issue workspace; workspace-specific code belongs to the overlay/composite
graph path.

The web and desktop Code Graph surfaces expose this operation for an empty
store. The configured repository is discoverable before the first index, so an
operator can choose `Index repository` without an admin MCP call. Progress
reports include parsed and persisted coverage; skipped files and diagnostics
remain visible in the empty-state panel; failures are retryable. After
`code_graph_updated`, the client reloads the repository summary and baseline
snapshot, retaining the selected target revision in the provenance strip. If an
accepted/progress job completes while the event stream is unavailable, the
client polls repository summaries and recognizes the indexed baseline directly.

## Workspace overlays

Run-scoped code reads compose the pinned target-branch merge-base snapshot with
the owning issue workspace. Git changes are enumerated as committed, staged,
unstaged, untracked, and deleted paths; renames are represented as a tombstone
plus an added path. Only changed supported files are parsed, and parsed
records are reused by content hash within the process. Unsupported, oversized,
failed, and limit-skipped paths remain in `unanalyzed_files` coverage.

The composite is an ephemeral projection: baseline rows are never mutated and
no fake worktree revision is persisted. The gateway pins a run's comparison
base and scopes the projection by repository, run, workspace, base revision,
and workspace content digest. It is therefore safe for concurrent workspaces
to edit the same path. The run endpoints are:

```text
GET /api/v1/runs/{run_id}/code/graph
GET /api/v1/runs/{run_id}/code/diff-overlay
```

Both reads rebuild from the recoverable workspace after a process restart;
workspace removal makes the projection unavailable. There is no continuous
watcher or second per-workspace DuckDB index.

## Agent Workflow

Use memory first, indexed discovery when the exact files are not known, and
live AST revalidation before edits:

```bash
opensymphony memory context --issue COE-123
# MCP: code.graph.context({repository, query|path|symbol, runId?, depth?, limit?})
opensymphony memory context --issue COE-123 \
  --paths crates/opensymphony-cli/src/memory.rs \
  --include-code-intel
```

Use `code.graph.context` to find likely symbols, callers, references, related
tests, and diagnostics without injecting the full repository graph into a
prompt. It returns bounded source citations and provenance for either the
indexed baseline or the supplied run's workspace overlay. The `runId` argument
is optional: omit it for a worker-granted baseline query, and supply it only
when the verified checkout overlay is needed. The server resolves
the repository and workspace; tool arguments cannot widen filesystem,
visibility, or snippet policies. Then read the cited files and run targeted
`memory.context --include-code-intel --paths ...` live scanning before changing
behavior and again after touched-file changes. Current source files and tests
remain authoritative over indexed evidence.

Worker grants with a nonempty checkout generation use that verified generation
for AST reads and run overlays. Generation-less legacy grants continue through
the registered repository source and legacy workspace path; they do not require
a checkout-generation manifest.

Generated, vendor, build, and cache directories such as `node_modules`,
`target`, `dist`, `build`, `.venv`, `.next`, `.turbo`, `vendor`, and
`generated` are skipped with trace warnings during directory traversal.
Explicitly requested files inside those directories are still parsed when they
stay inside the repo root and pass the configured limits. Oversized files are
skipped with trace warnings rather than failing the whole request.

## MCP Tools

When `code_intel.enabled` is true, `tools/list` exposes the read-only
`code.graph.context` indexed discovery tool. When `code_intel.ast.enabled` is
also true, it exposes these AST tools:

- `code.ast.status`
- `code.ast.outline`
- `code.ast.symbols`
- `code.ast.references`
- `code.ast.query`
- `code.ast.context`
- `code.ast.diagnostics`

`code.ast.query` accepts ad hoc Tree-sitter query text for local trusted use.
When an admin token is configured, it is admin-gated. All tools enforce the
configured file, match, and capture limits and run AST work off the async server
thread. `code.graph.context` is always read-only, bounded by depth and result
limits, returns parser/query-pack and freshness metadata with source
references, and never returns source snippets.

## Security

- Relative paths resolve under the configured repo root.
- Absolute paths and symlinks must canonicalize inside the repo root.
- Only built-in pinned grammars are trusted.
- The provider parses text and runs Tree-sitter queries only; it does not
  execute target-repo source, build scripts, package manager scripts, tests, or
  macros.
- Public docs should cite paths and line ranges, not private source snippets.

## Troubleshooting

- Empty AST output with a trace fallback usually means no paths were requested,
  the language is unsupported, or every requested file was skipped by limits.
- Parser diagnostics mean Tree-sitter recovered partial syntax; inspect the
  cited file before trusting symbol shape.
- Stale-looking context usually means the agent edited files after loading
  context. Re-run `memory.context --include-code-intel` for the touched paths.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-461 contributed: PR #164: Expose memory graph DTOs and endpoints (merge `762cec5`)
- COE-464 contributed: PR #165: Derive OKF memory graph metrics and communities (merge `7d58035`)
- COE-465 contributed: PR #166: Add shared frontend graph package (merge `21281d0`)
- COE-467 contributed: PR #171: Add Knowledge Graph renderer (merge `1337903`)
- COE-468 contributed: PR #169: Add Knowledge Graph inspector surface (merge `960541a`)
- COE-469 contributed: PR #170: Wire live memory graph privacy gates (merge `11ac876`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-461: Memory Graph DTOs And Gateway Endpoints
- COE-464: Graph Extraction, Metrics, And Community Pipeline
- COE-465: Shared Graph Frontend Package And Reducers
- COE-467: Three.js Graph Renderer And Worker Layouts
- COE-468: Concept Inspector, Search, Filters, And Accessibility Fallback
- COE-469: Live Memory Graph Integration And Privacy Gates
- COE-471: Graph Scale, Visual Regression, And Web/Desktop Hardening
- COE-498: Tree-sitter Provider Skeleton And Rust Parsing
- COE-499: Memory Context AST Provider Integration
- COE-500: Query Packs For Supported Agent Languages
- COE-501: Code Intelligence Persistence And Ingestion
- COE-502: Read-Only AST MCP And CLI Tools
- COE-503: Code Intelligence Performance Docs And Hardening
- COE-506: Invert CodeIntelIndex trait ownership after AST memory integration
- COE-507: Deduplicate query-pack assets for grammar variants
- COE-508: Cache code-intel parsers and compiled query packs
- COE-520: Route desktop Knowledge Graph through native gateway commands
- COE-521: Workflow Target Branch Model And Init Customization
- COE-522: Init Target Branch Prompt And Flag
- COE-523: Update Workflow Settings Mode
- COE-524: Template Docs And Settings Hardening
- COE-525: Desktop Installer Contract And Release Metadata
- COE-526: Desktop Release Bundle Pipeline
- COE-527: Source Build Fallback And Prerequisites
- COE-528: App Download Install And Launch Flow
- COE-529: Desktop Auto-Update Flow
- COE-530: Installer Docs And End-To-End Validation
- COE-531: Workspace Shell Graph Hero And Surface State
- COE-532: Symbol Identity Container Chain And Code Read Model
- COE-533: Code Graph DTOs Gateway Routes And Native Commands
- COE-534: Code Graph Frontend Surface Adapters And Inspector
- COE-535: Run Diff Symbol Navigation And Code Overlay
- COE-536: Cross Graph Code Memory And Work Chips
- COE-537: Code Graph Scale Accessibility And Parity Hardening
- COE-540: Canonical Codex Thread Reuse And Workspace Retention
- COE-541: Durable Codex Thread Archive And Debug Recovery
- COE-542: Target Branch Code Index And Revision Snapshots
- COE-543: Workspace Code Overlay And Composite Graph
- COE-544: Indexed Agent Code Context And Retrieval
- COE-545: Edge Delta And Module Topology Diff
- COE-546: Code Graph Bootstrap UX And End-To-End Validation
- COE-547: Central Multi-Repository Config And Safe Migration
- COE-548: Canonical Repository Binding And Task Propagation
- COE-549: Verified Checkouts Instructions And Harness Envelopes
- COE-550: Per-Instance Memory Catalog And Source Migration
- COE-551: Scoped Cross-Repository Memory And Leaf Overlays

## Source refs

- COE-461
- COE-464
- COE-465
- COE-467
- COE-468
- COE-469
- COE-471
- COE-498
- COE-499
- COE-500
- COE-501
- COE-502
- COE-503
- COE-506
- COE-507
- COE-508
- COE-520
- COE-521
- COE-522
- COE-523
- COE-524
- COE-525
- COE-526
- COE-527
- COE-528
- COE-529
- COE-530
- COE-531
- COE-532
- COE-533
- COE-534
- COE-535
- COE-536
- COE-537
- COE-540
- COE-541
- COE-542
- COE-543
- COE-544
- COE-545
- COE-546
- COE-547
- COE-548
- COE-549
- COE-550
- COE-551

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
