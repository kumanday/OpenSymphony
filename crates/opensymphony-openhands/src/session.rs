//! Orchestrator-facing issue session runner built on top of the OpenHands client.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use uuid::Uuid;

use opensymphony_domain::{IssueRef, PromptKind, PromptSet, SessionOutcome};
use opensymphony_workspace::{
    ConversationManifest, WorkspaceError, WorkspaceLayout, write_prompt_artifact,
};

use crate::client::OpenHandsClient;
use crate::config::{ConversationConfig, WebSocketConfig};
use crate::error::Result;
use crate::stream::AttachedConversation;
use crate::wire::{
    ConfirmationPolicy, ConversationInfo, CreateConversationRequest, OpenHandsWorkspace,
    RemoteExecutionStatus,
};

const CREATE_REQUEST_ARTIFACT: &str = "create-conversation-request.json";
const LAST_STATE_ARTIFACT: &str = "last-conversation-state.json";
const LAST_FULL_PROMPT_ARTIFACT: &str = "last-full-prompt.md";
const LAST_CONTINUATION_PROMPT_ARTIFACT: &str = "last-continuation-prompt.md";

/// Input required to execute one OpenHands issue session attempt.
#[derive(Clone, Debug)]
pub struct IssueSessionRequest {
    /// Tracker-normalized issue identity.
    pub issue: IssueRef,
    /// Resolved workspace layout owned by OpenSymphony.
    pub workspace: WorkspaceLayout,
    /// Full and continuation prompts derived by the workflow layer.
    pub prompts: PromptSet,
}

/// Issue session runner that creates or reuses one conversation per issue workspace.
#[derive(Clone, Debug)]
pub struct IssueSessionRunner {
    client: OpenHandsClient,
    conversation: ConversationConfig,
    websocket: WebSocketConfig,
    run_timeout: Duration,
}

impl IssueSessionRunner {
    /// Builds a new runner with the default one-hour run wait timeout.
    #[must_use]
    pub fn new(
        client: OpenHandsClient,
        conversation: ConversationConfig,
        websocket: WebSocketConfig,
    ) -> Self {
        Self {
            client,
            conversation,
            websocket,
            run_timeout: Duration::from_secs(3_600),
        }
    }

    /// Overrides the terminal wait timeout used after `POST /run`.
    #[must_use]
    pub fn with_run_timeout(mut self, run_timeout: Duration) -> Self {
        self.run_timeout = run_timeout;
        self
    }

    /// Executes one issue attempt against the configured OpenHands runtime.
    pub async fn execute(&self, request: &IssueSessionRequest) -> Result<SessionOutcome> {
        request.workspace.create()?;

        let prepared = self.prepare_conversation(request).await?;

        if let Some(create_request) = &prepared.create_request {
            write_json_artifact(
                request
                    .workspace
                    .openhands_dir
                    .join(CREATE_REQUEST_ARTIFACT),
                create_request,
            )?;
        }

        let prompt = match prepared.prompt_kind {
            PromptKind::Fresh => &request.prompts.full_prompt,
            PromptKind::Continuation => &request.prompts.continuation_prompt,
        };
        let prompt_artifact = match prepared.prompt_kind {
            PromptKind::Fresh => request
                .workspace
                .prompts_dir
                .join(LAST_FULL_PROMPT_ARTIFACT),
            PromptKind::Continuation => request
                .workspace
                .prompts_dir
                .join(LAST_CONTINUATION_PROMPT_ARTIFACT),
        };
        write_prompt_artifact(prompt_artifact, prompt)?;

        let mut attached = AttachedConversation::attach(
            self.client.clone(),
            prepared.manifest.conversation_id.clone(),
            self.websocket.clone(),
        )
        .await?;
        if run_in_progress(attached.state().await.execution_status) {
            let _ = attached.wait_for_terminal(self.run_timeout).await?;
        }
        self.client
            .send_user_message(
                &prepared.manifest.conversation_id,
                &crate::wire::SendMessageRequest::user_text(prompt),
            )
            .await?;
        // Persist only after the prompt is accepted so retries keep using the full prompt if the
        // first transport attempt fails before the conversation actually contains it.
        prepared
            .manifest
            .save(&request.workspace.conversation_manifest_path)?;
        loop {
            let run = self
                .client
                .run_conversation(&prepared.manifest.conversation_id)
                .await?;
            if !run.already_running {
                break;
            }
            let _ = attached.wait_for_terminal(self.run_timeout).await?;
        }
        attached.clear_execution_status().await;
        let final_info = attached.wait_for_terminal(self.run_timeout).await?;
        write_json_artifact(
            request.workspace.openhands_dir.join(LAST_STATE_ARTIFACT),
            &final_info,
        )?;

        let execution_status = final_info
            .execution_status
            .map(|status| status.to_domain())
            .unwrap_or(opensymphony_domain::ExecutionStatus::Unknown);
        let event_count = attached.event_count().await;
        attached.close().await?;

        Ok(SessionOutcome {
            conversation_id: prepared.manifest.conversation_id,
            prompt_kind: prepared.prompt_kind,
            execution_status,
            event_count,
        })
    }

    async fn prepare_conversation(
        &self,
        request: &IssueSessionRequest,
    ) -> Result<PreparedConversation> {
        let now = Utc::now();
        let manifest_path = &request.workspace.conversation_manifest_path;
        let (existing, corrupted_manifest) = match ConversationManifest::load(manifest_path) {
            Ok(existing) => (existing, false),
            Err(WorkspaceError::Json { .. }) => (None, true),
            Err(error) => return Err(error.into()),
        };
        let persistence_dir = self.persistence_dir_string(&request.workspace);
        let server_base_url = self.client.transport().base_url.to_string();

        match existing {
            Some(mut manifest)
                if manifest.issue_id == request.issue.issue_id
                    && manifest.identifier == request.issue.identifier
                    && manifest.runtime_contract_version
                        == self.conversation.runtime_contract_version =>
            {
                if let Some(conversation) = self
                    .client
                    .get_conversation(&manifest.conversation_id)
                    .await?
                {
                    if conversation_matches_workspace(
                        &conversation,
                        &request.workspace,
                        persistence_dir.as_str(),
                    ) {
                        manifest.last_attached_at = now;
                        manifest.fresh_conversation = false;
                        manifest.server_base_url = server_base_url;
                        manifest.persistence_dir = persistence_dir;
                        manifest.reset_reason = None;
                        return Ok(PreparedConversation {
                            manifest,
                            prompt_kind: PromptKind::Continuation,
                            create_request: None,
                        });
                    }

                    let create_request = self.build_create_request(&request.workspace, None);
                    let conversation = self.client.create_conversation(&create_request).await?;
                    manifest.last_attached_at = now;
                    manifest.fresh_conversation = true;
                    manifest.server_base_url = server_base_url;
                    manifest.persistence_dir = persistence_dir;
                    manifest.reset_reason = Some("workspace_binding_changed".to_string());
                    manifest.conversation_id = conversation.id;
                    return Ok(PreparedConversation {
                        manifest,
                        prompt_kind: PromptKind::Fresh,
                        create_request: Some(create_request),
                    });
                }

                let create_request = self.build_create_request(
                    &request.workspace,
                    Some(manifest.conversation_id.clone()),
                );
                let conversation = self.client.create_conversation(&create_request).await?;
                let rehydrated_history = conversation.id == manifest.conversation_id
                    && !self
                        .client
                        .search_events_all(&conversation.id)
                        .await?
                        .is_empty();
                manifest.last_attached_at = now;
                manifest.fresh_conversation = !rehydrated_history;
                manifest.server_base_url = server_base_url;
                manifest.persistence_dir = persistence_dir;
                manifest.reset_reason = if rehydrated_history {
                    None
                } else {
                    Some("conversation_missing".to_string())
                };
                manifest.conversation_id = conversation.id;
                Ok(PreparedConversation {
                    manifest,
                    prompt_kind: if rehydrated_history {
                        PromptKind::Continuation
                    } else {
                        PromptKind::Fresh
                    },
                    create_request: Some(create_request),
                })
            }
            Some(manifest) => {
                let create_request = self.build_create_request(&request.workspace, None);
                let conversation = self.client.create_conversation(&create_request).await?;
                Ok(PreparedConversation {
                    manifest: ConversationManifest {
                        issue_id: request.issue.issue_id.clone(),
                        identifier: request.issue.identifier.clone(),
                        conversation_id: conversation.id,
                        server_base_url,
                        persistence_dir,
                        created_at: now,
                        last_attached_at: now,
                        fresh_conversation: true,
                        reset_reason: Some(reset_reason(&manifest)),
                        runtime_contract_version: self
                            .conversation
                            .runtime_contract_version
                            .clone(),
                    },
                    prompt_kind: PromptKind::Fresh,
                    create_request: Some(create_request),
                })
            }
            None => {
                let create_request = self.build_create_request(&request.workspace, None);
                let conversation = self.client.create_conversation(&create_request).await?;
                Ok(PreparedConversation {
                    manifest: ConversationManifest {
                        issue_id: request.issue.issue_id.clone(),
                        identifier: request.issue.identifier.clone(),
                        conversation_id: conversation.id,
                        server_base_url,
                        persistence_dir,
                        created_at: now,
                        last_attached_at: now,
                        fresh_conversation: true,
                        reset_reason: corrupted_manifest.then(|| "corrupted_manifest".to_string()),
                        runtime_contract_version: self
                            .conversation
                            .runtime_contract_version
                            .clone(),
                    },
                    prompt_kind: PromptKind::Fresh,
                    create_request: Some(create_request),
                })
            }
        }
    }

    fn build_create_request(
        &self,
        workspace: &WorkspaceLayout,
        conversation_id: Option<String>,
    ) -> CreateConversationRequest {
        let mut agent = self.conversation.agent.clone();
        let persistence_dir = self.persistence_dir_string(workspace);
        for tool in &mut agent.tools {
            tool.name = tool.registry_name();
        }
        CreateConversationRequest {
            agent,
            workspace: OpenHandsWorkspace {
                working_dir: workspace.issue_workspace.display().to_string(),
                kind: None,
                extra: serde_json::Map::new(),
            },
            conversation_id: conversation_id.or_else(|| Some(Uuid::new_v4().to_string())),
            persistence_dir: Some(persistence_dir),
            confirmation_policy: if self.conversation.confirmation_policy.kind.is_empty() {
                ConfirmationPolicy::never_confirm()
            } else {
                self.conversation.confirmation_policy.clone()
            },
            initial_message: None,
            max_iterations: self.conversation.max_iterations,
            stuck_detection: self.conversation.stuck_detection,
            autotitle: self.conversation.autotitle,
            hook_config: self.conversation.hook_config.clone(),
            plugins: self.conversation.plugins.clone(),
            secrets: self.conversation.secrets.clone(),
            tool_module_qualnames: self
                .conversation
                .agent
                .tools
                .iter()
                .filter_map(|tool| {
                    tool.module_qualname()
                        .map(|module| (tool.registry_name(), module.to_string()))
                })
                .collect(),
        }
    }

    fn persistence_dir_path(&self, workspace: &WorkspaceLayout) -> PathBuf {
        workspace
            .issue_workspace
            .join(&self.conversation.persistence_dir_relative)
    }

    fn persistence_dir_string(&self, workspace: &WorkspaceLayout) -> String {
        self.persistence_dir_path(workspace).display().to_string()
    }
}

#[derive(Clone, Debug)]
struct PreparedConversation {
    manifest: ConversationManifest,
    prompt_kind: PromptKind,
    create_request: Option<CreateConversationRequest>,
}

fn write_json_artifact(path: impl AsRef<Path>, value: &impl serde::Serialize) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(value)?;
    fs::write(path, data)?;
    Ok(())
}

fn conversation_matches_workspace(
    conversation: &ConversationInfo,
    workspace: &WorkspaceLayout,
    expected_persistence_dir: &str,
) -> bool {
    let expected_working_dir = workspace.issue_workspace.display().to_string();

    conversation.workspace.working_dir == expected_working_dir
        && conversation.persistence_dir.as_deref() == Some(expected_persistence_dir)
}

fn reset_reason(manifest: &ConversationManifest) -> String {
    if manifest.runtime_contract_version.is_empty() {
        "missing_runtime_contract_version".to_string()
    } else {
        format!(
            "runtime_contract_changed:{}",
            manifest.runtime_contract_version
        )
    }
}

fn run_in_progress(status: Option<RemoteExecutionStatus>) -> bool {
    matches!(
        status,
        Some(
            RemoteExecutionStatus::Running
                | RemoteExecutionStatus::Paused
                | RemoteExecutionStatus::WaitingForConfirmation
        )
    )
}
