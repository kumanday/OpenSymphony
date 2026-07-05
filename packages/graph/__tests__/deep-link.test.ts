import {
  cachedConceptDetail,
  createFixtureGraphAdapter,
  createInitialGraphState,
  formatMemoryDeepLink,
  graphReducer,
  graphVizFixtureConceptDetail,
  graphVizFixtureSnapshot,
  memoryDeepLinkForGraphNode,
  memoryDeepLinkToGraphState,
  parseMemoryDeepLink,
  resolveMemoryDeepLinkNode,
} from "@opensymphony/graph";

describe("memory deep links", () => {
  it("round-trips bundle, community, and concept links", () => {
    const bundle = formatMemoryDeepLink({ bundleId: "viz-workbench" });
    expect(bundle).toBe("opensymphony://memory/viz-workbench");
    expect(parseMemoryDeepLink(bundle)).toEqual({
      bundleId: "viz-workbench",
      conceptId: null,
      communityId: null,
    });

    const community = formatMemoryDeepLink({ bundleId: "viz-workbench", communityId: "area:memory-graph" });
    expect(community).toBe("opensymphony://memory/viz-workbench/communities/area%3Amemory-graph");
    expect(parseMemoryDeepLink(community)).toEqual({
      bundleId: "viz-workbench",
      conceptId: null,
      communityId: "area:memory-graph",
    });

    const concept = formatMemoryDeepLink({ bundleId: "viz-workbench", conceptId: "issues/COE-399" });
    expect(concept).toBe("opensymphony://memory/viz-workbench/concepts/issues/COE-399");
    expect(parseMemoryDeepLink(concept)).toEqual({
      bundleId: "viz-workbench",
      conceptId: "issues/COE-399",
      communityId: null,
    });
  });

  it("percent-encodes hostile segment characters and decodes them back", () => {
    const link = formatMemoryDeepLink({ bundleId: "bundle id", conceptId: "issues/COE 399?x=#y" });
    expect(link).not.toMatch(/[ ?#]/);
    expect(parseMemoryDeepLink(link)).toEqual({
      bundleId: "bundle id",
      conceptId: "issues/COE 399?x=#y",
      communityId: null,
    });
  });

  it("rejects malformed links instead of guessing", () => {
    expect(parseMemoryDeepLink("https://example.com/memory/x")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/bundle/unknown/x")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/bundle/concepts")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/bundle/communities/a/b")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/bundle//concepts/x")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/bundle/concepts/x?query=1")).toBeNull();
    expect(parseMemoryDeepLink("opensymphony://memory/bundle/concepts/%ZZ")).toBeNull();
    expect(() => formatMemoryDeepLink({ bundleId: "" })).toThrow(/bundle id/);
  });

  it("builds node deep links only for addressable node kinds", () => {
    const snapshot = graphVizFixtureSnapshot;
    const concept = snapshot.nodes.find((node) => node.kind === "concept")!;
    const conceptLink = memoryDeepLinkForGraphNode(snapshot.bundle_id, concept)!;
    expect(conceptLink).toContain("/concepts/");

    const parsed = parseMemoryDeepLink(conceptLink)!;
    expect(resolveMemoryDeepLinkNode(snapshot, parsed)?.id).toBe(concept.id);

    const tag = snapshot.nodes.find((node) => node.kind === "tag")!;
    expect(memoryDeepLinkForGraphNode(snapshot.bundle_id, tag)).toBeNull();

    const bundle = snapshot.nodes.find((node) => node.kind === "bundle")!;
    const bundleLink = memoryDeepLinkForGraphNode(snapshot.bundle_id, bundle)!;
    expect(resolveMemoryDeepLinkNode(snapshot, parseMemoryDeepLink(bundleLink)!)?.id).toBe(bundle.id);
  });

  it("maps deep links to HISTORY_RESTORED-compatible state", () => {
    const communityState = memoryDeepLinkToGraphState({
      bundleId: "viz-workbench",
      conceptId: null,
      communityId: "area:gateway",
    });
    expect(communityState.mode).toBe("community");
    expect(communityState.bundleId).toBe("viz-workbench");

    const conceptState = memoryDeepLinkToGraphState({
      bundleId: "viz-workbench",
      conceptId: "concepts/gateway-01",
      communityId: null,
    });
    expect(conceptState.mode).toBe("atlas");
  });
});

describe("viz fixture concept details", () => {
  it("returns a deterministic capsule whose links resolve to snapshot nodes", () => {
    const concept = graphVizFixtureSnapshot.nodes.find((node) => node.kind === "concept")!;
    const detail = graphVizFixtureConceptDetail(concept.concept_id!)!;
    expect(detail).not.toBeNull();
    expect(detail.concept_id).toBe(concept.concept_id);
    expect(detail.body_markdown).toContain("## Summary");
    expect(graphVizFixtureConceptDetail(concept.concept_id!)).toEqual(detail);
    // Also resolvable by node id.
    expect(graphVizFixtureConceptDetail(concept.id)?.concept_id).toBe(concept.concept_id);

    const conceptIds = new Set(
      graphVizFixtureSnapshot.nodes
        .filter((node) => node.kind === "concept")
        .flatMap((node) => [node.id, node.concept_id ?? node.id]),
    );
    for (const link of detail.links) {
      expect(conceptIds.has(link.target)).toBe(true);
    }
  });

  it("covers every fixture concept and rejects unknown ids", () => {
    const concepts = graphVizFixtureSnapshot.nodes.filter((node) => node.kind === "concept");
    for (const concept of concepts) {
      expect(graphVizFixtureConceptDetail(concept.concept_id!)).not.toBeNull();
    }
    expect(graphVizFixtureConceptDetail("concepts/does-not-exist")).toBeNull();
  });
});

describe("fixture graph adapter concept resolution", () => {
  it("supports resolver-style concept details and rejects missing concepts", async () => {
    const adapter = createFixtureGraphAdapter({
      snapshot: graphVizFixtureSnapshot,
      conceptDetail: (_bundleId, conceptId) => graphVizFixtureConceptDetail(conceptId),
    });
    const concept = graphVizFixtureSnapshot.nodes.find((node) => node.kind === "concept")!;
    const detail = await adapter.getConceptDetail(graphVizFixtureSnapshot.bundle_id, concept.concept_id!);
    expect(detail.concept_id).toBe(concept.concept_id);
    await expect(
      adapter.getConceptDetail(graphVizFixtureSnapshot.bundle_id, "concepts/none"),
    ).rejects.toThrow(/not found/i);
  });

  it("caches loaded details behind cachedConceptDetail", () => {
    const concept = graphVizFixtureSnapshot.nodes.find((node) => node.kind === "concept")!;
    const detail = graphVizFixtureConceptDetail(concept.concept_id!)!;
    let state = createInitialGraphState();
    expect(cachedConceptDetail(state, detail.bundle_id, detail.concept_id)).toBeNull();
    state = graphReducer(state, { type: "CONCEPT_DETAIL_LOADED", detail });
    expect(cachedConceptDetail(state, detail.bundle_id, detail.concept_id)).toEqual(detail);
  });
});
