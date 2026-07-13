import type {
  ChangedFileEntry,
  CodeDiffOverlay,
  CodeFileOutline,
  CodeOutlineSymbol,
  FileDiffPage,
} from "@opensymphony/gateway-schema";
import { normalizeCodeDiffOverlay } from "@opensymphony/graph";
import { escapeHtml, escapeAttr } from "./html.js";

export interface DiffSymbolRegion {
  symbol: CodeOutlineSymbol;
  firstChangedLine: number;
  firstChangedLineKey: string;
  changedLines: number[];
  changedLineKeys: string[];
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
  codeDeepLink?: (symbolKey: string, path?: string) => string | null,
): string {
  const fileDeepLink = codeDeepLink?.("", diff.file_path) ?? null;
  const header = `<div class="os-diff-header" data-testid="diff-header">
    <span class="os-diff-path">${escapeHtml(diff.file_path)}</span>
    <span class="os-diff-stats">${renderLineStats(diff.total_lines_added, diff.total_lines_removed)}</span>
    ${fileDeepLink ? `<button type="button" class="os-diff-file-graph" data-diff-file-graph="${escapeAttr(diff.file_path)}" aria-label="Open ${escapeAttr(diff.file_path)} in Code Graph">Open file in Code Graph</button>` : ""}
  </div>`;
  if (diff.hunks.length === 0) {
    return `<div class="os-file-diff os-empty" data-testid="file-diff" data-file-path="${escapeAttr(diff.file_path)}">${header}<div>No diff available</div></div>`;
  }
  const regions = outline ? resolveDiffSymbolRegions(diff, outline) : [];
  const symbolLines = new Map<string, DiffSymbolRegion>();
  for (const region of regions) {
    for (const lineKey of region.changedLineKeys) symbolLines.set(lineKey, region);
  }
  const hunks = diff.hunks
    .map((hunk) => {
      let [oldLine, newLine] = hunkLineStarts(hunk);
      const lines = hunk.lines
        .map((line) => {
          const lineNumber = line.type === "deletion" ? oldLine : newLine;
          const lineKey = diffLineKey(line.type, lineNumber);
          const region = symbolLines.get(lineKey);
          const glyph = region && region.firstChangedLineKey === lineKey
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
        const symbol = innermostSymbolAtLine(outline.symbols, lineNumber)
          ?? (line.type === "deletion" ? nearestSymbolAtLine(outline.symbols, lineNumber) : null);
        if (symbol) {
          const lineKey = diffLineKey(line.type, renderedLine);
          const region = regions.get(symbol.symbol_key) ?? {
            symbol,
            firstChangedLine: renderedLine,
            firstChangedLineKey: lineKey,
            changedLines: [],
            changedLineKeys: [],
          };
          if (renderedLine < region.firstChangedLine
            || (renderedLine === region.firstChangedLine && lineKey < region.firstChangedLineKey)) {
            region.firstChangedLine = renderedLine;
            region.firstChangedLineKey = lineKey;
          }
          if (!region.changedLines.includes(renderedLine)) region.changedLines.push(renderedLine);
          if (!region.changedLineKeys.includes(lineKey)) region.changedLineKeys.push(lineKey);
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

function diffLineKey(type: FileDiffPage["hunks"][number]["lines"][number]["type"], line: number): string {
  return `${type}:${line}`;
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

function nearestSymbolAtLine(
  symbols: readonly CodeOutlineSymbol[],
  line: number,
): CodeOutlineSymbol | null {
  return [...symbols]
    .sort((a, b) => Math.min(Math.abs(a.span.start_line - line), Math.abs(a.span.end_line - line))
      - Math.min(Math.abs(b.span.start_line - line), Math.abs(b.span.end_line - line))
      || a.span.start_line - b.span.start_line
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
  overlay = normalizeCodeDiffOverlay(overlay);
 const counts = [
   `${overlay.added_symbols.length} added`,
   `${overlay.removed_symbols.length} removed`,
   `${overlay.modified_symbols.length} modified`,
    `${overlay.edge_deltas.length} topology edges`,
    `${overlay.module_connection_deltas.length} module connections`,
   `${overlay.blast_radius.length} blast radius`,
 ];
  return `<button type="button" class="os-run-code-summary" data-testid="run-code-summary" data-run-code-summary aria-label="Open code diff summary in Code Graph">${counts.map((value) => `<span>${escapeHtml(value)}</span>`).join(" · ")}</button>`;
}

export function renderCodeDiffDeltaList(overlay: CodeDiffOverlay | null | undefined): string {
  if (!overlay) return "";
  overlay = normalizeCodeDiffOverlay(overlay);
  const symbols = [...overlay.added_symbols, ...overlay.removed_symbols, ...overlay.modified_symbols];
  const changedKeys = new Set(symbols.map((symbol) => symbol.symbol_key));
  const rows = symbols.map((symbol) => {
    const side = symbol.after ?? symbol.before;
    const radius = overlay.blast_radius.find((entry) => entry.symbol_key === symbol.symbol_key);
    return `<li><span data-code-delta-status="${escapeAttr(symbol.status)}">${escapeHtml(symbol.status)}</span> <strong>${escapeHtml(side?.name ?? symbol.symbol_key)}</strong> <code>${escapeHtml(side?.path_display ?? "unknown path")}</code>${radius ? ` <span>(${radius.inbound_count} inbound)</span>` : ""}</li>`;
  }).join("");
  const radiusOnlyRows = overlay.blast_radius
    .filter((entry) => !changedKeys.has(entry.symbol_key))
    .map((entry) => `<li><span data-code-delta-status="blast-radius">blast radius</span> <strong>${escapeHtml(entry.symbol_key)}</strong> <code>${escapeHtml(entry.inbound[0]?.path ?? "unknown path")}</code> <span>(${entry.inbound_count} inbound, ${entry.outbound_count} outbound)</span></li>`)
    .join("");
  const radiusDetailRows = overlay.blast_radius.flatMap((entry) => [
    ...entry.inbound.map((relationship) => `<li data-testid="code-blast-radius-entry"><span data-code-delta-status="blast-radius">inbound</span> <strong>${escapeHtml(entry.symbol_key)}</strong> ← <code>${escapeHtml(relationship.symbol_key ?? "unresolved")}</code> <code>${escapeHtml(relationship.path)}</code> <span>${escapeHtml(relationship.edge_kind)} · ${escapeHtml(relationship.confidence)} · distance ${relationship.distance}</span></li>`),
    ...entry.outbound.map((relationship) => `<li data-testid="code-blast-radius-entry"><span data-code-delta-status="blast-radius">outbound</span> <strong>${escapeHtml(entry.symbol_key)}</strong> → <code>${escapeHtml(relationship.symbol_key ?? "unresolved")}</code> <code>${escapeHtml(relationship.path)}</code> <span>${escapeHtml(relationship.edge_kind)} · ${escapeHtml(relationship.confidence)} · distance ${relationship.distance}</span></li>`),
  ]).join("");
  const edgeRows = overlay.edge_deltas.map((delta) => {
    const side = delta.after ?? delta.before;
    const target = side?.target_symbol_key ?? side?.target_hint ?? "unresolved";
    const confidence = side?.confidence ?? "unknown";
    return `<li data-testid="code-edge-delta"><span data-code-delta-status="${escapeAttr(delta.status)}">${escapeHtml(delta.status)}</span> <strong>${escapeHtml(side?.kind ?? "edge")}</strong> <code>${escapeHtml(target)}</code> <span>confidence: ${escapeHtml(confidence)}${side?.unresolved ? " · unresolved" : ""}</span></li>`;
  }).join("");
  const connectionRows = overlay.module_connection_deltas.map((delta) => {
    const side = delta.after ?? delta.before;
    return `<li data-testid="code-module-connection-delta"><span data-code-delta-status="${escapeAttr(delta.status)}">${escapeHtml(delta.status)}</span> <strong>${escapeHtml(delta.scope)}</strong> <code>${escapeHtml(delta.source)} → ${escapeHtml(delta.target)}</code> <span>(${side?.edge_count ?? 0} edges)</span></li>`;
  }).join("");
  const files = overlay.unanalyzed_files.map((path) => `<li><span>unanalyzed</span> <code>${escapeHtml(path)}</code></li>`).join("");
  const details = `${rows}${radiusOnlyRows}${radiusDetailRows}${edgeRows}${connectionRows}${files}`;
  return `<details class="os-run-code-delta-list" data-testid="run-code-delta-list"><summary>Code delta details</summary><ul>${details || "<li>No analyzed symbol changes</li>"}</ul></details>`;
}

function renderLineStats(added: number, removed: number): string {
  return `<span class="os-lines-added">+${added}</span> <span class="os-lines-removed">-${removed}</span>`;
}
