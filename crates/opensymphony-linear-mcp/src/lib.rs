mod client;
mod error;
mod model;
mod server;

pub use model::ToolDefinition;
pub use server::{LinearMcpServer, LinearMcpServerError, run_stdio_server, tool_definitions};

pub const CRATE_NAME: &str = "opensymphony-linear-mcp";
