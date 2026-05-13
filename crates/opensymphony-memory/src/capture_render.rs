fn default_config_path(repo_root: &Path) -> Option<PathBuf> {
    let public = repo_root.join(DEFAULT_MEMORY_CONFIG_FILE);
    if public.exists() {
        return Some(public);
    }
    let private = repo_root.join(FALLBACK_PRIVATE_MEMORY_CONFIG_FILE);
    if private.exists() {
        Some(private)
    } else {
        None
    }
}

fn select_issues(source: &SourceFile, selection: &IssueSelection) -> Vec<IssueEvidence> {
    let selected_identifiers = selection
        .identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    let mut issues = source.issues.clone();
    issues.retain(|issue| {
        let issue_key = normalize_issue_key(&issue.identifier);
        if !selected_identifiers.is_empty() && !selected_identifiers.contains(&issue_key) {
            return false;
        }
        if let Some(milestone) = selection
            .milestone
            .as_ref()
            .and_then(|value| normalize_optional(value))
            && issue.milestone.as_deref() != Some(milestone.as_str())
        {
            return false;
        }
        if let Some(state) = selection
            .state
            .as_ref()
            .and_then(|value| normalize_optional(value))
            && issue
                .state
                .as_deref()
                .is_none_or(|issue_state| !issue_state.eq_ignore_ascii_case(&state))
        {
            return false;
        }
        if let Some(before_date) = selection.before_date {
            let issue_date = issue
                .completed_at
                .or(issue.updated_at)
                .map(|timestamp| timestamp.date_naive());
            if issue_date.is_none_or(|date| date >= before_date) {
                return false;
            }
        }
        if let Some(before_issue) = selection.before_issue.as_deref()
            && !issue_is_before(&issue_key, before_issue)
        {
            return false;
        }
        true
    });
    issues.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    issues
}

fn matched_prs(
    source: &SourceFile,
    issue: &IssueEvidence,
    issue_key: &str,
) -> Vec<PullRequestEvidence> {
    let override_prs = source
        .overrides
        .get(issue_key)
        .or_else(|| source.overrides.get(&issue.identifier))
        .map(|override_record| override_record.prs.clone())
        .unwrap_or_default();
    let explicit = issue
        .linked_prs
        .iter()
        .chain(override_prs.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut prs = source
        .prs
        .iter()
        .filter(|pr| {
            explicit.contains(&pr.number)
                || contains_issue_key(&pr.title, issue_key)
                || pr
                    .body
                    .as_deref()
                    .is_some_and(|body| contains_issue_key(body, issue_key))
                || pr
                    .branch
                    .as_deref()
                    .is_some_and(|branch| contains_issue_key(branch, issue_key))
        })
        .cloned()
        .collect::<Vec<_>>();
    prs.sort_by_key(|pr| pr.number);
    prs.dedup_by_key(|pr| pr.number);
    prs
}

fn merge_prs(target: &mut Vec<PullRequestEvidence>, incoming: Vec<PullRequestEvidence>) {
    for pr in incoming {
        if !target.iter().any(|existing| existing.number == pr.number) {
            target.push(pr);
        }
    }
    target.sort_by_key(|pr| pr.number);
}

fn infer_areas(
    config: &MemoryConfig,
    source: &SourceFile,
    issue: &IssueEvidence,
    prs: &[PullRequestEvidence],
) -> Vec<String> {
    let issue_key = normalize_issue_key(&issue.identifier);
    if let Some(overrides) = source
        .overrides
        .get(&issue_key)
        .or_else(|| source.overrides.get(&issue.identifier))
        && !overrides.areas.is_empty()
    {
        return normalize_list(overrides.areas.clone())
            .into_iter()
            .map(|area| slugify(&area))
            .collect();
    }

    let mut areas = BTreeSet::new();
    let labels = normalize_list(issue.labels.clone());
    for (slug, area) in &config.areas {
        if labels.iter().any(|label| area.labels.contains(label)) {
            areas.insert(slug.clone());
        }
    }
    let changed_files = prs
        .iter()
        .flat_map(|pr| pr.changed_files.iter())
        .map(|file| file.path.to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();
    for (slug, area) in &config.areas {
        if area.path_hints.iter().any(|hint| {
            changed_files
                .iter()
                .any(|file| file.contains(&hint.to_ascii_lowercase()))
        }) {
            areas.insert(slug.clone());
        }
    }

    let configured_labels = config
        .areas
        .values()
        .flat_map(|area| area.labels.iter().cloned())
        .collect::<BTreeSet<_>>();
    for label in labels {
        if label != "done" && label != "bug" && label != "feature" {
            let label_slug = slugify(&label);
            if !configured_labels.contains(&label) && !areas.contains(&label_slug) {
                areas.insert(label_slug);
            }
        }
    }

    if areas.is_empty() {
        let first_path_area = prs
            .iter()
            .flat_map(|pr| pr.changed_files.iter())
            .find_map(|file| file.path.components().next())
            .map(|component| slugify(&component.as_os_str().to_string_lossy()));
        areas.insert(first_path_area.unwrap_or_else(|| "general".to_string()));
    }

    areas.into_iter().collect()
}

fn source_hash(issue: &IssueEvidence, prs: &[PullRequestEvidence]) -> Result<String, MemoryError> {
    let mut hasher = Sha256::new();
    hasher.update(serde_json::to_vec(issue)?);
    hasher.update(serde_json::to_vec(prs)?);
    Ok(format!("{:x}", hasher.finalize()))
}

fn render_issue_capsule(
    config: &MemoryConfig,
    plan: &CaptureIssuePlan,
) -> Result<String, MemoryError> {
    let issue_key = normalize_issue_key(&plan.issue.identifier);
    let frontmatter = IssueCapsuleFrontmatter {
        capsule_type: "issue-capsule",
        visibility: config.visibility,
        issue: issue_key.clone(),
        title: issue_title(&plan.issue),
        state: plan.issue.state.clone(),
        milestone: plan.issue.milestone.clone(),
        linear_url: plan.issue.url.clone(),
        prs: plan
            .prs
            .iter()
            .map(|pr| CapsulePr {
                number: pr.number,
                url: pr.url.clone(),
                merge_sha: pr.merge_sha.clone(),
            })
            .collect(),
        areas: plan.areas.clone(),
        source_refs: SourceRefs {
            linear_issue: plan
                .issue
                .url
                .as_ref()
                .map(|_| format!("linear:{issue_key}")),
            github_prs: plan
                .prs
                .iter()
                .map(|pr| format!("github:pr:{}", pr.number))
                .collect(),
        },
        captured_at: Utc::now(),
        docs_sync: DocsSyncFrontmatter {
            status: "pending".to_string(),
        },
    };
    let frontmatter =
        serde_yaml::to_string(&frontmatter).map_err(|source| MemoryError::ParseYaml {
            path: plan.capsule_path.clone(),
            source,
        })?;

    let mut markdown = String::new();
    markdown.push_str("---\n");
    markdown.push_str(&frontmatter);
    markdown.push_str("---\n\n");
    markdown.push_str(ISSUE_CAPSULE_BEGIN);
    markdown.push_str("\n\n");
    markdown.push_str(&format!("# {issue_key}: {}\n\n", issue_title(&plan.issue)));
    markdown.push_str("## Original intent\n\n");
    markdown.push_str(&render_original_intent(&plan.issue));
    markdown.push_str("\n\n## Outcome\n\n");
    markdown.push_str(&render_outcome(plan));
    markdown.push_str("\n\n## Decisions and actions\n\n");
    markdown.push_str(&render_decisions(plan));
    markdown.push_str("\n\n## Validation evidence\n\n");
    markdown.push_str(&render_validation(plan));
    markdown.push_str("\n\n## Review and rework\n\n");
    markdown.push_str(&render_reviews(plan));
    markdown.push_str("\n\n## Follow-ups and risks\n\n");
    markdown.push_str(&render_followups(plan));
    markdown.push_str("\n\n## Documentation impact\n\n");
    for target in &plan.docs_targets {
        markdown.push_str(&format!("- {}\n", display_path(&config.repo_root, target)));
    }
    if !plan.warnings.is_empty() {
        markdown.push_str("\n## Capture warnings\n\n");
        for warning in &plan.warnings {
            markdown.push_str(&format!("- {warning}\n"));
        }
    }
    markdown.push_str("\n## Provenance\n\n");
    match &plan.issue.url {
        Some(url) => markdown.push_str(&format!("- Linear: {url}\n")),
        None => markdown.push_str(&format!("- Linear: {issue_key}\n")),
    }
    for pr in &plan.prs {
        let label = pr.url.as_deref().map_or_else(
            || format!("#{}", pr.number),
            |url| format!("[#{}]({url})", pr.number),
        );
        markdown.push_str(&format!("- PR: {label}\n"));
    }
    markdown.push_str(&format!("- Debug: `opensymphony debug {issue_key}`\n"));
    markdown.push('\n');
    markdown.push_str(ISSUE_CAPSULE_END);
    markdown.push('\n');

    Ok(markdown)
}

#[derive(Debug, Serialize)]
struct IssueCapsuleFrontmatter {
    #[serde(rename = "type")]
    capsule_type: &'static str,
    visibility: MemoryVisibility,
    issue: String,
    title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    milestone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_url: Option<String>,
    prs: Vec<CapsulePr>,
    areas: Vec<String>,
    source_refs: SourceRefs,
    captured_at: DateTime<Utc>,
    docs_sync: DocsSyncFrontmatter,
}

#[derive(Debug, Serialize)]
struct CapsulePr {
    number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_sha: Option<String>,
}

#[derive(Debug, Serialize)]
struct SourceRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_issue: Option<String>,
    github_prs: Vec<String>,
}

#[derive(Debug, Serialize)]
struct DocsSyncFrontmatter {
    status: String,
}

fn render_original_intent(issue: &IssueEvidence) -> String {
    issue.description.as_deref().map_or_else(
        || "- Source issue description was not available.".to_string(),
        |description| summarize_markdown(description, 900),
    )
}

fn render_outcome(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    if plan.prs.is_empty() {
        lines.push("- No merged PR source was matched during capture.".to_string());
    } else {
        for pr in &plan.prs {
            let mut line = format!(
                "- PR #{}: {}",
                pr.number,
                fallback_title(&pr.title, "untitled PR")
            );
            if let Some(sha) = pr.merge_sha.as_deref() {
                line.push_str(&format!(" (merge `{}`)", short_sha(sha)));
            }
            lines.push(line);
        }
    }
    let changed_files = plan
        .prs
        .iter()
        .flat_map(|pr| pr.changed_files.iter())
        .take(8)
        .map(|file| format!("  - {}", file.path.display()))
        .collect::<Vec<_>>();
    if !changed_files.is_empty() {
        lines.push("- Notable changed files:".to_string());
        lines.extend(changed_files);
    }
    lines.join("\n")
}

fn render_decisions(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    for comment in &plan.issue.comments {
        if should_copy_comment_summary(&comment.body) {
            lines.push(format!("- {}", summarize_text(&comment.body, 260)));
        }
    }
    for pr in &plan.prs {
        if let Some(body) = pr.body.as_deref().and_then(normalize_optional) {
            lines.push(format!(
                "- PR #{} summary: {}",
                pr.number,
                summarize_text(&body, 260)
            ));
        }
    }
    if lines.is_empty() {
        lines.push("- No explicit decision notes were found in source evidence.".to_string());
    }
    lines.join("\n")
}

fn render_validation(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    for pr in &plan.prs {
        for check in &pr.checks {
            lines.push(format!(
                "- PR #{} `{}`: {}",
                pr.number,
                check.name,
                check.conclusion.as_deref().unwrap_or("unknown")
            ));
        }
    }
    if lines.is_empty() {
        lines.push("- No check summary source was found.".to_string());
    }
    lines.join("\n")
}

fn render_reviews(plan: &CaptureIssuePlan) -> String {
    let mut lines = Vec::new();
    for pr in &plan.prs {
        for review in &pr.reviews {
            let reviewer = review.reviewer.as_deref().unwrap_or("reviewer");
            let state = review.state.as_deref().unwrap_or("reviewed");
            let disposition = review
                .disposition
                .as_deref()
                .map(|value| format!(": {}", summarize_text(value, 180)))
                .unwrap_or_default();
            lines.push(format!(
                "- PR #{} {reviewer} {state}{disposition}",
                pr.number
            ));
        }
    }
    if lines.is_empty() {
        lines.push("- No review summary source was found.".to_string());
    }
    lines.join("\n")
}

fn render_followups(plan: &CaptureIssuePlan) -> String {
    let followups = plan
        .issue
        .comments
        .iter()
        .filter(|comment| {
            let body = comment.body.to_ascii_lowercase();
            body.contains("follow-up") || body.contains("follow up") || body.contains("risk")
        })
        .map(|comment| format!("- {}", summarize_text(&comment.body, 240)))
        .collect::<Vec<_>>();
    if followups.is_empty() {
        "- No unresolved follow-ups were identified during capture.".to_string()
    } else {
        followups.join("\n")
    }
}
