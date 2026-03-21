use opensymphony_domain::OrchestratorSnapshot;

pub fn render_summary(snapshot: &OrchestratorSnapshot) -> String {
    format!(
        "running={} queued={}",
        snapshot.running_issue_ids.len(),
        snapshot.queued_retry_ids.len()
    )
}
