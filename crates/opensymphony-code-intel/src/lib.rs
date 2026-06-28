use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor, StreamingIterator};

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

const RUST_METADATA: &str = include_str!("../queries/rust/metadata.toml");
const TYPESCRIPT_METADATA: &str = include_str!("../queries/typescript/metadata.toml");
const TSX_METADATA: &str = include_str!("../queries/tsx/metadata.toml");
const JAVASCRIPT_METADATA: &str = include_str!("../queries/javascript/metadata.toml");
const JSX_METADATA: &str = include_str!("../queries/jsx/metadata.toml");
const PYTHON_METADATA: &str = include_str!("../queries/python/metadata.toml");

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

const RUST_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/rust/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/rust/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/rust/calls.scm")),
    QueryAsset::new("docs", include_str!("../queries/rust/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/rust/locals.scm")),
    QueryAsset::new(
        "diagnostics",
        include_str!("../queries/rust/diagnostics.scm"),
    ),
];

const TYPESCRIPT_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/typescript/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/typescript/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/typescript/calls.scm")),
    QueryAsset::new("tests", include_str!("../queries/typescript/tests.scm")),
    QueryAsset::new("docs", include_str!("../queries/typescript/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/typescript/locals.scm")),
    QueryAsset::new(
        "injections",
        include_str!("../queries/typescript/injections.scm"),
    ),
    QueryAsset::new(
        "diagnostics",
        include_str!("../queries/typescript/diagnostics.scm"),
    ),
];

const TSX_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/tsx/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/tsx/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/tsx/calls.scm")),
    QueryAsset::new("tests", include_str!("../queries/tsx/tests.scm")),
    QueryAsset::new("docs", include_str!("../queries/tsx/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/tsx/locals.scm")),
    QueryAsset::new("injections", include_str!("../queries/tsx/injections.scm")),
    QueryAsset::new(
        "diagnostics",
        include_str!("../queries/tsx/diagnostics.scm"),
    ),
];

const JAVASCRIPT_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/javascript/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/javascript/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/javascript/calls.scm")),
    QueryAsset::new("tests", include_str!("../queries/javascript/tests.scm")),
    QueryAsset::new("docs", include_str!("../queries/javascript/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/javascript/locals.scm")),
    QueryAsset::new(
        "diagnostics",
        include_str!("../queries/javascript/diagnostics.scm"),
    ),
];

const JSX_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/jsx/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/jsx/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/jsx/calls.scm")),
    QueryAsset::new("tests", include_str!("../queries/jsx/tests.scm")),
    QueryAsset::new("docs", include_str!("../queries/jsx/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/jsx/locals.scm")),
    QueryAsset::new("injections", include_str!("../queries/jsx/injections.scm")),
    QueryAsset::new(
        "diagnostics",
        include_str!("../queries/jsx/diagnostics.scm"),
    ),
];

const PYTHON_QUERIES: &[QueryAsset] = &[
    QueryAsset::new(
        "definitions",
        include_str!("../queries/python/definitions.scm"),
    ),
    QueryAsset::new("imports", include_str!("../queries/python/imports.scm")),
    QueryAsset::new("calls", include_str!("../queries/python/calls.scm")),
    QueryAsset::new("docs", include_str!("../queries/python/docs.scm")),
    QueryAsset::new("locals", include_str!("../queries/python/locals.scm")),
    QueryAsset::new(
        "diagnostics",
        include_str!("../queries/python/diagnostics.scm"),
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SourceLanguage {
    Rust,
    #[serde(rename = "typescript")]
    TypeScript,
    Tsx,
    #[serde(rename = "javascript")]
    JavaScript,
    Jsx,
    Python,
    Json,
    Yaml,
    Toml,
    Markdown,
}

impl SourceLanguage {
    fn id(self) -> &'static str {
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
    queries: &'static [QueryAsset],
    parser: fn() -> Language,
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
        Some("ts") => Some(SourceLanguage::TypeScript),
        Some("tsx") => Some(SourceLanguage::Tsx),
        Some("js" | "mjs" | "cjs") => Some(SourceLanguage::JavaScript),
        Some("jsx") => Some(SourceLanguage::Jsx),
        Some("py") => Some(SourceLanguage::Python),
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

fn language_config(language: SourceLanguage) -> LanguageConfig {
    match language {
        SourceLanguage::Rust => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-rust",
            grammar_version: RUST_GRAMMAR_VERSION,
            query_pack_version: RUST_QUERY_PACK_VERSION,
            metadata: RUST_METADATA,
            queries: RUST_QUERIES,
            parser: || tree_sitter_rust::LANGUAGE.into(),
        },
        SourceLanguage::TypeScript => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-typescript",
            grammar_version: TYPESCRIPT_GRAMMAR_VERSION,
            query_pack_version: TYPESCRIPT_QUERY_PACK_VERSION,
            metadata: TYPESCRIPT_METADATA,
            queries: TYPESCRIPT_QUERIES,
            parser: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        },
        SourceLanguage::Tsx => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-typescript",
            grammar_version: TYPESCRIPT_GRAMMAR_VERSION,
            query_pack_version: TSX_QUERY_PACK_VERSION,
            metadata: TSX_METADATA,
            queries: TSX_QUERIES,
            parser: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        },
        SourceLanguage::JavaScript => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-javascript",
            grammar_version: JAVASCRIPT_GRAMMAR_VERSION,
            query_pack_version: JAVASCRIPT_QUERY_PACK_VERSION,
            metadata: JAVASCRIPT_METADATA,
            queries: JAVASCRIPT_QUERIES,
            parser: || tree_sitter_javascript::LANGUAGE.into(),
        },
        SourceLanguage::Jsx => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-javascript",
            grammar_version: JAVASCRIPT_GRAMMAR_VERSION,
            query_pack_version: JSX_QUERY_PACK_VERSION,
            metadata: JSX_METADATA,
            queries: JSX_QUERIES,
            parser: || tree_sitter_javascript::LANGUAGE.into(),
        },
        SourceLanguage::Python => LanguageConfig {
            language,
            grammar_crate: "tree-sitter-python",
            grammar_version: PYTHON_GRAMMAR_VERSION,
            query_pack_version: PYTHON_QUERY_PACK_VERSION,
            metadata: PYTHON_METADATA,
            queries: PYTHON_QUERIES,
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
        .queries
        .iter()
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
    validate_capture_names(asset, &query, metadata)?;
    Ok(CompiledQuery {
        asset,
        query,
        metadata,
    })
}

fn validate_capture_names(
    asset: QueryAsset,
    query: &Query,
    metadata: &QueryPackMetadata,
) -> Result<(), CodeIntelError> {
    for capture_name in query.capture_names().iter().copied() {
        if !STANDARD_CAPTURE_NAMES.contains(&capture_name) {
            return Err(CodeIntelError::NonstandardCapture {
                query_name: asset.name.to_string(),
                capture_name: capture_name.to_string(),
            });
        }
        if !metadata.captures.contains_key(capture_name) {
            return Err(CodeIntelError::UndocumentedCapture {
                query_name: asset.name.to_string(),
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
                let span = one_based_span(capture.node);
                let text = capture.node.utf8_text(source)?.to_string();
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

fn is_method(language: SourceLanguage, node: Node<'_>) -> bool {
    match language {
        SourceLanguage::Rust => has_ancestor(node, "impl_item"),
        SourceLanguage::Python => is_python_class_body_function(node),
        _ => matches!(node.kind(), "method_definition"),
    }
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
        let trimmed = line.trim_start();
        if let Some((fence_tick_count, rest)) = markdown_backtick_fence(trimmed) {
            if open_fence
                .as_ref()
                .is_some_and(|fence| is_markdown_closing_fence(trimmed, fence.tick_count))
            {
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
                    query_name: "markdown_fences".to_string(),
                    capture_name: "injection.content".to_string(),
                    text: source[fence.content_start_byte..byte_offset].to_string(),
                    rendered_span: span.render(),
                    span,
                });
                captures.push(CaptureRecord {
                    query_name: "markdown_fences".to_string(),
                    capture_name: "injection.language".to_string(),
                    text: fence.language,
                    rendered_span: fence.language_span.render(),
                    span: fence.language_span,
                });
            } else if open_fence.is_none() {
                let language = rest.trim().to_string();
                let indent = line.len() - trimmed.len();
                let language_start_in_rest = rest.find(language.as_str()).unwrap_or(0);
                let language_start_byte =
                    byte_offset + indent + fence_tick_count + language_start_in_rest;
                let language_span = SourceSpan {
                    start_byte: language_start_byte,
                    end_byte: language_start_byte + language.len(),
                    start_line: line_number,
                    start_column: indent + fence_tick_count + 1 + language_start_in_rest,
                    end_line: line_number,
                    end_column: indent
                        + fence_tick_count
                        + 1
                        + language_start_in_rest
                        + language.len(),
                };
                open_fence = Some(MarkdownFence {
                    tick_count: fence_tick_count,
                    language,
                    language_span,
                    content_start_byte: byte_offset + line.len(),
                    content_start_line: line_number + 1,
                });
            }
        }
        byte_offset += line.len();
    }

    captures
}

fn markdown_backtick_fence(trimmed_line: &str) -> Option<(usize, &str)> {
    let tick_count = trimmed_line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b'`')
        .count();
    (tick_count >= 3).then(|| (tick_count, &trimmed_line[tick_count..]))
}

fn is_markdown_closing_fence(trimmed_line: &str, opening_tick_count: usize) -> bool {
    markdown_backtick_fence(trimmed_line)
        .map(|(tick_count, rest)| tick_count >= opening_tick_count && rest.trim().is_empty())
        .unwrap_or(false)
}

struct MarkdownFence {
    tick_count: usize,
    language: String,
    language_span: SourceSpan,
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

    #[test]
    fn detects_supported_languages_by_extension_and_name() {
        assert_eq!(detect_language("src/lib.rs"), Some(SourceLanguage::Rust));
        assert_eq!(
            detect_language("src/app.ts"),
            Some(SourceLanguage::TypeScript)
        );
        assert_eq!(detect_language("src/app.tsx"), Some(SourceLanguage::Tsx));
        assert_eq!(
            detect_language("src/app.js"),
            Some(SourceLanguage::JavaScript)
        );
        assert_eq!(detect_language("src/app.jsx"), Some(SourceLanguage::Jsx));
        assert_eq!(detect_language("script.py"), Some(SourceLanguage::Python));
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
        assert_symbol(&tsx, SymbolKind::Test, "test");
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
}
