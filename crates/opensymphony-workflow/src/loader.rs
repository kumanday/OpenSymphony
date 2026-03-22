use std::{fs, path::Path};

use crate::{error::WorkflowLoadError, WorkflowDefinition, WorkflowFrontMatter};

pub(crate) fn load_workflow_from_path(
    path: &Path,
) -> Result<WorkflowDefinition, WorkflowLoadError> {
    let contents = fs::read_to_string(path).map_err(|source| match source.kind() {
        std::io::ErrorKind::NotFound => WorkflowLoadError::MissingWorkflowFile {
            path: path.to_path_buf(),
        },
        _ => WorkflowLoadError::ReadWorkflowFile {
            path: path.to_path_buf(),
            source,
        },
    })?;

    parse_workflow(&contents)
}

pub(crate) fn parse_workflow(source: &str) -> Result<WorkflowDefinition, WorkflowLoadError> {
    let (front_matter_source, prompt_source) = split_front_matter(source)?;
    let front_matter = match front_matter_source {
        Some(front_matter) => parse_front_matter(front_matter)?,
        None => WorkflowFrontMatter::default(),
    };

    Ok(WorkflowDefinition {
        front_matter,
        prompt_template: prompt_source.to_owned(),
    })
}

fn parse_front_matter(front_matter: &str) -> Result<WorkflowFrontMatter, WorkflowLoadError> {
    let parsed = serde_yaml::from_str::<serde_yaml::Value>(front_matter)
        .map_err(|source| WorkflowLoadError::WorkflowParseError { source })?;

    match parsed {
        serde_yaml::Value::Null => Ok(WorkflowFrontMatter::default()),
        serde_yaml::Value::Mapping(_) => {
            let parsed: WorkflowFrontMatter = serde_yaml::from_value(parsed)
                .map_err(|source| WorkflowLoadError::WorkflowParseError { source })?;

            match parsed.extensions.keys().next() {
                Some(namespace) => Err(WorkflowLoadError::UnknownTopLevelNamespace {
                    namespace: namespace.clone(),
                }),
                None => Ok(parsed),
            }
        }
        _ => Err(WorkflowLoadError::WorkflowFrontMatterNotAMap),
    }
}

fn split_front_matter(source: &str) -> Result<(Option<&str>, &str), WorkflowLoadError> {
    let mut lines = source.split_inclusive('\n');
    let Some(first_line) = lines.next() else {
        return Ok((None, ""));
    };

    if trim_line(first_line) != "---" {
        return Ok((None, source));
    }

    let mut offset = first_line.len();
    for line in lines {
        let line_length = line.len();
        if trim_line(line) == "---" {
            let body_start = offset + line_length;
            return Ok((
                Some(&source[first_line.len()..offset]),
                &source[body_start..],
            ));
        }

        offset += line_length;
    }

    Ok((None, source))
}

fn trim_line(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}
