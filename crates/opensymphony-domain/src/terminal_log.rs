use std::collections::HashMap;

use chrono::{DateTime, Utc};
use tracing::warn;

use crate::opensymphony_gateway_schema::{
    event_journal::{EventKind, EventRecord},
    terminal::{
        TerminalEncoding, TerminalFrame, TerminalFrameKind, TerminalLogAssociation,
        TerminalSession, TerminalSnapshot,
    },
    version::SchemaVersion,
};

/// In-memory accumulator for terminal/log frames associated with runs.
///
/// The store keeps frames ordered by frame sequence and exposes scrollback
/// reads, live stream frames, search, and jump-to-event. It is designed to be
/// populated from the event journal (terminal/log partition) so the same
/// frames are available after reconnect and replay.
#[derive(Debug, Clone, Default)]
pub struct TerminalLogStore {
    sessions: HashMap<String, TerminalSessionState>,
}

#[derive(Debug, Clone)]
struct TerminalSessionState {
    association: TerminalLogAssociation,
    frames: Vec<TerminalFrame>,
    total_bytes: u64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TerminalLogStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ingest a journal event record into the store.
    ///
    /// Only `TerminalFrame` and `LogEntry` events are processed; other kinds
    /// are ignored. The payload is expected to carry a `TerminalFrame` shape
    /// for terminal frames, or a `message`/`content` field for log entries.
    pub fn ingest_event_record(&mut self, record: &EventRecord) {
        match &record.kind {
            EventKind::TerminalFrame { frame_id } => {
                let payload = match record.payload.as_ref() {
                    Some(p) => p,
                    None => {
                        warn!(
                            event_id = %record.event_id,
                            sequence = record.sequence,
                            "terminal frame event missing payload; dropping"
                        );
                        return;
                    }
                };
                match serde_json::from_value::<TerminalFrame>(payload.clone()) {
                    Ok(mut frame) => {
                        if frame.frame_id.is_none() {
                            frame.frame_id = Some(frame_id.clone());
                        }
                        self.ingest_frame(frame, frame_id.clone());
                    }
                    Err(err) => {
                        warn!(
                            event_id = %record.event_id,
                            sequence = record.sequence,
                            error = %err,
                            "failed to deserialize terminal frame payload; dropping"
                        );
                    }
                }
            }
            EventKind::LogEntry { level } => {
                if let Some(frame) = log_entry_to_frame(record, level) {
                    self.ingest_frame(frame, String::new());
                }
            }
            _ => {}
        }
    }

    /// Directly ingest a terminal frame. Useful for tests and for callers that
    /// already have decoded frames.
    pub fn ingest_frame(&mut self, frame: TerminalFrame, frame_id: String) {
        let session = self
            .sessions
            .entry(frame.terminal_session_id.clone())
            .or_insert_with(|| TerminalSessionState {
                association: frame.association.clone(),
                frames: Vec::new(),
                total_bytes: 0,
                created_at: frame.timestamp,
                updated_at: frame.timestamp,
            });
        // Replay-safe deduplication: skip if a frame with the same frame_id
        // or source_event_id is already present.
        if !frame_id.is_empty()
            && session
                .frames
                .iter()
                .any(|f| f.frame_id.as_deref() == Some(frame_id.as_str()))
        {
            return;
        }
        if frame.source_event_id.as_ref().is_some_and(|eid| {
            session
                .frames
                .iter()
                .any(|f| f.source_event_id.as_deref() == Some(eid.as_str()))
        }) {
            return;
        }
        session.total_bytes = session
            .total_bytes
            .saturating_add(frame.content.len() as u64);
        session.updated_at = frame.timestamp;
        session.frames.push(frame);
    }

    /// Return a snapshot of a terminal session starting from `cursor` (frame
    /// sequence) and including at most `limit` frames.
    pub fn snapshot(
        &self,
        terminal_session_id: impl AsRef<str>,
        cursor: Option<u64>,
        limit: usize,
    ) -> TerminalSnapshot {
        let id = terminal_session_id.as_ref();
        let Some(session) = self.sessions.get(id) else {
            return empty_snapshot(id);
        };

        let start = cursor.unwrap_or(0);
        let total = session.frames.len() as u64;
        let filtered: Vec<&TerminalFrame> = session
            .frames
            .iter()
            .skip_while(|f| f.frame_sequence < start)
            .take(limit)
            .collect();
        let truncated = filtered.len() < session.frames.len();
        let new_cursor = filtered
            .last()
            .map(|f| f.frame_sequence.saturating_add(1))
            .unwrap_or(start);

        TerminalSnapshot {
            schema_version: SchemaVersion::v1(),
            terminal_session_id: id.to_string(),
            run_id: session.association.run_id.clone(),
            frames: filtered.into_iter().cloned().collect(),
            total_frames: total,
            truncated,
            cursor: new_cursor,
            session: Some(TerminalSession {
                schema_version: SchemaVersion::v1(),
                terminal_session_id: id.to_string(),
                run_id: session.association.run_id.clone(),
                association: session.association.clone(),
                frame_count: total,
                total_bytes: session.total_bytes,
                created_at: session.created_at,
                updated_at: session.updated_at,
                current_cursor: total.saturating_add(1),
            }),
        }
    }

    /// Search all frames in a session for `query` and return matching frame
    /// sequences with a short snippet.
    pub fn search(
        &self,
        terminal_session_id: impl AsRef<str>,
        query: impl AsRef<str>,
    ) -> Vec<(u64, DateTime<Utc>, String)> {
        let id = terminal_session_id.as_ref();
        let query = query.as_ref().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let Some(session) = self.sessions.get(id) else {
            return Vec::new();
        };

        session
            .frames
            .iter()
            .filter_map(|f| {
                if f.content.to_lowercase().contains(&query) {
                    Some((f.frame_sequence, f.timestamp, snippet(&f.content, &query)))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Jump to the frame sequence associated with a journal event id.
    ///
    /// Frames that carry a `source_event_id` matching `event_id` are returned
    /// directly; otherwise the search falls back to the first frame whose
    /// content includes the event id.
    pub fn jump_to_event(
        &self,
        terminal_session_id: impl AsRef<str>,
        event_id: impl AsRef<str>,
    ) -> Option<u64> {
        let id = terminal_session_id.as_ref();
        let event_id = event_id.as_ref();
        let session = self.sessions.get(id)?;

        session
            .frames
            .iter()
            .find(|f| {
                f.source_event_id
                    .as_ref()
                    .is_some_and(|sid| sid == event_id)
            })
            .map(|f| f.frame_sequence)
            .or_else(|| {
                session
                    .frames
                    .iter()
                    .find(|f| f.content.contains(event_id))
                    .map(|f| f.frame_sequence)
            })
    }

    /// List all terminal session ids associated with a run id.
    pub fn sessions_for_run(&self, run_id: impl AsRef<str>) -> Vec<String> {
        let run_id = run_id.as_ref();
        self.sessions
            .iter()
            .filter_map(|(sid, state)| {
                if state.association.run_id == run_id {
                    Some(sid.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Return the association context for a session, if known.
    pub fn association(
        &self,
        terminal_session_id: impl AsRef<str>,
    ) -> Option<TerminalLogAssociation> {
        self.sessions
            .get(terminal_session_id.as_ref())
            .map(|s| s.association.clone())
    }

    /// Return all frames for a session without pagination.
    #[cfg(test)]
    pub fn all_frames(&self, terminal_session_id: impl AsRef<str>) -> Vec<TerminalFrame> {
        self.sessions
            .get(terminal_session_id.as_ref())
            .map(|s| s.frames.clone())
            .unwrap_or_default()
    }
}

fn empty_snapshot(terminal_session_id: impl AsRef<str>) -> TerminalSnapshot {
    let id = terminal_session_id.as_ref().to_string();
    TerminalSnapshot {
        schema_version: SchemaVersion::v1(),
        terminal_session_id: id.clone(),
        run_id: String::new(),
        frames: Vec::new(),
        total_frames: 0,
        truncated: false,
        cursor: 0,
        session: None,
    }
}

fn log_entry_to_frame(record: &EventRecord, _level: &str) -> Option<TerminalFrame> {
    let payload = record.payload.as_ref()?;
    let content = payload
        .get("message")
        .or_else(|| payload.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or(&record.summary)
        .to_string();
    let session_id = payload
        .get("terminal_session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    let run_id = payload
        .get("run_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let workspace_id = payload
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let command_id = payload
        .get("command_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let issue_id = payload
        .get("issue_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let sub_issue_id = payload
        .get("sub_issue_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    let harness_session_id = payload
        .get("harness_session_id")
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned);
    // Support nested association payload as a fallback.
    let association = payload.get("association");
    let issue_id = issue_id.or_else(|| {
        association
            .and_then(|a| a.get("issue_id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    });
    let sub_issue_id = sub_issue_id.or_else(|| {
        association
            .and_then(|a| a.get("sub_issue_id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    });
    let command_id = command_id.or_else(|| {
        association
            .and_then(|a| a.get("command_id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    });
    let harness_session_id = harness_session_id.or_else(|| {
        association
            .and_then(|a| a.get("harness_session_id"))
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned)
    });

    Some(TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: record.sequence,
        stream_id: session_id.clone(),
        run_id: run_id.clone(),
        terminal_session_id: session_id,
        frame_kind: TerminalFrameKind::Log,
        encoding: TerminalEncoding::Utf8,
        content,
        timestamp: record.happened_at,
        association: TerminalLogAssociation {
            run_id,
            workspace_id,
            command_id,
            issue_id,
            sub_issue_id,
            harness_session_id,
        },
        correlation_id: record.correlation_id.clone(),
        source_event_id: Some(record.event_id.clone()),
        frame_id: Some(record.event_id.clone()),
    })
}

fn snippet(text: &str, query: &str) -> String {
    let lower = text.to_lowercase();
    let Some(pos) = lower.find(query) else {
        return text.chars().take(120).collect();
    };
    let start = pos.saturating_sub(40);
    let end = (pos + query.len() + 40).min(text.len());
    text[start..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway_schema::event_journal::EventRecord;

    fn sample_frame(
        seq: u64,
        session: &str,
        content: &str,
        command: Option<&str>,
    ) -> TerminalFrame {
        TerminalFrame {
            schema_version: SchemaVersion::v1(),
            frame_sequence: seq,
            stream_id: session.into(),
            run_id: "run-1".into(),
            terminal_session_id: session.into(),
            frame_kind: TerminalFrameKind::Stdout,
            encoding: TerminalEncoding::Utf8,
            content: content.into(),
            timestamp: Utc::now(),
            association: TerminalLogAssociation {
                run_id: "run-1".into(),
                workspace_id: "ws-1".into(),
                command_id: command.map(String::from),
                issue_id: Some("iss-1".into()),
                sub_issue_id: Some("sub-1".into()),
                harness_session_id: Some("harness-1".into()),
            },
            correlation_id: None,
            source_event_id: Some(format!("evt-{seq}")),
            frame_id: Some(format!("fid-{seq}")),
        }
    }

    #[test]
    fn ingest_frame_and_snapshot() {
        let mut store = TerminalLogStore::new();
        store.ingest_frame(sample_frame(1, "term-1", "hello", None), "fid-1".into());
        store.ingest_frame(sample_frame(2, "term-1", "world", None), "fid-2".into());

        let snap = store.snapshot("term-1", None, 10);
        assert_eq!(snap.total_frames, 2);
        assert_eq!(snap.frames.len(), 2);
        assert!(snap.session.is_some());
        let session = snap.session.expect("session");
        assert_eq!(session.terminal_session_id, "term-1");
        assert_eq!(session.association.run_id, "run-1");
        assert_eq!(session.association.issue_id.as_deref(), Some("iss-1"));
    }

    #[test]
    fn search_finds_matching_frames() {
        let mut store = TerminalLogStore::new();
        store.ingest_frame(sample_frame(1, "term-1", "alpha", None), "fid-1".into());
        store.ingest_frame(sample_frame(2, "term-1", "beta", None), "fid-2".into());
        store.ingest_frame(sample_frame(3, "term-1", "alphabet", None), "fid-3".into());

        let matches = store.search("term-1", "alpha");
        let seqs: Vec<_> = matches.iter().map(|(seq, _, _)| *seq).collect();
        assert_eq!(seqs, vec![1, 3]);
    }

    #[test]
    fn jump_to_event_uses_source_event_id() {
        let mut store = TerminalLogStore::new();
        store.ingest_frame(sample_frame(5, "term-1", "output", None), "fid-5".into());

        assert_eq!(store.jump_to_event("term-1", "evt-5"), Some(5));
        assert_eq!(store.jump_to_event("term-1", "missing"), None);
    }

    #[test]
    fn replay_same_frame_is_deduplicated() {
        let mut store = TerminalLogStore::new();
        let mut frame = sample_frame(1, "term-1", "hello", None);
        frame.frame_id = Some("fid-1".into());
        store.ingest_frame(frame.clone(), "fid-1".into());
        store.ingest_frame(frame.clone(), "fid-1".into());
        let mut distinct = sample_frame(1, "term-1", "hello", None);
        distinct.frame_id = Some("different-frame-id".into());
        distinct.source_event_id = Some("different-event".into());
        store.ingest_frame(distinct, "different-frame-id".into());

        let snap = store.snapshot("term-1", None, 10);
        assert_eq!(snap.total_frames, 2);
    }

    #[test]
    fn replay_same_event_for_log_entry_is_deduplicated() {
        let mut store = TerminalLogStore::new();
        let record = EventRecord::builder()
            .event_id("log-1")
            .sequence(1)
            .kind(EventKind::LogEntry {
                level: "info".into(),
            })
            .payload(serde_json::json!({
                "message": "build started",
                "terminal_session_id": "term-1",
                "run_id": "run-1",
                "workspace_id": "ws-1",
                "command_id": "cmd-1",
                "issue_id": "iss-1",
                "sub_issue_id": "sub-1",
                "harness_session_id": "harness-1",
            }))
            .summary("build started")
            .build();
        store.ingest_event_record(&record);
        store.ingest_event_record(&record);

        let snap = store.snapshot("term-1", None, 10);
        assert_eq!(snap.total_frames, 1);
        let frame = snap.frames.first().expect("frame");
        assert_eq!(frame.frame_kind, TerminalFrameKind::Log);
        assert_eq!(frame.association.command_id.as_deref(), Some("cmd-1"));
        assert_eq!(frame.association.issue_id.as_deref(), Some("iss-1"));
        assert_eq!(frame.association.sub_issue_id.as_deref(), Some("sub-1"));
        assert_eq!(
            frame.association.harness_session_id.as_deref(),
            Some("harness-1")
        );
    }

    #[test]
    fn sessions_for_run_filters_by_run_id() {
        let mut store = TerminalLogStore::new();
        let mut f = sample_frame(1, "term-a", "x", None);
        f.association.run_id = "run-a".into();
        f.run_id = "run-a".into();
        store.ingest_frame(f, "fid".into());
        store.ingest_frame(sample_frame(1, "term-b", "y", None), "fid".into());

        assert_eq!(store.sessions_for_run("run-a"), vec!["term-a"]);
        assert_eq!(store.sessions_for_run("run-1"), vec!["term-b"]);
    }

    #[test]
    fn ingest_log_entry_record() {
        let mut store = TerminalLogStore::new();
        let record = EventRecord::builder()
            .event_id("log-1")
            .sequence(1)
            .kind(EventKind::LogEntry {
                level: "info".into(),
            })
            .payload(serde_json::json!({
                "message": "build started",
                "terminal_session_id": "term-1",
                "run_id": "run-1",
                "workspace_id": "ws-1",
                "command_id": "cmd-1"
            }))
            .summary("build started")
            .build();
        store.ingest_event_record(&record);

        let snap = store.snapshot("term-1", None, 10);
        assert_eq!(snap.total_frames, 1);
        let frame = snap.frames.first().expect("frame");
        assert_eq!(frame.frame_kind, TerminalFrameKind::Log);
        assert_eq!(frame.association.command_id.as_deref(), Some("cmd-1"));
    }

    #[test]
    fn log_entry_payload_extracts_association_fields() {
        let mut store = TerminalLogStore::new();
        let record = EventRecord::builder()
            .event_id("log-2")
            .sequence(2)
            .kind(EventKind::LogEntry {
                level: "info".into(),
            })
            .payload(serde_json::json!({
                "message": "nested association",
                "terminal_session_id": "term-1",
                "association": {
                    "run_id": "run-1",
                    "workspace_id": "ws-1",
                    "command_id": "cmd-2",
                    "issue_id": "iss-2",
                    "sub_issue_id": "sub-2",
                    "harness_session_id": "harness-2"
                }
            }))
            .summary("nested association")
            .build();
        store.ingest_event_record(&record);

        let snap = store.snapshot("term-1", None, 10);
        assert_eq!(snap.total_frames, 1);
        let frame = snap.frames.first().expect("frame");
        assert_eq!(frame.association.command_id.as_deref(), Some("cmd-2"));
        assert_eq!(frame.association.issue_id.as_deref(), Some("iss-2"));
        assert_eq!(frame.association.sub_issue_id.as_deref(), Some("sub-2"));
        assert_eq!(
            frame.association.harness_session_id.as_deref(),
            Some("harness-2")
        );
    }

    #[test]
    fn malformed_terminal_frame_payload_is_logged_and_dropped() {
        let mut store = TerminalLogStore::new();
        let record = EventRecord::builder()
            .event_id("bad-frame-1")
            .sequence(1)
            .kind(EventKind::TerminalFrame {
                frame_id: "fid-bad-1".into(),
            })
            .payload(serde_json::json!({
                "unexpected_shape": true,
            }))
            .summary("malformed terminal frame")
            .build();
        store.ingest_event_record(&record);

        assert!(store.sessions.is_empty());
    }

    #[test]
    fn terminal_frame_event_missing_payload_is_logged_and_dropped() {
        let mut store = TerminalLogStore::new();
        let record = EventRecord::builder()
            .event_id("no-payload-1")
            .sequence(1)
            .kind(EventKind::TerminalFrame {
                frame_id: "fid-none-1".into(),
            })
            .summary("no payload")
            .build();
        store.ingest_event_record(&record);

        assert!(store.sessions.is_empty());
    }
}
