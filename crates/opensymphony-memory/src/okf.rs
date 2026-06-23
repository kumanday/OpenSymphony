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
        let markdown_extension = normalized
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
        if normalized.as_os_str().is_empty() || !markdown_extension {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct OkfLintReport {
    findings: Vec<OkfLintFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OkfLintFinding {
    code: Option<LintCode>,
    severity: LintSeverity,
    path: Option<PathBuf>,
    message: String,
    next_command: Option<String>,
}

impl OkfLintFinding {
    fn into_public(self) -> LintFinding {
        LintFinding {
            severity: self.severity,
            path: self.path,
            message: self.message,
            next_command: self.next_command,
        }
    }
}

fn okf_lint_finding(
    code: Option<LintCode>,
    severity: LintSeverity,
    path: Option<PathBuf>,
    message: impl Into<String>,
    next_command: Option<String>,
) -> OkfLintFinding {
    OkfLintFinding {
        code,
        severity,
        path,
        message: message.into(),
        next_command,
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfExportReport {
    pub output_path: PathBuf,
    pub copied_files: Vec<PathBuf>,
    pub skipped_private_files: Vec<PathBuf>,
    pub finding_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfImportReport {
    pub source_path: PathBuf,
    pub target_path: PathBuf,
    pub copied_files: Vec<PathBuf>,
    pub finding_count: usize,
    pub reindex: MemoryReindexReport,
}

struct OkfPendingFile {
    relative: PathBuf,
    target: PathBuf,
    contents: String,
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
    } else if let Some(metadata) = frontmatter.opensymphony.clone() {
        remove_represented_legacy_opensymphony_fields(&mut frontmatter.extra, &metadata);
    }
    let frontmatter =
        serde_yaml::to_string(&frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: concept.path.as_path().to_path_buf(),
            source,
        })?;
    Ok(format!("---\n{frontmatter}---\n\n{}", concept.body))
}

pub fn export_okf_bundle(
    config: &MemoryConfig,
    visibility: MemoryVisibility,
    output: Option<&Path>,
) -> Result<OkfExportReport, MemoryError> {
    ensure_repo_contained(&config.repo_root, &config.memory_root)?;
    let source_root = canonicalize_existing_path(&config.memory_root)?;
    if !source_root.is_dir() {
        return Err(MemoryError::InvalidInput(format!(
            "OKF source bundle `{}` is not a directory",
            source_root.display()
        )));
    }
    let output_path = output
        .map(|path| resolve_path(&config.repo_root, path))
        .unwrap_or_else(|| config.repo_root.join(format!("okf-export-{visibility}")));
    ensure_repo_contained(&config.repo_root, &output_path)?;
    ensure_export_output_has_no_symlink_components(config, &output_path)?;
    let output_path = canonicalize_existing_prefix(&output_path)?;
    ensure_output_target_not_symlink(&output_path)?;
    if paths_overlap(&output_path, &source_root) {
        return Err(MemoryError::InvalidInput(format!(
            "OKF export output `{}` must not overlap the source bundle `{}`",
            output_path.display(),
            source_root.display()
        )));
    }
    ensure_empty_output_target(&output_path)?;

    let mut files = Vec::new();
    collect_okf_markdown_files(&source_root, &source_root, &mut files)?;
    let public = visibility == MemoryVisibility::Public;
    if !public {
        let lint = lint_okf_bundle_with_codes(&source_root, false)?;
        let errors = filtered_lint_errors(config, "OKF source bundle", &lint, true);
        if !errors.is_empty() {
            return Err(MemoryError::InvalidInput(errors));
        }
    }
    let output_parent = output_path.parent().ok_or_else(|| {
        MemoryError::InvalidInput(format!(
            "OKF export output `{}` has no parent directory",
            output_path.display()
        ))
    })?;

    let mut writes = Vec::<(PathBuf, String)>::new();
    let mut copied_files = Vec::new();
    let mut skipped_private_files = Vec::new();

    for path in files {
        let relative = bundle_relative_path(&source_root, &path)?;
        let bundle_path = OkfBundlePath::new(relative.clone())?;
        let contents = read_to_string(&path)?;
        if bundle_path.reserved_file().is_some() {
            if !public {
                writes.push((relative.clone(), contents));
                copied_files.push(relative);
            }
            continue;
        }

        if public {
            match raw_okf_visibility(&contents) {
                Some(MemoryVisibility::Public) => {}
                Some(MemoryVisibility::Private) | None => {
                    skipped_private_files.push(relative);
                    continue;
                }
            }
        }

        let concept = parse_okf_concept(&source_root, &path, &contents)?;
        if public && concept_visibility(&concept) == Some(MemoryVisibility::Private) {
            skipped_private_files.push(relative);
            continue;
        }
        writes.push((relative.clone(), contents));
        copied_files.push(relative);
    }

    if public {
        writes.push((PathBuf::from("index.md"), public_export_index()));
        writes.push((PathBuf::from("log.md"), public_export_log()));
        copied_files.push(PathBuf::from("index.md"));
        copied_files.push(PathBuf::from("log.md"));
    }
    create_dir_all(output_parent)?;
    let staging_path = create_staging_dir(output_parent, &output_path)?;
    let writes = writes
        .into_iter()
        .map(|(relative, contents)| (staging_path.join(relative), contents))
        .collect::<Vec<_>>();
    let exported_lint = match write_and_lint_staged_export(config, &staging_path, writes, public) {
        Ok(report) => report,
        Err(error) => return Err(cleanup_staging_after_export_failure(&staging_path, error)),
    };
    promote_staged_export(&staging_path, &output_path)?;

    Ok(OkfExportReport {
        output_path,
        copied_files,
        skipped_private_files,
        finding_count: exported_lint.findings.len(),
    })
}

pub fn import_okf_bundle(
    config: &MemoryConfig,
    source: &Path,
    force: bool,
) -> Result<OkfImportReport, MemoryError> {
    let source_path = canonicalize_existing_path(&resolve_path(&config.repo_root, source))?;
    ensure_repo_contained(&config.repo_root, &source_path)?;
    ensure_repo_contained(&config.repo_root, &config.memory_root)?;
    create_dir_all(&config.memory_root)?;
    let target_root = canonicalize_existing_path(&config.memory_root)?;
    if !source_path.is_dir() {
        return Err(MemoryError::InvalidInput(format!(
            "OKF import source `{}` is not a directory",
            source_path.display()
        )));
    }
    if paths_overlap(&source_path, &target_root) {
        return Err(MemoryError::InvalidInput(format!(
            "OKF import source `{}` must not overlap the target memory root `{}`",
            source_path.display(),
            target_root.display()
        )));
    }
    let target_lint = lint_okf_bundle_with_codes(&target_root, false)?;
    let target_errors = filtered_lint_errors(config, "OKF import target", &target_lint, true);
    if !target_errors.is_empty() {
        return Err(MemoryError::InvalidInput(target_errors));
    }
    let lint = lint_okf_bundle_with_codes(&source_path, false)?;
    let errors = filtered_lint_errors(config, "OKF import bundle", &lint, true);
    if !errors.is_empty() {
        return Err(MemoryError::InvalidInput(errors));
    }

    let mut files = Vec::new();
    collect_okf_markdown_files(&source_path, &source_path, &mut files)?;
    let pending = pending_okf_import_files(config, &source_path, &target_root, &files, force)?;
    let copied_files = pending
        .iter()
        .map(|file| file.relative.clone())
        .collect::<Vec<_>>();
    for file in pending {
        write_file(&file.target, &file.contents)?;
    }

    let reindex = refresh_memory_index_from_okf(config, &target_root)?;
    Ok(OkfImportReport {
        source_path,
        target_path: target_root,
        copied_files,
        finding_count: lint.findings.len(),
        reindex,
    })
}

fn pending_okf_import_files(
    config: &MemoryConfig,
    source_path: &Path,
    target_root: &Path,
    files: &[PathBuf],
    force: bool,
) -> Result<Vec<OkfPendingFile>, MemoryError> {
    let mut pending = Vec::new();
    for path in files {
        let relative = bundle_relative_path(source_path, path)?;
        let bundle_path = OkfBundlePath::new(relative.clone())?;
        if bundle_path.reserved_file().is_some() {
            continue;
        }
        let contents = read_to_string(path)?;
        let target = target_root.join(&relative);
        ensure_import_target_has_no_symlink_components(config, target_root, &target)?;
        if target.is_dir() {
            return Err(MemoryError::InvalidInput(format!(
                "{} already exists as a directory and cannot be overwritten by OKF import",
                display_path(&config.repo_root, &target)
            )));
        }
        if let Some(parent) = target.parent()
            && parent.exists()
            && !parent.is_dir()
        {
            return Err(MemoryError::InvalidInput(format!(
                "{} already exists and blocks OKF import",
                display_path(&config.repo_root, parent)
            )));
        }
        if target.exists() && !force {
            return Err(MemoryError::InvalidInput(format!(
                "{} already exists; rerun with --force to overwrite it",
                display_path(&config.repo_root, &target)
            )));
        }
        pending.push(OkfPendingFile {
            relative,
            target,
            contents,
        });
    }
    Ok(pending)
}

fn ensure_import_target_has_no_symlink_components(
    config: &MemoryConfig,
    target_root: &Path,
    target: &Path,
) -> Result<(), MemoryError> {
    let relative = target
        .strip_prefix(target_root)
        .map_err(|_| MemoryError::PathOutsideRepo {
            path: target.to_path_buf(),
            repo_root: config.repo_root.clone(),
        })?;
    let mut cursor = target_root.to_path_buf();
    for component in relative.components() {
        cursor.push(component.as_os_str());
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(MemoryError::InvalidInput(format!(
                    "{} is a symlink and cannot be overwritten by OKF import",
                    display_path(&config.repo_root, &cursor)
                )));
            }
            Ok(_) => {}
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                break;
            }
            Err(source) => {
                return Err(MemoryError::ReadFile {
                    path: cursor,
                    source,
                });
            }
        }
    }
    Ok(())
}

pub fn lint_okf_bundle(bundle_root: &Path, public_export: bool) -> Result<LintReport, MemoryError> {
    let report = lint_okf_bundle_with_codes(bundle_root, public_export)?;
    Ok(LintReport {
        findings: report
            .findings
            .into_iter()
            .map(OkfLintFinding::into_public)
            .collect(),
    })
}

fn lint_okf_bundle_with_codes(
    bundle_root: &Path,
    public_export: bool,
) -> Result<OkfLintReport, MemoryError> {
    if !bundle_root.is_dir() {
        return Err(MemoryError::InvalidInput(format!(
            "OKF bundle root `{}` is not a directory",
            bundle_root.display()
        )));
    }

    let mut files = Vec::new();
    collect_okf_markdown_files(bundle_root, bundle_root, &mut files)?;

    let mut findings = Vec::new();
    let mut dirs_with_concepts = BTreeSet::new();
    let mut dirs_with_index = BTreeSet::new();

    for path in files {
        let relative = bundle_relative_path(bundle_root, &path)?;
        let bundle_path = match OkfBundlePath::new(relative.clone()) {
            Ok(path) => path,
            Err(error) => {
                findings.push(okf_lint_finding(
                    None,
                    LintSeverity::Error,
                    Some(path),
                    error.to_string(),
                    None,
                ));
                continue;
            }
        };
        let directory = relative.parent().unwrap_or(Path::new("")).to_path_buf();
        if bundle_path.reserved_file() == Some(OkfReservedFile::Index) {
            dirs_with_index.insert(directory);
        } else if bundle_path.reserved_file().is_none() {
            dirs_with_concepts.insert(directory);
        }

        let contents = read_to_string(&path)?;
        match bundle_path.reserved_file() {
            Some(OkfReservedFile::Index) => {
                lint_okf_index(bundle_root, &path, &relative, &contents, &mut findings)
            }
            Some(OkfReservedFile::Log) => lint_okf_log(&path, &contents, &mut findings),
            None => lint_okf_concept(bundle_root, &path, &contents, public_export, &mut findings),
        }
    }

    for directory in dirs_with_concepts {
        if !dirs_with_index.contains(&directory) {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Warn,
                Some(bundle_root.join(&directory)),
                "missing generated index.md".to_string(),
                Some("opensymphony memory reindex".to_string()),
            ));
        }
    }

    Ok(OkfLintReport { findings })
}

fn ensure_empty_output_target(path: &Path) -> Result<(), MemoryError> {
    if path.exists() {
        if !path.is_dir() {
            return Err(MemoryError::InvalidInput(format!(
                "OKF export output `{}` exists and is not a directory",
                path.display()
            )));
        }
        let mut entries = fs::read_dir(path).map_err(|source| MemoryError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        if entries.next().is_some() {
            return Err(MemoryError::InvalidInput(format!(
                "OKF export output `{}` must be empty",
                path.display()
            )));
        }
    }
    Ok(())
}

fn ensure_output_target_not_symlink(path: &Path) -> Result<(), MemoryError> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(MemoryError::InvalidInput(format!(
            "OKF export output `{}` must not be a symlink",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_export_output_has_no_symlink_components(
    config: &MemoryConfig,
    path: &Path,
) -> Result<(), MemoryError> {
    let repo_root = canonicalize_existing_path(&config.repo_root)?;
    let output_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    let relative = output_path
        .strip_prefix(&repo_root)
        .map_err(|_| MemoryError::PathOutsideRepo {
            path: output_path.clone(),
            repo_root: repo_root.clone(),
        })?;
    let mut cursor = repo_root;
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => continue,
            std::path::Component::Normal(part) => cursor.push(part),
            _ => {
                return Err(MemoryError::PathOutsideRepo {
                    path: output_path,
                    repo_root: config.repo_root.clone(),
                });
            }
        }
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(MemoryError::InvalidInput(format!(
                    "OKF export output `{}` must not include symlink component `{}`",
                    display_path(&config.repo_root, path),
                    display_path(&config.repo_root, &cursor)
                )));
            }
            Ok(_) => {}
            Err(source)
                if matches!(
                    source.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                break;
            }
            Err(source) => {
                return Err(MemoryError::ReadFile {
                    path: cursor,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn create_staging_dir(parent: &Path, output_path: &Path) -> Result<PathBuf, MemoryError> {
    let output_name = output_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("okf-export");
    for attempt in 0..100 {
        let candidate = parent.join(format!(
            ".{output_name}.tmp-{}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(MemoryError::CreateDir {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Err(MemoryError::InvalidInput(format!(
        "could not allocate a staging directory for OKF export `{}`",
        output_path.display()
    )))
}

fn write_and_lint_staged_export(
    config: &MemoryConfig,
    staging_path: &Path,
    writes: Vec<(PathBuf, String)>,
    public: bool,
) -> Result<OkfLintReport, MemoryError> {
    for (path, contents) in writes {
        write_file(&path, &contents)?;
    }
    let exported_lint = lint_okf_bundle_with_codes(staging_path, public)?;
    let errors = filtered_lint_errors(config, "exported OKF bundle", &exported_lint, !public);
    if errors.is_empty() {
        Ok(exported_lint)
    } else {
        Err(MemoryError::InvalidInput(errors))
    }
}

fn promote_staged_export(staging_path: &Path, output_path: &Path) -> Result<(), MemoryError> {
    promote_staged_export_with(
        staging_path,
        output_path,
        |from, to| fs::rename(from, to),
        |path| fs::remove_dir_all(path),
    )
}

fn promote_staged_export_with<R, D>(
    staging_path: &Path,
    output_path: &Path,
    mut rename: R,
    mut remove_dir_all: D,
) -> Result<(), MemoryError>
where
    R: FnMut(&Path, &Path) -> io::Result<()>,
    D: FnMut(&Path) -> io::Result<()>,
{
    let backup_path = if output_path.exists() {
        let backup_path = create_promotion_backup_path(output_path)?;
        rename(output_path, &backup_path).map_err(|source| {
            promotion_failure(
                output_path,
                staging_path,
                None,
                source,
                "backing up existing output",
            )
        })?;
        Some(backup_path)
    } else {
        None
    };

    if let Err(source) = rename(staging_path, output_path) {
        let rollback = if let Some(backup_path) = backup_path.as_ref() {
            match rename(backup_path, output_path) {
                Ok(()) => format!(
                    "previous output was restored from `{}`",
                    backup_path.display()
                ),
                Err(rollback_source) => format!(
                    "previous output remains at `{}` because rollback failed: {}",
                    backup_path.display(),
                    rollback_source
                ),
            }
        } else {
            "no previous output existed".to_string()
        };
        return Err(promotion_failure(
            output_path,
            staging_path,
            Some(rollback),
            source,
            "moving staged bundle into place",
        ));
    }

    if let Some(backup_path) = backup_path
        && backup_path.exists()
    {
        remove_dir_all(&backup_path).map_err(|source| {
            MemoryError::InvalidInput(format!(
                "OKF export promoted to `{}` but failed to remove backup `{}`: {}; remove the backup manually",
                output_path.display(),
                backup_path.display(),
                source
            ))
        })?;
    }
    Ok(())
}

fn create_promotion_backup_path(output_path: &Path) -> Result<PathBuf, MemoryError> {
    let parent = output_path.parent().ok_or_else(|| {
        MemoryError::InvalidInput(format!(
            "OKF export output `{}` has no parent directory",
            output_path.display()
        ))
    })?;
    let output_name = output_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("okf-export");
    for attempt in 0..100 {
        let candidate = parent.join(format!(
            ".{output_name}.backup-{}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(MemoryError::InvalidInput(format!(
        "could not allocate a backup directory for OKF export `{}`",
        output_path.display()
    )))
}

fn promotion_failure(
    output_path: &Path,
    staging_path: &Path,
    recovery_detail: Option<String>,
    source: io::Error,
    action: &str,
) -> MemoryError {
    let recovery_detail = recovery_detail
        .map(|detail| format!("; {detail}"))
        .unwrap_or_default();
    MemoryError::InvalidInput(format!(
        "failed to promote OKF export to `{}` while {action}: {}; staged bundle preserved at `{}`{}",
        output_path.display(),
        source,
        staging_path.display(),
        recovery_detail
    ))
}

fn cleanup_staging_dir(path: &Path) -> Result<(), MemoryError> {
    if path.exists() {
        fs::remove_dir_all(path).map_err(|source| MemoryError::WriteFile {
            path: path.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

fn cleanup_staging_after_export_failure(path: &Path, error: MemoryError) -> MemoryError {
    cleanup_staging_after_export_failure_with(path, error, cleanup_staging_dir)
}

fn cleanup_staging_after_export_failure_with<D>(
    path: &Path,
    error: MemoryError,
    mut cleanup: D,
) -> MemoryError
where
    D: FnMut(&Path) -> Result<(), MemoryError>,
{
    match cleanup(path) {
        Ok(()) => error,
        Err(cleanup) => MemoryError::OkfExportStagingCleanup {
            path: path.to_path_buf(),
            source: Box::new(error),
            cleanup: Box::new(cleanup),
        },
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn filtered_lint_errors(
    config: &MemoryConfig,
    label: &str,
    report: &OkfLintReport,
    ignore_private_memory_links: bool,
) -> String {
    let errors = report
        .findings
        .iter()
        .filter(|finding| finding.severity == LintSeverity::Error)
        .filter(|finding| !ignored_private_export_leak(finding, ignore_private_memory_links))
        .map(|finding| {
            let path = finding
                .path
                .as_ref()
                .map(|path| display_path(&config.repo_root, path))
                .unwrap_or_else(|| "bundle".to_string());
            format!("{path}: {}", finding.message)
        })
        .collect::<Vec<_>>();
    if errors.is_empty() {
        String::new()
    } else {
        format!("{label} has error(s): {}", errors.join("; "))
    }
}

fn ignored_private_export_leak(
    finding: &OkfLintFinding,
    ignore_private_memory_links: bool,
) -> bool {
    ignore_private_memory_links && is_private_export_leak(finding)
}

fn is_private_export_leak(finding: &OkfLintFinding) -> bool {
    finding.code == Some(LintCode::OkfPrivateMemoryLink)
}

fn public_export_index() -> String {
    format!("---\nokf_version: \"{OKF_VERSION}\"\n---\n\n# OpenSymphony Public Memory Export\n")
}

fn public_export_log() -> String {
    format!(
        "# OpenSymphony Public Memory Export Log\n\n## {}\n\n- Public export generated.\n",
        Utc::now().date_naive()
    )
}

fn split_okf_frontmatter(path: &Path, contents: &str) -> Result<(String, String), MemoryError> {
    let normalized = contents.replace("\r\n", "\n").replace('\r', "\n");
    let first_end = normalized
        .find('\n')
        .map(|index| index + 1)
        .unwrap_or(normalized.len());
    if normalized[..first_end].trim() != "---" {
        return Err(MemoryError::OkfMissingFrontmatter {
            path: path.to_path_buf(),
        });
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

    Err(MemoryError::OkfUnterminatedFrontmatter {
        path: path.to_path_buf(),
    })
}

fn collect_okf_markdown_files(
    bundle_root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<(), MemoryError> {
    for entry in fs::read_dir(directory).map_err(|source| MemoryError::ReadFile {
        path: directory.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| MemoryError::ReadFile {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| MemoryError::ReadFile {
            path: path.clone(),
            source,
        })?;
        if file_type.is_dir() {
            collect_okf_markdown_files(bundle_root, &path, files)?;
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            bundle_relative_path(bundle_root, &path)?;
            files.push(path);
        }
    }
    files.sort();
    Ok(())
}

fn bundle_relative_path(bundle_root: &Path, path: &Path) -> Result<PathBuf, MemoryError> {
    path.strip_prefix(bundle_root)
        .map(Path::to_path_buf)
        .map_err(|_| MemoryError::PathOutsideBundle {
            path: path.to_path_buf(),
            bundle_root: bundle_root.to_path_buf(),
        })
}

fn lint_okf_index(
    bundle_root: &Path,
    path: &Path,
    relative: &Path,
    contents: &str,
    findings: &mut Vec<OkfLintFinding>,
) {
    if !has_okf_frontmatter(contents) {
        return;
    }
    if !relative
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty())
    {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "reserved index.md must not contain frontmatter outside the bundle root".to_string(),
            None,
        ));
        return;
    }
    let Ok((frontmatter, _)) = split_okf_frontmatter(path, contents) else {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "reserved index.md has invalid frontmatter".to_string(),
            None,
        ));
        return;
    };
    let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(&frontmatter) else {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "reserved index.md frontmatter is not parseable YAML".to_string(),
            None,
        ));
        return;
    };
    let Some(mapping) = value.as_mapping() else {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "reserved index.md frontmatter must be a YAML mapping".to_string(),
            None,
        ));
        return;
    };
    if let Some(version) = mapping_string(mapping, "okf_version")
        && version != OKF_VERSION
    {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Warn,
            Some(bundle_root.join(relative)),
            format!("unknown OKF version `{version}`"),
            None,
        ));
    }
}

fn lint_okf_log(path: &Path, contents: &str, findings: &mut Vec<OkfLintFinding>) {
    if has_okf_frontmatter(contents) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "reserved log.md must not contain frontmatter".to_string(),
            None,
        ));
    }

    let mut dates = Vec::new();
    let mut invalid_heading = false;
    for line in contents.lines() {
        let Some(heading) = line.strip_prefix("## ") else {
            continue;
        };
        let date = heading.split_whitespace().next().unwrap_or_default();
        if NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() {
            dates.push(date.to_string());
        } else {
            invalid_heading = true;
        }
    }

    if dates.is_empty() || invalid_heading || !dates.windows(2).all(|pair| pair[0] >= pair[1]) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "reserved log.md must use ISO date headings newest first".to_string(),
            Some("opensymphony memory reindex".to_string()),
        ));
    }
}

fn lint_okf_concept(
    bundle_root: &Path,
    path: &Path,
    contents: &str,
    public_export: bool,
    findings: &mut Vec<OkfLintFinding>,
) {
    let (frontmatter, body) = match split_okf_frontmatter(path, contents) {
        Ok(parts) => parts,
        Err(error) => {
            let message = okf_frontmatter_lint_message(&error);
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Error,
                Some(path.to_path_buf()),
                message,
                None,
            ));
            return;
        }
    };
    let value = match serde_yaml::from_str::<serde_yaml::Value>(&frontmatter) {
        Ok(value) => value,
        Err(_) => {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Error,
                Some(path.to_path_buf()),
                "frontmatter is not parseable YAML".to_string(),
                None,
            ));
            return;
        }
    };
    let Some(mapping) = value.as_mapping() else {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "frontmatter must be a YAML mapping".to_string(),
            None,
        ));
        return;
    };
    if mapping_string(mapping, "type")
        .as_deref()
        .is_none_or(str::is_empty)
    {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "frontmatter lacks non-empty `type`".to_string(),
            None,
        ));
        return;
    }

    let concept = match parse_okf_concept(bundle_root, path, contents) {
        Ok(concept) => concept,
        Err(error) => {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Error,
                Some(path.to_path_buf()),
                format!("frontmatter is not parseable OKF YAML: {error}"),
                None,
            ));
            return;
        }
    };

    lint_okf_recommended_fields(&concept, &body, path, findings);
    if !known_okf_type(&concept.frontmatter.concept_type) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Warn,
            Some(path.to_path_buf()),
            format!("unknown type `{}`", concept.frontmatter.concept_type),
            None,
        ));
    }
    let visible_text = markdown_visible_text(contents);
    if contains_private_memory_link(&visible_text) {
        findings.push(okf_lint_finding(
            Some(LintCode::OkfPrivateMemoryLink),
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "private export leak: document links to a private memory path".to_string(),
            None,
        ));
    }
    if public_export && concept_visibility(&concept) == Some(MemoryVisibility::Private) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            "public export includes a private concept".to_string(),
            None,
        ));
    }
    if public_export && let Some(reason) = public_export_private_material(&visible_text) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Error,
            Some(path.to_path_buf()),
            format!("public export contains {reason}"),
            None,
        ));
    }
    lint_okf_links(bundle_root, &concept, path, findings);
    lint_okf_citations(&concept, path, findings);
    lint_okf_info(&concept, mapping, path, findings);
}

fn lint_okf_recommended_fields(
    concept: &OkfConcept,
    body: &str,
    path: &Path,
    findings: &mut Vec<OkfLintFinding>,
) {
    let mut missing = Vec::new();
    if concept.frontmatter.title.is_none() {
        missing.push("title");
        if first_heading(body).is_some() {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Info,
                Some(path.to_path_buf()),
                "title can be synthesized from the first heading".to_string(),
                None,
            ));
        }
    }
    if concept.frontmatter.description.is_none() {
        missing.push("description");
        if first_paragraph(body).is_some() {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Info,
                Some(path.to_path_buf()),
                "description can be synthesized from the first paragraph".to_string(),
                None,
            ));
        }
    }
    if concept.frontmatter.resource.is_none() {
        missing.push("resource");
    }
    if concept.frontmatter.tags.is_empty() {
        missing.push("tags");
    }
    if concept.frontmatter.timestamp.is_none() {
        missing.push("timestamp");
    }
    if !missing.is_empty() {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Warn,
            Some(path.to_path_buf()),
            format!("missing recommended field(s): {}", missing.join(", ")),
            None,
        ));
    }
}

fn lint_okf_links(
    bundle_root: &Path,
    concept: &OkfConcept,
    path: &Path,
    findings: &mut Vec<OkfLintFinding>,
) {
    for link in &concept.links {
        let Some(resolved) =
            resolve_okf_markdown_link(bundle_root, concept.path.as_path(), &link.target)
        else {
            continue;
        };
        if !resolved.exists() {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Warn,
                Some(path.to_path_buf()),
                format!("broken Markdown link `{}`", link.target),
                None,
            ));
        }
    }

    let markdown_ids = concept
        .links
        .iter()
        .filter_map(|link| normalized_markdown_link_id(&link.target))
        .collect::<BTreeSet<_>>();
    for wiki_link in extract_wiki_links(&concept.body) {
        if !markdown_ids.contains(&normalized_wiki_link_id(&wiki_link)) {
            findings.push(okf_lint_finding(
                None,
                LintSeverity::Warn,
                Some(path.to_path_buf()),
                format!("wiki-only link `[[{wiki_link}]]` has no Markdown equivalent"),
                None,
            ));
        }
    }
}

fn lint_okf_citations(concept: &OkfConcept, path: &Path, findings: &mut Vec<OkfLintFinding>) {
    let source_backed = concept
        .frontmatter
        .opensymphony
        .as_ref()
        .is_some_and(|metadata| !metadata.source_refs.is_empty())
        || concept.frontmatter.extra.contains_key("source_refs")
        || concept.frontmatter.extra.contains_key("prs")
        || concept.frontmatter.extra.contains_key("linear_url");
    if source_backed && !has_citations_section(&concept.body) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Warn,
            Some(path.to_path_buf()),
            "citation section missing for source-backed claims".to_string(),
            None,
        ));
    }
}

fn lint_okf_info(
    concept: &OkfConcept,
    mapping: &serde_yaml::Mapping,
    path: &Path,
    findings: &mut Vec<OkfLintFinding>,
) {
    let retained_legacy = [
        "issue",
        "milestone",
        "milestone_id",
        "project",
        "project_id",
        "linear_url",
        "areas",
        "area",
        "repository",
        "repo",
        "prs",
        "source_refs",
        "docs_sync",
    ]
    .into_iter()
    .filter(|key| concept.frontmatter.extra.contains_key(*key))
    .collect::<Vec<_>>();
    if !retained_legacy.is_empty() {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Info,
            Some(path.to_path_buf()),
            format!("legacy field(s) retained: {}", retained_legacy.join(", ")),
            None,
        ));
    }
    if mapping.contains_key(serde_yaml::Value::String("opensymphony".to_string())) {
        findings.push(okf_lint_finding(
            None,
            LintSeverity::Info,
            Some(path.to_path_buf()),
            "bundle contains OpenSymphony extension fields".to_string(),
            None,
        ));
    }
}

fn has_okf_frontmatter(contents: &str) -> bool {
    contents
        .lines()
        .next()
        .is_some_and(|line| line.trim() == "---")
}

fn okf_frontmatter_lint_message(error: &MemoryError) -> String {
    match error {
        MemoryError::OkfMissingFrontmatter { .. } => "lacks OKF YAML frontmatter".to_string(),
        MemoryError::OkfUnterminatedFrontmatter { .. } => {
            "has unterminated OKF YAML frontmatter".to_string()
        }
        _ => error.to_string(),
    }
}

fn mapping_string(mapping: &serde_yaml::Mapping, key: &str) -> Option<String> {
    mapping
        .get(serde_yaml::Value::String(key.to_string()))
        .and_then(value_as_string)
        .filter(|value| !value.trim().is_empty())
}

fn raw_okf_visibility(contents: &str) -> Option<MemoryVisibility> {
    let (frontmatter, _) = split_okf_frontmatter(Path::new("okf-concept.md"), contents).ok()?;
    let value = serde_yaml::from_str::<serde_yaml::Value>(&frontmatter).ok()?;
    let mapping = value.as_mapping()?;
    mapping_visibility(mapping).or_else(|| {
        mapping
            .get(serde_yaml::Value::String("opensymphony".to_string()))
            .and_then(|value| value.as_mapping())
            .and_then(mapping_visibility)
    })
}

fn mapping_visibility(mapping: &serde_yaml::Mapping) -> Option<MemoryVisibility> {
    match mapping_string(mapping, "visibility")?.as_str() {
        "private" => Some(MemoryVisibility::Private),
        "public" => Some(MemoryVisibility::Public),
        _ => None,
    }
}

fn known_okf_type(concept_type: &str) -> bool {
    matches!(
        concept_type,
        "issue-capsule"
            | "milestone-memory-node"
            | "project-memory-node"
            | "area-memory-node"
            | "topic-doc"
            | "run-summary"
            | "code-context"
            | "repository-memory-node"
            | "reference"
    )
}

fn concept_visibility(concept: &OkfConcept) -> Option<MemoryVisibility> {
    concept
        .frontmatter
        .opensymphony
        .as_ref()
        .and_then(|metadata| metadata.visibility)
        .or_else(|| legacy_visibility(&concept.frontmatter))
}

fn resolve_okf_markdown_link(
    bundle_root: &Path,
    concept_path: &Path,
    target: &str,
) -> Option<PathBuf> {
    let target = local_markdown_target(target)?;
    let relative = if let Some(stripped) = target.strip_prefix('/') {
        PathBuf::from(stripped)
    } else {
        concept_path.parent().unwrap_or(Path::new("")).join(target)
    };
    let normalized = normalize_okf_relative_path(&relative)?;
    Some(bundle_root.join(normalized))
}

fn local_markdown_target(target: &str) -> Option<&str> {
    let target = target.trim();
    if target.is_empty()
        || target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
    {
        return None;
    }
    let end = target.find(['#', '?']).unwrap_or(target.len());
    let target = &target[..end];
    target
        .to_ascii_lowercase()
        .ends_with(".md")
        .then_some(target)
}

fn normalize_okf_relative_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(part) => normalized.push(part),
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(normalized)
}

fn normalized_markdown_link_id(target: &str) -> Option<String> {
    let target = local_markdown_target(target)?;
    Some(normalize_okf_link_id(target))
}

fn normalized_wiki_link_id(target: &str) -> String {
    normalize_okf_link_id(target.split('|').next().unwrap_or(target))
}

fn normalize_okf_link_id(target: &str) -> String {
    target
        .trim()
        .trim_start_matches('/')
        .trim_end_matches(".md")
        .trim_end_matches(".MD")
        .replace(' ', "-")
        .to_ascii_lowercase()
}

#[cfg(test)]
fn contains_private_memory_link_in_markdown(contents: &str) -> bool {
    let visible = markdown_visible_text(contents);
    contains_private_memory_link(&visible)
}

fn markdown_visible_text(contents: &str) -> String {
    let mut visible = String::with_capacity(contents.len());
    let mut index = 0;
    while index < contents.len() {
        let Some((current, next)) = char_at(contents, index) else {
            break;
        };
        if contents[index..].starts_with("<!--") {
            index = skip_html_comment(contents, index);
            continue;
        }
        if is_fenced_code_start(contents, index) {
            index = skip_fenced_code_block(contents, index);
            continue;
        }
        match current {
            '`' => index = skip_code_span(contents, index),
            '\\' => index = skip_escaped_char(contents, next),
            _ => {
                visible.push(current);
                index = next;
            }
        }
    }
    visible
}

const PUBLIC_PRIVATE_COMMENT_PATTERNS: &[&str] = &["linear:comment:"];
const PUBLIC_PRIVATE_LOCAL_PATH_PATTERNS: &[&str] = &[
    ".opensymphony/memory/issues",
    ".opensymphony\\memory\\issues",
    "../.opensymphony/memory/issues",
];
const PUBLIC_PRIVATE_SOURCE_PATTERNS: &[&str] = &[
    ".opensymphony/memory/source",
    ".opensymphony\\memory\\source",
    "../.opensymphony/memory/source",
    ".opensymphony/memory/snapshot",
    ".opensymphony\\memory\\snapshot",
    "../.opensymphony/memory/snapshot",
];

fn public_export_private_material(visible: &str) -> Option<&'static str> {
    if contains_any_ascii_case_insensitive(visible, PUBLIC_PRIVATE_COMMENT_PATTERNS) {
        return Some("private comment references");
    }
    if contains_any_ascii_case_insensitive(visible, PUBLIC_PRIVATE_LOCAL_PATH_PATTERNS) {
        return Some("private local paths");
    }
    if contains_any_ascii_case_insensitive(visible, PUBLIC_PRIVATE_SOURCE_PATTERNS) {
        return Some("private source snapshots");
    }
    None
}

fn contains_any_ascii_case_insensitive(contents: &str, patterns: &[&str]) -> bool {
    patterns
        .iter()
        .any(|pattern| contains_ascii_case_insensitive(contents, pattern))
}

fn contains_ascii_case_insensitive(contents: &str, pattern: &str) -> bool {
    let pattern = pattern.as_bytes();
    !pattern.is_empty()
        && contents
            .as_bytes()
            .windows(pattern.len())
            .any(|window| window.eq_ignore_ascii_case(pattern))
}

fn extract_wiki_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut index = 0;
    while index < body.len() {
        let Some((current, next)) = char_at(body, index) else {
            break;
        };
        if body[index..].starts_with("<!--") {
            index = skip_html_comment(body, index);
            continue;
        }
        if is_fenced_code_start(body, index) {
            index = skip_fenced_code_block(body, index);
            continue;
        }
        match current {
            '`' => index = skip_code_span(body, index),
            '\\' => index = skip_escaped_char(body, next),
            '[' if body[index..].starts_with("[[") => {
                let content_start = index + 2;
                let Some(end) = body[content_start..].find("]]") else {
                    index = next;
                    continue;
                };
                let content_end = content_start + end;
                let link = body[content_start..content_end].trim();
                if !link.is_empty() {
                    links.push(link.to_string());
                }
                index = content_end + 2;
            }
            _ => index = next,
        }
    }
    links
}

fn first_heading(body: &str) -> Option<&str> {
    body.lines()
        .find_map(|line| line.trim_start().strip_prefix("# "))
        .map(str::trim)
        .filter(|heading| !heading.is_empty())
}

fn first_paragraph(body: &str) -> Option<&str> {
    body.lines().map(str::trim).find(|line| {
        !line.is_empty()
            && !line.starts_with('#')
            && !line.starts_with('-')
            && !line.starts_with("```")
    })
}

fn has_citations_section(body: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim();
        line == "# Citations" || line == "## Citations" || line == "### Citations"
    })
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
        string_extra(frontmatter, "milestone_id")
            .or_else(|| string_extra(frontmatter, "milestone")),
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

fn remove_represented_legacy_opensymphony_fields(
    extra: &mut BTreeMap<String, serde_yaml::Value>,
    metadata: &OpenSymphonyOkfMetadata,
) {
    if metadata.visibility.is_some() {
        extra.remove("visibility");
    }
    if metadata.docs_sync.is_some() {
        extra.remove("docs_sync");
    }
    if has_scope_ref(metadata, &KnowledgeScopeKind::WorkItem) {
        extra.remove("issue");
    }
    if has_scope_ref(metadata, &KnowledgeScopeKind::Milestone) {
        extra.remove("milestone");
        extra.remove("milestone_id");
    }
    if has_scope_ref(metadata, &KnowledgeScopeKind::Project) {
        extra.remove("project");
        extra.remove("project_id");
    }
    if has_scope_ref(metadata, &KnowledgeScopeKind::Area) {
        extra.remove("area");
        extra.remove("areas");
    }
    if has_scope_ref(metadata, &KnowledgeScopeKind::Repository) {
        extra.remove("repo");
        extra.remove("repository");
    }
    if metadata
        .source_refs
        .iter()
        .any(|source_ref| source_ref.kind == "linear_issue" && source_ref.url.is_some())
    {
        extra.remove("linear_url");
    }

    let legacy_refs = legacy_source_refs_from_extra(extra);
    if !legacy_refs.is_empty()
        && legacy_refs
            .iter()
            .all(|legacy_ref| source_ref_is_represented(metadata, legacy_ref))
    {
        extra.remove("prs");
        extra.remove("source_refs");
    }
}

fn has_scope_ref(metadata: &OpenSymphonyOkfMetadata, kind: &KnowledgeScopeKind) -> bool {
    metadata
        .scope_refs
        .iter()
        .any(|scope_ref| &scope_ref.kind == kind)
}

fn source_ref_is_represented(
    metadata: &OpenSymphonyOkfMetadata,
    legacy_ref: &MemorySourceRef,
) -> bool {
    metadata.source_refs.iter().any(|source_ref| {
        source_ref.kind == legacy_ref.kind
            && source_ref.id == legacy_ref.id
            && (legacy_ref.url.is_none() || source_ref.url == legacy_ref.url)
    })
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
    legacy_source_refs_from_extra(&frontmatter.extra)
}

fn legacy_source_refs_from_extra(
    extra: &BTreeMap<String, serde_yaml::Value>,
) -> Vec<MemorySourceRef> {
    let mut refs = Vec::new();
    if let Some(serde_yaml::Value::Mapping(source_refs)) = extra.get("source_refs") {
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
    if let Some(serde_yaml::Value::Sequence(prs)) = extra.get("prs") {
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
    let references = reference_link_targets(body);
    let mut index = 0;
    while index < body.len() {
        let Some((current, next)) = char_at(body, index) else {
            break;
        };
        if body[index..].starts_with("<!--") {
            index = skip_html_comment(body, index);
            continue;
        }
        if is_fenced_code_start(body, index) {
            index = skip_fenced_code_block(body, index);
            continue;
        }
        match current {
            '`' => index = skip_code_span(body, index),
            '\\' => index = skip_escaped_char(body, next),
            '<' => {
                let Some((target, after_target)) = parse_autolink(body, next) else {
                    index = next;
                    continue;
                };
                links.push(OkfLink {
                    target,
                    label: None,
                });
                index = after_target;
            }
            '[' if !is_image_marker(body, index) => {
                let Some((label, after_label)) = parse_link_label(body, next) else {
                    index = next;
                    continue;
                };
                match char_at(body, after_label) {
                    Some(('(', after_open)) => {
                        let Some((target, after_target)) = parse_link_target(body, after_open)
                        else {
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
                    Some(('[', after_ref_open)) => {
                        let Some((reference, after_reference)) =
                            parse_link_label(body, after_ref_open)
                        else {
                            index = after_ref_open;
                            continue;
                        };
                        let key = if reference.is_empty() {
                            label.as_str()
                        } else {
                            reference.as_str()
                        };
                        if let Some(target) = references.get(&normalize_reference_label(key)) {
                            links.push(OkfLink {
                                target: target.clone(),
                                label: Some(label).filter(|label| !label.is_empty()),
                            });
                        }
                        index = after_reference;
                    }
                    Some((':', _)) => index = after_label,
                    _ => {
                        if let Some(target) = references.get(&normalize_reference_label(&label)) {
                            links.push(OkfLink {
                                target: target.clone(),
                                label: Some(label).filter(|label| !label.is_empty()),
                            });
                        }
                        index = after_label;
                    }
                }
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
    char_at(value, index).map(|(_, next)| next).unwrap_or(index)
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

fn skip_html_comment(value: &str, index: usize) -> usize {
    value[index..]
        .find("-->")
        .map(|end| index + end + 3)
        .unwrap_or(value.len())
}

fn is_fenced_code_start(value: &str, index: usize) -> bool {
    let line_start = value[..index].rfind('\n').map(|line| line + 1).unwrap_or(0);
    if value[line_start..index]
        .chars()
        .any(|character| character != ' ' && character != '\t')
    {
        return false;
    }
    value[index..].starts_with("```") || value[index..].starts_with("~~~")
}

fn skip_fenced_code_block(value: &str, index: usize) -> usize {
    let fence = &value[index..index + 3];
    let mut cursor = value[index..]
        .find('\n')
        .map(|line| index + line + 1)
        .unwrap_or(value.len());
    while cursor < value.len() {
        if is_fenced_code_start(value, cursor) && value[cursor..].starts_with(fence) {
            return value[cursor..]
                .find('\n')
                .map(|line| cursor + line + 1)
                .unwrap_or(value.len());
        }
        cursor = value[cursor..]
            .find('\n')
            .map(|line| cursor + line + 1)
            .unwrap_or(value.len());
    }
    value.len()
}

fn is_image_marker(value: &str, index: usize) -> bool {
    value[..index].ends_with('!') && !is_escaped(value, index - 1)
}

fn is_escaped(value: &str, index: usize) -> bool {
    let mut slash_count = 0;
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if value.as_bytes()[cursor] != b'\\' {
            break;
        }
        slash_count += 1;
    }
    slash_count % 2 == 1
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
    if let Some(target) = markdown_target_before_optional_title(raw) {
        return Some(target);
    }
    raw.split_whitespace().next().map(str::to_string)
}

fn markdown_target_before_optional_title(raw: &str) -> Option<String> {
    let mut boundary = raw.len();
    for (index, character) in raw.char_indices() {
        if character.is_whitespace() && local_markdown_target(&raw[..index]).is_some() {
            boundary = index;
            break;
        }
    }
    let candidate = raw[..boundary].trim();
    local_markdown_target(candidate)
        .is_some()
        .then(|| candidate.to_string())
        .filter(|candidate| !candidate.is_empty())
}

fn parse_autolink(value: &str, index: usize) -> Option<(String, usize)> {
    let end = value[index..].find('>')? + index;
    let target = &value[index..end];
    if target.starts_with("http://") || target.starts_with("https://") {
        Some((target.to_string(), end + 1))
    } else {
        None
    }
}

fn reference_link_targets(body: &str) -> BTreeMap<String, String> {
    let mut references = BTreeMap::new();
    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((label, target)) = rest.split_once("]:") else {
            continue;
        };
        let Some(target) = normalize_link_target(target) else {
            continue;
        };
        references.insert(normalize_reference_label(label), target);
    }
    references
}

fn normalize_reference_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod okf_tests {
    use super::*;

    #[test]
    fn wiki_link_extraction_ignores_code_and_comments() {
        let links = extract_wiki_links(
            r#"
Visible [[real-target|Real Target]].

`[[inline-code]]`

```text
[[fenced-code]]
```

<!-- [[commented]] -->
\[\[escaped\]\]
"#,
        );

        assert_eq!(links, vec!["real-target|Real Target"]);
    }

    #[test]
    fn wiki_link_matches_markdown_target_with_spaces() {
        let frontmatter = OkfFrontmatter::new("issue-capsule").expect("frontmatter should build");
        let concept = OkfConcept::new(
            "issues/COE-123.md",
            frontmatter,
            "[Some Page](Some Page.md)\n[[Some Page]]\n",
        )
        .expect("concept should build");
        let mut findings = Vec::new();

        lint_okf_links(
            Path::new("."),
            &concept,
            Path::new("issues/COE-123.md"),
            &mut findings,
        );

        assert!(
            !findings
                .iter()
                .any(|finding| finding.message.contains("wiki-only link")),
            "matching Markdown link should suppress wiki-only warning: {findings:?}"
        );
    }

    #[test]
    fn markdown_target_with_optional_title_requires_md_suffix() {
        assert_eq!(
            markdown_target_before_optional_title("Some Page.md \"Title\"").as_deref(),
            Some("Some Page.md")
        );
        assert_eq!(
            markdown_target_before_optional_title("assets/image.md.png \"Title\""),
            None
        );
    }

    #[test]
    fn export_promotion_failure_restores_output_and_preserves_staging() {
        let repo = tempfile::TempDir::new().expect("temp repo should exist");
        let staging = repo.path().join(".okf-export.tmp");
        let output = repo.path().join("okf-export-private");
        fs::create_dir_all(&staging).expect("staging should write");
        fs::write(staging.join("bundle.md"), "staged bundle").expect("staged file should write");
        fs::create_dir_all(&output).expect("existing output should write");

        let staging_for_failure = staging.clone();
        let output_for_failure = output.clone();
        let result = promote_staged_export_with(
            &staging,
            &output,
            |from, to| {
                if from == staging_for_failure.as_path() && to == output_for_failure.as_path() {
                    Err(io::Error::other("injected rename failure"))
                } else {
                    fs::rename(from, to)
                }
            },
            |path| fs::remove_dir_all(path),
        );

        let error = result.expect_err("promotion should fail");
        assert!(
            error.to_string().contains("staged bundle preserved"),
            "error should explain recovery path: {error}"
        );
        assert!(
            staging.join("bundle.md").is_file(),
            "failed promotion should preserve staged bundle"
        );
        assert!(
            output.is_dir(),
            "failed promotion should restore previous output directory"
        );
        let leaked_backup = fs::read_dir(repo.path())
            .expect("repo should list")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".backup-"));
        assert!(
            !leaked_backup,
            "successful rollback should consume backup dir"
        );
    }

    #[test]
    fn export_promotion_reports_backup_cleanup_failure() {
        let repo = tempfile::TempDir::new().expect("temp repo should exist");
        let staging = repo.path().join(".okf-export.tmp");
        let output = repo.path().join("okf-export-private");
        fs::create_dir_all(&staging).expect("staging should write");
        fs::write(staging.join("bundle.md"), "staged bundle").expect("staged file should write");
        fs::create_dir_all(&output).expect("existing output should write");

        let result = promote_staged_export_with(
            &staging,
            &output,
            |from, to| fs::rename(from, to),
            |_path| Err(io::Error::other("injected cleanup failure")),
        );

        let error = result.expect_err("cleanup failure should be reported");
        assert!(
            error.to_string().contains("failed to remove backup"),
            "error should explain backup cleanup failure: {error}"
        );
        assert!(
            output.join("bundle.md").is_file(),
            "successful promotion should leave staged bundle at output"
        );
        let leaked_backup = fs::read_dir(repo.path())
            .expect("repo should list")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".backup-"));
        assert!(
            leaked_backup,
            "failed cleanup should leave backup for manual operator cleanup"
        );
    }

    #[test]
    fn export_failure_reports_staging_cleanup_failure() {
        let repo = tempfile::TempDir::new().expect("temp repo should exist");
        let staging = repo.path().join(".okf-export.tmp");
        let original = MemoryError::InvalidInput("exported OKF bundle has error(s)".to_string());

        let error = cleanup_staging_after_export_failure_with(&staging, original, |path| {
            Err(MemoryError::WriteFile {
                path: path.to_path_buf(),
                source: io::Error::other("injected cleanup failure"),
            })
        });

        let message = error.to_string();
        assert!(
            message.contains("exported OKF bundle has error(s)"),
            "error should preserve the original export failure: {message}"
        );
        assert!(
            message.contains("failed to remove OKF export staging directory"),
            "error should report the failed staging cleanup: {message}"
        );
        assert!(
            message.contains(staging.to_string_lossy().as_ref()),
            "error should include the recoverable staging path: {message}"
        );
        let MemoryError::OkfExportStagingCleanup {
            source, cleanup, ..
        } = error
        else {
            panic!("error should preserve structured export and cleanup failures");
        };
        assert!(
            matches!(*source, MemoryError::InvalidInput(_)),
            "source error should preserve the original variant"
        );
        assert!(
            matches!(*cleanup, MemoryError::WriteFile { .. }),
            "cleanup error should preserve the cleanup variant"
        );
    }

    #[test]
    fn create_staging_dir_retries_existing_candidate() {
        let repo = tempfile::TempDir::new().expect("temp repo should exist");
        let output = repo.path().join("okf-export-public");
        let first_candidate = repo
            .path()
            .join(format!(".okf-export-public.tmp-{}-0", std::process::id()));
        fs::create_dir(&first_candidate).expect("first candidate should exist");

        let staging = create_staging_dir(repo.path(), &output).expect("staging should allocate");

        assert_ne!(
            staging, first_candidate,
            "allocator should retry after an existing candidate"
        );
        assert!(
            staging.is_dir(),
            "allocator should create the retried candidate atomically"
        );
    }

    #[cfg(unix)]
    #[test]
    fn export_output_target_rejects_final_symlink() {
        let repo = tempfile::TempDir::new().expect("temp repo should exist");
        let output = repo.path().join("public-okf");
        std::os::unix::fs::symlink(repo.path().join("missing-target"), &output)
            .expect("symlink should write");

        let result = ensure_output_target_not_symlink(&output);

        assert!(matches!(result, Err(MemoryError::InvalidInput(_))));
    }

    #[test]
    fn private_memory_link_scan_ignores_code_and_comments() {
        let visible = "See .opensymphony/memory/issues/COE-123.md";
        let hidden = r#"
` .opensymphony/memory/issues/COE-123.md `

```text
.opensymphony/memory/issues/COE-123.md
```

<!-- .opensymphony/memory/issues/COE-123.md -->
"#;

        assert!(contains_private_memory_link_in_markdown(visible));
        assert!(!contains_private_memory_link_in_markdown(hidden));
    }
}
