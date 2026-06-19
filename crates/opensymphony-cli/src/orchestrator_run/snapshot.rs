//! Snapshot and control-plane mapping helpers for the runtime CLI.

use std::{
    collections::{HashSet, VecDeque},
    path::Path,
};

use crate::opensymphony_control::{
    AgentServerStatus, ConversationEvent, DaemonSnapshot, DaemonState, DaemonStatus,
    IssueRuntimeState, IssueSnapshot, MemoryServerStatus, MetricsSnapshot, RecentEvent,
    RecentEventKind, WorkerOutcome,
};
use crate::opensymphony_domain::{
    HealthStatus, IssueIdentifier, OrchestratorSnapshot, SchedulerStatus, WorkerOutcomeKind,
};
use crate::opensymphony_openhands::LocalServerSupervisor;
use crate::opensymphony_workflow::ResolvedWorkflow;
use chrono::{DateTime, Utc};

use super::timestamp_to_datetime;

const RECENT_EVENT_LIMIT: usize = 24;

pub(super) fn map_snapshot(
    snapshot: &OrchestratorSnapshot,
    workspace_root: &Path,
    terminal_states: &HashSet<String>,
    agent_server: AgentServerStatus,
    memory_server: MemoryServerStatus,
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
        memory_server,
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

pub(super) fn current_memory_server_status(
    memory_server: Option<&super::super::memory::MemoryServerHandle>,
) -> MemoryServerStatus {
    let Some(memory_server) = memory_server else {
        return MemoryServerStatus::default();
    };
    let reachable = !memory_server.is_finished();
    MemoryServerStatus {
        enabled: true,
        reachable,
        endpoint: Some(memory_server.endpoint().to_string()),
        status_line: if reachable {
            "listening".to_string()
        } else {
            "stopped".to_string()
        },
    }
}

fn map_issue(
    issue: &crate::opensymphony_domain::IssueSnapshot,
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
        blocked_by: issue
            .issue
            .blocked_by
            .iter()
            .filter_map(|blocker| blocker.identifier.as_ref())
            .map(ToString::to_string)
            .collect(),
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
                    .map(|activity| ConversationEvent {
                        event_id: activity.event_id.clone(),
                        happened_at: timestamp_to_datetime(activity.happened_at),
                        kind: activity.kind.clone(),
                        summary: activity.summary.clone(),
                        sequence: activity.sequence,
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
        detached: false,
        cancel_acknowledged: false,
        cancel_failed: false,
    }
}

fn map_worker_outcome(
    issue: &crate::opensymphony_domain::IssueSnapshot,
    runtime_state: IssueRuntimeState,
) -> WorkerOutcome {
    match runtime_state {
        IssueRuntimeState::Running => WorkerOutcome::Running,
        IssueRuntimeState::Paused => WorkerOutcome::Unknown,
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
                | WorkerOutcomeKind::Stalled
                | WorkerOutcomeKind::Detached
                | WorkerOutcomeKind::CancelFailed,
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
                | WorkerOutcomeKind::Stalled
                | WorkerOutcomeKind::Detached
                | WorkerOutcomeKind::CancelFailed,
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
            reachable: matches!(
                status.state,
                crate::opensymphony_openhands::ServerState::Ready
            ),
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

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use crate::opensymphony_domain::{
        BlockerRef, ComponentHealthSnapshot, ConversationId, ConversationMetadata, DaemonSnapshot,
        HealthStatus, IssueId, IssueIdentifier, IssueRef, IssueSnapshot as DomainIssueSnapshot,
        IssueState, IssueStateCategory, NormalizedIssue, OrchestratorSnapshot,
        RuntimeStateSnapshot, RuntimeStreamState, RuntimeUsageTotals, SchedulerStatus, TimestampMs,
        WorkspaceKey, WorkspaceRecord,
    };

    use super::{map_snapshot, terminal_state_set};

    fn must<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{error}"),
        }
    }

    fn ts(value: u64) -> TimestampMs {
        TimestampMs::new(value)
    }

    fn resolved_workflow_for_tests() -> crate::opensymphony_workflow::ResolvedWorkflow {
        let workflow = crate::opensymphony_workflow::WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - In Progress
  terminal_states:
    - Done
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");

        workflow
            .resolve(
                std::path::Path::new("/tmp"),
                &BTreeMap::from([("LINEAR_API_KEY".to_owned(), "linear-token".to_owned())]),
            )
            .expect("workflow should resolve")
    }

    #[test]
    fn map_snapshot_preserves_full_recent_conversation_window() {
        let recent_activity = (0..12)
            .map(
                |index| crate::opensymphony_domain::ConversationActivityEvent {
                    event_id: format!("evt-{index}"),
                    happened_at: ts(1_000 + index),
                    kind: "ActionEvent".to_owned(),
                    summary: format!("summary {index}"),
                    sequence: index,
                },
            )
            .collect();

        let snapshot = OrchestratorSnapshot::new(
            ts(2_000),
            DaemonSnapshot::new(
                HealthStatus::Healthy,
                1_000,
                4,
                Some(ts(2_000)),
                ComponentHealthSnapshot::default(),
                RuntimeUsageTotals::default(),
            ),
            vec![DomainIssueSnapshot {
                issue: NormalizedIssue {
                    id: must(IssueId::new("lin_352")),
                    identifier: must(IssueIdentifier::new("COE-352")),
                    title: "Render media pipeline".to_owned(),
                    description: None,
                    priority: None,
                    state: IssueState {
                        id: None,
                        name: "In Progress".to_owned(),
                        category: IssueStateCategory::Active,
                    },
                    branch_name: None,
                    url: None,
                    labels: Vec::new(),
                    parent_id: None,
                    blocked_by: Vec::<BlockerRef>::new(),
                    sub_issues: Vec::<IssueRef>::new(),
                    created_at: None,
                    updated_at: None,
                },
                runtime: RuntimeStateSnapshot {
                    state: SchedulerStatus::Running,
                    claimed_at: None,
                    started_at: None,
                    released_at: None,
                    release_reason: None,
                    worker: None,
                    last_event_at: Some(ts(1_011)),
                    stalled_at: None,
                },
                workspace: Some(WorkspaceRecord {
                    path: PathBuf::from("/tmp/workspaces/COE-352"),
                    workspace_key: must(WorkspaceKey::new("COE-352")),
                    created_now: false,
                    created_at: None,
                    updated_at: None,
                    last_seen_tracker_refresh_at: None,
                }),
                conversation: Some(ConversationMetadata {
                    conversation_id: must(ConversationId::new("conv_352")),
                    server_base_url: Some("http://127.0.0.1:3000".to_owned()),
                    transport_target: Some("loopback".to_owned()),
                    http_auth_mode: Some("none".to_owned()),
                    websocket_auth_mode: Some("none".to_owned()),
                    websocket_query_param_name: None,
                    fresh_conversation: false,
                    runtime_contract_version: Some("openhands-sdk-agent-server-v1".to_owned()),
                    stream_state: RuntimeStreamState::Ready,
                    last_event_id: Some("evt-11".to_owned()),
                    last_event_kind: Some("ActionEvent".to_owned()),
                    last_event_at: Some(ts(1_011)),
                    last_event_summary: Some("summary 11".to_owned()),
                    recent_activity,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    total_tokens: 0,
                    runtime_seconds: 0,
                    next_activity_sequence: 0,
                }),
                retry: None,
                last_worker_outcome: None,
                recent_worker_outcomes: Vec::new(),
            }],
        );

        let mapped = map_snapshot(
            &snapshot,
            PathBuf::from("/tmp/workspaces").as_path(),
            &terminal_state_set(&resolved_workflow_for_tests()),
            crate::opensymphony_control::AgentServerStatus {
                reachable: true,
                base_url: "http://127.0.0.1:3000".to_owned(),
                conversation_count: 1,
                status_line: "healthy".to_owned(),
            },
            crate::opensymphony_control::MemoryServerStatus::default(),
            &std::collections::VecDeque::new(),
        );

        assert_eq!(mapped.issues[0].recent_events.len(), 12);
        assert_eq!(mapped.issues[0].recent_events[0].summary, "summary 11");
        assert_eq!(mapped.issues[0].recent_events[11].summary, "summary 0");
    }
}
