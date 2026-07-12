import type { StreamCursor } from "./cursor.js";
import type { SchemaVersion } from "./version.js";

export type CodeGraphFreshness = "current" | "stale" | "unknown";
export type CodeGraphConfidence = "exact" | "syntactic" | "heuristic";
export type CodeGraphMode = "atlas" | "file" | "neighborhood";
export type CodeGraphAggregate = "directory" | "community";
export type CodeGraphNodeKind = "directory" | "file" | "symbol" | "community";

export interface CodeSpan {
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
}

export interface CodeRepoList {
  schema_version: SchemaVersion;
  repos: CodeRepoSummary[];
}

export interface CodeRepoSummary {
  repo_id: string;
  display_root: string;
  languages: string[];
  document_count: number;
  symbol_count: number;
  edge_count: number;
  freshness: CodeGraphFreshness;
  indexed_at?: string | null;
  head_revision?: string | null;
  worktree_dirty?: boolean;
}

export interface CodeGraphSnapshot {
  schema_version: SchemaVersion;
  repo_id: string;
  mode: CodeGraphMode;
  cursor: StreamCursor;
  nodes: CodeGraphNode[];
  edges: CodeGraphEdge[];
  communities: CodeGraphCommunity[];
  truncation: CodeGraphTruncation;
  filters_applied: string[];
  generated_at: string;
}

export interface CodeGraphNode {
  id: string;
  kind: CodeGraphNodeKind;
  label: string;
  symbol_kind?: string | null;
  symbol_key?: string | null;
  symbol_id?: string | null;
  path_display?: string | null;
  language?: string | null;
  container_chain: string[];
  signature?: string | null;
  span?: CodeSpan | null;
  selection_span?: CodeSpan | null;
  freshness: CodeGraphFreshness;
  diagnostic_count: number;
  diagnostic_severity?: string | null;
  metrics: CodeGraphNodeMetrics;
}

export interface CodeGraphNodeMetrics {
  in_degree: number;
  out_degree: number;
  community_id?: string | null;
}

export interface CodeGraphEdge {
  id: string;
  kind: string;
  source_id: string;
  target_id: string;
  confidence: CodeGraphConfidence;
  unresolved: boolean;
  target_hint?: string | null;
}

export interface CodeGraphCommunity {
  id: string;
  label: string;
  node_ids: string[];
  symbol_count: number;
}

export interface CodeGraphTruncation {
  nodes_dropped: number;
  edges_dropped: number;
  reason?: string | null;
}

export interface CodeSymbolDetail {
  schema_version: SchemaVersion;
  repo_id: string;
  symbol_key: string;
  symbol_id: string;
  kind: string;
  name: string;
  path_display: string;
  language: string;
  container_chain: string[];
  signature?: string | null;
  span: CodeSpan;
  selection_span: CodeSpan;
  freshness: CodeGraphFreshness;
  provenance: CodeSymbolProvenance;
  diagnostics: CodeDiagnostic[];
  edge_summary: CodeEdgeSummary[];
  source_snippet?: CodeSourceSnippet | null;
  related_issues?: CodeGraphIssueChip[];
  related_memory_concepts?: CodeGraphMemoryChip[];
}

export interface CodeGraphIssueChip {
  issue_key: string;
  title: string;
  state?: string;
  url?: string;
  freshness: CodeGraphFreshness;
}

export interface CodeGraphMemoryChip {
  bundle_id: string;
  concept_id: string;
  title: string;
  visibility: string;
  freshness: CodeGraphFreshness;
}

export interface CodeSymbolProvenance {
  commit_sha?: string | null;
  content_sha256: string;
  snippet_sha256: string;
  parser_version: string;
  query_pack_version: string;
  indexed_at?: string | null;
}

export interface CodeDiagnostic {
  kind: string;
  severity: string;
  message: string;
  span: CodeSpan;
}

export interface CodeEdgeSummary {
  kind: string;
  confidence: CodeGraphConfidence;
  count: number;
  unresolved_count: number;
}

export interface CodeSourceSnippet {
  text: string;
  start_line: number;
  end_line: number;
  redacted: boolean;
}

export interface CodeFileOutline {
  schema_version: SchemaVersion;
  run_id: string;
  repo_id?: string | null;
  path: string;
  symbols: CodeOutlineSymbol[];
  generated_at: string;
}

export interface CodeOutlineSymbol {
  symbol_key: string;
  name: string;
  kind: string;
  path: string;
  span: CodeSpan;
  selection_span: CodeSpan;
  container_chain: string[];
}

export type CodeDiffSymbolStatus = "added" | "removed" | "modified";

export interface CodeDiffOverlay {
  schema_version: SchemaVersion;
  repo_id: string;
  base_revision: string;
  head_revision: string;
  added_symbols: CodeDiffSymbol[];
  removed_symbols: CodeDiffSymbol[];
  modified_symbols: CodeDiffSymbol[];
  blast_radius: CodeDiffBlastRadius[];
  unanalyzed_files: string[];
  truncation: CodeGraphTruncation;
  generated_at: string;
}

export interface CodeDiffSymbol {
  symbol_key: string;
  status: CodeDiffSymbolStatus;
  before?: CodeDiffSymbolSide | null;
  after?: CodeDiffSymbolSide | null;
}

export interface CodeDiffSymbolSide {
  symbol_id: string;
  kind: string;
  name: string;
  path_display: string;
  container_chain: string[];
  span: CodeSpan;
  freshness: CodeGraphFreshness;
}

export interface CodeDiffBlastRadius {
  symbol_key: string;
  inbound_count: number;
  outbound_count: number;
}

export interface CodeIndexReport {
  schema_version: SchemaVersion;
  repo_id: string;
  status: "accepted" | "progress" | "completed" | "unavailable" | "failed";
  head_revision?: string | null;
  parsed_files: number;
  persisted_documents: number;
  persisted_symbols: number;
  persisted_edges: number;
  persisted_diagnostics: number;
  stale_rows: number;
  skipped_files: string[];
  diagnostics: string[];
  cursor: StreamCursor;
  indexed_at: string;
}

export interface CodeGraphUpdatedEvent {
  schema_version: SchemaVersion;
  repo_id: string;
  head_revision?: string | null;
  cursor: StreamCursor;
  updated_at: string;
}
