use std::{collections::BTreeMap, path::Path};

use opensymphony_openhands::{
    HttpAuth, TransportAuthKind, TransportConfig, TransportTargetKind, WebSocketAuth,
};
use opensymphony_workflow::{ResolvedWorkflow, WorkflowDefinition};

fn resolve_workflow(source: &str, env: BTreeMap<String, String>) -> ResolvedWorkflow {
    let workflow = WorkflowDefinition::parse(source).expect("workflow should parse");
    workflow
        .resolve(Path::new("/repo"), &env)
        .expect("workflow should resolve")
}

#[test]
fn workflow_transport_config_uses_pinned_auto_auth_shape() {
    let workflow = resolve_workflow(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  transport:
    base_url: https://agent.example.com/runtime
    session_api_key_env: OPENHANDS_SESSION_API_KEY
---
{{ issue.identifier }}
"#,
        BTreeMap::from([
            ("LINEAR_API_KEY".to_string(), "linear-token".to_string()),
            (
                "OPENHANDS_SESSION_API_KEY".to_string(),
                "secret-token".to_string(),
            ),
        ]),
    );

    let transport = TransportConfig::from_workflow(
        &workflow,
        &BTreeMap::from([(
            "OPENHANDS_SESSION_API_KEY".to_string(),
            "secret-token".to_string(),
        )]),
    )
    .expect("transport should resolve");

    match &transport.auth().http {
        HttpAuth::Header(key) => {
            assert_eq!(key.name(), "x-session-api-key");
            assert_eq!(key.value(), "secret-token");
        }
        other => panic!("expected header HTTP auth, got {other:?}"),
    }

    match &transport.auth().websocket {
        WebSocketAuth::QueryParam(key) => {
            assert_eq!(key.name(), "session_api_key");
            assert_eq!(key.value(), "secret-token");
        }
        other => panic!("expected websocket query-param auth, got {other:?}"),
    }

    let diagnostics = transport.diagnostics().expect("diagnostics should resolve");
    assert_eq!(diagnostics.target_kind, TransportTargetKind::Remote);
    assert_eq!(diagnostics.http_auth_kind, TransportAuthKind::Header);
    assert_eq!(
        diagnostics.websocket_auth_kind,
        TransportAuthKind::QueryParam
    );
    assert_eq!(
        diagnostics.websocket_query_param_name.as_deref(),
        Some("session_api_key")
    );
    assert!(!diagnostics.managed_local_server_candidate);
}

#[test]
fn loopback_transport_without_auth_stays_a_managed_local_server_candidate() {
    let workflow = resolve_workflow(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  transport:
    base_url: http://127.0.0.1:8000
---
{{ issue.identifier }}
"#,
        BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
    );

    let transport = TransportConfig::from_workflow(&workflow, &BTreeMap::<String, String>::new())
        .expect("transport should resolve");
    let diagnostics = transport.diagnostics().expect("diagnostics should resolve");

    assert_eq!(diagnostics.target_kind, TransportTargetKind::Loopback);
    assert_eq!(diagnostics.http_auth_kind, TransportAuthKind::None);
    assert_eq!(diagnostics.websocket_auth_kind, TransportAuthKind::None);
    assert!(diagnostics.managed_local_server_candidate);
}

#[test]
fn path_prefixed_unauthenticated_loopback_transports_normalize_to_managed_local_supervision() {
    let workflow = resolve_workflow(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  transport:
    base_url: http://127.0.0.1:8000/runtime
---
{{ issue.identifier }}
"#,
        BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
    );

    let transport = TransportConfig::from_workflow(&workflow, &BTreeMap::<String, String>::new())
        .expect("transport should resolve");
    let diagnostics = transport.diagnostics().expect("diagnostics should resolve");

    assert_eq!(diagnostics.target_kind, TransportTargetKind::Loopback);
    assert_eq!(diagnostics.http_auth_kind, TransportAuthKind::None);
    assert_eq!(diagnostics.websocket_auth_kind, TransportAuthKind::None);
    assert!(diagnostics.managed_local_server_candidate);
    assert_eq!(
        transport
            .managed_local_server_base_url()
            .expect("managed local server base URL should resolve")
            .as_deref(),
        Some("http://127.0.0.1:8000")
    );
}

#[test]
fn authenticated_loopback_transports_stay_outside_managed_local_supervision() {
    let workflow = resolve_workflow(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  transport:
    base_url: http://127.0.0.1:8000/runtime
    session_api_key_env: OPENHANDS_SESSION_API_KEY
  websocket:
    auth_mode: header
---
{{ issue.identifier }}
"#,
        BTreeMap::from([
            ("LINEAR_API_KEY".to_string(), "linear-token".to_string()),
            (
                "OPENHANDS_SESSION_API_KEY".to_string(),
                "secret-token".to_string(),
            ),
        ]),
    );

    let transport = TransportConfig::from_workflow(
        &workflow,
        &BTreeMap::from([(
            "OPENHANDS_SESSION_API_KEY".to_string(),
            "secret-token".to_string(),
        )]),
    )
    .expect("transport should resolve");
    let diagnostics = transport.diagnostics().expect("diagnostics should resolve");

    assert_eq!(diagnostics.target_kind, TransportTargetKind::Loopback);
    assert_eq!(diagnostics.http_auth_kind, TransportAuthKind::Header);
    assert_eq!(diagnostics.websocket_auth_kind, TransportAuthKind::Header);
    assert!(!diagnostics.managed_local_server_candidate);
    assert_eq!(
        transport
            .managed_local_server_base_url()
            .expect("managed local server base URL should resolve"),
        None
    );
}

#[test]
fn workflow_transport_config_requires_a_non_blank_session_key_when_auth_is_enabled() {
    let workflow = resolve_workflow(
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  transport:
    base_url: https://agent.example.com/runtime
    session_api_key_env: OPENHANDS_SESSION_API_KEY
---
{{ issue.identifier }}
"#,
        BTreeMap::from([("LINEAR_API_KEY".to_string(), "linear-token".to_string())]),
    );

    let error = TransportConfig::from_workflow(
        &workflow,
        &BTreeMap::from([("OPENHANDS_SESSION_API_KEY".to_string(), "   ".to_string())]),
    )
    .expect_err("blank session key env should fail");

    assert!(
        error.to_string().contains("OPENHANDS_SESSION_API_KEY"),
        "unexpected error: {error}"
    );
}
