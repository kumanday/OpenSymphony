use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{cursor::StreamCursor, version::SchemaVersion};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeGraphFreshness {
    Current,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeGraphConfidence {
    Exact,
    Syntactic,
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeGraphMode {
    Atlas,
    File,
    Neighborhood,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeGraphAggregate {
    Directory,
    Community,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeGraphNodeKind {
    Directory,
    File,
    Symbol,
    Community,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSpan {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeRepoList {
    pub schema_version: SchemaVersion,
    pub repos: Vec<CodeRepoSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeRepoSummary {
    pub repo_id: String,
    pub display_root: String,
    pub languages: Vec<String>,
    pub document_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
    pub freshness: CodeGraphFreshness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_revision: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub worktree_dirty: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeGraphSnapshot {
    pub schema_version: SchemaVersion,
    pub repo_id: String,
    pub mode: CodeGraphMode,
    pub cursor: StreamCursor,
    pub nodes: Vec<CodeGraphNode>,
    pub edges: Vec<CodeGraphEdge>,
    #[serde(default)]
    pub communities: Vec<CodeGraphCommunity>,
    pub truncation: CodeGraphTruncation,
    #[serde(default)]
    pub filters_applied: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeGraphNode {
    pub id: String,
    pub kind: CodeGraphNodeKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default)]
    pub container_chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<CodeSpan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_span: Option<CodeSpan>,
    pub freshness: CodeGraphFreshness,
    #[serde(default)]
    pub diagnostic_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic_severity: Option<String>,
    #[serde(default)]
    pub metrics: CodeGraphNodeMetrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphNodeMetrics {
    #[serde(default)]
    pub in_degree: usize,
    #[serde(default)]
    pub out_degree: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphEdge {
    pub id: String,
    pub kind: String,
    pub source_id: String,
    pub target_id: String,
    pub confidence: CodeGraphConfidence,
    #[serde(default)]
    pub unresolved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphCommunity {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub symbol_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphTruncation {
    #[serde(default)]
    pub nodes_dropped: usize,
    #[serde(default)]
    pub edges_dropped: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeSymbolDetail {
    pub schema_version: SchemaVersion,
    pub repo_id: String,
    pub symbol_key: String,
    pub symbol_id: String,
    pub kind: String,
    pub name: String,
    pub path_display: String,
    pub language: String,
    #[serde(default)]
    pub container_chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    pub span: CodeSpan,
    pub selection_span: CodeSpan,
    pub freshness: CodeGraphFreshness,
    pub provenance: CodeSymbolProvenance,
    #[serde(default)]
    pub diagnostics: Vec<CodeDiagnostic>,
    #[serde(default)]
    pub edge_summary: Vec<CodeEdgeSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_snippet: Option<CodeSourceSnippet>,
    #[serde(default)]
    pub related_issues: Vec<CodeGraphIssueChip>,
    #[serde(default)]
    pub related_memory_concepts: Vec<CodeGraphMemoryChip>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphIssueChip {
    pub issue_key: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub freshness: CodeGraphFreshness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeGraphMemoryChip {
    pub bundle_id: String,
    pub concept_id: String,
    pub title: String,
    pub visibility: String,
    pub freshness: CodeGraphFreshness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbolProvenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    pub content_sha256: String,
    pub snippet_sha256: String,
    pub parser_version: String,
    pub query_pack_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiagnostic {
    pub kind: String,
    pub severity: String,
    pub message: String,
    pub span: CodeSpan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeEdgeSummary {
    pub kind: String,
    pub confidence: CodeGraphConfidence,
    pub count: usize,
    pub unresolved_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSourceSnippet {
    pub text: String,
    pub start_line: usize,
    pub end_line: usize,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeFileOutline {
    pub schema_version: SchemaVersion,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    pub path: String,
    #[serde(default)]
    pub symbols: Vec<CodeOutlineSymbol>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeOutlineSymbol {
    pub symbol_key: String,
    pub name: String,
    pub kind: String,
    pub path: String,
    pub span: CodeSpan,
    pub selection_span: CodeSpan,
    #[serde(default)]
    pub container_chain: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeDiffSymbolStatus {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeDiffEdgeStatus {
    Added,
    Removed,
    Retargeted,
    ConfidenceChanged,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffEdge {
    pub edge_key: String,
    pub status: CodeDiffEdgeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<CodeDiffEdgeSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<CodeDiffEdgeSide>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffEdgeSide {
    pub edge_id: String,
    pub kind: String,
    pub source_symbol_key: Option<String>,
    pub target_symbol_key: Option<String>,
    pub target_hint: Option<String>,
    pub confidence: CodeGraphConfidence,
    pub unresolved: bool,
    pub path: String,
    pub span: CodeSpan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeDiffConnectionScope {
    Directory,
    Module,
    Community,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffModuleConnection {
    pub connection_key: String,
    pub scope: CodeDiffConnectionScope,
    pub source: String,
    pub target: String,
    pub status: CodeDiffEdgeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<CodeDiffModuleConnectionSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<CodeDiffModuleConnectionSide>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffModuleConnectionSide {
    pub edge_count: usize,
    #[serde(default)]
    pub edge_kind_counts: Vec<CodeDiffCountByKind>,
    #[serde(default)]
    pub confidence_counts: Vec<CodeDiffCountByConfidence>,
    pub unresolved_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffCountByKind {
    pub kind: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffCountByConfidence {
    pub confidence: CodeGraphConfidence,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeDiffOverlay {
    pub schema_version: SchemaVersion,
    pub repo_id: String,
    pub base_revision: String,
    pub head_revision: String,
    #[serde(default)]
    pub added_symbols: Vec<CodeDiffSymbol>,
    #[serde(default)]
    pub removed_symbols: Vec<CodeDiffSymbol>,
    #[serde(default)]
    pub modified_symbols: Vec<CodeDiffSymbol>,
    #[serde(default)]
    pub edge_deltas: Vec<CodeDiffEdge>,
    #[serde(default)]
    pub module_connection_deltas: Vec<CodeDiffModuleConnection>,
    #[serde(default)]
    pub blast_radius: Vec<CodeDiffBlastRadius>,
    #[serde(default)]
    pub unanalyzed_files: Vec<String>,
    pub truncation: CodeGraphTruncation,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeDiffSymbol {
    pub symbol_key: String,
    pub status: CodeDiffSymbolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<CodeDiffSymbolSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<CodeDiffSymbolSide>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeDiffSymbolSide {
    pub symbol_id: String,
    pub kind: String,
    pub name: String,
    pub path_display: String,
    #[serde(default)]
    pub container_chain: Vec<String>,
    pub span: CodeSpan,
    pub freshness: CodeGraphFreshness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffBlastRadius {
    pub symbol_key: String,
    pub inbound_count: usize,
    pub outbound_count: usize,
    #[serde(default)]
    pub inbound: Vec<CodeDiffBlastRadiusEntry>,
    #[serde(default)]
    pub outbound: Vec<CodeDiffBlastRadiusEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeDiffBlastRadiusEntry {
    pub edge_key: String,
    pub symbol_key: Option<String>,
    pub path: String,
    pub edge_kind: String,
    pub confidence: CodeGraphConfidence,
    pub distance: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeIndexReport {
    pub schema_version: SchemaVersion,
    pub repo_id: String,
    pub status: CodeIndexStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_revision: Option<String>,
    pub parsed_files: usize,
    pub persisted_documents: usize,
    pub persisted_symbols: usize,
    pub persisted_edges: usize,
    pub persisted_diagnostics: usize,
    pub stale_rows: usize,
    #[serde(default)]
    pub skipped_files: Vec<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    pub cursor: StreamCursor,
    pub indexed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeIndexStatus {
    Accepted,
    Progress,
    Completed,
    Unavailable,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CodeGraphUpdatedEvent {
    pub schema_version: SchemaVersion,
    pub repo_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_revision: Option<String>,
    #[serde(default)]
    pub topology_delta_available: bool,
    pub cursor: StreamCursor,
    pub updated_at: DateTime<Utc>,
}
