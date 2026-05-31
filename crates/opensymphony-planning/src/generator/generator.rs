//! Implementation plan generator that produces structured artifacts.
//!
//! This module takes a PlanningSession containing intake, research, codebase
//! analysis, and Linear graph context and produces:
//!
//! - Planned milestones with issues and sub-issues
//! - Task package manifest (docs/tasks/task-package.yaml equivalent)
//! - Human-readable milestone index
//! - Individual task file contents
//! - Acceptance criteria, verification steps, and dependencies

use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::Utc;

use super::domain::*;
use super::session::{IntakeContext, PlanningSession};

/// Error type for plan generation operations.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    #[error("planning session is incomplete: missing {0}")]
    IncompleteSession(String),
    #[error("circular dependency detected: {0}")]
    CircularDependency(String),
}

/// Escapes a string for safe use in YAML frontmatter double-quoted values.
/// Emits only YAML-recognized double-quoted escapes so generated frontmatter
/// round-trips through parsers instead of relying on permissive behavior.
fn yaml_escape(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 16);
    let starts_with_complex_key_marker = s.starts_with("? ");
    for (idx, c) in s.chars().enumerate() {
        match c {
            '\\' => result.push_str("\\\\"),
            '"' => result.push_str("\\\""),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            '*' => result.push_str("\\u002a"),
            '&' => result.push_str("\\u0026"),
            '?' if idx == 0 && starts_with_complex_key_marker => result.push_str("\\u003f"),
            c if (c as u32) < 0x20 => {
                result.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => result.push(c),
        }
    }
    result
}

/// Collapses a string to one rendered Markdown line for list items and summaries.
fn collapse_markdown_line(s: &str) -> String {
    s.replace('\n', "\\n").replace('\r', "")
}

fn render_id_list(ids: &[TaskId]) -> String {
    ids.iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_bullets(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- {}", collapse_markdown_line(item)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_optional_bullets(items: &[String]) -> String {
    if items.is_empty() {
        "- None".to_string()
    } else {
        render_bullets(items)
    }
}

fn render_acceptance_criteria(criteria: &[AcceptanceCriterion]) -> String {
    criteria
        .iter()
        .map(|criterion| format!("- [ ] {}", collapse_markdown_line(&criterion.description)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_checklist(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("- [ ] {}", collapse_markdown_line(item)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_notes(notes: Option<&str>) -> String {
    notes
        .map(collapse_markdown_line)
        .unwrap_or_else(|| "None".to_string())
}

/// The generator produces structured plan artifacts from a planning session.
pub struct PlanGenerator {
    session: PlanningSession,
    task_counter: usize,
}

struct SubIssueGenerationContext {
    issue_id: TaskId,
    requirement: String,
    planning_wave: String,
    constraints: Vec<String>,
    success_criteria: Vec<String>,
}

impl SubIssueGenerationContext {
    fn from_intake(issue_id: &TaskId, requirement: &str, intake: &IntakeContext) -> Self {
        Self {
            issue_id: issue_id.clone(),
            requirement: requirement.to_string(),
            planning_wave: intake.planning_wave.clone(),
            constraints: intake.constraints.clone(),
            success_criteria: intake.success_criteria.clone(),
        }
    }

    fn implementation_context(&self) -> Vec<String> {
        let mut context = vec![
            format!("Parent issue: {}", self.issue_id),
            format!("Planning wave: {}", self.planning_wave),
        ];
        if !self.constraints.is_empty() {
            context.push(format!(
                "Technical constraints: {}",
                self.constraints.join(", ")
            ));
        }
        context
    }

    fn validation_context(&self) -> Vec<String> {
        let mut context = vec![format!("Validates implementation of {}", self.requirement)];
        if !self.success_criteria.is_empty() {
            context.push(format!(
                "Success criteria: {}",
                self.success_criteria.join("; ")
            ));
        }
        context
    }
}

impl PlanGenerator {
    /// Creates a new generator from a planning session.
    pub fn new(session: PlanningSession) -> Self {
        Self {
            session,
            task_counter: 0,
        }
    }

    /// Generates the complete set of plan artifacts.
    pub fn generate(&mut self) -> Result<PlanArtifacts, GenerationError> {
        self.validate_session()?;

        let milestones = self.generate_milestones();
        let manifest = self.generate_manifest(&milestones);
        let milestone_index = self.render_milestone_index(&milestones);
        let task_files = self.generate_task_files(&milestones);

        Ok(PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: self.session.intake.planning_wave.clone(),
            milestones,
            manifest,
            milestone_index,
            task_files,
        })
    }

    /// Regenerates only the artifacts specified in the scope, preserving others.
    /// When `RegenerationScope::Issues { milestone_ids }` or `SubIssues { issue_ids }`
    /// is used with specific IDs, only those milestones/issues are regenerated.
    pub fn regenerate(
        &mut self,
        existing: &PlanArtifacts,
        scope: &RegenerationScope,
    ) -> Result<PlanArtifacts, GenerationError> {
        self.validate_session()?;
        self.seed_task_counter_from_existing(existing);

        let milestones = match scope {
            RegenerationScope::Issues {
                milestone_ids: None,
            } => self.regenerate_issues_for_milestones(&existing.milestones, None),
            RegenerationScope::Issues {
                milestone_ids: Some(ids),
            } => self.regenerate_issues_for_milestones(&existing.milestones, Some(ids)),
            RegenerationScope::SubIssues { issue_ids: None } => {
                self.regenerate_sub_issues_for_issues(&existing.milestones, None)
            }
            RegenerationScope::SubIssues {
                issue_ids: Some(ids),
            } => self.regenerate_sub_issues_for_issues(&existing.milestones, Some(ids)),
            _ if scope.includes_milestones() => self.generate_milestones(),
            _ => existing.milestones.clone(),
        };

        let manifest = if scope.includes_manifest() {
            self.generate_manifest(&milestones)
        } else {
            existing.manifest.clone()
        };

        let milestone_index = if scope.includes_milestone_index() {
            self.render_milestone_index(&milestones)
        } else {
            existing.milestone_index.clone()
        };

        let task_files = if scope.includes_task_files() {
            self.generate_task_files(&milestones)
        } else {
            existing.task_files.clone()
        };

        Ok(PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: self.session.intake.planning_wave.clone(),
            milestones,
            manifest,
            milestone_index,
            task_files,
        })
    }

    fn seed_task_counter_from_existing(&mut self, existing: &PlanArtifacts) {
        for milestone in &existing.milestones {
            self.observe_task_id(&milestone.id);
            for issue in &milestone.issues {
                self.observe_task_id(&issue.id);
                for sub_issue in &issue.sub_issues {
                    self.observe_task_id(&sub_issue.id);
                }
            }
        }
    }

    fn observe_task_id(&mut self, id: &TaskId) {
        if let Some(number) =
            id.0.strip_prefix("TASK-")
                .and_then(|suffix| suffix.parse::<usize>().ok())
        {
            self.task_counter = self.task_counter.max(number);
        }
    }

    /// Regenerates issues only for the specified milestone IDs, preserving others.
    fn regenerate_issues_for_milestones(
        &mut self,
        existing: &[PlannedMilestone],
        target_ids: Option<&[TaskId]>,
    ) -> Vec<PlannedMilestone> {
        let target_set: Option<HashSet<&TaskId>> =
            target_ids.map(|ids| ids.iter().collect::<HashSet<_>>());
        let requirements_by_milestone = self.requirements_by_milestone(existing.len());

        existing
            .iter()
            .enumerate()
            .map(|(ms_idx, milestone)| {
                if target_set
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&milestone.id))
                {
                    let intake = IntakeContext {
                        planning_wave: self.session.intake.planning_wave.clone(),
                        project_description: self.session.intake.project_description.clone(),
                        success_criteria: self.session.intake.success_criteria.clone(),
                        requirements: requirements_by_milestone
                            .get(ms_idx)
                            .cloned()
                            .unwrap_or_default(),
                        constraints: self.session.intake.constraints.clone(),
                        open_questions: self.session.intake.open_questions.clone(),
                        reference_docs: self.session.intake.reference_docs.clone(),
                    };
                    let issues = self.generate_issues_for_milestone(&intake);
                    PlannedMilestone {
                        id: milestone.id.clone(),
                        name: milestone.name.clone(),
                        goal: milestone.goal.clone(),
                        issues,
                        acceptance_criteria: milestone.acceptance_criteria.clone(),
                        verification_steps: milestone.verification_steps.clone(),
                        notes: milestone.notes.clone(),
                    }
                } else {
                    milestone.clone()
                }
            })
            .collect()
    }

    /// Regenerates sub-issues only for the specified issue IDs, preserving others.
    fn regenerate_sub_issues_for_issues(
        &mut self,
        existing: &[PlannedMilestone],
        target_ids: Option<&[TaskId]>,
    ) -> Vec<PlannedMilestone> {
        let target_set: Option<HashSet<&TaskId>> =
            target_ids.map(|ids| ids.iter().collect::<HashSet<_>>());

        existing
            .iter()
            .map(|milestone| {
                let issues = milestone
                    .issues
                    .iter()
                    .map(|issue| {
                        if target_set
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&issue.id))
                        {
                            let requirement = issue.title.clone();
                            let intake = IntakeContext {
                                planning_wave: self.session.intake.planning_wave.clone(),
                                project_description: self
                                    .session
                                    .intake
                                    .project_description
                                    .clone(),
                                success_criteria: self.session.intake.success_criteria.clone(),
                                requirements: vec![requirement.clone()],
                                constraints: self.session.intake.constraints.clone(),
                                open_questions: self.session.intake.open_questions.clone(),
                                reference_docs: self.session.intake.reference_docs.clone(),
                            };
                            let sub_issue_context = SubIssueGenerationContext::from_intake(
                                &issue.id,
                                &requirement,
                                &intake,
                            );
                            let sub_issues = self.generate_sub_issues_for_issue(&sub_issue_context);
                            PlannedIssue {
                                id: issue.id.clone(),
                                title: issue.title.clone(),
                                summary: issue.summary.clone(),
                                scope_in: issue.scope_in.clone(),
                                scope_out: issue.scope_out.clone(),
                                deliverables: issue.deliverables.clone(),
                                acceptance_criteria: issue.acceptance_criteria.clone(),
                                verification_steps: issue.verification_steps.clone(),
                                context: issue.context.clone(),
                                definition_of_ready: issue.definition_of_ready.clone(),
                                notes: issue.notes.clone(),
                                priority: issue.priority,
                                estimate: issue.estimate,
                                blocked_by: issue.blocked_by.clone(),
                                blocks: issue.blocks.clone(),
                                sub_issues,
                                task_file: issue.task_file.clone(),
                            }
                        } else {
                            issue.clone()
                        }
                    })
                    .collect();

                PlannedMilestone {
                    id: milestone.id.clone(),
                    name: milestone.name.clone(),
                    goal: milestone.goal.clone(),
                    issues,
                    acceptance_criteria: milestone.acceptance_criteria.clone(),
                    verification_steps: milestone.verification_steps.clone(),
                    notes: milestone.notes.clone(),
                }
            })
            .collect()
    }

    fn requirements_by_milestone(&self, milestone_count: usize) -> Vec<Vec<String>> {
        let effective_count = milestone_count.max(1);
        let mut grouped = vec![Vec::new(); effective_count];

        for (idx, requirement) in self.session.intake.requirements.iter().enumerate() {
            grouped[idx % effective_count].push(requirement.clone());
        }

        grouped
    }

    fn validate_session(&self) -> Result<(), GenerationError> {
        if self.session.intake.planning_wave.is_empty() {
            return Err(GenerationError::IncompleteSession(
                "planning_wave".to_string(),
            ));
        }
        if self.session.intake.requirements.is_empty() {
            return Err(GenerationError::IncompleteSession(
                "requirements".to_string(),
            ));
        }
        Ok(())
    }

    fn next_task_id(&mut self) -> TaskId {
        self.task_counter += 1;
        TaskId(format!("TASK-{:03}", self.task_counter))
    }

    fn generate_milestones(&mut self) -> Vec<PlannedMilestone> {
        let planning_wave = self.session.intake.planning_wave.clone();
        let project_description = self.session.intake.project_description.clone();
        let success_criteria = self.session.intake.success_criteria.clone();
        let requirements = self.session.intake.requirements.clone();
        let open_questions = self.session.intake.open_questions.clone();
        let reference_docs = self.session.intake.reference_docs.clone();
        let constraints = self.session.intake.constraints.clone();

        let intake = IntakeContext {
            planning_wave,
            project_description,
            success_criteria,
            requirements: requirements.clone(),
            constraints,
            open_questions,
            reference_docs,
        };

        // Extract milestone structure from Linear analysis if available
        let linear_milestones = self
            .session
            .linear_graph_analysis
            .as_ref()
            .map(|a| a.milestones.clone())
            .unwrap_or_default();

        let mut milestones = Vec::new();

        if linear_milestones.is_empty() {
            // Create a single milestone from intake requirements
            let milestone_id = self.next_task_id();
            let milestone_name = format!(
                "M1: {}",
                intake
                    .project_description
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
            );

            let issues = self.generate_issues_for_milestone(&intake);

            milestones.push(PlannedMilestone {
                id: milestone_id,
                name: milestone_name,
                goal: intake.project_description.clone(),
                issues,
                acceptance_criteria: intake
                    .success_criteria
                    .iter()
                    .map(|c| AcceptanceCriterion {
                        description: c.clone(),
                        verification_command: None,
                    })
                    .collect(),
                verification_steps: Vec::new(),
                notes: None,
            });
        } else {
            // Distribute requirements across Linear milestones using round-robin
            // to ensure all requirements are assigned without dropping any
            let milestone_requirements = self.requirements_by_milestone(linear_milestones.len());

            for (ms_idx, ms) in linear_milestones.iter().enumerate() {
                let milestone_id = self.next_task_id();

                // Warn when a milestone has no assigned requirements instead of silently skipping
                if milestone_requirements[ms_idx].is_empty() {
                    // Internal source modules intentionally share the root
                    // opensymphony package dependency graph.
                    tracing::warn!(
                        milestone = %ms.milestone_name,
                        "Linear milestone has no assigned requirements, skipping."
                    );
                    continue;
                }

                let mut milestone_intake = intake.clone();
                milestone_intake.requirements = milestone_requirements[ms_idx].clone();

                let issues = self.generate_issues_for_milestone(&milestone_intake);

                milestones.push(PlannedMilestone {
                    id: milestone_id,
                    name: ms.milestone_name.clone(),
                    goal: format!("Deliver {} capabilities", ms.milestone_name),
                    issues,
                    acceptance_criteria: Vec::new(),
                    verification_steps: Vec::new(),
                    notes: None,
                });
            }
        }

        milestones
    }

    fn generate_issues_for_milestone(&mut self, intake: &IntakeContext) -> Vec<PlannedIssue> {
        let mut issues: Vec<PlannedIssue> = Vec::new();

        // Generate one issue per requirement as a starting point
        for (idx, requirement) in intake.requirements.iter().enumerate() {
            let issue_id = self.next_task_id();

            // Each issue gets sub-issues for implementation
            let sub_issue_context =
                SubIssueGenerationContext::from_intake(&issue_id, requirement, intake);
            let sub_issues = self.generate_sub_issues_for_issue(&sub_issue_context);

            let blocked_by: Vec<TaskId> = if idx > 0 {
                issues
                    .last()
                    .map(|i| vec![i.id.clone()])
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            // Populate blocks symmetrically: if this issue is blocked by the previous,
            // the previous issue blocks this one
            if !blocked_by.is_empty()
                && let Some(prev_issue) = issues.last_mut()
            {
                prev_issue.blocks.push(issue_id.clone());
            }

            issues.push(PlannedIssue {
                id: issue_id.clone(),
                title: requirement.clone(),
                summary: format!(
                    "Implement {} as a vertical deliverable for the {} planning wave.",
                    requirement, intake.planning_wave
                ),
                scope_in: vec![requirement.clone()],
                scope_out: Vec::new(),
                deliverables: vec![format!("Working {} implementation", requirement)],
                acceptance_criteria: vec![AcceptanceCriterion {
                    description: format!("{} meets acceptance standards", requirement),
                    verification_command: None,
                }],
                verification_steps: vec![format!("Test {} functionality", requirement)],
                context: vec![
                    format!("Planning wave: {}", intake.planning_wave),
                    format!("Requirement {} of {}", idx + 1, intake.requirements.len()),
                ],
                definition_of_ready: vec![
                    "Hidden assumptions from prior discussion are written down.".to_string(),
                    "Required files, docs, and dependencies are explicitly referenced.".to_string(),
                    "A coding agent could begin execution without additional planning context."
                        .to_string(),
                ],
                notes: None,
                priority: TaskPriority::default(),
                estimate: None,
                blocked_by,
                blocks: Vec::new(),
                sub_issues,
                task_file: Some(format!("{}/{}.md", self.session.tasks_dir, issue_id)),
            });
        }

        issues
    }

    fn generate_sub_issues_for_issue(
        &mut self,
        generation: &SubIssueGenerationContext,
    ) -> Vec<PlannedSubIssue> {
        let mut sub_issues = Vec::new();

        // Generate implementation sub-issue
        let impl_id = self.next_task_id();

        // Generate validation sub-issue (needs impl_id for blocked_by)
        let val_id = self.next_task_id();

        // Implementation sub-issue blocks the validation sub-issue
        sub_issues.push(PlannedSubIssue {
            id: impl_id.clone(),
            title: format!("Implement {}", generation.requirement),
            summary: format!(
                "Implementation unit for {} in the {} planning wave",
                generation.requirement, generation.planning_wave
            ),
            scope_in: vec![format!("Core implementation of {}", generation.requirement)],
            scope_out: vec![format!(
                "Testing and validation of {}",
                generation.requirement
            )],
            deliverables: vec!["Implementation code".to_string(), "Unit tests".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: format!(
                    "Implementation of {} compiles and passes tests",
                    generation.requirement
                ),
                verification_command: Some("cargo test".to_string()),
            }],
            verification_steps: vec![
                "Run unit tests".to_string(),
                "Verify code style".to_string(),
            ],
            context: generation.implementation_context(),
            definition_of_ready: vec![
                "Requirements are clear and understood.".to_string(),
                "Dependencies are available.".to_string(),
            ],
            notes: None,
            priority: TaskPriority::default(),
            estimate: Some(3),
            blocked_by: Vec::new(),
            blocks: vec![val_id.clone()],
            task_file: Some(format!("{}/{}.md", self.session.tasks_dir, impl_id)),
        });

        // Validation sub-issue is blocked by the implementation sub-issue
        sub_issues.push(PlannedSubIssue {
            id: val_id.clone(),
            title: format!("Validate {}", generation.requirement),
            summary: format!("Validation and testing for {}", generation.requirement),
            scope_in: vec![
                "Integration testing".to_string(),
                "Acceptance criteria verification".to_string(),
            ],
            scope_out: vec!["Implementation changes".to_string()],
            deliverables: vec!["Test report".to_string(), "Validation evidence".to_string()],
            acceptance_criteria: vec![AcceptanceCriterion {
                description: format!(
                    "All acceptance criteria for {} are met",
                    generation.requirement
                ),
                verification_command: Some("cargo test --all".to_string()),
            }],
            verification_steps: vec![
                "Run integration tests".to_string(),
                "Verify acceptance criteria".to_string(),
                "Generate validation report".to_string(),
            ],
            context: generation.validation_context(),
            definition_of_ready: vec![
                "Implementation is complete.".to_string(),
                "Test environment is configured.".to_string(),
            ],
            notes: None,
            priority: TaskPriority::default(),
            estimate: Some(2),
            blocked_by: vec![impl_id],
            blocks: Vec::new(),
            task_file: Some(format!("{}/{}.md", self.session.tasks_dir, val_id)),
        });

        sub_issues
    }

    fn generate_manifest(&self, milestones: &[PlannedMilestone]) -> TaskPackageManifest {
        let mut tasks = Vec::new();
        let mut milestone_names = Vec::new();

        for milestone in milestones {
            milestone_names.push(milestone.name.clone());

            for issue in &milestone.issues {
                if let Some(ref task_file) = issue.task_file {
                    tasks.push(ManifestTask {
                        id: issue.id.clone(),
                        file: task_file.clone(),
                    });
                }

                for sub_issue in &issue.sub_issues {
                    if let Some(ref task_file) = sub_issue.task_file {
                        tasks.push(ManifestTask {
                            id: sub_issue.id.clone(),
                            file: task_file.clone(),
                        });
                    }
                }
            }
        }

        TaskPackageManifest {
            planning_wave: self.session.intake.planning_wave.clone(),
            tasks_dir: self.session.tasks_dir.clone(),
            milestones: milestone_names,
            tasks,
        }
    }

    fn render_milestone_index(&self, milestones: &[PlannedMilestone]) -> String {
        let mut md = String::from("# Project Milestones\n\n");

        for milestone in milestones {
            md.push_str(&format!("## {}\n\n", milestone.name));
            md.push_str(&format!("Goal: {}\n\n", milestone.goal));

            if !milestone.issues.is_empty() {
                md.push_str("Tasks:\n\n");
                for issue in &milestone.issues {
                    md.push_str(&format!("- {} {}\n", issue.id, issue.title));
                    for sub_issue in &issue.sub_issues {
                        md.push_str(&format!("  - {} {}\n", sub_issue.id, sub_issue.title));
                    }
                }
            }
            md.push('\n');
        }

        md
    }

    fn generate_task_files(&self, milestones: &[PlannedMilestone]) -> BTreeMap<TaskId, String> {
        let mut task_files = BTreeMap::new();

        for milestone in milestones {
            for issue in &milestone.issues {
                let content = self.render_issue_task_file(issue, milestone);
                task_files.insert(issue.id.clone(), content);

                for sub_issue in &issue.sub_issues {
                    let content = self.render_sub_issue_task_file(sub_issue, issue, milestone);
                    task_files.insert(sub_issue.id.clone(), content);
                }
            }
        }

        task_files
    }

    fn render_issue_task_file(&self, issue: &PlannedIssue, milestone: &PlannedMilestone) -> String {
        let mut content = format!(
            r#"---
id: {id}
title: "{title}"
milestone: "{milestone}"
priority: {priority}
estimate: {estimate}
blockedBy: [{blocked_by}]
blocks: [{blocks}]
parent: null
---

## Summary

{summary}

## Scope

### In scope

{scope_in}

### Out of scope

{scope_out}

## Deliverables

{deliverables}

## Acceptance Criteria

{acceptance_criteria}

## Test Plan

{verification_steps}

## Context

{context}

## Definition of Ready

{definition_of_ready}

## Notes

{notes}
"#,
            id = issue.id,
            title = yaml_escape(&issue.title),
            milestone = yaml_escape(&milestone.name),
            priority = issue.priority.as_linear_priority(),
            estimate = issue
                .estimate
                .map(|estimate| estimate.to_string())
                .unwrap_or_else(|| "null".to_string()),
            blocked_by = render_id_list(&issue.blocked_by),
            blocks = render_id_list(&issue.blocks),
            summary = collapse_markdown_line(&issue.summary),
            scope_in = render_bullets(&issue.scope_in),
            scope_out = render_optional_bullets(&issue.scope_out),
            deliverables = render_bullets(&issue.deliverables),
            acceptance_criteria = render_acceptance_criteria(&issue.acceptance_criteria),
            verification_steps = render_bullets(&issue.verification_steps),
            context = render_bullets(&issue.context),
            definition_of_ready = render_checklist(&issue.definition_of_ready),
            notes = render_notes(issue.notes.as_deref()),
        );

        // Include sub-issues as part of the issue content
        if !issue.sub_issues.is_empty() {
            content.push_str("\n## Sub-issues\n\n");
            for sub_issue in &issue.sub_issues {
                content.push_str(&format!("- {} {}\n", sub_issue.id, sub_issue.title));
            }
        }

        content
    }

    fn render_sub_issue_task_file(
        &self,
        sub_issue: &PlannedSubIssue,
        parent_issue: &PlannedIssue,
        milestone: &PlannedMilestone,
    ) -> String {
        let content = format!(
            r#"---
id: {id}
title: "{title}"
milestone: "{milestone}"
priority: {priority}
estimate: {estimate}
blockedBy: [{blocked_by}]
blocks: [{blocks}]
parent: {parent}
---

## Summary

{summary}

## Scope

### In scope

{scope_in}

### Out of scope

{scope_out}

## Deliverables

{deliverables}

## Acceptance Criteria

{acceptance_criteria}

## Test Plan

{verification_steps}

## Context

{context}

## Definition of Ready

{definition_of_ready}

## Notes

{notes}
"#,
            id = sub_issue.id,
            title = yaml_escape(&sub_issue.title),
            milestone = yaml_escape(&milestone.name),
            priority = sub_issue.priority.as_linear_priority(),
            estimate = sub_issue
                .estimate
                .map(|estimate| estimate.to_string())
                .unwrap_or_else(|| "null".to_string()),
            blocked_by = render_id_list(&sub_issue.blocked_by),
            blocks = render_id_list(&sub_issue.blocks),
            parent = parent_issue.id,
            summary = collapse_markdown_line(&sub_issue.summary),
            scope_in = render_bullets(&sub_issue.scope_in),
            scope_out = render_optional_bullets(&sub_issue.scope_out),
            deliverables = render_bullets(&sub_issue.deliverables),
            acceptance_criteria = render_acceptance_criteria(&sub_issue.acceptance_criteria),
            verification_steps = render_bullets(&sub_issue.verification_steps),
            context = render_bullets(&sub_issue.context),
            definition_of_ready = render_checklist(&sub_issue.definition_of_ready),
            notes = render_notes(sub_issue.notes.as_deref()),
        );

        content
    }
}

/// Validates that a dependency graph has no cycles.
pub fn validate_dependency_graph(artifacts: &PlanArtifacts) -> Result<(), GenerationError> {
    // Build adjacency map once for O(1) lookups instead of O(N) linear scans
    let dep_map = build_dependency_map(artifacts);
    let mut visited = BTreeMap::new();

    for milestone in &artifacts.milestones {
        for issue in &milestone.issues {
            validate_task_dependencies_with_map(&issue.id, &dep_map, &mut visited)?;

            for sub_issue in &issue.sub_issues {
                validate_task_dependencies_with_map(&sub_issue.id, &dep_map, &mut visited)?;
            }
        }
    }

    Ok(())
}

/// Builds a lookup map from task ID to dependency IDs.
///
/// `blocked_by` is already in dependency orientation: task -> blocker.
/// `blocks` is the inverse metadata: blocker -> blocked task, so it must be
/// reversed before adding it to the dependency map. That lets validation catch
/// cycles recorded only in `blocks` without treating normal symmetric metadata
/// as a false two-node cycle.
fn build_dependency_map(artifacts: &PlanArtifacts) -> BTreeMap<TaskId, Vec<TaskId>> {
    let mut map: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
    for milestone in &artifacts.milestones {
        for issue in &milestone.issues {
            add_dependency_edges(&mut map, &issue.id, &issue.blocked_by, &issue.blocks);
            for sub_issue in &issue.sub_issues {
                add_dependency_edges(
                    &mut map,
                    &sub_issue.id,
                    &sub_issue.blocked_by,
                    &sub_issue.blocks,
                );
            }
        }
    }

    map.into_iter()
        .map(|(task_id, dependencies)| (task_id, dependencies.into_iter().collect()))
        .collect()
}

fn add_dependency_edges(
    map: &mut BTreeMap<TaskId, BTreeSet<TaskId>>,
    task_id: &TaskId,
    blocked_by: &[TaskId],
    blocks: &[TaskId],
) {
    map.entry(task_id.clone())
        .or_default()
        .extend(blocked_by.iter().cloned());

    for blocked_task in blocks {
        map.entry(blocked_task.clone())
            .or_default()
            .insert(task_id.clone());
    }
}

fn validate_task_dependencies_with_map(
    task_id: &TaskId,
    dep_map: &BTreeMap<TaskId, Vec<TaskId>>,
    visited: &mut BTreeMap<TaskId, bool>,
) -> Result<(), GenerationError> {
    if let Some(&in_progress) = visited.get(task_id) {
        if in_progress {
            return Err(GenerationError::CircularDependency(format!(
                "Cycle detected involving task {}",
                task_id
            )));
        }
        return Ok(());
    }

    visited.insert(task_id.clone(), true);

    // O(1) lookup instead of linear scan
    if let Some(deps) = dep_map.get(task_id) {
        for dep in deps {
            validate_task_dependencies_with_map(dep, dep_map, visited)?;
        }
    }

    visited.insert(task_id.clone(), false);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sample_session() -> PlanningSession {
        PlanningSession::new(
            IntakeContext {
                planning_wave: "test-wave".to_string(),
                project_description: "Test project for unit testing".to_string(),
                success_criteria: vec!["All tests pass".to_string()],
                requirements: vec!["Feature A".to_string(), "Feature B".to_string()],
                constraints: vec!["Must use Rust".to_string()],
                open_questions: vec![],
                reference_docs: vec![],
            },
            "docs/tasks",
        )
    }

    #[test]
    fn yaml_escape_round_trips_yaml_indicator_characters() {
        let raw = "? &anchor *alias # comment-ish\nquoted \"value\"\tcontrol\u{0007}";
        let yaml = format!("title: \"{}\"\n", yaml_escape(raw));

        let parsed: BTreeMap<String, String> =
            serde_yaml::from_str(&yaml).expect("escaped frontmatter should parse");

        assert_eq!(parsed.get("title").map(String::as_str), Some(raw));
    }

    #[test]
    fn generator_produces_milestones_with_issues_and_subissues() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(!artifacts.milestones.is_empty());

        // Each requirement should produce at least one issue
        let total_issues: usize = artifacts.milestones.iter().map(|m| m.issues.len()).sum();
        assert!(total_issues > 0);

        // Each issue should have sub-issues
        for milestone in &artifacts.milestones {
            for issue in &milestone.issues {
                assert!(!issue.sub_issues.is_empty());
            }
        }
    }

    #[test]
    fn generator_produces_valid_manifest() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert_eq!(artifacts.manifest.planning_wave, "test-wave");
        assert_eq!(artifacts.manifest.tasks_dir, "docs/tasks");
        assert!(!artifacts.manifest.milestones.is_empty());
        assert!(!artifacts.manifest.tasks.is_empty());

        // Each milestone in the manifest should have a matching entry
        for milestone_name in &artifacts.manifest.milestones {
            assert!(
                artifacts
                    .milestones
                    .iter()
                    .any(|m| &m.name == milestone_name),
                "Milestone {} not found in artifacts",
                milestone_name
            );
        }
    }

    #[test]
    fn generator_produces_milestone_index() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(artifacts.milestone_index.contains("# Project Milestones"));

        for milestone in &artifacts.milestones {
            assert!(artifacts.milestone_index.contains(&milestone.name));
        }
    }

    #[test]
    fn generator_produces_task_files() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(!artifacts.task_files.is_empty());

        // Each issue and sub-issue should have a task file
        for milestone in &artifacts.milestones {
            for issue in &milestone.issues {
                assert!(artifacts.task_files.contains_key(&issue.id));
                for sub_issue in &issue.sub_issues {
                    assert!(artifacts.task_files.contains_key(&sub_issue.id));
                }
            }
        }
    }

    #[test]
    fn generator_fails_without_requirements() {
        let mut session = make_sample_session();
        session.intake.requirements.clear();
        let mut generator = PlanGenerator::new(session);
        let result = generator.generate();

        assert!(result.is_err());
        match result.expect_err("expected error should be returned") {
            GenerationError::IncompleteSession(field) => {
                assert_eq!(field, "requirements");
            }
            other => panic!("expected IncompleteSession error, got {:?}", other),
        }
    }

    #[test]
    fn regeneration_preserves_unselected_artifacts() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let original = generator.generate().expect("generation should succeed");

        // Regenerate only the manifest
        let regenerated = generator
            .regenerate(&original, &RegenerationScope::Manifest)
            .expect("regeneration should succeed");

        // Milestones should be preserved
        assert_eq!(original.milestones.len(), regenerated.milestones.len());

        // Milestone index should be preserved
        assert_eq!(original.milestone_index, regenerated.milestone_index);

        // Task files should be preserved
        assert_eq!(original.task_files.len(), regenerated.task_files.len());
    }

    #[test]
    fn regeneration_with_unscoped_issues_regenerates_all_issues() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let original = generator.generate().expect("generation should succeed");

        let mut updated_session = make_sample_session();
        updated_session.intake.requirements = vec!["Feature C".to_string()];
        let mut generator = PlanGenerator::new(updated_session);
        let regenerated = generator
            .regenerate(
                &original,
                &RegenerationScope::Issues {
                    milestone_ids: None,
                },
            )
            .expect("issue regeneration should succeed");

        let milestone = regenerated
            .milestones
            .first()
            .expect("milestone should be preserved");
        assert_eq!(milestone.issues.len(), 1);
        assert_eq!(milestone.issues[0].title, "Feature C");
        assert_ne!(original.milestone_index, regenerated.milestone_index);
        assert!(regenerated.task_files.contains_key(&milestone.issues[0].id));
        assert_ne!(milestone.id, milestone.issues[0].id);
    }

    #[test]
    fn regeneration_with_unscoped_sub_issues_regenerates_all_sub_issues() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let original = generator.generate().expect("generation should succeed");
        let original_issue = &original.milestones[0].issues[0];
        let original_sub_issue_ids: Vec<TaskId> = original_issue
            .sub_issues
            .iter()
            .map(|sub_issue| sub_issue.id.clone())
            .collect();

        let mut updated_session = make_sample_session();
        updated_session
            .intake
            .constraints
            .push("Must include operator evidence".to_string());
        let mut generator = PlanGenerator::new(updated_session);
        let regenerated = generator
            .regenerate(&original, &RegenerationScope::SubIssues { issue_ids: None })
            .expect("sub-issue regeneration should succeed");

        let regenerated_issue = &regenerated.milestones[0].issues[0];
        let regenerated_sub_issue_ids: Vec<TaskId> = regenerated_issue
            .sub_issues
            .iter()
            .map(|sub_issue| sub_issue.id.clone())
            .collect();

        assert_eq!(original_issue.id, regenerated_issue.id);
        assert_ne!(original_sub_issue_ids, regenerated_sub_issue_ids);
        assert!(
            regenerated_issue.sub_issues[0]
                .context
                .iter()
                .any(|entry| entry.contains("Must include operator evidence"))
        );
        assert!(
            regenerated
                .task_files
                .contains_key(&regenerated_issue.sub_issues[0].id)
        );
    }

    #[test]
    fn dependency_graph_validation_passes_for_valid_graph() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        assert!(validate_dependency_graph(&artifacts).is_ok());
    }

    #[test]
    fn task_ids_are_unique() {
        let session = make_sample_session();
        let mut generator = PlanGenerator::new(session);
        let artifacts = generator.generate().expect("generation should succeed");

        use std::collections::BTreeSet;
        let mut all_ids = BTreeSet::new();

        // Count total expected unique IDs
        let mut total_expected = 0;
        for milestone in &artifacts.milestones {
            total_expected += 1;
            assert!(
                all_ids.insert(milestone.id.0.clone()),
                "duplicate milestone id: {}",
                milestone.id
            );
            for issue in &milestone.issues {
                total_expected += 1;
                assert!(
                    all_ids.insert(issue.id.0.clone()),
                    "duplicate issue id: {}",
                    issue.id
                );
                for sub_issue in &issue.sub_issues {
                    total_expected += 1;
                    assert!(
                        all_ids.insert(sub_issue.id.0.clone()),
                        "duplicate sub-issue id: {}",
                        sub_issue.id
                    );
                }
            }
        }

        assert_eq!(all_ids.len(), total_expected, "all ids should be unique");

        // Each manifest task should reference a known id
        for task in &artifacts.manifest.tasks {
            assert!(
                all_ids.contains(&task.id.0),
                "Task ID {} not found in milestone/issue/sub-issue structure",
                task.id.0
            );
        }
    }

    #[test]
    fn dependency_graph_validation_detects_cycle() {
        // Build artifacts with a cycle: A → B → C → A
        // All three tasks must exist as issues/sub-issues for the graph traversal to find the cycle.
        let cycle_a = TaskId("TASK-001".to_string());
        let cycle_b = TaskId("TASK-002".to_string());
        let cycle_c = TaskId("TASK-003".to_string());

        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "test".to_string(),
            milestones: vec![PlannedMilestone {
                id: TaskId("MS-1".to_string()),
                name: "M1: Test".to_string(),
                goal: "Test goal".to_string(),
                issues: vec![
                    PlannedIssue {
                        id: cycle_a.clone(),
                        title: "Task A".to_string(),
                        summary: "A".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_c.clone()], // A blocked by C (cycle)
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_b.clone(),
                        title: "Task B".to_string(),
                        summary: "B".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_a.clone()], // B blocked by A
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_c.clone(),
                        title: "Task C".to_string(),
                        summary: "C".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_b.clone()], // C blocked by B
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                ],
                acceptance_criteria: vec![],
                verification_steps: vec![],
                notes: None,
            }],
            manifest: TaskPackageManifest {
                planning_wave: "test".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec!["M1: Test".to_string()],
                tasks: vec![
                    ManifestTask {
                        id: cycle_a.clone(),
                        file: "docs/tasks/a.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_b.clone(),
                        file: "docs/tasks/b.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_c.clone(),
                        file: "docs/tasks/c.md".to_string(),
                    },
                ],
            },
            milestone_index: String::new(),
            task_files: BTreeMap::new(),
        };

        let result = validate_dependency_graph(&artifacts);
        assert!(result.is_err(), "cycle should be detected");
        match result.expect_err("expected error should be returned") {
            GenerationError::CircularDependency(msg) => {
                assert!(msg.contains("Cycle"));
            }
            other => panic!("expected CircularDependency, got {:?}", other),
        }
    }

    #[test]
    fn dependency_graph_validation_detects_deep_cycle() {
        // Build a 3-node cycle: A blocks B, B blocks C, C blocks A
        // The old buggy implementation (passing &[] for deps) would NOT detect this.
        let cycle_a = TaskId("TASK-001".to_string());
        let cycle_b = TaskId("TASK-002".to_string());
        let cycle_c = TaskId("TASK-003".to_string());

        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "test".to_string(),
            milestones: vec![PlannedMilestone {
                id: TaskId("MS-1".to_string()),
                name: "M1: Test".to_string(),
                goal: "Test goal".to_string(),
                issues: vec![
                    PlannedIssue {
                        id: cycle_a.clone(),
                        title: "Task A".to_string(),
                        summary: "A".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_c.clone()],
                        blocks: vec![cycle_b.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_b.clone(),
                        title: "Task B".to_string(),
                        summary: "B".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_a.clone()],
                        blocks: vec![cycle_c.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_c.clone(),
                        title: "Task C".to_string(),
                        summary: "C".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![cycle_b.clone()],
                        blocks: vec![],
                        sub_issues: vec![],
                        task_file: None,
                    },
                ],
                acceptance_criteria: vec![],
                verification_steps: vec![],
                notes: None,
            }],
            manifest: TaskPackageManifest {
                planning_wave: "test".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec!["M1: Test".to_string()],
                tasks: vec![
                    ManifestTask {
                        id: cycle_a.clone(),
                        file: "docs/tasks/a.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_b.clone(),
                        file: "docs/tasks/b.md".to_string(),
                    },
                    ManifestTask {
                        id: cycle_c.clone(),
                        file: "docs/tasks/c.md".to_string(),
                    },
                ],
            },
            milestone_index: String::new(),
            task_files: BTreeMap::new(),
        };

        let result = validate_dependency_graph(&artifacts);
        assert!(
            result.is_err(),
            "deep 3-node cycle should be detected (old bug passed &[] for deps)"
        );
        match result.expect_err("expected error should be returned") {
            GenerationError::CircularDependency(msg) => {
                assert!(msg.contains("Cycle"));
            }
            other => panic!("expected CircularDependency, got {:?}", other),
        }
    }

    #[test]
    fn dependency_graph_validation_detects_blocks_only_cycle() {
        let cycle_a = TaskId("TASK-001".to_string());
        let cycle_b = TaskId("TASK-002".to_string());
        let cycle_c = TaskId("TASK-003".to_string());

        let artifacts = PlanArtifacts {
            generated_at: Utc::now(),
            planning_wave: "test".to_string(),
            milestones: vec![PlannedMilestone {
                id: TaskId("MS-1".to_string()),
                name: "M1: Test".to_string(),
                goal: "Test goal".to_string(),
                issues: vec![
                    PlannedIssue {
                        id: cycle_a.clone(),
                        title: "Task A".to_string(),
                        summary: "A".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![],
                        blocks: vec![cycle_b.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_b.clone(),
                        title: "Task B".to_string(),
                        summary: "B".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![],
                        blocks: vec![cycle_c.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                    PlannedIssue {
                        id: cycle_c.clone(),
                        title: "Task C".to_string(),
                        summary: "C".to_string(),
                        scope_in: vec![],
                        scope_out: vec![],
                        deliverables: vec![],
                        acceptance_criteria: vec![],
                        verification_steps: vec![],
                        context: vec![],
                        definition_of_ready: vec![],
                        notes: None,
                        priority: TaskPriority::Normal,
                        estimate: None,
                        blocked_by: vec![],
                        blocks: vec![cycle_a.clone()],
                        sub_issues: vec![],
                        task_file: None,
                    },
                ],
                acceptance_criteria: vec![],
                verification_steps: vec![],
                notes: None,
            }],
            manifest: TaskPackageManifest {
                planning_wave: "test".to_string(),
                tasks_dir: "docs/tasks".to_string(),
                milestones: vec!["M1: Test".to_string()],
                tasks: vec![],
            },
            milestone_index: String::new(),
            task_files: BTreeMap::new(),
        };

        assert!(
            validate_dependency_graph(&artifacts).is_err(),
            "cycles represented only by blocks edges should be detected"
        );
    }
}
