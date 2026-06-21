use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use opensymphony::opensymphony_control::{ControlPlaneServer, SnapshotStore};
use opensymphony::opensymphony_domain::{
    ControlPlaneAgentServerStatus as AgentServerStatus,
    ControlPlaneConversationEvent as ConversationEvent,
    ControlPlaneDaemonSnapshot as DaemonSnapshot, ControlPlaneDaemonState as DaemonState,
    ControlPlaneDaemonStatus as DaemonStatus, ControlPlaneFileChange as FileChange,
    ControlPlaneFileChangeKind as FileChangeKind,
    ControlPlaneIssueRuntimeState as IssueRuntimeState, ControlPlaneIssueSnapshot as IssueSnapshot,
    ControlPlaneMetricsSnapshot as MetricsSnapshot, ControlPlaneRecentEvent as RecentEvent,
    ControlPlaneRecentEventKind as RecentEventKind, ControlPlaneWorkerOutcome as WorkerOutcome,
    SnapshotEnvelope, TrackerIssue, TrackerIssueBlocker, TrackerIssueRef, TrackerIssueState,
    TrackerIssueStateKind,
};
use opensymphony::opensymphony_gateway::{
    GatewayCapabilities, GatewayServer, LinearTaskGraphClient, control_plane_to_dashboard_snapshot,
    model_settings_for_llm_api_key, model_settings_for_llm_api_key_and_codex_readiness,
};
use opensymphony::opensymphony_gateway_schema::action::{
    ActionDispatch, ActionKind, ActionReceipt, ActionStatus, ActionTarget,
};
use opensymphony::opensymphony_gateway_schema::envelope::EntityKind;
use opensymphony::opensymphony_gateway_schema::model_settings::{
    CodexCliProbe, CodexLocalReadiness, CredentialStatusKind, CredentialStatusResponse,
    ModelSettingsResponse, ProbeCommandResult,
};
use opensymphony::opensymphony_gateway_schema::run::DiffLine;
use opensymphony::opensymphony_gateway_schema::validation::ValidationStatus;
use tokio::net::TcpListener;
use url::Url;

#[derive(Clone)]
struct FakeLinearTaskGraphClient {
    issues: Vec<TrackerIssue>,
}

#[async_trait]
impl LinearTaskGraphClient for FakeLinearTaskGraphClient {
    async fn issues_by_identifiers(
        &self,
        identifiers: &[String],
    ) -> Result<Vec<TrackerIssue>, String> {
        Ok(identifiers
            .iter()
            .filter_map(|identifier| {
                self.issues
                    .iter()
                    .find(|issue| issue.identifier == *identifier)
                    .cloned()
            })
            .collect())
    }
}

fn fake_linear_task_graph_client(
    snapshot: &DaemonSnapshot,
    blocker_overrides: &[(&str, Vec<&str>)],
) -> std::sync::Arc<dyn LinearTaskGraphClient> {
    fake_linear_task_graph_client_with_hierarchy(snapshot, blocker_overrides, &[])
}

fn fake_linear_task_graph_client_with_hierarchy(
    snapshot: &DaemonSnapshot,
    blocker_overrides: &[(&str, Vec<&str>)],
    parent_overrides: &[(&str, &str)],
) -> std::sync::Arc<dyn LinearTaskGraphClient> {
    let mut issues = snapshot
        .issues
        .iter()
        .map(|issue| tracker_issue_from_snapshot(issue, blocker_overrides))
        .collect::<Vec<_>>();

    for (child_identifier, parent_identifier) in parent_overrides {
        let parent_ref = issues
            .iter()
            .find(|issue| issue.identifier == *parent_identifier)
            .map(tracker_issue_ref_from_tracker)
            .unwrap_or_else(|| tracker_issue_ref_from_identifier(parent_identifier));
        let child_ref = issues
            .iter()
            .find(|issue| issue.identifier == *child_identifier)
            .map(tracker_issue_ref_from_tracker)
            .unwrap_or_else(|| tracker_issue_ref_from_identifier(child_identifier));

        if let Some(child_issue) = issues
            .iter_mut()
            .find(|issue| issue.identifier == *child_identifier)
        {
            child_issue.parent_id = Some(parent_ref.identifier.clone());
            child_issue.parent = Some(parent_ref);
        }

        if let Some(parent_issue) = issues
            .iter_mut()
            .find(|issue| issue.identifier == *parent_identifier)
        {
            parent_issue.sub_issues.push(child_ref);
        }
    }

    std::sync::Arc::new(FakeLinearTaskGraphClient { issues })
}

fn tracker_issue_from_snapshot(
    issue: &IssueSnapshot,
    blocker_overrides: &[(&str, Vec<&str>)],
) -> TrackerIssue {
    let blocked_by = blocker_overrides
        .iter()
        .find(|(identifier, _)| *identifier == issue.identifier)
        .map(|(_, blockers)| blockers.as_slice())
        .unwrap_or(&[]);
    TrackerIssue {
        id: issue.identifier.clone(),
        identifier: issue.identifier.clone(),
        url: format!("https://linear.app/kumanday/issue/{}", issue.identifier),
        title: issue.title.clone(),
        description: None,
        priority: None,
        state: issue.tracker_state.clone(),
        state_kind: tracker_state_kind_from_name(&issue.tracker_state),
        labels: Vec::new(),
        parent_id: None,
        parent: None,
        project_milestone: None,
        blocked_by: blocked_by
            .iter()
            .map(|identifier| TrackerIssueBlocker {
                id: (*identifier).to_owned(),
                identifier: (*identifier).to_owned(),
                title: format!("Blocker {identifier}"),
                state: TrackerIssueState {
                    id: format!("state-{identifier}"),
                    name: "Todo".to_owned(),
                    tracker_type: "unstarted".to_owned(),
                    kind: TrackerIssueStateKind::Unstarted,
                },
            })
            .collect(),
        sub_issues: Vec::new(),
        created_at: issue.last_event_at,
        updated_at: issue.last_event_at,
    }
}

fn tracker_issue_ref_from_tracker(issue: &TrackerIssue) -> TrackerIssueRef {
    TrackerIssueRef {
        id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        title: Some(issue.title.clone()),
        url: Some(issue.url.clone()),
        state: issue.state.clone(),
    }
}

fn tracker_issue_ref_from_identifier(identifier: &str) -> TrackerIssueRef {
    TrackerIssueRef {
        id: identifier.to_owned(),
        identifier: identifier.to_owned(),
        title: Some(format!("External {identifier}")),
        url: None,
        state: "Todo".to_owned(),
    }
}

fn tracker_state_kind_from_name(state: &str) -> TrackerIssueStateKind {
    match state.trim().to_ascii_lowercase().as_str() {
        "backlog" => TrackerIssueStateKind::Backlog,
        "todo" => TrackerIssueStateKind::Unstarted,
        "in progress" | "human review" | "review" => TrackerIssueStateKind::Started,
        "done" | "completed" | "closed" => TrackerIssueStateKind::Completed,
        "canceled" | "cancelled" => TrackerIssueStateKind::Canceled,
        other => TrackerIssueStateKind::Unknown(other.to_owned()),
    }
}

fn fixture_snapshot(step: u64) -> DaemonSnapshot {
    let now = Utc::now();
    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: DaemonState::Ready,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony".to_owned(),
            status_line: "ready".to_owned(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3000".to_owned(),
            conversation_count: 2,
            status_line: "healthy".to_owned(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 1,
            retry_queue_depth: 0,
            input_tokens: 2048,
            output_tokens: 2048,
            cache_read_tokens: 512,
            total_tokens: 4096 + step,
            total_cost_micros: 120_000,
        },
        issues: vec![IssueSnapshot {
            identifier: "COE-255".to_owned(),
            title: "Observability and FrankenTUI".to_owned(),
            tracker_state: "In Progress".to_owned(),
            runtime_state: IssueRuntimeState::Running,
            last_outcome: WorkerOutcome::Running,
            last_event_at: now,
            conversation_id_suffix: "c0e255".to_owned(),
            workspace_path_suffix: "COE-255".to_owned(),
            retry_count: 0,
            blocked: false,
            blocked_by: Vec::new(),
            server_base_url: Some("http://127.0.0.1:3000".to_owned()),
            transport_target: Some("loopback".to_owned()),
            http_auth_mode: Some("none".to_owned()),
            websocket_auth_mode: Some("none".to_owned()),
            websocket_query_param_name: None,
            recent_events: Vec::new(),
            modified_files: Vec::new(),
            input_tokens: 1024,
            output_tokens: 512,
            cache_read_tokens: 256,
            cancel_acknowledged: false,
            cancel_failed: false,
            detached: false,
        }],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-255".to_owned()),
            kind: RecentEventKind::SnapshotPublished,
            summary: format!("published step {step}"),
        }],
    }
}

fn fixture_envelope(step: u64) -> SnapshotEnvelope {
    let snapshot = fixture_snapshot(step);
    SnapshotEnvelope {
        sequence: step + 1,
        published_at: snapshot.generated_at,
        snapshot,
    }
}

/// Second fixture variant: one Idle issue, one Completed issue with events
/// and modified files, and one Failed issue (first attempt, no retries).
fn fixture_snapshot_rich(step: u64) -> DaemonSnapshot {
    let now = Utc::now();
    DaemonSnapshot {
        generated_at: now,
        daemon: DaemonStatus {
            state: DaemonState::Ready,
            last_poll_at: now,
            workspace_root: "/tmp/opensymphony".to_owned(),
            status_line: "ready".to_owned(),
        },
        agent_server: AgentServerStatus {
            reachable: true,
            base_url: "http://127.0.0.1:3000".to_owned(),
            conversation_count: 2,
            status_line: "healthy".to_owned(),
        },
        memory_server: Default::default(),
        metrics: MetricsSnapshot {
            running_issues: 1,
            retry_queue_depth: 0,
            input_tokens: 2048,
            output_tokens: 2048,
            cache_read_tokens: 512,
            total_tokens: 4096 + step,
            total_cost_micros: 120_000,
        },
        issues: vec![
            // Idle issue (eligible for execution)
            IssueSnapshot {
                identifier: "COE-300".to_owned(),
                title: "Idle task".to_owned(),
                tracker_state: "Todo".to_owned(),
                runtime_state: IssueRuntimeState::Idle,
                last_outcome: WorkerOutcome::Unknown,
                last_event_at: now,
                conversation_id_suffix: String::new(),
                workspace_path_suffix: String::new(),
                retry_count: 0,
                blocked: false,
                blocked_by: Vec::new(),
                server_base_url: None,
                transport_target: None,
                http_auth_mode: None,
                websocket_auth_mode: None,
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cancel_acknowledged: false,
                cancel_failed: false,
                detached: false,
            },
            // Completed issue with events and modified files
            IssueSnapshot {
                identifier: "COE-301".to_owned(),
                title: "Completed task".to_owned(),
                tracker_state: "Done".to_owned(),
                runtime_state: IssueRuntimeState::Completed,
                last_outcome: WorkerOutcome::Completed,
                last_event_at: now,
                conversation_id_suffix: "c0e301".to_owned(),
                workspace_path_suffix: "COE-301".to_owned(),
                retry_count: 0,
                blocked: false,
                blocked_by: Vec::new(),
                server_base_url: Some("http://127.0.0.1:3001".to_owned()),
                transport_target: Some("loopback".to_owned()),
                http_auth_mode: Some("none".to_owned()),
                websocket_auth_mode: Some("none".to_owned()),
                websocket_query_param_name: None,
                recent_events: vec![
                    ConversationEvent {
                        event_id: "evt-1".to_owned(),
                        happened_at: now,
                        kind: "worker_started".to_owned(),
                        summary: "worker started".to_owned(),
                        payload: Some(serde_json::json!({
                            "tool_name": "terminal",
                            "command": "npm test",
                        })),
                        sequence: 1,
                    },
                    ConversationEvent {
                        event_id: "evt-2".to_owned(),
                        happened_at: now,
                        kind: "worker_completed".to_owned(),
                        summary: "worker completed".to_owned(),
                        payload: None,
                        sequence: 2,
                    },
                ],
                modified_files: vec![
                    FileChange {
                        path: "/tmp/opensymphony/COE-301/src/main.rs".to_owned(),
                        change_kind: FileChangeKind::Modified,
                        lines_added: 10,
                        lines_removed: 3,
                        diff: Some(
                            "@@ -1,3 +1,10 @@\n\
                             -old line 1\n\
                             -old line 2\n\
                             -old line 3\n\
                             +new line 1\n\
                             +new line 2\n\
                             +new line 3\n\
                             +new line 4\n\
                             +new line 5\n\
                             +new line 6\n\
                             +new line 7\n\
                             +new line 8\n\
                             +new line 9\n\
                             +new line 10"
                                .to_owned(),
                        ),
                    },
                    FileChange {
                        path: "/tmp/opensymphony/COE-301/src/lib.rs".to_owned(),
                        change_kind: FileChangeKind::Created,
                        lines_added: 42,
                        lines_removed: 0,
                        diff: None,
                    },
                ],
                input_tokens: 2048,
                output_tokens: 1024,
                cache_read_tokens: 256,
                cancel_acknowledged: false,
                cancel_failed: false,
                detached: false,
            },
            // Failed issue, first attempt (no retries exhausted)
            IssueSnapshot {
                identifier: "COE-302".to_owned(),
                title: "Failed task".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::Failed,
                last_outcome: WorkerOutcome::Failed,
                last_event_at: now,
                conversation_id_suffix: "c0e302".to_owned(),
                workspace_path_suffix: "COE-302".to_owned(),
                retry_count: 0,
                blocked: false,
                blocked_by: Vec::new(),
                server_base_url: Some("http://127.0.0.1:3002".to_owned()),
                transport_target: Some("loopback".to_owned()),
                http_auth_mode: Some("none".to_owned()),
                websocket_auth_mode: Some("none".to_owned()),
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 512,
                output_tokens: 128,
                cache_read_tokens: 0,
                cancel_acknowledged: false,
                cancel_failed: false,
                detached: false,
            },
            // RetryQueued issue: queued but NOT eligible (not idle)
            IssueSnapshot {
                identifier: "COE-303".to_owned(),
                title: "Retry queued task".to_owned(),
                tracker_state: "In Progress".to_owned(),
                runtime_state: IssueRuntimeState::RetryQueued,
                last_outcome: WorkerOutcome::Failed,
                last_event_at: now,
                conversation_id_suffix: "c0e303".to_owned(),
                workspace_path_suffix: "COE-303".to_owned(),
                retry_count: 1,
                blocked: false,
                blocked_by: Vec::new(),
                server_base_url: Some("http://127.0.0.1:3003".to_owned()),
                transport_target: Some("loopback".to_owned()),
                http_auth_mode: Some("none".to_owned()),
                websocket_auth_mode: Some("none".to_owned()),
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 256,
                output_tokens: 64,
                cache_read_tokens: 0,
                cancel_acknowledged: false,
                cancel_failed: false,
                detached: false,
            },
            // Blocked Idle issue: NOT eligible AND NOT queued
            IssueSnapshot {
                identifier: "COE-304".to_owned(),
                title: "Blocked idle task".to_owned(),
                tracker_state: "Todo".to_owned(),
                runtime_state: IssueRuntimeState::Idle,
                last_outcome: WorkerOutcome::Unknown,
                last_event_at: now,
                conversation_id_suffix: String::new(),
                workspace_path_suffix: String::new(),
                retry_count: 0,
                blocked: true,
                blocked_by: vec!["COE-300".to_owned()],
                server_base_url: None,
                transport_target: None,
                http_auth_mode: None,
                websocket_auth_mode: None,
                websocket_query_param_name: None,
                recent_events: Vec::new(),
                modified_files: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cancel_acknowledged: false,
                cancel_failed: false,
                detached: false,
            },
        ],
        recent_events: vec![RecentEvent {
            happened_at: now,
            issue_identifier: Some("COE-301".to_owned()),
            kind: RecentEventKind::WorkerCompleted,
            summary: format!("completed step {step}"),
        }],
    }
}

#[test]
fn control_plane_to_dashboard_snapshot_maps_all_fields() {
    let envelope = fixture_envelope(5);
    let dashboard = control_plane_to_dashboard_snapshot(&envelope);

    assert_eq!(dashboard.schema_version.major, 1);
    assert_eq!(dashboard.sequence, 6);
    assert!(
        matches!(
            dashboard.health,
            opensymphony::opensymphony_gateway_schema::snapshot::GatewayHealth::Healthy
        ),
        "expected Healthy when daemon state is Ready"
    );

    let metrics = &dashboard.metrics;
    assert_eq!(metrics.running_issue_count, 1);
    assert_eq!(metrics.retry_queue_depth, 0);
    assert_eq!(metrics.total_input_tokens, 2048);
    assert_eq!(metrics.total_output_tokens, 2048);

    assert_eq!(dashboard.projects.len(), 1);
    let project = &dashboard.projects[0];
    assert_eq!(project.project_id, "default");
    assert_eq!(project.issue_count, 1);
    assert_eq!(project.running_count, 1);

    assert_eq!(dashboard.recent_events.len(), 1);
    let event = &dashboard.recent_events[0];
    assert_eq!(event.issue_identifier, Some("COE-255".to_owned()));
    assert_eq!(
        event.kind,
        opensymphony::opensymphony_gateway_schema::snapshot::SnapshotEventKind::SnapshotPublished
    );
}

#[test]
fn control_plane_to_dashboard_snapshot_handles_empty_issues() {
    let mut envelope = fixture_envelope(0);
    envelope.snapshot.issues.clear();
    envelope.snapshot.metrics.running_issues = 0;
    let dashboard = control_plane_to_dashboard_snapshot(&envelope);

    assert!(dashboard.projects.is_empty());
    assert_eq!(dashboard.metrics.running_issue_count, 0);
}

#[test]
fn gateway_capabilities_json_fixture_roundtrips() {
    let caps = GatewayCapabilities {
        schema_version: opensymphony::opensymphony_gateway_schema::version::SchemaVersion::v1(),
        gateway_version: "1.6.0".into(),
        supported_api_versions: vec!["1.0.0".into()],
        transports: vec![
            opensymphony::opensymphony_gateway_schema::capability::TransportCapability {
                transport: "sse".into(),
                modes: vec!["snapshot".into()],
                supported_encodings: vec!["utf-8".into()],
                bidirectional: false,
            },
        ],
        harnesses: vec![
            opensymphony::opensymphony_gateway_schema::capability::HarnessCapability::openhands_agent_server(),
        ],
        features: vec![
            opensymphony::opensymphony_gateway_schema::capability::FeatureCapability {
                feature: "planning".into(),
                available: true,
                requires_auth: false,
                requires_plan: None,
            },
        ],
        auth_modes: vec![
            opensymphony::opensymphony_gateway_schema::capability::AuthMode::None,
            opensymphony::opensymphony_gateway_schema::capability::AuthMode::ApiKey,
        ],
        max_event_page_size: 1000,
        max_terminal_frame_batch: 500,
    };

    let json = serde_json::to_string_pretty(&caps).expect("serialize capabilities");
    let back: GatewayCapabilities = serde_json::from_str(&json).expect("deserialize capabilities");

    assert_eq!(back.gateway_version, "1.6.0");
    assert_eq!(back.supported_api_versions, vec!["1.0.0"]);
    assert_eq!(back.auth_modes.len(), 2);
    assert_eq!(back.max_event_page_size, 1000);
    assert_eq!(back.harnesses[0].kind, "openhands_agent_server");
}

#[test]
fn gateway_model_settings_status_reflects_api_key_presence() {
    let installed_settings = model_settings_for_llm_api_key(Some("provider-secret"));
    let installed_profile = installed_settings
        .profiles
        .iter()
        .find(|profile| profile.id == "openhands-env-api-key")
        .expect("OpenHands env profile should exist");
    assert_eq!(installed_profile.status, CredentialStatusKind::Installed);
    assert!(installed_settings.credential_statuses.iter().any(|status| {
        status.credential_reference_id == "credential:env:LLM_API_KEY"
            && status.status == CredentialStatusKind::Installed
    }));

    let missing_settings = model_settings_for_llm_api_key(None);
    let missing_profile = missing_settings
        .profiles
        .iter()
        .find(|profile| profile.id == "openhands-env-api-key")
        .expect("OpenHands env profile should exist");
    assert_eq!(missing_profile.status, CredentialStatusKind::LoggedOut);
    assert!(missing_settings.credential_statuses.iter().any(|status| {
        status.credential_reference_id == "credential:env:LLM_API_KEY"
            && status.status == CredentialStatusKind::LoggedOut
    }));

    let blank_settings = model_settings_for_llm_api_key(Some("   "));
    let blank_profile = blank_settings
        .profiles
        .iter()
        .find(|profile| profile.id == "openhands-env-api-key")
        .expect("OpenHands env profile should exist");
    assert_eq!(blank_profile.status, CredentialStatusKind::LoggedOut);
    assert!(blank_settings.credential_statuses.iter().any(|status| {
        status.credential_reference_id == "credential:env:LLM_API_KEY"
            && status.status == CredentialStatusKind::LoggedOut
    }));
}

#[test]
fn gateway_model_settings_reflects_codex_cli_readiness() {
    let ready = CodexLocalReadiness::from_probe(CodexCliProbe {
        command: "codex".into(),
        version: ProbeCommandResult::success("codex-cli 0.138.0\n"),
        app_server_help: ProbeCommandResult::success("Usage: codex app-server\n"),
        login_status: ProbeCommandResult::success("Logged in using ChatGPT\n"),
    });
    let settings =
        model_settings_for_llm_api_key_and_codex_readiness(Some("provider-secret"), ready);

    assert_eq!(
        settings.codex_local_readiness.subscription_status,
        CredentialStatusKind::Installed
    );
    assert!(settings.profiles.iter().any(|profile| {
        profile.id == "codex-chatgpt-local-keychain"
            && profile.status == CredentialStatusKind::Installed
    }));
    assert!(settings.credential_statuses.iter().any(|status| {
        status.credential_reference_id == "credential:codex-cli:chatgpt-login"
            && status.status == CredentialStatusKind::Installed
            && status.checked_by == "codex_cli_supported_commands"
    }));

    let logged_out = CodexLocalReadiness::from_probe(CodexCliProbe {
        command: "codex".into(),
        version: ProbeCommandResult::success("codex-cli 0.138.0\n"),
        app_server_help: ProbeCommandResult::success("Usage: codex app-server\n"),
        login_status: ProbeCommandResult::failure("Not logged in"),
    });
    let settings = model_settings_for_llm_api_key_and_codex_readiness(None, logged_out);
    assert_eq!(
        settings.codex_local_readiness.subscription_status,
        CredentialStatusKind::LoggedOut
    );
    assert!(settings.profiles.iter().any(|profile| {
        profile.id == "codex-chatgpt-local-keychain"
            && profile.status == CredentialStatusKind::LoggedOut
    }));
}

#[test]
fn dashboard_snapshot_json_fixture_roundtrips() {
    let envelope = fixture_envelope(7);
    let dashboard = control_plane_to_dashboard_snapshot(&envelope);
    let json = serde_json::to_string_pretty(&dashboard).expect("serialize dashboard");
    let back: opensymphony::opensymphony_gateway_schema::snapshot::DashboardSnapshot =
        serde_json::from_str(&json).expect("deserialize dashboard");

    assert_eq!(back.sequence, 8);
    assert_eq!(back.projects.len(), 1);
    assert_eq!(back.metrics.running_issue_count, 1);
}

#[tokio::test]
async fn gateway_serves_capabilities_and_dashboard_snapshot() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let health_url = format!("http://{address}/healthz");
    let health_response = client
        .get(&health_url)
        .send()
        .await
        .expect("fetch healthz")
        .json::<serde_json::Value>()
        .await
        .expect("decode healthz");
    assert_eq!(health_response["status"], "ok");
    assert_eq!(health_response["current_sequence"], 1);

    let control_snapshot_url = format!("http://{address}/api/v1/snapshot");
    let control_snapshot_response = client
        .get(&control_snapshot_url)
        .send()
        .await
        .expect("fetch control snapshot")
        .json::<SnapshotEnvelope>()
        .await
        .expect("decode control snapshot");
    assert_eq!(control_snapshot_response.sequence, 1);

    let caps_url = format!("http://{address}/api/v1/capabilities");
    let caps_response = client
        .get(&caps_url)
        .send()
        .await
        .expect("fetch capabilities")
        .json::<GatewayCapabilities>()
        .await
        .expect("decode capabilities");

    assert!(
        caps_response
            .harnesses
            .iter()
            .any(|harness| harness.kind == "openhands_agent_server" && harness.available)
    );
    assert!(
        caps_response
            .harnesses
            .iter()
            .any(|harness| harness.kind == "codex_app_server"
                && harness.available
                && harness.runtime_contract_version.as_deref()
                    == Some("codex-app-server-json-rpc-v2")
                && harness.transport.modes == vec!["stdio"])
    );
    assert!(
        caps_response
            .features
            .iter()
            .any(|feature| feature.feature == "model_settings" && feature.available)
    );

    let model_settings_url = format!("http://{address}/api/v1/model-settings");
    let model_settings_response = client
        .get(&model_settings_url)
        .send()
        .await
        .expect("fetch model settings")
        .json::<ModelSettingsResponse>()
        .await
        .expect("decode model settings");
    // The endpoint derives API-key status from process environment. To avoid
    // mutating global env in an async integration test, installed, missing, and
    // blank-key cases are covered by
    // `gateway_model_settings_status_reflects_api_key_presence`.
    assert!(model_settings_response.profiles.iter().any(|profile| {
        profile.id == "openhands-env-api-key"
            && profile.compatible_harnesses == vec!["openhands_agent_server"]
            && profile.credential_reference.redacted
    }));
    assert!(model_settings_response.profiles.iter().any(|profile| {
        profile.id == "hosted-openai-subscription-broker"
            && profile.status == CredentialStatusKind::Unsupported
    }));
    assert!(
        model_settings_response
            .codex_local_readiness
            .status_command
            .contains("codex login status")
    );

    let credential_status_url = format!("http://{address}/api/v1/model-settings/credential-status");
    let credential_status_response = client
        .get(&credential_status_url)
        .send()
        .await
        .expect("fetch credential statuses")
        .json::<CredentialStatusResponse>()
        .await
        .expect("decode credential statuses");
    assert!(
        credential_status_response
            .supported_statuses
            .contains(&CredentialStatusKind::Expired)
    );
    assert!(
        credential_status_response
            .supported_statuses
            .contains(&CredentialStatusKind::PermissionDenied)
    );

    let snapshot_url = format!("http://{address}/api/v1/dashboard/snapshot");
    let snapshot_response = client
        .get(&snapshot_url)
        .send()
        .await
        .expect("fetch dashboard snapshot")
        .json::<opensymphony::opensymphony_gateway_schema::snapshot::DashboardSnapshot>()
        .await
        .expect("decode dashboard snapshot");

    assert_eq!(snapshot_response.sequence, 1);
    assert_eq!(snapshot_response.projects.len(), 1);
    assert_eq!(snapshot_response.projects[0].project_id, "default");

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_configured_web_assets() {
    let assets = tempfile::tempdir().expect("create assets tempdir");
    std::fs::write(
        assets.path().join("index.html"),
        "<main>OpenSymphony</main>",
    )
    .expect("write index.html");
    std::fs::write(assets.path().join("app.js"), "console.log('opensymphony');")
        .expect("write app.js");
    std::fs::write(assets.path().join("demo.mp4"), b"fake mp4").expect("write demo.mp4");
    std::fs::write(assets.path().join("report.pdf"), b"%PDF-1.7").expect("write report.pdf");

    let store = SnapshotStore::new(fixture_snapshot(0));
    let server =
        GatewayServer::new(store).with_web_assets(assets.path().to_string_lossy().to_string());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let app_root = client
        .get(format!("http://{address}/app"))
        .send()
        .await
        .expect("fetch app root");
    assert_eq!(app_root.status(), reqwest::StatusCode::OK);
    assert_eq!(
        app_root
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    assert!(
        app_root
            .text()
            .await
            .expect("read app root body")
            .contains("OpenSymphony")
    );

    let app_js = client
        .get(format!("http://{address}/app/app.js"))
        .send()
        .await
        .expect("fetch app js");
    assert_eq!(app_js.status(), reqwest::StatusCode::OK);
    assert_eq!(
        app_js
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/javascript; charset=utf-8")
    );
    assert!(
        app_js
            .text()
            .await
            .expect("read app js body")
            .contains("opensymphony")
    );

    let app_video = client
        .get(format!("http://{address}/app/demo.mp4"))
        .send()
        .await
        .expect("fetch app video");
    assert_eq!(app_video.status(), reqwest::StatusCode::OK);
    assert_eq!(
        app_video
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("video/mp4")
    );

    let app_pdf = client
        .get(format!("http://{address}/app/report.pdf"))
        .send()
        .await
        .expect("fetch app pdf");
    assert_eq!(app_pdf.status(), reqwest::StatusCode::OK);
    assert_eq!(
        app_pdf
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/pdf")
    );

    let spa_route = client
        .get(format!("http://{address}/app/projects/COE-393"))
        .send()
        .await
        .expect("fetch spa route");
    assert_eq!(spa_route.status(), reqwest::StatusCode::OK);
    assert_eq!(
        spa_route
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    assert!(
        spa_route
            .text()
            .await
            .expect("read spa route body")
            .contains("OpenSymphony")
    );

    let missing_asset = client
        .get(format!("http://{address}/app/missing.js"))
        .send()
        .await
        .expect("fetch missing asset");
    assert_eq!(missing_asset.status(), reqwest::StatusCode::NOT_FOUND);

    server_task.abort();
}

#[tokio::test]
async fn gateway_web_assets_reject_path_traversal() {
    let root = tempfile::tempdir().expect("create tempdir");
    let assets_dir = root.path().join("assets");
    std::fs::create_dir(&assets_dir).expect("create assets dir");
    std::fs::write(assets_dir.join("index.html"), "<main>OpenSymphony</main>")
        .expect("write index.html");
    std::fs::write(root.path().join("secret.txt"), "secret").expect("write secret");

    let store = SnapshotStore::new(fixture_snapshot(0));
    let server =
        GatewayServer::new(store).with_web_assets(assets_dir.to_string_lossy().to_string());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let response = reqwest::Client::new()
        .get(format!("http://{address}/app/%2e%2e/secret.txt"))
        .send()
        .await
        .expect("fetch traversal attempt");

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
    assert_ne!(response.text().await.expect("read response body"), "secret");

    server_task.abort();
}

#[tokio::test]
/// SSE endpoint now streams journal events (not snapshot updates).
/// This test verifies the SSE transport works with journal events and
/// delivers new events appended after the stream opens.
async fn gateway_events_stream_yields_journal_events() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");

    // Keep a clone of the journal so we can append events after the stream opens.
    let (journal, broker) = server.journal_and_broker();
    let server = GatewayServer::with_journal(store.clone(), journal.clone(), broker);
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let events_url =
        Url::parse(&format!("http://{address}/api/v1/events")).expect("valid events url");

    let client = reqwest::Client::new();
    let response = client
        .get(events_url)
        .send()
        .await
        .expect("open SSE stream");

    assert_eq!(
        response
            .headers()
            .get("content-type")
            .expect("content-type header")
            .to_str()
            .expect("valid header value"),
        "text/event-stream"
    );

    let mut stream = response.bytes_stream();
    let timeout_dur = std::time::Duration::from_secs(2);

    // Append an event after the stream opens and expect it to arrive via SSE.
    let event = opensymphony::opensymphony_domain::InMemoryEventJournal::orchestrator_event(
        opensymphony::opensymphony_gateway_schema::event_journal::EventKind::RunStarted,
        "test run started",
        None,
    );
    let _ = journal.append(event).await;

    // Read the journal event into a buffer.
    let mut first_buf = Vec::new();
    #[allow(clippy::while_let_loop)]
    loop {
        match tokio::time::timeout(timeout_dur, stream.next()).await {
            Ok(Some(Ok(chunk))) => {
                first_buf.extend_from_slice(&chunk);
                if first_buf.ends_with(b"\n\n") || first_buf.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            Ok(Some(Err(_))) | Ok(None) | Err(_) => break,
        }
    }
    let first_text =
        String::from_utf8(first_buf).expect("SSE event is valid UTF-8 when fully assembled");
    assert!(
        !first_text.is_empty() && first_text.contains("event: event"),
        "SSE event should be a journal event, got: {first_text}"
    );

    // Verify the payload is a valid EventRecord.
    let data_line = first_text
        .lines()
        .find(|l| l.starts_with("data:"))
        .expect("SSE event contains data line");
    let json_payload = data_line.trim_start_matches("data:").trim();
    let record: opensymphony::opensymphony_gateway_schema::event_journal::EventRecord =
        serde_json::from_str(json_payload).expect("deserialize SSE payload as EventRecord");
    assert_eq!(record.kind.kind_tag(), "run.started");

    server_task.abort();
}

#[tokio::test]
async fn gateway_and_control_plane_are_reachable() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let gateway = GatewayServer::new(store.clone());
    let control = ControlPlaneServer::new(store);

    let gateway_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind gateway listener");
    let gateway_address = gateway_listener
        .local_addr()
        .expect("gateway listener address");
    let gateway_task = tokio::spawn(async move {
        gateway
            .serve(gateway_listener)
            .await
            .expect("gateway serve")
    });

    let control_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind control listener");
    let control_address = control_listener
        .local_addr()
        .expect("control listener address");
    let control_task = tokio::spawn(async move {
        control
            .serve(control_listener)
            .await
            .expect("control serve")
    });

    let client = reqwest::Client::new();

    let gateway_caps = client
        .get(format!("http://{gateway_address}/api/v1/capabilities"))
        .send()
        .await
        .expect("gateway capabilities reachable");
    assert!(gateway_caps.status().is_success());

    let gateway_snapshot = client
        .get(format!(
            "http://{gateway_address}/api/v1/dashboard/snapshot"
        ))
        .send()
        .await
        .expect("gateway dashboard snapshot reachable");
    assert!(gateway_snapshot.status().is_success());

    let control_snapshot = client
        .get(format!("http://{control_address}/api/v1/snapshot"))
        .send()
        .await
        .expect("control snapshot reachable");
    assert!(control_snapshot.status().is_success());

    gateway_task.abort();
    control_task.abort();
}

/// Test that cursor-based event journal query works end-to-end via HTTP.
#[tokio::test]
async fn event_journal_cursor_returns_events_page() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());

    for i in 0..5u64 {
        let event = EventRecord::builder()
            .event_id(format!("evt_{i}"))
            .sequence(0)
            .actor(EventActor::system("test"))
            .kind(EventKind::RunStarted)
            .summary(format!("Test event {i}"))
            .build();
        journal.append(event).await.expect("append");
    }

    let server = GatewayServer::with_journal(store, journal, broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let url = format!("http://{address}/api/v1/event-journal?cursor=0&limit=2");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 2);
    assert!(page.has_more);
    assert!(page.next_cursor.is_some());

    let next_seq = page.next_cursor.expect("next cursor must exist").sequence;
    let url = format!("http://{address}/api/v1/event-journal?cursor={next_seq}&limit=2");
    let page2: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch next page")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page2.events.len(), 2);
    assert!(page2.has_more);

    let next_seq2 = page2.next_cursor.expect("next cursor must exist").sequence;
    let url = format!("http://{address}/api/v1/event-journal?cursor={next_seq2}&limit=2");
    let page3: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch last page")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page3.events.len(), 1);
    assert!(!page3.has_more);

    server_task.abort();
}

/// Test that partition filtering works via the event journal API.
#[tokio::test]
async fn event_journal_partition_filtering() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    let event = EventRecord::builder()
        .event_id("evt_control")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Control event")
        .build();
    journal.append(event).await.expect("append");

    let terminal = EventRecord::builder()
        .event_id("evt_term")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::TerminalFrame {
            frame_id: "f1".into(),
        })
        .summary("Terminal frame")
        .build();
    journal.append(terminal).await.expect("append");

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let url = format!("http://{address}/api/v1/event-journal?partition=events");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id, "evt_control");

    let url = format!("http://{address}/api/v1/event-journal?partition=terminal_log");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event_id, "evt_term");

    server_task.abort();
}

/// Test that unknown harness events with raw payload refs are retained.
#[tokio::test]
async fn event_journal_raw_payload_ref_retained() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    let event = EventRecord::builder()
        .event_id("evt_raw")
        .sequence(0)
        .actor(EventActor::harness("openhands-1"))
        .kind(EventKind::Unknown {
            raw_kind: "custom_harness_event".into(),
        })
        .summary("Unknown harness event")
        .raw_payload_ref("raw_ref_123")
        .build();
    journal.append(event).await.expect("append");

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let url = format!("http://{address}/api/v1/event-journal");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 1);
    assert!(page.events[0].has_raw_payload());
    assert_eq!(page.events[0].raw_payload_ref, Some("raw_ref_123".into()));

    server_task.abort();
}

/// Test that duplicate events are identifiable by stable event_id.
#[tokio::test]
async fn event_journal_duplicate_detection() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventPage, EventRecord,
    };

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    let event = EventRecord::builder()
        .event_id("evt_dup")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Duplicate event")
        .build();
    journal.append(event.clone()).await.expect("append");
    journal.append(event).await.expect("append");

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();

    let url = format!("http://{address}/api/v1/event-journal");
    let page: EventPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch events")
        .json::<EventPage>()
        .await
        .expect("decode events page");

    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].event_id, page.events[1].event_id);
    assert_ne!(page.events[0].sequence, page.events[1].sequence);

    server_task.abort();
}

/// Test that the WebSocket event stream endpoint works end-to-end.
/// Connects, sends an init message, receives backlog events, then a live event.
#[tokio::test]
async fn websocket_event_stream_delivers_events() {
    use futures_util::SinkExt;
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventRecord,
    };
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());

    let backlog_event = EventRecord::builder()
        .event_id("ws_test_1")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunStarted)
        .summary("Backlog event")
        .build();
    journal.append(backlog_event).await.expect("append");

    let server = GatewayServer::with_journal(store, journal.clone(), broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let ws_url = format!("ws://{address}/api/v1/streams/events");
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect to WS endpoint");

    let (mut write, mut read) = ws_stream.split();

    let init = serde_json::json!({ "cursor": 0, "partition": "events" });
    let init_msg = serde_json::to_string(&init).expect("serialize init");
    write
        .send(WsMessage::Text(init_msg.into()))
        .await
        .expect("send init");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), read.next())
        .await
        .expect("timed out waiting for backlog event")
        .expect("should receive a message")
        .expect("no WS error");
    let text = msg.to_text().expect("text message");
    assert!(
        text.starts_with("__event__"),
        "Expected __event__ prefix, got: {text}"
    );
    assert!(
        text.contains("ws_test_1"),
        "Backlog event should contain event_id ws_test_1, got: {text}"
    );

    let live_event = EventRecord::builder()
        .event_id("ws_test_2")
        .sequence(0)
        .actor(EventActor::system("test"))
        .kind(EventKind::RunCompleted)
        .summary("Live event")
        .build();
    journal.append(live_event).await.expect("append live");

    let msg = tokio::time::timeout(std::time::Duration::from_secs(3), read.next())
        .await
        .expect("timed out waiting for live event")
        .expect("should receive a message")
        .expect("no WS error");
    let text = msg.to_text().expect("text message");
    assert!(
        text.starts_with("__event__"),
        "Expected __event__ prefix, got: {text}"
    );
    assert!(
        text.contains("ws_test_2"),
        "Live event should contain event_id ws_test_2, got: {text}"
    );

    server_task.abort();
}

// ── Read API endpoint tests ────────────────────────────────────────────────────

#[tokio::test]
async fn gateway_serves_project_list() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/projects"))
        .send()
        .await
        .expect("fetch projects")
        .json::<opensymphony::opensymphony_gateway_schema::snapshot::ProjectList>()
        .await
        .expect("decode project list");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.projects.len(), 1);
    assert_eq!(response.projects[0].project_id, "default");
    assert_eq!(response.projects[0].name, "OpenSymphony");
    assert_eq!(response.projects[0].issue_count, 1);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_project_detail() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/projects/default"))
        .send()
        .await
        .expect("fetch project detail")
        .json::<opensymphony::opensymphony_gateway_schema::snapshot::ProjectDetail>()
        .await
        .expect("decode project detail");

    assert_eq!(response.project_id, "default");
    assert_eq!(response.name, "OpenSymphony");
    assert_eq!(response.issue_count, 1);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_task_graph() {
    let snapshot = fixture_snapshot(0);
    let store = SnapshotStore::new(snapshot.clone());
    let server = GatewayServer::new(store.clone())
        .with_linear_task_graph(Some(fake_linear_task_graph_client(&snapshot, &[])));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.project_id, "default");
    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.nodes[0].identifier, "COE-255");
    assert_eq!(response.root_ids, vec!["COE-255".to_owned()]);
    // Verify runtime overlay is present
    assert!(response.nodes[0].runtime_overlay.is_some());
    let overlay = response.nodes[0]
        .runtime_overlay
        .as_ref()
        .expect("task graph node should have runtime overlay");
    // Running issues are NOT eligible (only Idle issues are eligible).
    assert!(!overlay.eligible);
    assert_eq!(overlay.active_run_id, Some("COE-255".into()));

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_requires_linear_reader() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let response = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph");

    assert_eq!(response.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_detail() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255"))
        .send()
        .await
        .expect("fetch run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.run_id, "COE-255");
    assert_eq!(response.issue_identifier, "COE-255");
    assert_eq!(
        response.status,
        opensymphony::opensymphony_gateway_schema::run::RunStatus::Running
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_events() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255/events"))
        .send()
        .await
        .expect("fetch run events")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunEventPage>()
        .await
        .expect("decode run events");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.run_id, "COE-255");
    // The fixture has no recent_events for the issue, so page is empty
    assert!(response.events.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_files() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255/files"))
        .send()
        .await
        .expect("fetch run files")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunFilesPage>()
        .await
        .expect("decode run files");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.run_id, "COE-255");
    // The fixture has no modified_files, so page is empty
    assert!(response.files.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_diffs() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-255/diffs"))
        .send()
        .await
        .expect("fetch run diffs")
        .json::<opensymphony::opensymphony_gateway_schema::run::FileDiffPage>()
        .await
        .expect("decode run diffs");

    assert_eq!(response.schema_version.major, 1);
    assert_eq!(response.run_id, "COE-255");
    assert!(response.hunks.is_empty());

    server_task.abort();
}

#[test]
fn sanitize_file_path_strips_workspace_root() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony",
        "/tmp/opensymphony/COE-255/src/main.rs",
    );
    assert_eq!(result, "COE-255/src/main.rs");
}

#[test]
fn sanitize_file_path_falls_back_to_basename_for_unsafe_path() {
    let result =
        opensymphony::opensymphony_gateway::sanitize_file_path("/tmp/opensymphony", "/etc/passwd");
    assert_eq!(result, "passwd");
}

// ── Path traversal tests ─────────────────────────────────────────────────────

#[test]
fn sanitize_file_path_blocks_path_traversal_via_dotdot() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony",
        "/tmp/opensymphony/../etc/passwd",
    );
    // The traversal escapes the workspace root, so the fallback basename
    // (`passwd`) is returned instead of leaking `../etc/passwd`.
    assert_eq!(result, "passwd");
}

#[test]
fn sanitize_file_path_blocks_nested_path_traversal() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony",
        "/tmp/opensymphony/COE-255/../../etc/passwd",
    );
    assert_eq!(result, "passwd");
}

// Workspace root normalization: a crafted root that tries to escape its own
// boundary is normalized before the strip, so the file still resolves safely.
#[test]
fn sanitize_file_path_normalizes_workspace_root() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/other/../opensymphony",
        "/tmp/opensymphony/COE-255/src/main.rs",
    );
    assert_eq!(result, "COE-255/src/main.rs");
}

// When both root and file contain `..` components, normalization on both sides
// prevents a crafted root from widening the accepted prefix.
#[test]
fn sanitize_file_path_normalizes_both_sides() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path(
        "/tmp/opensymphony/../opensymphony",
        "/tmp/other/../opensymphony/../etc/passwd",
    );
    // Normalized: root=/tmp/opensymphony, file=/tmp/etc/passwd → escapes root
    assert_eq!(result, "passwd");
}

// Empty string file name fallback: a raw path that is only a root dir yields
// an empty string instead of leaking the workspace root.
#[test]
fn sanitize_file_path_empty_fallback_for_root_only_path() {
    let result = opensymphony::opensymphony_gateway::sanitize_file_path("/tmp/opensymphony", "/");
    assert_eq!(result, "");
}

// ── 404 negative-path tests ───────────────────────────────────────────────────

#[tokio::test]
async fn gateway_returns_404_for_unknown_project() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{address}/api/v1/projects/nonexistent"))
        .send()
        .await
        .expect("fetch unknown project");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_project_task_graph() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{address}/api/v1/projects/nonexistent/taskgraph"
        ))
        .send()
        .await
        .expect("fetch unknown task graph");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999"))
        .send()
        .await
        .expect("fetch unknown run");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_events() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999/events"))
        .send()
        .await
        .expect("fetch unknown run events");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_files() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999/files"))
        .send()
        .await
        .expect("fetch unknown run files");

    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_diffs() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{address}/api/v1/runs/UNKNOWN-999/diffs"))
        .send()
        .await
        .expect("fetch unknown run diffs");

    assert_eq!(resp.status(), 404);

    // Assert the 404 response body is well-formed
    let body: opensymphony::opensymphony_gateway_schema::run::FileDiffPage =
        resp.json().await.expect("decode 404 run diffs body");
    assert_eq!(body.run_id, "UNKNOWN-999");
    assert!(body.hunks.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_validation() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{address}/api/v1/runs/UNKNOWN-999/validation"
        ))
        .send()
        .await
        .expect("fetch unknown run validation");

    assert_eq!(resp.status(), 404);
    let body: opensymphony::opensymphony_gateway_schema::validation::RunValidationSummary =
        resp.json().await.expect("decode 404 run validation body");
    assert_eq!(body.run_id, "UNKNOWN-999");
    assert_eq!(body.overall_status, ValidationStatus::Error);
    assert!(body.commands.is_empty());
    assert!(body.evidence.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_returns_404_for_unknown_run_approvals() {
    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{address}/api/v1/runs/UNKNOWN-999/approvals"
        ))
        .send()
        .await
        .expect("fetch unknown run approvals");

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.expect("decode 404 run approvals body");
    assert_eq!(body["run_id"].as_str(), Some("UNKNOWN-999"));
    assert!(body["approvals"].as_array().is_none_or(|a| a.is_empty()));

    server_task.abort();
}

// ── Rich fixture tests (non-Running states, file/diff data) ────────────────────

#[tokio::test]
async fn gateway_serves_run_files_with_modified_files() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/files"))
        .send()
        .await
        .expect("fetch run files with data")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunFilesPage>()
        .await
        .expect("decode run files");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(response.files.len(), 2);
    // Files should have workspace root stripped
    let paths: Vec<_> = response.files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"COE-301/src/main.rs"));
    assert!(paths.contains(&"COE-301/src/lib.rs"));

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_diffs_with_modified_files() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/diffs"))
        .send()
        .await
        .expect("fetch run diffs with data")
        .json::<opensymphony::opensymphony_gateway_schema::run::FileDiffPage>()
        .await
        .expect("decode run diffs");

    assert_eq!(response.run_id, "COE-301");
    // Multi-file diff should show count label instead of single path
    assert_eq!(response.file_path, "[2 files]");
    assert_eq!(response.hunks.len(), 2);
    assert_eq!(response.total_lines_added, 52);
    assert_eq!(response.total_lines_removed, 3);
    // The first file has a real unified diff, so its hunk is populated with
    // line-level additions and deletions instead of an empty placeholder.
    let first_hunk = response.hunks.first().expect("first hunk");
    assert_eq!(first_hunk.lines.len(), 13);
    let added = first_hunk
        .lines
        .iter()
        .filter(|l| matches!(l, DiffLine::Addition { .. }))
        .count();
    let removed = first_hunk
        .lines
        .iter()
        .filter(|l| matches!(l, DiffLine::Deletion { .. }))
        .count();
    assert_eq!(added, 10);
    assert_eq!(removed, 3);
    assert_eq!(first_hunk.header, "@@ -1,3 +1,10 @@");
    assert_eq!(first_hunk.file_path, "COE-301/src/main.rs");
    let second_hunk = response.hunks.get(1).expect("second hunk");
    assert_eq!(second_hunk.file_path, "COE-301/src/lib.rs");

    let response = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-301/diffs?file_path=./COE-301/src/main.rs"
        ))
        .send()
        .await
        .expect("fetch normalized run diff")
        .json::<opensymphony::opensymphony_gateway_schema::run::FileDiffPage>()
        .await
        .expect("decode normalized run diff");

    assert_eq!(response.file_path, "COE-301/src/main.rs");
    assert_eq!(response.hunks.len(), 1);
    assert_eq!(response.hunks[0].file_path, "COE-301/src/main.rs");

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_validation_with_modified_files() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/validation"))
        .send()
        .await
        .expect("fetch run validation with data")
        .json::<opensymphony::opensymphony_gateway_schema::validation::RunValidationSummary>()
        .await
        .expect("decode run validation");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(response.overall_status, ValidationStatus::Passed);
    assert!(response.commands.is_empty());
    assert!(response.evidence.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_approvals_with_context() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301/approvals"))
        .send()
        .await
        .expect("fetch run approvals with data")
        .json::<serde_json::Value>()
        .await
        .expect("decode run approvals");

    assert_eq!(response["run_id"].as_str(), Some("COE-301"));
    let approvals = response["approvals"].as_array().expect("approvals array");
    assert!(approvals.is_empty());

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_events_with_data() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-301/events?page_size=1"
        ))
        .send()
        .await
        .expect("fetch run events with data")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunEventPage>()
        .await
        .expect("decode run events");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(response.events.len(), 1);
    assert_eq!(response.events[0].sequence, 1);
    assert_eq!(
        response.events[0].payload,
        Some(serde_json::json!({
            "tool_name": "terminal",
            "command": "npm test",
        }))
    );
    assert_eq!(
        response.events[0]
            .raw_payload
            .as_ref()
            .and_then(|payload| payload.get("payload"))
            .and_then(|payload| payload.get("command"))
            .and_then(serde_json::Value::as_str),
        Some("npm test")
    );
    assert_eq!(
        response
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.page_token.as_str()),
        Some("2")
    );

    let response = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-301/events?page_token=2&page_size=1"
        ))
        .send()
        .await
        .expect("fetch second run events page")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunEventPage>()
        .await
        .expect("decode second run events page");

    assert_eq!(response.events.len(), 1);
    assert_eq!(response.events[0].sequence, 2);
    assert!(response.next_cursor.is_none());

    let response = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-301/events?cursor=2&page_size=1"
        ))
        .send()
        .await
        .expect("fetch desktop cursor run events page")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunEventPage>()
        .await
        .expect("decode desktop cursor run events page");

    assert_eq!(response.events.len(), 1);
    assert_eq!(response.events[0].sequence, 2);

    let invalid_response = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-301/events?page_token=opaque"
        ))
        .send()
        .await
        .expect("fetch invalid run events page");
    assert_eq!(invalid_response.status(), reqwest::StatusCode::BAD_REQUEST);

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_eligible_for_idle_issue() {
    let snapshot = fixture_snapshot_rich(0);
    let store = SnapshotStore::new(snapshot.clone());
    let server = GatewayServer::new(store.clone()).with_linear_task_graph(Some(
        fake_linear_task_graph_client_with_hierarchy(
            &snapshot,
            &[("COE-304", vec!["COE-300", "COE-999"])],
            &[
                ("COE-304", "COE-300"),
                ("COE-999", "COE-300"),
                ("COE-302", "COE-999"),
            ],
        ),
    ));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    // Find the idle issue overlay
    let idle_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-300")
        .expect("COE-300 node should exist");
    let overlay = idle_node.runtime_overlay.as_ref().expect("overlay present");
    // Idle + not blocked = eligible
    assert!(overlay.eligible);
    assert!(overlay.queued);

    let blocked_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-304")
        .expect("COE-304 node should exist");
    assert_eq!(blocked_node.parent_id.as_deref(), Some("COE-300"));
    assert_eq!(blocked_node.blocked_by, vec!["COE-300".to_owned()]);
    let parent_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-300")
        .expect("COE-300 node should exist");
    assert_eq!(parent_node.children, vec!["COE-304".to_owned()]);

    let external_parent_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-302")
        .expect("COE-302 node should exist");
    assert!(external_parent_node.parent_id.is_none());

    // Completed issue should NOT be eligible
    let done_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-301")
        .expect("COE-301 node should exist");
    let done_overlay = done_node.runtime_overlay.as_ref().expect("overlay present");
    assert!(!done_overlay.eligible);

    assert_eq!(
        response.root_ids,
        vec![
            "COE-300".to_owned(),
            "COE-301".to_owned(),
            "COE-302".to_owned(),
            "COE-303".to_owned()
        ]
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_failed_without_retries() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-302"))
        .send()
        .await
        .expect("fetch failed run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.run_id, "COE-302");
    // Failed with retry_count == 0 should map to TrackerTerminal, not RetryExhausted
    assert_eq!(
        response.release_reason,
        Some(opensymphony::opensymphony_gateway_schema::run::ReleaseReason::TrackerTerminal)
    );
    // Finished at should be set for terminal states
    assert!(response.finished_at.is_some());

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_completed_state() {
    let store = SnapshotStore::new(fixture_snapshot_rich(0));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{address}/api/v1/runs/COE-301"))
        .send()
        .await
        .expect("fetch completed run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.run_id, "COE-301");
    assert_eq!(
        response.release_reason,
        Some(opensymphony::opensymphony_gateway_schema::run::ReleaseReason::Completed)
    );
    assert!(response.finished_at.is_some());

    server_task.abort();
}

// ── Runtime overlay: queued vs eligible semantics ──────────────────────────────

#[tokio::test]
async fn gateway_task_graph_queued_vs_eligible() {
    let snapshot = fixture_snapshot_rich(0);
    let store = SnapshotStore::new(snapshot.clone());
    let server = GatewayServer::new(store.clone())
        .with_linear_task_graph(Some(fake_linear_task_graph_client(&snapshot, &[])));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "http://{address}/api/v1/projects/default/taskgraph"
        ))
        .send()
        .await
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    // Idle + not blocked → eligible AND queued
    let idle_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-300")
        .expect("COE-300 node should exist");
    let idle_overlay = idle_node.runtime_overlay.as_ref().expect("overlay present");
    assert!(
        idle_overlay.eligible,
        "Idle unblocked issue should be eligible"
    );
    assert!(idle_overlay.queued, "Idle unblocked issue should be queued");

    // RetryQueued → queued BUT NOT eligible (not in Idle state)
    let retry_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-303")
        .expect("COE-303 node should exist");
    let retry_overlay = retry_node
        .runtime_overlay
        .as_ref()
        .expect("overlay present");
    assert!(
        !retry_overlay.eligible,
        "RetryQueued issue must NOT be eligible (not idle)"
    );
    assert!(
        retry_overlay.queued,
        "RetryQueued issue must be queued (waiting for retry)"
    );

    // Completed → neither eligible nor queued
    let done_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-301")
        .expect("COE-301 node should exist");
    let done_overlay = done_node.runtime_overlay.as_ref().expect("overlay present");
    assert!(
        !done_overlay.eligible,
        "Completed issue must not be eligible"
    );
    assert!(!done_overlay.queued, "Completed issue must not be queued");

    // Failed → neither eligible nor queued
    let failed_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-302")
        .expect("COE-302 node should exist");
    let failed_overlay = failed_node
        .runtime_overlay
        .as_ref()
        .expect("overlay present");
    assert!(
        !failed_overlay.eligible,
        "Failed issue must not be eligible"
    );
    assert!(!failed_overlay.queued, "Failed issue must not be queued");

    // Blocked Idle → NOT eligible AND NOT queued (blocked overrides Idle)
    let blocked_node = response
        .nodes
        .iter()
        .find(|n| n.identifier == "COE-304")
        .expect("COE-304 node should exist");
    let blocked_overlay = blocked_node
        .runtime_overlay
        .as_ref()
        .expect("overlay present");
    assert!(
        !blocked_overlay.eligible,
        "Blocked Idle issue must not be eligible"
    );
    assert!(
        !blocked_overlay.queued,
        "Blocked Idle issue must not be queued (blocked overrides)"
    );

    server_task.abort();
}

/// E2E evidence: POST /api/v1/actions/dispatch returns a receipt for a valid action
/// and a 400 for a rejected one.
#[tokio::test]
async fn gateway_dispatches_action_and_returns_receipt() {
    let store = SnapshotStore::new(fixture_snapshot(1));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let url = format!("http://{address}/api/v1/actions/dispatch");

    // Valid cancel action → accepted receipt
    let dispatch = ActionDispatch {
        schema_version: Default::default(),
        correlation_id: "corr_001".to_string(),
        action_kind: ActionKind::Cancel,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Issue,
            entity_id: "COE-255".to_string(),
        },
        payload: None,
        idempotency_key: Some("idempotency_001".to_string()),
    };
    let response = client
        .post(&url)
        .json(&dispatch)
        .send()
        .await
        .expect("POST /api/v1/actions/dispatch should respond");
    assert_eq!(response.status(), 200);
    let body: ActionReceipt = response.json().await.expect("should not be None");
    assert_eq!(body.status, ActionStatus::Accepted);
    assert_eq!(body.correlation_id, "corr_001");
    assert!(
        !body.action_id.is_empty(),
        "action_id should be non-empty: {:?}",
        body.action_id
    );

    // Duplicate idempotency key → rejected receipt
    let response = client
        .post(&url)
        .json(&dispatch)
        .send()
        .await
        .expect("POST /api/v1/actions/dispatch should respond");
    assert_eq!(response.status(), 409);
    let body: ActionReceipt = response.json().await.expect("should not be None");
    assert_eq!(body.status, ActionStatus::Rejected);
    assert!(
        body.reason
            .as_ref()
            .expect("should not be None")
            .contains("duplicate idempotency key"),
        "rejected reason should mention duplicate idempotency key: {:?}",
        body.reason
    );

    // Invalid retry action (already active) → rejected receipt
    let dispatch_retry = ActionDispatch {
        schema_version: Default::default(),
        correlation_id: "corr_002".to_string(),
        action_kind: ActionKind::Retry,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Issue,
            entity_id: "COE-255".to_string(),
        },
        payload: None,
        idempotency_key: None,
    };
    let response = client
        .post(&url)
        .json(&dispatch_retry)
        .send()
        .await
        .expect("POST /api/v1/actions/dispatch should respond");
    assert_eq!(response.status(), 422);
    let body: ActionReceipt = response.json().await.expect("should not be None");
    assert_eq!(body.status, ActionStatus::Rejected);
    assert!(
        body.reason
            .as_ref()
            .expect("should not be None")
            .contains("already active"),
        "rejected reason should mention already active: {:?}",
        body.reason
    );

    // Unknown issue → rejected receipt
    let dispatch_unknown = ActionDispatch {
        schema_version: Default::default(),
        correlation_id: "corr_003".to_string(),
        action_kind: ActionKind::Comment,
        target_entity: ActionTarget {
            entity_kind: EntityKind::Issue,
            entity_id: "COE-999".to_string(),
        },
        payload: None,
        idempotency_key: None,
    };
    let response = client
        .post(&url)
        .json(&dispatch_unknown)
        .send()
        .await
        .expect("POST /api/v1/actions/dispatch should respond");
    assert_eq!(response.status(), 404);
    let body: ActionReceipt = response.json().await.expect("should not be None");
    assert_eq!(body.status, ActionStatus::Rejected);

    server_task.abort();
}

/// E2E evidence: open_workspace and debug actions are accepted as dispatchable
/// action kinds and correlated to the target issue.
#[tokio::test]
async fn gateway_dispatches_open_workspace_and_debug_actions() {
    let store = SnapshotStore::new(fixture_snapshot(1));
    let server = GatewayServer::new(store.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let url = format!("http://{address}/api/v1/actions/dispatch");

    for (kind, correlation_id) in [
        (ActionKind::OpenWorkspace, "corr_open_workspace"),
        (ActionKind::Debug, "corr_debug"),
    ] {
        let dispatch = ActionDispatch {
            schema_version: Default::default(),
            correlation_id: correlation_id.to_string(),
            action_kind: kind,
            target_entity: ActionTarget {
                entity_kind: EntityKind::Issue,
                entity_id: "COE-255".to_string(),
            },
            payload: None,
            idempotency_key: None,
        };
        let response = client
            .post(&url)
            .json(&dispatch)
            .send()
            .await
            .expect("POST /api/v1/actions/dispatch should respond");
        assert_eq!(response.status(), 200, "{kind} should be accepted");
        let body: ActionReceipt = response.json().await.expect("should not be None");
        assert_eq!(body.status, ActionStatus::Accepted);
        assert_eq!(body.correlation_id, correlation_id);
    }

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_timeline_groups_runtime_events() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};
    use opensymphony::opensymphony_gateway_schema::event_journal::{EventKind, EventRecord};
    use opensymphony::opensymphony_gateway_schema::timeline::{RunTimeline, TimelineEntryKind};

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);

    let records = vec![
        EventRecord::builder()
            .event_id("evt-1")
            .sequence(1)
            .actor(
                opensymphony::opensymphony_gateway_schema::event_journal::EventActor::system(
                    "test",
                ),
            )
            .entity_ref(EntityRef {
                kind: EntityKind::Run,
                id: "run-1".into(),
                identifier: None,
            })
            .kind(EventKind::RunStarted)
            .summary("Run started")
            .build(),
        EventRecord::builder()
            .event_id("evt-2")
            .sequence(2)
            .actor(
                opensymphony::opensymphony_gateway_schema::event_journal::EventActor::system(
                    "test",
                ),
            )
            .entity_ref(EntityRef {
                kind: EntityKind::Run,
                id: "run-1".into(),
                identifier: None,
            })
            .kind(EventKind::HarnessConversationStateUpdate)
            .summary("waiting")
            .payload(serde_json::json!({ "execution_status": "waiting_for_prior_turn" }))
            .build(),
        EventRecord::builder()
            .event_id("evt-3")
            .sequence(3)
            .actor(
                opensymphony::opensymphony_gateway_schema::event_journal::EventActor::system(
                    "test",
                ),
            )
            .entity_ref(EntityRef {
                kind: EntityKind::Run,
                id: "run-1".into(),
                identifier: None,
            })
            .kind(EventKind::HarnessConversationStateUpdate)
            .summary("running")
            .payload(serde_json::json!({ "execution_status": "running" }))
            .build(),
        EventRecord::builder()
            .event_id("evt-4")
            .sequence(4)
            .actor(
                opensymphony::opensymphony_gateway_schema::event_journal::EventActor::system(
                    "test",
                ),
            )
            .entity_ref(EntityRef {
                kind: EntityKind::Run,
                id: "run-1".into(),
                identifier: None,
            })
            .kind(EventKind::HarnessToolCall)
            .summary("terminal tool")
            .payload(serde_json::json!({ "tool_name": "terminal" }))
            .build(),
        EventRecord::builder()
            .event_id("evt-5")
            .sequence(5)
            .actor(
                opensymphony::opensymphony_gateway_schema::event_journal::EventActor::system(
                    "test",
                ),
            )
            .entity_ref(EntityRef {
                kind: EntityKind::Run,
                id: "run-1".into(),
                identifier: None,
            })
            .kind(EventKind::RunCompleted)
            .summary("Run completed")
            .build(),
    ];
    for record in records {
        journal.append(record).await.expect("append");
    }

    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal, broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let client = reqwest::Client::new();
    let url = format!("http://{address}/api/v1/runs/run-1/timeline");
    let timeline: RunTimeline = client
        .get(&url)
        .send()
        .await
        .expect("fetch timeline")
        .json::<RunTimeline>()
        .await
        .expect("decode timeline");

    assert_eq!(timeline.run_id, "run-1");
    let kinds: Vec<_> = timeline.entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TimelineEntryKind::State,
            TimelineEntryKind::Progress,
            TimelineEntryKind::Progress,
            TimelineEntryKind::ToolCall,
            TimelineEntryKind::State,
        ]
    );
    assert!(
        timeline
            .entries
            .iter()
            .any(|e| e.title.to_lowercase().contains("waiting"))
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_terminal_log_associates_frames_and_reconnects() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventRecord,
    };
    use opensymphony::opensymphony_gateway_schema::terminal::{
        TerminalEncoding, TerminalFrame, TerminalFrameKind, TerminalLogAssociation,
        TerminalSnapshot,
    };
    use opensymphony::opensymphony_gateway_schema::version::SchemaVersion;

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal.clone(), broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    // Allow the router background task to start and subscribe.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let frame = TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: 1,
        stream_id: "term-1".into(),
        run_id: "run-1".into(),
        terminal_session_id: "term-1".into(),
        frame_kind: TerminalFrameKind::Stdout,
        encoding: TerminalEncoding::Utf8,
        content: "hello from command a".into(),
        timestamp: Utc::now(),
        association: TerminalLogAssociation {
            run_id: "run-1".into(),
            workspace_id: "ws-1".into(),
            command_id: Some("cmd-a".into()),
            issue_id: Some("iss-1".into()),
            sub_issue_id: Some("sub-1".into()),
            harness_session_id: Some("harness-1".into()),
        },
        correlation_id: None,
        source_event_id: Some("evt-1".into()),
        frame_id: Some("f1".into()),
    };
    let record = EventRecord::builder()
        .event_id("evt-1")
        .sequence(1)
        .actor(EventActor::system("test"))
        .entity_ref(EntityRef {
            kind: EntityKind::Run,
            id: "run-1".into(),
            identifier: None,
        })
        .kind(EventKind::TerminalFrame {
            frame_id: "f1".into(),
        })
        .summary("terminal frame")
        .payload(serde_json::to_value(&frame).expect("serialize frame"))
        .build();
    journal.append(record).await.expect("append");

    // Simulate reconnect with a second frame for the same session.
    let mut frame2 = frame.clone();
    frame2.frame_sequence = 2;
    frame2.content = "hello again after reconnect".into();
    frame2.source_event_id = Some("evt-2".into());
    let record2 = EventRecord::builder()
        .event_id("evt-2")
        .sequence(2)
        .actor(EventActor::system("test"))
        .entity_ref(EntityRef {
            kind: EntityKind::Run,
            id: "run-1".into(),
            identifier: None,
        })
        .kind(EventKind::TerminalFrame {
            frame_id: "f2".into(),
        })
        .summary("terminal frame after reconnect")
        .payload(serde_json::to_value(&frame2).expect("serialize frame"))
        .build();
    journal.append(record2).await.expect("append");

    // Give the background ingestion task time to catch up.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();
    let url = format!("http://{address}/api/v1/runs/run-1/terminal/term-1");
    let snapshot: TerminalSnapshot = client
        .get(&url)
        .send()
        .await
        .expect("fetch terminal snapshot")
        .json::<TerminalSnapshot>()
        .await
        .expect("decode snapshot");

    assert_eq!(snapshot.total_frames, 2);
    assert_eq!(snapshot.frames.len(), 2);
    let session = snapshot.session.expect("session present");
    assert_eq!(session.association.run_id, "run-1");
    assert_eq!(session.association.command_id.as_deref(), Some("cmd-a"));
    assert_eq!(session.association.issue_id.as_deref(), Some("iss-1"));
    assert_eq!(session.association.sub_issue_id.as_deref(), Some("sub-1"));

    // A request for a valid stream under a different run must not leak data.
    let wrong_url = format!("http://{address}/api/v1/runs/run-2/terminal/term-1");
    let resp = client
        .get(&wrong_url)
        .send()
        .await
        .expect("fetch wrong run snapshot");
    assert_eq!(resp.status(), 404);

    // Search should find the second frame.
    let url = format!("http://{address}/api/v1/runs/run-1/terminal/term-1/search?q=again");
    let result: opensymphony::opensymphony_gateway_schema::timeline::TerminalSearchResult = client
        .get(&url)
        .send()
        .await
        .expect("fetch search")
        .json()
        .await
        .expect("decode search result");
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].frame_sequence, 2);

    // Cross-run search should be rejected.
    let wrong_url = format!("http://{address}/api/v1/runs/run-2/terminal/term-1/search?q=again");
    let resp = client
        .get(&wrong_url)
        .send()
        .await
        .expect("fetch wrong run search");
    assert_eq!(resp.status(), 404);

    // Jump to event should resolve the first frame.
    let url = format!("http://{address}/api/v1/runs/run-1/terminal/term-1/jump?event_id=evt-1");
    let jump: opensymphony::opensymphony_gateway_schema::timeline::TerminalJumpResult = client
        .get(&url)
        .send()
        .await
        .expect("fetch jump")
        .json()
        .await
        .expect("decode jump result");
    assert!(jump.found);
    assert_eq!(jump.frame_sequence, Some(1));

    // Cross-run jump should be rejected.
    let wrong_url =
        format!("http://{address}/api/v1/runs/run-2/terminal/term-1/jump?event_id=evt-1");
    let resp = client
        .get(&wrong_url)
        .send()
        .await
        .expect("fetch wrong run jump");
    assert_eq!(resp.status(), 404);

    // Unknown stream should be rejected for snapshot, search, and jump.
    let unknown_url = format!("http://{address}/api/v1/runs/run-1/terminal/unknown/snapshot");
    let resp = client
        .get(&unknown_url)
        .send()
        .await
        .expect("fetch unknown snapshot");
    assert_eq!(resp.status(), 404);

    let unknown_url = format!("http://{address}/api/v1/runs/run-1/terminal/unknown/search?q=x");
    let resp = client
        .get(&unknown_url)
        .send()
        .await
        .expect("fetch unknown search");
    assert_eq!(resp.status(), 404);

    let unknown_url =
        format!("http://{address}/api/v1/runs/run-1/terminal/unknown/jump?event_id=evt-1");
    let resp = client
        .get(&unknown_url)
        .send()
        .await
        .expect("fetch unknown jump");
    assert_eq!(resp.status(), 404);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_logs_with_levels_and_pagination() {
    use opensymphony::opensymphony_domain::InMemoryEventJournal as DomainJournal;
    use opensymphony::opensymphony_gateway_schema::envelope::{EntityKind, EntityRef};
    use opensymphony::opensymphony_gateway_schema::event_journal::{
        EventActor, EventKind, EventRecord,
    };
    use opensymphony::opensymphony_gateway_schema::terminal::{
        TerminalEncoding, TerminalFrame, TerminalFrameKind, TerminalLogAssociation,
    };
    use opensymphony::opensymphony_gateway_schema::timeline::RunLogPage;
    use opensymphony::opensymphony_gateway_schema::version::SchemaVersion;

    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = DomainJournal::new(100, 64);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal.clone(), broker);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("test listener address");
    let server_task = tokio::spawn(async move {
        server
            .serve(listener)
            .await
            .expect("test gateway server should serve")
    });

    let association = TerminalLogAssociation {
        run_id: "run-1".into(),
        workspace_id: "ws-1".into(),
        command_id: Some("cmd-a".into()),
        issue_id: Some("iss-1".into()),
        sub_issue_id: Some("sub-1".into()),
        harness_session_id: Some("harness-1".into()),
    };

    async fn append(
        journal: opensymphony::opensymphony_domain::InMemoryEventJournal,
        sequence: u64,
        event_id: &str,
        kind: EventKind,
        summary: &str,
        payload: serde_json::Value,
    ) {
        let record = EventRecord::builder()
            .event_id(event_id)
            .sequence(sequence)
            .actor(EventActor::system("test"))
            .entity_ref(EntityRef {
                kind: EntityKind::Run,
                id: "run-1".into(),
                identifier: None,
            })
            .kind(kind)
            .summary(summary)
            .payload(payload)
            .build();
        journal.append(record).await.expect("append");
    }

    append(
        journal.clone(),
        1,
        "evt-log-1",
        EventKind::LogEntry {
            level: "info".into(),
        },
        "log line",
        serde_json::json!({
            "message": "info log line",
            "terminal_session_id": "term-1",
            "command_id": "cmd-a",
        }),
    )
    .await;

    let stdout_frame = TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: 2,
        stream_id: "term-1".into(),
        run_id: "run-1".into(),
        terminal_session_id: "term-1".into(),
        frame_kind: TerminalFrameKind::Stdout,
        encoding: TerminalEncoding::Utf8,
        content: "stdout line".into(),
        timestamp: Utc::now(),
        association: association.clone(),
        correlation_id: None,
        source_event_id: Some("evt-stdout-1".into()),
        frame_id: Some("f-stdout".into()),
    };
    append(
        journal.clone(),
        2,
        "evt-stdout-1",
        EventKind::TerminalFrame {
            frame_id: "f-stdout".into(),
        },
        "stdout frame",
        serde_json::to_value(&stdout_frame).expect("serialize stdout frame"),
    )
    .await;

    let stderr_frame = TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: 3,
        stream_id: "term-1".into(),
        run_id: "run-1".into(),
        terminal_session_id: "term-1".into(),
        frame_kind: TerminalFrameKind::Stderr,
        encoding: TerminalEncoding::Utf8,
        content: "stderr line".into(),
        timestamp: Utc::now(),
        association: association.clone(),
        correlation_id: None,
        source_event_id: Some("evt-stderr-1".into()),
        frame_id: Some("f-stderr".into()),
    };
    append(
        journal.clone(),
        3,
        "evt-stderr-1",
        EventKind::TerminalFrame {
            frame_id: "f-stderr".into(),
        },
        "stderr frame",
        serde_json::to_value(&stderr_frame).expect("serialize stderr frame"),
    )
    .await;

    let log_frame = TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: 4,
        stream_id: "term-1".into(),
        run_id: "run-1".into(),
        terminal_session_id: "term-1".into(),
        frame_kind: TerminalFrameKind::Log,
        encoding: TerminalEncoding::Utf8,
        content: "frame log line".into(),
        timestamp: Utc::now(),
        association,
        correlation_id: None,
        source_event_id: Some("evt-log-frame-1".into()),
        frame_id: Some("f-log".into()),
    };
    append(
        journal.clone(),
        4,
        "evt-log-frame-1",
        EventKind::TerminalFrame {
            frame_id: "f-log".into(),
        },
        "log frame",
        serde_json::to_value(&log_frame).expect("serialize log frame"),
    )
    .await;

    let client = reqwest::Client::new();
    let url = format!("http://{address}/api/v1/runs/run-1/logs?cursor=0&limit=2");
    let page: RunLogPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch run logs page 1")
        .json()
        .await
        .expect("decode run log page");

    assert_eq!(page.run_id, "run-1");
    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.next_cursor, Some(3));
    assert_eq!(page.entries[0].sequence, 1);
    assert_eq!(page.entries[0].level, "info");
    assert_eq!(page.entries[0].message, "info log line");
    assert_eq!(page.entries[1].sequence, 2);
    assert_eq!(page.entries[1].level, "stdout");
    assert_eq!(page.entries[1].message, "stdout line");

    let url = format!("http://{address}/api/v1/runs/run-1/logs?cursor=3&limit=2");
    let page: RunLogPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch run logs page 2")
        .json()
        .await
        .expect("decode run log page");

    assert_eq!(page.entries.len(), 2);
    assert_eq!(page.entries[0].sequence, 3);
    assert_eq!(page.entries[0].level, "stderr");
    assert_eq!(page.entries[0].message, "stderr line");
    assert_eq!(page.entries[1].sequence, 4);
    assert_eq!(page.entries[1].level, "log");
    assert_eq!(page.entries[1].message, "frame log line");
    assert_eq!(page.next_cursor, Some(5));

    // A subsequent request with the next cursor returns an empty page, signaling
    // the end of the log stream.
    let url = format!("http://{address}/api/v1/runs/run-1/logs?cursor=5&limit=2");
    let page: RunLogPage = client
        .get(&url)
        .send()
        .await
        .expect("fetch run logs tail page")
        .json()
        .await
        .expect("decode run log tail page");
    assert!(page.entries.is_empty());
    assert!(page.next_cursor.is_none());

    server_task.abort();
}
