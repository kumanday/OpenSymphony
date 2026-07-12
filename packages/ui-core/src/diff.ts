import type {
  ChangedFileEntry,
  CodeDiffOverlay,
  CodeFileOutline,
  CodeOutlineSymbol,
  FileDiffPage,
} from "@opensymphony/gateway-schema";
import { escapeHtml, escapeAttr } from "./html.js";

export interface DiffSymbolRegion {
  symbol: CodeOutlineSymbol;
  firstChangedLine: number;
  changedLines: number[];
}

/** UI model for a single changed file in the diff viewer. */
export interface DiffFileItem {
  path: string;
  changeKind: ChangedFileEntry["change_kind"];
  linesAdded: number;
  linesRemoved: number;
  sizeBytes?: number;
}

/** Render a changed-file list as a lightweight HTML string. */
export function renderChangedFileList(
  files: ChangedFileEntry[],
  selectedPath?: string,
): string {
  if (files.length === 0) {
    return `<div class="os-changed-file-list os-empty" data-testid="changed-file-list">No changed files</div>`;
  }
  const items = files
    .map((f) => {
      const kindClass = `os-change-kind-${escapeAttr(f.change_kind)}`;
      const selected = selectedPath === f.path ? " os-selected" : "";
      return `<button class="os-changed-file${selected}" data-path="${escapeAttr(f.path)}" data-testid="changed-file-item" data-kind="${escapeAttr(f.change_kind)}">
        <span class="os-change-kind ${kindClass}">${escapeHtml(f.change_kind)}</span>
        <span class="os-file-path">${escapeHtml(f.path)}</span>
        <span class="os-file-stats">${renderLineStats(f.lines_added, f.lines_removed)}</span>
      </button>`;
    })
    .join("");
  return `<div class="os-changed-file-list" data-testid="changed-file-list">${items}</div>`;
}

/** Render a single diff page as a lightweight HTML string. */
export function renderFileDiff(
  diff: FileDiffPage,
  outline?: CodeFileOutline | null,
  codeDeepLink?: (symbolKey: string) => string | null,
): string {
  if (diff.hunks.length === 0) {
    return `<div class="os-file-diff os-empty" data-testid="file-diff" data-file-path="${escapeAttr(diff.file_path)}">No diff available</div>`;
  }
  const regions = outline ? resolveDiffSymbolRegions(diff, outline) : [];
  const regionByLine = new Map(regions.map((region) => [region.firstChangedLine, region]));
  const symbolLines = new Map<number, DiffSymbolRegion>();
  for (const region of regions) {
    for (const line of region.changedLines) symbolLines.set(line, region);
  }
  const header = `<div class="os-diff-header" data-testid="diff-header">
    <span class="os-diff-path">${escapeHtml(diff.file_path)}</span>
    <span class="os-diff-stats">${renderLineStats(diff.total_lines_added, diff.total_lines_removed)}</span>
    <button type="button" class="os-diff-file-graph" data-diff-file-graph="${escapeAttr(diff.file_path)}" aria-label="Open ${escapeAttr(diff.file_path)} in Code Graph">Open file in Code Graph</button>
  </div>`;
  const hunks = diff.hunks
    .map((hunk) => {
      let [oldLine, newLine] = hunkLineStarts(hunk);
      const lines = hunk.lines
        .map((line) => {
          const lineNumber = line.type === "deletion" ? oldLine : newLine;
          const region = symbolLines.get(lineNumber);
          const glyph = region && regionByLine.get(lineNumber) === region
            ? renderDiffSymbolGlyph(region.symbol, codeDeepLink?.(region.symbol.symbol_key) ?? null)
            : "";
          const typeClass = `os-diff-line-${escapeAttr(line.type)}`;
          const prefix = line.type === "addition" ? "+" : line.type === "deletion" ? "-" : " ";
          const symbolAttrs = region
            ? ` data-diff-symbol-key="${escapeAttr(region.symbol.symbol_key)}" data-diff-symbol-name="${escapeAttr(region.symbol.name)}"`
            : "";
          const rendered = `<div class="os-diff-line ${typeClass}" data-line-type="${escapeAttr(line.type)}" data-line-number="${lineNumber}"${symbolAttrs}><span class="os-diff-line-number">${lineNumber}</span><span class="os-diff-prefix">${prefix}</span>${glyph}${escapeHtml(line.line)}</div>`;
          if (line.type === "deletion") oldLine += 1;
          else if (line.type === "addition") newLine += 1;
          else { oldLine += 1; newLine += 1; }
          return rendered;
        })
        .join("");
      return `<div class="os-diff-hunk" data-testid="diff-hunk">
        <div class="os-diff-hunk-header">${escapeHtml(hunk.header)}</div>
        ${lines}
      </div>`;
    })
    .join("");
  return `<div class="os-file-diff" data-testid="file-diff" data-file-path="${escapeAttr(diff.file_path)}">${header}${hunks}</div>`;
}

export function resolveDiffSymbolRegions(
  diff: FileDiffPage,
  outline: CodeFileOutline,
): DiffSymbolRegion[] {
  const regions = new Map<string, DiffSymbolRegion>();
  for (const hunk of diff.hunks) {
    let [oldLine, newLine] = hunkLineStarts(hunk);
    for (const line of hunk.lines) {
      const lineNumber = line.type === "deletion" ? Math.max(1, newLine) : newLine;
      const renderedLine = line.type === "deletion" ? oldLine : newLine;
      if (line.type !== "context") {
        const symbol = innermostSymbolAtLine(outline.symbols, lineNumber);
        if (symbol) {
          const region = regions.get(symbol.symbol_key) ?? { symbol, firstChangedLine: renderedLine, changedLines: [] };
          region.firstChangedLine = Math.min(region.firstChangedLine, renderedLine);
          if (!region.changedLines.includes(renderedLine)) region.changedLines.push(renderedLine);
          regions.set(symbol.symbol_key, region);
        }
      }
      if (line.type === "deletion") oldLine += 1;
      else if (line.type === "addition") newLine += 1;
      else { oldLine += 1; newLine += 1; }
    }
  }
  return [...regions.values()].sort((a, b) => a.firstChangedLine - b.firstChangedLine || a.symbol.symbol_key.localeCompare(b.symbol.symbol_key));
}

function hunkLineStarts(hunk: FileDiffPage["hunks"][number]): [number, number] {
  const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(hunk.header);
  return match ? [Number(match[1]), Number(match[2])] : [hunk.start_line, hunk.start_line];
}

export function innermostSymbolAtLine(
  symbols: readonly CodeOutlineSymbol[],
  line: number,
): CodeOutlineSymbol | null {
  return symbols
    .filter((symbol) => symbol.span.start_line <= line && line <= symbol.span.end_line)
    .sort((a, b) => (a.span.end_line - a.span.start_line) - (b.span.end_line - b.span.start_line)
      || b.span.start_line - a.span.start_line
      || a.symbol_key.localeCompare(b.symbol_key))[0] ?? null;
}

function renderDiffSymbolGlyph(symbol: CodeOutlineSymbol, deepLink: string | null): string {
  const label = `Open ${symbol.name} in Code Graph`;
  const copy = deepLink
    ? `<button type="button" class="os-diff-symbol-copy" data-diff-symbol-copy="${escapeAttr(deepLink)}" aria-label="Copy code deep link for ${escapeAttr(symbol.name)}" title="Copy code deep link">⧉</button>`
    : "";
  return `<button type="button" class="os-diff-symbol-glyph" data-diff-symbol-action="${escapeAttr(symbol.symbol_key)}" aria-label="${escapeAttr(label)}" title="${escapeAttr(label)}">⌘</button>${copy}`;
}

export function renderCodeDiffSummary(overlay: CodeDiffOverlay | null | undefined): string {
  if (!overlay) return "";
  const counts = [
    `${overlay.added_symbols.length} added`,
    `${overlay.removed_symbols.length} removed`,
    `${overlay.modified_symbols.length} modified`,
    `${overlay.blast_radius.length} blast radius`,
  ];
  return `<div class="os-run-code-summary" data-testid="run-code-summary" aria-label="Code diff summary">${counts.map((value) => `<span>${escapeHtml(value)}</span>`).join(" · ")}</div>`;
}

export function renderCodeDiffDeltaList(overlay: CodeDiffOverlay | null | undefined): string {
  if (!overlay) return "";
  const symbols = [...overlay.added_symbols, ...overlay.removed_symbols, ...overlay.modified_symbols];
  const rows = symbols.map((symbol) => {
    const side = symbol.after ?? symbol.before;
    const radius = overlay.blast_radius.find((entry) => entry.symbol_key === symbol.symbol_key);
    return `<li><span data-code-delta-status="${escapeAttr(symbol.status)}">${escapeHtml(symbol.status)}</span> <strong>${escapeHtml(side?.name ?? symbol.symbol_key)}</strong> <code>${escapeHtml(side?.path_display ?? "unknown path")}</code>${radius ? ` <span>(${radius.inbound_count} inbound)</span>` : ""}</li>`;
  }).join("");
  const files = overlay.unanalyzed_files.map((path) => `<li><span>unanalyzed</span> <code>${escapeHtml(path)}</code></li>`).join("");
  return `<details class="os-run-code-delta-list" data-testid="run-code-delta-list"><summary>Code delta details</summary><ul>${rows || "<li>No analyzed symbol changes</li>"}${files}</ul></details>`;
}

function renderLineStats(added: number, removed: number): string {
  return `<span class="os-lines-added">+${added}</span> <span class="os-lines-removed">-${removed}</span>`;
}
