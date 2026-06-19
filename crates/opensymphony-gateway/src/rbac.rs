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

/// Extension trait that derives a default [`Capability`] from an action kind.
///
/// This is the per-action default that [`ProtectedResource::capability_for_action`]
/// narrows per resource. Centralizing it means a new `ActionKind` variant gets a
/// sensible default capability (Operate, or Admin when added to the admin list)
/// without editing every resource arm, and the admin-gated action kinds are
/// declared in one auditable place.
pub trait ActionCapability {
    /// Default capability required for this action kind against its primary
    /// mutation resource.
    fn required_capability(self) -> Capability;
}

impl ActionCapability for ActionKind {
    fn required_capability(self) -> Capability {
        match self {
            // Admin-shape actions that change authorization or publish plans.
            ActionKind::ApprovalDecision
            | ActionKind::TransitionIssue
            | ActionKind::PublishPlan => Capability::Admin,
            // All other run/operate mutations default to Member-level operate.
            _ => Capability::Operate,
        }
    }
}

impl ProtectedResource {
    /// The capability required for an action kind against this resource.
    ///
    /// The policy derives a default capability from the action kind
    /// ([`ActionKind::required_capability`]) and then narrows it per resource,
    /// so adding a new `ActionKind` gets a sensible default without editing
    /// every arm: only resources with a distinct policy for that kind need an
    /// explicit override. This keeps the RBAC table auditable while removing
    /// the implicit `_` catch-all fallbacks that silently routed unknown
    /// (resource, action) pairs.
    pub fn capability_for_action(self, action_kind: ActionKind) -> Capability {
        match self {
            // Secrets are always admin-gated, regardless of action kind.
            ProtectedResource::Secret => Capability::Admin,
            // Streams are read-only subscriptions.
            ProtectedResource::Stream => Capability::Read,
            // Every run mutation operates at Member level.
            ProtectedResource::Run => Capability::Operate,
            // Planning sessions are operated by members; publishing a plan is
            // admin-gated.
            ProtectedResource::PlanningSession => match action_kind {
                ActionKind::PublishPlan => Capability::Admin,
                _ => Capability::Operate,
            },
            // Project task-graph mutations operate at Member level; any other
            // action against a project falls back to read-level access.
            ProtectedResource::Project => match action_kind {
                ActionKind::TaskGraphMilestone
                | ActionKind::TaskGraphIssue
                | ActionKind::TaskGraphSubIssue
                | ActionKind::TaskGraphRelation
                | ActionKind::TaskGraphEvidence => Capability::Operate,
                _ => Capability::Read,
            },
            // A bare action resource uses the action-kind default: admin-shape
            // actions (approval/transition/publish) require Admin, everything
            // else requires Operate.
            ProtectedResource::Action => action_kind.required_capability(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway::identity_store::{
        HostedIdentityStore, SeedMembership, SeedOrganization, SeedProjectAccess, SeedUser,
    };
    use crate::opensymphony_gateway_schema::action::ActionTarget;

    fn ctx(role: Role, dev_bypass: bool) -> AuthContext {
        AuthContext {
            schema_version: Default::default(),
            user_id: "u".into(),
            organization_id: "org-1".into(),
            role,
            user_handle: "u".into(),
            organization_slug: "acme".into(),
            dev_bypass,
        }
    }

    fn store_with_restricted_viewer() -> HostedIdentityStore {
        let store = HostedIdentityStore::new();
        store.seed(
            vec![SeedUser {
                user_id: "u".into(),
                email: "u@example.com".into(),
                display_name: "U".into(),
                handle: "u".into(),
                password: "pw".into(),
            }],
            vec![SeedOrganization {
                organization_id: "org-1".into(),
                slug: "acme".into(),
                display_name: "Acme".into(),
            }],
            vec![SeedMembership {
                user_id: "u".into(),
                organization_id: "org-1".into(),
                role: Role::Viewer,
            }],
            vec![SeedProjectAccess {
                user_id: "u".into(),
                organization_id: "org-1".into(),
                all_projects: false,
                projects: vec![("allowed".into(), Role::Viewer)],
            }],
        );
        store
    }

    #[test]
    fn read_requires_viewer_floor_for_all_resources() {
        let store = HostedIdentityStore::new();
        let evaluator = PermissionEvaluator::new(store);
        // A user with no role gap: viewer can read project/run/planning/secret.
        for resource in [
            ProtectedResource::Project,
            ProtectedResource::Run,
            ProtectedResource::PlanningSession,
            ProtectedResource::Stream,
        ] {
            let permission = evaluator.evaluate_read(&ctx(Role::Viewer, false), resource, None);
            assert!(permission.allowed, "viewer may read {resource:?}");
            assert!(permission.evaluated);
        }
        // Secret reads require Admin regardless of the read capability default.
        let secret =
            evaluator.evaluate_read(&ctx(Role::Viewer, false), ProtectedResource::Secret, None);
        assert!(!secret.allowed, "viewer cannot read secrets");
        assert_eq!(secret.denied_code.as_deref(), Some("unauthorized"));
        let admin_secret =
            evaluator.evaluate_read(&ctx(Role::Admin, false), ProtectedResource::Secret, None);
        assert!(admin_secret.allowed, "admin may read secrets");
    }

    #[test]
    fn read_denies_below_viewer_floor() {
        // Member satisfies Viewer, Admin satisfies Viewer, Owner satisfies Viewer.
        let store = HostedIdentityStore::new();
        let evaluator = PermissionEvaluator::new(store);
        for role in [Role::Member, Role::Admin, Role::Owner] {
            let permission =
                evaluator.evaluate_read(&ctx(role, false), ProtectedResource::Run, None);
            assert!(permission.allowed, "{role} may read runs");
        }
    }

    #[test]
    fn read_enforces_project_access_rules() {
        let store = store_with_restricted_viewer();
        let evaluator = PermissionEvaluator::new(store);
        let allowed = evaluator.evaluate_read(
            &ctx(Role::Viewer, false),
            ProtectedResource::Project,
            Some("allowed"),
        );
        assert!(allowed.allowed, "permitted project is readable");
        let denied = evaluator.evaluate_read(
            &ctx(Role::Viewer, false),
            ProtectedResource::Project,
            Some("other"),
        );
        assert!(!denied.allowed, "non-permitted project is denied");
        assert_eq!(denied.denied_code.as_deref(), Some("permission_denied"));
    }

    fn dispatch(entity: EntityKind, kind: ActionKind) -> ActionDispatch {
        ActionDispatch {
            schema_version: Default::default(),
            correlation_id: "c".into(),
            action_kind: kind,
            target_entity: ActionTarget {
                entity_kind: entity,
                entity_id: "id".into(),
            },
            payload: None,
            idempotency_key: None,
            actor: None,
        }
    }

    #[test]
    fn action_operate_requires_member_for_runs() {
        let store = HostedIdentityStore::new();
        let evaluator = PermissionEvaluator::new(store);
        let viewer = evaluator.evaluate_action(
            &ctx(Role::Viewer, false),
            &dispatch(EntityKind::Run, ActionKind::Retry),
        );
        assert!(!viewer.allowed, "viewer cannot retry a run");
        assert_eq!(viewer.denied_code.as_deref(), Some("unauthorized"));
        let member = evaluator.evaluate_action(
            &ctx(Role::Member, false),
            &dispatch(EntityKind::Run, ActionKind::Retry),
        );
        assert!(member.allowed, "member may retry a run");
    }

    #[test]
    fn action_admin_actions_require_admin_role() {
        let store = HostedIdentityStore::new();
        let evaluator = PermissionEvaluator::new(store);
        // ApprovalDecision / TransitionIssue / PublishPlan on an Action resource
        // require Admin.
        for kind in [
            ActionKind::ApprovalDecision,
            ActionKind::TransitionIssue,
            ActionKind::PublishPlan,
        ] {
            let member = evaluator.evaluate_action(
                &ctx(Role::Member, false),
                &dispatch(EntityKind::Unknown, kind),
            );
            assert!(
                !member.allowed,
                "member cannot perform admin action {kind:?}"
            );
            let admin = evaluator.evaluate_action(
                &ctx(Role::Admin, false),
                &dispatch(EntityKind::Unknown, kind),
            );
            assert!(admin.allowed, "admin may perform admin action {kind:?}");
        }
    }

    #[test]
    fn planning_session_publish_requires_admin() {
        let store = HostedIdentityStore::new();
        let evaluator = PermissionEvaluator::new(store);
        let member = evaluator.evaluate_action(
            &ctx(Role::Member, false),
            &dispatch(EntityKind::PlanningSession, ActionKind::PublishPlan),
        );
        assert!(
            !member.allowed,
            "member cannot publish a plan from a planning session"
        );
        let admin = evaluator.evaluate_action(
            &ctx(Role::Admin, false),
            &dispatch(EntityKind::PlanningSession, ActionKind::PublishPlan),
        );
        assert!(admin.allowed, "admin may publish a plan");
        // Non-publish planning-session actions require Operate (Member).
        let viewer = evaluator.evaluate_action(
            &ctx(Role::Viewer, false),
            &dispatch(EntityKind::PlanningSession, ActionKind::Comment),
        );
        assert!(
            !viewer.allowed,
            "viewer cannot operate on a planning session"
        );
        let member_operate = evaluator.evaluate_action(
            &ctx(Role::Member, false),
            &dispatch(EntityKind::PlanningSession, ActionKind::Comment),
        );
        assert!(
            member_operate.allowed,
            "member may operate on a planning session"
        );
    }

    #[test]
    fn dev_bypass_grants_all_resources() {
        let store = HostedIdentityStore::new();
        let evaluator = PermissionEvaluator::new(store);
        // Dev bypass short-circuits to an evaluated Owner decision for every
        // resource, including secrets and admin actions.
        for resource in [
            ProtectedResource::Project,
            ProtectedResource::Run,
            ProtectedResource::PlanningSession,
            ProtectedResource::Secret,
            ProtectedResource::Stream,
            ProtectedResource::Action,
        ] {
            let permission = evaluator.evaluate_read(&ctx(Role::Viewer, true), resource, None);
            assert!(permission.allowed, "dev bypass grants read on {resource:?}");
        }
        let admin_action = evaluator.evaluate_action(
            &ctx(Role::Viewer, true),
            &dispatch(EntityKind::Unknown, ActionKind::PublishPlan),
        );
        assert!(admin_action.allowed, "dev bypass grants admin actions");
    }

    #[test]
    fn action_project_access_is_enforced_from_payload() {
        // Promote the restricted viewer to Member so the role floor passes and
        // only the project-access rule can deny.
        let store_member = HostedIdentityStore::new();
        store_member.seed(
            vec![SeedUser {
                user_id: "u".into(),
                email: "u@example.com".into(),
                display_name: "U".into(),
                handle: "u".into(),
                password: "pw".into(),
            }],
            vec![SeedOrganization {
                organization_id: "org-1".into(),
                slug: "acme".into(),
                display_name: "Acme".into(),
            }],
            vec![SeedMembership {
                user_id: "u".into(),
                organization_id: "org-1".into(),
                role: Role::Member,
            }],
            vec![SeedProjectAccess {
                user_id: "u".into(),
                organization_id: "org-1".into(),
                all_projects: false,
                projects: vec![("allowed".into(), Role::Member)],
            }],
        );
        let evaluator = PermissionEvaluator::new(store_member);
        let mut allowed_action = dispatch(EntityKind::Project, ActionKind::TaskGraphIssue);
        allowed_action.payload = Some(serde_json::json!({"project_id": "allowed"}));
        let allowed = evaluator.evaluate_action(&ctx(Role::Member, false), &allowed_action);
        assert!(
            allowed.allowed,
            "task-graph action on permitted project allowed"
        );

        let mut denied_action = dispatch(EntityKind::Project, ActionKind::TaskGraphIssue);
        denied_action.payload = Some(serde_json::json!({"project_id": "other"}));
        let denied = evaluator.evaluate_action(&ctx(Role::Member, false), &denied_action);
        assert!(
            !denied.allowed,
            "task-graph action on non-permitted project denied"
        );
        assert_eq!(denied.denied_code.as_deref(), Some("permission_denied"));
    }

    #[test]
    fn capability_for_action_derives_from_action_kind_with_per_resource_overrides() {
        // Admin-shape actions default to Admin against a bare Action resource.
        for kind in [
            ActionKind::ApprovalDecision,
            ActionKind::TransitionIssue,
            ActionKind::PublishPlan,
        ] {
            assert_eq!(
                ProtectedResource::Action.capability_for_action(kind),
                Capability::Admin,
                "{kind:?} on Action requires Admin"
            );
        }
        // Operate-shape actions default to Operate against a bare Action resource.
        assert_eq!(
            ProtectedResource::Action.capability_for_action(ActionKind::Retry),
            Capability::Operate
        );

        // Runs always operate at Member level, even for admin-shape actions.
        for kind in [
            ActionKind::ApprovalDecision,
            ActionKind::PublishPlan,
            ActionKind::Retry,
            ActionKind::TaskGraphIssue,
        ] {
            assert_eq!(
                ProtectedResource::Run.capability_for_action(kind),
                Capability::Operate,
                "{kind:?} on Run requires Operate"
            );
        }

        // Secrets are always admin-gated.
        assert_eq!(
            ProtectedResource::Secret.capability_for_action(ActionKind::Retry),
            Capability::Admin
        );
        // Streams are read-only.
        assert_eq!(
            ProtectedResource::Stream.capability_for_action(ActionKind::Retry),
            Capability::Read
        );

        // Planning session: publish is Admin, everything else Operate.
        assert_eq!(
            ProtectedResource::PlanningSession.capability_for_action(ActionKind::PublishPlan),
            Capability::Admin
        );
        assert_eq!(
            ProtectedResource::PlanningSession.capability_for_action(ActionKind::Comment),
            Capability::Operate
        );

        // Project: task-graph mutations Operate, other actions Read.
        assert_eq!(
            ProtectedResource::Project.capability_for_action(ActionKind::TaskGraphIssue),
            Capability::Operate
        );
        assert_eq!(
            ProtectedResource::Project.capability_for_action(ActionKind::Retry),
            Capability::Read
        );
    }
}
