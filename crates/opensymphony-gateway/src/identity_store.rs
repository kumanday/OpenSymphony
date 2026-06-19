//! In-memory hosted identity and session store for the hosted alpha.
//!
//! Owns users, organizations, memberships, project access rules, and session
//! tokens. A relational store is a follow-on (out of scope for COE-420); the
//! in-memory store preserves the gateway contract and is seedable for tests.
//!
//! Credential storage is intentionally out of scope: passwords are compared in
//! plain text against seeded fixtures only because this is an alpha auth
//! provider strategy. Production must replace `SessionTokenAuthProvider` with
//! a real credential store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::opensymphony_gateway_schema::identity::{
    AuthContext, HostedUser, LoginRequest, LoginResponse, Membership, Organization,
    Organization as Org, ProjectAccess, Role, Session, SessionResponse,
};
use crate::opensymphony_gateway_schema::version::SchemaVersion;

/// Error returned by identity store operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("unknown user: {0}")]
    UnknownUser(String),
    #[error("unknown organization: {0}")]
    UnknownOrganization(String),
    #[error("user {0} is not a member of organization {1}")]
    NotAMember(String, String),
    #[error("invalid or expired session")]
    InvalidSession,
    #[error("user {0} has no access to project {1}")]
    NoProjectAccess(String, String),
}

/// A seedable user fixture for the in-memory identity store.
#[derive(Debug, Clone)]
pub struct SeedUser {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub handle: String,
    /// Plain-text password (alpha only; production must use a credential store).
    pub password: String,
}

/// A seedable organization fixture.
#[derive(Debug, Clone)]
pub struct SeedOrganization {
    pub organization_id: String,
    pub slug: String,
    pub display_name: String,
}

/// A seedable membership fixture linking a user to an organization with a role.
#[derive(Debug, Clone)]
pub struct SeedMembership {
    pub user_id: String,
    pub organization_id: String,
    pub role: Role,
}

/// A seedable project access rule.
#[derive(Debug, Clone)]
pub struct SeedProjectAccess {
    pub user_id: String,
    pub organization_id: String,
    pub all_projects: bool,
    pub projects: Vec<(String, Role)>,
}

/// In-memory hosted identity + session store.
#[derive(Clone)]
pub struct HostedIdentityStore {
    inner: Arc<Mutex<IdentityState>>,
    session_ttl: Duration,
}

#[derive(Default)]
struct IdentityState {
    users: HashMap<String, StoredUser>,
    users_by_email: HashMap<String, String>,
    orgs: HashMap<String, Organization>,
    orgs_by_slug: HashMap<String, String>,
    memberships: HashMap<String, Vec<Membership>>, // keyed by user_id
    project_access: HashMap<String, Vec<ProjectAccess>>, // keyed by user_id
    sessions: HashMap<String, Session>, // keyed by token
    sessions_by_id: HashMap<String, String>, // session_id -> token
}

#[derive(Clone)]
struct StoredUser {
    user: HostedUser,
    password: String,
}

impl HostedIdentityStore {
    /// Create an empty store with the default session TTL (24h).
    pub fn new() -> Self {
        Self::with_ttl(Duration::hours(24))
    }

    /// Create an empty store with a custom session TTL.
    pub fn with_ttl(session_ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(IdentityState::default())),
            session_ttl,
        }
    }

    /// Seed the store with users, organizations, memberships, and project
    /// access rules. Idempotent: re-seeding the same IDs replaces prior data.
    pub fn seed(
        &self,
        users: Vec<SeedUser>,
        orgs: Vec<SeedOrganization>,
        memberships: Vec<SeedMembership>,
        project_access: Vec<SeedProjectAccess>,
    ) {
        let mut state = self.inner.lock().expect("identity store mutex poisoned");
        for u in users {
            let user = HostedUser {
                schema_version: SchemaVersion::default(),
                user_id: u.user_id.clone(),
                email: u.email.clone(),
                display_name: u.display_name,
                handle: u.handle,
            };
            state.users_by_email.insert(u.email.to_lowercase(), u.user_id.clone());
            state.users.insert(
                u.user_id,
                StoredUser {
                    user,
                    password: u.password,
                },
            );
        }
        for o in orgs {
            let org = Organization {
                schema_version: SchemaVersion::default(),
                organization_id: o.organization_id.clone(),
                slug: o.slug.clone(),
                display_name: o.display_name,
            };
            state.orgs_by_slug.insert(o.slug.clone(), o.organization_id.clone());
            state.orgs.insert(o.organization_id, org);
        }
        for m in memberships {
            let membership = Membership {
                schema_version: SchemaVersion::default(),
                membership_id: Uuid::new_v4().to_string(),
                user_id: m.user_id.clone(),
                organization_id: m.organization_id.clone(),
                role: m.role,
            };
            state
                .memberships
                .entry(m.user_id)
                .or_default()
                .push(membership);
        }
        for pa in project_access {
            let access = ProjectAccess {
                schema_version: SchemaVersion::default(),
                user_id: pa.user_id.clone(),
                organization_id: pa.organization_id.clone(),
                all_projects: pa.all_projects,
                projects: pa
                    .projects
                    .into_iter()
                    .map(|(project_id, role)| {
                        crate::opensymphony_gateway_schema::identity::ProjectAccessEntry {
                            project_id,
                            role,
                        }
                    })
                    .collect(),
            };
            state
                .project_access
                .entry(pa.user_id)
                .or_default()
                .push(access);
        }
    }

    /// Resolve a user's membership in an organization, returning the role.
    pub fn membership_role(
        &self,
        user_id: &str,
        organization_id: &str,
    ) -> Result<Role, IdentityError> {
        let state = self.inner.lock().expect("identity store mutex poisoned");
        if !state.users.contains_key(user_id) {
            return Err(IdentityError::UnknownUser(user_id.into()));
        }
        if !state.orgs.contains_key(organization_id) {
            return Err(IdentityError::UnknownOrganization(organization_id.into()));
        }
        let memberships = state.memberships.get(user_id);
        let role = memberships
            .and_then(|ms| {
                ms.iter()
                    .find(|m| m.organization_id == organization_id)
                    .map(|m| m.role)
            })
            .ok_or_else(|| {
                IdentityError::NotAMember(user_id.into(), organization_id.into())
            })?;
        Ok(role)
    }

    /// Resolve an organization by slug.
    pub fn organization_by_slug(&self, slug: &str) -> Option<Organization> {
        let state = self.inner.lock().expect("identity store mutex poisoned");
        state
            .orgs_by_slug
            .get(slug)
            .and_then(|id| state.orgs.get(id).cloned())
    }

    /// Resolve an organization by id.
    pub fn organization(&self, organization_id: &str) -> Option<Organization> {
        self.inner.lock().expect("identity store mutex poisoned").orgs.get(organization_id).cloned()
    }

    /// Resolve a user by id.
    pub fn user(&self, user_id: &str) -> Option<HostedUser> {
        self.inner.lock().expect("identity store mutex poisoned").users.get(user_id).map(|s| s.user.clone())
    }

    /// Authenticate a user and issue a session token bound to an organization.
    pub fn login(&self, request: &LoginRequest) -> Result<LoginResponse, IdentityError> {
        let state = self.inner.lock().expect("identity store mutex poisoned");
        let user_id = state
            .users_by_email
            .get(&request.email.to_lowercase())
            .cloned()
            .ok_or(IdentityError::InvalidCredentials)?;
        let stored = state
            .users
            .get(&user_id)
            .ok_or(IdentityError::InvalidCredentials)?;
        if stored.password != request.password {
            return Err(IdentityError::InvalidCredentials);
        }
        let user = stored.user.clone();

        // Resolve the target organization: explicit slug, else the user's
        // first membership.
        let organization = if let Some(slug) = &request.organization_slug {
            state
                .orgs_by_slug
                .get(slug)
                .and_then(|id| state.orgs.get(id))
                .cloned()
                .ok_or(IdentityError::UnknownOrganization(slug.clone()))?
        } else {
            let membership = state
                .memberships
                .get(&user_id)
                .and_then(|ms| ms.first())
                .ok_or_else(|| {
                    IdentityError::NotAMember(user_id.clone(), "<none>".into())
                })?;
            state
                .orgs
                .get(&membership.organization_id)
                .cloned()
                .ok_or(IdentityError::UnknownOrganization(
                    membership.organization_id.clone(),
                ))?
        };

        let role = state
            .memberships
            .get(&user_id)
            .and_then(|ms| {
                ms.iter()
                    .find(|m| m.organization_id == organization.organization_id)
                    .map(|m| m.role)
            })
            .ok_or_else(|| {
                IdentityError::NotAMember(user_id.clone(), organization.organization_id.clone())
            })?;

        drop(state);

        let token = Uuid::new_v4().to_string();
        let session_id = Uuid::new_v4().to_string();
        let issued_at = Utc::now();
        let expires_at = issued_at + self.session_ttl;
        let session = Session {
            schema_version: SchemaVersion::default(),
            session_id: session_id.clone(),
            token: token.clone(),
            user_id: user.user_id.clone(),
            organization_id: organization.organization_id.clone(),
            issued_at: issued_at.to_rfc3339(),
            expires_at: expires_at.to_rfc3339(),
        };
        {
            let mut w = self.inner.lock().expect("identity store mutex poisoned");
            w.sessions_by_id.insert(session_id, token.clone());
            w.sessions.insert(token.clone(), session);
        }

        Ok(LoginResponse {
            schema_version: SchemaVersion::default(),
            session_token: token,
            user,
            organization,
            role,
            expires_at: expires_at.to_rfc3339(),
        })
    }

    /// Validate a bearer token and return the authenticated context.
    pub fn auth_context_for_token(
        &self,
        token: &str,
    ) -> Result<AuthContext, IdentityError> {
        let state = self.inner.lock().expect("identity store mutex poisoned");
        let session = state
            .sessions
            .get(token)
            .cloned()
            .ok_or(IdentityError::InvalidSession)?;
        // Expiry check.
        let expires = chrono::DateTime::parse_from_rfc3339(&session.expires_at)
            .map_err(|_| IdentityError::InvalidSession)?;
        if Utc::now() > expires.with_timezone(&Utc) {
            return Err(IdentityError::InvalidSession);
        }
        let user = state
            .users
            .get(&session.user_id)
            .map(|s| s.user.clone())
            .ok_or(IdentityError::InvalidSession)?;
        let organization = state
            .orgs
            .get(&session.organization_id)
            .cloned()
            .ok_or(IdentityError::InvalidSession)?;
        let role = state
            .memberships
            .get(&session.user_id)
            .and_then(|ms| {
                ms.iter()
                    .find(|m| m.organization_id == session.organization_id)
                    .map(|m| m.role)
            })
            .ok_or(IdentityError::InvalidSession)?;
        Ok(AuthContext {
            schema_version: SchemaVersion::default(),
            user_id: user.user_id,
            organization_id: organization.organization_id,
            role,
            user_handle: user.handle,
            organization_slug: organization.slug,
            dev_bypass: false,
        })
    }

    /// Resolve a session probe response from a token.
    pub fn session_response_for_token(
        &self,
        token: &str,
    ) -> Result<SessionResponse, IdentityError> {
        let ctx = self.auth_context_for_token(token)?;
        let state = self.inner.lock().expect("identity store mutex poisoned");
        let user = state
            .users
            .get(&ctx.user_id)
            .map(|s| s.user.clone())
            .ok_or(IdentityError::InvalidSession)?;
        let organization = state
            .orgs
            .get(&ctx.organization_id)
            .cloned()
            .ok_or(IdentityError::InvalidSession)?;
        let session = state
            .sessions
            .get(token)
            .cloned()
            .ok_or(IdentityError::InvalidSession)?;
        Ok(SessionResponse {
            schema_version: SchemaVersion::default(),
            user,
            organization,
            role: ctx.role,
            expires_at: session.expires_at,
        })
    }

    /// Invalidate a session (logout).
    pub fn logout(&self, token: &str) -> Result<(), IdentityError> {
        let mut state = self.inner.lock().expect("identity store mutex poisoned");
        if let Some(session) = state.sessions.remove(token) {
            state.sessions_by_id.remove(&session.session_id);
        }
        Ok(())
    }

    /// True when the user may access a project within their organization.
    pub fn may_access_project(
        &self,
        user_id: &str,
        organization_id: &str,
        project_id: &str,
    ) -> Result<bool, IdentityError> {
        let state = self.inner.lock().expect("identity store mutex poisoned");
        // Must be a member first.
        let is_member = state
            .memberships
            .get(user_id)
            .map(|ms| ms.iter().any(|m| m.organization_id == organization_id))
            .unwrap_or(false);
        if !is_member {
            return Err(IdentityError::NotAMember(
                user_id.into(),
                organization_id.into(),
            ));
        }
        let access_list = state.project_access.get(user_id);
        let Some(access_list) = access_list else {
            // No explicit project access rules: membership grants all projects.
            return Ok(true);
        };
        for access in access_list {
            if access.organization_id != organization_id {
                continue;
            }
            if access.all_projects {
                return Ok(true);
            }
            if access.projects.iter().any(|p| p.project_id == project_id) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The role cap a user holds within a project (capped by org role).
    pub fn project_role_cap(
        &self,
        user_id: &str,
        organization_id: &str,
        project_id: &str,
    ) -> Result<Role, IdentityError> {
        let org_role = self.membership_role(user_id, organization_id)?;
        let state = self.inner.lock().expect("identity store mutex poisoned");
        let access_list = state.project_access.get(user_id);
        let project_role = access_list
            .and_then(|list| {
                list.iter()
                    .find(|a| a.organization_id == organization_id)
                    .and_then(|a| {
                        a.projects
                            .iter()
                            .find(|p| p.project_id == project_id)
                            .map(|p| p.role)
                    })
            })
            .unwrap_or(org_role);
        // Cap the project role at the org role.
        Ok(if project_role.ordinal() > org_role.ordinal() {
            org_role
        } else {
            project_role
        })
    }

    /// Resolve an `Org` by id for responses.
    pub fn organization_ref(&self, organization_id: &str) -> Option<Org> {
        self.organization(organization_id)
    }
}

impl Default for HostedIdentityStore {
    fn default() -> Self {
        Self::new()
    }
}