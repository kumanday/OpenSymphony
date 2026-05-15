fn discover_github_prs(
    repo_root: &Path,
    issue_key: &str,
) -> Result<Vec<PullRequestEvidence>, String> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--search",
            issue_key,
            "--json",
            "number,title,url,headRefName,mergedAt,body,mergeCommit",
        ])
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                "GitHub PR discovery failed: gh CLI was not found in PATH; install GitHub CLI or rerun with --no-github".to_string()
            } else {
                format!("GitHub PR discovery failed: failed to run gh: {error}")
            }
        })?;
    if !output.status.success() {
        return Err(format!(
            "GitHub PR discovery failed: gh exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let values =
        serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).map_err(|error| {
            format!("GitHub PR discovery failed: failed to parse gh JSON: {error}")
        })?;
    let mut prs = Vec::new();
    for value in values {
        let Some(number) = value.get("number").and_then(serde_json::Value::as_u64) else {
            continue;
        };
        if !contains_issue_key(
            value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            issue_key,
        ) && !contains_issue_key(
            value
                .get("body")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            issue_key,
        ) && !contains_issue_key(
            value
                .get("headRefName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            issue_key,
        ) {
            continue;
        }
        let merge_sha = value
            .get("mergeCommit")
            .and_then(|commit| commit.get("oid"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let merged_at = value
            .get("mergedAt")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        let mut pr = PullRequestEvidence {
            number,
            title: value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            url: value
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            branch: value
                .get("headRefName")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            body: value
                .get("body")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            merge_sha,
            merged_at,
            ..PullRequestEvidence::default()
        };
        enrich_pr_from_gh(repo_root, &mut pr);
        prs.push(pr);
    }
    Ok(prs)
}

fn enrich_pr_from_gh(repo_root: &Path, pr: &mut PullRequestEvidence) {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr.number.to_string(),
            "--json",
            "files,commits,reviews,statusCheckRollup,mergeCommit",
        ])
        .current_dir(repo_root)
        .output();
    let Ok(output) = output else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return;
    };
    if pr.merge_sha.is_none() {
        pr.merge_sha = value
            .get("mergeCommit")
            .and_then(|commit| commit.get("oid"))
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
    }
    pr.changed_files = value
        .get("files")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            file.get("path")
                .and_then(serde_json::Value::as_str)
                .map(|path| ChangedFileEvidence {
                    path: PathBuf::from(path),
                    change_kind: file
                        .get("changeType")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                })
        })
        .collect();
    pr.commits = value
        .get("commits")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|commit| {
            let sha = commit
                .get("oid")
                .or_else(|| commit.get("sha"))
                .and_then(serde_json::Value::as_str)?;
            Some(CommitEvidence {
                sha: sha.to_string(),
                summary: commit
                    .get("messageHeadline")
                    .or_else(|| commit.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                author: None,
                timestamp: None,
            })
        })
        .collect();
    pr.reviews = value
        .get("reviews")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .map(|review| ReviewEvidence {
            reviewer: review
                .get("author")
                .and_then(|author| author.get("login"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            state: review
                .get("state")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            submitted_at: review
                .get("submittedAt")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc)),
            disposition: review
                .get("body")
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_optional),
        })
        .collect();
    pr.checks = value
        .get("statusCheckRollup")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|check| {
            let name = check
                .get("name")
                .or_else(|| check.get("context"))
                .and_then(serde_json::Value::as_str)?;
            Some(CheckEvidence {
                name: name.to_string(),
                conclusion: check
                    .get("conclusion")
                    .or_else(|| check.get("state"))
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                completed_at: check
                    .get("completedAt")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc)),
            })
        })
        .collect();
}
