use opensymphony_domain::OrchestratorSnapshot;

#[derive(Debug, Clone, Default)]
pub struct SnapshotStore {
    latest: Option<OrchestratorSnapshot>,
}

impl SnapshotStore {
    pub fn publish(&mut self, snapshot: OrchestratorSnapshot) {
        self.latest = Some(snapshot);
    }

    pub fn latest(&self) -> Option<&OrchestratorSnapshot> {
        self.latest.as_ref()
    }
}
