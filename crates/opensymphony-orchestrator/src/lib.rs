use chrono::{DateTime, Utc};
use opensymphony_domain::{
    continuation_retry_at, failure_retry_delay, sort_candidates, AttemptContext, Issue,
    IssueTracker, OrchestratorSnapshot, RetryEntry, RetryReason, SchedulerConfig, WorkerOutcome,
    WorkerOutcomeKind,
};
use opensymphony_openhands::{
    IssueRunRequest, IssueRunResult, IssueSessionError, IssueSessionRunner, PromptMode,
};
use opensymphony_workflow::{AgentConfig, WorkflowDocument, WorkflowError};
use opensymphony_workspace::{IssueManifest, WorkspaceManager};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

#[derive(Debug, Error)]
pub enum SchedulerError {
    #[error("tracker error: {0}")]
    Tracker(String),
    #[error("workspace error: {0}")]
    Workspace(String),
    #[error("workflow error: {0}")]
    Workflow(String),
}

pub struct Scheduler {
    config: SchedulerConfig,
    tracker: Arc<dyn IssueTracker>,
    runner: Arc<dyn IssueSessionRunner>,
    workspace: Arc<WorkspaceManager>,
    running: HashMap<String, RunningWorker>,
    retry_queue: Vec<RetryEntry>,
    reports_tx: mpsc::UnboundedSender<WorkerReport>,
    reports_rx: mpsc::UnboundedReceiver<WorkerReport>,
}

struct RunningWorker {
    issue: Issue,
    attempt: u32,
    started_at: DateTime<Utc>,
    task: JoinHandle<()>,
}

struct WorkerReport {
    issue: Issue,
    attempt: u32,
    workspace_path: PathBuf,
    result: Result<IssueRunResult, IssueSessionError>,
}

struct RenderedPrompt {
    prompt: String,
    max_turns: u32,
}

impl Scheduler {
    pub fn new(
        config: SchedulerConfig,
        tracker: Arc<dyn IssueTracker>,
        runner: Arc<dyn IssueSessionRunner>,
        workspace: Arc<WorkspaceManager>,
    ) -> Self {
        let (reports_tx, reports_rx) = mpsc::unbounded_channel();
        Self {
            config,
            tracker,
            runner,
            workspace,
            running: HashMap::new(),
            retry_queue: vec![],
            reports_tx,
            reports_rx,
        }
    }

    pub async fn recover(&mut self) -> Result<usize, SchedulerError> {
        let manifests = self
            .workspace
            .list_issue_manifests()
            .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
        if manifests.is_empty() {
            return Ok(0);
        }

        let issue_ids = manifests
            .iter()
            .map(|manifest| manifest.issue_id.clone())
            .collect::<Vec<_>>();
        let states = self
            .tracker
            .fetch_states_by_issue_ids(&issue_ids)
            .await
            .map_err(|error| SchedulerError::Tracker(error.to_string()))?;
        let states_by_id = states
            .into_iter()
            .map(|state| (state.id.clone(), state))
            .collect::<HashMap<_, _>>();

        let mut recovered = 0;
        for manifest in manifests {
            let Some(state) = states_by_id.get(&manifest.issue_id) else {
                continue;
            };
            let issue = issue_from_manifest(&manifest, &state.state);

            if state.is_terminal {
                self.workspace
                    .cleanup_terminal_workspace(&issue)
                    .await
                    .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
                continue;
            }

            if let Some(retry) = self
                .workspace
                .load_retry_manifest(&manifest.identifier)
                .map_err(|error| SchedulerError::Workspace(error.to_string()))?
            {
                self.retry_queue.push(RetryEntry {
                    issue: issue.clone(),
                    attempt: retry.attempt,
                    reason: retry.reason,
                    scheduled_at: retry.scheduled_at,
                });
                recovered += 1;
                continue;
            }

            if state.is_active {
                self.enqueue_retry(
                    issue,
                    manifest.last_attempt.max(1) + 1,
                    RetryReason::Recovery,
                    Utc::now(),
                )?;
                recovered += 1;
            }
        }

        self.retry_queue.sort_by_key(|entry| entry.scheduled_at);
        Ok(recovered)
    }

    pub async fn tick(&mut self) -> Result<(), SchedulerError> {
        self.drain_reports().await?;
        self.reconcile_running().await?;

        if self.running.len() >= self.config.max_concurrency {
            return Ok(());
        }

        let now = Utc::now();
        let (mut due_retries, deferred_retries) =
            partition_due_retries(std::mem::take(&mut self.retry_queue), now);
        self.retry_queue = deferred_retries;
        let mut retry_queued_ids = self
            .retry_queue
            .iter()
            .map(|entry| entry.issue.id.clone())
            .collect::<HashSet<_>>();

        let mut candidates = self
            .tracker
            .fetch_candidate_issues()
            .await
            .map_err(|error| SchedulerError::Tracker(error.to_string()))?;
        sort_candidates(&mut candidates);
        let mut candidates_by_id = candidates
            .iter()
            .cloned()
            .map(|issue| (issue.id.clone(), issue))
            .collect::<HashMap<_, _>>();

        let mut dispatch_queue = Vec::new();
        let mut reserved_ids = HashSet::new();
        let available_slots = self
            .config
            .max_concurrency
            .saturating_sub(self.running.len());

        due_retries.sort_by_key(|entry| entry.scheduled_at);
        for retry in due_retries.drain(..) {
            if dispatch_queue.len() >= available_slots {
                retry_queued_ids.insert(retry.issue.id.clone());
                self.retry_queue.push(retry);
                continue;
            }
            if self.running.contains_key(&retry.issue.id) {
                retry_queued_ids.insert(retry.issue.id.clone());
                self.retry_queue.push(retry);
                continue;
            }
            let Some(current_issue) = candidates_by_id.remove(&retry.issue.id) else {
                continue;
            };
            if current_issue.is_blocked_by_active_issue() {
                continue;
            }
            reserved_ids.insert(current_issue.id.clone());
            dispatch_queue.push((current_issue, retry.attempt));
        }

        for issue in candidates {
            if dispatch_queue.len() >= available_slots {
                break;
            }
            if self.running.contains_key(&issue.id)
                || reserved_ids.contains(&issue.id)
                || retry_queued_ids.contains(&issue.id)
                || issue.is_blocked_by_active_issue()
            {
                continue;
            }
            let attempt = self.next_attempt(&issue)?;
            reserved_ids.insert(issue.id.clone());
            dispatch_queue.push((issue, attempt));
        }

        for (issue, attempt) in dispatch_queue {
            self.dispatch_issue(issue, attempt).await?;
        }

        Ok(())
    }

    pub fn snapshot(&self) -> OrchestratorSnapshot {
        OrchestratorSnapshot {
            running_issue_ids: self.running.keys().cloned().collect(),
            queued_retry_ids: self
                .retry_queue
                .iter()
                .map(|entry| entry.issue.id.clone())
                .collect(),
        }
    }

    async fn drain_reports(&mut self) -> Result<(), SchedulerError> {
        while let Ok(report) = self.reports_rx.try_recv() {
            self.running.remove(&report.issue.id);
            let (outcome, conversation_id) = match report.result {
                Ok(result) => {
                    self.workspace
                        .save_conversation_manifest(
                            &report.issue,
                            &self
                                .workspace_context(&report.issue, report.workspace_path.clone())?,
                            &result.conversation_id,
                            matches!(result.prompt_mode, PromptMode::Fresh),
                            None,
                        )
                        .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
                    (result.outcome, Some(result.conversation_id))
                }
                Err(error) => (WorkerOutcome::failure(error.to_string()), None),
            };

            let retry_reason = match outcome.kind {
                WorkerOutcomeKind::Success => Some(RetryReason::Continuation),
                WorkerOutcomeKind::Failure => Some(RetryReason::Failure),
                WorkerOutcomeKind::Stalled => Some(RetryReason::Stall),
                WorkerOutcomeKind::Cancelled | WorkerOutcomeKind::Released => None,
            };

            self.workspace
                .finish_attempt(
                    &report.issue,
                    report.attempt,
                    &outcome,
                    conversation_id.as_deref(),
                    retry_reason.clone(),
                )
                .await
                .map_err(|error| SchedulerError::Workspace(error.to_string()))?;

            match outcome.kind {
                WorkerOutcomeKind::Success => {
                    self.enqueue_retry(
                        report.issue,
                        report.attempt + 1,
                        RetryReason::Continuation,
                        continuation_retry_at(Utc::now()),
                    )?;
                }
                WorkerOutcomeKind::Failure | WorkerOutcomeKind::Stalled => {
                    let scheduled_at = Utc::now()
                        + failure_retry_delay(report.attempt, self.config.max_retry_backoff_ms);
                    self.enqueue_retry(
                        report.issue,
                        report.attempt + 1,
                        match outcome.kind {
                            WorkerOutcomeKind::Stalled => RetryReason::Stall,
                            _ => RetryReason::Failure,
                        },
                        scheduled_at,
                    )?;
                }
                WorkerOutcomeKind::Cancelled | WorkerOutcomeKind::Released => {
                    self.workspace
                        .clear_retry_manifest(&report.issue.identifier)
                        .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
                }
            }
        }
        Ok(())
    }

    async fn reconcile_running(&mut self) -> Result<(), SchedulerError> {
        if self.running.is_empty() {
            return Ok(());
        }

        let issue_ids = self.running.keys().cloned().collect::<Vec<_>>();
        let states = self
            .tracker
            .fetch_states_by_issue_ids(&issue_ids)
            .await
            .map_err(|error| SchedulerError::Tracker(error.to_string()))?;
        let states_by_id = states
            .into_iter()
            .map(|state| (state.id.clone(), state))
            .collect::<HashMap<_, _>>();

        let now = Utc::now();
        let mut releases = Vec::new();
        for (issue_id, worker) in &self.running {
            if let Some(state) = states_by_id.get(issue_id) {
                if state.is_terminal {
                    releases.push((
                        worker.issue.clone(),
                        worker.attempt,
                        true,
                        "terminal tracker state".to_string(),
                    ));
                    continue;
                }
                if !state.is_active {
                    releases.push((
                        worker.issue.clone(),
                        worker.attempt,
                        false,
                        "inactive tracker state".to_string(),
                    ));
                    continue;
                }
            }

            if (now - worker.started_at).num_milliseconds() > self.config.stall_timeout_ms {
                releases.push((
                    worker.issue.clone(),
                    worker.attempt,
                    false,
                    "worker stalled".to_string(),
                ));
            }
        }

        for (issue, attempt, cleanup_terminal, detail) in releases {
            if let Some(worker) = self.running.remove(&issue.id) {
                worker.task.abort();
            }
            let outcome = if cleanup_terminal {
                WorkerOutcome::released(detail)
            } else if detail == "worker stalled" {
                WorkerOutcome::stalled(detail)
            } else {
                WorkerOutcome::cancelled(detail)
            };
            self.workspace
                .finish_attempt(&issue, attempt, &outcome, None, None)
                .await
                .map_err(|error| SchedulerError::Workspace(error.to_string()))?;

            if cleanup_terminal {
                self.workspace
                    .cleanup_terminal_workspace(&issue)
                    .await
                    .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
                self.workspace
                    .clear_retry_manifest(&issue.identifier)
                    .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
            } else if matches!(outcome.kind, WorkerOutcomeKind::Stalled) {
                let scheduled_at =
                    Utc::now() + failure_retry_delay(attempt, self.config.max_retry_backoff_ms);
                self.enqueue_retry(issue, attempt + 1, RetryReason::Stall, scheduled_at)?;
            }
        }

        Ok(())
    }

    async fn dispatch_issue(&mut self, issue: Issue, attempt: u32) -> Result<(), SchedulerError> {
        let context = match self
            .workspace
            .prepare_issue_workspace(&issue, attempt)
            .await
        {
            Ok(context) => context,
            Err(error) => {
                let scheduled_at =
                    Utc::now() + failure_retry_delay(attempt, self.config.max_retry_backoff_ms);
                self.enqueue_retry(issue, attempt + 1, RetryReason::Failure, scheduled_at)?;
                return Err(SchedulerError::Workspace(error.to_string()));
            }
        };

        let prompt_mode = if self
            .workspace
            .load_conversation_manifest(&issue.identifier)
            .map_err(|error| SchedulerError::Workspace(error.to_string()))?
            .is_some()
        {
            PromptMode::Continuation
        } else {
            PromptMode::Fresh
        };

        let rendered = render_prompt(&context.workspace_path, &issue, attempt, &prompt_mode)
            .map_err(|error| SchedulerError::Workflow(error.to_string()))?;
        self.workspace
            .write_prompt(&context, prompt_mode.clone(), &rendered.prompt)
            .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
        self.workspace
            .clear_retry_manifest(&issue.identifier)
            .map_err(|error| SchedulerError::Workspace(error.to_string()))?;

        let request = IssueRunRequest {
            issue: issue.clone(),
            attempt,
            workspace_path: context.workspace_path.clone(),
            prompt_mode: prompt_mode.clone(),
            prompt: rendered.prompt,
            max_turns: rendered.max_turns,
        };

        let reports_tx = self.reports_tx.clone();
        let runner = self.runner.clone();
        let issue_for_task = issue.clone();
        let workspace_path = context.workspace_path.clone();
        let task = tokio::spawn(async move {
            let result = runner.run_issue(request).await;
            let _ = reports_tx.send(WorkerReport {
                issue: issue_for_task,
                attempt,
                workspace_path,
                result,
            });
        });

        self.running.insert(
            issue.id.clone(),
            RunningWorker {
                issue,
                attempt,
                started_at: Utc::now(),
                task,
            },
        );

        Ok(())
    }

    fn next_attempt(&self, issue: &Issue) -> Result<u32, SchedulerError> {
        let manifests = self
            .workspace
            .list_issue_manifests()
            .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
        Ok(manifests
            .into_iter()
            .find(|manifest| manifest.issue_id == issue.id)
            .map(|manifest| manifest.last_attempt + 1)
            .unwrap_or(1))
    }

    fn enqueue_retry(
        &mut self,
        issue: Issue,
        attempt: u32,
        reason: RetryReason,
        scheduled_at: DateTime<Utc>,
    ) -> Result<(), SchedulerError> {
        let entry = RetryEntry {
            issue,
            attempt,
            reason,
            scheduled_at,
        };
        self.workspace
            .persist_retry(&entry)
            .map_err(|error| SchedulerError::Workspace(error.to_string()))?;
        self.retry_queue.push(entry);
        self.retry_queue.sort_by_key(|entry| entry.scheduled_at);
        Ok(())
    }

    fn workspace_context(
        &self,
        issue: &Issue,
        workspace_path: PathBuf,
    ) -> Result<opensymphony_workspace::WorkspaceContext, SchedulerError> {
        Ok(opensymphony_workspace::WorkspaceContext {
            workspace_path,
            metadata_dir: self
                .workspace
                .workspace_path(&issue.identifier)
                .map_err(|error| SchedulerError::Workspace(error.to_string()))?
                .join(".opensymphony"),
            sanitized_workspace_key:
                opensymphony_workspace::WorkspaceManager::sanitize_issue_identifier(
                    &issue.identifier,
                ),
            created: false,
        })
    }
}

fn render_prompt(
    workspace_path: &std::path::Path,
    issue: &Issue,
    attempt: u32,
    prompt_mode: &PromptMode,
) -> Result<RenderedPrompt, WorkflowError> {
    let workflow_path = workspace_path.join("WORKFLOW.md");
    if workflow_path.exists() {
        let workflow = WorkflowDocument::load_from_path(&workflow_path)?;
        let prompt = match prompt_mode {
            PromptMode::Fresh => workflow.render_fresh_prompt(issue)?,
            PromptMode::Continuation => workflow.render_continuation_prompt(
                issue,
                &AttemptContext {
                    number: attempt,
                    continuation: true,
                },
            )?,
        };
        return Ok(RenderedPrompt {
            prompt,
            max_turns: workflow.front_matter.agent.max_turns,
        });
    }

    Ok(RenderedPrompt {
        prompt: match prompt_mode {
            PromptMode::Fresh => format!(
                "You are working on issue {}: {}",
                issue.identifier, issue.title
            ),
            PromptMode::Continuation => format!(
                "Continue working on issue {} after attempt {}",
                issue.identifier, attempt
            ),
        },
        max_turns: AgentConfig::default().max_turns,
    })
}

fn partition_due_retries(
    retries: Vec<RetryEntry>,
    now: DateTime<Utc>,
) -> (Vec<RetryEntry>, Vec<RetryEntry>) {
    let mut due = Vec::new();
    let mut deferred = Vec::new();
    for retry in retries {
        if retry.scheduled_at <= now {
            due.push(retry);
        } else {
            deferred.push(retry);
        }
    }
    (due, deferred)
}

fn issue_from_manifest(manifest: &IssueManifest, state: &str) -> Issue {
    Issue {
        id: manifest.issue_id.clone(),
        identifier: manifest.identifier.clone(),
        title: manifest.title.clone(),
        description: None,
        priority: None,
        state: state.to_string(),
        labels: vec![],
        blocked_by: vec![],
        created_at: manifest.created_at,
        updated_at: manifest.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use opensymphony_linear::LinearWriteOperations;
    use opensymphony_testkit::{make_issue, MemoryTracker, ScriptedRun, ScriptedRunner};
    use opensymphony_workspace::{HookConfig, WorkspaceConfig};
    use std::fs;
    use tempfile::tempdir;
    use tokio::time::{sleep, Duration};

    fn timestamp(seconds: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, seconds).unwrap()
    }

    fn scheduler(
        tracker: Arc<MemoryTracker>,
        runner: Arc<ScriptedRunner>,
        workspace_root: PathBuf,
        cleanup_terminal_workspaces: bool,
        after_create: Option<String>,
    ) -> Scheduler {
        Scheduler::new(
            SchedulerConfig {
                max_concurrency: 2,
                max_retry_backoff_ms: 120_000,
                stall_timeout_ms: 50,
            },
            tracker,
            runner,
            Arc::new(WorkspaceManager::new(WorkspaceConfig {
                root: workspace_root,
                cleanup_terminal_workspaces,
                hooks: HookConfig {
                    after_create,
                    ..HookConfig::default()
                },
            })),
        )
    }

    #[tokio::test]
    async fn schedules_continuation_retry_after_success() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(0)]);

        let tempdir = tempdir().expect("tempdir should exist");
        let mut scheduler = scheduler(tracker, runner, tempdir.path().to_path_buf(), false, None);

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(10)).await;
        scheduler.tick().await.expect("completion should reconcile");

        let snapshot = scheduler.snapshot();
        assert_eq!(snapshot.queued_retry_ids, vec!["1"]);
    }

    #[tokio::test]
    async fn schedules_failure_backoff_for_worker_errors() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::failure(0, "boom")]);

        let tempdir = tempdir().expect("tempdir should exist");
        let mut scheduler = scheduler(tracker, runner, tempdir.path().to_path_buf(), false, None);

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(10)).await;
        scheduler.tick().await.expect("completion should reconcile");

        assert_eq!(scheduler.retry_queue.len(), 1);
        assert_eq!(scheduler.retry_queue[0].reason, RetryReason::Failure);
    }

    #[tokio::test]
    async fn detects_stalled_workers_and_retries_them() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(200)]);

        let tempdir = tempdir().expect("tempdir should exist");
        let mut scheduler = scheduler(tracker, runner, tempdir.path().to_path_buf(), false, None);

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(100)).await;
        scheduler.tick().await.expect("stall should reconcile");

        assert_eq!(scheduler.retry_queue.len(), 1);
        assert_eq!(scheduler.retry_queue[0].reason, RetryReason::Stall);
    }

    #[tokio::test]
    async fn cleans_up_terminal_workspaces_when_tracker_state_changes() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(200)]);

        let tempdir = tempdir().expect("tempdir should exist");
        let workspace_root = tempdir.path().to_path_buf();
        let mut scheduler = scheduler(tracker.clone(), runner, workspace_root.clone(), true, None);

        scheduler.tick().await.expect("dispatch should succeed");
        tracker
            .transition_issue("1", "Done")
            .expect("transition should succeed");
        scheduler.tick().await.expect("reconcile should succeed");

        assert!(!workspace_root.join("ABC-1").exists());
    }

    #[tokio::test]
    async fn recovers_persisted_retries_from_workspace_manifests() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        let tempdir = tempdir().expect("tempdir should exist");
        let workspace = Arc::new(WorkspaceManager::new(WorkspaceConfig {
            root: tempdir.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig::default(),
        }));

        let context = workspace
            .prepare_issue_workspace(&issue, 1)
            .await
            .expect("workspace should prepare");
        workspace
            .save_conversation_manifest(&issue, &context, "conversation-ABC-1", true, None)
            .expect("conversation should persist");
        workspace
            .persist_retry(&RetryEntry {
                issue: issue.clone(),
                attempt: 2,
                reason: RetryReason::Continuation,
                scheduled_at: Utc::now(),
            })
            .expect("retry should persist");

        let mut scheduler = Scheduler::new(SchedulerConfig::default(), tracker, runner, workspace);
        let recovered = scheduler.recover().await.expect("recovery should succeed");

        assert_eq!(recovered, 1);
        assert_eq!(scheduler.retry_queue.len(), 1);
        assert_eq!(scheduler.retry_queue[0].attempt, 2);
    }

    #[tokio::test]
    async fn clears_retry_manifest_for_terminal_issue_during_recovery_when_workspace_is_retained() {
        let issue = make_issue("1", "ABC-1", "Done", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        let tempdir = tempdir().expect("tempdir should exist");
        let workspace = Arc::new(WorkspaceManager::new(WorkspaceConfig {
            root: tempdir.path().to_path_buf(),
            cleanup_terminal_workspaces: false,
            hooks: HookConfig::default(),
        }));

        let context = workspace
            .prepare_issue_workspace(&issue, 1)
            .await
            .expect("workspace should prepare");
        workspace
            .persist_retry(&RetryEntry {
                issue: issue.clone(),
                attempt: 2,
                reason: RetryReason::Continuation,
                scheduled_at: Utc::now(),
            })
            .expect("retry should persist");

        let mut scheduler = Scheduler::new(SchedulerConfig::default(), tracker, runner, workspace);
        let recovered = scheduler.recover().await.expect("recovery should succeed");

        assert_eq!(recovered, 0);
        assert!(context.workspace_path.exists());
        assert!(!context
            .workspace_path
            .join(".opensymphony/retry.json")
            .exists());
    }

    #[tokio::test]
    async fn does_not_redispatch_issue_with_deferred_retry() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());

        let tempdir = tempdir().expect("tempdir should exist");
        let mut scheduler = scheduler(
            tracker,
            runner.clone(),
            tempdir.path().to_path_buf(),
            false,
            None,
        );
        let retry = RetryEntry {
            issue: issue.clone(),
            attempt: 2,
            reason: RetryReason::Failure,
            scheduled_at: Utc::now() + chrono::Duration::seconds(60),
        };

        let context = scheduler
            .workspace
            .prepare_issue_workspace(&issue, 1)
            .await
            .expect("workspace should prepare");
        scheduler
            .workspace
            .persist_retry(&retry)
            .expect("retry should persist");
        scheduler.retry_queue.push(retry);

        scheduler
            .tick()
            .await
            .expect("deferred retry should remain queued");

        assert!(runner.requests().is_empty());
        assert_eq!(scheduler.retry_queue.len(), 1);
        assert!(context
            .workspace_path
            .join(".opensymphony/retry.json")
            .exists());
    }

    #[tokio::test]
    async fn renders_fresh_workflow_prompt_without_continuation_guidance() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(0)]);

        let workspace_dir = tempdir().expect("tempdir should exist");
        let fixture_repo = tempdir().expect("fixture tempdir should exist");
        fs::write(
            fixture_repo.path().join("WORKFLOW.md"),
            include_str!("../../../examples/target-repo/WORKFLOW.md"),
        )
        .expect("workflow fixture should write");

        let after_create = format!(
            "cp {} {}",
            fixture_repo.path().join("WORKFLOW.md").display(),
            "./WORKFLOW.md"
        );
        let mut scheduler = scheduler(
            tracker,
            runner.clone(),
            workspace_dir.path().to_path_buf(),
            false,
            Some(after_create),
        );

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(10)).await;

        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].prompt_mode, PromptMode::Fresh);
        assert!(requests[0].prompt.contains("# Assignment"));
        assert!(!requests[0].prompt.contains("## Continuation"));
    }

    #[tokio::test]
    async fn renders_continuation_guidance_without_replaying_full_assignment() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(0)]);

        let workspace_dir = tempdir().expect("tempdir should exist");
        let fixture_repo = tempdir().expect("fixture tempdir should exist");
        fs::write(
            fixture_repo.path().join("WORKFLOW.md"),
            include_str!("../../../examples/target-repo/WORKFLOW.md"),
        )
        .expect("workflow fixture should write");

        let after_create = format!(
            "cp {} {}",
            fixture_repo.path().join("WORKFLOW.md").display(),
            "./WORKFLOW.md"
        );
        let mut scheduler = scheduler(
            tracker,
            runner.clone(),
            workspace_dir.path().to_path_buf(),
            false,
            Some(after_create),
        );
        let context = scheduler
            .workspace
            .prepare_issue_workspace(&issue, 1)
            .await
            .expect("workspace should prepare");
        scheduler
            .workspace
            .save_conversation_manifest(&issue, &context, "conversation-ABC-1", false, None)
            .expect("conversation should persist");

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(10)).await;

        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].prompt_mode, PromptMode::Continuation);
        assert!(requests[0].prompt.contains("## Continuation"));
        assert!(!requests[0].prompt.contains("# Assignment"));
        assert!(requests[0].prompt.contains("attempt 2"));
    }

    #[tokio::test]
    async fn honors_workflow_max_turns_when_dispatching() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(0)]);

        let workspace_dir = tempdir().expect("tempdir should exist");
        let fixture_repo = tempdir().expect("fixture tempdir should exist");
        fs::write(
            fixture_repo.path().join("WORKFLOW.md"),
            r#"---
tracker:
  project_slug: "example-project"
  active_states:
    - Todo
  terminal_states:
    - Done
agent:
  max_turns: 7
---

# Assignment

Issue {{ issue.identifier }}
"#,
        )
        .expect("workflow fixture should write");

        let after_create = format!(
            "cp {} {}",
            fixture_repo.path().join("WORKFLOW.md").display(),
            "./WORKFLOW.md"
        );
        let mut scheduler = scheduler(
            tracker,
            runner.clone(),
            workspace_dir.path().to_path_buf(),
            false,
            Some(after_create),
        );

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(10)).await;

        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].max_turns, 7);
        assert!(requests[0].prompt.contains("# Assignment"));
        assert!(!requests[0].prompt.contains("## Continuation"));
    }

    #[tokio::test]
    async fn executes_end_to_end_in_temp_repo_and_writes_artifacts() {
        let issue = make_issue("1", "ABC-1", "Todo", Some(1), timestamp(0));
        let tracker = Arc::new(MemoryTracker::new(
            vec![issue.clone()],
            vec!["Todo".to_string()],
            vec!["Done".to_string()],
            vec!["Todo".to_string(), "Done".to_string()],
        ));
        let runner = Arc::new(ScriptedRunner::default());
        runner.set_plan(&issue.id, vec![ScriptedRun::success(0)]);

        let workspace_dir = tempdir().expect("tempdir should exist");
        let fixture_repo = tempdir().expect("fixture tempdir should exist");
        fs::write(
            fixture_repo.path().join("WORKFLOW.md"),
            include_str!("../../../examples/target-repo/WORKFLOW.md"),
        )
        .expect("workflow fixture should write");

        let after_create = format!(
            "cp {} {}",
            fixture_repo.path().join("WORKFLOW.md").display(),
            "./WORKFLOW.md"
        );
        let mut scheduler = scheduler(
            tracker,
            runner.clone(),
            workspace_dir.path().to_path_buf(),
            false,
            Some(after_create),
        );

        scheduler.tick().await.expect("dispatch should succeed");
        sleep(Duration::from_millis(10)).await;
        scheduler.tick().await.expect("completion should reconcile");

        let workspace_path = workspace_dir.path().join("ABC-1");
        assert!(workspace_path
            .join(".opensymphony/generated/issue-context.md")
            .exists());
        assert!(workspace_path
            .join(".opensymphony/generated/session-context.json")
            .exists());
        assert!(workspace_path
            .join(".opensymphony/prompts/last-full-prompt.md")
            .exists());
        assert!(workspace_path
            .join(".opensymphony/conversation.json")
            .exists());

        let requests = runner.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].prompt.contains("ABC-1"));
    }
}
