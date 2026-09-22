//! Scheduler projection of the redacted source stream; wire evidence stays on the owner.
use std::collections::BTreeMap;

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

#[derive(Default)]
pub struct RuntimeProjection {
    last: Option<(u64, u64)>,
    tools: BTreeMap<String, Value>,
    tool_bytes: usize,
    prompt_request: Option<Value>,
}

impl RuntimeProjection {
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
        self.last = Some(cursor);
        if *replay || source_run != run_id {
            return None;
        }
        if frame.direction == "outgoing" {
            if frame.payload["method"] == "session/prompt" {
                self.prompt_request = frame.payload.get("id").cloned();
            }
            return None;
        }
        if frame.direction != "incoming" {
            return None;
        }
        if frame.payload.get("method").is_none()
            && self
                .prompt_request
                .as_ref()
                .is_some_and(|id| frame.payload.get("id") == Some(id))
        {
            self.prompt_request = None;
            // Optional end-turn usage is an SDK extension. Preserve only reported
            // counters, separately from context occupancy and session totals.
            let usage = frame.payload.pointer("/result/usage")?.as_object()?;
            let payload = usage
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
            if payload.is_empty() {
                return None;
            }
            return Some(RuntimeUpdate {
                sequence: frame.sequence,
                generation: *generation,
                observed_at: frame.observed_at,
                kind: "turn_usage".into(),
                summary: None,
                payload: Value::Object(payload),
            });
        }
        self.project(*generation, frame)
    }

    fn project(&mut self, generation: u64, frame: &SourceFrame) -> Option<RuntimeUpdate> {
        let method = frame.payload.get("method")?.as_str()?;
        if method == "session/request_permission" {
            return Some(RuntimeUpdate {
                sequence: frame.sequence,
                generation,
                observed_at: frame.observed_at,
                kind: "waiting_for_input".into(),
                summary: Some(
                    "ACP agent requested permission; operator responses are unavailable".into(),
                ),
                payload: json!({"reason":"permission_request", "operator_responses":false}),
            });
        }
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
            _ => return None,
        };
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

pub fn run_capability(
    state: &crate::opensymphony_workspace::AcpSessionState,
) -> crate::opensymphony_gateway_schema::capability::HarnessRunCapability {
    let caps = &state.initialization["agentCapabilities"];
    crate::opensymphony_gateway_schema::capability::HarnessRunCapability {
        harness: "acp".into(),
        profile_id: state.identity.profile_id.clone(),
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
        model_selection: false,
        cancellation: true,
        operator_responses: false,
    }
}

pub fn profile_capabilities(
    config: &crate::opensymphony_workflow::AcpConfig,
) -> Vec<crate::opensymphony_gateway_schema::capability::HarnessProfileCapability> {
    config
        .profiles
        .iter()
        .map(|(id, profile)| {
            let command = std::path::Path::new(&profile.command);
            let available = if command.components().count() > 1 {
                command.is_absolute() && executable(command)
            } else {
                std::env::var_os("PATH").is_some_and(|paths| {
                    std::env::split_paths(&paths).any(|path| executable(&path.join(command)))
                })
            };
            let reason = if profile.validate().is_err() {
                Some("invalid_profile")
            } else if !available {
                Some("executable_unavailable")
            } else if profile
                .env_refs
                .values()
                .any(|name| std::env::var(name).map_or(true, |value| value.is_empty()))
            {
                Some("credential_reference_unavailable")
            } else {
                None
            };
            crate::opensymphony_gateway_schema::capability::HarnessProfileCapability {
                harness: "acp".into(),
                profile_id: id.clone(),
                preflight_ready: reason.is_none(),
                unavailable_reason: reason.map(str::to_owned),
            }
        })
        .collect()
}

fn executable(path: &std::path::Path) -> bool {
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
}
