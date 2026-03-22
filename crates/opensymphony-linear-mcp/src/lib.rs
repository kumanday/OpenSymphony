mod client;
mod error;
mod model;
mod server;

pub use model::ToolDefinition;
pub use server::{run_stdio_server, tool_definitions, LinearMcpServer, LinearMcpServerError};

pub const CRATE_NAME: &str = "opensymphony-linear-mcp";
