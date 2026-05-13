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
