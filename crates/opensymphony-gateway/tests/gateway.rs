use async_trait::async_trait;
use chrono::Utc;
use duckdb::Connection as DuckDbConnection;
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
    ReleaseReason as DomainReleaseReason, SnapshotEnvelope, TrackerIssue, TrackerIssueBlocker,
    TrackerIssueRef, TrackerIssueState, TrackerIssueStateKind,
};
use opensymphony::opensymphony_gateway::{
    GatewayCapabilities, GatewayServer, LinearTaskGraphClient, control_plane_to_dashboard_snapshot,
    model_settings_for_llm_api_key, model_settings_for_llm_api_key_and_codex_readiness,
};
use opensymphony::opensymphony_gateway_schema::action::{
    ActionDispatch, ActionKind, ActionReceipt, ActionStatus, ActionTarget,
};
use opensymphony::opensymphony_gateway_schema::code_graph::{
    CodeDiffEdgeStatus, CodeDiffOverlay, CodeFileOutline, CodeGraphFreshness, CodeGraphNodeKind,
    CodeGraphSnapshot, CodeIndexReport, CodeIndexStatus, CodeRepoList, CodeSymbolDetail,
};
use opensymphony::opensymphony_gateway_schema::envelope::EntityKind;
use opensymphony::opensymphony_gateway_schema::memory_graph::{
    MemoryBundleList, MemoryCommunityList, MemoryCompletedTaskPage, MemoryConceptDetail,
    MemoryGraphEdgeKind, MemoryGraphSnapshot, MemorySearchResponse,
};
use opensymphony::opensymphony_gateway_schema::model_settings::{
    CodexCliProbe, CodexLocalReadiness, CredentialStatusKind, CredentialStatusResponse,
    ModelSettingsResponse, ProbeCommandResult,
};
use opensymphony::opensymphony_gateway_schema::run::DiffLine;
use opensymphony::opensymphony_gateway_schema::validation::ValidationStatus;
use opensymphony::opensymphony_memory::{
    CodeGraphContextQuery, CodeIntelDiagnosticInput, CodeIntelDocumentInput, CodeIntelEdgeInput,
    CodeIntelPersistBatch, CodeIntelSkippedFileInput, CodeIntelSymbolInput, MemoryConfig,
    code_graph_context, code_graph_workspace_context_overlay, code_graph_workspace_diff_overlay,
    persist_code_intel_documents, persist_code_intel_skipped_files, refresh_memory_index_from_okf,
};
use tokio::net::TcpListener;
use url::Url;

#[derive(Clone)]
struct FakeLinearTaskGraphClient {
    issues: Vec<TrackerIssue>,
}

#[derive(Clone)]
struct StrictLinearTaskGraphClient {
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

/// Fake that mirrors the real client's task-graph contract: requested
/// identifiers plus every unrequested backlog- or active-state issue from
/// the project scan.
#[derive(Clone)]
struct BacklogLinearTaskGraphClient {
    issues: Vec<TrackerIssue>,
    unrequested: Vec<TrackerIssue>,
}

#[async_trait]
impl LinearTaskGraphClient for BacklogLinearTaskGraphClient {
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

    async fn task_graph_issues(&self, identifiers: &[String]) -> Result<Vec<TrackerIssue>, String> {
        let mut issues = self.issues_by_identifiers(identifiers).await?;
        issues.extend(self.unrequested.iter().cloned());
        Ok(issues)
    }
}

#[async_trait]
impl LinearTaskGraphClient for StrictLinearTaskGraphClient {
    async fn issues_by_identifiers(
        &self,
        identifiers: &[String],
    ) -> Result<Vec<TrackerIssue>, String> {
        let issues = identifiers
            .iter()
            .filter_map(|identifier| {
                self.issues
                    .iter()
                    .find(|issue| issue.identifier == *identifier)
                    .cloned()
            })
            .collect::<Vec<_>>();
        if issues.len() == identifiers.len() {
            Ok(issues)
        } else {
            Err("missing requested issue".to_owned())
        }
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
        branch_name: issue.branch_name.clone(),
        pr_url: issue.pr_url.clone(),
        labels: Vec::new(),
        project_id: issue.project_id.clone(),
        project_slug: issue.project_slug.clone(),
        project_name: issue.project_name.clone(),
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

fn write_memory_graph_fixture(repo: &std::path::Path) -> MemoryConfig {
    let config_path = repo.join("opensymphony-memory.yaml");
    std::fs::write(
        &config_path,
        r#"
areas:
  graph-view:
    title: Graph View
    docs_target: docs/graph-view.md
    status: stable
    confidence: 90
"#,
    )
    .expect("memory config should write");
    let memory_root = repo.join(".opensymphony/memory");
    let issues_dir = memory_root.join("issues");
    std::fs::create_dir_all(&issues_dir).expect("memory issues dir should write");
    std::fs::write(
        issues_dir.join("COE-200.md"),
        format!(
            r#"---
type: topic-doc
title: "COE-200: Public graph concept"
description: Public graph DTO fixture.
resource: https://linear.app/example/issue/COE-200
tags: [memory, graph]
timestamp: 2026-06-22T10:00:00Z
custom_unknown: keep-me
auth_token: gateway-secret-fixture
opensymphony:
  visibility: public
  scope_refs:
    - kind: work_item
      id: COE-200
    - kind: area
      id: graph-view
  source_refs:
    - kind: linear_issue
      id: COE-200
      url: https://linear.app/example/issue/COE-200
  citations:
    - id: "1"
      target: https://linear.app/example/issue/COE-200
      label: COE-200
---

# COE-200: Public graph concept

Public graph body mentions .opensymphony/memory/issues/COE-999.md and {}.

See [external](https://example.com/reference).
"#,
            repo.display()
        ),
    )
    .expect("public concept should write");
    std::fs::write(
        issues_dir.join("COE-201.md"),
        r#"---
type: issue-capsule
title: "COE-201: Private graph concept"
description: Private graph DTO fixture.
tags: [memory, graph]
opensymphony:
  visibility: private
  scope_refs:
    - kind: work_item
      id: COE-201
    - kind: area
      id: graph-view
---

# COE-201: Private graph concept

Private graph body.
"#,
    )
    .expect("private concept should write");
    MemoryConfig::load(repo, Some(&config_path)).expect("memory config should load")
}

fn write_code_graph_fixture(repo: &std::path::Path) -> MemoryConfig {
    write_code_graph_fixture_with_revisions(repo, "base-rev", "head-rev")
}

fn write_code_graph_fixture_with_revisions(
    repo: &std::path::Path,
    base_revision: &str,
    head_revision: &str,
) -> MemoryConfig {
    let config = MemoryConfig::load(repo, None).expect("memory config should load");
    persist_code_intel_documents(
        &config,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some(base_revision.to_string()),
            worktree_dirty: false,
            documents: vec![
                code_graph_document(
                    "base-content",
                    vec![
                        code_graph_symbol(
                            "struct",
                            "App",
                            &[],
                            "struct App",
                            (1, 0, 4),
                            "app-base",
                        ),
                        code_graph_symbol(
                            "function",
                            "run",
                            &["App"],
                            "fn run(&self)",
                            (6, 2, 20),
                            "run-base",
                        ),
                        code_graph_symbol(
                            "function",
                            "legacy",
                            &["App"],
                            "fn legacy(&self)",
                            (22, 2, 36),
                            "legacy-base",
                        ),
                        code_graph_symbol(
                            "function",
                            "helper",
                            &["App"],
                            "fn helper(&self)",
                            (90, 2, 96),
                            "helper-shared",
                        ),
                    ],
                    vec![
                        CodeIntelEdgeInput {
                            edge_kind: "reference.call".to_string(),
                            target_hint: Some("legacy".to_string()),
                            confidence: "query_pack:calls".to_string(),
                            start_line: 6,
                            start_col: 8,
                            end_line: 6,
                            end_col: 18,
                            start_byte: 8,
                            end_byte: 16,
                        },
                        CodeIntelEdgeInput {
                            edge_kind: "reference.call".to_string(),
                            target_hint: Some("legacy".to_string()),
                            confidence: "query_pack:calls".to_string(),
                            start_line: 90,
                            start_col: 8,
                            end_line: 90,
                            end_col: 18,
                            start_byte: 80,
                            end_byte: 88,
                        },
                    ],
                    Vec::new(),
                ),
                code_graph_diagnostic_document(),
                code_graph_document_with_path(
                    "src/empty.rs",
                    "empty-base",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                code_graph_document_with_path(
                    "src/deleted_empty.rs",
                    "deleted-empty-base",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                code_graph_document_with_path(
                    "src/unchanged_empty.rs",
                    "unchanged-empty",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
            ],
        },
    )
    .expect("base code graph fixture should persist");
    persist_code_intel_documents(
        &config,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some(head_revision.to_string()),
            worktree_dirty: false,
            documents: vec![
                code_graph_head_document(),
                code_graph_diagnostic_document(),
                code_graph_document_with_path(
                    "src/empty.rs",
                    "empty-content",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                code_graph_document_with_path(
                    "src/added_empty.rs",
                    "added-empty-head",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
                code_graph_document_with_path(
                    "src/unchanged_empty.rs",
                    "unchanged-empty",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ),
            ],
        },
    )
    .expect("head code graph fixture should persist");
    config
}

fn code_graph_head_document() -> CodeIntelDocumentInput {
    code_graph_document(
        "head-content",
        vec![
            code_graph_symbol("struct", "App", &[], "struct App", (1, 0, 4), "app-base"),
            code_graph_symbol(
                "function",
                "run",
                &["App"],
                "fn run(&self) -> Result<()>",
                (60, 2, 80),
                "run-head",
            ),
            code_graph_symbol(
                "function",
                "new_feature",
                &["App"],
                "fn new_feature(&self)",
                (38, 2, 54),
                "new-feature-head",
            ),
            code_graph_symbol(
                "function",
                "helper",
                &["App"],
                "fn helper(&self)",
                (90, 2, 96),
                "helper-shared",
            ),
        ],
        vec![
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("new_feature".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 60,
                start_col: 6,
                end_line: 60,
                end_col: 18,
                start_byte: 8,
                end_byte: 16,
            },
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("run".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 38,
                start_col: 8,
                end_line: 38,
                end_col: 18,
                start_byte: 40,
                end_byte: 48,
            },
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("run".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 90,
                start_col: 8,
                end_line: 90,
                end_col: 18,
                start_byte: 80,
                end_byte: 88,
            },
            CodeIntelEdgeInput {
                edge_kind: "reference.call".to_string(),
                target_hint: Some("missing_call".to_string()),
                confidence: "query_pack:calls".to_string(),
                start_line: 60,
                start_col: 20,
                end_line: 60,
                end_col: 32,
                start_byte: 20,
                end_byte: 32,
            },
        ],
        vec![
            CodeIntelDiagnosticInput {
                kind: "warning".to_string(),
                severity: "warning".to_string(),
                message: "fixture diagnostic".to_string(),
                start_line: 60,
                start_col: 2,
                end_line: 60,
                end_col: 20,
                start_byte: 8,
                end_byte: 16,
            },
            CodeIntelDiagnosticInput {
                kind: "info".to_string(),
                severity: "info".to_string(),
                message: "secondary fixture diagnostic".to_string(),
                start_line: 60,
                start_col: 4,
                end_line: 60,
                end_col: 18,
                start_byte: 10,
                end_byte: 14,
            },
        ],
    )
}

fn code_graph_diagnostic_document() -> CodeIntelDocumentInput {
    code_graph_document_with_path(
        "src/diag.rs",
        "diag-content",
        vec![code_graph_symbol(
            "function",
            "diagnosed",
            &[],
            "fn diagnosed()",
            (1, 0, 22),
            "diag-symbol",
        )],
        Vec::new(),
        vec![CodeIntelDiagnosticInput {
            kind: "warning".to_string(),
            severity: "warning".to_string(),
            message: "diagnostic fixture".to_string(),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 22,
            start_byte: 0,
            end_byte: 22,
        }],
    )
}

fn code_graph_document(
    content_sha256: &str,
    symbols: Vec<CodeIntelSymbolInput>,
    edges: Vec<CodeIntelEdgeInput>,
    diagnostics: Vec<CodeIntelDiagnosticInput>,
) -> CodeIntelDocumentInput {
    code_graph_document_with_path("src/lib.rs", content_sha256, symbols, edges, diagnostics)
}

fn code_graph_document_with_path(
    path: &str,
    content_sha256: &str,
    symbols: Vec<CodeIntelSymbolInput>,
    edges: Vec<CodeIntelEdgeInput>,
    diagnostics: Vec<CodeIntelDiagnosticInput>,
) -> CodeIntelDocumentInput {
    CodeIntelDocumentInput {
        path: path.into(),
        language: "rust".to_string(),
        content_sha256: content_sha256.to_string(),
        parser_id: "tree-sitter".to_string(),
        parser_version: "tree-sitter-rust-vfixture".to_string(),
        query_pack_version: "rust-query-pack-vfixture".to_string(),
        byte_len: 96,
        line_count: 8,
        symbols,
        edges,
        diagnostics,
    }
}

fn code_graph_symbol(
    kind: &str,
    name: &str,
    container_chain: &[&str],
    signature: &str,
    span: (usize, usize, usize),
    snippet_sha256: &str,
) -> CodeIntelSymbolInput {
    let (start_line, start_col, end_byte) = span;
    CodeIntelSymbolInput {
        kind: kind.to_string(),
        name: name.to_string(),
        container_chain: container_chain
            .iter()
            .map(|value| value.to_string())
            .collect(),
        signature: Some(signature.to_string()),
        start_line,
        start_col,
        end_line: start_line + 1,
        end_col: start_col + 12,
        start_byte: start_col,
        end_byte,
        selection_start_line: start_line,
        selection_end_line: start_line,
        snippet_sha256: snippet_sha256.to_string(),
    }
}

fn run_git(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .current_dir(workspace)
        .args(args)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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
            codex_thread_id: None,
            workspace_path_suffix: "COE-255".to_owned(),
            branch_name: Some("feat/coe-255-observability".to_owned()),
            pr_url: Some("https://github.com/kumanday/OpenSymphony/pull/255".to_owned()),
            project_id: Some("proj-open".to_owned()),
            project_slug: Some("opensymphony-bootstrap".to_owned()),
            project_name: Some("OpenSymphony".to_owned()),
            workspace_label: Some("COE-255".to_owned()),
            retry_count: 0,
            release_reason: None,
            claimed_at: Some(now - chrono::Duration::seconds(80)),
            started_at: Some(now - chrono::Duration::seconds(75)),
            finished_at: None,
            turn_count: 3,
            max_turns: 8,
            runtime_seconds: 75,
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
            total_tokens: 0,
            cancel_requested: false,
            cancel_acknowledged: false,
            cancel_failed: false,
            cancel_timed_out: false,
            cancel_reason: None,
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
                codex_thread_id: None,
                workspace_path_suffix: String::new(),
                branch_name: None,
                pr_url: None,
                project_id: Some("proj-alpha".to_owned()),
                project_slug: Some("alpha-project".to_owned()),
                project_name: Some("Alpha Project".to_owned()),
                workspace_label: None,
                retry_count: 0,
                release_reason: None,
                claimed_at: None,
                started_at: None,
                finished_at: None,
                turn_count: 0,
                max_turns: 0,
                runtime_seconds: 0,
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
                total_tokens: 0,
                cancel_requested: false,
                cancel_acknowledged: false,
                cancel_failed: false,
                cancel_timed_out: false,
                cancel_reason: None,
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
                codex_thread_id: None,
                workspace_path_suffix: "COE-301".to_owned(),
                branch_name: None,
                pr_url: None,
                project_id: Some("proj-beta".to_owned()),
                project_slug: Some("beta-project".to_owned()),
                project_name: Some("Beta Project".to_owned()),
                workspace_label: Some("COE-301".to_owned()),
                retry_count: 0,
                release_reason: None,
                claimed_at: Some(now - chrono::Duration::seconds(90)),
                started_at: Some(now - chrono::Duration::seconds(80)),
                finished_at: Some(now - chrono::Duration::seconds(10)),
                turn_count: 2,
                max_turns: 0,
                runtime_seconds: 70,
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
                total_tokens: 0,
                cancel_requested: false,
                cancel_acknowledged: false,
                cancel_failed: false,
                cancel_timed_out: false,
                cancel_reason: None,
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
                codex_thread_id: None,
                workspace_path_suffix: "COE-302".to_owned(),
                branch_name: None,
                pr_url: None,
                project_id: None,
                project_slug: None,
                project_name: None,
                workspace_label: Some("COE-302".to_owned()),
                retry_count: 0,
                release_reason: None,
                claimed_at: Some(now - chrono::Duration::seconds(30)),
                started_at: Some(now - chrono::Duration::seconds(25)),
                finished_at: Some(now - chrono::Duration::seconds(5)),
                turn_count: 1,
                max_turns: 0,
                runtime_seconds: 20,
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
                total_tokens: 0,
                cancel_requested: false,
                cancel_acknowledged: false,
                cancel_failed: false,
                cancel_timed_out: false,
                cancel_reason: None,
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
                codex_thread_id: None,
                workspace_path_suffix: "COE-303".to_owned(),
                branch_name: None,
                pr_url: None,
                project_id: Some("proj-beta".to_owned()),
                project_slug: Some("beta-project".to_owned()),
                project_name: Some("Beta Project".to_owned()),
                workspace_label: Some("COE-303".to_owned()),
                retry_count: 1,
                release_reason: None,
                claimed_at: None,
                started_at: None,
                finished_at: None,
                turn_count: 0,
                max_turns: 0,
                runtime_seconds: 0,
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
                total_tokens: 0,
                cancel_requested: false,
                cancel_acknowledged: false,
                cancel_failed: false,
                cancel_timed_out: false,
                cancel_reason: None,
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
                codex_thread_id: None,
                workspace_path_suffix: String::new(),
                branch_name: None,
                pr_url: None,
                project_id: Some("proj-alpha".to_owned()),
                project_slug: Some("alpha-project".to_owned()),
                project_name: Some("Alpha Project".to_owned()),
                workspace_label: None,
                retry_count: 0,
                release_reason: None,
                claimed_at: None,
                started_at: None,
                finished_at: None,
                turn_count: 0,
                max_turns: 0,
                runtime_seconds: 0,
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
                total_tokens: 0,
                cancel_requested: false,
                cancel_acknowledged: false,
                cancel_failed: false,
                cancel_timed_out: false,
                cancel_reason: None,
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
async fn gateway_serves_memory_graph_contract_endpoints() {
    let repo = tempfile::tempdir().expect("memory repo");
    let config = write_memory_graph_fixture(repo.path());
    refresh_memory_index_from_okf(&config, &repo.path().join(".opensymphony/memory"))
        .expect("fixture should reindex");

    let store = SnapshotStore::new(fixture_snapshot(0));
    let server = GatewayServer::new(store).with_memory_config(Some(config));
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
    let base = format!("http://{address}/api/v1/memory");

    let bundles = client
        .get(format!("{base}/bundles?visibility=public"))
        .send()
        .await
        .expect("fetch bundles")
        .json::<MemoryBundleList>()
        .await
        .expect("decode bundles");
    assert_eq!(bundles.schema_version.major, 1);
    assert_eq!(bundles.bundles[0].id, "local-default");
    assert_eq!(bundles.bundles[0].concept_count, 1);

    let all_bundles = client
        .get(format!("{base}/bundles"))
        .send()
        .await
        .expect("fetch default bundles")
        .json::<MemoryBundleList>()
        .await
        .expect("decode default bundles");
    assert_eq!(all_bundles.bundles[0].concept_count, 2);

    let graph = client
        .get(format!(
            "{base}/bundles/local-default/graph?visibility=public"
        ))
        .send()
        .await
        .expect("fetch graph")
        .json::<MemoryGraphSnapshot>()
        .await
        .expect("decode graph");
    let graph_json = serde_json::to_string(&graph).expect("graph serializes");
    assert!(graph_json.contains("COE-200"));
    assert!(!graph_json.contains("COE-201"));
    assert!(!graph_json.contains("gateway-secret-fixture"));
    assert!(graph_json.contains("[redacted-secret]"));
    assert!(!graph_json.contains(".opensymphony/memory"));
    assert!(!graph_json.contains(&repo.path().display().to_string()));
    assert!(
        graph
            .edges
            .iter()
            .any(|edge| edge.kind == MemoryGraphEdgeKind::ExternalLink)
    );
    assert_eq!(graph.metrics.orphan_count, 0);
    let graph_with_tags = client
        .get(format!(
            "{base}/bundles/local-default/graph?visibility=public&include_tags=true"
        ))
        .send()
        .await
        .expect("fetch graph with tags")
        .json::<MemoryGraphSnapshot>()
        .await
        .expect("decode graph with tags");
    assert!(
        graph_with_tags
            .filters_applied
            .contains(&"communities:include_tags".to_string())
    );
    assert!(graph_with_tags.communities.iter().any(|community| {
        community.id == "area:graph-view"
            && community
                .node_ids
                .iter()
                .any(|node_id| node_id == "tag:graph")
    }));

    let all_graph = client
        .get(format!("{base}/bundles/local-default/graph"))
        .send()
        .await
        .expect("fetch default graph")
        .json::<MemoryGraphSnapshot>()
        .await
        .expect("decode default graph");
    let all_graph_json = serde_json::to_string(&all_graph).expect("default graph serializes");
    assert!(all_graph_json.contains("COE-200"));
    assert!(all_graph_json.contains("COE-201"));
    assert!(!all_graph_json.contains(".opensymphony/memory"));
    assert!(!all_graph_json.contains(&repo.path().display().to_string()));

    let detail = client
        .get(format!(
            "{base}/bundles/local-default/concepts/issues/COE-200?visibility=public"
        ))
        .send()
        .await
        .expect("fetch concept")
        .json::<MemoryConceptDetail>()
        .await
        .expect("decode concept");
    assert_eq!(detail.concept_id, "issues/COE-200");
    assert!(
        detail
            .frontmatter_view
            .opensymphony
            .contains_key("scope_refs")
    );
    assert!(
        detail
            .frontmatter_view
            .unknown
            .contains_key("custom_unknown")
    );
    assert_eq!(
        detail.frontmatter_view.unknown.get("auth_token"),
        Some(&serde_json::json!("[redacted-secret]"))
    );
    assert!(!detail.body_markdown.contains(".opensymphony/memory"));
    assert!(
        !detail
            .body_markdown
            .contains(&repo.path().display().to_string())
    );

    let communities = client
        .get(format!(
            "{base}/bundles/local-default/communities?visibility=public"
        ))
        .send()
        .await
        .expect("fetch communities")
        .json::<MemoryCommunityList>()
        .await
        .expect("decode communities");
    assert_eq!(communities.bundle_id, "local-default");
    assert!(
        communities
            .communities
            .iter()
            .any(|community| { community.id == "area:graph-view" && community.concept_count == 1 })
    );
    let communities_with_tags = client
        .get(format!(
            "{base}/bundles/local-default/communities?visibility=public&include_tags=true"
        ))
        .send()
        .await
        .expect("fetch communities with tags")
        .json::<MemoryCommunityList>()
        .await
        .expect("decode communities with tags");
    assert!(communities_with_tags.communities.iter().any(|community| {
        community.id == "area:graph-view"
            && community
                .node_ids
                .iter()
                .any(|node_id| node_id == "tag:graph")
    }));

    let search = client
        .get(format!(
            "{base}/search?query=public%20graph&visibility=public"
        ))
        .send()
        .await
        .expect("fetch search")
        .json::<MemorySearchResponse>()
        .await
        .expect("decode search");
    assert_eq!(search.results.len(), 1);
    assert_eq!(search.results[0].concept_id, "issues/COE-200");

    let invalid_visibility = client
        .get(format!("{base}/bundles?visibility=private"))
        .send()
        .await
        .expect("fetch invalid visibility");
    assert_eq!(
        invalid_visibility.status(),
        reqwest::StatusCode::BAD_REQUEST
    );
    let invalid_visibility_body = invalid_visibility
        .json::<serde_json::Value>()
        .await
        .expect("decode invalid visibility response");
    assert_eq!(
        invalid_visibility_body.pointer("/error/code"),
        Some(&serde_json::json!("invalid_visibility"))
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_index_starts_target_branch_job_and_journals_completion() {
    let repo = tempfile::tempdir().expect("target repository");
    std::fs::create_dir_all(repo.path().join("src")).expect("source directory");
    std::fs::write(
        repo.path().join("WORKFLOW.md"),
        "## Branch target\n\nTarget branch: `develop`\n",
    )
    .expect("workflow marker");
    std::fs::write(repo.path().join("src/lib.rs"), "pub fn main_branch() {}\n")
        .expect("main source");
    run_git(repo.path(), &["init", "-b", "main"]);
    run_git(repo.path(), &["config", "user.email", "test@example.com"]);
    run_git(repo.path(), &["config", "user.name", "Test User"]);
    run_git(repo.path(), &["add", "."]);
    run_git(repo.path(), &["commit", "-m", "main"]);
    run_git(repo.path(), &["switch", "-c", "develop"]);
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn develop_branch() { helper(); }\nfn helper() {}\n",
    )
    .expect("develop source");
    run_git(repo.path(), &["add", "."]);
    run_git(repo.path(), &["commit", "-m", "develop"]);

    let config = MemoryConfig::load(repo.path(), None).expect("memory config");
    let server = GatewayServer::new(SnapshotStore::new(fixture_snapshot(0)))
        .with_memory_config(Some(config));
    let (journal, _) = server.clone().journal_and_broker();
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener");
    let address = listener.local_addr().expect("listener address");
    let server_task = tokio::spawn(async move {
        server.serve(listener).await.expect("gateway should serve");
    });

    let report = reqwest::Client::new()
        .post(format!(
            "http://{address}/api/v1/code/repos/target-repo/index"
        ))
        .send()
        .await
        .expect("index request")
        .json::<CodeIndexReport>()
        .await
        .expect("accepted report");
    assert_eq!(report.status, CodeIndexStatus::Accepted);

    let mut completed = false;
    // Target-branch indexing runs in a blocking worker; allow slower CI
    // runners to finish the same deterministic completion assertion.
    for _ in 0..600 {
        let events = journal.all_events().await;
        completed = events.iter().any(|event| {
            matches!(
                event.kind,
                opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeGraphUpdated { .. }
            )
        });
        if completed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(completed, "index completion should be journaled");
    assert!(journal.all_events().await.iter().any(|event| {
        matches!(
            event.kind,
            opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeIndexProgress { .. }
        )
    }));
    let events = journal.all_events().await;
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeIndexProgress { .. }
        ) && event
            .payload
            .as_ref()
            .and_then(|payload| payload.get("status"))
            == Some(&serde_json::json!("progress"))
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeIndexProgress { .. }
        ) && event
            .payload
            .as_ref()
            .and_then(|payload| payload.get("status"))
            == Some(&serde_json::json!("completed"))
    }));
    server_task.abort();
}

#[tokio::test]
async fn gateway_code_graph_bootstraps_empty_store_and_dirty_workspace_flow() {
    let repo = tempfile::tempdir().expect("target repository");
    std::fs::create_dir_all(repo.path().join("src/services")).expect("source directories");
    std::fs::write(
        repo.path().join("WORKFLOW.md"),
        "## Branch target\n\nTarget branch: `develop`\n",
    )
    .expect("workflow marker");
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub mod app;\npub mod services { pub mod shared; }\n",
    )
    .expect("module root");
    std::fs::write(repo.path().join("src/app.rs"), "pub fn run() {}\n")
        .expect("application source");
    std::fs::write(
        repo.path().join("src/services/shared.rs"),
        "pub fn helper() {}\n",
    )
    .expect("shared source");
    run_git(repo.path(), &["init", "-b", "develop"]);
    run_git(repo.path(), &["config", "user.email", "test@example.com"]);
    run_git(repo.path(), &["config", "user.name", "Test User"]);
    run_git(repo.path(), &["add", "."]);
    run_git(repo.path(), &["commit", "-m", "code graph baseline"]);
    let base_revision = run_git(repo.path(), &["rev-parse", "HEAD"]);
    let config = MemoryConfig::load(repo.path(), None).expect("memory config");
    let repo_id = repo
        .path()
        .file_name()
        .expect("repository id")
        .to_string_lossy()
        .to_string();

    let journal = opensymphony::opensymphony_domain::InMemoryEventJournal::new(100, 32);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(
        SnapshotStore::new(fixture_snapshot(0)),
        journal.clone(),
        broker,
    )
    .with_memory_config(Some(config.clone()));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test listener");
    let address = listener.local_addr().expect("listener address");
    let server_task = tokio::spawn(async move {
        server.serve(listener).await.expect("gateway should serve");
    });
    let client = reqwest::Client::new();
    let base = format!("http://{address}/api/v1/code");

    let empty_repos = client
        .get(format!("{base}/repos"))
        .send()
        .await
        .expect("fetch empty code repos")
        .json::<CodeRepoList>()
        .await
        .expect("decode empty code repos");
    let empty_repo = empty_repos
        .repos
        .iter()
        .find(|summary| summary.repo_id == repo_id)
        .expect("configured repository should be discoverable before indexing");
    assert_eq!(empty_repo.document_count, 0);
    assert!(!empty_repo.indexed);

    let accepted = client
        .post(format!("{base}/repos/{repo_id}/index"))
        .send()
        .await
        .expect("index empty code repo")
        .json::<CodeIndexReport>()
        .await
        .expect("decode accepted index report");
    assert_eq!(accepted.status, CodeIndexStatus::Accepted);
    assert_eq!(accepted.repo_id, repo_id);

    let mut completed = false;
    for _ in 0..200 {
        let events = journal.all_events().await;
        completed = events.iter().any(|event| {
            matches!(
                event.kind,
                opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeGraphUpdated { .. }
            )
        });
        if completed {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(completed, "index completion should refresh the code graph");
    let events = journal.all_events().await;
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeIndexProgress { .. }
        ) && event
            .payload
            .as_ref()
            .and_then(|payload| payload.get("status"))
            == Some(&serde_json::json!("progress"))
    }));
    assert!(events.iter().any(|event| {
        matches!(
            event.kind,
            opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeIndexProgress { .. }
        ) && event
            .payload
            .as_ref()
            .and_then(|payload| payload.get("status"))
            == Some(&serde_json::json!("completed"))
    }));

    let indexed_repos = client
        .get(format!("{base}/repos"))
        .send()
        .await
        .expect("refresh indexed code repos")
        .json::<CodeRepoList>()
        .await
        .expect("decode indexed code repos");
    let indexed_repo = indexed_repos
        .repos
        .iter()
        .find(|summary| summary.repo_id == repo_id)
        .expect("indexed repository should remain discoverable");
    assert_eq!(
        indexed_repo.head_revision.as_deref(),
        Some(base_revision.as_str())
    );
    assert!(indexed_repo.indexed);
    assert!(indexed_repo.document_count > 0);

    let snapshot = client
        .get(format!("{base}/repos/{repo_id}/graph?mode=atlas"))
        .send()
        .await
        .expect("fetch indexed graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode indexed graph");
    assert!(
        !snapshot.nodes.is_empty(),
        "indexed graph should be nonempty"
    );

    let context_query = CodeGraphContextQuery {
        repo_id: repo_id.clone(),
        query: Some("run".to_string()),
        path: None,
        symbol: None,
        depth: 1,
        limit: 20,
    };
    let baseline_context = code_graph_context(&config, context_query.clone(), None)
        .expect("indexed baseline should be available");
    assert_eq!(
        baseline_context.pointer("/provenance/kind"),
        Some(&serde_json::json!("indexed_baseline"))
    );
    assert!(
        baseline_context
            .pointer("/evidence")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|evidence| !evidence.is_empty())
    );

    std::fs::write(
        repo.path().join("src/app.rs"),
        "pub fn run() { crate::services::shared::helper(); }\n",
    )
    .expect("dirty cross-module edit");
    let overlay = code_graph_workspace_context_overlay(
        &config,
        &repo_id,
        repo.path(),
        "COE-546",
        &base_revision,
        &context_query,
    )
    .expect("workspace context overlay");
    let overlay_context = code_graph_context(&config, context_query, Some(&overlay))
        .expect("dirty workspace context should be available");
    assert_eq!(
        overlay_context.pointer("/provenance/kind"),
        Some(&serde_json::json!("workspace_overlay"))
    );
    assert!(
        overlay_context
            .pointer("/evidence")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|evidence| evidence.iter().any(|entry| {
                entry.pointer("/provenance") == Some(&serde_json::json!("workspace_overlay"))
            }))
    );

    let diff = code_graph_workspace_diff_overlay(
        &config,
        &repo_id,
        repo.path(),
        "COE-546",
        &base_revision,
        500,
    )
    .expect("workspace topology diff");
    assert!(
        diff.modified_symbols
            .iter()
            .any(|symbol| symbol.after.as_ref().is_some_and(|side| side.name == "run"))
    );
    assert!(
        diff.edge_deltas
            .iter()
            .any(|edge| edge.status == CodeDiffEdgeStatus::Added)
    );
    assert!(
        diff.module_connection_deltas
            .iter()
            .any(|connection| connection.status == CodeDiffEdgeStatus::Added)
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_code_graph_contract_endpoints() {
    let repo = tempfile::tempdir().expect("memory repo");
    let config = write_code_graph_fixture(repo.path());
    let config_for_revision_regression = config.clone();
    let store = SnapshotStore::new(fixture_snapshot(0));
    let journal = opensymphony::opensymphony_domain::InMemoryEventJournal::new(100, 16);
    let broker = opensymphony::opensymphony_domain::StreamBroker::new(journal.clone());
    let server = GatewayServer::with_journal(store, journal.clone(), broker)
        .with_memory_config(Some(config));
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
    let base = format!("http://{address}/api/v1/code");

    let repos = client
        .get(format!("{base}/repos"))
        .send()
        .await
        .expect("fetch code repos")
        .json::<CodeRepoList>()
        .await
        .expect("decode code repos");
    assert_eq!(repos.schema_version.major, 1);
    assert_eq!(repos.repos.len(), 1);
    assert_eq!(repos.repos[0].repo_id, "opensymphony");
    assert_eq!(repos.repos[0].document_count, 6);
    assert_eq!(repos.repos[0].freshness, CodeGraphFreshness::Current);
    assert!(
        !serde_json::to_string(&repos)
            .expect("code repo list serializes")
            .contains(&repo.path().display().to_string())
    );

    let atlas_graph = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=atlas&aggregate=directory"
        ))
        .send()
        .await
        .expect("fetch atlas code graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode atlas code graph");
    assert_eq!(
        atlas_graph.mode,
        opensymphony::opensymphony_gateway_schema::code_graph::CodeGraphMode::Atlas
    );
    assert!(atlas_graph.nodes.iter().any(|node| {
        node.kind == CodeGraphNodeKind::File && node.path_display.as_deref() == Some("src/lib.rs")
    }));
    let unsupported_atlas = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=atlas&aggregate=community"
        ))
        .send()
        .await
        .expect("fetch unsupported aggregate code graph");
    assert_eq!(unsupported_atlas.status(), reqwest::StatusCode::BAD_REQUEST);

    let graph = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=file&path=src/lib.rs"
        ))
        .send()
        .await
        .expect("fetch code graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode code graph");
    let graph_json = serde_json::to_string(&graph).expect("graph serializes");
    assert_eq!(
        graph.mode,
        opensymphony::opensymphony_gateway_schema::code_graph::CodeGraphMode::File
    );
    assert!(graph_json.contains("new_feature"));
    assert!(!graph_json.contains("legacy"));
    assert!(!graph_json.contains("workspace_path"));
    assert!(!graph_json.contains(&repo.path().display().to_string()));
    let run_node = graph
        .nodes
        .iter()
        .find(|node| node.kind == CodeGraphNodeKind::Symbol && node.label == "run")
        .expect("run symbol node");
    let run_symbol_key = run_node.symbol_key.as_deref().expect("symbol key");
    assert_eq!(run_node.path_display.as_deref(), Some("src/lib.rs"));
    assert_eq!(run_node.container_chain, vec!["App".to_string()]);
    assert_eq!(run_node.diagnostic_count, 2);
    assert_eq!(run_node.diagnostic_severity.as_deref(), Some("warning"));
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        graph
            .edges
            .iter()
            .all(|edge| node_ids.contains(edge.source_id.as_str())
                && node_ids.contains(edge.target_id.as_str())),
        "file graph edges must not reference missing nodes"
    );
    assert!(
        graph.nodes.iter().any(|node| {
            node.kind == CodeGraphNodeKind::Symbol
                && node.symbol_key.is_none()
                && node.label == "missing_call"
        }),
        "unresolved edge targets should be represented as placeholder nodes"
    );

    let empty_graph = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=file&path=src/empty.rs"
        ))
        .send()
        .await
        .expect("fetch empty code graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode empty code graph");
    assert!(empty_graph.nodes.iter().any(|node| {
        node.kind == CodeGraphNodeKind::File
            && node.path_display.as_deref() == Some("src/empty.rs")
            && node.language.as_deref() == Some("rust")
    }));

    let missing_file_graph = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=file&path=src/missing.rs"
        ))
        .send()
        .await
        .expect("fetch missing file graph");
    assert_eq!(missing_file_graph.status(), reqwest::StatusCode::NOT_FOUND);

    let neighborhood_graph = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=neighborhood&symbol_key={run_symbol_key}&depth=1"
        ))
        .send()
        .await
        .expect("fetch neighborhood code graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode neighborhood code graph");
    assert_eq!(
        neighborhood_graph.mode,
        opensymphony::opensymphony_gateway_schema::code_graph::CodeGraphMode::Neighborhood
    );
    assert!(
        neighborhood_graph
            .nodes
            .iter()
            .any(|node| node.symbol_key.as_deref() == Some(run_symbol_key))
    );

    let stale_graph = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=file&path=src/lib.rs&include_stale=true"
        ))
        .send()
        .await
        .expect("fetch stale code graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode stale code graph");
    assert!(
        stale_graph
            .filters_applied
            .contains(&"include_stale:true".to_string())
    );
    assert!(
        serde_json::to_string(&stale_graph)
            .expect("stale graph serializes")
            .contains("legacy")
    );
    let stale_run = stale_graph
        .nodes
        .iter()
        .find(|node| node.kind == CodeGraphNodeKind::Symbol && node.label == "run")
        .expect("stale-inclusive run symbol");
    assert_eq!(
        stale_run.signature.as_deref(),
        Some("fn run(&self) -> Result<()>")
    );
    let legacy_node = stale_graph
        .nodes
        .iter()
        .find(|node| node.kind == CodeGraphNodeKind::Symbol && node.label == "legacy")
        .expect("stale legacy symbol");
    let legacy_symbol_key = legacy_node
        .symbol_key
        .as_deref()
        .expect("stale legacy symbol key");
    assert!(
        !stale_graph
            .edges
            .iter()
            .any(|edge| edge.source_id == stale_run.id && edge.target_id == legacy_node.id),
        "file graph must not bind current symbol edges to stale symbol nodes"
    );
    let stale_neighborhood = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=neighborhood&symbol_key={legacy_symbol_key}&depth=1&include_stale=true"
        ))
        .send()
        .await
        .expect("fetch stale neighborhood graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode stale neighborhood graph");
    assert!(
        stale_neighborhood
            .nodes
            .iter()
            .any(|node| node.symbol_key.as_deref() == Some(legacy_symbol_key))
    );
    assert!(
        stale_neighborhood
            .filters_applied
            .contains(&"include_stale:true".to_string())
    );
    let stale_neighborhood_run = stale_neighborhood
        .nodes
        .iter()
        .find(|node| node.kind == CodeGraphNodeKind::Symbol && node.label == "run")
        .expect("stale neighborhood should include base run caller");
    assert_eq!(
        stale_neighborhood_run.signature.as_deref(),
        Some("fn run(&self)"),
        "stale neighborhood edges must bind adjacent symbols from the edge revision"
    );
    assert!(
        !stale_neighborhood.nodes.iter().any(|node| {
            node.kind == CodeGraphNodeKind::Symbol
                && node.label == "run"
                && node.signature.as_deref() == Some("fn run(&self) -> Result<()>")
        }),
        "stale neighborhood must not pull current head symbols into stale/base edges"
    );

    let detail = client
        .get(format!(
            "{base}/repos/opensymphony/symbols/{}",
            run_symbol_key
        ))
        .send()
        .await
        .expect("fetch code symbol")
        .json::<CodeSymbolDetail>()
        .await
        .expect("decode code symbol");
    assert_eq!(detail.symbol_key, run_symbol_key);
    assert_eq!(detail.path_display, "src/lib.rs");
    assert_eq!(detail.container_chain, vec!["App".to_string()]);
    assert!(detail.source_snippet.is_none());
    assert!(
        detail
            .diagnostics
            .iter()
            .any(|diag| diag.severity == "warning")
    );

    let diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=head-rev"
        ))
        .send()
        .await
        .expect("fetch code diff overlay")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode code diff overlay");
    assert_eq!(diff.base_revision, "base-rev");
    assert_eq!(diff.head_revision, "head-rev");
    assert!(diff.added_symbols.iter().any(|symbol| {
        symbol
            .after
            .as_ref()
            .is_some_and(|side| side.name == "new_feature")
    }));
    assert!(diff.removed_symbols.iter().any(|symbol| {
        symbol
            .before
            .as_ref()
            .is_some_and(|side| side.name == "legacy")
    }));
    assert!(
        diff.modified_symbols
            .iter()
            .any(|symbol| { symbol.after.as_ref().is_some_and(|side| side.name == "run") })
    );
    assert!(
        !diff.edge_deltas.is_empty(),
        "edge topology deltas should be exposed"
    );
    assert!(
        diff.edge_deltas
            .iter()
            .any(|delta| delta.status == CodeDiffEdgeStatus::Added)
    );
    assert!(
        !diff.module_connection_deltas.is_empty(),
        "module topology deltas should be exposed"
    );
    let run_radius = diff
        .blast_radius
        .iter()
        .find(|radius| radius.symbol_key == run_symbol_key)
        .expect("modified run symbol should have inbound blast radius");
    assert!(run_radius.inbound_count > 0);
    assert!(run_radius.inbound.iter().all(|entry| {
        !entry.path.is_empty() && entry.distance == 1 && entry.symbol_key.is_some()
    }));
    assert_eq!(run_radius.outbound_count, 0);
    let legacy_radius = diff
        .blast_radius
        .iter()
        .find(|radius| {
            diff.removed_symbols.iter().any(|symbol| {
                symbol.symbol_key == radius.symbol_key
                    && symbol
                        .before
                        .as_ref()
                        .is_some_and(|side| side.name == "legacy")
            })
        })
        .expect("removed legacy symbol should count base-side inbound edges");
    assert_eq!(legacy_radius.inbound_count, 1);
    assert_eq!(
        diff.unanalyzed_files,
        vec![
            "src/added_empty.rs".to_string(),
            "src/deleted_empty.rs".to_string(),
            "src/empty.rs".to_string()
        ]
    );
    assert!(
        !diff
            .unanalyzed_files
            .contains(&"src/unchanged_empty.rs".to_string())
    );
    {
        let connection = DuckDbConnection::open(&config_for_revision_regression.index_path)
            .expect("index opens for legacy edge fallback fixture");
        connection
            .execute(
                "DELETE FROM code_edge_revisions WHERE repo_id = ? AND commit_sha = ?",
                duckdb::params!["opensymphony", "head-rev"],
            )
            .expect("delete head revision edge rows");
    }
    let legacy_edge_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=head-rev"
        ))
        .send()
        .await
        .expect("fetch legacy edge fallback diff")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode legacy edge fallback diff");
    let legacy_edge_radius = legacy_edge_diff
        .blast_radius
        .iter()
        .find(|radius| radius.symbol_key == run_symbol_key)
        .expect("legacy code_edges should backfill missing revision edge rows");
    assert!(legacy_edge_radius.inbound_count > 0);
    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some("head-rev".to_string()),
            worktree_dirty: false,
            documents: vec![code_graph_head_document()],
        },
    )
    .expect("head revision edge rows should restore after fallback assertion");

    let unindexed_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=missing-rev"
        ))
        .send()
        .await
        .expect("fetch unindexed code diff overlay");
    assert_eq!(unindexed_diff.status(), reqwest::StatusCode::NOT_FOUND);
    let unindexed_diff_body = unindexed_diff
        .json::<serde_json::Value>()
        .await
        .expect("decode unindexed diff response");
    assert_eq!(
        unindexed_diff_body.pointer("/error/code"),
        Some(&serde_json::json!("code_revision_not_found"))
    );

    let invalid_path = client
        .get(format!(
            "{base}/repos/opensymphony/graph?mode=file&path=/tmp/src/lib.rs"
        ))
        .send()
        .await
        .expect("fetch invalid code graph");
    assert_eq!(invalid_path.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_path_body = invalid_path
        .json::<serde_json::Value>()
        .await
        .expect("decode invalid code graph response");
    assert_eq!(
        invalid_path_body.pointer("/error/code"),
        Some(&serde_json::json!("invalid_code_graph_request"))
    );

    let report = client
        .post(format!("{base}/repos/opensymphony/index"))
        .send()
        .await
        .expect("index code repo")
        .json::<CodeIndexReport>()
        .await
        .expect("decode code index report");
    assert_eq!(report.status, CodeIndexStatus::Completed);
    assert_eq!(report.parsed_files, 6);
    assert_eq!(report.persisted_documents, 6);
    assert_eq!(report.persisted_symbols, 6);
    assert_eq!(report.persisted_edges, 4);
    assert_eq!(report.persisted_diagnostics, 3);
    assert!(report.stale_rows > 0);
    assert_eq!(report.cursor.partition, "code-graph:opensymphony");
    let second_report = client
        .post(format!("{base}/repos/opensymphony/index"))
        .send()
        .await
        .expect("index code repo again")
        .json::<CodeIndexReport>()
        .await
        .expect("decode second code index report");
    assert_eq!(second_report.status, CodeIndexStatus::Completed);
    let events = journal.all_events().await;
    let code_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                opensymphony::opensymphony_gateway_schema::event_journal::EventKind::CodeGraphUpdated { .. }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(code_events.len(), 2);
    let code_event = code_events[0];
    let second_code_event = code_events[1];
    let first_cursor_sequence = code_event
        .payload
        .as_ref()
        .and_then(|payload| payload.pointer("/cursor/sequence"))
        .and_then(|value| value.as_u64())
        .expect("first code graph cursor sequence");
    let second_cursor_sequence = second_code_event
        .payload
        .as_ref()
        .and_then(|payload| payload.pointer("/cursor/sequence"))
        .and_then(|value| value.as_u64())
        .expect("second code graph cursor sequence");
    assert!(second_cursor_sequence > first_cursor_sequence);
    assert_eq!(code_event.kind.kind_tag(), "code_graph_updated");
    let payload = code_event.payload.as_ref().expect("event payload");
    assert_eq!(
        payload.pointer("/repo_id"),
        Some(&serde_json::json!("opensymphony"))
    );
    assert_eq!(
        payload.pointer("/head_revision"),
        Some(&serde_json::json!("head-rev"))
    );
    assert_eq!(
        payload.pointer("/cursor/partition"),
        Some(&serde_json::json!("code-graph:opensymphony"))
    );
    assert_eq!(
        payload.pointer("/topology_delta_available"),
        Some(&serde_json::json!(true))
    );

    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some("same-rev".to_string()),
            worktree_dirty: false,
            documents: vec![code_graph_head_document()],
        },
    )
    .expect("same-content replacement revision should persist");
    let same_content_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=head-rev&head_revision=same-rev"
        ))
        .send()
        .await
        .expect("fetch same-content replacement diff");
    assert_eq!(same_content_diff.status(), reqwest::StatusCode::OK);
    let retained_edge_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=head-rev"
        ))
        .send()
        .await
        .expect("fetch retained edge diff")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode retained edge diff");
    let retained_run_radius = retained_edge_diff
        .blast_radius
        .iter()
        .find(|radius| radius.symbol_key == run_symbol_key)
        .expect("head revision edges should survive same-content replacement commits");
    assert!(retained_run_radius.inbound_count > 0);

    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some("stale-symbol-rev".to_string()),
            worktree_dirty: false,
            documents: vec![code_graph_head_document()],
        },
    )
    .expect("symbol-bearing stale revision should persist");
    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some("stale-symbol-rev".to_string()),
            worktree_dirty: false,
            documents: vec![code_graph_document(
                "no-symbol-head",
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )],
        },
    )
    .expect("symbol-free selected document revision should persist");
    let stale_symbol_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=stale-symbol-rev"
        ))
        .send()
        .await
        .expect("fetch stale symbol diff")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode stale symbol diff");
    assert!(
        stale_symbol_diff
            .unanalyzed_files
            .contains(&"src/lib.rs".to_string())
    );

    persist_code_intel_skipped_files(
        &config_for_revision_regression,
        "opensymphony",
        Some("base-rev"),
        false,
        &[
            CodeIntelSkippedFileInput {
                path: "README.md".into(),
                reason: "unsupported language".to_string(),
                content_sha256: "readme-base".to_string(),
            },
            CodeIntelSkippedFileInput {
                path: "assets/logo.png".into(),
                reason: "unsupported language".to_string(),
                content_sha256: "logo-base".to_string(),
            },
        ],
    )
    .expect("base skipped files should persist");
    persist_code_intel_skipped_files(
        &config_for_revision_regression,
        "opensymphony",
        Some("head-rev"),
        false,
        &[
            CodeIntelSkippedFileInput {
                path: "README.md".into(),
                reason: "unsupported language".to_string(),
                content_sha256: "readme-head".to_string(),
            },
            CodeIntelSkippedFileInput {
                path: "assets/logo.png".into(),
                reason: "unsupported language".to_string(),
                content_sha256: "logo-base".to_string(),
            },
        ],
    )
    .expect("head skipped files should persist");
    let skipped_file_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=head-rev"
        ))
        .send()
        .await
        .expect("fetch skipped-file diff")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode skipped-file diff");
    assert!(
        skipped_file_diff
            .unanalyzed_files
            .contains(&"README.md".to_string())
    );
    assert!(
        !skipped_file_diff
            .unanalyzed_files
            .contains(&"assets/logo.png".to_string())
    );
    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "skip-repo".to_string(),
            commit_sha: Some("parsed-rev".to_string()),
            worktree_dirty: false,
            documents: vec![code_graph_head_document()],
        },
    )
    .expect("parsed skip-repo file should persist");
    persist_code_intel_skipped_files(
        &config_for_revision_regression,
        "skip-repo",
        Some("skip-rev"),
        false,
        &[CodeIntelSkippedFileInput {
            path: "src/lib.rs".into(),
            reason: "parse error".to_string(),
            content_sha256: "skip-content".to_string(),
        }],
    )
    .expect("later skipped file should persist");
    let skipped_current_graph = client
        .get(format!(
            "{base}/repos/skip-repo/graph?mode=file&path=src/lib.rs"
        ))
        .send()
        .await
        .expect("fetch skipped current graph");
    assert_eq!(
        skipped_current_graph.status(),
        reqwest::StatusCode::NOT_FOUND,
        "a clean skipped revision must stale previously current parsed rows"
    );
    let skipped_stale_graph = client
        .get(format!(
            "{base}/repos/skip-repo/graph?mode=file&path=src/lib.rs&include_stale=true"
        ))
        .send()
        .await
        .expect("fetch skipped stale graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode skipped stale graph");
    assert!(
        skipped_stale_graph
            .nodes
            .iter()
            .any(|node| node.kind == CodeGraphNodeKind::Symbol && node.label == "run"),
        "stale-inclusive requests may still inspect prior parsed symbols"
    );

    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "opensymphony".to_string(),
            commit_sha: Some("dirty-only-rev".to_string()),
            worktree_dirty: true,
            documents: vec![code_graph_head_document()],
        },
    )
    .expect("dirty-only revision should persist");
    let dirty_only_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=head-rev&head_revision=dirty-only-rev"
        ))
        .send()
        .await
        .expect("fetch dirty-only revision diff");
    assert_eq!(dirty_only_diff.status(), reqwest::StatusCode::NOT_FOUND);
    let dirty_only_body = dirty_only_diff
        .json::<serde_json::Value>()
        .await
        .expect("decode dirty-only revision response");
    assert_eq!(
        dirty_only_body.pointer("/error/code"),
        Some(&serde_json::json!("code_revision_not_found"))
    );

    {
        let connection = DuckDbConnection::open(&config_for_revision_regression.index_path)
            .expect("open fixture index");
        connection
            .execute(
                "DELETE FROM code_document_revisions WHERE repo_id = 'opensymphony' AND path = 'src/empty.rs'",
                [],
            )
            .expect("delete one revision document row");
    }
    let legacy_document_diff = client
        .get(format!(
            "{base}/repos/opensymphony/diff-overlay?base_revision=base-rev&head_revision=head-rev"
        ))
        .send()
        .await
        .expect("fetch legacy document diff")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode legacy document diff");
    assert!(
        legacy_document_diff
            .unanalyzed_files
            .contains(&"src/empty.rs".to_string())
    );

    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "large-repo".to_string(),
            commit_sha: Some("large-rev".to_string()),
            worktree_dirty: false,
            documents: (0..505)
                .map(|index| {
                    code_graph_document_with_path(
                        &format!("src/file_{index}.rs"),
                        &format!("large-content-{index}"),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    )
                })
                .collect(),
        },
    )
    .expect("large code graph fixture should persist");
    let large_atlas = client
        .get(format!("{base}/repos/large-repo/graph?mode=atlas"))
        .send()
        .await
        .expect("fetch large atlas graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode large atlas graph");
    assert_eq!(large_atlas.truncation.nodes_dropped, 5);
    persist_code_intel_documents(
        &config_for_revision_regression,
        CodeIntelPersistBatch {
            repo_id: "dedupe-repo".to_string(),
            commit_sha: Some("dedupe-rev".to_string()),
            worktree_dirty: false,
            documents: (0..505)
                .map(|index| {
                    code_graph_document_with_path(
                        "src/dup.rs",
                        &format!("dedupe-content-{index}"),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                    )
                })
                .chain(std::iter::once(code_graph_document_with_path(
                    "src/z_current.rs",
                    "dedupe-current-z",
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                )))
                .collect(),
        },
    )
    .expect("duplicate-heavy atlas fixture should persist");
    let deduped_atlas = client
        .get(format!(
            "{base}/repos/dedupe-repo/graph?mode=atlas&include_stale=true"
        ))
        .send()
        .await
        .expect("fetch deduped atlas graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode deduped atlas graph");
    assert_eq!(deduped_atlas.truncation.nodes_dropped, 0);
    assert!(deduped_atlas.nodes.iter().any(|node| {
        node.kind == CodeGraphNodeKind::File
            && node.path_display.as_deref() == Some("src/z_current.rs")
    }));
    let duplicate_file = deduped_atlas
        .nodes
        .iter()
        .find(|node| {
            node.kind == CodeGraphNodeKind::File
                && node.path_display.as_deref() == Some("src/dup.rs")
        })
        .expect("duplicate path should still produce one file node");
    assert_eq!(duplicate_file.freshness, CodeGraphFreshness::Current);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_code_outline_without_workspace_root_leakage() {
    let memory_repo = tempfile::tempdir().expect("memory repository");
    let memory_config = MemoryConfig::load(memory_repo.path(), None).expect("memory config");
    let root = tempfile::tempdir().expect("workspace root");
    let workspace = root.path().join("COE-533");
    std::fs::create_dir_all(workspace.join("src")).expect("workspace dirs");
    std::fs::write(
        workspace.join("src/lib.rs"),
        "pub struct App;\nimpl App { pub fn run(&self) {} }\n",
    )
    .expect("workspace file");
    std::fs::write(workspace.join("data.txt"), "fixture\n").expect("workspace text file");
    let mut snapshot = fixture_snapshot(0);
    snapshot.daemon.workspace_root = root.path().to_string_lossy().to_string();
    snapshot.issues[0].identifier = "COE-533".to_string();
    snapshot.issues[0].workspace_path_suffix = "COE-533".to_string();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_memory_config(Some(memory_config));
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
    let outline = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/outline?file_path=src/lib.rs&repo_id=opensymphony"
        ))
        .send()
        .await
        .expect("fetch run outline")
        .json::<CodeFileOutline>()
        .await
        .expect("decode run outline");
    assert_eq!(outline.run_id, "COE-533");
    assert_eq!(outline.repo_id.as_deref(), Some("opensymphony"));
    assert_eq!(outline.path, "src/lib.rs");
    let outline_json = serde_json::to_string(&outline).expect("outline serializes");
    assert!(outline.symbols.iter().all(|symbol| {
        symbol.path == "src/lib.rs"
            && symbol.span.end_line >= symbol.span.start_line
            && symbol.selection_span.end_line >= symbol.selection_span.start_line
    }));
    assert!(!outline_json.contains("workspace_path"));
    assert!(!outline_json.contains(&root.path().display().to_string()));

    let unsupported_outline = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/outline?file_path=data.txt"
        ))
        .send()
        .await
        .expect("fetch unsupported run outline");
    assert_eq!(unsupported_outline.status(), reqwest::StatusCode::OK);
    let unsupported_outline = unsupported_outline
        .json::<CodeFileOutline>()
        .await
        .expect("decode unsupported run outline");
    assert_eq!(unsupported_outline.run_id, "COE-533");
    assert_eq!(unsupported_outline.path, "data.txt");
    assert!(unsupported_outline.symbols.is_empty());

    let invalid_outline = client
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/outline?file_path=../secret.rs"
        ))
        .send()
        .await
        .expect("fetch invalid run outline");
    assert_eq!(invalid_outline.status(), reqwest::StatusCode::BAD_REQUEST);

    #[cfg(unix)]
    {
        std::fs::write(root.path().join("secret.rs"), "pub fn secret() {}\n")
            .expect("outside workspace file");
        std::os::unix::fs::symlink(
            root.path().join("secret.rs"),
            workspace.join("src/escape.rs"),
        )
        .expect("symlink escape");
        let symlink_outline = client
            .get(format!(
                "http://{address}/api/v1/runs/COE-533/code/outline?file_path=src/escape.rs"
            ))
            .send()
            .await
            .expect("fetch symlink run outline");
        assert_eq!(symlink_outline.status(), reqwest::StatusCode::BAD_REQUEST);
    }

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_code_diff_overlay_with_resolved_revisions() {
    let memory_repo = tempfile::tempdir().expect("memory repo");
    let root = tempfile::tempdir().expect("workspace root");
    let workspace = root.path().join("COE-533");
    std::fs::create_dir_all(workspace.join("src")).expect("workspace dirs");
    run_git(&workspace, &["init"]);
    run_git(&workspace, &["checkout", "-b", "develop"]);
    run_git(&workspace, &["config", "user.email", "test@example.com"]);
    run_git(&workspace, &["config", "user.name", "Test User"]);
    std::fs::write(workspace.join(".gitignore"), "generated.rs\n").expect("gitignore");
    std::fs::write(workspace.join("src/deleted_empty.rs"), "").expect("deleted empty file");
    std::fs::write(workspace.join("src/diag.rs"), "pub fn diagnosed() {}\n")
        .expect("diagnostic file");
    std::fs::write(workspace.join("src/lib.rs"), "pub fn base() {}\n").expect("base file");
    run_git(&workspace, &["add", "."]);
    run_git(&workspace, &["commit", "-m", "base"]);
    let base_revision = run_git(&workspace, &["rev-parse", "HEAD"]);
    run_git(&workspace, &["checkout", "-b", "feat/code-graph"]);
    std::fs::write(
        workspace.join("src/lib.rs"),
        "pub fn base() {}\npub fn head() {}\n",
    )
    .expect("head file");
    std::fs::remove_file(workspace.join("src/deleted_empty.rs")).expect("delete empty file");
    run_git(&workspace, &["add", "-A"]);
    run_git(&workspace, &["commit", "-m", "head"]);
    let head_revision = run_git(&workspace, &["rev-parse", "HEAD"]);
    let config =
        write_code_graph_fixture_with_revisions(memory_repo.path(), &base_revision, &head_revision);

    let mut snapshot = fixture_snapshot(0);
    snapshot.daemon.workspace_root = root.path().to_string_lossy().to_string();
    snapshot.issues[0].identifier = "COE-533".to_string();
    snapshot.issues[0].workspace_path_suffix = "COE-533".to_string();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_memory_config(Some(config));
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

    std::fs::write(
        workspace.join("generated.rs"),
        "pub fn generated_outline() {}\n",
    )
    .expect("ignored generated file");
    let generated_outline = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/outline?file_path=generated.rs&repo_id=opensymphony"
        ))
        .send()
        .await
        .expect("fetch ignored generated outline");
    assert_eq!(generated_outline.status(), reqwest::StatusCode::OK);
    let generated_outline = generated_outline
        .json::<CodeFileOutline>()
        .await
        .expect("decode ignored generated outline");
    assert!(
        generated_outline
            .symbols
            .iter()
            .any(|symbol| symbol.name == "generated_outline")
    );

    let overlay = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/diff-overlay?repo_id=opensymphony"
        ))
        .send()
        .await
        .expect("fetch run code diff overlay")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode run code diff overlay");
    assert_eq!(overlay.repo_id, "opensymphony");
    assert_eq!(overlay.base_revision, base_revision);
    assert_eq!(overlay.head_revision, head_revision);
    let overlay_json = serde_json::to_string(&overlay).expect("overlay serializes");
    assert!(!overlay_json.contains("workspace_path"));
    assert!(!overlay_json.contains(&root.path().display().to_string()));

    std::fs::write(
        workspace.join("src/lib.rs"),
        "pub fn base() {}\npub fn head() {}\npub fn dirty() {}\n",
    )
    .expect("dirty file");
    let dirty_overlay = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/diff-overlay?repo_id=opensymphony"
        ))
        .send()
        .await
        .expect("fetch dirty run code diff overlay")
        .json::<CodeDiffOverlay>()
        .await
        .expect("decode dirty run code diff overlay");
    assert_eq!(dirty_overlay.repo_id, "opensymphony");
    assert_eq!(dirty_overlay.base_revision, base_revision);
    assert_eq!(
        dirty_overlay.head_revision,
        format!("{head_revision}+worktree")
    );
    assert!(
        !dirty_overlay.added_symbols.is_empty(),
        "dirty overlays should keep indexed base-to-HEAD symbol diffs"
    );
    assert!(
        dirty_overlay
            .unanalyzed_files
            .iter()
            .any(|path| path == "src/deleted_empty.rs")
    );
    let diagnostic_response = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/graph?repo_id=opensymphony&mode=atlas&include_stale=true"
        ))
        .send()
        .await
        .expect("fetch unchanged diagnostic graph");
    assert_eq!(diagnostic_response.status(), reqwest::StatusCode::OK);
    let diagnostic_graph = diagnostic_response
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode unchanged diagnostic graph");
    assert!(
        diagnostic_graph
            .filters_applied
            .contains(&"include_stale:true".to_string())
    );
    let diagnostic_node = diagnostic_graph
        .nodes
        .iter()
        .find(|node| node.label == "diagnosed")
        .expect("diagnosed symbol node");
    assert_eq!(diagnostic_node.diagnostic_count, 1);
    assert_eq!(
        diagnostic_node.diagnostic_severity.as_deref(),
        Some("warning")
    );
    let graph = reqwest::Client::new()
        .get(format!(
            "http://{address}/api/v1/runs/COE-533/code/graph?repo_id=opensymphony&mode=file&path=src/lib.rs"
        ))
        .send()
        .await
        .expect("fetch dirty run code graph")
        .json::<CodeGraphSnapshot>()
        .await
        .expect("decode dirty run code graph");
    assert!(graph.nodes.iter().any(|node| node.label == "dirty"));
    let dirty_overlay_json = serde_json::to_string(&dirty_overlay).expect("overlay serializes");
    assert!(!dirty_overlay_json.contains("workspace_path"));
    assert!(!dirty_overlay_json.contains(&root.path().display().to_string()));

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
async fn gateway_task_graph_skips_completed_issues_without_project_metadata() {
    let mut snapshot = fixture_snapshot(0);
    let mut stale_issue = snapshot.issues[0].clone();
    stale_issue.identifier = "COE-370".to_owned();
    stale_issue.runtime_state = IssueRuntimeState::Completed;
    stale_issue.project_id = None;
    stale_issue.project_slug = None;
    stale_issue.project_name = None;
    snapshot.issues.push(stale_issue);
    let linear_issues = snapshot
        .issues
        .iter()
        .filter(|issue| issue.project_slug.is_some())
        .map(|issue| tracker_issue_from_snapshot(issue, &[]))
        .collect::<Vec<_>>();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_linear_task_graph(Some(std::sync::Arc::new(
        StrictLinearTaskGraphClient {
            issues: linear_issues,
        },
    )));
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
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.nodes[0].identifier, "COE-255");

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_includes_backlog_issues_with_cross_edges() {
    let snapshot = fixture_snapshot(0);
    let issues = snapshot
        .issues
        .iter()
        .map(|issue| tracker_issue_from_snapshot(issue, &[]))
        .collect::<Vec<_>>();
    let now = Utc::now();
    let backlog_issue = TrackerIssue {
        id: "COE-900".to_owned(),
        identifier: "COE-900".to_owned(),
        url: "https://linear.app/kumanday/issue/COE-900".to_owned(),
        title: "Backlog follow-up".to_owned(),
        description: None,
        priority: None,
        state: "Backlog".to_owned(),
        state_kind: TrackerIssueStateKind::Backlog,
        branch_name: None,
        pr_url: None,
        labels: Vec::new(),
        project_id: Some("proj-open".to_owned()),
        project_slug: Some("opensymphony-bootstrap".to_owned()),
        project_name: Some("OpenSymphony".to_owned()),
        parent_id: None,
        parent: None,
        project_milestone: None,
        blocked_by: vec![TrackerIssueBlocker {
            id: "COE-255".to_owned(),
            identifier: "COE-255".to_owned(),
            title: "Observability and FrankenTUI".to_owned(),
            state: TrackerIssueState {
                id: "state-COE-255".to_owned(),
                name: "In Progress".to_owned(),
                tracker_type: "started".to_owned(),
                kind: TrackerIssueStateKind::Started,
            },
        }],
        sub_issues: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    // An issue just promoted Backlog→Todo: active in Linear but untracked by
    // the control plane, so it arrives only via the unrequested scan bucket.
    let promoted_issue = TrackerIssue {
        id: "COE-901".to_owned(),
        identifier: "COE-901".to_owned(),
        url: "https://linear.app/kumanday/issue/COE-901".to_owned(),
        title: "Freshly promoted todo".to_owned(),
        description: None,
        priority: None,
        state: "Todo".to_owned(),
        state_kind: TrackerIssueStateKind::Unstarted,
        branch_name: None,
        pr_url: None,
        labels: Vec::new(),
        project_id: Some("proj-open".to_owned()),
        project_slug: Some("opensymphony-bootstrap".to_owned()),
        project_name: Some("OpenSymphony".to_owned()),
        parent_id: None,
        parent: None,
        project_milestone: None,
        blocked_by: Vec::new(),
        sub_issues: Vec::new(),
        created_at: now,
        updated_at: now,
    };
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_linear_task_graph(Some(std::sync::Arc::new(
        BacklogLinearTaskGraphClient {
            issues,
            unrequested: vec![backlog_issue, promoted_issue],
        },
    )));
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
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    assert_eq!(response.nodes.len(), 3);
    // The untracked-but-active issue lands in the Current pane as Todo
    // instead of vanishing until the orchestrator picks it up.
    let promoted_node = response
        .nodes
        .iter()
        .find(|node| node.identifier == "COE-901")
        .expect("promoted todo issue should be present in the task graph");
    assert_eq!(
        promoted_node.state_category,
        opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphStateCategory::Todo,
    );
    assert!(promoted_node.runtime_overlay.is_none());
    let backlog_node = response
        .nodes
        .iter()
        .find(|node| node.identifier == "COE-900")
        .expect("backlog issue should be present in the task graph");
    assert_eq!(
        backlog_node.state_category,
        opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphStateCategory::Backlog,
    );
    // The blocked_by edge crosses from the backlog node to the tracked
    // current node — the UI draws it as a Current → Backlog edge.
    assert_eq!(backlog_node.blocked_by, vec!["COE-255".to_owned()]);
    assert!(backlog_node.runtime_overlay.is_none());

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_keeps_failed_issues_visible_as_todo() {
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].runtime_state = IssueRuntimeState::Failed;
    snapshot.issues[0].last_outcome = WorkerOutcome::Failed;
    let issues = snapshot
        .issues
        .iter()
        .map(|issue| tracker_issue_from_snapshot(issue, &[]))
        .collect::<Vec<_>>();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_linear_task_graph(Some(std::sync::Arc::new(
        FakeLinearTaskGraphClient { issues },
    )));
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
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    // A failed run is not completed work: `done` would hide the issue from
    // every pane (the Completed table only merges Completed rows), so it
    // categorizes as `todo` and keeps its run entry point in Current.
    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.nodes[0].identifier, "COE-255");
    assert_eq!(
        response.nodes[0].state_category,
        opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphStateCategory::Todo,
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_categorizes_idle_issues_by_tracker_state() {
    // An Idle control-plane entry carries no run information — e.g. a
    // recovered workspace parked while its issue sits in Backlog. The
    // tracker state must decide the category, or the issue would surface
    // as Todo in the Current pane instead of staying in Backlog.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].runtime_state = IssueRuntimeState::Idle;
    snapshot.issues[0].tracker_state = "Backlog".to_owned();
    let issues = snapshot
        .issues
        .iter()
        .map(|issue| tracker_issue_from_snapshot(issue, &[]))
        .collect::<Vec<_>>();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_linear_task_graph(Some(std::sync::Arc::new(
        FakeLinearTaskGraphClient { issues },
    )));
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
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    assert_eq!(response.nodes.len(), 1);
    assert_eq!(response.nodes[0].identifier, "COE-255");
    assert_eq!(
        response.nodes[0].state_category,
        opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphStateCategory::Backlog,
    );
    // A parked issue is not schedulable until its tracker state turns
    // active, so its overlay must not advertise it as queued or eligible.
    let overlay = response.nodes[0]
        .runtime_overlay
        .as_ref()
        .expect("tracked issue should carry a runtime overlay");
    assert!(!overlay.eligible);
    assert!(!overlay.queued);

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_suppresses_overlay_for_parked_non_active_state() {
    // A parked recovered issue whose tracker state maps to the coarse `todo`
    // graph category but is NOT a configured active state (e.g. Triage, or a
    // custom state) must not be advertised as queued/eligible: the scheduler
    // released it as tracker-inactive and will not dispatch it until it
    // enters a configured active state.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].runtime_state = IssueRuntimeState::Idle;
    snapshot.issues[0].tracker_state = "Triage".to_owned();
    let issues = snapshot
        .issues
        .iter()
        .map(|issue| tracker_issue_from_snapshot(issue, &[]))
        .collect::<Vec<_>>();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store)
        .with_linear_task_graph(Some(std::sync::Arc::new(FakeLinearTaskGraphClient {
            issues,
        })))
        .with_active_states(["Todo".to_owned(), "In Progress".to_owned()]);
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
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    // Triage still lands in the coarse `todo` category ...
    assert_eq!(
        response.nodes[0].state_category,
        opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphStateCategory::Todo,
    );
    // ... but it is not a configured active state, so the overlay is clean.
    let overlay = response.nodes[0]
        .runtime_overlay
        .as_ref()
        .expect("tracked issue should carry a runtime overlay");
    assert!(!overlay.eligible);
    assert!(!overlay.queued);

    server_task.abort();
}

#[tokio::test]
async fn gateway_task_graph_marks_configured_active_state_eligible() {
    // A custom active state that maps to the coarse `todo` category (e.g.
    // "Rework") is genuinely dispatchable when listed in active_states, so
    // its overlay must be queued/eligible even though the category alone is
    // ambiguous. This is why dispatchability keys on active_states, not the
    // category or the semantic tracker-state kind.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].runtime_state = IssueRuntimeState::Idle;
    snapshot.issues[0].tracker_state = "Rework".to_owned();
    let issues = snapshot
        .issues
        .iter()
        .map(|issue| tracker_issue_from_snapshot(issue, &[]))
        .collect::<Vec<_>>();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store)
        .with_linear_task_graph(Some(std::sync::Arc::new(FakeLinearTaskGraphClient {
            issues,
        })))
        .with_active_states(["Todo".to_owned(), "Rework".to_owned()]);
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
        .expect("fetch task graph")
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");

    let overlay = response.nodes[0]
        .runtime_overlay
        .as_ref()
        .expect("tracked issue should carry a runtime overlay");
    assert!(overlay.eligible);
    assert!(overlay.queued);

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_memory_completed_tasks() {
    let repo = tempfile::tempdir().expect("memory repo");
    let config = write_completed_tasks_fixture(repo.path());
    refresh_memory_index_from_okf(&config, &repo.path().join(".opensymphony/memory"))
        .expect("fixture should reindex");

    // The control plane knows one freshly completed issue that memory has
    // not captured yet — it must appear as an `orchestrator` row.
    let mut snapshot = fixture_snapshot(0);
    let mut completed_issue = snapshot.issues[0].clone();
    completed_issue.identifier = "COE-370".to_owned();
    completed_issue.title = "Fresh completion".to_owned();
    completed_issue.tracker_state = "Done".to_owned();
    completed_issue.runtime_state = IssueRuntimeState::Completed;
    completed_issue.finished_at = Some(Utc::now());
    completed_issue.pr_url = Some("https://github.com/kumanday/OpenSymphony/pull/370".to_owned());
    snapshot.issues.push(completed_issue);

    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store).with_memory_config(Some(config));
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
    let base = format!("http://{address}/api/v1/memory/completed-tasks");

    let page = client
        .get(&base)
        .send()
        .await
        .expect("fetch completed tasks")
        .json::<MemoryCompletedTaskPage>()
        .await
        .expect("decode completed tasks");
    assert_eq!(page.schema_version.major, 1);
    assert_eq!(page.total, 3);
    assert_eq!(page.sort, "completed_desc");
    // Freshest completion first: the orchestrator row finished just now,
    // the memory capsules' timestamps are in the past.
    assert_eq!(page.tasks[0].issue_key, "COE-370");
    assert_eq!(
        page.tasks[0].source,
        opensymphony::opensymphony_gateway_schema::memory_graph::MemoryCompletedTaskSource::Orchestrator,
    );
    assert_eq!(page.tasks[0].prs.len(), 1);
    assert_eq!(page.tasks[0].prs[0].number, 370);
    assert_eq!(page.tasks[1].issue_key, "COE-300");
    assert_eq!(
        page.tasks[1].source,
        opensymphony::opensymphony_gateway_schema::memory_graph::MemoryCompletedTaskSource::Memory,
    );
    assert_eq!(page.tasks[1].concept_id, "issues/COE-300");
    assert_eq!(page.tasks[1].state.as_deref(), Some("Done"));
    assert_eq!(page.tasks[2].issue_key, "COE-302");
    // The In Progress capsule must not leak into the completed list.
    assert!(page.tasks.iter().all(|task| task.issue_key != "COE-301"));

    // Public views serve only public memory capsules: the private COE-302
    // stays out, and so do orchestrator rows — the control plane has no
    // visibility metadata, so merging them would reintroduce
    // privately-captured tasks past the filter.
    let public = client
        .get(format!("{base}?visibility=public"))
        .send()
        .await
        .expect("fetch public completed tasks")
        .json::<MemoryCompletedTaskPage>()
        .await
        .expect("decode public completed tasks");
    assert_eq!(public.total, 1);
    assert_eq!(public.tasks[0].issue_key, "COE-300");
    assert_eq!(
        public.tasks[0].source,
        opensymphony::opensymphony_gateway_schema::memory_graph::MemoryCompletedTaskSource::Memory,
    );

    // Search narrows on title/key, pagination clamps.
    let filtered = client
        .get(format!("{base}?query=fresh&limit=1&sort=id_asc"))
        .send()
        .await
        .expect("fetch filtered completed tasks")
        .json::<MemoryCompletedTaskPage>()
        .await
        .expect("decode filtered completed tasks");
    assert_eq!(filtered.total, 1);
    assert_eq!(filtered.tasks[0].issue_key, "COE-370");
    assert_eq!(filtered.sort, "id_asc");
    assert_eq!(filtered.query.as_deref(), Some("fresh"));

    let second_page = client
        .get(format!("{base}?limit=1&offset=1"))
        .send()
        .await
        .expect("fetch second page")
        .json::<MemoryCompletedTaskPage>()
        .await
        .expect("decode second page");
    assert_eq!(second_page.total, 3);
    assert_eq!(second_page.tasks.len(), 1);
    assert_eq!(second_page.tasks[0].issue_key, "COE-300");

    server_task.abort();
}

#[tokio::test]
async fn gateway_completed_tasks_serve_orchestrator_rows_without_memory_catalog() {
    // No memory catalog configured: a local run's completed issues must
    // still surface as `orchestrator` rows (the desktop Completed pane is
    // the only place `done` nodes appear), not 503.
    let mut snapshot = fixture_snapshot(0);
    let mut completed_issue = snapshot.issues[0].clone();
    completed_issue.identifier = "COE-370".to_owned();
    completed_issue.title = "Local completion".to_owned();
    completed_issue.tracker_state = "Done".to_owned();
    completed_issue.runtime_state = IssueRuntimeState::Completed;
    completed_issue.finished_at = Some(Utc::now());
    completed_issue.pr_url = Some("https://github.com/kumanday/OpenSymphony/pull/370".to_owned());
    snapshot.issues.push(completed_issue);

    // GatewayServer::new leaves memory_config unset.
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store);
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
        .get(format!("http://{address}/api/v1/memory/completed-tasks"))
        .send()
        .await
        .expect("fetch completed tasks");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let page = response
        .json::<MemoryCompletedTaskPage>()
        .await
        .expect("decode completed tasks");
    assert_eq!(page.total, 1);
    assert_eq!(page.tasks[0].issue_key, "COE-370");
    assert_eq!(
        page.tasks[0].source,
        opensymphony::opensymphony_gateway_schema::memory_graph::MemoryCompletedTaskSource::Orchestrator,
    );

    server_task.abort();
}

fn write_completed_tasks_fixture(repo: &std::path::Path) -> MemoryConfig {
    let config_path = repo.join("opensymphony-memory.yaml");
    std::fs::write(
        &config_path,
        r#"
areas:
  graph-view:
    title: Graph View
    docs_target: docs/graph-view.md
    status: stable
    confidence: 90
"#,
    )
    .expect("memory config should write");
    let memory_root = repo.join(".opensymphony/memory");
    let issues_dir = memory_root.join("issues");
    std::fs::create_dir_all(&issues_dir).expect("memory issues dir should write");
    std::fs::write(
        issues_dir.join("COE-300.md"),
        r#"---
type: issue-capsule
title: "COE-300: Completed capsule"
description: Completed task fixture.
state: Done
timestamp: 2026-06-20T10:00:00Z
tags: [memory]
opensymphony:
  visibility: public
  scope_refs:
    - kind: work_item
      id: COE-300
    - kind: area
      id: graph-view
  source_refs:
    - kind: linear_issue
      id: COE-300
      url: https://linear.app/example/issue/COE-300
---

# COE-300: Completed capsule

Completed body.
"#,
    )
    .expect("completed capsule should write");
    std::fs::write(
        issues_dir.join("COE-301.md"),
        r#"---
type: issue-capsule
title: "COE-301: Active capsule"
description: Not completed yet.
state: In Progress
timestamp: 2026-06-21T10:00:00Z
tags: [memory]
opensymphony:
  visibility: public
  scope_refs:
    - kind: work_item
      id: COE-301
    - kind: area
      id: graph-view
---

# COE-301: Active capsule

Active body.
"#,
    )
    .expect("active capsule should write");
    std::fs::write(
        issues_dir.join("COE-302.md"),
        r#"---
type: issue-capsule
title: "COE-302: Private completed capsule"
description: Completed but private.
state: Done
timestamp: 2026-06-19T10:00:00Z
tags: [memory]
opensymphony:
  visibility: private
  scope_refs:
    - kind: work_item
      id: COE-302
    - kind: area
      id: graph-view
---

# COE-302: Private completed capsule

Private completed body.
"#,
    )
    .expect("private completed capsule should write");
    MemoryConfig::load(repo, Some(&config_path)).expect("memory config should load")
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
async fn gateway_task_graph_empty_project_without_linear_returns_empty_ok() {
    // A control-plane-only/local run with no tracked issues has nothing to
    // expand and no backlog to discover, so a missing Linear client is a
    // valid empty project (200), not a 503.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues.clear();
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store);
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
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let body = response
        .json::<opensymphony::opensymphony_gateway_schema::task_graph::TaskGraphSnapshot>()
        .await
        .expect("decode task graph");
    assert!(body.nodes.is_empty());
    assert!(body.root_ids.is_empty());

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
    assert_eq!(response.turn_count, 3);
    assert_eq!(response.max_turns, 8);
    assert_eq!(response.runtime_seconds, 75);
    assert_eq!(
        response.branch_name.as_deref(),
        Some("feat/coe-255-observability")
    );
    assert_eq!(
        response.pr_url.as_deref(),
        Some("https://github.com/kumanday/OpenSymphony/pull/255")
    );
    assert_eq!(
        response.status,
        opensymphony::opensymphony_gateway_schema::run::RunStatus::Running
    );
    // The desktop "Workspace" / "Debug" actions need the on-disk path: the
    // workspace root joined with the run's suffix.
    assert_eq!(
        response.workspace_path.as_deref(),
        Some("/tmp/opensymphony/COE-255")
    );
    // An OpenHands run reports the OpenHands harness and no Codex thread id.
    assert_eq!(response.harness_type.as_deref(), Some("openhands"));
    assert_eq!(response.codex_thread_id, None);

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_exposes_codex_thread_id_and_harness() {
    // A Codex run carries a Codex thread id, so run detail reports the
    // codex_app_server harness and the full thread id — the desktop Debug
    // button opens codex://threads/<id> from these.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].codex_thread_id = Some("019f3979-3aa3-71f3-86b1-18e92c71fbc9".to_owned());
    let store = SnapshotStore::new(snapshot);
    let server = GatewayServer::new(store);
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
        .get(format!("http://{address}/api/v1/runs/COE-255"))
        .send()
        .await
        .expect("fetch run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.harness_type.as_deref(), Some("codex_app_server"));
    assert_eq!(
        response.codex_thread_id.as_deref(),
        Some("019f3979-3aa3-71f3-86b1-18e92c71fbc9")
    );

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_parked_non_active_issue_is_not_eligible_or_retryable() {
    use opensymphony::opensymphony_gateway_schema::run::{RunAction, RunLifecycleState, RunStatus};

    // A recovered issue parked in Backlog maps to Idle in the control plane
    // but is not dispatchable until its tracker state becomes active. Run
    // detail must not advertise it as an eligible, retryable run.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].runtime_state = IssueRuntimeState::Idle;
    snapshot.issues[0].tracker_state = "Backlog".to_owned();
    let store = SnapshotStore::new(snapshot);
    let server =
        GatewayServer::new(store).with_active_states(["Todo".to_owned(), "In Progress".to_owned()]);
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
        .get(format!("http://{address}/api/v1/runs/COE-255"))
        .send()
        .await
        .expect("fetch run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.status, RunStatus::Unclaimed);
    assert_eq!(response.lifecycle_state, RunLifecycleState::Backlog);
    assert!(
        !response.allowed_actions.contains(&RunAction::Retry),
        "a parked non-active issue must not offer a retry action"
    );
    assert!(!response.safe_actions.retry);

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_dispatchable_idle_issue_is_eligible_and_retryable() {
    use opensymphony::opensymphony_gateway_schema::run::{RunAction, RunLifecycleState, RunStatus};

    // An Idle issue whose tracker state IS a configured active state is a
    // genuine queued run: eligible and retryable.
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].runtime_state = IssueRuntimeState::Idle;
    snapshot.issues[0].tracker_state = "Todo".to_owned();
    let store = SnapshotStore::new(snapshot);
    let server =
        GatewayServer::new(store).with_active_states(["Todo".to_owned(), "In Progress".to_owned()]);
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
        .get(format!("http://{address}/api/v1/runs/COE-255"))
        .send()
        .await
        .expect("fetch run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    assert_eq!(response.status, RunStatus::Unclaimed);
    assert_eq!(response.lifecycle_state, RunLifecycleState::Eligible);
    assert!(response.allowed_actions.contains(&RunAction::Retry));

    server_task.abort();
}

#[tokio::test]
async fn gateway_serves_run_detail_cancel_diagnostics() {
    let mut snapshot = fixture_snapshot(0);
    snapshot.issues[0].cancel_requested = true;
    snapshot.issues[0].cancel_reason = Some("operator_cancel".to_owned());
    let store = SnapshotStore::new(snapshot);
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
        .get(format!("http://{address}/api/v1/runs/COE-255"))
        .send()
        .await
        .expect("fetch run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode run detail");

    let diagnostics = response.diagnostics.expect("diagnostics");
    assert!(diagnostics.cancel_requested);
    assert_eq!(
        diagnostics.cancel_reason.as_deref(),
        Some("operator_cancel")
    );
    assert!(response.cancel_requested);
    assert_eq!(response.cancel_reason.as_deref(), Some("operator_cancel"));

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

    assert_eq!(parent_node.project_slug.as_deref(), Some("alpha-project"));
    assert_eq!(parent_node.project_name.as_deref(), Some("Alpha Project"));
    assert_eq!(done_node.project_slug.as_deref(), Some("beta-project"));
    assert_eq!(done_node.project_name.as_deref(), Some("Beta Project"));
    assert_eq!(external_parent_node.project_slug, None);
    assert_eq!(external_parent_node.project_name, None);

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
    assert_eq!(response.turn_count, 1);
    assert_eq!(response.max_turns, 0);
    assert_eq!(response.runtime_seconds, 20);

    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_preserves_explicit_tracker_inactive_reason() {
    let mut snapshot = fixture_snapshot_rich(0);
    let issue = snapshot
        .issues
        .iter_mut()
        .find(|issue| issue.identifier == "COE-302")
        .expect("failed fixture issue should exist");
    issue.release_reason = Some(DomainReleaseReason::TrackerInactive);
    let store = SnapshotStore::new(snapshot);
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
        .get(format!("http://{address}/api/v1/runs/COE-302"))
        .send()
        .await
        .expect("fetch tracker-inactive run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode tracker-inactive run detail");

    assert_eq!(
        response.release_reason,
        Some(opensymphony::opensymphony_gateway_schema::run::ReleaseReason::TrackerInactive)
    );
    server_task.abort();
}

#[tokio::test]
async fn gateway_run_detail_terminal_tracker_state_overrides_stale_inactive_reason() {
    let mut snapshot = fixture_snapshot_rich(0);
    let issue = snapshot
        .issues
        .iter_mut()
        .find(|issue| issue.identifier == "COE-302")
        .expect("failed fixture issue should exist");
    issue.release_reason = Some(DomainReleaseReason::TrackerInactive);
    issue.runtime_state = IssueRuntimeState::Completed;
    let store = SnapshotStore::new(snapshot);
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
        .get(format!("http://{address}/api/v1/runs/COE-302"))
        .send()
        .await
        .expect("fetch terminal tracker run detail")
        .json::<opensymphony::opensymphony_gateway_schema::run::RunDetail>()
        .await
        .expect("decode terminal tracker run detail");

    assert_eq!(
        response.release_reason,
        Some(opensymphony::opensymphony_gateway_schema::run::ReleaseReason::TrackerTerminal)
    );
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
    assert_eq!(response.turn_count, 2);
    assert_eq!(response.max_turns, 0);
    assert_eq!(response.runtime_seconds, 70);

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
