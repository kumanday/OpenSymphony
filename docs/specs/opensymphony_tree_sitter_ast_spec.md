# Tree-sitter Agent-Facing AST Parser for OpenSymphony Code Intelligence

Status: Draft specification  
Date: 2026-06-26  
Target repository: `kumanday/OpenSymphony`  
Target module: OpenSymphony code intelligence suite  
Primary consumer: OpenSymphony agents through memory and MCP context surfaces

## 1. Executive summary

OpenSymphony should integrate Tree-sitter as the structural parsing layer for its code intelligence suite. The integration should turn repository source files into queryable, source-cited, freshness-aware syntax artifacts that agents can use to navigate definitions, references, imports, calls, tests, and syntax boundaries during planning and implementation.

The best fit is an internal module tree named `opensymphony_code_intel`, compiled into the existing single `opensymphony` package. The module exposes an `AstCodeIntelProvider` that implements the code-intelligence-owned `CodeIntelProvider` contract. It replaces the current heuristic-only `CodebaseAnalyzer` path for `memory.context --include-code-intel`, while preserving compatibility by keeping the current analyzer as a fallback and repository-summary provider.

The first production slice should be read-only and agent-facing:

1. Parse source files with trusted, built-in Tree-sitter grammars.
2. Run versioned query packs to extract symbols, references, imports, tests, diagnostics, and local scopes.
3. Store lightweight structural artifacts in the memory catalog with repo, commit, path, language, parser version, query-pack version, content hash, source span, and freshness metadata.
4. Expose the results through the existing `memory.context` MCP and CLI path, plus optional read-only code-intelligence MCP tools for targeted AST exploration.
5. Preserve source evidence, traceability, and stale-index detection across local worktrees, branch switches, rebases, and completed issue memory.

This turns Tree-sitter into the concrete static-analysis grounding layer described by the Frontier Code Intelligence architecture stack: exact search and semantic retrieval find candidates, Tree-sitter supplies durable syntax handles, memory captures reasoning traces, and generated docs preserve stable subsystem knowledge.

## 2. Source review summary

### 2.1 Frontier Code Intelligence article

The article argues that modern coding assistants are becoming architecture intelligence systems. Its relevant product constraints are:

- Code intelligence must build repository models that index code, trace symbols, retrieve evidence, explain architecture, guide edits, and preserve development context.
- Agentic search is a multi-step reasoning process. A useful agent interprets the task, searches, reads files, follows definitions and tests, revises based on evidence, packs context, and records a trace.
- Retrieval needs several layers: exact search, semantic retrieval, symbol indexing, dependency expansion, reranking, context packing, and freshness logic.
- Static analysis grounds agent reasoning by identifying definitions, references, imports, exports, function and class boundaries, interface relationships, build entry points, test locations, and structural patterns.
- OpenSymphony memory is already positioned as a durable issue-capsule and docs-sync layer that stores completed work, source references, validation, review context, risks, and follow-ups.

Implication: Tree-sitter should not be a hidden parser utility. It should produce inspectable structural evidence that can be fused with exact search, semantic retrieval, generated documentation, and memory.

### 2.2 Tree-sitter docs

Tree-sitter is an incremental parser generator and parsing library. The docs emphasize four capabilities directly useful for OpenSymphony:

- It builds concrete syntax trees and can efficiently update them as source code changes.
- It is designed to be fast enough for editor-style parsing, robust in the presence of syntax errors, and embeddable through a dependency-free C runtime.
- Its syntax node API exposes byte ranges, row and column points, node types, named nodes, field names, parent and child traversal, and root nodes.
- Its query system uses S-expression patterns, captures, fields, operators, predicates, directives, immutable compiled queries, reusable query cursors, and byte or point range restrictions.

Implication: OpenSymphony can use Tree-sitter for fast, language-aware, range-cited structural extraction without waiting for a full compiler or language server pipeline.

### 2.3 OpenSymphony repository

The current OpenSymphony repository already has the right seams:

- Packaging is flat, but subsystem boundaries are explicit. The root crate includes internal module trees through `#[path = "../crates/.../src/lib.rs"]` declarations.
- `opensymphony_code_intel` owns `CodeIntelArtifact`, `CodeIntelProvider`, and rendered code-intelligence scope/source-reference types; `opensymphony_memory` consumes and may re-export them for compatibility.
- `opensymphony_cli` already supports `memory context --include-code-intel`.
- The memory MCP server already lists `memory.context`, `memory.search`, `memory.related`, `memory.docs`, and an admin `memory.ingest_code_intel` capability.
- The current `CodebaseAnalyzer` is useful for high-level repository summaries, packages, build systems, integration signals, conventions, and risks, but it does not parse ASTs, symbols, references, or syntax diagnostics.

Implication: Tree-sitter should be introduced as a provider upgrade behind existing code-intelligence and memory surfaces, not as a separate product lane.

## 3. Goals

### 3.1 Product goals

1. Give agents structural code awareness that can be inspected, cited, and audited.
2. Improve `memory.context --include-code-intel` from repository-level heuristics to path and symbol-level evidence.
3. Provide a stable foundation for definitions, references, imports, call sites, test discovery, structural search, syntax diagnostics, and source-bounded context packing.
4. Make static structure a first-class retrieval signal alongside lexical memory, future vector retrieval, generated docs, and issue capsules.
5. Keep agents grounded in current code by validating content hashes, commit SHAs, worktree dirty state, and query-pack versions before using structural artifacts.
6. Support local-first workflows with low setup burden and predictable resource use.

### 3.2 Engineering goals

1. Add a Rust module tree `crates/opensymphony-code-intel` and expose it through `src/lib.rs` as `opensymphony_code_intel`.
2. Keep parser loading trusted by default. Built-in grammar crates are allowed. Repo-supplied native parser code is disabled unless explicitly configured by an operator.
3. Preserve the current `memory.context` output contract while keeping rendered provider trait ownership in `opensymphony_code_intel`.
4. Preserve current CLI and MCP contracts where possible.
5. Add tests for query packs, source spans, freshness, concurrency, malformed code, generated files, and memory-context integration.
6. Keep the current `CodebaseAnalyzer` as a fallback, summary source, and planning-stage repository analyzer.

## 4. Non-goals for the first release

1. Full type checking.
2. Complete semantic call graphs for dynamic languages.
3. Cross-repository symbol resolution across all languages.
4. Arbitrary Tree-sitter grammar loading from target repos.
5. LSP replacement.
6. Compiler-grade refactoring support.
7. Persisting full parse trees to DuckDB.
8. Executing target-repo code.

## 5. Target architecture

### 5.1 Module placement

Add a new internal module tree:

```text
crates/opensymphony-code-intel/
├── src/
│   ├── lib.rs
│   ├── ast.rs
│   ├── cache.rs
│   ├── diagnostics.rs
│   ├── documents.rs
│   ├── edges.rs
│   ├── ingestion.rs
│   ├── languages.rs
│   ├── provider.rs
│   ├── query_pack.rs
│   ├── snippets.rs
│   ├── spans.rs
│   ├── symbols.rs
│   └── tests.rs
├── queries/
│   ├── rust/
│   │   ├── definitions.scm
│   │   ├── references.scm
│   │   ├── imports.scm
│   │   ├── calls.scm
│   │   ├── tests.scm
│   │   ├── docs.scm
│   │   └── diagnostics.scm
│   ├── typescript/
│   ├── javascript/
│   └── python/
└── fixtures/
```

Update `src/lib.rs`:

```rust
#[path = "../crates/opensymphony-code-intel/src/lib.rs"]
pub mod opensymphony_code_intel;
```

Update `Cargo.toml` dependencies with pinned versions selected during implementation:

```toml
[workspace.dependencies]
tree-sitter = "<pinned-current>"
tree-sitter-rust = "<pinned-current>"
tree-sitter-typescript = "<pinned-current>"
tree-sitter-javascript = "<pinned-current>"
tree-sitter-python = "<pinned-current>"
```

Implementation note: pin exact versions in `Cargo.lock` and prefer crates maintained by the upstream Tree-sitter organization or widely used official grammar repositories.

### 5.2 Provider model

The rendered context provider trait lives in `opensymphony_code_intel`. `opensymphony_memory` consumes and may re-export this contract for compatibility, but AST and composite providers do not import memory internals.

```rust
pub trait CodeIntelProvider {
    fn code_context(
        &self,
        paths: &[PathBuf],
        scope_refs: &[CodeIntelScope],
        limit: usize,
    ) -> Result<Vec<CodeIntelArtifact>, CodeIntelError>;
}
```

`AstCodeIntelProvider`, `CompositeCodeIntelProvider`, and the
`CodebaseAnalyzer` repository-summary fallback implement this trait:

```rust
impl CodeIntelProvider for AstCodeIntelProvider { /* ... */ }
impl CodeIntelProvider for CompositeCodeIntelProvider { /* ... */ }
impl CodeIntelProvider for CodebaseAnalyzer { /* ... */ }
```

### 5.3 Provider composition

Use provider composition rather than replacing all current analysis at once:

```text
CompositeCodeIntelProvider
├── AstCodeIntelProvider       -> symbols, spans, references, diagnostics, calls, tests
├── CodebaseAnalyzerProvider   -> package summaries, build systems, conventions, risks
└── NoopVectorProvider         -> current default until vector retrieval is added
```

The composite provider should merge artifacts by evidence type and score, then return a context pack ordered for agent use:

1. Direct AST evidence for requested paths and symbols.
2. Syntax diagnostics for requested paths.
3. Related definitions and call sites.
4. Related tests.
5. Package and convention summaries.
6. Prior memory references if the memory layer requests fusion.

## 6. Data model

### 6.1 Source identity

All AST-derived records must carry a stable source identity:

```rust
pub struct SourceIdentity {
    pub repo_id: String,
    pub repo_root: PathBuf,
    pub commit_sha: Option<String>,
    pub worktree_dirty: bool,
    pub path: PathBuf,
    pub language: LanguageId,
    pub content_sha256: String,
    pub parser_id: String,
    pub parser_version: String,
    pub query_pack_version: String,
    pub indexed_at: DateTime<Utc>,
}
```

Rules:

- `commit_sha` is the checked-out `HEAD` when available.
- `worktree_dirty` is true if the file content differs from `HEAD`, the repo has untracked included files, or `git status --porcelain` reports relevant changes.
- `content_sha256` is the primary freshness key for local worktree evidence.
- `query_pack_version` changes whenever query files or capture conventions change.

### 6.2 Source span

Agents should consume stable source spans instead of raw AST nodes:

```rust
pub struct SourceSpan {
    pub path: PathBuf,
    pub start_byte: u32,
    pub end_byte: u32,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub snippet_sha256: String,
}
```

Line numbers should be one-based in user and agent output. Tree-sitter row values are zero-based internally, so the adapter must add one when rendering spans.

### 6.3 Parsed document

```rust
pub struct ParsedDocumentSummary {
    pub identity: SourceIdentity,
    pub root_kind: String,
    pub named_node_count: usize,
    pub byte_len: usize,
    pub line_count: usize,
    pub diagnostics: Vec<AstDiagnostic>,
}
```

The `TSTree` itself should stay in the in-memory cache. It should not be serialized into DuckDB. Query-derived records are persisted.

### 6.4 Symbol record

```rust
pub struct SymbolRecord {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub language: LanguageId,
    pub span: SourceSpan,
    pub selection_span: SourceSpan,
    pub container_id: Option<SymbolId>,
    pub signature: Option<String>,
    pub doc_span: Option<SourceSpan>,
    pub visibility: Option<String>,
    pub source: SourceIdentity,
}
```

Canonical `SymbolKind` values:

- `module`
- `package`
- `class`
- `struct`
- `enum`
- `trait`
- `interface`
- `type_alias`
- `function`
- `method`
- `constructor`
- `field`
- `variable`
- `constant`
- `test`
- `macro`
- `route`
- `unknown`

`id` should be deterministic:

```text
symbol_id = sha256(repo_id + commit_or_worktree + path + language + kind + name + selection_span)
```

### 6.5 Edge record

```rust
pub struct CodeEdge {
    pub id: String,
    pub kind: CodeEdgeKind,
    pub source_symbol_id: Option<SymbolId>,
    pub target_symbol_id: Option<SymbolId>,
    pub source_span: SourceSpan,
    pub target_hint: Option<String>,
    pub confidence: EdgeConfidence,
    pub source: SourceIdentity,
}
```

Initial `CodeEdgeKind` values:

- `contains`
- `imports`
- `exports`
- `calls`
- `references`
- `implements`
- `extends`
- `tests`
- `configures`
- `unknown`

Confidence levels:

- `exact`: target symbol resolved inside current indexed set.
- `syntactic`: source syntax indicates a relationship, target resolution is unresolved.
- `heuristic`: inferred from file naming, path, or test convention.

### 6.6 Diagnostics

Tree-sitter can represent parse failures through `ERROR` and `MISSING` nodes. Capture both.

```rust
pub struct AstDiagnostic {
    pub kind: AstDiagnosticKind,
    pub span: SourceSpan,
    pub message: String,
    pub severity: DiagnosticSeverity,
}
```

`ERROR` nodes should be warnings. `MISSING` nodes should be warnings unless they occur inside an edited or generated file that is explicitly allowed to be partial.

### 6.7 Context bundle

```rust
pub struct AstContextBundle {
    pub request: AstContextRequest,
    pub summaries: Vec<ParsedDocumentSummary>,
    pub symbols: Vec<SymbolRecord>,
    pub references: Vec<ReferenceRecord>,
    pub edges: Vec<CodeEdge>,
    pub diagnostics: Vec<AstDiagnostic>,
    pub snippets: Vec<ContextSnippet>,
    pub trace: Vec<CodeIntelTraceStep>,
}
```

The trace should include parsed files, query packs used, range filters, truncation decisions, stale artifacts rejected, and fallback behavior.

## 7. Language registry

### 7.1 Initial languages

V1 should include:

1. Rust
2. TypeScript
3. TSX
4. JavaScript
5. JSX
6. Python
7. JSON, YAML, TOML, and Markdown as lightweight document/config languages

Rust and TypeScript should be the first full query-pack targets because OpenSymphony itself is Rust with TypeScript web and desktop surfaces.

### 7.2 Language descriptor

```rust
pub struct LanguageDescriptor {
    pub id: LanguageId,
    pub display_name: &'static str,
    pub file_extensions: &'static [&'static str],
    pub first_line_regex: Option<&'static str>,
    pub content_regex: Option<&'static str>,
    pub parser: fn() -> tree_sitter::Language,
    pub query_pack: QueryPackDescriptor,
    pub injections: Vec<InjectionRule>,
    pub max_file_bytes: usize,
}
```

Language detection order:

1. Explicit extension mapping.
2. Exact filenames such as `Cargo.toml`, `package.json`, `WORKFLOW.md`, `AGENTS.md`.
3. First-line hints where applicable.
4. Content regex tie-breaker.
5. Fallback to text summary provider.

### 7.3 Multi-language documents

Support embedded and injected languages through query packs and included ranges:

- TSX and JSX should parse JSX subtrees as part of the language grammar.
- Markdown code fences should create child parse jobs when the fence language is supported.
- HTML `<script>` and `<style>` blocks can be introduced after V1.
- Query captures should use `@injection.content` and `@injection.language` conventions where a grammar supports them.

## 8. Query packs

### 8.1 Query file conventions

Each language query pack should contain separate files by purpose:

```text
definitions.scm
references.scm
imports.scm
calls.scm
tests.scm
docs.scm
diagnostics.scm
locals.scm
injections.scm
```

Each pack also needs metadata:

```yaml
version: 1
language: rust
parser: tree-sitter-rust
captures:
  definition.function: Function or free function definition
  definition.struct: Struct definition
  reference.call: Function or method call expression
  import.path: Import path or module use path
  test.case: Test function or test block
limits:
  max_matches_per_request: 2000
  max_capture_bytes: 4096
```

### 8.2 Capture naming standard

Use dot-separated semantic captures:

```text
@definition.module
@definition.class
@definition.struct
@definition.enum
@definition.trait
@definition.interface
@definition.type
@definition.function
@definition.method
@definition.constructor
@definition.field
@definition.variable
@definition.constant
@definition.test
@reference.identifier
@reference.call
@reference.type
@import.source
@import.name
@export.name
@test.case
@test.subject
@doc.comment
@local.scope
@local.definition
@local.reference
@diagnostic.error
@diagnostic.missing
@injection.content
@injection.language
```

### 8.3 Query compilation

Compile query files once per process and cache immutable `TSQuery` values by:

```text
(language_id, parser_version, query_pack_version, query_name)
```

Use a query cursor per worker. Query cursors must not be shared concurrently.

### 8.4 Query range restriction

All agent-facing structural queries should accept byte or line ranges. Internally:

- Use Tree-sitter byte range filters when the caller supplies byte ranges.
- Convert line ranges to Tree-sitter point ranges when line ranges are supplied.
- Return matches that intersect the requested range by default.
- Add `containing_only=true` for exact bounded matches.

### 8.5 Query validation

Each query pack must have fixture tests:

```text
fixtures/rust/basic_symbols.rs
fixtures/rust/imports.rs
fixtures/rust/tests.rs
fixtures/rust/errors.rs
fixtures/typescript/react_component.tsx
fixtures/typescript/imports.ts
fixtures/python/basic_symbols.py
```

Test assertions:

- Expected captures exist.
- Source spans are stable and one-based in rendered output.
- `ERROR` and `MISSING` captures appear for malformed fixtures.
- Query packs fail fast on invalid node types or field names.
- Capture names conform to the standard.

## 9. Agent-facing interfaces

### 9.1 Keep `memory.context` as the canonical context path

The first agent instruction remains:

```bash
opensymphony memory context --issue COE-123
```

When the agent has paths or symbols, it should call:

```bash
opensymphony memory context \
  --issue COE-123 \
  --paths crates/opensymphony-cli/src/memory.rs \
  --include-code-intel
```

The resulting markdown should include a structural evidence section:

```markdown
## Code Intelligence

### AST summary: crates/opensymphony-cli/src/memory.rs

- Language: rust
- Commit: 9d64a690c454109a2de4e810a6b99c0443dfd43a
- Content hash: sha256:...
- Parser: tree-sitter-rust@...
- Query pack: rust@1
- Diagnostics: 0 ERROR, 0 MISSING

### Relevant symbols

- function `run_context` at crates/opensymphony-cli/src/memory.rs:954-990
  - calls `context_for_issue_with_options`
  - conditionally appends code intelligence when `include_code_intel` is true
- function `append_code_intel_context` at crates/opensymphony-cli/src/memory.rs:2211-2223
  - resolves repo path
  - builds scope refs
  - calls `CodebaseAnalyzer::code_context`

### Related tests

- crates/opensymphony-cli/tests/memory_server.rs:...

### Trace

- parsed 1 file
- ran definitions, references, calls, diagnostics
- rejected 0 stale artifacts
- truncated 0 matches
```

### 9.2 Add read-only AST MCP tools

Expose optional read-only tools through the same memory server when `code_intel.ast.enabled = true`.

Tool list:

```text
code.ast.status
code.ast.outline
code.ast.symbols
code.ast.references
code.ast.query
code.ast.context
code.ast.diagnostics
```

Rationale: agents keep `memory.context` as their main context-loading path, while advanced agentic search can inspect AST structure directly when a task requires targeted exploration.

### 9.3 MCP tool contracts

Response-level `limit` fields are request-wide caps across all returned files.

#### `code.ast.status`

Request:

```json
{
  "repo": ".",
  "languages": true
}
```

Response:

```json
{
  "provider": "tree-sitter-ast",
  "available": true,
  "languages": ["rust", "typescript", "javascript", "python"],
  "queryPackVersions": { "rust": 1, "typescript": 1 },
  "cache": { "documents": 128, "trees": 64 }
}
```

#### `code.ast.outline`

Request:

```json
{
  "repo": ".",
  "paths": ["crates/opensymphony-cli/src/memory.rs"],
  "includeDiagnostics": true,
  "limit": 200
}
```

Response:

```json
{
  "documents": [
    {
      "path": "crates/opensymphony-cli/src/memory.rs",
      "language": "rust",
      "contentSha256": "...",
      "symbols": [
        {
          "kind": "function",
          "name": "run_context",
          "span": { "startLine": 954, "endLine": 990 },
          "signature": "async fn run_context(...) -> Result<(), MemoryError>"
        }
      ],
      "diagnostics": []
    }
  ],
  "limit": 20,
  "trace": [
    "parsed 1 file(s)",
    "max files per request 200",
    "max matches per request 2000",
    "crates/opensymphony-cli/src/memory.rs lines 1-1400 parser tree-sitter-rust@... query-pack rust@1 content sha256:..."
  ]
}
```

#### `code.ast.symbols`

Request:

```json
{
  "repo": ".",
  "query": "run_context",
  "kinds": ["function", "method"],
  "paths": ["crates/opensymphony-cli/src"],
  "limit": 20
}
```

Response:

```json
{
  "symbols": [
    {
      "id": "sym_...",
      "kind": "function",
      "name": "run_context",
      "path": "crates/opensymphony-cli/src/memory.rs",
      "span": { "startLine": 954, "endLine": 990 },
      "selectionSpan": { "startLine": 954, "endLine": 954 },
      "source": {
        "contentSha256": "...",
        "parserVersion": "tree-sitter-rust:...",
        "queryPackVersion": "rust-query-pack-v2"
      }
    }
  ]
}
```

#### `code.ast.references`

Request:

```json
{
  "repo": ".",
  "symbol": "run_context",
  "paths": ["crates"],
  "limit": 50
}
```

Response:

```json
{
  "references": [
    {
      "kind": "reference.call",
      "path": "crates/opensymphony-cli/src/memory.rs",
      "span": { "startLine": 1258, "endLine": 1258 },
      "snippet": "run_context",
      "truncated": false,
      "source": {
        "contentSha256": "...",
        "parserVersion": "tree-sitter-rust:...",
        "queryPackVersion": "rust-query-pack-v2"
      }
    }
  ],
  "confidence": "syntactic",
  "limit": 50,
  "trace": [
    "parsed 1 file(s)",
    "max files per request 200",
    "max matches per request 2000",
    "crates/opensymphony-cli/src/memory.rs lines 1-1400 parser tree-sitter-rust@... query-pack rust@1 content sha256:..."
  ]
}
```

#### `code.ast.query`

Request:

```json
{
  "repo": ".",
  "language": "rust",
  "paths": ["crates/opensymphony-cli/src/memory.rs"],
  "query": "(function_item name: (identifier) @definition.function)",
  "limit": 100
}
```

Response:

```json
{
  "matches": [
    {
      "path": "crates/opensymphony-cli/src/memory.rs",
      "captures": [
        {
          "name": "definition.function",
          "text": "run_context",
          "span": { "startLine": 954, "endLine": 954 }
        }
      ],
      "source": {
        "contentSha256": "...",
        "parserVersion": "tree-sitter-rust:...",
        "queryPackVersion": "rust-query-pack-v2"
      }
    }
  ]
}
```

Security guard: `code.ast.query` only accepts query source when the caller is local or authenticated and when query execution limits are enforced. Hosted mode may disable ad hoc query execution and allow only named query packs.

#### `code.ast.context`

Request:

```json
{
  "repo": ".",
  "issue": "COE-123",
  "paths": ["crates/opensymphony-cli/src/memory.rs"],
  "symbols": ["function"],
  "includeCallers": true,
  "includeCallees": true,
  "includeTests": true,
  "limit": 40
}
```

The current MCP-backed implementation treats `symbols` as a symbol-kind filter
that reuses the same provider path as `memory.context --include-code-intel`.

Response:

```json
{
  "markdown": "## Structural Context\n...",
  "trace": [
    "parsed crates/opensymphony-cli/src/memory.rs",
    "ran rust definitions/references/calls/tests/diagnostics",
    "selected 8 symbols and 3 call edges"
  ]
}
```

#### `code.ast.diagnostics`

Request:

```json
{
  "repo": ".",
  "paths": ["crates/opensymphony-cli/src/memory.rs"],
  "limit": 50
}
```

Response:

```json
{
  "diagnostics": [
    {
      "path": "crates/opensymphony-cli/src/memory.rs",
      "kind": "error",
      "nodeKind": "ERROR",
      "span": { "startLine": 954, "endLine": 954 },
      "source": {
        "contentSha256": "...",
        "parserVersion": "tree-sitter-rust:...",
        "queryPackVersion": "rust-query-pack-v2"
      }
    }
  ],
  "limit": 50,
  "trace": [
    "parsed 1 file(s)",
    "max files per request 200",
    "max matches per request 2000"
  ]
}
```

Diagnostic `kind` values are the AST diagnostic vocabulary currently emitted by
the parser bridge: `error` and `missing`.

### 9.4 CLI surface

Minimum CLI changes:

```bash
opensymphony memory context --issue COE-123 --paths <paths> --include-code-intel
opensymphony memory serve --addr 127.0.0.1:8765
```

Optional operator/debug commands:

```bash
opensymphony code ast status
opensymphony code ast outline --paths crates/opensymphony-cli/src/memory.rs
opensymphony code ast query --language rust --paths crates/... --query-file query.scm
opensymphony code ast ingest --paths crates --persist
```

Agent workflow should prefer `memory.context`. Operator commands are for debugging, validation, and query-pack development.

## 10. Memory integration

### 10.1 Existing memory model extensions

Extend `MemoryRecordKind`:

```rust
pub enum MemoryRecordKind {
    IssueCapsule,
    TopicDoc,
    CodeContext,
    CodeSymbol,
    CodeEdge,
    CodeDiagnostic,
    RunSummary,
}
```

Current `CodeIntelArtifact` can be retained for rendered context, but persisted code intelligence needs structured rows.

### 10.2 DuckDB tables

Add migrations for derived code-intelligence tables:

```sql
CREATE TABLE IF NOT EXISTS code_documents (
    repo_id TEXT NOT NULL,
    commit_sha TEXT,
    worktree_dirty BOOLEAN NOT NULL,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    content_sha256 TEXT NOT NULL,
    parser_id TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    query_pack_version TEXT NOT NULL,
    byte_len BIGINT NOT NULL,
    line_count BIGINT NOT NULL,
    indexed_at TIMESTAMP NOT NULL,
    freshness TEXT NOT NULL,
    PRIMARY KEY (repo_id, path, content_sha256, query_pack_version)
);

CREATE TABLE IF NOT EXISTS code_symbols (
    symbol_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    container_symbol_id TEXT,
    signature TEXT,
    start_line BIGINT NOT NULL,
    start_col BIGINT NOT NULL,
    end_line BIGINT NOT NULL,
    end_col BIGINT NOT NULL,
    start_byte BIGINT NOT NULL,
    end_byte BIGINT NOT NULL,
    selection_start_line BIGINT NOT NULL,
    selection_end_line BIGINT NOT NULL,
    content_sha256 TEXT NOT NULL,
    snippet_sha256 TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    query_pack_version TEXT NOT NULL,
    indexed_at TIMESTAMP NOT NULL,
    freshness TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS code_edges (
    edge_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    edge_kind TEXT NOT NULL,
    source_symbol_id TEXT,
    target_symbol_id TEXT,
    target_hint TEXT,
    confidence TEXT NOT NULL,
    start_line BIGINT NOT NULL,
    end_line BIGINT NOT NULL,
    content_sha256 TEXT NOT NULL,
    query_pack_version TEXT NOT NULL,
    indexed_at TIMESTAMP NOT NULL,
    freshness TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS code_diagnostics (
    diagnostic_id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL,
    commit_sha TEXT,
    path TEXT NOT NULL,
    language TEXT NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT NOT NULL,
    start_line BIGINT NOT NULL,
    end_line BIGINT NOT NULL,
    content_sha256 TEXT NOT NULL,
    parser_version TEXT NOT NULL,
    query_pack_version TEXT NOT NULL,
    indexed_at TIMESTAMP NOT NULL,
    freshness TEXT NOT NULL
);
```

Indexes:

```sql
CREATE INDEX IF NOT EXISTS idx_code_symbols_name ON code_symbols(name);
CREATE INDEX IF NOT EXISTS idx_code_symbols_path ON code_symbols(path);
CREATE INDEX IF NOT EXISTS idx_code_symbols_kind ON code_symbols(kind);
CREATE INDEX IF NOT EXISTS idx_code_edges_source ON code_edges(source_symbol_id);
CREATE INDEX IF NOT EXISTS idx_code_edges_target ON code_edges(target_symbol_id);
CREATE INDEX IF NOT EXISTS idx_code_diagnostics_path ON code_diagnostics(path);
```

### 10.3 Freshness policy

A persisted code record is current only when all of these match:

- Same repo identity.
- Same file path.
- Same content hash.
- Same parser version.
- Same query-pack version.
- Commit SHA is same or record is explicitly marked worktree-current.

If any check fails, mark stale and avoid using it by default. If the agent asks for history or memory trace, stale artifacts may be returned with a clear `stale` marker.

### 10.4 Code-intelligence artifacts as memory records

When `memory.ingest_code_intel` runs with `persist=true`, create memory records with:

- `scope_refs`: project set, project, milestone, work item, repository, code path, area where available.
- `source_refs`: repo, commit SHA, path, symbol id, query pack id.
- `visibility`: private by default.
- `freshness`: current, stale, or unknown.
- `body_ref`: a generated markdown body for human and agent inspection.

The body should summarize symbols and edges with source spans. The DuckDB rows remain the structured query path.

## 11. Freshness and incremental parsing

### 11.1 Batch mode

For `memory.context` and `memory.ingest_code_intel`, batch mode is sufficient:

1. Resolve paths within repo boundary.
2. Compute content hashes.
3. Check cache and persisted records.
4. Parse changed or missing files.
5. Run query packs.
6. Return current artifacts.
7. Persist only when explicitly requested by an admin path or background indexing policy.

### 11.2 Incremental mode

Incremental mode becomes useful for a future watch service or desktop agent session:

1. Maintain a per-file cached `TSTree` keyed by path and content hash.
2. When an edit arrives, build a `TSInputEdit` from byte and point deltas.
3. Apply `ts_tree_edit` to the cached tree.
4. Reparse with the old tree to share unchanged structure.
5. Re-run only range-intersecting queries when possible.

### 11.3 Cache policy

Use two bounded LRU caches:

```text
DocumentTextCache: path + content hash -> Arc<str>
ParsedTreeCache: language + path + content hash + parser version -> TSTree
```

Policy:

- Default maximum parsed trees: 256.
- Default maximum document bytes: 128 MiB across cached documents.
- Single file parse limit: 2 MiB by default, configurable per language.
- Generated and vendor directories are skipped unless requested explicitly.

Default skipped directories:

```text
.git
node_modules
target
dist
build
.venv
__pycache__
coverage
.next
.turbo
```

### 11.4 Threading

Tree-sitter trees are cheap to copy for multi-threaded use, but individual tree instances should not be shared concurrently. The provider should:

- Use `spawn_blocking` for parse and query work from async paths.
- Copy `TSTree` before cross-thread use.
- Share compiled queries through immutable handles.
- Keep query cursors local to the worker.
- Bound parallelism to `min(num_cpus, configured_limit)`.

## 12. Retrieval and ranking

### 12.1 Retrieval fusion role

Tree-sitter structural evidence should be one retrieval signal. It should not replace exact search, semantic retrieval, or memory.

Suggested context assembly order:

1. Direct paths and symbols named by the issue, user, or previous agent trace.
2. Exact lexical hits for identifiers, config keys, errors, routes, and file names.
3. AST definitions and containing symbols for those hits.
4. AST edges: imports, references, calls, tests, and ownership boundaries.
5. Prior issue capsules and topic docs that cite the same paths or symbols.
6. Package and repository-level summaries.
7. Semantic retrieval and future multi-vector results.
8. Reranking and budget-aware context packing.

### 12.2 Scoring features

Each artifact gets a score from these features:

- Direct path match.
- Direct symbol name match.
- Identifier overlap with issue title and description.
- Capture kind priority, with definitions and tests ranked above incidental references.
- Recency and freshness.
- Existing memory links to the active issue, milestone, or area.
- Test coverage relationship.
- Proximity to files the agent already read.
- Diagnostic severity.

### 12.3 Context packing

For a single symbol, pack:

1. Symbol signature and doc comment.
2. Definition body, bounded by token budget.
3. Imports used by the definition.
4. Direct callers and callees, summarized when numerous.
5. Tests that reference or contain the symbol.
6. Relevant prior memory citations.
7. Diagnostics in the same file.

For a path, pack:

1. File outline.
2. Top-level symbols.
3. Relevant nested symbols.
4. Import/export summary.
5. Tests and diagnostics.
6. Package and convention summary.

## 13. Security model

### 13.1 Parser trust

Tree-sitter grammars are native code when compiled into Rust crates. Default policy:

- Only built-in, pinned grammar crates are loaded.
- Target repositories cannot provide arbitrary parser binaries.
- WASM grammar loading is deferred unless sandboxing is explicitly designed.
- Parser versions are audited through the normal dependency review process.

### 13.2 Path containment

Reuse existing repo-boundary path resolution behavior:

- Relative paths resolve under `MemoryConfig.repo_root`.
- Absolute paths must canonicalize inside the repo root or selected execution repo.
- Symlink traversal outside the repo is rejected.
- Hosted mode uses authorized workspace roots only.

### 13.3 Query safety

Risks: expensive ad hoc queries, enormous captures, and query packs that overmatch.

Controls:

- Maximum files per query.
- Maximum matches per file.
- Maximum captured text bytes.
- Timeout per query execution.
- Named query packs in hosted mode by default.
- Ad hoc query execution limited to local trusted mode or admin tokens.

### 13.4 Visibility

AST artifacts inherit memory visibility:

- Worktree-local AST artifacts are private by default.
- Generated public docs may cite paths and line ranges, but should not include private source excerpts unless the repo is public and policy allows it.
- Issue capsules can cite AST evidence with snippets because they are private memory by default.

### 13.5 No code execution

The provider parses text and runs Tree-sitter queries. It must not execute target-repo source, build scripts, package manager scripts, tests, or macros.

## 14. Configuration

Add `code_intel` to `config.yaml` or `opensymphony-memory.yaml`:

```yaml
code_intel:
  enabled: true
  ast:
    enabled: true
    provider: tree-sitter
    trusted_grammars_only: true
    persist_by_default: false
    max_file_bytes: 2097152
    max_files_per_request: 200
    max_matches_per_request: 2000
    max_capture_bytes: 4096
    languages:
      rust: true
      typescript: true
      javascript: true
      python: true
      markdown: true
      json: true
      yaml: true
      toml: true
    include_dirs:
      - crates
      - src
      - packages
      - apps
      - docs
    exclude_dirs:
      - .git
      - node_modules
      - target
      - dist
      - build
      - .venv
      - __pycache__
    generated_file_globs:
      - "**/*.generated.*"
      - "**/generated/**"
      - "**/schema.generated.*"
```

Environment overrides:

```text
OPENSYMPHONY_CODE_INTEL=1
OPENSYMPHONY_CODE_INTEL_AST=1
OPENSYMPHONY_CODE_INTEL_MAX_FILE_BYTES=2097152
OPENSYMPHONY_CODE_INTEL_ADHOC_QUERIES=0
```

## 15. OpenSymphony integration details

### 15.1 `opensymphony_memory`

Changes:

1. Extend memory record kinds for code symbols, edges, and diagnostics.
2. Add DuckDB migrations for code-intelligence tables.
3. Add freshness helpers for code records.
4. Consume/re-export `opensymphony_code_intel::CodeIntelProvider` and
   `CodeIntelArtifact` for compatibility instead of owning the provider trait.
5. Convert `CodeIntelError` into `MemoryError` only at memory integration
   boundaries.

### 15.2 `opensymphony_cli`

Changes:

1. Replace direct `CodebaseAnalyzer::new(repo_root).code_context(...)` with `CompositeCodeIntelProvider::new(config).code_context(...)`.
2. Preserve `memory context --include-code-intel` output shape, but enrich it with AST sections.
3. Expand `memory.ingest_code_intel` with `persist`, `languages`, `symbols`, and `queryPack` arguments.
4. Add code-intelligence MCP tools to `tools/list` when enabled.
5. Keep admin token requirements for ingestion.

### 15.3 `opensymphony_planning`

Changes:

1. Move or wrap `CodebaseAnalyzer` under the composite code-intelligence provider.
2. Use AST summaries for implementation plan generation where paths are known.
3. Use AST diagnostics as planning risks.
4. Preserve the existing high-level language, package, build-system, convention, integration-point, and risk analysis.

### 15.4 `opensymphony_gateway`

Optional read endpoints for rich clients:

```text
GET  /api/v1/code-intel/status
POST /api/v1/code-intel/outline
POST /api/v1/code-intel/symbols
POST /api/v1/code-intel/context
```

Gateway output should use the same schema as MCP responses.

### 15.5 `opensymphony_control`

Add code-intelligence status to diagnostics:

```json
{
  "codeIntel": {
    "enabled": true,
    "astProvider": "tree-sitter",
    "languages": ["rust", "typescript", "javascript", "python"],
    "cache": { "documents": 32, "trees": 24 },
    "lastIngestAt": "2026-06-26T00:00:00Z"
  }
}
```

## 16. Implementation plan

### Milestone 1: Provider skeleton and Rust parsing

Deliverables:

- `opensymphony_code_intel` module tree.
- Language registry with Rust support.
- Parse file API returning `ParsedDocumentSummary` and diagnostics.
- Rust definitions query pack.
- Unit tests for Rust fixture parsing, symbols, spans, and diagnostics.
- `src/lib.rs` export.
- Cargo dependency pins.

Acceptance criteria:

- `cargo test` passes.
- A Rust file can be parsed without executing repo code.
- Function, struct, enum, trait, impl, method, and test symbols are extracted from fixtures.
- Parse errors produce diagnostics rather than hard failures.

### Milestone 2: Memory context integration

Deliverables:

- `AstCodeIntelProvider` implements `CodeIntelProvider`.
- `CompositeCodeIntelProvider` uses AST first and `CodebaseAnalyzer` fallback.
- `opensymphony memory context --include-code-intel` prints structural evidence.
- Path containment and file-size limits are enforced.
- Current `memory.context` MCP path returns AST context when enabled.

Acceptance criteria:

- Existing memory context tests still pass.
- New tests verify AST context is included only when requested.
- Unsupported files fall back to repository-summary artifacts.
- Stale hashes are detected and re-parsed.

### Milestone 3: Query packs for TypeScript, JavaScript, Python, and diagnostics

Deliverables:

- Query packs for definitions, imports, calls, tests, docs, and diagnostics.
- Fixture tests for each language.
- Capture naming validation.
- Query-pack metadata loading.

Acceptance criteria:

- Supported source files produce symbols with one-based line spans.
- Import and call captures work for representative fixtures.
- Malformed code returns diagnostics.
- Query compilation errors fail tests before runtime.

### Milestone 4: Persistence and ingestion

Deliverables:

- DuckDB migrations for code documents, symbols, edges, and diagnostics.
- `memory.ingest_code_intel` supports `persist=true`.
- `memory.reindex` can rebuild code-intelligence derived tables.
- Freshness state is exposed in status and context.

Acceptance criteria:

- Ingesting a fixture repo writes structured rows.
- Changing a file marks old rows stale and writes current rows.
- Query-pack version changes invalidate derived rows.
- Admin token is required for remote ingestion.

### Milestone 5: Agentic AST tools and traces

Deliverables:

- `code.ast.status`, `code.ast.outline`, `code.ast.symbols`, `code.ast.references`, `code.ast.query`, `code.ast.context`, and `code.ast.diagnostics` MCP tools.
- Optional CLI debug commands.
- Trace rendering for parse, query, truncation, freshness, and fallback decisions.
- Tool-contract tests.

Acceptance criteria:

- Tools appear in `tools/list` only when enabled.
- Read-only tools work with read token.
- Ad hoc query execution respects local/admin policy.
- Agent traces cite paths, line ranges, parser versions, and query-pack versions.

### Milestone 6: Performance, concurrency, and docs

Deliverables:

- LRU caches.
- Spawn-blocking parse/query path.
- Parallel parse limits.
- Bench fixtures.
- Documentation under `docs/code-intelligence.md`.
- Workflow template update with AST-aware context guidance.

Acceptance criteria:

- Parallel `memory.context` calls do not corrupt parser or query state.
- Large files are skipped with clear warnings.
- Generated/vendor directories are excluded by default.
- Docs explain how agents and operators should use code intelligence.

## 17. Test plan

### 17.1 Unit tests

- Language detection by extension and exact filename.
- Parser initialization for each built-in language.
- Source span conversion from Tree-sitter points to one-based line output.
- Query-pack loading and validation.
- Definitions, references, imports, calls, tests, docs, locals, injections, and diagnostics captures.
- File-size and path-containment limits.
- Generated directory exclusion.

### 17.2 Integration tests

- `opensymphony memory context --include-code-intel` includes AST context for requested paths.
- `memory.context` MCP returns AST context with `includeCodeIntel=true`.
- `memory.ingest_code_intel` returns artifacts without persistence by default.
- `memory.ingest_code_intel` persists rows with admin token and `persist=true`.
- `memory.reindex` refreshes code-intelligence derived tables.
- `memory.status` shows code-intelligence health.

### 17.3 Freshness tests

- Same file and content hash reuses artifacts.
- Edited file re-parses and marks previous content hash stale.
- Query-pack version bump invalidates old artifacts.
- Parser version bump invalidates old artifacts.
- Branch switch produces new commit identity.
- Dirty worktree records are marked worktree-current, not commit-current.

### 17.4 Concurrency tests

- Parallel parse requests for distinct files.
- Parallel query requests over cached trees.
- Tree copies used across worker threads.
- Query cursors are not shared across threads.
- Cache eviction under load.

### 17.5 Security tests

- Absolute path outside repo is rejected.
- Symlink outside repo is rejected.
- Ad hoc query execution disabled in hosted/read-only mode.
- Oversized capture fails with controlled error.
- Oversized file produces skip warning.
- Repo-supplied native grammar is ignored by default.

### 17.6 Quality tests

- Agent context contains path and line citations for every symbol.
- Context trace records files parsed and queries run.
- Diagnostics are visible but do not block partial context.
- Fallback to `CodebaseAnalyzer` is visible in trace.
- Existing memory and planning tests remain compatible.

## 18. Performance targets

These are engineering targets for local mode and should be measured with repository fixtures:

- Parse a typical Rust or TypeScript file under 200 KB in under 100 ms p95 on a developer laptop.
- Build AST context for 10 requested files in under 2 seconds p95 when cache is warm.
- Keep `memory.context --include-code-intel` under 5 seconds p95 for 50 requested files with mixed cache state.
- Keep memory server responsive under concurrent read-only AST calls.
- Avoid persistent storage growth beyond query-derived rows and bounded snippets.

## 19. Failure handling

| Failure | Behavior |
|---|---|
| Unsupported language | Return fallback repository summary and a trace warning. |
| Parser initialization fails | Provider status becomes degraded, memory context continues without AST. |
| Query-pack compilation fails | Fail startup in strict mode, mark provider degraded in permissive mode. |
| Malformed source code | Return partial AST with diagnostics. |
| File too large | Skip file and return warning. |
| Stale persisted record | Reparse if file is available, otherwise return stale only when requested. |
| Path outside repo | Reject request. |
| Too many matches | Truncate with trace entry and deterministic ordering. |

## 20. Documentation updates

Add `docs/code-intelligence.md` with:

1. Conceptual model: exact search, memory, AST, symbols, and generated docs.
2. Agent workflow examples.
3. CLI examples.
4. MCP tool examples.
5. Configuration reference.
6. Freshness model.
7. Security model.
8. Query-pack development guide.
9. Troubleshooting.

Update `README.md` key features to include:

```markdown
- Tree-sitter code intelligence: local AST parsing, symbols, diagnostics, and source-cited structural context for agents.
```

Update `docs/tasks/multi-repo-memory-server-with-code-intelligence.md` to replace the generic symbol provider placeholder with the concrete Tree-sitter AST provider plan.

Update `WORKFLOW.md` template guidance:

```markdown
Before editing source files, load current memory context.
After initial file discovery, run memory context again with `--paths` and `--include-code-intel` so the agent receives AST-derived symbols, diagnostics, and related tests.
Treat memory and code intelligence as context. Current source files and tests remain authoritative.
```

## 21. Example rendered context

```markdown
## Code Intelligence

### AST document: crates/opensymphony-cli/src/memory.rs

- Language: rust
- Freshness: current worktree
- Parser: tree-sitter-rust@pinned
- Query pack: rust@1
- Diagnostics: none

### Symbols

1. `run_context` function
   - Source: crates/opensymphony-cli/src/memory.rs:954-990
   - Role: builds memory context and conditionally appends code-intelligence context
   - Calls: `context_for_issue_with_options`, `append_code_intel_context`

2. `append_code_intel_context` function
   - Source: crates/opensymphony-cli/src/memory.rs:2211-2223
   - Role: resolves repo, builds scope refs, asks the code-intelligence provider for artifacts

### Related edges

- `run_context` -> `append_code_intel_context` with confidence `syntactic`
- `append_code_intel_context` -> `CodeIntelProvider::code_context` with confidence `syntactic`

### Diagnostics

- No Tree-sitter `ERROR` or `MISSING` nodes found in requested ranges.

### Trace

- Parsed 1 Rust file.
- Ran definitions, calls, references, diagnostics.
- Used current worktree content hash.
- No stale artifacts used.
```

## 22. Open decisions

1. Whether ad hoc Tree-sitter queries should be available to read-token local agents or only to admin/operator tools.
2. Whether to support WASM grammar loading in hosted mode after a separate sandboxing review.
3. How aggressively to persist snippets. Default should be metadata and hashes, with snippets rendered on demand from local files.
4. Whether to use a filesystem watcher for local desktop mode or rely on lazy hash validation.
5. How much call-graph inference to expose before LSP or compiler-backed providers are added.
7. Whether generated public docs should include AST source snippets or only path and line references.

## 23. Acceptance criteria for V1

V1 is complete when:

1. `opensymphony memory context --include-code-intel --paths <path>` returns Tree-sitter-derived AST context for Rust and TypeScript paths.
2. Every AST artifact includes path, line range, content hash, parser version, query-pack version, and freshness.
3. Unsupported or malformed files degrade gracefully with diagnostics or fallback summaries.
4. Remote `memory.context` MCP supports AST context through `includeCodeIntel=true`.
5. `memory.ingest_code_intel` can generate artifacts and optionally persist code documents, symbols, edges, and diagnostics.
6. Query-pack tests cover Rust, TypeScript, JavaScript, and Python basics.
7. Security tests prove path containment, file-size limits, query limits, and trusted grammar policy.
8. Existing OpenSymphony memory, planning, and CLI tests continue to pass.
9. Documentation explains the agent workflow and operator configuration.

## 24. References reviewed

- Frontier Code Intelligence, Trilogy AI Center of Excellence, Leonardo Gonzalez, 2026-06-03: https://trilogyai.substack.com/p/frontier-code-intelligence
- Tree-sitter introduction: https://tree-sitter.github.io/tree-sitter/
- Tree-sitter using parsers guide: https://tree-sitter.github.io/tree-sitter/using-parsers/
- Tree-sitter basic parsing docs: https://tree-sitter.github.io/tree-sitter/using-parsers/2-basic-parsing.html
- Tree-sitter advanced parsing docs: https://tree-sitter.github.io/tree-sitter/using-parsers/3-advanced-parsing.html
- Tree-sitter query syntax docs: https://tree-sitter.github.io/tree-sitter/using-parsers/queries/1-syntax.html
- Tree-sitter query operators docs: https://tree-sitter.github.io/tree-sitter/using-parsers/queries/2-operators.html
- Tree-sitter predicates and directives docs: https://tree-sitter.github.io/tree-sitter/using-parsers/queries/3-predicates-and-directives.html
- Tree-sitter query API docs: https://tree-sitter.github.io/tree-sitter/using-parsers/queries/4-api.html
- Tree-sitter syntax highlighting and locals/injections docs: https://tree-sitter.github.io/tree-sitter/3-syntax-highlighting.html
- OpenSymphony repository: https://github.com/kumanday/OpenSymphony
- OpenSymphony multi-repo memory and code intelligence plan: docs/tasks/multi-repo-memory-server-with-code-intelligence.md
- OpenSymphony architecture: docs/architecture.md
- OpenSymphony memory module: crates/opensymphony-memory/src/lib.rs
- OpenSymphony current codebase analyzer: crates/opensymphony-planning/src/codebase.rs
- OpenSymphony memory CLI and MCP implementation: crates/opensymphony-cli/src/memory.rs
