use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::opensymphony_domain::{TrackerIssue, TrackerIssueStateSnapshot};
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, RETRY_AFTER},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio::time::sleep;
use tracing::debug;

use super::error::{GraphqlError, LinearError, ResponseMetadata};
use super::graphql::{
    COMMENT_CREATE_MUTATION, CommentCreateData, CommentCreateInput, CommentCreateVariables,
    GraphqlEnvelope, GraphqlErrorPayload, ISSUE_ARCHIVE_MUTATION, ISSUE_BY_IDENTIFIER_QUERY,
    ISSUE_COMMENTS_QUERY, ISSUE_CREATE_MUTATION, ISSUE_INVERSE_RELATIONS_QUERY, ISSUE_LABELS_QUERY,
    ISSUE_RELATION_CREATE_MUTATION, ISSUE_STATES_BY_IDS_QUERY, ISSUE_UPDATE_MUTATION,
    ISSUES_BY_STATE_QUERY, IssueArchiveData, IssueArchiveVariables, IssueByIdentifierData,
    IssueByIdentifierVariables, IssueCommentsData, IssueCommentsVariables, IssueCreateData,
    IssueCreateInput, IssueCreateVariables, IssueInverseRelationsData,
    IssueInverseRelationsVariables, IssueLabelsData, IssueLabelsVariables, IssueRelationCreateData,
    IssueRelationCreateInput, IssueRelationCreateVariables, IssueRelationMutationNode,
    IssueStatesByIdsData, IssueStatesByIdsVariables, IssueUpdateData, IssueUpdateInput,
    IssueUpdateVariables, IssuesByStateData, IssuesByStateVariables, LinearIssueNode,
    LinearLabelConnection, LinearProjectNode, LinearRelationConnection, PROJECT_BY_SLUG_QUERY,
    PROJECT_ISSUES_QUERY, PROJECT_MILESTONE_CREATE_MUTATION, PROJECT_MILESTONE_UPDATE_MUTATION,
    PROJECT_UPDATE_CONTENT_MUTATION, ProjectBySlugData, ProjectBySlugVariables, ProjectIssuesData,
    ProjectIssuesVariables, ProjectMilestoneCreateData, ProjectMilestoneCreateInput,
    ProjectMilestoneCreateVariables, ProjectMilestoneUpdateData, ProjectMilestoneUpdateInput,
    ProjectMilestoneUpdateVariables, ProjectUpdateContentData, ProjectUpdateContentVariables,
};
use super::normalize::{normalize_issue, normalize_issue_state};

const DEFAULT_BASE_URL: &str = "https://api.linear.app/graphql";
const DEFAULT_PAGE_SIZE: usize = 50;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_INITIAL_RELATION_PAGE_SIZE: usize = 10;
const MAX_INITIAL_LABEL_PAGE_SIZE: usize = 10;

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(2),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LinearConfig {
    pub api_key: String,
    pub base_url: String,
    pub project_slug: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub page_size: usize,
    pub request_timeout: Duration,
    pub retry_policy: RetryPolicy,
}

impl LinearConfig {
    pub fn new(api_key: impl Into<String>, project_slug: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            project_slug: project_slug.into(),
            active_states: Vec::new(),
            terminal_states: Vec::new(),
            page_size: DEFAULT_PAGE_SIZE,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            retry_policy: RetryPolicy::default(),
        }
    }
}

#[derive(Clone)]
pub struct LinearClient {
    http: Client,
    config: LinearConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkpadComment {
    pub id: String,
    pub body: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearProjectOverview {
    pub id: String,
    pub name: String,
    pub slug_id: String,
    pub url: String,
    pub content: Option<String>,
}

// =============================================================================
// Mutation response DTOs (Linear-native shapes).
//
// These are returned by the public mutation methods and are wrapper-shaped
// because the schema uses camelCase and we expose them as snake_case Rust DTOs.
// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct LinearMilestoneMutationResult {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub target_date: Option<String>,
    pub sort_order: Option<f64>,
    pub project_id: String,
    pub project_slug_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearIssueMutationResult {
    pub id: String,
    pub identifier: String,
    pub url: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<f64>,
    pub estimate: Option<f64>,
    pub state_id: String,
    pub state_name: String,
    pub state_kind: String,
    pub project_id: Option<String>,
    pub project_slug_id: Option<String>,
    pub project_milestone_id: Option<String>,
    pub project_milestone_name: Option<String>,
    pub parent_id: Option<String>,
    pub parent_identifier: Option<String>,
    pub assignee_id: Option<String>,
    pub assignee_name: Option<String>,
    pub assignee_email: Option<String>,
    pub label_names: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<super::graphql::IssueMutationNode> for LinearIssueMutationResult {
    fn from(node: super::graphql::IssueMutationNode) -> Self {
        Self {
            id: node.id,
            identifier: node.identifier,
            url: node.url,
            title: node.title,
            description: node.description,
            priority: node.priority,
            estimate: node.estimate,
            state_id: node.state.id,
            state_name: node.state.name,
            state_kind: node.state.kind,
            project_id: node.project.as_ref().map(|p| p.id.clone()),
            project_slug_id: node.project.as_ref().map(|p| p.slug_id.clone()),
            project_milestone_id: node.project_milestone.as_ref().map(|m| m.id.clone()),
            project_milestone_name: node.project_milestone.as_ref().map(|m| m.name.clone()),
            parent_id: node.parent.as_ref().map(|p| p.id.clone()),
            parent_identifier: node.parent.as_ref().map(|p| p.identifier.clone()),
            assignee_id: node.assignee.as_ref().map(|a| a.id.clone()),
            assignee_name: node.assignee.as_ref().map(|a| a.name.clone()),
            assignee_email: node.assignee.as_ref().and_then(|a| a.email.clone()),
            label_names: node.labels.nodes.iter().map(|l| l.name.clone()).collect(),
            created_at: node.created_at,
            updated_at: node.updated_at,
        }
    }
}

impl From<super::graphql::ProjectMilestoneMutationNode> for LinearMilestoneMutationResult {
    fn from(node: super::graphql::ProjectMilestoneMutationNode) -> Self {
        Self {
            id: node.id,
            name: node.name,
            description: node.description,
            target_date: node.target_date,
            sort_order: node.sort_order,
            project_id: node.project.id.clone(),
            project_slug_id: node.project.slug_id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearCommentMutationResult {
    pub id: String,
    pub body: String,
    pub url: Option<String>,
    pub issue_id: String,
    pub issue_identifier: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<super::graphql::CommentMutationNode> for LinearCommentMutationResult {
    fn from(node: super::graphql::CommentMutationNode) -> Self {
        Self {
            id: node.id,
            body: node.body,
            url: node.url,
            issue_id: node.issue.id.clone(),
            issue_identifier: node.issue.identifier.clone(),
            created_at: node.created_at,
            updated_at: node.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LinearIssueRelationMutationResult {
    pub id: String,
    pub relation_type: String,
    pub issue_id: String,
    pub issue_identifier: String,
    pub related_issue_id: String,
    pub related_issue_identifier: String,
}

impl From<IssueRelationMutationNode> for LinearIssueRelationMutationResult {
    fn from(node: IssueRelationMutationNode) -> Self {
        Self {
            id: node.id,
            relation_type: node.relation_type,
            issue_id: node.issue.id.clone(),
            issue_identifier: node.issue.identifier.clone(),
            related_issue_id: node.related_issue.id.clone(),
            related_issue_identifier: node.related_issue.identifier.clone(),
        }
    }
}

impl LinearClient {
    pub fn new(mut config: LinearConfig) -> Result<Self, LinearError> {
        if config.base_url.trim().is_empty() {
            config.base_url = DEFAULT_BASE_URL.to_string();
        }
        if config.page_size == 0 {
            config.page_size = DEFAULT_PAGE_SIZE;
        }
        if config.request_timeout.is_zero() {
            config.request_timeout = DEFAULT_REQUEST_TIMEOUT;
        }
        if config.retry_policy.max_attempts == 0 {
            config.retry_policy.max_attempts = 1;
        }
        if config.retry_policy.initial_backoff.is_zero() {
            config.retry_policy.initial_backoff = Duration::from_millis(1);
        }
        if config.retry_policy.max_backoff < config.retry_policy.initial_backoff {
            config.retry_policy.max_backoff = config.retry_policy.initial_backoff;
        }
        config.api_key = normalize_required_string("LINEAR_API_KEY", &config.api_key)?;
        config.project_slug =
            normalize_required_string("tracker.project_slug", &config.project_slug)?;
        config.active_states =
            normalize_required_state_names("tracker.active_states", &config.active_states)?;
        config.terminal_states =
            normalize_required_state_names("tracker.terminal_states", &config.terminal_states)?;

        let http = Client::builder()
            .timeout(config.request_timeout)
            .build()
            .map_err(|error| LinearError::InvalidConfiguration(error.to_string()))?;

        Ok(Self { http, config })
    }

    pub async fn candidate_issues(&self) -> Result<Vec<TrackerIssue>, LinearError> {
        self.issues_by_state_names(&self.config.active_states).await
    }

    pub async fn terminal_issues(&self) -> Result<Vec<TrackerIssue>, LinearError> {
        self.issues_by_state_names_with_archived(&self.config.terminal_states, true)
            .await
    }

    pub async fn issues_by_state_names<S>(
        &self,
        state_names: &[S],
    ) -> Result<Vec<TrackerIssue>, LinearError>
    where
        S: AsRef<str>,
    {
        self.issues_by_state_names_with_archived(state_names, false)
            .await
    }

    pub async fn issues_by_identifiers<S>(
        &self,
        identifiers: &[S],
    ) -> Result<Vec<TrackerIssue>, LinearError>
    where
        S: AsRef<str>,
    {
        let identifiers = normalize_strings(identifiers);
        if identifiers.is_empty() {
            return Ok(Vec::new());
        }

        let mut issues = Vec::new();
        let mut missing_issue_ids = Vec::new();

        for identifier in &identifiers {
            let variables = IssueByIdentifierVariables {
                identifier: identifier.clone(),
                relation_first: self.config.page_size.min(MAX_INITIAL_RELATION_PAGE_SIZE),
                label_first: self.config.page_size.min(MAX_INITIAL_LABEL_PAGE_SIZE),
            };
            let response: IssueByIdentifierData = self
                .execute_graphql(ISSUE_BY_IDENTIFIER_QUERY, json!(variables))
                .await?;
            let Some(issue) = response.issue else {
                missing_issue_ids.push(identifier.clone());
                continue;
            };
            let issue = normalize_issue(self.expand_issue(issue).await?)?;
            if issue.identifier.eq_ignore_ascii_case(identifier) {
                issues.push(issue);
            } else {
                return Err(LinearError::InvalidResponse(format!(
                    "Linear issue lookup for {identifier} returned {}",
                    issue.identifier
                )));
            }
        }

        if missing_issue_ids.is_empty() {
            Ok(issues)
        } else {
            Err(LinearError::MissingIssueIds {
                issue_ids: missing_issue_ids,
            })
        }
    }

    pub async fn project_issues_by_identifiers<S>(
        &self,
        identifiers: &[S],
    ) -> Result<Vec<TrackerIssue>, LinearError>
    where
        S: AsRef<str>,
    {
        let identifiers = normalize_strings(identifiers);
        if identifiers.is_empty() {
            return Ok(Vec::new());
        }

        let requested_keys = identifiers
            .iter()
            .map(|identifier| identifier.to_ascii_uppercase())
            .collect::<HashSet<_>>();
        let project_issues = self.project_issues(false).await?;
        let mut issues_by_identifier = HashMap::new();
        for issue in project_issues {
            let key = issue.identifier.to_ascii_uppercase();
            if requested_keys.contains(&key) {
                issues_by_identifier.insert(key, issue);
            }
        }

        let mut issues = Vec::new();
        let mut missing_issue_ids = Vec::new();
        for identifier in &identifiers {
            let key = identifier.to_ascii_uppercase();
            match issues_by_identifier.remove(&key) {
                Some(issue) => issues.push(issue),
                None => missing_issue_ids.push(identifier.clone()),
            }
        }

        if missing_issue_ids.is_empty() {
            Ok(issues)
        } else {
            Err(LinearError::MissingIssueIds {
                issue_ids: missing_issue_ids,
            })
        }
    }

    async fn project_issues(
        &self,
        include_archived: bool,
    ) -> Result<Vec<TrackerIssue>, LinearError> {
        let mut after = None;
        let mut issues = Vec::new();

        loop {
            let variables = ProjectIssuesVariables {
                project_slug: self.config.project_slug.clone(),
                include_archived,
                first: self.config.page_size,
                after: after.clone(),
                relation_first: self.config.page_size.min(MAX_INITIAL_RELATION_PAGE_SIZE),
                label_first: self.config.page_size.min(MAX_INITIAL_LABEL_PAGE_SIZE),
            };
            let response: ProjectIssuesData = self
                .execute_graphql(PROJECT_ISSUES_QUERY, json!(variables))
                .await?;

            let page_info = response.issues.page_info;
            for node in response.issues.nodes {
                issues.push(normalize_issue(self.expand_issue(node).await?)?);
            }

            if !page_info.has_next_page {
                return Ok(issues);
            }

            after = Some(page_info.end_cursor.ok_or_else(|| {
                LinearError::InvalidResponse(
                    "Linear project issues page indicated a next page without an end cursor"
                        .to_string(),
                )
            })?);
        }
    }

    async fn issues_by_state_names_with_archived<S>(
        &self,
        state_names: &[S],
        include_archived: bool,
    ) -> Result<Vec<TrackerIssue>, LinearError>
    where
        S: AsRef<str>,
    {
        let state_names = normalize_strings(state_names);
        if state_names.is_empty() {
            return Ok(Vec::new());
        }

        let mut after = None;
        let mut issues = Vec::new();

        loop {
            let variables = IssuesByStateVariables {
                project_slug: self.config.project_slug.clone(),
                state_names: state_names.clone(),
                include_archived,
                first: self.config.page_size,
                after: after.clone(),
                relation_first: self.config.page_size.min(MAX_INITIAL_RELATION_PAGE_SIZE),
                label_first: self.config.page_size.min(MAX_INITIAL_LABEL_PAGE_SIZE),
            };
            let response: IssuesByStateData = self
                .execute_graphql(ISSUES_BY_STATE_QUERY, json!(variables))
                .await?;

            let page_info = response.issues.page_info;
            for node in response.issues.nodes {
                issues.push(normalize_issue(self.expand_issue(node).await?)?);
            }

            if !page_info.has_next_page {
                return Ok(issues);
            }

            after = Some(page_info.end_cursor.ok_or_else(|| {
                LinearError::InvalidResponse(
                    "Linear issues page indicated a next page without an end cursor".to_string(),
                )
            })?);
        }
    }

    pub async fn issue_states_by_ids<S>(
        &self,
        issue_ids: &[S],
    ) -> Result<Vec<TrackerIssueStateSnapshot>, LinearError>
    where
        S: AsRef<str>,
    {
        let issue_ids = normalize_strings(issue_ids);
        if issue_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut after = None;
        let mut snapshots = Vec::new();

        loop {
            let variables = IssueStatesByIdsVariables {
                project_slug: self.config.project_slug.clone(),
                issue_ids: issue_ids.clone(),
                first: self.config.page_size,
                after: after.clone(),
            };
            let response: IssueStatesByIdsData = self
                .execute_graphql(ISSUE_STATES_BY_IDS_QUERY, json!(variables))
                .await?;

            let page_info = response.issues.page_info;
            for node in response.issues.nodes {
                snapshots.push(normalize_issue_state(node));
            }

            if !page_info.has_next_page {
                return Ok(snapshots);
            }

            after = Some(page_info.end_cursor.ok_or_else(|| {
                LinearError::InvalidResponse(
                    "Linear issue-state page indicated a next page without an end cursor"
                        .to_string(),
                )
            })?);
        }
    }

    pub async fn fetch_workpad_comment(
        &self,
        issue_id: &str,
    ) -> Result<Option<WorkpadComment>, LinearError> {
        let issue_id = normalize_required_string("issue_id", issue_id)?;
        let mut after = None;
        let mut latest = None;

        loop {
            let variables = IssueCommentsVariables {
                issue_id: issue_id.clone(),
                first: self.config.page_size,
                after: after.clone(),
            };
            let response: IssueCommentsData = self
                .execute_graphql(ISSUE_COMMENTS_QUERY, json!(variables))
                .await?;
            let issue = response.issue.ok_or_else(|| LinearError::MissingIssueIds {
                issue_ids: vec![issue_id.clone()],
            })?;
            if issue.id != issue_id {
                return Err(LinearError::InvalidResponse(format!(
                    "Linear comments page returned mismatched issue ID {} for {}",
                    issue.id, issue_id
                )));
            }

            for comment in issue.comments.nodes {
                if comment.resolved_at.is_some() || !contains_workpad_marker(&comment.body) {
                    continue;
                }

                let candidate = WorkpadComment {
                    id: comment.id,
                    body: comment.body,
                    updated_at: comment.updated_at,
                };
                if latest.as_ref().is_none_or(|existing: &WorkpadComment| {
                    candidate.updated_at > existing.updated_at
                }) {
                    latest = Some(candidate);
                }
            }

            if !issue.comments.page_info.has_next_page {
                return Ok(latest);
            }

            after = Some(issue.comments.page_info.end_cursor.ok_or_else(|| {
                LinearError::InvalidResponse(format!(
                    "Linear comments page for issue {issue_id} indicated a next page without an end cursor"
                ))
            })?);
        }
    }

    pub async fn archive_issue(&self, issue_id_or_identifier: &str) -> Result<(), LinearError> {
        let issue_id_or_identifier =
            normalize_required_string("issue_id_or_identifier", issue_id_or_identifier)?;
        let variables = IssueArchiveVariables {
            id: issue_id_or_identifier,
            trash: false,
        };
        let response: IssueArchiveData = self
            .execute_graphql(ISSUE_ARCHIVE_MUTATION, json!(variables))
            .await?;
        if response.issue_archive.success {
            Ok(())
        } else {
            Err(LinearError::InvalidResponse(
                "Linear issueArchive returned success=false".to_string(),
            ))
        }
    }

    pub async fn project_overview(&self) -> Result<Option<LinearProjectOverview>, LinearError> {
        let variables = ProjectBySlugVariables {
            slug: self.config.project_slug.clone(),
        };
        let response: ProjectBySlugData = self
            .execute_graphql(PROJECT_BY_SLUG_QUERY, json!(variables))
            .await?;
        Ok(response
            .projects
            .nodes
            .into_iter()
            .next()
            .map(LinearProjectOverview::from))
    }

    pub async fn update_project_content(
        &self,
        project_id: &str,
        content: &str,
    ) -> Result<(), LinearError> {
        let variables = ProjectUpdateContentVariables {
            id: normalize_required_string("project_id", project_id)?,
            content: content.to_string(),
        };
        let response: ProjectUpdateContentData = self
            .execute_graphql(PROJECT_UPDATE_CONTENT_MUTATION, json!(variables))
            .await?;
        if response.project_update.success {
            Ok(())
        } else {
            Err(LinearError::InvalidResponse(
                "Linear projectUpdate returned success=false".to_string(),
            ))
        }
    }

    /// Create a project milestone via Linear GraphQL `projectMilestoneCreate`.
    pub async fn create_project_milestone(
        &self,
        input: ProjectMilestoneCreateInput,
    ) -> Result<LinearMilestoneMutationResult, LinearError> {
        let variables = ProjectMilestoneCreateVariables { input };
        let response: ProjectMilestoneCreateData = self
            .execute_graphql(PROJECT_MILESTONE_CREATE_MUTATION, json!(variables))
            .await?;
        Self::milestone_mutation_result("projectMilestoneCreate", response.project_milestone_create)
    }

    /// Update a project milestone via Linear GraphQL `projectMilestoneUpdate`.
    pub async fn update_project_milestone(
        &self,
        milestone_id: &str,
        input: ProjectMilestoneUpdateInput,
    ) -> Result<LinearMilestoneMutationResult, LinearError> {
        let variables = ProjectMilestoneUpdateVariables {
            id: normalize_required_string("milestone_id", milestone_id)?,
            input,
        };
        let response: ProjectMilestoneUpdateData = self
            .execute_graphql(PROJECT_MILESTONE_UPDATE_MUTATION, json!(variables))
            .await?;
        Self::milestone_mutation_result("projectMilestoneUpdate", response.project_milestone_update)
    }

    fn milestone_mutation_result(
        operation: &'static str,
        payload: super::graphql::ProjectMilestoneMutationPayload,
    ) -> Result<LinearMilestoneMutationResult, LinearError> {
        if !payload.success {
            return Err(LinearError::InvalidResponse(format!(
                "Linear {operation} returned success=false",
            )));
        }
        payload.project_milestone.map_or_else(
            || {
                Err(LinearError::InvalidResponse(format!(
                    "Linear {operation} returned success=true without a projectMilestone"
                )))
            },
            |node| Ok(node.into()),
        )
    }

    /// Create an issue via Linear GraphQL `issueCreate`. Sub-issues are
    /// simply issues whose input has `parentId` set; callers pass the parent
    /// ID explicitly so this method serves both issue and sub-issue flows.
    pub async fn create_issue(
        &self,
        input: IssueCreateInput,
    ) -> Result<LinearIssueMutationResult, LinearError> {
        Self::validate_issue_create_input(&input)?;
        let variables = IssueCreateVariables { input };
        let response: IssueCreateData = self
            .execute_graphql(ISSUE_CREATE_MUTATION, json!(variables))
            .await?;
        Self::issue_mutation_result("issueCreate", response.issue_create)
    }

    /// Update an issue via Linear GraphQL `issueUpdate`.
    pub async fn update_issue(
        &self,
        issue_id: &str,
        input: IssueUpdateInput,
    ) -> Result<LinearIssueMutationResult, LinearError> {
        let variables = IssueUpdateVariables {
            id: normalize_required_string("issue_id", issue_id)?,
            input,
        };
        let response: IssueUpdateData = self
            .execute_graphql(ISSUE_UPDATE_MUTATION, json!(variables))
            .await?;
        Self::issue_mutation_result("issueUpdate", response.issue_update)
    }

    fn issue_mutation_result(
        operation: &'static str,
        payload: super::graphql::IssueMutationPayload,
    ) -> Result<LinearIssueMutationResult, LinearError> {
        if !payload.success {
            return Err(LinearError::InvalidResponse(format!(
                "Linear {operation} returned success=false",
            )));
        }
        payload.issue.map_or_else(
            || {
                Err(LinearError::InvalidResponse(format!(
                    "Linear {operation} returned success=true without an issue"
                )))
            },
            |node| Ok(node.into()),
        )
    }

    fn validate_issue_create_input(input: &IssueCreateInput) -> Result<(), LinearError> {
        let _ = normalize_required_string("issue.team_id", &input.team_id)?;
        let _ = normalize_required_string("issue.title", &input.title)?;
        Ok(())
    }

    /// Create a comment (evidence note) on an issue via Linear GraphQL
    /// `commentCreate`.
    pub async fn create_comment(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<LinearCommentMutationResult, LinearError> {
        let input = CommentCreateInput {
            issue_id: normalize_required_string("issue_id", issue_id)?,
            body: normalize_required_string("comment.body", body)?,
        };
        let variables = CommentCreateVariables { input };
        let response: CommentCreateData = self
            .execute_graphql(COMMENT_CREATE_MUTATION, json!(variables))
            .await?;
        if !response.comment_create.success {
            return Err(LinearError::InvalidResponse(
                "Linear commentCreate returned success=false".to_string(),
            ));
        }
        response.comment_create.comment.map_or_else(
            || {
                Err(LinearError::InvalidResponse(
                    "Linear commentCreate returned success=true without a comment".to_string(),
                ))
            },
            |node| Ok(node.into()),
        )
    }

    /// Create an issue relation (blocker, related, duplicate) via Linear
    /// GraphQL `issueRelationCreate`.
    pub async fn create_issue_relation(
        &self,
        issue_id: &str,
        related_issue_id: &str,
        relation_type: &str,
    ) -> Result<LinearIssueRelationMutationResult, LinearError> {
        let input = IssueRelationCreateInput {
            issue_id: normalize_required_string("issue_id", issue_id)?,
            related_issue_id: normalize_required_string("related_issue_id", related_issue_id)?,
            relation_type: normalize_required_string("relation_type", relation_type)?,
        };
        let variables = IssueRelationCreateVariables { input };
        let response: IssueRelationCreateData = self
            .execute_graphql(ISSUE_RELATION_CREATE_MUTATION, json!(variables))
            .await?;
        if !response.issue_relation_create.success {
            return Err(LinearError::InvalidResponse(
                "Linear issueRelationCreate returned success=false".to_string(),
            ));
        }
        response.issue_relation_create.issue_relation.map_or_else(
            || {
                Err(LinearError::InvalidResponse(
                    "Linear issueRelationCreate returned success=true without an issueRelation"
                        .to_string(),
                ))
            },
            |node| Ok(node.into()),
        )
    }

    async fn expand_issue(
        &self,
        mut issue: LinearIssueNode,
    ) -> Result<LinearIssueNode, LinearError> {
        issue.labels = self.load_all_labels(&issue.id, issue.labels).await?;
        issue.inverse_relations = self
            .load_all_inverse_relations(&issue.id, issue.inverse_relations)
            .await?;
        Ok(issue)
    }

    async fn load_all_labels(
        &self,
        issue_id: &str,
        mut connection: LinearLabelConnection,
    ) -> Result<LinearLabelConnection, LinearError> {
        let mut after = connection.page_info.end_cursor.clone();

        while connection.page_info.has_next_page {
            let cursor = after.clone().ok_or_else(|| {
                LinearError::InvalidResponse(format!(
                    "Linear labels page for issue {issue_id} indicated a next page without an end cursor"
                ))
            })?;
            let variables = IssueLabelsVariables {
                issue_id: issue_id.to_string(),
                first: self.config.page_size,
                after: Some(cursor),
            };
            let response: IssueLabelsData = self
                .execute_graphql(ISSUE_LABELS_QUERY, json!(variables))
                .await?;
            let issue = response.issue.ok_or_else(|| LinearError::MissingIssueIds {
                issue_ids: vec![issue_id.to_string()],
            })?;
            if issue.id != issue_id {
                return Err(LinearError::InvalidResponse(format!(
                    "Linear labels page returned mismatched issue ID {} for {}",
                    issue.id, issue_id
                )));
            }

            connection.nodes.extend(issue.labels.nodes);
            connection.page_info = issue.labels.page_info;
            after = connection.page_info.end_cursor.clone();
        }

        Ok(connection)
    }

    async fn load_all_inverse_relations(
        &self,
        issue_id: &str,
        mut connection: LinearRelationConnection,
    ) -> Result<LinearRelationConnection, LinearError> {
        let mut after = connection.page_info.end_cursor.clone();

        while connection.page_info.has_next_page {
            let cursor = after.clone().ok_or_else(|| {
                LinearError::InvalidResponse(format!(
                    "Linear inverseRelations page for issue {issue_id} indicated a next page without an end cursor"
                ))
            })?;
            let variables = IssueInverseRelationsVariables {
                issue_id: issue_id.to_string(),
                first: self.config.page_size,
                after: Some(cursor),
            };
            let response: IssueInverseRelationsData = self
                .execute_graphql(ISSUE_INVERSE_RELATIONS_QUERY, json!(variables))
                .await?;
            let issue = response.issue.ok_or_else(|| LinearError::MissingIssueIds {
                issue_ids: vec![issue_id.to_string()],
            })?;
            if issue.id != issue_id {
                return Err(LinearError::InvalidResponse(format!(
                    "Linear inverseRelations page returned mismatched issue ID {} for {}",
                    issue.id, issue_id
                )));
            }

            connection.nodes.extend(issue.inverse_relations.nodes);
            connection.page_info = issue.inverse_relations.page_info;
            after = connection.page_info.end_cursor.clone();
        }

        Ok(connection)
    }

    pub(super) async fn execute_graphql<T>(
        &self,
        query: &'static str,
        variables: Value,
    ) -> Result<T, LinearError>
    where
        T: DeserializeOwned,
    {
        let body = json!({
            "query": query,
            "variables": variables,
        });
        let authorization = self.config.api_key.as_str();
        let operation = graphql_operation_name(query);
        let mut attempt = 1;

        loop {
            let response = self
                .http
                .post(&self.config.base_url)
                .header(AUTHORIZATION, authorization)
                .header(CONTENT_TYPE, "application/json")
                .header(ACCEPT, "application/json")
                .json(&body)
                .send()
                .await;

            match response {
                Ok(response) => {
                    let status = response.status();
                    let retry_after = parse_retry_delay(response.headers());
                    let metadata = response_metadata(response.headers());
                    let payload = match response.text().await {
                        Ok(payload) => payload,
                        Err(error) => {
                            let error = LinearError::ResponseBody {
                                operation: operation.clone(),
                                status,
                                metadata: Box::new(metadata),
                                retry_after,
                                source: Box::new(error),
                            };
                            if self.should_retry(&error, attempt) {
                                self.sleep_before_retry(&error, attempt).await;
                                attempt += 1;
                                continue;
                            }
                            return Err(error);
                        }
                    };

                    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
                        let error = LinearError::HttpStatus {
                            status,
                            body: payload,
                            retry_after,
                        };
                        if self.should_retry(&error, attempt) {
                            self.sleep_before_retry(&error, attempt).await;
                            attempt += 1;
                            continue;
                        }
                        return Err(error);
                    }

                    if let Some(error) = decode_graphql_error_response(&payload, retry_after) {
                        if self.should_retry(&error, attempt) {
                            self.sleep_before_retry(&error, attempt).await;
                            attempt += 1;
                            continue;
                        }
                        return Err(error);
                    }

                    if !status.is_success() {
                        let error = LinearError::HttpStatus {
                            status,
                            body: payload,
                            retry_after,
                        };
                        if self.should_retry(&error, attempt) {
                            self.sleep_before_retry(&error, attempt).await;
                            attempt += 1;
                            continue;
                        }
                        return Err(error);
                    }

                    let envelope: GraphqlEnvelope<T> =
                        serde_json::from_str(&payload).map_err(|error| {
                            LinearError::InvalidResponse(format!(
                                "failed to decode Linear GraphQL response for {operation} after HTTP {status}: {error} ({metadata}, body_bytes={})",
                                payload.len()
                            ))
                        })?;

                    if let Some(errors) = envelope.errors {
                        let error = LinearError::from_graphql_errors_with_retry_after(
                            convert_graphql_errors(errors),
                            retry_after,
                        );
                        if self.should_retry(&error, attempt) {
                            self.sleep_before_retry(&error, attempt).await;
                            attempt += 1;
                            continue;
                        }
                        return Err(error);
                    }

                    return envelope.data.ok_or_else(|| {
                        LinearError::InvalidResponse(
                            format!(
                                "Linear GraphQL response for {operation} omitted both data and errors ({metadata}, body_bytes={})",
                                payload.len()
                            ),
                        )
                    });
                }
                Err(error) => {
                    let error = LinearError::Request(Box::new(error));
                    if self.should_retry(&error, attempt) {
                        self.sleep_before_retry(&error, attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(error);
                }
            }
        }
    }

    fn should_retry(&self, error: &LinearError, attempt: usize) -> bool {
        if attempt >= self.config.retry_policy.max_attempts {
            return false;
        }

        match error {
            LinearError::Request(_) => true,
            LinearError::ResponseBody { .. } => true,
            LinearError::HttpStatus { status, .. } => {
                *status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            LinearError::Graphql { .. } => error.is_rate_limited(),
            LinearError::MissingIssueIds { .. }
            | LinearError::InvalidConfiguration(_)
            | LinearError::InvalidResponse(_) => false,
        }
    }

    async fn sleep_before_retry(&self, error: &LinearError, attempt: usize) {
        let delay = error
            .retry_after()
            .unwrap_or_else(|| self.exponential_backoff(attempt));
        debug!(
            attempt,
            delay_ms = delay.as_millis(),
            category = ?error.category(),
            "retrying Linear GraphQL request"
        );
        sleep(delay).await;
    }

    fn exponential_backoff(&self, attempt: usize) -> Duration {
        let mut delay = self.config.retry_policy.initial_backoff;
        for _ in 1..attempt {
            match delay.checked_mul(2) {
                Some(next) if next <= self.config.retry_policy.max_backoff => delay = next,
                _ => return self.config.retry_policy.max_backoff,
            }
        }
        delay
    }
}

impl From<LinearProjectNode> for LinearProjectOverview {
    fn from(node: LinearProjectNode) -> Self {
        Self {
            id: node.id,
            name: node.name,
            slug_id: node.slug_id,
            url: node.url,
            content: node.content,
        }
    }
}

fn convert_graphql_errors(errors: Vec<GraphqlErrorPayload>) -> Vec<GraphqlError> {
    errors
        .into_iter()
        .map(|error| GraphqlError {
            message: error.message,
            code: error.extensions.and_then(|extensions| extensions.code),
        })
        .collect()
}

fn decode_graphql_error_response(
    payload: &str,
    retry_after: Option<Duration>,
) -> Option<LinearError> {
    let envelope: GraphqlEnvelope<Value> = match serde_json::from_str(payload) {
        Ok(envelope) => envelope,
        Err(_) => return None,
    };

    envelope.errors.map(|errors| {
        LinearError::from_graphql_errors_with_retry_after(
            convert_graphql_errors(errors),
            retry_after,
        )
    })
}

fn graphql_operation_name(query: &str) -> String {
    let mut tokens = query.split_whitespace();
    match tokens.next() {
        Some("query" | "mutation" | "subscription") => tokens
            .next()
            .map(|token| token.split(['(', '{']).next().unwrap_or(token).to_string())
            .filter(|token| !token.is_empty())
            .unwrap_or_else(|| "<anonymous>".to_string()),
        _ => "<anonymous>".to_string(),
    }
}

fn response_metadata(headers: &reqwest::header::HeaderMap) -> ResponseMetadata {
    ResponseMetadata {
        content_type: header_value(headers, CONTENT_TYPE),
        content_length: header_value(headers, CONTENT_LENGTH),
        content_encoding: header_value(headers, CONTENT_ENCODING),
    }
}

fn header_value(
    headers: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn normalize_strings<S>(values: &[S]) -> Vec<String>
where
    S: AsRef<str>,
{
    let mut normalized = Vec::new();
    for value in values {
        let value = value.as_ref().trim();
        if value.is_empty() {
            continue;
        }
        if !normalized.iter().any(|existing| existing == value) {
            normalized.push(value.to_string());
        }
    }
    normalized
}

fn normalize_required_state_names<S>(
    field_name: &str,
    values: &[S],
) -> Result<Vec<String>, LinearError>
where
    S: AsRef<str>,
{
    let normalized = normalize_strings(values);
    if normalized.is_empty() {
        Err(LinearError::InvalidConfiguration(format!(
            "workflow {field_name} must contain at least one non-empty state name"
        )))
    } else {
        Ok(normalized)
    }
}

fn normalize_required_string(field_name: &str, value: &str) -> Result<String, LinearError> {
    let normalized = value.trim();
    if normalized.is_empty() {
        Err(LinearError::InvalidConfiguration(format!(
            "{field_name} must be a non-empty string"
        )))
    } else {
        Ok(normalized.to_string())
    }
}

fn contains_workpad_marker(body: &str) -> bool {
    body.lines()
        .any(|line| line.trim_start().starts_with("## Agent Harness Workpad"))
}

fn parse_retry_after(header_value: Option<&reqwest::header::HeaderValue>) -> Option<Duration> {
    let seconds = header_value?.to_str().ok()?.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds))
}

fn parse_retry_delay(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    parse_rate_limit_reset(headers, SystemTime::now())
        .or_else(|| parse_retry_after(headers.get(RETRY_AFTER)))
}

fn parse_rate_limit_reset(
    headers: &reqwest::header::HeaderMap,
    now: SystemTime,
) -> Option<Duration> {
    const RESET_HEADERS: [&str; 3] = [
        "x-ratelimit-requests-reset",
        "x-ratelimit-endpoint-requests-reset",
        "x-ratelimit-complexity-reset",
    ];

    let now_ms = now.duration_since(UNIX_EPOCH).ok()?.as_millis();
    let latest_reset_ms = RESET_HEADERS
        .into_iter()
        .filter_map(|header_name| headers.get(header_name))
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.trim().parse::<u128>().ok())
        .max()?;
    let delay_ms = latest_reset_ms.saturating_sub(now_ms);
    let delay_ms = u64::try_from(delay_ms).unwrap_or(u64::MAX);

    Some(Duration::from_millis(delay_ms))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use reqwest::header::{HeaderMap, HeaderValue, RETRY_AFTER};

    use super::{parse_rate_limit_reset, parse_retry_delay};

    #[test]
    fn rate_limit_reset_headers_use_latest_reset_window() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-ratelimit-requests-reset",
            HeaderValue::from_static("1100"),
        );
        headers.insert(
            "x-ratelimit-endpoint-requests-reset",
            HeaderValue::from_static("1250"),
        );
        headers.insert(
            "x-ratelimit-complexity-reset",
            HeaderValue::from_static("1200"),
        );

        let delay = parse_rate_limit_reset(&headers, UNIX_EPOCH + Duration::from_millis(1_000));

        assert_eq!(delay, Some(Duration::from_millis(250)));
    }

    #[test]
    fn retry_delay_prefers_reset_headers_over_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("30"));
        headers.insert("x-ratelimit-requests-reset", HeaderValue::from_static("0"));

        let delay = parse_retry_delay(&headers);

        assert_eq!(delay, Some(Duration::ZERO));
    }
}
