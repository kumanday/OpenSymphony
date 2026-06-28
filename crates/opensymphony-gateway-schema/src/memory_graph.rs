use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{cursor::StreamCursor, version::SchemaVersion};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryBundleList {
    pub schema_version: SchemaVersion,
    pub bundles: Vec<MemoryBundleSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryBundleSummary {
    pub id: String,
    pub title: String,
    pub okf_version: String,
    pub visibility: MemoryGraphVisibility,
    pub concept_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryGraphSnapshot {
    pub schema_version: SchemaVersion,
    pub bundle_id: String,
    pub cursor: StreamCursor,
    pub nodes: Vec<MemoryGraphNode>,
    pub edges: Vec<MemoryGraphEdge>,
    pub communities: Vec<MemoryGraphCommunity>,
    #[serde(default)]
    pub filters_applied: Vec<String>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryConceptDetail {
    pub schema_version: SchemaVersion,
    pub bundle_id: String,
    pub concept_id: String,
    pub frontmatter_view: MemoryFrontmatterView,
    pub body_markdown: String,
    #[serde(default)]
    pub links: Vec<MemoryGraphLink>,
    #[serde(default)]
    pub citations: Vec<MemoryGraphCitation>,
    #[serde(default)]
    pub source_refs: Vec<MemoryGraphSourceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryCommunityList {
    pub schema_version: SchemaVersion,
    pub bundle_id: String,
    #[serde(default)]
    pub communities: Vec<MemoryGraphCommunity>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchResponse {
    pub schema_version: SchemaVersion,
    pub query: String,
    pub bundle_id: Option<String>,
    pub results: Vec<MemorySearchResult>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchResult {
    pub bundle_id: String,
    pub concept_id: String,
    pub title: String,
    pub visibility: MemoryGraphVisibility,
    pub snippet: String,
    #[serde(default)]
    pub areas: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryGraphNode {
    pub id: String,
    pub kind: MemoryGraphNodeKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concept_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<MemoryGraphVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<MemoryGraphFreshness>,
    #[serde(default)]
    pub warning_count: usize,
    #[serde(default)]
    pub frontmatter_summary: BTreeMap<String, Value>,
    #[serde(default)]
    pub unknown_frontmatter: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_preview: Option<String>,
    #[serde(default)]
    pub metrics: MemoryGraphNodeMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryGraphEdge {
    pub id: String,
    pub kind: MemoryGraphEdgeKind,
    pub source_id: String,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub unresolved: bool,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryGraphCommunity {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub concept_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryFrontmatterView {
    #[serde(default)]
    pub primary: BTreeMap<String, Value>,
    #[serde(default)]
    pub opensymphony: BTreeMap<String, Value>,
    #[serde(default)]
    pub unknown: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryGraphLink {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryGraphCitation {
    pub id: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryGraphSourceRef {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MemoryGraphNodeMetrics {
    #[serde(default)]
    pub indegree: usize,
    #[serde(default)]
    pub outdegree: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagerank: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryGraphUpdatedEvent {
    pub schema_version: SchemaVersion,
    pub bundle_id: String,
    pub cursor: StreamCursor,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryGraphVisibility {
    Public,
    Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryGraphFreshness {
    Current,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryGraphNodeKind {
    Bundle,
    Directory,
    Concept,
    Tag,
    Resource,
    Citation,
    SourceRef,
    Community,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryGraphEdgeKind {
    Contains,
    MarkdownLink,
    ExternalLink,
    Cites,
    TaggedWith,
    DescribesResource,
    ScopedTo,
    SourceSupportedBy,
    SameResource,
}
