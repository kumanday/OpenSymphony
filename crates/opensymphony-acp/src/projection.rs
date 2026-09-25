//! Scheduler projection of the redacted source stream; wire evidence stays on the owner.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use serde_json::{Value, json};

use super::{SessionEvent, SourceFrame};

#[derive(Debug)]
pub struct RuntimeUpdate {
    pub sequence: u64,
    pub generation: u64,
    pub observed_at: chrono::DateTime<chrono::Utc>,
    pub kind: String,
    pub summary: Option<String>,
    pub payload: Value,
}

pub(super) fn reported_turn_usage(payload: &Value) -> Option<serde_json::Map<String, Value>> {
    let reason = payload.pointer("/result/stopReason")?.as_str()?;
    if payload.get("method").is_some() || reason.is_empty() || reason.len() > 1024 {
        return None;
    }
    let usage = payload.pointer("/result/usage")?.as_object()?;
    let reported = usage
        .iter()
        .filter(|(name, value)| {
            matches!(
                name.as_str(),
                "totalTokens"
                    | "inputTokens"
                    | "outputTokens"
                    | "thoughtTokens"
                    | "cachedReadTokens"
                    | "cachedWriteTokens"
            ) && value.is_u64()
        })
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<serde_json::Map<_, _>>();
    (!reported.is_empty()).then_some(reported)
}

#[derive(Default)]
pub struct RuntimeProjection {
    cursor_enabled: bool,
    cursor_session_id: Option<String>,
    // Redacted, bounded candidates become activity only after the host's
    // correlated accepted response. A rejected callback is not a todo update.
    cursor_todo_candidates: BTreeMap<String, Option<Value>>,
    last: Option<(u64, u64)>,
    tools: BTreeMap<String, Value>,
    tool_bytes: usize,
    prompt_request: Option<Value>,
    terminal_creates: BTreeMap<String, (String, Option<String>)>,
    terminal_waits: BTreeMap<String, (String, bool)>,
    completed_terminals: BTreeSet<String>,
    filesystem_callbacks: BTreeSet<String>,
}

impl RuntimeProjection {
    pub fn after(cursor: (u64, u64)) -> Self {
        Self {
            last: Some(cursor),
            ..Self::default()
        }
    }

    pub fn with_cursor(
        mut self,
        enabled: bool,
        session_id: Option<&str>,
        _workspace: Option<&Path>,
    ) -> Self {
        self.cursor_enabled = enabled;
        self.cursor_session_id = session_id.map(str::to_owned);
        self
    }

    pub fn last_cursor(&self) -> Option<(u64, u64)> {
        self.last
    }

    pub fn apply(&mut self, event: &SessionEvent, run_id: &str) -> Option<RuntimeUpdate> {
        let SessionEvent::Source {
            generation,
            run_id: source_run,
            replay,
            frame,
        } = event
        else {
            return None;
        };
        let cursor = (*generation, frame.sequence);
        if self.last.is_some_and(|last| cursor <= last) {
            return None;
        }
        if self.last.is_some_and(|last| last.0 != *generation) {
            self.cursor_todo_candidates.clear();
        }
        self.last = Some(cursor);
        if *replay || source_run != run_id {
            return None;
        }
        if frame.direction == "outgoing" {
            if frame.payload["method"] == "session/prompt" {
                self.prompt_request = frame.payload.get("id").cloned();
            }
            if self.cursor_enabled
                && frame.payload.get("method").is_none()
                && let Some(id) = frame.payload.get("id")
                && (id.is_number() || id.is_string())
                && let Some(Some(payload)) = self.cursor_todo_candidates.remove(&id.to_string())
                && frame.payload.pointer("/result/outcome/outcome") == Some(&json!("accepted"))
            {
                return Some(RuntimeUpdate {
                    sequence: frame.sequence,
                    generation: *generation,
                    observed_at: frame.observed_at,
                    kind: "vendor_todos".into(),
                    summary: Some("Cursor todos updated".into()),
                    payload,
                });
            }
            return self
                .project_filesystem_response(*generation, frame)
                .or_else(|| self.project_terminal_response(*generation, frame));
        }
        if frame.direction != "incoming" {
            return None;
        }
        if self.cursor_enabled
            && self.cursor_session_id.is_some()
            && frame.payload.get("method").and_then(Value::as_str) == Some("cursor/update_todos")
            && let Some(params) = frame.payload.get("params")
            && params
                .get("sessionId")
                .is_none_or(|id| id.as_str() == self.cursor_session_id.as_deref())
            && super::extensions::cursor_todos(params)
            && let Some(id) = frame.payload.get("id")
            && (id.is_number() || id.is_string())
            && id.to_string().len() <= 128
        {
            let key = id.to_string();
            if let Some(candidate) = self.cursor_todo_candidates.get_mut(&key) {
                *candidate = None;
            } else if self.cursor_todo_candidates.len() < 16 {
                self.cursor_todo_candidates.insert(
                    key,
                    Some(json!({
                        "todos": params["todos"], "merge": params["merge"]
                    })),
                );
            }
            return None;
        }
        if let Some(activity) = self.project_filesystem_request(*generation, frame) {
            return Some(activity);
        }
        if let Some(running_poll) = self.record_terminal_request(frame) {
            return running_poll
                .then(|| callback_activity(*generation, frame, "ACP terminal output polled"));
        }
        if frame.payload.get("method").is_none()
            && self
                .prompt_request
                .as_ref()
                .is_some_and(|id| frame.payload.get("id") == Some(id))
        {
            self.prompt_request = None;
            // Optional end-turn usage is an SDK extension. Preserve reported
            // response counters separately from context occupancy; do not infer
            // delta accumulation from the peer's response observation.
            let payload = reported_turn_usage(&frame.payload)?;
            return Some(RuntimeUpdate {
                sequence: frame.sequence,
                generation: *generation,
                observed_at: frame.observed_at,
                kind: "turn_usage".into(),
                summary: Some("ACP turn usage reported".into()),
                payload: Value::Object(payload),
            });
        }
        self.project(*generation, frame)
    }

    fn project_filesystem_request(
        &mut self,
        generation: u64,
        frame: &SourceFrame,
    ) -> Option<RuntimeUpdate> {
        let method = frame.payload.get("method")?.as_str()?;
        if !matches!(method, "fs/read_text_file" | "fs/write_text_file") {
            return None;
        }
        if let Some(id) = frame
            .payload
            .get("id")
            .and_then(rpc_id)
            .filter(|id| id.len() <= 256)
        {
            if self.filesystem_callbacks.len() >= 1024 {
                self.filesystem_callbacks.pop_first();
            }
            self.filesystem_callbacks.insert(id);
        }
        Some(callback_activity(
            generation,
            frame,
            "ACP filesystem callback requested",
        ))
    }

    fn project_filesystem_response(
        &mut self,
        generation: u64,
        frame: &SourceFrame,
    ) -> Option<RuntimeUpdate> {
        if frame.payload.get("method").is_some() {
            return None;
        }
        let id = frame.payload.get("id").and_then(rpc_id)?;
        self.filesystem_callbacks
            .remove(&id)
            .then(|| callback_activity(generation, frame, "ACP filesystem callback completed"))
    }

    // Some(true) means a running output poll can advance scheduler liveness.
    // Other terminal requests are consumed without a projected event.
    fn record_terminal_request(&mut self, frame: &SourceFrame) -> Option<bool> {
        let method = frame.payload.get("method").and_then(Value::as_str)?;
        if !matches!(
            method,
            "terminal/create" | "terminal/wait_for_exit" | "terminal/output"
        ) {
            return None;
        }
        let Some(id) = frame.payload.get("id").and_then(rpc_id) else {
            return Some(false);
        };
        if method == "terminal/create" {
            let Some(command) = frame
                .payload
                .pointer("/params/command")
                .and_then(Value::as_str)
            else {
                return Some(false);
            };
            let args = match frame.payload.pointer("/params/args") {
                Some(Value::Array(args)) => args.as_slice(),
                None => &[],
                _ => return Some(false),
            };
            let Some(command) = terminal_command(command, args) else {
                return Some(false);
            };
            let cwd = frame
                .payload
                .pointer("/params/cwd")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if self.terminal_creates.len() >= 1024 {
                self.terminal_creates.pop_first();
            }
            self.terminal_creates.insert(id, (command, cwd));
        } else if let Some(terminal_id) = frame
            .payload
            .pointer("/params/terminalId")
            .and_then(Value::as_str)
        {
            let running_poll =
                method == "terminal/output" && !self.completed_terminals.contains(terminal_id);
            if self.terminal_waits.len() >= 1024 {
                self.terminal_waits.pop_first();
            }
            self.terminal_waits
                .insert(id, (terminal_id.to_owned(), method == "terminal/output"));
            return Some(running_poll);
        }
        Some(false)
    }

    fn project_terminal_response(
        &mut self,
        generation: u64,
        frame: &SourceFrame,
    ) -> Option<RuntimeUpdate> {
        if frame.payload.get("method").is_some() {
            return None;
        }
        let id = frame.payload.get("id").and_then(rpc_id)?;
        if let Some((command, cwd)) = self.terminal_creates.remove(&id) {
            let terminal_id = frame
                .payload
                .pointer("/result/terminalId")
                .and_then(Value::as_str)?;
            return Some(RuntimeUpdate {
                sequence: frame.sequence,
                generation,
                observed_at: frame.observed_at,
                kind: "command_started".into(),
                summary: Some("ACP terminal started".into()),
                payload: json!({"command_id":terminal_id,"command":command,"cwd":cwd}),
            });
        }
        let (terminal_id, output_poll) = self.terminal_waits.remove(&id)?;
        if self.completed_terminals.contains(&terminal_id) {
            return None;
        }
        if output_poll
            && frame
                .payload
                .pointer("/result/exitStatus")
                .is_some_and(Value::is_null)
        {
            return Some(callback_activity(
                generation,
                frame,
                "ACP terminal output received",
            ));
        }
        let exit_code = frame
            .payload
            .pointer("/result/exitCode")
            .or_else(|| frame.payload.pointer("/result/exitStatus/exitCode"))
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())?;
        if self.completed_terminals.len() >= 1024 {
            self.completed_terminals.pop_first();
        }
        self.completed_terminals.insert(terminal_id.clone());
        Some(RuntimeUpdate {
            sequence: frame.sequence,
            generation,
            observed_at: frame.observed_at,
            kind: "command_finished".into(),
            summary: Some("ACP terminal finished".into()),
            payload: json!({"command_id":terminal_id,"exit_code":exit_code}),
        })
    }

    fn project(&mut self, generation: u64, frame: &SourceFrame) -> Option<RuntimeUpdate> {
        let method = frame.payload.get("method")?.as_str()?;
        if method != "session/update" {
            return None;
        }
        let mut update = frame.payload.pointer("/params/update")?.clone();
        let kind = update.get("sessionUpdate")?.as_str()?.to_owned();
        let summary = match kind.as_str() {
            "agent_message_chunk" | "agent_thought_chunk" | "user_message_chunk" => update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .map(str::to_owned),
            "tool_call" | "tool_call_update" => {
                let id = update.get("toolCallId")?.as_str()?.to_owned();
                // Keep partial tool updates merged while bounding aggregate retained state.
                let mut merged = self.tools.get(&id).cloned().unwrap_or_else(|| json!({}));
                for (key, value) in update.as_object()? {
                    if !value.is_null() {
                        merged[key] = value.clone();
                    }
                }
                let bytes = serde_json::to_vec(&merged).ok()?.len();
                const MAX_TOOL_BYTES: usize = 1024 * 1024;
                if bytes > MAX_TOOL_BYTES {
                    return None;
                }
                if let Some(previous) = self.tools.remove(&id) {
                    self.tool_bytes -= serde_json::to_vec(&previous).ok()?.len();
                }
                while self.tools.len() >= 1024 || self.tool_bytes + bytes > MAX_TOOL_BYTES {
                    let (_, removed) = self.tools.pop_first()?;
                    self.tool_bytes -= serde_json::to_vec(&removed).ok()?.len();
                }
                self.tool_bytes += bytes;
                self.tools.insert(id, merged.clone());
                update = merged;
                update
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            }
            "plan"
            | "usage_update"
            | "available_commands_update"
            | "current_mode_update"
            | "config_option_update" => None,
            _ => {
                // Source frames are already redacted by Capture. Keep future
                // update shapes visible as activity without letting an
                // unrecognized payload grow scheduler storage without bound.
                const MAX_UNKNOWN_UPDATE_BYTES: usize = 16 * 1024;
                let payload = if serde_json::to_vec(&update).ok()?.len() <= MAX_UNKNOWN_UPDATE_BYTES
                {
                    update
                } else {
                    json!({
                        "sessionUpdate": kind.chars().take(128).collect::<String>(),
                        "truncated": true
                    })
                };
                return Some(RuntimeUpdate {
                    sequence: frame.sequence,
                    generation,
                    observed_at: frame.observed_at,
                    kind: "session_update".into(),
                    summary: Some("ACP session update".into()),
                    payload,
                });
            }
        };
        // Activity storage requires a summary; structural updates must remain
        // visible even when the peer sends no text or title.
        let summary = summary.or_else(|| {
            Some(
                match kind.as_str() {
                    "agent_message_chunk" => "ACP agent message",
                    "agent_thought_chunk" => "ACP agent reasoning",
                    "user_message_chunk" => "ACP user message",
                    "tool_call" | "tool_call_update" => "ACP tool updated",
                    "plan" => "ACP plan updated",
                    "usage_update" => "ACP context usage updated",
                    "available_commands_update" => "ACP commands updated",
                    "current_mode_update" => "ACP mode updated",
                    "config_option_update" => "ACP configuration updated",
                    _ => unreachable!("known projection kind"),
                }
                .into(),
            )
        });
        // usage_update describes context occupancy/cost, not input/output token deltas.
        // Keep its optional fields intact instead of charging invented token counters.
        Some(RuntimeUpdate {
            sequence: frame.sequence,
            generation,
            observed_at: frame.observed_at,
            kind,
            summary,
            payload: update,
        })
    }
}

fn callback_activity(generation: u64, frame: &SourceFrame, summary: &str) -> RuntimeUpdate {
    RuntimeUpdate {
        sequence: frame.sequence,
        generation,
        observed_at: frame.observed_at,
        kind: "callback_activity".into(),
        summary: Some(summary.into()),
        payload: Value::Null,
    }
}

fn rpc_id(value: &Value) -> Option<String> {
    (value.is_string() || value.is_number())
        .then(|| serde_json::to_string(value).ok())
        .flatten()
}

fn terminal_command(command: &str, args: &[Value]) -> Option<String> {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(command.to_owned());
    for arg in args {
        parts.push(arg.as_str()?.to_owned());
    }
    let rendered = parts
        .iter()
        .map(|part| {
            if part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._/:=@".contains(&byte))
            {
                part.clone()
            } else {
                format!("'{}'", part.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    (rendered.len() <= 8192).then_some(rendered)
}

pub fn run_capability(
    state: &crate::opensymphony_workspace::AcpSessionState,
) -> crate::opensymphony_gateway_schema::capability::HarnessRunCapability {
    let caps = &state.initialization["agentCapabilities"];
    crate::opensymphony_gateway_schema::capability::HarnessRunCapability {
        harness: "acp".into(),
        profile_id: state.identity.profile_id.clone(),
        run_binding_id: Some(state.identity.run_id.clone()),
        protocol: "acp".into(),
        protocol_version: 1,
        rpc: "json_rpc_2_0".into(),
        encoding: "utf-8".into(),
        framing: "lf".into(),
        carrier: "stdio".into(),
        session_restore: caps["loadSession"].as_bool() == Some(true)
            || caps
                .pointer("/sessionCapabilities/resume")
                .is_some_and(|v| !v.is_null()),
        history_replay: caps["loadSession"].as_bool() == Some(true),
        model_selection: state.model_selection,
        cancellation: true,
        operator_responses: true,
        operations: state.enabled_operations.clone(),
    }
}

pub fn profile_capabilities(
    config: &crate::opensymphony_workflow::AcpConfig,
    worker_environment: &BTreeMap<String, String>,
    excluded_environment: &BTreeSet<String>,
) -> Vec<crate::opensymphony_gateway_schema::capability::HarnessProfileCapability> {
    let mut environment = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| !super::is_reserved_memory_environment_name(name))
        .collect::<BTreeMap<_, _>>();
    for (name, value) in worker_environment
        .iter()
        .filter(|(name, _)| !super::is_reserved_memory_environment_name(name))
    {
        crate::opensymphony_workspace::insert_environment_value(
            &mut environment,
            name.clone(),
            value.clone(),
        );
    }
    profile_capabilities_with_environment(config, &environment, excluded_environment)
}

fn profile_capabilities_with_environment(
    config: &crate::opensymphony_workflow::AcpConfig,
    environment: &BTreeMap<String, String>,
    excluded_environment: &BTreeSet<String>,
) -> Vec<crate::opensymphony_gateway_schema::capability::HarnessProfileCapability> {
    use crate::opensymphony_workspace::{
        environment_variable_names_equal, has_environment_name_collision,
    };
    let value = |name: &str| {
        environment
            .iter()
            .find(|(key, _)| environment_variable_names_equal(key, name))
            .map(|(_, value)| value)
    };
    let excluded = |name: &str| {
        excluded_environment
            .iter()
            .any(|key| environment_variable_names_equal(key, name))
    };
    config
        .profiles
        .iter()
        .map(|(id, profile)| {
            // Launch replaces mapped targets and removes source-only variables.
            // Resolve PATH with the same precedence before advertising readiness.
            let path = match profile
                .env_refs
                .iter()
                .find(|(target, _)| environment_variable_names_equal(target, "PATH"))
            {
                Some((_, source)) => value(source),
                None if profile
                    .env_refs
                    .values()
                    .any(|source| environment_variable_names_equal(source, "PATH")) =>
                {
                    None
                }
                None => value("PATH"),
            };
            let command = Path::new(&profile.command);
            let available = if command.components().count() > 1 {
                command.is_absolute() && executable(command)
            } else {
                path.is_some_and(|paths| {
                    std::env::split_paths(paths).any(|path| {
                        // A profile preflight has no issue cwd yet.
                        path.is_absolute() && executable(&path.join(command))
                    })
                })
            };
            let reason = if profile.validate().is_err()
                || has_environment_name_collision(profile.env_refs.keys().map(String::as_str))
                || profile.env_refs.iter().any(|(target, source)| {
                    super::is_reserved_memory_environment_name(target)
                        || super::is_reserved_memory_environment_name(source)
                }) {
                Some("invalid_profile")
            } else if profile
                .env_refs
                .iter()
                .any(|(target, source)| excluded(target) || excluded(source))
            {
                Some("credential_reference_excluded")
            } else if profile.env_refs.values().any(|name| {
                value(name).is_none_or(|value| value.is_empty() || value.contains('\0'))
            }) {
                Some("credential_reference_unavailable")
            } else if !available {
                Some("executable_unavailable")
            } else {
                None
            };
            crate::opensymphony_gateway_schema::capability::HarnessProfileCapability {
                harness: "acp".into(),
                profile_id: id.clone(),
                preflight_ready: reason.is_none(),
                unavailable_reason: reason.map(str::to_owned),
                operations: super::extensions::configured_operations(profile),
            }
        })
        .collect()
}

fn executable(path: &Path) -> bool {
    #[cfg(windows)]
    if path.extension().is_none() && path.with_extension("exe").is_file() {
        return true;
    }
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_preflight_resolves_mapped_path_and_rejects_missing_references() {
        let temp = tempfile::tempdir().expect("temporary executable directory");
        let command = if cfg!(windows) { "peer.exe" } else { "peer" };
        let path = temp.path().join(command);
        std::fs::write(&path, "fixture").expect("executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("executable permissions");
        }
        let mut config: crate::opensymphony_workflow::AcpConfig = serde_json::from_value(json!({
            "profiles": {"profile": {"command": "peer", "env_refs": {"PATH": "ACP_PATH"}}}
        }))
        .expect("profile");
        let environment = BTreeMap::from([
            (
                "PATH".into(),
                temp.path().join("missing").to_string_lossy().into_owned(),
            ),
            (
                "ACP_PATH".into(),
                temp.path().to_string_lossy().into_owned(),
            ),
        ]);
        assert!(
            profile_capabilities_with_environment(&config, &environment, &BTreeSet::new())[0]
                .preflight_ready
        );
        config
            .profiles
            .get_mut("profile")
            .expect("profile")
            .extensions
            .push("fixture_echo@1".into());
        let advertised =
            profile_capabilities_with_environment(&config, &environment, &BTreeSet::new());
        assert_eq!(advertised[0].operations[0].operation_id, "fixture.echo");
        assert_eq!(
            advertised[0].operations[0].parameters_schema["required"],
            json!(["value"])
        );
        let profile = config.profiles.get_mut("profile").expect("profile");
        profile.env_refs.insert("PATH".into(), "MISSING".into());
        assert_eq!(
            profile_capabilities_with_environment(&config, &environment, &BTreeSet::new())[0]
                .unavailable_reason
                .as_deref(),
            Some("credential_reference_unavailable")
        );
        let profile = config.profiles.get_mut("profile").expect("profile");
        profile.env_refs = BTreeMap::from([("OTHER".into(), "PATH".into())]);
        let environment =
            BTreeMap::from([("PATH".into(), temp.path().to_string_lossy().into_owned())]);
        assert_eq!(
            profile_capabilities_with_environment(&config, &environment, &BTreeSet::new())[0]
                .unavailable_reason
                .as_deref(),
            Some("executable_unavailable")
        );
    }

    #[test]
    fn profile_preflight_includes_resolved_worker_environment() {
        let source = "OPENSYMPHONY_ACP_PREFLIGHT_OVERLAY_ONLY_TEST";
        let config: crate::opensymphony_workflow::AcpConfig = serde_json::from_value(json!({
            "profiles": {
                "profile": {
                    "command": std::env::current_exe().expect("test executable"),
                    "env_refs": {"LINEAR_API_KEY": source}
                }
            }
        }))
        .expect("profile");
        assert_eq!(
            profile_capabilities(&config, &BTreeMap::new(), &BTreeSet::new())[0]
                .unavailable_reason
                .as_deref(),
            Some("credential_reference_unavailable")
        );
        let overlay = BTreeMap::from([(source.into(), "resolved-worker-token".into())]);
        assert!(profile_capabilities(&config, &overlay, &BTreeSet::new())[0].preflight_ready);
    }

    #[test]
    fn profile_preflight_rejects_excluded_checkout_credential_mapping() {
        let config: crate::opensymphony_workflow::AcpConfig = serde_json::from_value(json!({
            "profiles": {
                "profile": {
                    "command": std::env::current_exe().expect("test executable"),
                    "env_refs": {"ACP_AUTH": "GITHUB_TOKEN"}
                }
            }
        }))
        .expect("profile");
        let overlay = BTreeMap::from([("GITHUB_TOKEN".into(), "checkout-only".into())]);
        assert!(profile_capabilities(&config, &overlay, &BTreeSet::new())[0].preflight_ready);
        let excluded = BTreeSet::from(["GITHUB_TOKEN".into()]);
        let capability = &profile_capabilities(&config, &overlay, &excluded)[0];
        assert!(!capability.preflight_ready);
        assert_eq!(
            capability.unavailable_reason.as_deref(),
            Some("credential_reference_excluded")
        );
        let excluded_target = BTreeSet::from(["ACP_AUTH".into()]);
        assert_eq!(
            profile_capabilities(&config, &overlay, &excluded_target)[0]
                .unavailable_reason
                .as_deref(),
            Some("credential_reference_excluded")
        );
    }

    #[test]
    fn profile_preflight_cannot_map_into_reserved_memory_scope() {
        let mut config: crate::opensymphony_workflow::AcpConfig = serde_json::from_value(json!({
            "profiles": {
                "profile": {
                    "command": std::env::current_exe().expect("test executable"),
                    "env_refs": {"OPENSYMPHONY_MEMORY_ADMIN_TOKEN": "AGENT_TOKEN"}
                }
            }
        }))
        .expect("profile");
        let overlay = BTreeMap::from([("AGENT_TOKEN".into(), "unscoped-bearer".into())]);
        let capability = &profile_capabilities(&config, &overlay, &BTreeSet::new())[0];
        assert_eq!(
            capability.unavailable_reason.as_deref(),
            Some("invalid_profile")
        );
        config
            .profiles
            .get_mut("profile")
            .expect("profile")
            .env_refs =
            BTreeMap::from([("AGENT_TOKEN".into(), "OPENSYMPHONY_MEMORY_TOKEN".into())]);
        let capability = &profile_capabilities(&config, &overlay, &BTreeSet::new())[0];
        assert_eq!(
            capability.unavailable_reason.as_deref(),
            Some("invalid_profile")
        );
    }

    fn event(sequence: u64, replay: bool, update: Value) -> SessionEvent {
        SessionEvent::Source {
            generation: 1,
            run_id: "run".into(),
            replay,
            frame: SourceFrame {
                sequence,
                direction: "incoming".into(),
                observed_at: chrono::Utc::now(),
                payload: json!({"method":"session/update","params":{"update":update}}),
            },
        }
    }

    fn callback_event(sequence: u64, direction: &str, payload: Value) -> SessionEvent {
        SessionEvent::Source {
            generation: 1,
            run_id: "run".into(),
            replay: false,
            frame: SourceFrame {
                sequence,
                direction: direction.into(),
                observed_at: chrono::Utc::now(),
                payload,
            },
        }
    }

    fn timed_callback_event(
        sequence: u64,
        observed_ms: i64,
        direction: &str,
        payload: Value,
    ) -> SessionEvent {
        let mut event = callback_event(sequence, direction, payload);
        let SessionEvent::Source { frame, .. } = &mut event else {
            unreachable!("callback fixture is a source event")
        };
        frame.observed_at =
            chrono::DateTime::from_timestamp_millis(observed_ms).expect("fixture timestamp");
        event
    }

    #[test]
    fn raw_operator_callbacks_do_not_claim_an_unrouted_wait() {
        let mut projection = RuntimeProjection::default();
        for (sequence, method) in [(1, "session/request_permission"), (2, "elicitation/create")] {
            let update = projection.apply(
                &callback_event(
                    sequence,
                    "incoming",
                    json!({
                        "id":sequence,"method":method,"params":{"sessionId":"s","secret":"private"}
                    }),
                ),
                "run",
            );
            assert!(update.is_none(), "only accepted routed requests may wait");
        }
    }

    #[test]
    fn cursor_todo_request_projects_only_for_enabled_bound_session() {
        let payload = |session: Option<&str>| {
            let mut value = json!({"id":0,"method":"cursor/update_todos","params":{
            "toolCallId":"todo-1","merge":true,
            "todos":[{"id":"one","content":"Verify","status":"completed"}]}});
            if let Some(session) = session {
                value["params"]["sessionId"] = json!(session);
            }
            value
        };
        let mut disabled = RuntimeProjection::default();
        assert!(
            disabled
                .apply(&callback_event(1, "incoming", payload(None)), "run")
                .is_none()
        );
        let mut enabled = RuntimeProjection::default().with_cursor(true, Some("s"), None);
        assert!(
            enabled
                .apply(
                    &callback_event(1, "incoming", payload(Some("other"))),
                    "run"
                )
                .is_none()
        );
        assert!(
            enabled
                .apply(&callback_event(2, "incoming", payload(None)), "run")
                .is_none()
        );
        let update = enabled
            .apply(
                &callback_event(
                    3,
                    "outgoing",
                    json!({"id":0,"result":{"outcome":{"outcome":"accepted","todos":[]}}}),
                ),
                "run",
            )
            .expect("accepted bound update");
        assert_eq!(update.kind, "vendor_todos");
        assert_eq!(update.payload["todos"][0]["content"], "Verify");
        assert!(enabled.apply(&callback_event(4, "incoming", json!({"method":"cursor/update_todos","params":{"toolCallId":"todo-1","merge":true,"todos":[]}})), "run").is_none());
        assert!(enabled.apply(&callback_event(5, "incoming", json!({"id":0,"method":"cursor/update_todos","params":{"sessionId":7,"toolCallId":"todo-1","merge":false,"todos":[]}})), "run").is_none());
        assert!(enabled.apply(&callback_event(6, "incoming", json!({"id":0,"method":"cursor/task","params":{"toolCallId":"task","description":"unqualified","prompt":"noop","subagentType":"general"}})), "run").is_none());
        assert!(
            enabled
                .apply(&callback_event(7, "incoming", payload(None)), "run")
                .is_none()
        );
        assert!(
            enabled
                .apply(
                    &callback_event(
                        8,
                        "outgoing",
                        json!({"id":0,"result":{"outcome":{"outcome":"rejected"}}})
                    ),
                    "run"
                )
                .is_none()
        );
        assert!(enabled.cursor_todo_candidates.is_empty());
        assert!(
            enabled
                .apply(&callback_event(9, "incoming", payload(None)), "run")
                .is_none()
        );
        let mut next_generation = callback_event(
            1,
            "outgoing",
            json!({"id":0,"result":{"outcome":{"outcome":"accepted"}}}),
        );
        if let SessionEvent::Source { generation, .. } = &mut next_generation {
            *generation += 1;
        }
        assert!(enabled.apply(&next_generation, "run").is_none());
        assert!(enabled.cursor_todo_candidates.is_empty());
    }

    #[test]
    fn cursor_todo_candidates_are_bounded_and_duplicate_ids_are_ambiguous() {
        let mut projection = RuntimeProjection::default().with_cursor(true, Some("s"), None);
        for sequence in 1..=17 {
            let request = json!({"id":sequence,"method":"cursor/update_todos","params":{
                "toolCallId":"todo","merge":false,"todos":[]}});
            assert!(
                projection
                    .apply(&callback_event(sequence, "incoming", request), "run")
                    .is_none()
            );
        }
        assert_eq!(projection.cursor_todo_candidates.len(), 16);
        assert!(
            projection
                .apply(
                    &callback_event(
                        18,
                        "outgoing",
                        json!({"id":17,"result":{"outcome":{"outcome":"accepted"}}})
                    ),
                    "run"
                )
                .is_none()
        );
        assert!(
            projection
                .apply(
                    &callback_event(
                        19,
                        "incoming",
                        json!({"id":1,"method":"cursor/update_todos","params":{
            "toolCallId":"todo","merge":false,"todos":[]}})
                    ),
                    "run"
                )
                .is_none()
        );
        assert!(
            projection
                .apply(
                    &callback_event(
                        20,
                        "outgoing",
                        json!({"id":1,"result":{"outcome":{"outcome":"accepted"}}})
                    ),
                    "run"
                )
                .is_none()
        );
    }

    #[test]
    fn terminal_callback_responses_project_completed_command_once() {
        let mut projection = RuntimeProjection::default();
        let create = callback_event(
            1,
            "incoming",
            json!({"id":1,"method":"terminal/create","params":{"command":"cargo","args":["test"],"cwd":"/workspace/repositories/one"}}),
        );
        assert!(projection.apply(&create, "run").is_none());
        let started = projection
            .apply(
                &callback_event(
                    2,
                    "outgoing",
                    json!({"id":1,"result":{"terminalId":"terminal-1"}}),
                ),
                "run",
            )
            .expect("command start");
        assert_eq!(started.kind, "command_started");
        assert_eq!(started.payload["command"], "cargo test");
        assert_eq!(started.payload["cwd"], "/workspace/repositories/one");
        assert!(projection.apply(&create, "run").is_none());

        assert!(projection.apply(&callback_event(3, "incoming", json!({"id":2,"method":"terminal/wait_for_exit","params":{"terminalId":"terminal-1"}})), "run").is_none());
        let finished = projection
            .apply(
                &callback_event(4, "outgoing", json!({"id":2,"result":{"exitCode":0}})),
                "run",
            )
            .expect("command finish");
        assert_eq!(finished.kind, "command_finished");
        assert_eq!(
            finished.payload,
            json!({"command_id":"terminal-1","exit_code":0})
        );

        assert!(projection.apply(&callback_event(5, "incoming", json!({"id":3,"method":"terminal/output","params":{"terminalId":"terminal-1"}})), "run").is_none());
        assert!(
            projection
                .apply(
                    &callback_event(
                        6,
                        "outgoing",
                        json!({"id":3,"result":{"exitStatus":{"exitCode":0}}})
                    ),
                    "run"
                )
                .is_none()
        );
        assert!(
            projection
                .apply(
                    &callback_event(
                        7,
                        "incoming",
                        json!({"id":4,"method":"terminal/create","params":{"command":"false"}})
                    ),
                    "run"
                )
                .is_none()
        );
        assert!(
            projection
                .apply(
                    &callback_event(8, "outgoing", json!({"id":4,"error":{"code":-32603}})),
                    "run"
                )
                .is_none()
        );
        assert!(
            projection
                .apply(
                    &callback_event(
                        9,
                        "incoming",
                        json!({"id":5,"method":"terminal/create","params":{"command":"true"}})
                    ),
                    "run"
                )
                .is_none()
        );
        assert!(
            projection
                .apply(
                    &callback_event(
                        10,
                        "outgoing",
                        json!({"id":5,"result":{"terminalId":"terminal-2"}})
                    ),
                    "run"
                )
                .is_some()
        );
    }

    #[test]
    fn running_terminal_output_polls_keep_idle_deadline_sliding_without_completion() {
        use crate::opensymphony_domain::{DurationMs, StallMetadata, TimestampMs};

        let mut projection = RuntimeProjection::default();
        let mut stall = StallMetadata::new(TimestampMs::new(0), DurationMs::new(1_000));
        assert!(
            projection
                .apply(
                    &timed_callback_event(
                        1,
                        0,
                        "incoming",
                        json!({
                            "id": 1, "method": "terminal/create",
                            "params": {"command": "cargo", "args": ["test"]}
                        })
                    ),
                    "run"
                )
                .is_none()
        );
        let started = projection
            .apply(
                &timed_callback_event(
                    2,
                    0,
                    "outgoing",
                    json!({
                        "id": 1, "result": {"terminalId": "terminal-1"}
                    }),
                ),
                "run",
            )
            .expect("command start");
        assert_eq!(started.kind, "command_started");
        stall.observe_activity(TimestampMs::new(0));

        for (request_sequence, observed_ms) in [(3, 500), (5, 900), (7, 1_700)] {
            let request = projection
                .apply(
                    &timed_callback_event(
                        request_sequence,
                        observed_ms,
                        "incoming",
                        json!({
                            "id": request_sequence, "method": "terminal/output",
                            "params": {"terminalId": "terminal-1"}
                        }),
                    ),
                    "run",
                )
                .expect("running poll request activity");
            assert_eq!(request.kind, "callback_activity");
            assert_eq!(request.payload, Value::Null);
            stall.observe_activity(TimestampMs::new(observed_ms as u64));

            let response = projection
                .apply(
                    &timed_callback_event(
                        request_sequence + 1,
                        observed_ms + 1,
                        "outgoing",
                        json!({
                            "id": request_sequence,
                            "result": {"output": "private build output", "exitStatus": null}
                        }),
                    ),
                    "run",
                )
                .expect("running poll response activity");
            assert_eq!(response.kind, "callback_activity");
            assert_eq!(response.payload, Value::Null);
            assert!(
                !response
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .contains("private")
            );
            stall.observe_activity(TimestampMs::new((observed_ms + 1) as u64));
        }
        assert_eq!(stall.last_activity_at, TimestampMs::new(1_701));
        assert!(!stall.is_stalled_at(TimestampMs::new(2_000)));
        assert!(
            projection
                .apply(
                    &timed_callback_event(
                        9,
                        2_100,
                        "incoming",
                        json!({
                            "id": 9, "method": "terminal/output",
                            "params": {"terminalId": "terminal-1"}
                        })
                    ),
                    "run"
                )
                .is_some()
        );
        let finished = projection
            .apply(
                &timed_callback_event(10, 2_101, "outgoing", json!({
                    "id": 9, "result": {"output": "private build output", "exitStatus": {"exitCode": 0}}
                })),
                "run",
            )
            .expect("actual exit receipt");
        assert_eq!(finished.kind, "command_finished");
        assert_eq!(
            finished.payload,
            json!({"command_id":"terminal-1","exit_code":0})
        );
        assert!(
            projection
                .apply(
                    &timed_callback_event(
                        11,
                        2_102,
                        "incoming",
                        json!({
                            "id": 11, "method": "terminal/output",
                            "params": {"terminalId": "terminal-1"}
                        })
                    ),
                    "run"
                )
                .is_none()
        );
    }

    #[test]
    fn filesystem_callbacks_project_payload_free_scheduler_activity() {
        let mut projection = RuntimeProjection::default();
        for (request_sequence, method) in [(1, "fs/read_text_file"), (3, "fs/write_text_file")] {
            let request = callback_event(
                request_sequence,
                "incoming",
                json!({"id":request_sequence,"method":method,"params":{"path":"/private/file","content":"private payload"}}),
            );
            let activity = projection.apply(&request, "run").expect("callback request");
            assert_eq!(activity.kind, "callback_activity");
            assert_eq!(
                activity.summary.as_deref(),
                Some("ACP filesystem callback requested")
            );
            assert!(activity.payload.is_null());
            let response = callback_event(
                request_sequence + 1,
                "outgoing",
                json!({"id":request_sequence,"result":{"content":"private payload"}}),
            );
            let activity = projection
                .apply(&response, "run")
                .expect("callback response");
            assert_eq!(activity.kind, "callback_activity");
            assert_eq!(
                activity.summary.as_deref(),
                Some("ACP filesystem callback completed")
            );
            assert!(activity.payload.is_null());
        }
        assert!(
            projection
                .apply(
                    &callback_event(
                        5,
                        "outgoing",
                        json!({"id":1,"result":{"content":"private payload"}})
                    ),
                    "run"
                )
                .is_none()
        );
        let oversized_id = "x".repeat(1024);
        assert!(
            projection
                .apply(
                    &callback_event(
                        6,
                        "incoming",
                        json!({"id":oversized_id,"method":"fs/read_text_file"})
                    ),
                    "run"
                )
                .is_some()
        );
        assert!(
            projection
                .apply(
                    &callback_event(7, "outgoing", json!({"id":oversized_id,"result":{}})),
                    "run"
                )
                .is_none()
        );
    }
    #[test]
    fn prompt_usage_preserves_absence_and_ignores_replayed_responses() {
        let mut projection = RuntimeProjection::default();
        let mut request = event(1, false, Value::Null);
        if let SessionEvent::Source { frame, .. } = &mut request {
            frame.direction = "outgoing".into();
            frame.payload = json!({"id":"prompt", "method":"session/prompt"});
        }
        assert!(projection.apply(&request, "run").is_none());
        let mut response = event(2, false, Value::Null);
        if let SessionEvent::Source { frame, .. } = &mut response {
            frame.payload = json!({"id":"prompt", "result":{"stopReason":"end_turn", "usage":{"inputTokens":4,"outputTokens":2,"totalTokens":6}}});
        }
        let usage = projection.apply(&response, "run").expect("usage");
        assert_eq!(usage.kind, "turn_usage");
        assert!(usage.payload.get("cachedReadTokens").is_none());
        assert!(projection.apply(&response, "run").is_none());
        if let SessionEvent::Source { frame, replay, .. } = &mut response {
            frame.sequence = 3;
            *replay = true;
        }
        assert!(projection.apply(&response, "run").is_none());
        let future_usage = reported_turn_usage(
            &json!({"result":{"stopReason":"future_stop_reason","usage":{"inputTokens":3}}}),
        )
        .expect("future terminal usage");
        assert_eq!(future_usage["inputTokens"], 3);
    }

    #[test]
    fn partial_tools_replay_and_optional_usage() {
        let mut projection = RuntimeProjection::default();
        let initial = event(
            1,
            false,
            json!({"sessionUpdate":"tool_call","toolCallId":"t","title":"Read","status":"in_progress","content":[1]}),
        );
        assert!(projection.apply(&initial, "run").is_some());
        assert!(projection.apply(&initial, "run").is_none());
        let merged = projection.apply(&event(2, false, json!({"sessionUpdate":"tool_call_update","toolCallId":"t","status":"completed","title":null})), "run").expect("projected update");
        assert_eq!(merged.payload["title"], "Read");
        assert_eq!(merged.payload["content"], json!([1]));
        assert!(
            projection
                .apply(
                    &event(
                        3,
                        true,
                        json!({"sessionUpdate":"usage_update","used":42,"size":100})
                    ),
                    "run"
                )
                .is_none()
        );
        let usage = projection
            .apply(
                &event(
                    4,
                    false,
                    json!({"sessionUpdate":"usage_update","used":42,"size":100}),
                ),
                "run",
            )
            .expect("projected update");
        assert!(usage.payload.get("inputTokens").is_none());
    }

    #[test]
    fn unknown_session_updates_remain_bounded_scheduler_activity() {
        let mut projection = RuntimeProjection::default();
        let unknown = event(
            1,
            false,
            json!({"sessionUpdate":"vendor_progress","detail":{"status":"working","credential":"[redacted]"}}),
        );
        let projected = projection.apply(&unknown, "run").expect("generic activity");
        assert_eq!(projected.kind, "session_update");
        assert_eq!(projected.summary.as_deref(), Some("ACP session update"));
        assert_eq!(projected.payload["sessionUpdate"], "vendor_progress");
        assert_eq!(projected.payload["detail"]["credential"], "[redacted]");
        assert!(projection.apply(&unknown, "run").is_none(), "deduplicated");

        let large = event(
            2,
            false,
            json!({"sessionUpdate":"vendor_progress","data":"x".repeat(20 * 1024)}),
        );
        let projected = projection.apply(&large, "run").expect("bounded activity");
        assert_eq!(projected.kind, "session_update");
        assert_eq!(projected.payload["truncated"], true);
        assert!(serde_json::to_vec(&projected.payload).expect("JSON").len() < 1024);
    }
}
