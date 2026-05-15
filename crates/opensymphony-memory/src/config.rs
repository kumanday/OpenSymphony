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
                    path_hints: normalize_list(area.path_hints),
                    labels: normalize_list(area.labels),
                    slug,
                },
            );
        }

        Ok(Self {
            enabled: parsed.enabled.unwrap_or(true),
            repo_root,
            memory_root,
            visibility,
            index_path,
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
                path_hints: Vec::new(),
                labels: Vec::new(),
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

fn render_memory_init_config(repo_root: &Path) -> Result<String, MemoryError> {
    let areas = discover_task_areas(repo_root)?;
    let areas = if areas.is_empty() {
        vec!["general".to_string()]
    } else {
        areas
    };

    let mut output = String::from(
        "memory_root: .opensymphony/memory\n\
visibility: private\n\
index_path: .opensymphony/memory/memory.duckdb\n\
source_snapshots: hashes\n\
markdown_indexes: true\n\n\
docs:\n\
  public_root: docs\n\
  default_visibility: public\n\
  deny_private_links: true\n\n\
areas:\n",
    );
    for area in areas {
        let docs_target = docs_target_for_area(repo_root, &area);
        output.push_str(&format!("  {area}:\n"));
        output.push_str(&format!("    title: {}\n", titleize_slug(&area)));
        output.push_str(&format!(
            "    docs_target: {}\n",
            display_path(repo_root, &docs_target)
        ));
        output.push_str("    path_hints:\n");
        for hint in path_hints_for_area(&area) {
            output.push_str(&format!("      - {hint}\n"));
        }
        output.push_str("    labels:\n");
        output.push_str(&format!("      - {area}\n"));
    }
    Ok(output)
}

fn discover_task_areas(repo_root: &Path) -> Result<Vec<String>, MemoryError> {
    let tasks_root = repo_root.join("docs/tasks");
    if !tasks_root.is_dir() {
        return Ok(Vec::new());
    }

    let mut areas = BTreeSet::new();
    for entry in fs::read_dir(&tasks_root).map_err(|source| MemoryError::ReadFile {
        path: tasks_root.clone(),
        source,
    })? {
        let entry = entry.map_err(|source| MemoryError::ReadFile {
            path: tasks_root.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("md") {
            continue;
        }
        let contents = read_to_string(&path)?;
        if let Some(area) = parse_task_area(&contents) {
            areas.insert(area);
        }
    }
    Ok(areas.into_iter().collect())
}

fn parse_task_area(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None;
        }
        let Some(value) = trimmed.strip_prefix("area:") else {
            continue;
        };
        return normalize_optional(value).map(|area| slugify(&area));
    }
    None
}

fn docs_target_for_area(repo_root: &Path, area: &str) -> PathBuf {
    repo_root.join("docs").join(format!("{area}.md"))
}

fn path_hints_for_area(area: &str) -> Vec<String> {
    let mut hints = BTreeSet::from([area.to_string()]);
    for token in area.split('-').filter(|token| !token.is_empty()) {
        hints.insert(token.to_string());
    }
    hints.into_iter().collect()
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
