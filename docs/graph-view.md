---
type: topic-doc
area: graph-view
visibility: public
last_memory_sync: 2026-07-02T03:46:15.373470+00:00
---

# Graph View

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- PR-197 contributed: PR #197: feat(graph): 3D knowledge-graph command center, differentiated task-graph arrows, shared viz fixtures (merge `8e341b5`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- PR-197: Graph visualization command center and shared fixtures

## Source refs

- PR-197

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
  overlapping skips from different blockers. `createGraphVizDemoTransport()`
  wraps it all in a `MockGatewayTransport`.

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

Task-graph dependency arrows route skip-level edges through a left gutter
with one lane and one hue per blocker (rounded corners, colored arrowheads);
hovering a task spotlights its incident arrows. See
`renderTaskGraphLink`/`buildTaskGraphLinks` in
`packages/ui-core/src/app-shell.ts`.

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
- `packages/ui-core/__tests__/memory-markdown.test.ts` — capsule markdown
  allowlist rendering and escaping.

When iterating on visuals, extend these fixtures and tests rather than
creating throwaway data; `AGENTS.md` ("UI separation") points here.
