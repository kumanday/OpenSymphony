//! Minimal YAML frontmatter reader for task files.
//!
//! The task files use a `---` delimited YAML block at the head of the
//! markdown document. We only need a small subset of fields for the
//! manifest validator and graph builder; this module implements a focused
//! wedge that splits on the first `---` line and delegates to
//! `serde_yaml` rather than dragging in a full markdown parser.
//!
//! The supported fields match the per-task contract documented in the
//! `convert-tasks-to-linear` skill README: `id`, `title`, `milestone`,
//! `priority`, `estimate`, `blockedBy`, `blocks`, `parent`, plus optional
//! `areas` and `notes`. Unknown fields are tolerated because task files
//! may carry future extension fields without breaking the validator.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::opensymphony_planning::generator::domain::TaskId;

/// YAML frontmatter block parsed from a task file.
///
/// The on-disk schema uses the same lowercase field names plus `id`,
/// `title`, `milestone`, `priority`, `estimate`. The newer `blockedBy`
/// and `blocks` fields are camelCase to match the rest of the project;
/// the manifest validator accepts both spellings by listing the legacy
/// snake_case alias under `#[serde(alias)]`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TaskFrontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<i64>,
    #[serde(rename = "blockedBy", alias = "blocked_by", default)]
    pub blocked_by: Vec<String>,
    #[serde(rename = "blocks", default)]
    pub blocks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub areas: Vec<String>,
    /// Any additional fields the task file carries. Preserved so the
    /// validator can re-emit the original block after normalisation.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

/// Parsed representation of a task file with frontmatter and body.
#[derive(Debug, Clone)]
pub struct ParsedTaskFile {
    pub frontmatter: TaskFrontmatter,
    /// Raw body after the frontmatter block (excluding the trailing `---`).
    pub body: String,
}

/// Errors raised by the YAML frontmatter loader.
#[derive(Debug, thiserror::Error)]
pub enum TaskFrontmatterError {
    #[error("failed to read task file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("task file {path} is missing YAML frontmatter")]
    MissingFrontmatter { path: String },
    #[error("failed to parse YAML frontmatter in {path}: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// Reads a task file from disk and parses its YAML frontmatter.
///
/// Returns `(frontmatter, body)` for the frontmatter block followed by the
/// raw markdown body. Body content is preserved verbatim because future
/// consumers (sub-issue readiness checks) may need unparsed markdown.
pub fn parse_task_file(path: &Path) -> Result<ParsedTaskFile, TaskFrontmatterError> {
    let raw = fs::read_to_string(path).map_err(|source| TaskFrontmatterError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_task_text(&raw, path.display().to_string().as_str())
}

/// Parses task-file text directly so tests can feed in fixtures without
/// creating real files. The path argument is propagated to error messages
/// to keep error reporting consistent between file and string entry points.
pub fn parse_task_text(raw: &str, path: &str) -> Result<ParsedTaskFile, TaskFrontmatterError> {
    let trimmed = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let first = lines.first().copied().unwrap_or("");
    if first.trim_end() != "---" {
        return Err(TaskFrontmatterError::MissingFrontmatter {
            path: path.to_string(),
        });
    }
    let mut yaml_lines: Vec<&str> = Vec::new();
    let mut closing_line_idx: Option<usize> = None;
    for (idx, line) in lines.iter().enumerate().skip(1) {
        let normalized = line.trim_end();
        if normalized == "---" || normalized == "..." {
            closing_line_idx = Some(idx);
            break;
        }
        yaml_lines.push(*line);
    }
    let closing = closing_line_idx.ok_or_else(|| TaskFrontmatterError::MissingFrontmatter {
        path: path.to_string(),
    })?;
    let yaml_text = yaml_lines.join("\n");
    let frontmatter: TaskFrontmatter =
        serde_yaml::from_str(&yaml_text).map_err(|source| TaskFrontmatterError::Yaml {
            path: path.to_string(),
            source,
        })?;
    let body = if closing + 1 < lines.len() {
        lines[closing + 1..].join("\n")
    } else {
        String::new()
    };
    Ok(ParsedTaskFile { frontmatter, body })
}

/// Convenience helper that wraps `parse_task_file` and ignores missing
/// files. Used by the manifest validator when iterating declared paths:
/// we want errors for "declared but missing" to be surfaced from the
/// validation layer, not from the loader.
///
/// Parse errors (malformed YAML, missing frontmatter delimiters, etc.) are
/// intentionally propagated so the validator can distinguish a true
/// "file does not exist" finding from a parse failure that demands a
/// different diagnostic.
pub fn read_task_frontmatter_or_default(
    path: &Path,
) -> Result<TaskFrontmatter, TaskFrontmatterError> {
    match parse_task_file(path) {
        Ok(parsed) => Ok(parsed.frontmatter),
        Err(TaskFrontmatterError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Ok(TaskFrontmatter::default())
        }
        Err(other) => Err(other),
    }
}

/// Returns the `TaskId` parsed from frontmatter when both `id` and a valid
/// identifier are present. Used by the manifest validator to map manifest
/// entries onto task files without depending on implicit ordering.
#[allow(dead_code)]
pub fn task_id_from(frontmatter: &TaskFrontmatter) -> Option<TaskId> {
    frontmatter
        .id
        .as_ref()
        .filter(|raw| !raw.is_empty())
        .map(|raw| TaskId::new(raw.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_frontmatter() {
        let text =
            "---\nid: OSYM-734\ntitle: Dependency Graph And Plan Checks\n---\n# Heading\nBody.\n";
        let parsed = parse_task_text(text, "test.md").expect("parse should succeed");
        assert_eq!(parsed.frontmatter.id.as_deref(), Some("OSYM-734"));
        assert_eq!(
            parsed.frontmatter.title.as_deref(),
            Some("Dependency Graph And Plan Checks")
        );
        assert!(parsed.body.starts_with("# Heading"));
        assert!(parsed.body.contains("Body."));
    }

    #[test]
    fn missing_frontmatter_is_error() {
        let err = parse_task_text("# heading\n", "test.md").expect_err("should fail");
        assert!(matches!(
            err,
            TaskFrontmatterError::MissingFrontmatter { .. }
        ));
    }

    #[test]
    fn invalid_yaml_is_error() {
        let err = parse_task_text("---\nid: [unclosed\n---\n", "test.md").expect_err("should fail");
        assert!(matches!(err, TaskFrontmatterError::Yaml { .. }));
    }

    #[test]
    fn unknown_keys_are_tolerated() {
        let text = "---\nid: OSYM-734\ntitle: T\nunknown_field: keep\n---\nbody\n";
        let parsed = parse_task_text(text, "test.md").expect("parse should succeed");
        assert_eq!(parsed.frontmatter.id.as_deref(), Some("OSYM-734"));
        assert!(parsed.frontmatter.extra.contains_key("unknown_field"));
    }

    #[test]
    fn read_task_frontmatter_or_default_swallows_missing_file_only() {
        // `NotFound` is the only IO error class we silently downgrade to
        // `TaskFrontmatter::default()`. Other errors (parse failures,
        // permission errors) are intentionally surfaced, see COE-416 review.
        let tmp = tempfile::tempdir().expect("tempdir");
        let missing = tmp.path().join("does-not-exist.md");
        let fm = read_task_frontmatter_or_default(&missing).expect("missing file OK");
        assert!(fm.id.is_none());
        assert!(fm.title.is_none());

        let dir = tmp.path();
        let malformed = dir.join("malformed.md");
        fs::write(&malformed, "---\nid: [unclosed\n---\n# body\n").expect("write");
        let err =
            read_task_frontmatter_or_default(&malformed).expect_err("yaml error must propagate");
        assert!(matches!(err, TaskFrontmatterError::Yaml { .. }));
    }
}
