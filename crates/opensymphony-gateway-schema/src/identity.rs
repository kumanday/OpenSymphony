//! Hosted identity, tenant, and RBAC schema.
//!
//! These types model the hosted-mode identity surface: users, organizations
//! (tenants), memberships, roles, project access rules, sessions, and the
//! authenticated context that flows through API, WebSocket, and
//! JSON-RPC-over-WebSocket requests.
//!
//! The schema is transport-agnostic. The gateway owns an in-memory store for
//! the hosted alpha; a relational store is a follow-on (out of scope here).
//! Entity IDs are tenant-scoped: a hosted entity belongs to exactly one
//! organization, and access decisions always evaluate membership plus project
//! access within that tenant.

use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// A hosted user identity.
///
/// Authentication credentials are never serialized into gateway DTOs; only
/// identity and display fields are exposed. Credential storage is out of scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedUser {
    pub schema_version: SchemaVersion,
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    /// Stable opaque handle used in audit records.
    pub handle: String,
}

/// An organization is the tenant boundary for hosted entities.
///
/// Every hosted project, run, planning session, secret reference, and action
/// target is scoped to exactly one organization. Tenant isolation in data,
/// storage, logs, events, and workspaces is a hosted requirement (PRD 4.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub schema_version: SchemaVersion,
    pub organization_id: String,
    pub slug: String,
    pub display_name: String,
}

/// Coarse role within an organization. Fine-grained capability checks are
/// derived from the role plus project access rules in the RBAC evaluator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Full control, including billing and admin actions.
    Owner,
    /// Manage projects, members, and runs within the organization.
    Admin,
    /// Create and operate on runs, planning sessions, and actions.
    Member,
    /// Read-only access to permitted projects.
    Viewer,
}

impl Role {
    /// Ordinal used for role comparison (higher = more privileged).
    pub fn ordinal(self) -> u8 {
        match self {
            Role::Viewer => 0,
            Role::Member => 1,
            Role::Admin => 2,
            Role::Owner => 3,
        }
    }

    /// True when this role satisfies at least `required`.
    pub fn satisfies(self, required: Role) -> bool {
        self.ordinal() >= required.ordinal()
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Viewer => "viewer",
        };
        f.write_str(s)
    }
}

impl From<Role> for String {
    fn from(role: Role) -> Self {
        role.to_string()
    }
}

/// A user's membership in an organization, carrying the org-scoped role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    pub schema_version: SchemaVersion,
    pub membership_id: String,
    pub user_id: String,
    pub organization_id: String,
    pub role: Role,
}

/// Per-project access rule within an organization.
///
/// A user may be a member of an organization but restricted to a subset of
/// projects. `ProjectAccess` records which projects a user may access and at
/// what role within that project. An empty `projects` list with
/// `all_projects: true` grants access to every project in the org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAccess {
    pub schema_version: SchemaVersion,
    pub user_id: String,
    pub organization_id: String,
    /// When true, the user may access every project in the organization.
    pub all_projects: bool,
    /// Explicit project IDs the user may access (ignored when `all_projects`).
    pub projects: Vec<ProjectAccessEntry>,
}

/// A single project grant with a project-scoped role cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAccessEntry {
    pub project_id: String,
    /// Maximum role the user holds within this project (capped by org role).
    pub role: Role,
}

/// A session token issued after successful login.
///
/// The token string is opaque to clients and is presented as a bearer token
/// on HTTP requests or as a query/init parameter on WebSocket upgrades.
/// Credential storage and rotation are out of scope for the hosted alpha.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub schema_version: SchemaVersion,
    pub session_id: String,
    /// Opaque bearer token presented by clients. Never logged or emitted.
    #[serde(skip_serializing, skip_deserializing)]
    pub token: String,
    pub user_id: String,
    pub organization_id: String,
    pub issued_at: String,
    pub expires_at: String,
}

/// Authenticated identity and tenant context for a single request or stream.
///
/// Carried through API middleware, WebSocket upgrades, and action dispatch so
/// every hosted request and stream carries authenticated user and tenant
/// context (acceptance criterion). `None`-equivalent states are modeled by
/// the caller (local/dev bypass uses `AuthContext::dev_bypass`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    pub schema_version: SchemaVersion,
    pub user_id: String,
    pub organization_id: String,
    pub role: Role,
    /// Stable display fields for audit records and receipts.
    pub user_handle: String,
    pub organization_slug: String,
    /// True when this context was injected by the explicit dev bypass.
    pub dev_bypass: bool,
}

impl AuthContext {
    /// Dev-bypass context: an explicit local-development identity. Only valid
    /// when the gateway is configured with dev bypass and not in production.
    pub fn dev_bypass() -> Self {
        Self {
            schema_version: SchemaVersion::default(),
            user_id: "dev-user".into(),
            organization_id: "dev-org".into(),
            role: Role::Owner,
            user_handle: "dev-user".into(),
            organization_slug: "dev-org".into(),
            dev_bypass: true,
        }
    }
}

/// Login request body for `POST /api/v1/auth/login`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    /// Organization the user is signing into (tenant selection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_slug: Option<String>,
}

/// Login response body returned after successful authentication.
///
/// The session token is returned once at login; clients persist it and present
/// it as a bearer token. The response also echoes the resolved identity and
/// tenant so the client can render organization/project selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginResponse {
    pub schema_version: SchemaVersion,
    pub session_token: String,
    pub user: HostedUser,
    pub organization: Organization,
    pub role: Role,
    pub expires_at: String,
}

/// Session probe response for `GET /api/v1/auth/session`.
///
/// Lets the client verify a persisted token is still valid and recover the
/// authenticated identity + tenant without re-prompting credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionResponse {
    pub schema_version: SchemaVersion,
    pub user: HostedUser,
    pub organization: Organization,
    pub role: Role,
    pub expires_at: String,
}

/// Standard error body for auth failures. Carries an `error_code` the client
/// classifier maps to an `AuthState` (see `authStateFromError`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthErrorBody {
    pub error_code: AuthErrorCode,
    pub message: String,
}

/// Error codes the client shell recognizes for auth-state classification.
///
/// Mirrors the TS `GatewayErrorCode`/`AuthErrorCode` set so a 403 with one of
/// `unauthorized`/`permission_denied`/`forbidden_resource` maps to
/// `unauthorized`, while a plain 403 maps to `forbidden`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorCode {
    /// No/invalid credentials (HTTP 401).
    Unauthenticated,
    /// Authenticated but lacking permission (HTTP 403, permission denial).
    Unauthorized,
    /// Permission-denial body signal variants the client also recognizes.
    PermissionDenied,
    ForbiddenResource,
    /// Hard server deny (HTTP 403 without a permission signal).
    Forbidden,
    /// Dev bypass attempted where it is not allowed.
    DevBypassDisabled,
}

impl AuthErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthErrorCode::Unauthenticated => "unauthenticated",
            AuthErrorCode::Unauthorized => "unauthorized",
            AuthErrorCode::PermissionDenied => "permission_denied",
            AuthErrorCode::ForbiddenResource => "forbidden_resource",
            AuthErrorCode::Forbidden => "forbidden",
            AuthErrorCode::DevBypassDisabled => "dev_bypass_disabled",
        }
    }
}