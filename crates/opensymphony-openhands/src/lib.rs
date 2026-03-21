pub const CRATE_NAME: &str = "opensymphony-openhands";

pub fn placeholder_summary() -> &'static str {
    "local server supervisor, REST client, WebSocket event stream, event cache/state mirror, issue session runner, and protocol error mapping"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-openhands");
        assert!(placeholder_summary().contains("WebSocket event stream"));
    }
}
