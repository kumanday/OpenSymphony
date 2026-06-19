use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::opensymphony_domain::{
    ConversationId, ConversationMetadata, DurationMs, IssueId, IssueIdentifier, NormalizedIssue,
    RunAttempt, RuntimeStreamState, TimestampMs, WorkerId, WorkerOutcomeKind, WorkerOutcomeRecord,
};
#[cfg(test)]
use crate::opensymphony_domain::{RuntimeLivenessPhase, RuntimeProgressSnapshot};
use crate::opensymphony_memory::{
    IssueEvidence, IssueLinkEvidence, MemoryConfig, MemoryContextOptions, SourceFile,
    context_for_issue_with_options,
};
use crate::opensymphony_workflow::{
    Environment, OpenHandsConversationToolConfig, ProcessEnvironment, ResolvedWorkflow,
};
use crate::opensymphony_workspace::{
    RunManifest, RunStatus, WorkspaceError, WorkspaceHandle, WorkspaceManager,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::time::{Instant, timeout_at};
use tracing::debug;
use uuid::Uuid;

use super::{
    AgentConfig, CondenserConfig, ConfirmationPolicy, Conversation, ConversationCreateRequest,
    ConversationStateMirror, EventEnvelope, KnownEvent, LlmConfig, OpenHandsClient, OpenHandsError,
    RuntimeEventStream, RuntimeStreamConfig, SendMessageRequest, TerminalExecutionStatus,
    ToolConfig, WorkspaceConfig,
};

pub const RUNTIME_CONTRACT_VERSION: &str = "openhands-sdk-agent-server-v1";
const DEFAULT_REUSE_POLICY: &str = "per_issue";
const FRESH_EACH_RUN_REUSE_POLICY: &str = "fresh_each_run";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueSessionReusePolicy {
    PerIssue,
    FreshEachRun,
    Unsupported(String),
}

impl IssueSessionReusePolicy {
    fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            DEFAULT_REUSE_POLICY => Self::PerIssue,
            FRESH_EACH_RUN_REUSE_POLICY => Self::FreshEachRun,
            other => Self::Unsupported(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::PerIssue => DEFAULT_REUSE_POLICY,
            Self::FreshEachRun => FRESH_EACH_RUN_REUSE_POLICY,
            Self::Unsupported(value) => value.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSessionRunnerConfig {
    pub reuse_policy: IssueSessionReusePolicy,
    pub runtime_stream: RuntimeStreamConfig,
    pub terminal_wait_timeout: Duration,
    /// Optional absolute wall-clock cap on total runtime. When `None`, only the
    /// idle/progress timeout (`terminal_wait_timeout`) applies. This field is a
    /// future hook: it is not yet populated from the workflow config, but is
    /// threaded through so the runner can eventually enforce a hard time limit
    /// independent of progress-based idle detection.
    pub total_runtime_cap_ms: Option<Duration>,
    pub finished_drain_timeout: Duration,
    pub memory: Option<MemoryWorkerAccess>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryWorkerAccess {
    pub endpoint: String,
    pub token: Option<String>,
    pub project: Option<String>,
    pub execution_repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkpadComment {
    pub id: String,
    pub body: String,
    pub updated_at: DateTime<Utc>,
}

#[async_trait]
pub trait WorkpadCommentSource: Send + Sync {
    async fn fetch_workpad_comment(&self, issue_id: &str)
    -> Result<Option<WorkpadComment>, String>;
}

pub trait IssueSessionObserver {
    fn on_launch(&mut self, _conversation: &ConversationMetadata) {}

    fn on_runtime_event(
        &mut self,
        _observed_at: TimestampMs,
        _event_id: Option<String>,
        _event_kind: Option<String>,
        _summary: Option<String>,
        _payload: Option<Value>,
    ) {
    }

    /// Called when conversation metadata is updated (e.g., after token accumulation)
    fn on_conversation_update(&mut self, _conversation: &ConversationMetadata) {}
}

impl IssueSessionObserver for () {}

/// Tracks liveness signals during a long-running turn.
///
/// Counts event deltas, token deltas, execution-status changes, reconnect/reconcile
/// progress, and terminal/log frames as liveness signals. When a progress signal is
/// observed, the stall deadline slides forward.
///
/// # Integration with `await_terminal_outcome`
///
/// This tracker records logical progress signals (events, tokens, status changes)
/// using `TimestampMs` timestamps, while the runner's timeout mechanics use
/// `Instant::now()` for wall-clock deadline tracking.  The two work together:
/// the tracker provides structured progress accounting that survives reconnects
/// and reconciliations, while the `Instant`-based sliding deadline handles the
/// actual timeout waits.
#[derive(Debug, Clone)]
pub(crate) struct LivenessTracker {
    /// Monotonic count of events observed since the session started.
    event_count: u64,
    /// Cumulative input tokens.
    input_tokens: u64,
    /// Cumulative output tokens.
    output_tokens: u64,
    /// Current execution status reported by the runtime.
    execution_status: Option<String>,
    /// Timestamp of the most recent liveness signal.
    last_activity_at: Option<TimestampMs>,
    /// Idle timeout — no liveness signal for this duration triggers a stall.
    idle_timeout_ms: DurationMs,
    /// Optional absolute wall-clock cap on total runtime.
    total_runtime_cap_ms: Option<DurationMs>,
    /// When the run started (wall-clock).
    started_at: Option<TimestampMs>,
}

impl LivenessTracker {
    #[cfg(test)]
    pub fn new(idle_timeout_ms: DurationMs) -> Self {
        Self::with_runtime_cap(idle_timeout_ms, None)
    }

    pub fn with_runtime_cap(
        idle_timeout_ms: DurationMs,
        total_runtime_cap_ms: Option<DurationMs>,
    ) -> Self {
        Self {
            event_count: 0,
            input_tokens: 0,
            output_tokens: 0,
            execution_status: None,
            last_activity_at: None,
            idle_timeout_ms,
            total_runtime_cap_ms,
            started_at: None,
        }
    }

    /// Mark the start of a turn.
    pub fn mark_started(&mut self, started_at: TimestampMs) {
        self.started_at = Some(started_at);
        self.last_activity_at = Some(started_at);
    }

    /// Record a new event as a liveness signal.
    pub fn record_event(&mut self, event_at: TimestampMs) -> bool {
        let advanced = self.advance_activity(event_at);
        self.event_count = self.event_count.saturating_add(1);
        advanced
    }

    /// Record a batch of newly-reconciled events.
    pub fn record_reconciled_events(&mut self, count: u64, reconciled_at: TimestampMs) -> bool {
        self.event_count = self.event_count.saturating_add(count);
        self.advance_activity(reconciled_at)
    }

    /// Record cumulative token totals from the conversation manifest.
    pub fn record_tokens(&mut self, input: u64, output: u64, recorded_at: TimestampMs) -> bool {
        let advanced = input > self.input_tokens || output > self.output_tokens;
        self.input_tokens = self.input_tokens.max(input);
        self.output_tokens = self.output_tokens.max(output);
        if advanced {
            self.advance_activity(recorded_at)
        } else {
            false
        }
    }

    /// Record an execution-status change as a liveness signal.
    pub fn record_status_change(&mut self, status: &str, recorded_at: TimestampMs) -> bool {
        let changed = self.execution_status.as_deref() != Some(status);
        self.execution_status = Some(status.to_string());
        if changed {
            self.advance_activity(recorded_at);
        }
        changed
    }

    /// Check whether the run is currently stalled (no progress within idle timeout).
    pub fn is_stalled_at(&self, now: TimestampMs) -> bool {
        // If we haven't started yet, don't declare stall
        let Some(started_at) = self.started_at else {
            return false;
        };
        let Some(last_activity) = self.last_activity_at else {
            return false;
        };

        // Check idle timeout first
        let idle_deadline = last_activity.saturating_add(self.idle_timeout_ms);
        if now >= idle_deadline {
            return true;
        }

        // Check total runtime cap if set
        if let Some(cap) = self.total_runtime_cap_ms {
            let hard_cap = started_at.saturating_add(cap);
            if now >= hard_cap {
                return true;
            }
        }

        false
    }

    /// Compute the stall deadline (sooner of idle deadline and runtime cap).
    pub fn stall_deadline_at(&self) -> Option<TimestampMs> {
        let last_activity = self.last_activity_at?;
        let started_at = self.started_at?;

        let idle_deadline = last_activity.saturating_add(self.idle_timeout_ms);
        match self.total_runtime_cap_ms {
            Some(cap) => {
                let hard_cap = started_at.saturating_add(cap);
                Some(idle_deadline.min(hard_cap))
            }
            None => Some(idle_deadline),
        }
    }

    /// Produce a progress snapshot relative to a previous snapshot.
    #[cfg(test)]
    pub fn snapshot(&self, previous: &RuntimeProgressSnapshot) -> RuntimeProgressSnapshot {
        let phase = match (self.last_activity_at, self.started_at) {
            (None, None) => RuntimeLivenessPhase::WaitingOnPriorTurn,
            (Some(_), Some(_)) => RuntimeLivenessPhase::RunningTurn,
            _ => RuntimeLivenessPhase::WaitingOnPriorTurn,
        };
        previous
            .update_with(phase)
            .with_event_count(self.event_count)
            .with_input_tokens(self.input_tokens)
            .with_output_tokens(self.output_tokens)
            .with_execution_status(self.execution_status.clone())
            .with_last_activity_at(self.last_activity_at)
            .with_stall_deadline_at(self.stall_deadline_at())
            .build()
    }

    fn advance_activity(&mut self, activity_at: TimestampMs) -> bool {
        if self.last_activity_at.is_some_and(|last| activity_at < last) {
            return false;
        }
        let advanced = self.last_activity_at.is_none_or(|last| activity_at > last);
        self.last_activity_at = Some(activity_at);
        advanced
    }
}

impl Default for IssueSessionRunnerConfig {
    fn default() -> Self {
        Self {
            reuse_policy: IssueSessionReusePolicy::PerIssue,
            runtime_stream: RuntimeStreamConfig::default(),
            terminal_wait_timeout: Duration::from_secs(300),
            total_runtime_cap_ms: None,
            finished_drain_timeout: Duration::from_millis(100),
            memory: None,
        }
    }
}

impl IssueSessionRunnerConfig {
    pub fn from_workflow(workflow: &ResolvedWorkflow) -> Self {
        let websocket = &workflow.extensions.openhands.websocket;
        Self {
            reuse_policy: IssueSessionReusePolicy::parse(
                &workflow.extensions.openhands.conversation.reuse_policy,
            ),
            runtime_stream: RuntimeStreamConfig {
                readiness_timeout: Duration::from_millis(websocket.ready_timeout_ms),
                reconnect_initial_backoff: Duration::from_millis(websocket.reconnect_initial_ms),
                reconnect_max_backoff: Duration::from_millis(websocket.reconnect_max_ms),
                ..RuntimeStreamConfig::default()
            },
            terminal_wait_timeout: Duration::from_millis(
                workflow.config.agent.stall_timeout_ms.unwrap_or(300_000),
            ),
            // Future hook: not yet exposed in workflow agent config.
            total_runtime_cap_ms: None,
            finished_drain_timeout: Duration::from_millis(100),
            memory: None,
        }
    }

    pub fn with_memory(mut self, memory: Option<MemoryWorkerAccess>) -> Self {
        self.memory = memory;
        self
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
    pub llm_api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_base_url_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condenser: Option<ConversationLaunchCondenserProfile>,
    pub agent_tools: Option<Vec<ToolConfig>>,
    pub agent_include_default_tools: Option<Vec<String>>,
    pub max_iterations: u32,
    pub stuck_detection: bool,
    /// Fingerprint of the API key used when creating this conversation.
    /// Used to detect when the API key has changed and the conversation needs reset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_api_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationLaunchCondenserProfile {
    pub max_size: u64,
    pub keep_first: u64,
}

impl ConversationLaunchProfile {
    /// Compute a fingerprint of the current API key from the environment.
    /// This is used to detect when the API key has changed.
    pub fn api_key_fingerprint(&self, env: &dyn Environment) -> Option<String> {
        let api_key = self
            .llm_api_key_env
            .as_deref()
            .and_then(|env_name| env.get(env_name))
            .or_else(|| env.get("LLM_API_KEY"))?;

        // Simple hash - hex-encoded SHA256 of API key
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        let result = hasher.finalize();
        // Return first 16 hex chars of hash
        Some(result[..8].iter().map(|b| format!("{:02x}", b)).collect())
    }

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
            llm_api_key_env: conversation
                .agent
                .llm
                .as_ref()
                .and_then(|llm| llm.api_key_env.clone()),
            llm_base_url_env: conversation
                .agent
                .llm
                .as_ref()
                .and_then(|llm| llm.base_url_env.clone()),
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
            llm_api_key_fingerprint: None, // Computed when manifest is created
        })
    }

    pub fn to_create_request(
        &self,
        env: &dyn Environment,
        working_dir: &Path,
        persistence_dir: &Path,
        conversation_id: Option<Uuid>,
    ) -> Result<ConversationCreateRequest, String> {
        let api_key = resolve_provider_override(
            env,
            "openhands.conversation.agent.llm.api_key_env",
            self.llm_api_key_env.as_deref(),
        )?
        .or_else(|| normalize_environment_value(env.get("LLM_API_KEY")));
        let base_url = resolve_provider_override(
            env,
            "openhands.conversation.agent.llm.base_url_env",
            self.llm_base_url_env.as_deref(),
        )?
        .or_else(|| normalize_environment_value(env.get("LLM_BASE_URL")));

        let llm = LlmConfig {
            model: self.llm_model.clone(),
            api_key,
            base_url,
            usage_id: None,
        };

        Ok(ConversationCreateRequest {
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
        })
    }
}

fn resolve_provider_override(
    env: &dyn Environment,
    field: &'static str,
    env_name: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(env_name) = env_name else {
        return Ok(None);
    };

    normalize_environment_value(env.get(env_name)).ok_or_else(|| {
        format!(
            "{field} references environment variable `{env_name}`, but it is not set or is blank"
        )
    })
    .map(Some)
}

fn normalize_environment_value(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn tool_config_from_workflow(tool: &OpenHandsConversationToolConfig) -> ToolConfig {
    ToolConfig {
        name: tool.name.clone(),
        params: tool.params.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfigFingerprint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url_hash: Option<String>,
    pub model: String,
}

impl LlmConfigFingerprint {
    pub fn from_llm_config(llm: &LlmConfig) -> Self {
        // Simplified: only track model name, not API key or base URL
        // since we're no longer using this for drift detection
        Self {
            api_key_hash: None,
            base_url_hash: None,
            model: llm.model.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueConversationManifest {
    pub issue_id: IssueId,
    pub identifier: IssueIdentifier,
    pub conversation_id: ConversationId,
    #[serde(default = "default_reuse_policy")]
    pub reuse_policy: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_config_fingerprint: Option<LlmConfigFingerprint>,
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
    // Token usage accumulation from LLM completions
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    // Tracks the last time tokens were accumulated to avoid double-counting
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_token_accumulation_at: Option<DateTime<Utc>>,
}

impl IssueConversationManifest {
    #[allow(clippy::too_many_arguments)]
    fn new(
        issue_id: IssueId,
        identifier: IssueIdentifier,
        conversation_id: ConversationId,
        reuse_policy: impl Into<String>,
        persistence_dir: PathBuf,
        attached_at: DateTime<Utc>,
        reset_reason: Option<String>,
        mut launch_profile: ConversationLaunchProfile,
        env: &dyn Environment,
    ) -> Self {
        // Compute and store API key fingerprint for drift detection
        launch_profile.llm_api_key_fingerprint = launch_profile.api_key_fingerprint(env);

        Self {
            issue_id,
            identifier,
            conversation_id,
            reuse_policy: reuse_policy.into(),
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
            llm_config_fingerprint: None,
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
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            last_token_accumulation_at: None,
        }
    }

    fn prompt_kind(&self) -> IssueSessionPromptKind {
        if self.workflow_prompt_seeded {
            IssueSessionPromptKind::Continuation
        } else {
            IssueSessionPromptKind::Full
        }
    }

    fn is_reusable_for(
        &self,
        issue: &NormalizedIssue,
        expected_persistence_dir: &Path,
        expected_reuse_policy: &str,
    ) -> bool {
        self.issue_id == issue.id
            && self.identifier == issue.identifier
            && self.reuse_policy == expected_reuse_policy
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

        // Update token counts from conversation state, but preserve existing counts if higher
        // This is important for rehydration where we carry over token counts from old conversations
        if let Some((input, output, cache_read)) = stream.state_mirror().accumulated_token_usage() {
            // Only update if the stream has higher counts (don't wipe out preserved counts)
            if input > self.input_tokens {
                self.input_tokens = input;
            }
            if output > self.output_tokens {
                self.output_tokens = output;
            }
            if cache_read > self.cache_read_tokens {
                self.cache_read_tokens = cache_read;
            }
        }

        self.updated_at = Utc::now();
    }

    fn apply_transport_diagnostics(
        &mut self,
        diagnostics: Option<&super::TransportDiagnostics>,
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
            recent_activity: Vec::new(),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_read_tokens: self.cache_read_tokens,
            total_tokens: self.input_tokens + self.output_tokens,
            runtime_seconds: 0, // TODO: track runtime
            next_activity_sequence: 0,
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
    #[serde(default = "default_reuse_policy")]
    pub reuse_policy: String,
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
    #[error("unexpected early result: {0}")]
    UnexpectedEarlyResult(String),
    #[error("rehydration failed: {0}")]
    RehydrationFailed(String),
}

#[derive(Debug, Clone)]
struct NormalizedOutcome {
    kind: WorkerOutcomeKind,
    summary: String,
    error: Option<String>,
}

/// Result of checking whether a runtime session has reached a terminal state.
///
/// Replaces the confusing `Option<(Outcome, bool)>` pattern where a fake
/// `Failed` outcome was used to encode a liveness signal. Callers can now
/// distinguish between three explicit cases:
/// - `Terminal(outcome)`: the session reached a terminal state.
/// - `StillRunningWithProgress`: the session is still running but a liveness
///   signal was observed (e.g., execution status changed, new events arrived).
/// - `NoProgress`: the session is still running with no new activity.
#[derive(Debug, Clone)]
enum StateCheckResult {
    Terminal(NormalizedOutcome),
    StillRunningWithProgress,
    NoProgress,
}

/// Result of handling reconcile progress in the terminal-outcome loop.
enum ReconcileResult {
    Terminal(NormalizedOutcome),
    Progress,
    NoProgress,
}

pub struct ActiveSession {
    stream: RuntimeEventStream,
    manifest: IssueConversationManifest,
    prompt_kind: IssueSessionPromptKind,
    prompt_path: Option<PathBuf>,
}

impl ActiveSession {
    /// Accumulate tokens from LLM completion events AND conversation state.
    /// Uses last_token_accumulation_at to avoid double-counting events,
    /// but also reads accumulated totals from conversation state (OpenHands tracks tokens there).
    fn accumulate_tokens(&mut self) {
        use super::events::KnownEvent;
        use chrono::DateTime;

        let cutoff = self.manifest.last_token_accumulation_at;
        let mut max_event_time: Option<DateTime<Utc>> = None;
        let mut events_scanned = 0;
        let mut llm_events_found = 0;
        let mut tokens_added = (0u64, 0u64);

        // Scan for LLM completion events (for backward compatibility, though OpenHands doesn't send these)
        for event in self.stream.event_cache().items() {
            events_scanned += 1;

            // Skip events we've already processed
            if let Some(cutoff_time) = cutoff
                && event.timestamp <= cutoff_time
            {
                continue;
            }

            if let KnownEvent::LlmCompletionLog(llm_event) = KnownEvent::from_envelope(event) {
                llm_events_found += 1;
                if let Some((input, output)) = llm_event.token_usage() {
                    self.manifest.input_tokens += input;
                    self.manifest.output_tokens += output;
                    tokens_added.0 += input;
                    tokens_added.1 += output;
                    tracing::debug!(
                        input_tokens = input,
                        output_tokens = output,
                        event_id = %event.id,
                        "accumulated tokens from LLM completion event"
                    );
                } else {
                    tracing::debug!(
                        event_id = %event.id,
                        payload = %llm_event.payload,
                        "LLMCompletionLogEvent has no token usage data"
                    );
                }
            }

            // Track the latest event timestamp we've seen
            if max_event_time.is_none_or(|current| event.timestamp > current) {
                max_event_time = Some(event.timestamp);
            }
        }

        // Update the cutoff time to the latest event we processed
        if let Some(latest_time) = max_event_time {
            self.manifest.last_token_accumulation_at = Some(latest_time);
        }

        // CRITICAL: Also read accumulated tokens from conversation state
        // OpenHands tracks tokens in conversation state.stats, not in LLMCompletionLogEvent
        if let Some((input, output, cache_read)) =
            self.stream.state_mirror().accumulated_token_usage()
        {
            // Use the state's accumulated totals directly (these are already summed by OpenHands)
            if input > self.manifest.input_tokens {
                let new_input = input - self.manifest.input_tokens;
                self.manifest.input_tokens = input;
                tokens_added.0 += new_input;
            }
            if output > self.manifest.output_tokens {
                let new_output = output - self.manifest.output_tokens;
                self.manifest.output_tokens = output;
                tokens_added.1 += new_output;
            }
            if cache_read > self.manifest.cache_read_tokens {
                self.manifest.cache_read_tokens = cache_read;
            }
            tracing::debug!(
                state_input = input,
                state_output = output,
                state_cache_read = cache_read,
                manifest_input = self.manifest.input_tokens,
                manifest_output = self.manifest.output_tokens,
                manifest_cache_read = self.manifest.cache_read_tokens,
                "updated tokens from conversation state"
            );
        }

        tracing::debug!(
            events_scanned,
            llm_events_found,
            input_tokens_added = tokens_added.0,
            output_tokens_added = tokens_added.1,
            total_input_tokens = self.manifest.input_tokens,
            total_output_tokens = self.manifest.output_tokens,
            total_cache_read_tokens = self.manifest.cache_read_tokens,
            "token accumulation complete"
        );
    }
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
    launch_reported: bool,
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
    environment: Arc<dyn Environment + Send + Sync>,
    workpad_comment_source: Option<Arc<dyn WorkpadCommentSource>>,
}

impl IssueSessionRunner {
    pub fn new(client: OpenHandsClient, config: IssueSessionRunnerConfig) -> Self {
        Self::with_environment(client, config, ProcessEnvironment)
    }

    pub fn with_environment<E>(
        client: OpenHandsClient,
        config: IssueSessionRunnerConfig,
        environment: E,
    ) -> Self
    where
        E: Environment + Send + Sync + 'static,
    {
        Self {
            client,
            config,
            environment: Arc::new(environment),
            workpad_comment_source: None,
        }
    }

    pub fn with_workpad_comment_source(
        mut self,
        workpad_comment_source: Arc<dyn WorkpadCommentSource>,
    ) -> Self {
        self.workpad_comment_source = Some(workpad_comment_source);
        self
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
        const MAX_CONTEXT_OVERFLOW_RETRIES: usize = 1;

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

        let mut launch_reported = false;
        if !active_session.manifest.fresh_conversation
            && active_session
                .stream
                .state_mirror()
                .execution_status()
                .is_some_and(turn_is_in_progress)
        {
            observer.on_launch(
                &active_session
                    .manifest
                    .to_domain_metadata(RuntimeStreamState::Ready),
            );
            launch_reported = true;
        }

        let mut retry_count = 0;
        let mut current_session = active_session;
        let (final_session, outcome) = loop {
            let (mut active_session, outcome, turn_launch_reported) = match self
                .execute_turn(
                    workspace_manager,
                    workspace,
                    run_manifest,
                    &observed_run,
                    workflow,
                    issue,
                    run,
                    current_session,
                    launch_reported,
                    observer,
                )
                .await?
            {
                Step::Continue(result) => result,
                Step::EarlyResult(result) => return Ok(*result),
            };
            launch_reported = turn_launch_reported;

            if is_context_overflow_outcome(&outcome) && retry_count < MAX_CONTEXT_OVERFLOW_RETRIES {
                retry_count += 1;
                tracing::warn!(
                    conversation_id = %active_session.manifest.conversation_id,
                    error = ?outcome.error,
                    retry_count,
                    max_retries = MAX_CONTEXT_OVERFLOW_RETRIES,
                    "context overflow detected, attempting auto-recovery via rehydration"
                );

                let old_manifest = active_session.manifest.clone();
                let _ = active_session.stream.close().await;

                match self
                    .recover_from_context_overflow(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        &observed_run,
                        issue,
                        workflow,
                        &old_manifest,
                    )
                    .await
                {
                    Ok(recovered_session) => {
                        tracing::info!(
                            old_conversation_id = %old_manifest.conversation_id,
                            new_conversation_id = %recovered_session.manifest.conversation_id,
                            "context overflow recovery: rehydration succeeded, re-running turn"
                        );
                        current_session = recovered_session;
                        continue;
                    }
                    Err(rehydration_error) => {
                        tracing::warn!(
                            %rehydration_error,
                            "context overflow recovery: rehydration failed, returning original failure"
                        );
                        break (active_session, outcome);
                    }
                }
            } else if is_condenser_tool_matching_outcome(&outcome)
                && retry_count < MAX_CONTEXT_OVERFLOW_RETRIES
            {
                // Condenser tool-matching errors indicate corrupted event history.
                // We need a FRESH conversation (not rehydration) since rehydration
                // would preserve the corrupted events.
                retry_count += 1;
                tracing::warn!(
                    conversation_id = %active_session.manifest.conversation_id,
                    error = ?outcome.error,
                    retry_count,
                    max_retries = MAX_CONTEXT_OVERFLOW_RETRIES,
                    "condenser tool-matching error detected (corrupted event history), creating fresh conversation"
                );

                let _ = active_session.stream.close().await;

                match self
                    .create_fresh_session(
                        workspace_manager,
                        workspace,
                        run_manifest,
                        &observed_run,
                        issue,
                        workflow,
                        Some("condenser tool-matching error auto-recovery".to_string()),
                    )
                    .await
                {
                    Ok(Step::Continue(fresh_session)) => {
                        tracing::info!(
                            old_conversation_id = %active_session.manifest.conversation_id,
                            new_conversation_id = %fresh_session.manifest.conversation_id,
                            "condenser tool-matching recovery: fresh conversation created, re-running turn"
                        );
                        current_session = fresh_session;
                        continue;
                    }
                    Ok(Step::EarlyResult(result)) => {
                        tracing::warn!(
                            "condenser tool-matching recovery: failed to create fresh session, returning early result"
                        );
                        return Ok(*result);
                    }
                    Err(fresh_error) => {
                        tracing::warn!(
                            %fresh_error,
                            "condenser tool-matching recovery: failed to create fresh session, returning original failure"
                        );
                        break (active_session, outcome);
                    }
                }
            } else {
                break (active_session, outcome);
            }
        };

        self.finalize_active_session(
            workspace_manager,
            workspace,
            run_manifest,
            &observed_run,
            final_session,
            outcome,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_turn<O>(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        workflow: &ResolvedWorkflow,
        issue: &NormalizedIssue,
        run: &RunAttempt,
        active_session: ActiveSession,
        launch_reported: bool,
        observer: &mut O,
    ) -> Result<Step<(ActiveSession, NormalizedOutcome, bool)>, IssueSessionError>
    where
        O: IssueSessionObserver,
    {
        let (mut active_session, mut prepared_turn) = match self
            .prepare_turn(
                workspace_manager,
                workspace,
                run_manifest,
                observed_run,
                workflow,
                issue,
                run,
                active_session,
                launch_reported,
                observer,
            )
            .await?
        {
            Step::Continue(state) => state,
            Step::EarlyResult(result) => return Ok(Step::EarlyResult(result)),
        };

        active_session = match self
            .start_turn(
                workspace_manager,
                workspace,
                run_manifest,
                observed_run,
                active_session,
                &mut prepared_turn,
                observer,
            )
            .await?
        {
            Step::Continue(session) => session,
            Step::EarlyResult(result) => return Ok(Step::EarlyResult(result)),
        };

        let outcome = self
            .await_terminal_outcome(
                &mut active_session,
                &prepared_turn.baseline_event_ids,
                observer,
            )
            .await;

        Ok(Step::Continue((
            active_session,
            outcome,
            prepared_turn.launch_reported,
        )))
    }

    #[allow(clippy::too_many_arguments)]
    async fn recover_from_context_overflow(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
        old_manifest: &IssueConversationManifest,
    ) -> Result<ActiveSession, IssueSessionError> {
        let rehydration_result = self
            .rehydrate_conversation(
                workspace_manager,
                workspace,
                run_manifest,
                observed_run,
                issue,
                workflow,
                old_manifest,
                RehydrationOptions {
                    reason: "context overflow auto-recovery".to_string(),
                    summarize: false,
                    max_summary_events: 0,
                },
            )
            .await?;

        Ok(rehydration_result.session)
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
        match &self.config.reuse_policy {
            IssueSessionReusePolicy::PerIssue => {
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
            IssueSessionReusePolicy::FreshEachRun => {
                let loaded = self
                    .load_existing_conversation_manifest(workspace_manager, workspace, issue, workflow)
                    .await?;
                let reset_reason = loaded.manifest.as_ref().map_or(loaded.reset_reason, |_| {
                    Some(
                        "workflow reuse policy `fresh_each_run` requested a new conversation for this run"
                            .to_string(),
                    )
                });
                self.create_fresh_session(
                    workspace_manager,
                    workspace,
                    run_manifest,
                    observed_run,
                    issue,
                    workflow,
                    reset_reason,
                )
                .await
            }
            IssueSessionReusePolicy::Unsupported(policy) => self
                .persist_failure_without_stream(
                    workspace_manager,
                    workspace,
                    run_manifest,
                    observed_run,
                    IssueSessionPromptKind::Full,
                    None,
                    failed_outcome(
                        "workflow configured an unsupported OpenHands conversation reuse policy",
                        format!(
                            "unsupported OpenHands conversation reuse policy `{policy}`; supported runtime policies: `{DEFAULT_REUSE_POLICY}`, `{FRESH_EACH_RUN_REUSE_POLICY}`"
                        ),
                    ),
                )
                .await
                .map(Box::new)
                .map(Step::EarlyResult),
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
        launch_reported: bool,
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

        if let Err(error) = write_memory_context_artifact(
            workspace_manager,
            workspace,
            issue,
            self.config.memory.as_ref(),
        )
        .await
        {
            debug!(
                %error,
                issue = %issue.identifier,
                "memory context artifact generation failed; continuing without it"
            );
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
        // Note: Rehydration context removed - we now reuse conversations as-is
        // without deleting and recreating them

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
                launch_reported,
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
                    if !prepared_turn.launch_reported {
                        observer.on_launch(
                            &active_session
                                .manifest
                                .to_domain_metadata(RuntimeStreamState::Ready),
                        );
                        prepared_turn.launch_reported = true;
                    }
                    if let Err(error) = active_session.stream.reconcile_events().await {
                        debug!(
                            %error,
                            conversation_id = %active_session.manifest.conversation_id,
                            "pre-wait reconcile failed after run retry conflict, proceeding anyway"
                        );
                    }
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
                    // Read any existing token counts from conversation state
                    active_session.accumulate_tokens();
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

        // Accumulate tokens from LLM completion events before creating metadata
        active_session.accumulate_tokens();

        if !prepared_turn.launch_reported {
            observer.on_launch(
                &active_session
                    .manifest
                    .to_domain_metadata(RuntimeStreamState::Ready),
            );
            prepared_turn.launch_reported = true;
        }

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
        let expected_reuse_policy = self.config.reuse_policy.as_str();
        if !manifest.is_reusable_for(issue, &expected_persistence_dir, expected_reuse_policy) {
            let reset_reason = if manifest.reuse_policy != expected_reuse_policy {
                format!(
                    "conversation manifest reuse policy `{}` does not match expected `{}`",
                    manifest.reuse_policy, expected_reuse_policy
                )
            } else {
                format!(
                    "conversation manifest is incompatible with issue {} or the current workspace",
                    issue.identifier
                )
            };
            return Ok(LoadedManifest {
                manifest: None,
                reset_reason: Some(reset_reason),
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
        _issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
        manifest: IssueConversationManifest,
    ) -> Result<ReuseSession, IssueSessionError> {
        let conversation_id = match parse_uuid(manifest.conversation_id.as_str()) {
            Ok(conversation_id) => conversation_id,
            Err(error) => return Ok(ReuseSession::Reset(error)),
        };

        // Check if condenser config has changed and requires reset
        // Old conversations without condenser should be reset when workflow now has condenser enabled
        let workflow_condenser = workflow
            .extensions
            .openhands
            .conversation
            .agent
            .condenser
            .as_ref();
        let manifest_condenser = manifest
            .launch_profile
            .as_ref()
            .and_then(|p| p.condenser.as_ref());

        if workflow_condenser.is_some() && manifest_condenser.is_none() {
            return Ok(ReuseSession::Reset(
                "workflow now has condenser enabled, but existing conversation was created without condenser - resetting to apply condenser".to_string(),
            ));
        }

        // Defensive check: if previous run ended with error status and had activity,
        // reset to avoid potential corrupted event history.
        // This is especially important for condenser tool-matching errors which indicate
        // corrupted state that would fail again on retry.
        if manifest.last_execution_status.as_deref() == Some("error")
            && manifest.last_event_id.is_some()
        {
            return Ok(ReuseSession::Reset(
                "previous run ended with error status, resetting to avoid potential corrupted event history".to_string(),
            ));
        }

        // Simplified conversation resumption: just try to attach directly.
        // The conversation's stored LLM config in meta.json is used as-is.
        // If the API key has changed, the attach will fail naturally and
        // the caller can use explicit rehydration via the CLI.
        self.try_attach_and_resume(workspace_manager, workspace, manifest, conversation_id)
            .await
    }

    async fn try_attach_and_resume(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        mut manifest: IssueConversationManifest,
        conversation_id: Uuid,
    ) -> Result<ReuseSession, IssueSessionError> {
        let manifest_conversation_id = manifest.conversation_id.clone();

        // Simplified conversation resumption: just try to attach directly
        // without checking for LLM config drift or rehydrating.
        // The conversation's stored LLM config in meta.json is used as-is.
        let stream = match self
            .client
            .attach_runtime_stream(conversation_id, self.config.runtime_stream.clone())
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                return Ok(ReuseSession::Reset(format!(
                    "failed to attach existing conversation {}: {error}",
                    manifest_conversation_id
                )));
            }
        };

        let attached_at = Utc::now();
        manifest.fresh_conversation = false;
        manifest.reuse_policy = self.config.reuse_policy.as_str().to_owned();
        // Note: We no longer update launch_profile on resume - use what's stored
        manifest.llm_config_fingerprint.get_or_insert_with(|| {
            LlmConfigFingerprint::from_llm_config(&stream.conversation().agent.llm)
        });
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

        let mut session = ActiveSession {
            prompt_kind: manifest.prompt_kind(),
            stream,
            manifest,
            prompt_path: None,
        };
        // Read existing token counts from conversation state on load
        session.accumulate_tokens();
        Ok(ReuseSession::Active(Box::new(session)))
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
        let request = match launch_profile.to_create_request(
            self.environment.as_ref(),
            workspace.workspace_path(),
            &configured_persistence_dir(workflow, workspace),
            None,
        ) {
            Ok(request) => request,
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
                            summary: "failed to build OpenHands conversation create request"
                                .to_string(),
                            error: Some(detail),
                        },
                    )
                    .await
                    .map(Box::new)
                    .map(Step::EarlyResult);
            }
        };
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
            self.config.reuse_policy.as_str(),
            configured_persistence_dir(workflow, workspace),
            attached_at,
            reset_reason,
            launch_profile,
            self.environment.as_ref(),
        );
        manifest.llm_config_fingerprint =
            Some(LlmConfigFingerprint::from_llm_config(&request.agent.llm));
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

        let mut session = ActiveSession {
            prompt_kind: manifest.prompt_kind(),
            stream,
            manifest,
            prompt_path: None,
        };
        // Read existing token counts from conversation state on load
        session.accumulate_tokens();
        Ok(Step::Continue(session))
    }

    /// Explicitly rehydrate a conversation by creating a fresh one with the same
    /// configuration, optionally preserving metrics and summarizing history.
    ///
    /// This is NOT automatically triggered - it must be explicitly called when
    /// rehydration is truly needed (e.g., corruption, export/import, etc.).
    /// For normal operation, conversations are simply reused as-is.
    #[allow(clippy::too_many_arguments)]
    pub async fn rehydrate_conversation(
        &self,
        workspace_manager: &WorkspaceManager,
        workspace: &WorkspaceHandle,
        run_manifest: &mut RunManifest,
        observed_run: &RunAttempt,
        issue: &NormalizedIssue,
        workflow: &ResolvedWorkflow,
        old_manifest: &IssueConversationManifest,
        options: RehydrationOptions,
    ) -> Result<RehydrationResult, IssueSessionError> {
        let old_conversation_id = old_manifest.conversation_id.clone();

        // Try to attach to the old conversation to get its state for summarization
        let old_stream = if options.summarize {
            match parse_uuid(old_conversation_id.as_str()) {
                Ok(conversation_id) => self
                    .client
                    .attach_runtime_stream(conversation_id, self.config.runtime_stream.clone())
                    .await
                    .ok(),
                Err(_) => None,
            }
        } else {
            None
        };

        // Build rehydration context if summarization is enabled
        let context = if let Some(stream) = old_stream {
            self.build_rehydration_context(&stream, options.max_summary_events)
                .await
        } else {
            None
        };

        // Delete the old conversation
        if let Ok(conversation_id) = parse_uuid(old_conversation_id.as_str())
            && let Err(error) = self.client.delete_conversation(conversation_id).await
        {
            tracing::warn!(
                conversation_id = %old_conversation_id,
                %error,
                "failed to delete old conversation during rehydration"
            );
        }

        // Create a fresh session with the current configuration
        let step = self
            .create_fresh_session(
                workspace_manager,
                workspace,
                run_manifest,
                observed_run,
                issue,
                workflow,
                Some(format!("rehydration: {}", options.reason)),
            )
            .await?;

        let mut session = match step {
            Step::Continue(session) => session,
            Step::EarlyResult(result) => {
                return Err(IssueSessionError::RehydrationFailed(
                    result
                        .worker_outcome
                        .error
                        .or(result.worker_outcome.summary)
                        .unwrap_or_else(|| "rehydration failed".to_string()),
                ));
            }
        };

        // Copy token counts from old manifest to preserve metrics across rehydration
        session.manifest.input_tokens = old_manifest.input_tokens;
        session.manifest.output_tokens = old_manifest.output_tokens;
        session.manifest.cache_read_tokens = old_manifest.cache_read_tokens;
        session.manifest.last_token_accumulation_at = old_manifest.last_token_accumulation_at;

        // Persist the updated manifest with token counts
        workspace_manager
            .write_json_artifact(
                workspace,
                &workspace.conversation_manifest_path(),
                &session.manifest,
            )
            .await?;

        Ok(RehydrationResult {
            session,
            context,
            old_conversation_id: old_conversation_id.to_string(),
        })
    }

    /// Build a rehydration context summarizing the conversation history.
    /// This is used when explicitly rehydrating to provide context to the new conversation.
    async fn build_rehydration_context(
        &self,
        stream: &RuntimeEventStream,
        max_events: usize,
    ) -> Option<String> {
        let conversation = stream.conversation();
        let summary = self.summarize_conversation(stream, max_events).await;

        let context = format!(
            "## Previous Conversation Context\n\n\
            This conversation was rehydrated from a previous session.\n\n\
            - Original conversation ID: {}\n\
            - Model: {}\n\
            - Max iterations: {}\n\
            - Execution status: {}\n\n\
            ### Summary of Previous Work\n\n\
            {}",
            conversation.conversation_id,
            conversation.agent.llm.model,
            conversation.max_iterations,
            conversation.execution_status,
            summary.unwrap_or_else(|| "No summary available.".to_string())
        );

        Some(context)
    }

    /// Summarize the conversation by extracting key events.
    async fn summarize_conversation(
        &self,
        stream: &RuntimeEventStream,
        max_events: usize,
    ) -> Option<String> {
        let events: Vec<_> = stream
            .event_cache()
            .items()
            .iter()
            .filter(|e| {
                matches!(
                    KnownEvent::from_envelope(e),
                    KnownEvent::Message(_)
                        | KnownEvent::Observation(_)
                        | KnownEvent::ConversationError(_)
                )
            })
            .take(max_events)
            .collect();

        if events.is_empty() {
            return None;
        }

        let mut summary = String::new();
        for event in events {
            let line = match KnownEvent::from_envelope(event) {
                KnownEvent::Message(msg) => {
                    format!("- User: {}\n", msg.text_preview.as_deref().unwrap_or("..."))
                }
                KnownEvent::Observation(obs) => {
                    format!(
                        "- {}: {}\n",
                        obs.tool_name.as_deref().unwrap_or("Result"),
                        obs.text_preview.as_deref().unwrap_or("...")
                    )
                }
                KnownEvent::ConversationError(err) => {
                    let msg = err
                        .payload
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown error");
                    format!("- Error: {}\n", msg)
                }
                _ => String::new(),
            };
            summary.push_str(&line);
        }

        Some(summary)
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
        session: &mut ActiveSession,
        baseline_event_ids: &HashSet<String>,
        observer: &mut O,
    ) -> NormalizedOutcome
    where
        O: IssueSessionObserver,
    {
        let idle_timeout_ms = DurationMs::new(self.config.terminal_wait_timeout.as_millis() as u64);
        let total_runtime_cap_ms = self
            .config
            .total_runtime_cap_ms
            .map(|d| DurationMs::new(d.as_millis() as u64));
        let mut tracker = LivenessTracker::with_runtime_cap(idle_timeout_ms, total_runtime_cap_ms);
        tracker.mark_started(timestamp_ms_from_datetime(Utc::now()));

        let mut next_token_accumulation = Instant::now() + Duration::from_secs(15);

        loop {
            if Instant::now() >= next_token_accumulation {
                session.accumulate_tokens();
                let (input, output) = (
                    session.manifest.input_tokens,
                    session.manifest.output_tokens,
                );
                tracker.record_tokens(input, output, timestamp_ms_from_datetime(Utc::now()));
                observer.on_conversation_update(
                    &session
                        .manifest
                        .to_domain_metadata(RuntimeStreamState::Ready),
                );
                next_token_accumulation = Instant::now() + Duration::from_secs(15);
            }

            match self
                .terminal_outcome_from_state(&mut session.stream, baseline_event_ids, observer)
                .await
            {
                StateCheckResult::Terminal(outcome) => {
                    session.accumulate_tokens();
                    let (input, output) = (
                        session.manifest.input_tokens,
                        session.manifest.output_tokens,
                    );
                    tracker.record_tokens(input, output, timestamp_ms_from_datetime(Utc::now()));
                    return outcome;
                }
                StateCheckResult::StillRunningWithProgress => {
                    // The state check only confirms current-turn activity already exists
                    // in the cache. Event accounting happens when the event is first read
                    // from the stream or reconciled from the server, so do not record a
                    // synthetic event here.
                }
                StateCheckResult::NoProgress => {}
            }

            let now = Instant::now();
            let now_ts = timestamp_ms_from_datetime(Utc::now());
            let event_timeout =
                compute_timeout_duration(&tracker, next_token_accumulation, now, now_ts);

            let next_event = timeout_at(now + event_timeout, session.stream.next_event()).await;

            match next_event {
                Err(_) => {
                    if Instant::now() >= next_token_accumulation {
                        session.accumulate_tokens();
                        next_token_accumulation = Instant::now() + Duration::from_secs(15);
                    }

                    let now_ts = timestamp_ms_from_datetime(Utc::now());
                    if tracker.is_stalled_at(now_ts) {
                        match self
                            .handle_reconcile_progress(
                                session,
                                baseline_event_ids,
                                observer,
                                Some(&mut tracker),
                            )
                            .await
                        {
                            ReconcileResult::Terminal(outcome) => {
                                session.accumulate_tokens();
                                return outcome;
                            }
                            ReconcileResult::Progress => {
                                // `handle_reconcile_progress` already called
                                // `record_reconciled_events()` which updates the tracker's
                                // activity timestamp and event count, sliding the stall
                                // deadline. No additional `record_event()` call is needed
                                // here — that would double-count the reconciled batch.
                                continue;
                            }
                            ReconcileResult::NoProgress => {}
                        }
                        // Re-check stall after reconciliation: the tracker's deadline
                        // may have been extended by reconciled events or token updates.
                        let now_ts = timestamp_ms_from_datetime(Utc::now());
                        if tracker.is_stalled_at(now_ts) {
                            return NormalizedOutcome {
                                kind: WorkerOutcomeKind::Stalled,
                                summary:
                                    "runtime did not reach a terminal state before the stall timeout"
                                        .to_string(),
                                error: Some(format!(
                                    "no progress observed within {} ms idle timeout",
                                    self.config.terminal_wait_timeout.as_millis()
                                )),
                            };
                        }
                    }
                }
                Ok(Ok(Some(event))) => {
                    observe_event(observer, &event);
                    tracker.record_event(timestamp_ms_from_datetime(event.timestamp));
                    if let Some(status) = session.stream.state_mirror().execution_status() {
                        tracker
                            .record_status_change(status, timestamp_ms_from_datetime(Utc::now()));
                    }
                }
                Ok(Ok(None)) => {
                    match self
                        .handle_reconcile_progress(
                            session,
                            baseline_event_ids,
                            observer,
                            Some(&mut tracker),
                        )
                        .await
                    {
                        ReconcileResult::Terminal(outcome) => {
                            session.accumulate_tokens();
                            return outcome;
                        }
                        ReconcileResult::Progress => {
                            // `handle_reconcile_progress` already called
                            // `record_reconciled_events()` — no additional
                            // `record_event()` needed to avoid double-counting.
                            continue;
                        }
                        ReconcileResult::NoProgress => {}
                    }

                    session.accumulate_tokens();
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
                    match self
                        .handle_reconcile_progress(
                            session,
                            baseline_event_ids,
                            observer,
                            Some(&mut tracker),
                        )
                        .await
                    {
                        ReconcileResult::Terminal(outcome) => {
                            session.accumulate_tokens();
                            return outcome;
                        }
                        ReconcileResult::Progress => {
                            // `handle_reconcile_progress` already called
                            // `record_reconciled_events()` — no additional
                            // `record_event()` needed to avoid double-counting.
                            continue;
                        }
                        ReconcileResult::NoProgress => {}
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

    /// Helper: reconcile events and check for terminal outcome if progress found.
    ///
    /// Extracts the repeated "reconcile -> check terminal -> slide deadline" pattern
    /// into a single place to avoid duplication.
    ///
    /// If a liveness tracker is provided, records newly-reconciled events so the
    /// tracker's progress accounting stays in sync with what the runner observes.
    async fn handle_reconcile_progress<O>(
        &self,
        session: &mut ActiveSession,
        baseline_event_ids: &HashSet<String>,
        observer: &mut O,
        tracker: Option<&mut LivenessTracker>,
    ) -> ReconcileResult
    where
        O: IssueSessionObserver,
    {
        if let Ok(inserted) = session.stream.reconcile_events().await
            && inserted > 0
        {
            observe_latest_event(observer, &session.stream);
            if let Some(tracker) = tracker {
                tracker.record_reconciled_events(
                    inserted as u64,
                    timestamp_ms_from_datetime(Utc::now()),
                );
            }
            match self
                .terminal_outcome_from_state(&mut session.stream, baseline_event_ids, observer)
                .await
            {
                StateCheckResult::Terminal(outcome) => return ReconcileResult::Terminal(outcome),
                StateCheckResult::StillRunningWithProgress => return ReconcileResult::Progress,
                // `NoProgress` here means "activity was observed (inserted > 0) but no
                // terminal state was reached."  This maps to `ReconcileResult::Progress`
                // because the reconciliation itself found fresh events from the server,
                // which is a valid liveness signal that should slide the stall deadline.
                // The runner should keep waiting rather than declaring the session stalled.
                StateCheckResult::NoProgress => return ReconcileResult::Progress,
            }
        }
        ReconcileResult::NoProgress
    }

    /// Check whether the runtime has reached a terminal state.
    ///
    /// Returns `StateCheckResult::Terminal(outcome)` when a terminal state is reached,
    /// `StateCheckResult::StillRunningWithProgress` when the session is still running
    /// but a liveness signal was observed, and `StateCheckResult::NoProgress` when
    /// there is no new activity to report.
    async fn terminal_outcome_from_state<O>(
        &self,
        stream: &mut RuntimeEventStream,
        baseline_event_ids: &HashSet<String>,
        observer: &mut O,
    ) -> StateCheckResult
    where
        O: IssueSessionObserver,
    {
        let has_current_turn_activity = stream
            .event_cache()
            .items()
            .iter()
            .any(|event| !baseline_event_ids.contains(&event.id));
        if !has_current_turn_activity {
            return StateCheckResult::NoProgress;
        }

        if let Some(error_detail) =
            latest_current_turn_error(stream.event_cache().items(), baseline_event_ids)
        {
            return StateCheckResult::Terminal(NormalizedOutcome {
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
                    StateCheckResult::Terminal(NormalizedOutcome {
                        kind: WorkerOutcomeKind::Succeeded,
                        summary: "OpenHands execution_status `finished`".to_string(),
                        error: None,
                    })
                } else {
                    StateCheckResult::NoProgress
                }
            }
            Some(TerminalExecutionStatus::Error) => {
                let error_detail = extract_error_detail_from_state(stream.state_mirror())
                    .unwrap_or_else(|| "execution_status error".to_string());
                StateCheckResult::Terminal(NormalizedOutcome {
                    kind: WorkerOutcomeKind::Failed,
                    summary: "OpenHands execution_status `error`".to_string(),
                    error: Some(error_detail),
                })
            }
            Some(TerminalExecutionStatus::Stuck) => StateCheckResult::Terminal(NormalizedOutcome {
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
            None => StateCheckResult::StillRunningWithProgress,
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

async fn write_memory_context_artifact(
    workspace_manager: &WorkspaceManager,
    workspace: &WorkspaceHandle,
    issue: &NormalizedIssue,
    memory: Option<&MemoryWorkerAccess>,
) -> Result<(), String> {
    if let Some(memory) = memory {
        let context = fetch_memory_context_from_server(memory, issue).await?;
        return workspace_manager
            .write_text_artifact(workspace, &workspace.memory_context_path(), &context)
            .await
            .map_err(|error| format!("failed to write memory context artifact: {error}"));
    }

    let config = MemoryConfig::load(workspace.workspace_path(), None).map_err(|error| {
        format!(
            "failed to load memory config from {}: {error}",
            workspace.workspace_path().display()
        )
    })?;
    if !config.enabled || (!config.config_path.exists() && !config.index_path.exists()) {
        return Ok(());
    }
    let source = SourceFile {
        issues: vec![IssueEvidence {
            id: Some(issue.id.as_str().to_string()),
            identifier: issue.identifier.as_str().to_string(),
            title: issue.title.clone(),
            url: issue.url.clone(),
            description: issue.description.clone(),
            state: Some(issue.state.name.clone()),
            labels: issue.labels.clone(),
            children: issue
                .sub_issues
                .iter()
                .map(|child| IssueLinkEvidence {
                    id: Some(child.id.as_str().to_string()),
                    identifier: child.identifier.as_str().to_string(),
                    state: Some(child.state.clone()),
                    ..IssueLinkEvidence::default()
                })
                .collect(),
            blocked_by: issue
                .blocked_by
                .iter()
                .filter_map(|blocker| {
                    blocker
                        .identifier
                        .as_ref()
                        .map(|identifier| IssueLinkEvidence {
                            id: blocker.id.as_ref().map(|id| id.as_str().to_string()),
                            identifier: identifier.as_str().to_string(),
                            state: blocker.state.clone(),
                            ..IssueLinkEvidence::default()
                        })
                })
                .collect(),
            ..IssueEvidence::default()
        }],
        ..SourceFile::default()
    };
    let options = MemoryContextOptions::for_issue(issue.identifier.as_str(), 20);
    let context = context_for_issue_with_options(&config, &source, &options)
        .map_err(|error| format!("failed to build memory context: {error}"))?;
    workspace_manager
        .write_text_artifact(workspace, &workspace.memory_context_path(), &context)
        .await
        .map_err(|error| format!("failed to write memory context artifact: {error}"))
}

async fn fetch_memory_context_from_server(
    memory: &MemoryWorkerAccess,
    issue: &NormalizedIssue,
) -> Result<String, String> {
    let mut arguments = json!({
        "issue": issue.identifier.to_string(),
        "limit": 20,
        "currentIssue": {
            "id": issue.id.to_string(),
            "identifier": issue.identifier.to_string(),
            "title": issue.title.clone(),
            "description": issue.description.clone(),
            "state": issue.state.name.clone(),
            "labels": issue.labels.clone(),
            "children": issue.sub_issues.iter().map(|child| json!({
                "id": child.id.to_string(),
                "identifier": child.identifier.to_string(),
                "state": child.state.clone(),
            })).collect::<Vec<_>>(),
            "blockedBy": issue.blocked_by.iter().filter_map(|blocker| {
                blocker.identifier.as_ref().map(|identifier| json!({
                    "id": blocker.id.as_ref().map(ToString::to_string),
                    "identifier": identifier.to_string(),
                    "state": blocker.state.clone(),
                }))
            }).collect::<Vec<_>>(),
        },
    });
    if let Value::Object(map) = &mut arguments {
        if let Some(project) = &memory.project {
            map.insert("project".to_string(), json!(project));
            map.insert("projectSet".to_string(), json!(project));
        }
        if let Some(repo) = &memory.execution_repo {
            map.insert("repo".to_string(), json!(repo));
        }
    }
    let request = json!({
        "jsonrpc": "2.0",
        "id": "opensymphony-worker-context",
        "method": "tools/call",
        "params": {
            "name": "memory.context",
            "arguments": arguments
        }
    });
    let client = reqwest::Client::new();
    let mut builder = client.post(&memory.endpoint).json(&request);
    if let Some(token) = &memory.token {
        builder = builder.bearer_auth(token);
    }
    let response = builder
        .send()
        .await
        .map_err(|error| format!("failed to call memory server: {error}"))?;
    let status = response.status();
    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("memory server response was not valid JSON: {error}"))?;
    if !status.is_success() {
        return Err(format!("memory server returned HTTP {status}: {payload}"));
    }
    if let Some(error) = payload.get("error") {
        return Err(format!("memory server returned an MCP error: {error}"));
    }
    payload
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|item| item.get("text"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "memory server response omitted text content".to_string())
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
        reuse_policy: manifest.reuse_policy.clone(),
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

fn default_reuse_policy() -> String {
    DEFAULT_REUSE_POLICY.to_owned()
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
    diagnostics: Option<&super::TransportDiagnostics>,
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
        recent_activity: Vec::new(),
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        total_tokens: 0,
        runtime_seconds: 0,
        next_activity_sequence: 0,
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
        Some(runtime_event_payload(event)),
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

fn is_context_overflow_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("prompt is too long")
        || lower.contains("maximum context length")
        || lower.contains("context window")
        || lower.contains("context_length")
        || lower.contains("token limit")
        || lower.contains("context length exceeded")
        || (lower.contains("exceeds") && lower.contains("context"))
}

fn is_context_overflow_outcome(outcome: &NormalizedOutcome) -> bool {
    outcome.kind == WorkerOutcomeKind::Failed
        && outcome
            .error
            .as_deref()
            .is_some_and(is_context_overflow_error)
}

/// Detect condenser internal errors from OpenHands SDK's ToolCallMatching.
/// The error appears as a KeyError string representation: the key in single quotes
/// containing "-tool-" which indicates a tool call ID.
/// This happens when event history has an ObservationBaseEvent without a corresponding ActionEvent.
fn is_condenser_tool_matching_error(msg: &str) -> bool {
    // Pattern: KeyError for a tool call ID from OpenHands SDK's ToolCallMatching.
    // The error appears as a KeyError string representation: the key in single quotes.
    //
    // Known formats:
    // - OpenAI: 'chatcmpl-tool-{hex}'
    //
    // We match the known OpenAI format. Other LLM provider formats can be added as needed.
    msg.starts_with("'") && msg.ends_with("'") && msg.contains("chatcmpl-tool-")
}

fn is_condenser_tool_matching_outcome(outcome: &NormalizedOutcome) -> bool {
    outcome.kind == WorkerOutcomeKind::Failed
        && outcome
            .error
            .as_deref()
            .is_some_and(is_condenser_tool_matching_error)
}

fn extract_error_detail_from_state(state: &ConversationStateMirror) -> Option<String> {
    let raw = state.raw_state();

    // Try to get error from state_delta.last_error
    if let Some(delta) = raw.get("state_delta")
        && let Some(error) = delta.get("last_error").and_then(|e: &Value| e.as_str())
    {
        return Some(error.to_string());
    }

    // Try to get error from stats.last_error
    if let Some(stats) = raw.get("stats")
        && let Some(error) = stats.get("last_error").and_then(|e: &Value| e.as_str())
    {
        return Some(error.to_string());
    }

    // Try to get error from top-level error field
    if let Some(error) = raw.get("error").and_then(|e: &Value| e.as_str()) {
        return Some(error.to_string());
    }

    None
}

fn summarize_event(event: &EventEnvelope) -> String {
    match KnownEvent::from_envelope(event) {
        KnownEvent::ConversationStateUpdate(payload) => match payload.execution_status {
            Some(execution_status) => format!("status: {}", execution_status),
            None => "state update".to_string(),
        },
        KnownEvent::ConversationError(_) => format!("error: {}", event.id),
        KnownEvent::LlmCompletionLog(_) => "llm completion".to_string(),
        KnownEvent::Message(msg) => {
            let preview = msg.text_preview.unwrap_or_else(|| "message".to_string());
            format!("{}: {}", msg.role, preview)
        }
        KnownEvent::Action(action) => {
            let tool = action.tool_name.as_deref().unwrap_or("");
            let command = action_command(&action.arguments);
            let msg = action.message.as_deref().or(command).unwrap_or("action");
            if tool.is_empty() {
                msg.to_string()
            } else {
                format!("{}: {}", tool, msg)
            }
        }
        KnownEvent::Observation(obs) => {
            let tool = obs.tool_name.as_deref().unwrap_or("");
            let preview = obs.text_preview.as_deref().unwrap_or("");
            if tool.is_empty() {
                if preview.is_empty() {
                    "result".to_string()
                } else {
                    format!("→ {}", preview)
                }
            } else {
                format!("{}: {}", tool, preview)
            }
        }
        KnownEvent::Unknown(unknown) => unknown.kind,
    }
}

fn runtime_event_payload(event: &EventEnvelope) -> Value {
    match KnownEvent::from_envelope(event) {
        KnownEvent::Action(action) => {
            let mut body = json!({
                "action_id": action.action_id,
                "tool_name": action.tool_name,
                "message": action.message,
            });
            if let (Value::Object(body), Value::Object(arguments)) = (&mut body, &action.arguments)
            {
                for (key, value) in arguments {
                    body.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
            body
        }
        KnownEvent::Observation(observation) => json!({
            "observation_id": observation.observation_id,
            "tool_name": observation.tool_name,
            "exit_code": observation.exit_code,
            "preview": observation.text_preview,
            "content": event
                .payload
                .get("observation")
                .and_then(|value| value.get("content"))
                .cloned()
                .unwrap_or(Value::Null),
        }),
        KnownEvent::ConversationStateUpdate(payload) => json!({
            "execution_status": payload.execution_status,
        }),
        KnownEvent::ConversationError(error) => error.payload,
        KnownEvent::LlmCompletionLog(log) => log.payload,
        KnownEvent::Message(message) => json!({
            "role": message.role,
            "preview": message.text_preview,
            "content": event.payload.get("content").cloned().unwrap_or(Value::Null),
        }),
        KnownEvent::Unknown(unknown) => json!({
            "kind": unknown.kind,
            "payload": unknown.payload,
        }),
    }
}

fn action_command(arguments: &Value) -> Option<&str> {
    arguments
        .get("command")
        .or_else(|| {
            arguments
                .get("arguments")
                .and_then(|value| value.get("command"))
        })
        .or_else(|| arguments.get("args").and_then(|value| value.get("command")))
        .and_then(Value::as_str)
        .filter(|command| !command.trim().is_empty())
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
        WorkerOutcomeKind::Failed
        | WorkerOutcomeKind::TimedOut
        | WorkerOutcomeKind::Stalled
        | WorkerOutcomeKind::Detached
        | WorkerOutcomeKind::CancelFailed => RunStatus::Failed,
    }
}

/// Rehydration result containing the new session and context for the prompt
pub struct RehydrationResult {
    pub session: ActiveSession,
    pub context: Option<String>,
    pub old_conversation_id: String,
}

impl std::fmt::Debug for RehydrationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RehydrationResult")
            .field("session", &"<ActiveSession>")
            .field("context", &self.context)
            .field("old_conversation_id", &self.old_conversation_id)
            .finish()
    }
}

/// Options for conversation rehydration
#[derive(Debug, Clone)]
pub struct RehydrationOptions {
    /// Reason for rehydration (for logging/metrics)
    pub reason: String,
    /// Whether to summarize the old conversation for context
    pub summarize: bool,
    /// Maximum events to include in summary
    pub max_summary_events: usize,
}

impl Default for RehydrationOptions {
    fn default() -> Self {
        Self {
            reason: "explicit rehydration request".to_string(),
            summarize: true,
            max_summary_events: 50,
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

/// Compute the remaining wait duration before the next event timeout fires.
///
/// The timeout is the earlier of:
/// 1. The next token-accumulation poll (wall-clock `Instant`).
/// 2. The tracker's stall deadline (logical `TimestampMs`), converted to a
///    wall-clock duration with the caller's sampled logical timestamp.
fn compute_timeout_duration(
    tracker: &LivenessTracker,
    next_token_accumulation: Instant,
    now: Instant,
    now_ts: TimestampMs,
) -> Duration {
    let token_remaining = next_token_accumulation.saturating_duration_since(now);

    let stall_remaining = tracker
        .stall_deadline_at()
        .map_or(Duration::MAX, |deadline_ts| {
            let remaining_ms = deadline_ts.as_u64().saturating_sub(now_ts.as_u64());
            Duration::from_millis(remaining_ms)
        });

    token_remaining
        .min(stall_remaining)
        .max(Duration::from_millis(1))
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
    use std::{collections::HashSet, sync::Arc};

    use crate::opensymphony_domain::{
        BlockerRef, IssueRef, IssueState, IssueStateCategory, WorkerOutcomeKind,
    };
    use crate::opensymphony_testkit::FakeOpenHandsServer;
    use axum::{Json, Router, extract::State, routing::post};
    use tokio::{net::TcpListener, sync::Mutex};

    use super::super::TransportConfig;
    use super::*;

    fn must<T, E: std::fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn action_event_payload_surfaces_command_when_message_is_generic() {
        let event = EventEnvelope::new(
            "evt-action",
            Utc::now(),
            "agent",
            "ActionEvent",
            serde_json::json!({
                "action": {
                    "tool_name": "terminal",
                    "arguments": { "command": "npm test -- apps/desktop" }
                }
            }),
        );

        assert_eq!(
            summarize_event(&event),
            "terminal: npm test -- apps/desktop"
        );
        let payload = runtime_event_payload(&event);
        assert_eq!(
            payload.get("tool_name").and_then(Value::as_str),
            Some("terminal")
        );
        assert_eq!(
            payload
                .get("arguments")
                .and_then(|arguments| arguments.get("command"))
                .and_then(Value::as_str),
            Some("npm test -- apps/desktop")
        );
    }

    #[tokio::test]
    async fn fetch_memory_context_from_server_calls_mcp_context_with_worker_scope() {
        let requests = Arc::new(Mutex::new(Vec::<Value>::new()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("memory test listener should bind");
        let address = listener
            .local_addr()
            .expect("memory test listener should expose an address");
        let app = Router::new()
            .route("/mcp", post(memory_test_mcp))
            .with_state(requests.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("memory test server should run");
        });
        let access = MemoryWorkerAccess {
            endpoint: format!("http://{address}/mcp"),
            token: Some("read-token".to_string()),
            project: Some("project-alpha".to_string()),
            execution_repo: Some("/tmp/repo-alpha".to_string()),
        };

        let issue = NormalizedIssue {
            id: must(IssueId::new("issue-999")),
            identifier: must(IssueIdentifier::new("COE-999")),
            title: "Memory server context".to_string(),
            description: Some("Use deterministic worker issue facts.".to_string()),
            priority: None,
            state: IssueState {
                id: None,
                name: "In Progress".to_string(),
                category: IssueStateCategory::Active,
            },
            branch_name: None,
            url: None,
            labels: vec!["area:memory".to_string()],
            parent_id: None,
            blocked_by: vec![BlockerRef {
                id: Some(must(IssueId::new("issue-100"))),
                identifier: Some(must(IssueIdentifier::new("COE-100"))),
                state: Some("Done".to_string()),
                created_at: None,
                updated_at: None,
            }],
            sub_issues: vec![IssueRef {
                id: must(IssueId::new("issue-101")),
                identifier: must(IssueIdentifier::new("COE-101")),
                state: "Done".to_string(),
            }],
            created_at: None,
            updated_at: None,
        };

        let context = fetch_memory_context_from_server(&access, &issue)
            .await
            .expect("memory server context should load");

        assert_eq!(context, "# Memory Context: COE-999\n");
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request["method"], "tools/call");
        assert_eq!(request["params"]["name"], "memory.context");
        assert_eq!(request["params"]["arguments"]["issue"], "COE-999");
        assert_eq!(
            request["params"]["arguments"]["currentIssue"]["labels"][0],
            "area:memory"
        );
        assert_eq!(
            request["params"]["arguments"]["currentIssue"]["children"][0]["identifier"],
            "COE-101"
        );
        assert_eq!(
            request["params"]["arguments"]["currentIssue"]["blockedBy"][0]["identifier"],
            "COE-100"
        );
        assert_eq!(request["params"]["arguments"]["project"], "project-alpha");
        assert_eq!(
            request["params"]["arguments"]["projectSet"],
            "project-alpha"
        );
        assert_eq!(request["params"]["arguments"]["repo"], "/tmp/repo-alpha");
        task.abort();
    }

    async fn memory_test_mcp(
        State(requests): State<Arc<Mutex<Vec<Value>>>>,
        Json(request): Json<Value>,
    ) -> Json<Value> {
        requests.lock().await.push(request.clone());
        Json(json!({
            "jsonrpc": "2.0",
            "id": request.get("id").cloned().unwrap_or(Value::Null),
            "result": {
                "content": [
                    { "type": "text", "text": "# Memory Context: COE-999\n" }
                ]
            }
        }))
    }

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
                    replay_existing_events_on_attach: false,
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
                reuse_policy: IssueSessionReusePolicy::PerIssue,
                runtime_stream: RuntimeStreamConfig {
                    readiness_timeout: Duration::from_secs(2),
                    reconnect_initial_backoff: Duration::from_millis(25),
                    reconnect_max_backoff: Duration::from_millis(25),
                    max_reconnect_attempts: 1,
                    replay_existing_events_on_attach: false,
                },
                terminal_wait_timeout: Duration::from_millis(25),
                total_runtime_cap_ms: None,
                finished_drain_timeout: Duration::from_millis(25),
                memory: None,
            },
        );

        // Create a minimal ActiveSession for the test
        let manifest = IssueConversationManifest {
            issue_id: must(IssueId::new("lin_test")),
            identifier: must(IssueIdentifier::new("TEST-123")),
            conversation_id: must(ConversationId::new(
                conversation.conversation_id.to_string(),
            )),
            reuse_policy: "per_issue".to_string(),
            server_base_url: None,
            transport_target: None,
            http_auth_mode: None,
            websocket_auth_mode: None,
            websocket_query_param_name: None,
            persistence_dir: PathBuf::from("/tmp/test"),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_attached_at: Utc::now(),
            launch_profile: None,
            llm_config_fingerprint: None,
            fresh_conversation: true,
            workflow_prompt_seeded: false,
            reset_reason: None,
            runtime_contract_version: None,
            last_prompt_kind: None,
            last_prompt_at: None,
            last_prompt_path: None,
            last_execution_status: None,
            last_event_id: None,
            last_event_kind: None,
            last_event_at: None,
            last_event_summary: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            last_token_accumulation_at: None,
        };

        let mut session = ActiveSession {
            stream,
            manifest,
            prompt_kind: IssueSessionPromptKind::Full,
            prompt_path: None,
        };

        let outcome = runner
            .await_terminal_outcome(&mut session, &baseline_event_ids, &mut ())
            .await;

        assert_eq!(outcome.kind, WorkerOutcomeKind::Succeeded);
        assert_eq!(
            session.stream.state_mirror().terminal_status(),
            Some(TerminalExecutionStatus::Finished)
        );
    }

    #[test]
    fn token_accumulation_extracts_tokens_from_llm_completion_events() {
        use chrono::Utc;
        use serde_json::json;

        // Create events with different token formats
        let event1 = EventEnvelope::new(
            "evt-1",
            Utc::now(),
            "llm",
            "LLMCompletionLogEvent",
            json!({
                "model": "gpt-4",
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 50,
                    "total_tokens": 150
                }
            }),
        );

        let event2 = EventEnvelope::new(
            "evt-2",
            Utc::now() + chrono::Duration::milliseconds(100),
            "llm",
            "LLMCompletionLogEvent",
            json!({
                "model": "gpt-4",
                "input_tokens": 200,
                "output_tokens": 75
            }),
        );

        let event3 = EventEnvelope::new(
            "evt-3",
            Utc::now() + chrono::Duration::milliseconds(200),
            "llm",
            "LLMCompletionLogEvent",
            json!({
                "model": "gpt-4",
                "tokens": 300
            }),
        );

        // Create a simple event cache and add the events
        let mut cache = super::super::events::EventCache::new();
        cache.insert(event1);
        cache.insert(event2);
        cache.insert(event3);

        // Verify tokens are extracted correctly
        let mut total_input = 0u64;
        let mut total_output = 0u64;

        for event in cache.items() {
            if let KnownEvent::LlmCompletionLog(llm_event) = KnownEvent::from_envelope(event)
                && let Some((input, output)) = llm_event.token_usage()
            {
                total_input += input;
                total_output += output;
            }
        }

        // Expected: 100 + 200 + 0 = 300 input tokens
        // Expected: 50 + 75 + 300 = 425 output tokens
        assert_eq!(total_input, 300, "input tokens should be 100 + 200 + 0");
        assert_eq!(total_output, 425, "output tokens should be 50 + 75 + 300");
    }

    #[test]
    fn condenser_tool_matching_error_detection() {
        // Valid condenser tool-matching errors from OpenHands SDK's ToolCallMatching
        // OpenAI-style tool call IDs: 'chatcmpl-tool-{hex}'
        assert!(is_condenser_tool_matching_error(
            "'chatcmpl-tool-bcea2761df6a8821'"
        ));
        assert!(is_condenser_tool_matching_error("'chatcmpl-tool-abc123'"));

        // Not tool-matching errors
        assert!(!is_condenser_tool_matching_error(
            "prompt is too long: 1000"
        ));
        assert!(!is_condenser_tool_matching_error("some other error"));
        assert!(!is_condenser_tool_matching_error(
            "tool error without quotes"
        ));
        assert!(!is_condenser_tool_matching_error(
            "'not-a-tool-matching-error'"
        ));
        assert!(!is_condenser_tool_matching_error("'no-tool-here'"));
        assert!(!is_condenser_tool_matching_error("'other-tool-id-123'")); // not chatcmpl format

        // Test outcome detection
        let tool_matching_outcome = NormalizedOutcome {
            kind: WorkerOutcomeKind::Failed,
            summary: "test".to_string(),
            error: Some("'chatcmpl-tool-xyz'".to_string()),
        };
        assert!(is_condenser_tool_matching_outcome(&tool_matching_outcome));

        let other_outcome = NormalizedOutcome {
            kind: WorkerOutcomeKind::Failed,
            summary: "test".to_string(),
            error: Some("some other error".to_string()),
        };
        assert!(!is_condenser_tool_matching_outcome(&other_outcome));

        let context_overflow_outcome = NormalizedOutcome {
            kind: WorkerOutcomeKind::Failed,
            summary: "test".to_string(),
            error: Some("prompt is too long: 1000".to_string()),
        };
        assert!(!is_condenser_tool_matching_outcome(
            &context_overflow_outcome
        ));
        assert!(is_context_overflow_outcome(&context_overflow_outcome));
    }

    #[test]
    fn liveness_tracker_slides_deadline_on_progress_signals() {
        let idle = DurationMs::new(300_000); // 5 minute idle timeout
        let cap = DurationMs::new(3_600_000); // 1 hour total runtime cap
        let mut tracker = LivenessTracker::with_runtime_cap(idle, Some(cap));
        let start = TimestampMs::new(1000);

        // Before start, should not be stalled
        assert!(!tracker.is_stalled_at(TimestampMs::new(1000)));

        // Mark start
        tracker.mark_started(start);
        assert!(!tracker.is_stalled_at(start));

        // At 4 minutes, should not be stalled (within idle timeout)
        assert!(!tracker.is_stalled_at(TimestampMs::new(1000 + 240_000)));

        // Record events at 3 minutes — slides idle deadline forward
        let t3 = TimestampMs::new(1000 + 180_000);
        tracker.record_event(t3);
        // Now at 7 minutes total (3 min + 5 min idle), should not be stalled
        assert!(!tracker.is_stalled_at(TimestampMs::new(1000 + 180_000 + 240_000)));

        // Record tokens at 6 minutes — slides deadline again
        let t6 = TimestampMs::new(1000 + 360_000);
        tracker.record_tokens(100, 50, t6);
        // Now at 10 minutes total (6 min + 4 min idle), should not be stalled
        assert!(!tracker.is_stalled_at(TimestampMs::new(1000 + 360_000 + 240_000)));

        // Past the idle deadline with no progress — should be stalled
        assert!(tracker.is_stalled_at(TimestampMs::new(1000 + 360_000 + 360_000)));

        // Past the total runtime cap — should be stalled even with recent progress
        tracker.mark_started(start);
        tracker.record_event(TimestampMs::new(start.as_u64() + 3_500_000));
        assert!(tracker.is_stalled_at(TimestampMs::new(start.as_u64() + 3_600_000 + 1)));
    }

    #[test]
    fn liveness_tracker_snapshot_produces_correct_deltas() {
        let mut tracker = LivenessTracker::new(DurationMs::new(60_000));
        tracker.mark_started(TimestampMs::new(1000));
        tracker.record_event(TimestampMs::new(1100));
        tracker.record_tokens(50, 25, TimestampMs::new(1200));

        let initial = RuntimeProgressSnapshot::initial(RuntimeLivenessPhase::RunningTurn);
        let snapshot = tracker.snapshot(&initial);

        assert_eq!(snapshot.phase, RuntimeLivenessPhase::RunningTurn);
        assert_eq!(snapshot.event_count, 1);
        assert_eq!(snapshot.event_delta, 1);
        assert_eq!(snapshot.input_tokens, 50);
        assert_eq!(snapshot.input_token_delta, 50);
        assert_eq!(snapshot.output_tokens, 25);
        assert_eq!(snapshot.output_token_delta, 25);

        // Second snapshot should have zero deltas if no new progress
        let snapshot2 = tracker.snapshot(&snapshot);
        assert_eq!(snapshot2.event_delta, 0);
        assert_eq!(snapshot2.input_token_delta, 0);
        assert_eq!(snapshot2.output_token_delta, 0);

        // Re-reading the same cumulative totals should not produce phantom deltas.
        let unchanged = tracker.record_tokens(50, 25, TimestampMs::new(1300));
        assert!(!unchanged);
        let snapshot3 = tracker.snapshot(&snapshot2);
        assert_eq!(snapshot3.input_token_delta, 0);
        assert_eq!(snapshot3.output_token_delta, 0);

        // Higher cumulative totals should produce non-zero deltas.
        tracker.record_event(TimestampMs::new(1300));
        tracker.record_tokens(60, 30, TimestampMs::new(1400));
        let snapshot4 = tracker.snapshot(&snapshot3);
        assert_eq!(snapshot4.event_delta, 1);
        assert_eq!(snapshot4.input_token_delta, 10);
        assert_eq!(snapshot4.output_token_delta, 5);
    }

    #[test]
    fn liveness_tracker_waiting_on_prior_turn_phase() {
        let tracker = LivenessTracker::new(DurationMs::new(60_000));
        let initial = RuntimeProgressSnapshot::initial(RuntimeLivenessPhase::WaitingOnPriorTurn);
        let snapshot = tracker.snapshot(&initial);

        assert_eq!(snapshot.phase, RuntimeLivenessPhase::WaitingOnPriorTurn);
        assert_eq!(snapshot.event_count, 0);
        assert!(snapshot.last_activity_at.is_none());
    }

    #[test]
    fn liveness_tracker_status_change_is_liveness_signal() {
        let mut tracker = LivenessTracker::new(DurationMs::new(60_000));
        tracker.mark_started(TimestampMs::new(1000));

        // First status change should advance activity
        let changed = tracker.record_status_change("running", TimestampMs::new(1100));
        assert!(changed);

        // Same status should not advance activity
        let changed = tracker.record_status_change("running", TimestampMs::new(1200));
        assert!(!changed);

        // Different status should advance activity
        let changed = tracker.record_status_change("finished", TimestampMs::new(1300));
        assert!(changed);
    }

    #[test]
    fn compute_timeout_duration_uses_tracker_deadline_with_sampled_logical_time() {
        let mut tracker = LivenessTracker::new(DurationMs::new(5_000));
        tracker.mark_started(TimestampMs::new(1_000));

        let now = Instant::now();
        let timeout = compute_timeout_duration(
            &tracker,
            now + Duration::from_secs(30),
            now,
            TimestampMs::new(3_000),
        );

        assert_eq!(timeout, Duration::from_millis(3_000));
    }

    #[test]
    fn compute_timeout_duration_uses_token_poll_when_tracker_has_no_deadline() {
        let tracker = LivenessTracker::new(DurationMs::new(5_000));
        let now = Instant::now();

        let timeout = compute_timeout_duration(
            &tracker,
            now + Duration::from_millis(750),
            now,
            TimestampMs::new(3_000),
        );

        assert_eq!(timeout, Duration::from_millis(750));
    }
}
