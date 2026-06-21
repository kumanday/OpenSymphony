pub const OKF_VERSION: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfBundlePath {
    relative: PathBuf,
}

impl OkfBundlePath {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let path = path.into();
        let mut has_component = false;
        for component in path.components() {
            match component {
                std::path::Component::Normal(_) => has_component = true,
                _ => {
                    return Err(MemoryError::InvalidInput(format!(
                        "OKF concept path `{}` must be bundle-relative and contained",
                        path.display()
                    )));
                }
            }
        }
        if !has_component || path.extension().and_then(OsStr::to_str) != Some("md") {
            return Err(MemoryError::InvalidInput(format!(
                "OKF concept path `{}` must name a Markdown file",
                path.display()
            )));
        }
        Ok(Self { relative: path })
    }

    pub fn as_path(&self) -> &Path {
        &self.relative
    }

    pub fn concept_id(&self) -> String {
        let mut id = self.relative.clone();
        id.set_extension("");
        id.components()
            .filter_map(|component| match component {
                std::path::Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    pub fn reserved_file(&self) -> Option<OkfReservedFile> {
        match self.relative.file_name().and_then(OsStr::to_str) {
            Some("index.md") => Some(OkfReservedFile::Index),
            Some("log.md") => Some(OkfReservedFile::Log),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OkfReservedFile {
    Index,
    Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OkfFrontmatter {
    #[serde(rename = "type")]
    pub concept_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opensymphony: Option<OpenSymphonyOkfMetadata>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl OkfFrontmatter {
    pub fn new(concept_type: impl Into<String>) -> Result<Self, MemoryError> {
        let concept_type = concept_type.into();
        require_okf_type(&concept_type)?;
        Ok(Self {
            concept_type,
            title: None,
            description: None,
            resource: None,
            tags: Vec::new(),
            timestamp: None,
            opensymphony: None,
            extra: BTreeMap::new(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenSymphonyOkfMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<MemoryVisibility>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_refs: Vec<KnowledgeScope>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<MemorySourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<OkfLink>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<OkfCitation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_sync: Option<serde_yaml::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkfLink {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkfCitation {
    pub id: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OkfConcept {
    pub path: OkfBundlePath,
    pub id: String,
    pub frontmatter: OkfFrontmatter,
    pub body: String,
    pub links: Vec<OkfLink>,
}

impl OkfConcept {
    pub fn new(
        path: impl Into<PathBuf>,
        frontmatter: OkfFrontmatter,
        body: impl Into<String>,
    ) -> Result<Self, MemoryError> {
        require_okf_type(&frontmatter.concept_type)?;
        let path = OkfBundlePath::new(path)?;
        let body = body.into();
        Ok(Self {
            id: path.concept_id(),
            links: extract_markdown_links(&body),
            path,
            frontmatter,
            body,
        })
    }
}

pub fn parse_okf_concept(
    bundle_root: &Path,
    document_path: &Path,
    contents: &str,
) -> Result<OkfConcept, MemoryError> {
    let relative_path = if document_path.is_absolute() {
        document_path
            .strip_prefix(bundle_root)
            .map_err(|_| MemoryError::PathOutsideRepo {
                path: document_path.to_path_buf(),
                repo_root: bundle_root.to_path_buf(),
            })?
            .to_path_buf()
    } else {
        document_path.to_path_buf()
    };
    let (frontmatter, body) = split_okf_frontmatter(document_path, contents)?;
    let mut frontmatter: OkfFrontmatter =
        serde_yaml::from_str(frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: document_path.to_path_buf(),
            source,
        })?;
    require_okf_type(&frontmatter.concept_type)?;
    let legacy = legacy_frontmatter_to_opensymphony_metadata(&frontmatter);
    merge_opensymphony_metadata(&mut frontmatter, legacy);
    OkfConcept::new(relative_path, frontmatter, body.to_string())
}

pub fn render_okf_concept(concept: &OkfConcept) -> Result<String, MemoryError> {
    require_okf_type(&concept.frontmatter.concept_type)?;
    OkfBundlePath::new(concept.path.as_path().to_path_buf())?;
    let frontmatter =
        serde_yaml::to_string(&concept.frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: concept.path.as_path().to_path_buf(),
            source,
        })?;
    Ok(format!("---\n{frontmatter}---\n\n{}", concept.body))
}

fn split_okf_frontmatter<'a>(
    path: &Path,
    contents: &'a str,
) -> Result<(&'a str, &'a str), MemoryError> {
    let Some(after_open) = contents.strip_prefix("---\n") else {
        return Err(MemoryError::InvalidInput(format!(
            "{} lacks OKF YAML frontmatter",
            path.display()
        )));
    };
    let Some((frontmatter, body)) = after_open.split_once("\n---\n") else {
        return Err(MemoryError::InvalidInput(format!(
            "{} has unterminated OKF YAML frontmatter",
            path.display()
        )));
    };
    Ok((frontmatter, body.strip_prefix('\n').unwrap_or(body)))
}

fn require_okf_type(concept_type: &str) -> Result<(), MemoryError> {
    if concept_type.trim().is_empty() {
        Err(MemoryError::InvalidInput(
            "OKF concept frontmatter requires non-empty `type`".to_string(),
        ))
    } else {
        Ok(())
    }
}

fn legacy_frontmatter_to_opensymphony_metadata(
    frontmatter: &OkfFrontmatter,
) -> OpenSymphonyOkfMetadata {
    let mut metadata = OpenSymphonyOkfMetadata {
        visibility: legacy_visibility(frontmatter),
        kind: Some(frontmatter.concept_type.replace('-', "_")),
        schema_version: Some(1),
        scope_refs: Vec::new(),
        source_refs: Vec::new(),
        links: Vec::new(),
        citations: Vec::new(),
        docs_sync: frontmatter.extra.get("docs_sync").cloned(),
        extra: BTreeMap::new(),
    };

    push_scope(
        &mut metadata.scope_refs,
        KnowledgeScopeKind::WorkItem,
        string_extra(frontmatter, "issue"),
        frontmatter.title.clone(),
    );
    push_scope(
        &mut metadata.scope_refs,
        KnowledgeScopeKind::Milestone,
        string_extra(frontmatter, "milestone_id").or_else(|| string_extra(frontmatter, "milestone")),
        string_extra(frontmatter, "milestone"),
    );
    push_scope(
        &mut metadata.scope_refs,
        KnowledgeScopeKind::Project,
        string_extra(frontmatter, "project_id").or_else(|| string_extra(frontmatter, "project")),
        string_extra(frontmatter, "project"),
    );
    for area in string_array_extra(frontmatter, "areas")
        .into_iter()
        .chain(string_extra(frontmatter, "area"))
    {
        push_scope(
            &mut metadata.scope_refs,
            KnowledgeScopeKind::Area,
            Some(area.clone()),
            Some(area),
        );
    }
    push_scope(
        &mut metadata.scope_refs,
        KnowledgeScopeKind::Repository,
        string_extra(frontmatter, "repository").or_else(|| string_extra(frontmatter, "repo")),
        string_extra(frontmatter, "repository").or_else(|| string_extra(frontmatter, "repo")),
    );

    if let Some(issue) = string_extra(frontmatter, "issue") {
        metadata.source_refs.push(MemorySourceRef {
            kind: "linear_issue".to_string(),
            id: issue,
            url: string_extra(frontmatter, "linear_url"),
        });
    }
    for source_ref in legacy_source_refs(frontmatter) {
        push_source_ref(&mut metadata.source_refs, source_ref);
    }

    metadata
}

fn merge_opensymphony_metadata(
    frontmatter: &mut OkfFrontmatter,
    legacy: OpenSymphonyOkfMetadata,
) {
    match &mut frontmatter.opensymphony {
        Some(existing) => {
            if existing.visibility.is_none() {
                existing.visibility = legacy.visibility;
            }
            if existing.kind.is_none() {
                existing.kind = legacy.kind;
            }
            if existing.schema_version.is_none() {
                existing.schema_version = legacy.schema_version;
            }
            for scope_ref in legacy.scope_refs {
                push_scope_ref(&mut existing.scope_refs, scope_ref);
            }
            for source_ref in legacy.source_refs {
                push_source_ref(&mut existing.source_refs, source_ref);
            }
            if existing.docs_sync.is_none() {
                existing.docs_sync = legacy.docs_sync;
            }
        }
        None => frontmatter.opensymphony = Some(legacy),
    }
}

fn legacy_visibility(frontmatter: &OkfFrontmatter) -> Option<MemoryVisibility> {
    string_extra(frontmatter, "visibility").and_then(|value| match value.as_str() {
        "public" => Some(MemoryVisibility::Public),
        "private" => Some(MemoryVisibility::Private),
        _ => None,
    })
}

fn string_extra(frontmatter: &OkfFrontmatter, key: &str) -> Option<String> {
    frontmatter.extra.get(key).and_then(value_as_string)
}

fn string_array_extra(frontmatter: &OkfFrontmatter, key: &str) -> Vec<String> {
    match frontmatter.extra.get(key) {
        Some(serde_yaml::Value::Sequence(values)) => {
            values.iter().filter_map(value_as_string).collect()
        }
        Some(value) => value_as_string(value).into_iter().collect(),
        None => Vec::new(),
    }
}

fn value_as_string(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
        serde_yaml::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn push_scope(
    refs: &mut Vec<KnowledgeScope>,
    kind: KnowledgeScopeKind,
    id: Option<String>,
    label: Option<String>,
) {
    let Some(id) = id.filter(|value| !value.trim().is_empty()) else {
        return;
    };
    push_scope_ref(refs, KnowledgeScope { kind, id, label });
}

fn push_scope_ref(refs: &mut Vec<KnowledgeScope>, scope_ref: KnowledgeScope) {
    if !refs
        .iter()
        .any(|existing| existing.kind == scope_ref.kind && existing.id == scope_ref.id)
    {
        refs.push(scope_ref);
    }
}

fn legacy_source_refs(frontmatter: &OkfFrontmatter) -> Vec<MemorySourceRef> {
    let mut refs = Vec::new();
    if let Some(serde_yaml::Value::Mapping(source_refs)) = frontmatter.extra.get("source_refs") {
        for (key, value) in source_refs {
            let Some(kind) = value_as_string(key) else {
                continue;
            };
            match value {
                serde_yaml::Value::Sequence(values) => {
                    for value in values {
                        if let Some(token) = value_as_string(value) {
                            push_source_ref(&mut refs, source_ref_from_token(&kind, &token));
                        }
                    }
                }
                value => {
                    if let Some(token) = value_as_string(value) {
                        push_source_ref(&mut refs, source_ref_from_token(&kind, &token));
                    }
                }
            }
        }
    }
    if let Some(serde_yaml::Value::Sequence(prs)) = frontmatter.extra.get("prs") {
        for pr in prs {
            let serde_yaml::Value::Mapping(pr) = pr else {
                continue;
            };
            let number = pr
                .get(serde_yaml::Value::String("number".to_string()))
                .and_then(value_as_string);
            if let Some(number) = number {
                let url = pr
                    .get(serde_yaml::Value::String("url".to_string()))
                    .and_then(value_as_string);
                push_source_ref(
                    &mut refs,
                    MemorySourceRef {
                        kind: "github_pr".to_string(),
                        id: number,
                        url,
                    },
                );
            }
            if let Some(sha) = pr
                .get(serde_yaml::Value::String("merge_sha".to_string()))
                .and_then(value_as_string)
            {
                push_source_ref(
                    &mut refs,
                    MemorySourceRef {
                        kind: "github_merge_sha".to_string(),
                        id: sha,
                        url: None,
                    },
                );
            }
        }
    }
    refs
}

fn source_ref_from_token(kind: &str, token: &str) -> MemorySourceRef {
    if let Some(id) = token.strip_prefix("github:pr:") {
        return MemorySourceRef {
            kind: "github_pr".to_string(),
            id: id.to_string(),
            url: None,
        };
    }
    if let Some(id) = token.strip_prefix("github:merge:") {
        return MemorySourceRef {
            kind: "github_merge_sha".to_string(),
            id: id.to_string(),
            url: None,
        };
    }
    if let Some(id) = token.strip_prefix("linear:") {
        return MemorySourceRef {
            kind: kind.to_string(),
            id: id.to_string(),
            url: None,
        };
    }
    MemorySourceRef {
        kind: kind.to_string(),
        id: token.to_string(),
        url: None,
    }
}

fn push_source_ref(refs: &mut Vec<MemorySourceRef>, source_ref: MemorySourceRef) {
    if !refs
        .iter()
        .any(|existing| existing.kind == source_ref.kind && existing.id == source_ref.id)
    {
        refs.push(source_ref);
    }
}

fn extract_markdown_links(body: &str) -> Vec<OkfLink> {
    let mut links = Vec::new();
    let mut rest = body;
    while let Some(label_start) = rest.find('[') {
        rest = &rest[label_start + 1..];
        let Some(label_end) = rest.find("](") else {
            continue;
        };
        let label = &rest[..label_end];
        rest = &rest[label_end + 2..];
        let Some(target_end) = rest.find(')') else {
            break;
        };
        let target = &rest[..target_end];
        if !target.trim().is_empty() {
            links.push(OkfLink {
                target: target.to_string(),
                label: Some(label.to_string()).filter(|label| !label.is_empty()),
            });
        }
        rest = &rest[target_end + 1..];
    }
    links
}
