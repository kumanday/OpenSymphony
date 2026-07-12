---
type: topic-doc
area: graph-view
visibility: public
last_memory_sync: 2026-07-02T03:46:15.373470+00:00
---

# Graph View

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-541 contributed: PR #207: feat(codex): archive and recover canonical threads (merge `c2723c2`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-541: Durable Codex Thread Archive And Debug Recovery

## Source refs

- COE-541

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->
## Graph visualization workbench

Graph rendering work needs dense, stable data long before a live daemon has
any. The workbench provides that as code, so every iteration, screenshot, and
test sees the same graph:

- `packages/graph/src/viz-fixture.ts` — deterministic knowledge-graph
  snapshot (~100 nodes, 7 `area:*` communities, concepts that belong to more
  than one area, mixed node kinds and degrees) built from a seeded PRNG.
- `packages/api-client/src/graph-viz-demo.ts` — matching task-graph demo
  (`graphVizDemoTaskGraph`) whose dependency shapes stress the arrow
  routing: several skip-level dependencies fanning out from one blocker plus
  overlapping skips from different blockers, and a backlog tier (VIZ-114…120)
  with cross-pane blockers and multi-hop chains that exercise ancestry
  critical-path highlighting. `createGraphVizDemoTransport()` wraps it all in
  a `MockGatewayTransport`.
- `graphVizFixtureCompletedTasks` (in `viz-fixture.ts`) — 31 completed tasks
  with PR evidence (including abandoned unmerged PRs) whose capsule ids
  resolve to real fixture concepts, feeding the Completed pane's table,
  search, sorting, and pagination in the workbench.

### Running the workbench

```bash
npm run dev --workspace @opensymphony/desktop
# then open http://127.0.0.1:1420/?fixtures
```

`?fixtures` mounts the desktop shell on the demo transport and fixture graph
adapter instead of the local gateway (see `apps/desktop/src/index.ts`).
Packaged Tauri builds never carry a query string, so production behavior is
unchanged.

### Renderer architecture

The knowledge-graph surface is a 3D command center:

- `packages/ui-core/src/knowledge-graph-scene.ts` — pure scene model: an
  orbital perspective camera (pan / cursor-anchored dolly / yaw-pitch
  orbit), a software projector that matches `THREE.PerspectiveCamera`
  (parity is unit-tested), diffuse per-area hull geometry with spatial
  outlier trimming (multi-area membership renders as overlapping hulls),
  zoom-dependent label opacities (area titles when zoomed out, node labels
  when zoomed in), hover adjacency emphasis, and hit-testing.
- `packages/ui-core/src/knowledge-graph-renderer.ts` — DOM/WebGL wiring:
  three.js rasterizes the already-projected screen-space scene (2D canvas
  fallback draws the identical scene), HTML overlays carry node labels,
  area titles, and the hover tooltip, and pointer handlers implement node
  dragging, panning, orbiting, wheel dolly, and double-click framing of a
  node's neighborhood or an area hull. Camera and drag overrides persist in
  `KnowledgeGraphViewState` across live refreshes.

Task-graph dependency arrows distinguish depth by where and how they meet the
target's connector circle: next-level edges arrive vertically at the **top** of
the circle (arrowhead pointing down), while skip-level edges route through a
left gutter — one lane and one hue per blocker, rounded corners, colored
arrowheads — and arrive horizontally at the **left** edge of the circle
(arrowhead pointing right). Hovering a task spotlights its incident arrows. See
`renderTaskGraphLink`/`buildTaskGraphLinks` in
`packages/ui-core/src/app-shell.ts`.

Read-only task cards are a single row: the connector circle, the identifier and
title, then the run **Status** pill and a **BLOCKER** badge (when the task is
actively blocking others) pinned to the right. Dependencies read from the
arrows and the connector glyph (`<`, `>`, `<>`), not a text line; the full
`blocked by … | blocks …` breakdown lives in the Run Detail panel. The task
filters are **Status** and **Search** only — every node is a task, so there is
no kind or runtime filter.

### Three-pane task graph

The desktop task surface splits into three panes
(`renderTaskGraphPanes` in `packages/ui-core/src/app-shell.ts`):

- **Completed** (collapsible, left) — a searchable, sortable, paginated
  table served by `GET /api/v1/memory/completed-tasks`. Rows come from the
  memory server's DuckDB catalog first (issue capsules with their
  normalized `pull_requests` evidence — completed tasks survive Linear
  archival and the Linear API is never queried on this path), merged with
  orchestrator-known completions not yet captured (`source:
  "orchestrator"`). Each row lists all of its PRs — the newest bold,
  unmerged ones struck through — plus a memory-capsule button that opens
  the task's capsule through `openMemoryDeepLink`.
- **Current** (center, never collapses) — the dispatchable dependency graph
  as before (Todo / In Progress / Human Review / Rework). Canceled nodes
  also stay here (they have no other pane, and the Canceled status filter
  must still surface them). Selecting or hovering a task boldens its
  incoming and outgoing edges, including outgoing edges that leave the
  pane's right side toward blocked Backlog tasks.
- **Backlog** (collapsible, right) — backlog-state Linear issues, now
  included in the task-graph snapshot (`LinearClient::project_task_graph_issues`
  returns them from the same single project scan that already served the
  identifier lookup; the scan also carries unrequested *active*-state issues,
  so a task promoted Backlog→Todo appears in Current even before the
  orchestrator control plane tracks it). Cards use the Current pane's grammar with faded
  edges; hovering or selecting a backlog task boldens its full **ancestry
  critical path** — every unfinished upstream chain that must complete to
  unblock it — across both panes, and dims unrelated backlog cards.

Cross-pane edges live in a measured SVG overlay
(`positionTaskGraphCrossLinks`): paths carry the same
`data-link-from`/`data-link-to` contract as in-pane edges and are
repositioned on scroll, resize, and collapse; endpoints scrolled out of
their pane hide instead of drawing across headers.

**Live status reflection.** The Current and Backlog panes are pure
functions of the latest task-graph snapshot — `renderTaskGraphPanes`
re-partitions the fresh nodes on every render — so a status change moves a
task between panes on the next live refresh with no restart: Backlog→Todo
lands it in Current, Todo→Backlog returns it, and a completion drops it from
Current. The Completed pane loads separately, so `refreshLiveGatewayData`
reloads it whenever `completedTasksSignature` changes — a signature over
both the task graph's done nodes and the dashboard snapshot's control-plane
completed count, so completions surface even when the finished issue is
absent from the task graph (e.g. no project metadata). `memory_graph_updated`
events reload it too, for capsule/PR evidence captured after completion.
Only real completions count: a workspace recovered at daemon startup whose
issue sits in a non-active tracker state is *parked* (runtime `idle`, and
the tracker state decides its pane), never reported as `completed`, and the
scheduler's 60-second dispatch discovery reopens a parked issue as soon as
its tracker state turns active again — no orchestrator restart required.

### Drill-down navigation

The knowledge graph navigates through three levels, mirroring the Obsidian
LLM-wiki experience (areas → concepts/tags → issue capsules) and back out:

- **Atlas → area**: a stationary click on an area cloud (while zoomed out
  enough that its title is visible) drills into that community — the view
  re-lays out around only its members (`COMMUNITY_SELECTED` + community
  filter). Dragging on the same cloud still pans.
- **Area → capsule**: selecting a concept lazily fetches its memory capsule
  through `GraphDataAdapter.getConceptDetail` and renders it in the
  inspector: frontmatter chips, the markdown body
  (`packages/ui-core/src/memory-markdown.ts`, escaped-first allowlist
  renderer), linked concepts, citations, and source refs. Capsule links
  (including `[[wiki-links]]` in the body) navigate the graph to their
  target node, re-drilling across areas when needed. The clickable entity
  list and the inspector live in the resizable lower workspace columns
  (narrow list left, capsule right), so the graph stage, the entities, and
  the capsule content share the fold.
- **Back out**: a breadcrumb (`Atlas › area › concept`) pops individual
  levels, Escape steps back one level at a time, and the "Show full graph"
  button still jumps straight home.

### Memory deep links

`packages/graph/src/deep-link.ts` defines the stable address format for
memory locations, designed to be embedded outside the graph UI (task-graph
artifacts, notifications, docs):

```
opensymphony://memory/<bundleId>
opensymphony://memory/<bundleId>/communities/<communityId>
opensymphony://memory/<bundleId>/concepts/<conceptId>   # conceptId keeps its slashes: issues/COE-399
```

`formatMemoryDeepLink`/`parseMemoryDeepLink` round-trip these strictly
(unknown shapes are rejected, never guessed). The app shell exposes
`OpenSymphonyAppHandle.openMemoryDeepLink(url)` — it switches to the
Knowledge Graph pane, loads the bundle, drills into the concept's area, and
opens its capsule; this is the wiring point for task-graph artifact links.
The inspector's "Copy deep link" button emits the same links. For manual
testing, `?memory=<deep-link>` on the desktop dev server (composable with
`?fixtures`) opens a link at boot, e.g.
`?fixtures&memory=opensymphony://memory/viz-workbench/concepts/concepts/code-intelligence-01`.

### Code Graph surface

Code Graph is a third shared surface in the graph toolbar. It uses the same
scene, layout adapter, orbital camera, hulls, label LOD, hover emphasis, and
2D fallback as Knowledge Graph; `packages/graph/src/code-graph.ts` only adapts
the code DTOs and state. Atlas requests are directory/community aggregates,
while File, Neighborhood, and Diff are scoped requests. The fixture adapter
in `viz-fixture.ts`, HTTP adapter, and Tauri-native adapter all consume the
same gateway-schema DTOs.

The Code Graph filter panel covers repository, language, symbol kind, edge
kind, confidence, freshness, diagnostics, path prefix, community, and delta
status. Confidence is rendered through line style and opacity; freshness is
rendered through node opacity, border style, and an inspector badge. The
lower workspace columns provide a structure-list fallback and symbol/file
inspector with provenance, diagnostics, relationships, and a raw-record
toggle.

Code navigation follows `Repo › module › file › symbol`: stationary aggregate
clicks drill in, breadcrumbs and Escape pop one level, and double-click frames
a symbol neighborhood. Code links are strict and round-trip through:

```
opensymphony://code/<repoId>/atlas
opensymphony://code/<repoId>/files/<path>
opensymphony://code/<repoId>/symbols/<symbolKey>
opensymphony://code/<repoId>/diff/<baseRevision>/<headRevision>
```

`OpenSymphonyAppHandle.openCodeDeepLink(url)` restores mode, target, filters,
depth, revisions, and layout seed. The desktop and web boot paths accept
`?code=<deep-link>` alongside `?fixtures`; `code_graph_updated` refreshes the
active snapshot without clearing camera, drag overrides, or selection.

Code Graph hardening uses the same fixture workbench for web and desktop. The
scale tiers are edge-heavy (500/2K, 5K/20K, and 20K/80K nodes/edges), while the
reference Atlas fixture represents 50K symbols and 200K edges through at most
2,000 total node-and-edge render elements (1,000 directory nodes and their
contained edges). Atlas never renders raw symbols;
expansion is a scoped follow-up request. When a bound trims a response, the
toolbar and screen-reader summary expose both dropped counts and the reason.

The structure list is always available as the keyboard and screen-reader
fallback for Atlas, File, Neighborhood, and Diff. Freshness, confidence,
diagnostics, and diff status have text, badges, opacity, borders, or line
styles in addition to color. Reduced motion settles the shared camera
immediately. Desktop native commands use the same camelCase request names and
gateway-schema DTOs as HTTP, including the hosted default-deny snippet policy;
paths exposed to clients remain workspace-relative and stale/unsupported
records stay visible only when explicitly requested.

### Tests that gate this area

- `packages/ui-core/__tests__/knowledge-graph-scene.test.ts` — projector ↔
  THREE parity, camera ops, hulls (incl. multi-area and outlier trimming),
  label LOD, hover emphasis, hit-tests, fixture density/determinism.
- `packages/ui-core/__tests__/app-shell.test.ts` — surface markup, LOD label
  budget through the real mount, arrow routing shape (`os-tg-hue-*`,
  rounded gutter paths), WebGL smoke via Playwright when available, plus the
  drill-down flows: area-cloud click drilling, capsule fetch/render/retry,
  capsule-link navigation, breadcrumbs, stepwise Escape, and
  `openMemoryDeepLink` end to end.
- `packages/graph/__tests__/deep-link.test.ts` — deep-link round-trips and
  strict rejection, node addressing, fixture capsule determinism and link
  resolvability.
- `packages/graph/__tests__/code-graph.test.ts` and
  `packages/ui-core/__tests__/code-graph.test.ts` — code adapters, filters,
  deep links, DTO-to-scene styling, inspector markup, and layout semantics.
- `packages/graph/__tests__/deep-link.test.ts` — code deep-link round-trips and
  strict rejection beside the memory deep-link suite.
- `packages/ui-core/__tests__/memory-markdown.test.ts` — capsule markdown
  allowlist rendering and escaping.
- `packages/graph/__tests__/completed-tasks.test.ts` — completed-task
  paging/sorting/search (`pageCompletedTasks`, the fixture adapter's twin of
  the gateway endpoint) and fixture-row determinism.
- The three-pane suite inside `app-shell.test.ts` — pane rendering,
  Completed search/sort/pagination, PR emphasis, capsule deep links,
  ancestry critical-path emphasis, and pane collapse.
- Rust: `crates/opensymphony-gateway/tests/gateway.rs`
  (`gateway_task_graph_includes_backlog_issues_with_cross_edges`,
  `gateway_serves_memory_completed_tasks`),
  `crates/opensymphony-linear/tests/linear_client.rs`
  (`project_task_graph_issues_return_requested_backlog_and_active_from_one_scan`),
  and the memory crate's PR-evidence projection unit test.

When iterating on visuals, extend these fixtures and tests rather than
creating throwaway data; `AGENTS.md` ("UI separation") points here.
