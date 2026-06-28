use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
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
// Keep these in sync with Cargo.toml pins and queries/rust/metadata.toml.
pub const TREE_SITTER_VERSION: &str = "0.26.9";
pub const RUST_GRAMMAR_VERSION: &str = "0.24.2";
pub const RUST_QUERY_PACK_VERSION: &str = "rust-definitions-v1";

const RUST_DEFINITIONS_QUERY: &str = include_str!("../queries/rust/definitions.scm");
const DEFAULT_MAX_FILE_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLanguage {
    Rust,
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
    pub diagnostics: Vec<AstDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// One-based Tree-sitter points; columns are byte offsets and end positions are exclusive.
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
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Method,
    Test,
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
#[serde(rename_all = "snake_case")]
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
pub enum CodeIntelError {
    #[error("unsupported source language for {path}")]
    UnsupportedLanguage { path: String },
    #[error("failed to load Rust parser: {0}")]
    Language(#[from] tree_sitter::LanguageError),
    #[error("failed to parse Rust source")]
    Parse,
    #[error("invalid Rust definitions query: {0}")]
    Query(#[from] tree_sitter::QueryError),
    #[error("captured symbol name is not utf-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
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
            let resolved = self.resolve_requested_path(&repo_root, path)?;
            let relative_path = resolved
                .strip_prefix(&repo_root)
                .unwrap_or(&resolved)
                .to_path_buf();
            let relative_display = relative_path.to_string_lossy().to_string();

            let Some(source) =
                self.read_limited_source(&resolved, &relative_display, &mut report)?
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
    ) -> Result<Option<String>, MemoryError> {
        let file = fs::File::open(resolved).map_err(|source| MemoryError::ReadFile {
            path: resolved.to_path_buf(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| MemoryError::ReadFile {
            path: resolved.to_path_buf(),
            source,
        })?;

        if !metadata.is_file() {
            report
                .fallback_reasons
                .push(format!("{relative_display} is not a file"));
            report.fallback_paths.push(PathBuf::from(relative_display));
            return Ok(None);
        }

        let mut bytes = Vec::new();
        file.take(self.max_file_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| MemoryError::ReadFile {
                path: resolved.to_path_buf(),
                source,
            })?;
        if bytes.len() as u64 > self.max_file_bytes {
            report.fallback_reasons.push(format!(
                "{relative_display} exceeds max AST file size of {} bytes",
                self.max_file_bytes
            ));
            report.fallback_paths.push(PathBuf::from(relative_display));
            return Ok(None);
        }

        String::from_utf8(bytes).map(Some).map_err(|error| {
            MemoryError::InvalidInput(format!("{relative_display} is not UTF-8: {error}"))
        })
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
    ) -> Result<PathBuf, MemoryError> {
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            repo_root.join(path)
        };
        let resolved = candidate
            .canonicalize()
            .map_err(|source| MemoryError::ResolvePath {
                path: candidate.clone(),
                source,
            })?;
        if !resolved.starts_with(repo_root) {
            return Err(MemoryError::PathOutsideRepo {
                path: resolved,
                repo_root: repo_root.to_path_buf(),
            });
        }
        Ok(resolved)
    }
}

impl CodeIntelIndex for AstCodeIntelProvider {
    fn code_context(
        &self,
        paths: &[PathBuf],
        scope_refs: &[KnowledgeScope],
        limit: usize,
    ) -> Result<Vec<CodeIntelArtifact>, MemoryError> {
        let report = self.code_context_report(paths, scope_refs, limit)?;
        let trace = report.trace_artifact(scope_refs, false);
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

        if use_fallback && has_fallback_target && (fallback_limit > 0 || artifacts.is_empty()) {
            artifacts.extend(self.fallback.code_context(
                fallback_paths,
                scope_refs,
                fallback_limit,
            )?);
            fallback_used = true;
        }
        let trace = report.trace_artifact(scope_refs, fallback_used);
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
        fallback_used: bool,
    ) -> CodeIntelArtifact {
        let fallback = match (self.fallback_reasons.is_empty(), fallback_used) {
            (true, _) => "fallback: CodebaseAnalyzer not used".to_string(),
            (false, true) => format!(
                "fallback: CodebaseAnalyzer used ({})",
                self.fallback_reasons.join("; ")
            ),
            (false, false) => format!(
                "fallback: CodebaseAnalyzer not used; fallback budget exhausted ({})",
                self.fallback_reasons.join("; ")
            ),
        };
        CodeIntelArtifact {
            provider: "composite-code-intel".to_string(),
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
}

fn ast_artifacts_for_summary(
    summary: ParsedDocumentSummary,
    relative_path: &Path,
    relative_display: &str,
    scope_refs: &[KnowledgeScope],
    commit_sha: Option<String>,
    symbol_limit: usize,
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
            "- Language: {}\n- Content hash: sha256:{}\n- Parser: tree-sitter-rust@{}\n- Query pack: {}\n- Diagnostics: {diagnostic_summary}",
            source_language_label(summary.source.language),
            summary.source.sha256,
            RUST_GRAMMAR_VERSION,
            summary.versions.query_pack,
        ),
    }];

    if !summary.symbols.is_empty() && symbol_limit > 0 {
        let max_symbols = symbol_limit;
        let used_symbols = summary.symbols.len().min(max_symbols);
        let symbols = summary
            .symbols
            .iter()
            .take(max_symbols)
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
            source_refs: summary
                .symbols
                .iter()
                .take(max_symbols)
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
    }
}

fn symbol_kind_label(kind: &SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Function => "function",
        SymbolKind::Struct => "struct",
        SymbolKind::Enum => "enum",
        SymbolKind::Trait => "trait",
        SymbolKind::Method => "method",
        SymbolKind::Test => "test",
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
    match path
        .as_ref()
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("rs") => Some(SourceLanguage::Rust),
        _ => None,
    }
}

pub fn parse_path(
    path: impl AsRef<Path>,
    source: &str,
) -> Result<ParsedDocumentSummary, CodeIntelError> {
    let path = path.as_ref();
    match detect_language(path) {
        Some(SourceLanguage::Rust) => parse_rust_source(Some(path.to_path_buf()), source),
        None => Err(CodeIntelError::UnsupportedLanguage {
            path: path.display().to_string(),
        }),
    }
}

pub fn parse_rust_source(
    path: Option<PathBuf>,
    source: &str,
) -> Result<ParsedDocumentSummary, CodeIntelError> {
    let language: Language = tree_sitter_rust::LANGUAGE.into();
    let mut parser = Parser::new();
    parser.set_language(&language)?;
    let tree = parser.parse(source, None).ok_or(CodeIntelError::Parse)?;
    let root = tree.root_node();
    let query = Query::new(&language, RUST_DEFINITIONS_QUERY)?;
    let versions = parser_versions();

    Ok(ParsedDocumentSummary {
        source: SourceIdentity {
            language: SourceLanguage::Rust,
            path,
            bytes: source.len(),
            sha256: source_sha256(source),
        },
        versions,
        root_kind: root.kind().to_string(),
        has_errors: root.has_error(),
        symbols: extract_symbols(&query, root, source.as_bytes())?,
        diagnostics: collect_diagnostics(root),
    })
}

fn parser_versions() -> ParserVersions {
    ParserVersions {
        provider: PROVIDER_NAME.to_string(),
        tree_sitter: TREE_SITTER_VERSION.to_string(),
        grammar: format!("rust-{RUST_GRAMMAR_VERSION}"),
        query_pack: RUST_QUERY_PACK_VERSION.to_string(),
    }
}

fn source_sha256(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn extract_symbols(
    query: &Query,
    root: Node<'_>,
    source: &[u8],
) -> Result<Vec<SymbolRecord>, CodeIntelError> {
    let mut cursor = QueryCursor::new();
    let definition_capture = query.capture_index_for_name("definition");
    let name_capture = query.capture_index_for_name("name");
    let mut matches = cursor.matches(query, root, source);
    let mut symbols = Vec::new();

    while let Some(query_match) = matches.next() {
        let definition = query_match
            .captures
            .iter()
            .find(|capture| Some(capture.index) == definition_capture)
            .map(|capture| capture.node);
        let name = query_match
            .captures
            .iter()
            .find(|capture| Some(capture.index) == name_capture)
            .map(|capture| capture.node);

        let (Some(definition), Some(name)) = (definition, name) else {
            continue;
        };
        let Some(kind) = symbol_kind(definition, source) else {
            continue;
        };
        let name = name.utf8_text(source)?.to_string();
        let span = one_based_span(definition);
        symbols.push(SymbolRecord {
            kind,
            name,
            rendered_span: span.render(),
            span,
            parser_version: TREE_SITTER_VERSION.to_string(),
            query_pack_version: RUST_QUERY_PACK_VERSION.to_string(),
        });
    }

    Ok(symbols)
}

fn symbol_kind(node: Node<'_>, source: &[u8]) -> Option<SymbolKind> {
    match node.kind() {
        "struct_item" => Some(SymbolKind::Struct),
        "enum_item" => Some(SymbolKind::Enum),
        "trait_item" => Some(SymbolKind::Trait),
        "function_item" if has_test_attribute(node, source) => Some(SymbolKind::Test),
        "function_item" if has_ancestor(node, "impl_item") => Some(SymbolKind::Method),
        "function_signature_item" if has_ancestor(node, "trait_item") => Some(SymbolKind::Method),
        "function_item" => Some(SymbolKind::Function),
        _ => None,
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
    // Rust MVP: classify only the built-in #[test] attribute as a Test symbol.
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
    matches!(node.kind(), "line_comment" | "block_comment")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_memory::CodeIntelIndex;
    use tempfile::TempDir;

    const COMPLETE: &str = include_str!("../fixtures/rust/complete.rs");
    const MALFORMED_ERROR: &str = include_str!("../fixtures/rust/malformed_error.rs");
    const MALFORMED_MISSING: &str = include_str!("../fixtures/rust/malformed_missing.rs");

    #[test]
    fn detects_rust_by_extension() {
        assert_eq!(detect_language("src/lib.rs"), Some(SourceLanguage::Rust));
        assert_eq!(detect_language("README.md"), None);
    }

    #[test]
    fn rust_fixture_extracts_expected_symbols() {
        let summary = parse_rust_source(Some(PathBuf::from("fixtures/rust/complete.rs")), COMPLETE)
            .expect("fixture parses");

        assert!(!summary.has_errors);
        assert_eq!(summary.root_kind, "source_file");
        assert_eq!(summary.versions.provider, PROVIDER_NAME);
        assert_eq!(summary.versions.tree_sitter, TREE_SITTER_VERSION);
        assert_eq!(summary.versions.grammar, "rust-0.24.2");
        assert_eq!(summary.versions.query_pack, RUST_QUERY_PACK_VERSION);

        let symbols = summary
            .symbols
            .iter()
            .map(|symbol| {
                (
                    symbol.kind.clone(),
                    symbol.name.as_str(),
                    symbol.rendered_span.as_str(),
                )
            })
            .collect::<Vec<_>>();

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
    fn malformed_code_returns_error_diagnostics() {
        let summary = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/malformed_error.rs")),
            MALFORMED_ERROR,
        )
        .expect("tree-sitter recovers malformed source");

        assert!(summary.has_errors);
        assert!(
            summary
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == AstDiagnosticKind::Error)
        );
    }

    #[test]
    fn malformed_code_returns_missing_diagnostics() {
        let summary = parse_rust_source(
            Some(PathBuf::from("fixtures/rust/malformed_missing.rs")),
            MALFORMED_MISSING,
        )
        .expect("tree-sitter recovers missing-token source");

        assert!(summary.has_errors);
        assert!(
            summary
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == AstDiagnosticKind::Missing)
        );
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
            format!("pub const LARGE: &str = \"{}\";\n", "x".repeat(1_000_001)),
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
        fs::write(repo.path().join("README.md"), "# Example\n").expect("readme");

        let artifacts = CompositeCodeIntelProvider::new(repo.path())
            .code_context(
                &[PathBuf::from("src/lib.rs"), PathBuf::from("README.md")],
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
                    .contains("README.md has unsupported language")
        }));
    }
}
