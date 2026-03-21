pub const CRATE_NAME: &str = "opensymphony-workflow";

pub fn placeholder_summary() -> &'static str {
    "WORKFLOW.md loading, front matter parsing, strict prompt rendering, openhands config schema, and env/path resolution helpers"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-workflow");
        assert!(placeholder_summary().contains("WORKFLOW.md"));
    }
}
