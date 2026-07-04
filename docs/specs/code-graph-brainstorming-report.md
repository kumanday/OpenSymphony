# Code Graph Brainstorming Report

Status: brainstorming handoff, not a spec
Date: 2026-07-02
Target repository: `kumanday/OpenSymphony`
Reader: a Claude Code session with local codebase access, tasked with producing `docs/specs/code-graph-view-spec.md` and the corresponding `docs/tasks/` packages.
Post-read action: verify every grounding claim in section 9 against the local codebase, resolve the decisions in section 8, then draft the spec using the skeleton in section 10.

---

## 1. Purpose and scope of this document

This report consolidates a design conversation about adding a **Code Graph** to OpenSymphony: an interactive, Three.js-rendered view over tree-sitter-derived code structure, serving both human operators and (indirectly) agents, for web and Tauri desktop clients.

It exists to satisfy the "Definition of Ready" convention used in `docs/tasks/`: *hidden assumptions from prior discussion are written down*. Everything below is either (a) a claim grounded in the existing specs, (b) a design position with its rationale, or (c) an explicitly open decision for the spec author to resolve against the codebase.

Three positions from earlier drafts of this thinking were **rejected during review and must not reappear in the spec**:

1. **Graph-as-spec / architecture conformance gates.** OpenSymphony specs are prose documents plus task packages (Summary, Scope, Deliverables, Acceptance Criteria, Test Plan). They do not express graph-edge invariants, and the code graph must not be framed as a spec-conformance or layering-enforcement system.
2. **Pinned global layout as a "spatial memory" mechanism.** Humans will not memorize node positions across thousands of nodes and dozens of projects. Layout stability is at most a per-session/per-query nicety (deterministic seeding so a shared deep link reproduces the same picture), never a design pillar.
3. **Realtime agent-attention visualization as a v1 goal.** Deferred. The diff-scoped overlay (section 6) is the tractable, high-value slice of that idea.

Two positions were **affirmed and are load-bearing**:

1. **Query-scoped visualization is the primary interaction.** The default entry into the code graph is a symbol, a file, a diff, or a search — rendering the relevant subgraph. The full-repo atlas is one mode among several (valid for free-form exploration and as the marketing surface), not the default.
2. **Symbol navigation from the diff pane is the anchor use case.** Devin Desktop DeepWiki-style symbol comprehension, adapted to OpenSymphony's Run Detail: click a symbol in a run's diff, land on its neighborhood in the code graph, see callers, callees, imports, tests, diagnostics, and cross-links into memory and the task graph. Cognition's Codemaps (flow-oriented shareable maps) are the longer-horizon reference point.

---

## 2. What already exists (context inventory)

The code graph is **not a greenfield product**. It is the third graph in an ecosystem where most of the infrastructure is already specified or tasked. The spec must be written as an extension of these documents, not a parallel system.

### 2.1 `docs/specs/opensymphony_tree_sitter_ast_spec.md` (draft, 2026-06-26)

The structural data source. Key facts the code graph inherits:

- Module `opensymphony_code_intel` with `AstCodeIntelProvider` implementing/extending the existing `CodeIntelIndex` trait; `CompositeCodeIntelProvider` keeps `CodebaseAnalyzer` as fallback/summary provider.
- Read-only, agent-facing first slice: trusted built-in grammars (Rust, TypeScript, JavaScript, Python), versioned `.scm` query packs, extraction of symbols, references, imports, calls, tests, diagnostics.
- **Data model that the graph can render directly:**
  - `SourceIdentity`: repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_version, query_pack_version, indexed_at.
  - `SourceSpan`: byte + 1-based line/col ranges + snippet_sha256.
  - `SymbolRecord` with `SymbolKind` (module, class, struct, enum, trait, interface, type_alias, function, method, constructor, field, variable, constant, test, macro, route, unknown) and deterministic ID: `sha256(repo_id + commit_or_worktree + path + language + kind + name + selection_span)`.
  - `CodeEdge` with `CodeEdgeKind` (contains, imports, exports, calls, references, implements, extends, tests, configures, unknown) and `EdgeConfidence` (**exact / syntactic / heuristic**).
- DuckDB tables: `code_documents`, `code_symbols`, `code_edges`, `code_diagnostics`, with name/path/kind/source/target indexes.
- Freshness policy: a persisted record is current only if repo identity, path, content hash, parser version, query-pack version, and commit all match; otherwise stale and excluded by default.
- Agent surfaces: `memory.context --include-code-intel` (canonical) plus read-only MCP tools `code.ast.status|outline|symbols|references|query|context|diagnostics`.
- Explicit non-goals that also bound the graph: no full type checking, no complete semantic call graphs for dynamic languages, no LSP replacement, no persisting full parse trees.

**Gap the code graph fills:** this spec has *no visualization surface at all*. It produces exactly the node/edge/diagnostic/freshness records a graph view needs, but only renders them as markdown context for agents.

### 2.2 `docs/specs/llm-wiki-graph-view-spec.md` (Knowledge Graph, draft)

The rendering and client architecture the code graph should reuse:

- Shared transport-agnostic frontend graph package for web + Tauri (OSYM-822): reducers for bundle/mode/filter/search/selection/layout/deep-link state; adapters for gateway, memory server, Tauri native, fixtures.
- Three.js renderer (OSYM-823): 2.5D orthographic default, instanced node geometry, batched edge geometry, LOD labels, GPU-friendly picking, worker-based force/hierarchical/radial-neighborhood/timeline layouts. Explicit instruction to evaluate libraries rather than hand-rolling physics or community detection (OSYM-821).
- Versioned DTO boundary with `schema_version`, monotonic cursors, and a `memory_graph_updated` event contract (OSYM-820); visibility filtering and path redaction at the server boundary (OSYM-825).
- Inspector, filters, keyboard navigation, accessible list fallback (OSYM-824); scale/regression hardening at 500 / 5,000 / 20,000-node fixture tiers with aggregation defaults (OSYM-826).
- Mount point: resizable left navigation pane with a `Task Graph` / `Knowledge Graph` toggle; run-detail Inspector stays scoped to Diff and Activity.
- **Open Question #2 in that spec directly anticipates this work:** "Should code-intelligence nodes be enabled by default, or hidden until the user opts into code context?"

### 2.3 `docs/specs/okf-memory-spec.md`

The cross-graph junction. OKF concepts include `code-context` and `repository-memory-node` types, a `code/` directory in the bundle layout, and namespaced `opensymphony.scope_refs` (project, milestone, work_item, area, repository) and `opensymphony.source_refs` (linear_issue, github_pr, merge SHA, snapshots). The AST spec's `memory.ingest_code_intel` persists code-intelligence artifacts as memory records with those refs. This is the existing, specified mechanism by which code nodes, memory concepts, and Linear issues connect — the tri-graph junction already has a schema.

### 2.4 `docs/specs/desktop-run-detail-operations-spec.md` + task conventions

- Run Detail exposes per-file diff rows with addition/deletion stats, branch, PR URL, workspace_path, conversation_id; gateway schema lives in `packages/gateway-schema/`, UI in `packages/ui-core/`.
- Task packages follow a strict format (frontmatter: id/title/milestone/priority/estimate/blockedBy/blocks/areas/parent; Summary, Scope in/out, Deliverables, Acceptance Criteria, Test Plan, Context with explicit spec-section reading assignments, Definition of Ready, Notes). The code graph tasks must follow it and cite spec sections the way OSYM-821 cites "sections 6 and 9."

---

## 3. Product thesis

**The Code Graph is the human-facing view over `opensymphony_code_intel`, mounted in the same graph surface as the Task Graph and Knowledge Graph, entered primarily through queries and diffs rather than through a global map.**

Three consumers, one substrate:

| Consumer | Entry point | What they get |
| --- | --- | --- |
| Operator reviewing a run | Diff pane symbol click | Neighborhood of the touched symbol: callers, callees, imports, tests, diagnostics, prior memory, blast-radius overlay |
| Human exploring a codebase | Search / atlas mode | Community-aggregated overview, drill into files and symbols, same inspector discipline as the Knowledge Graph |
| Agent (indirect) | `memory.context`, `code.ast.*` MCP tools | Already specified in the AST spec; the graph adds no agent surface in v1, but shares the same data, so what the operator sees is what the agent was given |

The durable-value argument, post-correction: value comes from the graph answering **recurring operator questions** (what does this change touch, what calls this, what tests cover this, what did we decide about this before), not from aesthetics, spatial memory, or conformance enforcement. Novelty-driven features (full 3D perspective, global hairball) are retained but demoted to secondary modes — the Knowledge Graph spec already establishes this discipline (2.5D orthographic default, aggregation over hairballs, "dense operational workspace, not a landing page").

Alignment with the Frontier Code Intelligence architecture stack: the AST spec occupies the *structural layer*; the Code Graph occupies the *repository-map layer* (Codemaps analog) and the *symbol-explanation layer* (Devin Desktop DeepWiki analog) for OpenSymphony's own surfaces. CodeRAG, when it comes, slots into the retrieval-fusion order already defined in AST spec §12 — the graph does not need to anticipate it beyond keeping DTOs stable.

---

## 4. Graph model proposal

Derive the visual graph model mechanically from the AST spec's data model — do not invent a parallel ontology.

### 4.1 Nodes

| Kind | Backing record | Notes |
| --- | --- | --- |
| `repository` | repo identity from `SourceIdentity` | Root scope; analog of `bundle` in the KG |
| `directory` | derived from paths | Containment only; consistent with KG `directory` |
| `file` | `code_documents` row | Carries language, freshness, diagnostic counts |
| `symbol` | `code_symbols` row | Sub-typed by `SymbolKind`; the primary node class |
| `diagnostic` | `code_diagnostics` row | Probably an overlay/badge on file/symbol nodes rather than a first-class node — decision for the spec |
| `community` | computed | Same overlay treatment as KG communities |

### 4.2 Edges

Directly from `CodeEdgeKind`: `contains`, `imports`, `exports`, `calls`, `references`, `implements`, `extends`, `tests`, `configures`. Every edge carries `EdgeConfidence`, and **confidence must be a visual channel** (e.g., solid = exact, dashed = syntactic, dotted/dim = heuristic) and a filter dimension. This is the honest-rendering requirement: tree-sitter without type resolution produces mostly `syntactic` call edges in dynamic languages, and the UI must not overstate them. (External validation: graphify's EXTRACTED/INFERRED/AMBIGUOUS tags serve the same purpose; codebase-memory-mcp's Hybrid LSP exists precisely because naive tree-sitter call edges are weak — a future confidence upgrade path, not a v1 requirement.)

### 4.3 Cross-graph edges (the tri-graph junction)

Reuse the KG spec's existing edge kinds rather than inventing new ones:

- `scoped_to`: code node → work-graph node (issue/milestone/project), via `opensymphony.scope_refs` on ingested code-context records.
- `source_supported_by` / `cites`: memory concept → code symbol/path, via `source_refs` that carry repo + path + symbol id (AST spec §10.4).
- Run/diff linkage: run touches files → files contain symbols (derivable, no new edge kind needed).

The payoff query this enables, expressed in product terms: *select a symbol → see the issues that touched it and the memory capsules that explain it.* That traversal is the reason the three graphs are one system rather than three panes.

### 4.4 Node identity across revisions (open problem — must be addressed in the spec)

The AST spec's `symbol_id` hashes `commit_or_worktree` and `selection_span`, so **every commit and every line-shift produces new IDs**. Correct for freshness; wrong as a stable graph identity (deep links break, diff overlays see every symbol as delete+add). The spec needs a two-tier identity:

- `symbol_id` (existing): exact, revision-bound, used for freshness and citation.
- `symbol_key` (new): stable-ish logical identity, e.g. `hash(repo_id + path + language + kind + container_chain + name)` — no span, no commit. Used for graph node identity, deep links, and diff matching.
- Rename/move detection across `symbol_key` boundaries (name changed, file moved) is a fallback matching problem (same kind + container + high snippet similarity). Recommend explicitly deferring sophisticated rename detection to a later milestone, but the two-tier split itself is v1-blocking — retrofitting it is expensive.

---

## 5. The anchor interaction: diff-pane symbol navigation

The flow to specify end-to-end:

1. Operator opens Run Detail for an issue; Diff pane shows per-file changes (exists today per the desktop spec).
2. Symbols in the diff become affordances. Two implementation options:
   - (a) Server-side: a symbol-at-position endpoint (`repo, path, line[, col]` → containing `SymbolRecord`) backed by a span-containment query over `code_symbols`. Robust; needs worktree-fresh parsing of the run's branch state.
   - (b) Client-side: fetch `code.ast.outline` for the touched files once, resolve clicks locally against spans. Fewer round trips; outline DTO already specified in the AST spec.
   - Recommend (b) for v1 — the outline contract exists and per-file symbol counts are small — with (a) as the eventual precise path.
3. Click navigates the left-pane graph surface to **Code Graph → Neighborhood mode** centered on that symbol, via the same deep-link mechanism the KG spec requires (bundle/concept/mode/selection → repo/symbol_key/mode/selection).
4. Neighborhood view renders: the symbol, its container chain, direct `calls`/`references` in and out, `imports` it depends on, `tests` edges, diagnostics badges, and cross-graph chips in the inspector (issues via `scoped_to`, memory concepts via `source_supported_by`).
5. Inspector reuses the KG inspector discipline: human-first sections (signature, doc span, freshness, parser/query-pack provenance), source-linked snippet, raw record behind a toggle. Selection must preserve run-detail context (the OSYM-824 acceptance criterion about not disrupting Diff/Activity applies verbatim).

Freshness is the correctness constraint here: the diff pane shows the run's branch/worktree state, so the graph must resolve symbols against that same content hash — falling back to base-commit records with an explicit `stale` marker (the AST spec's freshness policy already defines the semantics; the graph must render them, not hide them).

---

## 6. The diff overlay (code graph delta, scoped down)

Earlier discussion inflated this into "architecture conformance." The retained, corrected version is a **review-support overlay**, computed per run/branch against its base:

- **Delta classification** per symbol_key: added, removed, modified (same key, different content hash), moved (deferred detection).
- **Blast radius**: inbound `calls`/`references` edges into modified symbols, one or two hops, tagged with edge confidence — i.e., *unchanged code whose behavior may have changed*, which is precisely what the textual diff cannot show and what the AST spec's data makes cheap (a graph traversal over `code_edges`, no LLM).
- **Rendering**: overlay badges/coloring in Neighborhood and File modes, plus a summary strip in Run Detail (n symbols changed, blast radius n, n new diagnostics). The 3D/2.5D view is the drill-down; the numbers are the recurring-value artifact.
- **Consumers**: operator review triage first; optionally, the same numbers can be attached to the issue workpad or PR body by existing workflow mechanisms later (out of scope for the graph spec itself).
- **Explicit non-goals**: no pass/fail gating, no layering rules, no cycle-policing framed as spec enforcement. If a topology fact is interesting (new dependency between previously unconnected communities), surface it as information, not judgment.

Note the sequencing property: an agent working is a stream of micro-diffs, and the AST spec §11.2 already defines incremental parsing for a future watch service. The diff overlay is therefore the stepping stone to any future live view — build the delta computation once, and "watch the run" becomes replaying deltas. Do not build the live view now; do keep the delta computation independent of the git-diff trigger.

---

## 7. Modes, mounting, and scale

**Modes** (parallel to the KG's Atlas/Bundle/Community/Neighborhood/Timeline/Evidence, but code-native):

| Mode | Purpose | Default? |
| --- | --- | --- |
| Neighborhood | Symbol-centric N-hop view; target of diff-pane navigation | Yes (query-scoped principle) |
| File/Module | Containment hierarchy of one file or directory | — |
| Diff | Delta overlay for a selected run/branch | Auto when entered from Run Detail |
| Tests | Emphasize `tests` edges and coverage relations for a selection | — |
| Atlas | Whole-repo, community-aggregated overview; free-form exploration and the demo/marketing surface | Only mode where the full graph loads, always aggregated first |

**Mounting**: extend the existing left-pane toggle to `Task Graph / Knowledge Graph / Code Graph` (resolving KG Open Question #2 as: code nodes get their own surface rather than polluting the memory graph by default; cross-graph edges appear in both inspectors as chips/links). User-facing copy: `Code Graph`.

**Scale**: code graphs are 10–100× larger than memory graphs (reference points from the ecosystem: Django ≈ 49K nodes / 196K edges under a comparable tree-sitter extraction). The KG's 500/5,000/20,000 fixture tiers still work because the query-scoped principle keeps *rendered* subgraphs small: neighborhoods are tens-to-hundreds of nodes; File mode is bounded by file size; only Atlas approaches the full graph and it must default to community aggregation (OSYM-826 discipline). Edge volume, not node volume, is the renderer risk — batched line geometry plus confidence/kind filters as first-class reducers. Reuse the OSYM-823 renderer and worker layouts wholesale; the code graph should add layout *presets* (e.g., containment-aware clustering by directory/module), not a new renderer.

**Events/freshness**: mirror `memory_graph_updated` with a `code_graph_updated` event carrying repo scope and cursor; initially fired on ingest/reindex, later by the incremental watch path.

---

## 8. Decisions the spec must make (with recommendations)

1. **Data path: extend memory graph DTOs vs. separate code graph endpoints.**
   Recommend **separate endpoints** (`GET /api/v1/code/repos`, `/repos/{id}/graph`, `/repos/{id}/symbols/{key}`, `/repos/{id}/diff-overlay?run=...`) sharing the KG's envelope conventions (schema_version, cursors, visibility filtering, path redaction rules adapted to code). Rationale: freshness semantics differ (content-hash-driven vs capture-driven), scale differs by orders of magnitude, and the AST spec keeps code intel as a provider behind memory rather than inside it — the API boundary should match. Cross-graph edges are delivered on both sides as reference chips resolved lazily, not as one merged mega-graph DTO.
2. **Symbol-at-click resolution**: client-side against fetched outlines (v1) vs server-side position endpoint (later). See §5.
3. **Two-tier symbol identity** (`symbol_id` + `symbol_key`): v1-blocking; see §4.4. This likely lands as an amendment to the AST spec rather than only in the graph spec — flag it there.
4. **Diagnostics as nodes vs badges**: recommend badges/overlay, keeping the node ontology small.
5. **Diff overlay computation location**: recommend server-side in `opensymphony_code_intel` (it owns both revisions' records), exposed as a DTO, so TUI/desktop/web all consume the same numbers.
6. **Which repo states are indexed for graph purposes**: base branch + active run worktrees, on demand, honoring the AST spec's batch mode; no always-on watcher in v1.
7. **Library selection** for layout/metrics/community detection: defer to the OSYM-821 dependency evaluation — the code graph must consume its output, not run a second evaluation.
8. **Confidence upgrade path**: note (non-blocking) that `EdgeConfidence` leaves room for a future type-resolution provider (LSP/SCIP-backed or Hybrid-LSP-style) to promote `syntactic` → `exact` without schema change. The AST spec's open decision 6 already gestures at this.

---

## 9. Grounding checklist for the Claude Code session

Verify each of these against the local codebase before drafting; the report was written from spec documents and may lag implementation:

- [ ] Current shape of `CodeIntelIndex` and `CodeIntelArtifact` in `crates/opensymphony-memory/src/lib.rs`; how far `opensymphony_code_intel` implementation has progressed vs. the draft spec.
- [ ] Whether OSYM-820–826 have landed: does a shared graph frontend package exist yet (name/path), what DTO shapes shipped, what renderer/layout/community libraries were selected in the OSYM-821 dependency evaluation.
- [ ] `packages/gateway-schema/src/run.ts`: what per-file diff data Run Detail actually exposes (paths? hunks? line ranges?), and where diff content is sourced (worktree vs git objects) — this determines symbol-at-click feasibility.
- [ ] Whether the left-nav `Task Graph / Knowledge Graph` toggle shipped and how modes/panes are registered, so `Code Graph` slots in as a third entry.
- [ ] DuckDB migrations: do `code_documents/code_symbols/code_edges/code_diagnostics` exist as specified; are there indexes suitable for span-containment queries (symbol-at-position) and inbound-edge traversal (blast radius).
- [ ] `memory.ingest_code_intel` status and whether scope_refs/source_refs on code records are being written — the cross-graph edges depend on it.
- [ ] Confirm milestone numbering and pick the task ID range (the reviewed plan uses OSYM-870 through OSYM-876 under "M12.9: Code Graph View").
- [ ] Check `docs/tasks/multi-repo-memory-server-with-code-intelligence.md` (referenced by the AST spec §20) for prior placeholder language to supersede or align with.

---

## 10. Suggested spec skeleton and task slices

**Spec** (`docs/specs/code-graph-view-spec.md`), mirroring the house style of the Knowledge Graph spec:

1. Summary · 2. Goals · 3. Non-Goals (include the three rejected framings from §1) · 4. Users and Workflows (operator-review-first) · 5. Product Shape (modes, third toggle, Run Detail integration) · 6. Graph Model (nodes, edges, confidence channel, two-tier identity, cross-graph edges) · 7. Data Contracts (endpoints, DTOs, `code_graph_updated`, freshness/stale rendering, redaction) · 8. Rendering Architecture (reuse deltas vs KG: presets, edge filters, scale posture) · 9. Diff Overlay · 10. Interaction Requirements (diff-pane navigation, deep links, inspector) · 11. Security and Privacy · 12. Accessibility · 13. Performance Targets (tiers + edge-count budgets) · 14. Implementation Phases · 15. Test Plan · 16. Acceptance Criteria · 17. Open Questions.

**Candidate task slices** (in the standard package format; dependency-ordered):

1. Code graph DTOs and gateway endpoints (blockedBy: AST spec persistence milestone; analog of OSYM-820).
2. Symbol identity: add `symbol_key`, deep-link contract, span-containment query support (amendment work touching `opensymphony_code_intel`).
3. Shared graph package: Code Graph mode reducers, adapters, fixtures (analog of OSYM-822 extension).
4. Renderer adaptation: neighborhood/file/atlas presets, confidence + edge-kind visual channels (analog of OSYM-823 extension).
5. Diff-pane symbol affordance in `ui-core` + navigation into Code Graph Neighborhood.
6. Diff overlay computation and Run Detail summary strip.
7. Cross-graph edges: scope_refs/source_refs chips in both inspectors.
8. Scale fixtures, visual regression, accessibility parity (analog of OSYM-826).

---

## 11. External reference points (for the spec's context section, not for adoption)

- **codebase-memory-mcp** (arXiv:2603.27277): validates graph-mediated code exploration for agents (83% answer quality, ~10× token reduction across 31 repos); ships a 3D graph UI and a "structural backend, agent is the intelligence" philosophy consistent with OpenSymphony's; its Hybrid LSP layer is the reference for a future confidence-upgrade provider.
- **Devin Desktop DeepWiki / Codemaps** (per the Frontier Code Intelligence article): the symbol-explanation and repository-map layers the Code Graph occupies for OpenSymphony surfaces.
- **graphify**: confidence provenance tags (EXTRACTED/INFERRED/AMBIGUOUS) as prior art for the confidence-as-visual-channel requirement; its 5K-node HTML ceiling as the cautionary tale the KG renderer architecture already avoids.
- **Sourcetrail (discontinued 2021)**: the destination-app failure mode; the reason query-scoped, workflow-attached entry points are the primary interaction.
