import type { StreamCursor } from "./cursor.js";
import type { SchemaVersion } from "./version.js";

export type MemoryGraphVisibility = "public" | "private";
export type MemoryGraphFreshness = "current" | "stale" | "unknown";

export type MemoryGraphNodeKind =
  | "bundle"
  | "directory"
  | "concept"
  | "tag"
  | "resource"
  | "citation"
  | "source_ref"
  | "community";

export type MemoryGraphEdgeKind =
  | "contains"
  | "markdown_link"
  | "external_link"
  | "cites"
  | "tagged_with"
  | "describes_resource"
  | "scoped_to"
  | "source_supported_by"
  | "same_resource";

export interface MemoryBundleList {
  schema_version: SchemaVersion;
  bundles: MemoryBundleSummary[];
}

export interface MemoryBundleSummary {
  id: string;
  title: string;
  okf_version: string;
  visibility: MemoryGraphVisibility;
  concept_count: number;
  updated_at?: string;
}

export interface MemoryGraphSnapshot {
  schema_version: SchemaVersion;
  bundle_id: string;
  cursor: StreamCursor;
  nodes: MemoryGraphNode[];
  edges: MemoryGraphEdge[];
  communities: MemoryGraphCommunity[];
  metrics?: MemoryGraphSnapshotMetrics;
  filters_applied: string[];
  generated_at: string;
}

export interface MemoryConceptDetail {
  schema_version: SchemaVersion;
  bundle_id: string;
  concept_id: string;
  frontmatter_view: MemoryFrontmatterView;
  body_markdown: string;
  links: MemoryGraphLink[];
  citations: MemoryGraphCitation[];
  source_refs: MemoryGraphSourceRef[];
}

export interface MemoryCommunityList {
  schema_version: SchemaVersion;
  bundle_id: string;
  communities: MemoryGraphCommunity[];
  generated_at: string;
}

export interface MemorySearchResponse {
  schema_version: SchemaVersion;
  query: string;
  bundle_id?: string;
  results: MemorySearchResult[];
}

export interface MemorySearchResult {
  bundle_id: string;
  concept_id: string;
  title: string;
  visibility: MemoryGraphVisibility;
  snippet: string;
  areas: string[];
}

/**
 * One page of completed tasks for the task graph's Completed pane.
 * Primary source is the memory catalog (DuckDB issue capsules with PR
 * evidence); orchestrator-known completed issues not yet captured are
 * merged in with `source: "orchestrator"`.
 */
export interface MemoryCompletedTaskPage {
  schema_version: SchemaVersion;
  bundle_id: string;
  tasks: MemoryCompletedTask[];
  /** Total row count after filtering, before pagination. */
  total: number;
  offset: number;
  limit: number;
  sort: string;
  query?: string;
  generated_at: string;
}

export interface MemoryCompletedTask {
  issue_key: string;
  /** OKF concept id (e.g. `issues/COE-123`); empty for orchestrator rows. */
  concept_id: string;
  bundle_id?: string;
  title: string;
  state?: string;
  milestone?: string;
  url?: string;
  completed_at?: string;
  prs: MemoryTaskPullRequest[];
  source: MemoryCompletedTaskSource;
}

export interface MemoryTaskPullRequest {
  number: number;
  title: string;
  url?: string;
  merged: boolean;
  merged_at?: string;
}

export type MemoryCompletedTaskSource = "memory" | "orchestrator";

export interface MemoryGraphNode {
  id: string;
  kind: MemoryGraphNodeKind;
  label: string;
  bundle_id?: string;
  concept_id?: string;
  concept_type?: string;
  description?: string;
  path_display?: string;
  resource?: string;
  tags: string[];
  timestamp?: string;
  visibility?: MemoryGraphVisibility;
  freshness?: MemoryGraphFreshness;
  warning_count: number;
  frontmatter_summary: Record<string, unknown>;
  unknown_frontmatter: Record<string, unknown>;
  body_preview?: string;
  metrics: MemoryGraphNodeMetrics;
}

export interface MemoryGraphEdge {
  id: string;
  kind: MemoryGraphEdgeKind;
  source_id: string;
  target_id: string;
  label?: string;
  unresolved: boolean;
  metadata: Record<string, unknown>;
}

export interface MemoryGraphCommunity {
  id: string;
  label: string;
  node_ids: string[];
  concept_count: number;
}

export interface MemoryGraphSnapshotMetrics {
  orphan_count: number;
  broken_link_count: number;
  stale_concept_count: number;
  warning_count: number;
}

export interface MemoryFrontmatterView {
  primary: Record<string, unknown>;
  opensymphony: Record<string, unknown>;
  unknown: Record<string, unknown>;
}

export interface MemoryGraphLink {
  target: string;
  label?: string;
}

export interface MemoryGraphCitation {
  id: string;
  target: string;
  label?: string;
}

export interface MemoryGraphSourceRef {
  kind: string;
  id: string;
  url?: string;
}

export interface MemoryGraphNodeMetrics {
  degree?: number;
  indegree: number;
  outdegree: number;
  centrality?: number;
  bridge_score?: number;
  pagerank?: number;
  community_id?: string;
}

export interface MemoryGraphUpdatedEvent {
  schema_version: SchemaVersion;
  bundle_id: string;
  cursor: StreamCursor;
  updated_at: string;
}
