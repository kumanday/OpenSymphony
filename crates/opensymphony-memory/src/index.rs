const UNDATED_LOG_DATE: &str = "1970-01-01";
const LIVE_CAPTURE_OWNER: &str = "__live_capture__";
const LIVE_CAPTURE_OWNER_PREFIX: &str = "__live_capture__:";

fn live_capture_owner(repository_id: Option<&str>) -> String {
    repository_id
        .map(|repository_id| format!("{LIVE_CAPTURE_OWNER_PREFIX}{repository_id}"))
        .unwrap_or_else(|| LIVE_CAPTURE_OWNER.to_string())
}

fn is_live_capture_owner(owner: &str) -> bool {
    owner == LIVE_CAPTURE_OWNER || owner.starts_with(LIVE_CAPTURE_OWNER_PREFIX)
}

fn live_capture_owner_matches_repository(owner: &str, repository_id: &str) -> bool {
    owner == LIVE_CAPTURE_OWNER || owner == live_capture_owner(Some(repository_id))
}

fn source_owner_repository_id<'a>(
    registered_source_repositories: &'a BTreeMap<String, String>,
    owner: &'a str,
) -> Option<&'a str> {
    registered_source_repositories
        .get(owner)
        .map(String::as_str)
        .or_else(|| owner.strip_prefix(LIVE_CAPTURE_OWNER_PREFIX))
}

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
        let live_owner = live_capture_owner(config.default_repository_id.as_deref());
        let existing = transaction
            .query_row(
                "SELECT scope_refs_json, source_refs_json, source_ids_json FROM issues WHERE issue_key = ?",
                params![issue_key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        let (mut scope_refs, mut source_refs, mut source_ids) = existing
            .map(|(scope_refs_json, source_refs_json, source_ids_json)| {
                (
                    serde_json::from_str::<Vec<KnowledgeScope>>(&scope_refs_json)
                        .unwrap_or_default(),
                    serde_json::from_str::<Vec<MemorySourceRef>>(&source_refs_json)
                        .unwrap_or_default(),
                    serde_json::from_str::<Vec<String>>(&source_ids_json).unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        let concept_id = format!("issues/{issue_key}");
        let previous_source_scopes = {
            let mut statement = transaction
                .prepare(
                    "SELECT source_id, scope_kind, scope_id FROM source_scope_refs WHERE concept_id = ?",
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            statement
                .query_map(params![&concept_id], |row| {
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
                })?
        };
        let live_scope_refs = capture_scope_refs(config, issue_plan);
        let other_source_scopes = previous_source_scopes
            .iter()
            .filter(|(source_id, _, _)| {
                source_id != &live_owner && source_id != LIVE_CAPTURE_OWNER
            })
            .map(|(_, scope_kind, scope_id)| (scope_kind.as_str(), scope_id.as_str()))
            .collect::<Vec<_>>();
        let current_live_scopes = previous_source_scopes
            .iter()
            .filter(|(source_id, _, _)| {
                source_id == &live_owner || source_id == LIVE_CAPTURE_OWNER
            })
            .map(|(_, scope_kind, scope_id)| (scope_kind.as_str(), scope_id.as_str()))
            .collect::<Vec<_>>();
        scope_refs.retain(|scope| {
            let key = (scope_kind_name(&scope.kind), scope.id.as_str());
            !current_live_scopes.contains(&key) || other_source_scopes.contains(&key)
        });
        for scope_ref in &live_scope_refs {
            if !scope_refs.contains(scope_ref) {
                scope_refs.push(scope_ref.clone());
            }
        }
        source_refs.retain(|source_ref| {
            source_ref.registration_source_id.is_some()
                || source_ref.repo_id.as_deref() != config.default_repository_id.as_deref()
        });
        let live_source_refs = issue_plan
            .prs
            .iter()
            .map(|pr| MemorySourceRef {
                kind: "github_pr".to_string(),
                id: pr.number.to_string(),
                url: pr.url.clone(),
                repo_id: config.default_repository_id.clone(),
                symbol_key: None,
                registration_source_id: None,
            })
            .collect::<Vec<_>>();
        for source_ref in live_source_refs {
            if !source_refs.contains(&source_ref) {
                source_refs.push(source_ref);
            }
        }
        let had_legacy_live_owner = source_ids.iter().any(|owner| owner == LIVE_CAPTURE_OWNER);
        source_ids.retain(|owner| owner != &live_owner && owner != LIVE_CAPTURE_OWNER);
        source_ids.push(live_owner.clone());
        let scope_refs_json = serde_json::to_string(&scope_refs)?;
        let source_refs_json = serde_json::to_string(&source_refs)?;
        let source_ids_json = serde_json::to_string(&source_ids)?;
        let freshness = MemoryFreshness::Current;
        transaction
            .execute("DELETE FROM issues WHERE issue_key = ?", params![issue_key])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute(
                "INSERT INTO issues (issue_key, title, state, milestone, labels_json, completion_time, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, concept_type, description, tags_json, scope_refs_json, source_refs_json, source_ids_json, links_json, citations_json, freshness, warnings_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
                    scope_refs_json,
                    source_refs_json,
                    source_ids_json,
                    serde_json::to_string(&Vec::<String>::new())?,
                    serde_json::to_string(&Vec::<String>::new())?,
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
                "DELETE FROM scope_refs WHERE concept_id = ?",
                params![format!("issues/{issue_key}")],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for scope_ref in &scope_refs {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                    params![
                        format!("issues/{issue_key}"),
                        scope_kind_name(&scope_ref.kind),
                        scope_ref.id,
                        scope_ref.label,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        transaction
            .execute(
                "DELETE FROM source_scope_refs WHERE concept_id = ? AND source_id = ?",
                params![&concept_id, live_owner.clone()],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        if had_legacy_live_owner {
            transaction
                .execute(
                    "DELETE FROM source_scope_refs WHERE concept_id = ? AND source_id = ?",
                    params![&concept_id, LIVE_CAPTURE_OWNER],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        for scope_ref in &live_scope_refs {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?, ?)",
                    params![
                        format!("issues/{issue_key}"),
                        live_owner.clone(),
                        scope_kind_name(&scope_ref.kind),
                        scope_ref.id,
                        scope_ref.label,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        for table in [
            "issue_areas",
            "pull_requests",
            "changed_files",
            "checks",
            "reviews",
        ] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE issue_key = ? AND source_id = ?"),
                    params![issue_key, live_owner.clone()],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            if had_legacy_live_owner {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE issue_key = ? AND source_id IS NULL"),
                        params![issue_key],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }
        for area in &issue_plan.areas {
            transaction
                .execute(
                    "INSERT INTO issue_areas (issue_key, area, source_id) VALUES (?, ?, ?)",
                    params![issue_key, area, live_owner.clone()],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }

        for pr in &issue_plan.prs {
            transaction
                .execute(
                    "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at, source_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        issue_key,
                        pr.number as i64,
                        pr.title.clone(),
                        pr.url.clone(),
                        pr.branch.clone(),
                        pr.merge_sha.clone(),
                        pr.merged_at.map(|value| value.to_rfc3339()),
                        live_owner.clone(),
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            for file in &pr.changed_files {
                transaction
                    .execute(
                        "INSERT INTO changed_files (issue_key, pr_number, file_path, change_kind, source_id) VALUES (?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            file.path.to_string_lossy().to_string(),
                            file.change_kind.clone(),
                            live_owner.clone(),
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
                        "INSERT INTO checks (issue_key, pr_number, name, conclusion, completed_at, source_id) VALUES (?, ?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            check.name.clone(),
                            check.conclusion.clone(),
                            check.completed_at.map(|value| value.to_rfc3339()),
                            live_owner.clone(),
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
                        "INSERT INTO reviews (issue_key, pr_number, reviewer, state, submitted_at, disposition, source_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
                        params![
                            issue_key,
                            pr.number as i64,
                            review.reviewer.clone(),
                            review.state.clone(),
                            review.submitted_at.map(|value| value.to_rfc3339()),
                            review.disposition.clone(),
                            live_owner.clone(),
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

fn capture_scope_refs(config: &MemoryConfig, plan: &CaptureIssuePlan) -> Vec<KnowledgeScope> {
    let mut refs = vec![KnowledgeScope {
        kind: KnowledgeScopeKind::WorkItem,
        id: normalize_issue_key(&plan.issue.identifier),
        label: Some(issue_title(&plan.issue)),
    }];
    if let Some(milestone) = plan.issue.milestone_id.as_ref().or(plan.issue.milestone.as_ref()) {
        refs.push(KnowledgeScope {
            kind: KnowledgeScopeKind::Milestone,
            id: milestone.clone(),
            label: plan.issue.milestone.clone(),
        });
    }
    refs.extend(plan.areas.iter().map(|area| KnowledgeScope {
        kind: KnowledgeScopeKind::Area,
        id: area.clone(),
        label: None,
    }));
    if let Some(project_set_id) = config.default_project_set_id.as_deref() {
        refs.push(KnowledgeScope {
            kind: KnowledgeScopeKind::ProjectSet,
            id: project_set_id.to_string(),
            label: None,
        });
    }
    for project_id in [plan.issue.project_id.as_ref(), plan.issue.project_slug.as_ref()]
        .into_iter()
        .flatten()
    {
        if !refs
            .iter()
            .any(|scope| scope.kind == KnowledgeScopeKind::Project && scope.id == *project_id)
        {
            refs.push(KnowledgeScope {
                kind: KnowledgeScopeKind::Project,
                id: project_id.clone(),
                label: None,
            });
        }
    }
    let issue_projects = [plan.issue.project_id.as_ref(), plan.issue.project_slug.as_ref()]
        .into_iter()
        .flatten()
        .collect::<BTreeSet<_>>();
    let routed_repository_id = {
        let candidates = config
            .repository_sources
            .values()
            .filter(|source| {
                !issue_projects.is_empty()
                    && source
                        .project_scope_ids
                        .iter()
                        .any(|project| issue_projects.contains(project))
            })
            .map(|source| source.repository_id.clone())
            .collect::<BTreeSet<_>>();
        (candidates.len() == 1)
            .then(|| candidates.into_iter().next())
            .flatten()
            .or_else(|| config.default_repository_id.clone())
    }
        .or_else(|| {
            (config.repository_sources.len() == 1)
                .then(|| config.repository_sources.keys().next().cloned())
                .flatten()
        });
    if let Some(repository_id) = routed_repository_id.as_deref()
        && !refs
            .iter()
            .any(|scope| scope.kind == KnowledgeScopeKind::Repository)
    {
        refs.push(KnowledgeScope {
            kind: KnowledgeScopeKind::Repository,
            id: repository_id.to_string(),
            label: None,
        });
    }
    refs
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

fn open_existing_index_read_only(config: &MemoryConfig) -> Result<Option<Connection>, MemoryError> {
    if !config.index_path.exists() {
        return Ok(None);
    }
    open_index_read_only(config).map(Some)
}

fn table_has_columns(
    connection: &Connection,
    path: &Path,
    table: &str,
    columns: &[&str],
) -> Result<bool, MemoryError> {
    let mut statement = connection
        .prepare(
            "SELECT column_name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = ?",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: path.to_path_buf(),
            source,
        })?;
    let existing = statement
        .query_map(params![table], |row| row.get::<_, String>(0))
        .map_err(|source| MemoryError::DuckDb {
            path: path.to_path_buf(),
            source,
        })?
        .collect::<Result<BTreeSet<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(columns.iter().all(|column| existing.contains(*column)))
}

fn code_symbols_read_model_ready(connection: &Connection, path: &Path) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_symbols",
        &[
            "symbol_id",
            "symbol_key",
            "repo_id",
            "commit_sha",
            "path",
            "language",
            "kind",
            "name",
            "container_symbol_id",
            "container_chain",
            "signature",
            "start_line",
            "start_col",
            "end_line",
            "end_col",
            "start_byte",
            "end_byte",
            "selection_start_line",
            "selection_end_line",
            "content_sha256",
            "snippet_sha256",
            "parser_version",
            "query_pack_version",
            "freshness",
            "indexed_at",
            "worktree_dirty",
        ],
    )
}

fn code_edges_read_model_ready(connection: &Connection, path: &Path) -> Result<bool, MemoryError> {
    table_has_columns(
        connection,
        path,
        "code_edges",
        &["source_symbol_key", "target_symbol_key", "worktree_dirty"],
    )
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
  source_ids_json TEXT NOT NULL DEFAULT '[]',
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
ALTER TABLE issues ADD COLUMN IF NOT EXISTS source_ids_json TEXT DEFAULT '[]';
UPDATE issues SET source_ids_json = '[]' WHERE source_ids_json IS NULL;
ALTER TABLE issues ALTER COLUMN source_ids_json SET DEFAULT '[]';
ALTER TABLE issues ALTER COLUMN source_ids_json SET NOT NULL;
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
ALTER TABLE pull_requests ADD COLUMN IF NOT EXISTS source_id TEXT;
ALTER TABLE changed_files ADD COLUMN IF NOT EXISTS source_id TEXT;
ALTER TABLE checks ADD COLUMN IF NOT EXISTS source_id TEXT;
ALTER TABLE reviews ADD COLUMN IF NOT EXISTS source_id TEXT;
CREATE TABLE IF NOT EXISTS areas (
  area TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  docs_target TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS issue_areas (
  issue_key TEXT NOT NULL,
  area TEXT NOT NULL
);
ALTER TABLE issue_areas ADD COLUMN IF NOT EXISTS source_id TEXT;
CREATE TABLE IF NOT EXISTS scope_refs (
  concept_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL,
  scope_id TEXT NOT NULL,
  label TEXT,
  PRIMARY KEY (concept_id, scope_kind, scope_id)
);
CREATE INDEX IF NOT EXISTS idx_scope_refs_lookup ON scope_refs(scope_kind, scope_id);
CREATE TABLE IF NOT EXISTS source_scope_refs (
  concept_id TEXT NOT NULL,
  source_id TEXT NOT NULL,
  scope_kind TEXT NOT NULL,
  scope_id TEXT NOT NULL,
  label TEXT,
  PRIMARY KEY (concept_id, source_id, scope_kind, scope_id)
);
CREATE INDEX IF NOT EXISTS idx_source_scope_refs_lookup
  ON source_scope_refs(concept_id, source_id, scope_kind);
CREATE TABLE IF NOT EXISTS registered_memory_sources (
  source_id TEXT PRIMARY KEY,
  repository_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  source_root TEXT NOT NULL,
  status TEXT NOT NULL,
  generation TEXT NOT NULL,
  registered_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_registered_memory_sources_repository
  ON registered_memory_sources(repository_id, commit_sha);
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
CREATE TABLE IF NOT EXISTS code_documents_staging (
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
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
  PRIMARY KEY (repo_id, commit_sha, path, parser_version, query_pack_version)
);
CREATE TABLE IF NOT EXISTS code_document_revisions (
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  worktree_dirty BOOLEAN NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  parser_id TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  indexed_at TEXT NOT NULL,
  freshness TEXT NOT NULL,
  PRIMARY KEY (repo_id, commit_sha, path, parser_version, query_pack_version)
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
  container_chain TEXT NOT NULL DEFAULT '',
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
CREATE TABLE IF NOT EXISTS code_edge_revisions (
  edge_id TEXT NOT NULL,
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
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
  freshness TEXT NOT NULL,
  PRIMARY KEY (repo_id, commit_sha, edge_id)
);
CREATE TABLE IF NOT EXISTS code_skipped_files (
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  worktree_dirty BOOLEAN NOT NULL,
  path TEXT NOT NULL,
  reason TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  indexed_at TEXT NOT NULL,
  freshness TEXT NOT NULL,
  PRIMARY KEY (repo_id, commit_sha, path, content_sha256)
);
CREATE TABLE IF NOT EXISTS code_skipped_files_staging (
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  path TEXT NOT NULL,
  reason TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  indexed_at TEXT NOT NULL,
  PRIMARY KEY (repo_id, commit_sha, path, content_sha256)
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
CREATE TABLE IF NOT EXISTS code_diagnostic_revisions (
  diagnostic_id TEXT NOT NULL,
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
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
  freshness TEXT NOT NULL,
  PRIMARY KEY (repo_id, commit_sha, diagnostic_id)
);
CREATE TABLE IF NOT EXISTS code_index_snapshots (
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  target_branch TEXT NOT NULL,
  status TEXT NOT NULL,
  total_files BIGINT NOT NULL,
  parsed_files BIGINT NOT NULL,
  skipped_files BIGINT NOT NULL,
  deleted_files BIGINT NOT NULL,
  config_fingerprint TEXT NOT NULL DEFAULT '',
  indexed_at TEXT NOT NULL,
  PRIMARY KEY (repo_id, commit_sha)
);
CREATE TABLE IF NOT EXISTS code_snapshot_membership (
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  analyzed BOOLEAN NOT NULL,
  skip_reason TEXT,
  PRIMARY KEY (repo_id, commit_sha, path)
);
CREATE TABLE IF NOT EXISTS code_snapshot_membership_staging (
  run_id TEXT NOT NULL,
  repo_id TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  path TEXT NOT NULL,
  language TEXT NOT NULL,
  content_sha256 TEXT NOT NULL,
  parser_version TEXT NOT NULL,
  query_pack_version TEXT NOT NULL,
  analyzed BOOLEAN NOT NULL,
  skip_reason TEXT,
  PRIMARY KEY (run_id, repo_id, commit_sha, path)
);
ALTER TABLE code_symbols ADD COLUMN IF NOT EXISTS symbol_key TEXT DEFAULT '';
ALTER TABLE code_symbols ADD COLUMN IF NOT EXISTS container_chain TEXT DEFAULT '';
UPDATE code_symbols SET container_chain = '' WHERE container_chain IS NULL;
ALTER TABLE code_symbols ADD COLUMN IF NOT EXISTS parser_version TEXT DEFAULT '';
UPDATE code_symbols SET parser_version = '' WHERE parser_version IS NULL;
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
ALTER TABLE code_index_snapshots ADD COLUMN IF NOT EXISTS config_fingerprint TEXT DEFAULT '';
CREATE INDEX IF NOT EXISTS idx_code_symbols_name ON code_symbols(name);
CREATE INDEX IF NOT EXISTS idx_code_symbols_key ON code_symbols(symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_symbols_path ON code_symbols(path);
CREATE INDEX IF NOT EXISTS idx_code_symbols_kind ON code_symbols(kind);
CREATE INDEX IF NOT EXISTS idx_code_edges_source ON code_edges(source_symbol_id);
CREATE INDEX IF NOT EXISTS idx_code_edges_target ON code_edges(target_symbol_id);
CREATE INDEX IF NOT EXISTS idx_code_edges_source_key ON code_edges(source_symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_edges_target_key ON code_edges(target_symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_edge_revisions_target_key ON code_edge_revisions(target_symbol_key);
CREATE INDEX IF NOT EXISTS idx_code_skipped_files_revision ON code_skipped_files(repo_id, commit_sha, path);
CREATE INDEX IF NOT EXISTS idx_code_diagnostics_path ON code_diagnostics(path);
CREATE INDEX IF NOT EXISTS idx_code_diagnostic_revisions_path ON code_diagnostic_revisions(repo_id, commit_sha, path);
CREATE INDEX IF NOT EXISTS idx_code_snapshot_membership_staging_repo ON code_snapshot_membership_staging(repo_id, run_id);
CREATE INDEX IF NOT EXISTS idx_code_documents_staging_repo ON code_documents_staging(repo_id, commit_sha);
CREATE INDEX IF NOT EXISTS idx_code_skipped_files_staging_repo ON code_skipped_files_staging(repo_id, commit_sha);
"#,
    ))?;
    for table in [
        "code_documents",
        "code_symbols",
        "code_edges",
        "code_diagnostics",
    ] {
        ensure_worktree_dirty_column(connection, table)?;
    }
    Ok(())
}

fn ensure_worktree_dirty_column(connection: &Connection, table: &str) -> Result<(), duckdb::Error> {
    let column_count: i64 = connection.query_row(
        &format!("SELECT count(*) FROM pragma_table_info('{table}') WHERE name = 'worktree_dirty'"),
        [],
        |row| row.get(0),
    )?;
    if column_count == 0 {
        connection.execute(
            &format!("ALTER TABLE {table} ADD COLUMN worktree_dirty BOOLEAN DEFAULT false"),
            [],
        )?;
    }
    connection.execute(
        &format!("UPDATE {table} SET worktree_dirty = false WHERE worktree_dirty IS NULL"),
        [],
    )?;
    Ok(())
}

pub fn persist_code_intel_documents(
    config: &MemoryConfig,
    batch: CodeIntelPersistBatch,
) -> Result<CodeIntelPersistReport, MemoryError> {
    if !config.repository_sources.is_empty()
        && !config.repository_sources.contains_key(&batch.repo_id)
    {
        return Err(MemoryError::InvalidInput(format!(
            "code-intelligence repository `{}` is not a registered canonical repository",
            batch.repo_id
        )));
    }
    persist_code_intel_documents_with_freshness(config, batch, "current", true)
}

pub(crate) fn persist_code_intel_documents_with_freshness(
    config: &MemoryConfig,
    batch: CodeIntelPersistBatch,
    freshness: &str,
    stale_existing: bool,
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
    if freshness == "current"
        && !batch.worktree_dirty
        && !config.repository_sources.is_empty()
        && let Some(commit_sha) = batch.commit_sha.as_deref()
    {
        let registered_commit = match transaction.query_row(
            "SELECT commit_sha FROM registered_memory_sources WHERE repository_id = ? ORDER BY registered_at DESC LIMIT 1",
            params![batch.repo_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(commit) => Some(commit),
            Err(duckdb::Error::QueryReturnedNoRows) => None,
            Err(source) => {
                return Err(MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                });
            }
        };
        if let Some(registered_commit) = registered_commit
            && registered_commit != commit_sha
        {
            return Err(MemoryError::InvalidInput(format!(
                "code-intelligence commit `{commit_sha}` does not match registered source generation `{registered_commit}` for `{}`",
                batch.repo_id
            )));
        }
    }
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
    let worktree_dirty = if batch.worktree_dirty { 1_i64 } else { 0_i64 };
    for document in batch.documents {
        let path = document.path.to_string_lossy().to_string();
        if stale_existing {
            report.stale_rows += stale_code_rows(
                &transaction,
                &CodeFreshnessKey {
                    repo_id: &batch.repo_id,
                    path: &path,
                    commit_sha: batch.commit_sha.as_deref(),
                    content_sha256: &document.content_sha256,
                    parser_version: &document.parser_version,
                    query_pack_version: &document.query_pack_version,
                },
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        }

        if freshness == "staged" {
            let Some(commit_sha) = batch.commit_sha.as_deref() else {
                return Err(MemoryError::InvalidInput(
                    "staged code documents require a commit revision".to_string(),
                ));
            };
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_documents_staging (repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_id, parser_version, query_pack_version, byte_len, line_count, indexed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        batch.repo_id,
                        commit_sha,
                        worktree_dirty,
                        path,
                        document.language,
                        document.content_sha256,
                        document.parser_id,
                        document.parser_version,
                        document.query_pack_version,
                        document.byte_len as i64,
                        document.line_count as i64,
                        indexed_at,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            report.persisted_documents += 1;
        } else {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_documents (repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_id, parser_version, query_pack_version, byte_len, line_count, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        batch.repo_id,
                        batch.commit_sha.clone(),
                        worktree_dirty,
                        path,
                        document.language,
                        document.content_sha256,
                        document.parser_id,
                        document.parser_version,
                        document.query_pack_version,
                        document.byte_len as i64,
                        document.line_count as i64,
                        indexed_at,
                        freshness,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            report.persisted_documents += 1;
        }
        if !batch.worktree_dirty
            && let Some(commit_sha) = batch.commit_sha.as_deref()
        {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO code_document_revisions (repo_id, commit_sha, worktree_dirty, path, language, content_sha256, parser_id, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    params![
                        batch.repo_id,
                        commit_sha,
                        worktree_dirty,
                        path,
                        document.language,
                        document.content_sha256,
                        document.parser_id,
                        document.parser_version,
                        document.query_pack_version,
                        indexed_at,
                        freshness,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }

        let prepared_symbols = prepare_code_symbols(
            &batch.repo_id,
            batch.commit_sha.as_deref(),
            batch.worktree_dirty,
            &path,
            &document,
        );
        for prepared in &prepared_symbols {
            let symbol = prepared.symbol;
            let current_symbol_exists = if freshness == "staged" {
                transaction
                    .query_row(
                        "SELECT 1 FROM code_symbols WHERE symbol_id = ? AND freshness = 'current' LIMIT 1",
                        params![prepared.symbol_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?
                    .is_some()
            } else {
                false
            };
            if !current_symbol_exists {
                transaction
                    .execute(
                        "INSERT OR REPLACE INTO code_symbols (symbol_id, symbol_key, repo_id, commit_sha, worktree_dirty, path, language, kind, name, container_symbol_id, container_chain, signature, start_line, start_col, end_line, end_col, start_byte, end_byte, selection_start_line, selection_end_line, content_sha256, snippet_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            prepared.symbol_id,
                            prepared.symbol_key,
                            batch.repo_id,
                            batch.commit_sha.clone(),
                            worktree_dirty,
                            path,
                            document.language,
                            symbol.kind,
                            symbol.name,
                            prepared.container_symbol_id,
                            symbol.container_chain.join("\u{1f}"),
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
                            freshness,
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
                report.persisted_symbols += 1;
            }
        }

        for edge in &document.edges {
            let resolved = resolve_code_edge(edge, &prepared_symbols);
            let normalized_confidence =
                normalize_edge_confidence(&edge.confidence, resolved.target_resolved);
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
            let current_edge_exists = if freshness == "staged" {
                transaction
                    .query_row(
                        "SELECT 1 FROM code_edges WHERE edge_id = ? AND freshness = 'current' AND NOT worktree_dirty LIMIT 1",
                        params![edge_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?
                    .is_some()
            } else {
                false
            };
            if !current_edge_exists {
                transaction
                    .execute(
                        "INSERT OR REPLACE INTO code_edges (edge_id, repo_id, commit_sha, worktree_dirty, path, language, edge_kind, source_symbol_id, source_symbol_key, target_symbol_id, target_symbol_key, target_hint, confidence, start_line, start_col, end_line, end_col, start_byte, end_byte, content_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            edge_id,
                            batch.repo_id,
                            batch.commit_sha.clone(),
                            worktree_dirty,
                            path,
                            document.language,
                            edge.edge_kind,
                            resolved.source_symbol_id,
                            resolved.source_symbol_key,
                            resolved.target_symbol_id,
                            resolved.target_symbol_key,
                            edge.target_hint,
                            normalized_confidence,
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
                            freshness,
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
            if !batch.worktree_dirty
                && let Some(commit_sha) = batch.commit_sha.as_deref()
            {
                transaction
                    .execute(
                        "INSERT OR REPLACE INTO code_edge_revisions (edge_id, repo_id, commit_sha, worktree_dirty, path, language, edge_kind, source_symbol_id, source_symbol_key, target_symbol_id, target_symbol_key, target_hint, confidence, start_line, start_col, end_line, end_col, start_byte, end_byte, content_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            edge_id,
                            batch.repo_id,
                            commit_sha,
                            worktree_dirty,
                            path,
                            document.language,
                            edge.edge_kind,
                            resolved.source_symbol_id,
                            resolved.source_symbol_key,
                            resolved.target_symbol_id,
                            resolved.target_symbol_key,
                            edge.target_hint,
                            normalized_confidence,
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
                            freshness,
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
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
            let read_model_diagnostic_id = if freshness == "staged" {
                code_row_id(&[
                    &diagnostic_id,
                    batch.commit_sha.as_deref().unwrap_or(""),
                    &indexed_at,
                ])
            } else {
                diagnostic_id.clone()
            };
            transaction
                    .execute(
                        "INSERT OR REPLACE INTO code_diagnostics (diagnostic_id, repo_id, commit_sha, worktree_dirty, path, language, kind, severity, message, start_line, start_col, end_line, end_col, start_byte, end_byte, content_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            &read_model_diagnostic_id,
                            batch.repo_id,
                            batch.commit_sha.clone(),
                            worktree_dirty,
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
                            freshness,
                        ],
                    )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            if !batch.worktree_dirty
                && let Some(commit_sha) = batch.commit_sha.as_deref()
            {
                transaction
                    .execute(
                        "INSERT OR REPLACE INTO code_diagnostic_revisions (diagnostic_id, repo_id, commit_sha, worktree_dirty, path, language, kind, severity, message, start_line, start_col, end_line, end_col, start_byte, end_byte, content_sha256, parser_version, query_pack_version, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                        params![
                            &diagnostic_id,
                            batch.repo_id,
                            commit_sha,
                            worktree_dirty,
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
                            freshness,
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
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

pub fn persist_code_intel_skipped_files(
    config: &MemoryConfig,
    repo_id: &str,
    commit_sha: Option<&str>,
    worktree_dirty: bool,
    skipped_files: &[CodeIntelSkippedFileInput],
) -> Result<usize, MemoryError> {
    if !config.repository_sources.is_empty() && !config.repository_sources.contains_key(repo_id) {
        return Err(MemoryError::InvalidInput(format!(
            "code-intelligence repository `{repo_id}` is not a registered canonical repository"
        )));
    }
    persist_code_intel_skipped_files_with_freshness(
        config,
        repo_id,
        commit_sha,
        worktree_dirty,
        skipped_files,
        "current",
        true,
    )
}

pub(crate) fn persist_code_intel_skipped_files_with_freshness(
    config: &MemoryConfig,
    repo_id: &str,
    commit_sha: Option<&str>,
    worktree_dirty: bool,
    skipped_files: &[CodeIntelSkippedFileInput],
    freshness: &str,
    stale_existing: bool,
) -> Result<usize, MemoryError> {
    let Some(commit_sha) = commit_sha.filter(|_| !worktree_dirty) else {
        return Ok(0);
    };
    if skipped_files.is_empty() {
        return Ok(0);
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
    if freshness == "current" && !config.repository_sources.is_empty() {
        let registered_commit = match transaction.query_row(
            "SELECT commit_sha FROM registered_memory_sources WHERE repository_id = ? ORDER BY registered_at DESC LIMIT 1",
            params![repo_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(commit) => Some(commit),
            Err(duckdb::Error::QueryReturnedNoRows) => None,
            Err(source) => {
                return Err(MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                });
            }
        };
        if let Some(registered_commit) = registered_commit
            && registered_commit != commit_sha
        {
            return Err(MemoryError::InvalidInput(format!(
                "code-intelligence commit `{commit_sha}` does not match registered source generation `{registered_commit}` for `{repo_id}`"
            )));
        }
    }
    let indexed_at = Utc::now().to_rfc3339();
    for skipped in skipped_files {
        let path = skipped.path.to_string_lossy().to_string();
        if stale_existing {
            stale_code_rows_for_skipped_file(&transaction, repo_id, &path).map_err(|source| {
                MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                }
            })?;
            transaction
                .execute(
                    "UPDATE code_skipped_files SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND commit_sha = ? AND freshness = 'current' AND content_sha256 != ?",
                    params![repo_id, path, commit_sha, skipped.content_sha256],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        let table = if freshness == "staged" {
            "code_skipped_files_staging"
        } else {
            "code_skipped_files"
        };
        let query = if freshness == "staged" {
            format!(
                "INSERT OR REPLACE INTO {table} (repo_id, commit_sha, path, reason, content_sha256, indexed_at) VALUES (?, ?, ?, ?, ?, ?)"
            )
        } else {
            format!(
                "INSERT OR REPLACE INTO {table} (repo_id, commit_sha, worktree_dirty, path, reason, content_sha256, indexed_at, freshness) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
        };
        let result = if freshness == "staged" {
            transaction.execute(
                &query,
                params![repo_id, commit_sha, path, skipped.reason, skipped.content_sha256, indexed_at],
            )
        } else {
            transaction.execute(
                &query,
                params![
                    repo_id,
                    commit_sha,
                    0_i64,
                    path,
                    skipped.reason,
                    skipped.content_sha256,
                    indexed_at,
                    freshness,
                ],
            )
        };
        result
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    transaction.commit().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    Ok(skipped_files.len())
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
    commit_sha: Option<&str>,
    worktree_dirty: bool,
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
                commit_sha.unwrap_or(""),
                if worktree_dirty { "dirty" } else { "clean" },
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
    let matches = symbols
        .iter()
        .enumerate()
        .filter(|(candidate_index, candidate)| {
            *candidate_index != child_index
                && candidate.symbol.name == *parent_name
                && candidate.symbol.container_chain == parent_chain
        })
        .collect::<Vec<_>>();
    matches
        .iter()
        .copied()
        .filter(|(_, candidate)| symbol_contains_span(candidate.symbol, child.start_byte, child.end_byte))
        .min_by_key(|(_, candidate)| {
            (
                candidate.symbol.end_byte.saturating_sub(candidate.symbol.start_byte),
                std::cmp::Reverse(candidate.symbol.start_byte),
            )
        })
        .map(|(candidate_index, _)| candidate_index)
        .or_else(|| (matches.len() == 1).then(|| matches[0].0))
        .or_else(|| trait_impl_owner_symbol_index(symbols, child_index))
}

fn trait_impl_owner_symbol_index(
    symbols: &[PreparedCodeSymbol<'_>],
    child_index: usize,
) -> Option<usize> {
    let child = symbols[child_index].symbol;
    if child.container_chain.len() < 2 {
        return None;
    }
    let owner_index = child.container_chain.len().saturating_sub(2);
    let owner_name = child.container_chain.get(owner_index)?;
    let owner_chain = &child.container_chain[..owner_index];
    let matches = symbols
        .iter()
        .enumerate()
        .filter(|(candidate_index, candidate)| {
            *candidate_index != child_index
                && candidate.symbol.name == *owner_name
                && candidate.symbol.container_chain == owner_chain
        })
        .collect::<Vec<_>>();
    (matches.len() == 1).then(|| matches[0].0)
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
    let before_call = target_hint
        .split_once('(')
        .map_or(target_hint, |(name, _)| name)
        .trim();
    if before_call.contains("::") || before_call.contains('.') {
        return None;
    }
    (!before_call.is_empty()).then_some(before_call)
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
    let normalized = input.trim().to_ascii_lowercase();
    if target_resolved && normalized.contains("exact") {
        "exact"
    } else if normalized.contains("heuristic") {
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
    pub signature: Option<String>,
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub selection_start_line: usize,
    pub selection_end_line: usize,
    pub content_sha256: String,
    pub snippet_sha256: String,
    pub parser_version: String,
    pub query_pack_version: String,
    pub freshness: String,
    pub indexed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeEdgeRecord {
    pub edge_id: String,
    pub edge_key: String,
    pub edge_kind: String,
    pub source_symbol_key: Option<String>,
    pub target_symbol_key: Option<String>,
    pub target_hint: Option<String>,
    pub confidence: String,
    pub unresolved: bool,
    pub path: String,
    pub commit_sha: Option<String>,
    pub freshness: String,
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
    pub dropped_nodes: usize,
    pub dropped_edges: usize,
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
    pub dropped_records: usize,
}

const CODE_SYMBOL_SELECT: &str = "symbol_id, symbol_key, repo_id, commit_sha, path, language, kind, name, container_symbol_id, container_chain, signature, start_line, start_col, end_line, end_col, start_byte, end_byte, selection_start_line, selection_end_line, content_sha256, snippet_sha256, parser_version, query_pack_version, freshness, indexed_at";

pub fn code_symbol_detail(
    config: &MemoryConfig,
    symbol_key: &str,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(None);
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)? {
        return Ok(None);
    }
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
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(Vec::new());
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)? {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare(&format!(
            "SELECT {CODE_SYMBOL_SELECT} FROM code_symbols WHERE repo_id = ? AND path = ? AND freshness = 'current' AND symbol_key != '' ORDER BY start_line, start_col, end_line DESC, end_col DESC, symbol_key, indexed_at DESC, symbol_id"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map(params![repo_id, path], code_symbol_from_row)
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let mut seen_keys = BTreeSet::new();
    rows.into_iter()
        .filter(|symbol| symbol_contains_point(symbol, line, column))
        .filter(|symbol| seen_keys.insert(symbol.symbol_key.clone()))
        .take(limit)
        .map(|mut symbol| {
            fill_container_chain(&connection, &mut symbol)?;
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
    code_symbol_neighborhood_with_stale(config, symbol_key, max_depth, max_records, false)
}

fn code_symbol_neighborhood_with_stale(
    config: &MemoryConfig,
    symbol_key: &str,
    max_depth: usize,
    max_records: usize,
    include_stale: bool,
) -> Result<Option<CodeNeighborhood>, MemoryError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(None);
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)?
        || !code_edges_read_model_ready(&connection, &config.index_path)?
    {
        return Ok(None);
    }
    let Some(center) = query_code_symbol_by_key(&connection, symbol_key, !include_stale)? else {
        return Ok(None);
    };
    let mut symbols = BTreeMap::from([(center.symbol_key.clone(), center.clone())]);
    let mut edges = BTreeMap::<String, CodeEdgeRecord>::new();
    let mut frontier = BTreeSet::from([center.symbol_key.clone()]);
    let mut truncated = false;
    let mut dropped_nodes = 0;
    let mut dropped_edges = 0;

    for _ in 0..max_depth {
        let mut next_frontier = BTreeSet::new();
        for key in &frontier {
            for edge in query_edges_for_symbol_key_with_stale(&connection, key, include_stale)? {
                if edges.len() >= max_records {
                    truncated = true;
                    dropped_edges += 1;
                    continue;
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
                            dropped_nodes += 1;
                            continue;
                        }
                        if let Some(symbol) = query_code_symbol_for_edge(
                            &connection,
                            adjacent,
                            &edge,
                            include_stale,
                        )? {
                            next_frontier.insert(adjacent.to_string());
                            symbols.insert(adjacent.to_string(), symbol);
                        }
                    }
                }
                if !edge_endpoints_present(&edge, &symbols) {
                    truncated = true;
                    dropped_edges += 1;
                    continue;
                }
                edges.insert(edge.edge_id.clone(), edge);
            }
        }
        if next_frontier.is_empty() {
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
        dropped_nodes,
        dropped_edges,
    }))
}

fn edge_endpoints_present(edge: &CodeEdgeRecord, symbols: &BTreeMap<String, CodeSymbolRecord>) -> bool {
    let Some(source) = edge.source_symbol_key.as_deref() else {
        return false;
    };
    if !symbols.contains_key(source) {
        return false;
    }
    match edge.target_symbol_key.as_deref() {
        Some(target) => symbols.contains_key(target),
        None => edge.unresolved,
    }
}

pub fn compare_code_symbols(
    config: &MemoryConfig,
    repo_id: &str,
    base_revision: &str,
    head_revision: &str,
    max_records: usize,
) -> Result<CodeSymbolComparison, MemoryError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(CodeSymbolComparison {
            base_revision: base_revision.to_string(),
            head_revision: head_revision.to_string(),
            diffs: Vec::new(),
            max_records,
            truncated: false,
            dropped_records: 0,
        });
    };
    if !code_symbols_read_model_ready(&connection, &config.index_path)? {
        return Ok(CodeSymbolComparison {
            base_revision: base_revision.to_string(),
            head_revision: head_revision.to_string(),
            diffs: Vec::new(),
            max_records,
            truncated: false,
            dropped_records: 0,
        });
    }
    let base = query_symbols_for_revision(config, &connection, repo_id, base_revision)?;
    let head = query_symbols_for_revision(config, &connection, repo_id, head_revision)?;
    let mut keys = base.keys().cloned().collect::<BTreeSet<_>>();
    keys.extend(head.keys().cloned());
    let mut diffs = Vec::new();
    let mut dropped_records = 0;

    for key in keys {
        let diff = match (base.get(&key), head.get(&key)) {
            (None, Some(head_symbol)) => Some(CodeSymbolDiff {
                symbol_key: key,
                status: CodeSymbolDiffStatus::Added,
                base: None,
                head: Some(head_symbol.clone()),
            }),
            (Some(base_symbol), None) => Some(CodeSymbolDiff {
                symbol_key: key,
                status: CodeSymbolDiffStatus::Removed,
                base: Some(base_symbol.clone()),
                head: None,
            }),
            (Some(base_symbol), Some(head_symbol))
                if base_symbol.snippet_sha256 != head_symbol.snippet_sha256 =>
            {
                Some(CodeSymbolDiff {
                    symbol_key: key,
                    status: CodeSymbolDiffStatus::Modified,
                    base: Some(base_symbol.clone()),
                    head: Some(head_symbol.clone()),
                })
            }
            _ => None,
        };
        if let Some(diff) = diff {
            if diffs.len() >= max_records {
                dropped_records += 1;
                continue;
            }
            diffs.push(diff);
        }
    }

    Ok(CodeSymbolComparison {
        base_revision: base_revision.to_string(),
        head_revision: head_revision.to_string(),
        diffs,
        max_records,
        truncated: dropped_records > 0,
        dropped_records,
    })
}

fn query_code_symbol_by_key(
    connection: &Connection,
    symbol_key: &str,
    current_only: bool,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    let freshness = if current_only {
        "freshness = 'current'"
    } else {
        code_freshness_filter(true)
    };
    let sql = format!(
        "SELECT {CODE_SYMBOL_SELECT} FROM code_symbols WHERE symbol_key = ? AND {freshness} ORDER BY CASE WHEN freshness = 'current' THEN 0 ELSE 1 END, indexed_at DESC, symbol_id LIMIT 1"
    );
    let mut statement = connection
        .prepare(&sql)
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
    fill_container_chain(connection, &mut symbol)?;
    Ok(Some(symbol))
}

fn query_code_symbol_for_edge(
    connection: &Connection,
    symbol_key: &str,
    edge: &CodeEdgeRecord,
    include_stale: bool,
) -> Result<Option<CodeSymbolRecord>, MemoryError> {
    if !include_stale {
        return query_code_symbol_by_key(connection, symbol_key, true);
    }
    if let Some(commit_sha) = edge.commit_sha.as_deref() {
        let mut statement = connection
            .prepare(&format!(
                "SELECT {CODE_SYMBOL_SELECT} FROM code_symbols WHERE symbol_key = ? AND commit_sha = ? AND freshness = ? ORDER BY indexed_at DESC, symbol_id LIMIT 1"
            ))
            .map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?;
        let mut rows = statement
            .query(params![symbol_key, commit_sha, &edge.freshness])
            .map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?;
        if let Some(row) = rows.next().map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })? {
            let mut symbol = code_symbol_from_row(row).map_err(|source| MemoryError::DuckDb {
                path: PathBuf::from("<memory-index>"),
                source,
            })?;
            fill_container_chain(connection, &mut symbol)?;
            return Ok(Some(symbol));
        }
        return Ok(None);
    }
    query_code_symbol_by_key(connection, symbol_key, false)
}

fn query_symbols_for_revision(
    config: &MemoryConfig,
    connection: &Connection,
    repo_id: &str,
    revision: &str,
) -> Result<BTreeMap<String, CodeSymbolRecord>, MemoryError> {
    let snapshot_status = code_snapshot_status(connection, config, repo_id, revision)?;
    let membership_ready =
        code_snapshot_membership_read_model_ready(connection, &config.index_path)?
            && matches!(snapshot_status.as_deref(), None | Some("completed"));
    let query = if membership_ready {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN s.worktree_dirty THEN 1 ELSE 0 END FROM code_symbols AS s WHERE s.repo_id = ? AND s.symbol_key != '' AND s.freshness <> 'staged' AND NOT s.worktree_dirty AND (s.commit_sha = ? OR EXISTS (SELECT 1 FROM code_snapshot_membership AS m WHERE m.repo_id = s.repo_id AND m.commit_sha = ? AND m.path = s.path AND m.content_sha256 = s.content_sha256 AND m.parser_version = s.parser_version AND m.query_pack_version = s.query_pack_version AND m.analyzed)) ORDER BY s.symbol_key, s.indexed_at DESC, s.symbol_id"
        )
    } else {
        format!(
            "SELECT {CODE_SYMBOL_SELECT}, CASE WHEN worktree_dirty THEN 1 ELSE 0 END FROM code_symbols WHERE repo_id = ? AND commit_sha = ? AND symbol_key != '' AND freshness <> 'staged' ORDER BY symbol_key, indexed_at DESC, symbol_id"
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
            .query_map(params![repo_id, revision, revision], |row| {
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
            .query_map(params![repo_id, revision], |row| {
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
    Ok(symbols)
}

fn query_edges_for_symbol_key_with_stale(
    connection: &Connection,
    symbol_key: &str,
    include_stale: bool,
) -> Result<Vec<CodeEdgeRecord>, MemoryError> {
    let freshness = code_freshness_filter(include_stale);
    let mut statement = connection
        .prepare(&format!(
            "SELECT edge_id, edge_kind, source_symbol_key, target_symbol_key, target_hint, confidence, path, commit_sha, freshness, start_line, start_col, end_line, end_col FROM code_edges WHERE {freshness} AND (source_symbol_key = ? OR target_symbol_key = ?) ORDER BY edge_kind, path, start_line, start_col, edge_id"
        ))
        .map_err(|source| MemoryError::DuckDb {
            path: PathBuf::from("<memory-index>"),
            source,
        })?;
    statement
        .query_map(params![symbol_key, symbol_key], code_edge_from_row)
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

fn fill_container_chain(
    connection: &Connection,
    symbol: &mut CodeSymbolRecord,
) -> Result<(), MemoryError> {
    if symbol.container_chain.is_empty() {
        symbol.container_chain =
            load_container_chain(connection, symbol.container_symbol_id.as_deref())?;
    }
    Ok(())
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
        container_chain: row
            .get::<_, String>(9)?
            .split('\u{1f}')
            .filter(|part| !part.is_empty())
            .map(str::to_string)
            .collect(),
        signature: row.get(10)?,
        start_line: row.get::<_, i64>(11)? as usize,
        start_col: row.get::<_, i64>(12)? as usize,
        end_line: row.get::<_, i64>(13)? as usize,
        end_col: row.get::<_, i64>(14)? as usize,
        start_byte: row.get::<_, i64>(15)? as usize,
        end_byte: row.get::<_, i64>(16)? as usize,
        selection_start_line: row.get::<_, i64>(17)? as usize,
        selection_end_line: row.get::<_, i64>(18)? as usize,
        content_sha256: row.get(19)?,
        snippet_sha256: row.get(20)?,
        parser_version: row.get(21)?,
        query_pack_version: row.get(22)?,
        freshness: row.get(23)?,
        indexed_at: row.get(24)?,
    })
}

fn code_edge_from_row(row: &duckdb::Row<'_>) -> Result<CodeEdgeRecord, duckdb::Error> {
    let target_symbol_key = row.get::<_, Option<String>>(3)?;
    Ok(CodeEdgeRecord {
        edge_id: row.get(0)?,
        edge_key: String::new(),
        edge_kind: row.get(1)?,
        source_symbol_key: row.get(2)?,
        unresolved: target_symbol_key.is_none(),
        target_symbol_key,
        target_hint: row.get(4)?,
        confidence: row.get(5)?,
        path: row.get(6)?,
        commit_sha: row.get(7)?,
        freshness: row.get(8)?,
        start_line: row.get::<_, i64>(9)? as usize,
        start_col: row.get::<_, i64>(10)? as usize,
        end_line: row.get::<_, i64>(11)? as usize,
        end_col: row.get::<_, i64>(12)? as usize,
    })
}

fn symbol_contains_point(symbol: &CodeSymbolRecord, line: usize, column: usize) -> bool {
    (symbol.start_line < line || (symbol.start_line == line && symbol.start_col <= column))
        && (symbol.end_line > line || (symbol.end_line == line && column < symbol.end_col))
}

struct CodeFreshnessKey<'a> {
    repo_id: &'a str,
    path: &'a str,
    commit_sha: Option<&'a str>,
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
    if let Some(commit_sha) = key.commit_sha {
        stale_rows += connection.execute(
            "UPDATE code_skipped_files SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND commit_sha = ? AND freshness = 'current'",
            params![key.repo_id, key.path, commit_sha],
        )?;
    }
    Ok(stale_rows)
}

fn stale_code_rows_for_skipped_file(
    connection: &Connection,
    repo_id: &str,
    path: &str,
) -> Result<usize, duckdb::Error> {
    let mut stale_rows = 0;
    stale_rows += connection.execute(
        "UPDATE code_documents SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current'",
        params![repo_id, path],
    )?;
    stale_rows += connection.execute(
        "UPDATE code_symbols SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current'",
        params![repo_id, path],
    )?;
    stale_rows += connection.execute(
        "UPDATE code_edges SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current'",
        params![repo_id, path],
    )?;
    stale_rows += connection.execute(
        "UPDATE code_diagnostics SET freshness = 'stale' WHERE repo_id = ? AND path = ? AND freshness = 'current'",
        params![repo_id, path],
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
            "SELECT DISTINCT issue_key, number, title, url, branch, merge_sha, merged_at FROM pull_requests ORDER BY issue_key, number, merged_at NULLS LAST",
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
        let entries = by_issue.entry(issue_key).or_default();
        // Source ownership is retained in DuckDB, but identical evidence from
        // two owners is one logical read-model item.
        if !entries.iter().any(|existing| existing == &pr) {
            entries.push(pr);
        }
    }
    Ok(by_issue)
}

fn load_issue_areas(
    connection: &Connection,
    issue_key: &str,
) -> Result<Vec<String>, duckdb::Error> {
    let mut statement =
        connection.prepare("SELECT DISTINCT area FROM issue_areas WHERE issue_key = ? ORDER BY area")?;
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
        .prepare("SELECT DISTINCT file_path FROM changed_files WHERE issue_key = ? ORDER BY file_path")?;
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
    refresh_memory_index_from_okf_inner(config, bundle_root, true, None, None)
}

pub fn merge_memory_index_from_okf(
    config: &MemoryConfig,
    bundle_root: &Path,
    repository_id: &str,
    source_id: &str,
) -> Result<MemoryReindexReport, MemoryError> {
    refresh_memory_index_from_okf_inner(
        config,
        bundle_root,
        false,
        Some(repository_id),
        Some(source_id),
    )
}

pub fn merge_legacy_memory_index(
    config: &MemoryConfig,
    source_config: &MemoryConfig,
    source_id: &str,
) -> Result<(), MemoryError> {
    let Some(source_connection) = open_existing_index_read_only(source_config)? else {
        return Ok(());
    };
    let areas = {
        let mut statement = source_connection
            .prepare("SELECT issue_key, area FROM issue_areas")
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<(String, String)>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
    };
    let pull_requests = {
        let mut statement = source_connection
            .prepare("SELECT issue_key, number, title, url, branch, merge_sha, merged_at FROM pull_requests")
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
    };
    let changed_files = {
        let mut statement = source_connection
            .prepare("SELECT issue_key, pr_number, file_path, change_kind FROM changed_files")
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
    };
    let checks = {
        let mut statement = source_connection
            .prepare("SELECT issue_key, pr_number, name, conclusion, completed_at FROM checks")
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
    };
    let reviews = {
        let mut statement = source_connection
            .prepare("SELECT issue_key, pr_number, reviewer, state, submitted_at, disposition FROM reviews")
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| MemoryError::DuckDb {
                path: source_config.index_path.clone(),
                source,
            })?
    };

    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let transaction = connection.transaction().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let owned = {
        let mut statement = transaction
            .prepare("SELECT issue_key, source_ids_json FROM issues")
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| {
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
            })?
            .into_iter()
            .filter_map(|(issue_key, encoded)| {
                serde_json::from_str::<Vec<String>>(&encoded)
                    .ok()
                    .filter(|ids| ids.iter().any(|id| id == source_id))
                    .map(|ids| (issue_key, ids))
            })
            .collect::<BTreeMap<_, _>>()
    };
    for (issue_key, source_ids) in &owned {
        if source_ids.len() <= 1 {
            continue;
        }
        let preserves_live_relations = source_ids.iter().any(|owner| is_live_capture_owner(owner));
        let source_ids = source_ids
            .iter()
            .filter(|owner| source_id_belongs_to_configured_repository(config, owner))
            .collect::<Vec<_>>();
        for owner in source_ids {
            transaction
                .execute(
                    "INSERT INTO issue_areas (issue_key, area, source_id) SELECT issue_key, area, ? FROM issue_areas WHERE issue_key = ? AND source_id IS NULL AND NOT EXISTS (SELECT 1 FROM issue_areas AS existing WHERE existing.issue_key = issue_areas.issue_key AND existing.area = issue_areas.area AND existing.source_id = ?)",
                    params![owner, issue_key, owner],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            transaction
                .execute(
                    "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at, source_id) SELECT issue_key, number, title, url, branch, merge_sha, merged_at, ? FROM pull_requests WHERE issue_key = ? AND source_id IS NULL AND NOT EXISTS (SELECT 1 FROM pull_requests AS existing WHERE existing.issue_key = pull_requests.issue_key AND existing.number = pull_requests.number AND existing.source_id = ?)",
                    params![owner, issue_key, owner],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            transaction
                .execute(
                    "INSERT INTO changed_files (issue_key, pr_number, file_path, change_kind, source_id) SELECT issue_key, pr_number, file_path, change_kind, ? FROM changed_files WHERE issue_key = ? AND source_id IS NULL AND NOT EXISTS (SELECT 1 FROM changed_files AS existing WHERE existing.issue_key = changed_files.issue_key AND existing.pr_number = changed_files.pr_number AND existing.file_path = changed_files.file_path AND existing.source_id = ?)",
                    params![owner, issue_key, owner],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            transaction
                .execute(
                    "INSERT INTO checks (issue_key, pr_number, name, conclusion, completed_at, source_id) SELECT issue_key, pr_number, name, conclusion, completed_at, ? FROM checks WHERE issue_key = ? AND source_id IS NULL AND NOT EXISTS (SELECT 1 FROM checks AS existing WHERE existing.issue_key = checks.issue_key AND existing.pr_number = checks.pr_number AND existing.name = checks.name AND existing.source_id = ?)",
                    params![owner, issue_key, owner],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            transaction
                .execute(
                    "INSERT INTO reviews (issue_key, pr_number, reviewer, state, submitted_at, disposition, source_id) SELECT issue_key, pr_number, reviewer, state, submitted_at, disposition, ? FROM reviews WHERE issue_key = ? AND source_id IS NULL AND NOT EXISTS (SELECT 1 FROM reviews AS existing WHERE existing.issue_key = reviews.issue_key AND existing.pr_number = reviews.pr_number AND existing.reviewer IS NOT DISTINCT FROM reviews.reviewer AND existing.submitted_at IS NOT DISTINCT FROM reviews.submitted_at AND existing.source_id = ?)",
                    params![owner, issue_key, owner],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        if !preserves_live_relations {
            for table in [
                "issue_areas",
                "pull_requests",
                "changed_files",
                "checks",
                "reviews",
            ] {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE issue_key = ? AND source_id IS NULL"),
                        [issue_key],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }
    }
    for table in [
        "issue_areas",
        "pull_requests",
        "changed_files",
        "checks",
        "reviews",
    ] {
        transaction
            .execute(&format!("DELETE FROM {table} WHERE source_id = ?"), [source_id])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    for (issue_key, source_ids) in &owned {
        if source_ids.len() == 1 {
            for table in [
                "issue_areas",
                "pull_requests",
                "changed_files",
                "checks",
                "reviews",
            ] {
                transaction
                    .execute(&format!("DELETE FROM {table} WHERE issue_key = ?"), [issue_key])
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }
    }
    for (issue_key, area) in areas.into_iter().filter(|(key, _)| owned.contains_key(key)) {
        transaction.execute(
            "INSERT INTO issue_areas (issue_key, area, source_id) SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM issue_areas WHERE issue_key = ? AND area = ? AND source_id IS NOT DISTINCT FROM ?)",
            params![issue_key, area, source_id, issue_key, area, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
    }
    for (issue_key, number, title, url, branch, merge_sha, merged_at) in pull_requests.into_iter().filter(|row| owned.contains_key(&row.0)) {
        transaction.execute(
            "UPDATE pull_requests SET title = ?, url = ?, branch = ?, merge_sha = ?, merged_at = ? WHERE issue_key = ? AND number = ? AND source_id = ?",
            params![title, url, branch, merge_sha, merged_at, issue_key, number, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
        transaction.execute(
            "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at, source_id) SELECT ?, ?, ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM pull_requests WHERE issue_key = ? AND number = ? AND source_id = ?)",
            params![issue_key, number, title, url, branch, merge_sha, merged_at, source_id, issue_key, number, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
    }
    for (issue_key, number, path, kind) in changed_files.into_iter().filter(|row| owned.contains_key(&row.0)) {
        transaction.execute(
            "UPDATE changed_files SET change_kind = ? WHERE issue_key = ? AND pr_number = ? AND file_path = ? AND source_id = ?",
            params![kind, issue_key, number, path, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
        transaction.execute(
            "INSERT INTO changed_files (issue_key, pr_number, file_path, change_kind, source_id) SELECT ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM changed_files WHERE issue_key = ? AND pr_number = ? AND file_path = ? AND source_id = ?)",
            params![issue_key, number, path, kind, source_id, issue_key, number, path, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
    }
    for (issue_key, number, name, conclusion, completed_at) in checks.into_iter().filter(|row| owned.contains_key(&row.0)) {
        transaction.execute(
            "UPDATE checks SET conclusion = ?, completed_at = ? WHERE issue_key = ? AND pr_number = ? AND name = ? AND source_id = ?",
            params![conclusion, completed_at, issue_key, number, name, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
        transaction.execute(
            "INSERT INTO checks (issue_key, pr_number, name, conclusion, completed_at, source_id) SELECT ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM checks WHERE issue_key = ? AND pr_number = ? AND name = ? AND source_id = ?)",
            params![issue_key, number, name, conclusion, completed_at, source_id, issue_key, number, name, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
    }
    for (issue_key, number, reviewer, state, submitted_at, disposition) in reviews.into_iter().filter(|row| owned.contains_key(&row.0)) {
        transaction.execute(
            "UPDATE reviews SET state = ?, disposition = ? WHERE issue_key = ? AND pr_number = ? AND reviewer IS NOT DISTINCT FROM ? AND submitted_at IS NOT DISTINCT FROM ? AND source_id = ?",
            params![state, disposition, issue_key, number, reviewer, submitted_at, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
        transaction.execute(
            "INSERT INTO reviews (issue_key, pr_number, reviewer, state, submitted_at, disposition, source_id) SELECT ?, ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM reviews WHERE issue_key = ? AND pr_number = ? AND reviewer IS NOT DISTINCT FROM ? AND submitted_at IS NOT DISTINCT FROM ? AND source_id = ?)",
            params![issue_key, number, reviewer, state, submitted_at, disposition, source_id, issue_key, number, reviewer, submitted_at, source_id],
        ).map_err(|source| MemoryError::DuckDb { path: config.index_path.clone(), source })?;
    }
    transaction.commit().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })
}

pub fn backfill_legacy_memory_source_scopes(
    config: &MemoryConfig,
    repository_id: &str,
    source_id: &str,
) -> Result<(), MemoryError> {
    let project_scope_ids = config
        .repository_sources
        .get(repository_id)
        .map(|source| source.project_scope_ids.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_else(|| config.project_scope_ids.iter().cloned().collect());
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    let transaction = connection.transaction().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })?;
    transaction
        .execute(
            "DELETE FROM source_scope_refs WHERE source_id = ?",
            [source_id],
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = {
        let mut statement = transaction
            .prepare("SELECT issue_key, concept_id, scope_refs_json, source_refs_json, source_ids_json FROM issues")
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
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
            })?
    };
    for (issue_key, concept_id, encoded_scopes, encoded_sources, encoded_source_ids) in rows {
        let mut source_ids = serde_json::from_str::<Vec<String>>(&encoded_source_ids).unwrap_or_default();
        let has_registered_owner = source_ids.iter().any(|value| !is_live_capture_owner(value));
        if has_registered_owner && !source_ids.iter().any(|value| value == source_id) {
            continue;
        }
        if !source_ids.iter().any(|value| value == source_id) {
            source_ids.push(source_id.to_string());
        }
        let mut scopes = serde_json::from_str::<Vec<KnowledgeScope>>(&encoded_scopes).unwrap_or_default();
        let mut add_scope = |kind: KnowledgeScopeKind, id: String| {
            if !scopes.iter().any(|scope| scope.kind == kind && scope.id == id) {
                scopes.push(KnowledgeScope { kind, id, label: None });
            }
        };
        add_scope(KnowledgeScopeKind::Repository, repository_id.to_string());
        if let Some(project_set_id) = config.default_project_set_id.as_deref() {
            add_scope(KnowledgeScopeKind::ProjectSet, project_set_id.to_string());
        }
        for project_id in &project_scope_ids {
            add_scope(KnowledgeScopeKind::Project, project_id.clone());
        }
        let source_scopes = scopes
            .iter()
            .filter(|scope| match &scope.kind {
                KnowledgeScopeKind::Repository => scope.id == repository_id,
                KnowledgeScopeKind::ProjectSet => config
                    .default_project_set_id
                    .as_deref()
                    .is_some_and(|id| id == scope.id),
                KnowledgeScopeKind::Project => project_scope_ids.iter().any(|id| id == &scope.id),
                _ => false,
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut source_refs = serde_json::from_str::<Vec<MemorySourceRef>>(&encoded_sources).unwrap_or_default();
        let source_ref = MemorySourceRef {
            kind: "legacy_store".to_string(),
            id: source_id.to_string(),
            url: None,
            repo_id: Some(repository_id.to_string()),
            symbol_key: None,
            registration_source_id: Some(source_id.to_string()),
        };
        if !source_refs.contains(&source_ref) {
            source_refs.push(source_ref);
        }
        let scopes_json = serde_json::to_string(&scopes)?;
        transaction
            .execute("DELETE FROM scope_refs WHERE concept_id = ?", [&concept_id])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        for scope in &scopes {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                    duckdb::params![concept_id, scope_kind_name(&scope.kind), scope.id, scope.label],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        for scope in &source_scopes {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?, ?)",
                    duckdb::params![
                        concept_id,
                        source_id,
                        scope_kind_name(&scope.kind),
                        scope.id,
                        scope.label,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        transaction
            .execute(
                "UPDATE issues SET source_ids_json = ?, scope_refs_json = ?, source_refs_json = ? WHERE issue_key = ?",
                duckdb::params![
                    serde_json::to_string(&source_ids)?,
                    scopes_json,
                    serde_json::to_string(&source_refs)?,
                    issue_key,
                ],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    transaction.commit().map_err(|source| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source,
    })
}

fn source_id_belongs_to_configured_repository(config: &MemoryConfig, source_id: &str) -> bool {
    config
        .repository_sources
        .values()
        .any(|source| {
            source_id.starts_with(&format!("{}:", source.repository_id))
                || source_id == live_capture_owner(Some(&source.repository_id))
        })
}

fn refresh_memory_index_from_okf_inner(
    config: &MemoryConfig,
    bundle_root: &Path,
    replace_existing: bool,
    repository_id: Option<&str>,
    source_id: Option<&str>,
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
        let bundle_path = OkfBundlePath::new(&relative)?;
        if bundle_path.reserved_file().is_some() {
            continue;
        }
        let contents = read_to_string(&path)?;
        let concept = parse_okf_concept(&bundle_root, &path, &contents)?;
        let has_work_item_identity = okf_has_work_item_identity(&concept, &concept.frontmatter.opensymphony);
        let mut row = OkfIndexRow::from_concept(
            config,
            relative.clone(),
            concept,
            contents,
            warnings_by_path.remove(&path).unwrap_or_default(),
        )?;
        if let Some(source_id) = source_id
            && !has_work_item_identity
        {
            row.issue_key = format!("{source_id}:{}", row.issue_key);
            row.concept_id = format!("{source_id}:{}", row.concept_id);
        }
        if let Some(repository_id) = repository_id {
            let mut scope_refs =
                serde_json::from_str::<Vec<KnowledgeScope>>(&row.scope_refs_json)
                    .unwrap_or_default();
            if !scope_refs.iter().any(|scope| {
                scope.kind == KnowledgeScopeKind::Repository && scope.id == repository_id
            }) {
                scope_refs.push(KnowledgeScope {
                    kind: KnowledgeScopeKind::Repository,
                    id: repository_id.to_string(),
                    label: None,
                });
                row.scope_refs_json = serde_json::to_string(&scope_refs)?;
            }
        }
        if let Some(source_id) = source_id {
            let mut source_refs = serde_json::from_str::<Vec<MemorySourceRef>>(
                &row.source_refs_json,
            )
            .unwrap_or_default();
            for source_ref in &mut source_refs {
                source_ref.registration_source_id = Some(source_id.to_string());
            }
            row.source_refs_json = serde_json::to_string(&source_refs)?;
        }
        let mut scope_refs = serde_json::from_str::<Vec<KnowledgeScope>>(&row.scope_refs_json)
            .unwrap_or_default();
        if let Some(repository_id) = repository_id {
            let project_scope_ids = config
                .repository_sources
                .get(repository_id)
                .map(|source| &source.project_scope_ids);
            scope_refs.retain(|scope| match &scope.kind {
                KnowledgeScopeKind::Project => project_scope_ids
                    .is_some_and(|project_scope_ids| project_scope_ids.contains(&scope.id)),
                KnowledgeScopeKind::ProjectSet => config
                    .default_project_set_id
                    .as_deref()
                    .is_some_and(|project_set_id| project_set_id == scope.id),
                _ => true,
            });
        }
        if let Some(project_set_id) = config.default_project_set_id.as_deref()
            && !scope_refs
                .iter()
                .any(|scope| scope.kind == KnowledgeScopeKind::ProjectSet)
        {
            scope_refs.push(KnowledgeScope {
                kind: KnowledgeScopeKind::ProjectSet,
                id: project_set_id.to_string(),
                label: None,
            });
        }
        let project_scope_ids = repository_id
            .and_then(|repository_id| config.repository_sources.get(repository_id))
            .map(|source| &source.project_scope_ids)
            .unwrap_or(&config.project_scope_ids);
        if !scope_refs
            .iter()
            .any(|scope| scope.kind == KnowledgeScopeKind::Project)
        {
            for project_id in project_scope_ids {
                scope_refs.push(KnowledgeScope {
                    kind: KnowledgeScopeKind::Project,
                    id: project_id.clone(),
                    label: None,
                });
            }
        }
        row.scope_refs_json = serde_json::to_string(&scope_refs)?;
        if let Some(source_id) = source_id {
            row.source_ids_json = serde_json::to_string(&vec![source_id])?;
        }
        rows.push(row);
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
    if let Some(source_id) = source_id {
        for table in [
            "issue_areas",
            "pull_requests",
            "changed_files",
            "checks",
            "reviews",
        ] {
            transaction
                .execute(&format!("DELETE FROM {table} WHERE source_id = ?"), [source_id])
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
            })?;
        }
        transaction
            .execute(
                "DELETE FROM source_scope_refs WHERE source_id = ?",
                [source_id],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    if replace_existing {
        transaction
            .execute(
                "UPDATE registered_memory_sources SET status = 'pending'",
                [],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute(
                "DELETE FROM scope_refs WHERE concept_id IN (SELECT concept_id FROM issues)",
                [],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute("DELETE FROM source_scope_refs", [])
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
    }

    let issue_insert = if replace_existing {
        "INSERT INTO"
    } else {
        "INSERT OR REPLACE INTO"
    };
    if let Some(source_id) = source_id {
        let registered_source_repositories = {
            let mut statement = transaction
                .prepare("SELECT source_id, repository_id FROM registered_memory_sources")
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?
                .collect::<Result<BTreeMap<_, _>, _>>()
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?
        };
        let incoming = rows
            .iter()
            .map(|row| row.concept_id.as_str())
            .collect::<BTreeSet<_>>();
        let stale = {
            let mut statement = transaction
                .prepare("SELECT issue_key, concept_id, source_ids_json, scope_refs_json, source_refs_json FROM issues")
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
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
                })?
        };
        for (issue_key, concept_id, encoded_source_ids, encoded_scopes, encoded_sources) in stale {
            let mut source_ids = serde_json::from_str::<Vec<String>>(&encoded_source_ids)
                .unwrap_or_default();
            if incoming.contains(concept_id.as_str())
                || !source_ids.iter().any(|value| value == source_id)
            {
                continue;
            }
            source_ids.retain(|value| value != source_id);
            if source_ids.is_empty() {
                transaction
                    .execute("DELETE FROM scope_refs WHERE concept_id = ?", [&concept_id])
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
                transaction
                    .execute("DELETE FROM issues WHERE issue_key = ?", [&issue_key])
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
                for table in [
                    "issue_areas",
                    "pull_requests",
                    "changed_files",
                    "checks",
                    "reviews",
                ] {
                    transaction
                        .execute(&format!("DELETE FROM {table} WHERE issue_key = ?"), [&issue_key])
                        .map_err(|source| MemoryError::DuckDb {
                            path: config.index_path.clone(),
                            source,
                        })?;
                }
            } else {
                let source_scope_rows = {
                    let mut statement = transaction
                        .prepare(
                            "SELECT source_id, scope_id FROM source_scope_refs WHERE concept_id = ? AND scope_kind = 'project'",
                        )
                        .map_err(|source| MemoryError::DuckDb {
                            path: config.index_path.clone(),
                            source,
                        })?;
                    statement
                        .query_map([&concept_id], |row| {
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
                        })?
                };
                let remaining_project_scopes = if source_scope_rows.is_empty() {
                    source_ids
                        .iter()
                        .filter_map(|owner| {
                            source_owner_repository_id(&registered_source_repositories, owner)
                        })
                        .filter_map(|repository_id| config.repository_sources.get(repository_id))
                        .flat_map(|source| source.project_scope_ids.iter())
                        .cloned()
                        .collect::<BTreeSet<_>>()
                } else {
                    source_scope_rows
                        .into_iter()
                        .filter(|(owner, _)| source_ids.contains(owner))
                        .map(|(_, scope_id)| scope_id)
                        .collect::<BTreeSet<_>>()
                };
                let scopes = serde_json::from_str::<Vec<KnowledgeScope>>(&encoded_scopes)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|scope| match &scope.kind {
                        KnowledgeScopeKind::Repository => source_ids.iter().any(|owner| {
                            source_owner_repository_id(&registered_source_repositories, owner)
                                .is_some_and(|repository_id| repository_id == scope.id)
                        }),
                        KnowledgeScopeKind::Project => {
                            remaining_project_scopes.contains(&scope.id)
                        }
                        KnowledgeScopeKind::ProjectSet => config
                            .default_project_set_id
                            .as_deref()
                            .is_some_and(|id| id == scope.id),
                        _ => true,
                    })
                    .collect::<Vec<_>>();
                let sources = serde_json::from_str::<Vec<MemorySourceRef>>(&encoded_sources)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|source| {
                        source.registration_source_id.as_deref() != Some(source_id)
                            && !(source.registration_source_id.is_none()
                                && source.id == source_id)
                    })
                    .collect::<Vec<_>>();
                transaction
                    .execute("DELETE FROM scope_refs WHERE concept_id = ?", [&concept_id])
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
                for scope in &scopes {
                    transaction
                        .execute(
                            "INSERT OR IGNORE INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                            params![concept_id, scope_kind_name(&scope.kind), scope.id, scope.label],
                        )
                        .map_err(|source| MemoryError::DuckDb {
                            path: config.index_path.clone(),
                            source,
                        })?;
                }
                transaction
                    .execute(
                        "UPDATE issues SET source_ids_json = ?, scope_refs_json = ?, source_refs_json = ? WHERE issue_key = ?",
                        params![
                            serde_json::to_string(&source_ids)?,
                            serde_json::to_string(&scopes)?,
                            serde_json::to_string(&sources)?,
                            issue_key
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }
    }
    for row in &rows {
        let mut scope_refs = serde_json::from_str::<Vec<KnowledgeScope>>(&row.scope_refs_json)
            .unwrap_or_default();
        let incoming_source_scopes = scope_refs.clone();
        let mut source_refs = serde_json::from_str::<Vec<MemorySourceRef>>(&row.source_refs_json)
            .unwrap_or_default();
        if let Some(repository_id) = repository_id {
            // OKF frontmatter historically omitted repository provenance from
            // source refs. Stamp newly imported refs so a later refresh can
            // replace this source's references without retaining withdrawn
            // URLs or IDs.
            for source_ref in &mut source_refs {
                if source_ref.repo_id.is_none() {
                    source_ref.repo_id = Some(repository_id.to_string());
                }
            }
        }
        let mut source_ids = serde_json::from_str::<Vec<String>>(&row.source_ids_json)
            .unwrap_or_default();
        if !replace_existing {
            let existing = transaction
                .query_row(
                    "SELECT scope_refs_json, source_refs_json, source_ids_json FROM issues WHERE issue_key = ?",
                    params![row.issue_key],
                    |query_row| {
                        Ok((
                            query_row.get::<_, String>(0)?,
                            query_row.get::<_, String>(1)?,
                            query_row.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
            if let Some((existing_scopes, existing_sources, existing_source_ids)) = existing {
                let existing_source_ids = serde_json::from_str::<Vec<String>>(&existing_source_ids)
                    .unwrap_or_default();
                let source_only = source_id.is_some_and(|source_id| {
                    !existing_source_ids.is_empty()
                        && existing_source_ids.iter().all(|id| id == source_id)
                });
                if !source_only {
                    let refreshed_project_scopes = repository_id
                        .and_then(|repository_id| config.repository_sources.get(repository_id))
                        .map(|source| &source.project_scope_ids)
                        .unwrap_or(&config.project_scope_ids);
                    let other_owner_project_scopes = {
                        let source_id = source_id.unwrap_or_default();
                        let mut statement = transaction
                            .prepare(
                                "SELECT scope_id FROM source_scope_refs WHERE concept_id = ? AND source_id <> ? AND scope_kind = 'project'",
                            )
                            .map_err(|source| MemoryError::DuckDb {
                                path: config.index_path.clone(),
                                source,
                            })?;
                        statement
                            .query_map(params![row.concept_id, source_id], |row| row.get(0))
                            .map_err(|source| MemoryError::DuckDb {
                                path: config.index_path.clone(),
                                source,
                            })?
                            .collect::<Result<BTreeSet<String>, _>>()
                            .map_err(|source| MemoryError::DuckDb {
                                path: config.index_path.clone(),
                                source,
                            })?
                    };
                    let incoming_project_set_ids = scope_refs
                        .iter()
                        .filter(|scope| scope.kind == KnowledgeScopeKind::ProjectSet)
                        .map(|scope| scope.id.clone())
                        .collect::<BTreeSet<_>>();
                    for scope in serde_json::from_str::<Vec<KnowledgeScope>>(&existing_scopes)
                        .unwrap_or_default()
                    {
                        let source_scope = source_id.is_some_and(|source_id| {
                            existing_source_ids.iter().any(|id| id == source_id)
                        });
                        let replaced_scope = source_scope
                            && ((repository_id.is_some_and(|repository_id| {
                                scope.kind == KnowledgeScopeKind::Repository
                                    && scope.id == repository_id
                                }))
                                || (scope.kind == KnowledgeScopeKind::ProjectSet
                                    && !incoming_project_set_ids.contains(&scope.id))
                                || (scope.kind == KnowledgeScopeKind::Project
                                    && refreshed_project_scopes.contains(&scope.id)
                                    && !other_owner_project_scopes.contains(&scope.id)));
                        let stale_project_scope = scope.kind == KnowledgeScopeKind::Project
                            && !other_owner_project_scopes.contains(&scope.id)
                            && !scope_refs.contains(&scope);
                        if replaced_scope || stale_project_scope {
                            continue;
                        }
                        if !scope_refs.contains(&scope) {
                            scope_refs.push(scope);
                        }
                    }
                }
                for source in serde_json::from_str::<Vec<MemorySourceRef>>(&existing_sources)
                    .unwrap_or_default()
                {
                    if source_id.is_some_and(|source_id| {
                        source.registration_source_id.as_deref() == Some(source_id)
                    }) {
                        continue;
                    }
                    if !source_refs.contains(&source) {
                        source_refs.push(source);
                    }
                }
                for existing_source_id in existing_source_ids {
                    if !source_ids.contains(&existing_source_id) {
                        source_ids.push(existing_source_id);
                    }
                }
            }
        }
        let source_ids_json = serde_json::to_string(&source_ids)?;
        let scope_refs_json = serde_json::to_string(&scope_refs)?;
        let source_refs_json = serde_json::to_string(&source_refs)?;
        if let Some(source_id) = source_id {
            for scope_ref in &incoming_source_scopes {
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?, ?)",
                        params![
                            row.concept_id,
                            source_id,
                            scope_kind_name(&scope_ref.kind),
                            scope_ref.id,
                            scope_ref.label,
                        ],
                    )
                    .map_err(|source| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source,
                    })?;
            }
        }
        if !replace_existing {
            transaction
                .execute(
                    "DELETE FROM scope_refs WHERE concept_id = ?",
                    params![row.concept_id],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        transaction
            .execute(
                &format!("{issue_insert} issues (issue_key, title, state, milestone, labels_json, completion_time, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, concept_type, description, tags_json, scope_refs_json, source_refs_json, source_ids_json, links_json, citations_json, freshness, warnings_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"),
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
                    scope_refs_json,
                    source_refs_json,
                    source_ids_json,
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
        for scope_ref in scope_refs {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                    params![
                        row.concept_id,
                        scope_kind_name(&scope_ref.kind),
                        scope_ref.id,
                        scope_ref.label,
                    ],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
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
                    "INSERT INTO issue_areas (issue_key, area, source_id) SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM issue_areas WHERE issue_key = ? AND area = ? AND source_id IS NOT DISTINCT FROM ?)",
                    params![row.issue_key, area, source_id, row.issue_key, area, source_id],
                )
                .map_err(|source| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source,
                })?;
        }
        for pr in &row.prs {
            transaction
                .execute(
                    "INSERT INTO pull_requests (issue_key, number, title, url, branch, merge_sha, merged_at, source_id) SELECT ?, ?, ?, ?, ?, ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM pull_requests WHERE issue_key = ? AND number = ? AND source_id IS NOT DISTINCT FROM ?)",
                    params![
                        row.issue_key,
                        pr.number as i64,
                        pr.title.clone(),
                        pr.url.clone(),
                        pr.branch.clone(),
                        pr.merge_sha.clone(),
                        pr.merged_at.map(|value| value.to_rfc3339()),
                        source_id,
                        row.issue_key,
                        pr.number as i64,
                        source_id,
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
    source_ids_json: String,
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
            source_ids_json: "[]".to_string(),
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

fn okf_has_work_item_identity(
    concept: &OkfConcept,
    metadata: &Option<OpenSymphonyOkfMetadata>,
) -> bool {
    metadata
        .as_ref()
        .is_some_and(|metadata| {
            metadata
                .scope_refs
                .iter()
                .any(|scope| scope.kind == KnowledgeScopeKind::WorkItem)
                || metadata
                    .source_refs
                    .iter()
                    .any(|source| source.kind == "linear_issue")
        })
        || string_extra(&concept.frontmatter, "issue").is_some()
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
    sha256_bytes_hex(contents.as_bytes())
}

pub fn sha256_bytes_hex(contents: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(contents);
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

    fn test_code_document(path: &str, content_sha256: &str) -> CodeIntelDocumentInput {
        CodeIntelDocumentInput {
            path: PathBuf::from(path),
            language: "rust".to_string(),
            content_sha256: content_sha256.to_string(),
            parser_id: "tree-sitter".to_string(),
            parser_version: "test-parser".to_string(),
            query_pack_version: "test-query-pack".to_string(),
            byte_len: 1,
            line_count: 1,
            symbols: Vec::new(),
            edges: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn test_code_document_with_edges_and_diagnostics(
        path: &str,
        content_sha256: &str,
    ) -> CodeIntelDocumentInput {
        let mut document = test_code_document(path, content_sha256);
        document.edges.push(CodeIntelEdgeInput {
            edge_kind: "reference".to_string(),
            target_hint: Some("helper".to_string()),
            confidence: "exact".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 7,
            start_byte: 0,
            end_byte: 6,
        });
        document.diagnostics.push(CodeIntelDiagnosticInput {
            kind: "syntax".to_string(),
            severity: "warning".to_string(),
            message: "test diagnostic".to_string(),
            start_line: 1,
            start_col: 1,
            end_line: 1,
            end_col: 2,
            start_byte: 0,
            end_byte: 1,
        });
        document
    }

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

    #[test]
    fn backfill_legacy_source_scopes_migrates_unscoped_rows_in_place() {
        let repo = tempfile::TempDir::new().expect("repository tempdir");
        let mut config = MemoryConfig::load(repo.path(), None).expect("memory config");
        config = config
            .with_default_project_set_id("set-alpha")
            .with_repository_source(MemoryRepositorySource {
                repository_id: "repo-alpha".to_string(),
                root: repo.path().to_path_buf(),
                commit_sha: Some("commit-alpha".to_string()),
                project_scope_ids: BTreeSet::from(["project-alpha".to_string()]),
                target_branch: Some("develop".to_string()),
            });
        let connection = open_index(&config).expect("index should open");
        migrate_index(&connection).expect("index should migrate");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-550",
                    "Legacy memory",
                    "[]",
                    "not_archived",
                    "issues/COE-550.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-550",
                    "[]",
                    "[]",
                    "[]",
                ],
            )
            .expect("legacy issue should insert");

        backfill_legacy_memory_source_scopes(&config, "repo-alpha", "repo-alpha:legacy_store")
            .expect("legacy scopes should backfill");

        let issue = load_indexed_issues(&config)
            .expect("issues should load")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-550")
            .expect("backfilled issue should exist");
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::Repository && scope.id == "repo-alpha"
        }));
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::ProjectSet && scope.id == "set-alpha"
        }));
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::Project && scope.id == "project-alpha"
        }));
        assert!(issue
            .source_refs
            .iter()
            .any(|source| source.id == "repo-alpha:legacy_store"));
        let connection = open_existing_index_read_only(&config)
            .expect("index should reopen")
            .expect("index should exist");
        let source_scope_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM source_scope_refs WHERE concept_id = 'issues/COE-550' AND source_id = 'repo-alpha:legacy_store'",
                [],
                |row| row.get(0),
            )
            .expect("source scope refs");
        assert_eq!(source_scope_count, 3);
    }

    #[test]
    fn merge_legacy_memory_index_preserves_live_relation_rows() {
        let catalog_root = tempfile::TempDir::new().expect("catalog root");
        let source_root = tempfile::TempDir::new().expect("source root");
        let mut config = MemoryConfig::load(catalog_root.path(), None).expect("catalog config");
        config = config.with_repository_source(MemoryRepositorySource {
            repository_id: "repo-a".to_string(),
            root: source_root.path().to_path_buf(),
            commit_sha: None,
            project_scope_ids: BTreeSet::new(),
            target_branch: None,
        });
        let source_config = MemoryConfig::load(source_root.path(), None).expect("source config");
        let connection = open_index(&config).expect("catalog index");
        migrate_index(&connection).expect("catalog index migration");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-550",
                    "Live capture",
                    "[]",
                    "not_archived",
                    "issues/COE-550.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-08-01T00:00:00Z",
                    "issues/COE-550",
                    "[]",
                    "[]",
                    r#"["__live_capture__","repo-a:legacy"]"#,
                ],
            )
            .expect("catalog issue");
        connection
            .execute(
                "INSERT INTO issue_areas (issue_key, area) VALUES ('COE-550', 'memory')",
                [],
            )
            .expect("live relation");
        drop(connection);

        let source_connection = open_index(&source_config).expect("source index");
        migrate_index(&source_connection).expect("source index migration");
        source_connection
            .execute(
                "INSERT INTO issue_areas (issue_key, area) VALUES ('COE-550', 'memory')",
                [],
            )
            .expect("source relation");
        drop(source_connection);

        merge_legacy_memory_index(&config, &source_config, "repo-a:legacy")
            .expect("legacy merge");

        let connection = open_existing_index_read_only(&config)
            .expect("catalog index should reopen")
            .expect("catalog index exists");
        let live_relations: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM issue_areas WHERE issue_key = 'COE-550' AND source_id IS NULL",
                [],
                |row| row.get(0),
            )
            .expect("live relation count");
        let imported_relations: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM issue_areas WHERE issue_key = 'COE-550' AND source_id = 'repo-a:legacy'",
                [],
                |row| row.get(0),
            )
            .expect("imported relation count");
        assert_eq!(live_relations, 1);
        assert_eq!(imported_relations, 1);
    }

    #[test]
    fn staged_code_rows_are_hidden_and_parsed_files_stale_same_revision_skips() {
        let repo = tempfile::TempDir::new().expect("repository tempdir");
        let config = MemoryConfig::load(repo.path(), None).expect("memory config");

        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "fixture-repo".to_string(),
                commit_sha: Some("base-commit".to_string()),
                worktree_dirty: false,
                documents: vec![test_code_document_with_edges_and_diagnostics(
                    "src/reused.rs",
                    "same-hash",
                )],
            },
        )
        .expect("current document should persist");
        persist_code_intel_documents_with_freshness(
            &config,
            CodeIntelPersistBatch {
                repo_id: "fixture-repo".to_string(),
                commit_sha: Some("next-commit".to_string()),
                worktree_dirty: false,
                documents: vec![test_code_document_with_edges_and_diagnostics(
                    "src/reused.rs",
                    "same-hash",
                )],
            },
            "staged",
            false,
        )
        .expect("staged reuse should persist revision rows");
        let connection = Connection::open(&config.index_path).expect("index should open");
        let (commit_sha, freshness): (String, String) = connection
            .query_row(
                "SELECT commit_sha, freshness FROM code_documents WHERE repo_id = ? AND path = ?",
                params!["fixture-repo", "src/reused.rs"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("current document should remain queryable");
        assert_eq!(commit_sha, "base-commit");
        assert_eq!(freshness, "current");
        let (commit_sha, freshness): (String, String) = connection
            .query_row(
                "SELECT commit_sha, freshness FROM code_edges WHERE repo_id = ? AND path = ?",
                params!["fixture-repo", "src/reused.rs"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("current edge should remain queryable");
        assert_eq!(commit_sha, "base-commit");
        assert_eq!(freshness, "current");
        let (commit_sha, freshness): (String, String) = connection
            .query_row(
                "SELECT commit_sha, freshness FROM code_diagnostics WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                params!["fixture-repo", "src/reused.rs"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("current diagnostic should remain queryable");
        assert_eq!(commit_sha, "base-commit");
        assert_eq!(freshness, "current");
        let staged_diagnostics: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM code_diagnostics WHERE repo_id = ? AND path = ? AND commit_sha = ? AND freshness = 'staged'",
                params!["fixture-repo", "src/reused.rs", "next-commit"],
                |row| row.get(0),
            )
            .expect("staged diagnostic should remain visible to promotion");
        assert_eq!(staged_diagnostics, 1);
        drop(connection);

        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "dirty-repo".to_string(),
                commit_sha: Some("workspace-commit".to_string()),
                worktree_dirty: true,
                documents: vec![test_code_document_with_edges_and_diagnostics(
                    "src/reused.rs",
                    "same-hash",
                )],
            },
        )
        .expect("dirty current document should persist");
        persist_code_intel_documents_with_freshness(
            &config,
            CodeIntelPersistBatch {
                repo_id: "dirty-repo".to_string(),
                commit_sha: Some("next-commit".to_string()),
                worktree_dirty: false,
                documents: vec![test_code_document_with_edges_and_diagnostics(
                    "src/reused.rs",
                    "same-hash",
                )],
            },
            "staged",
            false,
        )
        .expect("staged document should not replace a dirty current row");
        let connection = Connection::open(&config.index_path).expect("index should open");
        let (commit_sha, worktree_dirty): (String, bool) = connection
            .query_row(
                "SELECT commit_sha, worktree_dirty FROM code_documents WHERE repo_id = ? AND path = ? AND freshness = 'current'",
                params!["dirty-repo", "src/reused.rs"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("dirty current document should remain queryable");
        assert_eq!(commit_sha, "workspace-commit");
        assert!(worktree_dirty);
        drop(connection);

        persist_code_intel_skipped_files(
            &config,
            "dirty-skipped-repo",
            Some("workspace-commit"),
            false,
            &[CodeIntelSkippedFileInput {
                path: PathBuf::from("src/skipped.rs"),
                reason: "unsupported".to_string(),
                content_sha256: "skipped-hash".to_string(),
            }],
        )
        .expect("current skipped row should persist");
        persist_code_intel_skipped_files_with_freshness(
            &config,
            "dirty-skipped-repo",
            Some("workspace-commit"),
            false,
            &[CodeIntelSkippedFileInput {
                path: PathBuf::from("src/skipped.rs"),
                reason: "unsupported".to_string(),
                content_sha256: "skipped-hash".to_string(),
            }],
            "staged",
            false,
        )
        .expect("staged skipped row should not replace current coverage");
        let connection = Connection::open(&config.index_path).expect("index should open");
        let (freshness, staged_count): (String, i64) = connection
            .query_row(
                "SELECT (SELECT freshness FROM code_skipped_files WHERE repo_id = ? AND path = ?), (SELECT COUNT(*) FROM code_skipped_files_staging WHERE repo_id = ? AND path = ?)",
                params![
                    "dirty-skipped-repo",
                    "src/skipped.rs",
                    "dirty-skipped-repo",
                    "src/skipped.rs"
                ],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("skipped staging rows should be readable");
        assert_eq!(freshness, "current");
        assert_eq!(staged_count, 1);
        drop(connection);

        persist_code_intel_documents_with_freshness(
            &config,
            CodeIntelPersistBatch {
                repo_id: "staged-repo".to_string(),
                commit_sha: Some("staged-commit".to_string()),
                worktree_dirty: false,
                documents: vec![test_code_document("src/staged.rs", "staged-hash")],
            },
            "staged",
            false,
        )
        .expect("staged document should persist");
        assert!(!code_graph_repos(&config, true)
            .expect("staged rows should be hidden")
            .repos
            .iter()
            .any(|repo| repo.repo_id == "staged-repo"));

        persist_code_intel_skipped_files(
            &config,
            "fixture-repo",
            Some("commit"),
            false,
            &[CodeIntelSkippedFileInput {
                path: PathBuf::from("src/lib.rs"),
                reason: "oversized".to_string(),
                content_sha256: "skipped-hash".to_string(),
            }],
        )
        .expect("skip coverage should persist");
        persist_code_intel_documents(
            &config,
            CodeIntelPersistBatch {
                repo_id: "fixture-repo".to_string(),
                commit_sha: Some("commit".to_string()),
                worktree_dirty: false,
                documents: vec![test_code_document("src/lib.rs", "parsed-hash")],
            },
        )
        .expect("parsed document should persist");

        let connection = Connection::open(&config.index_path).expect("index should open");
        let freshness: String = connection
            .query_row(
                "SELECT freshness FROM code_skipped_files WHERE repo_id = ? AND path = ? AND commit_sha = ?",
                params!["fixture-repo", "src/lib.rs", "commit"],
                |row| row.get(0),
            )
            .expect("skip coverage should remain queryable");
        assert_eq!(freshness, "stale");
    }

    #[test]
    fn configured_target_branch_rejects_invalid_git_branch_marker() {
        let repo = tempfile::TempDir::new().expect("repository tempdir");
        fs::write(
            repo.path().join("WORKFLOW.md"),
            "## Branch target\n\nTarget branch: `HEAD`\n",
        )
        .expect("workflow marker");

        let result = code_index_branch(repo.path());
        assert!(matches!(
            result,
            Err(CodeGraphProjectionError::InvalidRequest(message))
                if message == "configured target branch is invalid"
        ));
    }
}
