import type {
  MemoryBundleList,
  MemoryCommunityList,
  MemoryConceptDetail,
  MemoryGraphEdge,
  MemoryGraphNode,
  MemoryGraphSnapshot,
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
