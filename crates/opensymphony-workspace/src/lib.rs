pub const CRATE_NAME: &str = "opensymphony-workspace";

pub fn placeholder_summary() -> &'static str {
    "workspace path resolution, sanitization, containment checks, hook runner, and issue/conversation manifest helpers"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-workspace");
        assert!(placeholder_summary().contains("workspace path resolution"));
    }
}
