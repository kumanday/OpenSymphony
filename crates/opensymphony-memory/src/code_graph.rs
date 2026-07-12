use crate::opensymphony_code_intel::{SymbolKind, parse_path};
use crate::opensymphony_gateway_schema::{
    code_graph::{
        CodeDiagnostic, CodeDiffBlastRadius, CodeDiffOverlay, CodeDiffSymbol,
        CodeDiffSymbolSide, CodeDiffSymbolStatus as DtoDiffStatus, CodeEdgeSummary,
        CodeFileOutline, CodeGraphAggregate, CodeGraphCommunity, CodeGraphConfidence,
        CodeGraphEdge, CodeGraphFreshness, CodeGraphIssueChip, CodeGraphMemoryChip,
        CodeGraphMode, CodeGraphNode, CodeGraphNodeKind, CodeGraphNodeMetrics, CodeGraphSnapshot,
        CodeGraphTruncation, CodeGraphUpdatedEvent, CodeIndexReport, CodeIndexStatus,
        CodeOutlineSymbol, CodeRepoList, CodeRepoSummary, CodeSpan, CodeSymbolDetail,
        CodeSymbolProvenance,
    },
};

static CODE_GRAPH_SEQUENCE_FLOOR: AtomicU64 = AtomicU64::new(0);
const CODE_GRAPH_MAX_RECORDS: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum CodeGraphProjectionError {
    #[error("unknown code repo `{0}`")]
    RepoNotFound(String),
    #[error("unknown indexed code file `{0}`")]
    FileNotFound(String),
    #[error("unknown indexed code revision `{0}`")]
    RevisionNotFound(String),
    #[error("no code symbol found for `{0}`")]
    SymbolNotFound(String),
    #[error("{0}")]
    InvalidRequest(String),
    #[error(transparent)]
    Memory(#[from] MemoryError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeGraphSnapshotOptions {
    pub mode: CodeGraphMode,
    pub path: Option<String>,
    pub symbol_key: Option<String>,
    pub depth: usize,
    pub aggregate: Option<CodeGraphAggregate>,
    pub include_stale: bool,
}

pub fn code_graph_repos(
    config: &MemoryConfig,
    include_stale: bool,
) -> Result<CodeRepoList, CodeGraphProjectionError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(CodeRepoList {
            schema_version: SchemaVersion::v1(),
            repos: Vec::new(),
        });
    };
    if !code_documents_read_model_ready(&connection, &config.index_path)? {
        return Ok(CodeRepoList {
            schema_version: SchemaVersion::v1(),
            repos: Vec::new(),
        });
    }

    let freshness = code_freshness_filter(include_stale);
    let mut statement = connection
        .prepare(&format!(
            "SELECT repo_id, path, language, indexed_at, freshness, commit_sha, CASE WHEN worktree_dirty THEN 1 ELSE 0 END FROM code_documents WHERE {freshness} ORDER BY repo_id, path"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, i64>(6)? != 0,
            ))
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;

    let mut repos = BTreeMap::<String, CodeRepoAccumulator>::new();
    for (repo_id, path, language, indexed_at, freshness, commit_sha, dirty) in rows {
        let entry = repos.entry(repo_id.clone()).or_insert_with(|| {
            CodeRepoAccumulator {
                repo_id,
                ..CodeRepoAccumulator::default()
            }
        });
        entry.paths.insert(path);
        entry.languages.insert(language);
        entry.worktree_dirty |= dirty;
        entry.has_current |= freshness == "current";
        entry.has_stale |= freshness == "stale";
        let parsed_indexed_at = parse_code_datetime(&indexed_at);
        let is_newer = match (&entry.indexed_at, &parsed_indexed_at) {
            (None, Some(_)) => true,
            (Some(old), Some(new)) => new >= old,
            _ => false,
        };
        if is_newer {
            entry.indexed_at = parsed_indexed_at;
            entry.head_revision = commit_sha;
        }
    }

    let mut summaries = Vec::new();
    for mut entry in repos.into_values() {
        entry.symbol_count =
            count_code_rows(&connection, config, "code_symbols", "symbol_key", &entry.repo_id, include_stale)?;
        entry.edge_count =
            count_code_rows(&connection, config, "code_edges", "edge_id", &entry.repo_id, include_stale)?;
        summaries.push(entry.into_summary());
    }

    Ok(CodeRepoList {
        schema_version: SchemaVersion::v1(),
        repos: summaries,
    })
}

pub fn code_graph_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    options: CodeGraphSnapshotOptions,
) -> Result<CodeGraphSnapshot, CodeGraphProjectionError> {
    ensure_code_repo(config, repo_id, options.include_stale)?;
    match options.mode {
        CodeGraphMode::Atlas => code_graph_atlas_snapshot(config, repo_id, options),
        CodeGraphMode::File => {
            let path = options.path.as_deref().ok_or_else(|| {
                CodeGraphProjectionError::InvalidRequest("file mode requires `path`".to_string())
            })?;
            code_graph_file_snapshot(config, repo_id, path, options.include_stale)
        }
        CodeGraphMode::Neighborhood => {
            let symbol_key = options.symbol_key.as_deref().ok_or_else(|| {
                CodeGraphProjectionError::InvalidRequest(
                    "neighborhood mode requires `symbol_key`".to_string(),
                )
            })?;
            code_graph_neighborhood_snapshot(
                config,
                repo_id,
                symbol_key,
                options.depth,
                options.include_stale,
            )
        }
    }
}

pub fn code_graph_symbol_detail(
    config: &MemoryConfig,
    repo_id: &str,
    symbol_key: &str,
    include_stale: bool,
    access: MemoryGraphAccess,
) -> Result<CodeSymbolDetail, CodeGraphProjectionError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::SymbolNotFound(symbol_key.to_string()));
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)? {
        return Err(CodeGraphProjectionError::SymbolNotFound(symbol_key.to_string()));
    }
    let Some(symbol) = query_code_symbol_by_key(&connection, symbol_key, !include_stale)? else {
        return Err(CodeGraphProjectionError::SymbolNotFound(symbol_key.to_string()));
    };
    if symbol.repo_id != repo_id {
        return Err(CodeGraphProjectionError::SymbolNotFound(symbol_key.to_string()));
    }
    let diagnostics = query_symbol_diagnostics(&connection, config, &symbol, include_stale)?;
    let edge_summary = query_symbol_edge_summary(&connection, &symbol, include_stale)?;
    let (related_issues, related_memory_concepts) =
        code_graph_related_memory(config, repo_id, &symbol, access)?;

    Ok(CodeSymbolDetail {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        symbol_key: symbol.symbol_key.clone(),
        symbol_id: symbol.symbol_id.clone(),
        kind: symbol.kind.clone(),
        name: symbol.name.clone(),
        path_display: symbol.path.clone(),
        language: symbol.language.clone(),
        container_chain: symbol.container_chain.clone(),
        signature: symbol.signature.clone(),
        span: span_from_symbol(&symbol),
        selection_span: selection_span_from_symbol(&symbol),
        freshness: freshness_from_str(&symbol.freshness),
        provenance: CodeSymbolProvenance {
            commit_sha: symbol.commit_sha.clone(),
            content_sha256: symbol.content_sha256.clone(),
            snippet_sha256: symbol.snippet_sha256.clone(),
            parser_version: symbol.parser_version.clone(),
            query_pack_version: symbol.query_pack_version.clone(),
            indexed_at: parse_code_datetime(&symbol.indexed_at),
        },
        diagnostics,
        edge_summary,
        source_snippet: None,
        related_issues,
        related_memory_concepts,
    })
}

fn code_graph_related_memory(
    config: &MemoryConfig,
    repo_id: &str,
    symbol: &CodeSymbolRecord,
    access: MemoryGraphAccess,
) -> Result<(Vec<CodeGraphIssueChip>, Vec<CodeGraphMemoryChip>), CodeGraphProjectionError> {
    // ponytail: scan the accessible issue catalog per selected symbol; add a
    // source-ref index when cross-graph chip latency needs to scale further.
    let issues = accessible_issues(config, access).map_err(|error| match error {
        MemoryGraphProjectionError::Memory(error) => error,
        other => MemoryError::InvalidInput(other.to_string()),
    })?;
    let mut related_issues = Vec::new();
    let mut related_memory_concepts = Vec::new();
    for issue in issues {
        let source_match = issue.source_refs.iter().any(|source| {
            let repo_matches = source.repo_id.as_deref() == Some(repo_id);
            repo_matches && (source.symbol_key.as_deref() == Some(symbol.symbol_key.as_str())
                || (source.kind == "path" && source.id == symbol.path)
                || (source.kind == "code-symbol" && code_symbol_source_ref_matches(source, symbol)))
        });
        let scope_match = issue.scope_refs.iter().any(|scope| {
            matches!(scope.kind, KnowledgeScopeKind::CodePath)
                && (scope.id == symbol.path
                    || symbol.path.starts_with(&format!("{}/", scope.id)))
        });
        let citation_match = issue.citations.iter().any(|citation| {
            citation.target.contains(&symbol.symbol_key) || citation.target.contains(&symbol.path)
        });
        if !(source_match || scope_match || citation_match) {
            continue;
        }
        let freshness = freshness_from_str(issue.freshness.as_str());
        if issue.concept_type == "issue-capsule" {
            related_issues.push(CodeGraphIssueChip {
                issue_key: issue.issue_key.clone(),
                title: redact_for_dto(config, &issue.title),
                state: issue.state.clone().map(|state| redact_for_dto(config, &state)),
                url: None,
                freshness,
            });
        }
        related_memory_concepts.push(CodeGraphMemoryChip {
            bundle_id: DEFAULT_MEMORY_GRAPH_BUNDLE_ID.to_string(),
            concept_id: issue.concept_id.clone(),
            title: redact_for_dto(config, &issue.title),
            visibility: issue.visibility.as_str().to_string(),
            freshness,
        });
    }
    related_issues.sort_by(|left, right| left.issue_key.cmp(&right.issue_key));
    related_memory_concepts.sort_by(|left, right| left.concept_id.cmp(&right.concept_id));
    Ok((related_issues, related_memory_concepts))
}

fn code_symbol_source_ref_matches(source: &MemorySourceRef, symbol: &CodeSymbolRecord) -> bool {
    let Some(span) = source.id.strip_prefix(&format!("{}:", symbol.path)) else {
        return false;
    };
    code_symbol_span_matches(
        span,
        symbol.start_line,
        symbol.start_col,
        symbol.end_line,
        symbol.end_col,
    )
}

fn code_symbol_span_matches(
    span: &str,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
) -> bool {
    span == format!(
        "{}:{}-{}:{}",
        start_line, start_col, end_line, end_col
    )
}

pub fn code_graph_diff_overlay(
    config: &MemoryConfig,
    repo_id: &str,
    base_revision: &str,
    head_revision: &str,
    max_records: usize,
) -> Result<CodeDiffOverlay, CodeGraphProjectionError> {
    ensure_code_repo(config, repo_id, true)?;
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::RepoNotFound(repo_id.to_string()));
    };
    for revision in [base_revision, head_revision] {
        if !code_revision_indexed(&connection, config, repo_id, revision)? {
            return Err(CodeGraphProjectionError::RevisionNotFound(
                revision.to_string(),
            ));
        }
    }
    let comparison = compare_code_symbols(
        config,
        repo_id,
        base_revision,
        head_revision,
        max_records.max(1),
    )?;
    let blast_radius = query_diff_blast_radius(&connection, config, &comparison.diffs)?;
    let unanalyzed_files =
        query_unanalyzed_diff_files(&connection, config, repo_id, base_revision, head_revision)?;
    let mut added_symbols = Vec::new();
    let mut removed_symbols = Vec::new();
    let mut modified_symbols = Vec::new();

    for diff in comparison.diffs {
        let dto = CodeDiffSymbol {
            symbol_key: diff.symbol_key,
            status: match diff.status {
                CodeSymbolDiffStatus::Added => DtoDiffStatus::Added,
                CodeSymbolDiffStatus::Removed => DtoDiffStatus::Removed,
                CodeSymbolDiffStatus::Modified => DtoDiffStatus::Modified,
            },
            before: diff.base.as_ref().map(diff_side_from_symbol),
            after: diff.head.as_ref().map(diff_side_from_symbol),
        };
        match dto.status {
            DtoDiffStatus::Added => added_symbols.push(dto),
            DtoDiffStatus::Removed => removed_symbols.push(dto),
            DtoDiffStatus::Modified => modified_symbols.push(dto),
        }
    }

    Ok(CodeDiffOverlay {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        base_revision: base_revision.to_string(),
        head_revision: head_revision.to_string(),
        added_symbols,
        removed_symbols,
        modified_symbols,
        blast_radius,
        unanalyzed_files,
        truncation: truncation(comparison.dropped_records, 0, "diff symbols capped"),
        generated_at: Utc::now(),
    })
}

pub fn code_graph_index_report(
    config: &MemoryConfig,
    repo_id: &str,
) -> Result<CodeIndexReport, CodeGraphProjectionError> {
    let indexed_at = Utc::now();
    let repo = code_graph_repos(config, true)?
        .repos
        .into_iter()
        .find(|repo| repo.repo_id == repo_id);
    let status = if repo.is_some() {
        CodeIndexStatus::Completed
    } else {
        CodeIndexStatus::Unavailable
    };
    let head_revision = repo.and_then(|repo| repo.head_revision);
    let counts = code_index_counts(config, repo_id)?;
    Ok(CodeIndexReport {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        status,
        head_revision: head_revision.clone(),
        parsed_files: counts.documents,
        persisted_documents: counts.documents,
        persisted_symbols: counts.symbols,
        persisted_edges: counts.edges,
        persisted_diagnostics: counts.diagnostics,
        stale_rows: counts.stale_rows,
        skipped_files: Vec::new(),
        diagnostics: Vec::new(),
        cursor: code_graph_cursor(repo_id, indexed_at),
        indexed_at,
    })
}

pub fn code_graph_updated_event(
    _config: &MemoryConfig,
    repo_id: &str,
    head_revision: Option<String>,
) -> Result<CodeGraphUpdatedEvent, CodeGraphProjectionError> {
    let updated_at = Utc::now();
    Ok(CodeGraphUpdatedEvent {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        head_revision,
        cursor: code_graph_cursor(repo_id, updated_at),
        updated_at,
    })
}

pub fn code_file_outline_from_source(
    run_id: &str,
    repo_id: Option<String>,
    path: &str,
    source: &str,
) -> Result<CodeFileOutline, MemoryError> {
    let safe_path = normalize_code_path(path)
        .map_err(|message| MemoryError::InvalidInput(format!("invalid code path: {message}")))?;
    let summary = match parse_path(&safe_path, source) {
        Ok(summary) => summary,
        Err(CodeIntelError::UnsupportedLanguage { .. }) => {
            return Ok(empty_code_file_outline(run_id, repo_id, safe_path));
        }
        Err(error) => return Err(error.into()),
    };
    let language = summary.source.language.id().to_string();
    let parser_version = format!(
        "{}:{}",
        summary.versions.grammar, summary.versions.tree_sitter
    );
    let symbols = summary
        .symbols
        .iter()
        .map(|symbol| CodeIntelSymbolInput {
            kind: symbol_kind_id(&symbol.kind).to_string(),
            name: symbol.name.clone(),
            container_chain: symbol.container_chain.clone(),
            signature: None,
            start_line: symbol.span.start_line,
            start_col: symbol.span.start_column,
            end_line: symbol.span.end_line,
            end_col: symbol.span.end_column,
            start_byte: symbol.span.start_byte,
            end_byte: symbol.span.end_byte,
            selection_start_line: symbol.span.start_line,
            selection_end_line: symbol.span.end_line,
            snippet_sha256: code_row_id(&[&safe_path, &symbol.name, &symbol.rendered_span]),
        })
        .collect::<Vec<_>>();
    let document = CodeIntelDocumentInput {
        path: safe_path.clone().into(),
        language,
        content_sha256: summary.source.sha256,
        parser_id: summary.versions.provider,
        parser_version,
        query_pack_version: summary.versions.query_pack,
        byte_len: source.len(),
        line_count: source.lines().count(),
        symbols,
        edges: Vec::new(),
        diagnostics: Vec::new(),
    };
    let repo_key = repo_id.as_deref().unwrap_or("run");
    let prepared = prepare_code_symbols(repo_key, None, true, &safe_path, &document);
    let outline = prepared
        .into_iter()
        .map(|prepared| {
            let symbol = prepared.symbol;
            CodeOutlineSymbol {
                symbol_key: prepared.symbol_key,
                name: symbol.name.clone(),
                kind: symbol.kind.clone(),
                path: safe_path.clone(),
                span: CodeSpan {
                    start_line: symbol.start_line,
                    start_col: symbol.start_col,
                    end_line: symbol.end_line,
                    end_col: symbol.end_col,
                },
                selection_span: CodeSpan {
                    start_line: symbol.selection_start_line,
                    start_col: symbol.start_col,
                    end_line: symbol.selection_end_line,
                    end_col: symbol.end_col,
                },
                container_chain: symbol.container_chain.clone(),
            }
        })
        .collect();

    Ok(CodeFileOutline {
        schema_version: SchemaVersion::v1(),
        run_id: run_id.to_string(),
        repo_id,
        path: safe_path,
        symbols: outline,
        generated_at: Utc::now(),
    })
}

fn empty_code_file_outline(run_id: &str, repo_id: Option<String>, path: String) -> CodeFileOutline {
    CodeFileOutline {
        schema_version: SchemaVersion::v1(),
        run_id: run_id.to_string(),
        repo_id,
        path,
        symbols: Vec::new(),
        generated_at: Utc::now(),
    }
}

#[derive(Default)]
struct CodeRepoAccumulator {
    repo_id: String,
    paths: BTreeSet<String>,
    languages: BTreeSet<String>,
    has_current: bool,
    has_stale: bool,
    indexed_at: Option<DateTime<Utc>>,
    head_revision: Option<String>,
    worktree_dirty: bool,
    symbol_count: usize,
    edge_count: usize,
}

impl CodeRepoAccumulator {
    fn into_summary(self) -> CodeRepoSummary {
        CodeRepoSummary {
            display_root: self.repo_id.clone(),
            repo_id: self.repo_id,
            languages: self.languages.into_iter().collect(),
            document_count: self.paths.len(),
            symbol_count: self.symbol_count,
            edge_count: self.edge_count,
            freshness: if self.has_current {
                CodeGraphFreshness::Current
            } else if self.has_stale {
                CodeGraphFreshness::Stale
            } else {
                CodeGraphFreshness::Unknown
            },
            indexed_at: self.indexed_at,
            head_revision: self.head_revision,
            worktree_dirty: self.worktree_dirty,
        }
    }
}

fn code_graph_atlas_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    options: CodeGraphSnapshotOptions,
) -> Result<CodeGraphSnapshot, CodeGraphProjectionError> {
    if options.aggregate == Some(CodeGraphAggregate::Community) {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "community aggregation is not available for code graph atlas snapshots".to_string(),
        ));
    }
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::RepoNotFound(repo_id.to_string()));
    };
    let freshness = code_freshness_filter(options.include_stale);
    let total_documents = count_code_documents(&connection, config, repo_id, options.include_stale)?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT path, language, freshness FROM (
                SELECT path, language, freshness,
                    ROW_NUMBER() OVER (
                        PARTITION BY path
                        ORDER BY CASE WHEN freshness = 'current' THEN 0 ELSE 1 END,
                            indexed_at DESC,
                            content_sha256
                    ) AS row_rank
                FROM code_documents
                WHERE repo_id = ? AND {freshness}
            ) ranked
            WHERE row_rank = 1
            ORDER BY path
            LIMIT {}",
            CODE_GRAPH_MAX_RECORDS + 1
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let docs = statement
        .query_map(params![repo_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let dropped = total_documents.saturating_sub(CODE_GRAPH_MAX_RECORDS);
    let mut nodes = BTreeMap::new();
    let mut edges = BTreeMap::new();
    for (path, language, freshness) in docs.into_iter().take(CODE_GRAPH_MAX_RECORDS) {
        insert_path_nodes(&mut nodes, &mut edges, &path, Some(language), freshness_from_str(&freshness));
    }
    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let edges = edges.into_values().collect::<Vec<_>>();
    apply_code_node_metrics(&mut nodes, &edges);
    Ok(snapshot(
        repo_id,
        CodeGraphMode::Atlas,
        nodes,
        edges,
        options.include_stale,
        options.aggregate,
        truncation(dropped, 0, "atlas documents capped"),
    ))
}

fn code_graph_file_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    raw_path: &str,
    include_stale: bool,
) -> Result<CodeGraphSnapshot, CodeGraphProjectionError> {
    let path = normalize_code_path(raw_path).map_err(CodeGraphProjectionError::InvalidRequest)?;
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::RepoNotFound(repo_id.to_string()));
    };
    let Some((language, freshness)) =
        query_code_document(&connection, config, repo_id, &path, include_stale)?
    else {
        return Err(CodeGraphProjectionError::FileNotFound(path));
    };
    let symbols = query_file_symbols(&connection, config, repo_id, &path, include_stale)?;
    let selected_symbol_revisions = symbols
        .iter()
        .map(|symbol| {
            (
                symbol.symbol_key.clone(),
                SelectedSymbolRevision {
                    commit_sha: symbol.commit_sha.clone(),
                    freshness: symbol.freshness.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let edges = query_file_edges(&connection, config, repo_id, &path, include_stale)?;
    let mut nodes = BTreeMap::new();
    let mut graph_edges = BTreeMap::new();
    let file_id = file_node_id(&path);
    nodes.insert(
        file_id.clone(),
        file_node(&path, Some(language), freshness_from_str(&freshness)),
    );
    for symbol in symbols {
        let node = symbol_node(&connection, config, &symbol, include_stale)?;
        insert_code_graph_edge(
            &mut graph_edges,
            "contains".to_string(),
            file_id.clone(),
            node.id.clone(),
            CodeGraphConfidence::Exact,
            false,
            None,
        );
        nodes.entry(node.id.clone()).or_insert(node);
    }
    for edge in edges {
        if !edge_matches_selected_symbol_revisions(&edge, &selected_symbol_revisions) {
            continue;
        }
        let source_id = edge.source_symbol_key.as_ref().map(symbol_node_id);
        let Some(source_id) = source_id else {
            continue;
        };
        if !nodes.contains_key(&source_id) {
            continue;
        }
        let target_id = edge
            .target_symbol_key
            .as_ref()
            .map(symbol_node_id)
            .unwrap_or_else(|| format!("hint:{}", edge.edge_id));
        if edge.target_symbol_key.is_some() && !nodes.contains_key(&target_id) {
            continue;
        }
        if edge.target_symbol_key.is_none() {
            nodes
                .entry(target_id.clone())
                .or_insert_with(|| hint_node(&target_id, edge.target_hint.as_deref()));
        }
        insert_code_graph_edge(
            &mut graph_edges,
            edge.edge_kind,
            source_id,
            target_id,
            confidence_from_str(&edge.confidence),
            edge.unresolved,
            edge.target_hint,
        );
    }
    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let edges = graph_edges.into_values().collect::<Vec<_>>();
    apply_code_node_metrics(&mut nodes, &edges);
    Ok(snapshot(
        repo_id,
        CodeGraphMode::File,
        nodes,
        edges,
        include_stale,
        None,
        CodeGraphTruncation::default(),
    ))
}

fn code_graph_neighborhood_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    symbol_key: &str,
    depth: usize,
    include_stale: bool,
) -> Result<CodeGraphSnapshot, CodeGraphProjectionError> {
    let Some(neighborhood) = code_symbol_neighborhood_with_stale(
        config,
        symbol_key,
        depth.max(1),
        CODE_GRAPH_MAX_RECORDS,
        include_stale,
    )?
    else {
        return Err(CodeGraphProjectionError::SymbolNotFound(symbol_key.to_string()));
    };
    if neighborhood.center.repo_id != repo_id {
        return Err(CodeGraphProjectionError::SymbolNotFound(symbol_key.to_string()));
    }
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::RepoNotFound(repo_id.to_string()));
    };
    let mut nodes = Vec::new();
    for symbol in &neighborhood.symbols {
        nodes.push(symbol_node(&connection, config, symbol, include_stale)?);
    }
    let mut edges = neighborhood
        .edges
        .into_iter()
        .filter_map(|edge| {
            let source_id = edge.source_symbol_key.as_ref().map(symbol_node_id)?;
            let target_id = edge
                .target_symbol_key
                .as_ref()
                .map(symbol_node_id)
                .unwrap_or_else(|| format!("hint:{}", edge.edge_id));
            Some(CodeGraphEdge {
                id: edge.edge_id,
                kind: edge.edge_kind,
                source_id,
                target_id,
                confidence: confidence_from_str(&edge.confidence),
                unresolved: edge.unresolved,
                target_hint: edge.target_hint,
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    let mut node_ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    for edge in &edges {
        if edge.unresolved && !node_ids.contains(&edge.target_id) {
            nodes.push(hint_node(&edge.target_id, edge.target_hint.as_deref()));
            node_ids.insert(edge.target_id.clone());
        }
    }
    apply_code_node_metrics(&mut nodes, &edges);
    Ok(snapshot(
        repo_id,
        CodeGraphMode::Neighborhood,
        nodes,
        edges,
        include_stale,
        None,
        truncation(
            neighborhood.dropped_nodes,
            neighborhood.dropped_edges,
            "neighborhood records capped",
        ),
    ))
}

fn ensure_code_repo(
    config: &MemoryConfig,
    repo_id: &str,
    include_stale: bool,
) -> Result<(), CodeGraphProjectionError> {
    if code_graph_repos(config, include_stale)?
        .repos
        .iter()
        .any(|repo| repo.repo_id == repo_id)
    {
        Ok(())
    } else {
        Err(CodeGraphProjectionError::RepoNotFound(repo_id.to_string()))
    }
}

fn snapshot(
    repo_id: &str,
    mode: CodeGraphMode,
    nodes: Vec<CodeGraphNode>,
    edges: Vec<CodeGraphEdge>,
    include_stale: bool,
    aggregate: Option<CodeGraphAggregate>,
    truncation: CodeGraphTruncation,
) -> CodeGraphSnapshot {
    let generated_at = Utc::now();
    let mut filters_applied = Vec::new();
    if include_stale {
        filters_applied.push("include_stale:true".to_string());
    }
    if let Some(aggregate) = aggregate {
        filters_applied.push(format!("aggregate:{aggregate:?}").to_ascii_lowercase());
    }
    CodeGraphSnapshot {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        mode,
        cursor: code_graph_cursor(repo_id, generated_at),
        nodes,
        edges,
        communities: Vec::<CodeGraphCommunity>::new(),
        truncation,
        filters_applied,
        generated_at,
    }
}

fn query_file_symbols(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    path: &str,
    include_stale: bool,
) -> Result<Vec<CodeSymbolRecord>, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let mut statement = connection
        .prepare(&format!(
            "SELECT {CODE_SYMBOL_SELECT} FROM code_symbols WHERE repo_id = ? AND path = ? AND {freshness} AND symbol_key != '' ORDER BY CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC, start_line, start_col, end_line DESC, end_col DESC, symbol_key, symbol_id"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut seen = BTreeSet::new();
    statement
        .query_map(params![repo_id, path], code_symbol_from_row)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .into_iter()
        .filter(|symbol| seen.insert(symbol.symbol_key.clone()))
        .map(|mut symbol| {
            fill_container_chain(connection, &mut symbol)?;
            Ok(symbol)
        })
        .collect()
}

fn query_code_document(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    path: &str,
    include_stale: bool,
) -> Result<Option<(String, String)>, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let mut statement = connection
        .prepare(&format!(
            "SELECT language, freshness FROM code_documents WHERE repo_id = ? AND path = ? AND {freshness} ORDER BY CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC LIMIT 1"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = statement
        .query(params![repo_id, path])
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let Some(row) = rows.next().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?
    else {
        return Ok(None);
    };
    Ok(Some((
        row.get::<_, String>(0).map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?,
        row.get::<_, String>(1).map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?,
    )))
}

fn query_file_edges(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    path: &str,
    include_stale: bool,
) -> Result<Vec<CodeEdgeRecord>, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let mut statement = connection
        .prepare(&format!(
            "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges WHERE repo_id = ? AND path = ? AND {freshness} ORDER BY edge_kind, path, start_line, start_col, edge_id"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    statement
        .query_map(params![repo_id, path], code_edge_from_row)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SelectedSymbolRevision {
    commit_sha: Option<String>,
    freshness: String,
}

fn edge_matches_selected_symbol_revisions(
    edge: &CodeEdgeRecord,
    selected_symbols: &BTreeMap<String, SelectedSymbolRevision>,
) -> bool {
    edge_symbol_matches_selected_revision(edge, edge.source_symbol_key.as_deref(), selected_symbols)
        && edge_symbol_matches_selected_revision(
            edge,
            edge.target_symbol_key.as_deref(),
            selected_symbols,
        )
}

fn edge_symbol_matches_selected_revision(
    edge: &CodeEdgeRecord,
    symbol_key: Option<&str>,
    selected_symbols: &BTreeMap<String, SelectedSymbolRevision>,
) -> bool {
    let Some(symbol_key) = symbol_key else {
        return true;
    };
    let Some(selected) = selected_symbols.get(symbol_key) else {
        return true;
    };
    selected.commit_sha == edge.commit_sha && selected.freshness == edge.freshness
}

fn query_symbol_diagnostics(
    connection: &Connection,
    config: &MemoryConfig,
    symbol: &CodeSymbolRecord,
    include_stale: bool,
) -> Result<Vec<CodeDiagnostic>, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let mut statement = connection
        .prepare(&format!(
            "SELECT kind, severity, message, start_line, start_col, end_line, end_col FROM code_diagnostics WHERE repo_id = ? AND path = ? AND {freshness} AND content_sha256 = ? AND (commit_sha = ? OR (? IS NULL AND commit_sha IS NULL)) AND start_line <= ? AND end_line >= ? ORDER BY start_line, start_col, diagnostic_id"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    statement
        .query_map(
            params![
                &symbol.repo_id,
                &symbol.path,
                &symbol.content_sha256,
                symbol.commit_sha.as_deref(),
                symbol.commit_sha.as_deref(),
                symbol.end_line as i64,
                symbol.start_line as i64
            ],
            |row| {
                Ok(CodeDiagnostic {
                    kind: row.get(0)?,
                    severity: row.get(1)?,
                    message: row.get(2)?,
                    span: CodeSpan {
                        start_line: row.get::<_, i64>(3)? as usize,
                        start_col: row.get::<_, i64>(4)? as usize,
                        end_line: row.get::<_, i64>(5)? as usize,
                        end_col: row.get::<_, i64>(6)? as usize,
                    },
                })
            },
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
}

fn query_symbol_edge_summary(
    connection: &Connection,
    symbol: &CodeSymbolRecord,
    include_stale: bool,
) -> Result<Vec<CodeEdgeSummary>, MemoryError> {
    let mut groups = BTreeMap::<(String, CodeGraphConfidence), (usize, usize)>::new();
    for edge in query_edges_for_symbol_key_with_stale(connection, &symbol.symbol_key, include_stale)?
    {
        let key = (edge.edge_kind, confidence_from_str(&edge.confidence));
        let entry = groups.entry(key).or_default();
        entry.0 += 1;
        if edge.unresolved {
            entry.1 += 1;
        }
    }
    Ok(groups
        .into_iter()
        .map(|((kind, confidence), (count, unresolved_count))| CodeEdgeSummary {
            kind,
            confidence,
            count,
            unresolved_count,
        })
        .collect())
}

fn code_revision_indexed(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<bool, MemoryError> {
    for (table, ready) in [
        (
            "code_document_revisions",
            code_document_revisions_read_model_ready(connection, &config.index_path)?,
        ),
        (
            "code_documents",
            code_documents_read_model_ready(connection, &config.index_path)?,
        ),
        (
            "code_symbols",
            code_symbols_read_model_ready(connection, &config.index_path)?,
        ),
        (
            "code_edges",
            code_edges_read_model_ready(connection, &config.index_path)?,
        ),
        (
            "code_skipped_files",
            code_skipped_files_read_model_ready(connection, &config.index_path)?,
        ),
    ] {
        if ready && code_revision_exists_in_table(connection, config, table, repo_id, revision)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn query_diff_blast_radius(
    connection: &Connection,
    config: &MemoryConfig,
    diffs: &[CodeSymbolDiff],
) -> Result<Vec<CodeDiffBlastRadius>, MemoryError> {
    let mut radius = Vec::new();
    let changed_symbol_keys = diffs
        .iter()
        .map(|diff| diff.symbol_key.clone())
        .collect::<BTreeSet<_>>();
    for diff in diffs {
        let symbol = match diff.status {
            CodeSymbolDiffStatus::Added => None,
            CodeSymbolDiffStatus::Removed => diff.base.as_ref(),
            CodeSymbolDiffStatus::Modified => diff.head.as_ref().or(diff.base.as_ref()),
        };
        let Some(symbol) = symbol else {
            continue;
        };
        let inbound_count =
            query_retained_inbound_impact_count(connection, config, symbol, &changed_symbol_keys)?;
        if inbound_count > 0 {
            radius.push(CodeDiffBlastRadius {
                symbol_key: diff.symbol_key.clone(),
                inbound_count,
                outbound_count: 0,
            });
        }
    }
    Ok(radius)
}

fn code_revision_exists_in_table(
    connection: &Connection,
    config: &MemoryConfig,
    table: &str,
    repo_id: &str,
    revision: &str,
) -> Result<bool, MemoryError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT 1 FROM {table} WHERE repo_id = ? AND commit_sha = ? AND NOT worktree_dirty LIMIT 1"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = statement
        .query(params![repo_id, revision])
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(rows
        .next()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .is_some())
}

fn query_retained_inbound_impact_count(
    connection: &Connection,
    config: &MemoryConfig,
    symbol: &CodeSymbolRecord,
    changed_symbol_keys: &BTreeSet<String>,
) -> Result<usize, MemoryError> {
    let edge_table = if let Some(commit_sha) = symbol.commit_sha.as_deref()
        && code_edge_revision_rows_available(connection, config, &symbol.repo_id, commit_sha)?
    {
        "code_edge_revisions"
    } else {
        "code_edges"
    };
    let mut statement = connection
        .prepare(&format!(
            "SELECT DISTINCT edge_id, source_symbol_key FROM {edge_table} WHERE repo_id = ? AND target_symbol_key = ? AND NOT worktree_dirty AND (lower(edge_kind) LIKE '%call%' OR lower(edge_kind) LIKE '%reference%') AND (commit_sha = ? OR (? IS NULL AND commit_sha IS NULL))"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map(
            params![
                &symbol.repo_id,
                &symbol.symbol_key,
                symbol.commit_sha.as_deref(),
                symbol.commit_sha.as_deref(),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(rows
        .into_iter()
        .filter(|(_, source_symbol_key)| {
            source_symbol_key
                .as_ref()
                .is_none_or(|key| !changed_symbol_keys.contains(key))
        })
        .count())
}

fn code_edge_revision_rows_available(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<bool, MemoryError> {
    if !code_edge_revisions_read_model_ready(connection, &config.index_path)? {
        return Ok(false);
    }
    let mut statement = connection
        .prepare(
            "SELECT 1 FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ? AND NOT worktree_dirty LIMIT 1",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows =
        statement
            .query(params![repo_id, revision])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    Ok(rows
        .next()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .is_some())
}

fn symbol_node(
    connection: &Connection,
    config: &MemoryConfig,
    symbol: &CodeSymbolRecord,
    include_stale: bool,
) -> Result<CodeGraphNode, MemoryError> {
    let diagnostics = query_symbol_diagnostics(connection, config, symbol, include_stale)?;
    Ok(CodeGraphNode {
        id: symbol_node_id(&symbol.symbol_key),
        kind: CodeGraphNodeKind::Symbol,
        label: symbol.name.clone(),
        symbol_kind: Some(symbol.kind.clone()),
        symbol_key: Some(symbol.symbol_key.clone()),
        symbol_id: Some(symbol.symbol_id.clone()),
        path_display: Some(symbol.path.clone()),
        language: Some(symbol.language.clone()),
        container_chain: symbol.container_chain.clone(),
        signature: symbol.signature.clone(),
        span: Some(span_from_symbol(symbol)),
        selection_span: Some(selection_span_from_symbol(symbol)),
        freshness: freshness_from_str(&symbol.freshness),
        diagnostic_count: diagnostics.len(),
        diagnostic_severity: diagnostics
            .iter()
            .max_by_key(|diagnostic| diagnostic_severity_rank(&diagnostic.severity))
            .map(|diagnostic| diagnostic.severity.clone()),
        metrics: CodeGraphNodeMetrics::default(),
    })
}

fn diagnostic_severity_rank(severity: &str) -> u8 {
    match severity.trim().to_ascii_lowercase().as_str() {
        "fatal" | "error" => 4,
        "warning" | "warn" => 3,
        "info" | "information" => 2,
        "hint" | "note" => 1,
        _ => 0,
    }
}

fn insert_path_nodes(
    nodes: &mut BTreeMap<String, CodeGraphNode>,
    edges: &mut BTreeMap<String, CodeGraphEdge>,
    path: &str,
    language: Option<String>,
    freshness: CodeGraphFreshness,
) {
    let mut parent: Option<String> = None;
    let mut accumulated = Vec::<&str>::new();
    let parts = path.split('/').filter(|part| !part.is_empty()).collect::<Vec<_>>();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        accumulated.push(*part);
        let dir = accumulated.join("/");
        let id = format!("dir:{dir}");
        nodes.entry(id.clone()).or_insert_with(|| directory_node(&dir));
        if let Some(parent_id) = parent {
            insert_code_graph_edge(
                edges,
                "contains".to_string(),
                parent_id,
                id.clone(),
                CodeGraphConfidence::Exact,
                false,
                None,
            );
        }
        parent = Some(id);
    }
    let file = file_node(path, language, freshness);
    let file_id = file.id.clone();
    nodes.entry(file_id.clone()).or_insert(file);
    if let Some(parent_id) = parent {
        insert_code_graph_edge(
            edges,
            "contains".to_string(),
            parent_id,
            file_id,
            CodeGraphConfidence::Exact,
            false,
            None,
        );
    }
}

fn directory_node(path: &str) -> CodeGraphNode {
    CodeGraphNode {
        id: format!("dir:{path}"),
        kind: CodeGraphNodeKind::Directory,
        label: path.rsplit('/').next().unwrap_or(path).to_string(),
        symbol_kind: None,
        symbol_key: None,
        symbol_id: None,
        path_display: Some(path.to_string()),
        language: None,
        container_chain: Vec::new(),
        signature: None,
        span: None,
        selection_span: None,
        freshness: CodeGraphFreshness::Unknown,
        diagnostic_count: 0,
        diagnostic_severity: None,
        metrics: CodeGraphNodeMetrics::default(),
    }
}

fn file_node(
    path: &str,
    language: Option<String>,
    freshness: CodeGraphFreshness,
) -> CodeGraphNode {
    CodeGraphNode {
        id: file_node_id(path),
        kind: CodeGraphNodeKind::File,
        label: path.rsplit('/').next().unwrap_or(path).to_string(),
        symbol_kind: None,
        symbol_key: None,
        symbol_id: None,
        path_display: Some(path.to_string()),
        language,
        container_chain: Vec::new(),
        signature: None,
        span: None,
        selection_span: None,
        freshness,
        diagnostic_count: 0,
        diagnostic_severity: None,
        metrics: CodeGraphNodeMetrics::default(),
    }
}

fn hint_node(id: &str, label: Option<&str>) -> CodeGraphNode {
    CodeGraphNode {
        id: id.to_string(),
        kind: CodeGraphNodeKind::Symbol,
        label: label.unwrap_or("unresolved").to_string(),
        symbol_kind: None,
        symbol_key: None,
        symbol_id: None,
        path_display: None,
        language: None,
        container_chain: Vec::new(),
        signature: None,
        span: None,
        selection_span: None,
        freshness: CodeGraphFreshness::Unknown,
        diagnostic_count: 0,
        diagnostic_severity: None,
        metrics: CodeGraphNodeMetrics::default(),
    }
}

fn insert_code_graph_edge(
    edges: &mut BTreeMap<String, CodeGraphEdge>,
    kind: String,
    source_id: String,
    target_id: String,
    confidence: CodeGraphConfidence,
    unresolved: bool,
    target_hint: Option<String>,
) {
    let id = format!("edge:{}:{source_id}->{target_id}", edge_id_component(&kind));
    edges.entry(id.clone()).or_insert(CodeGraphEdge {
        id,
        kind,
        source_id,
        target_id,
        confidence,
        unresolved,
        target_hint,
    });
}

fn apply_code_node_metrics(nodes: &mut [CodeGraphNode], edges: &[CodeGraphEdge]) {
    let mut in_degree = BTreeMap::<String, usize>::new();
    let mut out_degree = BTreeMap::<String, usize>::new();
    for edge in edges {
        *out_degree.entry(edge.source_id.clone()).or_default() += 1;
        *in_degree.entry(edge.target_id.clone()).or_default() += 1;
    }
    for node in nodes {
        node.metrics.in_degree = in_degree.get(&node.id).copied().unwrap_or_default();
        node.metrics.out_degree = out_degree.get(&node.id).copied().unwrap_or_default();
    }
}

fn diff_side_from_symbol(symbol: &CodeSymbolRecord) -> CodeDiffSymbolSide {
    CodeDiffSymbolSide {
        symbol_id: symbol.symbol_id.clone(),
        kind: symbol.kind.clone(),
        name: symbol.name.clone(),
        path_display: symbol.path.clone(),
        container_chain: symbol.container_chain.clone(),
        span: span_from_symbol(symbol),
        freshness: freshness_from_str(&symbol.freshness),
    }
}

#[derive(Default)]
struct CodeIndexCounts {
    documents: usize,
    symbols: usize,
    edges: usize,
    diagnostics: usize,
    stale_rows: usize,
}

fn code_index_counts(config: &MemoryConfig, repo_id: &str) -> Result<CodeIndexCounts, MemoryError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(CodeIndexCounts::default());
    };
    let mut counts = CodeIndexCounts::default();
    if code_documents_read_model_ready(&connection, &config.index_path)? {
        counts.documents =
            count_code_table_rows(&connection, config, "code_documents", repo_id, "current")?;
        counts.stale_rows +=
            count_code_table_rows(&connection, config, "code_documents", repo_id, "stale")?;
    }
    if code_symbols_read_model_ready(&connection, &config.index_path)? {
        counts.symbols =
            count_code_table_rows(&connection, config, "code_symbols", repo_id, "current")?;
        counts.stale_rows +=
            count_code_table_rows(&connection, config, "code_symbols", repo_id, "stale")?;
    }
    if code_edges_read_model_ready(&connection, &config.index_path)? {
        counts.edges = count_code_table_rows(&connection, config, "code_edges", repo_id, "current")?;
        counts.stale_rows +=
            count_code_table_rows(&connection, config, "code_edges", repo_id, "stale")?;
    }
    if code_diagnostics_read_model_ready(&connection, &config.index_path)? {
        counts.diagnostics =
            count_code_table_rows(&connection, config, "code_diagnostics", repo_id, "current")?;
        counts.stale_rows +=
            count_code_table_rows(&connection, config, "code_diagnostics", repo_id, "stale")?;
    }
    Ok(counts)
}

fn count_code_table_rows(
    connection: &Connection,
    config: &MemoryConfig,
    table: &str,
    repo_id: &str,
    freshness: &str,
) -> Result<usize, MemoryError> {
    let count: i64 = connection
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE repo_id = ? AND freshness = ?"),
            params![repo_id, freshness],
            |row| row.get(0),
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(count as usize)
}

fn query_unanalyzed_diff_files(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    base_revision: &str,
    head_revision: &str,
) -> Result<Vec<String>, MemoryError> {
    if !code_documents_read_model_ready(connection, &config.index_path)? {
        return Ok(Vec::new());
    }
    let base = query_revision_documents(connection, config, repo_id, base_revision)?;
    let head = query_revision_documents(connection, config, repo_id, head_revision)?;
    let mut paths = base.keys().cloned().collect::<BTreeSet<_>>();
    paths.extend(head.keys().cloned());

    let mut unanalyzed = Vec::new();
    for path in paths {
        let changed = match (base.get(&path), head.get(&path)) {
            (Some(left), Some(right)) => left != right,
            (None, Some(_)) | (Some(_), None) => true,
            (None, None) => false,
        };
        if !changed {
            continue;
        }
        let base_unanalyzed = match base.get(&path) {
            Some(document) if !document.analyzed => true,
            Some(document) => !revision_path_has_symbols(
                connection,
                config,
                repo_id,
                base_revision,
                &path,
                document,
            )?,
            None => false,
        };
        let head_unanalyzed = match head.get(&path) {
            Some(document) if !document.analyzed => true,
            Some(document) => !revision_path_has_symbols(
                connection,
                config,
                repo_id,
                head_revision,
                &path,
                document,
            )?,
            None => false,
        };
        if base_unanalyzed || head_unanalyzed {
            unanalyzed.push(path);
        }
    }
    Ok(unanalyzed)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RevisionDocumentKey {
    content_sha256: String,
    parser_version: String,
    query_pack_version: String,
    analyzed: bool,
}

fn query_revision_documents(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<BTreeMap<String, RevisionDocumentKey>, MemoryError> {
    let mut documents = BTreeMap::new();
    if code_document_revisions_read_model_ready(connection, &config.index_path)? {
        documents = query_revision_documents_from_table(
            connection,
            config,
            "code_document_revisions",
            repo_id,
            revision,
        )?;
    }
    for (path, document_key) in
        query_revision_documents_from_table(connection, config, "code_documents", repo_id, revision)?
    {
        documents.entry(path).or_insert(document_key);
    }
    for (path, document_key) in query_revision_skipped_files(connection, config, repo_id, revision)?
    {
        documents.entry(path).or_insert(document_key);
    }
    Ok(documents)
}

fn query_revision_documents_from_table(
    connection: &Connection,
    config: &MemoryConfig,
    table: &str,
    repo_id: &str,
    revision: &str,
) -> Result<BTreeMap<String, RevisionDocumentKey>, MemoryError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT path, content_sha256, parser_version, query_pack_version FROM {table} WHERE repo_id = ? AND commit_sha = ? AND NOT worktree_dirty ORDER BY path, CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map(params![repo_id, revision], |row| {
            let path = row.get::<_, String>(0)?;
            let content_sha256 = row.get::<_, String>(1)?;
            let parser_version = row.get::<_, String>(2)?;
            let query_pack_version = row.get::<_, String>(3)?;
            Ok((
                path,
                RevisionDocumentKey {
                    content_sha256,
                    parser_version,
                    query_pack_version,
                    analyzed: true,
                },
            ))
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut documents = BTreeMap::new();
    for (path, document_key) in rows {
        documents.entry(path).or_insert(document_key);
    }
    Ok(documents)
}

fn query_revision_skipped_files(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<BTreeMap<String, RevisionDocumentKey>, MemoryError> {
    if !code_skipped_files_read_model_ready(connection, &config.index_path)? {
        return Ok(BTreeMap::new());
    }
    let mut statement = connection
        .prepare(
            "SELECT path, content_sha256 FROM code_skipped_files WHERE repo_id = ? AND commit_sha = ? AND NOT worktree_dirty ORDER BY path, CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map(params![repo_id, revision], |row| {
            Ok((
                row.get::<_, String>(0)?,
                RevisionDocumentKey {
                    content_sha256: row.get(1)?,
                    parser_version: String::new(),
                    query_pack_version: String::new(),
                    analyzed: false,
                },
            ))
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut documents = BTreeMap::new();
    for (path, document_key) in rows {
        documents.entry(path).or_insert(document_key);
    }
    Ok(documents)
}

fn revision_path_has_symbols(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
    path: &str,
    document: &RevisionDocumentKey,
) -> Result<bool, MemoryError> {
    if !code_symbols_read_model_ready(connection, &config.index_path)? {
        return Ok(false);
    }
    let mut statement = connection
        .prepare(
            "SELECT 1 FROM code_symbols WHERE repo_id = ? AND commit_sha = ? AND path = ? AND content_sha256 = ? AND parser_version = ? AND query_pack_version = ? AND symbol_key != '' AND NOT worktree_dirty LIMIT 1",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = statement
        .query(params![
            repo_id,
            revision,
            path,
            document.content_sha256,
            document.parser_version,
            document.query_pack_version,
        ])
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(rows
        .next()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .is_some())
}

fn count_code_rows(
    connection: &Connection,
    config: &MemoryConfig,
    table: &str,
    distinct_key: &str,
    repo_id: &str,
    include_stale: bool,
) -> Result<usize, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let count: i64 = connection
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT {distinct_key}) FROM {table} WHERE repo_id = ? AND {freshness}"
            ),
            params![repo_id],
            |row| row.get(0),
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(count as usize)
}

fn count_code_documents(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    include_stale: bool,
) -> Result<usize, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let count: i64 = connection
        .query_row(
            &format!("SELECT COUNT(DISTINCT path) FROM code_documents WHERE repo_id = ? AND {freshness}"),
            params![repo_id],
            |row| row.get(0),
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(count as usize)
}

fn code_documents_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_documents",
        &[
            "repo_id",
            "path",
            "language",
            "indexed_at",
            "freshness",
            "commit_sha",
            "worktree_dirty",
        ],
    )
}

fn code_document_revisions_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_document_revisions",
        &[
            "repo_id",
            "commit_sha",
            "path",
            "content_sha256",
            "parser_version",
            "query_pack_version",
            "worktree_dirty",
        ],
    )
}

fn code_edge_revisions_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_edge_revisions",
        &[
            "edge_id",
            "repo_id",
            "commit_sha",
            "target_symbol_key",
            "source_symbol_key",
            "edge_kind",
            "worktree_dirty",
        ],
    )
}

fn code_skipped_files_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_skipped_files",
        &[
            "repo_id",
            "commit_sha",
            "path",
            "content_sha256",
            "worktree_dirty",
        ],
    )
}

fn code_diagnostics_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_diagnostics",
        &["repo_id", "path", "freshness", "commit_sha", "worktree_dirty"],
    )
}

fn code_freshness_filter(include_stale: bool) -> &'static str {
    if include_stale {
        "1 = 1"
    } else {
        "freshness = 'current'"
    }
}

fn normalize_code_path(raw: &str) -> Result<String, String> {
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => {
                parts.push(part.to_string_lossy().to_string());
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                return Err("parent traversal is not allowed".to_string());
            }
            _ => return Err("unsupported path component".to_string()),
        }
    }
    if parts.is_empty() {
        Err("path must not be empty".to_string())
    } else {
        Ok(parts.join("/"))
    }
}

fn span_from_symbol(symbol: &CodeSymbolRecord) -> CodeSpan {
    CodeSpan {
        start_line: symbol.start_line,
        start_col: symbol.start_col,
        end_line: symbol.end_line,
        end_col: symbol.end_col,
    }
}

fn selection_span_from_symbol(symbol: &CodeSymbolRecord) -> CodeSpan {
    CodeSpan {
        start_line: symbol.selection_start_line,
        start_col: symbol.start_col,
        end_line: symbol.selection_end_line,
        end_col: symbol.end_col,
    }
}

fn freshness_from_str(value: &str) -> CodeGraphFreshness {
    match value {
        "current" => CodeGraphFreshness::Current,
        "stale" => CodeGraphFreshness::Stale,
        _ => CodeGraphFreshness::Unknown,
    }
}

fn confidence_from_str(value: &str) -> CodeGraphConfidence {
    match value {
        "exact" => CodeGraphConfidence::Exact,
        "heuristic" => CodeGraphConfidence::Heuristic,
        _ => CodeGraphConfidence::Syntactic,
    }
}

fn symbol_kind_id(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "module",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Type => "type",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
        SymbolKind::Constructor => "constructor",
        SymbolKind::Field => "field",
        SymbolKind::Variable => "variable",
        SymbolKind::Constant => "constant",
        SymbolKind::Test => "test",
        SymbolKind::Document => "document",
    }
}

fn truncation(nodes_dropped: usize, edges_dropped: usize, reason: &str) -> CodeGraphTruncation {
    if nodes_dropped > 0 || edges_dropped > 0 {
        CodeGraphTruncation {
            nodes_dropped,
            edges_dropped,
            reason: Some(reason.to_string()),
        }
    } else {
        CodeGraphTruncation::default()
    }
}

fn symbol_node_id(symbol_key: impl AsRef<str>) -> String {
    format!("sym:{}", symbol_key.as_ref())
}

fn file_node_id(path: &str) -> String {
    format!("file:{path}")
}

fn parse_code_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn code_graph_cursor(repo_id: &str, timestamp: DateTime<Utc>) -> StreamCursor {
    StreamCursor::new(
        code_graph_sequence(timestamp),
        format!("code-graph:{repo_id}"),
    )
}

fn code_graph_sequence(timestamp: DateTime<Utc>) -> u64 {
    let candidate = timestamp
        .timestamp_nanos_opt()
        .unwrap_or_else(|| timestamp.timestamp_millis().saturating_mul(1_000_000))
        .max(0) as u64;
    let mut previous = CODE_GRAPH_SEQUENCE_FLOOR.load(Ordering::Relaxed);
    loop {
        let next = candidate.max(previous.saturating_add(1));
        match CODE_GRAPH_SEQUENCE_FLOOR.compare_exchange_weak(
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

#[cfg(test)]
mod code_graph_tests {
    use super::code_symbol_span_matches;

    #[test]
    fn legacy_code_symbol_refs_match_only_their_exact_span() {
        assert!(code_symbol_span_matches("10:1-12:2", 10, 1, 12, 2));
        assert!(!code_symbol_span_matches("10:1-12:2", 20, 1, 22, 2));
        assert!(!code_symbol_span_matches("10:1-12:2", 10, 1, 12, 3));
    }
}
