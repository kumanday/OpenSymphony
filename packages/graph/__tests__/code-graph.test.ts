import {
  applyCodeGraphFilters,
  codeGraphReducer,
  codeGraphSnapshotForRendering,
  createFixtureCodeGraphAdapter,
  createHttpCodeGraphAdapter,
  createInitialCodeGraphState,
  createTauriNativeCodeGraphAdapter,
  codeDeepLinkFromLocationSearch,
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

  it("matches path filters only on exact or directory boundaries", async () => {
    const snapshot = await createFixtureCodeGraphAdapter().getGraphSnapshot("opensymphony", { mode: "file" });
    const sibling = {
      ...snapshot.nodes.find((node) => node.kind === "symbol")!,
      id: "symbol:graphite",
      label: "graphite",
      symbol_key: "graphite",
      path_display: "packages/graphite/src/index.ts",
    };
    const filtered = applyCodeGraphFilters(
      { ...snapshot, nodes: [...snapshot.nodes, sibling] },
      { ...initialCodeGraphFilters, pathPrefixes: ["packages/graph"] },
    );
    expect(filtered.nodes.some((node) => node.symbol_key === "graphite")).toBe(false);
    expect(filtered.nodes.some((node) => node.path_display === "packages/graph/src/index.ts")).toBe(true);
  });

  it("preserves selection while a newer code snapshot refreshes", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const snapshot = await adapter.getGraphSnapshot("opensymphony", { mode: "neighborhood" });
    const detail = await adapter.getSymbolDetail("opensymphony", "graphReducer");
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot });
    state = codeGraphReducer(state, { type: "SYMBOL_DETAIL_LOADED", detail });
    state = codeGraphReducer(state, { type: "NODE_SELECTED", nodeId: "symbol:graphReducer" });
    state = codeGraphReducer(state, { type: "GRAPH_UPDATED", repoId: "opensymphony", updatedAt: "2026-07-04T01:00:00Z" });
    expect(state.selectedNodeIds).toEqual(["symbol:graphReducer"]);
    expect(state.stale).toBe(true);
    state = codeGraphReducer(state, { type: "SNAPSHOT_LOADED", snapshot: { ...snapshot, cursor: { ...snapshot.cursor, sequence: 10 } } });
    expect(state.selectedNodeIds).toEqual(["symbol:graphReducer"]);
    expect(state.stale).toBe(false);
    expect(state.symbolDetails).toEqual({});
  });

  it("keeps removed diff symbols visible and clears diff state when leaving Diff", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const snapshot = await adapter.getGraphSnapshot("opensymphony", { mode: "neighborhood" });
    const overlay = {
      ...codeGraphFixtureDiffOverlays[0],
      removed_symbols: [{
        symbol_key: "removedSymbol",
        status: "removed" as const,
        before: {
          symbol_id: "removedSymbol:id",
          kind: "function",
          name: "removedSymbol",
          path_display: "packages/graph/src/removed.ts",
          container_chain: ["graph"],
          span: { start_line: 1, start_col: 1, end_line: 4, end_col: 2 },
          freshness: "current" as const,
        },
        after: null,
      }],
    };
    const filtered = applyCodeGraphFilters(snapshot, { ...initialCodeGraphFilters, deltaStatuses: ["removed"] }, overlay);
    expect(filtered.nodes.map((node) => node.symbol_key)).toEqual(["removedSymbol"]);
    expect(codeGraphSnapshotForRendering(snapshot, overlay).nodes.some((node) => node.concept_id === "removedSymbol")).toBe(true);

    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "DIFF_LOADED", overlay });
    state = codeGraphReducer(state, { type: "FILTERS_SET", filters: { deltaStatuses: ["removed"] } });
    state = codeGraphReducer(state, { type: "MODE_SET", mode: "atlas" });
    expect(state.diffOverlay).toBeNull();
    expect(state.baseRevision).toBeNull();
    expect(state.filters.deltaStatuses).toEqual([]);
  });

  it("synthesizes added and modified diff symbols and honors explicit target clears", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const snapshot = await adapter.getGraphSnapshot("opensymphony", { mode: "atlas" });
    const side = {
      symbol_id: "modifiedSymbol:id",
      kind: "function",
      name: "modifiedSymbol",
      path_display: "packages/graph/src/modified.ts",
      container_chain: ["graph"],
      span: { start_line: 1, start_col: 1, end_line: 4, end_col: 2 },
      freshness: "current" as const,
    };
    const overlay = {
      ...codeGraphFixtureDiffOverlays[0],
      added_symbols: [{
        symbol_key: "addedSymbol",
        status: "added" as const,
        before: null,
        after: { ...side, symbol_id: "addedSymbol:id", name: "addedSymbol" },
      }],
      removed_symbols: [],
      modified_symbols: [{
        symbol_key: "modifiedSymbol",
        status: "modified" as const,
        before: { ...side, name: "oldModifiedSymbol" },
        after: side,
      }],
    };
    const filtered = applyCodeGraphFilters(snapshot, {
      ...initialCodeGraphFilters,
      deltaStatuses: ["added", "modified"],
    }, overlay);
    expect(filtered.nodes.map((node) => node.symbol_key)).toEqual(["addedSymbol", "modifiedSymbol"]);

    let state = codeGraphReducer(createInitialCodeGraphState(), {
      type: "TARGET_SET",
      path: "packages/graph/src/index.ts",
      symbolKey: "oldSymbol",
    });
    state = codeGraphReducer(state, {
      type: "DRILL_IN",
      breadcrumb: { kind: "file", id: "packages/graph/src/index.ts", label: "index.ts" },
      mode: "file",
      path: null,
      symbolKey: null,
    });
    expect(state.path).toBeNull();
    expect(state.symbolKey).toBeNull();
    state = codeGraphReducer(state, {
      type: "DRILL_IN",
      breadcrumb: { kind: "symbol", id: "newSymbol", nodeId: "sym:newSymbol", label: "newSymbol" },
      mode: "neighborhood",
      symbolKey: "newSymbol",
    });
    state = codeGraphReducer(state, { type: "BREADCRUMB_POP", index: 1 });
    expect(state.symbolKey).toBe("newSymbol");
    expect(state.selectedNodeIds).toEqual(["sym:newSymbol"]);
  });

  it("uses community node membership without requiring a metrics community id", async () => {
    const adapter = createFixtureCodeGraphAdapter();
    const snapshot = await adapter.getGraphSnapshot("opensymphony", { mode: "file" });
    const member = snapshot.nodes.find((node) => node.kind === "symbol")!;
    const communitySnapshot = {
      ...snapshot,
      nodes: [{ ...member, metrics: { ...member.metrics, community_id: null } }],
      edges: [],
      communities: [{ id: "community:code", label: "Code", node_ids: [member.id], symbol_count: 1 }],
    };
    const filtered = applyCodeGraphFilters(communitySnapshot, {
      ...initialCodeGraphFilters,
      communities: ["community:code"],
    });
    expect(filtered.nodes.map((node) => node.id)).toEqual([member.id]);
  });

  it("clears an old diff overlay when restoring non-Diff history", () => {
    const overlay = codeGraphFixtureDiffOverlays[0];
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "DIFF_LOADED", overlay });
    state = codeGraphReducer(state, {
      type: "HISTORY_RESTORED",
      state: { mode: "atlas", baseRevision: null, headRevision: null, symbolKey: null, path: null },
    });
    expect(state.mode).toBe("atlas");
    expect(state.diffOverlay).toBeNull();
  });

  it("pops the final breadcrumb back to the Atlas and clears its path scope", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), {
      type: "DRILL_IN",
      breadcrumb: { kind: "directory", id: "packages/graph", label: "graph" },
      mode: "atlas",
      path: "packages/graph",
    });
    state = codeGraphReducer(state, { type: "FILTERS_SET", filters: { pathPrefixes: ["packages/graph"] } });
    state = codeGraphReducer(state, { type: "BREADCRUMB_POP" });
    expect(state.mode).toBe("atlas");
    expect(state.path).toBeNull();
    expect(state.breadcrumbs).toEqual([]);
    expect(state.filters.pathPrefixes).toEqual([]);
  });

  it("resets a repository switch to an aggregated Atlas request", () => {
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "MODE_SET", mode: "file" });
    state = codeGraphReducer(state, { type: "TARGET_SET", path: "packages/graph/src/index.ts" });
    state = codeGraphReducer(state, { type: "REPO_SELECTED", repoId: "other-repo" });
    expect(state.mode).toBe("atlas");
    expect(state.path).toBeNull();
    expect(state.symbolKey).toBeNull();
    expect(state.baseRevision).toBeNull();
    expect(state.headRevision).toBeNull();
    expect(state.diffOverlay).toBeNull();
  });

  it("falls back to an available repository when the selected one disappears", async () => {
    const repos = await createFixtureCodeGraphAdapter().listRepos();
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "REPO_SELECTED", repoId: "stale-only" });
    state = codeGraphReducer(state, {
      type: "REPOS_LOADED",
      repos: { ...repos, repos: [{ ...repos.repos[0], repo_id: "available" }] },
    });
    expect(state.repoId).toBe("available");
  });

  it("resets scoped state when repository fallback changes the selected repo", async () => {
    const snapshot = await createFixtureCodeGraphAdapter().getGraphSnapshot("opensymphony", { mode: "file" });
    const repos = await createFixtureCodeGraphAdapter().listRepos();
    let state = codeGraphReducer(createInitialCodeGraphState(), { type: "SNAPSHOT_LOADED", snapshot: { ...snapshot, repo_id: "stale-only" } });
    state = codeGraphReducer(state, {
      type: "DRILL_IN",
      breadcrumb: { kind: "file", id: "packages/graph/src/index.ts", label: "index.ts" },
      mode: "file",
      path: "packages/graph/src/index.ts",
    });
    state = codeGraphReducer(state, { type: "TARGET_SET", symbolKey: "oldSymbol" });
    state = codeGraphReducer(state, {
      type: "FILTERS_SET",
      filters: { repoIds: ["stale-only"], pathPrefixes: ["old/"], communities: ["old-community"] },
    });
    state = codeGraphReducer(state, {
      type: "REPOS_LOADED",
      repos: { ...(await createFixtureCodeGraphAdapter().listRepos()), repos: [{ ...repos.repos[0], repo_id: "available" }] },
    });
    expect(state).toMatchObject({
      repoId: "available",
      mode: "atlas",
      snapshot: null,
      symbolKey: null,
      path: null,
      selectedNodeIds: [],
      breadcrumbs: [],
      filters: { repoIds: [], pathPrefixes: [], communities: [], deltaStatuses: [] },
    });
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
    await http.listRepos({ includeStale: true });
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:2468/api/v1/code/repos?include_stale=true");
    await http.getGraphSnapshot("opensymphony", { mode: "atlas", includeStale: true });
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:2468/api/v1/code/repos/opensymphony/graph?mode=atlas&include_stale=true");
    await http.getSymbolDetail("opensymphony", "staleSymbol", { includeStale: true });
    expect(fetchMock).toHaveBeenCalledWith("http://localhost:2468/api/v1/code/repos/opensymphony/symbols/staleSymbol?include_stale=true");
    const native = createTauriNativeCodeGraphAdapter(http);
    await native.listRepos();
    expect(fetchMock).toHaveBeenCalledTimes(5);
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
    expect(parseCodeDeepLink("opensymphony://code/repo/symbols/run?mode=diff")).toBeNull();
    expect(() => formatCodeDeepLink({ repoId: "repo", baseRevision: "base" })).toThrow(/both base and head/);
    expect(() => formatCodeDeepLink({ repoId: "repo", symbolKey: "run", mode: "diff" })).toThrow(/target does not match/);
  });

  it("rejects invalid enum-valued filter entries", () => {
    const filters = encodeURIComponent(JSON.stringify({
      ...initialCodeGraphFilters,
      freshness: ["expired"],
    }));
    expect(parseCodeDeepLink(`opensymphony://code/repo/atlas?filters=${filters}`)).toBeNull();
    const deltaFilters = encodeURIComponent(JSON.stringify({
      ...initialCodeGraphFilters,
      deltaStatuses: ["renamed"],
    }));
    expect(parseCodeDeepLink(`opensymphony://code/repo/atlas?filters=${deltaFilters}`)).toBeNull();
  });

  it("recovers raw and encoded Code Graph boot links with composable app params", () => {
    const link = formatCodeDeepLink({
      repoId: "repo",
      symbolKey: "module::run",
      depth: 2,
      runId: "run-1",
      layoutSeed: "seed-1",
    });
    expect(codeDeepLinkFromLocationSearch(`?fixtures&code=${link}`)).toBe(link);
    expect(codeDeepLinkFromLocationSearch(`?code=${encodeURIComponent(link)}&fixtures`)).toBe(link);
    expect(codeDeepLinkFromLocationSearch(`?code=${link}&fixtures`)).toBe(link);
    expect(codeDeepLinkFromLocationSearch(`?code=${link}&unexpected=1`)).toBeNull();
  });
});
