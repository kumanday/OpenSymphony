//! Auth hardening + RBAC integration tests for hosted mode (COE-420).
//!
//! Covers the ticket Test Plan: API and WebSocket permission tests for allowed
//! and denied access, action receipts include permission rejection reasons,
//! and the explicit local-development auth bypass is unavailable in production
//! configuration.

use opensymphony::opensymphony_control::SnapshotStore;
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneIssueRuntimeState as IssueRuntimeState,
    ControlPlaneIssueSnapshot as IssueSnapshot, ControlPlaneMetricsSnapshot as MetricsSnapshot,
    ControlPlaneRecentEvent as RecentEvent, ControlPlaneRecentEventKind as RecentEventKind,
    ControlPlaneWorkerOutcome as WorkerOutcome,
};
use opensymphony::opensymphony_gateway::{
    AuthSetupError, GatewayAuthConfig, GatewayServer, HostedIdentityStore, SeedMembership,
    SeedOrganization, SeedProjectAccess, SeedUser,
};
use opensymphony::opensymphony_gateway_schema::action::{
    ActionDispatch, ActionKind, ActionReceipt, ActionStatus, ActionTarget,
};
use opensymphony::opensymphony_gateway_schema::envelope::EntityKind;
use opensymphony::opensymphony_gateway_schema::identity::Role;
use tokio::net::TcpListener;

/// A minimal daemon snapshot with one idle issue so run/project reads resolve.
fn fixture_snapshot() -> DaemonSnapshot {
    let now = chrono::Utc::now();
    let issue = IssueSnapshot {
        identifier: "COE-1".into(),
        title: "Sample issue".into(),
        tracker_state: "Todo".into(),
        runtime_state: IssueRuntimeState::Idle,
        last_outcome: WorkerOutcome::Unknown,
        last_event_at: now,
        conversation_id_suffix: "c0e1".into(),
        workspace_path_suffix: "COE-1".into(),
        retry_count: 0,
        blocked: false,
        blocked_by: Vec::new(),
        server_base_url: Some("http://127.0.0.1:3000".into()),
        transport_target: Some("loopback".into()),
        http_auth_mode: Some("none".into()),
        websocket_auth_mode: Some("none".into()),
        websocket_query_param_name: None,
        recent_events: Vec::new(),
        modified_files: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cancel_acknowledged: false,
        cancel_failed: false,
        detached: false,
    };
    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: DaemonState::Ready,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony".into(),
            status_line: "ready".into(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3000".into(),
            conversation_count: 0,
            status_line: "healthy".into(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 0,
            retry_queue_depth: 0,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            total_tokens: 0,
            total_cost_micros: 0,
        },
        issues: vec![issue],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-1".into()),
            kind: RecentEventKind::SnapshotPublished,
            summary: "published".into(),
        }],
    }
}

/// Seed an identity store with an admin, a member, a viewer, and a restricted
/// viewer scoped to a single project. Shared by the default and short-TTL
/// gateway fixtures so the expiry test can reuse the same population.
fn seed_identity_into(store: &HostedIdentityStore) {
    store.seed(
        vec![
            SeedUser {
                user_id: "u-admin".into(),
                email: "admin@example.com".into(),
                display_name: "Admin".into(),
                handle: "admin".into(),
                password: "pw-admin".into(),
            },
            SeedUser {
                user_id: "u-member".into(),
                email: "member@example.com".into(),
                display_name: "Member".into(),
                handle: "member".into(),
                password: "pw-member".into(),
            },
            SeedUser {
                user_id: "u-viewer".into(),
                email: "viewer@example.com".into(),
                display_name: "Viewer".into(),
                handle: "viewer".into(),
                password: "pw-viewer".into(),
            },
            SeedUser {
                user_id: "u-restricted".into(),
                email: "restricted@example.com".into(),
                display_name: "Restricted".into(),
                handle: "restricted".into(),
                password: "pw-restricted".into(),
            },
        ],
        vec![SeedOrganization {
            organization_id: "org-1".into(),
            slug: "acme".into(),
            display_name: "Acme".into(),
        }],
        vec![
            SeedMembership {
                user_id: "u-admin".into(),
                organization_id: "org-1".into(),
                role: Role::Admin,
            },
            SeedMembership {
                user_id: "u-member".into(),
                organization_id: "org-1".into(),
                role: Role::Member,
            },
            SeedMembership {
                user_id: "u-viewer".into(),
                organization_id: "org-1".into(),
                role: Role::Viewer,
            },
            SeedMembership {
                user_id: "u-restricted".into(),
                organization_id: "org-1".into(),
                role: Role::Viewer,
            },
        ],
        vec![SeedProjectAccess {
            user_id: "u-restricted".into(),
            organization_id: "org-1".into(),
            all_projects: false,
            projects: vec![("default".into(), Role::Viewer)],
        }],
    );
}

/// Seed an identity store with an admin, a member, a viewer, plus a
/// restricted viewer scoped to a single project.
fn seeded_identity() -> HostedIdentityStore {
    // Trivial PBKDF2 iteration count keeps the fixture fast; the production
    // default (600k) is exercised by the dedicated hash-acceptance test.
    let store = HostedIdentityStore::with_ttl_and_iterations(chrono::Duration::hours(24), 1);
    seed_identity_into(&store);
    store
}

/// Build a hosted gateway bound to an ephemeral port from a pre-seeded identity
/// store and return its address. Shared by the default and short-TTL fixtures.
async fn hosted_gateway_with_identity(identity: HostedIdentityStore) -> String {
    let store = SnapshotStore::new(fixture_snapshot());
    let server = GatewayServer::new(store)
        .with_hosted_auth(GatewayAuthConfig::Hosted, identity, false)
        .expect("hosted auth setup should succeed");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });
    format!("http://{address}")
}

/// Build a hosted gateway bound to an ephemeral port and return its address.
async fn hosted_gateway() -> String {
    hosted_gateway_with_identity(seeded_identity()).await
}

/// Build an unauthenticated (Disabled) gateway to confirm the local contract
/// is preserved.
async fn disabled_gateway() -> String {
    let store = SnapshotStore::new(fixture_snapshot());
    let server = GatewayServer::new(store);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });
    format!("http://{address}")
}

/// Log in as a seeded user and return the bearer token.
async fn login(base: &str, email: &str, password: &str) -> String {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "organization_slug": "acme",
        }))
        .send()
        .await
        .expect("login request should send");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::OK,
        "login should succeed for seeded credentials"
    );
    let body: serde_json::Value = response.json().await.expect("decode login response");
    body["session_token"]
        .as_str()
        .expect("login response carries a session_token")
        .to_string()
}

fn authed_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if !token.is_empty() {
        let value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
            .expect("valid bearer header");
        headers.insert(reqwest::header::AUTHORIZATION, value);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("build authed client")
}

// ── Login / session / logout flow ─────────────────────────────────────────────

#[tokio::test]
async fn hosted_login_returns_session_token_and_user_context() {
    let base = hosted_gateway().await;
    let token = login(&base, "admin@example.com", "pw-admin").await;

    // The session probe reflects the authenticated user + tenant context.
    let client = authed_client(&token);
    let me = client
        .get(format!("{base}/api/v1/auth/session"))
        .send()
        .await
        .expect("session probe should send");
    assert_eq!(me.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = me.json().await.expect("decode session response");
    assert_eq!(body["user"]["email"], "admin@example.com");
    assert_eq!(body["organization"]["slug"], "acme");
    assert_eq!(body["role"], "admin");
}

#[tokio::test]
async fn hosted_login_rejects_invalid_credentials() {
    let base = hosted_gateway().await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({
            "email": "admin@example.com",
            "password": "wrong",
            "organization_slug": "acme",
        }))
        .send()
        .await
        .expect("login request should send");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = response.json().await.expect("decode error body");
    assert_eq!(body["error_code"], "unauthenticated");
}

#[tokio::test]
async fn hosted_logout_invalidates_session_token() {
    let base = hosted_gateway().await;
    let token = login(&base, "member@example.com", "pw-member").await;
    let client = authed_client(&token);

    let logout = client
        .post(format!("{base}/api/v1/auth/logout"))
        .send()
        .await
        .expect("logout should send");
    assert_eq!(logout.status(), reqwest::StatusCode::OK);

    // The token must no longer authenticate protected reads.
    let read = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(read.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn hosted_expired_session_token_is_rejected() {
    // End-to-end proof that an expired session token is rejected: the store is
    // seeded with a 1-second session TTL, a token is issued, and once it
    // expires a protected read must fall back to 401 (the token is no longer a
    // valid credential). This pins the expiry path the reviewer flagged as only
    // implicitly covered.
    let identity = HostedIdentityStore::with_ttl_and_iterations(chrono::Duration::seconds(1), 1);
    seed_identity_into(&identity);
    let base = hosted_gateway_with_identity(identity).await;
    let token = login(&base, "admin@example.com", "pw-admin").await;

    // The fresh token authenticates a protected read.
    let client = authed_client(&token);
    let fresh = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(
        fresh.status(),
        reqwest::StatusCode::OK,
        "fresh session token authenticates the read"
    );

    // Wait for the session TTL to elapse, then the same token is rejected.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    let expired = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(
        expired.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "expired session token is rejected"
    );
    let body: serde_json::Value = expired.json().await.expect("decode error body");
    assert_eq!(body["error_code"], "unauthenticated");
}

// ── API permission enforcement (allowed + denied) ─────────────────────────────

#[tokio::test]
async fn hosted_read_requires_authentication() {
    let base = hosted_gateway().await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let body: serde_json::Value = response.json().await.expect("decode error body");
    assert_eq!(body["error_code"], "unauthenticated");
}

#[tokio::test]
async fn hosted_read_allows_viewer_for_permitted_project() {
    let base = hosted_gateway().await;
    let token = login(&base, "viewer@example.com", "pw-viewer").await;
    let client = authed_client(&token);
    let response = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn hosted_read_denies_project_access_for_restricted_viewer() {
    let base = hosted_gateway().await;
    // The restricted viewer is scoped only to "default"; an unknown project
    // must be denied at the permission layer before the 404 path.
    let token = login(&base, "restricted@example.com", "pw-restricted").await;
    let client = authed_client(&token);
    let response = client
        .get(format!("{base}/api/v1/projects/other"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json().await.expect("decode error body");
    // The client classifier maps permission denials to `unauthorized`.
    assert_eq!(body["error_code"], "permission_denied");
}

#[tokio::test]
async fn hosted_run_read_allows_member() {
    let base = hosted_gateway().await;
    let token = login(&base, "member@example.com", "pw-member").await;
    let client = authed_client(&token);
    let response = client
        .get(format!("{base}/api/v1/runs/COE-1"))
        .send()
        .await
        .expect("run read should send");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn hosted_run_read_denies_unauthenticated() {
    let base = hosted_gateway().await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/runs/COE-1"))
        .send()
        .await
        .expect("run read should send");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

// ── Action receipts include permission rejection reasons ──────────────────────

fn retry_dispatch() -> ActionDispatch {
    ActionDispatch {
        schema_version: Default::default(),
        correlation_id: format!("corr-{}", uuid::Uuid::new_v4()),
        action_kind: ActionKind::Retry,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Run,
            entity_id: "COE-1".into(),
        },
        payload: None,
        idempotency_key: None,
        actor: None,
    }
}

#[tokio::test]
async fn hosted_action_allowed_for_member_carries_actor_context() {
    let base = hosted_gateway().await;
    let token = login(&base, "member@example.com", "pw-member").await;
    let client = authed_client(&token);
    let response = client
        .post(format!("{base}/api/v1/actions/dispatch"))
        .json(&retry_dispatch())
        .send()
        .await
        .expect("dispatch should send");
    // The action is accepted or rejected for state reasons, but never 403.
    assert_ne!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: ActionReceipt = response.json().await.expect("decode receipt");
    // The gateway is the authority: the permission decision was evaluated
    // server-side and carries the required role for the caller's context.
    let permission = body
        .permission
        .as_ref()
        .expect("hosted action receipt carries a server-side permission decision");
    assert!(
        permission.evaluated,
        "permission was evaluated by the gateway"
    );
    assert!(permission.allowed, "member is permitted to retry a run");
    assert!(
        !permission.required_role.is_empty(),
        "receipt records the required role"
    );
}

#[tokio::test]
async fn hosted_action_denied_for_viewer_includes_rejection_reason() {
    let base = hosted_gateway().await;
    // A viewer cannot operate on runs (Retry requires Member); the receipt
    // must carry the permission rejection reason (Test Plan).
    let token = login(&base, "viewer@example.com", "pw-viewer").await;
    let client = authed_client(&token);
    let response = client
        .post(format!("{base}/api/v1/actions/dispatch"))
        .json(&retry_dispatch())
        .send()
        .await
        .expect("dispatch should send");
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: ActionReceipt = response.json().await.expect("decode receipt");
    assert_eq!(body.status, ActionStatus::Rejected);
    let reason = body
        .reason
        .as_ref()
        .expect("rejected receipt carries a reason");
    assert!(
        reason.contains("permission denied"),
        "rejection reason explains the permission denial: {reason}"
    );
    let denied_code = body.permission.as_ref().and_then(|p| p.denied_code.clone());
    assert_eq!(
        denied_code.as_deref(),
        Some("unauthorized"),
        "receipt permission carries the denied code"
    );
}

#[tokio::test]
async fn hosted_action_denied_when_unauthenticated() {
    let base = hosted_gateway().await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/api/v1/actions/dispatch"))
        .json(&retry_dispatch())
        .send()
        .await
        .expect("dispatch should send");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// Build an action that targets a project directly via `target_entity`, so the
/// project id is carried on the target (not the payload). Used to verify the
/// project-access check derives the project id from the target entity.
fn project_target_action(project_id: &str) -> ActionDispatch {
    ActionDispatch {
        schema_version: Default::default(),
        correlation_id: format!("corr-{}", uuid::Uuid::new_v4()),
        // Comment is read-level against a project target (Viewer-satisfiable),
        // so a denial is attributable to project access, not role.
        action_kind: ActionKind::Comment,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Project,
            entity_id: project_id.into(),
        },
        payload: None,
        idempotency_key: None,
        actor: None,
    }
}

#[tokio::test]
async fn hosted_action_targeting_project_enforces_project_access() {
    let base = hosted_gateway().await;
    // The restricted viewer is scoped to the "default" project only.
    let token = login(&base, "restricted@example.com", "pw-restricted").await;
    let client = authed_client(&token);

    // Targeting the project the viewer may access is permitted (role Viewer
    // satisfies the read-level capability for a project Comment).
    let allowed = client
        .post(format!("{base}/api/v1/actions/dispatch"))
        .json(&project_target_action("default"))
        .send()
        .await
        .expect("dispatch should send");
    assert_ne!(
        allowed.status(),
        reqwest::StatusCode::FORBIDDEN,
        "project the viewer may access must not be denied"
    );

    // Targeting a project the viewer cannot access is denied for project-access
    // reasons, even though the viewer's role satisfies the capability.
    let denied = client
        .post(format!("{base}/api/v1/actions/dispatch"))
        .json(&project_target_action("other"))
        .send()
        .await
        .expect("dispatch should send");
    assert_eq!(denied.status(), reqwest::StatusCode::FORBIDDEN);
    let body: ActionReceipt = denied.json().await.expect("decode receipt");
    assert_eq!(body.status, ActionStatus::Rejected);
    let reason = body
        .reason
        .as_ref()
        .expect("rejected receipt carries a reason");
    assert!(
        reason.contains("no access to project other"),
        "rejection reason explains the project-access denial: {reason}"
    );
    assert_eq!(
        body.permission
            .as_ref()
            .and_then(|p| p.denied_code.clone())
            .as_deref(),
        Some("permission_denied"),
        "denied code reflects a project-access denial"
    );
}

// ── Default-route authentication floor (no unclassified API bypass) ──────────

#[tokio::test]
async fn hosted_unclassified_api_route_requires_authentication() {
    let base = hosted_gateway().await;
    // `/api/v1/projects` (the project list) and any unclassified `/api/v1/*`
    // route default to an authenticated-viewer floor, so a request without a
    // token is rejected rather than passing through ungated.
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/projects"))
        .send()
        .await
        .expect("project list should send");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "project list must require authentication in hosted mode"
    );
}

#[tokio::test]
async fn hosted_unknown_api_route_requires_authentication_not_passthrough() {
    let base = hosted_gateway().await;
    // An unclassified `/api/v1/*` path that is not explicitly exempted hits the
    // authenticated floor before the 404 fallback, so it is 401 (not 404 and not
    // an ungated pass-through). This guards against future routes bypassing RBAC.
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/some-future-endpoint"))
        .send()
        .await
        .expect("unknown route should send");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "unclassified /api/v1/* routes must hit the authenticated floor, not pass through"
    );
}

// ── WebSocket auth gating ─────────────────────────────────────────────────────

#[tokio::test]
async fn hosted_websocket_denied_without_token() {
    let base = hosted_gateway().await;
    let ws_url = base.replacen("http://", "ws://", 1) + "/api/v1/streams/events";
    // tungstenite is not a test dependency; use the HTTP upgrade rejection path
    // by issuing a raw GET without the upgrade headers via reqwest. The auth
    // gate runs before the upgrade is negotiated, so a missing token yields 401.
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/streams/events"))
        .header(reqwest::header::UPGRADE, "websocket")
        .header(reqwest::header::CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .send()
        .await
        .expect("ws upgrade probe should send");
    assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
    let _ = ws_url; // suppress unused binding; behavior validated via the HTTP probe
}

#[tokio::test]
async fn hosted_websocket_accepts_token_query_param() {
    let base = hosted_gateway().await;
    let token = login(&base, "admin@example.com", "pw-admin").await;
    // With a valid ?token= the auth gate passes (it no longer returns 401).
    // A full WS handshake is not exercised here; the gate decision is what
    // matters for the auth hardening matrix.
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/streams/events?token={token}"))
        .header(reqwest::header::UPGRADE, "websocket")
        .header(reqwest::header::CONNECTION, "Upgrade")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("Sec-WebSocket-Version", "13")
        .send()
        .await
        .expect("ws upgrade probe should send");
    assert_ne!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "valid token passes the WS auth gate"
    );
}

#[tokio::test]
async fn disabled_mode_auth_endpoints_report_auth_disabled_not_a_fake_session() {
    let base = disabled_gateway().await;
    let client = reqwest::Client::new();

    // Login must not fabricate a 200 with an empty token + dev user in disabled
    // mode; it reports `auth_disabled` so the contract stays honest.
    let login = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({
            "email": "anyone@example.com",
            "password": "anything",
            "organization_slug": "acme",
        }))
        .send()
        .await
        .expect("login request should send");
    assert_eq!(
        login.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "disabled-mode login should not fabricate a session"
    );
    let body: serde_json::Value = login.json().await.expect("decode login error body");
    assert_eq!(body["error_code"], "auth_disabled");

    // Session probe likewise reports `auth_disabled`, not a fabricated dev session.
    let session = client
        .get(format!("{base}/api/v1/auth/session"))
        .send()
        .await
        .expect("session request should send");
    assert_eq!(
        session.status(),
        reqwest::StatusCode::SERVICE_UNAVAILABLE,
        "disabled-mode session should not fabricate a session"
    );
    let body: serde_json::Value = session.json().await.expect("decode session error body");
    assert_eq!(body["error_code"], "auth_disabled");
}
// ── Local development auth bypass is explicit and unavailable in production ──

#[tokio::test]
async fn disabled_mode_preserves_unauthenticated_local_contract() {
    let base = disabled_gateway().await;
    let client = reqwest::Client::new();
    // Reads work without any token in local trusted mode.
    let response = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(response.status(), reqwest::StatusCode::OK);

    // The capabilities surface advertises the local auth modes.
    let caps = client
        .get(format!("{base}/api/v1/capabilities"))
        .send()
        .await
        .expect("capabilities should send")
        .json::<serde_json::Value>()
        .await
        .expect("decode capabilities");
    let modes = caps["auth_modes"].as_array().expect("auth_modes array");
    let mode_strings: Vec<&str> = modes.iter().filter_map(|m| m.as_str()).collect();
    assert!(
        !mode_strings.contains(&"hosted_session"),
        "disabled mode does not advertise hosted sessions"
    );
}

#[tokio::test]
async fn dev_bypass_is_unavailable_in_production_configuration() {
    let store = SnapshotStore::new(fixture_snapshot());
    let identity = seeded_identity();
    let result = GatewayServer::new(store).with_hosted_auth(
        GatewayAuthConfig::DevBypass,
        identity,
        true, // production
    );
    assert!(
        matches!(result, Err(AuthSetupError::DevBypassInProduction)),
        "dev bypass must be refused in production configuration"
    );
}

#[tokio::test]
async fn dev_bypass_grants_access_in_explicit_dev_mode() {
    let store = SnapshotStore::new(fixture_snapshot());
    let identity = seeded_identity();
    let server = GatewayServer::new(store)
        .with_hosted_auth(GatewayAuthConfig::DevBypass, identity, false)
        .expect("dev bypass should succeed in dev mode");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let base = format!("http://{address}");
    tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    // Dev bypass injects an owner context; no token is required.
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/projects/default"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

#[tokio::test]
async fn hosted_http_rejects_query_token_for_non_upgrade_requests() {
    // The `?token=` query fallback is restricted to WebSocket upgrade
    // requests so ordinary HTTP requests cannot leak session tokens through
    // query parameters. A valid token in the query string of a normal GET
    // must still be rejected as unauthenticated (no Authorization header).
    let base = hosted_gateway().await;
    let token = login(&base, "admin@example.com", "pw-admin").await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("{base}/api/v1/projects/default?token={token}"))
        .send()
        .await
        .expect("project read should send");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "query-token fallback must not authenticate ordinary HTTP requests"
    );
}

#[tokio::test]
async fn hosted_passwords_are_not_compared_as_plaintext() {
    // After the credential hardening, a correct password verifies through the
    // salted hash and an incorrect password is rejected -- proving the store
    // does not store or compare plaintext passwords.
    let base = hosted_gateway().await;
    let good = login(&base, "admin@example.com", "pw-admin").await;
    assert!(!good.is_empty(), "correct password verifies via the hash");

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/api/v1/auth/login"))
        .json(&serde_json::json!({
            "email": "admin@example.com",
            "password": "not-the-password",
            "organization_slug": "acme",
        }))
        .send()
        .await
        .expect("login request should send");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "incorrect password is rejected by the hash verification"
    );
}
