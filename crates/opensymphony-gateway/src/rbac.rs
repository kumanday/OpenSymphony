//! Role-based permission evaluation for hosted mode.
//!
//! Maps an authenticated `AuthContext` plus a resource/action request into a
//! `PermissionResult`. The evaluator is the single authority for hosted
//! permission decisions so client-side state is never the final authority
//! (PRD 4.11).
//!
//! Resources protected: project, run, planning session, secret, and action
//! access (acceptance criterion). The evaluator consults the identity store
//! for membership and project access rules.

use crate::opensymphony_gateway_schema::action::{ActionDispatch, ActionKind, PermissionResult};
use crate::opensymphony_gateway_schema::envelope::EntityKind;
use crate::opensymphony_gateway_schema::identity::{AuthContext, Role};

use crate::opensymphony_gateway::identity_store::{HostedIdentityStore, IdentityError};

/// The kind of resource a hosted permission decision protects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectedResource {
    Project,
    Run,
    PlanningSession,
    Secret,
    Action,
    Stream,
}

/// Capability required to perform an action on a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
    Operate,
    Admin,
}

impl Capability {
    /// The minimum role that grants this capability by default.
    pub fn minimum_role(self) -> Role {
        match self {
            Capability::Read => Role::Viewer,
            Capability::Operate => Role::Member,
            Capability::Admin => Role::Admin,
        }
    }
}

impl ProtectedResource {
    /// The capability required for an action kind against this resource.
    pub fn capability_for_action(self, action_kind: ActionKind) -> Capability {
        match (self, action_kind) {
            // Mutations require Operate; admin-shape actions require Admin.
            (ProtectedResource::Action, ActionKind::ApprovalDecision)
            | (ProtectedResource::Action, ActionKind::TransitionIssue)
            | (ProtectedResource::Action, ActionKind::PublishPlan) => Capability::Admin,
            (ProtectedResource::Action, _) => Capability::Operate,
            (ProtectedResource::Run, ActionKind::Retry)
            | (ProtectedResource::Run, ActionKind::Cancel)
            | (ProtectedResource::Run, ActionKind::Pause)
            | (ProtectedResource::Run, ActionKind::Resume)
            | (ProtectedResource::Run, ActionKind::Rehydrate)
            | (ProtectedResource::Run, ActionKind::Comment)
            | (ProtectedResource::Run, ActionKind::OpenWorkspace)
            | (ProtectedResource::Run, ActionKind::Debug)
            | (ProtectedResource::Run, ActionKind::TransitionIssue)
            | (ProtectedResource::Run, ActionKind::CreateFollowup)
            | (ProtectedResource::Run, ActionKind::ApprovalDecision)
            | (ProtectedResource::Run, ActionKind::PublishPlan)
            | (ProtectedResource::Run, ActionKind::TaskGraphMilestone)
            | (ProtectedResource::Run, ActionKind::TaskGraphIssue)
            | (ProtectedResource::Run, ActionKind::TaskGraphSubIssue)
            | (ProtectedResource::Run, ActionKind::TaskGraphRelation)
            | (ProtectedResource::Run, ActionKind::TaskGraphEvidence) => Capability::Operate,
            (ProtectedResource::Secret, _) => Capability::Admin,
            (ProtectedResource::PlanningSession, ActionKind::PublishPlan) => Capability::Admin,
            (ProtectedResource::PlanningSession, _) => Capability::Operate,
            (ProtectedResource::Project, ActionKind::TaskGraphMilestone)
            | (ProtectedResource::Project, ActionKind::TaskGraphIssue)
            | (ProtectedResource::Project, ActionKind::TaskGraphSubIssue)
            | (ProtectedResource::Project, ActionKind::TaskGraphRelation)
            | (ProtectedResource::Project, ActionKind::TaskGraphEvidence) => Capability::Operate,
            (ProtectedResource::Project, _) => Capability::Read,
            (ProtectedResource::Stream, _) => Capability::Read,
        }
    }

    /// Classify the resource targeted by an action from its entity kind.
    pub fn from_action(action: &ActionDispatch) -> Self {
        match action.target_entity.entity_kind {
            EntityKind::Project => ProtectedResource::Project,
            EntityKind::Run | EntityKind::Issue | EntityKind::SubIssue => ProtectedResource::Run,
            EntityKind::PlanningSession => ProtectedResource::PlanningSession,
            EntityKind::Workspace => ProtectedResource::Run,
            _ => ProtectedResource::Action,
        }
    }
}

/// Hosted RBAC evaluator. Owns a reference to the identity store for
/// membership and project-access lookups.
#[derive(Clone)]
pub struct PermissionEvaluator {
    identity: HostedIdentityStore,
}

impl PermissionEvaluator {
    pub fn new(identity: HostedIdentityStore) -> Self {
        Self { identity }
    }

    /// Evaluate a hosted permission decision for a read on a project-scoped
    /// resource. `project_id` scopes the decision to a project within the
    /// caller's organization.
    pub fn evaluate_read(
        &self,
        ctx: &AuthContext,
        resource: ProtectedResource,
        project_id: Option<&str>,
    ) -> PermissionResult {
        if ctx.dev_bypass {
            return PermissionResult::evaluated(true, Role::Owner);
        }
        let capability = match resource {
            ProtectedResource::Secret => Capability::Admin,
            _ => Capability::Read,
        };
        let required_role = capability.minimum_role();
        if !ctx.role.satisfies(required_role) {
            return PermissionResult::denied(
                required_role,
                format!(
                    "permission denied: role {} cannot read {} (requires {})",
                    ctx.role,
                    resource_label(resource),
                    required_role
                ),
                "unauthorized",
            );
        }
        if let Some(project_id) = project_id {
            match self
                .identity
                .may_access_project(&ctx.user_id, &ctx.organization_id, project_id)
            {
                Ok(true) => {}
                Ok(false) => {
                    return PermissionResult::denied(
                        required_role,
                        format!(
                            "permission denied: no access to project {} in organization {}",
                            project_id, ctx.organization_id
                        ),
                        "permission_denied",
                    );
                }
                Err(IdentityError::NotAMember(user, org)) => {
                    return PermissionResult::denied(
                        required_role,
                        format!("permission denied: user {user} is not a member of {org}"),
                        "unauthorized",
                    );
                }
                Err(_) => {
                    return PermissionResult::denied(
                        required_role,
                        "permission denied: identity lookup failed",
                        "forbidden_resource",
                    );
                }
            }
        }
        PermissionResult::evaluated(true, required_role)
    }

    /// Evaluate a hosted permission decision for an action dispatch.
    pub fn evaluate_action(&self, ctx: &AuthContext, action: &ActionDispatch) -> PermissionResult {
        if ctx.dev_bypass {
            return PermissionResult::evaluated(true, Role::Owner);
        }
        let resource = ProtectedResource::from_action(action);
        let capability = resource.capability_for_action(action.action_kind);
        let required_role = capability.minimum_role();
        if !ctx.role.satisfies(required_role) {
            return PermissionResult::denied(
                required_role,
                format!(
                    "permission denied: role {} cannot {} {} (requires {})",
                    ctx.role,
                    action_kind_label(action.action_kind),
                    resource_label(resource),
                    required_role
                ),
                "unauthorized",
            );
        }
        // For project-scoped task graph actions, check project access when the
        // payload carries a project_id.
        if let Some(project_id) = action_project_id(action) {
            match self
                .identity
                .may_access_project(&ctx.user_id, &ctx.organization_id, &project_id)
            {
                Ok(true) => {}
                Ok(false) => {
                    return PermissionResult::denied(
                        required_role,
                        format!(
                            "permission denied: no access to project {} in organization {}",
                            project_id, ctx.organization_id
                        ),
                        "permission_denied",
                    );
                }
                Err(IdentityError::NotAMember(user, org)) => {
                    return PermissionResult::denied(
                        required_role,
                        format!("permission denied: user {user} is not a member of {org}"),
                        "unauthorized",
                    );
                }
                Err(_) => {
                    return PermissionResult::denied(
                        required_role,
                        "permission denied: identity lookup failed",
                        "forbidden_resource",
                    );
                }
            }
        }
        PermissionResult::evaluated(true, required_role)
    }
}

/// Extract a project_id from an action payload when present (task graph
/// mutations carry project scoping in their payload).
fn action_project_id(action: &ActionDispatch) -> Option<String> {
    let payload = action.payload.as_ref()?;
    let obj = payload.as_object()?;
    if let Some(pid) = obj.get("project_id").and_then(|v| v.as_str()) {
        return Some(pid.to_string());
    }
    if let Some(pid) = obj.get("projectId").and_then(|v| v.as_str()) {
        return Some(pid.to_string());
    }
    None
}

fn resource_label(resource: ProtectedResource) -> &'static str {
    match resource {
        ProtectedResource::Project => "project",
        ProtectedResource::Run => "run",
        ProtectedResource::PlanningSession => "planning_session",
        ProtectedResource::Secret => "secret",
        ProtectedResource::Action => "action",
        ProtectedResource::Stream => "stream",
    }
}

fn action_kind_label(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Retry => "retry",
        ActionKind::Cancel => "cancel",
        ActionKind::Pause => "pause",
        ActionKind::Resume => "resume",
        ActionKind::Rehydrate => "rehydrate",
        ActionKind::Comment => "comment",
        ActionKind::OpenWorkspace => "open_workspace",
        ActionKind::Debug => "debug",
        ActionKind::TransitionIssue => "transition_issue",
        ActionKind::CreateFollowup => "create_followup",
        ActionKind::ApprovalDecision => "approval_decision",
        ActionKind::PublishPlan => "publish_plan",
        ActionKind::TaskGraphMilestone => "task_graph_milestone",
        ActionKind::TaskGraphIssue => "task_graph_issue",
        ActionKind::TaskGraphSubIssue => "task_graph_sub_issue",
        ActionKind::TaskGraphRelation => "task_graph_relation",
        ActionKind::TaskGraphEvidence => "task_graph_evidence",
    }
}
