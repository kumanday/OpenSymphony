import type {
  MemoryBundleList,
  MemoryCommunityList,
  MemoryConceptDetail,
  MemoryGraphSnapshot,
  MemorySearchResponse,
} from "@opensymphony/gateway-schema";

const schema_version = { major: 1, minor: 0, patch: 0 };
const generated_at = "2026-06-28T00:00:00Z";

export const fixtureBundleList: MemoryBundleList = {
  schema_version,
  bundles: [
    {
      id: "local-default",
      title: "OpenSymphony Memory",
      okf_version: "0.1",
      visibility: "private",
      concept_count: 3,
      updated_at: generated_at,
    },
  ],
};

export const fixtureGraphSnapshot: MemoryGraphSnapshot = {
  schema_version,
  bundle_id: "local-default",
  cursor: { sequence: 1, partition: "memory-graph:local-default" },
  generated_at,
  filters_applied: [],
  communities: [
    {
      id: "area:graph-view",
      label: "Graph View",
      node_ids: ["concept:coe-465", "tag:graph-view"],
      concept_count: 1,
    },
  ],
  nodes: [
    {
      id: "bundle:local-default",
      kind: "bundle",
      label: "OpenSymphony Memory",
      bundle_id: "local-default",
      tags: [],
      visibility: "private",
      freshness: "current",
      warning_count: 0,
      frontmatter_summary: {},
      unknown_frontmatter: {},
      metrics: { indegree: 0, outdegree: 2 },
    },
    {
      id: "concept:coe-465",
      kind: "concept",
      label: "COE-465 Shared Graph Frontend Package And Reducers",
      bundle_id: "local-default",
      concept_id: "issues/COE-465",
      concept_type: "issue",
      path_display: "issues/COE-465.md",
      tags: ["graph-view", "frontend"],
      timestamp: generated_at,
      visibility: "private",
      freshness: "current",
      warning_count: 0,
      frontmatter_summary: {
        area: "graph-view",
        project: "OpenSymphony-bootstrap",
        milestone: "M11.5",
        issue: "COE-465",
        repository: "OpenSymphony",
      },
      unknown_frontmatter: {},
      body_preview: "Shared frontend graph package used by web and desktop clients.",
      metrics: { indegree: 1, outdegree: 2, community_id: "area:graph-view" },
    },
    {
      id: "tag:graph-view",
      kind: "tag",
      label: "graph-view",
      bundle_id: "local-default",
      tags: ["graph-view"],
      visibility: "private",
      freshness: "current",
      warning_count: 0,
      frontmatter_summary: {},
      unknown_frontmatter: {},
      metrics: { indegree: 1, outdegree: 0, community_id: "area:graph-view" },
    },
    {
      id: "source:osym-822",
      kind: "source_ref",
      label: "OSYM-822",
      bundle_id: "local-default",
      tags: [],
      visibility: "private",
      freshness: "current",
      warning_count: 0,
      frontmatter_summary: { source_kind: "task" },
      unknown_frontmatter: {},
      metrics: { indegree: 1, outdegree: 0 },
    },
  ],
  edges: [
    {
      id: "edge:bundle-local-default:concept-coe-465",
      kind: "contains",
      source_id: "bundle:local-default",
      target_id: "concept:coe-465",
      unresolved: false,
      metadata: {},
    },
    {
      id: "edge:concept-coe-465:tag-graph-view",
      kind: "tagged_with",
      source_id: "concept:coe-465",
      target_id: "tag:graph-view",
      unresolved: false,
      metadata: {},
    },
    {
      id: "edge:concept-coe-465:source-osym-822",
      kind: "source_supported_by",
      source_id: "concept:coe-465",
      target_id: "source:osym-822",
      unresolved: false,
      metadata: { source_kind: "task" },
    },
  ],
};

export const fixtureCommunityList: MemoryCommunityList = {
  schema_version,
  bundle_id: fixtureGraphSnapshot.bundle_id,
  generated_at,
  communities: fixtureGraphSnapshot.communities,
};

export const fixtureConceptDetail: MemoryConceptDetail = {
  schema_version,
  bundle_id: fixtureGraphSnapshot.bundle_id,
  concept_id: "issues/COE-465",
  frontmatter_view: {
    primary: { title: "Shared Graph Frontend Package And Reducers" },
    opensymphony: { issue: "COE-465", milestone: "M11.5" },
    unknown: {},
  },
  body_markdown: "# COE-465\n\nShared graph frontend package and reducers.",
  links: [{ target: "tag:graph-view", label: "graph-view" }],
  citations: [],
  source_refs: [{ kind: "task", id: "OSYM-822" }],
};

export const fixtureSearchResponse: MemorySearchResponse = {
  schema_version,
  query: "graph",
  bundle_id: fixtureGraphSnapshot.bundle_id,
  results: [
    {
      bundle_id: fixtureGraphSnapshot.bundle_id,
      concept_id: "issues/COE-465",
      title: "COE-465 Shared Graph Frontend Package And Reducers",
      visibility: "private",
      snippet: "Shared frontend graph package used by web and desktop clients.",
      areas: ["graph-view"],
    },
  ],
};
