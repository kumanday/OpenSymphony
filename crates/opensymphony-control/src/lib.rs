pub const CRATE_NAME: &str = "opensymphony-control";

pub fn placeholder_summary() -> &'static str {
    "local control-plane HTTP API, update stream, snapshot publication, and serialization"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-control");
        assert!(placeholder_summary().contains("control-plane HTTP API"));
    }
}
