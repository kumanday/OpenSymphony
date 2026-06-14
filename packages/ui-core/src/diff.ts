import type { ChangedFileEntry, FileDiffPage } from "@opensymphony/gateway-schema";
import { escapeHtml, escapeAttr } from "./html.js";

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
        <span class="os-file-stats">+${f.lines_added} -${f.lines_removed}</span>
      </button>`;
    })
    .join("");
  return `<div class="os-changed-file-list" data-testid="changed-file-list">${items}</div>`;
}

/** Render a single diff page as a lightweight HTML string. */
export function renderFileDiff(diff: FileDiffPage): string {
  if (diff.hunks.length === 0) {
    return `<div class="os-file-diff os-empty" data-testid="file-diff" data-file-path="${escapeAttr(diff.file_path)}">No diff available</div>`;
  }
  const header = `<div class="os-diff-header" data-testid="diff-header">
    <span class="os-diff-path">${escapeHtml(diff.file_path)}</span>
    <span class="os-diff-stats">+${diff.total_lines_added} -${diff.total_lines_removed}</span>
  </div>`;
  const hunks = diff.hunks
    .map((hunk) => {
      const lines = hunk.lines
        .map((line) => {
          const typeClass = `os-diff-line-${escapeAttr(line.type)}`;
          const prefix = line.type === "addition" ? "+" : line.type === "deletion" ? "-" : " ";
          return `<div class="os-diff-line ${typeClass}" data-line-type="${escapeAttr(line.type)}"><span class="os-diff-prefix">${prefix}</span>${escapeHtml(line.line)}</div>`;
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
