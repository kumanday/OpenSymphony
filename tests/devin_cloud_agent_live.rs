//! Live Devin API v3 verification.
//!
//! These tests talk to `https://api.devin.ai` with a real service-user token and
//! are opt-in: they are `#[ignore]`d and additionally skip themselves unless
//! `OPENSYMPHONY_DEVIN_LIVE=1` is set along with the credential environment
//! variables. `devin_live_session_lifecycle` creates a real session and
//! therefore consumes ACUs; it terminates and archives the session it creates.
//!
//! ```bash
//! OPENSYMPHONY_DEVIN_LIVE=1 cargo test --test devin_cloud_agent_live -- --ignored --nocapture
//! ```

use std::path::PathBuf;
use std::time::Duration;

use opensymphony::opensymphony_devin::{
    DevinClientError, DevinCloudClient, DevinCloudConfig, DevinEvidenceCollector,
    DevinEvidenceLimits, DevinRemoteWorkspaceBinding, DevinRunOptions, DevinRunOutcome,
    DevinSessionOptions, DevinSessionRunner, SessionsQueryParams, devin_event_summary,
    session_create_request,
};

const LIVE_ENV: &str = "OPENSYMPHONY_DEVIN_LIVE";

fn live_config() -> Option<DevinCloudConfig> {
    if std::env::var(LIVE_ENV).ok().as_deref() != Some("1") {
        eprintln!("skipping: set {LIVE_ENV}=1 to run live Devin API tests");
        return None;
    }
    Some(DevinCloudConfig::default())
}

async fn live_client(config: &DevinCloudConfig) -> DevinCloudClient {
    DevinCloudClient::from_environment(config, |name| std::env::var(name).ok())
        .expect("credentials from the environment")
        .bind_tenant()
        .await
        .expect("organization resolves")
}

/// Verifies the tenancy binding and organization-scoped secret resolution
/// against the live API. No session is created, so this consumes no ACUs.
#[tokio::test]
#[ignore = "live Devin API call"]
async fn devin_live_tenant_binding_and_secret_scope() {
    let Some(config) = live_config() else { return };
    let client = live_client(&config).await;

    let tenancy = client.tenancy().expect("bind_tenant records the tenancy");
    assert_eq!(Some(tenancy.org_id.as_str()), client.org_id());
    assert!(tenancy.org_id.starts_with("org-"));
    assert_eq!(tenancy.principal_type.as_deref(), Some("service_user"));

    // A credential bound to one organization must not be usable with another.
    let mismatched = DevinCloudConfig {
        org_id: Some("org-00000000000000000000000000000000".to_owned()),
        ..config.clone()
    };
    let error = DevinCloudClient::from_environment(&mismatched, |name| std::env::var(name).ok())
        .expect("credentials from the environment")
        .bind_tenant()
        .await
        .expect_err("a foreign organization must be rejected");
    assert!(
        matches!(error, DevinClientError::TenantMismatch { .. }),
        "unexpected error: {error}"
    );

    let secrets = client.list_secrets().await.expect("GET .../secrets");
    println!("organization exposes {} secret(s)", secrets.len());
    let unknown = client
        .resolve_secret_ids(&["secret-does-not-exist".to_owned()])
        .await
        .expect_err("unknown secret references must be rejected");
    assert!(
        matches!(unknown, DevinClientError::UnknownSecret { .. }),
        "unexpected error: {unknown}"
    );
    if let Some(secret) = secrets.first() {
        let resolved = client
            .resolve_secret_ids(std::slice::from_ref(&secret.secret_id))
            .await
            .expect("an organization-owned secret id resolves");
        assert_eq!(resolved, vec![secret.secret_id.clone()]);
    }
}

#[tokio::test]
#[ignore = "live Devin API call"]
async fn devin_live_identity_and_session_listing() {
    let Some(config) = live_config() else { return };
    let client = live_client(&config).await;

    let identity = client.identity().await.expect("GET /v3/self");
    assert_eq!(identity.principal_type.as_deref(), Some("service_user"));
    let org_id = identity.org_id.as_deref().expect("org_id from /v3/self");
    assert!(org_id.starts_with("org-"), "unexpected org id shape");
    assert_eq!(client.org_id(), Some(org_id));

    // `qs` carries the session query as a JSON document.
    let sessions = client
        .list_sessions(&SessionsQueryParams::default())
        .await
        .expect("GET /v3/organizations/{org_id}/sessions");
    for session in &sessions.items {
        assert_eq!(session.org_id, org_id);
        assert!(!session.session_id.is_empty());
        assert!(session.url.starts_with("https://"));
    }
    println!(
        "listed {} session(s); has_next_page={}",
        sessions.items.len(),
        sessions.has_next_page
    );
}

/// Creates a real session, polls it through the normalization path, sends an
/// operator message, then terminates and archives it. This consumes ACUs.
#[tokio::test]
#[ignore = "live Devin API call that consumes ACUs"]
async fn devin_live_session_lifecycle() {
    let Some(mut config) = live_config() else {
        return;
    };
    config.poll_interval = Duration::from_secs(5);
    config.session = DevinSessionOptions {
        max_acu_limit: Some(2),
        title: Some("OpenSymphony harness live contract check".into()),
        tags: vec!["opensymphony-live-check".into()],
        resumable: false,
        ..DevinSessionOptions::default()
    };

    let client = live_client(&config).await;
    let runner = DevinSessionRunner::new(
        client,
        DevinRunOptions {
            stop_when_waiting_on_operator: true,
            max_duration: Some(Duration::from_secs(240)),
            collect_attachments: true,
            message_page_size: 100,
        },
    );

    let binding = DevinRemoteWorkspaceBinding::new(
        "opensymphony-live-check",
        PathBuf::from("/tmp/opensymphony-live-check"),
        "https://github.com/kumanday/OpenSymphony",
        Some("main".to_owned()),
    )
    .expect("binding");
    let request = session_create_request(
        "Reply with the single word ACK and stop. Do not read the repository, \
         do not run commands, do not make changes, and do not open a pull request.",
        &binding,
        &config.session,
    );

    let mut events = Vec::new();
    let report = runner
        .run(&request, |event| {
            println!("[{:?}] {}", event.kind, devin_event_summary(&event));
            events.push(event);
        })
        .await;

    let report = match report {
        Ok(report) => report,
        Err(error) => panic!("live session run failed: {error}"),
    };

    println!(
        "session {} outcome={:?} status={} acus={}",
        report.session_id,
        report.outcome,
        report.status.as_str(),
        report.acus_consumed
    );
    assert!(!events.is_empty(), "no normalized events were observed");
    assert!(matches!(
        report.outcome,
        DevinRunOutcome::Finished | DevinRunOutcome::WaitingOnOperator
    ));

    runner
        .send_message(&report.session_id, "Thanks, that is all.")
        .await
        .expect("POST .../messages");

    // Evidence import is the artifact-retrieval path the CLI route runs: the
    // manifest and journal must land inside the local issue workspace only.
    let workspace = tempfile::tempdir().expect("evidence workspace");
    let evidence_root = workspace.path().join(".opensymphony/devin/live-check");
    let mut collector =
        DevinEvidenceCollector::new(evidence_root.clone(), DevinEvidenceLimits::default());
    for event in &events {
        collector.record(event);
    }
    let manifest = collector
        .persist(runner.client(), &report)
        .await
        .expect("evidence import");
    assert_eq!(manifest.session_id, report.session_id);
    assert!(manifest.event_count > 0);
    assert!(evidence_root.join("evidence.json").exists());
    assert!(evidence_root.join("events.jsonl").exists());
    assert!(manifest.evidence_root.starts_with(workspace.path()));
    println!(
        "imported evidence: {} event(s), {} attachment(s), {} skipped",
        manifest.event_count,
        manifest.attachments.len(),
        manifest.skipped_attachments.len()
    );

    let terminated = runner
        .terminate(&report.session_id, true)
        .await
        .expect("DELETE .../sessions/{devin_id}?archive=true");
    assert_eq!(terminated.outcome, DevinRunOutcome::Terminated);
}

/// A session that outlives the poll budget must be stopped, not orphaned. This
/// mirrors the CLI route's timeout cleanup and consumes ACUs.
#[tokio::test]
#[ignore = "live Devin API call that consumes ACUs"]
async fn devin_live_poll_timeout_terminates_the_remote_session() {
    let Some(mut config) = live_config() else {
        return;
    };
    config.poll_interval = Duration::from_secs(3);
    config.session = DevinSessionOptions {
        max_acu_limit: Some(1),
        title: Some("OpenSymphony harness timeout cleanup check".into()),
        tags: vec!["opensymphony-live-check".into()],
        resumable: false,
        ..DevinSessionOptions::default()
    };

    let client = live_client(&config).await;
    let runner = DevinSessionRunner::new(
        client.clone(),
        DevinRunOptions {
            stop_when_waiting_on_operator: false,
            max_duration: Some(Duration::from_secs(10)),
            collect_attachments: false,
            message_page_size: 100,
        },
    );

    let binding = DevinRemoteWorkspaceBinding::new(
        "opensymphony-live-timeout",
        PathBuf::from("/tmp/opensymphony-live-timeout"),
        "https://github.com/kumanday/OpenSymphony",
        Some("main".to_owned()),
    )
    .expect("binding");
    let request = session_create_request(
        "Wait quietly and do nothing. Do not read the repository, do not run \
         commands, and do not open a pull request.",
        &binding,
        &config.session,
    );

    let created = client
        .create_session(&request)
        .await
        .expect("POST .../sessions");
    let error = runner
        .follow(&created.session_id, |_| {})
        .await
        .expect_err("the poll budget must expire");
    assert!(
        matches!(error, DevinClientError::PollTimeout { .. }),
        "unexpected error: {error}"
    );

    client
        .delete_session(&created.session_id, true)
        .await
        .expect("timed-out sessions are terminated and archived");
    let stopped = client
        .get_session(&created.session_id)
        .await
        .expect("GET .../sessions/{devin_id}");
    assert!(
        stopped.is_archived,
        "session {} stayed active after cleanup",
        stopped.session_id
    );
}
