import type {
  CodeDiffOverlay,
  CodeFileOutline,
  CodeGraphSnapshot,
  CodeRepoList,
  CodeSymbolDetail,
  MemoryBundleList,
  MemoryCommunityList,
  MemoryCompletedTask,
  MemoryConceptDetail,
  MemoryGraphEdge,
  MemoryGraphNode,
  MemoryGraphSnapshot,
  MemoryTaskPullRequest,
} from "@opensymphony/gateway-schema";

/**
 * Dense, deterministic knowledge-graph fixture for graph-visualization work.
 *
 * The default fixture (`fixtureGraphSnapshot`) is intentionally tiny; it
 * exercises reducers, not rendering. Visualization work needs the opposite:
 * enough nodes that labels collide when zoomed out, several area clusters,
 * concepts that belong to more than one area (overlapping hulls), long and
 * short titles, and a spread of node kinds/degrees. This module builds that
 * graph from a seeded PRNG so every run — tests, the `?fixtures` desktop
 * workbench, screenshots in docs — sees the same graph.
 *
 * See docs/graph-view.md ("Graph visualization workbench") before reaching
 * for ad-hoc data in graph-viz iterations.
 */

const schema_version = { major: 1, minor: 0, patch: 0 };
const generated_at = "2026-07-04T00:00:00Z";
const bundleId = "viz-workbench";

interface VizArea {
  slug: string;
  label: string;
  topics: string[];
}

const vizAreas: VizArea[] = [
  {
    slug: "code-intelligence",
    label: "Code Intelligence",
    topics: [
      "Tree-sitter Provider Skeleton",
      "Query Packs For Supported Agent Languages",
      "Read-Only AST MCP And CLI Tools",
      "Memory Context AST Provider Integration",
      "Code Intelligence Persistence And Ingest",
      "Cache Code-Intel Parsers And Compiled Query Packs",
      "Graph Extraction Metrics And Ingest Benchmarks",
      "Deduplicate Query-Pack Assets Across Harnesses",
      "Symbol Search Ranking",
      "Incremental Reparse Scheduling",
      "Language Injection Handling",
      "AST Evidence Citations",
    ],
  },
  {
    slug: "memory-graph",
    label: "Memory Graph",
    topics: [
      "Memory Graph DTOs And Gateway Reads",
      "OKF Bundle Schema And Legacy Migration",
      "Concept Inspector Search And Filters",
      "Catalog Reindex And Query Compatibility From OKF",
      "Memory Bundle Visibility Policies",
      "Community Detection Aggregation",
      "Concept Freshness And Warning Counters",
      "Graph Snapshot Cursor Semantics",
      "Bundle Import Export Round-Trips",
      "Stale Snapshot Reconciliation",
      "Frontmatter Quarantine Rules",
      "Memory Admin Token Gating",
    ],
  },
  {
    slug: "desktop-shell",
    label: "Desktop Shell",
    topics: [
      "Desktop Project Grouping And Filters",
      "Desktop Run Detail Action Wiring",
      "Lazy Desktop Launcher Commands",
      "Desktop Task Graph Dependency Lanes",
      "Hosted Auth Placeholders",
      "Model Configuration Settings Surface",
      "Workspace Pane Resize Persistence",
      "Live Refresh Interaction Epochs",
      "DOM Morph Render Pipeline",
      "Status Strip Density Pass",
      "Planning Workspace Preview Banner",
      "Desktop Installer Validation",
    ],
  },
  {
    slug: "gateway",
    label: "Gateway",
    topics: [
      "Gateway DTO Boundary Checklist",
      "Run Diff Endpoint Batching",
      "Workspace Scan Coalescing",
      "PR URL Background Resolution",
      "Event Journal Cursor Replay",
      "Terminal Log Store Ingest",
      "Capabilities Matrix Truth",
      "Dashboard Snapshot Projection",
      "Approval Decision Dispatch",
      "Loopback Transport Contract",
    ],
  },
  {
    slug: "orchestrator",
    label: "Orchestrator",
    topics: [
      "Scheduler-Side Codex Stdio Interrupt Channel",
      "Retry Queue Depth Accounting",
      "Issue Session Runner Lifecycle",
      "Hierarchy Selection Guardrails",
      "Tracker Polling Cadence",
      "Cancellation Acknowledgement Path",
      "Workspace Key Sanitization",
      "Dependency-Aware Dispatch Ordering",
      "Blocked Issue Signaling",
      "Run Outcome Reconciliation",
    ],
  },
  {
    slug: "harness-runtime",
    label: "Harness Runtime",
    topics: [
      "OpenHands Agent-Server Integration",
      "Codex App Server Turn Interrupts",
      "ChatGPT OAuth For Codex Harness",
      "App-Server Turn Interrupt Fallbacks",
      "WebSocket Readiness Barrier",
      "Conversation Reuse Per Issue",
      "Subscription Credential Bootstrap",
      "Harness Capability Discovery",
      "Linear Polling And Rate-Limit Backoff",
      "Turn Token Usage Accounting",
    ],
  },
  {
    slug: "testing",
    label: "Testing And Operations",
    topics: [
      "Fake Server Contract Suite",
      "Live Pinned Server Checks",
      "Graph Scale Visual Regression Fixtures",
      "Terminal Renderer Benchmarks",
      "Client Resilience Scenarios",
      "Doctor Diagnostics Coverage",
      "Deployment Mode Smoke Runs",
      "Accessibility Audit Sweeps",
    ],
  },
];

const vizTags = [
  "frontend",
  "rust",
  "performance",
  "streaming",
  "security",
  "protocol",
  "ux",
  "observability",
] as const;

/** Deterministic PRNG (mulberry32) so the fixture never shifts between runs. */
function mulberry32(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface VizConcept {
  id: string;
  label: string;
  areas: string[];
  tags: string[];
}

function buildVizConcepts(): VizConcept[] {
  const random = mulberry32(0x0587_0a11);
  const concepts: VizConcept[] = [];
  vizAreas.forEach((area, areaIndex) => {
    area.topics.forEach((topic, topicIndex) => {
      const areas = [area.slug];
      // Roughly a quarter of concepts also belong to a neighboring area so
      // hull rendering must cope with overlapping membership.
      if (random() < 0.28) {
        const other = vizAreas[(areaIndex + 1 + Math.floor(random() * (vizAreas.length - 1))) % vizAreas.length];
        if (other.slug !== area.slug) areas.push(other.slug);
      }
      const tags = [vizTags[(areaIndex + topicIndex) % vizTags.length]];
      if (random() < 0.35) {
        tags.push(vizTags[(areaIndex * 3 + topicIndex * 5 + 2) % vizTags.length]);
      }
      concepts.push({
        id: `concept:${area.slug}-${String(topicIndex + 1).padStart(2, "0")}`,
        label: topic,
        areas,
        tags: [...new Set(tags)],
      });
    });
  });
  return concepts;
}

function conceptNode(concept: VizConcept, index: number): MemoryGraphNode {
  return {
    id: concept.id,
    kind: "concept",
    label: concept.label,
    bundle_id: bundleId,
    concept_id: concept.id.replace("concept:", "concepts/"),
    concept_type: "concept",
    path_display: `${concept.id.replace("concept:", "concepts/")}.md`,
    tags: concept.tags,
    timestamp: generated_at,
    visibility: index % 7 === 3 ? "public" : "private",
    freshness: index % 23 === 11 ? "stale" : "current",
    warning_count: index % 29 === 17 ? 1 : 0,
    frontmatter_summary: {
      areas: concept.areas,
      project: "OpenSymphony-bootstrap",
      repository: "OpenSymphony",
    },
    unknown_frontmatter: {},
    body_preview: `${concept.label} — fixture concept for graph visualization work.`,
    metrics: { indegree: 0, outdegree: 0, community_id: `area:${concept.areas[0]}` },
  };
}

function buildSnapshot(): MemoryGraphSnapshot {
  const random = mulberry32(0x870);
  const concepts = buildVizConcepts();
  const conceptNodes = concepts.map(conceptNode);
  const byArea = new Map<string, VizConcept[]>();
  for (const concept of concepts) {
    for (const area of concept.areas) {
      const members = byArea.get(area) ?? [];
      members.push(concept);
      byArea.set(area, members);
    }
  }

  const bundleNode: MemoryGraphNode = {
    id: `bundle:${bundleId}`,
    kind: "bundle",
    label: "Graph Viz Workbench",
    bundle_id: bundleId,
    tags: [],
    visibility: "private",
    freshness: "current",
    warning_count: 0,
    frontmatter_summary: {},
    unknown_frontmatter: {},
    metrics: { indegree: 0, outdegree: vizAreas.length },
  };

  const tagNodes: MemoryGraphNode[] = vizTags.map((tag) => ({
    id: `tag:${tag}`,
    kind: "tag",
    label: tag,
    bundle_id: bundleId,
    tags: [tag],
    visibility: "private",
    freshness: "current",
    warning_count: 0,
    frontmatter_summary: {},
    unknown_frontmatter: {},
    metrics: { indegree: 0, outdegree: 0 },
  }));

  const sourceNodes: MemoryGraphNode[] = vizAreas.slice(0, 5).map((area, index) => ({
    id: `source:osym-${900 + index}`,
    kind: "source_ref",
    label: `OSYM-${900 + index}`,
    bundle_id: bundleId,
    tags: [],
    visibility: "private",
    freshness: "current",
    warning_count: 0,
    frontmatter_summary: { source_kind: "task", areas: [area.slug] },
    unknown_frontmatter: {},
    metrics: { indegree: 0, outdegree: 0 },
  }));

  const edges: MemoryGraphEdge[] = [];
  const pushEdge = (kind: MemoryGraphEdge["kind"], sourceId: string, targetId: string) => {
    edges.push({
      id: `edge:${kind}:${sourceId}:${targetId}`,
      kind,
      source_id: sourceId,
      target_id: targetId,
      unresolved: false,
      metadata: {},
    });
  };

  // The bundle anchors one root concept per area (a full contains-fan would
  // collapse the force layout into a hub-and-spoke hairball).
  for (const area of vizAreas) {
    const root = byArea.get(area.slug)?.[0];
    if (root) pushEdge("contains", bundleNode.id, root.id);
  }

  // Intra-area web: chain plus a few shortcuts so clusters read as clusters.
  for (const area of vizAreas) {
    const members = (byArea.get(area.slug) ?? []).filter((concept) => concept.areas[0] === area.slug);
    for (let index = 1; index < members.length; index += 1) {
      pushEdge("markdown_link", members[index - 1].id, members[index].id);
    }
    for (let index = 0; index < members.length; index += 1) {
      if (random() < 0.4 && members.length > 3) {
        const target = members[Math.floor(random() * members.length)];
        if (target.id !== members[index].id) {
          pushEdge("markdown_link", members[index].id, target.id);
        }
      }
    }
  }

  // Sparse cross-area references: enough to pull hulls near one another
  // without merging the clusters.
  for (let index = 0; index < concepts.length; index += 1) {
    if (random() < 0.12) {
      const target = concepts[Math.floor(random() * concepts.length)];
      if (target.areas[0] !== concepts[index].areas[0]) {
        pushEdge("cites", concepts[index].id, target.id);
      }
    }
  }

  for (const concept of concepts) {
    if (random() < 0.3) {
      pushEdge("tagged_with", concept.id, `tag:${concept.tags[0]}`);
    }
  }
  sourceNodes.forEach((source, index) => {
    const area = vizAreas[index];
    const anchor = byArea.get(area.slug)?.[1];
    if (anchor) pushEdge("source_supported_by", anchor.id, source.id);
  });

  const dedupedEdges = [...new Map(edges.map((edge) => [edge.id, edge])).values()];
  const nodes = [bundleNode, ...conceptNodes, ...tagNodes, ...sourceNodes];
  const degreeById = new Map<string, { indegree: number; outdegree: number }>();
  for (const edge of dedupedEdges) {
    const source = degreeById.get(edge.source_id) ?? { indegree: 0, outdegree: 0 };
    source.outdegree += 1;
    degreeById.set(edge.source_id, source);
    const target = degreeById.get(edge.target_id) ?? { indegree: 0, outdegree: 0 };
    target.indegree += 1;
    degreeById.set(edge.target_id, target);
  }
  for (const node of nodes) {
    const degrees = degreeById.get(node.id);
    if (degrees) {
      node.metrics = { ...node.metrics, ...degrees };
    }
  }

  const communities = vizAreas.map((area) => ({
    id: `area:${area.slug}`,
    label: area.label,
    node_ids: (byArea.get(area.slug) ?? []).map((concept) => concept.id).sort(),
    concept_count: (byArea.get(area.slug) ?? []).length,
  }));

  return {
    schema_version,
    bundle_id: bundleId,
    cursor: { sequence: 1, partition: `memory-graph:${bundleId}` },
    generated_at,
    filters_applied: [],
    communities,
    nodes,
    edges: dedupedEdges,
    metrics: {
      orphan_count: 0,
      broken_link_count: 0,
      stale_concept_count: nodes.filter((node) => node.freshness === "stale").length,
      warning_count: nodes.reduce((sum, node) => sum + node.warning_count, 0),
    },
  };
}

export const graphVizFixtureSnapshot: MemoryGraphSnapshot = buildSnapshot();

export const graphVizFixtureBundleList: MemoryBundleList = {
  schema_version,
  bundles: [
    {
      id: bundleId,
      title: "Graph Viz Workbench",
      okf_version: "0.1",
      visibility: "private",
      concept_count: graphVizFixtureSnapshot.nodes.filter((node) => node.kind === "concept").length,
      updated_at: generated_at,
    },
  ],
};

export const graphVizFixtureCommunityList: MemoryCommunityList = {
  schema_version,
  bundle_id: bundleId,
  communities: graphVizFixtureSnapshot.communities,
  generated_at,
};

/** Named completed tasks matching the three-pane design mocks (newest first). */
const completedTaskTitles: ReadonlyArray<readonly [string, string]> = [
  ["VIZ-100", "Scene model spike"],
  ["VIZ-099", "Data pipeline setup"],
  ["VIZ-098", "Graph schema v1"],
  ["VIZ-097", "Tokenization service"],
  ["VIZ-096", "Auth integration"],
  ["VIZ-095", "Build system config"],
  ["VIZ-094", "Logging baseline"],
  ["VIZ-093", "Local runner"],
  ["VIZ-092", "CLI scaffolding"],
  ["VIZ-091", "Config loader"],
  ["VIZ-090", "Health checks"],
  ["VIZ-089", "Metrics exporter"],
  ["VIZ-088", "Cache layer"],
  ["VIZ-087", "Test harness"],
  ["VIZ-086", "Palette tokens pass"],
  ["VIZ-085", "Renderer profiling hooks"],
  ["VIZ-084", "Snapshot cursor plumbing"],
  ["VIZ-083", "Diff pager batching"],
  ["VIZ-082", "Fixture transport shims"],
  ["VIZ-081", "Event journal replay"],
  ["VIZ-080", "Keyboard focus audit"],
  ["VIZ-079", "Session token refresh"],
  ["VIZ-078", "Approval banner polish"],
  ["VIZ-077", "Workspace scan cache"],
  ["VIZ-076", "Terminal frame budget"],
  ["VIZ-075", "Retry queue telemetry"],
  ["VIZ-074", "Capability probe matrix"],
  ["VIZ-073", "Run detail parity sweep"],
  ["VIZ-072", "Dark theme contrast pass"],
  ["VIZ-071", "Icon set consolidation"],
  ["VIZ-070", "Bootstrap CLI docs"],
];

function buildCompletedTasks(): MemoryCompletedTask[] {
  const concepts = buildVizConcepts();
  return completedTaskTitles.map(([issueKey, title], index) => {
    const issueNumber = Number.parseInt(issueKey.slice(4), 10);
    // One completion per weekday-ish cadence walking back from May 1.
    const completedAt = new Date(Date.UTC(2026, 4, 1, 17, 0, 0) - index * 86_400_000).toISOString();
    const prNumber = issueNumber + 1;
    const prs: MemoryTaskPullRequest[] = [
      {
        number: prNumber,
        title: `${issueKey} ${title.toLowerCase()}`,
        url: `https://github.com/example/opensymphony/pull/${prNumber}`,
        merged: true,
        merged_at: completedAt,
      },
    ];
    // Every fifth task carries an earlier abandoned PR (never merged) so the
    // multi-PR presentation — newest bold, unmerged struck through — is
    // exercised by the fixture.
    if (index % 5 === 4) {
      prs.unshift({
        number: prNumber - 40,
        title: `${issueKey} first attempt (superseded)`,
        url: `https://github.com/example/opensymphony/pull/${prNumber - 40}`,
        merged: false,
      });
    }
    const concept = concepts[(index * 7) % concepts.length];
    return {
      issue_key: issueKey,
      // Reuse real fixture concepts so capsule deep links resolve inside the
      // workbench's knowledge graph.
      concept_id: concept.id.replace("concept:", "concepts/"),
      bundle_id: bundleId,
      title,
      state: "Done",
      milestone: "M13",
      url: `https://linear.app/example/issue/${issueKey}`,
      completed_at: completedAt,
      prs,
      source: "memory",
    };
  });
}

export const graphVizFixtureCompletedTasks: MemoryCompletedTask[] = buildCompletedTasks();

/**
 * Capsule detail for a fixture concept, derived from the snapshot's own
 * edges so every link inside a capsule resolves to a real node in the graph.
 * Returns null for unknown concept ids, mirroring the gateway's 404.
 * Accepts either the concept_id ("concepts/memory-graph-01") or node id.
 */
export function graphVizFixtureConceptDetail(conceptId: string): MemoryConceptDetail | null {
  const snapshot = graphVizFixtureSnapshot;
  const node = snapshot.nodes.find(
    (candidate) => candidate.kind === "concept"
      && (candidate.concept_id === conceptId || candidate.id === conceptId),
  );
  if (!node) return null;
  const nodesById = new Map(snapshot.nodes.map((candidate) => [candidate.id, candidate]));
  const linkTargets: Array<{ target: string; label?: string }> = [];
  const citations: MemoryConceptDetail["citations"] = [];
  const sourceRefs: MemoryConceptDetail["source_refs"] = [];
  for (const edge of snapshot.edges) {
    if (edge.source_id !== node.id && edge.target_id !== node.id) continue;
    const otherId = edge.source_id === node.id ? edge.target_id : edge.source_id;
    const other = nodesById.get(otherId);
    if (!other) continue;
    if (edge.kind === "markdown_link" && other.kind === "concept") {
      linkTargets.push({ target: other.concept_id ?? other.id, label: other.label });
    } else if (edge.kind === "cites" && other.kind === "concept") {
      citations.push({ id: `cite:${other.id}`, target: other.concept_id ?? other.id, label: other.label });
    } else if (edge.kind === "source_supported_by" && other.kind === "source_ref") {
      sourceRefs.push({ kind: "task", id: other.label, url: `https://example.invalid/tasks/${other.label}` });
    }
  }
  const areas = (node.frontmatter_summary.areas as string[] | undefined) ?? [];
  const related = linkTargets.slice(0, 4);
  const body = [
    `## Summary`,
    ``,
    `${node.label} — deterministic fixture capsule for drill-down and deep-link work. It plays the role of an issue memory capsule: frontmatter, sections, and links behave like the real vault content.`,
    ``,
    `## Decisions and actions`,
    ``,
    `- Anchored under ${areas.map((area) => `**${area}**`).join(", ") || "no area"} in the fixture graph.`,
    `- Tagged ${node.tags.map((tag) => `\`${tag}\``).join(", ") || "with nothing"}.`,
    ...(related.length > 0
      ? [
        ``,
        `## Relationships`,
        ``,
        ...related.map((link) => `- Links to [[${link.target}]] (${link.label ?? link.target})`),
      ]
      : []),
    ``,
    `## Validation evidence`,
    ``,
    `- Deterministic content: regenerating the fixture never changes this capsule.`,
  ].join("\n");
  return {
    schema_version,
    bundle_id: bundleId,
    concept_id: node.concept_id ?? node.id,
    frontmatter_view: {
      primary: { title: node.label, type: "issue-capsule" },
      opensymphony: {
        areas,
        state: node.freshness === "stale" ? "Stale" : "Done",
        visibility: node.visibility ?? "private",
      },
      unknown: {},
    },
    body_markdown: body,
    links: linkTargets,
    citations,
    source_refs: sourceRefs,
  };
}

// Code Graph shares this visualization workbench instead of introducing a
// second demo data source. Atlas is deliberately aggregate-only; the file and
// neighborhood snapshots are the scoped follow-up requests an Atlas drill-in
// would make against the real gateway.
const codeGraphSchemaVersion = { major: 1, minor: 0, patch: 0 };
const codeGraphCursor = { sequence: 7, partition: "code-graph:opensymphony" };

export const codeGraphFixtureRepos: CodeRepoList = {
  schema_version: codeGraphSchemaVersion,
  repos: [{
    repo_id: "opensymphony",
    display_root: "OpenSymphony",
    languages: ["rust", "typescript"],
    document_count: 42,
    symbol_count: 18,
    edge_count: 24,
    freshness: "current",
    indexed_at: "2026-07-04T00:00:00Z",
    head_revision: "head-rev",
    worktree_dirty: false,
  }],
};

const codeCommunity = {
  id: "dir:packages",
  label: "packages",
  node_ids: ["community:packages", "directory:packages/graph", "directory:packages/ui-core"],
  symbol_count: 0,
};

export const codeGraphFixtureSnapshots: CodeGraphSnapshot[] = [
  {
    schema_version: codeGraphSchemaVersion,
    repo_id: "opensymphony",
    mode: "atlas",
    cursor: codeGraphCursor,
    nodes: [
      codeNode("community:packages", "community", "packages", "typescript", "current", 0, "dir:packages"),
      codeNode("directory:packages/graph", "directory", "packages/graph", null, "current", 1, "dir:packages"),
      codeNode("directory:packages/ui-core", "directory", "packages/ui-core", null, "stale", 1, "dir:packages"),
    ],
    edges: [
      codeEdge("atlas:community-graph", "contains", "community:packages", "directory:packages/graph", "exact"),
      codeEdge("atlas:community-ui", "contains", "community:packages", "directory:packages/ui-core", "syntactic"),
    ],
    communities: [codeCommunity],
    truncation: { nodes_dropped: 13, edges_dropped: 18, reason: "directory aggregation" },
    filters_applied: ["aggregate:directory"],
    generated_at: "2026-07-04T00:00:00Z",
  },
  {
    schema_version: codeGraphSchemaVersion,
    repo_id: "opensymphony",
    mode: "file",
    cursor: { ...codeGraphCursor, sequence: 8 },
    nodes: [
      codeNode("file:packages/graph/src/index.ts", "file", "index.ts", "typescript", "current", 3, "dir:packages", undefined, "packages/graph/src/index.ts"),
      codeNode("symbol:graphReducer", "symbol", "graphReducer", "typescript", "current", 3, "dir:packages", "function", "packages/graph/src/index.ts", ["graph"]),
      codeNode("symbol:createHttpGraphAdapter", "symbol", "createHttpGraphAdapter", "typescript", "current", 2, "dir:packages", "function", "packages/graph/src/index.ts", ["adapters"]),
      codeNode("symbol:codeGraphReducer", "symbol", "codeGraphReducer", "typescript", "stale", 2, "dir:packages", "function", "packages/graph/src/code-graph.ts", ["code"]),
    ],
    edges: [
      codeEdge("file:contains-reducer", "contains", "file:packages/graph/src/index.ts", "symbol:graphReducer", "exact"),
      codeEdge("file:contains-adapter", "contains", "file:packages/graph/src/index.ts", "symbol:createHttpGraphAdapter", "exact"),
      codeEdge("file:imports-code", "references", "symbol:graphReducer", "symbol:codeGraphReducer", "syntactic"),
    ],
    communities: [codeCommunity],
    truncation: { nodes_dropped: 0, edges_dropped: 0, reason: null },
    filters_applied: ["mode:file"],
    generated_at: "2026-07-04T00:00:00Z",
  },
  {
    schema_version: codeGraphSchemaVersion,
    repo_id: "opensymphony",
    mode: "neighborhood",
    cursor: { ...codeGraphCursor, sequence: 9 },
    nodes: [
      codeNode("symbol:graphReducer", "symbol", "graphReducer", "typescript", "current", 3, "dir:packages", "function", "packages/graph/src/index.ts", ["graph"]),
      codeNode("symbol:codeGraphReducer", "symbol", "codeGraphReducer", "typescript", "current", 2, "dir:packages", "function", "packages/graph/src/code-graph.ts", ["code"]),
      codeNode("symbol:renderCodeGraphSurface", "symbol", "renderCodeGraphSurface", "typescript", "current", 1, "dir:packages", "function", "packages/ui-core/src/knowledge-graph-renderer.ts", ["renderer"]),
    ],
    edges: [
      codeEdge("neighborhood:reducer", "references", "symbol:graphReducer", "symbol:codeGraphReducer", "syntactic"),
      codeEdge("neighborhood:renderer", "calls", "symbol:codeGraphReducer", "symbol:renderCodeGraphSurface", "heuristic"),
    ],
    communities: [codeCommunity],
    truncation: { nodes_dropped: 0, edges_dropped: 0, reason: null },
    filters_applied: ["mode:neighborhood", "depth:1"],
    generated_at: "2026-07-04T00:00:00Z",
  },
];

export const codeGraphFixtureSymbolDetails: CodeSymbolDetail[] = [
  codeDetail("graphReducer", "packages/graph/src/index.ts", "typescript", ["graph"]),
  codeDetail("codeGraphReducer", "packages/graph/src/code-graph.ts", "typescript", ["code"]),
  codeDetail("renderCodeGraphSurface", "packages/ui-core/src/knowledge-graph-renderer.ts", "typescript", ["renderer"]),
];

export const codeGraphFixtureOutlines: CodeFileOutline[] = [{
  schema_version: codeGraphSchemaVersion,
  run_id: "run-code-fixture",
  repo_id: "opensymphony",
  path: "packages/graph/src/index.ts",
  symbols: codeGraphFixtureSymbolDetails.slice(0, 1).map((detail) => ({
    symbol_key: detail.symbol_key,
    name: detail.name,
    kind: detail.kind,
    path: detail.path_display,
    span: detail.span,
    selection_span: detail.selection_span,
    container_chain: detail.container_chain,
  })),
  generated_at: "2026-07-04T00:00:00Z",
}];

export const codeGraphFixtureDiffOverlays: CodeDiffOverlay[] = [{
  schema_version: codeGraphSchemaVersion,
  repo_id: "opensymphony",
  base_revision: "base-rev",
  head_revision: "head-rev",
  added_symbols: [{
    symbol_key: "newSymbol",
    status: "added",
    before: null,
    after: codeDiffSide("newSymbol", "packages/graph/src/new.ts", "current"),
  }],
  removed_symbols: [{
    symbol_key: "legacySymbol",
    status: "removed",
    before: codeDiffSide("legacySymbol", "packages/graph/src/legacy.ts", "stale"),
    after: null,
  }],
  modified_symbols: [{
    symbol_key: "codeGraphReducer",
    status: "modified",
    before: codeDiffSide("codeGraphReducer", "packages/graph/src/code-graph.ts", "stale"),
    after: codeDiffSide("codeGraphReducer", "packages/graph/src/code-graph.ts", "current"),
  }],
  blast_radius: [{ symbol_key: "graphReducer", inbound_count: 2, outbound_count: 1 }],
  unanalyzed_files: [],
  truncation: { nodes_dropped: 0, edges_dropped: 0, reason: null },
  generated_at: "2026-07-04T00:00:00Z",
}];

/**
 * Fixture tiers mirror the Code Graph spec's edge-heavy renderer budgets.
 * Generate them on demand so the normal desktop fixture stays small.
 */
export const codeGraphScaleTiers = [
  { name: "neighborhood-500", nodes: 500, edges: 2_000 },
  { name: "neighborhood-5k", nodes: 5_000, edges: 20_000 },
  { name: "neighborhood-20k", nodes: 20_000, edges: 80_000 },
] as const;

export function createCodeGraphScaleFixture(
  nodeCount: number,
  edgesPerNode = 4,
): CodeGraphSnapshot {
  const nodes = Array.from({ length: nodeCount }, (_, index) =>
    codeNode(
      `symbol:scale-${index}`,
      "symbol",
      `scale-${index}`,
      "rust",
      "current",
      edgesPerNode,
      "dir:scale",
      "function",
      `src/scale_${Math.floor(index / 100)}.rs`,
      ["scale"],
    ),
  );
  const edges = Array.from({ length: nodeCount * edgesPerNode }, (_, index) => {
    const source = Math.floor(index / edgesPerNode);
    const target = (source + (index % edgesPerNode) + 1) % nodeCount;
    return codeEdge(
      `scale:${source}:${target}:${index % edgesPerNode}`,
      "calls",
      `symbol:scale-${source}`,
      `symbol:scale-${target}`,
      index % 3 === 0 ? "exact" : index % 3 === 1 ? "syntactic" : "heuristic",
    );
  });
  return {
    schema_version: codeGraphSchemaVersion,
    repo_id: "scale-fixture",
    mode: "neighborhood",
    cursor: { ...codeGraphCursor, sequence: nodeCount },
    nodes,
    edges,
    communities: [{ id: "dir:scale", label: "scale", node_ids: nodes.map((node) => node.id), symbol_count: nodeCount }],
    truncation: { nodes_dropped: 0, edges_dropped: 0, reason: null },
    filters_applied: ["fixture:edge-heavy"],
    generated_at: generated_at,
  };
}

/** Aggregated Atlas reference scale: 50K symbols/200K edges, <=2K total nodes and edges. */
export function createCodeGraphReferenceAtlasFixture(): CodeGraphSnapshot {
  const renderedElementCap = 2_000;
  const nodeCount = renderedElementCap / 2;
  const nodes = Array.from({ length: nodeCount }, (_, index) =>
    codeNode(
      `directory:reference-${index}`,
      "directory",
      `src/reference-${index}`,
      null,
      "current",
      1,
      "community:reference",
      undefined,
      `src/reference-${index}`,
    ),
  );
  const edges = nodes.slice(1).map((node, index) =>
    codeEdge(`reference:${index}`, "contains", nodes[index]!.id, node.id, "exact"),
  );
  return {
    schema_version: codeGraphSchemaVersion,
    repo_id: "reference-scale",
    mode: "atlas",
    cursor: { ...codeGraphCursor, sequence: 50_000 },
    nodes,
    edges,
    communities: [{ id: "community:reference", label: "Reference repository", node_ids: nodes.map((node) => node.id), symbol_count: 50_000 }],
    truncation: { nodes_dropped: 49_000, edges_dropped: 199_001, reason: "directory aggregation" },
    filters_applied: ["aggregate:directory", "fixture:reference-scale"],
    generated_at: generated_at,
  };
}

const diffNeighborhood = codeGraphFixtureSnapshots.find((snapshot) => snapshot.mode === "neighborhood")!;
export const codeGraphFixtureDiffBaseSnapshot: CodeGraphSnapshot = {
  ...diffNeighborhood,
  cursor: { ...diffNeighborhood.cursor, sequence: 10 },
  nodes: [
    ...diffNeighborhood.nodes,
    codeNode("symbol:legacySymbol", "symbol", "legacySymbol", "typescript", "stale", 1, "dir:packages", "function", "packages/graph/src/legacy.ts", ["legacy"]),
  ],
  filters_applied: ["mode:neighborhood", "fixture:diff-base"],
};

export const codeGraphFixtureDiffHeadSnapshot: CodeGraphSnapshot = {
  ...diffNeighborhood,
  cursor: { ...diffNeighborhood.cursor, sequence: 11 },
  nodes: [
    ...diffNeighborhood.nodes,
    codeNode("symbol:newSymbol", "symbol", "newSymbol", "typescript", "current", 1, "dir:packages", "function", "packages/graph/src/new.ts", ["new"]),
  ],
  filters_applied: ["mode:neighborhood", "fixture:diff-head"],
};

function codeNode(
  id: string,
  kind: CodeGraphSnapshot["nodes"][number]["kind"],
  label: string,
  language: string | null,
  freshness: CodeGraphSnapshot["nodes"][number]["freshness"],
  degree: number,
  communityId: string,
  symbolKind?: string,
  pathDisplay?: string,
  containerChain: string[] = [],
): CodeGraphSnapshot["nodes"][number] {
  return {
    id,
    kind,
    label,
    symbol_kind: symbolKind ?? null,
    symbol_key: kind === "symbol" ? id.replace("symbol:", "") : null,
    symbol_id: kind === "symbol" ? `${id}:id` : null,
    path_display: pathDisplay ?? (kind === "directory" ? label : null),
    language,
    container_chain: containerChain,
    signature: kind === "symbol" ? `${symbolKind ?? "symbol"} ${label}()` : null,
    span: kind === "symbol" ? { start_line: 10, start_col: 1, end_line: 30, end_col: 2 } : null,
    selection_span: kind === "symbol" ? { start_line: 10, start_col: 1, end_line: 10, end_col: 20 } : null,
    freshness,
    diagnostic_count: freshness === "stale" ? 1 : 0,
    diagnostic_severity: freshness === "stale" ? "warning" : null,
    metrics: { in_degree: degree, out_degree: degree, community_id: communityId },
  };
}

function codeEdge(id: string, kind: string, sourceId: string, targetId: string, confidence: CodeGraphSnapshot["edges"][number]["confidence"]): CodeGraphSnapshot["edges"][number] {
  return { id, kind, source_id: sourceId, target_id: targetId, confidence, unresolved: false, target_hint: null };
}

function codeDetail(symbolKey: string, path: string, language: string, containerChain: string[]): CodeSymbolDetail {
  return {
    schema_version: codeGraphSchemaVersion,
    repo_id: "opensymphony",
    symbol_key: symbolKey,
    symbol_id: `${symbolKey}:id`,
    kind: "function",
    name: symbolKey,
    path_display: path,
    language,
    container_chain: containerChain,
    signature: `function ${symbolKey}(): CodeGraphState`,
    span: { start_line: 10, start_col: 1, end_line: 30, end_col: 2 },
    selection_span: { start_line: 10, start_col: 1, end_line: 10, end_col: 20 },
    freshness: "current",
    provenance: {
      commit_sha: "head-rev",
      content_sha256: `${symbolKey}-content`,
      snippet_sha256: `${symbolKey}-snippet`,
      parser_version: "tree-sitter-typescript",
      query_pack_version: "typescript-query-pack-v1",
      indexed_at: "2026-07-04T00:00:00Z",
    },
    diagnostics: [],
    edge_summary: [{ kind: "references", confidence: "syntactic", count: 1, unresolved_count: 0 }],
    source_snippet: null,
  };
}

function codeDiffSide(symbolKey: string, path: string, freshness: "current" | "stale"): CodeDiffOverlay["modified_symbols"][number]["before"] {
  return {
    symbol_id: `${symbolKey}:id`,
    kind: "function",
    name: symbolKey,
    path_display: path,
    container_chain: ["code"],
    span: { start_line: 10, start_col: 1, end_line: 30, end_col: 2 },
    freshness,
  };
}
