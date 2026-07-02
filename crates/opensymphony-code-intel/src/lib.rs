use std::{
    collections::{BTreeSet, HashMap},
    fs,
    io::{ErrorKind, Read},
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

// TODO(COE-506): invert this adapter so memory consumes a code-intel owned
// provider trait after the AST integration is stable.
// COE-499 keeps the existing memory CodeIntelIndex contract so memory.context
// gains AST evidence without adding a new public tool surface.
use crate::{
    opensymphony_memory::{
        CodeIntelArtifact, CodeIntelIndex, KnowledgeScope, MemoryError, MemorySourceRef,
    },
    opensymphony_planning::CodebaseAnalyzer,
};

pub const PROVIDER_NAME: &str = "tree-sitter";
pub const TREE_SITTER_VERSION: &str = "0.26.9";
pub const RUST_GRAMMAR_VERSION: &str = "0.24.2";
pub const RUST_QUERY_PACK_VERSION: &str = "rust-query-pack-v2";
pub const TYPESCRIPT_GRAMMAR_VERSION: &str = "0.23.2";
pub const JAVASCRIPT_GRAMMAR_VERSION: &str = "0.25.0";
pub const PYTHON_GRAMMAR_VERSION: &str = "0.25.0";
pub const LIGHTWEIGHT_PARSER_VERSION: &str = "lightweight-text-v1";
pub const LIGHTWEIGHT_PROVIDER_NAME: &str = "lightweight";
pub const LIGHTWEIGHT_TREE_SITTER_VERSION: &str = "n/a";
pub const TYPESCRIPT_QUERY_PACK_VERSION: &str = "typescript-query-pack-v1";
pub const TSX_QUERY_PACK_VERSION: &str = "tsx-query-pack-v1";
pub const JAVASCRIPT_QUERY_PACK_VERSION: &str = "javascript-query-pack-v1";
pub const JSX_QUERY_PACK_VERSION: &str = "jsx-query-pack-v1";
pub const PYTHON_QUERY_PACK_VERSION: &str = "python-query-pack-v1";
const MARKDOWN_FENCE_QUERY_NAME: &str = "markdown_fences";
const SKIPPED_DIRECTORY_COMPONENTS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "__pycache__",
    "coverage",
    ".next",
    ".turbo",
    "vendor",
    "generated",
];

const RUST_METADATA: &str = include_str!("../queries/rust/metadata.toml");
const TYPESCRIPT_METADATA: &str = include_str!("../queries/typescript/metadata.toml");
const TSX_METADATA: &str = include_str!("../queries/tsx/metadata.toml");
const JAVASCRIPT_METADATA: &str = include_str!("../queries/javascript/metadata.toml");
const JSX_METADATA: &str = include_str!("../queries/jsx/metadata.toml");
const PYTHON_METADATA: &str = include_str!("../queries/python/metadata.toml");
const DEFAULT_MAX_FILE_BYTES: u64 = 2_097_152;

const STANDARD_CAPTURE_NAMES: &[&str] = &[
    "definition.module",
    "definition.class",
    "definition.struct",
    "definition.enum",
    "definition.trait",
    "definition.interface",
    "definition.type",
    "definition.function",
    "definition.method",
    "definition.constructor",
    "definition.field",
    "definition.variable",
    "definition.constant",
    "definition.test",
    "reference.identifier",
    "reference.call",
    "reference.type",
    "import.source",
    "import.name",
    "export.name",
    "test.case",
    "test.subject",
    "doc.comment",
    "local.scope",
    "local.definition",
    "local.reference",
    "diagnostic.error",
    "diagnostic.missing",
    "injection.content",
    "injection.language",
];

const NO_VARIANT_QUERIES: &[QueryAsset] = &[];

const RUST_SHARED_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/rust/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/rust/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/rust/calls.scm")),
    QueryAsset::new("docs", include_str!("../queries/rust/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/rust/locals.scm")),
];

const RUST_DIAGNOSTIC_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "diagnostics",
    include_str!("../queries/rust/diagnostics.scm"),
)];

const TYPESCRIPT_SHARED_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/typescript/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/typescript/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/typescript/calls.scm")),
    QueryAsset::new("tests", include_str!("../queries/typescript/tests.scm")),
    QueryAsset::new("docs", include_str!("../queries/typescript/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/typescript/locals.scm")),
];

const TYPESCRIPT_INJECTION_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "injections",
    include_str!("../queries/typescript/injections.scm"),
)];

const TSX_INJECTION_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "injections",
    include_str!("../queries/tsx/injections.scm"),
)];

const TYPESCRIPT_DIAGNOSTIC_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "diagnostics",
    include_str!("../queries/typescript/diagnostics.scm"),
)];

const JAVASCRIPT_SHARED_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/javascript/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/javascript/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/javascript/calls.scm")),
    QueryAsset::new("tests", include_str!("../queries/javascript/tests.scm")),
    QueryAsset::new("docs", include_str!("../queries/javascript/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/javascript/locals.scm")),
];

const JSX_INJECTION_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "injections",
    include_str!("../queries/jsx/injections.scm"),
)];

const JAVASCRIPT_DIAGNOSTIC_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "diagnostics",
    include_str!("../queries/javascript/diagnostics.scm"),
)];

const PYTHON_SHARED_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/python/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/python/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/python/calls.scm")),
    QueryAsset::new("docs", include_str!("../queries/python/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/python/locals.scm")),
];

const PYTHON_DIAGNOSTIC_QUERIES: &[QueryAsset] = &[QueryAsset::new(
    "diagnostics",
    include_str!("../queries/python/diagnostics.scm"),
)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SourceLanguage {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Python,
    Json,
    Yaml,
    Toml,
    Markdown,
}

impl SourceLanguage {
    pub fn id(self) -> &'static str {
        match self {
            SourceLanguage::Rust => "rust",
            SourceLanguage::TypeScript => "typescript",
            SourceLanguage::Tsx => "tsx",
            SourceLanguage::JavaScript => "javascript",
            SourceLanguage::Jsx => "jsx",
            SourceLanguage::Python => "python",
            SourceLanguage::Json => "json",
            SourceLanguage::Yaml => "yaml",
            SourceLanguage::Toml => "toml",
            SourceLanguage::Markdown => "markdown",
        }
    }

    pub fn from_id(value: &str) -> Option<Self> {
        match value {
            "rust" => Some(SourceLanguage::Rust),
            "typescript" => Some(SourceLanguage::TypeScript),
            "tsx" => Some(SourceLanguage::Tsx),
            "javascript" => Some(SourceLanguage::JavaScript),
            "jsx" => Some(SourceLanguage::Jsx),
            "python" => Some(SourceLanguage::Python),
            "json" => Some(SourceLanguage::Json),
            "yaml" => Some(SourceLanguage::Yaml),
            "toml" => Some(SourceLanguage::Toml),
            "markdown" => Some(SourceLanguage::Markdown),
            _ => None,
        }
    }

    pub fn supports_ast_queries(self) -> bool {
        !self.is_lightweight()
    }

    fn display_name(self) -> &'static str {
        match self {
            SourceLanguage::Rust => "Rust",
            SourceLanguage::TypeScript => "TypeScript",
            SourceLanguage::Tsx => "TSX",
            SourceLanguage::JavaScript => "JavaScript",
            SourceLanguage::Jsx => "JSX",
            SourceLanguage::Python => "Python",
            SourceLanguage::Json => "JSON",
            SourceLanguage::Yaml => "YAML",
            SourceLanguage::Toml => "TOML",
            SourceLanguage::Markdown => "Markdown",
        }
    }

    fn is_lightweight(self) -> bool {
        matches!(
            self,
            SourceLanguage::Json
                | SourceLanguage::Yaml
                | SourceLanguage::Toml
                | SourceLanguage::Markdown
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceIdentity {
    pub language: SourceLanguage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub bytes: usize,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserVersions {
    pub provider: String,
    pub tree_sitter: String,
    pub grammar: String,
    pub query_pack: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedDocumentSummary {
    pub source: SourceIdentity,
    pub versions: ParserVersions,
    pub root_kind: String,
    pub has_errors: bool,
    pub symbols: Vec<SymbolRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub captures: Vec<CaptureRecord>,
    pub diagnostics: Vec<AstDiagnostic>,
}

/// One-based Tree-sitter points; columns are byte offsets and end positions are exclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl SourceSpan {
    pub fn render(&self) -> String {
        format!(
            "{}:{}-{}:{}",
            self.start_line, self.start_column, self.end_line, self.end_column
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SymbolKind {
    Module,
    Class,
    Struct,
    Enum,
    Trait,
    Interface,
    Type,
    Function,
    Method,
    Constructor,
    Field,
    Variable,
    Constant,
    Test,
    Document,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolRecord {
    pub kind: SymbolKind,
    pub name: String,
    pub span: SourceSpan,
    pub rendered_span: String,
    pub parser_version: String,
    pub query_pack_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub query_name: String,
    pub capture_name: String,
    pub text: String,
    pub span: SourceSpan,
    pub rendered_span: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdHocQueryMatch {
    pub captures: Vec<CaptureRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AstDiagnosticKind {
    Error,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AstDiagnostic {
    pub kind: AstDiagnosticKind,
    pub node_kind: String,
    pub span: SourceSpan,
    pub rendered_span: String,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodeIntelError {
    #[error("unsupported source language for {path}")]
    UnsupportedLanguage { path: String },
    #[error("failed to load parser: {0}")]
    Language(#[from] tree_sitter::LanguageError),
    #[error("failed to parse source")]
    Parse,
    #[error("invalid query pack metadata: {0}")]
    Metadata(#[from] toml::de::Error),
    #[error("query pack metadata mismatch for {language}: expected {expected}, got {actual}")]
    MetadataMismatch {
        language: String,
        expected: String,
        actual: String,
    },
    #[error("invalid query {query_name}: {source}")]
    Query {
        query_name: String,
        source: tree_sitter::QueryError,
    },
    #[error("query {query_name} uses nonstandard capture @{capture_name}")]
    NonstandardCapture {
        query_name: String,
        capture_name: String,
    },
    #[error("query {query_name} uses capture @{capture_name} missing from metadata")]
    UndocumentedCapture {
        query_name: String,
        capture_name: String,
    },
    #[error("captured text is not utf-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}

#[derive(Debug, Deserialize)]
struct QueryPackMetadata {
    language: String,
    version: String,
    grammar_crate: String,
    grammar_version: String,
    parser_crate: String,
    parser_version: String,
    captures: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
struct QueryAsset {
    name: &'static str,
    source: &'static str,
}

impl QueryAsset {
    const fn new(name: &'static str, source: &'static str) -> Self {
        Self { name, source }
    }
}

struct CompiledQuery<'a> {
    asset: QueryAsset,
    query: Query,
    metadata: &'a QueryPackMetadata,
}

#[derive(Clone, Copy)]
struct LanguageConfig {
    language: SourceLanguage,
    grammar_crate: &'static str,
    grammar_version: &'static str,
    query_pack_version: &'static str,
    metadata: &'static str,
    shared_queries: &'static [QueryAsset],
    variant_queries: &'static [QueryAsset],
    diagnostic_queries: &'static [QueryAsset],
    parser: fn() -> Language,
}

pub struct AstCodeIntelProvider {
    root: PathBuf,
    max_file_bytes: u64,
}

impl AstCodeIntelProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }

    fn code_context_report(
        &self,
        paths: &[PathBuf],
        scope_refs: &[KnowledgeScope],
        limit: usize,
    ) -> Result<AstCodeIntelReport, MemoryError> {
        let symbol_kinds = BTreeSet::new();
        self.code_context_report_with_symbols(paths, scope_refs, limit, &symbol_kinds)
    }

    fn code_context_report_with_symbols(
        &self,
        paths: &[PathBuf],
        scope_refs: &[KnowledgeScope],
        limit: usize,
        symbol_kinds: &BTreeSet<String>,
    ) -> Result<AstCodeIntelReport, MemoryError> {
        let repo_root = self.canonical_root()?;
        let commit_sha = git_commit_sha(&repo_root);
        let mut report = AstCodeIntelReport::default();
        let mut remaining_symbols = limit;

        if paths.is_empty() {
            report
                .fallback_reasons
                .push("no requested paths; repository summary fallback used".to_string());
            return Ok(report);
        }

        for path in paths {
            let Some(resolved) = self.resolve_requested_path(&repo_root, path)? else {
                let relative_path = self.relative_candidate_path(&repo_root, path)?;
                let relative_display = relative_path.to_string_lossy().to_string();
                report.fallback_reasons.push(format!(
                    "{relative_display} could not be read: file not found"
                ));
                report.fallback_paths.push(relative_path);
                continue;
            };
            let relative_path = resolved
                .strip_prefix(&repo_root)
                .unwrap_or(&resolved)
                .to_path_buf();
            let relative_display = relative_path.to_string_lossy().to_string();

            if resolved.is_dir() {
                if let Some(component) = skipped_directory_name(&resolved) {
                    report.fallback_reasons.push(format!(
                        "{relative_display} skipped directory `{component}`"
                    ));
                    continue;
                }
                report
                    .fallback_reasons
                    .push(format!("{relative_display} is a directory"));
                report.fallback_paths.push(relative_path);
                continue;
            }

            let Some(source) = self.read_limited_source(&resolved, &relative_display, &mut report)
            else {
                continue;
            };
            if detect_language(&relative_path).is_none() {
                report
                    .fallback_reasons
                    .push(format!("{relative_display} has unsupported language"));
                report.fallback_paths.push(relative_path);
                continue;
            }

            let summary = match parse_path(&relative_path, &source) {
                Ok(summary) => summary,
                Err(error) => {
                    report
                        .fallback_reasons
                        .push(format!("{relative_display} AST parse failed: {error}"));
                    report.fallback_paths.push(relative_path);
                    continue;
                }
            };

            report.parsed_files += 1;
            report.query_runs += 1;
            if summary.has_errors {
                report.fallback_reasons.push(format!(
                    "{relative_display} parsed with Tree-sitter diagnostics"
                ));
                report.fallback_paths.push(relative_path.clone());
            }
            let (artifacts, used_symbols) = ast_artifacts_for_summary(
                summary,
                &relative_path,
                &relative_display,
                scope_refs,
                commit_sha.clone(),
                remaining_symbols,
                symbol_kinds,
            );
            remaining_symbols = remaining_symbols.saturating_sub(used_symbols);
            report.used_symbols += used_symbols;
            report.artifacts.extend(artifacts);
        }

        Ok(report)
    }

    fn read_limited_source(
        &self,
        resolved: &Path,
        relative_display: &str,
        report: &mut AstCodeIntelReport,
    ) -> Option<String> {
        let file = match fs::File::open(resolved) {
            Ok(file) => file,
            Err(source) => {
                report
                    .fallback_reasons
                    .push(format!("{relative_display} could not be opened: {source}"));
                report.fallback_paths.push(PathBuf::from(relative_display));
                return None;
            }
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(source) => {
                report.fallback_reasons.push(format!(
                    "{relative_display} metadata could not be read: {source}"
                ));
                report.fallback_paths.push(PathBuf::from(relative_display));
                return None;
            }
        };

        if !metadata.is_file() {
            report
                .fallback_reasons
                .push(format!("{relative_display} is not a file"));
            report.fallback_paths.push(PathBuf::from(relative_display));
            return None;
        }

        let mut bytes = Vec::new();
        if let Err(source) = file.take(self.max_file_bytes + 1).read_to_end(&mut bytes) {
            report
                .fallback_reasons
                .push(format!("{relative_display} could not be read: {source}"));
            report.fallback_paths.push(PathBuf::from(relative_display));
            return None;
        }
        if bytes.len() as u64 > self.max_file_bytes {
            report.fallback_reasons.push(format!(
                "{relative_display} exceeds max AST file size of {} bytes",
                self.max_file_bytes
            ));
            report.fallback_paths.push(PathBuf::from(relative_display));
            return None;
        }

        match String::from_utf8(bytes) {
            Ok(source) => Some(source),
            Err(error) => {
                report
                    .fallback_reasons
                    .push(format!("{relative_display} is not UTF-8: {error}"));
                report.fallback_paths.push(PathBuf::from(relative_display));
                None
            }
        }
    }

    fn canonical_root(&self) -> Result<PathBuf, MemoryError> {
        self.root
            .canonicalize()
            .map_err(|source| MemoryError::ResolvePath {
                path: self.root.clone(),
                source,
            })
    }

    fn resolve_requested_path(
        &self,
        repo_root: &Path,
        path: &Path,
    ) -> Result<Option<PathBuf>, MemoryError> {
        let candidate = requested_candidate(repo_root, path);
        let resolved = match candidate.canonicalize() {
            Ok(resolved) => resolved,
            Err(source) if source.kind() == ErrorKind::NotFound => {
                let normalized = normalize_lexical(&candidate);
                if !normalized.starts_with(repo_root) {
                    return Err(MemoryError::PathOutsideRepo {
                        path: normalized,
                        repo_root: repo_root.to_path_buf(),
                    });
                }
                return Ok(None);
            }
            Err(source) => {
                return Err(MemoryError::ResolvePath {
                    path: candidate.clone(),
                    source,
                });
            }
        };
        if !resolved.starts_with(repo_root) {
            return Err(MemoryError::PathOutsideRepo {
                path: resolved,
                repo_root: repo_root.to_path_buf(),
            });
        }
        Ok(Some(resolved))
    }

    fn relative_candidate_path(
        &self,
        repo_root: &Path,
        path: &Path,
    ) -> Result<PathBuf, MemoryError> {
        let candidate = normalize_lexical(&requested_candidate(repo_root, path));
        if !candidate.starts_with(repo_root) {
            return Err(MemoryError::PathOutsideRepo {
                path: candidate.clone(),
                repo_root: repo_root.to_path_buf(),
            });
        }
        Ok(candidate
            .strip_prefix(repo_root)
            .unwrap_or(&candidate)
            .to_path_buf())
    }
}

pub fn skipped_directory_name(path: impl AsRef<Path>) -> Option<&'static str> {
    let value = path.as_ref().file_name()?.to_str()?;
    SKIPPED_DIRECTORY_COMPONENTS
        .iter()
        .copied()
        .find(|skipped| value == *skipped)
}

fn requested_candidate(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

impl CodeIntelIndex for AstCodeIntelProvider {
    fn code_context(
        &self,
        paths: &[PathBuf],
        scope_refs: &[KnowledgeScope],
        limit: usize,
    ) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
        let report = self.code_context_report(paths, scope_refs, limit)?;
        let trace = report.trace_artifact(scope_refs, PROVIDER_NAME, report.ast_only_fallback());
        let mut artifacts = report.artifacts;
        artifacts.push(trace);
        Ok(artifacts)
    }
}

pub struct CompositeCodeIntelProvider {
    ast: AstCodeIntelProvider,
    fallback: CodebaseAnalyzer,
}

impl CompositeCodeIntelProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            ast: AstCodeIntelProvider::new(&root),
            fallback: CodebaseAnalyzer::new(root),
        }
    }

    pub fn code_context_with_symbol_kinds(
        &self,
        paths: &[PathBuf],
        scope_refs: &[KnowledgeScope],
        limit: usize,
        symbol_kinds: &BTreeSet<String>,
    ) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
        let mut report =
            self.ast
                .code_context_report_with_symbols(paths, scope_refs, limit, symbol_kinds)?;
        let use_fallback = !report.fallback_reasons.is_empty();
        let mut artifacts = std::mem::take(&mut report.artifacts);
        let fallback_paths = if paths.is_empty() {
            paths
        } else {
            &report.fallback_paths
        };
        let has_fallback_target = paths.is_empty() || !fallback_paths.is_empty();
        let fallback_limit = limit.saturating_sub(report.used_symbols);
        let mut fallback_used = false;

        if use_fallback && has_fallback_target {
            artifacts.extend(self.fallback.code_context(
                fallback_paths,
                scope_refs,
                fallback_limit,
            )?);
            fallback_used = true;
        }
        let trace = report.trace_artifact(
            scope_refs,
            "composite-code-intel",
            report.composite_fallback(fallback_used),
        );
        artifacts.push(trace);
        Ok(artifacts)
    }
}

impl CodeIntelIndex for CompositeCodeIntelProvider {
    fn code_context(
        &self,
        paths: &[PathBuf],
        scope_refs: &[KnowledgeScope],
        limit: usize,
    ) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
        let mut report = self.ast.code_context_report(paths, scope_refs, limit)?;
        let use_fallback = !report.fallback_reasons.is_empty();
        let mut artifacts = std::mem::take(&mut report.artifacts);
        let fallback_paths = if paths.is_empty() {
            paths
        } else {
            &report.fallback_paths
        };
        let has_fallback_target = paths.is_empty() || !fallback_paths.is_empty();
        let fallback_limit = limit.saturating_sub(report.used_symbols);
        let mut fallback_used = false;

        if use_fallback && has_fallback_target {
            artifacts.extend(self.fallback.code_context(
                fallback_paths,
                scope_refs,
                fallback_limit,
            )?);
            fallback_used = true;
        }
        let trace = report.trace_artifact(
            scope_refs,
            "composite-code-intel",
            report.composite_fallback(fallback_used),
        );
        artifacts.push(trace);
        Ok(artifacts)
    }
}

#[derive(Default)]
struct AstCodeIntelReport {
    artifacts: Vec<CodeIntelArtifact>,
    fallback_reasons: Vec<String>,
    fallback_paths: Vec<PathBuf>,
    parsed_files: usize,
    query_runs: usize,
    used_symbols: usize,
}

impl AstCodeIntelReport {
    fn trace_artifact(
        &self,
        scope_refs: &[KnowledgeScope],
        provider: &str,
        fallback: String,
    ) -> CodeIntelArtifact {
        CodeIntelArtifact {
            provider: provider.to_string(),
            kind: "trace".to_string(),
            scope_refs: scope_refs.to_vec(),
            source_refs: Vec::new(),
            path: None,
            commit_sha: None,
            title: "Code-intelligence trace".to_string(),
            summary: format!(
                "- parse: parsed {} file(s)\n- query: ran {} Tree-sitter query pack(s)\n- {fallback}",
                self.parsed_files, self.query_runs
            ),
        }
    }

    fn composite_fallback(&self, fallback_used: bool) -> String {
        match (self.fallback_reasons.is_empty(), fallback_used) {
            (true, _) => "fallback: CodebaseAnalyzer not used".to_string(),
            (false, true) => format!(
                "fallback: CodebaseAnalyzer used ({})",
                self.fallback_reasons.join("; ")
            ),
            (false, false) => format!(
                "fallback: CodebaseAnalyzer not used; no fallback target ({})",
                self.fallback_reasons.join("; ")
            ),
        }
    }

    fn ast_only_fallback(&self) -> String {
        if self.fallback_reasons.is_empty() {
            "fallback: CodebaseAnalyzer not used".to_string()
        } else {
            format!(
                "fallback: AST provider only; CodebaseAnalyzer fallback not available ({})",
                self.fallback_reasons.join("; ")
            )
        }
    }
}

fn ast_artifacts_for_summary(
    summary: ParsedDocumentSummary,
    relative_path: &Path,
    relative_display: &str,
    scope_refs: &[KnowledgeScope],
    commit_sha: Option<String>,
    symbol_limit: usize,
    symbol_kinds: &BTreeSet<String>,
) -> (Vec<CodeIntelArtifact>, usize) {
    let diagnostic_summary = diagnostics_summary(&summary.diagnostics);
    let mut artifacts = vec![CodeIntelArtifact {
        provider: PROVIDER_NAME.to_string(),
        kind: "ast-summary".to_string(),
        scope_refs: scope_refs.to_vec(),
        source_refs: vec![MemorySourceRef {
            kind: "path".to_string(),
            id: relative_display.to_string(),
            url: None,
        }],
        path: Some(relative_path.to_path_buf()),
        commit_sha: commit_sha.clone(),
        title: relative_display.to_string(),
        summary: format!(
            "- Language: {}\n- Content hash: sha256:{}\n- Parser: {} ({}, {})\n- Query pack: {}\n- Diagnostics: {diagnostic_summary}",
            source_language_label(summary.source.language),
            summary.source.sha256,
            summary.versions.provider,
            summary.versions.grammar,
            summary.versions.tree_sitter,
            summary.versions.query_pack,
        ),
    }];

    if !summary.symbols.is_empty() && symbol_limit > 0 {
        let max_symbols = symbol_limit;
        let selected_symbols = summary
            .symbols
            .iter()
            .filter(|symbol| {
                symbol_kinds.is_empty() || symbol_kinds.contains(symbol_kind_label(&symbol.kind))
            })
            .take(max_symbols)
            .collect::<Vec<_>>();
        let used_symbols = selected_symbols.len();
        if selected_symbols.is_empty() {
            return (artifacts, 0);
        }
        let symbols = selected_symbols
            .iter()
            .map(|symbol| {
                format!(
                    "- {} `{}` at {}:{}",
                    symbol_kind_label(&symbol.kind),
                    symbol.name,
                    relative_display,
                    symbol.rendered_span
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        artifacts.push(CodeIntelArtifact {
            provider: PROVIDER_NAME.to_string(),
            kind: "ast-symbols".to_string(),
            scope_refs: scope_refs.to_vec(),
            source_refs: selected_symbols
                .iter()
                .map(|symbol| MemorySourceRef {
                    kind: "code-symbol".to_string(),
                    id: format!("{relative_display}:{}", symbol.rendered_span),
                    url: None,
                })
                .collect(),
            path: Some(relative_path.to_path_buf()),
            commit_sha,
            title: format!("Symbols in {relative_display}"),
            summary: symbols,
        });
        return (artifacts, used_symbols);
    }

    (artifacts, 0)
}

fn diagnostics_summary(diagnostics: &[AstDiagnostic]) -> String {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == AstDiagnosticKind::Error)
        .count();
    let missing = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == AstDiagnosticKind::Missing)
        .count();
    format!("{errors} ERROR, {missing} MISSING")
}

fn source_language_label(language: SourceLanguage) -> &'static str {
    match language {
        SourceLanguage::Rust => "rust",
        SourceLanguage::TypeScript => "typescript",
        SourceLanguage::Tsx => "tsx",
        SourceLanguage::JavaScript => "javascript",
        SourceLanguage::Jsx => "jsx",
        SourceLanguage::Python => "python",
        SourceLanguage::Json => "json",
        SourceLanguage::Yaml => "yaml",
        SourceLanguage::Toml => "toml",
        SourceLanguage::Markdown => "markdown",
    }
}

fn symbol_kind_label(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "module",
        SymbolKind::Class => "class",
        SymbolKind::Function => "function",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Type => "type",
        SymbolKind::Method => "method",
        SymbolKind::Constructor => "constructor",
        SymbolKind::Field => "field",
        SymbolKind::Variable => "variable",
        SymbolKind::Constant => "constant",
        SymbolKind::Test => "test",
        SymbolKind::Document => "document",
    }
}

fn git_commit_sha(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

pub fn detect_language(path: impl AsRef<Path>) -> Option<SourceLanguage> {
    let path = path.as_ref();
    match path.file_name().and_then(|file_name| file_name.to_str()) {
        Some("Cargo.toml") => return Some(SourceLanguage::Toml),
        Some("package.json") => return Some(SourceLanguage::Json),
        Some("AGENTS.md" | "WORKFLOW.md") => return Some(SourceLanguage::Markdown),
        _ => {}
    }

    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("rs") => Some(SourceLanguage::Rust),
        Some("ts" | "mts" | "cts") => Some(SourceLanguage::TypeScript),
        Some("tsx") => Some(SourceLanguage::Tsx),
        Some("js" | "mjs" | "cjs") => Some(SourceLanguage::JavaScript),
        Some("jsx") => Some(SourceLanguage::Jsx),
        Some("py" | "pyi" | "pyw") => Some(SourceLanguage::Python),
        Some("json") => Some(SourceLanguage::Json),
        Some("yaml" | "yml") => Some(SourceLanguage::Yaml),
        Some("toml") => Some(SourceLanguage::Toml),
        Some("md" | "markdown") => Some(SourceLanguage::Markdown),
        _ => None,
    }
}

pub fn parse_path(
    path: impl AsRef<Path>,
    source: &str,
) -> Result<ParsedDocumentSummary, CodeIntelError> {
    let path = path.as_ref();
    let language = detect_language(path).ok_or_else(|| CodeIntelError::UnsupportedLanguage {
        path: path.display().to_string(),
    })?;
    parse_source(language, Some(path.to_path_buf()), source)
}

pub fn parse_rust_source(
    path: Option<PathBuf>,
    source: &str,
) -> Result<ParsedDocumentSummary, CodeIntelError> {
    parse_source(SourceLanguage::Rust, path, source)
}

pub fn parse_source(
    language: SourceLanguage,
    path: Option<PathBuf>,
    source: &str,
) -> Result<ParsedDocumentSummary, CodeIntelError> {
    if language.is_lightweight() {
        return Ok(parse_lightweight_source(language, path, source));
    }

    let config = language_config(language);
    let parser_language = (config.parser)();
    let mut parser = Parser::new();
    parser.set_language(&parser_language)?;
    let tree = parser.parse(source, None).ok_or(CodeIntelError::Parse)?;
    let root = tree.root_node();
    let metadata = load_metadata(config)?;
    let queries = compile_query_pack(config, &parser_language, &metadata)?;
    let (symbols, captures) = run_query_pack(&queries, language, root, source.as_bytes())?;

    Ok(ParsedDocumentSummary {
        source: SourceIdentity {
            language,
            path,
            bytes: source.len(),
            sha256: source_sha256(source),
        },
        versions: ParserVersions {
            provider: PROVIDER_NAME.to_string(),
            tree_sitter: TREE_SITTER_VERSION.to_string(),
            grammar: format!("{}-{}", config.grammar_crate, config.grammar_version),
            query_pack: metadata.version.clone(),
        },
        root_kind: root.kind().to_string(),
        has_errors: root.has_error(),
        symbols,
        captures,
        diagnostics: collect_diagnostics(root),
    })
}

pub fn run_ad_hoc_query(
    language: SourceLanguage,
    source: &str,
    query_source: &str,
    limit: usize,
) -> Result<Vec<AdHocQueryMatch>, CodeIntelError> {
    if !language.supports_ast_queries() {
        return Err(CodeIntelError::UnsupportedLanguage {
            path: language.id().to_string(),
        });
    }
    let config = language_config(language);
    let parser_language = (config.parser)();
    let mut parser = Parser::new();
    parser.set_language(&parser_language)?;
    let tree = parser.parse(source, None).ok_or(CodeIntelError::Parse)?;
    let query =
        Query::new(&parser_language, query_source).map_err(|source| CodeIntelError::Query {
            query_name: "ad_hoc".to_string(),
            source,
        })?;

    let mut results = Vec::new();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
    while let Some(query_match) = matches.next() {
        if results.len() >= limit {
            break;
        }
        let captures = query_match
            .captures
            .iter()
            .map(|capture| {
                let span = one_based_span(capture.node);
                let rendered_span = span.render();
                Ok(CaptureRecord {
                    query_name: "ad_hoc".to_string(),
                    capture_name: query.capture_names()[capture.index as usize].to_string(),
                    text: capture.node.utf8_text(source.as_bytes())?.to_string(),
                    span,
                    rendered_span,
                })
            })
            .collect::<Result<Vec<_>, CodeIntelError>>()?;
        results.push(AdHocQueryMatch { captures });
    }
    Ok(results)
}

fn language_config(language: SourceLanguage) -> LanguageConfig {
    match language {
        SourceLanguage::Rust => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-rust",
            grammar_version: RUST_GRAMMAR_VERSION,
            query_pack_version: RUST_QUERY_PACK_VERSION,
            metadata: RUST_METADATA,
            shared_queries: RUST_SHARED_QUERIES,
            variant_queries: NO_VARIANT_QUERIES,
            diagnostic_queries: RUST_DIAGNOSTIC_QUERIES,
            parser: || tree_sitter_rust::LANGUAGE.into(),
        },
        SourceLanguage::TypeScript => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-typescript",
            grammar_version: TYPESCRIPT_GRAMMAR_VERSION,
            query_pack_version: TYPESCRIPT_QUERY_PACK_VERSION,
            metadata: TYPESCRIPT_METADATA,
            shared_queries: TYPESCRIPT_SHARED_QUERIES,
            variant_queries: TYPESCRIPT_INJECTION_QUERIES,
            diagnostic_queries: TYPESCRIPT_DIAGNOSTIC_QUERIES,
            parser: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        },
        SourceLanguage::Tsx => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-typescript",
            grammar_version: TYPESCRIPT_GRAMMAR_VERSION,
            query_pack_version: TSX_QUERY_PACK_VERSION,
            metadata: TSX_METADATA,
            shared_queries: TYPESCRIPT_SHARED_QUERIES,
            variant_queries: TSX_INJECTION_QUERIES,
            diagnostic_queries: TYPESCRIPT_DIAGNOSTIC_QUERIES,
            parser: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        },
        SourceLanguage::JavaScript => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-javascript",
            grammar_version: JAVASCRIPT_GRAMMAR_VERSION,
            query_pack_version: JAVASCRIPT_QUERY_PACK_VERSION,
            metadata: JAVASCRIPT_METADATA,
            shared_queries: JAVASCRIPT_SHARED_QUERIES,
            variant_queries: NO_VARIANT_QUERIES,
            diagnostic_queries: JAVASCRIPT_DIAGNOSTIC_QUERIES,
            parser: || tree_sitter_javascript::LANGUAGE.into(),
        },
        SourceLanguage::Jsx => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-javascript",
            grammar_version: JAVASCRIPT_GRAMMAR_VERSION,
            query_pack_version: JSX_QUERY_PACK_VERSION,
            metadata: JSX_METADATA,
            shared_queries: JAVASCRIPT_SHARED_QUERIES,
            variant_queries: JSX_INJECTION_QUERIES,
            diagnostic_queries: JAVASCRIPT_DIAGNOSTIC_QUERIES,
            parser: || tree_sitter_javascript::LANGUAGE.into(),
        },
        SourceLanguage::Python => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-python",
            grammar_version: PYTHON_GRAMMAR_VERSION,
            query_pack_version: PYTHON_QUERY_PACK_VERSION,
            metadata: PYTHON_METADATA,
            shared_queries: PYTHON_SHARED_QUERIES,
            variant_queries: NO_VARIANT_QUERIES,
            diagnostic_queries: PYTHON_DIAGNOSTIC_QUERIES,
            parser: || tree_sitter_python::LANGUAGE.into(),
        },
        _ => unreachable!("lightweight languages do not have tree-sitter configs"),
    }
}

fn load_metadata(config: LanguageConfig) -> Result<QueryPackMetadata, CodeIntelError> {
    let metadata: QueryPackMetadata = toml::from_str(config.metadata)?;
    metadata_matches(config.language.id(), &metadata.language, "language")?;
    metadata_matches(
        config.grammar_crate,
        &metadata.grammar_crate,
        "grammar_crate",
    )?;
    metadata_matches(
        config.grammar_version,
        &metadata.grammar_version,
        "grammar_version",
    )?;
    metadata_matches("tree-sitter", &metadata.parser_crate, "parser_crate")?;
    metadata_matches(
        TREE_SITTER_VERSION,
        &metadata.parser_version,
        "parser_version",
    )?;
    metadata_matches(
        config.query_pack_version,
        &metadata.version,
        "query_pack_version",
    )?;
    Ok(metadata)
}

fn metadata_matches(expected: &str, actual: &str, language: &str) -> Result<(), CodeIntelError> {
    if expected == actual {
        Ok(())
    } else {
        Err(CodeIntelError::MetadataMismatch {
            language: language.to_string(),
            expected: expected.to_string(),
            actual: actual.to_string(),
        })
    }
}

fn compile_query_pack<'a>(
    config: LanguageConfig,
    language: &Language,
    metadata: &'a QueryPackMetadata,
) -> Result<Vec<CompiledQuery<'a>>, CodeIntelError> {
    config
        .shared_queries
        .iter()
        .chain(config.variant_queries.iter())
        .chain(config.diagnostic_queries.iter())
        .map(|asset| compile_query(*asset, language, metadata))
        .collect()
}

fn compile_query<'a>(
    asset: QueryAsset,
    language: &Language,
    metadata: &'a QueryPackMetadata,
) -> Result<CompiledQuery<'a>, CodeIntelError> {
    let query = Query::new(language, asset.source).map_err(|source| CodeIntelError::Query {
        query_name: asset.name.to_string(),
        source,
    })?;
    validate_capture_names(asset.name, &query, metadata)?;
    Ok(CompiledQuery {
        asset,
        query,
        metadata,
    })
}

fn validate_capture_names(
    query_name: &str,
    query: &Query,
    metadata: &QueryPackMetadata,
) -> Result<(), CodeIntelError> {
    for capture_name in query.capture_names().iter().copied() {
        if !STANDARD_CAPTURE_NAMES.contains(&capture_name) {
            return Err(CodeIntelError::NonstandardCapture {
                query_name: query_name.to_string(),
                capture_name: capture_name.to_string(),
            });
        }
        if !metadata.captures.contains_key(capture_name) {
            return Err(CodeIntelError::UndocumentedCapture {
                query_name: query_name.to_string(),
                capture_name: capture_name.to_string(),
            });
        }
    }
    Ok(())
}

fn run_query_pack(
    queries: &[CompiledQuery<'_>],
    language: SourceLanguage,
    root: Node<'_>,
    source: &[u8],
) -> Result<(Vec<SymbolRecord>, Vec<CaptureRecord>), CodeIntelError> {
    let mut symbols = Vec::new();
    let mut captures = Vec::new();

    for compiled in queries {
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&compiled.query, root, source);
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                let capture_name =
                    compiled.query.capture_names()[capture.index as usize].to_string();
                let text = capture.node.utf8_text(source)?.to_string();
                if capture_name == "doc.comment" && !is_doc_comment(language, &text) {
                    continue;
                }
                let span = one_based_span(capture.node);
                let rendered_span = span.render();

                if let Some(kind) =
                    symbol_kind_for_capture(language, &capture_name, capture.node, source)
                    && let Some(name) =
                        symbol_name_for_capture(&capture_name, capture.node, source)?
                {
                    symbols.push(SymbolRecord {
                        kind,
                        name,
                        span: span.clone(),
                        rendered_span: rendered_span.clone(),
                        parser_version: TREE_SITTER_VERSION.to_string(),
                        query_pack_version: compiled.metadata.version.clone(),
                    });
                }

                captures.push(CaptureRecord {
                    query_name: compiled.asset.name.to_string(),
                    capture_name,
                    text,
                    span,
                    rendered_span,
                });
            }
        }
    }

    Ok((symbols, captures))
}

fn symbol_kind_for_capture(
    language: SourceLanguage,
    capture_name: &str,
    node: Node<'_>,
    source: &[u8],
) -> Option<SymbolKind> {
    match capture_name {
        "definition.module" => Some(SymbolKind::Module),
        "definition.class" => Some(SymbolKind::Class),
        "definition.struct" => Some(SymbolKind::Struct),
        "definition.enum" => Some(SymbolKind::Enum),
        "definition.trait" => Some(SymbolKind::Trait),
        "definition.interface" => Some(SymbolKind::Interface),
        "definition.type" => Some(SymbolKind::Type),
        "definition.method"
            if language == SourceLanguage::Rust
                && node.kind() == "function_signature_item"
                && !has_ancestor(node, "trait_item") =>
        {
            Some(SymbolKind::Function)
        }
        "definition.method" => Some(SymbolKind::Method),
        "definition.constructor" => Some(SymbolKind::Constructor),
        "definition.field" => Some(SymbolKind::Field),
        "definition.variable" => Some(SymbolKind::Variable),
        "definition.constant" => Some(SymbolKind::Constant),
        "definition.test" | "test.case" => Some(SymbolKind::Test),
        "definition.function" if is_test_function(language, node, source) => Some(SymbolKind::Test),
        "definition.function" if is_method(language, node) => Some(SymbolKind::Method),
        "definition.function" => Some(SymbolKind::Function),
        _ => None,
    }
}

fn symbol_name_for_capture(
    capture_name: &str,
    node: Node<'_>,
    source: &[u8],
) -> Result<Option<String>, CodeIntelError> {
    if capture_name == "test.case"
        && let Some(name) = test_case_name_from_call(node, source)?
    {
        return Ok(Some(name));
    }
    if let Some(name) = node.child_by_field_name("name") {
        return Ok(Some(name.utf8_text(source)?.to_string()));
    }
    if matches!(capture_name, "test.case" | "reference.call")
        && let Some(function) = node.child_by_field_name("function")
    {
        return Ok(Some(function.utf8_text(source)?.to_string()));
    }
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "property_identifier"
    ) {
        return Ok(Some(node.utf8_text(source)?.to_string()));
    }
    Ok(None)
}

fn test_case_name_from_call(
    node: Node<'_>,
    source: &[u8],
) -> Result<Option<String>, CodeIntelError> {
    let Some(call) = node
        .parent()
        .filter(|parent| parent.kind() == "call_expression")
    else {
        return Ok(None);
    };
    let Some(arguments) = call.child_by_field_name("arguments") else {
        return Ok(None);
    };

    let mut cursor = arguments.walk();
    for argument in arguments.named_children(&mut cursor) {
        if matches!(argument.kind(), "string" | "template_string") {
            return first_string_fragment(argument, source);
        }
    }
    Ok(None)
}

fn first_string_fragment(
    string_node: Node<'_>,
    source: &[u8],
) -> Result<Option<String>, CodeIntelError> {
    let mut name = String::new();
    let mut cursor = string_node.walk();
    for child in string_node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "string_fragment" | "escape_sequence" | "template_substitution"
        ) {
            name.push_str(child.utf8_text(source)?);
        }
    }
    if !name.is_empty() {
        return Ok(Some(name));
    }

    let text = string_node.utf8_text(source)?;
    Ok(Some(
        text.trim_matches(|character| matches!(character, '"' | '\'' | '`'))
            .to_string(),
    ))
}

fn is_method(language: SourceLanguage, node: Node<'_>) -> bool {
    match language {
        SourceLanguage::Rust => has_ancestor(node, "impl_item"),
        SourceLanguage::Python => is_python_class_body_function(node),
        _ => matches!(node.kind(), "method_definition"),
    }
}

fn is_doc_comment(language: SourceLanguage, text: &str) -> bool {
    if language != SourceLanguage::Rust {
        return true;
    }
    matches!(
        text.trim_start(),
        comment if comment.starts_with("///")
            || comment.starts_with("//!")
            || comment.starts_with("/**")
            || comment.starts_with("/*!")
    )
}

fn is_python_class_body_function(node: Node<'_>) -> bool {
    let class_body = match node.parent() {
        Some(parent) if parent.kind() == "block" => parent,
        Some(parent) if parent.kind() == "decorated_definition" => match parent.parent() {
            Some(block) if block.kind() == "block" => block,
            _ => return false,
        },
        _ => return false,
    };

    class_body
        .parent()
        .is_some_and(|parent| parent.kind() == "class_definition")
}

fn is_test_function(language: SourceLanguage, node: Node<'_>, source: &[u8]) -> bool {
    match language {
        SourceLanguage::Rust => has_test_attribute(node, source),
        SourceLanguage::Python => node
            .child_by_field_name("name")
            .and_then(|name| name.utf8_text(source).ok())
            .map(|name| name.starts_with("test_"))
            .unwrap_or(false),
        _ => false,
    }
}

fn has_ancestor(mut node: Node<'_>, kind: &str) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == kind {
            return true;
        }
        node = parent;
    }
    false
}

fn has_test_attribute(node: Node<'_>, source: &[u8]) -> bool {
    let mut sibling = node.prev_named_sibling();
    while let Some(candidate) = sibling {
        if is_comment(candidate) {
            sibling = candidate.prev_named_sibling();
            continue;
        }
        if candidate.kind() != "attribute_item" {
            return false;
        }
        if candidate
            .utf8_text(source)
            .map(|text| text.trim() == "#[test]")
            .unwrap_or(false)
        {
            return true;
        }
        sibling = candidate.prev_named_sibling();
    }
    false
}

fn is_comment(node: Node<'_>) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment" | "comment")
}

fn collect_diagnostics(root: Node<'_>) -> Vec<AstDiagnostic> {
    let mut diagnostics = Vec::new();
    collect_diagnostics_from(root, &mut diagnostics);
    diagnostics
}

fn collect_diagnostics_from(node: Node<'_>, diagnostics: &mut Vec<AstDiagnostic>) {
    if node.is_error() || node.is_missing() {
        let kind = if node.is_missing() {
            AstDiagnosticKind::Missing
        } else {
            AstDiagnosticKind::Error
        };
        let span = one_based_span(node);
        diagnostics.push(AstDiagnostic {
            kind,
            node_kind: node.kind().to_string(),
            rendered_span: span.render(),
            span,
        });
    }

    let walk_all_children = node.is_error() || node.is_missing();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if walk_all_children || child.has_error() || child.is_error() || child.is_missing() {
            collect_diagnostics_from(child, diagnostics);
        }
    }
}

fn parse_lightweight_source(
    language: SourceLanguage,
    path: Option<PathBuf>,
    source: &str,
) -> ParsedDocumentSummary {
    let span = source_span_for_text(source);
    let query_pack = format!("{}-lightweight-v1", language.id());
    let mut captures = Vec::new();
    if language == SourceLanguage::Markdown {
        captures.extend(markdown_fence_captures(source));
    }
    // Lightweight config/doc languages record file presence and spans only.
    // JSON, YAML, and TOML are not validated by this fast path.

    ParsedDocumentSummary {
        source: SourceIdentity {
            language,
            path: path.clone(),
            bytes: source.len(),
            sha256: source_sha256(source),
        },
        versions: ParserVersions {
            provider: LIGHTWEIGHT_PROVIDER_NAME.to_string(),
            tree_sitter: LIGHTWEIGHT_TREE_SITTER_VERSION.to_string(),
            grammar: "lightweight-text".to_string(),
            query_pack: query_pack.clone(),
        },
        root_kind: "document".to_string(),
        has_errors: false,
        symbols: vec![SymbolRecord {
            kind: SymbolKind::Document,
            name: path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|file_name| file_name.to_str())
                .unwrap_or_else(|| language.display_name())
                .to_string(),
            rendered_span: span.render(),
            span,
            parser_version: LIGHTWEIGHT_PARSER_VERSION.to_string(),
            query_pack_version: query_pack,
        }],
        captures,
        diagnostics: Vec::new(),
    }
}

fn markdown_fence_captures(source: &str) -> Vec<CaptureRecord> {
    let mut captures = Vec::new();
    let mut byte_offset = 0;
    let mut open_fence: Option<MarkdownFence> = None;

    for (line_index, line) in source.split_inclusive('\n').enumerate() {
        let line_number = line_index + 1;
        let line_body = markdown_line_body(line);
        let indent = line_body
            .as_bytes()
            .iter()
            .take_while(|byte| **byte == b' ')
            .count();
        if indent > 3 {
            byte_offset += line.len();
            continue;
        }
        let trimmed = &line_body[indent..];
        if let Some((fence_marker, fence_marker_count, rest)) = markdown_fence_marker(trimmed) {
            if open_fence.as_ref().is_some_and(|fence| {
                is_markdown_closing_fence(
                    trimmed,
                    indent,
                    fence.indent,
                    fence.marker,
                    fence.marker_count,
                )
            }) {
                let fence = open_fence.take().expect("checked open fence");
                let span = SourceSpan {
                    start_byte: fence.content_start_byte,
                    end_byte: byte_offset,
                    start_line: fence.content_start_line,
                    start_column: 1,
                    end_line: line_number,
                    end_column: 1,
                };
                captures.push(CaptureRecord {
                    query_name: MARKDOWN_FENCE_QUERY_NAME.to_string(),
                    capture_name: "injection.content".to_string(),
                    text: markdown_capture_text(&source[fence.content_start_byte..byte_offset]),
                    rendered_span: span.render(),
                    span,
                });
                if let Some((language, language_span)) = fence.language {
                    captures.push(CaptureRecord {
                        query_name: MARKDOWN_FENCE_QUERY_NAME.to_string(),
                        capture_name: "injection.language".to_string(),
                        text: language,
                        rendered_span: language_span.render(),
                        span: language_span,
                    });
                }
            } else if open_fence.is_none() {
                let language = markdown_fence_language(rest).map(|(language, start_in_rest)| {
                    let language_start_byte =
                        byte_offset + indent + fence_marker_count + start_in_rest;
                    let language_span = SourceSpan {
                        start_byte: language_start_byte,
                        end_byte: language_start_byte + language.len(),
                        start_line: line_number,
                        start_column: indent + fence_marker_count + 1 + start_in_rest,
                        end_line: line_number,
                        end_column: indent
                            + fence_marker_count
                            + 1
                            + start_in_rest
                            + language.len(),
                    };
                    (language.to_string(), language_span)
                });
                open_fence = Some(MarkdownFence {
                    marker: fence_marker,
                    marker_count: fence_marker_count,
                    indent,
                    language,
                    content_start_byte: byte_offset + line.len(),
                    content_start_line: line_number + 1,
                });
            }
        }
        byte_offset += line.len();
    }

    if let Some(fence) = open_fence.take() {
        let source_span = source_span_for_text(source);
        let span = SourceSpan {
            start_byte: fence.content_start_byte,
            end_byte: source.len(),
            start_line: fence.content_start_line,
            start_column: 1,
            end_line: source_span.end_line,
            end_column: source_span.end_column,
        };
        captures.push(CaptureRecord {
            query_name: MARKDOWN_FENCE_QUERY_NAME.to_string(),
            capture_name: "injection.content".to_string(),
            text: markdown_capture_text(&source[fence.content_start_byte..]),
            rendered_span: span.render(),
            span,
        });
        if let Some((language, language_span)) = fence.language {
            captures.push(CaptureRecord {
                query_name: MARKDOWN_FENCE_QUERY_NAME.to_string(),
                capture_name: "injection.language".to_string(),
                text: language,
                rendered_span: language_span.render(),
                span: language_span,
            });
        }
    }

    captures
}

fn markdown_line_body(line: &str) -> &str {
    let without_lf = line.strip_suffix('\n').unwrap_or(line);
    without_lf.strip_suffix('\r').unwrap_or(without_lf)
}

fn markdown_capture_text(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn markdown_fence_marker(trimmed_line: &str) -> Option<(u8, usize, &str)> {
    let marker = match trimmed_line.as_bytes().first().copied()? {
        b'`' => b'`',
        b'~' => b'~',
        _ => return None,
    };
    let marker_count = trimmed_line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    let rest = &trimmed_line[marker_count..];
    (marker_count >= 3 && !rest.as_bytes().contains(&marker)).then_some((
        marker,
        marker_count,
        rest,
    ))
}

fn markdown_fence_language(rest: &str) -> Option<(&str, usize)> {
    let language = rest.split_whitespace().next()?;
    rest.find(language).map(|start| (language, start))
}

fn is_markdown_closing_fence(
    trimmed_line: &str,
    closing_indent: usize,
    opening_indent: usize,
    opening_marker: u8,
    opening_marker_count: usize,
) -> bool {
    if closing_indent < opening_indent {
        return false;
    }

    markdown_fence_marker(trimmed_line)
        .map(|(marker, marker_count, rest)| {
            marker == opening_marker
                && marker_count >= opening_marker_count
                && rest.trim().is_empty()
        })
        .unwrap_or(false)
}

struct MarkdownFence {
    marker: u8,
    marker_count: usize,
    indent: usize,
    language: Option<(String, SourceSpan)>,
    content_start_byte: usize,
    content_start_line: usize,
}

fn source_sha256(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn one_based_span(node: Node<'_>) -> SourceSpan {
    let start = node.start_position();
    let end = node.end_position();
    SourceSpan {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
        start_line: start.row + 1,
        start_column: start.column + 1,
        end_line: end.row + 1,
        end_column: end.column + 1,
    }
}

fn source_span_for_text(source: &str) -> SourceSpan {
    let mut line_count = 1;
    let mut last_line_start = 0;
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_count += 1;
            last_line_start = index + 1;
        }
    }
    SourceSpan {
        start_byte: 0,
        end_byte: source.len(),
        start_line: 1,
        start_column: 1,
        end_line: line_count,
        end_column: source.len() - last_line_start + 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_memory::CodeIntelIndex;
    use tempfile::TempDir;

    const RUST_COMPLETE: &str = include_str!("../fixtures/rust/complete.rs");
    const RUST_IMPORTS_CALLS: &str = include_str!("../fixtures/rust/imports_calls.rs");
    const RUST_MALFORMED_ERROR: &str = include_str!("../fixtures/rust/malformed_error.rs");
    const RUST_MALFORMED_MISSING: &str = include_str!("../fixtures/rust/malformed_missing.rs");
    const TSX_REACT: &str = include_str!("../fixtures/typescript/react_component.tsx");
    const TS_IMPORTS: &str = include_str!("../fixtures/typescript/imports.ts");
    const JSX_MODULE: &str = include_str!("../fixtures/javascript/module.jsx");
    const PYTHON_BASIC: &str = include_str!("../fixtures/python/basic_symbols.py");
    const PYTHON_MALFORMED: &str = include_str!("../fixtures/python/malformed.py");
    const JSON_CONFIG: &str = include_str!("../fixtures/documents/config.json");
    const YAML_CONFIG: &str = include_str!("../fixtures/documents/config.yaml");
    const TOML_CONFIG: &str = include_str!("../fixtures/documents/config.toml");
    const MARKDOWN_NOTES: &str = include_str!("../fixtures/documents/notes.md");
    const FOUR_BACKTICK_MARKDOWN: &str = "````ts\n```not close\nvalue()\n````\n";
    const MARKDOWN_INFO_STRING: &str = "```python not-valid\nprint('ok')\n```\n";
    const MARKDOWN_UNLABELED: &str = "```\nraw\n```\n";
    const MARKDOWN_INDENTED: &str = "    ```ignored\nraw\n    ```\n";
    const MARKDOWN_CRLF: &str = "```python\r\nprint('ok')\r\n```\r\n";
    const MARKDOWN_TILDE: &str = "~~~python\nprint('tilde')\n~~~\n";
    const MARKDOWN_UNCLOSED: &str = "```python\nprint('open')\n";
    const MARKDOWN_TAB_INDENTED: &str = "\t```python\nprint('tab')\n\t```\n";
    const MARKDOWN_CLOSING_INDENT_TOO_SMALL: &str = "  ```rs\nvalue\n```\n";
    const MARKDOWN_BACKTICK_INFO: &str = "```python `invalid`\nprint('not fenced')\n";
    const RUST_EXTERN_SIGNATURE: &str = "extern \"C\" {\n    fn puts(message: *const i8) -> i32;\n}\n\ntrait Runnable {\n    fn run(&self);\n}\n";
    const RUST_DOC_FILTER: &str =
        "/// Real docs\nfn documented() {}\n// Ordinary comment\nfn plain() {}\n";

    #[test]
    fn detects_supported_languages_by_extension_and_name() {
        assert_eq!(detect_language("src/lib.rs"), Some(SourceLanguage::Rust));
        assert_eq!(
            detect_language("src/app.ts"),
            Some(SourceLanguage::TypeScript)
        );
        assert_eq!(
            detect_language("src/app.mts"),
            Some(SourceLanguage::TypeScript)
        );
        assert_eq!(
            detect_language("src/app.cts"),
            Some(SourceLanguage::TypeScript)
        );
        assert_eq!(detect_language("src/app.tsx"), Some(SourceLanguage::Tsx));
        assert_eq!(
            detect_language("src/app.js"),
            Some(SourceLanguage::JavaScript)
        );
        assert_eq!(detect_language("src/app.jsx"), Some(SourceLanguage::Jsx));
        assert_eq!(detect_language("script.py"), Some(SourceLanguage::Python));
        assert_eq!(detect_language("script.pyi"), Some(SourceLanguage::Python));
        assert_eq!(detect_language("script.pyw"), Some(SourceLanguage::Python));
        assert_eq!(detect_language("package.json"), Some(SourceLanguage::Json));
        assert_eq!(detect_language("config.yaml"), Some(SourceLanguage::Yaml));
        assert_eq!(detect_language("Cargo.toml"), Some(SourceLanguage::Toml));
        assert_eq!(detect_language("AGENTS.md"), Some(SourceLanguage::Markdown));
    }

    #[test]
    fn all_query_packs_compile_and_use_standard_captures() {
        for language in [
            SourceLanguage::Rust,
            SourceLanguage::TypeScript,
            SourceLanguage::Tsx,
            SourceLanguage::JavaScript,
            SourceLanguage::Jsx,
            SourceLanguage::Python,
        ] {
            let config = language_config(language);
            let metadata = load_metadata(config).expect("metadata loads");
            let parser_language = (config.parser)();
            compile_query_pack(config, &parser_language, &metadata).expect("queries compile");
        }
    }

    #[test]
    fn grammar_variants_reuse_base_assets_and_keep_injections_explicit() {
        let typescript = language_config(SourceLanguage::TypeScript);
        let tsx = language_config(SourceLanguage::Tsx);
        assert!(std::ptr::eq(
            typescript.shared_queries.as_ptr(),
            tsx.shared_queries.as_ptr()
        ));
        assert!(std::ptr::eq(
            typescript.diagnostic_queries.as_ptr(),
            tsx.diagnostic_queries.as_ptr()
        ));
        assert_eq!(
            query_asset_names(typescript.shared_queries),
            vec!["definitions", "imports", "calls", "tests", "docs", "locals"]
        );
        assert_eq!(typescript.variant_queries.len(), 1);
        assert_eq!(tsx.variant_queries.len(), 1);
        assert_eq!(
            typescript.variant_queries[0].source.trim(),
            "(template_string) @injection.content"
        );
        assert_eq!(
            tsx.variant_queries[0].source.trim(),
            "(jsx_element) @injection.content"
        );

        let javascript = language_config(SourceLanguage::JavaScript);
        let jsx = language_config(SourceLanguage::Jsx);
        assert!(std::ptr::eq(
            javascript.shared_queries.as_ptr(),
            jsx.shared_queries.as_ptr()
        ));
        assert!(std::ptr::eq(
            javascript.diagnostic_queries.as_ptr(),
            jsx.diagnostic_queries.as_ptr()
        ));
        assert!(javascript.variant_queries.is_empty());
        assert_eq!(jsx.variant_queries.len(), 1);
        assert_eq!(
            jsx.variant_queries[0].source.trim(),
            "(jsx_element) @injection.content"
        );
    }

    #[test]
    fn invalid_node_type_fails_query_validation() {
        let config = language_config(SourceLanguage::Rust);
        let metadata = load_metadata(config).expect("metadata loads");
        let language = (config.parser)();
        let query = QueryAsset::new("invalid", "(not_a_real_node) @definition.function");

        let Err(error) = compile_query(query, &language, &metadata) else {
            panic!("invalid node type fails");
        };

        assert!(matches!(error, CodeIntelError::Query { .. }));
    }

    #[test]
    fn invalid_field_fails_query_validation() {
        let config = language_config(SourceLanguage::Rust);
        let metadata = load_metadata(config).expect("metadata loads");
        let language = (config.parser)();
        let query = QueryAsset::new(
            "invalid",
            "(function_item not_a_real_field: (identifier) @definition.function)",
        );

        let Err(error) = compile_query(query, &language, &metadata) else {
            panic!("invalid field fails");
        };

        assert!(matches!(error, CodeIntelError::Query { .. }));
    }

    #[test]
    fn nonstandard_capture_fails_query_validation() {
        let config = language_config(SourceLanguage::Rust);
        let metadata = load_metadata(config).expect("metadata loads");
        let language = (config.parser)();
        let query = QueryAsset::new("invalid", "(function_item) @custom.capture");

        let Err(error) = compile_query(query, &language, &metadata) else {
            panic!("nonstandard capture fails");
        };

        assert!(matches!(error, CodeIntelError::NonstandardCapture { .. }));
    }

    #[test]
    fn rust_fixture_extracts_expected_symbols() {
        let summary = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/complete.rs")),
            RUST_COMPLETE,
        )
        .expect("fixture parses");

        assert!(!summary.has_errors);
        assert_eq!(summary.root_kind, "source_file");
        assert_eq!(summary.versions.provider, PROVIDER_NAME);
        assert_eq!(summary.versions.tree_sitter, TREE_SITTER_VERSION);
        assert_eq!(summary.versions.grammar, "tree-sitter-rust-0.24.2");
        assert_eq!(summary.versions.query_pack, "rust-query-pack-v2");

        let symbols = symbol_tuples(&summary);
        assert!(symbols.contains(&(SymbolKind::Function, "top_level", "1:1-3:2")));
        assert!(symbols.contains(&(SymbolKind::Struct, "Widget", "5:1-7:2")));
        assert!(symbols.contains(&(SymbolKind::Enum, "Mode", "9:1-12:2")));
        assert!(symbols.contains(&(SymbolKind::Trait, "Runnable", "14:1-16:2")));
        assert!(symbols.contains(&(SymbolKind::Method, "run", "15:5-15:19")));
        assert!(symbols.contains(&(SymbolKind::Method, "new", "19:5-21:6")));
        assert!(symbols.contains(&(SymbolKind::Method, "run", "23:5-25:6")));
        assert!(
            symbols.contains(&(SymbolKind::Test, "exercises_widget", "30:1-33:2")),
            "{symbols:?}"
        );
    }

    #[test]
    fn rust_import_and_call_captures_work() {
        let summary = parse_path("fixtures/rust/imports_calls.rs", RUST_IMPORTS_CALLS)
            .expect("fixture parses");

        assert_capture(&summary, "import.source", "std::collections::HashMap");
        assert_capture(&summary, "reference.call", "HashMap::new()");
        assert_capture(&summary, "reference.call", "build()");
    }

    #[test]
    fn rust_extern_signatures_remain_functions() {
        let summary = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/extern_signature.rs")),
            RUST_EXTERN_SIGNATURE,
        )
        .expect("rust parses");

        assert_symbol(&summary, SymbolKind::Function, "puts");
        assert_symbol(&summary, SymbolKind::Method, "run");
        assert!(
            !summary
                .symbols
                .iter()
                .any(|symbol| symbol.kind == SymbolKind::Method && symbol.name == "puts")
        );
    }

    #[test]
    fn rust_doc_comments_filter_out_plain_comments() {
        let summary = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/doc_filter.rs")),
            RUST_DOC_FILTER,
        )
        .expect("rust parses");

        assert_capture(&summary, "doc.comment", "/// Real docs\n");
        assert!(
            !summary
                .captures
                .iter()
                .any(|capture| capture.capture_name == "doc.comment"
                    && capture.text == "// Ordinary comment")
        );
    }

    #[test]
    fn typescript_and_tsx_fixtures_extract_symbols_imports_and_calls() {
        let ts = parse_path("fixtures/typescript/imports.ts", TS_IMPORTS).expect("ts parses");
        assert_eq!(ts.source.language, SourceLanguage::TypeScript);
        assert_symbols_are_one_based(&ts);
        assert_capture(&ts, "import.source", "\"node:fs/promises\"");
        assert_capture(&ts, "reference.call", "readFile(path, \"utf8\")");

        let tsx =
            parse_path("fixtures/typescript/react_component.tsx", TSX_REACT).expect("tsx parses");
        assert_eq!(tsx.source.language, SourceLanguage::Tsx);
        assert_symbols_are_one_based(&tsx);
        assert_symbol(&tsx, SymbolKind::Interface, "ButtonProps");
        assert_symbol(&tsx, SymbolKind::Function, "Button");
        assert_symbol(&tsx, SymbolKind::Test, "renders button");
        assert_capture(&tsx, "import.source", "\"react\"");
        assert_capture(&tsx, "reference.call", "label.toUpperCase()");
        assert_capture(&tsx, "reference.call", "helper()");
    }

    #[test]
    fn javascript_jsx_fixture_extracts_symbols_imports_and_calls() {
        let summary = parse_path("fixtures/javascript/module.jsx", JSX_MODULE).expect("jsx parses");

        assert_eq!(summary.source.language, SourceLanguage::Jsx);
        assert_symbols_are_one_based(&summary);
        assert_symbol(&summary, SymbolKind::Class, "Panel");
        assert_symbol(&summary, SymbolKind::Function, "mount");
        assert_symbol(&summary, SymbolKind::Test, "mounts");
        assert_symbol(&summary, SymbolKind::Test, "escapes\\nname");
        assert_capture(&summary, "import.source", "\"react\"");
        assert_capture(
            &summary,
            "reference.call",
            "React.createElement(\"section\", null, \"Ready\")",
        );
        assert_capture(&summary, "reference.call", "mount()");
    }

    #[test]
    fn python_fixture_extracts_symbols_imports_calls_and_tests() {
        let summary =
            parse_path("fixtures/python/basic_symbols.py", PYTHON_BASIC).expect("python parses");

        assert_eq!(summary.source.language, SourceLanguage::Python);
        assert_symbols_are_one_based(&summary);
        assert_symbol(&summary, SymbolKind::Class, "Worker");
        assert_symbol(&summary, SymbolKind::Method, "run");
        assert_symbol(&summary, SymbolKind::Function, "nested");
        assert_symbol(&summary, SymbolKind::Function, "make_queue");
        assert_symbol(&summary, SymbolKind::Test, "test_make_queue");
        assert_capture(&summary, "import.source", "pathlib");
        assert_capture(&summary, "import.source", "collections");
        assert_capture(&summary, "reference.call", "deque()");
        assert_capture(&summary, "reference.call", "make_queue()");
    }

    #[test]
    fn malformed_code_returns_diagnostics() {
        let rust_error = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/malformed_error.rs")),
            RUST_MALFORMED_ERROR,
        )
        .expect("tree-sitter recovers malformed source");
        let rust_missing = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/malformed_missing.rs")),
            RUST_MALFORMED_MISSING,
        )
        .expect("tree-sitter recovers missing-token source");
        let python =
            parse_path("fixtures/python/malformed.py", PYTHON_MALFORMED).expect("python recovers");

        assert!(rust_error.has_errors);
        assert!(
            rust_error
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == AstDiagnosticKind::Error)
        );
        assert!(rust_missing.has_errors);
        assert!(
            rust_missing
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == AstDiagnosticKind::Missing)
        );
        assert!(python.has_errors);
        assert!(python.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic.kind,
            AstDiagnosticKind::Error | AstDiagnosticKind::Missing
        )));
    }

    #[test]
    fn lightweight_documents_produce_summaries_and_markdown_fences() {
        for (path, source, language) in [
            (
                "fixtures/documents/config.json",
                JSON_CONFIG,
                SourceLanguage::Json,
            ),
            (
                "fixtures/documents/config.yaml",
                YAML_CONFIG,
                SourceLanguage::Yaml,
            ),
            (
                "fixtures/documents/config.toml",
                TOML_CONFIG,
                SourceLanguage::Toml,
            ),
        ] {
            let summary = parse_path(path, source).expect("lightweight document parses");
            assert_eq!(summary.source.language, language);
            assert_eq!(summary.root_kind, "document");
            assert_eq!(
                summary.versions.grammar, "lightweight-text",
                "lightweight summaries do not use a tree-sitter grammar"
            );
            assert_eq!(summary.versions.provider, LIGHTWEIGHT_PROVIDER_NAME);
            assert_eq!(
                summary.versions.tree_sitter,
                LIGHTWEIGHT_TREE_SITTER_VERSION
            );
            assert_eq!(summary.symbols[0].kind, SymbolKind::Document);
            assert_eq!(summary.symbols[0].span.start_line, 1);
            assert_eq!(
                summary.symbols[0].parser_version,
                LIGHTWEIGHT_PARSER_VERSION
            );
            assert_eq!(
                summary.symbols[0].query_pack_version,
                format!("{}-lightweight-v1", language.id())
            );
        }

        let markdown =
            parse_path("fixtures/documents/notes.md", MARKDOWN_NOTES).expect("markdown parses");
        assert_eq!(markdown.source.language, SourceLanguage::Markdown);
        assert_capture(&markdown, "injection.language", "python");
        assert_capture(&markdown, "injection.content", "print(\"hello\")\n");
        let language = find_capture(&markdown, "injection.language", "python");
        assert_eq!(language.span.start_byte, 12);
        assert_eq!(language.span.end_byte, 18);
        assert_eq!(language.rendered_span, "3:4-3:10");
        let content = find_capture(&markdown, "injection.content", "print(\"hello\")\n");
        assert_eq!(content.span.start_byte, 19);
        assert_eq!(content.span.end_byte, 34);
        assert_eq!(content.rendered_span, "4:1-5:1");

        let four_tick = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/four_tick.md")),
            FOUR_BACKTICK_MARKDOWN,
        )
        .expect("markdown parses");
        let language = find_capture(&four_tick, "injection.language", "ts");
        assert_eq!(language.span.start_byte, 4);
        assert_eq!(language.span.end_byte, 6);
        assert_eq!(language.rendered_span, "1:5-1:7");
        let content = find_capture(&four_tick, "injection.content", "```not close\nvalue()\n");
        assert_eq!(content.span.start_byte, 7);
        assert_eq!(content.span.end_byte, 28);
        assert_eq!(content.rendered_span, "2:1-4:1");

        let info_string = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/info_string.md")),
            MARKDOWN_INFO_STRING,
        )
        .expect("markdown parses");
        let language = find_capture(&info_string, "injection.language", "python");
        assert_eq!(language.span.start_byte, 3);
        assert_eq!(language.span.end_byte, 9);
        assert_eq!(language.rendered_span, "1:4-1:10");
        assert_capture(&info_string, "injection.content", "print('ok')\n");

        let unlabeled = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/unlabeled.md")),
            MARKDOWN_UNLABELED,
        )
        .expect("markdown parses");
        assert_capture(&unlabeled, "injection.content", "raw\n");
        assert!(
            !unlabeled
                .captures
                .iter()
                .any(|capture| capture.capture_name == "injection.language")
        );

        let indented = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/indented.md")),
            MARKDOWN_INDENTED,
        )
        .expect("markdown parses");
        assert!(indented.captures.is_empty());

        let crlf = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/crlf.md")),
            MARKDOWN_CRLF,
        )
        .expect("markdown parses");
        let language = find_capture(&crlf, "injection.language", "python");
        assert_eq!(language.span.start_byte, 3);
        assert_eq!(language.span.end_byte, 9);
        assert_eq!(language.rendered_span, "1:4-1:10");
        let content = find_capture(&crlf, "injection.content", "print('ok')\n");
        assert_eq!(content.span.start_byte, 11);
        assert_eq!(content.span.end_byte, 24);
        assert_eq!(content.rendered_span, "2:1-3:1");

        let tilde = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/tilde.md")),
            MARKDOWN_TILDE,
        )
        .expect("markdown parses");
        assert_capture(&tilde, "injection.language", "python");
        assert_capture(&tilde, "injection.content", "print('tilde')\n");

        let unclosed = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/unclosed.md")),
            MARKDOWN_UNCLOSED,
        )
        .expect("markdown parses");
        let content = find_capture(&unclosed, "injection.content", "print('open')\n");
        assert_eq!(content.span.start_byte, 10);
        assert_eq!(content.span.end_byte, MARKDOWN_UNCLOSED.len());
        assert_eq!(content.rendered_span, "2:1-3:1");

        let tab_indented = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/tab_indented.md")),
            MARKDOWN_TAB_INDENTED,
        )
        .expect("markdown parses");
        assert!(tab_indented.captures.is_empty());

        let closing_indent = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/closing_indent.md")),
            MARKDOWN_CLOSING_INDENT_TOO_SMALL,
        )
        .expect("markdown parses");
        let content = find_capture(&closing_indent, "injection.content", "value\n```\n");
        assert_eq!(content.span.start_byte, 8);
        assert_eq!(
            content.span.end_byte,
            MARKDOWN_CLOSING_INDENT_TOO_SMALL.len()
        );
        assert_eq!(content.rendered_span, "2:1-4:1");

        let invalid_info = parse_source(
            SourceLanguage::Markdown,
            Some(PathBuf::from("fixtures/documents/backtick_info.md")),
            MARKDOWN_BACKTICK_INFO,
        )
        .expect("markdown parses");
        assert!(invalid_info.captures.is_empty());
    }

    fn symbol_tuples(summary: &ParsedDocumentSummary) -> Vec<(SymbolKind, &str, &str)> {
        summary
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.kind.clone(),
                    symbol.name.as_str(),
                    symbol.rendered_span.as_str(),
                )
            })
            .collect()
    }

    fn query_asset_names(assets: &[QueryAsset]) -> Vec<&'static str> {
        assets.iter().map(|asset| asset.name).collect()
    }

    fn assert_capture(summary: &ParsedDocumentSummary, capture_name: &str, text: &str) {
        assert!(
            summary
                .captures
                .iter()
                .any(|capture| capture.capture_name == capture_name && capture.text == text),
            "missing @{capture_name} capture with text {text:?}; captures: {:?}",
            summary.captures
        );
    }

    fn find_capture<'a>(
        summary: &'a ParsedDocumentSummary,
        capture_name: &str,
        text: &str,
    ) -> &'a CaptureRecord {
        summary
            .captures
            .iter()
            .find(|capture| capture.capture_name == capture_name && capture.text == text)
            .unwrap_or_else(|| {
                panic!(
                    "missing @{capture_name} capture with text {text:?}; captures: {:?}",
                    summary.captures
                )
            })
    }

    fn assert_symbol(summary: &ParsedDocumentSummary, kind: SymbolKind, name: &str) {
        assert!(
            summary
                .symbols
                .iter()
                .any(|symbol| symbol.kind == kind && symbol.name == name),
            "missing {kind:?} symbol {name:?}; symbols: {:?}",
            summary.symbols
        );
    }

    fn assert_symbols_are_one_based(summary: &ParsedDocumentSummary) {
        assert!(!summary.symbols.is_empty());
        for symbol in &summary.symbols {
            assert!(symbol.span.start_line >= 1, "{symbol:?}");
            assert!(symbol.span.start_column >= 1, "{symbol:?}");
            assert_eq!(symbol.rendered_span, symbol.span.render());
        }
    }

    #[test]
    fn composite_falls_back_when_parser_reports_diagnostics() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(repo.path().join("src/lib.rs"), "pub fn broken( {\n").expect("malformed rust");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/lib.rs")], &[], 20)
            .expect("code context");

        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-summary")
        );
        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace"
                && artifact
                    .summary
                    .contains("parsed with Tree-sitter diagnostics")
        }));
    }

    #[test]
    fn composite_falls_back_when_file_exceeds_ast_size_limit() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            format!(
                "pub const LARGE: &str = \"{}\";\n",
                "x".repeat(DEFAULT_MAX_FILE_BYTES as usize + 1)
            ),
        )
        .expect("large rust");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/lib.rs")], &[], 20)
            .expect("code context");

        assert!(
            !artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-summary")
        );
        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace" && artifact.summary.contains("max AST file size")
        }));
    }

    #[test]
    fn composite_parses_explicit_files_under_generated_directories() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src/generated")).expect("generated dir");
        fs::write(
            repo.path().join("src/generated/mod.rs"),
            "pub fn answer() {}\n",
        )
        .expect("generated module");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/generated/mod.rs")], &[], 20)
            .expect("code context");

        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.title == "src/generated/mod.rs")
        );
        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-symbols"
                    && artifact.summary.contains("function `answer`"))
        );
    }

    #[test]
    fn composite_skips_requested_generated_directories_with_trace() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("generated")).expect("generated dir");
        fs::write(repo.path().join("generated/mod.rs"), "pub fn hidden() {}\n")
            .expect("generated module");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("generated")], &[], 20)
            .expect("code context");

        assert!(
            !artifacts
                .iter()
                .any(|artifact| artifact.title.contains("generated/mod.rs"))
        );
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace"
                && artifact
                    .summary
                    .contains("generated skipped directory `generated`")
        }));
    }

    #[test]
    fn composite_falls_back_when_requested_file_is_missing() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/missing.rs")], &[], 20)
            .expect("code context");

        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace" && artifact.summary.contains("file not found")
        }));
    }

    #[test]
    fn composite_falls_back_when_requested_file_is_not_utf8() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(repo.path().join("src/lib.rs"), [0xff, 0xfe]).expect("binary file");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/lib.rs")], &[], 20)
            .expect("code context");

        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace" && artifact.summary.contains("not UTF-8")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn composite_rejects_symlink_to_outside_repo() {
        let repo = TempDir::new().expect("temp repo");
        let outside = TempDir::new().expect("outside repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        let outside_file = outside.path().join("lib.rs");
        fs::write(&outside_file, "pub fn outside() {}\n").expect("outside file");
        std::os::unix::fs::symlink(&outside_file, repo.path().join("src/outside.rs"))
            .expect("symlink");

        let error = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/outside.rs")], &[], 20)
            .expect_err("outside symlink should be rejected");

        assert!(matches!(error, MemoryError::PathOutsideRepo { .. }));
    }

    #[test]
    fn composite_preserves_zero_symbol_limit() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("rust file");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("src/lib.rs")], &[], 0)
            .expect("code context");

        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-summary")
        );
        assert!(
            !artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-symbols")
        );
    }

    #[test]
    fn composite_mixed_paths_with_zero_limit_still_falls_back() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("rust file");
        fs::write(repo.path().join("notes.txt"), "Example\n").expect("notes");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(
                &[PathBuf::from("src/lib.rs"), PathBuf::from("notes.txt")],
                &[],
                0,
            )
            .expect("code context");

        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-summary")
        );
        assert!(
            !artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-symbols")
        );
        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
    }

    #[test]
    fn ast_provider_trace_says_fallback_is_not_available() {
        let repo = TempDir::new().expect("temp repo");
        fs::write(repo.path().join("notes.txt"), "Example\n").expect("notes");

        let artifacts = AstCodeIntelProvider::new(repo.path())
            .code_context(&[PathBuf::from("notes.txt")], &[], 20)
            .expect("code context");

        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == PROVIDER_NAME
                && artifact.kind == "trace"
                && artifact.summary.contains("AST provider only")
                && artifact
                    .summary
                    .contains("CodebaseAnalyzer fallback not available")
        }));
    }

    #[test]
    fn composite_empty_paths_return_repository_summary_fallback() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(repo.path().join("src/lib.rs"), "pub fn answer() {}\n").expect("rust file");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(&[], &[], 20)
            .expect("code context");

        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace" && artifact.summary.contains("no requested paths")
        }));
    }

    #[test]
    fn composite_mixed_paths_partition_fallback_budget() {
        let repo = TempDir::new().expect("temp repo");
        fs::create_dir_all(repo.path().join("src")).expect("src dir");
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 { 42 }\n",
        )
        .expect("rust file");
        fs::write(repo.path().join("notes.txt"), "Example\n").expect("notes");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(
                &[PathBuf::from("src/lib.rs"), PathBuf::from("notes.txt")],
                &[],
                2,
            )
            .expect("code context");

        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-summary")
        );
        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.kind == "ast-symbols")
        );
        assert!(artifacts.iter().any(|artifact| {
            artifact.provider == "codebase-analyzer" && artifact.title == "Repository summary"
        }));
        assert!(artifacts.iter().any(|artifact| {
            artifact.kind == "trace"
                && artifact
                    .summary
                    .contains("notes.txt has unsupported language")
        }));
    }
}
