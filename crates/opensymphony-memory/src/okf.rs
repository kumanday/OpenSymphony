pub const OKF_VERSION: &str = "0.1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfBundlePath {
    relative: PathBuf,
}

impl OkfBundlePath {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, MemoryError> {
        let path = path.into();
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::Normal(part) => normalized.push(part),
                _ => {
                    return Err(MemoryError::InvalidInput(format!(
                        "OKF concept path `{}` must be bundle-relative and contained",
                        path.display()
                    )));
                }
            }
        }
        if normalized.as_os_str().is_empty()
            || normalized.extension().and_then(OsStr::to_str) != Some("md")
        {
            return Err(MemoryError::InvalidInput(format!(
                "OKF concept path `{}` must name a Markdown file",
                path.display()
            )));
        }
        Ok(Self {
            relative: normalized,
        })
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
    pub derived_opensymphony: bool,
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
            derived_opensymphony: false,
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
        serde_yaml::from_str(&frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: document_path.to_path_buf(),
            source,
        })?;
    require_okf_type(&frontmatter.concept_type)?;
    let derived_opensymphony = frontmatter.opensymphony.is_none();
    if derived_opensymphony {
        let legacy = legacy_frontmatter_to_opensymphony_metadata(&frontmatter);
        frontmatter.opensymphony = Some(legacy);
    }
    let mut concept = OkfConcept::new(relative_path, frontmatter, body.to_string())?;
    concept.derived_opensymphony = derived_opensymphony;
    Ok(concept)
}

pub fn render_okf_concept(concept: &OkfConcept) -> Result<String, MemoryError> {
    require_okf_type(&concept.frontmatter.concept_type)?;
    OkfBundlePath::new(concept.path.as_path().to_path_buf())?;
    let mut frontmatter = concept.frontmatter.clone();
    if concept.derived_opensymphony {
        frontmatter.opensymphony = None;
    } else if frontmatter.opensymphony.is_some() {
        remove_legacy_opensymphony_fields(&mut frontmatter.extra);
    }
    let frontmatter =
        serde_yaml::to_string(&frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: concept.path.as_path().to_path_buf(),
            source,
        })?;
    Ok(format!("---\n{frontmatter}---\n\n{}", concept.body))
}

fn split_okf_frontmatter(path: &Path, contents: &str) -> Result<(String, String), MemoryError> {
    let normalized = contents.replace("\r\n", "\n").replace('\r', "\n");
    let first_end = normalized
        .find('\n')
        .map(|index| index + 1)
        .unwrap_or(normalized.len());
    if normalized[..first_end].trim() != "---" {
        return Err(MemoryError::InvalidInput(format!(
            "{} lacks OKF YAML frontmatter",
            path.display()
        )));
    };

    let mut offset = first_end;
    let mut frontmatter = String::new();
    while offset < normalized.len() {
        let next_end = normalized[offset..]
            .find('\n')
            .map(|index| offset + index + 1)
            .unwrap_or(normalized.len());
        let line = &normalized[offset..next_end];
        if line.trim_end() == "---" {
            let body = &normalized[next_end..];
            return Ok((
                frontmatter,
                body.strip_prefix('\n').unwrap_or(body).to_string(),
            ));
        }
        frontmatter.push_str(line);
        offset = next_end;
    }

    Err(MemoryError::InvalidInput(format!(
        "{} has unterminated OKF YAML frontmatter",
        path.display()
    )))
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

fn remove_legacy_opensymphony_fields(extra: &mut BTreeMap<String, serde_yaml::Value>) {
    for key in [
        "area",
        "areas",
        "docs_sync",
        "issue",
        "linear_url",
        "milestone",
        "milestone_id",
        "project",
        "project_id",
        "prs",
        "repo",
        "repository",
        "source_refs",
        "visibility",
    ] {
        extra.remove(key);
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
    if let Some(existing) = refs
        .iter_mut()
        .find(|existing| existing.kind == source_ref.kind && existing.id == source_ref.id)
    {
        if existing.url.is_none() {
            existing.url = source_ref.url;
        }
    } else {
        refs.push(source_ref);
    }
}

fn extract_markdown_links(body: &str) -> Vec<OkfLink> {
    let mut links = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let Some((current, next)) = char_at(body, index) else {
            break;
        };
        match current {
            '`' => index = skip_code_span(body, index),
            '\\' => index = skip_escaped_char(body, next),
            '[' if !is_image_marker(body, index) => {
                let Some((label, after_label)) = parse_link_label(body, next) else {
                    index = next;
                    continue;
                };
                let Some(('(', after_open)) = char_at(body, after_label) else {
                    index = after_label;
                    continue;
                };
                let Some((target, after_target)) = parse_link_target(body, after_open) else {
                    index = after_open;
                    continue;
                };
                if !target.is_empty() {
                    links.push(OkfLink {
                        target,
                        label: Some(label).filter(|label| !label.is_empty()),
                    });
                }
                index = after_target;
            }
            _ => index = next,
        }
    }
    links
}

fn char_at(value: &str, index: usize) -> Option<(char, usize)> {
    value[index..]
        .chars()
        .next()
        .map(|character| (character, index + character.len_utf8()))
}

fn skip_escaped_char(value: &str, index: usize) -> usize {
    char_at(value, index)
        .map(|(_, next)| next)
        .unwrap_or(index)
}

fn skip_code_span(value: &str, index: usize) -> usize {
    let mut tick_end = index;
    while let Some(('`', next)) = char_at(value, tick_end) {
        tick_end = next;
    }
    let tick_count = tick_end - index;
    let mut cursor = tick_end;
    while cursor < value.len() {
        if value[cursor..].starts_with(&"`".repeat(tick_count)) {
            return cursor + tick_count;
        }
        let Some((_, next)) = char_at(value, cursor) else {
            break;
        };
        cursor = next;
    }
    tick_end
}

fn is_image_marker(value: &str, index: usize) -> bool {
    value[..index].ends_with('!')
}

fn parse_link_label(value: &str, mut index: usize) -> Option<(String, usize)> {
    let label_start = index;
    let mut depth = 1usize;
    while index < value.len() {
        let (current, next) = char_at(value, index)?;
        match current {
            '\\' => index = skip_escaped_char(value, next),
            '`' => index = skip_code_span(value, index),
            '[' => {
                depth += 1;
                index = next;
            }
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some((value[label_start..index].to_string(), next));
                }
                index = next;
            }
            _ => index = next,
        }
    }
    None
}

fn parse_link_target(value: &str, mut index: usize) -> Option<(String, usize)> {
    let target_start = index;
    let mut depth = 1usize;
    while index < value.len() {
        let (current, next) = char_at(value, index)?;
        match current {
            '\\' => index = skip_escaped_char(value, next),
            '(' => {
                depth += 1;
                index = next;
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return normalize_link_target(&value[target_start..index])
                        .map(|target| (target, next));
                }
                index = next;
            }
            _ => index = next,
        }
    }
    None
}

fn normalize_link_target(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(after_open) = raw.strip_prefix('<')
        && let Some((target, _)) = after_open.split_once('>')
    {
        return Some(target.to_string()).filter(|target| !target.is_empty());
    }
    raw.split_whitespace().next().map(str::to_string)
}
