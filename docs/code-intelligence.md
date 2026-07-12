# Code Intelligence

OpenSymphony code intelligence is local, read-only context for agents. It parses
current source files with pinned Tree-sitter grammars, extracts symbols,
diagnostics, references, and source-cited spans, then renders that evidence
through `memory.context` and the read-only `code.ast.*` MCP tools.

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

## Target-branch repository snapshots

The Code Graph repository index is an explicit, server-side operation:

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

## Agent Workflow

Use memory first, then code-intelligence context after file discovery:

```bash
opensymphony memory context --issue COE-123
opensymphony memory context --issue COE-123 \
  --paths crates/opensymphony-cli/src/memory.rs \
  --include-code-intel
```

Use the output to find likely symbols, diagnostics, and related tests. Then
read the cited files and run the relevant tests before changing behavior.

Generated, vendor, build, and cache directories such as `node_modules`,
`target`, `dist`, `build`, `.venv`, `.next`, `.turbo`, `vendor`, and
`generated` are skipped with trace warnings during directory traversal.
Explicitly requested files inside those directories are still parsed when they
stay inside the repo root and pass the configured limits. Oversized files are
skipped with trace warnings rather than failing the whole request.

## MCP Tools

When `code_intel.ast.enabled` is true, `tools/list` exposes these read-only
tools:

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
thread.

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

- COE-531 contributed: PR #209: feat(graph): add Code Graph frontend surface (merge `b6fcf57`)
- COE-532 contributed: PR #201: fix(orchestrator): stop reporting parked recovered issues as completed (merge `f6cddee`)
- COE-533 contributed: PR #209: feat(graph): add Code Graph frontend surface (merge `b6fcf57`)
- COE-534 contributed: PR #209: feat(graph): add Code Graph frontend surface (merge `b6fcf57`)
- COE-535 contributed: PR #210: feat(run-detail): add diff symbol navigation (merge `863e523`)
- COE-536 contributed: PR #211: feat(graph): connect cross-graph code chips (merge `f815ca2`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-531: Workspace Shell Graph Hero And Surface State
- COE-532: Symbol Identity Container Chain And Code Read Model
- COE-533: Code Graph DTOs Gateway Routes And Native Commands
- COE-534: Code Graph Frontend Surface Adapters And Inspector
- COE-535: Run Diff Symbol Navigation And Code Overlay
- COE-536: Cross Graph Code Memory And Work Chips
- COE-537: Code Graph Scale Accessibility And Parity Hardening
- COE-540: Canonical Codex Thread Reuse And Workspace Retention
- COE-541: Durable Codex Thread Archive And Debug Recovery

## Source refs

- COE-531
- COE-532
- COE-533
- COE-534
- COE-535
- COE-536
- COE-537
- COE-540
- COE-541

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
