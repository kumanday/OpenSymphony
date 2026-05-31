pub fn plan_archive(
    config: &MemoryConfig,
    identifiers: &[String],
    from_memory: bool,
    state: Option<&str>,
    write: bool,
    force: bool,
) -> Result<ArchivePlan, MemoryError> {
    let issues = load_indexed_issues(config)?;
    let mut selected_keys = identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    if from_memory {
        for issue in &issues {
            if state.is_none_or(|state| archive_state_matches(issue, state)) {
                selected_keys.insert(issue.issue_key.clone());
            }
        }
    }
    if selected_keys.is_empty() {
        return Err(MemoryError::InvalidInput(
            "no Linear issues selected for archive".to_string(),
        ));
    }

    let mut plans = Vec::new();
    let mut warnings = Vec::new();
    for issue_key in selected_keys {
        let indexed = issues
            .iter()
            .find(|issue| issue.issue_key == issue_key)
            .cloned();
        let (eligible, reason, capsule_path) = match indexed {
            Some(issue) if force => (
                true,
                "eligible because --force bypasses capture freshness checks".to_string(),
                Some(issue.capsule_path),
            ),
            Some(issue) if issue.warning_count == 0 => (
                true,
                "eligible: fresh captured memory exists with no unresolved warnings".to_string(),
                Some(issue.capsule_path),
            ),
            Some(issue) => (
                false,
                format!(
                    "blocked: captured memory has {} unresolved warning(s); rerun capture or use --force",
                    issue.warning_count
                ),
                Some(issue.capsule_path),
            ),
            None if force => (
                true,
                "eligible because --force bypasses missing memory checks".to_string(),
                None,
            ),
            None => (
                false,
                "blocked: no captured memory found; run `opensymphony memory capture` first"
                    .to_string(),
                None,
            ),
        };
        if !eligible {
            warnings.push(format!("{issue_key}: {reason}"));
        }
        plans.push(ArchiveIssuePlan {
            issue_key,
            eligible,
            reason,
            capsule_path,
        });
    }

    Ok(ArchivePlan {
        write,
        force,
        issues: plans,
        warnings,
    })
}

fn archive_state_matches(issue: &IndexedIssue, state: &str) -> bool {
    let state = state.trim();
    state.eq_ignore_ascii_case("captured")
        || issue.docs_sync_status.eq_ignore_ascii_case(state)
        || issue
            .state
            .as_deref()
            .is_some_and(|issue_state| issue_state.eq_ignore_ascii_case(state))
}

pub fn render_archive_plan(config: &MemoryConfig, plan: &ArchivePlan) -> String {
    let mut output = String::new();
    if plan.write {
        output.push_str("# Linear Archive Eligibility\n\n");
    } else {
        output.push_str("# Linear Archive Dry Run\n\n");
    }
    for issue in &plan.issues {
        output.push_str(&format!(
            "- {}: {} ({})\n",
            issue.issue_key,
            if issue.eligible {
                "eligible"
            } else {
                "blocked"
            },
            issue.reason
        ));
        if let Some(path) = &issue.capsule_path {
            output.push_str(&format!(
                "  Capsule: {}\n",
                display_path(&config.repo_root, path)
            ));
        }
    }
    if !plan.warnings.is_empty() {
        output.push_str("\n## Warnings\n\n");
        for warning in &plan.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }
    output
}

pub fn mark_archived(config: &MemoryConfig, issue_keys: &[String]) -> Result<(), MemoryError> {
    if !config.index_path.exists() {
        return Ok(());
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
    for issue_key in issue_keys {
        transaction
            .execute(
                "UPDATE issues SET archive_status = 'archived' WHERE issue_key = ?",
                params![normalize_issue_key(issue_key)],
            )
            .map_err(|source| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source,
            })?;
    }
    transaction
        .commit()
        .map_err(|source| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source,
        })?;
    Ok(())
}

pub fn expand_issue_range(range: &str) -> Result<Vec<String>, MemoryError> {
    let Some((start, end)) = range.split_once("..") else {
        return Err(MemoryError::InvalidInput(format!(
            "issue range `{range}` must look like COE-100..COE-199"
        )));
    };
    let (start_prefix, start_number) = split_issue_key(start)?;
    let (end_prefix, end_number) = split_issue_key(end)?;
    if start_prefix != end_prefix {
        return Err(MemoryError::InvalidInput(format!(
            "issue range `{range}` must use the same prefix on both ends"
        )));
    }
    if start_number > end_number {
        return Err(MemoryError::InvalidInput(format!(
            "issue range `{range}` must be ascending"
        )));
    }
    Ok((start_number..=end_number)
        .map(|number| format!("{start_prefix}-{number}"))
        .collect())
}

impl IndexedIssue {
    fn areas(&self) -> Vec<String> {
        self.areas.clone()
    }
}
