//! Snapshot and control-plane mapping helpers for the runtime CLI.

use std::{
    collections::{HashSet, VecDeque},
    path::Path,
};

use chrono::{DateTime, Utc};
use opensymphony_control::{
    AgentServerStatus, ConversationEvent, DaemonSnapshot, DaemonState, DaemonStatus,
    IssueRuntimeState, IssueSnapshot, MetricsSnapshot, RecentEvent, RecentEventKind, WorkerOutcome,
};
use opensymphony_domain::{
    HealthStatus, IssueIdentifier, OrchestratorSnapshot, SchedulerStatus, WorkerOutcomeKind,
};
use opensymphony_openhands::LocalServerSupervisor;
use opensymphony_workflow::ResolvedWorkflow;

use super::timestamp_to_datetime;

const RECENT_EVENT_LIMIT: usize = 24;

pub(super) fn map_snapshot(
    snapshot: &OrchestratorSnapshot,
    workspace_root: &Path,
    terminal_states: &HashSet<String>,
    agent_server: AgentServerStatus,
    recent_events: &VecDeque<RecentEvent>,
) -> DaemonSnapshot {
    let generated_at = timestamp_to_datetime(snapshot.generated_at);
    let last_poll_at = snapshot
        .daemon
        .last_poll_at
        .map(timestamp_to_datetime)
        .unwrap_or(generated_at);
    DaemonSnapshot {
        generated_at,
        daemon: DaemonStatus {
            state: map_daemon_state(snapshot.daemon.health),
            last_poll_at,
            workspace_root: workspace_root.display().to_string(),
            status_line: format!(
                "poll={}ms, running={}, retry_queue={}",
                snapshot.daemon.poll_interval_ms,
                snapshot.daemon.running_issue_count,
                snapshot.daemon.retry_queue_count
            ),
        },
        agent_server,
        metrics: MetricsSnapshot {
            running_issues: snapshot.daemon.running_issue_count as u32,
            retry_queue_depth: snapshot.daemon.retry_queue_count as u32,
            input_tokens: snapshot.daemon.usage.input_tokens,
            output_tokens: snapshot.daemon.usage.output_tokens,
            cache_read_tokens: snapshot.daemon.usage.cache_read_tokens,
            total_tokens: snapshot.daemon.usage.total_tokens,
            total_cost_micros: snapshot.daemon.usage.estimated_cost_usd_micros.unwrap_or(0),
        },
        issues: snapshot
            .issues
            .iter()
            .map(|issue| map_issue(issue, terminal_states, generated_at))
            .collect(),
        recent_events: recent_events.iter().cloned().collect(),
    }
}

fn map_issue(
    issue: &opensymphony_domain::IssueSnapshot,
    terminal_states: &HashSet<String>,
    generated_at: DateTime<Utc>,
) -> IssueSnapshot {
    let runtime_state = match issue.runtime.state {
        SchedulerStatus::Running | SchedulerStatus::Claimed => IssueRuntimeState::Running,
        SchedulerStatus::RetryQueued => IssueRuntimeState::RetryQueued,
        SchedulerStatus::Released => match issue
            .last_worker_outcome
            .as_ref()
            .map(|outcome| outcome.outcome)
        {
            Some(
                WorkerOutcomeKind::Failed
                | WorkerOutcomeKind::TimedOut
                | WorkerOutcomeKind::Stalled,
            ) => IssueRuntimeState::Failed,
            _ => IssueRuntimeState::Completed,
        },
        SchedulerStatus::Unclaimed => IssueRuntimeState::Idle,
    };
    let last_outcome = map_worker_outcome(issue, runtime_state);
    let last_event_at = issue
        .conversation
        .as_ref()
        .and_then(|conversation| conversation.last_event_at)
        .map(timestamp_to_datetime)
        .or_else(|| {
            issue
                .last_worker_outcome
                .as_ref()
                .map(|outcome| timestamp_to_datetime(outcome.finished_at))
        })
        .unwrap_or(generated_at);

    IssueSnapshot {
        identifier: issue.issue.identifier.to_string(),
        title: issue.issue.title.clone(),
        tracker_state: issue.issue.state.name.clone(),
        runtime_state,
        last_outcome,
        last_event_at,
        conversation_id_suffix: issue
            .conversation
            .as_ref()
            .map(|conversation| suffix(conversation.conversation_id.as_str()))
            .unwrap_or_else(|| "-".to_string()),
        workspace_path_suffix: issue
            .workspace
            .as_ref()
            .map(|workspace| suffix_path(&workspace.path))
            .unwrap_or_else(|| "-".to_string()),
        retry_count: issue
            .retry
            .as_ref()
            .map(|retry| retry.normal_retry_count)
            .unwrap_or(0),
        blocked: issue.issue.blocked_by.iter().any(|blocker| {
            blocker
                .state
                .as_deref()
                .is_none_or(|state| !is_terminal_state(terminal_states, state))
        }) || (!issue.issue.sub_issues.is_empty()
            && issue
                .issue
                .sub_issues
                .iter()
                .any(|sub_issue| !is_terminal_state(terminal_states, &sub_issue.state))),
        server_base_url: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.server_base_url.clone()),
        transport_target: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.transport_target.clone()),
        http_auth_mode: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.http_auth_mode.clone()),
        websocket_auth_mode: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.websocket_auth_mode.clone()),
        websocket_query_param_name: issue
            .conversation
            .as_ref()
            .and_then(|conversation| conversation.websocket_query_param_name.clone()),
        recent_events: issue
            .conversation
            .as_ref()
            .map(|conversation| {
                conversation
                    .recent_activity
                    .iter()
                    .rev()
                    .take(10)
                    .map(|activity| ConversationEvent {
                        event_id: activity.event_id.clone(),
                        happened_at: timestamp_to_datetime(activity.happened_at),
                        kind: activity.kind.clone(),
                        summary: activity.summary.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        modified_files: Vec::new(),
        input_tokens: issue
            .conversation
            .as_ref()
            .map(|conversation| conversation.input_tokens)
            .unwrap_or(0),
        output_tokens: issue
            .conversation
            .as_ref()
            .map(|conversation| conversation.output_tokens)
            .unwrap_or(0),
        cache_read_tokens: issue
            .conversation
            .as_ref()
            .map(|conversation| conversation.cache_read_tokens)
            .unwrap_or(0),
    }
}

fn map_worker_outcome(
    issue: &opensymphony_domain::IssueSnapshot,
    runtime_state: IssueRuntimeState,
) -> WorkerOutcome {
    match runtime_state {
        IssueRuntimeState::Running => WorkerOutcome::Running,
        IssueRuntimeState::RetryQueued => match issue
            .last_worker_outcome
            .as_ref()
            .map(|outcome| outcome.outcome)
        {
            Some(WorkerOutcomeKind::Succeeded) => WorkerOutcome::Continued,
            Some(WorkerOutcomeKind::Cancelled) => WorkerOutcome::Canceled,
            Some(
                WorkerOutcomeKind::Failed
                | WorkerOutcomeKind::TimedOut
                | WorkerOutcomeKind::Stalled,
            ) => WorkerOutcome::Failed,
            None => WorkerOutcome::Continued,
        },
        IssueRuntimeState::Completed => match issue
            .last_worker_outcome
            .as_ref()
            .map(|outcome| outcome.outcome)
        {
            Some(WorkerOutcomeKind::Cancelled) => WorkerOutcome::Canceled,
            Some(
                WorkerOutcomeKind::Failed
                | WorkerOutcomeKind::TimedOut
                | WorkerOutcomeKind::Stalled,
            ) => WorkerOutcome::Failed,
            _ => WorkerOutcome::Completed,
        },
        IssueRuntimeState::Failed => WorkerOutcome::Failed,
        IssueRuntimeState::Idle => WorkerOutcome::Unknown,
        IssueRuntimeState::Releasing => WorkerOutcome::Unknown,
    }
}

pub(super) fn current_agent_server_status(
    supervisor: &mut Option<LocalServerSupervisor>,
    base_url: &str,
) -> AgentServerStatus {
    if let Some(supervisor) = supervisor.as_mut()
        && let Ok(status) = supervisor.status()
    {
        return AgentServerStatus {
            reachable: matches!(status.state, opensymphony_openhands::ServerState::Ready),
            base_url: status.base_url,
            conversation_count: 0,
            status_line: format!("{:?}", status.state).to_ascii_lowercase(),
        };
    }

    AgentServerStatus {
        reachable: true,
        base_url: base_url.to_string(),
        conversation_count: 0,
        status_line: "reachable".to_string(),
    }
}

pub(super) fn push_recent_event(
    recent_events: &mut VecDeque<RecentEvent>,
    kind: RecentEventKind,
    issue_identifier: Option<IssueIdentifier>,
    summary: String,
    happened_at: DateTime<Utc>,
) {
    recent_events.push_front(RecentEvent {
        happened_at,
        issue_identifier: issue_identifier.map(|identifier| identifier.to_string()),
        kind,
        summary,
    });
    while recent_events.len() > RECENT_EVENT_LIMIT {
        let _ = recent_events.pop_back();
    }
}

pub(super) fn terminal_state_set(workflow: &ResolvedWorkflow) -> HashSet<String> {
    workflow
        .config
        .tracker
        .terminal_states
        .iter()
        .map(|state| state.trim().to_ascii_lowercase())
        .collect()
}

fn is_terminal_state(terminal_states: &HashSet<String>, state: &str) -> bool {
    terminal_states.contains(&state.trim().to_ascii_lowercase())
}

fn map_daemon_state(health: HealthStatus) -> DaemonState {
    match health {
        HealthStatus::Unknown | HealthStatus::Starting => DaemonState::Starting,
        HealthStatus::Healthy => DaemonState::Ready,
        HealthStatus::Degraded | HealthStatus::Failed => DaemonState::Degraded,
    }
}

fn suffix(value: &str) -> String {
    if value.len() <= 8 {
        value.to_string()
    } else {
        value[value.len() - 8..].to_string()
    }
}

fn suffix_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}
