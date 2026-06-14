/**
 * Deterministic, ASCII-safe string hash used for stable idempotency keys.
 *
 * Uses SHA-256 (256-bit) via the Web Crypto API, with a Node fallback, so
 * collision risk is negligible for multi-user idempotency keys.
 */
export async function stableHash(input: string): Promise<string> {
  const subtle = await getSubtleCrypto();
  const data = new TextEncoder().encode(input);
  const digest = await subtle.digest("SHA-256", data);
  const bytes = new Uint8Array(digest);
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

async function getSubtleCrypto(): Promise<SubtleCrypto> {
  const globalCrypto = (globalThis as any).crypto;
  if (globalCrypto?.subtle) {
    return globalCrypto.subtle as SubtleCrypto;
  }
  // Node fallback; only evaluated when WebCrypto is not globally present.
  const { webcrypto } = await import("crypto");
  return webcrypto.subtle as SubtleCrypto;
}

/**
 * Deterministic hash of a JSON-serializable value, with stable key ordering.
 *
 * Used for idempotency keys where the payload is an object and key insertion
 * order may vary between callers (e.g. form data converted to a follow-up
 * payload).
 */
export async function stableHashJson(value: unknown): Promise<string> {
  return stableHash(stableStringify(value));
}

function stableStringify(value: unknown): string {
  if (value === null || typeof value !== "object") {
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) {
    return `[${value.map(stableStringify).join(",")}]`;
  }
  const keys = Object.keys(value as Record<string, unknown>).sort();
  const entries = keys.map((k) => `${stableStringify(k)}:${stableStringify((value as Record<string, unknown>)[k])}`);
  return `{${entries.join(",")}}`;
}
