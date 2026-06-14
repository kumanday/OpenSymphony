/**
 * Minimal HTML escaping helpers used by lightweight DOM renderers.
 *
 * These escape both text and double-quoted attribute contexts. Single quotes
 * are also escaped so values remain safe if the surrounding HTML is later
 * wrapped in single quotes.
 */
export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** Escape a value that will be inserted into a double-quoted attribute. */
export function escapeAttr(text: string): string {
  return escapeHtml(text);
}
