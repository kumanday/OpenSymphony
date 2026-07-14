/** @jest-environment jsdom */

import {
  codeEdgeVisualStyle,
  codeGraphFixtureSnapshots,
  codeGraphFixtureRepos,
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

  it("renders an accessible empty-state index action with progress and retry diagnostics", () => {
    const repo = {
      ...codeGraphFixtureRepos.repos[0],
      document_count: 0,
      symbol_count: 0,
      edge_count: 0,
      freshness: "unknown" as const,
      indexed_at: null,
      head_revision: null,
    };
    let state = codeGraphReducer(createInitialCodeGraphState(), {
      type: "REPOS_LOADED",
      repos: { ...codeGraphFixtureRepos, repos: [repo] },
    });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphSurface({ snapshot: null, layout: null, state });
    expect(root.querySelector("[data-testid='code-graph-index']")?.textContent).toContain("Index repository");
    expect(root.querySelector("[data-testid='code-graph-index']")?.getAttribute("type")).toBe("button");
    expect(root.querySelector("[data-testid='code-graph-index-empty']")?.textContent).toContain("Repository is not indexed");

    state = codeGraphReducer(state, { type: "INDEX_STARTED", repoId: "opensymphony" });
    root.innerHTML = renderCodeGraphSurface({ snapshot: null, layout: null, state });
    expect(root.querySelector("[data-testid='code-graph-index']")?.hasAttribute("disabled")).toBe(true);
    expect(root.querySelector("[data-testid='code-graph-index-empty']")?.textContent).toContain("Indexing repository");

    state = codeGraphReducer(state, {
      type: "INDEX_REPORT",
      report: {
        schema_version: { major: 1, minor: 0, patch: 0 },
        repo_id: "opensymphony",
        status: "failed",
        head_revision: null,
        parsed_files: 2,
        persisted_documents: 1,
        persisted_symbols: 3,
        persisted_edges: 1,
        persisted_diagnostics: 1,
        stale_rows: 0,
        skipped_files: ["vendor/generated.rs"],
        diagnostics: ["parser limit reached"],
        cursor: { sequence: 2, partition: "code-graph:opensymphony" },
        indexed_at: "2026-07-13T00:00:00Z",
      },
    });
    root.innerHTML = renderCodeGraphSurface({ snapshot: null, layout: null, state });
    expect(root.querySelector("[data-testid='code-graph-index']")?.textContent).toContain("Retry indexing");
    expect(root.querySelector("[data-testid='code-graph-index-diagnostics']")?.textContent).toContain("parser limit reached");
    expect(root.querySelector("[data-testid='code-graph-index-coverage']")?.textContent).toContain("1 skipped");
  });

  it("shows revision, workspace provenance, stale, truncated, and partial coverage status", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), {
      type: "REPOS_LOADED",
      repos: codeGraphFixtureRepos,
    });
    state = codeGraphReducer(state, { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "TARGET_SET", runId: "COE-546" });
    state = codeGraphReducer(state, { type: "GRAPH_UPDATED", repoId: "opensymphony", updatedAt: "2026-07-13T00:00:00Z" });
    state = codeGraphReducer(state, {
      type: "INDEX_REPORT",
      report: {
        schema_version: { major: 1, minor: 0, patch: 0 },
        repo_id: "opensymphony",
        status: "completed",
        head_revision: "target-revision",
        parsed_files: 4,
        persisted_documents: 4,
        persisted_symbols: 8,
        persisted_edges: 6,
        persisted_diagnostics: 0,
        stale_rows: 0,
        skipped_files: ["src/generated.rs"],
        diagnostics: [],
        cursor: { sequence: 3, partition: "code-graph:opensymphony" },
        indexed_at: "2026-07-13T00:00:00Z",
      },
    });
    const truncated = { ...snapshot, truncation: { nodes_dropped: 2, edges_dropped: 1, reason: "bounded" } };
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphSurface({ snapshot: truncated, layout: null, state });
    expect(root.querySelector("[data-testid='code-graph-target-revision']")?.textContent).toBe("target-revision");
    expect(root.querySelector("[data-testid='code-graph-view-provenance']")?.textContent)
      .toContain("Workspace-composed");
    expect(root.querySelector("[data-testid='code-graph-view-provenance']")?.textContent)
      .toContain("Stale");
    expect(root.querySelector("[data-testid='code-graph-view-provenance']")?.textContent)
      .toContain("Truncated");
    expect(root.querySelector("[data-testid='code-graph-view-provenance']")?.textContent)
      .toContain("Partial coverage");
  });

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
    expect(summary?.textContent).toContain("49,000 nodes");
    expect(summary?.textContent).toContain("199,001 edges");
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

  it("renders topology deltas and selected relationship details accessibly", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "DIFF_LOADED", overlay: codeGraphFixtureDiffOverlays[0] });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphSurface({ snapshot, layout: null, state });
    root.insertAdjacentHTML("beforeend", renderCodeGraphInspector({ snapshot, layout: null, state, rawRecord: false }));

    expect(root.querySelector("[data-testid='code-graph-topology-summary']")?.textContent).toContain("confidence exact");
    expect(root.querySelector("[data-testid='code-graph-topology-edge-list']")).not.toBeNull();
    expect(root.querySelector("[data-testid='code-graph-topology-connection-list']")).not.toBeNull();
    expect(root.querySelector("[data-testid='code-graph-selected-topology']")?.textContent).toContain("added");
    expect(root.querySelector("[data-testid='code-graph-screen-reader-summary']")?.textContent).toContain("topology edge changes");
  });

  it("shows both sides of a selected retargeted topology edge", () => {
    const overlay = {
      ...codeGraphFixtureDiffOverlays[0],
      edge_deltas: [{
        edge_key: "selected-retarget",
        status: "retargeted" as const,
        before: {
          edge_id: "selected-retarget-before",
          kind: "call",
          source_symbol_key: "codeGraphReducer",
          target_symbol_key: "oldTarget",
          target_hint: null,
          confidence: "exact" as const,
          unresolved: false,
          path: "src/old.ts",
          span: { start_line: 1, start_col: 1, end_line: 1, end_col: 8 },
        },
        after: {
          edge_id: "selected-retarget-after",
          kind: "call",
          source_symbol_key: "codeGraphReducer",
          target_symbol_key: "newTarget",
          target_hint: null,
          confidence: "exact" as const,
          unresolved: false,
          path: "src/new.ts",
          span: { start_line: 2, start_col: 1, end_line: 2, end_col: 8 },
        },
      }],
    };
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "DIFF_LOADED", overlay });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:codeGraphReducer" });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphInspector({ snapshot, layout: null, state, rawRecord: false });

    const details = root.querySelector("[data-testid='code-graph-selected-topology']")?.textContent;
    expect(details).toContain("retargeted before");
    expect(details).toContain("oldTarget");
    expect(details).toContain("retargeted after");
    expect(details).toContain("newTarget");
  });

  it("announces diff truncation and its reason in the toolbar", () => {
    const diffOverlay = {
      ...codeGraphFixtureDiffOverlays[0],
      truncation: { nodes_dropped: 3, edges_dropped: 5, reason: "diff symbols capped" },
    };
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "DIFF_LOADED", overlay: diffOverlay });
    const root = document.createElement("div");
    root.innerHTML = renderCodeGraphSurface({ snapshot, layout: null, state });

    expect(root.querySelector("[data-testid='code-graph-metrics']")?.textContent).toContain("3 nodes + 5 edges truncated: diff symbols capped");
    expect(root.querySelector("[data-testid='code-graph-screen-reader-summary']")?.textContent).toContain("3 nodes and 5 edges");
    expect(root.querySelector("[data-testid='code-graph-screen-reader-summary']")?.textContent).toContain("diff symbols capped");
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
