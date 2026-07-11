import type {
  MemoryBundleList,
  MemoryCommunityList,
  MemoryCompletedTask,
  MemoryCompletedTaskPage,
  MemoryConceptDetail,
  MemoryGraphEdge,
  MemoryGraphEdgeKind,
  MemoryGraphFreshness,
  MemoryGraphNode,
  MemoryGraphNodeKind,
  MemoryGraphSnapshot,
  MemoryGraphUpdatedEvent,
  MemoryGraphVisibility,
  MemorySearchResponse,
  MemorySearchResult,
} from "@opensymphony/gateway-schema";
import {
  createScaleGraphSnapshot,
  fixtureBundleList,
  fixtureCommunityList,
  fixtureConceptDetail,
  fixtureGraphSnapshot,
  fixtureSearchResponse,
} from "./fixture.js";

export type {
  CodeDiffOverlay,
  CodeFileOutline,
  CodeGraphNode,
  CodeGraphSnapshot,
  CodeRepoList,
  CodeSymbolDetail,
  MemoryBundleList,
  MemoryCommunityList,
  MemoryCompletedTask,
  MemoryCompletedTaskPage,
  MemoryConceptDetail,
  MemoryGraphNode,
  MemoryGraphSnapshot,
  MemorySearchResponse,
  MemoryTaskPullRequest,
} from "@opensymphony/gateway-schema";
export {
  fixtureBundleList,
  fixtureCommunityList,
  fixtureConceptDetail,
  fixtureGraphSnapshot,
  fixtureSearchResponse,
  createScaleGraphSnapshot,
} from "./fixture.js";
export {
  graphVizFixtureBundleList,
  graphVizFixtureCommunityList,
  graphVizFixtureCompletedTasks,
  graphVizFixtureConceptDetail,
  graphVizFixtureSnapshot,
  codeGraphFixtureDiffOverlays,
  codeGraphFixtureOutlines,
  codeGraphFixtureRepos,
  codeGraphFixtureSnapshots,
  codeGraphFixtureSymbolDetails,
} from "./viz-fixture.js";
export {
  applyCodeGraphFilters,
  codeEdgeVisualStyle,
  codeGraphFilterTokens,
  codeGraphLayoutKindForMode,
  codeGraphNodeDeltaStatus,
  codeGraphSnapshotForRendering,
  codeGraphStateToHistory,
  codeGraphReducer,
  codeNodeVisualStyle,
  createFixtureCodeGraphAdapter,
  createCodeGraphFixtureAdapter,
  createGatewayCodeGraphAdapter,
  createHttpCodeGraphAdapter,
  createInitialCodeGraphFilters,
  createInitialCodeGraphState,
  createTauriNativeCodeGraphAdapter,
  currentCodeGraphSnapshot,
  initialCodeGraphFilters,
  initialCodeGraphState,
  normalizeCodeGraphFilters,
  type CodeEdgeVisualStyle,
  type CodeGraphAction,
  type CodeGraphAdapter,
  type CodeGraphBreadcrumb,
  type CodeGraphDeltaStatus,
  type CodeGraphDiffOptions,
  type CodeGraphFilters,
  type CodeGraphFixtures,
  type CodeGraphHistoryState,
  type CodeGraphMode,
  type CodeGraphRequestOptions,
  type CodeGraphState,
  type CodeNodeVisualStyle,
  type NativeCodeGraphApi,
} from "./code-graph.js";
export {
  codeDeepLinkForFile,
  codeDeepLinkForSymbol,
  codeDeepLinkPrefix,
  codeDeepLinkToGraphState,
  formatCodeDeepLink,
  formatMemoryDeepLink,
  memoryDeepLinkForGraphNode,
  memoryDeepLinkPrefix,
  memoryDeepLinkToGraphState,
  parseCodeDeepLink,
  parseMemoryDeepLink,
  resolveMemoryDeepLinkNode,
  type CodeDeepLink,
  type MemoryDeepLink,
} from "./deep-link.js";

export type GraphMode =
  | "atlas"
  | "bundle"
  | "community"
  | "neighborhood"
  | "timeline"
  | "evidence";

export type LayoutStatus = "idle" | "loading" | "stabilizing" | "ready" | "failed";
export type GraphFreshnessStatus = "current" | "stale" | "warning";

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
  freshnessStatus: GraphFreshnessStatus;
  staleBundleIds: string[];
  staleCursors: Record<string, MemoryGraphSnapshot["cursor"]>;
  warningBundleIds: string[];
  /**
   * Cached concept-detail keys whose bundle advanced to a newer snapshot.
   * The details stay in `conceptDetails` so the open capsule keeps rendering,
   * but callers refetch stale entries in the background and swap the fresh
   * result in atomically — dropping them outright blanked the inspector (and
   * reset its scroll) on every snapshot tick while a capsule was open.
   */
  staleConceptDetailKeys: string[];
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
  | { type: "GRAPH_UPDATED"; event: MemoryGraphUpdatedEvent }
  | { type: "HISTORY_RESTORED"; state: Partial<GraphDeepLinkState> }
  | { type: "GRAPH_RESET" };

export interface GraphDataAdapter {
  listBundles(): Promise<MemoryBundleList>;
  getGraphSnapshot(bundleId: string, options?: GraphRequestOptions): Promise<MemoryGraphSnapshot>;
  getConceptDetail(bundleId: string, conceptId: string, options?: GraphRequestOptions): Promise<MemoryConceptDetail>;
  getCommunities(bundleId: string, options?: GraphRequestOptions): Promise<MemoryCommunityList>;
  search(query: string, options?: GraphSearchOptions): Promise<MemorySearchResponse>;
  /**
   * Paginated completed tasks (memory capsules with PR evidence, merged
   * with orchestrator-known completions). Optional so lean adapters keep
   * working; surfaces without it render an empty Completed pane.
   */
  getCompletedTasks?(options?: GraphCompletedTasksOptions): Promise<MemoryCompletedTaskPage>;
}

export interface GraphRequestOptions {
  visibility?: MemoryGraphVisibility | "all_accessible";
}

export interface GraphSearchOptions extends GraphRequestOptions {
  limit?: number;
  bundleId?: string;
}

export interface GraphCompletedTasksOptions extends GraphRequestOptions {
  query?: string;
  sort?: string;
  limit?: number;
  offset?: number;
}

export interface GraphAdapterPolicy {
  defaultVisibility?: MemoryGraphVisibility | "all_accessible";
  maxVisibility?: MemoryGraphVisibility | "all_accessible";
}

export type GraphLayoutKind = "force" | "hierarchical" | "radial" | "timeline";

export const graphOverviewNodeThreshold = 10_000;

export interface GraphLayoutOptions {
  kind: GraphLayoutKind;
  width?: number;
  height?: number;
  focusedNodeId?: string | null;
}

export interface GraphLayoutNode {
  nodeId: string;
  x: number;
  y: number;
  z: number;
  radius: number;
  label: string;
  kind: string;
  communityId?: string;
  freshness?: string;
  diagnosticCount?: number;
  symbolKind?: string;
}

export interface GraphLayoutEdge {
  edgeId: string;
  sourceId: string;
  targetId: string;
  kind?: string;
  confidence?: string;
}

export interface GraphLayoutResult {
  kind: GraphLayoutKind;
  width: number;
  height: number;
  nodes: GraphLayoutNode[];
  edges: GraphLayoutEdge[];
  generatedAt: string;
}

export interface GraphLayoutAdapter {
  layout(snapshot: MemoryGraphSnapshot, options: GraphLayoutOptions): Promise<GraphLayoutResult>;
  dispose(): void;
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
    freshnessStatus: "current",
    staleBundleIds: [],
    staleCursors: {},
    warningBundleIds: [],
    staleConceptDetailKeys: [],
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
      {
        const staleCursor = state.staleCursors[action.snapshot.bundle_id];
        const existingSnapshot = state.snapshots[action.snapshot.bundle_id];
        // Same-partition snapshots must be strictly newer to replace the
        // current one: polls frequently redeliver the identical sequence,
        // and treating those as fresh data forced a full re-layout (and
        // reset hover/zoom) every refresh cycle.
        const isOlderSamePartitionSnapshot = existingSnapshot !== undefined
          && existingSnapshot.cursor.partition === action.snapshot.cursor.partition
          && action.snapshot.cursor.sequence <= existingSnapshot.cursor.sequence;
        const isStaleSnapshot = (staleCursor !== undefined && isCursorBefore(action.snapshot.cursor, staleCursor))
          || isOlderSamePartitionSnapshot;
        if (isStaleSnapshot) {
          return state;
        }
        const staleBundleIds = state.staleBundleIds.filter((bundleId) => bundleId !== action.snapshot.bundle_id);
        const staleCursors = { ...state.staleCursors };
        delete staleCursors[action.snapshot.bundle_id];
        const warningBundleIds = action.snapshot.metrics && action.snapshot.metrics.warning_count > 0
          ? uniqueSorted([...state.warningBundleIds, action.snapshot.bundle_id])
          : state.warningBundleIds.filter((bundleId) => bundleId !== action.snapshot.bundle_id);
        // An accepted (strictly newer) snapshot may reflect capsule edits.
        // Keep the bundle's cached capsules on screen but mark them stale, so
        // an open capsule refetches in the background and swaps in atomically
        // rather than blanking the inspector (and resetting its scroll) on
        // every snapshot tick.
        const bundlePrefix = `${action.snapshot.bundle_id}:`;
        const staleConceptDetailKeys = uniqueSorted([
          ...state.staleConceptDetailKeys,
          ...Object.keys(state.conceptDetails).filter((key) => key.startsWith(bundlePrefix)),
        ]);
        return {
          ...state,
          staleConceptDetailKeys,
          snapshots: { ...state.snapshots, [action.snapshot.bundle_id]: action.snapshot },
          selectedBundleId: state.selectedBundleId ?? action.snapshot.bundle_id,
          lastUpdatedAt: action.snapshot.generated_at,
          staleBundleIds,
          staleCursors,
          warningBundleIds,
          freshnessStatus: graphFreshnessStatus(staleBundleIds, warningBundleIds),
        };
      }
    case "CONCEPT_DETAIL_LOADED":
      {
        const key = conceptDetailKey(action.detail.bundle_id, action.detail.concept_id);
        return {
          ...state,
          conceptDetails: {
            ...state.conceptDetails,
            [key]: action.detail,
          },
          staleConceptDetailKeys: state.staleConceptDetailKeys.filter((staleKey) => staleKey !== key),
        };
      }
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
        neighborhoodDepth: action.neighborhoodDepth === undefined ? state.neighborhoodDepth : action.neighborhoodDepth,
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
    case "GRAPH_UPDATED": {
      const current = state.snapshots[action.event.bundle_id];
      if (current && !isCursorAfter(action.event.cursor, current.cursor)) {
        return state;
      }
      const staleBundleIds = uniqueSorted([...state.staleBundleIds, action.event.bundle_id]);
      return {
        ...state,
        lastUpdatedAt: action.event.updated_at,
        staleBundleIds,
        staleCursors: {
          ...state.staleCursors,
          [action.event.bundle_id]: action.event.cursor,
        },
        freshnessStatus: graphFreshnessStatus(staleBundleIds, state.warningBundleIds),
      };
    }
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
        neighborhoodDepth: restored.neighborhoodDepth === undefined ? state.neighborhoodDepth : restored.neighborhoodDepth,
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

/**
 * True when two snapshots have the same graph topology — identical node and
 * edge id sets for the same bundle. Node positions depend only on topology, so
 * this is the granularity that decides whether a re-layout is warranted;
 * cursor/timestamp churn and body/label edits (which the capsule fetch handles
 * separately) do not move nodes and must not trigger one.
 */
export function sameGraphTopology(
  a: MemoryGraphSnapshot | null,
  b: MemoryGraphSnapshot | null,
): boolean {
  if (!a || !b) return false;
  if (a.bundle_id !== b.bundle_id) return false;
  if (a.nodes.length !== b.nodes.length || a.edges.length !== b.edges.length) return false;
  const sameIds = (left: readonly { id: string }[], right: readonly { id: string }[]): boolean => {
    const leftIds = left.map((item) => item.id).sort();
    const rightIds = right.map((item) => item.id).sort();
    return leftIds.every((id, index) => id === rightIds[index]);
  };
  return sameIds(a.nodes, b.nodes) && sameIds(a.edges, b.edges);
}

/** Cached capsule detail for a concept, or null until CONCEPT_DETAIL_LOADED lands. */
export function cachedConceptDetail(
  state: GraphState,
  bundleId: string,
  conceptId: string,
): MemoryConceptDetail | null {
  return state.conceptDetails[conceptDetailKey(bundleId, conceptId)] ?? null;
}

/**
 * True when a cached concept detail is still displayable but its bundle has
 * advanced to a newer snapshot, so callers should refetch it in the
 * background and swap the fresh result in once it lands.
 */
export function isConceptDetailStale(
  state: GraphState,
  bundleId: string,
  conceptId: string,
): boolean {
  return state.staleConceptDetailKeys.includes(conceptDetailKey(bundleId, conceptId));
}

export function visibleGraphSnapshot(state: GraphState): MemoryGraphSnapshot | null {
  const snapshot = currentGraphSnapshot(state);
  if (!snapshot) return null;
  const visible = applyGraphFilters(snapshot, state.filters, state.mode, state.focusedNodeId, state.neighborhoodDepth);
  if (visible.nodes.length < graphOverviewNodeThreshold) return visible;
  return createCommunityOverviewSnapshot(visible);
}

export function createCommunityOverviewSnapshot(snapshot: MemoryGraphSnapshot): MemoryGraphSnapshot {
  const bundleNodes = snapshot.nodes.filter((node) => node.kind === "bundle");
  if (bundleNodes.length !== 1 || bundleNodes[0]?.bundle_id !== snapshot.bundle_id) {
    throw new Error("Community overview requires a single bundle node matching snapshot.bundle_id");
  }
  const bundle = bundleNodes[0];
  const nodesById = new Map(snapshot.nodes.map((node) => [node.id, node]));
  const normalizedCommunities = snapshot.communities
    .map((community) => {
      const nodeIds = community.node_ids.filter((nodeId) => nodesById.has(nodeId)).sort();
      return {
        ...community,
        node_ids: nodeIds,
        concept_count: nodeIds.filter((nodeId) => nodesById.get(nodeId)?.kind === "concept").length,
      };
    })
    .filter((community) => community.node_ids.length > 0)
    .sort((a, b) => compareStrings(a.id, b.id));
  const communityNodes: MemoryGraphNode[] = normalizedCommunities.map((community) => {
    const members = community.node_ids.map((nodeId) => nodesById.get(nodeId)).filter((node): node is MemoryGraphNode => Boolean(node));
    const warningCount = members.reduce((sum, node) => sum + node.warning_count, 0);
    return {
      id: community.id,
      kind: "community",
      label: community.label,
      bundle_id: snapshot.bundle_id,
      tags: uniqueSorted(members.flatMap((node) => node.tags)),
      visibility: mostCommon(members.map((node) => node.visibility).filter((value): value is MemoryGraphVisibility => Boolean(value)))
        ?? bundle?.visibility,
      freshness: members.some((node) => node.freshness === "stale") ? "stale" : bundle?.freshness ?? "current",
      warning_count: warningCount,
      frontmatter_summary: { concept_count: community.concept_count },
      unknown_frontmatter: {},
      body_preview: `${community.concept_count} concepts`,
      metrics: { indegree: bundle ? 1 : 0, outdegree: 0, community_id: community.id },
    };
  });
  const nodes = [...(bundle ? [bundle] : []), ...communityNodes].sort(compareNodes);
  const edges: MemoryGraphEdge[] = bundle
    ? communityNodes.map((node) => ({
      id: `edge:${bundle.id}:${node.id}`,
      kind: "contains",
      source_id: bundle.id,
      target_id: node.id,
      unresolved: false,
      metadata: { aggregation: "community" },
    }))
    : [];
  const { metrics: _metrics, ...rest } = snapshot;
  return {
    ...rest,
    nodes,
    edges,
    communities: normalizedCommunities,
    filters_applied: uniqueSorted([...snapshot.filters_applied, "overview:community-aggregation"]),
  };
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
  // Communities are membership lists, not just primary assignments: a
  // multi-area concept carries one community in metrics but appears in every
  // area's node_ids. Drilling into an area must keep those secondary members
  // (they are part of the hull the operator clicked).
  const communityMembers = normalized.communities.length > 0
    ? new Set(
      snapshot.communities
        .filter((community) => normalized.communities.includes(community.id))
        .flatMap((community) => community.node_ids),
    )
    : null;
  const nodes = snapshot.nodes.filter((node) => {
    if (neighborhood && !neighborhood.has(node.id)) return false;
    return matchesNodeFilters(node, normalized, communityMembers);
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
    .sort((a, b) => compareStrings(a.id, b.id));
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

export function createHttpGraphAdapter(
  baseUri: string,
  fetchFn: typeof fetch = globalThis.fetch,
  policy: GraphAdapterPolicy = {},
): GraphDataAdapter {
  const base = baseUri.replace(/\/+$/, "");
  async function read<T>(path: string, params: URLSearchParams = new URLSearchParams()): Promise<T> {
    const query = params.toString();
    const response = await fetchFn(`${base}${path}${query ? `?${query}` : ""}`);
    if (!response.ok) throw new Error(`Graph request failed: HTTP ${response.status}`);
    return await response.json() as T;
  }
  return {
    listBundles: () => read<MemoryBundleList>("/api/v1/memory/bundles", visibilityParams(undefined, policy)),
    getGraphSnapshot: (bundleId, options) =>
      read<MemoryGraphSnapshot>(`/api/v1/memory/bundles/${encodeURIComponent(bundleId)}/graph`, visibilityParams(options, policy)),
    getConceptDetail: (bundleId, conceptId, options) =>
      read<MemoryConceptDetail>(
        `/api/v1/memory/bundles/${encodeURIComponent(bundleId)}/concepts/${encodeURIComponent(conceptId)}`,
        visibilityParams(options, policy),
      ),
    getCommunities: (bundleId, options) =>
      read<MemoryCommunityList>(
        `/api/v1/memory/bundles/${encodeURIComponent(bundleId)}/communities`,
        visibilityParams(options, policy),
      ),
    search: (query, options) => {
      const params = visibilityParams(options, policy);
      params.set("query", query);
      if (options?.limit !== undefined) params.set("limit", String(options.limit));
      if (options?.bundleId) params.set("bundle_id", options.bundleId);
      return read<MemorySearchResponse>("/api/v1/memory/search", params);
    },
    getCompletedTasks: (options) => {
      const params = visibilityParams(options, policy);
      if (options?.query) params.set("query", options.query);
      if (options?.sort) params.set("sort", options.sort);
      if (options?.limit !== undefined) params.set("limit", String(options.limit));
      if (options?.offset !== undefined) params.set("offset", String(options.offset));
      return read<MemoryCompletedTaskPage>("/api/v1/memory/completed-tasks", params);
    },
  };
}

/**
 * Client-side twin of the gateway's completed-tasks search/sort/pagination,
 * used by the fixture adapter (and tests) so the workbench behaves like the
 * real endpoint.
 */
export function pageCompletedTasks(
  rows: readonly MemoryCompletedTask[],
  options: GraphCompletedTasksOptions = {},
): MemoryCompletedTaskPage {
  const query = options.query?.trim() ?? "";
  const needle = query.toLowerCase();
  const filtered = needle
    ? rows.filter((row) =>
      row.issue_key.toLowerCase().includes(needle)
      || row.title.toLowerCase().includes(needle)
      || (row.state?.toLowerCase().includes(needle) ?? false)
      || (row.milestone?.toLowerCase().includes(needle) ?? false)
      || row.prs.some((pr) => pr.title.toLowerCase().includes(needle) || `#${pr.number}`.includes(needle)))
    : [...rows];

  const sort = normalizeCompletedTasksSort(options.sort);
  filtered.sort(completedTaskComparator(sort));

  const total = filtered.length;
  const limit = Math.min(Math.max(options.limit ?? 25, 1), 100);
  const offset = Math.min(Math.max(options.offset ?? 0, 0), total);
  return {
    schema_version: { major: 1, minor: 0, patch: 0 },
    bundle_id: "local-default",
    tasks: filtered.slice(offset, offset + limit),
    total,
    offset,
    limit,
    sort,
    query: query || undefined,
    generated_at: new Date().toISOString(),
  };
}

export const completedTaskSorts = [
  "completed_desc",
  "completed_asc",
  "id_asc",
  "id_desc",
  "title_asc",
  "title_desc",
  "pr_desc",
  "pr_asc",
] as const;
export type CompletedTaskSort = (typeof completedTaskSorts)[number];

function normalizeCompletedTasksSort(sort: string | undefined): CompletedTaskSort {
  return (completedTaskSorts as readonly string[]).includes(sort ?? "")
    ? sort as CompletedTaskSort
    : "completed_desc";
}

function completedTaskComparator(sort: CompletedTaskSort): (a: MemoryCompletedTask, b: MemoryCompletedTask) => number {
  switch (sort) {
    case "completed_asc":
      return (a, b) => missingDatesLast(a.completed_at, b.completed_at)
        || compareNullableStrings(a.completed_at, b.completed_at)
        || compareIssueKeys(a.issue_key, b.issue_key);
    case "id_asc":
      return (a, b) => compareIssueKeys(a.issue_key, b.issue_key);
    case "id_desc":
      return (a, b) => compareIssueKeys(b.issue_key, a.issue_key);
    case "title_asc":
      return (a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: "base" });
    case "title_desc":
      return (a, b) => b.title.localeCompare(a.title, undefined, { sensitivity: "base" });
    case "pr_desc":
      return (a, b) => latestPrNumber(b) - latestPrNumber(a);
    case "pr_asc":
      return (a, b) => latestPrNumber(a) - latestPrNumber(b);
    default:
      return (a, b) => missingDatesLast(a.completed_at, b.completed_at)
        || compareNullableStrings(b.completed_at, a.completed_at)
        || compareIssueKeys(b.issue_key, a.issue_key);
  }
}

/** Rows without a completion date sort after dated rows in either direction. */
function missingDatesLast(a: string | undefined, b: string | undefined): number {
  if ((a === undefined) === (b === undefined)) return 0;
  return a === undefined ? 1 : -1;
}

function latestPrNumber(row: MemoryCompletedTask): number {
  return row.prs.reduce((max, pr) => Math.max(max, pr.number), 0);
}

function compareNullableStrings(a: string | undefined, b: string | undefined): number {
  if (a === b) return 0;
  if (a === undefined) return 1;
  if (b === undefined) return -1;
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Natural compare so `COE-99` orders before `COE-100`. */
function compareIssueKeys(a: string, b: string): number {
  const splitKey = (key: string): [string, number] => {
    const at = Math.max(key.lastIndexOf("-"), key.lastIndexOf("_"));
    if (at < 0) return [key.toUpperCase(), 0];
    const numeric = Number.parseInt(key.slice(at + 1), 10);
    return [key.slice(0, at).toUpperCase(), Number.isNaN(numeric) ? 0 : numeric];
  };
  const [prefixA, numberA] = splitKey(a);
  const [prefixB, numberB] = splitKey(b);
  return prefixA.localeCompare(prefixB) || numberA - numberB;
}

export const createGatewayGraphAdapter = createHttpGraphAdapter;
export const createMemoryServerGraphAdapter = createHttpGraphAdapter;

export function graphLayoutKindForMode(mode: GraphMode): GraphLayoutKind {
  switch (mode) {
    case "bundle":
      return "hierarchical";
    case "neighborhood":
      return "radial";
    case "timeline":
      return "timeline";
    default:
      return "force";
  }
}

export function computeGraphLayout(
  snapshot: MemoryGraphSnapshot,
  options: GraphLayoutOptions,
): GraphLayoutResult {
  const width = Math.max(320, options.width ?? 720);
  const height = Math.max(240, options.height ?? 420);
  const nodeIds = new Set(snapshot.nodes.map((node) => node.id));
  const edges = snapshot.edges.filter((edge) => nodeIds.has(edge.source_id) && nodeIds.has(edge.target_id));
  const nodes = options.kind === "hierarchical"
    ? hierarchicalLayout(snapshot.nodes, width, height)
    : options.kind === "radial"
      ? radialLayout(snapshot.nodes, edges, width, height, options.focusedNodeId)
      : options.kind === "timeline"
        ? timelineLayout(snapshot.nodes, width, height)
        : forceLayout(snapshot.nodes, edges, width, height);
  return {
    kind: options.kind,
    width,
    height,
    nodes,
    edges: edges.map((edge) => ({
      edgeId: edge.id,
      sourceId: edge.source_id,
      targetId: edge.target_id,
      kind: edge.kind,
      confidence: typeof edge.metadata?.confidence === "string" ? edge.metadata.confidence : undefined,
    })),
    generatedAt: new Date().toISOString(),
  };
}

export function createGraphLayoutAdapter(
  workerFactory: () => Worker | null = defaultGraphLayoutWorkerFactory,
): GraphLayoutAdapter {
  const worker = workerFactory();
  if (!worker) {
    return {
      layout: async (snapshot, options) => computeGraphLayout(snapshot, options),
      dispose: () => undefined,
    };
  }
  let nextId = 1;
  const pending = new Map<number, {
    resolve: (result: GraphLayoutResult) => void;
    reject: (error: Error) => void;
    timeout: ReturnType<typeof setTimeout>;
  }>();
  const workerTimeoutMs = 2_000;
  worker.onmessage = (event: MessageEvent<{ id: number; result?: GraphLayoutResult; error?: string }>) => {
    const request = pending.get(event.data.id);
    if (!request) return;
    pending.delete(event.data.id);
    clearTimeout(request.timeout);
    if (event.data.error) {
      request.reject(new Error(event.data.error));
      return;
    }
    if (event.data.result) request.resolve(event.data.result);
  };
  worker.onerror = (event) => {
    for (const request of pending.values()) {
      clearTimeout(request.timeout);
      request.reject(new Error(event.message || "Graph layout worker failed"));
    }
    pending.clear();
  };
  return {
    layout: (snapshot, options) => new Promise((resolve, reject) => {
      const id = nextId++;
      const timeout = setTimeout(() => {
        if (!pending.has(id)) return;
        pending.delete(id);
        console.warn("Graph layout worker timed out; falling back to synchronous layout computation.");
        resolve(computeGraphLayout(snapshot, options));
      }, workerTimeoutMs);
      pending.set(id, { resolve, reject, timeout });
      worker.postMessage({ id, snapshot, options });
    }),
    dispose: () => {
      for (const request of pending.values()) {
        clearTimeout(request.timeout);
        request.reject(new Error("Graph layout worker disposed before completing layout."));
      }
      pending.clear();
      worker.terminate();
    },
  };
}

export function createTauriNativeGraphAdapter(api: NativeGraphApi): GraphDataAdapter {
  return api;
}

export function createFixtureGraphAdapter(fixtures: {
  bundles?: MemoryBundleList;
  snapshot?: MemoryGraphSnapshot;
  /** Static detail for every concept, or a resolver (null → reject like a gateway 404). */
  conceptDetail?: MemoryConceptDetail | ((bundleId: string, conceptId: string) => MemoryConceptDetail | null);
  communities?: MemoryCommunityList;
  search?: MemorySearchResponse;
  /** Full row set; the adapter applies query/sort/pagination like the gateway. */
  completedTasks?: readonly MemoryCompletedTask[];
} = {}): GraphDataAdapter {
  const bundles = fixtures.bundles ?? fixtureBundleList;
  const snapshot = fixtures.snapshot ?? fixtureGraphSnapshot;
  const conceptDetail = fixtures.conceptDetail ?? fixtureConceptDetail;
  const communities = fixtures.communities ?? fixtureCommunityList;
  const search = fixtures.search ?? fixtureSearchResponse;
  const completedTasks = fixtures.completedTasks ?? [];
  return {
    getCompletedTasks: async (options) => pageCompletedTasks(completedTasks, options),
    listBundles: async () => bundles,
    getGraphSnapshot: async () => snapshot,
    getConceptDetail: async (bundleId, conceptId) => {
      const detail = typeof conceptDetail === "function"
        ? conceptDetail(bundleId, conceptId)
        : conceptDetail;
      if (!detail) throw new Error(`Concept not found: ${conceptId}`);
      return detail;
    },
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

function defaultGraphLayoutWorkerFactory(): Worker | null {
  return null;
}

function forceLayout(
  nodes: readonly MemoryGraphNode[],
  edges: readonly MemoryGraphEdge[],
  width: number,
  height: number,
): GraphLayoutNode[] {
  if (nodes.length > 400) return progressiveCommunityLayout(nodes, width, height);
  if (nodes.length === 0) return [];
  const nodesById = new Map(nodes.map((node) => [node.id, node]));
  const tickCount = nodes.length > 160 ? 140 : 220;
  // Fruchterman–Reingold ideal spacing: nodes should spread over the whole
  // canvas instead of contracting into a center clump, which made zoomed-out
  // views an unreadable label pile.
  const k = 0.6 * Math.sqrt((width * height) / nodes.length);
  const communityById = new Map(nodes.map((node) => [node.id, node.metrics?.community_id]));
  // Communities get fixed anchors on an ellipse so clusters occupy distinct
  // regions; without this, cross-community edges shuffle every community
  // across the whole canvas and cluster hulls all overlap.
  const communityIds = [...new Set(
    nodes.map((node) => node.metrics?.community_id).filter((id): id is string => Boolean(id)),
  )].sort(compareStrings);
  const communityAnchors = new Map<string, { x: number; y: number }>(
    communityIds.map((communityId, index) => {
      const angle = (index / Math.max(1, communityIds.length)) * Math.PI * 2 - Math.PI / 2;
      return [communityId, {
        x: width / 2 + Math.cos(angle) * width * 0.26,
        y: height / 2 + Math.sin(angle) * height * 0.26,
      }];
    }),
  );
  // Deterministic golden-angle spiral seed around each node's community
  // anchor: clusters start separated so the simulated annealing only has to
  // refine locally instead of untangling an interleaved global spiral.
  const seedCounters = new Map<string, number>();
  const points = nodes.map((node) => {
    const communityId = node.metrics?.community_id ?? "";
    const anchor = communityAnchors.get(communityId) ?? { x: width / 2, y: height / 2 };
    const seedIndex = seedCounters.get(communityId) ?? 0;
    seedCounters.set(communityId, seedIndex + 1);
    const angle = seedIndex * 2.399963; // golden angle
    const radius = 8 + Math.sqrt(seedIndex) * k * 0.42;
    return {
      id: node.id,
      x: clamp(anchor.x + Math.cos(angle) * radius, 28, width - 28),
      y: clamp(anchor.y + Math.sin(angle) * radius, 28, height - 28),
      vx: 0,
      vy: 0,
    };
  });
  const byId = new Map(points.map((point) => [point.id, point]));
  let temperature = Math.max(width, height) / 10;
  const cooling = 0.975;
  for (let tick = 0; tick < tickCount; tick += 1) {
    const repulsionRange = k * 3;
    for (let i = 0; i < points.length; i += 1) {
      for (let j = i + 1; j < points.length; j += 1) {
        const a = points[i];
        const b = points[j];
        const dx = a.x - b.x || 0.01;
        const dy = a.y - b.y || 0.01;
        const distance = Math.max(6, Math.hypot(dx, dy));
        // Short-range repulsion only: distant pairs contribute nothing, so
        // the summed outward pressure cannot overpower the community
        // anchors and pin every node against the canvas walls.
        if (distance > repulsionRange) continue;
        const push = (k * k) / distance / distance;
        a.vx += dx * push;
        a.vy += dy * push;
        b.vx -= dx * push;
        b.vy -= dy * push;
      }
    }
    for (const edge of edges) {
      const source = byId.get(edge.source_id);
      const target = byId.get(edge.target_id);
      if (!source || !target) continue;
      const dx = target.x - source.x;
      const dy = target.y - source.y;
      const distance = Math.max(6, Math.hypot(dx, dy));
      // Same-community edges pull a little harder so clusters condense
      // around their areas while distinct areas stay apart.
      const sameCommunity = communityById.get(edge.source_id) !== undefined
        && communityById.get(edge.source_id) === communityById.get(edge.target_id);
      const pull = (distance * distance) / k * (sameCommunity ? 0.0016 : 0.0006);
      source.vx += (dx / distance) * pull;
      source.vy += (dy / distance) * pull;
      target.vx -= (dx / distance) * pull;
      target.vy -= (dy / distance) * pull;
    }
    for (const point of points) {
      const anchor = communityAnchors.get(communityById.get(point.id) ?? "");
      if (anchor) {
        point.vx += (anchor.x - point.x) * 0.05;
        point.vy += (anchor.y - point.y) * 0.05;
      } else {
        point.vx += (width / 2 - point.x) * 0.0012;
        point.vy += (height / 2 - point.y) * 0.0012;
      }
      const speed = Math.hypot(point.vx, point.vy);
      const limited = speed > temperature ? temperature / speed : 1;
      point.x = clamp(point.x + point.vx * limited, 28, width - 28);
      point.y = clamp(point.y + point.vy * limited, 28, height - 28);
      point.vx = 0;
      point.vy = 0;
    }
    temperature = Math.max(2.5, temperature * cooling);
  }
  return points.map((point) => {
    const node = nodesById.get(point.id)!;
    return layoutNode(node, point.x, point.y, zFor(node));
  }).sort(compareLayoutNodes);
}

function progressiveCommunityLayout(
  nodes: readonly MemoryGraphNode[],
  width: number,
  height: number,
): GraphLayoutNode[] {
  // ponytail: unknown-community nodes, including bundles, share coarse kind cells until real data needs finer grouping.
  const groups = [...groupBy(nodes, (node) => node.metrics?.community_id ?? `kind:${node.kind}`).entries()]
    .sort(([a], [b]) => compareStrings(a, b));
  const columns = Math.max(1, Math.ceil(Math.sqrt(groups.length)));
  const cellWidth = width / columns;
  const rows = Math.max(1, Math.ceil(groups.length / columns));
  const cellHeight = height / rows;
  return groups.flatMap(([_key, group], groupIndex) => {
    const column = groupIndex % columns;
    const row = Math.floor(groupIndex / columns);
    const sorted = [...group].sort(compareNodes);
    const innerColumns = Math.max(1, Math.ceil(Math.sqrt(sorted.length)));
    return sorted.map((node, nodeIndex) => {
      const innerColumn = nodeIndex % innerColumns;
      const innerRow = Math.floor(nodeIndex / innerColumns);
      const innerRows = Math.max(1, Math.ceil(sorted.length / innerColumns));
      return layoutNode(
        node,
        column * cellWidth + ((innerColumn + 1) / (innerColumns + 1)) * cellWidth,
        row * cellHeight + ((innerRow + 1) / (innerRows + 1)) * cellHeight,
        zFor(node),
      );
    });
  }).sort(compareLayoutNodes);
}

function hierarchicalLayout(
  nodes: readonly MemoryGraphNode[],
  width: number,
  height: number,
): GraphLayoutNode[] {
  const levels = new Map<string, number>([
    ["bundle", 0],
    ["directory", 1],
    ["concept", 2],
    ["tag", 3],
    ["resource", 3],
    ["citation", 4],
    ["source_ref", 4],
  ]);
  const groups = groupBy(nodes, (node) => String(levels.get(node.kind) ?? 2));
  const levelKeys = [...groups.keys()].sort((a, b) => Number(a) - Number(b));
  return levelKeys.flatMap((levelKey, levelIndex) => {
    const group = [...(groups.get(levelKey) ?? [])].sort(compareNodes);
    return group.map((node, index) => layoutNode(
      node,
      ((levelIndex + 1) / (levelKeys.length + 1)) * width,
      ((index + 1) / (group.length + 1)) * height,
      zFor(node),
    ));
  }).sort(compareLayoutNodes);
}

function radialLayout(
  nodes: readonly MemoryGraphNode[],
  edges: readonly MemoryGraphEdge[],
  width: number,
  height: number,
  focusedNodeId?: string | null,
): GraphLayoutNode[] {
  const focus = focusedNodeId && nodes.some((node) => node.id === focusedNodeId)
    ? focusedNodeId
    : nodes[0]?.id ?? null;
  if (!focus) return [];
  const distances = graphDistances(edges, focus);
  const rings = groupBy(nodes, (node) => String(distances.get(node.id) ?? 2));
  return [...rings.entries()].flatMap(([ringKey, ringNodes]) => {
    const ring = Number(ringKey);
    const sorted = [...ringNodes].sort(compareNodes);
    const radius = ring === 0 ? 0 : Math.min(width, height) * (0.18 + ring * 0.13);
    return sorted.map((node, index) => {
      const angle = sorted.length === 1 ? -Math.PI / 2 : (Math.PI * 2 * index) / sorted.length - Math.PI / 2;
      return layoutNode(
        node,
        width / 2 + Math.cos(angle) * radius,
        height / 2 + Math.sin(angle) * radius,
        zFor(node),
      );
    });
  }).sort(compareLayoutNodes);
}

function timelineLayout(
  nodes: readonly MemoryGraphNode[],
  width: number,
  height: number,
): GraphLayoutNode[] {
  const sorted = [...nodes].sort((a, b) =>
    compareTimelineNodes(a, b) || compareNodes(a, b)
  );
  const lanes = groupBy(sorted, (node) => node.kind);
  const laneKeys = [...lanes.keys()].sort(compareStrings);
  return sorted.map((node, index) => {
    const lane = Math.max(0, laneKeys.indexOf(node.kind));
    return layoutNode(
      node,
      ((index + 1) / (sorted.length + 1)) * width,
      ((lane + 1) / (laneKeys.length + 1)) * height,
      zFor(node),
    );
  }).sort(compareLayoutNodes);
}

function compareTimelineNodes(a: MemoryGraphNode, b: MemoryGraphNode): number {
  const aTime = timestampMillis(a.timestamp);
  const bTime = timestampMillis(b.timestamp);
  if (aTime !== null && bTime !== null && aTime !== bTime) return aTime - bTime;
  if (aTime !== null && bTime === null) return -1;
  if (aTime === null && bTime !== null) return 1;
  return compareStrings(a.timestamp ?? a.label, b.timestamp ?? b.label);
}

function timestampMillis(timestamp: string | undefined): number | null {
  if (!timestamp) return null;
  const value = Date.parse(timestamp);
  return Number.isFinite(value) ? value : null;
}

function layoutNode(node: MemoryGraphNode, x: number, y: number, z: number): GraphLayoutNode {
  return {
    nodeId: node.id,
    x,
    y,
    z,
    radius: node.kind === "concept" || (node.kind as string) === "symbol" ? 9 : 7,
    label: node.label,
    kind: node.kind,
    communityId: node.metrics?.community_id,
    freshness: node.freshness,
    diagnosticCount: node.warning_count,
    symbolKind: node.concept_type,
  };
}

function graphDistances(edges: readonly MemoryGraphEdge[], root: string): Map<string, number> {
  const distances = new Map([[root, 0]]);
  let frontier = [root];
  while (frontier.length > 0) {
    const frontierSet = new Set(frontier);
    const next: string[] = [];
    for (const edge of edges) {
      for (const [from, to] of [[edge.source_id, edge.target_id], [edge.target_id, edge.source_id]] as const) {
        if (!frontierSet.has(from) || distances.has(to)) continue;
        distances.set(to, (distances.get(from) ?? 0) + 1);
        next.push(to);
      }
    }
    frontier = next;
  }
  return distances;
}

function zFor(node: MemoryGraphNode): number {
  return node.metrics?.community_id ? 12 : 0;
}

function groupBy<T>(items: readonly T[], key: (item: T) => string): Map<string, T[]> {
  const grouped = new Map<string, T[]>();
  for (const item of items) {
    const bucket = key(item);
    const values = grouped.get(bucket);
    if (values) {
      values.push(item);
    } else {
      grouped.set(bucket, [item]);
    }
  }
  return grouped;
}

function compareLayoutNodes(a: GraphLayoutNode, b: GraphLayoutNode): number {
  return compareStrings(a.nodeId, b.nodeId);
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function mostCommon<T extends string>(values: readonly T[]): T | undefined {
  const counts = new Map<T, number>();
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1);
  return [...counts.entries()].sort((a, b) => b[1] - a[1] || compareStrings(a[0], b[0]))[0]?.[0];
}

function matchesNodeFilters(
  node: MemoryGraphNode,
  filters: GraphFilters,
  communityMembers: ReadonlySet<string> | null = null,
): boolean {
  if (filters.bundleIds.length > 0 && (!node.bundle_id || !filters.bundleIds.includes(node.bundle_id))) return false;
  if (filters.nodeKinds.length > 0 && !filters.nodeKinds.includes(node.kind)) return false;
  if (filters.tags.length > 0 && !filters.tags.some((tag) => node.tags.includes(tag))) return false;
  if (filters.visibility.length > 0 && (!node.visibility || !filters.visibility.includes(node.visibility))) return false;
  if (filters.freshness.length > 0 && (!node.freshness || !filters.freshness.includes(node.freshness))) return false;
  if (filters.warning === "with_warnings" && node.warning_count <= 0) return false;
  if (filters.warning === "without_warnings" && node.warning_count > 0) return false;
  if (filters.communities.length > 0) {
    const primaryMatch = node.metrics?.community_id !== undefined
      && filters.communities.includes(node.metrics.community_id);
    if (!primaryMatch && !communityMembers?.has(node.id)) return false;
  }
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

function visibilityParams(options?: GraphRequestOptions, policy: GraphAdapterPolicy = {}): URLSearchParams {
  const params = new URLSearchParams();
  const visibility = effectiveVisibility(options?.visibility, policy);
  if (visibility) params.set("visibility", visibility);
  return params;
}

function effectiveVisibility(
  requested: GraphRequestOptions["visibility"] | undefined,
  policy: GraphAdapterPolicy,
): GraphRequestOptions["visibility"] | undefined {
  if (
    policy.maxVisibility !== undefined
    && requested !== undefined
    && visibilityRank(requested) > visibilityRank(policy.maxVisibility)
  ) {
    throw new Error(`Graph visibility "${requested}" exceeds adapter policy "${policy.maxVisibility}"`);
  }
  const visibility = requested ?? policy.defaultVisibility ?? policy.maxVisibility;
  if (
    visibility !== undefined
    && policy.maxVisibility !== undefined
    && visibilityRank(visibility) > visibilityRank(policy.maxVisibility)
  ) {
    return policy.maxVisibility;
  }
  return visibility;
}

function visibilityRank(visibility: GraphRequestOptions["visibility"]): number {
  switch (visibility) {
    case "public":
      return 0;
    case "private":
      return 1;
    case "all_accessible":
      return 2;
    default:
      return -1;
  }
}

function isCursorBefore(
  candidate: MemoryGraphSnapshot["cursor"],
  marker: MemoryGraphSnapshot["cursor"],
): boolean {
  return candidate.partition !== marker.partition || candidate.sequence < marker.sequence;
}

function isCursorAfter(
  candidate: MemoryGraphSnapshot["cursor"],
  current: MemoryGraphSnapshot["cursor"],
): boolean {
  return candidate.partition !== current.partition || candidate.sequence > current.sequence;
}

function graphFreshnessStatus(
  staleBundleIds: readonly string[],
  warningBundleIds: readonly string[],
): GraphFreshnessStatus {
  if (staleBundleIds.length > 0) return "stale";
  if (warningBundleIds.length > 0) return "warning";
  return "current";
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
