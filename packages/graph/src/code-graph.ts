import type {
  CodeDiffOverlay,
  CodeDiffSymbolSide,
  CodeFileOutline,
  CodeGraphConfidence,
  CodeGraphEdge,
  CodeGraphFreshness,
  CodeGraphMode as SnapshotMode,
  CodeGraphNode,
  CodeGraphSnapshot,
  CodeRepoList,
  CodeSymbolDetail,
  MemoryGraphEdgeKind,
  MemoryGraphFreshness,
  MemoryGraphNodeKind,
  MemoryGraphSnapshot,
} from "@opensymphony/gateway-schema";

export type CodeGraphMode = SnapshotMode | "diff";
export type CodeGraphDeltaStatus = "added" | "removed" | "modified" | "unchanged";

export interface CodeGraphFilters {
  repoIds: string[];
  languages: string[];
  symbolKinds: string[];
  edgeKinds: string[];
  confidences: CodeGraphConfidence[];
  freshness: CodeGraphFreshness[];
  diagnostics: "all" | "with_diagnostics" | "without_diagnostics";
  pathPrefixes: string[];
  communities: string[];
  deltaStatuses: CodeGraphDeltaStatus[];
}

export interface CodeGraphRequestOptions {
  mode?: SnapshotMode;
  path?: string;
  symbolKey?: string;
  depth?: number;
  aggregate?: "directory" | "community";
  includeStale?: boolean;
}

export interface CodeGraphDiffOptions {
  limit?: number;
}

export interface CodeRepoListRequestOptions {
  includeStale?: boolean;
}

export interface CodeSymbolDetailRequestOptions {
  includeStale?: boolean;
  visibility?: "public" | "all_accessible";
}

export interface CodeGraphAdapter {
  listRepos(options?: CodeRepoListRequestOptions): Promise<CodeRepoList>;
  getGraphSnapshot(repoId: string, options?: CodeGraphRequestOptions): Promise<CodeGraphSnapshot>;
  getSymbolDetail(repoId: string, symbolKey: string, options?: CodeSymbolDetailRequestOptions): Promise<CodeSymbolDetail>;
  getFileOutline(runId: string, filePath: string, repoId?: string): Promise<CodeFileOutline>;
  getDiffOverlay(
    repoId: string,
    baseRevision: string,
    headRevision: string,
    options?: CodeGraphDiffOptions,
  ): Promise<CodeDiffOverlay>;
}

/** Tauri/native providers keep the same DTO contract without importing Tauri. */
export type NativeCodeGraphApi = CodeGraphAdapter;

export interface CodeGraphBreadcrumb {
  kind: "repo" | "directory" | "file" | "symbol";
  id: string;
  label: string;
  nodeId?: string;
}

export interface CodeGraphHistoryState {
  repoId: string | null;
  mode: CodeGraphMode;
  symbolKey: string | null;
  path: string | null;
  runId: string | null;
  depth: number;
  filters: CodeGraphFilters;
  selectedNodeIds: string[];
  layoutSeed: string | null;
  baseRevision: string | null;
  headRevision: string | null;
}

export interface CodeGraphState extends CodeGraphHistoryState {
  repos: CodeRepoList | null;
  snapshot: CodeGraphSnapshot | null;
  symbolDetails: Record<string, CodeSymbolDetail>;
  diffOverlay: CodeDiffOverlay | null;
  breadcrumbs: CodeGraphBreadcrumb[];
  layoutStatus: "idle" | "loading" | "ready" | "failed";
  layoutError: string | null;
  stale: boolean;
  lastUpdatedAt: string | null;
}

export type CodeGraphAction =
  | { type: "REPOS_LOADED"; repos: CodeRepoList }
  | { type: "REPOS_INVALIDATED" }
  | { type: "LOAD_FAILED"; error: string }
  | { type: "SNAPSHOT_LOADED"; snapshot: CodeGraphSnapshot }
  | { type: "SYMBOL_DETAIL_LOADED"; detail: CodeSymbolDetail }
  | { type: "SYMBOL_DETAILS_INVALIDATED" }
  | { type: "DIFF_LOADED"; overlay: CodeDiffOverlay }
  | { type: "MODE_SET"; mode: CodeGraphMode }
  | { type: "REPO_SELECTED"; repoId: string | null }
  | { type: "TARGET_SET"; symbolKey?: string | null; path?: string | null; runId?: string | null }
  | { type: "NODE_SELECTED"; nodeId: string | null }
  | { type: "SELECTION_SET"; nodeIds: string[] }
  | { type: "DRILL_IN"; breadcrumb: CodeGraphBreadcrumb; mode: CodeGraphMode; path?: string | null; symbolKey?: string | null }
  | { type: "BREADCRUMB_POP"; index?: number }
  | { type: "DEPTH_SET"; depth: number }
  | { type: "FILTERS_SET"; filters: Partial<CodeGraphFilters> }
  | { type: "FILTERS_RESET" }
  | { type: "LAYOUT_SEED_SET"; seed: string | null }
  | { type: "LAYOUT_STATUS_SET"; status: CodeGraphState["layoutStatus"]; error?: string | null }
  | { type: "GRAPH_UPDATED"; repoId: string; updatedAt: string }
  | { type: "HISTORY_RESTORED"; state: Partial<CodeGraphHistoryState> }
  | { type: "GRAPH_RESET" };

export const codeGraphModes: readonly CodeGraphMode[] = ["atlas", "file", "neighborhood", "diff"];
export const codeGraphDepthBounds = { min: 1, max: 2 } as const;

export function codeGraphNeedsBroadFreshness(filters: Pick<CodeGraphFilters, "freshness">): boolean {
  return filters.freshness.includes("stale") || filters.freshness.includes("unknown");
}

export function createInitialCodeGraphFilters(): CodeGraphFilters {
  return {
    repoIds: [],
    languages: [],
    symbolKinds: [],
    edgeKinds: [],
    confidences: [],
    freshness: [],
    diagnostics: "all",
    pathPrefixes: [],
    communities: [],
    deltaStatuses: [],
  };
}

export const initialCodeGraphFilters = createInitialCodeGraphFilters();

export function createInitialCodeGraphState(): CodeGraphState {
  return {
    repos: null,
    snapshot: null,
    symbolDetails: {},
    diffOverlay: null,
    repoId: null,
    mode: "atlas",
    symbolKey: null,
    path: null,
    runId: null,
    depth: 1,
    filters: createInitialCodeGraphFilters(),
    selectedNodeIds: [],
    layoutSeed: null,
    baseRevision: null,
    headRevision: null,
    breadcrumbs: [],
    layoutStatus: "idle",
    layoutError: null,
    stale: false,
    lastUpdatedAt: null,
  };
}

export const initialCodeGraphState = createInitialCodeGraphState();

export function codeGraphReducer(state: CodeGraphState, action: CodeGraphAction): CodeGraphState {
  switch (action.type) {
    case "REPOS_LOADED": {
      const repoId = state.repoId && action.repos.repos.some((repo) => repo.repo_id === state.repoId)
        ? state.repoId
        : action.repos.repos[0]?.repo_id ?? null;
      if (repoId !== state.repoId) {
        const reset = selectCodeGraphRepo(state, repoId);
        return {
          ...reset,
          filters: { ...reset.filters, repoIds: [] },
          repos: action.repos,
        };
      }
      return {
        ...state,
        repos: action.repos,
        repoId,
      };
    }
    case "REPOS_INVALIDATED":
      return { ...state, repos: null };
    case "LOAD_FAILED":
      return {
        ...state,
        snapshot: null,
        symbolDetails: {},
        diffOverlay: null,
        selectedNodeIds: [],
        stale: false,
        lastUpdatedAt: null,
        layoutStatus: "failed",
        layoutError: action.error,
      };
    case "SNAPSHOT_LOADED": {
      const samePartition = state.snapshot?.cursor.partition === action.snapshot.cursor.partition;
      const responseMatchesTarget = state.mode === "diff"
        ? false
        : state.mode === action.snapshot.mode
          && (!state.symbolKey || action.snapshot.nodes.some((node) => node.symbol_key === state.symbolKey))
          && (!state.path || action.snapshot.nodes.some((node) => node.path_display === state.path));
      const responseIncludesStale = action.snapshot.filters_applied.includes("include_stale:true");
      const responseMatchesFreshness = responseIncludesStale === codeGraphNeedsBroadFreshness(state.filters);
      const currentMatchesView = state.snapshot?.mode === state.mode;
      if (samePartition && state.snapshot && action.snapshot.cursor.sequence <= state.snapshot.cursor.sequence && currentMatchesView && responseMatchesTarget && responseMatchesFreshness) {
        return state;
      }
      return {
        ...state,
        snapshot: action.snapshot,
        symbolDetails: {},
        repoId: action.snapshot.repo_id,
        mode: state.mode === "diff" ? state.mode : action.snapshot.mode,
        stale: false,
        lastUpdatedAt: action.snapshot.generated_at,
        layoutStatus: "idle",
        layoutError: null,
      };
    }
    case "SYMBOL_DETAIL_LOADED":
      return {
        ...state,
        symbolDetails: {
          ...state.symbolDetails,
          [`${action.detail.repo_id}:${action.detail.symbol_key}`]: action.detail,
        },
      };
    case "SYMBOL_DETAILS_INVALIDATED":
      return { ...state, symbolDetails: {} };
    case "DIFF_LOADED":
      return {
        ...state,
        diffOverlay: action.overlay,
        repoId: action.overlay.repo_id,
        baseRevision: action.overlay.base_revision,
        headRevision: action.overlay.head_revision,
        mode: "diff",
        lastUpdatedAt: action.overlay.generated_at,
      };
    case "MODE_SET":
      return action.mode === "diff" || state.mode !== "diff"
        ? { ...state, mode: action.mode }
        : {
            ...state,
            mode: action.mode,
            baseRevision: null,
            headRevision: null,
            diffOverlay: null,
            filters: { ...state.filters, deltaStatuses: [] },
          };
    case "REPO_SELECTED":
      return selectCodeGraphRepo(state, action.repoId);
    case "TARGET_SET":
      return {
        ...state,
        symbolKey: action.symbolKey === undefined ? state.symbolKey : action.symbolKey,
        path: action.path === undefined ? state.path : action.path,
        runId: action.runId === undefined ? state.runId : action.runId,
      };
    case "NODE_SELECTED":
      return { ...state, selectedNodeIds: action.nodeId ? [action.nodeId] : [] };
    case "SELECTION_SET":
      return { ...state, selectedNodeIds: uniqueSorted(action.nodeIds) };
    case "DRILL_IN":
      return {
        ...state,
        mode: action.mode,
        path: action.path === undefined ? state.path : action.path,
        symbolKey: action.symbolKey === undefined ? state.symbolKey : action.symbolKey,
        selectedNodeIds: [],
        breadcrumbs: [...state.breadcrumbs, action.breadcrumb],
        layoutStatus: "idle",
      };
    case "BREADCRUMB_POP": {
      const index = action.index ?? state.breadcrumbs.length - 2;
      const breadcrumbs = state.breadcrumbs.slice(0, Math.max(0, index + 1));
      const current = breadcrumbs[breadcrumbs.length - 1];
      const symbolKey = current?.kind === "symbol"
        ? current.id.replace(/^symbol:/, "")
        : null;
      return {
        ...state,
        breadcrumbs,
        mode: current?.kind === "symbol" ? "neighborhood" : current?.kind === "file" ? "file" : "atlas",
        path: current?.kind === "file" ? current.id : null,
        symbolKey,
        selectedNodeIds: symbolKey ? [current?.nodeId ?? `symbol:${symbolKey}`] : [],
        filters: breadcrumbs.length === 0 ? { ...state.filters, pathPrefixes: [], communities: [] } : state.filters,
        layoutStatus: "idle",
      };
    }
    case "DEPTH_SET":
      return { ...state, depth: clamp(action.depth, codeGraphDepthBounds.min, codeGraphDepthBounds.max) };
    case "FILTERS_SET":
      return {
        ...state,
        filters: normalizeCodeGraphFilters({
          ...state.filters,
          ...action.filters,
          ...((state.mode !== "diff")
            ? { deltaStatuses: [] }
            : {}),
        }),
      };
    case "FILTERS_RESET":
      return { ...state, filters: createInitialCodeGraphFilters() };
    case "LAYOUT_SEED_SET":
      return { ...state, layoutSeed: action.seed };
    case "LAYOUT_STATUS_SET":
      return { ...state, layoutStatus: action.status, layoutError: action.error ?? null };
    case "GRAPH_UPDATED":
      if (state.repoId !== action.repoId) return state;
      return { ...state, stale: true, lastUpdatedAt: action.updatedAt };
    case "HISTORY_RESTORED": {
      const restoredMode = action.state.mode ?? state.mode;
      const restoredBaseRevision = action.state.baseRevision === undefined
        ? state.baseRevision
        : action.state.baseRevision;
      const restoredHeadRevision = action.state.headRevision === undefined
        ? state.headRevision
        : action.state.headRevision;
      const restoredRepoId = action.state.repoId === undefined ? state.repoId : action.state.repoId;
      const keepsDiffOverlay = restoredMode === "diff"
        && restoredRepoId === state.repoId
        && restoredBaseRevision === state.baseRevision
        && restoredHeadRevision === state.headRevision;
      return {
        ...state,
        ...action.state,
        diffOverlay: keepsDiffOverlay ? state.diffOverlay : null,
        depth: clamp(action.state.depth ?? state.depth, codeGraphDepthBounds.min, codeGraphDepthBounds.max),
        filters: normalizeCodeGraphFilters({
          ...(action.state.filters ?? state.filters),
          ...((restoredMode !== "diff") ? { deltaStatuses: [] } : {}),
        }),
        selectedNodeIds: uniqueSorted(action.state.selectedNodeIds ?? state.selectedNodeIds),
      };
    }
    case "GRAPH_RESET":
      return createInitialCodeGraphState();
    default:
      return state;
  }
}

function selectCodeGraphRepo(state: CodeGraphState, repoId: string | null): CodeGraphState {
  return {
    ...state,
    repoId,
    mode: "atlas",
    snapshot: repoId === state.snapshot?.repo_id ? state.snapshot : null,
    symbolKey: null,
    path: null,
    runId: null,
    baseRevision: null,
    headRevision: null,
    diffOverlay: null,
    filters: { ...state.filters, pathPrefixes: [], communities: [], deltaStatuses: [] },
    selectedNodeIds: [],
    breadcrumbs: [],
    stale: false,
    layoutStatus: "idle",
    layoutError: null,
  };
}

export function currentCodeGraphSnapshot(state: CodeGraphState): CodeGraphSnapshot | null {
  return state.snapshot;
}

export function codeGraphStateToHistory(state: CodeGraphState): CodeGraphHistoryState {
  return {
    repoId: state.repoId,
    mode: state.mode,
    symbolKey: state.symbolKey,
    path: state.path,
    runId: state.runId,
    depth: state.depth,
    filters: normalizeCodeGraphFilters(state.filters),
    selectedNodeIds: uniqueSorted(state.selectedNodeIds),
    layoutSeed: state.layoutSeed,
    baseRevision: state.baseRevision,
    headRevision: state.headRevision,
  };
}

export function normalizeCodeGraphFilters(filters: CodeGraphFilters): CodeGraphFilters {
  return {
    repoIds: uniqueSorted(filters.repoIds),
    languages: uniqueSorted(filters.languages),
    symbolKinds: uniqueSorted(filters.symbolKinds),
    edgeKinds: uniqueSorted(filters.edgeKinds),
    confidences: uniqueSorted(filters.confidences),
    freshness: uniqueSorted(filters.freshness),
    diagnostics: filters.diagnostics === "with_diagnostics" || filters.diagnostics === "without_diagnostics"
      ? filters.diagnostics
      : "all",
    pathPrefixes: uniqueSorted(filters.pathPrefixes),
    communities: uniqueSorted(filters.communities),
    deltaStatuses: uniqueSorted(filters.deltaStatuses),
  };
}

export function applyCodeGraphFilters(
  snapshot: CodeGraphSnapshot,
  filters: CodeGraphFilters,
  overlay?: CodeDiffOverlay | null,
): CodeGraphSnapshot {
  const normalized = normalizeCodeGraphFilters(filters);
  const sourceSnapshot = withCodeDiffNodes(snapshot, overlay);
  const members = new Set(
    sourceSnapshot.communities
      .filter((community) => normalized.communities.includes(community.id))
      .flatMap((community) => community.node_ids),
  );
  const deltaBySymbol = deltaStatuses(overlay);
  const visibleIds = new Set(sourceSnapshot.nodes
    .filter((node) => matchesCodeNode(node, sourceSnapshot.repo_id, normalized, members, deltaBySymbol))
    .map((node) => node.id));
  const edges = sourceSnapshot.edges.filter((edge) =>
    visibleIds.has(edge.source_id)
    && visibleIds.has(edge.target_id)
    && (normalized.edgeKinds.length === 0 || normalized.edgeKinds.includes(edge.kind))
    && (normalized.confidences.length === 0 || normalized.confidences.includes(edge.confidence)),
  );
  return {
    ...sourceSnapshot,
    nodes: sourceSnapshot.nodes.filter((node) => visibleIds.has(node.id)),
    edges,
    communities: sourceSnapshot.communities
      .map((community) => ({
        ...community,
        node_ids: community.node_ids.filter((nodeId) => visibleIds.has(nodeId)),
        symbol_count: community.node_ids.filter((nodeId) => visibleIds.has(nodeId)).length,
      }))
      .filter((community) => community.node_ids.length > 0),
    filters_applied: uniqueSorted([
      ...snapshot.filters_applied,
      ...codeGraphFilterTokens(normalized),
    ]),
  };
}

export function codeGraphFilterTokens(filters: CodeGraphFilters): string[] {
  return [
    ...filters.repoIds.map((value) => `repo:${value}`),
    ...filters.languages.map((value) => `language:${value}`),
    ...filters.symbolKinds.map((value) => `symbol-kind:${value}`),
    ...filters.edgeKinds.map((value) => `edge-kind:${value}`),
    ...filters.confidences.map((value) => `confidence:${value}`),
    ...filters.freshness.map((value) => `freshness:${value}`),
    ...(filters.diagnostics === "all" ? [] : [`diagnostics:${filters.diagnostics}`]),
    ...filters.pathPrefixes.map((value) => `path:${value}`),
    ...filters.communities.map((value) => `community:${value}`),
    ...filters.deltaStatuses.map((value) => `delta:${value}`),
  ].sort();
}

export function codeGraphLayoutKindForMode(mode: CodeGraphMode): "force" | "hierarchical" | "radial" {
  switch (mode) {
    case "file":
      return "hierarchical";
    case "neighborhood":
    case "diff":
      return "radial";
    default:
      return "force";
  }
}

/** Convert the code DTO into the shared graph scene/layout input shape. */
export function codeGraphSnapshotForRendering(
  snapshot: CodeGraphSnapshot,
  overlay?: CodeDiffOverlay | null,
): MemoryGraphSnapshot {
  const sourceSnapshot = withCodeDiffNodes(snapshot, overlay);
  const deltaBySymbol = deltaStatuses(overlay);
  return {
    schema_version: sourceSnapshot.schema_version,
    bundle_id: sourceSnapshot.repo_id,
    cursor: sourceSnapshot.cursor,
    generated_at: sourceSnapshot.generated_at,
    filters_applied: sourceSnapshot.filters_applied,
    communities: sourceSnapshot.communities.map((community) => ({
      id: community.id,
      label: community.label,
      node_ids: community.node_ids,
      concept_count: community.symbol_count,
    })),
    nodes: sourceSnapshot.nodes.map((node) => ({
      id: node.id,
      kind: node.kind as unknown as MemoryGraphNodeKind,
      label: node.label,
      bundle_id: sourceSnapshot.repo_id,
      concept_id: node.symbol_key ?? undefined,
      concept_type: node.symbol_kind ?? undefined,
      path_display: node.path_display ?? undefined,
      description: node.signature ?? undefined,
      tags: node.language ? [node.language] : [],
      freshness: node.freshness as unknown as MemoryGraphFreshness,
      warning_count: node.diagnostic_count,
      frontmatter_summary: {
        language: node.language ?? undefined,
        symbol_kind: node.symbol_kind ?? undefined,
        delta_status: node.symbol_key ? deltaBySymbol.get(node.symbol_key) : undefined,
      },
      unknown_frontmatter: {},
      metrics: {
        indegree: node.metrics.in_degree,
        outdegree: node.metrics.out_degree,
        community_id: node.metrics.community_id ?? undefined,
      },
    })),
    edges: sourceSnapshot.edges.map((edge) => ({
      id: edge.id,
      kind: edge.kind as MemoryGraphEdgeKind,
      source_id: edge.source_id,
      target_id: edge.target_id,
      unresolved: edge.unresolved,
      metadata: {
        confidence: edge.confidence,
        target_hint: edge.target_hint ?? undefined,
      },
    })),
  };
}

export interface CodeNodeVisualStyle {
  color: string;
  opacity: number;
  borderStyle: "solid" | "dashed" | "dotted";
  freshnessLabel: string;
}

export interface CodeEdgeVisualStyle {
  color: string;
  opacity: number;
  lineStyle: "solid" | "dashed" | "dotted";
}

export function codeNodeVisualStyle(node: Pick<CodeGraphNode, "kind" | "symbol_kind" | "freshness" | "diagnostic_count">): CodeNodeVisualStyle {
  const color = node.kind === "directory"
    ? "#475569"
    : node.kind === "file"
      ? "#2563eb"
      : node.kind === "community"
        ? "#7c3aed"
        : node.symbol_kind?.startsWith("test")
          ? "#0f766e"
          : "#0f766e";
  const freshness = node.freshness === "current"
    ? { opacity: 1, borderStyle: "solid" as const, label: "current" }
    : node.freshness === "stale"
      ? { opacity: 0.62, borderStyle: "dashed" as const, label: "stale" }
      : { opacity: 0.45, borderStyle: "dotted" as const, label: "unknown" };
  return {
    color,
    opacity: node.diagnostic_count > 0 ? Math.max(0.5, freshness.opacity) : freshness.opacity,
    borderStyle: freshness.borderStyle,
    freshnessLabel: node.diagnostic_count > 0 ? `${freshness.label}, diagnostics` : freshness.label,
  };
}

export function codeEdgeVisualStyle(edge: Pick<CodeGraphEdge, "confidence">): CodeEdgeVisualStyle {
  switch (edge.confidence) {
    case "exact":
      return { color: "#2563eb", opacity: 0.9, lineStyle: "solid" };
    case "syntactic":
      return { color: "#7c3aed", opacity: 0.68, lineStyle: "dashed" };
    default:
      return { color: "#64748b", opacity: 0.48, lineStyle: "dotted" };
  }
}

export function codeGraphNodeDeltaStatus(
  symbolKey: string | null | undefined,
  overlay: CodeDiffOverlay | null | undefined,
): CodeGraphDeltaStatus {
  return (symbolKey ? deltaStatuses(overlay).get(symbolKey) : undefined) ?? "unchanged";
}

export function createHttpCodeGraphAdapter(
  baseUri: string,
  fetchFn: typeof fetch = globalThis.fetch,
): CodeGraphAdapter {
  const base = baseUri.replace(/\/+$/, "");
  async function read<T>(path: string, params = new URLSearchParams()): Promise<T> {
    const query = params.toString();
    const response = await fetchFn(`${base}${path}${query ? `?${query}` : ""}`);
    if (!response.ok) throw new Error(`Code graph request failed: HTTP ${response.status}`);
    return await response.json() as T;
  }
  return {
    listRepos: (options) => {
      const params = new URLSearchParams();
      if (options?.includeStale !== undefined) params.set("include_stale", String(options.includeStale));
      return read<CodeRepoList>("/api/v1/code/repos", params);
    },
    getGraphSnapshot: (repoId, options) => {
      const params = codeGraphRequestParams(options);
      return read<CodeGraphSnapshot>(`/api/v1/code/repos/${encodeURIComponent(repoId)}/graph`, params);
    },
    getSymbolDetail: (repoId, symbolKey, options) => {
      const params = new URLSearchParams();
      if (options?.includeStale !== undefined) params.set("include_stale", String(options.includeStale));
      if (options?.visibility !== undefined) params.set("visibility", options.visibility);
      return read<CodeSymbolDetail>(
        `/api/v1/code/repos/${encodeURIComponent(repoId)}/symbols/${encodeURIComponent(symbolKey)}`,
        params,
      );
    },
    getFileOutline: (runId, filePath, repoId) => {
      const params = new URLSearchParams({ file_path: filePath });
      if (repoId) params.set("repo_id", repoId);
      return read<CodeFileOutline>(`/api/v1/runs/${encodeURIComponent(runId)}/code/outline`, params);
    },
    getDiffOverlay: (repoId, baseRevision, headRevision, options) => {
      const params = new URLSearchParams({ base_revision: baseRevision, head_revision: headRevision });
      if (options?.limit !== undefined) params.set("limit", String(options.limit));
      return read<CodeDiffOverlay>(`/api/v1/code/repos/${encodeURIComponent(repoId)}/diff-overlay`, params);
    },
  };
}

export const createGatewayCodeGraphAdapter = createHttpCodeGraphAdapter;

export function createTauriNativeCodeGraphAdapter(api: NativeCodeGraphApi): CodeGraphAdapter {
  return api;
}

export interface CodeGraphFixtures {
  repos?: CodeRepoList;
  snapshots?: CodeGraphSnapshot | readonly CodeGraphSnapshot[];
  symbolDetails?: readonly CodeSymbolDetail[];
  fileOutlines?: readonly CodeFileOutline[];
  diffOverlays?: readonly CodeDiffOverlay[];
}

export function createFixtureCodeGraphAdapter(fixtures: CodeGraphFixtures = {}): CodeGraphAdapter {
  const repos = fixtures.repos ?? codeGraphFixtureRepos;
  const snapshots: readonly CodeGraphSnapshot[] = Array.isArray(fixtures.snapshots)
    ? fixtures.snapshots
    : fixtures.snapshots
      ? [fixtures.snapshots]
      : codeGraphFixtureSnapshots;
  const details = fixtures.symbolDetails ?? codeGraphFixtureSymbolDetails;
  const outlines = fixtures.fileOutlines ?? codeGraphFixtureOutlines;
  const overlays = fixtures.diffOverlays ?? codeGraphFixtureDiffOverlays;
  return {
    listRepos: async () => repos,
    getGraphSnapshot: async (repoId, options) => {
      const mode = options?.mode ?? "atlas";
      const snapshot = snapshots.find((candidate) =>
        candidate.repo_id === repoId
        && candidate.mode === mode
        && (options?.path === undefined || candidate.nodes.some((node) => node.path_display === options.path)),
      ) ?? snapshots.find((candidate) => candidate.repo_id === repoId && candidate.mode === mode);
      if (!snapshot) throw new Error(`Code graph snapshot not found: ${repoId}/${mode}`);
      return snapshot;
    },
    getSymbolDetail: async (repoId, symbolKey) => {
      const detail = details.find((candidate) => candidate.repo_id === repoId && candidate.symbol_key === symbolKey);
      if (!detail) throw new Error(`Code symbol not found: ${symbolKey}`);
      return detail;
    },
    getFileOutline: async (runId, filePath) => {
      const outline = outlines.find((candidate) => candidate.run_id === runId && candidate.path === filePath)
        ?? outlines.find((candidate) => candidate.path === filePath);
      if (!outline) throw new Error(`Code outline not found: ${filePath}`);
      return outline;
    },
    getDiffOverlay: async (repoId, baseRevision, headRevision) => {
      const overlay = overlays.find((candidate) =>
        candidate.repo_id === repoId
        && candidate.base_revision === baseRevision
        && candidate.head_revision === headRevision,
      );
      if (!overlay) throw new Error(`Code diff overlay not found: ${baseRevision}..${headRevision}`);
      return overlay;
    },
  };
}

export const createCodeGraphFixtureAdapter = createFixtureCodeGraphAdapter;

function codeGraphRequestParams(options?: CodeGraphRequestOptions): URLSearchParams {
  const params = new URLSearchParams();
  if (options?.mode) params.set("mode", options.mode);
  if (options?.path) params.set("path", options.path);
  if (options?.symbolKey) params.set("symbol_key", options.symbolKey);
  if (options?.depth !== undefined) params.set("depth", String(clamp(options.depth, codeGraphDepthBounds.min, codeGraphDepthBounds.max)));
  if (options?.aggregate) params.set("aggregate", options.aggregate);
  if (options?.includeStale !== undefined) params.set("include_stale", String(options.includeStale));
  return params;
}

function withCodeDiffNodes(snapshot: CodeGraphSnapshot, overlay?: CodeDiffOverlay | null): CodeGraphSnapshot {
  if (!overlay) return snapshot;
  const existingKeys = new Set(snapshot.nodes.map((node) => node.symbol_key).filter((key): key is string => Boolean(key)));
  const syntheticSides = new Map<string, CodeDiffSymbolSide>();
  for (const symbol of [...overlay.added_symbols, ...overlay.modified_symbols, ...overlay.removed_symbols]) {
    if (existingKeys.has(symbol.symbol_key) || syntheticSides.has(symbol.symbol_key)) continue;
    const side = symbol.status === "removed" ? symbol.before : symbol.after ?? symbol.before;
    if (side) syntheticSides.set(symbol.symbol_key, side);
  }
  if (syntheticSides.size === 0) return snapshot;
  const syntheticNodes = [...syntheticSides].map(([symbolKey, side]) => ({
    id: `symbol:${symbolKey}`,
    kind: "symbol" as const,
    label: side.name,
    symbol_kind: side.kind,
    symbol_key: symbolKey,
    symbol_id: side.symbol_id,
    path_display: side.path_display,
    language: null,
    container_chain: side.container_chain,
    signature: null,
    span: side.span,
    selection_span: side.span,
    freshness: side.freshness,
    diagnostic_count: 0,
    diagnostic_severity: null,
    metrics: { in_degree: 0, out_degree: 0, community_id: null },
  }));
  return { ...snapshot, nodes: [...snapshot.nodes, ...syntheticNodes] };
}

function matchesCodeNode(
  node: CodeGraphNode,
  repoId: string,
  filters: CodeGraphFilters,
  communityMembers: ReadonlySet<string>,
  deltaBySymbol: ReadonlyMap<string, CodeGraphDeltaStatus>,
): boolean {
  if (filters.repoIds.length > 0 && !filters.repoIds.includes(repoId)) return false;
  if (filters.languages.length > 0 && (!node.language || !filters.languages.includes(node.language))) return false;
  if (filters.symbolKinds.length > 0 && (!node.symbol_kind || !filters.symbolKinds.includes(node.symbol_kind))) return false;
  if (filters.freshness.length > 0 && !filters.freshness.includes(node.freshness)) return false;
  if (filters.diagnostics === "with_diagnostics" && node.diagnostic_count <= 0) return false;
  if (filters.diagnostics === "without_diagnostics" && node.diagnostic_count > 0) return false;
  if (filters.pathPrefixes.length > 0 && (!node.path_display || !filters.pathPrefixes.some((prefix) => codePathMatchesPrefix(node.path_display!, prefix)))) return false;
  if (filters.communities.length > 0 && !communityMembers.has(node.id)
    && (!node.metrics.community_id || !filters.communities.includes(node.metrics.community_id))) return false;
  if (filters.deltaStatuses.length > 0 && (!node.symbol_key || !filters.deltaStatuses.includes(deltaBySymbol.get(node.symbol_key) ?? "unchanged"))) return false;
  return true;
}

function codePathMatchesPrefix(path: string, prefix: string): boolean {
  const normalizedPrefix = prefix.endsWith("/") ? prefix.slice(0, -1) : prefix;
  return path === normalizedPrefix || path.startsWith(`${normalizedPrefix}/`);
}

function deltaStatuses(overlay?: CodeDiffOverlay | null): Map<string, CodeGraphDeltaStatus> {
  const result = new Map<string, CodeGraphDeltaStatus>();
  if (!overlay) return result;
  for (const symbol of [...overlay.added_symbols, ...overlay.removed_symbols, ...overlay.modified_symbols]) {
    result.set(symbol.symbol_key, symbol.status);
  }
  return result;
}

function uniqueSorted<T extends string>(values: readonly T[]): T[] {
  return [...new Set(values)].sort();
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.floor(value)));
}

// Imported lazily through the bottom-of-file re-export to keep fixture data in
// the graph visualization workbench module rather than inventing another demo.
import {
  codeGraphFixtureDiffOverlays,
  codeGraphFixtureOutlines,
  codeGraphFixtureRepos,
  codeGraphFixtureSnapshots,
  codeGraphFixtureSymbolDetails,
} from "./viz-fixture.js";
