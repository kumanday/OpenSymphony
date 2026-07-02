use chrono::{DateTime, Utc};

use crate::opensymphony_domain::{
    TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind,
    TrackerIssueStateSnapshot, TrackerIssueSummary, TrackerProjectMilestone,
};

use super::adf::document_text;
use super::error::JiraError;
use super::rest::{JiraIssueBean, JiraIssueLink, JiraIssueStub, JiraPriority, JiraStatus};

pub(super) fn normalize_issue(
    bean: JiraIssueBean,
    site_base: &str,
) -> Result<TrackerIssue, JiraError> {
    let fields = bean.fields;
    let state = normalize_state(&fields.status);
    Ok(TrackerIssue {
        url: browse_url(site_base, &bean.key),
        id: bean.id,
        identifier: bean.key,
        title: fields.summary.unwrap_or_default(),
        description: document_text(&fields.description),
        priority: normalize_priority(fields.priority.as_ref()),
        state: state.name.clone(),
        state_kind: state.kind,
        branch_name: None,
        pr_url: None,
        labels: normalize_labels(fields.labels),
        project_id: fields.project.as_ref().map(|project| project.id.clone()),
        project_slug: fields.project.as_ref().map(|project| project.key.clone()),
        project_name: fields.project.as_ref().map(|project| project.name.clone()),
        parent_id: fields.parent.as_ref().map(|parent| parent.id.clone()),
        parent: fields
            .parent
            .map(|parent| issue_ref_from_stub(parent, site_base)),
        project_milestone: fields.fix_versions.into_iter().next().map(|version| {
            TrackerProjectMilestone {
                id: version.id,
                name: version.name,
            }
        }),
        blocked_by: normalize_blockers(fields.issue_links),
        sub_issues: normalize_sub_issues(fields.subtasks, site_base),
        created_at: parse_datetime("created", fields.created.as_deref())?,
        updated_at: parse_datetime("updated", fields.updated.as_deref())?,
    })
}

pub(super) fn normalize_issue_summary(
    bean: JiraIssueBean,
    site_base: &str,
) -> Result<TrackerIssueSummary, JiraError> {
    let fields = bean.fields;
    let state = normalize_state(&fields.status);
    Ok(TrackerIssueSummary {
        url: browse_url(site_base, &bean.key),
        id: bean.id,
        identifier: bean.key,
        title: fields.summary.unwrap_or_default(),
        priority: normalize_priority(fields.priority.as_ref()),
        state: state.name.clone(),
        state_kind: state.kind,
        blocked_by: normalize_blockers(fields.issue_links),
        sub_issues: normalize_sub_issues(fields.subtasks, site_base),
        created_at: parse_datetime("created", fields.created.as_deref())?,
        updated_at: parse_datetime("updated", fields.updated.as_deref())?,
    })
}

pub(super) fn normalize_issue_state(
    bean: JiraIssueBean,
) -> Result<TrackerIssueStateSnapshot, JiraError> {
    Ok(TrackerIssueStateSnapshot {
        state: normalize_state(&bean.fields.status),
        updated_at: parse_datetime("updated", bean.fields.updated.as_deref())?,
        id: bean.id,
        identifier: bean.key,
    })
}

fn normalize_state(status: &JiraStatus) -> TrackerIssueState {
    let category_key = status
        .status_category
        .as_ref()
        .map(|category| category.key.clone())
        .unwrap_or_else(|| "undefined".to_string());
    TrackerIssueState {
        id: status.id.clone().unwrap_or_default(),
        name: status.name.clone(),
        kind: TrackerIssueStateKind::from_tracker_type(&category_key),
        tracker_type: category_key,
    }
}

fn browse_url(site_base: &str, key: &str) -> String {
    format!("{}/browse/{key}", site_base.trim_end_matches('/'))
}

fn normalize_labels(mut labels: Vec<String>) -> Vec<String> {
    labels.sort_unstable();
    labels.dedup();
    labels
}

// Jira priorities are workspace-configurable, so unrecognized schemes degrade
// to no priority rather than failing the poll. Default schemes map onto the
// domain's 1 (urgent) .. 4 (low) scale.
fn normalize_priority(priority: Option<&JiraPriority>) -> Option<u8> {
    let priority = priority?;
    if let Some(name) = priority.name.as_deref() {
        match name.trim().to_ascii_lowercase().as_str() {
            "highest" | "blocker" => return Some(1),
            "high" | "critical" => return Some(2),
            "medium" | "major" => return Some(3),
            "low" | "minor" => return Some(4),
            "lowest" | "trivial" => return Some(4),
            _ => {}
        }
    }
    match priority.id.as_deref()?.trim().parse::<u8>().ok()? {
        0 => None,
        value @ 1..=4 => Some(value),
        _ => Some(4),
    }
}

fn normalize_blockers(links: Vec<JiraIssueLink>) -> Vec<TrackerIssueBlocker> {
    let mut blockers = links
        .into_iter()
        .filter(|link| {
            link.link_type
                .inward
                .as_deref()
                .is_some_and(|inward| inward.eq_ignore_ascii_case("is blocked by"))
                || link
                    .link_type
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("blocks"))
        })
        .filter_map(|link| link.inward_issue)
        .map(|stub| {
            let (title, state) = stub_fields(&stub);
            TrackerIssueBlocker {
                id: stub.id,
                identifier: stub.key,
                title,
                state,
            }
        })
        .collect::<Vec<_>>();
    blockers.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    blockers.dedup_by(|left, right| left.id == right.id);
    blockers
}

fn normalize_sub_issues(subtasks: Vec<JiraIssueStub>, site_base: &str) -> Vec<TrackerIssueRef> {
    let mut sub_issues = subtasks
        .into_iter()
        .map(|stub| issue_ref_from_stub(stub, site_base))
        .collect::<Vec<_>>();
    sub_issues.sort_by(|left, right| left.identifier.cmp(&right.identifier));
    sub_issues.dedup_by(|left, right| left.id == right.id);
    sub_issues
}

fn issue_ref_from_stub(stub: JiraIssueStub, site_base: &str) -> TrackerIssueRef {
    let url = browse_url(site_base, &stub.key);
    let state = stub
        .fields
        .as_ref()
        .and_then(|fields| fields.status.as_ref())
        .map(|status| status.name.clone())
        .unwrap_or_else(|| "unknown".to_string());
    TrackerIssueRef {
        title: stub.fields.and_then(|fields| fields.summary),
        id: stub.id,
        identifier: stub.key,
        url: Some(url),
        state,
    }
}

fn stub_fields(stub: &JiraIssueStub) -> (String, TrackerIssueState) {
    let title = stub
        .fields
        .as_ref()
        .and_then(|fields| fields.summary.clone())
        .unwrap_or_default();
    let state = stub
        .fields
        .as_ref()
        .and_then(|fields| fields.status.as_ref())
        .map(normalize_state)
        .unwrap_or_else(|| TrackerIssueState {
            id: String::new(),
            name: "unknown".to_string(),
            tracker_type: "undefined".to_string(),
            kind: TrackerIssueStateKind::from_tracker_type("undefined"),
        });
    (title, state)
}

pub(super) fn parse_datetime(field: &str, value: Option<&str>) -> Result<DateTime<Utc>, JiraError> {
    let value = value.ok_or_else(|| {
        JiraError::InvalidResponse(format!("Jira issue omitted the `{field}` timestamp"))
    })?;
    parse_jira_datetime(value).ok_or_else(|| {
        JiraError::InvalidResponse(format!(
            "Jira issue returned an unparseable `{field}` timestamp: {value}"
        ))
    })
}

pub(super) fn parse_jira_datetime(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Some(parsed.with_timezone(&Utc));
    }
    // Jira's default REST timestamp format: 2024-05-01T10:00:00.000+0000
    DateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f%z")
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use crate::opensymphony_domain::TrackerIssueStateKind;

    use super::super::rest::JiraPriority;
    use super::{normalize_priority, parse_jira_datetime};

    fn priority(id: Option<&str>, name: Option<&str>) -> JiraPriority {
        JiraPriority {
            id: id.map(str::to_string),
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn default_priority_names_map_to_domain_scale() {
        assert_eq!(
            normalize_priority(Some(&priority(None, Some("Highest")))),
            Some(1)
        );
        assert_eq!(
            normalize_priority(Some(&priority(None, Some("Medium")))),
            Some(3)
        );
        assert_eq!(
            normalize_priority(Some(&priority(None, Some("Lowest")))),
            Some(4)
        );
    }

    #[test]
    fn unknown_priority_names_fall_back_to_numeric_ids() {
        assert_eq!(
            normalize_priority(Some(&priority(Some("2"), Some("P2 - Pressing")))),
            Some(2)
        );
        assert_eq!(
            normalize_priority(Some(&priority(Some("9"), Some("Someday")))),
            Some(4)
        );
        assert_eq!(
            normalize_priority(Some(&priority(Some("nope"), Some("Someday")))),
            None
        );
        assert_eq!(normalize_priority(None), None);
    }

    #[test]
    fn jira_status_category_keys_map_to_state_kinds() {
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("new"),
            TrackerIssueStateKind::Unstarted
        );
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("indeterminate"),
            TrackerIssueStateKind::Started
        );
        assert_eq!(
            TrackerIssueStateKind::from_tracker_type("done"),
            TrackerIssueStateKind::Completed
        );
    }

    #[test]
    fn jira_timestamps_parse_with_and_without_colon_offsets() {
        assert!(parse_jira_datetime("2024-05-01T10:00:00.000+0000").is_some());
        assert!(parse_jira_datetime("2024-05-01T10:00:00+02:00").is_some());
        assert!(parse_jira_datetime("2024-05-01T10:00:00.123456Z").is_some());
        assert!(parse_jira_datetime("yesterday").is_none());
    }
}
