//! Negotiated session selections; options remain private operational metadata.
use super::{AcpProfile, ClientError};
use agent_client_protocol::{
    Agent, ConnectionTo, UntypedMessage,
    schema::v1::{AgentCapabilities, McpServer},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SessionConfiguration {
    pub options: Vec<Value>,
    pub current_mode: Option<String>,
    pub available_modes: Vec<Value>,
}
impl std::fmt::Debug for SessionConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfiguration")
            .field("option_count", &self.options.len())
            .finish_non_exhaustive()
    }
}
impl SessionConfiguration {
    pub(super) fn initial(&mut self, session: &Value) {
        self.options = session
            .get("configOptions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        self.current_mode = session
            .pointer("/modes/currentModeId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.available_modes = session
            .pointer("/modes/availableModes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
    }
    pub(super) fn update(&mut self, update: &Value) -> Result<(), agent_client_protocol::Error> {
        match update.get("sessionUpdate").and_then(Value::as_str) {
            Some("config_option_update") => {
                self.options = update
                    .get("configOptions")
                    .and_then(Value::as_array)
                    .ok_or_else(agent_client_protocol::Error::invalid_params)?
                    .clone()
            }
            Some("current_mode_update") => {
                self.current_mode = Some(
                    update
                        .get("currentModeId")
                        .and_then(Value::as_str)
                        .ok_or_else(agent_client_protocol::Error::invalid_params)?
                        .into(),
                )
            }
            _ => {}
        }
        Ok(())
    }
    fn selections(&self, profile: &AcpProfile) -> Result<BTreeMap<String, String>, ClientError> {
        let mut result = profile.session.options.clone();
        for (category, value) in [
            ("model", &profile.session.model),
            ("mode", &profile.session.mode),
        ] {
            let Some(value) = value else { continue };
            if let Some(option) = self
                .options
                .iter()
                .find(|o| o.get("category").and_then(Value::as_str) == Some(category))
            {
                let id = option
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported(category))?;
                if result.get(id).is_some_and(|existing| existing != value) {
                    return Err(unsupported("conflicting selections"));
                }
                result.insert(id.into(), value.clone());
            } else if category != "mode" || !self.options.is_empty() {
                return Err(unsupported(category));
            }
        }
        Ok(result)
    }
    fn accepts(&self, id: &str, value: &str) -> bool {
        self.options
            .iter()
            .find(|o| o.get("id").and_then(Value::as_str) == Some(id))
            .is_some_and(|o| {
                o.get("type").and_then(Value::as_str) == Some("select")
                    && o.get("options")
                        .and_then(Value::as_array)
                        .is_some_and(|values| {
                            values.iter().any(|v| {
                                v.get("value").and_then(Value::as_str) == Some(value)
                                    || v.get("options").and_then(Value::as_array).is_some_and(
                                        |group| {
                                            group.iter().any(|v| {
                                                v.get("value").and_then(Value::as_str)
                                                    == Some(value)
                                            })
                                        },
                                    )
                            })
                        })
            })
    }
}
fn unsupported(selection: &str) -> ClientError {
    ClientError::Setup(format!(
        "explicit session {selection} is not advertised or was not applied"
    ))
}

pub(super) fn validate_transports(
    servers: &[McpServer],
    capabilities: &AgentCapabilities,
) -> Result<(), ClientError> {
    for server in servers {
        let supported = match server {
            McpServer::Stdio(_) => true,
            McpServer::Http(_) => capabilities.mcp_capabilities.http,
            McpServer::Sse(_) => capabilities.mcp_capabilities.sse,
            _ => false,
        };
        if !supported {
            return Err(ClientError::Setup(
                "required scoped MCP transport is not advertised by the agent".into(),
            ));
        }
    }
    Ok(())
}

pub(super) async fn apply(
    connection: &ConnectionTo<Agent>,
    profile: &AcpProfile,
    session: &str,
    state: &std::sync::Arc<std::sync::Mutex<SessionConfiguration>>,
) -> Result<(), ClientError> {
    let selections = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .selections(profile)?;
    for (id, value) in &selections {
        if !state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accepts(id, value)
        {
            return Err(unsupported("config option"));
        }
        let request = UntypedMessage::new(
            "session/set_config_option",
            json!({"sessionId":session,"configId":id,"value":value}),
        )
        .map_err(|_| unsupported("config option"))?;
        let state = state.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        // Commit response metadata inside ordered dispatch, before adjacent updates.
        connection
            .send_request(request)
            .on_receiving_result(async move |result| {
                let result = result.and_then(|result| {
                    let options = result
                        .get("configOptions")
                        .and_then(Value::as_array)
                        .ok_or_else(agent_client_protocol::Error::invalid_params)?;
                    state.lock().unwrap_or_else(|e| e.into_inner()).options = options.clone();
                    Ok(())
                });
                let _ = tx.send(result);
                Ok(())
            })
            .map_err(|_| unsupported("config option"))?;
        rx.await
            .map_err(|_| unsupported("config option"))?
            .map_err(|_| unsupported("config option"))?;
    }
    let legacy_mode = {
        let state = state.lock().unwrap_or_else(|e| e.into_inner());
        if state.options.is_empty() {
            profile.session.mode.clone()
        } else {
            None
        }
    };
    if let Some(mode) = legacy_mode {
        if !state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .available_modes
            .iter()
            .any(|m| m.get("id").and_then(Value::as_str) == Some(&mode))
        {
            return Err(unsupported("mode"));
        }
        let request = UntypedMessage::new(
            "session/set_mode",
            json!({"sessionId":session,"modeId":mode}),
        )
        .map_err(|_| unsupported("mode"))?;
        let state = state.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        connection
            .send_request(request)
            .on_receiving_result(async move |result| {
                if result.is_ok() {
                    state.lock().unwrap_or_else(|e| e.into_inner()).current_mode = Some(mode);
                }
                let _ = tx.send(result);
                Ok(())
            })
            .map_err(|_| unsupported("mode"))?;
        rx.await
            .map_err(|_| unsupported("mode"))?
            .map_err(|_| unsupported("mode"))?;
    }
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    if state.options.is_empty()
        && profile
            .session
            .mode
            .as_ref()
            .is_some_and(|mode| state.current_mode.as_ref() != Some(mode))
    {
        return Err(unsupported("mode"));
    }
    for (id, value) in selections {
        if !state.options.iter().any(|o| {
            o.get("id").and_then(Value::as_str) == Some(&id)
                && o.get("currentValue").and_then(Value::as_str) == Some(&value)
        }) {
            return Err(unsupported("config option"));
        }
    }
    Ok(())
}
