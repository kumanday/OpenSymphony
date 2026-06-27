use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

pub const PROVIDER_NAME: &str = "tree-sitter";
pub const TREE_SITTER_VERSION: &str = "0.26.9";
pub const RUST_GRAMMAR_VERSION: &str = "0.24.2";
pub const RUST_QUERY_PACK_VERSION: &str = "rust-definitions-v1";

const RUST_DEFINITIONS_QUERY: &str = include_str!("../queries/rust/definitions.scm");

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
    let mut symbols = BTreeMap::new();

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
        let key = (definition.start_byte(), definition.end_byte(), name.clone());
        symbols.entry(key).or_insert_with(|| SymbolRecord {
            kind,
            name,
            rendered_span: span.render(),
            span,
            parser_version: TREE_SITTER_VERSION.to_string(),
            query_pack_version: RUST_QUERY_PACK_VERSION.to_string(),
        });
    }

    Ok(symbols.into_values().collect())
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

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.has_error() || child.is_error() || child.is_missing() {
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
}
