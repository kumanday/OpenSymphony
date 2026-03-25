use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{DateTime, Utc};
use opensymphony_domain::{
    ConversationId, ConversationMetadata, IssueId, IssueIdentifier, NormalizedIssue, RunAttempt,
    RuntimeStreamState, TimestampMs, WorkerId, WorkerOutcomeKind, WorkerOutcomeRecord,
};
use opensymphony_workflow::{OpenHandsConversationToolConfig, ResolvedWorkflow};
use opensymphony_workspace::{
    RunManifest, RunStatus, WorkspaceError, WorkspaceHandle, WorkspaceManager,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::time::{Instant, timeout_at};
use tracing::debug;
use uuid::Uuid;

use crate::{
    AgentConfig, CondenserConfig, ConfirmationPolicy, Conversation, ConversationCreateRequest,
    EventEnvelope, KnownEvent, LlmConfig, McpConfig, McpStdioServerConfig, OpenHandsClient,
    OpenHandsError, RuntimeEventStream, RuntimeStreamConfig, SendMessageRequest,
    TerminalExecutionStatus, ToolConfig, WorkspaceConfig,
};

pub const RUNTIME_CONTRACT_VERSION: &str = "openhands-sdk-agent-server-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSessionRunnerConfig {
    pub runtime_stream: RuntimeStreamConfig,
    pub terminal_wait_timeout: Duration,
    pub finished_drain_timeout: Duration,
}

pub trait IssueSessionObserver {
    fn on_launch(&mut self, _conversation: &ConversationMetadata) {}

    fn on_runtime_event(
        &mut self,
        _observed_at: TimestampMs,
        _event_id: Option<String>,
        _event_kind: Option<String>,
        _summary: Option<String>,
    ) {
    }
}

impl IssueSessionObserver for () {}

impl Default for IssueSessionRunnerConfig {
    fn default() -> Self {
        Self {
            runtime_stream: RuntimeStreamConfig::default(),
            terminal_wait_timeout: Duration::from_secs(300),
            finished_drain_timeout: Duration::from_millis(100),
        }
    }
}

impl IssueSessionRunnerConfig {
    pub fn from_workflow(workflow: &ResolvedWorkflow) -> Self {
        let websocket = &workflow.extensions.openhands.websocket;
        Self {
            runtime_stream: RuntimeStreamConfig {
                readiness_timeout: Duration::from_millis(websocket.ready_timeout_ms),
                reconnect_initial_backoff: Duration::from_millis(websocket.reconnect_initial_ms),
                reconnect_max_backoff: Duration::from_millis(websocket.reconnect_max_ms),
                ..RuntimeStreamConfig::default()
            },
            terminal_wait_timeout: Duration::from_millis(
                workflow.config.agent.stall_timeout_ms.unwrap_or(300_000),
            ),
            finished_drain_timeout: Duration::from_millis(100),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSessionPromptKind {
    Full,
    Continuation,
}

impl IssueSessionPromptKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Continuation => "continuation",
        }
    }

    fn artifact_name(self) -> &'static str {
        match self {
            Self::Full => "last-full-prompt.md",
            Self::Continuation => "last-continuation-prompt.md",
        }
    }

    fn artifact_path(self, workspace: &WorkspaceHandle) -> PathBuf {
        workspace.prompts_dir().join(self.artifact_name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationLaunchProfile {
    pub workspace_kind: String,
    pub confirmation_policy_kind: String,
    pub agent_kind: String,
    pub llm_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condenser: Option<ConversationLaunchCondenserProfile>,
    pub agent_tools: Option<Vec<ToolConfig>>,
    pub agent_include_default_tools: Option<Vec<String>>,
    pub max_iterations: u32,
    pub stuck_detection: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_stdio_servers: Vec<McpStdioServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationLaunchCondenserProfile {
    pub max_size: u64,
    pub keep_first: u64,
}

impl ConversationLaunchProfile {
    pub fn from_workflow(workflow: &ResolvedWorkflow) -> Result<Self, String> {
        let conversation = &workflow.extensions.openhands.conversation;
        let max_iterations = u32::try_from(conversation.max_iterations).map_err(|_| {
            format!(
                "workflow max_iterations {} exceeds u32::MAX ({})",
                conversation.max_iterations,
                u32::MAX
            )
        })?;
        let llm_model = conversation
            .agent
            .llm
            .as_ref()
            .and_then(|llm| llm.model.as_ref())
            .cloned()
            .ok_or_else(|| {
                "workflow openhands.conversation.agent.llm.model is required".to_string()
            })?;

        Ok(Self {
            workspace_kind: "LocalWorkspace".to_string(),
            confirmation_policy_kind: conversation.confirmation_policy.kind.clone(),
            agent_kind: conversation.agent.kind.clone(),
            llm_model,
            condenser: conversation.agent.condenser.as_ref().map(|condenser| {
                ConversationLaunchCondenserProfile {
                    max_size: condenser.max_size,
                    keep_first: condenser.keep_first,
                }
            }),
            agent_tools: conversation
                .agent
                .tools
                .as_ref()
                .map(|tools| tools.iter().map(tool_config_from_workflow).collect()),
            agent_include_default_tools: conversation.agent.include_default_tools.clone(),
            max_iterations,
            stuck_detection: conversation.stuck_detection,
            mcp_stdio_servers: workflow
                .extensions
                .openhands
                .mcp
                .stdio_servers
                .iter()
                .map(launch_profile_stdio_server)
                .collect(),
        })
    }

    pub fn to_create_request(
        &self,
        working_dir: &Path,
        persistence_dir: &Path,
        conversation_id: Option<Uuid>,
    ) -> ConversationCreateRequest {
        let llm = LlmConfig {
            model: self.llm_model.clone(),
            api_key: std::env::var("LLM_API_KEY").ok(),
            base_url: std::env::var("LLM_BASE_URL").ok(),
            usage_id: None,
        };

        ConversationCreateRequest {
            conversation_id: conversation_id.unwrap_or_else(Uuid::new_v4),
            workspace: WorkspaceConfig {
                working_dir: working_dir.display().to_string(),
                kind: self.workspace_kind.clone(),
            },
            persistence_dir: persistence_dir.display().to_string(),
            max_iterations: self.max_iterations,
            stuck_detection: self.stuck_detection,
            confirmation_policy: ConfirmationPolicy {
                kind: self.confirmation_policy_kind.clone(),
            },
            agent: AgentConfig {
                kind: self.agent_kind.clone(),
                llm: llm.clone(),
                condenser: self.condenser.as_ref().map(|condenser| {
                    CondenserConfig::llm_summarizing(
                        llm.clone(),
                        condenser.max_size,
                        condenser.keep_first,
                    )
                }),
                tools: self.agent_tools.clone(),
                include_default_tools: self.agent_include_default_tools.clone(),
            },
            mcp_config: McpConfig::from_stdio_servers(self.mcp_stdio_servers.clone()),
        }
    }
}

fn launch_profile_stdio_server(
    server: &opensymphony_workflow::OpenHandsStdioServerConfig,
) -> McpStdioServerConfig {
    let (command, args) = server
        .command
        .split_first()
        .expect("workflow stdio server commands should be validated during resolution");
    McpStdioServerConfig {
        name: server.name.clone(),
        command: command.clone(),
        args: args.to_vec(),
        env: Default::default(),
    }
}

fn tool_config_from_workflow(tool: &OpenHandsConversationToolConfig) -> ToolConfig {
    ToolConfig {
        name: tool.name.clone(),
        params: tool.params.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueConversationManifest {
    pub issue_id: IssueId,
    pub identifier: IssueIdentifier,
    pub conversation_id: ConversationId,
    pub server_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_query_param_name: Option<String>,
    pub persistence_dir: PathBuf,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_attached_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_profile: Option<ConversationLaunchProfile>,
    pub fresh_conversation: bool,
    pub workflow_prompt_seeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_contract_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_kind: Option<IssueSessionPromptKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_prompt_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_execution_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_summary: Option<String>,
}

impl IssueConversationManifest {
    fn new(
        issue_id: IssueId,
        identifier: IssueIdentifier,
        conversation_id: ConversationId,
        persistence_dir: PathBuf,
        attached_at: DateTime<Utc>,
        reset_reason: Option<String>,
        launch_profile: ConversationLaunchProfile,
    ) -> Self {
        Self {
            issue_id,
            identifier,
            conversation_id,
            server_base_url: None,
            transport_target: None,
            http_auth_mode: None,
            websocket_auth_mode: None,
            websocket_query_param_name: None,
            persistence_dir,
            created_at: attached_at,
            updated_at: attached_at,
            last_attached_at: attached_at,
            launch_profile: Some(launch_profile),
            fresh_conversation: true,
            workflow_prompt_seeded: false,
            reset_reason,
            runtime_contract_version: Some(RUNTIME_CONTRACT_VERSION.to_string()),
            last_prompt_kind: None,
            last_prompt_at: None,
            last_prompt_path: None,
            last_execution_status: None,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
        }
    }

    fn prompt_kind(&self) -> IssueSessionPromptKind {
        if self.workflow_prompt_seeded {
            IssueSessionPromptKind::Continuation
        } else {
            IssueSessionPromptKind::Full
        }
    }

    fn is_reusable_for(&self, issue: &NormalizedIssue, expected_persistence_dir: &Path) -> bool {
        self.issue_id == issue.id
            && self.identifier == issue.identifier
            && self.persistence_dir == expected_persistence_dir
            && self.runtime_contract_version.as_deref() == Some(RUNTIME_CONTRACT_VERSION)
    }

    fn record_prompt(
        &mut self,
        prompt_kind: IssueSessionPromptKind,
        prompt_path: PathBuf,
        recorded_at: DateTime<Utc>,
    ) {
        self.last_prompt_kind = Some(prompt_kind);
        self.last_prompt_at = Some(recorded_at);
        self.last_prompt_path = Some(prompt_path);
        self.updated_at = recorded_at;
    }

    fn apply_runtime_snapshot(&mut self, stream: &RuntimeEventStream) {
        self.last_execution_status = stream
            .state_mirror()
            .execution_status()
            .map(ToOwned::to_owned);

        if let Some(event) = stream.event_cache().items().last() {
            self.last_event_id = Some(event.id.clone());
            self.last_event_kind = Some(event.kind.clone());
            self.last_event_at = Some(event.timestamp);
            self.last_event_summary = Some(summarize_event(event));
        }

        self.updated_at = Utc::now();
    }

    fn apply_transport_diagnostics(
        &mut self,
        diagnostics: Option<&crate::TransportDiagnostics>,
        server_base_url: &str,
    ) {
        self.server_base_url = Some(server_base_url.to_string());
        self.transport_target =
            diagnostics.map(|diagnostics| diagnostics.target_kind.as_str().to_string());
        self.http_auth_mode =
            diagnostics.map(|diagnostics| diagnostics.http_auth_kind.as_str().to_string());
        self.websocket_auth_mode =
            diagnostics.map(|diagnostics| diagnostics.websocket_auth_kind.as_str().to_string());
        self.websocket_query_param_name =
            diagnostics.and_then(|diagnostics| diagnostics.websocket_query_param_name.clone());
    }

    fn to_domain_metadata(&self, stream_state: RuntimeStreamState) -> ConversationMetadata {
        ConversationMetadata {
            conversation_id: self.conversation_id.clone(),
            server_base_url: self.server_base_url.clone(),
            transport_target: self.transport_target.clone(),
            http_auth_mode: self.http_auth_mode.clone(),
            websocket_auth_mode: self.websocket_auth_mode.clone(),
            websocket_query_param_name: self.websocket_query_param_name.clone(),
            fresh_conversation: self.fresh_conversation,
            runtime_contract_version: self.runtime_contract_version.clone(),
            stream_state,
            last_event_id: self.last_event_id.clone(),
            last_event_kind: self.last_event_kind.clone(),
            last_event_at: self.last_event_at.map(timestamp_ms_from_datetime),
            last_event_summary: self.last_event_summary.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSessionContext {
    pub run_id: String,
    pub issue_id: IssueId,
    pub identifier: IssueIdentifier,
    pub worker_id: WorkerId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    pub normal_retry_count: u32,
    pub turn_count: u32,
    pub max_turns: u32,
    pub prompt_kind: IssueSessionPromptKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_path: Option<PathBuf>,
    pub conversation_id: ConversationId,
    pub fresh_conversation: bool,
    pub workflow_prompt_seeded: bool,
    pub server_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_auth_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_query_param_name: Option<String>,
    pub persistence_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_execution_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_outcome: Option<WorkerOutcomeRecord>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSessionResult {
    pub prompt_kind: IssueSessionPromptKind,
    pub conversation: Option<ConversationMetadata>,
    pub worker_outcome: WorkerOutcomeRecord,
    pub run_status: RunStatus,
}

#[derive(Debug, Error)]
pub enum IssueSessionError {
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
}

#[derive(Debug, Clone)]
struct NormalizedOutcome {
    kind: WorkerOutcomeKind,
    summary: String,
    error: Option<String>,
}

struct ActiveSession {
    stream: RuntimeEventStream,
    manifest: IssueConversationManifest,
    prompt_kind: IssueSessionPromptKind,
    prompt_path: Option<PathBuf>,
}

enum Step<T> {
    Continue(T),
    EarlyResult(Box<IssueSessionResult>),
}

enum ReuseSession {
    Active(Box<ActiveSession>),
    Reset(String),
}

struct PreparedTurn {
    conversation_id: Uuid,
    prompt: String,
    baseline_event_ids: HashSet<String>,
    waited_for_prior_turn: bool,
}

#[derive(Default)]
struct LoadedManifest {
    manifest: Option<IssueConversationManifest>,
    reset_reason: Option<String>,
}

pub struct IssueSessionRunner {
    client: OpenHandsClient,
    config: IssueSessionRunnerConfig,
}

impl IssueSessionRunner {
    pub fn new(client: OpenHandsClient, config: IssueSessionRunnerConfig) -> Self {
        Self { client, config }
    }

    pub fn client(&self) -> &OpenHandsClient {
        &self.client
    }

    pub fn config(&self) -> &IssueSessionRunnerConfig {
        &self.config
    }

    pub async fn run(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        issue: &NormalizedIssue,
        run: &RunAttempt,
        workflow: &ResolvedWorkflow,
    ) -> Result<IssueSessionResult, IssueSessionError> {
        self.run_with_observer(
            workspace_manager,
            workspace,
            run_manifest,
            issue,
            run,
            workflow,
            &mut (),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_with_observer<O>(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        issue: &NormalizedIssue,
        run: &RunAttempt,
        workflow: &ResolvedWorkflow,
        observer: &mut O,
    ) -> Result<IssueSessionResult, IssueSessionError>
    where
        O: IssueSessionObserver,
    {
        let observed_run = observed_run_for_turn(run);
        let active_session = match self
            .initialize_session(
                workspace_manager,
                workspace,
                run_manifest,
                &observed_run,
                issue,
                workflow,
            )
            .await?
        {
            Step::Continue(session) => session,
            Step::EarlyResult(result) => return Ok(*result),
        };

        let (mut active_session, mut prepared_turn) = match self
            .prepare_turn(
                workspace_manager,
                workspace,
                run_manifest,
                &observed_run,
                workflow,
                issue,
                run,
                active_session,
                observer,
            )
            .await?
        {
            Step::Continue(state) => state,
            Step::EarlyResult(result) => return Ok(*result),
        };

        active_session = match self
            .start_turn(
                workspace_manager,
                workspace,
                run_manifest,
                &observed_run,
                active_session,
                &mut prepared_turn,
                observer,
            )
            .await?
        {
            Step::Continue(session) => session,
            Step::EarlyResult(result) => return Ok(*result),
        };

        let outcome = self
            .await_terminal_outcome(
                &mut active_session.stream,
                &prepared_turn.baseline_event_ids,
                observer,
            )
            .await;
        self.finalize_active_session(
            workspace_manager,
            workspace,
            run_manifest,
            &observed_run,
            active_session,
            outcome,
        )
        .await
    }

    async fn initialize_session(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
    ) -> Result<Step<ActiveSession>, IssueSessionError> {
        let loaded = self
            .load_existing_conversation_manifest(workspace_manager, workspace, issue, workflow)
            .await?;

        match loaded.manifest {
            Some(manifest) => match self
                .try_reuse_session(workspace_manager, workspace, issue, workflow, manifest)
                .await?
            {
                ReuseSession::Active(session) => Ok(Step::Continue(*session)),
                ReuseSession::Reset(reason) => {
                    self.create_fresh_session(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        issue,
                        workflow,
                        Some(reason),
                    )
                    .await
                }
            },
            None => {
                self.create_fresh_session(
                    workspace_manager,
                    workspace,
                    run_manifest,
                    observed_run,
                    issue,
                    workflow,
                    loaded.reset_reason,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_turn<O>(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        workflow: &ResolvedWorkflow,
        issue: &NormalizedIssue,
        run: &RunAttempt,
        mut active_session: ActiveSession,
        observer: &mut O,
    ) -> Result<Step<(ActiveSession, PreparedTurn)>, IssueSessionError>
    where
        O: IssueSessionObserver,
    {
        let mut waited_for_prior_turn = false;
        if let Some(status) = active_session.stream.state_mirror().execution_status()
            && turn_is_in_progress(status)
        {
            if let Err(error) = self
                .wait_for_active_turn_to_finish(&mut active_session.stream, observer)
                .await
            {
                return self
                    .finalize_active_session(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        active_session,
                        failed_outcome(
                            "previous OpenHands turn did not finish before retrying",
                            error.to_string(),
                        ),
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
            waited_for_prior_turn = true;
            active_session
                .manifest
                .apply_runtime_snapshot(&active_session.stream);
        }

        let prompt = match self.render_prompt(workflow, issue, run, active_session.prompt_kind) {
            Ok(prompt) => prompt,
            Err(detail) => {
                let summary = format!(
                    "failed to render {} prompt",
                    active_session.prompt_kind.as_str()
                );
                return self
                    .finalize_active_session(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        active_session,
                        failed_outcome(summary, detail),
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
        };

        let prompt_path = active_session.prompt_kind.artifact_path(workspace);
        workspace_manager
            .write_text_artifact(workspace, &prompt_path, &prompt)
            .await?;
        let prompt_recorded_at = Utc::now();
        active_session.manifest.record_prompt(
            active_session.prompt_kind,
            prompt_path.clone(),
            prompt_recorded_at,
        );
        active_session.prompt_path = Some(prompt_path);
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &active_session.manifest,
            )
            .await?;

        let conversation_id = match parse_uuid(active_session.manifest.conversation_id.as_str()) {
            Ok(conversation_id) => conversation_id,
            Err(detail) => {
                return self
                    .finalize_active_session(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        active_session,
                        failed_outcome(
                            "conversation manifest contained an invalid conversation ID",
                            detail,
                        ),
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
        };

        let baseline_event_ids = active_session
            .stream
            .event_cache()
            .items()
            .iter()
            .map(|event| event.id.clone())
            .collect::<HashSet<_>>();

        Ok(Step::Continue((
            active_session,
            PreparedTurn {
                conversation_id,
                prompt,
                baseline_event_ids,
                waited_for_prior_turn,
            },
        )))
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_turn<O>(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        mut active_session: ActiveSession,
        prepared_turn: &mut PreparedTurn,
        observer: &mut O,
    ) -> Result<Step<ActiveSession>, IssueSessionError>
    where
        O: IssueSessionObserver,
    {
        if let Err(error) = self
            .client
            .send_message(
                prepared_turn.conversation_id,
                &SendMessageRequest::user_text(prepared_turn.prompt.clone()),
            )
            .await
        {
            let summary = format!(
                "failed to send {} prompt event",
                active_session.prompt_kind.as_str()
            );
            return self
                .finalize_active_session(
                    workspace_manager,
                    workspace,
                    run_manifest,
                    observed_run,
                    active_session,
                    failed_outcome(summary, error.to_string()),
                )
                .await
                .map(Box::new)
                .map(Step::EarlyResult);
        }

        if active_session.prompt_kind == IssueSessionPromptKind::Full {
            active_session.manifest.workflow_prompt_seeded = true;
        }
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &active_session.manifest,
            )
            .await?;

        let mut had_run_conflict = false;
        loop {
            match self
                .client
                .run_conversation(prepared_turn.conversation_id)
                .await
            {
                Ok(_) => break,
                Err(OpenHandsError::HttpStatus {
                    status_code: 409, ..
                }) => {
                    had_run_conflict = true;
                    if let Err(error) = self
                        .wait_for_active_turn_to_finish(&mut active_session.stream, observer)
                        .await
                    {
                        return self
                            .finalize_active_session(
                                workspace_manager,
                                workspace,
                                run_manifest,
                                observed_run,
                                active_session,
                                failed_outcome(
                                    "previous OpenHands turn did not finish after run retry conflict",
                                    error.to_string(),
                                ),
                            )
                            .await
                            .map(Box::new)
                            .map(Step::EarlyResult);
                    }
                    active_session
                        .manifest
                        .apply_runtime_snapshot(&active_session.stream);
                    prepared_turn.baseline_event_ids.extend(
                        active_session
                            .stream
                            .event_cache()
                            .items()
                            .iter()
                            .map(|event| event.id.clone()),
                    );
                }
                Err(error) => {
                    return self
                        .finalize_active_session(
                            workspace_manager,
                            workspace,
                            run_manifest,
                            observed_run,
                            active_session,
                            failed_outcome("failed to trigger OpenHands run", error.to_string()),
                        )
                        .await
                        .map(Box::new)
                        .map(Step::EarlyResult);
                }
            }
        }
        if (prepared_turn.waited_for_prior_turn || had_run_conflict)
            && let Err(error) = active_session.stream.reconcile_events().await
        {
            debug!(
                %error,
                conversation_id = %active_session.manifest.conversation_id,
                "post-conflict reconcile failed, proceeding anyway"
            );
        }

        run_manifest.status = RunStatus::Running;
        run_manifest.status_detail = Some(format!(
            "{} prompt sent to conversation {}",
            active_session.prompt_kind.as_str(),
            active_session.manifest.conversation_id
        ));
        workspace_manager
            .write_run_manifest(workspace, run_manifest)
            .await?;
        workspace_manager
            .write_json_artifact(
                workspace,
                &session_context_path(workspace),
                &build_session_context(
                    run_manifest,
                    observed_run,
                    &active_session.manifest,
                    active_session.prompt_kind,
                    active_session.prompt_path.clone(),
                    None,
                ),
            )
            .await?;

        observer.on_launch(
            &active_session
                .manifest
                .to_domain_metadata(RuntimeStreamState::Ready),
        );

        Ok(Step::Continue(active_session))
    }

    async fn load_existing_conversation_manifest(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
    ) -> Result<LoadedManifest, IssueSessionError> {
        let Some(raw) = workspace_manager
            .read_text_artifact(workspace, &workspace.conversation_manifest_path())
            .await?
        else {
            return Ok(LoadedManifest::default());
        };

        let manifest = match serde_json::from_str::<IssueConversationManifest>(&raw) {
            Ok(manifest) => manifest,
            Err(error) => {
                return Ok(LoadedManifest {
                    manifest: None,
                    reset_reason: Some(format!("invalid conversation manifest: {error}")),
                });
            }
        };

        let expected_persistence_dir = configured_persistence_dir(workflow, workspace);
        if !manifest.is_reusable_for(issue, &expected_persistence_dir) {
            return Ok(LoadedManifest {
                manifest: None,
                reset_reason: Some(format!(
                    "conversation manifest is incompatible with issue {} or the current workspace",
                    issue.identifier
                )),
            });
        }

        Ok(LoadedManifest {
            manifest: Some(manifest),
            reset_reason: None,
        })
    }

    async fn try_reuse_session(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
        mut manifest: IssueConversationManifest,
    ) -> Result<ReuseSession, IssueSessionError> {
        let manifest_conversation_id = manifest.conversation_id.clone();
        let conversation_id = match parse_uuid(manifest.conversation_id.as_str()) {
            Ok(conversation_id) => conversation_id,
            Err(error) => return Ok(ReuseSession::Reset(error)),
        };
        let stream = match self
            .client
            .attach_runtime_stream(conversation_id, self.config.runtime_stream.clone())
            .await
        {
            Ok(stream) => stream,
            Err(error) => match self
                .rehydrate_existing_session(
                    workspace_manager,
                    workspace,
                    issue,
                    workflow,
                    manifest,
                    conversation_id,
                )
                .await?
            {
                Some(session) => return Ok(ReuseSession::Active(Box::new(session))),
                None => {
                    return Ok(ReuseSession::Reset(format!(
                        "failed to attach existing conversation {}: {error}",
                        manifest_conversation_id
                    )));
                }
            },
        };

        let attached_at = Utc::now();
        manifest.fresh_conversation = false;
        if manifest.launch_profile.is_none() {
            manifest.launch_profile = ConversationLaunchProfile::from_workflow(workflow).ok();
        }
        let transport_diagnostics = self.client.transport_diagnostics().ok();
        manifest
            .apply_transport_diagnostics(transport_diagnostics.as_ref(), self.client.base_url());
        manifest.runtime_contract_version = Some(RUNTIME_CONTRACT_VERSION.to_string());
        manifest.last_attached_at = attached_at;
        manifest.updated_at = attached_at;
        manifest.reset_reason = None;
        manifest.apply_runtime_snapshot(&stream);
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &manifest,
            )
            .await?;
        workspace_manager
            .write_json_artifact(
                workspace,
                &last_conversation_state_path(workspace),
                &conversation_snapshot(&stream),
            )
            .await?;

        Ok(ReuseSession::Active(Box::new(ActiveSession {
            prompt_kind: manifest.prompt_kind(),
            stream,
            manifest,
            prompt_path: None,
        })))
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_fresh_session(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
        reset_reason: Option<String>,
    ) -> Result<Step<ActiveSession>, IssueSessionError> {
        let launch_profile = match ConversationLaunchProfile::from_workflow(workflow) {
            Ok(launch_profile) => launch_profile,
            Err(detail) => {
                return self
                    .persist_failure_without_stream(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        IssueSessionPromptKind::Full,
                        None,
                        NormalizedOutcome {
                            kind: WorkerOutcomeKind::Failed,
                            summary: "failed to build conversation launch profile".to_string(),
                            error: Some(detail),
                        },
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
        };
        let request = launch_profile.to_create_request(
            workspace.workspace_path(),
            &configured_persistence_dir(workflow, workspace),
            None,
        );
        workspace_manager
            .write_json_artifact(
                workspace,
                &create_conversation_request_path(workspace),
                &request,
            )
            .await?;

        let conversation = match self.client.create_conversation(&request).await {
            Ok(conversation) => conversation,
            Err(error) => {
                return self
                    .persist_failure_without_stream(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        IssueSessionPromptKind::Full,
                        None,
                        NormalizedOutcome {
                            kind: WorkerOutcomeKind::Failed,
                            summary: "failed to create OpenHands conversation".to_string(),
                            error: Some(error.to_string()),
                        },
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
        };

        let stream = match self
            .client
            .attach_runtime_stream(
                conversation.conversation_id,
                self.config.runtime_stream.clone(),
            )
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                return self
                    .persist_failure_without_stream(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        observed_run,
                        IssueSessionPromptKind::Full,
                        Some(build_summary_metadata(
                            &conversation,
                            true,
                            RuntimeStreamState::Failed,
                            self.client.transport_diagnostics().ok().as_ref(),
                            self.client.base_url(),
                        )),
                        NormalizedOutcome {
                            kind: WorkerOutcomeKind::Failed,
                            summary: "failed to attach runtime stream for a fresh conversation"
                                .to_string(),
                            error: Some(error.to_string()),
                        },
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
        };

        let attached_at = Utc::now();
        let mut manifest = IssueConversationManifest::new(
            issue.id.clone(),
            issue.identifier.clone(),
            ConversationId::new(conversation.conversation_id.to_string())
                .expect("UUID-backed conversation ID should not be empty"),
            configured_persistence_dir(workflow, workspace),
            attached_at,
            reset_reason,
            launch_profile,
        );
        let transport_diagnostics = self.client.transport_diagnostics().ok();
        manifest
            .apply_transport_diagnostics(transport_diagnostics.as_ref(), self.client.base_url());
        manifest.apply_runtime_snapshot(&stream);
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &manifest,
            )
            .await?;
        workspace_manager
            .write_json_artifact(
                workspace,
                &last_conversation_state_path(workspace),
                &conversation_snapshot(&stream),
            )
            .await?;

        Ok(Step::Continue(ActiveSession {
            prompt_kind: manifest.prompt_kind(),
            stream,
            manifest,
            prompt_path: None,
        }))
    }

    async fn rehydrate_existing_session(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
        mut manifest: IssueConversationManifest,
        conversation_id: Uuid,
    ) -> Result<Option<ActiveSession>, IssueSessionError> {
        let Some(launch_profile) = manifest
            .launch_profile
            .clone()
            .or_else(|| ConversationLaunchProfile::from_workflow(workflow).ok())
        else {
            debug!(
                %conversation_id,
                "skipping session rehydrate because no launch profile is available"
            );
            return Ok(None);
        };
        let request = launch_profile.to_create_request(
            workspace.workspace_path(),
            &configured_persistence_dir(workflow, workspace),
            Some(conversation_id),
        );
        workspace_manager
            .write_json_artifact(
                workspace,
                &create_conversation_request_path(workspace),
                &request,
            )
            .await?;

        let conversation = match self.client.create_conversation(&request).await {
            Ok(conversation) => conversation,
            Err(error) => {
                debug!(
                    %error,
                    %conversation_id,
                    "skipping session rehydrate because conversation recreation failed"
                );
                return Ok(None);
            }
        };
        if conversation.conversation_id != conversation_id {
            debug!(
                %conversation_id,
                recreated_conversation_id = %conversation.conversation_id,
                "skipping session rehydrate because recreated conversation id changed"
            );
            return Ok(None);
        }

        let stream = match self
            .client
            .attach_runtime_stream(conversation_id, self.config.runtime_stream.clone())
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                debug!(
                    %error,
                    %conversation_id,
                    "skipping session rehydrate because runtime stream attach failed"
                );
                return Ok(None);
            }
        };
        if !stream_has_persisted_history(&stream) {
            debug!(
                %conversation_id,
                "skipping session rehydrate because recreated conversation has no persisted history"
            );
            return Ok(None);
        }

        let attached_at = Utc::now();
        manifest.issue_id = issue.id.clone();
        manifest.identifier = issue.identifier.clone();
        manifest.fresh_conversation = false;
        manifest.workflow_prompt_seeded = true;
        manifest.server_base_url = Some(self.client.base_url().to_string());
        manifest.persistence_dir = configured_persistence_dir(workflow, workspace);
        manifest.last_attached_at = attached_at;
        manifest.updated_at = attached_at;
        manifest.launch_profile.get_or_insert(launch_profile);
        manifest.reset_reason = None;
        manifest.runtime_contract_version = Some(RUNTIME_CONTRACT_VERSION.to_string());
        manifest.apply_runtime_snapshot(&stream);
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &manifest,
            )
            .await?;
        workspace_manager
            .write_json_artifact(
                workspace,
                &last_conversation_state_path(workspace),
                &conversation_snapshot(&stream),
            )
            .await?;

        Ok(Some(ActiveSession {
            prompt_kind: manifest.prompt_kind(),
            stream,
            manifest,
            prompt_path: None,
        }))
    }

    fn render_prompt(
        &self,
        workflow: &ResolvedWorkflow,
        issue: &NormalizedIssue,
        run: &RunAttempt,
        prompt_kind: IssueSessionPromptKind,
    ) -> Result<String, String> {
        match prompt_kind {
            IssueSessionPromptKind::Full => workflow
                .render_prompt(issue, run.attempt.map(|attempt| attempt.get()))
                .map_err(|error| error.to_string()),
            IssueSessionPromptKind::Continuation => Ok(build_continuation_guidance(issue, run)),
        }
    }

    async fn wait_for_active_turn_to_finish<O>(
        &self,
        stream: &mut RuntimeEventStream,
        observer: &mut O,
    ) -> Result<(), OpenHandsError>
    where
        O: IssueSessionObserver,
    {
        if stream
            .state_mirror()
            .execution_status()
            .is_none_or(turn_has_stopped)
        {
            return Ok(());
        }

        let deadline = Instant::now() + self.config.terminal_wait_timeout;
        loop {
            if stream
                .state_mirror()
                .execution_status()
                .is_some_and(turn_has_stopped)
            {
                return Ok(());
            }

            match timeout_at(deadline, stream.next_event()).await {
                Err(_) => {
                    let status = stream
                        .state_mirror()
                        .execution_status()
                        .unwrap_or("unknown");
                    return Err(OpenHandsError::Protocol {
                        operation: "wait for active turn to finish",
                        detail: format!(
                            "execution_status `{status}` did not stop within {} ms",
                            self.config.terminal_wait_timeout.as_millis()
                        ),
                    });
                }
                Ok(Ok(Some(event))) => observe_event(observer, &event),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    if stream
                        .state_mirror()
                        .execution_status()
                        .is_some_and(turn_has_stopped)
                        && finished_stream_error_is_tolerable(&error)
                    {
                        return Ok(());
                    }
                    return Err(error);
                }
            }
        }
    }

    async fn await_terminal_outcome<O>(
        &self,
        stream: &mut RuntimeEventStream,
        baseline_event_ids: &HashSet<String>,
        observer: &mut O,
    ) -> NormalizedOutcome
    where
        O: IssueSessionObserver,
    {
        let deadline = Instant::now() + self.config.terminal_wait_timeout;

        loop {
            if let Some(outcome) = self
                .terminal_outcome_from_state(stream, baseline_event_ids, observer)
                .await
            {
                return outcome;
            }

            let next_event = timeout_at(deadline, stream.next_event()).await;
            match next_event {
                Err(_) => {
                    if let Ok(inserted) = stream.reconcile_events().await
                        && inserted > 0
                    {
                        observe_latest_event(observer, stream);
                        if let Some(outcome) = self
                            .terminal_outcome_from_state(stream, baseline_event_ids, observer)
                            .await
                        {
                            return outcome;
                        }
                        continue;
                    }
                    return NormalizedOutcome {
                        kind: WorkerOutcomeKind::Stalled,
                        summary: "runtime did not reach a terminal state before the stall timeout"
                            .to_string(),
                        error: Some(format!(
                            "no terminal runtime state was observed within {} ms",
                            self.config.terminal_wait_timeout.as_millis()
                        )),
                    };
                }
                Ok(Ok(Some(event))) => observe_event(observer, &event),
                Ok(Ok(None)) => {
                    if let Ok(inserted) = stream.reconcile_events().await
                        && inserted > 0
                    {
                        observe_latest_event(observer, stream);
                        if let Some(outcome) = self
                            .terminal_outcome_from_state(stream, baseline_event_ids, observer)
                            .await
                        {
                            return outcome;
                        }
                    }

                    return NormalizedOutcome {
                        kind: WorkerOutcomeKind::Failed,
                        summary: "runtime event stream ended before terminal status".to_string(),
                        error: Some(
                            "runtime event stream closed before a terminal state was observed"
                                .to_string(),
                        ),
                    };
                }
                Ok(Err(error)) => {
                    if let Some(outcome) = self
                        .terminal_outcome_from_state(stream, baseline_event_ids, observer)
                        .await
                    {
                        return outcome;
                    }

                    return NormalizedOutcome {
                        kind: WorkerOutcomeKind::Failed,
                        summary: "runtime event stream failed before terminal status".to_string(),
                        error: Some(error.to_string()),
                    };
                }
            }
        }
    }

    async fn terminal_outcome_from_state<O>(
        &self,
        stream: &mut RuntimeEventStream,
        baseline_event_ids: &HashSet<String>,
        observer: &mut O,
    ) -> Option<NormalizedOutcome>
    where
        O: IssueSessionObserver,
    {
        let has_current_turn_activity = stream
            .event_cache()
            .items()
            .iter()
            .any(|event| !baseline_event_ids.contains(&event.id));
        if !has_current_turn_activity {
            return None;
        }

        if let Some(error_detail) =
            latest_current_turn_error(stream.event_cache().items(), baseline_event_ids)
        {
            return Some(NormalizedOutcome {
                kind: WorkerOutcomeKind::Failed,
                summary: "received ConversationErrorEvent during the current run".to_string(),
                error: Some(error_detail),
            });
        }

        match stream.state_mirror().terminal_status() {
            Some(TerminalExecutionStatus::Finished) => {
                if self
                    .confirm_finished_terminal_state(stream, baseline_event_ids, observer)
                    .await
                {
                    Some(NormalizedOutcome {
                        kind: WorkerOutcomeKind::Succeeded,
                        summary: "OpenHands execution_status `finished`".to_string(),
                        error: None,
                    })
                } else {
                    None
                }
            }
            Some(TerminalExecutionStatus::Error) => Some(NormalizedOutcome {
                kind: WorkerOutcomeKind::Failed,
                summary: "OpenHands execution_status `error`".to_string(),
                error: Some(
                    stream
                        .state_mirror()
                        .execution_status()
                        .unwrap_or_default()
                        .to_string(),
                ),
            }),
            Some(TerminalExecutionStatus::Stuck) => Some(NormalizedOutcome {
                kind: WorkerOutcomeKind::Stalled,
                summary: "OpenHands execution_status `stuck`".to_string(),
                error: Some(
                    stream
                        .state_mirror()
                        .execution_status()
                        .unwrap_or_default()
                        .to_string(),
                ),
            }),
            None => None,
        }
    }

    async fn confirm_finished_terminal_state<O>(
        &self,
        stream: &mut RuntimeEventStream,
        baseline_event_ids: &HashSet<String>,
        observer: &mut O,
    ) -> bool
    where
        O: IssueSessionObserver,
    {
        let deadline = Instant::now() + self.config.finished_drain_timeout;

        loop {
            if latest_current_turn_error(stream.event_cache().items(), baseline_event_ids).is_some()
            {
                return false;
            }
            if !matches!(
                stream.state_mirror().terminal_status(),
                Some(TerminalExecutionStatus::Finished)
            ) {
                return false;
            }

            match timeout_at(deadline, stream.next_event()).await {
                Err(_) => return true,
                Ok(Ok(Some(event))) => {
                    observe_event(observer, &event);
                    continue;
                }
                Ok(Ok(None)) => return true,
                Ok(Err(error)) => return finished_stream_error_is_tolerable(&error),
            }
        }
    }

    async fn finalize_active_session(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        mut session: ActiveSession,
        outcome: NormalizedOutcome,
    ) -> Result<IssueSessionResult, IssueSessionError> {
        session.manifest.apply_runtime_snapshot(&session.stream);
        workspace_manager
            .write_json_artifact(
                workspace,
                &last_conversation_state_path(workspace),
                &conversation_snapshot(&session.stream),
            )
            .await?;

        let run_status = run_status_for(outcome.kind);
        run_manifest.status = run_status;
        run_manifest.status_detail = Some(
            outcome
                .error
                .clone()
                .unwrap_or_else(|| outcome.summary.clone()),
        );
        workspace_manager
            .finish_run(workspace, run_manifest, run_status)
            .await?;

        let worker_outcome = WorkerOutcomeRecord::from_run(
            observed_run,
            outcome.kind,
            timestamp_ms_from_datetime(Utc::now()),
            Some(outcome.summary.clone()),
            outcome.error.clone(),
        );

        workspace_manager
            .write_json_artifact(
                workspace,
                &session_context_path(workspace),
                &build_session_context(
                    run_manifest,
                    observed_run,
                    &session.manifest,
                    session.prompt_kind,
                    session.prompt_path.clone(),
                    Some(worker_outcome.clone()),
                ),
            )
            .await?;
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &session.manifest,
            )
            .await?;

        let conversation = session
            .manifest
            .to_domain_metadata(RuntimeStreamState::Closed);
        let _ = session.stream.close().await;

        Ok(IssueSessionResult {
            prompt_kind: session.prompt_kind,
            conversation: Some(conversation),
            worker_outcome,
            run_status,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn persist_failure_without_stream(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        prompt_kind: IssueSessionPromptKind,
        conversation: Option<ConversationMetadata>,
        outcome: NormalizedOutcome,
    ) -> Result<IssueSessionResult, IssueSessionError> {
        let run_status = run_status_for(outcome.kind);
        run_manifest.status = run_status;
        run_manifest.status_detail = Some(
            outcome
                .error
                .clone()
                .unwrap_or_else(|| outcome.summary.clone()),
        );
        workspace_manager
            .finish_run(workspace, run_manifest, run_status)
            .await?;

        let worker_outcome = WorkerOutcomeRecord::from_run(
            observed_run,
            outcome.kind,
            timestamp_ms_from_datetime(Utc::now()),
            Some(outcome.summary),
            outcome.error,
        );

        Ok(IssueSessionResult {
            prompt_kind,
            conversation,
            worker_outcome,
            run_status,
        })
    }
}

fn configured_persistence_dir(workflow: &ResolvedWorkflow, workspace: &WorkspaceHandle) -> PathBuf {
    workspace.workspace_path().join(
        &workflow
            .extensions
            .openhands
            .conversation
            .persistence_dir_relative,
    )
}

fn turn_is_in_progress(status: &str) -> bool {
    !matches!(status, "idle" | "finished" | "error" | "stuck")
}

fn turn_has_stopped(status: &str) -> bool {
    !turn_is_in_progress(status)
}

fn stream_has_persisted_history(stream: &RuntimeEventStream) -> bool {
    // Fresh conversations emit only the initial state snapshot, while rehydrated
    // conversations replay that snapshot plus prior history.
    stream.event_cache().items().len() > 1
}

fn build_continuation_guidance(issue: &NormalizedIssue, run: &RunAttempt) -> String {
    let attempt = run
        .attempt
        .map(|attempt| format!("Worker retry attempt: {}.", attempt.get()))
        .unwrap_or_else(|| "Worker retry attempt: initial worker lifetime.".to_string());

    format!(
        "Continue working on issue {}: {}.\nThe original workflow prompt is already present in this conversation, so do not resend or restate it.\nResume from the current workspace and conversation context, inspect the latest progress, and continue from where the previous worker left off.\nCurrent issue state: {}\n{}\n",
        issue.identifier, issue.title, issue.state.name, attempt,
    )
}

fn build_session_context(
    run_manifest: &RunManifest,
    observed_run: &RunAttempt,
    manifest: &IssueConversationManifest,
    prompt_kind: IssueSessionPromptKind,
    prompt_path: Option<PathBuf>,
    worker_outcome: Option<WorkerOutcomeRecord>,
) -> IssueSessionContext {
    IssueSessionContext {
        run_id: run_manifest.run_id.clone(),
        issue_id: manifest.issue_id.clone(),
        identifier: manifest.identifier.clone(),
        worker_id: observed_run.worker_id.clone(),
        attempt: observed_run.attempt.map(|attempt| attempt.get()),
        normal_retry_count: observed_run.normal_retry_count,
        turn_count: observed_run.turn_count,
        max_turns: observed_run.max_turns,
        prompt_kind,
        prompt_path,
        conversation_id: manifest.conversation_id.clone(),
        fresh_conversation: manifest.fresh_conversation,
        workflow_prompt_seeded: manifest.workflow_prompt_seeded,
        server_base_url: manifest.server_base_url.clone(),
        transport_target: manifest.transport_target.clone(),
        http_auth_mode: manifest.http_auth_mode.clone(),
        websocket_auth_mode: manifest.websocket_auth_mode.clone(),
        websocket_query_param_name: manifest.websocket_query_param_name.clone(),
        persistence_dir: manifest.persistence_dir.clone(),
        last_execution_status: manifest.last_execution_status.clone(),
        last_event_id: manifest.last_event_id.clone(),
        last_event_kind: manifest.last_event_kind.clone(),
        last_event_at: manifest.last_event_at,
        last_event_summary: manifest.last_event_summary.clone(),
        worker_outcome,
        updated_at: Utc::now(),
    }
}

fn conversation_snapshot(stream: &RuntimeEventStream) -> Conversation {
    let mut conversation = stream.conversation().clone();
    if let Some(status) = stream.state_mirror().execution_status() {
        conversation.execution_status = status.to_string();
    }
    conversation
}

fn build_summary_metadata(
    conversation: &Conversation,
    fresh_conversation: bool,
    stream_state: RuntimeStreamState,
    diagnostics: Option<&crate::TransportDiagnostics>,
    server_base_url: &str,
) -> ConversationMetadata {
    ConversationMetadata {
        conversation_id: ConversationId::new(conversation.conversation_id.to_string())
            .expect("UUID-backed conversation ID should not be empty"),
        server_base_url: Some(server_base_url.to_string()),
        transport_target: diagnostics
            .map(|diagnostics| diagnostics.target_kind.as_str().to_string()),
        http_auth_mode: diagnostics
            .map(|diagnostics| diagnostics.http_auth_kind.as_str().to_string()),
        websocket_auth_mode: diagnostics
            .map(|diagnostics| diagnostics.websocket_auth_kind.as_str().to_string()),
        websocket_query_param_name: diagnostics
            .and_then(|diagnostics| diagnostics.websocket_query_param_name.clone()),
        fresh_conversation,
        runtime_contract_version: Some(RUNTIME_CONTRACT_VERSION.to_string()),
        stream_state,
        last_event_id: None,
        last_event_kind: None,
        last_event_at: None,
        last_event_summary: None,
    }
}

fn observe_event<O>(observer: &mut O, event: &EventEnvelope)
where
    O: IssueSessionObserver,
{
    observer.on_runtime_event(
        timestamp_ms_from_datetime(event.timestamp),
        Some(event.id.clone()),
        Some(event.kind.clone()),
        Some(summarize_event(event)),
    );
}

fn observe_latest_event<O>(observer: &mut O, stream: &RuntimeEventStream)
where
    O: IssueSessionObserver,
{
    if let Some(event) = stream.event_cache().items().last() {
        observe_event(observer, event);
    }
}

fn failed_outcome(summary: impl Into<String>, error: impl Into<String>) -> NormalizedOutcome {
    NormalizedOutcome {
        kind: WorkerOutcomeKind::Failed,
        summary: summary.into(),
        error: Some(error.into()),
    }
}

fn summarize_event(event: &EventEnvelope) -> String {
    match KnownEvent::from_envelope(event) {
        KnownEvent::ConversationStateUpdate(payload) => match payload.execution_status {
            Some(execution_status) => format!("ConversationStateUpdateEvent `{execution_status}`"),
            None => "ConversationStateUpdateEvent".to_string(),
        },
        KnownEvent::ConversationError(_) => format!("ConversationErrorEvent {}", event.id),
        KnownEvent::LlmCompletionLog(_) => "LLMCompletionLogEvent".to_string(),
        KnownEvent::Unknown(unknown) => unknown.kind,
    }
}

fn latest_current_turn_error(
    events: &[EventEnvelope],
    baseline_event_ids: &HashSet<String>,
) -> Option<String> {
    events
        .iter()
        .rev()
        .find(|event| {
            !baseline_event_ids.contains(&event.id)
                && matches!(
                    KnownEvent::from_envelope(event),
                    KnownEvent::ConversationError(_)
                )
        })
        .map(conversation_error_detail)
}

fn conversation_error_detail(event: &EventEnvelope) -> String {
    let message = event
        .payload
        .get("message")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            serde_json::to_string(&event.payload)
                .unwrap_or_else(|_| "unable to encode ConversationErrorEvent payload".to_string())
        });

    format!("ConversationErrorEvent {}: {}", event.id, message)
}

fn run_status_for(outcome_kind: WorkerOutcomeKind) -> RunStatus {
    match outcome_kind {
        WorkerOutcomeKind::Succeeded => RunStatus::Succeeded,
        WorkerOutcomeKind::Cancelled => RunStatus::Cancelled,
        WorkerOutcomeKind::Failed | WorkerOutcomeKind::TimedOut | WorkerOutcomeKind::Stalled => {
            RunStatus::Failed
        }
    }
}

fn observed_run_for_turn(run: &RunAttempt) -> RunAttempt {
    let mut observed_run = run.clone();
    if observed_run.started_at.is_none() {
        observed_run = observed_run.mark_started(timestamp_ms_from_datetime(Utc::now()));
    }
    observed_run.record_turn_started();
    observed_run
}

fn timestamp_ms_from_datetime(value: DateTime<Utc>) -> TimestampMs {
    TimestampMs::new(value.timestamp_millis().max(0) as u64)
}

fn parse_uuid(value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|error| format!("invalid UUID `{value}`: {error}"))
}

fn create_conversation_request_path(workspace: &WorkspaceHandle) -> PathBuf {
    workspace
        .openhands_dir()
        .join("create-conversation-request.json")
}

fn last_conversation_state_path(workspace: &WorkspaceHandle) -> PathBuf {
    workspace
        .openhands_dir()
        .join("last-conversation-state.json")
}

fn session_context_path(workspace: &WorkspaceHandle) -> PathBuf {
    workspace.generated_dir().join("session-context.json")
}

fn finished_stream_error_is_tolerable(error: &OpenHandsError) -> bool {
    matches!(
        error,
        OpenHandsError::ReconnectExhausted { .. } | OpenHandsError::WebSocketClosed
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use opensymphony_domain::WorkerOutcomeKind;
    use opensymphony_testkit::FakeOpenHandsServer;

    use super::*;
    use crate::TransportConfig;

    #[tokio::test]
    async fn await_terminal_outcome_accepts_reconciled_finished_state_after_stream_close() {
        let server = FakeOpenHandsServer::start()
            .await
            .expect("fake server should start");
        let client = OpenHandsClient::new(TransportConfig::new(server.base_url()));
        let conversation = client
            .create_conversation(&ConversationCreateRequest::doctor_probe(
                "/tmp/opensymphony-live",
                "/tmp/opensymphony-live/.opensymphony/openhands",
                Some("fake-model".to_string()),
                None,
            ))
            .await
            .expect("conversation should be created");
        let mut stream = client
            .attach_runtime_stream(
                conversation.conversation_id,
                RuntimeStreamConfig {
                    readiness_timeout: Duration::from_secs(2),
                    reconnect_initial_backoff: Duration::from_millis(25),
                    reconnect_max_backoff: Duration::from_millis(25),
                    max_reconnect_attempts: 1,
                },
            )
            .await
            .expect("runtime stream should attach");
        let baseline_event_ids = stream
            .event_cache()
            .items()
            .iter()
            .map(|event| event.id.clone())
            .collect::<HashSet<_>>();

        server
            .emit_state_update(conversation.conversation_id, "running")
            .await
            .expect("running state should be recorded");
        server
            .emit_state_update(conversation.conversation_id, "finished")
            .await
            .expect("finished state should be recorded");
        stream.close().await.expect("stream should close cleanly");

        let runner = IssueSessionRunner::new(
            client,
            IssueSessionRunnerConfig {
                runtime_stream: RuntimeStreamConfig {
                    readiness_timeout: Duration::from_secs(2),
                    reconnect_initial_backoff: Duration::from_millis(25),
                    reconnect_max_backoff: Duration::from_millis(25),
                    max_reconnect_attempts: 1,
                },
                terminal_wait_timeout: Duration::from_millis(25),
                finished_drain_timeout: Duration::from_millis(25),
            },
        );
        let outcome = runner
            .await_terminal_outcome(&mut stream, &baseline_event_ids, &mut ())
            .await;

        assert_eq!(outcome.kind, WorkerOutcomeKind::Succeeded);
        assert_eq!(
            stream.state_mirror().terminal_status(),
            Some(TerminalExecutionStatus::Finished)
        );
    }
}
