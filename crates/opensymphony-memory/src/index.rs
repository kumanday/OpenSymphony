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
        transaction
            .execute("DELETE FROM issues WHERE issue_key = ?", params![issue_key])
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
        transaction
            .execute(
                "INSERT INTO issues (issue_key, title, state, milestone, labels_json, completion_time, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
  captured_at TEXT NOT NULL
);
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
"#,
    ))
}

fn load_indexed_issues(config: &MemoryConfig) -> Result<Vec<IndexedIssue>, MemoryError> {
    if !config.index_path.exists() {
        return Ok(Vec::new());
    }
    let connection = open_index_read_only(config)?;

    let mut statement = connection
        .prepare(
            "SELECT issue_key, title, state, milestone, labels_json, capsule_path, visibility, source_hash, warning_count, docs_sync_status, completion_time, captured_at, body FROM issues ORDER BY issue_key",
        )
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    let rows = statement
        .query_map([], |row| {
            let labels_json: String = row.get(4)?;
            Ok(IndexedIssue {
                issue_key: row.get(0)?,
                title: row.get(1)?,
                state: row.get(2)?,
                milestone: row.get(3)?,
                labels: serde_json::from_str::<Vec<String>>(&labels_json).unwrap_or_default(),
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

    let issue_count = load_indexed_issues(config)?.len();
    let markdown_indexes = write_markdown_indexes(config)?;
    Ok(MemoryReindexReport {
        issue_count,
        index_path: config.index_path.clone(),
        markdown_indexes,
    })
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
            title: "Malformed timestamps".to_string(),
            state: None,
            milestone: None,
            labels: Vec::new(),
            areas: Vec::new(),
            capsule_path: PathBuf::from(".opensymphony/memory/issues/COE-999.md"),
            visibility: MemoryVisibility::Private,
            source_hash: String::new(),
            warning_count: 0,
            docs_sync_status: "pending".to_string(),
            completion_time: Some("not-a-date".to_string()),
            captured_at: "also-not-a-date".to_string(),
            changed_files: Vec::new(),
            body: String::new(),
        };

        assert_eq!(issue_log_date(&issue), UNDATED_LOG_DATE);
    }
}
