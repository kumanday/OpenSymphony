import {
  applyCodeGraphFilters,
  codeGraphReducer,
  createFixtureCodeGraphAdapter,
  createHttpCodeGraphAdapter,
  createInitialCodeGraphState,
  createTauriNativeCodeGraphAdapter,
  formatCodeDeepLink,
  initialCodeGraphFilters,
  parseCodeDeepLink,
  codeGraphFixtureDiffOverlays,
} from "@opensymphony/graph";

describe("Code Graph adapters and state", () => {
  it("keeps Atlas aggregated and serves scoped File and Neighborhood fixtures", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const atlas = await adapter.getGraphSnapshot("opensymphony", { mode: "atlas", aggregate: "directory" });
    expect(atlas.nodes.every((node) => node.kind !== "symbol")).toBe(true);
    expect(atlas.truncation.reason).toBe("directory aggregation");

    await expect(adapter.getGraphSnapshot("opensymphony", { mode: "file", path: "packages/graph/src/index.ts" }))
      .resolves.toMatchObject({ mode: "file", nodes: expect.arrayContaining([expect.objectContaining({ kind: "symbol" })]) });
    await expect(adapter.getGraphSnapshot("opensymphony", { mode: "neighborhood", symbolKey: "graphReducer", depth: 1 }))
      .resolves.toMatchObject({ mode: "neighborhood" });
  });

  it("filters code records by freshness, diagnostics, path, confidence, and diff status", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const snapshot = await adapter.getGraphSnapshot("opensymphony", { mode: "file" });
    const filtered = applyCodeGraphFilters(snapshot, {
      ...initialCodeGraphFilters,
      languages: ["typescript"],
      freshness: ["stale"],
      diagnostics: "with_diagnostics",
      pathPrefixes: ["packages/graph/"],
      confidences: ["syntactic"],
      deltaStatuses: ["modified"],
    }, codeGraphFixtureDiffOverlays[0]);
    expect(filtered.nodes.map((node) => node.label)).toEqual(["codeGraphReducer"]);
    expect(filtered.edges.every((edge) => edge.confidence === "syntactic")).toBe(true);
  });

  it("preserves selection while a newer code snapshot refreshes", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const snapshot = await adapter.getGraphSnapshot("opensymphony", { mode: "neighborhood" });
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:graphReducer" });
    state = codeGraphReducer(state, { type: "GRAPH_UPDATED", repoId: "opensymphony", updatedAt: "2026-07-04T01:00:00Z" });
    expect(state.selectedNodeIds).toEqual(["symbol:graphReducer"]);
    expect(state.stale).toBe(true);
    state = codeGraphReducer(state, { type: "SNAPSHOT_LOADED", snapshot: { ...snapshot, cursor: { ...snapshot.cursor, sequence: 10 } } });
    expect(state.selectedNodeIds).toEqual(["symbol:graphReducer"]);
    expect(state.stale).toBe(false);
  });

  it("keeps HTTP and native adapters on the same DTO contract", async () => {
    const fetchMock = jest.fn(async (url: string) => ({
      ok: true,
      status: 200,
      json: async () => ({ repos: [] }),
    })) as unknown as typeof fetch;
    const http = createHttpCodeGraphAdapter("http://localhost:2468", fetchMock);
    await http.listRepos();
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:2468/api/v1/code/repos");
    const native = createTauriNativeCodeGraphAdapter(http);
    await native.listRepos();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});

describe("Code Graph deep links", () => {
  it("round-trips symbol state, filters, depth, and layout seed", () => {
    const link = formatCodeDeepLink({
      repoId: "team/repo",
      symbolKey: "crate::module::run",
      depth: 2,
      runId: "run/123",
      layoutSeed: "seed-7",
      filters: {
        ...initialCodeGraphFilters,
        languages: ["rust"],
        confidences: ["syntactic"],
      },
    });
    const parsed = parseCodeDeepLink(link);
    expect(parsed).toMatchObject({
      repoId: "team/repo",
      mode: "neighborhood",
      symbolKey: "crate::module::run",
      depth: 2,
      runId: "run/123",
      layoutSeed: "seed-7",
    });
    expect(parsed?.filters.languages).toEqual(["rust"]);
    expect(formatCodeDeepLink(parsed!)).toBe(link);
  });

  it("round-trips file and base/head diff forms", () => {
    const file = formatCodeDeepLink({ repoId: "opensymphony", path: "src/lib.rs" });
    expect(parseCodeDeepLink(file)).toMatchObject({ mode: "file", path: "src/lib.rs" });
    const diff = formatCodeDeepLink({ repoId: "opensymphony", baseRevision: "base/rev", headRevision: "head-rev" });
    expect(parseCodeDeepLink(diff)).toMatchObject({ mode: "diff", baseRevision: "base/rev", headRevision: "head-rev" });
  });

  it("rejects unknown shapes and query keys", () => {
    expect(parseCodeDeepLink("https://example.com/code/repo/files/src/lib.rs")).toBeNull();
    expect(parseCodeDeepLink("opensymphony://code/repo/unknown/value")).toBeNull();
    expect(parseCodeDeepLink("opensymphony://code/repo/files/src/lib.rs?guess=1")).toBeNull();
    expect(parseCodeDeepLink("opensymphony://code/repo/diff/base")).toBeNull();
    expect(() => formatCodeDeepLink({ repoId: "repo", baseRevision: "base" })).toThrow(/both base and head/);
  });
});
