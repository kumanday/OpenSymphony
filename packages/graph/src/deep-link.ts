import type { MemoryGraphNode, MemoryGraphSnapshot } from "@opensymphony/gateway-schema";
import {
  createInitialCodeGraphFilters,
  normalizeCodeGraphFilters,
  type CodeGraphFilters,
  type CodeGraphMode,
} from "./code-graph.js";
import { createInitialGraphFilters, type GraphDeepLinkState } from "./index.js";

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
 * Every path segment is percent-encoded. Concept ids keep their internal
 * slashes (`issues/COE-399` becomes `concepts/issues/COE-399`), matching the
 * gateway's wildcard concept route; bundle and community ids encode as a
 * single segment (a hosted bundle id like `team/a` or a directory-derived
 * community like `directory:path/to` must round-trip through their
 * one-segment position). Parsing is strict: unknown collections, empty
 * segments, or query/fragment suffixes are rejected rather than guessed at,
 * so a link either resolves exactly or not at all.
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
  const base = `${memoryDeepLinkPrefix}${encodeURIComponent(link.bundleId)}`;
  if (link.conceptId) {
    return `${base}/concepts/${encodeSegments(link.conceptId)}`;
  }
  if (link.communityId) {
    return `${base}/communities/${encodeURIComponent(link.communityId)}`;
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
    // Server snapshots prefix community graph nodes ("community:area:x")
    // while the community list carries the bare id ("area:x"); links always
    // address the bare community id.
    return formatMemoryDeepLink({ bundleId, communityId: bareCommunityId(node.id) });
  }
  if (node.kind === "bundle") {
    return formatMemoryDeepLink({ bundleId });
  }
  return null;
}

/** Strip the graph-node prefix from a community node id, if present. */
export function bareCommunityId(nodeId: string): string {
  return nodeId.startsWith("community:") ? nodeId.slice("community:".length) : nodeId;
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
    return snapshot.nodes.find((node) => node.id === link.communityId)
      ?? snapshot.nodes.find((node) => node.id === `community:${link.communityId}`)
      ?? null;
  }
  return snapshot.nodes.find((node) => node.kind === "bundle" && node.bundle_id === link.bundleId) ?? null;
}

/**
 * Translate a parsed deep link into the partial graph state applied through
 * the HISTORY_RESTORED reducer action. Node resolution happens against the
 * loaded snapshot (see resolveMemoryDeepLinkNode); this only carries the
 * navigation shape. Filters are always included: visibility is driven by
 * `filters.communities` (mode alone filters nothing), so a community link
 * must install its filter and every other link must clear a stale one.
 */
export function memoryDeepLinkToGraphState(link: MemoryDeepLink): Partial<GraphDeepLinkState> {
  return {
    mode: link.communityId ? "community" : "atlas",
    bundleId: link.bundleId,
    focusedNodeId: null,
    selectedNodeIds: [],
    searchQuery: "",
    filters: {
      ...createInitialGraphFilters(),
      communities: link.communityId ? [link.communityId] : [],
    },
  };
}

function encodeSegments(value: string): string {
  return value.split("/").map((segment) => encodeURIComponent(segment)).join("/");
}

export const codeDeepLinkPrefix = "opensymphony://code/";

const codeBootQueryKeys = new Set(["mode", "depth", "run_id", "filters", "seed"]);
const appBootQueryKeys = new Set(["code", "fixtures", "memory"]);

/** Read an encoded or raw Code Graph link from an app location query. */
export function codeDeepLinkFromLocationSearch(search: string): string | null {
  const query = search.startsWith("?") ? search.slice(1) : search;
  const marker = /(?:^|&)code=/.exec(query);
  if (!marker || marker.index === undefined) return null;
  const rawValue = query.slice(marker.index + marker[0].length);
  if (!rawValue) return null;
  const [first, ...rest] = rawValue.split("&");
  if (!first) return null;
  let candidate = first;
  if (first.startsWith(codeDeepLinkPrefix)) {
    for (const pair of rest) {
      const separator = pair.indexOf("=");
      const key = separator === -1 ? pair : pair.slice(0, separator);
      if (codeBootQueryKeys.has(key)) {
        candidate += `&${pair}`;
      } else if (appBootQueryKeys.has(key)) {
        break;
      } else {
        return null;
      }
    }
  }
  try {
    const link = first.startsWith(codeDeepLinkPrefix) ? candidate : decodeURIComponent(candidate);
    return link.startsWith(codeDeepLinkPrefix) ? link : null;
  } catch {
    return null;
  }
}

export interface CodeDeepLink {
  repoId: string;
  mode: CodeGraphMode;
  symbolKey: string | null;
  path: string | null;
  runId: string | null;
  depth: number;
  filters: CodeGraphFilters;
  layoutSeed: string | null;
  baseRevision: string | null;
  headRevision: string | null;
}

export function formatCodeDeepLink(link: {
  repoId: string;
  mode?: CodeGraphMode;
  symbolKey?: string | null;
  path?: string | null;
  runId?: string | null;
  depth?: number;
  filters?: CodeGraphFilters;
  layoutSeed?: string | null;
  baseRevision?: string | null;
  headRevision?: string | null;
}): string {
  if (!link.repoId) throw new Error("Code deep links require a repo id");
  const symbolKey = link.symbolKey ?? null;
  const path = link.path ?? null;
  const baseRevision = link.baseRevision ?? null;
  const headRevision = link.headRevision ?? null;
  if (symbolKey && path) throw new Error("Code deep links cannot address a symbol and file together");
  if ((baseRevision === null) !== (headRevision === null)) {
    throw new Error("Code diff deep links require both base and head revisions");
  }
  const inferredMode: CodeGraphMode = baseRevision
    ? "diff"
    : symbolKey
      ? "neighborhood"
      : path
        ? "file"
        : "atlas";
  const mode = link.mode ?? inferredMode;
  if (!codeGraphModesForPath(mode, symbolKey, path, baseRevision)) {
    throw new Error("Code deep-link target does not match its mode");
  }
  const depth = normalizeCodeDepth(link.depth ?? 1);
  const route = symbolKey
    ? `symbols/${encodeSegments(symbolKey)}`
    : path
      ? `files/${encodeSegments(path)}`
      : baseRevision
        ? `diff/${encodeURIComponent(baseRevision)}/${encodeURIComponent(headRevision!)}`
        : "atlas";
  const params = new URLSearchParams();
  if (mode !== inferredMode) params.set("mode", mode);
  if (depth !== 1) params.set("depth", String(depth));
  if (link.runId) params.set("run_id", link.runId);
  const filters = normalizeCodeGraphFilters(link.filters ?? createInitialCodeGraphFilters());
  if (JSON.stringify(filters) !== JSON.stringify(createInitialCodeGraphFilters())) {
    params.set("filters", JSON.stringify(filters));
  }
  if (link.layoutSeed) params.set("seed", link.layoutSeed);
  const query = params.toString();
  return `${codeDeepLinkPrefix}${encodeURIComponent(link.repoId)}/${route}${query ? `?${query}` : ""}`;
}

export function parseCodeDeepLink(url: string): CodeDeepLink | null {
  if (!url.startsWith(codeDeepLinkPrefix)) return null;
  const rest = url.slice(codeDeepLinkPrefix.length);
  if (!rest || rest.includes("#")) return null;
  const [rawPath, rawQuery = ""] = rest.split("?");
  if (rest.split("?").length > 2 || !rawPath) return null;
  const rawSegments = rawPath.split("/");
  if (rawSegments.some((segment) => segment.length === 0)) return null;
  const segments = rawSegments.map(decodeSegment);
  if (segments.some((segment) => segment === null || segment.length === 0)) return null;
  const [repoId, collection, ...tail] = segments as string[];
  let symbolKey: string | null = null;
  let path: string | null = null;
  let baseRevision: string | null = null;
  let headRevision: string | null = null;
  let inferredMode: CodeGraphMode;
  if (!collection) {
    inferredMode = "atlas";
  } else if (collection === "atlas" && tail.length === 0) {
    inferredMode = "atlas";
  } else if (collection === "symbols" && tail.length > 0) {
    symbolKey = tail.join("/");
    inferredMode = "neighborhood";
  } else if (collection === "files" && tail.length > 0) {
    path = tail.join("/");
    inferredMode = "file";
  } else if (collection === "diff" && tail.length === 2) {
    [baseRevision, headRevision] = tail;
    inferredMode = "diff";
  } else {
    return null;
  }
  const query = parseCodeQuery(rawQuery);
  if (!query) return null;
  const mode = query.mode ?? inferredMode;
  if (!codeGraphModesForPath(mode, symbolKey, path, baseRevision)) return null;
  return {
    repoId,
    mode,
    symbolKey,
    path,
    runId: query.runId,
    depth: query.depth,
    filters: query.filters,
    layoutSeed: query.layoutSeed,
    baseRevision,
    headRevision,
  };
}

export function codeDeepLinkToGraphState(link: CodeDeepLink): {
  repoId: string | null;
  mode: CodeGraphMode;
  symbolKey: string | null;
  path: string | null;
  runId: string | null;
  depth: number;
  filters: CodeGraphFilters;
  selectedNodeIds: string[];
  layoutSeed: string | null;
  baseRevision: string | null;
  headRevision: string | null;
} {
  return {
    repoId: link.repoId,
    mode: link.mode,
    symbolKey: link.symbolKey,
    path: link.path,
    runId: link.runId,
    depth: link.depth,
    filters: normalizeCodeGraphFilters(link.filters),
    selectedNodeIds: link.symbolKey ? [`symbol:${link.symbolKey}`, link.symbolKey] : [],
    layoutSeed: link.layoutSeed,
    baseRevision: link.baseRevision,
    headRevision: link.headRevision,
  };
}

export function codeDeepLinkForSymbol(repoId: string, symbolKey: string, options: Omit<Parameters<typeof formatCodeDeepLink>[0], "repoId" | "symbolKey"> = {}): string {
  return formatCodeDeepLink({ ...options, repoId, symbolKey });
}

export function codeDeepLinkForFile(repoId: string, path: string, options: Omit<Parameters<typeof formatCodeDeepLink>[0], "repoId" | "path"> = {}): string {
  return formatCodeDeepLink({ ...options, repoId, path });
}

function parseCodeQuery(rawQuery: string): {
  mode?: CodeGraphMode;
  depth: number;
  runId: string | null;
  filters: CodeGraphFilters;
  layoutSeed: string | null;
} | null {
  try {
    decodeURIComponent(rawQuery.replaceAll("+", "%20"));
  } catch {
    return null;
  }
  const values = new URLSearchParams(rawQuery);
  const allowed = new Set(["mode", "depth", "run_id", "filters", "seed"]);
  const seen = new Set<string>();
  for (const [key] of values) {
    if (!allowed.has(key) || seen.has(key)) return null;
    seen.add(key);
  }
  const rawMode = values.get("mode");
  const mode = rawMode === null ? undefined : isCodeGraphMode(rawMode) ? rawMode : null;
  if (mode === null) return null;
  const rawDepth = values.get("depth");
  const depth = rawDepth === null ? 1 : Number(rawDepth);
  if (!Number.isInteger(depth) || depth < 0 || depth > 2) return null;
  const runId = values.get("run_id");
  if (runId === "") return null;
  const layoutSeed = values.get("seed");
  if (layoutSeed === "") return null;
  const rawFilters = values.get("filters");
  const filters = rawFilters === null ? createInitialCodeGraphFilters() : parseCodeFilters(rawFilters);
  if (!filters) return null;
  return { mode, depth, runId, filters, layoutSeed, };
}

function parseCodeFilters(raw: string): CodeGraphFilters | null {
  const enumValues: Record<string, ReadonlySet<string>> = {
    confidences: new Set(["exact", "syntactic", "heuristic"]),
    freshness: new Set(["current", "stale", "unknown"]),
    deltaStatuses: new Set(["added", "removed", "modified", "unchanged"]),
  };
  try {
    const value = JSON.parse(raw) as Record<string, unknown>;
    const expected = Object.keys(createInitialCodeGraphFilters());
    if (!value || typeof value !== "object" || Object.keys(value).some((key) => !expected.includes(key))) return null;
    const filters = createInitialCodeGraphFilters();
    for (const key of expected) {
      if (key === "diagnostics") {
        const diagnostics = value[key];
        if (diagnostics !== "all" && diagnostics !== "with_diagnostics" && diagnostics !== "without_diagnostics") return null;
        filters.diagnostics = diagnostics;
      } else {
        const items = value[key];
        if (!Array.isArray(items) || items.some((item) => typeof item !== "string")) return null;
        const allowedValues = enumValues[key];
        if (allowedValues && items.some((item) => !allowedValues.has(item as string))) return null;
        (filters[key as keyof CodeGraphFilters] as string[]).push(...items);
      }
    }
    return normalizeCodeGraphFilters(filters);
  } catch {
    return null;
  }
}

function codeGraphModesForPath(
  mode: CodeGraphMode,
  symbolKey: string | null,
  path: string | null,
  baseRevision: string | null,
): boolean {
  if (mode === "diff" && baseRevision === null) return false;
  if (baseRevision) return mode === "diff" && symbolKey === null && path === null;
  if (symbolKey) return mode === "neighborhood" || mode === "diff";
  if (path) return mode === "file" || mode === "diff";
  return mode === "atlas";
}

function isCodeGraphMode(value: string): value is CodeGraphMode {
  return value === "atlas" || value === "file" || value === "neighborhood" || value === "diff";
}

function normalizeCodeDepth(value: number): number {
  if (!Number.isInteger(value) || value < 0 || value > 2) throw new Error("Code deep-link depth must be an integer from 0 to 2");
  return value;
}

function decodeSegment(value: string): string | null {
  try {
    return decodeURIComponent(value);
  } catch {
    return null;
  }
}
