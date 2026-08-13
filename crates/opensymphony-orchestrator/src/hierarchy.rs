use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, IssueId, IssueIdentifier, TrackerIssue, TrackerIssueRef,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const HIERARCHY_STATE_SCHEMA_VERSION: u32 = 1;
const MAX_INACTIVE_LEASE_HISTORY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchyChildEdge {
    pub child_id: IssueId,
    pub child_identifier: IssueIdentifier,
    pub required: bool,
}

impl From<&TrackerIssueRef> for HierarchyChildEdge {
    fn from(child: &TrackerIssueRef) -> Self {
        Self {
            child_id: IssueId::new(child.id.clone()).expect("tracker child ids are validated"),
            child_identifier: IssueIdentifier::new(child.identifier.clone())
                .expect("tracker child identifiers are validated"),
            required: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HierarchyBlockedReason {
    HierarchyChanged,
    MissingMergeEvidence,
    MissingTargetCommit,
    MissingCheckoutEvidence,
    UnresolvedFailure(String),
    StaleGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HierarchyReconciliation {
    Unchanged,
    GenerationAdvanced { generation: u64 },
    BlockedForReplanning { generation: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HierarchySnapshot {
    pub parent_id: IssueId,
    pub generation: u64,
    pub required_child_edges: Vec<HierarchyChildEdge>,
    pub frozen: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<HierarchyBlockedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_intent_generation: Option<u64>,
}

impl HierarchySnapshot {
    pub fn new(parent: &TrackerIssue) -> Self {
        Self::new_with_canceled_states(parent, &[])
    }

    pub fn new_with_canceled_states(parent: &TrackerIssue, canceled_states: &[String]) -> Self {
        let mut snapshot = Self {
            parent_id: IssueId::new(parent.id.clone()).expect("tracker ids are validated"),
            generation: 1,
            required_child_edges: Vec::new(),
            frozen: false,
            blocked_reason: None,
            dispatched_generation: None,
            dispatch_intent_generation: None,
        };
        snapshot.required_child_edges = child_edges(&parent.sub_issues, canceled_states);
        snapshot
    }

    pub fn reconcile(&mut self, children: &[TrackerIssueRef]) -> HierarchyReconciliation {
        self.reconcile_with_canceled_states(children, &[])
    }

    pub fn reconcile_with_canceled_states(
        &mut self,
        children: &[TrackerIssueRef],
        canceled_states: &[String],
    ) -> HierarchyReconciliation {
        let next_edges = child_edges(children, canceled_states);
        if next_edges == self.required_child_edges {
            return HierarchyReconciliation::Unchanged;
        }

        let current_scope = self
            .required_child_edges
            .iter()
            .map(|edge| (&edge.child_id, edge.required))
            .collect::<BTreeSet<_>>();
        let next_scope = next_edges
            .iter()
            .map(|edge| (&edge.child_id, edge.required))
            .collect::<BTreeSet<_>>();
        if current_scope == next_scope {
            self.required_child_edges = next_edges;
            return HierarchyReconciliation::Unchanged;
        }

        self.generation = self.generation.saturating_add(1);
        self.required_child_edges = next_edges;
        self.dispatched_generation = None;
        self.dispatch_intent_generation = None;
        if self.frozen {
            self.blocked_reason = Some(HierarchyBlockedReason::HierarchyChanged);
            HierarchyReconciliation::BlockedForReplanning {
                generation: self.generation,
            }
        } else {
            self.blocked_reason = None;
            HierarchyReconciliation::GenerationAdvanced {
                generation: self.generation,
            }
        }
    }

    pub fn freeze(&mut self) -> Result<(), HierarchyBlockedReason> {
        if self.blocked_reason.is_some() {
            return Err(self.blocked_reason.clone().expect("checked above"));
        }
        self.frozen = true;
        Ok(())
    }

    pub fn accepts_event(&self, generation: u64) -> bool {
        generation == self.generation && self.blocked_reason.is_none()
    }

    pub fn replan(&mut self) {
        self.frozen = false;
        self.blocked_reason = None;
        self.dispatched_generation = None;
        self.dispatch_intent_generation = None;
    }

    pub fn mark_dispatch_intent(&mut self) {
        self.dispatch_intent_generation = Some(self.generation);
    }

    pub fn clear_dispatch_intent(&mut self) {
        self.dispatch_intent_generation = None;
    }

    pub fn dispatch_intended(&self) -> bool {
        self.dispatch_intent_generation == Some(self.generation)
    }

    pub fn mark_dispatched(&mut self) {
        self.dispatched_generation = Some(self.generation);
        self.dispatch_intent_generation = None;
    }

    pub fn dispatch_claimed(&self) -> bool {
        self.dispatched_generation == Some(self.generation)
    }
}

fn child_edges(
    children: &[TrackerIssueRef],
    canceled_states: &[String],
) -> Vec<HierarchyChildEdge> {
    let mut edges = children
        .iter()
        .map(|child| HierarchyChildEdge {
            child_id: IssueId::new(child.id.clone()).expect("tracker child ids are validated"),
            child_identifier: IssueIdentifier::new(child.identifier.clone())
                .expect("tracker child identifiers are validated"),
            required: !canceled_states.iter().any(|configured_state| {
                configured_state.to_ascii_lowercase().contains("cancel")
                    && configured_state
                        .trim()
                        .eq_ignore_ascii_case(child.state.trim())
            }),
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| left.child_id.cmp(&right.child_id));
    edges
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LeaseResource {
    pub issue_id: IssueId,
    pub repository_id: CanonicalRepositoryId,
    pub checkout_generation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LeaseOwner {
    pub id: String,
    pub kind: String,
}

impl LeaseOwner {
    pub fn leaf_worker(issue_id: &IssueId) -> Self {
        Self {
            id: format!("leaf-worker:{issue_id}"),
            kind: "leaf_worker".to_owned(),
        }
    }

    pub fn ancestor(parent_id: &IssueId) -> Self {
        Self {
            id: format!("ancestor-integration:{parent_id}"),
            kind: "ancestor_integration".to_owned(),
        }
    }

    pub fn review(child_id: &IssueId) -> Self {
        Self {
            id: format!("review:{child_id}"),
            kind: "review".to_owned(),
        }
    }

    pub fn review_for_parent(parent_id: &IssueId, child_id: &IssueId) -> Self {
        Self {
            id: format!("review:{parent_id}:{child_id}"),
            kind: "review".to_owned(),
        }
    }

    pub fn repair(parent_id: &IssueId) -> Self {
        Self {
            id: format!("repair:{parent_id}"),
            kind: "repair".to_owned(),
        }
    }

    pub fn diagnostic(operator_id: impl Into<String>) -> Self {
        Self {
            id: format!("diagnostic:{}", operator_id.into()),
            kind: "diagnostic_hold".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseKind {
    LeafWorker,
    Review,
    AncestorIntegration,
    Repair,
    DiagnosticHold,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub kind: LeaseKind,
    pub resource: LeaseResource,
    pub owner: LeaseOwner,
    pub hierarchy_generation: u64,
    pub acquired_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub released_at: Option<u64>,
}

impl LeaseRecord {
    pub fn active(&self) -> bool {
        self.active_at(current_epoch_millis())
    }

    pub fn active_at(&self, now: u64) -> bool {
        self.released_at.is_none() && self.expires_at.is_none_or(|expires_at| expires_at > now)
    }
}

fn current_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().min(u128::from(u64::MAX)) as u64
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaseError {
    #[error("lease resource {resource:?} is already owned by {owner}")]
    Conflict {
        resource: LeaseResource,
        owner: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildEligibilityEvidence {
    pub child_id: IssueId,
    pub hierarchy_generation: u64,
    pub orchestrator_terminal: bool,
    pub provider_merge_confirmed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_result_commit: Option<String>,
    /// Direct or descendant merge commits recorded by the provider. The
    /// singular field remains the compatibility projection for older state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge_result_commits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_repository_id: Option<CanonicalRepositoryId>,
    /// Repository identities for every descendant merge commit. The
    /// singular field remains the compatibility projection for direct child
    /// evidence, while nested parents may contribute several repositories.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub merge_repository_ids: Vec<CanonicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<LeaseResource>,
    /// Repository-neutral child parents contribute the retained descendant
    /// generations held by their own ancestor edge. Keep the legacy singular
    /// field for direct leaf evidence and use this collection for subtrees.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<LeaseResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unresolved_failure: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParentEligibilityEvidence {
    pub hierarchy_generation: u64,
    pub children: Vec<ChildEligibilityEvidence>,
}

impl ParentEligibilityEvidence {
    pub fn tracker_only(parent: &TrackerIssue, generation: u64) -> Self {
        Self {
            hierarchy_generation: generation,
            children: parent
                .sub_issues
                .iter()
                .map(|child| ChildEligibilityEvidence {
                    child_id: IssueId::new(child.id.clone())
                        .expect("tracker child ids are validated"),
                    hierarchy_generation: generation,
                    orchestrator_terminal: false,
                    provider_merge_confirmed: false,
                    merge_result_commit: None,
                    merge_result_commits: Vec::new(),
                    merge_repository_id: None,
                    merge_repository_ids: Vec::new(),
                    resource: None,
                    resources: Vec::new(),
                    unresolved_failure: None,
                })
                .collect(),
        }
    }

    pub fn tracker_only_for_snapshot(snapshot: &HierarchySnapshot) -> Self {
        Self {
            hierarchy_generation: snapshot.generation,
            children: snapshot
                .required_child_edges
                .iter()
                .map(|edge| ChildEligibilityEvidence {
                    child_id: edge.child_id.clone(),
                    hierarchy_generation: snapshot.generation,
                    orchestrator_terminal: false,
                    provider_merge_confirmed: false,
                    merge_result_commit: None,
                    merge_result_commits: Vec::new(),
                    merge_repository_id: None,
                    merge_repository_ids: Vec::new(),
                    resource: None,
                    resources: Vec::new(),
                    unresolved_failure: None,
                })
                .collect(),
        }
    }

    pub fn eligible_for(&self, snapshot: &HierarchySnapshot) -> Result<(), HierarchyBlockedReason> {
        if snapshot.blocked_reason.is_some() || self.hierarchy_generation != snapshot.generation {
            return Err(HierarchyBlockedReason::StaleGeneration);
        }
        let expected = snapshot
            .required_child_edges
            .iter()
            .filter(|edge| edge.required)
            .map(|edge| edge.child_id.clone())
            .collect::<BTreeSet<_>>();
        let actual = self
            .children
            .iter()
            .map(|child| child.child_id.clone())
            .collect::<BTreeSet<_>>();
        if expected != actual {
            return Err(HierarchyBlockedReason::HierarchyChanged);
        }
        for child in &self.children {
            if child.hierarchy_generation != snapshot.generation {
                return Err(HierarchyBlockedReason::StaleGeneration);
            }
            if !child.orchestrator_terminal {
                return Err(HierarchyBlockedReason::UnresolvedFailure(
                    "child orchestrator outcome is not terminal".to_owned(),
                ));
            }
            if !child.provider_merge_confirmed {
                return Err(HierarchyBlockedReason::MissingMergeEvidence);
            }
            if child
                .merge_result_commit
                .as_deref()
                .is_none_or(|commit| commit.trim().is_empty())
                && child.merge_result_commits.is_empty()
            {
                return Err(HierarchyBlockedReason::MissingTargetCommit);
            }
            let resources = child.resources();
            let merge_repository_ids = if child.merge_repository_ids.is_empty() {
                child.merge_repository_id.iter().collect::<Vec<_>>()
            } else {
                child.merge_repository_ids.iter().collect::<Vec<_>>()
            };
            if merge_repository_ids.iter().any(|merge_repository| {
                !resources
                    .iter()
                    .any(|resource| &resource.repository_id == *merge_repository)
            }) {
                return Err(HierarchyBlockedReason::MissingCheckoutEvidence);
            }
            if resources.is_empty() {
                return Err(HierarchyBlockedReason::MissingCheckoutEvidence);
            }
            if child.resources.is_empty()
                && child
                    .resource
                    .as_ref()
                    .is_some_and(|resource| resource.issue_id != child.child_id)
            {
                return Err(HierarchyBlockedReason::MissingCheckoutEvidence);
            }
            if let Some(failure) = &child.unresolved_failure {
                return Err(HierarchyBlockedReason::UnresolvedFailure(failure.clone()));
            }
        }
        Ok(())
    }

    pub fn integration_leases(&self, parent_id: &IssueId, now: u64) -> Vec<LeaseRecord> {
        let owner = LeaseOwner::ancestor(parent_id);
        self.children
            .iter()
            .flat_map(|child| child.resources().into_iter().cloned())
            .map(|resource| LeaseRecord {
                kind: LeaseKind::AncestorIntegration,
                resource,
                owner: owner.clone(),
                hierarchy_generation: self.hierarchy_generation,
                acquired_at: now,
                expires_at: None,
                released_at: None,
            })
            .collect()
    }
}

impl ChildEligibilityEvidence {
    pub fn resources(&self) -> Vec<&LeaseResource> {
        if self.resources.is_empty() {
            self.resource.iter().collect()
        } else {
            self.resources.iter().collect()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurableOrchestratorState {
    pub schema_version: u32,
    #[serde(default)]
    pub hierarchy: BTreeMap<IssueId, HierarchySnapshot>,
    #[serde(default)]
    pub leases: Vec<LeaseRecord>,
    #[serde(default)]
    pub run_hierarchy_generations: BTreeMap<IssueId, u64>,
}

impl Default for DurableOrchestratorState {
    fn default() -> Self {
        Self {
            schema_version: HIERARCHY_STATE_SCHEMA_VERSION,
            hierarchy: BTreeMap::new(),
            leases: Vec::new(),
            run_hierarchy_generations: BTreeMap::new(),
        }
    }
}

impl DurableOrchestratorState {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != HIERARCHY_STATE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported durable hierarchy schema version {}",
                self.schema_version
            ));
        }
        if self.hierarchy.iter().any(|(parent_id, snapshot)| {
            parent_id != &snapshot.parent_id || snapshot.generation == 0
        }) {
            return Err("durable hierarchy snapshot has invalid identity or generation".to_owned());
        }
        Ok(())
    }

    pub fn acquire_leases(&mut self, records: Vec<LeaseRecord>) -> Result<(), LeaseError> {
        let mut next = self.leases.clone();
        for requested in records {
            if let Some(existing) = next.iter_mut().find(|existing| {
                existing.active()
                    && existing.kind == requested.kind
                    && existing.resource == requested.resource
                    && existing.owner == requested.owner
            }) {
                *existing = requested;
                continue;
            }
            if let Some(existing) = next.iter().find(|existing| {
                existing.active()
                    && existing.kind == requested.kind
                    && existing.resource == requested.resource
                    && existing.owner != requested.owner
                    && matches!(requested.kind, LeaseKind::LeafWorker)
            }) {
                return Err(LeaseError::Conflict {
                    resource: requested.resource,
                    owner: existing.owner.id.clone(),
                });
            }
            next.push(requested);
        }
        self.leases = next;
        self.compact_lease_history();
        Ok(())
    }

    pub fn acquire_required_and_release_leaf(
        &mut self,
        required: Vec<LeaseRecord>,
        leaf_resources: &[LeaseResource],
        released_at: u64,
    ) -> Result<(), LeaseError> {
        let mut next = self.clone();
        next.acquire_leases(required)?;
        let leaf_owner = LeaseOwner::leaf_worker;
        for resource in leaf_resources {
            for lease in next.leases.iter_mut().filter(|lease| {
                lease.active()
                    && lease.kind == LeaseKind::LeafWorker
                    && lease.resource == *resource
                    && lease.owner == leaf_owner(&resource.issue_id)
            }) {
                lease.released_at = Some(released_at);
            }
        }
        *self = next;
        self.compact_lease_history();
        Ok(())
    }

    pub fn release_owner(&mut self, owner: &LeaseOwner, released_at: u64) {
        for lease in &mut self.leases {
            if lease.active() && lease.owner == *owner {
                lease.released_at = Some(released_at);
            }
        }
        self.compact_lease_history();
    }

    pub fn release_parent_leases(&mut self, parent_id: &IssueId, released_at: u64) -> bool {
        self.release_parent_leases_inner(parent_id, released_at, true)
    }

    pub fn release_parent_leases_preserving_ancestor(
        &mut self,
        parent_id: &IssueId,
        released_at: u64,
    ) -> bool {
        self.release_parent_leases_inner(parent_id, released_at, false)
    }

    fn release_parent_leases_inner(
        &mut self,
        parent_id: &IssueId,
        released_at: u64,
        release_ancestor: bool,
    ) -> bool {
        let ancestor_owner = LeaseOwner::ancestor(parent_id);
        let review_prefix = format!("review:{parent_id}:");
        let mut released = false;
        for lease in &mut self.leases {
            if !lease.active() {
                continue;
            }
            let owned_by_parent = (release_ancestor && lease.owner == ancestor_owner)
                || (lease.kind == LeaseKind::Review
                    && lease.owner.kind == "review"
                    && lease.owner.id.starts_with(&review_prefix));
            if owned_by_parent {
                lease.released_at = Some(released_at);
                released = true;
            }
        }
        self.compact_lease_history();
        released
    }

    pub fn descendant_resources_for(&self, parent_id: &IssueId) -> Vec<LeaseResource> {
        let owner = LeaseOwner::ancestor(parent_id);
        let current_generation = self
            .hierarchy
            .get(parent_id)
            .map(|snapshot| snapshot.generation)
            .or_else(|| self.run_hierarchy_generations.get(parent_id).copied());
        let mut resources = BTreeMap::<LeaseResource, u64>::new();
        for lease in &self.leases {
            if lease.active()
                && lease.kind == LeaseKind::AncestorIntegration
                && lease.owner == owner
                && current_generation
                    .is_none_or(|generation| lease.hierarchy_generation == generation)
            {
                resources
                    .entry(lease.resource.clone())
                    .and_modify(|acquired_at| *acquired_at = (*acquired_at).max(lease.acquired_at))
                    .or_insert(lease.acquired_at);
            }
        }
        resources.into_keys().collect()
    }

    pub fn release_obsolete_leaf_leases(
        &mut self,
        removed_child_ids: &[IssueId],
        released_at: u64,
    ) {
        let retained_child_ids = self
            .hierarchy
            .values()
            .flat_map(|snapshot| snapshot.required_child_edges.iter())
            .filter(|edge| edge.required)
            .map(|edge| edge.child_id.clone())
            .collect::<BTreeSet<_>>();
        for lease in &mut self.leases {
            if lease.active()
                && lease.kind == LeaseKind::LeafWorker
                && removed_child_ids.contains(&lease.resource.issue_id)
                && !retained_child_ids.contains(&lease.resource.issue_id)
            {
                lease.released_at = Some(released_at);
            }
        }
        self.compact_lease_history();
    }

    pub fn rebind_leaf_leases(&mut self, child_ids: &BTreeSet<IssueId>, generation: u64) {
        for lease in &mut self.leases {
            if lease.active()
                && lease.kind == LeaseKind::LeafWorker
                && child_ids.contains(&lease.resource.issue_id)
            {
                lease.hierarchy_generation = generation;
            }
        }
    }

    pub fn release_ancestor_leases_for_children(
        &mut self,
        child_ids: &[IssueId],
        released_at: u64,
    ) {
        for lease in &mut self.leases {
            if lease.active()
                && lease.kind == LeaseKind::AncestorIntegration
                && child_ids
                    .iter()
                    .any(|child_id| lease.owner == LeaseOwner::ancestor(child_id))
            {
                lease.released_at = Some(released_at);
            }
        }
        self.compact_lease_history();
    }

    pub fn release_subtree_evidence_for_undispatched_parent(
        &mut self,
        parent_id: &IssueId,
        released_at: u64,
    ) -> bool {
        let mut subtree = BTreeSet::new();
        let mut pending = self
            .hierarchy
            .get(parent_id)
            .into_iter()
            .flat_map(|snapshot| snapshot.required_child_edges.iter())
            .filter(|edge| edge.required)
            .map(|edge| edge.child_id.clone())
            .collect::<Vec<_>>();
        while let Some(issue_id) = pending.pop() {
            if !subtree.insert(issue_id.clone()) {
                continue;
            }
            if let Some(snapshot) = self.hierarchy.get(&issue_id) {
                pending.extend(
                    snapshot
                        .required_child_edges
                        .iter()
                        .filter(|edge| edge.required)
                        .map(|edge| edge.child_id.clone()),
                );
            }
        }

        let review_prefixes = self
            .hierarchy
            .keys()
            .filter(|issue_id| subtree.contains(*issue_id))
            .map(|issue_id| format!("review:{issue_id}:"))
            .collect::<Vec<_>>();
        let mut released = false;
        for lease in &mut self.leases {
            if !lease.active() {
                continue;
            }
            let owned_by_subtree = match lease.kind {
                LeaseKind::LeafWorker => subtree.contains(&lease.resource.issue_id),
                LeaseKind::AncestorIntegration => subtree
                    .iter()
                    .any(|issue_id| lease.owner == LeaseOwner::ancestor(issue_id)),
                LeaseKind::Review => review_prefixes
                    .iter()
                    .any(|prefix| lease.owner.id.starts_with(prefix)),
                LeaseKind::Repair | LeaseKind::DiagnosticHold => false,
            };
            if owned_by_subtree {
                lease.released_at = Some(released_at);
                released = true;
            }
        }
        self.compact_lease_history();
        released
    }

    pub fn release_leaf_leases_for_parent(&mut self, parent_id: &IssueId, released_at: u64) {
        let ancestor_owner = LeaseOwner::ancestor(parent_id);
        let resources = self
            .leases
            .iter()
            .filter(|lease| {
                lease.active()
                    && lease.kind == LeaseKind::AncestorIntegration
                    && lease.owner == ancestor_owner
            })
            .map(|lease| lease.resource.clone())
            .collect::<Vec<_>>();
        for lease in &mut self.leases {
            if lease.active()
                && lease.kind == LeaseKind::LeafWorker
                && resources.iter().any(|resource| resource == &lease.resource)
                && lease.owner == LeaseOwner::leaf_worker(&lease.resource.issue_id)
            {
                lease.released_at = Some(released_at);
            }
        }
        self.compact_lease_history();
    }

    fn compact_lease_history(&mut self) {
        let mut latest_inactive = BTreeSet::new();
        for lease in self.leases.iter().rev() {
            if !lease.active() {
                latest_inactive.insert((
                    lease.kind.clone(),
                    lease.resource.clone(),
                    lease.owner.clone(),
                ));
            }
        }
        let mut compacted = Vec::new();
        for lease in self.leases.drain(..) {
            let key = (
                lease.kind.clone(),
                lease.resource.clone(),
                lease.owner.clone(),
            );
            if lease.active() || latest_inactive.remove(&key) {
                compacted.push(lease);
            }
        }

        let inactive_count = compacted.iter().filter(|lease| !lease.active()).count();
        if inactive_count > MAX_INACTIVE_LEASE_HISTORY {
            let drop_count = inactive_count - MAX_INACTIVE_LEASE_HISTORY;
            let mut candidates = compacted
                .iter()
                .enumerate()
                .filter(|(_, lease)| !lease.active())
                .map(|(index, lease)| {
                    (
                        index,
                        lease
                            .released_at
                            .or(lease.expires_at)
                            .unwrap_or(lease.acquired_at),
                    )
                })
                .collect::<Vec<_>>();
            candidates.sort_by_key(|(_, timestamp)| *timestamp);
            let drop_indices = candidates
                .into_iter()
                .take(drop_count)
                .map(|(index, _)| index)
                .collect::<BTreeSet<_>>();
            self.leases = compacted
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !drop_indices.contains(index))
                .map(|(_, lease)| lease)
                .collect();
        } else {
            self.leases = compacted;
        }
    }

    pub fn active_for(&self, resource: &LeaseResource) -> bool {
        self.leases
            .iter()
            .any(|lease| lease.active() && lease.resource == *resource)
    }

    pub fn has_ancestor_edge(&self, child_id: &IssueId) -> bool {
        self.hierarchy.values().any(|snapshot| {
            snapshot
                .required_child_edges
                .iter()
                .any(|edge| edge.required && &edge.child_id == child_id)
        })
    }

    pub fn has_dispatched_ancestor(&self, child_id: &IssueId) -> bool {
        self.hierarchy.values().any(|snapshot| {
            snapshot.dispatch_claimed()
                && snapshot
                    .required_child_edges
                    .iter()
                    .any(|edge| edge.required && &edge.child_id == child_id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(id: &str, state: &str) -> TrackerIssueRef {
        TrackerIssueRef {
            id: id.to_owned(),
            identifier: id.to_owned(),
            title: None,
            url: None,
            state: state.to_owned(),
        }
    }

    fn parent(children: Vec<TrackerIssueRef>) -> TrackerIssue {
        TrackerIssue {
            id: "parent".to_owned(),
            identifier: "COE-1".to_owned(),
            url: String::new(),
            title: String::new(),
            description: None,
            priority: None,
            state: "In Progress".to_owned(),
            state_kind: crate::opensymphony_domain::TrackerIssueStateKind::Started,
            branch_name: None,
            pr_url: None,
            pr_urls: Vec::new(),
            labels: Vec::new(),
            project_id: None,
            project_slug: None,
            project_name: None,
            parent_id: None,
            parent: None,
            project_milestone: None,
            blocked_by: Vec::new(),
            sub_issues: children,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn mutation_after_freeze_blocks_and_stale_events_are_rejected() {
        let mut snapshot = HierarchySnapshot::new(&parent(vec![child("child-a", "Done")]));
        assert_eq!(snapshot.generation, 1);
        snapshot.freeze().expect("initial scope should freeze");

        assert_eq!(
            snapshot.reconcile(&[child("child-b", "Done")]),
            HierarchyReconciliation::BlockedForReplanning { generation: 2 }
        );
        assert!(!snapshot.accepts_event(1));
        assert!(!snapshot.accepts_event(2));
        snapshot.replan();
        assert!(snapshot.accepts_event(2));
    }

    #[test]
    fn configured_canceled_children_are_non_required_edges() {
        let snapshot = HierarchySnapshot::new_with_canceled_states(
            &parent(vec![child("child-a", "Canceled"), child("child-b", "Done")]),
            &["Canceled".to_owned()],
        );

        assert_eq!(snapshot.required_child_edges.len(), 2);
        assert!(!snapshot.required_child_edges[0].required);
        assert!(snapshot.required_child_edges[1].required);
    }

    #[test]
    fn identifier_rename_refreshes_edge_without_advancing_generation() {
        let parent = parent(vec![child("child-a", "Done")]);
        let mut snapshot = HierarchySnapshot::new(&parent);
        let generation = snapshot.generation;
        let mut renamed = child("child-a", "Done");
        renamed.identifier = "COE-RENAMED".to_owned();

        assert_eq!(
            snapshot.reconcile(&[renamed]),
            HierarchyReconciliation::Unchanged
        );
        assert_eq!(snapshot.generation, generation);
        assert_eq!(
            snapshot.required_child_edges[0].child_identifier,
            IssueIdentifier::new("COE-RENAMED").expect("identifier")
        );
    }

    #[test]
    fn required_leases_are_atomic_and_release_leaf_only_after_acquisition() {
        let resource = LeaseResource {
            issue_id: IssueId::new("child").expect("id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState::default();
        state
            .acquire_leases(vec![LeaseRecord {
                kind: LeaseKind::LeafWorker,
                resource: resource.clone(),
                owner: LeaseOwner::leaf_worker(&resource.issue_id),
                hierarchy_generation: 1,
                acquired_at: 1,
                expires_at: None,
                released_at: None,
            }])
            .expect("leaf lease should acquire");
        let parent_id = IssueId::new("parent").expect("id");
        state
            .acquire_required_and_release_leaf(
                vec![LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&parent_id),
                    hierarchy_generation: 1,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                }],
                std::slice::from_ref(&resource),
                3,
            )
            .expect("required lease transaction should succeed");
        assert!(state.active_for(&resource));
        assert!(
            state.leases.iter().any(|lease| {
                lease.kind == LeaseKind::LeafWorker && lease.released_at == Some(3)
            })
        );
    }

    #[test]
    fn removed_child_edges_release_obsolete_leaf_leases() {
        let resource = LeaseResource {
            issue_id: IssueId::new("child").expect("id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let parent_id = IssueId::new("parent").expect("id");
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(
                parent_id.clone(),
                HierarchySnapshot {
                    parent_id: parent_id.clone(),
                    generation: 2,
                    required_child_edges: Vec::new(),
                    frozen: false,
                    blocked_reason: None,
                    dispatched_generation: None,
                    dispatch_intent_generation: None,
                },
            )]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![LeaseRecord {
                kind: LeaseKind::LeafWorker,
                resource: resource.clone(),
                owner: LeaseOwner::leaf_worker(&resource.issue_id),
                hierarchy_generation: 1,
                acquired_at: 1,
                expires_at: None,
                released_at: None,
            }])
            .expect("leaf lease should acquire");

        state.release_obsolete_leaf_leases(std::slice::from_ref(&resource.issue_id), 2);
        assert_eq!(state.leases[0].released_at, Some(2));
    }

    #[test]
    fn parent_release_keeps_higher_ancestor_and_nested_resources() {
        let resource = LeaseResource {
            issue_id: IssueId::new("leaf").expect("id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let intermediate = IssueId::new("intermediate").expect("id");
        let higher = IssueId::new("higher").expect("id");
        let mut state = DurableOrchestratorState::default();
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&intermediate),
                    hierarchy_generation: 2,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::Review,
                    resource: resource.clone(),
                    owner: LeaseOwner::review_for_parent(&intermediate, &resource.issue_id),
                    hierarchy_generation: 2,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&higher),
                    hierarchy_generation: 1,
                    acquired_at: 3,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("nested leases should acquire");

        assert_eq!(
            state.descendant_resources_for(&intermediate),
            vec![resource.clone()]
        );
        assert!(state.release_parent_leases(&intermediate, 4));
        assert!(
            state.leases.iter().any(|lease| {
                lease.owner == LeaseOwner::ancestor(&higher) && lease.active_at(4)
            })
        );
        assert!(
            state
                .leases
                .iter()
                .filter(|lease| {
                    lease.owner == LeaseOwner::ancestor(&intermediate)
                        || lease.owner
                            == LeaseOwner::review_for_parent(&intermediate, &resource.issue_id)
                })
                .all(|lease| lease.released_at == Some(4))
        );
    }

    #[test]
    fn tracker_terminal_state_alone_is_not_parent_eligibility() {
        let parent = parent(vec![child("child-a", "Done")]);
        let snapshot = HierarchySnapshot::new(&parent);
        let evidence = ParentEligibilityEvidence::tracker_only(&parent, snapshot.generation);

        assert_eq!(
            evidence.eligible_for(&snapshot),
            Err(HierarchyBlockedReason::UnresolvedFailure(
                "child orchestrator outcome is not terminal".to_owned()
            ))
        );
    }

    #[test]
    fn provider_evidence_requires_merge_commit_and_retained_generation() {
        let parent = parent(vec![child("child-a", "Done")]);
        let snapshot = HierarchySnapshot::new(&parent);
        let child_id = IssueId::new("child-a").expect("id");
        let resource = LeaseResource {
            issue_id: child_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut evidence = ParentEligibilityEvidence {
            hierarchy_generation: snapshot.generation,
            children: vec![ChildEligibilityEvidence {
                child_id,
                hierarchy_generation: snapshot.generation,
                orchestrator_terminal: true,
                provider_merge_confirmed: true,
                merge_result_commit: Some("abc123".to_owned()),
                merge_result_commits: Vec::new(),
                merge_repository_id: None,
                merge_repository_ids: Vec::new(),
                resource: Some(resource.clone()),
                resources: Vec::new(),
                unresolved_failure: None,
            }],
        };
        assert!(evidence.eligible_for(&snapshot).is_ok());
        evidence.children[0].merge_result_commit = None;
        assert_eq!(
            evidence.eligible_for(&snapshot),
            Err(HierarchyBlockedReason::MissingTargetCommit)
        );
    }

    #[test]
    fn nested_evidence_accepts_recorded_descendant_commits() {
        let parent = parent(vec![child("nested-parent", "Done")]);
        let snapshot = HierarchySnapshot::new(&parent);
        let resource = LeaseResource {
            issue_id: IssueId::new("leaf").expect("id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut evidence = ParentEligibilityEvidence {
            hierarchy_generation: snapshot.generation,
            children: vec![ChildEligibilityEvidence {
                child_id: IssueId::new("nested-parent").expect("id"),
                hierarchy_generation: snapshot.generation,
                orchestrator_terminal: true,
                provider_merge_confirmed: true,
                merge_result_commit: None,
                merge_result_commits: vec!["descendant-merge".to_owned()],
                merge_repository_id: None,
                merge_repository_ids: Vec::new(),
                resource: None,
                resources: vec![resource],
                unresolved_failure: None,
            }],
        };
        assert!(evidence.eligible_for(&snapshot).is_ok());
        evidence.children[0].merge_repository_ids =
            vec![CanonicalRepositoryId::new("github:other-repo").expect("repository")];
        assert_eq!(
            evidence.eligible_for(&snapshot),
            Err(HierarchyBlockedReason::MissingCheckoutEvidence)
        );
    }

    #[test]
    fn nested_ancestor_leases_keep_distinct_owner_edges() {
        let resource = LeaseResource {
            issue_id: IssueId::new("child").expect("id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let first_parent = IssueId::new("parent-a").expect("id");
        let second_parent = IssueId::new("parent-b").expect("id");
        let mut state = DurableOrchestratorState::default();
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&first_parent),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&second_parent),
                    hierarchy_generation: 2,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("nested owners should coexist");

        state.release_owner(&LeaseOwner::ancestor(&first_parent), 3);

        assert_eq!(
            state.leases.iter().filter(|lease| lease.active()).count(),
            1
        );
        assert_eq!(state.leases[1].owner, LeaseOwner::ancestor(&second_parent));
    }

    #[test]
    fn undispatched_root_release_clears_nested_child_evidence() {
        let root = parent(vec![child("nested-parent", "Done")]);
        let nested = TrackerIssue {
            id: "nested-parent".to_owned(),
            identifier: "COE-2".to_owned(),
            sub_issues: vec![child("leaf", "Done")],
            ..parent(Vec::new())
        };
        let root_id = IssueId::new("parent").expect("root");
        let nested_id = IssueId::new("nested-parent").expect("nested");
        let leaf_id = IssueId::new("leaf").expect("leaf");
        let resource = LeaseResource {
            issue_id: leaf_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([
                (root_id.clone(), HierarchySnapshot::new(&root)),
                (nested_id.clone(), HierarchySnapshot::new(&nested)),
            ]),
            ..DurableOrchestratorState::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource.clone(),
                    owner: LeaseOwner::leaf_worker(&leaf_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&nested_id),
                    hierarchy_generation: 1,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::Review,
                    resource,
                    owner: LeaseOwner::review_for_parent(&nested_id, &leaf_id),
                    hierarchy_generation: 1,
                    acquired_at: 3,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("nested evidence leases should acquire");

        assert!(state.release_subtree_evidence_for_undispatched_parent(&root_id, 4));
        assert!(
            state
                .leases
                .iter()
                .all(|lease| lease.released_at == Some(4))
        );
    }

    #[test]
    fn durable_state_validation_rejects_unknown_schema_and_invalid_snapshot() {
        let state = DurableOrchestratorState {
            schema_version: HIERARCHY_STATE_SCHEMA_VERSION + 1,
            ..Default::default()
        };
        assert!(state.validate().is_err());

        let parent = parent(vec![child("child-a", "Done")]);
        let parent_id = IssueId::new("parent").expect("id");
        let mut state = DurableOrchestratorState::default();
        let mut snapshot = HierarchySnapshot::new(&parent);
        snapshot.generation = 0;
        state.hierarchy.insert(parent_id, snapshot);
        assert!(state.validate().is_err());
    }

    #[test]
    fn expired_leases_are_not_active() {
        let resource = LeaseResource {
            issue_id: IssueId::new("child").expect("id"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let lease = LeaseRecord {
            kind: LeaseKind::DiagnosticHold,
            resource,
            owner: LeaseOwner::diagnostic("operator"),
            hierarchy_generation: 1,
            acquired_at: 1,
            expires_at: Some(10),
            released_at: None,
        };
        assert!(lease.active_at(9));
        assert!(!lease.active_at(10));
    }

    #[test]
    fn inactive_lease_history_is_deduplicated_and_bounded() {
        let mut state = DurableOrchestratorState::default();
        for index in 0..(MAX_INACTIVE_LEASE_HISTORY + 32) {
            let issue_id = IssueId::new(format!("child-{index}")).expect("issue id");
            let lease = LeaseRecord {
                kind: LeaseKind::LeafWorker,
                resource: LeaseResource {
                    issue_id: issue_id.clone(),
                    repository_id: CanonicalRepositoryId::new(format!("github:repo-{index}"))
                        .expect("repository id"),
                    checkout_generation: format!("checkout-{index}"),
                },
                owner: LeaseOwner::leaf_worker(&issue_id),
                hierarchy_generation: 1,
                acquired_at: index as u64,
                expires_at: None,
                released_at: None,
            };
            state
                .acquire_leases(vec![lease])
                .expect("lease should acquire");
            state.release_owner(&LeaseOwner::leaf_worker(&issue_id), index as u64 + 1);
        }

        assert_eq!(state.leases.len(), MAX_INACTIVE_LEASE_HISTORY);
        assert!(state.leases.iter().all(|lease| !lease.active()));
    }
}
