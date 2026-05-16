fn read_to_string(path: &Path) -> Result<String, MemoryError> {
    fs::read_to_string(path).map_err(|source| MemoryError::ReadFile {
        path: path.to_path_buf(),
        source,
    })
}

fn create_dir_all(path: &Path) -> Result<(), MemoryError> {
    fs::create_dir_all(path).map_err(|source| MemoryError::CreateDir {
        path: path.to_path_buf(),
        source,
    })
}

fn write_file(path: &Path, contents: &str) -> Result<(), MemoryError> {
    if let Some(parent) = path.parent() {
        create_dir_all(parent)?;
    }
    fs::write(path, contents).map_err(|source| MemoryError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

fn normalize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn resolve_path(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn ensure_repo_contained(repo_root: &Path, path: &Path) -> Result<(), MemoryError> {
    let repo_root = canonicalize_existing_path(repo_root)?;
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    let resolved = canonicalize_existing_prefix(&path)?;
    if resolved.starts_with(&repo_root) {
        Ok(())
    } else {
        Err(MemoryError::PathOutsideRepo {
            path: resolved,
            repo_root,
        })
    }
}

fn canonicalize_existing_path(path: &Path) -> Result<PathBuf, MemoryError> {
    fs::canonicalize(path).map_err(|source| MemoryError::ResolvePath {
        path: path.to_path_buf(),
        source,
    })
}

fn canonicalize_existing_prefix(path: &Path) -> Result<PathBuf, MemoryError> {
    let mut cursor = path;
    let mut missing = Vec::<OsString>::new();

    loop {
        match fs::canonicalize(cursor) {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    append_missing_component(&mut resolved, component);
                }
                return Ok(resolved);
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                let Some(file_name) = cursor.file_name() else {
                    return Err(MemoryError::ResolvePath {
                        path: path.to_path_buf(),
                        source,
                    });
                };
                missing.push(file_name.to_os_string());
                let Some(parent) = cursor.parent() else {
                    return Err(MemoryError::ResolvePath {
                        path: path.to_path_buf(),
                        source,
                    });
                };
                cursor = parent;
            }
            Err(source) => {
                return Err(MemoryError::ResolvePath {
                    path: cursor.to_path_buf(),
                    source,
                });
            }
        }
    }
}

fn append_missing_component(path: &mut PathBuf, component: &OsStr) {
    if component == OsStr::new(".") {
        return;
    }
    if component == OsStr::new("..") {
        path.pop();
        return;
    }
    path.push(component);
}

fn normalize_issue_key(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

pub fn archive_blocking_warning_count(warnings: &[String]) -> usize {
    warnings
        .iter()
        .filter(|warning| is_archive_blocking_capture_warning(warning))
        .count()
}

pub fn is_archive_blocking_capture_warning(warning: &str) -> bool {
    !warning
        .trim()
        .eq_ignore_ascii_case("no GitHub PR source was matched")
}

fn sanitize_issue_key(value: &str) -> String {
    let normalized = normalize_issue_key(value);
    let sanitized = normalized
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized == normalized {
        sanitized
    } else {
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        let digest = hasher.finalize();
        format!(
            "{sanitized}-{:02x}{:02x}{:02x}{:02x}",
            digest[0], digest[1], digest[2], digest[3]
        )
    }
}

fn issue_title(issue: &IssueEvidence) -> String {
    fallback_title(&issue.title, &issue.identifier)
}

fn fallback_title(value: &str, fallback: &str) -> String {
    normalize_optional(value).unwrap_or_else(|| fallback.to_string())
}

fn normalize_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = values
        .into_iter()
        .filter_map(|value| normalize_optional(&value))
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized.sort();
    normalized.dedup();
    normalized
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn contains_issue_key(text: &str, issue_key: &str) -> bool {
    text.to_ascii_uppercase()
        .contains(&normalize_issue_key(issue_key))
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in value.trim().to_ascii_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn titleize_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn summarize_markdown(value: &str, limit: usize) -> String {
    let summary = summarize_text(value, limit);
    if summary.starts_with('-') || summary.starts_with('#') {
        summary
    } else {
        format!("{summary}\n")
    }
}

fn summarize_text(value: &str, limit: usize) -> String {
    let collapsed = value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() <= limit {
        collapsed
    } else {
        format!(
            "{}...",
            collapsed
                .chars()
                .take(limit.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn should_copy_comment_summary(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    !lower.contains("full transcript")
        && !lower.contains("assistant:")
        && !lower.contains("user:")
        && body.split_whitespace().count() < 400
}

fn short_sha(sha: &str) -> &str {
    sha.get(0..7).unwrap_or(sha)
}

fn compact_capsule_body(body: &str) -> String {
    let mut output = String::new();
    let mut include = false;
    for line in body.lines() {
        if line.starts_with("## Outcome")
            || line.starts_with("## Decisions")
            || line.starts_with("## Validation")
            || line.starts_with("## Follow-ups")
            || line.starts_with("## Documentation")
        {
            include = true;
            output.push_str(line);
            output.push('\n');
            continue;
        }
        if line.starts_with("## ") && include {
            include = false;
        }
        if include && output.lines().count() < 80 {
            output.push_str(line);
            output.push('\n');
        }
    }
    if output.trim().is_empty() {
        first_interesting_line(body)
    } else {
        output
    }
}

fn first_interesting_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("---")
                && !line.starts_with("<!--")
                && !line.starts_with("type:")
        })
        .unwrap_or("No summary available.")
        .to_string()
}

fn first_section_line(body: &str, section: &str) -> Option<String> {
    let mut in_section = false;
    for line in body.lines() {
        if line.starts_with(section) {
            in_section = true;
            continue;
        }
        if in_section && line.starts_with("## ") {
            return None;
        }
        if in_section {
            let trimmed = line.trim().trim_start_matches("- ").trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn normalize_query_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '-')
        .filter_map(normalize_optional)
        .map(|term| term.to_ascii_lowercase())
        .collect()
}

fn snippet_for_terms(body: &str, terms: &[String]) -> String {
    body.lines()
        .find(|line| {
            let lower = line.to_ascii_lowercase();
            terms.iter().any(|term| lower.contains(term))
        })
        .map(|line| summarize_text(line, 240))
        .unwrap_or_else(|| first_interesting_line(body))
}

fn contains_private_memory_link(contents: &str) -> bool {
    let contents = contents.to_ascii_lowercase();
    [
        ".opensymphony/memory/issues",
        ".opensymphony\\memory\\issues",
        "../.opensymphony/memory/issues",
    ]
    .iter()
    .any(|private_path| contents.contains(private_path))
}

fn display_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn path_relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn replace_managed_block(existing: &str, begin: &str, end: &str, replacement: &str) -> String {
    let Some(begin_index) = existing.find(begin) else {
        return existing.to_string();
    };
    let Some(end_index) = existing.find(end) else {
        return existing.to_string();
    };
    let end_index = end_index + end.len();
    let mut output = String::new();
    output.push_str(existing[..begin_index].trim_end());
    output.push_str("\n\n");
    output.push_str(replacement.trim_end());
    output.push('\n');
    output.push_str(existing[end_index..].trim_start_matches('\n'));
    output
}

fn split_issue_key(value: &str) -> Result<(String, u64), MemoryError> {
    let value = normalize_issue_key(value);
    let Some((prefix, number)) = value.rsplit_once('-') else {
        return Err(MemoryError::InvalidInput(format!(
            "issue key `{value}` must look like PREFIX-123"
        )));
    };
    let number = number.parse::<u64>().map_err(|_| {
        MemoryError::InvalidInput(format!("issue key `{value}` has an invalid numeric suffix"))
    })?;
    Ok((prefix.to_string(), number))
}

fn issue_is_before(issue_key: &str, before_issue: &str) -> bool {
    match (split_issue_key(issue_key), split_issue_key(before_issue)) {
        (Ok((issue_prefix, issue_number)), Ok((before_prefix, before_number))) => {
            issue_prefix == before_prefix && issue_number < before_number
        }
        _ => false,
    }
}
