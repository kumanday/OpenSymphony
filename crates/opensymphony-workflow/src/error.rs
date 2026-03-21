use std::{io, path::PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkflowLoadError {
    #[error("workflow file not found: {path}")]
    MissingWorkflowFile { path: PathBuf },

    #[error("failed to read workflow file {path}: {source}")]
    ReadWorkflowFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("workflow front matter is missing a closing `---` delimiter")]
    MissingFrontMatterTerminator,

    #[error("failed to parse workflow front matter: {source}")]
    WorkflowParseError {
        #[source]
        source: serde_yaml::Error,
    },

    #[error("workflow front matter must decode to a YAML map")]
    WorkflowFrontMatterNotAMap,
}

#[derive(Debug, Error)]
pub enum WorkflowConfigError {
    #[error("missing required config field `{field}`")]
    MissingRequiredField { field: &'static str },

    #[error("missing required environment variable `{variable}` for `{field}`")]
    MissingEnvironmentVariable {
        field: &'static str,
        variable: String,
    },

    #[error("unsupported tracker kind `{kind}`")]
    UnsupportedTrackerKind { kind: String },

    #[error("invalid integer for `{field}`: `{value}`")]
    InvalidInteger { field: &'static str, value: String },

    #[error("invalid config for `{field}`: {message}")]
    InvalidField {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum PromptTemplateError {
    #[error("failed to serialize template context: {source}")]
    Context {
        #[source]
        source: liquid::Error,
    },

    #[error("failed to parse prompt template: {source}")]
    Parse {
        #[source]
        source: liquid::Error,
    },

    #[error("failed to render prompt template: {source}")]
    Render {
        #[source]
        source: liquid::Error,
    },
}
