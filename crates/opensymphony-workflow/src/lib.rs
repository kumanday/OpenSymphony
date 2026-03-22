mod error;
mod loader;
mod model;
mod resolve;
mod template;

use std::path::Path;

use serde::Serialize;

pub use error::{PromptTemplateError, WorkflowConfigError, WorkflowLoadError};
pub use model::{
    AgentConfig, AgentFrontMatter, Environment, HooksConfig, HooksFrontMatter, IntegerLike,
    OpenHandsConfig, OpenHandsConfirmationPolicy, OpenHandsConfirmationPolicyFrontMatter,
    OpenHandsConversationAgentConfig, OpenHandsConversationAgentFrontMatter,
    OpenHandsConversationConfig, OpenHandsConversationFrontMatter, OpenHandsFrontMatter,
    OpenHandsLlmConfig, OpenHandsLlmFrontMatter, OpenHandsLocalServerConfig,
    OpenHandsLocalServerFrontMatter, OpenHandsMcpConfig, OpenHandsMcpFrontMatter,
    OpenHandsStdioServerConfig, OpenHandsStdioServerFrontMatter, OpenHandsTransportConfig,
    OpenHandsTransportFrontMatter, OpenHandsWebSocketConfig, OpenHandsWebSocketFrontMatter,
    PollingConfig, PollingFrontMatter, ProcessEnvironment, PromptContext, ResolvedWorkflow,
    TrackerConfig, TrackerFrontMatter, TrackerKind, WorkflowConfig, WorkflowDefinition,
    WorkflowExtensions, WorkflowFrontMatter, WorkspaceConfig, WorkspaceFrontMatter,
    DEFAULT_PROMPT_TEMPLATE,
};

pub const CRATE_NAME: &str = "opensymphony-workflow";

impl WorkflowDefinition {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, WorkflowLoadError> {
        loader::load_workflow_from_path(path.as_ref())
    }

    pub fn parse(source: &str) -> Result<Self, WorkflowLoadError> {
        loader::parse_workflow(source)
    }

    pub fn effective_prompt_template(&self) -> &str {
        if self.prompt_template.trim().is_empty() {
            DEFAULT_PROMPT_TEMPLATE
        } else {
            &self.prompt_template
        }
    }

    pub fn resolve<E: Environment>(
        &self,
        base_dir: &Path,
        env: &E,
    ) -> Result<ResolvedWorkflow, WorkflowConfigError> {
        resolve::resolve_workflow(self, base_dir, env)
    }

    pub fn resolve_with_process_env(
        &self,
        base_dir: &Path,
    ) -> Result<ResolvedWorkflow, WorkflowConfigError> {
        self.resolve(base_dir, &ProcessEnvironment)
    }

    pub fn render_prompt<T: Serialize>(
        &self,
        issue: &T,
        attempt: Option<u32>,
    ) -> Result<String, PromptTemplateError> {
        template::render_prompt(self.effective_prompt_template(), issue, attempt)
    }
}

impl std::str::FromStr for WorkflowDefinition {
    type Err = WorkflowLoadError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::parse(source)
    }
}

impl ResolvedWorkflow {
    pub fn effective_prompt_template(&self) -> &str {
        if self.prompt_template.trim().is_empty() {
            DEFAULT_PROMPT_TEMPLATE
        } else {
            &self.prompt_template
        }
    }

    pub fn render_prompt<T: Serialize>(
        &self,
        issue: &T,
        attempt: Option<u32>,
    ) -> Result<String, PromptTemplateError> {
        template::render_prompt(self.effective_prompt_template(), issue, attempt)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };

    use serde::Serialize;

    use super::{
        model::{
            DEFAULT_HOOK_TIMEOUT_MS, DEFAULT_LINEAR_ENDPOINT, DEFAULT_MAX_CONCURRENT_AGENTS,
            DEFAULT_MAX_RETRY_BACKOFF_MS, DEFAULT_MAX_TURNS, DEFAULT_OPENHANDS_BASE_URL,
            DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND, DEFAULT_OPENHANDS_PERSISTENCE_DIR,
            DEFAULT_OPENHANDS_QUERY_PARAM_NAME, DEFAULT_OPENHANDS_READY_TIMEOUT_MS,
            DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS, DEFAULT_OPENHANDS_RECONNECT_MAX_MS,
            DEFAULT_POLL_INTERVAL_MS, DEFAULT_PROMPT_TEMPLATE, DEFAULT_STALL_TIMEOUT_MS,
            DEFAULT_WORKSPACE_ROOT,
        },
        PromptTemplateError, TrackerKind, WorkflowConfigError, WorkflowDefinition,
        WorkflowLoadError,
    };

    #[derive(Debug, Serialize)]
    struct TestIssue<'a> {
        identifier: &'a str,
        title: &'a str,
        state: &'a str,
        description: Option<&'a str>,
        labels: Vec<&'a str>,
    }

    #[test]
    fn parses_valid_front_matter_and_prompt_body() {
        let workflow =
            WorkflowDefinition::parse(sample_workflow()).expect("sample workflow should parse");

        assert_eq!(
            workflow.front_matter.tracker.kind.as_deref(),
            Some("linear")
        );
        assert_eq!(
            workflow.front_matter.agent.max_turns,
            Some(super::IntegerLike::Integer(8))
        );
        assert_eq!(
            workflow.prompt_template,
            "\n# Assignment\n\nTicket: {{ issue.identifier }}\n"
        );
    }

    #[test]
    fn parses_workflow_without_front_matter() {
        let workflow = WorkflowDefinition::parse("\n\nPrompt only\n")
            .expect("prompt-only workflow should parse");

        assert_eq!(workflow.front_matter, super::WorkflowFrontMatter::default());
        assert_eq!(workflow.prompt_template, "\n\nPrompt only\n");
    }

    #[test]
    fn rejects_non_map_front_matter() {
        let error = WorkflowDefinition::parse("---\n- nope\n---\nbody")
            .expect_err("list front matter should be rejected");

        assert!(matches!(
            error,
            WorkflowLoadError::WorkflowFrontMatterNotAMap
        ));
    }

    #[test]
    fn treats_unmatched_leading_delimiter_as_prompt_body() {
        let source = "---\n# Assignment\n";
        let workflow = WorkflowDefinition::parse(source)
            .expect("unterminated leading delimiter should fall back to prompt body");

        assert_eq!(workflow.front_matter, super::WorkflowFrontMatter::default());
        assert_eq!(workflow.prompt_template, source);
    }

    #[test]
    fn rejects_unknown_top_level_namespaces() {
        let error = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
openhadns:
  transport:
    base_url: http://127.0.0.1:8000
---
{{ issue.identifier }}
"#,
        )
        .expect_err("unknown namespaces should fail deterministically");

        assert!(matches!(
            error,
            WorkflowLoadError::UnknownTopLevelNamespace { namespace } if namespace == "openhadns"
        ));
    }

    #[test]
    fn accepts_repo_codex_namespace() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
codex:
  command: codex app-server
---
{{ issue.identifier }}
"#,
        )
        .expect("codex namespace should be accepted");

        assert_eq!(
            workflow
                .front_matter
                .codex
                .as_ref()
                .and_then(|codex| codex.get("command")),
            Some(&serde_yaml::Value::String("codex app-server".to_owned()))
        );
    }

    #[test]
    fn loads_checked_in_workflows() {
        let repo_root = repo_root();

        WorkflowDefinition::load_from_path(repo_root.join("WORKFLOW.md"))
            .expect("repo root workflow should parse");
        WorkflowDefinition::load_from_path(repo_root.join("examples/target-repo/WORKFLOW.md"))
            .expect("bundled target repo workflow should parse");
    }

    #[test]
    fn resolves_checked_in_target_repo_workflow() {
        let repo_root = repo_root();
        let workflow =
            WorkflowDefinition::load_from_path(repo_root.join("examples/target-repo/WORKFLOW.md"))
                .expect("bundled target repo workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let resolved = workflow
            .resolve(&repo_root.join("examples/target-repo"), &env)
            .expect("bundled target repo workflow should resolve");

        assert!(matches!(resolved.config.tracker.kind, TrackerKind::Linear));
        assert_eq!(resolved.config.tracker.project_slug, "sample-project");
        assert_eq!(
            resolved.config.tracker.active_states,
            vec!["Todo".to_string(), "In Progress".to_string()]
        );
        assert_eq!(
            resolved.config.tracker.terminal_states,
            vec!["Done".to_string()]
        );
        assert_eq!(resolved.extensions.openhands.local_server.command, None);
    }

    #[test]
    fn reports_missing_workflow_file() {
        let path = Path::new("/definitely/missing/WORKFLOW.md");
        let error = WorkflowDefinition::load_from_path(path)
            .expect_err("missing workflow file should fail");

        assert!(matches!(
            error,
            WorkflowLoadError::MissingWorkflowFile { path: missing } if missing == path
        ));
    }

    #[test]
    fn leaves_openhands_local_server_command_unset_when_omitted() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let resolved = workflow
            .resolve(Path::new("/repo/target"), &env)
            .expect("workflow should resolve");

        assert_eq!(resolved.extensions.openhands.local_server.command, None);
    }

    #[test]
    fn resolves_defaults_and_openhands_extension() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
    - Closed
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([
            ("LINEAR_API_KEY", "linear-token"),
            ("HOME", "/Users/tester"),
        ]);

        let resolved = workflow
            .resolve(Path::new("/repo"), &env)
            .expect("workflow should resolve");

        assert!(matches!(resolved.config.tracker.kind, TrackerKind::Linear));
        assert_eq!(resolved.config.tracker.endpoint, DEFAULT_LINEAR_ENDPOINT);
        assert_eq!(resolved.config.tracker.api_key, "linear-token");
        assert_eq!(
            resolved.config.polling.interval_ms,
            DEFAULT_POLL_INTERVAL_MS
        );
        assert_eq!(
            resolved.config.workspace.root,
            PathBuf::from(DEFAULT_WORKSPACE_ROOT)
        );
        assert_eq!(resolved.config.hooks.timeout_ms, DEFAULT_HOOK_TIMEOUT_MS);
        assert_eq!(
            resolved.config.agent.max_concurrent_agents,
            DEFAULT_MAX_CONCURRENT_AGENTS
        );
        assert_eq!(resolved.config.agent.max_turns, DEFAULT_MAX_TURNS);
        assert_eq!(
            resolved.config.agent.max_retry_backoff_ms,
            DEFAULT_MAX_RETRY_BACKOFF_MS
        );
        assert_eq!(
            resolved.config.agent.stall_timeout_ms,
            Some(DEFAULT_STALL_TIMEOUT_MS)
        );
        assert_eq!(
            resolved.extensions.openhands.transport.base_url,
            DEFAULT_OPENHANDS_BASE_URL
        );
        assert_eq!(resolved.extensions.openhands.local_server.command, None);
        assert_eq!(
            resolved
                .extensions
                .openhands
                .conversation
                .persistence_dir_relative,
            PathBuf::from(DEFAULT_OPENHANDS_PERSISTENCE_DIR)
        );
        assert_eq!(
            resolved
                .extensions
                .openhands
                .conversation
                .confirmation_policy
                .kind,
            DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND
        );
        assert_eq!(
            resolved.extensions.openhands.conversation.agent.kind,
            "Agent"
        );
        assert_eq!(
            resolved.extensions.openhands.websocket.ready_timeout_ms,
            DEFAULT_OPENHANDS_READY_TIMEOUT_MS
        );
        assert_eq!(
            resolved.extensions.openhands.websocket.reconnect_initial_ms,
            DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS
        );
        assert_eq!(
            resolved.extensions.openhands.websocket.reconnect_max_ms,
            DEFAULT_OPENHANDS_RECONNECT_MAX_MS
        );
        assert_eq!(
            resolved.extensions.openhands.websocket.query_param_name,
            DEFAULT_OPENHANDS_QUERY_PARAM_NAME
        );
        assert!(resolved.extensions.openhands.mcp.stdio_servers.is_empty());
    }

    #[test]
    fn rejects_explicit_openhands_local_server_command() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  local_server:
    command:
      - bash
      - ./scripts/run-openhands.sh
      - --port
      - "9000"
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("explicit local server commands should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.local_server.command",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_local_server_enabled_override() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  local_server:
    enabled: false
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("unsupported local server disablement should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.local_server.enabled",
                ..
            }
        ));
    }

    #[test]
    fn explicit_tracker_api_key_env_reference_must_resolve() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  api_key: ${TRACKER_API_KEY}
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "fallback-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("unset explicit tracker api key env should fail");

        assert!(matches!(
            error,
            WorkflowConfigError::MissingEnvironmentVariable {
                field: "tracker.api_key",
                ..
            }
        ));
    }

    #[test]
    fn resolves_env_substitution_and_path_rules() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ${WORKSPACE_ROOT}
hooks:
  timeout_ms: 0
agent:
  max_turns: "5"
  stall_timeout_ms: 0
  max_concurrent_agents_by_state:
    In Review: 2
openhands:
  transport:
    base_url: ${OPENHANDS_BASE_URL}
  conversation:
    persistence_dir_relative: .cache/openhands
    agent:
      llm:
        model: ${OPENHANDS_MODEL}
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([
            ("LINEAR_API_KEY", "linear-token"),
            ("WORKSPACE_ROOT", "/tmp/workspaces"),
            ("OPENHANDS_BASE_URL", "http://localhost:8000"),
            ("OPENHANDS_MODEL", "gpt-4.1-mini"),
        ]);

        let resolved = workflow
            .resolve(Path::new("/repo/config"), &env)
            .expect("workflow should resolve");

        assert_eq!(
            resolved.config.workspace.root,
            PathBuf::from("/tmp/workspaces")
        );
        assert_eq!(resolved.config.hooks.timeout_ms, DEFAULT_HOOK_TIMEOUT_MS);
        assert_eq!(resolved.config.agent.max_turns, 5);
        assert_eq!(resolved.config.agent.stall_timeout_ms, None);
        assert_eq!(
            resolved
                .config
                .agent
                .max_concurrent_agents_by_state
                .get("in review"),
            Some(&2)
        );
        assert_eq!(
            resolved.extensions.openhands.transport.base_url,
            "http://localhost:8000"
        );
        assert_eq!(
            resolved
                .extensions
                .openhands
                .conversation
                .persistence_dir_relative,
            PathBuf::from(".cache/openhands")
        );
        assert_eq!(
            resolved
                .extensions
                .openhands
                .conversation
                .agent
                .llm
                .as_ref()
                .expect("llm config should exist")
                .model
                .as_deref(),
            Some("gpt-4.1-mini")
        );
        assert_eq!(
            resolved.extensions.openhands.conversation.agent.kind,
            "Agent"
        );
    }

    #[test]
    fn rejects_invalid_openhands_transport_base_urls() {
        for invalid_base_url in [
            "localhost:8000",
            "ws://127.0.0.1:8000",
            "http://127.0.0.1:8000/",
            "https://example.com/runtime",
            "http://127.0.0.1:8000/api",
            "https://example.com/runtime/api/",
        ] {
            let workflow = WorkflowDefinition::parse(&format!(
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
    base_url: {invalid_base_url}
---
{{{{ issue.identifier }}}}
"#
            ))
            .expect("workflow should parse");
            let env = env([("LINEAR_API_KEY", "linear-token")]);

            let error = workflow
                .resolve(Path::new("/repo"), &env)
                .expect_err("invalid OpenHands base URLs should fail during resolution");

            assert!(matches!(
                error,
                WorkflowConfigError::InvalidField {
                    field: "openhands.transport.base_url",
                    ..
                }
            ));
        }
    }

    #[test]
    fn rejects_unsupported_openhands_mcp_stdio_servers() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  mcp:
    stdio_servers:
      - name: linear
        command:
          - opensymphony
          - linear-mcp
          - --stdio
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("workflow-owned mcp stdio servers should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.mcp.stdio_servers",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_conversation_reuse_policy_override() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    reuse_policy: fresh_each_run
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("unsupported reuse policies should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.conversation.reuse_policy",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_agent_option_overrides() {
        for workflow_source in [
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      log_completions: true
---
{{ issue.identifier }}
"#,
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      custom_mode: verbose
---
{{ issue.identifier }}
"#,
        ] {
            let workflow =
                WorkflowDefinition::parse(workflow_source).expect("workflow should parse");
            let env = env([("LINEAR_API_KEY", "linear-token")]);

            let error = workflow
                .resolve(Path::new("/repo"), &env)
                .expect_err("unsupported agent options should fail during resolution");

            assert!(matches!(
                error,
                WorkflowConfigError::InvalidField {
                    field: "openhands.conversation.agent.log_completions",
                    ..
                } | WorkflowConfigError::InvalidField {
                    field: "openhands.conversation.agent",
                    ..
                }
            ));
        }
    }

    #[test]
    fn rejects_openhands_max_iterations_above_u32_range() {
        let workflow = WorkflowDefinition::parse(&format!(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    max_iterations: {}
---
{{{{ issue.identifier }}}}
"#,
            u64::from(u32::MAX) + 1
        ))
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("oversized max_iterations should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.conversation.max_iterations",
                ..
            }
        ));
    }

    #[test]
    fn defaults_confirmation_policy_kind_when_block_omits_it() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    confirmation_policy: {}
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let resolved = workflow
            .resolve(Path::new("/repo"), &env)
            .expect("confirmation policy defaults should resolve");

        assert_eq!(
            resolved
                .extensions
                .openhands
                .conversation
                .confirmation_policy
                .kind,
            DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND
        );
        assert_eq!(
            resolved
                .extensions
                .openhands
                .conversation
                .confirmation_policy
                .kind,
            DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND
        );
    }

    #[test]
    fn rejects_confirmation_policy_options_that_cannot_reach_runtime() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    confirmation_policy:
      max_budget_usd: 5
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("unsupported confirmation policy options should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.conversation.confirmation_policy",
                ..
            }
        ));
    }

    #[test]
    fn rejects_openhands_llm_blocks_without_model() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      llm: {}
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("llm blocks without model should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::MissingRequiredField {
                field: "openhands.conversation.agent.llm.model",
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_transport_session_api_key_env() {
        let workflow = WorkflowDefinition::parse(
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
    session_api_key_env: OPENHANDS_SESSION_API_KEY
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("transport auth overrides should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.transport.session_api_key_env",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_websocket_auth_mode_override() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  websocket:
    auth_mode: header
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("websocket auth mode overrides should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.websocket.auth_mode",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_websocket_query_param_override() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  websocket:
    query_param_name: openhands_token
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("websocket query-param overrides should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.websocket.query_param_name",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_llm_api_key_env_override() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      llm:
        model: ${OPENHANDS_MODEL}
        api_key_env: OPENHANDS_API_KEY
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([
            ("LINEAR_API_KEY", "linear-token"),
            ("OPENHANDS_MODEL", "gpt-4.1"),
        ]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("llm api-key env overrides should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.conversation.agent.llm.api_key_env",
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_openhands_llm_option_overrides() {
        for workflow_source in [
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      llm:
        model: gpt-4.1-mini
        temperature: 0.1
---
{{ issue.identifier }}
"#,
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      llm:
        model: gpt-4.1-mini
        reasoning_effort: high
---
{{ issue.identifier }}
"#,
        ] {
            let workflow =
                WorkflowDefinition::parse(workflow_source).expect("workflow should parse");
            let env = env([("LINEAR_API_KEY", "linear-token")]);

            let error = workflow
                .resolve(Path::new("/repo"), &env)
                .expect_err("unsupported llm options should fail during resolution");

            assert!(matches!(
                error,
                WorkflowConfigError::InvalidField {
                    field: "openhands.conversation.agent.llm",
                    ..
                }
            ));
        }
    }

    #[test]
    fn rejects_unsupported_openhands_llm_base_url_env_override() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    agent:
      llm:
        model: ${OPENHANDS_MODEL}
        base_url_env: OPENHANDS_BASE_URL
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([
            ("LINEAR_API_KEY", "linear-token"),
            ("OPENHANDS_MODEL", "gpt-4.1"),
        ]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("llm base-url env overrides should fail during resolution");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.conversation.agent.llm.base_url_env",
                ..
            }
        ));
    }

    #[test]
    fn rejects_persistence_paths_that_escape_the_workspace() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
openhands:
  conversation:
    persistence_dir_relative: ../shared-state
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("parent-directory traversal should be rejected");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "openhands.conversation.persistence_dir_relative",
                ..
            }
        ));
    }

    #[test]
    fn resolves_relative_workspace_paths_against_workflow_directory() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
workspace:
  root: ./nested/workspaces
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let resolved = workflow
            .resolve(Path::new("/repo/config"), &env)
            .expect("workflow should resolve");

        assert_eq!(
            resolved.config.workspace.root,
            PathBuf::from("/repo/config/nested/workspaces")
        );
    }

    #[test]
    fn resolves_bare_workspace_roots_against_workflow_directory() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
workspace:
  root: workspaces
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let resolved = workflow
            .resolve(Path::new("/repo/config"), &env)
            .expect("workflow should resolve");

        assert_eq!(
            resolved.config.workspace.root,
            PathBuf::from("/repo/config/workspaces")
        );
    }

    #[test]
    fn renders_prompt_for_first_run_and_continuation() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
---
Ticket {{ issue.identifier }}
{% if attempt %}
Attempt {{ attempt }}
{% endif %}
"#,
        )
        .expect("workflow should parse");
        let issue = TestIssue {
            identifier: "COE-259",
            title: "Workflow loader",
            state: "In Progress",
            description: Some("Implement the workflow crate"),
            labels: vec!["rust", "workflow"],
        };

        let first = workflow
            .render_prompt(&issue, None)
            .expect("first run render should succeed");
        let continuation = workflow
            .render_prompt(&issue, Some(2))
            .expect("continuation render should succeed");

        assert!(first.contains("Ticket COE-259"));
        assert!(!first.contains("Attempt"));
        assert!(continuation.contains("Attempt 2"));
    }

    #[test]
    fn rejects_unknown_template_variables() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
---
{{ issue.missing_field }}
"#,
        )
        .expect("workflow should parse");
        let issue = TestIssue {
            identifier: "COE-259",
            title: "Workflow loader",
            state: "In Progress",
            description: None,
            labels: vec![],
        };

        let error = workflow
            .render_prompt(&issue, None)
            .expect_err("missing template variables should fail");

        assert!(matches!(error, PromptTemplateError::Render { .. }));
    }

    #[test]
    fn rejects_unknown_template_filters() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
---
{{ issue.title | missing_filter }}
"#,
        )
        .expect("workflow should parse");
        let issue = TestIssue {
            identifier: "COE-259",
            title: "Workflow loader",
            state: "In Progress",
            description: None,
            labels: vec![],
        };

        let error = workflow
            .render_prompt(&issue, None)
            .expect_err("unknown filters should fail");

        assert!(matches!(error, PromptTemplateError::Parse { .. }));
    }

    #[test]
    fn uses_default_prompt_when_body_is_empty() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
---
"#,
        )
        .expect("workflow should parse");
        let issue = TestIssue {
            identifier: "COE-259",
            title: "Workflow loader",
            state: "In Progress",
            description: None,
            labels: vec![],
        };

        let rendered = workflow
            .render_prompt(&issue, None)
            .expect("default prompt render should succeed");

        assert_eq!(rendered, DEFAULT_PROMPT_TEMPLATE);
    }

    #[test]
    fn uses_default_prompt_when_body_is_whitespace_only() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
---

"#,
        )
        .expect("workflow should parse");
        let issue = TestIssue {
            identifier: "COE-259",
            title: "Workflow loader",
            state: "In Progress",
            description: None,
            labels: vec![],
        };

        let rendered = workflow
            .render_prompt(&issue, None)
            .expect("whitespace-only prompt should use the default template");

        assert_eq!(rendered, DEFAULT_PROMPT_TEMPLATE);
    }

    #[test]
    fn preserves_whitespace_sensitive_prompt_body() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
---

    code block
"#,
        )
        .expect("workflow should parse");

        assert_eq!(workflow.prompt_template, "\n    code block\n");
    }

    #[test]
    fn errors_on_missing_required_tracker_config() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  active_states:
    - Todo
  terminal_states:
    - Done
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("missing project slug should fail");

        assert!(matches!(
            error,
            WorkflowConfigError::MissingRequiredField {
                field: "tracker.project_slug"
            }
        ));
    }

    #[test]
    fn missing_tracker_terminal_states_fail() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("missing terminal states should fail");

        assert!(matches!(
            error,
            WorkflowConfigError::MissingRequiredField {
                field: "tracker.terminal_states"
            }
        ));
    }

    #[test]
    fn rejects_invalid_per_state_concurrency_limits() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
agent:
  max_concurrent_agents_by_state:
    In Review: two
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("malformed state limits should fail");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidInteger {
                field: "agent.max_concurrent_agents_by_state",
                ..
            }
        ));
    }

    #[test]
    fn rejects_non_positive_per_state_concurrency_limits() {
        let workflow = WorkflowDefinition::parse(
            r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
  terminal_states:
    - Done
agent:
  max_concurrent_agents_by_state:
    In Review: 0
---
{{ issue.identifier }}
"#,
        )
        .expect("workflow should parse");
        let env = env([("LINEAR_API_KEY", "linear-token")]);

        let error = workflow
            .resolve(Path::new("/repo"), &env)
            .expect_err("non-positive state limits should fail");

        assert!(matches!(
            error,
            WorkflowConfigError::InvalidField {
                field: "agent.max_concurrent_agents_by_state",
                ..
            }
        ));
    }

    fn env<const N: usize>(pairs: [(&str, &str); N]) -> BTreeMap<String, String> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    fn sample_workflow() -> &'static str {
        r#"---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
    - Closed
polling:
  interval_ms: 5000
workspace:
  root: ~/workspaces
hooks:
  timeout_ms: 60000
agent:
  max_concurrent_agents: 4
  max_turns: 8
  max_retry_backoff_ms: 120000
  stall_timeout_ms: 90000
openhands:
  transport:
    base_url: http://127.0.0.1:8000
  conversation:
    persistence_dir_relative: .opensymphony/openhands
    agent:
      llm:
        model: ${OPENHANDS_MODEL}
---

# Assignment

Ticket: {{ issue.identifier }}
"#
    }

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crate dir should have workspace parent")
            .parent()
            .expect("workspace root should exist")
            .to_path_buf()
    }
}
