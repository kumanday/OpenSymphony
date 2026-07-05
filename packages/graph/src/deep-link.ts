import type { MemoryGraphNode, MemoryGraphSnapshot } from "@opensymphony/gateway-schema";
import type { GraphDeepLinkState } from "./index.js";

/**
 * Memory deep links address a location in the knowledge graph — a bundle, a
 * community (area), or a concept capsule — with a stable string that can be
 * embedded outside the graph UI (task artifacts, notifications, docs) and
 * later resolved back into graph navigation state.
 *
 * Format:
 *   opensymphony://memory/<bundleId>
 *   opensymphony://memory/<bundleId>/communities/<communityId>
 *   opensymphony://memory/<bundleId>/concepts/<conceptId>
 *
 * Every path segment is percent-encoded; concept ids keep their internal
 * slashes (`issues/COE-399` becomes `concepts/issues/COE-399`), matching the
 * gateway's wildcard concept route. Parsing is strict: unknown collections,
 * empty segments, or query/fragment suffixes are rejected rather than
 * guessed at, so a link either resolves exactly or not at all.
 */

export const memoryDeepLinkPrefix = "opensymphony://memory/";

export interface MemoryDeepLink {
  bundleId: string;
  conceptId: string | null;
  communityId: string | null;
}

export function formatMemoryDeepLink(link: {
  bundleId: string;
  conceptId?: string | null;
  communityId?: string | null;
}): string {
  if (!link.bundleId) {
    throw new Error("Memory deep links require a bundle id");
  }
  const base = `${memoryDeepLinkPrefix}${encodeSegments(link.bundleId)}`;
  if (link.conceptId) {
    return `${base}/concepts/${encodeSegments(link.conceptId)}`;
  }
  if (link.communityId) {
    return `${base}/communities/${encodeSegments(link.communityId)}`;
  }
  return base;
}

export function parseMemoryDeepLink(url: string): MemoryDeepLink | null {
  if (!url.startsWith(memoryDeepLinkPrefix)) return null;
  const rest = url.slice(memoryDeepLinkPrefix.length);
  if (rest.length === 0 || /[?#]/.test(rest)) return null;
  const segments = rest.split("/");
  if (segments.some((segment) => segment.length === 0)) return null;
  const decoded: string[] = [];
  for (const segment of segments) {
    try {
      decoded.push(decodeURIComponent(segment));
    } catch {
      return null;
    }
  }
  const [bundleId, collection, ...tail] = decoded;
  if (collection === undefined) {
    return { bundleId, conceptId: null, communityId: null };
  }
  if (collection === "concepts" && tail.length > 0) {
    return { bundleId, conceptId: tail.join("/"), communityId: null };
  }
  if (collection === "communities" && tail.length === 1) {
    return { bundleId, conceptId: null, communityId: tail[0] };
  }
  return null;
}

/**
 * Deep link for a graph node, or null for node kinds that have no stable
 * address (tags, resources, citations). Concept nodes link to their capsule;
 * community nodes link to their area.
 */
export function memoryDeepLinkForGraphNode(
  bundleId: string,
  node: Pick<MemoryGraphNode, "kind" | "id" | "concept_id">,
): string | null {
  if (node.kind === "concept" && node.concept_id) {
    return formatMemoryDeepLink({ bundleId, conceptId: node.concept_id });
  }
  if (node.kind === "community") {
    return formatMemoryDeepLink({ bundleId, communityId: node.id });
  }
  if (node.kind === "bundle") {
    return formatMemoryDeepLink({ bundleId });
  }
  return null;
}

/** Find the snapshot node a parsed deep link points at. */
export function resolveMemoryDeepLinkNode(
  snapshot: MemoryGraphSnapshot,
  link: MemoryDeepLink,
): MemoryGraphNode | null {
  if (link.conceptId) {
    return snapshot.nodes.find((node) => node.concept_id === link.conceptId)
      ?? snapshot.nodes.find((node) => node.id === link.conceptId)
      ?? null;
  }
  if (link.communityId) {
    return snapshot.nodes.find((node) => node.id === link.communityId) ?? null;
  }
  return snapshot.nodes.find((node) => node.kind === "bundle" && node.bundle_id === link.bundleId) ?? null;
}

/**
 * Translate a parsed deep link into the partial graph state applied through
 * the HISTORY_RESTORED reducer action. Node resolution happens against the
 * loaded snapshot (see resolveMemoryDeepLinkNode); this only carries the
 * navigation shape.
 */
export function memoryDeepLinkToGraphState(link: MemoryDeepLink): Partial<GraphDeepLinkState> {
  return {
    mode: link.communityId ? "community" : "atlas",
    bundleId: link.bundleId,
    focusedNodeId: null,
    selectedNodeIds: [],
    searchQuery: "",
  };
}

function encodeSegments(value: string): string {
  return value.split("/").map((segment) => encodeURIComponent(segment)).join("/");
}
