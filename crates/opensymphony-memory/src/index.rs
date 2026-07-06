const UNDATED_LOG_DATE: &str = "1970-01-01";

fn index_capture_plan(config: &MemoryConfig, plan: &CapturePlan) -> Result<(), MemoryError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let transaction = connection
        .transaction()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
    })?;
    for issue_plan in &plan.selected {
        let issue_key = normalize_issue_key(&issue_plan.issue.identifier);
        let body = read_to_string(&issue_plan.capsule_path)?;
        let labels_json = serde_json::to_string(&issue_plan.issue.labels)?;
        let warnings_json = serde_json::to_string(&issue_plan.warnings)?;
        let empty_json = serde_json::to_string(&Vec::<String>::new())?;
        let freshness = MemoryFreshness::Current;
        transaction
            .execute("DELETE FROM issues WHERE issue_key = ?", params![issue_key])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute(
                "INSERT INTO issues (issue_key, title, state, milestone, labels_json, completion_time, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, concept_type, description, tags_json, scope_refs_json, source_refs_json, links_json, citations_json, freshness, warnings_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    issue_key,
                    issue_title(&issue_plan.issue),
                    issue_plan.issue.state.clone(),
                    issue_plan.issue.milestone.clone(),
                    labels_json,
                    issue_plan
                        .issue
                        .completed_at
                        .or(issue_plan.issue.updated_at)
                        .map(|value| value.to_rfc3339()),
                    "not_archived",
                    issue_plan.capsule_path.to_string_lossy().to_string(),
                    config.visibility.as_str(),
                    issue_plan.source_hash.clone(),
                    archive_blocking_warning_count(&issue_plan.warnings) as i64,
                    "pending",
                    body,
                    Utc::now().to_rfc3339(),
                    format!("issues/{issue_key}"),
                    "issue-capsule",
                    issue_plan.issue.description.clone(),
                    labels_json.clone(),
                    empty_json.clone(),
                    empty_json.clone(),
                    empty_json.clone(),
                    empty_json.clone(),
                    freshness.as_str(),
                    warnings_json,
                ],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;

        transaction
            .execute(
                "DELETE FROM issue_areas WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for area in &issue_plan.areas {
            transaction
                .execute(
                    "INSERT INTO issue_areas (issue_key, area) VALUES (?, ?)",
                    params![issue_key, area],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }

        transaction
            .execute(
                "DELETE FROM pull_requests WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute(
                "DELETE FROM changed_files WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute("DELETE FROM checks WHERE issue_key = ?", params![issue_key])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute(
                "DELETE FROM reviews WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;

        for pr in &issue_plan.prs {
            transaction
                .execute(
                    "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        issue_key,
                        pr.number as i64,
                        pr.title.clone(),
                        pr.url.clone(),
                        pr.branch.clone(),
                        pr.merge_sha.clone(),
                        pr.merged_at.map(|value| value.to_rfc3339()),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            for file in &pr.changed_files {
                transaction
                    .execute(
                        "INSERT INTO changed_files (issue_key, pr_number, file_path, change_kind) VALUES (?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            file.path.to_string_lossy().to_string(),
                            file.change_kind.clone(),
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                })?;
            }
            for check in &pr.checks {
                transaction
                    .execute(
                        "INSERT INTO checks (issue_key, pr_number, name, conclusion, completed_at) VALUES (?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            check.name.clone(),
                            check.conclusion.clone(),
                            check.completed_at.map(|value| value.to_rfc3339()),
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                })?;
            }
            for review in &pr.reviews {
                transaction
                    .execute(
                        "INSERT INTO reviews (issue_key, pr_number, reviewer, state, submitted_at, disposition) VALUES (?, ?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            review.reviewer.clone(),
                            review.state.clone(),
                            review.submitted_at.map(|value| value.to_rfc3339()),
                            review.disposition.clone(),
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }

        for area in &issue_plan.areas {
            let area_config = config.area_or_default(area);
            transaction
                .execute("DELETE FROM areas WHERE area = ?", params![area])
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            transaction
                .execute(
                    "INSERT INTO areas (area, display_name, docs_target) VALUES (?, ?, ?)",
                    params![
                        area,
                        area_config.title,
                        area_config.docs_target.to_string_lossy().to_string(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
    }

    transaction
        .commit()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(())
}

fn open_index(config: &MemoryConfig) -> Result<Connection, MemoryError> {
    if let Some(parent) = config.index_path.parent() {
        create_dir_all(parent)?;
    }
    Connection::open(&config.index_path).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })
}

fn open_index_read_only(config: &MemoryConfig) -> Result<Connection, MemoryError> {
    let read_only_config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Connection::open_with_flags(&config.index_path, read_only_config).map_err(|source| {
        MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        }
    })
}

fn migrate_index(connection: &Connection) -> Result<(), duckdb::Error> {
    connection.execute_batch(&format!(
        r#"
CREATE TABLE IF NOT EXISTS schema_version (
  component TEXT PRIMARY KEY,
  version BIGINT NOT NULL,
  updated_at TEXT NOT NULL
);
DELETE FROM schema_version WHERE component = 'memory';
INSERT INTO schema_version (component, version, updated_at)
VALUES ('memory', {MEMORY_SCHEMA_VERSION}, CAST(current_timestamp AS TEXT));
CREATE TABLE IF NOT EXISTS issues (
  issue_key TEXT PRIMARY KEY,
  title TEXT NOT NULL,
  state TEXT,
  milestone TEXT,
  labels_json TEXT NOT NULL,
  completion_time TEXT,
  archive_status TEXT NOT NULL,
  capsule_path TEXT NOT NULL,
  visibility TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  warning_count BIGINT NOT NULL,
  docs_sync_status TEXT NOT NULL,
  body TEXT NOT NULL,
  captured_at TEXT NOT NULL,
  concept_id TEXT NOT NULL DEFAULT '',
  concept_type TEXT NOT NULL DEFAULT 'issue-capsule',
  description TEXT,
  tags_json TEXT NOT NULL DEFAULT '[]',
  scope_refs_json TEXT NOT NULL DEFAULT '[]',
  source_refs_json TEXT NOT NULL DEFAULT '[]',
  links_json TEXT NOT NULL DEFAULT '[]',
  citations_json TEXT NOT NULL DEFAULT '[]',
  freshness TEXT NOT NULL DEFAULT 'unknown',
  warnings_json TEXT NOT NULL DEFAULT '[]'
);
ALTER TABLE issues ADD COLUMN IF NOT EXISTS concept_id TEXT DEFAULT '';
UPDATE issues SET concept_id = '' WHERE concept_id IS NULL;
ALTER TABLE issues ALTER COLUMN concept_id SET DEFAULT '';
ALTER TABLE issues ALTER COLUMN concept_id SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS concept_type TEXT DEFAULT 'issue-capsule';
UPDATE issues SET concept_type = 'issue-capsule' WHERE concept_type IS NULL;
ALTER TABLE issues ALTER COLUMN concept_type SET DEFAULT 'issue-capsule';
ALTER TABLE issues ALTER COLUMN concept_type SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS description TEXT;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS tags_json TEXT DEFAULT '[]';
UPDATE issues SET tags_json = '[]' WHERE tags_json IS NULL;
ALTER TABLE issues ALTER COLUMN tags_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN tags_json SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS scope_refs_json TEXT DEFAULT '[]';
UPDATE issues SET scope_refs_json = '[]' WHERE scope_refs_json IS NULL;
ALTER TABLE issues ALTER COLUMN scope_refs_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN scope_refs_json SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS source_refs_json TEXT DEFAULT '[]';
UPDATE issues SET source_refs_json = '[]' WHERE source_refs_json IS NULL;
ALTER TABLE issues ALTER COLUMN source_refs_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN source_refs_json SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS links_json TEXT DEFAULT '[]';
UPDATE issues SET links_json = '[]' WHERE links_json IS NULL;
ALTER TABLE issues ALTER COLUMN links_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN links_json SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS citations_json TEXT DEFAULT '[]';
UPDATE issues SET citations_json = '[]' WHERE citations_json IS NULL;
ALTER TABLE issues ALTER COLUMN citations_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN citations_json SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS freshness TEXT DEFAULT 'unknown';
UPDATE issues SET freshness = 'unknown' WHERE freshness IS NULL;
ALTER TABLE issues ALTER COLUMN freshness SET DEFAULT 'unknown';
ALTER TABLE issues ALTER COLUMN freshness SET NOT NULL;
ALTER TABLE issues ADD COLUMN IF NOT EXISTS warnings_json TEXT DEFAULT '[]';
UPDATE issues SET warnings_json = '[]' WHERE warnings_json IS NULL;
ALTER TABLE issues ALTER COLUMN warnings_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN warnings_json SET NOT NULL;
CREATE TABLE IF NOT EXISTS pull_requests (
  issue_key TEXT NOT NULL,
  number BIGINT NOT NULL,
  title TEXT NOT NULL,
  url TEXT,
  branch TEXT,
  merge_sha TEXT,
  merged_at TEXT
);
CREATE TABLE IF NOT EXISTS changed_files (
  issue_key TEXT NOT NULL,
  pr_number BIGINT NOT NULL,
  file_path TEXT NOT NULL,
  change_kind TEXT
);
CREATE TABLE IF NOT EXISTS checks (
  issue_key TEXT NOT NULL,
  pr_number BIGINT NOT NULL,
  name TEXT NOT NULL,
  conclusion TEXT,
  completed_at TEXT
);
CREATE TABLE IF NOT EXISTS reviews (
  issue_key TEXT NOT NULL,
  pr_number BIGINT NOT NULL,
  reviewer TEXT,
  state TEXT,
  submitted_at TEXT,
  disposition TEXT
);
CREATE TABLE IF NOT EXISTS areas (
  area TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  docs_target TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS issue_areas (
  issue_key TEXT NOT NULL,
  area TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS doc_sync_runs (
  run_id TEXT PRIMARY KEY,
  selected_issues_json TEXT NOT NULL,
  target_docs_json TEXT NOT NULL,
  generated_at TEXT NOT NULL,
  status TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS doc_memory_links (
  topic_doc TEXT NOT NULL,
  issue_key TEXT NOT NULL,
  visibility TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS code_documents (
  repo_id TEXT NOT NULL,
  commit_sha TEXT,
  worktree_dirty BOOLEAN NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  parser_id TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  byte_len BIGINT NOT NULL,
  line_count BIGINT NOT NULL,
  indexed_at TEXT NOT NULL,
  freshness TEXT NOT NULL,
  PRIMARY KEY (repo_id, path, content_sha256, parser_version, query_pack_version)
);
CREATE TABLE IF NOT EXISTS code_symbols (
  symbol_id TEXT PRIMARY KEY,
  symbol_key TEXT NOT NULL,
  repo_id TEXT NOT NULL,
  commit_sha TEXT,
  worktree_dirty BOOLEAN NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  kind TEXT NOT NULL,
  name TEXT NOT NULL,
  container_symbol_id TEXT,
  signature TEXT,
  start_line BIGINT NOT NULL,
  start_col BIGINT NOT NULL,
  end_line BIGINT NOT NULL,
  end_col BIGINT NOT NULL,
  start_byte BIGINT NOT NULL,
  end_byte BIGINT NOT NULL,
  selection_start_line BIGINT NOT NULL,
  selection_end_line BIGINT NOT NULL,
  content_sha256 TEXT NOT NULL,
  snippet_sha256 TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  indexed_at TEXT NOT NULL,
  freshness TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS code_edges (
  edge_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  commit_sha TEXT,
  worktree_dirty BOOLEAN NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  edge_kind TEXT NOT NULL,
  source_symbol_id TEXT,
  source_symbol_key TEXT,
  target_symbol_id TEXT,
  target_symbol_key TEXT,
  target_hint TEXT,
  confidence TEXT NOT NULL,
  start_line BIGINT NOT NULL,
  start_col BIGINT NOT NULL,
  end_line BIGINT NOT NULL,
  end_col BIGINT NOT NULL,
  start_byte BIGINT NOT NULL,
  end_byte BIGINT NOT NULL,
  content_sha256 TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  indexed_at TEXT NOT NULL,
  freshness TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS code_diagnostics (
  diagnostic_id TEXT PRIMARY KEY,
  repo_id TEXT NOT NULL,
  commit_sha TEXT,
  worktree_dirty BOOLEAN NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  kind TEXT NOT NULL,
  severity TEXT NOT NULL,
  message TEXT NOT NULL,
  start_line BIGINT NOT NULL,
  start_col BIGINT NOT NULL,
  end_line BIGINT NOT NULL,
  end_col BIGINT NOT NULL,
  start_byte BIGINT NOT NULL,
  end_byte BIGINT NOT NULL,
  content_sha256 TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  indexed_at TEXT NOT NULL,
  freshness TEXT NOT NULL
);
ALTER TABLE code_symbols ADD COLUMN IF NOT EXISTS worktree_dirty BOOLEAN DEFAULT false;
UPDATE code_symbols SET worktree_dirty = false WHERE worktree_dirty IS NULL;
ALTER TABLE code_symbols ADD COLUMN IF NOT EXISTS symbol_key TEXT DEFAULT '';
ALTER TABLE code_symbols ADD COLUMN IF NOT EXISTS parser_version TEXT DEFAULT '';
UPDATE code_symbols SET parser_version = '' WHERE parser_version IS NULL;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS worktree_dirty BOOLEAN DEFAULT false;
UPDATE code_edges SET worktree_dirty = false WHERE worktree_dirty IS NULL;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS source_symbol_key TEXT;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS target_symbol_key TEXT;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS start_col BIGINT DEFAULT 0;
UPDATE code_edges SET start_col = 0 WHERE start_col IS NULL;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS end_col BIGINT DEFAULT 0;
UPDATE code_edges SET end_col = 0 WHERE end_col IS NULL;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS start_byte BIGINT DEFAULT 0;
UPDATE code_edges SET start_byte = 0 WHERE start_byte IS NULL;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS end_byte BIGINT DEFAULT 0;
UPDATE code_edges SET end_byte = 0 WHERE end_byte IS NULL;
ALTER TABLE code_edges ADD COLUMN IF NOT EXISTS parser_version TEXT DEFAULT '';
UPDATE code_edges SET parser_version = '' WHERE parser_version IS NULL;
ALTER TABLE code_diagnostics ADD COLUMN IF NOT EXISTS worktree_dirty BOOLEAN DEFAULT false;
UPDATE code_diagnostics SET worktree_dirty = false WHERE worktree_dirty IS NULL;
ALTER TABLE code_diagnostics ADD COLUMN IF NOT EXISTS start_col BIGINT DEFAULT 0;
UPDATE code_diagnostics SET start_col = 0 WHERE start_col IS NULL;
ALTER TABLE code_diagnostics ADD COLUMN IF NOT EXISTS end_col BIGINT DEFAULT 0;
UPDATE code_diagnostics SET end_col = 0 WHERE end_col IS NULL;
ALTER TABLE code_diagnostics ADD COLUMN IF NOT EXISTS start_byte BIGINT DEFAULT 0;
UPDATE code_diagnostics SET start_byte = 0 WHERE start_byte IS NULL;
ALTER TABLE code_diagnostics ADD COLUMN IF NOT EXISTS end_byte BIGINT DEFAULT 0;
UPDATE code_diagnostics SET end_byte = 0 WHERE end_byte IS NULL;
ALTER TABLE code_diagnostics ADD COLUMN IF NOT EXISTS parser_version TEXT DEFAULT '';
UPDATE code_diagnostics SET parser_version = '' WHERE parser_version IS NULL;
CREATE INDEX IF NOT EXISTS idx_code_symbols_name ON code_symbols(name);
CREATE INDEX IF NOT EXISTS idx_code_symbols_key ON code_symbols(symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_symbols_path ON code_symbols(path);
CREATE INDEX IF NOT EXISTS idx_code_symbols_kind ON code_symbols(kind);
CREATE INDEX IF NOT EXISTS idx_code_edges_source ON code_edges(source_symbol_id);
CREATE INDEX IF NOT EXISTS idx_code_edges_target ON code_edges(target_symbol_id);
CREATE INDEX IF NOT EXISTS idx_code_edges_source_key ON code_edges(source_symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_edges_target_key ON code_edges(target_symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_diagnostics_path ON code_diagnostics(path);
"#,
    ))
}

pub fn persist_code_intel_documents(
    config: &MemoryConfig,
    batch: CodeIntelPersistBatch,
) -> Result<CodeIntelPersistReport, MemoryError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let transaction = connection
        .transaction()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let indexed_at = Utc::now().to_rfc3339();
    let mut report = CodeIntelPersistReport {
        parsed_files: batch.documents.len(),
        persisted_documents: 0,
        persisted_symbols: 0,
        persisted_edges: 0,
        persisted_diagnostics: 0,
        stale_rows: 0,
        skipped_files: Vec::new(),
        diagnostics: Vec::new(),
    };

    for document in batch.documents {
        let path = document.path.to_string_lossy().to_string();
        report.stale_rows += stale_code_rows(
            &transaction,
            &CodeFreshnessKey {
                repo_id: &batch.repo_id,
                path: &path,
                content_sha256: &document.content_sha256,
                parser_version: &document.parser_version,
                query_pack_version: &document.query_pack_version,
            },
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;

        transaction
            .execute(
                "INSERT OR REPLACE INTO code_documents (repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_id, parser_version, query_pack_version, byte_len, line_count, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    batch.repo_id,
                    batch.commit_sha,
                    batch.worktree_dirty,
                    path,
                    document.language,
                    document.content_sha256,
                    document.parser_id,
                    document.parser_version,
                    document.query_pack_version,
                    document.byte_len as i64,
                    document.line_count as i64,
                    indexed_at,
                    MemoryFreshness::Current.as_str(),
                ],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        report.persisted_documents += 1;

        let prepared_symbols = prepare_code_symbols(&batch.repo_id, &path, &document);

        for prepared in &prepared_symbols {
            let symbol = prepared.symbol;
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_symbols (symbol_id, symbol_key, repo_id, commit_sha, worktree_dirty, path, language, kind, name, container_symbol_id, signature, start_line, start_col, end_line, end_col, start_byte, end_byte, selection_start_line, selection_end_line, content_sha256, snippet_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        prepared.symbol_id,
                        prepared.symbol_key,
                        batch.repo_id,
                        batch.commit_sha,
                        batch.worktree_dirty,
                        path,
                        document.language,
                        symbol.kind,
                        symbol.name,
                        prepared.container_symbol_id,
                        symbol.signature,
                        symbol.start_line as i64,
                        symbol.start_col as i64,
                        symbol.end_line as i64,
                        symbol.end_col as i64,
                        symbol.start_byte as i64,
                        symbol.end_byte as i64,
                        symbol.selection_start_line as i64,
                        symbol.selection_end_line as i64,
                        document.content_sha256,
                        symbol.snippet_sha256,
                        document.parser_version,
                        document.query_pack_version,
                        indexed_at,
                        MemoryFreshness::Current.as_str(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            report.persisted_symbols += 1;
        }

        for edge in &document.edges {
            let resolved = resolve_code_edge(edge, &prepared_symbols);
            let edge_id = code_row_id(&[
                &batch.repo_id,
                &path,
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
            ]);
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_edges (edge_id, repo_id, commit_sha, worktree_dirty, path, language, edge_kind, source_symbol_id, source_symbol_key, target_symbol_id, target_symbol_key, target_hint, confidence, start_line, start_col, end_line, end_col, start_byte, end_byte, content_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        edge_id,
                        batch.repo_id,
                        batch.commit_sha,
                        batch.worktree_dirty,
                        path,
                        document.language,
                        edge.edge_kind,
                        resolved.source_symbol_id,
                        resolved.source_symbol_key,
                        resolved.target_symbol_id,
                        resolved.target_symbol_key,
                        edge.target_hint,
                        normalize_edge_confidence(&edge.confidence, resolved.target_resolved),
                        edge.start_line as i64,
                        edge.start_col as i64,
                        edge.end_line as i64,
                        edge.end_col as i64,
                        edge.start_byte as i64,
                        edge.end_byte as i64,
                        document.content_sha256,
                        document.parser_version,
                        document.query_pack_version,
                        indexed_at,
                        MemoryFreshness::Current.as_str(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            report.persisted_edges += 1;
        }

        for diagnostic in &document.diagnostics {
            let diagnostic_id = code_row_id(&[
                &batch.repo_id,
                &path,
                &document.content_sha256,
                &document.parser_version,
                &document.query_pack_version,
                &diagnostic.kind,
                &diagnostic.message,
                &diagnostic.start_line.to_string(),
                &diagnostic.start_col.to_string(),
                &diagnostic.end_line.to_string(),
                &diagnostic.end_col.to_string(),
                &diagnostic.start_byte.to_string(),
                &diagnostic.end_byte.to_string(),
            ]);
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_diagnostics (diagnostic_id, repo_id, commit_sha, worktree_dirty, path, language, kind, severity, message, start_line, start_col, end_line, end_col, start_byte, end_byte, content_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        diagnostic_id,
                        batch.repo_id,
                        batch.commit_sha,
                        batch.worktree_dirty,
                        path,
                        document.language,
                        diagnostic.kind,
                        diagnostic.severity,
                        diagnostic.message,
                        diagnostic.start_line as i64,
                        diagnostic.start_col as i64,
                        diagnostic.end_line as i64,
                        diagnostic.end_col as i64,
                        diagnostic.start_byte as i64,
                        diagnostic.end_byte as i64,
                        document.content_sha256,
                        document.parser_version,
                        document.query_pack_version,
                        indexed_at,
                        MemoryFreshness::Current.as_str(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            report.persisted_diagnostics += 1;
        }
    }

    transaction
        .commit()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(report)
}

struct PreparedCodeSymbol<'a> {
    symbol: &'a CodeIntelSymbolInput,
    symbol_id: String,
    symbol_key: String,
    container_symbol_id: Option<String>,
}

struct ResolvedCodeEdge {
    source_symbol_id: Option<String>,
    source_symbol_key: Option<String>,
    target_symbol_id: Option<String>,
    target_symbol_key: Option<String>,
    target_resolved: bool,
}

fn prepare_code_symbols<'a>(
    repo_id: &str,
    path: &str,
    document: &'a CodeIntelDocumentInput,
) -> Vec<PreparedCodeSymbol<'a>> {
    let mut base_key_counts = BTreeMap::<String, usize>::new();
    let mut prepared = document
        .symbols
        .iter()
        .map(|symbol| {
            let symbol_id = code_row_id(&[
                repo_id,
                path,
                &document.content_sha256,
                &document.parser_version,
                &document.query_pack_version,
                &symbol.kind,
                &symbol.name,
                &symbol.start_line.to_string(),
                &symbol.start_col.to_string(),
                &symbol.end_line.to_string(),
                &symbol.end_col.to_string(),
            ]);
            let base_key = code_row_id(&[
                repo_id,
                path,
                &document.language,
                &symbol.kind,
                &symbol.container_chain.join("\u{1f}"),
                &symbol.name,
            ]);
            let ordinal = base_key_counts.entry(base_key.clone()).or_default();
            *ordinal += 1;
            let symbol_key = if *ordinal == 1 {
                base_key
            } else {
                format!("{base_key}#{ordinal}")
            };
            PreparedCodeSymbol {
                symbol,
                symbol_id,
                symbol_key,
                container_symbol_id: None,
            }
        })
        .collect::<Vec<_>>();

    for index in 0..prepared.len() {
        if let Some(parent_index) = container_symbol_index(&prepared, index) {
            prepared[index].container_symbol_id = Some(prepared[parent_index].symbol_id.clone());
        }
    }

    prepared
}

fn container_symbol_index(symbols: &[PreparedCodeSymbol<'_>], child_index: usize) -> Option<usize> {
    let child = symbols[child_index].symbol;
    let parent_name = child.container_chain.last()?;
    let parent_chain = &child.container_chain[..child.container_chain.len().saturating_sub(1)];
    symbols
        .iter()
        .enumerate()
        .filter(|(candidate_index, candidate)| {
            *candidate_index != child_index
                && candidate.symbol.name == *parent_name
                && candidate.symbol.container_chain == parent_chain
                && symbol_contains_span(candidate.symbol, child.start_byte, child.end_byte)
        })
        .min_by_key(|(_, candidate)| {
            (
                candidate.symbol.end_byte.saturating_sub(candidate.symbol.start_byte),
                std::cmp::Reverse(candidate.symbol.start_byte),
            )
        })
        .map(|(candidate_index, _)| candidate_index)
}

fn resolve_code_edge(
    edge: &CodeIntelEdgeInput,
    symbols: &[PreparedCodeSymbol<'_>],
) -> ResolvedCodeEdge {
    let source = symbols
        .iter()
        .filter(|symbol| symbol_contains_span(symbol.symbol, edge.start_byte, edge.end_byte))
        .min_by_key(|symbol| {
            (
                symbol.symbol.end_byte.saturating_sub(symbol.symbol.start_byte),
                std::cmp::Reverse(symbol.symbol.start_byte),
            )
        });
    let target = edge
        .target_hint
        .as_deref()
        .and_then(edge_target_name)
        .and_then(|name| single_symbol_named(symbols, name));

    ResolvedCodeEdge {
        source_symbol_id: source.map(|symbol| symbol.symbol_id.clone()),
        source_symbol_key: source.map(|symbol| symbol.symbol_key.clone()),
        target_symbol_id: target.map(|symbol| symbol.symbol_id.clone()),
        target_symbol_key: target.map(|symbol| symbol.symbol_key.clone()),
        target_resolved: target.is_some(),
    }
}

fn symbol_contains_span(symbol: &CodeIntelSymbolInput, start_byte: usize, end_byte: usize) -> bool {
    symbol.start_byte <= start_byte && symbol.end_byte >= end_byte
}

fn edge_target_name(target_hint: &str) -> Option<&str> {
    let before_call = target_hint.split_once('(').map_or(target_hint, |(name, _)| name);
    before_call
        .split([':', '.'])
        .filter(|part| !part.is_empty())
        .next_back()
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn single_symbol_named<'a>(
    symbols: &'a [PreparedCodeSymbol<'_>],
    name: &str,
) -> Option<&'a PreparedCodeSymbol<'a>> {
    let mut matches = symbols.iter().filter(|symbol| symbol.symbol.name == name);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn normalize_edge_confidence(input: &str, target_resolved: bool) -> &'static str {
    if target_resolved {
        return "exact";
    }
    let normalized = input.trim().to_ascii_lowercase();
    if normalized.contains("heuristic") {
        "heuristic"
    } else {
        "syntactic"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSymbolRecord {
    pub symbol_id: String,
    pub symbol_key: String,
    pub repo_id: String,
    pub commit_sha: Option<String>,
    pub path: String,
    pub language: String,
    pub kind: String,
    pub name: String,
    pub container_symbol_id: Option<String>,
    pub container_chain: Vec<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub snippet_sha256: String,
    pub freshness: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeEdgeRecord {
    pub edge_id: String,
    pub edge_kind: String,
    pub source_symbol_key: Option<String>,
    pub target_symbol_key: Option<String>,
    pub target_hint: Option<String>,
    pub confidence: String,
    pub unresolved: bool,
    pub path: String,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeNeighborhood {
    pub center: CodeSymbolRecord,
    pub symbols: Vec<CodeSymbolRecord>,
    pub edges: Vec<CodeEdgeRecord>,
    pub max_depth: usize,
    pub max_records: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeSymbolDiffStatus {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSymbolDiff {
    pub symbol_key: String,
    pub status: CodeSymbolDiffStatus,
    pub base: Option<CodeSymbolRecord>,
    pub head: Option<CodeSymbolRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSymbolComparison {
    pub base_revision: String,
    pub head_revision: String,
    pub diffs: Vec<CodeSymbolDiff>,
    pub max_records: usize,
    pub truncated: bool,
}

pub fn code_symbol_detail(
    config: &MemoryConfig,
    symbol_key: &str,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    query_code_symbol_by_key(&connection, symbol_key, true)
}

pub fn code_symbols_containing_span(
    config: &MemoryConfig,
    repo_id: &str,
    path: &str,
    line: usize,
    column: usize,
    limit: usize,
) -> Result<Vec<CodeSymbolRecord>, MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let mut statement = connection
        .prepare(
            "SELECT symbol_id, symbol_key, repo_id, commit_sha, path, language, kind, name, container_symbol_id, start_line, start_col, end_line, end_col, start_byte, end_byte, snippet_sha256, freshness FROM code_symbols WHERE repo_id = ? AND path = ? AND freshness = 'current' ORDER BY start_line, start_col, end_line DESC, end_col DESC, symbol_key",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map(params![repo_id, path], |row| code_symbol_from_row(row))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    rows.into_iter()
        .filter(|symbol| symbol_contains_point(symbol, line, column))
        .take(limit)
        .map(|mut symbol| {
            symbol.container_chain = load_container_chain(&connection, symbol.container_symbol_id.as_deref())?;
            Ok(symbol)
        })
        .collect()
}

pub fn code_symbol_neighborhood(
    config: &MemoryConfig,
    symbol_key: &str,
    max_depth: usize,
    max_records: usize,
) -> Result<Option<CodeNeighborhood>, MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let Some(center) = query_code_symbol_by_key(&connection, symbol_key, true)? else {
        return Ok(None);
    };
    let mut symbols = BTreeMap::from([(center.symbol_key.clone(), center.clone())]);
    let mut edges = BTreeMap::<String, CodeEdgeRecord>::new();
    let mut frontier = BTreeSet::from([center.symbol_key.clone()]);
    let mut truncated = false;

    for _ in 0..max_depth {
        let mut next_frontier = BTreeSet::new();
        for key in &frontier {
            for edge in query_edges_for_symbol_key(&connection, key)? {
                if edges.len() >= max_records {
                    truncated = true;
                    break;
                }
                for adjacent in [
                    edge.source_symbol_key.as_deref(),
                    edge.target_symbol_key.as_deref(),
                ]
                .into_iter()
                .flatten()
                {
                    if !symbols.contains_key(adjacent) {
                        if symbols.len() >= max_records {
                            truncated = true;
                            continue;
                        }
                        if let Some(symbol) = query_code_symbol_by_key(&connection, adjacent, true)? {
                            next_frontier.insert(adjacent.to_string());
                            symbols.insert(adjacent.to_string(), symbol);
                        }
                    }
                }
                edges.insert(edge.edge_id.clone(), edge);
            }
            if truncated {
                break;
            }
        }
        if truncated || next_frontier.is_empty() {
            break;
        }
        frontier = next_frontier;
    }

    Ok(Some(CodeNeighborhood {
        center,
        symbols: symbols.into_values().collect(),
        edges: edges.into_values().collect(),
        max_depth,
        max_records,
        truncated,
    }))
}

pub fn compare_code_symbols(
    config: &MemoryConfig,
    repo_id: &str,
    base_revision: &str,
    head_revision: &str,
    max_records: usize,
) -> Result<CodeSymbolComparison, MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let base = query_symbols_for_revision(&connection, repo_id, base_revision)?;
    let head = query_symbols_for_revision(&connection, repo_id, head_revision)?;
    let mut keys = base.keys().cloned().collect::<BTreeSet<_>>();
    keys.extend(head.keys().cloned());
    let mut diffs = Vec::new();
    let mut truncated = false;

    for key in keys {
        if diffs.len() >= max_records {
            truncated = true;
            break;
        }
        match (base.get(&key), head.get(&key)) {
            (None, Some(head_symbol)) => diffs.push(CodeSymbolDiff {
                symbol_key: key,
                status: CodeSymbolDiffStatus::Added,
                base: None,
                head: Some(head_symbol.clone()),
            }),
            (Some(base_symbol), None) => diffs.push(CodeSymbolDiff {
                symbol_key: key,
                status: CodeSymbolDiffStatus::Removed,
                base: Some(base_symbol.clone()),
                head: None,
            }),
            (Some(base_symbol), Some(head_symbol))
                if base_symbol.snippet_sha256 != head_symbol.snippet_sha256 =>
            {
                diffs.push(CodeSymbolDiff {
                    symbol_key: key,
                    status: CodeSymbolDiffStatus::Modified,
                    base: Some(base_symbol.clone()),
                    head: Some(head_symbol.clone()),
                });
            }
            _ => {}
        }
    }

    Ok(CodeSymbolComparison {
        base_revision: base_revision.to_string(),
        head_revision: head_revision.to_string(),
        diffs,
        max_records,
        truncated,
    })
}

fn query_code_symbol_by_key(
    connection: &Connection,
    symbol_key: &str,
    current_only: bool,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    let sql = if current_only {
        "SELECT symbol_id, symbol_key, repo_id, commit_sha, path, language, kind, name, container_symbol_id, start_line, start_col, end_line, end_col, start_byte, end_byte, snippet_sha256, freshness FROM code_symbols WHERE symbol_key = ? AND freshness = 'current' ORDER BY indexed_at DESC, symbol_id LIMIT 1"
    } else {
        "SELECT symbol_id, symbol_key, repo_id, commit_sha, path, language, kind, name, container_symbol_id, start_line, start_col, end_line, end_col, start_byte, end_byte, snippet_sha256, freshness FROM code_symbols WHERE symbol_key = ? ORDER BY indexed_at DESC, symbol_id LIMIT 1"
    };
    let mut statement = connection
        .prepare(sql)
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    let mut rows = statement
        .query(params![symbol_key])
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    let Some(row) = rows.next().map_err(|source| MemoryError::DuckDb {
        path: PathBuf::from("<memory-index>"),
        source,
    })?
    else {
        return Ok(None);
    };
    let mut symbol = code_symbol_from_row(row).map_err(|source| MemoryError::DuckDb {
        path: PathBuf::from("<memory-index>"),
        source,
    })?;
    symbol.container_chain = load_container_chain(connection, symbol.container_symbol_id.as_deref())?;
    Ok(Some(symbol))
}

fn query_symbols_for_revision(
    connection: &Connection,
    repo_id: &str,
    revision: &str,
) -> Result<BTreeMap<String, CodeSymbolRecord>, MemoryError> {
    let mut statement = connection
        .prepare(
            "SELECT symbol_id, symbol_key, repo_id, commit_sha, path, language, kind, name, container_symbol_id, start_line, start_col, end_line, end_col, start_byte, end_byte, snippet_sha256, freshness FROM code_symbols WHERE repo_id = ? AND commit_sha = ? ORDER BY symbol_key, indexed_at DESC, symbol_id",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    let rows = statement
        .query_map(params![repo_id, revision], |row| code_symbol_from_row(row))
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    let mut symbols = BTreeMap::new();
    for mut symbol in rows {
        if !symbols.contains_key(&symbol.symbol_key) {
            symbol.container_chain = load_container_chain(connection, symbol.container_symbol_id.as_deref())?;
            symbols.insert(symbol.symbol_key.clone(), symbol);
        }
    }
    Ok(symbols)
}

fn query_edges_for_symbol_key(
    connection: &Connection,
    symbol_key: &str,
) -> Result<Vec<CodeEdgeRecord>, MemoryError> {
    let mut statement = connection
        .prepare(
            "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, start_line, start_col, end_line, end_col FROM code_edges WHERE freshness = 'current' AND (source_symbol_key = ? OR target_symbol_key = ?) ORDER BY edge_kind, path, start_line, start_col, edge_id",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    statement
        .query_map(params![symbol_key, symbol_key], |row| code_edge_from_row(row))
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })
}

fn load_container_chain(
    connection: &Connection,
    container_symbol_id: Option<&str>,
) -> Result<Vec<String>, MemoryError> {
    let mut chain = Vec::new();
    let mut next = container_symbol_id.map(str::to_string);
    while let Some(symbol_id) = next {
        let mut statement = connection
            .prepare("SELECT name, container_symbol_id FROM code_symbols WHERE symbol_id = ? LIMIT 1")
            .map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?;
        let mut rows = statement
            .query(params![symbol_id])
            .map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?;
        let Some(row) = rows.next().map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?
        else {
            break;
        };
        chain.push(row.get::<_, String>(0).map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?);
        next = row.get::<_, Option<String>>(1).map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    }
    chain.reverse();
    Ok(chain)
}

fn code_symbol_from_row(row: &duckdb::Row<'_>) -> Result<CodeSymbolRecord, duckdb::Error> {
    Ok(CodeSymbolRecord {
        symbol_id: row.get(0)?,
        symbol_key: row.get(1)?,
        repo_id: row.get(2)?,
        commit_sha: row.get(3)?,
        path: row.get(4)?,
        language: row.get(5)?,
        kind: row.get(6)?,
        name: row.get(7)?,
        container_symbol_id: row.get(8)?,
        container_chain: Vec::new(),
        start_line: row.get::<_, i64>(9)? as usize,
        start_col: row.get::<_, i64>(10)? as usize,
        end_line: row.get::<_, i64>(11)? as usize,
        end_col: row.get::<_, i64>(12)? as usize,
        start_byte: row.get::<_, i64>(13)? as usize,
        end_byte: row.get::<_, i64>(14)? as usize,
        snippet_sha256: row.get(15)?,
        freshness: row.get(16)?,
    })
}

fn code_edge_from_row(row: &duckdb::Row<'_>) -> Result<CodeEdgeRecord, duckdb::Error> {
    let target_symbol_key = row.get::<_, Option<String>>(3)?;
    Ok(CodeEdgeRecord {
        edge_id: row.get(0)?,
        edge_kind: row.get(1)?,
        source_symbol_key: row.get(2)?,
        unresolved: target_symbol_key.is_none(),
        target_symbol_key,
        target_hint: row.get(4)?,
        confidence: row.get(5)?,
        path: row.get(6)?,
        start_line: row.get::<_, i64>(7)? as usize,
        start_col: row.get::<_, i64>(8)? as usize,
        end_line: row.get::<_, i64>(9)? as usize,
        end_col: row.get::<_, i64>(10)? as usize,
    })
}

fn symbol_contains_point(symbol: &CodeSymbolRecord, line: usize, column: usize) -> bool {
    (symbol.start_line < line || (symbol.start_line == line && symbol.start_col <= column))
        && (symbol.end_line > line || (symbol.end_line == line && symbol.end_col >= column))
}

struct CodeFreshnessKey<'a> {
    repo_id: &'a str,
    path: &'a str,
    content_sha256: &'a str,
    parser_version: &'a str,
    query_pack_version: &'a str,
}

fn stale_code_rows(
    connection: &Connection,
    key: &CodeFreshnessKey<'_>,
) -> Result<usize, duckdb::Error> {
    let mut stale_rows = 0;
    stale_rows += connection.execute(
        "UPDATE code_documents SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current' AND NOT (content_sha256 = ? AND parser_version = ? AND query_pack_version = ?)",
        params![key.repo_id, key.path, key.content_sha256, key.parser_version, key.query_pack_version],
    )?;
    stale_rows += connection.execute(
        "UPDATE code_symbols SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current' AND NOT (content_sha256 = ? AND parser_version = ? AND query_pack_version = ?)",
        params![key.repo_id, key.path, key.content_sha256, key.parser_version, key.query_pack_version],
    )?;
    stale_rows += connection.execute(
        "UPDATE code_edges SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current' AND NOT (content_sha256 = ? AND parser_version = ? AND query_pack_version = ?)",
        params![key.repo_id, key.path, key.content_sha256, key.parser_version, key.query_pack_version],
    )?;
    stale_rows += connection.execute(
        "UPDATE code_diagnostics SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current' AND NOT (content_sha256 = ? AND parser_version = ? AND query_pack_version = ?)",
        params![key.repo_id, key.path, key.content_sha256, key.parser_version, key.query_pack_version],
    )?;
    Ok(stale_rows)
}

fn code_row_id(parts: &[&str]) -> String {
    sha256_hex(&parts.join("\u{1f}"))
}

fn load_indexed_issues(config: &MemoryConfig) -> Result<Vec<IndexedIssue>, MemoryError> {
    if !config.index_path.exists() {
        return Ok(Vec::new());
    }
    let connection = open_index_read_only(config)?;

    let mut statement = connection
        .prepare(
            "SELECT issue_key, title, state, milestone, labels_json, capsule_path, visibility, source_hash, warning_count, docs_sync_status, completion_time, captured_at, body, concept_id, concept_type, description, tags_json, scope_refs_json, source_refs_json, links_json, citations_json, freshness, warnings_json FROM issues ORDER BY issue_key",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map([], |row| {
            let labels_json: String = row.get(4)?;
            let tags_json: String = row.get(16)?;
            let scope_refs_json: String = row.get(17)?;
            let source_refs_json: String = row.get(18)?;
            let links_json: String = row.get(19)?;
            let citations_json: String = row.get(20)?;
            let warnings_json: String = row.get(22)?;
            Ok(IndexedIssue {
                issue_key: row.get(0)?,
                concept_id: row.get(13)?,
                concept_type: row.get(14)?,
                title: row.get(1)?,
                description: row.get(15)?,
                state: row.get(2)?,
                milestone: row.get(3)?,
                labels: serde_json::from_str::<Vec<String>>(&labels_json).unwrap_or_default(),
                tags: serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default(),
                areas: Vec::new(),
                capsule_path: PathBuf::from(row.get::<_, String>(5)?),
                visibility: match row.get::<_, String>(6)?.as_str() {
                    "public" => MemoryVisibility::Public,
                    _ => MemoryVisibility::Private,
                },
                source_hash: row.get(7)?,
                warning_count: row.get::<_, i64>(8)? as usize,
                docs_sync_status: row.get(9)?,
                completion_time: row.get(10)?,
                captured_at: row.get(11)?,
                changed_files: Vec::new(),
                scope_refs: serde_json::from_str::<Vec<KnowledgeScope>>(&scope_refs_json)
                    .unwrap_or_default(),
                source_refs: serde_json::from_str::<Vec<MemorySourceRef>>(&source_refs_json)
                    .unwrap_or_default(),
                links: serde_json::from_str::<Vec<OkfLink>>(&links_json).unwrap_or_default(),
                citations: serde_json::from_str::<Vec<OkfCitation>>(&citations_json)
                    .unwrap_or_default(),
                freshness: match row.get::<_, String>(21)?.as_str() {
                    "current" => MemoryFreshness::Current,
                    "stale" => MemoryFreshness::Stale,
                    _ => MemoryFreshness::Unknown,
                },
                warnings: serde_json::from_str::<Vec<String>>(&warnings_json)
                    .unwrap_or_default(),
                body: row.get(12)?,
            })
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;

    let mut issues = Vec::new();
    for row in rows {
        issues.push(row.map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?);
    }
    drop(statement);

    for issue in &mut issues {
        issue.areas = load_issue_areas(&connection, &issue.issue_key).map_err(|source| {
            MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            }
        })?;
        issue.changed_files =
            load_issue_changed_files(&connection, &issue.issue_key).map_err(|source| {
                MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                }
            })?;
    }
    Ok(issues)
}

/// Pull-request evidence grouped by issue key, ordered by PR number. Only
/// the columns persisted in the `pull_requests` table are populated; the
/// nested commit/file/check/review evidence stays empty.
fn load_pull_requests_by_issue(
    config: &MemoryConfig,
) -> Result<BTreeMap<String, Vec<PullRequestEvidence>>, MemoryError> {
    if !config.index_path.exists() {
        return Ok(BTreeMap::new());
    }
    let connection = open_index_read_only(config)?;
    let mut statement = connection
        .prepare(
            "SELECT issue_key, number, title, url, branch, merge_sha, merged_at FROM pull_requests ORDER BY issue_key, number",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map([], |row| {
            let merged_at: Option<String> = row.get(6)?;
            Ok((
                row.get::<_, String>(0)?,
                PullRequestEvidence {
                    number: row.get::<_, i64>(1)?.max(0) as u64,
                    title: row.get(2)?,
                    url: row.get(3)?,
                    branch: row.get(4)?,
                    merge_sha: row.get(5)?,
                    merged_at: merged_at
                        .as_deref()
                        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&Utc)),
                    ..PullRequestEvidence::default()
                },
            ))
        })
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;

    let mut by_issue = BTreeMap::<String, Vec<PullRequestEvidence>>::new();
    for row in rows {
        let (issue_key, pr) = row.map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
        by_issue.entry(issue_key).or_default().push(pr);
    }
    Ok(by_issue)
}

fn load_issue_areas(
    connection: &Connection,
    issue_key: &str,
) -> Result<Vec<String>, duckdb::Error> {
    let mut statement =
        connection.prepare("SELECT area FROM issue_areas WHERE issue_key = ? ORDER BY area")?;
    let rows = statement.query_map(params![issue_key], |row| row.get::<_, String>(0))?;
    let mut areas = Vec::new();
    for row in rows {
        areas.push(row?);
    }
    Ok(areas)
}

fn load_issue_changed_files(
    connection: &Connection,
    issue_key: &str,
) -> Result<Vec<PathBuf>, duckdb::Error> {
    let mut statement = connection
        .prepare("SELECT file_path FROM changed_files WHERE issue_key = ? ORDER BY file_path")?;
    let rows = statement.query_map(params![issue_key], |row| {
        Ok(PathBuf::from(row.get::<_, String>(0)?))
    })?;
    let mut paths = Vec::new();
    for row in rows {
        paths.push(row?);
    }
    Ok(paths)
}

fn find_indexed_issue(
    config: &MemoryConfig,
    issue_key: &str,
) -> Result<Option<IndexedIssue>, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    Ok(load_indexed_issues(config)?
        .into_iter()
        .find(|issue| issue.issue_key == issue_key))
}

fn write_markdown_indexes(config: &MemoryConfig) -> Result<Vec<PathBuf>, MemoryError> {
    create_dir_all(&config.memory_root.join("indexes"))?;
    let issues = load_indexed_issues(config)?;
    let index_path = config.memory_root.join("indexes/index.md");
    let log_path = config.memory_root.join("indexes/log.md");

    let mut index = String::new();
    index.push_str("# OpenSymphony Memory Index\n\n");
    for issue in &issues {
        index.push_str(&format!(
            "- [{}: {}]({}) ({})\n",
            issue.issue_key,
            issue.title,
            path_relative_to(&config.memory_root, &issue.capsule_path),
            issue.areas().join(", ")
        ));
    }
    write_file(&index_path, &index)?;

    let mut log = String::new();
    log.push_str("# OpenSymphony Memory Log\n\n");
    let mut log_entries = issues.iter().collect::<Vec<_>>();
    log_entries.sort_by(|left, right| {
        issue_log_date(right)
            .cmp(&issue_log_date(left))
            .then_with(|| right.issue_key.cmp(&left.issue_key))
    });
    let mut current_date = String::new();
    for issue in log_entries {
        let date = issue_log_date(issue);
        if date != current_date {
            if !current_date.is_empty() {
                log.push('\n');
            }
            log.push_str(&format!("## {date}\n\n"));
            current_date = date;
        }
        log.push_str(&format!(
            "- {}: {} [{}]\n",
            issue.issue_key, issue.title, issue.docs_sync_status
        ));
    }
    write_file(&log_path, &log)?;

    Ok(vec![index_path, log_path])
}

fn issue_log_date(issue: &IndexedIssue) -> String {
    issue
        .completion_time
        .as_deref()
        .and_then(iso_date_prefix)
        .or_else(|| iso_date_prefix(&issue.captured_at))
        .unwrap_or_else(|| UNDATED_LOG_DATE.to_string())
}

fn iso_date_prefix(value: &str) -> Option<String> {
    let candidate = value.get(..10)?;
    NaiveDate::parse_from_str(candidate, "%Y-%m-%d").ok()?;
    Some(candidate.to_string())
}

pub fn refresh_memory_index(config: &MemoryConfig) -> Result<MemoryReindexReport, MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    drop(connection);

    let issues = load_indexed_issues(config)?;
    let issue_count = issues.len();
    let warning_count = issues.iter().map(|issue| issue.warning_count).sum();
    let markdown_indexes = write_markdown_indexes(config)?;
    Ok(MemoryReindexReport {
        issue_count,
        index_path: config.index_path.clone(),
        markdown_indexes,
        warning_count,
    })
}

pub fn refresh_memory_index_from_okf(
    config: &MemoryConfig,
    bundle_root: &Path,
) -> Result<MemoryReindexReport, MemoryError> {
    ensure_repo_contained(&config.repo_root, bundle_root)?;
    let bundle_root = canonicalize_existing_path(bundle_root)?;
    let lint = lint_okf_bundle_with_codes(&bundle_root, false)?;
    let errors = lint
        .findings
        .iter()
        .filter(|finding| finding.severity == LintSeverity::Error)
        .filter(|finding| !is_private_export_leak(finding))
        .map(|finding| {
            let path = finding
                .path
                .as_ref()
                .map(|path| display_path(&config.repo_root, path))
                .unwrap_or_else(|| "bundle".to_string());
            format!("{path}: {}", finding.message)
        })
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(MemoryError::InvalidInput(format!(
            "OKF bundle has error(s): {}",
            errors.join("; ")
        )));
    }
    let mut warnings_by_path = BTreeMap::<PathBuf, Vec<String>>::new();
    for finding in lint
        .findings
        .into_iter()
        .filter(|finding| finding.severity == LintSeverity::Warn)
    {
        if let Some(path) = finding.path
            && path.is_file()
        {
            warnings_by_path
                .entry(path)
                .or_default()
                .push(finding.message);
        }
    }

    let mut files = Vec::new();
    collect_okf_markdown_files(&bundle_root, &bundle_root, &mut files)?;
    let mut rows = Vec::new();
    for path in files {
        let relative = bundle_relative_path(&bundle_root, &path)?;
        let bundle_path = OkfBundlePath::new(relative)?;
        if bundle_path.reserved_file().is_some() {
            continue;
        }
        let contents = read_to_string(&path)?;
        let concept = parse_okf_concept(&bundle_root, &path, &contents)?;
        rows.push(OkfIndexRow::from_concept(
            config,
            path.clone(),
            concept,
            contents,
            warnings_by_path.remove(&path).unwrap_or_default(),
        )?);
    }

    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let transaction = connection
        .transaction()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    for table in [
        "issues",
        "issue_areas",
        "pull_requests",
        "changed_files",
        "checks",
        "reviews",
        "areas",
    ] {
        transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }

    for row in &rows {
        transaction
            .execute(
                "INSERT INTO issues (issue_key, title, state, milestone, labels_json, completion_time, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, concept_type, description, tags_json, scope_refs_json, source_refs_json, links_json, citations_json, freshness, warnings_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    row.issue_key,
                    row.title,
                    row.state,
                    row.milestone,
                    row.labels_json,
                    row.completion_time,
                    "not_archived",
                    row.capsule_path.to_string_lossy().to_string(),
                    row.visibility.as_str(),
                    row.source_hash,
                    row.warning_count as i64,
                    row.docs_sync_status,
                    row.body,
                    row.captured_at,
                    row.concept_id,
                    row.concept_type,
                    row.description,
                    row.tags_json,
                    row.scope_refs_json,
                    row.source_refs_json,
                    row.links_json,
                    row.citations_json,
                    row.freshness.as_str(),
                    row.warnings_json,
                ],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for area in &row.areas {
            let area_config = config.area_or_default(area);
            transaction
                .execute(
                    "INSERT OR REPLACE INTO areas (area, display_name, docs_target) VALUES (?, ?, ?)",
                    params![
                        area,
                        area_config.title,
                        area_config.docs_target.to_string_lossy().to_string(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            transaction
                .execute(
                    "INSERT INTO issue_areas (issue_key, area) VALUES (?, ?)",
                    params![row.issue_key, area],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        for pr in &row.prs {
            transaction
                .execute(
                    "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params![
                        row.issue_key,
                        pr.number as i64,
                        pr.title.clone(),
                        pr.url.clone(),
                        pr.branch.clone(),
                        pr.merge_sha.clone(),
                        pr.merged_at.map(|value| value.to_rfc3339()),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
    }
    transaction
        .commit()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;

    let warning_count = rows.iter().map(|row| row.warning_count).sum();
    let markdown_indexes = if config.markdown_indexes {
        write_markdown_indexes(config)?
    } else {
        Vec::new()
    };
    Ok(MemoryReindexReport {
        issue_count: rows.len(),
        index_path: config.index_path.clone(),
        markdown_indexes,
        warning_count,
    })
}

struct OkfIndexRow {
    issue_key: String,
    concept_id: String,
    concept_type: String,
    title: String,
    state: Option<String>,
    milestone: Option<String>,
    labels_json: String,
    completion_time: Option<String>,
    capsule_path: PathBuf,
    visibility: MemoryVisibility,
    source_hash: String,
    warning_count: usize,
    docs_sync_status: String,
    body: String,
    captured_at: String,
    description: Option<String>,
    tags_json: String,
    scope_refs_json: String,
    source_refs_json: String,
    links_json: String,
    citations_json: String,
    freshness: MemoryFreshness,
    warnings_json: String,
    areas: Vec<String>,
    prs: Vec<PullRequestEvidence>,
}

impl OkfIndexRow {
    fn from_concept(
        config: &MemoryConfig,
        path: PathBuf,
        concept: OkfConcept,
        contents: String,
        warnings: Vec<String>,
    ) -> Result<Self, MemoryError> {
        let metadata = concept.frontmatter.opensymphony.clone();
        let scope_refs = metadata
            .as_ref()
            .map(|metadata| metadata.scope_refs.clone())
            .unwrap_or_default();
        let source_refs = metadata
            .as_ref()
            .map(|metadata| metadata.source_refs.clone())
            .unwrap_or_default();
        let links = okf_index_links(&concept, metadata.as_ref());
        let citations = metadata
            .as_ref()
            .map(|metadata| metadata.citations.clone())
            .unwrap_or_default();
        let tags = normalize_list(concept.frontmatter.tags.clone());
        let warning_count = archive_blocking_warning_count(&warnings);
        let warnings_json = serde_json::to_string(&warnings)?;

        Ok(Self {
            issue_key: okf_issue_key(&concept, &scope_refs, &source_refs),
            concept_id: concept.id.clone(),
            concept_type: okf_index_concept_type(&concept.frontmatter.concept_type),
            title: okf_index_title(&concept),
            state: string_extra(&concept.frontmatter, "state"),
            milestone: okf_index_milestone(&concept, &scope_refs),
            labels_json: serde_json::to_string(&tags)?,
            completion_time: concept.frontmatter.timestamp.clone(),
            capsule_path: path,
            visibility: metadata
                .as_ref()
                .and_then(|metadata| metadata.visibility)
                .unwrap_or(config.visibility),
            source_hash: sha256_hex(&contents),
            warning_count,
            docs_sync_status: metadata
                .as_ref()
                .and_then(|metadata| okf_docs_sync_status(metadata.docs_sync.as_ref()))
                .unwrap_or_else(|| "pending".to_string()),
            body: concept.body.clone(),
            captured_at: concept
                .frontmatter
                .timestamp
                .clone()
                .unwrap_or_else(|| Utc::now().to_rfc3339()),
            description: okf_index_description(&concept),
            tags_json: serde_json::to_string(&tags)?,
            scope_refs_json: serde_json::to_string(&scope_refs)?,
            source_refs_json: serde_json::to_string(&source_refs)?,
            links_json: serde_json::to_string(&links)?,
            citations_json: serde_json::to_string(&citations)?,
            freshness: okf_index_freshness(&concept),
            warnings_json,
            areas: okf_index_areas(&scope_refs),
            prs: okf_index_prs(&concept.frontmatter),
        })
    }
}

/// Pull-request evidence carried in an issue capsule's `prs` frontmatter
/// (number/url/merge_sha, per `render_issue_capsule`). OKF reindex rewrites
/// the `pull_requests` table from these so completed rows keep their PR
/// links after `refresh_memory_index_from_okf`, not just after a live
/// capture. Malformed entries are skipped rather than failing the reindex.
fn okf_index_prs(frontmatter: &OkfFrontmatter) -> Vec<PullRequestEvidence> {
    let Some(serde_yaml::Value::Sequence(entries)) = frontmatter.extra.get("prs") else {
        return Vec::new();
    };
    // Parse entries individually so one malformed or newer-format entry
    // (e.g. a renamed `number`) drops only itself, not every valid PR for
    // the issue — reindex must preserve the good rows.
    entries
        .iter()
        .filter_map(|entry| serde_yaml::from_value::<PullRequestEvidence>(entry.clone()).ok())
        .collect()
}

fn okf_issue_key(
    concept: &OkfConcept,
    scope_refs: &[KnowledgeScope],
    source_refs: &[MemorySourceRef],
) -> String {
    scope_refs
        .iter()
        .find(|scope| scope.kind == KnowledgeScopeKind::WorkItem)
        .map(|scope| normalize_issue_key(&scope.id))
        .or_else(|| string_extra(&concept.frontmatter, "issue").map(|issue| normalize_issue_key(&issue)))
        .or_else(|| {
            source_refs
                .iter()
                .find(|source| source.kind == "linear_issue")
                .map(|source| normalize_issue_key(&source.id))
        })
        .unwrap_or_else(|| normalize_issue_key(&concept.id))
}

fn okf_index_concept_type(concept_type: &str) -> String {
    if known_okf_type(concept_type) {
        concept_type.to_string()
    } else {
        "generic-concept".to_string()
    }
}

fn okf_index_title(concept: &OkfConcept) -> String {
    concept
        .frontmatter
        .title
        .as_deref()
        .and_then(normalize_optional)
        .or_else(|| first_heading(&concept.body).map(str::to_string))
        .or_else(|| {
            concept
                .path
                .as_path()
                .file_stem()
                .and_then(OsStr::to_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| concept.id.clone())
}

fn okf_index_description(concept: &OkfConcept) -> Option<String> {
    concept
        .frontmatter
        .description
        .as_deref()
        .and_then(normalize_optional)
        .or_else(|| first_paragraph(&concept.body).map(str::to_string))
}

fn okf_index_milestone(concept: &OkfConcept, scope_refs: &[KnowledgeScope]) -> Option<String> {
    scope_refs
        .iter()
        .find(|scope| scope.kind == KnowledgeScopeKind::Milestone)
        .and_then(|scope| scope.label.clone().or_else(|| Some(scope.id.clone())))
        .or_else(|| string_extra(&concept.frontmatter, "milestone"))
}

fn okf_index_areas(scope_refs: &[KnowledgeScope]) -> Vec<String> {
    let mut areas = scope_refs
        .iter()
        .filter(|scope| scope.kind == KnowledgeScopeKind::Area)
        .filter_map(|scope| normalize_optional(&scope.id))
        .map(|area| slugify(&area))
        .collect::<Vec<_>>();
    areas.sort();
    areas.dedup();
    areas
}

fn okf_index_links(
    concept: &OkfConcept,
    metadata: Option<&OpenSymphonyOkfMetadata>,
) -> Vec<OkfLink> {
    let mut links = concept.links.clone();
    if let Some(metadata) = metadata {
        for link in &metadata.links {
            if !links.iter().any(|existing| existing.target == link.target) {
                links.push(link.clone());
            }
        }
    }
    links
}

fn okf_docs_sync_status(value: Option<&serde_yaml::Value>) -> Option<String> {
    let serde_yaml::Value::Mapping(mapping) = value? else {
        return None;
    };
    mapping
        .get(serde_yaml::Value::String("status".to_string()))
        .and_then(value_as_string)
}

fn okf_index_freshness(concept: &OkfConcept) -> MemoryFreshness {
    match string_extra(&concept.frontmatter, "freshness")
        .unwrap_or_default()
        .as_str()
    {
        "current" => MemoryFreshness::Current,
        "stale" => MemoryFreshness::Stale,
        _ => MemoryFreshness::Unknown,
    }
}

pub fn sha256_hex(contents: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(contents.as_bytes());
    let digest = hasher.finalize();
    digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn write_milestone_nodes(
    config: &MemoryConfig,
    plan: &CapturePlan,
) -> Result<Vec<PathBuf>, MemoryError> {
    let milestone_names = plan
        .selected
        .iter()
        .filter_map(|issue| issue.issue.milestone.as_deref())
        .filter_map(normalize_optional)
        .collect::<BTreeSet<_>>();
    if milestone_names.is_empty() {
        return Ok(Vec::new());
    }

    let issues = load_indexed_issues(config)?;
    let milestone_dir = config.memory_root.join("milestones");
    create_dir_all(&milestone_dir)?;
    let mut written = Vec::new();
    for milestone in milestone_names {
        let slug = slugify(&milestone);
        let path = milestone_dir.join(format!("{slug}.md"));
        let mut markdown = String::new();
        markdown.push_str("---\n");
        markdown.push_str("type: milestone-memory-node\n");
        markdown.push_str(&format!("milestone: {}\n", serde_json::to_string(&milestone)?));
        markdown.push_str(&format!("updated_at: {}\n", Utc::now().to_rfc3339()));
        markdown.push_str("---\n\n");
        markdown.push_str(&format!("# {milestone}\n\n"));
        markdown.push_str("## Issues\n\n");
        let milestone_issues = issues
            .iter()
            .filter(|issue| issue.milestone.as_deref() == Some(milestone.as_str()))
            .collect::<Vec<_>>();
        if milestone_issues.is_empty() {
            markdown.push_str("- No captured issues currently reference this milestone.\n");
        } else {
            for issue in milestone_issues {
                markdown.push_str(&format!(
                    "- [[{}|{}: {}]]\n",
                    issue.issue_key, issue.issue_key, issue.title
                ));
            }
        }
        write_file(&path, &markdown)?;
        written.push(path);
    }
    Ok(written)
}

fn select_indexed_issues_for_docs(
    config: &MemoryConfig,
    selection: &IssueSelection,
) -> Result<Vec<IndexedIssue>, MemoryError> {
    let mut issues = load_indexed_issues(config)?;
    let selected_identifiers = selection
        .identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    if !selected_identifiers.is_empty() {
        issues.retain(|issue| selected_identifiers.contains(&issue.issue_key));
    }
    if selection.since_last_sync {
        issues.retain(|issue| issue.docs_sync_status == "pending");
    }
    if let Some(area) = selection.area.as_ref().map(|area| slugify(area)) {
        issues.retain(|issue| issue.areas().contains(&area));
    }
    Ok(issues)
}

fn render_topic_doc(
    config: &MemoryConfig,
    area: &AreaConfig,
    issues: &[IndexedIssue],
    before: Option<&str>,
    with_diagrams: bool,
) -> String {
    let frontmatter = format!(
        "---\ntype: topic-doc\narea: {}\nvisibility: {}\nlast_memory_sync: {}\n---\n\n",
        area.slug,
        area.visibility,
        Utc::now().to_rfc3339()
    );
    let mut managed = String::new();
    managed.push_str(TOPIC_DOC_BEGIN);
    managed.push_str("\n\n");
    managed.push_str("## Current model\n\n");
    managed.push_str(&current_model_from_issues(issues));
    managed.push_str("\n\n## Important invariants\n\n");
    managed.push_str(&invariants_from_issues(issues));
    managed.push_str("\n\n## Operational flow\n\n");
    if with_diagrams {
        managed.push_str(&format!(
            "```mermaid\nflowchart TD\n  memory[\"Captured issue memory\"] --> area[\"{}\"]\n  area --> docs[\"{}\"]\n```\n",
            area.title,
            display_path(&config.repo_root, &area.docs_target)
        ));
    } else {
        managed.push_str("- No generated diagram requested for this sync.\n");
    }
    managed.push_str("\n## Known gotchas\n\n");
    managed.push_str(&gotchas_from_issues(issues));
    managed.push_str("\n\n## Recent changes\n\n");
    for issue in issues {
        managed.push_str(&format!("- {}: {}\n", issue.issue_key, issue.title));
    }
    managed.push_str("\n## Source refs\n\n");
    for issue in issues {
        managed.push_str(&format!("- {}\n", issue.issue_key));
    }
    managed.push('\n');
    managed.push_str(TOPIC_DOC_END);
    managed.push('\n');

    let title = format!("# {}\n\n", area.title);
    match before {
        Some(existing)
            if existing.contains(TOPIC_DOC_BEGIN) && existing.contains(TOPIC_DOC_END) =>
        {
            replace_managed_block(existing, TOPIC_DOC_BEGIN, TOPIC_DOC_END, &managed)
        }
        Some(existing) => {
            let mut output = existing.trim_end().to_string();
            output.push_str("\n\n");
            output.push_str(&managed);
            output
        }
        None => {
            let mut output = frontmatter;
            output.push_str(&title);
            output.push_str(&managed);
            output
        }
    }
}

fn current_model_from_issues(issues: &[IndexedIssue]) -> String {
    let mut lines = Vec::new();
    for issue in issues.iter().take(6) {
        lines.push(format!(
            "- {} contributed: {}",
            issue.issue_key,
            first_section_line(&issue.body, "## Outcome").unwrap_or_else(|| issue.title.clone())
        ));
    }
    if lines.is_empty() {
        "- No captured issue memory selected.".to_string()
    } else {
        lines.join("\n")
    }
}

fn invariants_from_issues(issues: &[IndexedIssue]) -> String {
    let mut lines = Vec::new();
    for issue in issues {
        if issue.body.to_ascii_lowercase().contains("invariant") {
            lines.push(format!(
                "- Recheck invariant notes in {} before changing this area.",
                issue.issue_key
            ));
        }
    }
    if lines.is_empty() {
        "- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.\n- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.".to_string()
    } else {
        lines.join("\n")
    }
}

fn gotchas_from_issues(issues: &[IndexedIssue]) -> String {
    let mut lines = Vec::new();
    for issue in issues {
        if issue.warning_count > 0 {
            lines.push(format!(
                "- {} had capture warnings; verify source evidence before relying on it.",
                issue.issue_key
            ));
        }
    }
    if lines.is_empty() {
        "- No area-specific gotchas were inferred from the selected memory.".to_string()
    } else {
        lines.join("\n")
    }
}

fn mark_docs_synced(config: &MemoryConfig, plan: &DocsSyncPlan) -> Result<(), MemoryError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let transaction = connection
        .transaction()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let run_id = format!("doc-sync-{}", Utc::now().timestamp_millis());
    let target_docs = plan
        .targets
        .iter()
        .map(|target| target.path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    transaction
        .execute(
            "INSERT INTO doc_sync_runs (run_id, selected_issues_json, target_docs_json, generated_at, status) VALUES (?, ?, ?, ?, ?)",
            params![
                run_id,
                serde_json::to_string(&plan.selected_issue_keys)?,
                serde_json::to_string(&target_docs)?,
                Utc::now().to_rfc3339(),
                "written",
            ],
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    for issue_key in &plan.selected_issue_keys {
        transaction
            .execute(
                "UPDATE issues SET docs_sync_status = 'synced' WHERE issue_key = ?",
                params![issue_key],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    for target in &plan.targets {
        transaction
            .execute(
                "DELETE FROM doc_memory_links WHERE topic_doc = ?",
                params![target.path.to_string_lossy().to_string()],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for issue_key in &target.issue_keys {
            transaction
                .execute(
                    "INSERT INTO doc_memory_links (topic_doc, issue_key, visibility) VALUES (?, ?, ?)",
                    params![
                        target.path.to_string_lossy().to_string(),
                        issue_key,
                        target.visibility.as_str(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
    }
    transaction
        .commit()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(())
}

fn render_diff_stat(before: &str, after: &str, path: &Path) -> String {
    if before == after {
        return format!("{} | no changes\n", path.display());
    }
    let operations = line_diff(before, after);
    let added = operations
        .iter()
        .filter(|operation| matches!(operation, DiffOperation::Added(_)))
        .count();
    let removed = operations
        .iter()
        .filter(|operation| matches!(operation, DiffOperation::Removed(_)))
        .count();
    format!(
        "{} | {} -> {} lines, +{} -{}\n",
        path.display(),
        before.lines().count(),
        after.lines().count(),
        added,
        removed
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffOperation<'a> {
    Unchanged(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

fn line_diff<'a>(before: &'a str, after: &'a str) -> Vec<DiffOperation<'a>> {
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();
    let mut lengths = vec![vec![0usize; after_lines.len() + 1]; before_lines.len() + 1];

    for before_index in (0..before_lines.len()).rev() {
        for after_index in (0..after_lines.len()).rev() {
            lengths[before_index][after_index] =
                if before_lines[before_index] == after_lines[after_index] {
                    lengths[before_index + 1][after_index + 1] + 1
                } else {
                    lengths[before_index + 1][after_index]
                        .max(lengths[before_index][after_index + 1])
                };
        }
    }

    let mut operations = Vec::new();
    let mut before_index = 0;
    let mut after_index = 0;
    while before_index < before_lines.len() && after_index < after_lines.len() {
        if before_lines[before_index] == after_lines[after_index] {
            operations.push(DiffOperation::Unchanged(before_lines[before_index]));
            before_index += 1;
            after_index += 1;
        } else if lengths[before_index + 1][after_index]
            >= lengths[before_index][after_index + 1]
        {
            operations.push(DiffOperation::Removed(before_lines[before_index]));
            before_index += 1;
        } else {
            operations.push(DiffOperation::Added(after_lines[after_index]));
            after_index += 1;
        }
    }
    operations.extend(before_lines[before_index..].iter().map(|line| DiffOperation::Removed(line)));
    operations.extend(after_lines[after_index..].iter().map(|line| DiffOperation::Added(line)));
    operations
}

fn all_known_areas(config: &MemoryConfig, issues: &[IndexedIssue]) -> Vec<AreaConfig> {
    let mut slugs = config.areas.keys().cloned().collect::<BTreeSet<_>>();
    for issue in issues {
        for area in issue.areas() {
            slugs.insert(area);
        }
    }
    slugs
        .into_iter()
        .map(|slug| config.area_or_default(&slug))
        .collect()
}

#[cfg(test)]
mod index_tests {
    use super::*;

    #[test]
    fn issue_log_date_uses_stable_sentinel_for_malformed_timestamps() {
        let issue = IndexedIssue {
            issue_key: "COE-999".to_string(),
            concept_id: "issues/COE-999".to_string(),
            concept_type: "issue-capsule".to_string(),
            title: "Malformed timestamps".to_string(),
            description: None,
            state: None,
            milestone: None,
            labels: Vec::new(),
            tags: Vec::new(),
            areas: Vec::new(),
            capsule_path: PathBuf::from(".opensymphony/memory/issues/COE-999.md"),
            visibility: MemoryVisibility::Private,
            source_hash: String::new(),
            warning_count: 0,
            docs_sync_status: "pending".to_string(),
            completion_time: Some("not-a-date".to_string()),
            captured_at: "also-not-a-date".to_string(),
            changed_files: Vec::new(),
            scope_refs: Vec::new(),
            source_refs: Vec::new(),
            links: Vec::new(),
            citations: Vec::new(),
            freshness: MemoryFreshness::Unknown,
            warnings: Vec::new(),
            body: String::new(),
        };

        assert_eq!(issue_log_date(&issue), UNDATED_LOG_DATE);
    }
}
