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
    let all_selected_issue_keys = selected
        .iter()
        .map(|issue| issue.issue_key.clone())
        .collect::<BTreeSet<_>>();

    let mut by_area: BTreeMap<String, (AreaConfig, Vec<IndexedIssue>)> = BTreeMap::new();
    let mut unmapped_areas = BTreeSet::new();
    for issue in selected {
        for area in issue.areas() {
            if selection
                .area
                .as_ref()
                .is_some_and(|selected_area| slugify(selected_area) != area)
            {
                continue;
            }
            if let Some(area_config) = config.areas.get(&area) {
                if area_config.status != AreaStatus::Stable
                    || area_config.confidence < config.confidence_threshold
                {
                    unmapped_areas.insert(area);
                    continue;
                }
                by_area
                    .entry(area)
                    .or_insert_with(|| (area_config.clone(), Vec::new()))
                    .1
                    .push(issue.clone());
            } else {
                unmapped_areas.insert(area);
            }
        }
    }
    let mut targets = Vec::new();
    let mut warnings = Vec::new();
    if by_area.is_empty() {
        warnings.push(missing_docs_area_mapping_message(config, unmapped_areas));
    }
    for (_area_slug, (area, issues)) in by_area {
        let before = if area.docs_target.exists() {
            Some(read_to_string(&area.docs_target)?)
        } else {
            None
        };
        let after = render_topic_doc(config, &area, &issues, before.as_deref(), with_diagrams);
        if area.visibility == MemoryVisibility::Public
            && config.docs.deny_private_links
            && contains_private_memory_link(&markdown_visible_text(&after))
        {
            warnings.push(format!(
                "{} would contain private memory links",
                display_path(&config.repo_root, &area.docs_target)
            ));
        }
        let diff = render_diff_stat(before.as_deref().unwrap_or(""), &after, &area.docs_target);
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

    let mut selected_issue_keys = targets
        .iter()
        .flat_map(|target| target.issue_keys.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if selected_issue_keys.is_empty() {
        selected_issue_keys = all_selected_issue_keys.into_iter().collect();
    }

    Ok(DocsSyncPlan {
        write,
        selected_issue_keys,
        targets,
        warnings,
    })
}

fn missing_docs_area_mapping_message(
    config: &MemoryConfig,
    unmapped_areas: BTreeSet<String>,
) -> String {
    if config.areas.is_empty() {
        return "No stable learned docs area found. Run opensymphony memory init or capture issues with stronger Linear/PR evidence.".to_string();
    }

    if unmapped_areas.is_empty() {
        return "Selected issues have no stable docs areas yet; capture kept their memory private until confidence improves.".to_string();
    }

    let areas = unmapped_areas.into_iter().collect::<Vec<_>>().join(", ");
    format!(
        "selected issues only reference candidate or unmapped docs areas ({areas}); capture kept them private until confidence improves."
    )
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
