pub const CRATE_NAME: &str = "opensymphony-orchestrator";

pub fn placeholder_summary() -> &'static str {
    "poll tick, runtime state machine, worker supervision, retry queue, cancellation/reconciliation, and snapshot derivation inputs"
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, placeholder_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-orchestrator");
        assert!(placeholder_summary().contains("runtime state machine"));
    }
}
