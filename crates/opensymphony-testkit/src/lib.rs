pub const CRATE_NAME: &str = "opensymphony-testkit";

pub fn placeholder_summary() -> &'static str {
    "fake OpenHands agent-server, fake Linear helpers, integration fixtures, and protocol contract assertions"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-testkit");
        assert!(placeholder_summary().contains("fake OpenHands agent-server"));
    }
}
