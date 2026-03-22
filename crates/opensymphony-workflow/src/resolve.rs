use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use url::{Host, Url};

use crate::{
    error::WorkflowConfigError,
    model::{
        AgentConfig, AgentFrontMatter, Environment, HooksConfig, HooksFrontMatter, IntegerLike,
        OpenHandsConfig, OpenHandsConfirmationPolicy, OpenHandsConfirmationPolicyFrontMatter,
        OpenHandsConversationAgentConfig, OpenHandsConversationAgentFrontMatter,
        OpenHandsConversationConfig, OpenHandsConversationFrontMatter, OpenHandsFrontMatter,
        OpenHandsLlmConfig, OpenHandsLlmFrontMatter, OpenHandsLocalServerConfig,
        OpenHandsLocalServerFrontMatter, OpenHandsMcpConfig, OpenHandsMcpFrontMatter,
        OpenHandsTransportConfig, OpenHandsTransportFrontMatter, OpenHandsWebSocketConfig,
        OpenHandsWebSocketFrontMatter, PollingConfig, PollingFrontMatter, ResolvedWorkflow,
        TrackerConfig, TrackerFrontMatter, TrackerKind, WorkflowConfig, WorkflowDefinition,
        WorkflowExtensions, WorkspaceConfig, WorkspaceFrontMatter, DEFAULT_HOOK_TIMEOUT_MS,
        DEFAULT_LINEAR_ENDPOINT, DEFAULT_MAX_CONCURRENT_AGENTS, DEFAULT_MAX_RETRY_BACKOFF_MS,
        DEFAULT_MAX_TURNS, DEFAULT_OPENHANDS_AGENT_KIND, DEFAULT_OPENHANDS_AUTH_MODE,
        DEFAULT_OPENHANDS_BASE_URL, DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND,
        DEFAULT_OPENHANDS_MAX_ITERATIONS, DEFAULT_OPENHANDS_PERSISTENCE_DIR,
        DEFAULT_OPENHANDS_QUERY_PARAM_NAME, DEFAULT_OPENHANDS_READINESS_PROBE_PATH,
        DEFAULT_OPENHANDS_READY_TIMEOUT_MS, DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS,
        DEFAULT_OPENHANDS_RECONNECT_MAX_MS, DEFAULT_OPENHANDS_STARTUP_TIMEOUT_MS,
        DEFAULT_POLL_INTERVAL_MS, DEFAULT_STALL_TIMEOUT_MS, DEFAULT_WORKSPACE_ROOT,
    },
};

pub(crate) fn resolve_workflow<E: Environment>(
    workflow: &WorkflowDefinition,
    base_dir: &Path,
    env: &E,
) -> Result<ResolvedWorkflow, WorkflowConfigError> {
    Ok(ResolvedWorkflow {
        config: WorkflowConfig {
            tracker: resolve_tracker(&workflow.front_matter.tracker, env)?,
            polling: resolve_polling(&workflow.front_matter.polling)?,
            workspace: resolve_workspace(&workflow.front_matter.workspace, base_dir, env)?,
            hooks: resolve_hooks(&workflow.front_matter.hooks)?,
            agent: resolve_agent(&workflow.front_matter.agent)?,
        },
        extensions: WorkflowExtensions {
            openhands: resolve_openhands(&workflow.front_matter.openhands, base_dir, env)?,
        },
        prompt_template: workflow.prompt_template.clone(),
    })
}

fn resolve_tracker<E: Environment>(
    tracker: &TrackerFrontMatter,
    env: &E,
) -> Result<TrackerConfig, WorkflowConfigError> {
    let kind = match normalize_optional_literal(&tracker.kind) {
        Some(kind) if kind.eq_ignore_ascii_case("linear") => TrackerKind::Linear,
        Some(kind) => return Err(WorkflowConfigError::UnsupportedTrackerKind { kind }),
        None => {
            return Err(WorkflowConfigError::MissingRequiredField {
                field: "tracker.kind",
            });
        }
    };

    let endpoint = resolve_string_or_default(
        tracker.endpoint.as_deref(),
        env,
        "tracker.endpoint",
        DEFAULT_LINEAR_ENDPOINT,
    )?;
    let project_slug = require_literal(tracker.project_slug.as_deref(), "tracker.project_slug")?;
    let api_key = resolve_tracker_api_key(tracker, env)?;

    Ok(TrackerConfig {
        kind,
        endpoint,
        api_key,
        project_slug,
        active_states: resolve_state_list(
            tracker.active_states.as_deref(),
            "tracker.active_states",
        )?,
        terminal_states: resolve_state_list(
            tracker.terminal_states.as_deref(),
            "tracker.terminal_states",
        )?,
    })
}

fn resolve_tracker_api_key<E: Environment>(
    tracker: &TrackerFrontMatter,
    env: &E,
) -> Result<String, WorkflowConfigError> {
    if let Some(configured) = tracker.api_key.as_deref() {
        let configured = require_literal(Some(configured), "tracker.api_key")?;
        return resolve_string(&configured, env, "tracker.api_key");
    }

    env.get("LINEAR_API_KEY")
        .and_then(normalize_optional_owned)
        .ok_or(WorkflowConfigError::MissingRequiredField {
            field: "tracker.api_key",
        })
}

fn resolve_polling(polling: &PollingFrontMatter) -> Result<PollingConfig, WorkflowConfigError> {
    Ok(PollingConfig {
        interval_ms: resolve_positive_u64(
            polling.interval_ms.as_ref(),
            "polling.interval_ms",
            DEFAULT_POLL_INTERVAL_MS,
        )?,
    })
}

fn resolve_workspace<E: Environment>(
    workspace: &WorkspaceFrontMatter,
    base_dir: &Path,
    env: &E,
) -> Result<WorkspaceConfig, WorkflowConfigError> {
    let root_value = workspace.root.as_deref().unwrap_or(DEFAULT_WORKSPACE_ROOT);
    Ok(WorkspaceConfig {
        root: resolve_workspace_root(root_value, base_dir, env)?,
    })
}

fn resolve_hooks(hooks: &HooksFrontMatter) -> Result<HooksConfig, WorkflowConfigError> {
    Ok(HooksConfig {
        after_create: hooks.after_create.clone(),
        before_run: hooks.before_run.clone(),
        after_run: hooks.after_run.clone(),
        before_remove: hooks.before_remove.clone(),
        timeout_ms: resolve_non_positive_to_default(
            hooks.timeout_ms.as_ref(),
            "hooks.timeout_ms",
            DEFAULT_HOOK_TIMEOUT_MS,
        )?,
    })
}

fn resolve_agent(agent: &AgentFrontMatter) -> Result<AgentConfig, WorkflowConfigError> {
    Ok(AgentConfig {
        max_concurrent_agents: resolve_positive_u64(
            agent.max_concurrent_agents.as_ref(),
            "agent.max_concurrent_agents",
            DEFAULT_MAX_CONCURRENT_AGENTS,
        )?,
        max_turns: resolve_positive_u64(
            agent.max_turns.as_ref(),
            "agent.max_turns",
            DEFAULT_MAX_TURNS,
        )?,
        max_retry_backoff_ms: resolve_positive_u64(
            agent.max_retry_backoff_ms.as_ref(),
            "agent.max_retry_backoff_ms",
            DEFAULT_MAX_RETRY_BACKOFF_MS,
        )?,
        stall_timeout_ms: resolve_stall_timeout(agent.stall_timeout_ms.as_ref())?,
        max_concurrent_agents_by_state: resolve_state_limits(
            agent.max_concurrent_agents_by_state.as_ref(),
        )?,
    })
}

fn resolve_stall_timeout(
    stall_timeout_ms: Option<&IntegerLike>,
) -> Result<Option<u64>, WorkflowConfigError> {
    let Some(value) = stall_timeout_ms else {
        return Ok(Some(DEFAULT_STALL_TIMEOUT_MS));
    };

    let parsed = parse_i64(value, "agent.stall_timeout_ms")?;
    if parsed <= 0 {
        Ok(None)
    } else {
        Ok(Some(parsed as u64))
    }
}

fn resolve_state_list(
    raw: Option<&[String]>,
    field: &'static str,
) -> Result<Vec<String>, WorkflowConfigError> {
    let raw = raw.ok_or(WorkflowConfigError::MissingRequiredField { field })?;
    if raw.is_empty() {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must contain at least one state".to_owned(),
        });
    }

    raw.iter()
        .map(|state| {
            normalize_optional(state).ok_or_else(|| WorkflowConfigError::InvalidField {
                field,
                message: "state names must not be empty".to_owned(),
            })
        })
        .collect()
}

fn resolve_state_limits(
    raw: Option<&BTreeMap<String, IntegerLike>>,
) -> Result<BTreeMap<String, u64>, WorkflowConfigError> {
    let mut resolved = BTreeMap::new();
    let Some(raw) = raw else {
        return Ok(resolved);
    };

    for (state, value) in raw {
        let state = normalize_optional(state).ok_or_else(|| WorkflowConfigError::InvalidField {
            field: "agent.max_concurrent_agents_by_state",
            message: "state names must not be empty".to_owned(),
        })?;
        let parsed = parse_i64(value, "agent.max_concurrent_agents_by_state")?;
        if parsed <= 0 {
            return Err(WorkflowConfigError::InvalidField {
                field: "agent.max_concurrent_agents_by_state",
                message: "state limits must be greater than zero".to_owned(),
            });
        }
        resolved.insert(state.to_lowercase(), parsed as u64);
    }

    Ok(resolved)
}

fn resolve_openhands<E: Environment>(
    openhands: &OpenHandsFrontMatter,
    _base_dir: &Path,
    env: &E,
) -> Result<OpenHandsConfig, WorkflowConfigError> {
    reject_unsupported_openhands_transport_auth(&openhands.transport)?;
    reject_unsupported_openhands_local_server_overrides(&openhands.local_server)?;
    reject_unsupported_openhands_websocket_overrides(&openhands.websocket)?;
    reject_unsupported_openhands_mcp(&openhands.mcp)?;

    Ok(OpenHandsConfig {
        transport: OpenHandsTransportConfig {
            base_url: resolve_openhands_base_url(openhands.transport.base_url.as_deref(), env)?,
            session_api_key_env: normalize_optional_literal(
                &openhands.transport.session_api_key_env,
            ),
        },
        local_server: OpenHandsLocalServerConfig {
            enabled: openhands.local_server.enabled.unwrap_or(true),
            command: openhands
                .local_server
                .command
                .as_deref()
                .map(|configured| {
                    resolve_command(
                        Some(configured),
                        "openhands.local_server.command",
                        Vec::new(),
                    )
                })
                .transpose()?,
            startup_timeout_ms: resolve_positive_u64(
                openhands.local_server.startup_timeout_ms.as_ref(),
                "openhands.local_server.startup_timeout_ms",
                DEFAULT_OPENHANDS_STARTUP_TIMEOUT_MS,
            )?,
            readiness_probe_path: resolve_string_or_default(
                openhands.local_server.readiness_probe_path.as_deref(),
                env,
                "openhands.local_server.readiness_probe_path",
                DEFAULT_OPENHANDS_READINESS_PROBE_PATH,
            )?,
            env: resolve_string_map(
                &openhands.local_server.env,
                env,
                "openhands.local_server.env",
            )?,
        },
        conversation: resolve_openhands_conversation(&openhands.conversation, env)?,
        websocket: OpenHandsWebSocketConfig {
            enabled: openhands.websocket.enabled.unwrap_or(true),
            ready_timeout_ms: resolve_positive_u64(
                openhands.websocket.ready_timeout_ms.as_ref(),
                "openhands.websocket.ready_timeout_ms",
                DEFAULT_OPENHANDS_READY_TIMEOUT_MS,
            )?,
            reconnect_initial_ms: resolve_positive_u64(
                openhands.websocket.reconnect_initial_ms.as_ref(),
                "openhands.websocket.reconnect_initial_ms",
                DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS,
            )?,
            reconnect_max_ms: resolve_positive_u64(
                openhands.websocket.reconnect_max_ms.as_ref(),
                "openhands.websocket.reconnect_max_ms",
                DEFAULT_OPENHANDS_RECONNECT_MAX_MS,
            )?,
            auth_mode: resolve_string_or_default(
                openhands.websocket.auth_mode.as_deref(),
                env,
                "openhands.websocket.auth_mode",
                DEFAULT_OPENHANDS_AUTH_MODE,
            )?,
            query_param_name: resolve_string_or_default(
                openhands.websocket.query_param_name.as_deref(),
                env,
                "openhands.websocket.query_param_name",
                DEFAULT_OPENHANDS_QUERY_PARAM_NAME,
            )?,
        },
        mcp: OpenHandsMcpConfig {
            stdio_servers: Vec::new(),
        },
    })
}

fn reject_unsupported_openhands_transport_auth(
    transport: &OpenHandsTransportFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if transport.session_api_key_env.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.transport.session_api_key_env",
            message:
                "is not supported until the runtime transport layer wires workflow auth into AuthConfig"
                    .to_owned(),
        });
    }

    Ok(())
}

fn reject_unsupported_openhands_local_server_overrides(
    local_server: &OpenHandsLocalServerFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if matches!(local_server.enabled, Some(false)) {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.local_server.enabled",
            message:
                "is not supported until the runtime supervisor can honor workflow-owned local-server disablement"
                    .to_owned(),
        });
    }

    if local_server.startup_timeout_ms.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.local_server.startup_timeout_ms",
            message:
                "is not supported until the runtime supervisor creation path consumes workflow-owned startup timeouts"
                    .to_owned(),
        });
    }

    if local_server.readiness_probe_path.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.local_server.readiness_probe_path",
            message:
                "is not supported until the runtime supervisor launch path consumes workflow-owned readiness probe settings"
                    .to_owned(),
        });
    }

    if local_server.command.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.local_server.command",
            message:
                "is not supported until the runtime supervisor can honor workflow-owned launcher overrides"
                    .to_owned(),
        });
    }

    Ok(())
}

fn reject_unsupported_openhands_websocket_overrides(
    websocket: &OpenHandsWebSocketFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if websocket.enabled.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.websocket.enabled",
            message:
                "is not supported until the runtime readiness path can honor workflow-owned websocket enablement"
                    .to_owned(),
        });
    }

    if websocket.ready_timeout_ms.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.websocket.ready_timeout_ms",
            message:
                "is not supported until the runtime readiness path consumes workflow-owned websocket timeouts"
                    .to_owned(),
        });
    }

    if websocket.reconnect_initial_ms.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.websocket.reconnect_initial_ms",
            message:
                "is not supported until the runtime reconnect path consumes workflow-owned websocket backoff settings"
                    .to_owned(),
        });
    }

    if websocket.reconnect_max_ms.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.websocket.reconnect_max_ms",
            message:
                "is not supported until the runtime reconnect path consumes workflow-owned websocket backoff settings"
                    .to_owned(),
        });
    }

    if websocket.auth_mode.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.websocket.auth_mode",
            message:
                "is not supported until the runtime transport layer wires workflow auth into AuthConfig"
                    .to_owned(),
        });
    }

    if websocket.query_param_name.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.websocket.query_param_name",
            message:
                "is not supported until the runtime transport layer wires workflow auth into AuthConfig"
                    .to_owned(),
        });
    }

    Ok(())
}

fn reject_unsupported_openhands_mcp(
    mcp: &OpenHandsMcpFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if mcp.stdio_servers.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.mcp.stdio_servers",
            message:
                "is not supported until the runtime conversation-create adapter can forward MCP config"
                    .to_owned(),
        });
    }

    Ok(())
}

fn resolve_openhands_base_url<E: Environment>(
    configured: Option<&str>,
    env: &E,
) -> Result<String, WorkflowConfigError> {
    let base_url = resolve_string_or_default(
        configured,
        env,
        "openhands.transport.base_url",
        DEFAULT_OPENHANDS_BASE_URL,
    )?;
    validate_openhands_base_url(&base_url)?;
    Ok(base_url)
}

fn validate_openhands_base_url(base_url: &str) -> Result<(), WorkflowConfigError> {
    let parsed = Url::parse(base_url).map_err(|error| WorkflowConfigError::InvalidField {
        field: "openhands.transport.base_url",
        message: format!("must be an absolute http URL: {error}"),
    })?;

    match parsed.scheme() {
        "http" => {}
        _ => {
            return Err(WorkflowConfigError::InvalidField {
                field: "openhands.transport.base_url",
                message:
                    "must use the http scheme until supervisor readiness probes support TLS endpoints"
                        .to_owned(),
            });
        }
    }

    match parsed.host() {
        Some(Host::Ipv6(_)) => {
            return Err(WorkflowConfigError::InvalidField {
                field: "openhands.transport.base_url",
                message:
                    "must not use bracketed IPv6 hosts until supervisor readiness probes support them"
                        .to_owned(),
            });
        }
        Some(_) => {}
        None => {
            return Err(WorkflowConfigError::InvalidField {
                field: "openhands.transport.base_url",
                message: "must include a host".to_owned(),
            });
        }
    }

    let without_scheme =
        base_url
            .strip_prefix("http://")
            .ok_or_else(|| {
                WorkflowConfigError::InvalidField {
            field: "openhands.transport.base_url",
            message:
                "must use the http scheme until supervisor readiness probes support TLS endpoints"
                    .to_owned(),
        }
            })?;

    if without_scheme.contains('/') {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.transport.base_url",
            message:
                "must not include a path until supervisor readiness probes support prefixed base URLs"
                    .to_owned(),
        });
    }

    Ok(())
}

fn resolve_openhands_conversation<E: Environment>(
    conversation: &OpenHandsConversationFrontMatter,
    env: &E,
) -> Result<OpenHandsConversationConfig, WorkflowConfigError> {
    let reuse_policy = resolve_openhands_reuse_policy(conversation.reuse_policy.as_deref(), env)?;
    let confirmation_policy = match conversation.confirmation_policy.clone() {
        Some(policy) => resolve_openhands_confirmation_policy(policy)?,
        None => OpenHandsConfirmationPolicy {
            kind: DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND.to_owned(),
        },
    };

    let agent = match conversation.agent.as_ref() {
        Some(agent) => resolve_openhands_agent(agent, env)?,
        None => OpenHandsConversationAgentConfig {
            kind: DEFAULT_OPENHANDS_AGENT_KIND.to_owned(),
            llm: None,
            log_completions: false,
            options: BTreeMap::new(),
        },
    };

    if agent.kind.trim().is_empty() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.agent.kind",
            message: "must not be empty".to_owned(),
        });
    }

    Ok(OpenHandsConversationConfig {
        reuse_policy,
        persistence_dir_relative: resolve_relative_path(
            conversation.persistence_dir_relative.as_deref(),
            env,
            "openhands.conversation.persistence_dir_relative",
            DEFAULT_OPENHANDS_PERSISTENCE_DIR,
        )?,
        max_iterations: resolve_openhands_max_iterations(conversation.max_iterations.as_ref())?,
        stuck_detection: conversation.stuck_detection.unwrap_or(true),
        confirmation_policy,
        agent,
    })
}

fn resolve_openhands_confirmation_policy(
    policy: OpenHandsConfirmationPolicyFrontMatter,
) -> Result<OpenHandsConfirmationPolicy, WorkflowConfigError> {
    if !policy.options.is_empty() {
        let unsupported = policy
            .options
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.confirmation_policy",
            message: format!(
                "unsupported options cannot be forwarded to the current OpenHands request subset: {unsupported}"
            ),
        });
    }

    let kind = match policy.kind.as_deref() {
        Some(kind) => {
            normalize_optional(kind).ok_or_else(|| WorkflowConfigError::InvalidField {
                field: "openhands.conversation.confirmation_policy.kind",
                message: "must not be empty".to_owned(),
            })?
        }
        None => DEFAULT_OPENHANDS_CONFIRMATION_POLICY_KIND.to_owned(),
    };

    Ok(OpenHandsConfirmationPolicy { kind })
}

fn resolve_openhands_agent<E: Environment>(
    agent: &OpenHandsConversationAgentFrontMatter,
    env: &E,
) -> Result<OpenHandsConversationAgentConfig, WorkflowConfigError> {
    reject_unsupported_openhands_agent_options(agent)?;

    let kind = match agent.kind.as_deref() {
        Some(kind) => {
            normalize_optional(kind).ok_or_else(|| WorkflowConfigError::InvalidField {
                field: "openhands.conversation.agent.kind",
                message: "must not be empty".to_owned(),
            })?
        }
        None => DEFAULT_OPENHANDS_AGENT_KIND.to_owned(),
    };

    Ok(OpenHandsConversationAgentConfig {
        kind,
        llm: agent
            .llm
            .as_ref()
            .map(|llm| resolve_openhands_llm(llm, env))
            .transpose()?,
        log_completions: false,
        options: BTreeMap::new(),
    })
}

fn resolve_openhands_reuse_policy<E: Environment>(
    configured: Option<&str>,
    env: &E,
) -> Result<String, WorkflowConfigError> {
    let reuse_policy = resolve_string_or_default(
        configured,
        env,
        "openhands.conversation.reuse_policy",
        "per_issue",
    )?;
    if !reuse_policy.eq_ignore_ascii_case("per_issue") {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.reuse_policy",
            message:
                "is not supported until the orchestrator/runtime path can honor non-default conversation reuse policies"
                    .to_owned(),
        });
    }

    Ok("per_issue".to_owned())
}

fn reject_unsupported_openhands_agent_options(
    agent: &OpenHandsConversationAgentFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if agent.log_completions.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.agent.log_completions",
            message:
                "is not supported until the runtime conversation-create adapter can forward agent logging options"
                    .to_owned(),
        });
    }

    if !agent.options.is_empty() {
        let unsupported = agent.options.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.agent",
            message: format!(
                "unsupported options cannot be forwarded to the current OpenHands agent request subset: {unsupported}"
            ),
        });
    }

    Ok(())
}

fn resolve_openhands_llm<E: Environment>(
    llm: &OpenHandsLlmFrontMatter,
    env: &E,
) -> Result<OpenHandsLlmConfig, WorkflowConfigError> {
    reject_unsupported_openhands_llm_provider_overrides(llm)?;
    reject_unsupported_openhands_llm_options(llm)?;

    let field = "openhands.conversation.agent.llm.model";
    let model = llm
        .model
        .as_deref()
        .ok_or(WorkflowConfigError::MissingRequiredField { field })?;
    let model = resolve_string(model, env, field)?;
    if model.trim().is_empty() {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must not be empty".to_owned(),
        });
    }

    Ok(OpenHandsLlmConfig {
        model: Some(model),
        api_key_env: normalize_optional_literal(&llm.api_key_env),
        base_url_env: normalize_optional_literal(&llm.base_url_env),
        options: llm.options.clone(),
    })
}

fn reject_unsupported_openhands_llm_options(
    llm: &OpenHandsLlmFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if !llm.options.is_empty() {
        let unsupported = llm.options.keys().cloned().collect::<Vec<_>>().join(", ");
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.agent.llm",
            message: format!(
                "unsupported options cannot be forwarded to the current OpenHands llm request subset: {unsupported}"
            ),
        });
    }

    Ok(())
}

fn reject_unsupported_openhands_llm_provider_overrides(
    llm: &OpenHandsLlmFrontMatter,
) -> Result<(), WorkflowConfigError> {
    if llm.api_key_env.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.agent.llm.api_key_env",
            message:
                "is not supported until the runtime conversation-create adapter resolves provider credentials"
                    .to_owned(),
        });
    }

    if llm.base_url_env.is_some() {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.agent.llm.base_url_env",
            message:
                "is not supported until the runtime conversation-create adapter can forward provider base URLs"
                    .to_owned(),
        });
    }

    Ok(())
}

fn resolve_openhands_max_iterations(
    value: Option<&IntegerLike>,
) -> Result<u64, WorkflowConfigError> {
    let max_iterations = resolve_positive_u64(
        value,
        "openhands.conversation.max_iterations",
        DEFAULT_OPENHANDS_MAX_ITERATIONS,
    )?;
    if max_iterations > u32::MAX as u64 {
        return Err(WorkflowConfigError::InvalidField {
            field: "openhands.conversation.max_iterations",
            message: format!("must be less than or equal to {}", u32::MAX),
        });
    }

    Ok(max_iterations)
}

fn resolve_command(
    configured: Option<&[String]>,
    field: &'static str,
    default: Vec<String>,
) -> Result<Vec<String>, WorkflowConfigError> {
    let command = configured
        .map(|configured| configured.to_vec())
        .unwrap_or(default);

    if command.is_empty() {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must contain at least one argument".to_owned(),
        });
    }

    if command.iter().any(|part| part.trim().is_empty()) {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must not contain empty arguments".to_owned(),
        });
    }

    Ok(command)
}

fn resolve_string_map<E: Environment>(
    raw: &BTreeMap<String, String>,
    env: &E,
    field: &'static str,
) -> Result<BTreeMap<String, String>, WorkflowConfigError> {
    raw.iter()
        .map(|(key, value)| Ok((key.clone(), resolve_string(value, env, field)?)))
        .collect()
}

fn resolve_workspace_root<E: Environment>(
    value: &str,
    base_dir: &Path,
    env: &E,
) -> Result<PathBuf, WorkflowConfigError> {
    let resolved = resolve_string(value, env, "workspace.root")?;
    if resolved.trim().is_empty() {
        return Err(WorkflowConfigError::InvalidField {
            field: "workspace.root",
            message: "must not be empty".to_owned(),
        });
    }

    let expanded = expand_home_directory(&resolved, env)?;
    if expanded.is_absolute() {
        return Ok(normalize_path(&expanded));
    }

    let base_dir = normalize_workflow_base_dir(base_dir)?;
    Ok(normalize_path(&base_dir.join(expanded)))
}

fn normalize_workflow_base_dir(base_dir: &Path) -> Result<PathBuf, WorkflowConfigError> {
    if base_dir.is_absolute() {
        return Ok(normalize_path(base_dir));
    }

    let cwd = std::env::current_dir().map_err(|error| WorkflowConfigError::InvalidField {
        field: "workspace.root",
        message: format!(
            "cannot resolve a relative workflow directory without the current working directory: {error}"
        ),
    })?;

    Ok(normalize_path(&cwd.join(base_dir)))
}

fn resolve_relative_path<E: Environment>(
    configured: Option<&str>,
    env: &E,
    field: &'static str,
    default: &str,
) -> Result<PathBuf, WorkflowConfigError> {
    let value = configured.unwrap_or(default);
    let resolved = resolve_string(value, env, field)?;
    let path = PathBuf::from(&resolved);
    if resolved.trim().is_empty() {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must not be empty".to_owned(),
        });
    }
    if path.is_absolute() || resolved.starts_with('~') {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must stay relative to the issue workspace".to_owned(),
        });
    }

    let normalized = normalize_path(&path);
    if !stays_within_relative_root(&path) {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must not escape the issue workspace".to_owned(),
        });
    }

    Ok(normalized)
}

fn resolve_string_or_default<E: Environment>(
    configured: Option<&str>,
    env: &E,
    field: &'static str,
    default: &str,
) -> Result<String, WorkflowConfigError> {
    match configured.and_then(normalize_optional) {
        Some(value) => resolve_string(&value, env, field),
        None => Ok(default.to_owned()),
    }
}

fn resolve_string<E: Environment>(
    value: &str,
    env: &E,
    field: &'static str,
) -> Result<String, WorkflowConfigError> {
    if let Some(variable) = parse_env_token(value) {
        let resolved = env
            .get(variable)
            .and_then(normalize_optional_owned)
            .ok_or_else(|| WorkflowConfigError::MissingEnvironmentVariable {
                field,
                variable: variable.to_owned(),
            })?;
        return Ok(resolved);
    }

    Ok(value.to_owned())
}

fn require_literal(
    value: Option<&str>,
    field: &'static str,
) -> Result<String, WorkflowConfigError> {
    value
        .and_then(normalize_optional)
        .ok_or(WorkflowConfigError::MissingRequiredField { field })
}

fn resolve_positive_u64(
    value: Option<&IntegerLike>,
    field: &'static str,
    default: u64,
) -> Result<u64, WorkflowConfigError> {
    let Some(value) = value else {
        return Ok(default);
    };

    let parsed = parse_i64(value, field)?;
    if parsed <= 0 {
        return Err(WorkflowConfigError::InvalidField {
            field,
            message: "must be greater than zero".to_owned(),
        });
    }

    Ok(parsed as u64)
}

fn resolve_non_positive_to_default(
    value: Option<&IntegerLike>,
    field: &'static str,
    default: u64,
) -> Result<u64, WorkflowConfigError> {
    let Some(value) = value else {
        return Ok(default);
    };

    let parsed = parse_i64(value, field)?;
    if parsed <= 0 {
        Ok(default)
    } else {
        Ok(parsed as u64)
    }
}

fn parse_i64(value: &IntegerLike, field: &'static str) -> Result<i64, WorkflowConfigError> {
    match value {
        IntegerLike::Integer(value) => Ok(*value),
        IntegerLike::String(value) => {
            value
                .trim()
                .parse::<i64>()
                .map_err(|_| WorkflowConfigError::InvalidInteger {
                    field,
                    value: value.clone(),
                })
        }
    }
}

fn expand_home_directory<E: Environment>(
    value: &str,
    env: &E,
) -> Result<PathBuf, WorkflowConfigError> {
    if value == "~" {
        return home_directory(env);
    }

    if let Some(rest) = value.strip_prefix("~/") {
        return Ok(home_directory(env)?.join(rest));
    }

    Ok(PathBuf::from(value))
}

fn home_directory<E: Environment>(env: &E) -> Result<PathBuf, WorkflowConfigError> {
    env.get("HOME")
        .or_else(|| env.get("USERPROFILE"))
        .and_then(normalize_optional_owned)
        .map(PathBuf::from)
        .ok_or_else(|| WorkflowConfigError::MissingEnvironmentVariable {
            field: "workspace.root",
            variable: "HOME".to_owned(),
        })
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    let mut saw_root = false;

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => {
                saw_root = true;
                normalized.push(Path::new("/"));
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !saw_root {
                    normalized.push("..");
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        if saw_root {
            PathBuf::from("/")
        } else {
            PathBuf::from(".")
        }
    } else {
        normalized
    }
}

fn stays_within_relative_root(path: &Path) -> bool {
    let mut depth: usize = 0;

    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => return false,
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Component::Normal(_) => depth += 1,
        }
    }

    true
}

fn parse_env_token(value: &str) -> Option<&str> {
    if let Some(variable) = value
        .strip_prefix("${")
        .and_then(|value| value.strip_suffix('}'))
    {
        return is_env_name(variable).then_some(variable);
    }

    let variable = value.strip_prefix('$')?;
    is_env_name(variable).then_some(variable)
}

fn is_env_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn normalize_optional(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn normalize_optional_owned(value: String) -> Option<String> {
    normalize_optional(&value)
}

fn normalize_optional_literal(value: &Option<String>) -> Option<String> {
    value.as_deref().and_then(normalize_optional)
}
