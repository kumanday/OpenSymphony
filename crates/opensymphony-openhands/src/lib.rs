mod supervisor;
mod tooling;

pub use supervisor::{
    ExternalServerConfig, LaunchOwnership, LocalServerSupervisor, ProbeConfig, ServerMode,
    ServerState, ServerStatus, SupervisedServerConfig, SupervisorConfig, SupervisorError,
};
pub use tooling::{
    LocalServerTooling, LocalToolingError, LocalToolingLayout, PinStatus, ResolvedLaunch,
    ToolingMetadata,
};

pub const CRATE_NAME: &str = "opensymphony-openhands";

pub fn crate_summary() -> &'static str {
    "local server supervisor, repo-local tooling resolution, conservative readiness probes, doctor diagnostics, REST client, WebSocket event stream, event cache/state mirror, issue session runner, and protocol error mapping"
}

pub fn placeholder_summary() -> &'static str {
    crate_summary()
}

#[cfg(test)]
mod tests {
    use super::{CRATE_NAME, crate_summary};

    #[test]
    fn reports_its_boundary() {
        assert_eq!(CRATE_NAME, "opensymphony-openhands");
        assert!(crate_summary().contains("local server supervisor"));
    }
}
