import {
  computeGraphLayout,
  createInitialGraphState,
  fixtureGraphSnapshot,
  graphReducer,
} from "@opensymphony/graph";
import { renderKnowledgeGraphSurface } from "../src/knowledge-graph-renderer.js";

describe("Knowledge Graph renderer", () => {
  it("renders the canvas shell and semantic fallback list", () => {
    const state = graphReducer(
      graphReducer(createInitialGraphState(), { type: "SNAPSHOT_LOADED", snapshot: fixtureGraphSnapshot }),
      { type: "LAYOUT_STATUS_SET", status: "ready" },
    );
    const layout = computeGraphLayout(fixtureGraphSnapshot, {
      kind: "force",
      width: 640,
      height: 360,
    });
    const html = renderKnowledgeGraphSurface({
      snapshot: fixtureGraphSnapshot,
      layout,
      state,
    });

    expect(html).toContain(`data-testid="knowledge-graph-renderer" data-layout-status="ready"`);
    expect(html).toContain(`data-testid="knowledge-graph-canvas"`);
    expect(html).toContain(`aria-label="Visible graph nodes"`);
    expect(html).toContain("concept");
  });
});
