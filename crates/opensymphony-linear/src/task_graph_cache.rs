use std::{collections::HashMap, time::Duration};

use chrono::{DateTime, Utc};

use crate::opensymphony_domain::TrackerIssue;

/// Cached Linear entity with sync metadata.
#[derive(Debug, Clone)]
pub struct CachedLinearEntity {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub state_kind: String,
    pub priority: Option<u8>,
    pub labels: Vec<String>,
    pub parent_id: Option<String>,
    pub project_milestone: Option<CachedMilestone>,
    pub blocked_by: Vec<CachedBlockerRef>,
    pub sub_issues: Vec<CachedIssueRef>,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub synced_at: DateTime<Utc>,
}

/// Cached milestone reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedMilestone {
    pub id: String,
    pub name: String,
}

/// Cached blocker reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBlockerRef {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state: String,
    pub is_terminal: bool,
}

/// Cached sub-issue reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedIssueRef {
    pub id: String,
    pub identifier: String,
    pub state: String,
}

/// Runtime overlay data for a cached entity.
#[derive(Debug, Clone)]
pub struct RuntimeOverlay {
    pub issue_id: String,
    pub eligible: bool,
    pub queued: bool,
    pub active_run_id: Option<String>,
    pub last_outcome: Option<String>,
    pub retry_count: u32,
    pub workspace_id: Option<String>,
    pub conversation_id: Option<String>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub validation_status: Option<String>,
    pub blocker_summary: Option<String>,
    pub synced_at: DateTime<Utc>,
}

/// Cache for Linear entities with sync timestamps.
#[derive(Debug, Clone)]
pub struct TaskGraphCache {
    entities: HashMap<String, CachedLinearEntity>,
    overlays: HashMap<String, RuntimeOverlay>,
    pub project_id: String,
    last_synced_at: Option<DateTime<Utc>>,
    ttl: Duration,
}

impl TaskGraphCache {
    pub fn new(project_id: impl Into<String>, ttl: Duration) -> Self {
        Self {
            entities: HashMap::new(),
            overlays: HashMap::new(),
            project_id: project_id.into(),
            last_synced_at: None,
            ttl,
        }
    }

    /// Insert or update Linear entities from a tracker poll.
    pub fn upsert_entities(&mut self, issues: Vec<TrackerIssue>) {
        let synced_at = Utc::now();
        for issue in issues {
            let mut entity: CachedLinearEntity = issue.into();
            entity.synced_at = synced_at;
            self.entities.insert(entity.id.clone(), entity);
        }
        self.last_synced_at = Some(synced_at);
    }

    /// Update the runtime overlay for a single issue.
    pub fn upsert_overlay(&mut self, overlay: RuntimeOverlay) {
        self.overlays.insert(overlay.issue_id.clone(), overlay);
    }

    /// Clear overlays for resolved issues.
    pub fn clear_overlay(&mut self, issue_id: &str) {
        self.overlays.remove(issue_id);
    }

    /// Get a cached entity by its Linear ID.
    pub fn get_entity(&self, id: &str) -> Option<&CachedLinearEntity> {
        self.entities.get(id)
    }

    /// Get a runtime overlay by `issue_id`.
    pub fn get_overlay(&self, id: &str) -> Option<&RuntimeOverlay> {
        self.overlays.get(id)
    }

    /// Return true if the entire cache is expired.
    pub fn is_expired(&self) -> bool {
        match self.last_synced_at {
            Some(synced) => match chrono::TimeDelta::from_std(self.ttl) {
                Ok(ttl) => Utc::now().signed_duration_since(synced) > ttl,
                Err(_) => true,
            },
            None => true,
        }
    }

    /// Number of cached entities.
    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    /// Number of cached overlays.
    pub fn overlay_count(&self) -> usize {
        self.overlays.len()
    }

    /// All entities (useful for iteration / snapshotting).
    pub fn entities(&self) -> impl Iterator<Item = (&String, &CachedLinearEntity)> {
        self.entities.iter()
    }

    /// All overlays.
    pub fn overlays(&self) -> impl Iterator<Item = (&String, &RuntimeOverlay)> {
        self.overlays.iter()
    }

    fn infer_state_kind(state: &str) -> String {
        match state.to_lowercase().as_str() {
            "done" | "completed" | "closed" => "completed",
            "canceled" | "cancelled" => "canceled",
            "in progress" | "started" => "started",
            "todo" | "unstarted" => "unstarted",
            "backlog" => "backlog",
            _ => "unknown",
        }
        .to_string()
    }
}

impl From<TrackerIssue> for CachedLinearEntity {
    fn from(issue: TrackerIssue) -> Self {
        Self {
            id: issue.id.clone(),
            identifier: issue.identifier.clone(),
            title: issue.title.clone(),
            state: issue.state.clone(),
            state_kind: TaskGraphCache::infer_state_kind(&issue.state),
            priority: issue.priority,
            labels: issue.labels,
            parent_id: issue.parent_id,
            project_milestone: issue.project_milestone.map(|m| CachedMilestone {
                id: m.id,
                name: m.name,
            }),
            blocked_by: issue
                .blocked_by
                .into_iter()
                .map(|b| {
                    let is_terminal = b.is_terminal();
                    CachedBlockerRef {
                        id: b.id,
                        identifier: b.identifier,
                        title: b.title,
                        state: b.state.name,
                        is_terminal,
                    }
                })
                .collect(),
            sub_issues: issue
                .sub_issues
                .into_iter()
                .map(|s| CachedIssueRef {
                    id: s.id,
                    identifier: s.identifier,
                    state: s.state,
                })
                .collect(),
            url: issue.url,
            created_at: issue.created_at,
            updated_at: issue.updated_at,
            synced_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_issue(id: &str, identifier: &str, state: &str) -> TrackerIssue {
        TrackerIssue {
            id: id.to_string(),
            identifier: identifier.to_string(),
            url: format!("https://linear.app/test/issue/{identifier}"),
            title: format!("Issue {identifier}"),
            description: None,
            priority: Some(1),
            state: state.to_string(),
            labels: vec!["backend".to_string()],
            parent_id: None,
            parent: None,
            project_milestone: Some(crate::opensymphony_domain::TrackerProjectMilestone {
                id: "ms-1".to_string(),
                name: "M1".to_string(),
            }),
            blocked_by: Vec::new(),
            sub_issues: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn cache_upsert_entities_tracks_sync_timestamp() {
        let mut cache = TaskGraphCache::new("default", Duration::from_secs(300));
        let issues = vec![build_test_issue("lin-1", "COE-1", "In Progress")];
        let synced_before = Utc::now();
        cache.upsert_entities(issues);
        let synced_after = Utc::now();

        assert_eq!(cache.entity_count(), 1);
        let entity = cache.get_entity("lin-1").expect("entity should exist");
        assert_eq!(entity.identifier, "COE-1");
        assert!(entity.synced_at >= synced_before);
        assert!(entity.synced_at <= synced_after);
        assert_eq!(cache.last_synced_at, Some(entity.synced_at));
    }

    #[test]
    fn cache_upsert_overlay_by_issue_id() {
        let mut cache = TaskGraphCache::new("default", Duration::from_secs(300));
        let overlay = RuntimeOverlay {
            issue_id: "lin-1".to_string(),
            eligible: true,
            queued: false,
            active_run_id: Some("run-1".to_string()),
            last_outcome: None,
            retry_count: 0,
            workspace_id: None,
            conversation_id: None,
            last_event_at: None,
            validation_status: None,
            blocker_summary: None,
            synced_at: Utc::now(),
        };
        cache.upsert_overlay(overlay);

        assert_eq!(cache.overlay_count(), 1);
        let result = cache.get_overlay("lin-1").expect("overlay should exist");
        assert!(result.eligible);
    }

    #[test]
    fn cache_clear_overlay_removes_entry() {
        let mut cache = TaskGraphCache::new("default", Duration::from_secs(300));
        cache.upsert_overlay(RuntimeOverlay {
            issue_id: "lin-1".to_string(),
            eligible: true,
            queued: false,
            active_run_id: Some("run-1".to_string()),
            last_outcome: None,
            retry_count: 0,
            workspace_id: None,
            conversation_id: None,
            last_event_at: None,
            validation_status: None,
            blocker_summary: None,
            synced_at: Utc::now(),
        });
        cache.clear_overlay("lin-1");
        assert_eq!(cache.overlay_count(), 0);
    }

    #[test]
    fn cache_is_expired_returns_true_when_ttl_passed() {
        let mut cache = TaskGraphCache::new("default", Duration::from_secs(1));
        cache.upsert_entities(vec![build_test_issue("lin-1", "COE-1", "Todo")]);
        assert!(!cache.is_expired());

        cache.last_synced_at = Some(Utc::now() - chrono::TimeDelta::seconds(2));
        assert!(cache.is_expired());
    }

    #[test]
    fn cache_is_expired_returns_true_when_never_synced() {
        let cache = TaskGraphCache::new("default", Duration::from_secs(300));
        assert!(cache.is_expired());
    }

    #[test]
    fn infer_state_kind_maps_known_states() {
        assert_eq!(TaskGraphCache::infer_state_kind("Done"), "completed");
        assert_eq!(TaskGraphCache::infer_state_kind("In Progress"), "started");
        assert_eq!(TaskGraphCache::infer_state_kind("Todo"), "unstarted");
        assert_eq!(TaskGraphCache::infer_state_kind("Backlog"), "backlog");
        assert_eq!(TaskGraphCache::infer_state_kind("Custom"), "unknown");
    }

    #[test]
    fn from_tracker_issue_converts_milestone_and_blockers() {
        let issue = build_test_issue("lin-1", "COE-1", "Done");
        let entity: CachedLinearEntity = issue.into();
        assert!(entity.project_milestone.is_some());
        let milestone = entity
            .project_milestone
            .expect("test issue should have milestone");
        assert_eq!(milestone.name, "M1");
        assert_eq!(entity.state_kind, "completed");
    }

    #[test]
    fn cache_entities_iterator_yields_all() {
        let mut cache = TaskGraphCache::new("default", Duration::from_secs(300));
        cache.upsert_entities(vec![
            build_test_issue("lin-1", "COE-1", "Todo"),
            build_test_issue("lin-2", "COE-2", "In Progress"),
        ]);
        let ids: Vec<_> = cache.entities().map(|(k, _)| k.as_str()).collect();
        assert!(ids.contains(&"lin-1"));
        assert!(ids.contains(&"lin-2"));
    }
}
