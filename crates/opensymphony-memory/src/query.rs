pub fn brief(config: &MemoryConfig, issue_key: &str) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let indexed = find_indexed_issue(config, &issue_key)?
        .ok_or_else(|| MemoryError::InvalidInput(format!("no capsule found for {issue_key}")))?;
    Ok(render_indexed_brief(config, &indexed))
}

fn render_indexed_brief(config: &MemoryConfig, indexed: &IndexedIssue) -> String {
    let mut output = String::new();
    output.push_str(&format!("# {}: {}\n\n", indexed.issue_key, indexed.title));
    output.push_str(&format!(
        "- Capsule: {}\n",
        display_path(&config.repo_root, &indexed.capsule_path)
    ));
    output.push_str(&format!("- Visibility: {}\n", indexed.visibility));
    if !indexed.areas().is_empty() {
        output.push_str(&format!("- Areas: {}\n", indexed.areas().join(", ")));
    }
    output.push('\n');
    output.push_str(&compact_capsule_body(&indexed.body));
    output
}

pub fn search(
    config: &MemoryConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    search_with_scope(config, query, limit, &MemoryScopeFilter::default())
}

pub fn search_with_scope(
    config: &MemoryConfig,
    query: &str,
    limit: usize,
    scope: &MemoryScopeFilter,
) -> Result<Vec<SearchResult>, MemoryError> {
    let terms = normalize_query_terms(query);
    if terms.is_empty() {
        return Err(MemoryError::InvalidInput(
            "search query must not be empty".to_string(),
        ));
    }

    let mut scored = Vec::new();
    for indexed in load_indexed_issues(config)?
        .into_iter()
        .filter(|issue| indexed_issue_matches_scope(config, issue, scope))
    {
        let haystack = format!(
            "{} {} {} {}",
            indexed.issue_key,
            indexed.title,
            indexed.labels.join(" "),
            indexed.body
        )
        .to_ascii_lowercase();
        let score = terms
            .iter()
            .filter(|term| haystack.contains(term.as_str()))
            .count();
        if score > 0 {
            scored.push((
                score,
                SearchResult {
                    issue_key: indexed.issue_key.clone(),
                    title: indexed.title.clone(),
                    capsule_path: indexed.capsule_path.clone(),
                    areas: indexed.areas(),
                    snippet: snippet_for_terms(&indexed.body, &terms),
                },
            ));
        }
    }
    scored.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.issue_key.cmp(&right.1.issue_key))
    });
    Ok(scored
        .into_iter()
        .take(limit.max(1))
        .map(|(_, result)| result)
        .collect())
}

pub fn related_by_issue(
    config: &MemoryConfig,
    issue_key: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    related_by_issue_with_scope(config, issue_key, limit, &MemoryScopeFilter::default())
}

pub fn related_by_issue_with_scope(
    config: &MemoryConfig,
    issue_key: &str,
    limit: usize,
    scope: &MemoryScopeFilter,
) -> Result<Vec<SearchResult>, MemoryError> {
    let issue_key = normalize_issue_key(issue_key);
    let indexed = find_indexed_issue(config, &issue_key)?
        .ok_or_else(|| MemoryError::InvalidInput(format!("no capsule found for {issue_key}")))?;
    let mut related = Vec::new();
    let indexed_areas = indexed.areas();
    for candidate in load_indexed_issues(config)?
        .into_iter()
        .filter(|issue| indexed_issue_matches_scope(config, issue, scope))
    {
        if candidate.issue_key == issue_key {
            continue;
        }
        let candidate_areas = candidate.areas();
        let overlap = candidate_areas
            .iter()
            .filter(|area| indexed_areas.contains(area))
            .count();
        if overlap > 0 {
            related.push((
                overlap,
                SearchResult {
                    issue_key: candidate.issue_key.clone(),
                    title: candidate.title.clone(),
                    capsule_path: candidate.capsule_path.clone(),
                    areas: candidate_areas,
                    snippet: first_interesting_line(&candidate.body),
                },
            ));
        }
    }
    related.sort_by(|left, right| {
        right
            .0
            .cmp(&left.0)
            .then_with(|| left.1.issue_key.cmp(&right.1.issue_key))
    });
    Ok(related
        .into_iter()
        .take(limit.max(1))
        .map(|(_, result)| result)
        .collect())
}

pub fn related_by_area(
    config: &MemoryConfig,
    area: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    related_by_area_with_scope(config, area, limit, &MemoryScopeFilter::default())
}

pub fn related_by_area_with_scope(
    config: &MemoryConfig,
    area: &str,
    limit: usize,
    scope: &MemoryScopeFilter,
) -> Result<Vec<SearchResult>, MemoryError> {
    let area = slugify(area);
    let mut results = Vec::new();
    for candidate in load_indexed_issues(config)?
        .into_iter()
        .filter(|issue| indexed_issue_matches_scope(config, issue, scope))
    {
        let areas = candidate.areas();
        if areas.iter().any(|candidate_area| candidate_area == &area) {
            results.push(SearchResult {
                issue_key: candidate.issue_key.clone(),
                title: candidate.title.clone(),
                capsule_path: candidate.capsule_path.clone(),
                areas,
                snippet: first_interesting_line(&candidate.body),
            });
        }
    }
    results.sort_by(|left, right| left.issue_key.cmp(&right.issue_key));
    results.truncate(limit.max(1));
    Ok(results)
}

pub fn related_by_paths(
    config: &MemoryConfig,
    paths: &[PathBuf],
    limit: usize,
) -> Result<Vec<SearchResult>, MemoryError> {
    related_by_paths_with_scope(config, paths, limit, &MemoryScopeFilter::default())
}

pub fn related_by_paths_with_scope(
    config: &MemoryConfig,
    paths: &[PathBuf],
    limit: usize,
    scope: &MemoryScopeFilter,
) -> Result<Vec<SearchResult>, MemoryError> {
    let terms = paths
        .iter()
        .flat_map(|path| {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .filter_map(|value| normalize_optional(&value))
        .collect::<Vec<_>>();
    search_with_scope(config, &terms.join(" "), limit, scope)
}

pub fn docs_for_area(config: &MemoryConfig, area: &str) -> Result<String, MemoryError> {
    docs_for_area_with_scope(config, area, &MemoryScopeFilter::default())
}

pub fn docs_for_area_with_scope(
    config: &MemoryConfig,
    area: &str,
    scope: &MemoryScopeFilter,
) -> Result<String, MemoryError> {
    let area = config.area_or_default(area);
    if !area.docs_target.exists() {
        return Err(MemoryError::InvalidInput(format!(
            "no topic doc exists for area `{}` at {}",
            area.slug,
            area.docs_target.display()
        )));
    }
    if docs_scope_requires_index_check(scope) {
        let mut scoped = scope.clone();
        scoped.area = Some(area.slug.clone());
        let issues = load_indexed_issues(config)?;
        if !issues
            .iter()
            .any(|issue| indexed_issue_matches_scope(config, issue, &scoped))
        {
            return Err(MemoryError::InvalidInput(format!(
                "no captured memory for area `{}` in the requested docs scope",
                area.slug
            )));
        }
    }
    read_to_string(&area.docs_target)
}

fn docs_scope_requires_index_check(scope: &MemoryScopeFilter) -> bool {
    scope.issue.is_some() || scope.milestone.is_some() || scope.repo.is_some()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContextOptions {
    pub issue: String,
    pub explicit_includes: Vec<String>,
    pub paths: Vec<PathBuf>,
    pub limit: usize,
}

impl MemoryContextOptions {
    pub fn for_issue(issue: impl Into<String>, limit: usize) -> Self {
        Self {
            issue: issue.into(),
            explicit_includes: Vec::new(),
            paths: Vec::new(),
            limit,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ContextBucket {
    ExplicitIncludes,
    BlockingPredecessors,
    CompletedChildren,
    CompletedSiblings,
    PathMatches,
    AreaMatches,
}

impl ContextBucket {
    fn title(self) -> &'static str {
        match self {
            Self::ExplicitIncludes => "Explicit Includes",
            Self::BlockingPredecessors => "Blocking Predecessors",
            Self::CompletedChildren => "Completed Children",
            Self::CompletedSiblings => "Completed Siblings",
            Self::PathMatches => "Path Matches",
            Self::AreaMatches => "Area Matches",
        }
    }

    fn reason(self) -> &'static str {
        match self {
            Self::ExplicitIncludes => "explicit include",
            Self::BlockingPredecessors => "blocking predecessor",
            Self::CompletedChildren => "completed child",
            Self::CompletedSiblings => "completed sibling",
            Self::PathMatches => "path match",
            Self::AreaMatches => "area match",
        }
    }

    fn cap(self) -> usize {
        match self {
            Self::ExplicitIncludes => 12,
            Self::BlockingPredecessors => 12,
            Self::CompletedChildren => 12,
            Self::CompletedSiblings => 6,
            Self::PathMatches => 8,
            Self::AreaMatches => 8,
        }
    }

    fn ordered() -> [Self; 6] {
        [
            Self::ExplicitIncludes,
            Self::BlockingPredecessors,
            Self::CompletedChildren,
            Self::CompletedSiblings,
            Self::PathMatches,
            Self::AreaMatches,
        ]
    }
}

#[derive(Debug, Clone, Default)]
struct ContextCandidate {
    issue_key: String,
    reasons: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct SelectedContextBrief {
    bucket: ContextBucket,
    issue_key: String,
    title: String,
    reasons: Vec<String>,
    body: String,
}

pub fn context_for_issue(
    config: &MemoryConfig,
    source: &SourceFile,
    issue_key: &str,
    limit: usize,
) -> Result<String, MemoryError> {
    let options = MemoryContextOptions::for_issue(issue_key, limit);
    context_for_issue_with_options(config, source, &options)
}

pub fn context_for_issue_with_options(
    config: &MemoryConfig,
    source: &SourceFile,
    options: &MemoryContextOptions,
) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(&options.issue);
    let max_total = options.limit.clamp(1, 20);
    let indexed_issues = load_indexed_issues(config)?;
    let indexed_by_key = indexed_issues
        .iter()
        .map(|issue| (issue.issue_key.clone(), issue))
        .collect::<BTreeMap<_, _>>();
    let source_by_key = source
        .issues
        .iter()
        .map(|issue| (normalize_issue_key(&issue.identifier), issue))
        .collect::<BTreeMap<_, _>>();
    let current_issue = source_by_key.get(&issue_key).copied();
    let mut candidates = BTreeMap::<ContextBucket, Vec<ContextCandidate>>::new();
    let mut missing_capsules = Vec::<String>::new();

    for include in &options.explicit_includes {
        add_context_candidate(
            &mut candidates,
            ContextBucket::ExplicitIncludes,
            include,
            &issue_key,
            ContextBucket::ExplicitIncludes.reason(),
        );
    }

    if let Some(issue) = current_issue {
        for blocker in &issue.blocked_by {
            let blocker_key = normalize_issue_key(&blocker.identifier);
            add_context_candidate(
                &mut candidates,
                ContextBucket::BlockingPredecessors,
                &blocker_key,
                &issue_key,
                ContextBucket::BlockingPredecessors.reason(),
            );
            if !indexed_by_key.contains_key(&blocker_key) {
                missing_capsules.push(format!(
                    "- Blocking predecessor {blocker_key}: captured memory is missing."
                ));
            }
        }

        for child in &issue.children {
            let child_key = normalize_issue_key(&child.identifier);
            if link_is_completed(child) || indexed_by_key.contains_key(&child_key) {
                add_context_candidate(
                    &mut candidates,
                    ContextBucket::CompletedChildren,
                    &child_key,
                    &issue_key,
                    ContextBucket::CompletedChildren.reason(),
                );
            }
            if link_is_completed(child) && !indexed_by_key.contains_key(&child_key) {
                missing_capsules.push(format!(
                    "- Completed child {child_key}: captured memory is missing."
                ));
            }
        }

        if let Some(parent_key) = issue
            .parent
            .as_ref()
            .map(|parent| normalize_issue_key(&parent.identifier))
            && let Some(parent) = source_by_key.get(&parent_key).copied()
        {
            for sibling in &parent.children {
                let sibling_key = normalize_issue_key(&sibling.identifier);
                if sibling_key == issue_key {
                    continue;
                }
                if link_is_completed(sibling) || indexed_by_key.contains_key(&sibling_key) {
                    add_context_candidate(
                        &mut candidates,
                        ContextBucket::CompletedSiblings,
                        &sibling_key,
                        &issue_key,
                        ContextBucket::CompletedSiblings.reason(),
                    );
                }
            }
        }

        let current_areas = canonical_issue_area_slugs(config, issue);
        for indexed in &indexed_issues {
            if indexed.issue_key == issue_key {
                continue;
            }
            if indexed
                .areas()
                .iter()
                .any(|area| current_areas.contains(area))
            {
                add_context_candidate(
                    &mut candidates,
                    ContextBucket::AreaMatches,
                    &indexed.issue_key,
                    &issue_key,
                    ContextBucket::AreaMatches.reason(),
                );
            }
        }
    }

    if !options.paths.is_empty() {
        for result in related_by_paths(config, &options.paths, ContextBucket::PathMatches.cap())? {
            add_context_candidate(
                &mut candidates,
                ContextBucket::PathMatches,
                &result.issue_key,
                &issue_key,
                ContextBucket::PathMatches.reason(),
            );
        }
    }

    let mut selected = Vec::<SelectedContextBrief>::new();
    let mut emitted = BTreeSet::<String>::new();
    let mut documentation_paths = BTreeSet::<String>::new();
    for bucket in ContextBucket::ordered() {
        let Some(bucket_candidates) = candidates.get(&bucket) else {
            continue;
        };
        let mut bucket_count = 0;
        for candidate in bucket_candidates {
            if selected.len() >= max_total || bucket_count >= bucket.cap() {
                break;
            }
            if !emitted.insert(candidate.issue_key.clone()) {
                continue;
            }
            let Some(indexed) = indexed_by_key.get(&candidate.issue_key) else {
                continue;
            };
            let (body, docs) = strip_documentation_impact_section(&render_indexed_brief(config, indexed));
            documentation_paths.extend(docs);
            selected.push(SelectedContextBrief {
                bucket,
                issue_key: indexed.issue_key.clone(),
                title: indexed.title.clone(),
                reasons: context_reasons(&candidates, &candidate.issue_key),
                body,
            });
            bucket_count += 1;
        }
    }

    Ok(render_memory_context(
        &issue_key,
        current_issue,
        &selected,
        &missing_capsules,
        &documentation_paths,
    ))
}

fn context_reasons(
    candidates: &BTreeMap<ContextBucket, Vec<ContextCandidate>>,
    issue_key: &str,
) -> Vec<String> {
    let mut reasons = BTreeSet::new();
    for bucket_candidates in candidates.values() {
        for candidate in bucket_candidates {
            if candidate.issue_key == issue_key {
                reasons.extend(candidate.reasons.iter().cloned());
            }
        }
    }
    reasons.into_iter().collect()
}

fn add_context_candidate(
    candidates: &mut BTreeMap<ContextBucket, Vec<ContextCandidate>>,
    bucket: ContextBucket,
    issue_key: &str,
    current_issue_key: &str,
    reason: &str,
) {
    let issue_key = normalize_issue_key(issue_key);
    if issue_key.is_empty() || issue_key == current_issue_key {
        return;
    }
    let bucket_candidates = candidates.entry(bucket).or_default();
    if let Some(candidate) = bucket_candidates
        .iter_mut()
        .find(|candidate| candidate.issue_key == issue_key)
    {
        candidate.reasons.insert(reason.to_string());
        return;
    }
    bucket_candidates.push(ContextCandidate {
        issue_key,
        reasons: BTreeSet::from([reason.to_string()]),
    });
}

fn canonical_issue_area_slugs(config: &MemoryConfig, issue: &IssueEvidence) -> BTreeSet<String> {
    let labels = normalize_list(issue.labels.clone());
    let mut areas = BTreeSet::new();
    for label in &labels {
        if let Some(area) = canonical_area_label_slug(label) {
            areas.insert(area);
        }
    }
    for (slug, area) in &config.areas {
        if labels.iter().any(|label| area_alias_matches(area, label)) {
            areas.insert(slug.clone());
        }
    }
    areas
}

fn link_is_completed(link: &IssueLinkEvidence) -> bool {
    link.state
        .as_deref()
        .and_then(normalize_optional)
        .is_some_and(|state| {
            matches!(
                state.to_ascii_lowercase().as_str(),
                "done" | "completed" | "closed" | "cancelled" | "canceled" | "duplicate"
            )
        })
}

fn strip_documentation_impact_section(markdown: &str) -> (String, Vec<String>) {
    let mut output = String::new();
    let mut docs = Vec::new();
    let mut in_documentation = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("## Documentation impact") {
            in_documentation = true;
            continue;
        }
        if in_documentation && trimmed.starts_with("## ") {
            in_documentation = false;
        }
        if in_documentation {
            if let Some(path) = trimmed
                .strip_prefix("- ")
                .and_then(normalize_optional)
            {
                docs.push(path);
            }
            continue;
        }
        output.push_str(line);
        output.push('\n');
    }

    (output.trim_end().to_string() + "\n", docs)
}

fn render_memory_context(
    issue_key: &str,
    current_issue: Option<&IssueEvidence>,
    selected: &[SelectedContextBrief],
    missing_capsules: &[String],
    documentation_paths: &BTreeSet<String>,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("# Memory Context: {issue_key}\n\n"));
    output.push_str(
        "This is pre-implementation guidance assembled from completed captured memory. It intentionally excludes any capsule for the current issue.\n\n",
    );
    if let Some(issue) = current_issue {
        output.push_str(&format!("## Current Issue\n\n{}\n\n", issue_title(issue)));
        if let Some(description) = issue.description.as_deref().and_then(normalize_optional) {
            output.push_str(&format!("{}\n\n", summarize_text(&description, 600)));
        }
    }

    if selected.is_empty() {
        output.push_str("## Selected Memory\n\n- No deterministic captured memory found.\n\n");
    } else {
        for bucket in ContextBucket::ordered() {
            let bucket_briefs = selected
                .iter()
                .filter(|brief| brief.bucket == bucket)
                .collect::<Vec<_>>();
            if bucket_briefs.is_empty() {
                continue;
            }
            output.push_str(&format!("## {}\n\n", bucket.title()));
            for brief in bucket_briefs {
                output.push_str(&format!("### {}: {}\n\n", brief.issue_key, brief.title));
                output.push_str(&format!("- Reasons: {}\n\n", brief.reasons.join(", ")));
                output.push_str(&brief.body);
                if !brief.body.ends_with("\n\n") {
                    output.push('\n');
                }
            }
        }
    }

    if !missing_capsules.is_empty() {
        output.push_str("## Missing Captures\n\n");
        for line in missing_capsules {
            output.push_str(line);
            output.push('\n');
        }
        output.push('\n');
    }

    output.push_str("## Guidance\n\n");
    output.push_str("- Treat memory as context, not as authority over current code.\n");
    output.push_str("- Inspect referenced docs and current files before editing.\n");
    output.push_str(
        "- Re-run `opensymphony memory context --paths ... --include-code-intel` after file discovery.\n",
    );
    output.push_str("- Use `opensymphony debug ");
    output.push_str(issue_key);
    output.push_str("` only when the original agent conversation is needed.\n");

    if !documentation_paths.is_empty() {
        output.push_str("\n## Documentation impact\n\n");
        for path in documentation_paths {
            output.push_str(&format!("- {path}\n"));
        }
    }

    output
}

pub fn status(
    config: &MemoryConfig,
    selection: &IssueSelection,
) -> Result<StatusReport, MemoryError> {
    status_with_scope(config, selection, &MemoryScopeFilter::default())
}

pub fn status_with_scope(
    config: &MemoryConfig,
    selection: &IssueSelection,
    scope: &MemoryScopeFilter,
) -> Result<StatusReport, MemoryError> {
    let mut issues = load_indexed_issues(config)?;
    issues.retain(|issue| indexed_issue_matches_scope(config, issue, scope));
    if let Some(area) = selection.area.as_ref().map(|area| slugify(area)) {
        issues.retain(|issue| issue.areas().contains(&area));
    }
    if let Some(milestone) = selection
        .milestone
        .as_ref()
        .and_then(|value| normalize_optional(value))
    {
        issues.retain(|issue| issue.milestone.as_deref() == Some(milestone.as_str()));
    }

    let warning_count = issues.iter().map(|issue| issue.warning_count).sum();
    let docs_pending_count = issues
        .iter()
        .filter(|issue| issue.docs_sync_status == "pending")
        .count();
    let status_issues = issues
        .into_iter()
        .map(|issue| {
            let areas = issue.areas();
            StatusIssue {
                issue_key: issue.issue_key,
                title: issue.title,
                state: issue.state,
                milestone: issue.milestone,
                capsule_path: issue.capsule_path,
                visibility: issue.visibility,
                areas,
                docs_sync_status: issue.docs_sync_status,
                warning_count: issue.warning_count,
            }
        })
        .collect::<Vec<_>>();

    Ok(StatusReport {
        issue_count: status_issues.len(),
        warning_count,
        docs_pending_count,
        issues: status_issues,
    })
}

fn indexed_issue_matches_scope(
    config: &MemoryConfig,
    issue: &IndexedIssue,
    scope: &MemoryScopeFilter,
) -> bool {
    if let Some(issue_key) = scope.issue.as_ref().map(|issue| normalize_issue_key(issue))
        && issue.issue_key != issue_key
    {
        return false;
    }
    if let Some(milestone) = scope.milestone.as_ref().and_then(|value| normalize_optional(value))
        && issue.milestone.as_deref() != Some(milestone.as_str())
    {
        return false;
    }
    if let Some(area) = scope.area.as_ref().map(|area| slugify(area))
        && !issue.areas().contains(&area)
    {
        return false;
    }
    if let Some(repo) = scope.repo.as_ref().and_then(|value| normalize_optional(value))
        && !indexed_issue_matches_repo(config, issue, &repo)
    {
        return false;
    }
    true
}

fn indexed_issue_matches_repo(config: &MemoryConfig, issue: &IndexedIssue, repo: &str) -> bool {
    let repo = repo_scope_prefix(config, repo);
    if repo.is_empty() || repo == "." {
        return true;
    }
    issue.changed_files.iter().any(|path| {
        let path = path.to_string_lossy();
        path == repo || path.starts_with(&format!("{repo}/"))
    })
}

fn repo_scope_prefix(config: &MemoryConfig, repo: &str) -> String {
    let path = PathBuf::from(repo);
    let relative = if path.is_absolute() {
        path.strip_prefix(&config.repo_root)
            .map(Path::to_path_buf)
            .unwrap_or(path)
    } else {
        path
    };
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .filter(|component| component != ".")
        .collect::<Vec<_>>()
        .join("/")
}

pub fn lint(config: &MemoryConfig, public_docs: bool) -> Result<LintReport, MemoryError> {
    let mut findings = Vec::new();
    let issues = load_indexed_issues(config)?;
    for issue in &issues {
        if issue.warning_count > 0 {
            findings.push(LintFinding {
                severity: LintSeverity::Warn,
                path: Some(issue.capsule_path.clone()),
                message: format!(
                    "{} has {} unresolved capture warning(s)",
                    issue.issue_key, issue.warning_count
                ),
                next_command: Some(format!("opensymphony memory show {}", issue.issue_key)),
            });
        }
        if issue.areas().is_empty() {
            findings.push(LintFinding {
                severity: LintSeverity::Error,
                path: Some(issue.capsule_path.clone()),
                message: format!("{} has no learned memory area", issue.issue_key),
                next_command: Some(format!(
                    "opensymphony memory capture {} --force",
                    issue.issue_key
                )),
            });
        }
    }

    if public_docs && config.docs.deny_private_links {
        for area in all_known_areas(config, &issues) {
            let path = area.docs_target;
            if !path.exists() {
                continue;
            }
            let contents = read_to_string(&path)?;
            if contains_private_memory_link(&markdown_visible_text(&contents)) {
                findings.push(LintFinding {
                    severity: LintSeverity::Error,
                    path: Some(path),
                    message: "public docs contain a private memory path".to_string(),
                    next_command: Some("opensymphony memory sync-docs".to_string()),
                });
            }
        }
    }

    Ok(LintReport { findings })
}
