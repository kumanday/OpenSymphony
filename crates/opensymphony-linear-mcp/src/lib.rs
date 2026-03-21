pub const CRATE_NAME: &str = "opensymphony-linear-mcp";

pub fn placeholder_summary() -> &'static str {
    "stdio MCP server for agent-side Linear writes"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-linear-mcp");
        assert!(placeholder_summary().contains("Linear writes"));
    }
}
