pub fn brief(config: &MemoryConfig, issue_key: &str) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let indexed = find_indexed_issue(config, &issue_key)?
        .ok_or_else(|| MemoryError::InvalidInput(format!("no capsule found for {issue_key}")))?;
    let mut output = String::new();
    output.push_str(&format!("# {}: {}\n\n", indexed.issue_key, indexed.title));
    output.push_str(&format!(
        "- Capsule: {}\n",
        display_path(&config.repo_root, &indexed.capsule_path)
    ));
    output.push_str(&format!("- Visibility: {}\n", indexed.visibility));
    if !indexed.areas().is_empty() {
        output.push_str(&format!("- Areas: {}\n", indexed.areas().join(", ")));
    }
    output.push('\n');
    output.push_str(&compact_capsule_body(&indexed.body));
    Ok(output)
}

pub fn search(
    config: &MemoryConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let terms = normalize_query_terms(query);
    if terms.is_empty() {
        return Err(MemoryError::InvalidInput(
            "search query must not be empty".to_string(),
        ));
    }

    let mut scored = Vec::new();
    for indexed in load_indexed_issues(config)? {
        let haystack = format!(
            "{} {} {} {}",
            indexed.issue_key,
            indexed.title,
            indexed.labels.join(" "),
            indexed.body
        )
        .to_ascii_lowercase();
        let score = terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        if score > 0 {
            scored.push((
                score,
                SearchResult {
                    issue_key: indexed.issue_key.clone(),
                    title: indexed.title.clone(),
                    capsule_path: indexed.capsule_path.clone(),
                    areas: indexed.areas(),
                    snippet: snippet_for_terms(&indexed.body, &terms),
                },
            ));
        }
    }
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.issue_key.cmp(&right.1.issue_key))
    });
    Ok(scored
        .into_iter()
        .take(limit.max(1))
        .map(|(_, result)| result)
        .collect())
}

pub fn related_by_issue(
    config: &MemoryConfig,
    issue_key: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let indexed = find_indexed_issue(config, &issue_key)?
        .ok_or_else(|| MemoryError::InvalidInput(format!("no capsule found for {issue_key}")))?;
    let mut related = Vec::new();
    let indexed_areas = indexed.areas();
    for candidate in load_indexed_issues(config)? {
        if candidate.issue_key == issue_key {
            continue;
        }
        let candidate_areas = candidate.areas();
        let overlap = candidate_areas
            .iter()
            .filter(|area| indexed_areas.contains(area))
            .count();
        if overlap > 0 {
            related.push((
                overlap,
                SearchResult {
                    issue_key: candidate.issue_key.clone(),
                    title: candidate.title.clone(),
                    capsule_path: candidate.capsule_path.clone(),
                    areas: candidate_areas,
                    snippet: first_interesting_line(&candidate.body),
                },
            ));
        }
    }
    related.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.issue_key.cmp(&right.1.issue_key))
    });
    Ok(related
        .into_iter()
        .take(limit.max(1))
        .map(|(_, result)| result)
        .collect())
}

pub fn related_by_area(
    config: &MemoryConfig,
    area: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let area = slugify(area);
    let mut results = Vec::new();
    for candidate in load_indexed_issues(config)? {
        let areas = candidate.areas();
        if areas.iter().any(|candidate_area| candidate_area == &area) {
            results.push(SearchResult {
                issue_key: candidate.issue_key.clone(),
                title: candidate.title.clone(),
                capsule_path: candidate.capsule_path.clone(),
                areas,
                snippet: first_interesting_line(&candidate.body),
            });
        }
    }
    results.sort_by(|left, right| left.issue_key.cmp(&right.issue_key));
    results.truncate(limit.max(1));
    Ok(results)
}

pub fn related_by_paths(
    config: &MemoryConfig,
    paths: &[PathBuf],
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    let terms = paths
        .iter()
        .flat_map(|path| {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .filter_map(|value| normalize_optional(&value))
        .collect::<Vec<_>>();
    search(config, &terms.join(" "), limit)
}

pub fn docs_for_area(config: &MemoryConfig, area: &str) -> Result<String, MemoryError> {
    let area = config.area_or_default(area);
    if !area.docs_target.exists() {
        return Err(MemoryError::InvalidInput(format!(
            "no topic doc exists for area `{}` at {}",
            area.slug,
            area.docs_target.display()
        )));
    }
    read_to_string(&area.docs_target)
}

pub fn context_for_issue(
    config: &MemoryConfig,
    source: &SourceFile,
    issue_key: &str,
    limit: usize,
) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let mut output = String::new();
    output.push_str(&format!("# Memory Context: {issue_key}\n\n"));
    if let Some(issue) = source
        .issues
        .iter()
        .find(|issue| normalize_issue_key(&issue.identifier) == issue_key)
    {
        output.push_str(&format!("## Current Issue\n\n{}\n\n", issue_title(issue)));
        if let Some(description) = issue.description.as_deref().and_then(normalize_optional) {
            output.push_str(&format!("{}\n\n", summarize_text(&description, 600)));
        }
    }

    let query = source
        .issues
        .iter()
        .find(|issue| normalize_issue_key(&issue.identifier) == issue_key)
        .map(|issue| {
            format!(
                "{} {} {}",
                issue.title,
                issue.labels.join(" "),
                issue.description.clone().unwrap_or_default()
            )
        })
        .unwrap_or_else(|| issue_key.clone());
    let results = search(config, &query, limit).unwrap_or_default();
    output.push_str("## Related Memory\n\n");
    if results.is_empty() {
        output.push_str("- No related captured memory found.\n");
    } else {
        for result in results {
            output.push_str(&format!(
                "- {}: {} ({})\n",
                result.issue_key,
                result.title,
                result.areas.join(", ")
            ));
        }
    }
    output.push_str("\n## Guidance\n\n");
    output.push_str("- Treat memory as context, not as authority over current code.\n");
    output.push_str("- Inspect the referenced docs and current files before editing.\n");
    output.push_str("- Use `opensymphony debug ");
    output.push_str(&issue_key);
    output.push_str("` only when the original agent conversation is needed.\n");
    Ok(output)
}

pub fn status(
    config: &MemoryConfig,
    selection: &IssueSelection,
) -> Result<StatusReport, MemoryError> {
    let mut issues = load_indexed_issues(config)?;
    if let Some(area) = selection.area.as_ref().map(|area| slugify(area)) {
        issues.retain(|issue| issue.areas().contains(&area));
    }
    if let Some(milestone) = selection
        .milestone
        .as_ref()
        .and_then(|value| normalize_optional(value))
    {
        issues.retain(|issue| issue.milestone.as_deref() == Some(milestone.as_str()));
    }

    let warning_count = issues.iter().map(|issue| issue.warning_count).sum();
    let docs_pending_count = issues
        .iter()
        .filter(|issue| issue.docs_sync_status == "pending")
        .count();
    let status_issues = issues
        .into_iter()
        .map(|issue| {
            let areas = issue.areas();
            StatusIssue {
                issue_key: issue.issue_key,
                title: issue.title,
                state: issue.state,
                milestone: issue.milestone,
                capsule_path: issue.capsule_path,
                visibility: issue.visibility,
                areas,
                docs_sync_status: issue.docs_sync_status,
                warning_count: issue.warning_count,
            }
        })
        .collect::<Vec<_>>();

    Ok(StatusReport {
        issue_count: status_issues.len(),
        warning_count,
        docs_pending_count,
        issues: status_issues,
    })
}

pub fn lint(config: &MemoryConfig, public_docs: bool) -> Result<LintReport, MemoryError> {
    let mut findings = Vec::new();
    let issues = load_indexed_issues(config).unwrap_or_default();
    for issue in &issues {
        if issue.warning_count > 0 {
            findings.push(LintFinding {
                severity: LintSeverity::Warn,
                path: Some(issue.capsule_path.clone()),
                message: format!(
                    "{} has {} unresolved capture warning(s)",
                    issue.issue_key, issue.warning_count
                ),
                next_command: Some(format!("opensymphony memory show {}", issue.issue_key)),
            });
        }
        if issue.areas().is_empty() {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                path: Some(issue.capsule_path.clone()),
                message: format!("{} has no learned memory area", issue.issue_key),
                next_command: Some(format!(
                    "opensymphony memory capture {} --force",
                    issue.issue_key
                )),
            });
        }
    }

    if public_docs && config.docs.deny_private_links {
        for area in all_known_areas(config, &issues) {
            let path = area.docs_target;
            if !path.exists() {
                continue;
            }
            let contents = read_to_string(&path)?;
            if contains_private_memory_link(&contents) {
                findings.push(LintFinding {
                    severity: LintSeverity::Error,
                    path: Some(path),
                    message: "public docs contain a private memory path".to_string(),
                    next_command: Some("opensymphony memory sync-docs".to_string()),
                });
            }
        }
    }

    Ok(LintReport { findings })
}
