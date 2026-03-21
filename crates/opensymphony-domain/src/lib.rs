pub const CRATE_NAME: &str = "opensymphony-domain";

pub fn placeholder_summary() -> &'static str {
    "shared domain types, runtime enums, snapshot models, and config-independent constants"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-domain");
        assert!(placeholder_summary().contains("shared domain types"));
    }
}
