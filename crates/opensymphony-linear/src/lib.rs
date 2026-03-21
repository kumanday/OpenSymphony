pub const CRATE_NAME: &str = "opensymphony-linear";

pub fn placeholder_summary() -> &'static str {
    "Linear GraphQL adapter, issue normalization, pagination, and tracker reconciliation helpers"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-linear");
        assert!(placeholder_summary().contains("Linear GraphQL adapter"));
    }
}
