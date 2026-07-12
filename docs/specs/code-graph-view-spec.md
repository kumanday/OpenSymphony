# Code Graph View Specification

Status: draft

Date: 2026-07-02

Source basis: `docs/specs/code-graph-brainstorming-report.md` (design handoff), `docs/specs/opensymphony_tree_sitter_ast_spec.md` (structural data source), `docs/specs/llm-wiki-graph-view-spec.md` (rendering and client architecture), `docs/specs/okf-memory-spec.md` (cross-graph scope and source refs), `docs/specs/desktop-run-detail-operations-spec.md` (Run Detail integration), and the current codebase verified on 2026-07-02.

Reader: an OpenSymphony engineer extending the shipped Knowledge Graph surface and the code-intelligence backend into a human-facing Code Graph.

Post-read action: implement the Code Graph as a third graph surface in the shared graph pane, entered primarily through queries and diffs, backed by `opensymphony_code_intel` data through stable gateway and native-command contracts. Task decomposition happens separately after this spec is reviewed.

## 1. Summary

The Code Graph is the human-facing view over the code-intelligence substrate that already ships in `crates/opensymphony-code-intel` and the `code_*` DuckDB tables. It renders tree-sitter-derived structure — files, symbols, and the `contains`/`imports`/`calls`/`references`/`tests` edges between them — inside the same graph surface, renderer, and inspector discipline that the Knowledge Graph established, mounted as the full-width hero of a reworked workspace layout (section 6.1).

It has two top-level modes:

- **Query mode** renders a scoped working set: a symbol's neighborhood, a file's containment tree, or the delta a run produced with its blast radius. It is the anticipated primary workflow because reviews start here: click a changed region in a run's Diff pane, land on the enclosing symbol's neighborhood, and see callers, callees, imports, tests, diagnostics, and cross-links into memory and the work graph.
- **Atlas mode** renders the whole indexed repository, community/directory-aggregated first with expand-on-demand. It is the landing view when the operator opens the Code Graph directly, and the surface for free-form structural exploration.

The modes are peers: they differ in how they are entered (section 6.3) and how much of the graph they load at once (section 6.5), not in rank.

The value proposition is answering recurring operator questions — *what does this change touch, what calls this, what tests cover this, what did we decide about this before* — not aesthetics or spatial memory. Confidence honesty is a core requirement: tree-sitter without type resolution produces mostly `syntactic` edges in dynamic languages, and every edge's confidence must be a visible channel, never overstated.

The viewer is a read-only client surface. It must not mutate orchestrator state, write memory, or bypass memory visibility rules.

## 2. Current Evidence

Verified against the working tree on 2026-07-02. The Code Graph is an extension of shipped systems, not a greenfield build.

Data substrate (shipped):

- `crates/opensymphony-code-intel` exists with `AstCodeIntelProvider` and `CompositeCodeIntelProvider`, pinned tree-sitter 0.26.x grammars, and per-language `.scm` query packs for Rust, TypeScript, TSX, JavaScript, JSX, and Python covering definitions, imports, calls, tests, docs, locals, injections, and diagnostics (coverage varies by language; Rust currently lacks `tests.scm`).
- DuckDB tables `code_documents`, `code_symbols`, `code_edges`, `code_diagnostics` exist in `crates/opensymphony-memory/src/index.rs` (`migrate_index`), with indexes on symbol name/path/kind, edge source/target, and diagnostic path. `persist_code_intel_documents` and freshness keys are implemented.
- `memory.ingest_code_intel` is implemented (admin MCP tool) with `persist`, `languages`, `symbols`, and `queryPacks` arguments, and writes `scope_refs`/`source_refs` on persisted records — the cross-graph junction data path exists.
- All seven read-only `code.ast.*` MCP tools are registered in `crates/opensymphony-cli/src/memory.rs`.

Rendering and client infrastructure (shipped, OSYM-820–826 complete):

- `@opensymphony/graph` (`packages/graph`) provides `GraphState`/`graphReducer`, mode/filter/search/selection/deep-link state, and transport-agnostic adapters: `createHttpGraphAdapter` (gateway/memory server), `createTauriNativeGraphAdapter`, `createFixtureGraphAdapter`, plus a worker-based `createGraphLayoutAdapter` with synchronous fallback.
- `packages/ui-core/src/knowledge-graph-renderer.ts` implements the Three.js (^0.171.0) renderer: 2.5D orthographic camera, instanced node geometry, batched edge line geometry, LOD labels (80-label budget), nearest-node picking, and a 2D-canvas fallback. Layouts are custom (force, hierarchical, radial neighborhood, timeline) with progressive community aggregation for large force layouts. The OSYM-821 dependency evaluation resolved to custom layout code plus server-computed communities; the Code Graph consumes this outcome and must not run a second evaluation.
- The left graph pane in `packages/ui-core/src/app-shell.ts` has a segmented `data-graph-view` toggle (`Task Graph` / `Knowledge Graph`) with `selectGraphPaneView()`; adding a third entry is a registration change. The workspace geometry around the graph pane is reworked by this spec (section 6.1).
- Desktop reads graph data through Tauri native commands (`memory_bundles`, `memory_graph`, `memory_concept_detail`, `memory_communities`, `memory_search` in `apps/desktop/src-tauri/src/commands.rs`), selected over loopback HTTP by `createDesktopGraphAdapter`.

Run Detail diff pipeline (shipped):

- `packages/gateway-schema/src/run.ts` defines `RunDetail` (branch_name, pr_url, conversation_id, issue identifiers), `ChangedFileEntry` (path, change_kind, lines added/removed), and `FileDiffPage`/`DiffHunk`/`DiffLine` with per-line types and line numbers.
- The gateway computes diffs from the run's git worktree against `git merge-base` with the default branch (`workspace_comparison_base`, `get_run_diffs` in `crates/opensymphony-gateway/src/lib.rs`), with cached-snapshot and synthetic fallbacks. `workspace_path` is resolved server-side and never sent to the client.
- The Diff pane (`packages/ui-core/src/diff.ts`) renders file rows and hunk lines as plain HTML with no token or symbol affordances today.

Load-bearing gaps this spec must close:

1. No `/api/v1/code/*` gateway routes or code-graph DTOs exist.
2. Persisted symbol row IDs hash `repo_id + path + content_sha256 + parser/query-pack versions + kind + name + exact span`, so every edit and line shift produces new IDs. A stable `symbol_key` tier is required before any graph identity, deep link, or diff overlay work (section 7.3).
3. Symbol container chains are not extracted (`container_id` absent from the crate's `SymbolRecord`), and `symbol_key` and neighborhood containment both depend on them.
4. Edge confidence is persisted as a string, not a shared enum, and edge targets are largely unresolved `target_hint`s; the read model must resolve them at snapshot-build time.

## 3. Goals

1. Give operators a navigable, query-scoped view of code structure for the repositories OpenSymphony works on, in the same pane and interaction grammar as the Task Graph and Knowledge Graph.
2. Make the run diff the primary entry: from any changed line in Run Detail to the enclosing symbol's neighborhood in two interactions or fewer.
3. Render an honest graph: edge confidence (`exact`/`syntactic`/`heuristic`) and record freshness (`current`/`stale`/`unknown`) are always visible channels and filter dimensions.
4. Compute and display a per-run diff overlay — symbols added/removed/modified plus inbound blast radius — as review-support information, with a summary strip in Run Detail.
5. Connect the three graphs: from a symbol to the issues that touched it (`scoped_to`) and the memory concepts that explain it (`source_supported_by`/`cites`), via the scope/source refs that `memory.ingest_code_intel` already writes.
6. Reuse the shipped `@opensymphony/graph` state machinery, renderer, worker layouts, and accessibility fallbacks; add presets and channels, not a new engine.
7. Keep web and Tauri desktop clients on the same versioned DTOs, with native-command parity for desktop reads.
8. Establish stable symbol identity (`symbol_key`) so deep links and overlays survive edits, as a compatible amendment to the tree-sitter AST implementation.

## 4. Non-Goals

Three framings were considered and rejected during design review. They must not reappear in implementation or follow-on tasks:

1. **Graph-as-spec / architecture conformance gates.** The Code Graph does not express or enforce layering rules, dependency policies, or spec-conformance checks. Topology facts (for example a new edge between previously unconnected communities) may be surfaced as information, never as pass/fail judgment.
2. **Pinned global layout as spatial memory.** Layout stability is a per-session, per-query nicety (deterministic seeding so a shared deep link reproduces the same picture). It is not a design pillar, and no cross-session position persistence is built.
3. **Realtime agent-attention visualization.** No live "watch the agent work" view in v1. The diff overlay is the tractable slice; its delta computation must stay independent of the git-diff trigger so a future live path can replay deltas, but the live path itself is out of scope.

Additional non-goals, inherited or scoped:

- No type checking, LSP replacement, or compiler-grade call graphs; the graph renders what the AST provider extracts, at the confidence it extracts it (AST spec section 4).
- No graph editing; no mutation of code, memory, or orchestrator state from the pane.
- No always-on filesystem watcher; indexing is on-demand batch mode (AST spec section 11.1).
- No new renderer, layout library, or community-detection dependency.
- No merged tri-graph mega-DTO; cross-graph relationships are delivered as lazily resolved reference chips on both sides.
- No sophisticated rename/move detection across revisions in v1 (section 7.3 defers it explicitly).
- No agent-facing surface changes: agents keep `memory.context` and `code.ast.*`; the graph adds no new agent tools in v1.

## 5. Users And Workflows

### 5.1 Operator reviewing a run (anchor)

The operator has Run Detail open on an issue and wants to answer:

- Which symbols did this run actually change, beyond the textual diff?
- What unchanged code calls into the changed symbols — what might behave differently?
- Which tests relate to the changed symbols, and did the run touch them?
- What did we previously decide or record about this area?

Flow: visible symbol affordance in the Diff pane → the graph hero switches to Code Graph Query mode (neighborhood of the enclosing symbol, diff overlay active) while Run Detail and the Inspector stay put → the detail surfaces show signature, freshness, diagnostics, issue and memory chips.

### 5.2 Human exploring a codebase

A newcomer or a lead wants to answer:

- What are the major structural regions of this repository?
- What is in this directory/module, and what does it depend on?
- Where are the tests for this subsystem?

Flow: Code Graph → Atlas mode (aggregated) → expand a community or directory → drill into File and Neighborhood views with the same inspector discipline as the Knowledge Graph.

### 5.3 Implementation agent supervisor

The supervisor wants to verify what context an agent had or would get:

- Which symbols and edges would `memory.context --include-code-intel` cite for these paths?
- Are the records fresh for the run's branch state, or stale?

The graph adds no agent surface, but it renders the same records the agent receives, so what the operator sees is what the agent was given.

## 6. Product Shape

### 6.1 Workspace layout prerequisite: graph as hero

The current dashboard stacks a Status card, a Connection card, and a Model Configuration section above a three-column workspace (graph pane / Run Detail / Inspector at roughly 48/26/26). Three columns are too cramped for each column's content, and a third graph surface would make that worse. This spec therefore depends on a workspace shell rework that all three graph surfaces benefit from:

- **Status strip.** The Status pane compacts into a one-to-two-row top-bar component between the `OpenSymphony Desktop` identifier (upper left) and the `Dashboard` / `Planning` tabs; the metric tiles (running, retry queue, token counters) become inline compact stats.
- **Event log mini-view.** The event ticks (`snapshot_published` and friends) become a compact two-line scrollable region to the left of the `Connected` badge in the upper right, expandable to a large modal for the full log.
- **Model Configuration** collapses to a gear icon in the top bar, expandable to a large modal.
- **Graph hero.** The graph surface becomes a full-width hero section directly under the top bar, with the `Task Graph` / `Knowledge Graph` / `Code Graph` segmented toggle in its toolbar.
- **Two lower columns.** Below the hero, two resizable columns replace the previous three, each half-width by default. Column content is registered per graph surface (6.4).

The shell rework is Phase 0 (section 15) and is deliberately independent of the Code Graph backend: it ships against the existing Task and Knowledge graphs before any code-graph data exists.

### 6.2 Mounting

The Code Graph is the third segmented-toggle entry in the hero toolbar: `Task Graph` / `Knowledge Graph` / `Code Graph` (`data-graph-view="code"`). This resolves Knowledge Graph spec Open Question 2: code-intelligence nodes get their own surface and are not mixed into the memory graph by default; the graphs connect through inspector chips and deep links instead.

Each surface retains its own mode, selection, and filter state for the session. Toggling away and back restores exactly where the operator left off — this is the primary back-navigation mechanism (11.2). Selecting nodes in the Code Graph must not disturb Run Detail or Inspector state (the OSYM-824 acceptance criterion, restated for the new geometry).

### 6.3 Modes

Two top-level modes. They are peers: Query mode is the anticipated primary workflow because reviews start from diffs; Atlas is the natural landing for direct exploration. Neither is subordinate — they differ in how they are entered and how much of the graph they load.

| Mode | Sub-view | Working set | Entered by |
| --- | --- | --- | --- |
| Atlas | — | Whole repo, community/directory-aggregated, expand-on-demand | Opening the Code Graph directly with no restorable prior state; repo picker; deep link |
| Query | Neighborhood | Symbol-centric N-hop view (default depth 1, max 2) | Diff-pane symbol affordance, search result, inspector chip, Atlas drill-in, deep link |
| Query | File | Containment tree of one file or directory plus its import boundary | File header action in the Diff pane, Atlas drill-in, deep link |
| Query | Diff | Neighborhood or File view with the run's delta overlay active (section 10) | Diff-pane navigation from a run, Run Detail summary strip, deep link |

Opening the Code Graph with restorable prior state resumes that state; otherwise it lands on Atlas for the active repo (or the repo picker when several are indexed). Entering through a diff affordance, search hit, or chip lands directly in the corresponding Query sub-view.

Tests emphasis is a filter preset available in every mode (highlights `tests` edges and test-kind symbols for the current selection), not a separate mode.

Mode names map onto the shared `GraphMode` machinery in `@opensymphony/graph`; the code surface registers `atlas`, `neighborhood`, `file`, and `diff` and reuses the existing layout-kind mapping (`graphLayoutKindForMode`) with the presets in section 9.2.

### 6.4 Lower-column content per surface

The two columns below the hero are registered per surface:

| Hero surface | Context | Left column | Right column |
| --- | --- | --- | --- |
| Task Graph | always | Run Detail | Inspector (Diff / Activity) |
| Code Graph | entered from a run's diff (Query/Diff) | Run Detail (unchanged) | Inspector (unchanged, Diff tab) |
| Code Graph | standalone (Atlas, or Query via search/chip/deep link) | Structure list: neighborhood or community members of the current selection; doubles as the accessibility list fallback (section 13) | Symbol/file detail: the inspector content of 11.3, with room for source-linked snippets |
| Knowledge Graph | standalone | Neighborhood node list (planned) | Concept content rendering (planned) |

The run-entered Code Graph row is the load-bearing one: when the operator activates a diff affordance, only the hero changes (Task Graph → Code Graph with the Query context applied); Run Detail and the Inspector keep their columns and state, so the operator reads the code neighborhood directly above the diff they came from. The Knowledge Graph row records the shell contract this spec introduces; iterating the KG columns themselves (capsule content rendering, memory-node coverage) is adjacent work outside this spec's scope.

### 6.5 Scope discipline

Rendered working sets stay small by construction: neighborhoods are tens-to-hundreds of nodes, File mode is bounded by file size, and Atlas loads aggregated because repository-scale code graphs are 10–100× larger than memory graphs — a scale posture (section 14), not a ranking of modes. Atlas expansion is incremental: expanding a community or directory issues a scoped follow-up request rather than ever rendering the raw full graph. Sourcetrail's destination-app failure mode is the cautionary tale the Query entry points avoid; the Code Graph is workflow-attached first, and Atlas gives it a first-class global map rather than a hairball.

### 6.6 Repository scope

v1 targets the single active workspace repository per run (the current schema has no multi-repo runs). Atlas mode operates on any repo present in `code_documents`, selected through a repo picker fed by the repos endpoint. Indexing is explicit: an unindexed repo shows an "Index repository" action that triggers batch ingest with progress, then a `code_graph_updated` event refreshes the pane. Run-scoped views index the touched files on demand against the run worktree.

## 7. Graph Model

Derived mechanically from the AST spec's data model. No parallel ontology.

### 7.1 Nodes

| Kind | Backing record | Notes |
| --- | --- | --- |
| `repository` | repo identity from ingest batches | Root scope; analog of `bundle` |
| `directory` | derived from `code_documents` paths | Containment only |
| `file` | `code_documents` row | Carries language, freshness, diagnostic counts, symbol count |
| `symbol` | `code_symbols` row | Sub-typed by `SymbolKind`; the primary node class |
| `community` | computed server-side | Overlay/aggregate, same treatment as KG communities; in code graphs, directory-seeded |

Diagnostics are **badges, not nodes**: `diagnostic_count` and max severity render as badges on file and symbol nodes, with the diagnostic list in the inspector. This keeps the ontology small and matches the warning-count treatment in the Knowledge Graph.

Symbol nodes carry: `symbol_key`, `symbol_id`, name, kind, language, path display, container chain, signature (when available), span (1-based lines), freshness, content/snippet hashes, parser and query-pack versions, diagnostic badge counts, and metrics (degree in/out, community id).

### 7.2 Edges

Directly from `CodeEdgeKind`: `contains`, `imports`, `exports`, `calls`, `references`, `implements`, `extends`, `tests`, `configures` (plus `unknown`). Every edge carries `confidence`:

- `exact` — target resolved to a symbol within the indexed set.
- `syntactic` — syntax indicates the relationship; target unresolved or ambiguous.
- `heuristic` — inferred from naming, path, or test convention.

Confidence is a required **visual channel** (solid = exact, dashed = syntactic, dotted/dim = heuristic) and a first-class filter dimension. Color is never the only signal (section 13). Edges whose target could not be resolved to any indexed symbol carry `unresolved: true` and either render as dim stubs toward a `target_hint` label or are hidden by filter — mirroring the Knowledge Graph's broken-link discipline. The read model performs best-effort target resolution at snapshot-build time (name + container + import context within the indexed set); promotion of `syntactic` edges to `exact` by a future type-resolution provider (LSP/SCIP-style) requires no schema change.

### 7.3 Two-tier symbol identity (v1-blocking)

The persisted `symbol_id` hashes content hash and exact span, so it changes on every edit and every line shift. That is correct for freshness and citation and wrong for graph identity: deep links would break and diff overlays would see every symbol as delete+add.

The spec therefore requires a two-tier identity, implemented as a compatible amendment to the code-intel persistence layer:

- `symbol_id` (existing) — exact, revision-bound row identity. Used for freshness, citation, and provenance. Unchanged.
- `symbol_key` (new) — stable logical identity: `sha256(repo_id + path + language + kind + container_chain + name)`, no span, no content hash, no commit. Duplicate keys within one document (overloads, same-name locals) get a deterministic ordinal suffix by document order (`…#2`). Stored as a new indexed column on `code_symbols` and carried on edges' resolved endpoints. Used for graph node identity, deep links, diff matching, and cross-graph refs.

Prerequisite: container-chain extraction. The extractor must populate `container_symbol_id` (enclosing symbol) during ingest so that `container_chain` (names root→leaf) is derivable. This is prerequisite work for both `symbol_key` and neighborhood containment rendering, and it lands in the code-intel crate, flagged as an amendment to the AST spec's implementation rather than graph-only work.

Rename/move detection across `symbol_key` boundaries (renamed symbol, moved file) is explicitly deferred: a rename reads as remove+add in v1. The two-tier split itself is v1-blocking because retrofitting identity is expensive; rename matching is additive later (same kind + container + high snippet similarity).

### 7.4 Cross-graph edges (the tri-graph junction)

Reuse the Knowledge Graph's existing edge kinds; invent nothing new:

- `scoped_to`: code node → work-graph node (issue/milestone/project), from `opensymphony.scope_refs` on ingested code-context memory records.
- `source_supported_by` / `cites`: memory concept → code symbol/path, from `source_refs` carrying repo, path, and symbol identity (AST spec section 10.4). Newly written code source refs carry the optional `repo_id` and stable `symbol_key` alongside their path/span identity; older refs remain valid without backfill.
- Run/diff linkage: run → touched files → contained symbols is derivable from the diff endpoints plus `contains`; no new edge kind.

Delivery: cross-graph relationships appear as **inspector chips on both sides**, resolved lazily through the existing memory endpoints — issue and memory-concept chips in the Code Graph inspector; code-symbol chips in the Knowledge Graph inspector for concepts whose source refs cite code. Chip activation goes through the shared code/memory deep-link handles, and a valid `opensymphony://code/...` markdown link follows the same code handle. They are not merged into a single graph DTO. The payoff query: select a symbol → see the issues that touched it and the memory capsules that explain it.

## 8. Data Contracts

Separate code-graph endpoints, sharing the memory-graph envelope conventions (`schema_version`, `cursor`, `generated_at`, `filters_applied`, server-side visibility filtering). Rationale: freshness semantics are content-hash-driven rather than capture-driven, scale differs by orders of magnitude, and the multi-repo memory plan explicitly warns against coupling memory query APIs to a specific code graph schema. DTOs live in a new `packages/gateway-schema/src/code_graph.ts`; field naming is snake_case, matching `memory_graph.ts`.

### 8.1 Repo list

```
GET /api/v1/code/repos
```

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "repos": [
    {
      "repo_id": "opensymphony",
      "display_root": "OpenSymphony",
      "languages": ["rust", "typescript"],
      "document_count": 1240,
      "symbol_count": 18420,
      "edge_count": 96110,
      "freshness": "current",
      "indexed_at": "2026-07-02T00:00:00Z"
    }
  ]
}
```

### 8.2 Graph snapshot

```
GET /api/v1/code/repos/{repo_id}/graph?mode=atlas&aggregate=directory
GET /api/v1/code/repos/{repo_id}/graph?mode=file&path=crates/opensymphony-cli/src/memory.rs
GET /api/v1/code/repos/{repo_id}/graph?mode=neighborhood&symbol_key=...&depth=1
```

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "repo_id": "opensymphony",
  "mode": "neighborhood",
  "cursor": {"sequence": 512, "partition": "code-graph:opensymphony"},
  "nodes": [
    {
      "id": "sym:...symbol_key...",
      "kind": "symbol",
      "symbol_kind": "function",
      "label": "run_context",
      "symbol_key": "...",
      "symbol_id": "...",
      "path_display": "crates/opensymphony-cli/src/memory.rs",
      "language": "rust",
      "container_chain": ["memory"],
      "signature": "async fn run_context(...)",
      "span": {"start_line": 954, "end_line": 990},
      "freshness": "current",
      "diagnostic_count": 0,
      "diagnostic_severity": null,
      "metrics": {"in_degree": 3, "out_degree": 5, "community_id": "dir:crates/opensymphony-cli"}
    }
  ],
  "edges": [
    {
      "id": "edge:...",
      "kind": "calls",
      "source_id": "sym:...",
      "target_id": "sym:...",
      "confidence": "syntactic",
      "unresolved": false,
      "target_hint": null
    }
  ],
  "communities": [],
  "truncation": {"nodes_dropped": 0, "edges_dropped": 0, "reason": null},
  "filters_applied": [],
  "generated_at": "2026-07-02T00:00:00Z"
}
```

Contract rules:

- Atlas responses default to aggregation (`aggregate=directory` or `community`); expanding an aggregate is a follow-up scoped request, not a bigger snapshot.
- Neighborhood responses are bounded (section 14); when bounds trim the result, `truncation` says so explicitly — silent truncation is prohibited.
- Every symbol node carries freshness; snapshots never silently mix stale and current records. Stale-by-default exclusion follows the AST spec's freshness policy, with `include_stale=true` opt-in that marks stale nodes visibly.

### 8.3 Symbol detail

```
GET /api/v1/code/repos/{repo_id}/symbols/{symbol_key}
```

Returns the full record for the inspector: identity (both tiers), signature, doc span, spans, container chain, freshness and provenance (content hash, parser version, query-pack version, indexed_at), diagnostics, edge summary (grouped by kind/confidence with counts), a source-linked snippet (subject to section 12 redaction), and cross-graph chips (`scoped_to` issues, citing memory concepts) resolved from memory records.

### 8.4 Run-scoped endpoints

The client never knows `workspace_path`; run-scoped endpoints resolve the run's worktree server-side, exactly as `get_run_diffs` does today.

```
GET /api/v1/runs/{run_id}/code/outline?file_path=packages/ui-core/src/diff.ts
```

Returns the ordered symbol list for one touched file, parsed from the run worktree (AST batch mode; content-hash cached): for each symbol `symbol_key`, name, kind, `span`, `selection_span`, container chain. This is the client-side symbol-at-click substrate (section 11.1). Shape follows `code.ast.outline` with `symbol_key` added.

```
GET /api/v1/runs/{run_id}/code/diff-overlay
```

Resolves the run worktree server-side to the same overlay DTO as the repo diff route (section 10.3): base is the run merge-base and head is the run worktree head. The client never receives the workspace root.

```
GET /api/v1/code/repos/{repo_id}/diff-overlay?base_revision=...&head_revision=...
```

Returns the graph-diff DTO for any two indexed revisions of the repo. This revision-pair contract is the canonical diff overlay shape; run-scoped requests are only resolvers in front of it.

```
POST /api/v1/code/repos/{repo_id}/index
```

Operator-triggered batch ingest (equivalent to admin `memory.ingest_code_intel` with `persist=true`), local-trusted by default and admin-gated in hosted mode. The initial gateway route may report the current read-model index state when no async indexer is configured; successful ingest/reindex emits `code_graph_updated` on completion.

### 8.5 Events

`code_graph_updated` mirrors `memory_graph_updated` through the gateway event journal envelope:

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "repo_id": "opensymphony",
  "head_revision": "head-rev",
  "cursor": {"sequence": 513, "partition": "code-graph:opensymphony"},
  "updated_at": "2026-07-02T00:01:00Z"
}
```

Fired after ingest/reindex completes and after a diff-overlay computation persists fresh rows. Cursors use the same strictly monotonic sequence semantics as memory-graph cursors. A future incremental watch path fires the same event; consumers do not change.

### 8.6 Tauri native commands

Desktop reads mirror the shipped memory-graph pattern (native commands wrapping the gateway bridge, selected by `createDesktopGraphAdapter`-style detection):

```
code_repos            -> CodeRepoList
code_graph            -> CodeGraphSnapshot   (repoId, mode, path?, symbolKey?, depth?, aggregate?)
code_symbol_detail    -> CodeSymbolDetail    (repoId, symbolKey)
run_code_outline      -> CodeFileOutline     (runId, filePath)
run_code_diff_overlay -> CodeDiffOverlay     (runId, repoId?)
code_index_repo       -> CodeIndexReport     (repoId)
```

Same DTOs, same visibility semantics; the web client uses the HTTP routes.

## 9. Rendering Architecture

### 9.1 Reuse, with deltas

The Code Graph reuses `@opensymphony/graph` state machinery and the ui-core Three.js renderer wholesale. Deltas, exhaustively:

1. A code surface registration: `CodeGraphAdapter` implementing the adapter interface against the section 8 contracts (HTTP, native, and fixture variants), plus code-specific filter state (language, symbol kind, edge kind, confidence, freshness, diagnostics, path prefix).
2. Node styling for code kinds: shape/color per `symbol_kind` class (container kinds vs callable kinds vs test kinds), file/directory/repository styling consistent with KG directory/bundle treatment.
3. Edge visual channels: kind → hue/arrowhead; confidence → line style (solid/dashed/dotted) and opacity. Edge-kind and confidence filters act as reducers before geometry build, because edge volume, not node volume, is the renderer risk.
4. Layout presets (9.2). No new layout engine, no new physics, no client-side community detection — code communities arrive server-computed (directory-seeded), exactly as KG communities arrive in snapshots.
5. Diff overlay styling (section 10.4).
6. Inspector sections for code records (11.3).

### 9.2 Layout presets

| Mode | Layout | Notes |
| --- | --- | --- |
| Atlas | Progressive community layout | Existing >400-node aggregation path, seeded by directory communities; deterministic seed per (repo_id, cursor) so shared links reproduce the picture |
| File | Hierarchical | Existing level-based layout; levels = repository → directory → file → symbol container chain |
| Neighborhood | Radial | Existing BFS ring layout centered on the focused symbol |
| Diff | Inherits Neighborhood/File | Overlay only changes styling, never layout |

### 9.3 Scale posture

Query-scoped rendering keeps working sets small: neighborhoods are tens-to-hundreds of nodes; File mode is bounded by file size; only Atlas approaches the full graph and it always arrives aggregated. Reference point: a Django-scale repository under comparable extraction is ≈49K nodes / 196K edges — the existing 500/5K/20K node tiers remain valid for rendered subgraphs, but fixtures must be edge-heavy (section 14) because code graphs carry 3–5× the edge:node ratio of memory graphs.

## 10. Diff Overlay

A review-support overlay computed per run against its base. Information, not judgment: no pass/fail gating, no layering rules, no conformance framing.

### 10.1 Delta classification

Computed server-side in the code-intel layer (it owns both revisions' records), exposed as a DTO so TUI, desktop, and web consume identical numbers:

- Base snapshot: symbols/edges indexed at the run's comparison base — the same `git merge-base` the diff pane already uses (`workspace_comparison_base`).
- Head snapshot: symbols/edges parsed from the run worktree (content-hash cached, batch mode).
- Per `symbol_key`: `added` (head only), `removed` (base only), `modified` (both, differing snippet hash). `moved` detection is deferred with rename detection (7.3).

### 10.2 Blast radius

Inbound `calls` and `references` edges into modified/removed symbols — unchanged code whose behavior may have changed, which the textual diff cannot show. Default 1 hop, optional 2; every entry tagged with edge confidence; computed by traversal over `code_edges` via the existing target index. Cheap graph traversal, no LLM.

### 10.3 Overlay DTO

```
GET /api/v1/code/repos/{repo_id}/diff-overlay?base_revision=...&head_revision=...
GET /api/v1/runs/{run_id}/code/diff-overlay
```

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "repo_id": "opensymphony",
  "base_revision": "9d64a69",
  "head_revision": "c2f78d1",
  "added_symbols": [
    {
      "symbol_key": "...",
      "status": "added",
      "after": {"symbol_id": "...", "kind": "function", "name": "newCommand", "path_display": "packages/ui-core/src/diff.ts", "container_chain": [], "span": {"start_line": 42, "start_col": 0, "end_line": 60, "end_col": 1}, "freshness": "current"}
    }
  ],
  "removed_symbols": [],
  "modified_symbols": [],
  "blast_radius": [{"symbol_key": "...", "inbound_count": 3, "outbound_count": 1}],
  "unanalyzed_files": ["assets/logo.svg"],
  "truncation": {"nodes_dropped": 0, "edges_dropped": 0, "reason": null},
  "generated_at": "2026-07-02T00:00:00Z"
}
```

`unanalyzed_files` keeps the overlay honest about coverage: files the diff touched that produced no symbols (unsupported language, oversized, generated-excluded) are listed, never silently omitted. The repo route accepts any indexed base/head revision pair; the run route resolves the run's merge-base/head server-side and returns the same DTO.

### 10.4 Rendering

- **Run Detail summary strip**: one line above the changed-file list — `5 symbols modified · +2 / −0 · blast radius 9 · 0 new diagnostics` — the recurring-value artifact. Clicking it opens the Code Graph in Diff mode. Degrades gracefully (strip absent) when code intel is disabled or the run has no worktree.
- **Graph overlay**: status coloring/badging on nodes (added/removed/modified), halo or badge on blast-radius nodes with distance and confidence, in Neighborhood and File views. Removed symbols render as ghosts from the base snapshot.
- The delta computation is independent of the git-diff trigger (input: two revision snapshots), so a future watch service can replay deltas without rework. Attaching overlay numbers to workpads or PR bodies is out of scope for this spec.

## 11. Interaction Requirements

### 11.1 Diff-pane symbol navigation (anchor flow)

1. Run Detail Diff pane renders per-file diffs (exists today).
2. When code intel is enabled and the file's language is supported, the client fetches the run-scoped outline for the selected file once (`/api/v1/runs/{run_id}/code/outline`) and resolves affordances locally: each rendered `DiffLine` with a new-file line number maps to the innermost symbol whose span contains it. Line-level containment is the honest v1 affordance — no token parsing in the client. Deletion-only regions map to the nearest containing symbol in the head outline, or the file node when none exists.
3. Affordance rendering — visible, compact, bounded:
   - A small graph glyph renders in the diff gutter at the first changed line of each distinct enclosing symbol region, so the operator can see the affordance exists without hovering, and the glyph count stays small (one per symbol region, not one per line).
   - Hovering any contained line highlights its symbol region, names the enclosing symbol (`fn renderFileDiff`), and exposes the same glyph on that line.
   - Right-click on a contained line offers `Open symbol in Code Graph` as a secondary path; the file header row gets an `Open file in Code Graph` action targeting File mode.
   - Glyphs are keyboard-focusable and labeled (section 13). When code intel is unavailable for the file, no glyphs render and the diff is unchanged.
4. Activating an affordance switches the hero surface to Code Graph → Query/Neighborhood centered on that `symbol_key`, with the run's diff overlay active, via the shared deep-link mechanism. Only the hero changes: Run Detail and the Inspector keep their columns and state (section 6.4), so the neighborhood renders directly above the diff it came from.
5. Freshness correctness: the outline is parsed from the same worktree state the diff pane shows. If only base-commit records are available (worktree gone, hosted mode), symbols render with explicit `stale` markers per the AST freshness policy — rendered, never hidden.

The server-side symbol-at-position endpoint (span-containment query over `code_symbols`) is the eventual precise path and is deferred; the outline contract makes it unnecessary for v1 because per-file symbol counts are small.

### 11.2 Navigation, search, deep links

- Click selects; double-click focuses neighborhood (re-centers, depth reset); keyboard arrows move between visible neighbors; Home/End in list fallback — all inherited behaviors.
- Search covers symbol names, file paths, and signatures within the active repo, backed by the existing name/path indexes; results open Neighborhood.
- Deep links extend the shipped app-history mechanism with a surface discriminator and code state: `{surface: "code", repo_id, mode, symbol_key?, path?, run_id?, depth, filters}`. A shared deep link with the same snapshot cursor reproduces the same layout (deterministic seed).
- History: mode/selection/filter changes push app history exactly as the Knowledge Graph does.
- Returning: each surface's session state persists (section 6.2), so clicking `Task Graph` restores the prior task selection exactly. When the Code Graph was entered from a run's diff, its toolbar additionally shows an entry-context chip (for example `from COE-505 · AGENTS.md`) that returns to the Task Graph surface in one click.

### 11.3 Inspector

Human-first sections, same discipline as KG frontmatter presentation:

1. Primary: name, kind chip, language, signature, container-chain breadcrumb (each element navigable).
2. Provenance: freshness badge, content hash, parser version, query-pack version, indexed_at.
3. Source: source-linked snippet (path:line, opens via existing capability-gated file-open on desktop), doc comment when extracted.
4. Relationships: edges grouped by kind with confidence styling and counts; each row navigates.
5. Diagnostics: list with severity, span, message.
6. Cross-graph: issue chips (`scoped_to`) and memory-concept chips (citing records), lazily resolved; each opens its home surface (work graph selection / Knowledge Graph concept).
7. Raw: the underlying record behind a toggle (KG raw-YAML analog).

### 11.4 Filters

Repo, language, symbol kind, edge kind, confidence, freshness, diagnostics presence, path prefix, community, and (in Diff mode) delta status. Filters are reducers applied before geometry build. The Tests preset is a saved filter combination (tests edges + test symbols emphasized).

## 12. Security And Privacy

The Code Graph enforces the same boundary discipline as memory retrieval, with code-specific rules:

- Code-graph records are private by default (AST spec section 13.4). Hosted tokens cannot widen scope through client filters; visibility filtering happens at the server boundary before DTO serialization, matching the memory-graph implementation.
- Path redaction: DTOs carry workspace-relative display paths only — the same rule the diff pane already follows. Absolute paths, `workspace_path`, and repo roots never reach the client; run-scoped endpoints resolve worktrees server-side.
- Snippets: rendered on demand from local files in local/desktop mode; hosted mode returns metadata, spans, and hashes without source excerpts unless repo policy allows (AST spec open decision 7 governs; default deny).
- No ad hoc query execution through graph endpoints; `code.ast.query` policy (local-trusted/admin) is unchanged and unexposed here.
- Index trigger (`POST /api/v1/code/repos/{repo_id}/index`) is local-trusted by default, admin-token-gated in hosted mode, mirroring `memory.ingest_code_intel` access.
- All parsing is read-only tree-sitter over text; no target-repo code execution, ever.
- Clipboard/copy and local-file-open actions require explicit user interaction and the desktop capability gate.

## 13. Accessibility

Parity with the Knowledge Graph requirements, code-flavored:

- Keyboard selection and navigation across visible nodes; predictable focus order between toolbar, canvas, inspector, and list fallback.
- Searchable list/table fallback for every mode, including Diff mode (delta table: symbol, status, path, blast-radius flag) — the summary-strip numbers must be reachable without the canvas. In the standalone Code Graph layout the structure-list column (section 6.4) is this fallback, permanently visible rather than toggled.
- Diff gutter glyphs are keyboard-focusable buttons whose accessible names include the target symbol (`Open renderFileDiff in Code Graph`).
- Screen-reader summaries announce mode, repo, selection, active filters, overlay status, and truncation.
- Confidence and delta status are never encoded by color alone: line style carries confidence; icons/badges carry delta status and diagnostics.
- Reduced-motion mode stops layout animation after initial stabilization.
- Inspector uses semantic HTML; snippet blocks are focusable and labeled with path and line range.

## 14. Performance Targets

Rendered-subgraph tiers reuse the KG targets; code-specific budgets added because edge volume is the risk:

- Neighborhood (≤500 nodes / ≤1,500 edges after filters): interactive within 500 ms after data load; snapshot request p95 under 300 ms warm.
- File mode: bounded by file size; typical file (≤200 symbols) renders within 500 ms.
- Atlas, aggregated: first paint under 2 s for a repo at the ≈50K-symbol / ≈200K-edge reference scale, because aggregation happens server-side and the client receives ≤2,000 rendered elements before expansion.
- Run-scoped outline: p95 under 2 s for a typical run (≤50 touched files, warm content-hash cache) — inherits AST spec section 18 targets.
- Diff overlay: p95 under 5 s cold for a typical run (base persisted, head parsed on demand); summary strip renders independently of the graph canvas.
- Selection → inspector under 100 ms for loaded records; lazy chips may resolve after.
- Scale fixtures: extend the 500/5K/20K node tiers with edge-heavy variants at 1:4 node:edge ratio (e.g., 5K/20K, 20K/80K) plus a generated atlas fixture at reference scale for aggregation tests. Any bound that trims a response reports itself via `truncation`.

## 15. Implementation Phases

Phases are dependency-ordered; each is sized to decompose into standard task packages during planning, which happens after this spec is reviewed. Section references identify the requirements each phase implements.

### Phase 0: Workspace Shell Layout (prerequisite)

- Status pane compaction into the top-bar status strip; event-log mini-view with a full-log modal; Model Configuration gear and modal (section 6.1).
- Full-width graph hero with the surface toggle in its toolbar; two resizable half-width lower columns with per-surface content registration (section 6.4); per-surface session-state persistence across toggles (section 6.2).
- Ships against the existing Task and Knowledge graphs with no code-graph data dependency.

### Phase 1: Symbol Identity And Code Graph Contracts

- Container-chain extraction in `opensymphony-code-intel` during ingest.
- `symbol_key` column and index on `code_symbols` with deterministic ordinal disambiguation — the compatible amendment to the tree-sitter AST implementation (sections 2, 7.3).
- Edge-target resolution at snapshot-build time and a shared edge-confidence enum at the DTO boundary (section 7.2).
- Neighborhood traversal and span-containment queries over the existing tables.
- `code_graph.ts` DTO module, `/api/v1/code/*` and run-scoped gateway routes, the `code_graph_updated` event, Tauri native-command mirrors, and boundary visibility/redaction (section 8).

Phases 2–4 depend on this phase, and on Phase 0 for anything user-facing; Phase 0 and Phase 1 are independent of each other and can proceed in parallel.

### Phase 2: Pane MVP With Atlas And Neighborhood

- Code surface in `@opensymphony/graph`: adapters (HTTP, native, fixture), code filter state, mode registration, deep-link extension, and fixtures (sections 6, 9.1, 11.2).
- Third `Code Graph` toggle entry; symbol/file/directory styling; confidence and edge-kind visual channels; layout presets; code inspector sections; list fallback (sections 6.1, 9, 11.3, 13).

### Phase 3: Run Integration

- Diff-pane symbol affordances: run-scoped outline consumption, line-containment resolution, gutter affordances, and navigation into Query/Neighborhood that preserves Run Detail and Inspector state (section 11.1).
- Server-side delta and blast-radius computation, the overlay DTO, the Run Detail summary strip, overlay styling in the graph, and the delta list fallback (section 10).

### Phase 4: Cross-Graph Junction And Hardening

- Cross-graph chips in both inspectors: issue and memory-concept chips in the Code Graph inspector, code chips in the Knowledge Graph inspector, `symbol_key` on newly written source refs (section 7.4).
- Edge-heavy scale fixtures, atlas aggregation tests, truncation reporting, accessibility parity, reduced motion, and visual regression checks for web and desktop (sections 13–14).

## 16. Test Plan

- Identity: `symbol_key` stability across content edits and line shifts; ordinal determinism for duplicates; container-chain extraction fixtures per language; `symbol_id` churn still tracked for freshness.
- Read model: neighborhood traversal correctness and bounds; edge-target resolution (exact vs syntactic vs unresolved) on fixtures; directory-community aggregation; stale exclusion and `include_stale` marking.
- Contracts: DTO schema-version checks; visibility filtering; path-redaction (no absolute paths in any response); truncation reporting; `code_graph_updated` cursor monotonicity; native-command/HTTP parity on identical fixtures.
- Diff overlay: added/removed/modified classification fixtures; blast-radius traversal with confidence tags; `unanalyzed_files` completeness; merge-base agreement with the diff pane; summary-strip numbers equal DTO numbers.
- Anchor flow: outline fetch and line-containment resolution unit tests; affordance activation → neighborhood deep link; Run Detail/Inspector state untouched (OSYM-824 regression); stale-marker rendering when worktree records are unavailable.
- Shell layout: hero and two-column registration per surface; status strip, event-log modal, and model-configuration modal render; per-surface state restoration on toggle round-trips (Task → Code → Task keeps the task selection; Code keeps mode/selection/filters); column resize persistence.
- Affordances: one gutter glyph per enclosing symbol region; keyboard focus and accessible names; right-click path; no glyphs when code intel is unavailable for the file.
- Rendering: confidence/kind channel styling snapshots; overlay styling; layout-preset determinism per seed; edge-filter reducer behavior; WebGL nonblank and canvas-fallback checks web + desktop.
- Accessibility: keyboard navigation, list fallback per mode including delta table, screen-reader summaries, non-color confidence encoding, reduced motion.
- Scale: fixture tiers of section 14 within budget; aggregated atlas at reference scale; no unaggregated full-graph render path reachable from Atlas entry.

## 17. Acceptance Criteria

- The `Code Graph` entry in the graph hero's toggle renders Atlas and Query modes in web and Tauri desktop through the same DTOs, with fixture, HTTP, and native adapters.
- The workspace shell rework ships: compacted top-bar status strip with an expandable event-log modal, model-configuration gear modal, full-width graph hero, and two resizable half-width columns with per-surface content registration.
- Opening the Code Graph directly lands on Atlas (or restores prior session state); toggling between graph surfaces restores each surface's prior mode, selection, and filters.
- Atlas opens aggregated for an indexed repo, expands on demand, and never renders an unaggregated full repository graph by default.
- From a run's Diff pane, a visible symbol affordance switches the hero to the enclosing symbol's neighborhood with the diff overlay active, in two interactions or fewer, while Run Detail and the Inspector keep their columns and state.
- Every rendered edge visibly encodes confidence; every rendered node visibly encodes freshness; both are filterable; color is never the only channel.
- The Run Detail summary strip shows symbols added/removed/modified, blast radius, and new diagnostics matching the overlay DTO, and degrades gracefully when code intel is unavailable.
- Deep links with `symbol_key` survive commits that shift lines but do not rename/move the symbol.
- Selecting a symbol shows issue and memory chips when ingested records reference it, and each chip navigates to its home surface.
- No response DTO contains an absolute path, `workspace_path`, or hosted-forbidden snippet; hosted visibility cannot be widened from the client.
- All graph reads are read-only; no orchestrator, memory, or code mutation is reachable from the pane.

## 18. Open Questions

1. Span-containment indexing: are the existing `code_symbols` path/name indexes sufficient for symbol-at-position and containment queries at reference scale, or is a composite `(repo_id, path, start_line, end_line)` index needed? Decide with benchmarks during the Phase 1 read-model work.
2. Rename/move detection (deferred from 7.3): when it lands, does matching (same kind + container + snippet similarity) live in the overlay computation or in ingest? Overlay-side is currently favored.
3. Hosted-mode snippet policy for code records: per-repo opt-in, or global deny until repo visibility semantics exist in hosted deployments?
4. Base-commit indexing strategy for overlays: index merge-base on demand per run (current plan) vs. a background policy that keeps default-branch snapshots warm. Revisit once overlay latency data exists.
5. Should Atlas offer an "index automatically on first open" convenience for the active workspace repo, or stay strictly explicit? Strictly explicit in v1; revisit with usage.
6. Multi-repo runs: the run schema is single-workspace today. When multi-repo lands, do run-scoped code endpoints take a repo discriminator, or does the run own an ordered repo list?
7. When a type-resolution provider (LSP/SCIP-style) later promotes `syntactic` edges to `exact`, should promoted edges be re-persisted or overlaid at read time? Schema supports either; decide with that provider's spec.
8. Standalone Code Graph lower columns: the structure-list / detail split (section 6.4) is the starting composition. After Phase 2 usage, revisit whether the left column should also offer community membership or recent overlay deltas touching the selection.

## 19. References

- `docs/specs/code-graph-brainstorming-report.md` — design handoff; sections 8–10 resolved here.
- `docs/specs/opensymphony_tree_sitter_ast_spec.md` — data model, freshness policy, persistence, security model.
- `docs/specs/llm-wiki-graph-view-spec.md` — rendering architecture, inspector/accessibility/scale discipline this spec extends.
- `docs/specs/okf-memory-spec.md` — scope refs and source refs used for cross-graph edges.
- `docs/specs/desktop-run-detail-operations-spec.md` — Run Detail and Inspector conventions.
- `docs/tasks/multi-repo-memory-server-with-code-intelligence.md` — provider framing; "do not couple memory query APIs to a specific code graph schema."
- External prior art (context, not adoption): codebase-memory-mcp (arXiv:2603.27277) for graph-mediated agent exploration and the Hybrid-LSP confidence-upgrade reference; Devin Desktop DeepWiki and Cognition Codemaps for the symbol-explanation and repository-map layers; graphify's EXTRACTED/INFERRED/AMBIGUOUS provenance tags for confidence-as-channel; Sourcetrail's discontinuation for the destination-app failure mode that motivates query-scoped, workflow-attached entry.
