//! Plan quality checks plus graph-level helpers (cycle / missing-blocker /
//! parallelizable-work).
//!
//! Cycle detection reuses the in-memory validator from
//! `generator::validate_dependency_graph` so the OSYM-732 invariant stays
//! authoritative for the planning crate. The checks here operate on
//! `PlanArtifacts` (the in-memory representation produced by the planner),
//! so they can be invoked before the manifest is persisted to disk or
//! from any planning session that has just generated a draft.
//!
//! Findings are reported as [`PlanCheckFinding`] so the planning-session
//! validation artefact can render them alongside the manifest validation
//! results. Severity follows the same convention as
//! `compiler::ValidationSeverity`: hard errors block publish, warnings
//! are advisory.

use std::collections::{BTreeMap, BTreeSet};

use crate::opensymphony_planning::generator::domain::{PlanArtifacts, PlannedMilestone, TaskId};

use super::domain::{PlanCheckCategory, PlanCheckFinding, PlanCheckSeverity};

/// Quality checker for in-memory planning artefacts.
///
/// The checker consumes a [`PlanArtifacts`] and the optional research and
/// codebase-analysis artefacts loaded by the planning session. It returns
/// a sorted list of [`PlanCheckFinding`]s; missing optional artefacts are
/// surfaced as warnings rather than errors so the checker can run before
/// the generator has finished gathering all of its inputs.
#[allow(dead_code)]
pub struct PlanQualityChecker<'a> {
    artifacts: &'a PlanArtifacts,
    research_finding_count: Option<usize>,
    codebase_risk_count: Option<usize>,
}

impl<'a> PlanQualityChecker<'a> {
    /// Creates a checker that only sees the planning artefacts themselves.
    /// Use [`PlanQualityChecker::with_research`] / `with_codebase` to layer
    /// in the optional research and codebase inputs.
    pub fn new(artifacts: &'a PlanArtifacts) -> Self {
        Self {
            artifacts,
            research_finding_count: None,
            codebase_risk_count: None,
        }
    }

    /// Records the number of findings present in the loaded research brief.
    ///
    /// Semantics (mirrored by [`Self::with_codebase`] / [`Self::check_research_coverage`]):
    ///
    /// * `Some(0)` — the caller loaded the research layer and the brief
    ///   contained zero findings. Emits a `ResearchCoverage` warning asking
    ///   the planning session to verify the brief actually represents
    ///   upstream research.
    /// * `Some(n)` where `n > 0` — research layer loaded with `n` findings,
    ///   no warning emitted.
    /// * `None` — the caller has chosen to skip the research layer (e.g.
    ///   the planning session has not yet loaded it). No warning is
    ///   emitted because the decision to skip is upstream of the checker.
    ///   Inverting this to warn on `None` would silently fail planning
    ///   sessions that legitimately defer research gathering.
    #[allow(dead_code)]
    pub fn with_research(mut self, finding_count: usize) -> Self {
        self.research_finding_count = Some(finding_count);
        self
    }

    /// Records the number of risks present in the loaded codebase analysis.
    ///
    /// Semantics mirror [`Self::with_research`]:
    ///
    /// * `Some(0)` — caller loaded the codebase layer and the analyzer
    ///   reported zero risks; emits a `CodebaseAnalysis` warning.
    /// * `Some(n)` where `n > 0` — analyzer reported `n` risks; no warning.
    /// * `None` — caller chose to skip the codebase layer; no warning.
    #[allow(dead_code)]
    pub fn with_codebase(mut self, risk_count: usize) -> Self {
        self.codebase_risk_count = Some(risk_count);
        self
    }

    /// Runs every plan check and returns the sorted list of findings.
    /// Findings are sorted by `(category, severity, task_id, field)` so the
    /// artifact round-trips through the planning session API predictably.
    pub fn run(&self) -> Vec<PlanCheckFinding> {
        let mut findings = Vec::new();
        self.check_scope_clarity(&mut findings);
        self.check_research_coverage(&mut findings);
        self.check_codebase_analysis(&mut findings);
        self.check_dependency_cycle(&mut findings);
        self.check_missing_inverse_blockers(&mut findings);
        self.check_acceptance_criteria(&mut findings);
        self.check_verification_expectations(&mut findings);

        findings.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| severity_rank(a.severity).cmp(&severity_rank(b.severity)))
                .then_with(|| a.task_id.cmp(&b.task_id))
                .then_with(|| a.field.cmp(&b.field))
        });
        findings
    }

    fn check_scope_clarity(&self, findings: &mut Vec<PlanCheckFinding>) {
        if self.artifacts.milestones.is_empty() {
            findings.push(PlanCheckFinding::error(
                PlanCheckCategory::ScopeClarity,
                None,
                "milestones",
                "Plan contains no milestones; expected at least one milestone",
            ));
        }
        for milestone in &self.artifacts.milestones {
            if milestone.name.trim().is_empty() {
                findings.push(PlanCheckFinding::error(
                    PlanCheckCategory::ScopeClarity,
                    Some(milestone.id.clone()),
                    "milestones",
                    format!("milestone {} has an empty name", milestone.id),
                ));
            }
            if milestone.issues.is_empty() {
                findings.push(PlanCheckFinding::warning(
                    PlanCheckCategory::ScopeClarity,
                    Some(milestone.id.clone()),
                    "issues",
                    format!(
                        "milestone '{}' has no issues; expected at least one issue",
                        milestone.name
                    ),
                ));
            }
            for issue in &milestone.issues {
                if issue.scope_in.is_empty() {
                    findings.push(PlanCheckFinding::warning(
                        PlanCheckCategory::ScopeClarity,
                        Some(issue.id.clone()),
                        "scope.in",
                        format!(
                            "issue '{}' has no in-scope items; expected at least one bullet",
                            issue.title
                        ),
                    ));
                }
            }
        }
    }

    /// Emits a `ResearchCoverage` warning when the caller has loaded
    /// the research layer (`Some`) but the brief contains zero findings.
    /// `None` is intentional — the planning session chose to skip the
    /// layer upstream — and is not warning-eligible. See
    /// [`Self::with_research`] for the full contract.
    fn check_research_coverage(&self, findings: &mut Vec<PlanCheckFinding>) {
        if let Some(0) = self.research_finding_count {
            findings.push(PlanCheckFinding::warning(
                PlanCheckCategory::ResearchCoverage,
                None,
                "research.findings",
                "Planning session reports zero research findings; downstream consumers may not be able to trace plan decisions back to research citations",
            ));
        }
    }

    /// Emits a `CodebaseAnalysis` warning when the caller has loaded the
    /// codebase layer (`Some`) but the analyzer reported zero risks.
    /// `None` means the caller skipped the analyzer upstream and is not
    /// warning-eligible. See [`Self::with_codebase`] for the full contract.
    fn check_codebase_analysis(&self, findings: &mut Vec<PlanCheckFinding>) {
        if let Some(0) = self.codebase_risk_count {
            findings.push(PlanCheckFinding::warning(
                PlanCheckCategory::CodebaseAnalysis,
                None,
                "codebase.risks",
                "Planning session loaded a CodebaseAnalysis with zero risks; rerun the analyzer to confirm the repository has no ownership or integration risks",
            ));
        }
    }

    fn check_dependency_cycle(&self, findings: &mut Vec<PlanCheckFinding>) {
        if let Err(cycle_msg) =
            crate::opensymphony_planning::generator::generator::validate_dependency_graph(
                self.artifacts,
            )
        {
            findings.push(PlanCheckFinding::error(
                PlanCheckCategory::Dependencies,
                None,
                "dependencies",
                cycle_msg.to_string(),
            ));
        }
    }

    fn check_missing_inverse_blockers(&self, findings: &mut Vec<PlanCheckFinding>) {
        // inverse[t] = set of tasks that list t in `blocked_by`.
        let inverse = build_blocker_inverse(self.artifacts);
        for milestone in &self.artifacts.milestones {
            for issue in &milestone.issues {
                check_task_blocker_inverse(issue, milestone, &inverse, findings);
                for sub in &issue.sub_issues {
                    check_task_blocker_inverse(sub, milestone, &inverse, findings);
                }
            }
        }
    }

    fn check_acceptance_criteria(&self, findings: &mut Vec<PlanCheckFinding>) {
        for milestone in &self.artifacts.milestones {
            for issue in &milestone.issues {
                if issue.acceptance_criteria.is_empty() {
                    findings.push(PlanCheckFinding::error(
                        PlanCheckCategory::AcceptanceCriteria,
                        Some(issue.id.clone()),
                        "acceptance_criteria",
                        format!(
                            "issue '{}' has no acceptance criteria; expected at least one",
                            issue.title
                        ),
                    ));
                }
                for sub in &issue.sub_issues {
                    if sub.acceptance_criteria.is_empty() {
                        findings.push(PlanCheckFinding::warning(
                            PlanCheckCategory::AcceptanceCriteria,
                            Some(sub.id.clone()),
                            "acceptance_criteria",
                            format!(
                                "sub-issue '{}' has no acceptance criteria; expected at least one",
                                sub.title
                            ),
                        ));
                    }
                }
            }
        }
    }

    fn check_verification_expectations(&self, findings: &mut Vec<PlanCheckFinding>) {
        for milestone in &self.artifacts.milestones {
            for issue in &milestone.issues {
                for sub in &issue.sub_issues {
                    if sub.verification_steps.is_empty() {
                        findings.push(PlanCheckFinding::error(
                            PlanCheckCategory::VerificationExpectations,
                            Some(sub.id.clone()),
                            "verification_steps",
                            format!(
                                "sub-issue '{}' has no verification expectations; expected at least one",
                                sub.title
                            ),
                        ));
                    }
                }
            }
        }
    }
}

fn severity_rank(severity: PlanCheckSeverity) -> u8 {
    match severity {
        PlanCheckSeverity::Error => 0,
        PlanCheckSeverity::Warning => 1,
    }
}

fn check_task_blocker_inverse(
    task: &dyn BlockingTask,
    milestone: &PlannedMilestone,
    inverse: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    findings: &mut Vec<PlanCheckFinding>,
) {
    // A task `task` declares `target` in `blocks`; that means `target` should list
    // `task` in its `blocked_by`. The inverse map records `target -> task` only when
    // `target.blocked_by` actually contains `task`. If it does not, we surface a
    // warning so reviewers can either fix the `blocks` arrow or add the reciprocal
    // `blockedBy` arrow on the target.
    for target in task.blocks() {
        let reciprocal = inverse.get(target).cloned().unwrap_or_default();
        if !reciprocal.contains(&task.id()) {
            findings.push(PlanCheckFinding::warning(
                PlanCheckCategory::Dependencies,
                Some(task.id()),
                "blocks",
                format!(
                    "task '{}' in milestone '{}' claims to block '{}' but the inverse 'blockedBy' arrow is missing on '{}'",
                    task.id(),
                    milestone.name,
                    target,
                    target
                ),
            ));
        }
    }
}

// `BlockingTask` lives in the parent `graph_validate` module so the
// `graph` and `checks` helpers can share its `id` / `blocks` walks
// without duplicating the trait definition.
use super::BlockingTask;

/// Build the inverse blocker map: for every task `T`, the set of tasks
/// that list `T` as a blocker. Used by the missing-blocker check.
///
/// The map is built only from `blockedBy` (not `blocks`) because blocking
/// metadata is sourced from the inverse direction in the artifact: a task
/// declares who blocks it, and the inverse is consumed uniformly here.
pub fn build_blocker_inverse(artifacts: &PlanArtifacts) -> BTreeMap<TaskId, BTreeSet<TaskId>> {
    let mut inverse: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
    for milestone in &artifacts.milestones {
        for issue in &milestone.issues {
            collect_inverse(&mut inverse, &issue.id, &issue.blocked_by);
            for sub in &issue.sub_issues {
                collect_inverse(&mut inverse, &sub.id, &sub.blocked_by);
            }
        }
    }
    inverse
}

fn collect_inverse(
    inverse: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    task_id: &TaskId,
    blocked_by: &[TaskId],
) {
    for blocker in blocked_by {
        inverse
            .entry(blocker.clone())
            .or_default()
            .insert(task_id.clone());
    }
}

/// Returns the creation-order topological waves of the planning artefacts.
///
/// Each wave is a vector of task ids whose blockers are all present in
/// earlier waves; tasks within a wave can safely be executed in parallel
/// once their preceding wave is complete. The waves are returned in
/// creation order so callers can render them as columns of work.
///
/// A cycle is not detected here: callers must run
/// [`crate::opensymphony_planning::generator::generator::validate_dependency_graph`]
/// first if they want to ensure the artefacts are acyclic before
/// computing waves.
pub fn creation_order_waves(artifacts: &PlanArtifacts) -> Vec<Vec<TaskId>> {
    let mut dependency_map: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
    for milestone in &artifacts.milestones {
        for issue in &milestone.issues {
            dependency_map
                .entry(issue.id.clone())
                .or_default()
                .extend(issue.blocked_by.iter().cloned());
            for sub in &issue.sub_issues {
                dependency_map
                    .entry(sub.id.clone())
                    .or_default()
                    .extend(sub.blocked_by.iter().cloned());
            }
        }
    }
    topo_waves(&dependency_map)
}

fn topo_waves(dependency_map: &BTreeMap<TaskId, BTreeSet<TaskId>>) -> Vec<Vec<TaskId>> {
    let mut remaining: BTreeMap<TaskId, BTreeSet<TaskId>> = dependency_map.clone();
    let mut waves: Vec<Vec<TaskId>> = Vec::new();
    while !remaining.is_empty() {
        let current: Vec<TaskId> = remaining
            .iter()
            .filter_map(|(task_id, deps)| {
                if deps.is_empty() {
                    Some(task_id.clone())
                } else {
                    None
                }
            })
            .collect();
        if current.is_empty() {
            // Cycle remains. Returning what we have is safer than looping
            // forever; callers should run validate_dependency_graph first.
            break;
        }
        for task_id in &current {
            remaining.remove(task_id);
        }
        for deps in remaining.values_mut() {
            // Removing the dependency from the working set via `remaining.remove`
            // above is sufficient: each affected `deps` entry is a TaskId and
            // can only name a node that left the working set this wave, so we
            // retain *any* dependency whose target is still unprocessed.
            deps.retain(|dep| !current.contains(dep));
        }
        waves.push(current);
    }
    waves
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::opensymphony_planning::generator::domain::{PlannedIssue, PlannedMilestone};

    fn issue(id: &str, blocked_by: &[&str]) -> PlannedIssue {
        PlannedIssue {
            id: TaskId::new(id),
            title: format!("Issue {}", id),
            summary: format!("Summary for {}", id),
            scope_in: vec!["in".to_string()],
            scope_out: vec![],
            deliverables: vec!["d".to_string()],
            acceptance_criteria: vec![
                crate::opensymphony_planning::generator::domain::AcceptanceCriterion {
                    description: "AC".to_string(),
                    verification_command: None,
                },
            ],
            verification_steps: vec![],
            context: vec![],
            definition_of_ready: vec![],
            notes: None,
            priority: crate::opensymphony_planning::generator::domain::TaskPriority::Normal,
            estimate: None,
            blocked_by: blocked_by.iter().map(|s| TaskId::new(*s)).collect(),
            blocks: vec![],
            sub_issues: vec![],
            task_file: None,
        }
    }

    fn milestone(id: &str, name: &str, issues: Vec<PlannedIssue>) -> PlannedMilestone {
        PlannedMilestone {
            id: TaskId::new(id),
            name: name.to_string(),
            goal: "Goal".to_string(),
            issues,
            acceptance_criteria: vec![],
            verification_steps: vec![],
            notes: None,
        }
    }

    fn artifacts(milestones: Vec<PlannedMilestone>) -> PlanArtifacts {
        let mut manifest_tasks = Vec::new();
        for milestone in &milestones {
            manifest_tasks.push(
                crate::opensymphony_planning::generator::domain::ManifestTask {
                    id: milestone.id.clone(),
                    file: format!("docs/tasks/{}.md", milestone.id),
                },
            );
        }
        PlanArtifacts {
            generated_at: chrono::Utc::now(),
            planning_wave: "test".to_string(),
            milestones: milestones.clone(),
            manifest: crate::opensymphony_planning::generator::domain::TaskPackageManifest {
                planning_wave: "test".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: milestones.iter().map(|m| m.name.clone()).collect(),
                tasks: manifest_tasks,
            },
            milestone_index: String::new(),
            task_files: Default::default(),
        }
    }

    #[test]
    fn creation_order_waves_serial_chain() {
        let artifacts = artifacts(vec![milestone(
            "M0",
            "M0: wave",
            vec![issue("A", &[]), issue("B", &["A"]), issue("C", &["B"])],
        )]);
        let waves = creation_order_waves(&artifacts);
        assert_eq!(
            waves,
            vec![
                vec![TaskId::new("A")],
                vec![TaskId::new("B")],
                vec![TaskId::new("C")]
            ]
        );
    }

    #[test]
    fn creation_order_waves_parallelizable() {
        let artifacts = artifacts(vec![milestone(
            "M0",
            "M0: parallel",
            vec![issue("A", &[]), issue("B", &[]), issue("C", &["A", "B"])],
        )]);
        let waves = creation_order_waves(&artifacts);
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0], vec![TaskId::new("A"), TaskId::new("B")]);
        assert_eq!(waves[1], vec![TaskId::new("C")]);
    }

    #[test]
    fn missing_inverse_blocker_is_warning() {
        let mut issue_a = issue("A", &[]);
        issue_a.blocks = vec![TaskId::new("B")];
        let artifacts = artifacts(vec![milestone(
            "M0",
            "M0: inverse",
            vec![issue_a, issue("B", &[])],
        )]);
        let findings = PlanQualityChecker::new(&artifacts).run();
        let missing_inverse = findings.iter().any(|f| {
            f.category == PlanCheckCategory::Dependencies && f.message.contains("inverse")
        });
        assert!(missing_inverse);
    }

    #[test]
    fn acceptable_artifact_has_no_errors() {
        let mut issue_a = issue("A", &[]);
        issue_a.blocks = vec![TaskId::new("B")];
        let mut issue_b = issue("B", &["A"]);
        issue_b.blocks = vec![];
        let artifacts = artifacts(vec![milestone("M0", "M0: clean", vec![issue_a, issue_b])]);
        let findings = PlanQualityChecker::new(&artifacts).run();
        assert!(
            !findings
                .iter()
                .any(|f| matches!(f.severity, PlanCheckSeverity::Error)),
            "expected no errors, got {findings:?}"
        );
    }

    #[test]
    fn verification_check_flags_empty_sub_issues() {
        use crate::opensymphony_planning::generator::domain::{PlannedSubIssue, TaskPriority};
        let sub = PlannedSubIssue {
            id: TaskId::new("SUB"),
            title: "Sub".to_string(),
            summary: "Summary".to_string(),
            scope_in: vec!["in".to_string()],
            scope_out: vec![],
            deliverables: vec!["d".to_string()],
            acceptance_criteria: vec![
                crate::opensymphony_planning::generator::domain::AcceptanceCriterion {
                    description: "AC".to_string(),
                    verification_command: None,
                },
            ],
            verification_steps: vec![],
            context: vec![],
            definition_of_ready: vec![],
            notes: None,
            priority: TaskPriority::Normal,
            estimate: None,
            blocked_by: vec![],
            blocks: vec![],
            task_file: None,
        };
        let mut parent_issue = issue("I", &[]);
        parent_issue.acceptance_criteria = vec![
            crate::opensymphony_planning::generator::domain::AcceptanceCriterion {
                description: "AC".to_string(),
                verification_command: None,
            },
        ];
        parent_issue.sub_issues = vec![sub];
        let artifacts = artifacts(vec![milestone("M0", "M0: verify", vec![parent_issue])]);
        let findings = PlanQualityChecker::new(&artifacts).run();
        let verification = findings.iter().any(|f| {
            f.category == PlanCheckCategory::VerificationExpectations
                && f.task_id.as_ref() == Some(&TaskId::new("SUB"))
        });
        assert!(verification);
    }

    #[test]
    fn cyclic_dependency_surfaces_plan_check_error() {
        // A->B->C->A is a cycle. The PlanQualityChecker delegates to the
        // existing `validate_dependency_graph` and surfaces the cycle message
        // as a `PlanCheckFinding` in the `Dependencies` category.
        let a = issue("A", &["C"]);
        let b = issue("B", &["A"]);
        let c = issue("C", &["B"]);
        let artifacts = artifacts(vec![milestone("M0", "M0: cycle", vec![a, b, c])]);
        let findings = PlanQualityChecker::new(&artifacts).run();
        let cycle = findings
            .iter()
            .find(|f| f.category == PlanCheckCategory::Dependencies)
            .expect("expected a Dependencies finding for the cycle");
        assert!(matches!(cycle.severity, PlanCheckSeverity::Error));
        assert!(cycle.message.contains("Cycle"));
    }

    #[test]
    fn cyclic_parallelizable_waves_break_at_cycle() {
        // Even when a cycle exists, `creation_order_waves` should not loop
        // forever: it returns the waves it could compute before getting
        // stuck on the cycle. Callers should have already run the in-memory
        // validator.
        let a = issue("A", &["C"]);
        let b = issue("B", &["A"]);
        let c = issue("C", &["B"]);
        let artifacts = artifacts(vec![milestone("M0", "M0: cyc", vec![a, b, c])]);
        let waves = creation_order_waves(&artifacts);
        assert!(waves.is_empty(), "no wave is solvable when a cycle exists");
    }
}
