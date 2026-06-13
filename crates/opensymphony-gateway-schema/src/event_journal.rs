//! Event journal domain types for durable event records and replayable streams.
//!
//! The journal records state transitions from gateway, orchestrator, task graph,
//! run lifecycle, terminal/log, approval, and planning activity. Each record carries
//! a stable ID, monotonic sequence number, schema version, actor, correlation ID,
//! entity references, timestamp, summary, payload, and optional raw payload reference.
//!
//! Cursors enable clients to resume from a specific sequence position and receive
//! committed events in order. Unknown harness payloads are retained through raw
//! JSON references for diagnostics without requiring schema evolution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::{cursor::StreamCursor, envelope::EntityRef, version::SchemaVersion};

/// Stable event identifier, generated as UUID v4.
pub type EventId = String;

/// Actor who produced the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventActor {
    System { id: String },
    User { id: String },
    Agent { id: String },
    Harness { id: String },
}

impl EventActor {
    pub fn system(id: impl Into<String>) -> Self {
        Self::System { id: id.into() }
    }

    pub fn user(id: impl Into<String>) -> Self {
        Self::User { id: id.into() }
    }

    pub fn agent(id: impl Into<String>) -> Self {
        Self::Agent { id: id.into() }
    }

    pub fn harness(id: impl Into<String>) -> Self {
        Self::Harness { id: id.into() }
    }

    /// Return the actor kind label (matches `kind` tag).
    pub fn kind_label(&self) -> &str {
        match self {
            Self::System { .. } => "system",
            Self::User { .. } => "user",
            Self::Agent { .. } => "agent",
            Self::Harness { .. } => "harness",
        }
    }

    /// Return the actor identifier.
    pub fn actor_id(&self) -> &str {
        match self {
            Self::System { id } => id,
            Self::User { id } => id,
            Self::Agent { id } => id,
            Self::Harness { id } => id,
        }
    }
}

/// High-level event kind discriminator for gateway event journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    // Orchestrator events
    OrchestratorStateTransition {
        from: String,
        to: String,
    },
    OrchestratorWorkerStarted,
    OrchestratorWorkerCompleted {
        outcome: String,
    },
    OrchestratorWorkerFailed {
        reason: String,
    },
    OrchestratorRetryScheduled {
        attempt: u32,
    },

    // Gateway action events
    GatewayActionDispatched {
        action: String,
    },
    GatewayActionCompleted {
        action: String,
    },
    GatewayActionFailed {
        action: String,
        reason: String,
    },

    // Normalized harness events
    HarnessEventNormalized {
        source_kind: String,
    },
    HarnessSseEvent,
    HarnessToolCall,
    HarnessToolResult,
    HarnessConversationStateUpdate,

    // Run lifecycle
    RunStarted,
    RunCompleted,
    RunFailed,
    RunCancelled,

    // Terminal / log events
    TerminalFrame {
        frame_id: String,
    },
    LogEntry {
        level: String,
    },

    // Approval events
    ApprovalRequested,
    ApprovalGranted,
    ApprovalDenied,

    // Planning events
    PlanningSessionStarted,
    PlanningMessage,
    PlanningArtifactGenerated,

    // Stream connection events
    StreamConnected {
        client_id: String,
    },
    StreamDisconnected {
        client_id: String,
    },
    StreamReconnected {
        client_id: String,
    },

    // Task graph mutation events (mirrors `/api/v1/taskgraph/*`).
    // Each variant carries the Linear node id so callers can correlate the
    // mutation back to the cached task graph entry plus the gateway
    // `correlation_id` propagated by the envelope.
    TaskGraphMilestoneCreated {
        milestone_id: String,
    },
    TaskGraphMilestoneUpdated {
        milestone_id: String,
    },
    TaskGraphIssueCreated {
        issue_id: String,
    },
    TaskGraphIssueUpdated {
        issue_id: String,
    },
    TaskGraphSubIssueCreated {
        sub_issue_id: String,
        parent_identifier: String,
    },
    TaskGraphSubIssueUpdated {
        sub_issue_id: String,
    },
    TaskGraphRelationCreated {
        relation_id: String,
        relation_type: String,
    },
    TaskGraphCommentCreated {
        comment_id: String,
        issue_id: String,
    },

    // Catch-all for unknown future events
    Unknown {
        raw_kind: String,
    },
}

impl EventKind {
    /// Serialize the event kind to a dotted string for the envelope.
    pub fn kind_tag(&self) -> String {
        match self {
            Self::OrchestratorStateTransition { .. } => "orchestrator.state_transition".into(),
            Self::OrchestratorWorkerStarted => "orchestrator.worker_started".into(),
            Self::OrchestratorWorkerCompleted { .. } => "orchestrator.worker_completed".into(),
            Self::OrchestratorWorkerFailed { .. } => "orchestrator.worker_failed".into(),
            Self::OrchestratorRetryScheduled { .. } => "orchestrator.retry_scheduled".into(),
            Self::GatewayActionDispatched { .. } => "gateway.action_dispatched".into(),
            Self::GatewayActionCompleted { .. } => "gateway.action_completed".into(),
            Self::GatewayActionFailed { .. } => "gateway.action_failed".into(),
            Self::HarnessEventNormalized { .. } => "harness.event_normalized".into(),
            Self::HarnessSseEvent => "harness.sse_event".into(),
            Self::HarnessToolCall => "harness.tool_call".into(),
            Self::HarnessToolResult => "harness.tool_result".into(),
            Self::HarnessConversationStateUpdate => "harness.conversation_state_update".into(),
            Self::RunStarted => "run.started".into(),
            Self::RunCompleted => "run.completed".into(),
            Self::RunFailed => "run.failed".into(),
            Self::RunCancelled => "run.cancelled".into(),
            Self::TerminalFrame { .. } => "terminal.frame".into(),
            Self::LogEntry { .. } => "log.entry".into(),
            Self::ApprovalRequested => "approval.requested".into(),
            Self::ApprovalGranted => "approval.granted".into(),
            Self::ApprovalDenied => "approval.denied".into(),
            Self::PlanningSessionStarted => "planning.session_started".into(),
            Self::PlanningMessage => "planning.message".into(),
            Self::PlanningArtifactGenerated => "planning.artifact_generated".into(),
            Self::StreamConnected { .. } => "stream.connected".into(),
            Self::StreamDisconnected { .. } => "stream.disconnected".into(),
            Self::StreamReconnected { .. } => "stream.reconnected".into(),
            Self::TaskGraphMilestoneCreated { .. } => "task_graph.milestone_created".into(),
            Self::TaskGraphMilestoneUpdated { .. } => "task_graph.milestone_updated".into(),
            Self::TaskGraphIssueCreated { .. } => "task_graph.issue_created".into(),
            Self::TaskGraphIssueUpdated { .. } => "task_graph.issue_updated".into(),
            Self::TaskGraphSubIssueCreated { .. } => "task_graph.sub_issue_created".into(),
            Self::TaskGraphSubIssueUpdated { .. } => "task_graph.sub_issue_updated".into(),
            Self::TaskGraphRelationCreated { .. } => "task_graph.relation_created".into(),
            Self::TaskGraphCommentCreated { .. } => "task_graph.comment_created".into(),
            Self::Unknown { raw_kind } => raw_kind.clone(),
        }
    }

    /// Whether this event is high-volume (terminal/log frames).
    pub fn is_high_volume(&self) -> bool {
        matches!(self, Self::TerminalFrame { .. } | Self::LogEntry { .. })
    }

    /// Partition name for this event kind.
    pub fn default_partition(&self) -> &str {
        if self.is_high_volume() {
            "terminal_log"
        } else {
            "events"
        }
    }
}

/// Durable event record stored in the event journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    /// Stable event ID (UUID v4).
    pub event_id: EventId,
    /// Monotonic sequence number within the partition.
    pub sequence: u64,
    /// Schema version of the event payload.
    pub schema_version: SchemaVersion,
    /// Actor who produced the event.
    pub actor: EventActor,
    /// Correlation ID linking related events (e.g., action dispatch + completion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<EventId>,
    /// Entity references associated with this event.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entity_refs: Vec<EntityRef>,
    /// Wall-clock timestamp when the event occurred.
    pub happened_at: DateTime<Utc>,
    /// Human-readable summary.
    pub summary: String,
    /// Typed event kind discriminator.
    pub kind: EventKind,
    /// Typed payload when the kind is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Raw harness payload reference for forward compatibility and diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_payload_ref: Option<String>,
}

impl EventRecord {
    /// Create a new event record with a fresh UUID and auto-generated timestamp.
    pub fn builder() -> EventRecordBuilder {
        EventRecordBuilder::default()
    }

    /// Build a cursor for replaying events after this one.
    pub fn next_cursor(&self, partition: impl Into<String>) -> StreamCursor {
        StreamCursor::new(self.sequence, partition)
    }

    /// Check if this event is a duplicate by comparing event_id.
    pub fn is_duplicate_of(&self, other: &EventRecord) -> bool {
        self.event_id == other.event_id
    }

    /// Whether raw payload is retained (for diagnostics on unknown harness events).
    pub fn has_raw_payload(&self) -> bool {
        self.raw_payload_ref.is_some()
    }
}

/// Builder for constructing EventRecord instances.
#[derive(Debug, Default, Clone)]
pub struct EventRecordBuilder {
    event_id: Option<EventId>,
    sequence: Option<u64>,
    schema_version: Option<SchemaVersion>,
    actor: Option<EventActor>,
    correlation_id: Option<EventId>,
    entity_refs: Vec<EntityRef>,
    happened_at: Option<DateTime<Utc>>,
    summary: String,
    kind: Option<EventKind>,
    payload: Option<Value>,
    raw_payload_ref: Option<String>,
}

impl EventRecordBuilder {
    pub fn event_id(mut self, id: impl Into<EventId>) -> Self {
        self.event_id = Some(id.into());
        self
    }

    pub fn sequence(mut self, seq: u64) -> Self {
        self.sequence = Some(seq);
        self
    }

    pub fn schema_version(mut self, sv: SchemaVersion) -> Self {
        self.schema_version = Some(sv);
        self
    }

    pub fn actor(mut self, actor: EventActor) -> Self {
        self.actor = Some(actor);
        self
    }

    pub fn correlation_id(mut self, id: impl Into<EventId>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    pub fn correlation_id_opt(mut self, id: Option<EventId>) -> Self {
        self.correlation_id = id;
        self
    }

    pub fn entity_ref(mut self, ref_: EntityRef) -> Self {
        self.entity_refs.push(ref_);
        self
    }

    pub fn entity_refs(mut self, refs: Vec<EntityRef>) -> Self {
        self.entity_refs = refs;
        self
    }

    pub fn happened_at(mut self, ts: DateTime<Utc>) -> Self {
        self.happened_at = Some(ts);
        self
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = summary.into();
        self
    }

    pub fn kind(mut self, kind: EventKind) -> Self {
        self.kind = Some(kind);
        self
    }

    pub fn payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn payload_or_none(mut self, payload: Option<Value>) -> Self {
        self.payload = payload;
        self
    }

    pub fn raw_payload_ref(mut self, ref_: impl Into<String>) -> Self {
        self.raw_payload_ref = Some(ref_.into());
        self
    }

    /// Finalize the builder and return the EventRecord.
    pub fn build(self) -> EventRecord {
        EventRecord {
            event_id: self.event_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            sequence: self.sequence.unwrap_or(0),
            schema_version: self.schema_version.unwrap_or_default(),
            actor: self.actor.unwrap_or_else(|| EventActor::system("system")),
            correlation_id: self.correlation_id,
            entity_refs: self.entity_refs,
            happened_at: self.happened_at.unwrap_or_else(Utc::now),
            summary: self.summary,
            kind: self.kind.unwrap_or_else(|| EventKind::Unknown {
                raw_kind: "unspecified".into(),
            }),
            payload: self.payload,
            raw_payload_ref: self.raw_payload_ref,
        }
    }
}

/// Paged event response for cursor-based queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPage {
    pub schema_version: SchemaVersion,
    pub events: Vec<EventRecord>,
    pub next_cursor: Option<StreamCursor>,
    pub has_more: bool,
}

/// Error types for journal operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JournalError {
    Backpressure { capacity: usize },
    NotFound { event_id: String },
    InvalidCursor { reason: String },
    PartitionNotFound { partition: String },
}

/// Journal health status for monitoring.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalHealth {
    pub status: JournalHealthStatus,
    pub capacity: usize,
    pub used: usize,
    pub oldest_sequence: Option<u64>,
    pub newest_sequence: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalHealthStatus {
    Healthy,
    NearCapacity,
    AtCapacity,
    Backpressured,
}

/// Connection state reported to stream clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamConnectionState {
    pub connected: bool,
    pub backpressure_active: bool,
    pub last_sequence: Option<u64>,
    pub error: Option<StreamError>,
}

/// Stream error reported to clients when degraded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamError {
    pub error_type: StreamErrorType,
    pub message: String,
    pub recoverable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamErrorType {
    Backpressure,
    Disconnected,
    CursorNotFound,
    ServerError,
}

impl StreamError {
    pub fn backpressure() -> Self {
        Self {
            error_type: StreamErrorType::Backpressure,
            message: "Stream backpressure active; delivery delayed".into(),
            recoverable: true,
        }
    }

    pub fn disconnected(reason: impl Into<String>) -> Self {
        Self {
            error_type: StreamErrorType::Disconnected,
            message: reason.into(),
            recoverable: true,
        }
    }

    pub fn cursor_not_found(cursor_seq: u64) -> Self {
        Self {
            error_type: StreamErrorType::CursorNotFound,
            message: format!(
                "Cursor sequence {} not found; oldest available events may have been evicted",
                cursor_seq
            ),
            recoverable: true,
        }
    }

    pub fn server_error(message: impl Into<String>) -> Self {
        Self {
            error_type: StreamErrorType::ServerError,
            message: message.into(),
            recoverable: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_record_roundtrip_serialization() {
        let record = EventRecord::builder()
            .event_id("evt_test_001")
            .sequence(42)
            .actor(EventActor::agent("agent-1"))
            .kind(EventKind::RunStarted)
            .summary("Run started for COE-393")
            .entity_ref(EntityRef::run("run_123"))
            .payload(json!({ "run_id": "run_123" }))
            .build();

        let json = serde_json::to_string(&record).expect("serialize");
        let deserialized: EventRecord = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(record, deserialized);
        assert_eq!(deserialized.event_id, "evt_test_001");
        assert_eq!(deserialized.sequence, 42);
    }

    #[test]
    fn raw_payload_ref_is_retained_for_unknown_harness_events() {
        let record = EventRecord::builder()
            .event_id("evt_raw_001")
            .sequence(10)
            .actor(EventActor::harness("openhands-1"))
            .kind(EventKind::Unknown {
                raw_kind: "custom_harness_event".into(),
            })
            .summary("Unknown harness event")
            .raw_payload_ref("raw_evt_abc123")
            .build();

        assert!(record.has_raw_payload());
        assert_eq!(record.raw_payload_ref, Some("raw_evt_abc123".into()));
    }

    #[test]
    fn duplicate_detection_by_event_id() {
        let a = EventRecord::builder()
            .event_id("evt_dup")
            .sequence(1)
            .kind(EventKind::RunStarted)
            .summary("A")
            .build();

        let b = EventRecord::builder()
            .event_id("evt_dup")
            .sequence(2)
            .kind(EventKind::RunCompleted)
            .summary("B")
            .build();

        assert!(a.is_duplicate_of(&b));
        assert!(b.is_duplicate_of(&a));
    }

    #[test]
    fn unique_events_are_not_duplicates() {
        let a = EventRecord::builder()
            .event_id("evt_a")
            .sequence(1)
            .kind(EventKind::RunStarted)
            .summary("A")
            .build();

        let b = EventRecord::builder()
            .event_id("evt_b")
            .sequence(2)
            .kind(EventKind::RunCompleted)
            .summary("B")
            .build();

        assert!(!a.is_duplicate_of(&b));
    }

    #[test]
    fn high_volume_events_use_terminal_log_partition() {
        assert!(
            EventKind::TerminalFrame {
                frame_id: "f1".into()
            }
            .is_high_volume()
        );
        assert!(
            EventKind::LogEntry {
                level: "info".into()
            }
            .is_high_volume()
        );
        assert!(!EventKind::RunStarted.is_high_volume());

        assert_eq!(
            EventKind::TerminalFrame {
                frame_id: "f1".into()
            }
            .default_partition(),
            "terminal_log"
        );
        assert_eq!(EventKind::RunStarted.default_partition(), "events");
    }

    #[test]
    fn event_actor_serialization() {
        let actor = EventActor::agent("agent-1");
        let json = serde_json::to_string(&actor).expect("serialize");
        let parsed: EventActor = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(actor, parsed);
        assert_eq!(actor.kind_label(), "agent");
        assert_eq!(actor.actor_id(), "agent-1");
    }

    #[test]
    fn next_cursor_advances_sequence() {
        let record = EventRecord::builder()
            .event_id("evt_1")
            .sequence(5)
            .kind(EventKind::RunStarted)
            .summary("test")
            .build();

        let cursor = record.next_cursor("events");
        assert_eq!(cursor.sequence, 5);
        assert_eq!(cursor.partition, "events");
    }

    #[test]
    fn event_page_serialization() {
        let events = vec![
            EventRecord::builder()
                .event_id("evt_1")
                .sequence(1)
                .kind(EventKind::RunStarted)
                .summary("started")
                .build(),
            EventRecord::builder()
                .event_id("evt_2")
                .sequence(2)
                .kind(EventKind::RunCompleted)
                .summary("completed")
                .build(),
        ];

        let page = EventPage {
            schema_version: SchemaVersion::v1(),
            events,
            next_cursor: Some(StreamCursor::new(3, "events")),
            has_more: true,
        };

        let json = serde_json::to_string(&page).expect("serialize");
        let deserialized: EventPage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(page, deserialized);
        assert!(deserialized.has_more);
    }

    #[test]
    fn correlation_id_links_related_events() {
        let correlation = "act_dispatch_001";
        let dispatched = EventRecord::builder()
            .event_id("evt_dispatch")
            .sequence(1)
            .correlation_id(correlation)
            .kind(EventKind::GatewayActionDispatched {
                action: "retry".into(),
            })
            .summary("Action dispatched")
            .build();

        let completed = EventRecord::builder()
            .event_id("evt_completed")
            .sequence(2)
            .correlation_id(correlation)
            .kind(EventKind::GatewayActionCompleted {
                action: "retry".into(),
            })
            .summary("Action completed")
            .build();

        assert_eq!(dispatched.correlation_id, completed.correlation_id);
        assert_eq!(dispatched.correlation_id, Some(correlation.into()));
    }

    #[test]
    fn builder_generates_uuid_when_event_id_not_set() {
        let a = EventRecord::builder()
            .sequence(1)
            .kind(EventKind::RunStarted)
            .summary("A")
            .build();

        let b = EventRecord::builder()
            .sequence(2)
            .kind(EventKind::RunStarted)
            .summary("B")
            .build();

        assert!(!a.is_duplicate_of(&b));
        assert!(!a.event_id.is_empty());
        assert!(!b.event_id.is_empty());
    }

    #[test]
    fn stream_error_serialization() {
        let err = StreamError::backpressure();
        assert!(err.recoverable);
        assert_eq!(err.error_type, StreamErrorType::Backpressure);

        let err2 = StreamError::cursor_not_found(42);
        assert!(err2.message.contains("42"));

        let err3 = StreamError::server_error("crash");
        assert!(!err3.recoverable);
    }
}
