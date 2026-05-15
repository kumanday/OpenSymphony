pub fn plan_docs_sync(
    config: &MemoryConfig,
    selection: &IssueSelection,
    write: bool,
    with_diagrams: bool,
) -> Result<DocsSyncPlan, MemoryError> {
    let selected = select_indexed_issues_for_docs(config, selection)?;
    if selected.is_empty() {
        return Err(MemoryError::InvalidInput(
            "no captured issues selected for docs sync".to_string(),
        ));
    }

    let mut by_area: BTreeMap<String, Vec<IndexedIssue>> = BTreeMap::new();
    for issue in selected {
        for area in issue.areas() {
            if selection
                .area
                .as_ref()
                .is_some_and(|selected_area| slugify(selected_area) != area)
            {
                continue;
            }
            by_area.entry(area).or_default().push(issue.clone());
        }
    }

    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    for (area_slug, issues) in by_area {
        let area = config.area_or_default(&area_slug);
        let before = if area.docs_target.exists() {
            Some(read_to_string(&area.docs_target)?)
        } else {
            None
        };
        let after = render_topic_doc(config, &area, &issues, before.as_deref(), with_diagrams);
        if area.visibility == MemoryVisibility::Public
            && config.docs.deny_private_links
            && contains_private_memory_link(&after)
        {
            warnings.push(format!(
                "{} would contain private memory links",
                display_path(&config.repo_root, &area.docs_target)
            ));
        }
        let diff = render_diff(before.as_deref().unwrap_or(""), &after, &area.docs_target);
        targets.push(DocsTargetPlan {
            area: area.slug,
            title: area.title,
            path: area.docs_target,
            visibility: area.visibility,
            create: before.is_none(),
            before,
            after,
            diff,
            issue_keys: issues.into_iter().map(|issue| issue.issue_key).collect(),
        });
    }

    let selected_issue_keys = targets
        .iter()
        .flat_map(|target| target.issue_keys.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    Ok(DocsSyncPlan {
        write,
        selected_issue_keys,
        targets,
        warnings,
    })
}

pub fn write_docs_sync_plan(
    config: &MemoryConfig,
    plan: &DocsSyncPlan,
) -> Result<Vec<PathBuf>, MemoryError> {
    let mut written = Vec::new();
    for target in &plan.targets {
        ensure_repo_contained(&config.repo_root, &target.path)?;
        write_file(&target.path, &target.after)?;
        written.push(target.path.clone());
    }
    mark_docs_synced(config, plan)?;
    Ok(written)
}
