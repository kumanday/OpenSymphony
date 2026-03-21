use async_trait::async_trait;
use chrono::Utc;
use opensymphony_domain::{
    Issue, IssueStateSnapshot, IssueTracker, RetryReason, TrackerError, WorkerOutcome,
    WorkerOutcomeKind,
};
use opensymphony_linear::{LinearError, LinearWriteOperations};
use opensymphony_openhands::{
    IssueRunRequest, IssueRunResult, IssueSessionError, IssueSessionRunner,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, Default)]
pub struct MemoryTracker {
    inner: Arc<Mutex<MemoryTrackerState>>,
}

#[derive(Debug, Default)]
struct MemoryTrackerState {
    issues: BTreeMap<String, Issue>,
    comments: HashMap<String, Vec<String>>,
    links: HashMap<String, Vec<String>>,
    active_states: Vec<String>,
    terminal_states: Vec<String>,
    project_states: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureStore {
    pub issues: Vec<Issue>,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
    pub project_states: Vec<String>,
}

impl MemoryTracker {
    pub fn new(
        issues: Vec<Issue>,
        active_states: Vec<String>,
        terminal_states: Vec<String>,
        project_states: Vec<String>,
    ) -> Self {
        let mut issue_map = BTreeMap::new();
        for issue in issues {
            issue_map.insert(issue.id.clone(), issue);
        }

        Self {
            inner: Arc::new(Mutex::new(MemoryTrackerState {
                issues: issue_map,
                comments: HashMap::new(),
                links: HashMap::new(),
                active_states,
                terminal_states,
                project_states,
            })),
        }
    }

    pub fn from_fixture_path(path: &Path) -> Result<Self, LinearError> {
        let fixture = fs::read_to_string(path)
            .map_err(|error| LinearError::InvalidResponse(error.to_string()))?;
        let store = serde_json::from_str::<FixtureStore>(&fixture)
            .map_err(|error| LinearError::InvalidResponse(error.to_string()))?;
        Ok(Self::new(
            store.issues,
            store.active_states,
            store.terminal_states,
            store.project_states,
        ))
    }

    pub fn comments_for(&self, issue_id: &str) -> Vec<String> {
        let state = self.inner.lock().expect("lock should succeed");
        state.comments.get(issue_id).cloned().unwrap_or_default()
    }

    pub fn links_for(&self, issue_id: &str) -> Vec<String> {
        let state = self.inner.lock().expect("lock should succeed");
        state.links.get(issue_id).cloned().unwrap_or_default()
    }

    fn snapshot_for(state: &MemoryTrackerState, issue: &Issue) -> IssueStateSnapshot {
        IssueStateSnapshot {
            id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            state: issue.state.clone(),
            is_active: state.active_states.contains(&issue.state),
            is_terminal: state.terminal_states.contains(&issue.state),
        }
    }
}

#[async_trait]
impl IssueTracker for MemoryTracker {
    async fn fetch_candidate_issues(&self) -> Result<Vec<Issue>, TrackerError> {
        let state = self.inner.lock().expect("lock should succeed");
        Ok(state
            .issues
            .values()
            .filter(|issue| state.active_states.contains(&issue.state))
            .cloned()
            .collect())
    }

    async fn fetch_states_by_issue_ids(
        &self,
        issue_ids: &[String],
    ) -> Result<Vec<IssueStateSnapshot>, TrackerError> {
        let state = self.inner.lock().expect("lock should succeed");
        Ok(issue_ids
            .iter()
            .filter_map(|issue_id| {
                state
                    .issues
                    .get(issue_id)
                    .map(|issue| Self::snapshot_for(&state, issue))
            })
            .collect())
    }

    async fn fetch_terminal_issues(&self) -> Result<Vec<Issue>, TrackerError> {
        let state = self.inner.lock().expect("lock should succeed");
        Ok(state
            .issues
            .values()
            .filter(|issue| state.terminal_states.contains(&issue.state))
            .cloned()
            .collect())
    }
}

impl LinearWriteOperations for MemoryTracker {
    fn get_issue(&self, query: &str) -> Result<Issue, LinearError> {
        let state = self.inner.lock().expect("lock should succeed");
        state
            .issues
            .values()
            .find(|issue| issue.id == query || issue.identifier == query)
            .cloned()
            .ok_or_else(|| LinearError::NotFound(query.to_string()))
    }

    fn comment_issue(&self, issue_id: &str, body: &str) -> Result<Issue, LinearError> {
        let mut state = self.inner.lock().expect("lock should succeed");
        let issue = state
            .issues
            .get(issue_id)
            .cloned()
            .ok_or_else(|| LinearError::NotFound(issue_id.to_string()))?;
        state
            .comments
            .entry(issue_id.to_string())
            .or_default()
            .push(body.to_string());
        Ok(issue)
    }

    fn transition_issue(&self, issue_id: &str, state_name: &str) -> Result<Issue, LinearError> {
        let mut state = self.inner.lock().expect("lock should succeed");
        if !state.project_states.iter().any(|state| state == state_name) {
            return Err(LinearError::InvalidStateTransition(state_name.to_string()));
        }

        let issue = state
            .issues
            .get_mut(issue_id)
            .ok_or_else(|| LinearError::NotFound(issue_id.to_string()))?;
        issue.state = state_name.to_string();
        issue.updated_at = Utc::now();
        Ok(issue.clone())
    }

    fn link_pr(
        &self,
        issue_id: &str,
        url: &str,
        title: Option<&str>,
    ) -> Result<Issue, LinearError> {
        let mut state = self.inner.lock().expect("lock should succeed");
        let issue = state
            .issues
            .get(issue_id)
            .cloned()
            .ok_or_else(|| LinearError::NotFound(issue_id.to_string()))?;
        let link = match title {
            Some(title) => format!("{title}: {url}"),
            None => url.to_string(),
        };
        state
            .links
            .entry(issue_id.to_string())
            .or_default()
            .push(link);
        Ok(issue)
    }

    fn list_project_states(&self, _project_slug: &str) -> Result<Vec<String>, LinearError> {
        let state = self.inner.lock().expect("lock should succeed");
        Ok(state.project_states.clone())
    }
}

#[derive(Debug, Clone)]
pub struct ScriptedRun {
    pub delay_ms: u64,
    pub outcome_kind: WorkerOutcomeKind,
    pub detail: Option<String>,
    pub conversation_id: Option<String>,
}

impl ScriptedRun {
    pub fn success(delay_ms: u64) -> Self {
        Self {
            delay_ms,
            outcome_kind: WorkerOutcomeKind::Success,
            detail: None,
            conversation_id: None,
        }
    }

    pub fn failure(delay_ms: u64, detail: impl Into<String>) -> Self {
        Self {
            delay_ms,
            outcome_kind: WorkerOutcomeKind::Failure,
            detail: Some(detail.into()),
            conversation_id: None,
        }
    }

    pub fn stalled(delay_ms: u64, detail: impl Into<String>) -> Self {
        Self {
            delay_ms,
            outcome_kind: WorkerOutcomeKind::Stalled,
            detail: Some(detail.into()),
            conversation_id: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScriptedRunner {
    plans: Arc<Mutex<HashMap<String, VecDeque<ScriptedRun>>>>,
    requests: Arc<Mutex<Vec<IssueRunRequest>>>,
}

impl ScriptedRunner {
    pub fn set_plan(&self, issue_id: &str, runs: Vec<ScriptedRun>) {
        let mut plans = self.plans.lock().expect("lock should succeed");
        plans.insert(issue_id.to_string(), runs.into());
    }

    pub fn requests(&self) -> Vec<IssueRunRequest> {
        self.requests.lock().expect("lock should succeed").clone()
    }
}

#[async_trait]
impl IssueSessionRunner for ScriptedRunner {
    async fn run_issue(
        &self,
        request: IssueRunRequest,
    ) -> Result<IssueRunResult, IssueSessionError> {
        self.requests
            .lock()
            .expect("lock should succeed")
            .push(request.clone());

        let scripted = {
            let mut plans = self.plans.lock().expect("lock should succeed");
            plans
                .entry(request.issue.id.clone())
                .or_default()
                .pop_front()
                .unwrap_or_else(|| ScriptedRun::success(0))
        };

        sleep(Duration::from_millis(scripted.delay_ms)).await;

        let outcome = match scripted.outcome_kind {
            WorkerOutcomeKind::Success => WorkerOutcome::success(),
            WorkerOutcomeKind::Failure => WorkerOutcome::failure(
                scripted
                    .detail
                    .unwrap_or_else(|| "scripted failure".to_string()),
            ),
            WorkerOutcomeKind::Cancelled => WorkerOutcome::cancelled(
                scripted
                    .detail
                    .unwrap_or_else(|| "scripted cancel".to_string()),
            ),
            WorkerOutcomeKind::Stalled => WorkerOutcome::stalled(
                scripted
                    .detail
                    .unwrap_or_else(|| "scripted stall".to_string()),
            ),
            WorkerOutcomeKind::Released => WorkerOutcome::released(
                scripted
                    .detail
                    .unwrap_or_else(|| "scripted release".to_string()),
            ),
        };

        Ok(IssueRunResult {
            outcome,
            conversation_id: scripted
                .conversation_id
                .unwrap_or_else(|| format!("conversation-{}", request.issue.identifier)),
            prompt_mode: request.prompt_mode,
        })
    }
}

pub fn make_issue(
    id: &str,
    identifier: &str,
    state: &str,
    priority: Option<u8>,
    created_at: chrono::DateTime<chrono::Utc>,
) -> Issue {
    Issue {
        id: id.to_string(),
        identifier: identifier.to_string(),
        title: format!("Issue {identifier}"),
        description: Some("Fixture issue".to_string()),
        priority,
        state: state.to_string(),
        labels: vec!["fixture".to_string()],
        blocked_by: vec![],
        created_at,
        updated_at: created_at,
    }
}

pub fn retry_reason_label(reason: RetryReason) -> &'static str {
    match reason {
        RetryReason::Continuation => "continuation",
        RetryReason::Failure => "failure",
        RetryReason::Stall => "stall",
        RetryReason::Recovery => "recovery",
        RetryReason::Reconcile => "reconcile",
    }
}
