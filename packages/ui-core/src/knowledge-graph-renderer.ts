import {
  codeDeepLinkForFile,
  codeDeepLinkForSymbol,
  codeDeepLinkPrefix,
  formatCodeDeepLink,
  codeEdgeVisualStyle,
  codeGraphNodeDeltaStatus,
  codeGraphSnapshotForRendering,
  codeNodeVisualStyle,
  formatMemoryDeepLink,
  type GraphLayoutEdge,
  memoryDeepLinkForGraphNode,
  type GraphLayoutResult,
  type CodeGraphNode,
  type CodeGraphFilters,
  type CodeGraphSnapshot,
  type CodeGraphState,
  type CodeSymbolDetail,
  type GraphState,
  type MemoryConceptDetail,
  type MemoryGraphNode,
  type MemoryGraphSnapshot,
} from "@opensymphony/graph";
import * as THREE from "three";
import { escapeAttr, escapeHtml } from "./html.js";
import { renderMemoryMarkdown } from "./memory-markdown.js";
import {
  advanceCameraToward,
  buildGraphScene,
  defaultCameraForLayout,
  dollyCamera,
  frameWorldPoints,
  hitTestHull,
  hitTestScene,
  nodeEmphasisColor,
  orbitCamera,
  panCamera,
  unprojectToPlaneThroughPoint,
  worldNodesFor,
  worldToLayout,
  type GraphCameraState,
  type GraphScene,
  type KnowledgeGraphViewState,
  type SceneViewport,
  type WorldNode,
} from "./knowledge-graph-scene.js";

export { createKnowledgeGraphViewState } from "./knowledge-graph-scene.js";
export type { KnowledgeGraphViewState } from "./knowledge-graph-scene.js";

const graphSurfaceColor = "#eef1f4";
const graphSurfaceColorInt = 0xeef1f4;

export interface KnowledgeGraphSurface {
  snapshot: MemoryGraphSnapshot | null;
  layout: GraphLayoutResult | null;
  state: GraphState;
  /** Cached capsule detail for the selected concept (null while loading). */
  conceptDetail?: MemoryConceptDetail | null;
  conceptDetailError?: string | null;
}

export interface KnowledgeGraphMountOptions {
  snapshot: MemoryGraphSnapshot | null;
  layout: GraphLayoutResult | null;
  selectedNodeIds: readonly string[];
  view: KnowledgeGraphViewState;
  onSelect(nodeId: string): void;
  onFocus(nodeId: string): void;
  /** Click on an area cloud in the zoomed-out view: drill into that community. */
  onSelectArea?(areaId: string): void;
  nodeStyle?(node: WorldNode): {
    color?: string;
    opacity?: number;
    borderStyle?: "solid" | "dashed" | "dotted";
    freshnessLabel?: string;
  } | undefined;
  edgeStyle?(edge: GraphLayoutEdge): {
    color?: string;
    opacity?: number;
    lineStyle?: "solid" | "dashed" | "dotted";
  } | undefined;
}

export interface CodeGraphSurface {
  snapshot: CodeGraphSnapshot | null;
  layout: GraphLayoutResult | null;
  state: CodeGraphState;
  filtersOpen?: boolean;
  symbolDetail?: CodeSymbolDetail | null;
  detailError?: string | null;
  rawRecord?: boolean;
}

export function renderKnowledgeGraphSurface(surface: KnowledgeGraphSurface): string {
  const { snapshot, state } = surface;
  const status = state.layoutStatus;
  const metrics = snapshot?.metrics;
  const summary = snapshot
    ? `${snapshot.nodes.length} nodes / ${snapshot.edges.length} edges / ${metrics?.stale_concept_count ?? 0} stale / ${metrics?.warning_count ?? 0} warnings`
    : "No graph snapshot";
  // Home affordance: once the operator narrows the view (neighborhood focus,
  // a selection, or a non-atlas mode) there must always be a one-click path
  // back to the full graph.
  const narrowed = state.mode !== "atlas"
    || state.focusedNodeId !== null
    || state.selectedNodeIds.length > 0;
  const resetButton = narrowed
    ? `<button type="button" class="os-icon-button os-kg-reset" data-kg-reset data-testid="knowledge-graph-reset" title="Show the full graph (Esc)">Show full graph</button>`
    : "";
  return `
    <div class="os-knowledge-graph" data-testid="knowledge-graph-renderer" data-layout-status="${escapeAttr(status)}">
      <div class="os-knowledge-toolbar">
        <div>
          <strong>Knowledge Graph</strong>
          <span data-testid="knowledge-graph-metrics">${escapeHtml(summary)}</span>
        </div>
        ${resetButton}
        ${renderStatus(surface.state)}
      </div>
      ${renderBreadcrumb(surface)}
      <div class="os-knowledge-stage" data-kg-stage>
        <canvas class="os-knowledge-canvas" data-testid="knowledge-graph-canvas" aria-label="Knowledge Graph canvas"></canvas>
        <div class="os-knowledge-labels" data-kg-labels data-morph-ignore-children></div>
        <span class="os-kg-controls-hint" aria-hidden="true">drag pan &middot; &#8997;-drag orbit &middot; scroll zoom &middot; click area to drill in &middot; esc to back out</span>
      </div>
    </div>
  `;
}

export function renderCodeGraphSurface(surface: CodeGraphSurface): string {
  const { snapshot, state } = surface;
  const formatCount = (value: number) => value.toLocaleString("en-US");
  const truncation = snapshot?.truncation;
  const truncationSummary = truncation && (truncation.nodes_dropped > 0 || truncation.edges_dropped > 0)
    ? ` Truncated ${formatCount(truncation.nodes_dropped)} nodes and ${formatCount(truncation.edges_dropped)} edges${truncation.reason ? `: ${truncation.reason}` : "."}`
    : " No records were truncated.";
  const summary = snapshot
    ? `${formatCount(snapshot.nodes.length)} nodes / ${formatCount(snapshot.edges.length)} edges${truncation && (truncation.nodes_dropped > 0 || truncation.edges_dropped > 0) ? ` / ${formatCount(truncation.nodes_dropped)} nodes + ${formatCount(truncation.edges_dropped)} edges truncated` : ""}`
    : "No code graph snapshot";
  const accessibleSummary = snapshot
    ? `Code Graph ${state.mode} mode for ${snapshot.repo_id}: ${snapshot.nodes.length} nodes and ${snapshot.edges.length} edges.${truncationSummary} ${state.stale ? "Refreshing." : ""}`
    : "Code Graph has no loaded snapshot.";
  const narrowed = state.mode !== "atlas" || state.selectedNodeIds.length > 0 || state.path !== null || state.symbolKey !== null;
  const diffUnavailable = !state.baseRevision || !state.headRevision;
  return `
    <div class="os-knowledge-graph os-code-graph" data-testid="code-graph-renderer" data-layout-status="${escapeAttr(state.layoutStatus)}">
      <div class="os-knowledge-toolbar os-code-graph-toolbar">
        <div>
          <strong>Code Graph</strong>
          <span data-testid="code-graph-metrics">${escapeHtml(summary)}</span>
        </div>
        <div class="os-segmented" data-testid="code-graph-mode-toggle">
          ${(["atlas", "file", "neighborhood", "diff"] as const).map((mode) =>
            `<button type="button" class="${state.mode === mode ? "is-selected" : ""}" data-code-mode="${mode}"${mode === "diff" && diffUnavailable ? " disabled" : ""}>${mode[0].toUpperCase()}${mode.slice(1)}</button>`).join("")}
        </div>
        ${narrowed ? `<button type="button" class="os-icon-button os-kg-reset" data-code-reset data-testid="code-graph-reset">Show full graph</button>` : ""}
        <span class="os-kg-status" data-testid="code-graph-status">${escapeHtml(state.layoutStatus === "failed" ? state.layoutError ?? "Unavailable" : state.stale ? "Refreshing" : state.layoutStatus === "ready" ? "Ready" : "Idle")}</span>
      </div>
      ${renderCodeGraphFilters(surface)}
      ${renderCodeBreadcrumb(state)}
      <p class="os-sr-only" data-testid="code-graph-screen-reader-summary" role="status" aria-live="polite">${escapeHtml(accessibleSummary)}</p>
      <div class="os-knowledge-stage" data-kg-stage>
        <canvas class="os-knowledge-canvas os-code-graph-canvas" data-testid="code-graph-canvas" role="img" aria-label="Code Graph canvas" aria-describedby="code-graph-screen-reader-summary"></canvas>
        <div class="os-knowledge-labels" data-kg-labels data-morph-ignore-children></div>
        <span class="os-kg-controls-hint" aria-hidden="true">drag pan &middot; &#8997;-drag orbit &middot; scroll zoom &middot; double-click neighborhood &middot; esc to back out</span>
      </div>
    </div>
  `;
}

export function renderCodeGraphFilters(surface: CodeGraphSurface): string {
  const { snapshot, state } = surface;
  const filters = state.filters;
  const filterCount = Object.entries(filters).reduce((count, [key, value]) => {
    if (key === "diagnostics") return count + (value === "all" ? 0 : 1);
    return count + (Array.isArray(value) ? value.length : 0);
  }, 0);
  const repoValues = state.repos?.repos.map((repo) => repo.repo_id) ?? [];
  const languageValues = [
    ...((state.repos?.repos ?? []).flatMap((repo) => repo.languages)),
    ...(snapshot?.nodes.map((node) => node.language).filter((value): value is string => Boolean(value)) ?? []),
  ];
  const nodeValues = snapshot?.nodes.map((node) => node.symbol_kind).filter((value): value is string => Boolean(value)) ?? [];
  const edgeValues = snapshot?.edges.map((edge) => edge.kind) ?? [];
  const communityValues = snapshot?.communities.map((community) => community.id) ?? [];
  const groups: Array<[Exclude<keyof CodeGraphFilters, "diagnostics" | "pathPrefixes">, string, string[]]> = [
    ["repoIds", "Repository", repoValues],
    ["languages", "Language", languageValues],
    ["symbolKinds", "Symbol kind", nodeValues],
    ["edgeKinds", "Edge kind", edgeValues],
    ["confidences", "Confidence", ["exact", "syntactic", "heuristic"]],
    ["freshness", "Freshness", ["current", "stale", "unknown"]],
    ["communities", "Community", communityValues],
  ];
  if (state.mode === "diff" && state.diffOverlay) {
    groups.push(["deltaStatuses", "Delta status", ["added", "removed", "modified", "unchanged"]]);
  }
  const checkboxGroups = groups.map(([key, label, rawValues]) => {
    const values = [...new Set(rawValues)].sort();
    if (values.length === 0) return "";
    const selected = new Set(filters[key] as readonly string[]);
    return `<fieldset class="os-code-filter-group"><legend>${escapeHtml(label)}</legend>${values.map((value) => `
      <label><input type="checkbox" data-code-filter="${key}" data-code-filter-value="${escapeAttr(value)}" ${selected.has(value) ? "checked" : ""} /> ${escapeHtml(value)}</label>
    `).join("")}</fieldset>`;
  }).join("");
  return `
    <details class="os-code-filters" data-testid="code-graph-filters"${surface.filtersOpen ? " open" : ""}>
      <summary>Filters${filterCount > 0 ? ` (${filterCount})` : ""}</summary>
      <div class="os-code-filter-grid">
        ${checkboxGroups}
        <label class="os-code-filter-path">Path prefix
          <input type="text" data-code-filter="pathPrefixes" value="${escapeAttr(filters.pathPrefixes.join(", "))}" placeholder="packages/graph/" />
        </label>
        <label class="os-code-filter-diagnostics">Diagnostics
          <select data-code-filter="diagnostics">
            ${["all", "with_diagnostics", "without_diagnostics"].map((value) => `<option value="${value}" ${filters.diagnostics === value ? "selected" : ""}>${value.replaceAll("_", " ")}</option>`).join("")}
          </select>
        </label>
        <button type="button" data-code-filter-reset>Reset filters</button>
      </div>
    </details>
  `;
}

function renderCodeBreadcrumb(state: CodeGraphState): string {
  if (state.breadcrumbs.length === 0) return "";
  const crumbs = [`<button type="button" data-code-crumb="-1">Repo</button>`];
  state.breadcrumbs.forEach((crumb, index) => {
    crumbs.push(
      index === state.breadcrumbs.length - 1
        ? `<span aria-current="location">${escapeHtml(crumb.label)}</span>`
        : `<button type="button" data-code-crumb="${index}">${escapeHtml(crumb.label)}</button>`,
    );
  });
  return `<nav class="os-kg-breadcrumb os-code-breadcrumb" data-testid="code-graph-breadcrumb" aria-label="Code Graph drill path">${crumbs.join(`<span class="os-kg-crumb-sep" aria-hidden="true">&rsaquo;</span>`)}</nav>`;
}

export function renderCodeGraphNodeList(
  snapshot: CodeGraphSnapshot | null,
  selectedNodeIds: readonly string[],
  diffOverlay: CodeGraphState["diffOverlay"] = null,
): string {
  if (!snapshot || snapshot.nodes.length === 0) return `<div class="os-empty" data-testid="code-graph-structure-list">No code structure matches the current filters.</div>`;
  const selected = new Set(selectedNodeIds);
  return `
    <ul class="os-kg-list os-code-graph-list" data-testid="code-graph-structure-list" aria-label="Code structure list">
      ${snapshot.nodes.map((node) => {
        const deltaStatus = codeGraphNodeDeltaStatus(node.symbol_key, diffOverlay);
        return `
          <li class="${selected.has(node.id) ? "is-selected" : ""}" data-code-node-kind="${escapeAttr(node.kind)}">
            <button type="button" data-kg-node-id="${escapeAttr(node.id)}" data-code-node-id="${escapeAttr(node.id)}" ${selected.has(node.id) ? `aria-current="true"` : ""}>${escapeHtml(node.label)}</button>
            <span class="os-code-node-meta">${escapeHtml(node.symbol_kind ?? node.kind)} · ${escapeHtml(node.freshness)}</span>
            ${diffOverlay && deltaStatus !== "unchanged" ? `<span class="os-code-delta-badge" data-code-delta-status="${deltaStatus}">${deltaStatus}</span>` : ""}
            ${node.diagnostic_count > 0 ? `<span class="os-code-diagnostic-badge">${node.diagnostic_count} diagnostic${node.diagnostic_count === 1 ? "" : "s"}</span>` : ""}
          </li>
        `;
      }).join("")}
    </ul>
  `;
}

export function renderCodeGraphInspector(surface: CodeGraphSurface): string {
  const snapshot = surface.snapshot;
  const selected = new Set(surface.state.selectedNodeIds);
  const node = snapshot?.nodes.find((candidate) => selected.has(candidate.id)) ?? null;
  if (!node) return `<section class="os-code-inspector" data-testid="code-graph-detail"><h3>Symbol Detail</h3><p>No code record selected</p></section>`;
  const detail = surface.symbolDetail ?? null;
  const deltaStatus = codeGraphNodeDeltaStatus(node.symbol_key, surface.state.diffOverlay);
  const deepLink = codeDeepLinkForNode(surface.state, node);
  const raw = surface.rawRecord ? JSON.stringify(detail ?? node, null, 2) : "";
  return `
    <section class="os-code-inspector" data-testid="code-graph-detail" data-code-freshness="${escapeAttr(node.freshness)}">
      <div class="os-kg-inspector-header">
        <div><h3>${escapeHtml(detail?.name ?? node.label)}</h3><span>${escapeHtml(detail?.kind ?? node.kind)}</span></div>
        ${deepLink ? `<button type="button" class="os-kg-copy-deeplink" data-code-copy-deeplink="${escapeAttr(deepLink)}">Copy deep link</button>` : ""}
      </div>
      <dl>
        <dt>Path</dt><dd>${escapeHtml(detail?.path_display ?? node.path_display ?? "—")}</dd>
        <dt>Language</dt><dd>${escapeHtml(detail?.language ?? node.language ?? "—")}</dd>
        <dt>Freshness</dt><dd data-code-freshness-badge="${escapeAttr(node.freshness)}">${escapeHtml(node.freshness)}${node.diagnostic_count > 0 ? ` · ${node.diagnostic_count} diagnostics` : ""}</dd>
        <dt>Delta</dt><dd data-code-delta-status="${deltaStatus}">${escapeHtml(deltaStatus)}</dd>
        <dt>Signature</dt><dd>${escapeHtml(detail?.signature ?? node.signature ?? "—")}</dd>
        <dt>Container</dt><dd>${escapeHtml((detail?.container_chain ?? node.container_chain).join(" › ") || "—")}</dd>
      </dl>
      ${detail
        ? renderCodeDetailSections(detail)
        : node.kind === "symbol" && deltaStatus !== "removed" && !surface.detailError
          ? `<p data-testid="code-graph-detail-loading">Loading symbol detail…</p>`
          : renderCodeNodeFallback(node, surface.detailError)}
      ${detail ? renderCodeCrossGraphChips(detail) : ""}
      <button type="button" data-code-raw-toggle>${surface.rawRecord ? "Hide raw record" : "Show raw record"}</button>
      ${surface.rawRecord ? `<pre data-testid="code-graph-raw-record">${escapeHtml(raw)}</pre>` : ""}
    </section>
  `;
}

function renderCodeNodeFallback(node: CodeGraphNode, detailError?: string | null): string {
  return `
    <h4>Record</h4>
    <dl>
      <dt>Kind</dt><dd>${escapeHtml(node.kind)}</dd>
      <dt>Path</dt><dd>${escapeHtml(node.path_display ?? "—")}</dd>
      <dt>Freshness</dt><dd>${escapeHtml(node.freshness)}</dd>
      <dt>Children</dt><dd>${node.metrics.out_degree}</dd>
    </dl>
    <p data-testid="code-graph-file-fallback">${detailError ? "Symbol detail unavailable; showing the graph record." : "No symbol detail is required for this record."}</p>
  `;
}

function renderCodeDetailSections(detail: CodeSymbolDetail): string {
  const diagnostics = detail.diagnostics.length > 0
    ? `<h4>Diagnostics</h4><ul>${detail.diagnostics.map((diagnostic) => `<li>${escapeHtml(diagnostic.severity)}: ${escapeHtml(diagnostic.message)}</li>`).join("")}</ul>`
    : `<p data-testid="code-graph-no-diagnostics">No diagnostics</p>`;
  return `
    <h4>Provenance</h4>
    <dl><dt>Content</dt><dd>${escapeHtml(detail.provenance.content_sha256)}</dd><dt>Parser</dt><dd>${escapeHtml(detail.provenance.parser_version)}</dd><dt>Query pack</dt><dd>${escapeHtml(detail.provenance.query_pack_version)}</dd></dl>
    <h4>Relationships</h4>
    <ul>${detail.edge_summary.map((edge) => `<li data-code-confidence="${escapeAttr(edge.confidence)}">${escapeHtml(edge.kind)} · ${escapeHtml(edge.confidence)} · ${edge.count}</li>`).join("") || "<li>None</li>"}</ul>
    ${diagnostics}
    ${detail.source_snippet ? `<h4>Source</h4><pre>${escapeHtml(detail.source_snippet.text)}</pre>` : ""}
  `;
}

function renderCodeCrossGraphChips(detail: CodeSymbolDetail): string {
  const issues = detail.related_issues ?? [];
  const concepts = detail.related_memory_concepts ?? [];
  const issueChips = issues.map((issue) => issue.freshness === "current"
    ? `<button type="button" class="os-cross-graph-chip" data-task-issue-key="${escapeAttr(issue.issue_key)}" data-testid="code-graph-issue-chip">${escapeHtml(issue.issue_key)}: ${escapeHtml(issue.title)}</button>`
    : `<span class="os-cross-graph-chip is-stale" data-testid="code-graph-issue-chip">${escapeHtml(issue.issue_key)}: ${escapeHtml(issue.title)} (stale)</span>`).join("");
  const memoryChips = concepts.map((concept) => {
    const link = formatMemoryDeepLink({ bundleId: concept.bundle_id, conceptId: concept.concept_id });
    return concept.freshness === "current"
      ? `<button type="button" class="os-cross-graph-chip" data-memory-deeplink="${escapeAttr(link)}" data-testid="code-graph-memory-chip">${escapeHtml(concept.title)}</button>`
      : `<span class="os-cross-graph-chip is-stale" data-testid="code-graph-memory-chip">${escapeHtml(concept.title)} (stale)</span>`;
  }).join("");
  return `
    <section class="os-cross-graph-links" data-testid="code-graph-cross-links">
      <h4>Related work</h4>
      ${issueChips || `<p>No related work items found.</p>`}
      <h4>Related memory</h4>
      ${memoryChips || `<p>No related memory concepts found.</p>`}
    </section>
  `;
}

function codeDeepLinkForNode(state: CodeGraphState, node: CodeGraphNode): string | null {
  try {
    const overlayOnly = Boolean(
      state.mode === "diff"
      && state.diffOverlay
      && node.symbol_key
      && !state.snapshot?.nodes.some((candidate) => candidate.symbol_key === node.symbol_key),
    );
    if (overlayOnly) return null;
    const filters = state.filters.deltaStatuses.length > 0
      ? { ...state.filters, deltaStatuses: [] }
      : state.filters;
    if (state.mode === "diff" && state.baseRevision && state.headRevision) {
      return formatCodeDeepLink({
        repoId: state.repoId ?? "",
        symbolKey: node.symbol_key,
        path: node.symbol_key ? null : node.path_display,
        baseRevision: state.baseRevision,
        headRevision: state.headRevision,
        depth: state.depth,
        filters,
        layoutSeed: state.layoutSeed,
      });
    }
    if (node.symbol_key) return codeDeepLinkForSymbol(state.repoId ?? "", node.symbol_key, { mode: "neighborhood", depth: state.depth, filters, layoutSeed: state.layoutSeed });
    if (node.path_display) return codeDeepLinkForFile(state.repoId ?? "", node.path_display, { mode: state.mode === "diff" ? "file" : state.mode, depth: state.depth, filters, layoutSeed: state.layoutSeed });
  } catch {
    return null;
  }
  return null;
}

/**
 * Drill trail: Atlas › area › concept. Each ancestor is a button that pops
 * the view back to that level; the current level renders as plain text.
 * Hidden entirely at the atlas level so the default view stays quiet.
 */
function renderBreadcrumb(surface: KnowledgeGraphSurface): string {
  const { snapshot, state } = surface;
  const communityId = state.filters.communities[0] ?? null;
  const community = communityId
    ? snapshot?.communities.find((candidate) => candidate.id === communityId) ?? null
    : null;
  const selected = new Set(state.selectedNodeIds);
  const node = snapshot?.nodes.find((candidate) => selected.has(candidate.id)) ?? null;
  if (!community && !node) return "";
  const crumbs: string[] = [
    `<button type="button" data-kg-crumb="atlas">Atlas</button>`,
  ];
  if (community) {
    crumbs.push(
      node
        ? `<button type="button" data-kg-crumb="community">${escapeHtml(community.label)}</button>`
        : `<span aria-current="location">${escapeHtml(community.label)}</span>`,
    );
  }
  if (node) {
    crumbs.push(`<span aria-current="location">${escapeHtml(node.label)}</span>`);
  }
  return `
    <nav class="os-kg-breadcrumb" data-testid="knowledge-graph-breadcrumb" aria-label="Knowledge Graph drill path">
      ${crumbs.join(`<span class="os-kg-crumb-sep" aria-hidden="true">&rsaquo;</span>`)}
    </nav>
  `;
}

// ─── Renderer state ─────────────────────────────────────────────────────────

interface RendererState {
  options: KnowledgeGraphMountOptions;
  camera: GraphCameraState;
  goal: GraphCameraState;
  hoveredNodeId: string | null;
  pointer: {
    mode: "idle" | "pan" | "orbit" | "drag-node";
    nodeId: string | null;
    lastX: number;
    lastY: number;
    moved: boolean;
  };
  rafHandle: number | ReturnType<typeof setTimeout> | null;
  rafKind: "animation-frame" | "timeout";
  lastFrameAt: number;
  animating: boolean;
  reducedMotion: boolean;
  canvasSize: { width: number; height: number; cssWidth: number; cssHeight: number };
  lastScene: GraphScene | null;
  layoutIdentity: string | null;
}

const rendererStates = new WeakMap<HTMLCanvasElement, RendererState>();

export function mountKnowledgeGraphRenderer(
  root: HTMLElement,
  options: KnowledgeGraphMountOptions,
): void {
  mountGraphRenderer(root, options, "knowledge-graph-canvas");
}

export function mountCodeGraphRenderer(
  root: HTMLElement,
  options: KnowledgeGraphMountOptions,
): void {
  mountGraphRenderer(root, options, "code-graph-canvas");
}

function mountGraphRenderer(
  root: HTMLElement,
  options: KnowledgeGraphMountOptions,
  canvasTestId: "knowledge-graph-canvas" | "code-graph-canvas",
): void {
  const canvas = root.querySelector<HTMLCanvasElement>(`[data-testid='${canvasTestId}']`);
  if (!canvas) return;
  if (!options.snapshot || !options.layout) {
    // The morphing render preserves the canvas node, so without drawable
    // data the previous mount's bitmap and handlers would otherwise stay
    // live and interactive against a stale layout. Always detach the
    // interaction handlers and clear the label/tooltip overlays (they are
    // positioned against the old layout and remain clickable otherwise);
    // drop the bitmap too once the snapshot itself is gone (bundle
    // switch/reset). During a pure re-layout the snapshot is still present
    // and keeping the bitmap avoids a blank flash.
    detachCanvasHandlers(canvas);
    clearOverlayContainer(root);
    if (!options.snapshot) {
      disposeKnowledgeGraphCanvas(canvas);
      canvas.width = canvas.width || 1;
      canvas.dataset.nonblank = "false";
    }
    return;
  }

  const stage = canvas.closest<HTMLElement>("[data-kg-stage]");
  const viewport = canvasViewport(stage);
  const layoutIdentity = `${options.layout.kind}:${options.layout.width}:${options.layout.height}:${options.layout.generatedAt}`;
  let state = rendererStates.get(canvas);
  if (!state) {
    const camera = options.view.camera ?? defaultCameraForLayout(options.layout, viewport);
    state = {
      options,
      camera,
      goal: camera,
      hoveredNodeId: null,
      pointer: { mode: "idle", nodeId: null, lastX: 0, lastY: 0, moved: false },
      rafHandle: null,
      rafKind: "animation-frame",
      lastFrameAt: 0,
      animating: false,
      reducedMotion: prefersReducedMotion(),
      canvasSize: { width: 0, height: 0, cssWidth: 0, cssHeight: 0 },
      lastScene: null,
      layoutIdentity,
    };
    rendererStates.set(canvas, state);
  } else {
    state.options = options;
    if (options.view.camera) {
      state.camera = options.view.camera;
      if (!state.animating) state.goal = options.view.camera;
    }
    if (state.layoutIdentity !== layoutIdentity) {
      state.layoutIdentity = layoutIdentity;
      state.hoveredNodeId = null;
      // A new layout means the content genuinely changed (identical
      // snapshots no longer trigger relayouts): mode switches, bundle
      // switches, and resize relayouts all reposition nodes, so glide the
      // camera toward framing the new layout instead of keeping a view
      // that may point entirely off-frame.
      state.goal = defaultCameraForLayout(options.layout, viewport);
    }
  }
  options.view.camera = state.camera;

  attachCanvasHandlers(canvas, root, stage, state);
  if (cameraDiffers(state.camera, state.goal)) {
    startAnimation(canvas, root, stage, state);
  } else {
    drawFrame(canvas, root, stage, state);
  }
  bindListNavigation(root, options);
}

function cameraDiffers(a: GraphCameraState, b: GraphCameraState): boolean {
  return a.targetX !== b.targetX
    || a.targetY !== b.targetY
    || a.targetZ !== b.targetZ
    || a.distance !== b.distance
    || a.yaw !== b.yaw
    || a.pitch !== b.pitch;
}

export function disposeKnowledgeGraphRenderer(root: ParentNode): void {
  disposeGraphRenderers(root, "knowledge-graph-canvas");
}

export function disposeCodeGraphRenderer(root: ParentNode): void {
  disposeGraphRenderers(root, "code-graph-canvas");
}

function disposeGraphRenderers(
  root: ParentNode,
  canvasTestId: "knowledge-graph-canvas" | "code-graph-canvas",
): void {
  root.querySelectorAll<HTMLCanvasElement>(`[data-testid='${canvasTestId}']`).forEach((canvas) => {
    disposeKnowledgeGraphCanvas(canvas);
  });
}

export function disposeKnowledgeGraphCanvas(canvas: HTMLCanvasElement): void {
  const state = rendererStates.get(canvas);
  if (state?.rafHandle !== null && state?.rafHandle !== undefined) {
    if (state.rafKind === "animation-frame" && typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(state.rafHandle as number);
    } else {
      clearTimeout(state.rafHandle as ReturnType<typeof setTimeout>);
    }
    state.rafHandle = null;
  }
  rendererStates.delete(canvas);
  resetThreeCanvasState(canvas);
}

// ─── Interaction ────────────────────────────────────────────────────────────

function canvasPointFromEvent(
  canvas: HTMLCanvasElement,
  event: MouseEvent,
): { x: number; y: number } {
  // offsetX/offsetY are relative to the event *target*, which may be an
  // overlay label the pointer is passing over (events bubble to the canvas
  // handlers) — always resolve against the canvas box instead.
  const rect = canvas.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}

function detachCanvasHandlers(canvas: HTMLCanvasElement): void {
  canvas.onwheel = null;
  canvas.onpointerdown = null;
  canvas.onpointermove = null;
  canvas.onpointerup = null;
  canvas.onpointerleave = null;
  canvas.ondblclick = null;
  canvas.oncontextmenu = null;
}

function attachCanvasHandlers(
  canvas: HTMLCanvasElement,
  root: HTMLElement,
  stage: HTMLElement | null,
  state: RendererState,
): void {
  canvas.oncontextmenu = (event) => {
    event.preventDefault();
  };
  canvas.onwheel = (event) => {
    event.preventDefault();
    const viewport = canvasViewport(stage);
    const factor = event.deltaY < 0 ? 0.88 : 1.14;
    const point = canvasPointFromEvent(canvas, event);
    state.goal = dollyCamera(state.goal, factor, {
      viewport,
      screenX: point.x,
      screenY: point.y,
    });
    startAnimation(canvas, root, stage, state);
  };
  canvas.onpointerdown = (event) => {
    try {
      canvas.setPointerCapture(event.pointerId);
    } catch {
      // Pointer capture is an enhancement (keeps drags alive outside the
      // canvas); synthetic pointers in tests have no capturable id.
    }
    const viewport = canvasViewport(stage);
    const scene = state.lastScene ?? rebuildScene(state, viewport);
    const point = canvasPointFromEvent(canvas, event);
    const nodeId = event.button === 0 && !event.altKey && !event.metaKey
      ? hitTestScene(scene, point.x, point.y)
      : null;
    state.pointer = {
      mode: nodeId
        ? "drag-node"
        : event.button === 2 || event.altKey || event.metaKey
          ? "orbit"
          : "pan",
      nodeId,
      lastX: event.clientX,
      lastY: event.clientY,
      moved: false,
    };
    canvas.dataset.kgPointer = state.pointer.mode;
  };
  canvas.onpointermove = (event) => {
    const viewport = canvasViewport(stage);
    if (state.pointer.mode === "idle") {
      const scene = state.lastScene ?? rebuildScene(state, viewport);
      const point = canvasPointFromEvent(canvas, event);
      const nodeId = hitTestScene(scene, point.x, point.y);
      if (nodeId !== state.hoveredNodeId) {
        state.hoveredNodeId = nodeId;
        drawFrame(canvas, root, stage, state);
      }
      canvas.style.cursor = nodeId || drillableHullAt(scene, point.x, point.y) ? "pointer" : "grab";
      return;
    }
    const deltaX = event.clientX - state.pointer.lastX;
    const deltaY = event.clientY - state.pointer.lastY;
    if (Math.abs(deltaX) + Math.abs(deltaY) > 2) state.pointer.moved = true;
    state.pointer.lastX = event.clientX;
    state.pointer.lastY = event.clientY;
    if (state.pointer.mode === "pan") {
      state.camera = panCamera(state.camera, viewport, deltaX, deltaY);
      state.goal = state.camera;
    } else if (state.pointer.mode === "orbit") {
      // Grab-the-scene orbit, tuned by feel: dragging right swings the
      // scene right (yaw inverted from the raw delta), and dragging up
      // tilts the scene away/up (pitch follows the raw delta). Signs are
      // deliberate — both once mapped straight onto yaw/pitch and read
      // reversed horizontally; a full inversion read reversed vertically.
      state.camera = orbitCamera(state.camera, -deltaX * 0.005, deltaY * 0.004);
      state.goal = state.camera;
    } else if (state.pointer.mode === "drag-node" && state.pointer.nodeId && state.options.layout) {
      const worldNode = worldNodesFor(state.options.layout, state.options.view.overrides)
        .find((node) => node.nodeId === state.pointer.nodeId);
      if (worldNode) {
        const dragPoint = canvasPointFromEvent(canvas, event);
        const dropped = unprojectToPlaneThroughPoint(
          state.camera,
          viewport,
          dragPoint.x,
          dragPoint.y,
          worldNode,
        );
        if (dropped) {
          state.options.view.overrides.set(state.pointer.nodeId, {
            ...worldToLayout(state.options.layout, dropped),
            z: dropped.z,
          });
        }
      }
    }
    state.options.view.camera = state.camera;
    drawFrame(canvas, root, stage, state);
  };
  canvas.onpointerup = (event) => {
    try {
      canvas.releasePointerCapture(event.pointerId);
    } catch {
      // Matching guard for synthetic pointers; see onpointerdown.
    }
    const { mode, nodeId, moved } = state.pointer;
    state.pointer = { mode: "idle", nodeId: null, lastX: 0, lastY: 0, moved: false };
    delete canvas.dataset.kgPointer;
    canvas.style.cursor = "grab";
    if (!moved && mode === "drag-node" && nodeId) {
      state.options.onSelect(nodeId);
      return;
    }
    if (!moved && mode === "pan") {
      // A stationary click on an area cloud drills into that community; the
      // same gesture with movement stays a pan.
      const viewport = canvasViewport(stage);
      const scene = state.lastScene ?? rebuildScene(state, viewport);
      const point = canvasPointFromEvent(canvas, event);
      const hull = drillableHullAt(scene, point.x, point.y);
      if (hull) state.options.onSelectArea?.(hull.areaId);
    }
  };
  canvas.onpointerleave = () => {
    if (state.hoveredNodeId !== null && state.pointer.mode === "idle") {
      state.hoveredNodeId = null;
      drawFrame(canvas, root, stage, state);
    }
  };
  canvas.ondblclick = (event) => {
    event.preventDefault();
    const viewport = canvasViewport(stage);
    const scene = state.lastScene ?? rebuildScene(state, viewport);
    const layout = state.options.layout;
    if (!layout) return;
    const worldNodes = worldNodesFor(layout, state.options.view.overrides);
    const point = canvasPointFromEvent(canvas, event);
    const nodeId = hitTestScene(scene, point.x, point.y);
    if (nodeId) {
      // Frame the node and its direct neighborhood.
      const neighborhood = new Set([nodeId]);
      for (const edge of layout.edges) {
        if (edge.sourceId === nodeId) neighborhood.add(edge.targetId);
        if (edge.targetId === nodeId) neighborhood.add(edge.sourceId);
      }
      const points = worldNodes.filter((node) => neighborhood.has(node.nodeId));
      state.goal = frameWorldPoints(points, viewport, state.camera, 1.35);
      startAnimation(canvas, root, stage, state);
      state.options.onSelect(nodeId);
      return;
    }
    const hull = hitTestHull(scene, point.x, point.y);
    if (hull) {
      const members = new Set(
        state.options.snapshot?.communities.find((community) => community.id === hull.areaId)?.node_ids ?? [],
      );
      const points = worldNodes.filter((node) => members.has(node.nodeId));
      if (points.length > 0) {
        state.goal = frameWorldPoints(points, viewport, state.camera, 1.25);
        startAnimation(canvas, root, stage, state);
        return;
      }
    }
    state.goal = defaultCameraForLayout(layout, viewport);
    startAnimation(canvas, root, stage, state);
  };
}

/**
 * Hull under the cursor, but only while the view is zoomed out enough that
 * areas read as navigable clouds (their titles are visible). Zoomed in, the
 * hulls still exist underneath every node and a background click should not
 * yank the operator into a different community.
 */
function drillableHullAt(scene: GraphScene, x: number, y: number): { areaId: string } | null {
  const hull = hitTestHull(scene, x, y);
  return hull && hull.labelAlpha > 0.05 ? hull : null;
}

/** The subset of mount callbacks the node list/labels need. */
export interface KnowledgeGraphListOptions {
  onSelect(nodeId: string): void;
  onFocus(nodeId: string): void;
}

/**
 * Bind click/keyboard/focus behavior for every node button under `root`
 * (overlay labels and the entity-list column alike). Handler properties
 * (not addEventListener) so re-binding after every render stays idempotent:
 * the DOM morph preserves these buttons across renders, and stacked
 * listeners would fire once per past render.
 */
export function bindKnowledgeGraphListNavigation(root: HTMLElement, options: KnowledgeGraphListOptions): void {
  root.querySelectorAll<HTMLElement>("[data-kg-node-id]").forEach((button) => {
    bindNodeButton(root, button, options);
    markTruncatedListLabel(button);
  });
}

/**
 * Entity-list rows ellipsize long names; flag the ones that actually
 * overflow so CSS can surface the full name in an instant (no-delay)
 * hover tooltip. `title` is kept in sync as the assistive/native fallback.
 */
function markTruncatedListLabel(button: HTMLElement): void {
  if (!button.closest(".os-kg-list")) return;
  const label = button.textContent ?? "";
  if (button.scrollWidth > button.clientWidth && label) {
    button.dataset.kgOverflow = label;
    button.title = label;
  } else {
    delete button.dataset.kgOverflow;
    button.removeAttribute("title");
  }
}

const bindListNavigation = bindKnowledgeGraphListNavigation;

function bindNodeButton(root: HTMLElement, button: HTMLElement, options: KnowledgeGraphListOptions): void {
  button.onclick = () => {
    const nodeId = button.dataset.kgNodeId;
    if (nodeId) options.onSelect(nodeId);
  };
  button.onkeydown = (event) => {
    const direction = graphListNavigationDirection(event.key);
    if (!direction) return;
    const buttons = Array.from(root.querySelectorAll<HTMLElement>(".os-kg-list [data-kg-node-id]"));
    const index = buttons.indexOf(button);
    if (index < 0) return;
    event.preventDefault();
    const nextIndex = direction === "first"
      ? 0
      : direction === "last"
        ? buttons.length - 1
        : (index + direction + buttons.length) % buttons.length;
    buttons[nextIndex]?.focus();
  };
  button.onfocus = () => {
    const nodeId = button.dataset.kgNodeId;
    // Focus-driven neighborhood preview is a keyboard affordance; a mouse
    // click also focuses the button, and jumping into neighborhood mode on
    // every click stranded users away from the full graph.
    if (nodeId && matchesFocusVisible(button)) options.onFocus(nodeId);
  };
}

// ─── Frame pipeline ─────────────────────────────────────────────────────────

function startAnimation(
  canvas: HTMLCanvasElement,
  root: HTMLElement,
  stage: HTMLElement | null,
  state: RendererState,
): void {
  if (state.reducedMotion) {
    state.camera = state.goal;
    state.options.view.camera = state.camera;
    drawFrame(canvas, root, stage, state);
    return;
  }
  state.animating = true;
  if (state.rafHandle !== null) return;
  state.lastFrameAt = now();
  const step = () => {
    state.rafHandle = null;
    if (!canvas.isConnected) return;
    const at = now();
    const deltaSeconds = Math.min(0.1, (at - state.lastFrameAt) / 1000);
    state.lastFrameAt = at;
    const advanced = advanceCameraToward(state.camera, state.goal, deltaSeconds);
    state.camera = advanced.camera;
    state.options.view.camera = state.camera;
    drawFrame(canvas, root, stage, state);
    if (!advanced.done) {
      schedule();
    } else {
      state.animating = false;
    }
  };
  const schedule = () => {
    if (typeof requestAnimationFrame === "function") {
      state.rafKind = "animation-frame";
      state.rafHandle = requestAnimationFrame(step);
    } else {
      state.rafKind = "timeout";
      state.rafHandle = setTimeout(step, 16);
    }
  };
  schedule();
}

function rebuildScene(state: RendererState, viewport: SceneViewport): GraphScene {
  const scene = buildGraphScene({
    layout: state.options.layout!,
    communities: state.options.snapshot?.communities ?? [],
    camera: state.camera,
    viewport,
    overrides: state.options.view.overrides,
    selectedNodeIds: state.options.selectedNodeIds,
    hoveredNodeId: state.hoveredNodeId,
    nodeStyle: state.options.nodeStyle,
    edgeStyle: state.options.edgeStyle,
  });
  state.lastScene = scene;
  return scene;
}

function drawFrame(
  canvas: HTMLCanvasElement,
  root: HTMLElement,
  stage: HTMLElement | null,
  state: RendererState,
): void {
  const viewport = canvasViewport(stage);
  state.canvasSize = resizeCanvasIfNeeded(canvas, viewport, state.canvasSize);
  const scene = rebuildScene(state, viewport);
  if (!drawThree(canvas, viewport, scene)) {
    drawCanvas2d(canvas, scene);
  }
  syncOverlay(root, state, scene);
  canvas.dataset.nonblank = scene.nodes.length > 0 ? "true" : "false";
  canvas.dataset.reducedMotion = state.reducedMotion ? "true" : "false";
  // Debug/E2E handle: the last drawn scene, camera, and viewport.
  (canvas as HTMLCanvasElement & { __kgDebug?: unknown }).__kgDebug = {
    camera: state.camera,
    viewport,
    nodes: scene.nodes.length,
    scene,
  };
}

// ─── HTML overlay: labels, area titles, tooltip ─────────────────────────────

function overlayContainer(root: HTMLElement): HTMLElement | null {
  return root.querySelector<HTMLElement>("[data-kg-labels]");
}

function clearOverlayContainer(root: ParentNode): void {
  const container = (root as HTMLElement).querySelector?.<HTMLElement>("[data-kg-labels]");
  container?.replaceChildren();
}

function syncOverlay(
  root: HTMLElement,
  state: RendererState,
  scene: GraphScene,
): void {
  const container = overlayContainer(root);
  if (!container) return;
  const document = container.ownerDocument;
  const seen = new Set<Element>();

  // Area titles surface when zoomed out, replacing the node-label noise.
  // A light declutter pass nudges titles apart when neighboring clusters
  // overlap so two areas never render on top of each other.
  const placedAreaLabels: Array<{ x: number; y: number; halfWidth: number }> = [];
  for (const hull of scene.hulls) {
    if (hull.labelAlpha <= 0.02) continue;
    let label = container.querySelector<HTMLElement>(`[data-kg-area-label="${cssEscape(hull.areaId)}"]`);
    if (!label) {
      label = document.createElement("div");
      label.className = "os-kg-area-label";
      label.dataset.kgAreaLabel = hull.areaId;
      container.appendChild(label);
    }
    if (label.textContent !== hull.label) label.textContent = hull.label;
    const halfWidth = hull.label.length * 5.4;
    let labelY = hull.labelY;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const collision = placedAreaLabels.find((placed) =>
        Math.abs(placed.y - labelY) < 24
        && Math.abs(placed.x - hull.labelX) < placed.halfWidth + halfWidth,
      );
      if (!collision) break;
      labelY += 26;
    }
    placedAreaLabels.push({ x: hull.labelX, y: labelY, halfWidth });
    label.style.left = `${hull.labelX.toFixed(1)}px`;
    label.style.top = `${labelY.toFixed(1)}px`;
    label.style.opacity = hull.labelAlpha.toFixed(2);
    label.style.color = hull.color;
    seen.add(label);
  }

  for (const node of scene.nodes) {
    if (node.labelAlpha <= 0.05) continue;
    let label = container.querySelector<HTMLElement>(`.os-kg-label[data-kg-node-id="${cssEscape(node.nodeId)}"]`);
    if (!label) {
      label = document.createElement("button");
      (label as HTMLButtonElement).type = "button";
      label.className = "os-kg-label";
      label.dataset.kgNodeId = node.nodeId;
      container.appendChild(label);
      bindNodeButton(root, label, state.options);
    }
    const text = shortLabel(node.labelText);
    if (label.textContent !== text) label.textContent = text;
    label.style.left = `${node.x.toFixed(1)}px`;
    label.style.top = `${(node.y + node.screenRadius + 3).toFixed(1)}px`;
    label.style.opacity = node.labelAlpha.toFixed(2);
    label.classList.toggle("is-selected", node.emphasis === "selected");
    label.classList.toggle("is-hovered", node.emphasis === "hovered");
    if (node.freshnessLabel) label.dataset.freshness = node.freshnessLabel;
    seen.add(label);
  }

  const tooltip = syncTooltip(container, state, scene);
  if (tooltip) seen.add(tooltip);

  for (const child of Array.from(container.children)) {
    if (!seen.has(child) && child !== document.activeElement) {
      child.remove();
    }
  }
}

function syncTooltip(
  container: HTMLElement,
  state: RendererState,
  scene: GraphScene,
): HTMLElement | null {
  const hovered = scene.hoveredNodeId
    ? scene.nodes.find((node) => node.nodeId === scene.hoveredNodeId)
    : null;
  let tooltip = container.querySelector<HTMLElement>("[data-kg-tooltip]");
  if (!hovered) {
    tooltip?.remove();
    return null;
  }
  const document = container.ownerDocument;
  if (!tooltip) {
    tooltip = document.createElement("div");
    tooltip.className = "os-kg-tooltip";
    tooltip.dataset.kgTooltip = "true";
    tooltip.setAttribute("role", "status");
    container.appendChild(tooltip);
  }
  const snapshotNode = state.options.snapshot?.nodes.find((node) => node.id === hovered.nodeId);
  const areas = state.options.snapshot?.communities
    .filter((community) => community.node_ids.includes(hovered.nodeId))
    .map((community) => community.label)
    ?? [];
  const degree = (snapshotNode?.metrics?.indegree ?? 0) + (snapshotNode?.metrics?.outdegree ?? 0);
  tooltip.innerHTML = `
    <strong>${escapeHtml(hovered.labelText)}</strong>
    <span>${escapeHtml(snapshotNode?.kind ?? "node")}${degree ? ` &middot; ${degree} links` : ""}</span>
    ${areas.length > 0 ? `<em>${escapeHtml(areas.join(" · "))}</em>` : ""}
  `;
  positionTooltip(tooltip, hovered.x, hovered.y - hovered.screenRadius - 10);
  return tooltip;
}

function positionTooltip(tooltip: HTMLElement, x: number, y: number): void {
  tooltip.style.left = `${x.toFixed(1)}px`;
  tooltip.style.top = `${Math.max(8, y).toFixed(1)}px`;
}

// ─── WebGL backend ──────────────────────────────────────────────────────────

interface ThreeCanvasState {
  renderer: THREE.WebGLRenderer;
  scene: THREE.Scene;
  camera: THREE.OrthographicCamera;
  graph: THREE.Group;
  viewportKey: string | null;
  contentKey: string | null;
}

const threeCanvasState = new WeakMap<HTMLCanvasElement, ThreeCanvasState>();

function resetThreeCanvasState(canvas: HTMLCanvasElement): void {
  const state = threeCanvasState.get(canvas);
  if (!state) return;
  try {
    disposeObject3D(state.graph);
    state.renderer.dispose();
    state.renderer.forceContextLoss();
  } catch {
    // Fall back to 2D drawing; the next WebGL attempt starts from a fresh state.
  }
  threeCanvasState.delete(canvas);
}

/**
 * GPU rasterization of the already-projected scene. The scene is authored in
 * screen space by the shared projector, so the WebGL path draws with a
 * simple orthographic screen-space camera and can never disagree with the
 * hit-testing, labels, or the 2D fallback.
 */
function drawThree(
  canvas: HTMLCanvasElement,
  viewport: { width: number; height: number; ratio: number },
  scene: GraphScene,
): boolean {
  try {
    const three = threeStateFor(canvas);
    const viewportKey = `${viewport.width}:${viewport.height}:${viewport.ratio}`;
    if (three.viewportKey !== viewportKey) {
      three.renderer.setPixelRatio(viewport.ratio);
      three.renderer.setSize(viewport.width, viewport.height, false);
      three.camera.left = 0;
      three.camera.right = viewport.width;
      three.camera.top = 0;
      three.camera.bottom = viewport.height;
      three.camera.updateProjectionMatrix();
      three.viewportKey = viewportKey;
    }
    syncThreeScene(three, scene);
    three.renderer.setClearColor(graphSurfaceColorInt, 1);
    three.renderer.clear(true, true, true);
    three.renderer.render(three.scene, three.camera);
    return true;
  } catch {
    resetThreeCanvasState(canvas);
    return false;
  }
}

function threeStateFor(canvas: HTMLCanvasElement): ThreeCanvasState {
  const existing = threeCanvasState.get(canvas);
  if (existing) return existing;
  const contextAttributes = { alpha: false, antialias: true, preserveDrawingBuffer: true };
  const context = (canvas.getContext("webgl2", contextAttributes) as WebGL2RenderingContext | null)
    ?? (canvas.getContext("webgl", contextAttributes) as WebGLRenderingContext | null);
  if (!context) throw new Error("WebGL is unavailable");
  const renderer = new THREE.WebGLRenderer({
    canvas,
    context,
    alpha: false,
    antialias: true,
    preserveDrawingBuffer: true,
  });
  const scene = new THREE.Scene();
  const graph = new THREE.Group();
  scene.add(graph);
  // Screen-space orthographic camera: with top=0 and bottom=height the
  // default -Z view maps our y-down screen coordinates directly; no up-flip
  // or lookAt (those would mirror x out of the clip volume).
  const camera = new THREE.OrthographicCamera(0, 1, 0, 1, -1000, 1000);
  camera.position.set(0, 0, 500);
  const state: ThreeCanvasState = { renderer, scene, camera, graph, viewportKey: null, contentKey: null };
  threeCanvasState.set(canvas, state);
  return state;
}

function syncThreeScene(three: ThreeCanvasState, scene: GraphScene): void {
  disposeObject3D(three.graph);
  three.graph.clear();

  let order = 0;
  for (const hull of scene.hulls) {
    if (hull.outline.length < 3) continue;
    const shape = new THREE.Shape(hull.outline.map((point) => new THREE.Vector2(point.x, point.y)));
    const mesh = new THREE.Mesh(
      new THREE.ShapeGeometry(shape),
      new THREE.MeshBasicMaterial({
        color: new THREE.Color(hull.color),
        transparent: true,
        opacity: hull.alpha,
        depthWrite: false,
        side: THREE.DoubleSide,
      }),
    );
    mesh.renderOrder = order++;
    three.graph.add(mesh);
  }

  const edgeGroups = new Map<string, {
    positions: number[];
    color: string;
    alpha: number;
    lineStyle: "solid" | "dashed" | "dotted";
  }>();
  for (const edge of scene.edges) {
    const alphaBucket = Math.round(edge.alpha * 20) / 20;
    const color = edge.color;
    const key = `${color}:${alphaBucket}:${edge.lineStyle}`;
    const group = edgeGroups.get(key) ?? {
      positions: [],
      color,
      alpha: alphaBucket,
      lineStyle: edge.lineStyle,
    };
    group.positions.push(edge.x1, edge.y1, 0, edge.x2, edge.y2, 0);
    edgeGroups.set(key, group);
  }
  for (const group of edgeGroups.values()) {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(group.positions, 3));
    const material = group.lineStyle === "solid"
      ? new THREE.LineBasicMaterial({
          color: new THREE.Color(group.color),
          transparent: true,
          opacity: group.alpha,
          depthWrite: false,
        })
      : new THREE.LineDashedMaterial({
          color: new THREE.Color(group.color),
          transparent: true,
          opacity: group.alpha,
          depthWrite: false,
          dashSize: group.lineStyle === "dashed" ? 8 : 3,
          gapSize: group.lineStyle === "dashed" ? 5 : 4,
          scale: 1,
        });
    const lines = new THREE.LineSegments(geometry, material);
    if (group.lineStyle !== "solid") lines.computeLineDistances();
    lines.renderOrder = order++;
    three.graph.add(lines);
  }

  const nodeGroups = new Map<string, { nodes: Array<{ x: number; y: number; r: number }>; color: string; alpha: number }>();
  for (const node of scene.nodes) {
    const color = node.emphasis === "hovered" || node.emphasis === "selected" ? nodeEmphasisColor : node.color;
    const alphaBucket = Math.round(node.alpha * 10) / 10;
    const key = `${color}:${alphaBucket}`;
    const radius = node.emphasis === "hovered" || node.emphasis === "selected"
      ? node.screenRadius * 1.3
      : node.screenRadius;
    const group = nodeGroups.get(key) ?? { nodes: [], color, alpha: alphaBucket };
    group.nodes.push({ x: node.x, y: node.y, r: radius });
    nodeGroups.set(key, group);
  }
  for (const group of nodeGroups.values()) {
    const geometry = new THREE.CircleGeometry(1, 22);
    const material = new THREE.MeshBasicMaterial({
      color: new THREE.Color(group.color),
      transparent: group.alpha < 1,
      opacity: group.alpha,
      depthWrite: false,
    });
    const mesh = new THREE.InstancedMesh(geometry, material, group.nodes.length);
    const matrix = new THREE.Matrix4();
    group.nodes.forEach((node, index) => {
      matrix.compose(
        new THREE.Vector3(node.x, node.y, 0),
        new THREE.Quaternion(),
        new THREE.Vector3(node.r, node.r, 1),
      );
      mesh.setMatrixAt(index, matrix);
    });
    mesh.instanceMatrix.needsUpdate = true;
    mesh.renderOrder = order++;
    three.graph.add(mesh);
  }

  const borderGroups = new Map<string, {
    positions: number[];
    alpha: number;
    borderStyle: "solid" | "dashed" | "dotted";
  }>();
  for (const node of scene.nodes) {
    const emphasized = node.emphasis === "hovered" || node.emphasis === "selected";
    const radius = (emphasized ? node.screenRadius * 1.3 : node.screenRadius) * 1.02;
    const alpha = Math.round(node.alpha * 10) / 10;
    const borderStyle = node.borderStyle;
    const key = `${borderStyle}:${alpha}`;
    const group = borderGroups.get(key) ?? { positions: [], alpha, borderStyle };
    const segments = 22;
    for (let index = 0; index < segments; index += 1) {
      const start = (index / segments) * Math.PI * 2;
      const end = ((index + 1) / segments) * Math.PI * 2;
      group.positions.push(
        node.x + Math.cos(start) * radius,
        node.y + Math.sin(start) * radius,
        1,
        node.x + Math.cos(end) * radius,
        node.y + Math.sin(end) * radius,
        1,
      );
    }
    borderGroups.set(key, group);
  }
  for (const group of borderGroups.values()) {
    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.Float32BufferAttribute(group.positions, 3));
    const material = group.borderStyle === "solid"
      ? new THREE.LineBasicMaterial({
          color: 0xffffff,
          transparent: true,
          opacity: group.alpha,
          depthWrite: false,
        })
      : new THREE.LineDashedMaterial({
          color: 0xffffff,
          transparent: true,
          opacity: group.alpha,
          depthWrite: false,
          dashSize: group.borderStyle === "dashed" ? 5 : 1.5,
          gapSize: group.borderStyle === "dashed" ? 3 : 3,
          scale: 1,
        });
    const lines = new THREE.LineSegments(geometry, material);
    if (group.borderStyle !== "solid") lines.computeLineDistances();
    lines.renderOrder = order++;
    three.graph.add(lines);
  }
}

function disposeObject3D(object: THREE.Object3D): void {
  object.traverse((child) => {
    if (child === object) return;
    disposeRenderable(child);
  });
}

function disposeRenderable(object: THREE.Object3D): void {
  const renderable = object as THREE.Object3D & {
    geometry?: { dispose(): void };
    material?: THREE.Material | THREE.Material[];
  };
  renderable.geometry?.dispose();
  const materials = renderable.material
    ? Array.isArray(renderable.material) ? renderable.material : [renderable.material]
    : [];
  for (const material of materials) material.dispose();
}

// ─── 2D fallback ────────────────────────────────────────────────────────────

function drawCanvas2d(canvas: HTMLCanvasElement, scene: GraphScene): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const ratio = canvas.width / Number.parseFloat(canvas.style.width || String(canvas.width));
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.fillStyle = graphSurfaceColor;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  for (const hull of scene.hulls) {
    if (hull.outline.length < 3) continue;
    ctx.beginPath();
    ctx.moveTo(hull.outline[0].x, hull.outline[0].y);
    for (const point of hull.outline.slice(1)) ctx.lineTo(point.x, point.y);
    ctx.closePath();
    ctx.globalAlpha = hull.alpha;
    ctx.fillStyle = hull.color;
    ctx.fill();
    ctx.globalAlpha = Math.min(1, hull.alpha * 2);
    ctx.lineWidth = 10;
    ctx.strokeStyle = hull.color;
    ctx.stroke();
  }

  ctx.lineWidth = 1;
  for (const edge of scene.edges) {
    ctx.globalAlpha = edge.alpha;
    ctx.strokeStyle = edge.color;
    ctx.lineWidth = edge.emphasized ? 1.6 : 1;
    ctx.setLineDash?.(edge.lineStyle === "dashed" ? [8, 5] : edge.lineStyle === "dotted" ? [2, 4] : []);
    ctx.beginPath();
    ctx.moveTo(edge.x1, edge.y1);
    ctx.lineTo(edge.x2, edge.y2);
    ctx.stroke();
  }

  for (const node of scene.nodes) {
    const emphasized = node.emphasis === "hovered" || node.emphasis === "selected";
    ctx.globalAlpha = node.alpha;
    ctx.beginPath();
    ctx.fillStyle = emphasized ? nodeEmphasisColor : node.color;
    ctx.arc(node.x, node.y, emphasized ? node.screenRadius * 1.3 : node.screenRadius, 0, Math.PI * 2);
    ctx.fill();
    ctx.lineWidth = emphasized ? 2.4 : 1.4;
    ctx.strokeStyle = "#ffffff";
    ctx.setLineDash?.(node.borderStyle === "dashed" ? [5, 3] : node.borderStyle === "dotted" ? [1, 3] : []);
    ctx.stroke();
  }
  ctx.setLineDash?.([]);
  ctx.globalAlpha = 1;
}

// ─── Shared helpers ─────────────────────────────────────────────────────────

function resizeCanvasIfNeeded(
  canvas: HTMLCanvasElement,
  viewport: { width: number; height: number; ratio: number },
  previous: { width: number; height: number; cssWidth: number; cssHeight: number },
): { width: number; height: number; cssWidth: number; cssHeight: number } {
  const width = Math.floor(viewport.width * viewport.ratio);
  const height = Math.floor(viewport.height * viewport.ratio);
  if (
    previous.width === width
    && previous.height === height
    && previous.cssWidth === viewport.width
    && previous.cssHeight === viewport.height
  ) {
    return previous;
  }
  canvas.width = width;
  canvas.height = height;
  canvas.style.width = `${viewport.width}px`;
  canvas.style.height = `${viewport.height}px`;
  return { width, height, cssWidth: viewport.width, cssHeight: viewport.height };
}

function renderStatus(state: GraphState): string {
  const { layoutError: error, layoutStatus: status } = state;
  if (status === "failed") {
    return `<span class="os-kg-status os-kg-status-failed" data-testid="knowledge-graph-status">Graph unavailable${error ? `: ${escapeHtml(error)}` : ""}</span>`;
  }
  if (state.freshnessStatus === "stale") {
    return `<span class="os-kg-status os-kg-status-stale" data-testid="knowledge-graph-status">Graph stale: ${escapeHtml(state.staleBundleIds.join(", ") || "pending refresh")}</span>`;
  }
  if (state.freshnessStatus === "warning") {
    return `<span class="os-kg-status os-kg-status-warning" data-testid="knowledge-graph-status">Graph warnings: ${escapeHtml(state.warningBundleIds.join(", ") || "review graph metrics")}</span>`;
  }
  if (status === "loading" || status === "stabilizing") {
    return `<span class="os-kg-status" data-testid="knowledge-graph-status">Stabilizing</span>`;
  }
  if (status === "ready") return `<span class="os-kg-status" data-testid="knowledge-graph-status">Ready</span>`;
  return `<span class="os-kg-status" data-testid="knowledge-graph-status">Idle</span>`;
}

/**
 * Inspector card for the selected node: metadata, frontmatter chips, and the
 * concept's memory capsule. Rendered in the lower-right workspace column so
 * the graph stage and capsule content share the fold.
 */
export function renderKnowledgeGraphInspector(surface: KnowledgeGraphSurface): string {
  const { snapshot, state } = surface;
  const selected = new Set(state.selectedNodeIds);
  const node = snapshot?.nodes.find((candidate) => selected.has(candidate.id)) ?? null;
  if (!node) {
    return `<section class="os-kg-inspector" data-testid="knowledge-graph-inspector"><h3>Inspector</h3><p>No node selected</p></section>`;
  }
  const community = node.metrics?.community_id
    ? snapshot?.communities.find((candidate) => candidate.id === node.metrics.community_id)?.label ?? node.metrics.community_id
    : "None";
  const deepLink = snapshot ? memoryDeepLinkForGraphNode(snapshot.bundle_id, node) : null;
  const deepLinkButton = deepLink
    ? `<button type="button" class="os-kg-copy-deeplink" data-kg-copy-deeplink="${escapeAttr(deepLink)}" data-testid="knowledge-graph-copy-deeplink" title="Copy a deep link to this capsule">Copy deep link</button>`
    : "";
  return `
    <section class="os-kg-inspector" data-testid="knowledge-graph-inspector">
      <div class="os-kg-inspector-header">
        <h3>${escapeHtml(node.label)}</h3>
        ${deepLinkButton}
      </div>
      <dl>
        <dt>Kind</dt><dd>${escapeHtml(node.kind)}</dd>
        <dt>Visibility</dt><dd>${escapeHtml(node.visibility ?? "unknown")}</dd>
        <dt>Community</dt><dd>${escapeHtml(community)}</dd>
        <dt>Tags</dt><dd>${escapeHtml(node.tags.join(", ") || "None")}</dd>
      </dl>
      ${renderCapsule(node, surface)}
    </section>
  `;
}

/**
 * Capsule pane inside the inspector: the concept's memory capsule fetched
 * from the concept-detail endpoint. Non-concept nodes have no capsule; for
 * concepts the pane moves through loading → detail (or error with retry).
 */
function renderCapsule(node: MemoryGraphNode, surface: KnowledgeGraphSurface): string {
  if (node.kind !== "concept" || !node.concept_id) return "";
  if (surface.conceptDetailError) {
    return `
      <div class="os-kg-capsule" data-testid="knowledge-graph-capsule">
        <p class="os-kg-capsule-error" data-testid="knowledge-graph-capsule-error">Capsule unavailable: ${escapeHtml(surface.conceptDetailError)}</p>
        <button type="button" data-kg-capsule-retry data-testid="knowledge-graph-capsule-retry">Retry</button>
      </div>
    `;
  }
  const detail = surface.conceptDetail ?? null;
  if (!detail || detail.concept_id !== node.concept_id) {
    return `<div class="os-kg-capsule" data-testid="knowledge-graph-capsule"><p data-testid="knowledge-graph-capsule-loading">Loading capsule&hellip;</p></div>`;
  }
  const frontmatter = { ...detail.frontmatter_view.primary, ...detail.frontmatter_view.opensymphony };
  const chips = Object.entries(frontmatter)
    .filter(([key, value]) => key !== "title" && isChipValue(value))
    .slice(0, 8)
    .map(([key, value]) => `<span class="os-kg-chip">${escapeHtml(key)}: ${escapeHtml(chipText(value))}</span>`)
    .join("");
  const links = detail.links.length > 0
    ? `
      <h4>Linked concepts</h4>
      <ul class="os-kg-capsule-links">
        ${detail.links.map((link) => `
          <li><button type="button" class="os-kg-capsule-link" data-kg-link-target="${escapeAttr(link.target)}">${escapeHtml(link.label ?? link.target)}</button></li>
        `).join("")}
      </ul>
    `
    : "";
  const citations = detail.citations.length > 0
    ? `
      <h4>Citations</h4>
      <ul class="os-kg-capsule-links" data-testid="knowledge-graph-capsule-citations">
        ${detail.citations.map((citation) => `
          <li>${citation.target.startsWith(codeDeepLinkPrefix)
            ? `<button type="button" class="os-kg-capsule-link" data-code-deeplink="${escapeAttr(citation.target)}">${escapeHtml(citation.label ?? citation.target)}</button>`
            : /^https?:\/\//.test(citation.target)
            // Real OKF citations often carry a URL target with a short
            // label; those are external evidence, not graph nodes.
            ? `<a href="${escapeAttr(citation.target)}" target="_blank" rel="noreferrer">${escapeHtml(citation.label ?? citation.id)}</a>`
            : `<button type="button" class="os-kg-capsule-link" data-kg-link-target="${escapeAttr(citation.target)}">${escapeHtml(citation.label ?? citation.target)}</button>`}</li>
        `).join("")}
      </ul>
    `
    : "";
  const sources = detail.source_refs.length > 0
    ? `
      <h4>Sources</h4>
      <ul class="os-kg-capsule-sources">
        ${detail.source_refs.map((ref) => {
          const codeLink = codeDeepLinkForSourceRef(ref, detail);
          return `
          <li>${codeLink
            ? `<button type="button" class="os-kg-capsule-link" data-code-deeplink="${escapeAttr(codeLink)}">${escapeHtml(ref.kind)}: ${escapeHtml(ref.id)}</button>`
            : ref.url && /^https?:\/\//.test(ref.url)
            ? `<a href="${escapeAttr(ref.url)}" target="_blank" rel="noreferrer">${escapeHtml(ref.kind)}: ${escapeHtml(ref.id)}</a>`
            : `${escapeHtml(ref.kind)}: ${escapeHtml(ref.id)}`}</li>
        `;
        }).join("")}
      </ul>
    `
    : "";
  return `
    <div class="os-kg-capsule" data-testid="knowledge-graph-capsule">
      ${chips ? `<div class="os-kg-capsule-chips">${chips}</div>` : ""}
      <div class="os-kg-capsule-body" data-testid="knowledge-graph-capsule-body">${renderMemoryMarkdown(detail.body_markdown)}</div>
      ${links}
      ${citations}
      ${sources}
    </div>
  `;
}

function codeDeepLinkForSourceRef(
  ref: NonNullable<MemoryConceptDetail["source_refs"]>[number],
  detail: MemoryConceptDetail,
): string | null {
  const repoId = ref.repo_id ?? codeRepoIdFromConceptScopes(detail);
  if (!repoId) return null;
  try {
    return ref.symbol_key
      ? codeDeepLinkForSymbol(repoId, ref.symbol_key)
      : ref.kind === "path"
        ? codeDeepLinkForFile(repoId, ref.id)
        : ref.kind === "code-symbol"
          ? codeDeepLinkForFile(repoId, legacyCodePathFromSourceRef(ref))
        : null;
  } catch {
    return null;
  }
}

function codeRepoIdFromConceptScopes(detail: MemoryConceptDetail): string | null {
  const scopeRefs = detail.frontmatter_view.opensymphony.scope_refs;
  if (!Array.isArray(scopeRefs)) return null;
  const repositoryIds = scopeRefs
    .filter((scope): scope is { kind: unknown; id: string } => (
      typeof scope === "object"
      && scope !== null
      && (scope as { kind?: unknown }).kind === "repository"
      && typeof (scope as { id?: unknown }).id === "string"
    ))
    .map((scope) => scope.id);
  return repositoryIds.length === 1 ? repositoryIds[0] : null;
}

function legacyCodePathFromSourceRef(
  ref: NonNullable<MemoryConceptDetail["source_refs"]>[number],
): string {
  return /^(.*):\d+:\d+-\d+:\d+$/.exec(ref.id)?.[1] ?? ref.id;
}

function isChipValue(value: unknown): boolean {
  return typeof value === "string" || typeof value === "number" || typeof value === "boolean"
    || (Array.isArray(value) && value.every((entry) => typeof entry === "string"));
}

function chipText(value: unknown): string {
  return Array.isArray(value) ? value.join(", ") : String(value);
}

/**
 * Clickable entity list for the visible snapshot (also the keyboard/screen-
 * reader path into the graph). Rendered in the narrow lower-left workspace
 * column; bind with bindKnowledgeGraphListNavigation.
 */
export function renderKnowledgeGraphNodeList(snapshot: MemoryGraphSnapshot | null, selectedNodeIds: readonly string[]): string {
  if (!snapshot || snapshot.nodes.length === 0) {
    return `<div class="os-empty">No graph data available.</div>`;
  }
  const selected = new Set(selectedNodeIds);
  return `
    <ul class="os-kg-list" data-testid="knowledge-graph-node-list" aria-label="Visible graph nodes">
      ${snapshot.nodes.map((node) => `
        <li class="${selected.has(node.id) ? "is-selected" : ""}">
          <button type="button" data-kg-node-id="${escapeAttr(node.id)}" ${selected.has(node.id) ? `aria-current="true"` : ""}>${escapeHtml(node.label)}</button>
          <span>${escapeHtml(node.kind)}</span>
        </li>
      `).join("")}
    </ul>
  `;
}

function prefersReducedMotion(): boolean {
  if (typeof globalThis.matchMedia !== "function") return false;
  try {
    return globalThis.matchMedia("(prefers-reduced-motion: reduce)").matches;
  } catch {
    return false;
  }
}

function graphListNavigationDirection(key: string): -1 | 1 | "first" | "last" | null {
  switch (key) {
    case "ArrowUp":
    case "ArrowLeft":
      return -1;
    case "ArrowDown":
    case "ArrowRight":
      return 1;
    case "Home":
      return "first";
    case "End":
      return "last";
    default:
      return null;
  }
}

function canvasViewport(stage: HTMLElement | null): { width: number; height: number; ratio: number } {
  const rect = stage?.getBoundingClientRect();
  const width = Math.max(320, Math.floor(rect?.width || 640));
  const height = Math.max(260, Math.floor(rect?.height || 360));
  return { width, height, ratio: globalThis.devicePixelRatio || 1 };
}

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

function shortLabel(label: string): string {
  return label.length > 34 ? `${label.slice(0, 31)}...` : label;
}

function matchesFocusVisible(element: Element): boolean {
  try {
    return element.matches(":focus-visible");
  } catch {
    // Environments without :focus-visible (older jsdom) keep the previous
    // behavior of treating any focus as intentional.
    return true;
  }
}

function cssEscape(value: string): string {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(value);
  }
  return value.replace(/["\\\]]/g, "\\$&");
}
