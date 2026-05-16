impl MemoryConfig {
    pub fn load(
        repo_root: impl AsRef<Path>,
        config_path: Option<&Path>,
    ) -> Result<Self, MemoryError> {
        let repo_root = normalize_path(repo_root.as_ref());
        let config_file = match config_path {
            Some(path) => Some(resolve_path(&repo_root, path)),
            None => default_config_path(&repo_root),
        };
        let resolved_config_path = config_file
            .clone()
            .unwrap_or_else(|| repo_root.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE));

        let parsed = match config_file {
            Some(path) => {
                let contents = read_to_string(&path)?;
                serde_yaml::from_str::<MemoryConfigFile>(&contents).map_err(|source| {
                    MemoryError::ParseYaml {
                        path: path.clone(),
                        source,
                    }
                })?
            }
            None => MemoryConfigFile::default(),
        };

        let memory_root = resolve_path(
            &repo_root,
            parsed
                .memory_root
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_MEMORY_ROOT)),
        );
        let index_path = parsed
            .index_path
            .as_deref()
            .map(|path| resolve_path(&repo_root, path))
            .unwrap_or_else(|| memory_root.join(DEFAULT_INDEX_FILE_NAME));
        let visibility = parsed.visibility.unwrap_or_default();
        let docs_file = parsed.docs.unwrap_or_default();
        let public_root = resolve_path(
            &repo_root,
            docs_file
                .public_root
                .as_deref()
                .unwrap_or_else(|| Path::new(DEFAULT_PUBLIC_DOCS_ROOT)),
        );
        let default_doc_visibility = docs_file
            .default_visibility
            .unwrap_or(MemoryVisibility::Public);
        let mut areas = BTreeMap::new();
        for (slug, area) in parsed.areas {
            let slug = slugify(&slug);
            areas.insert(
                slug.clone(),
                AreaConfig {
                    title: area.title.unwrap_or_else(|| titleize_slug(&slug)),
                    docs_target: area
                        .docs_target
                        .as_deref()
                        .map(|path| resolve_path(&repo_root, path))
                        .unwrap_or_else(|| public_root.join(format!("{slug}.md"))),
                    visibility: area.visibility.unwrap_or(default_doc_visibility),
                    status: area.status.unwrap_or(AreaStatus::Candidate),
                    confidence: area.confidence.unwrap_or_default(),
                    aliases: normalize_list(area.aliases),
                    source_refs: normalize_area_source_refs(area.source_refs),
                    slug,
                },
            );
        }

        Ok(Self {
            enabled: parsed.enabled.unwrap_or(true),
            config_path: resolved_config_path,
            repo_root,
            memory_root,
            visibility,
            index_path,
            confidence_threshold: parsed.confidence_threshold.unwrap_or(75),
            source_snapshot_policy: parsed.source_snapshots.unwrap_or_default(),
            markdown_indexes: parsed.markdown_indexes.unwrap_or(true),
            docs: DocsConfig {
                public_root,
                default_visibility: default_doc_visibility,
                deny_private_links: docs_file.deny_private_links.unwrap_or(true),
            },
            areas,
            redaction: parsed
                .redaction
                .map_or_else(RedactionConfig::default, |redaction| RedactionConfig {
                    deny_patterns: normalize_list(redaction.deny_patterns),
                }),
        })
    }

    pub fn issue_capsule_path(&self, issue_key: &str) -> PathBuf {
        self.memory_root
            .join("issues")
            .join(format!("{}.md", sanitize_issue_key(issue_key)))
    }

    pub fn area_or_default(&self, slug: &str) -> AreaConfig {
        let slug = slugify(slug);
        self.areas
            .get(&slug)
            .cloned()
            .unwrap_or_else(|| AreaConfig {
                title: titleize_slug(&slug),
                docs_target: self.docs.public_root.join(format!("{slug}.md")),
                visibility: self.docs.default_visibility,
                status: AreaStatus::Candidate,
                confidence: 0,
                aliases: Vec::new(),
                source_refs: AreaSourceRefs::default(),
                slug,
            })
    }
}

pub fn plan_memory_init(
    repo_root: impl AsRef<Path>,
    config_path: Option<&Path>,
    force: bool,
) -> Result<MemoryInitPlan, MemoryError> {
    let repo_root = normalize_path(repo_root.as_ref());
    let config_path = config_path
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| repo_root.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE));
    if config_path.exists() && !force {
        return Err(MemoryError::InvalidInput(format!(
            "{} already exists; use --force to overwrite it",
            display_path(&repo_root, &config_path)
        )));
    }

    let gitignore_path = repo_root.join(".gitignore");
    let gitignore_before = fs::read_to_string(&gitignore_path).ok();
    let gitignore_after = render_memory_gitignore(gitignore_before.as_deref());

    Ok(MemoryInitPlan {
        config_path,
        config_contents: render_memory_init_config(&repo_root)?,
        gitignore_path,
        gitignore_before,
        gitignore_after,
    })
}

pub fn write_memory_init_plan(plan: &MemoryInitPlan) -> Result<(), MemoryError> {
    write_file(&plan.config_path, &plan.config_contents)?;
    write_file(&plan.gitignore_path, &plan.gitignore_after)?;
    Ok(())
}

pub fn ensure_memory_initialized(
    repo_root: impl AsRef<Path>,
    config_path: Option<&Path>,
) -> Result<MemoryInitApplyReport, MemoryError> {
    let repo_root = normalize_path(repo_root.as_ref());
    let config_path = config_path
        .map(|path| resolve_path(&repo_root, path))
        .unwrap_or_else(|| repo_root.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE));
    let config = if config_path.exists() {
        MemoryInitFileChange::Unchanged
    } else {
        write_file(&config_path, &render_memory_init_config(&repo_root)?)?;
        MemoryInitFileChange::Created
    };

    let gitignore_path = repo_root.join(".gitignore");
    let gitignore_before = match fs::read_to_string(&gitignore_path) {
        Ok(contents) => Some(contents),
        Err(source) if source.kind() == io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(MemoryError::ReadFile {
                path: gitignore_path,
                source,
            });
        }
    };
    let gitignore_after = render_memory_gitignore(gitignore_before.as_deref());
    let gitignore = match gitignore_before {
        Some(before) if before == gitignore_after => MemoryInitFileChange::Unchanged,
        Some(_) => {
            write_file(&gitignore_path, &gitignore_after)?;
            MemoryInitFileChange::Updated
        }
        None => {
            write_file(&gitignore_path, &gitignore_after)?;
            MemoryInitFileChange::Created
        }
    };

    Ok(MemoryInitApplyReport {
        config_path,
        config,
        gitignore_path,
        gitignore,
    })
}

fn write_memory_config(config: &MemoryConfig) -> Result<(), MemoryError> {
    let mut areas = BTreeMap::new();
    for (slug, area) in &config.areas {
        areas.insert(
            slug.clone(),
            AreaConfigFile {
                title: Some(area.title.clone()),
                docs_target: Some(PathBuf::from(path_relative_to(
                    &config.repo_root,
                    &area.docs_target,
                ))),
                visibility: Some(area.visibility),
                status: Some(area.status),
                confidence: Some(area.confidence),
                aliases: area.aliases.clone(),
                source_refs: tracked_area_source_refs(&area.source_refs),
            },
        );
    }
    let file = MemoryConfigFile {
        enabled: Some(config.enabled),
        memory_root: Some(PathBuf::from(path_relative_to(
            &config.repo_root,
            &config.memory_root,
        ))),
        visibility: Some(config.visibility),
        index_path: Some(PathBuf::from(path_relative_to(
            &config.repo_root,
            &config.index_path,
        ))),
        confidence_threshold: Some(config.confidence_threshold),
        source_snapshots: Some(config.source_snapshot_policy),
        markdown_indexes: Some(config.markdown_indexes),
        docs: Some(DocsConfigFile {
            public_root: Some(PathBuf::from(path_relative_to(
                &config.repo_root,
                &config.docs.public_root,
            ))),
            default_visibility: Some(config.docs.default_visibility),
            deny_private_links: Some(config.docs.deny_private_links),
        }),
        areas,
        redaction: Some(RedactionConfigFile {
            deny_patterns: config.redaction.deny_patterns.clone(),
        }),
    };
    let contents = serde_yaml::to_string(&file).map_err(|source| MemoryError::ParseYaml {
        path: config.config_path.clone(),
        source,
    })?;
    write_file(&config.config_path, &contents)
}

fn render_memory_init_config(repo_root: &Path) -> Result<String, MemoryError> {
    let mut areas = BTreeMap::new();
    for area in discover_doc_areas(repo_root)? {
        areas.insert(
            area.slug,
            AreaConfigFile {
                title: Some(area.title.clone()),
                docs_target: Some(PathBuf::from(display_path(repo_root, &area.docs_target))),
                visibility: Some(MemoryVisibility::Public),
                status: Some(AreaStatus::Stable),
                confidence: Some(85),
                aliases: vec![area.title],
                source_refs: AreaSourceRefs {
                    docs: vec![display_path(repo_root, &area.docs_target)],
                    ..AreaSourceRefs::default()
                },
            },
        );
    }
    let config = MemoryConfigFile {
        memory_root: Some(PathBuf::from(DEFAULT_MEMORY_ROOT)),
        visibility: Some(MemoryVisibility::Private),
        index_path: Some(PathBuf::from(format!(
            "{DEFAULT_MEMORY_ROOT}/{DEFAULT_INDEX_FILE_NAME}"
        ))),
        confidence_threshold: Some(75),
        source_snapshots: Some(SourceSnapshotPolicy::Hashes),
        markdown_indexes: Some(true),
        docs: Some(DocsConfigFile {
            public_root: Some(PathBuf::from(DEFAULT_PUBLIC_DOCS_ROOT)),
            default_visibility: Some(MemoryVisibility::Public),
            deny_private_links: Some(true),
        }),
        areas,
        ..MemoryConfigFile::default()
    };
    serde_yaml::to_string(&config).map_err(|source| MemoryError::ParseYaml {
        path: repo_root.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE),
        source,
    })
}

struct DiscoveredDocArea {
    slug: String,
    title: String,
    docs_target: PathBuf,
}

fn discover_doc_areas(repo_root: &Path) -> Result<Vec<DiscoveredDocArea>, MemoryError> {
    let docs_root = repo_root.join("docs");
    if !docs_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut areas = BTreeMap::new();
    for entry in fs::read_dir(&docs_root).map_err(|source| MemoryError::ReadFile {
        path: docs_root.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| MemoryError::ReadFile {
            path: docs_root.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        let slug = slugify(stem);
        if slug.is_empty() {
            continue;
        }
        let contents = read_to_string(&path)?;
        let title = first_markdown_heading(&contents).unwrap_or_else(|| titleize_slug(&slug));
        areas.insert(
            slug.clone(),
            DiscoveredDocArea {
                slug,
                title,
                docs_target: path,
            },
        );
    }
    Ok(areas.into_values().collect())
}

fn first_markdown_heading(contents: &str) -> Option<String> {
    for line in contents.lines() {
        let trimmed = line.trim();
        let Some(value) = trimmed.strip_prefix("# ") else {
            continue;
        };
        return normalize_optional(value);
    }
    None
}

fn normalize_area_source_refs(mut refs: AreaSourceRefs) -> AreaSourceRefs {
    refs.docs = normalize_source_ref_list(refs.docs);
    refs.linear_labels = normalize_source_ref_list(refs.linear_labels);
    refs.linear_milestones = normalize_source_ref_list(refs.linear_milestones);
    refs.linear_issues = normalize_source_ref_list(refs.linear_issues);
    refs.github_prs = normalize_source_ref_list(refs.github_prs);
    refs
}

fn tracked_area_source_refs(refs: &AreaSourceRefs) -> AreaSourceRefs {
    AreaSourceRefs {
        docs: refs.docs.clone(),
        linear_labels: refs.linear_labels.clone(),
        linear_milestones: refs.linear_milestones.clone(),
        linear_issues: Vec::new(),
        github_prs: Vec::new(),
    }
}

fn normalize_source_ref_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = values
        .into_iter()
        .filter_map(|value| normalize_optional(&value))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn render_memory_gitignore(before: Option<&str>) -> String {
    const MEMORY_IGNORE_LINES: [&str; 6] = [
        ".opensymphony*",
        "!.opensymphony/",
        ".opensymphony/*",
        "!.opensymphony/memory/",
        ".opensymphony/memory/*",
        "!.opensymphony/memory/memory.yaml",
    ];

    let mut lines = before
        .unwrap_or_default()
        .lines()
        .filter(|line| !MEMORY_IGNORE_LINES.contains(&line.trim()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }

    let mut output = lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    for line in MEMORY_IGNORE_LINES {
        output.push_str(line);
        output.push('\n');
    }
    output
}
