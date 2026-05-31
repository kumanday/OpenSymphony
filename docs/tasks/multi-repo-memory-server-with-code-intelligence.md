# Multi-Repo Memory Server And Code Intelligence Plan

## Summary

Revise the memory server design so **repository is a facet, not the top-level memory taxonomy**. The canonical scope is the OpenSymphony work graph: organization or local instance, project set, projects, milestones, issues, sub-issues, and only then repository checkouts attached to executable terminal work items.

Keep **MCP over Streamable HTTP** as the canonical agent/CLI protocol. It still fits local and hosted modes, works across future virtualized workspaces, and mirrors the way DeepWiki exposes codebase context through MCP. External references used for product-shape inspiration only: [MCP transports](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports), [DeepWiki](https://cognition.ai/blog/deepwiki), [DeepWiki MCP](https://cognition.ai/blog/deepwiki-mcp-server), [Codemaps](https://cognition.ai/blog/codemaps), [DuckDB concurrency](https://duckdb.org/docs/current/connect/concurrency.html).

## Domain Model

- Introduce a **Knowledge Scope** model:
  - `LocalInstance | Organization`
  - `ProjectSet`
  - `Project`
  - `Milestone`
  - `WorkItem` with kind `issue | sub_issue`
  - `Repository`
  - `CodePath`
  - `Area`
- A `Repository` is an execution/input facet. It is not the memory root.
- Terminal executable work items may have exactly one `execution_repo_id`; non-terminal issues and milestones may reference many repos for cross-cutting design, architecture, and planning memory.
- Every memory record stores `scope_refs[]`, not a single repo foreign key. Search defaults to the current `ProjectSet` and may be filtered or widened by project, milestone, work item, repo, path, area, source kind, and visibility.
- Existing issue capsules become `MemoryRecord { kind: issue_capsule, scope_refs, source_refs, visibility, body_ref, indexed_at }`.

## Storage And Retrieval Architecture

- Split memory into provider boundaries:
  - `MemoryCatalog`: relational metadata, scope refs, source refs, visibility, freshness, and audit state. DuckDB remains the initial local backend.
  - `DocumentStore`: Markdown capsules, generated docs, summaries, and private source snapshots. Local FS first; object storage later.
  - `LexicalIndex`: current text search over capsules/docs.
  - `VectorIndex`: optional provider, initially `Noop`; future Qdrant implementation.
  - `CodeIntelIndex`: provider interface for code graph, embeddings, wiki-style docs, symbols, call/dependency maps, and path-level summaries.
  - `FusionRetriever`: combines lexical memory, vector results, code graph hits, and code-intelligence summaries into ranked context.
- Move DuckDB migrations to startup/write paths only. Read APIs must not mutate `schema_version`.
- Keep direct file/DB mode only as offline admin fallback. Normal `opensymphony run` and worker usage goes through the memory server.

## Protocol And CLI

- MCP endpoint:
  - local: `http://127.0.0.1:<port>/mcp`
  - hosted: `/api/v1/memory/mcp`
- Read-only tools:
  - `memory.context({ work_item?, scope?, paths?, limit?, include_code_intel? })`
  - `memory.search({ query, scope?, filters?, limit? })`
  - `memory.related({ work_item?, paths?, area?, scope?, limit? })`
  - `memory.brief({ work_item })`
  - `memory.docs({ area, scope? })`
  - `memory.status({ scope?, filters? })`
  - Code-intelligence retrieval is exposed through `memory.context` rather than
    a separate CLI command, so agents keep one context-loading path.
- Admin tools require elevated token capabilities:
  - `memory.capture`
  - `memory.sync_docs`
  - `memory.lint`
  - `memory.reindex`
  - `memory.ingest_code_intel`
- CLI commands call the MCP server when `OPENSYMPHONY_MEMORY_ENDPOINT` is present; otherwise they use offline direct mode.
- Add CLI scope flags consistently: `--project-set`, `--project`, `--milestone`, `--issue`, `--repo`, `--area`, `--all-accessible`.
- Worker env injection provides endpoint, token, current project set, current work item, and execution repo. Agents should use `opensymphony memory ...`, not read `.opensymphony/memory` directly.

## Code Intelligence Integration

- Treat DeepWiki/Codemaps as inspiration for a clean-room OpenSymphony capability: generated architecture docs, code maps, source links, summaries, dependency/call graphs, and queryable code understanding.
- Add `CodeIntelProvider` with initial implementation wrapping the existing repository `CodebaseAnalyzer`.
- Future providers:
  - Qdrant-backed multi-vector embeddings.
  - Colleague code graph adapter.
  - Generated wiki/code-map artifacts.
  - Symbol/dependency/call graph providers.
- Store code-intelligence artifacts as first-class memory records with `repo_id`, `commit_sha`, `paths`, `symbols`, `generator_kind`, `artifact_kind`, `visibility`, and `freshness`.
- Retrieval must cite source refs: issue key, PR, doc path, repo, commit SHA, file path, symbol, or generated artifact id.
- Do not couple memory query APIs to DuckDB, Markdown, Qdrant, or a specific code graph schema.

## Workflow And Runtime Changes

- Update `WORKFLOW.md` template:
  - First implementation step: run `opensymphony memory context --issue {{ issue.identifier }}`.
  - After initial file discovery: run `opensymphony memory context --issue {{ issue.identifier }} --paths <paths> --include-code-intel`.
  - Treat memory as context, not authority over current code, tests, or specs.
- Update the project/workspace model so checkout selection happens only for terminal executable work items.
- Add memory server health to diagnostics/control plane.
- In hosted mode, tokens enforce scope and visibility; virtualized workspaces receive only read-only memory tokens unless explicitly elevated.

## Test Plan

- Multi-repo fixture: one project set, two projects, three repos, cross-cutting milestone memory, and terminal sub-issues each bound to one execution repo.
- Search tests prove default project-set search returns cross-repo memory and `--repo` acts only as a filter.
- Scheduling/config tests enforce one execution repo for terminal work items.
- MCP contract tests cover tools, resources, capability filtering, scope filtering, and stable error codes.
- Concurrency tests run parallel read clients while capture/reindex executes without DuckDB lock leakage.
- Code-intel provider tests cover current `CodebaseAnalyzer`, `memory.context`
  code-intelligence inclusion, provider fusion ordering, source citations,
  stale commit detection, and no-provider fallback.
- Security tests verify private memory is hidden across project/org boundaries and hosted tokens cannot widen scope beyond authorization.

## Assumptions

- V1 memory server supports local project sets with a single local instance identity; hosted organization/tenant identity is additive later.
- Qdrant and code graph integrations are planned extension providers, not mandatory for the first memory-server slice.
- Existing repo-local `.opensymphony/memory` data is migrated or registered as one source within the broader project-set catalog.
- MCP remains the agent/CLI protocol; REST may exist only for health, diagnostics, and gateway integration.
