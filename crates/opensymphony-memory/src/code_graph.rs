use crate::opensymphony_code_intel::{
    AstDiagnosticKind, CaptureRecord, SymbolKind, current_parser_versions, detect_language,
    parse_path, skipped_directory_name,
};
use crate::opensymphony_gateway_schema::{
    code_graph::{
        CodeDiagnostic, CodeDiffBlastRadius, CodeDiffBlastRadiusEntry, CodeDiffConnectionScope,
        CodeDiffCountByConfidence, CodeDiffCountByKind, CodeDiffEdge, CodeDiffEdgeSide,
        CodeDiffEdgeStatus, CodeDiffModuleConnection, CodeDiffModuleConnectionSide,
        CodeDiffOverlay, CodeDiffSymbol, CodeDiffSymbolSide,
        CodeDiffSymbolStatus as DtoDiffStatus, CodeEdgeSummary,
        CodeFileOutline, CodeGraphAggregate, CodeGraphCommunity, CodeGraphConfidence,
        CodeGraphEdge, CodeGraphFreshness, CodeGraphIssueChip, CodeGraphMemoryChip,
        CodeGraphMode, CodeGraphNode, CodeGraphNodeKind, CodeGraphNodeMetrics, CodeGraphSnapshot,
        CodeGraphTruncation, CodeGraphUpdatedEvent, CodeIndexReport, CodeIndexStatus,
        CodeOutlineSymbol, CodeRepoList, CodeRepoSummary, CodeSpan, CodeSymbolDetail,
        CodeSymbolProvenance,
    },
};
use duckdb::OptionalExt;
use url::Url;
use std::sync::{Mutex, OnceLock};

static CODE_GRAPH_SEQUENCE_FLOOR: AtomicU64 = AtomicU64::new(0);
static CODE_INDEX_WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
const CODE_GRAPH_MAX_RECORDS: usize = 500;
const DEFAULT_CODE_INDEX_BRANCH: &str = "develop";

#[derive(Debug, thiserror::Error)]
pub enum CodeGraphProjectionError {
    #[error("code graph index is unavailable")]
    IndexUnavailable,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeWorkspaceOverlay {
    pub run_id: String,
    pub base_revision: String,
    pub head_revision: String,
    pub workspace_content_digest: String,
    pub base_symbols: BTreeMap<String, CodeSymbolRecord>,
    pub base_paths: BTreeSet<String>,
    pub base_edges: Vec<CodeEdgeRecord>,
    pub symbols: BTreeMap<String, CodeSymbolRecord>,
    pub edges: Vec<CodeEdgeRecord>,
    pub changed_paths: BTreeSet<String>,
    pub tombstones: BTreeSet<String>,
    pub unanalyzed_files: Vec<String>,
    pub diagnostics: BTreeMap<String, Vec<CodeDiagnostic>>,
    pub diagnostic_metadata: BTreeMap<String, (String, String)>,
    pub retrieval_dropped: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeGraphContextQuery {
    pub repo_id: String,
    pub query: Option<String>,
    pub path: Option<String>,
    pub symbol: Option<String>,
    pub depth: usize,
    pub limit: usize,
}

/// Return bounded, source-cited discovery evidence from the indexed graph.
///
/// The optional overlay is already resolved by the caller so this read path
/// cannot accept a client filesystem root or accidentally persist workspace
/// text. Changed paths in the overlay replace the baseline maps before the
/// selectors and neighborhood traversal run.
pub fn code_graph_context(
    config: &MemoryConfig,
    options: CodeGraphContextQuery,
    overlay: Option<&CodeWorkspaceOverlay>,
) -> Result<serde_json::Value, CodeGraphProjectionError> {
    if options.query.as_deref().is_none_or(|value| value.trim().is_empty())
        && options.path.as_deref().is_none_or(|value| value.trim().is_empty())
        && options
            .symbol
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "code graph context requires query, path, or symbol".to_string(),
        ));
    }
    ensure_code_repo(config, &options.repo_id, false)?;

    let path = options
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(normalize_code_path)
        .transpose()
        .map_err(CodeGraphProjectionError::InvalidRequest)?;
    let query = options
        .query
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase());
    let symbol_selector = options
        .symbol
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase());
    let base_revision = match overlay {
        Some(overlay) => Some(overlay.base_revision.clone()),
        None => code_graph_repos(config, false)?
            .repos
            .into_iter()
            .find(|repo| repo.repo_id == options.repo_id)
            .and_then(|repo| repo.head_revision),
    }
    .ok_or(CodeGraphProjectionError::IndexUnavailable)?;
    let depth = options.depth.min(8);
    let limit = options.limit.clamp(1, CODE_GRAPH_MAX_RECORDS);
    let connection = open_existing_index_read_only(config)?;
    let baseline_query = overlay.is_none();
    let (mut symbols, mut edges, mut query_dropped) = if let Some(overlay) = overlay {
        let mut symbols = overlay.symbols.clone();
        let unavailable_paths = overlay
            .unanalyzed_files
            .iter()
            .filter(|path| overlay.changed_paths.contains(*path))
            .cloned()
            .collect::<BTreeSet<_>>();
        let unavailable_keys = symbols
            .values()
            .filter(|symbol| unavailable_paths.contains(&symbol.path))
            .map(|symbol| symbol.symbol_key.clone())
            .collect::<BTreeSet<_>>();
        symbols.retain(|_, symbol| !unavailable_paths.contains(&symbol.path));
        let edges = overlay
            .edges
            .iter()
            .filter(|edge| {
                !unavailable_paths.contains(&edge.path)
                    && edge
                        .source_symbol_key
                        .as_ref()
                        .is_none_or(|key| !unavailable_keys.contains(key))
                    && edge
                        .target_symbol_key
                        .as_ref()
                        .is_none_or(|key| !unavailable_keys.contains(key))
            })
            .cloned()
            .collect();
        (symbols, edges, overlay.retrieval_dropped)
    } else {
        let Some(connection) = connection.as_ref() else {
            return Err(CodeGraphProjectionError::IndexUnavailable);
        };
        let (symbols, dropped) = query_context_symbols_for_revision(
            config,
            connection,
            &options.repo_id,
            &base_revision,
            CodeGraphContextSelectors {
                path: path.as_deref(),
                query: query.as_deref(),
                symbol: symbol_selector.as_deref(),
            },
            limit,
        )?;
        (symbols, Vec::new(), dropped)
    };
    let mut selected = BTreeMap::<String, (CodeSymbolRecord, &'static str)>::new();
    let mut dropped_direct_matches = 0;
    for candidate in symbols.values().filter(|candidate| {
        let path_matches = path.as_deref().is_none_or(|path| {
            candidate.path == path || candidate.path.starts_with(&format!("{path}/"))
        });
        let query_matches = query.as_deref().is_none_or(|query| {
            candidate.name.to_ascii_lowercase().contains(query)
                || candidate.path.to_ascii_lowercase().contains(query)
                || candidate
                    .container_chain
                    .iter()
                    .any(|container| container.to_ascii_lowercase().contains(query))
        });
        let symbol_matches = symbol_selector.as_deref().is_none_or(|selector| {
            candidate.symbol_key.to_ascii_lowercase() == selector
                || candidate.name.to_ascii_lowercase() == selector
                || candidate.name.to_ascii_lowercase().contains(selector)
        });
        path_matches && query_matches && symbol_matches
    }) {
        if selected.len() >= limit {
            dropped_direct_matches += 1;
            continue;
        }
        selected.insert(candidate.symbol_key.clone(), (candidate.clone(), "match"));
    }

    if baseline_query && !selected.is_empty() {
        let Some(connection) = connection.as_ref() else {
            return Err(CodeGraphProjectionError::IndexUnavailable);
        };
        let seed_keys = selected.keys().cloned().collect::<BTreeSet<_>>();
        let (context_edges, dropped) = query_context_edges_and_neighbors(
            config,
            connection,
            &options.repo_id,
            &base_revision,
            &mut symbols,
            &seed_keys,
            CodeGraphContextBounds { depth, limit },
        )?;
        edges = context_edges;
        query_dropped = query_dropped.saturating_add(dropped);
    }

    let mut frontier = selected.keys().cloned().collect::<BTreeSet<_>>();
    let mut dropped_neighbors = 0;
    for _ in 0..depth {
        let mut next_frontier = BTreeSet::new();
        for edge in &edges {
            let adjacent = if edge
                .source_symbol_key
                .as_deref()
                .is_some_and(|key| frontier.contains(key))
            {
                edge.target_symbol_key.as_deref()
            } else if edge
                .target_symbol_key
                .as_deref()
                .is_some_and(|key| frontier.contains(key))
            {
                edge.source_symbol_key.as_deref()
            } else {
                None
            };
            let Some(adjacent) = adjacent else {
                continue;
            };
            if selected.contains_key(adjacent) {
                continue;
            }
            let Some(symbol) = symbols.get(adjacent) else {
                continue;
            };
            if selected.len() >= limit {
                dropped_neighbors += 1;
                continue;
            }
            let relation = if is_test_code_symbol(symbol) {
                "related_test"
            } else if edge
                .target_symbol_key
                .as_deref()
                .is_some_and(|key| frontier.contains(key))
            {
                "caller"
            } else {
                "reference"
            };
            selected.insert(adjacent.to_string(), (symbol.clone(), relation));
            next_frontier.insert(adjacent.to_string());
        }
        if next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    let provenance = overlay
        .map(|overlay| {
            serde_json::json!({
                "kind": "workspace_overlay",
                "runId": overlay.run_id,
                "baseRevision": overlay.base_revision,
                "headRevision": overlay.head_revision,
                "overlayDigest": overlay.workspace_content_digest
            })
        })
        .unwrap_or_else(|| {
            serde_json::json!({
                "kind": "indexed_baseline",
                "baseRevision": base_revision
            })
        });
    let mut evidence = Vec::new();
    for (key, (symbol, relation)) in &selected {
        evidence.push(context_symbol_json(
            &options.repo_id,
            &base_revision,
            overlay,
            symbol,
            relation,
        ));
        for edge in edges.iter().filter(|edge| {
            edge.source_symbol_key.as_deref() == Some(key.as_str())
                || edge.target_symbol_key.as_deref() == Some(key.as_str())
        }) {
            let edge_relation = if edge.target_symbol_key.as_deref() == Some(key.as_str()) {
                "caller"
            } else {
                "reference"
            };
            evidence.push(context_edge_json(
                &options.repo_id,
                &base_revision,
                overlay,
                edge,
                edge.source_symbol_key
                    .as_deref()
                    .and_then(|source_key| symbols.get(source_key))
                    .or_else(|| {
                        edge.target_symbol_key
                            .as_deref()
                            .and_then(|target_key| symbols.get(target_key))
                    }),
                edge_relation,
            ));
        }
        let diagnostics = if let Some(overlay) = overlay {
            if is_live_overlay_path(Some(overlay), &symbol.path) {
                overlay
                    .diagnostics
                    .get(&symbol.path)
                    .into_iter()
                    .flatten()
                    .filter(|diagnostic| {
                        diagnostic.span.start_line <= symbol.end_line
                            && diagnostic.span.end_line >= symbol.start_line
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            } else if let Some(connection) = connection.as_ref() {
                query_symbol_diagnostics(connection, config, symbol, false)?
            } else {
                Vec::new()
            }
        } else if let Some(connection) = connection.as_ref() {
            query_symbol_diagnostics(connection, config, symbol, false)?
        } else {
            Vec::new()
        };
        for diagnostic in diagnostics {
            evidence.push(context_diagnostic_json(
                &options.repo_id,
                &base_revision,
                overlay,
                &symbol.path,
                ContextDiagnosticMetadata {
                    symbol_key: Some(&symbol.symbol_key),
                    parser_version: &symbol.parser_version,
                    query_pack_version: &symbol.query_pack_version,
                    freshness: &symbol.freshness,
                },
                &diagnostic,
            ));
        }
    }

    if selected.is_empty() {
        if let Some(overlay) = overlay {
            for path in overlay.diagnostics.keys().filter(|candidate| {
                path.as_deref().is_some_and(|path| {
                    *candidate == path || candidate.starts_with(&format!("{path}/"))
                }) || query.as_deref().is_some_and(|query| {
                    candidate.to_ascii_lowercase().contains(query)
                })
            }) {
                let (parser_version, query_pack_version) = overlay
                    .diagnostic_metadata
                    .get(path)
                    .cloned()
                    .unwrap_or_default();
                for diagnostic in overlay.diagnostics.get(path).into_iter().flatten() {
                    evidence.push(context_diagnostic_json(
                        &options.repo_id,
                        &base_revision,
                        Some(overlay),
                        path,
                        ContextDiagnosticMetadata {
                            symbol_key: None,
                            parser_version: &parser_version,
                            query_pack_version: &query_pack_version,
                            freshness: "current",
                        },
                        diagnostic,
                    ));
                }
            }
        } else if let (Some(connection), Some(path)) = (connection.as_ref(), path.as_deref()) {
            let (diagnostics, dropped) = query_context_diagnostics_for_path(
                config,
                connection,
                &options.repo_id,
                &base_revision,
                path,
                limit,
            )?;
            query_dropped = query_dropped.saturating_add(usize::from(dropped));
            for diagnostic in diagnostics {
                evidence.push(context_diagnostic_json(
                    &options.repo_id,
                    &base_revision,
                    None,
                    &diagnostic.path,
                    ContextDiagnosticMetadata {
                        symbol_key: None,
                        parser_version: &diagnostic.parser_version,
                        query_pack_version: &diagnostic.query_pack_version,
                        freshness: &diagnostic.freshness,
                    },
                    &diagnostic.diagnostic,
                ));
            }
        }
    }

    let dropped = query_dropped
        + dropped_direct_matches
        + dropped_neighbors
        + evidence.len().saturating_sub(limit);
    evidence.truncate(limit);
    Ok(serde_json::json!({
        "repository": options.repo_id,
        "baseRevision": base_revision,
        "overlayDigest": overlay.map(|overlay| overlay.workspace_content_digest.clone()),
        "provenance": provenance,
        "depth": depth,
        "limit": limit,
        "truncated": dropped > 0,
        "dropped": dropped,
        "evidence": evidence,
        "unanalyzedFiles": overlay.map(|overlay| overlay.unanalyzed_files.clone()).unwrap_or_default()
    }))
}

fn context_diagnostic_json(
    repo_id: &str,
    base_revision: &str,
    overlay: Option<&CodeWorkspaceOverlay>,
    path: &str,
    metadata: ContextDiagnosticMetadata<'_>,
    diagnostic: &CodeDiagnostic,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "diagnostic",
        "repository": repo_id,
        "path": path,
        "symbolKey": metadata.symbol_key,
        "span": context_span_json(&diagnostic.span),
        "message": diagnostic.message,
        "diagnosticKind": diagnostic.kind,
        "severity": diagnostic.severity,
        "parserVersion": metadata.parser_version,
        "queryPackVersion": metadata.query_pack_version,
        "baseRevision": base_revision,
        "overlayDigest": overlay.map(|overlay| overlay.workspace_content_digest.clone()),
        "provenance": if is_live_overlay_path(overlay, path) {
            "workspace_overlay"
        } else {
            "indexed_baseline"
        },
        "freshness": metadata.freshness,
        "sourceRef": context_source_ref(
            path,
            diagnostic.span.start_line,
            diagnostic.span.start_col,
            diagnostic.span.end_line,
            diagnostic.span.end_col,
        )
    })
}

struct CodeGraphContextSelectors<'a> {
    path: Option<&'a str>,
    query: Option<&'a str>,
    symbol: Option<&'a str>,
}

struct CodeGraphContextBounds {
    depth: usize,
    limit: usize,
}

#[derive(Debug, Clone)]
struct ContextDiagnostic {
    path: String,
    diagnostic: CodeDiagnostic,
    parser_version: String,
    query_pack_version: String,
    freshness: String,
}

struct ContextDiagnosticMetadata<'a> {
    symbol_key: Option<&'a str>,
    parser_version: &'a str,
    query_pack_version: &'a str,
    freshness: &'a str,
}

fn query_context_diagnostics_for_path(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    path: &str,
    limit: usize,
) -> Result<(Vec<ContextDiagnostic>, bool), MemoryError> {
    let path_prefix = format!("{path}/%");
    let fetch_limit = limit.saturating_add(1) as i64;
    let rows = if code_diagnostic_revisions_read_model_ready(connection, &config.index_path)? {
        let mut statement = connection
            .prepare(
                "SELECT path, kind, severity, message, start_line, start_col, end_line, end_col, parser_version, query_pack_version, freshness FROM code_diagnostic_revisions WHERE repo_id = ? AND commit_sha = ? AND freshness = 'current' AND NOT worktree_dirty AND (path = ? OR path LIKE ?) ORDER BY path, start_line, start_col, diagnostic_id LIMIT ?",
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        statement
            .query_map(
                params![repo_id, revision, path, path_prefix, fetch_limit],
                |row| {
                    Ok(ContextDiagnostic {
                        path: row.get(0)?,
                        diagnostic: CodeDiagnostic {
                            kind: row.get(1)?,
                            severity: row.get(2)?,
                            message: row.get(3)?,
                            span: CodeSpan {
                                start_line: row.get::<_, i64>(4)? as usize,
                                start_col: row.get::<_, i64>(5)? as usize,
                                end_line: row.get::<_, i64>(6)? as usize,
                                end_col: row.get::<_, i64>(7)? as usize,
                            },
                        },
                        parser_version: row.get(8)?,
                        query_pack_version: row.get(9)?,
                        freshness: row.get(10)?,
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
            })?
    } else {
        let mut statement = connection
            .prepare(
                "SELECT path, kind, severity, message, start_line, start_col, end_line, end_col, parser_version, query_pack_version, freshness FROM code_diagnostics WHERE repo_id = ? AND commit_sha = ? AND freshness = 'current' AND NOT worktree_dirty AND (path = ? OR path LIKE ?) ORDER BY path, start_line, start_col, diagnostic_id LIMIT ?",
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        statement
            .query_map(
                params![repo_id, revision, path, path_prefix, fetch_limit],
                |row| {
                    Ok(ContextDiagnostic {
                        path: row.get(0)?,
                        diagnostic: CodeDiagnostic {
                            kind: row.get(1)?,
                            severity: row.get(2)?,
                            message: row.get(3)?,
                            span: CodeSpan {
                                start_line: row.get::<_, i64>(4)? as usize,
                                start_col: row.get::<_, i64>(5)? as usize,
                                end_line: row.get::<_, i64>(6)? as usize,
                                end_col: row.get::<_, i64>(7)? as usize,
                            },
                        },
                        parser_version: row.get(8)?,
                        query_pack_version: row.get(9)?,
                        freshness: row.get(10)?,
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
            })?
    };
    let dropped = rows.len() > limit;
    Ok((rows.into_iter().take(limit).collect(), dropped))
}

fn query_context_symbols_for_revision(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    selectors: CodeGraphContextSelectors<'_>,
    limit: usize,
) -> Result<(BTreeMap<String, CodeSymbolRecord>, usize), MemoryError> {
    let membership_ready = matches!(
        code_snapshot_status(connection, config, repo_id, revision)?.as_deref(),
        None | Some("completed")
    ) && code_snapshot_membership_read_model_ready(connection, &config.index_path)?;
    let revision_predicate = if membership_ready {
        "s.repo_id = ? AND s.symbol_key != '' AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND (s.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = s.repo_id AND m.commit_sha = ? AND m.path = s.path AND m.content_sha256 = s.content_sha256 AND m.parser_version = s.parser_version AND m.query_pack_version = s.query_pack_version AND m.analyzed) )"
    } else {
        "s.repo_id = ? AND s.commit_sha = ? AND s.symbol_key != '' AND s.freshness <> 'staged' AND NOT s.worktree_dirty"
    };
    let selector_predicate = concat!(
        "(? IS NULL OR s.path = ? OR s.path LIKE ?) ",
        "AND (? IS NULL OR lower(s.name) LIKE ? OR lower(s.path) LIKE ? ",
        "OR lower(s.container_chain) LIKE ?) ",
        "AND (? IS NULL OR lower(s.symbol_key) = ? OR lower(s.name) = ? ",
        "OR lower(s.name) LIKE ?)",
    );
    let from_where = format!(
        "FROM code_symbols AS s WHERE {revision_predicate} AND {selector_predicate}"
    );
    let path_exact = selectors.path.map(str::to_string);
    let path_prefix = selectors.path.map(|path| format!("{path}/%"));
    let query_like = selectors.query.map(|query| format!("%{query}%"));
    let symbol_exact = selectors.symbol.map(str::to_string);
    let symbol_like = selectors.symbol.map(|selector| format!("%{selector}%"));
    let count_query = format!("SELECT COUNT(DISTINCT s.symbol_key) {from_where}");
    let total = if membership_ready {
        connection.query_row(
            &count_query,
            params![
                repo_id,
                revision,
                revision,
                path_exact.as_deref(),
                path_exact.as_deref(),
                path_prefix.as_deref(),
                query_like.as_deref(),
                query_like.as_deref(),
                query_like.as_deref(),
                query_like.as_deref(),
                symbol_exact.as_deref(),
                symbol_exact.as_deref(),
                symbol_exact.as_deref(),
                symbol_like.as_deref(),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
    } else {
        connection.query_row(
            &count_query,
            params![
                repo_id,
                revision,
                path_exact.as_deref(),
                path_exact.as_deref(),
                path_prefix.as_deref(),
                query_like.as_deref(),
                query_like.as_deref(),
                query_like.as_deref(),
                query_like.as_deref(),
                symbol_exact.as_deref(),
                symbol_exact.as_deref(),
                symbol_exact.as_deref(),
                symbol_like.as_deref(),
            ],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
    } as usize;

    let query = format!(
        "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN s.worktree_dirty THEN 1 ELSE 0 END {from_where} QUALIFY ROW_NUMBER() OVER (PARTITION BY s.symbol_key ORDER BY s.indexed_at DESC, s.symbol_id) = 1 ORDER BY s.symbol_key, s.indexed_at DESC, s.symbol_id LIMIT ?"
    );
    let mut statement = connection.prepare(&query).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let rows = if membership_ready {
        statement
            .query_map(
                params![
                    repo_id,
                    revision,
                    revision,
                    path_exact.as_deref(),
                    path_exact.as_deref(),
                    path_prefix.as_deref(),
                    query_like.as_deref(),
                    query_like.as_deref(),
                    query_like.as_deref(),
                    query_like.as_deref(),
                    symbol_exact.as_deref(),
                    symbol_exact.as_deref(),
                    symbol_exact.as_deref(),
                    symbol_like.as_deref(),
                    limit as i64,
                ],
                |row| {
                    let dirty = row.get::<_, i64>(25)?;
                    if dirty != 0 {
                        Ok(None)
                    } else {
                        code_symbol_from_row(row).map(Some)
                    }
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
    } else {
        statement
            .query_map(
                params![
                    repo_id,
                    revision,
                    path_exact.as_deref(),
                    path_exact.as_deref(),
                    path_prefix.as_deref(),
                    query_like.as_deref(),
                    query_like.as_deref(),
                    query_like.as_deref(),
                    query_like.as_deref(),
                    symbol_exact.as_deref(),
                    symbol_exact.as_deref(),
                    symbol_exact.as_deref(),
                    symbol_like.as_deref(),
                    limit as i64,
                ],
                |row| {
                    let dirty = row.get::<_, i64>(25)?;
                    if dirty != 0 {
                        Ok(None)
                    } else {
                        code_symbol_from_row(row).map(Some)
                    }
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
    }?;
    let mut symbols = BTreeMap::new();
    for row in rows {
        let Some(mut symbol) = row else {
            continue;
        };
        if symbol.commit_sha.as_deref() != Some(revision) {
            symbol.commit_sha = Some(revision.to_string());
        }
        fill_container_chain(connection, &mut symbol)?;
        symbols.insert(symbol.symbol_key.clone(), symbol);
    }
    Ok((symbols, total.saturating_sub(limit)))
}

fn query_context_edges_and_neighbors(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    symbols: &mut BTreeMap<String, CodeSymbolRecord>,
    seed_keys: &BTreeSet<String>,
    bounds: CodeGraphContextBounds,
) -> Result<(Vec<CodeEdgeRecord>, usize), MemoryError> {
    let max_edges = bounds
        .limit
        .saturating_mul(bounds.depth.saturating_add(1))
        .max(bounds.limit);
    let mut edges = BTreeMap::new();
    let mut frontier = seed_keys.clone();
    let mut dropped = 0;

    for layer in 0..=bounds.depth {
        if frontier.is_empty() {
            break;
        }
        let mut layer_edges = Vec::new();
        let mut remaining_keys = frontier.len();
        for key in &frontier {
            let remaining_budget = max_edges.saturating_sub(edges.len());
            if remaining_budget == 0 {
                dropped += 1;
                break;
            }
            let per_key_limit = remaining_budget
                .div_ceil(remaining_keys.max(1))
                .min(bounds.limit.max(1));
            let (key_edges, truncated) = query_context_edges_for_key(
                config,
                connection,
                repo_id,
                revision,
                key,
                per_key_limit.min(bounds.limit.max(1)),
            )?;
            if truncated {
                dropped += 1;
            }
            remaining_keys = remaining_keys.saturating_sub(1);
            for edge in key_edges {
                edges.insert(edge.edge_id.clone(), edge.clone());
                layer_edges.push(edge);
            }
        }

        let mut adjacent = BTreeSet::new();
        for edge in layer_edges {
            if edge
                .source_symbol_key
                .as_ref()
                .is_some_and(|key| frontier.contains(key))
                && let Some(key) = edge.target_symbol_key
            {
                adjacent.insert(key);
            } else if edge
                .target_symbol_key
                .as_ref()
                .is_some_and(|key| frontier.contains(key))
                && let Some(key) = edge.source_symbol_key
            {
                adjacent.insert(key);
            }
        }
        if adjacent.len() > bounds.limit {
            dropped += adjacent.len() - bounds.limit;
        }
        let mut next_frontier = BTreeSet::new();
        for key in adjacent.into_iter().take(bounds.limit) {
            if !symbols.contains_key(&key)
                && let Some(symbol) =
                    query_revision_symbol_by_key(config, connection, repo_id, revision, &key)?
            {
                symbols.insert(key.clone(), symbol);
            }
            if symbols.contains_key(&key) {
                next_frontier.insert(key);
            }
        }
        if layer == bounds.depth {
            break;
        }
        frontier = next_frontier;
    }

    Ok((edges.into_values().collect(), dropped))
}

fn query_context_edges_for_key(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    symbol_key: &str,
    limit: usize,
) -> Result<(Vec<CodeEdgeRecord>, bool), MemoryError> {
    let membership_ready = matches!(
        code_snapshot_status(connection, config, repo_id, revision)?.as_deref(),
        None | Some("completed")
    ) && code_snapshot_membership_read_model_ready(connection, &config.index_path)?;
    let query = if membership_ready {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND (e.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = e.repo_id AND m.commit_sha = ? AND m.path = e.path AND m.content_sha256 = e.content_sha256 AND m.parser_version = e.parser_version AND m.query_pack_version = e.query_pack_version AND m.analyzed)) AND (e.source_symbol_key = ? OR e.target_symbol_key = ?) ORDER BY e.path, e.start_line, e.start_col, e.edge_id LIMIT ?"
    } else {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.commit_sha = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND (e.source_symbol_key = ? OR e.target_symbol_key = ?) ORDER BY e.path, e.start_line, e.start_col, e.edge_id LIMIT ?"
    };
    let mut statement = connection.prepare(query).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let fetch_limit = limit.saturating_add(1) as i64;
    let rows = if membership_ready {
        statement
            .query_map(
            params![repo_id, revision, revision, symbol_key, symbol_key, fetch_limit],
            code_edge_from_row,
        )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
    } else {
        statement
            .query_map(
            params![repo_id, revision, symbol_key, symbol_key, fetch_limit],
            code_edge_from_row,
        )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
    }
    .collect::<Result<Vec<_>, _>>()
    .map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let truncated = rows.len() > limit;
    Ok((rows.into_iter().take(limit).collect(), truncated))
}

fn is_test_code_symbol(symbol: &CodeSymbolRecord) -> bool {
    let path = symbol.path.to_ascii_lowercase();
    path.contains("/test/")
        || path.starts_with("test/")
        || path.contains("/tests/")
        || path.starts_with("tests/")
        || symbol.name.to_ascii_lowercase().starts_with("test")
        || symbol
            .container_chain
            .iter()
            .any(|container| container.to_ascii_lowercase().starts_with("test"))
}

fn context_symbol_json(
    repo_id: &str,
    base_revision: &str,
    overlay: Option<&CodeWorkspaceOverlay>,
    symbol: &CodeSymbolRecord,
    relation: &str,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "symbol",
        "relation": relation,
        "repository": repo_id,
        "symbolKey": symbol.symbol_key,
        "symbolId": symbol.symbol_id,
        "name": symbol.name,
        "symbolKind": symbol.kind,
        "path": symbol.path,
        "containerChain": symbol.container_chain,
        "span": context_span_json_values(symbol.start_line, symbol.start_col, symbol.end_line, symbol.end_col),
        "parserVersion": symbol.parser_version,
        "queryPackVersion": symbol.query_pack_version,
        "confidence": "exact",
        "freshness": symbol.freshness,
        "baseRevision": base_revision,
        "overlayDigest": overlay.map(|overlay| overlay.workspace_content_digest.clone()),
        "provenance": if is_live_overlay_path(overlay, &symbol.path) { "workspace_overlay" } else { "indexed_baseline" },
        "sourceRef": context_source_ref(&symbol.path, symbol.start_line, symbol.start_col, symbol.end_line, symbol.end_col)
    })
}

fn context_edge_json(
    repo_id: &str,
    base_revision: &str,
    overlay: Option<&CodeWorkspaceOverlay>,
    edge: &CodeEdgeRecord,
    related_symbol: Option<&CodeSymbolRecord>,
    relation: &str,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "edge",
        "relation": relation,
        "repository": repo_id,
        "edgeKind": edge.edge_kind,
        "sourceSymbolKey": edge.source_symbol_key,
        "targetSymbolKey": edge.target_symbol_key,
        "targetHint": edge.target_hint,
        "path": edge.path,
        "span": context_span_json_values(edge.start_line, edge.start_col, edge.end_line, edge.end_col),
        "parserVersion": related_symbol.map(|symbol| symbol.parser_version.clone()),
        "queryPackVersion": related_symbol.map(|symbol| symbol.query_pack_version.clone()),
        "confidence": edge.confidence,
        "freshness": edge.freshness,
        "baseRevision": base_revision,
        "overlayDigest": overlay.map(|overlay| overlay.workspace_content_digest.clone()),
        "provenance": if is_live_overlay_path(overlay, &edge.path) { "workspace_overlay" } else { "indexed_baseline" },
        "sourceRef": context_source_ref(&edge.path, edge.start_line, edge.start_col, edge.end_line, edge.end_col)
    })
}

fn is_live_overlay_path(overlay: Option<&CodeWorkspaceOverlay>, path: &str) -> bool {
    overlay.is_some_and(|overlay| {
        overlay.changed_paths.contains(path)
            && !overlay.unanalyzed_files.iter().any(|candidate| candidate == path)
    })
}

fn context_span_json(span: &CodeSpan) -> serde_json::Value {
    context_span_json_values(span.start_line, span.start_col, span.end_line, span.end_col)
}

fn context_span_json_values(
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
) -> serde_json::Value {
    serde_json::json!({
        "startLine": start_line,
        "startColumn": start_col,
        "endLine": end_line,
        "endColumn": end_col
    })
}

fn context_source_ref(
    path: &str,
    start_line: usize,
    start_col: usize,
    end_line: usize,
    end_col: usize,
) -> String {
    format!("{path}:{start_line}:{start_col}-{end_line}:{end_col}")
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct WorkspaceDocumentCacheKey {
    repo_id: String,
    path: String,
    content_sha256: String,
    max_capture_bytes: usize,
    max_matches: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkspaceDocumentRecords {
    symbols: Vec<CodeSymbolRecord>,
    edges: Vec<CodeEdgeRecord>,
    diagnostics: Vec<CodeDiagnostic>,
    parser_version: String,
    query_pack_version: String,
}

type WorkspaceLiveChangedRecords = (
    BTreeMap<String, CodeSymbolRecord>,
    BTreeMap<String, CodeEdgeRecord>,
    BTreeSet<String>,
    Vec<String>,
);

static WORKSPACE_DOCUMENT_CACHE: OnceLock<Mutex<BTreeMap<WorkspaceDocumentCacheKey, WorkspaceDocumentRecords>>> =
    OnceLock::new();

pub fn code_graph_repos(
    config: &MemoryConfig,
    include_stale: bool,
) -> Result<CodeRepoList, CodeGraphProjectionError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(unindexed_code_repo_list(config));
    };
    if !code_documents_read_model_ready(&connection, &config.index_path)? {
        return Ok(unindexed_code_repo_list(config));
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

    if table_has_columns(
        &connection,
        &config.index_path,
        "code_index_snapshots",
        &["repo_id", "commit_sha", "status", "indexed_at"],
    )? && code_snapshot_membership_read_model_ready(&connection, &config.index_path)?
    {
        let mut snapshots = connection
            .prepare(
                r#"
                    SELECT repo_id, commit_sha, indexed_at
                    FROM (
                        SELECT repo_id, commit_sha, indexed_at,
                            ROW_NUMBER() OVER (
                                PARTITION BY repo_id
                                ORDER BY indexed_at DESC, commit_sha DESC
                            ) AS row_rank
                        FROM code_index_snapshots
                        WHERE status = 'completed'
                    ) latest
                    WHERE row_rank = 1
                "#,
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        let snapshots = snapshots
            .query_map([], |row| {
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
        for (repo_id, commit_sha, indexed_at) in snapshots {
            let entry = repos.entry(repo_id.clone()).or_insert_with(|| {
                CodeRepoAccumulator {
                    repo_id: repo_id.clone(),
                    ..CodeRepoAccumulator::default()
                }
            });
            entry.has_current = true;
            entry.head_revision = Some(commit_sha.clone());
            entry.indexed_at = parse_code_datetime(&indexed_at);

            let mut membership = connection
                .prepare(
                    "SELECT path, language FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? AND analyzed ORDER BY path",
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            let membership = membership
                .query_map(params![&repo_id, &commit_sha], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
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
            if let Some(entry) = repos.get_mut(&repo_id) {
                for (path, language) in membership {
                    entry.paths.insert(path);
                    entry.languages.insert(language);
                }
            }
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

    if summaries.is_empty() {
        summaries.push(unindexed_code_repo_summary(config));
    }

    Ok(CodeRepoList {
        schema_version: SchemaVersion::v1(),
        repos: summaries,
    })
}

fn unindexed_code_repo_list(config: &MemoryConfig) -> CodeRepoList {
    CodeRepoList {
        schema_version: SchemaVersion::v1(),
        repos: vec![unindexed_code_repo_summary(config)],
    }
}

fn unindexed_code_repo_summary(config: &MemoryConfig) -> CodeRepoSummary {
    let repo_id = configured_code_repo_id(config);
    CodeRepoSummary {
        repo_id: repo_id.clone(),
        display_root: repo_id,
        languages: Vec::new(),
        document_count: 0,
        symbol_count: 0,
        edge_count: 0,
        freshness: CodeGraphFreshness::Unknown,
        indexed: false,
        indexed_at: None,
        head_revision: None,
        worktree_dirty: false,
    }
}

fn configured_code_repo_id(config: &MemoryConfig) -> String {
    let remote = Command::new("git")
        .args(["-C"])
        .arg(&config.repo_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| repo_id_from_remote_url(&String::from_utf8_lossy(&output.stdout)));
    remote.unwrap_or_else(|| {
        config
            .repo_root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("repository")
            .to_string()
    })
}

fn repo_id_from_remote_url(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(".git").trim_end_matches('/');
    let name = trimmed
        .rsplit(['/', ':'])
        .next()
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    Some(name.to_string())
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
        let repository_scope_matches = repository_scope_matches(&issue.scope_refs, repo_id);
        let source_match = issue.source_refs.iter().any(|source| {
            let source_repo_matches = source
                .repo_id
                .as_deref()
                .is_none_or(|source_repo| source_repo == repo_id);
            let symbol_match = source.repo_id.as_deref() == Some(repo_id)
                && source.symbol_key.as_deref() == Some(symbol.symbol_key.as_str());
            let legacy_path_match = source.symbol_key.is_none()
                && ((source.kind == "path" && source.id == symbol.path)
                    || (source.kind == "code-symbol"
                        && code_symbol_source_ref_matches(source, symbol)));
            repository_scope_matches
                && source_repo_matches
                && (symbol_match || legacy_path_match)
        });
        let scope_match = repository_scope_matches && issue.scope_refs.iter().any(|scope| {
            matches!(scope.kind, KnowledgeScopeKind::CodePath)
                && (scope.id == symbol.path
                    || symbol.path.starts_with(&format!("{}/", scope.id)))
        });
        let citation_match = issue
            .citations
            .iter()
            .any(|citation| {
                code_citation_matches_symbol(
                    &citation.target,
                    repo_id,
                    &symbol.symbol_key,
                    &symbol.path,
                )
            });
        if !(source_match || scope_match || citation_match) {
            continue;
        }
        let freshness = freshness_from_str(issue.freshness.as_str());
        if (issue.concept_type == "issue-capsule" || has_work_item_scope(&issue.scope_refs))
            && !related_issues
                .iter()
                .any(|chip: &CodeGraphIssueChip| chip.issue_key == issue.issue_key)
        {
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

fn repository_scope_matches(scope_refs: &[KnowledgeScope], repo_id: &str) -> bool {
    let has_repository_scope = scope_refs
        .iter()
        .any(|scope| matches!(scope.kind, KnowledgeScopeKind::Repository));
    !has_repository_scope
        || scope_refs.iter().any(|scope| {
            matches!(scope.kind, KnowledgeScopeKind::Repository) && scope.id == repo_id
        })
}

fn has_work_item_scope(scope_refs: &[KnowledgeScope]) -> bool {
    scope_refs
        .iter()
        .any(|scope| matches!(scope.kind, KnowledgeScopeKind::WorkItem))
}

fn code_citation_matches_symbol(
    target: &str,
    repo_id: &str,
    symbol_key: &str,
    path: &str,
) -> bool {
    let Ok(url) = Url::parse(target) else {
        return false;
    };
    if url.scheme() != "opensymphony" || url.host_str() != Some("code") {
        return false;
    }
    let Some(segments) = url.path_segments() else {
        return false;
    };
    let Some(actual_segments) = segments
        .map(decode_code_path_segment)
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };

    [
        ("symbols", symbol_key),
        ("files", path),
    ]
    .into_iter()
    .any(|(collection, value)| {
        let mut expected = vec![repo_id.to_string(), collection.to_string()];
        expected.extend(value.split('/').map(str::to_string));
        actual_segments == expected
    })
}

fn decode_code_path_segment(segment: &str) -> Option<String> {
    url::form_urlencoded::parse(format!("value={segment}").as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
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
    let base_symbols = query_symbols_for_revision(config, &connection, repo_id, base_revision)?;
    let head_symbols = query_symbols_for_revision(config, &connection, repo_id, head_revision)?;
    let base_edges = query_code_edges_for_revision(&connection, config, repo_id, base_revision)?;
    let head_edges = query_code_edges_for_revision(&connection, config, repo_id, head_revision)?;
    let (edge_deltas, dropped_edges) =
        compare_code_edges(base_edges.clone(), head_edges.clone(), max_records.max(1));
    let (module_connection_deltas, dropped_connections) = module_connection_deltas(
        &base_edges,
        &head_edges,
        &base_symbols,
        &head_symbols,
        max_records,
    );
    let unanalyzed_files =
        query_unanalyzed_diff_files(&connection, config, repo_id, base_revision, head_revision)?;
    let mut added_symbols = Vec::new();
    let mut removed_symbols = Vec::new();
    let mut modified_symbols = Vec::new();
    let mut diff_dtos = Vec::new();

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
        diff_dtos.push(dto.clone());
        match dto.status {
            DtoDiffStatus::Added => added_symbols.push(dto),
            DtoDiffStatus::Removed => removed_symbols.push(dto),
            DtoDiffStatus::Modified => modified_symbols.push(dto),
        }
    }
    let (blast_radius, dropped_blast_radius) =
        detailed_blast_radius(&diff_dtos, &base_edges, &head_edges, max_records.max(1));

    Ok(CodeDiffOverlay {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        base_revision: base_revision.to_string(),
        head_revision: head_revision.to_string(),
        added_symbols,
        removed_symbols,
        modified_symbols,
        edge_deltas,
        module_connection_deltas,
        blast_radius,
        unanalyzed_files,
        truncation: truncation(
            comparison.dropped_records,
            dropped_edges
                .saturating_add(dropped_blast_radius)
                .saturating_add(dropped_connections),
            "diff symbols or edges capped",
        ),
        generated_at: Utc::now(),
    })
}

pub fn code_graph_workspace_overlay(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    run_id: &str,
    base_revision: &str,
) -> Result<CodeWorkspaceOverlay, CodeGraphProjectionError> {
    let connection = open_workspace_graph_connection(config, repo_id, workspace_path, base_revision)?;

    let base_symbols = query_symbols_for_revision(config, &connection, repo_id, base_revision)?;
    let base_paths = query_revision_documents(&connection, config, repo_id, base_revision)?
        .into_keys()
        .collect::<BTreeSet<_>>();
    let base_edges = query_code_edges_for_revision(&connection, config, repo_id, base_revision)?;
    let head_revision = workspace_head_revision(workspace_path)?;
    let (changed_paths, tombstones, workspace_content_digest) = workspace_changed_paths(
        workspace_path,
        base_revision,
        config.code_intel.ast.max_file_bytes,
    )?;

    let mut symbols = base_symbols.clone();
    let mut edges = base_edges
        .clone()
        .into_iter()
        .map(|edge| (edge.edge_id.clone(), edge))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = BTreeMap::<String, Vec<CodeDiagnostic>>::new();
    let mut diagnostic_metadata = BTreeMap::<String, (String, String)>::new();
    let mut unanalyzed_files = BTreeSet::new();
    let mut remaining_files = config.code_intel.ast.max_files_per_request;

    for path in &changed_paths {
        if tombstones.contains(path) {
            if !base_symbols.values().any(|symbol| symbol.path == *path) {
                unanalyzed_files.insert(path.clone());
            }
            symbols.retain(|_, symbol| symbol.path != *path);
            edges.retain(|_, edge| edge.path != *path);
            continue;
        }

        let Some(file_path) = workspace_file_path(workspace_path, path)? else {
            unanalyzed_files.insert(path.clone());
            continue;
        };
        if remaining_files == 0 {
            unanalyzed_files.insert(path.clone());
            continue;
        }

        let Some(records) = workspace_document_records(
            config,
            repo_id,
            path,
            &file_path,
        )?
        else {
            unanalyzed_files.insert(path.clone());
            continue;
        };
        remaining_files = remaining_files.saturating_sub(1);
        symbols.retain(|_, symbol| symbol.path != *path);
        edges.retain(|_, edge| edge.path != *path);
        diagnostics.insert(path.clone(), records.diagnostics);
        diagnostic_metadata.insert(
            path.clone(),
            (records.parser_version.clone(), records.query_pack_version.clone()),
        );
        for symbol in records.symbols {
            symbols.insert(symbol.symbol_key.clone(), symbol);
        }
        for edge in records.edges {
            edges.insert(edge.edge_id.clone(), edge);
        }
    }

    re_resolve_workspace_edges(&mut edges, &symbols);
    Ok(CodeWorkspaceOverlay {
        run_id: run_id.to_string(),
        base_revision: base_revision.to_string(),
        head_revision,
        workspace_content_digest,
        base_symbols,
        base_paths,
        base_edges,
        symbols,
        edges: edges.into_values().collect(),
        changed_paths,
        tombstones,
        unanalyzed_files: unanalyzed_files.into_iter().collect(),
        diagnostics,
        diagnostic_metadata,
        retrieval_dropped: 0,
    })
}

pub fn code_graph_workspace_context_overlay(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    run_id: &str,
    base_revision: &str,
    options: &CodeGraphContextQuery,
) -> Result<CodeWorkspaceOverlay, CodeGraphProjectionError> {
    let connection = open_workspace_graph_connection(config, repo_id, workspace_path, base_revision)?;
    let path = options
        .path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(normalize_code_path)
        .transpose()
        .map_err(CodeGraphProjectionError::InvalidRequest)?;
    let query = options
        .query
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase());
    let symbol_selector = options
        .symbol
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_ascii_lowercase());
    let depth = options.depth.min(8);
    let limit = options.limit.clamp(1, CODE_GRAPH_MAX_RECORDS);
    let (mut base_symbols, mut retrieval_dropped) = query_context_symbols_for_revision(
        config,
        &connection,
        repo_id,
        base_revision,
        CodeGraphContextSelectors {
            path: path.as_deref(),
            query: query.as_deref(),
            symbol: symbol_selector.as_deref(),
        },
        limit,
    )?;
    let seed_keys = base_symbols.keys().cloned().collect::<BTreeSet<_>>();
    let (base_edges, dropped) = query_context_edges_and_neighbors(
        config,
        &connection,
        repo_id,
        base_revision,
        &mut base_symbols,
        &seed_keys,
        CodeGraphContextBounds { depth, limit },
    )?;
    retrieval_dropped = retrieval_dropped.saturating_add(dropped);

    let head_revision = workspace_head_revision(workspace_path)?;
    let (changed_paths, tombstones, workspace_content_digest) = workspace_changed_paths(
        workspace_path,
        base_revision,
        config.code_intel.ast.max_file_bytes,
    )?;
    let mut symbols = base_symbols.clone();
    let mut edges = base_edges
        .iter()
        .cloned()
        .map(|edge| (edge.edge_id.clone(), edge))
        .collect::<BTreeMap<_, _>>();
    let mut diagnostics = BTreeMap::<String, Vec<CodeDiagnostic>>::new();
    let mut diagnostic_metadata = BTreeMap::<String, (String, String)>::new();
    let mut unanalyzed_files = BTreeSet::new();
    let mut remaining_files = config.code_intel.ast.max_files_per_request;
    let path_matches = |candidate: &str| {
        path.as_deref().is_none_or(|selector| {
            candidate == selector || candidate.starts_with(&format!("{selector}/"))
        })
    };

    for changed_path in changed_paths.iter().filter(|candidate| path_matches(candidate)) {
        if tombstones.contains(changed_path) {
            symbols.retain(|_, symbol| symbol.path != *changed_path);
            edges.retain(|_, edge| edge.path != *changed_path);
            continue;
        }
        let has_baseline_record = symbols.values().any(|symbol| symbol.path == *changed_path);
        if remaining_files == 0 {
            if has_baseline_record || path.is_some() {
                unanalyzed_files.insert(changed_path.clone());
            }
            continue;
        }
        let Some(file_path) = workspace_file_path(workspace_path, changed_path)? else {
            if has_baseline_record || path.is_some() {
                unanalyzed_files.insert(changed_path.clone());
            }
            continue;
        };
        let Some(records) = workspace_document_records(
            config,
            repo_id,
            changed_path,
            &file_path,
        )? else {
            if has_baseline_record || path.is_some() {
                unanalyzed_files.insert(changed_path.clone());
            }
            continue;
        };
        remaining_files = remaining_files.saturating_sub(1);
        symbols.retain(|_, symbol| symbol.path != *changed_path);
        edges.retain(|_, edge| edge.path != *changed_path);
        let WorkspaceDocumentRecords {
            symbols: live_symbols,
            edges: live_edges,
            diagnostics: live_diagnostics,
            parser_version,
            query_pack_version,
        } = records;
        diagnostics.insert(changed_path.clone(), live_diagnostics);
        diagnostic_metadata.insert(
            changed_path.clone(),
            (parser_version, query_pack_version),
        );
        for symbol in live_symbols {
            symbols.insert(symbol.symbol_key.clone(), symbol);
        }
        for edge in live_edges {
            edges.insert(edge.edge_id.clone(), edge);
        }
    }

    re_resolve_workspace_edges(&mut edges, &symbols);
    Ok(CodeWorkspaceOverlay {
        run_id: run_id.to_string(),
        base_revision: base_revision.to_string(),
        head_revision,
        workspace_content_digest,
        base_paths: base_symbols.values().map(|symbol| symbol.path.clone()).collect(),
        base_symbols,
        base_edges,
        symbols,
        edges: edges.into_values().collect(),
        changed_paths,
        tombstones,
        unanalyzed_files: unanalyzed_files.into_iter().collect(),
        diagnostics,
        diagnostic_metadata,
        retrieval_dropped,
    })
}

fn open_workspace_graph_connection(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    base_revision: &str,
) -> Result<Connection, CodeGraphProjectionError> {
    if !workspace_path.is_dir() {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "run workspace is not available".to_string(),
        ));
    }
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::IndexUnavailable);
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)? {
        return Err(CodeGraphProjectionError::IndexUnavailable);
    }
    ensure_code_repo(config, repo_id, true)?;
    if !code_revision_indexed(&connection, config, repo_id, base_revision)? {
        return Err(CodeGraphProjectionError::RevisionNotFound(
            base_revision.to_string(),
        ));
    }
    Ok(connection)
}

fn code_graph_workspace_focused_overlay(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    run_id: &str,
    base_revision: &str,
    options: &CodeGraphSnapshotOptions,
) -> Result<CodeWorkspaceOverlay, CodeGraphProjectionError> {
    let connection = open_workspace_graph_connection(config, repo_id, workspace_path, base_revision)?;
    let head_revision = workspace_head_revision(workspace_path)?;
    let (changed_paths, tombstones, workspace_content_digest) = workspace_changed_paths(
        workspace_path,
        base_revision,
        config.code_intel.ast.max_file_bytes,
    )?;

    match options.mode {
        CodeGraphMode::File => {
            let raw_path = options.path.as_deref().ok_or_else(|| {
                CodeGraphProjectionError::InvalidRequest("file mode requires `path`".to_string())
            })?;
            let path = normalize_code_path(raw_path).map_err(CodeGraphProjectionError::InvalidRequest)?;
            let base_exists = query_revision_file_exists(
                &connection,
                config,
                repo_id,
                base_revision,
                &path,
            )?;
            let changed = changed_paths.contains(&path);
            if !base_exists && !changed {
                return Err(CodeGraphProjectionError::FileNotFound(path));
            }
            let base_symbols = if base_exists {
                query_revision_file_symbols(config, &connection, repo_id, base_revision, &path)?
            } else {
                Vec::new()
            };
            let base_edges = if base_exists {
                query_revision_file_edges(config, &connection, repo_id, base_revision, &path)?
            } else {
                Vec::new()
            };
            let mut unanalyzed_files = BTreeSet::new();
            let live_records = if tombstones.contains(&path) {
                None
            } else if changed {
                match workspace_file_path(workspace_path, &path)? {
                    Some(file_path) => match workspace_document_records(config, repo_id, &path, &file_path)? {
                        Some(records) => Some(records),
                        None => {
                            unanalyzed_files.insert(path.clone());
                            None
                        }
                    },
                    None => {
                        unanalyzed_files.insert(path.clone());
                        None
                    }
                }
            } else {
                None
            };
            let (symbols, edges) = match live_records {
                Some(records) => (records.symbols, records.edges),
                None => (base_symbols.clone(), base_edges.clone()),
            };
            let mut base_symbol_map = base_symbols
                .into_iter()
                .map(|symbol| (symbol.symbol_key.clone(), symbol))
                .collect::<BTreeMap<_, _>>();
            let mut symbol_map = symbols
                .into_iter()
                .map(|symbol| (symbol.symbol_key.clone(), symbol))
                .collect::<BTreeMap<_, _>>();
            let mut base_paths = BTreeSet::new();
            if base_exists {
                base_paths.insert(path.clone());
            }
            let mut edge_map = edges
                .into_iter()
                .map(|edge| (edge.edge_id.clone(), edge))
                .collect::<BTreeMap<_, _>>();
            re_resolve_workspace_edges(&mut edge_map, &symbol_map);
            Ok(CodeWorkspaceOverlay {
                run_id: run_id.to_string(),
                base_revision: base_revision.to_string(),
                head_revision,
                workspace_content_digest,
                base_symbols: std::mem::take(&mut base_symbol_map),
                base_paths,
                base_edges,
                symbols: std::mem::take(&mut symbol_map),
                edges: edge_map.into_values().collect(),
                changed_paths,
                tombstones,
                unanalyzed_files: unanalyzed_files.into_iter().collect(),
                diagnostics: BTreeMap::new(),
                diagnostic_metadata: BTreeMap::new(),
                retrieval_dropped: 0,
            })
        }
        CodeGraphMode::Neighborhood => {
            let center = options.symbol_key.as_deref().ok_or_else(|| {
                CodeGraphProjectionError::InvalidRequest(
                    "neighborhood mode requires `symbol_key`".to_string(),
                )
            })?;
            let (live_symbols, mut live_edges, analyzed_paths, unanalyzed_files) =
                workspace_live_changed_records(config, repo_id, workspace_path, &changed_paths, &tombstones)?;
            let mut symbols = BTreeMap::new();
            let mut base_symbols = BTreeMap::new();
            if let Some(symbol) = live_symbols.get(center) {
                symbols.insert(center.to_string(), symbol.clone());
            } else if let Some(symbol) = query_revision_symbol_by_key(
                config,
                &connection,
                repo_id,
                base_revision,
                center,
            )? {
                if tombstones.contains(&symbol.path) || analyzed_paths.contains(&symbol.path) {
                    return Err(CodeGraphProjectionError::SymbolNotFound(center.to_string()));
                }
                base_symbols.insert(center.to_string(), symbol.clone());
                symbols.insert(center.to_string(), symbol);
            } else {
                return Err(CodeGraphProjectionError::SymbolNotFound(center.to_string()));
            }

            let mut live_edge_resolution_symbols = live_symbols.clone();
            live_edge_resolution_symbols.extend(symbols.clone());
            re_resolve_workspace_edges(&mut live_edges, &live_edge_resolution_symbols);

            let mut edges = BTreeMap::new();
            let mut base_edges = BTreeMap::new();
            let mut frontier = BTreeSet::from([center.to_string()]);
            for _ in 0..options.depth.max(1) {
                let mut next = BTreeSet::new();
                for key in &frontier {
                    let baseline_edges = query_revision_edges_for_symbol(
                        config,
                        &connection,
                        repo_id,
                        base_revision,
                        key,
                    )?;
                    for edge in baseline_edges {
                        if analyzed_paths.contains(&edge.path) {
                            continue;
                        }
                        base_edges.insert(edge.edge_id.clone(), edge.clone());
                        edges.insert(edge.edge_id.clone(), edge);
                    }
                    for edge in live_edges.values().filter(|edge| {
                        edge.source_symbol_key.as_deref() == Some(key.as_str())
                            || edge.target_symbol_key.as_deref() == Some(key.as_str())
                    }) {
                        edges.insert(edge.edge_id.clone(), edge.clone());
                    }
                }

                let edge_ids = edges.keys().cloned().collect::<Vec<_>>();
                for edge_id in edge_ids {
                    let Some((source_in_frontier, target_hint)) = edges.get(&edge_id).map(|edge| {
                        (
                            edge.source_symbol_key
                                .as_ref()
                                .is_some_and(|source| frontier.contains(source)),
                            edge.target_hint.clone(),
                        )
                    }) else {
                        continue;
                    };
                    if !source_in_frontier {
                        continue;
                    }
                    let Some(name) = target_hint.as_deref().and_then(edge_target_name) else {
                        continue;
                    };
                    if edges
                        .get(&edge_id)
                        .is_some_and(|edge| edge.target_symbol_key.is_some())
                    {
                        continue;
                    }
                    let live_matches = live_symbols
                        .values()
                        .filter(|symbol| symbol.name == name)
                        .collect::<Vec<_>>();
                    let (symbol, from_baseline) = match live_matches.as_slice() {
                        [symbol] => (Some((*symbol).clone()), false),
                        [] => (
                            query_revision_symbol_by_name(
                                config,
                                &connection,
                                repo_id,
                                base_revision,
                                name,
                            )?,
                            true,
                        ),
                        _ => (None, false),
                    };
                    let Some(symbol) = symbol else {
                        continue;
                    };
                    if tombstones.contains(&symbol.path)
                        || (from_baseline && analyzed_paths.contains(&symbol.path))
                    {
                        continue;
                    }
                    base_symbols
                        .entry(symbol.symbol_key.clone())
                        .or_insert_with(|| symbol.clone());
                    if let Some(edge) = edges.get_mut(&edge_id) {
                        edge.target_symbol_key = Some(symbol.symbol_key);
                        edge.unresolved = false;
                        edge.confidence = normalize_edge_confidence(&edge.confidence, true).to_string();
                    }
                }

                for edge in edges.values() {
                    let neighbor = if edge
                        .source_symbol_key
                        .as_ref()
                        .is_some_and(|source| frontier.contains(source))
                    {
                        edge.target_symbol_key.clone()
                    } else if edge
                        .target_symbol_key
                        .as_ref()
                        .is_some_and(|target| frontier.contains(target))
                    {
                        edge.source_symbol_key.clone()
                    } else {
                        None
                    };
                    let Some(neighbor) = neighbor else {
                        continue;
                    };
                    if symbols.contains_key(&neighbor) {
                        continue;
                    }
                    let (symbol, from_baseline) = if let Some(symbol) = live_symbols.get(&neighbor) {
                        (Some(symbol.clone()), false)
                    } else if let Some(symbol) = base_symbols.get(&neighbor) {
                        (Some(symbol.clone()), true)
                    } else {
                        (
                            query_revision_symbol_by_key(
                                config,
                                &connection,
                                repo_id,
                                base_revision,
                                &neighbor,
                            )?,
                            true,
                        )
                    };
                    let Some(symbol) = symbol else {
                        continue;
                    };
                    if tombstones.contains(&symbol.path)
                        || (from_baseline && analyzed_paths.contains(&symbol.path))
                    {
                        continue;
                    }
                    base_symbols
                        .entry(neighbor.clone())
                        .or_insert_with(|| symbol.clone());
                    symbols.insert(neighbor.clone(), symbol);
                    next.insert(neighbor);
                }
                if next.is_empty() {
                    break;
                }
                frontier = next;
            }
            let mut edge_map = edges;
            re_resolve_workspace_edges(&mut edge_map, &symbols);
            Ok(CodeWorkspaceOverlay {
                run_id: run_id.to_string(),
                base_revision: base_revision.to_string(),
                head_revision,
                workspace_content_digest,
                base_symbols,
                base_paths: symbols.values().map(|symbol| symbol.path.clone()).collect(),
                base_edges: base_edges.into_values().collect(),
                symbols,
                edges: edge_map.into_values().collect(),
                changed_paths,
                tombstones,
                unanalyzed_files,
                diagnostics: BTreeMap::new(),
                diagnostic_metadata: BTreeMap::new(),
                retrieval_dropped: 0,
            })
        }
        CodeGraphMode::Atlas => unreachable!("focused overlay only handles file and neighborhood modes"),
    }
}

fn workspace_live_changed_records(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    changed_paths: &BTreeSet<String>,
    tombstones: &BTreeSet<String>,
) -> Result<WorkspaceLiveChangedRecords, CodeGraphProjectionError> {
    let mut symbols = BTreeMap::new();
    let mut edges = BTreeMap::new();
    let mut analyzed_paths = BTreeSet::new();
    let mut unanalyzed_files = BTreeSet::new();
    let mut remaining_files = config.code_intel.ast.max_files_per_request;
    for path in changed_paths {
        if tombstones.contains(path) {
            continue;
        }
        let Some(file_path) = workspace_file_path(workspace_path, path)? else {
            unanalyzed_files.insert(path.clone());
            continue;
        };
        if remaining_files == 0 {
            unanalyzed_files.insert(path.clone());
            continue;
        }
        let Some(records) = workspace_document_records(config, repo_id, path, &file_path)? else {
            unanalyzed_files.insert(path.clone());
            continue;
        };
        analyzed_paths.insert(path.clone());
        remaining_files = remaining_files.saturating_sub(1);
        for symbol in records.symbols {
            symbols.insert(symbol.symbol_key.clone(), symbol);
        }
        for edge in records.edges {
            edges.insert(edge.edge_id.clone(), edge);
        }
    }
    Ok((
        symbols,
        edges,
        analyzed_paths,
        unanalyzed_files.into_iter().collect(),
    ))
}

pub fn code_graph_workspace_diff_overlay(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    run_id: &str,
    base_revision: &str,
    max_records: usize,
) -> Result<CodeDiffOverlay, CodeGraphProjectionError> {
    let head_revision = workspace_git_line(workspace_path, ["rev-parse", "HEAD"])?;
    if !workspace_has_uncommitted_changes(workspace_path)?
        && let Some(connection) = open_existing_index_read_only(config)?
        && code_revision_indexed(&connection, config, repo_id, base_revision)?
        && code_revision_indexed(&connection, config, repo_id, &head_revision)?
    {
        return code_graph_diff_overlay(
            config,
            repo_id,
            base_revision,
            &head_revision,
            max_records,
        );
    }
    let overlay = code_graph_workspace_overlay(
        config,
        repo_id,
        workspace_path,
        run_id,
        base_revision,
    )?;
    let mut keys = overlay.base_symbols.keys().cloned().collect::<BTreeSet<_>>();
    keys.extend(overlay.symbols.keys().cloned());
    let max_records = max_records.max(1);
    let mut diffs = Vec::new();
    let mut dropped_records = 0;
    for key in keys {
        let diff = match (overlay.base_symbols.get(&key), overlay.symbols.get(&key)) {
            (None, Some(head)) => Some(CodeDiffSymbol {
                symbol_key: key,
                status: DtoDiffStatus::Added,
                before: None,
                after: Some(diff_side_from_symbol(head)),
            }),
            (Some(base), None) => Some(CodeDiffSymbol {
                symbol_key: key,
                status: DtoDiffStatus::Removed,
                before: Some(diff_side_from_symbol(base)),
                after: None,
            }),
            (Some(base), Some(head)) if base.snippet_sha256 != head.snippet_sha256 => {
                Some(CodeDiffSymbol {
                    symbol_key: key,
                    status: DtoDiffStatus::Modified,
                    before: Some(diff_side_from_symbol(base)),
                    after: Some(diff_side_from_symbol(head)),
                })
            }
            _ => None,
        };
        if let Some(diff) = diff {
            if diffs.len() == max_records {
                dropped_records += 1;
            } else {
                diffs.push(diff);
            }
        }
    }

    let (edge_deltas, dropped_edges) = compare_code_edges(
        overlay.base_edges.clone(),
        overlay.edges.clone(),
        max_records,
    );
    let (blast_radius, dropped_blast_radius) = detailed_blast_radius(
        &diffs,
        &overlay.base_edges,
        &overlay.edges,
        max_records,
    );
    let (module_connection_deltas, dropped_connections) = module_connection_deltas(
        &overlay.base_edges,
        &overlay.edges,
        &overlay.base_symbols,
        &overlay.symbols,
        max_records,
    );

    Ok(CodeDiffOverlay {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        base_revision: overlay.base_revision,
        head_revision: overlay.head_revision,
        added_symbols: diffs
            .iter()
            .filter(|diff| diff.status == DtoDiffStatus::Added)
            .cloned()
            .collect(),
        removed_symbols: diffs
            .iter()
            .filter(|diff| diff.status == DtoDiffStatus::Removed)
            .cloned()
            .collect(),
        modified_symbols: diffs
            .iter()
            .filter(|diff| diff.status == DtoDiffStatus::Modified)
            .cloned()
            .collect(),
        edge_deltas,
        module_connection_deltas,
        blast_radius,
        unanalyzed_files: overlay.unanalyzed_files,
        truncation: truncation(
            dropped_records,
            dropped_edges
                .saturating_add(dropped_blast_radius)
                .saturating_add(dropped_connections),
            "workspace overlay symbols or edges capped",
        ),
        generated_at: Utc::now(),
    })
}

pub fn code_file_outline_from_workspace(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    run_id: &str,
    base_revision: &str,
    raw_path: &str,
) -> Result<CodeFileOutline, CodeGraphProjectionError> {
    let path = normalize_code_path(raw_path).map_err(CodeGraphProjectionError::InvalidRequest)?;
    let Some(file_path) = workspace_outline_file_path(workspace_path, &path)? else {
        return Err(CodeGraphProjectionError::FileNotFound(path));
    };
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Err(CodeGraphProjectionError::IndexUnavailable);
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)? {
        return Err(CodeGraphProjectionError::IndexUnavailable);
    }
    ensure_code_repo(config, repo_id, true)?;
    if !code_revision_indexed(&connection, config, repo_id, base_revision)? {
        return Err(CodeGraphProjectionError::RevisionNotFound(
            base_revision.to_string(),
        ));
    }

    let changed = workspace_path_changed(workspace_path, base_revision, &path)?;
    let symbols = if changed {
        match workspace_document_records(config, repo_id, &path, &file_path)? {
            Some(records) => records.symbols,
            None => query_revision_file_symbols(
                config,
                &connection,
                repo_id,
                base_revision,
                &path,
            )?,
        }
    } else if query_revision_file_exists(
        &connection,
        config,
        repo_id,
        base_revision,
        &path,
    )? {
        query_revision_file_symbols(config, &connection, repo_id, base_revision, &path)?
    } else {
        return Err(CodeGraphProjectionError::FileNotFound(path));
    };
    let symbols = symbols
        .iter()
        .map(|symbol| CodeOutlineSymbol {
            symbol_key: symbol.symbol_key.clone(),
            name: symbol.name.clone(),
            kind: symbol.kind.clone(),
            path: symbol.path.clone(),
            span: span_from_symbol(symbol),
            selection_span: selection_span_from_symbol(symbol),
            container_chain: symbol.container_chain.clone(),
        })
        .collect();
    Ok(CodeFileOutline {
        schema_version: SchemaVersion::v1(),
        run_id: run_id.to_string(),
        repo_id: Some(repo_id.to_string()),
        path,
        symbols,
        generated_at: Utc::now(),
    })
}

pub fn code_graph_workspace_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    workspace_path: &Path,
    run_id: &str,
    base_revision: &str,
    options: CodeGraphSnapshotOptions,
) -> Result<CodeGraphSnapshot, CodeGraphProjectionError> {
    if options.aggregate == Some(CodeGraphAggregate::Community) {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "community aggregation is not available for workspace code graphs".to_string(),
        ));
    }
    let mode = options.mode;
    if matches!(mode, CodeGraphMode::File | CodeGraphMode::Neighborhood) {
        let overlay = code_graph_workspace_focused_overlay(
            config,
            repo_id,
            workspace_path,
            run_id,
            base_revision,
            &options,
        )?;
        return render_workspace_graph_snapshot(config, repo_id, options, overlay);
    }
    let overlay = code_graph_workspace_overlay(
        config,
        repo_id,
        workspace_path,
        run_id,
        base_revision,
    )?;
    render_workspace_graph_snapshot(config, repo_id, options, overlay)
}

fn render_workspace_graph_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    options: CodeGraphSnapshotOptions,
    overlay: CodeWorkspaceOverlay,
) -> Result<CodeGraphSnapshot, CodeGraphProjectionError> {
    let connection = open_existing_index_read_only(config)?;
    let diagnostics_ready = if let Some(connection) = connection.as_ref() {
        code_diagnostics_read_model_ready(connection, &config.index_path)?
    } else {
        false
    };
    let mode = options.mode;
    let mut paths = BTreeSet::new();
    let mut selected_keys = BTreeSet::new();
    let mut dropped_paths = 0;
    let mut tombstoned_file = false;
    match mode {
        CodeGraphMode::Atlas => {
            let changed_paths = overlay
                .changed_paths
                .iter()
                .filter(|path| !overlay.tombstones.contains(*path))
                .cloned()
                .collect::<BTreeSet<_>>();
            let prioritized_paths = changed_paths
                .iter()
                .cloned()
                .chain(
                    overlay
                        .base_paths
                        .iter()
                        .filter(|path| !changed_paths.contains(*path))
                        .cloned(),
                )
                .collect::<Vec<_>>();
            dropped_paths = prioritized_paths.len().saturating_sub(CODE_GRAPH_MAX_RECORDS);
            paths = prioritized_paths
                .into_iter()
                .take(CODE_GRAPH_MAX_RECORDS)
                .collect();
            if options.aggregate != Some(CodeGraphAggregate::Directory) {
                selected_keys.extend(
                    overlay
                        .symbols
                        .iter()
                        .filter(|(_, symbol)| paths.contains(&symbol.path))
                        .map(|(key, _)| key.clone()),
                );
            }
        }
        CodeGraphMode::File => {
            let path = options.path.as_deref().ok_or_else(|| {
                CodeGraphProjectionError::InvalidRequest("file mode requires `path`".to_string())
            })?;
            let path = normalize_code_path(path).map_err(CodeGraphProjectionError::InvalidRequest)?;
            tombstoned_file = overlay.tombstones.contains(&path);
            if !overlay.symbols.values().any(|symbol| symbol.path == path)
                && !overlay.changed_paths.contains(&path)
                && !overlay.base_paths.contains(&path)
            {
                return Err(CodeGraphProjectionError::FileNotFound(path));
            }
            paths.insert(path.clone());
            if tombstoned_file {
                selected_keys.extend(
                    overlay
                        .base_symbols
                        .values()
                        .filter(|symbol| symbol.path == path)
                        .map(|symbol| symbol.symbol_key.clone()),
                );
            } else {
                selected_keys.extend(
                    overlay
                        .symbols
                        .values()
                        .filter(|symbol| symbol.path == path)
                        .map(|symbol| symbol.symbol_key.clone()),
                );
            }
        }
        CodeGraphMode::Neighborhood => {
            let center = options.symbol_key.as_deref().ok_or_else(|| {
                CodeGraphProjectionError::InvalidRequest(
                    "neighborhood mode requires `symbol_key`".to_string(),
                )
            })?;
            if !overlay.symbols.contains_key(center) {
                return Err(CodeGraphProjectionError::SymbolNotFound(center.to_string()));
            }
            selected_keys.insert(center.to_string());
            let mut frontier = BTreeSet::from([center.to_string()]);
            for _ in 0..options.depth.max(1) {
                let mut next = BTreeSet::new();
                for edge in &overlay.edges {
                    let source = edge.source_symbol_key.as_deref();
                    let target = edge.target_symbol_key.as_deref();
                    if source.is_some_and(|source| frontier.contains(source))
                        && let Some(target) =
                            target.filter(|target| overlay.symbols.contains_key(*target))
                    {
                        next.insert(target.to_string());
                    }
                    if target.is_some_and(|target| frontier.contains(target))
                        && let Some(source) =
                            source.filter(|source| overlay.symbols.contains_key(*source))
                    {
                        next.insert(source.to_string());
                    }
                }
                next.retain(|key| !selected_keys.contains(key));
                if next.is_empty() {
                    break;
                }
                selected_keys.extend(next.iter().cloned());
                frontier = next;
            }
        }
    }

    let dropped_symbols = selected_keys.len().saturating_sub(CODE_GRAPH_MAX_RECORDS);
    selected_keys = selected_keys
        .into_iter()
        .take(CODE_GRAPH_MAX_RECORDS)
        .collect();
    for key in &selected_keys {
        let symbol = if tombstoned_file {
            overlay.base_symbols.get(key)
        } else {
            overlay.symbols.get(key)
        };
        if let Some(symbol) = symbol {
            paths.insert(symbol.path.clone());
        }
    }

    let mut nodes = BTreeMap::new();
    let mut graph_edges = BTreeMap::new();
    for path in paths {
        if overlay.tombstones.contains(&path) && !tombstoned_file {
            continue;
        }
        let language = overlay
            .symbols
            .values()
            .find(|symbol| symbol.path == path)
            .map(|symbol| symbol.language.clone())
            .or_else(|| detect_language(Path::new(&path)).map(|language| language.id().to_string()));
        insert_path_nodes(
            &mut nodes,
            &mut graph_edges,
            &path,
            language,
            if tombstoned_file {
                CodeGraphFreshness::Stale
            } else {
                CodeGraphFreshness::Current
            },
        );
    }
    for key in &selected_keys {
        let symbol = if tombstoned_file {
            overlay.base_symbols.get(key)
        } else {
            overlay.symbols.get(key)
        };
        let Some(symbol) = symbol else {
            continue;
        };
        let mut node = if diagnostics_ready {
            symbol_node(
                connection.as_ref().expect("diagnostics require an index connection"),
                config,
                symbol,
                options.include_stale,
            )?
        } else {
            workspace_symbol_node(symbol)
        };
        if tombstoned_file {
            node.freshness = CodeGraphFreshness::Stale;
        }
        let file_id = file_node_id(&symbol.path);
        insert_code_graph_edge(
            &mut graph_edges,
            "contains".to_string(),
            file_id,
            node.id.clone(),
            CodeGraphConfidence::Exact,
            false,
            None,
        );
        nodes.insert(node.id.clone(), node);
    }

    let mut dropped_edges = 0;
    let graph_source_edges = if tombstoned_file {
        &overlay.base_edges
    } else {
        &overlay.edges
    };
    for edge in graph_source_edges {
        let Some(source) = edge.source_symbol_key.as_deref() else {
            continue;
        };
        if !selected_keys.contains(source) {
            continue;
        }
        let target_id = match edge.target_symbol_key.as_deref() {
            Some(target) if selected_keys.contains(target) => symbol_node_id(target),
            Some(_) => continue,
            None => format!("hint:{}", edge.edge_id),
        };
        if graph_edges.len() >= CODE_GRAPH_MAX_RECORDS {
            dropped_edges += 1;
            continue;
        }
        if edge.target_symbol_key.is_none() {
            nodes
                .entry(target_id.clone())
                .or_insert_with(|| hint_node(&target_id, edge.target_hint.as_deref()));
        }
        insert_code_graph_edge(
            &mut graph_edges,
            edge.edge_kind.clone(),
            symbol_node_id(source),
            target_id,
            confidence_from_str(&edge.confidence),
            edge.unresolved,
            edge.target_hint.clone(),
        );
    }

    let mut nodes = nodes.into_values().collect::<Vec<_>>();
    let edges = graph_edges.into_values().collect::<Vec<_>>();
    apply_code_node_metrics(&mut nodes, &edges);
    Ok(snapshot(
        repo_id,
        mode,
        nodes,
        edges,
        options.include_stale,
        options.aggregate,
        truncation(
            dropped_paths + dropped_symbols,
            dropped_edges,
            "workspace graph records capped",
        ),
    ))
}

fn workspace_document_records(
    config: &MemoryConfig,
    repo_id: &str,
    path: &str,
    file_path: &Path,
) -> Result<Option<WorkspaceDocumentRecords>, CodeGraphProjectionError> {
    let Some(language) = detect_language(Path::new(path)) else {
        return Ok(None);
    };
    let Ok(metadata) = fs::metadata(file_path) else {
        return Ok(None);
    };
    if metadata.len() > config.code_intel.ast.max_file_bytes {
        return Ok(None);
    }
    let Ok(source) = fs::read_to_string(file_path) else {
        return Ok(None);
    };
    let content_sha256 = sha256_hex(&source);
    let key = WorkspaceDocumentCacheKey {
        repo_id: repo_id.to_string(),
        path: path.to_string(),
        content_sha256,
        max_capture_bytes: config.code_intel.ast.max_capture_bytes,
        max_matches: config.code_intel.ast.max_matches_per_request,
    };
    if let Some(cached) = WORKSPACE_DOCUMENT_CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| CodeGraphProjectionError::InvalidRequest("workspace overlay cache poisoned".to_string()))?
        .get(&key)
        .cloned()
    {
        return Ok(Some(cached));
    }

    let Ok(summary) = parse_path(Path::new(path), &source) else {
        return Ok(None);
    };
    let mut remaining_matches = config.code_intel.ast.max_matches_per_request;
    let document = code_index_document(
        PathBuf::from(path),
        &source,
        &summary,
        &mut remaining_matches,
        config.code_intel.ast.max_capture_bytes,
    );
    let prepared = prepare_code_symbols(repo_id, None, true, path, &document);
    let indexed_at = Utc::now().to_rfc3339();
    let symbols = prepared
        .iter()
        .map(|prepared| {
            let symbol = prepared.symbol;
            CodeSymbolRecord {
                symbol_id: prepared.symbol_id.clone(),
                symbol_key: prepared.symbol_key.clone(),
                repo_id: repo_id.to_string(),
                commit_sha: None,
                path: path.to_string(),
                language: language.id().to_string(),
                kind: symbol.kind.clone(),
                name: symbol.name.clone(),
                container_symbol_id: prepared.container_symbol_id.clone(),
                container_chain: symbol.container_chain.clone(),
                signature: symbol.signature.clone(),
                start_line: symbol.start_line,
                start_col: symbol.start_col,
                end_line: symbol.end_line,
                end_col: symbol.end_col,
                start_byte: symbol.start_byte,
                end_byte: symbol.end_byte,
                selection_start_line: symbol.selection_start_line,
                selection_end_line: symbol.selection_end_line,
                content_sha256: document.content_sha256.clone(),
                snippet_sha256: symbol.snippet_sha256.clone(),
                parser_version: document.parser_version.clone(),
                query_pack_version: document.query_pack_version.clone(),
                freshness: "current".to_string(),
                indexed_at: indexed_at.clone(),
            }
        })
        .collect::<Vec<_>>();
    let edges = document
        .edges
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            let resolved = resolve_code_edge(edge, &prepared);
            CodeEdgeRecord {
                edge_id: code_row_id(&[
                    repo_id,
                    path,
                    &document.content_sha256,
                    &document.parser_version,
                    &document.query_pack_version,
                    &edge.edge_kind,
                    edge.target_hint.as_deref().unwrap_or(""),
                    &edge.start_line.to_string(),
                    &edge.start_col.to_string(),
                    &edge.end_line.to_string(),
                    &edge.end_col.to_string(),
                    &edge.start_byte.to_string(),
                    &edge.end_byte.to_string(),
                    &index.to_string(),
                ]),
                edge_key: String::new(),
                edge_kind: edge.edge_kind.clone(),
                source_symbol_key: resolved.source_symbol_key,
                target_symbol_key: resolved.target_symbol_key,
                target_hint: edge.target_hint.clone(),
                confidence: normalize_edge_confidence(&edge.confidence, resolved.target_resolved).to_string(),
                unresolved: !resolved.target_resolved,
                path: path.to_string(),
                commit_sha: None,
                freshness: "current".to_string(),
                start_line: edge.start_line,
                start_col: edge.start_col,
                end_line: edge.end_line,
                end_col: edge.end_col,
            }
        })
        .filter(|edge| edge.source_symbol_key.is_some())
        .collect::<Vec<_>>();
    let diagnostics = document
        .diagnostics
        .iter()
        .map(|diagnostic| CodeDiagnostic {
            kind: diagnostic.kind.clone(),
            severity: diagnostic.severity.clone(),
            message: diagnostic.message.clone(),
            span: CodeSpan {
                start_line: diagnostic.start_line,
                start_col: diagnostic.start_col,
                end_line: diagnostic.end_line,
                end_col: diagnostic.end_col,
            },
        })
        .collect();
    let records = WorkspaceDocumentRecords {
        symbols,
        edges,
        diagnostics,
        parser_version: document.parser_version,
        query_pack_version: document.query_pack_version,
    };
    let mut cache = WORKSPACE_DOCUMENT_CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| CodeGraphProjectionError::InvalidRequest("workspace overlay cache poisoned".to_string()))?;
    if cache.len() >= 128
        && let Some(first) = cache.keys().next().cloned()
    {
        cache.remove(&first);
    }
    cache.insert(key, records.clone());
    Ok(Some(records))
}

fn workspace_changed_paths(
    workspace_path: &Path,
    base_revision: &str,
    max_file_bytes: u64,
) -> Result<(BTreeSet<String>, BTreeSet<String>, String), CodeGraphProjectionError> {
    let diff = workspace_git_bytes(
        workspace_path,
        ["diff", "--name-status", "--find-renames", "-z", base_revision, "--"],
    )?;
    let mut changed = BTreeSet::new();
    let mut tombstones = BTreeSet::new();
    let fields = diff
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .map(|field| String::from_utf8_lossy(field).into_owned())
        .collect::<Vec<_>>();
    let mut index = 0;
    while index < fields.len() {
        let status = &fields[index];
        index += 1;
        if status.starts_with('R') || status.starts_with('C') {
            let old = fields.get(index).cloned().unwrap_or_default();
            let new = fields.get(index + 1).cloned().unwrap_or_default();
            index += 2;
            if !old.is_empty() {
                changed.insert(old.clone());
                tombstones.insert(old);
            }
            if !new.is_empty() {
                changed.insert(new);
            }
        } else if let Some(path) = fields.get(index) {
            index += 1;
            if !path.is_empty() {
                changed.insert(path.clone());
                if status.starts_with('D') {
                    tombstones.insert(path.clone());
                }
            }
        }
    }
    for path in String::from_utf8_lossy(&workspace_git_bytes(
        workspace_path,
        ["ls-files", "--others", "--exclude-standard", "-z"],
    )?)
    .split('\0')
    .filter(|path| !path.is_empty())
    {
        changed.insert(path.to_string());
    }

    let mut hasher = Sha256::new();
    for path in &changed {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        match workspace_file_path(workspace_path, path)? {
            Some(file_path) => {
                let Ok(metadata) = fs::metadata(&file_path) else {
                    hasher.update(b"<unreadable>");
                    hasher.update([0]);
                    continue;
                };
                if metadata.len() > max_file_bytes {
                    hasher.update(b"<oversized>");
                    hasher.update(metadata.len().to_le_bytes());
                    hasher.update([0]);
                    continue;
                }
                let mut file = match fs::File::open(file_path) {
                    Ok(file) => file,
                    Err(_) => {
                        hasher.update(b"<unreadable>");
                        hasher.update([0]);
                        continue;
                    }
                };
                let mut buffer = [0_u8; 64 * 1024];
                loop {
                    match io::Read::read(&mut file, &mut buffer) {
                        Ok(0) => break,
                        Ok(read) => hasher.update(&buffer[..read]),
                        Err(_) => {
                            hasher.update(b"<unreadable>");
                            break;
                        }
                    }
                }
            }
            None => hasher.update(b"<deleted-or-outside-workspace>"),
        }
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let digest = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok((changed, tombstones, digest))
}

fn workspace_path_changed(
    workspace_path: &Path,
    base_revision: &str,
    path: &str,
) -> Result<bool, CodeGraphProjectionError> {
    if !workspace_git_bytes(
        workspace_path,
        ["diff", "--name-only", base_revision, "--", path],
    )?
    .is_empty()
    {
        return Ok(true);
    }
    Ok(!workspace_git_bytes(
        workspace_path,
        ["ls-files", "--others", "--exclude-standard", "--", path],
    )?
    .is_empty())
}

fn workspace_file_path(
    workspace_path: &Path,
    path: &str,
) -> Result<Option<PathBuf>, CodeGraphProjectionError> {
    workspace_file_path_with_symlink_policy(workspace_path, path, false)
}

fn workspace_outline_file_path(
    workspace_path: &Path,
    path: &str,
) -> Result<Option<PathBuf>, CodeGraphProjectionError> {
    workspace_file_path_with_symlink_policy(workspace_path, path, true)
}

fn workspace_file_path_with_symlink_policy(
    workspace_path: &Path,
    path: &str,
    allow_contained_symlink: bool,
) -> Result<Option<PathBuf>, CodeGraphProjectionError> {
    let relative = normalize_code_path(path).map_err(CodeGraphProjectionError::InvalidRequest)?;
    let root = workspace_path
        .canonicalize()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: workspace_path.to_path_buf(),
            source,
        }))?;
    let candidate = workspace_path.join(relative);
    let Ok(metadata) = fs::symlink_metadata(&candidate) else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() && !allow_contained_symlink {
        return Ok(None);
    }
    let Ok(resolved) = candidate.canonicalize() else {
        return Ok(None);
    };
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Ok(None);
    }
    Ok(Some(resolved))
}

fn workspace_git_line<const N: usize>(
    workspace_path: &Path,
    args: [&str; N],
) -> Result<String, CodeGraphProjectionError> {
    Ok(String::from_utf8_lossy(&workspace_git_bytes(workspace_path, args)?)
        .trim()
        .to_string())
}

fn workspace_has_uncommitted_changes(
    workspace_path: &Path,
) -> Result<bool, CodeGraphProjectionError> {
    Ok(!workspace_git_bytes(workspace_path, ["status", "--porcelain", "--untracked-files=all"])?
        .is_empty())
}

fn workspace_head_revision(workspace_path: &Path) -> Result<String, CodeGraphProjectionError> {
    let head_revision = workspace_git_line(workspace_path, ["rev-parse", "HEAD"])?;
    if workspace_has_uncommitted_changes(workspace_path)? {
        Ok(format!("{head_revision}+worktree"))
    } else {
        Ok(head_revision)
    }
}

fn workspace_git_bytes<const N: usize>(
    workspace_path: &Path,
    args: [&str; N],
) -> Result<Vec<u8>, CodeGraphProjectionError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace_path)
        .args(args)
        .output()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: workspace_path.to_path_buf(),
            source,
        }))?;
    if !output.status.success() {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "git command failed in run workspace".to_string(),
        ));
    }
    Ok(output.stdout)
}

fn re_resolve_workspace_edges(
    edges: &mut BTreeMap<String, CodeEdgeRecord>,
    symbols: &BTreeMap<String, CodeSymbolRecord>,
) {
    let mut names = BTreeMap::<String, Option<String>>::new();
    for symbol in symbols.values() {
        names
            .entry(symbol.name.clone())
            .and_modify(|value| *value = None)
            .or_insert_with(|| Some(symbol.symbol_key.clone()));
    }
    edges.retain(|_, edge| {
        if edge
            .source_symbol_key
            .as_deref()
            .is_none_or(|source| !symbols.contains_key(source))
        {
            return false;
        }
        let was_unresolved = edge.target_symbol_key.is_none() && edge.unresolved;
        if edge
            .target_symbol_key
            .as_deref()
            .is_some_and(|target| !symbols.contains_key(target))
        {
            edge.target_symbol_key = None;
            edge.unresolved = true;
        }
        if was_unresolved
            && edge.target_symbol_key.is_none()
            && let Some(name) = edge
                .target_hint
                .as_deref()
                .and_then(edge_target_name)
            && let Some(Some(target)) = names.get(name)
        {
            edge.target_symbol_key = Some(target.clone());
            edge.unresolved = false;
            edge.confidence = normalize_edge_confidence(&edge.confidence, true).to_string();
        }
        true
    });
}

fn is_blast_radius_edge(edge: &CodeEdgeRecord) -> bool {
    let kind = edge.edge_kind.to_ascii_lowercase();
    kind.contains("call") || kind.contains("reference")
}

fn normalized_edge_hint(hint: Option<&str>) -> String {
    let hint = hint.unwrap_or_default().trim();
    let hint = edge_target_name(hint).unwrap_or(hint);
    hint.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn logical_edge_base(edge: &CodeEdgeRecord) -> String {
    let target_identity = edge
        .target_symbol_key
        .clone()
        .unwrap_or_else(|| normalized_edge_hint(edge.target_hint.as_deref()));
    code_row_id(&[
        "logical-edge",
        &edge.edge_kind,
        edge.source_symbol_key.as_deref().unwrap_or_default(),
        &target_identity,
    ])
}

fn assign_edge_keys(edges: &mut Vec<CodeEdgeRecord>) {
    edges.sort_by(|left, right| {
        (
            logical_edge_base(left),
            left.path.as_str(),
            left.start_line,
            left.start_col,
            left.end_line,
            left.end_col,
            left.edge_id.as_str(),
        )
            .cmp(&(
                logical_edge_base(right),
                right.path.as_str(),
                right.start_line,
                right.start_col,
                right.end_line,
                right.end_col,
                right.edge_id.as_str(),
            ))
    });
    let mut ordinals = BTreeMap::<String, usize>::new();
    for edge in edges {
        let base = logical_edge_base(edge);
        let ordinal = ordinals.entry(base.clone()).or_default();
        *ordinal += 1;
        edge.edge_key = if *ordinal == 1 {
            base
        } else {
            format!("{base}#{ordinal}")
        };
    }
}

fn edge_slot(edge: &CodeEdgeRecord) -> (String, String) {
    (
        edge.edge_kind.clone(),
        edge.source_symbol_key.clone().unwrap_or_default(),
    )
}

fn code_diff_edge_side(edge: &CodeEdgeRecord) -> CodeDiffEdgeSide {
    CodeDiffEdgeSide {
        edge_id: edge.edge_id.clone(),
        kind: edge.edge_kind.clone(),
        source_symbol_key: edge.source_symbol_key.clone(),
        target_symbol_key: edge.target_symbol_key.clone(),
        target_hint: edge.target_hint.clone(),
        confidence: confidence_from_str(&edge.confidence),
        unresolved: edge.unresolved,
        path: edge.path.clone(),
        span: CodeSpan {
            start_line: edge.start_line,
            start_col: edge.start_col,
            end_line: edge.end_line,
            end_col: edge.end_col,
        },
    }
}

fn edge_targets_differ(left: &CodeEdgeRecord, right: &CodeEdgeRecord) -> bool {
    match (
        left.target_symbol_key.as_deref(),
        right.target_symbol_key.as_deref(),
    ) {
        (Some(target_left), Some(target_right)) => {
            target_left != target_right || left.unresolved != right.unresolved
        }
        (None, None) => {
            normalized_edge_hint(left.target_hint.as_deref())
                != normalized_edge_hint(right.target_hint.as_deref())
                || left.unresolved != right.unresolved
        }
        _ => true,
    }
}

fn edge_delta_status(left: &CodeEdgeRecord, right: &CodeEdgeRecord) -> Option<CodeDiffEdgeStatus> {
    if edge_targets_differ(left, right) {
        Some(CodeDiffEdgeStatus::Retargeted)
    } else if confidence_from_str(&left.confidence) != confidence_from_str(&right.confidence) {
        Some(CodeDiffEdgeStatus::ConfidenceChanged)
    } else {
        None
    }
}

fn compare_code_edges(
    mut base: Vec<CodeEdgeRecord>,
    mut head: Vec<CodeEdgeRecord>,
    max_records: usize,
) -> (Vec<CodeDiffEdge>, usize) {
    assign_edge_keys(&mut base);
    assign_edge_keys(&mut head);
    let base_by_key = base
        .iter()
        .map(|edge| (edge.edge_key.clone(), edge))
        .collect::<BTreeMap<_, _>>();
    let head_by_key = head
        .iter()
        .map(|edge| (edge.edge_key.clone(), edge))
        .collect::<BTreeMap<_, _>>();
    let mut deltas = Vec::new();
    let mut dropped = 0;
    let mut unmatched_base = Vec::new();
    let mut unmatched_head = Vec::new();

    let mut keys = BTreeSet::new();
    keys.extend(base_by_key.keys().cloned());
    keys.extend(head_by_key.keys().cloned());
    for key in keys {
        match (base_by_key.get(&key), head_by_key.get(&key)) {
            (Some(before), Some(after)) => {
                if let Some(status) = edge_delta_status(before, after) {
                    push_edge_delta(
                        &mut deltas,
                        &mut dropped,
                        max_records,
                        CodeDiffEdge {
                            edge_key: key,
                            status,
                            before: Some(code_diff_edge_side(before)),
                            after: Some(code_diff_edge_side(after)),
                        },
                    );
                }
            }
            (Some(before), None) => unmatched_base.push(*before),
            (None, Some(after)) => unmatched_head.push(*after),
            (None, None) => unreachable!(),
        }
    }

    unmatched_base.sort_by(|left, right| left.edge_key.cmp(&right.edge_key));
    unmatched_head.sort_by(|left, right| left.edge_key.cmp(&right.edge_key));
    let mut base_slots = BTreeMap::<(String, String), Vec<&CodeEdgeRecord>>::new();
    let mut head_slots = BTreeMap::<(String, String), Vec<&CodeEdgeRecord>>::new();
    for edge in &unmatched_base {
        base_slots.entry(edge_slot(edge)).or_default().push(edge);
    }
    for edge in &unmatched_head {
        head_slots.entry(edge_slot(edge)).or_default().push(edge);
    }
    let mut slots = BTreeSet::new();
    slots.extend(base_slots.keys().cloned());
    slots.extend(head_slots.keys().cloned());
    for slot in slots {
        let before = base_slots.remove(&slot).unwrap_or_default();
        let after = head_slots.remove(&slot).unwrap_or_default();
        let pairs = before.len().min(after.len());
        for index in 0..pairs {
            let before = before[index];
            let after = after[index];
            push_edge_delta(
                &mut deltas,
                &mut dropped,
                max_records,
                CodeDiffEdge {
                    edge_key: after.edge_key.clone(),
                    status: CodeDiffEdgeStatus::Retargeted,
                    before: Some(code_diff_edge_side(before)),
                    after: Some(code_diff_edge_side(after)),
                },
            );
        }
        for edge in before.into_iter().skip(pairs) {
            push_edge_delta(
                &mut deltas,
                &mut dropped,
                max_records,
                CodeDiffEdge {
                    edge_key: edge.edge_key.clone(),
                    status: CodeDiffEdgeStatus::Removed,
                    before: Some(code_diff_edge_side(edge)),
                    after: None,
                },
            );
        }
        for edge in after.into_iter().skip(pairs) {
            push_edge_delta(
                &mut deltas,
                &mut dropped,
                max_records,
                CodeDiffEdge {
                    edge_key: edge.edge_key.clone(),
                    status: CodeDiffEdgeStatus::Added,
                    before: None,
                    after: Some(code_diff_edge_side(edge)),
                },
            );
        }
    }
    deltas.sort_by(|left, right| {
        (left.status, left.edge_key.as_str()).cmp(&(right.status, right.edge_key.as_str()))
    });
    (deltas, dropped)
}

fn push_edge_delta(
    deltas: &mut Vec<CodeDiffEdge>,
    dropped: &mut usize,
    max_records: usize,
    delta: CodeDiffEdge,
) {
    if deltas.len() >= max_records.max(1) {
        *dropped += 1;
    } else {
        deltas.push(delta);
    }
}

fn blast_radius_entry(
    edge: &CodeEdgeRecord,
    related_symbol_key: Option<String>,
) -> CodeDiffBlastRadiusEntry {
    CodeDiffBlastRadiusEntry {
        edge_key: edge.edge_key.clone(),
        symbol_key: related_symbol_key,
        path: edge.path.clone(),
        edge_kind: edge.edge_kind.clone(),
        confidence: confidence_from_str(&edge.confidence),
        distance: 1,
    }
}

fn detailed_blast_radius(
    diffs: &[CodeDiffSymbol],
    base_edges: &[CodeEdgeRecord],
    head_edges: &[CodeEdgeRecord],
    max_records: usize,
) -> (Vec<CodeDiffBlastRadius>, usize) {
    let changed_keys = diffs
        .iter()
        .map(|diff| diff.symbol_key.as_str())
        .collect::<BTreeSet<_>>();
    let mut base_edges = base_edges.to_vec();
    let mut head_edges = head_edges.to_vec();
    assign_edge_keys(&mut base_edges);
    assign_edge_keys(&mut head_edges);
    let mut radius = Vec::new();
    let mut kept_entries = 0;
    let mut dropped_entries = 0;
    let max_entries = max_records.max(1);
    for diff in diffs {
        let edges = if diff.status == DtoDiffStatus::Removed {
            &base_edges
        } else {
            &head_edges
        };
        let mut inbound = edges
            .iter()
            .filter(|edge| {
                is_blast_radius_edge(edge)
                    && edge.target_symbol_key.as_deref() == Some(diff.symbol_key.as_str())
                    && edge
                        .source_symbol_key
                        .as_deref()
                        .is_none_or(|source| !changed_keys.contains(source))
            })
            .map(|edge| blast_radius_entry(edge, edge.source_symbol_key.clone()))
            .collect::<Vec<_>>();
        let mut outbound = edges
            .iter()
            .filter(|edge| {
                is_blast_radius_edge(edge)
                    && edge.source_symbol_key.as_deref() == Some(diff.symbol_key.as_str())
            })
            .map(|edge| blast_radius_entry(edge, edge.target_symbol_key.clone()))
            .collect::<Vec<_>>();
        inbound.sort_by(|left, right| {
            (left.path.as_str(), left.edge_key.as_str())
                .cmp(&(right.path.as_str(), right.edge_key.as_str()))
        });
        outbound.sort_by(|left, right| {
            (left.path.as_str(), left.edge_key.as_str())
                .cmp(&(right.path.as_str(), right.edge_key.as_str()))
        });
        let inbound_count = inbound.len();
        let outbound_count = outbound.len();
        let total_entries = inbound_count + outbound_count;
        let remaining = max_entries.saturating_sub(kept_entries);
        let keep_inbound = inbound.len().min(remaining);
        let keep_outbound = outbound.len().min(remaining.saturating_sub(keep_inbound));
        dropped_entries += total_entries.saturating_sub(keep_inbound + keep_outbound);
        kept_entries += keep_inbound + keep_outbound;
        inbound.truncate(keep_inbound);
        outbound.truncate(keep_outbound);
        if total_entries > 0 {
            radius.push(CodeDiffBlastRadius {
                symbol_key: diff.symbol_key.clone(),
                inbound_count,
                outbound_count,
                inbound,
                outbound,
            });
        }
    }
    (radius, dropped_entries)
}

#[derive(Clone, Default)]
struct ModuleConnectionAccumulator {
    edge_count: usize,
    edge_kind_counts: BTreeMap<String, usize>,
    confidence_counts: BTreeMap<CodeGraphConfidence, usize>,
    unresolved_count: usize,
}

fn connection_scope_name(scope: CodeDiffConnectionScope) -> &'static str {
    match scope {
        CodeDiffConnectionScope::Directory => "directory",
        CodeDiffConnectionScope::Module => "module",
        CodeDiffConnectionScope::Community => "community",
    }
}

fn connection_endpoint(
    scope: CodeDiffConnectionScope,
    symbol_key: Option<&str>,
    hint: Option<&str>,
    symbols: &BTreeMap<String, CodeSymbolRecord>,
) -> String {
    let Some(symbol_key) = symbol_key else {
        return format!("unresolved:{}", normalized_edge_hint(hint));
    };
    let Some(symbol) = symbols.get(symbol_key) else {
        return format!("symbol:{symbol_key}");
    };
    match scope {
        CodeDiffConnectionScope::Directory => symbol
            .path
            .rsplit_once('/')
            .map(|(directory, _)| directory.to_string())
            .unwrap_or_else(|| ".".to_string()),
        CodeDiffConnectionScope::Module => {
            if symbol.container_chain.is_empty() {
                symbol.path.clone()
            } else {
                format!("{}::{}", symbol.path, symbol.container_chain.join("::"))
            }
        }
        CodeDiffConnectionScope::Community => symbol
            .path
            .split('/')
            .next()
            .unwrap_or(".")
            .to_string(),
    }
}

fn aggregate_module_connections(
    edges: &[CodeEdgeRecord],
    symbols: &BTreeMap<String, CodeSymbolRecord>,
) -> BTreeMap<(CodeDiffConnectionScope, String, String), ModuleConnectionAccumulator> {
    let mut aggregated =
        BTreeMap::<(CodeDiffConnectionScope, String, String), ModuleConnectionAccumulator>::new();
    for edge in edges {
        for scope in [
            CodeDiffConnectionScope::Directory,
            CodeDiffConnectionScope::Module,
            CodeDiffConnectionScope::Community,
        ] {
            let source = connection_endpoint(
                scope,
                edge.source_symbol_key.as_deref(),
                None,
                symbols,
            );
            let target = connection_endpoint(
                scope,
                edge.target_symbol_key.as_deref(),
                edge.target_hint.as_deref(),
                symbols,
            );
            let aggregate = aggregated.entry((scope, source, target)).or_default();
            aggregate.edge_count += 1;
            *aggregate
                .edge_kind_counts
                .entry(edge.edge_kind.clone())
                .or_default() += 1;
            *aggregate
                .confidence_counts
                .entry(confidence_from_str(&edge.confidence))
                .or_default() += 1;
            if edge.unresolved {
                aggregate.unresolved_count += 1;
            }
        }
    }
    aggregated
}

fn module_connection_side(
    aggregate: ModuleConnectionAccumulator,
) -> CodeDiffModuleConnectionSide {
    CodeDiffModuleConnectionSide {
        edge_count: aggregate.edge_count,
        edge_kind_counts: aggregate
            .edge_kind_counts
            .into_iter()
            .map(|(kind, count)| CodeDiffCountByKind { kind, count })
            .collect(),
        confidence_counts: aggregate
            .confidence_counts
            .into_iter()
            .map(|(confidence, count)| CodeDiffCountByConfidence { confidence, count })
            .collect(),
        unresolved_count: aggregate.unresolved_count,
    }
}

fn module_connection_deltas(
    base_edges: &[CodeEdgeRecord],
    head_edges: &[CodeEdgeRecord],
    base_symbols: &BTreeMap<String, CodeSymbolRecord>,
    head_symbols: &BTreeMap<String, CodeSymbolRecord>,
    max_records: usize,
) -> (Vec<CodeDiffModuleConnection>, usize) {
    let base = aggregate_module_connections(base_edges, base_symbols);
    let head = aggregate_module_connections(head_edges, head_symbols);
    let mut keys = BTreeSet::new();
    keys.extend(base.keys().cloned());
    keys.extend(head.keys().cloned());
    let mut dropped = 0;
    let mut deltas = Vec::new();
    for (scope, source, target) in keys {
        let delta = (|| {
            let before = base
                .get(&(scope, source.clone(), target.clone()))
                .map(|aggregate| module_connection_side(aggregate.clone()));
            let after = head
                .get(&(scope, source.clone(), target.clone()))
                .map(|aggregate| module_connection_side(aggregate.clone()));
            if before == after {
                return None;
            }
            let status = match (&before, &after) {
                (None, Some(_)) => CodeDiffEdgeStatus::Added,
                (Some(_), None) => CodeDiffEdgeStatus::Removed,
                (Some(before), Some(after)) if before.confidence_counts != after.confidence_counts => {
                    CodeDiffEdgeStatus::ConfidenceChanged
                }
                (Some(_), Some(_)) => CodeDiffEdgeStatus::Changed,
                (None, None) => unreachable!(),
            };
            Some(CodeDiffModuleConnection {
                connection_key: code_row_id(&[
                    "module-connection",
                    connection_scope_name(scope),
                    &source,
                    &target,
                ]),
                scope,
                source,
                target,
                status,
                before,
                after,
            })
        })();
        let Some(delta) = delta else {
            continue;
        };
        if deltas.len() >= max_records.max(1) {
            dropped += 1;
        } else {
            deltas.push(delta);
        }
    }
    (deltas, dropped)
}

fn query_code_edges_for_revision(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<Vec<CodeEdgeRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_model_ready = code_snapshot_membership_read_model_ready(connection, &config.index_path)?;
    let membership_ready = membership_model_ready
        && match snapshot_status.as_deref() {
            Some("completed") => true,
            None => code_snapshot_membership_rows_available(connection, config, repo_id, revision)?,
            _ => false,
        };
    let revision_rows = code_edge_revisions_read_model_ready(connection, &config.index_path)?
        && code_edge_revision_rows_available(connection, config, repo_id, revision)?;
    let exact_revision_rows = membership_ready && revision_rows;
    let query = if exact_revision_rows {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM (SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty UNION ALL SELECT e.edge_id, e.edge_kind, e.source_symbol_key, e.target_symbol_key, e.target_hint, e.confidence, e.path, e.commit_sha, e.freshness, e.start_line, e.start_col, e.end_line, e.end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = e.repo_id AND m.commit_sha = ? AND m.path = e.path AND m.content_sha256 = e.content_sha256 AND m.parser_version = e.parser_version AND m.query_pack_version = e.query_pack_version AND m.analyzed) AND NOT EXISTS (SELECT 1 FROM code_edge_revisions AS r WHERE r.repo_id = e.repo_id AND r.commit_sha = ? AND r.path = e.path AND r.freshness <> 'staged' AND NOT r.worktree_dirty)) AS edges ORDER BY path, start_line, start_col, edge_id"
    } else if membership_ready {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND (e.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = e.repo_id AND m.commit_sha = ? AND m.path = e.path AND m.content_sha256 = e.content_sha256 AND m.parser_version = e.parser_version AND m.query_pack_version = e.query_pack_version AND m.analyzed)) ORDER BY e.path, e.start_line, e.start_col, e.edge_id"
    } else if revision_rows {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty ORDER BY path, start_line, start_col, edge_id"
    } else {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty ORDER BY path, start_line, start_col, edge_id"
    };
    let mut statement = connection.prepare(query).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let rows = if exact_revision_rows {
        statement
            .query_map(params![repo_id, revision, repo_id, revision, revision], code_edge_from_row)
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
    } else if membership_ready {
        statement
            .query_map(params![repo_id, revision, revision], code_edge_from_row)
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
    } else if revision_rows {
        statement
            .query_map(params![repo_id, revision], code_edge_from_row)
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
    } else {
        statement
            .query_map(params![repo_id, revision], code_edge_from_row)
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
    };
    let mut edges = rows.map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    if edges.is_empty() && membership_ready && revision_rows {
        let mut fallback = connection
            .prepare("SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty ORDER BY path, start_line, start_col, edge_id")
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        edges = fallback
            .query_map(params![repo_id, revision], code_edge_from_row)
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    Ok(edges)
}

fn code_edge_revision_rows_available(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<bool, MemoryError> {
    let mut statement = connection
        .prepare(
            "SELECT 1 FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty LIMIT 1",
        )
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

pub fn code_graph_index_report(
    config: &MemoryConfig,
    repo_id: &str,
) -> Result<CodeIndexReport, CodeGraphProjectionError> {
    let indexed_at = Utc::now();
    let repo = code_graph_repos(config, true)?
        .repos
        .into_iter()
        .find(|repo| repo.repo_id == repo_id);
    let indexed = repo.as_ref().is_some_and(|repo| repo.indexed);
    let status = if indexed {
        CodeIndexStatus::Completed
    } else {
        CodeIndexStatus::Unavailable
    };
    let head_revision = repo.as_ref().and_then(|repo| repo.head_revision.clone());
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
        diagnostics: if indexed {
            Vec::new()
        } else {
            vec!["configured repository has not been indexed".to_string()]
        },
        cursor: code_graph_cursor(repo_id, indexed_at),
        indexed_at,
    })
}

#[derive(Debug, Clone)]
struct CodeSnapshotFile {
    language: String,
    content_sha256: String,
    parser_version: String,
    query_pack_version: String,
    analyzed: bool,
    skip_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct CodeSnapshotMembershipInput {
    path: PathBuf,
    language: String,
    content_sha256: String,
    parser_version: String,
    query_pack_version: String,
    analyzed: bool,
    skip_reason: Option<String>,
}

type CodeSnapshotFiles = BTreeMap<String, CodeSnapshotFile>;

struct CodeSnapshotState<'a> {
    repo_id: &'a str,
    commit_sha: &'a str,
    target_branch: &'a str,
    status: &'a str,
    total_files: usize,
    parsed_files: usize,
    skipped_files: usize,
    deleted_files: usize,
    config_fingerprint: &'a str,
    indexed_at: DateTime<Utc>,
}

pub fn code_index_target(
    config: &MemoryConfig,
) -> Result<Option<(String, String)>, CodeGraphProjectionError> {
    let branch = code_index_branch(&config.repo_root)?;
    let Some(commit) = git_target_commit(&config.repo_root, &branch)? else {
        return Ok(None);
    };
    Ok(Some((branch, commit)))
}

pub fn code_index_repository_is_git(config: &MemoryConfig) -> bool {
    Command::new("git")
        .args(["-C"])
        .arg(&config.repo_root)
        .args(["rev-parse", "--git-dir"])
        .output()
        .is_ok_and(|output| output.status.success())
}

pub fn index_code_repository(
    config: &MemoryConfig,
    repo_id: &str,
) -> Result<CodeIndexReport, CodeGraphProjectionError> {
    if !config.enabled || !config.code_intel.enabled || !config.code_intel.ast.enabled {
        return index_code_repository_at(config, repo_id, None);
    }
    let target = code_index_target(config)?;
    index_code_repository_at(config, repo_id, target)
}

pub fn index_code_repository_at(
    config: &MemoryConfig,
    repo_id: &str,
    target: Option<(String, String)>,
) -> Result<CodeIndexReport, CodeGraphProjectionError> {
    index_code_repository_at_checked(config, repo_id, target, false)
}

pub fn index_code_repository_at_current_target(
    config: &MemoryConfig,
    repo_id: &str,
    target: Option<(String, String)>,
) -> Result<CodeIndexReport, CodeGraphProjectionError> {
    index_code_repository_at_checked(config, repo_id, target, true)
}

fn index_code_repository_at_checked(
    config: &MemoryConfig,
    repo_id: &str,
    target: Option<(String, String)>,
    revalidate_target: bool,
) -> Result<CodeIndexReport, CodeGraphProjectionError> {
    if !config.enabled || !config.code_intel.enabled || !config.code_intel.ast.enabled {
        let indexed_at = Utc::now();
        return Ok(CodeIndexReport {
            schema_version: SchemaVersion::v1(),
            repo_id: repo_id.to_string(),
            status: CodeIndexStatus::Unavailable,
            head_revision: None,
            parsed_files: 0,
            persisted_documents: 0,
            persisted_symbols: 0,
            persisted_edges: 0,
            persisted_diagnostics: 0,
            stale_rows: 0,
            skipped_files: Vec::new(),
            diagnostics: vec![if !config.enabled {
                "memory is disabled in configuration".to_string()
            } else {
                "code intelligence is disabled in configuration".to_string()
            }],
            cursor: code_graph_cursor(repo_id, indexed_at),
            indexed_at,
        });
    }
    let _guard = CODE_INDEX_WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            CodeGraphProjectionError::InvalidRequest(
                "code index writer lock is poisoned".to_string(),
            )
        })?;
    let indexed_at = Utc::now();
    let Some((target_branch, commit_sha)) = target else {
        let mut report = code_graph_index_report(config, repo_id)?;
        report.status = CodeIndexStatus::Unavailable;
        report.head_revision = None;
        report.diagnostics.push(format!(
            "target branch is unavailable for configured repository {}",
            config.repo_root.display()
        ));
        return Ok(report);
    };
    if revalidate_target {
        let current_target = code_index_target(config)?;
        if current_target.as_ref() != Some(&(target_branch.clone(), commit_sha.clone())) {
            let mut report = code_graph_index_report(config, repo_id)?;
            report.status = CodeIndexStatus::Unavailable;
            report.head_revision = None;
            report.diagnostics.push(
                "configured target branch advanced before index promotion; retry the index"
                    .to_string(),
            );
            return Ok(report);
        }
    }

    // Existing memory stores may predate the snapshot tables. Migrate once
    // before the read-only previous-snapshot lookup so a first index works on
    // both empty and previously initialized stores.
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| CodeGraphProjectionError::Memory(
        MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        },
    ))?;
    drop(connection);

    discard_staged_code_index_rows(config, repo_id)?;
    let paths = git_tree_paths(&config.repo_root, &commit_sha)?;
    let repo_prefix = git_repo_prefix(&config.repo_root)?;
    let config_fingerprint = code_index_config_fingerprint(config);
    let previous = latest_code_snapshot(config, repo_id, &target_branch)?;
    let changed_paths = previous
        .as_ref()
        .filter(|(revision, _, _)| revision != &commit_sha)
        .map(|(revision, _, _)| {
            git_changed_tree_paths(&config.repo_root, revision, &commit_sha, &repo_prefix)
        })
        .transpose()?
        .unwrap_or_default();
    let same_revision = previous
        .as_ref()
        .is_some_and(|(revision, _, _)| revision == &commit_sha);
    let reuse_edge_limits = previous
        .as_ref()
        .is_some_and(|(_, _, fingerprint)| fingerprint == &config_fingerprint);

    persist_code_snapshot_state(
        config,
        CodeSnapshotState {
            repo_id,
            commit_sha: &commit_sha,
            target_branch: &target_branch,
            status: "running",
            total_files: 0,
            parsed_files: 0,
            skipped_files: 0,
            deleted_files: 0,
            config_fingerprint: &config_fingerprint,
            indexed_at,
        },
    )?;

    let previous_files = previous
        .map(|(_, files, _)| files)
        .unwrap_or_default();
    let mut current_files = BTreeMap::new();
    let mut memberships = Vec::new();
    let mut skipped_inputs = Vec::new();
    let mut skipped_files = Vec::new();
    let mut diagnostics = Vec::new();
    let mut documents = Vec::new();
    let mut parsed_files = 0;
    let mut remaining_files = config.code_intel.ast.max_files_per_request;
    let mut remaining_matches = config.code_intel.ast.max_matches_per_request;
    let mut stale_rows = 0;
    let mut deleted_files = 0;
    let mut deleted_paths = Vec::new();
    let mut edge_refresh_paths = Vec::new();

    for (path, mode, blob_id) in paths {
        let skipped_directory = path.components().any(|component| {
            skipped_directory_name(Path::new(component.as_os_str())).is_some()
        });
        if mode == "120000" || mode == "160000" || skipped_directory {
            let reason = match mode.as_str() {
                "120000" => "symlink not indexed",
                "160000" => "gitlink not indexed",
                _ => "skipped directory",
            };
            let relative_display = path.to_string_lossy().to_string();
            skipped_files.push(format!("{relative_display}: {reason}"));
            memberships.push(CodeSnapshotMembershipInput {
                path: path.clone(),
                language: "unknown".to_string(),
                content_sha256: format!("git-blob:{blob_id}"),
                parser_version: String::new(),
                query_pack_version: String::new(),
                analyzed: false,
                skip_reason: Some(reason.to_string()),
            });
            current_files.insert(
                path.to_string_lossy().to_string(),
                CodeSnapshotFile {
                    language: "unknown".to_string(),
                    content_sha256: format!("git-blob:{blob_id}"),
                    parser_version: String::new(),
                    query_pack_version: String::new(),
                    analyzed: false,
                    skip_reason: Some(reason.to_string()),
                },
            );
            skipped_inputs.push(CodeIntelSkippedFileInput {
                path,
                reason: reason.to_string(),
                content_sha256: format!("git-blob:{blob_id}"),
            });
            continue;
        }
        let relative_display = path.to_string_lossy().to_string();
        let blob_size = git_blob_size(&config.repo_root, &repo_prefix, &commit_sha, &path)?;
        let Some(language) = detect_language(&path) else {
            let content_sha256 = format!("git-blob:{blob_id}");
            skipped_files.push(format!("{relative_display}: unsupported language"));
            memberships.push(CodeSnapshotMembershipInput {
                path: path.clone(),
                language: "unknown".to_string(),
                content_sha256: content_sha256.clone(),
                parser_version: String::new(),
                query_pack_version: String::new(),
                analyzed: false,
                skip_reason: Some("unsupported language".to_string()),
            });
            current_files.insert(
                relative_display.clone(),
                CodeSnapshotFile {
                    language: "unknown".to_string(),
                    content_sha256: content_sha256.clone(),
                    parser_version: String::new(),
                    query_pack_version: String::new(),
                    analyzed: false,
                    skip_reason: Some("unsupported language".to_string()),
                },
            );
            skipped_inputs.push(CodeIntelSkippedFileInput {
                path,
                reason: "unsupported language".to_string(),
                content_sha256,
            });
            continue;
        };
        let language_id = language.id().to_string();
        if blob_size > config.code_intel.ast.max_file_bytes {
            let reason = format!("max file size {} bytes", config.code_intel.ast.max_file_bytes);
            let content_sha256 = format!("git-blob:{blob_id}");
            skipped_files.push(format!("{relative_display}: {reason}"));
            memberships.push(CodeSnapshotMembershipInput {
                path: path.clone(),
                language: language_id.clone(),
                content_sha256: content_sha256.clone(),
                parser_version: String::new(),
                query_pack_version: String::new(),
                analyzed: false,
                skip_reason: Some(reason.clone()),
            });
            current_files.insert(
                relative_display.clone(),
                CodeSnapshotFile {
                    language: language_id.clone(),
                    content_sha256: content_sha256.clone(),
                    parser_version: String::new(),
                    query_pack_version: String::new(),
                    analyzed: false,
                    skip_reason: Some(reason.clone()),
                },
            );
            skipped_inputs.push(CodeIntelSkippedFileInput {
                path,
                reason,
                content_sha256,
            });
            continue;
        }

        let versions = current_parser_versions(language);
        let current_parser_version = format!("{}:{}", versions.grammar, versions.tree_sitter);
        let previous_file = previous_files.get(&relative_display);
        let path_unchanged = previous_file.is_some_and(|_| {
            !changed_paths.contains(&relative_display)
        });
        let edge_limit_reparse = path_unchanged
            && previous_file.is_some_and(|previous_file| {
                previous_file.analyzed
                    && previous_file.language == language_id
                    && previous_file.parser_version == current_parser_version
                    && previous_file.query_pack_version == versions.query_pack
                    && !reuse_edge_limits
            });
        let reusable_previous = if !same_revision && path_unchanged {
            previous_file
                .filter(|previous_file| {
                    (previous_file.analyzed
                        && reuse_edge_limits
                        && previous_file.language == language_id
                        && previous_file.parser_version == current_parser_version
                        && previous_file.query_pack_version == versions.query_pack)
                        || (!previous_file.analyzed
                            && previous_file.parser_version.is_empty()
                            && previous_file.query_pack_version.is_empty()
                            && previous_file.skip_reason.as_deref() != Some("max files per request")
                            && !previous_file
                                .skip_reason
                                .as_deref()
                                .is_some_and(|reason| reason.starts_with("max file size "))
                            && !matches!(
                                previous_file.skip_reason.as_deref(),
                                Some("unsupported language") | Some("parse failed")
                            ))
                })
                .map(|previous_file| {
                    current_code_snapshot_file_matches(
                        config,
                        repo_id,
                        &CodeSnapshotMembershipInput {
                            path: path.clone(),
                            language: previous_file.language.clone(),
                            content_sha256: previous_file.content_sha256.clone(),
                            parser_version: previous_file.parser_version.clone(),
                            query_pack_version: previous_file.query_pack_version.clone(),
                            analyzed: previous_file.analyzed,
                            skip_reason: previous_file.skip_reason.clone(),
                        },
                    )
                    .map(|matches| (previous_file, matches))
                })
                .transpose()?
                .and_then(|(previous_file, matches)| matches.then_some(previous_file))
        } else {
            None
        };
        if let Some(previous_file) = reusable_previous {
            current_files.insert(relative_display.clone(), previous_file.clone());
            memberships.push(CodeSnapshotMembershipInput {
                path: path.clone(),
                language: previous_file.language.clone(),
                content_sha256: previous_file.content_sha256.clone(),
                parser_version: previous_file.parser_version.clone(),
                query_pack_version: previous_file.query_pack_version.clone(),
                analyzed: previous_file.analyzed,
                skip_reason: previous_file.skip_reason.clone(),
            });
            if !previous_file.analyzed {
                let reason = previous_file
                    .skip_reason
                    .clone()
                    .unwrap_or_else(|| "previously skipped".to_string());
                skipped_files.push(format!("{relative_display}: {reason}"));
                skipped_inputs.push(CodeIntelSkippedFileInput {
                    path,
                    reason,
                    content_sha256: previous_file.content_sha256.clone(),
                });
            }
            continue;
        }
        if remaining_files == 0 {
            let reason = "max files per request".to_string();
            let content_sha256 = format!("git-blob:{blob_id}");
            skipped_files.push(format!("{relative_display}: {reason}"));
            let file = CodeSnapshotFile {
                language: language_id.clone(),
                content_sha256: content_sha256.clone(),
                parser_version: String::new(),
                query_pack_version: String::new(),
                analyzed: false,
                skip_reason: Some(reason.clone()),
            };
            current_files.insert(relative_display, file);
            memberships.push(CodeSnapshotMembershipInput {
                path: path.clone(),
                language: language_id,
                content_sha256: content_sha256.clone(),
                parser_version: String::new(),
                query_pack_version: String::new(),
                analyzed: false,
                skip_reason: Some(reason.clone()),
            });
            skipped_inputs.push(CodeIntelSkippedFileInput {
                path,
                reason,
                content_sha256,
            });
            continue;
        }
        remaining_files -= 1;
        let bytes = git_blob(&config.repo_root, &repo_prefix, &commit_sha, &path)?;
        let content_sha256 = sha256_bytes_hex(&bytes);
        let source = match String::from_utf8(bytes) {
            Ok(source) => source,
            Err(error) => {
                let reason = "invalid UTF-8".to_string();
                skipped_files.push(format!("{relative_display}: {reason}"));
                diagnostics.push(format!("{relative_display}: {error}"));
                current_files.insert(
                    relative_display,
                    CodeSnapshotFile {
                        language: language_id.clone(),
                        content_sha256: content_sha256.clone(),
                        parser_version: String::new(),
                        query_pack_version: String::new(),
                        analyzed: false,
                        skip_reason: Some(reason.clone()),
                    },
                );
                memberships.push(CodeSnapshotMembershipInput {
                    path: path.clone(),
                    language: language_id,
                    content_sha256: content_sha256.clone(),
                    parser_version: String::new(),
                    query_pack_version: String::new(),
                    analyzed: false,
                    skip_reason: Some(reason.clone()),
                });
                skipped_inputs.push(CodeIntelSkippedFileInput {
                    path,
                    reason,
                    content_sha256,
                });
                continue;
            }
        };
        let summary = match parse_path(&path, &source) {
            Ok(summary) => summary,
            Err(error) => {
                let reason = "parse failed".to_string();
                skipped_files.push(format!("{relative_display}: {reason}"));
                diagnostics.push(format!("{relative_display}: {error}"));
                current_files.insert(
                    relative_display,
                    CodeSnapshotFile {
                        language: language_id.clone(),
                        content_sha256: content_sha256.clone(),
                        parser_version: String::new(),
                        query_pack_version: String::new(),
                        analyzed: false,
                        skip_reason: Some(reason.clone()),
                    },
                );
                memberships.push(CodeSnapshotMembershipInput {
                    path: path.clone(),
                    language: language_id,
                    content_sha256: content_sha256.clone(),
                    parser_version: String::new(),
                    query_pack_version: String::new(),
                    analyzed: false,
                    skip_reason: Some(reason.clone()),
                });
                skipped_inputs.push(CodeIntelSkippedFileInput {
                    path,
                    reason,
                    content_sha256,
                });
                continue;
            }
        };
        let parser_version = format!("{}:{}", summary.versions.grammar, summary.versions.tree_sitter);
        let query_pack_version = summary.versions.query_pack.clone();
        let document = code_index_document(
            path.clone(),
            &source,
            &summary,
            &mut remaining_matches,
            config.code_intel.ast.max_capture_bytes,
        );
        diagnostics.extend(summary.diagnostics.iter().map(|diagnostic| {
            format!("{relative_display}: {} at {}", diagnostic.node_kind, diagnostic.rendered_span)
        }));
        current_files.insert(
            relative_display.clone(),
            CodeSnapshotFile {
                language: language_id.clone(),
                content_sha256: content_sha256.clone(),
                parser_version: parser_version.clone(),
                query_pack_version: query_pack_version.clone(),
                analyzed: true,
                skip_reason: None,
            },
        );
        memberships.push(CodeSnapshotMembershipInput {
            path,
            language: language_id,
            content_sha256,
            parser_version,
            query_pack_version,
            analyzed: true,
            skip_reason: None,
        });
        documents.push(document);
        if edge_limit_reparse {
            edge_refresh_paths.push(relative_display.clone());
        }
        parsed_files += 1;
    }

    for path in previous_files.keys() {
        if !current_files.contains_key(path) {
            deleted_files += 1;
            deleted_paths.push(path.clone());
            diagnostics.push(format!("{path}: deleted from target branch"));
        }
    }
    for path in current_code_snapshot_paths(config, repo_id)? {
        if !current_files.contains_key(&path) && !deleted_paths.contains(&path) {
            deleted_files += 1;
            deleted_paths.push(path.clone());
            diagnostics.push(format!("{path}: removed from configured target branch"));
        }
    }

    let persisted = persist_code_index_documents_in_batches(
        config,
        repo_id,
        &commit_sha,
        documents,
        "staged",
        false,
    )?;
    for batch in skipped_inputs.chunks(32) {
        persist_code_intel_skipped_files_with_freshness(
            config,
            repo_id,
            Some(&commit_sha),
            false,
            batch,
            "staged",
            false,
        )?;
    }
    let run_id = indexed_at.to_rfc3339();
    persist_code_snapshot_membership(config, repo_id, &commit_sha, &run_id, &memberships)?;
    let mut promotion_paths = Vec::new();
    for file in &memberships {
        let path = file.path.to_string_lossy().to_string();
        if !edge_refresh_paths.iter().any(|candidate| candidate == &path)
            && !current_code_snapshot_file_matches(config, repo_id, file)?
        {
            promotion_paths.push(path);
        }
    }
    for path in deleted_paths {
        promotion_paths.push(path);
    }
    let completed_state = CodeSnapshotState {
        repo_id,
        commit_sha: &commit_sha,
        target_branch: &target_branch,
        status: "completed",
        total_files: memberships.len(),
        parsed_files,
        skipped_files: skipped_files.len(),
        deleted_files,
        config_fingerprint: &config_fingerprint,
        indexed_at,
    };
    if revalidate_target {
        let current_target = code_index_target(config)?;
        if current_target.as_ref() != Some(&(target_branch.clone(), commit_sha.clone())) {
            discard_staged_code_index_rows(config, repo_id)?;
            let mut report = code_graph_index_report(config, repo_id)?;
            report.status = CodeIndexStatus::Unavailable;
            report.head_revision = None;
            report.diagnostics.push(
                "configured target branch advanced before index promotion; retry the index"
                    .to_string(),
            );
            return Ok(report);
        }
    }
    stale_rows += promote_staged_code_snapshot(
        config,
        repo_id,
        &commit_sha,
        &promotion_paths,
        &edge_refresh_paths,
        completed_state,
    )?;
    let analyzed_documents = memberships.iter().filter(|file| file.analyzed).count();
    let current_counts = code_index_counts(config, repo_id)?;
    stale_rows += persisted.stale_rows;
    Ok(CodeIndexReport {
        schema_version: SchemaVersion::v1(),
        repo_id: repo_id.to_string(),
        status: CodeIndexStatus::Completed,
        head_revision: Some(commit_sha),
        parsed_files,
        persisted_documents: analyzed_documents.max(current_counts.documents),
        persisted_symbols: current_counts.symbols.max(persisted.persisted_symbols),
        persisted_edges: current_counts.edges.max(persisted.persisted_edges),
        persisted_diagnostics: current_counts.diagnostics.max(persisted.persisted_diagnostics),
        stale_rows,
        skipped_files,
        diagnostics,
        cursor: code_graph_cursor(repo_id, indexed_at),
        indexed_at,
    })
}

pub fn code_index_branch(root: &Path) -> Result<String, CodeGraphProjectionError> {
    let workflow = root.join("WORKFLOW.md");
    if !workflow.is_file() {
        return Ok(DEFAULT_CODE_INDEX_BRANCH.to_string());
    }
    let contents = fs::read_to_string(&workflow).map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::ReadFile {
            path: workflow.clone(),
            source,
        })
    })?;
    let branch = contents
        .lines()
        .find_map(|line| line.trim().strip_prefix("Target branch:"))
        .map(str::trim)
        .map(|value| value.trim_matches('`').trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_CODE_INDEX_BRANCH.to_string());
    if branch.starts_with('-')
        || branch.starts_with("origin/")
        || branch.starts_with("refs/")
        || branch.starts_with("@{-")
        || branch.contains("..")
        || branch.contains("@{")
        || branch.contains('`')
        || branch.split('/').any(|part| part.is_empty())
    {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "configured target branch is invalid".to_string(),
        ));
    }
    let branch_check = Command::new("git")
        .args(["check-ref-format", "--branch", &branch])
        .output()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: root.to_path_buf(),
            source,
        }))?;
    if !branch_check.status.success() {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "configured target branch is invalid".to_string(),
        ));
    }
    Ok(branch)
}

fn git_target_commit(
    root: &Path,
    branch: &str,
) -> Result<Option<String>, CodeGraphProjectionError> {
    for reference in [
        format!("refs/remotes/origin/{branch}"),
        format!("refs/heads/{branch}"),
    ] {
        let output = Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(&reference)
            .output()
            .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
                path: root.to_path_buf(),
                source,
            }))?;
        if output.status.success() {
            return Ok(Some(String::from_utf8_lossy(&output.stdout).trim().to_string()));
        }
    }
    Ok(None)
}

fn git_tree_paths(
    root: &Path,
    commit: &str,
) -> Result<Vec<(PathBuf, String, String)>, CodeGraphProjectionError> {
    let mut directories = vec![String::new()];
    let mut entries = Vec::new();
    while let Some(directory) = directories.pop() {
        let pathspec = if directory.is_empty() {
            ".".to_string()
        } else {
            format!("{directory}/")
        };
        let output = Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(["ls-tree", "-z"])
            .arg(commit)
            .args(["--", &pathspec])
            .output()
            .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
                path: root.to_path_buf(),
                source,
            }))?;
        if !output.status.success() {
            return Err(CodeGraphProjectionError::InvalidRequest(
                "failed to list configured target branch files".to_string(),
            ));
        }
        for record in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let record = String::from_utf8_lossy(record);
            let Some((metadata, path)) = record.split_once('\t') else {
                continue;
            };
            let mut parts = metadata.split_whitespace();
            let mode = parts.next().unwrap_or_default().to_string();
            let object_kind = parts.next().unwrap_or_default();
            let object_id = parts.next().unwrap_or_default().to_string();
            if object_kind == "tree" {
                if skipped_directory_name(Path::new(path)).is_none() {
                    directories.push(path.to_string());
                } else {
                    entries.push((PathBuf::from(path), mode, object_id));
                }
            } else if object_kind == "blob" || object_kind == "commit" {
                entries.push((PathBuf::from(path), mode, object_id));
            }
        }
    }
    for (path, _, _) in &entries {
        if normalize_code_path(&path.to_string_lossy()).is_err() {
            return Err(CodeGraphProjectionError::InvalidRequest(format!(
                "target branch contains an invalid path: {}",
                path.display()
            )));
        }
    }
    Ok(entries)
}

fn git_repo_prefix(root: &Path) -> Result<PathBuf, CodeGraphProjectionError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "--show-prefix"])
        .output()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: root.to_path_buf(),
            source,
        }))?;
    if !output.status.success() {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "failed to resolve configured repository path".to_string(),
        ));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

fn git_changed_tree_paths(
    root: &Path,
    base: &str,
    head: &str,
    repo_prefix: &Path,
) -> Result<BTreeSet<String>, CodeGraphProjectionError> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["diff", "--name-only", "-z", base, head, "--", "."])
        .output()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: root.to_path_buf(),
            source,
        }))?;
    if !output.status.success() {
        return Err(CodeGraphProjectionError::InvalidRequest(
            "failed to compare target branch revisions".to_string(),
        ));
    }
    let prefix = repo_prefix.to_string_lossy().trim_end_matches('/').to_string();
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).to_string())
        .map(|path| {
            path.strip_prefix(&format!("{prefix}/"))
                .unwrap_or(&path)
                .to_string()
        })
        .collect())
}

fn git_blob(
    root: &Path,
    repo_prefix: &Path,
    commit: &str,
    path: &Path,
) -> Result<Vec<u8>, CodeGraphProjectionError> {
    let object = format!("{}:{}", commit, repo_prefix.join(path).to_string_lossy());
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["cat-file", "blob"])
        .arg(object)
        .output()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: root.to_path_buf(),
            source,
        }))?;
    if !output.status.success() {
        return Err(CodeGraphProjectionError::InvalidRequest(format!(
            "failed to read target-branch file {}",
            path.display()
        )));
    }
    Ok(output.stdout)
}

fn git_blob_size(
    root: &Path,
    repo_prefix: &Path,
    commit: &str,
    path: &Path,
) -> Result<u64, CodeGraphProjectionError> {
    let object = format!("{}:{}", commit, repo_prefix.join(path).to_string_lossy());
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["cat-file", "-s"])
        .arg(object)
        .output()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::ResolvePath {
            path: root.to_path_buf(),
            source,
        }))?;
    if !output.status.success() {
        return Err(CodeGraphProjectionError::InvalidRequest(format!(
            "failed to inspect target-branch file {}",
            path.display()
        )));
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
        .map_err(|_| {
            CodeGraphProjectionError::InvalidRequest(format!(
                "invalid target-branch blob size for {}",
                path.display()
            ))
        })
}

fn latest_code_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    target_branch: &str,
) -> Result<Option<(String, CodeSnapshotFiles, String)>, CodeGraphProjectionError> {
    if !config.index_path.exists() {
        return Ok(None);
    }
    let connection = open_index_read_only(config)?;
    let mut statement = connection
        .prepare("SELECT commit_sha, config_fingerprint FROM code_index_snapshots WHERE repo_id = ? AND target_branch = ? AND status = 'completed' ORDER BY indexed_at DESC LIMIT 1")
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    let mut rows = statement
        .query(params![repo_id, target_branch])
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    let Some(row) = rows.next().map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })? else {
        return Ok(None);
    };
    let revision = row.get::<_, String>(0).map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })?;
    let config_fingerprint = row.get::<_, String>(1).map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })?;
    let mut statement = connection
        .prepare("SELECT path, language, content_sha256, parser_version, query_pack_version, analyzed, skip_reason FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? ORDER BY path")
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    let files = statement
        .query_map(params![repo_id, &revision], |row| {
            Ok((
                row.get::<_, String>(0)?,
                CodeSnapshotFile {
                    language: row.get(1)?,
                    content_sha256: row.get(2)?,
                    parser_version: row.get(3)?,
                    query_pack_version: row.get(4)?,
                    analyzed: row.get(5)?,
                    skip_reason: row.get(6)?,
                },
            ))
        })
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    Ok(Some((revision, files, config_fingerprint)))
}

fn code_index_config_fingerprint(config: &MemoryConfig) -> String {
    format!(
        "edge-limits:{}:{}",
        config.code_intel.ast.max_matches_per_request,
        config.code_intel.ast.max_capture_bytes
    )
}

fn persist_code_snapshot_state(
    config: &MemoryConfig,
    state: CodeSnapshotState<'_>,
) -> Result<(), CodeGraphProjectionError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| CodeGraphProjectionError::Memory(
        MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        },
    ))?;
    if state.status == "running"
        && connection
            .query_row(
                "SELECT status FROM code_index_snapshots WHERE repo_id = ? AND commit_sha = ?",
                params![state.repo_id, state.commit_sha],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            }))?
            .as_deref()
            == Some("completed")
    {
        return Ok(());
    }
    connection
        .execute(
            "INSERT OR REPLACE INTO code_index_snapshots (repo_id, commit_sha, target_branch, status, total_files, parsed_files, skipped_files, deleted_files, config_fingerprint, indexed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                state.repo_id,
                state.commit_sha,
                state.target_branch,
                state.status,
                state.total_files as i64,
                state.parsed_files as i64,
                state.skipped_files as i64,
                state.deleted_files as i64,
                state.config_fingerprint,
                state.indexed_at.to_rfc3339(),
            ],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    Ok(())
}

fn discard_staged_code_index_rows(
    config: &MemoryConfig,
    repo_id: &str,
) -> Result<(), CodeGraphProjectionError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| CodeGraphProjectionError::Memory(
        MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        },
    ))?;
    let transaction = connection.transaction().map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })?;
    for table in [
        "code_documents",
        "code_document_revisions",
        "code_symbols",
        "code_edges",
        "code_edge_revisions",
        "code_diagnostics",
        "code_diagnostic_revisions",
        "code_skipped_files",
    ] {
        transaction
            .execute(
                &format!("DELETE FROM {table} WHERE repo_id = ? AND freshness = 'staged'"),
                params![repo_id],
            )
            .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            }))?;
    }
    transaction
        .execute(
            "DELETE FROM code_skipped_files_staging WHERE repo_id = ?",
            params![repo_id],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "DELETE FROM code_documents_staging WHERE repo_id = ?",
            params![repo_id],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "DELETE FROM code_snapshot_membership_staging WHERE repo_id = ?",
            params![repo_id],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction.commit().map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })?;
    Ok(())
}

fn persist_code_snapshot_membership(
    config: &MemoryConfig,
    repo_id: &str,
    commit_sha: &str,
    run_id: &str,
    files: &[CodeSnapshotMembershipInput],
) -> Result<(), CodeGraphProjectionError> {
    for batch in files.chunks(256) {
        let mut connection = open_index(config)?;
        migrate_index(&connection).map_err(|source| CodeGraphProjectionError::Memory(
            MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            },
        ))?;
        let transaction = connection.transaction().map_err(|source| {
            CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })
        })?;
        for file in batch {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_snapshot_membership_staging (run_id, repo_id, commit_sha, path, language, content_sha256, parser_version, query_pack_version, analyzed, skip_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        run_id,
                        repo_id,
                        commit_sha,
                        file.path.to_string_lossy().to_string(),
                        file.language,
                        file.content_sha256,
                        file.parser_version,
                        file.query_pack_version,
                        if file.analyzed { 1_i64 } else { 0_i64 },
                        file.skip_reason,
                    ],
                )
                .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                }))?;
        }
        transaction.commit().map_err(|source| {
            CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })
        })?;
    }
    Ok(())
}

fn promote_staged_code_snapshot(
    config: &MemoryConfig,
    repo_id: &str,
    commit_sha: &str,
    changed_paths: &[String],
    edge_refresh_paths: &[String],
    completed_state: CodeSnapshotState<'_>,
) -> Result<usize, CodeGraphProjectionError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| CodeGraphProjectionError::Memory(
        MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        },
    ))?;
    let transaction = connection.transaction().map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })?;
    let mut stale_rows = 0;
    transaction
        .execute(
            "UPDATE code_edges AS current SET commit_sha = staged.commit_sha, worktree_dirty = staged.worktree_dirty, path = staged.path, language = staged.language, edge_kind = staged.edge_kind, source_symbol_id = staged.source_symbol_id, source_symbol_key = staged.source_symbol_key, target_symbol_id = staged.target_symbol_id, target_symbol_key = staged.target_symbol_key, target_hint = staged.target_hint, confidence = staged.confidence, start_line = staged.start_line, start_col = staged.start_col, end_line = staged.end_line, end_col = staged.end_col, start_byte = staged.start_byte, end_byte = staged.end_byte, content_sha256 = staged.content_sha256, parser_version = staged.parser_version, query_pack_version = staged.query_pack_version, indexed_at = staged.indexed_at FROM code_edge_revisions AS staged WHERE current.edge_id = staged.edge_id AND current.repo_id = staged.repo_id AND current.freshness = 'current' AND staged.repo_id = ? AND staged.commit_sha = ? AND staged.freshness = 'staged'",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    stale_rows += transaction
        .execute(
            "UPDATE code_symbols AS current SET freshness = 'stale' WHERE current.repo_id = ? AND current.freshness = 'current' AND EXISTS (SELECT 1 FROM code_symbols AS staged WHERE staged.repo_id = current.repo_id AND staged.path = current.path AND staged.symbol_key = current.symbol_key AND staged.commit_sha = ? AND staged.freshness = 'staged')",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    for path in changed_paths {
        for table in [
            "code_documents",
            "code_symbols",
            "code_edges",
            "code_diagnostics",
            "code_skipped_files",
        ] {
            stale_rows += transaction
                .execute(
                    &format!(
                        "UPDATE {table} SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current'"
                    ),
                    params![repo_id, path],
                )
                .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                }))?;
        }
    }
    for path in edge_refresh_paths {
        stale_rows += transaction
            .execute(
                "UPDATE code_edges AS current SET freshness = 'stale' WHERE current.repo_id = ? AND current.path = ? AND current.freshness = 'current' AND NOT EXISTS (SELECT 1 FROM code_edge_revisions AS staged WHERE staged.edge_id = current.edge_id AND staged.repo_id = ? AND staged.commit_sha = ? AND staged.freshness = 'staged')",
                params![repo_id, path, repo_id, commit_sha],
            )
            .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            }))?;
    }
    stale_rows += transaction
        .execute(
            "UPDATE code_diagnostics AS current SET freshness = 'stale' WHERE current.repo_id = ? AND current.freshness = 'current' AND EXISTS (SELECT 1 FROM code_diagnostics AS staged WHERE staged.repo_id = current.repo_id AND staged.path = current.path AND staged.commit_sha = ? AND staged.freshness = 'staged')",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO code_documents (repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_id, parser_version, query_pack_version, byte_len, line_count, indexed_at, freshness) SELECT repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_id, parser_version, query_pack_version, byte_len, line_count, indexed_at, 'current' FROM code_documents_staging WHERE repo_id = ? AND commit_sha = ?",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "DELETE FROM code_documents_staging WHERE repo_id = ? AND commit_sha = ?",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO code_skipped_files (repo_id, commit_sha, worktree_dirty, path, reason, content_sha256, indexed_at, freshness) SELECT repo_id, commit_sha, false, path, reason, content_sha256, indexed_at, 'current' FROM code_skipped_files_staging WHERE repo_id = ? AND commit_sha = ?",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "DELETE FROM code_skipped_files_staging WHERE repo_id = ? AND commit_sha = ?",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    for table in [
        "code_documents",
        "code_document_revisions",
        "code_symbols",
        "code_edges",
        "code_edge_revisions",
        "code_diagnostics",
        "code_diagnostic_revisions",
        "code_skipped_files",
    ] {
        transaction
            .execute(
                &format!(
                    "UPDATE {table} SET freshness = 'current' WHERE repo_id = ? AND commit_sha = ? AND freshness = 'staged'"
                ),
                params![repo_id, commit_sha],
            )
            .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            }))?;
    }
    let run_id = completed_state.indexed_at.to_rfc3339();
    transaction
        .execute(
            "DELETE FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ?",
            params![repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "INSERT INTO code_snapshot_membership (repo_id, commit_sha, path, language, content_sha256, parser_version, query_pack_version, analyzed, skip_reason) SELECT repo_id, commit_sha, path, language, content_sha256, parser_version, query_pack_version, analyzed, skip_reason FROM code_snapshot_membership_staging WHERE run_id = ? AND repo_id = ? AND commit_sha = ?",
            params![&run_id, repo_id, commit_sha],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "DELETE FROM code_snapshot_membership_staging WHERE run_id = ?",
            params![&run_id],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction
        .execute(
            "UPDATE code_index_snapshots SET target_branch = ?, status = ?, total_files = ?, parsed_files = ?, skipped_files = ?, deleted_files = ?, config_fingerprint = ?, indexed_at = ? WHERE repo_id = ? AND commit_sha = ?",
            params![
                completed_state.target_branch,
                completed_state.status,
                completed_state.total_files as i64,
                completed_state.parsed_files as i64,
                completed_state.skipped_files as i64,
                completed_state.deleted_files as i64,
                completed_state.config_fingerprint,
                completed_state.indexed_at.to_rfc3339(),
                completed_state.repo_id,
                completed_state.commit_sha,
            ],
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    transaction.commit().map_err(|source| {
        CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
    })?;
    Ok(stale_rows)
}

fn current_code_snapshot_paths(
    config: &MemoryConfig,
    repo_id: &str,
) -> Result<BTreeSet<String>, CodeGraphProjectionError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(BTreeSet::new());
    };
    let mut statement = connection
        .prepare(
            "SELECT path FROM code_documents WHERE repo_id = ? AND freshness = 'current' UNION SELECT path FROM code_skipped_files WHERE repo_id = ? AND freshness = 'current'",
        )
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?;
    statement
        .query_map(params![repo_id, repo_id], |row| row.get::<_, String>(0))
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|source| CodeGraphProjectionError::Memory(MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }))
}

fn current_code_snapshot_file_matches(
    config: &MemoryConfig,
    repo_id: &str,
    file: &CodeSnapshotMembershipInput,
) -> Result<bool, CodeGraphProjectionError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(false);
    };
    let path = file.path.to_string_lossy().to_string();
    let exists = if file.analyzed {
        connection
            .query_row(
                "SELECT 1 FROM code_documents WHERE repo_id = ? AND path = ? AND content_sha256 = ? AND parser_version = ? AND query_pack_version = ? AND freshness = 'current' AND NOT worktree_dirty LIMIT 1",
                params![
                    repo_id,
                    path,
                    file.content_sha256,
                    file.parser_version,
                    file.query_pack_version,
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .is_some()
    } else {
        connection
            .query_row(
                "SELECT 1 FROM code_skipped_files WHERE repo_id = ? AND path = ? AND reason = ? AND content_sha256 = ? AND freshness = 'current' AND NOT worktree_dirty LIMIT 1",
                params![
                    repo_id,
                    path,
                    file.skip_reason.as_deref().unwrap_or(""),
                    file.content_sha256,
                ],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .is_some()
    };
    Ok(exists)
}

fn persist_code_index_documents_in_batches(
    config: &MemoryConfig,
    repo_id: &str,
    commit_sha: &str,
    documents: Vec<CodeIntelDocumentInput>,
    freshness: &str,
    stale_existing: bool,
) -> Result<CodeIntelPersistReport, CodeGraphProjectionError> {
    let mut total = CodeIntelPersistReport {
        parsed_files: 0,
        persisted_documents: 0,
        persisted_symbols: 0,
        persisted_edges: 0,
        persisted_diagnostics: 0,
        stale_rows: 0,
        skipped_files: Vec::new(),
        diagnostics: Vec::new(),
    };
    for batch in documents.chunks(32) {
        let report = persist_code_intel_documents_with_freshness(
            config,
            CodeIntelPersistBatch {
                repo_id: repo_id.to_string(),
                commit_sha: Some(commit_sha.to_string()),
                worktree_dirty: false,
                documents: batch.to_vec(),
            },
            freshness,
            stale_existing,
        )?;
        total.parsed_files += report.parsed_files;
        total.persisted_documents += report.persisted_documents;
        total.persisted_symbols += report.persisted_symbols;
        total.persisted_edges += report.persisted_edges;
        total.persisted_diagnostics += report.persisted_diagnostics;
        total.stale_rows += report.stale_rows;
    }
    Ok(total)
}

fn code_index_document(
    path: PathBuf,
    source: &str,
    summary: &crate::opensymphony_code_intel::ParsedDocumentSummary,
    remaining_matches: &mut usize,
    max_capture_bytes: usize,
) -> CodeIntelDocumentInput {
    let parser_version = format!("{}:{}", summary.versions.grammar, summary.versions.tree_sitter);
    let symbols = summary
        .symbols
        .iter()
        .map(|symbol| {
            let snippet = source
                .get(symbol.span.start_byte..symbol.span.end_byte)
                .unwrap_or(symbol.name.as_str());
            CodeIntelSymbolInput {
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
                snippet_sha256: sha256_hex(snippet),
            }
        })
        .collect();
    let edges = summary
        .captures
        .iter()
        .filter_map(|capture| code_index_edge_input(capture, max_capture_bytes))
        .take(*remaining_matches)
        .collect::<Vec<_>>();
    *remaining_matches = remaining_matches.saturating_sub(edges.len());
    CodeIntelDocumentInput {
        path,
        language: summary.source.language.id().to_string(),
        content_sha256: summary.source.sha256.clone(),
        parser_id: summary.versions.provider.clone(),
        parser_version,
        query_pack_version: summary.versions.query_pack.clone(),
        byte_len: summary.source.bytes,
        line_count: source.lines().count(),
        symbols,
        edges,
        diagnostics: summary
            .diagnostics
            .iter()
            .map(|diagnostic| {
                let (kind, severity) = match diagnostic.kind {
                    AstDiagnosticKind::Error => ("error", "error"),
                    AstDiagnosticKind::Missing => ("missing", "warning"),
                };
                CodeIntelDiagnosticInput {
                    kind: kind.to_string(),
                    severity: severity.to_string(),
                    message: format!("{} parse diagnostic", diagnostic.node_kind),
                    start_line: diagnostic.span.start_line,
                    start_col: diagnostic.span.start_column,
                    end_line: diagnostic.span.end_line,
                    end_col: diagnostic.span.end_column,
                    start_byte: diagnostic.span.start_byte,
                    end_byte: diagnostic.span.end_byte,
                }
            })
            .collect(),
    }
}

fn code_index_edge_input(
    capture: &CaptureRecord,
    max_capture_bytes: usize,
) -> Option<CodeIntelEdgeInput> {
    if !matches!(
        capture.capture_name.split('.').next(),
        Some("reference" | "import" | "export" | "test")
    ) {
        return None;
    }
    let end = capture
        .text
        .char_indices()
        .map(|(index, character)| (index, index + character.len_utf8()))
        .take_while(|(_, end)| *end <= max_capture_bytes)
        .map(|(_, end)| end)
        .last()
        .unwrap_or(0);
    Some(CodeIntelEdgeInput {
        edge_kind: capture.capture_name.clone(),
        target_hint: Some(capture.text[..end].to_string()),
        confidence: format!("query_pack:{}", capture.query_name),
        start_line: capture.span.start_line,
        start_col: capture.span.start_column,
        end_line: capture.span.end_line,
        end_col: capture.span.end_column,
        start_byte: capture.span.start_byte,
        end_byte: capture.span.end_byte,
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
        topology_delta_available: true,
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
            indexed: self.indexed_at.is_some() || self.head_revision.is_some(),
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
    if !config.repository_sources.is_empty()
        && !config.repository_sources.contains_key(repo_id)
    {
        return Err(CodeGraphProjectionError::RepoNotFound(repo_id.to_string()));
    }
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

fn query_revision_file_symbols(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    path: &str,
) -> Result<Vec<CodeSymbolRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN s.worktree_dirty THEN 1 ELSE 0 END FROM code_symbols AS s WHERE s.repo_id = ? AND s.path = ? AND s.symbol_key != '' AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND (s.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = s.repo_id AND m.commit_sha = ? AND m.path = s.path AND m.content_sha256 = s.content_sha256 AND m.parser_version = s.parser_version AND m.query_pack_version = s.query_pack_version AND m.analyzed)) ORDER BY s.symbol_key, s.indexed_at DESC, s.symbol_id"
        )
    } else {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN worktree_dirty THEN 1 ELSE 0 END FROM code_symbols WHERE repo_id = ? AND path = ? AND commit_sha = ? AND symbol_key != '' AND freshness <> 'staged' ORDER BY symbol_key, indexed_at DESC, symbol_id"
        )
    };
    let mut statement = connection
        .prepare(&query)
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    let rows = if membership_ready {
        statement
            .query_map(params![repo_id, path, revision, revision], |row| {
                let dirty = row.get::<_, i64>(25)?;
                if dirty != 0 {
                    Ok(None)
                } else {
                    code_symbol_from_row(row).map(Some)
                }
            })
            .map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
    } else {
        statement
            .query_map(params![repo_id, path, revision], |row| {
                let dirty = row.get::<_, i64>(25)?;
                if dirty != 0 {
                    Ok(None)
                } else {
                    code_symbol_from_row(row).map(Some)
                }
            })
            .map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
    }
    .map_err(|source| MemoryError::DuckDb {
        path: PathBuf::from("<memory-index>"),
        source,
    })?;
    let mut symbols = BTreeMap::new();
    for row in rows {
        let Some(mut symbol) = row else {
            continue;
        };
        if symbol.commit_sha.as_deref() != Some(revision) {
            symbol.commit_sha = Some(revision.to_string());
        }
        if !symbols.contains_key(&symbol.symbol_key) {
            fill_container_chain(connection, &mut symbol)?;
            symbols.insert(symbol.symbol_key.clone(), symbol);
        }
    }
    Ok(symbols.into_values().collect())
}

fn query_revision_symbol_by_key(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    symbol_key: &str,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN s.worktree_dirty THEN 1 ELSE 0 END FROM code_symbols AS s WHERE s.repo_id = ? AND s.symbol_key = ? AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND (s.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = s.repo_id AND m.commit_sha = ? AND m.path = s.path AND m.content_sha256 = s.content_sha256 AND m.parser_version = s.parser_version AND m.query_pack_version = s.query_pack_version AND m.analyzed)) ORDER BY s.indexed_at DESC, s.symbol_id"
        )
    } else {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN worktree_dirty THEN 1 ELSE 0 END FROM code_symbols WHERE repo_id = ? AND symbol_key = ? AND commit_sha = ? AND freshness <> 'staged' ORDER BY indexed_at DESC, symbol_id"
        )
    };
    let mut statement = connection
        .prepare(&query)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = if membership_ready {
        statement
            .query(params![repo_id, symbol_key, revision, revision])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
    } else {
        statement
            .query(params![repo_id, symbol_key, revision])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
    };
    let Some(row) = rows.next().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?
    else {
        return Ok(None);
    };
    if row
        .get::<_, i64>(25)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        != 0
    {
        return Ok(None);
    }
    let mut symbol = code_symbol_from_row(row).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    if symbol.commit_sha.as_deref() != Some(revision) {
        symbol.commit_sha = Some(revision.to_string());
    }
    fill_container_chain(connection, &mut symbol)?;
    Ok(Some(symbol))
}

fn query_revision_symbol_by_name(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    name: &str,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN s.worktree_dirty THEN 1 ELSE 0 END FROM code_symbols AS s WHERE s.repo_id = ? AND s.name = ? AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND (s.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = s.repo_id AND m.commit_sha = ? AND m.path = s.path AND m.content_sha256 = s.content_sha256 AND m.parser_version = s.parser_version AND m.query_pack_version = s.query_pack_version AND m.analyzed)) ORDER BY s.path, s.start_line, s.start_col, s.symbol_key"
        )
    } else {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN worktree_dirty THEN 1 ELSE 0 END FROM code_symbols WHERE repo_id = ? AND name = ? AND commit_sha = ? AND freshness <> 'staged' ORDER BY path, start_line, start_col, symbol_key"
        )
    };
    let mut statement = connection
        .prepare(&query)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = if membership_ready {
        statement
            .query(params![repo_id, name, revision, revision])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
    } else {
        statement
            .query(params![repo_id, name, revision])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
    };
    let mut candidate = None;
    while let Some(row) = rows.next().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })? {
        if row
            .get::<_, i64>(25)
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            != 0
        {
            continue;
        }
        let mut symbol = code_symbol_from_row(row).map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
        if symbol.commit_sha.as_deref() != Some(revision) {
            symbol.commit_sha = Some(revision.to_string());
        }
        if candidate
            .as_ref()
            .is_some_and(|previous: &CodeSymbolRecord| previous.symbol_key != symbol.symbol_key)
        {
            return Ok(None);
        }
        candidate = Some(symbol);
    }
    if let Some(symbol) = candidate.as_mut() {
        fill_container_chain(connection, symbol)?;
    }
    Ok(candidate)
}

fn query_revision_file_edges(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    path: &str,
) -> Result<Vec<CodeEdgeRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.path = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND (e.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = e.repo_id AND m.commit_sha = ? AND m.path = e.path AND m.content_sha256 = e.content_sha256 AND m.parser_version = e.parser_version AND m.query_pack_version = e.query_pack_version AND m.analyzed)) ORDER BY e.start_line, e.start_col, e.edge_id"
    } else {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.path = ? AND e.commit_sha = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty ORDER BY e.start_line, e.start_col, e.edge_id"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = if membership_ready {
        statement
            .query_map(params![repo_id, path, revision, revision], code_edge_from_row)
    } else {
        statement.query_map(params![repo_id, path, revision], code_edge_from_row)
    }
    .map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })
}

fn query_revision_edges_for_symbol(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
    symbol_key: &str,
) -> Result<Vec<CodeEdgeRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND (e.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = e.repo_id AND m.commit_sha = ? AND m.path = e.path AND m.content_sha256 = e.content_sha256 AND m.parser_version = e.parser_version AND m.query_pack_version = e.query_pack_version AND m.analyzed)) AND (e.source_symbol_key = ? OR e.target_symbol_key = ?) ORDER BY e.path, e.start_line, e.start_col, e.edge_id"
    } else {
        "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges AS e WHERE e.repo_id = ? AND e.commit_sha = ? AND e.freshness <> 'staged' AND NOT e.worktree_dirty AND (e.source_symbol_key = ? OR e.target_symbol_key = ?) ORDER BY e.path, e.start_line, e.start_col, e.edge_id"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = if membership_ready {
        statement.query_map(
            params![repo_id, revision, revision, symbol_key, symbol_key],
            code_edge_from_row,
        )
    } else {
        statement.query_map(params![repo_id, revision, symbol_key, symbol_key], code_edge_from_row)
    }
    .map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })
}

fn query_revision_file_exists(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
    path: &str,
) -> Result<bool, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        "SELECT 1 FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? AND path = ? LIMIT 1"
    } else {
        "SELECT 1 FROM code_documents WHERE repo_id = ? AND commit_sha = ? AND path = ? AND freshness <> 'staged' AND NOT worktree_dirty LIMIT 1"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = statement
        .query(params![repo_id, revision, path])
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
    if symbol.commit_sha.is_some()
        && code_diagnostic_revisions_read_model_ready(connection, &config.index_path)?
    {
        let freshness = code_freshness_filter(include_stale);
        let mut statement = connection
            .prepare(&format!(
                "SELECT DISTINCT kind, severity, message, start_line, start_col, end_line, end_col FROM code_diagnostic_revisions WHERE repo_id = ? AND path = ? AND {freshness} AND content_sha256 = ? AND parser_version = ? AND query_pack_version = ? AND start_line <= ? AND end_line >= ? ORDER BY start_line, start_col, message"
            ))
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        let rows = statement
            .query_map(
                params![
                    &symbol.repo_id,
                    &symbol.path,
                    &symbol.content_sha256,
                    &symbol.parser_version,
                    &symbol.query_pack_version,
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
            })?;
        return Ok(rows);
    }
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
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    if snapshot_status.as_deref() == Some("completed") {
        return Ok(true);
    }
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
    if matches!(snapshot_status.as_deref(), None | Some("completed"))
        && code_snapshot_membership_read_model_ready(connection, &config.index_path)?
    {
        let mut statement = connection
            .prepare("SELECT 1 FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? LIMIT 1")
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
        if rows
            .next()
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
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
            "SELECT 1 FROM {table} WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty LIMIT 1"
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

fn workspace_symbol_node(symbol: &CodeSymbolRecord) -> CodeGraphNode {
    CodeGraphNode {
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
        freshness: CodeGraphFreshness::Current,
        diagnostic_count: 0,
        diagnostic_severity: None,
        metrics: CodeGraphNodeMetrics::default(),
    }
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
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    if matches!(snapshot_status.as_deref(), None | Some("completed"))
        && code_snapshot_membership_read_model_ready(connection, &config.index_path)?
    {
        let mut statement = connection
            .prepare("SELECT path, content_sha256, parser_version, query_pack_version, analyzed FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? ORDER BY path")
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
                        parser_version: row.get(2)?,
                        query_pack_version: row.get(3)?,
                        analyzed: row.get(4)?,
                    },
                ))
            })
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        if snapshot_status.is_some() {
            return Ok(rows);
        }
    }
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
            "SELECT path, content_sha256, parser_version, query_pack_version FROM {table} WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty ORDER BY path, CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC"
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
            "SELECT path, content_sha256 FROM code_skipped_files WHERE repo_id = ? AND commit_sha = ? AND freshness <> 'staged' AND NOT worktree_dirty ORDER BY path, CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC",
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
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?;
    let query = if membership_ready {
        "SELECT 1 FROM code_symbols AS s WHERE s.repo_id = ? AND s.path = ? AND s.content_sha256 = ? AND s.parser_version = ? AND s.query_pack_version = ? AND s.symbol_key != '' AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND (s.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = s.repo_id AND m.commit_sha = ? AND m.path = s.path AND m.content_sha256 = s.content_sha256 AND m.parser_version = s.parser_version AND m.query_pack_version = s.query_pack_version)) LIMIT 1"
    } else {
        "SELECT 1 FROM code_symbols AS s WHERE s.repo_id = ? AND s.path = ? AND s.content_sha256 = ? AND s.parser_version = ? AND s.query_pack_version = ? AND s.symbol_key != '' AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND s.commit_sha = ? LIMIT 1"
    };
    let mut statement = connection
        .prepare(query)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut rows = if membership_ready {
        statement.query(params![
            repo_id,
            path,
            document.content_sha256,
            document.parser_version,
            document.query_pack_version,
            revision,
            revision,
        ])
    } else {
        statement.query(params![
            repo_id,
            path,
            document.content_sha256,
            document.parser_version,
            document.query_pack_version,
            revision,
        ])
    }
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

fn code_snapshot_membership_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_snapshot_membership",
        &[
            "repo_id",
            "commit_sha",
            "path",
            "content_sha256",
            "parser_version",
            "query_pack_version",
            "analyzed",
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

fn code_snapshot_status(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<Option<String>, MemoryError> {
    if !table_has_columns(
        connection,
        &config.index_path,
        "code_index_snapshots",
        &["repo_id", "commit_sha", "status"],
    )? {
        return Ok(None);
    }
    connection
        .query_row(
            "SELECT status FROM code_index_snapshots WHERE repo_id = ? AND commit_sha = ? LIMIT 1",
            params![repo_id, revision],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })
}

fn code_snapshot_membership_rows_available(
    connection: &Connection,
    config: &MemoryConfig,
    repo_id: &str,
    revision: &str,
) -> Result<bool, MemoryError> {
    let mut statement = connection
        .prepare(
            "SELECT 1 FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? AND analyzed LIMIT 1",
        )
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

fn code_diagnostic_revisions_read_model_ready(
    connection: &Connection,
    path: &Path,
) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_diagnostic_revisions",
        &["repo_id", "commit_sha", "path", "content_sha256", "freshness"],
    )
}

fn code_freshness_filter(include_stale: bool) -> &'static str {
    if include_stale {
        "freshness IN ('current', 'stale')"
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
    use crate::opensymphony_code_intel::{CaptureRecord, SourceSpan, parse_path};
    use crate::opensymphony_memory::{
        CodeIntelDiagnosticInput, CodeIntelDocumentInput, CodeIntelEdgeInput,
        CodeIntelPersistBatch,
        CodeIntelSymbolInput, KnowledgeScope, KnowledgeScopeKind, MemoryConfig,
        persist_code_intel_documents,
    };
    use chrono::Utc;
    use duckdb::Connection;
    use std::{collections::BTreeMap, fs, path::{Path, PathBuf}, process::Command, sync::Arc};
    use tempfile::TempDir;

    use super::{
        code_citation_matches_symbol, code_file_outline_from_workspace, code_graph_diff_overlay,
        code_graph_index_report, code_graph_repos, code_graph_snapshot, code_graph_workspace_diff_overlay,
        code_graph_workspace_overlay,
        code_graph_workspace_snapshot,
        assign_edge_keys, code_index_document, code_symbol_span_matches,
        compare_code_edges, detailed_blast_radius, has_work_item_scope,
        module_connection_deltas,
        CodeSnapshotMembershipInput,
        CodeDiffConnectionScope, CodeDiffEdgeStatus, CodeDiffSymbol, CodeGraphConfidence,
        CodeGraphMode, CodeGraphSnapshotOptions, DtoDiffStatus,
        index_code_repository, index_code_repository_at, index_code_repository_at_current_target,
        migrate_index, open_existing_index_read_only, open_index,
        query_code_edges_for_revision, query_revision_file_symbols, query_symbol_diagnostics,
        re_resolve_workspace_edges,
        CodeEdgeRecord, CodeSymbolRecord,
        repository_scope_matches, workspace_git_bytes, CodeGraphProjectionError,
    };

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(["-C"])
            .arg(root)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    #[allow(clippy::too_many_arguments)]
    fn test_edge(
        edge_id: &str,
        path: &str,
        line: usize,
        source: Option<&str>,
        target: Option<&str>,
        hint: Option<&str>,
        confidence: &str,
        unresolved: bool,
    ) -> CodeEdgeRecord {
        CodeEdgeRecord {
            edge_id: edge_id.to_string(),
            edge_key: String::new(),
            edge_kind: "call".to_string(),
            source_symbol_key: source.map(str::to_string),
            target_symbol_key: target.map(str::to_string),
            target_hint: hint.map(str::to_string),
            confidence: confidence.to_string(),
            unresolved,
            path: path.to_string(),
            commit_sha: None,
            freshness: "current".to_string(),
            start_line: line,
            start_col: 0,
            end_line: line,
            end_col: 8,
        }
    }

    fn test_symbol(symbol_key: &str, path: &str) -> CodeSymbolRecord {
        CodeSymbolRecord {
            symbol_id: format!("{symbol_key}:id"),
            symbol_key: symbol_key.to_string(),
            repo_id: "repo".to_string(),
            commit_sha: None,
            path: path.to_string(),
            language: "rust".to_string(),
            kind: "function".to_string(),
            name: symbol_key.to_string(),
            container_symbol_id: None,
            container_chain: Vec::new(),
            signature: None,
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 1,
            start_byte: 0,
            end_byte: 1,
            selection_start_line: 1,
            selection_end_line: 1,
            content_sha256: "content".to_string(),
            snippet_sha256: "snippet".to_string(),
            parser_version: "parser".to_string(),
            query_pack_version: "query-pack".to_string(),
            freshness: "current".to_string(),
            indexed_at: "now".to_string(),
        }
    }

    #[test]
    fn logical_edge_keys_ignore_line_shifts_and_disambiguate_duplicates() {
        let mut base = vec![
            test_edge("base", "src/lib.rs", 10, Some("caller"), Some("callee"), None, "exact", false),
            test_edge("base-duplicate", "src/lib.rs", 20, Some("caller"), Some("callee"), None, "exact", false),
        ];
        let mut head = vec![
            test_edge("head-duplicate", "src/lib.rs", 40, Some("caller"), Some("callee"), None, "exact", false),
            test_edge("head", "src/lib.rs", 30, Some("caller"), Some("callee"), None, "exact", false),
        ];
        assign_edge_keys(&mut base);
        assign_edge_keys(&mut head);
        assert_eq!(
            base.iter().map(|edge| edge.edge_key.clone()).collect::<Vec<_>>(),
            head.iter().map(|edge| edge.edge_key.clone()).collect::<Vec<_>>(),
        );
        let (deltas, dropped) = compare_code_edges(base, head, 20);
        assert!(deltas.is_empty());
        assert_eq!(dropped, 0);

        let (deltas, dropped) = compare_code_edges(
            vec![test_edge(
                "resolved-base",
                "src/lib.rs",
                10,
                Some("caller"),
                Some("callee"),
                Some("callee()"),
                "exact",
                false,
            )],
            vec![test_edge(
                "resolved-head",
                "src/lib.rs",
                10,
                Some("caller"),
                Some("callee"),
                Some("self.callee()"),
                "exact",
                false,
            )],
            20,
        );
        assert!(deltas.is_empty(), "resolved target identity should ignore hint spelling");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn edge_comparison_separates_retargeting_and_confidence_changes() {
        let base = vec![
            test_edge("retarget-base", "src/lib.rs", 10, Some("caller"), Some("old"), None, "exact", false),
            test_edge("confidence-base", "src/lib.rs", 20, Some("caller"), Some("same"), None, "syntactic", false),
        ];
        let head = vec![
            test_edge("retarget-head", "src/lib.rs", 10, Some("caller"), Some("new"), None, "exact", false),
            test_edge("confidence-head", "src/lib.rs", 20, Some("caller"), Some("same"), None, "exact", false),
        ];
        let (deltas, dropped) = compare_code_edges(base, head, 20);
        assert_eq!(dropped, 0);
        assert_eq!(deltas.len(), 2);
        assert!(deltas.iter().any(|delta| delta.status == CodeDiffEdgeStatus::Retargeted));
        assert!(deltas.iter().any(|delta| delta.status == CodeDiffEdgeStatus::ConfidenceChanged));
    }

    #[test]
    fn module_connections_include_scope_counts_and_confidence() {
        let symbols = BTreeMap::from([
            ("caller".to_string(), test_symbol("caller", "src/a.rs")),
            ("callee".to_string(), test_symbol("callee", "packages/b.rs")),
        ]);
        let edges = vec![test_edge(
            "edge",
            "src/a.rs",
            10,
            Some("caller"),
            Some("callee"),
            None,
            "heuristic",
            false,
        )];
        let (deltas, dropped) =
            module_connection_deltas(&[], &edges, &BTreeMap::new(), &symbols, 20);
        assert_eq!(dropped, 0);
        assert_eq!(deltas.len(), 3);
        assert!(deltas.iter().all(|delta| delta.status == CodeDiffEdgeStatus::Added));
        let directory = deltas
            .iter()
            .find(|delta| delta.scope == CodeDiffConnectionScope::Directory)
            .expect("directory connection");
        let after = directory.after.as_ref().expect("directory counts");
        assert_eq!(after.edge_count, 1);
        assert_eq!(after.edge_kind_counts[0].kind, "call");
        assert_eq!(after.confidence_counts[0].confidence, CodeGraphConfidence::Heuristic);
    }

    #[test]
    fn blast_radius_keeps_source_cited_relationship_details() {
        let diffs = vec![CodeDiffSymbol {
            symbol_key: "changed".to_string(),
            status: DtoDiffStatus::Modified,
            before: None,
            after: None,
        }];
        let base = vec![test_edge(
            "inbound",
            "src/caller.rs",
            3,
            Some("caller"),
            Some("changed"),
            None,
            "syntactic",
            false,
        )];
        let head = vec![
            test_edge(
                "inbound-head",
                "src/caller.rs",
                3,
                Some("caller"),
                Some("changed"),
                None,
                "syntactic",
                false,
            ),
            test_edge(
                "outbound",
                "src/changed.rs",
                8,
                Some("changed"),
                Some("callee"),
                None,
                "heuristic",
                false,
            ),
        ];
        let (radius, dropped) = detailed_blast_radius(&diffs, &base, &head, 20);
        assert_eq!(dropped, 0);
        assert_eq!(radius.len(), 1);
        assert_eq!(radius[0].inbound[0].path, "src/caller.rs");
        assert_eq!(radius[0].inbound[0].symbol_key.as_deref(), Some("caller"));
        assert_eq!(radius[0].inbound[0].confidence, CodeGraphConfidence::Syntactic);
        assert_eq!(radius[0].outbound[0].symbol_key.as_deref(), Some("callee"));
        assert_eq!(radius[0].outbound[0].distance, 1);
    }

    #[test]
    fn workspace_edges_do_not_retarget_disappeared_symbols() {
        let symbol = |symbol_key: &str, name: &str| CodeSymbolRecord {
            symbol_id: symbol_key.to_string(),
            symbol_key: symbol_key.to_string(),
            repo_id: "repo".to_string(),
            commit_sha: None,
            path: "src/lib.rs".to_string(),
            language: "rust".to_string(),
            kind: "function".to_string(),
            name: name.to_string(),
            container_symbol_id: None,
            container_chain: Vec::new(),
            signature: None,
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 1,
            start_byte: 0,
            end_byte: 1,
            selection_start_line: 1,
            selection_end_line: 1,
            content_sha256: "content".to_string(),
            snippet_sha256: "snippet".to_string(),
            parser_version: "parser".to_string(),
            query_pack_version: "query-pack".to_string(),
            freshness: "current".to_string(),
            indexed_at: "now".to_string(),
        };
        let symbols = BTreeMap::from([
            ("source".to_string(), symbol("source", "source")),
            ("replacement".to_string(), symbol("replacement", "helper")),
        ]);
        let edge = |edge_id: &str, target_symbol_key: Option<&str>, unresolved: bool| {
            CodeEdgeRecord {
                edge_id: edge_id.to_string(),
                edge_key: String::new(),
                edge_kind: "call".to_string(),
                source_symbol_key: Some("source".to_string()),
                target_symbol_key: target_symbol_key.map(str::to_string),
                target_hint: Some("helper".to_string()),
                confidence: "exact".to_string(),
                unresolved,
                path: "src/lib.rs".to_string(),
                commit_sha: None,
                freshness: "current".to_string(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 1,
            }
        };
        let mut edges = BTreeMap::from([
            ("resolved".to_string(), edge("resolved", Some("deleted"), false)),
            ("unresolved".to_string(), edge("unresolved", None, true)),
        ]);

        re_resolve_workspace_edges(&mut edges, &symbols);

        let disappeared = edges.get("resolved").expect("disappeared edge retained");
        assert_eq!(disappeared.target_symbol_key, None);
        assert!(disappeared.unresolved);
        assert_eq!(
            edges
                .get("unresolved")
                .expect("unresolved edge retained")
                .target_symbol_key
                .as_deref(),
            Some("replacement")
        );
    }

    #[test]
    fn workspace_diff_overlay_uses_indexed_clean_head() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::create_dir_all(repo.path().join("src")).expect("source directory");
        fs::write(repo.path().join("src/lib.rs"), "pub fn base() {}\n")
            .expect("base source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "base"]);
        let base = git(repo.path(), &["rev-parse", "HEAD"]);
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn base() {}\npub fn head() {}\n",
        )
        .expect("head source");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "head"]);
        let head = git(repo.path(), &["rev-parse", "HEAD"]);
        let memory_root = TempDir::new().expect("memory root tempdir");
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.memory_root = memory_root.path().to_path_buf();
        config.index_path = memory_root.path().join("index.duckdb");
        index_code_repository_at(
            &config,
            "clean-head-repo",
            Some(("develop".to_string(), base.clone())),
        )
        .expect("base index");
        index_code_repository_at(
            &config,
            "clean-head-repo",
            Some(("develop".to_string(), head.clone())),
        )
        .expect("head index");

        let mut limited_config = config.clone();
        limited_config.code_intel.ast.max_files_per_request = 0;
        let overlay = code_graph_workspace_diff_overlay(
            &limited_config,
            "clean-head-repo",
            repo.path(),
            "COE-543-clean-head",
            &base,
            500,
        )
        .expect("clean indexed head diff");
        assert_eq!(overlay.head_revision, head);
        assert!(overlay
            .added_symbols
            .iter()
            .any(|symbol| symbol.after.as_ref().is_some_and(|side| side.name == "head")));
        assert!(overlay.unanalyzed_files.is_empty());
    }

    #[test]
    fn completed_snapshot_prefers_exact_revision_edges_over_mutable_membership_rows() {
        let repo = TempDir::new().expect("repository tempdir");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "revision-edge-repo".to_string(),
                commit_sha: Some("snapshot-head".to_string()),
                worktree_dirty: false,
                documents: vec![CodeIntelDocumentInput {
                    path: PathBuf::from("src/lib.rs"),
                    language: "rust".to_string(),
                    content_sha256: "snapshot-content".to_string(),
                    parser_id: "tree-sitter".to_string(),
                    parser_version: "test-parser".to_string(),
                    query_pack_version: "test-query-pack".to_string(),
                    byte_len: 32,
                    line_count: 4,
                    symbols: vec![
                        CodeIntelSymbolInput {
                            kind: "function".to_string(),
                            name: "source".to_string(),
                            container_chain: Vec::new(),
                            signature: Some("fn source()".to_string()),
                            start_line: 1,
                            start_col: 0,
                            end_line: 2,
                            end_col: 1,
                            start_byte: 0,
                            end_byte: 16,
                            selection_start_line: 1,
                            selection_end_line: 1,
                            snippet_sha256: "source-snippet".to_string(),
                        },
                        CodeIntelSymbolInput {
                            kind: "function".to_string(),
                            name: "old_target".to_string(),
                            container_chain: Vec::new(),
                            signature: Some("fn old_target()".to_string()),
                            start_line: 3,
                            start_col: 0,
                            end_line: 4,
                            end_col: 1,
                            start_byte: 17,
                            end_byte: 32,
                            selection_start_line: 3,
                            selection_end_line: 3,
                            snippet_sha256: "target-snippet".to_string(),
                        },
                    ],
                    edges: vec![CodeIntelEdgeInput {
                        edge_kind: "call".to_string(),
                        target_hint: Some("old_target".to_string()),
                        confidence: "exact".to_string(),
                        start_line: 2,
                        start_col: 0,
                        end_line: 2,
                        end_col: 1,
                        start_byte: 8,
                        end_byte: 9,
                    }],
                    diagnostics: Vec::new(),
                }],
            },
        )
        .expect("persist snapshot edge");

        let connection = open_index(&config).expect("open revision edge index");
        migrate_index(&connection).expect("migrate revision edge index");
        connection
            .execute(
                "INSERT INTO code_index_snapshots (repo_id, commit_sha, target_branch, status, total_files, parsed_files, skipped_files, deleted_files, config_fingerprint, indexed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "revision-edge-repo",
                    "snapshot-head",
                    "develop",
                    "completed",
                    1_i64,
                    1_i64,
                    0_i64,
                    0_i64,
                    "test-config",
                    "now",
                ],
            )
            .expect("completed snapshot");
        connection
            .execute(
                "INSERT INTO code_snapshot_membership (repo_id, commit_sha, path, language, content_sha256, parser_version, query_pack_version, analyzed, skip_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "revision-edge-repo",
                    "snapshot-head",
                    "src/lib.rs",
                    "rust",
                    "snapshot-content",
                    "test-parser",
                    "test-query-pack",
                    1_i64,
                    Option::<String>::None,
                ],
            )
            .expect("snapshot membership");
        let exact_target_key = connection
            .query_row(
                "SELECT target_symbol_key FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ?",
                duckdb::params!["revision-edge-repo", "snapshot-head"],
                |row| row.get::<_, String>(0),
            )
            .expect("exact revision target");
        connection
            .execute(
                "UPDATE code_edges SET target_symbol_key = ?, target_hint = ? WHERE repo_id = ?",
                duckdb::params!["latest_target", "latest_target", "revision-edge-repo"],
            )
            .expect("mutate current edge read model");

        let edges = query_code_edges_for_revision(
            &connection,
            &config,
            "revision-edge-repo",
            "snapshot-head",
        )
        .expect("query completed snapshot edges");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_symbol_key.as_deref(), Some(exact_target_key.as_str()));
        assert_eq!(edges[0].target_hint.as_deref(), Some("old_target"));
    }

    #[test]
    fn revision_diagnostics_are_scoped_to_symbol_spans() {
        let repo = TempDir::new().expect("repository tempdir");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "diagnostic-repo".to_string(),
                commit_sha: Some("diagnostic-revision".to_string()),
                worktree_dirty: false,
                documents: vec![CodeIntelDocumentInput {
                    path: PathBuf::from("src/diagnostics.rs"),
                    language: "rust".to_string(),
                    content_sha256: "diagnostic-content".to_string(),
                    parser_id: "tree-sitter".to_string(),
                    parser_version: "test-parser".to_string(),
                    query_pack_version: "test-query-pack".to_string(),
                    byte_len: 64,
                    line_count: 12,
                    symbols: vec![
                        CodeIntelSymbolInput {
                            kind: "function".to_string(),
                            name: "first".to_string(),
                            container_chain: Vec::new(),
                            signature: Some("fn first()".to_string()),
                            start_line: 1,
                            start_col: 0,
                            end_line: 2,
                            end_col: 1,
                            start_byte: 0,
                            end_byte: 16,
                            selection_start_line: 1,
                            selection_end_line: 1,
                            snippet_sha256: "first-snippet".to_string(),
                        },
                        CodeIntelSymbolInput {
                            kind: "function".to_string(),
                            name: "second".to_string(),
                            container_chain: Vec::new(),
                            signature: Some("fn second()".to_string()),
                            start_line: 10,
                            start_col: 0,
                            end_line: 11,
                            end_col: 1,
                            start_byte: 32,
                            end_byte: 50,
                            selection_start_line: 10,
                            selection_end_line: 10,
                            snippet_sha256: "second-snippet".to_string(),
                        },
                    ],
                    edges: Vec::new(),
                    diagnostics: vec![CodeIntelDiagnosticInput {
                        kind: "warning".to_string(),
                        severity: "warning".to_string(),
                        message: "first only".to_string(),
                        start_line: 1,
                        start_col: 0,
                        end_line: 1,
                        end_col: 1,
                        start_byte: 0,
                        end_byte: 1,
                    }],
                }],
            },
        )
        .expect("diagnostic fixture");

        let snapshot = code_graph_snapshot(
            &config,
            "diagnostic-repo",
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::File,
                path: Some("src/diagnostics.rs".to_string()),
                symbol_key: None,
                depth: 1,
                aggregate: None,
                include_stale: false,
            },
        )
        .expect("diagnostic graph");
        let first = snapshot
            .nodes
            .iter()
            .find(|node| node.label == "first")
            .expect("first symbol node");
        let second = snapshot
            .nodes
            .iter()
            .find(|node| node.label == "second")
            .expect("second symbol node");
        assert_eq!(first.diagnostic_count, 1);
        assert_eq!(second.diagnostic_count, 0);

        let connection = open_index(&config).expect("open diagnostic index");
        migrate_index(&connection).expect("migrate diagnostic index");
        connection
            .execute(
                "INSERT INTO code_snapshot_membership (repo_id, commit_sha, path, language, content_sha256, parser_version, query_pack_version, analyzed, skip_reason) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "diagnostic-repo",
                    "diagnostic-snapshot",
                    "src/diagnostics.rs",
                    "rust",
                    "diagnostic-content",
                    "test-parser",
                    "test-query-pack",
                    1_i64,
                    Option::<String>::None,
                ],
            )
            .expect("snapshot membership");
        drop(connection);
        let connection = open_existing_index_read_only(&config)
            .expect("diagnostic index should open")
            .expect("diagnostic index should exist");
        let reused_symbol = query_revision_file_symbols(
            &config,
            &connection,
            "diagnostic-repo",
            "diagnostic-snapshot",
            "src/diagnostics.rs",
        )
        .expect("reused snapshot symbols")
        .into_iter()
        .find(|symbol| symbol.name == "first")
        .expect("reused first symbol");
        assert_eq!(reused_symbol.commit_sha.as_deref(), Some("diagnostic-snapshot"));
        assert_eq!(
            query_symbol_diagnostics(&connection, &config, &reused_symbol, false)
                .expect("reused diagnostics")
                .len(),
            1
        );
    }

    #[test]
    fn workspace_overlay_composes_live_changes_and_coverage() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::create_dir_all(repo.path().join("src")).expect("source directory");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn baseline() {}\npub fn removed_from_live() {}\npub fn caller() { baseline(); }\n",
        )
        .expect("baseline source");
        fs::write(repo.path().join("src/target.rs"), "pub fn target() {}\n")
            .expect("unchanged target source");
        fs::write(repo.path().join("src/remove.rs"), "pub fn removed() {}\n")
            .expect("removed source");
        fs::write(repo.path().join("src/delete_empty.rs"), "").expect("empty deleted source");
        fs::write(
            repo.path().join("src/large.rs"),
            "pub fn baseline_large() {}\n",
        )
        .expect("baseline oversized source");
        fs::write(repo.path().join("src/empty.rs"), "").expect("baseline empty source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "overlay baseline"]);
        let base = git(repo.path(), &["rev-parse", "HEAD"]);
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        index_code_repository_at(
            &config,
            "overlay-repo",
            Some(("develop".to_string(), base.clone())),
        )
        .expect("baseline index");

        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn baseline() { changed(); }\npub fn changed() {}\npub fn caller() { baseline(); }\npub fn changed_caller() { target(); }\n",
        )
        .expect("modified source");
        fs::remove_file(repo.path().join("src/remove.rs")).expect("deleted source");
        fs::remove_file(repo.path().join("src/delete_empty.rs")).expect("deleted empty source");
        fs::write(repo.path().join("src/new.rs"), "pub fn added() {}\n").expect("added source");
        fs::write(repo.path().join("src/zchanged_empty.rs"), "").expect("empty added source");
        fs::write(repo.path().join("notes.txt"), "unsupported\n").expect("unsupported source");
        fs::write(
            repo.path().join("src/large.rs"),
            "pub fn too_large() {}\n".repeat(10),
        )
        .expect("oversized source");
        #[cfg(unix)]
        let _outside = {
            let outside = TempDir::new().expect("outside tempdir");
            fs::write(outside.path().join("escaped.rs"), "pub fn escaped() {}\n")
                .expect("outside source");
            std::os::unix::fs::symlink(
                outside.path().join("escaped.rs"),
                repo.path().join("src/linked.rs"),
            )
            .expect("workspace symlink");
            outside
        };
        let mut limited_config = config.clone();
        limited_config.code_intel.ast.max_file_bytes = 128;

        let overlay = code_graph_workspace_overlay(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
        )
        .expect("workspace overlay");
        assert!(overlay.changed_paths.contains("src/lib.rs"));
        assert!(overlay.changed_paths.contains("src/new.rs"));
        assert!(overlay.changed_paths.contains("src/remove.rs"));
        assert!(overlay.tombstones.contains("src/remove.rs"));
        assert!(overlay
            .symbols
            .values()
            .any(|symbol| symbol.name == "changed"));
        assert!(overlay.symbols.values().any(|symbol| symbol.name == "added"));
        assert!(!overlay
            .unanalyzed_files
            .iter()
            .any(|path| path == "src/zchanged_empty.rs"));
        assert!(overlay
            .symbols
            .values()
            .any(|symbol| symbol.name == "baseline_large"));
        let outline = code_file_outline_from_workspace(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            "src/large.rs",
        )
        .expect("oversized edit should retain baseline outline");
        assert!(outline.symbols.iter().any(|symbol| symbol.name == "baseline_large"));
        let empty_snapshot = code_graph_workspace_snapshot(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::File,
                path: Some("src/empty.rs".to_string()),
                symbol_key: None,
                depth: 1,
                aggregate: None,
                include_stale: false,
            },
        )
        .expect("indexed empty file should have a file graph");
        assert!(empty_snapshot
            .nodes
            .iter()
            .any(|node| node.id == "file:src/empty.rs"));
        let directory_snapshot = code_graph_workspace_snapshot(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::Atlas,
                path: None,
                symbol_key: None,
                depth: 1,
                aggregate: Some(super::CodeGraphAggregate::Directory),
                include_stale: false,
            },
        )
        .expect("directory atlas graph");
        assert!(directory_snapshot
            .nodes
            .iter()
            .all(|node| !node.id.starts_with("symbol:") && !node.id.starts_with("hint:")));
        let caller_key = overlay
            .symbols
            .values()
            .find(|symbol| symbol.name == "caller")
            .map(|symbol| symbol.symbol_key.clone())
            .expect("changed caller symbol");
        let neighborhood_snapshot = code_graph_workspace_snapshot(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::Neighborhood,
                path: None,
                symbol_key: Some(caller_key),
                depth: 1,
                aggregate: None,
                include_stale: false,
            },
        )
        .expect("focused neighborhood graph");
        assert!(neighborhood_snapshot
            .nodes
            .iter()
            .any(|node| node.label == "caller"));
        assert!(neighborhood_snapshot
            .nodes
            .iter()
            .any(|node| node.label == "baseline"));
        let target_key = overlay
            .base_symbols
            .values()
            .find(|symbol| symbol.name == "target")
            .map(|symbol| symbol.symbol_key.clone())
            .expect("unchanged baseline target symbol");
        let target_neighborhood = code_graph_workspace_snapshot(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::Neighborhood,
                path: None,
                symbol_key: Some(target_key),
                depth: 1,
                aggregate: None,
                include_stale: false,
            },
        )
        .expect("focused neighborhood should include changed caller edges");
        assert!(target_neighborhood
            .nodes
            .iter()
            .any(|node| node.label == "changed_caller"));
        let removed_from_live_key = overlay
            .base_symbols
            .values()
            .find(|symbol| symbol.name == "removed_from_live")
            .map(|symbol| symbol.symbol_key.clone())
            .expect("baseline-only symbol");
        assert!(matches!(
            code_graph_workspace_snapshot(
                &limited_config,
                "overlay-repo",
                repo.path(),
                "COE-543",
                &base,
                CodeGraphSnapshotOptions {
                    mode: CodeGraphMode::Neighborhood,
                    path: None,
                    symbol_key: Some(removed_from_live_key),
                    depth: 1,
                    aggregate: None,
                    include_stale: false,
                },
            ),
            Err(CodeGraphProjectionError::SymbolNotFound(_))
        ));
        let deleted_snapshot = code_graph_workspace_snapshot(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::File,
                path: Some("src/remove.rs".to_string()),
                symbol_key: None,
                depth: 1,
                aggregate: None,
                include_stale: false,
            },
        )
        .expect("deleted file should retain its baseline graph");
        assert!(deleted_snapshot
            .nodes
            .iter()
            .any(|node| node.id == "file:src/remove.rs"));
        assert!(deleted_snapshot
            .nodes
            .iter()
            .any(|node| node.label == "removed" && node.freshness == super::CodeGraphFreshness::Stale));
        assert!(!overlay.symbols.values().any(|symbol| symbol.name == "removed"));
        assert!(overlay
            .unanalyzed_files
            .iter()
            .any(|path| path == "notes.txt"));
        assert!(overlay
            .unanalyzed_files
            .iter()
            .any(|path| path == "src/large.rs"));
        assert!(overlay
            .unanalyzed_files
            .iter()
            .any(|path| path == "src/delete_empty.rs"));
        #[cfg(unix)]
        {
            assert!(overlay
                .unanalyzed_files
                .iter()
                .any(|path| path == "src/linked.rs"));
            assert!(!overlay.symbols.values().any(|symbol| symbol.name == "escaped"));
        }
        assert_eq!(overlay.base_revision, base);
        assert_eq!(overlay.run_id, "COE-543");
        assert!(!overlay.workspace_content_digest.is_empty());
        let diff = code_graph_workspace_diff_overlay(
            &limited_config,
            "overlay-repo",
            repo.path(),
            "COE-543",
            &base,
            500,
        )
        .expect("workspace diff overlay");
        assert!(diff
            .added_symbols
            .iter()
            .any(|symbol| symbol.after.as_ref().is_some_and(|side| side.name == "added")));
        assert!(diff
            .removed_symbols
            .iter()
            .any(|symbol| symbol.before.as_ref().is_some_and(|side| side.name == "removed")));
        assert!(diff
            .modified_symbols
            .iter()
            .any(|symbol| symbol.after.as_ref().is_some_and(|side| side.name == "baseline")));
        assert!(diff.blast_radius.iter().any(|radius| {
            diff.modified_symbols
                .iter()
                .find(|symbol| symbol.after.as_ref().is_some_and(|side| side.name == "baseline"))
                .is_some_and(|symbol| symbol.symbol_key == radius.symbol_key)
                && radius.inbound_count > 0
        }));

        let mut budget_config = limited_config.clone();
        budget_config.code_intel.ast.max_files_per_request = 1;
        let budget_overlay = code_graph_workspace_overlay(
            &budget_config,
            "overlay-repo",
            repo.path(),
            "COE-543-budget",
            &base,
        )
        .expect("workspace overlay with file budget");
        assert!(budget_overlay.symbols.values().any(|symbol| symbol.name == "changed"));
        assert!(!budget_overlay
            .unanalyzed_files
            .iter()
            .any(|path| path == "src/lib.rs"));
        let mut no_parse_config = limited_config.clone();
        no_parse_config.code_intel.ast.max_files_per_request = 0;
        let baseline_key = overlay
            .base_symbols
            .values()
            .find(|symbol| symbol.name == "baseline")
            .map(|symbol| symbol.symbol_key.clone())
            .expect("baseline symbol");
        let unanalyzed_neighborhood = code_graph_workspace_snapshot(
            &no_parse_config,
            "overlay-repo",
            repo.path(),
            "COE-543-unparsed",
            &base,
            CodeGraphSnapshotOptions {
                mode: CodeGraphMode::Neighborhood,
                path: None,
                symbol_key: Some(baseline_key),
                depth: 1,
                aggregate: None,
                include_stale: false,
            },
        )
        .expect("unanalyzed baseline neighborhood");
        assert!(unanalyzed_neighborhood
            .nodes
            .iter()
            .any(|node| node.label == "caller"));
    }

    #[test]
    fn workspace_git_errors_do_not_include_workspace_paths() {
        let workspace = TempDir::new().expect("workspace tempdir");
        let error = workspace_git_bytes(workspace.path(), ["status"])
            .expect_err("non-git workspace should fail");
        assert!(!error.to_string().contains(&workspace.path().display().to_string()));
        assert!(error
            .to_string()
            .contains("git command failed in run workspace"));
    }

    #[test]
    fn workspace_overlays_are_isolated_and_rebuild_without_persisted_state() {
        let source = TempDir::new().expect("source repository tempdir");
        fs::create_dir_all(source.path().join("src")).expect("source directory");
        fs::write(
            source.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(source.path().join("src/lib.rs"), "pub fn baseline() {}\n")
            .expect("baseline source");
        git(source.path(), &["init", "-b", "develop"]);
        git(source.path(), &["config", "user.email", "test@example.com"]);
        git(source.path(), &["config", "user.name", "Test User"]);
        git(source.path(), &["add", "."]);
        git(source.path(), &["commit", "-m", "shared baseline"]);
        let base = git(source.path(), &["rev-parse", "HEAD"]);
        let config = MemoryConfig::load(source.path(), None).expect("memory config");
        index_code_repository_at(
            &config,
            "shared-repo",
            Some(("develop".to_string(), base.clone())),
        )
        .expect("shared baseline index");

        assert!(matches!(
            code_graph_workspace_overlay(
                &config,
                "missing-repo",
                source.path(),
                "COE-543-missing-repo",
                &base,
            ),
            Err(CodeGraphProjectionError::RepoNotFound(_))
        ));
        assert!(matches!(
            code_graph_workspace_overlay(
                &config,
                "shared-repo",
                source.path(),
                "COE-543-missing-revision",
                "missing-revision",
            ),
            Err(CodeGraphProjectionError::RevisionNotFound(_))
        ));

        let left = TempDir::new().expect("left workspace tempdir");
        let right = TempDir::new().expect("right workspace tempdir");
        git(
            left.path(),
            &[
                "clone",
                source.path().to_string_lossy().as_ref(),
                left.path().join("workspace").to_string_lossy().as_ref(),
            ],
        );
        git(
            right.path(),
            &[
                "clone",
                source.path().to_string_lossy().as_ref(),
                right.path().join("workspace").to_string_lossy().as_ref(),
            ],
        );
        let left_workspace = left.path().join("workspace");
        let right_workspace = right.path().join("workspace");
        fs::write(left_workspace.join("src/lib.rs"), "pub fn left_only() {}\n")
            .expect("left edit");
        fs::write(right_workspace.join("src/lib.rs"), "pub fn right_only() {}\n")
            .expect("right edit");

        let left_overlay = code_graph_workspace_overlay(
            &config,
            "shared-repo",
            &left_workspace,
            "COE-543-left",
            &base,
        )
        .expect("left overlay");
        let right_overlay = code_graph_workspace_overlay(
            &config,
            "shared-repo",
            &right_workspace,
            "COE-543-right",
            &base,
        )
        .expect("right overlay");
        assert!(left_overlay.symbols.values().any(|symbol| symbol.name == "left_only"));
        assert!(!left_overlay.symbols.values().any(|symbol| symbol.name == "right_only"));
        assert!(right_overlay
            .symbols
            .values()
            .any(|symbol| symbol.name == "right_only"));
        assert!(!right_overlay.symbols.values().any(|symbol| symbol.name == "left_only"));
        assert_ne!(
            left_overlay.workspace_content_digest,
            right_overlay.workspace_content_digest
        );

        let rebuilt = code_graph_workspace_overlay(
            &config,
            "shared-repo",
            &left_workspace,
            "COE-543-left",
            &base,
        )
        .expect("overlay should rebuild after process-local cache reuse");
        assert_eq!(rebuilt.workspace_content_digest, left_overlay.workspace_content_digest);
        assert_eq!(rebuilt.symbols, left_overlay.symbols);
        fs::remove_dir_all(&left_workspace).expect("workspace cleanup");
        assert!(matches!(
            code_graph_workspace_overlay(
                &config,
                "shared-repo",
                &left_workspace,
                "COE-543-left",
                &base,
            ),
            Err(CodeGraphProjectionError::InvalidRequest(_))
        ));
    }

    #[test]
    fn target_branch_index_bootstraps_and_advances_immutable_snapshots() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::create_dir_all(repo.path().join("src")).expect("src directory");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "## Branch target\n\nTarget branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn main_branch() {}\n",
        )
        .expect("main source");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "main"]);
        git(repo.path(), &["switch", "-c", "develop"]);
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn develop_branch() { helper(); }\n",
        )
        .expect("develop source");
        fs::write(repo.path().join("src/helper.rs"), "pub fn helper() {}\n")
            .expect("helper source");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "develop baseline"]);
        let baseline = git(repo.path(), &["rev-parse", "HEAD"]);
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        let source = fs::read_to_string(repo.path().join("src/lib.rs")).expect("baseline source");
        let summary = parse_path(Path::new("src/lib.rs"), &source).expect("baseline parse");
        let mut remaining_matches = config.code_intel.ast.max_matches_per_request;
        let legacy_document = code_index_document(
            PathBuf::from("src/lib.rs"),
            &source,
            &summary,
            &mut remaining_matches,
            config.code_intel.ast.max_capture_bytes,
        );
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "fixture-repo".to_string(),
                commit_sha: Some("legacy-code-intel".to_string()),
                worktree_dirty: false,
                documents: vec![legacy_document.clone()],
            },
        )
        .expect("legacy code-intel document should persist");

        let first = index_code_repository(&config, "fixture-repo").expect("baseline index");
        assert_eq!(first.status, crate::opensymphony_gateway_schema::code_graph::CodeIndexStatus::Completed);
        assert_eq!(first.head_revision.as_deref(), Some(baseline.as_str()));
        assert!(first.persisted_documents > 0);
        assert!(first.persisted_symbols > 0);
        assert!(first.persisted_edges > 0);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let freshness: String = connection
            .query_row(
                "SELECT freshness FROM code_documents WHERE repo_id = ? AND path = ?",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("legacy document should remain current");
        assert_eq!(freshness, "current");
        let (symbol_count, unique_symbol_count): (i64, i64) = connection
            .query_row(
                "SELECT COUNT(*), COUNT(DISTINCT symbol_key) FROM code_symbols WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("current symbols should remain unique");
        assert_eq!(symbol_count, unique_symbol_count);
        let current_edge_commit: String = connection
            .query_row(
                "SELECT commit_sha FROM code_edges WHERE repo_id = ? AND path = ? AND freshness = 'current' LIMIT 1",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("target snapshot edge should be current");
        assert_eq!(current_edge_commit, baseline);
        drop(connection);

        let current_membership_parser: String = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist")
            .query_row(
                "SELECT parser_version FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? AND path = ?",
                duckdb::params!["fixture-repo", baseline, "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("current membership should persist");
        super::persist_code_snapshot_membership(
            &config,
            "fixture-repo",
            &baseline,
            "interrupted-membership-run",
            &[CodeSnapshotMembershipInput {
                path: PathBuf::from("src/lib.rs"),
                language: "rust".to_string(),
                content_sha256: "replacement-hash".to_string(),
                parser_version: "replacement-parser".to_string(),
                query_pack_version: "replacement-query-pack".to_string(),
                analyzed: true,
                skip_reason: None,
            }],
        )
        .expect("membership should stage separately from the completed snapshot");
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let preserved_membership_parser: String = connection
            .query_row(
                "SELECT parser_version FROM code_snapshot_membership WHERE repo_id = ? AND commit_sha = ? AND path = ?",
                duckdb::params!["fixture-repo", baseline, "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("completed membership should remain readable");
        let staged_membership_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_snapshot_membership_staging WHERE run_id = ?",
                duckdb::params!["interrupted-membership-run"],
                |row| row.get(0),
            )
            .expect("staged membership should be isolated");
        assert_eq!(preserved_membership_parser, current_membership_parser);
        assert_eq!(staged_membership_count, 1);
        drop(connection);

        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "fixture-repo".to_string(),
                commit_sha: Some("workspace-dirty".to_string()),
                worktree_dirty: true,
                documents: vec![legacy_document.clone()],
            },
        )
        .expect("dirty workspace document should persist");
        index_code_repository(&config, "fixture-repo")
            .expect("clean target rows should replace dirty rows");
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let clean_documents: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current' AND NOT worktree_dirty",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("clean document count should be readable");
        let dirty_documents: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current' AND worktree_dirty",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("dirty document count should be readable");
        assert_eq!(clean_documents, 1);
        assert_eq!(dirty_documents, 0);
        drop(connection);

        let before_staged_symbols = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist")
            .query_row(
                "SELECT COUNT(*) FROM code_symbols WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| row.get::<_, i64>(0),
            )
            .expect("current symbol count should be readable");
        super::persist_code_index_documents_in_batches(
            &config,
            "fixture-repo",
            &baseline,
            vec![legacy_document.clone()],
            "staged",
            false,
        )
        .expect("same-commit symbols should stage");
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let after_staged_symbols: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_symbols WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["fixture-repo", "src/lib.rs"],
                |row| row.get(0),
            )
            .expect("current symbols should survive staging");
        assert_eq!(after_staged_symbols, before_staged_symbols);
        drop(connection);

        let mut stale_document = legacy_document.clone();
        stale_document.path = PathBuf::from("stale.rs");
        super::persist_code_index_documents_in_batches(
            &config,
            "fixture-repo",
            &baseline,
            vec![stale_document],
            "staged",
            false,
        )
        .expect("interrupted rows should stage");

        let refreshed = index_code_repository(&config, "fixture-repo")
            .expect("same revision should be revalidated");
        assert!(refreshed.parsed_files > 0);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let stale_current: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["fixture-repo", "stale.rs"],
                |row| row.get(0),
            )
            .expect("stale staged document query should work");
        let stale_staged: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'staged'",
                duckdb::params!["fixture-repo", "stale.rs"],
                |row| row.get(0),
            )
            .expect("staged cleanup query should work");
        assert_eq!(stale_current, 0);
        assert_eq!(stale_staged, 0);
        drop(connection);
        super::persist_code_snapshot_state(
            &config,
            super::CodeSnapshotState {
                repo_id: "fixture-repo",
                commit_sha: &baseline,
                target_branch: "develop",
                status: "running",
                total_files: 0,
                parsed_files: 0,
                skipped_files: 0,
                deleted_files: 0,
                config_fingerprint: "",
                indexed_at: Utc::now(),
            },
        )
        .expect("same-revision reindex should keep completed snapshot");
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let snapshot_status: String = connection
            .query_row(
                "SELECT status FROM code_index_snapshots WHERE repo_id = ? AND commit_sha = ?",
                duckdb::params!["fixture-repo", baseline],
                |row| row.get(0),
            )
            .expect("snapshot should exist");
        assert_eq!(snapshot_status, "completed");
        drop(connection);
        super::persist_code_snapshot_state(
            &config,
            super::CodeSnapshotState {
                repo_id: "fixture-repo",
                commit_sha: "legacy-code-intel",
                target_branch: "develop",
                status: "running",
                total_files: 0,
                parsed_files: 0,
                skipped_files: 0,
                deleted_files: 0,
                config_fingerprint: "",
                indexed_at: Utc::now(),
            },
        )
        .expect("record legacy interrupted index");
        assert!(code_graph_diff_overlay(
            &config,
            "fixture-repo",
            "legacy-code-intel",
            &baseline,
            500,
        )
        .is_ok());

        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn develop_branch() {}\n",
        )
        .expect("changed source");
        fs::remove_file(repo.path().join("src/helper.rs")).expect("deleted source");
        fs::write(repo.path().join("assets.bin"), [1_u8, 2, 3]).expect("skipped source");
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-m", "develop change"]);
        let changed = git(repo.path(), &["rev-parse", "HEAD"]);

        super::persist_code_snapshot_state(
            &config,
            super::CodeSnapshotState {
                repo_id: "fixture-repo",
                commit_sha: &changed,
                target_branch: "develop",
                status: "running",
                total_files: 0,
                parsed_files: 0,
                skipped_files: 0,
                deleted_files: 0,
                config_fingerprint: "",
                indexed_at: Utc::now(),
            },
        )
        .expect("record interrupted index");
        let during_interrupted = code_graph_repos(&config, false).expect("current graph remains readable");
        assert_eq!(
            during_interrupted.repos[0].head_revision.as_deref(),
            Some(baseline.as_str())
        );

        let second = index_code_repository(&config, "fixture-repo").expect("incremental index");
        assert_eq!(second.head_revision.as_deref(), Some(changed.as_str()));
        assert_eq!(second.parsed_files, 1);
        assert!(second.skipped_files.iter().any(|path| path.contains("assets.bin")));
        let overlay = code_graph_diff_overlay(&config, "fixture-repo", &baseline, &changed, 500)
            .expect("immutable revisions remain queryable");
        assert!(!overlay.removed_symbols.is_empty());

        git(repo.path(), &["commit", "--allow-empty", "-m", "identical content"]);
        let identical = git(repo.path(), &["rev-parse", "HEAD"]);
        let third = index_code_repository(&config, "fixture-repo").expect("identical index");
        assert_eq!(third.head_revision.as_deref(), Some(identical.as_str()));
        assert_eq!(third.parsed_files, 0);
        assert!(code_graph_diff_overlay(&config, "fixture-repo", &changed, &identical, 500).is_ok());

        let repos = code_graph_repos(&config, false).expect("current repo list");
        assert_eq!(repos.repos.len(), 1);
        assert_eq!(repos.repos[0].repo_id, "fixture-repo");
        assert_eq!(repos.repos[0].head_revision.as_deref(), Some(identical.as_str()));

        let bound = index_code_repository_at(
            &config,
            "bound-repo",
            Some(("develop".to_string(), baseline.clone())),
        )
        .expect("explicit accepted revision should index");
        assert_eq!(bound.head_revision.as_deref(), Some(baseline.as_str()));
        let stale_bound = index_code_repository_at_current_target(
            &config,
            "stale-bound-repo",
            Some(("develop".to_string(), baseline.clone())),
        )
        .expect("stale accepted revision should be rejected without indexing");
        assert_eq!(
            stale_bound.status,
            crate::opensymphony_gateway_schema::code_graph::CodeIndexStatus::Unavailable
        );
        assert!(stale_bound
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("advanced before index promotion")));

        fs::write(repo.path().join("src/new.rs"), "pub fn new_symbol() {}\n")
            .expect("new source");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "new source"]);

        let config = Arc::new(config);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let config = Arc::clone(&config);
                scope.spawn(move || {
                    index_code_repository(&config, "fixture-repo").expect("serialized reindex");
                });
            }
        });
    }

    #[test]
    fn target_index_reparses_unchanged_dirty_documents() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(repo.path().join("WORKFLOW.md"), "Target branch: `develop`\n")
            .expect("workflow marker");
        fs::write(repo.path().join("src.rs"), "pub fn indexed() {}\n").expect("source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "dirty reuse baseline"]);

        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        index_code_repository(&config, "dirty-reuse-repo").expect("baseline index");
        let source = fs::read_to_string(repo.path().join("src.rs")).expect("source");
        let summary = parse_path(Path::new("src.rs"), &source).expect("parse");
        let mut remaining_matches = config.code_intel.ast.max_matches_per_request;
        let document = code_index_document(
            PathBuf::from("src.rs"),
            &source,
            &summary,
            &mut remaining_matches,
            config.code_intel.ast.max_capture_bytes,
        );
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "dirty-reuse-repo".to_string(),
                commit_sha: Some("workspace-dirty".to_string()),
                worktree_dirty: true,
                documents: vec![document],
            },
        )
        .expect("dirty document should persist");

        git(repo.path(), &["commit", "--allow-empty", "-m", "dirty reuse advance"]);
        let report = index_code_repository(&config, "dirty-reuse-repo")
            .expect("unchanged dirty document should be reparsed");
        assert!(report.parsed_files > 0);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let dirty_current: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current' AND worktree_dirty",
                duckdb::params!["dirty-reuse-repo", "src.rs"],
                |row| row.get(0),
            )
            .expect("dirty row count should be readable");
        assert_eq!(dirty_current, 0);
    }

    #[test]
    fn target_tree_listing_prunes_skipped_directories_before_recursion() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::create_dir_all(repo.path().join("src")).expect("src directory");
        fs::create_dir_all(repo.path().join("node_modules/dependency")).expect("node modules");
        fs::write(repo.path().join("src/lib.rs"), "pub fn indexed() {}\n")
            .expect("source file");
        fs::write(
            repo.path().join("node_modules/dependency/generated.js"),
            "module.exports = {};\n",
        )
        .expect("skipped file");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "tree"]);
        let commit = git(repo.path(), &["rev-parse", "HEAD"]);

        let paths = super::git_tree_paths(repo.path(), &commit).expect("tree should list");
        assert!(paths.iter().any(|(path, _, _)| path == Path::new("src/lib.rs")));
        assert!(paths
            .iter()
            .any(|(path, mode, _)| path == Path::new("node_modules") && mode == "040000"));
        assert!(!paths.iter().any(|(path, _, _)| {
            path != Path::new("node_modules")
                && path.starts_with(Path::new("node_modules"))
        }));
    }

    #[test]
    fn target_tree_listing_is_scoped_to_configured_repository_root() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::create_dir_all(repo.path().join("configured/src")).expect("configured source");
        fs::write(repo.path().join("outside.rs"), "pub fn outside() {}\n")
            .expect("outside source");
        fs::write(repo.path().join("configured/src/lib.rs"), "pub fn inside() {}\n")
            .expect("inside source");
        fs::write(
            repo.path().join("configured/WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "scoped tree"]);
        let commit = git(repo.path(), &["rev-parse", "HEAD"]);

        let paths = super::git_tree_paths(&repo.path().join("configured"), &commit)
            .expect("tree should stay scoped");
        assert!(paths
            .iter()
            .any(|(path, _, _)| path == Path::new("src/lib.rs")));
        assert!(!paths
            .iter()
            .any(|(path, _, _)| path == Path::new("outside.rs")));
        let config = MemoryConfig::load(repo.path().join("configured"), None)
            .expect("scoped memory config should load");
        let report = index_code_repository(&config, "scoped-repo")
            .expect("scoped target index should read configured blobs");
        assert!(report.parsed_files >= 1);
        assert!(super::code_graph_file_snapshot(&config, "scoped-repo", "src/lib.rs", false).is_ok());
        assert!(super::code_graph_file_snapshot(&config, "scoped-repo", "outside.rs", false).is_err());
    }

    #[test]
    fn target_index_reparses_files_previously_skipped_by_file_limit() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(repo.path().join("a.rs"), "pub fn first() {}\n").expect("first source");
        fs::write(repo.path().join("b.rs"), "pub fn second() {}\n").expect("second source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "limited tree"]);
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.code_intel.ast.max_files_per_request = 1;
        let first = index_code_repository(&config, "limited-repo").expect("limited index");
        assert_eq!(first.parsed_files, 1);

        git(repo.path(), &["commit", "--allow-empty", "-m", "raise limit"]);
        config.code_intel.ast.max_files_per_request = 2;
        let second = index_code_repository(&config, "limited-repo").expect("expanded index");
        assert!(second.parsed_files > first.parsed_files);
        assert!(!second
            .skipped_files
            .iter()
            .any(|path| path.contains("max files per request")));
    }

    #[test]
    fn target_index_reparses_files_previously_skipped_by_byte_limit() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(repo.path().join("large.rs"), "pub fn large() {}\n")
            .expect("source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "byte limited tree"]);
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.code_intel.ast.max_file_bytes = 1;
        let first = index_code_repository(&config, "byte-limited-repo").expect("limited index");
        assert_eq!(first.parsed_files, 0);

        git(repo.path(), &["commit", "--allow-empty", "-m", "raise byte limit"]);
        config.code_intel.ast.max_file_bytes = 1024;
        let second = index_code_repository(&config, "byte-limited-repo")
            .expect("expanded index");
        assert!(second.parsed_files > first.parsed_files);
        assert!(!second
            .skipped_files
            .iter()
            .any(|path| path.contains("max file size")));
    }

    #[test]
    fn code_repo_summary_excludes_skipped_only_snapshot_membership() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(repo.path().join("notes.txt"), "not indexed\n").expect("unsupported source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "skipped-only snapshot"]);
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.code_intel.ast.max_file_bytes = 1;

        let report = index_code_repository(&config, "skipped-only-repo").expect("index");
        assert_eq!(report.parsed_files, 0);
        let summary = code_graph_repos(&config, false)
            .expect("repo summaries")
            .repos
            .into_iter()
            .find(|repo| repo.repo_id == "skipped-only-repo")
            .expect("skipped-only repo summary");
        assert_eq!(summary.document_count, 0);
        assert!(summary.languages.is_empty());
    }

    #[test]
    fn empty_repo_discovery_uses_origin_id_and_stays_unavailable() {
        let repo = TempDir::new().expect("repository tempdir");
        git(repo.path(), &["init", "-b", "develop"]);
        git(
            repo.path(),
            &["remote", "add", "origin", "git@github.com:example/canonical.git"],
        );
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        let repos = code_graph_repos(&config, false).expect("empty repo discovery");
        assert_eq!(repos.repos[0].repo_id, "canonical");
        assert!(!repos.repos[0].indexed);

        let report = code_graph_index_report(&config, "canonical").expect("index report");
        assert_eq!(
            report.status,
            crate::opensymphony_gateway_schema::code_graph::CodeIndexStatus::Unavailable
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("has not been indexed")));
    }

    #[test]
    fn repo_discovery_tolerates_legacy_index_without_snapshot_tables() {
        let repo = TempDir::new().expect("repository tempdir");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        fs::create_dir_all(config.index_path.parent().expect("index parent"))
            .expect("index directory");
        let connection = Connection::open(&config.index_path).expect("legacy index");
        connection
            .execute_batch(
                "CREATE TABLE code_documents (
                    repo_id TEXT NOT NULL,
                    path TEXT NOT NULL,
                    language TEXT NOT NULL,
                    indexed_at TEXT NOT NULL,
                    freshness TEXT NOT NULL,
                    commit_sha TEXT,
                    worktree_dirty BOOLEAN NOT NULL
                );",
            )
            .expect("legacy code documents table");
        drop(connection);

        let repos = code_graph_repos(&config, false).expect("legacy repo discovery");
        assert_eq!(repos.repos.len(), 1);
        assert!(!repos.repos[0].indexed);
    }

    #[test]
    fn target_index_records_gitlink_coverage() {
        let repo = TempDir::new().expect("repository tempdir");
        let submodule = TempDir::new().expect("submodule tempdir");
        fs::write(submodule.path().join("README.md"), "submodule\n")
            .expect("submodule file");
        git(submodule.path(), &["init", "-b", "develop"]);
        git(submodule.path(), &["config", "user.email", "test@example.com"]);
        git(submodule.path(), &["config", "user.name", "Test User"]);
        git(submodule.path(), &["add", "."]);
        git(submodule.path(), &["commit", "-m", "submodule"]);
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::create_dir_all(repo.path().join("src")).expect("source directory");
        fs::write(repo.path().join("src/lib.rs"), "pub fn indexed() {}\n")
            .expect("source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "gitlink base"]);
        git(
            repo.path(),
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                submodule.path().to_string_lossy().as_ref(),
                "deps/submodule",
            ],
        );
        git(repo.path(), &["commit", "-m", "gitlink pointer"]);
        let head = git(repo.path(), &["rev-parse", "HEAD"]);

        let paths = super::git_tree_paths(repo.path(), &head).expect("tree should include gitlink");
        assert!(paths.iter().any(|(path, mode, _)| {
            path == Path::new("deps/submodule") && mode == "160000"
        }));

        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        let report = index_code_repository(&config, "gitlink-repo").expect("index");
        assert!(report
            .skipped_files
            .iter()
            .any(|path| path.contains("deps/submodule: gitlink not indexed")));
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let reason: String = connection
            .query_row(
                "SELECT skip_reason FROM code_snapshot_membership WHERE repo_id = ? AND path = ?",
                duckdb::params!["gitlink-repo", "deps/submodule"],
                |row| row.get(0),
            )
            .expect("gitlink membership should persist");
        assert_eq!(reason, "gitlink not indexed");
    }

    #[test]
    fn target_index_reparses_unchanged_files_when_edge_limits_change() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("workflow marker");
        fs::write(
            repo.path().join("src.rs"),
            "pub fn helper() {}\npub fn caller() { helper(); }\n",
        )
        .expect("source");
        git(repo.path(), &["init", "-b", "develop"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "bounded edges"]);
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.code_intel.ast.max_matches_per_request = 100;
        let first = index_code_repository(&config, "edge-limit-repo").expect("expanded index");

        git(repo.path(), &["commit", "--allow-empty", "-m", "lower edge limit"]);
        config.code_intel.ast.max_matches_per_request = 0;
        let second = index_code_repository(&config, "edge-limit-repo").expect("limited index");
        assert!(
            first.parsed_files > 0,
            "first parsed {}",
            first.parsed_files
        );
        assert!(second.parsed_files > 0);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let current_edges: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_edges WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["edge-limit-repo", "src.rs"],
                |row| row.get(0),
            )
            .expect("edge count should be readable");
        assert_eq!(current_edges, 0);
        let stale_edges: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_edges WHERE repo_id = ? AND path = ? AND freshness = 'stale'",
                duckdb::params!["edge-limit-repo", "src.rs"],
                |row| row.get(0),
            )
            .expect("stale edge count should be readable");
        assert!(stale_edges > 0);
        drop(connection);

        git(repo.path(), &["commit", "--allow-empty", "-m", "raise edge limit"]);
        config.code_intel.ast.max_matches_per_request = 100;
        let third = index_code_repository(&config, "edge-limit-repo").expect("raised index");
        assert!(third.parsed_files > 0);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let current_edges_after_raise: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_edges WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["edge-limit-repo", "src.rs"],
                |row| row.get(0),
            )
            .expect("raised edge count should be readable");
        assert!(current_edges_after_raise > 0);
    }

    #[test]
    fn code_index_is_unavailable_without_opening_disabled_memory() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(repo.path().join("src.rs"), "pub fn source() {}\n").expect("source");
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config.enabled = false;

        let report = index_code_repository_at(
            &config,
            "disabled-memory-repo",
            Some(("develop".to_string(), "commit".to_string())),
        )
        .expect("disabled memory should return an unavailable report");
        assert_eq!(
            report.status,
            crate::opensymphony_gateway_schema::code_graph::CodeIndexStatus::Unavailable
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("memory is disabled")));
        assert!(!config.index_path.exists());

        config.enabled = true;
        config.code_intel.enabled = false;
        let report = index_code_repository_at(
            &config,
            "disabled-code-intel-repo",
            Some(("develop".to_string(), "commit".to_string())),
        )
        .expect("disabled code intelligence should return unavailable");
        assert_eq!(
            report.status,
            crate::opensymphony_gateway_schema::code_graph::CodeIndexStatus::Unavailable
        );
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("code intelligence is disabled")));
        assert!(!config.index_path.exists());

        config.code_intel.enabled = true;
        config.code_intel.ast.enabled = false;
        let report = index_code_repository_at(
            &config,
            "disabled-ast-repo",
            Some(("develop".to_string(), "commit".to_string())),
        )
        .expect("disabled AST code intelligence should return unavailable");
        assert_eq!(
            report.status,
            crate::opensymphony_gateway_schema::code_graph::CodeIndexStatus::Unavailable
        );
        assert!(!config.index_path.exists());
    }

    #[test]
    fn target_branch_switch_stales_rows_from_the_previous_branch() {
        let repo = TempDir::new().expect("repository tempdir");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `main`\n",
        )
        .expect("workflow marker");
        fs::write(repo.path().join("old.rs"), "pub fn old() {}\n").expect("old source");
        git(repo.path(), &["init", "-b", "main"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test User"]);
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "main baseline"]);
        git(repo.path(), &["switch", "-c", "develop"]);
        fs::remove_file(repo.path().join("old.rs")).expect("remove old source");
        fs::write(repo.path().join("new.rs"), "pub fn new() {}\n").expect("new source");
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-m", "develop baseline"]);

        let config = MemoryConfig::load(repo.path(), None).expect("memory config");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `develop`\n",
        )
        .expect("develop target marker");
        index_code_repository(&config, "branch-switch-repo").expect("develop index");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "Target branch: `main`\n",
        )
        .expect("main target marker");
        let report = index_code_repository(&config, "branch-switch-repo").expect("main index");
        assert!(report.stale_rows > 0);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let old_current: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["branch-switch-repo", "new.rs"],
                |row| row.get(0),
            )
            .expect("old branch document count should be readable");
        let new_current: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                duckdb::params!["branch-switch-repo", "old.rs"],
                |row| row.get(0),
            )
            .expect("new branch document count should be readable");
        assert_eq!(old_current, 0);
        assert_eq!(new_current, 1);
    }

    #[test]
    fn code_index_edge_input_keeps_the_final_hint_character() {
        let capture = CaptureRecord {
            query_name: "rust.references".to_string(),
            capture_name: "reference.call".to_string(),
            text: "helper".to_string(),
            span: SourceSpan {
                start_byte: 0,
                end_byte: 6,
                start_line: 1,
                start_column: 1,
                end_line: 1,
                end_column: 7,
            },
            rendered_span: "1:1-1:7".to_string(),
        };
        assert_eq!(
            super::code_index_edge_input(&capture, 6)
                .and_then(|edge| edge.target_hint),
            Some("helper".to_string())
        );
    }

    #[test]
    fn legacy_code_symbol_refs_match_only_their_exact_span() {
        assert!(code_symbol_span_matches("10:1-12:2", 10, 1, 12, 2));
        assert!(!code_symbol_span_matches("10:1-12:2", 20, 1, 22, 2));
        assert!(!code_symbol_span_matches("10:1-12:2", 10, 1, 12, 3));
    }

    #[test]
    fn code_citations_require_a_matching_code_deep_link_target() {
        assert!(code_citation_matches_symbol(
            "opensymphony://code/team%2Frepo/symbols/crate%3A%3Arun?depth=2",
            "team/repo",
            "crate::run",
            "src/lib.rs",
        ));
        assert!(code_citation_matches_symbol(
            "opensymphony://code/team%2Frepo/files/src/lib.rs",
            "team/repo",
            "crate::run",
            "src/lib.rs",
        ));
        assert!(!code_citation_matches_symbol(
            "https://example.test/team/repo/src/lib.rs",
            "team/repo",
            "crate::run",
            "src/lib.rs",
        ));
        assert!(!code_citation_matches_symbol(
            "opensymphony://code/other-repo/symbols/crate%3A%3Arun",
            "team/repo",
            "crate::run",
            "src/lib.rs",
        ));
    }

    #[test]
    fn repository_scopes_match_any_repository_and_work_item_scopes_are_chip_eligible() {
        let scopes = vec![
            KnowledgeScope {
                kind: KnowledgeScopeKind::Repository,
                id: "other-repo".to_string(),
                label: None,
            },
            KnowledgeScope {
                kind: KnowledgeScopeKind::Repository,
                id: "team/repo".to_string(),
                label: None,
            },
            KnowledgeScope {
                kind: KnowledgeScopeKind::WorkItem,
                id: "COE-536".to_string(),
                label: None,
            },
        ];
        assert!(repository_scope_matches(&scopes, "team/repo"));
        assert!(!repository_scope_matches(&scopes, "missing-repo"));
        assert!(has_work_item_scope(&scopes));
    }
}
