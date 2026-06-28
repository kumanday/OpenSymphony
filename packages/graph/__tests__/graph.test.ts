import {
  applyGraphFilters,
  createFixtureGraphAdapter,
  createInitialGraphState,
  createGatewayGraphAdapter,
  createTauriNativeGraphAdapter,
  fixtureGraphSnapshot,
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
      },
    });

    expect(restored.selectedBundleId).toBeNull();
    expect(restored.focusedNodeId).toBeNull();
    expect(restored.selectedNodeIds).toEqual([]);
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
