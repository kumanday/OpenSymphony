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
    extract::{FromRequestParts, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use percent_encoding::percent_decode_str;

use crate::opensymphony_gateway::identity_store::{HostedIdentityStore, IdentityError};
use crate::opensymphony_gateway::rbac::{PermissionEvaluator, ProtectedResource};
use crate::opensymphony_gateway_schema::identity::{
    AuthContext, AuthErrorBody, AuthErrorCode, LoginRequest, LoginResponse, SessionResponse,
};

/// Map a denied-code string from a `PermissionResult` into the wire
/// `AuthErrorCode` the client classifier recognizes.
fn denied_code_to_auth_error(code: &str) -> AuthErrorCode {
    match code {
        "permission_denied" => AuthErrorCode::PermissionDenied,
        "forbidden_resource" => AuthErrorCode::ForbiddenResource,
        _ => AuthErrorCode::Unauthorized,
    }
}

/// Build a 403 response from a denied `PermissionResult`, carrying the
/// `error_code` the client maps to `unauthorized` and the denial reason.
pub(crate) fn permission_denied_response(
    permission: &crate::opensymphony_gateway_schema::action::PermissionResult,
) -> Response {
    let code = permission
        .denied_code
        .as_deref()
        .map(denied_code_to_auth_error)
        .unwrap_or(AuthErrorCode::Unauthorized);
    unauthorized_response(
        code,
        permission
            .denied_reason
            .clone()
            .unwrap_or_else(|| "permission denied".into()),
    )
}

/// Axum extractor for the authenticated principal, pulled from request
/// extensions (injected by `auth_middleware`). Implements `FromRequestParts`
/// so it composes with body-consuming extractors like `Json`. Yields `None`
/// when auth is disabled or the route is public.
#[derive(Debug, Clone, Default)]
pub struct AuthPrincipal(pub Option<AuthContext>);

impl<S: Send + Sync> FromRequestParts<S> for AuthPrincipal {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        Ok(AuthPrincipal(
            parts.extensions.get::<AuthContext>().cloned(),
        ))
    }
}

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
        matches!(
            self,
            GatewayAuthConfig::DevBypass | GatewayAuthConfig::Hosted
        )
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
        Self {
            identity,
            dev_bypass,
        }
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
            Ok(Some(Arc::new(SessionTokenAuthProvider::new(
                identity, true,
            ))))
        }
        GatewayAuthConfig::Hosted => Ok(Some(Arc::new(SessionTokenAuthProvider::new(
            identity, false,
        )))),
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
    let rest = trimmed.strip_prefix("Bearer ")?;
    Some(rest.trim())
}

/// Auth middleware state shared with the `from_fn_with_state` layer.
#[derive(Clone)]
pub struct AuthMiddlewareState {
    pub provider: Option<Arc<dyn HostedAuthProvider>>,
    pub config: GatewayAuthConfig,
}

/// Extract the bearer token from a request's `Authorization` header, mapping a
/// missing/malformed header to a 401 response. Shared by the logout and session
/// handlers so token parsing and the unauthenticated error stay consistent.
/// The `Err` variant is boxed to keep the `Result` small (`Response` is large).
fn require_bearer_token(request: &Request) -> Result<String, Box<Response>> {
    bearer_token_from_header(
        request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    )
    .map(|t| t.to_string())
    .ok_or_else(|| {
        Box::new(unauthenticated_response(
            "missing Authorization bearer token",
        ))
    })
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

    let header_token = bearer_token_from_header(
        request
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok()),
    );
    // Browser fallback for WebSocket upgrades only: browsers cannot set
    // `Authorization` headers on a WS upgrade, so accept a `?token=` query
    // parameter as an equivalent bearer credential *only* for upgrade
    // requests. Restricting the fallback to upgrades avoids leaking session
    // tokens through server logs, proxies, browser history, and referrer
    // headers that capture query parameters on ordinary HTTP requests. The
    // token is still validated by the provider below.
    let query_token = if is_websocket_upgrade(request.headers()) {
        ws_query_token(request.uri())
    } else {
        None
    };
    let token: &str = match header_token.or(query_token.as_deref()) {
        Some(token) => token,
        None => return unauthenticated_response("missing or malformed Authorization bearer token"),
    };
    let Some(ctx) = provider.auth_context_for_token(token) else {
        return unauthenticated_response("invalid or expired session token");
    };
    request.extensions_mut().insert(ctx);
    next.run(request).await
}

/// State shared with the read-RBAC middleware layer.
#[derive(Clone)]
pub struct RbacMiddlewareState {
    /// `None` in local trusted mode (no enforcement); `Some` in hosted/dev-bypass.
    pub evaluator: Option<PermissionEvaluator>,
}

/// Classify a protected read route into the resource it exposes and an
/// optional project id for project-scoped access checks.
///
/// Returns `None` for routes the read-RBAC layer does not gate (public routes,
/// the action dispatch endpoint, auth routes, web assets, the WebSocket event
/// stream, and `/api/v1/taskgraph/*` mutations) so they pass through unchanged.
///
/// For every other `/api/v1/*` route, classification defaults to an
/// authenticated-viewer floor (`Project` with no project id) instead of passing
/// through ungated. This prevents a future `/api/v1/*` endpoint from silently
/// bypassing RBAC: any new sensitive route is authenticated by default, and an
/// explicit passthrough must be added here for routes that handle their own
/// auth/RBAC. Action dispatch is gated per-handler because it needs the parsed
/// `ActionDispatch` to classify the targeted resource.
fn classify_read_route(path: &str) -> Option<(ProtectedResource, Option<String>)> {
    if is_public_route(path) || path == "/api/v1/actions/dispatch" {
        return None;
    }
    // Explicit passthrough: routes that handle their own auth/RBAC and must not
    // be gated by this read layer.
    if path == "/api/v1/streams/events"
        || path.starts_with("/api/v1/auth/")
        || path.starts_with("/api/v1/taskgraph/")
    {
        return None;
    }
    // Project-scoped reads: /api/v1/projects/{id} and the task graph view.
    if let Some(rest) = path.strip_prefix("/api/v1/projects/") {
        let project_id = rest.split('/').next().filter(|s| !s.is_empty());
        return Some((ProtectedResource::Project, project_id.map(str::to_owned)));
    }
    // Run-scoped reads: the run detail and every sub-resource (events, files,
    // diffs, validation, approvals, timeline, logs, terminal). Runs are
    // tenant-scoped issue identifiers; project isolation of run data is out of
    // scope for the alpha, so the check is an authenticated-viewer floor.
    if path.starts_with("/api/v1/runs/") {
        return Some((ProtectedResource::Run, None));
    }
    // Org-wide reads: the project list, snapshots, dashboard, event journal,
    // and the control / event stream read endpoints. These require an
    // authenticated viewer but are not project-scoped.
    match path {
        "/api/v1/projects"
        | "/api/v1/snapshot"
        | "/api/v1/dashboard/snapshot"
        | "/api/v1/events"
        | "/api/v1/event-journal"
        | "/api/v1/control/events" => Some((ProtectedResource::Project, None)),
        // Default: any other `/api/v1/*` route is gated to an authenticated
        // viewer floor so unclassified API routes cannot bypass RBAC. Routes
        // that must pass through (e.g. action dispatch, auth, WS streams,
        // taskgraph mutations) are exempted above.
        _ if path.starts_with("/api/v1/") => Some((ProtectedResource::Project, None)),
        // Non-API paths (web assets under /app, healthz, etc.) are not gated
        // here; public assets are already exempted via `is_public_route`.
        _ => None,
    }
}

/// Read-RBAC middleware. Runs after `auth_middleware` and enforces a hosted
/// permission decision for classified read routes using the `AuthContext`
/// injected into request extensions. Passes through when no evaluator is
/// configured (local trusted mode) or the route is not classified.
pub async fn read_rbac_middleware(
    State(state): State<RbacMiddlewareState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(evaluator) = &state.evaluator else {
        return next.run(request).await;
    };
    let path = request.uri().path();
    let Some((resource, project_id)) = classify_read_route(path) else {
        return next.run(request).await;
    };
    let Some(ctx) = request.extensions().get::<AuthContext>().cloned() else {
        return unauthenticated_response("authenticated context required for hosted read access");
    };
    let permission = evaluator.evaluate_read(&ctx, resource, project_id.as_deref());
    if !permission.allowed {
        return permission_denied_response(&permission);
    }
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

/// Build a 503 response signaling that hosted auth is not enabled in this
/// gateway configuration (local trusted `Disabled` mode). Used by the
/// login/logout/session handlers so the auth endpoints do not fabricate a
/// success response with an empty token; local trusted mode requires no
/// session, and clients receive a clear, classifiable `auth_disabled` signal.
fn auth_disabled_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(AuthErrorBody {
            error_code: AuthErrorCode::AuthDisabled,
            message: "hosted auth is disabled in this configuration; local trusted mode requires no session"
                .into(),
        }),
    )
        .into_response()
}

/// Extractor for the authenticated context from request extensions.
pub fn auth_context_from_request(request: &Request) -> Option<AuthContext> {
    request.extensions().get::<AuthContext>().cloned()
}

/// True when the request is a WebSocket upgrade (carries an
/// `Upgrade: websocket` header). Used to restrict the `?token=` query fallback
/// to upgrade requests only, so ordinary HTTP requests must use the
/// `Authorization` header and do not leak tokens through query parameters.
pub fn is_websocket_upgrade(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get(axum::http::header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

/// Extract a `?token=` query parameter from a URI, for the browser WebSocket
/// auth fallback. The value is percent-decoded so a token containing reserved
/// characters survives the query string. Returns the parsed token when present,
/// `None` otherwise. A value that is not valid UTF-8 after decoding is treated
/// as absent.
pub fn ws_query_token(uri: &axum::http::Uri) -> Option<String> {
    let query = uri.query()?;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == "token"
        {
            return percent_decode_str(v)
                .decode_utf8()
                .ok()
                .map(|s| s.into_owned());
        }
    }
    None
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
pub fn auth_router<S: Clone + Send + Sync + 'static>(state: AuthRouterState) -> Router<S> {
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
        return auth_disabled_response();
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

async fn logout_handler(State(state): State<AuthRouterState>, request: Request) -> Response {
    let Some(provider) = state.provider.as_ref() else {
        return auth_disabled_response();
    };
    let token = match require_bearer_token(&request) {
        Ok(token) => token,
        Err(response) => return *response,
    };
    let _ = provider.logout(&token);
    (
        StatusCode::OK,
        Json(serde_json::json!({"logged_out": true})),
    )
        .into_response()
}

async fn session_handler(State(state): State<AuthRouterState>, request: Request) -> Response {
    let Some(provider) = state.provider.as_ref() else {
        return auth_disabled_response();
    };
    let token = match require_bearer_token(&request) {
        Ok(token) => token,
        Err(response) => return *response,
    };
    match provider.session_response(&token) {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(_) => unauthenticated_response("invalid or expired session token"),
    }
}
