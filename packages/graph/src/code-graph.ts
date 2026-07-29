import type {
  CodeDiffBlastRadius,
  CodeDiffEdge,
  CodeDiffEdgeSide,
  CodeDiffOverlay,
  CodeDiffSymbolSide,
  CodeFileOutline,
  CodeGraphConfidence,
  CodeGraphEdge,
  CodeGraphFreshness,
  CodeGraphMode as SnapshotMode,
  CodeGraphNode,
  CodeGraphSnapshot,
  CodeIndexReport,
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

type CodeDiffOverlayArrayField =
  | "added_symbols"
  | "removed_symbols"
  | "modified_symbols"
  | "edge_deltas"
  | "module_connection_deltas"
  | "blast_radius"
  | "unanalyzed_files";

type CodeDiffBlastRadiusPayload = Omit<CodeDiffBlastRadius, "inbound" | "outbound">
  & Partial<Pick<CodeDiffBlastRadius, "inbound" | "outbound">>;

type CodeDiffOverlayPayload = Omit<CodeDiffOverlay, CodeDiffOverlayArrayField>
  & Partial<Pick<CodeDiffOverlay, Exclude<CodeDiffOverlayArrayField, "blast_radius">>>
  & { blast_radius?: CodeDiffBlastRadiusPayload[] };

export function normalizeCodeDiffOverlay(overlay: CodeDiffOverlayPayload): CodeDiffOverlay {
  return {
    ...overlay,
    added_symbols: overlay.added_symbols ?? [],
    removed_symbols: overlay.removed_symbols ?? [],
    modified_symbols: overlay.modified_symbols ?? [],
    edge_deltas: overlay.edge_deltas ?? [],
    module_connection_deltas: overlay.module_connection_deltas ?? [],
    blast_radius: (overlay.blast_radius ?? []).map((entry) => ({
      ...entry,
      inbound: entry.inbound ?? [],
      outbound: entry.outbound ?? [],
    })),
    unanalyzed_files: overlay.unanalyzed_files ?? [],
  };
}

export interface CodeRepoListRequestOptions {
  includeStale?: boolean;
}

export interface CodeSymbolDetailRequestOptions {
  includeStale?: boolean;
  visibility?: "public" | "all_accessible";
}

export interface CodeGraphAdapterPolicy {
  defaultVisibility?: "public" | "all_accessible";
  maxVisibility?: "public" | "all_accessible";
}

export interface CodeGraphAdapter {
  listRepos(options?: CodeRepoListRequestOptions): Promise<CodeRepoList>;
  indexRepo(repoId: string): Promise<CodeIndexReport>;
  getGraphSnapshot(repoId: string, options?: CodeGraphRequestOptions): Promise<CodeGraphSnapshot>;
  getRunGraphSnapshot?(runId: string, repoId?: string, options?: CodeGraphRequestOptions): Promise<CodeGraphSnapshot>;
  getSymbolDetail(repoId: string, symbolKey: string, options?: CodeSymbolDetailRequestOptions): Promise<CodeSymbolDetail>;
  getFileOutline(runId: string, filePath: string, repoId?: string): Promise<CodeFileOutline>;
  getDiffOverlay(
    repoId: string,
    baseRevision: string,
    headRevision: string,
    options?: CodeGraphDiffOptions,
  ): Promise<CodeDiffOverlay>;
  getRunDiffOverlay?(runId: string, repoId?: string, options?: CodeGraphDiffOptions): Promise<CodeDiffOverlay>;
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
  indexReport: CodeIndexReport | null;
  indexing: boolean;
  indexError: string | null;
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
  | { type: "INDEX_STARTED"; repoId: string }
  | { type: "INDEX_REPORT"; report: CodeIndexReport }
  | { type: "INDEX_REQUEST_FAILED"; repoId: string; error: string }
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
    indexReport: null,
    indexing: false,
    indexError: null,
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
    case "INDEX_STARTED":
      return state.repoId === action.repoId
        ? { ...state, indexing: true, indexError: null }
        : state;
    case "INDEX_REPORT":
      return state.repoId === action.report.repo_id
        && !isStaleCodeIndexReport(state.indexReport, action.report)
        ? {
            ...state,
            indexReport: action.report,
            indexing: action.report.status === "accepted" || action.report.status === "progress",
            indexError: action.report.status === "failed" || action.report.status === "unavailable"
              ? action.report.diagnostics[0] ?? `Code Graph index ${action.report.status}`
              : null,
          }
        : state;
    case "INDEX_REQUEST_FAILED":
      return state.repoId === action.repoId
        ? { ...state, indexing: false, indexError: action.error }
        : state;
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
      {
        const overlay = normalizeCodeDiffOverlay(action.overlay);
      return {
        ...state,
        diffOverlay: overlay,
        repoId: overlay.repo_id,
        baseRevision: overlay.base_revision,
        headRevision: overlay.head_revision,
        mode: "diff",
        lastUpdatedAt: overlay.generated_at,
      };
      }
    case "MODE_SET":
      return action.mode === "diff" || state.mode !== "diff"
        ? { ...state, mode: action.mode }
        : {
            ...state,
            mode: action.mode,
            baseRevision: state.runId ? state.baseRevision : null,
            headRevision: state.runId ? state.headRevision : null,
            diffOverlay: state.runId ? state.diffOverlay : null,
            filters: { ...state.filters, deltaStatuses: [] },
          };
    case "REPO_SELECTED":
      return selectCodeGraphRepo(state, action.repoId);
    case "TARGET_SET": {
      const runChanged = action.runId !== undefined && action.runId !== state.runId;
      return {
        ...state,
        symbolKey: action.symbolKey === undefined ? state.symbolKey : action.symbolKey,
        path: action.path === undefined ? state.path : action.path,
        runId: action.runId === undefined ? state.runId : action.runId,
        ...(runChanged
          ? {
              baseRevision: null,
              headRevision: null,
              diffOverlay: null,
              filters: { ...state.filters, deltaStatuses: [] },
            }
          : {}),
      };
    }
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
      const restoredRunId = action.state.runId === undefined
        ? state.runId
        : action.state.runId;
      const runChanged = restoredRunId !== state.runId;
      const restoredBaseRevision = action.state.baseRevision === undefined
        ? runChanged ? null : state.baseRevision
        : action.state.baseRevision;
      const restoredHeadRevision = action.state.headRevision === undefined
        ? runChanged ? null : state.headRevision
        : action.state.headRevision;
      const restoredRepoId = action.state.repoId === undefined ? state.repoId : action.state.repoId;
      const keepsDiffOverlay = restoredMode === "diff"
        && !runChanged
        && restoredRepoId === state.repoId
        && restoredBaseRevision === state.baseRevision
        && restoredHeadRevision === state.headRevision;
      return {
        ...state,
        ...action.state,
        baseRevision: restoredBaseRevision,
        headRevision: restoredHeadRevision,
        diffOverlay: keepsDiffOverlay ? state.diffOverlay : null,
        depth: clamp(action.state.depth ?? state.depth, codeGraphDepthBounds.min, codeGraphDepthBounds.max),
        filters: normalizeCodeGraphFilters({
          ...(action.state.filters ?? state.filters),
          ...((restoredMode !== "diff" || runChanged) ? { deltaStatuses: [] } : {}),
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
    indexReport: null,
    indexing: false,
    indexError: null,
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

function isStaleCodeIndexReport(
  current: CodeIndexReport | null,
  candidate: CodeIndexReport,
): boolean {
  if (!current || current.repo_id !== candidate.repo_id) return false;
  if (current.cursor.partition !== candidate.cursor.partition) return false;
  if (candidate.cursor.sequence < current.cursor.sequence) return true;
  if (candidate.cursor.sequence > current.cursor.sequence) return false;
  return codeIndexStatusRank(candidate.status) <= codeIndexStatusRank(current.status);
}

function codeIndexStatusRank(status: CodeIndexReport["status"]): number {
  switch (status) {
    case "accepted":
      return 0;
    case "progress":
      return 1;
    case "failed":
    case "unavailable":
      return 2;
    case "completed":
      return 3;
  }
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

const materializedCodeDiffSnapshots = new WeakSet<CodeGraphSnapshot>();

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
  const filteredSnapshot = {
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
  if (overlay) materializedCodeDiffSnapshots.add(filteredSnapshot);
  return filteredSnapshot;
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
  const sourceSnapshot = materializedCodeDiffSnapshots.has(snapshot)
    ? snapshot
    : withCodeDiffNodes(snapshot, overlay);
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

export function codeNodeVisualStyle(
  node: Pick<CodeGraphNode, "kind" | "symbol_kind" | "freshness" | "diagnostic_count">,
  deltaStatus: CodeGraphDeltaStatus = "unchanged",
  blastRadius = false,
): CodeNodeVisualStyle {
  const color = deltaStatus === "added"
    ? "#4ade80"
    : deltaStatus === "removed"
      ? "#94a3b8"
      : deltaStatus === "modified"
        ? "#fbbf24"
        : node.kind === "directory"
    ? "#a8b3bf"
    : node.kind === "file"
      ? "#60a5fa"
      : node.kind === "community"
        ? "#c084fc"
        : node.symbol_kind?.startsWith("test")
          ? "#5eead4"
          : "#2dd4bf";
  const freshness = node.freshness === "current"
    ? { opacity: 1, borderStyle: "solid" as const, label: "current" }
    : node.freshness === "stale"
      ? { opacity: 0.62, borderStyle: "dashed" as const, label: "stale" }
      : { opacity: 0.45, borderStyle: "dotted" as const, label: "unknown" };
  return {
    color,
    opacity: deltaStatus === "removed" ? 0.42 : node.diagnostic_count > 0 ? Math.max(0.5, freshness.opacity) : freshness.opacity,
    borderStyle: deltaStatus === "removed" || blastRadius ? "dashed" : freshness.borderStyle,
    freshnessLabel: deltaStatus !== "unchanged"
      ? `${deltaStatus}${blastRadius ? ", blast radius" : ""}`
      : blastRadius ? `${freshness.label}, blast radius` : node.diagnostic_count > 0 ? `${freshness.label}, diagnostics` : freshness.label,
  };
}

export function codeEdgeVisualStyle(edge: Pick<CodeGraphEdge, "confidence">): CodeEdgeVisualStyle {
  switch (edge.confidence) {
    case "exact":
      return { color: "#60a5fa", opacity: 0.9, lineStyle: "solid" };
    case "syntactic":
      return { color: "#c084fc", opacity: 0.72, lineStyle: "dashed" };
    default:
      return { color: "#cbd5e1", opacity: 0.58, lineStyle: "dotted" };
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
  policy: CodeGraphAdapterPolicy = {},
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
    indexRepo: async (repoId) => {
      const response = await fetchFn(`${base}/api/v1/code/repos/${encodeURIComponent(repoId)}/index`, {
        method: "POST",
      });
      if (!response.ok) throw new Error(`Code graph request failed: HTTP ${response.status}`);
      return await response.json() as CodeIndexReport;
    },
    getGraphSnapshot: (repoId, options) => {
      const params = codeGraphRequestParams(options);
      return read<CodeGraphSnapshot>(`/api/v1/code/repos/${encodeURIComponent(repoId)}/graph`, params);
    },
    getRunGraphSnapshot: (runId, repoId, options) => {
      const params = codeGraphRequestParams(options);
      if (repoId) params.set("repo_id", repoId);
      return read<CodeGraphSnapshot>(`/api/v1/runs/${encodeURIComponent(runId)}/code/graph`, params);
    },
    getSymbolDetail: (repoId, symbolKey, options) => {
      const params = new URLSearchParams();
      if (options?.includeStale !== undefined) params.set("include_stale", String(options.includeStale));
      const visibility = effectiveCodeVisibility(options?.visibility, policy);
      if (visibility !== undefined) params.set("visibility", visibility);
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
      return read<CodeDiffOverlayPayload>(`/api/v1/code/repos/${encodeURIComponent(repoId)}/diff-overlay`, params)
        .then(normalizeCodeDiffOverlay);
    },
    getRunDiffOverlay: (runId, repoId, options) => {
      const params = new URLSearchParams();
      if (repoId) params.set("repo_id", repoId);
      if (options?.limit !== undefined) params.set("limit", String(options.limit));
      return read<CodeDiffOverlayPayload>(`/api/v1/runs/${encodeURIComponent(runId)}/code/diff-overlay`, params)
        .then(normalizeCodeDiffOverlay);
    },
  };
}

function effectiveCodeVisibility(
  requested: CodeSymbolDetailRequestOptions["visibility"] | undefined,
  policy: CodeGraphAdapterPolicy,
): CodeSymbolDetailRequestOptions["visibility"] | undefined {
  if (policy.maxVisibility === "public" && requested === "all_accessible") {
    throw new Error('Code graph visibility "all_accessible" exceeds adapter policy "public"');
  }
  return requested ?? policy.defaultVisibility ?? policy.maxVisibility;
}

export const createGatewayCodeGraphAdapter = createHttpCodeGraphAdapter;

export function createTauriNativeCodeGraphAdapter(
  api: NativeCodeGraphApi,
  policy: CodeGraphAdapterPolicy = {},
): CodeGraphAdapter {
  return {
    ...api,
    getDiffOverlay: (repoId, baseRevision, headRevision, options) =>
      api.getDiffOverlay(repoId, baseRevision, headRevision, options).then(normalizeCodeDiffOverlay),
    getRunDiffOverlay: api.getRunDiffOverlay
      ? (runId, repoId, options) => api.getRunDiffOverlay!(runId, repoId, options).then(normalizeCodeDiffOverlay)
      : undefined,
    getSymbolDetail: (repoId, symbolKey, options) => api.getSymbolDetail(repoId, symbolKey, {
      ...options,
      visibility: effectiveCodeVisibility(options?.visibility, policy),
    }),
  };
}

export interface CodeGraphFixtures {
  repos?: CodeRepoList;
  indexReports?: readonly CodeIndexReport[];
  indexRepo?: (repoId: string) => Promise<CodeIndexReport>;
  snapshots?: CodeGraphSnapshot | readonly CodeGraphSnapshot[];
  symbolDetails?: readonly CodeSymbolDetail[];
  fileOutlines?: readonly CodeFileOutline[];
  diffOverlays?: readonly CodeDiffOverlay[];
}

export function createFixtureCodeGraphAdapter(fixtures: CodeGraphFixtures = {}): CodeGraphAdapter {
  const repos = fixtures.repos ?? codeGraphFixtureRepos;
  const indexReports = fixtures.indexReports ?? [];
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
    indexRepo: async (repoId) => {
      if (fixtures.indexRepo) return fixtures.indexRepo(repoId);
      const report = indexReports.find((candidate) => candidate.repo_id === repoId)
        ?? repos.repos.find((candidate) => candidate.repo_id === repoId);
      if (!report) throw new Error(`Code graph index target not found: ${repoId}`);
      if ("status" in report) return report;
      return {
        schema_version: { major: 1, minor: 0, patch: 0 },
        repo_id: repoId,
        status: "completed",
        head_revision: report.head_revision ?? null,
        parsed_files: report.document_count,
        persisted_documents: report.document_count,
        persisted_symbols: report.symbol_count,
        persisted_edges: report.edge_count,
        persisted_diagnostics: 0,
        stale_rows: 0,
        skipped_files: [],
        diagnostics: [],
        cursor: { sequence: 1, partition: `code-graph:${repoId}` },
        indexed_at: report.indexed_at ?? new Date().toISOString(),
      };
    },
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
      return normalizeCodeDiffOverlay(overlay);
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
  overlay = normalizeCodeDiffOverlay(overlay);
  const existingKeys = new Set(snapshot.nodes.map((node) => node.symbol_key).filter((key): key is string => Boolean(key)));
  const syntheticSides = new Map<string, CodeDiffSymbolSide>();
  for (const symbol of [...overlay.added_symbols, ...overlay.modified_symbols, ...overlay.removed_symbols]) {
    if (existingKeys.has(symbol.symbol_key) || syntheticSides.has(symbol.symbol_key)) continue;
    const side = symbol.status === "removed" ? symbol.before : symbol.after ?? symbol.before;
    if (side) syntheticSides.set(symbol.symbol_key, side);
  }
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
  const radiusNodes = overlay.blast_radius
    .filter((entry) => !existingKeys.has(entry.symbol_key) && !syntheticSides.has(entry.symbol_key))
    .map((entry) => ({
      id: `symbol:${entry.symbol_key}`,
      kind: "symbol" as const,
      label: entry.symbol_key,
      symbol_kind: "blast_radius",
      symbol_key: entry.symbol_key,
      symbol_id: null,
      path_display: null,
      language: null,
      container_chain: [],
      signature: null,
      span: null,
      selection_span: null,
      freshness: "unknown" as const,
      diagnostic_count: 0,
      diagnostic_severity: null,
      metrics: { in_degree: entry.inbound_count, out_degree: entry.outbound_count, community_id: null },
    }));
  const radiusKeys = new Set(radiusNodes.map((node) => node.symbol_key).filter((key): key is string => Boolean(key)));
  const symbolNodeIds = new Map<string, string>();
  for (const node of [...snapshot.nodes, ...syntheticNodes, ...radiusNodes]) {
    if (node.symbol_key && !symbolNodeIds.has(node.symbol_key)) symbolNodeIds.set(node.symbol_key, node.id);
  }
  const topologySides = (delta: CodeDiffEdge): Array<{ side: CodeDiffEdgeSide; suffix: string }> => {
    if (delta.status === "retargeted" && delta.before && delta.after) {
      return [{ side: delta.before, suffix: ":before" }, { side: delta.after, suffix: ":after" }];
    }
    const side = delta.after ?? delta.before;
    return side ? [{ side, suffix: "" }] : [];
  };
  const topologyEntries = overlay.edge_deltas.flatMap((delta) =>
    topologySides(delta).map(({ side, suffix }) => ({ delta, side, suffix })));
  const topologySymbolKeys = new Set<string>();
  const topologySymbolPaths = new Map<string, string>();
  for (const { side } of topologyEntries) {
    for (const symbolKey of [side?.source_symbol_key, side?.target_symbol_key]) {
      if (!symbolKey) continue;
      if (!existingKeys.has(symbolKey) && !syntheticSides.has(symbolKey) && !radiusKeys.has(symbolKey)) topologySymbolKeys.add(symbolKey);
    }
    if (side.source_symbol_key && !topologySymbolPaths.has(side.source_symbol_key)) {
      topologySymbolPaths.set(side.source_symbol_key, side.path);
    }
  }
  const topologySymbolNodes: CodeGraphNode[] = [...topologySymbolKeys].map((symbolKey) => ({
    id: `symbol:${symbolKey}`,
    kind: "symbol",
    label: symbolKey,
    symbol_kind: "topology",
    symbol_key: symbolKey,
    symbol_id: null,
    path_display: topologySymbolPaths.get(symbolKey) ?? null,
    language: null,
    container_chain: [],
    signature: null,
    span: null,
    selection_span: null,
    freshness: "unknown",
    diagnostic_count: 0,
    diagnostic_severity: null,
    metrics: { in_degree: 0, out_degree: 0, community_id: null },
  }));
  for (const node of topologySymbolNodes) {
    if (node.symbol_key && !symbolNodeIds.has(node.symbol_key)) symbolNodeIds.set(node.symbol_key, node.id);
  }
  const symbolNodeId = (symbolKey: string): string => symbolNodeIds.get(symbolKey) ?? `symbol:${symbolKey}`;
  const existingNodeIds = new Set(snapshot.nodes.map((node) => node.id));
  const existingEdgeIds = new Set(snapshot.edges.map((edge) => edge.id));
  const topologyHintNodes = topologyEntries.flatMap(({ delta, side, suffix }) => {
    if (!side || side.target_symbol_key || (!side.target_hint && !side.unresolved)) return [];
    const id = `hint:${delta.edge_key}${suffix}`;
    if (existingNodeIds.has(id)) return [];
    return [{
      id,
      kind: "symbol" as const,
      label: side.target_hint ?? "unresolved",
      symbol_kind: "unresolved",
      symbol_key: null,
      symbol_id: null,
      path_display: null,
      language: null,
      container_chain: [],
      signature: null,
      span: null,
      selection_span: null,
      freshness: "unknown" as const,
      diagnostic_count: 0,
      diagnostic_severity: null,
      metrics: { in_degree: 0, out_degree: 0, community_id: null },
    }];
  });
  const topologyEdges = topologyEntries.flatMap(({ delta, side, suffix }) => {
    if (!side?.source_symbol_key) return [];
    const id = `${delta.edge_key}${suffix}`;
    if (existingEdgeIds.has(id)) return [];
    const targetId = side.target_symbol_key ? symbolNodeId(side.target_symbol_key) : `hint:${delta.edge_key}${suffix}`;
    return [{
      id,
      kind: side.kind as CodeGraphEdge["kind"],
      source_id: symbolNodeId(side.source_symbol_key),
      target_id: targetId,
      confidence: side.confidence,
      unresolved: side.unresolved,
      target_hint: side.target_hint ?? null,
    }];
  });
  if (syntheticNodes.length === 0 && radiusNodes.length === 0 && topologySymbolNodes.length === 0 && topologyHintNodes.length === 0 && topologyEdges.length === 0) return snapshot;
  const materializedSnapshot = {
    ...snapshot,
    nodes: [...snapshot.nodes, ...syntheticNodes, ...radiusNodes, ...topologySymbolNodes, ...topologyHintNodes.filter((node, index, nodes) => nodes.findIndex((candidate) => candidate.id === node.id) === index)],
    edges: [...snapshot.edges, ...topologyEdges],
  };
  materializedCodeDiffSnapshots.add(materializedSnapshot);
  return materializedSnapshot;
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
  overlay = normalizeCodeDiffOverlay(overlay);
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
