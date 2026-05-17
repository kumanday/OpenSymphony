use serde::{Deserialize, Serialize};

use super::envelope::EntityKind;
use super::version::SchemaVersion;

/// Generic action dispatch payload accepted by
/// `POST /api/v1/actions/dispatch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDispatch {
    pub schema_version: SchemaVersion,
    pub correlation_id: String,
    pub action_kind: ActionKind,
    pub target_entity: ActionTarget,
    pub payload: Option<serde_json::Value>,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Retry,
    Cancel,
    Pause,
    Resume,
    Rehydrate,
    Comment,
    TransitionIssue,
    CreateFollowup,
    ApprovalDecision,
    PublishPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionTarget {
    pub entity_kind: EntityKind,
    pub entity_id: String,
}
