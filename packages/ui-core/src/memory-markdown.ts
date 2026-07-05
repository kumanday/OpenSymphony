import { escapeHtml } from "./html.js";

/**
 * Minimal, dependency-free markdown renderer for memory capsule bodies.
 *
 * Capsules are trusted-source but round-trip through external systems
 * (Linear narratives, PR comments), so everything is HTML-escaped first and
 * only an allowlisted set of constructs is re-introduced:
 *
 * - `#`–`###` headings (rendered as h4/h5/h6 so they nest inside the inspector)
 * - `-`/`*` unordered lists
 * - paragraphs split on blank lines
 * - `**bold**`, `` `code` `` spans
 * - `[label](https://…)` external links (http/https only; anything else
 *   renders as plain text)
 * - `[[target]]` wiki links, rendered as graph-navigation buttons carrying
 *   `data-kg-link-target` so the shell can resolve them to nodes
 */
export function renderMemoryMarkdown(markdown: string): string {
  const blocks: string[] = [];
  let paragraph: string[] = [];
  let list: string[] = [];

  const flushParagraph = () => {
    if (paragraph.length === 0) return;
    blocks.push(`<p>${renderInline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };
  const flushList = () => {
    if (list.length === 0) return;
    blocks.push(`<ul>${list.map((item) => `<li>${renderInline(item)}</li>`).join("")}</ul>`);
    list = [];
  };

  for (const rawLine of markdown.split(/\r?\n/)) {
    const line = rawLine.trimEnd();
    const heading = /^(#{1,3})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      flushList();
      const level = Math.min(6, heading[1].length + 3);
      blocks.push(`<h${level}>${renderInline(heading[2])}</h${level}>`);
      continue;
    }
    const listItem = /^[-*]\s+(.*)$/.exec(line);
    if (listItem) {
      flushParagraph();
      list.push(listItem[1]);
      continue;
    }
    if (line.trim().length === 0) {
      flushParagraph();
      flushList();
      continue;
    }
    flushList();
    paragraph.push(line.trim());
  }
  flushParagraph();
  flushList();
  return blocks.join("");
}

function renderInline(text: string): string {
  // The whole line is escaped up front (escapeHtml also covers quotes), so
  // captured fragments are already safe in both text and attribute position;
  // escaping again here would double-encode ampersands.
  let html = escapeHtml(text);
  // Wiki links first: their targets may contain characters the other
  // patterns would otherwise chew on.
  html = html.replace(/\[\[([^\]]+)\]\]/g, (_match, target: string) => {
    const label = target.split("/").at(-1) ?? target;
    return `<button type="button" class="os-kg-capsule-link" data-kg-link-target="${target}">${label}</button>`;
  });
  html = html.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (match, label: string, href: string) => {
    if (!/^https?:\/\//.test(href)) return match;
    return `<a href="${href}" target="_blank" rel="noreferrer">${label}</a>`;
  });
  html = html.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  return html;
}
