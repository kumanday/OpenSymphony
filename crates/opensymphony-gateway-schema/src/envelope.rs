use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{cursor::StreamCursor, version::SchemaVersion};

/// Base envelope for every gateway event or snapshot stream item.
///
/// Carries versioning, cursor, and an optional raw payload so unknown
/// future event kinds can be forwarded without loss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayEnvelope {
    pub schema_version: SchemaVersion,
    pub cursor: StreamCursor,
    pub entity_ref: EntityRef,
    pub event_kind: String,
    /// Typed payload. Use `raw_payload` for events not yet mapped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Unmodified original payload for forward-compatibility and audit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<Value>,
    pub emitted_at: DateTime<Utc>,
}

/// Lightweight reference to an entity so every envelope is self-describing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef {
    /// Entity type discriminator.
    pub kind: EntityKind,
    /// Primary identifier (stable across local and hosted modes).
    pub id: String,
    /// Human-readable secondary identifier (e.g. "COE-390").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

/// Known entity kinds referenced by gateway schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Issue,
    SubIssue,
    Milestone,
    Run,
    Workspace,
    Conversation,
    TerminalSession,
    PlanningSession,
    Project,
    Repository,
    Agent,
    Harness,
    Unknown,
}

impl GatewayEnvelope {
    /// Create an envelope with a typed payload.
    ///
    /// `raw_payload` is set to a clone of `payload` so future schema evolutions
    /// can diverge without breaking round-trips.
    pub fn new(
        cursor: StreamCursor,
        entity_ref: EntityRef,
        event_kind: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            schema_version: SchemaVersion::default(),
            cursor,
            entity_ref,
            event_kind: event_kind.into(),
            raw_payload: Some(payload.clone()),
            payload: Some(payload),
            emitted_at: Utc::now(),
        }
    }

    /// Create an envelope for an unknown/unmapped event kind.
    ///
    /// `payload` is `None`; only `raw_payload` is populated so the gateway
    /// can forward future event kinds without forcing a typed parse.
    pub fn from_raw_payload(
        cursor: StreamCursor,
        entity_ref: EntityRef,
        event_kind: impl Into<String>,
        raw_payload: Value,
    ) -> Self {
        Self {
            schema_version: SchemaVersion::default(),
            cursor,
            entity_ref,
            event_kind: event_kind.into(),
            payload: None,
            raw_payload: Some(raw_payload),
            emitted_at: Utc::now(),
        }
    }
}

impl EntityRef {
    pub fn issue(id: impl Into<String>, identifier: Option<String>) -> Self {
        Self {
            kind: EntityKind::Issue,
            id: id.into(),
            identifier,
        }
    }

    pub fn run(id: impl Into<String>) -> Self {
        Self {
            kind: EntityKind::Run,
            id: id.into(),
            identifier: None,
        }
    }

    pub fn terminal(id: impl Into<String>) -> Self {
        Self {
            kind: EntityKind::TerminalSession,
            id: id.into(),
            identifier: None,
        }
    }
}
