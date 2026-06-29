import {
  applyGraphFilters,
  createFixtureGraphAdapter,
  computeGraphLayout,
  createGraphLayoutAdapter,
  createInitialGraphState,
  createGatewayGraphAdapter,
  createTauriNativeGraphAdapter,
  fixtureGraphSnapshot,
  graphLayoutKindForMode,
  graphReducer,
  graphStateToHistory,
  initialGraphFilters,
  initialGraphState,
  searchGraphSnapshot,
} from "@opensymphony/graph";

describe("@opensymphony/graph", () => {
  it("loads DTOs and switches through all graph modes", () => {
    let state = graphReducer(initialGraphState, {
      type: "SNAPSHOT_LOADED",
      snapshot: fixtureGraphSnapshot,
    });
    for (const mode of ["atlas", "bundle", "community", "neighborhood", "timeline", "evidence"] as const) {
      state = graphReducer(state, { type: "MODE_SET", mode });
      expect(state.mode).toBe(mode);
    }
  });

  it("applies deterministic filters and search results", () => {
    const filtered = applyGraphFilters(fixtureGraphSnapshot, {
      ...initialGraphFilters,
      tags: ["graph-view"],
      areas: ["graph-view"],
    });
    expect(filtered.nodes.map((node) => node.id)).toEqual(["concept:coe-465"]);
    expect(filtered.filters_applied).toEqual(["area:graph-view", "tag:graph-view"]);

    const results = searchGraphSnapshot(fixtureGraphSnapshot, "graph", initialGraphFilters);
    expect(results.map((result) => result.concept_id)).toEqual(["issues/COE-465", "tag:graph-view"]);
  });

  it("normalizes selection and history state for URLs or app history", () => {
    const state = graphReducer(initialGraphState, {
      type: "HISTORY_RESTORED",
      state: {
        mode: "neighborhood",
        bundleId: "local-default",
        focusedNodeId: "concept:coe-465",
        selectedNodeIds: ["tag:graph-view", "concept:coe-465", "tag:graph-view"],
        searchQuery: "  graph   view ",
        filters: { ...initialGraphFilters, tags: ["frontend", "graph-view", "frontend"] },
        neighborhoodDepth: 2,
      },
    });
    expect(graphStateToHistory(state)).toMatchObject({
      mode: "neighborhood",
      selectedNodeIds: ["concept:coe-465", "tag:graph-view"],
      searchQuery: "graph view",
      neighborhoodDepth: 2,
    });
    expect(graphStateToHistory(state).filters.tags).toEqual(["frontend", "graph-view"]);
  });

  it("restores history clearing values without carrying stale state", () => {
    const populated = graphReducer(initialGraphState, {
      type: "HISTORY_RESTORED",
      state: {
        mode: "neighborhood",
        bundleId: "local-default",
        focusedNodeId: "concept:coe-465",
        selectedNodeIds: ["concept:coe-465"],
        searchQuery: "graph",
        filters: { ...initialGraphFilters, tags: ["graph-view"] },
        neighborhoodDepth: 2,
      },
    });

    const restored = graphReducer(populated, {
      type: "HISTORY_RESTORED",
      state: {
        bundleId: null,
        focusedNodeId: null,
        selectedNodeIds: [],
        neighborhoodDepth: 0,
      },
    });

    expect(restored.selectedBundleId).toBeNull();
    expect(restored.focusedNodeId).toBeNull();
    expect(restored.selectedNodeIds).toEqual([]);
    expect(restored.neighborhoodDepth).toBe(0);
    expect(restored.searchQuery).toBe("graph");
    expect(restored.filters.tags).toEqual(["graph-view"]);
  });

  it("provides fixture and HTTP adapters without Tauri imports", async () => {
    const fixture = createFixtureGraphAdapter();
    await expect(fixture.listBundles()).resolves.toMatchObject({
      bundles: [{ id: "local-default" }],
    });

    const fetchMock = jest.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({ bundles: [] }),
    })) as unknown as typeof fetch;
    const gateway = createGatewayGraphAdapter("http://localhost:2468", fetchMock);
    await gateway.listBundles();
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:2468/api/v1/memory/bundles");
    await gateway.search("graph", {
      visibility: "public",
      limit: 5,
      bundleId: "local-default",
    });
    expect(fetchMock).toHaveBeenCalledWith(
      "http://localhost:2468/api/v1/memory/search?visibility=public&query=graph&limit=5&bundle_id=local-default",
    );
  });

  it("rejects scoped HTTP graph adapter requests that exceed public visibility", async () => {
    const fetchMock = jest.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({ bundles: [] }),
    })) as unknown as typeof fetch;
    const gateway = createGatewayGraphAdapter("http://localhost:2468", fetchMock, {
      defaultVisibility: "public",
      maxVisibility: "public",
    });

    await gateway.listBundles();
    await gateway.getGraphSnapshot("local-default", { visibility: "public" });
    expect(() => gateway.getGraphSnapshot("local-default", { visibility: "all_accessible" }))
      .toThrow('Graph visibility "all_accessible" exceeds adapter policy "public"');
    expect(() => gateway.search("graph", { visibility: "private", limit: 5 }))
      .toThrow('Graph visibility "private" exceeds adapter policy "public"');

    expect(fetchMock).toHaveBeenNthCalledWith(
      1,
      "http://localhost:2468/api/v1/memory/bundles?visibility=public",
    );
    expect(fetchMock).toHaveBeenNthCalledWith(
      2,
      "http://localhost:2468/api/v1/memory/bundles/local-default/graph?visibility=public",
    );
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("rejects visibility requests above non-public adapter policies", async () => {
    const fetchMock = jest.fn(async () => ({
      ok: true,
      status: 200,
      json: async () => ({ bundles: [] }),
    })) as unknown as typeof fetch;
    const gateway = createGatewayGraphAdapter("http://localhost:2468", fetchMock, {
      maxVisibility: "private",
    });

    await gateway.listBundles();
    expect(() => gateway.getGraphSnapshot("local-default", { visibility: "all_accessible" }))
      .toThrow('Graph visibility "all_accessible" exceeds adapter policy "private"');
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:2468/api/v1/memory/bundles?visibility=private");
  });

  it("marks graph updates stale until the updated snapshot loads", () => {
    const current = graphReducer(initialGraphState, {
      type: "SNAPSHOT_LOADED",
      snapshot: fixtureGraphSnapshot,
    });

    const stale = graphReducer(current, {
      type: "GRAPH_UPDATED",
      event: {
        schema_version: fixtureGraphSnapshot.schema_version,
        bundle_id: fixtureGraphSnapshot.bundle_id,
        cursor: { sequence: fixtureGraphSnapshot.cursor.sequence + 1, partition: fixtureGraphSnapshot.cursor.partition },
        updated_at: "2026-06-28T00:01:00Z",
      },
    });

    expect(stale.freshnessStatus).toBe("stale");
    expect(stale.staleBundleIds).toEqual(["local-default"]);

    const refreshed = graphReducer(stale, {
      type: "SNAPSHOT_LOADED",
      snapshot: {
        ...fixtureGraphSnapshot,
        cursor: { ...fixtureGraphSnapshot.cursor, sequence: fixtureGraphSnapshot.cursor.sequence + 1 },
        metrics: { orphan_count: 0, broken_link_count: 0, stale_concept_count: 1, warning_count: 1 },
      },
    });
    expect(refreshed.freshnessStatus).toBe("warning");
    expect(refreshed.staleBundleIds).toEqual([]);
    expect(refreshed.warningBundleIds).toEqual(["local-default"]);
  });

  it("does not let an older snapshot clear a newer graph update", () => {
    const current = graphReducer(initialGraphState, {
      type: "SNAPSHOT_LOADED",
      snapshot: fixtureGraphSnapshot,
    });
    const stale = graphReducer(current, {
      type: "GRAPH_UPDATED",
      event: {
        schema_version: fixtureGraphSnapshot.schema_version,
        bundle_id: fixtureGraphSnapshot.bundle_id,
        cursor: { sequence: fixtureGraphSnapshot.cursor.sequence + 2, partition: fixtureGraphSnapshot.cursor.partition },
        updated_at: "2026-06-28T00:02:00Z",
      },
    });

    const unchanged = graphReducer(stale, {
      type: "SNAPSHOT_LOADED",
      snapshot: {
        ...fixtureGraphSnapshot,
        cursor: { ...fixtureGraphSnapshot.cursor, sequence: fixtureGraphSnapshot.cursor.sequence + 1 },
      },
    });

    expect(unchanged.freshnessStatus).toBe("stale");
    expect(unchanged.layoutStatus).toBe("idle");
    expect(unchanged.staleBundleIds).toEqual(["local-default"]);
    expect(unchanged.snapshots["local-default"].cursor.sequence).toBe(1);

    const loading = graphReducer(stale, { type: "LAYOUT_STATUS_SET", status: "loading" });
    const stillLoading = graphReducer(loading, {
      type: "SNAPSHOT_LOADED",
      snapshot: {
        ...fixtureGraphSnapshot,
        cursor: { ...fixtureGraphSnapshot.cursor, sequence: fixtureGraphSnapshot.cursor.sequence + 1 },
      },
    });
    expect(stillLoading.layoutStatus).toBe("loading");
  });

  it("compares stale graph cursors within their partitions", () => {
    const current = graphReducer(initialGraphState, {
      type: "SNAPSHOT_LOADED",
      snapshot: fixtureGraphSnapshot,
    });
    const repartitioned = graphReducer(current, {
      type: "GRAPH_UPDATED",
      event: {
        schema_version: fixtureGraphSnapshot.schema_version,
        bundle_id: fixtureGraphSnapshot.bundle_id,
        cursor: { sequence: 0, partition: "memory-graph:local-default:v2" },
        updated_at: "2026-06-28T00:03:00Z",
      },
    });

    expect(repartitioned.freshnessStatus).toBe("stale");
    expect(repartitioned.staleBundleIds).toEqual(["local-default"]);

    const oldPartitionSnapshot = graphReducer(repartitioned, {
      type: "SNAPSHOT_LOADED",
      snapshot: fixtureGraphSnapshot,
    });

    expect(oldPartitionSnapshot.freshnessStatus).toBe("stale");
    expect(oldPartitionSnapshot.staleBundleIds).toEqual(["local-default"]);
    expect(oldPartitionSnapshot.snapshots["local-default"].cursor).toEqual(fixtureGraphSnapshot.cursor);

    const refreshed = graphReducer(repartitioned, {
      type: "SNAPSHOT_LOADED",
      snapshot: {
        ...fixtureGraphSnapshot,
        cursor: { sequence: 0, partition: "memory-graph:local-default:v2" },
      },
    });

    expect(refreshed.freshnessStatus).toBe("current");
    expect(refreshed.staleBundleIds).toEqual([]);
    expect(refreshed.snapshots["local-default"].cursor).toEqual({
      sequence: 0,
      partition: "memory-graph:local-default:v2",
    });
  });

  it("rejects out-of-order same-partition snapshots without a stale cursor marker", () => {
    const current = graphReducer(initialGraphState, {
      type: "SNAPSHOT_LOADED",
      snapshot: {
        ...fixtureGraphSnapshot,
        cursor: { ...fixtureGraphSnapshot.cursor, sequence: 5 },
      },
    });

    const unchanged = graphReducer(current, {
      type: "SNAPSHOT_LOADED",
      snapshot: {
        ...fixtureGraphSnapshot,
        cursor: { ...fixtureGraphSnapshot.cursor, sequence: 4 },
      },
    });

    expect(unchanged.snapshots["local-default"].cursor.sequence).toBe(5);
    expect(unchanged.freshnessStatus).toBe("current");
    expect(unchanged.staleBundleIds).toEqual([]);
  });

  it("rejects non-OK HTTP graph responses before parsing JSON", async () => {
    const json = jest.fn();
    const fetchMock = jest.fn(async () => ({
      ok: false,
      status: 503,
      json,
    })) as unknown as typeof fetch;
    const gateway = createGatewayGraphAdapter("http://localhost:2468", fetchMock);

    await expect(gateway.listBundles()).rejects.toThrow("Graph request failed: HTTP 503");
    expect(json).not.toHaveBeenCalled();
  });

  it("keeps filtered community concept counts aligned to concept nodes only", () => {
    const filtered = applyGraphFilters(fixtureGraphSnapshot, {
      ...initialGraphFilters,
      communities: ["area:graph-view"],
    });
    expect(filtered.communities).toEqual([
      {
        id: "area:graph-view",
        label: "Graph View",
        node_ids: ["concept:coe-465", "tag:graph-view"],
        concept_count: 1,
      },
    ]);
  });

  it("does not crash when community-filtered DTO nodes omit metrics", () => {
    const nodeWithoutMetrics = { ...fixtureGraphSnapshot.nodes[0] } as Record<string, unknown>;
    delete nodeWithoutMetrics.metrics;

    const filtered = applyGraphFilters(
      {
        ...fixtureGraphSnapshot,
        nodes: [nodeWithoutMetrics as never],
      },
      {
        ...initialGraphFilters,
        communities: ["area:graph-view"],
      },
    );

    expect(filtered.nodes).toEqual([]);
  });

  it("sorts communities with code-point ordering", () => {
    const filtered = applyGraphFilters({
      ...fixtureGraphSnapshot,
      communities: [
        {
          id: "a-community",
          label: "lowercase",
          node_ids: ["concept:coe-465"],
          concept_count: 1,
        },
        {
          id: "B-community",
          label: "uppercase",
          node_ids: ["concept:coe-465"],
          concept_count: 1,
        },
      ],
    }, initialGraphFilters);

    expect(filtered.communities.map((community) => community.id)).toEqual([
      "B-community",
      "a-community",
    ]);
  });

  it("preserves depth zero for focused neighborhoods", () => {
    const state = graphReducer(initialGraphState, {
      type: "NODE_FOCUSED",
      nodeId: "concept:coe-465",
      neighborhoodDepth: 0,
    });

    expect(state.neighborhoodDepth).toBe(0);
  });

  it("expands neighborhoods beyond direct neighbors deterministically", () => {
    const filtered = applyGraphFilters(
      fixtureGraphSnapshot,
      initialGraphFilters,
      "neighborhood",
      "tag:graph-view",
      2,
    );
    expect(filtered.nodes.map((node) => node.id)).toEqual([
      "concept:coe-465",
      "source:osym-822",
      "bundle:local-default",
      "tag:graph-view",
    ]);
    expect(filtered.filters_applied).toEqual([
      "neighborhood-depth:2",
      "neighborhood:tag:graph-view",
    ]);
  });

  it("passes through native graph adapters without importing Tauri", async () => {
    const native = createTauriNativeGraphAdapter(createFixtureGraphAdapter());
    await expect(native.getCommunities("local-default")).resolves.toMatchObject({
      communities: [{ id: "area:graph-view", concept_count: 1 }],
    });
  });

  it("honors fixture search bundle and limit options", async () => {
    const fixture = createFixtureGraphAdapter();
    await expect(fixture.search("graph", { bundleId: "other-bundle" })).resolves.toMatchObject({
      bundle_id: "other-bundle",
      results: [],
    });
    await expect(fixture.search("graph", { bundleId: "local-default", limit: 1 })).resolves.toMatchObject({
      bundle_id: "local-default",
      results: [{ concept_id: "issues/COE-465" }],
    });
  });

  it("computes deterministic graph layouts for every graph mode", () => {
    const layouts = ["atlas", "bundle", "neighborhood", "timeline"] as const;
    const byMode = new Map<(typeof layouts)[number], ReturnType<typeof computeGraphLayout>>();
    for (const mode of layouts) {
      const layout = computeGraphLayout(fixtureGraphSnapshot, {
        kind: graphLayoutKindForMode(mode),
        focusedNodeId: "concept:coe-465",
        width: 640,
        height: 360,
      });
      byMode.set(mode, layout);
      expect(new Set(layout.nodes.map((node) => node.nodeId))).toEqual(new Set([
        "bundle:local-default",
        "concept:coe-465",
        "source:osym-822",
        "tag:graph-view",
      ]));
      expect(layout.edges).toHaveLength(3);
      expect(layout.nodes.every((node) => node.x >= 0 && node.x <= 640 && node.y >= 0 && node.y <= 360)).toBe(true);
    }
    const bundle = nodeById(byMode.get("bundle")!, "bundle:local-default");
    const concept = nodeById(byMode.get("bundle")!, "concept:coe-465");
    const tag = nodeById(byMode.get("bundle")!, "tag:graph-view");
    expect(bundle.x).toBeLessThan(concept.x);
    expect(concept.x).toBeLessThan(tag.x);
    const focused = nodeById(byMode.get("neighborhood")!, "concept:coe-465");
    expect(focused.x).toBeCloseTo(320);
    expect(focused.y).toBeCloseTo(180);
    const timeline = byMode.get("timeline")!;
    expect(new Set(timeline.nodes.map((node) => node.x)).size).toBe(fixtureGraphSnapshot.nodes.length);
    const timelineConcept = nodeById(timeline, "concept:coe-465");
    const timelineTag = nodeById(timeline, "tag:graph-view");
    expect(timelineConcept.y).not.toBe(timelineTag.y);
    const timestamped = computeGraphLayout({
      ...fixtureGraphSnapshot,
      nodes: [
        { ...fixtureGraphSnapshot.nodes[0], id: "concept:older", kind: "concept", timestamp: "2026-01-01T00:00:00Z" },
        { ...fixtureGraphSnapshot.nodes[1], id: "concept:newer", kind: "concept", timestamp: "2026-02-01T00:00:00Z" },
      ],
      edges: [],
    }, { kind: "timeline", width: 640, height: 360 });
    expect(nodeById(timestamped, "concept:older").x).toBeLessThan(nodeById(timestamped, "concept:newer").x);
    const offsetTimestamped = computeGraphLayout({
      ...fixtureGraphSnapshot,
      nodes: [
        { ...fixtureGraphSnapshot.nodes[0], id: "concept:later-offset", kind: "concept", timestamp: "2026-01-01T01:00:00+02:00" },
        { ...fixtureGraphSnapshot.nodes[1], id: "concept:earlier-z", kind: "concept", timestamp: "2025-12-31T23:30:00Z" },
      ],
      edges: [],
    }, { kind: "timeline", width: 640, height: 360 });
    expect(nodeById(offsetTimestamped, "concept:later-offset").x).toBeLessThan(nodeById(offsetTimestamped, "concept:earlier-z").x);
  });

  it("uses a worker adapter when a worker is available", async () => {
    class FakeWorker {
      onmessage: ((event: MessageEvent) => void) | null = null;
      onerror: ((event: ErrorEvent) => void) | null = null;
      terminated = false;

      postMessage(message: { id: number; snapshot: typeof fixtureGraphSnapshot; options: { kind: "force" } }): void {
        const result = computeGraphLayout(message.snapshot, message.options);
        queueMicrotask(() => this.onmessage?.({ data: { id: message.id, result } } as MessageEvent));
      }

      terminate(): void {
        this.terminated = true;
      }
    }
    const worker = new FakeWorker();
    const adapter = createGraphLayoutAdapter(() => worker as unknown as Worker);
    const layout = await adapter.layout(fixtureGraphSnapshot, { kind: "force" });
    expect(layout.nodes).toHaveLength(fixtureGraphSnapshot.nodes.length);
    adapter.dispose();
    expect(worker.terminated).toBe(true);
  });

  it("returns fresh objects for graph and filter resets", () => {
    const dirty = {
      ...createInitialGraphState(),
      filters: { ...initialGraphFilters, tags: ["graph-view"] },
    };
    const filtersReset = graphReducer(dirty, { type: "FILTERS_RESET" });
    const graphReset = graphReducer(dirty, { type: "GRAPH_RESET" });

    expect(filtersReset.filters).toEqual(initialGraphFilters);
    expect(filtersReset.filters).not.toBe(initialGraphFilters);
    expect(graphReset).toEqual(initialGraphState);
    expect(graphReset).not.toBe(initialGraphState);
    expect(graphReset.filters).not.toBe(initialGraphFilters);
  });
});

function nodeById(
  layout: ReturnType<typeof computeGraphLayout>,
  nodeId: string,
): ReturnType<typeof computeGraphLayout>["nodes"][number] {
  const node = layout.nodes.find((candidate) => candidate.nodeId === nodeId);
  if (!node) throw new Error(`Missing layout node ${nodeId}`);
  return node;
}
