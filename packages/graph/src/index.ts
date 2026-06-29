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

export type GraphLayoutKind = "force" | "hierarchical" | "radial" | "timeline";

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
  kind: MemoryGraphNodeKind;
  communityId?: string;
}

export interface GraphLayoutEdge {
  edgeId: string;
  sourceId: string;
  targetId: string;
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
    edges: edges.map((edge) => ({ edgeId: edge.id, sourceId: edge.source_id, targetId: edge.target_id })),
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
  let didWarnWorkerFallback = false;
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
        if (!didWarnWorkerFallback) {
          didWarnWorkerFallback = true;
          console.warn("Graph layout worker timed out; falling back to synchronous layout computation.");
        }
        resolve(computeGraphLayout(snapshot, options));
      }, 250);
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

function defaultGraphLayoutWorkerFactory(): Worker | null {
  return null;
}

function forceLayout(
  nodes: readonly MemoryGraphNode[],
  edges: readonly MemoryGraphEdge[],
  width: number,
  height: number,
): GraphLayoutNode[] {
  const points = nodes.map((node, index) => ({
    id: node.id,
    x: width / 2 + Math.cos(index) * 80,
    y: height / 2 + Math.sin(index) * 80,
    vx: 0,
    vy: 0,
  }));
  const byId = new Map(points.map((point) => [point.id, point]));
  for (let tick = 0; tick < 90; tick += 1) {
    for (let i = 0; i < points.length; i += 1) {
      for (let j = i + 1; j < points.length; j += 1) {
        const a = points[i];
        const b = points[j];
        const dx = a.x - b.x || 0.01;
        const dy = a.y - b.y || 0.01;
        const distance = Math.max(24, Math.hypot(dx, dy));
        const push = 70 / (distance * distance);
        a.vx += (dx / distance) * push;
        a.vy += (dy / distance) * push;
        b.vx -= (dx / distance) * push;
        b.vy -= (dy / distance) * push;
      }
    }
    for (const edge of edges) {
      const source = byId.get(edge.source_id);
      const target = byId.get(edge.target_id);
      if (!source || !target) continue;
      const dx = target.x - source.x;
      const dy = target.y - source.y;
      const distance = Math.max(1, Math.hypot(dx, dy));
      const pull = (distance - 95) * 0.004;
      source.vx += (dx / distance) * pull;
      source.vy += (dy / distance) * pull;
      target.vx -= (dx / distance) * pull;
      target.vy -= (dy / distance) * pull;
    }
    for (const point of points) {
      point.vx += (width / 2 - point.x) * 0.002;
      point.vy += (height / 2 - point.y) * 0.002;
      point.x = clamp(point.x + point.vx, 28, width - 28);
      point.y = clamp(point.y + point.vy, 28, height - 28);
      point.vx *= 0.82;
      point.vy *= 0.82;
    }
  }
  return points.map((point) => {
    const node = nodes.find((candidate) => candidate.id === point.id)!;
    return layoutNode(node, point.x, point.y, zFor(node));
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
  const levelKeys = [...groups.keys()].sort(compareStrings);
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
    compareStrings(a.timestamp ?? a.label, b.timestamp ?? b.label) || compareNodes(a, b)
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

function layoutNode(node: MemoryGraphNode, x: number, y: number, z: number): GraphLayoutNode {
  return {
    nodeId: node.id,
    x,
    y,
    z,
    radius: node.kind === "concept" ? 9 : 7,
    label: node.label,
    kind: node.kind,
    communityId: node.metrics?.community_id,
  };
}

function graphDistances(edges: readonly MemoryGraphEdge[], root: string): Map<string, number> {
  const distances = new Map([[root, 0]]);
  let frontier = [root];
  while (frontier.length > 0) {
    const next: string[] = [];
    for (const edge of edges) {
      for (const [from, to] of [[edge.source_id, edge.target_id], [edge.target_id, edge.source_id]] as const) {
        if (!frontier.includes(from) || distances.has(to)) continue;
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

function matchesNodeFilters(node: MemoryGraphNode, filters: GraphFilters): boolean {
  if (filters.bundleIds.length > 0 && (!node.bundle_id || !filters.bundleIds.includes(node.bundle_id))) return false;
  if (filters.nodeKinds.length > 0 && !filters.nodeKinds.includes(node.kind)) return false;
  if (filters.tags.length > 0 && !filters.tags.some((tag) => node.tags.includes(tag))) return false;
  if (filters.visibility.length > 0 && (!node.visibility || !filters.visibility.includes(node.visibility))) return false;
  if (filters.freshness.length > 0 && (!node.freshness || !filters.freshness.includes(node.freshness))) return false;
  if (filters.warning === "with_warnings" && node.warning_count <= 0) return false;
  if (filters.warning === "without_warnings" && node.warning_count > 0) return false;
  if (filters.communities.length > 0 && (!node.metrics?.community_id || !filters.communities.includes(node.metrics.community_id))) return false;
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
