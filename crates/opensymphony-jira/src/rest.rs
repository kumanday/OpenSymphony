//! Serde DTOs for the Jira Cloud REST API v3 payloads consumed by the client.

use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub(crate) struct SearchResponse {
    #[serde(default)]
    pub issues: Vec<JiraIssueBean>,
    #[serde(default, rename = "nextPageToken")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraIssueBean {
    pub id: String,
    pub key: String,
    pub fields: JiraIssueFields,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraIssueFields {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Value,
    pub status: JiraStatus,
    #[serde(default)]
    pub priority: Option<JiraPriority>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
    #[serde(default)]
    pub project: Option<JiraProject>,
    #[serde(default)]
    pub parent: Option<JiraIssueStub>,
    #[serde(default)]
    pub subtasks: Vec<JiraIssueStub>,
    #[serde(default, rename = "issuelinks")]
    pub issue_links: Vec<JiraIssueLink>,
    #[serde(default, rename = "fixVersions")]
    pub fix_versions: Vec<JiraVersion>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraStatus {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, rename = "statusCategory")]
    pub status_category: Option<JiraStatusCategory>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraStatusCategory {
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraPriority {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraProject {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraIssueStub {
    pub id: String,
    pub key: String,
    #[serde(default)]
    pub fields: Option<JiraStubFields>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraStubFields {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub status: Option<JiraStatus>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraIssueLink {
    #[serde(rename = "type")]
    pub link_type: JiraIssueLinkType,
    #[serde(default, rename = "inwardIssue")]
    pub inward_issue: Option<JiraIssueStub>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraIssueLinkType {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub inward: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraVersion {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CommentsResponse {
    #[serde(default)]
    pub comments: Vec<JiraComment>,
    #[serde(rename = "startAt")]
    pub start_at: usize,
    #[serde(default)]
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JiraComment {
    pub id: String,
    #[serde(default)]
    pub body: Value,
    pub updated: String,
}
