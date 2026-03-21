pub const CRATE_NAME: &str = "opensymphony-tui";

pub fn placeholder_summary() -> &'static str {
    "FrankenTUI operator app, control-plane client, reducers, and rendering"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-tui");
        assert!(placeholder_summary().contains("FrankenTUI operator app"));
    }
}
