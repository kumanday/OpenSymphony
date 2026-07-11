/** @jest-environment jsdom */

import {
  codeEdgeVisualStyle,
  codeGraphFixtureSnapshots,
  codeGraphFixtureSymbolDetails,
  codeGraphFixtureDiffOverlays,
  codeGraphReducer,
  codeGraphSnapshotForRendering,
  codeNodeVisualStyle,
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
});
