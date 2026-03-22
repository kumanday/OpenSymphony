use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use crate::{
    error::WorkflowConfigError,
    model::{
        default_active_states, default_openhands_local_server_command, default_terminal_states,
        AgentConfig, AgentFrontMatter, Environment, HooksConfig, HooksFrontMatter, IntegerLike,
        OpenHandsConfig, OpenHandsConversationAgentConfig, OpenHandsConversationAgentFrontMatter,
        OpenHandsConversationConfig, OpenHandsConversationFrontMatter, OpenHandsFrontMatter,
        OpenHandsLlmConfig, OpenHandsLlmFrontMatter, OpenHandsLocalServerConfig,
        OpenHandsMcpConfig, OpenHandsStdioServerConfig, OpenHandsStdioServerFrontMatter,
        OpenHandsTransportConfig, OpenHandsWebSocketConfig, PollingConfig, PollingFrontMatter,
        ResolvedWorkflow, TrackerConfig, TrackerFrontMatter, TrackerKind, WorkflowConfig,
        WorkflowDefinition, WorkflowExtensions, WorkspaceConfig, WorkspaceFrontMatter,
        DEFAULT_HOOK_TIMEOUT_MS, DEFAULT_LINEAR_ENDPOINT, DEFAULT_MAX_CONCURRENT_AGENTS,
        DEFAULT_MAX_RETRY_BACKOFF_MS, DEFAULT_MAX_TURNS, DEFAULT_OPENHANDS_AUTH_MODE,
        DEFAULT_OPENHANDS_BASE_URL, DEFAULT_OPENHANDS_MAX_ITERATIONS,
        DEFAULT_OPENHANDS_PERSISTENCE_DIR, DEFAULT_OPENHANDS_QUERY_PARAM_NAME,
        DEFAULT_OPENHANDS_READINESS_PROBE_PATH, DEFAULT_OPENHANDS_READY_TIMEOUT_MS,
        DEFAULT_OPENHANDS_RECONNECT_INITIAL_MS, DEFAULT_OPENHANDS_RECONNECT_MAX_MS,
        DEFAULT_OPENHANDS_STARTUP_TIMEOUT_MS, DEFAULT_POLL_INTERVAL_MS, DEFAULT_STALL_TIMEOUT_MS,
        DEFAULT_WORKSPACE_ROOT,
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
            openhands: resolve_openhands(&workflow.front_matter.openhands, env)?,
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
        active_states: tracker
            .active_states
            .clone()
            .unwrap_or_else(default_active_states),
        terminal_states: tracker
            .terminal_states
            .clone()
            .unwrap_or_else(default_terminal_states),
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
        ),
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

fn resolve_state_limits(raw: Option<&BTreeMap<String, IntegerLike>>) -> BTreeMap<String, u64> {
    let mut resolved = BTreeMap::new();
    let Some(raw) = raw else {
        return resolved;
    };

    for (state, value) in raw {
        if let Ok(parsed) = parse_i64(value, "agent.max_concurrent_agents_by_state") {
            if parsed > 0 {
                resolved.insert(state.to_lowercase(), parsed as u64);
            }
        }
    }

    resolved
}

fn resolve_openhands<E: Environment>(
    openhands: &OpenHandsFrontMatter,
    env: &E,
) -> Result<OpenHandsConfig, WorkflowConfigError> {
    Ok(OpenHandsConfig {
        transport: OpenHandsTransportConfig {
            base_url: resolve_string_or_default(
                openhands.transport.base_url.as_deref(),
                env,
                "openhands.transport.base_url",
                DEFAULT_OPENHANDS_BASE_URL,
            )?,
            session_api_key_env: normalize_optional_literal(
                &openhands.transport.session_api_key_env,
            ),
        },
        local_server: OpenHandsLocalServerConfig {
            enabled: openhands.local_server.enabled.unwrap_or(true),
            command: resolve_command(
                openhands.local_server.command.as_deref(),
                "openhands.local_server.command",
                default_openhands_local_server_command(),
            )?,
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
            stdio_servers: resolve_stdio_servers(openhands.mcp.stdio_servers.as_deref())?,
        },
    })
}

fn resolve_openhands_conversation<E: Environment>(
    conversation: &OpenHandsConversationFrontMatter,
    env: &E,
) -> Result<OpenHandsConversationConfig, WorkflowConfigError> {
    let confirmation_policy = conversation.confirmation_policy.clone();
    if let Some(policy) = &confirmation_policy {
        if policy.kind.trim().is_empty() {
            return Err(WorkflowConfigError::InvalidField {
                field: "openhands.conversation.confirmation_policy.kind",
                message: "must not be empty".to_owned(),
            });
        }
    }

    Ok(OpenHandsConversationConfig {
        reuse_policy: resolve_string_or_default(
            conversation.reuse_policy.as_deref(),
            env,
            "openhands.conversation.reuse_policy",
            "per_issue",
        )?,
        persistence_dir_relative: resolve_relative_path(
            conversation.persistence_dir_relative.as_deref(),
            env,
            "openhands.conversation.persistence_dir_relative",
            DEFAULT_OPENHANDS_PERSISTENCE_DIR,
        )?,
        max_iterations: resolve_positive_u64(
            conversation.max_iterations.as_ref(),
            "openhands.conversation.max_iterations",
            DEFAULT_OPENHANDS_MAX_ITERATIONS,
        )?,
        stuck_detection: conversation.stuck_detection.unwrap_or(true),
        confirmation_policy,
        agent: conversation
            .agent
            .as_ref()
            .map(|agent| resolve_openhands_agent(agent, env))
            .transpose()?,
    })
}

fn resolve_openhands_agent<E: Environment>(
    agent: &OpenHandsConversationAgentFrontMatter,
    env: &E,
) -> Result<OpenHandsConversationAgentConfig, WorkflowConfigError> {
    Ok(OpenHandsConversationAgentConfig {
        kind: normalize_optional_literal(&agent.kind),
        llm: agent
            .llm
            .as_ref()
            .map(|llm| resolve_openhands_llm(llm, env))
            .transpose()?,
        log_completions: agent.log_completions.unwrap_or(false),
        options: agent.options.clone(),
    })
}

fn resolve_openhands_llm<E: Environment>(
    llm: &OpenHandsLlmFrontMatter,
    env: &E,
) -> Result<OpenHandsLlmConfig, WorkflowConfigError> {
    Ok(OpenHandsLlmConfig {
        model: llm
            .model
            .as_deref()
            .map(|value| resolve_string(value, env, "openhands.conversation.agent.llm.model"))
            .transpose()?,
        api_key_env: normalize_optional_literal(&llm.api_key_env),
        base_url_env: normalize_optional_literal(&llm.base_url_env),
        options: llm.options.clone(),
    })
}

fn resolve_stdio_servers(
    raw: Option<&[OpenHandsStdioServerFrontMatter]>,
) -> Result<Vec<OpenHandsStdioServerConfig>, WorkflowConfigError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };

    raw.iter()
        .map(|server| {
            let name = require_literal(
                Some(server.name.as_str()),
                "openhands.mcp.stdio_servers[].name",
            )?;
            let command = resolve_command(
                Some(server.command.as_slice()),
                "openhands.mcp.stdio_servers[].command",
                Vec::new(),
            )?;

            Ok(OpenHandsStdioServerConfig { name, command })
        })
        .collect()
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

    Ok(normalize_path(&base_dir.join(expanded)))
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
