use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::opensymphony_domain::{
    CanonicalRepositoryId, IssueId, IssueIdentifier, TimestampMs, TrackerIssue, TrackerIssueRef,
    TrackerIssueStateKind,
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
            required: !matches!(&child.state_kind, TrackerIssueStateKind::Canceled),
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
    /// The latest provider or orchestrator eligibility result. Unlike
    /// `blocked_reason`, this is diagnostic state and does not require an
    /// explicit hierarchy replan before the next eligibility attempt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eligibility_blocked_reason: Option<HierarchyBlockedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatched_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch_intent_generation: Option<u64>,
    /// The generation of a worker that was already dispatched when a frozen
    /// scope changed. Keep this fence until the scheduler observes the old
    /// execution stopped; clearing `dispatched_generation` alone would let a
    /// descendant mutate a checkout under the still-running worker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_flight_generation: Option<u64>,
    /// Merge-result commits captured with a durable dispatch intent. The
    /// scheduler verifies that the exact checkout prepared for this parent
    /// still contains every commit before launching the worker.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatch_required_merge_commits: Vec<String>,
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
            eligibility_blocked_reason: None,
            dispatched_generation: None,
            dispatch_intent_generation: None,
            in_flight_generation: None,
            dispatch_required_merge_commits: Vec::new(),
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

        let previous_generation = self.generation;
        self.generation = self.generation.saturating_add(1);
        self.required_child_edges = next_edges;
        self.eligibility_blocked_reason = None;
        if self.frozen && self.dispatched_generation == Some(previous_generation) {
            self.in_flight_generation.get_or_insert(previous_generation);
        }
        self.dispatched_generation = None;
        self.dispatch_intent_generation = None;
        self.dispatch_required_merge_commits.clear();
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
        self.eligibility_blocked_reason = None;
        self.dispatched_generation = None;
        self.dispatch_intent_generation = None;
        self.dispatch_required_merge_commits.clear();
        self.in_flight_generation = None;
    }

    pub fn mark_dispatch_intent(&mut self) {
        self.dispatch_intent_generation = Some(self.generation);
    }

    pub fn clear_dispatch_intent(&mut self) {
        self.dispatch_intent_generation = None;
        self.dispatch_required_merge_commits.clear();
    }

    pub fn dispatch_intended(&self) -> bool {
        self.dispatch_intent_generation == Some(self.generation)
    }

    pub fn restore_in_flight_dispatch(&mut self) -> bool {
        let Some(generation) = self.dispatch_intent_generation else {
            return false;
        };
        self.restore_in_flight_dispatch_for_generation(generation)
    }

    /// Restore the execution fence for a recovered run before tracker
    /// reconciliation. A normally launched run has already replaced its
    /// dispatch intent with the durable claim, so recovery must accept either
    /// marker as evidence that the generation is in flight.
    pub fn restore_recovered_dispatch_fence(&mut self) -> bool {
        let Some(generation) = self
            .dispatch_intent_generation
            .or(self.dispatched_generation)
        else {
            return false;
        };
        self.restore_in_flight_dispatch_for_generation(generation)
    }

    fn restore_in_flight_dispatch_for_generation(&mut self, generation: u64) -> bool {
        if self.in_flight_generation == Some(generation) {
            return false;
        }
        self.in_flight_generation = Some(generation);
        true
    }

    pub fn mark_dispatched(&mut self) {
        self.dispatched_generation = Some(self.generation);
        self.dispatch_intent_generation = None;
        self.dispatch_required_merge_commits.clear();
        self.in_flight_generation = None;
    }

    pub fn dispatch_claimed(&self) -> bool {
        self.dispatched_generation == Some(self.generation)
    }

    pub fn has_dispatched_execution_fence(&self) -> bool {
        self.dispatch_claimed() || self.in_flight_generation.is_some()
    }

    pub fn has_in_flight_dispatch(&self) -> bool {
        self.in_flight_generation.is_some()
    }

    pub fn clear_in_flight_dispatch(&mut self) -> bool {
        self.in_flight_generation.take().is_some()
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
            required: !matches!(&child.state_kind, TrackerIssueStateKind::Canceled)
                && !canceled_states.iter().any(|configured_state| {
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
pub struct ProviderEvidenceBoundary {
    pub issue_id: IssueId,
    pub evidence_at: TimestampMs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildEligibilityEvidence {
    pub child_id: IssueId,
    pub hierarchy_generation: u64,
    pub orchestrator_terminal: bool,
    pub provider_merge_confirmed: bool,
    /// Repository-neutral descendants with only canceled leaves have no
    /// provider merge to prove. Keep that resolved edge distinct from a
    /// missing merge commit on a repository-backed child.
    #[serde(default = "default_merge_required", skip_serializing_if = "is_true")]
    pub merge_required: bool,
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
    /// Provider evidence must be newer than the child run that produced the
    /// retained checkout. This prevents a reactivated child from reusing a
    /// previously merged PR on the same branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_evidence_at: Option<TimestampMs>,
    /// Provider evidence for every repository-backed descendant leaf. The
    /// aggregate timestamp remains for compatibility, while nested parents
    /// need each leaf boundary to fence reactivated descendants.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_evidence_by_issue: Vec<ProviderEvidenceBoundary>,
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
                    merge_required: true,
                    merge_result_commit: None,
                    merge_result_commits: Vec::new(),
                    merge_repository_id: None,
                    merge_repository_ids: Vec::new(),
                    provider_evidence_at: None,
                    provider_evidence_by_issue: Vec::new(),
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
                    merge_required: true,
                    merge_result_commit: None,
                    merge_result_commits: Vec::new(),
                    merge_repository_id: None,
                    merge_repository_ids: Vec::new(),
                    provider_evidence_at: None,
                    provider_evidence_by_issue: Vec::new(),
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
            if let Some(failure) = &child.unresolved_failure {
                return Err(HierarchyBlockedReason::UnresolvedFailure(failure.clone()));
            }
            if !child.merge_required {
                if child.merge_result_commit.is_some()
                    || !child.merge_result_commits.is_empty()
                    || child.merge_repository_id.is_some()
                    || !child.merge_repository_ids.is_empty()
                    || child.resource.is_some()
                    || !child.resources.is_empty()
                {
                    return Err(HierarchyBlockedReason::MissingCheckoutEvidence);
                }
                continue;
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
    /// Durable run-start boundaries used to fence provider evidence after
    /// recovery, when the in-memory execution history is not yet hydrated.
    #[serde(default)]
    pub run_started_at_by_issue: BTreeMap<IssueId, TimestampMs>,
    /// Successful orchestrator outcomes that remain relevant to higher
    /// parents after the completed workspace has been cleaned up.
    #[serde(default)]
    pub terminal_orchestrator_issues: BTreeSet<IssueId>,
}

impl Default for DurableOrchestratorState {
    fn default() -> Self {
        Self {
            schema_version: HIERARCHY_STATE_SCHEMA_VERSION,
            hierarchy: BTreeMap::new(),
            leases: Vec::new(),
            run_hierarchy_generations: BTreeMap::new(),
            run_started_at_by_issue: BTreeMap::new(),
            terminal_orchestrator_issues: BTreeSet::new(),
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

    pub fn release_resource_leases(&mut self, resource: &LeaseResource, released_at: u64) -> bool {
        let mut released = false;
        for lease in &mut self.leases {
            if lease.active() && lease.resource == *resource {
                lease.released_at = Some(released_at);
                released = true;
            }
        }
        self.compact_lease_history();
        released
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
        let current_subtree = self.current_subtree_issue_ids(parent_id);
        let current_generation = self
            .hierarchy
            .get(parent_id)
            .map(|snapshot| snapshot.generation)
            .or_else(|| self.run_hierarchy_generations.get(parent_id).copied());
        self.resources_for_ancestor_owner(&owner, current_generation, current_subtree)
    }

    pub fn ancestor_resources_for_child(
        &self,
        parent_id: &IssueId,
        child_id: &IssueId,
    ) -> Vec<LeaseResource> {
        let Some(snapshot) = self.hierarchy.get(parent_id) else {
            return Vec::new();
        };
        if !snapshot
            .required_child_edges
            .iter()
            .any(|edge| edge.required && edge.child_id == *child_id)
        {
            return Vec::new();
        }
        self.resources_for_ancestor_owner(
            &LeaseOwner::ancestor(parent_id),
            Some(snapshot.generation),
            Some(self.subtree_issue_ids(child_id)),
        )
    }

    fn resources_for_ancestor_owner(
        &self,
        owner: &LeaseOwner,
        current_generation: Option<u64>,
        current_subtree: Option<BTreeSet<IssueId>>,
    ) -> Vec<LeaseResource> {
        let mut resources = BTreeMap::<LeaseResource, u64>::new();
        for lease in &self.leases {
            if lease.active()
                && lease.kind == LeaseKind::AncestorIntegration
                && &lease.owner == owner
                && current_generation
                    .is_none_or(|generation| lease.hierarchy_generation == generation)
                && current_subtree
                    .as_ref()
                    .is_some_and(|subtree| subtree.contains(&lease.resource.issue_id))
            {
                resources
                    .entry(lease.resource.clone())
                    .and_modify(|acquired_at| *acquired_at = (*acquired_at).max(lease.acquired_at))
                    .or_insert(lease.acquired_at);
            }
        }
        resources.into_keys().collect()
    }

    pub fn prune_obsolete_run_boundaries(
        &mut self,
        retained_issue_ids: &BTreeSet<IssueId>,
    ) -> bool {
        let mut live_issue_ids = retained_issue_ids.clone();
        live_issue_ids.extend(self.hierarchy.keys().cloned());
        live_issue_ids.extend(self.hierarchy.values().flat_map(|snapshot| {
            snapshot
                .required_child_edges
                .iter()
                .map(|edge| edge.child_id.clone())
        }));
        live_issue_ids.extend(
            self.leases
                .iter()
                .filter(|lease| lease.active())
                .map(|lease| lease.resource.issue_id.clone()),
        );

        let previous_run_hierarchy_len = self.run_hierarchy_generations.len();
        let previous_run_started_len = self.run_started_at_by_issue.len();
        let previous_terminal_outcomes_len = self.terminal_orchestrator_issues.len();
        self.run_hierarchy_generations
            .retain(|issue_id, _| live_issue_ids.contains(issue_id));
        self.run_started_at_by_issue
            .retain(|issue_id, _| live_issue_ids.contains(issue_id));
        self.terminal_orchestrator_issues
            .retain(|issue_id| live_issue_ids.contains(issue_id));
        previous_run_hierarchy_len != self.run_hierarchy_generations.len()
            || previous_run_started_len != self.run_started_at_by_issue.len()
            || previous_terminal_outcomes_len != self.terminal_orchestrator_issues.len()
    }

    fn current_subtree_issue_ids(&self, parent_id: &IssueId) -> Option<BTreeSet<IssueId>> {
        if !self.hierarchy.contains_key(parent_id) {
            return None;
        }
        Some(self.subtree_issue_ids(parent_id))
    }

    fn subtree_issue_ids(&self, parent_id: &IssueId) -> BTreeSet<IssueId> {
        let mut subtree = BTreeSet::from([parent_id.clone()]);
        let mut pending = vec![parent_id.clone()];
        while let Some(issue_id) = pending.pop() {
            let Some(snapshot) = self.hierarchy.get(&issue_id) else {
                continue;
            };
            for child_id in snapshot
                .required_child_edges
                .iter()
                .filter(|edge| edge.required)
                .map(|edge| edge.child_id.clone())
            {
                if subtree.insert(child_id.clone()) {
                    pending.push(child_id);
                }
            }
        }
        subtree
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

    /// Release evidence owned by child subtrees removed from a parent's
    /// frozen scope. Owners are intentionally scoped to the removed subtree;
    /// leases held by the still-retained parent or siblings remain active.
    pub fn release_removed_subtree_leases(
        &mut self,
        parent_id: &IssueId,
        removed_child_ids: &[IssueId],
        reachable_child_edges: Option<&BTreeSet<(IssueId, IssueId)>>,
        released_at: u64,
    ) -> bool {
        let mut subtree = BTreeSet::new();
        let mut pending = removed_child_ids.to_vec();
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
        if subtree.is_empty() {
            return false;
        }

        let retained_child_ids = self
            .hierarchy
            .iter()
            .filter(|(candidate_parent_id, _)| {
                *candidate_parent_id != parent_id && !subtree.contains(*candidate_parent_id)
            })
            .flat_map(|(_, snapshot)| snapshot.required_child_edges.iter())
            .filter(|edge| edge.required)
            .map(|edge| edge.child_id.clone())
            .collect::<BTreeSet<_>>();

        let mut review_prefixes = removed_child_ids
            .iter()
            .map(|child_id| format!("review:{parent_id}:{child_id}"))
            .collect::<Vec<_>>();
        review_prefixes.extend(
            self.hierarchy
                .keys()
                .filter(|issue_id| subtree.contains(*issue_id))
                .map(|issue_id| format!("review:{issue_id}")),
        );

        let mut released = false;
        let still_reachable = |issue_id: &IssueId| {
            retained_child_ids.contains(issue_id)
                || reachable_child_edges.is_some_and(|edges| {
                    edges
                        .iter()
                        .any(|(candidate_parent_id, candidate_child_id)| {
                            candidate_child_id == issue_id
                                && candidate_parent_id != parent_id
                                && !subtree.contains(candidate_parent_id)
                        })
                })
        };
        for lease in &mut self.leases {
            if !lease.active() {
                continue;
            }
            let owned_by_removed_subtree = match lease.kind {
                LeaseKind::LeafWorker => {
                    subtree.contains(&lease.resource.issue_id)
                        && !still_reachable(&lease.resource.issue_id)
                }
                LeaseKind::AncestorIntegration => subtree.iter().any(|issue_id| {
                    lease.owner == LeaseOwner::ancestor(issue_id) && !still_reachable(issue_id)
                }),
                LeaseKind::Review => review_prefixes.iter().any(|prefix| {
                    lease.owner.id == *prefix || lease.owner.id.starts_with(&format!("{prefix}:"))
                }),
                LeaseKind::Repair | LeaseKind::DiagnosticHold => false,
            };
            if owned_by_removed_subtree {
                lease.released_at = Some(released_at);
                released = true;
            }
        }
        self.compact_lease_history();
        released
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
        self.release_subtree_evidence_for_undispatched_parent_with_reachability(
            parent_id,
            None,
            released_at,
        )
    }

    pub fn release_subtree_evidence_for_undispatched_parent_with_reachability(
        &mut self,
        parent_id: &IssueId,
        reachable_child_edges: Option<&BTreeSet<(IssueId, IssueId)>>,
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

        let mut review_prefixes = vec![format!("review:{parent_id}:")];
        review_prefixes.extend(
            self.hierarchy
                .keys()
                .filter(|issue_id| subtree.contains(*issue_id))
                .map(|issue_id| format!("review:{issue_id}:"))
                .collect::<Vec<_>>(),
        );
        let parent_ancestor_owner = LeaseOwner::ancestor(parent_id);
        let still_reachable = |issue_id: &IssueId| {
            reachable_child_edges.is_some_and(|edges| {
                edges
                    .iter()
                    .any(|(candidate_parent_id, candidate_child_id)| {
                        candidate_child_id == issue_id
                            && candidate_parent_id != parent_id
                            && !subtree.contains(candidate_parent_id)
                    })
            })
        };
        let mut released = false;
        for lease in &mut self.leases {
            if !lease.active() {
                continue;
            }
            let owned_by_subtree = match lease.kind {
                LeaseKind::LeafWorker => {
                    subtree.contains(&lease.resource.issue_id)
                        && !still_reachable(&lease.resource.issue_id)
                }
                LeaseKind::AncestorIntegration => {
                    lease.owner == parent_ancestor_owner
                        || subtree.iter().any(|issue_id| {
                            lease.owner == LeaseOwner::ancestor(issue_id)
                                && !still_reachable(issue_id)
                        })
                }
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

    pub fn rebind_ancestor_leases(&mut self, parent_id: &IssueId, generation: u64) {
        let owner = LeaseOwner::ancestor(parent_id);
        for lease in &mut self.leases {
            if lease.active()
                && lease.kind == LeaseKind::AncestorIntegration
                && lease.owner == owner
            {
                lease.hierarchy_generation = generation;
            }
        }
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
        let mut compacted = Vec::new();
        let mut seen_inactive = BTreeSet::new();
        for lease in self.leases.drain(..).rev() {
            let key = (
                lease.kind.clone(),
                lease.resource.clone(),
                lease.owner.clone(),
            );
            if lease.active() || seen_inactive.insert(key) {
                compacted.push(lease);
            }
        }
        compacted.reverse();

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

    pub fn has_active_dispatched_ancestor(&self, child_id: &IssueId) -> bool {
        let mut descendants = vec![child_id.clone()];
        let mut visited = BTreeSet::from([child_id.clone()]);
        while let Some(descendant_id) = descendants.pop() {
            for (parent_id, snapshot) in &self.hierarchy {
                if !snapshot
                    .required_child_edges
                    .iter()
                    .any(|edge| edge.required && edge.child_id == descendant_id)
                {
                    continue;
                }
                if snapshot.has_dispatched_execution_fence()
                    && self.leases.iter().any(|lease| {
                        lease.active()
                            && lease.kind == LeaseKind::AncestorIntegration
                            && lease.owner == LeaseOwner::ancestor(parent_id)
                    })
                {
                    return true;
                }
                if visited.insert(parent_id.clone()) {
                    descendants.push(parent_id.clone());
                }
            }
        }
        false
    }
}

fn default_merge_required() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
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
            state_kind: TrackerIssueStateKind::from_tracker_name(state),
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
            state_kind: TrackerIssueStateKind::Started,
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
    fn frozen_scope_change_keeps_in_flight_dispatch_fence_until_replan() {
        let mut snapshot = HierarchySnapshot::new(&parent(vec![child("child-a", "Done")]));
        snapshot.freeze().expect("initial scope should freeze");
        snapshot.mark_dispatched();

        assert_eq!(
            snapshot.reconcile(&[child("child-b", "Done")]),
            HierarchyReconciliation::BlockedForReplanning { generation: 2 }
        );
        assert!(!snapshot.dispatch_claimed());
        assert!(snapshot.has_dispatched_execution_fence());
        assert!(snapshot.clear_in_flight_dispatch());
        assert!(!snapshot.has_dispatched_execution_fence());

        snapshot.replan();
        assert!(!snapshot.has_dispatched_execution_fence());
    }

    #[test]
    fn dispatch_intent_restores_a_recovered_in_flight_fence() {
        let mut snapshot = HierarchySnapshot::new(&parent(vec![child("child-a", "Done")]));
        snapshot.freeze().expect("initial scope should freeze");
        snapshot.mark_dispatch_intent();

        assert!(snapshot.restore_in_flight_dispatch());
        assert!(snapshot.has_dispatched_execution_fence());
        assert!(!snapshot.restore_in_flight_dispatch());
    }

    #[test]
    fn dispatched_generation_restores_a_recovered_in_flight_fence() {
        let mut snapshot = HierarchySnapshot::new(&parent(vec![child("child-a", "Done")]));
        snapshot.freeze().expect("initial scope should freeze");
        snapshot.mark_dispatched();

        assert!(snapshot.restore_recovered_dispatch_fence());
        assert!(snapshot.has_in_flight_dispatch());
        assert!(!snapshot.restore_recovered_dispatch_fence());
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
    fn tracker_canceled_kind_makes_custom_terminal_state_non_required() {
        let mut duplicate = child("duplicate", "Duplicate");
        duplicate.state_kind = TrackerIssueStateKind::Canceled;
        let snapshot = HierarchySnapshot::new(&parent(vec![duplicate]));

        assert!(!snapshot.required_child_edges[0].required);
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
                    eligibility_blocked_reason: None,
                    dispatched_generation: None,
                    dispatch_intent_generation: None,
                    in_flight_generation: None,
                    dispatch_required_merge_commits: Vec::new(),
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
    fn removed_subtree_preserves_leaf_lease_reparented_to_another_hierarchy() {
        let parent_a = IssueId::new("parent-a").expect("parent a");
        let parent_b = IssueId::new("parent-b").expect("parent b");
        let child_id = IssueId::new("child").expect("child");
        let resource = LeaseResource {
            issue_id: child_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([
                (
                    parent_a.clone(),
                    HierarchySnapshot::new(&parent(vec![child("child", "Done")])),
                ),
                (
                    parent_b,
                    HierarchySnapshot::new(&parent(vec![child("child", "Done")])),
                ),
            ]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource.clone(),
                    owner: LeaseOwner::leaf_worker(&child_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource,
                    owner: LeaseOwner::ancestor(&child_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("leaf lease should acquire");

        assert!(!state.release_removed_subtree_leases(
            &parent_a,
            std::slice::from_ref(&child_id),
            None,
            2,
        ));
        assert!(state.leases.iter().all(LeaseRecord::active));
    }

    #[test]
    fn removed_subtree_preserves_lease_reparented_to_unmaterialized_parent() {
        let parent_a = IssueId::new("parent-a").expect("parent a");
        let parent_b = IssueId::new("parent-b").expect("parent b");
        let child_id = IssueId::new("child").expect("child");
        let resource = LeaseResource {
            issue_id: child_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(
                parent_a.clone(),
                HierarchySnapshot::new(&parent(vec![child("child", "Done")])),
            )]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource.clone(),
                    owner: LeaseOwner::leaf_worker(&child_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource,
                    owner: LeaseOwner::ancestor(&child_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("leases should acquire");
        let reachable_child_edges = BTreeSet::from([(parent_b, child_id.clone())]);

        assert!(!state.release_removed_subtree_leases(
            &parent_a,
            std::slice::from_ref(&child_id),
            Some(&reachable_child_edges),
            2,
        ));
        assert!(state.leases.iter().all(LeaseRecord::active));
    }

    #[test]
    fn removed_nested_subtree_releases_its_ancestor_and_review_owners() {
        let parent_id = IssueId::new("parent").expect("parent");
        let nested_id = IssueId::new("nested").expect("nested");
        let leaf_id = IssueId::new("leaf").expect("leaf");
        let mut nested_issue = parent(vec![child("leaf", "Done")]);
        nested_issue.id = nested_id.to_string();
        let state_parent = parent(vec![child("nested", "Done")]);
        let resource = |issue_id: &IssueId| LeaseResource {
            issue_id: issue_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([
                (parent_id.clone(), HierarchySnapshot::new(&state_parent)),
                (nested_id.clone(), HierarchySnapshot::new(&nested_issue)),
            ]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource(&leaf_id),
                    owner: LeaseOwner::leaf_worker(&leaf_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource(&leaf_id),
                    owner: LeaseOwner::ancestor(&nested_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::Review,
                    resource: resource(&nested_id),
                    owner: LeaseOwner::review_for_parent(&parent_id, &nested_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::Review,
                    resource: resource(&leaf_id),
                    owner: LeaseOwner::review_for_parent(&nested_id, &leaf_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource(&nested_id),
                    owner: LeaseOwner::ancestor(&parent_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("nested leases should acquire");

        assert!(state.release_removed_subtree_leases(
            &parent_id,
            std::slice::from_ref(&nested_id),
            None,
            2,
        ));
        assert!(
            state.leases[..4]
                .iter()
                .all(|lease| lease.released_at == Some(2))
        );
        assert!(state.leases[4].active());
    }

    #[test]
    fn obsolete_run_boundaries_are_pruned_after_hierarchy_and_lease_release() {
        let retained = IssueId::new("retained").expect("retained");
        let obsolete = IssueId::new("obsolete").expect("obsolete");
        let mut state = DurableOrchestratorState {
            run_hierarchy_generations: BTreeMap::from([(obsolete.clone(), 1)]),
            run_started_at_by_issue: BTreeMap::from([
                (retained.clone(), TimestampMs::new(10)),
                (obsolete.clone(), TimestampMs::new(20)),
            ]),
            terminal_orchestrator_issues: BTreeSet::from([retained.clone(), obsolete]),
            ..Default::default()
        };

        assert!(state.prune_obsolete_run_boundaries(&BTreeSet::from([retained.clone()])));
        assert_eq!(
            state.run_started_at_by_issue,
            BTreeMap::from([(retained.clone(), TimestampMs::new(10))])
        );
        assert!(state.run_hierarchy_generations.is_empty());
        assert_eq!(
            state.terminal_orchestrator_issues,
            BTreeSet::from([retained])
        );
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
        let intermediate_issue = TrackerIssue {
            id: intermediate.to_string(),
            identifier: "COE-INTERMEDIATE".to_owned(),
            sub_issues: vec![child("leaf", "Done")],
            ..parent(Vec::new())
        };
        let mut intermediate_snapshot = HierarchySnapshot::new(&intermediate_issue);
        intermediate_snapshot.generation = 2;
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(intermediate.clone(), intermediate_snapshot)]),
            ..Default::default()
        };
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
    fn descendant_resources_ignore_removed_child_subtrees() {
        let parent_id = IssueId::new("parent").expect("parent");
        let current_child_id = IssueId::new("current-child").expect("current child");
        let removed_child_id = IssueId::new("removed-child").expect("removed child");
        let repository_id = CanonicalRepositoryId::new("github:repo").expect("repository");
        let current_resource = LeaseResource {
            issue_id: current_child_id,
            repository_id: repository_id.clone(),
            checkout_generation: "current".to_owned(),
        };
        let removed_resource = LeaseResource {
            issue_id: removed_child_id,
            repository_id,
            checkout_generation: "removed".to_owned(),
        };
        let snapshot = HierarchySnapshot::new(&parent(vec![child("current-child", "Done")]));
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(parent_id.clone(), snapshot)]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: current_resource.clone(),
                    owner: LeaseOwner::ancestor(&parent_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: removed_resource,
                    owner: LeaseOwner::ancestor(&parent_id),
                    hierarchy_generation: 1,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("ancestor resources should acquire");

        assert_eq!(
            state.descendant_resources_for(&parent_id),
            vec![current_resource]
        );
    }

    #[test]
    fn ancestor_resources_for_child_ignore_sibling_resources() {
        let parent_id = IssueId::new("parent").expect("parent");
        let first_child_id = IssueId::new("first-child").expect("first child");
        let second_child_id = IssueId::new("second-child").expect("second child");
        let repository_id = CanonicalRepositoryId::new("github:repo").expect("repository");
        let first_resource = LeaseResource {
            issue_id: first_child_id.clone(),
            repository_id: repository_id.clone(),
            checkout_generation: "first".to_owned(),
        };
        let second_resource = LeaseResource {
            issue_id: second_child_id.clone(),
            repository_id,
            checkout_generation: "second".to_owned(),
        };
        let snapshot = HierarchySnapshot::new(&parent(vec![
            child("first-child", "Done"),
            child("second-child", "Done"),
        ]));
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(parent_id.clone(), snapshot)]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: first_resource.clone(),
                    owner: LeaseOwner::ancestor(&parent_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: second_resource,
                    owner: LeaseOwner::ancestor(&parent_id),
                    hierarchy_generation: 1,
                    acquired_at: 2,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("ancestor resources should acquire");

        assert_eq!(
            state.ancestor_resources_for_child(&parent_id, &first_child_id),
            vec![first_resource]
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
                merge_required: true,
                merge_result_commit: Some("abc123".to_owned()),
                merge_result_commits: Vec::new(),
                merge_repository_id: None,
                merge_repository_ids: Vec::new(),
                provider_evidence_at: None,
                provider_evidence_by_issue: Vec::new(),
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
                merge_required: true,
                merge_result_commit: None,
                merge_result_commits: vec!["descendant-merge".to_owned()],
                merge_repository_id: None,
                merge_repository_ids: Vec::new(),
                provider_evidence_at: None,
                provider_evidence_by_issue: Vec::new(),
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
    fn canceled_repository_neutral_subtree_is_resolved_without_merge_evidence() {
        let parent = parent(vec![child("nested-parent", "Done")]);
        let snapshot = HierarchySnapshot::new(&parent);
        let evidence = ParentEligibilityEvidence {
            hierarchy_generation: snapshot.generation,
            children: vec![ChildEligibilityEvidence {
                child_id: IssueId::new("nested-parent").expect("id"),
                hierarchy_generation: snapshot.generation,
                orchestrator_terminal: true,
                provider_merge_confirmed: false,
                merge_required: false,
                merge_result_commit: None,
                merge_result_commits: Vec::new(),
                merge_repository_id: None,
                merge_repository_ids: Vec::new(),
                provider_evidence_at: None,
                provider_evidence_by_issue: Vec::new(),
                resource: None,
                resources: Vec::new(),
                unresolved_failure: None,
            }],
        };

        assert!(evidence.eligible_for(&snapshot).is_ok());
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
    fn stale_binding_resource_release_clears_all_owners() {
        let issue_id = IssueId::new("child").expect("id");
        let resource = LeaseResource {
            issue_id: issue_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:old-repo").expect("repo"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState::default();
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource.clone(),
                    owner: LeaseOwner::leaf_worker(&issue_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&IssueId::new("parent").expect("parent")),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("leases should acquire");

        assert!(state.release_resource_leases(&resource, 2));
        assert!(state.leases.iter().all(|lease| !lease.active()));
    }

    #[test]
    fn dispatched_ancestor_fence_ends_when_its_lease_is_released() {
        let parent_id = IssueId::new("parent").expect("parent id");
        let child_id = IssueId::new("child").expect("child id");
        let parent = parent(vec![child("child", "Done")]);
        let mut snapshot = HierarchySnapshot::new(&parent);
        snapshot.mark_dispatched();
        let resource = LeaseResource {
            issue_id: child_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository id"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(parent_id.clone(), snapshot)]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![LeaseRecord {
                kind: LeaseKind::AncestorIntegration,
                resource,
                owner: LeaseOwner::ancestor(&parent_id),
                hierarchy_generation: 1,
                acquired_at: 1,
                expires_at: None,
                released_at: None,
            }])
            .expect("ancestor lease should acquire");

        assert!(state.has_active_dispatched_ancestor(&child_id));
        state.release_owner(&LeaseOwner::ancestor(&parent_id), 2);
        assert!(!state.has_active_dispatched_ancestor(&child_id));
    }

    #[test]
    fn active_dispatched_ancestor_fence_walks_nested_hierarchy() {
        let root_id = IssueId::new("root").expect("root id");
        let nested_id = IssueId::new("nested").expect("nested id");
        let leaf_id = IssueId::new("leaf").expect("leaf id");
        let root = parent(vec![child("nested", "Done")]);
        let nested = TrackerIssue {
            id: "nested".to_owned(),
            identifier: "COE-2".to_owned(),
            sub_issues: vec![child("leaf", "Done")],
            ..parent(Vec::new())
        };
        let mut root_snapshot = HierarchySnapshot::new(&root);
        root_snapshot.mark_dispatched();
        let resource = LeaseResource {
            issue_id: leaf_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository id"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([
                (root_id.clone(), root_snapshot),
                (nested_id, HierarchySnapshot::new(&nested)),
            ]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![LeaseRecord {
                kind: LeaseKind::AncestorIntegration,
                resource,
                owner: LeaseOwner::ancestor(&root_id),
                hierarchy_generation: 1,
                acquired_at: 1,
                expires_at: None,
                released_at: None,
            }])
            .expect("root ancestor lease should acquire");

        assert!(state.has_active_dispatched_ancestor(&leaf_id));
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
                LeaseRecord {
                    kind: LeaseKind::Review,
                    resource: LeaseResource {
                        issue_id: leaf_id.clone(),
                        repository_id: CanonicalRepositoryId::new("github:repo")
                            .expect("repository"),
                        checkout_generation: "checkout-1".to_owned(),
                    },
                    owner: LeaseOwner::review_for_parent(&root_id, &leaf_id),
                    hierarchy_generation: 1,
                    acquired_at: 3,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: LeaseResource {
                        issue_id: leaf_id.clone(),
                        repository_id: CanonicalRepositoryId::new("github:repo")
                            .expect("repository"),
                        checkout_generation: "checkout-1".to_owned(),
                    },
                    owner: LeaseOwner::ancestor(&root_id),
                    hierarchy_generation: 1,
                    acquired_at: 5,
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
    fn undispatched_root_release_preserves_reparented_child_evidence() {
        let parent_a = IssueId::new("parent-a").expect("parent a");
        let parent_b = IssueId::new("parent-b").expect("parent b");
        let child_id = IssueId::new("child").expect("child");
        let resource = LeaseResource {
            issue_id: child_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            hierarchy: BTreeMap::from([(
                parent_a.clone(),
                HierarchySnapshot::new(&parent(vec![child("child", "Done")])),
            )]),
            ..Default::default()
        };
        state
            .acquire_leases(vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource.clone(),
                    owner: LeaseOwner::leaf_worker(&child_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource: resource.clone(),
                    owner: LeaseOwner::ancestor(&child_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
                LeaseRecord {
                    kind: LeaseKind::AncestorIntegration,
                    resource,
                    owner: LeaseOwner::ancestor(&parent_a),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: None,
                },
            ])
            .expect("leases should acquire");

        let reachable_child_edges = BTreeSet::from([(parent_b, child_id.clone())]);
        assert!(
            state.release_subtree_evidence_for_undispatched_parent_with_reachability(
                &parent_a,
                Some(&reachable_child_edges),
                2,
            )
        );
        assert!(state.leases.iter().any(|lease| {
            lease.active()
                && lease.kind == LeaseKind::LeafWorker
                && lease.owner == LeaseOwner::leaf_worker(&child_id)
        }));
        assert!(state.leases.iter().any(|lease| {
            lease.active()
                && lease.kind == LeaseKind::AncestorIntegration
                && lease.owner == LeaseOwner::ancestor(&child_id)
        }));
        assert!(!state.leases.iter().any(|lease| {
            lease.active()
                && lease.kind == LeaseKind::AncestorIntegration
                && lease.owner == LeaseOwner::ancestor(&parent_a)
        }));
    }

    #[test]
    fn ancestor_leases_rebind_to_an_explicit_replan_generation() {
        let parent_id = IssueId::new("parent").expect("parent");
        let resource = LeaseResource {
            issue_id: IssueId::new("leaf").expect("leaf"),
            repository_id: CanonicalRepositoryId::new("github:repo").expect("repository"),
            checkout_generation: "checkout-1".to_owned(),
        };
        let mut state = DurableOrchestratorState::default();
        state
            .acquire_leases(vec![LeaseRecord {
                kind: LeaseKind::AncestorIntegration,
                resource,
                owner: LeaseOwner::ancestor(&parent_id),
                hierarchy_generation: 1,
                acquired_at: 1,
                expires_at: None,
                released_at: None,
            }])
            .expect("parent lease should acquire");

        state.rebind_ancestor_leases(&parent_id, 2);

        assert_eq!(state.leases[0].hierarchy_generation, 2);
        assert!(state.leases[0].active());
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
        let issue_id = IssueId::new("reused-child").expect("issue id");
        let resource = LeaseResource {
            issue_id: issue_id.clone(),
            repository_id: CanonicalRepositoryId::new("github:reused-repo").expect("repository id"),
            checkout_generation: "checkout-reused".to_owned(),
        };
        let mut state = DurableOrchestratorState {
            leases: vec![
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource: resource.clone(),
                    owner: LeaseOwner::leaf_worker(&issue_id),
                    hierarchy_generation: 1,
                    acquired_at: 1,
                    expires_at: None,
                    released_at: Some(2),
                },
                LeaseRecord {
                    kind: LeaseKind::LeafWorker,
                    resource,
                    owner: LeaseOwner::leaf_worker(&issue_id),
                    hierarchy_generation: 2,
                    acquired_at: 3,
                    expires_at: None,
                    released_at: Some(4),
                },
            ],
            ..Default::default()
        };
        state
            .acquire_leases(Vec::new())
            .expect("compaction should succeed");
        assert_eq!(state.leases.len(), 1);
        assert_eq!(state.leases[0].hierarchy_generation, 2);
        assert_eq!(state.leases[0].released_at, Some(4));

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
