import {
  applyGraphFilters,
  createFixtureGraphAdapter,
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
      "tag:graph-view",
      "bundle:local-default",
      "source:osym-822",
    ]);
  });

  it("passes through native graph adapters without importing Tauri", async () => {
    const native = createTauriNativeGraphAdapter(createFixtureGraphAdapter());
    await expect(native.getCommunities("local-default")).resolves.toMatchObject({
      communities: [{ id: "area:graph-view", concept_count: 1 }],
    });
  });
});
