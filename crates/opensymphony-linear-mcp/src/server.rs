use std::io;

use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use uuid::Uuid;

use crate::{
    client::{LinearMcpClient, TeamSelector},
    error::ToolFailure,
    model::ToolDefinition,
};

const SUPPORTED_PROTOCOL_VERSIONS: &[&str] =
    &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Debug, thiserror::Error)]
pub enum LinearMcpServerError {
    #[error(transparent)]
    Linear(#[from] crate::error::LinearMcpError),
    #[error("stdio transport failed: {0}")]
    Io(#[from] io::Error),
}

pub struct LinearMcpServer {
    client: LinearMcpClient,
    negotiated_protocol_version: Option<String>,
    ready_for_requests: bool,
}

impl LinearMcpServer {
    pub fn new(client: LinearMcpClient) -> Self {
        Self {
            client,
            negotiated_protocol_version: None,
            ready_for_requests: false,
        }
    }

    pub async fn serve<R, W>(&mut self, reader: R, mut writer: W) -> Result<(), io::Error>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut lines = reader.lines();
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }

            let responses = match serde_json::from_str::<Value>(&line) {
                Ok(value) => self.handle_message(value).await,
                Err(error) => vec![jsonrpc_error(
                    Value::Null,
                    -32700,
                    "Parse error",
                    Some(json!({ "detail": error.to_string() })),
                )],
            };

            for response in responses {
                let encoded = serde_json::to_vec(&response).map_err(io::Error::other)?;
                writer.write_all(&encoded).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
        }

        Ok(())
    }

    async fn handle_message(&mut self, value: Value) -> Vec<Value> {
        match value {
            Value::Array(items) => {
                if items.is_empty() {
                    return vec![jsonrpc_error(Value::Null, -32600, "Invalid Request", None)];
                }

                let mut responses = Vec::new();
                for item in items {
                    if let Some(response) = self.handle_single(item).await {
                        responses.push(response);
                    }
                }
                responses
            }
            other => self.handle_single(other).await.into_iter().collect(),
        }
    }

    async fn handle_single(&mut self, value: Value) -> Option<Value> {
        let request = match serde_json::from_value::<JsonRpcRequest>(value) {
            Ok(request) => request,
            Err(error) => {
                return Some(jsonrpc_error(
                    Value::Null,
                    -32600,
                    "Invalid Request",
                    Some(json!({ "detail": error.to_string() })),
                ))
            }
        };

        if request.jsonrpc != "2.0" {
            return Some(jsonrpc_error(
                request.id,
                -32600,
                "Invalid Request",
                Some(json!({ "detail": "jsonrpc must be `2.0`" })),
            ));
        }

        let is_notification = request.id.is_null();
        let response = match request.method.as_str() {
            "initialize" => self.handle_initialize(request.id, request.params).await,
            "notifications/initialized" => {
                self.ready_for_requests = true;
                None
            }
            "ping" => Some(jsonrpc_result(request.id, json!({}))),
            "tools/list" => {
                if let Some(error) = self.ensure_initialized(&request.id) {
                    Some(error)
                } else {
                    Some(jsonrpc_result(
                        request.id,
                        json!({
                            "tools": tool_definitions(),
                        }),
                    ))
                }
            }
            "tools/call" => self.handle_tool_call(request.id, request.params).await,
            method if method.starts_with("notifications/") => None,
            _ => Some(jsonrpc_error(request.id, -32601, "Method not found", None)),
        };

        if is_notification {
            None
        } else {
            response
        }
    }

    async fn handle_initialize(&mut self, id: Value, params: Value) -> Option<Value> {
        let params = match serde_json::from_value::<InitializeParams>(params) {
            Ok(params) => params,
            Err(error) => {
                return Some(jsonrpc_error(
                    id,
                    -32602,
                    "Invalid params",
                    Some(json!({ "detail": error.to_string() })),
                ))
            }
        };

        let negotiated = negotiate_protocol_version(&params.protocol_version);
        self.negotiated_protocol_version = Some(negotiated.to_string());
        self.ready_for_requests = false;

        Some(jsonrpc_result(
            id,
            json!({
                "protocolVersion": negotiated,
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": crate::CRATE_NAME,
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": "Use the Linear tools to fetch issue context, add comments, transition states, attach pull requests, and list valid workflow states.",
            }),
        ))
    }

    fn ensure_initialized(&self, id: &Value) -> Option<Value> {
        if self.negotiated_protocol_version.is_none() {
            return Some(jsonrpc_error(
                id.clone(),
                -32002,
                "Server not initialized",
                Some(json!({
                    "detail": "Call `initialize` before using the Linear MCP tools."
                })),
            ));
        }

        if !self.ready_for_requests {
            return Some(jsonrpc_error(
                id.clone(),
                -32002,
                "Server not initialized",
                Some(json!({
                    "detail": "Send `notifications/initialized` after a successful initialize response."
                })),
            ));
        }

        None
    }

    async fn handle_tool_call(&mut self, id: Value, params: Value) -> Option<Value> {
        if let Some(response) = self.ensure_initialized(&id) {
            return Some(response);
        }

        let params = match serde_json::from_value::<ToolCallParams>(params) {
            Ok(params) => params,
            Err(error) => {
                return Some(jsonrpc_error(
                    id,
                    -32602,
                    "Invalid params",
                    Some(json!({ "detail": error.to_string() })),
                ))
            }
        };

        let protocol = self.negotiated_protocol_version.as_deref();
        let result = match self
            .dispatch_tool_call(&params.name, params.arguments)
            .await
        {
            Ok(payload) => jsonrpc_result(id, call_tool_result(payload, false, protocol)),
            Err(error) => jsonrpc_result(
                id,
                call_tool_result(json!({ "error": error }), true, protocol),
            ),
        };

        Some(result)
    }

    async fn dispatch_tool_call(&self, name: &str, arguments: Value) -> Result<Value, ToolFailure> {
        match name {
            "linear_get_issue" => {
                let args = serde_json::from_value::<GetIssueArgs>(arguments)
                    .map_err(|error| ToolFailure::invalid_input(error.to_string()))?;
                let issue = self.client.resolve_issue(&args.issue).await?;
                Ok(json!({ "issue": issue }))
            }
            "linear_comment_issue" => {
                let args = serde_json::from_value::<CommentIssueArgs>(arguments)
                    .map_err(|error| ToolFailure::invalid_input(error.to_string()))?;
                let body = normalize_non_empty("body", &args.body)?;
                let issue = self.client.resolve_issue(&args.issue).await?;
                let comment = self.client.comment_issue(&issue.id, &body).await?;
                Ok(json!({ "issue": issue, "comment": comment }))
            }
            "linear_transition_issue" => {
                let args = serde_json::from_value::<TransitionIssueArgs>(arguments)
                    .map_err(|error| ToolFailure::invalid_input(error.to_string()))?;
                let requested_state = normalize_non_empty("state", &args.state)?;
                let issue = self.client.resolve_issue(&args.issue).await?;
                let (_, states) = self.client.workflow_states_for_issue(&issue).await?;
                let state = states
                    .into_iter()
                    .find(|candidate| candidate.name.eq_ignore_ascii_case(&requested_state))
                    .ok_or_else(|| ToolFailure {
                        code: "invalid_state_transition".to_string(),
                        message: format!(
                            "State `{requested_state}` is not valid for team `{}`.",
                            issue.team.key
                        ),
                    })?;
                let issue = self.client.transition_issue(&issue.id, &state.id).await?;
                Ok(json!({ "issue": issue }))
            }
            "linear_link_pr" => {
                let args = serde_json::from_value::<LinkPrArgs>(arguments)
                    .map_err(|error| ToolFailure::invalid_input(error.to_string()))?;
                let issue = self.client.resolve_issue(&args.issue).await?;
                let attachment = self
                    .client
                    .link_issue_url(&issue.id, &args.url, args.title.as_deref())
                    .await?;
                Ok(json!({ "issue": issue, "attachment": attachment }))
            }
            "linear_list_project_states" => {
                let args = serde_json::from_value::<ListProjectStatesArgs>(arguments)
                    .map_err(|error| ToolFailure::invalid_input(error.to_string()))?;
                let (team, states) = match (args.issue, args.team) {
                    (Some(issue), None) => {
                        let issue = self.client.resolve_issue(&issue).await?;
                        self.client.workflow_states_for_issue(&issue).await?
                    }
                    (None, Some(team)) => {
                        let selector = if Uuid::parse_str(team.trim()).is_ok() {
                            TeamSelector::Id(team)
                        } else {
                            TeamSelector::Key(team)
                        };
                        self.client.workflow_states_for_team(selector).await?
                    }
                    _ => {
                        return Err(ToolFailure::invalid_input(
                            "Provide exactly one of `issue` or `team`.",
                        ))
                    }
                };
                Ok(json!({ "team": team, "states": states }))
            }
            _ => Err(ToolFailure {
                code: "unknown_tool".to_string(),
                message: format!("Unknown tool `{name}`."),
            }),
        }
    }
}

pub async fn run_stdio_server() -> Result<(), LinearMcpServerError> {
    let client = LinearMcpClient::from_env()?;
    let reader = BufReader::new(tokio::io::stdin());
    let writer = tokio::io::stdout();
    let mut server = LinearMcpServer::new(client);
    server.serve(reader, writer).await?;
    Ok(())
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "linear_get_issue",
            description: "Fetch a Linear issue by UUID or identifier and return a normalized issue snapshot.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "issue": {
                        "type": "string",
                        "description": "Linear issue UUID or identifier such as `COE-267`."
                    }
                },
                "required": ["issue"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "linear_comment_issue",
            description: "Add a comment to a Linear issue and return the created comment with the resolved issue snapshot.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "issue": {
                        "type": "string",
                        "description": "Linear issue UUID or identifier such as `COE-267`."
                    },
                    "body": {
                        "type": "string",
                        "description": "Markdown comment body to append to the issue."
                    }
                },
                "required": ["issue", "body"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "linear_transition_issue",
            description: "Move a Linear issue to a named workflow state for that issue's team.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "issue": {
                        "type": "string",
                        "description": "Linear issue UUID or identifier such as `COE-267`."
                    },
                    "state": {
                        "type": "string",
                        "description": "Exact workflow state name, for example `In Progress` or `Human Review`."
                    }
                },
                "required": ["issue", "state"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "linear_link_pr",
            description: "Attach a GitHub pull request URL or related link to a Linear issue.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "issue": {
                        "type": "string",
                        "description": "Linear issue UUID or identifier such as `COE-267`."
                    },
                    "url": {
                        "type": "string",
                        "format": "uri",
                        "description": "GitHub pull request URL or another related link to attach."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional attachment title override."
                    }
                },
                "required": ["issue", "url"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "linear_list_project_states",
            description: "List valid workflow state names for a team, addressed either by issue or by team key/UUID.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "issue": {
                        "type": "string",
                        "description": "Linear issue UUID or identifier used to derive the team."
                    },
                    "team": {
                        "type": "string",
                        "description": "Linear team UUID or team key such as `COE`."
                    }
                },
                "oneOf": [
                    { "required": ["issue"] },
                    { "required": ["team"] }
                ],
                "additionalProperties": false
            }),
        },
    ]
}

fn negotiate_protocol_version(requested: &str) -> &'static str {
    SUPPORTED_PROTOCOL_VERSIONS
        .iter()
        .copied()
        .find(|candidate| *candidate == requested)
        .unwrap_or(SUPPORTED_PROTOCOL_VERSIONS[0])
}

fn normalize_non_empty(field_name: &str, value: &str) -> Result<String, ToolFailure> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err(ToolFailure::invalid_input(format!(
            "{field_name} must not be blank."
        )));
    }
    Ok(normalized.to_string())
}

fn call_tool_result(payload: Value, is_error: bool, protocol: Option<&str>) -> Value {
    let mut result = Map::new();
    result.insert(
        "content".to_string(),
        json!([{
            "type": "text",
            "text": serde_json::to_string(&payload).unwrap_or_else(|_| "{\"error\":\"failed to encode tool payload\"}".to_string())
        }]),
    );
    result.insert("isError".to_string(), Value::Bool(is_error));
    if protocol_supports_structured_content(protocol) {
        result.insert("structuredContent".to_string(), payload);
    }
    Value::Object(result)
}

fn protocol_supports_structured_content(protocol: Option<&str>) -> bool {
    matches!(protocol, Some("2025-06-18" | "2025-11-25"))
}

fn jsonrpc_result(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn jsonrpc_error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".to_string(), Value::from(code));
    error.insert("message".to_string(), Value::String(message.to_string()));
    if let Some(data) = data {
        error.insert("data".to_string(), data);
    }

    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": Value::Object(error),
    })
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    protocol_version: String,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default = "empty_object")]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GetIssueArgs {
    issue: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentIssueArgs {
    issue: String,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionIssueArgs {
    issue: String,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LinkPrArgs {
    issue: String,
    url: String,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListProjectStatesArgs {
    issue: Option<String>,
    team: Option<String>,
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use crate::server::{negotiate_protocol_version, tool_definitions};

    #[test]
    fn tool_definitions_match_documented_surface() {
        let tools = tool_definitions();
        let names = tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "linear_get_issue",
                "linear_comment_issue",
                "linear_transition_issue",
                "linear_link_pr",
                "linear_list_project_states",
            ]
        );
    }

    #[test]
    fn list_project_states_schema_requires_issue_or_team() {
        let tool = tool_definitions()
            .into_iter()
            .find(|tool| tool.name == "linear_list_project_states")
            .expect("tool should be present");

        assert_eq!(tool.input_schema["oneOf"].as_array().unwrap().len(), 2);
        assert_eq!(
            tool.input_schema["additionalProperties"],
            serde_json::json!(false)
        );
    }

    #[test]
    fn protocol_negotiation_preserves_supported_versions() {
        assert_eq!(negotiate_protocol_version("2025-11-25"), "2025-11-25");
        assert_eq!(negotiate_protocol_version("2024-11-05"), "2024-11-05");
        assert_eq!(negotiate_protocol_version("1999-01-01"), "2025-11-25");
    }
}
