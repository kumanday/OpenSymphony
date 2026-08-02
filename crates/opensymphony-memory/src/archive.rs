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

    fn areas_for_scope(&self, config: &MemoryConfig, scope: &MemoryScopeFilter) -> Vec<String> {
        let repository_id = scope.repo.as_deref();
        let has_project_scope = scope.project.is_some() || scope.project_set.is_some();
        if repository_id.is_none() && !has_project_scope {
            return self.areas();
        }
        if config.repository_sources.is_empty()
            || repository_id.is_some_and(|id| !config.repository_sources.contains_key(id))
            || self.source_scope_refs.is_empty()
        {
            return self.areas();
        }
        self.area_source_ids
            .iter()
            .filter(|(_, source_ids)| {
                let has_qualified_source = source_ids
                    .iter()
                    .any(|source_id| !source_id.is_empty() && source_id != LIVE_CAPTURE_OWNER);
                let issue_has_repository_scope = repository_id.is_some_and(|repository_id| {
                    self.scope_refs.iter().any(|source_scope| {
                        source_scope.kind == KnowledgeScopeKind::Repository
                            && source_scope.id.eq_ignore_ascii_case(repository_id)
                    })
                });
                source_ids.iter().any(|source_id| {
                    let repository_matches = repository_id.is_none_or(|repository_id| {
                        source_id == &format!("__live_capture__:{repository_id}")
                            || source_id.starts_with(&format!("{repository_id}:"))
                            || ((source_id.is_empty() || source_id == LIVE_CAPTURE_OWNER)
                                && !has_qualified_source
                                && issue_has_repository_scope)
                            || self.source_refs.iter().any(|source| {
                                source.repo_id.as_deref() == Some(repository_id)
                                    && (source.registration_source_id.as_deref()
                                        == Some(source_id)
                                        || source.id == *source_id)
                            })
                    });
                    let project_matches = scope.project.as_deref().is_none_or(|project| {
                        let source_scopes = self.source_scope_refs.get(source_id);
                        let has_source_project_scope = source_scopes.is_some_and(|scopes| {
                            scopes
                                .iter()
                                .any(|source_scope| source_scope.kind == KnowledgeScopeKind::Project)
                        });
                        source_scopes.is_some_and(|scopes| {
                            scopes.iter().any(|source_scope| {
                                source_scope.kind == KnowledgeScopeKind::Project
                                    && source_scope.id.eq_ignore_ascii_case(project)
                            })
                        }) || (!has_source_project_scope
                            && (source_id.is_empty() || source_id == LIVE_CAPTURE_OWNER)
                            && !has_qualified_source
                            && issue_has_repository_scope
                            && self.scope_refs.iter().any(|source_scope| {
                                source_scope.kind == KnowledgeScopeKind::Project
                                    && source_scope.id.eq_ignore_ascii_case(project)
                            }))
                    });
                    let project_set_matches =
                        scope.project_set.as_deref().is_none_or(|project_set| {
                            self.source_scope_refs.get(source_id).is_some_and(|scopes| {
                                scopes.iter().any(|source_scope| {
                                    source_scope.kind == KnowledgeScopeKind::ProjectSet
                                        && source_scope.id.eq_ignore_ascii_case(project_set)
                                })
                            })
                        });
                    repository_matches && project_matches && project_set_matches
                })
            })
            .map(|(area, _)| area.clone())
            .collect()
    }
}
