//! Devin cloud harness boundary tests.
//!
//! Request shapes are checked against the vendored Devin API v3 OpenAPI subset
//! in `crates/opensymphony-devin/contracts/devin-v3-openapi.yaml`, and response
//! decoding is exercised with payloads shaped like the documented schemas.
//! Nothing here talks to the live API.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use opensymphony::opensymphony_devin::{
    DEVIN_CLOUD_AGENT_KIND, DEVIN_CLOUD_API_CONTRACT, DEVIN_REMOTE_CONTAINMENT,
    DEVIN_UNDISCRIMINATED_EVENT_KIND, DevinCloudAdapter, DevinCloudClient, DevinCloudConfig,
    DevinHttpMethod, DevinMessageCursor, DevinMode, DevinOperation, DevinProblemDetail,
    DevinRemoteWorkspaceBinding, DevinRequest, DevinRequestBuilder, DevinSessionOptions,
    DevinSessionStatus, DevinStatusDetail, NormalizedDevinEventKind, PaginatedResponse,
    SessionMessage, SessionResponse, SessionsQueryParams, devin_event_summary,
    normalize_devin_event, normalized_event_to_journal_record, session_create_request,
};
use opensymphony::opensymphony_domain::HarnessAdapter;
use opensymphony::opensymphony_gateway_schema::event_journal::EventKind;
use serde_json::{Value, json};

const ORG_ID: &str = "org-abc123";

fn binding() -> DevinRemoteWorkspaceBinding {
    DevinRemoteWorkspaceBinding::new(
        "acme-123",
        PathBuf::from("/workspaces/acme-123"),
        "https://github.com/acme/api",
        Some("main".to_owned()),
    )
    .expect("binding")
}

fn builder() -> DevinRequestBuilder {
    DevinRequestBuilder::new("https://api.devin.ai", ORG_ID)
}

fn contract() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("crates/opensymphony-devin/contracts/devin-v3-openapi.yaml");
    let text = std::fs::read_to_string(path).expect("vendored devin v3 contract");
    serde_yaml::from_str(&text).expect("contract parses as yaml")
}

/// Path with the concrete organization and session identifiers replaced by the
/// OpenAPI template variables, and query suffix removed.
fn as_path_template(path: &str) -> String {
    path.split('?')
        .next()
        .unwrap_or(path)
        .replace(ORG_ID, "{org_id}")
        .replace("devin-1", "{devin_id}")
}

fn session_json() -> Value {
    json!({
        "session_id": "devin-1",
        "url": "https://app.devin.ai/sessions/1",
        "status": "running",
        "status_detail": "working",
        "tags": ["opensymphony:acme-123"],
        "org_id": ORG_ID,
        "created_at": 1_757_000_000,
        "updated_at": 1_757_000_600,
        "acus_consumed": 1.5,
        "pull_requests": [{ "pr_url": "https://github.com/acme/api/pull/7", "pr_state": "open" }],
        "structured_output": { "summary": "done" },
        "title": "Fix the flake",
        "is_archived": false,
        "devin_mode": "fast",
        "a_field_devin_added_later": { "nested": true },
    })
}

fn session(status: &str, status_detail: Option<&str>) -> SessionResponse {
    let mut value = session_json();
    value["status"] = json!(status);
    value["status_detail"] = match status_detail {
        Some(detail) => json!(detail),
        None => Value::Null,
    };
    serde_json::from_value(value).expect("session decodes")
}

fn message_page(messages: &[(&str, &str, &str)], end_cursor: Option<&str>, more: bool) -> Value {
    json!({
        "items": messages
            .iter()
            .map(|(event_id, source, message)| json!({
                "event_id": event_id,
                "source": source,
                "message": message,
                "created_at": 1_757_000_100_i64,
            }))
            .collect::<Vec<_>>(),
        "end_cursor": end_cursor,
        "has_next_page": more,
        "total": messages.len(),
    })
}

#[test]
fn adapter_exposes_stable_unavailable_capability() {
    let adapter = DevinCloudAdapter::default();
    let capability = adapter.capabilities();

    assert_eq!(adapter.harness_kind(), DEVIN_CLOUD_AGENT_KIND);
    assert_eq!(capability.kind, DEVIN_CLOUD_AGENT_KIND);
    assert!(!capability.available);
    assert_eq!(capability.transport.protocol, "https");
    assert!(capability.transport.remote);
    assert!(!capability.transport.local);
    assert_eq!(capability.feature_gaps.len(), 4);
    assert!(
        adapter
            .unavailability_reason()
            .contains("advertised as unavailable")
    );
}

#[test]
fn remote_binding_keeps_local_workspace_as_evidence_only() {
    let binding = binding();

    assert_eq!(binding.effective_containment(), DEVIN_REMOTE_CONTAINMENT);
    assert_eq!(
        binding.local_evidence_path,
        PathBuf::from("/workspaces/acme-123")
    );
    assert_eq!(binding.repo_name().as_deref(), Some("acme/api"));
    assert_eq!(binding.correlation_tag(), "opensymphony:acme-123");
    // The API has no branch field, so branch intent has to reach Devin through
    // the prompt.
    let prompt = binding.compose_prompt("fix the flake");
    assert!(prompt.contains("Base branch: main"));
    assert!(prompt.contains("evidence only"));
    assert!(prompt.trim_end().ends_with("fix the flake"));

    for rejected in [
        "https://token@github.com/acme/api",
        "git@github.com:acme/api.git",
        "https://github.com/acme/api?token=secret",
        "https://github.com/acme/api#token=secret",
    ] {
        assert!(
            DevinRemoteWorkspaceBinding::new(
                "acme-123",
                PathBuf::from("/workspaces/acme-123"),
                rejected,
                None,
            )
            .is_err(),
            "{rejected} must be rejected"
        );
    }
}

#[test]
fn config_rejects_insecure_credentialed_or_malformed_settings() {
    assert!(DevinCloudConfig::default().validate().is_ok());

    for invalid in [
        DevinCloudConfig {
            base_url: "http://api.devin.ai".into(),
            ..DevinCloudConfig::default()
        },
        DevinCloudConfig {
            base_url: "https://user:secret@api.devin.ai".into(),
            ..DevinCloudConfig::default()
        },
        DevinCloudConfig {
            base_url: "https://api.devin.ai?token=secret".into(),
            ..DevinCloudConfig::default()
        },
        DevinCloudConfig {
            api_key_env: "devin key".into(),
            ..DevinCloudConfig::default()
        },
        DevinCloudConfig {
            org_id_env: "devin org".into(),
            ..DevinCloudConfig::default()
        },
        DevinCloudConfig {
            org_id: Some("acme".into()),
            ..DevinCloudConfig::default()
        },
        DevinCloudConfig {
            session: DevinSessionOptions {
                max_acu_limit: Some(0),
                ..DevinSessionOptions::default()
            },
            ..DevinCloudConfig::default()
        },
    ] {
        assert!(invalid.validate().is_err(), "{invalid:?} must be rejected");
    }
}

#[test]
fn request_paths_match_the_vendored_v3_contract() {
    let contract = contract();
    let paths = contract["paths"]
        .as_object()
        .expect("contract paths")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let builder = builder();

    let requests: Vec<(DevinRequest, DevinHttpMethod, DevinOperation)> = vec![
        (
            builder.get_self(),
            DevinHttpMethod::Get,
            DevinOperation::GetSelf,
        ),
        (
            builder.create_session(&session_create_request(
                "fix the flake",
                &binding(),
                &DevinSessionOptions::default(),
            )),
            DevinHttpMethod::Post,
            DevinOperation::CreateSession,
        ),
        (
            builder.get_session("devin-1"),
            DevinHttpMethod::Get,
            DevinOperation::GetSession,
        ),
        (
            builder.list_sessions(&SessionsQueryParams::by_tag("opensymphony:acme-123")),
            DevinHttpMethod::Get,
            DevinOperation::ListSessions,
        ),
        (
            builder.list_messages("devin-1", Some("cursor-1"), Some(50)),
            DevinHttpMethod::Get,
            DevinOperation::ListSessionMessages,
        ),
        (
            builder.send_message("devin-1", "please rerun the tests"),
            DevinHttpMethod::Post,
            DevinOperation::SendSessionMessage,
        ),
        (
            builder.delete_session("devin-1", true),
            DevinHttpMethod::Delete,
            DevinOperation::DeleteSession,
        ),
        (
            builder.list_attachments("devin-1"),
            DevinHttpMethod::Get,
            DevinOperation::ListSessionAttachments,
        ),
        (
            builder.replace_tags("devin-1", vec!["opensymphony:acme-123".into()]),
            DevinHttpMethod::Put,
            DevinOperation::UpdateSessionTags,
        ),
    ];

    for (request, method, operation) in requests {
        let template = as_path_template(&request.path);
        assert!(
            paths.contains(&template),
            "`{template}` is not a documented v3 path"
        );
        let documented = &contract["paths"][&template][method.as_str().to_lowercase()];
        assert!(
            documented.is_object(),
            "`{} {template}` is not documented",
            method.as_str()
        );
        assert_eq!(request.method, method);
        assert_eq!(request.operation, operation);
    }

    assert_eq!(DEVIN_CLOUD_API_CONTRACT, "devin-api-v3");
}

#[test]
fn session_requests_carry_documented_query_and_body_shapes() {
    let builder = builder();

    let messages = builder.list_messages("devin-1", Some("cursor 1"), Some(500));
    assert_eq!(
        messages.path,
        format!("/v3/organizations/{ORG_ID}/sessions/devin-1/messages?after=cursor%201&first=200")
    );
    assert_eq!(
        builder
            .absolute_url(&messages)
            .expect("absolute url")
            .as_str(),
        format!(
            "https://api.devin.ai/v3/organizations/{ORG_ID}/sessions/devin-1/messages?after=cursor%201&first=200"
        )
    );

    let listed = builder.list_sessions(&SessionsQueryParams::by_tag("opensymphony:acme-123"));
    let query = listed
        .path
        .split_once("?qs=")
        .map(|(_, encoded)| encoded.to_owned())
        .expect("qs parameter");
    let decoded = percent_decode(&query);
    assert_eq!(
        serde_json::from_str::<Value>(&decoded).expect("qs is json"),
        json!({ "tags": ["opensymphony:acme-123"] })
    );

    let terminate = builder.delete_session("devin-1", false);
    assert!(terminate.path.ends_with("?archive=false"));

    assert_eq!(
        builder.send_message("devin-1", "rerun").body,
        Some(json!({ "message": "rerun" }))
    );
    assert_eq!(
        builder.replace_tags("devin-1", vec!["a".into()]).body,
        Some(json!({ "tags": ["a"] }))
    );
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).expect("hex digits");
            decoded.push(u8::from_str_radix(hex, 16).expect("hex byte"));
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).expect("utf-8")
}

#[test]
fn create_session_request_maps_binding_and_options_onto_v3_fields() {
    let options = DevinSessionOptions {
        playbook_id: Some("playbook-1".into()),
        knowledge_ids: Some(vec!["note-1".into()]),
        secret_ids: Some(vec!["secret-1".into()]),
        max_acu_limit: Some(20),
        tags: vec!["team:core".into()],
        title: Some("Fix the flake".into()),
        devin_mode: Some(DevinMode::Fast),
        platform: Some("windows".into()),
        resumable: true,
    };
    let request = session_create_request("fix the flake", &binding(), &options);

    assert_eq!(request.repos, Some(vec!["acme/api".to_owned()]));
    assert_eq!(
        request.tags,
        Some(vec![
            "team:core".to_owned(),
            "opensymphony:acme-123".to_owned()
        ])
    );
    assert_eq!(request.devin_mode, Some(DevinMode::Fast));
    assert_eq!(request.resumable, Some(true));
    assert!(
        request
            .prompt
            .contains("Repository: https://github.com/acme/api")
    );

    // Secret *references* are forwarded; values are never part of the request.
    assert_eq!(request.secret_ids, Some(vec!["secret-1".to_owned()]));

    let body = serde_json::to_value(&request).expect("serializes");
    assert!(body.get("structured_output_schema").is_none());
    assert_eq!(body["max_acu_limit"], json!(20));

    let crowded = DevinSessionOptions {
        tags: (0..60).map(|index| format!("tag-{index}")).collect(),
        ..DevinSessionOptions::default()
    };
    let request = session_create_request("fix the flake", &binding(), &crowded);
    assert_eq!(request.tags.expect("tags").len(), 50);
}

#[test]
fn documented_payloads_decode_and_retain_unknown_fields() {
    let session: SessionResponse = serde_json::from_value(session_json()).expect("session decodes");

    assert_eq!(session.session_id, "devin-1");
    assert_eq!(session.status, DevinSessionStatus::Running);
    assert_eq!(
        session.status_detail,
        Some(DevinStatusDetail("working".into()))
    );
    assert_eq!(session.acus_consumed, 1.5);
    assert_eq!(
        session.pull_requests[0].pr_url,
        "https://github.com/acme/api/pull/7"
    );
    assert_eq!(
        session.extra.get("a_field_devin_added_later"),
        Some(&json!({ "nested": true }))
    );
    assert!(!session.is_settled());

    // A status Devin adds later must not fail decoding.
    let future: SessionResponse = serde_json::from_value(json!({
        "session_id": "devin-1",
        "url": "https://app.devin.ai/sessions/1",
        "status": "hibernating",
        "org_id": ORG_ID,
        "created_at": 1,
        "updated_at": 2,
    }))
    .expect("unknown status decodes");
    assert_eq!(
        future.status,
        DevinSessionStatus::Other("hibernating".into())
    );

    let page: PaginatedResponse<SessionMessage> = serde_json::from_value(message_page(
        &[("event-1", "devin", "starting work")],
        Some("cursor-1"),
        true,
    ))
    .expect("page decodes");
    assert_eq!(page.items[0].event_id, "event-1");
    assert!(page.has_next_page);
    assert_eq!(page.end_cursor.as_deref(), Some("cursor-1"));

    let problem: DevinProblemDetail = serde_json::from_value(json!({
        "title": "Not Found",
        "status": 404,
        "detail": "session devin-1 does not exist",
    }))
    .expect("problem decodes");
    assert_eq!(
        problem.to_string(),
        "Not Found: session devin-1 does not exist"
    );
}

#[test]
fn message_cursor_deduplicates_replayed_pages_and_tracks_status() {
    let mut cursor = DevinMessageCursor::new();
    let first: PaginatedResponse<SessionMessage> = serde_json::from_value(message_page(
        &[
            ("event-1", "devin", "starting work"),
            ("event-2", "user", "please rerun"),
        ],
        Some("cursor-1"),
        true,
    ))
    .expect("page decodes");

    let events = cursor.ingest_messages("devin-1", &first);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].kind, NormalizedDevinEventKind::DevinMessage);
    assert_eq!(events[1].kind, NormalizedDevinEventKind::UserMessage);
    assert_eq!(cursor.cursor(), Some("cursor-1"));

    // Devin replays the last page plus one new message.
    let replayed: PaginatedResponse<SessionMessage> = serde_json::from_value(message_page(
        &[
            ("event-2", "user", "please rerun"),
            ("event-3", "devin", "rerunning"),
        ],
        Some("cursor-2"),
        false,
    ))
    .expect("page decodes");
    let events = cursor.ingest_messages("devin-1", &replayed);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id.as_deref(), Some("event-3"));
    assert_eq!(cursor.cursor(), Some("cursor-2"));

    // A source Devin adds later is preserved rather than dropped.
    let unknown_source: PaginatedResponse<SessionMessage> = serde_json::from_value(message_page(
        &[("event-4", "system", "vm resized")],
        Some("cursor-3"),
        false,
    ))
    .expect("page decodes");
    let events = cursor.ingest_messages("devin-1", &unknown_source);
    assert_eq!(events[0].kind, NormalizedDevinEventKind::Unknown);
    assert_eq!(events[0].raw["source"], json!("system"));

    assert_eq!(
        cursor
            .ingest_status(&session("running", Some("working")))
            .map(|event| event.kind),
        Some(NormalizedDevinEventKind::StatusChanged)
    );
    assert!(
        cursor
            .ingest_status(&session("running", Some("working")))
            .is_none(),
        "an unchanged status must not re-emit"
    );
    assert_eq!(
        cursor
            .ingest_status(&session("running", Some("waiting_for_user")))
            .map(|event| event.kind),
        Some(NormalizedDevinEventKind::SessionBlocked)
    );
    assert_eq!(
        cursor
            .ingest_status(&session("running", Some("finished")))
            .map(|event| event.kind),
        Some(NormalizedDevinEventKind::SessionFinished)
    );
    assert_eq!(
        cursor
            .ingest_status(&session("error", Some("blocked_error")))
            .map(|event| event.kind),
        Some(NormalizedDevinEventKind::SessionFailed)
    );
    assert_eq!(
        cursor
            .ingest_status(&session("suspended", None))
            .map(|event| event.kind),
        Some(NormalizedDevinEventKind::SessionSuspended)
    );
}

#[test]
fn client_requires_credentials_and_a_resolvable_organization() {
    let config = DevinCloudConfig {
        org_id: Some(ORG_ID.into()),
        ..DevinCloudConfig::default()
    };

    assert!(DevinCloudClient::from_environment(&config, |_| None).is_err());
    assert!(DevinCloudClient::from_environment(&config, |_| Some("   ".into())).is_err());

    let client = DevinCloudClient::from_environment(&config, |name| {
        (name == config.api_key_env).then(|| "cog_token".to_owned())
    })
    .expect("client");
    assert_eq!(client.org_id(), Some(ORG_ID));
    assert!(format!("{client:?}").contains("redacted"));
    assert!(!format!("{client:?}").contains("cog_token"));

    // The organization can also be supplied beside the credential.
    let discovered =
        DevinCloudClient::from_environment(&DevinCloudConfig::default(), |name| match name {
            "COG_SERVICE_USER_TOKEN" => Some("cog_token".to_owned()),
            "DEVIN_ORG_ID" => Some(ORG_ID.to_owned()),
            _ => None,
        })
        .expect("client");
    assert_eq!(discovered.org_id(), Some(ORG_ID));

    // Without an organization, requests cannot be built until `/v3/self`
    // resolves one.
    let unresolved = DevinCloudClient::from_environment(&DevinCloudConfig::default(), |name| {
        (name == "COG_SERVICE_USER_TOKEN").then(|| "cog_token".to_owned())
    })
    .expect("client");
    assert!(unresolved.org_id().is_none());
    assert!(unresolved.requests().is_err());

    // A bearer credential is attached to every request, so an invalid endpoint
    // must be rejected before the token is read.
    let invalid = DevinCloudConfig {
        base_url: "http://api.devin.ai".into(),
        ..DevinCloudConfig::default()
    };
    assert!(DevinCloudClient::from_environment(&invalid, |_| Some("cog_token".into())).is_err());

    // A malformed organization from the environment is rejected too.
    let bad_org =
        DevinCloudClient::from_environment(&DevinCloudConfig::default(), |name| match name {
            "COG_SERVICE_USER_TOKEN" => Some("cog_token".to_owned()),
            "DEVIN_ORG_ID" => Some("acme".to_owned()),
            _ => None,
        });
    assert!(bad_org.is_err());
}

#[test]
fn messages_normalize_into_journal_records() {
    let session = session("exit", Some("finished"));
    let mut cursor = DevinMessageCursor::new();
    let event = cursor.ingest_status(&session).expect("status event");

    assert_eq!(event.kind, NormalizedDevinEventKind::SessionFinished);
    assert_eq!(devin_event_summary(&event), "Devin session finished");

    let record = normalized_event_to_journal_record("run-9", 7, &event);
    assert_eq!(record.kind, EventKind::RunCompleted);
    assert_eq!(record.actor.kind_label(), "harness");
    assert_eq!(
        record.payload.expect("payload")["session_id"],
        json!("devin-1")
    );
    assert_eq!(record.raw_payload_ref.as_deref(), Some("devin:run-9:7"));
}

#[test]
fn undecodable_payloads_are_preserved_as_unknown() {
    let raw = json!({
        "source": "devin",
        "session_id": "devin-1",
        "payload": { "nested": [1, 2, 3] },
    });
    let event = normalize_devin_event(raw.clone());
    assert_eq!(event.kind, NormalizedDevinEventKind::DevinMessage);
    assert_eq!(event.raw, raw);

    let undiscriminated = json!({ "session_id": "devin-1", "details": { "nested": true } });
    let event = normalize_devin_event(undiscriminated.clone());
    assert_eq!(event.kind, NormalizedDevinEventKind::Unknown);
    assert_eq!(event.event_type, DEVIN_UNDISCRIMINATED_EVENT_KIND);
    assert_eq!(event.raw, undiscriminated);

    let record = normalized_event_to_journal_record("run-9", 3, &event);
    assert_eq!(
        record.kind,
        EventKind::Unknown {
            raw_kind: DEVIN_UNDISCRIMINATED_EVENT_KIND.to_owned(),
        }
    );
    assert_eq!(
        record.payload.expect("payload")["raw_payload"],
        undiscriminated
    );
}
