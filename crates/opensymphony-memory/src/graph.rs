use crate::opensymphony_gateway_schema::{
    cursor::StreamCursor,
    memory_graph::{
        MemoryBundleList, MemoryBundleSummary, MemoryCommunityList, MemoryConceptDetail,
        MemoryFrontmatterView, MemoryGraphCitation, MemoryGraphCommunity, MemoryGraphEdge,
        MemoryGraphEdgeKind, MemoryGraphFreshness, MemoryGraphLink, MemoryGraphNode,
        MemoryGraphNodeKind, MemoryGraphNodeMetrics, MemoryGraphSnapshot, MemoryGraphSourceRef,
        MemoryGraphUpdatedEvent, MemoryGraphVisibility, MemorySearchResponse, MemorySearchResult,
    },
    version::SchemaVersion,
};

pub const DEFAULT_MEMORY_GRAPH_BUNDLE_ID: &str = "local-default";

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
    ensure_default_memory_bundle(bundle_id)?;
    let generated_at = Utc::now();
    let issues = accessible_issues(config, access)?;
    let communities = memory_graph_communities_from_issues(&issues);
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
    for issue in &issues {
        let concept_node_id = concept_node_id(issue);
        insert_directory_nodes(config, issue, &mut nodes, &mut edges);

        let parsed = parsed_okf_concept(config, issue);
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

    insert_same_resource_edges(&issues, config, &mut edges);

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let edges = edges.into_values().collect::<Vec<_>>();
    apply_node_metrics(&mut nodes, &edges, &communities);

    Ok(MemoryGraphSnapshot {
        schema_version: SchemaVersion::v1(),
        bundle_id: bundle_id.to_string(),
        cursor: memory_graph_cursor(bundle_id, generated_at),
        nodes,
        edges,
        communities,
        filters_applied: filters_applied(access),
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
    ensure_default_memory_bundle(bundle_id)?;
    let issues = accessible_issues(config, access)?;
    Ok(MemoryCommunityList {
        schema_version: SchemaVersion::v1(),
        bundle_id: bundle_id.to_string(),
        communities: memory_graph_communities_from_issues(&issues),
        generated_at: Utc::now(),
    })
}

pub fn memory_graph_search(
    config: &MemoryConfig,
    query: &str,
    limit: usize,
    access: MemoryGraphAccess,
) -> Result<MemorySearchResponse, MemoryGraphProjectionError> {
    let issues = accessible_issues(config, access)?;
    let by_issue = issues
        .iter()
        .map(|issue| (issue.issue_key.clone(), issue))
        .collect::<BTreeMap<_, _>>();
    let scope = MemoryScopeFilter::default();
    let results = search_with_scope(config, query, limit, &scope)?
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
        .collect();

    Ok(MemorySearchResponse {
        schema_version: SchemaVersion::v1(),
        query: query.to_string(),
        bundle_id: Some(DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string()),
        results,
    })
}

pub fn memory_graph_updated_event(
    config: &MemoryConfig,
    bundle_id: &str,
    access: MemoryGraphAccess,
) -> Result<MemoryGraphUpdatedEvent, MemoryGraphProjectionError> {
    ensure_default_memory_bundle(bundle_id)?;
    let _ = accessible_issues(config, access)?;
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
    let mut issues = load_indexed_issues(config)?;
    if access == MemoryGraphAccess::Public {
        issues.retain(|issue| issue.visibility == MemoryVisibility::Public);
    }
    Ok(issues)
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

fn filters_applied(access: MemoryGraphAccess) -> Vec<String> {
    match access {
        MemoryGraphAccess::Public => vec!["visibility:public".to_string()],
        MemoryGraphAccess::AllAccessible => Vec::new(),
    }
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
    timestamp.timestamp_millis().max(0) as u64
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
    let label_key = label.as_deref().unwrap_or_default();
    let id = format!(
        "{kind:?}:{source_id}->{target_id}:{}:{:016x}",
        unresolved,
        stable_edge_hash(label_key)
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

fn stable_edge_hash(value: &str) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
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
    let repo_path = config.repo_root.join(path);
    if repo_path.exists() {
        return repo_path;
    }
    config.memory_root.join(path)
}

fn redact_for_dto(config: &MemoryConfig, value: &str) -> String {
    let repo_root = config.repo_root.to_string_lossy();
    let memory_root = config.memory_root.to_string_lossy();
    value
        .replace(repo_root.as_ref(), "[redacted-local-path]")
        .replace(memory_root.as_ref(), "[redacted-memory-path]")
        .replace(".opensymphony/memory/", "[redacted-memory-path]/")
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
            .map(|(key, value)| (key, redact_value_for_dto(config, value)))
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
        .map(|(key, value)| (key, redact_value_for_dto(config, value)))
        .collect()
}

fn redact_value_for_dto(
    config: &MemoryConfig,
    value: serde_json::Value,
) -> serde_json::Value {
    match value {
        serde_json::Value::String(value) => serde_json::Value::String(redact_for_dto(config, &value)),
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(|value| redact_value_for_dto(config, value))
                .collect(),
        ),
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact_value_for_dto(config, value)))
                .collect(),
        ),
        value => value,
    }
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
        || issue.issue_key == normalize_issue_key(concept_id)
        || concept_id.ends_with(&issue.issue_key)
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
    config: &MemoryConfig,
    edges: &mut BTreeMap<String, MemoryGraphEdge>,
) {
    let mut by_resource = BTreeMap::<String, Vec<&IndexedIssue>>::new();
    for issue in issues {
        if let Some(resource) = parsed_okf_concept(config, issue)
            .and_then(|concept| concept.frontmatter.resource)
        {
            by_resource.entry(resource).or_default().push(issue);
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

fn memory_graph_communities_from_issues(issues: &[IndexedIssue]) -> Vec<MemoryGraphCommunity> {
    let mut by_area = BTreeMap::<String, Vec<String>>::new();
    for issue in issues {
        for area in issue.areas() {
            by_area.entry(area).or_default().push(concept_node_id(issue));
        }
    }
    by_area
        .into_iter()
        .map(|(area, mut node_ids)| {
            node_ids.sort();
            MemoryGraphCommunity {
                id: format!("area:{area}"),
                label: area,
                concept_count: node_ids.len(),
                node_ids,
            }
        })
        .collect()
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
    let community_by_node = communities
        .iter()
        .flat_map(|community| {
            community
                .node_ids
                .iter()
                .map(move |node_id| (node_id.clone(), community.id.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    for node in nodes {
        node.metrics.indegree = indegree.get(&node.id).copied().unwrap_or_default();
        node.metrics.outdegree = outdegree.get(&node.id).copied().unwrap_or_default();
        node.metrics.community_id = community_by_node.get(&node.id).cloned();
    }
}
