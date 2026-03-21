//! Downstream compile-time smoke helpers for public OpenSymphony contracts.

use chrono::{DateTime, Utc};
use opensymphony_domain::{Issue, OrchestratorSnapshot};
use opensymphony_orchestrator::{SchedulerConfig, SchedulerState};
use opensymphony_workflow::Workflow;

/// Exercise the M1 public API surface from a downstream crate.
pub fn public_api_smoke(
    workflow_markdown: &str,
    issue: Issue,
    now: DateTime<Utc>,
) -> (Workflow, OrchestratorSnapshot, String) {
    let workflow = Workflow::load_from_str(workflow_markdown).expect("workflow should parse");
    let mut scheduler = SchedulerState::new(SchedulerConfig::default());
    let claimed = scheduler.claim_candidate_batch(std::slice::from_ref(&issue));
    assert_eq!(
        claimed.len(),
        1,
        "issue should be claimable before start_run"
    );
    scheduler
        .start_run(
            issue.clone(),
            std::path::PathBuf::from(format!("/tmp/{}", issue.identifier)),
            None,
            now,
        )
        .expect("scheduler should accept the issue");
    let snapshot = scheduler.snapshot(now);
    let prompt = workflow
        .render_prompt(&issue, None)
        .expect("prompt should render");
    (workflow, snapshot, prompt)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn downstream_crate_can_build_against_public_interfaces() {
        let issue = Issue::new(
            "1",
            "OSYM-1",
            "Foundation",
            "In Progress",
            Utc.timestamp_opt(1, 0).single().unwrap(),
        );

        let (_workflow, snapshot, prompt) = public_api_smoke(
            "Hello {{ issue.identifier }}",
            issue,
            Utc.timestamp_opt(2, 0).single().unwrap(),
        );

        assert_eq!(snapshot.running.len(), 1);
        assert_eq!(prompt, "Hello OSYM-1");
    }
}
