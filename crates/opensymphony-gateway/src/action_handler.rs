use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use serde_json::json;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::opensymphony_domain::{
    ControlPlaneDaemonSnapshot, ControlPlaneIssueRuntimeState, ControlPlaneIssueSnapshot,
    InMemoryEventJournal, SnapshotEnvelope,
};
use crate::opensymphony_gateway_schema::{
    action::{ActionDispatch, ActionKind, ActionReceipt, ActionStatus, PermissionResult},
    envelope::{EntityKind, EntityRef},
    event_journal::{EventActor, EventKind, EventRecord},
    run::RunAction,
};

pub struct ValidatedAction {
    pub action_id: String,
    pub receipt: ActionReceipt,
    pub event: Option<EventRecord>,
}

pub struct ActionHandler {
    journal: InMemoryEventJournal,
    permission_checker: Option<Arc<dyn PermissionChecker>>,
    idempotency_guard: Arc<RwLock<LruCache<String, ()>>>,
}

impl Clone for ActionHandler {
    fn clone(&self) -> Self {
        Self {
            journal: self.journal.clone(),
            permission_checker: self.permission_checker.clone(),
            idempotency_guard: self.idempotency_guard.clone(),
        }
    }
}

impl std::fmt::Debug for ActionHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionHandler")
            .field("journal", &self.journal)
            .field("has_permission_checker", &self.permission_checker.is_some())
            .finish()
    }
}

pub trait PermissionChecker: Send + Sync + 'static {
    fn check(&self, action: &ActionDispatch) -> PermissionResult;
}

#[derive(Debug, Clone)]
pub struct LocalPermissionChecker;

impl PermissionChecker for LocalPermissionChecker {
    fn check(&self, _action: &ActionDispatch) -> PermissionResult {
        PermissionResult::local()
    }
}

impl ActionHandler {
    pub fn new(journal: InMemoryEventJournal) -> Self {
        Self {
            journal,
            permission_checker: None,
            idempotency_guard: Arc::new(RwLock::new(LruCache::new(
                NonZeroUsize::new(10_000).expect("nonzero"),
            ))),
        }
    }

    pub fn with_permission_checker(
        journal: InMemoryEventJournal,
        checker: Arc<dyn PermissionChecker>,
    ) -> Self {
        Self {
            journal,
            permission_checker: Some(checker),
            idempotency_guard: Arc::new(RwLock::new(LruCache::new(
                NonZeroUsize::new(10_000).expect("nonzero"),
            ))),
        }
    }

    pub async fn dispatch(
        &self,
        action: ActionDispatch,
        snapshot: &SnapshotEnvelope,
    ) -> ActionReceipt {
        // When an idempotency key is present, the entire dispatch critical section
        // (check + validation + journal append + insert) runs under a single write
        // lock to prevent TOCTOU races under concurrent load.
        if let Some(key) = action.idempotency_key.clone() {
            let mut guard = self.idempotency_guard.write().await;
            if guard.get(&key).is_some() {
                return ActionReceipt::rejected(
                    Uuid::new_v4().to_string(),
                    action.correlation_id,
                    action.action_kind,
                    "duplicate idempotency key",
                );
            }
            let receipt = self.dispatch_unlocked(action, snapshot).await;
            if receipt.status == ActionStatus::Accepted {
                guard.put(key, ());
            }
            return receipt;
        }

        self.dispatch_unlocked(action, snapshot).await
    }

    /// Core dispatch logic without idempotency locking.
    async fn dispatch_unlocked(
        &self,
        action: ActionDispatch,
        snapshot: &SnapshotEnvelope,
    ) -> ActionReceipt {
        let permission = self
            .permission_checker
            .as_ref()
            .map_or_else(PermissionResult::local, |checker| checker.check(&action));

        if !permission.allowed {
            let mut receipt = ActionReceipt::rejected(
                Uuid::new_v4().to_string(),
                action.correlation_id.clone(),
                action.action_kind,
                format!(
                    "permission denied: required role {}",
                    permission.required_role
                ),
            );
            receipt = receipt.with_permission(permission);
            return receipt;
        }

        let issue = match action.target_entity.entity_kind {
            EntityKind::Issue | EntityKind::Run => {
                find_issue_by_id(&snapshot.snapshot, &action.target_entity.entity_id)
            }
            _ => None,
        };

        let action_id = Uuid::new_v4().to_string();

        let validated = match action.action_kind {
            ActionKind::Retry => validate_retry(&action, issue.as_ref(), &action_id),
            ActionKind::Cancel => validate_cancel(&action, issue.as_ref(), &action_id),
            ActionKind::Rehydrate => validate_rehydrate(&action, issue.as_ref(), &action_id),
            ActionKind::Comment => validate_comment(&action, issue.as_ref(), &action_id),
            ActionKind::Pause => validate_pause(&action, issue.as_ref(), &action_id),
            ActionKind::Resume => validate_resume(&action, issue.as_ref(), &action_id),
            ActionKind::OpenWorkspace => validate_generic(&action, issue.as_ref(), &action_id),
            ActionKind::Debug => validate_generic(&action, issue.as_ref(), &action_id),
            ActionKind::TransitionIssue => validate_generic(&action, issue.as_ref(), &action_id),
            ActionKind::CreateFollowup => validate_generic(&action, issue.as_ref(), &action_id),
            ActionKind::ApprovalDecision => validate_generic(&action, issue.as_ref(), &action_id),
            ActionKind::PublishPlan => validate_generic(&action, issue.as_ref(), &action_id),
            ActionKind::TaskGraphMilestone => validate_task_graph(&action, &action_id),
            ActionKind::TaskGraphIssue => validate_task_graph(&action, &action_id),
            ActionKind::TaskGraphSubIssue => validate_task_graph(&action, &action_id),
            ActionKind::TaskGraphRelation => validate_task_graph(&action, &action_id),
            ActionKind::TaskGraphEvidence => validate_task_graph(&action, &action_id),
        };

        let receipt = validated.receipt;

        if let Some(event) = validated.event {
            let _ = self.journal.append(event).await;
        }

        let mut receipt = receipt;
        receipt = receipt.with_permission(permission);
        receipt
    }

    /// Stub: receipt lookup by action ID is not yet persisted.
    /// TODO(COE-405): Persist action receipts to a queryable store so this
    /// returns real data instead of `None`. Intentionally deferred to keep the
    /// initial envelope small and auditable.
    pub async fn receipt_by_id(&self, _action_id: &str) -> Option<ActionReceipt> {
        None
    }
}

fn find_issue_by_id(
    snapshot: &ControlPlaneDaemonSnapshot,
    entity_id: &str,
) -> Option<ControlPlaneIssueSnapshot> {
    snapshot
        .issues
        .iter()
        .find(|issue| {
            issue.identifier.eq_ignore_ascii_case(entity_id)
                || issue.conversation_id_suffix.eq_ignore_ascii_case(entity_id)
        })
        .cloned()
}

fn is_run_action_safe(issue: &ControlPlaneIssueSnapshot, action: RunAction) -> bool {
    let safe = super::safe_actions_for_issue(issue);
    match action {
        RunAction::Retry => safe.retry,
        RunAction::Cancel => safe.cancel,
        RunAction::Rehydrate => safe.rehydrate,
        RunAction::Detach => safe.detach,
        // Pause and Resume are validated by their own dedicated validators
        // (validate_pause and validate_resume) and are never routed here.
        // Comment, follow-up, workspace, and debug are not gated by SafeActions;
        // their eligibility is driven by allowed_actions in the run detail.
        RunAction::Pause
        | RunAction::Resume
        | RunAction::Comment
        | RunAction::CreateFollowup
        | RunAction::OpenWorkspace
        | RunAction::Debug => false,
    }
}

fn validate_retry(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    // Reject retry when an active run is already in progress (duplicate active-run prevention).
    if issue.runtime_state == ControlPlaneIssueRuntimeState::Running
        || issue.runtime_state == ControlPlaneIssueRuntimeState::RetryQueued
    {
        return reject(
            action,
            action_id,
            "cannot retry while a run is already active",
        );
    }

    if !is_run_action_safe(issue, RunAction::Retry) {
        return reject(
            action,
            action_id,
            format!(
                "retry unsafe in state {:?} for issue {}",
                issue.runtime_state, issue.identifier
            ),
        );
    }

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: "retry".into(),
        },
    )
}

fn validate_cancel(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    if !is_run_action_safe(issue, RunAction::Cancel) {
        return reject(
            action,
            action_id,
            format!(
                "cancel unsafe in state {:?} for issue {}",
                issue.runtime_state, issue.identifier
            ),
        );
    }

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: "cancel".into(),
        },
    )
}

fn validate_rehydrate(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    if !is_run_action_safe(issue, RunAction::Rehydrate) {
        return reject(
            action,
            action_id,
            format!(
                "rehydrate unsafe in state {:?} for issue {}. Rehydrate is only available after terminal, cancelled, or explicitly detached states.",
                issue.runtime_state, issue.identifier
            ),
        );
    }

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: "rehydrate".into(),
        },
    )
}

fn validate_comment(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: "comment".into(),
        },
    )
}

fn validate_pause(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    if issue.runtime_state != ControlPlaneIssueRuntimeState::Running {
        return reject(action, action_id, "pause only valid on a running issue");
    }

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: "pause".into(),
        },
    )
}

fn validate_resume(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    if issue.runtime_state != ControlPlaneIssueRuntimeState::Paused {
        return reject(action, action_id, "resume only valid on a paused issue");
    }

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: "resume".into(),
        },
    )
}

/// Generic action validation for actions that do not require runtime state gating.
///
/// Actions validated here (`OpenWorkspace`, `Debug`, `TransitionIssue`, `CreateFollowup`,
/// `ApprovalDecision`, `PublishPlan`) are inherently safe because they operate on the
/// issue tracker, planning layer, or local UI rather than the active harness runtime.
/// They do not mutate scheduler state and are therefore accepted for any valid issue
/// snapshot. If a future action needs runtime state gating, it should be promoted from
/// `validate_generic` to a dedicated validator (e.g., `validate_pause`, `validate_resume`).
fn validate_generic(
    action: &ActionDispatch,
    issue: Option<&ControlPlaneIssueSnapshot>,
    action_id: &str,
) -> ValidatedAction {
    let Some(issue) = issue else {
        return reject(action, action_id, "target issue not found in snapshot");
    };

    accepted(
        action,
        action_id,
        issue,
        EventKind::GatewayActionDispatched {
            action: action.action_kind.to_string(),
        },
    )
}

/// Validate a task-graph mutation action.
///
/// Unlike the orchestrator-scoped actions, task-graph mutations operate
/// against the Linear tracker (project milestones, issues, sub-issues,
/// relations, evidence notes). They are inherently safe at the gateway
/// validation layer (the gateway is not an orchestrator), so they only need:
///
/// 1. The action target entity kind must be one of `Milestone`, `Issue`,
///    `SubIssue`, or `Project`. Relation and evidence actions still target an
///    issue under the hood, so the kind stays `Issue`/`SubIssue` here.
/// 2. The correlation ID must be non-empty so events can be correlated with
///    the returned action receipt.
/// 3. If a payload shape is supplied, it must be a JSON object — non-object
///    payloads are rejected because the downstream `TaskGraphMutator` expects
///    a typed request.
fn validate_task_graph(action: &ActionDispatch, action_id: &str) -> ValidatedAction {
    use EntityKind as Ek;

    let kind = action.target_entity.entity_kind;
    if !matches!(kind, Ek::Milestone | Ek::Issue | Ek::SubIssue | Ek::Project) {
        return reject(
            action,
            action_id,
            format!(
                "task-graph action {} requires Milestone/Issue/SubIssue/Project target, got {:?}",
                action.action_kind, kind
            ),
        );
    }
    if action.correlation_id.trim().is_empty() {
        return reject(
            action,
            action_id,
            "task-graph action requires non-empty correlation_id",
        );
    }
    if let Some(payload) = action.payload.as_ref()
        && !payload.is_object()
    {
        return reject(
            action,
            action_id,
            "task-graph action payload, when provided, must be a JSON object",
        );
    }

    let receipt = ActionReceipt::accepted(
        action_id.to_owned(),
        action.correlation_id.clone(),
        action.action_kind,
    );

    let entity_ref = EntityRef {
        kind,
        id: action.target_entity.entity_id.clone(),
        identifier: None,
    };

    let event = EventRecord::builder()
        .actor(EventActor::system("gateway"))
        .correlation_id(action.correlation_id.clone())
        .kind(EventKind::GatewayActionDispatched {
            action: action.action_kind.to_string(),
        })
        .entity_ref(entity_ref)
        .summary(format!(
            "Action {} dispatched against {:?} {}",
            action.action_kind, kind, action.target_entity.entity_id
        ))
        .payload(json!({
            "action_id": action_id,
            "action_kind": action.action_kind.to_string(),
            "correlation_id": action.correlation_id,
            "status": "accepted",
            "idempotency_key": action.idempotency_key,
            "target_entity": {
                "kind": format!("{:?}", kind).to_lowercase(),
                "id": action.target_entity.entity_id,
            },
        }))
        .build();

    ValidatedAction {
        action_id: action_id.to_owned(),
        receipt,
        event: Some(event),
    }
}

fn accepted(
    action: &ActionDispatch,
    action_id: &str,
    issue: &ControlPlaneIssueSnapshot,
    kind: EventKind,
) -> ValidatedAction {
    let receipt = ActionReceipt::accepted(
        action_id.to_owned(),
        action.correlation_id.clone(),
        action.action_kind,
    );

    let event = build_audit_event(
        action,
        action_id,
        &kind,
        ActionStatus::Accepted,
        None,
        issue,
    );

    ValidatedAction {
        action_id: action_id.to_owned(),
        receipt,
        event: Some(event),
    }
}

fn reject(action: &ActionDispatch, action_id: &str, reason: impl Into<String>) -> ValidatedAction {
    let reason = reason.into();
    let receipt = ActionReceipt::rejected(
        action_id.to_owned(),
        action.correlation_id.clone(),
        action.action_kind,
        reason.clone(),
    );

    let event = EventRecord::builder()
        .actor(EventActor::system("gateway"))
        .correlation_id(action.correlation_id.clone())
        .kind(EventKind::GatewayActionFailed {
            action: action.action_kind.to_string(),
            reason: reason.clone(),
        })
        .summary(format!(
            "Action {} rejected: {}",
            action.action_kind, reason
        ))
        .payload(json!({
            "action_id": action_id,
            "action_kind": action.action_kind.to_string(),
            "correlation_id": action.correlation_id,
            "status": "rejected",
            "reason": reason,
        }))
        .build();

    ValidatedAction {
        action_id: action_id.to_owned(),
        receipt,
        event: Some(event),
    }
}

fn build_audit_event(
    action: &ActionDispatch,
    action_id: &str,
    kind: &EventKind,
    status: ActionStatus,
    _reason: Option<String>,
    issue: &ControlPlaneIssueSnapshot,
) -> EventRecord {
    let status_str = match status {
        ActionStatus::Accepted => "accepted",
        ActionStatus::Rejected => "rejected",
    };

    EventRecord::builder()
        .actor(EventActor::system("gateway"))
        .correlation_id(action.correlation_id.clone())
        .kind(kind.clone())
        .entity_ref(EntityRef::issue(
            &issue.identifier,
            Some(issue.identifier.clone()),
        ))
        .summary(format!(
            "Action {} {} for {}",
            action.action_kind, status_str, issue.identifier
        ))
        .payload(json!({
            "action_id": action_id,
            "action_kind": action.action_kind.to_string(),
            "correlation_id": action.correlation_id,
            "status": status_str,
            "target_entity": {
                "kind": format!("{:?}", action.target_entity.entity_kind).to_lowercase(),
                "id": action.target_entity.entity_id,
            },
        }))
        .build()
}
