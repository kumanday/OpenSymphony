pub fn load_source_file(path: impl AsRef<Path>) -> Result<SourceFile, MemoryError> {
    let path = path.as_ref();
    let contents = read_to_string(path)?;
    serde_yaml::from_str::<SourceFile>(&contents).map_err(|source| MemoryError::ParseYaml {
        path: path.to_path_buf(),
        source,
    })
}

pub fn plan_capture(
    config: &MemoryConfig,
    source: &SourceFile,
    selection: &IssueSelection,
    write: bool,
    discover_github: bool,
) -> Result<CapturePlan, MemoryError> {
    if !config.enabled {
        return Err(MemoryError::InvalidInput(
            "memory is disabled in configuration".to_string(),
        ));
    }

    let selected = select_issues(source, selection);
    let warnings = Vec::new();

    if selected.is_empty() {
        return Err(MemoryError::InvalidInput(
            "no issues selected for memory capture".to_string(),
        ));
    }

    let mut plans = Vec::new();
    let indexed = load_indexed_issues(config)?;
    for issue in selected {
        let issue_key = normalize_issue_key(&issue.identifier);
        let mut issue_warnings = Vec::new();
        if issue.title.trim().is_empty() {
            issue_warnings.push("Linear issue title was not available".to_string());
        }
        if issue.url.is_none() {
            issue_warnings.push("Linear issue URL was not available".to_string());
        }

        let mut prs = matched_prs(source, &issue, &issue_key);
        if discover_github {
            match discover_github_prs(&config.repo_root, &issue_key) {
                Ok((discovered, github_warnings)) => {
                    merge_prs(&mut prs, discovered);
                    issue_warnings.extend(github_warnings);
                }
                Err(error) => return Err(MemoryError::InvalidInput(error)),
            }
        }
        if prs.is_empty() {
            issue_warnings.push("no GitHub PR source was matched".to_string());
        }

        let areas = infer_areas(config, source, &issue, &prs);
        let docs_targets = areas
            .iter()
            .map(|area| config.area_or_default(area).docs_target)
            .collect::<Vec<_>>();
        let source_hash = source_hash(&issue, &prs)?;
        let already_captured = indexed
            .iter()
            .any(|indexed| indexed.issue_key.eq_ignore_ascii_case(&issue_key));
        let stale = indexed
            .iter()
            .find(|indexed| indexed.issue_key.eq_ignore_ascii_case(&issue_key))
            .is_some_and(|indexed| indexed.source_hash != source_hash);
        let capsule_path = config.issue_capsule_path(&issue_key);

        plans.push(CaptureIssuePlan {
            issue,
            prs,
            capsule_path,
            areas,
            docs_targets,
            source_hash,
            already_captured,
            stale,
            warnings: issue_warnings,
        });
    }

    plans.sort_by(|left, right| left.issue.identifier.cmp(&right.issue.identifier));

    Ok(CapturePlan {
        write,
        selected: plans,
        warnings,
    })
}

pub fn write_capture_plan(
    config: &MemoryConfig,
    plan: &CapturePlan,
    force: bool,
) -> Result<CaptureWriteReport, MemoryError> {
    let issue_dir = config.memory_root.join("issues");
    create_dir_all(&issue_dir)?;
    create_dir_all(config.index_path.parent().unwrap_or(&config.memory_root))?;

    let mut written_capsules = Vec::new();
    let mut warnings = plan.warnings.clone();
    for issue_plan in &plan.selected {
        let markdown = render_issue_capsule(config, issue_plan)?;
        if issue_plan.capsule_path.exists() {
            let existing = read_to_string(&issue_plan.capsule_path)?;
            if !force && !existing.contains(ISSUE_CAPSULE_BEGIN) {
                return Err(MemoryError::InvalidInput(format!(
                    "{} already exists and does not look generated; rerun with --force to overwrite it",
                    issue_plan.capsule_path.display()
                )));
            }
        }

        write_file(&issue_plan.capsule_path, &markdown)?;
        written_capsules.push(issue_plan.capsule_path.clone());
    }

    let evolved_config = evolve_memory_config(config, plan);
    write_memory_config(&evolved_config)?;
    index_capture_plan(&evolved_config, plan)?;
    let milestone_nodes = write_milestone_nodes(&evolved_config, plan)?;
    let markdown_indexes = if evolved_config.markdown_indexes {
        write_markdown_indexes(&evolved_config)?
    } else {
        Vec::new()
    };

    for issue_plan in &plan.selected {
        warnings.extend(issue_plan.warnings.clone());
    }

    Ok(CaptureWriteReport {
        written_capsules,
        index_path: evolved_config.index_path.clone(),
        markdown_indexes,
        milestone_nodes,
        warnings,
    })
}

pub fn render_capture_dry_run(config: &MemoryConfig, plan: &CapturePlan) -> String {
    let mut output = String::new();
    output.push_str("# Memory Capture Dry Run\n\n");
    output.push_str(&format!(
        "Memory root: {}\n\n",
        display_path(&config.repo_root, &config.memory_root)
    ));
    if plan.selected.is_empty() {
        output.push_str("No issues selected.\n");
        return output;
    }

    output.push_str("## Selected Issues\n\n");
    for issue in &plan.selected {
        output.push_str(&format!(
            "- {}: {}\n",
            issue.issue.identifier,
            issue_title(&issue.issue)
        ));
        output.push_str(&format!(
            "  Capsule: {}\n",
            display_path(&config.repo_root, &issue.capsule_path)
        ));
        output.push_str(&format!(
            "  Linear source: {}\n",
            issue.issue.url.as_deref().unwrap_or("missing")
        ));
        let prs = if issue.prs.is_empty() {
            "none".to_string()
        } else {
            issue
                .prs
                .iter()
                .map(|pr| format!("#{}", pr.number))
                .collect::<Vec<_>>()
                .join(", ")
        };
        output.push_str(&format!("  GitHub PRs: {prs}\n"));
        output.push_str(&format!("  Areas: {}\n", issue.areas.join(", ")));
        output.push_str(&format!(
            "  Docs impact: {}\n",
            issue
                .docs_targets
                .iter()
                .map(|path| display_path(&config.repo_root, path))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        output.push_str(&format!(
            "  Existing capsule: {}\n",
            if issue.already_captured {
                if issue.stale { "stale" } else { "fresh" }
            } else {
                "missing"
            }
        ));
        if !issue.warnings.is_empty() {
            output.push_str("  Warnings:\n");
            for warning in &issue.warnings {
                output.push_str(&format!("  - {warning}\n"));
            }
        }
    }

    if !plan.warnings.is_empty() {
        output.push_str("\n## Plan Warnings\n\n");
        for warning in &plan.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
    }
    output
}
