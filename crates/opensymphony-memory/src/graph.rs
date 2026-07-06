use std::{fmt::Write as _, sync::atomic::{AtomicU64, Ordering}};

use crate::opensymphony_gateway_schema::{
    cursor::StreamCursor,
    memory_graph::{
        MemoryBundleList, MemoryBundleSummary, MemoryCommunityList, MemoryCompletedTask,
        MemoryCompletedTaskSource, MemoryConceptDetail, MemoryFrontmatterView,
        MemoryGraphCitation, MemoryGraphCommunity, MemoryGraphEdge, MemoryGraphEdgeKind,
        MemoryGraphFreshness, MemoryGraphLink, MemoryGraphNode, MemoryGraphNodeKind,
        MemoryGraphNodeMetrics, MemoryGraphSnapshot, MemoryGraphSourceRef,
        MemoryGraphSnapshotMetrics, MemoryGraphUpdatedEvent, MemoryGraphVisibility,
        MemorySearchResponse, MemorySearchResult, MemoryTaskPullRequest,
    },
    version::SchemaVersion,
};

pub const DEFAULT_MEMORY_GRAPH_BUNDLE_ID: &str = "local-default";
static MEMORY_GRAPH_SEQUENCE_FLOOR: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, thiserror::Error)]
pub enum MemoryGraphProjectionError {
    #[error("unknown memory bundle `{0}`")]
    BundleNotFound(String),
    #[error("no concept found for `{0}`")]
    ConceptNotFound(String),
    #[error(transparent)]
    Memory(#[from] MemoryError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryGraphAccess {
    Public,
    AllAccessible,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryGraphCommunityOptions {
    pub include_tags: bool,
    pub include_citations: bool,
    pub include_source_refs: bool,
}

pub fn memory_graph_bundles(
    config: &MemoryConfig,
    access: MemoryGraphAccess,
) -> Result<MemoryBundleList, MemoryGraphProjectionError> {
    let issues = accessible_issues(config, access)?;
    Ok(MemoryBundleList {
        schema_version: SchemaVersion::v1(),
        bundles: vec![MemoryBundleSummary {
            id: DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string(),
            title: "OpenSymphony Memory".to_string(),
            okf_version: OKF_VERSION.to_string(),
            visibility: bundle_visibility(&issues),
            concept_count: issues.len(),
            updated_at: issues.iter().filter_map(indexed_issue_updated_at).max(),
        }],
    })
}

pub fn memory_graph_snapshot(
    config: &MemoryConfig,
    bundle_id: &str,
    access: MemoryGraphAccess,
) -> Result<MemoryGraphSnapshot, MemoryGraphProjectionError> {
    memory_graph_snapshot_with_options(
        config,
        bundle_id,
        access,
        MemoryGraphCommunityOptions::default(),
    )
}

pub fn memory_graph_snapshot_with_options(
    config: &MemoryConfig,
    bundle_id: &str,
    access: MemoryGraphAccess,
    community_options: MemoryGraphCommunityOptions,
) -> Result<MemoryGraphSnapshot, MemoryGraphProjectionError> {
    ensure_default_memory_bundle(bundle_id)?;
    let generated_at = Utc::now();
    let issues = accessible_issues(config, access)?;
    let communities = memory_graph_communities_from_issues(&issues, community_options);
    let mut nodes = BTreeMap::<String, MemoryGraphNode>::new();
    let mut edges = BTreeMap::<String, MemoryGraphEdge>::new();

    insert_node(
        &mut nodes,
        MemoryGraphNode {
            id: "bundle:local-default".to_string(),
            kind: MemoryGraphNodeKind::Bundle,
            label: "OpenSymphony Memory".to_string(),
            bundle_id: Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string()),
            concept_id: None,
            concept_type: None,
            description: None,
            path_display: None,
            resource: None,
            tags: Vec::new(),
            timestamp: None,
            visibility: Some(bundle_visibility(&issues)),
            freshness: None,
            warning_count: 0,
            frontmatter_summary: BTreeMap::new(),
            unknown_frontmatter: BTreeMap::new(),
            body_preview: None,
            metrics: MemoryGraphNodeMetrics::default(),
        },
    );

    for community in &communities {
        insert_node(
            &mut nodes,
            MemoryGraphNode {
                id: format!("community:{}", community.id),
                kind: MemoryGraphNodeKind::Community,
                label: community.label.clone(),
                bundle_id: Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string()),
                concept_id: None,
                concept_type: None,
                description: None,
                path_display: None,
                resource: None,
                tags: Vec::new(),
                timestamp: None,
                visibility: None,
                freshness: None,
                warning_count: 0,
                frontmatter_summary: BTreeMap::new(),
                unknown_frontmatter: BTreeMap::new(),
                body_preview: None,
                metrics: MemoryGraphNodeMetrics::default(),
            },
        );
    }

    let concept_ids = issues
        .iter()
        .map(|issue| (issue.concept_id.clone(), concept_node_id(issue)))
        .collect::<BTreeMap<_, _>>();
    let parsed_concepts = issues
        .iter()
        .map(|issue| (issue.concept_id.clone(), parsed_okf_concept(config, issue)))
        .collect::<BTreeMap<_, _>>();
    for issue in &issues {
        let concept_node_id = concept_node_id(issue);
        insert_directory_nodes(config, issue, &mut nodes, &mut edges);

        let parsed = parsed_concepts
            .get(&issue.concept_id)
            .and_then(Option::as_ref);
        let frontmatter = parsed.as_ref().map(|concept| frontmatter_view(config, concept));
        let resource = parsed
            .as_ref()
            .and_then(|concept| concept.frontmatter.resource.as_ref())
            .map(|resource| redact_for_dto(config, resource));
        let timestamp = parsed
            .as_ref()
            .and_then(|concept| concept.frontmatter.timestamp.clone())
            .or_else(|| issue.completion_time.clone());

        insert_node(
            &mut nodes,
            MemoryGraphNode {
                id: concept_node_id.clone(),
                kind: MemoryGraphNodeKind::Concept,
                label: redact_for_dto(config, &issue.title),
                bundle_id: Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string()),
                concept_id: Some(issue.concept_id.clone()),
                concept_type: Some(issue.concept_type.clone()),
                description: issue.description.as_ref().map(|value| redact_for_dto(config, value)),
                path_display: Some(safe_memory_path(config, &issue.capsule_path, &issue.concept_id)),
                resource: resource.clone(),
                tags: issue.tags.clone(),
                timestamp,
                visibility: Some(visibility_dto(issue.visibility)),
                freshness: Some(freshness_dto(issue.freshness)),
                warning_count: issue.warning_count,
                frontmatter_summary: frontmatter
                    .as_ref()
                    .map(|view| view.primary.clone())
                    .unwrap_or_default(),
                unknown_frontmatter: frontmatter
                    .as_ref()
                    .map(|view| view.unknown.clone())
                    .unwrap_or_default(),
                body_preview: Some(redact_for_dto(config, &summarize_text(&issue.body, 280))),
                metrics: MemoryGraphNodeMetrics::default(),
            },
        );

        for tag in &issue.tags {
            let tag_node = format!("tag:{tag}");
            insert_node(
                &mut nodes,
                simple_node(&tag_node, MemoryGraphNodeKind::Tag, tag, Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID)),
            );
            insert_edge(
                &mut edges,
                MemoryGraphEdgeKind::TaggedWith,
                &concept_node_id,
                &tag_node,
                None,
                false,
            );
        }

        if let Some(resource) = resource {
            let resource_node = format!("resource:{resource}");
            insert_node(
                &mut nodes,
                simple_node(
                    &resource_node,
                    MemoryGraphNodeKind::Resource,
                    &resource,
                    Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID),
                ),
            );
            insert_edge(
                &mut edges,
                MemoryGraphEdgeKind::DescribesResource,
                &concept_node_id,
                &resource_node,
                None,
                false,
            );
        }

        for link in &issue.links {
            let target = redact_for_dto(config, &link.target);
            if is_external_target(&target) {
                let target_node = format!("resource:{target}");
                insert_node(
                    &mut nodes,
                    simple_node(
                        &target_node,
                        MemoryGraphNodeKind::Resource,
                        &target,
                        Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID),
                    ),
                );
                insert_edge(
                    &mut edges,
                    MemoryGraphEdgeKind::ExternalLink,
                    &concept_node_id,
                    &target_node,
                    link.label.clone(),
                    false,
                );
            } else {
                let (target_node, unresolved) =
                    resolve_markdown_link_target(&target, &concept_ids).unwrap_or_else(|| {
                        (format!("unresolved:{target}"), true)
                    });
                insert_edge(
                    &mut edges,
                    MemoryGraphEdgeKind::MarkdownLink,
                    &concept_node_id,
                    &target_node,
                    link.label.clone(),
                    unresolved,
                );
            }
        }

        for citation in &issue.citations {
            let target = redact_for_dto(config, &citation.target);
            let citation_node = format!("citation:{}", citation.id);
            insert_node(
                &mut nodes,
                simple_node(
                    &citation_node,
                    MemoryGraphNodeKind::Citation,
                    citation.label.as_deref().unwrap_or(&target),
                    Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID),
                ),
            );
            insert_edge(
                &mut edges,
                MemoryGraphEdgeKind::Cites,
                &concept_node_id,
                &citation_node,
                citation.label.clone(),
                false,
            );
        }

        for source_ref in &issue.source_refs {
            let source_node = format!("source_ref:{}:{}", source_ref.kind, source_ref.id);
            insert_node(
                &mut nodes,
                simple_node(
                    &source_node,
                    MemoryGraphNodeKind::SourceRef,
                    &format!("{}: {}", source_ref.kind, source_ref.id),
                    Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID),
                ),
            );
            insert_edge(
                &mut edges,
                MemoryGraphEdgeKind::SourceSupportedBy,
                &concept_node_id,
                &source_node,
                None,
                false,
            );
        }

        for scope_ref in &issue.scope_refs {
            let scope_node = format!(
                "scope_ref:{}:{}",
                scope_kind_key(&scope_ref.kind),
                scope_ref.id
            );
            insert_node(
                &mut nodes,
                simple_node(
                    &scope_node,
                    MemoryGraphNodeKind::SourceRef,
                    scope_ref.label.as_deref().unwrap_or(&scope_ref.id),
                    Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID),
                ),
            );
            insert_edge(
                &mut edges,
                MemoryGraphEdgeKind::ScopedTo,
                &concept_node_id,
                &scope_node,
                None,
                false,
            );
        }
    }

    insert_same_resource_edges(&issues, &parsed_concepts, &mut edges);

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let edges = edges.into_values().collect::<Vec<_>>();
    apply_node_metrics(&mut nodes, &edges, &communities);
    let metrics = graph_snapshot_metrics(&nodes, &edges);

    Ok(MemoryGraphSnapshot {
        schema_version: SchemaVersion::v1(),
        bundle_id: bundle_id.to_string(),
        cursor: memory_graph_cursor(bundle_id, generated_at),
        nodes,
        edges,
        communities,
        metrics,
        filters_applied: filters_applied(access, community_options),
        generated_at,
    })
}

pub fn memory_concept_detail(
    config: &MemoryConfig,
    bundle_id: &str,
    concept_id: &str,
    access: MemoryGraphAccess,
) -> Result<MemoryConceptDetail, MemoryGraphProjectionError> {
    ensure_default_memory_bundle(bundle_id)?;
    let concept_id = normalize_concept_id(concept_id);
    let issue = accessible_issues(config, access)?
        .into_iter()
        .find(|issue| issue_matches_concept(issue, &concept_id))
        .ok_or_else(|| MemoryGraphProjectionError::ConceptNotFound(concept_id.clone()))?;
    let parsed = parsed_okf_concept(config, &issue);

    Ok(MemoryConceptDetail {
        schema_version: SchemaVersion::v1(),
        bundle_id: bundle_id.to_string(),
        concept_id: issue.concept_id.clone(),
        frontmatter_view: parsed
            .as_ref()
            .map(|concept| frontmatter_view(config, concept))
            .unwrap_or_else(|| fallback_frontmatter_view(config, &issue)),
        body_markdown: redact_for_dto(config, &issue.body),
        links: issue
            .links
            .iter()
            .map(|link| MemoryGraphLink {
                target: redact_for_dto(config, &link.target),
                label: link.label.clone(),
            })
            .collect(),
        citations: issue
            .citations
            .iter()
            .map(|citation| MemoryGraphCitation {
                id: citation.id.clone(),
                target: redact_for_dto(config, &citation.target),
                label: citation.label.clone(),
            })
            .collect(),
        source_refs: issue
            .source_refs
            .iter()
            .map(|source| MemoryGraphSourceRef {
                kind: source.kind.clone(),
                id: source.id.clone(),
                url: source.url.as_ref().map(|url| redact_for_dto(config, url)),
            })
            .collect(),
    })
}

pub fn memory_graph_communities(
    config: &MemoryConfig,
    bundle_id: &str,
    access: MemoryGraphAccess,
) -> Result<MemoryCommunityList, MemoryGraphProjectionError> {
    memory_graph_communities_with_options(
        config,
        bundle_id,
        access,
        MemoryGraphCommunityOptions::default(),
    )
}

pub fn memory_graph_communities_with_options(
    config: &MemoryConfig,
    bundle_id: &str,
    access: MemoryGraphAccess,
    community_options: MemoryGraphCommunityOptions,
) -> Result<MemoryCommunityList, MemoryGraphProjectionError> {
    ensure_default_memory_bundle(bundle_id)?;
    let issues = accessible_issues(config, access)?;
    Ok(MemoryCommunityList {
        schema_version: SchemaVersion::v1(),
        bundle_id: bundle_id.to_string(),
        communities: memory_graph_communities_from_issues(&issues, community_options),
        generated_at: Utc::now(),
    })
}

pub fn memory_graph_search(
    config: &MemoryConfig,
    query: &str,
    limit: usize,
    access: MemoryGraphAccess,
) -> Result<MemorySearchResponse, MemoryGraphProjectionError> {
    let all_issues = load_indexed_issues(config)?;
    let search_limit = all_issues.len().max(limit.max(1));
    let issues = filter_issues_for_access(all_issues, access);
    let limit = limit.max(1);
    let by_issue = issues
        .iter()
        .map(|issue| (issue.issue_key.clone(), issue))
        .collect::<BTreeMap<_, _>>();
    let scope = MemoryScopeFilter::default();
    let results = search_with_scope(config, query, search_limit, &scope)?
        .into_iter()
        .filter_map(|result| {
            let issue = by_issue.get(&result.issue_key)?;
            Some(MemorySearchResult {
                bundle_id: DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string(),
                concept_id: issue.concept_id.clone(),
                title: redact_for_dto(config, &issue.title),
                visibility: visibility_dto(issue.visibility),
                snippet: redact_for_dto(config, &result.snippet),
                areas: result.areas,
            })
        })
        .take(limit)
        .collect();

    Ok(MemorySearchResponse {
        schema_version: SchemaVersion::v1(),
        query: query.to_string(),
        bundle_id: Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string()),
        results,
    })
}

/// Unpaginated completed-task rows backed by captured memory capsules,
/// including their normalized pull-request evidence. The gateway merges
/// these with orchestrator-known completed issues and applies
/// search/sort/pagination for `/api/v1/memory/completed-tasks`; keeping
/// rows unpaginated here lets that merge happen before slicing.
pub fn memory_completed_task_rows(
    config: &MemoryConfig,
    access: MemoryGraphAccess,
) -> Result<Vec<MemoryCompletedTask>, MemoryGraphProjectionError> {
    let issues = accessible_issues(config, access)?;
    let mut prs_by_issue = load_pull_requests_by_issue(config)?;
    Ok(issues
        .into_iter()
        .filter(is_completed_indexed_issue)
        .map(|issue| {
            let prs = prs_by_issue.remove(&issue.issue_key).unwrap_or_default();
            MemoryCompletedTask {
                issue_key: issue.issue_key.clone(),
                concept_id: issue.concept_id.clone(),
                bundle_id: Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string()),
                title: redact_for_dto(config, &issue.title),
                state: issue.state.clone(),
                milestone: issue.milestone.clone(),
                url: linear_issue_url(&issue).map(|url| redact_for_dto(config, &url)),
                completed_at: completed_at_timestamp(&issue),
                prs: prs
                    .into_iter()
                    .map(|pr| MemoryTaskPullRequest {
                        number: pr.number,
                        title: redact_for_dto(config, &pr.title),
                        url: pr.url.as_ref().map(|url| redact_for_dto(config, url)),
                        merged: pr.merged_at.is_some() || pr.merge_sha.is_some(),
                        merged_at: pr.merged_at,
                    })
                    .collect(),
                source: MemoryCompletedTaskSource::Memory,
            }
        })
        .collect())
}

fn is_completed_indexed_issue(issue: &IndexedIssue) -> bool {
    // Only issue capsules count as tasks; topic docs and generic concepts
    // also live in the catalog but are not completed work items.
    if issue.concept_type != "issue-capsule" {
        return false;
    }
    // The state, when present, is authoritative: OKF reindexing fills
    // `completion_time` from the frontmatter timestamp, so a bare
    // timestamp on an "In Progress" capsule must not read as completed.
    match issue
        .state
        .as_deref()
        .map(|state| state.trim().to_ascii_lowercase())
    {
        Some(state) => matches!(state.as_str(), "done" | "completed" | "closed" | "merged"),
        None => issue.completion_time.is_some(),
    }
}

/// The task's actual completion date, from the recorded completion
/// timestamp only. Unlike `indexed_issue_updated_at`, this never falls back
/// to `captured_at`: reporting the capture/reindex time as the done date
/// would corrupt the Completed table's dates and completed-date sorting for
/// legacy or manually authored capsules that carry no completion timestamp.
fn completed_at_timestamp(issue: &IndexedIssue) -> Option<DateTime<Utc>> {
    issue
        .completion_time
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn linear_issue_url(issue: &IndexedIssue) -> Option<String> {
    issue
        .source_refs
        .iter()
        .find(|source| source.kind == "linear_issue")
        .and_then(|source| source.url.clone())
}

pub fn memory_graph_updated_event(
    _config: &MemoryConfig,
    bundle_id: &str,
    _access: MemoryGraphAccess,
) -> Result<MemoryGraphUpdatedEvent, MemoryGraphProjectionError> {
    ensure_default_memory_bundle(bundle_id)?;
    let updated_at = Utc::now();
    Ok(MemoryGraphUpdatedEvent {
        schema_version: SchemaVersion::v1(),
        bundle_id: bundle_id.to_string(),
        cursor: memory_graph_cursor(bundle_id, updated_at),
        updated_at,
    })
}

fn accessible_issues(
    config: &MemoryConfig,
    access: MemoryGraphAccess,
) -> Result<Vec<IndexedIssue>, MemoryGraphProjectionError> {
    Ok(filter_issues_for_access(load_indexed_issues(config)?, access))
}

fn filter_issues_for_access(
    mut issues: Vec<IndexedIssue>,
    access: MemoryGraphAccess,
) -> Vec<IndexedIssue> {
    if access == MemoryGraphAccess::Public {
        issues.retain(|issue| issue.visibility == MemoryVisibility::Public);
    }
    issues
}

fn ensure_default_memory_bundle(bundle_id: &str) -> Result<(), MemoryGraphProjectionError> {
    if bundle_id == DEFAULT_MEMORY_GRAPH_BUNDLE_ID {
        Ok(())
    } else {
        Err(MemoryGraphProjectionError::BundleNotFound(bundle_id.to_string()))
    }
}

fn bundle_visibility(issues: &[IndexedIssue]) -> MemoryGraphVisibility {
    if issues
        .iter()
        .any(|issue| issue.visibility == MemoryVisibility::Private)
    {
        MemoryGraphVisibility::Private
    } else {
        MemoryGraphVisibility::Public
    }
}

fn visibility_dto(visibility: MemoryVisibility) -> MemoryGraphVisibility {
    match visibility {
        MemoryVisibility::Public => MemoryGraphVisibility::Public,
        MemoryVisibility::Private => MemoryGraphVisibility::Private,
    }
}

fn freshness_dto(freshness: MemoryFreshness) -> MemoryGraphFreshness {
    match freshness {
        MemoryFreshness::Current => MemoryGraphFreshness::Current,
        MemoryFreshness::Stale => MemoryGraphFreshness::Stale,
        MemoryFreshness::Unknown => MemoryGraphFreshness::Unknown,
    }
}

fn filters_applied(
    access: MemoryGraphAccess,
    community_options: MemoryGraphCommunityOptions,
) -> Vec<String> {
    let mut filters = match access {
        MemoryGraphAccess::Public => vec!["visibility:public".to_string()],
        MemoryGraphAccess::AllAccessible => Vec::new(),
    };
    if community_options.include_tags {
        filters.push("communities:include_tags".to_string());
    }
    if community_options.include_citations {
        filters.push("communities:include_citations".to_string());
    }
    if community_options.include_source_refs {
        filters.push("communities:include_source_refs".to_string());
    }
    filters
}

fn indexed_issue_updated_at(issue: &IndexedIssue) -> Option<DateTime<Utc>> {
    issue
        .completion_time
        .as_deref()
        .or(Some(issue.captured_at.as_str()))
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
}

fn memory_graph_cursor(bundle_id: &str, timestamp: DateTime<Utc>) -> StreamCursor {
    StreamCursor::new(
        memory_graph_sequence(timestamp),
        format!("memory-graph:{bundle_id}"),
    )
}

fn memory_graph_sequence(timestamp: DateTime<Utc>) -> u64 {
    let candidate = timestamp
        .timestamp_nanos_opt()
        .unwrap_or_else(|| timestamp.timestamp_millis().saturating_mul(1_000_000))
        .max(0) as u64;
    let mut previous = MEMORY_GRAPH_SEQUENCE_FLOOR.load(Ordering::Relaxed);
    loop {
        let next = candidate.max(previous.saturating_add(1));
        match MEMORY_GRAPH_SEQUENCE_FLOOR.compare_exchange_weak(
            previous,
            next,
            Ordering::SeqCst,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(current) => previous = current,
        }
    }
}

fn concept_node_id(issue: &IndexedIssue) -> String {
    format!("concept:{}", issue.concept_id)
}

fn insert_node(nodes: &mut BTreeMap<String, MemoryGraphNode>, node: MemoryGraphNode) {
    nodes.entry(node.id.clone()).or_insert(node);
}

fn simple_node(
    id: &str,
    kind: MemoryGraphNodeKind,
    label: &str,
    bundle_id: Option<&str>,
) -> MemoryGraphNode {
    MemoryGraphNode {
        id: id.to_string(),
        kind,
        label: label.to_string(),
        bundle_id: bundle_id.map(str::to_string),
        concept_id: None,
        concept_type: None,
        description: None,
        path_display: None,
        resource: None,
        tags: Vec::new(),
        timestamp: None,
        visibility: None,
        freshness: None,
        warning_count: 0,
        frontmatter_summary: BTreeMap::new(),
        unknown_frontmatter: BTreeMap::new(),
        body_preview: None,
        metrics: MemoryGraphNodeMetrics::default(),
    }
}

fn insert_edge(
    edges: &mut BTreeMap<String, MemoryGraphEdge>,
    kind: MemoryGraphEdgeKind,
    source_id: &str,
    target_id: &str,
    label: Option<String>,
    unresolved: bool,
) {
    let label_key = label
        .as_deref()
        .map(|value| format!("label:{}", edge_id_component(value)))
        .unwrap_or_else(|| "no_label".to_string());
    let id = format!(
        "{}:{source_id}->{target_id}:{}:{}",
        edge_kind_key(kind),
        unresolved,
        label_key
    );
    edges.entry(id.clone()).or_insert(MemoryGraphEdge {
        id,
        kind,
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        label,
        unresolved,
        metadata: BTreeMap::new(),
    });
}

fn edge_id_component(value: &str) -> String {
    let mut escaped = String::new();
    for byte in value.bytes() {
        if matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_') {
            escaped.push(char::from(byte));
        } else {
            write!(&mut escaped, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    escaped
}

fn insert_directory_nodes(
    config: &MemoryConfig,
    issue: &IndexedIssue,
    nodes: &mut BTreeMap<String, MemoryGraphNode>,
    edges: &mut BTreeMap<String, MemoryGraphEdge>,
) {
    let path = safe_memory_path(config, &issue.capsule_path, &issue.concept_id);
    let parts = path.split('/').collect::<Vec<_>>();
    let mut parent = "bundle:local-default".to_string();
    let mut accumulated = Vec::<&str>::new();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        accumulated.push(part);
        let dir = accumulated.join("/");
        let id = format!("directory:{dir}");
        insert_node(
            nodes,
            simple_node(&id, MemoryGraphNodeKind::Directory, &dir, Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID)),
        );
        insert_edge(edges, MemoryGraphEdgeKind::Contains, &parent, &id, None, false);
        parent = id;
    }
    insert_edge(
        edges,
        MemoryGraphEdgeKind::Contains,
        &parent,
        &concept_node_id(issue),
        None,
        false,
    );
}

fn safe_memory_path(config: &MemoryConfig, path: &Path, fallback_concept_id: &str) -> String {
    let absolute = resolve_index_path(config, path);
    absolute
        .strip_prefix(&config.memory_root)
        .or_else(|_| absolute.strip_prefix(&config.repo_root))
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| fallback_concept_id.to_string())
}

fn resolve_index_path(config: &MemoryConfig, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    if path.starts_with(DEFAULT_MEMORY_ROOT) {
        return config.repo_root.join(path);
    }
    let memory_path = config.memory_root.join(path);
    if memory_path.exists() {
        return memory_path;
    }
    let repo_path = config.repo_root.join(path);
    if repo_path.exists() {
        return repo_path;
    }
    memory_path
}

fn redact_for_dto(config: &MemoryConfig, value: &str) -> String {
    let repo_root = config.repo_root.to_string_lossy();
    let memory_root = config.memory_root.to_string_lossy();
    let value = replace_path_token(value, repo_root.as_ref(), "[redacted-local-path]");
    let value = replace_path_token(&value, memory_root.as_ref(), "[redacted-memory-path]");
    replace_path_token(&value, ".opensymphony/memory", "[redacted-memory-path]")
}

fn replace_path_token(value: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return value.to_string();
    }
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(index) = remaining.find(needle) {
        result.push_str(&remaining[..index]);
        let before = &remaining[..index];
        let after = &remaining[index + needle.len()..];
        if is_path_token_left_boundary(before) && is_path_token_right_boundary(after) {
            result.push_str(replacement);
        } else {
            result.push_str(needle);
        }
        remaining = after;
    }
    result.push_str(remaining);
    result
}

fn is_path_token_left_boundary(before: &str) -> bool {
    match before.chars().next_back() {
        None => true,
        Some('/') | Some('\\') => true,
        Some(character) => {
            !(character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | '.'))
        }
    }
}

fn is_path_token_right_boundary(after: &str) -> bool {
    let mut chars = after.chars();
    match chars.next() {
        None => true,
        Some('/') | Some('\\') => true,
        Some('.') => chars.next().is_none_or(|next| {
            next.is_whitespace() || matches!(next, ',' | ';' | ':' | ')' | ']' | '}')
        }),
        Some(character) => {
            !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
        }
    }
}

fn parsed_okf_concept(config: &MemoryConfig, issue: &IndexedIssue) -> Option<OkfConcept> {
    let path = resolve_index_path(config, &issue.capsule_path);
    let contents = fs::read_to_string(&path).ok()?;
    let relative_path = path
        .strip_prefix(&config.memory_root)
        .or_else(|_| path.strip_prefix(config.repo_root.join(DEFAULT_MEMORY_ROOT)))
        .map(Path::to_path_buf)
        .ok()
        .or_else(|| issue.capsule_path.is_relative().then(|| issue.capsule_path.clone()))
        .or_else(|| memory_relative_path_from_components(&path))?;
    parse_okf_concept(&config.memory_root, &relative_path, &contents).ok()
}

fn memory_relative_path_from_components(path: &Path) -> Option<PathBuf> {
    let parts = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let marker = [".opensymphony", "memory"];
    let index = parts
        .windows(marker.len())
        .position(|window| window == marker)?;
    let mut relative = PathBuf::new();
    for part in &parts[index + marker.len()..] {
        relative.push(part);
    }
    Some(relative)
}

fn frontmatter_view(config: &MemoryConfig, concept: &OkfConcept) -> MemoryFrontmatterView {
    let mut primary = BTreeMap::new();
    primary.insert("type".to_string(), json_string(&concept.frontmatter.concept_type));
    if let Some(title) = &concept.frontmatter.title {
        primary.insert("title".to_string(), json_string(&redact_for_dto(config, title)));
    }
    if let Some(description) = &concept.frontmatter.description {
        primary.insert(
            "description".to_string(),
            json_string(&redact_for_dto(config, description)),
        );
    }
    if let Some(resource) = &concept.frontmatter.resource {
        primary.insert(
            "resource".to_string(),
            json_string(&redact_for_dto(config, resource)),
        );
    }
    if !concept.frontmatter.tags.is_empty() {
        primary.insert(
            "tags".to_string(),
            serde_json::to_value(&concept.frontmatter.tags).unwrap_or(serde_json::Value::Null),
        );
    }
    if let Some(timestamp) = &concept.frontmatter.timestamp {
        primary.insert("timestamp".to_string(), json_string(timestamp));
    }

    MemoryFrontmatterView {
        primary,
        opensymphony: concept
            .frontmatter
            .opensymphony
            .as_ref()
            .and_then(json_object_map)
            .map(|map| redact_map_for_dto(config, map))
            .unwrap_or_default(),
        unknown: concept
            .frontmatter
            .extra
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
                )
            })
            .map(|(key, value)| {
                let value = redact_value_for_dto_key(config, &key, value);
                (key, value)
            })
            .collect(),
    }
}

fn fallback_frontmatter_view(config: &MemoryConfig, issue: &IndexedIssue) -> MemoryFrontmatterView {
    let mut primary = BTreeMap::new();
    primary.insert("type".to_string(), json_string(&issue.concept_type));
    primary.insert("title".to_string(), json_string(&redact_for_dto(config, &issue.title)));
    if let Some(description) = &issue.description {
        primary.insert(
            "description".to_string(),
            json_string(&redact_for_dto(config, description)),
        );
    }
    if !issue.tags.is_empty() {
        primary.insert(
            "tags".to_string(),
            serde_json::to_value(&issue.tags).unwrap_or(serde_json::Value::Null),
        );
    }
    let mut opensymphony = BTreeMap::new();
    opensymphony.insert(
        "visibility".to_string(),
        serde_json::to_value(issue.visibility).unwrap_or(serde_json::Value::Null),
    );
    opensymphony.insert(
        "scope_refs".to_string(),
        serde_json::to_value(&issue.scope_refs).unwrap_or(serde_json::Value::Null),
    );
    opensymphony.insert(
        "source_refs".to_string(),
        serde_json::to_value(&issue.source_refs).unwrap_or(serde_json::Value::Null),
    );
    opensymphony.insert(
        "links".to_string(),
        serde_json::to_value(&issue.links).unwrap_or(serde_json::Value::Null),
    );
    opensymphony.insert(
        "citations".to_string(),
        serde_json::to_value(&issue.citations).unwrap_or(serde_json::Value::Null),
    );
    MemoryFrontmatterView {
        primary,
        opensymphony: redact_map_for_dto(config, opensymphony),
        unknown: BTreeMap::new(),
    }
}

fn redact_map_for_dto(
    config: &MemoryConfig,
    map: BTreeMap<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    map.into_iter()
        .map(|(key, value)| {
            let value = redact_value_for_dto_key(config, &key, value);
            (key, value)
        })
        .collect()
}

fn redact_value_for_dto_key(
    config: &MemoryConfig,
    key: &str,
    value: serde_json::Value,
) -> serde_json::Value {
    if is_secret_like_frontmatter_key(key) {
        return redact_secret_value_shape(value);
    }
    match value {
        serde_json::Value::String(value) => serde_json::Value::String(redact_for_dto(config, &value)),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| redact_value_for_dto_key(config, key, value))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let value = redact_value_for_dto_key(config, &key, value);
                    (key, value)
                })
                .collect(),
        ),
        value => value,
    }
}

fn redact_secret_value_shape(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(redact_secret_value_shape)
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact_secret_value_shape(value)))
                .collect(),
        ),
        _ => serde_json::Value::String("[redacted-secret]".to_string()),
    }
}

fn is_secret_like_frontmatter_key(key: &str) -> bool {
    let parts = frontmatter_key_parts(key);
    let part_refs = parts.iter().map(String::as_str).collect::<Vec<_>>();
    if part_refs.is_empty() || has_non_secret_descriptor(&part_refs) {
        return false;
    }
    if has_any(&part_refs, &[
        "secret",
        "secrets",
        "password",
        "passwords",
        "credential",
        "credentials",
        "pwd",
        "pwds",
        "jwt",
        "jwts",
    ]) {
        return true;
    }
    if has_any(&part_refs, &["cookie", "cookies"])
        && part_refs
            .iter()
            .any(|part| matches!(*part, "auth" | "oauth" | "session" | "access" | "refresh"))
    {
        return true;
    }
    if has_any(&part_refs, &["token", "tokens"])
        && (part_refs.len() == 1
            || part_refs
                .iter()
                .any(|part| matches!(*part, "auth" | "oauth" | "access" | "refresh" | "session" | "bearer" | "client" | "id" | "api" | "xsrf" | "csrf")))
    {
        return true;
    }
    has_adjacent_parts(&part_refs, "api", "key")
        || has_adjacent_parts(&part_refs, "access", "key")
        || has_adjacent_parts(&part_refs, "private", "key")
        || has_adjacent_parts(&part_refs, "signing", "key")
        || has_adjacent_parts(&part_refs, "encryption", "key")
        || has_adjacent_parts(&part_refs, "session", "id")
        || has_compound_secret_key(&part_refs)
}

fn frontmatter_key_parts(key: &str) -> Vec<String> {
    let characters = key.chars().collect::<Vec<_>>();
    let mut segmented = String::with_capacity(key.len());
    for (index, character) in characters.iter().copied().enumerate() {
        if index > 0 {
            let previous = characters[index - 1];
            let next = characters.get(index + 1).copied();
            let camel_boundary = character.is_ascii_uppercase()
                && (previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase()
                        && next.is_some_and(|next| next.is_ascii_lowercase())));
            let digit_boundary =
                character.is_ascii_digit() && previous.is_ascii_alphabetic();
            let alpha_after_digit =
                character.is_ascii_alphabetic() && previous.is_ascii_digit();
            if camel_boundary || digit_boundary || alpha_after_digit {
                segmented.push('_');
            }
        }
        segmented.push(character.to_ascii_lowercase());
    }
    segmented
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn has_non_secret_descriptor(parts: &[&str]) -> bool {
    parts.iter().any(|part| {
        matches!(
            *part,
            "algorithm"
                | "algorithms"
                | "policy"
                | "policies"
                | "type"
                | "types"
                | "format"
                | "formats"
                | "example"
                | "examples"
        )
    })
}

fn has_adjacent_parts(parts: &[&str], left: &str, right: &str) -> bool {
    parts
        .windows(2)
        .any(|window| window == [left, right])
}

fn has_any(parts: &[&str], candidates: &[&str]) -> bool {
    parts.iter().any(|part| candidates.contains(part))
}

fn has_compound_secret_key(parts: &[&str]) -> bool {
    if parts.len() != 1 {
        return false;
    }
    let compact = parts[0];
    (compact.ends_with("token")
        && matches!(
            compact.trim_end_matches("token"),
            "api" | "auth" | "oauth" | "refresh" | "id" | "xsrf" | "csrf" | "bearer" | "access" | "client" | "session"
        ))
        || (compact.ends_with("key")
            && matches!(
                compact.trim_end_matches("key"),
                "api" | "private" | "signing" | "encryption" | "access"
            ))
        || (compact.ends_with("secret")
            && matches!(compact.trim_end_matches("secret"), "api" | "client"))
        || (compact.ends_with("id") && compact.trim_end_matches("id") == "session")
}

fn json_string(value: &str) -> serde_json::Value {
    serde_json::Value::String(value.to_string())
}

fn json_object_map<T: Serialize>(value: &T) -> Option<BTreeMap<String, serde_json::Value>> {
    serde_json::to_value(value)
        .ok()?
        .as_object()
        .map(|map| map.iter().map(|(key, value)| (key.clone(), value.clone())).collect())
}

fn normalize_concept_id(concept_id: &str) -> String {
    concept_id
        .trim()
        .trim_matches('/')
        .trim_end_matches(".md")
        .to_string()
}

fn issue_matches_concept(issue: &IndexedIssue, concept_id: &str) -> bool {
    issue.concept_id == concept_id
        || issue.concept_id.trim_start_matches('/') == concept_id.trim_start_matches('/')
        || (!concept_id.contains('/') && issue.issue_key == normalize_issue_key(concept_id))
}

fn edge_kind_key(kind: MemoryGraphEdgeKind) -> &'static str {
    match kind {
        MemoryGraphEdgeKind::Contains => "contains",
        MemoryGraphEdgeKind::MarkdownLink => "markdown_link",
        MemoryGraphEdgeKind::ExternalLink => "external_link",
        MemoryGraphEdgeKind::Cites => "cites",
        MemoryGraphEdgeKind::TaggedWith => "tagged_with",
        MemoryGraphEdgeKind::DescribesResource => "describes_resource",
        MemoryGraphEdgeKind::ScopedTo => "scoped_to",
        MemoryGraphEdgeKind::SourceSupportedBy => "source_supported_by",
        MemoryGraphEdgeKind::SameResource => "same_resource",
    }
}

fn scope_kind_key(kind: &KnowledgeScopeKind) -> &'static str {
    match kind {
        KnowledgeScopeKind::LocalInstance => "local_instance",
        KnowledgeScopeKind::Organization => "organization",
        KnowledgeScopeKind::ProjectSet => "project_set",
        KnowledgeScopeKind::Project => "project",
        KnowledgeScopeKind::Milestone => "milestone",
        KnowledgeScopeKind::WorkItem => "work_item",
        KnowledgeScopeKind::Repository => "repository",
        KnowledgeScopeKind::CodePath => "code_path",
        KnowledgeScopeKind::Area => "area",
    }
}

fn is_external_target(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

fn resolve_markdown_link_target(
    target: &str,
    concept_ids: &BTreeMap<String, String>,
) -> Option<(String, bool)> {
    let normalized = normalize_concept_id(target);
    let exact = concept_ids
        .get(&normalized)
        .or_else(|| concept_ids.get(normalized.trim_start_matches('/')));
    if let Some(node_id) = exact {
        return Some((node_id.clone(), false));
    }

    let normalized_leaf = normalized.trim_start_matches('/');
    let mut suffix_matches = concept_ids
        .iter()
        .filter(|(concept_id, _)| {
            concept_id
                .rsplit('/')
                .next()
                .is_some_and(|leaf| leaf == normalized_leaf)
                || concept_id.ends_with(&format!("/{normalized_leaf}"))
        })
        .map(|(_, node_id)| node_id.clone())
        .collect::<Vec<_>>();
    suffix_matches.sort();
    suffix_matches.dedup();
    if suffix_matches.len() == 1 {
        suffix_matches.pop().map(|node_id| (node_id, false))
    } else {
        None
    }
}

fn insert_same_resource_edges(
    issues: &[IndexedIssue],
    parsed_concepts: &BTreeMap<String, Option<OkfConcept>>,
    edges: &mut BTreeMap<String, MemoryGraphEdge>,
) {
    let mut by_resource = BTreeMap::<String, Vec<&IndexedIssue>>::new();
    for issue in issues {
        if let Some(resource) = parsed_concepts
            .get(&issue.concept_id)
            .and_then(Option::as_ref)
            .and_then(|concept| concept.frontmatter.resource.as_ref())
        {
            by_resource.entry(resource.clone()).or_default().push(issue);
        }
    }
    for issues in by_resource.values() {
        for (index, left) in issues.iter().enumerate() {
            for right in issues.iter().skip(index + 1) {
                insert_edge(
                    edges,
                    MemoryGraphEdgeKind::SameResource,
                    &concept_node_id(left),
                    &concept_node_id(right),
                    None,
                    false,
                );
            }
        }
    }
}

fn memory_graph_communities_from_issues(
    issues: &[IndexedIssue],
    options: MemoryGraphCommunityOptions,
) -> Vec<MemoryGraphCommunity> {
    let mut communities = BTreeMap::<String, (String, Vec<String>)>::new();
    for issue in issues {
        let (id, label) = community_key(issue);
        let (_, node_ids) = communities
            .entry(id)
            .or_insert_with(|| (label, Vec::new()));
        node_ids.push(concept_node_id(issue));
        if options.include_tags {
            node_ids.extend(issue.tags.iter().map(|tag| format!("tag:{tag}")));
        }
        if options.include_citations {
            node_ids.extend(
                issue
                    .citations
                    .iter()
                    .map(|citation| format!("citation:{}", citation.id)),
            );
        }
        if options.include_source_refs {
            node_ids.extend(
                issue
                    .source_refs
                    .iter()
                    .map(|source_ref| format!("source_ref:{}:{}", source_ref.kind, source_ref.id)),
            );
        }
    }
    communities
        .into_iter()
        .map(|(id, (label, mut node_ids))| {
            node_ids.sort();
            node_ids.dedup();
            let concept_count = node_ids
                .iter()
                .filter(|node_id| node_id.starts_with("concept:"))
                .count();
            MemoryGraphCommunity {
                id,
                label,
                concept_count,
                node_ids,
            }
        })
        .collect()
}

fn community_key(issue: &IndexedIssue) -> (String, String) {
    // Assign exactly one stable community per concept. Multiple areas are
    // ordered before selecting the first; tags keep frontmatter order.
    if let Some(area) = issue.areas().into_iter().next() {
        return (format!("area:{area}"), area);
    }
    if let Some(tag) = issue.tags.first() {
        return (format!("tag:{tag}"), tag.clone());
    }
    if let Some((directory, _)) = issue.concept_id.rsplit_once('/') {
        return (format!("directory:{directory}"), directory.to_string());
    }
    (
        format!("type:{}", issue.concept_type),
        issue.concept_type.clone(),
    )
}

fn apply_node_metrics(
    nodes: &mut [MemoryGraphNode],
    edges: &[MemoryGraphEdge],
    communities: &[MemoryGraphCommunity],
) {
    let mut indegree = BTreeMap::<String, usize>::new();
    let mut outdegree = BTreeMap::<String, usize>::new();
    for edge in edges {
        *outdegree.entry(edge.source_id.clone()).or_default() += 1;
        *indegree.entry(edge.target_id.clone()).or_default() += 1;
    }
    let max_degree = nodes
        .iter()
        .map(|node| {
            indegree.get(&node.id).copied().unwrap_or_default()
                + outdegree.get(&node.id).copied().unwrap_or_default()
        })
        .max()
        .unwrap_or(0);
    // Centrality is global normalized degree across all graph node kinds, not
    // concept-only centrality.
    let community_by_node = communities
        .iter()
        .flat_map(|community| {
            community
                .node_ids
                .iter()
                .map(move |node_id| (node_id.clone(), community.id.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut bridge_edges = BTreeMap::<String, usize>::new();
    for edge in edges {
        let source_community = community_by_node.get(&edge.source_id);
        let target_community = community_by_node.get(&edge.target_id);
        if source_community.zip(target_community).is_some_and(|(left, right)| left != right) {
            *bridge_edges.entry(edge.source_id.clone()).or_default() += 1;
            *bridge_edges.entry(edge.target_id.clone()).or_default() += 1;
        }
    }
    for node in nodes {
        let indegree = indegree.get(&node.id).copied().unwrap_or_default();
        let outdegree = outdegree.get(&node.id).copied().unwrap_or_default();
        let degree = indegree + outdegree;
        node.metrics.degree = degree;
        node.metrics.indegree = indegree;
        node.metrics.outdegree = outdegree;
        if max_degree > 0 {
            node.metrics.centrality = Some(degree as f64 / max_degree as f64);
        }
        if let Some(bridge_count) = bridge_edges.get(&node.id).copied() {
            node.metrics.bridge_score = Some(bridge_count as f64);
        }
        node.metrics.community_id = community_by_node.get(&node.id).cloned();
    }
}

fn graph_snapshot_metrics(
    nodes: &[MemoryGraphNode],
    edges: &[MemoryGraphEdge],
) -> MemoryGraphSnapshotMetrics {
    let mut semantic_nodes = BTreeSet::<String>::new();
    for edge in edges.iter().filter(|edge| edge.kind != MemoryGraphEdgeKind::Contains) {
        semantic_nodes.insert(edge.source_id.clone());
        semantic_nodes.insert(edge.target_id.clone());
    }
    MemoryGraphSnapshotMetrics {
        orphan_count: nodes
            .iter()
            .filter(|node| {
                node.kind == MemoryGraphNodeKind::Concept && !semantic_nodes.contains(&node.id)
            })
            .count(),
        broken_link_count: edges.iter().filter(|edge| edge.unresolved).count(),
        stale_concept_count: nodes
            .iter()
            .filter(|node| {
                node.kind == MemoryGraphNodeKind::Concept
                    && node.freshness == Some(MemoryGraphFreshness::Stale)
            })
            .count(),
        warning_count: nodes
            .iter()
            .filter(|node| node.kind == MemoryGraphNodeKind::Concept)
            .map(|node| node.warning_count)
            .sum(),
    }
}
