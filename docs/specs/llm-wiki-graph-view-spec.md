# LLM Wiki Graph View Specification

Status: draft

Source basis: [Open Knowledge Format v0.1 draft](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md), the OpenSymphony OKF memory specification, and the current rich client gateway architecture.

Reader: an OpenSymphony engineer designing the shared web and Tauri desktop knowledge visualization experience.

Post-read action: implement a client-side graph view that visualizes OKF memory bundles and LLM wiki concepts through stable gateway data contracts, with Three.js rendering where scale and interaction justify it.

## 1. Summary

OpenSymphony should provide a first-class LLM Wiki Graph View for OKF bundles. The tool is similar in spirit to Obsidian Graph View, but it is purpose-built for agent memory, work graph context, code intelligence, and hosted/private visibility rules.

The graph view lets users:

- See bundles, concepts, directories, tags, citations, source refs, and cross-bundle relationships.
- Navigate from project memory to issue capsules, topic docs, repository facets, code context, and references.
- Inspect frontmatter in a polished human-readable panel.
- Find graph communities and understand why they exist.
- Move fluidly between graph overview, focused neighborhoods, and concept reading.

The viewer is a client surface. It must not mutate orchestrator state or bypass memory visibility rules.

## 2. Goals

1. Make OKF bundle structure visually navigable for humans.
2. Support both local Tauri desktop and browser-hosted web clients through shared frontend code.
3. Use the OpenSymphony Gateway or memory server as the data boundary.
4. Render large graphs smoothly with a Three.js/WebGL path and worker-based layout.
5. Show frontmatter as meaningful metadata, not raw YAML by default.
6. Help users discover graph communities, stale areas, disconnected concepts, and high-leverage bridges.
7. Preserve accessibility through keyboard navigation, list views, and non-canvas summaries.

## 3. Non-Goals

- Do not edit scheduling state, Linear state, or orchestrator internals from the graph.
- Do not make graph rendering the source of truth for memory links.
- Do not require Three.js for every small graph; SVG or Canvas fallback may be enough for tiny fixtures.
- Do not infer strong semantic meaning from plain Markdown links. OKF links are relationships, but the surrounding prose owns the precise meaning.
- Do not expose private concepts, local paths, source snapshots, or frontmatter fields to unauthorized clients.

## 4. Users And Workflows

### 4.1 Operator

The operator wants to answer:

- What does OpenSymphony remember about this project?
- Which issues, milestones, and topic areas are connected?
- What changed recently?
- Which memory records are stale, isolated, or warning-heavy?

### 4.2 Implementation Agent Supervisor

The supervisor wants to answer:

- What context would an agent receive for this issue?
- Which prior decisions connect to the files or areas the agent is about to touch?
- Which source refs support a memory claim?

### 4.3 Technical Lead

The technical lead wants to answer:

- Which subsystems have dense historical context?
- Which work items created cross-cutting design decisions?
- Which repositories, areas, and milestones form communities?

## 5. Product Shape

The primary view is a dense operational workspace, not a landing page.

Recommended layout:

- Left rail: bundle selector, scope filters, saved views, graph metrics.
- Center: full-bleed graph canvas.
- Right inspector: selected concept, frontmatter summary, links, citations, source refs, and body preview.
- Bottom strip: selection path, active filters, layout status, and community legend.

Primary modes:

| Mode | Purpose |
| --- | --- |
| Atlas | Shows all accessible bundles and cross-bundle links. |
| Bundle | Shows one bundle with directory hierarchy and concept links. |
| Community | Focuses one detected community and its boundary edges. |
| Neighborhood | Shows a selected concept and N-hop related concepts. |
| Timeline | Highlights recently created or updated concepts. |
| Evidence | Emphasizes citations, source refs, and mirrored references. |

## 6. Graph Model

### 6.1 Nodes

Required node kinds:

| Kind | Description |
| --- | --- |
| `bundle` | OKF bundle root. |
| `directory` | Directory grouping inside a bundle. |
| `concept` | Any OKF concept document. |
| `tag` | Tag synthesized from frontmatter. |
| `resource` | Canonical resource URI when present. |
| `citation` | External URL or mirrored reference concept. |
| `source_ref` | Linear issue, GitHub PR, merge SHA, source snapshot, or similar support. |
| `community` | Detected graph community, used as an overlay or virtual node. |

Concept nodes carry:

- bundle ID
- concept ID
- type
- title
- description
- path display
- resource
- tags
- timestamp
- visibility
- freshness
- warning count
- frontmatter summary
- unknown frontmatter fields
- body preview
- metrics such as indegree, outdegree, PageRank, and community ID

### 6.2 Edges

Required edge kinds:

| Kind | Direction | Source |
| --- | --- | --- |
| `contains` | parent to child | Bundle and directory hierarchy. |
| `markdown_link` | source concept to target concept | Standard Markdown links. |
| `external_link` | concept to URL | External Markdown links. |
| `cites` | concept to citation or reference | Citations section and source refs. |
| `tagged_with` | concept to tag | Frontmatter tags. |
| `describes_resource` | concept to resource | Frontmatter resource URI. |
| `scoped_to` | concept to work graph node | OpenSymphony scope refs. |
| `source_supported_by` | concept to source ref | OpenSymphony source refs. |
| `same_resource` | concept to concept | Shared canonical resource, derived by catalog. |

`markdown_link` edges should remain untyped unless a producer supplies additional metadata. The UI can show link context snippets, but it should avoid pretending the link means "depends on" or "blocks" unless that relation came from typed metadata.

Broken Markdown links should appear as dim unresolved targets when useful. They are not malformed concepts.

## 7. Data Contracts

The client should not parse private files directly during normal operation. It should consume versioned DTOs from the gateway or memory server.

### 7.1 Bundle List

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "bundles": [
    {
      "id": "local-default",
      "title": "OpenSymphony Memory",
      "okf_version": "0.1",
      "visibility": "private",
      "concept_count": 412,
      "updated_at": "2026-06-13T17:00:00Z"
    }
  ]
}
```

### 7.2 Graph Snapshot

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "bundle_id": "local-default",
  "cursor": {"sequence": 1842, "partition": "memory-graph:local-default"},
  "nodes": [],
  "edges": [],
  "communities": [],
  "filters_applied": [],
  "generated_at": "2026-06-13T17:00:00Z"
}
```

### 7.3 Concept Detail

```json
{
  "schema_version": {"major": 1, "minor": 0, "patch": 0},
  "bundle_id": "local-default",
  "concept_id": "issues/COE-123",
  "frontmatter_view": {
    "primary": {},
    "opensymphony": {},
    "unknown": {}
  },
  "body_markdown": "# COE-123",
  "links": [],
  "citations": [],
  "source_refs": []
}
```

Recommended gateway endpoints:

- `GET /api/v1/memory/bundles`
- `GET /api/v1/memory/bundles/{bundle_id}/graph`
- `GET /api/v1/memory/bundles/{bundle_id}/concepts/{concept_id}`
- `GET /api/v1/memory/bundles/{bundle_id}/communities`
- `GET /api/v1/memory/search`
- event stream kind `memory_graph_updated`

The same schemas can be used by a Tauri native adapter, loopback HTTP, or hosted HTTPS.

## 8. Rendering Architecture

### 8.1 Shared Frontend Package

Create a shared graph package for web and desktop clients. It should be transport-agnostic and receive graph DTOs plus callbacks.

Responsibilities:

- graph state reducer
- graph layout worker
- Three.js renderer
- inspector components
- search and filter state
- keyboard navigation
- accessibility fallback list

### 8.2 Three.js Renderer

Use Three.js when graphs exceed the comfortable DOM/SVG threshold or when 2.5D/3D interaction is valuable.

Default mode should be 2.5D with an orthographic camera:

- pan and zoom are primary
- labels stay readable
- depth separates clusters without making navigation disorienting

Optional 3D mode can use perspective camera and orbit controls, but it should not be the default for operational work.

Rendering requirements:

- Nodes use instanced geometry.
- Edges use batched line geometry.
- Labels use level-of-detail rules.
- Hover and selection use GPU-friendly picking or a spatial index.
- Large layouts run in a Web Worker.
- The UI remains responsive while layout stabilizes.

Recommended libraries:

- Three.js for rendering.
- A proven force-layout or graph library for layout and metrics.
- A graph community package for Louvain or Leiden-style clustering.

Do not hand-roll force physics or community detection unless the dependency evaluation rejects available libraries.

## 9. Layout And Community Detection

Required layouts:

- Force layout for exploratory graph view.
- Hierarchical layout for bundle/directory containment.
- Radial neighborhood layout for selected concepts.
- Timeline layout for recency mode.

Community detection:

- Compute communities from concept and typed auxiliary edges.
- Let users choose whether tags, citations, and source refs participate in community detection.
- Label communities from dominant concept types, tags, areas, or directories.
- Show boundary edges that connect communities.
- Let users pin, isolate, hide, or compare communities.

Graph metrics:

- degree
- betweenness or bridge score
- PageRank or centrality score
- orphan count
- broken-link count
- stale concept count
- warning count

## 10. Interaction Requirements

Navigation:

- click selects
- double-click focuses neighborhood
- keyboard arrows move between visible neighbors
- command palette searches bundles, concepts, tags, resources, citations, and source refs
- browser history or app history stores selected concept and filters
- deep links open a bundle, concept, mode, and selection

Filtering:

- bundle
- concept type
- tag
- area
- project
- milestone
- issue
- repository
- visibility
- freshness
- warning status
- source kind
- link kind
- community

Inspection:

- right inspector renders frontmatter as sections and chips
- raw YAML is available behind a toggle
- Markdown body preview is readable and source-linked
- citations and source refs are grouped and clickable
- unknown frontmatter fields are preserved and shown in an advanced section
- broken links have clear unresolved state

Graph editing is out of scope for the first version. Future link editing must route through memory or docs authoring APIs and preserve OKF round-tripping.

## 11. Frontmatter Presentation

The default frontmatter view should be human-first:

- Primary summary: title, type, description, timestamp, visibility, freshness.
- Relationship chips: tags, areas, project, milestone, issue, repository.
- Resource card: canonical URI with copy/open actions.
- Source support: Linear, GitHub PR, merge SHA, snapshots, mirrored references.
- System metadata: producer, schema version, capture time, docs sync status.
- Advanced section: unknown keys rendered as formatted key/value rows.
- Raw section: syntax-highlighted YAML for exact inspection.

Design tone:

- dense, calm, and operational
- readable on desktop and browser
- no decorative hero treatment
- use color to encode meaning, not decoration
- preserve enough contrast for long inspection sessions

## 12. Security And Privacy

The graph view must enforce the same visibility boundary as memory retrieval.

Requirements:

- Gateway filters private concepts before sending graph DTOs.
- Hosted tokens cannot widen scope through client filters.
- Local paths are hidden or normalized unless the client is authorized for local desktop mode.
- Secret-like values in frontmatter are redacted before rendering.
- Clipboard actions require explicit user interaction.
- Tauri filesystem access is not required for normal graph browsing.
- Any local-file open action uses a desktop capability gate.

## 13. Accessibility

Canvas-only graph views are not enough.

Requirements:

- keyboard selection and navigation
- searchable table/list alternative for visible nodes
- screen-reader summary of selected graph, filters, and communities
- inspector content uses semantic HTML
- focus order is predictable
- reduced-motion mode stops layout animation after initial stabilization
- color is never the only signal for type, status, or community

## 14. Performance Targets

Initial targets:

- 500 nodes: interactive within 500 ms after data load.
- 5,000 nodes: usable with progressive layout and level-of-detail labels.
- 20,000 nodes: supported through filtering, aggregation, and community overview, not full-label rendering.
- Selection and inspector update under 100 ms for loaded concepts.
- Filter changes show immediate busy state and stream incremental results where possible.

Large graphs should default to community aggregation. The user can expand communities rather than receiving an unreadable hairball.

## 15. Implementation Phases

### Phase 1: Read-Only OKF Graph

- Add graph DTOs derived from OKF concepts.
- Render bundle, directory, concept, tag, citation, and source-ref nodes.
- Support pan, zoom, select, search, filter, and inspector.
- Use fixture data in web and desktop builds.

### Phase 2: Layouts And Communities

- Move layout to a worker.
- Add community detection and labels.
- Add neighborhood, community, and evidence modes.
- Add deep links and app history.

### Phase 3: Live Memory Integration

- Connect to live gateway or memory server.
- Add `memory_graph_updated` event handling.
- Support stale graph state and reindex warnings.
- Enforce hosted visibility filtering.

### Phase 4: Scale And Polish

- Add aggregation for very large bundles.
- Add minimap or overview lens.
- Add comparative community view.
- Add visual regression checks for desktop and browser.

## 16. Test Plan

Required tests:

- OKF fixture graph extraction.
- Broken-link rendering.
- Unknown frontmatter rendering.
- Visibility filtering.
- Search and filter reducer behavior.
- Community detection fixture stability.
- Keyboard navigation.
- Accessibility list fallback.
- Web build excludes Tauri-only dependencies.
- Desktop build uses the same DTO contract.
- Playwright screenshot checks for desktop and browser viewports.
- WebGL canvas nonblank checks for graph render.

## 17. Acceptance Criteria

- Users can open an accessible OKF bundle and see a navigable graph.
- Users can select a concept and read human-friendly frontmatter without opening raw YAML.
- Users can filter by type, tag, area, project, issue, repository, source kind, freshness, and community.
- Users can switch between atlas, bundle, community, neighborhood, timeline, and evidence modes.
- The graph handles private/public visibility correctly.
- The component works in both Tauri desktop and browser clients through the same graph DTOs.
- Three.js rendering is nonblank, responsive, and tested at desktop and mobile viewports.

## 18. Open Questions

1. Should the first graph API be served by the gateway, the memory MCP server, or both with shared DTOs?
2. Should code-intelligence nodes be enabled by default, or hidden until the user opts into code context?
3. Should local desktop mode support direct bundle opening from disk, or should all local browsing still go through the memory server?
4. Which graph library should own layout and community detection after dependency review?
