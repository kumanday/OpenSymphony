fn default_config_path(repo_root: &Path) -> Option<PathBuf> {
    let private = repo_root.join(DEFAULT_PRIVATE_MEMORY_CONFIG_FILE);
    if private.exists() {
        return Some(private);
    }
    let public = repo_root.join(DEFAULT_MEMORY_CONFIG_FILE);
    if public.exists() {
        return Some(public);
    }
    let legacy_private = repo_root.join(FALLBACK_PRIVATE_MEMORY_CONFIG_FILE);
    if legacy_private.exists() {
        Some(legacy_private)
    } else {
        None
    }
}

fn select_issues(source: &SourceFile, selection: &IssueSelection) -> Vec<IssueEvidence> {
    let selected_identifiers = expanded_selected_identifiers(source, &selection.identifiers);
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

fn expanded_selected_identifiers(source: &SourceFile, identifiers: &[String]) -> BTreeSet<String> {
    let mut selected = identifiers
        .iter()
        .map(|identifier| normalize_issue_key(identifier))
        .collect::<BTreeSet<_>>();
    if selected.is_empty() {
        return selected;
    }

    let issue_by_key = source
        .issues
        .iter()
        .map(|issue| (normalize_issue_key(&issue.identifier), issue))
        .collect::<BTreeMap<_, _>>();
    let mut queue = selected.iter().cloned().collect::<Vec<_>>();
    while let Some(issue_key) = queue.pop() {
        let Some(issue) = issue_by_key.get(&issue_key) else {
            continue;
        };
        for child in &issue.children {
            let child_key = normalize_issue_key(&child.identifier);
            if selected.insert(child_key.clone()) {
                queue.push(child_key);
            }
        }
    }
    selected
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
    for label in &labels {
        if let Some(area) = canonical_area_label_slug(label) {
            areas.insert(area);
        }
    }
    for (slug, area) in &config.areas {
        if labels.iter().any(|label| area_alias_matches(area, label))
            || area_matches_evidence(area, issue, prs)
        {
            areas.insert(slug.clone());
        }
    }
    for label in labels {
        if !is_generic_label(&label) {
            let label_slug = canonical_area_label_slug(&label).unwrap_or_else(|| slugify(&label));
            if !areas.contains(&label_slug) {
                areas.insert(label_slug);
            }
        }
    }

    if areas.is_empty()
        && let Some(milestone) = issue.milestone.as_deref().and_then(normalize_optional)
        && !is_generic_milestone(&milestone)
    {
        areas.insert(slugify(&milestone));
    }
    if areas.is_empty()
        && let Some(candidate) = infer_candidate_area_from_narrative(issue, prs)
    {
        areas.insert(candidate);
    }
    if areas.is_empty() {
        areas.insert("general".to_string());
    }

    areas.into_iter().collect()
}

fn area_alias_matches(area: &AreaConfig, value: &str) -> bool {
    let value_slug = slugify(value);
    let canonical_area = canonical_area_label_slug(value);
    canonical_area.as_deref() == Some(area.slug.as_str())
        || value_slug == area.slug
        || area.aliases.iter().any(|alias| {
            let alias_slug = slugify(alias);
            value_slug == alias_slug || value == alias
        })
        || area
            .source_refs
            .linear_labels
            .iter()
            .any(|label| {
                canonical_area_label_slug(label).as_deref() == Some(area.slug.as_str())
                    || value_slug == slugify(label)
            })
}

fn canonical_area_label_slug(value: &str) -> Option<String> {
    let (prefix, suffix) = value.trim().split_once(':')?;
    if !prefix.eq_ignore_ascii_case("area") {
        return None;
    }
    let slug = slugify(suffix);
    if slug.is_empty() { None } else { Some(slug) }
}

fn area_matches_evidence(
    area: &AreaConfig,
    issue: &IssueEvidence,
    prs: &[PullRequestEvidence],
) -> bool {
    let evidence = search_tokens(&area_evidence_text(issue, prs));
    let mut candidates = vec![area.slug.clone(), area.title.to_ascii_lowercase()];
    candidates.extend(area.aliases.clone());
    candidates.extend(area.source_refs.linear_milestones.clone());
    candidates.into_iter().any(|candidate| {
        let candidate = candidate.trim().to_ascii_lowercase();
        candidate.len() >= 4 && token_sequence_contains(&evidence, &search_tokens(&candidate))
    })
}

fn token_sequence_contains(haystack: &[String], needle: &[String]) -> bool {
    // Whole-token equality keeps aliases like "runtime" from matching tokens
    // such as "gruntime" or "runtimeerror".
    !needle.is_empty()
        && needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn search_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .filter_map(normalize_optional)
        .map(|token| token.to_ascii_lowercase())
        .collect()
}

fn area_evidence_text(issue: &IssueEvidence, prs: &[PullRequestEvidence]) -> String {
    let mut parts = vec![issue.title.clone()];
    if let Some(description) = &issue.description {
        parts.push(description.clone());
    }
    if let Some(milestone) = &issue.milestone {
        parts.push(milestone.clone());
    }
    if let Some(parent) = &issue.parent
        && let Some(title) = &parent.title
    {
        parts.push(title.clone());
    }
    for child in &issue.children {
        if let Some(title) = &child.title {
            parts.push(title.clone());
        }
    }
    parts.extend(issue.comments.iter().map(|comment| comment.body.clone()));
    for pr in prs {
        parts.push(pr.title.clone());
        if let Some(body) = &pr.body {
            parts.push(body.clone());
        }
        for review in &pr.reviews {
            if let Some(disposition) = &review.disposition {
                parts.push(disposition.clone());
            }
        }
        for check in &pr.checks {
            parts.push(check.name.clone());
            if let Some(conclusion) = &check.conclusion {
                parts.push(conclusion.clone());
            }
        }
    }
    parts.join("\n").to_ascii_lowercase()
}

fn is_generic_label(label: &str) -> bool {
    matches!(
        label,
        "done"
            | "bug"
            | "feature"
            | "enhancement"
            | "documentation"
            | "docs"
            | "urgent"
            | "high"
            | "medium"
            | "low"
            | "priority"
    )
}

fn is_generic_milestone(milestone: &str) -> bool {
    let slug = slugify(milestone);
    slug.is_empty()
        || slug.chars().all(|character| character.is_ascii_digit())
        || slug.len() <= 2
        || matches!(slug.as_str(), "mvp" | "v1" | "v2" | "m1" | "m2" | "m3" | "m4")
}

fn infer_candidate_area_from_narrative(
    issue: &IssueEvidence,
    prs: &[PullRequestEvidence],
) -> Option<String> {
    for text in std::iter::once(issue.title.as_str())
        .chain(issue.description.iter().map(String::as_str))
        .chain(
            issue
                .parent
                .iter()
                .filter_map(|parent| parent.title.as_deref()),
        )
        .chain(issue.children.iter().filter_map(|child| child.title.as_deref()))
        .chain(issue.comments.iter().map(|comment| comment.body.as_str()))
        .chain(prs.iter().map(|pr| pr.title.as_str()))
        .chain(prs.iter().filter_map(|pr| pr.body.as_deref()))
        .chain(
            prs.iter()
                .flat_map(|pr| pr.reviews.iter())
                .filter_map(|review| review.disposition.as_deref()),
        )
        .chain(
            prs.iter()
                .flat_map(|pr| pr.checks.iter())
                .map(|check| check.name.as_str()),
        )
    {
        if let Some(slug) = candidate_area_slug_from_text(text) {
            return Some(slug);
        }
    }
    None
}

fn candidate_area_slug_from_text(text: &str) -> Option<String> {
    let mut tokens = Vec::new();
    let raw_tokens = text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.trim().is_empty())
        .collect::<Vec<_>>();
    for (index, raw) in raw_tokens.iter().enumerate() {
        if raw.len() <= 5
            && raw.chars().all(|character| character.is_ascii_uppercase())
            && raw_tokens
                .get(index + 1)
                .is_some_and(|next| next.chars().all(|character| character.is_ascii_digit()))
        {
            continue;
        }
        let token = raw.trim().to_ascii_lowercase();
        if token.len() < 3
            || token.chars().all(|character| character.is_ascii_digit())
            || looks_like_issue_key_token(&token)
            || is_area_stopword(&token)
        {
            continue;
        }
        tokens.push(token);
        if tokens.len() == 3 {
            break;
        }
    }
    if tokens.is_empty() {
        return None;
    }
    Some(slugify(&tokens.join("-")))
}

fn looks_like_issue_key_token(token: &str) -> bool {
    let Some((prefix, suffix)) = token.split_once('-') else {
        return false;
    };
    prefix.chars().all(|character| character.is_ascii_alphabetic())
        && suffix.chars().all(|character| character.is_ascii_digit())
}

fn is_area_stopword(token: &str) -> bool {
    matches!(
        token,
        "add"
            | "adds"
            | "added"
            | "build"
            | "create"
            | "define"
            | "fix"
            | "for"
            | "from"
            | "implement"
            | "implements"
            | "issue"
            | "make"
            | "new"
            | "support"
            | "the"
            | "this"
            | "update"
            | "updates"
            | "with"
    )
}

fn evolve_memory_config(
    config: &MemoryConfig,
    plan: &CapturePlan,
) -> MemoryConfig {
    let mut evolved = config.clone();
    for issue_plan in &plan.selected {
        for area_slug in &issue_plan.areas {
            let mut area = evolved.area_or_default(area_slug);
            merge_area_evidence(&mut area, issue_plan, config.confidence_threshold);
            evolved.areas.insert(area.slug.clone(), area);
        }
    }
    evolved
}

fn merge_area_evidence(
    area: &mut AreaConfig,
    issue_plan: &CaptureIssuePlan,
    confidence_threshold: u8,
) {
    if let Some(milestone) = issue_plan.issue.milestone.as_deref().and_then(normalize_optional) {
        push_unique(&mut area.source_refs.linear_milestones, milestone);
    }
    for label in normalize_list(issue_plan.issue.labels.clone()) {
        let label_area = canonical_area_label_slug(&label);
        if label_area.as_deref() == Some(area.slug.as_str())
            || slugify(&label) == area.slug
            || area_alias_matches(area, &label)
        {
            push_unique(&mut area.source_refs.linear_labels, label.clone());
            if label_area.is_none() {
                push_unique(&mut area.aliases, label);
            }
        }
    }
    push_unique(&mut area.aliases, area.slug.clone());
    push_unique(&mut area.aliases, area.title.to_ascii_lowercase());
    let confidence = inferred_area_confidence(area, issue_plan);
    area.confidence = area.confidence.max(confidence);
    if area.confidence >= confidence_threshold {
        area.status = AreaStatus::Stable;
    }
}

fn inferred_area_confidence(area: &AreaConfig, issue_plan: &CaptureIssuePlan) -> u8 {
    let base: u8 = if normalize_list(issue_plan.issue.labels.clone())
        .iter()
        .any(|label| {
            canonical_area_label_slug(label).as_deref() == Some(area.slug.as_str())
                || (slugify(label) == area.slug && !is_generic_label(label))
        })
    {
        90
    } else if issue_plan
        .issue
        .milestone
        .as_deref()
        .is_some_and(|milestone| slugify(milestone) == area.slug)
    {
        65
    } else if area.slug == "general" {
        40
    } else {
        60
    };
    let reinforcement = u8::from(!issue_plan.prs.is_empty()).saturating_mul(10)
        + u8::from(issue_plan.issue.milestone.is_some()).saturating_mul(5);
    base.saturating_add(reinforcement).min(95)
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if normalize_optional(&value).is_none() {
        return;
    }
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
        values.sort();
    }
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
        milestone_id: plan.issue.milestone_id.clone(),
        linear_url: plan.issue.url.clone(),
        parent: plan.issue.parent.as_ref().map(CapsuleIssueLink::from),
        children: plan
            .issue
            .children
            .iter()
            .map(CapsuleIssueLink::from)
            .collect(),
        blocked_by: plan
            .issue
            .blocked_by
            .iter()
            .map(CapsuleIssueLink::from)
            .collect(),
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
            linear_parent: plan
                .issue
                .parent
                .as_ref()
                .map(|parent| format!("linear:{}", normalize_issue_key(&parent.identifier))),
            linear_children: plan
                .issue
                .children
                .iter()
                .map(|child| format!("linear:{}", normalize_issue_key(&child.identifier)))
                .collect(),
            linear_blockers: plan
                .issue
                .blocked_by
                .iter()
                .map(|blocker| format!("linear:{}", normalize_issue_key(&blocker.identifier)))
                .collect(),
            linear_milestone: plan
                .issue
                .milestone_id
                .as_ref()
                .map(|milestone_id| format!("linear:project-milestone:{milestone_id}")),
            linear_workpad_comment: plan
                .issue
                .comments
                .iter()
                .find(|comment| comment.source.as_deref() == Some("linear:workpad"))
                .and_then(|comment| comment.id.as_ref())
                .map(|id| format!("linear:comment:{id}")),
            github_prs: plan
                .prs
                .iter()
                .map(|pr| format!("github:pr:{}", pr.number))
                .collect(),
            github_merge_shas: plan
                .prs
                .iter()
                .filter_map(|pr| pr.merge_sha.clone())
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
    if let Some(relationships) = render_relationships(&plan.issue) {
        markdown.push_str("\n\n## Relationships\n\n");
        markdown.push_str(&relationships);
    }
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
    markdown.push_str("\n## Source refs\n\n");
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
        if let Some(sha) = pr.merge_sha.as_deref() {
            markdown.push_str(&format!("- Merge SHA: `{sha}`\n"));
        }
    }
    if let Some(milestone) = plan.issue.milestone.as_deref() {
        markdown.push_str(&format!(
            "- Milestone: {}\n",
            milestone_link(markdown_slug(milestone), milestone)
        ));
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
    milestone_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<CapsuleIssueLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<CapsuleIssueLink>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    blocked_by: Vec<CapsuleIssueLink>,
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
struct CapsuleIssueLink {
    issue: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
}

impl From<&IssueLinkEvidence> for CapsuleIssueLink {
    fn from(link: &IssueLinkEvidence) -> Self {
        Self {
            issue: normalize_issue_key(&link.identifier),
            title: link.title.clone(),
            url: link.url.clone(),
            state: link.state.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct SourceRefs {
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_issue: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_parent: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    linear_children: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    linear_blockers: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_milestone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linear_workpad_comment: Option<String>,
    github_prs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    github_merge_shas: Vec<String>,
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

fn render_relationships(issue: &IssueEvidence) -> Option<String> {
    let mut lines = Vec::new();
    if let Some(parent) = &issue.parent {
        lines.push(format!("- Parent: {}", issue_link(parent)));
    }
    if !issue.children.is_empty() {
        lines.push("- Children:".to_string());
        for child in &issue.children {
            lines.push(format!("  - {}", issue_link(child)));
        }
    }
    if !issue.blocked_by.is_empty() {
        lines.push("- Blocked by:".to_string());
        for blocker in &issue.blocked_by {
            lines.push(format!("  - {}", issue_link(blocker)));
        }
    }
    if let Some(milestone) = issue.milestone.as_deref().and_then(normalize_optional) {
        lines.push(format!(
            "- Milestone: {}",
            milestone_link(markdown_slug(&milestone), &milestone)
        ));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn issue_link(issue: &IssueLinkEvidence) -> String {
    let issue_key = normalize_issue_key(&issue.identifier);
    let label = issue
        .title
        .as_deref()
        .and_then(normalize_optional)
        .map(|title| format!("{issue_key}: {title}"))
        .unwrap_or_else(|| issue_key.clone());
    format!("[[{issue_key}|{label}]]")
}

fn milestone_link(slug: String, label: &str) -> String {
    format!("[[milestones/{slug}|{label}]]")
}

fn markdown_slug(value: &str) -> String {
    slugify(value)
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
        let mut seen = BTreeSet::new();
        let mut emitted = 0;
        for review in &pr.reviews {
            let Some(entry) = review_signal(review) else {
                continue;
            };
            if !seen.insert(entry.clone()) {
                continue;
            }
            lines.push(format!("- PR #{} {entry}", pr.number));
            emitted += 1;
            if emitted >= 4 {
                break;
            }
        }
    }
    if lines.is_empty() {
        lines.push("- No high-signal review or rework notes were found.".to_string());
    }
    lines.join("\n")
}

fn review_signal(review: &ReviewEvidence) -> Option<String> {
    let reviewer = review.reviewer.as_deref().unwrap_or("reviewer");
    let state = review.state.as_deref().unwrap_or("reviewed");
    let state_upper = state.trim().to_ascii_uppercase();
    let summary = review.disposition.as_deref().and_then(review_signal_summary);

    if summary.is_none() && state_upper == "COMMENTED" {
        return None;
    }

    let mut entry = format!("{reviewer} {state}");
    if let Some(summary) = summary {
        entry.push_str(": ");
        entry.push_str(&summary);
    }
    Some(entry)
}

fn review_signal_summary(body: &str) -> Option<String> {
    let lines = meaningful_review_lines(body);
    if lines.is_empty() {
        None
    } else {
        Some(summarize_text(&lines.join(" "), 180))
    }
}

fn meaningful_review_lines(body: &str) -> Vec<String> {
    let mut priority = Vec::new();
    let mut fallback = Vec::new();
    for raw in body.lines() {
        let Some(line) = clean_review_line(raw) else {
            continue;
        };
        let lower = line.to_ascii_lowercase();
        if is_review_boilerplate(&lower) {
            continue;
        }
        if is_priority_review_line(&lower) {
            priority.push(line);
            if priority.len() >= 3 {
                break;
            }
        } else if fallback.len() < 2 {
            fallback.push(line);
        }
    }
    if priority.is_empty() {
        fallback
    } else {
        priority
    }
}

fn clean_review_line(raw: &str) -> Option<String> {
    let mut line = raw
        .trim()
        .trim_start_matches('>')
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_start_matches("- ")
        .trim()
        .replace("**", "")
        .replace('`', "");
    if line.contains("Badge]") && line.contains("</sub>")
        && let Some(index) = line.rfind("</sub>")
    {
        line = line[index + "</sub>".len()..].trim().to_string();
    }
    let line = line.trim().trim_start_matches(|ch: char| !ch.is_ascii()).trim();
    if line.is_empty() || line == "---" || line == "```" {
        None
    } else {
        Some(line.to_string())
    }
}

fn is_review_boilerplate(lower: &str) -> bool {
    lower.contains("codex review")
        || lower.starts_with("here are some automated review suggestions")
        || lower.starts_with("reviewed commit:")
        || lower.starts_with("<details")
        || lower.starts_with("<summary")
        || lower.starts_with("</")
        || lower.starts_with("<br")
        || lower.starts_with("https://github.com/")
        || lower.contains("about codex in github")
        || lower.starts_with("[your team has set up codex")
        || lower.starts_with("reviews are triggered")
        || lower.starts_with("if codex has suggestions")
        || lower.starts_with("codex can also answer")
        || lower.starts_with("try commenting")
        || lower.starts_with("open a pull request")
        || lower.starts_with("mark a draft")
        || lower.starts_with("comment \"@codex")
        || lower.starts_with("improve this review?")
        || lower.starts_with("resolve with ai?")
}

fn is_priority_review_line(lower: &str) -> bool {
    lower.contains("taste rating")
        || lower.contains("good taste")
        || lower.contains("needs rework")
        || lower.contains("worth merging")
        || lower.contains("no new issues")
        || lower.contains("all previously flagged")
        || lower.contains("previously flagged")
        || lower.contains("unresolved")
        || lower.contains("critical")
        || lower.contains("important")
        || lower.contains("verdict")
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
