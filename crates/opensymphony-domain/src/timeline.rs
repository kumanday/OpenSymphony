use chrono::Utc;
use serde_json::Value;

use crate::opensymphony_gateway_schema::{
    envelope::EntityKind,
    event_journal::{EventKind, EventRecord},
    run::{RunPhase, RunStreamLiveness},
    timeline::{
        RunStateEvidence, RunTimeline, TimelineEntityRef, TimelineEntry, TimelineEntryKind,
        TokenDelta,
    },
    version::SchemaVersion,
};

/// Build a [`RunTimeline`] by grouping a run's event journal records into
/// readable timeline entries.
///
/// The builder is intentionally deterministic and stateless: it receives a
/// slice of [`EventRecord`]s and returns a fully materialized timeline. Callers
/// (gateway, tests, diagnostics) are responsible for filtering the records to
/// the run they care about.
#[derive(Debug, Clone)]
pub struct TimelineBuilder {
    run_id: String,
}

impl TimelineBuilder {
    /// Create a builder scoped to a run id.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
        }
    }

    /// Build a timeline from a collection of event records.
    ///
    /// Records are grouped by adjacent similarity (kind, phase, tool, command,
    /// terminal session). Each group becomes one [`TimelineEntry`].
    pub fn build(&self, records: &[EventRecord]) -> RunTimeline {
        let mut entries: Vec<TimelineEntry> = Vec::new();
        let mut current: Option<TimelineEntry> = None;

        for record in records.iter().filter(|r| belongs_to_run(&self.run_id, r)) {
            let (kind, phase, command_id, tool_name, terminal_session_id, log_level, token_delta) =
                classify(record);
            let entity_refs = entity_refs_for(record, &self.run_id);
            let file_paths = file_paths_from(record);
            let title = title_for(
                kind,
                phase,
                record,
                tool_name.as_deref(),
                command_id.as_deref(),
            );

            let can_extend = current.as_ref().is_some_and(|c| {
                c.kind == kind
                    && c.phase == phase
                    && c.title == title
                    && c.command_id == command_id
                    && c.tool_name == tool_name
                    && c.terminal_session_id == terminal_session_id
                    && c.log_level == log_level
            });

            if can_extend {
                let mut c = current.take().expect("current is some");
                c.sequence_end = record.sequence;
                c.event_ids.push(record.event_id.clone());
                if c.summary.len() < 200 {
                    c.summary = format!("{}; {}", c.summary, record.summary);
                }
                // Merge token deltas.
                if let (Some(acc), Some(delta)) = (&mut c.token_delta, token_delta) {
                    acc.input = acc.input.saturating_add(delta.input);
                    acc.output = acc.output.saturating_add(delta.output);
                    acc.cache_read = acc.cache_read.saturating_add(delta.cache_read);
                } else if c.token_delta.is_none() {
                    c.token_delta = token_delta;
                }
                // Merge file paths.
                for path in file_paths {
                    if !c.file_paths.contains(&path) {
                        c.file_paths.push(path);
                    }
                }
                // Merge entity refs.
                for r in entity_refs {
                    if !c
                        .entity_refs
                        .iter()
                        .any(|er| er.kind == r.kind && er.id == r.id)
                    {
                        c.entity_refs.push(r);
                    }
                }
                c.state_evidence = state_evidence(kind, phase, &c, record);
                current = Some(c);
            } else {
                if let Some(c) = current.take() {
                    entries.push(c);
                }
                let base_entry = TimelineEntry {
                    entry_id: record.event_id.clone(),
                    sequence_start: record.sequence,
                    sequence_end: record.sequence,
                    happened_at: record.happened_at,
                    kind,
                    phase,
                    title: title.clone(),
                    summary: record.summary.clone(),
                    event_ids: vec![record.event_id.clone()],
                    entity_refs,
                    command_id,
                    tool_name,
                    file_paths,
                    terminal_session_id,
                    log_level,
                    token_delta,
                    state_evidence: None,
                };
                let evidence = state_evidence(kind, phase, &base_entry, record);
                current = Some(TimelineEntry {
                    state_evidence: evidence,
                    ..base_entry
                });
            }
        }

        if let Some(c) = current {
            entries.push(c);
        }

        RunTimeline {
            schema_version: SchemaVersion::v1(),
            run_id: self.run_id.clone(),
            generated_at: Utc::now(),
            entries,
        }
    }
}

pub fn belongs_to_run(run_id: &str, record: &EventRecord) -> bool {
    record.entity_refs.iter().any(|r| match r.kind {
        EntityKind::Run => r.id == run_id,
        EntityKind::Issue => r.identifier.as_deref() == Some(run_id) || r.id == run_id,
        _ => false,
    }) || record
        .payload
        .as_ref()
        .is_some_and(|p| payload_run_id(p) == Some(run_id))
}

pub fn payload_run_id(payload: &Value) -> Option<&str> {
    payload
        .get("run_id")
        .or_else(|| payload.get("association").and_then(|a| a.get("run_id")))
        .and_then(|v| v.as_str())
}

fn entity_refs_for(record: &EventRecord, run_id: &str) -> Vec<TimelineEntityRef> {
    let mut refs: Vec<TimelineEntityRef> = record
        .entity_refs
        .iter()
        .filter(|r| {
            matches!(
                r.kind,
                EntityKind::Run
                    | EntityKind::Issue
                    | EntityKind::SubIssue
                    | EntityKind::TerminalSession
            )
        })
        .map(|r| TimelineEntityRef {
            kind: r.kind,
            id: r.id.clone(),
            identifier: r.identifier.clone(),
        })
        .collect();

    // Ensure the run itself is always present.
    if !refs
        .iter()
        .any(|r| r.kind == EntityKind::Run && r.id == run_id)
    {
        refs.push(TimelineEntityRef::run(run_id));
    }

    // Pull association references out of terminal/log payloads.
    if let Some(association) = record.payload.as_ref().and_then(|p| p.get("association")) {
        if let Some(issue) = association.get("issue_id").and_then(|v| v.as_str())
            && !refs
                .iter()
                .any(|r| r.kind == EntityKind::Issue && r.id == issue)
        {
            refs.push(TimelineEntityRef::issue(issue, issue));
        }
        if let Some(sub) = association.get("sub_issue_id").and_then(|v| v.as_str())
            && !refs
                .iter()
                .any(|r| r.kind == EntityKind::SubIssue && r.id == sub)
        {
            refs.push(TimelineEntityRef::sub_issue(sub));
        }
    }

    refs
}

fn file_paths_from(record: &EventRecord) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(payload) = record.payload.as_ref() {
        if let Some(path) = payload.get("path").and_then(|v| v.as_str()) {
            paths.push(path.to_string());
        }
        if let Some(files) = payload.get("files").and_then(|v| v.as_array()) {
            for f in files {
                if let Some(p) = f.get("path").and_then(|v| v.as_str()) {
                    paths.push(p.to_string());
                }
            }
        }
    }
    paths
}

type Classified = (
    TimelineEntryKind,
    Option<RunPhase>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<TokenDelta>,
);

fn classify(record: &EventRecord) -> Classified {
    match &record.kind {
        EventKind::RunStarted => (
            TimelineEntryKind::State,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::RunCompleted => (
            TimelineEntryKind::State,
            Some(RunPhase::Completed),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::RunFailed => (
            TimelineEntryKind::State,
            Some(RunPhase::Stalled),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::RunCancelled => (
            TimelineEntryKind::State,
            Some(RunPhase::Cancelled),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::OrchestratorRetryScheduled { .. } => (
            TimelineEntryKind::State,
            Some(RunPhase::RetryQueued),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::OrchestratorWorkerFailed { reason } => {
            let phase = if reason.to_lowercase().contains("detach") {
                Some(RunPhase::Detached)
            } else {
                Some(RunPhase::Stalled)
            };
            (
                TimelineEntryKind::StallProbe,
                phase,
                None,
                None,
                None,
                None,
                None,
            )
        }
        EventKind::GatewayActionFailed { .. } => (
            TimelineEntryKind::StallProbe,
            Some(RunPhase::Degraded),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::StreamConnected { .. } => (
            TimelineEntryKind::Reconnect,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::StreamDisconnected { .. } => (
            TimelineEntryKind::Reconnect,
            Some(RunPhase::Degraded),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::StreamReconnected { .. } => (
            TimelineEntryKind::Reconnect,
            Some(RunPhase::Degraded),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::TerminalFrame { .. } => {
            let (command_id, session_id) = terminal_context(record);
            (
                TimelineEntryKind::Terminal,
                Some(RunPhase::Active),
                command_id,
                None,
                session_id,
                None,
                None,
            )
        }
        EventKind::LogEntry { level } => {
            let (command_id, session_id) = terminal_context(record);
            (
                TimelineEntryKind::Log,
                Some(RunPhase::Active),
                command_id,
                None,
                session_id,
                Some(level.clone()),
                None,
            )
        }
        EventKind::HarnessToolCall => (
            TimelineEntryKind::ToolCall,
            Some(RunPhase::Active),
            None,
            tool_name(record),
            None,
            None,
            None,
        ),
        EventKind::HarnessToolResult => (
            TimelineEntryKind::ToolCall,
            Some(RunPhase::Active),
            None,
            tool_name(record),
            None,
            None,
            None,
        ),
        EventKind::HarnessConversationStateUpdate => {
            let (phase, _title) = phase_from_state_update(record);
            (
                TimelineEntryKind::Progress,
                Some(phase),
                None,
                None,
                None,
                None,
                None,
            )
        }
        EventKind::HarnessEventNormalized { source_kind: _ } => {
            let delta = token_delta(record);
            if delta.is_some() {
                (
                    TimelineEntryKind::TokenUpdate,
                    Some(RunPhase::Active),
                    None,
                    None,
                    None,
                    None,
                    delta,
                )
            } else {
                (
                    TimelineEntryKind::Progress,
                    Some(RunPhase::Active),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
        }
        EventKind::OrchestratorWorkerStarted => (
            TimelineEntryKind::Progress,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::TaskGraphMilestoneCreated { milestone_id: _ }
        | EventKind::TaskGraphMilestoneUpdated { milestone_id: _ } => (
            TimelineEntryKind::File,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::TaskGraphIssueCreated { issue_id: _ }
        | EventKind::TaskGraphIssueUpdated { issue_id: _ } => (
            TimelineEntryKind::File,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::TaskGraphSubIssueCreated { .. } | EventKind::TaskGraphSubIssueUpdated { .. } => {
            (
                TimelineEntryKind::File,
                Some(RunPhase::Active),
                None,
                None,
                None,
                None,
                None,
            )
        }
        EventKind::TaskGraphRelationCreated { .. } => (
            TimelineEntryKind::File,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        EventKind::TaskGraphCommentCreated { .. } => (
            TimelineEntryKind::File,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
        _ => (
            TimelineEntryKind::Unknown,
            Some(RunPhase::Active),
            None,
            None,
            None,
            None,
            None,
        ),
    }
}

fn terminal_context(record: &EventRecord) -> (Option<String>, Option<String>) {
    let payload = record.payload.as_ref();
    let session_id = payload
        .and_then(|p| p.get("terminal_session_id").or_else(|| p.get("stream_id")))
        .and_then(|v| v.as_str())
        .map(String::from);
    let command_id = payload
        .and_then(|p| {
            p.get("association")
                .and_then(|a| a.get("command_id"))
                .or_else(|| p.get("command_id"))
        })
        .and_then(|v| v.as_str())
        .map(String::from);
    (command_id, session_id)
}

fn tool_name(record: &EventRecord) -> Option<String> {
    record
        .payload
        .as_ref()
        .and_then(|p| p.get("tool_name").and_then(|v| v.as_str()))
        .map(String::from)
}

fn phase_from_state_update(record: &EventRecord) -> (RunPhase, &'static str) {
    let status = record
        .payload
        .as_ref()
        .and_then(|p| {
            p.get("execution_status")
                .or_else(|| p.get("state"))
                .or_else(|| p.get("status"))
        })
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    match status.as_str() {
        "finished" | "completed" | "done" => (RunPhase::Completed, "Run completed"),
        "error" | "stuck" | "failed" => (RunPhase::Stalled, "Run stalled or failed"),
        "waiting_for_prior_turn" | "waiting_on_prior_turn" | "waiting" => {
            (RunPhase::Active, "Waiting on prior turn")
        }
        "running" | "in_progress" | "active" => (RunPhase::Active, "Running turn"),
        "paused" => (RunPhase::Quiet, "Run paused"),
        _ => (RunPhase::Active, "Progress update"),
    }
}

fn token_delta(record: &EventRecord) -> Option<TokenDelta> {
    let payload = record.payload.as_ref()?;
    let usage = payload.get("usage")?;
    let input = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_read = usage
        .get("cache_read_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if input == 0 && output == 0 && cache_read == 0 {
        return None;
    }
    Some(TokenDelta {
        input,
        output,
        cache_read,
    })
}

fn title_for(
    kind: TimelineEntryKind,
    phase: Option<RunPhase>,
    record: &EventRecord,
    tool_name: Option<&str>,
    command_id: Option<&str>,
) -> String {
    match kind {
        TimelineEntryKind::Phase => format!("Phase: {}", phase_to_label(phase)),
        TimelineEntryKind::ToolCall => {
            if let Some(name) = tool_name {
                format!("Tool: {name}")
            } else {
                "Tool call".into()
            }
        }
        TimelineEntryKind::Command => {
            if let Some(cmd) = command_id {
                format!("Command: {cmd}")
            } else {
                "Command".into()
            }
        }
        TimelineEntryKind::TokenUpdate => "Token update".into(),
        TimelineEntryKind::Reconnect => match &record.kind {
            EventKind::StreamConnected { .. } => "Stream connected".into(),
            EventKind::StreamDisconnected { .. } => "Stream disconnected".into(),
            EventKind::StreamReconnected { .. } => "Stream reconnected".into(),
            _ => format!("Reconnect: {}", phase_to_label(phase)),
        },
        TimelineEntryKind::StallProbe => format!("Stall: {}", phase_to_label(phase)),
        TimelineEntryKind::Progress => {
            if matches!(record.kind, EventKind::HarnessConversationStateUpdate) {
                let (_, title) = phase_from_state_update(record);
                title.into()
            } else {
                format!("Progress: {}", phase_to_label(phase))
            }
        }
        TimelineEntryKind::State => match &record.kind {
            EventKind::RunStarted => "Run started".into(),
            EventKind::RunCompleted => "Run completed".into(),
            EventKind::RunFailed => "Run failed".into(),
            EventKind::RunCancelled => "Run cancelled".into(),
            EventKind::OrchestratorRetryScheduled { .. } => "Retry queued".into(),
            EventKind::OrchestratorWorkerFailed { .. } => "Worker failed".into(),
            EventKind::GatewayActionFailed { .. } => "Action failed".into(),
            _ => format!("State: {}", phase_to_label(phase)),
        },
        TimelineEntryKind::Log => "Log".into(),
        TimelineEntryKind::Terminal => "Terminal output".into(),
        TimelineEntryKind::File => "Task graph / file".into(),
        TimelineEntryKind::Unknown => "Event".into(),
    }
}

fn phase_to_label(phase: Option<RunPhase>) -> &'static str {
    match phase {
        Some(RunPhase::Active) => "active",
        Some(RunPhase::Quiet) => "quiet",
        Some(RunPhase::Degraded) => "degraded",
        Some(RunPhase::Stalled) => "stalled",
        Some(RunPhase::RetryQueued) => "retry queued",
        Some(RunPhase::Cancelled) => "cancelled",
        Some(RunPhase::Detached) => "detached",
        Some(RunPhase::Completed) => "completed",
        None => "unknown",
    }
}

fn state_evidence(
    kind: TimelineEntryKind,
    phase: Option<RunPhase>,
    entry: &TimelineEntry,
    record: &EventRecord,
) -> Option<RunStateEvidence> {
    let phase = phase?;
    let stream = stream_health_for(kind, record);
    let explanation = match phase {
        RunPhase::Active => "Turn is actively executing or waiting on a prior turn.",
        RunPhase::Quiet => "Run is alive but no recent progress signal arrived.",
        RunPhase::Degraded => {
            "Stream is reachable but unhealthy; awaiting reconnect or reconciliation."
        }
        RunPhase::Stalled => "No progress observed within the idle timeout; run stalled.",
        RunPhase::RetryQueued => "Scheduler queued a retry after a worker outcome.",
        RunPhase::Cancelled => "Run was cancelled by user or system action.",
        RunPhase::Detached => "Worker could not stop the runtime; execution is detached.",
        RunPhase::Completed => "Run reached a terminal completed state.",
    };

    Some(RunStateEvidence {
        phase,
        stream,
        last_activity_at: Some(entry.happened_at),
        stall_deadline_at: None,
        explanation: format!("{explanation} ({})", record.summary),
    })
}

fn stream_health_for(kind: TimelineEntryKind, record: &EventRecord) -> RunStreamLiveness {
    match kind {
        TimelineEntryKind::Reconnect => match &record.kind {
            EventKind::StreamConnected { .. } => RunStreamLiveness::Healthy,
            EventKind::StreamReconnected { .. } => RunStreamLiveness::Stale,
            _ => RunStreamLiveness::Dead,
        },
        TimelineEntryKind::StallProbe => RunStreamLiveness::Dead,
        _ => RunStreamLiveness::Healthy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway_schema::{envelope::EntityRef, event_journal::EventRecord};

    fn run_record(seq: u64, kind: EventKind, summary: &str, payload: Option<Value>) -> EventRecord {
        EventRecord::builder()
            .event_id(format!("evt-{seq}"))
            .sequence(seq)
            .entity_ref(EntityRef::run("run-1"))
            .kind(kind)
            .summary(summary)
            .payload_or_none(payload)
            .build()
    }

    #[test]
    fn groups_waiting_on_prior_turn_and_running_turn() {
        let builder = TimelineBuilder::new("run-1");
        let records = vec![
            run_record(1, EventKind::RunStarted, "Run started", Some(Value::Null)),
            run_record(
                2,
                EventKind::HarnessConversationStateUpdate,
                "waiting",
                Some(serde_json::json!({ "execution_status": "waiting_for_prior_turn" })),
            ),
            run_record(
                3,
                EventKind::HarnessConversationStateUpdate,
                "running",
                Some(serde_json::json!({ "execution_status": "running" })),
            ),
            run_record(
                4,
                EventKind::HarnessToolCall,
                "tool call",
                Some(serde_json::json!({ "tool_name": "terminal" })),
            ),
            run_record(
                5,
                EventKind::HarnessToolResult,
                "tool result",
                Some(serde_json::json!({ "tool_name": "terminal" })),
            ),
            run_record(
                6,
                EventKind::RunCompleted,
                "Run completed",
                Some(Value::Null),
            ),
        ];
        let timeline = builder.build(&records);
        let kinds: Vec<_> = timeline.entries.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TimelineEntryKind::State,
                TimelineEntryKind::Progress,
                TimelineEntryKind::Progress,
                TimelineEntryKind::ToolCall,
                TimelineEntryKind::State,
            ]
        );
        assert_eq!(timeline.entries[1].phase, Some(RunPhase::Active));
        assert!(timeline.entries[1].title.to_lowercase().contains("waiting"));
        assert_eq!(timeline.entries[4].phase, Some(RunPhase::Completed));
    }

    #[test]
    fn groups_token_updates() {
        let builder = TimelineBuilder::new("run-1");
        let records = vec![
            run_record(
                1,
                EventKind::HarnessEventNormalized {
                    source_kind: "LLMCompletionLogEvent".into(),
                },
                "tokens",
                Some(serde_json::json!({ "usage": { "input_tokens": 100, "output_tokens": 50 } })),
            ),
            run_record(
                2,
                EventKind::HarnessEventNormalized {
                    source_kind: "LLMCompletionLogEvent".into(),
                },
                "more tokens",
                Some(serde_json::json!({ "usage": { "input_tokens": 10, "output_tokens": 20 } })),
            ),
        ];
        let timeline = builder.build(&records);
        assert_eq!(timeline.entries.len(), 1);
        let entry = &timeline.entries[0];
        assert_eq!(entry.kind, TimelineEntryKind::TokenUpdate);
        assert_eq!(
            entry.token_delta,
            Some(TokenDelta {
                input: 110,
                output: 70,
                cache_read: 0,
            })
        );
    }

    #[test]
    fn reconnect_events_grouped_as_reconnect() {
        let builder = TimelineBuilder::new("run-1");
        let records = vec![
            run_record(
                1,
                EventKind::StreamDisconnected {
                    client_id: "c1".into(),
                },
                "stream disconnected",
                None,
            ),
            run_record(
                2,
                EventKind::StreamReconnected {
                    client_id: "c1".into(),
                },
                "stream reconnected",
                None,
            ),
        ];
        let timeline = builder.build(&records);
        assert_eq!(timeline.entries.len(), 2);
        assert_eq!(timeline.entries[0].kind, TimelineEntryKind::Reconnect);
        assert_eq!(timeline.entries[0].phase, Some(RunPhase::Degraded));
        assert_eq!(timeline.entries[1].kind, TimelineEntryKind::Reconnect);
        assert_eq!(timeline.entries[1].phase, Some(RunPhase::Degraded));
    }

    #[test]
    fn stall_probe_explains_stalled_state() {
        let builder = TimelineBuilder::new("run-1");
        let records = vec![run_record(
            1,
            EventKind::OrchestratorWorkerFailed {
                reason: "idle timeout".into(),
            },
            "worker stalled",
            None,
        )];
        let timeline = builder.build(&records);
        let entry = &timeline.entries[0];
        assert_eq!(entry.kind, TimelineEntryKind::StallProbe);
        assert_eq!(entry.phase, Some(RunPhase::Stalled));
        assert!(entry.state_evidence.is_some());
        assert!(
            entry
                .state_evidence
                .as_ref()
                .expect("state_evidence")
                .explanation
                .contains("stalled")
        );
    }

    #[test]
    fn terminal_frames_grouped_by_session_and_command() {
        let builder = TimelineBuilder::new("run-1");
        let frame_payload = |cmd: &str| {
            serde_json::json!({
                "terminal_session_id": "term-1",
                "stream_id": "term-1",
                "content": "line",
                "association": {
                    "run_id": "run-1",
                    "workspace_id": "ws-1",
                    "command_id": cmd
                }
            })
        };
        let records = vec![
            run_record(
                1,
                EventKind::TerminalFrame {
                    frame_id: "f1".into(),
                },
                "frame 1",
                Some(frame_payload("cmd-a")),
            ),
            run_record(
                2,
                EventKind::TerminalFrame {
                    frame_id: "f2".into(),
                },
                "frame 2",
                Some(frame_payload("cmd-a")),
            ),
            run_record(
                3,
                EventKind::TerminalFrame {
                    frame_id: "f3".into(),
                },
                "frame 3",
                Some(frame_payload("cmd-b")),
            ),
        ];
        let timeline = builder.build(&records);
        assert_eq!(timeline.entries.len(), 2);
        assert_eq!(timeline.entries[0].command_id.as_deref(), Some("cmd-a"));
        assert_eq!(timeline.entries[0].event_ids.len(), 2);
        assert_eq!(timeline.entries[1].command_id.as_deref(), Some("cmd-b"));
    }
}
