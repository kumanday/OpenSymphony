/** @jest-environment jsdom */

import {
  codeEdgeVisualStyle,
  codeGraphFixtureSnapshots,
  codeGraphFixtureSymbolDetails,
  codeGraphFixtureDiffOverlays,
  codeGraphReducer,
  codeGraphSnapshotForRendering,
  codeNodeVisualStyle,
  createCodeGraphReferenceAtlasFixture,
  computeGraphLayout,
  createInitialCodeGraphState,
  parseCodeDeepLink,
} from "@opensymphony/graph";
import {
  buildGraphScene,
  defaultCameraForLayout,
} from "../src/knowledge-graph-scene.js";
import {
  renderCodeGraphInspector,
  renderCodeGraphNodeList,
  renderCodeGraphSurface,
} from "../src/knowledge-graph-renderer.js";
import { createKnowledgeGraphViewState } from "../src/knowledge-graph-scene.js";

describe("Code Graph renderer surface", () => {
  const snapshot = codeGraphFixtureSnapshots.find((candidate) => candidate.mode === "file")!;

  it("renders modes, structure fallback, freshness, diagnostics, and raw detail", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const detail = codeGraphFixtureSymbolDetails.find((candidate) => candidate.symbol_key === "codeGraphReducer")!;
    const surface = renderCodeGraphSurface({ snapshot, layout: null, state, symbolDetail: detail, rawRecord: true });
    const root = document.createElement("div");
    root.innerHTML = surface;
    root.insertAdjacentHTML("beforeend", renderCodeGraphNodeList(snapshot, state.selectedNodeIds, codeGraphFixtureDiffOverlays[0]));
    root.insertAdjacentHTML("beforeend", renderCodeGraphInspector({ snapshot, layout: null, state, symbolDetail: detail, rawRecord: true }));

    expect(root.querySelectorAll("[data-code-mode]")).toHaveLength(4);
    expect(root.querySelector("[data-testid='code-graph-filters']")).not.toBeNull();
    expect(root.querySelector("[data-code-filter='confidences']")).not.toBeNull();
    expect(root.querySelector("[data-code-filter='pathPrefixes']")).not.toBeNull();
    expect(root.querySelector("[data-testid='code-graph-canvas']")).not.toBeNull();
    expect(root.querySelector("[data-code-node-kind='symbol']")).not.toBeNull();
    expect(root.querySelector("[data-code-freshness-badge='stale']")).not.toBeNull();
    expect(root.querySelector("[data-testid='code-graph-raw-record']")?.textContent).toContain("codeGraphReducer");
    expect(root.querySelector("[data-code-confidence='syntactic']")).not.toBeNull();
    expect(root.querySelector("[data-code-delta-status='modified']")).not.toBeNull();
  });

  it("announces truncation, mode, and repo without relying on canvas or color", () => {
    const reference = createCodeGraphReferenceAtlasFixture();
    const state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot: reference });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphSurface({ snapshot: reference, layout: null, state });
    const summary = root.querySelector("[data-testid='code-graph-screen-reader-summary']");
    expect(summary?.textContent).toContain("reference-scale");
    expect(summary?.textContent).toContain("48,000 nodes");
    expect(summary?.textContent).toContain("198,001 edges");
    expect(summary?.textContent).toContain("directory aggregation");
    expect(summary?.id).toBe("code-graph-screen-reader-summary");
    expect(root.querySelector("canvas")?.getAttribute("aria-describedby")).toBe("code-graph-screen-reader-summary");
  });

  it("encodes confidence as line style and freshness as opacity/border", () => {
    const renderSnapshot = codeGraphSnapshotForRendering(snapshot);
    const layout = computeGraphLayout(renderSnapshot, { kind: "hierarchical", width: 1280, height: 900 });
    const viewport = { width: 1200, height: 700 };
    const scene = buildGraphScene({
      layout,
      communities: renderSnapshot.communities,
      camera: defaultCameraForLayout(layout, viewport),
      viewport,
      overrides: createKnowledgeGraphViewState().overrides,
      selectedNodeIds: [],
      hoveredNodeId: null,
      nodeStyle: (node) => {
        const source = snapshot.nodes.find((candidate) => candidate.id === node.nodeId)!;
        return codeNodeVisualStyle(source);
      },
      edgeStyle: (edge) => edge.confidence
        ? codeEdgeVisualStyle({ confidence: edge.confidence as "exact" | "syntactic" | "heuristic" })
        : undefined,
    });
    expect(scene.edges.map((edge) => edge.lineStyle)).toEqual(expect.arrayContaining(["solid", "dashed"]));
    const current = scene.nodes.find((node) => node.nodeId === "symbol:graphReducer")!;
    const stale = scene.nodes.find((node) => node.nodeId === "symbol:codeGraphReducer")!;
    expect(stale.alpha).toBeLessThan(current.alpha);
    expect(stale.borderStyle).toBe("dashed");
  });

  it("keeps symbol deep links valid from File mode", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({
      snapshot,
      layout: null,
      state,
      symbolDetail: codeGraphFixtureSymbolDetails.find((candidate) => candidate.symbol_key === "codeGraphReducer"),
      rawRecord: false,
    });
    const deepLink = root.querySelector<HTMLButtonElement>("[data-code-copy-deeplink]")?.dataset.codeCopyDeeplink;
    expect(deepLink).toBeDefined();
    expect(parseCodeDeepLink(deepLink!)).toMatchObject({ mode: "neighborhood", symbolKey: "codeGraphReducer" });
  });

  it("renders cross-graph chips as links to current targets", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const detail = codeGraphFixtureSymbolDetails.find((candidate) => candidate.symbol_key === "codeGraphReducer")!;
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({
      snapshot,
      layout: null,
      state,
      symbolDetail: {
        ...detail,
        related_issues: [{ issue_key: "COE-536", title: "Cross graph", freshness: "current" }],
        related_memory_concepts: [{ bundle_id: "local-default", concept_id: "issues/COE-536", title: "Cross graph", visibility: "private", freshness: "current" }],
      },
      rawRecord: false,
    });
    expect(root.querySelector("[data-task-issue-key='COE-536']")).not.toBeNull();
    expect(root.querySelector("[data-memory-deeplink='opensymphony://memory/local-default/concepts/issues/COE-536']")).not.toBeNull();
  });

  it("renders a file fallback and omits delta-only filters from copied links", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "FILTERS_SET", filters: { deltaStatuses: ["modified"] } });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "file:packages/graph/src/index.ts" });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({ snapshot, layout: null, state, rawRecord: false });
    expect(root.querySelector("[data-testid='code-graph-file-fallback']")).not.toBeNull();
    const deepLink = root.querySelector<HTMLButtonElement>("[data-code-copy-deeplink]")?.dataset.codeCopyDeeplink;
    expect(parseCodeDeepLink(deepLink!)?.filters.deltaStatuses).toEqual([]);
  });

  it("omits copied links for diff-only synthetic symbols", () => {
    const overlay = {
      ...codeGraphFixtureDiffOverlays[0],
      added_symbols: [{
        symbol_key: "addedSymbol",
        status: "added" as const,
        before: null,
        after: {
          symbol_id: "addedSymbol:id",
          kind: "function",
          name: "addedSymbol",
          path_display: "packages/graph/src/added.ts",
          container_chain: ["code"],
          span: { start_line: 1, start_col: 1, end_line: 4, end_col: 2 },
          freshness: "current" as const,
        },
      }],
    };
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "DIFF_LOADED", overlay });
    const synthetic = {
      ...snapshot.nodes[1],
      id: "symbol:addedSymbol",
      label: "addedSymbol",
      symbol_key: "addedSymbol",
    };
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: synthetic.id });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({
      snapshot: { ...snapshot, nodes: [...snapshot.nodes, synthetic] },
      layout: null,
      state,
      rawRecord: false,
    });
    expect(root.querySelector("[data-code-copy-deeplink]")).toBeNull();
  });

  it("keeps copied links in Diff mode tied to the revision pair", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "DIFF_LOADED", overlay: codeGraphFixtureDiffOverlays[0] });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({ snapshot, layout: null, state, rawRecord: false });
    const deepLink = root.querySelector<HTMLButtonElement>("[data-code-copy-deeplink]")?.dataset.codeCopyDeeplink;
    expect(parseCodeDeepLink(deepLink!)).toMatchObject({
      mode: "diff",
      symbolKey: "codeGraphReducer",
      baseRevision: codeGraphFixtureDiffOverlays[0].base_revision,
      headRevision: codeGraphFixtureDiffOverlays[0].head_revision,
    });
  });

  it("shows delta filters only when a Diff overlay is active", () => {
    let atlasState = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    atlasState = codeGraphReducer(atlasState, { type: "FILTERS_SET", filters: { deltaStatuses: ["modified"] } });
    const atlasRoot = document.createElement("div");
    atlasRoot.innerHTML = renderCodeGraphSurface({ snapshot, layout: null, state: atlasState });
    expect(atlasRoot.querySelector("[data-code-filter='deltaStatuses']")).toBeNull();

    const diffState = codeGraphReducer(atlasState, { type: "DIFF_LOADED", overlay: codeGraphFixtureDiffOverlays[0] });
    const diffRoot = document.createElement("div");
    diffRoot.innerHTML = renderCodeGraphSurface({ snapshot, layout: null, state: diffState });
    expect(diffRoot.querySelector("[data-code-filter='deltaStatuses']")).not.toBeNull();
  });

  it("shows the graph record when symbol detail loading fails", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({
      snapshot,
      layout: null,
      state,
      detailError: "Symbol detail unavailable",
      rawRecord: false,
    });
    expect(root.querySelector("[data-testid='code-graph-detail-loading']")).toBeNull();
    expect(root.querySelector("[data-testid='code-graph-file-fallback']")?.textContent)
      .toContain("Symbol detail unavailable");
  });
});
