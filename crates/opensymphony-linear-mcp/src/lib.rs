use opensymphony_domain::Issue;
use opensymphony_linear::{LinearError, LinearWriteOperations};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LinearMcpError {
    #[error("stdio framing error: {0}")]
    Framing(String),
    #[error("JSON-RPC error: {0}")]
    JsonRpc(String),
    #[error("linear backend error: {0}")]
    Backend(String),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("IO error: {0}")]
    Io(String),
}

#[derive(Debug, Clone)]
pub struct McpServer<T> {
    backend: T,
}

impl<T> McpServer<T>
where
    T: LinearWriteOperations,
{
    pub fn new(backend: T) -> Self {
        Self { backend }
    }

    pub fn list_tools(&self) -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "linear_get_issue".to_string(),
                description: "Fetch an issue by Linear identifier or ID.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["query"],
                    "properties": {
                        "query": { "type": "string" }
                    }
                }),
            },
            ToolDefinition {
                name: "linear_comment_issue".to_string(),
                description: "Add a comment to a Linear issue.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["issue_id", "body"],
                    "properties": {
                        "issue_id": { "type": "string" },
                        "body": { "type": "string" }
                    }
                }),
            },
            ToolDefinition {
                name: "linear_transition_issue".to_string(),
                description: "Move a Linear issue to a named state.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["issue_id", "state"],
                    "properties": {
                        "issue_id": { "type": "string" },
                        "state": { "type": "string" }
                    }
                }),
            },
            ToolDefinition {
                name: "linear_link_pr".to_string(),
                description: "Attach a pull request link to a Linear issue.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["issue_id", "url"],
                    "properties": {
                        "issue_id": { "type": "string" },
                        "url": { "type": "string" },
                        "title": { "type": "string" }
                    }
                }),
            },
            ToolDefinition {
                name: "linear_list_project_states".to_string(),
                description: "List valid project state names for safer transitions.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "required": ["project_slug"],
                    "properties": {
                        "project_slug": { "type": "string" }
                    }
                }),
            },
        ]
    }

    fn handle_json_rpc(&self, request: JsonRpcRequest) -> Option<Value> {
        let id = request.id.unwrap_or(Value::Null);
        match request.method.as_str() {
            "initialize" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "opensymphony-linear-mcp",
                        "version": "0.1.0"
                    }
                }
            })),
            "notifications/initialized" => None,
            "tools/list" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": self.list_tools() }
            })),
            "tools/call" => Some(
                match (|| -> Result<Value, LinearMcpError> {
                    let params = request
                        .params
                        .ok_or_else(|| LinearMcpError::JsonRpc("missing params".to_string()))?;
                    let tool_name = params
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| LinearMcpError::JsonRpc("missing tool name".to_string()))?;
                    let arguments = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    self.call_tool(tool_name, arguments)
                })() {
                    Ok(result) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }),
                    Err(error) => json_rpc_error_response(id, error),
                },
            ),
            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("unsupported method `{}`", request.method)
                }
            })),
        }
    }

    pub fn call_tool(&self, tool_name: &str, arguments: Value) -> Result<Value, LinearMcpError> {
        let issue = match tool_name {
            "linear_get_issue" => {
                let input: GetIssueInput = parse_arguments(arguments)?;
                ToolResult::issue(self.backend.get_issue(&input.query)?)
            }
            "linear_comment_issue" => {
                let input: CommentIssueInput = parse_arguments(arguments)?;
                ToolResult::issue(self.backend.comment_issue(&input.issue_id, &input.body)?)
            }
            "linear_transition_issue" => {
                let input: TransitionIssueInput = parse_arguments(arguments)?;
                ToolResult::issue(
                    self.backend
                        .transition_issue(&input.issue_id, &input.state)?,
                )
            }
            "linear_link_pr" => {
                let input: LinkPrInput = parse_arguments(arguments)?;
                ToolResult::issue(self.backend.link_pr(
                    &input.issue_id,
                    &input.url,
                    input.title.as_deref(),
                )?)
            }
            "linear_list_project_states" => {
                let input: ListProjectStatesInput = parse_arguments(arguments)?;
                ToolResult::states(self.backend.list_project_states(&input.project_slug)?)
            }
            _ => {
                return Err(LinearMcpError::JsonRpc(format!(
                    "unsupported tool `{tool_name}`"
                )))
            }
        };

        Ok(json!({
            "content": [{
                "type": "text",
                "text": issue.text
            }],
            "isError": false
        }))
    }
}

pub fn serve_stdio<T, R, W>(
    server: &McpServer<T>,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), LinearMcpError>
where
    T: LinearWriteOperations,
    R: BufRead,
    W: Write,
{
    while let Some(message) = read_frame(reader)? {
        let response = match serde_json::from_slice::<JsonRpcRequest>(&message) {
            Ok(request) => server.handle_json_rpc(request),
            Err(error) => Some(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": {
                    "code": -32700,
                    "message": format!("invalid JSON-RPC payload: {error}")
                }
            })),
        };
        if let Some(response) = response {
            write_frame(writer, &response)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct GetIssueInput {
    query: String,
}

#[derive(Debug, Deserialize)]
struct CommentIssueInput {
    issue_id: String,
    body: String,
}

#[derive(Debug, Deserialize)]
struct TransitionIssueInput {
    issue_id: String,
    state: String,
}

#[derive(Debug, Deserialize)]
struct LinkPrInput {
    issue_id: String,
    url: String,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListProjectStatesInput {
    project_slug: String,
}

struct ToolResult {
    text: String,
}

impl ToolResult {
    fn issue(issue: Issue) -> Self {
        Self {
            text: serde_json::to_string(&json!({
                "id": issue.id,
                "identifier": issue.identifier,
                "title": issue.title,
                "state": issue.state,
                "priority": issue.priority,
                "labels": issue.labels,
            }))
            .expect("issue summary should serialize"),
        }
    }

    fn states(states: Vec<String>) -> Self {
        Self {
            text: serde_json::to_string(&json!({ "states": states }))
                .expect("states should serialize"),
        }
    }
}

fn parse_arguments<T>(arguments: Value) -> Result<T, LinearMcpError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(arguments).map_err(|error| LinearMcpError::JsonRpc(error.to_string()))
}

fn read_frame<R: BufRead>(reader: &mut R) -> Result<Option<Vec<u8>>, LinearMcpError> {
    let mut content_length = None::<usize>;
    let mut header_line = String::new();

    loop {
        header_line.clear();
        let read = reader
            .read_line(&mut header_line)
            .map_err(|error| LinearMcpError::Io(error.to_string()))?;
        if read == 0 {
            return Ok(None);
        }

        let trimmed = header_line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|error| LinearMcpError::Framing(error.to_string()))?;
            content_length = Some(parsed);
        }
    }

    let content_length = content_length
        .ok_or_else(|| LinearMcpError::Framing("missing Content-Length".to_string()))?;
    let mut body = vec![0; content_length];
    reader
        .read_exact(&mut body)
        .map_err(|error| LinearMcpError::Io(error.to_string()))?;
    Ok(Some(body))
}

fn write_frame<W: Write>(writer: &mut W, message: &Value) -> Result<(), LinearMcpError> {
    let body = serde_json::to_vec(message)
        .map_err(|error| LinearMcpError::Serialization(error.to_string()))?;
    writer
        .write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .map_err(|error| LinearMcpError::Io(error.to_string()))?;
    writer
        .write_all(&body)
        .map_err(|error| LinearMcpError::Io(error.to_string()))?;
    writer
        .flush()
        .map_err(|error| LinearMcpError::Io(error.to_string()))
}

impl From<LinearError> for LinearMcpError {
    fn from(value: LinearError) -> Self {
        Self::Backend(value.to_string())
    }
}

fn json_rpc_error_response(id: Value, error: LinearMcpError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": json_rpc_error_code(&error),
            "message": json_rpc_error_message(&error)
        }
    })
}

fn json_rpc_error_code(error: &LinearMcpError) -> i32 {
    match error {
        LinearMcpError::JsonRpc(_) => -32602,
        LinearMcpError::Backend(_) => -32000,
        LinearMcpError::Framing(_) | LinearMcpError::Serialization(_) | LinearMcpError::Io(_) => {
            -32603
        }
    }
}

fn json_rpc_error_message(error: &LinearMcpError) -> String {
    match error {
        LinearMcpError::Framing(message)
        | LinearMcpError::JsonRpc(message)
        | LinearMcpError::Backend(message)
        | LinearMcpError::Serialization(message)
        | LinearMcpError::Io(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use opensymphony_testkit::{make_issue, MemoryTracker};
    use std::io::{BufReader, Cursor};

    fn server() -> McpServer<MemoryTracker> {
        let issue = make_issue(
            "1",
            "ABC-1",
            "Todo",
            Some(1),
            chrono::Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap(),
        );
        McpServer::new(MemoryTracker::new(
            vec![issue],
            vec!["Todo".to_string(), "In Progress".to_string()],
            vec!["Done".to_string()],
            vec![
                "Todo".to_string(),
                "In Progress".to_string(),
                "Done".to_string(),
            ],
        ))
    }

    fn frame(message: Value) -> Vec<u8> {
        let body = serde_json::to_vec(&message).expect("request should serialize");
        let mut framed = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        framed.extend(body);
        framed
    }

    fn read_all_responses(bytes: &[u8]) -> Vec<Value> {
        let mut reader = BufReader::new(Cursor::new(bytes.to_vec()));
        let mut responses = Vec::new();
        while let Some(body) = read_frame(&mut reader).expect("response frame should parse") {
            responses.push(serde_json::from_slice(&body).expect("response should be valid json"));
        }
        responses
    }

    #[test]
    fn lists_expected_tool_schemas() {
        let tools = server().list_tools();
        let names = tools.into_iter().map(|tool| tool.name).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "linear_get_issue",
                "linear_comment_issue",
                "linear_transition_issue",
                "linear_link_pr",
                "linear_list_project_states"
            ]
        );
    }

    #[test]
    fn handles_stdio_initialize_and_tool_listing() {
        let input = [
            frame(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} })),
            frame(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} })),
        ]
        .concat();
        let mut output = Vec::new();
        serve_stdio(
            &server(),
            &mut BufReader::new(Cursor::new(input)),
            &mut output,
        )
        .expect("stdio server should succeed");

        let responses = read_all_responses(&output);
        let first = &responses[0];
        assert_eq!(
            first["result"]["serverInfo"]["name"],
            "opensymphony-linear-mcp"
        );
        assert_eq!(
            responses[1]["result"]["tools"][0]["name"],
            "linear_get_issue"
        );
    }

    #[test]
    fn calls_tools_over_stdio() {
        let input = frame(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "linear_transition_issue",
                "arguments": {
                    "issue_id": "1",
                    "state": "In Progress"
                }
            }
        }));

        let mut output = Vec::new();
        serve_stdio(
            &server(),
            &mut BufReader::new(Cursor::new(input)),
            &mut output,
        )
        .expect("stdio call should succeed");

        let responses = read_all_responses(&output);
        let response = &responses[0];
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool response should be text");
        assert!(text.contains("In Progress"));
    }

    #[test]
    fn returns_json_rpc_errors_without_terminating_stdio() {
        let input = [
            frame(json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "linear_transition_issue",
                    "arguments": {
                        "issue_id": "1",
                        "state": "Unknown"
                    }
                }
            })),
            frame(json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {
                    "name": "linear_get_issue",
                    "arguments": {
                        "query": "1"
                    }
                }
            })),
        ]
        .concat();

        let mut output = Vec::new();
        serve_stdio(
            &server(),
            &mut BufReader::new(Cursor::new(input)),
            &mut output,
        )
        .expect("stdio server should survive backend tool errors");

        let responses = read_all_responses(&output);
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32000);
        assert!(responses[0]["error"]["message"]
            .as_str()
            .expect("error message should be text")
            .contains("Unknown"));
        assert_eq!(responses[1]["result"]["content"][0]["type"], "text");
    }
}
