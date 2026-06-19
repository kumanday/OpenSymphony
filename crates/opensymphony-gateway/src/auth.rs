//! Hosted authentication: config, provider strategy, middleware, and
//! login/logout/session endpoints.
//!
//! The gateway selects an auth strategy via `GatewayAuthConfig`:
//! - `Disabled`: local trusted mode (current behavior). No auth context.
//! - `DevBypass`: explicit local-development bypass that injects a dev
//!   `AuthContext` without credentials. Only valid when not production; the
//!   config refuses to build a dev-bypass provider when `production` is true
//!   (acceptance criterion).
//! - `Hosted`: require a valid session bearer token issued by
//!   `POST /api/v1/auth/login`. Enforces RBAC on reads, streams, and actions.
//!
//! Auth middleware runs on protected HTTP routes; WebSocket upgrades validate
//! the token at upgrade time (query param or init auth message) so both the
//! JSON event-stream and JSON-RPC-over-WebSocket protocols are gated.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};

use crate::opensymphony_gateway_schema::identity::{
    AuthContext, AuthErrorBody, AuthErrorCode, LoginRequest, LoginResponse, SessionResponse,
};

use crate::opensymphony_gateway::identity_store::{HostedIdentityStore, IdentityError};

/// Hosted auth configuration selected at gateway construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayAuthConfig {
    /// Local trusted mode: no authentication. Preserves the existing contract.
    Disabled,
    /// Explicit local-development bypass. Injects a dev `AuthContext`. Refused
    /// when the gateway is in production configuration.
    DevBypass,
    /// Hosted mode: session bearer tokens + RBAC enforcement.
    Hosted,
}

impl GatewayAuthConfig {
    /// True when the gateway advertises hosted/session auth.
    pub fn requires_session(&self) -> bool {
        matches!(self, GatewayAuthConfig::Hosted)
    }

    /// True when an auth context is injected for every request (dev bypass or
    /// hosted). `Disabled` produces no auth context.
    pub fn injects_context(&self) -> bool {
        matches!(self, GatewayAuthConfig::DevBypass | GatewayAuthConfig::Hosted)
    }
}

/// Hosted auth provider strategy. The session-token provider is the alpha
/// implementation; the trait lets a future provider (OAuth/SSO) plug in.
pub trait HostedAuthProvider: Send + Sync + 'static {
    /// Resolve an authenticated context from a bearer token. Returns
    /// `None` when the token is missing/invalid so the caller maps to 401.
    fn auth_context_for_token(&self, token: &str) -> Option<AuthContext>;

    /// Authenticate a login request and return a session response.
    fn login(&self, request: &LoginRequest) -> Result<LoginResponse, IdentityError>;

    /// Resolve a session probe response from a token.
    fn session_response(&self, token: &str) -> Result<SessionResponse, IdentityError>;

    /// Invalidate a session.
    fn logout(&self, token: &str) -> Result<(), IdentityError>;

    /// The dev-bypass context, when dev bypass is enabled.
    fn dev_bypass_context(&self) -> Option<AuthContext> {
        None
    }
}

/// Session-token auth provider backed by the in-memory identity store.
pub struct SessionTokenAuthProvider {
    identity: HostedIdentityStore,
    dev_bypass: bool,
}

impl SessionTokenAuthProvider {
    pub fn new(identity: HostedIdentityStore, dev_bypass: bool) -> Self {
        Self { identity, dev_bypass }
    }
}

impl HostedAuthProvider for SessionTokenAuthProvider {
    fn auth_context_for_token(&self, token: &str) -> Option<AuthContext> {
        self.identity.auth_context_for_token(token).ok()
    }

    fn login(&self, request: &LoginRequest) -> Result<LoginResponse, IdentityError> {
        self.identity.login(request)
    }

    fn session_response(&self, token: &str) -> Result<SessionResponse, IdentityError> {
        self.identity.session_response_for_token(token)
    }

    fn logout(&self, token: &str) -> Result<(), IdentityError> {
        self.identity.logout(token)
    }

    fn dev_bypass_context(&self) -> Option<AuthContext> {
        if self.dev_bypass {
            Some(AuthContext::dev_bypass())
        } else {
            None
        }
    }
}

/// Build the auth provider for a config, or `None` for `Disabled`.
///
/// Returns an error if `DevBypass` is selected while `production` is true:
/// the local development auth bypass must be explicit and unavailable in
/// production configuration (acceptance criterion).
pub fn build_auth_provider(
    config: GatewayAuthConfig,
    identity: HostedIdentityStore,
    production: bool,
) -> Result<Option<Arc<dyn HostedAuthProvider>>, AuthSetupError> {
    match config {
        GatewayAuthConfig::Disabled => Ok(None),
        GatewayAuthConfig::DevBypass => {
            if production {
                return Err(AuthSetupError::DevBypassInProduction);
            }
            Ok(Some(Arc::new(SessionTokenAuthProvider::new(identity, true))))
        }
        GatewayAuthConfig::Hosted => {
            Ok(Some(Arc::new(SessionTokenAuthProvider::new(identity, false))))
        }
    }
}

/// Error raised when auth configuration is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthSetupError {
    #[error("dev bypass auth is not available in production configuration")]
    DevBypassInProduction,
}

/// Public routes exempt from auth middleware. Capabilities and login must be
/// reachable before authentication so clients can negotiate auth modes and
/// sign in.
pub fn is_public_route(path: &str) -> bool {
    matches!(
        path,
        "/healthz" | "/api/v1/capabilities" | "/api/v1/auth/login"
    ) || path.starts_with("/app")
}

/// Extract a bearer token from an `Authorization: Bearer <token>` header.
pub fn bearer_token_from_header(value: Option<&str>) -> Option<&str> {
    let value = value?;
    let trimmed = value.trim();
    let rest = trimmed.strip_prefix("Bearer ")? ;
    Some(rest.trim())
}

/// Auth middleware state shared with the `from_fn_with_state` layer.
#[derive(Clone)]
pub struct AuthMiddlewareState {
    pub provider: Option<Arc<dyn HostedAuthProvider>>,
    pub config: GatewayAuthConfig,
}

/// Axum middleware that authenticates protected requests and injects an
/// `AuthContext` into request extensions. Public routes pass through.
pub async fn auth_middleware(
    State(auth_state): State<AuthMiddlewareState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();

    if is_public_route(&path) {
        return next.run(request).await;
    }

    let Some(provider) = auth_state.provider.as_ref() else {
        // Disabled: no auth context. Local trusted mode.
        return next.run(request).await;
    };

    // Dev bypass: inject the dev context without requiring credentials.
    if let Some(ctx) = provider.dev_bypass_context() {
        request.extensions_mut().insert(ctx);
        return next.run(request).await;
    }

    let token = bearer_token_from_header(
        request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    let Some(token) = token else {
        return unauthenticated_response("missing or malformed Authorization bearer token");
    };
    let Some(ctx) = provider.auth_context_for_token(token) else {
        return unauthenticated_response("invalid or expired session token");
    };
    request.extensions_mut().insert(ctx);
    next.run(request).await
}

/// Build a 401 response with the standard auth error body.
pub fn unauthenticated_response(message: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(AuthErrorBody {
            error_code: AuthErrorCode::Unauthenticated,
            message: message.into(),
        }),
    )
        .into_response()
}

/// Build a 403 permission-denial response with an `error_code` the client
/// classifier maps to `unauthorized`.
pub fn unauthorized_response(code: AuthErrorCode, message: impl Into<String>) -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(AuthErrorBody {
            error_code: code,
            message: message.into(),
        }),
    )
        .into_response()
}

/// Extractor for the authenticated context from request extensions.
pub fn auth_context_from_request(request: &Request) -> Option<AuthContext> {
    request.extensions().get::<AuthContext>().cloned()
}

/// Query params for WebSocket auth via `?token=`.
#[derive(Debug, serde::Deserialize)]
pub struct WsAuthQuery {
    #[serde(default)]
    pub token: Option<String>,
}

/// Resolve an auth context for a WebSocket upgrade request.
///
/// Validates the token from the `Authorization` header or the `?token=` query
/// parameter so both browser WS clients (which cannot always set headers) and
/// non-browser clients are supported. Returns `None` when unauthenticated.
pub fn ws_auth_context(
    provider: &dyn HostedAuthProvider,
    headers: &axum::http::HeaderMap,
    query_token: Option<&str>,
) -> Option<AuthContext> {
    if let Some(ctx) = provider.dev_bypass_context() {
        return Some(ctx);
    }
    if let Some(token) = bearer_token_from_header(
        headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    ) {
        return provider.auth_context_for_token(token);
    }
    if let Some(token) = query_token {
        return provider.auth_context_for_token(token);
    }
    None
}

/// Router for `/api/v1/auth/*` endpoints. Always mounted (login works in
/// hosted mode; in dev-bypass/disabled mode the endpoints report the mode).
pub fn auth_router<S: Clone + Send + Sync + 'static>(
    state: AuthRouterState,
) -> Router<S> {
    Router::new()
        .route("/login", post(login_handler))
        .route("/logout", post(logout_handler))
        .route("/session", get(session_handler))
        .with_state(state)
}

/// State for the auth router.
#[derive(Clone)]
pub struct AuthRouterState {
    pub provider: Option<Arc<dyn HostedAuthProvider>>,
    pub config: GatewayAuthConfig,
}

async fn login_handler(
    State(state): State<AuthRouterState>,
    Json(request): Json<LoginRequest>,
) -> Response {
    let Some(provider) = state.provider.as_ref() else {
        return (
            StatusCode::OK,
            Json(LoginResponse {
                schema_version: crate::opensymphony_gateway_schema::version::SchemaVersion::default(),
                session_token: String::new(),
                user: dev_user(),
                organization: dev_org(),
                role: crate::opensymphony_gateway_schema::identity::Role::Owner,
                expires_at: chrono::Utc::now().to_rfc3339(),
            }),
        )
            .into_response();
    };
    match provider.login(&request) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(IdentityError::InvalidCredentials) => (
            StatusCode::UNAUTHORIZED,
            Json(AuthErrorBody {
                error_code: AuthErrorCode::Unauthenticated,
                message: "invalid credentials".into(),
            }),
        )
            .into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            Json(AuthErrorBody {
                error_code: AuthErrorCode::Forbidden,
                message: err.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn logout_handler(
    State(state): State<AuthRouterState>,
    request: Request,
) -> Response {
    let Some(provider) = state.provider.as_ref() else {
        return (StatusCode::OK, Json(serde_json::json!({"logged_out": true}))).into_response();
    };
    let token = bearer_token_from_header(
        request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    let Some(token) = token else {
        return unauthenticated_response("missing Authorization bearer token");
    };
    let _ = provider.logout(token);
    (StatusCode::OK, Json(serde_json::json!({"logged_out": true}))).into_response()
}

async fn session_handler(
    State(state): State<AuthRouterState>,
    request: Request,
) -> Response {
    let Some(provider) = state.provider.as_ref() else {
        return (
            StatusCode::OK,
            Json(SessionResponse {
                schema_version: crate::opensymphony_gateway_schema::version::SchemaVersion::default(),
                user: dev_user(),
                organization: dev_org(),
                role: crate::opensymphony_gateway_schema::identity::Role::Owner,
                expires_at: chrono::Utc::now().to_rfc3339(),
            }),
        )
            .into_response();
    };
    let token = bearer_token_from_header(
        request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    let Some(token) = token else {
        return unauthenticated_response("missing Authorization bearer token");
    };
    match provider.session_response(token) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(_) => unauthenticated_response("invalid or expired session token"),
    }
}

fn dev_user() -> crate::opensymphony_gateway_schema::identity::HostedUser {
    use crate::opensymphony_gateway_schema::identity::HostedUser;
    use crate::opensymphony_gateway_schema::version::SchemaVersion;
    HostedUser {
        schema_version: SchemaVersion::default(),
        user_id: "dev-user".into(),
        email: "dev@local".into(),
        display_name: "Dev User".into(),
        handle: "dev-user".into(),
    }
}

fn dev_org() -> crate::opensymphony_gateway_schema::identity::Organization {
    use crate::opensymphony_gateway_schema::identity::Organization;
    use crate::opensymphony_gateway_schema::version::SchemaVersion;
    Organization {
        schema_version: SchemaVersion::default(),
        organization_id: "dev-org".into(),
        slug: "dev-org".into(),
        display_name: "Dev Organization".into(),
    }
}