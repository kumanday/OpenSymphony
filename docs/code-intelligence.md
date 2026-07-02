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

- COE-464 contributed: PR #165: Derive OKF memory graph metrics and communities (merge `7d58035`)
- COE-465 contributed: PR #166: Add shared frontend graph package (merge `21281d0`)
- COE-467 contributed: PR #171: Add Knowledge Graph renderer (merge `1337903`)
- COE-468 contributed: PR #169: Add Knowledge Graph inspector surface (merge `960541a`)
- COE-469 contributed: PR #170: Wire live memory graph privacy gates (merge `11ac876`)
- COE-471 contributed: PR #173: Harden graph scale and visual coverage (merge `97f8018`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

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

## Source refs

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

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
