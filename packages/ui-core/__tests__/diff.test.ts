/** @jest-environment jsdom */

import type { CodeDiffOverlay, CodeFileOutline, FileDiffPage } from "@opensymphony/gateway-schema";
import { innermostSymbolAtLine, renderCodeDiffDeltaList, renderCodeDiffSummary, renderFileDiff, resolveDiffSymbolRegions } from "../src/diff.js";

const outline: CodeFileOutline = {
  schema_version: { major: 1, minor: 0, patch: 0 },
  run_id: "run-535",
  repo_id: "opensymphony",
  path: "src/lib.ts",
  generated_at: "2026-07-12T00:00:00Z",
  symbols: [
    {
      symbol_key: "outer",
      name: "outer",
      kind: "function",
      path: "src/lib.ts",
      span: { start_line: 1, start_col: 0, end_line: 8, end_col: 1 },
      selection_span: { start_line: 1, start_col: 0, end_line: 1, end_col: 5 },
      container_chain: [],
    },
    {
      symbol_key: "inner",
      name: "inner",
      kind: "function",
      path: "src/lib.ts",
      span: { start_line: 3, start_col: 0, end_line: 5, end_col: 1 },
      selection_span: { start_line: 3, start_col: 0, end_line: 3, end_col: 5 },
      container_chain: ["outer"],
    },
  ],
};

const diff: FileDiffPage = {
  schema_version: { major: 1, minor: 0, patch: 0 },
  run_id: "run-535",
  file_path: "src/lib.ts",
  next_cursor: undefined,
  hunks: [{
    file_path: "src/lib.ts",
    header: "@@ -1,5 +1,5 @@",
    start_line: 1,
    old_line_count: 5,
    new_line_count: 5,
    lines: [
      { type: "context", line: "function outer() {" },
      { type: "context", line: "  return inner();" },
      { type: "addition", line: "  const changed = true;" },
      { type: "addition", line: "  return changed;" },
    ],
  }],
  total_lines_added: 2,
  total_lines_removed: 0,
};

describe("diff symbol navigation", () => {
  it("chooses the innermost containing symbol", () => {
    expect(innermostSymbolAtLine(outline.symbols, 4)?.symbol_key).toBe("inner");
  });

  it("renders one glyph per changed symbol region", () => {
    const regions = resolveDiffSymbolRegions(diff, outline);
    expect(regions.map((region) => region.symbol.symbol_key)).toEqual(["inner"]);
    const root = document.createElement("div");
    root.innerHTML = renderFileDiff(diff, outline, (symbolKey) => `opensymphony://code/repo/diff/base/head/symbols/${symbolKey}`);
    expect(root.querySelectorAll("[data-diff-symbol-action]")).toHaveLength(1);
    expect(root.querySelector("[data-diff-symbol-action]")?.getAttribute("aria-label")).toContain("inner");
    expect(root.querySelector("[data-diff-symbol-copy]")?.getAttribute("data-diff-symbol-copy")).toContain("opensymphony://code/");
  });

  it("keeps deletion-only regions navigable and hides unavailable file actions", () => {
    const deletion: FileDiffPage = {
      ...diff,
      hunks: [{ ...diff.hunks[0], header: "@@ -9,1 +9,0 @@", lines: [{ type: "deletion", line: "}" }] }],
      total_lines_added: 0,
      total_lines_removed: 1,
    };
    expect(resolveDiffSymbolRegions(deletion, outline).map((region) => region.symbol.symbol_key)).toEqual(["outer"]);
    expect(renderFileDiff(diff, outline)).not.toContain("os-diff-file-graph");
    expect(renderFileDiff(diff, outline, (_symbolKey, path) => `opensymphony://code/file/${path}`)).toContain("os-diff-file-graph");
  });

  it("keeps file navigation available for empty diffs", () => {
    const emptyDiff: FileDiffPage = { ...diff, hunks: [], total_lines_added: 0, total_lines_removed: 0 };
    const html = renderFileDiff(emptyDiff, null, (_symbolKey, path) => `opensymphony://code/file/${path}`);
    expect(html).toContain("data-testid=\"diff-header\"");
    expect(html).toContain("data-diff-file-graph=\"src/lib.ts\"");
    expect(html).toContain("No diff available");
  });

  it("keeps old and new line coordinates distinct", () => {
    const coordinateDiff: FileDiffPage = {
      ...diff,
      hunks: [{
        ...diff.hunks[0],
        header: "@@ -5,7 +10,7 @@",
        start_line: 5,
        lines: [
          { type: "addition", line: "new symbol" },
          ...Array.from({ length: 5 }, () => ({ type: "context" as const, line: "context" })),
          { type: "deletion", line: "old symbol" },
        ],
      }],
    };
    const coordinateOutline: CodeFileOutline = {
      ...outline,
      symbols: [
        { ...outline.symbols[0], symbol_key: "added", name: "added", span: { ...outline.symbols[0].span, start_line: 10, end_line: 10 } },
        { ...outline.symbols[1], symbol_key: "removed", name: "removed", span: { ...outline.symbols[1].span, start_line: 16, end_line: 16 } },
      ],
    };
    const root = document.createElement("div");
    root.innerHTML = renderFileDiff(coordinateDiff, coordinateOutline, (symbolKey) => `opensymphony://code/${symbolKey}`);
    expect(root.querySelectorAll("[data-diff-symbol-action]")).toHaveLength(2);
    expect(root.querySelector("[data-line-type='addition']")?.getAttribute("data-diff-symbol-key")).toBe("added");
    expect(root.querySelector("[data-line-type='deletion']")?.getAttribute("data-diff-symbol-key")).toBe("removed");
  });

  it("lists blast-radius-only symbols in the accessible delta", () => {
    const overlay = {
      schema_version: { major: 1, minor: 0, patch: 0 },
      repo_id: "repo",
      base_revision: "base",
      head_revision: "head",
      added_symbols: [],
      removed_symbols: [],
      modified_symbols: [],
      edge_deltas: [],
      module_connection_deltas: [],
      blast_radius: [{ symbol_key: "caller", inbound_count: 2, outbound_count: 0, inbound: [], outbound: [] }],
      unanalyzed_files: [],
      truncation: { nodes_dropped: 0, edges_dropped: 0, reason: null },
      generated_at: "2026-07-12T00:00:00Z",
    } satisfies CodeDiffOverlay;
    expect(renderCodeDiffSummary(overlay)).toContain("data-run-code-summary");
    expect(renderCodeDiffDeltaList(overlay)).toContain("caller");
  });

  it("renders legacy overlays with missing topology arrays", () => {
    const legacyOverlay = {
      schema_version: { major: 1, minor: 0, patch: 0 },
      repo_id: "repo",
      base_revision: "base",
      head_revision: "head",
      added_symbols: [],
      removed_symbols: [],
      modified_symbols: [],
      blast_radius: [{ symbol_key: "legacyCaller", inbound_count: 1, outbound_count: 1 }],
      unanalyzed_files: [],
      truncation: { nodes_dropped: 0, edges_dropped: 0, reason: null },
      generated_at: "2026-07-12T00:00:00Z",
    } as unknown as CodeDiffOverlay;
    expect(renderCodeDiffSummary(legacyOverlay)).toContain("0 topology edges");
    expect(renderCodeDiffDeltaList(legacyOverlay)).toContain("legacyCaller");
  });
});
