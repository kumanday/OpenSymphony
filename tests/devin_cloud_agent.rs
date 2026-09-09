use std::path::PathBuf;

use opensymphony::opensymphony_devin::{
    DEVIN_CLOUD_AGENT_KIND, DEVIN_REMOTE_CONTAINMENT, DEVIN_UNDISCRIMINATED_EVENT_KIND,
    DevinCloudAdapter, DevinCloudClient, DevinCloudConfig, DevinHttpMethod, DevinLifecycleRequest,
    DevinRemoteWorkspaceBinding, DevinRequestBuilder, NormalizedDevinEventKind,
    devin_event_summary, normalize_devin_event, normalize_event_page,
    normalized_event_to_journal_record,
};
use opensymphony::opensymphony_domain::HarnessAdapter;
use opensymphony::opensymphony_gateway_schema::event_journal::EventKind;
use serde_json::json;

fn binding() -> DevinRemoteWorkspaceBinding {
    DevinRemoteWorkspaceBinding::new(
        "acme-123",
        PathBuf::from("/workspaces/acme-123"),
        "https://github.com/acme/api",
        Some("main".to_owned()),
    )
    .expect("binding")
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
    assert!(
        DevinRemoteWorkspaceBinding::new(
            "acme-123",
            PathBuf::from("/workspaces/acme-123"),
            "https://token@github.com/acme/api",
            None,
        )
        .is_err()
    );
    assert!(
        DevinRemoteWorkspaceBinding::new(
            "acme-123",
            PathBuf::from("/workspaces/acme-123"),
            "git@github.com:acme/api.git",
            None,
        )
        .is_err()
    );
    assert!(
        DevinRemoteWorkspaceBinding::new(
            "acme-123",
            PathBuf::from("/workspaces/acme-123"),
            "https://github.com/acme/api?token=secret",
            None,
        )
        .is_err()
    );
    assert!(
        DevinRemoteWorkspaceBinding::new(
            "acme-123",
            PathBuf::from("/workspaces/acme-123"),
            "https://github.com/acme/api#token=secret",
            None,
        )
        .is_err()
    );
}

#[test]
fn config_rejects_insecure_or_credentialed_endpoints() {
    assert!(DevinCloudConfig::default().validate().is_ok());

    let insecure = DevinCloudConfig {
        base_url: "http://api.devin.ai/v1".into(),
        ..DevinCloudConfig::default()
    };
    assert!(insecure.validate().is_err());

    let credentialed = DevinCloudConfig {
        base_url: "https://user:secret@api.devin.ai/v1".into(),
        ..DevinCloudConfig::default()
    };
    assert!(credentialed.validate().is_err());

    let bad_env = DevinCloudConfig {
        api_key_env: "devin key".into(),
        ..DevinCloudConfig::default()
    };
    assert!(bad_env.validate().is_err());

    let decorated = DevinCloudConfig {
        base_url: "https://api.devin.ai/v1?token=secret".into(),
        ..DevinCloudConfig::default()
    };
    assert!(decorated.validate().is_err());
}

#[test]
fn lifecycle_requests_cover_session_message_run_and_events() {
    let builder = DevinRequestBuilder::new("https://api.devin.ai/v1");
    let binding = binding();

    let create = builder.create_session("fix the flake", &binding, Some("run-1".into()));
    assert_eq!(create.lifecycle, DevinLifecycleRequest::SessionCreate);
    assert_eq!(create.method, DevinHttpMethod::Post);
    assert_eq!(create.path, "/sessions");
    let body = create.body.expect("create body");
    assert_eq!(body["prompt"], json!("fix the flake"));
    assert_eq!(body["repository"]["url"], json!(binding.repository_url));
    assert_eq!(body["repository"]["base_branch"], json!("main"));
    assert_eq!(body["idempotency_key"], json!("run-1"));

    let resume = builder.resume_session("sess-1");
    assert_eq!(resume.lifecycle, DevinLifecycleRequest::SessionResume);
    assert_eq!(resume.path, "/sessions/sess-1/resume");

    let message = builder.send_message("sess-1", "please rerun the tests");
    assert_eq!(message.lifecycle, DevinLifecycleRequest::MessageSend);
    assert_eq!(message.path, "/sessions/sess-1/messages");
    assert_eq!(
        message.body.expect("message body")["message"],
        json!("please rerun the tests")
    );

    let start = builder.start_run("sess-1", "continue");
    assert_eq!(start.lifecycle, DevinLifecycleRequest::RunStart);
    assert_eq!(start.path, "/sessions/sess-1/runs");

    let cancel = builder.cancel_run("sess-1", "run-9");
    assert_eq!(cancel.lifecycle, DevinLifecycleRequest::RunCancel);
    assert_eq!(cancel.path, "/sessions/sess-1/runs/run-9/cancel");

    let events = builder.fetch_events("sess-1", Some(42));
    assert_eq!(events.lifecycle, DevinLifecycleRequest::EventFetch);
    assert_eq!(events.method, DevinHttpMethod::Get);
    assert_eq!(events.path, "/sessions/sess-1/events?after=42");
    assert_eq!(
        builder
            .absolute_url(&events)
            .expect("absolute url")
            .as_str(),
        "https://api.devin.ai/v1/sessions/sess-1/events?after=42"
    );
}

#[test]
fn client_requires_the_configured_credential_environment_variable() {
    let config = DevinCloudConfig::default();

    assert!(DevinCloudClient::from_environment(&config, |_| None).is_err());
    assert!(DevinCloudClient::from_environment(&config, |_| Some("   ".into())).is_err());
    assert!(
        DevinCloudClient::from_environment(&config, |name| {
            (name == config.api_key_env).then(|| "devin-token".to_owned())
        })
        .is_ok()
    );

    // A bearer credential is attached to every request, so an invalid endpoint
    // must be rejected before the token is ever handed to the HTTP client.
    let invalid = DevinCloudConfig {
        base_url: "http://api.devin.ai/v1".into(),
        ..DevinCloudConfig::default()
    };
    assert!(DevinCloudClient::from_environment(&invalid, |_| Some("devin-token".into())).is_err());
}

#[test]
fn known_events_normalize_into_journal_records() {
    let event = normalize_devin_event(json!({
        "type": "run.completed",

        "session_id": "sess-1",
        "run_id": "run-9",
        "cursor": 7,
        "status": "finished",
    }));

    assert_eq!(event.kind, NormalizedDevinEventKind::RunCompleted);
    assert_eq!(event.session_id.as_deref(), Some("sess-1"));
    assert_eq!(event.run_id.as_deref(), Some("run-9"));
    assert_eq!(event.cursor, Some(7));
    assert_eq!(devin_event_summary(&event), "Devin run completed");

    let record = normalized_event_to_journal_record("run-9", 7, &event);
    assert_eq!(record.kind, EventKind::RunCompleted);
    assert_eq!(record.actor.kind_label(), "harness");
    assert_eq!(
        record.payload.expect("payload")["session_id"],
        json!("sess-1")
    );
    assert_eq!(record.raw_payload_ref.as_deref(), Some("devin:run-9:7"));
}

#[test]
fn unknown_events_retain_their_raw_payload() {
    let raw = json!({
        "type": "devin.future_event",
        "session_id": "sess-1",
        "payload": { "nested": [1, 2, 3] },
    });
    let event = normalize_devin_event(raw.clone());

    assert_eq!(event.kind, NormalizedDevinEventKind::Unknown);
    assert_eq!(event.raw, raw);

    let record = normalized_event_to_journal_record("run-9", 3, &event);
    assert_eq!(
        record.kind,
        EventKind::Unknown {
            raw_kind: "devin.future_event".to_owned(),
        }
    );
    assert_eq!(record.payload.expect("payload")["raw_payload"], raw);
}

#[test]
fn events_without_a_discriminator_are_preserved_as_unknown() {
    let raw = json!({ "session_id": "sess-1", "details": { "nested": true } });
    let event = normalize_devin_event(raw.clone());

    assert_eq!(event.kind, NormalizedDevinEventKind::Unknown);
    assert_eq!(event.event_type, DEVIN_UNDISCRIMINATED_EVENT_KIND);
    assert_eq!(event.raw, raw);
}

#[test]
fn event_pages_accept_arrays_and_envelopes() {
    let page = json!({
        "events": [
            { "type": "devin_message", "message": "starting work" },
            { "type": "tool_call" },
            { "missing_type": true },
        ]
    });
    let normalized = normalize_event_page(&page);

    assert_eq!(normalized.len(), 3);
    assert_eq!(normalized[0].kind, NormalizedDevinEventKind::AgentMessage);
    assert_eq!(devin_event_summary(&normalized[0]), "starting work");
    assert_eq!(normalized[1].kind, NormalizedDevinEventKind::ToolCall);
    assert_eq!(normalized[2].kind, NormalizedDevinEventKind::Unknown);
    assert_eq!(normalized[2].raw, json!({ "missing_type": true }));

    let bare = json!([{ "type": "run.started" }]);
    assert_eq!(normalize_event_page(&bare).len(), 1);
}
