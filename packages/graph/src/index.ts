import type {
  MemoryBundleList,
  MemoryCommunityList,
  MemoryConceptDetail,
  MemoryGraphEdge,
  MemoryGraphEdgeKind,
  MemoryGraphFreshness,
  MemoryGraphNode,
  MemoryGraphNodeKind,
  MemoryGraphSnapshot,
  MemoryGraphVisibility,
  MemorySearchResponse,
  MemorySearchResult,
} from "@opensymphony/gateway-schema";
import {
  fixtureBundleList,
  fixtureCommunityList,
  fixtureConceptDetail,
  fixtureGraphSnapshot,
  fixtureSearchResponse,
} from "./fixture.js";

export type {
  MemoryBundleList,
  MemoryCommunityList,
  MemoryConceptDetail,
  MemoryGraphSnapshot,
  MemorySearchResponse,
} from "@opensymphony/gateway-schema";
export {
  fixtureBundleList,
  fixtureCommunityList,
  fixtureConceptDetail,
  fixtureGraphSnapshot,
  fixtureSearchResponse,
} from "./fixture.js";

export type GraphMode =
  | "atlas"
  | "bundle"
  | "community"
  | "neighborhood"
  | "timeline"
  | "evidence";

export type LayoutStatus = "idle" | "loading" | "stabilizing" | "ready" | "failed";

export interface GraphFilters {
  bundleIds: string[];
  nodeKinds: MemoryGraphNodeKind[];
  tags: string[];
  areas: string[];
  projects: string[];
  milestones: string[];
  issues: string[];
  repositories: string[];
  visibility: MemoryGraphVisibility[];
  freshness: MemoryGraphFreshness[];
  warning: "all" | "with_warnings" | "without_warnings";
  sourceKinds: string[];
  edgeKinds: MemoryGraphEdgeKind[];
  communities: string[];
}

export interface GraphDeepLinkState {
  mode: GraphMode;
  bundleId: string | null;
  focusedNodeId: string | null;
  selectedNodeIds: string[];
  searchQuery: string;
  filters: GraphFilters;
  neighborhoodDepth: number;
}

export interface GraphState {
  bundles: MemoryBundleList | null;
  snapshots: Record<string, MemoryGraphSnapshot>;
  conceptDetails: Record<string, MemoryConceptDetail>;
  communities: Record<string, MemoryCommunityList>;
  mode: GraphMode;
  selectedBundleId: string | null;
  focusedNodeId: string | null;
  selectedNodeIds: string[];
  searchQuery: string;
  searchResults: MemorySearchResult[];
  filters: GraphFilters;
  layoutStatus: LayoutStatus;
  layoutError: string | null;
  neighborhoodDepth: number;
  lastUpdatedAt: string | null;
}

export type GraphAction =
  | { type: "BUNDLES_LOADED"; bundles: MemoryBundleList }
  | { type: "SNAPSHOT_LOADED"; snapshot: MemoryGraphSnapshot }
  | { type: "CONCEPT_DETAIL_LOADED"; detail: MemoryConceptDetail }
  | { type: "COMMUNITIES_LOADED"; communities: MemoryCommunityList }
  | { type: "MODE_SET"; mode: GraphMode }
  | { type: "BUNDLE_SELECTED"; bundleId: string | null }
  | { type: "COMMUNITY_SELECTED"; communityId: string | null }
  | { type: "NODE_FOCUSED"; nodeId: string | null; neighborhoodDepth?: number }
  | { type: "SELECTION_SET"; nodeIds: string[] }
  | { type: "FILTERS_SET"; filters: Partial<GraphFilters> }
  | { type: "FILTERS_RESET" }
  | { type: "SEARCH_SET"; query: string }
  | { type: "SEARCH_RESULTS_LOADED"; response: MemorySearchResponse }
  | { type: "LAYOUT_STATUS_SET"; status: LayoutStatus; error?: string | null }
  | { type: "HISTORY_RESTORED"; state: Partial<GraphDeepLinkState> }
  | { type: "GRAPH_RESET" };

export interface GraphDataAdapter {
  listBundles(): Promise<MemoryBundleList>;
  getGraphSnapshot(bundleId: string, options?: GraphRequestOptions): Promise<MemoryGraphSnapshot>;
  getConceptDetail(bundleId: string, conceptId: string, options?: GraphRequestOptions): Promise<MemoryConceptDetail>;
  getCommunities(bundleId: string, options?: GraphRequestOptions): Promise<MemoryCommunityList>;
  search(query: string, options?: GraphSearchOptions): Promise<MemorySearchResponse>;
}

export interface GraphRequestOptions {
  visibility?: MemoryGraphVisibility | "all_accessible";
}

export interface GraphSearchOptions extends GraphRequestOptions {
  limit?: number;
  bundleId?: string;
}

/** Tauri/native providers expose the same graph DTO contract without importing Tauri here. */
export type NativeGraphApi = GraphDataAdapter;

export const graphModes: readonly GraphMode[] = [
  "atlas",
  "bundle",
  "community",
  "neighborhood",
  "timeline",
  "evidence",
];

export const initialGraphFilters: GraphFilters = createInitialGraphFilters();
export const initialGraphState: GraphState = createInitialGraphState();

export function createInitialGraphFilters(): GraphFilters {
  return {
    bundleIds: [],
    nodeKinds: [],
    tags: [],
    areas: [],
    projects: [],
    milestones: [],
    issues: [],
    repositories: [],
    visibility: [],
    freshness: [],
    warning: "all",
    sourceKinds: [],
    edgeKinds: [],
    communities: [],
  };
}

export function createInitialGraphState(): GraphState {
  return {
    bundles: null,
    snapshots: {},
    conceptDetails: {},
    communities: {},
    mode: "atlas",
    selectedBundleId: null,
    focusedNodeId: null,
    selectedNodeIds: [],
    searchQuery: "",
    searchResults: [],
    filters: createInitialGraphFilters(),
    layoutStatus: "idle",
    layoutError: null,
    neighborhoodDepth: 1,
    lastUpdatedAt: null,
  };
}

export function graphReducer(state: GraphState, action: GraphAction): GraphState {
  switch (action.type) {
    case "BUNDLES_LOADED":
      return {
        ...state,
        bundles: action.bundles,
        selectedBundleId: state.selectedBundleId ?? action.bundles.bundles[0]?.id ?? null,
        lastUpdatedAt: latestBundleTimestamp(action.bundles),
      };
    case "SNAPSHOT_LOADED":
      return {
        ...state,
        snapshots: { ...state.snapshots, [action.snapshot.bundle_id]: action.snapshot },
        selectedBundleId: state.selectedBundleId ?? action.snapshot.bundle_id,
        lastUpdatedAt: action.snapshot.generated_at,
      };
    case "CONCEPT_DETAIL_LOADED":
      return {
        ...state,
        conceptDetails: {
          ...state.conceptDetails,
          [conceptDetailKey(action.detail.bundle_id, action.detail.concept_id)]: action.detail,
        },
      };
    case "COMMUNITIES_LOADED":
      return {
        ...state,
        communities: { ...state.communities, [action.communities.bundle_id]: action.communities },
      };
    case "MODE_SET":
      return { ...state, mode: action.mode };
    case "BUNDLE_SELECTED":
      return { ...state, selectedBundleId: action.bundleId, focusedNodeId: null, selectedNodeIds: [] };
    case "COMMUNITY_SELECTED":
      return graphReducer(
        { ...state, mode: "community" },
        { type: "FILTERS_SET", filters: { communities: action.communityId ? [action.communityId] : [] } },
      );
    case "NODE_FOCUSED":
      return {
        ...state,
        mode: action.nodeId ? "neighborhood" : state.mode,
        focusedNodeId: action.nodeId,
        selectedNodeIds: action.nodeId ? uniqueSorted([...state.selectedNodeIds, action.nodeId]) : state.selectedNodeIds,
        neighborhoodDepth: action.neighborhoodDepth ?? state.neighborhoodDepth,
      };
    case "SELECTION_SET":
      return { ...state, selectedNodeIds: uniqueSorted(action.nodeIds) };
    case "FILTERS_SET":
      return { ...state, filters: normalizeFilters({ ...state.filters, ...action.filters }) };
    case "FILTERS_RESET":
      return { ...state, filters: createInitialGraphFilters() };
    case "SEARCH_SET":
      return { ...state, searchQuery: normalizeQuery(action.query) };
    case "SEARCH_RESULTS_LOADED":
      return {
        ...state,
        searchQuery: normalizeQuery(action.response.query),
        searchResults: [...action.response.results].sort(compareSearchResults),
      };
    case "LAYOUT_STATUS_SET":
      return { ...state, layoutStatus: action.status, layoutError: action.error ?? null };
    case "HISTORY_RESTORED": {
      const restored = action.state;
      return {
        ...state,
        mode: restored.mode ?? state.mode,
        selectedBundleId: restored.bundleId === undefined ? state.selectedBundleId : restored.bundleId,
        focusedNodeId: restored.focusedNodeId === undefined ? state.focusedNodeId : restored.focusedNodeId,
        selectedNodeIds: uniqueSorted(restored.selectedNodeIds === undefined ? state.selectedNodeIds : restored.selectedNodeIds),
        searchQuery: normalizeQuery(restored.searchQuery === undefined ? state.searchQuery : restored.searchQuery),
        filters: normalizeFilters(restored.filters === undefined ? state.filters : restored.filters),
        neighborhoodDepth: restored.neighborhoodDepth ?? state.neighborhoodDepth,
      };
    }
    case "GRAPH_RESET":
      return createInitialGraphState();
    default:
      return state;
  }
}

export function currentGraphSnapshot(state: GraphState): MemoryGraphSnapshot | null {
  const bundleId = state.selectedBundleId;
  return bundleId ? state.snapshots[bundleId] ?? null : null;
}

export function visibleGraphSnapshot(state: GraphState): MemoryGraphSnapshot | null {
  const snapshot = currentGraphSnapshot(state);
  if (!snapshot) return null;
  return applyGraphFilters(snapshot, state.filters, state.mode, state.focusedNodeId, state.neighborhoodDepth);
}

export function applyGraphFilters(
  snapshot: MemoryGraphSnapshot,
  filters: GraphFilters,
  mode: GraphMode = "atlas",
  focusedNodeId: string | null = null,
  neighborhoodDepth = 1,
): MemoryGraphSnapshot {
  const normalized = normalizeFilters(filters);
  const neighborhood = mode === "neighborhood" && focusedNodeId
    ? collectNeighborhood(snapshot.edges, focusedNodeId, neighborhoodDepth)
    : null;
  const nodes = snapshot.nodes.filter((node) => {
    if (neighborhood && !neighborhood.has(node.id)) return false;
    return matchesNodeFilters(node, normalized);
  });
  const nodeKindById = new Map(nodes.map((node) => [node.id, node.kind]));
  const edges = snapshot.edges.filter((edge) => {
    if (!nodeKindById.has(edge.source_id) || !nodeKindById.has(edge.target_id)) return false;
    if (normalized.edgeKinds.length > 0 && !normalized.edgeKinds.includes(edge.kind)) return false;
    return true;
  });
  const communities = snapshot.communities
    .map((community) => ({
      ...community,
      node_ids: community.node_ids.filter((nodeId) => nodeKindById.has(nodeId)).sort(),
      concept_count: community.node_ids.filter((nodeId) => nodeKindById.get(nodeId) === "concept").length,
    }))
    .filter((community) => community.node_ids.length > 0)
    .sort((a, b) => a.id.localeCompare(b.id));
  const neighborhoodTokens = neighborhood
    ? [`neighborhood:${focusedNodeId}`, `neighborhood-depth:${neighborhoodDepth}`]
    : [];
  return {
    ...snapshot,
    nodes: [...nodes].sort(compareNodes),
    edges: [...edges].sort(compareEdges),
    communities,
    filters_applied: [...filterTokens(normalized), ...neighborhoodTokens].sort(),
  };
}

export function searchGraphSnapshot(
  snapshot: MemoryGraphSnapshot,
  query: string,
  filters: GraphFilters = initialGraphFilters,
): MemorySearchResult[] {
  const needle = normalizeQuery(query).toLowerCase();
  if (!needle) return [];
  return applyGraphFilters(snapshot, filters)
    .nodes.map((node) => ({ node, score: scoreNode(node, needle) }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || compareNodes(a.node, b.node))
    .map(({ node }) => ({
      bundle_id: node.bundle_id ?? snapshot.bundle_id,
      concept_id: node.concept_id ?? node.id,
      title: node.label,
      visibility: node.visibility ?? "private",
      snippet: node.body_preview ?? node.description ?? node.path_display ?? node.label,
      areas: valuesFor(node, "area"),
    }));
}

export function graphStateToHistory(state: GraphState): GraphDeepLinkState {
  return {
    mode: state.mode,
    bundleId: state.selectedBundleId,
    focusedNodeId: state.focusedNodeId,
    selectedNodeIds: uniqueSorted(state.selectedNodeIds),
    searchQuery: normalizeQuery(state.searchQuery),
    filters: normalizeFilters(state.filters),
    neighborhoodDepth: state.neighborhoodDepth,
  };
}

export function createHttpGraphAdapter(baseUri: string, fetchFn: typeof fetch = globalThis.fetch): GraphDataAdapter {
  const base = baseUri.replace(/\/+$/, "");
  async function read<T>(path: string, params: URLSearchParams = new URLSearchParams()): Promise<T> {
    const query = params.toString();
    const response = await fetchFn(`${base}${path}${query ? `?${query}` : ""}`);
    if (!response.ok) throw new Error(`Graph request failed: HTTP ${response.status}`);
    return await response.json() as T;
  }
  return {
    listBundles: () => read<MemoryBundleList>("/api/v1/memory/bundles"),
    getGraphSnapshot: (bundleId, options) =>
      read<MemoryGraphSnapshot>(`/api/v1/memory/bundles/${encodeURIComponent(bundleId)}/graph`, visibilityParams(options)),
    getConceptDetail: (bundleId, conceptId, options) =>
      read<MemoryConceptDetail>(
        `/api/v1/memory/bundles/${encodeURIComponent(bundleId)}/concepts/${encodeURIComponent(conceptId)}`,
        visibilityParams(options),
      ),
    getCommunities: (bundleId, options) =>
      read<MemoryCommunityList>(
        `/api/v1/memory/bundles/${encodeURIComponent(bundleId)}/communities`,
        visibilityParams(options),
      ),
    search: (query, options) => {
      const params = visibilityParams(options);
      params.set("query", query);
      if (options?.limit !== undefined) params.set("limit", String(options.limit));
      if (options?.bundleId) params.set("bundle_id", options.bundleId);
      return read<MemorySearchResponse>("/api/v1/memory/search", params);
    },
  };
}

export const createGatewayGraphAdapter = createHttpGraphAdapter;
export const createMemoryServerGraphAdapter = createHttpGraphAdapter;

export function createTauriNativeGraphAdapter(api: NativeGraphApi): GraphDataAdapter {
  return api;
}

export function createFixtureGraphAdapter(fixtures: {
  bundles?: MemoryBundleList;
  snapshot?: MemoryGraphSnapshot;
  conceptDetail?: MemoryConceptDetail;
  communities?: MemoryCommunityList;
  search?: MemorySearchResponse;
} = {}): GraphDataAdapter {
  const bundles = fixtures.bundles ?? fixtureBundleList;
  const snapshot = fixtures.snapshot ?? fixtureGraphSnapshot;
  const conceptDetail = fixtures.conceptDetail ?? fixtureConceptDetail;
  const communities = fixtures.communities ?? fixtureCommunityList;
  const search = fixtures.search ?? fixtureSearchResponse;
  return {
    listBundles: async () => bundles,
    getGraphSnapshot: async () => snapshot,
    getConceptDetail: async () => conceptDetail,
    getCommunities: async () => communities,
    search: async (query, options) => {
      const results = options?.bundleId && options.bundleId !== snapshot.bundle_id
        ? []
        : query === search.query
          ? search.results
          : searchGraphSnapshot(snapshot, query);
      return {
        ...search,
        query,
        bundle_id: options?.bundleId ?? search.bundle_id,
        results: results.slice(0, options?.limit ?? results.length),
      };
    },
  };
}

function normalizeFilters(filters: GraphFilters): GraphFilters {
  return {
    bundleIds: uniqueSorted(filters.bundleIds),
    nodeKinds: uniqueSorted(filters.nodeKinds),
    tags: uniqueSorted(filters.tags),
    areas: uniqueSorted(filters.areas),
    projects: uniqueSorted(filters.projects),
    milestones: uniqueSorted(filters.milestones),
    issues: uniqueSorted(filters.issues),
    repositories: uniqueSorted(filters.repositories),
    visibility: uniqueSorted(filters.visibility),
    freshness: uniqueSorted(filters.freshness),
    warning: filters.warning,
    sourceKinds: uniqueSorted(filters.sourceKinds),
    edgeKinds: uniqueSorted(filters.edgeKinds),
    communities: uniqueSorted(filters.communities),
  };
}

function matchesNodeFilters(node: MemoryGraphNode, filters: GraphFilters): boolean {
  if (filters.bundleIds.length > 0 && (!node.bundle_id || !filters.bundleIds.includes(node.bundle_id))) return false;
  if (filters.nodeKinds.length > 0 && !filters.nodeKinds.includes(node.kind)) return false;
  if (filters.tags.length > 0 && !filters.tags.some((tag) => node.tags.includes(tag))) return false;
  if (filters.visibility.length > 0 && (!node.visibility || !filters.visibility.includes(node.visibility))) return false;
  if (filters.freshness.length > 0 && (!node.freshness || !filters.freshness.includes(node.freshness))) return false;
  if (filters.warning === "with_warnings" && node.warning_count <= 0) return false;
  if (filters.warning === "without_warnings" && node.warning_count > 0) return false;
  if (filters.communities.length > 0 && (!node.metrics.community_id || !filters.communities.includes(node.metrics.community_id))) return false;
  if (filters.areas.length > 0 && !hasAny(node, "area", filters.areas)) return false;
  if (filters.projects.length > 0 && !hasAny(node, "project", filters.projects)) return false;
  if (filters.milestones.length > 0 && !hasAny(node, "milestone", filters.milestones)) return false;
  if (filters.issues.length > 0 && !hasAny(node, "issue", filters.issues)) return false;
  if (filters.repositories.length > 0 && !hasAny(node, "repository", filters.repositories)) return false;
  if (filters.sourceKinds.length > 0 && !hasAny(node, "source_kind", filters.sourceKinds)) return false;
  return true;
}

function hasAny(node: MemoryGraphNode, key: string, values: string[]): boolean {
  const got = valuesFor(node, key);
  return values.some((value) => got.includes(value));
}

function valuesFor(node: MemoryGraphNode, key: string): string[] {
  const values = [
    node.frontmatter_summary[key],
    node.frontmatter_summary[`${key}s`],
    node.unknown_frontmatter[key],
    node.unknown_frontmatter[`${key}s`],
  ];
  return values.flatMap(stringValues).sort();
}

function stringValues(value: unknown): string[] {
  if (typeof value === "string" && value) return [value];
  if (Array.isArray(value)) return value.flatMap(stringValues);
  return [];
}

function collectNeighborhood(
  edges: readonly MemoryGraphEdge[],
  rootId: string,
  depth: number,
): Set<string> {
  const seen = new Set([rootId]);
  let frontier = new Set([rootId]);
  for (let step = 0; step < Math.max(0, depth); step++) {
    const next: string[] = [];
    for (const edge of edges) {
      if (frontier.has(edge.source_id) && !seen.has(edge.target_id)) next.push(edge.target_id);
      if (frontier.has(edge.target_id) && !seen.has(edge.source_id)) next.push(edge.source_id);
    }
    for (const id of next) seen.add(id);
    frontier = new Set(next);
  }
  return seen;
}

function filterTokens(filters: GraphFilters): string[] {
  return [
    ...filters.bundleIds.map((v) => `bundle:${v}`),
    ...filters.nodeKinds.map((v) => `kind:${v}`),
    ...filters.tags.map((v) => `tag:${v}`),
    ...filters.areas.map((v) => `area:${v}`),
    ...filters.projects.map((v) => `project:${v}`),
    ...filters.milestones.map((v) => `milestone:${v}`),
    ...filters.issues.map((v) => `issue:${v}`),
    ...filters.repositories.map((v) => `repository:${v}`),
    ...filters.visibility.map((v) => `visibility:${v}`),
    ...filters.freshness.map((v) => `freshness:${v}`),
    ...(filters.warning === "all" ? [] : [`warning:${filters.warning}`]),
    ...filters.sourceKinds.map((v) => `source:${v}`),
    ...filters.edgeKinds.map((v) => `edge:${v}`),
    ...filters.communities.map((v) => `community:${v}`),
  ].sort();
}

function scoreNode(node: MemoryGraphNode, needle: string): number {
  let score = 0;
  for (const value of [node.label, node.concept_id, node.path_display, node.description, node.body_preview]) {
    const text = value?.toLowerCase();
    if (text === needle) score += 100;
    else if (text?.includes(needle)) score += 20;
  }
  for (const tag of node.tags) {
    if (tag.toLowerCase() === needle) score += 30;
    else if (tag.toLowerCase().includes(needle)) score += 10;
  }
  return score;
}

function visibilityParams(options?: GraphRequestOptions): URLSearchParams {
  const params = new URLSearchParams();
  if (options?.visibility) params.set("visibility", options.visibility);
  return params;
}

function conceptDetailKey(bundleId: string, conceptId: string): string {
  return `${bundleId}:${conceptId}`;
}

function latestBundleTimestamp(list: MemoryBundleList): string | null {
  return [...list.bundles].map((bundle) => bundle.updated_at).filter(Boolean).sort().at(-1) ?? null;
}

function normalizeQuery(query: string): string {
  return query.trim().replace(/\s+/g, " ");
}

function compareStrings(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

function compareNodes(a: MemoryGraphNode, b: MemoryGraphNode): number {
  return compareStrings(a.label, b.label) || compareStrings(a.id, b.id);
}

function compareEdges(a: MemoryGraphEdge, b: MemoryGraphEdge): number {
  return compareStrings(a.kind, b.kind) || compareStrings(a.source_id, b.source_id) || compareStrings(a.target_id, b.target_id) || compareStrings(a.id, b.id);
}

function compareSearchResults(a: MemorySearchResult, b: MemorySearchResult): number {
  return compareStrings(a.title, b.title) || compareStrings(a.bundle_id, b.bundle_id) || compareStrings(a.concept_id, b.concept_id);
}

function uniqueSorted<T extends string>(values: readonly T[]): T[] {
  return [...new Set(values)].sort();
}
